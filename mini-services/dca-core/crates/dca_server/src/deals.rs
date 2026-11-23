//! Closed Deals Ledger & Daily PnL Aggregation
//!
//! Exposes:
//! - `GET /api/deals` -> Closed deals history with day/bot/symbol filters and aggregate stats
//! - `GET /api/daily-pnl` -> Daily realized PnL breakdown for bar charts over 7/14/30 days

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::state::{now_ms, AppState};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClosedDeal {
    pub id: String,
    pub deal_no: Option<i64>,
    pub bot_id: String,
    pub bot_name: String,
    pub symbol: String,
    pub venue: String,
    pub kind: String,
    pub closed_ts: i64,
    pub date: String,
    pub time: String,
    pub gross_pnl: f64,
    pub fees: f64,
    pub funding: f64,
    pub net_pnl: f64,
    pub wins: u32,
    pub losses: u32,
    pub result: String, // "WIN" | "LOSS"
    pub reason: String, // "TP" | "SL" | "LIQUIDATION" | "MANUAL"
}

struct BotMeta {
    name: String,
    symbol: String,
    venue: String,
    kind: String,
}

fn parse_closed_deal_message(
    id: i64,
    bot_id: &str,
    msg: &str,
    ts: i64,
    bots: &HashMap<String, BotMeta>,
) -> ClosedDeal {
    let mut net = 0.0f64;
    let mut gross = 0.0f64;
    let mut fees = 0.0f64;
    let mut funding = 0.0f64;
    let mut wins = 0u32;
    let mut losses = 0u32;
    let mut has_net = false;
    let mut has_gross = false;
    let mut deal_no = None;
    let mut closed_ts = None;

    for token in msg.split_whitespace() {
        if let Some((k, v)) = token.split_once('=') {
            match k {
                "deal_no" => deal_no = v.parse::<i64>().ok().filter(|value| *value > 0),
                "closed_ts" => closed_ts = v.parse::<i64>().ok().filter(|value| *value > 0),
                "net" => {
                    if let Some(val) = v.parse::<f64>().ok().filter(|val| val.is_finite()) {
                        net = val;
                        has_net = true;
                    }
                }
                "gross" => {
                    if let Some(val) = v.parse::<f64>().ok().filter(|val| val.is_finite()) {
                        gross = val;
                        has_gross = true;
                    }
                }
                "net_gross" => {
                    if let Some(val) = v.parse::<f64>().ok().filter(|val| val.is_finite()) {
                        net = val;
                        gross = val;
                        has_net = true;
                        has_gross = true;
                    }
                }
                "fees" => {
                    if let Some(val) = v.parse::<f64>().ok().filter(|val| val.is_finite()) {
                        fees = val;
                    }
                }
                "funding" => {
                    if let Some(val) = v.parse::<f64>().ok().filter(|val| val.is_finite()) {
                        funding = val;
                    }
                }
                "wins" => {
                    if let Ok(val) = v.parse::<u32>() {
                        wins = val;
                    }
                }
                "losses" => {
                    if let Ok(val) = v.parse::<u32>() {
                        losses = val;
                    }
                }
                _ => {}
            }
        }
    }

    if !has_net && has_gross {
        net = gross - fees - funding;
    } else if has_net && !has_gross {
        gross = net + fees + funding;
    }

    let meta = bots.get(bot_id);
    let bot_name = meta.map(|m| m.name.clone()).unwrap_or_else(|| bot_id.to_string());
    let symbol = meta.map(|m| m.symbol.clone()).unwrap_or_else(|| "BTCUSDT".to_string());
    let venue = meta.map(|m| m.venue.clone()).unwrap_or_else(|| "BYBIT".to_string());
    let kind = meta.map(|m| m.kind.clone()).unwrap_or_else(|| "DCA".to_string());

    let closed_ts = closed_ts.unwrap_or(ts);
    let dt = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(closed_ts)
        .unwrap_or_else(chrono::Utc::now);
    let date = dt.format("%Y-%m-%d").to_string();
    let time = dt.format("%H:%M:%S").to_string();

    let result = if net > 0.0 { "WIN".to_string() } else { "LOSS".to_string() };
    let reason = msg
        .split_whitespace()
        .find_map(|token| token.strip_prefix("reason="))
        .filter(|reason| matches!(*reason, "TP" | "SL" | "LIQUIDATION" | "MANUAL" | "TRAIL" | "CLOSE"))
        .unwrap_or("CLOSE")
        .to_string();

    ClosedDeal {
        id: format!("deal-{id}"),
        deal_no,
        bot_id: bot_id.to_string(),
        bot_name,
        symbol,
        venue,
        kind,
        closed_ts,
        date,
        time,
        gross_pnl: (gross * 100_000_000.0).round() / 100_000_000.0,
        fees: (fees * 100_000_000.0).round() / 100_000_000.0,
        funding: (funding * 100_000_000.0).round() / 100_000_000.0,
        net_pnl: (net * 100_000_000.0).round() / 100_000_000.0,
        wins,
        losses,
        result,
        reason,
    }
}

