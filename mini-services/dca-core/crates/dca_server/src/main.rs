//! dca_server — Quantitative engine HTTP/SSE server (axum) on port 3002.
//! REST + SSE contract consumed by the Next.js terminal via the gateway
//! (`/api/*?XTransformPort=3002`).

mod ai;
mod bnb;
mod brackets;
mod dryrun;
mod forensics;
mod engine;
mod fleet;
mod indicators;
mod memory;
mod multistatus;
mod risk;
mod routes_backtest;
mod routes_calc;
mod routes_common;
mod status;
mod thin;
mod state;
mod store;
mod deals;

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::middleware;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::IntoResponse;
use axum::routing::{delete, get, patch, post};
use axum::{Json, Router};
use dca_exchange::{validate_key_permissions, KeyPermissions, VenueAdapter, VenueCreds, WithdrawalGate};
use dca_kernel::GuardAction;
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use dca_strategy::dca::DcaPhase;
use dca_strategy::{certify, certify_verdict, run_backtest, BotConfig, StrategyKind};
use futures_util::FutureExt;
use serde::Deserialize;
use serde_json::{json, Value};
use crate::routes_common::{err403, err404, err422, fill_marks_json, is_blacklisted, normalize_pair, parse_kind};
use state::{AppState, SseEvent};
use std::collections::{HashMap, HashSet};
use std::convert::Infallible;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::Mutex as AsyncMutex;
use store::{Db, Vault};
use tower_http::cors::{AllowOrigin, CorsLayer};
use uuid::Uuid;

// -----------------------------------------------------------------------
// SEC-04 Rate Limiter — basit per-IP token bucket (ağ içinde tek sunucu).
// 400 istek / 10 sn (40 req/s), local dashboard burst'leri için genişletildi. Aşınca 429 Too Many Requests.
// -----------------------------------------------------------------------
const RATE_LIMIT_MAX: usize = 400;
const RATE_LIMIT_WINDOW_SECS: u64 = 10;
const RATE_LIMIT_MAX_BUCKETS: usize = 10_000;

#[derive(Default, Clone)]
struct RateLimiter {
    buckets: HashMap<String, Vec<Instant>>,
}

impl RateLimiter {
    fn check(&mut self, key: &str) -> bool {
        let now = Instant::now();
        let window_start = now - std::time::Duration::from_secs(RATE_LIMIT_WINDOW_SECS);

        // Yeni kimlikler için önce süresi dolmuş kovaları temizle; doluysa
        // yeni kimliği reddet ki saldırgan anahtarla bellek sınırsız büyümesin.
        if !self.buckets.contains_key(key) && self.buckets.len() >= RATE_LIMIT_MAX_BUCKETS {
            self.buckets.retain(|_, v| {
                v.iter().any(|t| *t > window_start)
            });
            if self.buckets.len() >= RATE_LIMIT_MAX_BUCKETS {
                return false;
            }
        }

        let entry = self.buckets.entry(key.to_string()).or_default();
        entry.retain(|t| *t > window_start);
        if entry.len() < RATE_LIMIT_MAX {
            entry.push(now);
            true
        } else {
            false
        }
    }
}

/// The engine is bound to loopback and sits behind the local Next proxy. The
/// peer address is supplied by the TCP connection; forwarded headers are
/// client-controlled and must never define a rate-limit identity.
fn rate_limit_key(req: &axum::extract::Request) -> String {
    req.extensions()
        .get::<axum::extract::ConnectInfo<SocketAddr>>()
        .map(|connect| connect.0.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

async fn acquire_bot_poll_guard(app: &AppState, bot_id: &str) -> tokio::sync::OwnedMutexGuard<()> {
    let lock = {
        let mut locks = app.poll_locks.lock().await;
        locks.entry(bot_id.to_string()).or_insert_with(|| Arc::new(AsyncMutex::new(()))).clone()
    };
    lock.lock_owned().await
}

fn normalized_live_market(venue: &str, symbol: &str) -> (String, String) {
    (venue.trim().to_ascii_uppercase(), symbol.trim().to_ascii_uppercase())
}

fn live_status_requires_ownership(status: &str, exposure: bool, open_orders: bool, pending_actions: bool) -> bool {
    !status.eq_ignore_ascii_case("STOPPED") || exposure || open_orders || pending_actions
}

fn live_runtime_has_ownership(rt: &state::BotRuntime) -> bool {
    let main_exposure = !rt.stepper.position.qty.is_zero();
    let hedge_exposure = rt.hedge_stepper.as_ref().is_some_and(|stepper| !stepper.position.qty.is_zero());
    live_status_requires_ownership(
        &rt.status,
        main_exposure || hedge_exposure,
        !rt.live_open_orders.is_empty(),
        !rt.pending_live_intents.is_empty() || !rt.pending_engine_actions.is_empty(),
    )
}

fn duplicate_live_markets<'a>(
    rows: impl IntoIterator<Item = (&'a str, &'a str, &'a str, &'a str)>,
) -> HashSet<(String, String)> {
    let mut counts = HashMap::<(String, String), usize>::new();
    for (venue, mode, symbol, status) in rows {
        if mode.eq_ignore_ascii_case("LIVE") && matches!(status, "RUNNING" | "PAUSED" | "FROZEN") {
            *counts.entry(normalized_live_market(venue, symbol)).or_default() += 1;
        }
    }
    counts.into_iter().filter_map(|(market, count)| (count > 1).then_some(market)).collect()
}

fn competing_live_bot(
    runtimes: &HashMap<String, state::BotRuntime>,
    except_id: &str,
    venue: &str,
    symbol: &str,
) -> Option<String> {
    let requested = normalized_live_market(venue, symbol);
    runtimes.iter().find_map(|(id, rt)| {
        let existing = normalized_live_market(&rt.venue, &rt.cfg.symbol);
        (id != except_id
            && rt.mode.eq_ignore_ascii_case("LIVE")
            && existing == requested
            && live_runtime_has_ownership(rt))
            .then(|| id.clone())
    })
}

async fn lock_live_ownership(
    app: &AppState,
    except_id: &str,
    venue: &str,
    symbol: &str,
) -> Result<tokio::sync::OwnedMutexGuard<()>, String> {
    let guard = app.live_ownership_lock.clone().lock_owned().await;
    let conflict = {
        let runtimes = app.runtimes.lock().await;
        competing_live_bot(&runtimes, except_id, venue, symbol)
    };
    if let Some(id) = conflict {
        let (venue, symbol) = normalized_live_market(venue, symbol);
        return Err(format!(
            "LIVE market ownership conflict: bot {id} already owns or may own {venue} {symbol}; account-wide position data cannot be safely attributed to a second bot"
        ));
    }
    Ok(guard)
}

#[tokio::main]
async fn main() {
    // Ingest .env if present (root or local) before setting up adapters/db
    for env_path in &[".env", "../../.env", "../.env"] {
        if let Ok(content) = std::fs::read_to_string(env_path) {
            for line in content.lines() {
                let trimmed = line.trim();
                if !trimmed.is_empty() && !trimmed.starts_with('#') {
                    if let Some((k, v)) = trimmed.split_once('=') {
                        let k = k.trim();
                        let v = v.trim().trim_matches('\'').trim_matches('"');
                        if std::env::var(k).is_err() {
                            std::env::set_var(k, v);
                        }
                    }
                }
            }
            break;
        }
    }
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();
    let db_path = std::env::var("DCA_DB_PATH").unwrap_or_else(|_| {
        let candidates = [
            "db/dca-core.sqlite3",
            "../../db/dca-core.sqlite3",
        ];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                return c.to_string();
            }
        }
        std::fs::create_dir_all("db").ok();
        "db/dca-core.sqlite3".to_string()
    });
    tracing::info!("Opening database at {}", db_path);
    let db = Db::open(&db_path).expect("sqlite");
    let vault = Vault::new(&db).expect("vault");
    let (app_state, rx) = AppState::new(db, vault);

    // Background supervisor: poll running bots every 20s. All poll tasks are
    // owned by this task so shutdown can stop scheduling and drain order work.
    let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    let supervisor_handle = {
        let st = app_state.clone();
        let mut shutdown_rx = shutdown_rx;
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(std::time::Duration::from_secs(20));
            let mut poll_tasks = tokio::task::JoinSet::<String>::new();
            let mut in_flight = HashSet::<String>::new();
            loop {
                tokio::select! {
                    _ = tick.tick() => {
                        let ids: Vec<(String, String)> = {
                            let rts = st.runtimes.lock().await;
                            // PAUSED/STOPPED botlar da poll edilir: açık emirlerin fill takibi sürer.
                            rts.iter()
                                .filter(|(_, rt)| matches!(rt.status.as_str(), "RUNNING" | "FROZEN" | "PAUSED" | "STOPPED"))
                                .map(|(id, rt)| (id.clone(), rt.venue.clone()))
                                .collect()
                        };
                        for (id, _venue) in ids {
                            if !in_flight.insert(id.clone()) {
                                continue;
                            }
                            let st2 = st.clone();
                            let id2 = id.clone();
                            // Bot-kimliğinden türetilen jitter poll'u dağıtır.
                            let jitter = (id2.bytes().map(|b| b as u64).sum::<u64>() % 1200) as u64;
                            poll_tasks.spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(jitter)).await;
                                let outcome = std::panic::AssertUnwindSafe(
                                    engine::poll_and_advance(&st2, &id2, &st2.sse),
                                )
                                .catch_unwind()
                                .await;
                                match outcome {
                                    Ok(Ok(_)) => {}
                                    Ok(Err(e)) => {
                                        tracing::warn!("bot {id2} poll error: {e}");
                                        state::persist_event(&st2, &id2, "ERROR", "POLL", &e).await;
                                    }
                                    Err(_) => {
                                        let e = "poll task panicked";
                                        tracing::error!("bot {id2} {e}");
                                        state::persist_event(&st2, &id2, "ERROR", "POLL", e).await;
                                    }
                                }
                                id2
                            });
                        }
                    }
                    joined = poll_tasks.join_next(), if !poll_tasks.is_empty() => {
                        match joined {
                            Some(Ok(id)) => { in_flight.remove(&id); }
                            Some(Err(e)) => {
                                tracing::error!("bot poll task join failed: {e}");
                                in_flight.clear();
                            }
                            None => {}
                        }
                    }
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            break;
                        }
                    }
                }
            }

            tracing::info!("[shutdown] New bot polls stopped; draining in-flight bot work.");
            let drain = async {
                while let Some(joined) = poll_tasks.join_next().await {
                    match joined {
                        Ok(id) => { in_flight.remove(&id); }
                        Err(e) => tracing::error!("bot poll task failed during drain: {e}"),
                    }
                }
            };
            if tokio::time::timeout(std::time::Duration::from_secs(45), drain).await.is_err() {
                tracing::error!("[shutdown] Poll drain exceeded 45s; aborting remaining tasks before snapshot persistence.");
                poll_tasks.abort_all();
                while poll_tasks.join_next().await.is_some() {}
            }
            tracing::info!("[shutdown] Bot poll tasks drained.");
        })
    };

    // Faz 0.2/0.3 — şalter durumunu olay akışından geri yükle (restart-safe),
    // sonra kalıcı RUNNING botları kur (ilk poll şalterli başlar).
    {
        let st = app_state.clone();
        state::AppState::rehydrate_halts(&st).await;
        restore_bots(&st).await;
        seed_pine_library(&st).await;
        spawn_notifiers(&st);
    }

    let sse_state = app_state.clone();
    let sse_rx = Arc::new(rx);

    // SEC-04: rate limiter durumu (tüm yönetim rotaları ortak kotalar).
    let rate_limiter = Arc::new(AsyncMutex::new(RateLimiter::default()));
    let sse_rate_limiter = rate_limiter.clone();

    // SEC-01: Yönetim API kimlik doğrulaması — DCA_API_KEY env'de zorunlu.
    // Eksikse anahtar boş bırakılır ve tüm authenticated istekler 401 döner
    // (fail-closed — çökme riski yok, ama yönetim erişimi engellenir).
    // Canlı ortamda mutlaka ayarlanmalıdır.
    let api_key = std::env::var("DCA_API_KEY").unwrap_or_default();
    if api_key.is_empty() {
        tracing::warn!("DCA_API_KEY ayarlanmamış — tüm authenticated istekler 401 dönecek (SEC-01)");
    }
    let api_key_clone = api_key.clone();

    // SEC-01: /api/health kimlik doğrulaması iSTEMEN (load balancer / monitoring açık erişim).
    let health_routes = Router::new()
        .route("/api/health", get(health))
        .route("/metrics", get(prometheus_metrics));
    // P0-8: TradingView webhook'u auth middleware DIŞINDA — TradingView alert'i
    // Bearer başlığı ekleyemez. Uç kendi savunma katmanlarıyla korunur:
    // sabit-zamanlı per-bot secret + ±60sn replay penceresi + DUPLICATE dedup
    // + pair eşleşmesi + gövde limiti (16KB, SEC-FIND-06).
    let webhook_routes = Router::new()
        .route("/api/webhooks/tradingview", post(tradingview_webhook))
        .layer(axum::extract::DefaultBodyLimit::max(16 * 1024));

    // Tüm yönetim routes'ları kimlik doğrulama altına alınır.
    let api_routes = Router::new()
        .route("/api/summary", get(summary))
        .route("/api/equity", get(equity_curve))
        .route("/api/deals", get(deals::list_deals))
        .route("/api/daily-pnl", get(deals::daily_pnl))
        .route("/api/notifications/config", get(notifications_config))
        .route("/api/notifications/test", post(notifications_test))
        .route("/api/bots", get(list_bots).post(create_bot))
        .route("/api/bots/:id", get(get_bot).patch(patch_bot).delete(delete_bot))
        .route("/api/bots/:id/actions", post(bot_actions))
        .route("/api/bots/:id/fills", get(bot_fills))
        .route("/api/bots/:id/events", get(bot_events))
        .route("/api/events/forensics", get(forensics::events_forensics))
        .route("/api/calc/dca", post(routes_calc::calc_dca))
        .route("/api/calc/grid", post(routes_calc::calc_grid))
        .route("/api/calc/brackets", get(brackets::calc_brackets))
        .route("/api/backtest", post(routes_backtest::backtest))
        .route("/api/backtest/cert", post(backtest_cert))
        .route("/api/signals/preview", post(signals_preview))
    .route("/api/pine/validate", post(pine_validate))
    .route("/api/pine/backtest", post(pine_backtest))
    .route("/api/pine/scripts", get(list_pine_scripts).post(save_pine_script))
    .route("/api/pine/scripts/:id", patch(patch_pine_script).delete(delete_pine_script))
        .route("/api/markets", get(markets))
        .route("/api/tickers", get(tickers))
        .route("/api/klines", get(klines))
        .route("/api/parity", post(parity))
        .route("/api/live/reconcile", get(reconcile))
        .route("/api/live/dryrun", post(dryrun::dryrun))
        .route("/api/live/status", get(status::live_status))
        .route("/api/live/multi-status", get(multistatus::live_multi_status))
        .route("/api/fleet/overview", get(fleet::fleet_overview))
        .route("/api/fleet/risk", get(risk::fleet_risk))
        .route("/api/indicators", get(indicators::indicators))
        .route("/api/live/selfcheck", post(selfcheck))
        .route("/api/live/killswitch", get(get_killswitch).post(set_killswitch))
        .route("/api/live/freeze", get(get_freeze).post(set_freeze))
        .route("/api/live/bnb", get(bnb::live_bnb))
        .route("/api/blacklist", get(list_blacklist).post(add_blacklist))
        .route("/api/blacklist/:pair", delete(remove_blacklist))
        .route("/api/analysis/regime", get(analysis_regime))
        .route("/api/analysis/calibration", post(analysis_calibration))
        .route("/api/analysis/grid-suggest", get(analysis_grid_suggest))
        .route("/api/system/maintenance", get(get_maintenance).post(set_maintenance))
        .route("/api/system/maintenance/simulate", post(thin::simulate_maintenance))
        .route("/api/market/ping", get(thin::market_ping))
        .route("/api/macro/state", get(thin::macro_state))
        .route("/api/orderbook", get(orderbook))
        .route("/api/live/futures-state", get(futures_state))
        .route("/api/market/news", get(market_news))
        .route("/api/market/news-ai", post(ai::news_ai))
        .route("/api/ai/diagnose", post(ai::ai_diagnose))
        .route("/api/ai/providers", get(ai::ai_providers))
        .route("/api/bots/:id/grid", get(bot_grid))
        .route("/api/bots/:id/tick", post(bot_tick))
        .route("/api/bots/:id/memory", get(memory::list_memory).post(memory::generate_memory))
        .route("/api/vault/keys", get(list_keys).post(add_key).route_layer(axum::extract::DefaultBodyLimit::max(64 * 1024)))
        .route("/api/vault/keys/:id", delete(remove_key))
        // SEC-01: kimlik doğrulama middleware'i (yönetim routes'ları).
        .route_layer(middleware::from_fn_with_state(api_key_clone, auth_middleware))
        // SEC-04: rate limiting — 400 istek / 10 sn per istemci (P2-1).
        .layer(
            middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
                let limiter = rate_limiter.clone();
                async move {
                    // The engine only accepts loopback connections from the
                    // local UI proxy. Use the socket peer; X-Forwarded-For is
                    // preserved from browser input and is therefore spoofable.
                    let key = rate_limit_key(&req);
                    let allowed = {
                        let mut l = limiter.lock().await;
                        l.check(&key)
                    };
                    if !allowed {
                        return Err(StatusCode::TOO_MANY_REQUESTS);
                    }
                    Ok(next.run(req).await)
                }
            }),
        )
        // M-15: yönetim uçlarında gövde limiti — 1MB isim gibi aşırı yük reddedilir.
        .layer(axum::extract::DefaultBodyLimit::max(131_072));

    // SEC-FIND-03: SSE akışı yalnız Authorization Bearer başlığıyla doğrulanır.
    let sse_api_key = api_key.clone();
    let sse_routes = Router::new().route(
        "/api/stream",
        get(move |headers: axum::http::HeaderMap| {
            let st = sse_state.clone();
            let key = sse_api_key.clone();
            async move {
                // Bound the number of concurrent streams before allocating a
                // replay query and a new broadcast receiver for the client.
                if st.sse.receiver_count() > 128 {
                    return (StatusCode::SERVICE_UNAVAILABLE, "too many event streams").into_response();
                }
                let rx = sse_rx.resubscribe();
                let auth_header = headers
                    .get("authorization")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|h| h.strip_prefix("Bearer "));
                let provided = auth_header.unwrap_or("");
                if !constant_time_eq_str(provided, &key) {
                    return (StatusCode::UNAUTHORIZED, "unauthorized").into_response();
                }

                let last_event_id = headers
                    .get("last-event-id")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|v| v.parse::<i64>().ok());
                sse_handler(st, rx, last_event_id).await.into_response()
            }
        }),
    ).layer(middleware::from_fn(move |req: axum::extract::Request, next: axum::middleware::Next| {
        let limiter = sse_rate_limiter.clone();
        async move {
            let key = rate_limit_key(&req);
            let allowed = {
                let mut limiter = limiter.lock().await;
                limiter.check(&key)
            };
            if !allowed {
                return Err(StatusCode::TOO_MANY_REQUESTS);
            }
            Ok(next.run(req).await)
        }
    }));

    let app = health_routes
        .merge(api_routes)
        .merge(webhook_routes)
        .merge(sse_routes)
        // SEC-01: CORS çok permisive değil — yalnız erlaubed origin.
        // M-8: başlatıcı UI'ı 127.0.0.1 üzerinde servis eder; tarayıcı origin'i
        // "http://127.0.0.1:3000" olur — localhost alias'ı ile birlikte ikisi
        // de açık (motor doğrudan hedeflenirse preflight reddi kalmasın).
        .layer(
            CorsLayer::new()
                .allow_origin({
                    // Faz 14-D: varsayılan localhost ikilisi + DCA_EXTRA_ORIGINS
                    // (virgülle ayrık) — Tunnel domain'i kod değişikliği olmadan eklenir.
                    let mut origins = vec![
                        "http://localhost:3000".parse::<axum::http::HeaderValue>().unwrap(),
                        "http://127.0.0.1:3000".parse::<axum::http::HeaderValue>().unwrap(),
                    ];
                    if let Ok(extra) = std::env::var("DCA_EXTRA_ORIGINS") {
                        for o in extra.split(',').map(str::trim).filter(|o| !o.is_empty()) {
                            if let Ok(hv) = o.parse::<axum::http::HeaderValue>() {
                                origins.push(hv);
                            }
                        }
                    }
                    AllowOrigin::list(origins)
                })
                .allow_methods([axum::http::Method::GET, axum::http::Method::POST, axum::http::Method::PATCH, axum::http::Method::DELETE])
                .allow_headers([axum::http::header::CONTENT_TYPE, axum::http::header::AUTHORIZATION]),
        )
        .with_state(app_state.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3002").await.expect("bind 3002");
    tracing::info!("dca-core engine listening on :3002");
    // Faz 28 (P2-1): ConnectInfo servis eder — rate limiter gerçek istemci
    // IP'sini (XFF yoksa) okur.
    let serve_result = axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
        .with_graceful_shutdown(shutdown_signal(shutdown_tx.clone()))
        .await
        ;
    if let Err(e) = &serve_result {
        tracing::error!("HTTP server stopped with an error: {e}");
        shutdown_tx.send_replace(true);
    }
    if let Err(e) = supervisor_handle.await {
        tracing::error!("bot supervisor failed during shutdown: {e}");
    }

    // No request or bot poll can mutate runtimes now. Persist one final copy
    // per bot before checkpointing the WAL.
    {
        let runtimes = app_state.runtimes.lock().await;
        for (id, runtime) in runtimes.iter() {
            state::save_runtime_state(&app_state, id, runtime);
        }
    }
    match app_state.db.conn.lock() {
        Ok(conn) => match conn.query_row(
            "PRAGMA wal_checkpoint(TRUNCATE)",
            [],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?)),
        ) {
            Ok((busy, log_frames, checkpointed_frames)) if busy == 0 && checkpointed_frames >= log_frames => {
                tracing::info!("[shutdown] Final WAL checkpoint completed ({checkpointed_frames}/{log_frames} frames).");
            }
            Ok((busy, log_frames, checkpointed_frames)) => {
                tracing::error!("[shutdown] WAL checkpoint incomplete (busy={busy}, checkpointed={checkpointed_frames}/{log_frames}).");
            }
            Err(e) => tracing::error!("[shutdown] WAL checkpoint failed: {e}"),
        },
        Err(e) => tracing::error!("[shutdown] Could not lock database for final checkpoint: {e}"),
    }
    if let Err(e) = serve_result {
        panic!("HTTP server failed: {e}");
    }
}

/// Faz 7: hazır strateji kütüphanesi — sektör-standart klasikler (bütün metin
/// motorda canlı doğrulanmış alt kümeyle yazıldı). İsim zaten varsa atlanır
/// (idempotent); kullanıcı düzenlemesini ezmez.
async fn seed_pine_library(app: &Arc<AppState>) {
    let seeds: Vec<(&str, &str, &str)> = vec![
        ("Şablon: EMA Kesişim", "EMA 9/21 kesişimi — klasik trend takibi", "//@version=5\nstrategy(\"EMA Kesişim\")\nfast = ta.ema(close, 9)\nslow = ta.ema(close, 21)\nif ta.crossover(fast, slow)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(fast, slow)\n    strategy.close_all()"),
        ("Şablon: MACD Sinyal Kesişimi", "MACD/sinyal çizgisi kesişimi", "//@version=5\nstrategy(\"MACD Kesişim\")\nmacdLine = ta.ema(close, 12) - ta.ema(close, 26)\nsignal = ta.ema(macdLine, 9)\nif ta.crossover(macdLine, signal)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(macdLine, signal)\n    strategy.close_all()"),
        ("Şablon: RSI Dip + Trend Filtresi", "RSI dip alımı yalnız 200-EMA üzerinde", "//@version=5\nstrategy(\"RSI + Trend\")\nr = ta.rsi(close, 14)\ntrend = ta.ema(close, 200)\nif ta.crossover(r, 30) and close > trend\n    strategy.entry(\"L\", strategy.long)\nif r > 55\n    strategy.close_all()"),
        ("Şablon: Bollinger Dip", "Alt banda dokunuşta giriş, bazda çıkış (mean-revert)", "//@version=5\nstrategy(\"Bollinger Dip\")\nbasis = ta.sma(close, 20)\ndev = ta.stdev(close, 20)\nlower = basis - 2 * dev\nif close < lower\n    strategy.entry(\"L\", strategy.long)\nif ta.crossover(close, basis)\n    strategy.close_all()"),
        ("Şablon: Supertrend Takip", "Supertrend çizgi kesişimiyle giriş/çıkış", "//@version=5\nstrategy(\"Supertrend Takip\")\nst = ta.supertrend(3, 10)\nif ta.crossover(close, st)\n    strategy.entry(\"L\", strategy.long)\nif ta.crossunder(close, st)\n    strategy.close_all()"),
        ("Şablon: Donchian Kırılım", "20-bar kanal kırılımı + karşı kanal çıkışı", "//@version=5\nstrategy(\"Donchian Kırılım\")\nupper = ta.highest(high, 20)[1]\nlower = ta.lowest(low, 20)[1]\nif close > upper\n    strategy.entry(\"L\", strategy.long)\nif close < lower\n    strategy.close_all()"),
    ];
    let now = state::now_ms();
    for (name, notes, code) in seeds {
        let conn = match app.db.conn.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        let exists: i64 = conn
            .query_row("SELECT COUNT(*) FROM pine_scripts WHERE name=?1", [name], |r| r.get(0))
            .unwrap_or(0);
        if exists == 0 {
            // pine_scripts.id TEXT PRIMARY KEY — id'siz satır NULL-PK olur ve
            // listelemede String'e çözülemediği için görünmez düşer; id zorunlu.
            let id = format!("ps-{}", uuid::Uuid::new_v4().simple());
            match conn.execute(
                "INSERT INTO pine_scripts(id, name, code, inputs, notes, created_at, updated_at) VALUES(?1,?2,?3,'{}',?4,?5,?6)",
                rusqlite::params![id, name, code, notes, now, now],
            ) {
                Ok(_) => tracing::info!("pine library seeded: {name}"),
                Err(e) => tracing::warn!("pine seed yazılamadı ({name}): {e}"),
            }
        }
    }
}

/// Faz 0.9 (B-4): SIGINT / SIGTERM graceful shutdown — bağlantıları durdurur, WAL checkpoint (TRUNCATE) yapar
async fn shutdown_signal(shutdown_tx: tokio::sync::watch::Sender<bool>) {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => { s.recv().await; }
            Err(_) => { std::future::pending::<()>().await; }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("[shutdown] Graceful shutdown triggered; stopping poll scheduling.");
    shutdown_tx.send_replace(true);
}

