//! Uc venue saglik ozeti (v3 `GET /api/live/multi-status` karsiligi - Faz 1.3).
//!
//! v3 bilesen eslemesi (durustluk notlariyla):
//! - venue registry / driver hazirligi -> halka-acik `server_time` sondasi +
//!   kasa anahtar VARLIGI (secret asla; imza yok). Sonda basarisizligi
//!   `connected:false` olur, uc ASLA 500 donmez.
//! - OKX `posMode` assertion -> P4-3d kapisinin salt-okunur karsiligi:
//!   yalniz OKX anahtari varsa sorgulanir, `long_short_mode` degilse TUZAK notu.
//! - reconcile snapshot -> bizde arka-plan rutini YOK (v3 `ensureMultiReconcile`
//!   bizde uydurulmaz); yerine venue basina bot listesi + `supports` kurali
//!   (status.rs ile ayni `reconcile_support`).
//! - sentinel -> bakim sentineli (`maint_quiet` + ayni probe'larla
//!   QUIET/DEGRADED/NORMAL); AI sentinel bizde yok.
//! - kill-switch -> mevcut `kill_json`/`freeze_json`.

use axum::extract::State;
use axum::Json;
use dca_exchange::VenueAdapter;
use serde_json::{json, Value};
use std::sync::Arc;

use super::{maint_flag, venue_probe};
use super::status::reconcile_support;
use crate::state::{now_ms, AppState};

/// OKX posMode hukmu: Some->assertion, None->bilinmiyor (anahtar yok/hata).
pub fn okx_pos_verdict(mode: Option<&str>) -> (&'static str, bool) {
    match mode {
        Some(m) if m == dca_exchange::okx::REQUIRED_POS_MODE => ("uyumlu (long_short_mode)", true),
        Some(m) => (if m.is_empty() { "bos posMode - TUZAK olabilir" } else { "TUZAK: net_mode - vadelide long_short_mode gerekli" }, false),
        None => ("bilinmiyor (anahtar yok veya sorgu basarisiz)", false),
    }
}

pub async fn live_multi_status(State(app): State<Arc<AppState>>) -> Json<Value> {
    // Uc halka-acik sonda paralel (v3 `venueStatus` karsiligi; herbiri kendi
    // hatasini icinde tasir - join! hicbirinde patlamaz).
    let (bybit_probe, binance_probe, okx_probe) = tokio::join!(
        venue_probe("BYBIT", app.bybit.server_time_ms()),
        venue_probe("BINANCE", app.binance.server_time_ms()),
        venue_probe("OKX", app.okx.server_time_ms()),
    );

    // Venue basina bot listesi + reconcile destegi (ag yok).
    let mut venue_bots: std::collections::HashMap<String, Vec<Value>> = std::collections::HashMap::new();
    let mut venue_supported: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    {
        let rts = app.runtimes.lock().await;
        for rt in rts.values() {
            let is_dca = matches!(rt.engine_kind, dca_strategy::EngineTag::Dca(_));
            let (supported, reason) = reconcile_support(&rt.mode, is_dca);
            venue_bots.entry(rt.venue.clone()).or_default().push(json!({
                "id": rt.bot_id,
                "mode": rt.mode,
                "status": rt.status,
                "reconcileSupported": supported,
                "reconcileReason": reason,
            }));
            if supported {
                *venue_supported.entry(rt.venue.clone()).or_default() += 1;
            }
        }
    }

    let has_key = |venue: &str| {
        let environment = app.execution_environment(venue, "LIVE");
        crate::store::load_active_key_for_environment(&app.db, &app.vault, venue, environment)
            .ok()
            .flatten()
            .is_some()
    };
    let bybit_key = has_key("BYBIT");
    let binance_key = has_key("BINANCE");
    let okx_key = has_key("OKX");

    // OKX posMode: yalniz anahtar varsa, zaman asimli (10sn), hatada None.
    let okx_pos: Option<String> = if okx_key {
        let environment = app.execution_environment("OKX", "LIVE");
        match crate::store::load_active_key_for_environment(&app.db, &app.vault, "OKX", environment) {
            Ok(Some(creds)) => tokio::time::timeout(
                std::time::Duration::from_secs(10),
                app.okx.pos_mode(&creds),
            )
            .await
            .ok()
            .and_then(|r| r.ok())
            .flatten(),
            _ => None,
        }
    } else {
        None
    };
    let (okx_pos_note, okx_pos_ok) = okx_pos_verdict(okx_pos.as_deref());

    let venue_json = |name: &str, probe: Value, key: bool, extra: Value| {
        let bots = venue_bots.get(name).cloned().unwrap_or_default();
        json!({
            "venue": name,
            "probe": probe,
            "hasKey": key,
            "keyless": name == "PAPER",
            "bots": bots,
            "reconcileSupported": venue_supported.get(name).copied().unwrap_or(0),
            "detail": extra,
        })
    };
    let paper = json!({"venue": "PAPER", "ok": true, "local": true, "ms": 0});

    let probes_ok = [&bybit_probe, &binance_probe, &okx_probe]
        .iter()
        .all(|p| p.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
    let (quiet, quiet_note) = maint_flag(&app);
    let mode = if quiet { "QUIET" } else if !probes_ok { "DEGRADED" } else { "NORMAL" };

    Json(json!({
        "ok": true,
        "generatedAt": now_ms(),
        "venues": {
            "BYBIT": venue_json("BYBIT", bybit_probe, bybit_key, json!({"gate": "vault anahtari + motor place_order kapilari"})),
            "BINANCE": venue_json("BINANCE", binance_probe, binance_key, json!({"gate": "vault anahtari + motor place_order kapilari"})),
            "OKX": venue_json("OKX", okx_probe, okx_key, json!({
                "gate": "posMode === 'long_short_mode'",
                "posMode": okx_pos,
                "posModeOk": okx_pos_ok,
                "posModeNote": okx_pos_note,
            })),
            "PAPER": venue_json("PAPER", paper, false, json!({"note": "yerel simulasyon - borsa erisimi yok"})),
        },
        "maintenance": {"mode": mode, "quiet": quiet, "note": quiet_note},
        "killSwitch": app.kill_json().await,
        "freeze": app.freeze_json().await,
        "note": "Faz 1.3: arka-plan reconcile rutini bizde yok (uydurulmadi); reconcile destegi kural bayragidir.",
    }))
}

#[cfg(test)]
mod multistatus_tests {
    use super::*;

    #[test]
    fn support_matrix_matches_reconcile_handler() {
        assert_eq!(reconcile_support("LIVE", true), (true, "destekleniyor"));
        assert_eq!(reconcile_support("PAPER", true).0, false);
        assert_eq!(reconcile_support("LIVE", false).0, false);
        assert_eq!(reconcile_support("PAPER", false).0, false);
    }

    #[test]
    fn okx_pos_verdict_matrix() {
        assert_eq!(okx_pos_verdict(Some("long_short_mode")), ("uyumlu (long_short_mode)", true));
        assert!(!okx_pos_verdict(Some("net_mode")).1);
        assert!(okx_pos_verdict(Some("net_mode")).0.contains("TUZAK"));
        assert!(!okx_pos_verdict(None).1);
        assert!(!okx_pos_verdict(Some("")).1);
    }
}
