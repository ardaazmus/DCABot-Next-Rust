//! dca_signals::types — wire types of the signal DSL plus the closed-bar input type.
//!
//! SERDE NAMES ARE FROZEN: the Next.js UI and the Python E2E suite send exactly these
//! JSON shapes. Do not rename fields/variants without an orchestrator-level migration.
//! Wire example accepted by `SignalConfig`:
//! ```json
//! {"root":{"op":"And","children":[
//!   {"left":{"key":"Rsi","timeframe":"M15","period":14},"op":"Lt","right":{"kind":"number","value":30}},
//!   {"left":{"key":"Price","timeframe":"M15","output":"close"},"op":"Gt",
//!    "right":{"kind":"indicator","value":{"key":"Ema","timeframe":"H1","period":200}}}]}
//! ```

use serde::{Deserialize, Serialize};

/// Candlestick interval of a series. Serializes as the variant name itself
/// (`"M1"`, `"M5"`, `"M15"`, `"H1"`, `"H4"`, `"D1"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Timeframe {
    M1,
    M5,
    M15,
    H1,
    H4,
    D1,
}

impl Timeframe {
    /// Canonical wire string of this timeframe.
    pub fn as_str(self) -> &'static str {
        match self {
            Timeframe::M1 => "M1",
            Timeframe::M5 => "M5",
            Timeframe::M15 => "M15",
            Timeframe::H1 => "H1",
            Timeframe::H4 => "H4",
            Timeframe::D1 => "D1",
        }
    }
}

/// Supported indicator families. Serializes as the variant name itself
/// (`"Rsi"`, `"Ema"`, `"Sma"`, `"Macd"`, `"Bb"`, `"Stoch"`, `"Atr"`, `"Adx"`, `"Vol"`, `"Price"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum IndicatorKey {
    Rsi,
    Ema,
    Sma,
    Macd,
    Bb,
    Stoch,
    Atr,
    Adx,
    Vol,
    Price,
}

/// One indicator request: family + timeframe + parameters + requested output.
/// `None` fields mean "use the default for this [`IndicatorKey`]" (see [`default_period`],
/// [`default_output`]); they deserialize when absent and are skipped when `None` on
/// serialization for clean JSON.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IndicatorSpec {
    pub key: IndicatorKey,
    pub timeframe: Timeframe,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub period: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fast: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slow: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signal: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub std_dev: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub d_period: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

impl IndicatorSpec {
    /// Effective `period` after applying the per-key default.
    pub fn resolved_period(&self) -> usize {
        self.period.unwrap_or_else(|| default_period(self.key))
    }
    /// Effective MACD fast period (default 12).
    pub fn resolved_fast(&self) -> usize {
        self.fast.unwrap_or(12)
    }
    /// Effective MACD slow period (default 26).
    pub fn resolved_slow(&self) -> usize {
        self.slow.unwrap_or(26)
    }
    /// Effective MACD signal period (default 9).
    pub fn resolved_signal(&self) -> usize {
        self.signal.unwrap_or(9)
    }
    /// Effective Bollinger std-dev multiplier (default 2.0).
    pub fn resolved_std_dev(&self) -> f64 {
        self.std_dev.unwrap_or(2.0)
    }
    /// Effective Stochastic %D period (default 3).
    pub fn resolved_d_period(&self) -> usize {
        self.d_period.unwrap_or(3)
    }
    /// Effective output name; `"default"` = the single/primary output of the key.
    pub fn resolved_output(&self) -> String {
        self.output
            .clone()
            .unwrap_or_else(|| default_output(self.key).to_string())
    }
}

/// Default `period` per indicator key (`Macd`/`Price` do not use `period`).
pub fn default_period(key: IndicatorKey) -> usize {
    match key {
        IndicatorKey::Rsi | IndicatorKey::Stoch | IndicatorKey::Atr | IndicatorKey::Adx => 14,
        IndicatorKey::Ema | IndicatorKey::Sma | IndicatorKey::Bb | IndicatorKey::Vol => 20,
        IndicatorKey::Macd | IndicatorKey::Price => 0,
    }
}

/// Default (primary) output name per indicator key.
pub fn default_output(key: IndicatorKey) -> &'static str {
    match key {
        IndicatorKey::Rsi | IndicatorKey::Ema | IndicatorKey::Sma | IndicatorKey::Adx => "default",
        IndicatorKey::Macd => "macd",
        IndicatorKey::Bb => "mid",
        IndicatorKey::Stoch => "k",
        IndicatorKey::Atr => "atr_abs",
        IndicatorKey::Vol => "ratio",
        IndicatorKey::Price => "close",
    }
}

/// All valid `output` strings per indicator key (used by validation).
pub fn valid_outputs(key: IndicatorKey) -> &'static [&'static str] {
    match key {
        IndicatorKey::Rsi | IndicatorKey::Ema | IndicatorKey::Sma | IndicatorKey::Adx => {
            &["default"]
        }
        IndicatorKey::Macd => &["macd", "signal", "hist"],
        IndicatorKey::Bb => &["lower", "mid", "upper", "bandwidth"],
        IndicatorKey::Stoch => &["k", "d"],
        IndicatorKey::Atr => &["atr_abs", "atr_pct"],
        IndicatorKey::Vol => &["ratio"],
        IndicatorKey::Price => &["close", "change_pct"],
    }
}

