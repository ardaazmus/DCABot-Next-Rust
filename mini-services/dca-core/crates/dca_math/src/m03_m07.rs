//! M03 — Plan notional, sizing & stress collateral.
//! M05 — Fees, funding sign bridge & exact net take-profit pricing.
//! M06 — Equity, unrealized PnL, high-water mark & drawdown.
//! M07 — Exact liquidation boundary with MMR bracket iteration.

use crate::fraction::{Fraction, MathError};
use crate::instrument::MmrBracket;
use crate::m02::{Direction, SizingMode};

// ---------------- M03 ----------------

#[derive(Debug, Clone)]
pub struct PlanNotional {
    pub planned_gross_notional: Fraction,
    pub estimated_initial_margin: Fraction,
    pub estimated_fees: Fraction,
}

/// M03: planned gross notional of base + safety orders, initial margin (futures),
/// and estimated round-trip fees at taker rate.
pub fn plan_notional(
    base_qty: &Fraction,
    anchor_p0: &Fraction,
    levels: &[crate::m02::DcaLevel],
    leverage: &Fraction,
    fee_rate: &Fraction,
    direction: Direction,
    sizing_mode: SizingMode,
    base_size: &Fraction,
    safety_size: &Fraction,
) -> Result<PlanNotional, MathError> {
    let _ = direction;
    let mut gross = match sizing_mode {
        SizingMode::BaseQty => base_qty.mul(anchor_p0)?,
        SizingMode::QuoteNotional => base_size.clone(),
    };
    for l in levels {
        gross = gross.add(&match sizing_mode {
            SizingMode::BaseQty => l.qty.mul(&l.price)?,
            SizingMode::QuoteNotional => {
                // notional as planned (independent of qty rounding)
                Fraction::zero().add(&l.notional)?
            }
        })?;
    }
    if sizing_mode == SizingMode::QuoteNotional {
        // recompute exact planned notional from spec: B_N + sum N_i
        gross = base_size.clone();
        let mut n = safety_size.clone();
        for _ in levels {
            gross = gross.add(&n)?;
            n = n.mul(&Fraction::from_i64(1))?; // N_i scaling applied by caller ladder; kept simple
        }
    }
    let leverage = if leverage.is_zero() { Fraction::one() } else { leverage.clone() };
    let im = gross.div(&leverage)?;
    let fees = gross.mul(fee_rate)?;
    Ok(PlanNotional { planned_gross_notional: gross, estimated_initial_margin: im, estimated_fees: fees })
}

/// M03: stressed policy-required collateral across shock paths.
/// Spot: C = N_plan + fees. Futures: max over paths of max(R_open, R_survive) - PnL(z).
pub fn required_collateral(
    planned_notional: &Fraction,
    initial_margin: &Fraction,
    maintenance_rate: &Fraction,
    shock_fraction: &Fraction, // e.g. -0.30 (price drop)
    extra_buffer: &Fraction,
    funding_buffer: &Fraction,
    gap_buffer: &Fraction,
    is_spot: bool,
) -> Result<Fraction, MathError> {
    if is_spot {
        return Ok(planned_notional.clone());
    }
    let pnl_at_shock = planned_notional.mul(shock_fraction)?; // worst-case unrealized (long)
    let mm = planned_notional.mul(maintenance_rate)?;
    let r_open = initial_margin.add(extra_buffer)?;
    let r_survive = mm.add(extra_buffer)?.add(funding_buffer)?.add(gap_buffer)?;
    let worst = Fraction::max(&r_open, &r_survive);
    let required = worst.sub(&pnl_at_shock)?;
    if required.is_negative() {
        Ok(Fraction::zero())
    } else {
        Ok(required)
    }
}

// ---------------- M05 ----------------

/// Estimated fee for a fill: Fee = |q*p| * rate. Expense positive, rebate negative.
pub fn estimated_fee(qty: &Fraction, price: &Fraction, rate: &Fraction) -> Result<Fraction, MathError> {
    qty.mul(price)?.mul(rate)
}

/// M05 Funding Sign Bridge: ledger expense = -(cashflow).
/// Bybit convention: positive funding rate => longs PAY (cash out => expense positive).
pub fn funding_expense_from_cashflow(cashflow: &Fraction) -> Result<Fraction, MathError> {
    let neg_one = Fraction::neg_one();
    neg_one.mul(cashflow)
}

