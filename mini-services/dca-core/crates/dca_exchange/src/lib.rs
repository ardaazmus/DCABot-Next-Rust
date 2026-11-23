//! dca_exchange — Venue adapter layer. Bybit v5 PRIORITY (fully implemented),
//! Binance secondary, Paper venue for simulation. Includes M16 reconciliation
//! helpers and the Zero-Withdrawal hard gate validation.

// Faz 21 (CI): limit.min(1000).max(1) min<max sabitlerle güvenlidir; clamp
// geçişi clamp(max<min) panik sözleşmesini tartışmaya açar — bu biçim kalır.
#![allow(clippy::manual_clamp)]

pub mod binance;
pub mod bybit;
pub mod okx;
pub mod paper;

use dca_math::fraction::Fraction;
use dca_math::Instrument;
use serde::{Deserialize, Serialize};
use std::fmt;

/// Serialize an already tick/step-quantized exchange value without the old
/// eight-decimal truncation. Exchange increments can require more precision;
/// trailing zeroes are stripped so adapters send the minimum meaningful scale.
pub fn format_order_decimal(value: &Fraction) -> String {
    let mut text = value.to_half_even_display(18);
    if let Some(dot) = text.find('.') {
        while text.ends_with('0') {
            text.pop();
        }
        if text.len() == dot + 1 {
            text.pop();
        }
    }
    if text == "-0" {
        "0".into()
    } else {
        text
    }
}

fn normalize_venue_base_url(
    raw: &str,
    venue: &str,
    allowed_hosts: &[&str],
) -> Result<(String, String), String> {
    let url = reqwest::Url::parse(raw).map_err(|_| format!("{venue} base URL geçersiz"))?;
    let host = url
        .host_str()
        .ok_or_else(|| format!("{venue} base URL ana makine içermiyor"))?
        .to_ascii_lowercase();
    if url.scheme() != "https"
        || url.port_or_known_default() != Some(443)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(format!(
            "{venue} base URL yalnızca temiz HTTPS kök adresi olabilir"
        ));
    }
    if !allowed_hosts.iter().any(|allowed| host == *allowed) {
        return Err(format!("{venue} base URL izinli borsa alan adında değil"));
    }
    Ok((format!("https://{host}"), host))
}

/// Resolve exchange endpoints to known HTTPS origins and require an explicit
/// opt-in before a release binary can target a mainnet host.
pub fn guarded_venue_base_url(
    raw: &str,
    venue: &str,
    allowed_hosts: &[&str],
    mainnet_hosts: &[&str],
    demo_or_testnet: bool,
) -> String {
    try_guarded_venue_base_url(raw, venue, allowed_hosts, mainnet_hosts, demo_or_testnet)
        .unwrap_or_else(|error| panic!("unsafe exchange endpoint: {error}"))
}