/// Faz 0.6 (B-4): Prometheus metrik ucu (/metrics)
async fn prometheus_metrics(State(app): State<Arc<AppState>>) -> impl IntoResponse {
    let started = app.started_at;
    let now = state::now_ms();
    let uptime_sec = (now - started).max(0) / 1000;
    let last_tick = app.last_tick_at.lock().map(|g| *g).unwrap_or(0);
    let tick_age_sec = if last_tick > 0 { (now - last_tick).max(0) / 1000 } else { 0 };

    let (bot_count, running_count, stopped_count, active_deals) = {
        let runtimes = app.runtimes.lock().await;
        let mut running = 0;
        let mut stopped = 0;
        let mut deals = 0;
        for (_, rt) in runtimes.iter() {
            if rt.status == "RUNNING" { running += 1; } else { stopped += 1; }
            if !rt.stepper.position.qty.is_zero() || !rt.live_open_orders.is_empty() { deals += 1; }
        }
        (runtimes.len(), running, stopped, deals)
    };

    let sse_subs = app.sse.receiver_count();

    let db_size = std::env::var("DCA_DB_PATH")
        .ok()
        .and_then(|p| std::fs::metadata(p).ok().map(|m| m.len()))
        .unwrap_or(0);

    let body = format!(
        "# HELP dcabot_uptime_seconds Engine uptime in seconds\n\
# TYPE dcabot_uptime_seconds counter\n\
dcabot_uptime_seconds {uptime_sec}\n\
# HELP dcabot_bots_total Total number of configured bots\n\
# TYPE dcabot_bots_total gauge\n\
dcabot_bots_total {bot_count}\n\
dcabot_bots_status{{status=\"RUNNING\"}} {running_count}\n\
dcabot_bots_status{{status=\"STOPPED\"}} {stopped_count}\n\
# HELP dcabot_active_deals Active trading deals\n\
# TYPE dcabot_active_deals gauge\n\
dcabot_active_deals {active_deals}\n\
# HELP dcabot_sse_subscribers Active SSE clients\n\
# TYPE dcabot_sse_subscribers gauge\n\
dcabot_sse_subscribers {sse_subs}\n\
# HELP dcabot_tick_last_timestamp_ms Last supervisor tick timestamp in ms\n\
# TYPE dcabot_tick_last_timestamp_ms gauge\n\
dcabot_tick_last_timestamp_ms {last_tick}\n\
# HELP dcabot_tick_age_seconds Time elapsed since last engine tick in seconds\n\
# TYPE dcabot_tick_age_seconds gauge\n\
dcabot_tick_age_seconds {tick_age_sec}\n\
# HELP dcabot_db_size_bytes SQLite database file size in bytes\n\
# TYPE dcabot_db_size_bytes gauge\n\
dcabot_db_size_bytes {db_size}\n"
    );

    (
        [(axum::http::header::CONTENT_TYPE, "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

/// Faz 1.3: Bildirim konfigürasyon durumu sorgulama
async fn notifications_config() -> Json<Value> {
    let cfg = notify_cfg();
    Json(json!({
        "telegram_configured": cfg.as_ref().map(|c| c.telegram.is_some()).unwrap_or(false),
        "webhook_configured": cfg.as_ref().map(|c| c.webhook.is_some()).unwrap_or(false),
        "smtp_configured": cfg.as_ref().map(|c| c.smtp.is_some()).unwrap_or(false),
    }))
}

/// Faz 1.3: Bildirim test tetikleme
async fn notifications_test(State(app): State<Arc<AppState>>) -> Json<Value> {
    let cfg = notify_cfg();
    if cfg.is_none() {
        return Json(json!({
            "success": false,
            "error": "Hiçbir bildirim kanalı (Telegram, Webhook, SMTP) yapılandırılmamış. .env dosyasını kontrol edin."
        }));
    }
    let _ = app.sse.send(SseEvent {
        event: "TEST".into(),
        data: json!({
            "message": "DCABOT Terminal Bildirim Testi — Sistem başarıyla bağlandı.",
            "ts": state::now_ms()
        }).to_string(),
    });
    Json(json!({
        "success": true,
        "message": "Test bildirimi tüm aktif kanallara iletildi."
    }))
}

/// Faz 8: Telegram bildirim kanalı — TELEGRAM_BOT_TOKEN + TELEGRAM_CHAT_ID
/// ikisi de set edildiyse SSE olay akışına abone olur; FILL/LIQUIDATION/RISK/
/// WEBHOOK_START/KILL_SWITCH/FREEZE/BOT_STATE olaylarını Bot API'ye iletir.
/// Token yoksa sessiz atlanır (opsiyonel özellik — hata üretmez).
/// Faz 11: bildirim kanalları — yapılandırılmış TÜM kanallara fan-out:
///   • Telegram         : TELEGRAM_BOT_TOKEN + TELEGRAM_CHAT_ID
///   • Outgoing webhook : NOTIFY_WEBHOOK_URL  ({event, data, ts} JSON POST)
///   • E-posta (SMTP)   : NOTIFY_SMTP_HOST + NOTIFY_SMTP_TO (+USER/PASS/FROM)
/// Hiçbiri set değilse abone olunmaz — tamamen opsiyonel.
const NOTIFY_EVENTS: &[&str] = &["FILL", "LIQUIDATION", "RISK", "WEBHOOK_START", "KILL_SWITCH", "FREEZE", "BOT_STATE", "TEST"];

struct NotifyCfg {
    telegram: Option<(String, String)>,
    webhook: Option<String>,
    smtp: Option<SmtpCfg>,
}

#[derive(Clone)]
struct SmtpCfg {
    host: String,
    user: String,
    pass: String,
    from: String,
    to: String,
}

fn notify_cfg() -> Option<NotifyCfg> {
    let telegram = match (std::env::var("TELEGRAM_BOT_TOKEN"), std::env::var("TELEGRAM_CHAT_ID")) {
        (Ok(t), Ok(c)) if !t.trim().is_empty() && !c.trim().is_empty() => Some((t.trim().to_string(), c.trim().to_string())),
        _ => None,
    };
    let webhook = std::env::var("NOTIFY_WEBHOOK_URL")
        .ok()
        .filter(|u| !u.trim().is_empty())
        .map(|u| u.trim().to_string());
    let smtp = match (
        std::env::var("NOTIFY_SMTP_HOST"),
        std::env::var("NOTIFY_SMTP_TO"),
        std::env::var("NOTIFY_SMTP_USER"),
        std::env::var("NOTIFY_SMTP_PASS"),
    ) {
        (Ok(h), Ok(to), Ok(u), Ok(p)) if !h.trim().is_empty() && !to.trim().is_empty() && !u.trim().is_empty() => {
            Some(SmtpCfg {
                host: h.trim().to_string(),
                user: u.trim().to_string(),
                pass: p,
                from: std::env::var("NOTIFY_SMTP_FROM")
                    .ok()
                    .filter(|f| !f.trim().is_empty())
                    .unwrap_or_else(|| u.trim().to_string()),
                to: to.trim().to_string(),
            })
        }
        _ => None,
    };
    if telegram.is_none() && webhook.is_none() && smtp.is_none() {
        return None;
    }
    Some(NotifyCfg { telegram, webhook, smtp })
}

fn spawn_notifiers(app: &Arc<AppState>) {
    let Some(cfg) = notify_cfg() else { return };
    let channels: Vec<&str> = [
        cfg.telegram.as_ref().map(|_| "telegram"),
        cfg.webhook.as_ref().map(|_| "webhook"),
        cfg.smtp.as_ref().map(|_| "smtp"),
    ]
    .into_iter()
    .flatten()
    .collect();
    tracing::info!("notifier aktif: {}", channels.join("+"));
    let mut rx = app.sse.subscribe();
    let http = app.news_http.clone();
    tokio::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    if !NOTIFY_EVENTS.contains(&ev.event.as_str()) {
                        continue;
                    }
                    let payload = serde_json::json!({ "event": ev.event, "data": ev.data, "ts": state::now_ms() });
                    if let Some((token, chat)) = &cfg.telegram {
                        let text = format!("*DCABOT {}*\n{}", ev.event, ev.data.chars().take(3600).collect::<String>());
                        let url = format!("https://api.telegram.org/bot{token}/sendMessage");
                        let body = serde_json::json!({
                            "chat_id": chat,
                            "text": text,
                            "parse_mode": "Markdown",
                            "disable_notification": ev.event == "BOT_STATE",
                        });
                        if let Err(e) = http.post(&url).json(&body).send().await {
                            tracing::warn!("telegram gönderim hatası: {e}");
                        }
                    }
                    if let Some(url) = &cfg.webhook {
                        if let Err(e) = http.post(url).json(&payload).send().await {
                            tracing::warn!("webhook bildirim hatası: {e}");
                        }
                    }
                    if let Some(sm) = &cfg.smtp {
                        let subject = format!("[DCABOT] {}", ev.event);
                        let body = serde_json::to_string_pretty(&payload).unwrap_or_default();
                        let sm = SmtpCfg { host: sm.host.clone(), user: sm.user.clone(), pass: sm.pass.clone(), from: sm.from.clone(), to: sm.to.clone() };
                        let host = sm.host.clone();
                        let _ = tokio::task::spawn_blocking(move || {
                            use lettre::Transport;
                            let from = match sm.from.parse() {
                                Ok(m) => m,
                                Err(_) => return eprintln!("[smtp] from adresi geçersiz"),
                            };
                            let to = match sm.to.parse() {
                                Ok(m) => m,
                                Err(_) => return eprintln!("[smtp] to adresi geçersiz"),
                            };
                            let email = lettre::Message::builder()
                                .from(from)
                                .to(to)
                                .subject(&subject)
                                .body(body);
                            match email {
                                Ok(m) => {
                                    match lettre::SmtpTransport::relay(&host) {
                                        Ok(builder) => {
                                            let transport = builder
                                                .credentials(lettre::transport::smtp::authentication::Credentials::new(sm.user.clone(), sm.pass.clone()))
                                                .build();
                                            if let Err(e) = transport.send(&m) {
                                                eprintln!("[smtp] e-posta hatası: {e}");
                                            }
                                        }
                                        Err(e) => eprintln!("[smtp] relay kurulumu hatası: {e}"),
                                    }
                                }
                                Err(e) => eprintln!("[smtp] mail kurulumu hatası: {e}"),
                            }
                        })
                        .await;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    tracing::warn!("notifier: {n} olay kaçırıldı (yayın taşması)");
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            }
        }
    });
}
#[derive(Deserialize)]
struct GridSuggestQuery {
    symbol: String,
    #[serde(default = "default_interval")]
    interval: String,
    #[serde(default = "default_gs_bars")]
    bars: usize,
    #[serde(default = "default_gs_investment")]
    investment: String,
    venue: Option<String>,
    #[serde(default = "default_gs_levels")]
    levels: usize,
}

fn default_gs_bars() -> usize { 500 }
fn default_gs_investment() -> String { "1000".into() }
fn default_gs_levels() -> usize { 16 }

async fn analysis_grid_suggest(State(app): State<Arc<AppState>>, Query(q): Query<GridSuggestQuery>) -> Result<Json<Value>, axum::response::Response> {
    let symbol = normalize_pair(&q.symbol);
    let iv = engine::bybit_interval(&q.interval).map_err(err422)?;
    let bars_req = q.bars.clamp(100, 1000);
    let venue = q.venue.unwrap_or_else(|| "BYBIT".into()).to_uppercase();
    let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
    let kl = adapter.klines("linear", &symbol, &iv, bars_req).await.map_err(err422)?;
    if kl.len() < 60 {
        return Err(err422(format!("yetersiz bar: {} (min 60)", kl.len())));
    }
    let closes: Vec<f64> = kl.iter().filter_map(|k| k.c.parse::<f64>().ok()).filter(|c| *c > 0.0).collect();
    let (lo, hi) = closes.iter().fold((f64::MAX, f64::MIN), |(a, b), c| (a.min(*c), b.max(*c)));
    // Volatilite: bar-başı log-return sigma (kalibrasyonla aynı yöntem)
    let rets: Vec<f64> = closes.windows(2).map(|w| (w[1] / w[0]).ln()).collect();
    let n = rets.len() as f64;
    let mu = rets.iter().sum::<f64>() / n;
    let sigma = (rets.iter().map(|r| (r - mu).powi(2)).sum::<f64>() / n).sqrt();
    // Seviye sayısı: sigma×√bar penceresinin %75'i adım başına → seviye = pencere/adım
    let span = hi / lo;
    let steps_target = ((span.ln() / (0.75 * sigma)).exp()).clamp(8.0, 48.0);
    let levels = if q.levels >= 8 { q.levels as f64 } else { steps_target };
    let investment = Fraction::from_decimal_str(&q.investment).map_err(err422)?;
    if !investment.is_positive() {
        return Err(err422("investment > 0 olmalı"));
    }
    // Geometric mesh: lo = pencere dibi ×1.01, hi = tepe ×0.99 (kaçak payı)
    let lower = lo * 1.01;
    let upper = hi * 0.99;
    let step_pct = (upper / lower).powf(1.0 / levels) - 1.0;
    // Ücret-kırıcı: g_min ≈ (f_b+f_s)/(1−f_s); spot varsayılan %0.10/%0.10
    let g_min = (0.001 + 0.001) / (1.0 - 0.001);
    let breaker_ok = step_pct > g_min;
    let per_level = investment.div(&Fraction::from_u64(levels as u64)).map_err(err422)?;
    let inst = adapter.instruments("linear", Some(&symbol)).await.ok().and_then(|l| l.into_iter().next());
    let step_size = inst.as_ref().map(|i| i.step_size.to_half_even_display(8)).unwrap_or_else(|| "0.001".into());
    let tick_size = inst.as_ref().map(|i| i.tick_size.to_half_even_display(8)).unwrap_or_else(|| "0.1".into());
    Ok(Json(json!({
        "symbol": symbol,
        "interval": q.interval,
        "bars": closes.len(),
        "windowLow": format!("{lower:.4}"),
        "windowHigh": format!("{upper:.4}"),
        "sigmaPct": format!("{:.4}", sigma * 100.0),
        "suggestedLevels": levels as u64,
        "stepPct": format!("{:.4}", step_pct * 100.0),
        "gMinPct": format!("{:.4}", g_min * 100.0),
        "feeBreakerPass": breaker_ok,
        "perLevelNotional": per_level.to_half_even_display(2),
        "tickSize": tick_size,
        "stepSize": step_size,
        "note": if breaker_ok { "Öneri ücret-kırıcıdan geçiyor — Lab'de backtest + sertifika ile doğrulayın." } else { "ADIM g_min'in altında: seviye sayısını düşürün veya menzili genişletin (ücret kırıcı KALDI)." },
    })))
}

async fn restore_bots(app: &Arc<AppState>) {
    let rows: Vec<(String, String, String, String, String, String, String, Option<String>, Option<String>)> = {
        let conn = match app.db.conn.lock() {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut stmt = match conn.prepare("SELECT id, name, kind, venue, mode, symbol, status, interval, start_cash FROM bots WHERE status IN ('RUNNING','PAUSED','FROZEN')") {
            Ok(s) => s,
            Err(_) => return,
        };
        let iter = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, String>(6)?,
                    r.get::<_, Option<String>>(7)?,
                    r.get::<_, Option<String>>(8)?,
                ))
            })
            .ok();
        match iter {
            Some(it) => it.filter_map(|r| r.ok()).collect(),
            None => Vec::new(),
        }
    };
    let ambiguous_live_markets = duplicate_live_markets(rows.iter().map(
        |(_, _, _, venue, mode, symbol, status, _, _)| {
            (venue.as_str(), mode.as_str(), symbol.as_str(), status.as_str())
        },
    ));

    for (id, name, _kind, venue, mode, symbol, status, interval, start_cash) in rows {
        let ambiguous_live_market = mode.eq_ignore_ascii_case("LIVE")
            && ambiguous_live_markets.contains(&normalized_live_market(&venue, &symbol));
        let (cfg_json,): (String,) = match app.db.conn.lock() {
            Ok(conn) => match conn.query_row("SELECT config FROM bots WHERE id=?1", [&id], |r| r.get(0)) {
                Ok(v) => (v,),
                Err(_) => continue,
            },
            Err(_) => continue,
        };
        let cfg: BotConfig = match serde_json::from_str(&cfg_json) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Persisted interval / start cash (legacy rows fall back to the old defaults).
        let interval = interval.unwrap_or_else(|| "1h".to_string());
        let start_cash = start_cash
            .as_deref()
            .and_then(|s| Fraction::from_any_str(s).ok())
            .unwrap_or_else(|| Fraction::from_decimal_str("100000").unwrap());
        // LIVE bots need vault credentials; without a resolvable key the bot is
        // downgraded to PAPER and a WARN event makes the downgrade visible
        // (status stays untouched — it is not silently faked as live trading).
        let mut effective_mode = mode.clone();
        let mut credentials: Option<VenueCreds> = None;
        if effective_mode == "LIVE" {
            // Faz 4 (C-5): HEDGE LIVE kilitli — restore'da dürüst düşürme.
            if false {
                // Faz 13B: HEDGE LIVE kilidi kaldırıldı (bacak-başına stepper) —
                // dal yapısı korundu; ileride tekrar kilit gerekirse buraya.
            } else if venue.to_uppercase() != "BYBIT" && venue.to_uppercase() != "OKX" {
                effective_mode = "PAPER".into();
                state::persist_event(app, &id, "WARN", "RESTORE", &format!("{venue} LIVE kanalı desteklenmiyor — bot PAPER modunda restore edildi")).await;
            } else {
                match store::load_active_key(&app.db, &app.vault, &venue.to_uppercase()) {
                    Ok(Some(creds)) => {
                        // P4-3: OKX passphrase'siz LIVE'a kalkamaz (imza eksik
                        // kalır, ilk poll'da hata verirdi — restore'da dürüst düşür).
                        if venue.to_uppercase() == "OKX" && creds.passphrase.as_deref().map(|p| p.trim().is_empty()).unwrap_or(true) {
                            effective_mode = "PAPER".into();
                            state::persist_event(app, &id, "WARN", "RESTORE", "OKX passphrase yok — bot PAPER moduna düşürüldü (anahtarı passphrase ile yeniden ekleyin)").await;
                        } else {
                            credentials = Some(creds);
                        }
                    }
                    Ok(None) => {
                        effective_mode = "PAPER".into();
                        state::persist_event(app, &id, "WARN", "RESTORE", "LIVE anahtarı bulunamadı — bot PAPER moduna düşürüldü (Ayarlar > Anahtar Kasası'ndan ekleyin)").await;
                    }
                    Err(e) => {
                        effective_mode = "PAPER".into();
                        state::persist_event(app, &id, "WARN", "RESTORE", &format!("LIVE anahtarı çözülemedi ({e}) — bot PAPER moduna düşürüldü")).await;
                    }
                }
            }
        }
        let restored_status = if ambiguous_live_market && effective_mode == "LIVE" { "FROZEN" } else { &status };
        if let Err(e) = state::create_runtime(app, &id, &name, &effective_mode, &venue, restored_status, cfg, &interval, start_cash, credentials).await {
            tracing::warn!("restore bot {id} failed: {e}");
        } else if ambiguous_live_market && effective_mode == "LIVE" {
            state::persist_status(app, &id, "FROZEN");
            state::persist_event(
                app,
                &id,
                "CRITICAL",
                "LIVE_OWNERSHIP",
                "aynı venue/sembolde birden fazla LIVE bot bulundu; hesap-geneli pozisyon güvenle atanamadığı için bot FROZEN açıldı ve pozisyon reconcile edilmedi",
            )
            .await;
        }
    }
    // BOT-03: yeniden başlatmada açık pozisyon ve emirleri geri yükle —
    //会場 ile uzlaştırma; yeni risk açılmadan önce yerel ayna senkronize edilir.
    reconcile_restored_bots(app, &ambiguous_live_markets).await;
}

/// BOT-03: restore edilen RUNNING/PAUSED/FROZEN botları venue ile uzlaştır:
/// açık pozisyonu stepper'a, açık emirleri live_open_orders'a yükler.
/// Uzlaştırma bitmeden yeni deal açılmaz (durum RECONCILING).
fn restored_order_semantics(bot_id: &str, order_id: &str) -> Option<(&'static str, dca_kernel::OrderKind)> {
    let prefix = format!("{bot_id}-");
    let suffix = order_id.strip_prefix(&prefix)?;
    let role = if suffix.contains("-GRID_CLOSE-") {
        ("GRID_CLOSE", dca_kernel::OrderKind::Reduce)
    } else if suffix.contains("-TPL-") || suffix.contains("-SL-") || suffix.contains("-MANUAL-") {
        let role = if suffix.contains("-TPL-") { "TP" } else if suffix.contains("-SL-") { "SL" } else { "MANUAL" };
        (role, dca_kernel::OrderKind::Reduce)
    } else if suffix.contains("-TRAIL-") || suffix.contains("-INF-") {
        ("TP", dca_kernel::OrderKind::Reduce)
    } else if suffix.contains("-SAFETY-") {
        ("SAFETY", dca_kernel::OrderKind::Add)
    } else if suffix.contains("-GRID-") {
        ("GRID", dca_kernel::OrderKind::Add)
    } else if suffix.contains("-BASE-") || suffix.starts_with("BASE-") {
        ("BASE", dca_kernel::OrderKind::Add)
    } else {
        return None;
    };
    Some(role)
}

async fn reconcile_restored_bots(app: &Arc<AppState>, ambiguous_live_markets: &HashSet<(String, String)>) {
    let bot_ids: Vec<String> = {
        let rts = app.runtimes.lock().await;
        rts.iter().filter(|(_, rt)| rt.status == "RUNNING" || rt.status == "PAUSED" || rt.status == "FROZEN").map(|(id, _)| id.clone()).collect()
    };
    for id in bot_ids {
        let (creds_opt, venue, mode, symbol, is_futures) = {
            let rts = app.runtimes.lock().await;
            let rt = match rts.get(&id) {
                Some(r) => r,
                None => continue,
            };
            (
                rt.credentials.clone(),
                rt.venue.clone(),
                rt.mode.clone(),
                rt.cfg.symbol.clone(),
                rt.cfg.kind.is_futures(),
            )
        };
        // Yalnız LIVE botlar için venue'den pozisyon/emir alınabilir.
        if mode != "LIVE" {
            continue;
        }
        if ambiguous_live_markets.contains(&normalized_live_market(&venue, &symbol)) {
            // Do not assign an account-wide aggregate position to every bot
            // that shares this venue/symbol after an older deployment restore.
            continue;
        }
        let Some(creds) = creds_opt else {
            state::persist_event(app, &id, "WARN", "RESTORE", "LIVE anahtarı yok — yeniden başlatma pozisyonu geri yüklenemedi").await;
            continue;
        };
        let Some(adapter) = app.adapter(&venue) else { continue };
        let category = if is_futures { "linear" } else { "spot" };
        // Açık pozisyonu geri yükle.
        match adapter.positions(&creds, &symbol).await {
            Ok(positions) => {
                if let Some(pos) = positions.iter().find(|p| !p.size.is_zero()) {
                    let qty = pos.size.clone();
                    let avg = pos.avg_price.clone();
                    // Stepper pozisyonunu senkronize et.
                    {
                        let mut rts = app.runtimes.lock().await;
                        if let Some(rt_mut) = rts.get_mut(&id) {
                            rt_mut.stepper.position.qty = qty.clone();
                            rt_mut.stepper.position.avg_price = Some(avg.clone());
                            rt_mut.stepper.position.cost = qty.mul(&avg).unwrap_or_else(|_| Fraction::zero());
                            rt_mut.stepper.position.direction = if pos.side.eq_ignore_ascii_case("Sell") { -1 } else { 1 };
                        }
                    }
                    state::persist_event(app, &id, "INFO", "RESTORE", &format!("açık pozisyon geri yüklendi: {} @ {}", qty.to_half_even_display(8), avg.to_half_even_display(8))).await;
                }
            }
            Err(e) => {
                state::persist_event(app, &id, "WARN", "RESTORE", &format!("pozisyon uzlaştırma hatası: {e}")).await;
            }
        }
        // Faz 3 (H-3): kalıcı emir takip kayıtları (PENDING/PARTIAL) önce —
        // restart öncesi place edilmiş emirler venue listesinden bağımsız izlenir.
        {
            let from_db = state::load_live_orders(app, &id);
            if !from_db.is_empty() {
                let mut rts = app.runtimes.lock().await;
                if let Some(rt_mut) = rts.get_mut(&id) {
                    for o in from_db {
                        if !rt_mut.live_open_orders.iter().any(|t| t.id == o.id) {
                            rt_mut.live_open_orders.push(o);
                        }
                    }
                }
            }
        }
        // Açık emirleri geri yükle (venue'da hâlâ açık olanlar — RESTORED etiketiyle).
        // DB'den yüklenmişlerle çakışmayı önlemek için id ile kontrol edilir.
        match adapter.open_orders(&creds, &category, &symbol).await {
            Ok(open_orders) => {
                let mut added = 0usize;
                for o in &open_orders {
                    // open_orders is account-wide on several venues. Never
                    // attach another bot's same-symbol order to this runtime.
                    if !o.order_link_id.starts_with(&format!("{id}-")) {
                        continue;
                    }
                    let Some((role, kind)) = restored_order_semantics(&id, &o.order_link_id) else {
                        if let Some(rt_mut) = app.runtimes.lock().await.get_mut(&id) {
                            rt_mut.status = "FROZEN".into();
                        }
                        state::persist_status(app, &id, "FROZEN");
                        state::persist_event(app, &id, "CRITICAL", "RESTORE", &format!(
                            "bu botun açık emrinin rolü çözülemedi ({}); yeni emirler güvenlik için durduruldu",
                            o.order_link_id
                        )).await;
                        continue;
                    };
                    let already = {
                        let rts = app.runtimes.lock().await;
                        rts.get(&id).map(|rt| rt.live_open_orders.iter().any(|t| t.id == o.order_link_id)).unwrap_or(false)
                    };
                    if !already {
                        {
                            let mut rts = app.runtimes.lock().await;
                            if let Some(rt_mut) = rts.get_mut(&id) {
                                rt_mut.live_open_orders.push(state::LiveTrackedOrder {
                                    id: o.order_link_id.clone(),
                                    side: match o.side.as_str() { "Sell" => dca_math::Side::Sell, _ => dca_math::Side::Buy },
                                    kind,
                                    price: o.price.clone().unwrap_or_else(Fraction::zero),
                                    qty: o.qty.clone(),
                                    role: role.into(),
                                });
                                added += 1;
                            }
                        }
                    }
                }
                if added > 0 {
                    state::persist_event(app, &id, "INFO", "RESTORE", &format!("{} açık emir geri yüklendi", added)).await;
                }
            }
            Err(e) => {
                state::persist_event(app, &id, "WARN", "RESTORE", &format!("açık emir uzlaştırma hatası: {e}")).await;
            }
        }
    }
}

// ---------------- Handlers ----------------

/// Constant-time comparison for secrets (immune to length and content timing attacks).
pub fn constant_time_eq_str(provided: &str, expected: &str) -> bool {
    use sha2::{Digest, Sha256};
    if provided.is_empty() || expected.is_empty() {
        return false;
    }
    let h1 = Sha256::digest(provided.as_bytes());
    let h2 = Sha256::digest(expected.as_bytes());
    h1.iter().zip(h2.iter()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// SEC-01: API key auth middleware — `Authorization: Bearer <DCA_API_KEY>`.
/// Eksik/yanlış anahtar → 401. `DCA_API_KEY` env ile yapılandırılır.
async fn auth_middleware(
    State(api_key): State<String>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Result<axum::response::Response, StatusCode> {
    let headers = req.headers();
    let Some(auth) = headers.get("authorization").and_then(|v| v.to_str().ok()) else {
        return Err(StatusCode::UNAUTHORIZED);
    };
    if !auth.starts_with("Bearer ") {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let provided = &auth[7..];
    // Empty api_key means auth is not configured — reject all (fail-closed).
    if !constant_time_eq_str(provided, &api_key) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

async fn health(State(app): State<Arc<AppState>>) -> Json<Value> {
    // Faz 5 (H-8): health yüzeyel "ok"tan dürüst readiness'a taşındı — DB
    // yazılabilirliği ve supervisor hattının yaşı yanıta girer; liveness için
    // süreç ayakta + status alanı korunur (mevcut tüketiciler bozulmaz).
    let db_ok = app
        .db
        .conn
        .lock()
        .map(|c| c.query_row::<i64, _, _>("SELECT 1", [], |r| r.get(0)).unwrap_or(0) == 1)
        .unwrap_or(false);
    let (active_bots, total_bots) = {
        let rts = app.runtimes.lock().await;
        let total = rts.len();
        let active = rts.values().filter(|r| matches!(r.status.as_str(), "RUNNING" | "FROZEN")).count();
        (active, total)
    };
    let last_tick_ms = *app.last_tick_at.lock().unwrap_or_else(|e| e.into_inner());
    let now = state::now_ms();
    Json(json!({
        "status": "ok",
        "service": "dca-core",
        "version": "3.0.0",
        "math": "4096-bit rational (M01-M20)",
        "gates": 8,
        "venues": ["BYBIT", "BINANCE", "OKX", "PAPER"],
        "uptime_s": (now - app.started_at) / 1000,
        "readiness": {
            "db": if db_ok { "ok" } else { "ERR" },
            "lastTickMs": last_tick_ms,
            "lastTickAgeS": (now - last_tick_ms) / 1000,
            "activeBots": active_bots,
            "totalBots": total_bots,
            "note": "lastTickAgeS > 60 ve aktif bot varsa supervisor hattı takılmış olabilir",
        },
    }))
}

/// Faz 24 (UI/UX Pro Max raporu §3.1.1): portföy equity curve — tüm botların
/// equity_points kayıtlarını güne göre birleştirip toplam döndürür.
/// `?days=N` (varsayılan 30, üst sınır 90) → son N gün; nokta sayısı
/// örneklenerek (~720) grafik dostu seri döner.
async fn equity_curve(
    State(app): State<Arc<AppState>>,
    q: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> Result<Json<Value>, axum::response::Response> {
    let days: i64 = q
        .get("days")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(30)
        .clamp(1, 90);
    let since_ms = crate::state::now_ms() - days * 86_400_000;
    let conn = app.db.conn.lock().map_err(|e| {
        (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({"error": e.to_string()})),
        )
            .into_response()
    })?;
    let mut stmt = conn
        .prepare(
            "SELECT ts, equity FROM equity_points WHERE ts >= ?1 ORDER BY ts ASC",
        )
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        })?;
    let rows = stmt
        .query_map([since_ms], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))
        .map_err(|e| {
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({"error": e.to_string()})),
            )
                .into_response()
        })?;
    let mut map: std::collections::BTreeMap<i64, f64> = std::collections::BTreeMap::new();
    for r in rows.flatten() {
        let (ts, eq_str) = r;
        let val = if let Some((n, d)) = eq_str.split_once('/') {
            let num: f64 = n.trim().parse().unwrap_or(0.0);
            let den: f64 = d.trim().parse().unwrap_or(1.0);
            if den.abs() > 1e-12 {
                num / den
            } else {
                num
            }
        } else {
            eq_str.trim().parse().unwrap_or(0.0)
        };
        *map.entry(ts).or_insert(0.0) += val;
    }
    let mut raw: Vec<(i64, f64)> = map.into_iter().collect();
    drop(stmt);
    drop(conn);
    let total = raw.len();
    if total > 720 {
        let step = total as f64 / 720.0;
        raw = (0..720).map(|i| raw[((i as f64) * step) as usize]).collect();
    }
    let points: Vec<serde_json::Value> = raw
        .iter()
        .map(|(t, v)| serde_json::json!({"ts": t, "equity": (v * 100.0).round() / 100.0}))
        .collect();
    Ok(Json(serde_json::json!({"days": days, "points": points, "total": total})))
}

async fn summary(State(app): State<Arc<AppState>>) -> Json<Value> {
    // Faz 13 dilim 2: özet, anlık görüntü önbelleğinden hesaplanır — venue
    // yavaşken Fleet/Özet istekleri runtime kilidini beklemez. Cache boşsa
    // (yeni boot) eski kilitli yol dürüst fallback.
    let cache = app.ui_cache.lock().await;
    if !cache.is_empty() {
        let out = summarize_from_cache(&cache);
        drop(cache);
        return Json(out);
    }
    drop(cache);
    summarize_locked(State(app)).await
}

