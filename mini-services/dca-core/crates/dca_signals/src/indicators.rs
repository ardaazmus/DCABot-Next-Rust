//! dca_signals::indicators — deterministic `f64` indicator functions.
//!
//! Contract for every function:
//! - Input: a slice of CLOSED `BarData`, ascending by `t`.
//! - Output: `Vec<f64>` aligned 1:1 with the input bars; warm-up indices are `f64::NAN`.
//! - Zero look-ahead: the value at index `i` depends only on `bars[0..=i]`.
//! - No panics on degenerate input (short series, zero ranges) — NaN / guards instead.

use crate::types::{BarData, IndicatorKey, IndicatorSpec};

fn convex_update(previous: f64, value: f64, weight: f64) -> f64 {
    if previous.is_sign_positive() == value.is_sign_positive() {
        // Same-sign subtraction cannot exceed the finite input range, and this
        // form preserves a constant series even at the edge of f64::MAX.
        previous + (value - previous) * weight
    } else {
        // Opposite signs can overflow in `value - previous`; use a fused
        // weighted sum so cancellation happens before the final rounding.
        previous.mul_add(1.0 - weight, value * weight)
    }
}

/// EMA over an arbitrary series (reusable helper; MACD signal line uses it).
/// Assumes NaNs only at the LEADING edge. Seed = SMA of the first `period` values
/// (placed at index `first_valid + period - 1`), then `v*alpha + prev*(1-alpha)`
/// with `alpha = 2/(p+1)`. NaN before the seed index.
pub fn ema_of_series(values: &[f64], period: usize) -> Vec<f64> {
    let n = values.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n == 0 {
        return out;
    }
    let Some(f0) = values.iter().position(|v| !v.is_nan()) else {
        return out;
    };
    if n - f0 < period {
        return out;
    }
    let mut prev: f64 = 0.0;
    for (i, value) in values[f0..f0 + period].iter().copied().enumerate() {
        let weight = 1.0 / (i + 1) as f64;
        // A conventional sum can overflow even when the mean is representable
        // (for example, several finite values near f64::MAX). Compute the
        // running mean as a convex combination and fuse the final addition.
        prev = convex_update(prev, value, weight);
    }
    out[f0 + period - 1] = prev;
    let alpha = 2.0 / (period as f64 + 1.0);
    for i in f0 + period..n {
        prev = convex_update(prev, values[i], alpha);
        out[i] = prev;
    }
    out
}

/// SMA over an arbitrary series (reusable helper; Stochastic %D uses it).
/// Assumes NaNs only at the LEADING edge; NaN before index `first_valid + period - 1`.
pub fn sma_of_series(values: &[f64], period: usize) -> Vec<f64> {
    let n = values.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n == 0 {
        return out;
    }
    let Some(f0) = values.iter().position(|v| !v.is_nan()) else {
        return out;
    };
    if n - f0 < period {
        return out;
    }
    let mut sum = 0.0;
    for v in &values[f0..f0 + period] {
        sum += v;
    }
    out[f0 + period - 1] = sum / period as f64;
    for i in f0 + period..n {
        sum += values[i] - values[i - period];
        out[i] = sum / period as f64;
    }
    out
}

/// RSI with Wilder's smoothing, EXACTLY: seed average gain/loss over the first
/// `period` changes (bars[1..=period] vs bars[0..period]), placed at index `period`,
/// then Wilder recursion `avg = (prev*(p-1) + x)/p`. NaN for indices < period.
/// AvgLoss == 0 → 100.0.
pub fn compute_rsi(bars: &[BarData], period: usize) -> Vec<f64> {
    let n = bars.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n <= period {
        return out;
    }
    let p = period as f64;
    let mut avg_gain = 0.0;
    let mut avg_loss = 0.0;
    for i in 1..=period {
        let d = bars[i].c - bars[i - 1].c;
        if d > 0.0 {
            avg_gain += d;
        } else {
            avg_loss -= d;
        }
    }
    avg_gain /= p;
    avg_loss /= p;
    out[period] = rsi_from(avg_gain, avg_loss);
    for i in period + 1..n {
        let d = bars[i].c - bars[i - 1].c;
        let g = if d > 0.0 { d } else { 0.0 };
        let l = if d < 0.0 { -d } else { 0.0 };
        avg_gain = (avg_gain * (p - 1.0) + g) / p;
        avg_loss = (avg_loss * (p - 1.0) + l) / p;
        out[i] = rsi_from(avg_gain, avg_loss);
    }
    out
}

