//! Backtest Certifier — Walk-Forward Efficiency (WFE), Deflated Sharpe Ratio
//! (DSR), Probability of Backtest Overfitting (PBO via CSCV) and Monte Carlo
//! circular block bootstrap. Reporting-boundary statistics (f64, display only);
//! every equity path is produced by the exact-fraction kernel.

use crate::backtest::run_backtest;
use crate::{BotConfig, StrategyKind};
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use serde::{Deserialize, Serialize};

const EULER_GAMMA: f64 = 0.577_215_664_901_532_9;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CertifyOutput {
    /// Number of trial configurations evaluated.
    pub trials: usize,
    /// Mean out-of-sample / in-sample return ratio across CSCV folds (percent).
    pub wfe_pct: String,
    /// Deflated Sharpe Ratio probability in [0, 1] (Bailey & López de Prado).
    pub dsr: String,
    /// Probability of Backtest Overfitting via CSCV (percent, lower is better).
    pub pbo_pct: String,
    /// CSCV fold count used (S).
    pub blocks: usize,
    /// Number of CSCV combinations evaluated.
    pub combos: usize,
    /// Monte Carlo run count.
    pub mc_runs: usize,
    /// Fraction of Monte Carlo paths ending in loss (percent).
    pub mc_prob_loss_pct: String,
    /// 5th percentile terminal return (percent).
    pub mc_return_p5_pct: String,
    /// Median terminal return (percent).
    pub mc_return_p50_pct: String,
    /// 95th percentile terminal return (percent).
    pub mc_return_p95_pct: String,
    /// Mean max-drawdown across Monte Carlo paths (percent).
    pub mc_mean_mdd_pct: String,
    /// Warnings surfaced during certification (skipped trials, degenerate folds).
    pub warnings: Vec<String>,
}

/// Standard normal CDF (Abramowitz & Stegun 7.1.26 on erf, |err| < 1.5e-7).
pub fn norm_cdf(x: f64) -> f64 {
    let t = 1.0 / (1.0 + 0.231_641_9 * x.abs());
    let poly = t * (0.319_381_530
        + t * (-0.356_563_782
            + t * (1.781_477_937 + t * (-1.821_255_978 + t * 1.330_274_429))));
    let cdf = 1.0 - (-(x * x) / 2.0).exp() * poly / (2.0 * std::f64::consts::PI).sqrt();
    if x >= 0.0 {
        cdf
    } else {
        1.0 - cdf
    }
}