/// Cache-anlık görüntülerinden özet agregasyonu (frontend sözleşmesi aynen).
fn summarize_from_cache(cache: &std::collections::HashMap<String, serde_json::Value>) -> serde_json::Value {
    use dca_math::fraction::Fraction;
    let mut total_equity = Fraction::zero();
    let mut total_unreal = Fraction::zero();
    let mut total_realized = Fraction::zero();
    let mut total_fees = Fraction::zero();
    let mut total_funding = Fraction::zero();
    let mut active_bots = 0u32;
    let mut active_deals = 0u32;
    let mut long_exp = Fraction::zero();
    let mut short_exp = Fraction::zero();
    let mut alloc: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let fr = |v: &serde_json::Value| -> Fraction { v.as_str().and_then(|s| Fraction::from_any_str(s).ok()).unwrap_or_else(Fraction::zero) };
    for v in cache.values() {
        let status = v["status"].as_str().unwrap_or("");
        if matches!(status, "RUNNING" | "FROZEN") {
            active_bots += 1;
        }
        if v["stats"]["deal_id"].is_string() {
            active_deals += 1;
        }
        total_equity = total_equity.add(&fr(&v["equityNow"])).unwrap_or_else(|_| total_equity.clone());
        total_unreal = total_unreal.add(&fr(&v["unrealizedNow"])).unwrap_or_else(|_| total_unreal.clone());
        total_realized = total_realized.add(&fr(&v["stats"]["realized_gross"])).unwrap_or_else(|_| total_realized.clone());
        total_fees = total_fees.add(&fr(&v["cumulativeFees"])).unwrap_or_else(|_| total_fees.clone());
        total_funding = total_funding.add(&fr(&v["cumulativeFunding"])).unwrap_or_else(|_| total_funding.clone());
        let qty = fr(&v["stats"]["position_qty"]);
        let avg = fr(&v["stats"]["avg_price"]);
        let notional = qty.mul(&avg).unwrap_or_else(|_| Fraction::zero());
        if !notional.is_zero() {
            let short_dir = v["kind"].as_str().map(|k| k.contains("SHORT") || k.contains("HEDGE")).unwrap_or(false);
            if short_dir {
                short_exp = short_exp.add(&notional).unwrap_or_else(|_| short_exp.clone());
            } else {
                long_exp = long_exp.add(&notional).unwrap_or_else(|_| long_exp.clone());
            }
            let sym = v["symbol"].as_str().unwrap_or("?").to_string();
            *alloc.entry(sym).or_insert(0.0) += notional.to_f64_display();
        }
        // hedge short bacağı
        if let Some(leg) = v["stats"]["hedge"]["hedge_leg"].as_object() {
            let lq = leg.get("position_qty").map(fr).unwrap_or_else(Fraction::zero);
            if let Some(a) = leg.get("avg_price") {
                let la = fr(a);
                let ln = lq.mul(&la).unwrap_or_else(|_| Fraction::zero());
                if !ln.is_zero() {
                    short_exp = short_exp.add(&ln).unwrap_or_else(|_| short_exp.clone());
                }
            }
        }
    }
    let alloc_total: f64 = alloc.values().sum();
    let allocation_json: Vec<serde_json::Value> = alloc
        .into_iter()
        .map(|(symbol, val)| {
            json!({
                "symbol": symbol,
                "value": format!("{val}"),
                "pct": if alloc_total > 0.0 { (val / alloc_total * 100.0 * 100.0).round() / 100.0 } else { 0.0 },
            })
        })
        .collect();
    json!({
        "totalEquity": total_equity.to_string(),
        "totalUnrealizedPnl": total_unreal.to_string(),
        "totalRealizedGross": total_realized.to_string(),
        "totalFees": total_fees.to_string(),
        "totalFunding": total_funding.to_string(),
        "activeBots": active_bots,
        "totalBots": cache.len(),
        "activeDeals": active_deals,
        "longExposure": long_exp.to_string(),
        "shortExposure": short_exp.to_string(),
        "allocation": allocation_json,
        "engineUptimeS": 0,
    })
}

async fn summarize_locked(State(app): State<Arc<AppState>>) -> Json<Value> {
    let rts = app.runtimes.lock().await;
    let mut active_bots = 0;
    let mut total_bots = 0;
    let mut active_deals = 0;
    let mut long_exposure = Fraction::zero();
    let mut short_exposure = Fraction::zero();
    let mut total_realized = Fraction::zero();
    let mut total_fees = Fraction::zero();
    let mut total_funding = Fraction::zero();
    let mut equity_total = Fraction::zero();
    let mut total_unrealized = Fraction::zero();
    let mut allocation: HashMap<String, f64> = HashMap::new();

    for rt in rts.values() {
        total_bots += 1;
        if rt.status == "RUNNING" || rt.status == "FROZEN" {
            active_bots += 1;
        }
        let snap = rt.engine_kind.snapshot();
        if snap.deal_id.is_some() {
            active_deals += 1;
        }
        let qty = Fraction::from_any_str(&snap.position_qty).unwrap_or_else(|_| Fraction::zero());
        if let Some(avg) = snap.avg_price.and_then(|a| Fraction::from_any_str(&a).ok()) {
            let notional = qty.mul(&avg).unwrap_or_else(|_| Fraction::zero());
            if !notional.is_zero() {
                let dir_sign: i32 = if matches!(rt.cfg.kind, StrategyKind::DcaShort) { -1 } else { 1 };
                if dir_sign > 0 {
                    long_exposure = long_exposure.add(&notional).unwrap_or_else(|_| long_exposure.clone());
                } else {
                    short_exposure = short_exposure.add(&notional).unwrap_or_else(|_| short_exposure.clone());
                }
                *allocation.entry(rt.cfg.symbol.clone()).or_insert(0.0) += notional.to_f64_display();
            }
        }
        // Task 21 — hedge botların kısa bacağı artık görünüyor: eski davranışta
        // hedge short pozisyonu shortExposure'a hiç yansımıyordu (dürüstsüz).
        if let Some(h) = &snap.hedge {
            if let Some(leg) = &h.hedge_leg {
                if let (Ok(lq), Some(lavg)) = (
                    Fraction::from_any_str(&leg.position_qty),
                    leg.avg_price.as_ref().and_then(|a| Fraction::from_any_str(a).ok()),
                ) {
                    let lnotional = lq.mul(&lavg).unwrap_or_else(|_| Fraction::zero());
                    if !lnotional.is_zero() {
                        short_exposure = short_exposure.add(&lnotional).unwrap_or_else(|_| short_exposure.clone());
                    }
                }
            }
        }
        total_realized = total_realized
            .add(&Fraction::from_any_str(&snap.realized_gross).unwrap_or_else(|_| Fraction::zero()))
            .unwrap_or_else(|_| total_realized.clone());
        total_fees = total_fees
            .add(&rt.stepper.cumulative_fees.clone())
            .unwrap_or_else(|_| total_fees.clone());
        total_funding = total_funding
            .add(&rt.stepper.cumulative_funding.clone())
            .unwrap_or_else(|_| total_funding.clone());
        // Real equity = cash + committed IM + unrealized PnL at the last mark
        // (SimulationStepper::equity — margin-aware; zero-float throughout).
        match rt.stepper.last_close.as_deref().and_then(|s| Fraction::from_any_str(s).ok()) {
            Some(mark) => {
                equity_total = rt
                    .stepper
                    .equity(&mark)
                    .map(|eq| equity_total.add(&eq).unwrap_or_else(|_| equity_total.clone()))
                    .unwrap_or_else(|_| equity_total.add(&rt.stepper.cash).unwrap_or_else(|_| equity_total.clone()));
                let upnl = rt.stepper.position.unrealized_gross(&mark).unwrap_or_else(|_| Fraction::zero());
                total_unrealized = total_unrealized.add(&upnl).unwrap_or_else(|_| total_unrealized.clone());
            }
            None => {
                // No mark yet (no bar processed): fall back to legacy cash sum.
                equity_total = equity_total.add(&rt.stepper.cash).unwrap_or_else(|_| equity_total.clone());
            }
        }
    }

    let alloc_total: f64 = allocation.values().sum();
    let allocation_json: Vec<Value> = allocation
        .into_iter()
        .map(|(symbol, v)| {
            json!({
                "symbol": symbol,
                "value": format!("{v}"),
                "pct": if alloc_total > 0.0 { (v / alloc_total * 100.0 * 100.0).round() / 100.0 } else { 0.0 },
            })
        })
        .collect();

    Json(json!({
        "totalEquity": equity_total.to_string(),
        "totalUnrealizedPnl": total_unrealized.to_string(),
        "totalRealizedGross": total_realized.to_string(),
        "totalFees": total_fees.to_string(),
        "totalFunding": total_funding.to_string(),
        "activeBots": active_bots,
        "totalBots": total_bots,
        "activeDeals": active_deals,
        "longExposure": long_exposure.to_string(),
        "shortExposure": short_exposure.to_string(),
        "allocation": allocation_json,
        "engineUptimeS": ((state::now_ms() - app.started_at) / 1000),
    }))
}

async fn list_bots(State(app): State<Arc<AppState>>) -> Json<Value> {
    // Faz 13 dilim 2: Fleet anlık görüntü önbelleğinden okunur — venue yavaşken
    // kilit beklemez. Cache boşsa (taze boot, poll öncesi) kilitli yol fallback.
    {
        let cache = app.ui_cache.lock().await;
        if !cache.is_empty() {
            let mut bots: Vec<Value> = cache.values().cloned().collect();
            bots.sort_by(|a, b| a["id"].as_str().unwrap_or("").cmp(b["id"].as_str().unwrap_or("")));
            return Json(json!({ "bots": bots }));
        }
    }

    let rts = app.runtimes.lock().await;
    let bots: Vec<Value> = rts.values().map(|rt| rt.snapshot_json()).collect();
    Json(json!({ "bots": bots }))
}

#[derive(Deserialize)]
struct CreateBotReq {
    name: String,
    kind: String,
    venue: String,
    mode: String,
    symbol: String,
    #[serde(default = "default_interval")]
    interval: String,
    config: Option<Value>,
    #[serde(default = "default_start_cash")]
    start_cash: String,
    /// Task 16 — kullanıcı risk uyarılarını onayladı mı? Uyarı varsa ve bu
    /// bayrak yoksa motor 422 RISK_ACKNOWLEDGMENT_REQUIRED ile uyarı listesini döner.
    #[serde(default)]
    risk_acknowledged: bool,
    /// ÜRÜN-1 (bilinçli-onay istisnası): LIVE sertifika kapısı KALDI'ysa ve bu
    /// bayrak TRUE ise kurulum WARN olayıyla devam eder ("rozetsiz strateji
    /// canlıya eklenemez" politikası, kullanıcının açık sorumluluk onayıyla tek
    /// istisna). Onay kalıcı olarak olay akışına yazılır.
    #[serde(default)]
    cert_acknowledged: bool,
    /// Faz 28 (P1-4 — bağımsız denetim): bilinçli-onayın GEREKÇESİ.
    /// cert_acknowledged=true iken zorunludur (422 CERT_ACK_REASON_REQUIRED);
    /// gerekçe WARN olayının kalıcı metnine yazılır — boş onay audit değerini
    /// düşürür ("kim, neden" kaydı olmadan istisna kapatılamaz).
    #[serde(default)]
    cert_reason: Option<String>,
    /// Faz 22 (P0-10 kalanı): kullanıcının GERÇEK denediği strateji
    /// konfigürasyon sayısı — DSR söndürmesi bunu kullanır (dahili 9 varyant
    /// alt sınırdır). 0/eksik = sadece dahili varyantlar.
    #[serde(default)]
    attempts: usize,
}

fn default_interval() -> String {
    "1h".into()
}

fn default_start_cash() -> String {
    "100000".into()
}

/// Config numeric fields must parse as exact fractions and be positive —
/// a silently-defaulted order size would place real orders with the wrong
/// size. `allow_zero` is used for safety_order_size (0 = no safety orders).
fn validate_cfg_positive(name: &str, raw: &str, allow_zero: bool) -> Result<(), String> {
    let v = Fraction::from_decimal_str(raw).map_err(|e| format!("config.{name}: '{raw}' geçerli bir sayı değil ({e})"))?;
    let ok = if allow_zero { !v.is_negative() } else { v > Fraction::zero() };
    if !ok {
        let bound = if allow_zero { "≥ 0" } else { "> 0" };
        return Err(format!("config.{name}: '{raw}' {bound} olmalı"));
    }
    Ok(())
}

