//! M02 — DCA fixed-level plan & cumulative deviation + M08 tick/qty lattice.

use crate::fraction::{Fraction, MathError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SizingMode {
    BaseQty,
    QuoteNotional,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Direction {
    Long,
    Short,
}

impl Direction {
    pub fn sign(&self) -> i8 {
        match self {
            Direction::Long => 1,
            Direction::Short => -1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcaLevel {
    pub level_index: usize,
    pub price: Fraction,
    pub qty: Fraction,
    pub notional: Fraction,
    pub cumulative_deviation: Fraction,
}

/// M08: floor to lattice origin + h*n (origin 0 by default for exchange ticks).
pub fn tick_down(x: &Fraction, tick: &Fraction) -> Result<Fraction, MathError> {
    let n = x.floor_div(tick)?;
    Fraction::new(n * tick.num(), tick.den().clone())
}

/// M08: ceil to lattice.
pub fn tick_up(x: &Fraction, tick: &Fraction) -> Result<Fraction, MathError> {
    let n = x.ceil_div(tick)?;
    Fraction::new(n * tick.num(), tick.den().clone())
}

/// M08: quantities always quantize DOWN.
pub fn qty_down(q: &Fraction, step: &Fraction) -> Result<Fraction, MathError> {
    tick_down(q, step)
}

/// M02 — build the full safety-order ladder for a DCA plan.
///
/// Anchor P0 (base order final VWAP). For level i in 1..=n:
///   D_i = d * sum_{k=0}^{i-1} a^k   (geometric step accumulation)
///   Long:  P_i = tick_down(P0 * (1 - D_i))
///   Short: P_i = tick_up  (P0 * (1 + D_i))
/// Volume scale v multiplies safety sizes; BASE_QTY sizes in base units,
/// QUOTE_NOTIONAL sizes converted via qty = floor(N_i / P_i, step).
#[allow(clippy::too_many_arguments)]
pub fn build_dca_plan(
    anchor_p0: &Fraction,
    base_qty: &Fraction,
    safety_quote_size_or_qty: &Fraction,
    deviation_pct: &Fraction,
    step_scale: &Fraction,
    volume_scale: &Fraction,
    safety_count: usize,
    direction: Direction,
    tick_size: &Fraction,
    step_size: &Fraction,
    sizing_mode: SizingMode,
) -> Result<Vec<DcaLevel>, MathError> {
    if deviation_pct.is_zero() || deviation_pct.is_negative() {
        return Err(MathError::DomainViolation("deviation d must be > 0".into()));
    }
    if step_scale.is_negative() {
        return Err(MathError::DomainViolation("step scale a must be >= 0".into()));
    }
    if volume_scale.is_negative() {
        return Err(MathError::DomainViolation("volume scale v must be >= 0".into()));
    }
    if anchor_p0.is_zero() || anchor_p0.is_negative() {
        return Err(MathError::DomainViolation("anchor P0 must be > 0".into()));
    }

    let mut levels = Vec::with_capacity(safety_count);
    let mut current_step_dev = deviation_pct.clone();
    let mut cum_dev = Fraction::zero();
    let mut current_size = safety_quote_size_or_qty.clone();
    // M-1 (SÖZLEŞME — testle sabitlenir): safety_order_size BİRİNCİ safety
    // emrinin TABANIDIR; volume_scale HER seviyede (ilkinde dâhil) uygulanır:
    //   safety_i = safety_order_size × v^i  (i = 1..n)
    // Yani safety=100, v=1.2 → ilk emir 120, ikinci 144. "İlk emir 100'ün
    // kendisi" BEKLENIYORSA v=1 verilmeli ya da safety = istenen_ilk / v
    // hesaplanmalı. Hesaplayıcı, sihirbaz ve motor aynı zinciri kullanır.
    for i in 1..=safety_count {
        cum_dev = cum_dev.add(&current_step_dev)?;
        if direction == Direction::Long && cum_dev >= Fraction::one() {
            return Err(MathError::DomainViolation(format!(
                "cumulative deviation {} >= 100%% at level {} (Long)",
                cum_dev, i
            )));
        }
        let raw_price = match direction {
            Direction::Long => anchor_p0.mul(&Fraction::one().sub(&cum_dev)?)?,
            Direction::Short => anchor_p0.mul(&Fraction::one().add(&cum_dev)?)?,
        };
        let price = match direction {
            Direction::Long => tick_down(&raw_price, tick_size)?,
            Direction::Short => tick_up(&raw_price, tick_size)?,
        };
        if price.is_zero() || price.is_negative() {
            return Err(MathError::DomainViolation(format!("level {} price <= 0", i)));
        }
        let qty = match sizing_mode {
            SizingMode::BaseQty => {
                current_size = current_size.mul(volume_scale)?;
                qty_down(&current_size, step_size)?
            }
            SizingMode::QuoteNotional => {
                current_size = current_size.mul(volume_scale)?;
                qty_down(&current_size.div(&price)?, step_size)?
            }
        };
        let notional = price.mul(&qty)?;
        // Untradeable levels (qty rounds to zero below the instrument step size)
        // are dropped from the plan: they can never fill, and emitting them
        // would poison M01 (fill qty must be > 0). Sizing chain continues so
        // subsequent levels keep their geometric sizes.
        if qty.is_zero() {
            current_step_dev = current_step_dev.mul(step_scale)?;
            continue;
        }
        // M08 filter: qty>0 and notional >= min_notional is validated by caller
        levels.push(DcaLevel { level_index: i, price, qty, notional, cumulative_deviation: cum_dev.clone() });
        current_step_dev = current_step_dev.mul(step_scale)?;
    }
    // Strictly decreasing (Long) / increasing (Short) tick-separated prices enforced:
    let mut prev = anchor_p0.clone();
    for l in &levels {
        match direction {
            Direction::Long if l.price >= prev => {
                return Err(MathError::DomainViolation(format!("level {} price not below previous tick level", l.level_index)))
            }
            Direction::Short if l.price <= prev => {
                return Err(MathError::DomainViolation(format!("level {} price not above previous tick level", l.level_index)))
            }
            _ => {}
        }
        prev = l.price.clone();
    }
    Ok(levels)
}

/// M02 test vectors — canonical plan with cumulative geometric deviation.
#[cfg(test)]
mod tests {
    use super::*;

    fn f(s: &str) -> Fraction {
        Fraction::from_decimal_str(s).unwrap()
    }

    
    /// M-1: safety_order_size × v^i zinciri — ilk emir TABAN × v'dir (100→120→144).
    #[test]
    fn m01_safety_volume_chain_first_level_is_base_times_v() {
        let anchor = Fraction::from_decimal_str("100000").unwrap();
        let base = Fraction::from_decimal_str("500").unwrap();
        let safety = Fraction::from_decimal_str("100").unwrap();
        let dev = Fraction::from_decimal_str("0.01").unwrap();
        let ss = Fraction::one();
        let vs = Fraction::from_decimal_str("1.2").unwrap();
        let tick = Fraction::from_decimal_str("0.1").unwrap();
        let step = Fraction::from_decimal_str("0.001").unwrap();
        let levels = build_dca_plan(&anchor, &base, &safety, &dev, &ss, &vs, 3, Direction::Long, &tick, &step, SizingMode::QuoteNotional).unwrap();
        assert_eq!(levels.len(), 3);
        // Beklenen notional: safety × v^i (100→120→144); tol = bir adım.
        let mut expected = safety.clone();
        for (i, l) in levels.iter().enumerate() {
            expected = expected.mul(&vs).unwrap();
            let got = l.qty.mul(&l.price).unwrap();
            // hangisi büyükse |got - expected|
            let diff = if got >= expected {
                got.sub(&expected).unwrap()
            } else {
                expected.sub(&got).unwrap()
            };
            let tol = l.price.mul(&step).unwrap();
            assert!(diff <= tol, "seviye {}: notional {} ≈ beklenen {} (tol {})", i + 1, got.to_half_even_display(4), expected.to_half_even_display(4), tol.to_half_even_display(6));
        }
    }

#[test]
    fn long_ladder_basic() {
        // P0=100, d=2%, a=1 (uniform steps), v=1, 3 safety orders, tick 0.01, step 0.001
        let levels = build_dca_plan(
            &f("100"), &f("0.1"), &f("0.1"), &f("0.02"), &f("1"), &f("1"),
            3, Direction::Long, &f("0.01"), &f("0.001"), SizingMode::BaseQty,
        )
        .unwrap();
        assert_eq!(levels.len(), 3);
        assert_eq!(levels[0].price.to_half_even_display(2), "98.00"); // 100*(1-0.02)
        assert_eq!(levels[1].price.to_half_even_display(2), "96.00"); // 1-0.04
        assert_eq!(levels[2].price.to_half_even_display(2), "94.00"); // 1-0.06
        assert_eq!(levels[2].cumulative_deviation.to_string(), "3/50");
    }

    #[test]
    fn long_ladder_geometric_steps() {
        // d=2%, a=2 -> steps 2%,4%,8% -> cum 2%,6%,14%
        let levels = build_dca_plan(
            &f("1000"), &f("0.01"), &f("0.01"), &f("0.02"), &f("2"), &f("1"),
            3, Direction::Long, &f("0.01"), &f("0.00001"), SizingMode::BaseQty,
        )
        .unwrap();
        assert_eq!(levels[0].cumulative_deviation.to_string(), "1/50");
        assert_eq!(levels[1].cumulative_deviation.to_string(), "3/50");
        assert_eq!(levels[2].cumulative_deviation.to_string(), "7/50");
        assert_eq!(levels[2].price.to_half_even_display(2), "860.00");
    }

    #[test]
    fn short_ladder_rises() {
        let levels = build_dca_plan(
            &f("100"), &f("0.1"), &f("0.1"), &f("0.03"), &f("1"), &f("1.5"),
            2, Direction::Short, &f("0.01"), &f("0.001"), SizingMode::BaseQty,
        )
        .unwrap();
        assert_eq!(levels[0].price.to_half_even_display(2), "103.00");
        assert_eq!(levels[1].price.to_half_even_display(2), "106.00");
        assert_eq!(levels[1].qty.to_half_even_display(4), "0.2250");
    }

    #[test]
    fn quote_notional_sizing_down() {
        // d=50%, a=0.5 -> P: 50 (cum 50%), 25 (cum 75%); N: 100, 200, 400 (v=2)
        let levels = build_dca_plan(
            &f("100"), &f("100"), &f("100"), &f("0.5"), &f("0.5"), &f("2"),
            2, Direction::Long, &f("0.01"), &f("0.1"), SizingMode::QuoteNotional,
        )
        .unwrap();
        // level1: N=200, P=50 -> q=4.0 ; level2: N=400, P=25 -> q=16.0
        assert_eq!(levels[0].qty.to_string(), "4");
        assert_eq!(levels[1].qty.to_string(), "16");
    }

    #[test]
    fn long_cum_dev_100_rejected() {
        let err = build_dca_plan(
            &f("100"), &f("1"), &f("1"), &f("0.6"), &f("1"), &f("1"),
            2, Direction::Long, &f("0.01"), &f("0.001"), SizingMode::BaseQty,
        )
        .unwrap_err();
        assert!(matches!(err, MathError::DomainViolation(_)));
    }

    #[test]
    fn tick_alignment() {
        assert_eq!(tick_down(&f("100.239"), &f("0.01")).unwrap().to_half_even_display(2), "100.23");
        assert_eq!(tick_up(&f("100.231"), &f("0.01")).unwrap().to_half_even_display(2), "100.24");
        assert_eq!(tick_up(&f("100.231"), &f("0.01")).unwrap().to_string(), "2506/25");
        assert_eq!(tick_down(&f("100.239"), &f("0.01")).unwrap().to_string(), "10023/100");
        assert_eq!(qty_down(&f("0.12345678"), &f("0.0001")).unwrap().to_half_even_display(8), "0.12340000");
    }
}
