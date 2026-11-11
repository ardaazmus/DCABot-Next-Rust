//! Regression suite reproducing and verifying all 10 critical findings
//! from the comprehensive independent audit (2026-11-11).

use dca_kernel::{Bar, SimulationStepper, StepperConfig, OrderIntent, OrderKind, FillRecord, FeeRole};
use dca_math::fraction::Fraction;
use dca_math::instrument::Instrument;
use dca_math::m02::{Direction, SizingMode};
use dca_math::m03_m07::calculate_net_take_profit;
use dca_math::m15_m20::{geometric_mesh, m20_conflict_scan};
use dca_math::Side;
use dca_risk::FillRealism;
use dca_strategy::dca::{DcaEngine, DcaPhase};
use dca_strategy::hedge::HedgeEngine;
use dca_strategy::{Action, BarContext, BotConfig, StrategyKind};
use dca_signals::{BarData, CompareOp, IndicatorKey, IndicatorSpec, SignalCondition, SignalNode, SignalValue, Timeframe, SignalEngine};

fn f(s: &str) -> Fraction {
    Fraction::from_decimal_str(s).unwrap()
}

fn make_bar(t: i64, o: &str, h: &str, l: &str, c: &str) -> Bar {
    Bar {
        t,
        o: o.into(),
        h: h.into(),
        l: l.into(),
        c: c.into(),
        v: "100".into(),
    }
}

fn make_bar_data(t: i64, c: f64) -> BarData {
    BarData {
        t,
        o: c,
        h: c * 1.01,
        l: c * 0.99,
        c,
        v: 100.0,
    }
}

fn make_fill(order_id: &str, role: &str, side: Side, kind: OrderKind, price: &str, qty: &str, fee: &str, ts: i64) -> FillRecord {
    FillRecord {
        fill_key: format!("{order_id}-{ts}"),
        order_id: order_id.to_string(),
        role: role.to_string(),
        side,
        kind,
        price: f(price),
        qty: f(qty),
        fee: f(fee),
        fee_role: FeeRole::Maker,
        realism: FillRealism::RestingTouch,
        ts,
    }
}

fn dummy_context<'a>(bar: &'a Bar, mark: Fraction, pos_qty: &'a Fraction) -> BarContext<'a> {
    BarContext {
        bar,
        open: f(&bar.o),
        high: f(&bar.h),
        low: f(&bar.l),
        close: f(&bar.c),
        position_qty: pos_qty,
        position_avg: None,
        cash: pos_qty, // dummy reference
        mark,
        signal_ok: None,
        regime: None,
        deal_open_blocked: false,
    }
}

#[test]
fn lookahead_four_hour_signal_changes_when_future_bars_are_appended() {
    // Audit 1.4: Resampling and signal evaluation must be strictly causal (zero look-ahead).
    // An evaluation at time t must depend only on completed historical bars at or before t.
    let spec = IndicatorSpec {
        key: IndicatorKey::Rsi,
        timeframe: Timeframe::H4,
        period: Some(2),
        fast: None,
        slow: None,
        signal: None,
        std_dev: None,
        d_period: None,
        output: None,
    };
    let node = SignalNode::Condition(SignalCondition {
        left: spec.clone(),
        op: CompareOp::Gt,
        right: SignalValue::Number(30.0),
    });

    let bars_at_t1 = vec![
        make_bar_data(1000, 100.0),
        make_bar_data(2000, 105.0),
        make_bar_data(3000, 110.0),
    ];
    let se1 = SignalEngine::new(&[(Timeframe::H4, bars_at_t1)], &[spec.clone()]);
    let sig_t3 = se1.evaluate_at_time(&node, 3000);

    // Append future bars with a massive dump at t=4000 and t=5000
    let bars_with_future = vec![
        make_bar_data(1000, 100.0),
        make_bar_data(2000, 105.0),
        make_bar_data(3000, 110.0),
        make_bar_data(4000, 50.0),
        make_bar_data(5000, 30.0),
    ];
    let se2 = SignalEngine::new(&[(Timeframe::H4, bars_with_future)], &[spec.clone()]);

    // Evaluation at historical t=3000 MUST be identical and unaffected by future dump
    let sig_t3_future_present = se2.evaluate_at_time(&node, 3000);
    assert_eq!(sig_t3, sig_t3_future_present, "evaluation at t=3000 must not be tainted by future bars");
}

