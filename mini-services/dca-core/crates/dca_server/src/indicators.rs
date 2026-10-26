//! Faz 2.2 indikatör ucu (v3 `GET /api/indicators` karşılığı — ŞARTLI port).
//!
//! v3 şekli aynen: `?symbol&interval&limit&set=EMA20,RSI14,MACD,BB20` →
//! `{symbol, interval, times, indicators}` (NaN → null). Hesap
//! `dca_signals::indicators` ile (sinyal-üreticiyle aynı fonksiyonlar —
//! hesap birliği; UI rozetiyle motor aynı sayıyı görür).
//!
//! ŞARTLILIK (yol haritası 2.2 hükmü): v3 `macdCalc` signal hattını
//! null-iken-sıfır-dolgulu seriden EMA'lar (standarttan sapma) — birebir
//! port YANLIŞ olur. Bizim `compute_macd` ilk geçerli MACD'den tohumlanır
//! (düzeltilmiş); fark `dca_signals` kilit testiyle belgeli
//! (`macd_signal_differs_from_v3_zero_padded_quirk`). Float sınır-görünümde
//! sorun değil (para değil — f64).
//!
//! İki bilinçli sapma (not düşüldü): kline kaynağı v3'te Binance, bizde
//! BYBIT (linear → düşüşte spot — risk ucuyla aynı kural); bilinmeyen `set`
//! jetonu v3 sessizce atlar, biz 422 döneriz (yazım hatası gizlenmesin).

use axum::extract::{Query, State};
use axum::Json;
use dca_exchange::VenueAdapter;
use dca_signals::types::BarData;
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::state::{now_ms, AppState};

pub enum IndReq {
    Ema(usize),
    Sma(usize),
    Rsi(usize),
    Bb(usize),
    Macd,
}

/// `set` jetonu ayrıştırır (v3 `EMA20,RSI14,MACD,BB20` biçimi).
pub fn parse_set(set: &str) -> Result<Vec<IndReq>, String> {
    let mut out = Vec::new();
    for tok in set.split(',').map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()) {
        if tok == "MACD" {
            out.push(IndReq::Macd);
            continue;
        }
        let (req, num) = if let Some(n) = tok.strip_prefix("EMA") {
            (true, n)
        } else if let Some(n) = tok.strip_prefix("SMA") {
            (false, n)
        } else {
            (false, "")
        };
        if !num.is_empty() {
            let p: usize = num.parse().map_err(|_| format!("gecersiz periyot: '{tok}'"))?;
            if p == 0 || p > 500 {
                return Err(format!("periyot aralik disi (1-500): '{tok}'"));
            }
            out.push(if req { IndReq::Ema(p) } else { IndReq::Sma(p) });
            continue;
        }
        if let Some(n) = tok.strip_prefix("RSI") {
            let p: usize = n.parse().map_err(|_| format!("gecersiz periyot: '{tok}'"))?;
            if p == 0 || p > 500 {
                return Err(format!("periyot aralik disi (1-500): '{tok}'"));
            }
            out.push(IndReq::Rsi(p));
            continue;
        }
        if let Some(n) = tok.strip_prefix("BB") {
            let p: usize = n.parse().map_err(|_| format!("gecersiz periyot: '{tok}'"))?;
            if p == 0 || p > 500 {
                return Err(format!("periyot aralik disi (1-500): '{tok}'"));
            }
            out.push(IndReq::Bb(p));
            continue;
        }
        return Err(format!(
            "bilinmeyen indikator '{tok}' — gecerli: EMA<n> | SMA<n> | RSI<n> | BB<n> | MACD"
        ));
    }
    if out.is_empty() {
        return Err("set bos — orn: set=EMA20,RSI14,MACD,BB20".into());
    }
    Ok(out)
}

fn series_json(v: &[f64]) -> Vec<Value> {
    v.iter()
        .map(|x| if x.is_nan() || !x.is_finite() { Value::Null } else { json!(x) })
        .collect()
}

#[derive(Deserialize)]
pub struct IndicatorsQuery {
    pub symbol: Option<String>,
    pub interval: Option<String>,
    pub limit: Option<usize>,
    pub set: Option<String>,
    pub venue: Option<String>,
}

