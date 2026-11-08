//! M15 — Grid mathematical suite (arithmetic/geometric meshes, fee breaker).
//! M17 — Infinity grid (constant base-value conservation).
//! M20 — Stop-loss to safety-ladder conflict detector.

use crate::fraction::{Fraction, MathError};
use crate::m02::Direction;

/// M15 — Arithmetic mesh: P_j = lower + j * (upper - lower)/n, exact rational.
pub fn arithmetic_mesh(lower: &Fraction, upper: &Fraction, n: usize) -> Result<Vec<Fraction>, MathError> {
    if n == 0 {
        return Err(MathError::DomainViolation("grid n must be > 0".into()));
    }
    if lower >= upper {
        return Err(MathError::DomainViolation("lower must be < upper".into()));
    }
    let span = upper.sub(lower)?;
    let step = span.div(&Fraction::from_u64(n as u64))?;
    let mut out = Vec::with_capacity(n + 1);
    for j in 0..=n {
        let p = lower.add(&step.mul(&Fraction::from_u64(j as u64))?)?;
        out.push(p);
    }
    Ok(out)
}

/// M15 — Geometric mesh with EXACT integer n-th roots: P_j^n = L^(n-j) * U^j.
/// P_j = nth_root(L^(n-j) * U^j, n) — zero-float via big-int Newton iteration.
pub fn geometric_mesh(lower: &Fraction, upper: &Fraction, n: usize) -> Result<Vec<Fraction>, MathError> {
    if n == 0 {
        return Err(MathError::DomainViolation("grid n must be > 0".into()));
    }
    if lower >= upper {
        return Err(MathError::DomainViolation("lower must be < upper".into()));
    }
    // To stay rational: scale both to integers (denominators are powers of 10 from decimal strings;
    // use LCM of denominators as common scale).
    let l_num = lower.num().clone();
    let u_num = upper.num().clone();
    let l_den = lower.den().clone();
    let u_den = upper.den().clone();
    let common_den = l_den.lcm(&u_den);
    let li = l_num * (&common_den / &l_den);
    let ui = u_num * (&common_den / &u_den);
    if li <= num_bigint::BigInt::ZERO || ui <= li {
        return Err(MathError::DomainViolation("grid bounds must be positive, lower < upper".into()));
    }
    let mut out = Vec::with_capacity(n + 1);
    for j in 0..=n {
        // value = li^(n-j) * ui^j ; root index n
        let pow_val = if j == 0 {
            li.pow(n as u32)
        } else if j == n {
            ui.pow(n as u32)
        } else {
            li.pow((n - j) as u32) * ui.pow(j as u32)
        };
        let root = integer_nth_root(&pow_val, n as u32);
        out.push(Fraction::new(root, common_den.clone())?);
    }
    Ok(out)
}

use num_integer::Integer as _;

/// Exact integer n-th root (floor) via binary search on BigInt.
pub fn integer_nth_root(v: &num_bigint::BigInt, n: u32) -> num_bigint::BigInt {
    use num_bigint::BigInt;
    use num_traits::{One, Zero};
    if n == 0 {
        return BigInt::one();
    }
    if v.is_zero() || *v == BigInt::one() {
        return v.clone();
    }
    let one = BigInt::one();
    let two = BigInt::from(2u32);
    // initial guess: bit-length / n
    let bits = v.bits() as u32;
    let mut hi: BigInt = BigInt::from(1u32) << ((bits / n + 2) as usize);
    let mut lo = BigInt::zero();
    while lo < hi {
        let mid = (&lo + &hi + &one) / &two;
        let mid_pow = mid.pow(n);
        if mid_pow <= *v {
            lo = mid;
        } else {
            hi = mid - &one;
        }
    }
    lo
}

/// M15 fee-breaker: g_min = (f_buy + f_sell) / (1 - f_sell) as a fraction of price.
/// Grid step % must EXCEED g_min or formation is rejected (fee erosion guard).
pub fn fee_breaker_min_step(fee_buy: &Fraction, fee_sell: &Fraction) -> Result<Fraction, MathError> {
    let denom = Fraction::one().sub(fee_sell)?;
    fee_buy.add(fee_sell)?.div(&denom)
}

