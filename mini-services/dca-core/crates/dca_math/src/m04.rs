//! M04 — Weighted Average Cost (WAC) position accounting with idempotent fills.

use crate::fraction::{Fraction, MathError};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Faz 3 (C-4): runtime snapshot — motor durumunun serde ile kalıcılığı için.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionState {
    /// +1 Long, -1 Short
    pub direction: i8,
    pub qty: Fraction,
    pub cost: Fraction,
    pub avg_price: Option<Fraction>,
    /// Realized GROSS pnl (fees/funding excluded; tracked separately by M05).
    pub realized_gross: Fraction,
    pub seen_fill_keys: HashSet<String>,
    /// Overfill / anomaly audit log.
    pub violations: Vec<String>,
}

impl PositionState {
    pub fn new(direction: i8) -> Self {
        Self {
            direction,
            qty: Fraction::zero(),
            cost: Fraction::zero(),
            avg_price: None,
            realized_gross: Fraction::zero(),
            seen_fill_keys: HashSet::new(),
            violations: Vec::new(),
        }
    }

    pub fn avg_price_or_zero(&self) -> Fraction {
        self.avg_price.clone().unwrap_or_else(Fraction::zero)
    }

    /// Apply a fill keyed by `fill_key` (idempotent). `is_add=true` expands
    /// inventory; `is_add=false` reduces (WAC: average price stays unchanged).
    /// Returns Ok(true) if state changed, Ok(false) if duplicate fill_key ignored.
    pub fn apply_fill(
        &mut self,
        fill_key: &str,
        price: &Fraction,
        qty: &Fraction,
        is_add: bool,
    ) -> Result<bool, MathError> {
        if self.seen_fill_keys.contains(fill_key) {
            return Ok(false);
        }
        if price.is_zero() || price.is_negative() {
            return Err(MathError::DomainViolation("fill price must be > 0".into()));
        }
        if qty.is_zero() || qty.is_negative() {
            return Err(MathError::DomainViolation("fill qty must be > 0".into()));
        }

        if is_add {
            self.qty = self.qty.add(qty)?;
            self.cost = self.cost.add(&qty.mul(price)?)?;
            self.avg_price = Some(self.cost.div(&self.qty)?);
        } else {
            // Reduction: cannot exceed open quantity (overfill -> violation record)
            if qty > &self.qty {
                self.violations.push(format!(
                    "OVERFILL reduce {} > open {} (fill_key={}); clamped",
                    qty, self.qty, fill_key
                ));
            }
            let r = Fraction::min(qty, &self.qty);
            if self.avg_price.is_none() {
                self.seen_fill_keys.insert(fill_key.to_string());
                return Ok(false); // reduce on flat position: nothing to do
            }
            let avg = self.avg_price.clone().unwrap();
            let diff = if self.direction == 1 { price.sub(&avg)? } else { avg.sub(price)? };
            let delta_realized = r.mul(&diff)?;
            self.realized_gross = self.realized_gross.add(&delta_realized)?;
            self.qty = self.qty.sub(&r)?;
            self.cost = self.cost.sub(&r.mul(&avg)?)?;
            if self.qty.is_zero() {
                self.cost = Fraction::zero();
                self.avg_price = None;
            }
        }

        self.seen_fill_keys.insert(fill_key.to_string());
        Ok(true)
    }

    /// Unrealized GROSS pnl at mark price m (M06 formula).
    pub fn unrealized_gross(&self, mark: &Fraction) -> Result<Fraction, MathError> {
        if self.qty.is_zero() {
            return Ok(Fraction::zero());
        }
        let diff = if self.direction == 1 { mark.sub(&self.avg_price_or_zero())? } else { self.avg_price_or_zero().sub(mark)? };
        self.qty.mul(&diff)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fraction::Fraction as F;

    fn f(s: &str) -> Fraction {
        F::from_decimal_str(s).unwrap()
    }

    #[test]
    fn wac_long_expansion() {
        let mut p = PositionState::new(1);
        p.apply_fill("k1", &f("100"), &f("1"), true).unwrap();
        p.apply_fill("k2", &f("60"), &f("1"), true).unwrap();
        // avg = (100+60)/2 = 80
        assert_eq!(p.avg_price.unwrap().to_string(), "80");
        assert_eq!(p.qty.to_string(), "2");
        assert_eq!(p.cost.to_string(), "160");
    }

    #[test]
    fn wac_partial_exit_keeps_avg() {
        let mut p = PositionState::new(1);
        p.apply_fill("k1", &f("100"), &f("2"), true).unwrap();
        p.apply_fill("k2", &f("110"), &f("1"), false).unwrap();
        // realized = 1*(110-100)=10; remaining avg stays 100
        assert_eq!(p.realized_gross.to_string(), "10");
        assert_eq!(p.avg_price.unwrap().to_string(), "100");
        assert_eq!(p.qty.to_string(), "1");
    }

    #[test]
    fn idempotent_fill_keys() {
        let mut p = PositionState::new(1);
        assert!(p.apply_fill("k1", &f("100"), &f("1"), true).unwrap());
        assert!(!p.apply_fill("k1", &f("100"), &f("1"), true).unwrap()); // duplicate ignored
        assert_eq!(p.qty.to_string(), "1");
    }

    #[test]
    fn short_realized() {
        let mut p = PositionState::new(-1);
        p.apply_fill("s1", &f("200"), &f("1"), true).unwrap();
        p.apply_fill("s2", &f("180"), &f("1"), false).unwrap();
        assert_eq!(p.realized_gross.to_string(), "20");
        assert!(p.qty.is_zero());
        assert!(p.avg_price.is_none());
    }

    #[test]
    fn overfill_violation_recorded() {
        let mut p = PositionState::new(1);
        p.apply_fill("k1", &f("100"), &f("1"), true).unwrap();
        p.apply_fill("k2", &f("110"), &f("5"), false).unwrap();
        assert_eq!(p.violations.len(), 1);
        assert!(p.qty.is_zero());
    }

    #[test]
    fn unrealized_signs() {
        let mut p = PositionState::new(1);
        p.apply_fill("k1", &f("100"), &f("2"), true).unwrap();
        assert_eq!(p.unrealized_gross(&f("110")).unwrap().to_string(), "20");
        let mut s = PositionState::new(-1);
        s.apply_fill("k1", &f("100"), &f("2"), true).unwrap();
        assert_eq!(s.unrealized_gross(&f("90")).unwrap().to_string(), "20");
    }
}