fn rsi_from(avg_gain: f64, avg_loss: f64) -> f64 {
    if avg_loss == 0.0 {
        // Spec: zero average loss (all gains / flat) maps to the strongest reading.
        return 100.0;
    }
    let rs = avg_gain / avg_loss;
    100.0 - 100.0 / (1.0 + rs)
}

/// EMA of closes: seed = SMA of the first `period` closes at index `period-1`,
/// then `ema = c*alpha + prev*(1-alpha)`, `alpha = 2/(p+1)`. NaN before `period-1`.
pub fn compute_ema(bars: &[BarData], period: usize) -> Vec<f64> {
    let closes: Vec<f64> = bars.iter().map(|b| b.c).collect();
    ema_of_series(&closes, period)
}

/// Simple moving average of closes. NaN before index `period-1`.
pub fn compute_sma(bars: &[BarData], period: usize) -> Vec<f64> {
    let closes: Vec<f64> = bars.iter().map(|b| b.c).collect();
    sma_of_series(&closes, period)
}

/// MACD. `macd_line = ema_fast - ema_slow` (NaN until the slow EMA is ready);
/// `signal` = EMA over the macd values (starts once `signal_p` macd values exist,
/// seeded with their SMA); `hist = macd - signal`. `output`: `"macd"` | `"signal"` | `"hist"`.
pub fn compute_macd(bars: &[BarData], fast: usize, slow: usize, signal_p: usize, output: &str) -> Vec<f64> {
    let n = bars.len();
    let mut out = vec![f64::NAN; n];
    if n == 0 || fast == 0 || slow == 0 {
        return out;
    }
    let closes: Vec<f64> = bars.iter().map(|b| b.c).collect();
    let ef = ema_of_series(&closes, fast);
    let es = ema_of_series(&closes, slow);
    let mut macd = vec![f64::NAN; n];
    for i in 0..n {
        if !ef[i].is_nan() && !es[i].is_nan() {
            macd[i] = ef[i] - es[i];
        }
    }
    let sig = ema_of_series(&macd, signal_p);
    match output {
        "signal" => sig,
        "hist" => {
            for i in 0..n {
                if !macd[i].is_nan() && !sig[i].is_nan() {
                    out[i] = macd[i] - sig[i];
                }
            }
            out
        }
        _ => macd, // "macd"
    }
}

/// Bollinger Bands. `mid` = SMA(close); `sigma` = POPULATION std (divide by p);
/// `upper`/`lower` = mid ± k*sigma; `bandwidth` = (upper-lower)/mid*100.
/// `output`: `"lower"` | `"mid"` | `"upper"` | `"bandwidth"`.
pub fn compute_bollinger(bars: &[BarData], period: usize, std_dev: f64, output: &str) -> Vec<f64> {
    let n = bars.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n < period {
        return out;
    }
    for i in period - 1..n {
        let w = &bars[i + 1 - period..=i];
        let mean = w.iter().map(|b| b.c).sum::<f64>() / period as f64;
        let var = w.iter().map(|b| (b.c - mean) * (b.c - mean)).sum::<f64>() / period as f64;
        let sigma = var.sqrt();
        let upper = mean + std_dev * sigma;
        let lower = mean - std_dev * sigma;
        out[i] = match output {
            "upper" => upper,
            "lower" => lower,
            "bandwidth" => {
                if mean != 0.0 {
                    (upper - lower) / mean * 100.0
                } else {
                    f64::NAN
                }
            }
            _ => mean, // "mid"
        };
    }
    out
}

/// Stochastic Oscillator. `%K = (c - LL(low,k_p)) / (HH(high,k_p) - LL(low,k_p)) * 100`
/// (HH == LL → 50.0 guard); `%D = SMA(%K, d_p)`. `output`: `"k"` | `"d"`.
pub fn compute_stoch(bars: &[BarData], k_period: usize, d_period: usize, output: &str) -> Vec<f64> {
    let n = bars.len();
    let mut k = vec![f64::NAN; n];
    if k_period == 0 || n == 0 {
        return k;
    }
    for i in k_period - 1..n {
        let w = &bars[i + 1 - k_period..=i];
        let hh = w.iter().map(|b| b.h).fold(f64::NEG_INFINITY, f64::max);
        let ll = w.iter().map(|b| b.l).fold(f64::INFINITY, f64::min);
        k[i] = if hh == ll {
            50.0
        } else {
            (bars[i].c - ll) / (hh - ll) * 100.0
        };
    }
    match output {
        "d" => sma_of_series(&k, d_period),
        _ => k, // "k"
    }
}