pub fn fetch_all_closed_deals(conn: &rusqlite::Connection) -> Result<Vec<ClosedDeal>, rusqlite::Error> {
    // 1. Load bots lookup
    let mut bot_map: HashMap<String, BotMeta> = HashMap::new();
    let mut b_stmt = conn.prepare("SELECT id, name, symbol, venue, kind FROM bots")?;
    let b_rows = b_stmt.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            BotMeta {
                name: r.get(1)?,
                symbol: r.get(2)?,
                venue: r.get(3)?,
                kind: r.get(4)?,
            },
        ))
    })?;
    for row in b_rows.flatten() {
        bot_map.insert(row.0, row.1);
    }

    // 2. Load closed deals from events
    let mut stmt = conn.prepare(
        "SELECT id, bot_id, message, ts FROM events WHERE message LIKE '%deal closed%' ORDER BY ts DESC",
    )?;
    let ev_rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, Option<String>>(1)?.unwrap_or_default(),
            r.get::<_, Option<String>>(2)?.unwrap_or_default(),
            r.get::<_, i64>(3)?,
        ))
    })?;

    let mut event_deals = Vec::new();
    for row in ev_rows.flatten() {
        let (id, bot_id, msg, ts) = row;
        event_deals.push(parse_closed_deal_message(id, &bot_id, &msg, ts, &bot_map));
    }

    // 3. Load the durable ledger. It is authoritative when it carries the
    // same bot/deal number as an event; the event remains a fallback for old
    // databases written before the deals table was introduced.
    let mut table_deals = Vec::new();
    if let Ok(mut d_stmt) = conn.prepare(
        "SELECT id, bot_id, deal_no, gross_pnl, fees, funding, closed_ts FROM deals WHERE closed_ts > 0 ORDER BY closed_ts DESC",
    ) {
        let d_rows = d_stmt.query_map([], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, Option<i64>>(2)?,
                r.get::<_, Option<String>>(3)?.unwrap_or_default(),
                r.get::<_, Option<String>>(4)?.unwrap_or_default(),
                r.get::<_, Option<String>>(5)?.unwrap_or_default(),
                r.get::<_, i64>(6)?,
            ))
        });
        if let Ok(rows) = d_rows {
            for row in rows.flatten() {
                let (id, bot_id, deal_no, gross_str, fees_str, funding_str, ts) = row;
                let gross = gross_str.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0);
                let fees = fees_str.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0);
                let funding = funding_str.parse::<f64>().ok().filter(|v| v.is_finite()).unwrap_or(0.0);
                let net = gross - fees - funding;
                let meta = bot_map.get(&bot_id);
                let dt = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ts)
                    .unwrap_or_else(chrono::Utc::now);
                table_deals.push(ClosedDeal {
                    id: format!("deal-tbl-{id}"),
                    deal_no,
                    bot_name: meta.map(|m| m.name.clone()).unwrap_or_else(|| bot_id.clone()),
                    symbol: meta.map(|m| m.symbol.clone()).unwrap_or_else(|| "BTCUSDT".to_string()),
                    venue: meta.map(|m| m.venue.clone()).unwrap_or_else(|| "BYBIT".to_string()),
                    kind: meta.map(|m| m.kind.clone()).unwrap_or_else(|| "DCA".to_string()),
                    bot_id,
                    closed_ts: ts,
                    date: dt.format("%Y-%m-%d").to_string(),
                    time: dt.format("%H:%M:%S").to_string(),
                    gross_pnl: gross,
                    fees,
                    funding,
                    net_pnl: net,
                    wins: if net > 0.0 { 1 } else { 0 },
                    losses: if net <= 0.0 { 1 } else { 0 },
                    result: if net > 0.0 { "WIN".to_string() } else { "LOSS".to_string() },
                    reason: "CLOSE".to_string(),
                });
            }
        }
    }

    let table_deal_keys: HashSet<(String, i64)> = table_deals
        .iter()
        .filter_map(|deal| deal.deal_no.map(|number| (deal.bot_id.clone(), number)))
        .collect();
    let mut deals = Vec::with_capacity(event_deals.len() + table_deals.len());
    for (event_index, event) in event_deals.iter().enumerate() {
        if event
            .deal_no
            .is_some_and(|number| table_deal_keys.contains(&(event.bot_id.clone(), number)))
        {
            continue;
        }
        if event.deal_no.is_none()
            && is_unambiguous_legacy_duplicate(event_index, &event_deals, &table_deals)
        {
            continue;
        }
        deals.push(event.clone());
    }
    deals.extend(table_deals);

    // Sort by closed_ts descending
    deals.sort_by(|a, b| b.closed_ts.cmp(&a.closed_ts));
    Ok(deals)
}

