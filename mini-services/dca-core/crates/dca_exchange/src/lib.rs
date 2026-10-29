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
        return WithdrawalGate::HardFail("API key is read-only; trading permission required.".into());
    }
    WithdrawalGate::Ok
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
#[derive(Debug, Clone)]
pub struct VenueCreds {
    pub key: String,
    pub secret: String,
    pub passphrase: Option<String>,
}

impl From<(String, String)> for VenueCreds {
    fn from((key, secret): (String, String)) -> Self {
        Self { key, secret, passphrase: None }
    }
}

/// Common adapter surface. All numerics as exact decimal strings.
#[async_trait::async_trait]
pub trait VenueAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    async fn server_time_ms(&self) -> Result<i64, String>;
    /// P3-6 L2 derinlik anlık görüntüsü (REST; WS diff akışı yok — kapsam
    /// dışı, dürüst sınır). PAPER desteklemez.
    async fn orderbook(&self, category: &str, symbol: &str, limit: usize) -> Result<OrderBook, String>;
    async fn instruments(&self, category: &str, symbol: Option<&str>) -> Result<Vec<Instrument>, String>;
    async fn klines(&self, category: &str, symbol: &str, interval: &str, limit: usize) -> Result<Vec<Kline>, String>;
    async fn ticker(&self, category: &str, symbol: &str) -> Result<TickerSnapshot, String>;
    /// 24h istatistik listesi (tüm semboller). PAPER boş döner.
    async fn tickers_24h(&self, category: &str) -> Result<Vec<Ticker24h>, String>;
    async fn funding_history(&self, symbol: &str, limit: usize) -> Result<Vec<(i64, String)>, String>;
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
    async fn positions(&self, creds: &VenueCreds, symbol: &str) -> Result<Vec<PositionSnapshot>, String>;
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

/// A tracked resting order that vanished from the venue book — treated as
/// filled by the caller (kernel synthesizes a maker fill at the limit price).
#[derive(Debug, Clone)]
pub struct MissingFill {
    pub order_id: String,
    pub price: Fraction,
    pub side: dca_math::Side,
}

/// Fetch the venue's open orders and return every tracked resting order that
/// is no longer visible there (i.e. it was filled or cancelled venue-side).
/// Pure diff over `order_link_id`s; network errors propagate to the caller,
/// which must treat them as non-fatal (log + skip).
pub async fn sweep_missing_fills(
    adapter: &dyn VenueAdapter,
    creds: &VenueCreds,
    category: &str,
    symbol: &str,
    tracked: &[TrackedOrder],
) -> Result<Vec<MissingFill>, String> {
    let open = adapter.open_orders(creds, category, symbol).await?;
    let live_links: std::collections::HashSet<&str> = open.iter().map(|o| o.order_link_id.as_str()).collect();
    Ok(tracked
        .iter()
        .filter(|t| !live_links.contains(t.id.as_str()))
        .map(|t| MissingFill { order_id: t.id.clone(), price: t.price.clone(), side: t.side })
        .collect())
}

#[cfg(test)]
mod live_sweep_tests {
    use super::*;

    struct StubVenue {
        open_link_ids: Vec<String>,
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
        async fn klines(&self, _c: &str, _s: &str, _i: &str, _l: usize) -> Result<Vec<Kline>, String> {
            Ok(vec![])
        }
        async fn ticker(&self, _c: &str, s: &str) -> Result<TickerSnapshot, String> {
            Ok(TickerSnapshot { symbol: s.into(), last_price: Fraction::one(), mark_price: Fraction::one(), funding_rate: None, next_funding_ts: None })
        }
        async fn funding_history(&self, _s: &str, _l: usize) -> Result<Vec<(i64, String)>, String> {
            Ok(vec![])
        }
        async fn tickers_24h(&self, _c: &str) -> Result<Vec<Ticker24h>, String> {
            Ok(vec![])
        }
        async fn key_permissions(&self, _c: &VenueCreds) -> Result<KeyPermissions, String> {
            Ok(KeyPermissions { read_only: false, can_withdraw: false, spot_trade: true, contract_trade: true, ips: vec![], note: "stub".into() })
        }
        async fn wallet(&self, _c: &VenueCreds, c: &str) -> Result<WalletSnapshot, String> {
            Ok(WalletSnapshot { coin: c.into(), wallet_balance: Fraction::zero(), available: Fraction::zero(), equity: Fraction::zero(), unrealized_pnl: Fraction::zero() })
        }
        async fn place_order(&self, _c: &VenueCreds, _r: &OrderRequest) -> Result<String, String> {
            Ok("stub-order".into())
        }
        async fn cancel_order(&self, _c: &VenueCreds, _cat: &str, _sym: &str, _link: &str) -> Result<(), String> {
            Ok(())
        }
        async fn open_orders(&self, _c: &VenueCreds, _cat: &str, _sym: &str) -> Result<Vec<VenueOrderState>, String> {
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
        async fn positions(&self, _c: &VenueCreds, _sym: &str) -> Result<Vec<PositionSnapshot>, String> {
            Ok(vec![])
        }
        async fn orderbook(&self, _c: &str, s: &str, _l: usize) -> Result<OrderBook, String> {
            Ok(OrderBook { symbol: s.into(), bids: vec![], asks: vec![], ts: 0 })
        }
    }

    #[tokio::test]
    async fn sweep_flags_vanished_orders_as_filled() {
        let adapter = StubVenue { open_link_ids: vec!["still-open".into()] };
        let tracked = vec![
            TrackedOrder { id: "vanished".into(), side: dca_math::Side::Buy, price: Fraction::from_u64(100) },
            TrackedOrder { id: "still-open".into(), side: dca_math::Side::Sell, price: Fraction::from_u64(110) },
        ];
        let creds = VenueCreds { key: "k".into(), secret: "s".into(), passphrase: None };
        let missing = sweep_missing_fills(&adapter, &creds, "spot", "TESTUSDT", &tracked).await.unwrap();
        assert_eq!(missing.len(), 1, "exactly the vanished order is synthesized");
        assert_eq!(missing[0].order_id, "vanished");
        assert_eq!(missing[0].price.to_string(), "100");
        assert_eq!(missing[0].side, dca_math::Side::Buy);
    }

    #[tokio::test]
    async fn sweep_empty_when_all_orders_still_resting() {
        let adapter = StubVenue { open_link_ids: vec!["a".into(), "b".into()] };
        let tracked = vec![
            TrackedOrder { id: "a".into(), side: dca_math::Side::Buy, price: Fraction::one() },
            TrackedOrder { id: "b".into(), side: dca_math::Side::Sell, price: Fraction::one() },
        ];
        let creds = VenueCreds { key: "k".into(), secret: "s".into(), passphrase: None };
        let missing = sweep_missing_fills(&adapter, &creds, "spot", "TESTUSDT", &tracked).await.unwrap();
        assert!(missing.is_empty());
    }
}