pub async fn indicators(
    State(app): State<Arc<AppState>>,
    Query(q): Query<IndicatorsQuery>,
) -> Result<Json<Value>, axum::response::Response> {
    let symbol = q.symbol.map(|s| s.trim().to_uppercase()).filter(|s| !s.is_empty()).unwrap_or_else(|| "BTCUSDT".into());
    let iv = crate::engine::bybit_interval(q.interval.as_deref().unwrap_or("1h")).map_err(crate::err422)?;
    let limit = q.limit.unwrap_or(300).clamp(30, 1000);
    let reqs = parse_set(q.set.as_deref().unwrap_or("EMA20,RSI14")).map_err(crate::err422)?;

    // Kline kaynagi: secili venue (varsayilan BYBIT) linear → dususte spot.
    let venue = q.venue.map(|v| v.to_uppercase()).unwrap_or_else(|| "BYBIT".into());
    let adapter: &dyn VenueAdapter = match venue.as_str() {
        "BINANCE" => &app.binance,
        "OKX" => &app.okx,
        _ => &app.bybit,
    };
    let okx_iv = if venue == "OKX" {
        Some(dca_exchange::okx::okx_bar(&iv).map_err(crate::err422)?)
    } else {
        None
    };
    let kl = async {
        let linear = adapter
            .klines("linear", &symbol, okx_iv.as_deref().unwrap_or(&iv), limit)
            .await;
        match linear {
            Ok(k) if k.len() >= 10 => Ok(k),
            _ => adapter.klines("spot", &symbol, okx_iv.as_deref().unwrap_or(&iv), limit).await,
        }
    }
    .await
    .map_err(crate::err422)?;
    if kl.len() < 10 {
        return Err(crate::err422(format!("yetersiz bar: {} (min 10)", kl.len())));
    }
    let mut bars: Vec<BarData> = Vec::with_capacity(kl.len());
    let mut times: Vec<i64> = Vec::with_capacity(kl.len());
    for k in &kl {
        let (Ok(o), Ok(h), Ok(l), Ok(c), Ok(v)) =
            (k.o.parse::<f64>(), k.h.parse::<f64>(), k.l.parse::<f64>(), k.c.parse::<f64>(), k.v.parse::<f64>())
        else {
            continue; // bozuk bar atlanır (kapı-1 ruhu); times hizalı tutulur
        };
        bars.push(BarData { t: k.t, o, h, l, c, v });
        times.push(k.t);
    }
    if bars.len() < 10 {
        return Err(crate::err422(format!("cozulebilen bar yetersiz: {}", bars.len())));
    }

    let mut ind = serde_json::Map::new();
    for r in &reqs {
        match r {
            IndReq::Ema(p) => {
                ind.insert(format!("EMA{p}"), series_json(&dca_signals::compute_ema(&bars, *p)).into());
            }
            IndReq::Sma(p) => {
                ind.insert(format!("SMA{p}"), series_json(&dca_signals::compute_sma(&bars, *p)).into());
            }
            IndReq::Rsi(p) => {
                ind.insert(format!("RSI{p}"), series_json(&dca_signals::compute_rsi(&bars, *p)).into());
            }
            IndReq::Bb(p) => {
                ind.insert(format!("BBU{p}"), series_json(&dca_signals::compute_bollinger(&bars, *p, 2.0, "upper")).into());
                ind.insert(format!("BBL{p}"), series_json(&dca_signals::compute_bollinger(&bars, *p, 2.0, "lower")).into());
                ind.insert(format!("BBM{p}"), series_json(&dca_signals::compute_sma(&bars, *p)).into());
            }
            IndReq::Macd => {
                // v3 sabitleri aynen: 12/26/9.
                ind.insert("MACD".into(), series_json(&dca_signals::compute_macd(&bars, 12, 26, 9, "macd")).into());
                ind.insert(
                    "MACDSignal".into(),
                    series_json(&dca_signals::compute_macd(&bars, 12, 26, 9, "signal")).into(),
                );
                ind.insert("MACDHist".into(), series_json(&dca_signals::compute_macd(&bars, 12, 26, 9, "hist")).into());
            }
        }
    }
    Ok(Json(json!({
        "symbol": symbol,
        "interval": q.interval.as_deref().unwrap_or("1h"),
        "venue": venue,
        "times": times,
        "bars": bars.len(),
        "indicators": ind,
        "at": now_ms(),
        "note": "MACD signal ilk gecerli MACD'den tohumlanir (v3 sifir-dolgulu quirk duzeltildi — kilit test belgeli); kline kaynagi BYBIT.",
    })))
}

#[cfg(test)]
mod indicators_tests {
    use super::*;

    #[test]
    fn set_parsing_matches_v3_tokens() {
        let reqs = parse_set("EMA20,RSI14,MACD,BB20").unwrap();
        assert_eq!(reqs.len(), 4);
        assert!(parse_set("ema12 , sma5").unwrap().len() == 2);
        assert!(parse_set("STOCH14").is_err());
        assert!(parse_set("EMA0").is_err());
        assert!(parse_set("").is_err());
    }

    #[test]
    fn nan_becomes_json_null() {
        let v = series_json(&[1.0, f64::NAN, f64::INFINITY]);
        assert_eq!(v, vec![json!(1.0), Value::Null, Value::Null]);
    }
}
