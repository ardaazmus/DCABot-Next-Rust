//! OKX V5 adapter (tertiary venue) — public market data, signed trade,
//! account snapshot, and the v3 "posMode tuzağı" pre-flight assertion.
//!
//! v3'teki `okx-adapter.ts` karşılığı (WS2.2): cross-research hedefi
//! "OKX posMode tuzağı" — hesap varsayılan net_mode'dayken DCA_SHORT açma
//! girişimi borrow olmadan reddedilir veya spot satışa dönüşür. Çözüm:
//! vadeli kaldıraç senkronu ÖNCESİ `get-position-mode` assertion'ı —
//! posMode != 'long_short_mode' ise set_leverage fail-closed hata döner,
//! motor da poll başına retry ile WARN düşer (sessiz işlem yok).
//!
//! İmza sözleşmesi (V5):
//!   OK-ACCESS-SIGN = Base64( HMAC_SHA256( timestamp + method + requestPath + body ) )
//!   OK-ACCESS-TIMESTAMP = ISO8601 milisaniyeli (…T09:08:57.715Z)
//!   OK-ACCESS-PASSPHRASE = anahtar oluşturulurken verilen parola
//! Sembol eşleme: BTCUSDT → spot `BTC-USDT`, linear `BTC-USDT-SWAP`.
//! Swap emir adedi kontrattır: contracts = floor(base_qty / ctVal, lotSz adımı)
//! (exact stringaritmetik — float YOK).

use super::{
    format_order_decimal, KeyPermissions, Kline, OrderBook, OrderBookLevel, OrderRequest,
    PositionSnapshot, Ticker24h, TickerSnapshot, VenueAdapter, VenueCreds, VenueOrderState,
    WalletSnapshot,
};
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

pub const MAINNET: &str = "https://www.okx.com";

/// v3 REQUIRED_POS_MODE karşılığı — vadeli hesap için zorunlu pozisyon modu.
pub const REQUIRED_POS_MODE: &str = "long_short_mode";

pub struct OkxAdapter {
    pub base_url: String,
    pub http: reqwest::Client,
    pub is_demo: bool,
}

impl OkxAdapter {
    pub fn new() -> Self {
        let is_demo = std::env::var("DCA_OKX_DEMO").as_deref() == Ok("1")
            || std::env::var("DCA_OKX_TESTNET").as_deref() == Ok("1");
        Self::with_demo(is_demo)
    }

    /// Explicit demo-mode adapter for server test fixtures.
    pub fn new_demo() -> Self {
        Self::with_demo(true)
    }

    fn with_demo(is_demo: bool) -> Self {
        let raw_base_url = std::env::var("OKX_BASE_URL")
            .or_else(|_| std::env::var("DCA_OKX_BASE_URL"))
            .unwrap_or_else(|_| MAINNET.into());
        let base_url = super::guarded_venue_base_url(
            &raw_base_url,
            "OKX",
            &["www.okx.com"],
            &["www.okx.com"],
            is_demo,
        );
        Self {
            base_url,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .expect("reqwest"),
            is_demo,
        }
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
            .send()
            .await
            .map_err(|e| format!("okx GET {path} failed: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        check_code(v, path)
    }

    fn timestamp() -> String {
        chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    }

    async fn private(
        &self,
        creds: &VenueCreds,
        method: &str,
        path: &str,
        query: &str,
        body: &str,
    ) -> Result<Value, String> {
        let pass = creds.passphrase.as_deref().unwrap_or("").trim();
        if pass.is_empty() {
            return Err(
                "okx: passphrase eksik — vault anahtarını passphrase ile yeniden ekleyin".into(),
            );
        }
        let request_path = if query.is_empty() {
            path.to_string()
        } else {
            format!("{path}?{query}")
        };
        let ts = Self::timestamp();
        let prehash = format!("{ts}{method}{request_path}{body}");
        let sign = sign_base64(&creds.secret, &prehash);
        let url = format!("{}{}", self.base_url, request_path);
        let mut req = if method == "POST" {
            self.http.post(&url).body(body.to_string())
        } else {
            self.http.get(&url)
        };
        if self.is_demo {
            req = req.header("x-simulated-trading", "1");
        }
        let resp = req
            .header("Content-Type", "application/json")
            .header("OK-ACCESS-KEY", creds.key.trim())
            .header("OK-ACCESS-SIGN", sign)
            .header("OK-ACCESS-TIMESTAMP", &ts)
            .header("OK-ACCESS-PASSPHRASE", pass)
            .send()
            .await
            .map_err(|e| format!("okx {method} {path} failed: {e}"))?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        check_code(v, path)
    }

    async fn private_get(
        &self,
        creds: &VenueCreds,
        path: &str,
        query: &str,
    ) -> Result<Value, String> {
        self.private(creds, "GET", path, query, "").await
    }

    async fn private_post(
        &self,
        creds: &VenueCreds,
        path: &str,
        body: &str,
    ) -> Result<Value, String> {
        self.private(creds, "POST", path, "", body).await
    }

    fn inst_spec(&self, v: &Value) -> Option<(String, String, String, Option<String>)> {
        // (tickSz, lotSz, minSz, ctVal?)
        Some((
            v.get("tickSz")?.as_str()?.to_string(),
            v.get("lotSz")?.as_str()?.to_string(),
            v.get("minSz")?.as_str()?.to_string(),
            v.get("ctVal").and_then(|x| x.as_str()).map(String::from),
        ))
    }
}

impl Default for OkxAdapter {
    fn default() -> Self {
        Self::new()
    }
}

/// OKX zarf denetimi: code "0" değilse dürüst hata (msg kısaltılır).
fn check_code(v: Value, path: &str) -> Result<Value, String> {
    let code = v.get("code").and_then(|x| x.as_str()).unwrap_or("-1");
    if code != "0" {
        let msg = v.get("msg").and_then(|x| x.as_str()).unwrap_or("unknown");
        return Err(format!(
            "okx {path} code={code}: {}",
            msg.chars().take(160).collect::<String>()
        ));
    }
    Ok(v.get("data").cloned().unwrap_or(Value::Null))
}

/// P3-4 deseni: secret Zeroizing zarfında, imza base64 (RFC 4648).
fn sign_base64(secret: &str, prehash: &str) -> String {
    use zeroize::Zeroizing;
    let key = Zeroizing::new(secret.as_bytes().to_vec());
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("hmac key");
    mac.update(prehash.as_bytes());
    base64_encode(&mac.finalize().into_bytes())
}

fn base64_encode(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(T[((n >> 18) & 63) as usize] as char);
        out.push(T[((n >> 12) & 63) as usize] as char);
        out.push(if chunk.len() > 1 {
            T[((n >> 6) & 63) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            T[(n & 63) as usize] as char
        } else {
            '='
        });
    }
    out
}

/// "BTCUSDT" → ("BTC", "USDT"). Quote sonekleri USDT/USDC/BTC/ETH.
fn split_symbol(symbol: &str) -> Result<(String, String), String> {
    let s = symbol.trim().to_uppercase();
    for q in ["USDT", "USDC", "BTC", "ETH"] {
        if s.len() > q.len() && s.ends_with(q) {
            let base = s[..s.len() - q.len()].to_string();
            if !base.is_empty() && base.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Ok((base, q.to_string()));
            }
        }
    }
    Err(format!("okx: sembol ayrıştırılamadı: {symbol}"))
}

/// category: "spot" → BTC-USDT, "linear" → BTC-USDT-SWAP.
fn okx_inst_id(symbol: &str, category: &str) -> Result<String, String> {
    let (base, quote) = split_symbol(symbol)?;
    Ok(if category == "linear" {
        format!("{base}-{quote}-SWAP")
    } else {
        format!("{base}-{quote}")
    })
}

/// Motor aralığı (bybit_interval çıktısı: "60", "D"…) → OKX bar adı.
pub fn okx_bar(interval: &str) -> Result<&'static str, String> {
    match interval {
        "1" => Ok("1m"),
        "3" => Ok("3m"),
        "5" => Ok("5m"),
        "15" => Ok("15m"),
        "30" => Ok("30m"),
        "60" => Ok("1H"),
        "120" => Ok("2H"),
        "240" => Ok("4H"),
        "D" => Ok("1D"),
        other => Err(format!("okx: desteklenmeyen aralık: {other}")),
    }
}

