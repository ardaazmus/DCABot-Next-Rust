//! Faz 1.5 ince uclar: market ping + makro durum + bakim simulasyonu.
//!
//! v3 eslemesi:
//! - `GET /api/market/ping` (v8 WS1: Binance testnet ping -> {ok, rttMs, at}).
//!   Bizde venue parametreli (`?venue=BINANCE|BYBIT|OKX|PAPER`, varsayilan
//!   BINANCE): adaptorun kendi base'ine `server_time` sondasi, 4sn timeout
//!   (v3 aynen). Hangi base'e vuruldugu `base` alaninda aciklanir (testnet mi
//!   mainnet mi surpriz degil); PAPER yereldir (saglik gostergesi degil).
//! - `GET /api/macro/state` (v3: statik takvim + ag YOK, sir YOK). Ayni ic
//!   takvim (`macro_calendar.rs`); v3'teki `tier` alani bizde yok - uydurulmadi.
//! - `POST /api/system/maintenance/simulate` (v3: 4 asamali AI sentinel
//!   makinesi). Bizde o makine YOK (uydurulmadi); karsilik: mevcut QUIET /
//!   DEGRADED / NORMAL modunun bellek-ici gecici ust-yazimi (restart'ta
//!   silinir, `settings` tablosuna yazilmaz, otomatik durdurma yapmaz - QUIET
//!   zaten bilgilendiricidir). Yalniz dev/testnet kurali: `DCA_ALLOW_SIMULATE=1`
//!   yoksa 403 (fail-closed).
//!
//! Uc ucta ASLA 500 donmez.

use axum::extract::{Query, State};
use axum::Json;
use dca_exchange::VenueAdapter;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::state::{now_ms, AppState};

const PING_TIMEOUT: Duration = Duration::from_secs(4);

fn macro_tr(label: &str) -> &str {
    match label {
        "FOMC" => "FOMC faiz karari",
        "CPI" => "TUFE (CPI)",
        _ => label,
    }
}

// ---------------- GET /api/market/ping ----------------

#[derive(Deserialize)]
pub struct PingQuery {
    pub venue: Option<String>,
}

pub async fn market_ping(State(app): State<Arc<AppState>>, Query(q): Query<PingQuery>) -> Json<Value> {
    let venue = q.venue.map(|v| v.to_uppercase()).unwrap_or_else(|| "BINANCE".into());
    let base = match venue.as_str() {
        "BYBIT" => app.bybit.base_url.clone(),
        "OKX" => app.okx.base_url.clone(),
        "PAPER" => "local".into(),
        "BINANCE" => app.binance.base_url.clone(),
        _ => String::new(),
    };
    if base.is_empty() {
        return Json(json!({
            "ok": false,
            "error": "bilinmeyen venue (gecerli: BINANCE | BYBIT | OKX | PAPER)",
            "venue": venue,
            "at": now_ms(),
        }));
    }
    let t = Instant::now();
    let rtt = || t.elapsed().as_millis() as u64;
    let res = match venue.as_str() {
        "BYBIT" => tokio::time::timeout(PING_TIMEOUT, app.bybit.server_time_ms()).await,
        "OKX" => tokio::time::timeout(PING_TIMEOUT, app.okx.server_time_ms()).await,
        "PAPER" => tokio::time::timeout(PING_TIMEOUT, app.paper.server_time_ms()).await,
        _ => tokio::time::timeout(PING_TIMEOUT, app.binance.server_time_ms()).await,
    };
    match res {
        Ok(Ok(ts)) if ts > 0 => Json(json!({"ok": true, "rttMs": rtt(), "at": now_ms(), "venue": venue, "base": base})),
        Ok(Ok(_)) => Json(json!({"ok": false, "rttMs": rtt(), "at": now_ms(), "venue": venue, "base": base, "error": "sifir zaman"})),
        Ok(Err(e)) => Json(json!({"ok": false, "rttMs": rtt(), "at": now_ms(), "venue": venue, "base": base, "error": e.chars().take(120).collect::<String>()})),
        Err(_) => Json(json!({"ok": false, "rttMs": rtt(), "at": now_ms(), "venue": venue, "base": base, "error": "zaman asimi (4sn)"})),
    }
}

// ---------------- GET /api/macro/state ----------------