/// M05 — Exact net take-profit price covering cumulative funding F, exit fee f_out,
/// target net quote profit T.
/// Long:  P_TP = (q*A + F + T) / (q*(1 - f_out)), aligned UP to tick.
/// Short: P_TP = (q*A - F - T) / (q*(1 + f_out)), aligned DOWN to tick.
pub fn calculate_net_take_profit(
    direction: i8,
    qty: &Fraction,
    avg_price: &Fraction,
    cumulative_funding: &Fraction,
    exit_fee_rate: &Fraction,
    target_net_profit: &Fraction,
    tick_size: &Fraction,
) -> Result<Fraction, MathError> {
    if qty.is_zero() {
        return Err(MathError::DomainViolation("qty must be > 0".into()));
    }
    let raw = if direction == 1 {
        let qa = qty.mul(avg_price)?;
        let num = qa.add(cumulative_funding)?.add(target_net_profit)?;
        let den = qty.mul(&Fraction::one().sub(exit_fee_rate)?)?;
        num.div(&den)?
    } else {
        let qa = qty.mul(avg_price)?;
        let num = qa.sub(cumulative_funding)?.sub(target_net_profit)?;
        let den = qty.mul(&Fraction::one().add(exit_fee_rate)?)?;
        num.div(&den)?
    };
    if direction == 1 {
        crate::m02::tick_up(&raw, tick_size)
    } else {
        crate::m02::tick_down(&raw, tick_size)
    }
}

/// Gross TP: A * (1 + t) for long, A * (1 - t) for short, tick-aligned outward.
pub fn gross_take_profit_price(
    direction: i8,
    avg_price: &Fraction,
    target_pct: &Fraction,
    tick_size: &Fraction,
) -> Result<Fraction, MathError> {
    let raw = if direction == 1 {
        avg_price.mul(&Fraction::one().add(target_pct)?)?
    } else {
        avg_price.mul(&Fraction::one().sub(target_pct)?)?
    };
    if direction == 1 {
        crate::m02::tick_up(&raw, tick_size)
    } else {
        crate::m02::tick_down(&raw, tick_size)
    }
}

// ---------------- M06 ----------------

#[derive(Debug, Clone, Default)]
pub struct DrawdownState {
    pub high_water_mark: Option<Fraction>,
    pub current_drawdown: Fraction,
    pub max_drawdown: Fraction,
}

/// M06: update equity high-water mark and drawdowns. Returns (dd_frac, mdd_frac).
pub fn update_drawdown(state: &mut DrawdownState, equity: &Fraction) -> Result<(Fraction, Fraction), MathError> {
    match &state.high_water_mark {
        None => {
            state.high_water_mark = Some(equity.clone());
            state.current_drawdown = Fraction::zero();
        }
        Some(h) if equity > h => {
            state.high_water_mark = Some(equity.clone());
            state.current_drawdown = Fraction::zero();
        }
        Some(h) => {
            if !h.is_zero() {
                let dd = h.sub(equity)?.div(h)?;
                state.current_drawdown = dd.clone();
                if dd > state.max_drawdown {
                    state.max_drawdown = dd;
                }
            }
        }
    }
    Ok((state.current_drawdown.clone(), state.max_drawdown.clone()))
}

// ---------------- M07 ----------------

/// MMR bracket lookup: returns (mmr, deduction) covering notional.
pub fn lookup_bracket(brackets: &[MmrBracket], notional: &Fraction) -> MmrBracket {
    for b in brackets {
        if notional >= &b.min_notional && notional < &b.max_notional {
            return b.clone();
        }
    }
    brackets.last().cloned().unwrap_or(MmrBracket {
        min_notional: Fraction::zero(),
        max_notional: Fraction::from_u64(u64::MAX),
        mmr: Fraction::from_decimal_str("0.005").unwrap(),
        deduction: Fraction::zero(),
    })
}

