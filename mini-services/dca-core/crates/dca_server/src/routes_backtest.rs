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

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use dca_math::fraction::Fraction;
use dca_strategy::{run_backtest_with_mmr_cancellable, BotConfig};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

use crate::engine::bybit_interval;
use crate::routes_common::{err422, fill_marks_json, parse_kind};
use crate::{default_interval, default_start_cash};
use crate::state::{now_ms, persist_event, AppState};
use dca_math::Instrument;

const MAX_CONCURRENT_BACKTEST_JOBS: usize = 4;

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
    #[serde(default)]
    request_id: Option<String>,
}

pub(crate) struct BacktestJobGuard {
    pub(crate) cancellation: Arc<AtomicBool>,
    id: Uuid,
    registry: Arc<std::sync::Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,
}

pub(crate) fn parse_job_id(raw: Option<String>) -> Result<Option<Uuid>, axum::response::Response> {
    raw.map(|value| Uuid::parse_str(&value).map_err(|_| err422("request_id UUID biçiminde olmalı")))
        .transpose()
}

impl Drop for BacktestJobGuard {
    fn drop(&mut self) {
        self.registry.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).remove(&self.id);
    }
}

pub(crate) fn register_job(
    app: &Arc<AppState>,
    id: Option<Uuid>,
) -> Result<BacktestJobGuard, axum::response::Response> {
    let id = id.unwrap_or_else(Uuid::new_v4);
    let cancellation = Arc::new(AtomicBool::new(false));
    let mut jobs = app.backtest_jobs.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if jobs.contains_key(&id) {
        return Err((StatusCode::CONFLICT, Json(json!({"error":"backtest request_id is already active"}))).into_response());
    }
    if jobs.len() >= MAX_CONCURRENT_BACKTEST_JOBS {
        return Err((StatusCode::TOO_MANY_REQUESTS, Json(json!({"error":"backtest capacity reached"}))).into_response());
    }
    jobs.insert(id, cancellation.clone());
    drop(jobs);
    Ok(BacktestJobGuard { cancellation, id, registry: app.backtest_jobs.clone() })
}

fn cancellation_requested(cancellation: Option<&Arc<AtomicBool>>) -> bool {
    cancellation.is_some_and(|flag| flag.load(Ordering::Relaxed))
}

async fn wait_for_cancellation(flag: Arc<AtomicBool>) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Drop an in-flight async market-data request as soon as the user cancels it.
/// `None` keeps compatibility for callers that don't register a cancellable job.
pub(crate) async fn await_job_or_cancel<F, T>(
    future: F,
    cancellation: Option<Arc<AtomicBool>>,
) -> Result<T, axum::response::Response>
where
    F: Future<Output = T>,
{
    let Some(flag) = cancellation else {
        return Ok(future.await);
    };
    tokio::select! {
        biased;
        _ = wait_for_cancellation(flag.clone()) => Err(cancellation_response()),
        result = future => {
            if flag.load(Ordering::Relaxed) {
                Err(cancellation_response())
            } else {
                Ok(result)
            }
        }
    }
}

pub(crate) fn cancellation_response() -> axum::response::Response {
    (StatusCode::CONFLICT, Json(json!({"error":"backtest cancelled","code":"BACKTEST_CANCELLED"}))).into_response()
}

