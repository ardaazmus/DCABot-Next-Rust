//! `POST /api/backtest` — geçmiş barlar üzerinde strateji koşumu.
//!
//! v3 eşlemesi: `POST /api/backtest` (v3 ile aynı sözleşme).
//!
//! Dürüstlük notları (uydurma YOK):
//! - Market verisi GERÇEK venue akışıdır (`adapters.klines`); PAPER akışı
//!   yoktur → venue PAPER ise Bybit verisine düşer (yanıtta `venue` alanı
//!   hangi verinin kullanıldığını açıklar).
//! - `fundingWarning: null` DEGILDIR — veri yoksa Task 18 N-5 kuralıyla
//!   dürüst uyarı metni döner.
//! - `macroCalendarStale`: takvim bayatsa Gate 7 koruması fiilen kapalıdır;
//!   yanıt bunu açıkça işaretler.
//! - `realismCompare`: aynı koşunun iyimser varyantı — UI farkı hesaplar.
//! - `noDealReasons`: boş sonuçların makine-okunur nedenleri (motor sayaçları).
//! - Equity eğrisi 500 noktaya downsample edilir; fill işaretçileri ilk 400
//!   fill ile sınırlıdır (`routes_common::fill_marks_json`).
//! - `fetch_backtest_klines` OKX bar adlarını çevirir (60→1H, D→1D); aynı
//!   yardımcı `create_bot` LIVE sertifika kapısı tarafından da kullanılır.
//!
//! Yardımcı `default_*` serde varsayılanları main.rs ile aynı davranır
//! (farklı modülde aynı-isimli özel kopya — davranış kopyası, uydurma yok).

