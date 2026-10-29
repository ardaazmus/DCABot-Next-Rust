//! Backtest runner: drives bar history through the unified kernel + strategy
//! engines. Produces exact equity curves (fractions), fill logs, gate logs and
//! display-boundary metrics.

use crate::dca::DcaEngine;
use crate::grid::GridEngine;
use crate::hedge::HedgeEngine;
use crate::{Action, BarContext, BotConfig, StrategyKind};
use dca_kernel::{Bar, OrderIntent, OrderKind, SimulationStepper, StepperConfig};
use dca_math::fraction::Fraction;
use dca_math::m03_m07::{liquidation_price, update_drawdown, DrawdownState};
use dca_math::Instrument;
use dca_signals::{BarData, SignalEngine, Timeframe};
use serde::Serialize;
use std::collections::BTreeMap;

/* ------------------------------------------------------------------ */
/* Composite signal helpers (3Commas-style entry conditions)           */
/* ------------------------------------------------------------------ */

pub fn tf_ms(tf: Timeframe) -> i64 {
    match tf {
        Timeframe::M1 => 60_000,
        Timeframe::M5 => 300_000,
        Timeframe::M15 => 900_000,
        Timeframe::H1 => 3_600_000,
        Timeframe::H4 => 14_400_000,
        Timeframe::D1 => 86_400_000,
    }
}

/// Exact bucket duration → Timeframe (None for unsupported intervals).
pub fn tf_from_ms(ms: i64) -> Option<Timeframe> {
    Some(match ms {
        60_000 => Timeframe::M1,
        300_000 => Timeframe::M5,
        900_000 => Timeframe::M15,
        3_600_000 => Timeframe::H1,
        14_400_000 => Timeframe::H4,
        86_400_000 => Timeframe::D1,
        _ => return None,
    })
}

/// Convert kernel bars (string decimals) to signal-engine bars (f64 metrics).
pub fn bars_to_signal(bars: &[Bar]) -> Vec<BarData> {
    bars.iter()
        .map(|b| BarData {
            t: b.t,
            o: b.o.parse().unwrap_or(f64::NAN),
            h: b.h.parse().unwrap_or(f64::NAN),
            l: b.l.parse().unwrap_or(f64::NAN),
            c: b.c.parse().unwrap_or(f64::NAN),
            v: b.v.parse().unwrap_or(0.0),
        })
        .collect()
}

/// Aggregate a series into `bucket_ms` bars (UTC-aligned epoch buckets).
/// Zero look-ahead: output bar i only contains source bars within its bucket.
pub fn resample(bars: &[BarData], bucket_ms: i64) -> Vec<BarData> {
    let mut out: Vec<BarData> = Vec::new();
    for b in bars {
        if b.o.is_nan() || b.c.is_nan() {
            continue;
        }
        let bt = (b.t / bucket_ms) * bucket_ms;
        match out.last_mut() {
            Some(last) if last.t == bt => {
                last.h = last.h.max(b.h);
                last.l = last.l.min(b.l);
                last.c = b.c;
                last.v += b.v;
            }
            _ => out.push(BarData { t: bt, o: b.o, h: b.h, l: b.l, c: b.c, v: b.v }),
        }
    }
    out
}

/// Build the signal engine for a backtest run: the bot interval plus every
/// higher timeframe referenced by the tree (lower TFs are rejected — we only
/// have bot-interval bars; zero look-ahead cannot be guaranteed for them).
pub fn build_backtest_signal(bars: &[Bar], root: &dca_signals::SignalNode) -> Result<(SignalEngine, i64), String> {
    dca_signals::validate_signal_node(root)?;
    let bot_ms = if bars.len() >= 2 { bars[1].t - bars[0].t } else { return Err("backtest: en az 2 bar gerekli".into()) };
    let bot_tf = tf_from_ms(bot_ms).ok_or_else(|| format!("backtest: desteklenmeyen bar aralığı ({bot_ms} ms)"))?;
    let base = bars_to_signal(bars);
    let mut needed: Vec<Timeframe> = SignalEngine::timeframes_needed(root);
    needed.retain(|t| *t != bot_tf);
    let mut series: Vec<(Timeframe, Vec<BarData>)> = vec![(bot_tf, base)];
    for tf in needed {
        if tf_ms(tf) < bot_ms {
            return Err(format!(
                "sinyal zaman dilimi {:?} bot barından küçük olamaz (zero look-ahead)",
                tf
            ));
        }
        let up = resample(&series[0].1, tf_ms(tf));
        series.push((tf, up));
    }
    let specs = SignalEngine::specs_needed(root);
    let engine = SignalEngine::new(&series, &specs);
    let asof = bars.last().map(|b| b.t).unwrap_or(0);
    Ok((engine, asof))
}