/// Validate grid spacing against fee breaker. step_pct = (P_{j+1}/P_j - 1).
pub fn validate_grid_fee_breaker(
    mesh: &[Fraction],
    fee_buy: &Fraction,
    fee_sell: &Fraction,
) -> Result<bool, MathError> {
    if mesh.len() < 2 {
        return Ok(true);
    }
    let g_min = fee_breaker_min_step(fee_buy, fee_sell)?;
    for w in mesh.windows(2) {
        let step_pct = w[1].div(&w[0])?.sub(&Fraction::one())?;
        if step_pct <= g_min {
            return Ok(false);
        }
    }
    Ok(true)
}

/// M17 — Infinity grid: as price rises by dp, sell excess to keep base value constant.
/// dq = q - V_base / (p + dp); sells only when dq > 0 (quantized to step).
pub fn infinity_sell_delta(base_value: &Fraction, holding_qty: &Fraction, new_price: &Fraction, step: &Fraction) -> Result<Fraction, MathError> {
    if new_price.is_zero() || new_price.is_negative() {
        return Err(MathError::DomainViolation("price must be > 0".into()));
    }
    let target_qty = base_value.div(new_price)?;
    let dq = holding_qty.sub(&target_qty)?;
    if dq.is_positive() {
        crate::m02::qty_down(&dq, step)
    } else {
        Ok(Fraction::zero())
    }
}

/// M20 — Stop-loss to safety-ladder conflict detector.
/// For every level k: post-fill average A_k (base + levels 1..=k).
/// Level k+1 unreachable if SL_trigger(A_k) >= P_{k+1} (Long) — inverted for Short.
/// Returns: (is_viable, first_unreachable_level Option, per-level SL triggers).
pub fn m20_conflict_scan(
    direction: Direction,
    stop_loss_pct: &Fraction,
    base_qty: &Fraction,
    anchor_p0: &Fraction,
    levels: &[crate::m02::DcaLevel],
) -> Result<(bool, Option<usize>, Vec<Fraction>), MathError> {
    let mut qty = base_qty.clone();
    let mut cost = base_qty.mul(anchor_p0)?;
    let mut triggers = Vec::with_capacity(levels.len());
    let mut first_unreachable: Option<usize> = None;

    for (idx, l) in levels.iter().enumerate() {
        let next_price = &l.price;
        // A_k after THIS level fills:
        qty = qty.add(&l.qty)?;
        cost = cost.add(&l.qty.mul(&l.price)?)?;
        let a_k = cost.div(&qty)?;
        let sl_trigger = if direction == Direction::Long {
            a_k.mul(&Fraction::one().sub(stop_loss_pct)?)?
        } else {
            a_k.mul(&Fraction::one().add(stop_loss_pct)?)?
        };
        triggers.push(sl_trigger.clone());
        // Is the NEXT level reachable before SL fires? For the LAST level there is no next.
        let is_last = idx + 1 == levels.len();
        if !is_last {
            let next_level_price = &levels[idx + 1].price;
            let unreachable = if direction == Direction::Long {
                sl_trigger >= *next_level_price
            } else {
                sl_trigger <= *next_level_price
            };
            if unreachable && first_unreachable.is_none() {
                first_unreachable = Some(l.level_index);
            }
        }
        let _ = next_price;
    }
    Ok((first_unreachable.is_none(), first_unreachable, triggers))
}

/// Convenience: full M20 validation of a raw config (builds plan internally).
#[allow(clippy::too_many_arguments)]
pub fn m20_validate_config(
    direction: Direction,
    anchor_p0: &Fraction,
    base_qty: &Fraction,
    safety_size: &Fraction,
    deviation_pct: &Fraction,
    step_scale: &Fraction,
    volume_scale: &Fraction,
    safety_count: usize,
    sizing_mode: SizingModeReexport,
    tick_size: &Fraction,
    step_size: &Fraction,
    stop_loss_pct: &Fraction,
) -> Result<(Vec<crate::m02::DcaLevel>, bool, Option<usize>), MathError> {
    let levels = crate::m02::build_dca_plan(
        anchor_p0,
        base_qty,
        safety_size,
        deviation_pct,
        step_scale,
        volume_scale,
        safety_count,
        direction,
        tick_size,
        step_size,
        match sizing_mode {
            SizingModeReexport::BaseQty => crate::m02::SizingMode::BaseQty,
            SizingModeReexport::QuoteNotional => crate::m02::SizingMode::QuoteNotional,
        },
    )?;
    if stop_loss_pct.is_zero() {
        return Ok((levels, true, None));
    }
    let (viable, first, _) = m20_conflict_scan(direction, stop_loss_pct, base_qty, anchor_p0, &levels)?;
    Ok((levels, viable, first))
}