#[test]
fn base_fee_is_erased_and_win_classification_can_flip() {
    // Audit 1.7: Deal net realized PnL must deduct entry fees as well as exit fees.
    let mut e = DcaEngine::new("fee-flip", BotConfig::default(), Instrument::default()).unwrap();
    let bar = make_bar(1000, "100", "100", "100", "100");
    let zero = Fraction::zero();
    let ctx = dummy_context(&bar, f("100"), &zero);
    let _ = e.on_bar(&ctx);

    // Fill BASE with $2.00 entry fee
    let base_fill = make_fill(&format!("{}-BASE-1", e.bot_id), "BASE", Side::Buy, OrderKind::Add, "100", "1", "2.00", 1000);
    e.on_fill(&base_fill);

    // Close deal with $0.50 gross profit (100.50 sale) but $2.00 entry fee + $0.10 exit fee
    // Gross PnL: +0.50. Total Fees: 2.10. Net PnL: -1.60. Must be classified as a LOSS!
    let close_fill = make_fill(&format!("{}-TP-1", e.bot_id), "TP", Side::Sell, OrderKind::Reduce, "100.50", "1", "0.10", 2000);
    e.on_fill(&close_fill);

    assert_eq!(e.phase, DcaPhase::Cooldown);
    assert_eq!(e.losses, 1, "deal net of fees was negative (-1.60), win classification must be loss");
    assert_eq!(e.wins, 0, "wins must remain 0 when net PnL is negative");
}

#[test]
fn net_tp_omits_entry_fee() {
    // Audit 1.7: calculate_net_take_profit must include entry_fees in numerator
    let q = f("1");
    let avg = f("100");
    let target = f("1"); // 1 USD target profit
    let f_out = f("0.001");
    let entry_fees = f("0.10"); // $0.10 paid on entry
    let tick = f("0.01");

    let tp_price = calculate_net_take_profit(
        1,
        &q,
        &avg,
        &Fraction::zero(),
        &entry_fees,
        &f_out,
        &target,
        &tick,
    ).unwrap();

    // Verify gross revenue - cost - entry_fee - exit_fee >= target
    let revenue = q.mul(&tp_price).unwrap();
    let cost = q.mul(&avg).unwrap();
    let gross_profit = revenue.sub(&cost).unwrap();
    let exit_fee = revenue.mul(&f_out).unwrap();
    let net_profit = gross_profit.sub(&entry_fees).unwrap().sub(&exit_fee).unwrap();

    assert!(net_profit >= target, "net profit {net_profit} must meet or exceed target {target}");
}

#[test]
fn tp_close_does_not_cancel_safety_and_old_fill_reopens_in_cooldown() {
    // Audit 1.1: TP, trailing, or manual close must cancel resting SAFETY orders via wildcard
    let mut e = DcaEngine::new("safety-cancel", BotConfig::default(), Instrument::default()).unwrap();
    let bar = make_bar(1000, "100", "100", "100", "100");
    let zero = Fraction::zero();
    let ctx = dummy_context(&bar, f("100"), &zero);
    let _ = e.on_bar(&ctx);

    let base_fill = make_fill(&format!("{}-BASE-1", e.bot_id), "BASE", Side::Buy, OrderKind::Add, "100", "1", "0.01", 1000);
    e.on_fill(&base_fill);
    assert_eq!(e.phase, DcaPhase::Open);

    // Request manual close
    e.manual_close_requested = true;
    let acts = e.on_bar(&ctx);
    let cancel_safety = acts.iter().any(|a| matches!(a, Action::CancelResting { id } if id.contains("SAFETY-*")));
    assert!(cancel_safety, "manual close must issue CancelResting wildcard for SAFETY orders");

    // Fill TP close
    let close_fill = make_fill(&format!("{}-TP-1", e.bot_id), "TP", Side::Sell, OrderKind::Reduce, "102", "1", "0.01", 2000);
    let fill_acts = e.on_fill(&close_fill);
    let fill_cancel_safety = fill_acts.iter().any(|a| matches!(a, Action::CancelResting { id } if id.contains("SAFETY-*")));
    assert!(fill_cancel_safety, "on_fill completing deal must issue CancelResting wildcard for SAFETY orders");
    assert_eq!(e.phase, DcaPhase::Cooldown);
}

#[test]
fn liquidated_close_then_force_liquidate_double_counts() {
    // Audit 1.6: force_liquidate called after on_fill(LIQ) must be idempotent
    let mut e = DcaEngine::new("liq-idempotent", BotConfig::default(), Instrument::default()).unwrap();
    let bar = make_bar(1000, "100", "100", "100", "100");
    let zero = Fraction::zero();
    let ctx = dummy_context(&bar, f("100"), &zero);
    let _ = e.on_bar(&ctx);

    let base_fill = make_fill(&format!("{}-BASE-1", e.bot_id), "BASE", Side::Buy, OrderKind::Add, "100", "1", "0.01", 1000);
    e.on_fill(&base_fill);

    let liq_fill = make_fill(&format!("{}-LIQ-1", e.bot_id), "LIQ", Side::Sell, OrderKind::Reduce, "80", "1", "0.01", 2000);
    e.on_fill(&liq_fill);
    assert_eq!(e.losses, 1);
    assert_eq!(e.deals_closed, 1);

    // Subsequent runner invocation of force_liquidate must skip double count
    e.force_liquidate();
    assert_eq!(e.losses, 1, "losses must not be incremented twice");
    assert_eq!(e.deals_closed, 1, "deals_closed must not be incremented twice");
}

