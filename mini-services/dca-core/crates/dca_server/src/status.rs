//! Canli katman durumu (v3 `GET /api/live/status` karsiligi - Faz 1.2).
//!
//! Ince toplama ucu: `summary` + kill/freeze + son selfcheck + reconcile
//! destegi + certGate sayaci + bot listesi tek yanitta. Yeni matematik yok.
//!
//! TAsINMAYANLAR (yol haritasi 1.2 hukmu): v6Shield alt telemetrileri
//! (rate-limiter durumu, orderbook-ensure, bracket, bnbSentinel, telegram) -
//! bizde bu alt sistemler yok; "dolu gorunmek icin" uydurma telemetri
//! tasinmaz, uc `telemetry: null` doner.
//!
//! ASLA 500 DEGIL: v3 hata halinde 500 doner; burada her bacak ayri
//! korunur - eksik bacak `null`/gerekceli not olur, yanit hep 200.

use axum::extract::State;
use axum::Json;
use serde_json::{json, Value};
use std::sync::Arc;

use super::summary;
use crate::state::AppState;

pub const SELF_CHECK_MAX_AGE_MS: i64 = 15 * 60 * 1_000;

/// Self-check results are operational evidence, not a permanent approval.
/// Future timestamps are invalid as well as stale records.
pub fn selfcheck_is_fresh(at_ms: i64, now_ms: i64) -> bool {
    at_ms > 0 && at_ms <= now_ms && now_ms.saturating_sub(at_ms) <= SELF_CHECK_MAX_AGE_MS
}

/// `verdict=GECTI passed=19 failed=0 abstained=0` bicimindeki SELF_CHECK
/// olay mesajini cozer (selfcheck handler'inin persist formati).
pub fn parse_selfcheck_message(msg: &str) -> Option<(String, u32, u32, u32)> {
    let mut verdict: Option<String> = None;
    let mut passed: Option<u32> = None;
    let mut failed: Option<u32> = None;
    let mut abstained: Option<u32> = None;
    for part in msg.split_whitespace() {
        let (k, v) = part.split_once('=')?;
        match k {
            "verdict" => verdict = Some(v.to_string()),
            "passed" => passed = v.parse().ok(),
            "failed" => failed = v.parse().ok(),
            "abstained" => abstained = v.parse().ok(),
            _ => {}
        }
    }
    Some((verdict?, passed?, failed?, abstained?))
}

/// Rampa karari (v3 §11.3 sira-tabanli, freeze dali bize ozgu):
/// selfcheck GECTI degilse T0-hazirlik; kill devredeyse T0-kill;
/// freeze devredeyse T0-freeze; hepsi temizse T1.
pub fn ramp_stage(
    selfcheck_ok: bool,
    kill: bool,
    freeze: bool,
) -> (&'static str, &'static str, &'static str) {
    if !selfcheck_ok {
        (
            "T0",
            "T0 - Hazirlik",
            "Selfcheck PASS (anahtarlar Ayarlar > Kasa'da)",
        )
    } else if kill {
        (
            "T0",
            "T0 - Kill-switch devrede",
            "Kill-switch birak + selfcheck yenile",
        )
    } else if freeze {
        ("T0", "T0 - Freeze devrede", "Freeze coz + hatti gozle")
    } else {
        (
            "T1",
            "T1 - Canli hat acik (Faz 0 kapilari devrede)",
            "Dry-run ile prova; mutabakat ile izle",
        )
    }
}

/// Reconcile destegi kurali (reconcile handler'inin `supported` mantigiyla ayni:
/// LIVE + DCA; ag cagrisi yok). status + multi-status ortak kullanir.
pub fn reconcile_support(mode: &str, is_dca: bool) -> (bool, &'static str) {
    if mode != "LIVE" {
        (false, "PAPER modda borsa pozisyonu yok")
    } else if !is_dca {
        (false, "v1 yalniz DCA motorlarinda")
    } else {
        (true, "destekleniyor")
    }
}

fn latest_selfcheck(app: &AppState) -> Option<Value> {
    let (level, message, ts): (String, String, i64) = (|| {
        let conn = app.db.conn.lock().ok()?;
        let mut stmt = conn
            .prepare(
                "SELECT level, message, ts FROM events WHERE bot_id='SYSTEM' AND gate='SELF_CHECK' ORDER BY rowid DESC LIMIT 1",
            )
            .ok()?;
        let mut rows = stmt.query([]).ok()?;
        let row = rows.next().ok()??;
        Some((row.get(0).ok()?, row.get(1).ok()?, row.get(2).ok()?))
    })()?;
    let (verdict, passed, failed, abstained) = parse_selfcheck_message(&message)?;
    let now = crate::state::now_ms();
    Some(json!({
        "verdict": verdict,
        "passed": passed,
        "failed": failed,
        "abstained": abstained,
        "level": level,
        "at": ts,
        "ageMs": now.saturating_sub(ts),
        "fresh": selfcheck_is_fresh(ts, now),
    }))
}