pub use crate::m02::SizingMode as SizingModeReexport;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fraction::Fraction as F;

    fn f(s: &str) -> Fraction {
        F::from_decimal_str(s).unwrap()
    }

    #[test]
    fn arithmetic_mesh_exact() {
        let mesh = arithmetic_mesh(&f("100"), &f("110"), 10).unwrap();
        assert_eq!(mesh.len(), 11);
        assert_eq!(mesh[3].to_string(), "103");
        assert_eq!(mesh[10].to_string(), "110");
    }

    #[test]
    fn geometric_mesh_exact_roots() {
        // 100 -> 400, n=2: ratio = 2 -> levels 100, 200, 400 (exact integer roots)
        let mesh = geometric_mesh(&f("100"), &f("400"), 2).unwrap();
        assert_eq!(mesh[0].to_string(), "100");
        assert_eq!(mesh[1].to_string(), "200");
        assert_eq!(mesh[2].to_string(), "400");
        // 100 -> 121, n=2: ratio=1.1 -> 100, 110, 121
        let mesh2 = geometric_mesh(&f("100"), &f("121"), 2).unwrap();
        assert_eq!(mesh2[1].to_string(), "110");
    }

    #[test]
    fn nth_root_binary_search() {
        use num_bigint::BigInt;
        assert_eq!(integer_nth_root(&BigInt::from(1_000_000u64), 2).to_string(), "1000");
        assert_eq!(integer_nth_root(&BigInt::from(999_999u64), 2).to_string(), "999");
        let big = BigInt::from(2u32).pow(60u32);
        assert_eq!(integer_nth_root(&big, 2).to_string(), format!("{}", 2u64.pow(30)));
    }

    #[test]
    fn fee_breaker() {
        // f_buy = f_sell = 0.001 -> g_min = 0.002/0.999 = 0.002002...
        let g = fee_breaker_min_step(&f("0.001"), &f("0.001")).unwrap();
        assert!(g > f("0.002") && g < f("0.0021"));
        // Mesh with 1% steps passes; with 0.1% steps fails.
        let mesh_ok = arithmetic_mesh(&f("100"), &f("110"), 10).unwrap();
        assert!(validate_grid_fee_breaker(&mesh_ok, &f("0.001"), &f("0.001")).unwrap());
        let mesh_bad = arithmetic_mesh(&f("100"), &f("101"), 10).unwrap();
        assert!(!validate_grid_fee_breaker(&mesh_bad, &f("0.001"), &f("0.001")).unwrap());
    }

    #[test]
    fn infinity_delta() {
        // V=1000 USDT, holding 10 @110 -> target at 110 is 1000/110=9.09..; dq=0.909.. -> 0.9 @step 0.1
        let dq = infinity_sell_delta(&f("1000"), &f("10"), &f("110"), &f("0.1")).unwrap();
        assert_eq!(dq.to_half_even_display(1), "0.9");
        // Price down: no sell
        let dq0 = infinity_sell_delta(&f("1000"), &f("10"), &f("90"), &f("0.1")).unwrap();
        assert!(dq0.is_zero());
    }

    #[test]
    fn m20_conflict_detected() {
        // P0=100, d=2%, a=1, 5 levels, v=1, base 1. SL=3%.
        // After level1 (98, q=2): A=99, SL_trig=96.03 >= P2=96 -> CONFLICT
        let levels = crate::m02::build_dca_plan(
            &f("100"), &f("1"), &f("1"), &f("0.02"), &f("1"), &f("1"),
            5, Direction::Long, &f("0.01"), &f("0.001"), crate::m02::SizingMode::BaseQty,
        )
        .unwrap();
        let (viable, first, _) = m20_conflict_scan(Direction::Long, &f("0.03"), &f("1"), &f("100"), &levels).unwrap();
        assert!(!viable);
        assert_eq!(first, Some(1));
    }

    #[test]
    fn m20_wide_ladder_viable() {
        // d=2%, a=1, n=5, sl=10%: after each fill A_k*(1-0.10) stays below next level
        let levels = crate::m02::build_dca_plan(
            &f("100"), &f("1"), &f("1"), &f("0.02"), &f("1"), &f("1"),
            5, Direction::Long, &f("0.01"), &f("0.001"), crate::m02::SizingMode::BaseQty,
        )
        .unwrap();
        let (viable, first, _) = m20_conflict_scan(Direction::Long, &f("0.10"), &f("1"), &f("100"), &levels).unwrap();
        assert!(viable, "expected viable, first_unreachable={:?}", first);
    }
}
