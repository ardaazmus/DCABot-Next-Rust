//! dca_signals::engine — `SignalEngine`: caching, zero-look-ahead evaluation,
//! condition reports, human descriptions and DSL validation.
//!
//! Semantics (frozen):
//! - `evaluate_at_time(node, ts)` resolves, per timeframe, the index of the LAST CLOSED
//!   bar with `t <= ts`; each side of a condition is resolved on ITS OWN timeframe.
//! - CROSS ops compare against `index-1` WITHIN each side's own series; `idx == 0` on
//!   either side → false; both sides of a CROSS must share the timeframe (fail-closed
//!   false otherwise; validation rejects earlier anyway).
//! - ANY NaN (warm-up) or missing timeframe/unresolvable index → condition `false`.
//! - `And` = all children true, `Or` = any child true; empty group → false (fail-closed).

use std::collections::HashMap;

use crate::indicators::compute_indicator;
use crate::types::{
    fmt_f64, BarData, CompareOp, IndicatorKey, IndicatorSpec, LogicalOp, SignalCondition,
    SignalNode, SignalValue, Timeframe,
};
use serde::Serialize;

/// Per-TF cache of precomputed indicator series + the closed-bar series themselves.
pub struct SignalEngine {
    /// Per-timeframe CLOSED bars, ascending by `t` (sorted defensively at construction).
    series: HashMap<Timeframe, Vec<BarData>>,
    /// `spec_key(spec)` → indicator values aligned with that timeframe's series.
    cached: HashMap<String, Vec<f64>>,
    /// The "bot" timeframe = first entry passed to [`SignalEngine::new`]; used by
    /// [`SignalEngine::evaluate_latest`].
    primary_tf: Option<Timeframe>,
}

/// Per-LEAF condition report for API preview endpoints.
/// NaN/missing values are reported as `None` with `warmup = true` (fail-closed).
#[derive(Debug, Clone, Serialize)]
pub struct ConditionReport {
    pub desc: String,
    pub tf: String,
    pub left_value: Option<f64>,
    pub right_value: Option<f64>,
    pub met: bool,
    pub warmup: bool,
}

impl SignalEngine {
    /// Builds the engine: stores per-TF closed bars (ascending by `t`) and precomputes
    /// (+ caches) every requested spec. Specs whose timeframe has no bars are simply
    /// not cached — evaluation then fails closed.
    pub fn new(bars_by_tf: &[(Timeframe, Vec<BarData>)], specs: &[IndicatorSpec]) -> Self {
        let mut series: HashMap<Timeframe, Vec<BarData>> = HashMap::new();
        for (tf, bars) in bars_by_tf {
            let mut b = bars.clone();
            b.sort_by_key(|x| x.t);
            series.insert(*tf, b);
        }
        let mut cached: HashMap<String, Vec<f64>> = HashMap::new();
        for spec in specs {
            let key = Self::spec_key(spec);
            if cached.contains_key(&key) {
                continue;
            }
            if let Some(bars) = series.get(&spec.timeframe) {
                cached.insert(key, compute_indicator(bars, spec));
            }
        }
        let primary_tf = bars_by_tf.first().map(|(tf, _)| *tf);
        Self { series, cached, primary_tf }
    }

    /// Collects the deduplicated set of indicator specs (leaves) referenced by `node`,
    /// in first-occurrence order. Both condition sides count (right-side indicators too).
    pub fn specs_needed(node: &SignalNode) -> Vec<IndicatorSpec> {
        let mut out: Vec<IndicatorSpec> = Vec::new();
        collect_specs(node, &mut out);
        out
    }

    /// Deduplicated timeframes referenced by `node`, in first-occurrence order.
    pub fn timeframes_needed(node: &SignalNode) -> Vec<Timeframe> {
        let mut out: Vec<Timeframe> = Vec::new();
        for spec in Self::specs_needed(node) {
            if !out.contains(&spec.timeframe) {
                out.push(spec.timeframe);
            }
        }
        out
    }

    /// Stable cache key of a spec: includes family, timeframe and ALL parameters
    /// (defaults applied) + requested output.
    pub fn spec_key(spec: &IndicatorSpec) -> String {
        format!(
            "{:?}@{}|p={}|f={}|s={}|g={}|sd={}|d={}|o={}",
            spec.key,
            spec.timeframe.as_str(),
            spec.resolved_period(),
            spec.resolved_fast(),
            spec.resolved_slow(),
            spec.resolved_signal(),
            fmt_f64(spec.resolved_std_dev()),
            spec.resolved_d_period(),
            spec.resolved_output(),
        )
    }

