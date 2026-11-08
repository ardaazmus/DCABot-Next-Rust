//! Faz 2.3 bot hafızası (v3 `/api/bots/[id]/memory` + `lib/engine/memory`
//! karşılığı — ŞARTLI port).
//!
//! Sözleşme (v3 §7.11 + ABSTAIN, aynen):
//!  1) Model YALNIZCA verdiğimiz JSON bağlamı görür (uydurma system
//!     prompt'ta yasak); ham bağlam `context` sütununa aynen yazılır.
//!  2) LLM hatası/boş yanıt → verdict=ABSTAIN, content ÜRETİLMEZ.
//!  3) Her koşu `bot_memories` satırı + append-only MEMORY olayı bırakır.
//!  4) Sayısal muhasebe LLM'de değil çekirdekte kalır (LLM karar otoritesi
//!     DEĞİL, özetleyicidir).
//!
//! ŞARTLILIK NOTLARI (yol haritası 2.3 hükmü):
//! - Salt-okunur: bu modül yürütmeye dokunmaz (place/cancel yok).
//! - AI anahtarı gövdede yaşar, diske yazılmaz (mevcut `ai/diagnose`
//!   konvansiyonu); anahtar yoksa/çağrı patlarsa HTTP 200 + `ok:false`
//!   + gerekçeli ABSTAIN (422 DEĞİL — diagnose ile aynı kural).
//! - Veri yetersizse (bot yok → 404; bağlam kurulamazsa → ERROR) content null.
//! - v3'teki Faz-14-P2 otomatik hafıza (deal-kapanış tetikleme, 10dk throttle)
//!   TAŞINMADI (motor hattına LLM yan-etkisi sokulmaz — bilinçli; not düşüldü).
//! - Kapanan-deal yüzdeleri (avgNet/lastNet) bizde YOK: deals kapanış defteri
//!   yazılmıyor → null + not (uydurulmadı). Sertifika saklı değil (motor-içi
//!   kapı) → null + not. Rejim dağılımı serisi tutulmaz → yalnız güncel etiket.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::state::{now_ms, AppState};

/// Bağlam yetersizliği eşiği: model yanıtı bu boydan kısaysa ABSTAIN.
pub const MIN_CONTENT_LEN: usize = 40;

/// `?limit=` kıskacı (v3: 1..50, varsayılan 20).
pub fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(20).clamp(1, 50)
}

/// Ham model yanıtından hüküm (saf — birim testli).
pub fn verdict_for_len(chars: usize) -> &'static str {
    if chars >= MIN_CONTENT_LEN {
        "OK"
    } else {
        "ABSTAIN"
    }
}

fn memory_prompt(ctx: &Value) -> (String, String) {
    let system = [
        "Sen bir alım-satım botu yöneticisisin. Görevin: verilen JSON bağlamına dayanarak botun durumunu Türkçe özetlemek.",
        "KURALLAR (mutlak):",
        "1) YALNIZCA bağlamdaki verileri kullan — bağlamda olmayan hiçbir sayıyı, tarihi ya da olayı UYDURMA.",
        "2) Veri yoksa \"veri yok\" yaz (ör. kapanan deal yoksa bunu dürüstçe söyle).",
        "3) Tavsiye değil DURUM özeti yaz: bu bot şu anda ne yapıyor, geçmişi nasıl, riskler ne?",
        "4) Biçim: \"DURUM\", \"GEÇMİŞ\", \"RİSK VE NOTLAR\" başlıklı 3 kısa bölüm; her bölümde 1-3 madde; toplamda en fazla ~180 kelime.",
        "5) Finansal değerleri olduğu gibi aktar; kendi yorum getirip yüzdelik uydurma.",
        "6) Kestirim/hoş iyimserlik yasak: zarardaysa zararda de.",
    ]
    .join("\n");
    let user = format!("Bot bağlamı (tek kaynak):\n{}\n\nYukarıdaki verilerle 3 bölümlük durum özeti üret.", ctx);
    (system, user)
}