/// Fallible variant for endpoints whose configuration is loaded after the
/// adapter starts (for example Binance Futures). Invalid configuration must
/// fail the operation cleanly instead of panicking inside a background task.
pub fn try_guarded_venue_base_url(
    raw: &str,
    venue: &str,
    allowed_hosts: &[&str],
    mainnet_hosts: &[&str],
    demo_or_testnet: bool,
) -> Result<String, String> {
    let (url, host) = normalize_venue_base_url(raw, venue, allowed_hosts)?;
    // `cfg!(test)` is a compile-time test harness signal. Executable path
    // heuristics such as checking for "deps" are attacker-controlled and
    // must never disable the explicit mainnet opt-in.
    let test_runner = cfg!(test);
    let is_mainnet = mainnet_hosts.iter().any(|mainnet| host == *mainnet);
    if is_mainnet
        && !demo_or_testnet
        && !test_runner
        && std::env::var("DCA_ALLOW_MAINNET").as_deref() != Ok("1")
    {
        return Err(format!("CRITICAL SAFETY VIOLATION: {venue} mainnet requires DCA_ALLOW_MAINNET=1; select demo/testnet for local use."));
    }
    Ok(url)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Kline {
    pub t: i64,
    pub o: String,
    pub h: String,
    pub l: String,
    pub c: String,
    pub v: String,
}

/// 24h istatistik satırı — sembol tarayıcı (P2-1) için. Tüm sayılar ondalık
/// string; `change_pct_24h` yüzde puan (örn. "-0.87" = -%0.87), borsadan
/// gelen ham birim farkları adapter içinde normalize edilir.
#[derive(Debug, Clone, Serialize)]
pub struct Ticker24h {
    pub symbol: String,
    pub last: String,
    pub change_pct_24h: String,
    pub high_24h: String,
    pub low_24h: String,
    pub volume_24h: String,
    pub turnover_24h: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct TickerSnapshot {
    pub symbol: String,
    pub last_price: Fraction,
    pub mark_price: Fraction,
    pub funding_rate: Option<Fraction>,
    pub next_funding_ts: Option<i64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WalletSnapshot {
    pub coin: String,
    pub wallet_balance: Fraction,
    pub available: Fraction,
    pub equity: Fraction,
    pub unrealized_pnl: Fraction,
}

#[derive(Debug, Clone, Serialize)]
pub struct PositionSnapshot {
    pub symbol: String,
    pub side: String,
    pub size: Fraction,
    pub avg_price: Fraction,
    pub mark_price: Fraction,
    pub unrealized_pnl: Fraction,
    pub liq_price: Option<Fraction>,
    pub leverage: Fraction,
    /// Faz 4.5 (C-5): Bybit 0=one-way, 1/2=hedge — DCA ailesi (positionIdx=0)
    /// hesap hedge-mode'daysa reddedilir; None = venue bildirmiyor.
    pub position_idx: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VenueOrderState {
    pub order_id: String,
    pub order_link_id: String,
    pub symbol: String,
    pub side: String,
    pub price: Option<Fraction>,
    pub qty: Fraction,
    pub cum_exec_qty: Fraction,
    /// Faz 3 (H-4): venue'un bildirdiği kümülatif ücret (quote cinsi varsayımı
    /// belgelenir; None = venue bildirmiyor → yerel instrument oranı kullanılır).
    pub cum_exec_fee: Option<Fraction>,
    pub status: String,
}

/// Faz 3 (C-3): venue execution kaydı — fill kanıtının birincil kaynağı.
/// Sentetik "kaybolan emir = doldu" dönemi kapandı: fill yalnız bu kayıtlarla
/// (veya terminal Filled durumuyla) işlenir; Cancelled/Rejected fill ÜRETMEZ.
#[derive(Debug, Clone, Serialize)]
pub struct ExecutionRecord {
    pub order_link_id: String,
    pub exec_id: String,
    pub exec_qty: Fraction,
    pub exec_price: Fraction,
    /// Venue'un bildirdiği bu execution'ın ücreti (quote cinsi varsayımı).
    pub exec_fee: Option<Fraction>,
    pub is_maker: bool,
    pub exec_time: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KeyPermissions {
    pub read_only: bool,
    pub can_withdraw: bool,
    pub spot_trade: bool,
    pub contract_trade: bool,
    pub ips: Vec<String>,
    pub note: String,
}

pub enum WithdrawalGate {
    Ok,
    HardFail(String),
}

#[cfg(test)]
mod live_permission_tests {
    use super::*;

    fn permissions() -> KeyPermissions {
        KeyPermissions {
            read_only: false,
            can_withdraw: false,
            spot_trade: true,
            contract_trade: true,
            ips: vec!["203.0.113.10".into()],
            note: "test".into(),
        }
    }

    #[test]
    fn mainnet_live_key_requires_trade_scope_and_ip_restriction() {
        let mut p = permissions();
        assert!(validate_live_trade_permissions(&p, false, true).is_ok());
        assert!(validate_live_trade_permissions(&p, true, true).is_ok());
        p.ips.clear();
        assert!(validate_live_trade_permissions(&p, false, true)
            .unwrap_err()
            .contains("IP"));
        assert!(validate_live_trade_permissions(&p, false, false).is_ok());
        p.spot_trade = false;
        assert!(validate_live_trade_permissions(&p, false, false)
            .unwrap_err()
            .contains("spot"));
    }

    #[test]
    fn live_key_never_accepts_withdrawal_or_read_only_permissions() {
        let mut p = permissions();
        p.can_withdraw = true;
        assert!(validate_live_trade_permissions(&p, false, false)
            .unwrap_err()
            .contains("WITHDRAWAL"));
        p.can_withdraw = false;
        p.read_only = true;
        assert!(validate_live_trade_permissions(&p, false, false)
            .unwrap_err()
            .contains("read-only"));
    }
}

/// THE ZERO-WITHDRAWAL HARD GATE: any key with withdrawal permission is
/// rejected with an unbypassable error (server maps this to HTTP 422).
pub fn validate_key_permissions(p: &KeyPermissions) -> WithdrawalGate {
    if p.can_withdraw {
        return WithdrawalGate::HardFail(
            "ZERO-WITHDRAWAL GATE VIOLATION: API key has withdrawal permission (Wallet: Withdraw). \
             Regenerate the key WITHOUT the Withdraw permission."
                .into(),
        );
    }
    if p.read_only {
        return WithdrawalGate::HardFail(
            "API key is read-only; trading permission required.".into(),
        );
    }
    WithdrawalGate::Ok
}

/// Validate the minimum exchange-key scope needed by a live bot. Mainnet
/// trading keys must be IP-restricted; withdrawal and account-transfer access
/// remain forbidden by the existing hard gate.
pub fn validate_live_trade_permissions(
    p: &KeyPermissions,
    require_contract: bool,
    require_ip_restriction: bool,
) -> Result<(), String> {
    if let WithdrawalGate::HardFail(reason) = validate_key_permissions(p) {
        return Err(reason);
    }
    let has_trade_scope = if require_contract {
        p.contract_trade
    } else {
        p.spot_trade
    };
    if !has_trade_scope {
        return Err(if require_contract {
            "API key does not have contract/futures trading permission".into()
        } else {
            "API key does not have spot trading permission".into()
        });
    }
    if require_ip_restriction && p.ips.is_empty() {
        return Err("mainnet API key must be restricted to trusted IP addresses".into());
    }
    Ok(())
}

/// L2 derinlik satırı + anlık görüntü. Fiyat/adet ondalık string (zero-float).
#[derive(Debug, Clone, Serialize)]
pub struct OrderBookLevel {
    pub price: String,
    pub qty: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderBook {
    pub symbol: String,
    pub bids: Vec<OrderBookLevel>,
    pub asks: Vec<OrderBookLevel>,
    pub ts: i64,
}

/// Venue credentials. OKX additionally requires the API passphrase
/// (v3 OkxCredentials {key, secret, passphrase} karşılığı); Bybit/Binance
/// için passphrase None'dır. Secret/passphrase asla loglanmaz — imza
/// yardımcıları Zeroizing zarfında kopyayla çalışır (P3-4 deseni).
#[derive(Clone)]
pub struct VenueCreds {
    pub key: String,
    pub secret: String,
    pub passphrase: Option<String>,
}

impl fmt::Debug for VenueCreds {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VenueCreds")
            .field("key", &"[REDACTED]")
            .field("secret", &"[REDACTED]")
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl Drop for VenueCreds {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.key.zeroize();
        self.secret.zeroize();
        if let Some(passphrase) = &mut self.passphrase {
            passphrase.zeroize();
        }
    }
}

impl From<(String, String)> for VenueCreds {
    fn from((key, secret): (String, String)) -> Self {
        Self {
            key,
            secret,
            passphrase: None,
        }
    }
}

/// Common adapter surface. All numerics as exact decimal strings.
#[async_trait::async_trait]
pub trait VenueAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    async fn server_time_ms(&self) -> Result<i64, String>;
    /// P3-6 L2 derinlik anlık görüntüsü (REST; WS diff akışı yok — kapsam
    /// dışı, dürüst sınır). PAPER desteklemez.
    async fn orderbook(
        &self,
        category: &str,
        symbol: &str,
        limit: usize,
    ) -> Result<OrderBook, String>;
    async fn instruments(
        &self,
        category: &str,
        symbol: Option<&str>,
    ) -> Result<Vec<Instrument>, String>;
    async fn klines(
        &self,
        category: &str,
        symbol: &str,
        interval: &str,
        limit: usize,
    ) -> Result<Vec<Kline>, String>;
    async fn ticker(&self, category: &str, symbol: &str) -> Result<TickerSnapshot, String>;
    /// 24h istatistik listesi (tüm semboller). PAPER boş döner.
    async fn tickers_24h(&self, category: &str) -> Result<Vec<Ticker24h>, String>;
    async fn funding_history(
        &self,
        symbol: &str,
        limit: usize,
    ) -> Result<Vec<(i64, String)>, String>;
    /// Faz 22 (P1-23): HESAP-bazlı gerçek ücretler (VIP/komisyon kademesi) —
    /// (maker, taker). Desteklemeyen venue'lar None döner (enstrüman varsayılanı
    /// kalır). Bybit: /v5/account/fee-rate.
    async fn account_fee_rate(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
    ) -> Result<Option<(Fraction, Fraction)>, String> {
        let _ = (creds, category, symbol);
        Ok(None)
    }
    /// Private (requires credentials); returns permissions for zero-withdrawal gate.
    async fn key_permissions(&self, creds: &VenueCreds) -> Result<KeyPermissions, String>;
    async fn wallet(&self, creds: &VenueCreds, coin: &str) -> Result<WalletSnapshot, String>;
    async fn place_order(&self, creds: &VenueCreds, req: &OrderRequest) -> Result<String, String>;
    /// Cancel a resting order by the bot's orderLinkId. `category` is the venue
    /// product category ("linear" futures / "spot") — Bybit rejects a cancel
    /// whose category does not match the resting order's.
    async fn cancel_order(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
        order_link_id: &str,
    ) -> Result<(), String>;
    /// Task 20 L1: `category` artık açık parametre — spot LIVE'da linear
    /// hardcode'u botun spot resting emirlerini GÖRMEZDI (phantom-fill kusuru:
    /// ilk sweep'te emir dolmuş sanılırdı). Kategori botun kendi ürün
    /// kategorisidir ("spot" / "linear").
    async fn open_orders(
        &self,
        creds: &VenueCreds,
        category: &str,
        symbol: &str,
    ) -> Result<Vec<VenueOrderState>, String>;
    /// Faz 3 (C-3): bir emrin venue execution kayıtları (fill kanıtı).
    /// Default boş — desteklemeyen venue'da çağıran terminal-durum fallback'ine
    /// düşer (query_order Filled + accFillSz).
    async fn executions(
        &self,
        _creds: &VenueCreds,
        _category: &str,
        _symbol: &str,
        _order_link_id: &str,
    ) -> Result<Vec<ExecutionRecord>, String> {
        Ok(Vec::new())
    }
    /// Task 20 L3: orderLinkId ile tek emrin güncel durumu (LIVE market-fill
    /// yakalama: market emir open_orders'a ASLA düşmez; place sonrası hemen
    /// sorgulanarak dolum kernel'e ingest edilir). None = bulunamadı.
    async fn query_order(
        &self,
        _creds: &VenueCreds,
        _category: &str,
        _symbol: &str,
        _order_link_id: &str,
    ) -> Result<Option<VenueOrderState>, String> {
        Ok(None)
    }
    async fn positions(
        &self,
        creds: &VenueCreds,
        symbol: &str,
    ) -> Result<Vec<PositionSnapshot>, String>;
    /// Push the bot's configured leverage to the venue (linear futures only).
    /// Default: no-op (paper venues keep leverage in simulation state).
    async fn set_leverage(
        &self,
        _creds: &VenueCreds,
        _category: &str,
        _symbol: &str,
        _leverage: &dca_math::Fraction,
    ) -> Result<(), String> {
        Ok(())
    }
    /// Read-only position-mode assertion (OKX `long_short_mode` tuzağı).
    /// Default: `None` (venue'de bu kavram yok / bilinmiyor) — salt-okunur,
    /// anahtar gerektirir; yoksa çağrılmaz.
    async fn pos_mode(&self, _creds: &VenueCreds) -> Result<Option<String>, String> {
        Ok(None)
    }
    /// Faz 2.7 — venue bakım-marjı kademe aynası (M07 `MmrBracket`).
    /// Bybit: halka açık risk-limit (anahtarsız); Binance: imzalı futures
    /// `leverageBracket` (anahtar gerekir, yoksa cagri hatasi — ABSTAIN
    /// handler katmaninda uretilir). Default: desteklenmiyor (OKX/Paper).
    async fn mmr_brackets(
        &self,
        _symbol: &str,
        _creds: Option<&VenueCreds>,
    ) -> Result<Vec<dca_math::MmrBracket>, String> {
        Err("bu venue kademe aynası desteklemiyor".into())
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OrderRequest {
    pub category: String, // spot | linear
    pub symbol: String,
    pub side: dca_math::Side,
    pub is_market: bool,
    pub qty: Fraction,
    pub price: Option<Fraction>,
    pub reduce_only: bool,
    pub order_link_id: String,
    pub position_idx: i32, // 0 one-way, 1 hedge long, 2 hedge short
}

/// Helper: map interval minutes to venue strings.
pub fn interval_to_bybit(minutes: u32) -> String {
    match minutes {
        1 => "1".into(),
        3 => "3".into(),
        5 => "5".into(),
        15 => "15".into(),
        30 => "30".into(),
        60 => "60".into(),
        120 => "120".into(),
        240 => "240".into(),
        360 => "360".into(),
        720 => "720".into(),
        1440 => "D".into(),
        _ => "60".into(),
    }
}

/* ------------------------------------------------------------------ */
/* LIVE fill reconciliation (M16 sweep)                                */
/* ------------------------------------------------------------------ */

/// A resting order the bot's kernel tracks while it waits on the venue
/// (minimal view needed for the open-order diff).
#[derive(Debug, Clone)]
pub struct TrackedOrder {
    pub id: String,
    pub side: dca_math::Side,
    pub price: Fraction,
}

/// A tracked resting order that vanished from the venue book.
/// EX-03 düzeltmesi: sadece FILLED/PARTIALLY_FILLED durumundaki emirler
/// sentezik fill olarak döner; CANCELLED/EXPIRED/REJECTED emirler REDDEDİLİR.
/// `executed_qty` Gerçekte yürütülen miktardır (query_order'dan cum_exec_qty).
#[derive(Debug, Clone)]
pub struct MissingFill {
    pub order_id: String,
    pub price: Fraction,
    pub side: dca_math::Side,
    /// Gerçekte yürütülen miktar (None = durum bilinmiyor → fail-closed, fill üretilmez).
    pub executed_qty: Option<Fraction>,
}

/// Fetch the venue's open orders and return every tracked resting order that
/// is no longer visible there. EX-03: her kaybolan emrin `query_order` ile
/// gerçek durumu kontrol edilir — yalnız FILLED/PARTIALLY_FILLED durumunda
/// sentezik fill üretilir; CANCELLED/EXPIRED/REJECTED reddedilir.
/// Ağ hataları çağıcıya yayılır (non-fatal: log + skip).
pub async fn sweep_missing_fills(
    adapter: &dyn VenueAdapter,
    creds: &VenueCreds,
    category: &str,
    symbol: &str,
    tracked: &[TrackedOrder],
) -> Result<Vec<MissingFill>, String> {
    let open = adapter.open_orders(creds, category, symbol).await?;
    let live_links: std::collections::HashSet<&str> =
        open.iter().map(|o| o.order_link_id.as_str()).collect();
    let mut missing = Vec::new();
    for t in tracked {
        if live_links.contains(t.id.as_str()) {
            continue; // hala açık — atla
        }
        // EX-03: gerçek durumu sor — olmayan/iptal edilmiş emir fill üretmez.
        match adapter.query_order(creds, category, symbol, &t.id).await {
            Ok(Some(st)) => {
                let status = st.status.as_str();
                let has_execution = !st.cum_exec_qty.is_zero();
                let terminal = matches!(
                    status,
                    "Filled"
                        | "PartiallyFilled"
                        | "Cancelled"
                        | "Expired"
                        | "Rejected"
                        | "Deactivated"
                );
                if has_execution && terminal {
                    missing.push(MissingFill {
                        order_id: t.id.clone(),
                        price: st.price.unwrap_or_else(|| t.price.clone()),
                        side: t.side,
                        executed_qty: Some(st.cum_exec_qty.clone()),
                    });
                }
                // Cancelled/Expired/Rejected → fill ÜRETİLMEZ (fail-closed)
            }
            Ok(None) => {
                // Order book'ta veya geçmişte yok — iptal edilmiş olabilir,
                // fail-closed: fill üretme.
            }
            Err(_) => {
                // Ağ hatası — fail-closed: fill üretme, çağıcıya log bırakır.
            }
        }
    }
    Ok(missing)
}

#[cfg(test)]
mod live_sweep_tests {
    use super::*;

    struct StubVenue {
        open_link_ids: Vec<String>,
        // query_order döndürülecek durum: None = bulunamadı (fail-closed),
        // Some(Filled) = gerçekten doldu, Some(Cancelled) = iptal edildi.
        query_result: std::collections::HashMap<String, VenueOrderState>,
    }

    #[async_trait::async_trait]
    impl VenueAdapter for StubVenue {
        fn name(&self) -> &'static str {
            "STUB"
        }
        async fn server_time_ms(&self) -> Result<i64, String> {
            Ok(0)
        }
        async fn instruments(&self, _c: &str, _s: Option<&str>) -> Result<Vec<Instrument>, String> {
            Ok(vec![])
        }
        async fn klines(
            &self,
            _c: &str,
            _s: &str,
            _i: &str,
            _l: usize,
        ) -> Result<Vec<Kline>, String> {
            Ok(vec![])
        }
        async fn ticker(&self, _c: &str, s: &str) -> Result<TickerSnapshot, String> {
            Ok(TickerSnapshot {
                symbol: s.into(),
                last_price: Fraction::one(),
                mark_price: Fraction::one(),
                funding_rate: None,
                next_funding_ts: None,
            })
        }
        async fn funding_history(&self, _s: &str, _l: usize) -> Result<Vec<(i64, String)>, String> {
            Ok(vec![])
        }
        async fn tickers_24h(&self, _c: &str) -> Result<Vec<Ticker24h>, String> {
            Ok(vec![])
        }
        async fn key_permissions(&self, _c: &VenueCreds) -> Result<KeyPermissions, String> {
            Ok(KeyPermissions {
                read_only: false,
                can_withdraw: false,
                spot_trade: true,
                contract_trade: true,
                ips: vec![],
                note: "stub".into(),
            })
        }
        async fn wallet(&self, _c: &VenueCreds, c: &str) -> Result<WalletSnapshot, String> {
            Ok(WalletSnapshot {
                coin: c.into(),
                wallet_balance: Fraction::zero(),
                available: Fraction::zero(),
                equity: Fraction::zero(),
                unrealized_pnl: Fraction::zero(),
            })
        }
        async fn place_order(&self, _c: &VenueCreds, _r: &OrderRequest) -> Result<String, String> {
            Ok("stub-order".into())
        }
        async fn cancel_order(
            &self,
            _c: &VenueCreds,
            _cat: &str,
            _sym: &str,
            _link: &str,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn open_orders(
            &self,
            _c: &VenueCreds,
            _cat: &str,
            _sym: &str,
        ) -> Result<Vec<VenueOrderState>, String> {
            Ok(self
                .open_link_ids
                .iter()
                .map(|l| VenueOrderState {
                    order_id: format!("vid-{l}"),
                    order_link_id: l.clone(),
                    symbol: "TESTUSDT".into(),
                    side: "Buy".into(),
                    price: None,
                    qty: Fraction::one(),
                    cum_exec_qty: Fraction::zero(),
                    cum_exec_fee: None,
                    status: "New".into(),
                })
                .collect())
        }
        async fn query_order(
            &self,
            _c: &VenueCreds,
            _cat: &str,
            _sym: &str,
            link: &str,
        ) -> Result<Option<VenueOrderState>, String> {
            Ok(self.query_result.get(link).cloned())
        }
        async fn positions(
            &self,
            _c: &VenueCreds,
            _sym: &str,
        ) -> Result<Vec<PositionSnapshot>, String> {
            Ok(vec![])
        }
        async fn orderbook(&self, _c: &str, s: &str, _l: usize) -> Result<OrderBook, String> {
            Ok(OrderBook {
                symbol: s.into(),
                bids: vec![],
                asks: vec![],
                ts: 0,
            })
        }
    }

    #[tokio::test]
    async fn sweep_flags_vanished_orders_as_filled() {
        let mut query_result = std::collections::HashMap::new();
        query_result.insert(
            "vanished".into(),
            VenueOrderState {
                order_id: "vid-vanished".into(),
                order_link_id: "vanished".into(),
                symbol: "TESTUSDT".into(),
                side: "Buy".into(),
                price: Some(Fraction::from_u64(100)),
                qty: Fraction::from_u64(1),
                cum_exec_qty: Fraction::from_u64(1),
                cum_exec_fee: None,
                status: "Filled".into(),
            },
        );
        let adapter = StubVenue {
            open_link_ids: vec!["still-open".into()],
            query_result,
        };
        let tracked = vec![
            TrackedOrder {
                id: "vanished".into(),
                side: dca_math::Side::Buy,
                price: Fraction::from_u64(100),
            },
            TrackedOrder {
                id: "still-open".into(),
                side: dca_math::Side::Sell,
                price: Fraction::from_u64(110),
            },
        ];
        let creds = VenueCreds {
            key: "k".into(),
            secret: "s".into(),
            passphrase: None,
        };
        let missing = sweep_missing_fills(&adapter, &creds, "spot", "TESTUSDT", &tracked)
            .await
            .unwrap();
        assert_eq!(
            missing.len(),
            1,
            "sadece FILLED olan vanished order üretilir"
        );
        assert_eq!(missing[0].order_id, "vanished");
        assert_eq!(missing[0].price.to_string(), "100");
        assert_eq!(missing[0].side, dca_math::Side::Buy);
        assert_eq!(missing[0].executed_qty, Some(Fraction::from_u64(1)));
    }

    #[tokio::test]
    async fn sweep_empty_when_all_orders_still_resting() {
        let adapter = StubVenue {
            open_link_ids: vec!["a".into(), "b".into()],
            query_result: std::collections::HashMap::new(),
        };
        let tracked = vec![
            TrackedOrder {
                id: "a".into(),
                side: dca_math::Side::Buy,
                price: Fraction::one(),
            },
            TrackedOrder {
                id: "b".into(),
                side: dca_math::Side::Sell,
                price: Fraction::one(),
            },
        ];
        let creds = VenueCreds {
            key: "k".into(),
            secret: "s".into(),
            passphrase: None,
        };
        let missing = sweep_missing_fills(&adapter, &creds, "spot", "TESTUSDT", &tracked)
            .await
            .unwrap();
        assert!(missing.is_empty());
    }

    #[tokio::test]
    async fn sweep_cancelled_order_does_not_produce_fill() {
        let mut query_result = std::collections::HashMap::new();
        query_result.insert(
            "cancelled".into(),
            VenueOrderState {
                order_id: "vid-cancelled".into(),
                order_link_id: "cancelled".into(),
                symbol: "TESTUSDT".into(),
                side: "Buy".into(),
                price: Some(Fraction::from_u64(100)),
                qty: Fraction::from_u64(1),
                cum_exec_qty: Fraction::zero(),
                cum_exec_fee: None,
                status: "Cancelled".into(),
            },
        );
        let adapter = StubVenue {
            open_link_ids: vec![],
            query_result,
        };
        let tracked = vec![TrackedOrder {
            id: "cancelled".into(),
            side: dca_math::Side::Buy,
            price: Fraction::from_u64(100),
        }];
        let creds = VenueCreds {
            key: "k".into(),
            secret: "s".into(),
            passphrase: None,
        };
        let missing = sweep_missing_fills(&adapter, &creds, "spot", "TESTUSDT", &tracked)
            .await
            .unwrap();
        assert!(
            missing.is_empty(),
            "iptal edilmiş emir fill üretmez (EX-03)"
        );
    }

    #[tokio::test]
    async fn sweep_cancelled_partial_order_preserves_cumulative_execution() {
        let mut query_result = std::collections::HashMap::new();
        query_result.insert(
            "cancelled-partial".into(),
            VenueOrderState {
                order_id: "vid-cancelled-partial".into(),
                order_link_id: "cancelled-partial".into(),
                symbol: "TESTUSDT".into(),
                side: "Buy".into(),
                price: Some(Fraction::from_u64(100)),
                qty: Fraction::from_u64(2),
                cum_exec_qty: Fraction::one(),
                cum_exec_fee: None,
                status: "Cancelled".into(),
            },
        );
        let adapter = StubVenue {
            open_link_ids: vec![],
            query_result,
        };
        let tracked = vec![TrackedOrder {
            id: "cancelled-partial".into(),
            side: dca_math::Side::Buy,
            price: Fraction::from_u64(100),
        }];
        let creds = VenueCreds {
            key: "k".into(),
            secret: "s".into(),
            passphrase: None,
        };
        let missing = sweep_missing_fills(&adapter, &creds, "spot", "TESTUSDT", &tracked)
            .await
            .unwrap();
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].executed_qty, Some(Fraction::one()));
        assert_eq!(missing[0].order_id, "cancelled-partial");
    }
}

#[cfg(test)]
mod endpoint_and_secret_tests {
    use super::*;

    #[test]
    fn venue_endpoint_validation_rejects_custom_hosts_and_non_https_urls() {
        let allowed = ["api.bybit.com", "api-testnet.bybit.com"];
        assert_eq!(
            normalize_venue_base_url("https://api-testnet.bybit.com/", "Bybit", &allowed)
                .unwrap()
                .0,
            "https://api-testnet.bybit.com"
        );
        assert!(
            normalize_venue_base_url("http://api-testnet.bybit.com", "Bybit", &allowed).is_err()
        );
        assert!(
            normalize_venue_base_url("https://api.bybit.com.evil.test", "Bybit", &allowed).is_err()
        );
        assert!(
            normalize_venue_base_url("https://user:pass@api.bybit.com", "Bybit", &allowed).is_err()
        );
        assert!(normalize_venue_base_url("https://api.bybit.com/path", "Bybit", &allowed).is_err());
        assert!(normalize_venue_base_url("https://api.bybit.com:8443", "Bybit", &allowed).is_err());
    }

    #[test]
    fn venue_credentials_debug_is_redacted() {
        let creds = VenueCreds {
            key: "visible-key-must-not-appear".into(),
            secret: "visible-secret-must-not-appear".into(),
            passphrase: Some("visible-pass-must-not-appear".into()),
        };
        let debug = format!("{creds:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("visible-key-must-not-appear"));
        assert!(!debug.contains("visible-secret-must-not-appear"));
        assert!(!debug.contains("visible-pass-must-not-appear"));
    }
}