    /// Core evaluation at an arbitrary timestamp (ms). Deterministic, zero look-ahead,
    /// fail-closed (see module docs).
    pub fn evaluate_at_time(&self, node: &SignalNode, ts: i64) -> bool {
        match node {
            SignalNode::Condition(c) => self.eval_condition(c, ts),
            SignalNode::Group(g) => {
                if g.children.is_empty() {
                    return false; // fail-closed (validation rejects empty groups anyway)
                }
                match g.op {
                    LogicalOp::And => g.children.iter().all(|ch| self.evaluate_at_time(ch, ts)),
                    LogicalOp::Or => g.children.iter().any(|ch| self.evaluate_at_time(ch, ts)),
                }
            }
        }
    }

    /// Convenience: evaluate at the last closed bar of the bot timeframe (the first
    /// timeframe passed to [`SignalEngine::new`]). No series → false.
    pub fn evaluate_latest(&self, node: &SignalNode) -> bool {
        let Some(tf) = self.primary_tf else {
            return false;
        };
        let Some(bars) = self.series.get(&tf) else {
            return false;
        };
        let Some(last) = bars.last() else {
            return false;
        };
        self.evaluate_at_time(node, last.t)
    }

    /// Per-LEAF reports (depth-first order) for API preview at timestamp `ts`.
    pub fn inspect(&self, node: &SignalNode, ts: i64) -> Vec<ConditionReport> {
        let mut out = Vec::new();
        self.collect_reports(node, ts, &mut out);
        out
    }

    // ---- internals ----

    fn collect_reports(&self, node: &SignalNode, ts: i64, out: &mut Vec<ConditionReport>) {
        match node {
            SignalNode::Condition(c) => out.push(self.report_leaf(c, ts)),
            SignalNode::Group(g) => {
                for ch in &g.children {
                    self.collect_reports(ch, ts, out);
                }
            }
        }
    }

    fn report_leaf(&self, c: &SignalCondition, ts: i64) -> ConditionReport {
        let left_value = self.value_at(&c.left, ts).map(|(_, v)| v);
        let right_value = match &c.right {
            SignalValue::Number(n) => Some(*n),
            SignalValue::Indicator(rs) => self.value_at(rs, ts).map(|(_, v)| v),
        };
        let met = self.eval_condition(c, ts);
        // warmup=true ⇔ a side could not be resolved (NaN warm-up / missing TF / no bar yet)
        let warmup = left_value.is_none() || right_value.is_none();
        ConditionReport {
            desc: describe_condition(c),
            tf: c.left.timeframe.as_str().to_string(),
            left_value,
            right_value,
            met,
            warmup,
        }
    }

    fn eval_condition(&self, c: &SignalCondition, ts: i64) -> bool {
        let (li, lv) = match self.value_at(&c.left, ts) {
            Some(x) => x,
            None => return false,
        };
        match &c.right {
            SignalValue::Number(n) => match c.op {
                CompareOp::Lt => lv < *n,
                CompareOp::Lte => lv <= *n,
                CompareOp::Gt => lv > *n,
                CompareOp::Gte => lv >= *n,
                CompareOp::CrossUp | CompareOp::CrossDown => {
                    if li == 0 {
                        return false;
                    }
                    let lprev = self.raw_value_at(&c.left, li - 1);
                    if lprev.is_nan() {
                        return false;
                    }
                    // A number is constant → its "previous" value is itself.
                    match c.op {
                        CompareOp::CrossUp => lprev <= *n && lv > *n,
                        _ => lprev >= *n && lv < *n,
                    }
                }
            },
            SignalValue::Indicator(rs) => {
                if matches!(c.op, CompareOp::CrossUp | CompareOp::CrossDown)
                    && rs.timeframe != c.left.timeframe
                {
                    return false; // cross across timeframes is not defined → fail-closed
                }
                let (ri, rv) = match self.value_at(rs, ts) {
                    Some(x) => x,
                    None => return false,
                };
                match c.op {
                    CompareOp::Lt => lv < rv,
                    CompareOp::Lte => lv <= rv,
                    CompareOp::Gt => lv > rv,
                    CompareOp::Gte => lv >= rv,
                    CompareOp::CrossUp | CompareOp::CrossDown => {
                        if li == 0 || ri == 0 {
                            return false;
                        }
                        let lprev = self.raw_value_at(&c.left, li - 1);
                        let rprev = self.raw_value_at(rs, ri - 1);
                        if lprev.is_nan() || rprev.is_nan() {
                            return false;
                        }
                        match c.op {
                            CompareOp::CrossUp => lprev <= rprev && lv > rv,
                            _ => lprev >= rprev && lv < rv,
                        }
                    }
                }
            }
        }
    }

    /// Resolved (idx, non-NaN value) of `spec` on its own timeframe at `ts`.
    fn value_at(&self, spec: &IndicatorSpec, ts: i64) -> Option<(usize, f64)> {
        let bars = self.series.get(&spec.timeframe)?;
        let idx = last_idx_at_or_before(bars, ts)?;
        let v = self.raw_value_at(spec, idx);
        if v.is_nan() {
            None
        } else {
            Some((idx, v))
        }
    }

