//! Faz 2.7 venue bakim-marji kademe aynasi — GET only.
//!
//! v3 eslemesi: `POST /api/calculator/brackets/refresh` (Binance'ten al) +
//! `POST .../brackets/manual` (elle gir). Bizde tek salt-okunur uc:
//! `GET /api/calc/brackets?venue=BYBIT|BINANCE&symbol=BTCUSDT`.
//!
//! - BYBIT: halka acik `/v5/market/risk-limit` (anahtarsiz).
//! - BINANCE: imzali futures `leverageBracket` (anahtar yoksa ABSTAIN).
//! - OKX/PAPER: kademe aynasi desteklenmiyor -> `source: UNSUPPORTED`
//!   (motor uydurmaz; 422 degil cunku venue gecerli, yetenek yok).
//! - Bilinmeyen venue / eksik symbol -> 422 (istemci hatasi).
//! - Manuel kademe girisi sunucuya yazilmaz: kullanici kademeleri
//!   Hesaplayici'da gorur/duzenler, yalniz bellek-ici (istemci state).
//!   Venue verisi taklidi yapilmaz.
//! - Bu uc ASLA 500 donmez.
//! - maxLeverage tasinmaz — M07 (`MmrBracket`) tuketmiyor (notlu).

use axum::extract::{Query, State};
use axum::Json;
use dca_exchange::VenueAdapter;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::state::{now_ms, AppState};

#[derive(Deserialize)]
pub struct BracketsQuery {
    pub venue: Option<String>,
    pub symbol: Option<String>,
}

fn tiers_body(venue: &str, symbol: &str, source: &str, tiers: Value, reason: Option<String>) -> Value {
    let mut b = serde_json::Map::new();
    b.insert("venue".into(), json!(venue));
    b.insert("symbol".into(), json!(symbol));
    b.insert("source".into(), json!(source));
    b.insert("tiers".into(), tiers);
    if let Some(r) = reason {
        b.insert("reason".into(), json!(r));
    }
    b.insert("at".into(), json!(now_ms()));
    b.insert(
        "note".into(),
        json!("M07 kademeleri (venue aynasi); maxLeverage tasinmaz. Hesaplayici tahmini tek-kademe (%0.5) kullanir — ayna yalniz operator referansidir."),
    );
    Value::Object(b)
}

fn unsupported(venue: &str, symbol: &str) -> Value {
    tiers_body(
        venue,
        symbol,
        "UNSUPPORTED",
        json!([]),
        Some("bu venue kademe aynasi desteklemiyor (OKX/PAPER)".into()),
    )
}

fn abstain(venue: &str, symbol: &str, reason: String) -> Value {
    tiers_body(venue, symbol, "ABSTAIN", json!([]), Some(reason))
}

pub async fn calc_brackets(
    State(app): State<Arc<AppState>>,
    Query(q): Query<BracketsQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let venue = q.venue.map(|v| v.to_uppercase()).unwrap_or_default();
    if venue != "BYBIT" && venue != "BINANCE" && venue != "OKX" && venue != "PAPER" {
        return Err(crate::err422("bilinmeyen venue (gecerli: BYBIT | BINANCE | OKX | PAPER)"));
    }
    let symbol: String = q
        .symbol
        .map(|s| {
            s.trim().to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).take(20).collect()
        })
        .unwrap_or_default();
    if symbol.is_empty() {
        return Err(crate::err422("symbol gerekli (?symbol=BTCUSDT)"));
    }
    if venue == "BYBIT" {
        return match app.bybit.mmr_brackets(&symbol, None).await {
            Ok(t) => Ok(Json(tiers_body(&venue, &symbol, "EXCHANGE", json!(t), None))),
            Err(e) => Ok(Json(abstain(&venue, &symbol, e.chars().take(140).collect()))),
        };
    }
    if venue == "BINANCE" {
        let creds = crate::active_keypair(&app, "BINANCE");
        return match app.binance.mmr_brackets(&symbol, creds.as_ref()).await {
            Ok(t) => Ok(Json(tiers_body(&venue, &symbol, "EXCHANGE", json!(t), None))),
            Err(e) => Ok(Json(abstain(&venue, &symbol, e.chars().take(140).collect()))),
        };
    }
    Ok(Json(unsupported(&venue, &symbol)))
}