/// Comparison / crossover operator. Serializes as the variant name
/// (`"Lt"`, `"Lte"`, `"Gt"`, `"Gte"`, `"CrossUp"`, `"CrossDown"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum CompareOp {
    Lt,
    Lte,
    Gt,
    Gte,
    CrossUp,
    CrossDown,
}

/// Right-hand side of a condition. Internally tagged:
/// `{"kind":"number","value":30}` or `{"kind":"indicator","value":{...IndicatorSpec...}}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SignalValue {
    Number(f64),
    Indicator(IndicatorSpec),
}

/// One leaf condition: `left <op> right`, both sides resolved on their own timeframe.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalCondition {
    pub left: IndicatorSpec,
    pub op: CompareOp,
    pub right: SignalValue,
}

/// Logical combinator. Serializes as `"And"` / `"Or"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum LogicalOp {
    And,
    Or,
}

/// A logical group of child nodes. `And` = all children true; `Or` = any child true.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalGroup {
    pub op: LogicalOp,
    pub children: Vec<SignalNode>,
}

/// Untagged tree node: a condition has `left`/`op`/`right`, a group has `op`/`children`.
/// Untagged works because the two shapes are mutually exclusive (op `"Lt"` is not a
/// valid `LogicalOp`; groups require `children`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum SignalNode {
    Condition(SignalCondition),
    Group(SignalGroup),
}

/// Root wrapper of the signal DSL.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SignalConfig {
    pub root: SignalNode,
}

/// Closed OHLCV bar (deterministic input; ascending `t` per timeframe).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct BarData {
    /// Close time of the bar (ms epoch); bars are CLOSED, ascending by `t`.
    pub t: i64,
    pub o: f64,
    pub h: f64,
    pub l: f64,
    pub c: f64,
    pub v: f64,
}

/// Formats a float with at most 2 decimals, trimming trailing zeros but keeping at
/// least one decimal digit (`2.0` → `"2.0"`, `2.5` → `"2.5"`, `1.25` → `"1.25"`).
pub(crate) fn fmt_f64(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    let s = format!("{:.2}", x);
    let s = s.trim_end_matches('0').trim_end_matches('.').to_string();
    if s.contains('.') {
        s
    } else {
        format!("{}.0", s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeframe_and_enum_serde_names() {
        assert_eq!(serde_json::to_string(&Timeframe::M15).unwrap(), "\"M15\"");
        assert_eq!(serde_json::to_string(&IndicatorKey::Rsi).unwrap(), "\"Rsi\"");
        assert_eq!(serde_json::to_string(&CompareOp::CrossUp).unwrap(), "\"CrossUp\"");
        assert_eq!(serde_json::to_string(&LogicalOp::And).unwrap(), "\"And\"");
        assert_eq!(
            serde_json::from_str::<Timeframe>("\"H1\"").unwrap(),
            Timeframe::H1
        );
    }

    #[test]
    fn signal_value_tagged_wire() {
        let v = SignalValue::Number(30.0);
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#"{"kind":"number","value":30.0}"#);
        let back: SignalValue = serde_json::from_str(&s).unwrap();
        assert_eq!(back, v);
        let iv = SignalValue::Indicator(IndicatorSpec {
            key: IndicatorKey::Ema,
            timeframe: Timeframe::H1,
            period: Some(200),
            fast: None,
            slow: None,
            signal: None,
            std_dev: None,
            d_period: None,
            output: None,
        });
        let s = serde_json::to_string(&iv).unwrap();
        assert!(s.contains("\"kind\":\"indicator\""));
        assert!(s.contains("\"key\":\"Ema\""));
        assert!(!s.contains("null"), "None fields must be skipped: {}", s);
    }

    #[test]
    fn resolved_defaults_per_key() {
        let rsi = IndicatorSpec {
            key: IndicatorKey::Rsi,
            timeframe: Timeframe::M15,
            period: None,
            fast: None,
            slow: None,
            signal: None,
            std_dev: None,
            d_period: None,
            output: None,
        };
        assert_eq!(rsi.resolved_period(), 14);
        assert_eq!(rsi.resolved_output(), "default");
        let mut macd = rsi.clone();
        macd.key = IndicatorKey::Macd;
        assert_eq!(macd.resolved_fast(), 12);
        assert_eq!(macd.resolved_slow(), 26);
        assert_eq!(macd.resolved_signal(), 9);
        assert_eq!(macd.resolved_output(), "macd");
        let mut bb = rsi.clone();
        bb.key = IndicatorKey::Bb;
        assert_eq!(bb.resolved_std_dev(), 2.0);
        assert_eq!(bb.resolved_output(), "mid");
        assert_eq!(valid_outputs(IndicatorKey::Bb).len(), 4);
    }

    #[test]
    fn fmt_f64_trims_but_keeps_one_decimal() {
        assert_eq!(fmt_f64(2.0), "2.0");
        assert_eq!(fmt_f64(2.5), "2.5");
        assert_eq!(fmt_f64(1.25), "1.25");
        assert_eq!(fmt_f64(30.0), "30.0");
        assert_eq!(fmt_f64(0.0), "0.0");
    }
}