    /// Raw value at an explicit index (NaN for warm-up / out of range).
    fn raw_value_at(&self, spec: &IndicatorSpec, idx: usize) -> f64 {
        let values = self.values_for(spec);
        values.get(idx).copied().unwrap_or(f64::NAN)
    }

    /// Cached indicator values; on a cache miss compute them on the fly
    /// (deterministic + causal, so this never breaks zero look-ahead).
    fn values_for(&self, spec: &IndicatorSpec) -> Vec<f64> {
        let key = Self::spec_key(spec);
        if let Some(v) = self.cached.get(&key) {
            return v.clone();
        }
        match self.series.get(&spec.timeframe) {
            Some(bars) => compute_indicator(bars, spec),
            None => Vec::new(),
        }
    }
}

fn collect_specs(node: &SignalNode, out: &mut Vec<IndicatorSpec>) {
    match node {
        SignalNode::Condition(c) => {
            push_spec(&c.left, out);
            if let SignalValue::Indicator(s) = &c.right {
                push_spec(s, out);
            }
        }
        SignalNode::Group(g) => {
            for ch in &g.children {
                collect_specs(ch, out);
            }
        }
    }
}

fn push_spec(spec: &IndicatorSpec, out: &mut Vec<IndicatorSpec>) {
    let key = SignalEngine::spec_key(spec);
    if !out.iter().any(|s| SignalEngine::spec_key(s) == key) {
        out.push(spec.clone());
    }
}

/// Index of the LAST bar with `t <= ts` in an ascending series (binary search).
fn last_idx_at_or_before(bars: &[BarData], ts: i64) -> Option<usize> {
    if bars.is_empty() {
        return None;
    }
    let n = bars.partition_point(|b| b.t <= ts);
    if n == 0 {
        None
    } else {
        Some(n - 1)
    }
}

/// Human-readable spec description, e.g. `"RSI(14,M15)"`, `"MACD(12,26,9,H1).macd"`,
/// `"BB(20,2.0,M5).lower"`, `"EMA(200,H1)"`, `"VOL(20,M15).ratio"`, `"PRICE(M15).close"`.
/// The `.output` suffix appears whenever the resolved output is not `"default"`.
pub fn describe_spec(spec: &IndicatorSpec) -> String {
    let tf = spec.timeframe.as_str();
    let out = spec.resolved_output();
    let suffix = if out == "default" { String::new() } else { format!(".{}", out) };
    let body = match spec.key {
        IndicatorKey::Rsi => format!("RSI({},{})", spec.resolved_period(), tf),
        IndicatorKey::Ema => format!("EMA({},{})", spec.resolved_period(), tf),
        IndicatorKey::Sma => format!("SMA({},{})", spec.resolved_period(), tf),
        IndicatorKey::Macd => format!(
            "MACD({},{},{},{})",
            spec.resolved_fast(),
            spec.resolved_slow(),
            spec.resolved_signal(),
            tf
        ),
        IndicatorKey::Bb => format!(
            "BB({},{},{})",
            spec.resolved_period(),
            fmt_f64(spec.resolved_std_dev()),
            tf
        ),
        IndicatorKey::Stoch => format!(
            "STOCH({},{},{})",
            spec.resolved_period(),
            spec.resolved_d_period(),
            tf
        ),
        IndicatorKey::Atr => format!("ATR({},{})", spec.resolved_period(), tf),
        IndicatorKey::Adx => format!("ADX({},{})", spec.resolved_period(), tf),
        IndicatorKey::Vol => format!("VOL({},{})", spec.resolved_period(), tf),
        IndicatorKey::Price => format!("PRICE({})", tf),
    };
    format!("{}{}", body, suffix)
}

/// Human-readable one-line condition, e.g. `"RSI(14,M15) < 30.0"`.
pub fn describe_condition(c: &SignalCondition) -> String {
    let op = match c.op {
        CompareOp::Lt => "<",
        CompareOp::Lte => "<=",
        CompareOp::Gt => ">",
        CompareOp::Gte => ">=",
        CompareOp::CrossUp => "cross-up",
        CompareOp::CrossDown => "cross-down",
    };
    let right = match &c.right {
        SignalValue::Number(n) => fmt_f64(*n),
        SignalValue::Indicator(s) => describe_spec(s),
    };
    format!("{} {} {}", describe_spec(&c.left), op, right)
}