/// Persist each bot-local deal identity at most once. A single SQLite
/// statement makes the existence check and insert atomic for concurrent
/// engine processes that share the same database file.
pub(crate) fn insert_closed_deal_once(
    conn: &rusqlite::Connection,
    bot_id: &str,
    deal_no: i64,
    opened_ts: i64,
    closed_ts: i64,
    gross_pnl: &str,
    fees: &str,
    funding: &str,
    net_pnl: &str,
    safety_triggered: usize,
) -> Result<bool, rusqlite::Error> {
    let inserted = conn.execute(
        "INSERT INTO deals(bot_id, deal_no, opened_ts, closed_ts, gross_pnl, fees, funding, net_pnl, safety_triggered, status)
         SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'CLOSED'
         WHERE NOT EXISTS (SELECT 1 FROM deals WHERE bot_id=?1 AND deal_no=?2)",
        rusqlite::params![
            bot_id,
            deal_no,
            opened_ts,
            closed_ts,
            gross_pnl,
            fees,
            funding,
            net_pnl,
            safety_triggered as i64
        ],
    )?;
    Ok(inserted > 0)
}

const LEGACY_DEAL_TIMESTAMP_SKEW_MS: u64 = 6 * 60 * 60 * 1000;

fn same_financial_deal(left: &ClosedDeal, right: &ClosedDeal) -> bool {
    let same_amount = |a: f64, b: f64| (a - b).abs() <= 0.00000001;
    left.bot_id == right.bot_id
        && same_amount(left.gross_pnl, right.gross_pnl)
        && same_amount(left.fees, right.fees)
        && same_amount(left.funding, right.funding)
        && same_amount(left.net_pnl, right.net_pnl)
        && left.closed_ts.abs_diff(right.closed_ts) <= LEGACY_DEAL_TIMESTAMP_SKEW_MS
}

/// Old close events lack the durable deal number. Merge one only when both
/// sides have a unique exact financial match within the observed event-time
/// skew; ambiguous matches stay visible so a real trade is never discarded.
fn is_unambiguous_legacy_duplicate(
    event_index: usize,
    events: &[ClosedDeal],
    table_deals: &[ClosedDeal],
) -> bool {
    let mut table_match = None;
    for table_deal in table_deals {
        if same_financial_deal(&events[event_index], table_deal) {
            if table_match.is_some() {
                return false;
            }
            table_match = Some(table_deal);
        }
    }
    let Some(table_deal) = table_match else {
        return false;
    };

    let reciprocal_match_count = events
        .iter()
        .filter(|candidate| {
            candidate.deal_no.is_none() && same_financial_deal(candidate, table_deal)
        })
        .count();
    reciprocal_match_count == 1
}

