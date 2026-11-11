//! /api/calc/dca + /api/calc/grid — saf matematik hesap uçları (state YOK:
//! salt girdi → salt çıktı).
//!
//! v3 eşlemesi:
//! - `POST /api/calc/dca`  ↔ v3 `POST /api/calc/dca` (v8 SAF-G2 formu)
//! - `POST /api/calc/grid` ↔ v3 `POST /api/bot/mesh` (M15–M20 ag + ucret kirici)
//!
//! Hesap motorları:
//! - dca:  `dca_math::m02` SAF-G2 (tavan-fiyat + agirlikli toplama SAF, weight
//!   tablosu üstel, cap=8, epsilon=1e-6; v3 ile ayni)
//! - grid: `Fraction` deterministik örgü + `dca_math::m15_m20` geometrik/aritmetik
//!   mesh + feeBreaker + gMin + totalInvested
//!
//! Not: indikatörler burada DEGIL — `GET /api/indicators` `routes_indicators.rs`'tedir.
//!
//! İki uç da `A0002` kuralına uyar: parse/validation hatası → 422
//! (`common::err422`), asla 500 degil.

use axum::response::IntoResponse;
use axum::Json;
use dca_math::fraction::Fraction;
use dca_math::m02::{Direction, SizingMode};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::routes_common::err422;

// ---------------- Calculators ----------------

#[allow(dead_code)]
#[derive(Deserialize)]
pub(crate) struct DcaCalcReq {
    anchor: String,
    base_order_size: String,
    safety_order_size: String,
    sizing_mode: String,
    deviation_pct: String,
    step_scale: String,
    volume_scale: String,
    max_safety_orders: usize,
    target_profit_pct: String,
    use_net_tp: bool,
    stop_loss_pct: Option<String>,
    tick_size: String,
    step_size: String,
    maker_fee: String,
    taker_fee: String,
    leverage: String,
    market: String, // spot | linear
    direction: Option<String>, // long | short (defaults to long)
}