/// Average True Range (Wilder smoothing of the true range).
/// `TR_0 = h-l`; `TR_i = max(h-l, |h-c[i-1]|, |l-c[i-1]|)`.
/// `output`: `"atr_abs"` | `"atr_pct"` (= atr/close*100).
pub fn compute_atr(bars: &[BarData], period: usize, output: &str) -> Vec<f64> {
    let n = bars.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n == 0 || n < period {
        return out;
    }
    let mut tr = vec![0.0f64; n];
    tr[0] = bars[0].h - bars[0].l;
    for i in 1..n {
        let pc = bars[i - 1].c;
        tr[i] = (bars[i].h - bars[i].l)
            .max((bars[i].h - pc).abs())
            .max((bars[i].l - pc).abs());
    }
    let p = period as f64;
    let mut a = tr[..period].iter().sum::<f64>() / p;
    out[period - 1] = a;
    for i in period..n {
        a = (a * (p - 1.0) + tr[i]) / p;
        out[i] = a;
    }
    if output == "atr_pct" {
        for i in 0..n {
            if !out[i].is_nan() && bars[i].c != 0.0 {
                out[i] = out[i] / bars[i].c * 100.0;
            }
        }
    }
    out
}

/// ADX (standard Wilder construction):
/// +DM/-DM → Wilder-smoothed → +DI/-DI → `DX = 100*|+DI - -DI|/(+DI + -DI)` (0 guard)
/// → ADX = Wilder SMA of DX. NaN during warm-up (~`2*period-1`).
pub fn compute_adx(bars: &[BarData], period: usize) -> Vec<f64> {
    let n = bars.len();
    let mut out = vec![f64::NAN; n];
    if period == 0 || n < 2 * period {
        return out;
    }
    let mut tr = vec![0.0f64; n];
    let mut pdm = vec![0.0f64; n];
    let mut ndm = vec![0.0f64; n];
    for i in 1..n {
        let up = bars[i].h - bars[i - 1].h;
        let dn = bars[i - 1].l - bars[i].l;
        pdm[i] = if up > dn && up > 0.0 { up } else { 0.0 };
        ndm[i] = if dn > up && dn > 0.0 { dn } else { 0.0 };
        let pc = bars[i - 1].c;
        tr[i] = (bars[i].h - bars[i].l)
            .max((bars[i].h - pc).abs())
            .max((bars[i].l - pc).abs());
    }
    let p = period as f64;
    // Wilder-smoothed TR/+DM/-DM, seeded at index `period` with the mean of 1..=period.
    let mut s_tr = tr[1..=period].iter().sum::<f64>() / p;
    let mut s_p = pdm[1..=period].iter().sum::<f64>() / p;
    let mut s_n = ndm[1..=period].iter().sum::<f64>() / p;
    let mut dx = vec![f64::NAN; n];
    for i in period..n {
        if i > period {
            s_tr = (s_tr * (p - 1.0) + tr[i]) / p;
            s_p = (s_p * (p - 1.0) + pdm[i]) / p;
            s_n = (s_n * (p - 1.0) + ndm[i]) / p;
        }
        let pdi = if s_tr > 0.0 { 100.0 * s_p / s_tr } else { 0.0 };
        let ndi = if s_tr > 0.0 { 100.0 * s_n / s_tr } else { 0.0 };
        let denom = pdi + ndi;
        dx[i] = if denom > 0.0 { 100.0 * (pdi - ndi).abs() / denom } else { 0.0 };
    }
    // ADX = Wilder SMA of DX; seed = mean of the first `period` DX values
    // (indices period..=2*period-1) → first valid ADX at index 2*period-1.
    let mut a = dx[period..=2 * period - 1].iter().sum::<f64>() / p;
    out[2 * period - 1] = a;
    for i in 2 * period..n {
        a = (a * (p - 1.0) + dx[i]) / p;
        out[i] = a;
    }
    out
}