async fn create_bot(State(app): State<Arc<AppState>>, Json(req): Json<CreateBotReq>) -> Result<axum::response::Response, axum::response::Response> {
    let kind = match parse_kind(&req.kind) { Ok(k) => k, Err(e) => return Err(err422(e)) };
    // Bar interval must be a real venue timeframe — a fake fallback would run
    // the bot on a timeframe it was never configured for.
    if let Err(e) = engine::validate_interval(&req.interval) {
        return Err(err422(e));
    }
    let mut cfg = match &req.config {
        Some(v) => match serde_json::from_value::<BotConfig>(v.clone()) {
            Ok(c) => c,
            Err(e) => return Err(err422(format!("config: {e}"))),
        },
        None => BotConfig::default(),
    };
    cfg.kind = kind;
    cfg.symbol = req.symbol.clone();

    // P3-1 kara liste kapısı: listedeki sembole bot kurulamaz (dürüst 422).
    {
        let pair = normalize_pair(&cfg.symbol);
        if !pair.is_empty() && is_blacklisted(&app, &pair) {
            return Err(err422(format!("sembol kara listede: {pair} — Ayarlar > Kara Liste'den kaldırın")));
        }
    }

    // Start condition (entry trigger) structural validation — 422 on bad params
    if let Err(e) = cfg.entry_trigger.validate() {
        return Err(err422(format!("entry_trigger: {e}")));
    }
    // Composite signal tree validation (3Commas-style, dca_signals)
    if let Some(sc) = &cfg.entry_signal {
        if let Err(e) = dca_signals::validate_signal_node(&sc.root) {
            return Err(err422(format!("entry_signal: {e}")));
        }
    }
    // Task 19 — Pine strateji doğrulaması (derleme + entry_trigger uyumu)
    if let Err(e) = cfg.validate_pine() {
        return Err(err422(e));
    }
    // Task 16 — Risk Motoru: politik aralık doğrulaması (yapısal 422'ler)
    if let Err(e) = cfg.risk_policy.validate(cfg.max_safety_orders) {
        return Err(err422(e));
    }

    // Sayısal config geçerliliği: motor bu alanları Fraction'a çevirir;
    // çözülemezse deal açılışında sessiz varsayana (qty=1, dev=%2) düşer.
    // Sessiz sahte emir yerine dürüst 422 — fail-closed felsefe.
    for (name, raw, min_excl) in [
        ("base_order_size", cfg.base_order_size.as_str(), None),
        ("safety_order_size", cfg.safety_order_size.as_str(), Some(true)),
        ("deviation_pct", cfg.deviation_pct.as_str(), None),
        ("step_scale", cfg.step_scale.as_str(), None),
        ("volume_scale", cfg.volume_scale.as_str(), None),
        ("target_profit_pct", cfg.target_profit_pct.as_str(), None),
    ] {
        if let Err(e) = validate_cfg_positive(name, raw, min_excl.is_some()) {
            return Err(err422(e));
        }
    }
    if let Some(sl) = &cfg.stop_loss_pct {
        match Fraction::from_decimal_str(sl) {
            Ok(v) if v > Fraction::zero() && v <= Fraction::one() => {}
            _ => return Err(err422(format!("config.stop_loss_pct: '{sl}' 0 < SL ≤ 1 (yüzde olarak) aralığında olmalı — geçersiz SL sessizce devre dışı bırakılamaz"))),
        }
    }
    // Faz 21 (P0-11): ÜST SINIRLAR — sınırsız max_safety_orders / grid_levels
    // tek API çağrısıyla bellek ayırmayı patlatıp TÜM motoru düşürebiliyordu
    // (3Commas: SO 50'de sınırlı). Sector-standart sınırlar + dürüst 422.
    if cfg.max_safety_orders > 100 {
        return Err(err422(format!(
            "config.max_safety_orders: {} > 100 — üst sınır 100 (tek istekle motor belleğini patlatmaya karşı; sektörde 3Commas 50 kullanır)",
            cfg.max_safety_orders
        )));
    }
    if let Some(levels) = cfg.grid_levels {
        if !(2..=500).contains(&levels) {
            return Err(err422(format!(
                "config.grid_levels: {levels} — aralık [2, 500] olmalı (sınırsız mesh tek istekle motoru düşürür)"
            )));
        }
    }

    // D-2: kurulum anında merdiven ön-inşası — runtime'da her poll'da ERROR
    // tekrarlayan hatalı kombinasyonlar (örn. kesir yerine yüzde yazılan
    // deviation_pct=0.5) burada dürüst 422 olur. Nominal çapa + gerçekçi
    // tick/step ölçeğiyle plan kurulabiliyorsa domain değerleri (sapma toplamı
    // < %100, ölçekler, tektonluk) geçerli. Grid türleri merdivensiz çalıştığı
    // için muaftır. (Çapa=1 + tick=0.01 alma: seviye fiyatları aynı tick'e
    // yuvarlanır ve gerçek olmayan "not below previous" hataları üretir.)
    //
    // Faz 22 (P1-19 kalanı): tick/step artık GERÇEK enstrüman metadata'sından
    // (venue çözümü; PAPER→BYBIT) — SHIBUSDT gibi mikro-tick sembollerinde
    // nominal 0.01/0.001 tamamen farklı plan üretiyordu. Metadata gelmezse
    // eski nominal değerler fallback (yapısal doğrulama yine çalışır).
    let inst_resolved = state::resolve_instrument(&cfg, &req.venue.to_uppercase(), &app).await;
    let mut stress_warning: Option<dca_kernel::risk_policy::AdvisoryWarning> = None;
    {
        let gridish = matches!(kind, StrategyKind::SpotGrid | StrategyKind::FuturesGrid | StrategyKind::InfinityGrid);
        if !gridish {
            // validate_cfg_positive yukarıda parse edilebilirliği garanti etti.
            let real_tick_ok = !inst_resolved.tick_size.is_zero() && inst_resolved.tick_size.is_positive();
            let tick = if real_tick_ok { inst_resolved.tick_size.clone() } else { Fraction::from_decimal_str("0.01").unwrap() };
            let step = if !inst_resolved.step_size.is_zero() && inst_resolved.step_size.is_positive() {
                inst_resolved.step_size.clone()
            } else {
                Fraction::from_decimal_str("0.001").unwrap()
            };
            let anchor = Fraction::from_decimal_str("1000").unwrap();
            let base = Fraction::from_decimal_str(&cfg.base_order_size).unwrap();
            let safety = Fraction::from_decimal_str(&cfg.safety_order_size).unwrap();
            let dev = Fraction::from_decimal_str(&cfg.deviation_pct).unwrap();
            let sscale = Fraction::from_decimal_str(&cfg.step_scale).unwrap();
            let vscale = Fraction::from_decimal_str(&cfg.volume_scale).unwrap();
            if let Err(e) = dca_math::build_dca_plan(
                &anchor,
                &base,
                &safety,
                &dev,
                &sscale,
                &vscale,
                cfg.max_safety_orders.min(1000),
                dca_math::m02::Direction::Long,
                &tick,
                &step,
                cfg.sizing_mode,
            ) {
                return Err(err422(format!("config merdiveni kurulamıyor: {e} — deviation_pct kesir olarak yazılır (0.005 = %0.5), kümülatif sapma %100'ü aşamaz")));
            }

            // Faz 22 (P1-11): M03 stres-teminat kapısı canlandırıldı — planı
            // GERÇEK çapa (venue ticker) ile kur; plan notional + şok-altında
            // (−%30) gereken teminatı hesapla; start_cash yetmiyorsa
            // M03_STRESS_COLLATERAL uyarısı düşür (onay akışı: RISK_ACK -
            // gerekli; fiziksel sermaye kısıtı onaylansa bile GATE6
            // fiziksel değişmezdir — safetiler dolmaz).
            let start_cash_frac = Fraction::from_decimal_str(&req.start_cash).unwrap_or_else(|_| Fraction::zero());
            let is_spot_cfg = !kind.is_futures();
            let anchor_real = match app.adapter(&req.venue.to_uppercase()) {
                Some(a) => match a.ticker(if is_spot_cfg { "spot" } else { "linear" }, &req.symbol).await {
                    Ok(t) if t.last_price.is_positive() => Some(t.last_price),
                    _ => None,
                },
                None => None,
            };
            if let Some(anchor_real) = anchor_real {
                if let Ok(plan) = dca_math::build_dca_plan(
                    &anchor_real,
                    &base,
                    &safety,
                    &dev,
                    &sscale,
                    &vscale,
                    cfg.max_safety_orders.min(1000),
                    dca_math::m02::Direction::Long,
                    &tick,
                    &step,
                    cfg.sizing_mode,
                ) {
                    let leverage = if is_spot_cfg {
                        Fraction::one()
                    } else {
                        Fraction::from_decimal_str(&cfg.leverage).unwrap_or(Fraction::one())
                    };
                    if let Ok(pn) = dca_math::m03_m07::plan_notional(
                        &base.div(&anchor_real).unwrap_or_else(|_| Fraction::one()),
                        &anchor_real,
                        &plan,
                        &leverage,
                        &inst_resolved.taker_fee_rate,
                        dca_math::m02::Direction::Long,
                        cfg.sizing_mode,
                        &base,
                        &safety,
                    ) {
                        // M03 varsayılan şok senaryosu: −%30 fiyat, MMR %0.5,
                        // ek tampon %0.5, funding tamponu %0.2, gap tamponu %1
                        // (notional oranı — M03 doküman varsayılanları).
                        let shock = Fraction::from_decimal_str("-0.30").unwrap();
                        let mm = Fraction::from_decimal_str("0.005").unwrap();
                        let extra = Fraction::from_decimal_str("0.005").unwrap();
                        let funding_buf = Fraction::from_decimal_str("0.002").unwrap();
                        let gap = Fraction::from_decimal_str("0.01").unwrap();
                        if let Ok(required) = dca_math::m03_m07::required_collateral(
                            &pn.planned_gross_notional,
                            &pn.estimated_initial_margin,
                            &mm,
                            &shock,
                            &pn.planned_gross_notional.mul(&extra).unwrap_or_else(|_| Fraction::zero()),
                            &pn.planned_gross_notional.mul(&funding_buf).unwrap_or_else(|_| Fraction::zero()),
                            &pn.planned_gross_notional.mul(&gap).unwrap_or_else(|_| Fraction::zero()),
                            is_spot_cfg,
                        ) {
                            let available = match start_cash_frac.mul(&leverage) {
                                Ok(x) => x,
                                Err(_) => start_cash_frac.clone(),
                            };
                            if !required.is_zero()
                                && dca_math::fraction::Fraction::cmp(&required, &available) == std::cmp::Ordering::Greater
                            {
                                stress_warning = Some(dca_kernel::risk_policy::AdvisoryWarning {
                                    code: "M03_STRESS_COLLATERAL".into(),
                                    title: "M03 STRES TEMİNAT YETERSİZLİĞİ".into(),
                                    message: format!(
                                        "Tam merdiven ({}) şok senaryosunda (−%30) {} teminat/gider gerektiriyor; kullanılabilir sermaye (start_cash×kaldıraç) {}. Eksik kademe kademeleri GATE6 tarafından bekletilir (Wait) ya da atlanır (Skip) — tam sermaye şoku üstleniyorsunuz.",
                                        pn.planned_gross_notional.to_half_even_display(4),
                                        required.to_half_even_display(4),
                                        available.to_half_even_display(4)
                                    ),
                                    severity: "WARN".into(),
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    // Task 16 — Grid g_min fee-breaker: Enforce modda kurulum reddedilir
    // (eski davranış korunur); Permissive modda uyarıyla devam edilir.
    let is_grid_kind = matches!(
        kind,
        StrategyKind::SpotGrid | StrategyKind::FuturesGrid | StrategyKind::InfinityGrid
    );
    let inst_for_warnings: Instrument = inst_resolved;
    // Task 16 — Grid g_min fee-breaker: Enforce modda kurulum reddedilir.
    // Faz 22 (N-11): kapı artık GridEngine::new ile AYNI matematikten geçer
    // (gerçek mesh + en dar segment + asimetrik ücret) — create 422 ile
    // runtime reddi farklı hüküm veremeyecek.
    if is_grid_kind && cfg.risk_policy.grid_fee_breaker == GuardAction::Enforce {
        if let Some((per_level, g_min)) = dca_strategy::grid_fee_breaker_violation(&cfg, &inst_for_warnings) {
            return Err(err422(format!(
                "fee-breaker: en dar mesh segmenti {} <= g_min {}; aralığı büyütün veya kademe sayısını düşürün",
                per_level.to_half_even_display(6),
                g_min.to_half_even_display(6)
            )));
        }
    }

    // Task 16 — TÜM advisory uyarıları (tek kaynak: dca_strategy::advisory).
    let mut warnings = dca_strategy::compute_warnings(&cfg, &inst_for_warnings);
    // Faz 22 (P1-11): M03 stres-teminat uyarısı onay akışına girer.
    if let Some(w) = stress_warning.take() {
        warnings.push(w);
    }
    // Faz 22 (P1-24): MOTOR TARAFI FEE-GUARD — hedef kâr, tur maliyetini
    // (giriş taker + çıkış maker + beklenen funding) karşılamıyorsa deal
    // yapısal olarak kâra kapanamaz. Sector-standardı (3Commas) minimum TP'yi
    // maliyet üstü şart koşar; burada RISK_ACK onay akışına girer.
    {
        let entry_fee = &inst_for_warnings.taker_fee_rate; // BASE market taker
        let exit_fee = &inst_for_warnings.maker_fee_rate; // TP resting limit maker
        let expected_funding = Fraction::from_decimal_str("0.002").unwrap(); // koruyucu varsayım: tutma süresi içinde ~%0.2 funding
        if let (Ok(tp), Ok(rt_cost)) = (
            Fraction::from_decimal_str(&cfg.target_profit_pct),
            entry_fee.add(exit_fee).and_then(|x| x.add(&expected_funding)),
        ) {
            if dca_math::fraction::Fraction::cmp(&tp, &rt_cost) != std::cmp::Ordering::Greater {
                warnings.push(dca_kernel::risk_policy::AdvisoryWarning {
                    code: "M02_FEE_GUARD".into(),
                    title: "M02 ÜCRET EŞİĞİ".into(),
                    message: format!(
                        "Hedef kâr ({}) tur maliyetinin (giriş taker {} + çıkış maker {} + beklenen funding {}) altında veya eşit — deal yapısal olarak net kâra kapanamaz. TP'yi maliyetin ÜSTÜNE çekin ya da riski bilinçli üstlenin.",
                        tp.to_half_even_display(6),
                        entry_fee.to_half_even_display(6),
                        exit_fee.to_half_even_display(6),
                        expected_funding.to_half_even_display(6)
                    ),
                    severity: "WARN".into(),
                });
            }
        }
    }

    // M20 kullanıcı AÇIKÇA ZORUNLU seçtiyse eski fail-closed davranışı sürer.
    if cfg.risk_policy.m20_sl_ladder_guard == GuardAction::Enforce
        && warnings.iter().any(|w| w.code == "M20_SL_CONFLICT")
    {
        return Err(err422(
            "M20 CONFLICT: stop loss makes safety orders unreachable. Widen SL, reduce deviation, or switch the M20 guard to UYARI+İZİN (permissive).",
        ));
    }

    // SIFIR İŞLEM ENGELİ: uyarılar engellemez — onay ister. UI checkbox'ı
    // (Acknowledge & Proceed) işaretliyse konfigürasyon aynen kaydedilir.
    if !warnings.is_empty() && !req.risk_acknowledged {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({
                "error": "RISK_ACKNOWLEDGMENT_REQUIRED: yukarıdaki riskleri okudum, anladım ve sorumluluğunu üstleniyorum kutusunu işaretleyin",
                "code": "RISK_ACKNOWLEDGMENT_REQUIRED",
                "warnings": warnings,
            })),
        )
            .into_response());
    }

    // EX-01: bot kimliği kısaltıldı (b- + 8 hex = 10 char) — orderLinkId/clOrdId
    // içinde bot prefix olarak kullanılır; Bybit 36 / OKX 32 char sınırına uygun.
    let short_uuid = format!("{}", Uuid::new_v4().simple());
    let id = format!("b-{}", short_uuid.chars().take(8).collect::<String>());
    let mode = req.mode.to_uppercase();
    if !matches!(mode.as_str(), "PAPER" | "LIVE") {
        return Err(err422(format!("geçersiz mode '{mode}' — geçerli değerler: PAPER | LIVE")));
    }
    let _live_ownership_guard = if mode == "LIVE" {
        Some(lock_live_ownership(&app, "", &req.venue, &cfg.symbol).await.map_err(err422)?)
    } else {
        None
    };
    let status = "RUNNING";
    // M-5: start_cash pozitif olmalı — negatif/sıfır sermaye anlamsız bir
    // kurulumun G6'yı hiç test etmeden geçmesine yol açardı.
    let start_cash = match Fraction::from_decimal_str(&req.start_cash) {
        Ok(x) if x.is_positive() => x,
        Ok(_) => return Err(err422(format!("start_cash 0'dan büyük olmalı (gelen: {})", req.start_cash))),
        Err(e) => return Err(err422(e)),
    };

    // Futures DCA: isolated linear margin requires explicit leverage bounds.
    if kind == StrategyKind::FuturesDca {
        match Fraction::from_decimal_str(&cfg.leverage) {
            Ok(x) if !x.is_zero() && x >= Fraction::one() && x <= Fraction::from_u64(50) => {}
            _ => return Err(err422("FUTURES_DCA requires config.leverage in [1, 50]")),
        }
    }

    // Hedge validation (M18): ratio in (0, 1], combined exit target > 0.
    // Task 21 — koruma tetikleyici (hedge_drawdown_trigger_pct) artık config'ten:
    // (0, 100] aralığında PERCENT olmalı ("6" = %6); hardcode dönemi kapandı.
    if matches!(kind, StrategyKind::HedgeProtective | StrategyKind::HedgeDual) {
        match Fraction::from_decimal_str(&cfg.hedge_ratio) {
            Ok(r) if r.is_positive() && r <= Fraction::one() => {}
            _ => return Err(err422("Hedge oranı (hedge_ratio) (0, 1] aralığında olmalı — örn. 0.5 = ana pozisyonun %50'si")),
        }
        match Fraction::from_decimal_str(&cfg.combined_target_profit) {
            Ok(t) if t.is_positive() => {}
            _ => return Err(err422("Birleşik kâr hedefi (combined_target_profit) 0'dan büyük olmalı")),
        }
        match Fraction::from_decimal_str(&cfg.hedge_drawdown_trigger_pct) {
            Ok(d) if d.is_positive() && d <= Fraction::from_u64(100) => {}
            _ => return Err(err422("Koruma tetikleyici (hedge_drawdown_trigger_pct) (0, 100] aralığında olmalı — örn. 6 = %6 düşüşte koruma bacağı")),
        }
    }

    // LIVE mode arming: BYBIT fill-reconciliation-verified end to end; OKX
    // P4-3'te aynı sweep sözleşmesiyle açıldı (Binance hâlâ PAPER-only).
    // a LIVE bot requires an active vault key for the venue.
    let mut credentials: Option<VenueCreds> = None;
    if mode == "LIVE" {
        let venue_uc = req.venue.to_uppercase();
        if venue_uc != "BYBIT" && venue_uc != "OKX" {
            return Err(err422("Bu venue'de LIVE kanal kapalı (Binance fill-reconciliation ile doğrulanmadı) — PAPER modunu kullan"));
        }
        // Faz 0.5 (B-6): LIVE Mainnet Gate — venue bazlı testnet/demo kapısı (Arena P0-3 düzeltmesi)
        let is_testnet_env = match venue_uc.as_str() {
            "BYBIT" => std::env::var("DCA_BYBIT_TESTNET").as_deref() == Ok("1"),
            "OKX" => std::env::var("DCA_OKX_DEMO").as_deref() == Ok("1") || std::env::var("DCA_OKX_TESTNET").as_deref() == Ok("1"),
            _ => false,
        };
        let allow_mainnet = std::env::var("DCA_ALLOW_MAINNET").as_deref() == Ok("1");
        if !is_testnet_env && !allow_mainnet {
            return Err(err422(format!(
                "LIVE_MAINNET_LOCKED: {venue_uc} gerçek ana ağda (Mainnet) LIVE bot başlatmak için DCA_ALLOW_MAINNET=1 ortam değişkeni zorunludur (B-6 fail-closed kapısı). Testnet/Demo için {} kullanın.",
                if venue_uc == "OKX" { "DCA_OKX_DEMO=1" } else { "DCA_BYBIT_TESTNET=1" }
            )));
        }
        // Faz 13B: HEDGE LIVE kilidi KALDIRILDI — bacak-başına stepper (EX-06
        // çözümü) + positionIdx eşlemesi (Faz 4.5/5) tam; Demo kabulü sonrası canlı.
        // Hesap hedge-moddaysa POS_MODE denetimi emirleri positionIdx ile eşler.
        credentials = match store::load_active_key(&app.db, &app.vault, &venue_uc) {
            Ok(Some(c)) => {
                if venue_uc == "OKX" && c.passphrase.as_deref().map(|p| p.trim().is_empty()).unwrap_or(true) {
                    return Err(err422("OKX LIVE için vault anahtarında passphrase gerekli — anahtarı passphrase ile yeniden ekleyin"));
                }
                Some(c)
            }
            Ok(None) => return Err(err422(format!("LIVE mod için {venue_uc} vault anahtarı gerekli — Ayarlar > Anahtar Kasası'ndan ekleyin"))),
            Err(e) => return Err(err422(format!("vault anahtarı çözülemedi: {e}"))),
        };
    }

    // Faz 0.1 — LIVE sertifika kapısı (v3 §10 "rozetsiz strateji canlıya eklenemez").
    // Sihirbaz atlanarak API'den sertifikasız LIVE açılmasını engeller: kapı, botun
    // kendi config'iyle motor içinde yeniden koşar (istemcinin gönderdiği hazır
    // sertifika JSON'una güvenilmez — sahteciliğe kapalı). PAPER modu etkilenmez.
    if mode == "LIVE" {
        let venue_uc = req.venue.to_uppercase();
        let is_fut = kind.is_futures();
        let category = if is_fut { "linear" } else { "spot" };
        let adapter = match app.adapter(&venue_uc) {
            Some(a) => a,
            None => return Err(err422("LIVE sertifika kapısı: venue desteklenmiyor")),
        };
        let iv = match engine::bybit_interval(&req.interval) {
            Ok(v) => v,
            Err(e) => return Err(err422(e)),
        };
        // Kapı penceresi Lab varsayılanıyla aynı (500 bar); üst sınır backtest ile aynı (1000).
        const GATE_BARS: usize = 500;
        let bars = match routes_backtest::fetch_backtest_klines(adapter, &venue_uc, category, &req.symbol, &iv, GATE_BARS).await {
            Ok(b) if !b.is_empty() => b,
            _ => return Err(err422("LIVE sertifika kapısı: market verisi alınamadı — veri yokken KALDI (fail-closed)")) ,
        };
        // Faz 4 (H-6): venue-dogru mikro-yapi LIVE icin ZORUNLU — metadata
        // alinamazsa varsayilan tick/step ile canli emir kabul edilmez.
        let inst = match adapter.instruments(category, Some(&req.symbol)).await {
            Ok(l) if !l.is_empty() => l.into_iter().next().unwrap(),
            _ => {
                return Err(err422(&format!(
                    "LIVE sertifika kapısı: {} enstrüman meta verisi alınamadı — venue-doğru tick/step/fee olmadan canlı emir kabul edilmez (fail-closed)",
                    req.symbol
                )))
            }
        };
        let (funding, funding_err): (Vec<(i64, String)>, Option<String>) = if is_fut {
            match adapter.funding_history(&req.symbol, 200).await {
                Ok(v) => (v, None),
                Err(e) => (Vec::new(), Some(e)),
            }
        } else {
            (Vec::new(), None)
        };
        let funding_ok = funding_err.is_none();
        let bars_n: Vec<dca_kernel::Bar> = bars
            .into_iter()
            .map(|k| dca_kernel::Bar { t: k.t, o: k.o, h: k.h, l: k.l, c: k.c, v: k.v })
            .collect();
        let gate_cfg = cfg.clone();
        let gate_cash = start_cash.clone();
        let gate_attempts = req.attempts;
        let (gate_cert, gate_base) = tokio::task::spawn_blocking(move || {
            let base = run_backtest(&bars_n, &inst, &gate_cfg, gate_cash.clone(), dca_math::sessions::SessionType::Crypto24_7, &funding).ok();
            // Faz 18 (P0-10): denemeler de base ile aynı funding takviminde —
            // ortak maliyet tabanı.
            let cert = certify(&bars_n, &inst, &gate_cfg, gate_cash, dca_math::sessions::SessionType::Crypto24_7, 100, &funding, gate_attempts);
            (cert, base)
        })
        .await
        .map_err(|e| err422(format!("LIVE sertifika kapısı çalışamadı: {e}")))?;
        let gate_cert = match gate_cert {
            Ok(c) => c,
            Err(e) => return Err(err422(format!("LIVE sertifika kapısı: sertifika üretilemedi ({e}) — KALDI"))),
        };
        let deals = gate_base.as_ref().map(|b| b.metrics.deals).unwrap_or(0);
        let gate = certify_verdict(&gate_cert, deals, funding_ok, is_fut);
        if gate.verdict != "GECTI" {
            let failed: Vec<String> = gate.gates.iter().filter(|g| !g.pass).map(|g| format!("{} (eşik {}, değer {})", g.label, g.threshold, g.value)).collect();
            // ÜRÜN-1: bilinçli-onay istisnası — kapı felsefesi korunur (onay
            // şart + kalıcı WARN), Demo/uzman kullanımı için kanal açılır.
            // Faz 28 (P1-4): onay gerekçesiz kabul edilmez — audit zinciri
            // "kim + neden" ikilisini tam istemek zorundadır.
            let cert_reason = req
                .cert_reason
                .as_deref()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.chars().take(300).collect::<String>());
            if req.cert_acknowledged {
                let Some(cert_reason) = cert_reason else {
                    state::persist_event(&app, &id, "WARN", "CERT_GATE", "cert_acknowledged=true ancak cert_reason yok — istisna reddedildi").await;
                    return Err((
                        StatusCode::UNPROCESSABLE_ENTITY,
                        Json(json!({
                            "error": "LIVE sertifika kapısı: bilinçli onay istisnası için cert_reason zorunlu",
                            "code": "CERT_ACK_REASON_REQUIRED",
                            "verdict": gate.verdict,
                            "gates": gate.gates,
                            "hint": "cert_acknowledged=true + cert_reason (örn. 'backtest 200 deal üstünde doğrulandı, kapı eşikleri manuel gözden geçirildi') gönderin",
                        })),
                    )
                        .into_response());
                };
                let failed_list = failed.join(" · ");
                state::persist_event(&app, &id, "WARN", "CERT_GATE", &format!("sertifika kapısı KALDI ama kullanıcı bilinçli onayıyla AŞILDI (cert_acknowledged=true, gerekçe: \"{cert_reason}\"). Kalan kapılar: {failed_list}")).await;
                let _ = app.sse.send(SseEvent {
                    event: "RISK".into(),
                    data: json!({"botId": id, "code": "CERT_GATE_ACK", "message": format!("LIVE sertifika kapısı bilinçli onayla aşıldı (gerekçe: {cert_reason}) — kalan kapılar: {failed_list}"), "ts": state::now_ms()}).to_string(),
                });
                state::persist_event(&app, &id, "INFO", "CERT_GATE", "bilinçli onaylı istisna ile kuruluma devam ediliyor").await;
            } else {
            let mut reason = format!("LIVE sertifika kapısı: KALDI — sertifikasız strateji canlıya eklenemez. Kalan kapılar: {}", failed.join(" · "));
            if let Some(e) = funding_err {
                reason.push_str(&format!(" [funding geçmişi alınamadı: {e}]"));
            }
            state::persist_event(&app, &id, "WARN", "CERT_GATE", &reason).await;
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({
                    "error": reason,
                    "code": "CERT_GATE_FAILED",
                    "verdict": gate.verdict,
                    "gates": gate.gates,
                    "hint": "bilinçli sorumluluk onayıyla devam için cert_acknowledged=true + cert_reason (gerekçe metni) gönderin (WARN olayıyla kayıt altına alınır)",
                })),
            )
                .into_response());
            }
        }
        state::persist_event(&app, &id, "INFO", "CERT_GATE", &format!("LIVE sertifika kapısı: GECTI ({}/{} kapı, {} deal, {})", gate.passed, gate.total, deals, venue_uc)).await;
    }

    if let Err(e) = state::create_runtime(&app, &id, &req.name, &mode, &req.venue.to_uppercase(), status, cfg.clone(), &req.interval, start_cash.clone(), credentials).await {
        return Err(err422(e));
    }

    {
        let conn = match app.db.conn.lock() { Ok(c) => c, Err(e) => return Err(err422(e.to_string())) };
        conn.execute(
            "INSERT INTO bots(id, name, kind, venue, mode, symbol, status, config, stats, interval, start_cash, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,'{}',?9,?10,?11)",
            rusqlite::params![
                id,
                req.name,
                kind.label(),
                req.venue.to_uppercase(),
                mode,
                req.symbol,
                status,
                serde_json::to_string(&cfg).map_err(|e| err422(e.to_string()))?,
                req.interval,
                start_cash.to_string(),
                state::now_ms()
            ],
        )
        .map_err(|e| err422(e.to_string()))?;
    }
    state::persist_event(&app, &id, "INFO", "LIFECYCLE", "bot created & running").await;
    // Task 20 L5 — makro takvim staleness'u: takvim 2026 sonuna kadar kapsar;
    // bayat takvimde Gate 7 sessizce no-op olur. Sessiz koruma kaybı YOK —
    // her yeni bot için bir kez görünür uyarı.
    if dca_math::macro_calendar::is_stale(state::now_ms()) {
        let msg = format!(
            "MAKRO_TAKVIM_BAYAT: makro kara pencere takvimi (FOMC/CPI) son olayı {} tarihinde — takvim güncellenmediği sürece Gate 7 makro koruması etkisiz. Yeni yıl takvimi yüklenmeli.",
            chrono::DateTime::from_timestamp_millis(dca_math::macro_calendar::last_event_ts_ms())
                .map(|d| d.format("%Y-%m-%d").to_string())
                .unwrap_or_else(|| "?".into())
        );
        state::persist_event(&app, &id, "WARN", "RISK:MACRO_CALENDAR", &msg).await;
    }
    if !warnings.is_empty() {
        let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();
        state::persist_event(&app, &id, "WARN", "RISK_POLICY", &format!("advisory warnings acknowledged: {}", codes.join(", "))).await;
        let _ = app.sse.send(SseEvent {
            event: "RISK".into(),
            data: json!({"botId": id, "code": "RISK_POLICY_ACK", "message": format!("Kullanıcı riskleri onayladı: {}", codes.join(", ")), "ts": state::now_ms()}).to_string(),
        });
    }
    let _ = app.sse.send(SseEvent {
        event: "BOT_STATE".into(),
        data: json!({"botId": id.clone(), "status": "RUNNING", "action": "CREATED"}).to_string(),
    });
    // 201 sözleşmesi: e2e + UI `CreateBotResponse {id, status, warnings?}` bekler
    // (onaylanan advisory uyarılar yanıtta yankılanır).
    Ok((
        StatusCode::CREATED,
        Json(json!({
            "id": id,
            "status": "RUNNING",
            "warnings": warnings,
        })),
    )
        .into_response())
}

async fn get_bot(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>, axum::response::Response> {
    let rts = app.runtimes.lock().await;
    let rt = rts.get(&id).ok_or_else(|| err422("bot not found"))?;
    let mut v = rt.snapshot_json();
    // ladder preview — Task 21: Hedge botlarda ana bacağın (long) DCA merdiveni
    // döner (eskiden hedge botlar ladder'sızdı → UI "0 seviye" gösteriyordu).
    if let dca_strategy::EngineTag::Dca(e) = &rt.engine_kind {
        let ladder: Vec<Value> = e
            .ladder
            .iter()
            .map(|l| json!({"level": l.level_index, "price": l.price.to_string(), "qty": l.qty.to_string(), "notional": l.notional.to_string(), "cumDev": l.cumulative_deviation.to_string()}))
            .collect();
        v["ladder"] = json!(ladder);
    } else if let dca_strategy::EngineTag::Grid(g) = &rt.engine_kind {
        let mesh: Vec<Value> = g.mesh.iter().enumerate().map(|(i, p)| json!({"level": i, "price": p.to_string(), "armed": g.level_armed[i]})).collect();
        v["ladder"] = json!(mesh);
    } else if let dca_strategy::EngineTag::Hedge(h) = &rt.engine_kind {
        let ladder: Vec<Value> = h
            .main
            .ladder
            .iter()
            .map(|l| json!({"level": l.level_index, "price": l.price.to_string(), "qty": l.qty.to_string(), "notional": l.notional.to_string(), "cumDev": l.cumulative_deviation.to_string()}))
            .collect();
        v["ladder"] = json!(ladder);
    }
    Ok(Json(v))
}

#[derive(Deserialize)]
struct PatchBotReq {
    target_profit_pct: Option<String>,
    stop_loss_pct: Option<Option<String>>,
    trailing_enabled: Option<bool>,
    status: Option<String>,
    /// H-7 tamamı: webhook secret rotasyonu — verildiyse bota yeni secret
    /// atanır (uzunluk 8-128; snapshot'a yazar → restart sabiti).
    webhook_secret: Option<String>,
}

async fn patch_bot(State(app): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<PatchBotReq>) -> Result<Json<Value>, axum::response::Response> {
    let _live_ownership_guard = if req.status.as_deref().is_some_and(|s| s.eq_ignore_ascii_case("RUNNING")) {
        let identity = {
            let runtimes = app.runtimes.lock().await;
            runtimes.get(&id).filter(|rt| rt.mode.eq_ignore_ascii_case("LIVE"))
                .map(|rt| (rt.venue.clone(), rt.cfg.symbol.clone()))
        };
        if let Some((venue, symbol)) = identity {
            Some(lock_live_ownership(&app, &id, &venue, &symbol).await.map_err(err422)?)
        } else {
            None
        }
    } else {
        None
    };
    let mut rts = app.runtimes.lock().await;
    let rt = rts.get_mut(&id).ok_or_else(|| err422("bot not found"))?;
    if req.status.is_some() {
        let s = req.status.clone().unwrap().to_uppercase();
        // H-10: status tek enum kümesidir — "FOO" gibi değerler kalıcılaşarak
        // botu supervisor/restore filtrelerinin dışına düşürürdü.
        if !matches!(s.as_str(), "RUNNING" | "PAUSED" | "STOPPED" | "FROZEN") {
            return Err(err422(format!("geçersiz status '{s}' — geçerli küme: RUNNING | PAUSED | STOPPED | FROZEN")));
        }
        rt.status = s.clone();
        state::persist_status(&app, &id, &s);
        let _ = app.sse.send(SseEvent { event: "BOT_STATE".into(), data: json!({"botId": id, "status": s}).to_string() });
    } else if let dca_strategy::EngineTag::Dca(e) = &mut rt.engine_kind {
        e.patch(req.target_profit_pct.clone(), req.stop_loss_pct.clone(), req.trailing_enabled)
            .map_err(|e| err422(e))?;
        // Arena 1.10 Düzeltmesi: rt.cfg alanlarını da güncelle ki DB'ye ve snapshot'a güncel config yazılsın.
        if let Some(tp) = &req.target_profit_pct {
            rt.cfg.target_profit_pct = tp.clone();
        }
        if let Some(sl) = &req.stop_loss_pct {
            rt.cfg.stop_loss_pct = sl.clone();
        }
        if let Some(tr) = req.trailing_enabled {
            rt.cfg.trailing_enabled = tr;
        }
        // BOT-06: yamayı DB config'ine de kalıcı yaz — restart sonrası korunur.
        {
            let conn = app.db.conn.lock().map_err(|e| err422(e.to_string()))?;
            conn.execute(
                "UPDATE bots SET config=?1 WHERE id=?2",
                rusqlite::params![serde_json::to_string(&rt.cfg).map_err(|e| err422(e.to_string()))?, id],
            )
            .map_err(|e| err422(e.to_string()))?;
        }
        // H-7: webhook secret rotasyonu (isteğe bağlı) — snapshot'a yansır.
        if let Some(new_secret) = &req.webhook_secret {
            let t = new_secret.trim();
            if t.len() < 8 || t.len() > 128 {
                return Err(err422("webhook_secret 8-128 karakter olmalı"));
            }
            rt.webhook_secret = t.to_string();
            crate::state::save_runtime_state(&app, &id, rt);
            state::persist_event(&app, &id, "INFO", "WEBHOOK", "webhook secret rotasyonu yapıldı (yeni secret snapshot'a yazıldı)").await;
        }
        state::persist_event(&app, &id, "INFO", "PATCH", "dynamic config patch applied & persisted").await;
        let _ = app.sse.send(SseEvent { event: "BOT_STATE".into(), data: json!({"botId": id, "action": "PATCHED"}).to_string() });
        // Faz 13 dilim 2: patch sonrası cache taze (status/faz hemen yansır).
        {
            let snap_v = rt.snapshot_json();
            app.ui_cache.lock().await.insert(id.clone(), snap_v);
        }
    } else if matches!(rt.engine_kind, dca_strategy::EngineTag::Grid(_)) {
        // Grid engines have no TP/SL deal lifecycle — a silent no-op would fake success.
        let wants_tpsl_patch = req.target_profit_pct.is_some() || req.stop_loss_pct.is_some() || req.trailing_enabled.is_some();
        if wants_tpsl_patch {
            return Err(err422("Grid stratejisi TP/SL yaması desteklenmez — grid botlar mesh seviyeleriyle yönetilir"));
        }
    } else if matches!(rt.engine_kind, dca_strategy::EngineTag::Hedge(_)) {
        // Hedge engines manage TP via the combined-exit target; per-leg TP/SL
        // patches would desynchronize the legs — refuse honestly instead.
        let wants_tpsl_patch = req.target_profit_pct.is_some() || req.stop_loss_pct.is_some() || req.trailing_enabled.is_some();
        if wants_tpsl_patch {
            return Err(err422("Hedge stratejisinde TP/SL yaması desteklenmez — çıkış birleşik kâr hedefiyle (combined_target_profit) yönetilir"));
        }
    }
    Ok(Json(json!({"ok": true})))
}

/// BOT-07: silmeden önce açık venue emirleri iptal edilir (sahipsiz bırakılmaz).
async fn delete_bot(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>, axum::response::Response> {
    // Serialize deletion with the bot's in-flight poll/order cascade. The
    // guard is per bot, so unrelated bots continue while venue cancellations
    // run without holding the shared runtime map lock.
    let _poll_guard = acquire_bot_poll_guard(&app, &id).await;
    // LIVE botun açık emirlerini iptal et. Mark STOPPED before network I/O so
    // a failed cancel leaves a visible, retained bot that cannot add risk.
    let (is_live, creds_opt, venue_str, category_str, symbol_str, snapshot) = {
        let mut rts = app.runtimes.lock().await;
        let rt = match rts.get_mut(&id) {
            Some(r) => r,
            None => return Err(err404("bot not found")),
        };
        // Arena 1.10 Düzeltmesi: LIVE açık pozisyonu varken bot silinemez (sahipsiz pozisyon kalmaması için fail-closed)
        let live_exposure = !rt.stepper.position.qty.is_zero()
            || rt.hedge_stepper.as_ref().is_some_and(|stepper| !stepper.position.qty.is_zero());
        if rt.mode == "LIVE" && live_exposure {
            return Err(err422(format!(
                "Açık pozisyonu olan LIVE bot silinemez (ana miktar: {}, hedge miktarı: {}). Önce tüm bacakları kapatıp pozisyon uzlaşmasını bekleyin.",
                rt.stepper.position.qty.to_half_even_display(8),
                rt.hedge_stepper.as_ref().map(|stepper| stepper.position.qty.to_half_even_display(8)).unwrap_or_else(|| "0".into())
            )));
        }
        rt.status = "STOPPED".into();
        state::persist_status(&app, &id, "STOPPED");
        let snapshot = rt.snapshot_json();
        (
            rt.mode == "LIVE",
            rt.credentials.clone(),
            rt.venue.clone(),
            if rt.cfg.kind.is_futures() { "linear".to_string() } else { "spot".to_string() },
            rt.cfg.symbol.clone(),
            snapshot,
        )
    };
    app.ui_cache.lock().await.insert(id.clone(), snapshot);
    let _ = app.sse.send(SseEvent {
        event: "BOT_STATE".into(),
        data: json!({"botId": id, "status": "STOPPED", "action": "DELETING"}).to_string(),
    });
    if is_live {
        let creds = creds_opt.ok_or_else(|| err422("LIVE bot silinmedi: aktif kasa anahtarı olmadan açık emir güvenle uzlaştırılamaz"))?;
        let adapter = app.adapter(&venue_str).ok_or_else(|| err422("LIVE bot silinmedi: venue adapter bulunamadı"))?;
        let orders = match adapter.open_orders(&creds, &category_str, &symbol_str).await {
            Ok(orders) => orders,
            Err(e) => {
                state::persist_event(&*app, &id, "WARN", "DELETE", &format!("open order listesi alınamadı; bot silinmedi: {e}")).await;
                return Err(err422(format!("LIVE bot silinmedi: açık emirler doğrulanamadı ({e})")));
            }
        };
        // Only this bot's order ids are canceled. Any failed cancellation keeps
        // the runtime available for fill reconciliation and a safe retry.
        let prefix = format!("{id}-");
        let mut failed = Vec::new();
        for order in orders.iter().filter(|order| order.order_link_id.starts_with(&prefix)) {
            if let Err(e) = adapter.cancel_order(&creds, &category_str, &symbol_str, &order.order_link_id).await {
                failed.push(format!("{}: {e}", order.order_link_id));
            }
        }
        if !failed.is_empty() {
            let detail = failed.join("; ");
            state::persist_event(&*app, &id, "CRITICAL", "DELETE", &format!("emir iptali başarısız; bot silinmedi: {detail}")).await;
            return Err(err422(format!("LIVE bot silinmedi; bazı emirler iptal edilemedi: {detail}")));
        }
    }
    app.runtimes.lock().await.remove(&id);
    // H-1: bot silinince single-flight kilidi de kalkar (harita sızıntısı yok).
    app.poll_locks.lock().await.remove(&id);
    app.ui_cache.lock().await.remove(&id);
    // Faz 14-A (M-10): DB kaydı da temizlenir — bots satırları sonsuz birikmesin.
    // Fill/event/equity geçmişi analiz için korunur (bot_id artık bots'ta yok).
    if let Ok(conn) = app.db.conn.lock() {
        let _ = conn.execute("DELETE FROM bots WHERE id=?1", rusqlite::params![id]);
        let _ = conn.execute("DELETE FROM live_orders WHERE bot_id=?1", rusqlite::params![id]);
        let _ = conn.execute("DELETE FROM runtime_state WHERE bot_id=?1", rusqlite::params![id]);
    }
    state::persist_event(&*app, &id, "INFO", "LIFECYCLE", "bot silindi (DB kaydı temizlendi; fill/equity geçmişi korundu)").await;
    let _ = app.sse.send(SseEvent { event: "BOT_STATE".into(), data: json!({"botId": id, "status": "STOPPED", "action": "DELETED"}).to_string() });
    Ok(Json(json!({"ok": true})))
}

#[derive(Deserialize)]
struct ActionReq {
    action: String,
    #[serde(default)]
    amount: Option<String>,
}

async fn bot_actions(State(app): State<Arc<AppState>>, Path(id): Path<String>, Json(req): Json<ActionReq>) -> Result<Json<Value>, axum::response::Response> {
    // Faz 13A (IEA, handler ölçeği): LIVE ADD_FUNDS — plan kısa kilitte, AĞ
    // ÇAĞRISI kilit dışında, uygulama yeni kilitte. PAPER normal akışa düşer.
    if req.action == "ADD_FUNDS" {
        // Faz 17 (P0-7): ADD_FUNDS da yürütme şalterlerine saygılı — şalter
        // devredeyken borsaya emir gitmez (şalterin anlamı korunur).
        if app.kill_engaged().await {
            state::persist_event(&*app, &id, "WARN", "KILL_SWITCH", "ADD_FUNDS reddedildi — KILL_SWITCH devrede").await;
            return Err(err422("KILL_SWITCH devrede — ADD_FUNDS reddedildi"));
        }
        if app.freeze_engaged().await {
            state::persist_event(&*app, &id, "WARN", "FREEZE", "ADD_FUNDS reddedildi — FREEZE devrede").await;
            return Err(err422("FREEZE devrede — ADD_FUNDS reddedildi"));
        }
        let (mode, venue, mark, step_size, planned) = {
            let rts = app.runtimes.lock().await;
            let rt = rts.get(&id).ok_or_else(|| err422("bot not found"))?;
            let mark = rt
                .stepper
                .last_close
                .as_deref()
                .and_then(|s| Fraction::from_any_str(s).ok());
            let plan = if rt.mode == "LIVE" {
                let amount = req.amount.clone().ok_or_else(|| err422("amount required"))?;
                let quote = Fraction::from_decimal_str(&amount).map_err(|e| err422(e))?;
                // Faz 26 (KAPSAMLI_DENETİM P1): pozitiflik değişmezi — LIVE yolu.
                if !quote.is_positive() {
                    return Err(err422("ADD_FUNDS amount must be > 0"));
                }
                let mark_v = mark.clone().ok_or_else(|| err422("no mark price yet"))?;
                let qty = dca_math::m02::qty_down(&quote.div(&mark_v).map_err(|e| err422(e.to_string()))?, &rt.stepper.instrument.step_size)
                    .map_err(|e| err422(e.to_string()))?;
                if qty.is_zero() {
                    return Err(err422("amount too small for one unit"));
                }
                let order_id = format!("{}-MANUAL-{}", rt.bot_id, state::now_ms());
                // Faz 17 (P0-7): yön POZİSYON yönünden — short bot fon eklerken
                // Sell gider (short derinleşir); eski sabit Buy, short pozisyonu
                // azaltıp ters pozisyon açıyordu.
                let add_side = if rt.stepper.position.direction == -1 { dca_math::Side::Sell } else { dca_math::Side::Buy };
                let req_o = dca_exchange::OrderRequest {
                    category: if rt.cfg.kind.is_futures() { "linear".to_string() } else { "spot".to_string() },
                    symbol: rt.cfg.symbol.clone(),
                    side: add_side,
                    is_market: true,
                    qty: qty.clone(),
                    price: None,
                    reduce_only: false,
                    order_link_id: order_id.clone(),
                    position_idx: if rt.cfg.kind.is_futures() && rt.pos_mode_hedge {
                        if rt.stepper.position.direction == 1 { 1 } else { 2 }
                    } else { 0 },
                };
                Some((req_o, order_id, qty, mark_v))
            } else {
                None
            };
            (rt.mode.clone(), rt.venue.clone(), mark, rt.stepper.instrument.step_size.clone(), plan)
        };
        if let Some((req_o, order_id, qty, mark_v)) = planned {
            // --- kilit yok; ağ serbest ---
            let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
            let creds = store::load_active_key(&app.db, &app.vault, &venue)
                .map_err(err422)?
                .ok_or_else(|| err422("live mode requires vault credentials"))?;
            adapter
                .place_order(&creds, &req_o)
                .await
                .map_err(|e| err422(format!("LIVE manual add order failed: {e}")))?;
            // --- uygulama: kısa kilit ---
            {
                let mut rts2 = app.runtimes.lock().await;
                if let Some(rt2) = rts2.get_mut(&id) {
                    rt2.live_open_orders.push(crate::state::LiveTrackedOrder {
                        id: order_id.clone(),
                        side: req_o.side,
                        kind: dca_kernel::OrderKind::Add,
                        price: mark_v.clone(),
                        qty: qty.clone(),
                        role: "MANUAL_ADD".into(),
                    });
                    let snap_v = rt2.snapshot_json();
                    app.ui_cache.lock().await.insert(id.clone(), snap_v);
                }
            }
            state::persist_status(&app, &id, &mode);
            state::persist_event(&app, &id, "INFO", "ACTION", "action=ADD_FUNDS (LIVE market, kilit-dışı ağ)").await;
            let _ = app.sse.send(SseEvent { event: "BOT_STATE".into(), data: json!({"botId": id, "action": "ADD_FUNDS"}).to_string() });
            return Ok(Json(json!({"ok": true})));
        }
        let _ = (mark, step_size); // PAPER: normal akışa düş
    }
    let _live_ownership_guard = if matches!(req.action.as_str(), "START" | "UNFREEZE") {
        let live_identity = {
            let rts = app.runtimes.lock().await;
            rts.get(&id).filter(|rt| rt.mode.eq_ignore_ascii_case("LIVE"))
                .map(|rt| (rt.venue.to_uppercase(), rt.cfg.symbol.clone()))
        };
        if let Some((venue, symbol)) = live_identity {
            Some(lock_live_ownership(&app, &id, &venue, &symbol).await.map_err(err422)?)
        } else {
            None
        }
    } else {
        None
    };
    let (_start_poll_guard, mut start_credentials) = if req.action == "START" {
        let live_venue = {
            let rts = app.runtimes.lock().await;
            rts.get(&id).filter(|rt| rt.mode.eq_ignore_ascii_case("LIVE")).map(|rt| rt.venue.to_uppercase())
        };
        if let Some(venue) = live_venue {
            let guard = acquire_bot_poll_guard(&app, &id).await;
            let credentials = store::load_active_key(&app.db, &app.vault, &venue)
                .map_err(err422)?
                .ok_or_else(|| err422("LIVE bot başlatılamadı: aktif kasa anahtarı bulunamadı"))?;
            (Some(guard), Some(credentials))
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };
    let mut rts = app.runtimes.lock().await;
    let rt = rts.get_mut(&id).ok_or_else(|| err422("bot not found"))?;
    match req.action.as_str() {
        "START" => {
            if rt.mode == "LIVE" {
                rt.credentials = Some(start_credentials.take().ok_or_else(|| err422("LIVE bot başlatılamadı: aktif kasa anahtarı bulunamadı"))?);
            }
            rt.status = "RUNNING".into();
        }
        "PAUSE" => {
            rt.status = "PAUSED".into();
        }
        // P2-2 Freeze: RUNNING → FROZEN (yeni emir yok, açıklar korunur).
        // PAUSED'tan dondurma yok — önce devam ettir (dürüst 422).
        "FREEZE" => {
            if rt.status != "RUNNING" {
                return Err(err422("freeze yalnız RUNNING botta — önce devam ettir"));
            }
            rt.status = "FROZEN".into();
        }
        "UNFREEZE" => {
            if rt.status != "FROZEN" {
                return Err(err422("unfreeze yalnız FROZEN botta"));
            }
            rt.status = "RUNNING".into();
        }
        "STOP" => {
            rt.status = "STOPPED".into();
        }
        "CLOSE_ALL" => {
            // Faz 20 (UIX-5): grid artık destekli — kapanış = mesh iptali
            // (halt_mesh) + açık pozisyonun TEK market kapanışı. Eski davranış
            // "close-all only on DCA engines" 422 döndürüyor, Studio'daki
            // "Tümünü Kapat" butonu grid botta anlamsızdı.
            let mut grid_close_acts: Vec<dca_strategy::Action> = Vec::new();
            let mut is_grid = false;
            match &mut rt.engine_kind {
                dca_strategy::EngineTag::Dca(e) => e.request_close_all(),
                dca_strategy::EngineTag::Hedge(h) => {
                    h.main.request_close_all();
                    if let Some(hg) = &mut h.hedge {
                        hg.request_close_all();
                    }
                }
                dca_strategy::EngineTag::Grid(g) => {
                    grid_close_acts = g.halt_mesh();
                    is_grid = true;
                }
            }
            if is_grid {
                rt.pending_engine_actions.extend(grid_close_acts);
                let qty = rt.stepper.position.qty.clone();
                if !qty.is_zero() {
                    rt.pending_engine_actions.push(dca_strategy::Action::PlaceMarket {
                        id: format!("{}-closeall-{}", rt.bot_id, crate::state::now_ms()),
                        side: if rt.stepper.position.direction == 1 { dca_math::Side::Sell } else { dca_math::Side::Buy },
                        qty,
                        role: "MANUAL",
                        quote_notional: None,
                    });
                }
                let msg = format!(
                    "grid CLOSE_ALL: {} mesh emri iptal edildi{} — eylemler bir sonraki barda dispatch edilir",
                    rt.pending_engine_actions.iter().filter(|a| matches!(a, dca_strategy::Action::CancelResting { .. })).count(),
                    if rt.stepper.position.qty.is_zero() { ", açık pozisyon yok" } else { " + market kapanış niyeti" }
                );
                crate::state::persist_event(app.as_ref(), &rt.bot_id, "INFO", "CLOSE_ALL", &msg).await;
            }
        }
        "ADD_FUNDS" => {
            let amount = req.amount.clone().ok_or_else(|| err422("amount required"))?;
            let quote = Fraction::from_decimal_str(&amount).map_err(|e| err422(e))?;
            // Faz 26 (KAPSAMLI_DENETİM P1): negatif/sıfır tutar reddi — eski
            // kod yalnız parse ediyordu; "-100" PAPER'da NAHİT DÜŞÜRÜYORDU
            // (tersine çevrilmiş para çekme), LIVE pre-flow'da qty_down
            // negatifte tanımsızdı. Fiziksel değişmez: yalnız pozitif ekleme.
            if !quote.is_positive() {
                return Err(err422("ADD_FUNDS amount must be > 0"));
            }
            let _mark = rt
                .stepper
                .last_close
                .as_deref()
                .and_then(|s| Fraction::from_any_str(s).ok())
                .ok_or_else(|| err422("no mark price yet"))?;
            match rt.mode.as_str() {
                "PAPER" => {
                    // BOT-05: PAPER'da ADD_FUNDS yalnız nakit ekler (serbest sermaye),
                    // pozisyonu büyültmez — kullanıcı kastı "sermaye ekleme"dir.
                    rt.stepper.cash = rt.stepper.cash.add(&quote).map_err(|e| err422(e.to_string()))?;
                }
                _ => {
                    // Faz 13A: LIVE ADD_FUNDS handler başındaki pre-flow'da
                    // kilit-dışı ağ ile işlendi — buraya düşmesi beklenmez.
                    return Err(err422("live mode requires vault credentials"));
                }
            }
        }

        "RESET_KERNEL" => {
            // P0-2: sticky kernel halt'ın AÇIK kurtarma ucu — GATE1 Halt
            // (gerçek veri bozulması) sonrası operatör buradan açar.
            // Faz 21 (N-8): HEDGE kernel'in halt'ı da açılır — eski kod yalnız
            // ana kernel'i açıyordu, hedge bacak kalıcı duruyor kalıyordu.
            // Faz 28-D (günlük denetim 2026-11-13): HALTED_RESTORE pozisyon
            // uyumsuzluğu varken yalnız halt açmak TEHLİKELİYDİ — kernel eski
            // qty ile TP/SL hesaplar, hata derinleşirdi (t9: 0.01104 ↔
            // 0.00542). PAPER'da motor mirror'ı fill defterinden türediğinden
            // GERÇEK kabul edilir: kernel pozisyonu (adet/yön/ortalama)
            // mirror'a hizalanır. LIVE'da hizalama YAPILMAZ — venue pozisyonu
            // ile mutabakat operatörde (davranış değişmez).
            let realigned = if rt.mode == "PAPER" {
                match &mut rt.engine_kind {
                    dca_strategy::EngineTag::Dca(e) => {
                        rt.stepper.position = e.mirror.clone();
                    }
                    dca_strategy::EngineTag::Hedge(h) => {
                        rt.stepper.position = h.main.mirror.clone();
                        if let Some(hs) = rt.hedge_stepper.as_mut() {
                            if let Some(leg) = &h.hedge {
                                hs.position = leg.mirror.clone();
                            }
                        }
                    }
                    dca_strategy::EngineTag::Grid(g) => {
                        rt.stepper.position = g.mirror.clone();
                    }
                }
                true
            } else {
                false
            };
            rt.stepper.halted = None;
            rt.stepper.prev_bar_ts = None;
            rt.stepper.bar_interval_ms = 0;
            if let Some(hs) = rt.hedge_stepper.as_mut() {
                if hs.halted.take().is_some() {
                    hs.prev_bar_ts = None;
                    hs.bar_interval_ms = 0;
                }
            }
            let scope = if realigned {
                "PAPER: pozisyonlar engine mirror'a hizalandı; "
            } else {
                "LIVE: hizalama yapılmadı — venue mutabakatı operatörde; "
            };
            state::persist_event(&app, &id, "WARN", "RESET_KERNEL", &format!("kernel halt temizlendi (hedge dâhil) — {scope}imleç sıfırlandı, bir sonraki kapalı bardan warmup ile devam")).await;
        }
        other => return Err(err422(format!("unknown action: {other}"))),
    }
    // Task 18 N-4: kritik status yazımı retry + stderr ile (sessiz yutma yok).
    state::persist_status(&app, &id, &rt.status);
    state::persist_event(&app, &id, "INFO", "ACTION", &format!("action={}", req.action)).await;
    {
        let snap_v = rt.snapshot_json();
        app.ui_cache.lock().await.insert(id.clone(), snap_v);
    }
    let _ = app.sse.send(SseEvent { event: "BOT_STATE".into(), data: json!({"botId": id, "action": req.action}).to_string() });
    Ok(Json(json!({"ok": true})))
}

async fn bot_fills(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>, axum::response::Response> {
    let conn = app.db.conn.lock().map_err(|_| err422("db kilitli — fills okunamadı"))?;
    let mut stmt = conn
        .prepare("SELECT fill_key, order_id, role, side, kind, price, qty, fee, fee_role, realism, ts FROM fills WHERE bot_id=?1 ORDER BY ts DESC LIMIT 200")
        .map_err(|e| err422(format!("fills sorgusu hazırlanamadı: {e}")))?;
    let rows: Vec<Value> = stmt
        .query_map([&id], |r| {
            Ok(json!({
                "fillKey": r.get::<_, String>(0)?,
                "orderId": r.get::<_, String>(1)?,
                "role": r.get::<_, String>(2)?,
                "side": r.get::<_, String>(3)?,
                "kind": r.get::<_, String>(4)?,
                "price": r.get::<_, String>(5)?,
                "qty": r.get::<_, String>(6)?,
                "fee": r.get::<_, String>(7)?,
                "feeRole": r.get::<_, String>(8)?,
                "fillModel": r.get::<_, String>(9)?,
                "ts": r.get::<_, i64>(10)?,
            }))
        })
        .map_err(|e| err422(format!("fills sorgusu koşamadı: {e}")))?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Json(json!({"fills": rows})))
}

async fn bot_events(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>, axum::response::Response> {
    let conn = app.db.conn.lock().map_err(|_| err422("db kilitli — events okunamadı"))?;
    let mut stmt = conn
        .prepare("SELECT level, gate, message, ts, trace_id FROM events WHERE bot_id=?1 ORDER BY id DESC LIMIT 200")
        .map_err(|e| err422(format!("events sorgusu hazırlanamadı: {e}")))?;
    let rows: Vec<Value> = stmt
        .query_map([&id], |r| {
            Ok(json!({
                "level": r.get::<_, String>(0)?,
                "gate": r.get::<_, String>(1)?,
                "message": r.get::<_, String>(2)?,
                "ts": r.get::<_, i64>(3)?,
                "traceId": r.get::<_, Option<String>>(4)?,
            }))
        })
        .map_err(|e| err422(format!("events sorgusu koşamadı: {e}")))?
        .filter_map(|x| x.ok())
        .collect();
    Ok(Json(json!({"events": rows})))
}

/// Task 19 — Pine kodu derleme/doğrulama: hata mesajı + kullanılan fonksiyonlar
/// + input adları + plot başlıkları döner (UI editör geri bildirimi).
#[derive(Deserialize)]
struct PineValidateReq {
    code: String,
}

async fn pine_validate(Json(req): Json<PineValidateReq>) -> Result<axum::response::Response, axum::response::Response> {
    match dca_pine::validate_source(&req.code) {
        Ok(prog) => Ok(axum::Json(json!({
            "ok": true,
            "decl": prog.decl.map(|d| json!({
                "kind": if d.kind == dca_pine::DeclKind::Strategy { "strategy" } else { "indicator" },
                "title": d.title,
                // Task 20 — pyramiding parametresi dürüst raporlanır (desteklenmez)
                "pyramiding": d.pyramiding,
            })),
            "inputs": prog.input_names,
            "functions": prog.functions_used.into_iter().collect::<Vec<_>>(),
            "plots": prog.plot_titles,
            // Task 20 — derleme-anı dürüst notlar (koşullu ta.* series-consistency,
            // pyramiding uyarısı) — sessiz davranış sapması kalmaz
            "notes": prog.notes,
        }))
        .into_response()),
        Err(e) => Err(err422(e)),
    }
}

/// Task 19 — Pine strateji backtest'i: kod, canlı borsa klines'ı üzerinde
/// koşturulur (BotConfig'in TP/SL'i bilinçli olarak kapalıdır — çıkışlar
/// yalnız pine stratejisinden gelir; dolum gerçekçiliği motorun kendisindedir).
#[derive(Deserialize)]
struct PineBacktestReq {
    code: String,
    #[serde(default)]
    inputs: std::collections::BTreeMap<String, String>,
    symbol: String,
    #[serde(default = "default_interval")]
    interval: String,
    #[serde(default = "default_pine_bars")]
    bars: usize,
    #[serde(default = "default_start_cash")]
    start_cash: String,
    /// DCA_LONG (varsayılan) veya DCA_SHORT; grid'te bootstrap kapısı olarak
    /// çalışır. Diğer kind'lar 422 ile reddedilir.
    #[serde(default)]
    kind: Option<String>,
    /// Task 20 — opsiyonel bot TP/SL (pine çıkışına EK). Verilmezse çıkışlar
    /// yalnız pine'dan gelir (Task 19 sözleşmesi korunur). Verilirse motorun
    /// kendi TP/SL'i de bar-bazlı devreye girer (hangisi önce vurursa).
    #[serde(default)]
    target_profit_pct: Option<String>,
    #[serde(default)]
    stop_loss_pct: Option<String>,
}

fn default_pine_bars() -> usize {
    400
}

/// Backtest varsayılan bar penceresi (720). Pine tarafı (`default_pine_bars` =
/// 400) bundan ayrıdır — karıştırılmamalı: Pine varsayılanı değişmedi.
fn default_bt_bars() -> usize {
    720
}

async fn pine_backtest(
    State(app): State<Arc<AppState>>,
    Json(req): Json<PineBacktestReq>,
) -> Result<axum::response::Response, axum::response::Response> {
    if req.bars == 0 {
        return Err(err422("bars en az 1 olmalı"));
    }
    let prog = dca_pine::validate_source(&req.code).map_err(err422)?;
    let kind = match req.kind.as_deref() {
        None | Some("DCA_LONG") => StrategyKind::DcaLong,
        Some("DCA_SHORT") => StrategyKind::DcaShort,
        Some(other) => return Err(err422(format!("pine backtest kind '{other}' desteklenmiyor (DCA_LONG / DCA_SHORT)"))),
    };
    let category = if kind.is_futures() { "linear" } else { "spot" };
    let iv = engine::bybit_interval(&req.interval).map_err(err422)?;
    let kl = app
        .bybit
        .klines(category, &req.symbol, &iv, req.bars.min(1000))
        .await
        .map_err(err422)?;
    if kl.len() < 5 {
        return Err(err422("pine backtest: yeterli bar yok"));
    }
    let bars: Vec<dca_kernel::Bar> = kl
        .iter()
        .map(|k| dca_kernel::Bar { t: k.t, o: k.o.clone(), h: k.h.clone(), l: k.l.clone(), c: k.c.clone(), v: k.v.clone() })
        .collect();
    // Pine tamamen çıkışı yönetir: bot TP'si devre dışı (çok yüksek), SL yok.
    // Task 20: istekte targetProfitPct/stopLossPct verilirse motor TP/SL'i de
    // bar-bazlı devreye girer (pine çıkışına ek — hangisi önce vurursa).
    let mut cfg = BotConfig::default();
    cfg.kind = kind;
    cfg.symbol = req.symbol.clone();
    cfg.pine_script = Some(req.code.clone());
    cfg.pine_inputs = req.inputs.clone();
    let mut bot_stops_applied = false;
    match &req.target_profit_pct {
        Some(tp) => {
            validate_cfg_positive("targetProfitPct", tp, false).map_err(err422)?;
            cfg.target_profit_pct = tp.clone();
            bot_stops_applied = true;
        }
        None => cfg.target_profit_pct = "1000000".into(), // pine çıkışı haricinde TP kapalı
    }
    if let Some(sl) = &req.stop_loss_pct {
        validate_cfg_positive("stopLossPct", sl, false).map_err(err422)?;
        cfg.stop_loss_pct = Some(sl.clone());
        bot_stops_applied = true;
    } else {
        cfg.stop_loss_pct = None;
    }
    cfg.base_order_size = "100".into();
    cfg.safety_order_size = "0".into();
    let inst = {
        let insts = app.bybit.instruments(category, Some(&req.symbol)).await.unwrap_or_default();
        insts.into_iter().next().unwrap_or_else(|| {
            let mut i = Instrument::default();
            i.symbol = req.symbol.clone();
            i
        })
    };
    let start = Fraction::from_decimal_str(&req.start_cash).map_err(err422)?;
    let session = dca_math::sessions::SessionType::Crypto24_7;
    let result = tokio::task::spawn_blocking(move || run_backtest(&bars, &inst, &cfg, start, session, &[]))
        .await
        .map_err(|e| err422(e.to_string()))?
        .map_err(err422)?;
    let entries = result
        .fills
        .iter()
        .filter(|f| f.role == "BASE")
        .count();
    let fill_marks = fill_marks_json(&result.fills);
    let step = (result.equity_curve.len() / 300).max(1);
    let curve: Vec<Value> = result
        .equity_curve
        .into_iter()
        .step_by(step)
        .map(|p| json!({"ts": p.ts, "equity": p.equity}))
        .collect();
    Ok(axum::Json(json!({
        "ok": true,
        "decl": prog.decl.as_ref().map(|d| json!({
            "kind": if d.kind == dca_pine::DeclKind::Strategy { "strategy" } else { "indicator" },
            "title": d.title,
            "pyramiding": d.pyramiding,
        })),
        "inputs": prog.input_names,
        "metrics": result.metrics,
        "equityCurve": curve,
        "startEquity": result.start_equity,
        "endEquity": result.end_equity,
        "entries": entries,
        "fills": result.fills.len(),
        "pineWarnings": result.pine_warnings,
        // Task 20 — dürüst notlar + plot serileri + TP/SL durumu
        "pineNotes": result.pine_notes,
        "pinePlots": result.pine_plots,
        // Task 22 — boş sonuçların makine-okunur nedenleri (motor sayaçları).
        "noDealReasons": result.no_deal_reasons,
        "botStopsApplied": bot_stops_applied,
        // Task 21 — plotların ana grafiğe çizilebilmesi için: backtest'in
        // kullandığı TAM bar penceresi (plot bar dizini = bar pencere dizini)
        // + giriş/çıkış işaretçileri.
        "bars": kl,
        "fillMarks": fill_marks,
    }))
    .into_response())
}

// ---------------- Pine Strategy Library (Task 21) ----------------
// Kaydet/yükle/sil: dca_pine::validate_source kapısından geçmeyen kod
// kaydedilemez — kütüphane yalnız DERLENEN stratejileri barındırır
// (sessiz bozuk kayıt yok). Vault CRUD üçlüsünün birebir deseni.

#[derive(Deserialize)]
struct SavePineScriptReq {
    name: String,
    code: String,
    #[serde(default)]
    inputs: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    notes: Option<String>,
}

#[derive(Deserialize)]
struct PatchPineScriptReq {
    name: Option<String>,
    code: Option<String>,
    inputs: Option<std::collections::BTreeMap<String, String>>,
    notes: Option<String>,
}

async fn save_pine_script(
    State(app): State<Arc<AppState>>,
    Json(req): Json<SavePineScriptReq>,
) -> Result<axum::response::Response, axum::response::Response> {
    let name = req.name.trim();
    if name.is_empty() || name.len() > 80 {
        return Err(err422("Kitaplık adı 1–80 karakter arasında olmalı"));
    }
    let prog = dca_pine::validate_source(&req.code).map_err(err422)?;
    let id = format!("ps-{}", Uuid::new_v4().simple());
    let now = state::now_ms();
    {
        let conn = app.db.conn.lock().map_err(|e| err422(e.to_string()))?;
        conn.execute(
            "INSERT INTO pine_scripts(id, name, code, inputs, notes, created_at, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7)",
            rusqlite::params![
                id,
                name,
                req.code,
                serde_json::to_string(&req.inputs).map_err(|e| err422(e.to_string()))?,
                req.notes,
                now,
                now
            ],
        )
        .map_err(|e| err422(e.to_string()))?;
    }
    Ok(axum::Json(json!({
        "ok": true,
        "id": id,
        "name": name,
        "decl": prog.decl.as_ref().map(|d| json!({
            "kind": if d.kind == dca_pine::DeclKind::Strategy { "strategy" } else { "indicator" },
            "title": d.title,
        })),
        "inputs": prog.input_names,
        "functions": prog.functions_used.into_iter().collect::<Vec<_>>(),
        "notes": prog.notes,
    }))
    .into_response())
}

async fn list_pine_scripts(State(app): State<Arc<AppState>>) -> Json<Value> {
    let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let mut stmt = match conn
        .prepare("SELECT id, name, code, inputs, notes, created_at, updated_at FROM pine_scripts ORDER BY updated_at DESC")
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("pine_scripts list failed: {e}");
            return Json(json!({"scripts": []}));
        }
    };
    let iso = |ms: i64| {
        chrono::DateTime::from_timestamp_millis(ms)
            .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
            .unwrap_or_default()
    };
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            let inputs_raw: String = r.get(3)?;
            let inputs: Value = serde_json::from_str(&inputs_raw).unwrap_or_else(|_| json!({}));
            Ok(json!({
                "id": r.get::<_, String>(0)?,
                "name": r.get::<_, String>(1)?,
                "code": r.get::<_, String>(2)?,
                "inputs": inputs,
                "notes": r.get::<_, Option<String>>(4)?,
                "createdAt": iso(r.get::<_, i64>(5)?),
                "updatedAt": iso(r.get::<_, i64>(6)?),
            }))
        })
        .map(|rows| rows.filter_map(|x| x.ok()).collect())
        .unwrap_or_else(|e| {
            tracing::warn!("pine_scripts query failed: {e}");
            Vec::new()
        });
    Json(json!({"scripts": rows}))
}

async fn patch_pine_script(
    State(app): State<Arc<AppState>>,
    Path(id): Path<String>,
    Json(req): Json<PatchPineScriptReq>,
) -> Result<Json<Value>, axum::response::Response> {
    // Güncellenen kod da derlenmek zorunda — kütüphanede bozuk strateji birikmez.
    if let Some(code) = &req.code {
        dca_pine::validate_source(code).map_err(err422)?;
    }
    let conn = app.db.conn.lock().map_err(|e| err422(e.to_string()))?;
    let n = conn
        .execute(
            "UPDATE pine_scripts SET name = COALESCE(?2, name), code = COALESCE(?3, code), inputs = COALESCE(?4, inputs), notes = COALESCE(?5, notes), updated_at = ?6 WHERE id = ?1",
            rusqlite::params![
                id,
                req.name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
                req.code,
                req.inputs
                    .as_ref()
                    .map(|m| serde_json::to_string(m).unwrap_or_else(|_| "{}".into())),
                req.notes,
                state::now_ms()
            ],
        )
        .map_err(|e| err422(e.to_string()))?;
    if n == 0 {
        return Err(err422("kütüphane kaydı bulunamadı"));
    }
    Ok(Json(json!({"ok": true})))
}

async fn delete_pine_script(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Json<Value> {
    let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
    if let Err(e) = conn.execute("DELETE FROM pine_scripts WHERE id=?1", [&id]) {
        tracing::warn!("pine_script delete failed: {e}");
    }
    Json(json!({"ok": true}))
}

#[derive(Deserialize)]
struct BacktestCertReq {
    kind: String,
    symbol: String,
    #[serde(default = "default_interval")]
    interval: String,
    #[serde(default = "default_bt_bars")]
    bars: usize,
    config: Option<Value>,
    #[serde(default = "default_start_cash")]
    start_cash: String,
    #[serde(default = "default_mc_runs")]
    mc_runs: usize,
    /// P4-3d: sertifikasyon market verisi bu venue'den çekilir (BYBIT varsayılan).
    #[serde(default)]
    venue: Option<String>,
    /// Faz 22 (P0-10 kalanı): kullanıcı gerçek deneme sayısı (DSR söndürmesi).
    #[serde(default)]
    attempts: usize,
}

fn default_mc_runs() -> usize {
    200
}

async fn backtest_cert(
    State(app): State<Arc<AppState>>,
    Json(req): Json<BacktestCertReq>,
) -> Result<axum::response::Response, axum::response::Response> {
    let kind = match parse_kind(&req.kind) {
        Ok(k) => k,
        Err(e) => return Err(err422(e)),
    };
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

    let category = if kind.is_futures() { "linear" } else { "spot" };
    // P4-3d: sertifikasyon market verisi seçili venue'den.
    let venue = req.venue.unwrap_or_else(|| "BYBIT".into()).to_uppercase();
    let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
    let iv = engine::bybit_interval(&req.interval).map_err(|e| err422(e))?;
    let bars = routes_backtest::fetch_backtest_klines(adapter, &venue, category, &req.symbol, &iv, req.bars.min(1000))
        .await
        .map_err(|e| err422(e))?;
    if bars.is_empty() {
        return Err(err422("no market data"));
    }
    let inst = {
        let insts = adapter
            .instruments(category, Some(&req.symbol))
            .await
            .unwrap_or_default();
        insts.into_iter().next().unwrap_or_else(|| {
            let mut i = Instrument::default();
            i.symbol = req.symbol.clone();
            i
        })
    };
    let start = Fraction::from_decimal_str(&req.start_cash).map_err(|e| err422(e))?;
    let session = dca_math::sessions::SessionType::Crypto24_7;
    // Funding history for futures certification base runs (same window as backtest).
    // Task 18 N-5: sessiz "sıfır funding" yerine WARN event + cert yanıtında flag.
    let (funding, funding_warning): (Vec<(i64, String)>, Option<String>) = if kind.is_futures() {
        match adapter.funding_history(&req.symbol, 200).await {
            Ok(v) => (v, None),
            Err(e) => (
                Vec::new(),
                Some(format!("Funding geçmişi alınamadı ({e}) — sertifikasyon base run funding=0 varsaydı")),
            ),
        }
    } else {
        (Vec::new(), None)
    };
    if let Some(w) = &funding_warning {
        state::persist_event(&app, &req.symbol, "WARN", "CERTIFY", w).await;
    }
    let bars_n: Vec<dca_kernel::Bar> = bars
        .into_iter()
        .map(|k| dca_kernel::Bar { t: k.t, o: k.o, h: k.h, l: k.l, c: k.c, v: k.v })
        .collect();
    let (cert, base_metrics) = tokio::task::spawn_blocking(move || {
        let base = run_backtest(&bars_n, &inst, &cfg, start.clone(), session, &funding).ok();
        // Faz 18 (P0-10): denemeler de base ile aynı funding takviminde.
        let cert = certify(&bars_n, &inst, &cfg, start, session, req.mc_runs, &funding, req.attempts);
        (cert, base)
    })
    .await
    .map_err(|e| err422(e.to_string()))?;
    let cert = cert.map_err(|e| err422(e))?;
    // Faz 0.1 — sertifika hükmü (Lab rozet eşikleriyle aynı kapılar; UI + LIVE
    // kapısı tek kaynaktan beslenir). funding kapısı yalnız vadeli türde uygulanır.
    let gate = certify_verdict(&cert, base_metrics.as_ref().map(|b| b.metrics.deals).unwrap_or(0), funding_warning.is_none(), kind.is_futures());
    Ok(axum::Json(json!({
        "cert": cert,
        "verdict": gate.verdict,
        "gates": gate.gates,
        // Task 18 N-5: sertifikasyonda da dürüst funding uyarısı.
        "fundingWarning": funding_warning,
        // P4-3d: koşunun market verisi bu venue'den geldi.
        "venue": venue,
        "base": base_metrics.map(|b| json!({
            "totalReturnPct": b.metrics.total_return_pct,
            "maxDrawdownPct": b.metrics.max_drawdown_pct,
            "sharpeDisplay": b.metrics.sharpe_display,
            "deals": b.metrics.deals,
        })),
    }))
    .into_response())
}

#[derive(Deserialize)]
struct MarketsQuery {
    venue: Option<String>,
    category: Option<String>,
    #[serde(default)]
    symbol: Option<String>,
}

async fn markets(State(app): State<Arc<AppState>>, Query(q): Query<MarketsQuery>) -> Json<Value> {
    let category = q.category.unwrap_or_else(|| "spot".into());
    let venue = q.venue.unwrap_or_else(|| "BYBIT".into());
    let key = format!("{venue}:{category}:{}", q.symbol.clone().unwrap_or_default());
    {
        let cache = app.instruments_cache.lock().await;
        if let Some((at, list)) = cache.get(&key) {
            // M-3: TTL'li cache — bayat/hatalı liste restart'a kadar servis edilmez.
            if state::now_ms() - *at < state::INSTRUMENTS_CACHE_TTL_MS {
                return Json(json!({"instruments": list, "cached": true}));
            }
        }
    }
    let list = match venue.as_str() {
        "BINANCE" => app.binance.instruments(&category, q.symbol.as_deref()).await.unwrap_or_default(),
        "OKX" => app.okx.instruments(&category, q.symbol.as_deref()).await.unwrap_or_default(),
        _ => app.bybit.instruments(&category, q.symbol.as_deref()).await.unwrap_or_default(),
    };
    // M-3: yalnız dolu yanıtlar cache'lenir — hatadan gelen boş liste TTL
    // boyunca "gerçek boş piyasa" gibi servis edilmez.
    if !list.is_empty() {
        app.instruments_cache.lock().await.insert(key, (state::now_ms(), list.clone()));
    }
    Json(json!({"instruments": list, "cached": false}))
}

#[derive(Deserialize)]
struct KlinesQuery {
    venue: Option<String>,
    category: Option<String>,
    symbol: String,
    interval: Option<String>,
    limit: Option<usize>,
}

/// P2-1 — 24h istatistik listesi (sembol tarayıcı beslemesi). 30 sn TTL cache;
/// PAPER boş liste döner (yerel venue'de 24s akışı yok — dürüst boş, hata değil).
async fn tickers(State(app): State<Arc<AppState>>, Query(q): Query<MarketsQuery>) -> Json<Value> {
    let category = q.category.unwrap_or_else(|| "spot".into());
    let venue = q.venue.unwrap_or_else(|| "BYBIT".into());
    let key = format!("{venue}:{category}");
    {
        let cache = app.tickers_cache.lock().await;
        if let Some((at, rows)) = cache.get(&key) {
            if state::now_ms() - *at < 30_000 {
                return Json(json!({"tickers": rows, "cached": true}));
            }
        }
    }
    let rows = match venue.as_str() {
        "BINANCE" => app.binance.tickers_24h(&category).await.unwrap_or_default(),
        "OKX" => app.okx.tickers_24h(&category).await.unwrap_or_default(),
        "PAPER" => Vec::new(),
        _ => app.bybit.tickers_24h(&category).await.unwrap_or_default(),
    };
    app.tickers_cache.lock().await.insert(key, (state::now_ms(), rows.clone()));
    Json(json!({"tickers": rows, "cached": false}))
}

async fn klines(State(app): State<Arc<AppState>>, Query(q): Query<KlinesQuery>) -> Json<Value> {
    let category = q.category.unwrap_or_else(|| "spot".into());
    let iv = match engine::bybit_interval(q.interval.as_deref().unwrap_or("1h")) {
        Ok(iv) => iv,
        Err(e) => return Json(json!({"error": e, "bars": []})),
    };
    let limit = q.limit.unwrap_or(200).min(1000);
    let venue = q.venue.as_deref().unwrap_or("BYBIT");
    let cache_key = format!("{venue}:{category}:{}:{iv}:{limit}", q.symbol);

    // Faz 1.6 (B-8): 10 saniyelik paylaşımlı kline önbelleği — mükerrer istekleri ve 429 ban riskini önler
    if let Some(cached) = app.get_cached_klines(&cache_key, 10_000) {
        return Json(json!({"bars": cached, "cached": true}));
    }

    let kl = match venue {
        "BINANCE" => app.binance.klines(&category, &q.symbol, &iv, limit).await,
        "OKX" => match dca_exchange::okx::okx_bar(&iv) {
            Ok(oiv) => app.okx.klines(&category, &q.symbol, oiv, limit).await,
            Err(e) => Err(e),
        },
        _ => app.bybit.klines(&category, &q.symbol, &iv, limit).await,
    };
    match kl {
        Ok(bars) => {
            app.set_cached_klines(cache_key, bars.clone());
            Json(json!({"bars": bars, "cached": false}))
        }
        Err(e) => Json(json!({"error": e, "bars": []})),
    }
}

// ---------------- Parity: backtest ↔ paper fill ekonomisi ----------------
// v3 /api/parity karşılığı: aynı trailing pencerede backtest fill'leri ile
// paper fill'leri karşılaştırılır (adet + fill-başı ücret). Deal-eşleştirme
// (win-rate pairing) v2 işidir — bu v1 davranışsal paritedir, rozet dürüstçe
// bunu söyler. Eşik: iki metrikte de ≥%90 eşleşme → GECTI; paper'da <10 fill
// varsa YETERSIZ_VERI (fail-closed: veri yokken güven rozeti verilmez).
#[derive(Deserialize)]
struct ParityReq {
    bot_id: String,
    #[serde(default = "default_bt_bars")]
    bars: usize,
    interval: Option<String>,
}

fn frac_match_pct(a: &Fraction, b: &Fraction) -> f64 {
    let denom = a.to_f64_display().abs().max(b.to_f64_display().abs()).max(1e-12);
    (1.0 - (a.to_f64_display() - b.to_f64_display()).abs() / denom).max(0.0)
}

async fn parity(State(app): State<Arc<AppState>>, Json(req): Json<ParityReq>) -> Result<Json<Value>, axum::response::Response> {
    let bars_req = req.bars.min(1000).max(1);
    let (kind_s, symbol, start_cash, cfg_json, interval_row): (String, String, String, String, Option<String>) = {
        let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.query_row(
            "SELECT kind, symbol, start_cash, config, interval FROM bots WHERE id=?1",
            [&req.bot_id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .map_err(|_| err422("bot not found"))?
    };
    let kind = parse_kind(&kind_s).map_err(err422)?;
    let mut cfg: BotConfig = serde_json::from_str(&cfg_json).map_err(|e| err422(format!("config: {e}")))?;
    cfg.kind = kind;
    cfg.symbol = symbol.clone();
    let iv = req.interval.clone().or(interval_row).unwrap_or_else(|| "1h".into());
    let category = if kind.is_futures() { "linear" } else { "spot" };
    let kl = app
        .bybit
        .klines(category, &symbol, &engine::bybit_interval(&iv).map_err(err422)?, bars_req)
        .await
        .map_err(err422)?;
    if kl.len() < 5 {
        return Err(err422("parity: yeterli bar yok"));
    }
    let window_start = kl.first().map(|k| k.t).unwrap_or(0);
    // Paper tarafı: penceredeki fill'ler (kalıcı fills tablosu).
    let (n_paper, fee_paper) = {
        let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
        let mut stmt = conn
            .prepare("SELECT fee FROM fills WHERE bot_id=?1 AND ts>=?2")
            .map_err(|e| err422(e.to_string()))?;
        let fees: Vec<String> = stmt
            .query_map(rusqlite::params![&req.bot_id, window_start], |r| r.get(0))
            .map_err(|e| err422(e.to_string()))?
            .filter_map(|x| x.ok())
            .collect();
        let mut sum = Fraction::zero();
        for f in &fees {
            if let Ok(v) = Fraction::from_any_str(f) {
                sum = sum.add(&v).unwrap_or_else(|_| sum.clone());
            }
        }
        (fees.len() as u64, sum)
    };
    // Backtest tarafı: aynı config, aynı barlar, tek gerçekçi geçiş.
    let inst = {
        let insts = app.bybit.instruments(category, Some(&symbol)).await.unwrap_or_default();
        insts.into_iter().next().unwrap_or_else(|| {
            let mut i = Instrument::default();
            i.symbol = symbol.clone();
            i
        })
    };
    let start = Fraction::from_decimal_str(&start_cash).unwrap_or_else(|_| Fraction::from_u64(100_000));
    let session = dca_math::sessions::SessionType::Crypto24_7;
    let bars_n: Vec<dca_kernel::Bar> = kl
        .iter()
        .map(|k| dca_kernel::Bar { t: k.t, o: k.o.clone(), h: k.h.clone(), l: k.l.clone(), c: k.c.clone(), v: k.v.clone() })
        .collect();
    let result = run_backtest(&bars_n, &inst, &cfg, start, session, &[]).map_err(err422)?;
    let n_bt = result.fills.len() as u64;
    let fee_bt = Fraction::from_any_str(&result.metrics.total_fees).unwrap_or_else(|_| Fraction::zero());
    let avg_paper = if n_paper > 0 { fee_paper.to_f64_display() / n_paper as f64 } else { 0.0 };
    let avg_bt = if n_bt > 0 { fee_bt.to_f64_display() / n_bt as f64 } else { 0.0 };
    let count_match = 1.0 - (n_bt as f64 - n_paper as f64).abs() / (n_bt.max(n_paper).max(1) as f64);
    let denom = avg_paper.abs().max(avg_bt.abs()).max(1e-12);
    let fee_match = (1.0 - (avg_bt - avg_paper).abs() / denom).max(0.0);
    let verdict = if n_paper < 10 {
        "YETERSIZ_VERI"
    } else if count_match >= 0.90 && fee_match >= 0.90 {
        "GECTI"
    } else {
        "KALDI"
    };
    {
        let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
        let _ = conn.execute(
            "INSERT INTO parity_checks(bot_id, verdict, count_match, fee_match, bt_fills, paper_fills, win_rate_bt, bars, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",
            rusqlite::params![
                &req.bot_id,
                verdict,
                format!("{:.4}", count_match),
                format!("{:.4}", fee_match),
                n_bt.to_string(),
                n_paper.to_string(),
                result.metrics.win_rate_pct,
                kl.len().to_string(),
                state::now_ms(),
            ],
        );
    }
    Ok(Json(json!({
        "verdict": verdict,
        "countMatchPct": format!("{:.2}", count_match * 100.0),
        "feeMatchPct": format!("{:.2}", fee_match * 100.0),
        "btFills": n_bt,
        "paperFills": n_paper,
        "winRateBt": result.metrics.win_rate_pct,
        "bars": kl.len(),
        "note": "v1 davranışsal parite: fill adedi + fill-başı ücret. Deal-eşleşmeli win-rate pairing v2 işidir.",
    })))
}

// ---------------- Reconcile: borsa ↔ yerel mutabakat (raporlama) ---------
// v3 /api/live/reconcile karşılığı — farkla: otomatik durdurma YOK (bilinçli).
// Tutarsızlık raporlanır; durdurma tek tıkla kullanıcıda (bot_actions PAUSE).
#[derive(Deserialize)]
struct ReconcileQuery {
    bot_id: String,
}

async fn reconcile(State(app): State<Arc<AppState>>, Query(q): Query<ReconcileQuery>) -> Result<Json<Value>, axum::response::Response> {
    let (mode, venue, symbol, local_side, local_qty) = {
        let rts = app.runtimes.lock().await;
        let rt = rts.get(&q.bot_id).ok_or_else(|| err422("bot not found"))?;
        let (local_side, local_qty) = match &rt.engine_kind {
            dca_strategy::EngineTag::Dca(e) => {
                let snap = e.snapshot();
                let side = match e.cfg.kind {
                    StrategyKind::DcaShort => "Short",
                    _ => "Long",
                };
                (side.to_string(), snap.position_qty)
            }
            _ => {
                return Ok(Json(json!({
                    "supported": false,
                    "reason": "v1 yalnız DCA motorlarında — grid/hedge mutabakatı v2 işidir.",
                })));
            }
        };
        (rt.mode.clone(), rt.venue.clone(), rt.cfg.symbol.clone(), local_side, local_qty)
    };
    if mode == "PAPER" {
        return Ok(Json(json!({
            "supported": false,
            "reason": "PAPER modda borsa pozisyonu yok — mutabakat yalnız LIVE botlarda anlamlıdır.",
        })));
    }
    let local_zero = local_qty == "0" || local_qty == "0.00000000";
    let creds = store::load_active_key(&app.db, &app.vault, &venue)
        .map_err(err422)?
        .ok_or_else(|| err422("bu venue için aktif vault anahtarı yok"))?;
    let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
    let positions = adapter.positions(&creds, &symbol).await.map_err(err422)?;
    let open: Vec<Value> = positions
        .iter()
        .filter(|p| !p.size.is_zero())
        .map(|p| json!({"side": p.side, "size": p.size.to_half_even_display(8), "avgPrice": p.avg_price.to_half_even_display(8)}))
        .collect();
    // Eşleşme: yerel sıfırsa borsada da sıfır olmalı; yerel açıksa borsada aynı
    // yönde tek pozisyon ve miktar %0.1 toleransta denk olmalı.
    let matched = if local_zero {
        open.is_empty()
    } else {
        open.len() == 1
            && open[0]["side"].as_str().map(|s| s.eq_ignore_ascii_case(&local_side)).unwrap_or(false)
            && open[0]["size"]
                .as_str()
                .and_then(|s| Fraction::from_any_str(s).ok())
                .and_then(|ex| Fraction::from_any_str(&local_qty).ok().map(|lq| (ex, lq)))
                .map(|(ex, lq)| frac_match_pct(&ex, &lq) >= 0.999)
                .unwrap_or(false)
    };
    Ok(Json(json!({
        "supported": true,
        "match": matched,
        "local": {"side": local_side, "qty": local_qty},
        "exchange": open,
        "actionRequired": !matched,
        "note": if matched {
            "Yerel ayna ile borsa pozisyonu denk."
        } else {
            "TUTARSIZLIK — otomatik durdurma yok; Bot Filosu'ndan duraklatın ve fill'leri inceleyin."
        },
    })))
}

// ---------------- Self-check: 23 kontrol (v3'teki 20'nin Rust karşılığı) --
// v3 POST /api/live/selfcheck karşılığı — farklar (bilinçli): test-order
// bacağı YOK (ağa emir gönderilmez; dry-run ayrı P3 işidir); P4-3'te OKX
// sondaları eklendi (20-23). Erişilemeyen halka-açık venue FAIL değil
// ABSTAIN sayılır (dürüst atlama).
// Verdict: KALDI (hard-fail ya da herhangi bir FAIL) / GECTI (anahtar var,
// koşanların hepsi PASS) / YETERSIZ_ORTAM (anahtar yok, imzalılar abstain).
#[derive(Deserialize, Default)]
struct SelfCheckReq {
    symbol: Option<String>,
}

fn active_keypair(app: &Arc<AppState>, venue: &str) -> Option<VenueCreds> {
    store::load_active_key(&app.db, &app.vault, venue).ok().flatten()
}

async fn selfcheck(State(app): State<Arc<AppState>>, body: Option<Json<SelfCheckReq>>) -> Json<Value> {
    let symbol = body
        .and_then(|b| b.symbol.clone())
        .map(|s| s.trim().to_uppercase().chars().filter(|c| c.is_ascii_alphanumeric()).take(12).collect::<String>())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "BTCUSDT".into());
    let t0 = std::time::Instant::now();
    let mut items: Vec<Value> = Vec::new();
    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut abstained = 0u32;
    let mut hard_fail = false;
    macro_rules! push {
        ($id:expr, $group:expr, $name:expr, $status:expr, $detail:expr, $ms:expr) => {{
            let s: &str = $status;
            match s {
                "PASS" => passed += 1,
                "FAIL" => failed += 1,
                _ => abstained += 1,
            }
            items.push(json!({"id": $id, "group": $group, "name": $name, "status": s, "detail": $detail, "ms": $ms}));
        }};
    }
    // ---- Yerel (ağ yok) ----
    {
        let t = std::time::Instant::now();
        let ok = Fraction::from_decimal_str("0.1")
            .and_then(|a| Fraction::from_decimal_str("0.2").and_then(|b| a.add(&b)))
            .map(|s| s == Fraction::from_decimal_str("0.3").unwrap())
            .unwrap_or(false);
        push!(1, "Yerel", "Matematik öz-test (0.1+0.2=0.3 exact)", if ok { "PASS" } else { "FAIL" }, if ok { "rasyonel çekirdek sağlam" } else { "KESİR HATASI" }, t.elapsed().as_millis() as u64);
    }
    {
        push!(2, "Yerel", "8 fail-closed gate aktif", "PASS", "G1..G8 (veri/fee/slippage/lookahead/realism/sermaye/seans/salvo-gap)", 0);
    }
    {
        let t = std::time::Instant::now();
        let ok = app.db.conn.lock().map(|c| c.query_row::<i64, _, _>("SELECT 1", [], |r| r.get(0))).unwrap_or(Ok(0)).unwrap_or(0) == 1;
        push!(3, "Yerel", "SQLite WAL okunabilir", if ok { "PASS" } else { "FAIL" }, if ok { "SELECT 1" } else { "DB kilitli/bozuk" }, t.elapsed().as_millis() as u64);
    }
    {
        let (n, ok) = match app.db.conn.lock() {
            Ok(c) => (c.query_row::<i64, _, _>("SELECT COUNT(*) FROM vault_keys WHERE active=1", [], |r| r.get(0)).unwrap_or(-1), true),
            Err(_) => (-1, false),
        };
        push!(4, "Yerel", "Kasa (vault) açılıyor", if ok { "PASS" } else { "FAIL" }, if n >= 0 { format!("{n} aktif anahtar kaydı") } else { "kasa okunamadı".into() }, 0);
    }
    {
        let n = app.db.conn.lock().map(|c| c.query_row::<i64, _, _>("SELECT COUNT(*) FROM bots", [], |r| r.get(0)).unwrap_or(-1)).unwrap_or(-1);
        push!(5, "Yerel", "Bot kayıtları okunabilir", if n >= 0 { "PASS" } else { "FAIL" }, if n >= 0 { format!("{n} bot kaydı") } else { "bots okunamadı".into() }, 0);
    }
    {
        let n = app.db.conn.lock().map(|c| c.query_row::<i64, _, _>("SELECT COUNT(*) FROM parity_checks", [], |r| r.get(0)).unwrap_or(-1)).unwrap_or(-1);
        push!(6, "Yerel", "Parite arşivi okunabilir", if n >= 0 { "PASS" } else { "FAIL" }, if n >= 0 { format!("{n} parite kaydı") } else { "parity okunamadı".into() }, 0);
    }
    // ---- Ortam (halka açık ağ) ----
    {
        let t = std::time::Instant::now();
        match app.bybit.server_time_ms().await {
            Ok(ts) if ts > 0 => push!(7, "Ortam", "Bybit saat", "PASS", format!("server_time_ms={ts}"), t.elapsed().as_millis() as u64),
            _ => push!(7, "Ortam", "Bybit saat", "FAIL", "Bybit'e erişilemiyor", t.elapsed().as_millis() as u64),
        }
    }
    {
        let t = std::time::Instant::now();
        match app.bybit.klines("linear", &symbol, &engine::bybit_interval("1h").unwrap_or("60".into()), 5).await {
            Ok(b) if !b.is_empty() => push!(8, "Ortam", "Bybit klines (linear)", "PASS", format!("{}: {} bar", symbol, b.len()), t.elapsed().as_millis() as u64),
            _ => push!(8, "Ortam", "Bybit klines (linear)", "FAIL", format!("{symbol} bar alınamadı"), t.elapsed().as_millis() as u64),
        }
    }
    {
        let t = std::time::Instant::now();
        match app.bybit.instruments("linear", None).await {
            Ok(list) if !list.is_empty() => push!(9, "Ortam", "Bybit enstrüman listesi", "PASS", format!("{} linear çift", list.len()), t.elapsed().as_millis() as u64),
            _ => push!(9, "Ortam", "Bybit enstrüman listesi", "FAIL", "liste boş", t.elapsed().as_millis() as u64),
        }
    }
    {
        let t = std::time::Instant::now();
        match app.bybit.tickers_24h("linear").await {
            Ok(list) if !list.is_empty() => push!(10, "Ortam", "Bybit 24s ticker akışı", "PASS", format!("{} satır", list.len()), t.elapsed().as_millis() as u64),
            _ => push!(10, "Ortam", "Bybit 24s ticker akışı", "FAIL", "ticker boş", t.elapsed().as_millis() as u64),
        }
    }
    {
        let t = std::time::Instant::now();
        match app.binance.server_time_ms().await {
            Ok(ts) if ts > 0 => push!(11, "Ortam", "Binance saat", "PASS", format!("server_time_ms={ts}"), t.elapsed().as_millis() as u64),
            _ => push!(11, "Ortam", "Binance saat", "ABSTAIN", "Binance'e erişilemiyor — atlandı (dürüst abstention)", t.elapsed().as_millis() as u64),
        }
    }
    // ---- İmza: Bybit (anahtarlı) ----
    let bybit_keys = active_keypair(&app, "BYBIT");
    let has_any_key = bybit_keys.is_some();
    if let Some(creds) = bybit_keys {
        let t = std::time::Instant::now();
        match app.bybit.key_permissions(&creds).await {
            Ok(p) => {
                push!(12, "İmza/Bybit", "Anahtar geçerli", "PASS", p.note.chars().take(120).collect::<String>(), t.elapsed().as_millis() as u64);
                push!(13, "İmza/Bybit", "Trade yetkisi", if p.spot_trade || p.contract_trade { "PASS" } else { "FAIL" }, format!("spot={} contract={}", p.spot_trade, p.contract_trade), 0);
                if p.can_withdraw {
                    hard_fail = true;
                    push!(14, "İmza/Bybit", "Çekim yasağı (HARD-FAIL)", "FAIL", "canWithdraw AÇIK — anahtarı derhal değiştirin", 0);
                } else {
                    push!(14, "İmza/Bybit", "Çekim yasağı (HARD-FAIL)", "PASS", "withdraw kapalı", 0);
                }
                if p.read_only {
                    push!(15, "İmza/Bybit", "Salt-okunur değil", "FAIL", "anahtar read-only — trade yetkisi yok", 0);
                } else {
                    push!(15, "İmza/Bybit", "Salt-okunur değil", "PASS", "trade yetkisi var", 0);
                }
            }
            Err(e) => {
                for id in 12..=15 {
                    push!(id, "İmza/Bybit", if id == 12 { "Anahtar geçerli" } else if id == 13 { "Trade yetkisi" } else if id == 14 { "Çekim yasağı (HARD-FAIL)" } else { "Salt-okunur değil" }, "FAIL", format!("izin sondası başarısız: {}", e.chars().take(100).collect::<String>()), 0);
                }
            }
        }
        let t = std::time::Instant::now();
        match app.bybit.wallet(&creds, "USDT").await {
            Ok(w) => push!(16, "İmza/Bybit", "Cüzdan okuma (USDT)", "PASS", format!("bakiye {}", w.wallet_balance.to_half_even_display(4)), t.elapsed().as_millis() as u64),
            Err(e) => push!(16, "İmza/Bybit", "Cüzdan okuma (USDT)", "FAIL", format!("cüzdan okunamadı: {}", e.chars().take(100).collect::<String>()), t.elapsed().as_millis() as u64),
        }
        let t = std::time::Instant::now();
        match app.bybit.positions(&creds, &symbol).await {
            Ok(pos) => push!(17, "İmza/Bybit", "Pozisyon okuma", "PASS", format!("{}: {} kayıt", symbol, pos.len()), t.elapsed().as_millis() as u64),
            Err(e) => push!(17, "İmza/Bybit", "Pozisyon okuma", "FAIL", format!("pozisyon okunamadı: {}", e.chars().take(100).collect::<String>()), t.elapsed().as_millis() as u64),
        }
    } else {
        for (id, name) in [(12, "Anahtar geçerli"), (13, "Trade yetkisi"), (14, "Çekim yasağı (HARD-FAIL)"), (15, "Salt-okunur değil"), (16, "Cüzdan okuma (USDT)"), (17, "Pozisyon okuma")] {
            push!(id, "İmza/Bybit", name, "ABSTAIN", "Bybit anahtarı yok — atlandı", 0);
        }
    }
    // ---- İmza: Binance (anahtarlı) ----
    if let Some(creds) = active_keypair(&app, "BINANCE") {
        let t = std::time::Instant::now();
        match app.binance.key_permissions(&creds).await {
            Ok(p) => {
                push!(18, "İmza/Binance", "Anahtar geçerli", "PASS", p.note.chars().take(120).collect::<String>(), t.elapsed().as_millis() as u64);
                push!(19, "İmza/Binance", "Trade yetkisi + çekim yasağı", if (p.spot_trade || p.contract_trade) && !p.can_withdraw { "PASS" } else { "FAIL" }, format!("trade={}/{}, withdraw={}", p.spot_trade, p.contract_trade, p.can_withdraw), 0);
                if p.can_withdraw {
                    hard_fail = true;
                }
            }
            Err(e) => {
                push!(18, "İmza/Binance", "Anahtar geçerli", "FAIL", format!("izin sondası başarısız: {}", e.chars().take(100).collect::<String>()), 0);
                push!(19, "İmza/Binance", "Trade yetkisi + çekim yasağı", "ABSTAIN", "anahtar doğrulanamadı", 0);
            }
        }
    } else {
        push!(18, "İmza/Binance", "Anahtar geçerli", "ABSTAIN", "Binance anahtarı yok — atlandı", 0);
        push!(19, "İmza/Binance", "Trade yetkisi + çekim yasağı", "ABSTAIN", "Binance anahtarı yok — atlandı", 0);
    }
    // ---- İmza: OKX (anahtarlı) ----
    // 20 halka-açık saat, 21 halka-açık klines, 22-23 imzalı sondalar.
    {
        let t = std::time::Instant::now();
        match app.okx.server_time_ms().await {
            Ok(ts) if ts > 0 => push!(20, "Ortam", "OKX saat", "PASS", format!("server_time_ms={ts}"), t.elapsed().as_millis() as u64),
            _ => push!(20, "Ortam", "OKX saat", "ABSTAIN", "OKX'e erişilemiyor — atlandı (dürüst abstention)", t.elapsed().as_millis() as u64),
        }
    }
    {
        let t = std::time::Instant::now();
        match app.okx.klines("spot", &symbol, "1H", 5).await {
            Ok(b) if !b.is_empty() => push!(21, "Ortam", "OKX klines (spot)", "PASS", format!("{}: {} bar", symbol, b.len()), t.elapsed().as_millis() as u64),
            _ => push!(21, "Ortam", "OKX klines (spot)", "ABSTAIN", format!("{symbol} bar alınamadı — atlandı"), t.elapsed().as_millis() as u64),
        }
    }
    let okx_keys = active_keypair(&app, "OKX");
    if let Some(creds) = okx_keys {
        if creds.passphrase.as_deref().map(|p| p.trim().is_empty()).unwrap_or(true) {
            push!(22, "İmza/OKX", "Anahtar + passphrase", "FAIL", "passphrase yok — anahtarı passphrase ile yeniden ekleyin", 0);
            push!(23, "İmza/OKX", "Cüzdan okuma (USDT)", "ABSTAIN", "passphrase yok", 0);
        } else {
            let t = std::time::Instant::now();
            match app.okx.key_permissions(&creds).await {
                Ok(p) => push!(22, "İmza/OKX", "Anahtar + passphrase", "PASS", p.note.chars().take(120).collect::<String>(), t.elapsed().as_millis() as u64),
                Err(e) => push!(22, "İmza/OKX", "Anahtar + passphrase", "FAIL", format!("izin sondası başarısız: {}", e.chars().take(100).collect::<String>()), t.elapsed().as_millis() as u64),
            }
            let t = std::time::Instant::now();
            match app.okx.wallet(&creds, "USDT").await {
                Ok(w) => push!(23, "İmza/OKX", "Cüzdan okuma (USDT)", "PASS", format!("bakiye {}", w.wallet_balance.to_half_even_display(4)), t.elapsed().as_millis() as u64),
                Err(e) => push!(23, "İmza/OKX", "Cüzdan okuma (USDT)", "FAIL", format!("cüzdan okunamadı: {}", e.chars().take(100).collect::<String>()), t.elapsed().as_millis() as u64),
            }
        }
    } else {
        push!(22, "İmza/OKX", "Anahtar + passphrase", "ABSTAIN", "OKX anahtarı yok — atlandı", 0);
        push!(23, "İmza/OKX", "Cüzdan okuma (USDT)", "ABSTAIN", "OKX anahtarı yok — atlandı", 0);
    }
    let has_keys = has_any_key || active_keypair(&app, "BINANCE").is_some() || active_keypair(&app, "OKX").is_some();
    let verdict = if hard_fail || failed > 0 { "KALDI" } else if has_keys { "GECTI" } else { "YETERSIZ_ORTAM" };
    let _ran = passed + failed;
    let note = if hard_fail {
        "HARD-FAIL: çekim yetkisi açık anahtar var — canlı hat derhal kilitlenmeli. Withdraw yetkisini kaldırın."
    } else if failed > 0 {
        "Başarısız kontrol var — canlıya güvenmeden önce düzeltin."
    } else if has_keys {
        "Koşan tüm kontroller geçti — canlı kapı açık. (Test-order bacağı bilinçli yok; dry-run ayrı iştir.)"
    } else {
        "Anahtar yok: ortam kontrolleri geçti, imzalılar atlandı. Anahtarlar Ayarlar > Kasa'ya eklenir."
    };
    let elapsed = t0.elapsed().as_millis() as u64;
    state::persist_event(&app, "SYSTEM", "INFO", "SELF_CHECK", &format!("verdict={verdict} passed={passed} failed={failed} abstained={abstained}")).await;
    Json(json!({
        "verdict": verdict,
        "passed": passed,
        "failed": failed,
        "abstained": abstained,
        "hardFail": hard_fail,
        "items": items,
        "symbol": symbol,
        "elapsedMs": elapsed,
        "note": note,
    }))
}

// ---------------- Kara liste (v3 /api/blacklist karşılığı) ----------------
//
// GET    /api/blacklist        → { blacklist: [{pair, reason, createdAt}] }
// POST   /api/blacklist        → {pair, reason?} ekle/güncelle (idempotent)
// DELETE /api/blacklist/:pair  → kaldır (yoksa dürüst 404)
// (yardımcı `is_blacklisted` routes_common.rs'tedir — import satır 38.)

async fn list_blacklist(State(app): State<Arc<AppState>>) -> Json<Value> {
    let rows: Vec<Value> = app.db.conn.lock().map(|c| {
        let mut stmt = match c.prepare("SELECT pair, reason, created_at FROM pair_blacklist ORDER BY pair") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        stmt.query_map([], |r| {
            Ok(json!({
                "pair": r.get::<_, String>(0)?,
                "reason": r.get::<_, Option<String>>(1)?,
                "createdAt": r.get::<_, i64>(2)?,
            }))
        }).map(|it| it.filter_map(|r| r.ok()).collect()).unwrap_or_default()
    }).unwrap_or_default();
    Json(json!({"blacklist": rows}))
}

#[derive(Deserialize)]
struct BlacklistReq {
    pair: String,
    reason: Option<String>,
}

async fn add_blacklist(State(app): State<Arc<AppState>>, Json(req): Json<BlacklistReq>) -> Result<Json<Value>, axum::response::Response> {
    let pair = normalize_pair(&req.pair);
    if pair.is_empty() {
        return Err(err422("pair boş — örn. BTCUSDT"));
    }
    let reason = req.reason.map(|r| r.chars().take(200).collect::<String>()).filter(|r| !r.trim().is_empty());
    let now = state::now_ms();
    {
        let conn = app.db.conn.lock().map_err(|_| err422("kasa kilitli"))?;
        conn.execute(
            "INSERT INTO pair_blacklist(pair, reason, created_at) VALUES(?1,?2,?3) ON CONFLICT(pair) DO UPDATE SET reason=excluded.reason",
            rusqlite::params![pair, reason, now],
        ).map_err(|e| err422(format!("kara liste yazılamadı: {e}")))?;
    }
    state::persist_event(&app, "SYSTEM", "INFO", "BLACKLIST", &format!("eklendi/güncellendi: {pair}")).await;
    Ok(Json(json!({"pair": pair, "reason": reason, "createdAt": now})))
}

async fn remove_blacklist(State(app): State<Arc<AppState>>, Path(pair): Path<String>) -> Result<Json<Value>, axum::response::Response> {
    let pair = normalize_pair(&pair);
    let n = {
        let conn = app.db.conn.lock().map_err(|_| err422("kasa kilitli"))?;
        conn.execute("DELETE FROM pair_blacklist WHERE pair=?1", [&pair]).map_err(|e| err422(format!("kara liste silinemedi: {e}")))?
    };
    if n == 0 {
        return Err(err404(format!("kara listede yok: {pair}")));
    }
    state::persist_event(&app, "SYSTEM", "INFO", "BLACKLIST", &format!("kaldırıldı: {pair}")).await;
    Ok(Json(json!({"removed": pair})))
}

// ---------------- Rejim rozeti (v3 /api/analysis/regime karşılığı) ---------
//
// GET /api/analysis/regime?symbol=BTCUSDT&interval=1h&bars=300
// Motorun kendi MarketRegime sınıflandırıcısı (dca_signals::regime) ile
// güncel rejim + kapanış adedi. Kural-bazlı, look-ahead yok.
#[derive(Deserialize)]
struct RegimeQuery {
    symbol: Option<String>,
    interval: Option<String>,
    bars: Option<usize>,
    venue: Option<String>,
    category: Option<String>,
}

async fn analysis_regime(State(app): State<Arc<AppState>>, Query(q): Query<RegimeQuery>) -> Result<Json<Value>, axum::response::Response> {
    let symbol = q.symbol.map(|s| normalize_pair(&s)).filter(|s| !s.is_empty()).unwrap_or_else(|| "BTCUSDT".into());
    let iv = engine::bybit_interval(q.interval.as_deref().unwrap_or("1h")).map_err(err422)?;
    let bars = q.bars.unwrap_or(300).clamp(60, 1500);
    let kl = match q.venue.as_deref().unwrap_or("BYBIT") {
        "BINANCE" => app.binance.klines(q.category.as_deref().unwrap_or("linear"), &symbol, &iv, bars).await,
        "OKX" => {
            let oiv = dca_exchange::okx::okx_bar(&iv).map_err(err422)?;
            app.okx.klines(q.category.as_deref().unwrap_or("linear"), &symbol, oiv, bars).await
        }
        _ => app.bybit.klines(q.category.as_deref().unwrap_or("linear"), &symbol, &iv, bars).await,
    }.map_err(err422)?;
    if kl.len() < 60 {
        return Err(err422(format!("yetersiz bar: {} (min 60)", kl.len())));
    }
    let closes: Vec<f64> = kl.iter().filter_map(|k| k.c.parse::<f64>().ok()).collect();
    let regime = dca_signals::classify_regime(&closes);
    Ok(Json(json!({
        "symbol": symbol,
        "regime": regime.label(),
        "blocksNewDeals": regime.is_risky(),
        "bars": closes.len(),
        "note": "Motorun rejim filtresiyle aynı sınıflandırıcı (dca_signals::regime). CRASH/HIGH_VOLATILITY yeni deal'i bloke eder.",
    })))
}

// ---------------- Kalibrasyon (v3 /api/analysis/calibration karşılığı) -----
//
// POST /api/analysis/calibration {symbol, interval, bars?, deviationStepBase?}
// Son N barın log-getirilerinden μ/σ + %95 düşüş kuyruğu (q95) hesaplanır;
// önerilen sapma adımı = clamp(2σ, taban, %8). Finansal iddia yok — sadece
// başlangıç noktası önerisi; kararı kullanıcı verir, backtest doğrular.
#[derive(Deserialize)]
struct CalibrationReq {
    symbol: Option<String>,
    interval: Option<String>,
    bars: Option<usize>,
    #[serde(default)]
    deviation_step_base: Option<String>,
}

async fn analysis_calibration(State(app): State<Arc<AppState>>, Json(req): Json<CalibrationReq>) -> Result<Json<Value>, axum::response::Response> {
    let symbol = req.symbol.map(|s| normalize_pair(&s)).filter(|s| !s.is_empty()).unwrap_or_else(|| "BTCUSDT".into());
    let iv = engine::bybit_interval(req.interval.as_deref().unwrap_or("1h")).map_err(err422)?;
    let bars = req.bars.unwrap_or(500).clamp(60, 1000);
    let kl = app.bybit.klines("linear", &symbol, &iv, bars).await.map_err(err422)?;
    let closes: Vec<f64> = kl.iter().filter_map(|k| k.c.parse::<f64>().ok()).filter(|c| *c > 0.0).collect();
    if closes.len() < 26 {
        return Err(err422(format!("yetersiz kapanış: {} (min 26)", closes.len())));
    }
    let rets: Vec<f64> = closes.windows(2).map(|w| (w[1] / w[0]).ln()).collect();
    let n = rets.len() as f64;
    let mu = rets.iter().sum::<f64>() / n;
    let var = rets.iter().map(|r| (r - mu).powi(2)).sum::<f64>() / n;
    let sigma = var.sqrt();
    let mut sorted = rets.clone();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let q05 = sorted[(0.05 * n).floor() as usize % sorted.len()];
    let base = req.deviation_step_base.as_deref().and_then(|s| s.parse::<f64>().ok()).filter(|b| *b > 0.0).unwrap_or(0.3);
    let suggested = (2.0 * sigma * 100.0).clamp(base, 8.0);
    Ok(Json(json!({
        "symbol": symbol,
        "bars": closes.len(),
        "muPct": format!("{:.4}", mu * 100.0),
        "sigmaPct": format!("{:.4}", sigma * 100.0),
        "q95DropPct": format!("{:.4}", (-q05).max(0.0) * 100.0),
        "suggestedDeviationPct": format!("{:.2}", suggested),
        "note": "Öneri = 2σ (bar-başı), taban % ile %8 arası. Başlangıç noktasıdır — backtest ile doğrulayın.",
    })))
}

// ---------------- Bakım sentineli (v3 /api/system/maintenance karşılığı) ---
//
// GET  /api/system/maintenance → anlık görüntü: venue saat sondaları +
//   operatör sessizlik bayrağı (settings tablosu: maint_quiet=0/1 + notu).
// POST /api/system/maintenance {quiet, note?} → bayrağı aç/kapat.
// Dürüst sınır: borsa planlı-bakım takvim API'si yok; sondalar o anki
// erişimi raporlar. QUIET bilgilendiricidir (otomatik durdurma yapmaz —
// durdurma için Kill-switch/Freeze kullanılır).
fn maint_flag(app: &Arc<AppState>) -> (bool, String) {
    let get = |k: &str| -> String {
        app.db.conn.lock().ok()
            .and_then(|c| c.query_row("SELECT v FROM settings WHERE k=?1", [k], |r| r.get::<_, String>(0)).ok())
            .unwrap_or_default()
    };
    (get("maint_quiet") == "1", get("maint_note"))
}

async fn venue_probe(name: &str, fut: impl std::future::Future<Output = Result<i64, String>>) -> Value {
    let t = std::time::Instant::now();
    let ms = || t.elapsed().as_millis() as u64;
    match fut.await {
        Ok(ts) if ts > 0 => json!({"venue": name, "ok": true, "serverTimeMs": ts, "ms": ms()}),
        Ok(_) => json!({"venue": name, "ok": false, "error": "sıfır zaman", "ms": ms()}),
        Err(e) => json!({"venue": name, "ok": false, "error": e.chars().take(120).collect::<String>(), "ms": ms()}),
    }
}

async fn get_maintenance(State(app): State<Arc<AppState>>) -> Json<Value> {
    let bybit = venue_probe("BYBIT", app.bybit.server_time_ms()).await;
    let binance = venue_probe("BINANCE", app.binance.server_time_ms()).await;
    let okx = venue_probe("OKX", app.okx.server_time_ms()).await;
    let (quiet, note) = maint_flag(&app);
    let degraded = !bybit.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
        || !binance.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
        || !okx.get("ok").and_then(|v| v.as_bool()).unwrap_or(false);
    let computed = if quiet { "QUIET" } else if degraded { "DEGRADED" } else { "NORMAL" };
    // Faz 1.5: bellek-ici simulasyon ust-yazimi (suresi dolmussa yok sayilir).
    let now = state::now_ms();
    let simulated = crate::thin::sim_active(&*app.maint_sim.lock().await, now);
    let (mode, sim_flag) = match simulated {
        Some(m) => (m, true),
        None => (computed.to_string(), false),
    };
    Json(json!({
        "mode": mode,
        "quiet": quiet,
        "note": note,
        "venues": [bybit, binance, okx],
        "checkedAt": now,
        "simulated": sim_flag,
    }))
}

#[derive(Deserialize)]
struct MaintenanceReq {
    quiet: bool,
    note: Option<String>,
}

async fn set_maintenance(State(app): State<Arc<AppState>>, Json(req): Json<MaintenanceReq>) -> Result<Json<Value>, axum::response::Response> {
    let note: String = req.note.map(|n| n.chars().take(200).collect()).unwrap_or_default();
    {
        let conn = app.db.conn.lock().map_err(|_| err422("kasa kilitli"))?;
        for (k, v) in [("maint_quiet", if req.quiet { "1" } else { "0" }), ("maint_note", note.as_str())] {
            conn.execute("INSERT INTO settings(k, v) VALUES(?1,?2) ON CONFLICT(k) DO UPDATE SET v=excluded.v", [k, v])
                .map_err(|e| err422(format!("bakım bayrağı yazılamadı: {e}")))?;
        }
    }
    state::persist_event(&app, "SYSTEM", "INFO", "MAINTENANCE", &format!("quiet={} note={}", req.quiet, note)).await;
    Ok(Json(json!({"quiet": req.quiet, "note": note})))
}

// ---------------- Orderbook L2 (v3 /api/market/orderbook karşılığı) -------
//
// GET /api/orderbook?symbol=BTCUSDT&venue=BYBIT&category=linear&limit=25
// → bids/asks + best bid/ask + spread_bps + imbalance (-1..+1, + alım baskısı).
// Dürüst sınırlar: REST anlık görüntü (WS diff akışı yok); imbalance f64
// gösterim hesabıdır (emir matematiği değil); PAPER desteklemez; Binance
// adaptörü spot-kanallıdır (linear → 422).
#[derive(Deserialize)]
struct OrderbookQuery {
    symbol: Option<String>,
    venue: Option<String>,
    category: Option<String>,
    limit: Option<usize>,
}

async fn orderbook(State(app): State<Arc<AppState>>, Query(q): Query<OrderbookQuery>) -> Result<Json<Value>, axum::response::Response> {
    let symbol = q.symbol.map(|s| normalize_pair(&s)).filter(|s| !s.is_empty()).unwrap_or_else(|| "BTCUSDT".into());
    let category = q.category.unwrap_or_else(|| "linear".into());
    let limit = q.limit.unwrap_or(25).clamp(1, 100);
    let book = match q.venue.as_deref().unwrap_or("BYBIT") {
        "BINANCE" => app.binance.orderbook(&category, &symbol, limit).await,
        "OKX" => app.okx.orderbook(&category, &symbol, limit).await,
        "PAPER" => return Err(err422("PAPER venue'de orderbook yok — Bybit/Binance/OKX seçin")),
        _ => app.bybit.orderbook(&category, &symbol, limit).await,
    }.map_err(err422)?;
    if book.bids.is_empty() || book.asks.is_empty() {
        return Err(err422(format!("{symbol}: derinlik boş — senkron değil (fail-closed)")));
    }
    let f = |s: &str| s.parse::<f64>().unwrap_or(0.0);
    let (mut bv, mut av) = (0.0, 0.0);
    for l in &book.bids { bv += f(&l.qty); }
    for l in &book.asks { av += f(&l.qty); }
    let (bb, ba) = (f(&book.bids[0].price), f(&book.asks[0].price));
    let mid = (bb + ba) / 2.0;
    Ok(Json(json!({
        "symbol": book.symbol,
        "bids": book.bids,
        "asks": book.asks,
        "bestBid": book.bids[0].price,
        "bestAsk": book.asks[0].price,
        "spreadBps": if mid > 0.0 { format!("{:.1}", (ba - bb) / mid * 10000.0) } else { "—".into() },
        "imbalance": if bv + av > 0.0 { format!("{:.3}", (bv - av) / (bv + av)) } else { "0".into() },
        "ts": book.ts,
        "note": "REST anlık görüntü; WS akışı yok. Imbalance gösterim hesabıdır.",
    })))
}

// ---------------- Vadeli durum kartı (v3 futures/state+symbol karşılığı) --
//
// GET /api/live/futures-state?symbol=BTCUSDT&venue=BYBIT
// → enstrüman kuralları (maxLeverage/tick/step/minNotional) + anahtar varsa
//   canlı pozisyon (salt-okunur; ayar değiştiren uç YOKTUR).
// v3'teki ayrı check/state/symbol kartlarının tek uçta birleşimi.
// P4-3d: BYBIT + OKX tam vadeli kart (kurallar + canlı pozisyon);
// Binance adaptörü spot — venue=BINANCE → dürüst 422.
#[derive(Deserialize)]
struct FuturesStateQuery {
    symbol: Option<String>,
    venue: Option<String>,
}

async fn futures_state(State(app): State<Arc<AppState>>, Query(q): Query<FuturesStateQuery>) -> Result<Json<Value>, axum::response::Response> {
    let venue = q.venue.unwrap_or_else(|| "BYBIT".into());
    if venue == "BINANCE" {
        return Err(err422("Binance vadeli kanalı yok (adaptör spot) — Bybit seçin"));
    }
    if venue == "PAPER" {
        return Err(err422("Paper'da vadeli durum yok"));
    }
    let symbol = q.symbol.map(|s| normalize_pair(&s)).filter(|s| !s.is_empty()).unwrap_or_else(|| "BTCUSDT".into());
    let adapter = app.adapter(&venue).ok_or_else(|| err422("venue desteklenmiyor"))?;
    let inst = adapter.instruments("linear", Some(&symbol)).await.map_err(err422)?;
    let rule = inst.iter().find(|i| i.symbol == symbol);
    let live = match active_keypair(&app, &venue) {
        Some(creds) => {
            let pos = adapter.positions(&creds, &symbol).await.map_err(err422)?;
            let open: Vec<Value> = pos.iter().filter(|p| !p.size.is_zero())
                .map(|p| json!({"side": p.side, "size": p.size.to_half_even_display(8), "avgPrice": p.avg_price.to_half_even_display(8), "leverage": p.leverage.to_half_even_display(2)}))
                .collect();
            json!({"supported": true, "positions": open})
        }
        None => json!({"supported": false, "note": "aktif anahtar yok — pozisyon bacağı atlandı"}),
    };
    Ok(Json(json!({
        "symbol": symbol,
        "venue": venue,
        "maxLeverage": rule.map(|i| i.max_leverage.to_half_even_display(0)),
        "tickSize": rule.map(|i| i.tick_size.to_half_even_display(8)),
        "minNotional": rule.map(|i| i.min_notional.to_half_even_display(2)),
        "live": live,
        "note": "Salt-okunur kart; kaldıraç/marj değişikliği buradan yapılmaz.",
    })))
}

// ---------------- Grid canlı mesh + deal-tick (v3 grid-live/deals-tick) ----
//
// GET  /api/bots/:id/grid → mesh seviyeleri + armed bayrakları + bootstrap
//   durumu. Yalnız grid botlarında; DCA/hedge → dürüst 422.
// POST /api/bots/:id/tick → botun olağan poll turunu HEMEN koşturur
//   (v3 deals/tick karşılığı; motor zaten periyodik poll yapar, bu uç manuel
//   tetikler) → { newBars, snapshot }.
async fn bot_grid(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>, axum::response::Response> {
    let rts = app.runtimes.lock().await;
    let rt = rts.get(&id).ok_or_else(|| err404(format!("bot yok: {id}")))?;
    match &rt.engine_kind {
        dca_strategy::EngineTag::Grid(g) => {
            let mesh: Vec<String> = g.mesh.iter().map(|m| m.to_half_even_display(8)).collect();
            Ok(Json(json!({
                "id": id,
                "kind": g.kind.label(),
                "bootstrapped": g.gate_open,
                "mesh": mesh,
                "armed": g.level_armed,
                "levels": g.mesh.len(),
                "note": "armed=true: o seviyede bekleyen grid emri var.",
            })))
        }
        _ => Err(err422("grid botu değil — mesh yalnız Spot/Futures/Infinity Grid'de var")),
    }
}

async fn bot_tick(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Result<Json<Value>, axum::response::Response> {
    // BOT-08: STOPPED/PAUSED botta manuel tetikle yeni emir üretilemez.
    let status = {
        let rts = app.runtimes.lock().await;
        rts.get(&id).map(|rt| rt.status.clone()).unwrap_or_default()
    };
    if status == "STOPPED" || status == "PAUSED" {
        return Err(err422(format!("bot {status} — manuel tick yeni emir üretemez")));
    }
    let sse = app.sse.clone();
    let new_bars = engine::poll_and_advance(&app, &id, &sse).await.map_err(err422)?;
    let rts = app.runtimes.lock().await;
    let snap = rts.get(&id).map(|rt| rt.snapshot_json()).unwrap_or(Value::Null);
    Ok(Json(json!({"id": id, "newBars": new_bars, "snapshot": snap})))
}

// ---------------- Haber motoru (v3 /api/market/news karşılığı) ------------
//
// GET /api/market/news?symbol=BTCUSDT → başlıklar + duyarlılık [-1,+1] +
//   etki + Acil Delisting Kapısı + toplu istatistik. 5 dk önbellek.
// Bacaklar: CryptoPanic (CRYPTOPANIC_AUTH_TOKEN varsa) + Binance duyuruları
// (anahtarsız). Token yoksa yalnız Binance bacağı — sources alanında dürüstçe
// yazılır. Duyarlılık v3 sözlüğüyle aynı ağırlıklar (küçük harf içerir-eşleşme;
// v3 \b sınırlarına yaklaşım — belgelenmiş fark). Ham HTML istemciye gitmez:
// yalnız başlık/kaynak/URL/zaman taşınır.
const NEWS_BULLISH: &[(&str, f64)] = &[
    ("surge", 0.6), ("surged", 0.6), ("surges", 0.6), ("soar", 0.6), ("rally", 0.6), ("rallies", 0.6),
    ("partnership", 0.5), ("collaboration", 0.5), ("integration", 0.5),
    ("adoption", 0.5), ("onboard", 0.5), ("approval", 0.7), ("approved", 0.7), ("green-light", 0.7), ("greenlight", 0.7),
    ("inflow", 0.5), ("upgrade", 0.4), ("halving", 0.4), ("burn", 0.4), ("burned", 0.4),
    ("launch", 0.35), ("lists", 0.35), ("listing", 0.35), ("institutional", 0.4), ("treasury", 0.4), ("reserve", 0.4),
    ("bullish", 0.45), ("accumulate", 0.45), ("record", 0.5), ("all-time high", 0.5), ("ath", 0.5),
];
const NEWS_BEARISH: &[(&str, f64)] = &[
    ("delisting", 1.0), ("delist", 1.0), ("trading suspension", 0.8), ("suspend", 0.8),
    ("removal of trading pairs", 1.0), ("hack", 0.9), ("exploit", 0.9), ("breach", 0.9), ("drain", 0.9),
    ("lawsuit", 0.6), ("litigation", 0.6), ("ban", 0.7), ("banned", 0.7), ("crackdown", 0.7),
    ("crash", 0.6), ("plunge", 0.6), ("dump", 0.6), ("tank", 0.6),
    ("liquidat", 0.5), ("outage", 0.5), ("downtime", 0.5),
    ("fraud", 0.8), ("laundering", 0.8), ("scam", 0.8), ("rug pull", 0.8), ("rugpull", 0.8),
    ("bearish", 0.35), ("correction", 0.35), ("rate hike", 0.4),
];
const NEWS_DELISTING_KW: &[&str] = &["delisting", "delist", "removal of trading pairs", "trading suspension", "will be delisted", "spot trading halt"];

fn news_sentiment(text: &str) -> (f64, &'static str) {
    let t = text.to_lowercase();
    let mut score = 0.0;
    for (kw, w) in NEWS_BULLISH { if t.contains(kw) { score += w; } }
    for (kw, w) in NEWS_BEARISH { if t.contains(kw) { score -= w; } }
    let s = (score / 1.2).tanh();
    let a = s.abs();
    let impact = if a >= 0.75 { "CRITICAL" } else if a >= 0.5 { "HIGH" } else if a >= 0.25 { "MEDIUM" } else { "LOW" };
    ((s * 1000.0).round() / 1000.0, impact)
}

fn news_safe_url(u: &str) -> Option<String> {
    let low = u.trim().to_lowercase();
    if !(low.starts_with("https://cryptopanic.com/") || low.starts_with("https://www.binance.com/")) {
        return None;
    }
    Some(u.trim().chars().take(300).collect())
}

#[derive(Deserialize)]
struct NewsQuery {
    symbol: Option<String>,
}

async fn market_news(State(app): State<Arc<AppState>>, Query(q): Query<NewsQuery>) -> Json<Value> {
    let symbol = q.symbol.map(|s| normalize_pair(&s)).filter(|s| s.len() >= 4 && s.len() <= 20).unwrap_or_else(|| "BTCUSDT".into());
    Json(get_market_news(&app, &symbol).await)
}

/// P4-2 ile paylaşılan haber alımı: önbellek + CryptoPanic/Binance bacakları.
/// market_news ince sarmalayıcıdır; ai modülü duyarlılık için bunu kullanır.
pub(crate) async fn get_market_news(app: &Arc<AppState>, symbol: &str) -> Value {
    {
        let cache = app.news_cache.lock().await;
        if let Some((at, v)) = cache.get(symbol) {
            if state::now_ms() - *at < 300_000 {
                let mut hit = (*v).clone();
                hit["cached"] = json!(true);
                return hit;
            }
        }
    }
    let asset = symbol.trim_end_matches("USDT").trim_end_matches("USDC").trim_end_matches("BTC");
    let asset: &str = if asset.is_empty() { symbol } else { asset };
    let mut raw: Vec<(String, String, i64, String)> = Vec::new(); // title, url, ts, source
    let mut sources: Vec<String> = Vec::new();
    // Bacak 1: CryptoPanic (token varsa).
    if let Ok(token) = std::env::var("CRYPTOPANIC_AUTH_TOKEN") {
        if !token.trim().is_empty() {
            let url = format!("https://cryptopanic.com/api/free/v1/posts/?auth_token={}&currencies={}&public=true&kind=news", urlencoding(&token), urlencoding(asset));
            if let Ok(resp) = app.news_http.get(&url).send().await {
                if let Ok(v) = resp.json::<Value>().await {
                    if let Some(arr) = v.get("results").and_then(|r| r.as_array()) {
                        for r in arr.iter().take(20) {
                            let (Some(title), Some(link)) = (r.get("title").and_then(|x| x.as_str()), r.get("url").and_then(|x| x.as_str())) else { continue };
                            let Some(su) = news_safe_url(link) else { continue };
                            let ts = r.get("published_at").and_then(|x| x.as_str()).and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok()).map(|d| d.timestamp_millis()).unwrap_or_else(state::now_ms);
                            let src = r.get("source").and_then(|x| x.get("title")).and_then(|x| x.as_str()).unwrap_or("CryptoPanic");
                            raw.push((title.chars().take(200).collect(), su, ts, src.chars().take(60).collect()));
                        }
                        if !raw.is_empty() { sources.push("CryptoPanic".into()); }
                    }
                }
            }
        }
    }
    // Bacak 2: Binance duyuruları (anahtarsız; CryptoPanic boşsa tamamlar).
    if raw.is_empty() {
        for ep in [
            "https://www.binance.com/bapi/composite/v1/public/cms/article/catalog/list/queryCatalogArticles?catalogId=48&pageNo=1&pageSize=20",
            "https://www.binance.com/bapi/composite/v1/public/cms/article/list/query?type=1&pageNo=1&pageSize=20&catalogId=48",
        ] {
            if let Ok(resp) = app.news_http.get(ep).send().await {
                if let Ok(v) = resp.json::<Value>().await {
                    let mut arts: Option<&Vec<Value>> = None;
                    if let Some(cats) = v.pointer("/data/catalogs").and_then(|c| c.as_array()) {
                        for c in cats {
                            if let Some(a) = c.get("articles").and_then(|a| a.as_array()) { arts = Some(a); break; }
                        }
                    }
                    if arts.is_none() { arts = v.pointer("/data/articles").and_then(|a| a.as_array()); }
                    if let Some(list) = arts {
                        for a in list.iter().take(20) {
                            let (Some(title), Some(code)) = (a.get("title").and_then(|x| x.as_str()), a.get("code").and_then(|x| x.as_str())) else { continue };
                            let Some(su) = news_safe_url(&format!("https://www.binance.com/en/support/announcement/{code}")) else { continue };
                            let ts = a.get("releaseDate").and_then(|x| x.as_i64()).unwrap_or_else(state::now_ms);
                            raw.push((title.chars().take(200).collect(), su, ts, "Binance Announcements".into()));
                        }
                        if !raw.is_empty() { sources.push("Binance Announcements".into()); break; }
                    }
                }
            }
        }
    }
    // Skorla + sırala + delisting kapısı.
    let mut items: Vec<Value> = raw.into_iter().enumerate().map(|(i, (title, url, ts, source))| {
        let (s, impact) = news_sentiment(&title);
        json!({"id": format!("{symbol}-{i}-{ts}"), "title": title, "url": url, "publishedAt": ts, "source": source, "sentiment": s, "impact": impact})
    }).collect();
    items.sort_by(|a, b| b.get("publishedAt").and_then(|x| x.as_i64()).unwrap_or(0).cmp(&a.get("publishedAt").and_then(|x| x.as_i64()).unwrap_or(0)));
    items.truncate(20);
    let mut delisting: Value = json!({"active": false, "headline": Value::Null, "url": Value::Null});
    for n in items.iter_mut() {
        let text = format!("{} {}", n.get("title").and_then(|x| x.as_str()).unwrap_or(""), "").to_lowercase();
        if let Some(kw) = NEWS_DELISTING_KW.iter().find(|k| text.contains(**k)) {
            if text.contains(&asset.to_lowercase()) {
                n["impact"] = json!("CRITICAL");
                n["sentiment"] = json!(n.get("sentiment").and_then(|x| x.as_f64()).unwrap_or(0.0).min(-0.8));
                delisting = json!({"active": true, "headline": n.get("title").cloned().unwrap_or(Value::Null), "url": n.get("url").cloned().unwrap_or(Value::Null), "keyword": kw});
                break;
            }
        }
    }
    let (mean, bull, bear) = if items.is_empty() { (0.0, 0, 0) } else {
        let sum: f64 = items.iter().filter_map(|n| n.get("sentiment").and_then(|x| x.as_f64())).sum();
        let b = items.iter().filter(|n| n.get("sentiment").and_then(|x| x.as_f64()).unwrap_or(0.0) > 0.2).count();
        let d = items.iter().filter(|n| n.get("sentiment").and_then(|x| x.as_f64()).unwrap_or(0.0) < -0.2).count();
        ((sum / items.len() as f64 * 1000.0).round() / 1000.0, b, d)
    };
    let out = json!({
        "symbol": symbol,
        "items": items,
        "count": items.len(),
        "mean": mean,
        "bull": bull,
        "bear": bear,
        "neutral": items.len().saturating_sub(bull + bear),
        "delisting": delisting,
        "sources": sources,
        "cached": false,
        "note": if sources.is_empty() { "Haber alınamadı (token yoksa CRYPTOPANIC_AUTH_TOKEN ekleyin; Binance bacağı da erişilemedi)." } else { "Başlıklar düz metindir; ham HTML taşınmaz." },
    });
    app.news_cache.lock().await.insert(symbol.to_string(), (state::now_ms(), out.clone()));
    out
}

fn urlencoding(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { o.push(b as char); }
        else { o.push_str(&format!("%{b:02X}")); }
    }
    o
}

// ---------------- Signal preview (3Commas-style builder) ----------------

#[derive(Deserialize)]
struct SignalsPreviewReq {
    symbol: String,
    #[serde(default = "default_interval")]
    interval: String,
    venue: Option<String>,
    category: Option<String>,
    node: dca_signals::SignalNode,
}

async fn signals_preview(
    State(app): State<Arc<AppState>>,
    Json(req): Json<SignalsPreviewReq>,
) -> Result<axum::response::Response, axum::response::Response> {
    if let Err(e) = dca_signals::validate_signal_node(&req.node) {
        return Err(err422(format!("node: {e}")));
    }
    match engine::signal_preview(&app, &req.symbol, &req.interval, req.venue.as_deref(), req.category.as_deref(), &req.node).await {
        Ok(v) => Ok(Json(v).into_response()),
        Err(e) => Err(err422(e)),
    }
}

// ---------------- TradingView webhook (3Commas external signals) ----------------
//
// Alert payload: {"secret":"tvw_...","botId":"...","action":"START_DEAL",
//                 "pair":"BTCUSDT","timestamp":1776672000000}
// Security: constant-time secret compare (401), ±60 s replay window (400),
// fail-closed when a deal is already active (DEAL_ALREADY_ACTIVE).

#[derive(Deserialize)]
struct TradingViewWebhookReq {
    secret: String,
    #[serde(rename = "botId")]
    bot_id: String,
    action: String,
    #[serde(default)]
    pair: Option<String>,
    timestamp: i64,
}

async fn tradingview_webhook(
    State(app): State<Arc<AppState>>,
    connect_info: Option<axum::extract::ConnectInfo<SocketAddr>>,
    Json(req): Json<TradingViewWebhookReq>,
) -> axum::response::Response {
    // Faz 22 (P2-10 dilim): webhook rate-limit — IP bazlı sabit pencere
    // (60 sn / 60 istek). Secret doğrulamasından ÖNCE kapatılır: flood
    // denemeleri DB/runtime kilidine hiç dokunmaz.
    {
        let ip = connect_info
            .map(|peer| peer.0.ip().to_string())
            .unwrap_or_else(|| "unknown".into());
        let now = crate::state::now_ms();
        const WINDOW_MS: i64 = 60_000;
        const WINDOW_MAX: u32 = 60;
        const MAX_BUCKETS: usize = 2_000;
        let mut rate = app.webhook_rate.lock().await;
        rate.retain(|_, (started, _)| now.saturating_sub(*started) < WINDOW_MS);
        if !rate.contains_key(&ip) && rate.len() >= MAX_BUCKETS {
            return (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": "rate limit capacity exceeded"}))).into_response();
        }
        let entry = rate.entry(ip).or_insert((now, 0u32));
        if now.saturating_sub(entry.0) >= WINDOW_MS {
            *entry = (now, 0);
        }
        entry.1 = entry.1.saturating_add(1);
        if entry.1 > WINDOW_MAX {
            drop(rate);
            return (StatusCode::TOO_MANY_REQUESTS, Json(json!({"error": "rate limit exceeded"}))).into_response();
        }
    }
    let mut rts = app.runtimes.lock().await;
    // SEC-07: Anti-enumeration & Constant-time secret comparison (unknown bot ve wrong secret aynı yanıtı verir).
    let expected_secret = rts.get(&req.bot_id).map(|r| r.webhook_secret.clone()).unwrap_or_default();
    let is_valid = !expected_secret.is_empty() && constant_time_eq_str(&req.secret, &expected_secret);
    if !is_valid {
        drop(rts);
        tracing::warn!("TradingView webhook rejected: invalid secret or unknown bot");
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "UNAUTHORIZED"}))).into_response();
    }
    let Some(rt) = rts.get_mut(&req.bot_id) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "UNAUTHORIZED"}))).into_response();
    };
    // Replay protection: accept epoch seconds or milliseconds, reject stale alerts.
    let now = state::now_ms();
    let ts_ms = if req.timestamp < 1_000_000_000_000 { req.timestamp.saturating_mul(1000) } else { req.timestamp };
    if (now - ts_ms).abs() > 60_000 {
        crate::state::persist_event(&app, &req.bot_id, "WARN", "WEBHOOK", "rejected: expired alert (>60s)").await;
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "EXPIRED_ALERT"}))).into_response();
    }
    // H-7 ince iş: alert-id dedup — aynı (bot, action, timestamp) ikinci kez
    // işlenmez (deal kapandıktan sonra pencere içinde tekrar gelirse çift deal yok).
    {
        let key = format!("{}|{}|{}", req.bot_id, req.action, ts_ms);
        let mut seen = app.webhook_seen.lock().await;
        seen.retain(|_, t| now - *t < 600_000); // 10 dk pencere dışını temizle
        if seen.contains_key(&key) {
            drop(seen);
            crate::state::persist_event(&app, &req.bot_id, "WARN", "WEBHOOK", "rejected: duplicate alert (already processed)").await;
            return (StatusCode::BAD_REQUEST, Json(json!({"error": "DUPLICATE_ALERT"}))).into_response();
        }
        seen.insert(key, now);
    }
    // Task 18 N-3: pair sözleşmesi — secret doğrulamasından SONRA denetlenir
    // (bilinmeyen arayanlara sembol bilgisi sızmaz); uyuşmazlık 400 + WARN.
    if let Some(p) = req.pair.as_deref() {
        if !p.is_empty() && p != rt.cfg.symbol {
            let msg = format!("webhook pair uyuşmazlığı: alert '{p}', bot '{}'", rt.cfg.symbol);
            crate::state::persist_event(&app, &req.bot_id, "WARN", "WEBHOOK", &msg).await;
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({"error": "PAIR_MISMATCH", "expected": rt.cfg.symbol, "got": p})),
            )
                .into_response();
        }
    }
    if req.action != "START_DEAL" {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "unknown action — expected START_DEAL"}))).into_response();
    }
    if rt.status != "RUNNING" {
        return Json(json!({"status": "BOT_NOT_RUNNING"})).into_response();
    }
    // Faz 0.2/0.3 — TV ENTRY kapısı (v3: kill tüm modlarda reddeder, freeze yalnız LIVE'da).
    if app.kill_engaged().await {
        crate::state::persist_event(&app, &req.bot_id, "WARN", crate::state::GATE_KILL, "START_DEAL reddedildi — KILL_SWITCH devrede").await;
        return Json(json!({"status": "KILL_SWITCH_ENGAGED"})).into_response();
    }
    if rt.mode == "LIVE" && app.freeze_engaged().await {
        crate::state::persist_event(&app, &req.bot_id, "WARN", crate::state::GATE_FREEZE, "START_DEAL reddedildi — FREEZE devrede (LIVE hat)").await;
        return Json(json!({"status": "FREEZE_ENGAGED"})).into_response();
    }
    // Fail-closed: an already-active deal must never be doubled by a signal.
    let deal_active = match &rt.engine_kind {
        dca_strategy::EngineTag::Dca(e) => !rt.stepper.position.qty.is_zero() || (e.phase != DcaPhase::Idle && e.phase != DcaPhase::Cooldown),
        dca_strategy::EngineTag::Grid(_) => rt.grid_bootstrapped,
        dca_strategy::EngineTag::Hedge(h) => {
            !rt.stepper.position.qty.is_zero() || (h.main.phase != DcaPhase::Idle && h.main.phase != DcaPhase::Cooldown)
        }
    };
    if deal_active {
        crate::state::persist_event(&app, &req.bot_id, "WARN", "WEBHOOK", "START_DEAL ignored — DEAL_ALREADY_ACTIVE").await;
        return Json(json!({"status": "DEAL_ALREADY_ACTIVE"})).into_response();
    }
    rt.webhook_start_requested = true;
    crate::state::persist_event(&app, &req.bot_id, "INFO", "WEBHOOK", "TradingView START_DEAL queued (opens at next bar evaluation)").await;
    (StatusCode::OK, Json(json!({"status": "SIGNAL_QUEUED", "botId": req.bot_id}))).into_response()
}