/// Botun ÖLÇÜLEN gerçekliğini topla (model yalnız bunu görür).
async fn collect_context(app: &AppState, bot_id: &str) -> Result<Value, String> {
    let (name, kind, venue, mode, symbol, status, start_cash, interval): (String, String, String, String, String, String, String, Option<String>) = {
        let conn = app.db.conn.lock().map_err(|_| "kasa kilitli".to_string())?;
        conn.query_row(
            "SELECT name, kind, venue, mode, symbol, status, start_cash, interval FROM bots WHERE id=?1",
            [bot_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
        )
        .map_err(|_| "BOT_BULUNAMADI".to_string())?
    };
    // Canlı snapshot (bellekteyse; yoksa null dallar — STOPPED bot dürüstçe eksik).
    let snap = app.runtimes.lock().await;
    let rt = snap.get(bot_id);
    let (open, closed_stats, regime, last_price) = match rt {
        Some(r) => {
            let s = r.engine_kind.snapshot();
            let is_short = matches!(r.cfg.kind, dca_strategy::StrategyKind::DcaShort);
            let mark = r.stepper.last_close.as_deref().and_then(|m| m.parse::<f64>().ok()).filter(|m| *m > 0.0);
            let _qty = s.position_qty.split('/').next().and_then(|q| q.trim().parse::<f64>().ok()).unwrap_or(0.0);
            let avg = s.avg_price.as_deref().and_then(|a| a.parse::<f64>().ok()).filter(|a| *a > 0.0);
            let upct = match (mark, avg) {
                (Some(m), Some(a)) => Some(((m - a) / a) * if is_short { -100.0 } else { 100.0 }),
                _ => None,
            };
            let liq = crate::engine::current_liq_price(r).map(|p| p.to_string());
            let open = if s.deal_id.is_some() {
                json!({
                    "dealId": s.deal_id,
                    "side": if is_short { "SHORT" } else { "LONG" },
                    "avgPrice": s.avg_price,
                    "qty": s.position_qty,
                    "tpPrice": s.tp_price,
                    "slPrice": s.sl_price,
                    "liqPrice": liq,
                    "safetyFilled": s.safety_triggered,
                    "maxSafety": r.cfg.max_safety_orders,
                    "trailArmed": s.trailing_state.is_some(),
                    "unrealizedPct": upct.map(|u| (u * 100.0).round() / 100.0),
                })
            } else {
                Value::Null
            };
            let closed = s.deals_closed as u32;
            let win_rate = if closed > 0 { Some((s.wins as f64) * 100.0 / (closed as f64)) } else { None };
            let stats = json!({
                "wins": s.wins,
                "losses": s.losses,
                "winRatePct": win_rate.map(|w| (w * 10.0).round() / 10.0),
                "avgNetPct": Value::Null,
                "totalFee": r.stepper.cumulative_fees.to_string(),
                "totalFunding": r.stepper.cumulative_funding.to_string(),
                "lastNetPct": Value::Null,
            });
            (open, (1, stats, closed), r.last_regime.label().to_string(), mark.map(|m| m.to_string()))
        }
        None => (Value::Null, (0, json!({"wins": 0, "losses": 0, "winRatePct": Value::Null, "avgNetPct": Value::Null, "totalFee": "0", "totalFunding": "0", "lastNetPct": Value::Null}), 0), "bilinmiyor (bellek-dışı)".to_string(), None),
    };
    let (open_count, closed_stats, closed_count) = (if open.is_null() { 0 } else { 1 }, closed_stats.1, closed_stats.2);
    // Parite: son kontrol + koşu sayacı (yoksa null — YETERSİZ_VERİ değil, veri yok).
    let (parity_latest, parity_count): (Value, i64) = (|| {
        let conn = app.db.conn.lock().ok()?;
        let count: i64 = conn.query_row("SELECT COUNT(*) FROM parity_checks WHERE bot_id=?1", [bot_id], |r| r.get(0)).unwrap_or(0);
        let latest: Option<(String, String)> = conn
            .query_row(
                "SELECT verdict, count_match FROM parity_checks WHERE bot_id=?1 ORDER BY id DESC LIMIT 1",
                [bot_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .ok();
        let v = match latest {
            Some((verdict, score_s)) => {
                let score: Option<f64> = score_s.parse().ok();
                json!({"latestScore": score, "latestVerdict": verdict, "checksRun": count})
            }
            None => Value::Null,
        };
        Some((v, count))
    })()
    .unwrap_or((Value::Null, 0));
    let kill = app.kill.lock().await;
    let kill_json = json!({"killSwitchEngaged": kill.active, "killSwitchReason": if kill.active { kill.reason.clone().into() } else { Value::Null }});

    Ok(json!({
        "bot": {"name": name, "strategy": kind, "pair": symbol, "status": status, "mode": mode, "venue": venue,
                "startingEquity": start_cash, "interval": interval},
        "deals": {"openCount": open_count, "closedCount": closed_count, "openPosition": open, "closedStats": closed_stats},
        "parity": parity_latest,
        "parityChecksRun": parity_count,
        "cert": Value::Null,
        "regime": {"current": regime, "rvQuantile": Value::Null, "distribution": Value::Null},
        "market": {"lastPrice": last_price},
        "risk": kill_json,
        "meta": {"generatedAt": now_ms(), "contextNote": "Bu JSON botun ölçülen gerçekliğidir; model yalnız bu veriyle sınırlıdır. Yok olan alan \"veri yok\" demektir. Kapanan-deal yüzdeleri deals kapanış defteri yazılmadığı için null; sertifika motor-içi kapı olduğu için saklı değil; rejim dağılımı serisi tutulmaz."},
    }))
}

#[derive(Deserialize)]
pub struct MemoryQuery {
    pub limit: Option<usize>,
}

pub async fn list_memory(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Query(q): Query<MemoryQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let bot_exists: bool = app
        .db
        .conn
        .lock()
        .ok()
        .and_then(|c| c.query_row("SELECT 1 FROM bots WHERE id=?1", [&id], |_| Ok(())).ok())
        .is_some();
    if !bot_exists {
        return Err(crate::err404("BOT_BULUNAMADI"));
    }
    let limit = clamp_limit(q.limit);
    let rows: Vec<Value> = (|| {
        let conn = app.db.conn.lock().ok()?;
        let mut stmt = conn
            .prepare("SELECT id, verdict, content, model, note, elapsed_ms, created_at, context FROM bot_memories WHERE bot_id=?1 ORDER BY id DESC LIMIT ?2")
            .ok()?;
        let mapped = stmt
            .query_map(rusqlite::params![&id, limit as i64], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, Option<String>>(2)?, r.get::<_, String>(3)?,
                    r.get::<_, Option<String>>(4)?, r.get::<_, i64>(5)?, r.get::<_, i64>(6)?, r.get::<_, String>(7)?))
            })
            .ok()?;
        let mut out = Vec::new();
        for row in mapped.filter_map(|r| r.ok()) {
            let (rid, verdict, content, model, note, elapsed, created, ctx_s) = row;
            let ctx_v: Value = serde_json::from_str(&ctx_s).unwrap_or(Value::Null);
            out.push(json!({"id": rid, "verdict": verdict, "content": content, "model": model,
                            "note": note, "elapsedMs": elapsed, "createdAt": created, "context": ctx_v}));
        }
        Some(out)
    })()
    .unwrap_or_default();
    Ok(Json(json!({"botId": id, "memories": rows})))
}

