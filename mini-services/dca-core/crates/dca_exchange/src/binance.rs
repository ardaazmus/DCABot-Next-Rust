//! Binance spot adapter (secondary venue) — exchangeInfo, klines, signed trade.

use super::{
    KeyPermissions, Kline, OrderBook, OrderBookLevel, OrderRequest, PositionSnapshot, Ticker24h,
    TickerSnapshot, VenueAdapter, VenueCreds, VenueOrderState, WalletSnapshot,
};
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use hmac::{Hmac, Mac};
use serde_json::Value;
use sha2::Sha256;

pub const MAINNET: &str = "https://api.binance.com";
/// Task 20 B1 — testnet base (DCA_BINANCE_TESTNET=1 ile aktif): LIVE gate
/// kaldırılmadan önce fill-reconciliation'ın gerçek anahtarlarla doğrulanacağı
/// ortam (Bybit DCA_BYBIT_TESTNET deseniyle aynı).
pub const TESTNET: &str = "https://testnet.binance.vision";

fn parse_account_restrictions(v: &Value) -> Result<KeyPermissions, String> {
    let required_bool = |field: &str| {
        v.get(field)
            .and_then(Value::as_bool)
            .ok_or_else(|| format!("binance permission response missing boolean '{field}'"))
    };
    let can_withdraw = required_bool("enableWithdrawals")?;
    let spot_trade = required_bool("enableSpotAndMarginTrading")?;
    let contract_trade = required_bool("enableFutures")?;
    let can_read = required_bool("enableReading")?;
    let ip_restrict = required_bool("ipRestrict")?;
    let read_only = can_read && !spot_trade && !contract_trade;
    let ips = if ip_restrict {
        vec!["IP_RESTRICTED".into()]
    } else {
        vec![]
    };
    Ok(KeyPermissions {
        read_only,
        can_withdraw,
        spot_trade,
        contract_trade,
        ips,
        note: format!("binance /sapi/v1/account/apiRestrictions: can_withdraw={can_withdraw}, spot={spot_trade}, futures={contract_trade}"),
    })
}

pub struct BinanceAdapter {
    pub base_url: String,
    pub http: reqwest::Client,
}

impl BinanceAdapter {
    pub fn new() -> Self {
        let raw_base = if let Ok(u) =
            std::env::var("BINANCE_SPOT_BASE").or_else(|_| std::env::var("DCA_BINANCE_BASE_URL"))
        {
            u
        } else if std::env::var("DCA_BINANCE_TESTNET").as_deref() == Ok("1") {
            TESTNET.to_string()
        } else {
            MAINNET.to_string()
        };
        Self::with_spot_base(&raw_base)
    }

    /// Explicit safe constructor for tests that create the full server state.
    pub fn new_testnet() -> Self {
        Self::with_spot_base(TESTNET)
    }

    fn with_spot_base(raw_base: &str) -> Self {
        let base = super::guarded_venue_base_url(
            raw_base,
            "Binance Spot",
            &["api.binance.com", "testnet.binance.vision"],
            &["api.binance.com"],
            false,
        );
        Self {
            base_url: base,
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .redirect(reqwest::redirect::Policy::none())
                .no_proxy()
                .build()
                .expect("reqwest"),
        }
    }

    fn signed_query(&self, params: &str, secret: &str) -> String {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        sign_query(params, secret, ts)
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
            .map_err(|e| e.to_string())?;
        resp.json().await.map_err(|e| e.to_string())
    }
}

/// Pure (testable) signed-query builder for every private Binance endpoint:
/// `{params}&timestamp={ts}&signature={HMAC-SHA256(secret, payload) upper-hex}`.
pub fn sign_query(params: &str, secret: &str, ts: u128) -> String {
    use zeroize::Zeroizing;
    let payload = format!("{params}&timestamp={ts}");
    let key = Zeroizing::new(secret.as_bytes().to_vec());
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).expect("hmac");
    mac.update(payload.as_bytes());
    let sig = hex::encode_upper(mac.finalize().into_bytes());
    format!("{payload}&signature={sig}")
}