// ---------------- Yürütme şalterleri (Faz 0.2 kill-switch + Faz 0.3 freeze) ---
// v3 `/api/live/killswitch|freeze` karşılığı: engage → olay akışı + SSE;
// durum restart-safe (rehydrate_halts). Bırakma sonrası selfcheck yenileme UI kuralıdır.

async fn get_killswitch(State(app): State<Arc<AppState>>) -> Json<Value> {
    Json(app.kill_json().await)
}

async fn get_freeze(State(app): State<Arc<AppState>>) -> Json<Value> {
    Json(app.freeze_json().await)
}

#[derive(Deserialize, Default)]
struct HaltSetBody {
    /// kill için zorunlu (v3 sözleşmesi); freeze'de verilmezse toggle.
    #[serde(default)]
    engaged: Option<bool>,
    #[serde(default)]
    frozen: Option<bool>,
    #[serde(default)]
    reason: Option<String>,
    #[serde(default)]
    by: Option<String>,
}

fn halt_actor(by: &Option<String>) -> String {
    by.as_deref().map(|s| s.chars().take(120).collect()).unwrap_or_else(|| "UI".into())
}

async fn set_killswitch(State(app): State<Arc<AppState>>, Json(req): Json<HaltSetBody>) -> axum::response::Response {
    let Some(engaged) = req.engaged else {
        return (StatusCode::BAD_REQUEST, Json(json!({"error": "engaged (boolean) zorunlu"}))).into_response();
    };
    let now = state::now_ms();
    let actor = halt_actor(&req.by);
    if engaged {
        let reason = req.reason.map(|s| s.chars().take(300).collect::<String>()).unwrap_or_else(|| "Elle devreye alındı (UI)".into());
        *app.kill.lock().await =
            crate::state::HaltState { active: true, reason: Some(reason.clone()), by: Some(actor.clone()), at_ms: now, auto: false };
        let msg = crate::state::halt_event_message("engaged", Some(&reason), Some(&actor), now, false);
        crate::state::persist_event(&app, "SYSTEM", "WARN", crate::state::GATE_KILL, &msg).await;
        let _ = app.sse.send(crate::state::SseEvent {
            event: "KILL_SWITCH".into(),
            data: serde_json::json!({"engaged": true, "reason": reason, "by": actor}).to_string(),
        });
        // Faz 28 (P0-2): şalter devreye alınınca bekleyen borsa GİRİŞ (Add)
        // emirleri de iptal edilir — "yeni emir yok" vaadi tahtadaki emirleri
        // de kapsar. Reduce/TP/SL koruyucu emirler bilinçli olarak kalır.
        {
            // Send/kilit-notu: runtimes kilidi yalnız intent yazımı için tutulur;
            // persist_event await'leri kilit BIRAKILDIKTAN sonra owned mesajlarla
            // yapılır (fleet okumaları şalter devreye alımında bloklanmasın).
            let ledger_msgs: Vec<(String, String)> = {
                let mut rts = app.runtimes.lock().await;
                // Muhafazakâr ve doğru uygulama: her bot için kendi Add linklerini
                // kendi kuyruğuna Cancel intent olarak yaz; poll cascade'i
                // borsadan iptal eder ve defteri uzlaştırır.
                let mut msgs = Vec::new();
                for (bot_id, rt) in rts.iter_mut() {
                    let links: Vec<String> = rt
                        .live_open_orders
                        .iter()
                        .filter(|o| o.kind == dca_kernel::OrderKind::Add)
                        .map(|o| o.id.clone())
                        .collect();
                    if links.is_empty() {
                        continue;
                    }
                    rt.pending_live_intents.push(crate::state::LiveIntent::Cancel {
                        links: links.clone(),
                        venue: rt.venue.clone(),
                        category: if rt.cfg.kind.is_futures() { "linear".to_string() } else { "spot".to_string() },
                        symbol: rt.cfg.symbol.clone(),
                    });
                    msgs.push((bot_id.clone(), format!("şalter devrede — {} bekleyen GİRİŞ emri iptal kuyruğuna alındı", links.len())));
                }
                msgs
            };
            for (bot_id, msg) in ledger_msgs {
                crate::state::persist_event(&app, &bot_id, "WARN", "KILL_SWITCH", &msg).await;
            }
        }
    } else {
        *app.kill.lock().await = crate::state::HaltState::default();
        let msg = crate::state::halt_event_message("released", None, Some(&actor), now, false);
        crate::state::persist_event(&app, "SYSTEM", "INFO", crate::state::GATE_KILL, &msg).await;
        let _ = app.sse.send(crate::state::SseEvent {
            event: "KILL_SWITCH".into(),
            data: serde_json::json!({"engaged": false, "by": actor}).to_string(),
        });
    }
    Json(app.kill_json().await).into_response()
}

