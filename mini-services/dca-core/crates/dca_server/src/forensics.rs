//! Faz 2.4 olay adli tıp — `GET /api/events/forensics` (3 mod).
//!
//! v3 eslemesi (`/api/events/forensics`): traceId / traces / serbest filtre.
//! Bizim `events` tablosu v3 `EventLog`'dan dardir: (id, bot_id, level, gate,
//! message, ts) + Faz 2.4 `trace_id`. Uyarlamalar (uydurma yok):
//! - `type` ≈ gate; `payload` yok (message düz metin); `elapsedMs`/span yok.
//! - `spanCount` daima 0 + `spanNote` (alt-sistem kırılımı bizde üretilmiyor).
//! - traces özeti TICK kök satırından çözülür (`symbol=`/`kind=` anahtarları).
//! - minLatencyMs v3'te olay-bazlıdır; bizde iz-toplamına uygulanır ve yalnız
//!   traces modunda desteklenir (filtre modunda yoksayılır + not düşülür).
//! - Bu uç ASLA 500 dönmez (kilit/DB hatası Json hatasıyla döner).

use axum::extract::{Query, State};
use axum::Json;
use rusqlite::Connection;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::state::{now_ms, AppState};

#[derive(Deserialize, Default)]
pub struct ForensicsQuery {
    pub trace_id: Option<String>,
    pub mode: Option<String>,
    pub bot_id: Option<String>,
    pub error_only: Option<bool>,
    pub min_latency_ms: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct EventRow {
    pub id: i64,
    pub bot_id: String,
    pub level: String,
    pub gate: String,
    pub message: String,
    pub ts: i64,
    pub trace_id: Option<String>,
}

fn row_to_json(r: &EventRow) -> Value {
    json!({
        "id": r.id,
        "botId": r.bot_id,
        "level": r.level,
        "gate": r.gate,
        "message": r.message,
        "ts": r.ts,
        "traceId": r.trace_id,
    })
}

/// TICK kök satırından `symbol=`/`kind=` çözer (`TICK symbol=X kind=Y ...`).
pub fn parse_tick_root(message: &str) -> (Option<String>, Option<String>) {
    let mut symbol = None;
    let mut kind = None;
    for part in message.split_whitespace() {
        if let Some(v) = part.strip_prefix("symbol=") {
            if !v.is_empty() {
                symbol = Some(v.to_string());
            }
        } else if let Some(v) = part.strip_prefix("kind=") {
            if !v.is_empty() {
                kind = Some(v.to_string());
            }
        }
    }
    (symbol, kind)
}

fn read_rows(stmt: &mut rusqlite::Statement, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<EventRow>, String> {
    let rows = stmt
        .query_map(params, |r| {
            Ok(EventRow {
                id: r.get(0)?,
                bot_id: r.get(1)?,
                level: r.get(2)?,
                gate: r.get(3)?,
                message: r.get(4)?,
                ts: r.get(5)?,
                trace_id: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        out.push(r.map_err(|e| e.to_string())?);
    }
    Ok(out)
}

/// Mod 1: tek izin olayları (ts asc) + özet.
pub fn query_trace(conn: &Connection, trace_id: &str) -> Result<(Value, Vec<EventRow>), String> {
    let mut stmt = conn
        .prepare("SELECT id, bot_id, level, gate, message, ts, trace_id FROM events WHERE trace_id=?1 ORDER BY ts ASC, id ASC LIMIT 1000")
        .map_err(|e| e.to_string())?;
    let events = read_rows(&mut stmt, &[&trace_id])?;
    if events.is_empty() {
        return Ok((Value::Null, events));
    }
    let root_ts = events[0].ts;
    let last_ts = events.last().map(|e| e.ts).unwrap_or(root_ts);
    let error_count = events.iter().filter(|e| e.level == "ERROR").count();
    let summary = json!({
        "traceId": trace_id,
        "rootTs": root_ts,
        "totalMs": (last_ts - root_ts).max(0),
        "eventCount": events.len(),
        "errorCount": error_count,
        "spanCount": 0,
        "spanNote": "alt-sistem span kirilimi bizde uretilmiyor (TICK + olaylar ham verilir)",
    });
    Ok((summary, events))
}

#[derive(Debug)]
pub struct TraceSummary {
    pub trace_id: String,
    pub event_count: i64,
    pub error_count: i64,
    pub first_ts: i64,
    pub last_ts: i64,
    pub bot_id: String,
}

/// Mod 2: son izler (gruflu). min_latency_ms iz-toplamına uygulanır.
pub fn query_traces(conn: &Connection, limit: i64, min_latency_ms: Option<i64>) -> Result<Vec<Value>, String> {
    let lim = limit.clamp(1, 50);
    let mut stmt = conn
        .prepare(
            "SELECT trace_id, COUNT(*), SUM(CASE WHEN level='ERROR' THEN 1 ELSE 0 END), MIN(ts), MAX(ts), MIN(bot_id) \
             FROM events WHERE trace_id IS NOT NULL GROUP BY trace_id ORDER BY MAX(ts) DESC LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([lim], |r| {
            Ok(TraceSummary {
                trace_id: r.get(0)?,
                event_count: r.get(1)?,
                error_count: r.get(2)?,
                first_ts: r.get(3)?,
                last_ts: r.get(4)?,
                bot_id: r.get(5)?,
            })
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for r in rows {
        let t = r.map_err(|e| e.to_string())?;
        let total_ms = (t.last_ts - t.first_ts).max(0);
        if let Some(min) = min_latency_ms {
            if total_ms < min {
                continue;
            }
        }
        let (symbol, kind) = root_of(conn, &t.trace_id)?;
        out.push(json!({
            "traceId": t.trace_id,
            "eventCount": t.event_count,
            "errorCount": t.error_count,
            "firstTs": t.first_ts,
            "lastTs": t.last_ts,
            "totalMs": total_ms,
            "botId": t.bot_id,
            "symbol": symbol,
            "kind": kind,
        }));
    }
    Ok(out)
}

fn root_of(conn: &Connection, trace_id: &str) -> Result<(Value, Value), String> {
    let msg: Option<String> = conn
        .query_row(
            "SELECT message FROM events WHERE trace_id=?1 AND gate='TICK' ORDER BY id ASC LIMIT 1",
            [trace_id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())
        .ok();
    match msg {
        Some(m) => {
            let (s, k) = parse_tick_root(&m);
            Ok((s.map(Value::String).unwrap_or(Value::Null), k.map(Value::String).unwrap_or(Value::Null)))
        }
        None => Ok((Value::Null, Value::Null)),
    }
}

/// Mod 3: serbest filtre (bot + errorOnly + limit; minLatency burada yok).
pub fn query_filter(
    conn: &Connection,
    bot_id: Option<&str>,
    error_only: bool,
    limit: i64,
) -> Result<Vec<EventRow>, String> {
    let lim = limit.clamp(1, 1000);
    let (sql, bot): (String, Option<String>) = match bot_id {
        Some(b) if !b.is_empty() => (
            "SELECT id, bot_id, level, gate, message, ts, trace_id FROM events WHERE bot_id=?1 AND (?2=0 OR level='ERROR') ORDER BY id DESC LIMIT ?3".into(),
            Some(b.to_string()),
        ),
        _ => (
            "SELECT id, bot_id, level, gate, message, ts, trace_id FROM events WHERE (?1=0 OR level='ERROR') ORDER BY id DESC LIMIT ?2".into(),
            None,
        ),
    };
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let err_flag: i64 = if error_only { 1 } else { 0 };
    match bot {
        Some(b) => read_rows(&mut stmt, &[&b as &dyn rusqlite::ToSql, &err_flag, &lim]),
        None => read_rows(&mut stmt, &[&err_flag as &dyn rusqlite::ToSql, &lim]),
    }
}

fn db_locked(app: &AppState) -> Result<std::sync::MutexGuard<'_, Connection>, String> {
    app.db.conn.lock().map_err(|e| format!("kilit: {e}"))
}

pub async fn events_forensics(State(app): State<Arc<AppState>>, Query(q): Query<ForensicsQuery>) -> Json<Value> {
    if let Some(t) = q.trace_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
        let trace: String = t.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').take(64).collect();
        if trace.is_empty() {
            return Json(json!({"trace": Value::Null, "events": [], "at": now_ms()}));
        }
        return match db_locked(&app).and_then(|c| query_trace(&c, &trace)) {
            Ok((summary, events)) => Json(json!({
                "trace": summary,
                "events": events.iter().map(row_to_json).collect::<Vec<_>>(),
                "at": now_ms(),
            })),
            Err(e) => Json(json!({"trace": Value::Null, "events": [], "error": e, "at": now_ms()})),
        };
    }
    if q.mode.as_deref() == Some("traces") {
        let lim = q.limit.unwrap_or(50);
        return match db_locked(&app).and_then(|c| query_traces(&c, lim, q.min_latency_ms)) {
            Ok(traces) => Json(json!({"traces": traces, "at": now_ms()})),
            Err(e) => Json(json!({"traces": [], "error": e, "at": now_ms()})),
        };
    }
    let bot = q.bot_id.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let lim = q.limit.unwrap_or(200);
    match db_locked(&app).and_then(|c| query_filter(&c, bot.as_deref(), q.error_only.unwrap_or(false), lim)) {
        Ok(events) => Json(json!({
            "events": events.iter().map(row_to_json).collect::<Vec<_>>(),
            "filters": {"botId": bot, "errorOnly": q.error_only.unwrap_or(false), "minLatencyMs": q.min_latency_ms,
                        "minLatencyNote": "minLatencyMs yalniz traces modunda iz-toplamina uygulanir"},
            "at": now_ms(),
        })),
        Err(e) => Json(json!({"events": [], "error": e, "at": now_ms()})),
    }
}

#[cfg(test)]
mod forensics_tests {
    use super::*;
    use rusqlite::Connection;
    use std::sync::Mutex;

    fn mem_db() -> crate::store::Db {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, bot_id TEXT, level TEXT, gate TEXT, message TEXT, ts INTEGER, trace_id TEXT);",
        )
        .unwrap();
        crate::store::Db { conn: Mutex::new(conn) }
    }

    fn seed(db: &crate::store::Db) {
        let c = db.conn.lock().unwrap();
        // iz-1: TICK kökü + 2 olay (1 ERROR)
        c.execute("INSERT INTO events(bot_id, level, gate, message, ts, trace_id) VALUES('b1','INFO','TICK','TICK symbol=BTCUSDT kind=FUTURES_DCA venue=BYBIT mode=PAPER',1000,'tr-aaa-1000')", []).unwrap();
        c.execute("INSERT INTO events(bot_id, level, gate, message, ts, trace_id) VALUES('b1','INFO','ENGINE','bar islendi',1010,'tr-aaa-1000')", []).unwrap();
        c.execute("INSERT INTO events(bot_id, level, gate, message, ts, trace_id) VALUES('b1','ERROR','ENGINE','kline bos',1120,'tr-aaa-1000')", []).unwrap();
        // iz-2: kısa, hatasız
        c.execute("INSERT INTO events(bot_id, level, gate, message, ts, trace_id) VALUES('b1','INFO','TICK','TICK symbol=ETHUSDT kind=DCA_LONG venue=BYBIT mode=PAPER',2000,'tr-bbb-2000')", []).unwrap();
        // izsiz eski satır
        c.execute("INSERT INTO events(bot_id, level, gate, message, ts, trace_id) VALUES('b1','WARN','HALT','salter devrede',500,NULL)", []).unwrap();
    }

    #[test]
    fn trace_mode_returns_summary_and_asc_events() {
        let db = mem_db();
        seed(&db);
        let c = db.conn.lock().unwrap();
        let (summary, events) = query_trace(&c, "tr-aaa-1000").unwrap();
        assert_eq!(summary["eventCount"], 3);
        assert_eq!(summary["errorCount"], 1);
        assert_eq!(summary["totalMs"], 120);
        assert_eq!(summary["spanCount"], 0);
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].gate, "TICK");
        assert_eq!(events[2].level, "ERROR");
    }

    #[test]
    fn trace_mode_empty_for_unknown() {
        let db = mem_db();
        seed(&db);
        let c = db.conn.lock().unwrap();
        let (summary, events) = query_trace(&c, "tr-yok").unwrap();
        assert!(summary.is_null());
        assert!(events.is_empty());
    }

    #[test]
    fn traces_mode_groups_and_parses_root() {
        let db = mem_db();
        seed(&db);
        let c = db.conn.lock().unwrap();
        let traces = query_traces(&c, 50, None).unwrap();
        assert_eq!(traces.len(), 2);
        // son iz önce (MAX(ts) desc)
        assert_eq!(traces[0]["traceId"], "tr-bbb-2000");
        assert_eq!(traces[0]["symbol"], "ETHUSDT");
        assert_eq!(traces[0]["kind"], "DCA_LONG");
        assert_eq!(traces[1]["errorCount"], 1);
        assert_eq!(traces[1]["totalMs"], 120);
        // izsiz satır gruba girmez
        assert!(traces.iter().all(|t| t["traceId"] != Value::Null));
    }

    #[test]
    fn traces_mode_min_latency_filters_short() {
        let db = mem_db();
        seed(&db);
        let c = db.conn.lock().unwrap();
        let traces = query_traces(&c, 50, Some(100)).unwrap();
        assert_eq!(traces.len(), 1);
        assert_eq!(traces[0]["traceId"], "tr-aaa-1000");
    }

    #[test]
    fn filter_mode_bot_and_error_only() {
        let db = mem_db();
        seed(&db);
        let c = db.conn.lock().unwrap();
        let all = query_filter(&c, Some("b1"), false, 200).unwrap();
        assert_eq!(all.len(), 5); // izsiz dahil
        let errs = query_filter(&c, Some("b1"), true, 200).unwrap();
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].level, "ERROR");
        let none = query_filter(&c, Some("yok"), false, 200).unwrap();
        assert!(none.is_empty());
    }

    #[test]
    fn tick_root_parse() {
        let (s, k) = parse_tick_root("TICK symbol=BTCUSDT kind=FUTURES_DCA venue=BYBIT mode=PAPER");
        assert_eq!(s.as_deref(), Some("BTCUSDT"));
        assert_eq!(k.as_deref(), Some("FUTURES_DCA"));
        let (s2, k2) = parse_tick_root("bar islendi");
        assert!(s2.is_none() && k2.is_none());
    }
}