/// Recursive human-readable node description: `"(A AND B)"` / `"(A OR B)"`.
pub fn describe_node(node: &SignalNode) -> String {
    match node {
        SignalNode::Condition(c) => describe_condition(c),
        SignalNode::Group(g) => {
            let op = match g.op {
                LogicalOp::And => "AND",
                LogicalOp::Or => "OR",
            };
            let joiner = format!(" {} ", op);
            let parts: Vec<String> = g.children.iter().map(describe_node).collect();
            format!("({})", parts.join(&joiner))
        }
    }
}

/// Validates a signal tree (fail-closed DSL). Errors carry Turkish user-facing messages
/// (they surface verbatim in the UI / API 422 responses).
pub fn validate_signal_node(node: &SignalNode) -> Result<(), String> {
    validate_node(node, 1)
}

fn validate_node(node: &SignalNode, depth: usize) -> Result<(), String> {
    if depth > 3 {
        return Err(
            "Sinyal ağacı çok derin: en fazla 3 seviyeye (grup içinde grup) izin verilir."
                .to_string(),
        );
    }
    match node {
        SignalNode::Group(g) => {
            if g.children.is_empty() {
                return Err("Koşul grubu en az 1 çocuk içermelidir.".to_string());
            }
            if g.children.len() > 12 {
                return Err("Koşul grubu en fazla 12 çocuk içerebilir.".to_string());
            }
            for ch in &g.children {
                validate_node(ch, depth + 1)?;
            }
            Ok(())
        }
        SignalNode::Condition(c) => validate_condition(c),
    }
}

fn validate_condition(c: &SignalCondition) -> Result<(), String> {
    validate_spec(&c.left)?;
    match &c.right {
        SignalValue::Number(n) => {
            if n.is_nan() || n.is_infinite() {
                return Err("Koşulun sağ tarafındaki sayı geçerli olmalıdır.".to_string());
            }
            Ok(())
        }
        SignalValue::Indicator(rs) => {
            validate_spec(rs)?;
            if matches!(c.op, CompareOp::CrossUp | CompareOp::CrossDown)
                && rs.timeframe != c.left.timeframe
            {
                return Err(format!(
                    "CrossUp/CrossDown koşulunda iki tarafın zaman dilimi aynı olmalı (sol: {}, sağ: {}).",
                    c.left.timeframe.as_str(),
                    rs.timeframe.as_str()
                ));
            }
            Ok(())
        }
    }
}