/// Profit factor over individual trade net results: positive net sum divided
/// by the absolute negative net sum (fees and funding are already included).
fn profit_factor_from_net_values(values: impl IntoIterator<Item = f64>) -> f64 {
    let mut positive_net = 0.0f64;
    let mut negative_net = 0.0f64;
    for value in values.into_iter().filter(|value| value.is_finite()) {
        if value > 0.0 {
            positive_net = (positive_net + value).min(f64::MAX);
        } else if value < 0.0 {
            negative_net = (negative_net + value.abs()).min(f64::MAX);
        }
    }
    if negative_net > 0.0 {
        let ratio = positive_net / negative_net;
        if !ratio.is_finite() || ratio > 99.99 {
            99.99
        } else {
            (ratio * 100.0).round() / 100.0
        }
    } else if positive_net > 0.0 {
        99.99
    } else {
        0.0
    }
}

fn deal_cutoff_ts(days: Option<i64>, now: i64) -> Option<i64> {
    let days = days?.clamp(1, 3_650);
    Some(now.saturating_sub(days.saturating_mul(86_400_000)))
}

/// GET /api/deals
pub async fn list_deals(
    State(app): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, Response> {
    let conn = app.db.conn.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response()
    })?;

    let all_deals = fetch_all_closed_deals(&conn).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response()
    })?;
    drop(conn);

    let days: Option<i64> = params.get("days").and_then(|v| v.parse().ok());
    let date_filter = params.get("date").map(|s| s.trim()).filter(|s| !s.is_empty());
    let bot_id_filter = params.get("bot_id").map(|s| s.trim()).filter(|s| !s.is_empty());
    let symbol_filter = params.get("symbol").map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty());
    let query_filter = params.get("q").map(|s| s.trim().to_lowercase()).filter(|s| !s.is_empty());
    let limit: usize = params
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(200)
        .min(1000);

    let cutoff_ts = deal_cutoff_ts(days, now_ms());

    let filtered: Vec<ClosedDeal> = all_deals
        .into_iter()
        .filter(|d| {
            if let Some(cutoff) = cutoff_ts {
                if d.closed_ts < cutoff {
                    return false;
                }
            }
            if let Some(df) = date_filter {
                if d.date != df {
                    return false;
                }
            }
            if let Some(ref bid) = bot_id_filter {
                if &d.bot_id != bid {
                    return false;
                }
            }
            if let Some(ref sym) = symbol_filter {
                if &d.symbol != sym {
                    return false;
                }
            }
            if let Some(ref q) = query_filter {
                let matches_name = d.bot_name.to_lowercase().contains(q);
                let matches_sym = d.symbol.to_lowercase().contains(q);
                let matches_bot_id = d.bot_id.to_lowercase().contains(q);
                if !matches_name && !matches_sym && !matches_bot_id {
                    return false;
                }
            }
            true
        })
        .collect();

    let total = filtered.len();
    let mut total_net_pnl = 0.0f64;
    let mut total_gross_pnl = 0.0f64;
    let mut total_fees = 0.0f64;
    let mut total_funding = 0.0f64;
    let mut wins = 0usize;
    let mut losses = 0usize;

    for d in &filtered {
        total_net_pnl += d.net_pnl;
        total_gross_pnl += d.gross_pnl;
        total_fees += d.fees;
        total_funding += d.funding;
        if d.net_pnl > 0.0 {
            wins += 1;
        } else {
            losses += 1;
        }
    }

    let win_rate = if total > 0 {
        (wins as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let profit_factor = profit_factor_from_net_values(filtered.iter().map(|d| d.net_pnl));

    let avg_deal_profit = if total > 0 {
        ((total_net_pnl / total as f64) * 100.0).round() / 100.0
    } else {
        0.0
    };

    let paged: Vec<ClosedDeal> = filtered.into_iter().take(limit).collect();

    Ok(Json(json!({
        "deals": paged,
        "total": total,
        "summary": {
            "totalNetPnl": (total_net_pnl * 100.0).round() / 100.0,
            "totalGrossPnl": (total_gross_pnl * 100.0).round() / 100.0,
            "totalFees": (total_fees * 10000.0).round() / 10000.0,
            "totalFunding": (total_funding * 10000.0).round() / 10000.0,
            "wins": wins,
            "losses": losses,
            "winRate": (win_rate * 100.0).round() / 100.0,
            "profitFactor": profit_factor,
            "avgDealProfit": avg_deal_profit
        }
    })))
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DailyPnlBar {
    pub date: String,
    pub day_ts: i64,
    pub net_pnl: f64,
    pub gross_pnl: f64,
    pub fees: f64,
    pub funding: f64,
    pub count: usize,
    pub wins: usize,
    pub losses: usize,
    pub win_rate: f64,
}

/// GET /api/daily-pnl
pub async fn daily_pnl(
    State(app): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<Value>, Response> {
    let days: i64 = params
        .get("days")
        .and_then(|v| v.parse().ok())
        .unwrap_or(30)
        .clamp(1, 90);

    let bot_id_filter = params.get("bot_id").map(|s| s.trim()).filter(|s| !s.is_empty());
    let symbol_filter = params.get("symbol").map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty());

    let conn = app.db.conn.lock().map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response()
    })?;

    let all_deals = fetch_all_closed_deals(&conn).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({"error": e.to_string()})),
        )
            .into_response()
    })?;
    drop(conn);

    let now = now_ms();
    let cutoff_ts = now - days * 86_400_000;

    // Filter deals in range
    let in_range_deals: Vec<&ClosedDeal> = all_deals
        .iter()
        .filter(|d| {
            if d.closed_ts < cutoff_ts {
                return false;
            }
            if let Some(ref bid) = bot_id_filter {
                if &d.bot_id != bid {
                    return false;
                }
            }
            if let Some(ref sym) = symbol_filter {
                if &d.symbol != sym {
                    return false;
                }
            }
            true
        })
        .collect();

    // Group by calendar day string "YYYY-MM-DD"
    let mut day_map: HashMap<String, Vec<&ClosedDeal>> = HashMap::new();
    for &d in &in_range_deals {
        day_map.entry(d.date.clone()).or_default().push(d);
    }

    // Build continuous chronological day sequence from (now - days) to now
    let mut bars: Vec<DailyPnlBar> = Vec::new();
    let mut total_net_pnl = 0.0f64;
    let mut total_deals = 0usize;
    let mut profitable_days = 0usize;
    let mut unprofitable_days = 0usize;
    let mut neutral_days = 0usize;

    let now_dt = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now)
        .unwrap_or_else(chrono::Utc::now);

    for i in (0..days).rev() {
        let target_dt = now_dt - chrono::Duration::days(i);
        let date_str = target_dt.format("%Y-%m-%d").to_string();
        let day_start_ts = target_dt
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .unwrap_or_default()
            .and_utc()
            .timestamp_millis();

        if let Some(deals) = day_map.get(&date_str) {
            let count = deals.len();
            let mut net_sum = 0.0f64;
            let mut gross_sum = 0.0f64;
            let mut fees_sum = 0.0f64;
            let mut funding_sum = 0.0f64;
            let mut wins = 0usize;
            let mut losses = 0usize;

            for d in deals {
                net_sum += d.net_pnl;
                gross_sum += d.gross_pnl;
                fees_sum += d.fees;
                funding_sum += d.funding;
                if d.net_pnl > 0.0 {
                    wins += 1;
                } else {
                    losses += 1;
                }
            }

            let win_rate = if count > 0 {
                (wins as f64 / count as f64) * 100.0
            } else {
                0.0
            };

            total_net_pnl += net_sum;
            total_deals += count;

            if net_sum > 0.0001 {
                profitable_days += 1;
            } else if net_sum < -0.0001 {
                unprofitable_days += 1;
            } else {
                neutral_days += 1;
            }

            bars.push(DailyPnlBar {
                date: date_str,
                day_ts: day_start_ts,
                net_pnl: (net_sum * 100.0).round() / 100.0,
                gross_pnl: (gross_sum * 100.0).round() / 100.0,
                fees: (fees_sum * 10000.0).round() / 10000.0,
                funding: (funding_sum * 10000.0).round() / 10000.0,
                count,
                wins,
                losses,
                win_rate: (win_rate * 10.0).round() / 10.0,
            });
        } else {
            neutral_days += 1;
            bars.push(DailyPnlBar {
                date: date_str,
                day_ts: day_start_ts,
                net_pnl: 0.0,
                gross_pnl: 0.0,
                fees: 0.0,
                funding: 0.0,
                count: 0,
                wins: 0,
                losses: 0,
                win_rate: 0.0,
            });
        }
    }

    let best_day = bars
        .iter()
        .filter(|b| b.count > 0)
        .max_by(|a, b| a.net_pnl.partial_cmp(&b.net_pnl).unwrap_or(std::cmp::Ordering::Equal))
        .map(|b| json!({"date": b.date, "netPnl": b.net_pnl, "count": b.count}));

    let worst_day = bars
        .iter()
        .filter(|b| b.count > 0)
        .min_by(|a, b| a.net_pnl.partial_cmp(&b.net_pnl).unwrap_or(std::cmp::Ordering::Equal))
        .map(|b| json!({"date": b.date, "netPnl": b.net_pnl, "count": b.count}));

    let profit_factor = profit_factor_from_net_values(in_range_deals.iter().map(|d| d.net_pnl));

    Ok(Json(json!({
        "days": bars,
        "summary": {
            "totalNetPnl": (total_net_pnl * 100.0).round() / 100.0,
            "totalDeals": total_deals,
            "profitableDays": profitable_days,
            "unprofitableDays": unprofitable_days,
            "neutralDays": neutral_days,
            "bestDay": best_day,
            "worstDay": worst_day,
            "profitFactor": profit_factor
        }
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_deal_without_reason_is_not_mislabeled_as_take_profit() {
        let deal = parse_closed_deal_message(
            1,
            "bot-1",
            "deal closed gross=1 fees=0 funding=0 net=0.9 wins=1 losses=0",
            1_700_000_000_000,
            &HashMap::new(),
        );
        assert_eq!(deal.reason, "CLOSE");
    }

    #[test]
    fn closed_deal_keeps_an_explicit_exit_reason() {
        let deal = parse_closed_deal_message(
            2,
            "bot-1",
            "deal closed gross=-1 fees=0 funding=0 net=-1 reason=TRAIL wins=0 losses=1",
            1_700_000_000_000,
            &HashMap::new(),
        );
        assert_eq!(deal.reason, "TRAIL");
    }

    #[test]
    fn break_even_deal_is_not_counted_as_a_win() {
        let deal = parse_closed_deal_message(
            4,
            "bot-1",
            "deal closed gross=0 fees=0 funding=0 net=0 wins=0 losses=1 reason=MANUAL",
            1_700_000_000_000,
            &HashMap::new(),
        );
        assert_eq!(deal.result, "LOSS");
    }

    #[test]
    fn malformed_nonfinite_pnl_is_ignored() {
        let deal = parse_closed_deal_message(
            3,
            "bot-1",
            "deal closed gross=1 fees=0 funding=0 net=NaN wins=0 losses=1",
            1_700_000_000_000,
            &HashMap::new(),
        );
        assert!(deal.net_pnl.is_finite());
        assert_eq!(deal.net_pnl, 1.0);
    }

    fn ledger_test_connection() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE bots(id TEXT PRIMARY KEY, name TEXT, symbol TEXT, venue TEXT, kind TEXT);
             CREATE TABLE events(id INTEGER PRIMARY KEY, bot_id TEXT, message TEXT, ts INTEGER);
             CREATE TABLE deals(id INTEGER PRIMARY KEY, bot_id TEXT, deal_no INTEGER, opened_ts INTEGER, gross_pnl TEXT, fees TEXT, funding TEXT, net_pnl TEXT, safety_triggered INTEGER, status TEXT, closed_ts INTEGER);
             INSERT INTO bots(id, name, symbol, venue, kind) VALUES('bot-1', 'Paper bot', 'BTCUSDT', 'BYBIT', 'DCA');",
        )
        .unwrap();
        conn
    }

    #[test]
    fn stable_deal_number_merges_event_and_table_even_when_event_logged_late() {
        let conn = ledger_test_connection();
        conn.execute(
            "INSERT INTO events(id, bot_id, message, ts) VALUES(1, 'bot-1', ?1, 9999999)",
            ["deal closed deal_no=7 closed_ts=1000 gross=1 fees=0.1 funding=0 net=0.9 reason=TP wins=1 losses=0"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deals(id, bot_id, deal_no, opened_ts, gross_pnl, fees, funding, net_pnl, safety_triggered, status, closed_ts) VALUES(1, 'bot-1', 7, 0, '1', '0.1', '0', '0.9', 0, 'CLOSED', 1000)",
            [],
        )
        .unwrap();

        let deals = fetch_all_closed_deals(&conn).unwrap();
        assert_eq!(deals.len(), 1);
        assert_eq!(deals[0].id, "deal-tbl-1");
        assert_eq!(deals[0].deal_no, Some(7));
        assert_eq!(deals[0].closed_ts, 1000);
    }

    #[test]
    fn unique_legacy_event_table_pair_is_merged_within_time_skew() {
        let conn = ledger_test_connection();
        conn.execute(
            "INSERT INTO events(id, bot_id, message, ts) VALUES(1, 'bot-1', ?1, 120000)",
            ["deal closed gross=1 fees=0.1 funding=0 net=0.9 reason=TP wins=1 losses=0"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deals(id, bot_id, deal_no, opened_ts, gross_pnl, fees, funding, net_pnl, safety_triggered, status, closed_ts) VALUES(1, 'bot-1', 7, 0, '1', '0.1', '0', '0.9', 0, 'CLOSED', 1000)",
            [],
        )
        .unwrap();

        let deals = fetch_all_closed_deals(&conn).unwrap();
        assert_eq!(deals.len(), 1);
        assert_eq!(deals[0].id, "deal-tbl-1");
    }

    #[test]
    fn ambiguous_legacy_matches_are_preserved() {
        let conn = ledger_test_connection();
        conn.execute(
            "INSERT INTO events(id, bot_id, message, ts) VALUES(1, 'bot-1', ?1, 1500)",
            ["deal closed gross=1 fees=0.1 funding=0 net=0.9 reason=TP wins=1 losses=0"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO deals(id, bot_id, deal_no, opened_ts, gross_pnl, fees, funding, net_pnl, safety_triggered, status, closed_ts) VALUES(1, 'bot-1', 7, 0, '1', '0.1', '0', '0.9', 0, 'CLOSED', 1000), (2, 'bot-1', 8, 0, '1', '0.1', '0', '0.9', 0, 'CLOSED', 2000)",
            [],
        )
        .unwrap();

        let deals = fetch_all_closed_deals(&conn).unwrap();
        assert_eq!(deals.len(), 3);
    }

    #[test]
    fn durable_deal_insert_is_idempotent_by_bot_and_deal_number() {
        let conn = ledger_test_connection();
        let inserted = insert_closed_deal_once(
            &conn, "bot-1", 7, 900, 1000, "1", "0.1", "0", "0.9", 0,
        )
        .unwrap();
        let duplicate = insert_closed_deal_once(
            &conn, "bot-1", 7, 900, 1000, "1", "0.1", "0", "0.9", 0,
        )
        .unwrap();
        assert!(inserted);
        assert!(!duplicate);
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM deals WHERE bot_id='bot-1' AND deal_no=7", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn profit_factor_uses_each_trade_net_result_not_daily_net_totals() {
        let factor = profit_factor_from_net_values([10.0, -9.0, -5.0]);
        assert_eq!(factor, 0.71);
    }

    #[test]
    fn profit_factor_caps_unrepresentable_extreme_ratios_for_json() {
        assert_eq!(profit_factor_from_net_values([f64::MAX, -f64::MIN_POSITIVE]), 99.99);
    }

    #[test]
    fn deal_date_filter_saturates_untrusted_day_counts() {
        assert_eq!(deal_cutoff_ts(Some(i64::MAX), 1_800_000_000_000), Some(1_484_640_000_000));
        assert_eq!(deal_cutoff_ts(Some(i64::MIN), 1_800_000_000_000), Some(1_799_913_600_000));
        assert_eq!(deal_cutoff_ts(None, 1_800_000_000_000), None);
    }
}