use axum::extract::State;
use axum::response::IntoResponse;
use axum::Json;
use dca_math::fraction::Fraction;
use dca_strategy::{run_backtest, BotConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use std::sync::Arc;

use crate::engine::bybit_interval;
use crate::routes_common::{err422, fill_marks_json, parse_kind};
use crate::{default_interval, default_start_cash};
use crate::state::{now_ms, persist_event, AppState};
use dca_math::Instrument;

#[derive(Deserialize)]
pub(crate) struct BacktestReq {
    kind: String,
    symbol: String,
    #[serde(default = "default_interval")]
    interval: String,
    #[serde(default = "default_bt_bars")]
    bars: usize,
    config: Option<Value>,
    #[serde(default = "default_start_cash")]
    start_cash: String,
    /// P4-3d: backtest market verisi bu venue'den çekilir (BYBIT varsayılan).
    #[serde(default)]
    venue: Option<String>,
}

fn default_bt_bars() -> usize {
    720
}

pub(crate) async fn backtest(State(app): State<Arc<AppState>>, Json(req): Json<BacktestReq>) -> Result<axum::response::Response, axum::response::Response> {
    // bars=0 borsa katmanında varsayılan limite düşerdi — sessiz bar üretimi
    // yerine dürüst 422 (fail-closed; boş koşu anlamsızdır).
    if req.bars == 0 {
        return Err(err422("bars en az 1 olmalı"));
    }
    let kind = match parse_kind(&req.kind) { Ok(k) => k, Err(e) => return Err(err422(e)) };
    let mut cfg = match &req.config {
        Some(v) => match serde_json::from_value::<BotConfig>(v.clone()) {
            Ok(c) => c,
            Err(e) => return Err(err422(format!("config: {e}"))),
        },
        None => BotConfig::default(),
    };
    cfg.kind = kind;
    cfg.symbol = req.symbol.clone();

    if let Err(e) = cfg.entry_trigger.validate() {
        return Err(err422(format!("entry_trigger: {e}")));
    }
    // Composite signal tree validation (3Commas-style, dca_signals)
    if let Some(sc) = &cfg.entry_signal {
        if let Err(e) = dca_signals::validate_signal_node(&sc.root) {
            return Err(err422(format!("entry_signal: {e}")));
        }
    }
    // Task 19 — Pine doğrulaması (backtest tarafı da aynı fail-closed kapıdan geçer)
    if let Err(e) = cfg.validate_pine() {
        return Err(err422(e));
    }

    let category = if kind.is_futures() { "linear" } else { "spot" };
    // P4-3d: backtest market verisi seçili venue'den (kağıt/PAPER hariç —
    // PAPER'ın kline akışı yok, Bybit'e düşer).
    let venue = req.venue.unwrap_or_else(|| "BYBIT".into()).to_uppercase();
    let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
    let iv = bybit_interval(&req.interval).map_err(|e| err422(e))?;
    let bars = fetch_backtest_klines(adapter, &venue, category, &req.symbol, &iv, req.bars.min(1000))
        .await
        .map_err(|e| err422(e))?;
    if bars.is_empty() {
        return Err(err422("no market data"));
    }
    let inst = {
        let insts = adapter.instruments(category, Some(&req.symbol)).await.unwrap_or_default();
        insts.into_iter().next().unwrap_or_else(|| {
            let mut i = Instrument::default();
            i.symbol = req.symbol.clone();
            i
        })
    };
    let start = Fraction::from_decimal_str(&req.start_cash).map_err(|e| err422(e))?;
    let session = dca_math::sessions::SessionType::Crypto24_7;
    // Real funding history for futures backtests (seçili venue, linear).
    // Task 18 N-5: fetch hatası artık yutulmaz — WARN event + yanıtta dürüst flag.
    let (funding, funding_warning): (Vec<(i64, String)>, Option<String>) = if kind.is_futures() {
        match adapter.funding_history(&req.symbol, 200).await {
            Ok(v) => (v, None),
            Err(e) => (
                Vec::new(),
                Some(format!("Funding geçmişi alınamadı ({e}) — backtest funding=0 varsayıldı; gerçek fonlama maliyeti metriklerde YOK")),
            ),
        }
    } else {
        (Vec::new(), None)
    };
    if let Some(w) = &funding_warning {
        persist_event(&app, &req.symbol, "WARN", "BACKTEST", w).await;
    }
    let bars_n: Vec<dca_kernel::Bar> = bars
        .iter()
        .map(|k| dca_kernel::Bar { t: k.t, o: k.o.clone(), h: k.h.clone(), l: k.l.clone(), c: k.c.clone(), v: k.v.clone() })
        .collect();
    // Task 19 — dolum gerçekçiliği ayrıştırıcı karşılaştırma: aynı config
    // İYİMSER dolum modeliyle (teğet dokunuş dolar) ikinci kez koşturulur.
    // Delta dürüstçe raporlanır — "backtest iyi ama canlıda emirler dolmuyor"
    // tuzağını görünür kılar. İyimser koşulan config zaten Optimistic ise
    // karşılaştırma tekrarı yoktur (tek koşu döner).
    let is_realistic = cfg.risk_policy.fill_model == dca_kernel::FillModel::Realistic;
    let cfg_opt = {
        let mut c = cfg.clone();
        c.risk_policy.fill_model = dca_kernel::FillModel::Optimistic;
        c
    };
    let bars_n2 = bars_n.clone();
    let inst2 = inst.clone();
    let start2 = start.clone();
    let funding2 = funding.clone();
    let result = tokio::task::spawn_blocking(move || {
        run_backtest(&bars_n, &inst, &cfg, start, session, &funding)
    })
        .await
        .map_err(|e| err422(e.to_string()))?
        .map_err(|e| err422(e))?;
    let result_opt = if is_realistic {
        Some(
            tokio::task::spawn_blocking(move || {
                run_backtest(&bars_n2, &inst2, &cfg_opt, start2, session, &funding2)
            })
            .await
            .map_err(|e| err422(e.to_string()))?
            .map_err(|e| err422(e))?,
        )
    } else {
        None
    };
    // Downsample equity curve to <= 500 points
    let fill_marks = fill_marks_json(&result.fills);
    let step = (result.equity_curve.len() / 500).max(1);
    let curve: Vec<Value> = result
        .equity_curve
        .into_iter()
        .step_by(step)
        .map(|p| json!({"ts": p.ts, "equity": p.equity}))
        .collect();
    let realism_compare = result_opt.map(|o| {
        json!({
            // gerçekçi koşu (mevcut config) metrikleri "metrics" içinde;
            // iyimser karşılaştırma koşusu burada — UI farkı hesaplar.
            "optimisticMetrics": o.metrics,
            "optimisticEndEquity": o.end_equity,
            "optimisticFills": o.fills.len(),
        })
    });
    Ok(axum::Json(json!({
        "metrics": result.metrics,
        "equityCurve": curve,
        "startEquity": result.start_equity,
        "endEquity": result.end_equity,
        "fills": result.fills.len(),
        "gateLogs": result.gate_logs.len(),
        // Task 18 N-5: fetch hatasında "null değil" — dürüst uyarı metni.
        "fundingWarning": funding_warning,
        // Task 20 L5 — makro takvim tazeliği (bayatsa Gate 7 koruması fiilen kapalı)
        "macroCalendarStale": dca_math::macro_calendar::is_stale(now_ms()),
        // Task 19 — Pine strateji uyarıları (pyramiding vb.) + realism karşılaştırması
        "pineWarnings": result.pine_warnings,
        // Task 20 — derleme notları + plot serileri
        "pineNotes": result.pine_notes,
        "pinePlots": result.pine_plots,
        // Task 22 — boş sonuçların makine-okunur nedenleri (motor sayaçları).
        "noDealReasons": result.no_deal_reasons,
        "realismCompare": realism_compare,
        // Task 21 — grafik verileri: backtest'in KULLANDIĞI tam bar penceresi
        // (plot serileri bar diziniyle hizalanır) + fill işaretçileri.
        "bars": bars,
        "fillMarks": fill_marks,
        // P4-3d: koşunun market verisi bu venue'den geldi.
        "venue": venue,
    }))
    .into_response())
}

/// P4-3d: venue-duyarlı kline çekimi — OKX'in bar adları Bybit token'lerinden
/// farklıdır (60→1H, D→1D); PAPER'ın akışı yok → Bybit'e düşer.
pub(crate) async fn fetch_backtest_klines(
    adapter: &dyn dca_exchange::VenueAdapter,
    venue: &str,
    category: &str,
    symbol: &str,
    iv: &str,
    limit: usize,
) -> Result<Vec<dca_exchange::Kline>, String> {
    if venue == "OKX" {
        let oiv = dca_exchange::okx::okx_bar(iv)?;
        adapter.klines(category, symbol, oiv, limit).await
    } else {
        adapter.klines(category, symbol, iv, limit).await
    }
}
