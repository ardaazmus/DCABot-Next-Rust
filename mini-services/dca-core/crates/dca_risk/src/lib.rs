//! dca_risk — The 8 Fail-Closed Risk Gates (ADR-006).
//! Pure, deterministic gate functions consumed by dca_kernel's SimulationStepper.
//! No gate may be bypassed; any FAIL halts the cycle or rejects the order intent.

use dca_math::fraction::Fraction;
use dca_math::sessions::{in_opening_salvo_buffer, is_market_open, SessionType};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FillRealism {
    MarketableTaker,
    RestingTouch,
    OpenGapBetter,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GateVerdict {
    Pass,
    Defer(String),
    Reject(String),
    Halt(String),
}

fn flt(a: &Fraction, b: &Fraction) -> bool {
    a.cmp(b) == std::cmp::Ordering::Less
}

/// Gate 1 — Strict data integrity & monotonicity.
pub fn gate1_bar_integrity(
    prev_ts: Option<i64>,
    t: i64,
    o: &Fraction,
    h: &Fraction,
    l: &Fraction,
    c: &Fraction,
    v: &Fraction,
    bar_interval_ms: i64,
) -> GateVerdict {
    // Faz 18 (P1-13): sıfır/negatif zaman damgası gerçek veri bozulmasıdır —
    // deterministik zaman ilkesi bozulur, Halt zorunludur.
    if t <= 0 {
        return GateVerdict::Halt("GATE1: non-positive bar timestamp".into());
    }
    if o.is_zero() || o.is_negative() || h.is_negative() || l.is_negative() || c.is_zero() {
        return GateVerdict::Halt("GATE1: non-positive price".into());
    }
    if flt(h, l) || flt(o, l) || flt(h, o) || flt(c, l) || flt(h, c) {
        return GateVerdict::Halt("GATE1: OHLC integrity violation".into());
    }
    if v.is_negative() {
        return GateVerdict::Halt("GATE1: negative volume".into());
    }
    // Faz 18 (P1-15): sıfır hacim = işlem görmemiş (donmuş/likiditesiz) bar.
    // Kalıcı veri bozulması değil — Halt değil, Defer: dolum kararı bu bara
    // uygulanmaz, emirler sonraki likit bara devreder (kernel skip & resync).
    if v.is_zero() {
        return GateVerdict::Defer("GATE1: zero-volume bar (frozen market)".into());
    }
    if let Some(pt) = prev_ts {
        if t <= pt {
            return GateVerdict::Halt("GATE1: non-monotonic bar timestamps".into());
        }
        if bar_interval_ms > 0 && t - pt != bar_interval_ms {
            return GateVerdict::Reject(format!(
                "GATE1: interval gap {} != expected {}",
                t - pt,
                bar_interval_ms
            ));
        }
    }
    GateVerdict::Pass
}

/// Gate 2 — Universal fee deduction. Zero-fee fills rejected unless verified promo.
pub fn gate2_fee_guard(fee_rate: &Fraction, is_verified_zero_fee_promo: bool) -> GateVerdict {
    // Faz 17 (P1-12): oran aralığı denetimi — negatif ve saçma değerler reddedilir.
    if fee_rate.is_negative() {
        return GateVerdict::Reject("GATE2: negative fee rate rejected".into());
    }
    if *fee_rate > Fraction::from_decimal_str("0.01").unwrap_or_else(|_| Fraction::zero()) {
        return GateVerdict::Reject("GATE2: absurd fee rate (>1%) rejected".into());
    }
    if fee_rate.is_zero() && !is_verified_zero_fee_promo {
        return GateVerdict::Reject("GATE2: zero-fee fill rejected (no verified promo pair)".into());
    }
    GateVerdict::Pass
}

/// Gate 3 — Market order slippage penalty.
/// Buy fills at open*(1+slip), Sell fills at open*(1-slip).
pub fn gate3_slippage(side: dca_math::Side, open: &Fraction, slip_bps: &Fraction) -> Result<Fraction, String> {
    if slip_bps.is_negative() {
        return Err("GATE3: negative slippage".into());
    }
    let slip = open.mul(slip_bps).map_err(|e| e.to_string())?.div(&Fraction::from_u64(10_000)).map_err(|e| e.to_string())?;
    match side {
        dca_math::Side::Buy => open.add(&slip).map_err(|e| e.to_string()),
        dca_math::Side::Sell => open.sub(&slip).map_err(|e| e.to_string()),
    }
}

/// Gate 4 — Zero look-ahead bias: decisions confined to closed bars.
/// order_ts must be strictly after decision bar close.
pub fn gate4_no_lookahead(order_eligible_ts: i64, current_bar_close_ts: i64) -> GateVerdict {
    if order_eligible_ts <= current_bar_close_ts {
        return GateVerdict::Reject("GATE4: order would execute at/before decision bar close".into());
    }
    GateVerdict::Pass
}

/// Gate 5 — Limit realism tagging + max bar volume share (10%).
///
/// Task 16 (spec 2.10) dolum modeli:
/// - `optimistic_touch = false` (REALISTIC — varsayılan): buy için bar low limitin
///   ALTINA inmiş (en az 1 tick penetrasyon), sell için high ÜSTÜNE çıkmış olmalı.
///   Teğet dokunuş (low == limit) DOLMAZ — dokümanlanmış RESTING_TOUCH semantiği.
/// - `optimistic_touch = true` (OPTIMISTIC/legacy): teğet dokunuş (low == limit)
///   dolmuş sayılır; canlı borsada bu emirlerin çoğu dolmaz.
/// OpenGapBetter (open zaten limitin ötesinde/üstünde açıldı) her iki modda da
/// açılış fiyatından dolar — borsa açılışında emir zaten piyasa fiyatındadır.
#[allow(clippy::too_many_arguments)]
pub fn gate5_limit_realism(
    side: dca_math::Side,
    limit_price: &Fraction,
    placement_price: &Fraction,
    bar_open: &Fraction,
    bar_high: &Fraction,
    bar_low: &Fraction,
    order_qty: &Fraction,
    bar_volume: &Fraction,
    already_filled_qty_this_bar: &Fraction,
    optimistic_touch: bool,
) -> (FillRealism, GateVerdict) {
    let tag = if side == dca_math::Side::Buy && (flt(placement_price, limit_price) || limit_price == placement_price) {
        FillRealism::MarketableTaker
    } else if side == dca_math::Side::Sell && (flt(limit_price, placement_price) || limit_price == placement_price) {
        FillRealism::MarketableTaker
    } else if side == dca_math::Side::Buy && (flt(bar_open, limit_price) || bar_open == limit_price) {
        FillRealism::OpenGapBetter
    } else if side == dca_math::Side::Sell && (flt(limit_price, bar_open) || bar_open == limit_price) {
        FillRealism::OpenGapBetter
    } else {
        FillRealism::RestingTouch
    };
    // Penetration check: REALISTIC buy fills only if bar low < limit (1-tick
    // beyond); OPTIMISTIC allows the touch (low == limit). Sell mirrored.
    let penetrated = if optimistic_touch {
        if side == dca_math::Side::Buy {
            flt(bar_low, limit_price) || bar_low == limit_price
        } else {
            flt(limit_price, bar_high) || bar_high == limit_price
        }
    } else if side == dca_math::Side::Buy {
        flt(bar_low, limit_price)
    } else {
        flt(limit_price, bar_high)
    };
    if !penetrated {
        return (tag, GateVerdict::Defer("GATE5: no price penetration".into()));
    }
    // Volume share cap: single bar cannot fill > 10% of its volume for strategy orders.
    let cap = bar_volume
        .mul(&Fraction::from_decimal_str("0.1").unwrap())
        .map_err(|e| e.to_string())
        .unwrap_or_else(|_| Fraction::zero());
    let total = already_filled_qty_this_bar
        .add(order_qty)
        .unwrap_or_else(|_| Fraction::zero());
    if flt(&cap, &total) {
        return (tag, GateVerdict::Defer("GATE5: bar volume share cap (10%) exceeded".into()));
    }
    (tag, GateVerdict::Pass)
}

/// Gate 6 — Pre-trade capital & margin guard.
/// cash_required = q*p + fee (spot) or q*p/L + fee (futures). SKIP/WAIT on shortfall.
pub fn gate6_capital_guard(
    qty: &Fraction,
    price: &Fraction,
    fee: &Fraction,
    leverage: &Fraction,
    is_spot: bool,
    available: &Fraction,
    mode: &CapitalShortfallMode,
) -> GateVerdict {
    // Faz 17 (P1-9): aritmetik hata fail-open DEĞİL — Halt döner (required=0
    // sahte PASS üretirdi). Kernel Halt'ı "GERÇEK veri bozulması" olarak işler.
    let notional = match qty.mul(price) {
        Ok(n) => n,
        Err(e) => return GateVerdict::Halt(format!("GATE6: notional hesap hatası: {e}")),
    };
    let required = if is_spot {
        match notional.add(fee) {
            Ok(x) => x,
            Err(e) => return GateVerdict::Halt(format!("GATE6: required hesap hatası: {e}")),
        }
    } else {
        let lev = if leverage.is_zero() { Fraction::one() } else { leverage.clone() };
        match notional.div(&lev).and_then(|x| x.add(fee)) {
            Ok(x) => x,
            Err(e) => return GateVerdict::Halt(format!("GATE6: required hesap hatası: {e}")),
        }
    };
    if required <= *available {
        GateVerdict::Pass
    } else {        match mode {
            CapitalShortfallMode::Skip => GateVerdict::Reject(format!(
                "GATE6 SKIP: required {} > available {}",
                required.to_half_even_display(8),
                available.to_half_even_display(8)
            )),
            CapitalShortfallMode::Wait => GateVerdict::Defer(format!(
                "GATE6 WAIT: required {} > available {}",
                required.to_half_even_display(8),
                available.to_half_even_display(8)
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapitalShortfallMode {
    Skip,
    Wait,
}

/// Gate 7 — Session & macro blackout: new risk intents rejected when closed;
/// risk-reducing exits stay protected.
pub fn gate7_session_guard(session: SessionType, ts_ms: i64, macro_blackout: bool, is_risk_reducing: bool) -> GateVerdict {
    let ts = chrono::DateTime::from_timestamp_millis(ts_ms).unwrap_or_else(|| chrono::Utc::now());
    if is_risk_reducing {
        return GateVerdict::Pass; // TP/SL/reduce always protected
    }
    if !is_market_open(session, ts) {
        return GateVerdict::Reject("GATE7: market closed (FAIL_CLOSED on ADD intent)".into());
    }
    if macro_blackout {
        return GateVerdict::Reject("GATE7: macro blackout window (CPI/FOMC)".into());
    }
    GateVerdict::Pass
}

/// Gate 8 — Opening salvo buffer (09:30–09:35 ET).
///
/// Faz 18 (P1-14): gap-through birleştirme bu kapıdan ÇIKARILDI — seviye
/// fiyatlarını çağıran hiçbir zaman gerçek bir liste iletmiyordu (`&[]` çağrısı
/// her zaman Clear üretirdi: ölü dal) ve `consolidated_qty` seviye başına 1
/// sayan sahte bir büyüklüktü. Gerçek birleştirme (gerçek adet + sentetik VWAP,
/// hacim payı kuralıyla) kernel'in Gate 8b `gap_batch` yolunda yaşar; bu kapı
/// yalnız açılış salvo tamponunu korur.
pub enum Gate8Outcome {
    Clear,
    DeferOpeningSalvo,
}

/// Daily drawdown breaker threshold (Task 18 fix N-1).
///
/// `daily_dd_breaker_pct` is stored as a PERCENT string ("2" = %2, validated to
/// [0,100] in risk_policy). The runtime compares it against a drawdown FRACTION
/// (dd = (anchor − equity)/anchor, so 2% = 2/100). The threshold therefore must
/// be divided by 100 before the comparison — previously the raw percent was
/// compared against the fraction, which demanded a 200% equity loss for the
/// default "2" and left the breaker permanently dormant.
pub fn daily_dd_breaker_threshold(pct_str: &str) -> Fraction {
    Fraction::from_decimal_str(pct_str)
        .unwrap_or_else(|_| Fraction::from_u64(2))
        .div(&Fraction::from_u64(100))
        .unwrap_or_else(|_| Fraction::from_decimal_str("0.02").unwrap_or_else(|_| Fraction::zero()))
}

pub fn gate8_gap_protection(is_safety_order: bool, ts_ms: i64) -> Gate8Outcome {
    if in_opening_salvo_buffer(chrono::DateTime::from_timestamp_millis(ts_ms).unwrap_or_else(|| chrono::Utc::now())) {
        if is_safety_order {
            return Gate8Outcome::DeferOpeningSalvo;
        }
    }
    Gate8Outcome::Clear
}

#[cfg(test)]
mod tests {
    use super::*;
    use dca_math::Fraction as F;

    fn f(s: &str) -> Fraction {
        F::from_decimal_str(s).unwrap()
    }

    #[test]
    fn gate1_catches_ohlc_violations() {
        assert!(matches!(
            gate1_bar_integrity(None, 1000, &f("100"), &f("90"), &f("95"), &f("97"), &f("10"), 0),
            GateVerdict::Halt(_)
        ));
        assert!(matches!(
            gate1_bar_integrity(Some(1000), 900, &f("100"), &f("105"), &f("95"), &f("97"), &f("10"), 0),
            GateVerdict::Halt(_)
        ));
        assert!(matches!(
            gate1_bar_integrity(None, 1000, &f("100"), &f("105"), &f("95"), &f("97"), &f("10"), 0),
            GateVerdict::Pass
        ));
    }

    #[test]
    fn gate1_timestamp_and_frozen_bar() {
        // Faz 18 (P1-13): t <= 0 gerçek veri bozulması → Halt
        assert!(matches!(
            gate1_bar_integrity(None, 0, &f("100"), &f("105"), &f("95"), &f("97"), &f("10"), 0),
            GateVerdict::Halt(_)
        ));
        // Faz 18 (P1-15): hacim 0 → Defer (donmuş bar, emir devreder)
        assert!(matches!(
            gate1_bar_integrity(None, 1000, &f("100"), &f("105"), &f("95"), &f("97"), &f("0"), 0),
            GateVerdict::Defer(_)
        ));
        // negatif hacim kalır Halt
        assert!(matches!(
            gate1_bar_integrity(None, 1000, &f("100"), &f("105"), &f("95"), &f("97"), &f("-1"), 0),
            GateVerdict::Halt(_)
        ));
    }

    #[test]
    fn gate8_opening_salvo_only() {
        // Faz 18 (P1-14): kapı yalnız açılış salvo tamponunu korur; gap-through
        // birleştirme kernel Gate 8b'de gerçek adetle yapılır (ölü dal kaldırıldı).
        // 1776691860000 = 2026-04-20 13:31 UTC = 09:31 ET (EDT) — tampon içinde.
        let in_salvo = 1776691860000_i64;
        assert!(matches!(gate8_gap_protection(true, in_salvo), Gate8Outcome::DeferOpeningSalvo));
        assert!(matches!(gate8_gap_protection(false, in_salvo), Gate8Outcome::Clear));
        // 1776736800000 = 2026-04-20 22:00 UTC = 18:00 ET — tampon dışı.
        let outside = 1776736800000_i64;
        assert!(matches!(gate8_gap_protection(true, outside), Gate8Outcome::Clear));
    }

    #[test]
    fn gate2_zero_fee_rejected() {
        assert!(matches!(gate2_fee_guard(&f("0"), false), GateVerdict::Reject(_)));
        assert!(matches!(gate2_fee_guard(&f("0"), true), GateVerdict::Pass));
        assert!(matches!(gate2_fee_guard(&f("0.001"), false), GateVerdict::Pass));
    }

    #[test]
    fn gate3_slippage_direction() {
        let buy = gate3_slippage(dca_math::Side::Buy, &f("100"), &f("10")).unwrap(); // +0.1%
        assert_eq!(buy.to_half_even_display(4), "100.1000");
        let sell = gate3_slippage(dca_math::Side::Sell, &f("100"), &f("10")).unwrap();
        assert_eq!(sell.to_half_even_display(4), "99.9000");
    }

    #[test]
    fn gate4_lookahead() {
        assert!(matches!(gate4_no_lookahead(1000, 1000), GateVerdict::Reject(_)));
        assert!(matches!(gate4_no_lookahead(1001, 1000), GateVerdict::Pass));
    }

    #[test]
    fn daily_dd_breaker_threshold_is_fraction_of_percent() {
        // "2" (percent) must become 2/100 — the runtime dd is a fraction.
        let th = daily_dd_breaker_threshold("2");
        assert_eq!(th.to_half_even_display(6), "0.020000");
        // 1.5% → 0.015
        assert_eq!(daily_dd_breaker_threshold("1.5").to_half_even_display(6), "0.015000");
        // A 2% drawdown fraction must now trip the default "2" percent threshold.
        let dd = f("0.02");
        assert!(dd.cmp(&th) != std::cmp::Ordering::Less, "2% dd must trip 2% threshold");
        // Garbage input falls back to the 2% default (as a fraction).
        assert_eq!(daily_dd_breaker_threshold("abc").to_half_even_display(6), "0.020000");
        // 0 = disabled stays zero (engine skips when threshold is zero).
        assert!(daily_dd_breaker_threshold("0").is_zero());
    }

    #[test]
    fn gate5_resting_touch_and_volume_cap() {
        // REALISTIC (default): touch (low == limit) does NOT fill — strict 1-tick penetration.
        let (_, verdict) = gate5_limit_realism(
            dca_math::Side::Buy,
            &f("98"),
            &f("100"),
            &f("99"),
            &f("100.5"),
            &f("98"), // low == limit → touch only
            &f("1"),
            &f("100"),
            &f("0"),
            false,
        );
        assert!(matches!(verdict, GateVerdict::Defer(_)), "REALISTIC: teğet dokunuş dolmamalı");
        // OPTIMISTIC: same bar fills (legacy touch semantics).
        let (_, v_touch) = gate5_limit_realism(
            dca_math::Side::Buy, &f("98"), &f("100"), &f("99"), &f("100.5"), &f("98"),
            &f("1"), &f("100"), &f("0"), true,
        );
        assert!(matches!(v_touch, GateVerdict::Pass), "OPTIMISTIC: teğet dokunuş dolmalı");
        // Realistic pass: low strictly beyond the limit.
        let (tag, verdict) = gate5_limit_realism(
            dca_math::Side::Buy,
            &f("98"),
            &f("100"),
            &f("99"),
            &f("100.5"),
            &f("97.5"),
            &f("1"),
            &f("100"),
            &f("0"),
            false,
        );
        assert_eq!(tag, FillRealism::RestingTouch);
        assert!(matches!(verdict, GateVerdict::Pass));
        // volume cap: bar volume 5, order 1 -> 20% > 10%
        let (_, v2) = gate5_limit_realism(
            dca_math::Side::Buy, &f("98"), &f("100"), &f("99"), &f("100.5"), &f("97.5"),
            &f("1"), &f("5"), &f("0"), false,
        );
        assert!(matches!(v2, GateVerdict::Defer(_)));
        // no penetration at all
        let (_, v3) = gate5_limit_realism(
            dca_math::Side::Buy, &f("97"), &f("100"), &f("99"), &f("100.5"), &f("98.5"),
            &f("1"), &f("100"), &f("0"), false,
        );
        assert!(matches!(v3, GateVerdict::Defer(_)));
    }

    #[test]
    fn gate6_capital() {
        let ok = gate6_capital_guard(&f("1"), &f("100"), &f("0.1"), &f("1"), true, &f("100.2"), &CapitalShortfallMode::Skip);
        assert!(matches!(ok, GateVerdict::Pass));
        let bad = gate6_capital_guard(&f("1"), &f("100"), &f("0.1"), &f("1"), true, &f("50"), &CapitalShortfallMode::Skip);
        assert!(matches!(bad, GateVerdict::Reject(_)));
        let wait = gate6_capital_guard(&f("1"), &f("100"), &f("0.1"), &f("1"), true, &f("50"), &CapitalShortfallMode::Wait);
        assert!(matches!(wait, GateVerdict::Defer(_)));
    }

    #[test]
    fn gate7_session() {
        // Sunday crypto: ADD passes
        let t = 1776672000000_i64; // 2026-04-19? Use crypto always-open anyway
        assert!(matches!(gate7_session_guard(SessionType::Crypto24_7, t, false, false), GateVerdict::Pass));
        // Market closed rejects ADD, protects REDUCE
        let sat = 1776768000000_i64;
        assert!(matches!(gate7_session_guard(SessionType::UsEquity, sat, false, false), GateVerdict::Reject(_)));
        assert!(matches!(gate7_session_guard(SessionType::UsEquity, sat, false, true), GateVerdict::Pass));
    }
}