fn validate_spec(s: &IndicatorSpec) -> Result<(), String> {
    let check_period = |name: &str, v: Option<usize>| -> Result<(), String> {
        if let Some(p) = v {
            if p == 0 || p > 1000 {
                return Err(format!("{} periyodu 1 ile 1000 arasında olmalıdır.", name));
            }
        }
        Ok(())
    };
    check_period("Periyot", s.period)?;
    check_period("MACD fast", s.fast)?;
    check_period("MACD slow", s.slow)?;
    check_period("MACD signal", s.signal)?;
    check_period("Stoch %D", s.d_period)?;
    if let Some(sd) = s.std_dev {
        if sd <= 0.0 || sd > 10.0 {
            return Err("Bollinger std_dev (0, 10] aralığında olmalıdır.".to_string());
        }
    }
    if s.key == IndicatorKey::Macd && s.resolved_fast() >= s.resolved_slow() {
        return Err(
            "MACD için fast periyodu slow periyodundan küçük olmalıdır.".to_string(),
        );
    }
    if let Some(o) = &s.output {
        let valid = crate::types::valid_outputs(s.key);
        if !valid.contains(&o.as_str()) {
            return Err(format!(
                "Bilinmeyen çıktı '{}': '{}' için geçerli çıktılar: {}.",
                o,
                format!("{:?}", s.key),
                valid.join(", ")
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indicators::{compute_macd, compute_rsi};
    use crate::types::{IndicatorKey, SignalConfig, SignalGroup, Timeframe};

    /// The EXACT wire payload from the customer spec (group root).
    const GROUP_JSON: &str = r#"{"root":{"op":"And","children":[
      {"left":{"key":"Rsi","timeframe":"M15","period":14},"op":"Lt","right":{"kind":"number","value":30}},
      {"left":{"key":"Price","timeframe":"M15","output":"close"},"op":"Gt","right":{"kind":"indicator","value":{"key":"Ema","timeframe":"H1","period":200}}}
    ]}}"#;

    /// A bare condition as root must deserialize too.
    const BARE_JSON: &str = r#"{"root":{"left":{"key":"Rsi","timeframe":"M15","period":14},"op":"CrossUp","right":{"kind":"indicator","value":{"key":"Ema","timeframe":"M15","period":20}}}}"#;

    fn bars_from_closes(closes: &[f64], t0: i64, step: i64) -> Vec<BarData> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| BarData {
                t: t0 + i as i64 * step,
                o: c,
                h: c,
                l: c,
                c,
                v: 100.0,
            })
            .collect()
    }

    fn spec(key: IndicatorKey, tf: Timeframe, period: Option<usize>, output: Option<&str>) -> IndicatorSpec {
        IndicatorSpec {
            key,
            timeframe: tf,
            period,
            fast: None,
            slow: None,
            signal: None,
            std_dev: None,
            d_period: None,
            output: output.map(|s| s.to_string()),
        }
    }

    fn cond(left: IndicatorSpec, op: CompareOp, right: SignalValue) -> SignalNode {
        SignalNode::Condition(SignalCondition { left, op, right })
    }

    fn group(op: LogicalOp, children: Vec<SignalNode>) -> SignalNode {
        SignalNode::Group(SignalGroup { op, children })
    }

    #[test]
    fn engine_and_or_truth_table() {
        // closes [10,20,30,40] at t=1..4
        let bars = bars_from_closes(&[10.0, 20.0, 30.0, 40.0], 1, 1);
        let price = || spec(IndicatorKey::Price, Timeframe::M1, None, Some("close"));
        let cond_a = cond(price(), CompareOp::Gt, SignalValue::Number(15.0)); // true idx 1..3
        let cond_b = cond(price(), CompareOp::Lt, SignalValue::Number(25.0)); // true idx 0..1
        let cond_c = cond(price(), CompareOp::Gte, SignalValue::Number(40.0)); // true idx 3
        let specs = SignalEngine::specs_needed(&group(
            LogicalOp::And,
            vec![cond_a.clone(), group(LogicalOp::Or, vec![cond_b.clone(), cond_c.clone()])],
        ));
        let engine = SignalEngine::new(&[(Timeframe::M1, bars)], &specs);

        // Nested: AND[A, OR[B, C]]
        let root_and = group(
            LogicalOp::And,
            vec![cond_a.clone(), group(LogicalOp::Or, vec![cond_b.clone(), cond_c.clone()])],
        );
        assert!(!engine.evaluate_at_time(&root_and, 1), "A false at idx0 → false");
        assert!(engine.evaluate_at_time(&root_and, 2), "A ✓, B ✓ at idx1 → true");
        assert!(!engine.evaluate_at_time(&root_and, 3), "A ✓, B ✗ C ✗ at idx2 → false");
        assert!(engine.evaluate_at_time(&root_and, 4), "A ✓, C ✓ at idx3 → true");

        // Flat OR[B, C]
        let root_or = group(LogicalOp::Or, vec![cond_b.clone(), cond_c.clone()]);
        assert!(engine.evaluate_at_time(&root_or, 1));
        assert!(engine.evaluate_at_time(&root_or, 2));
        assert!(!engine.evaluate_at_time(&root_or, 3));
        assert!(engine.evaluate_at_time(&root_or, 4));

        // evaluate_latest uses the last closed bar of the bot TF (M1 → t=4)
        assert_eq!(engine.evaluate_latest(&root_and), engine.evaluate_at_time(&root_and, 4));
        assert!(engine.evaluate_latest(&root_or));
    }

    #[test]
    fn engine_warmup_fail_closed() {
        // RSI(14) on 10 bars → every value NaN → any condition false.
        let closes: Vec<f64> = (0..10).map(|i| 100.0 + i as f64).collect();
        let bars = bars_from_closes(&closes, 1, 1);
        let rsi = spec(IndicatorKey::Rsi, Timeframe::M1, Some(14), None);
        let node = cond(rsi.clone(), CompareOp::Lt, SignalValue::Number(100.0));
        let engine = SignalEngine::new(&[(Timeframe::M1, bars)], &[rsi.clone()]);
        for ts in 1..=10i64 {
            assert!(!engine.evaluate_at_time(&node, ts), "warm-up must fail closed");
        }
        let rep = &engine.inspect(&node, 5)[0];
        assert!(rep.warmup && rep.left_value.is_none() && !rep.met);
        assert_eq!(rep.tf, "M1");
        assert!(rep.desc.contains("RSI(14,M1)"));

        // CROSS at idx 0 must be false even when values would satisfy it.
        let bars2 = bars_from_closes(&[10.0, 20.0], 1, 1);
        let price = spec(IndicatorKey::Price, Timeframe::M1, None, Some("close"));
        let cross0 = cond(
            price.clone(),
            CompareOp::CrossUp,
            SignalValue::Number(0.0),
        );
        let engine2 = SignalEngine::new(&[(Timeframe::M1, bars2)], &[price.clone()]);
        assert!(!engine2.evaluate_at_time(&cross0, 1), "idx==0 → false");
        assert!(!engine2.evaluate_at_time(&cross0, 2), "prev(10) <= 0 is false");

        // CROSS vs indicator with a NaN prev (%SMA warm-up) → false.
        let sma1 = spec(IndicatorKey::Sma, Timeframe::M1, Some(2), None);
        let bars3 = bars_from_closes(&[10.0, 20.0, 30.0], 1, 1);
        let cross_ind = cond(
            price.clone(),
            CompareOp::CrossUp,
            SignalValue::Indicator(sma1.clone()),
        );
        let engine3 = SignalEngine::new(&[(Timeframe::M1, bars3)], &[price, sma1]);
        assert!(!engine3.evaluate_at_time(&cross_ind, 1), "idx0 → false");
    }

    #[test]
    fn engine_zero_lookahead() {
        // close > 25 becomes true ONLY at bar index 7 (t=8).
        let closes = [10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 10.0, 50.0, 10.0, 10.0];
        let bars = bars_from_closes(&closes, 1000, 1000); // t = 1000..10000
        let price = spec(IndicatorKey::Price, Timeframe::M1, None, Some("close"));
        let node = cond(price.clone(), CompareOp::Gt, SignalValue::Number(25.0));
        let engine = SignalEngine::new(&[(Timeframe::M1, bars)], &[price]);
        assert!(!engine.evaluate_at_time(&node, 1000 + 5 * 1000), "ts(bar5) → 10 ≤ 25 → false");
        assert!(!engine.evaluate_at_time(&node, 1000 + 6 * 1000), "ts(bar6) still false");
        assert!(engine.evaluate_at_time(&node, 1000 + 7 * 1000), "ts(bar7) → 50 > 25 → true");
        // Between-bars ts resolves to the last CLOSED bar (bar 7 at t=7999 → bar 6 → false)
        assert!(!engine.evaluate_at_time(&node, 1000 + 7 * 1000 - 1));
        // ts beyond the last bar resolves to the last bar (c=10 → false)
        assert!(!engine.evaluate_at_time(&node, 99_999));
    }

    #[test]
    fn engine_macd_cross_detected() {
        // Decline then rise: macd starts below its signal, then crosses UP.
        let mut closes = Vec::new();
        for i in 0..15 {
            closes.push(100.0 - i as f64);
        }
        for i in 15..40 {
            closes.push(86.0 + (i - 14) as f64);
        }
        let bars = bars_from_closes(&closes, 1, 1);
        let left = IndicatorSpec {
            key: IndicatorKey::Macd,
            timeframe: Timeframe::M1,
            period: None,
            fast: Some(3),
            slow: Some(6),
            signal: Some(3),
            std_dev: None,
            d_period: None,
            output: Some("macd".into()),
        };
        let mut right = left.clone();
        right.output = Some("signal".into());
        let node = cond(left.clone(), CompareOp::CrossUp, SignalValue::Indicator(right.clone()));
        let specs = vec![left, right];
        let engine = SignalEngine::new(&[(Timeframe::M1, bars)], &specs);
        let mut crossups = 0;
        for ts in 1..=40i64 {
            if engine.evaluate_at_time(&node, ts) {
                crossups += 1;
            }
        }
        assert!(crossups >= 1, "expected at least one MACD/Signal CrossUp");
    }

    #[test]
    fn serde_roundtrip_group_and_bare_root() {
        // 1) EXACT group payload deserializes.
        let cfg: SignalConfig = serde_json::from_str(GROUP_JSON).expect("group payload must parse");
        let SignalNode::Group(g) = &cfg.root else {
            panic!("root must be a group");
        };
        assert_eq!(g.op, LogicalOp::And);
        assert_eq!(g.children.len(), 2);

        // 2) Re-serialize → deserialize again → structurally equal (no nulls for None).
        let s1 = serde_json::to_string(&cfg).unwrap();
        assert!(!s1.contains("null"), "None fields must be skipped: {}", s1);
        let cfg2: SignalConfig = serde_json::from_str(&s1).unwrap();
        assert_eq!(cfg, cfg2);

        // 3) Bare condition root deserializes too.
        let bare: SignalConfig = serde_json::from_str(BARE_JSON).expect("bare payload must parse");
        assert!(matches!(bare.root, SignalNode::Condition(_)));
        let bare2: SignalConfig =
            serde_json::from_str(&serde_json::to_string(&bare).unwrap()).unwrap();
        assert_eq!(bare, bare2);

        // 4) Evaluate without panics: M15 30 bars + H1 210 bars.
        let m15: Vec<BarData> = (0..30)
            .map(|i| {
                let c = 100.0 + i as f64 * 0.5;
                BarData { t: i as i64 * 900_000, o: c, h: c, l: c, c, v: 10.0 }
            })
            .collect();
        let h1: Vec<BarData> = (0..210)
            .map(|i| {
                let c = 1000.0 + i as f64;
                BarData { t: i as i64 * 3_600_000, o: c, h: c, l: c, c, v: 10.0 }
            })
            .collect();
        let specs = SignalEngine::specs_needed(&cfg.root);
        assert_eq!(specs.len(), 3, "Rsi + Price + Ema deduped");
        assert_eq!(SignalEngine::timeframes_needed(&cfg.root), vec![Timeframe::M15, Timeframe::H1]);
        let engine = SignalEngine::new(&[(Timeframe::M15, m15), (Timeframe::H1, h1)], &specs);
        // At the last M15 bar the EMA(200,H1) is still in warm-up → fail-closed false.
        assert!(!engine.evaluate_latest(&cfg.root));
        // CrossUp payload on a single TF: both sides ready at the end → bool without panic.
        let m15b: Vec<BarData> = (0..60)
            .map(|i| {
                let c = 100.0 + (i as f64).sin();
                BarData { t: i as i64 * 900_000, o: c, h: c, l: c, c, v: 10.0 }
            })
            .collect();
        let bspecs = SignalEngine::specs_needed(&bare.root);
        let bengine = SignalEngine::new(&[(Timeframe::M15, m15b)], &bspecs);
        let _ = bengine.evaluate_latest(&bare.root); // must not panic
        assert_eq!(bengine.inspect(&bare.root, 59 * 900_000).len(), 1);
    }

    #[test]
    fn validate_rejects_invalid_signals() {
        let price = || spec(IndicatorKey::Price, Timeframe::M15, None, Some("close"));
        let rsi_m15 = || spec(IndicatorKey::Rsi, Timeframe::M15, Some(14), None);

        // depth > 3
        let deep = group(
            LogicalOp::And,
            vec![group(
                LogicalOp::Or,
                vec![group(LogicalOp::And, vec![cond(rsi_m15(), CompareOp::Lt, SignalValue::Number(30.0))])],
            )],
        );
        let err = validate_signal_node(&deep).unwrap_err();
        assert!(!err.trim().is_empty());

        // CROSS with mismatched timeframes
        let cross_tf = cond(
            rsi_m15(),
            CompareOp::CrossUp,
            SignalValue::Indicator(spec(IndicatorKey::Ema, Timeframe::H1, Some(20), None)),
        );
        let err = validate_signal_node(&cross_tf).unwrap_err();
        assert!(!err.trim().is_empty());
        assert!(err.contains("zaman dilimi"));

        // period 0
        let bad_period = cond(
            spec(IndicatorKey::Rsi, Timeframe::M15, Some(0), None),
            CompareOp::Lt,
            SignalValue::Number(30.0),
        );
        assert!(validate_signal_node(&bad_period).is_err());

        // MACD fast >= slow
        let bad_macd = cond(
            IndicatorSpec {
                key: IndicatorKey::Macd,
                timeframe: Timeframe::M15,
                period: None,
                fast: Some(26),
                slow: Some(12),
                signal: Some(9),
                std_dev: None,
                d_period: None,
                output: None,
            },
            CompareOp::Gt,
            SignalValue::Number(0.0),
        );
        assert!(validate_signal_node(&bad_macd).is_err());

        // unknown output
        let bad_output = cond(
            spec(IndicatorKey::Rsi, Timeframe::M15, Some(14), Some("banana")),
            CompareOp::Lt,
            SignalValue::Number(30.0),
        );
        let err = validate_signal_node(&bad_output).unwrap_err();
        assert!(err.contains("Bilinmeyen çıktı"));

        // empty group also rejected
        let empty = group(LogicalOp::And, vec![]);
        assert!(validate_signal_node(&empty).is_err());

        // valid nested tree passes
        let ok = group(
            LogicalOp::And,
            vec![
                cond(rsi_m15(), CompareOp::Lt, SignalValue::Number(30.0)),
                cond(price(), CompareOp::Gt, SignalValue::Number(1.0)),
            ],
        );
        assert!(validate_signal_node(&ok).is_ok());
    }

    #[test]
    fn describe_strings() {
        let rsi = spec(IndicatorKey::Rsi, Timeframe::M15, Some(14), None);
        assert_eq!(describe_spec(&rsi), "RSI(14,M15)");

        let macd = IndicatorSpec {
            key: IndicatorKey::Macd,
            timeframe: Timeframe::H1,
            period: None,
            fast: Some(12),
            slow: Some(26),
            signal: Some(9),
            std_dev: None,
            d_period: None,
            output: Some("macd".into()),
        };
        assert_eq!(describe_spec(&macd), "MACD(12,26,9,H1).macd");

        let bb = IndicatorSpec {
            key: IndicatorKey::Bb,
            timeframe: Timeframe::M5,
            period: Some(20),
            fast: None,
            slow: None,
            signal: None,
            std_dev: Some(2.0),
            d_period: None,
            output: Some("lower".into()),
        };
        assert_eq!(describe_spec(&bb), "BB(20,2.0,M5).lower");

        let ema = spec(IndicatorKey::Ema, Timeframe::H1, Some(200), None);
        assert_eq!(describe_spec(&ema), "EMA(200,H1)");

        let vol = spec(IndicatorKey::Vol, Timeframe::M15, Some(20), None);
        assert_eq!(describe_spec(&vol), "VOL(20,M15).ratio");

        let price = spec(IndicatorKey::Price, Timeframe::M15, None, None);
        assert_eq!(describe_spec(&price), "PRICE(M15).close");

        let node = group(
            LogicalOp::And,
            vec![
                cond(rsi.clone(), CompareOp::Lt, SignalValue::Number(30.0)),
                cond(ema.clone(), CompareOp::Gt, SignalValue::Number(100.0)),
            ],
        );
        let d = describe_node(&node);
        assert!(!d.is_empty());
        assert!(d.contains("AND"));
        assert!(d.contains("RSI(14,M15) < 30.0"));
        assert!(d.contains("EMA(200,H1) > 100.0"));
        assert_eq!(describe_node(&SignalNode::Condition(SignalCondition {
            left: rsi,
            op: CompareOp::CrossUp,
            right: SignalValue::Indicator(ema),
        })), "RSI(14,M15) cross-up EMA(200,H1)");
    }

    #[test]
    fn spec_key_stable_and_param_sensitive() {
        let a = spec(IndicatorKey::Ema, Timeframe::H1, None, None);
        let b = spec(IndicatorKey::Ema, Timeframe::H1, Some(20), None);
        assert_eq!(SignalEngine::spec_key(&a), SignalEngine::spec_key(&b), "default == explicit");
        let c = spec(IndicatorKey::Ema, Timeframe::H1, Some(50), None);
        assert_ne!(SignalEngine::spec_key(&a), SignalEngine::spec_key(&c));
        let d = spec(IndicatorKey::Ema, Timeframe::M15, Some(20), None);
        assert_ne!(SignalEngine::spec_key(&a), SignalEngine::spec_key(&d));
        let mut e = a.clone();
        e.output = Some("close".into());
        assert_ne!(SignalEngine::spec_key(&a), SignalEngine::spec_key(&e));
    }

    #[test]
    fn indicator_value_matches_direct_computation() {
        // Engine cache must return exactly what indicators.rs computes.
        let closes: Vec<f64> = (0..30).map(|i| 100.0 + i as f64 * 0.7).collect();
        let bars = bars_from_closes(&closes, 1, 1);
        let rsi_spec = spec(IndicatorKey::Rsi, Timeframe::M1, Some(14), None);
        let engine = SignalEngine::new(&[(Timeframe::M1, bars.clone())], &[rsi_spec.clone()]);
        let expected = compute_rsi(&bars, 14);
        for ts in 1..=30i64 {
            let idx = (ts - 1) as usize;
            let via_engine = engine.inspect(
                &cond(rsi_spec.clone(), CompareOp::Lt, SignalValue::Number(101.0)),
                ts,
            )[0]
            .left_value;
            if expected[idx].is_nan() {
                assert!(via_engine.is_none());
            } else {
                assert_eq!(via_engine, Some(expected[idx]));
            }
        }
        // MACD hist identity through the engine cache
        let mut macd_spec = spec(IndicatorKey::Macd, Timeframe::M1, None, Some("macd"));
        macd_spec.fast = Some(3);
        macd_spec.slow = Some(6);
        macd_spec.signal = Some(3);
        let mut sig_spec = macd_spec.clone();
        sig_spec.output = Some("signal".into());
        let mut hist_spec = macd_spec.clone();
        hist_spec.output = Some("hist".into());
        let specs = vec![macd_spec.clone(), sig_spec.clone(), hist_spec.clone()];
        let engine = SignalEngine::new(&[(Timeframe::M1, bars.clone())], &specs);
        let macd = compute_macd(&bars, 3, 6, 3, "macd");
        let sig = compute_macd(&bars, 3, 6, 3, "signal");
        let node_hist = cond(hist_spec, CompareOp::Gt, SignalValue::Number(0.0));
        // hist > 0 ⇔ macd > signal
        let node_gt = cond(macd_spec, CompareOp::Gt, SignalValue::Indicator(sig_spec));
        for ts in 1..=30i64 {
            let idx = (ts - 1) as usize;
            let hv = engine.inspect(&node_hist, ts)[0].left_value;
            let both_ready = !macd[idx].is_nan() && !sig[idx].is_nan();
            if both_ready {
                assert_eq!(hv, Some(macd[idx] - sig[idx]));
                // hist > 0 ⇔ macd > signal
                assert_eq!(
                    engine.evaluate_at_time(&node_hist, ts),
                    engine.evaluate_at_time(&node_gt, ts)
                );
            }
        }
    }
}