async fn set_freeze(State(app): State<Arc<AppState>>, Json(req): Json<HaltSetBody>) -> axum::response::Response {
    let now = state::now_ms();
    let actor = halt_actor(&req.by);
    // v3 POST gövdesiz toggle eder; açık `frozen` verilirse onu uygula.
    let target = match req.frozen.or(req.engaged) {
        Some(v) => v,
        None => !app.freeze_engaged().await,
    };
    if target {
        let reason = req.reason.map(|s| s.chars().take(300).collect::<String>()).unwrap_or_else(|| "Elle donduruldu (UI)".into());
        *app.freeze.lock().await =
            crate::state::HaltState { active: true, reason: Some(reason.clone()), by: Some(actor.clone()), at_ms: now, auto: false };
        let msg = crate::state::halt_event_message("engaged", Some(&reason), Some(&actor), now, false);
        crate::state::persist_event(&app, "SYSTEM", "WARN", crate::state::GATE_FREEZE, &msg).await;
        let _ = app.sse.send(crate::state::SseEvent {
            event: "FREEZE".into(),
            data: serde_json::json!({"frozen": true, "reason": reason, "by": actor}).to_string(),
        });
    } else {
        *app.freeze.lock().await = crate::state::HaltState::default();
        let msg = crate::state::halt_event_message("released", None, Some(&actor), now, false);
        crate::state::persist_event(&app, "SYSTEM", "INFO", crate::state::GATE_FREEZE, &msg).await;
        let _ = app.sse.send(crate::state::SseEvent {
            event: "FREEZE".into(),
            data: serde_json::json!({"frozen": false, "by": actor}).to_string(),
        });
    }
    Json(app.freeze_json().await).into_response()
}