impl Default for BinanceAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl VenueAdapter for BinanceAdapter {
    fn name(&self) -> &'static str {
        "BINANCE"
    }

    async fn server_time_ms(&self) -> Result<i64, String> {
        let v = self.public_get("/api/v3/time", "").await?;
        v.get("serverTime")
            .and_then(|x| x.as_i64())
            .ok_or_else(|| "no time".into())
    }

    async fn instruments(
        &self,
        _category: &str,
        symbol: Option<&str>,
    ) -> Result<Vec<Instrument>, String> {
        // Faz 1.1: sembol verilmişse sunucu-taraflı filtrele — tam liste MB'lerce
        // (yavaş hatlarda 15sn istemci zaman aşımını aşırır); şekil aynı döner.
        let query = symbol.map(|s| format!("symbol={s}")).unwrap_or_default();
        let v = self.public_get("/api/v3/exchangeInfo", &query).await?;
        let mut out = Vec::new();
        if let Some(list) = v.get("symbols").and_then(|x| x.as_array()) {
            for item in list {
                let sym = item.get("symbol").and_then(|x| x.as_str()).unwrap_or("");
                if let Some(want) = symbol {
                    if sym != want {
                        continue;
                    }
                }
                if item.get("status").and_then(|x| x.as_str()) != Some("TRADING") {
                    continue;
                }
                let (tick, step, min_qty, min_notional) = (
                    item.get("filters")
                        .and_then(|f| f.as_array())
                        .and_then(|f| {
                            f.iter()
                                .find(|x| {
                                    x.get("filterType").and_then(|t| t.as_str())
                                        == Some("PRICE_FILTER")
                                })
                                .and_then(|x| {
                                    x.get("tickSize").and_then(|t| t.as_str()).map(String::from)
                                })
                        })
                        .unwrap_or_else(|| "0.01".into()),
                    item.get("filters")
                        .and_then(|f| f.as_array())
                        .and_then(|f| {
                            f.iter()
                                .find(|x| {
                                    x.get("filterType").and_then(|t| t.as_str()) == Some("LOT_SIZE")
                                })
                                .and_then(|x| {
                                    x.get("stepSize").and_then(|t| t.as_str()).map(String::from)
                                })
                        })
                        .unwrap_or_else(|| "0.00001".into()),
                    item.get("filters")
                        .and_then(|f| f.as_array())
                        .and_then(|f| {
                            f.iter()
                                .find(|x| {
                                    x.get("filterType").and_then(|t| t.as_str()) == Some("LOT_SIZE")
                                })
                                .and_then(|x| {
                                    x.get("minQty").and_then(|t| t.as_str()).map(String::from)
                                })
                        })
                        .unwrap_or_else(|| "0.00001".into()),
                    item.get("filters")
                        .and_then(|f| f.as_array())
                        .and_then(|f| {
                            f.iter()
                                .find(|x| {
                                    x.get("filterType").and_then(|t| t.as_str()) == Some("NOTIONAL")
                                })
                                .and_then(|x| {
                                    x.get("minNotional")
                                        .and_then(|t| t.as_str())
                                        .map(String::from)
                                })
                        })
                        .unwrap_or_else(|| "5".into()),
                );
                out.push(Instrument {
                    symbol: sym.to_string(),
                    base: item
                        .get("baseAsset")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .into(),
                    quote: item
                        .get("quoteAsset")
                        .and_then(|x| x.as_str())
                        .unwrap_or("USDT")
                        .into(),
                    tick_size: Fraction::from_decimal_str(&tick)
                        .unwrap_or_else(|_| Fraction::from_decimal_str("0.01").unwrap()),
                    step_size: Fraction::from_decimal_str(&step)
                        .unwrap_or_else(|_| Fraction::from_decimal_str("0.00001").unwrap()),
                    min_qty: Fraction::from_decimal_str(&min_qty)
                        .unwrap_or_else(|_| Fraction::from_decimal_str("0.00001").unwrap()),
                    min_notional: Fraction::from_decimal_str(&min_notional)
                        .unwrap_or_else(|_| Fraction::from_u64(5)),
                    maker_fee_rate: Fraction::from_decimal_str("0.001").unwrap(),
                    taker_fee_rate: Fraction::from_decimal_str("0.001").unwrap(),
                    max_leverage: Fraction::one(),
                });
                if symbol.is_some() {
                    break;
                }
            }
        }
        Ok(out)
    }

    async fn klines(
        &self,
        _category: &str,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Kline>, String> {
        let iv = match interval {
            "1" | "3" | "5" | "15" | "30" => format!("{}m", interval),
            "60" => "1h".into(),
            "120" => "2h".into(),
            "240" => "4h".into(),
            "D" => "1d".into(),
            _ => "1h".into(),
        };
        let query = format!("symbol={symbol}&interval={iv}&limit={}", limit.min(1000));
        let v = self.public_get("/api/v3/klines", &query).await?;
        let mut out = Vec::new();
        if let Some(rows) = v.as_array() {
            for row in rows {
                if let Some(arr) = row.as_array() {
                    if arr.len() >= 6 {
                        out.push(Kline {
                            t: arr[0].as_i64().unwrap_or(0),
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
        Ok(out)
    }

    async fn ticker(&self, _category: &str, symbol: &str) -> Result<TickerSnapshot, String> {
        let v = self
            .public_get("/api/v3/ticker/price", &format!("symbol={symbol}"))
            .await?;
        let p = v.get("price").and_then(|x| x.as_str()).unwrap_or("0");
        Ok(TickerSnapshot {
            symbol: symbol.into(),
            last_price: Fraction::from_any_str(p).unwrap_or_else(|_| Fraction::zero()),
            mark_price: Fraction::from_any_str(p).unwrap_or_else(|_| Fraction::zero()),
            funding_rate: None,
            next_funding_ts: None,
        })
    }

    async fn tickers_24h(&self, _category: &str) -> Result<Vec<Ticker24h>, String> {
        let v = self.public_get("/api/v3/ticker/24hr", "").await?;
        let arr: Vec<serde_json::Value> = if let Some(a) = v.as_array() {
            a.clone()
        } else {
            vec![v]
        };
        Ok(arr
            .iter()
            .map(|item| {
                let s = |k: &str| {
                    item.get(k)
                        .and_then(|x| x.as_str())
                        .unwrap_or("0")
                        .to_string()
                };
                Ticker24h {
                    symbol: s("symbol"),
                    last: s("lastPrice"),
                    change_pct_24h: s("priceChangePercent"),
                    high_24h: s("highPrice"),
                    low_24h: s("lowPrice"),
                    volume_24h: s("volume"),
                    turnover_24h: s("quoteVolume"),
                }
            })
            .collect())
    }

    async fn funding_history(
        &self,
        _symbol: &str,
        _limit: usize,
    ) -> Result<Vec<(i64, String)>, String> {
        Ok(Vec::new()) // spot venue: no funding
    }

    async fn key_permissions(&self, creds: &VenueCreds) -> Result<KeyPermissions, String> {
        // This is the authoritative endpoint for withdrawal and trade flags.
        // Missing fields or an unsupported testnet endpoint must fail closed.
        let q = self.signed_query("recvWindow=5000", &creds.secret);
        let sapi_url = format!("{}/sapi/v1/account/apiRestrictions?{}", self.base_url, q);
        let resp = self
            .http
            .get(&sapi_url)
            .header("X-MBX-APIKEY", &creds.key)
            .send()
            .await
            .map_err(|e| {
                format!(
                    "binance permission check network failure: {}",
                    e.without_url()
                )
            })?;

        if !resp.status().is_success() {
            return Err(format!(
                "binance permission check failed ({})",
                resp.status()
            ));
        }

        let v: Value = resp
            .json()
            .await
            .map_err(|e| format!("binance response parse error: {e}"))?;
        parse_account_restrictions(&v)
    }

    async fn wallet(&self, creds: &VenueCreds, coin: &str) -> Result<WalletSnapshot, String> {
        let q = self.signed_query("recvWindow=5000", &creds.secret);
        let url = format!("{}/api/v3/account?{}", self.base_url, q);
        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &creds.key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        let balances = v
            .get("balances")
            .and_then(|x| x.as_array())
            .ok_or("no balances")?;
        let target = balances
            .iter()
            .find(|c| c.get("asset").and_then(|x| x.as_str()) == Some(coin))
            .ok_or("coin not found")?;
        Ok(WalletSnapshot {
            coin: coin.into(),
            wallet_balance: Fraction::from_any_str(
                target.get("free").and_then(|x| x.as_str()).unwrap_or("0"),
            )
            .unwrap_or_else(|_| Fraction::zero()),
            available: Fraction::from_any_str(
                target.get("free").and_then(|x| x.as_str()).unwrap_or("0"),
            )
            .unwrap_or_else(|_| Fraction::zero()),
            equity: Fraction::from_any_str(
                target.get("free").and_then(|x| x.as_str()).unwrap_or("0"),
            )
            .unwrap_or_else(|_| Fraction::zero()),
            unrealized_pnl: Fraction::zero(),
        })
    }

    async fn place_order(&self, creds: &VenueCreds, req: &OrderRequest) -> Result<String, String> {
        let mut params = format!(
            "symbol={}&side={}&type={}&quantity={}&newOrderRespType=RESULT&newClientOrderId={}",
            req.symbol,
            match req.side {
                dca_math::Side::Buy => "BUY",
                dca_math::Side::Sell => "SELL",
            },
            if req.is_market { "MARKET" } else { "LIMIT" },
            req.qty.to_half_even_display(8),
            req.order_link_id
        );
        if !req.is_market {
            params.push_str(&format!(
                "&price={}&timeInForce=GTC",
                req.price
                    .as_ref()
                    .map(|p| p.to_half_even_display(8))
                    .unwrap_or_default()
            ));
        }
        let q = self.signed_query(&params, &creds.secret);
        let url = format!("{}/api/v3/order?{}", self.base_url, q);
        let resp = self
            .http
            .post(&url)
            .header("X-MBX-APIKEY", &creds.key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        v.get("orderId")
            .and_then(|x| x.as_i64())
            .map(|i| i.to_string())
            .ok_or_else(|| format!("binance order failed: {v}"))
    }

    async fn cancel_order(
        &self,
        creds: &VenueCreds,
        _category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<(), String> {
        // Real signed DELETE /api/v3/order keyed by the bot's client order id.
        let params = format!("symbol={symbol}&origClientOrderId={order_link_id}");
        let q = self.signed_query(&params, &creds.secret);
        let url = format!("{}/api/v3/order?{}", self.base_url, q);
        let resp = self
            .http
            .delete(&url)
            .header("X-MBX-APIKEY", &creds.key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        if v.get("status").and_then(|x| x.as_str()) == Some("CANCELED") {
            return Ok(());
        }
        // -2011 "Unknown order sent" = already gone/cancelled venue-side —
        // treat as idempotent success so the caller's cancel chain continues.
        if v.get("code").and_then(|x| x.as_i64()) == Some(-2011) {
            return Ok(());
        }
        Err(format!("binance cancel failed: {v}"))
    }

    /// Task 20 B1 — gerçek imzalı GET /api/v3/openOrders (spot). Önceden boş
    /// stub'tı: gate kalkarsa sweep TÜM takip edilen emirleri "doldu" sayardı
    /// (kritik phantom-fill). Şimdi gerçek defter görünürlüğü var; kategori
    /// spot değilse dürüst hata (futures kanalı ayrı implementasyondur).
    async fn open_orders(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
    ) -> Result<Vec<VenueOrderState>, String> {
        if category != "spot" {
            return Err(format!(
                "binance open_orders: kategori '{category}' desteklenmiyor (LIVE spot kanalı)"
            ));
        }
        let params = format!("symbol={symbol}");
        let q = self.signed_query(&params, &creds.secret);
        let url = format!("{}/api/v3/openOrders?{}", self.base_url, q);
        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &creds.key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        let mut out = Vec::new();
        if let Some(list) = v.as_array() {
            for item in list {
                out.push(VenueOrderState {
                    order_id: item
                        .get("orderId")
                        .and_then(|x| x.as_i64())
                        .map(|i| i.to_string())
                        .unwrap_or_default(),
                    order_link_id: item
                        .get("clientOrderId")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .into(),
                    symbol: item
                        .get("symbol")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .into(),
                    side: item
                        .get("side")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .into(),
                    price: item
                        .get("price")
                        .and_then(|x| x.as_str())
                        .and_then(|s| Fraction::from_decimal_str(s).ok()),
                    qty: Fraction::from_any_str(
                        item.get("origQty").and_then(|x| x.as_str()).unwrap_or("0"),
                    )
                    .unwrap_or_else(|_| Fraction::zero()),
                    cum_exec_qty: Fraction::from_any_str(
                        item.get("executedQty")
                            .and_then(|x| x.as_str())
                            .unwrap_or("0"),
                    )
                    .unwrap_or_else(|_| Fraction::zero()),
                    cum_exec_fee: None,
                    status: normalize_binance_order_status(
                        item.get("status").and_then(|x| x.as_str()).unwrap_or(""),
                    ),
                });
            }
        }
        Ok(out)
    }

    /// Task 20 L3 — tek emir sorgusu (spot): origClientOrderId ile.
    async fn query_order(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<Option<VenueOrderState>, String> {
        if category != "spot" {
            return Err(format!(
                "binance query_order: kategori '{category}' desteklenmiyor (LIVE spot kanalı)"
            ));
        }
        let params = format!("symbol={symbol}&origClientOrderId={order_link_id}");
        let q = self.signed_query(&params, &creds.secret);
        let url = format!("{}/api/v3/order?{}", self.base_url, q);
        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &creds.key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        // bilinmeyen emir: {-2011 code} → None (dürüst: bulunamadı)
        if v.get("code").and_then(|x| x.as_i64()) == Some(-2011) {
            return Ok(None);
        }
        if v.get("orderId").is_none() {
            return Err(format!("binance query_order failed: {v}"));
        }
        let executed =
            Fraction::from_any_str(v.get("executedQty").and_then(|x| x.as_str()).unwrap_or("0"))
                .unwrap_or_else(|_| Fraction::zero());
        let quote = v
            .get("cummulativeQuoteQty")
            .and_then(|x| x.as_str())
            .and_then(|s| Fraction::from_any_str(s).ok());
        // ortalama dolum fiyatı: quote/executed (yoksa limit price)
        let avg = if !executed.is_zero() {
            quote.and_then(|q| q.div(&executed).ok())
        } else {
            None
        };
        let price = avg.or_else(|| {
            v.get("price")
                .and_then(|x| x.as_str())
                .and_then(|s| Fraction::from_decimal_str(s).ok())
        });
        Ok(Some(VenueOrderState {
            order_id: v
                .get("orderId")
                .and_then(|x| x.as_i64())
                .map(|i| i.to_string())
                .unwrap_or_default(),
            order_link_id: v
                .get("clientOrderId")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .into(),
            symbol: v
                .get("symbol")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .into(),
            side: v.get("side").and_then(|x| x.as_str()).unwrap_or("").into(),
            price,
            qty: Fraction::from_any_str(v.get("origQty").and_then(|x| x.as_str()).unwrap_or("0"))
                .unwrap_or_else(|_| Fraction::zero()),
            cum_exec_qty: executed,
            cum_exec_fee: None,
            status: normalize_binance_order_status(
                v.get("status").and_then(|x| x.as_str()).unwrap_or(""),
            ),
        }))
    }

    async fn positions(
        &self,
        _creds: &VenueCreds,
        _symbol: &str,
    ) -> Result<Vec<PositionSnapshot>, String> {
        Ok(Vec::new()) // spot: no positions
    }

    /// Faz 2.7 — Binance futures kademe aynası (imzalı `leverageBracket`).
    /// Adaptor spot-kanallidir; futures icin ayri `fapi` tabani kullanilir
    /// (ayni API anahtari spot+futures'ta gecerlidir). Anahtar yoksa hata
    /// dondurur — handler ABSTAIN uretir.
    async fn mmr_brackets(
        &self,
        symbol: &str,
        creds: Option<&VenueCreds>,
    ) -> Result<Vec<dca_math::MmrBracket>, String> {
        let c = creds.ok_or("BINANCE anahtari yok (abstain)")?;
        let q = self.signed_query(&format!("symbol={symbol}"), &c.secret);
        let url = format!("{}/fapi/v1/leverageBracket?{}", futures_base()?, q);
        let resp = self
            .http
            .get(&url)
            .header("X-MBX-APIKEY", &c.key)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        if v.get("code").and_then(|x| x.as_i64()).is_some() {
            let msg = v
                .get("msg")
                .and_then(|x| x.as_str())
                .unwrap_or("bilinmiyor");
            return Err(format!("binance leverageBracket reddi: {msg}"));
        }
        parse_binance_brackets(&v, symbol)
    }

    /// P3-6 L2 derinlik (Binance spot /api/v3/depth, halka açık).
    /// Adaptör spot-kanallıdır; linear istenirse dürüst 422.
    async fn orderbook(
        &self,
        category: &str,
        symbol: &str,
        limit: usize,
    ) -> Result<OrderBook, String> {
        if category != "spot" {
            return Err(format!(
                "binance orderbook: '{category}' desteklenmiyor (spot kanalı)"
            ));
        }
        let lim = match limit {
            0..=5 => 5,
            6..=10 => 10,
            11..=20 => 20,
            21..=50 => 50,
            _ => 100,
        };
        let v = self
            .public_get("/api/v3/depth", &format!("symbol={symbol}&limit={lim}"))
            .await?;
        let side = |k: &str| -> Vec<OrderBookLevel> {
            v.get(k)
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
            ts: v.get("lastUpdateId").and_then(|x| x.as_i64()).unwrap_or(0),
        })
    }
}

/// Normalize Binance's REST enum into the venue-neutral states consumed by
/// `sweep_missing_fills`. Unknown values remain unknown so reconciliation
/// fails closed instead of inventing a fill.
fn normalize_binance_order_status(status: &str) -> String {
    match status {
        "NEW" => "New".into(),
        "PARTIALLY_FILLED" => "PartiallyFilled".into(),
        "FILLED" => "Filled".into(),
        "CANCELED" => "Cancelled".into(),
        "PENDING_CANCEL" => "PendingCancel".into(),
        "REJECTED" => "Rejected".into(),
        "EXPIRED" | "EXPIRED_IN_MATCH" => "Expired".into(),
        other => other.into(),
    }
}

/// Faz 2.7 — futures tabanı (spot tabanından ayrı; testnet bayrağı paralel).
fn futures_base() -> Result<String, String> {
    let raw = if std::env::var("DCA_BINANCE_TESTNET").as_deref() == Ok("1") {
        std::env::var("BINANCE_FUTURES_TESTNET_BASE")
            .unwrap_or_else(|_| "https://testnet.binancefuture.com".into())
    } else {
        std::env::var("BINANCE_FUTURES_BASE").unwrap_or_else(|_| "https://fapi.binance.com".into())
    };
    super::try_guarded_venue_base_url(
        &raw,
        "Binance Futures",
        &["fapi.binance.com", "testnet.binancefuture.com"],
        &["fapi.binance.com"],
        false,
    )
}

/// Faz 2.7 — saf: Binance `leverageBracket` yanıtını M07 kademelerine
/// normalize eder. Girdi: `[{symbol, brackets:[{bracket, notionalFloor,
/// notionalCap, maintMarginRatio, cum}]}]` (sayılar JSON number gelir).
/// maxLeverage (initialLeverage) tasinmaz — M07 tuketmiyor (notlu).
pub fn parse_binance_brackets(
    v: &Value,
    symbol: &str,
) -> Result<Vec<dca_math::MmrBracket>, String> {
    let arr = v.as_array().ok_or("binance leverageBracket: dizi yok")?;
    let entry = arr
        .iter()
        .find(|e| e.get("symbol").and_then(|x| x.as_str()) == Some(symbol))
        .ok_or(format!("binance leverageBracket: {symbol} yok"))?;
    let list = entry
        .get("brackets")
        .and_then(|x| x.as_array())
        .ok_or("brackets yok")?;
    if list.is_empty() {
        return Err("binance leverageBracket: bos kademe".into());
    }
    let num = |it: &Value, k: &str| -> Result<dca_math::fraction::Fraction, String> {
        let s = it.get(k).map(|x| x.to_string()).ok_or(format!("{k} yok"))?;
        dca_math::fraction::Fraction::from_any_str(&s)
            .map_err(|e| format!("{k} cozulmedi ({s}): {e}"))
    };
    let mut rows: Vec<(i64, dca_math::MmrBracket)> = Vec::with_capacity(list.len());
    for it in list {
        let no = it
            .get("bracket")
            .and_then(|x| x.as_i64())
            .ok_or("bracket no yok")?;
        rows.push((
            no,
            dca_math::MmrBracket {
                min_notional: num(it, "notionalFloor")?,
                max_notional: num(it, "notionalCap")?,
                mmr: num(it, "maintMarginRatio")?,
                deduction: num(it, "cum")?,
            },
        ));
    }
    rows.sort_by_key(|(no, _)| *no);
    Ok(rows.into_iter().map(|(_, t)| t).collect())
}

#[cfg(test)]
mod binance_brackets_tests {
    use super::*;

    /// Faz 2.7 kilit test: Binance `leverageBracket` belge-şeklindeki yanıt
    /// M07 kademelerine eşlenir (sıralı, floor/cap/mmr/cum birebir).
    /// NOT: canlı kilit yok — kasadaki anahtar futures izni vermiyor
    /// (canlı çağrı dürüst ABSTAIN döndürür); şekil Binance API belgesine kilitli.
    #[test]
    fn parse_binance_brackets_maps_documented_shape() {
        let raw = serde_json::json!([
            {"symbol": "BTCUSDT", "brackets": [
                {"bracket": 1, "initialLeverage": 125, "notionalCap": 50000, "notionalFloor": 0, "maintMarginRatio": 0.004, "cum": 0.0},
                {"bracket": 2, "initialLeverage": 100, "notionalCap": 250000, "notionalFloor": 50000, "maintMarginRatio": 0.005, "cum": 50.0}
            ]}
        ]);
        let tiers = parse_binance_brackets(&raw, "BTCUSDT").expect("belge şekli çözülmeli");
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].min_notional.to_string(), "0");
        assert_eq!(tiers[0].max_notional.to_string(), "50000");
        assert_eq!(tiers[0].mmr.to_string(), "1/250");
        assert_eq!(tiers[0].deduction.to_string(), "0");
        assert_eq!(tiers[1].min_notional.to_string(), "50000");
        assert_eq!(tiers[1].deduction.to_string(), "50");
    }

    #[test]
    fn parse_binance_brackets_rejects_non_array() {
        // Hata zarfı ({code, msg}) dizi değildir — parse'e girmeden elenmeli.
        let raw = serde_json::json!({"code": -2015, "msg": "Invalid API-key"});
        assert!(parse_binance_brackets(&raw, "BTCUSDT").is_err());
    }
}

#[cfg(test)]
mod binance_sign_tests {
    use super::*;

    /// Task 15-a item 11: the signing primitive is pinned against RFC 4231
    /// test case 2 (key "Jefe", data "what do ya want for nothing?"), then the
    /// query composition is verified as payload + upper-hex signature.
    #[test]
    fn sign_query_uses_upper_hex_hmac_and_timestamp() {
        let mut mac = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        mac.update(b"what do ya want for nothing?");
        assert_eq!(
            hex::encode_upper(mac.finalize().into_bytes()),
            "5BDCC146BF60754E6A042426089575C75A003F089D2739839DEC58B964EC3843",
            "HMAC-SHA256 upper-hex primitive must match the RFC 4231 vector"
        );

        let payload = "symbol=BTCUSDT&side=BUY&origClientOrderId=b-1-main-BASE-1";
        let q = sign_query(payload, "Jefe", 1_700_000_000_000);
        assert!(q.starts_with(&format!("{payload}&timestamp=1700000000000&signature=")));
        let sig = q.rsplit("&signature=").next().unwrap();
        assert_eq!(sig.len(), 64, "SHA-256 hex digest length");
        assert!(
            sig.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
            "signature must be UPPER hex"
        );
        let mut expected = Hmac::<Sha256>::new_from_slice(b"Jefe").unwrap();
        expected.update(format!("{payload}&timestamp=1700000000000").as_bytes());
        assert_eq!(
            sig,
            hex::encode_upper(expected.finalize().into_bytes()),
            "signature must cover params + timestamp"
        );
    }
}

#[cfg(test)]
mod permission_tests {
    use super::*;

    #[test]
    fn missing_restriction_fields_fail_closed() {
        assert!(parse_account_restrictions(&serde_json::json!({})).is_err());
        assert!(parse_account_restrictions(&serde_json::json!({
            "enableWithdrawals": false,
            "enableSpotAndMarginTrading": false,
            "enableFutures": false,
            "enableReading": true
        }))
        .is_err());
    }

    #[test]
    fn withdrawal_and_trade_permissions_are_taken_from_authoritative_fields() {
        let safe = parse_account_restrictions(&serde_json::json!({
            "enableWithdrawals": false,
            "enableSpotAndMarginTrading": true,
            "enableFutures": false,
            "enableReading": true,
            "ipRestrict": true
        }))
        .unwrap();
        assert!(!safe.can_withdraw);
        assert!(safe.spot_trade);
        assert!(!safe.contract_trade);
        assert_eq!(safe.ips, vec!["IP_RESTRICTED"]);

        let unsafe_key = parse_account_restrictions(&serde_json::json!({
            "enableWithdrawals": true,
            "enableSpotAndMarginTrading": true,
            "enableFutures": false,
            "enableReading": true,
            "ipRestrict": false
        }))
        .unwrap();
        assert!(unsafe_key.can_withdraw);
    }
}

#[cfg(test)]
mod order_status_tests {
    use super::normalize_binance_order_status;

    #[test]
    fn normalizes_terminal_and_partial_binance_order_states() {
        assert_eq!(normalize_binance_order_status("FILLED"), "Filled");
        assert_eq!(
            normalize_binance_order_status("PARTIALLY_FILLED"),
            "PartiallyFilled"
        );
        assert_eq!(normalize_binance_order_status("CANCELED"), "Cancelled");
        assert_eq!(
            normalize_binance_order_status("EXPIRED_IN_MATCH"),
            "Expired"
        );
        assert_eq!(
            normalize_binance_order_status("SOME_FUTURE_STATE"),
            "SOME_FUTURE_STATE"
        );
    }
}
