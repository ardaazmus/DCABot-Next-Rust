//! M16 — Order reconciliation state machine (desired vs venue diff) + M18 hedge coupling.

use crate::fraction::Fraction;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueOrder {
    pub client_id: String,
    pub side: Side,
    pub price: Fraction,
    pub qty: Fraction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Side {
    Buy,
    Sell,
}

impl std::fmt::Display for Side {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Side::Buy => write!(f, "Buy"),
            Side::Sell => write!(f, "Sell"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ReconcileDiff {
    pub cancel: Vec<VenueOrder>,
    pub place: Vec<VenueOrder>,
    pub keep: Vec<VenueOrder>,
}

/// M16 — compute minimal diff. Cancellations dispatched BEFORE placements,
/// guaranteeing zero duplicate fills. Match key: client_id + side + price + qty exact.
pub fn reconcile(desired: &[VenueOrder], venue: &[VenueOrder]) -> ReconcileDiff {
    let mut diff = ReconcileDiff::default();
    for v in venue {
        let matched = desired
            .iter()
            .any(|d| d.client_id == v.client_id && d.side == v.side && d.price == v.price && d.qty == v.qty);
        if matched {
            diff.keep.push(v.clone());
        } else {
            diff.cancel.push(v.clone());
        }
    }
    for d in desired {
        let exists = venue
            .iter()
            .any(|v| d.client_id == v.client_id && d.side == v.side && d.price == v.price && d.qty == v.qty);
        if !exists {
            diff.place.push(d.clone());
        }
    }
    diff
}

// ---------------- M18 — Hedge coupling ----------------

/// Target net delta: Delta = q_main - rho * q_hedge.
pub fn target_net_delta(q_main: &Fraction, hedge_ratio: &Fraction, q_hedge: &Fraction) -> Result<Fraction, crate::fraction::MathError> {
    q_main.sub(&hedge_ratio.mul(q_hedge)?)
}

/// Combined exit condition: Pi_main + Pi_hedge >= T_combined.
pub fn combined_exit_reached(pnl_main: &Fraction, pnl_hedge: &Fraction, target: &Fraction) -> Result<bool, crate::fraction::MathError> {
    let combined = pnl_main.add(pnl_hedge)?;
    Ok(combined >= *target)
}

/// Hedge sizing: q_hedge = rho * q_main.
pub fn hedge_qty(q_main: &Fraction, rho: &Fraction) -> Result<Fraction, crate::fraction::MathError> {
    q_main.mul(rho)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fraction::Fraction as F;

    fn f(s: &str) -> Fraction {
        F::from_decimal_str(s).unwrap()
    }

    #[test]
    fn reconcile_diff_minimal() {
        let desired = vec![
            VenueOrder { client_id: "a".into(), side: Side::Buy, price: f("98"), qty: f("1") },
            VenueOrder { client_id: "b".into(), side: Side::Buy, price: f("96"), qty: f("1") },
        ];
        let venue = vec![
            VenueOrder { client_id: "a".into(), side: Side::Buy, price: f("98"), qty: f("1") },
            VenueOrder { client_id: "stale".into(), side: Side::Sell, price: f("120"), qty: f("2") },
        ];
        let d = reconcile(&desired, &venue);
        assert_eq!(d.keep.len(), 1);
        assert_eq!(d.cancel[0].client_id, "stale");
        assert_eq!(d.place[0].client_id, "b");
    }

    #[test]
    fn price_mismatch_causes_cancel_place() {
        let desired = vec![VenueOrder { client_id: "a".into(), side: Side::Buy, price: f("98"), qty: f("1") }];
        let venue = vec![VenueOrder { client_id: "a".into(), side: Side::Buy, price: f("99"), qty: f("1") }];
        let d = reconcile(&desired, &venue);
        assert_eq!(d.cancel.len(), 1);
        assert_eq!(d.place.len(), 1);
    }

    #[test]
    fn hedge_math() {
        let delta = target_net_delta(&f("10"), &f("0.5"), &f("10")).unwrap();
        assert_eq!(delta.to_string(), "5");
        assert!(hedge_qty(&f("10"), &f("0.5")).unwrap() == f("5"));
        assert!(combined_exit_reached(&f("6"), &f("4"), &f("10")).unwrap());
    }
}