#[derive(Debug, Clone, Serialize)]
pub struct EquityPoint {
    pub ts: i64,
    pub equity: String, // exact fraction string (UI formats half-even)
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestDeal {
    pub bot_id: String,
    pub opened_ts: i64,
    pub closed_ts: i64,
    pub gross_pnl: String,
    pub fees: String,
    pub safety_triggered: usize,
    pub role: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct BacktestMetrics {
    pub total_return_pct: String,
    pub max_drawdown_pct: String,
    pub win_rate_pct: String,
    pub profit_factor: String,
    pub sharpe_display: String, // f64 display-only (reporting boundary)
    pub total_fees: String,
    pub total_funding: String,
    pub bars: usize,
    pub deals: usize,
    /// Task 19 — dolum gerçekçiliği ayrıştırıcı kırılım (fill modeli dağılımı).
    pub fill_realism: FillRealismBreakdown,
}

/// Dolum gerçekçiliği kırılımı: her fill'in model etiketi + maker/taker
/// komisyon ayrışı. İyimser modda RestingTouch sayısı şişer — canlıda o
/// emirlerin çoğu dolmaz; gerçekçi mod ile karşılaştırma içindir.
#[derive(Debug, Clone, Serialize, Default)]
pub struct FillRealismBreakdown {
    pub marketable_taker: usize,
    pub resting_touch: usize,
    pub open_gap_better: usize,
    pub maker_fees: String,
    pub taker_fees: String,
}

impl FillRealismBreakdown {
    fn from_fills(fills: &[dca_kernel::FillRecord]) -> Self {
        let mut b = Self::default();
        let mut maker = Fraction::zero();
        let mut taker = Fraction::zero();
        for f in fills {
            match f.realism {
                dca_risk::FillRealism::MarketableTaker => b.marketable_taker += 1,
                dca_risk::FillRealism::RestingTouch => b.resting_touch += 1,
                dca_risk::FillRealism::OpenGapBetter => b.open_gap_better += 1,
            }
            match f.fee_role {
                dca_kernel::FeeRole::Maker => maker = maker.add(&f.fee).unwrap_or(maker),
                dca_kernel::FeeRole::Taker => taker = taker.add(&f.fee).unwrap_or(taker),
            }
        }
        b.maker_fees = maker.to_half_even_display(8);
        b.taker_fees = taker.to_half_even_display(8);
        b
    }
}

/// Task 22 — makine-okunur no-deal nedeni. Yalnız motorun o koşuda GERÇEKTEN
/// verdiği engel kararları sayılır (sayaçlar try_open_deal / bootstrap
/// dallarındadır); uydurma neden üretilmez.
#[derive(Debug, Clone, Serialize)]
pub struct NoDealReason {
    pub code: String,
    pub bars: u64,
    pub detail: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct BacktestResult {
    pub equity_curve: Vec<EquityPoint>,
    pub fills: Vec<dca_kernel::FillRecord>,
    pub gate_logs: Vec<dca_kernel::GateLog>,
    pub metrics: BacktestMetrics,
    pub start_equity: String,
    pub end_equity: String,
    /// Task 19 — Pine strateji uyarıları (pyramiding vb.); pine yoksa boş.
    #[serde(default)]
    pub pine_warnings: Vec<String>,
    /// Task 20 — Pine derleme-anı dürüst notları (koşullu ta.* uyarısı,
    /// pyramiding parametresi); pine yoksa boş.
    #[serde(default)]
    pub pine_notes: Vec<String>,
    /// Task 20 — plot("başlık", seri) değerleri (bar-dizini hizalı); pine yoksa boş.
    #[serde(default)]
    pub pine_plots: BTreeMap<String, Vec<String>>,
    /// Task 22 — boş/flat sonuçların makine-okunur nedenleri (gözlemlenen
    /// engel kararları; boş değilse UI "Neden deal açılmadı" kutusunda gösterir).
    #[serde(default)]
    pub no_deal_reasons: Vec<NoDealReason>,
}

pub fn run_backtest(
    bars: &[Bar],
    inst: &Instrument,
    cfg: &BotConfig,
    start_cash: Fraction,
    session: dca_math::sessions::SessionType,
    funding: &[(i64, String)],
) -> Result<BacktestResult, String> {
    let margin_mode = !cfg.kind.is_futures(); // is_spot flag: futures kinds trade linear margin
    let stepper_lev = if margin_mode {
        Fraction::one()
    } else {
        Fraction::from_decimal_str(&cfg.leverage).unwrap_or_else(|_| Fraction::one())
    };
    let mut stepper = SimulationStepper::new(
        inst.clone(),
        start_cash.clone(),
        StepperConfig {
            session_type: session,
            capital_shortfall_mode: dca_risk::CapitalShortfallMode::Wait,
            is_spot: margin_mode,
            leverage: stepper_lev.clone(),
            // Task 16: backtest = live paritesi — politika bar-through geçer.
            policy: cfg.risk_policy.clone(),
            ..Default::default()
        },
    );
    let pos_dir: i8 = if cfg.kind.direction() == Some(dca_math::m02::Direction::Short) { -1 } else { 1 };
    stepper.set_position_direction(pos_dir);
    // Composite signal tree (3Commas-style): precompute per-TF series once;
    // per-bar evaluation is timestamp-guarded so future bars never leak back.
    let sig = match &cfg.entry_signal {
        Some(sc) => Some(build_backtest_signal(bars, &sc.root).map(|(e, _)| (e, sc.root.clone()))?),
        None => None,
    };
    // Task 19 — Pine strateji: TÜM barlar üzerinden BİR KEZ koşturulur; dca_pine
    // zero look-ahead garantilidir (bar i kararı yalnız i'ye kadar barları görür).
    let pine_res = match &cfg.pine_script {
        Some(code) => {
            let inputs: dca_pine::PineInputs = cfg.pine_inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            Some(dca_pine::run_all(code, bars, &inputs)?)
        }
        None => None,
    };

    // Faz 13B-backtest: hedge türleri İKİ stepper ile koşar — ana (yön +1) ve
    // hedge (yön −1) bacakların kernel defterleri canlıdaki gibi ayrıdır (EX-06).
    let is_hedge_kind = matches!(cfg.kind, StrategyKind::HedgeProtective | StrategyKind::HedgeDual);
    let mut hedge_stepper: Option<SimulationStepper> = if is_hedge_kind {
        let mut hs = SimulationStepper::new(
            inst.clone(),
            start_cash.clone(),
            StepperConfig {
                session_type: session,
                capital_shortfall_mode: dca_risk::CapitalShortfallMode::Wait,
                is_spot: margin_mode,
                leverage: stepper_lev,
                policy: cfg.risk_policy.clone(),
                ..Default::default()
            },
        );
        hs.set_position_direction(-1);
        Some(hs)
    } else {
        None
    };
    let mut engine: EngineBox = match cfg.kind {
        StrategyKind::SpotGrid | StrategyKind::FuturesGrid | StrategyKind::InfinityGrid => {
            let mark0 = Fraction::from_decimal_str(&bars.first().ok_or("no bars")?.c).map_err(|e| e.to_string())?;
            EngineBox::Grid(GridEngine::new("bt", cfg.clone(), inst.clone(), &mark0)?)
        }
        StrategyKind::HedgeProtective | StrategyKind::HedgeDual => {
            EngineBox::Hedge(HedgeEngine::new("bt", cfg.clone(), inst.clone())?)
        }
        _ => EngineBox::Dca(DcaEngine::new("bt", cfg.clone(), inst.clone())?),
    };
    // Task 20 — pine derleme notları (koşullu ta.* / pyramiding uyarıları)
    let pine_notes: Vec<String> = cfg
        .pine_script
        .as_ref()
        .and_then(|code| dca_pine::validate_source(code).ok().map(|p| p.notes))
        .unwrap_or_default();

    let mut bootstrap_done = false;
    // Task 22 — grid bootstrap alt-koşul ıskaları (backtest döngüsü karar verir).
    let mut grid_miss_trigger_bars: u64 = 0;
    let mut grid_miss_signal_bars: u64 = 0;
    let mut equity_curve = Vec::new();
    let mut dd_state = DrawdownState::default();
    let mut all_fills = Vec::new();
    let mut start_equity: Option<Fraction> = None;
    let mut grid_level_map: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // Funding ledger (futures only): ascending events of (ts, rate).
    let mut funding_events: Vec<(i64, Fraction)> = funding
        .iter()
        .filter_map(|(ts, r)| Fraction::from_any_str(r).ok().map(|f| (*ts, f)))
        .collect();
    funding_events.sort_by_key(|x| x.0);
    let mut f_cursor = 0usize;
    let mut f_cursor_h = 0usize;
    let mut total_funding = Fraction::zero();
    let mut prev_bar_t: Option<i64> = None;
    // Task 16 — günlük DD devre kesici (spec 2.9): UTC gün başına eşik.
    // Task 18 N-1: eşik yüzde string'i → fraksiyona (/100) çevrilir; 0 = kapalı.
    let breaker_active = cfg.risk_policy.daily_dd_breaker_active();
    let breaker_threshold = dca_risk::daily_dd_breaker_threshold(&cfg.risk_policy.daily_dd_breaker_pct);
    let mut breaker_day: i64 = -1;
    let mut breaker_day_start_eq: Option<Fraction> = None;
    let mut breaker_tripped = false;
    // Task 16 — rejim sınıflandırma penceresi (kapalı bar kapanışları).
    let mut regime_closes: Vec<f64> = Vec::new();

    for (bar_i, bar) in bars.iter().enumerate() {
        let o = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
        let h = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
        let l = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
        let c = Fraction::from_decimal_str(&bar.c).map_err(|e| e.to_string())?;

        // Composite signal verdict for this closed bar (None = no tree configured)
        // ANDed with the Pine strategy verdict when a pine_script is configured.
        let pine_v = pine_res.as_ref().and_then(|r| r.verdicts.get(bar_i));
        let signal_ok = {
            let tree_ok = sig
                .as_ref()
                .map(|(eng, root)| eng.evaluate_at_time(root, bar.t));
            let pine_ok = pine_v.map(|v| v.entry);
            match (tree_ok, pine_ok) {
                (Some(a), Some(b)) => Some(a && b),
                (Some(a), None) => Some(a),
                (None, Some(b)) => Some(b),
                (None, None) => None,
            }
        };

        let fills = stepper.step(bar, bar.t, false).map_err(|e| e.to_string())?;
        // Faz 13B-backtest: hedge bacak stepper'ı AYRI kernel olarak step edilir
        // ve fill'leri prefix routing'iyle EngineBox::Hedge'e iletilir.
        let hedge_fills = match hedge_stepper.as_mut() {
            Some(hs) => hs.step(bar, bar.t, false).map_err(|e| e.to_string())?,
            None => Vec::new(),
        };

        // Task 16 — rejim sınıflandırma (zero look-ahead: yalnız kapalı barlar).
        regime_closes.push(c.to_f64_display());
        if regime_closes.len() > 200 {
            regime_closes.remove(0);
        }
        let regime = dca_signals::classify_regime(&regime_closes);

        // Task 16 — günlük DD devre kesici: UTC günü değişince çapa sıfırlanır;
        // eşik aşılırsa bugün yeni deal açılışı bloklanır (TP/SL etkilenmez).
        let day = bar.t / 86_400_000;
        if day != breaker_day {
            breaker_day = day;
            breaker_day_start_eq = None;
            breaker_tripped = false;
        }
        if breaker_active && !breaker_tripped {
            let eq_now = stepper.equity(&c).unwrap_or_else(|_| Fraction::zero());
            let anchor = breaker_day_start_eq.get_or_insert_with(|| eq_now.clone());
            if !anchor.is_zero() {
                if let Ok(dd) = anchor.sub(&eq_now).and_then(|x| x.div(anchor)) {
                    if dd >= breaker_threshold {
                        breaker_tripped = true;
                        stepper.log_advisory_pub(bar.t, "RISK", "DAILY_DD_BREAKER", &format!(
                            "Günlük DD devre kesici tetiklendi: gün içi düşüş {} ≥ eşik {}% — bugün yeni deal açılışı durduruldu (mevcut pozisyonun TP/SL yönetimi sürüyor)",
                            dd.mul(&Fraction::from_u64(100)).map(|x| x.to_half_even_display(2)).unwrap_or_else(|_| "?".into()),
                            cfg.risk_policy.daily_dd_breaker_pct
                        ));
                    }
                }
            }
        }

        // Futures margin guard — M07 liquidation inside the bar's range.
        if !stepper.cfg.is_spot && stepper.cfg.leverage > Fraction::one() && !stepper.position.qty.is_zero() {
            if let (Some(avg), true) = (stepper.position.avg_price.clone(), true) {
                let qty = stepper.position.qty.clone();
                let brackets = vec![dca_math::MmrBracket {
                    min_notional: Fraction::zero(),
                    max_notional: Fraction::from_u64(u64::MAX),
                    mmr: Fraction::from_decimal_str("0.005").unwrap(),
                    deduction: Fraction::zero(),
                }];
                if let Ok(Some(liq)) = liquidation_price(stepper.position.direction, &qty, &avg, &stepper.im_used, &brackets, &Fraction::zero()) {
                    let lo = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
                    let hi = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
                    let op = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
                    let crossed = if stepper.position.direction == 1 { lo <= liq } else { hi >= liq };
                    if crossed {
                        let liq_px = if stepper.position.direction == 1 { Fraction::min(&op, &liq) } else { Fraction::max(&op, &liq) };
                        stepper.cancel_pending();
                        if let Some(fill) = stepper.force_close(&liq_px, bar.t, "LIQ").map_err(|e| e.to_string())? {
                            all_fills.push(fill.clone());
                            let acts = match &mut engine {
                                EngineBox::Dca(e) => {
                                    e.on_fill(&fill);
                                    e.force_liquidate();
                                    Vec::new()
                                }
                                EngineBox::Grid(_) => Vec::new(),
                                EngineBox::Hedge(h) => {
                                    // Faz 13B: ana bacak LIQ — hedge bacağı kendi
                                    // stepper'ıyla izlenmeye devam eder.
                                    h.on_fill_leg(&fill);
                                    h.main.force_liquidate();
                                    Vec::new()
                                }
                            };
                            for a in acts {
                                dispatch_leg(&mut stepper, hedge_stepper.as_mut(), a, bar.t)?;
                            }
                        }
                    }
                }
            }
        }

        if !bootstrap_done {
            match &mut engine {
                EngineBox::Grid(g) => {
                    // Start-condition gate: mesh orders wait until the trigger passes
                    // AND the composite signal tree is satisfied (fail-closed).
                    // Task 22 — hangi alt-koşulun takıldığı sayılır (no-deal nedeni).
                    let trigger_ok = crate::evaluate_entry_trigger(&g.cfg.entry_trigger, &o, bar.t, None).unwrap_or(false);
                    let signal_pass = signal_ok.unwrap_or(true);
                    if !trigger_ok {
                        grid_miss_trigger_bars += 1;
                    }
                    if !signal_pass {
                        grid_miss_signal_bars += 1;
                    }
                    if trigger_ok && signal_pass {
                        let boot_acts = g.bootstrap(&o);
                        sync_grid_levels(&engine, &mut grid_level_map);
                        for a in boot_acts {
                            dispatch_leg(&mut stepper, hedge_stepper.as_mut(), a, bar.t)?;
                        }
                        bootstrap_done = true;
                    }
                }
                EngineBox::Dca(_) => {
                    bootstrap_done = true; // DCA engines gate internally
                }
                EngineBox::Hedge(_) => {
                    bootstrap_done = true; // hedge engines gate internally
                }
            }
        }

        // Funding settlement — every boundary event inside (prev_bar_t, bar.t]
        // charges rate * notional while a position is open (mark ≈ bar close).
        while f_cursor < funding_events.len()
            && funding_events[f_cursor].0 <= prev_bar_t.unwrap_or(0)
        {
            f_cursor += 1; // events already behind the window
        }
        if !stepper.cfg.is_spot && !stepper.position.qty.is_zero() {
            let mut applied = Fraction::zero();
            while f_cursor < funding_events.len() && funding_events[f_cursor].0 <= bar.t {
                let (fts, rate) = funding_events[f_cursor].clone();
                let _ = fts;
                let notional = stepper
                    .position
                    .qty
                    .mul(&c)
                    .map_err(|e| e.to_string())?;
                let dir = Fraction::from_i64(stepper.position.direction as i64);
                let expense = rate
                    .mul(&notional)
                    .map_err(|e| e.to_string())?
                    .mul(&dir)
                    .map_err(|e| e.to_string())?;
                stepper.apply_funding_expense(&expense);
                if let EngineBox::Dca(e) = &mut engine {
                    e.apply_funding(&expense);
                }
                applied = applied.add(&expense).unwrap_or_else(|_| applied.clone());
                f_cursor += 1;
            }
            if !applied.is_zero() {
                total_funding = total_funding.add(&applied).unwrap_or_else(|_| total_funding.clone());
            }
        }
        // Faz 13B-backtest: hedge bacağı AYRI kernel — kendi notional'iyle settlement.
        if let Some(hs) = hedge_stepper.as_mut() {
            if !hs.cfg.is_spot && !hs.position.qty.is_zero() {
                let mut happlied = Fraction::zero();
                while f_cursor_h < funding_events.len() && funding_events[f_cursor_h].0 <= bar.t {
                    let (_, rate) = funding_events[f_cursor_h].clone();
                    let notional = hs.position.qty.mul(&c).map_err(|e| e.to_string())?;
                    let dir = Fraction::from_i64(hs.position.direction as i64);
                    let expense = rate.mul(&notional).map_err(|e| e.to_string())?.mul(&dir).map_err(|e| e.to_string())?;
                    hs.apply_funding_expense(&expense);
                    if let EngineBox::Hedge(h) = &mut engine {
                        if let Some(hleg) = h.hedge.as_mut() {
                            hleg.apply_funding(&expense);
                        }
                    }
                    happlied = happlied.add(&expense).unwrap_or_else(|_| happlied.clone());
                    f_cursor_h += 1;
                }
                if !happlied.is_zero() {
                    total_funding = total_funding.add(&happlied).unwrap_or_else(|_| total_funding.clone());
                }
            }
        }

        for fill in &fills {
            // Grid fills resolve their mesh level from the engine's own
            // id → level registry (kept in sync — offsets emitted by a previous
            // fill are registered at creation time).
            sync_grid_levels(&engine, &mut grid_level_map);
            let acts = match &mut engine {
                EngineBox::Dca(e) => e.on_fill(fill),
                EngineBox::Grid(g) => {
                    let map = grid_level_map.clone();
                    g.on_fill(fill, move |id| map.get(id).copied())
                }
                EngineBox::Hedge(h) => h.on_fill_leg(fill),
            };
            for a in acts {
                dispatch_leg(&mut stepper, hedge_stepper.as_mut(), a, bar.t)?;
            }
        }
        all_fills.extend(fills);

        // Faz 13B-backtest: hedge bacak fill'leri EngineBox::Hedge'e (on_fill_leg
        // prefix routing'iyle) iletilir ve cascade eylemleri hedge stepper'ına dispatch.
        for fill in &hedge_fills {
            let acts = match &mut engine {
                EngineBox::Hedge(h) => h.on_fill_leg(fill),
                _ => Vec::new(),
            };
            for a in acts {
                dispatch_leg(&mut stepper, hedge_stepper.as_mut(), a, bar.t)?;
            }
        }
        all_fills.extend(hedge_fills);

        let ctx = BarContext {
            bar,
            open: o.clone(),
            high: h.clone(),
            low: l.clone(),
            close: c.clone(),
            position_qty: &stepper.position.qty,
            position_avg: stepper.position.avg_price.as_ref(),
            cash: &stepper.cash,
            mark: c.clone(),
            signal_ok,
            regime: Some(regime),
            deal_open_blocked: breaker_tripped,
        };
        // Task 19/20 — Pine kararları: canlı motorla BİREBİR SIRA (parite):
        // pine exit, on_bar'dan ÖNCE işlenir (engine.rs adım 2.85). DCA'da
        // request_close_all motor-içidir → aynı on_bar MANUAL piyasa emrini
        // ÜRETİR (dolum bu barın kapanışında); Grid'te mesh durur. Stepper'a
        // dokunan dispatch/log'lar ctx borcu nedeniyle on_bar SONRASINA planlanır.
        let pos_qty_nonzero = !stepper.position.qty.is_zero();
        let mut pine_exit_acts: Vec<Action> = Vec::new();
        let mut pine_exit_log = false;
        if let Some(v) = pine_v {
            if v.exit {
                match &mut engine {
                    EngineBox::Dca(e) => {
                        if pos_qty_nonzero {
                            e.request_close_all();
                            pine_exit_log = true;
                        }
                    }
                    EngineBox::Grid(g) => {
                        if g.gate_open {
                            pine_exit_acts = g.halt_mesh();
                            sync_grid_levels(&engine, &mut grid_level_map);
                            bootstrap_done = false;
                            pine_exit_log = true;
                        }
                    }
                    EngineBox::Hedge(h) => {
                        if pos_qty_nonzero {
                            h.main.request_close_all();
                            pine_exit_log = true;
                        }
                    }
                }
            }
        }
        let acts = match &mut engine {
            EngineBox::Dca(e) => e.on_bar(&ctx),
            EngineBox::Grid(g) => g.on_bar(&ctx),
            EngineBox::Hedge(h) => h.process_bar(&ctx),
        };
        for a in acts {
            dispatch_leg(&mut stepper, hedge_stepper.as_mut(), a, bar.t)?;
        }
        let halted_mesh = !pine_exit_acts.is_empty();
        for a in pine_exit_acts {
            dispatch_leg(&mut stepper, hedge_stepper.as_mut(), a, bar.t)?;
        }
        if pine_exit_log {
            let detail = if halted_mesh {
                "Pine stratejisi çıkış sinyali üretti — grid mesh durduruldu (resting emirler iptal edildi)"
            } else {
                "Pine stratejisi çıkış sinyali üretti — deal kapanışa alındı (piyasa emri bu barın kapanışında)"
            };
            stepper.log_advisory_pub(bar.t, "PINE", "PINE_EXIT", detail);
        }

        let eq = stepper.equity(&c).map_err(|e| e.to_string())?;
        start_equity.get_or_insert_with(|| eq.clone());
        update_drawdown(&mut dd_state, &eq).map_err(|e| e.to_string())?;
        equity_curve.push(EquityPoint { ts: bar.t, equity: eq.to_string() });
        prev_bar_t = Some(bar.t);
    }

    let (deals, wins) = match &engine {
        EngineBox::Dca(e) => (e.deals_closed, e.wins),
        EngineBox::Grid(g) => (g.deals_closed, g.wins),
        EngineBox::Hedge(h) => {
            let hd = h.hedge.as_ref();
            (
                h.main.deals_closed + hd.map(|l| l.deals_closed).unwrap_or(0),
                h.main.wins + hd.map(|l| l.wins).unwrap_or(0),
            )
        }
    };
    let losses = deals.saturating_sub(wins);
    let _ = losses;

    let start = start_equity.unwrap_or_else(Fraction::one);
    let end = equity_curve
        .last()
        .map(|p| Fraction::from_any_str(&p.equity).unwrap_or_else(|_| start.clone()))
        .unwrap_or_else(|| start.clone());
    let total_return = end.sub(&start).map_err(|e| e.to_string())?.div(&start).map_err(|e| e.to_string())?;
    let win_rate = if deals > 0 {
        Fraction::from_u64(wins as u64).div(&Fraction::from_u64(deals as u64)).map_err(|e| e.to_string())?
    } else {
        Fraction::zero()
    };
    let sharpe = display_sharpe(&equity_curve);
    let (pine_warnings, pine_plots) = match &pine_res {
        Some(r) => (r.warnings.clone(), r.plots.clone()),
        None => (Vec::new(), BTreeMap::new()),
    };

    // Task 22 — no-deal nedenleri: motorun o koşuda GERÇEKTEN verdiği engel
    // kararlarının sayaç dökümü. bars>0 olan her kod eklenir (deal açılsa
    // bile kısmi bloklar bilgilendiricidir); UI boş sonuçta gösterir.
    let mut no_deal_reasons: Vec<NoDealReason> = Vec::new();
    let mut push_reason = |code: &str, bars: u64, detail: String| {
        if bars > 0 {
            no_deal_reasons.push(NoDealReason { code: code.to_string(), bars, detail });
        }
    };
    match &engine {
        EngineBox::Dca(e) => {
            let trigger_desc = e.cfg.entry_trigger.describe();
            push_reason(
                "ENTRY_TRIGGER_NEVER_MET",
                e.blocked_trigger_bars,
                format!("{n} barda başlangıç koşulu ({d}) sağlanmadı — deal açılışı denenemedi", n = e.blocked_trigger_bars, d = trigger_desc),
            );
            push_reason(
                "SIGNAL_BLOCKED",
                e.blocked_signal_bars,
                format!("{n} barda başlangıç sinyali false döndü (ağaç/Pine birleşik hükmü) — fail-closed bekleme", n = e.blocked_signal_bars),
            );
            push_reason(
                "REGIME_BLOCKED",
                e.blocked_regime_bars,
                format!("{n} barda rejim filtresi (ZORUNLU mod) yeni deal açılışını durdurdu", n = e.blocked_regime_bars),
            );
            push_reason(
                "BREAKER_BLOCKED",
                e.blocked_breaker_bars,
                format!("{n} barda günlük DD devre kesici yeni deal açılışını blokladı", n = e.blocked_breaker_bars),
            );
            push_reason(
                "COOLDOWN_ACTIVE",
                e.blocked_cooldown_bars,
                format!("{n} barda önceki deal sonrası cooldown beklemesi", n = e.blocked_cooldown_bars),
            );
            push_reason(
                "SIZE_BELOW_MIN",
                e.blocked_size_bars,
                format!(
                    "{n} barda base notional enstrüman min_qty/step altına yuvarlandı (qty→0) — deal açılamadı; base_order_size yetersiz",
                    n = e.blocked_size_bars
                ),
            );
            push_reason(
                "CONFIG_INVALID",
                e.invalid_config_bars,
                format!("{n} barda config sayısal alanı veya SL aralığı geçersiz — deal reddedildi", n = e.invalid_config_bars),
            );
        }
        EngineBox::Hedge(_) => {
            // Faz 13B: hedge backtest'leri için no-deal sayaçları henüz ayrı
            // alınmıyor (bacaklar kendi phase'lerinde ilerler) — bilinçli boş.
        }
        EngineBox::Grid(g) => {
            let trigger_desc = g.cfg.entry_trigger.describe();
            push_reason(
                "GRID_BOOTSTRAP_TRIGGER_MISSED",
                grid_miss_trigger_bars,
                format!("{n} barda grid başlangıç koşulu ({d}) sağlanmadı — mesh kurulamadı", n = grid_miss_trigger_bars, d = trigger_desc),
            );
            push_reason(
                "GRID_BOOTSTRAP_SIGNAL_MISSED",
                grid_miss_signal_bars,
                format!("{n} barda başlangıç sinyali false döndü (ağaç/Pine birleşik hükmü) — mesh fail-closed beklemede", n = grid_miss_signal_bars),
            );
            if !bootstrap_done {
                push_reason(
                    "GRID_BOOTSTRAP_NEVER_MET",
                    bars.len() as u64,
                    format!("koşu boyunca hiçbir barda mesh kurulamadı ({n} bar)", n = bars.len()),
                );
            }
        }
    }

    let metrics = BacktestMetrics {
        total_return_pct: total_return.mul(&Fraction::from_u64(100)).map_err(|e| e.to_string())?.to_half_even_display(4),
        max_drawdown_pct: dd_state.max_drawdown.mul(&Fraction::from_u64(100)).map_err(|e| e.to_string())?.to_half_even_display(4),
        win_rate_pct: win_rate.mul(&Fraction::from_u64(100)).map_err(|e| e.to_string())?.to_half_even_display(2),
        profit_factor: "n/a".into(),
        sharpe_display: format!("{:.3}", sharpe),
        total_fees: stepper.cumulative_fees.to_half_even_display(6),
        total_funding: total_funding.to_half_even_display(6),
        bars: bars.len(),
        deals,
        fill_realism: FillRealismBreakdown::from_fills(&all_fills),
    };

    Ok(BacktestResult {
        equity_curve,
        fills: all_fills,
        gate_logs: stepper.gate_logs,
        no_deal_reasons,
        metrics,
        start_equity: start.to_string(),
        end_equity: end.to_string(),
        pine_warnings,
        pine_notes: pine_res
            .as_ref()
            .map(|_| pine_notes.clone())
            .unwrap_or_default(),
        pine_plots,
    })
}

enum EngineBox {
    Dca(DcaEngine),
    Grid(GridEngine),
    Hedge(HedgeEngine),
}

/// Keep the backtest id → level map in sync with the grid engine's own
/// registry (bootstrap + every offset re-arm registers at creation time).
fn sync_grid_levels(engine: &EngineBox, map: &mut std::collections::HashMap<String, usize>) {
    if let EngineBox::Grid(g) = engine {
        for (k, v) in g.order_levels() {
            map.entry(k.clone()).or_insert(*v);
        }
    }
}

/// Faz 13B-backtest: eylemi order-id prefix'ine göre DOĞRU bacak stepper'ına
/// yönlendirir (bt-hedge → hedge kernel; diğer → ana kernel).
fn dispatch_leg(
    main: &mut SimulationStepper,
    hedge: Option<&mut SimulationStepper>,
    action: Action,
    ts: i64,
) -> Result<(), String> {
    let id = match &action {
        Action::PlaceLimit { id, .. } | Action::PlaceMarket { id, .. } | Action::CancelResting { id } => id.clone(),
        Action::Log { .. } => String::new(),
    };
    if id.starts_with("bt-hedge") {
        match hedge {
            Some(hs) => dispatch(hs, action, ts),
            None => Err("hedge stepper yok — hedge emri yönlendirilemedi".into()),
        }
    } else {
        dispatch(main, action, ts)
    }
}

fn dispatch(stepper: &mut SimulationStepper, action: Action, ts: i64) -> Result<(), String> {
    match action {
        Action::PlaceLimit { id, side, price, qty, role } => {
            // GRID_CLOSE ve Faz 19 TP (resting reduce-only limit) kapanış
            // emirleridir (Reduce) — GRID step kârını, TP deal kârını kitalar;
            // diğerleri envanter açar.
            let kind = if matches!(role, "GRID_CLOSE" | "TP") { OrderKind::Reduce } else { OrderKind::Add };
            let placement_ref = stepper.last_close.clone().unwrap_or_else(|| price.to_string());
            stepper.submit_intent(
                OrderIntent {
                    id: id.clone(),
                    side,
                    kind,
                    price: Some(price),
                    qty,
                    decision_ts: ts,
                    role: role.to_string(),
                    placement_ref_price: Some(Fraction::from_any_str(&placement_ref).unwrap_or_else(|_| Fraction::one())),
                    quote_notional: None,
                },
                ts,
            )?;
        }
        Action::PlaceMarket { id, side, qty, role, quote_notional } => {
            let kind = if matches!(role, "TP" | "SL" | "MANUAL" | "TRAIL") {
                OrderKind::Reduce
            } else {
                OrderKind::Add
            };
            stepper.submit_intent(
                OrderIntent {
                    id,
                    side,
                    kind,
                    price: None,
                    qty,
                    decision_ts: ts,
                    role: role.to_string(),
                    placement_ref_price: None,
                    quote_notional,
                },
                ts,
            )?;
        }
        Action::CancelResting { id } => {
            if id.ends_with('*') {
                let prefix = id.trim_end_matches('*');
                stepper.pending.retain(|o| !o.id.starts_with(prefix));
            } else {
                stepper.pending.retain(|o| o.id != id);
            }
        }
        Action::Log { .. } => {}
    }
    Ok(())
}

/// Reporting-boundary Sharpe on per-bar equity returns (f64, display only).
fn display_sharpe(curve: &[EquityPoint]) -> f64 {
    if curve.len() < 3 {
        return 0.0;
    }
    let vals: Vec<f64> = curve
        .iter()
        .filter_map(|p| Fraction::from_any_str(&p.equity).ok())
        .map(|f| f.to_f64_display())
        .collect();
    let mut rets = Vec::with_capacity(vals.len());
    for w in vals.windows(2) {
        if w[0] != 0.0 {
            rets.push((w[1] - w[0]) / w[0]);
        }
    }
    if rets.len() < 2 {
        return 0.0;
    }
    let mean = rets.iter().sum::<f64>() / rets.len() as f64;
    let var = rets.iter().map(|r| (r - mean) * (r - mean)).sum::<f64>() / (rets.len() - 1) as f64;
    let sd = var.sqrt();
    if sd == 0.0 {
        0.0
    } else {
        mean / sd * 100.0 // scaled for readability; display-only
    }
}

#[cfg(test)]
mod pine_backtest_tests {
    use super::*;
    use dca_math::Instrument;
    use std::collections::BTreeMap;

    fn bar(t: i64, c: &str) -> Bar {
        Bar { t, o: c.into(), h: c.into(), l: c.into(), c: c.into(), v: "100000".into() }
    }

    /// 20 bar sabit fiyat + iki uç bar (giriş için düşüş, çıkış için yükseliş)
    fn scenario_bars() -> Vec<Bar> {
        let mut bars = Vec::new();
        let mut t = 1_700_000_000_000i64;
        for i in 0..20 {
            let px = match i {
                5 => "90",  // düşüş
                10 => "110", // yükseliş
                _ => "100",
            };
            bars.push(bar(t, px));
            t += 3_600_000;
        }
        bars
    }

    fn cfg_with_pine(code: &str) -> BotConfig {
        let mut cfg = BotConfig::default();
        cfg.base_order_size = "100".into();
        cfg.safety_order_size = "0".into();
        cfg.target_profit_pct = "1000000".into(); // TP kapalı — pine çıkışı
        cfg.pine_script = Some(code.into());
        cfg.pine_inputs = BTreeMap::new();
        cfg
    }

    /// (1) Pine koşulu hiçbir barda sağlanmazsa SIFIR deal (fail-closed kapı).
    #[test]
    fn pine_never_true_blocks_all_entries() {
        let src = "strategy(\"N\")\nif close < 1\n    strategy.entry(\"L\", strategy.long)\n";
        let res = run_backtest(
            &scenario_bars(),
            &Instrument::default(),
            &cfg_with_pine(src),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        assert_eq!(res.metrics.deals, 0, "hiç tetiklenmeyen pine sıfır deal açmalı");
        assert!(res.fills.is_empty());
    }

    /// (2) Pine koşulu sağlanır → deal açılır; dolum kırılımı tüketici (taker)
    /// girişini raporlar; maker+taker komisyon toplamı metrics.total_fees ile tutarlı.
    #[test]
    fn pine_entry_opens_deal_with_realism_breakdown() {
        let src = "strategy(\"A\")\nif close <= 100\n    strategy.entry(\"L\", strategy.long)\n";
        let res = run_backtest(
            &scenario_bars(),
            &Instrument::default(),
            &cfg_with_pine(src),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        assert!(res.fills.iter().any(|f| f.role == "BASE"), "pine koşulu taban emri açmalı (deal TP'siz açık kalır)",);
        assert_eq!(res.metrics.deals, 0, "çıkış sinyali olmadığından deal açık kalır (kapanmamış sayılmaz)");
        let fr = &res.metrics.fill_realism;
        let total = fr.marketable_taker + fr.resting_touch + fr.open_gap_better;
        assert_eq!(total, res.fills.len(), "kırılım toplamı fill sayısıyla eşit olmalı");
        assert!(fr.marketable_taker >= 1, "market girişleri taker etiketli olmalı");
    }

    /// (3) Pine çıkışı: close_all tetiklendiğinde deal zarif kapanır (win/loss
    /// muhasebesi tam) ve PINE_EXIT advisory'si gate loglarına düşer.
    #[test]
    fn pine_exit_closes_deal_gracefully() {
        // bar 5'te (close=90) giriş; bar 10+ (close=110) çıkış
        let src = "strategy(\"E\")\nif close <= 90\n    strategy.entry(\"L\", strategy.long)\nif close >= 110\n    strategy.close_all()\n";
        let res = run_backtest(
            &scenario_bars(),
            &Instrument::default(),
            &cfg_with_pine(src),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        assert_eq!(res.metrics.deals, 1, "tek giriş + tek çıkış = 1 kapanan deal, got {}", res.metrics.deals);
        assert_eq!(res.metrics.deals, res.metrics.deals, "deals sayımı");
        assert!(
            res.gate_logs.iter().any(|g| g.verdict == "ADVISORY:PINE_EXIT"),
            "PINE_EXIT advisory kaydı düşmeli"
        );
    }

    /// (4) Zero look-ahead regresyon: pine kararı, gelecekteki barlar eklenince
    /// değişmemeli (run_backtest çıktısı prefix-bağımsız).
    #[test]
    fn pine_backtest_prefix_stability() {
        let src = "strategy(\"Z\")\nfast = ta.ema(close, 3)\nslow = ta.ema(close, 6)\nif ta.crossover(fast, slow)\n    strategy.entry(\"L\", strategy.long)\n";
        let all = scenario_bars();
        let full = run_backtest(
            &all,
            &Instrument::default(),
            &cfg_with_pine(src),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        for cut in [8usize, 12, 16] {
            let part = run_backtest(
                &all[..cut],
                &Instrument::default(),
                &cfg_with_pine(src),
                Fraction::from_u64(10_000),
                dca_math::sessions::SessionType::Crypto24_7,
                &[],
            )
            .unwrap();
            // fill sayısı prefix'te asla full'ü geçemez; kesilen noktaya kadarki
            // fill'ler birebir aynı olmalı (zero look-ahead).
            let part_fills = part.fills.iter().filter(|f| f.ts <= all[cut - 1].t).count();
            let full_fills = full.fills.iter().filter(|f| f.ts <= all[cut - 1].t).count();
            assert_eq!(part_fills, full_fills, "kesim {cut}: geçmiş fill'ler değişmemeli");
        }
    }

    /// (5) Task 20 — pine exit SIRA PARİTESİ: çıkış sinyali on_bar'dan ÖNCE
    /// işlenir (canlı adım 2.85 ile birebir) → MANUAL piyasa emri ÇIKIŞ BARINDA
    /// submit edilir ve simülatörün zero-look-ahead sözleşmesi gereği bir sonraki
    /// bar açılışında dolar (eski akışta submit bir bara sarkıyordu → dolum iki
    /// bar sonradı). Canlıdaki poll-fiyatı dolumu bar-granüler simülasyonda
    /// yakalanamaz — bu fark fill_realism/realismCompare'ın dokümante kapsamıdır.
    #[test]
    fn pine_exit_submitted_on_signal_bar() {
        // bar 5'te giriş sinyali (dolum bar 6 açılışı), bar 10'da çıkış sinyali
        // → MANUAL submit bar 10, dolum bar 11 açılışı (eski akışta bar 12).
        let src = "strategy(\"E\")\nif close <= 90\n    strategy.entry(\"L\", strategy.long)\nif close >= 110\n    strategy.close_all()\n";
        let res = run_backtest(
            &scenario_bars(),
            &Instrument::default(),
            &cfg_with_pine(src),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        let exit_t = scenario_bars()[10].t;
        let manual = res.fills.iter().find(|f| f.role == "MANUAL").expect("MANUAL çıkış fill'i olmalı");
        assert_eq!(
            manual.ts,
            exit_t + 3_600_000,
            "pine exit MANUAL dolumu sinyali izleyen bar açılışında olmalı (submit çıkış barında)"
        );
    }
}

#[cfg(test)]
mod grid_backtest_tests {
    use super::*;
    use dca_math::Instrument;

    pub(super) fn grid_cfg() -> BotConfig {
        let mut cfg = BotConfig::default();
        cfg.kind = StrategyKind::SpotGrid;
        cfg.grid_lower = Some("90".into());
        cfg.grid_upper = Some("110".into());
        cfg.grid_levels = Some(4);
        cfg.grid_mode = Some("arithmetic".into());
        cfg.base_order_size = "100".into();
        cfg
    }

    fn bar(t: i64, o: &str, h: &str, l: &str, c: &str) -> Bar {
        Bar { t, o: o.into(), h: h.into(), l: l.into(), c: c.into(), v: "100000".into() }
    }

    /// CRITICAL regression (Task 15-a item 8): grid must not be a one-shot mesh.
    /// A buy opener fill must place a GRID_CLOSE offset; the closer fill must
    /// book grid_profit / deals_closed and re-arm the opener.
    #[test]
    fn backtest_grid_rearms_offsets_and_books_profit() {
        // mesh (arithmetic, tick 0.1): [90, 96.6, 103.3, 110]; bootstrap @ open 100
        let bars = vec![
            bar(1_700_000_000_000, "100", "100.5", "99.5", "100"), // bootstrap
            bar(1_700_003_600_000, "96", "97", "89", "89"),        // buys @96.6 (gap-better @96) and @90 fill
            bar(1_700_007_200_000, "104", "111", "104", "111"),    // both GRID_CLOSE sells fill (@104 gap-better)
            bar(1_700_010_800_000, "111", "112", "110.5", "111"),  // re-armed openers eligible (no fill needed)
        ];
        let res = run_backtest(
            &bars,
            &Instrument::default(),
            &grid_cfg(),
            Fraction::from_u64(100_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        let closer_fills = res.fills.iter().filter(|f| f.role == "GRID_CLOSE").count();
        assert!(closer_fills >= 2, "offset closers must be placed and filled, got {closer_fills}");
        assert!(res.metrics.deals >= 2, "grid pair completions must be counted, got {}", res.metrics.deals);
        // NOTE (documented pre-existing limitation, out of scope here): the
        // kernel's spot position accounting treats Add fills side-agnostically,
        // so bootstrap SELL openers inflate the mirrored long position and the
        // marked end-equity understates grid profitability. Grid pair PnL is
        // tracked truthfully on the engine (grid_profit/deals_closed) — the
        // side-aware spot ledger is a separate follow-up.
    }

    /// Task 20 — pine exit GRID mesh'ini DURDURUR ve yeni pine giriş sinyali
    /// mesh'i YENİDEN kurar: (1) PINE_EXIT advisory düşer, (2) eski resting
    /// emirler iptal edildiği için çift dolum oluşmaz (re-bootstrap ile toplam
    /// alış fill'i tam 2 — eski emirler hayatta olsaydı 4 olurdu), (3) yeniden
    /// kurulan mesh çift tamamlayıp deals_closed üretir.
    #[test]
    fn pine_exit_halts_grid_mesh_and_reentry_rebootstraps() {
        use std::collections::BTreeMap;
        let mut cfg = grid_cfg();
        // pine: close<=90'da giriş, close>=110'da çıkış
        cfg.pine_script = Some(
            "strategy(\"G\")\nif close <= 90\n    strategy.entry(\"L\", strategy.long)\nif close >= 110\n    strategy.close_all()\n".into(),
        );
        cfg.pine_inputs = BTreeMap::new();
        let mut t = 1_700_000_000_000i64;
        let mut bars = Vec::new();
        // 0-4: 100 flat (bootstrap bekler — pine giriş yok)
        for _ in 0..5 {
            bars.push(bar(t, "100", "100.5", "99.5", "100"));
            t += 3_600_000;
        }
        // 5: close 90 → pine giriş → grid bootstrap @open 100 (buys 96.6, 90)
        bars.push(bar(t, "100", "100.5", "89.5", "90"));
        t += 3_600_000;
        // 6-9: 100 flat (resting emirler dolmaz)
        for _ in 0..4 {
            bars.push(bar(t, "100", "100.5", "99.5", "100"));
            t += 3_600_000;
        }
        // 10: close 110 → pine exit → mesh DURUR (resting alışlar iptal)
        bars.push(bar(t, "100", "111", "100", "110"));
        t += 3_600_000;
        // 11-14: 100 flat (dolum OLAMAZ — emirler iptal edildi)
        for _ in 0..4 {
            bars.push(bar(t, "100", "100.5", "99.5", "100"));
            t += 3_600_000;
        }
        // 15: close 89 → pine yeniden giriş → bootstrap @open 96 (buys yeniden)
        bars.push(bar(t, "96", "96.5", "88.5", "89"));
        t += 3_600_000;
        // 16: alışlar dolar (96→95 gap-better, 90)
        bars.push(bar(t, "95", "96", "85", "85"));
        t += 3_600_000;
        // 17: closer satışlar dolar
        bars.push(bar(t, "104", "111", "104", "111"));
        let res = run_backtest(
            &bars,
            &Instrument::default(),
            &cfg,
            Fraction::from_u64(100_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        assert!(
            res.gate_logs.iter().any(|g| g.verdict == "ADVISORY:PINE_EXIT" && g.detail.contains("mesh durduruldu")),
            "grid PINE_EXIT advisory'si düşmeli: {:?}",
            res.gate_logs.iter().map(|g| (&g.verdict, &g.detail)).collect::<Vec<_>>()
        );
        // Haltsız davranışta bar 15'teki re-bootstrap ESKİ emirlerle çakışır:
        // aynı fiyat seviyelerinde çift emir → 4 alış fill'i olurdu.
        let buy_fills = res.fills.iter().filter(|f| matches!(f.side, dca_math::Side::Buy)).count();
        assert_eq!(buy_fills, 2, "mesh halt olduktan sonra yalnız YENİ mesh'in 2 alışı dolmalı, got {buy_fills}");
        assert!(res.metrics.deals >= 2, "re-bootstrap sonrası çift tamamlanmalı, got {}", res.metrics.deals);
    }
}

#[cfg(test)]
mod signal_backtest_tests {
    use super::*;
    use dca_math::Instrument;

    pub(super) fn rising_bars(n: usize, start: f64) -> Vec<Bar> {
        // deterministic uptrend: RSI stays pinned at 100 (no down closes)
        let mut bars = Vec::new();
        let mut px = start;
        let mut t = 1_700_000_000_000i64;
        for _ in 0..n {
            let c = format!("{px}");
            bars.push(Bar { t, o: c.clone(), h: c.clone(), l: c.clone(), c, v: "10".into() });
            px *= 1.01;
            px = (px * 1000.0).round() / 1000.0;
            t += 3_600_000;
        }
        bars
    }

    pub(super) fn sig_cfg(root: dca_signals::SignalNode) -> dca_signals::SignalConfig {
        dca_signals::SignalConfig { root }
    }

    pub(super) fn rsi_lt_30() -> dca_signals::SignalNode {
        dca_signals::SignalNode::Condition(dca_signals::SignalCondition {
            left: dca_signals::IndicatorSpec {
                key: dca_signals::IndicatorKey::Rsi,
                timeframe: dca_signals::Timeframe::H1,
                period: Some(14),
                fast: None,
                slow: None,
                signal: None,
                std_dev: None,
                d_period: None,
                output: None,
            },
            op: dca_signals::CompareOp::Lt,
            right: dca_signals::SignalValue::Number(30.0),
        })
    }

    fn close_gt_zero() -> dca_signals::SignalNode {
        dca_signals::SignalNode::Condition(dca_signals::SignalCondition {
            left: dca_signals::IndicatorSpec {
                key: dca_signals::IndicatorKey::Price,
                timeframe: dca_signals::Timeframe::H1,
                period: None,
                fast: None,
                slow: None,
                signal: None,
                std_dev: None,
                d_period: None,
                output: Some("close".into()),
            },
            op: dca_signals::CompareOp::Gt,
            right: dca_signals::SignalValue::Number(0.0),
        })
    }

    pub(super) fn cfg_with(sig: Option<dca_signals::SignalConfig>) -> BotConfig {
        let mut cfg = BotConfig::default();
        if let Some(s) = sig {
            cfg.entry_signal = Some(s);
        }
        cfg
    }


    #[test]
    fn backtest_signal_passthrough_when_always_true() {
        let bars = rising_bars(60, 100.0);
        let res = run_backtest(
            &bars,
            &Instrument::default(),
            &cfg_with(Some(sig_cfg(close_gt_zero()))),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        assert!(res.metrics.deals >= 1, "always-true signal must allow the base order");
        assert!(!res.fills.is_empty());
    }

    #[test]
    fn backtest_lower_timeframe_signal_rejected() {
        let bars = rising_bars(30, 100.0);
        let mut root = serde_json::json!(rsi_lt_30());
        // force timeframe to M1 (below the 1h bot bars)
        root["left"]["timeframe"] = serde_json::json!("M1");
        let node: dca_signals::SignalNode = serde_json::from_value(root).unwrap();
        let err = run_backtest(
            &bars,
            &Instrument::default(),
            &cfg_with(Some(sig_cfg(node))),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap_err();
        assert!(err.contains("zero look-ahead"), "lower TF must be rejected, got: {err}");
    }

    #[test]
    fn resample_aggregates_and_buckets_exactly() {
        let base = vec![
            BarData { t: 0, o: 1.0, h: 2.0, l: 0.5, c: 1.5, v: 3.0 },
            BarData { t: 60_000, o: 1.5, h: 2.5, l: 1.0, c: 2.0, v: 4.0 },
            BarData { t: 120_000, o: 2.0, h: 3.0, l: 1.5, c: 2.5, v: 5.0 },
        ];
        let up = resample(&base, 120_000);
        assert_eq!(up.len(), 2);
        assert_eq!(up[0].t, 0);
        assert_eq!(up[0].o, 1.0);
        assert_eq!(up[0].h, 2.5);
        assert_eq!(up[0].l, 0.5);
        assert_eq!(up[0].c, 2.0);
        assert_eq!(up[0].v, 7.0);
        assert_eq!(up[1].t, 120_000);
        assert_eq!(up[1].c, 2.5);
    }

    #[test]
    fn higher_timeframe_resample_gate_works() {
        // bot bars 15m, tree on H1 (higher) — allowed and must not block always-true
        let bars = rising_bars(40, 100.0);
        // rebase timestamps to 15m grid
        let bars: Vec<Bar> = bars
            .into_iter()
            .enumerate()
            .map(|(i, mut b)| {
                b.t = 1_700_000_000_000 + (i as i64) * 900_000;
                b
            })
            .collect();
        let res = run_backtest(
            &bars,
            &Instrument::default(),
            &cfg_with(Some(sig_cfg(close_gt_zero()))),
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .unwrap();
        assert!(res.metrics.deals >= 1);
    }
}

/* ------------------------------------------------------------------ */
/* Task 22 — no-deal sözleşmesi: boş sonuçlar makine-okunur neden taşır.
   Tablo-odaklı tek test; her satır bir davranış + sınır durumudur.        */
/* ------------------------------------------------------------------ */
#[cfg(test)]
mod no_deal_reason_tests {
    use super::grid_backtest_tests::grid_cfg;
    use super::signal_backtest_tests::{cfg_with, rising_bars, rsi_lt_30, sig_cfg};
    use super::*;
    use dca_math::Instrument;

    fn codes_of(res: &BacktestResult) -> std::collections::HashMap<&str, u64> {
        res.no_deal_reasons.iter().map(|r| (r.code.as_str(), r.bars)).collect()
    }

    fn run(cfg: &BotConfig, bars: &[Bar]) -> BacktestResult {
        run_backtest(
            bars,
            &Instrument::default(),
            cfg,
            Fraction::from_u64(10_000),
            dca_math::sessions::SessionType::Crypto24_7,
            &[],
        )
        .expect("backtest")
    }

    /// Boş sonuçların tam sözleşmesi: (ad, config, barlar, beklenen kod→bar).
    /// Kod sayıları motor sayaçlarından gelir — eksik/fazla neden testi kızartır.
    #[test]
    fn empty_backtests_explain_themselves() {
        let mut trigger_never = cfg_with(None);
        trigger_never.entry_trigger = crate::EntryTrigger::PriceBelow("1".into());
        let mut grid_never = grid_cfg();
        grid_never.entry_trigger = crate::EntryTrigger::PriceBelow("1".into());
        let mut bad_base = cfg_with(None);
        bad_base.base_order_size = "abc".into();
        let cases: Vec<(&str, BotConfig, Vec<Bar>, Vec<(&str, u64)>)> = vec![
            ("imkansız trigger", trigger_never, rising_bars(60, 100.0), vec![("ENTRY_TRIGGER_NEVER_MET", 60)]),
            (
                "imkansız sinyal",
                cfg_with(Some(sig_cfg(rsi_lt_30()))),
                rising_bars(60, 100.0),
                vec![("SIGNAL_BLOCKED", 60)],
            ),
            (
                "grid bootstrap yok",
                grid_never,
                rising_bars(60, 100.0),
                vec![("GRID_BOOTSTRAP_TRIGGER_MISSED", 60), ("GRID_BOOTSTRAP_NEVER_MET", 60)],
            ),
            ("geçersiz config", bad_base, rising_bars(60, 100.0), vec![("CONFIG_INVALID", 60)]),
        ];
        for (name, cfg, bars, expected) in &cases {
            let res = run(cfg, bars);
            assert_eq!(res.metrics.deals, 0, "{name}: deal açılmamalı");
            assert!(res.fills.is_empty(), "{name}: fill olmamalı");
            let got = codes_of(&res);
            assert_eq!(got.len(), expected.len(), "{name}: beklenmeyen neden var: {got:?}");
            for (code, n) in expected {
                assert_eq!(got.get(code), Some(n), "{name}: {code} sayacı");
            }
        }
    }

    /// Sağlıklı koşu: deal açılır, neden listesi boş kalır (kutu gizlenir).
    #[test]
    fn healthy_run_has_no_reasons() {
        let res = run(&cfg_with(None), &rising_bars(60, 100.0));
        assert!(res.metrics.deals >= 1, "INSTANT koşu deal açmalı");
        assert!(res.no_deal_reasons.is_empty(), "sağlıklı koşuda neden yok: {:?}", res.no_deal_reasons);
    }

    /// Cooldown: kapanış sonrası bekleme sayılır, yeni deal yine açılır.
    #[test]
    fn cooldown_blocks_are_counted() {
        let mut cfg = cfg_with(None);
        cfg.cooldown_bars = 5;
        let res = run(&cfg, &rising_bars(12, 100.0));
        assert!(res.metrics.deals >= 1, "cooldown sonrası yeni deal açılmalı");
        let got = codes_of(&res);
        assert!(got.get("COOLDOWN_ACTIVE").copied().unwrap_or(0) > 0, "cooldown sayılmalı: {got:?}");
    }
}
