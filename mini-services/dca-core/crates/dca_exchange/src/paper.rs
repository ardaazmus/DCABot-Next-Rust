//! Paper venue — in-memory simulated execution for backtest/paper bots with
//! live market data. Deterministic, zero side effects, full gate pipeline.

use super::{VenueCreds, KeyPermissions, Kline, OrderBook, OrderRequest, PositionSnapshot, TickerSnapshot, VenueAdapter, VenueOrderState, WalletSnapshot};
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default)]
pub struct PaperState {
    pub bars: HashMap<String, Vec<Kline>>,
    pub balances: HashMap<String, Fraction>,
    pub orders: Vec<PaperOrder>,
    pub seq: u64,
    /// Newest close per symbol (updated by `klines`) — the reference price for
    /// MARKET fills. A market order without a reference is refused honestly
    /// instead of filling at a fake 0.0 price.
    pub last_close: HashMap<String, Fraction>,
}

#[derive(Clone)]
pub struct PaperOrder {
    pub link_id: String,
    pub symbol: String,
    pub side: dca_math::Side,
    pub price: Fraction,
    pub qty: Fraction,
    pub filled: bool,
}

pub struct PaperVenue {
    pub state: Arc<Mutex<PaperState>>,
    pub fee_rate: Fraction,
}

impl PaperVenue {
    pub fn new(start_cash: Fraction) -> Self {
        let mut balances = HashMap::new();
        balances.insert("USDT".to_string(), start_cash);
        Self {
            state: Arc::new(Mutex::new(PaperState { balances, ..Default::default() })),
            fee_rate: Fraction::from_decimal_str("0.001").unwrap(),
        }
    }

    pub async fn inject_bars(&self, symbol: &str, bars: Vec<Kline>) {
        self.state.lock().await.bars.insert(symbol.to_string(), bars);
    }

    pub async fn balance(&self, coin: &str) -> Fraction {
        self.state.lock().await.balances.get(coin).cloned().unwrap_or_else(Fraction::zero)
    }

    pub async fn settle_paper_order(&self, req: &OrderRequest, fill_price: Fraction) -> String {
        let mut st = self.state.lock().await;
        st.seq += 1;
        let id = format!("paper-{}", st.seq);
        let notional = req.qty.mul(&fill_price).unwrap_or_else(|_| Fraction::zero());
        let fee = notional.mul(&self.fee_rate).unwrap_or_else(|_| Fraction::zero());
        match req.side {
            dca_math::Side::Buy => {
                let cash = st.balances.entry("USDT".into()).or_default().clone();
                st.balances.insert("USDT".into(), cash.sub(&notional.add(&fee).unwrap_or_else(|_| Fraction::zero())).unwrap_or_else(|_| Fraction::zero()));
            }
            dca_math::Side::Sell => {
                let cash = st.balances.entry("USDT".into()).or_default().clone();
                st.balances.insert("USDT".into(), cash.add(&notional.sub(&fee).unwrap_or_else(|_| Fraction::zero())).unwrap_or_else(|_| Fraction::zero()));
            }
        }
        st.orders.push(PaperOrder {
            link_id: req.order_link_id.clone(),
            symbol: req.symbol.clone(),
            side: req.side,
            price: fill_price,
            qty: req.qty.clone(),
            filled: true,
        });
        id
    }
}