pub async fn macro_state() -> Json<Value> {
    let now_dt = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms()).unwrap_or_else(chrono::Utc::now);
    let active = dca_math::macro_calendar::in_macro_blackout(now_dt);
    let next = dca_math::macro_calendar::event_times().into_iter().find(|(t, _)| *t >= now_dt);
    fn ev(t: chrono::DateTime<chrono::Utc>, label: &str) -> Value {
        json!({"name": label, "labelTr": macro_tr(label), "t": t.to_rfc3339()})
    }
    // Aktif pencerenin merkezi = bitis - 15dk (kara pencere yari-genisligi).
    let active_ev = active.map(|(label, end)| {
        let center = end - dca_math::macro_calendar::BLACKOUT_HALF_WINDOW;
        (ev(center, label), end.to_rfc3339())
    });
    Json(json!({
        "active": active_ev.is_some(),
        "activeEvent": active_ev.as_ref().map(|(e, _)| e).cloned().unwrap_or(Value::Null),
        "nextEvent": next.map(|(t, label)| ev(t, label)).unwrap_or(Value::Null),
        "windowEndsAt": active_ev.map(|(_, end)| Value::String(end)).unwrap_or(Value::Null),
        "at": now_ms(),
        "note": "statik FOMC/CPI takvimi (ag yok); v3'teki tier alani bizde yok.",
    }))
}

// ---------------- POST /api/system/maintenance/simulate ----------------

/// Simulate izni: yalniz acik dev/test bayragi (v3 makinesi bizde yok;
/// gercek hatta dokunmayan bellek-ici ust-yazim bile kapili olmalidir).
pub fn simulate_allowed() -> bool {
    std::env::var("DCA_ALLOW_SIMULATE").as_deref() == Ok("1")
}

/// Gecerli sim modlari (v3 dortlusu degil; bizim uc modumuz + reset).
pub fn valid_sim_mode(mode: &str) -> bool {
    matches!(mode, "QUIET" | "DEGRADED" | "NORMAL" | "reset")
}

/// Sure kisiti (v3 aynen: 30..3600sn, varsayilan 600sn).
pub fn clamp_duration(sec: i64) -> i64 {
    sec.clamp(30, 3600)
}

/// Ust-yazim gecerli mi (suresi dolmamis)?
pub fn sim_active(entry: &Option<(String, i64)>, now: i64) -> Option<String> {
    match entry {
        Some((mode, exp)) if *exp > now => Some(mode.clone()),
        _ => None,
    }
}

pub async fn simulate_maintenance(
    State(app): State<Arc<AppState>>,
    body: axum::extract::Json<Value>,
) -> Result<Json<Value>, axum::response::Response> {
    if !simulate_allowed() {
        return Err(crate::err403("simulate kapali (yalniz dev/test: DCA_ALLOW_SIMULATE=1)"));
    }
    let mode = body.get("mode").and_then(|m| m.as_str()).unwrap_or("").to_string();
    if !valid_sim_mode(&mode) {
        return Err(crate::err422(format!(
            "bilinmeyen mod \"{mode}\" - gecerli: reset | QUIET | DEGRADED | NORMAL"
        )));
    }
    if mode == "reset" {
        *app.maint_sim.lock().await = None;
        crate::state::persist_event(&app, "SYSTEM", "INFO", "MAINTENANCE", "simulate=reset").await;
        return Ok(Json(json!({"ok": true, "action": "reset", "note": "simulasyon kaldirildi (hesaplanan moda donuldu)"})));
    }
    let secs = body.get("durationSec").and_then(|v| v.as_i64()).unwrap_or(600);
    let secs = clamp_duration(secs);
    let exp = now_ms() + secs * 1000;
    *app.maint_sim.lock().await = Some((mode.clone(), exp));
    crate::state::persist_event(
        &app,
        "SYSTEM",
        "WARN",
        "MAINTENANCE",
        &format!("simulate={mode} {secs}sn (bellek-ici, gercek hatta dokunmaz)"),
    )
    .await;
    Ok(Json(json!({
        "ok": true,
        "action": "force",
        "mode": mode,
        "expiresAt": exp,
        "note": "bellek-ici ust-yazim; restart/sure-bitimi/reset ile kalkar; settings tablosuna yazilmaz",
    })))
}

#[cfg(test)]
mod thin_tests {
    use super::*;

    #[test]
    fn sim_modes_match_our_three_modes() {
        assert!(valid_sim_mode("QUIET"));
        assert!(valid_sim_mode("DEGRADED"));
        assert!(valid_sim_mode("NORMAL"));
        assert!(valid_sim_mode("reset"));
        assert!(!valid_sim_mode("PRE_MAINTENANCE_FREEZE")); // v3 makinesi bizde yok
        assert!(!valid_sim_mode(""));
    }

    #[test]
    fn duration_clamp_matches_v3() {
        assert_eq!(clamp_duration(600), 600);
        assert_eq!(clamp_duration(5), 30);
        assert_eq!(clamp_duration(99999), 3600);
    }

    #[test]
    fn sim_expiry_is_fail_closed() {
        assert_eq!(sim_active(&Some(("QUIET".into(), 2000)), 1000), Some("QUIET".into()));
        assert_eq!(sim_active(&Some(("QUIET".into(), 1000)), 2000), None); // suresi dolmus
        assert_eq!(sim_active(&None, 1000), None);
    }
}
