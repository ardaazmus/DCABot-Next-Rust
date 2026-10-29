//! dca_signals — deterministic technical-indicator + signal-condition engine.
//!
// Faz 21 (CI): indikatör formülleri (ADX/ATR/RSI/stoch) klasik index-loop
// gösterimiyle yazıldı — iter zincirleri formül okunabilirliğini bozar;
// enum varyant boyut farkı tip sözleşmesinden (Signal vs Regime payload).
#![allow(
    clippy::needless_range_loop,
    clippy::large_enum_variant,
    // Faz 21 (CI): hata mesajında enum debug adı biçimlenir (tek kullanım);
    // LCG seed sabiti bitwise okunurluk için gruplanmıştır.
    clippy::format_in_format_args,
    clippy::unusual_byte_groupings,
    clippy::doc_lazy_continuation
)]

//! This crate is a FILTERING/METRICS crate: it deliberately uses `f64` for indicator
//! math (per customer specification). Money accounting stays in `dca_math` rationals.
//!
//! Core guarantees:
//! - Zero look-ahead: every indicator value at index `i` depends only on `bars[0..=i]`,
//!   and `SignalEngine::evaluate_at_time(ts)` resolves the last CLOSED bar with `t <= ts`.
//! - Fail-closed: any warm-up NaN, missing timeframe, or unresolvable bar index makes a
//!   condition evaluate to `false` (never `true` by accident).
//! - Deterministic: no randomness, no wall-clock, no I/O inside the library.
//! - Wire format frozen: the Next.js UI and the Python E2E suite send the JSON shapes
//!   implemented in [`types`] (unit enums serialize as their variant names, `SignalValue`
//!   is internally tagged `{"kind":..,"value":..}`, `SignalNode` is untagged).

#![forbid(unsafe_code)]

pub mod engine;
pub mod indicators;
pub mod regime;
pub mod types;

pub use engine::{describe_node, describe_spec, validate_signal_node, ConditionReport, SignalEngine};
pub use indicators::{
    compute_adx, compute_atr, compute_bollinger, compute_ema, compute_indicator, compute_macd,
    compute_rsi, compute_sma, compute_stoch, compute_vol_ratio, ema_of_series, price_series,
    sma_of_series,
};
pub use regime::{classify as classify_regime, MarketRegime};
pub use types::{
    BarData, CompareOp, IndicatorKey, IndicatorSpec, LogicalOp, SignalCondition, SignalConfig,
    SignalGroup, SignalNode, SignalValue, Timeframe,
};