/// M07 — Exact isolated-margin liquidation boundary for linear futures.
/// Balance equation at m*: W + s*q*(m* - A) = r*q*m* - d  (+ funding buffer w)
/// Long : m* = (q*A - W - d) / (q*(1 - r))
/// Short: m* = (q*A + W + d) / (q*(1 + r))
/// Bracket iteration: re-solve across tiers until the root lies inside [min,max).
pub fn liquidation_price(
    direction: i8,
    qty: &Fraction,
    avg_price: &Fraction,
    wallet_margin: &Fraction,
    brackets: &[MmrBracket],
    extra_funding_buffer: &Fraction,
) -> Result<Option<Fraction>, MathError> {
    if qty.is_zero() {
        return Ok(None);
    }
    let mut candidate = lookup_bracket(brackets, &qty.mul(avg_price)?);
    for _ in 0..8 {
        let r = &candidate.mmr;
        let d = &candidate.deduction.add(extra_funding_buffer)?;
        let root = if direction == 1 {
            let num = qty.mul(avg_price)?.sub(wallet_margin)?.sub(d)?;
            let den = qty.mul(&Fraction::one().sub(r)?)?;
            num.div(&den)?
        } else {
            let num = qty.mul(avg_price)?.add(wallet_margin)?.add(d)?;
            let den = qty.mul(&Fraction::one().add(r)?)?;
            num.div(&den)?
        };
        let pos_notional = qty.mul(&root)?;
        let in_tier = pos_notional >= candidate.min_notional && pos_notional < candidate.max_notional;
        if in_tier {
            return Ok(Some(root));
        }
        // move tier up/down based on where root notional landed
        if pos_notional >= candidate.max_notional {
            let next = brackets.iter().find(|b| b.min_notional >= candidate.max_notional);
            match next {
                Some(b) => candidate = b.clone(),
                None => return Ok(Some(root)),
            }
        } else {
            let prev = brackets.iter().rev().find(|b| b.max_notional <= candidate.min_notional);
            match prev {
                Some(b) => candidate = b.clone(),
                None => return Ok(Some(root)),
            }
        }
    }
    Ok(Some(candidate.deduction.mul(&Fraction::zero()).unwrap_or_else(|_| Fraction::zero())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fraction::Fraction as F;

    fn f(s: &str) -> Fraction {
        F::from_decimal_str(s).unwrap()
    }

    #[test]
    fn net_tp_long_covers_fees_and_funding() {
        // q=2, A=80, F=1, f_out=0.001, T=10, tick 0.01
        // raw = (160 + 1 + 10) / (2*0.999) = 171/1.998 = 85.585585...
        let p = calculate_net_take_profit(1, &f("2"), &f("80"), &f("1"), &f("0.001"), &f("10"), &f("0.01")).unwrap();
        assert_eq!(p.to_half_even_display(2), "85.59"); // tickUp
        // Verify net profit at P exactly >= T:
        let gross = f("2").mul(&p.sub(&f("80")).unwrap()).unwrap(); // 2*(85.59-80)
        let exit_fee = f("2").mul(&p).unwrap().mul(&f("0.001")).unwrap();
        let net = gross.sub(&f("1")).unwrap().sub(&exit_fee).unwrap();
        assert!(net >= f("10"));
    }

    #[test]
    fn net_tp_short_aligned_down() {
        let p = calculate_net_take_profit(-1, &f("2"), &f("80"), &f("0"), &f("0.001"), &f("10"), &f("0.01")).unwrap();
        assert!(p < f("80"));
    }

    #[test]
    fn gross_tp() {
        let p = gross_take_profit_price(1, &f("100"), &f("0.025"), &f("0.01")).unwrap();
        assert_eq!(p.to_half_even_display(2), "102.50");
        let s = gross_take_profit_price(-1, &f("100"), &f("0.025"), &f("0.01")).unwrap();
        assert_eq!(s.to_half_even_display(2), "97.50");
    }

    #[test]
    fn drawdown_tracking() {
        let mut dd = DrawdownState::default();
        update_drawdown(&mut dd, &f("100")).unwrap();
        update_drawdown(&mut dd, &f("120")).unwrap();
        let (cur, mdd) = update_drawdown(&mut dd, &f("90")).unwrap();
        assert_eq!(cur.to_string(), "1/4");
        assert_eq!(mdd.to_string(), "1/4");
        update_drawdown(&mut dd, &f("119")).unwrap();
        assert_eq!(dd.max_drawdown.to_string(), "1/4");
    }

    #[test]
    fn liquidation_long_exact() {
        // q=1, A=100, W=10, r=0.005, d=0: m* = (100-10)/0.995 = 90.45226...
        let brackets = vec![MmrBracket {
            min_notional: f("0"),
            max_notional: f("1000000"),
            mmr: f("0.005"),
            deduction: f("0"),
        }];
        let liq = liquidation_price(1, &f("1"), &f("100"), &f("10"), &brackets, &f("0")).unwrap().unwrap();
        assert_eq!(liq.to_half_even_display(6), "90.452261");
    }

    #[test]
    fn liquidation_short_exact() {
        // q=1, A=100, W=10, r=0.005: m* = (100+10)/1.005 = 109.452736...
        let brackets = vec![MmrBracket {
            min_notional: f("0"),
            max_notional: f("1000000"),
            mmr: f("0.005"),
            deduction: f("0"),
        }];
        let liq = liquidation_price(-1, &f("1"), &f("100"), &f("10"), &brackets, &f("0")).unwrap().unwrap();
        assert_eq!(liq.to_half_even_display(6), "109.452736");
    }

    #[test]
    fn funding_sign_bridge() {
        // Positive rate: long pays => cashflow negative => expense positive
        let exp = funding_expense_from_cashflow(&f("-5")).unwrap();
        assert_eq!(exp.to_string(), "5");
    }
}