/// Exact floor(qty / step) * step — ondalık string aritmetiği (float YOK).
/// Adım 10^-d biçiminde değilse (örn. "3") i128 ölçekli bölmeyle tabanlanır;
/// taşma/çözümlenemezlikte Err (fail-closed — borsa reddi yerine erken hata).
fn floor_to_step(qty: &str, step: &str) -> Result<String, String> {
    let (qm, qs) = parse_decimal(qty).map_err(|e| format!("qty: {e}"))?;
    let (sm, ss) = parse_decimal(step).map_err(|e| format!("step: {e}"))?;
    if sm <= 0 {
        return Err("step must be positive".into());
    }
    // k = floor((qm * 10^ss) / (sm * 10^qs)); sonuç = k * step.
    let num = qm.checked_mul(pow10(ss)).ok_or("overflow")?;
    let den = sm.checked_mul(pow10(qs)).ok_or("overflow")?;
    if den <= 0 {
        return Err("non-positive step".into());
    }
    let k = num.div_euclid(den);
    let res = k.checked_mul(sm).ok_or("overflow")?;
    Ok(format_scaled(res, ss))
}

fn pow10(n: u32) -> i128 {
    let mut r: i128 = 1;
    for _ in 0..n {
        r = r.saturating_mul(10);
    }
    r
}

/// "12.340" → (12340, 3). Yalnız düz ondalık (işaret +/-, üs yok).
fn parse_decimal(s: &str) -> Result<(i128, u32), String> {
    let t = s.trim().trim_start_matches('+');
    let neg = t.starts_with('-');
    let t = t.trim_start_matches('-');
    let mut it = t.split('.');
    let int_p = it.next().unwrap_or("");
    let frac_p = it.next().unwrap_or("");
    if it.next().is_some() {
        return Err(format!("bad decimal: {s}"));
    }
    if int_p.is_empty() && frac_p.is_empty() {
        return Err(format!("bad decimal: {s}"));
    }
    if !int_p.chars().all(|c| c.is_ascii_digit()) || !frac_p.chars().all(|c| c.is_ascii_digit()) {
        return Err(format!("bad decimal: {s}"));
    }
    let digits = format!("{}{}", if int_p.is_empty() { "0" } else { int_p }, frac_p);
    let mut m: i128 = digits.parse().map_err(|_| format!("overflow: {s}"))?;
    if neg {
        m = -m;
    }
    Ok((m, frac_p.len() as u32))
}

/// Ölçekli tamsayıyı ondalık stringe çevir (scale basamak).
fn format_scaled(m: i128, scale: u32) -> String {
    if scale == 0 {
        return m.to_string();
    }
    let neg = m < 0;
    let d = m.unsigned_abs().to_string();
    let p = pow10(scale) as u128;
    let _ = p;
    let s = if d.len() <= scale as usize {
        format!("0.{:0>w$}", d, w = scale as usize)
    } else {
        let (i, f) = d.split_at(d.len() - scale as usize);
        format!("{i}.{f}")
    };
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

fn first_data(v: &Value) -> Option<&Value> {
    v.as_array()?.first()
}

fn normalize_contract_qty(qty: &Fraction, ct_val: &Fraction) -> Result<Fraction, String> {
    if !ct_val.is_positive() {
        return Err("okx: ctVal must be positive to normalize contract quantity".into());
    }
    qty.mul(ct_val)
        .map_err(|e| format!("okx contract quantity conversion failed: {e}"))
}

fn normalize_okx_fee(raw: Option<&str>) -> Option<Fraction> {
    raw.and_then(|value| Fraction::from_any_str(value).ok())
        .and_then(|fee| fee.neg().ok())
}

fn parse_key_permissions(v: &Value) -> Result<KeyPermissions, String> {
    let config = first_data(v).ok_or("okx account config: no data")?;
    let permission_text = config
        .get("perm")
        .and_then(Value::as_str)
        .ok_or("okx account config: permission field missing or malformed")?;
    if permission_text.trim().is_empty() {
        return Err("okx account config: empty permission field".into());
    }
    let permissions: Vec<&str> = permission_text.split(',').map(str::trim).collect();
    let can_withdraw = permissions.contains(&"withdraw");
    let has_trade = permissions.contains(&"trade");
    // OKX's `read_only` token means the key has Read permission; it does not
    // mean the key is restricted to reading. The Trade token is authoritative.
    let read_only = !has_trade;
    let ips = config
        .get("ip")
        .and_then(Value::as_str)
        .unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|ip| !ip.is_empty())
        .map(str::to_string)
        .collect();
    Ok(KeyPermissions {
        read_only,
        can_withdraw,
        spot_trade: has_trade,
        contract_trade: has_trade,
        ips,
        note: format!("okx account config permissions: read_only={read_only}, trade={has_trade}, withdraw={can_withdraw}"),
    })
}

