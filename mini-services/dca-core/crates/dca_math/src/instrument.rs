//! Canonical instrument model shared by all crates (M08 lattice params).

use crate::fraction::Fraction;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Instrument {
    pub symbol: String,
    pub base: String,
    pub quote: String,
    pub tick_size: Fraction,
    pub step_size: Fraction,
    pub min_qty: Fraction,
    pub min_notional: Fraction,
    pub maker_fee_rate: Fraction,
    pub taker_fee_rate: Fraction,
    pub max_leverage: Fraction,
}

impl Default for Instrument {
    fn default() -> Self {
        Self {
            symbol: "BTCUSDT".into(),
            base: "BTC".into(),
            quote: "USDT".into(),
            tick_size: Fraction::from_decimal_str("0.1").unwrap(),
            step_size: Fraction::from_decimal_str("0.0001").unwrap(),
            min_qty: Fraction::from_decimal_str("0.0001").unwrap(),
            min_notional: Fraction::from_decimal_str("5").unwrap(),
            maker_fee_rate: Fraction::from_decimal_str("0.001").unwrap(),
            taker_fee_rate: Fraction::from_decimal_str("0.001").unwrap(),
            max_leverage: Fraction::from_u64(100),
        }
    }
}

/// MMR bracket for linear futures (Bybit risk-limit style): notional range with
/// maintenance margin rate `mmr` and fixed `deduction`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MmrBracket {
    pub min_notional: Fraction,
    pub max_notional: Fraction, // exclusive upper; use huge for last tier
    pub mmr: Fraction,
    pub deduction: Fraction,
}