pub async fn generate_memory(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    body: Option<axum::Json<crate::ai::AiCallConfig>>,
) -> Result<Json<Value>, axum::response::Response> {
    let t0 = now_ms();
    let ctx = collect_context(&app, &id).await.map_err(|e| {
        if e == "BOT_BULUNAMADI" {
            crate::err404(e)
        } else {
            crate::err422(e)
        }
    })?;
    let ctx_s = ctx.to_string();
    let cfg = body.map(|b| b.0);
    let rc = crate::ai::resolve_call(cfg.as_ref()).map_err(|e| crate::err422(format!("AI yapılandırması: {e}")))?;
    tracing::info!("[memory] bot={id} provider={} model={}", rc.provider.id, rc.model);
    let (system, user) = memory_prompt(&ctx);
    // LLM çağrısı: her hata ABSTAIN (sahte özet yok); bağlam yine de kaydedilir.
    let (verdict, content, note) = match crate::ai::call_provider(&app, &rc, &system, &user, &[]).await {
        Ok(raw) => {
            let len = raw.chars().count();
            if verdict_for_len(len) == "OK" {
                ("OK", Some(raw), None)
            } else {
                ("ABSTAIN", None, Some(format!("Model yanıtı çok kısa/boş döndü ({len} karakter) — sahte özet üretilmedi (abstention dürüstlüğü). Bağlam yine de kaydedildi.")))
            }
        }
        Err(e) => (
            "ABSTAIN",
            None,
            Some(format!("Model çağrısı başarısız: {} — özet üretilmedi (bağlam kaydedildi; yeniden deneyebilirsiniz).",
                e.chars().take(160).collect::<String>())),
        ),
    };
    let model_s = format!("{} · {}", rc.provider.label, rc.model);
    let elapsed = now_ms() - t0;
    let row_id: i64 = {
        let conn = app.db.conn.lock().map_err(|_| crate::err422("kasa kilitli"))?;
        conn.execute(
            "INSERT INTO bot_memories(bot_id, kind, verdict, content, context, model, note, elapsed_ms, created_at) VALUES(?1,'SUMMARY',?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![&id, verdict, content, ctx_s, model_s, note, elapsed, now_ms()],
        )
        .map_err(|e| crate::err422(format!("hafıza yazılamadı: {e}")))?;
        conn.last_insert_rowid()
    };
    crate::state::persist_event(
        &app,
        &id,
        if verdict == "OK" { "INFO" } else { "WARN" },
        "MEMORY",
        &format!("memoryId={row_id} verdict={verdict} model={model_s} elapsedMs={elapsed} trigger=MANUAL"),
    )
    .await;
    let mem = json!({"id": row_id, "verdict": verdict, "content": content, "model": model_s,
                     "note": note, "elapsedMs": elapsed, "createdAt": now_ms()});
    Ok(Json(json!({"ok": verdict == "OK", "memory": mem})))
}

#[cfg(test)]
mod memory_tests {
    use super::*;

    #[test]
    fn verdict_threshold_is_40_chars() {
        assert_eq!(verdict_for_len(39), "ABSTAIN");
        assert_eq!(verdict_for_len(40), "OK");
        assert_eq!(verdict_for_len(0), "ABSTAIN");
    }

    #[test]
    fn limit_clamp_matches_v3() {
        assert_eq!(clamp_limit(None), 20);
        assert_eq!(clamp_limit(Some(0)), 1);
        assert_eq!(clamp_limit(Some(200)), 50);
    }

    #[test]
    fn prompt_forbids_hallucination() {
        let (sys, user) = memory_prompt(&json!({"bot": {"name": "x"}}));
        assert!(sys.contains("UYDURMA"), "system prompt uydurma yasagi tasimali");
        assert!(user.contains("\"bot\""), "baglam prompta gomulmeli");
        assert!(sys.contains("180 kelime"), "v3 bicim kurali korunmali");
    }
}