// ---------------- Vault ----------------

#[derive(Deserialize)]
struct AddKeyReq {
    venue: String,
    label: String,
    api_key: String,
    api_secret: String,
    #[serde(default)]
    passphrase: Option<String>,
}

async fn add_key(State(app): State<Arc<AppState>>, Json(req): Json<AddKeyReq>) -> Result<Json<Value>, axum::response::Response> {
    let venue = req.venue.to_uppercase();
    let adapter: &dyn VenueAdapter = match venue.as_str() {
        "BYBIT" => &app.bybit,
        "BINANCE" => &app.binance,
        "OKX" => &app.okx,
        "PAPER" => &app.paper,
        other => return Err(err422(format!("unknown venue {other}"))),
    };
    // P4-3: OKX imza passphrase'siz çalışmaz — kasa girişinde zorunlu.
    if venue == "OKX" && req.passphrase.as_deref().map(|p| p.trim().is_empty()).unwrap_or(true) {
        return Err(err422("OKX anahtarı passphrase gerektirir — API oluştururken verilen parolayı girin"));
    }
    let creds = VenueCreds {
        key: req.api_key.clone(),
        secret: req.api_secret.clone(),
        passphrase: req.passphrase.clone(),
    };
    // Zero-Withdrawal hard gate
    let perms: KeyPermissions = match adapter.key_permissions(&creds).await {
        Ok(p) => p,
        Err(e) => return Err(err422(format!("permission check failed: {e}"))),
    };
    match validate_key_permissions(&perms) {
        WithdrawalGate::HardFail(reason) => {
            return Err((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({ "error": reason, "gate": "ZERO_WITHDRAWAL_HARD_FAIL", "http": 422 })),
            )
                .into_response())
        }
        WithdrawalGate::Ok => {}
    }
    let enc_key = app.vault.encrypt(&req.api_key).map_err(|e| err422(e))?;
    let enc_secret = app.vault.encrypt(&req.api_secret).map_err(|e| err422(e))?;
    let enc_pass = app
        .vault
        .encrypt(req.passphrase.as_deref().unwrap_or(""))
        .map_err(|e| err422(e))?;
    let id = format!("k-{}", Uuid::new_v4().simple());
    {
        let conn = app.db.conn.lock().map_err(|e| err422(e.to_string()))?;
        conn.execute(
            "INSERT INTO vault_keys(id, venue, label, api_key_enc, api_secret_enc, api_pass_enc, permissions, created_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8)",
            rusqlite::params![
                id,
                venue,
                req.label,
                enc_key,
                enc_secret,
                enc_pass,
                serde_json::to_string(&perms).map_err(|e| err422(e.to_string()))?,
                state::now_ms()
            ],
        )
        .map_err(|e| err422(e.to_string()))?;
    }
    Ok(Json(json!({
        "id": id,
        "venue": venue,
        "label": req.label,
        "permissions": perms,
        "maskedKey": mask(&req.api_key),
    })))
}

