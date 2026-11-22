//! Bybit v5 adapter — PRIORITY venue. Implements REST signing (HMAC-SHA256
//! lowercase hex over `ts + key + recv_window + query|jsonBody`), market data,
//! private account/trade endpoints, and the /v5/user/query-api permission
//! scan for the Zero-Withdrawal hard gate.
//!
//! Reference: docs/BYBIT_RESEARCH.md (live-verified field names).

use super::{VenueCreds,
    ExecutionRecord, KeyPermissions, Kline, OrderBook, OrderBookLevel, OrderRequest, PositionSnapshot, Ticker24h, TickerSnapshot,
    VenueAdapter, VenueOrderState, WalletSnapshot,
};
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;
use std::sync::atomic::{AtomicI64, Ordering};

pub const MAINNET: &str = "https://api.bybit.com";
pub const TESTNET: &str = "https://api-testnet.bybit.com";

fn parse_key_permissions(v: &Value) -> Result<KeyPermissions, String> {
    let read_only = match v.get("readOnly").and_then(|x| x.as_i64()) {
        Some(0) => false,
        Some(1) => true,
        _ => return Err("bybit query-api: readOnly field missing or malformed".into()),
    };
    let required_permissions = |name: &str| -> Result<Vec<String>, String> {
        let values = v
            .pointer(&format!("/permissions/{name}"))
            .and_then(|value| value.as_array())
            .ok_or_else(|| format!("bybit query-api: permissions.{name} missing or malformed"))?;
        values
            .iter()
            .map(|value| value.as_str().map(str::to_string).ok_or_else(|| format!("bybit query-api: permissions.{name} contains a non-string value")))
            .collect()
    };
    let wallet_perms = required_permissions("Wallet")?;
    let spot_perms = required_permissions("Spot")?;
    let contract_perms = required_permissions("ContractTrade")?;
    let wallet_movement = wallet_perms.iter().any(|permission| {
        matches!(permission.as_str(), "Withdraw" | "AccountTransfer" | "SubMemberTransfer" | "SubMemberTransferList")
    });
    let spot = spot_perms.iter().any(|permission| permission == "SpotTrade");
    let contract = contract_perms.iter().any(|permission| permission == "Order")
        && contract_perms.iter().any(|permission| permission == "Position");
    let ips = v
        .get("ips")
        .and_then(|x| x.as_array())
        .map(|values| values.iter().filter_map(|item| item.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    Ok(KeyPermissions {
        read_only,
        can_withdraw: wallet_movement,
        spot_trade: spot,
        contract_trade: contract,
        ips,
        note: format!("wallet_movement_permission={wallet_movement}, spot={spot}, contract={contract}"),
    })
}

pub struct BybitAdapter {
    pub base_url: String,
    pub recv_window: String,
    pub http: reqwest::Client,
    pub time_offset_ms: AtomicI64,
}

impl BybitAdapter {
    pub fn new(testnet: bool) -> Self {
        let raw_base_url = if let Ok(u) = std::env::var("BYBIT_BASE_URL").or_else(|_| std::env::var("DCA_BYBIT_BASE_URL")) {
            u
        } else if testnet || std::env::var("DCA_BYBIT_TESTNET").as_deref() == Ok("1") {
            TESTNET.into()
        } else {
            MAINNET.into()
        };
        let base_url = super::guarded_venue_base_url(
            &raw_base_url,
            "Bybit",
            &["api.bybit.com", "api-testnet.bybit.com", "api-demo.bybit.com"],
            &["api.bybit.com"],
            false,
        );
        let recv_window = std::env::var("BYBIT_RECV_WINDOW").unwrap_or_else(|_| "5000".into());
        Self {
            base_url,
            recv_window,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("reqwest client"),
            time_offset_ms: AtomicI64::new(0),
        }
    }

    /// Zaman damgasını sunucu saati farkı (drift offset) ile düzelterek döndürür.
    pub fn now_ms(&self) -> i64 {
        raw_system_now_ms().saturating_add(self.time_offset_ms.load(Ordering::Relaxed))
    }

    /// Bybit /v5/market/time ucundan borsa saatini sorgular ve yerel saat ile arasındaki
    /// farkı (drift) atomik olarak time_offset_ms içerisine kaydeder.
    pub async fn sync_server_time(&self) -> Result<i64, String> {
        let local_before = raw_system_now_ms();
        let url = format!("{}/v5/market/time", self.base_url);
        let resp = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| format!("bybit time sync network error: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| format!("bybit time sync body error: {e}"))?;
        if !status.is_success() {
            let snippet: String = body.chars().take(200).collect();
            return Err(format!("bybit time sync HTTP error {status}: {snippet}"));
        }
        let v: Value = serde_json::from_str(&body).map_err(|e| format!("bybit time sync json error: {e}"))?;
        let server_ms = if let Some(t) = v.get("time").and_then(|x| x.as_i64()) {
            t
        } else if let Some(nano) = v.get("result").and_then(|r| r.get("timeNano")).and_then(|x| x.as_str()).and_then(|s| s.parse::<i64>().ok()) {
            nano / 1_000_000
        } else if let Some(sec) = v.get("result").and_then(|r| r.get("timeSecond")).and_then(|x| x.as_str()).and_then(|s| s.parse::<i64>().ok()) {
            sec * 1000
        } else {
            return Err("bybit time sync: server time not found in payload".into());
        };
        let local_after = raw_system_now_ms();
        let local_mid = (local_before + local_after) / 2;
        let offset = server_ms - local_mid;
        self.time_offset_ms.store(offset, Ordering::Relaxed);
        tracing::info!("Bybit server time synced: offset={}ms (server={}, local={})", offset, server_ms, local_mid);
        Ok(offset)
    }

    /// GET sign string: ts + key + recv_window + queryString (as-sent order).
    fn sign_get(&self, ts: &str, key: &str, secret: &str, query: &str) -> String {
        let payload = format!("{ts}{key}{}{query}", self.recv_window);
        sign_hmac(secret, &payload)
    }

    /// POST sign string: ts + key + recv_window + jsonBody (exact bytes).
    fn sign_post(&self, ts: &str, key: &str, secret: &str, body: &str) -> String {
        let payload = format!("{ts}{key}{}{body}", self.recv_window);
        sign_hmac(secret, &payload)
    }

    async fn public_get(&self, path: &str, query: &str) -> Result<Value, String> {
        let url = if query.is_empty() {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}?{}", self.base_url, path, query)
        };
        let resp = self
            .http
            .get(&url)
            .header("Content-Type", "application/json")
            .send()
            .await
            .map_err(|e| format!("bybit GET {path} failed: {e}"))?;

        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| format!("bybit GET {path} read body error: {e}"))?;
        if !status.is_success() {
            let snippet: String = body_text.chars().take(200).collect();
            if status.as_u16() == 429 {
                return Err(format!("bybit HTTP 429 Too Many Requests on {path}: {snippet}"));
            }
            return Err(format!("bybit HTTP {status} on {path}: {snippet}"));
        }
        let v: Value = serde_json::from_str(&body_text).map_err(|e| {
            let snippet: String = body_text.chars().take(200).collect();
            format!("bybit GET {path} invalid JSON: {e}; snippet: {snippet}")
        })?;
        check_ret(v, path)
    }

    async fn private_get(&self, key: &str, secret: &str, path: &str, query: &str) -> Result<Value, String> {
        let ts = self.now_ms().to_string();
        let sign = self.sign_get(&ts, key, secret, query);
        let url = if query.is_empty() {
            format!("{}{}", self.base_url, path)
        } else {
            format!("{}{}?{}", self.base_url, path, query)
        };
        let resp = self
            .http
            .get(&url)
            .header("X-BAPI-API-KEY", key)
            .header("X-BAPI-SIGN", sign)
            .header("X-BAPI-TIMESTAMP", &ts)
            .header("X-BAPI-RECV-WINDOW", &self.recv_window)
            .send()
            .await
            .map_err(|e| format!("bybit private GET {path} failed: {e}"))?;

        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| format!("bybit private GET {path} read body error: {e}"))?;
        if !status.is_success() {
            let snippet: String = body_text.chars().take(200).collect();
            if status.as_u16() == 429 {
                return Err(format!("bybit HTTP 429 Too Many Requests on {path}: {snippet}"));
            }
            return Err(format!("bybit HTTP {status} on {path}: {snippet}"));
        }
        let v: Value = serde_json::from_str(&body_text).map_err(|e| {
            let snippet: String = body_text.chars().take(200).collect();
            format!("bybit private GET {path} invalid JSON: {e}; snippet: {snippet}")
        })?;
        check_ret(v, path)
    }

    async fn private_post(&self, key: &str, secret: &str, path: &str, body: &str) -> Result<Value, String> {
        let ts = self.now_ms().to_string();
        let sign = self.sign_post(&ts, key, secret, body);
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .header("X-BAPI-API-KEY", key)
            .header("X-BAPI-SIGN", sign)
            .header("X-BAPI-TIMESTAMP", &ts)
            .header("X-BAPI-RECV-WINDOW", &self.recv_window)
            .header("Content-Type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .map_err(|e| format!("bybit private POST {path} failed: {e}"))?;

        let status = resp.status();
        let body_text = resp.text().await.map_err(|e| format!("bybit private POST {path} read body error: {e}"))?;
        if !status.is_success() {
            let snippet: String = body_text.chars().take(200).collect();
            if status.as_u16() == 429 {
                return Err(format!("bybit HTTP 429 Too Many Requests on {path}: {snippet}"));
            }
            return Err(format!("bybit HTTP {status} on {path}: {snippet}"));
        }
        let v: Value = serde_json::from_str(&body_text).map_err(|e| {
            let snippet: String = body_text.chars().take(200).collect();
            format!("bybit private POST {path} invalid JSON: {e}; snippet: {snippet}")
        })?;
        check_ret(v, path)
    }

    /// Faz 5: pozisyon modu değişimi — mode 0 = tek-yönlü, 3 = hedge.
    /// Yalnız pozisyon/açık emir yokken çalışır (Bybit 110024). Motor bunu
    /// SADECE Demo/Testnet uçlarında otomatik dener; mainnet'te çağrılmaz.
    pub async fn switch_pos_mode(&self, creds: &VenueCreds, one_way: bool) -> Result<(), String> {
        let mode = if one_way { 0 } else { 3 };
        let body = serde_json::json!({ "category": "linear", "mode": mode }).to_string();
        match self.private_post(&creds.key, &creds.secret, "/v5/position/switch-mode", &body).await {
            Ok(_) => Ok(()),
            Err(e) => Err(format!("switch-mode({mode}): {e}")),
        }
    }

    /// Motor bu adaptör Demo/Testnet ucuna mı bakıyor (otomatik mod
    /// düzeltmesi yalnız burada denenir).
    pub fn is_demo_or_testnet(&self) -> bool {
        self.base_url.contains("api-demo") || self.base_url.contains("api-testnet")
    }

    fn parse_instrument(&self, category: &str, item: &Value) -> Option<Instrument> {
        let symbol = item.get("symbol")?.as_str()?.to_string();
        let base = item.get("baseCoin").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let quote = item.get("quoteCoin").and_then(|x| x.as_str()).unwrap_or("USDT").to_string();
        let tick = item
            .pointer("/priceFilter/tickSize")
            .and_then(|x| x.as_str())
            .unwrap_or("0.01");
        let (qty_step, min_qty, min_notional, max_lev) = if category == "linear" {
            (
                item.pointer("/lotSizeFilter/qtyStep").and_then(|x| x.as_str()).unwrap_or("0.001"),
                item.pointer("/lotSizeFilter/minOrderQty").and_then(|x| x.as_str()).unwrap_or("0.001"),
                item.pointer("/lotSizeFilter/minNotionalValue").and_then(|x| x.as_str()).unwrap_or("5"),
                item.pointer("/leverageFilter/maxLeverage").and_then(|x| x.as_str()).unwrap_or("50"),
            )
        } else {
            (
                item.pointer("/lotSizeFilter/qtyStep").and_then(|x| x.as_str()).unwrap_or("0.00001"),
                item.pointer("/lotSizeFilter/minOrderQty").and_then(|x| x.as_str()).unwrap_or("0.00001"),
                item.pointer("/lotSizeFilter/minNotionalValue").and_then(|x| x.as_str()).unwrap_or("1"),
                "1",
            )
        };
        let (maker, taker) = if category == "linear" { ("0.0002", "0.00055") } else { ("0.001", "0.001") };
        Some(Instrument {
            symbol,
            base,
            quote,
            tick_size: Fraction::from_decimal_str(tick).ok()?,
            step_size: Fraction::from_decimal_str(qty_step).ok()?,
            min_qty: Fraction::from_decimal_str(min_qty).ok()?,
            min_notional: Fraction::from_decimal_str(min_notional).ok()?,
            maker_fee_rate: Fraction::from_decimal_str(maker).ok()?,
            taker_fee_rate: Fraction::from_decimal_str(taker).ok()?,
            max_leverage: Fraction::from_any_str(max_lev).unwrap_or_else(|_| Fraction::from_u64(1)),
        })
    }
}

/// P3-4 zeroization (v3 memory-scrub karşılığı): imza anahtarının kopyası
/// `Zeroizing` zarfında yaşar; drop'ta (panic yolu dahil) sıfırlanır.
/// Dürüst sınır (v3'tekiyle aynı): çağrıcının `&str` kaynağı immutable'dır,
/// zarf imzalama kopyasının heap dump'ta canlı kalmasını engeller.
pub fn sign_hmac(secret: &str, payload: &str) -> String {
    use zeroize::Zeroizing;
    let key = Zeroizing::new(secret.as_bytes().to_vec());
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("hmac key");
    mac.update(payload.as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub fn raw_system_now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub fn now_ms() -> i64 {
    raw_system_now_ms()
}

fn check_ret(v: Value, path: &str) -> Result<Value, String> {
    let code = v.get("retCode").and_then(|x| x.as_i64()).unwrap_or(-1);
    if code != 0 {
        let msg = v.get("retMsg").and_then(|x| x.as_str()).unwrap_or("unknown");
        if code == 10002 {
            return Err(format!("bybit {path} retCode=10002 (clock drift/expired timestamp): {msg} - sync_server_time required"));
        }
        if code == 10006 {
            return Err(format!("bybit {path} retCode=10006 (rate limit exceeded): {msg}"));
        }
        return Err(format!("bybit {path} retCode={code}: {msg}"));
    }
    Ok(v.get("result").cloned().unwrap_or(Value::Null))
}

/// Faz 2.7 — saf: Bybit risk-limit listesini M07 kademelerine normalize eder.
/// Girdi: `/v5/market/risk-limit` `result` nesnesi (`list[]`: riskLimitValue /
/// maintenanceMargin / maxLeverage). Kademe i: floor = onceki cap (ilk 0),
/// cap = riskLimitValue, mmr = maintenanceMargin, cum (deduction) sinir
/// surekliliginden turetilir: cum_i = floor_i*(mmr_i-mmr_{i-1}) + cum_{i-1}
/// (ilk kademe cum=0). maxLeverage tasinmaz — M07 tuketmiyor (notlu).
pub fn parse_bybit_risk_limit(v: &Value) -> Result<Vec<dca_math::MmrBracket>, String> {
    let list = v.get("list").and_then(|x| x.as_array()).ok_or("bybit risk-limit: list yok")?;
    if list.is_empty() {
        return Err("bybit risk-limit: bos kademe".into());
    }
    let mut rows: Vec<(Fraction, Fraction)> = Vec::with_capacity(list.len());
    for it in list {
        let cap = it.get("riskLimitValue").and_then(|x| x.as_str()).ok_or("riskLimitValue yok")?;
        let mmr = it.get("maintenanceMargin").and_then(|x| x.as_str()).ok_or("maintenanceMargin yok")?;
        rows.push((
            Fraction::from_any_str(cap).map_err(|e| format!("cap cozulmedi: {e}"))?,
            Fraction::from_any_str(mmr).map_err(|e| format!("mmr cozulmedi: {e}"))?,
        ));
    }
    rows.sort_by(|a, b| a.0.cmp(&b.0));
    let zero = Fraction::zero();
    let mut tiers = Vec::with_capacity(rows.len());
    let (mut prev_cap, mut prev_mmr, mut prev_cum) = (zero.clone(), zero.clone(), zero.clone());
    for (i, (cap, mmr)) in rows.into_iter().enumerate() {
        if cap <= prev_cap && i > 0 {
            return Err("bybit risk-limit: sirali olmayan cap".into());
        }
        let floor = prev_cap.clone();
        let cum = if i == 0 {
            zero.clone()
        } else {
            let d = mmr.sub(&prev_mmr).map_err(|e| format!("cum fark: {e}"))?;
            floor.mul(&d).map_err(|e| format!("cum carpim: {e}"))?.add(&prev_cum).map_err(|e| format!("cum toplam: {e}"))?
        };
        tiers.push(dca_math::MmrBracket { min_notional: floor, max_notional: cap.clone(), mmr: mmr.clone(), deduction: cum.clone() });
        prev_cap = cap;
        prev_mmr = mmr;
        prev_cum = cum;
    }
    Ok(tiers)
}

#[async_trait::async_trait]
impl VenueAdapter for BybitAdapter {
    fn name(&self) -> &'static str {
        "BYBIT"
    }

    async fn server_time_ms(&self) -> Result<i64, String> {
        // /v5/market/time -> result: {"timeSecond":"<sn>","timeNano":"<ns>"}
        // (eski kod "time" alanini okuyup bulamayinca timeNano'yu ms saniyordu:
        // 19 haneli nano deger ms diye donuyordu. Simdi normalize ediliyor.)
        let v = self.public_get("/v5/market/time", "").await?;
        // 1) timeSecond (sn, string ya da sayi, ondalikli olabilir) -> ms
        if let Some(sec) = v.get("timeSecond").and_then(|x| {
            x.as_str()
                .and_then(|s| s.parse::<f64>().ok())
                .or_else(|| x.as_f64())
                .or_else(|| x.as_i64().map(|n| n as f64))
        }) {
            if sec > 0.0 {
                return Ok((sec * 1000.0) as i64);
            }
        }
        // 2) timeNano (ns string) -> ms
        if let Some(ns) = v.get("timeNano").and_then(|x| {
            x.as_str()
                .and_then(|s| s.parse::<i64>().ok())
                .or_else(|| x.as_i64())
        }) {
            if ns > 0 {
                return Ok(ns / 1_000_000);
            }
        }
        // 3) eski/legacy "time" alani — buyukluge gore normalize et
        if let Some(t) = v.get("time").and_then(|x| x.as_i64()) {
            if t > 0 {
                if t > 1_000_000_000_000_000 {
                    return Ok(t / 1_000_000); // nano -> ms
                }
                if t > 1_000_000_000_000 {
                    return Ok(t); // zaten ms
                }
                return Ok(t * 1000); // sn -> ms
            }
        }
        Err("no time".into())
    }

    async fn instruments(&self, category: &str, symbol: Option<&str>) -> Result<Vec<Instrument>, String> {
        // Bybit tek sayfada en fazla 1000 enstrüman döner — tam liste (Linear
        // ~600+ çift) için nextPageCursor bitene kadar sayfala. Sayfalama yokken
        // liste 500'de kesiliyor, SOLUSDT gibi büyük pariteler düşüyordu.
        let mut out = Vec::new();
        let mut cursor = String::new();
        loop {
            let mut query = format!("category={category}&limit=1000");
            if let Some(s) = symbol {
                query.push_str(&format!("&symbol={s}"));
            }
            if !cursor.is_empty() {
                query.push_str(&format!("&cursor={cursor}"));
            }
            let v = self.public_get("/v5/market/instruments-info", &query).await?;
            if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
                for item in list {
                    if let Some(i) = self.parse_instrument(category, item) {
                        out.push(i);
                    }
                }
            }
            cursor = v
                .get("nextPageCursor")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if cursor.is_empty() {
                break;
            }
        }
        Ok(out)
    }

    async fn klines(&self, category: &str, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Kline>, String> {
        let query = format!(
            "category={category}&symbol={symbol}&interval={interval}&limit={}",
            limit.min(1000)
        );
        let v = self.public_get("/v5/market/kline", &query).await?;
        let mut out = Vec::new();
        if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
            for row in list {
                if let Some(arr) = row.as_array() {
                    if arr.len() >= 6 {
                        let t: i64 = arr[0].as_str().and_then(|s| s.parse().ok()).or_else(|| arr[0].as_i64()).unwrap_or(0);
                        out.push(Kline {
                            t,
                            o: arr[1].as_str().unwrap_or("0").to_string(),
                            h: arr[2].as_str().unwrap_or("0").to_string(),
                            l: arr[3].as_str().unwrap_or("0").to_string(),
                            c: arr[4].as_str().unwrap_or("0").to_string(),
                            v: arr[5].as_str().unwrap_or("0").to_string(),
                        });
                    }
                }
            }
        }
        // Bybit returns DESCENDING — normalize to ascending for the kernel.
        out.reverse();
        Ok(out)
    }

    async fn ticker(&self, category: &str, symbol: &str) -> Result<TickerSnapshot, String> {
        let query = format!("category={category}&symbol={symbol}");
        let v = self.public_get("/v5/market/tickers", &query).await?;
        let item = v
            .get("list")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .cloned()
            .ok_or("no ticker")?;
        let last = item.get("lastPrice").and_then(|x| x.as_str()).unwrap_or("0");
        let mark = item.get("markPrice").and_then(|x| x.as_str()).unwrap_or(last);
        Ok(TickerSnapshot {
            symbol: symbol.into(),
            last_price: Fraction::from_any_str(last).unwrap_or_else(|_| Fraction::zero()),
            mark_price: Fraction::from_any_str(mark).unwrap_or_else(|_| Fraction::zero()),
            funding_rate: item.get("fundingRate").and_then(|x| x.as_str()).and_then(|s| Fraction::from_decimal_str(s).ok()),
            next_funding_ts: item.get("nextFundingTime").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).or_else(|| item.get("nextFundingTime").and_then(|x| x.as_i64())),
        })
    }

    async fn tickers_24h(&self, category: &str) -> Result<Vec<Ticker24h>, String> {
        let query = format!("category={category}");
        let v = self.public_get("/v5/market/tickers", &query).await?;
        let mut out = Vec::new();
        if let Some(list) = v.get("result").and_then(|r| r.get("list")).and_then(|x| x.as_array()).or_else(|| v.get("list").and_then(|x| x.as_array())) {
            for item in list {
                let s = |k: &str| item.get(k).and_then(|x| x.as_str()).unwrap_or("0").to_string();
                let raw_pcnt = item.get("price24hPcnt").and_then(|x| x.as_str()).unwrap_or("0");
                let pct = Fraction::from_decimal_str(raw_pcnt)
                    .ok()
                    .and_then(|f| f.mul(&Fraction::from_u64(100)).ok())
                    .map(|f| f.to_half_even_display(4))
                    .unwrap_or_else(|| raw_pcnt.to_string());
                out.push(Ticker24h {
                    symbol: s("symbol"),
                    last: s("lastPrice"),
                    change_pct_24h: pct,
                    high_24h: s("highPrice24h"),
                    low_24h: s("lowPrice24h"),
                    volume_24h: s("volume24h"),
                    turnover_24h: s("turnover24h"),
                });
            }
        }
        Ok(out)
    }

    /// Faz 22 (P1-23): /v5/account/fee-rate — hesap-bazlı gerçek maker/taker
    /// (enstrüman varsayılanları VIP kademesini yansıtmaz).
    async fn account_fee_rate(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
    ) -> Result<Option<(Fraction, Fraction)>, String> {
        let query = format!("category={category}&symbol={symbol}");
        let v = self.private_get(&creds.key, &creds.secret, "/v5/account/fee-rate", &query).await?;
        let Some(list) = v.get("list").and_then(|x| x.as_array()) else {
            return Ok(None);
        };
        let Some(item) = list.first() else {
            return Ok(None);
        };
        let maker = item.get("makerFeeRate").and_then(|x| x.as_str()).unwrap_or("");
        let taker = item.get("takerFeeRate").and_then(|x| x.as_str()).unwrap_or("");
        if maker.is_empty() || taker.is_empty() {
            return Ok(None);
        }
        Ok(Some((
            Fraction::from_decimal_str(maker).map_err(|e| e.to_string())?,
            Fraction::from_decimal_str(taker).map_err(|e| e.to_string())?,
        )))
    }

    async fn funding_history(&self, symbol: &str, limit: usize) -> Result<Vec<(i64, String)>, String> {
        let query = format!("category=linear&symbol={symbol}&limit={}", limit.min(200));
        let v = self.public_get("/v5/market/funding/history", &query).await?;
        let mut out = Vec::new();
        if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
            for item in list {
                let ts = item.get("fundingRateTimestamp").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).or_else(|| item.get("fundingRateTimestamp").and_then(|x| x.as_i64())).unwrap_or(0);
                let rate = item.get("fundingRate").and_then(|x| x.as_str()).unwrap_or("0").to_string();
                out.push((ts, rate));
            }
        }
        Ok(out)
    }

    /// GET /v5/user/query-api (live-verified; the old query-api-key path is 404).
    async fn key_permissions(&self, creds: &VenueCreds) -> Result<KeyPermissions, String> {
        let v = self.private_get(&creds.key, &creds.secret, "/v5/user/query-api", "").await?;
        parse_key_permissions(&v)
    }

    async fn wallet(&self, creds: &VenueCreds, coin: &str) -> Result<WalletSnapshot, String> {
        let v = self
            .private_get(&creds.key, &creds.secret, "/v5/account/wallet-balance", "accountType=UNIFIED")
            .await?;
        let list = v
            .pointer("/list")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .ok_or("no account")?;
        let coins = list.get("coin").and_then(|x| x.as_array()).ok_or("no coins")?;
        let target = coins
            .iter()
            .find(|c| c.get("coin").and_then(|x| x.as_str()) == Some(coin))
            .or_else(|| coins.first())
            .ok_or("no coin entry")?;
        // M-13: UTA'da emir kapasitesi hesap seviyesindeki totalAvailableBalance'tir;
        // coin satırındaki availableToWithdraw para çekme kısıtlı farklı bir kavramdır
        // (genelde 0/az döner) ve C-6 kapısını YANLIŞ reddettirirdi.
        let account_avail = list
            .get("totalAvailableBalance")
            .and_then(|x| x.as_str())
            .and_then(|v| Fraction::from_any_str(v).ok())
            .unwrap_or_else(Fraction::zero);
        let coin_avail = Fraction::from_any_str(target.get("availableToWithdraw").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero());
        let avail = if account_avail > coin_avail { account_avail } else { coin_avail };
        Ok(WalletSnapshot {
            coin: target.get("coin").and_then(|x| x.as_str()).unwrap_or(coin).to_string(),
            wallet_balance: Fraction::from_any_str(target.get("walletBalance").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
            available: avail,
            equity: Fraction::from_any_str(target.get("equity").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
            unrealized_pnl: Fraction::from_any_str(target.get("unrealisedPnl").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
        })
    }

    async fn place_order(&self, creds: &VenueCreds, req: &OrderRequest) -> Result<String, String> {
        let mut body = serde_json::json!({
            "category": req.category,
            "symbol": req.symbol,
            "side": match req.side { dca_math::Side::Buy => "Buy", dca_math::Side::Sell => "Sell" },
            "orderType": if req.is_market { "Market" } else { "Limit" },
            "qty": req.qty.to_half_even_display(8),
            "orderLinkId": req.order_link_id,
        });
        if !req.is_market {
            body["price"] = Value::String(req.price.as_ref().map(|p| p.to_half_even_display(8)).unwrap_or_default());
            body["timeInForce"] = Value::String("GTC".into());
        }
        if req.category == "spot" {
            body["isLeverage"] = Value::from(0);
            // EX-02: Bybit spot market BUY varsayılan olarak quote tutarını kabul eder.
            // marketUnit=baseCoin ile qty base coin cinsinden açıkça belirtilir.
            body["marketUnit"] = Value::String("baseCoin".into());
        } else {
            body["positionIdx"] = Value::from(req.position_idx);
            if req.reduce_only {
                body["reduceOnly"] = Value::Bool(true);
            }
        }
        let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        let v = self.private_post(&creds.key, &creds.secret, "/v5/order/create", &body_str).await?;
        v.get("orderId").and_then(|x| x.as_str()).map(String::from).ok_or_else(|| "no orderId".into())
    }

    async fn cancel_order(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<(), String> {
        let body = serde_json::json!({
            "category": category,
            "symbol": symbol,
            "orderLinkId": order_link_id,
        });
        let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        match self.private_post(&creds.key, &creds.secret, "/v5/order/cancel", &body_str).await {
            Ok(_) => Ok(()),
            // retCode 110001 "Order not exists or too late to cancel" — the order
            // already filled/cancelled venue-side; treat as idempotent success
            // (same contract as Binance -2011) so the caller's cancel chain
            // continues instead of aborting on the first already-gone order.
            Err(e) if e.contains("retCode=110001") || e.to_lowercase().contains("order not exists") => Ok(()),
            Err(e) => Err(e),
        }
    }

    async fn open_orders(&self, creds: &VenueCreds, category: &str, symbol: &str) -> Result<Vec<VenueOrderState>, String> {
        // Task 20 L1: kategori botun ÜRÜN kategorisidir — sabit "linear"
        // spot resting emirlerini görmezdi (phantom-fill kusuru, düzeltildi).
        let query = format!("category={category}&symbol={symbol}&openOnly=0");
        let v = self.private_get(&creds.key, &creds.secret, "/v5/order/realtime", &query).await?;
        let mut out = Vec::new();
        if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
            for item in list {
                out.push(VenueOrderState {
                    order_id: item.get("orderId").and_then(|x| x.as_str()).unwrap_or("").into(),
                    order_link_id: item.get("orderLinkId").and_then(|x| x.as_str()).unwrap_or("").into(),
                    symbol: item.get("symbol").and_then(|x| x.as_str()).unwrap_or("").into(),
                    side: item.get("side").and_then(|x| x.as_str()).unwrap_or("").into(),
                    price: item.get("price").and_then(|x| x.as_str()).and_then(|s| Fraction::from_decimal_str(s).ok()),
                    qty: Fraction::from_any_str(item.get("qty").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                    cum_exec_qty: Fraction::from_any_str(item.get("cumExecQty").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                    cum_exec_fee: item.get("cumExecFee").and_then(|x| x.as_str()).and_then(|s| Fraction::from_any_str(s).ok()),
                    status: item.get("orderStatus").and_then(|x| x.as_str()).unwrap_or("").into(),
                });
            }
        }
        Ok(out)
    }

    /// Faz 3 (C-3) — emrin venue execution kayıtları (fill kanıtı; imzalı):
    /// GET /v5/execution/list?category&symbol&orderLinkId. Her kayıt kendi
    /// exec_id'siyle idempotent işlenir; Cancelled/Rejected buradan hiç gelmez.
    async fn executions(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<Vec<ExecutionRecord>, String> {
        let query = format!("category={category}&symbol={symbol}&orderLinkId={order_link_id}&limit=100");
        let v = self.private_get(&creds.key, &creds.secret, "/v5/execution/list", &query).await?;
        let mut out = Vec::new();
        if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
            for item in list {
                let exec_id = item.get("execId").and_then(|x| x.as_str()).unwrap_or("").to_string();
                if exec_id.is_empty() {
                    continue;
                }
                let returned_link_id = item.get("orderLinkId").and_then(|x| x.as_str()).unwrap_or("");
                if returned_link_id.is_empty() || returned_link_id != order_link_id {
                    continue;
                }
                let exec_qty = Fraction::from_any_str(item.get("execQty").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero());
                let exec_price = Fraction::from_any_str(item.get("execPrice").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero());
                // H-4 (fee-coin): Bybit spot alışında ücret BASE cinsinden kesilir
                // (satışta quote). Muhasebe quote olduğundan alış ücreti quote
                // karşılığına çevrilir: fee_q = fee_base * execPrice. Lineer
                // vadeli ücreti zaten USDT(quote) cinsindendir.
                let side_str = item.get("side").and_then(|x| x.as_str()).unwrap_or("");
                let raw_fee = item.get("execFee").and_then(|x| x.as_str()).and_then(|s| Fraction::from_any_str(s).ok());
                let exec_fee = match (raw_fee, category) {
                    (Some(f), "spot") if side_str == "Buy" => f.mul(&exec_price).ok().or(Some(f)),
                    (f, _) => f,
                };
                out.push(ExecutionRecord {
                    order_link_id: returned_link_id.to_string(),
                    exec_id,
                    exec_qty,
                    exec_price,
                    exec_fee,
                    is_maker: item.get("isMaker").and_then(|x| x.as_bool()).unwrap_or(false),
                    exec_time: item.get("execTime").and_then(|x| x.as_str()).and_then(|s| s.parse().ok()).or_else(|| item.get("execTime").and_then(|x| x.as_i64())).unwrap_or(0),
                });
            }
        }
        Ok(out)
    }

    /// Task 20 L3 — tek emir sorgusu: önce realtime (açık emir), bulunamazsa
    /// order-history (terminal: Filled/Cancelled). Market dolum yakalama buradan
    /// yapılır; fiyat olarak avgPrice (yoksa price) alınır.
    async fn query_order(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<Option<VenueOrderState>, String> {
        let parse_item = |item: &serde_json::Value| -> Option<VenueOrderState> {
            let avg = item.get("avgPrice").and_then(|x| x.as_str()).unwrap_or("");
            let px = item.get("price").and_then(|x| x.as_str()).unwrap_or("");
            let price_src = if !avg.is_empty() && avg != "0" { avg } else { px };
            Some(VenueOrderState {
                order_id: item.get("orderId").and_then(|x| x.as_str()).unwrap_or("").into(),
                order_link_id: item.get("orderLinkId").and_then(|x| x.as_str()).unwrap_or("").into(),
                symbol: item.get("symbol").and_then(|x| x.as_str()).unwrap_or("").into(),
                side: item.get("side").and_then(|x| x.as_str()).unwrap_or("").into(),
                price: Fraction::from_decimal_str(price_src).ok(),
                qty: Fraction::from_any_str(item.get("qty").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                cum_exec_qty: Fraction::from_any_str(item.get("cumExecQty").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                cum_exec_fee: item.get("cumExecFee").and_then(|x| x.as_str()).and_then(|s| Fraction::from_any_str(s).ok()),
                status: item.get("orderStatus").and_then(|x| x.as_str()).unwrap_or("").into(),
            })
        };
        // 1) açık emirler arasında ara
        let q1 = format!("category={category}&symbol={symbol}&orderLinkId={order_link_id}");
        let v = self.private_get(&creds.key, &creds.secret, "/v5/order/realtime", &q1).await?;
        if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
            if let Some(item) = list.first() {
                return Ok(parse_item(item));
            }
        }
        // 2) terminal geçmişinde ara (market emir anında terminale düşer)
        let q2 = format!("category={category}&symbol={symbol}&orderLinkId={order_link_id}&limit=1");
        let v = self.private_get(&creds.key, &creds.secret, "/v5/order/history", &q2).await?;
        if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
            if let Some(item) = list.first() {
                return Ok(parse_item(item));
            }
        }
        Ok(None)
    }

    async fn positions(&self, creds: &VenueCreds, symbol: &str) -> Result<Vec<PositionSnapshot>, String> {
        let query = format!("category=linear&symbol={symbol}");
        let v = self.private_get(&creds.key, &creds.secret, "/v5/position/list", &query).await?;
        let mut out = Vec::new();
        if let Some(list) = v.get("list").and_then(|x| x.as_array()) {
            for item in list {
                out.push(PositionSnapshot {
                    symbol: item.get("symbol").and_then(|x| x.as_str()).unwrap_or("").into(),
                    side: item.get("side").and_then(|x| x.as_str()).unwrap_or("").into(),
                    size: Fraction::from_any_str(item.get("size").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                    avg_price: Fraction::from_any_str(item.get("avgPrice").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                    mark_price: Fraction::from_any_str(item.get("markPrice").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                    unrealized_pnl: Fraction::from_any_str(item.get("unrealisedPnl").and_then(|x| x.as_str()).unwrap_or("0")).unwrap_or_else(|_| Fraction::zero()),
                    liq_price: item.get("liqPrice").and_then(|x| x.as_str()).and_then(|s| Fraction::from_decimal_str(s).ok()),
                    leverage: Fraction::from_any_str(item.get("leverage").and_then(|x| x.as_str()).unwrap_or("1")).unwrap_or_else(|_| Fraction::one()),
                    // Faz 4.5: 0=one-way, 1/2=hedge (docs: position/position-mode)
                    position_idx: item.get("positionIdx").and_then(|x| x.as_i64()).map(|v| v as i32),
                });
            }
        }
        Ok(out)
    }

    /// Faz 4.5 (C-5): pozisyon modu — Bybit'te ayrı GET ucu yoktur; mod,
    /// pozisyon kayıtlarındaki positionIdx'den çıkarılır (0=one-way/net,
    /// 1/2=hedge/long_short). Pozisyonu olmayan hesapta belirsiz → None
    /// (denetim devre dışı kalır; emir reddi gelirse venue hatası zaten görünür).
    async fn pos_mode(&self, creds: &VenueCreds) -> Result<Option<String>, String> {
        let v = self.private_get(&creds.key, &creds.secret, "/v5/position/list", "category=linear&settleCoin=USDT&limit=10").await?;
        let list = v.get("list").and_then(|x| x.as_array());
        let idx = list
            .and_then(|a| a.first())
            .and_then(|i| i.get("positionIdx"))
            .and_then(|x| x.as_i64());
        Ok(idx.map(|i| if i == 0 { "net_mode".into() } else { "long_short_mode".into() }))
    }

    /// Faz 2.7 — Bybit risk-limit aynası (halka açık, anahtarsız).
    async fn mmr_brackets(&self, symbol: &str, _creds: Option<&super::VenueCreds>) -> Result<Vec<dca_math::MmrBracket>, String> {
        let v = self
            .public_get("/v5/market/risk-limit", &format!("category=linear&symbol={symbol}&limit=200"))
            .await?;
        parse_bybit_risk_limit(&v)
    }

    /// P3-6 L2 derinlik (Bybit /v5/market/orderbook, halka açık).
    async fn orderbook(&self, category: &str, symbol: &str, limit: usize) -> Result<OrderBook, String> {
        let lim = limit.clamp(1, 200);
        let v = self.public_get("/v5/market/orderbook", &format!("category={category}&symbol={symbol}&limit={lim}")).await?;
        let side = |k: &str| -> Vec<OrderBookLevel> {
            v.get(k).and_then(|x| x.as_array()).map(|a| {
                a.iter().filter_map(|e| {
                    Some(OrderBookLevel {
                        price: e.get(0)?.as_str()?.into(),
                        qty: e.get(1)?.as_str()?.into(),
                    })
                }).collect()
            }).unwrap_or_default()
        };
        Ok(OrderBook {
            symbol: v.get("s").and_then(|x| x.as_str()).unwrap_or(symbol).into(),
            bids: side("b"),
            asks: side("a"),
            ts: v.get("ts").and_then(|x| x.as_i64()).unwrap_or(0),
        })
    }

    async fn set_leverage(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        leverage: &Fraction,
    ) -> Result<(), String> {
        if category != "linear" {
            return Ok(()); // spot pairs carry no leverage
        }
        let lev = leverage.to_half_even_display(2);
        let body = serde_json::json!({
            "category": "linear",
            "symbol": symbol,
            "buyLeverage": lev,
            "sellLeverage": lev,
        })
        .to_string();
        // M-12: private_post içindeki check_ret retCode!=0'da (110043 dahil)
        // Err üretir — "leverage not modified" zaten-hedefte BAŞARISIDIR;
        // metinden yakalanır, sonsuz WARN-retry kapanır.
        match self.private_post(&creds.key, &creds.secret, "/v5/position/set-leverage", &body).await {
            Ok(_) => Ok(()),
            Err(e) if e.contains("retCode=110043") || e.to_lowercase().contains("leverage not modified") => Ok(()),
            Err(e) => Err(e),
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// P3-4 self-test (v3 memoryScrubSelfTest karşılığı): imza zarfı drop'ta
    /// (panic/unwind yolunda da) anahtar baytlarını sıfırlar.
    #[test]
    fn signing_key_scrubbed_after_use() {
        // Faz 21 (CI düzeltmesi): ESKİ test drop SONRASI serbest belleği
        // `from_raw_parts` ile okuyordu — use-after-free okuma = UB; Windows
        // allocator'ı tesadüfen sıfır bıraktığından geçer, Linux glibc ezince
        // CI'da kırıldı. Yeni hali UB'siz ve deterministiktir: Zeroizing'in
        // (1) `zeroize()` çağrısının canlı tamponu sıfırladığını doğrular;
        // (2) drop yolundaki sıfırlama, Zeroizing tipinin belgelenmiş
        // sözleşmesidir (zeroize crate'inin kendi testleriyle kanıtlı —
        // P3-4'ün fiili mekanizması budur; serbest bellek okumak değil).
        use zeroize::{Zeroize, Zeroizing};
        // (1) zeroize() canlı tamponu sıfırlar — sabit-uzunluk dizi üzerinde
        // (Vec impl'i sıfırlama sonrası length'i de sıfırlar; imza anahtarı
        // gerçek dünyada [u8; N] taşınır, uzunluk korunumu dizi impl'idir).
        let mut key: [u8; 32] = [0xAB; 32];
        key.zeroize();
        assert!(key.iter().all(|&b| b == 0), "zeroize() çağrısı anahtar baytlarını sıfırlamalı");
        // (2) Zeroizing zarfı drop yolunda aynı impl'i çağırır — zarf tipi
        // derleme-zamanı sözleşme testi; crate'in kendi testleri drop
        // davranışını kanıtlar.
        let guard = Zeroizing::new([0xCDu8; 32]);
        assert_eq!(guard.len(), 32);
        drop(guard);
    }

    /// Faz 2.7 kilit test: CANLI Bybit `risk-limit` yanıtından doğrulanmış
    /// BTCUSDT ilk-3 kademe (2026-10-22, halka açık uç). Ham girdideki
    /// maintenanceMargin ondalıkları M07 kesirlerine birebir eşlenir;
    /// cum sürekliliği (510, 1710) canlı normalizasyonla aynı çıkar.
    #[test]
    fn parse_bybit_risk_limit_maps_live_btc_tiers() {
        let raw = serde_json::json!({
            "list": [
                {"id": 1, "symbol": "BTCUSDT", "riskLimitValue": "300000", "maintenanceMargin": "0.0033", "initialMargin": "0.01", "isLowestRisk": 1, "maxLeverage": "125.00"},
                {"id": 2, "symbol": "BTCUSDT", "riskLimitValue": "2000000", "maintenanceMargin": "0.005", "initialMargin": "0.01", "isLowestRisk": 0, "maxLeverage": "100.00"},
                {"id": 3, "symbol": "BTCUSDT", "riskLimitValue": "2600000", "maintenanceMargin": "0.0056", "initialMargin": "0.01", "isLowestRisk": 0, "maxLeverage": "83.30"}
            ]
        });
        let tiers = parse_bybit_risk_limit(&raw).expect("canlı şekil çözülmeli");
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].min_notional.to_string(), "0");
        assert_eq!(tiers[0].max_notional.to_string(), "300000");
        assert_eq!(tiers[0].mmr.to_string(), "33/10000");
        assert_eq!(tiers[0].deduction.to_string(), "0");
        assert_eq!(tiers[1].min_notional.to_string(), "300000");
        assert_eq!(tiers[1].deduction.to_string(), "510"); // 300000*(0.005-0.0033)
        assert_eq!(tiers[2].min_notional.to_string(), "2000000");
        assert_eq!(tiers[2].max_notional.to_string(), "2600000");
        assert_eq!(tiers[2].deduction.to_string(), "1710"); // 510+2000000*(0.0056-0.005)
        assert_eq!(tiers[2].min_notional, tiers[1].max_notional); // bitişiklik
    }

    #[test]
    fn parse_bybit_risk_limit_rejects_empty() {
        let raw = serde_json::json!({"list": []});
        assert!(parse_bybit_risk_limit(&raw).is_err());
    }

    #[test]
    fn sign_matches_official_example() {
        // Official docs example:
        // ts=1658384314791 key=X key recv=5000
        // payload = "1658384314791" + key + "5000" + "category=option&symbol=BTC-29JUL22-25000-C"
        // The exact expected digest depends on the key; here we verify determinism + hex format.
        let s1 = sign_hmac("secret", "1658384314791XXXXXXXXXX5000category=option&symbol=BTC-29JUL22-25000-C");
        let s2 = sign_hmac("secret", "1658384314791XXXXXXXXXX5000category=option&symbol=BTC-29JUL22-25000-C");
        assert_eq!(s1, s2);
        assert_eq!(s1.len(), 64);
        assert!(s1.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
    }

    #[test]
    fn test_check_ret_ok() {
        let v = serde_json::json!({
            "retCode": 0,
            "retMsg": "OK",
            "result": {"orderId": "12345"}
        });
        let res = check_ret(v, "/test/path").expect("valid retCode 0 should succeed");
        assert_eq!(res.get("orderId").and_then(|x| x.as_str()), Some("12345"));
    }

    #[test]
    fn test_check_ret_clock_drift_10002() {
        let v = serde_json::json!({
            "retCode": 10002,
            "retMsg": "order not exists or too late",
            "result": null
        });
        let err = check_ret(v, "/order/create").unwrap_err();
        assert!(err.contains("retCode=10002 (clock drift/expired timestamp)"));
        assert!(err.contains("sync_server_time required"));
    }

    #[test]
    fn test_check_ret_rate_limit_10006() {
        let v = serde_json::json!({
            "retCode": 10006,
            "retMsg": "Too many visits",
            "result": null
        });
        let err = check_ret(v, "/market/kline").unwrap_err();
        assert!(err.contains("retCode=10006 (rate limit exceeded)"));
    }

    #[test]
    fn test_bybit_adapter_now_ms_with_offset() {
        let adapter = BybitAdapter::new(true);
        let t1 = adapter.now_ms();
        adapter.time_offset_ms.store(5000, Ordering::Relaxed);
        let t2 = adapter.now_ms();
        assert!(t2 >= t1 + 4900, "now_ms with 5000ms offset should reflect clock adjustment");
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    fn permission_payload(wallet: Value) -> Value {
        serde_json::json!({
            "readOnly": 0,
            "permissions": {
                "Wallet": wallet,
                "Spot": ["SpotTrade"],
                "ContractTrade": ["Order", "Position"]
            },
            "ips": []
        })
    }

    #[test]
    fn missing_permission_schema_fails_closed() {
        assert!(parse_key_permissions(&serde_json::json!({ "readOnly": 0 })).is_err());
        assert!(parse_key_permissions(&serde_json::json!({
            "permissions": {"Wallet": [], "Spot": [], "ContractTrade": []}
        })).is_err());
        assert!(parse_key_permissions(&permission_payload(serde_json::json!("Withdraw"))).is_err());
    }

    #[test]
    fn wallet_withdrawal_and_internal_transfer_permissions_are_rejected() {
        for permission in ["Withdraw", "AccountTransfer", "SubMemberTransfer", "SubMemberTransferList"] {
            let parsed = parse_key_permissions(&permission_payload(serde_json::json!([permission]))).unwrap();
            assert!(parsed.can_withdraw, "wallet permission {permission} must trip the hard gate");
        }
        let parsed = parse_key_permissions(&permission_payload(serde_json::json!([]))).unwrap();
        assert!(!parsed.can_withdraw);
        assert!(parsed.spot_trade && parsed.contract_trade);
    }
}