/// Inverse standard normal CDF (Acklam's rational approximation, rel. err < 1.15e-9).
pub fn inv_norm_cdf(p: f64) -> f64 {
    if !(0.0 < p && p < 1.0) {
        return 0.0;
    }
    let a: [f64; 6] = [-3.969_683_028_665_376e1, 2.209_460_984_245_205e2, -2.759_285_104_469_687e2, 1.383_577_518_672_690e2, -3.066_479_806_614_716e1, 2.506_628_277_459_239];
    let b: [f64; 5] = [-5.447_609_879_822_406e1, 1.615_858_368_580_409e2, -1.556_989_798_598_866e2, 6.680_131_188_771_972e1, -1.328_068_155_288_572e1];
    let c: [f64; 6] = [-7.784_894_002_430_293e-3, -3.223_964_580_411_365e-1, -2.400_758_277_161_838, -2.549_732_539_343_734, 4.374_664_141_464_968, 2.938_163_982_698_783];
    let d: [f64; 4] = [7.784_695_702_914_633e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996, 3.754_408_661_907_416];
    let p_low = 0.024_25;
    let p_high = 1.0 - p_low;
    if p < p_low {
        let q = (-2.0 * p.ln()).sqrt();
        (((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    } else if p <= p_high {
        let q = p - 0.5;
        let r = q * q;
        (((((a[0] * r + a[1]) * r + a[2]) * r + a[3]) * r + a[4]) * r + a[5]) * q
            / (((((b[0] * r + b[1]) * r + b[2]) * r + b[3]) * r + b[4]) * r + 1.0)
    } else {
        let q = (-2.0 * (1.0 - p).ln()).sqrt();
        -(((((c[0] * q + c[1]) * q + c[2]) * q + c[3]) * q + c[4]) * q + c[5])
            / ((((d[0] * q + d[1]) * q + d[2]) * q + d[3]) * q + 1.0)
    }
}

/// Deterministic xorshift64* RNG (reproducible certification output).
struct XorShift(u64);
impl XorShift {
    fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next_u64() % n.max(1)
    }
}

/// Per-trial summary used by CSCV / DSR.
struct TrialSeries {
    per_bar_returns: Vec<f64>, // display-boundary returns from exact equity curve
    total_return: f64,
    sharpe: f64,
}

fn series_sharpe(rets: &[f64]) -> f64 {
    if rets.len() < 2 {
        return 0.0;
    }
    let mean = rets.iter().sum::<f64>() / rets.len() as f64;
    let var = rets.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / (rets.len() - 1) as f64;
    let sd = var.sqrt();
    if sd < 1e-15 {
        0.0
    } else {
        mean / sd
    }
}

/// Exact-fraction config mutation: scale a decimal-string field by `factor`.
fn scale_field(field: &str, factor: &str) -> String {
    match (Fraction::from_decimal_str(field), Fraction::from_decimal_str(factor)) {
        (Ok(v), Ok(f)) => v.mul(&f).map(|r| r.to_half_even_display(12)).unwrap_or_else(|_| field.to_string()),
        _ => field.to_string(),
    }
}

/// Build the trial grid around the base configuration (up to 9 variants).
pub fn variant_configs(base: &BotConfig) -> Vec<BotConfig> {
    let mut trials = vec![base.clone()];
    let is_dca = matches!(base.kind, StrategyKind::DcaLong | StrategyKind::DcaShort | StrategyKind::FuturesDca);
    let dev = base.deviation_pct.clone();
    let step = base.step_scale.clone();
    let target = base.target_profit_pct.clone();

    for f in ["0.5", "1.5"] {
        if is_dca {
            let mut c = base.clone();
            c.deviation_pct = scale_field(&dev, f);
            trials.push(c);
        }
        let mut t = base.clone();
        t.target_profit_pct = scale_field(&target, f);
        trials.push(t);
    }
    if is_dca {
        for f in ["1.0", "1.5"] {
            let mut c = base.clone();
            c.step_scale = scale_field(&step, f);
            trials.push(c);
        }
        let mut plus = base.clone();
        plus.max_safety_orders = base.max_safety_orders.saturating_add(2);
        trials.push(plus);
        let mut minus = base.clone();
        minus.max_safety_orders = base.max_safety_orders.saturating_sub(2);
        trials.push(minus);
    } else if let Some(levels) = base.grid_levels {
        for dl in [2isize, -2] {
            let mut c = base.clone();
            c.grid_levels = Some((levels as isize + dl).max(3) as usize);
            trials.push(c);
        }
    }
    trials
}

fn run_trial(bars: &[dca_kernel::Bar], inst: &Instrument, cfg: &BotConfig, start: &Fraction, session: dca_math::sessions::SessionType) -> Result<TrialSeries, String> {
    // Certifier trials compare *relative* stability across bar splits; funding is
    // a near-uniform overlay within a split window and does not change split
    // ranking, so trials run without per-event funding (base run includes it).
    let res = run_backtest(bars, inst, cfg, start.clone(), session, &[])?;
    let curve = &res.equity_curve;
    let mut rets = Vec::with_capacity(curve.len());
    let mut prev: Option<f64> = None;
    for p in curve {
        let e = Fraction::from_any_str(&p.equity).map_err(|e| e.to_string())?.to_f64_display();
        if let Some(prev_e) = prev {
            if prev_e.abs() > 1e-15 {
                rets.push((e - prev_e) / prev_e);
            }
        }
        prev = Some(e);
    }
    let total = res
        .end_equity
        .parse::<f64>()
        .ok()
        .zip(res.start_equity.parse::<f64>().ok())
        .map(|(e, s)| if s.abs() > 1e-15 { (e - s) / s } else { 0.0 })
        .unwrap_or(0.0);
    Ok(TrialSeries { sharpe: series_sharpe(&rets), total_return: total, per_bar_returns: rets })
}

/// CSCV split indices for S blocks → all C(S, S/2) combinations (IS block set).
fn cscv_combos(s: usize) -> Vec<Vec<usize>> {
    let half = s / 2;
    let mut out = Vec::new();
    let mut combo = Vec::new();
    fn rec(start: usize, s: usize, half: usize, combo: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if combo.len() == half {
            out.push(combo.clone());
            return;
        }
        for i in start..s {
            combo.push(i);
            rec(i + 1, s, half, combo, out);
            combo.pop();
        }
    }
    rec(0, s, half, &mut combo, &mut out);
    out
}

fn block_slice_lengths(n: usize, s: usize) -> Vec<usize> {
    let base = n / s;
    let rem = n % s;
    (0..s).map(|i| base + if i < rem { 1 } else { 0 }).collect()
}

/// Certification entry point. `mc_runs` clamped to [50, 500].
pub fn certify(
    bars: &[dca_kernel::Bar],
    inst: &Instrument,
    cfg: &BotConfig,
    start_cash: Fraction,
    session: dca_math::sessions::SessionType,
    mc_runs: usize,
) -> Result<CertifyOutput, String> {
    if bars.len() < 20 {
        return Err("certify: need at least 20 bars".into());
    }
    let mc_runs = mc_runs.clamp(50, 500);
    let mut warnings = Vec::new();
    let configs = variant_configs(cfg);
    let mut trials: Vec<TrialSeries> = Vec::new();
    for (i, c) in configs.iter().enumerate() {
        match run_trial(bars, inst, c, &start_cash, session) {
            Ok(t) => trials.push(t),
            Err(e) => warnings.push(format!("trial {i} skipped: {e}")),
        }
    }
    if trials.is_empty() {
        return Err("certify: all trial backtests failed".into());
    }

    let n_bars = trials[0].per_bar_returns.len();
    // ---- CSCV: PBO + WFE ----
    let s = if n_bars >= 60 { 6 } else if n_bars >= 40 { 4 } else { 0 };
    let mut pbo = f64::NAN;
    let mut wfe = f64::NAN;
    let mut combos_done = 0usize;
    let mut blocks = 0usize;
    if s >= 4 {
        blocks = s;
        let lens = block_slice_lengths(n_bars, s);
        let combos = cscv_combos(s);
        let mut overfit = 0usize;
        let mut wfe_sum = 0.0f64;
        let mut wfe_cnt = 0usize;
        let starts: Vec<usize> = {
            let mut acc = 0usize;
            lens.iter().map(|l| { let st = acc; acc += l; st }).collect()
        };
        for is_set in &combos {
            let mut trial_is_sr = Vec::with_capacity(trials.len());
            let mut trial_oos_sr = Vec::with_capacity(trials.len());
            let mut trial_is_tot = Vec::with_capacity(trials.len());
            let mut trial_oos_tot = Vec::with_capacity(trials.len());
            for t in &trials {
                let mut is_rets = Vec::new();
                let mut oos_rets = Vec::new();
                for b in 0..s {
                    let seg: &[f64] = &t.per_bar_returns[starts[b]..starts[b] + lens[b]];
                    if is_set.contains(&b) {
                        is_rets.extend_from_slice(seg);
                    } else {
                        oos_rets.extend_from_slice(seg);
                    }
                }
                trial_is_sr.push(series_sharpe(&is_rets));
                trial_oos_sr.push(series_sharpe(&oos_rets));
                trial_is_tot.push(is_rets.iter().sum::<f64>());
                trial_oos_tot.push(oos_rets.iter().sum::<f64>());
            }
            // In-sample winner (Walk-Forward Optimizer selection)
            let best = (0..trials.len())
                .max_by(|&a, &b| trial_is_sr[a].partial_cmp(&trial_is_sr[b]).unwrap_or(std::cmp::Ordering::Equal))
                .unwrap_or(0);
            // OOS relative rank of the IS-best trial → overfit occurrence if below median
            let mut oos_rank: Vec<usize> = (0..trials.len()).collect();
            oos_rank.sort_by(|&a, &b| {
                trial_oos_sr[b].partial_cmp(&trial_oos_sr[a]).unwrap_or(std::cmp::Ordering::Equal)
            });
            let rank = oos_rank.iter().position(|&ti| ti == best).unwrap_or(0);
            let rel = (rank as f64 + 1.0) / (trials.len() as f64 + 1.0);
            if rel <= 0.5 {
                overfit += 1;
            }
            // WFE: best trial's OOS total return / IS total return
            if trial_is_tot[best] > 1e-12 {
                wfe_sum += trial_oos_tot[best] / trial_is_tot[best];
                wfe_cnt += 1;
            }
            combos_done += 1;
        }
        if combos_done > 0 {
            pbo = overfit as f64 / combos_done as f64 * 100.0;
        }
        if wfe_cnt > 0 {
            wfe = wfe_sum / wfe_cnt as f64 * 100.0;
        } else {
            warnings.push("WFE: no fold with positive in-sample return".into());
        }
    } else {
        warnings.push("CSCV skipped: fewer than 40 return observations".into());
    }

    // ---- DSR (Bailey & López de Prado 2014) ----
    let base = &trials[0];
    let rets = &base.per_bar_returns;
    let n = rets.len();
    let sr = base.sharpe;
    let srs: Vec<f64> = trials.iter().map(|t| t.sharpe).collect();
    let trials_n = srs.len() as f64;
    let sr_var = if srs.len() > 1 {
        let m = srs.iter().sum::<f64>() / trials_n;
        srs.iter().map(|s| (s - m) * (s - m)).sum::<f64>() / (srs.len() - 1) as f64
    } else {
        0.0
    };
    let em = 1.0 - 1.0 / trials_n.max(2.0);
    let e2 = 1.0 - 1.0 / (trials_n.max(2.0) * std::f64::consts::E);
    let sr0 = sr_var.sqrt() * ((1.0 - EULER_GAMMA) * inv_norm_cdf(em) + EULER_GAMMA * inv_norm_cdf(e2));
    let (m2, m3, m4) = {
        let mean = rets.iter().sum::<f64>() / n as f64;
        let mut m2 = 0.0; let mut m3 = 0.0; let mut m4 = 0.0;
        for r in rets {
            let d = r - mean;
            m2 += d * d; m3 += d * d * d; m4 += d * d * d * d;
        }
        (m2 / n as f64, m3 / n as f64, m4 / n as f64)
    };
    let skew = if m2 > 1e-18 { m3 / (m2 * m2.sqrt()) } else { 0.0 };
    let kurt = if m2 > 1e-18 { m4 / (m2 * m2) } else { 3.0 };
    let denom = 1.0 - skew * sr + ((kurt - 1.0) / 4.0) * sr * sr;
    let dsr = if denom > 1e-12 && n > 2 {
        let z = (sr - sr0) * ((n - 1) as f64).sqrt() / denom.sqrt();
        norm_cdf(z).clamp(0.0, 1.0)
    } else {
        0.0
    };

    // ---- Monte Carlo circular block bootstrap on base returns ----
    let mut rng = XorShift(0xDCA_901F_3B7C_5A11 ^ (n_bars as u64));
    let blen = ((n as f64).sqrt().round() as usize).clamp(2, 40).min(n);
    let mut finals = Vec::with_capacity(mc_runs);
    let mut mdds = Vec::with_capacity(mc_runs);
    for _ in 0..mc_runs {
        let mut eq = 1.0f64;
        let mut peak = 1.0f64;
        let mut mdd = 0.0f64;
        let mut filled = 0usize;
        while filled < n {
            let start_idx = rng.below(n as u64) as usize;
            for k in 0..blen {
                if filled >= n {
                    break;
                }
                let r = rets[(start_idx + k) % n];
                eq *= 1.0 + r;
                if eq > peak {
                    peak = eq;
                }
                if peak > 1e-12 {
                    let dd = (peak - eq) / peak;
                    if dd > mdd {
                        mdd = dd;
                    }
                }
                filled += 1;
            }
        }
        finals.push(eq - 1.0);
        mdds.push(mdd);
    }
    finals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let pct = |q: f64| -> f64 {
        if finals.is_empty() {
            return 0.0;
        }
        let idx = (q * (finals.len() - 1) as f64).round() as usize;
        finals[idx.min(finals.len() - 1)]
    };
    let prob_loss = finals.iter().filter(|r| **r < 0.0).count() as f64 / finals.len() as f64 * 100.0;
    let mean_mdd = mdds.iter().sum::<f64>() / mdds.len() as f64 * 100.0;

    Ok(CertifyOutput {
        trials: trials.len(),
        wfe_pct: fmt_pct(wfe),
        dsr: format!("{:.3}", dsr),
        pbo_pct: fmt_pct(pbo),
        blocks,
        combos: combos_done,
        mc_runs,
        mc_prob_loss_pct: format!("{:.1}", prob_loss),
        mc_return_p5_pct: fmt_pct(pct(0.05) * 100.0),
        mc_return_p50_pct: fmt_pct(pct(0.50) * 100.0),
        mc_return_p95_pct: fmt_pct(pct(0.95) * 100.0),
        mc_mean_mdd_pct: format!("{:.2}", mean_mdd),
        warnings,
    })
}

/// LIVE creation gate verdict (Faz 0.1).
///
/// v3 `buildCertificate` (ADR-006) 6 kapı ister; bizim `CertifyOutput`'ta OOS
/// getirisi ve MinTRL ayrı alan olarak yok, o yüzden geçit **bizim Lab
/// rozetlerinin "good" eşiklerine** kilitlidir (tek doğruluk kaynağı — rozette
/// yeşil yanan, kapıdan da geçer):
///  · trades ≥ 5 (v3 §10 minimum; baz koşudaki kapanmış deal sayısı)
///  · WFE ≥ %50 (v3 `wfe > 0.5` oranının yüzde karşılığı)
///  · DSR ≥ 0.95 (bizim DSR bir olasılık; v3 `z > 0`'dan sıkı, Lab ile aynı)
///  · PBO ≤ %20 (v3 `< 0.2` kesrinin yüzde karşılığı; Lab ile aynı)
/// Çözümlenemeyen metrik ("n/a", NaN) = KALDI (fail-closed).
/// MC istatistikleri kapıda kullanılmaz — bilgi amaçlıdır.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertGate {
    pub key: &'static str,
    pub label: &'static str,
    pub value: String,
    pub threshold: &'static str,
    pub pass: bool,
    pub detail: &'static str,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CertGateVerdict {
    pub verdict: &'static str,
    pub passed: usize,
    pub total: usize,
    pub gates: Vec<CertGate>,
}

fn parse_metric(s: &str) -> Option<f64> {
    let v: f64 = s.trim().trim_end_matches('%').parse().ok()?;
    if v.is_finite() { Some(v) } else { None }
}

/// `base_deals`: aynı pencerede koşan baz backtest'in kapanmış deal sayısı.
/// `funding_ok`: vadeli türlerde funding geçmişi alınabildi mi (alınamadıysa
/// sertifika funding=0 varsaymıştır — LIVE kapısında bu iyimserlik geçersizdir).
/// `is_futures`: vadeli tür mü (spot'ta funding kapısı uygulanmaz).
pub fn certify_verdict(cert: &CertifyOutput, base_deals: usize, funding_ok: bool, is_futures: bool) -> CertGateVerdict {
    let wfe = parse_metric(&cert.wfe_pct);
    let dsr = parse_metric(&cert.dsr);
    let pbo = parse_metric(&cert.pbo_pct);
    let funding_pass = !is_futures || funding_ok;
    let gates = vec![
        CertGate {
            key: "trades",
            label: "Örneklem büyüklüğü",
            value: base_deals.to_string(),
            threshold: "≥ 5 deal",
            pass: base_deals >= 5,
            detail: "Az işlem = istatistik anlamsız (v3 §10: ≥30 ideal, 5 minimum).",
        },
        CertGate {
            key: "wfe",
            label: "Walk-Forward Verimliliği",
            value: cert.wfe_pct.clone(),
            threshold: "≥ %50",
            pass: wfe.map(|v| v >= 50.0).unwrap_or(false),
            detail: "OOS/IS getirisi; IS negatifse payda koruması WFE'yi düşürür.",
        },
        CertGate {
            key: "dsr",
            label: "Deflated Sharpe (olasılık)",
            value: cert.dsr.clone(),
            threshold: "≥ 0.95",
            pass: dsr.map(|v| v >= 0.95).unwrap_or(false),
            detail: "Deneme varyansı ile söndürülmüş Sharpe olasılığı (Lab rozetiyle aynı eşik).",
        },
        CertGate {
            key: "pbo",
            label: "Overfit olasılığı (PBO)",
            value: cert.pbo_pct.clone(),
            threshold: "≤ %20",
            pass: pbo.map(|v| v <= 20.0).unwrap_or(false),
            detail: "CSCV kombinatoryal aşırı-uyum olasılığı (Lab rozetiyle aynı eşik).",
        },
        CertGate {
            key: "funding",
            label: "Funding geçmişi (vadeli)",
            value: if !is_futures { "n/a (spot)".into() } else if funding_ok { "alındı".into() } else { "ALINAMADI".into() },
            threshold: "vadeli: alınmalı",
            pass: funding_pass,
            detail: "Funding alınamazsa sertifika funding=0 varsayar — LIVE'da bu iyimserlik geçersiz.",
        },
    ];
    let passed = gates.iter().filter(|g| g.pass).count();
    CertGateVerdict {
        verdict: if passed == gates.len() { "GECTI" } else { "KALDI" },
        passed,
        total: gates.len(),
        gates,
    }
}

fn fmt_pct(v: f64) -> String {
    if v.is_finite() {
        format!("{:.1}", v)
    } else {
        "n/a".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SizingMode;

    fn synth_bars(n: usize, seed: u64) -> Vec<dca_kernel::Bar> {
        let mut rng = XorShift(seed);
        let mut bars = Vec::with_capacity(n);
        let mut px = 2000.0f64;
        for i in 0..n {
            // mean-reverting drift so DCA deals actually cycle
            let drift = (1.05 - px / 2000.0) * 0.02 + (rng.next_f64() - 0.5) * 0.02;
            let o = px;
            let c = (px * (1.0 + drift)).max(100.0);
            let hi = o.max(c) * (1.0 + rng.next_f64() * 0.005);
            let lo = o.min(c) * (1.0 - rng.next_f64() * 0.005);
            bars.push(dca_kernel::Bar {
                t: 1_700_000_000_000i64 + (i as i64) * 60_000,
                o: format!("{o:.2}"),
                h: format!("{hi:.2}"),
                l: format!("{lo:.2}"),
                c: format!("{c:.2}"),
                v: "10".into(),
            });
            px = c;
        }
        bars
    }

    fn base_cfg() -> BotConfig {
        BotConfig {
            kind: StrategyKind::DcaLong,
            symbol: "TESTUSDT".into(),
            base_order_size: "50".into(),
            safety_order_size: "50".into(),
            sizing_mode: SizingMode::QuoteNotional,
            deviation_pct: "0.02".into(),
            step_scale: "1.3".into(),
            volume_scale: "1.2".into(),
            max_safety_orders: 4,
            target_profit_pct: "0.01".into(),
            use_net_tp: false,
            trailing_enabled: false,
            trailing_trigger_pct: "0.01".into(),
            trailing_deviation_pct: "0.005".into(),
            stop_loss_pct: None,
            leverage: "1".into(),
            hedge_ratio: "1".into(),
            combined_target_profit: "0.02".into(),
            hedge_drawdown_trigger_pct: "6".into(),
            grid_lower: None,
            grid_upper: None,
            grid_levels: None,
            grid_mode: None,
            grid_trail: false,
            cooldown_bars: 0,
            entry_trigger: crate::EntryTrigger::default(),
            entry_signal: None,
            pine_script: None,
            pine_inputs: std::collections::BTreeMap::new(),
            risk_policy: dca_kernel::UserRiskPolicy::default(),
        }
    }

    #[test]
    fn norm_cdf_symmetric() {
        assert!((norm_cdf(0.0) - 0.5).abs() < 1e-9);
        assert!((norm_cdf(1.96) - 0.975).abs() < 2e-4);
        assert!((norm_cdf(-1.96) - 0.025).abs() < 2e-4);
    }

    #[test]
    fn inv_norm_cdf_accurate() {
        assert!((inv_norm_cdf(0.975) - 1.959_964).abs() < 1e-4);
        assert!((inv_norm_cdf(0.5) - 0.0).abs() < 1e-9);
        assert!((inv_norm_cdf(0.01) + 2.326_348).abs() < 1e-4);
        assert_eq!(inv_norm_cdf(0.0), 0.0);
        assert_eq!(inv_norm_cdf(1.0), 0.0);
    }

    #[test]
    fn cscv_combos_count() {
        assert_eq!(cscv_combos(6).len(), 20); // C(6,3)
        assert_eq!(cscv_combos(4).len(), 6); // C(4,2)
        assert!(cscv_combos(6).iter().all(|c| c.len() == 3));
    }

    #[test]
    fn certify_produces_bounded_stats() {
        let bars = synth_bars(240, 42);
        let inst = Instrument::default();
        let cfg = base_cfg();
        let start = Fraction::from_decimal_str("10000").unwrap();
        let out = certify(&bars, &inst, &cfg, start, dca_math::sessions::SessionType::Crypto24_7, 120).expect("certify ok");
        assert!(out.trials >= 5, "expected >= 5 trials, got {}", out.trials);
        let pbo: f64 = out.pbo_pct.trim_end_matches('%').parse().unwrap_or(f64::NAN);
        assert!(pbo >= 0.0 && pbo <= 100.0, "pbo out of range: {pbo}");
        let dsr: f64 = out.dsr.parse().unwrap_or(f64::NAN);
        assert!((0.0..=1.0).contains(&dsr), "dsr out of range: {dsr}");
        assert_eq!(out.combos, 20);
        assert_eq!(out.blocks, 6);
        assert_eq!(out.mc_runs, 120);
        let p5: f64 = out.mc_return_p5_pct.parse().unwrap_or(f64::NAN);
        let p50: f64 = out.mc_return_p50_pct.parse().unwrap_or(f64::NAN);
        let p95: f64 = out.mc_return_p95_pct.parse().unwrap_or(f64::NAN);
        assert!(p5 <= p50 && p50 <= p95, "percentiles not ordered: {p5} {p50} {p95}");
        if !out.warnings.is_empty() {
            // warnings allowed, but WFE must either parse or be n/a
            assert!(out.wfe_pct == "n/a" || out.wfe_pct.parse::<f64>().is_ok());
        }
    }

    #[test]
    fn certify_deterministic() {
        let bars = synth_bars(120, 7);
        let inst = Instrument::default();
        let cfg = base_cfg();
        let start = Fraction::from_decimal_str("10000").unwrap();
        let a = certify(&bars, &inst, &cfg, start.clone(), dca_math::sessions::SessionType::Crypto24_7, 80).unwrap();
        let b = certify(&bars, &inst, &cfg, start, dca_math::sessions::SessionType::Crypto24_7, 80).unwrap();
        assert_eq!(a.dsr, b.dsr);
        assert_eq!(a.mc_return_p5_pct, b.mc_return_p5_pct);
        assert_eq!(a.pbo_pct, b.pbo_pct);
    }

    fn gate_cert(wfe: &str, dsr: &str, pbo: &str) -> CertifyOutput {
        CertifyOutput {
            trials: 9,
            wfe_pct: wfe.into(),
            dsr: dsr.into(),
            pbo_pct: pbo.into(),
            blocks: 6,
            combos: 20,
            mc_runs: 100,
            mc_prob_loss_pct: "10.0".into(),
            mc_return_p5_pct: "-2.0".into(),
            mc_return_p50_pct: "5.0".into(),
            mc_return_p95_pct: "12.0".into(),
            mc_mean_mdd_pct: "3.00".into(),
            warnings: vec![],
        }
    }

    #[test]
    fn gate_passes_on_lab_good_cutoffs() {
        let v = certify_verdict(&gate_cert("62.3", "0.970", "12.4"), 9, true, true);
        assert_eq!(v.verdict, "GECTI");
        assert_eq!(v.passed, v.total);
    }

    #[test]
    fn gate_fails_each_metric_fail_closed() {
        // trades < 5
        let v = certify_verdict(&gate_cert("62.3", "0.970", "12.4"), 4, true, false);
        assert_eq!(v.verdict, "KALDI");
        // wfe < 50
        let v = certify_verdict(&gate_cert("49.9", "0.970", "12.4"), 9, true, false);
        assert_eq!(v.verdict, "KALDI");
        // dsr < 0.95
        let v = certify_verdict(&gate_cert("62.3", "0.940", "12.4"), 9, true, false);
        assert_eq!(v.verdict, "KALDI");
        // pbo > 20
        let v = certify_verdict(&gate_cert("62.3", "0.970", "20.1"), 9, true, false);
        assert_eq!(v.verdict, "KALDI");
        // çözümlenemeyen metrik = KALDI (fail-closed)
        let v = certify_verdict(&gate_cert("n/a", "0.970", "12.4"), 9, true, false);
        assert_eq!(v.verdict, "KALDI");
        let v = certify_verdict(&gate_cert("62.3", "NaN", "12.4"), 9, true, false);
        assert_eq!(v.verdict, "KALDI");
    }

    #[test]
    fn gate_funding_fail_closed_for_futures_only() {
        let ok_spot = certify_verdict(&gate_cert("62.3", "0.970", "12.4"), 9, false, false);
        assert_eq!(ok_spot.verdict, "GECTI");
        let bad_fut = certify_verdict(&gate_cert("62.3", "0.970", "12.4"), 9, false, true);
        assert_eq!(bad_fut.verdict, "KALDI");
        assert!(bad_fut.gates.iter().any(|g| g.key == "funding" && !g.pass));
    }

    #[test]
    fn variant_grid_shape() {
        let cfg = base_cfg();
        assert_eq!(variant_configs(&cfg).len(), 9);
        let mut g = cfg.clone();
        g.kind = StrategyKind::SpotGrid;
        g.grid_levels = Some(8);
        assert_eq!(variant_configs(&g).len(), 5);
    }

}