#[async_trait::async_trait]
impl VenueAdapter for PaperVenue {
    fn name(&self) -> &'static str {
        "PAPER"
    }

    async fn server_time_ms(&self) -> Result<i64, String> {
        Ok(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0))
    }

    async fn instruments(&self, _category: &str, symbol: Option<&str>) -> Result<Vec<Instrument>, String> {
        let mut i = Instrument::default();
        if let Some(s) = symbol {
            i.symbol = s.to_string();
        }
        i.maker_fee_rate = self.fee_rate.clone();
        i.taker_fee_rate = self.fee_rate.clone();
        Ok(vec![i])
    }

    async fn klines(&self, _category: &str, symbol: &str, _interval: &str, limit: usize) -> Result<Vec<Kline>, String> {
        let mut st = self.state.lock().await;
        let bars = st.bars.get(symbol).cloned().unwrap_or_default();
        let out: Vec<Kline> = bars.into_iter().rev().take(limit).collect::<Vec<_>>().into_iter().rev().collect();
        // Track the newest close as the market-fill reference price.
        if let Some(last) = out.last() {
            if let Ok(px) = Fraction::from_any_str(&last.c) {
                st.last_close.insert(symbol.to_string(), px);
            }
        }
        Ok(out)
    }

    async fn ticker(&self, _category: &str, symbol: &str) -> Result<TickerSnapshot, String> {
        let st = self.state.lock().await;
        let last = st
            .bars
            .get(symbol)
            .and_then(|b| b.last())
            .map(|k| k.c.clone())
            .unwrap_or_else(|| "0".into());
        let p = Fraction::from_any_str(&last).unwrap_or_else(|_| Fraction::zero());
        Ok(TickerSnapshot {
            symbol: symbol.into(),
            last_price: p.clone(),
            mark_price: p,
            funding_rate: None,
            next_funding_ts: None,
        })
    }

    async fn tickers_24h(&self, _category: &str) -> Result<Vec<super::Ticker24h>, String> {
        Ok(Vec::new())
    }

    async fn funding_history(&self, _symbol: &str, _limit: usize) -> Result<Vec<(i64, String)>, String> {
        Ok(Vec::new())
    }

    async fn key_permissions(&self, _creds: &VenueCreds) -> Result<KeyPermissions, String> {
        Ok(KeyPermissions { read_only: false, can_withdraw: false, spot_trade: true, contract_trade: true, ips: vec![], note: "paper venue".into() })
    }

    async fn wallet(&self, _creds: &VenueCreds, coin: &str) -> Result<WalletSnapshot, String> {
        let bal = self.balance(coin).await;
        Ok(WalletSnapshot { coin: coin.into(), wallet_balance: bal.clone(), available: bal.clone(), equity: bal, unrealized_pnl: Fraction::zero() })
    }

    async fn place_order(&self, _creds: &VenueCreds, req: &OrderRequest) -> Result<String, String> {
        let price = match req.price.clone() {
            Some(p) => p,
            None => {
                // MARKET order: fill at the venue's last tracked close.
                let last = self.state.lock().await.last_close.get(&req.symbol).cloned();
                match last {
                    Some(p) if p.is_positive() => p,
                    _ => return Err("PAPER venue: market emri için referans fiyat yok".into()),
                }
            }
        };
        let id = self.settle_paper_order(req, price).await;
        Ok(id)
    }

    async fn cancel_order(
        &self,
        _creds: &VenueCreds,
        _category: &str,
        _symbol: &str,
        _order_link_id: &str,
    ) -> Result<(), String> {
        Ok(())
    }

    async fn open_orders(&self, _creds: &VenueCreds, _category: &str, _symbol: &str) -> Result<Vec<VenueOrderState>, String> {
        Ok(Vec::new())
    }

    async fn positions(&self, _creds: &VenueCreds, _symbol: &str) -> Result<Vec<PositionSnapshot>, String> {
        Ok(Vec::new())
    }

    async fn orderbook(&self, _category: &str, _symbol: &str, _limit: usize) -> Result<OrderBook, String> {
        Err("PAPER venue'de orderbook yok — Bybit/Binance seçin".into())
    }
}

#[cfg(test)]
mod paper_tests {
    use super::*;

    fn market_req(symbol: &str, qty: &str) -> OrderRequest {
        OrderRequest {
            category: "spot".into(),
            symbol: symbol.into(),
            side: dca_math::Side::Buy,
            is_market: true,
            qty: Fraction::from_decimal_str(qty).unwrap(),
            price: None,
            reduce_only: false,
            order_link_id: format!("mkt-{symbol}"),
            position_idx: 0,
        }
    }

    /// Task 15-a item 10: a MARKET order must never fill at a fake 0.0 price.
    /// Before any klines: honest Turkish error. After klines: fills at the
    /// newest close (> 0).
    #[tokio::test]
    async fn market_fill_uses_last_close_and_refuses_without_reference() {
        let pv = PaperVenue::new(Fraction::from_u64(1_000_000));
        let err = pv.place_order(&VenueCreds { key: "k".into(), secret: "s".into(), passphrase: None }, &market_req("TESTUSDT", "1")).await.unwrap_err();
        assert!(err.contains("referans fiyat yok"), "no-reference market order must be refused, got: {err}");

        pv.inject_bars(
            "TESTUSDT",
            vec![
                Kline { t: 1, o: "99".into(), h: "101".into(), l: "98".into(), c: "100".into(), v: "1".into() },
                Kline { t: 2, o: "100".into(), h: "105".into(), l: "99".into(), c: "103.5".into(), v: "1".into() },
            ],
        )
        .await;
        // klines() refreshes the reference to the newest close.
        let _ = pv.klines("spot", "TESTUSDT", "60", 10).await.unwrap();
        let id = pv.place_order(&VenueCreds { key: "k".into(), secret: "s".into(), passphrase: None }, &market_req("TESTUSDT", "1")).await.expect("market fill");
        let st = pv.state.lock().await;
        let order = st.orders.iter().find(|o| o.link_id == "mkt-TESTUSDT").expect("order settled");
        assert!(order.price.is_positive(), "fill price must be > 0");
        assert_eq!(order.price, Fraction::from_decimal_str("103.5").unwrap(), "fills at the newest tracked close");
        assert!(id.starts_with("paper-"));
    }
}