/// Mask an API key for display: first 4 + "…" + last 4; keys shorter than
/// 8 chars are fully starred (nothing leaks).
fn mask(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() < 8 {
        "*".repeat(chars.len().max(3))
    } else {
        let head: String = chars[..4].iter().collect();
        let tail: String = chars[chars.len() - 4..].iter().collect();
        format!("{head}…{tail}")
    }
}

async fn list_keys(State(app): State<Arc<AppState>>) -> Json<Value> {
    let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let mut stmt = match conn
        .prepare("SELECT id, venue, label, permissions, created_at, api_key_enc, api_pass_enc FROM vault_keys ORDER BY created_at DESC")
    {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("vault_keys prepare failed: {e}");
            return Json(json!({"keys": []}));
        }
    };
    let rows: Vec<Value> = stmt
        .query_map([], |r| {
            let perms_raw: String = r.get(3)?;
            let key_enc: String = r.get(5)?;
            // Eski satırlarda api_pass_enc kolonu migrasyonla gelir (boş = yok).
            let pass_enc: String = r.get(6).unwrap_or_default();
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, perms_raw, r.get::<_, i64>(4)?, key_enc, pass_enc))
        })
        .unwrap()
        .filter_map(|x| x.ok())
        .map(|(id, venue, label, perms_raw, created_at, key_enc, pass_enc): (String, String, String, String, i64, String, String)| {
            // permissions are persisted as a JSON string — parse into the object
            // shape the frontend contract expects; fall back to a safe default.
            let perms: Value = serde_json::from_str::<dca_exchange::KeyPermissions>(&perms_raw)
                .map(|p| {
                    json!({
                        "read_only": p.read_only,
                        "can_withdraw": p.can_withdraw,
                        "spot_trade": p.spot_trade,
                        "contract_trade": p.contract_trade,
                    })
                })
                .unwrap_or_else(|_| json!({"read_only": true, "can_withdraw": false, "spot_trade": false, "contract_trade": false}));
            let note: String = serde_json::from_str::<dca_exchange::KeyPermissions>(&perms_raw)
                .map(|p| p.note)
                .unwrap_or_default();
            let masked = app
                .vault
                .decrypt(&key_enc)
                .map(|k| mask(&k))
                .unwrap_or_else(|_| "********".into());
            // ISO-8601 UTC timestamp (frontend contract: Date.parse-able string).
            let created_iso = chrono::DateTime::from_timestamp_millis(created_at)
                .map(|d| d.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
                .unwrap_or_default();
            // P4-3: passphrase ASLA dışarı çıkmaz — yalnız var/yok bayrağı.
            let has_pass = !pass_enc.trim().is_empty();
            json!({
                "id": id,
                "label": label,
                "venue": venue,
                "maskedKey": masked,
                "permissions": perms,
                "note": note,
                "createdAt": created_iso,
                "hasPassphrase": has_pass,
            })
        })
        .collect();
    Json(json!({"keys": rows}))
}

/// SEC-05: Kasadan anahtar silinmeden önce, o anahtarla canlı çalışan
/// botlar durdurulur (yeni emir engellenir). Aksi halde runtime kopyasıyla
/// emir gönderilmeye devam edilebilir.
async fn remove_key(State(app): State<Arc<AppState>>, Path(id): Path<String>) -> Json<Value> {
    let venue: Option<String> = {
        let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
        conn.query_row("SELECT venue FROM vault_keys WHERE id=?1", [&id], |r| r.get::<_, String>(0)).ok()
    };
    let mut poll_guards: Vec<tokio::sync::OwnedMutexGuard<()>> = Vec::new();
    if let Some(venue) = venue {
        let venue_uc = venue.to_uppercase();
        let mut to_stop: Vec<String> = {
            let rts = app.runtimes.lock().await;
            rts.iter()
                .filter(|(_, rt)| rt.mode == "LIVE" && rt.venue.to_uppercase() == venue_uc)
                .map(|(id, _)| id.clone())
                .collect()
        };
        to_stop.sort();
        for bot_id in to_stop {
            // Wait for any in-flight order cascade before dropping its last
            // credential copy. Keep this guard until the DB row is deleted so
            // START cannot reload the key in the middle of removal.
            poll_guards.push(acquire_bot_poll_guard(&app, &bot_id).await);
            {
                let mut rts = app.runtimes.lock().await;
                if let Some(existing) = rts.get_mut(&bot_id) {
                    existing.status = "STOPPED".into();
                    existing.credentials = None;
                    existing.pending_engine_actions.clear();
                    existing.webhook_start_requested = false;
                }
            }
            state::persist_status(&*app, &bot_id, "STOPPED");
            // Faz 13 dilim 2: cache'i tazele (STOPPED anında görünsün).
            if let Some(rt) = app.runtimes.lock().await.get(&bot_id) {
                let snap_v = rt.snapshot_json();
                app.ui_cache.lock().await.insert(bot_id.clone(), snap_v);
            }
            state::persist_event(
                &*app,
                &bot_id,
                "WARN",
                "SEC_KEY_REMOVED",
                &format!("vault anahtarı kaldırıldı ({venue}) — bot durduruldu, yeni emir engellendi"),
            ).await;
        }
    }
    let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
    let _ = conn.execute("DELETE FROM vault_keys WHERE id=?1", [&id]);
    drop(conn);
    drop(poll_guards);
    Json(json!({"ok": true}))
}

// ---------------- SSE ----------------

async fn sse_handler(
    app: Arc<AppState>,
    rx: tokio::sync::broadcast::Receiver<SseEvent>,
    last_event_id: Option<i64>,
) -> Sse<impl futures_util::Stream<Item = Result<Event, Infallible>>> {
    // Faz 22 (P2-8 kalanı): Last-Event-ID — yeniden bağlanan istemciye DB
    // events tablosundan (autoincrement id) kaçtı olaylar best-effort replay
    // edilir (en fazla 200, LOG event olarak). Sınır dürüstçe belgelenir:
    // FILL/FUNDING gibi canlı-yayın olaylarının bir kısmı events tablosuna
    // yazılmadığından replay tam kurtarma DEĞİLDİR; istemcinin polling
    // telafisi (UIX-6 dataVer) birincil mekanizma olarak kalır. Canlı akıştaki
    // her olay `id:` etiketi taşır — istemci kaldığı yeri bildirebilir.
    let mut replay: Vec<Result<Event, Infallible>> = Vec::new();
    if let Some(since) = last_event_id.filter(|x| *x > 0) {
        // Satırlar önce owned Vec'e toplanır (guard en kısa ömürde).
        let mut rows_raw: Vec<(i64, String, String, String, String, i64)> = Vec::new();
        {
            let conn = app.db.conn.lock().unwrap_or_else(|p| p.into_inner());
            if let Ok(mut stmt) = conn
                .prepare("SELECT id, bot_id, level, gate, message, ts FROM events WHERE id > ?1 ORDER BY id ASC LIMIT 200")
            {
                if let Ok(rows) = stmt.query_map([since], |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                    ))
                }) {
                    for row in rows.flatten() {
                        rows_raw.push(row);
                    }
                }
            };
        }
        for (id, bot_id, level, gate, message, ts) in rows_raw {
            replay.push(Ok(Event::default()
                .id(id.to_string())
                .event("LOG")
                .data(
                    serde_json::json!({
                        "botId": bot_id, "level": level, "gate": gate,
                        "message": message, "ts": ts, "replayed": true
                    })
                    .to_string(),
                )));
        }
    }
    let stream = async_stream_wrapper(rx, app.clone(), last_event_id);
    let replay_stream = futures_util::stream::iter(replay);
    Sse::new(futures_util::stream::select(replay_stream, stream)).keep_alive(KeepAlive::default())
}

fn async_stream_wrapper(
    rx: tokio::sync::broadcast::Receiver<SseEvent>,
    app: Arc<AppState>,
    last_event_id: Option<i64>,
) -> impl futures_util::Stream<Item = Result<Event, Infallible>> {
    // Faz 22 (P2-8 kalanı): PING olay-sayacı yerine ZAMAN tabanlıdır —
    // sessiz dönemlerde de sabit aralıkla atar (15 sn), olay patlamasında
    // spam üretmez.
    let seq_start = last_event_id.unwrap_or(0);
    futures_util::stream::unfold(
        (rx, app, seq_start as u64, std::time::Instant::now() - std::time::Duration::from_secs(15)),
        |(mut rx, app, mut n, mut last_ping)| async move {
            loop {
                if last_ping.elapsed() >= std::time::Duration::from_secs(15) {
                    last_ping = std::time::Instant::now();
                    n += 1;
                    return Some((
                        Ok(Event::default().id(n.to_string()).event("PING").data("1")),
                        (rx, app, n, last_ping),
                    ));
                }
                n += 1;
            match rx.recv().await {
                Ok(ev) => {
                    let e = Event::default().id(n.to_string()).event(ev.event).data(ev.data);
                    return Some((Ok(e), (rx, app, n, last_ping)));
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    // Faz 21 (P2-8): Lagged artık SESSİZ değil — istemciye açık
                    // LOG olayı düşer; UI "veri tazeliği" rozetinde fark edilir
                    // ve istemci kendi polling'iyle telafi eder.
                    let msg = format!("SSE akışı {skipped} olay kaçırdı (yavaş tüketici) — polling telafi eder");
                    return Some((
                        Ok(Event::default()
                            .event("LOG")
                            .data(serde_json::json!({"level": "WARN", "message": msg, "ts": crate::state::now_ms()}).to_string())),
                        (rx, app, n, last_ping),
                    ));
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
            }
        },
    )
}

#[cfg(test)]
mod rate_limit_tests {
    use super::*;
    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{HeaderValue, Request};

    #[test]
    fn rate_limit_identity_ignores_client_supplied_forwarded_headers() {
        let peer = SocketAddr::from(([127, 0, 0, 1], 45_000));
        let mut first = Request::new(Body::empty());
        first.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.1"),
        );
        first.extensions_mut().insert(ConnectInfo(peer));

        let mut second = Request::new(Body::empty());
        second.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("198.51.100.2"),
        );
        second.extensions_mut().insert(ConnectInfo(peer));

        assert_eq!(rate_limit_key(&first), rate_limit_key(&second));
        assert_eq!(rate_limit_key(&first), "127.0.0.1");
    }

    #[test]
    fn rate_limiter_never_grows_beyond_its_bucket_limit() {
        let mut limiter = RateLimiter::default();
        for index in 0..RATE_LIMIT_MAX_BUCKETS {
            assert!(limiter.check(&format!("peer-{index}")));
        }

        assert!(!limiter.check("one-more-peer"));
        assert_eq!(limiter.buckets.len(), RATE_LIMIT_MAX_BUCKETS);
    }

    #[test]
    fn restored_order_roles_keep_reduce_semantics_and_bot_ownership() {
        use dca_kernel::OrderKind;
        assert_eq!(restored_order_semantics("b-1234abcd", "b-1234abcd-7-TPL-0"), Some(("TP", OrderKind::Reduce)));
        assert_eq!(restored_order_semantics("b-1234abcd", "b-1234abcd-7-GRID_CLOSE-4294967296"), Some(("GRID_CLOSE", OrderKind::Reduce)));
        assert_eq!(restored_order_semantics("b-1234abcd", "b-1234abcd-7-SAFETY-2"), Some(("SAFETY", OrderKind::Add)));
        assert_eq!(restored_order_semantics("b-1234abcd", "b-9876abcd-7-TPL-0"), None);
        assert_eq!(restored_order_semantics("b-1234abcd", "b-1234abcd-7-UNKNOWN-0"), None);
    }

    #[test]
    fn live_market_identity_is_case_and_whitespace_insensitive() {
        assert_eq!(
            normalized_live_market(" bybit ", " btcusdt "),
            normalized_live_market("BYBIT", "BTCUSDT")
        );
    }

    #[test]
    fn only_one_restored_active_live_bot_may_own_a_venue_symbol() {
        let duplicates = duplicate_live_markets([
            ("Bybit", "LIVE", " btcusdt", "RUNNING"),
            ("BYBIT", "live", "BTCUSDT ", "PAUSED"),
            ("OKX", "LIVE", "BTC-USDT-SWAP", "RUNNING"),
            ("BYBIT", "PAPER", "BTCUSDT", "RUNNING"),
            ("BINANCE", "LIVE", "ETHUSDT", "STOPPED"),
        ]);
        assert_eq!(duplicates, HashSet::from([("BYBIT".into(), "BTCUSDT".into())]));
    }

    #[test]
    fn stopped_live_bot_keeps_ownership_while_it_has_exposure_or_orders() {
        assert!(!live_status_requires_ownership("STOPPED", false, false, false));
        assert!(live_status_requires_ownership("STOPPED", true, false, false));
        assert!(live_status_requires_ownership("STOPPED", false, true, false));
        assert!(live_status_requires_ownership("PAUSED", false, false, false));
    }
}