/// Volume ratio: `v[i] / SMA(v, period)` — the CURRENT bar is included in the average
/// (deterministic closed-bar semantics). NaN during warm-up or when SMA == 0.
pub fn compute_vol_ratio(bars: &[BarData], period: usize) -> Vec<f64> {
    let vols: Vec<f64> = bars.iter().map(|b| b.v).collect();
    let sma = sma_of_series(&vols, period);
    bars.iter()
        .zip(sma.iter())
        .map(|(b, s)| {
            if s.is_nan() || *s == 0.0 {
                f64::NAN
            } else {
                b.v / s
            }
        })
        .collect()
}

/// Raw price-derived series. `output`: `"close"` | `"change_pct"`
/// (= (c[i]/c[i-1] - 1)*100; NaN at index 0).
pub fn price_series(bars: &[BarData], output: &str) -> Vec<f64> {
    match output {
        "change_pct" => {
            let n = bars.len();
            let mut out = vec![f64::NAN; n];
            for i in 1..n {
                if bars[i - 1].c != 0.0 {
                    out[i] = (bars[i].c / bars[i - 1].c - 1.0) * 100.0;
                }
            }
            out
        }
        _ => bars.iter().map(|b| b.c).collect(), // "close"
    }
}

/// Dispatch an [`IndicatorSpec`] to its computation (defaults applied).
/// Returns a series aligned with the bars of `spec.timeframe`.
pub fn compute_indicator(bars: &[BarData], spec: &IndicatorSpec) -> Vec<f64> {
    let out = spec.resolved_output();
    match spec.key {
        IndicatorKey::Rsi => compute_rsi(bars, spec.resolved_period()),
        IndicatorKey::Ema => compute_ema(bars, spec.resolved_period()),
        IndicatorKey::Sma => compute_sma(bars, spec.resolved_period()),
        IndicatorKey::Macd => compute_macd(
            bars,
            spec.resolved_fast(),
            spec.resolved_slow(),
            spec.resolved_signal(),
            &out,
        ),
        IndicatorKey::Bb => compute_bollinger(bars, spec.resolved_period(), spec.resolved_std_dev(), &out),
        IndicatorKey::Stoch => compute_stoch(bars, spec.resolved_period(), spec.resolved_d_period(), &out),
        IndicatorKey::Atr => compute_atr(bars, spec.resolved_period(), &out),
        IndicatorKey::Adx => compute_adx(bars, spec.resolved_period()),
        IndicatorKey::Vol => compute_vol_ratio(bars, spec.resolved_period()),
        IndicatorKey::Price => price_series(bars, &out),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Timeframe;

    /// Bars from closes only (o=h=l=c, fixed volume, t ascending from 1 step 1).
    fn bars_from_closes(closes: &[f64]) -> Vec<BarData> {
        closes
            .iter()
            .enumerate()
            .map(|(i, &c)| BarData { t: (i + 1) as i64, o: c, h: c, l: c, c, v: 100.0 })
            .collect()
    }

    #[test]
    fn rsi_wilder_known_value() {
        // Classic Wilder RSI(14) dataset (first 14 closes of Wilder's example table).
        // Seed covers the 14 changes bars[1..=14]; the seed RSI lands at index 14.
        // Wilder's table rounds intermediates (published ≈ 70.46); full-precision
        // recomputation lands within 0.5 of it, hence the tolerance.
        let closes = [
            44.34, 44.09, 44.15, 43.61, 44.33, 44.83, 45.10, 45.42, 45.84, 46.08, 45.89, 46.03,
            45.61, 46.28, 46.28, 46.00, 46.03, 46.41, 46.22, 45.64,
        ];
        let bars = bars_from_closes(&closes);
        let rsi = compute_rsi(&bars, 14);
        assert!((rsi[14] - 70.46).abs() < 0.5, "rsi[14] = {}", rsi[14]);
    }

    #[test]
    fn rsi_all_gains_is_100() {
        let closes: Vec<f64> = (0..20).map(|i| 100.0 + i as f64).collect();
        let bars = bars_from_closes(&closes);
        let rsi = compute_rsi(&bars, 14);
        for i in 14..rsi.len() {
            assert_eq!(rsi[i], 100.0, "index {}", i);
        }
    }

    #[test]
    fn rsi_warmup_nan() {
        let closes: Vec<f64> = (0..10).map(|i| 50.0 + i as f64).collect();
        let bars = bars_from_closes(&closes);
        let rsi = compute_rsi(&bars, 14);
        assert_eq!(rsi.len(), 10);
        for v in &rsi {
            assert!(v.is_nan(), "10 bars with period 14 → all NaN");
        }
    }

    #[test]
    fn ema_seed_is_sma() {
        let closes: Vec<f64> = (1..=8).map(|i| i as f64).collect();
        let bars = bars_from_closes(&closes);
        let ema = compute_ema(&bars, 4);
        let sma = compute_sma(&bars, 4);
        for i in 0..3 {
            assert!(ema[i].is_nan());
            assert!(sma[i].is_nan());
        }
        assert!((ema[3] - sma[3]).abs() < 1e-12, "seed must equal SMA");
    }

    #[test]
    fn ema_recursion_hand_case() {
        // closes [1,2,3,4,5], p=3: alpha=0.5; seed ema[2]=2.0;
        // ema[3] = 4*0.5 + 2*0.5 = 3.0; ema[4] = 5*0.5 + 3*0.5 = 4.0.
        let closes = [1.0, 2.0, 3.0, 4.0, 5.0];
        let bars = bars_from_closes(&closes);
        let ema = compute_ema(&bars, 3);
        assert!((ema[2] - 2.0).abs() < 1e-12);
        assert!((ema[3] - 3.0).abs() < 1e-12);
        assert!((ema[4] - 4.0).abs() < 1e-12);
    }

    #[test]
    fn ema_handles_finite_values_whose_raw_seed_sum_overflows() {
        let closes = [f64::MAX, f64::MAX, f64::MAX, f64::MAX];
        let bars = bars_from_closes(&closes);
        let ema = compute_ema(&bars, 3);
        assert!(ema[..2].iter().all(|value| value.is_nan()));
        assert!(ema[2..].iter().all(|value| value.is_finite()));
        assert_eq!(ema[2], f64::MAX);
        assert_eq!(ema[3], f64::MAX);
    }

    #[test]
    fn sma_known() {
        let closes: Vec<f64> = (1..=10).map(|i| i as f64).collect();
        let bars = bars_from_closes(&closes);
        let sma = compute_sma(&bars, 5);
        assert!(sma[3].is_nan());
        assert!((sma[4] - 3.0).abs() < 1e-12);
        assert!((sma[9] - 8.0).abs() < 1e-12);
    }

    #[test]
    fn macd_hist_identity_and_cross_setup() {
        // Declining then rising closes → macd crosses its signal upward somewhere.
        let mut closes = Vec::new();
        for i in 0..15 {
            closes.push(100.0 - i as f64);
        }
        for i in 15..40 {
            closes.push(86.0 + (i - 14) as f64);
        }
        let bars = bars_from_closes(&closes);
        let macd = compute_macd(&bars, 3, 6, 3, "macd");
        let sig = compute_macd(&bars, 3, 6, 3, "signal");
        let hist = compute_macd(&bars, 3, 6, 3, "hist");
        let mut checked = 0;
        for i in 0..bars.len() {
            if !macd[i].is_nan() && !sig[i].is_nan() {
                assert!(!hist[i].is_nan());
                assert!(
                    (hist[i] - (macd[i] - sig[i])).abs() < 1e-9,
                    "hist = macd - signal violated at {}",
                    i
                );
                checked += 1;
            } else {
                assert!(hist[i].is_nan(), "hist must be NaN when inputs are NaN");
            }
        }
        assert!(checked > 10, "expected plenty of ready MACD points, got {}", checked);
        // macd becomes valid only when the slow EMA is ready (idx slow-1 = 5);
        // signal needs `signal_p` macd values → first valid at idx 5+3-1 = 7.
        assert!(macd[4].is_nan() && !macd[5].is_nan());
        assert!(sig[6].is_nan() && !sig[7].is_nan());
    }

    #[test]
    fn bollinger_symmetric_and_mid_is_sma() {
        let closes = [10.0, 12.0, 11.0, 13.0, 12.0, 14.0, 13.0, 15.0, 14.0, 16.0];
        let bars = bars_from_closes(&closes);
        let upper = compute_bollinger(&bars, 4, 2.0, "upper");
        let mid = compute_bollinger(&bars, 4, 2.0, "mid");
        let lower = compute_bollinger(&bars, 4, 2.0, "lower");
        let sma = compute_sma(&bars, 4);
        for i in 3..bars.len() {
            assert!((upper[i] + lower[i] - 2.0 * mid[i]).abs() < 1e-9, "index {}", i);
            assert!((mid[i] - sma[i]).abs() < 1e-12, "mid == SMA");
            assert!(upper[i] >= mid[i] && mid[i] >= lower[i]);
        }
        for i in 0..3 {
            assert!(mid[i].is_nan() && upper[i].is_nan() && lower[i].is_nan());
        }
    }

    #[test]
    fn stoch_bounds_on_random_walk() {
        // Seeded LCG (no rand crate) → deterministic pseudo-random walk.
        let mut seed = 0x2B_992D_FA5u64;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / ((u64::MAX >> 11) as f64)) * 2.0 - 1.0
        };
        let mut bars = Vec::new();
        let mut c = 100.0;
        for i in 0..200 {
            c += next() * 1.5;
            let h = c + next().abs() * 0.8 + 0.05;
            let l = c - next().abs() * 0.8 - 0.05;
            bars.push(BarData { t: (i + 1) as i64, o: c, h, l, c, v: 1000.0 });
        }
        let k = compute_stoch(&bars, 14, 3, "k");
        let d = compute_stoch(&bars, 14, 3, "d");
        for i in 13..bars.len() {
            assert!((0.0..=100.0).contains(&k[i]), "%K out of bounds at {}: {}", i, k[i]);
        }
        for i in 15..bars.len() {
            assert!(
                (-1e-9..=100.0 + 1e-9).contains(&d[i]),
                "%D out of bounds at {}: {}",
                i,
                d[i]
            );
        }
    }

    #[test]
    fn stoch_flat_series_guard() {
        let bars: Vec<BarData> = (0..20)
            .map(|i| BarData { t: (i + 1) as i64, o: 50.0, h: 50.0, l: 50.0, c: 50.0, v: 1.0 })
            .collect();
        let k = compute_stoch(&bars, 14, 3, "k");
        for i in 13..bars.len() {
            assert_eq!(k[i], 50.0, "HH == LL → 50.0 guard");
        }
    }

    #[test]
    fn atr_positive_after_warmup() {
        let bars: Vec<BarData> = (0..30)
            .map(|i| {
                let c = 100.0 + i as f64 * 0.3;
                BarData { t: (i + 1) as i64, o: c, h: c + 1.0, l: c - 1.0, c, v: 10.0 }
            })
            .collect();
        let atr = compute_atr(&bars, 14, "atr_abs");
        for i in 0..13 {
            assert!(atr[i].is_nan());
        }
        for i in 13..bars.len() {
            assert!(atr[i] > 0.0, "ATR must be positive at {}", i);
        }
        let pct = compute_atr(&bars, 14, "atr_pct");
        assert!((pct[20] - atr[20] / bars[20].c * 100.0).abs() < 1e-12);
    }

    #[test]
    fn atr_warmup_nan() {
        let bars: Vec<BarData> = (0..10)
            .map(|i| BarData { t: (i + 1) as i64, o: 10.0, h: 11.0, l: 9.0, c: 10.0, v: 1.0 })
            .collect();
        let atr = compute_atr(&bars, 14, "atr_abs");
        assert!(atr.iter().all(|v| v.is_nan()));
    }

    #[test]
    fn adx_range_and_warmup() {
        let mut seed: u64 = 0xCAFE_D00D;
        let mut next = || {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((seed >> 11) as f64 / ((u64::MAX >> 11) as f64)) * 2.0 - 1.0
        };
        let mut bars = Vec::new();
        let mut c = 100.0;
        for i in 0..120 {
            c += next() * 1.0;
            let h = c + next().abs() * 0.6 + 0.05;
            let l = c - next().abs() * 0.6 - 0.05;
            bars.push(BarData { t: (i + 1) as i64, o: c, h, l, c, v: 100.0 });
        }
        let adx = compute_adx(&bars, 5);
        for i in 0..9 {
            assert!(adx[i].is_nan(), "ADX must be NaN before 2*period-1 at {}", i);
        }
        for i in 9..bars.len() {
            assert!(
                (0.0..=100.0).contains(&adx[i]),
                "ADX out of range at {}: {}",
                i,
                adx[i]
            );
        }
    }

    #[test]
    fn vol_ratio_known() {
        let bars: Vec<BarData> = (0..12)
            .map(|i| BarData { t: (i + 1) as i64, o: 10.0, h: 10.0, l: 10.0, c: 10.0, v: 100.0 })
            .collect();
        let vr = compute_vol_ratio(&bars, 5);
        for i in 0..4 {
            assert!(vr[i].is_nan());
        }
        for i in 4..bars.len() {
            assert!((vr[i] - 1.0).abs() < 1e-12, "constant volume → ratio 1.0");
        }
    }

    #[test]
    fn price_close_and_change_pct() {
        let closes = [100.0, 110.0, 99.0];
        let bars = bars_from_closes(&closes);
        let close = price_series(&bars, "close");
        assert_eq!(close, vec![100.0, 110.0, 99.0]);
        let chg = price_series(&bars, "change_pct");
        assert!(chg[0].is_nan());
        assert!((chg[1] - 10.0).abs() < 1e-12);
        assert!((chg[2] - (-10.0)).abs() < 1e-12);
    }

    #[test]
    fn macd_signal_differs_from_v3_zero_padded_quirk() {
        // YOL HARITASI 2.2 kilit testi: v3 `macdCalc` signal hattini
        // null-iken-sifir-dolgulu seriden EMA'lar (tohum = ilk 9 dolgu
        // degerinin SMA'si — cogunlukla sifirlar). Bizim `compute_macd`
        // ilk 9 GECERLI macd'den tohumlanir. Bu test quirk'i aynen
        // kopyalayip farki kilitler — duzeltme sessizce geri alinamaz.
        let closes: Vec<f64> = (0..60).map(|i| 100.0 + i as f64 * 0.7 + ((i * 37) % 11) as f64 * 0.13).collect();
        let bars = bars_from_closes(&closes);
        let macd = compute_macd(&bars, 12, 26, 9, "macd");
        let sig = compute_macd(&bars, 12, 26, 9, "signal");
        let m0 = macd.iter().position(|v| !v.is_nan()).expect("macd hazir olmali");
        assert_eq!(m0, 25, "yavas EMA(26) idx 25'te hazir");
        // Bizim tohum: ilk 9 gecerli macd'in SMA'si, idx m0+8'de.
        let seed: f64 = macd[m0..m0 + 9].iter().sum::<f64>() / 9.0;
        assert!(seed.abs() > 1e-9, "test verisi trendli olmali (seed {seed})");
        assert!((sig[m0 + 8] - seed).abs() < 1e-9, "sig[{}] = {seed}, got {}", m0 + 8, sig[m0 + 8]);
        assert!(sig[m0 + 7].is_nan(), "tohumdan once NaN");
        // v3 quirk kopyasi: sifir-dolgulu EMA(9), idx 0'dan baslar.
        let padded: Vec<f64> = macd.iter().map(|v| if v.is_nan() { 0.0 } else { *v }).collect();
        let qseed: f64 = padded[0..9].iter().sum::<f64>() / 9.0;
        assert!(qseed.abs() < 1e-9, "quirk tohumu sifirlar ortalamasidir");
        let alpha = 2.0 / 10.0;
        let mut q = vec![f64::NAN; padded.len()];
        let mut prev = qseed;
        q[8] = prev;
        for i in 9..padded.len() {
            prev = padded[i] * alpha + prev * (1.0 - alpha);
            q[i] = prev;
        }
        // v3 signal'i maskeler (macd null ise null) ama DEGERLER kirlidir:
        // tohum idx'inde quirk degeri bizimkinden ayrilir.
        assert!(
            (q[m0 + 8] - sig[m0 + 8]).abs() > 1e-9,
            "quirk {} vs duzeltilmis {}",
            q[m0 + 8],
            sig[m0 + 8]
        );
    }

    #[test]
    fn compute_indicator_dispatch_matches_direct_calls() {
        let closes: Vec<f64> = (0..40).map(|i| 100.0 + (i as f64).sin() * 3.0).collect();
        let bars = bars_from_closes(&closes);
        let spec = IndicatorSpec {
            key: IndicatorKey::Ema,
            timeframe: Timeframe::M1,
            period: Some(5),
            fast: None,
            slow: None,
            signal: None,
            std_dev: None,
            d_period: None,
            output: None,
        };
        let via_dispatch = compute_indicator(&bars, &spec);
        let direct = compute_ema(&bars, 5);
        for i in 0..bars.len() {
            match (via_dispatch[i], direct[i]) {
                (a, b) if a.is_nan() && b.is_nan() => {}
                (a, b) => assert_eq!(a, b, "index {}", i),
            }
        }
    }
}