#[async_trait::async_trait]
impl VenueAdapter for OkxAdapter {
    fn name(&self) -> &'static str {
        "OKX"
    }

    async fn server_time_ms(&self) -> Result<i64, String> {
        let v = self.public_get("/api/v5/public/time", "").await?;
        first_data(&v)
            .and_then(|d| d.get("ts"))
            .and_then(|t| {
                t.as_str()
                    .and_then(|s| s.parse::<i64>().ok())
                    .or_else(|| t.as_i64())
            })
            .filter(|t| *t > 0)
            .ok_or_else(|| "no time".into())
    }

    async fn instruments(
        &self,
        category: &str,
        symbol: Option<&str>,
    ) -> Result<Vec<Instrument>, String> {
        let inst_type = if category == "linear" { "SWAP" } else { "SPOT" };
        let mut query = format!("instType={inst_type}");
        if let Some(s) = symbol {
            query.push_str(&format!("&instId={}", okx_inst_id(s, category)?));
        }
        let v = self
            .public_get("/api/v5/public/instruments", &query)
            .await?;
        let mut out = Vec::new();
        if let Some(list) = v.as_array() {
            for item in list {
                let inst_id = item.get("instId").and_then(|x| x.as_str()).unwrap_or("");
                if inst_id.is_empty() {
                    continue;
                }
                let (tick, step, min_qty, ct_val) = match self.inst_spec(item) {
                    Some(t) => t,
                    None => continue,
                };
                let is_swap = category == "linear";
                let max_lev = if is_swap {
                    item.get("lever").and_then(|x| x.as_str()).unwrap_or("50")
                } else {
                    "1"
                };
                let (maker, taker) = if is_swap {
                    ("0.0002", "0.0005")
                } else {
                    ("0.0008", "0.001")
                };
                let our_symbol = if is_swap {
                    inst_id.trim_end_matches("-SWAP").replace('-', "")
                } else {
                    inst_id.replace('-', "")
                };
                out.push(Instrument {
                    symbol: our_symbol,
                    base: item
                        .get("baseCcy")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .into(),
                    quote: item
                        .get("quoteCcy")
                        .and_then(|x| x.as_str())
                        .unwrap_or("USDT")
                        .into(),
                    tick_size: Fraction::from_decimal_str(&tick).map_err(|e| e.to_string())?,
                    step_size: Fraction::from_decimal_str(&step).map_err(|e| e.to_string())?,
                    min_qty: Fraction::from_decimal_str(&min_qty).map_err(|e| e.to_string())?,
                    min_notional: Fraction::from_decimal_str("1").unwrap(),
                    maker_fee_rate: Fraction::from_decimal_str(maker).unwrap(),
                    taker_fee_rate: Fraction::from_decimal_str(taker).unwrap(),
                    max_leverage: Fraction::from_any_str(max_lev)
                        .unwrap_or_else(|_| Fraction::from_u64(1)),
                });
                // ctVal swap emir boyutunda kullanılır (place_order anlık okur).
                let _ = ct_val;
                if symbol.is_some() {
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn klines(
        &self,
        category: &str,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Kline>, String> {
        let inst_id = okx_inst_id(symbol, category)?;
        let bar = okx_bar(interval)?;
        // history-candles sayfa başına ≤100 — after imleciyle geriye sar.
        let mut out: Vec<Kline> = Vec::new();
        let mut after: Option<String> = None;
        let want = limit.min(1000).max(1);
        for _ in 0..10 {
            let need = (want - out.len()).min(100);
            if need == 0 {
                break;
            }
            let mut query = format!("instId={inst_id}&bar={bar}&limit={need}");
            if let Some(a) = &after {
                query.push_str(&format!("&after={a}"));
            }
            let v = self
                .public_get("/api/v5/market/history-candles", &query)
                .await?;
            let rows = v.as_array().cloned().unwrap_or_default();
            if rows.is_empty() {
                break;
            }
            for row in &rows {
                let arr = match row.as_array() {
                    Some(a) if a.len() >= 6 => a,
                    _ => continue,
                };
                let s = |i: usize| arr[i].as_str().unwrap_or("0").to_string();
                out.push(Kline {
                    t: arr[0]
                        .as_str()
                        .and_then(|x| x.parse::<i64>().ok())
                        .unwrap_or(0),
                    o: s(1),
                    h: s(2),
                    l: s(3),
                    c: s(4),
                    v: arr
                        .get(5)
                        .and_then(|x| x.as_str())
                        .unwrap_or("0")
                        .to_string(),
                });
            }
            if rows.len() < need {
                break;
            }
            if let Some(oldest) = out.last().map(|k| k.t) {
                after = Some(oldest.to_string());
            } else {
                break;
            }
            if out.len() >= want {
                break;
            }
        }
        out.truncate(want);
        // OKX sayfaları yeniden-eskidir; kronolojik artan sıraya çevir.
        out.sort_by_key(|k| k.t);
        Ok(out)
    }

    async fn ticker(&self, category: &str, symbol: &str) -> Result<TickerSnapshot, String> {
        let inst_id = okx_inst_id(symbol, category)?;
        let v = self
            .public_get("/api/v5/market/ticker", &format!("instId={inst_id}"))
            .await?;
        let d = first_data(&v).ok_or("no ticker")?;
        let last = d.get("last").and_then(|x| x.as_str()).unwrap_or("0");
        let funding = d
            .get("fundingRate")
            .and_then(|x| x.as_str())
            .and_then(|s| Fraction::from_decimal_str(s).ok());
        let next_ts = d
            .get("nextFundingTime")
            .and_then(|x| x.as_str())
            .and_then(|s| s.parse::<i64>().ok());
        Ok(TickerSnapshot {
            symbol: symbol.into(),
            last_price: Fraction::from_any_str(last).unwrap_or_else(|_| Fraction::zero()),
            mark_price: Fraction::from_any_str(last).unwrap_or_else(|_| Fraction::zero()),
            funding_rate: funding,
            next_funding_ts: next_ts,
        })
    }

    async fn tickers_24h(&self, category: &str) -> Result<Vec<Ticker24h>, String> {
        let inst_type = if category == "linear" { "SWAP" } else { "SPOT" };
        let v = self
            .public_get("/api/v5/market/tickers", &format!("instType={inst_type}"))
            .await?;
        let mut out = Vec::new();
        if let Some(list) = v.as_array() {
            for item in list {
                let inst_id = item.get("instId").and_then(|x| x.as_str()).unwrap_or("");
                if inst_id.is_empty() {
                    continue;
                }
                let s = |k: &str| {
                    item.get(k)
                        .and_then(|x| x.as_str())
                        .unwrap_or("0")
                        .to_string()
                };
                let (last, open) = (
                    s("last").parse::<f64>().unwrap_or(0.0),
                    s("open24h").parse::<f64>().unwrap_or(0.0),
                );
                let pct = if open > 0.0 {
                    (last - open) / open * 100.0
                } else {
                    0.0
                };
                let our = if category == "linear" {
                    inst_id.trim_end_matches("-SWAP").replace('-', "")
                } else {
                    inst_id.replace('-', "")
                };
                out.push(Ticker24h {
                    symbol: our,
                    last: s("last"),
                    change_pct_24h: format!("{pct:.4}"),
                    high_24h: s("high24h"),
                    low_24h: s("low24h"),
                    volume_24h: s("vol24h"),
                    turnover_24h: s("volCcy24h"),
                });
            }
        }
        Ok(out)
    }

    async fn funding_history(
        &self,
        symbol: &str,
        limit: usize,
    ) -> Result<Vec<(i64, String)>, String> {
        let inst_id = okx_inst_id(symbol, "linear")?;
        let v = self
            .public_get(
                "/api/v5/public/funding-rate-history",
                &format!("instId={inst_id}&limit={}", limit.min(100)),
            )
            .await?;
        let mut out = Vec::new();
        if let Some(list) = v.as_array() {
            for item in list {
                let ts = item
                    .get("fundingTime")
                    .and_then(|x| x.as_str())
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(0);
                let rate = item
                    .get("fundingRate")
                    .and_then(|x| x.as_str())
                    .unwrap_or("0")
                    .to_string();
                out.push((ts, rate));
            }
        }
        out.sort_by_key(|(t, _)| *t);
        Ok(out)
    }

    async fn orderbook(
        &self,
        category: &str,
        symbol: &str,
        limit: usize,
    ) -> Result<OrderBook, String> {
        let inst_id = okx_inst_id(symbol, category)?;
        let sz = limit.clamp(1, 400);
        let v = self
            .public_get("/api/v5/market/books", &format!("instId={inst_id}&sz={sz}"))
            .await?;
        let d = first_data(&v).ok_or("no book")?;
        let side = |k: &str| -> Vec<OrderBookLevel> {
            d.get(k)
                .and_then(|x| x.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|e| {
                            Some(OrderBookLevel {
                                price: e.get(0)?.as_str()?.into(),
                                qty: e.get(1)?.as_str()?.into(),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        Ok(OrderBook {
            symbol: symbol.into(),
            bids: side("bids"),
            asks: side("asks"),
            ts: d
                .get("ts")
                .and_then(|x| x.as_str())
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(0),
        })
    }

    /// OKX /api/v5/account/config exposes the current key's comma-separated
    /// permissions. Missing or malformed permission data must fail closed.
    async fn key_permissions(&self, creds: &VenueCreds) -> Result<KeyPermissions, String> {
        let v = self
            .private_get(creds, "/api/v5/account/config", "")
            .await?;
        parse_key_permissions(&v)
    }

    async fn wallet(&self, creds: &VenueCreds, coin: &str) -> Result<WalletSnapshot, String> {
        let v = self
            .private_get(creds, "/api/v5/account/balance", &format!("ccy={coin}"))
            .await?;
        let d = first_data(&v).ok_or("no account")?;
        let det = d
            .get("details")
            .and_then(|x| x.as_array())
            .and_then(|a| a.first())
            .ok_or("no ccy")?;
        let s = |k: &str| det.get(k).and_then(|x| x.as_str()).unwrap_or("0");
        Ok(WalletSnapshot {
            coin: det
                .get("ccy")
                .and_then(|x| x.as_str())
                .unwrap_or(coin)
                .to_string(),
            wallet_balance: Fraction::from_any_str(s("eq")).unwrap_or_else(|_| Fraction::zero()),
            available: Fraction::from_any_str(s("availBal")).unwrap_or_else(|_| Fraction::zero()),
            equity: Fraction::from_any_str(s("eq")).unwrap_or_else(|_| Fraction::zero()),
            unrealized_pnl: Fraction::from_any_str(
                det.get("upl").and_then(|x| x.as_str()).unwrap_or("0"),
            )
            .unwrap_or_else(|_| Fraction::zero()),
        })
    }

    async fn place_order(&self, creds: &VenueCreds, req: &OrderRequest) -> Result<String, String> {
        let inst_id = okx_inst_id(&req.symbol, &req.category)?;
        // EX-02: Spot market BUY varsayılan olarak quote tutarını kabul eder.
        // tgtCcy=base_ccy ile sz cinsinden (base coin) açıkça belirtilir.
        // EX-07: OKX long_short_mode gerektirir — posSide eklenir (long→long, short→short).
        let sz = if req.category == "linear" {
            let spec = self
                .public_get(
                    "/api/v5/public/instruments",
                    &format!("instType=SWAP&instId={inst_id}"),
                )
                .await?;
            let row = first_data(&spec).ok_or("swap spec yok")?;
            let (_, lot_sz, _, ct_val) = self.inst_spec(row).ok_or("swap spec eksik")?;
            let ct = ct_val.ok_or("ctVal yok")?;
            let contracts = Fraction::from_any_str(&format_order_decimal(&req.qty))
                .map_err(|e| e.to_string())?
                .div(&Fraction::from_decimal_str(&ct).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            floor_to_step(&format_order_decimal(&contracts), &lot_sz)?
        } else {
            // Spot: miktar base cinsindendir (EX-02 düzeltmesi).
            format_order_decimal(&req.qty)
        };
        let pos_side =
            resolve_okx_pos_side(&req.category, req.position_idx, req.side, req.reduce_only);
        let mut body = serde_json::json!({
            "instId": inst_id,
            "tdMode": if req.category == "linear" { "isolated" } else { "cash" },
            "side": match req.side { dca_math::Side::Buy => "buy", dca_math::Side::Sell => "sell" },
            "ordType": if req.is_market { "market" } else { "limit" },
            "sz": sz,
            "clOrdId": encode_okx_client_id(&req.order_link_id)?,
        });
        if let Some(ps) = pos_side {
            body["posSide"] = Value::String(ps.to_string());
        }
        if req.category == "spot" {
            body["tgtCcy"] = Value::String("base_ccy".into());
        }
        if !req.is_market {
            body["px"] = Value::String(
                req.price
                    .as_ref()
                    .map(format_order_decimal)
                    .unwrap_or_default(),
            );
        }
        if req.reduce_only {
            body["reduceOnly"] = Value::Bool(true);
        }
        let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        let v = self
            .private_post(creds, "/api/v5/trade/order", &body_str)
            .await?;
        let d = first_data(&v).ok_or("no order data")?;
        let scode = d.get("sCode").and_then(|x| x.as_str()).unwrap_or("-1");
        if scode != "0" {
            let msg = d.get("sMsg").and_then(|x| x.as_str()).unwrap_or("unknown");
            return Err(format!(
                "okx order sCode={scode}: {}",
                msg.chars().take(160).collect::<String>()
            ));
        }
        d.get("ordId")
            .and_then(|x| x.as_str())
            .map(String::from)
            .ok_or_else(|| "no ordId".into())
    }

    async fn cancel_order(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<(), String> {
        let inst_id = okx_inst_id(symbol, category)?;
        let body =
            serde_json::json!({"instId": inst_id, "clOrdId": encode_okx_client_id(order_link_id)?});
        let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        let v = self
            .private_post(creds, "/api/v5/trade/cancel-order", &body_str)
            .await?;
        let d = first_data(&v);
        let scode = d
            .and_then(|x| x.get("sCode"))
            .and_then(|x| x.as_str())
            .unwrap_or("-1");
        if scode == "0" {
            return Ok(());
        }
        // 51603 "Order does not exist" — zaten gitmiş/kapanmış; Bybit 110001
        // ve Binance -2011 ile aynı idempotent başarı sözleşmesi.
        if scode == "51603" {
            return Ok(());
        }
        Err(format!("okx cancel sCode={scode}"))
    }

    async fn open_orders(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
    ) -> Result<Vec<VenueOrderState>, String> {
        let inst_id = okx_inst_id(symbol, category)?;
        let inst_type = if category == "linear" { "SWAP" } else { "SPOT" };
        let v = self
            .private_get(
                creds,
                "/api/v5/trade/orders-pending",
                &format!("instType={inst_type}&instId={inst_id}"),
            )
            .await?;
        // Faz 4 (M-2): swap emirlerinde sz/accFillSz KONTRAT cinsindendir —
        // ctVal ile coin cinsine normalize edilir (ctVal okunamazsa ham bırakılır).
        let ct = if category == "linear" {
            self.instrument_ct_val(&inst_id).await?
        } else {
            Fraction::one()
        };
        if !ct.is_positive() {
            return Err("okx: ctVal must be positive for linear order normalization".into());
        }
        let norm = |x: Fraction| -> Result<Fraction, String> {
            if category == "linear" {
                normalize_contract_qty(&x, &ct)
            } else {
                Ok(x)
            }
        };
        let mut out = Vec::new();
        if let Some(list) = v.as_array() {
            for item in list {
                out.push(VenueOrderState {
                    order_id: item
                        .get("ordId")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .into(),
                    order_link_id: item
                        .get("clOrdId")
                        .and_then(|x| x.as_str())
                        .map(decode_okx_client_id)
                        .unwrap_or_default(),
                    symbol: symbol.into(),
                    side: match item.get("side").and_then(|x| x.as_str()).unwrap_or("") {
                        "buy" => "Buy".into(),
                        "sell" => "Sell".into(),
                        other => other.into(),
                    },
                    price: item
                        .get("px")
                        .and_then(|x| x.as_str())
                        .and_then(|s| Fraction::from_decimal_str(s).ok()),
                    qty: norm(
                        Fraction::from_any_str(
                            item.get("sz").and_then(|x| x.as_str()).unwrap_or("0"),
                        )
                        .unwrap_or_else(|_| Fraction::zero()),
                    )?,
                    cum_exec_qty: norm(
                        Fraction::from_any_str(
                            item.get("accFillSz")
                                .and_then(|x| x.as_str())
                                .unwrap_or("0"),
                        )
                        .unwrap_or_else(|_| Fraction::zero()),
                    )?,
                    cum_exec_fee: normalize_okx_fee(
                        item.get("accFillFee").and_then(|x| x.as_str()),
                    ),
                    status: map_okx_state(item.get("state").and_then(|x| x.as_str()).unwrap_or(""))
                        .into(),
                });
            }
        }
        Ok(out)
    }

    async fn query_order(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<Option<VenueOrderState>, String> {
        let inst_id = okx_inst_id(symbol, category)?;
        let venue_id = encode_okx_client_id(order_link_id)?;
        let v = self
            .private_get(
                creds,
                "/api/v5/trade/order",
                &format!("instId={inst_id}&clOrdId={venue_id}"),
            )
            .await?;
        let d = match first_data(&v) {
            Some(d) => d,
            None => return Ok(None),
        };
        let state = d.get("state").and_then(|x| x.as_str()).unwrap_or("");
        let avg = d
            .get("avgPx")
            .and_then(|x| x.as_str())
            .and_then(|s| Fraction::from_decimal_str(s).ok());
        let price = avg.or_else(|| {
            d.get("px")
                .and_then(|x| x.as_str())
                .and_then(|s| Fraction::from_decimal_str(s).ok())
        });
        // EX-08: swap (linear) kategorisinde accFillSz kontrat adedindedir;
        // ctVal ile coin cinsine dönüştürülür.
        let raw_qty = Fraction::from_any_str(d.get("sz").and_then(|x| x.as_str()).unwrap_or("0"))
            .unwrap_or_else(|_| Fraction::zero());
        let raw_fill =
            Fraction::from_any_str(d.get("accFillSz").and_then(|x| x.as_str()).unwrap_or("0"))
                .unwrap_or_else(|_| Fraction::zero());
        let (qty, cum_exec_qty) = if category == "linear" {
            // ctVal okunamıyorsa raw değer kullanılır (fail-safe:Contract boyutu bilinmiyor).
            let ct = self.instrument_ct_val(&inst_id).await?;
            (
                normalize_contract_qty(&raw_qty, &ct)?,
                normalize_contract_qty(&raw_fill, &ct)?,
            )
        } else {
            (raw_qty, raw_fill)
        };
        Ok(Some(VenueOrderState {
            cum_exec_fee: normalize_okx_fee(d.get("accFillFee").and_then(|x| x.as_str())),
            order_id: d.get("ordId").and_then(|x| x.as_str()).unwrap_or("").into(),
            order_link_id: d
                .get("clOrdId")
                .and_then(|x| x.as_str())
                .map(decode_okx_client_id)
                .unwrap_or_else(|| order_link_id.to_string()),
            symbol: symbol.into(),
            side: match d.get("side").and_then(|x| x.as_str()).unwrap_or("") {
                "buy" => "Buy".into(),
                "sell" => "Sell".into(),
                other => other.into(),
            },
            price,
            qty,
            cum_exec_qty,
            status: map_okx_state(state).into(),
        }))
    }

    async fn positions(
        &self,
        creds: &VenueCreds,
        symbol: &str,
    ) -> Result<Vec<PositionSnapshot>, String> {
        // Spot'ta pozisyon kavramı yok — boş (Binance ile aynı sözleşme).
        let inst_id = match okx_inst_id(symbol, "linear") {
            Ok(i) => i,
            Err(_) => return Ok(Vec::new()),
        };
        let v = self
            .private_get(
                creds,
                "/api/v5/account/positions",
                &format!("instId={inst_id}"),
            )
            .await?;
        let mut out = Vec::new();
        if let Some(list) = v.as_array() {
            let has_open_position = list.iter().any(|item| {
                item.get("pos")
                    .and_then(|x| x.as_str())
                    .is_some_and(|pos| !pos.is_empty() && pos != "0")
            });
            let ct_val = if has_open_position {
                Some(self.instrument_ct_val(&inst_id).await?)
            } else {
                None
            };
            for item in list {
                let pos = item.get("pos").and_then(|x| x.as_str()).unwrap_or("0");
                if pos == "0" || pos.is_empty() {
                    continue;
                }
                let side_raw = item
                    .get("posSide")
                    .and_then(|x| x.as_str())
                    .or_else(|| item.get("side").and_then(|x| x.as_str()))
                    .unwrap_or("net");
                let side = match side_raw {
                    "long" => "Buy",
                    "short" => "Sell",
                    _ => {
                        if pos.trim_start_matches('-').is_empty() || pos.starts_with('-') {
                            "Sell"
                        } else {
                            "Buy"
                        }
                    }
                };
                let f = |k: &str| {
                    Fraction::from_any_str(item.get(k).and_then(|x| x.as_str()).unwrap_or("0"))
                        .unwrap_or_else(|_| Fraction::zero())
                };
                out.push(PositionSnapshot {
                    symbol: symbol.into(),
                    side: side.into(),
                    position_idx: None,
                    size: {
                        let raw = f("pos");
                        let absolute = if raw.is_negative() {
                            raw.neg().unwrap_or_else(|_| Fraction::zero())
                        } else {
                            raw
                        };
                        let ct = ct_val
                            .as_ref()
                            .ok_or("okx ctVal missing for open position")?;
                        normalize_contract_qty(&absolute, ct)?
                    },
                    avg_price: f("avgPx"),
                    mark_price: f("markPx"),
                    unrealized_pnl: f("upl"),
                    liq_price: item
                        .get("liqPx")
                        .and_then(|x| x.as_str())
                        .and_then(|s| Fraction::from_decimal_str(s).ok()),
                    leverage: f("lever"),
                });
            }
        }
        Ok(out)
    }

    /// Salt-okunur posMode sorgusu (Faz 1.3 multi-status): anahtar varsa
    /// `long_short_mode` assertion'ı raporlanır; anahtar yoksa çağrılmaz.
    async fn pos_mode(&self, creds: &VenueCreds) -> Result<Option<String>, String> {
        let pm = self
            .private_get(creds, "/api/v5/account/get-position-mode", "")
            .await?;
        Ok(first_data(&pm)
            .and_then(|d| d.get("posMode"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()))
    }
    /// v3 posMode tuzağı pre-flight assertion: net_mode hesapta vadeli
    /// kaldıraç senkronu fail-closed reddedilir (Türkçe operasyonel mesaj).
    async fn set_leverage(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        leverage: &Fraction,
    ) -> Result<(), String> {
        if category != "linear" {
            return Ok(());
        }
        let pm = self
            .private_get(creds, "/api/v5/account/get-position-mode", "")
            .await?;
        let mode = first_data(&pm)
            .and_then(|d| d.get("posMode"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        if mode != REQUIRED_POS_MODE {
            return Err(format!(
                "okx posMode tuzağı: hesap '{mode}' modunda — vadeli için long_short_mode gerekli (net_mode DCA_SHORT'u reddeder veya spot satışa çevirir). OKX web/API üzerinden pozisyon modunu değiştirin, sonra tekrar deneyin."
            ));
        }
        let inst_id = okx_inst_id(symbol, category)?;
        let body = serde_json::json!({
            "instId": inst_id,
            "lever": leverage.to_half_even_display(0),
            "mgnMode": "isolated",
        });
        let body_str = serde_json::to_string(&body).map_err(|e| e.to_string())?;
        self.private_post(creds, "/api/v5/account/set-leverage", &body_str)
            .await?;
        Ok(())
    }
}

/// OKX v5 Linear vadeli emirlerinde posSide eşleme kuralı (P0 Düzeltmesi).
/// Long pozisyon (1) için hem alış hem kapanış satışı "long" gönderilmelidir.
/// Short pozisyon (2) için hem satış hem kapanış alışı "short" gönderilmelidir.
pub fn resolve_okx_pos_side(
    category: &str,
    position_idx: i32,
    side: dca_math::Side,
    reduce_only: bool,
) -> Option<&'static str> {
    if category != "linear" {
        return None;
    }
    match position_idx {
        1 => Some("long"),
        2 => Some("short"),
        _ => {
            if reduce_only {
                match side {
                    dca_math::Side::Sell => Some("long"),
                    dca_math::Side::Buy => Some("short"),
                }
            } else {
                match side {
                    dca_math::Side::Buy => Some("long"),
                    dca_math::Side::Sell => Some("short"),
                }
            }
        }
    }
}
impl OkxAdapter {
    async fn instrument_ct_val(&self, inst_id: &str) -> Result<Fraction, String> {
        let v = self
            .public_get(
                "/api/v5/public/instruments",
                &format!("instType=SWAP&instId={inst_id}"),
            )
            .await?;
        let row = first_data(&v).ok_or("okx instrument spec yok")?;
        let ct_str = row
            .get("ctVal")
            .and_then(|x| x.as_str())
            .ok_or("okx ctVal alanı yok")?;
        let ct_val =
            Fraction::from_decimal_str(ct_str).map_err(|e| format!("okx ctVal parse: {e}"))?;
        if !ct_val.is_positive() {
            return Err("okx ctVal must be positive".into());
        }
        Ok(ct_val)
    }
}

/// OKX `clOrdId` accepts only alphanumeric characters (max 32), while the
/// engine's stable order IDs contain separators. Bot IDs are `b-` followed by
/// an eight-character hex ID; map that prefix to `D`, dashes to reserved `z`,
/// and underscores to reserved `y`, so the venue ID remains reversible. The
/// long `GRID_CLOSE` role is compacted to stay within OKX's limit.
fn encode_okx_client_id(internal: &str) -> Result<String, String> {
    let body = internal
        .strip_prefix("b-")
        .ok_or_else(|| "OKX clOrdId: engine order ID must start with b-".to_string())?;
    if body.len() < 8 || !body.as_bytes()[..8].iter().all(u8::is_ascii_hexdigit) {
        return Err("OKX clOrdId: bot ID must contain eight hex characters".into());
    }
    if body
        .bytes()
        .any(|c| !(c.is_ascii_alphanumeric() || c == b'-' || c == b'_') || c == b'y' || c == b'z')
    {
        return Err("OKX clOrdId: unsupported character in engine order ID".into());
    }
    let compact = body
        .replace("-GRID_CLOSE-", "zXz")
        .replace('-', "z")
        .replace('_', "y");
    let mut venue_id = format!("D{compact}");
    if venue_id.len() > 32 {
        let (prefix, sequence) = body
            .rsplit_once('-')
            .ok_or_else(|| "OKX clOrdId exceeds the 32-character venue limit".to_string())?;
        let sequence = sequence
            .parse::<u64>()
            .map_err(|_| "OKX clOrdId exceeds the 32-character venue limit".to_string())?;
        let compact_sequence = format!("{prefix}-Z{}", encode_base36(sequence));
        let shortened = compact_sequence
            .replace("-GRID_CLOSE-", "zXz")
            .replace('-', "z")
            .replace('_', "y");
        venue_id = format!("D{shortened}");
    }
    if venue_id.len() > 32 {
        return Err("OKX clOrdId exceeds the 32-character venue limit".into());
    }
    Ok(venue_id)
}

/// Reverse IDs emitted by `encode_okx_client_id`. Unrecognized client IDs are
/// left untouched so unrelated account orders cannot be mistaken for ours.
fn decode_okx_client_id(venue_id: &str) -> String {
    let Some(body) = venue_id.strip_prefix('D') else {
        return venue_id.into();
    };
    if body.len() < 8 || !body.as_bytes()[..8].iter().all(u8::is_ascii_hexdigit) {
        return venue_id.into();
    }
    let decoded = if let Some((prefix, sequence)) = body.rsplit_once("zZ") {
        match decode_base36(sequence) {
            Some(value) => format!("{prefix}z{value}"),
            None => body.to_string(),
        }
    } else {
        body.to_string()
    };
    let internal_body = decoded
        .replace("zXz", "-GRID_CLOSE-")
        .replace('z', "-")
        .replace('y', "_");
    format!("b-{internal_body}")
}

fn encode_base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".into();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base36 digits are ASCII")
}

fn decode_base36(value: &str) -> Option<u64> {
    if value.is_empty() {
        return None;
    }
    value.bytes().try_fold(0u64, |acc, digit| {
        let n = match digit {
            b'0'..=b'9' => (digit - b'0') as u64,
            b'a'..=b'z' => (digit - b'a' + 10) as u64,
            _ => return None,
        };
        acc.checked_mul(36)?.checked_add(n)
    })
}

fn map_okx_state(s: &str) -> &'static str {
    match s {
        "live" => "New",
        "partially_filled" => "PartiallyFilled",
        "filled" => "Filled",
        "canceled" | "mmp_canceled" => "Cancelled",
        _ => "New",
    }
}

#[cfg(test)]
mod okx_tests {
    use super::*;

    #[test]
    fn base64_matches_rfc4648_vector() {
        // "foobar" → "Zm9vYmFy" (RFC 4648 §10).
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
    }

    #[test]
    fn symbol_mapping_spot_and_swap() {
        assert_eq!(okx_inst_id("BTCUSDT", "spot").unwrap(), "BTC-USDT");
        assert_eq!(okx_inst_id("BTCUSDT", "linear").unwrap(), "BTC-USDT-SWAP");
        assert_eq!(okx_inst_id("ETHUSDC", "spot").unwrap(), "ETH-USDC");
        assert!(
            okx_inst_id("XYZ", "spot").is_err(),
            "quote'suz sembol reddedilir"
        );
    }

    #[test]
    fn bar_mapping_matches_engine_tokens() {
        assert_eq!(okx_bar("60").unwrap(), "1H");
        assert_eq!(okx_bar("D").unwrap(), "1D");
        assert!(okx_bar("999").is_err());
    }

    #[test]
    fn floor_to_step_exact_cases() {
        assert_eq!(floor_to_step("10.567", "0.1").unwrap(), "10.5");
        assert_eq!(floor_to_step("10.567", "1").unwrap(), "10");
        assert_eq!(floor_to_step("0.00012345", "0.0001").unwrap(), "0.0001");
        assert_eq!(floor_to_step("5", "10").unwrap(), "0");
        assert!(floor_to_step("abc", "0.1").is_err());
        assert!(floor_to_step("10", "0").is_err());
    }

    #[test]
    fn instrument_spec_parsed_from_fixture() {
        let v: Value = serde_json::from_str(
            r#"{"instId":"BTC-USDT-SWAP","baseCcy":"BTC","quoteCcy":"USDT","tickSz":"0.1","lotSz":"1","minSz":"1","ctVal":"0.01","lever":"125"}"#,
        )
        .unwrap();
        let a = OkxAdapter::new();
        let (tick, lot, min, ct) = a.inst_spec(&v).unwrap();
        assert_eq!(
            (tick.as_str(), lot.as_str(), min.as_str()),
            ("0.1", "1", "1")
        );
        assert_eq!(ct.as_deref(), Some("0.01"));
    }

    #[test]
    fn swap_contracts_convert_to_coin_and_okx_fee_sign_is_reversed() {
        assert_eq!(
            normalize_contract_qty(
                &Fraction::from_u64(10_000),
                &Fraction::from_decimal_str("0.01").unwrap()
            )
            .unwrap()
            .to_string(),
            "100"
        );
        assert_eq!(normalize_okx_fee(Some("-0.25")).unwrap().to_string(), "1/4");
        assert_eq!(normalize_okx_fee(Some("0.1")).unwrap().to_string(), "-1/10");
        assert!(normalize_contract_qty(&Fraction::one(), &Fraction::zero()).is_err());
    }

    #[test]
    fn okx_client_order_ids_are_alphanumeric_reversible_and_bounded() {
        let ids = [
            "b-0123abcd-1-TPL-0",
            "b-0123abcd-1-GRID_CLOSE-4294967296",
            "b-0123abcd-hedge-main-7-BASE-0",
        ];
        for internal in ids {
            let venue = encode_okx_client_id(internal).unwrap();
            assert!(venue.len() <= 32);
            assert!(venue.bytes().all(|c| c.is_ascii_alphanumeric()));
            assert_eq!(decode_okx_client_id(&venue), internal);
        }
        assert_eq!(decode_okx_client_id("OTHER1234zXz"), "OTHER1234zXz");
        assert!(
            encode_okx_client_id("b-0123abcd-1-GRID_CLOSE-4294967296")
                .unwrap()
                .len()
                <= 32
        );
        let max_seq = "b-0123abcd-1-GRID_CLOSE-18446744073709551615";
        let encoded = encode_okx_client_id(max_seq).unwrap();
        assert!(encoded.len() <= 32);
        assert_eq!(decode_okx_client_id(&encoded), max_seq);
        assert!(encode_okx_client_id("b-0123abcd-1-z_ROLE-0").is_err());
    }

    #[test]
    fn state_mapping_covers_okx_lifecycle() {
        assert_eq!(map_okx_state("live"), "New");
        assert_eq!(map_okx_state("partially_filled"), "PartiallyFilled");
        assert_eq!(map_okx_state("filled"), "Filled");
        assert_eq!(map_okx_state("canceled"), "Cancelled");
    }

    #[test]
    fn test_okx_pos_side_resolution() {
        // Spot için posSide olmamalı
        assert_eq!(
            resolve_okx_pos_side("spot", 1, dca_math::Side::Buy, false),
            None
        );

        // Linear Long bacağı (idx 1): hem açılış (Buy) hem kapanış (Sell) posSide="long"
        assert_eq!(
            resolve_okx_pos_side("linear", 1, dca_math::Side::Buy, false),
            Some("long")
        );
        assert_eq!(
            resolve_okx_pos_side("linear", 1, dca_math::Side::Sell, true),
            Some("long")
        );

        // Linear Short bacağı (idx 2): hem açılış (Sell) hem kapanış (Buy) posSide="short"
        assert_eq!(
            resolve_okx_pos_side("linear", 2, dca_math::Side::Sell, false),
            Some("short")
        );
        assert_eq!(
            resolve_okx_pos_side("linear", 2, dca_math::Side::Buy, true),
            Some("short")
        );

        // Net / One-Way mod (idx 0):
        // Kapanışta: Long kapatan Sell -> "long", Short kapatan Buy -> "short"
        assert_eq!(
            resolve_okx_pos_side("linear", 0, dca_math::Side::Sell, true),
            Some("long")
        );
        assert_eq!(
            resolve_okx_pos_side("linear", 0, dca_math::Side::Buy, true),
            Some("short")
        );
        // Açılışta: Buy -> "long", Sell -> "short"
        assert_eq!(
            resolve_okx_pos_side("linear", 0, dca_math::Side::Buy, false),
            Some("long")
        );
        assert_eq!(
            resolve_okx_pos_side("linear", 0, dca_math::Side::Sell, false),
            Some("short")
        );
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn account_config_permissions_are_parsed_from_the_unwrapped_data_array() {
        let parsed = parse_key_permissions(&serde_json::json!([
            {"perm": "read_only,trade", "ip": "192.0.2.1, 2001:db8::1"}
        ]))
        .unwrap();
        assert!(!parsed.read_only);
        assert!(!parsed.can_withdraw);
        assert!(parsed.spot_trade && parsed.contract_trade);
        assert_eq!(parsed.ips, vec!["192.0.2.1", "2001:db8::1"]);
    }

    #[test]
    fn missing_or_malformed_account_permission_data_fails_closed() {
        assert!(parse_key_permissions(&serde_json::json!([])).is_err());
        assert!(parse_key_permissions(&serde_json::json!([{"perm": ""}])).is_err());
        assert!(parse_key_permissions(&serde_json::json!([{"perm": true}])).is_err());
        let parsed =
            parse_key_permissions(&serde_json::json!([{"perm": "read_only,withdraw"}])).unwrap();
        assert!(parsed.read_only && parsed.can_withdraw);
        assert!(!parsed.spot_trade && !parsed.contract_trade);
    }
}