pub(crate) async fn cancel_backtest(
    State(app): State<Arc<AppState>>,
    Path(raw_id): Path<String>,
) -> axum::response::Response {
    let id = match Uuid::parse_str(&raw_id) {
        Ok(id) => id,
        Err(_) => return err422("request_id UUID biçiminde olmalı"),
    };
    let jobs = app.backtest_jobs.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    match jobs.get(&id) {
        Some(flag) => {
            flag.store(true, Ordering::Relaxed);
            (StatusCode::ACCEPTED, Json(json!({"ok":true,"cancelRequested":true}))).into_response()
        }
        None => (StatusCode::OK, Json(json!({"ok":true,"cancelRequested":false}))).into_response(),
    }
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

    let job = register_job(&app, parse_job_id(req.request_id)?)?;
    let cancellation = Some(job.cancellation.clone());

    let category = if kind.is_futures() { "linear" } else { "spot" };
    // P4-3d: backtest market verisi seçili venue'den (kağıt/PAPER hariç —
    // PAPER'ın kline akışı yok, Bybit'e düşer).
    let venue = req.venue.unwrap_or_else(|| "BYBIT".into()).to_uppercase();
    let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
    let iv = bybit_interval(&req.interval).map_err(|e| err422(e))?;
    let bars_result = await_job_or_cancel(
        fetch_backtest_klines(adapter, &venue, category, &req.symbol, &iv, req.bars.min(1000)),
        cancellation.clone(),
    ).await?;
    if cancellation_requested(cancellation.as_ref()) { return Err(cancellation_response()); }
    let bars = bars_result.map_err(err422)?;
    if bars.is_empty() {
        return Err(err422("no market data"));
    }
    let insts = await_job_or_cancel(adapter.instruments(category, Some(&req.symbol)), cancellation.clone()).await?
        .unwrap_or_default();
    if cancellation_requested(cancellation.as_ref()) { return Err(cancellation_response()); }
    let inst = {
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
        match await_job_or_cancel(adapter.funding_history(&req.symbol, 200), cancellation.clone()).await? {
            Ok(v) => (v, None),
            Err(e) => (
                Vec::new(),
                Some(format!("Funding geçmişi alınamadı ({e}) — backtest funding=0 varsayıldı; gerçek fonlama maliyeti metriklerde YOK")),
            ),
        }
    } else {
        (Vec::new(), None)
    };
    if cancellation_requested(cancellation.as_ref()) { return Err(cancellation_response()); }
    if let Some(w) = &funding_warning {
        persist_event(&app, &req.symbol, "WARN", "BACKTEST", w).await;
    }
    let (mmr_brackets, mmr_warning): (Option<Vec<dca_math::MmrBracket>>, Option<String>) =
        if kind.is_futures() {
            match await_job_or_cancel(adapter.mmr_brackets(&req.symbol, None), cancellation.clone()).await? {
                Ok(brackets) if !brackets.is_empty() => (Some(brackets), None),
                Ok(_) => (None, Some("Borsa MMR kademesi boş döndü — likidasyon hesabı %0,5 varsayılanıyla yapıldı".into())),
                Err(error) => (None, Some(format!("Borsa MMR kademesi alınamadı ({error}) — likidasyon hesabı %0,5 varsayılanıyla yapıldı"))),
            }
        } else {
            (None, None)
        };
    if cancellation_requested(cancellation.as_ref()) { return Err(cancellation_response()); }
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
    let mmr2 = mmr_brackets.clone();
    let cancellation_for_task = cancellation.clone();
    let result_pair = tokio::task::spawn_blocking(move || {
        // Keep the cancellation registration alive even if the browser drops
        // its request while the blocking worker is still computing.
        let _job_guard = job;
        let result = run_backtest_with_mmr_cancellable(
            &bars_n, &inst, &cfg, start, session, &funding, mmr_brackets.as_deref(), cancellation_for_task.as_deref(),
        )?;
        let result_opt = if is_realistic {
            Some(run_backtest_with_mmr_cancellable(
                &bars_n2, &inst2, &cfg_opt, start2, session, &funding2, mmr2.as_deref(), cancellation_for_task.as_deref(),
            )?)
        } else {
            None
        };
        Ok::<_, String>((result, result_opt))
    })
    .await
    .map_err(|e| err422(e.to_string()))?
    .map_err(|error| if error == "backtest cancelled" { cancellation_response() } else { err422(error) })?;
    let (result, result_opt) = result_pair;
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
        "mmrWarning": mmr_warning,
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
