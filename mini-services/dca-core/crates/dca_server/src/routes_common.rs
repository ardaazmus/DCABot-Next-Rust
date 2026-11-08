//! Faz E/G1 — paylaşılan saf yardımcılar (main.rs'ten taşındı, davranış aynı).

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dca_strategy::StrategyKind;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::state::AppState;

pub fn parse_kind(s: &str) -> Result<StrategyKind, String> {
    match s.to_uppercase().as_str() {
        "DCA_LONG" => Ok(StrategyKind::DcaLong),
        "DCA_SHORT" => Ok(StrategyKind::DcaShort),
        "FUTURES_DCA" => Ok(StrategyKind::FuturesDca),
        "SPOT_GRID" => Ok(StrategyKind::SpotGrid),
        "FUTURES_GRID" => Ok(StrategyKind::FuturesGrid),
        "INFINITY_GRID" => Ok(StrategyKind::InfinityGrid),
        "HEDGE_PROTECTIVE" => Ok(StrategyKind::HedgeProtective),
        "HEDGE_DUAL" => Ok(StrategyKind::HedgeDual),
        _ => Err(format!("unknown kind: {s}")),
    }
}

pub fn err403(msg: impl std::fmt::Display) -> axum::response::Response {
    (StatusCode::FORBIDDEN, Json(json!({"error": msg.to_string()}))).into_response()
}

pub fn err422(msg: impl std::fmt::Display) -> axum::response::Response {
    (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({"error": msg.to_string()}))).into_response()
}

pub fn err404(msg: impl std::fmt::Display) -> axum::response::Response {
    (StatusCode::NOT_FOUND, Json(json!({"error": msg.to_string()}))).into_response()
}
/// Task 21 — backtest fill'lerini grafik işaretçilerine çevir (fiyat grafiği
/// üzerinde giriş/çıkış okları). Sınır: ilk 400 fill (yanıt şişmesin).
/// Fiyat 8 haneli half-even display (UI Number() dönüşümü için düz ondalık).
pub fn fill_marks_json(fills: &[dca_kernel::FillRecord]) -> Vec<Value> {
    fills
        .iter()
        .take(400)
        .map(|f| {
            json!({
                "t": f.ts,
                "price": f.price.to_half_even_display(8),
                "side": format!("{:?}", f.side),
                "role": f.role,
            })
        })
        .collect()
}
pub fn normalize_pair(raw: &str) -> String {
    raw.trim().to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

pub fn is_blacklisted(app: &Arc<AppState>, pair: &str) -> bool {
    app.db.conn.lock().map(|c| {
        c.query_row("SELECT 1 FROM pair_blacklist WHERE pair=?1", [&pair], |_| Ok(())).is_ok()
    }).unwrap_or(false)
}