pub async fn live_status(State(app): State<Arc<AppState>>) -> Json<Value> {
    // Env envanteri: salt-okunur anahtar VARLIGI (parmak izi degil, sir degil -
    // asla secret tasmaZ; ag cagrisi yok, DB kilidi kisa).
    let mut venues = Vec::new();
    for v in ["BYBIT", "BINANCE", "OKX", "PAPER"] {
        let has_key = if v == "PAPER" {
            false
        } else {
            crate::store::load_active_key(&app.db, &app.vault, v)
                .ok()
                .flatten()
                .is_some()
        };
        venues.push(json!({"venue": v, "hasKey": has_key, "keyless": v == "PAPER"}));
    }
    let kill = app.kill_json().await;
    let freeze = app.freeze_json().await;
    let kill_on = kill
        .get("engaged")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let freeze_on = freeze
        .get("frozen")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);

    let selfcheck = latest_selfcheck(&app);
    let selfcheck_ok = selfcheck.as_ref().is_some_and(|s| {
        s.get("verdict").and_then(|v| v.as_str()) == Some("GECTI")
            && s.get("failed").and_then(|v| v.as_u64()) == Some(0)
            && s.get("fresh").and_then(|v| v.as_bool()) == Some(true)
    });
    let (stage, label, next_requirement) = ramp_stage(selfcheck_ok, kill_on, freeze_on);

    // Reconcile destegi: ag cagrisi YOK - yalniz kural bayragi (reconcile
    // handler'inin `supported` mantigiyla ayni: LIVE + DCA).
    let mut supports = Vec::new();
    let mut bots = Vec::new();
    let mut live_bots = 0u32;
    let mut running_live = 0u32;
    {
        let rts = app.runtimes.lock().await;
        for rt in rts.values() {
            let is_dca = matches!(rt.engine_kind, dca_strategy::EngineTag::Dca(_));
            let (supported, reason) = reconcile_support(&rt.mode, is_dca);
            supports.push(json!({"id": rt.bot_id, "supported": supported, "reason": reason}));
            if rt.mode == "LIVE" {
                live_bots += 1;
                if rt.status == "RUNNING" {
                    running_live += 1;
                }
            }
            bots.push(json!({
                "id": rt.bot_id,
                "name": rt.name,
                "symbol": rt.cfg.symbol,
                "mode": rt.mode,
                "venue": rt.venue,
                "status": rt.status,
            }));
        }
    }

    let summ = summary(State(app.clone())).await.0;
    Json(json!({
        "env": {"venues": venues},
        "killSwitch": kill,
        "freeze": freeze,
        "selfcheck": selfcheck,
        "ramp": {"stage": stage, "label": label, "nextRequirement": next_requirement},
        "reconcile": {"supports": supports},
        "certGate": {
            "liveBots": live_bots,
            "runningLive": running_live,
            "rule": "LIVE botlar sertifika geçmeden başlatılamaz; anahtar, self-check ve endpoint kontrolleri her başlatmada yenilenir",
        },
        "bots": bots,
        "summary": summ,
        "telemetry": null,
        "note": "Ince toplama ucu (Faz 1.2): v6Shield telemetrisi bizde yok, uydurulmadi (telemetry:null). Bu uc asla 500 donmez.",
    }))
}

#[cfg(test)]
mod status_tests {
    use super::*;

    #[test]
    fn selfcheck_message_parses_verdict_counts() {
        let (v, p, f, a) =
            parse_selfcheck_message("verdict=GECTI passed=19 failed=0 abstained=3").unwrap();
        assert_eq!((v.as_str(), p, f, a), ("GECTI", 19, 0, 3));
    }

    #[test]
    fn selfcheck_message_rejects_garbage() {
        assert!(parse_selfcheck_message("bozuk").is_none());
        assert!(parse_selfcheck_message("verdict=GECTI passed=x failed=0 abstained=0").is_none());
    }

    #[test]
    fn ramp_no_selfcheck_is_t0_prep() {
        assert_eq!(ramp_stage(false, false, false).0, "T0");
    }

    #[test]
    fn ramp_kaldi_selfcheck_is_t0_prep() {
        // selfcheck_ok=false -> KALDI da hazirlikta kalir.
        let (stage, _, next) = ramp_stage(false, true, true);
        assert_eq!(stage, "T0");
        assert!(next.contains("Selfcheck"));
    }

    #[test]
    fn ramp_kill_overrides_gecti() {
        let (stage, label, _) = ramp_stage(true, true, false);
        assert_eq!((stage, label), ("T0", "T0 - Kill-switch devrede"));
    }

    #[test]
    fn ramp_freeze_overrides_gecti() {
        let (stage, label, _) = ramp_stage(true, false, true);
        assert_eq!((stage, label), ("T0", "T0 - Freeze devrede"));
    }

    #[test]
    fn selfcheck_freshness_rejects_old_future_and_missing_timestamps() {
        let now = 2_000_000;
        assert!(selfcheck_is_fresh(now - SELF_CHECK_MAX_AGE_MS, now));
        assert!(!selfcheck_is_fresh(now - SELF_CHECK_MAX_AGE_MS - 1, now));
        assert!(!selfcheck_is_fresh(now + 1, now));
        assert!(!selfcheck_is_fresh(0, now));
    }

    #[test]
    fn ramp_clean_is_t1() {
        let (stage, _, _) = ramp_stage(true, false, false);
        assert_eq!(stage, "T1");
    }
}