pub(crate) async fn calc_dca(Json(req): Json<DcaCalcReq>) -> Result<axum::response::Response, axum::response::Response> {
    let fr = |s: &str| -> Result<Fraction, axum::response::Response> { Fraction::from_decimal_str(s).map_err(|e| err422(e)) };
    let anchor = fr(&req.anchor)?;
    let base = fr(&req.base_order_size)?;
    let safety = fr(&req.safety_order_size)?;
    let dev = fr(&req.deviation_pct)?;
    let step_scale = fr(&req.step_scale)?;
    let vol_scale = fr(&req.volume_scale)?;
    let tick = fr(&req.tick_size)?;
    let step = fr(&req.step_size)?;
    let leverage = fr(&req.leverage)?;
    let direction = match req.direction.as_deref() {
        Some(d) if d.eq_ignore_ascii_case("short") => Direction::Short,
        _ => Direction::Long,
    };

    let sizing_mode = if req.sizing_mode == "QUOTE_NOTIONAL" { SizingMode::QuoteNotional } else { SizingMode::BaseQty };

    // base_order_size is USDT under QUOTE_NOTIONAL — convert to coin qty exactly
    // like the runtime engine does (dca.rs), so totals/M20/liquidation all work
    // in coin units. Raw use here caused $41M phantom notionals on $500 orders.
    let base_qty = match sizing_mode {
        SizingMode::BaseQty => base.clone(),
        SizingMode::QuoteNotional => {
            let raw = base.div(&anchor).map_err(|e| err422(e))?;
            dca_math::m02::qty_down(&raw, &step).map_err(|e| err422(e))?
        }
    };

    let levels = dca_math::build_dca_plan(&anchor, &base, &safety, &dev, &step_scale, &vol_scale, req.max_safety_orders, direction, &tick, &step, sizing_mode)
        .map_err(|e| err422(e))?;

    // M20 conflict scan (expects base_qty in coin units)
    let (m20_viable, m20_first) = match &req.stop_loss_pct {
        Some(sl) if !sl.is_empty() => {
            let (viable, first, _) = dca_math::m15_m20::m20_conflict_scan(direction, &fr(sl)?, &base_qty, &anchor, &levels).map_err(|e| err422(e))?;
            (viable, first)
        }
        _ => (true, None),
    };

    // WAC trajectory per level (seed with coin-unit base qty, not raw size)
    let mut rows: Vec<Value> = Vec::new();
    let mut q = base_qty.clone();
    let mut cost = base_qty.mul(&anchor).map_err(|e| err422(e))?;
    for l in &levels {
        let lq = l.qty.mul(&l.price).map_err(|e| err422(e))?;
        q = q.add(&l.qty).map_err(|e| err422(e))?;
        cost = cost.add(&lq).map_err(|e| err422(e))?;
        let avg = cost.div(&q).map_err(|e| err422(e))?;
        rows.push(json!({
            "level": l.level_index,
            "price": l.price.to_string(),
            "priceDisplay": l.price.to_half_even_display(8),
            "qty": l.qty.to_string(),
            "notional": l.notional.to_string(),
            "cumDeviation": l.cumulative_deviation.to_string(),
            "avgAfter": avg.to_string(),
            "avgAfterDisplay": avg.to_half_even_display(8),
        }));
    }

    // Plan notional + collateral (M03)
    let planned = cost.sub(&base_qty.mul(&anchor).unwrap_or_else(|_| Fraction::zero())).unwrap_or_else(|_| Fraction::zero());
    let total_notional = cost.clone();
    let is_futures = req.market == "linear";
    let im = if is_futures { total_notional.div(&leverage).unwrap_or_else(|_| total_notional.clone()) } else { total_notional.clone() };

    // Liquidation estimate (M07) at 0.5% MMR for linear
    let mut liq: Option<String> = None;
    if is_futures {
        let brackets = vec![dca_math::MmrBracket {
            min_notional: Fraction::zero(),
            max_notional: Fraction::from_u64(u64::MAX),
            mmr: Fraction::from_decimal_str("0.005").unwrap(),
            deduction: Fraction::zero(),
        }];
        let w = im.clone();
        let wac = cost.div(&q).unwrap_or_else(|_| anchor.clone());
        let pos_side: i8 = if direction == Direction::Long { 1 } else { -1 };
        if let Ok(Some(p)) = dca_math::m03_m07::liquidation_price(pos_side, &q, &wac, &w, &brackets, &Fraction::zero()) {
            liq = Some(p.to_half_even_display(6));
        }
    }

    // Fee breaker for grid? not here. Estimated round-trip fee:
    let fee_rate = match fr(&req.taker_fee) { Ok(x) => x, Err(r) => return Err(r) };
    let est_fee = total_notional.mul(&fee_rate).unwrap_or_else(|_| Fraction::zero());

    let body = json!({
        "levels": rows,
        "totalNotional": total_notional.to_string(),
        "totalNotionalDisplay": total_notional.to_half_even_display(2),
        "totalQty": q.to_string(),
        "avgAllIn": cost.div(&q).map(|a| a.to_string()).unwrap_or_else(|_| "0".into()),
        "estimatedInitialMargin": im.to_string(),
        "estimatedFees": est_fee.to_string(),
        "liquidationPrice": liq,
        "m20": {
            "viable": m20_viable,
            "firstUnreachableLevel": m20_first,
        },
        "plannedSafetyNotional": planned.to_string(),
    });
    Ok(axum::Json(body).into_response())
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub(crate) struct GridCalcReq {
    lower: String,
    upper: String,
    levels: usize,
    grid_mode: String,
    notional_per_level: String,
    tick_size: String,
    step_size: String,
    maker_fee: String,
    taker_fee: String,
}

pub(crate) async fn calc_grid(Json(req): Json<GridCalcReq>) -> Result<axum::response::Response, axum::response::Response> {
    let fr = |s: &str| -> Result<Fraction, axum::response::Response> { Fraction::from_decimal_str(s).map_err(|e| err422(e)) };
    let lower = match fr(&req.lower) { Ok(x) => x, Err(r) => return Err(r) };
    let upper = match fr(&req.upper) { Ok(x) => x, Err(r) => return Err(r) };
    let tick = match fr(&req.tick_size) { Ok(x) => x, Err(r) => return Err(r) };
    let step = match fr(&req.step_size) { Ok(x) => x, Err(r) => return Err(r) };
    let maker = match fr(&req.maker_fee) { Ok(x) => x, Err(r) => return Err(r) };
    let notional = match fr(&req.notional_per_level) { Ok(x) => x, Err(r) => return Err(r) };

    let mesh = match req.grid_mode.as_str() {
        "geometric" => match dca_math::m15_m20::geometric_mesh(&lower, &upper, req.levels) { Ok(m) => m, Err(e) => return Err(err422(e)) },
        _ => match dca_math::m15_m20::arithmetic_mesh(&lower, &upper, req.levels) { Ok(m) => m, Err(e) => return Err(err422(e)) },
    };
    let g_min = match dca_math::m15_m20::fee_breaker_min_step(&maker, &maker) { Ok(g) => g, Err(e) => return Err(err422(e)) };
    let mut rows = Vec::new();
    let mut total_qty = Fraction::zero();
    let mut fee_ok = true;
    for (i, p) in mesh.iter().enumerate() {
        let p_tick = dca_math::m02::tick_down(p, &tick).unwrap_or_else(|_| p.clone());
        if i > 0 {
            let prev = &mesh[i - 1];
            if let Ok(sp) = p_tick.div(prev).and_then(|x| x.sub(&Fraction::one())) {
                if sp <= g_min {
                    fee_ok = false;
                }
            }
        }
        let qty = dca_math::m02::qty_down(&notional.div(&p_tick).unwrap_or_else(|_| Fraction::zero()), &step).unwrap_or_else(|_| Fraction::zero());
        total_qty = total_qty.add(&qty).unwrap_or_else(|_| total_qty.clone());
        rows.push(json!({
            "level": i,
            "price": p_tick.to_string(),
            "priceDisplay": p_tick.to_half_even_display(8),
            "qty": qty.to_string(),
            "notional": p_tick.mul(&qty).map(|n| n.to_string()).unwrap_or_else(|_| "0".into()),
        }));
    }
    let invested = total_qty.mul(&upper).map(|x| x.to_string()).unwrap_or_else(|_| "0".into());
    Ok(axum::Json(json!({
        "mesh": rows,
        "feeBreakerPass": fee_ok,
        "gMin": g_min.to_string(),
        "totalInvested": invested,
    }))
    .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_calc_dca_short_and_wac_liquidation() {
        let req = DcaCalcReq {
            anchor: "100".into(),
            base_order_size: "100".into(),
            safety_order_size: "100".into(),
            sizing_mode: "QUOTE_NOTIONAL".into(),
            deviation_pct: "0.02".into(),
            step_scale: "1.0".into(),
            volume_scale: "1.0".into(),
            max_safety_orders: 2,
            target_profit_pct: "0.02".into(),
            use_net_tp: true,
            stop_loss_pct: Some("0.05".into()),
            tick_size: "0.01".into(),
            step_size: "0.001".into(),
            maker_fee: "0.001".into(),
            taker_fee: "0.001".into(),
            leverage: "10".into(),
            market: "linear".into(),
            direction: Some("short".into()),
        };
        let resp = calc_dca(Json(req)).await.unwrap();
        assert_eq!(resp.status(), axum::http::StatusCode::OK);
    }
}