#[test]
fn geometric_grid_integer_anchors_create_duplicate_levels() {
    // Audit 1.8: Close bounds like 100..101 with n=2 must not produce duplicate levels [100, 100, 101]
    let mesh = geometric_mesh(&f("100"), &f("101"), 2).unwrap();
    assert_eq!(mesh.len(), 3);
    assert_eq!(mesh[0].to_string(), "100");
    assert_eq!(mesh[2].to_string(), "101");
    assert!(mesh[0] < mesh[1], "level 0 must be strictly less than level 1");
    assert!(mesh[1] < mesh[2], "level 1 must be strictly less than level 2");
    assert_eq!(mesh[1].to_half_even_display(4), "100.4988");
}

#[test]
fn m20_does_not_test_stop_before_first_safety() {
    // Audit 1.8: M20 must detect if SL triggers before the first safety order can ever fill
    let levels = dca_math::m02::build_dca_plan(
        &f("100"), &f("1"), &f("1"), &f("0.05"), &f("1"), &f("1"),
        3, Direction::Long, &f("0.01"), &f("0.001"), SizingMode::BaseQty,
    ).unwrap();
    // Anchor 100, Level 1 at 95. SL=2% (triggers at 98). First safety is unreachable!
    let (viable, first_unreachable, _) = m20_conflict_scan(Direction::Long, &f("0.02"), &f("1"), &f("100"), &levels).unwrap();
    assert!(!viable, "must be non-viable because SL fires at 98 before first safety at 95");
    assert_eq!(first_unreachable, Some(0), "level 0 (base order) must be flagged as conflict trigger");
}

#[test]
fn hedge_dual_fractional_base_fails_to_open_hedge_leg() {
    // Audit 1.5: Hedge Dual must initialize both main and hedge legs correctly
    let mut cfg = BotConfig::default();
    cfg.kind = StrategyKind::HedgeDual;
    cfg.base_order_size = "100".into();
    cfg.hedge_ratio = "0.5".into();
    let mut hs = HedgeEngine::new("hedge-test", cfg, Instrument::default()).unwrap();
    
    // First bar opens both legs
    let bar = make_bar(1000, "100", "100", "100", "100");
    let zero = Fraction::zero();
    let ctx = dummy_context(&bar, f("100"), &zero);
    let acts = hs.process_bar(&ctx);
    
    let has_main = acts.iter().any(|a| matches!(a, Action::PlaceMarket { role: "BASE", id, side: Side::Buy, .. } if id.contains("-main-")));
    let has_hedge = acts.iter().any(|a| matches!(a, Action::PlaceMarket { role: "BASE", id, side: Side::Sell, .. } if id.contains("-hedge-")));
    assert!(has_main, "Hedge Dual must open main leg");
    assert!(has_hedge, "Hedge Dual must open hedge leg on first bar");
}

#[test]
fn gap_batch_counts_previous_filled_twice() {
    // Audit 2: In SimulationStepper, multiple limit order fills in a gap bar
    // must add only each order's qty to candidate_batch, avoiding double counting.
    let mut st = SimulationStepper::new(Instrument::default(), f("10000"), StepperConfig::default());
    st.submit_intent(OrderIntent {
        id: "o1".into(),
        side: Side::Buy,
        kind: OrderKind::Add,
        price: Some(f("95")),
        qty: f("1"),
        decision_ts: 1000,
        role: "SAFETY".into(),
        placement_ref_price: None,
        quote_notional: None,
    }, 1000).unwrap();
    st.submit_intent(OrderIntent {
        id: "o2".into(),
        side: Side::Buy,
        kind: OrderKind::Add,
        price: Some(f("90")),
        qty: f("2"),
        decision_ts: 1000,
        role: "SAFETY".into(),
        placement_ref_price: None,
        quote_notional: None,
    }, 1000).unwrap();

    // Gap bar penetrates both 95 and 90 down to 85
    let bar = make_bar(2000, "98", "99", "85", "86");
    let fills = st.step(&bar, 2000, false).unwrap();
    assert_eq!(fills.len(), 2, "both orders must fill");
    assert_eq!(st.position.qty, f("3"), "total filled position qty must be 1 + 2 = 3 exactly");
}

#[test]
fn fraction_display_vs_from_decimal_str() {
    // Audit 2: Fraction::Display outputs canonical n/d ("1/2"), which must be parseable by from_decimal_str
    let frac = Fraction::new(1.into(), 2.into()).unwrap();
    assert_eq!(frac.to_string(), "1/2");
    let parsed = Fraction::from_decimal_str(&frac.to_string()).unwrap();
    assert_eq!(parsed, frac, "Fraction::from_decimal_str must parse canonical n/d format");
}
