//! Bot runtime supervisor: per-bot tokio task driving the unified kernel +
//! strategy engines against live market data (paper fills) or real venue
//! orders (live mode). Emits SSE events for the frontend.

use crate::state::{AppState, BotRuntime};
use crate::state::SseEvent;
use dca_exchange::{Kline, VenueAdapter};
use dca_kernel::{Bar, FillRecord, OrderIntent, OrderKind};
use dca_risk::FillRealism;
use dca_math::fraction::Fraction;
use dca_math::m02::Direction;
use dca_math::{Instrument, Side};
use dca_signals::{BarData, SignalEngine, Timeframe};
use dca_strategy::{Action, BotConfig, StrategyKind};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::broadcast;

/// Process ONE bar through the bot runtime (stepper + engine), dispatching
/// paper fills internally. Returns SSE-worthy events.
/// H-1 adim 2: poll basina ONCE hesaplanan sinyal/pine kararidir. Eskiden her
/// bar icin yeniden ag fetch'i yapiliyordu (60 bar = 60 kez, runtimes kilidi
/// altinda); fetch'ler zaten 'en son kapali' pencereye baktigindan karar
/// poll-basina tek hesapla ayni sonucu verir — kilitsiz ve 60x daha az ag.
pub struct PreSignals {
    pub tree_ok: Option<bool>,
    pub pine_out: Option<Result<dca_pine::BarVerdict, String>>,
    pub sig_engine: Option<std::sync::Arc<dca_signals::SignalEngine>>,
    pub sig_root: Option<dca_signals::SignalNode>,
}

pub async fn process_bar(
    app: &AppState,
    rt: &mut BotRuntime,
    bar: &Bar,
    sse: &broadcast::Sender<SseEvent>,
    pre: &PreSignals,
) -> Result<(), String> {
    let bot_id = rt.bot_id.clone();
    // 1) kernel step: fill resting orders through gates (ana bacak)
    let fills = rt.stepper.step(bar, bar.t, false).map_err(|e| e.to_string())?;
    ingest_kernel_fills(app, rt, fills, bar.t, sse).await?;
    // 1b) Faz 13B: hedge bacak stepper'ı AYRI kernel olarak step edilir —
    // iki bacak tek kernel pozisyonunda birleşmez (EX-06 kalıcı çözüm).
    if rt.hedge_stepper.is_some() {
        let (hfills, hadv) = {
            let hs = rt.hedge_stepper.as_mut().unwrap();
            let hfills = hs.step(bar, bar.t, false).map_err(|e| e.to_string())?;
            let hadv: Vec<String> = hs
                .drain_new_gate_logs()
                .iter()
                .filter_map(|adv| adv.verdict.strip_prefix("ADVISORY:").map(|code| format!("RISK:{code}|{}", adv.detail)))
                .collect();
            (hfills, hadv)
        };
        ingest_kernel_fills(app, rt, hfills, bar.t, sse).await?;
        for entry in hadv {
            let (code, detail) = entry.split_once('|').unwrap_or(("RISK:?", ""));
            let code = code.trim_start_matches("RISK:");
            crate::state::persist_event(app, &bot_id, "WARN", &format!("RISK:{code}"), &format!("[hedge] {detail}")).await;
            let _ = sse.send(SseEvent {
                event: "RISK".into(),
                data: json!({"botId": bot_id, "code": code, "message": detail}).to_string(),
            });
        }
    }
    // 2.5) Futures margin guard — M07 liquidation boundary (paper/live parity)
    let liq_fired = check_liquidation(app, rt, bar, sse).await?;
    let _ = liq_fired;
    // 2.6) Futures funding settlement at UTC boundary crossings (M05 sign bridge)
    check_funding(app, rt, bar, sse).await?;
    // 2.7) TradingView webhook force-start consumption (external signal parity):
    // arms the engine so the NEXT gate evaluation bypasses start conditions.
    if rt.webhook_start_requested {
        rt.webhook_start_requested = false;
        match &mut rt.engine_kind {
            dca_strategy::EngineTag::Dca(e) => e.request_force_start(),
            dca_strategy::EngineTag::Grid(g) => g.force_start_pending = true,
            dca_strategy::EngineTag::Hedge(h) => h.main.request_force_start(),
        }
        crate::state::persist_event(app, &bot_id, "INFO", "WEBHOOK", "START_DEAL accepted — deal opens at next gate evaluation (conditions bypassed)").await;
    }
    // 2.8) Composite signal-tree verdict on CLOSED bars only (Gate 4 zero
    // look-ahead): indicator series are rebuilt from venue klines with the
    // forming bar dropped; evaluation timestamp = now (last closed bar ≤ now).
    // H-1 adim 2: ag fetch'i yok — karar kilitsiz Faz 2'de hesaplandi;
    // fail-closed yazilari orada yapildi. UI gostergesi burada guncellenir.
    // Sıfır Look-Ahead: bar işlenirken sinyal motoru varsa tam o barın t anındaki karar alınır;
    // aksi halde (veya Pine) pre sinyali fallback kullanılır.
    let resolved_tree_ok = if let (Some(eng), Some(root)) = (&pre.sig_engine, &pre.sig_root) {
        Some(eng.evaluate_at_time(root, bar.t))
    } else {
        pre.tree_ok
    };
    let tree_ok: Option<bool> = resolved_tree_ok.map(|met| {
        rt.entry_signal_met = Some(met);
        met
    });
    // 2.85) Task 19 — Pine strateji kararı (kapalı bar penceresi, zero
    // look-ahead): giriş kararı signal_ok'a AND'lenir, çıkış kararı zarif
    // kapanış üretir. Her hata fail-closed'tur (sinyal yok = giriş yok).
    // H-1 adim 2: pine karari kilitsiz Faz 2'de hesaplandi (fail-closed
    // yazilari orada); burada yalniz tuketilir.
    let pine_out: Option<Result<dca_pine::BarVerdict, String>> = pre.pine_out.clone();
    let pine_entry: Option<bool> = match &pine_out {
        None => None,
        Some(Ok(v)) => Some(v.entry),
        Some(Err(_)) => Some(false), // fail-closed
    };
    let signal_ok = match (tree_ok, pine_entry) {
        (Some(a), Some(b)) => Some(a && b),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    // Pine çıkış sinyali: DCA ailesinde zarif kapanış (request_close_all →
    // sonraki karar geçişi MANUAL piyasa emri üretir; deal muhasebesi tam).
    // Task 20 — Grid'te pine exit ARTIK mesh'i DURDURUR: resting emirler
    // idempotent iptal edilir, bootstrap kapısı yeni pine giriş sinyali bekler.
    let mut grid_halt_acts: Vec<dca_strategy::Action> = Vec::new();
    if let Some(Ok(v)) = &pine_out {
        if v.exit {
            match &mut rt.engine_kind {
                dca_strategy::EngineTag::Dca(e) => {
                    if !rt.stepper.position.qty.is_zero() {
                        e.request_close_all();
                        crate::state::persist_event(app, &bot_id, "WARN", "PINE_EXIT", "Pine stratejisi çıkış sinyali üretti — deal kapanışa alındı").await;
                        let _ = sse.send(SseEvent {
                            event: "RISK".into(),
                            data: json!({"botId": bot_id, "code": "PINE_EXIT", "message": "Pine çıkış sinyali — deal kapanıyor", "ts": bar.t}).to_string(),
                        });
                    }
                }
                dca_strategy::EngineTag::Hedge(h) => {
                    if !rt.stepper.position.qty.is_zero() {
                        h.main.request_close_all();
                        crate::state::persist_event(app, &bot_id, "WARN", "PINE_EXIT", "Pine çıkış sinyali — ana bacak kapanışa alındı").await;
                    }
                }
                dca_strategy::EngineTag::Grid(g) => {
                    if g.gate_open {
                        grid_halt_acts = g.halt_mesh();
                    }
                }
            }
        }
    }
    if !grid_halt_acts.is_empty() {
        rt.grid_bootstrapped = false;
        crate::state::persist_event(app, &bot_id, "WARN", "PINE_EXIT", "Pine çıkış sinyali — grid mesh durduruldu (resting emirler iptal edildi, yeni giriş sinyalinde yeniden kurulur)").await;
        let _ = sse.send(SseEvent {
            event: "RISK".into(),
            data: json!({"botId": bot_id, "code": "PINE_EXIT", "message": "Pine çıkış sinyali — grid mesh durduruldu", "ts": bar.t}).to_string(),
        });
        for a in grid_halt_acts {
            dispatch_action(app, rt, a, bar.t, sse).await?;
        }
    }
    // 3) engine decision pass at bar close
    let o = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
    let h = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
    let l = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
    let c = Fraction::from_decimal_str(&bar.c).map_err(|e| e.to_string())?;
    // Task 16 — rejim sınıflandırma (kapalı bar kapanışları, zero look-ahead)
    rt.regime_closes.push(c.to_f64_display());
    if rt.regime_closes.len() > 200 {
        rt.regime_closes.remove(0);
    }
    let regime = dca_signals::classify_regime(&rt.regime_closes);
    rt.last_regime = regime;
    // Task 16 — günlük DD devre kesici (spec 2.9): UTC gün çapası + eşik.
    if rt.cfg.risk_policy.daily_dd_breaker_active() {
        let day = bar.t / 86_400_000;
        if day != rt.breaker_day {
            rt.breaker_day = day;
            rt.breaker_day_start_eq = None;
            rt.breaker_tripped = false;
        }
        if !rt.breaker_tripped {
            let eq_now = rt.stepper.equity(&c).unwrap_or_else(|_| Fraction::zero());
            let anchor = rt.breaker_day_start_eq.get_or_insert_with(|| eq_now.clone());
            if !anchor.is_zero() {
                if let Ok(dd) = anchor.sub(&eq_now).and_then(|x| x.div(anchor)) {
                    // Task 18 N-1: eşik yüzde string'idir ("2" = %2) — dd fraksiyonla
                    // (0.02) karşılaştırılır, bu yüzden /100 şart. (Önceki kod ham
                    // yüzdeyi fraksiyonla kıyaslıyordu → kesici asla tetiklenmezdi.)
                    let th = dca_risk::daily_dd_breaker_threshold(&rt.cfg.risk_policy.daily_dd_breaker_pct);
                    if !th.is_zero() && dd >= th {
                        rt.breaker_tripped = true;
                        let msg = format!(
                            "Günlük DD devre kesici tetiklendi: gün içi düşüş {} ≥ eşik {}% — bugün yeni deal açılışı durduruldu (mevcut pozisyonun TP/SL yönetimi sürüyor)",
                            dd.mul(&Fraction::from_u64(100)).map(|x| x.to_half_even_display(2)).unwrap_or_else(|_| "?".into()),
                            rt.cfg.risk_policy.daily_dd_breaker_pct
                        );
                        crate::state::persist_event(app, &bot_id, "WARN", "RISK:DAILY_DD_BREAKER", &msg).await;
                        let _ = sse.send(SseEvent {
                            event: "RISK".into(),
                            data: json!({"botId": bot_id, "code": "DAILY_DD_BREAKER", "message": msg, "ts": bar.t}).to_string(),
                        });
                    }
                }
            }
        }
    } else {
        rt.breaker_tripped = false;
    }
    let deal_open_blocked = rt.breaker_tripped;
    let ctx = dca_strategy::BarContext {
        bar,
        open: o.clone(),
        high: h.clone(),
        low: l.clone(),
        close: c.clone(),
        position_qty: &rt.stepper.position.qty,
        position_avg: rt.stepper.position.avg_price.as_ref(),
        cash: &rt.stepper.cash,
        mark: c.clone(),
        signal_ok,
        regime: Some(regime),
        deal_open_blocked,
    };
    let acts = rt.engine_kind.on_bar(&ctx);
    for a in acts {
        dispatch_action(app, rt, a, bar.t, sse).await?;
    }
    // Faz 20 (UIX-5): REST handler kaynaklı eylemler (grid CLOSE_ALL →
    // halt_mesh + market kapanış) — motor döngüsüyle aynı dispatch disiplini.
    let pending_acts = std::mem::take(&mut rt.pending_engine_actions);
    for a in pending_acts {
        dispatch_action(app, rt, a, bar.t, sse).await?;
    }
    // 3.5) Grid initial placement — gated by the bot's start condition.
    // (live/paper parity with backtest bootstrap; evaluates on every new bar
    // until the condition passes, then mesh orders are placed exactly once)
    if !rt.grid_bootstrapped {
        let bootstrap_acts: Vec<dca_strategy::Action> = match &mut rt.engine_kind {
            dca_strategy::EngineTag::Grid(g) => {
                // Force path (webhook) bypasses both the simple trigger and the
                // composite signal tree — 3Commas external-signal parity.
                let forced = std::mem::take(&mut g.force_start_pending);
                let trig_ok = dca_strategy::evaluate_entry_trigger(&g.cfg.entry_trigger, &c, bar.t, None).unwrap_or(false);
                let sig_ok = signal_ok.unwrap_or(true);
                if forced || (trig_ok && sig_ok) {
                    if forced {
                        crate::state::persist_event(app, &bot_id, "INFO", "WEBHOOK", "grid bootstrap forced by TradingView START_DEAL").await;
                    }
                    g.bootstrap(&c)
                } else {
                    Vec::new()
                }
            }
            _ => {
                rt.grid_bootstrapped = true;
                Vec::new()
            }
        };
        if !bootstrap_acts.is_empty() {
            rt.grid_bootstrapped = true;
            let n = bootstrap_acts.len();
            for a in bootstrap_acts {
                dispatch_action(app, rt, a, bar.t, sse).await?;
            }
            crate::state::persist_event(
                app,
                &bot_id,
                "INFO",
                "START",
                &format!("start condition met — grid bootstrap placed {n} mesh orders"),
            )
            .await;
        }
    }
    // 4) equity tracking
    let eq = rt.stepper.equity(&c).map_err(|e| e.to_string())?;
    persist_equity(app, &bot_id, bar.t, &eq).await?;
    let _ = sse.send(SseEvent {
        event: "EQUITY".into(),
        data: json!({
            "botId": bot_id,
            "ts": bar.t,
            "equity": eq.to_string(),
            "positionQty": rt.stepper.position.qty.to_string(),
            "avgPrice": rt.stepper.position.avg_price.as_ref().map(|a| a.to_string()),
            "realizedGross": rt.stepper.position.realized_gross.to_string(),
            "cash": rt.stepper.cash.to_string(),
            "imUsed": rt.stepper.im_used.to_string(),
            "marginMode": !rt.stepper.cfg.is_spot,
            "liqPrice": current_liq_price(rt).map(|p| p.to_string()),
        })
        .to_string(),
    });
    Ok(())
}

/// Per-bar funding settlement for futures bots (linear perpetuals).
/// Funding settlement — P1-6 (Faz 22): mutabakat artık GERÇEK settlement
/// geçmişiyle. `funding_history` kayıtlarından bar penceresine (prev_ts, bar.t]
/// düşen her settlement AYRI uygulanır; interval kayıt aralığından çıkarılır
/// (venue'daki fundingIntervalHour'un fiili karşılığı). Eski "sabit 8h × anlık
/// ticker rate" yaklaşımı iki hata yapıyordu: (a) sembol 1h/2h/4h funding
/// yapabilir, (b) anlık rate geçmiş settlement'i temsil etmez. Ticker tabanlı
/// yol yalnız geçmiş BOŞSA fallback'tir ve hata yolları SESSİZ DEĞİLDİR
/// (görünür WARN olayı — sessiz `Ok(())` yok).
async fn check_funding(
    app: &AppState,
    rt: &mut BotRuntime,
    bar: &Bar,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    const FALLBACK_INTERVAL_MS: i64 = 8 * 3600 * 1000;
    const HISTORY_TTL_MS: i64 = 6 * 3600 * 1000; // başarılı fetch tazeliği
    const RETRY_TTL_MS: i64 = 10 * 60 * 1000; // başarısız fetch yeniden deneme aralığı
    if rt.stepper.cfg.is_spot {
        return Ok(());
    }
    let main_open = !rt.stepper.position.qty.is_zero();
    let hedge_open = rt
        .hedge_stepper
        .as_ref()
        .map(|h| !h.position.qty.is_zero())
        .unwrap_or(false);
    if !main_open && !hedge_open {
        return Ok(());
    }
    let category = if rt.cfg.kind.is_futures() { "linear" } else { "spot" };
    let Some(adapter) = app.adapter(&rt.venue) else {
        crate::state::persist_event(app, &rt.bot_id, "WARN", "FUNDING", "venue adapter yok — funding mutabakatı atlandı").await;
        return Ok(());
    };

    // Geçmiş cache: başarılı fetch HISTORY_TTL taze; başarısızlık RETRY_TTL
    // sonra yeniden denenir (her bar isteği spam olmasın).
    let now_ms = crate::state::now_ms();
    let stale = now_ms - rt.funding_history_ok_at > HISTORY_TTL_MS;
    let retry_due = now_ms - rt.funding_history_tried_at > RETRY_TTL_MS;
    if rt.funding_history.is_empty() || (stale && retry_due) {
        rt.funding_history_tried_at = now_ms;
        match adapter.funding_history(&rt.cfg.symbol, 200).await {
            Ok(mut v) => {
                v.sort_by_key(|(t, _)| *t);
                let n = v.len();
                rt.funding_history = v;
                rt.funding_history_ok_at = now_ms;
                if n == 0 {
                    crate::state::persist_event(app, &rt.bot_id, "WARN", "FUNDING", "venue funding geçmişi boş döndü — settlement'lar fallback (anlık rate) ile karşılanacak").await;
                }
            }
            Err(e) => {
                crate::state::persist_event(
                    app,
                    &rt.bot_id,
                    "WARN",
                    "FUNDING",
                    &format!("funding geçmişi alınamadı: {e} — son bilinen kayıtlarla devam (10 dk sonra yeniden denenecek)"),
                )
                .await;
            }
        }
    }

    // Interval: geçmişteki ardışık settlement aralıklarının medyanı; yetersiz
    // kayıt varsa 8h fallback (eski davranış).
    let interval_ms = {
        let hs = &rt.funding_history;
        if hs.len() >= 3 {
            let mut diffs: Vec<i64> = hs.windows(2).map(|w| w[1].0 - w[0].0).filter(|d| *d > 0).collect();
            if !diffs.is_empty() {
                diffs.sort_unstable();
                diffs[diffs.len() / 2]
            } else {
                FALLBACK_INTERVAL_MS
            }
        } else {
            FALLBACK_INTERVAL_MS
        }
    };

    let prev_ts = rt.last_bar_ts.max(0);
    let window: Vec<(i64, String)> = rt
        .funding_history
        .iter()
        .filter(|(t, _)| *t > prev_ts && *t <= bar.t)
        .cloned()
        .collect();

    // Gerçek kayıt yoksa: boundary gerçekten geçilmedi olabilir (en yaygın)
    // YA DA geçmiş cache'i pencereyi kapsamıyor. Boundary sayısıyla ayırt et;
    // geçildiyse fallback anlık-rate yoluna in — hataları SESSİZ geçme.
    if window.is_empty() {
        let crossings = if interval_ms > 0 { bar.t / interval_ms - prev_ts / interval_ms } else { 0 };
        if crossings <= 0 {
            return Ok(());
        }
        let Ok(ticker) = adapter.ticker(category, &rt.cfg.symbol).await else {
            crate::state::persist_event(app, &rt.bot_id, "WARN", "FUNDING", &format!("funding boundary geçildi ({crossings}x) ama ticker alınamadı — settlement ertelendi (P1-6: sessiz yutma yok)")).await;
            return Ok(());
        };
        let Some(rate) = ticker.funding_rate else {
            crate::state::persist_event(app, &rt.bot_id, "WARN", "FUNDING", &format!("funding boundary geçildi ({crossings}x) ama ticker'da rate yok — settlement ertelendi")).await;
            return Ok(());
        };
        if !rate.is_zero() {
            let close = Fraction::from_decimal_str(&bar.c).map_err(|e| e.to_string())?;
            let crossings_f = Fraction::from_i64(crossings);
            let scale = if crossings > 1 { Some(&crossings_f) } else { None };
            apply_funding_settlement(app, rt, bar, &rate, &close, scale, "fallback(anlık-rate)", sse).await?;
        }
        return Ok(());
    }

    // GERÇEK mutabakat: penceredeki her settlement kaydı ayrı uygulanır.
    let close = Fraction::from_decimal_str(&bar.c).map_err(|e| e.to_string())?;
    for (t, rate_str) in &window {
        let Ok(rate) = Fraction::from_any_str(rate_str) else {
            crate::state::persist_event(app, &rt.bot_id, "WARN", "FUNDING", &format!("funding kaydı çözülemedi (ts={t}, rate='{rate_str}') — atlandı")).await;
            continue;
        };
        if rate.is_zero() {
            continue;
        }
        apply_funding_settlement(app, rt, bar, &rate, &close, None, "history", sse).await?;
    }
    Ok(())
}

/// Tek settlement'ın uygulanması — ana + hedge bacak, SSE FUNDING + olay kaydı.
#[allow(clippy::too_many_arguments)]
async fn apply_funding_settlement(
    app: &AppState,
    rt: &mut BotRuntime,
    bar: &Bar,
    rate: &Fraction,
    close: &Fraction,
    scale: Option<&Fraction>,
    source: &str,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    let scale_f = |x: &Fraction| -> Fraction {
        match scale {
            Some(s) => x.mul(s).unwrap_or_else(|_| x.clone()),
            None => x.clone(),
        }
    };
    if !rt.stepper.position.qty.is_zero() {
        let notional = rt.stepper.position.qty.mul(close).map_err(|e| e.to_string())?;
        let dir = Fraction::from_i64(rt.stepper.position.direction as i64);
        let expense = scale_f(&rate.mul(&notional).map_err(|e| e.to_string())?.mul(&dir).map_err(|e| e.to_string())?);
        rt.stepper.apply_funding_expense(&expense);
        match &mut rt.engine_kind {
            dca_strategy::EngineTag::Dca(e) => e.apply_funding(&expense),
            dca_strategy::EngineTag::Hedge(h) => h.main.apply_funding(&expense),
            dca_strategy::EngineTag::Grid(_) => {}
        }
        let bot_id = rt.bot_id.clone();
        let _ = sse.send(SseEvent {
            event: "FUNDING".into(),
            data: json!({
                "botId": bot_id,
                "ts": bar.t,
                "rate": rate.to_string(),
                "expense": expense.to_string(),
                "cash": rt.stepper.cash.to_string(),
                "cumulativeFunding": rt.stepper.cumulative_funding.to_string(),
                "source": source,
            })
            .to_string(),
        });
        crate::state::persist_event(
            app,
            &bot_id,
            "INFO",
            "FUNDING",
            &format!(
                "funding settled [{source}]: rate={} expense={} qty={}",
                rate.to_half_even_display(8),
                expense.to_half_even_display(8),
                rt.stepper.position.qty.to_half_even_display(8)
            ),
        )
        .await;
    }
    // Faz 13B: hedge bacağı AYRI kernel — kendi pozisyonuyla ayrı settlement.
    if let Some(hs) = rt.hedge_stepper.as_mut() {
        if !hs.position.qty.is_zero() {
            let hnotional = hs.position.qty.mul(close).unwrap_or_else(|_| Fraction::zero());
            let hdir = Fraction::from_i64(hs.position.direction as i64);
            if let Ok(hexp) = rate
                .mul(&hnotional)
                .and_then(|x| x.mul(&hdir))
                .and_then(|x| Ok(scale_f(&x)))
            {
                hs.apply_funding_expense(&hexp);
                if let dca_strategy::EngineTag::Hedge(h) = &mut rt.engine_kind {
                    if let Some(hleg) = h.hedge.as_mut() {
                        hleg.apply_funding(&hexp);
                    }
                }
            }
        }
    }
    Ok(())
}

/// M07 liquidation price for the current position (margin bots only).
/// ISOLATED semantics: only the committed position margin (im_used) backs the
/// position — the bot's free reserve (cash) is for safety orders, not collateral.
pub fn current_liq_price(rt: &BotRuntime) -> Option<Fraction> {
    liq_price_of_stepper(&rt.stepper, rt.mmr_brackets.as_ref())
}

/// Faz 13B: stepper-bağımsız likidasyon fiyatı — her bacak kendi M07 sınırını
/// kendi pozisyonuyla hesaplar.
pub fn liq_price_of_stepper(
    st: &dca_kernel::SimulationStepper,
    brackets: Option<&Vec<dca_math::MmrBracket>>,
) -> Option<Fraction> {
    if st.cfg.is_spot || st.position.qty.is_zero() {
        return None;
    }
    let avg = st.position.avg_price.as_ref()?;
    let qty = st.position.qty.clone();
    let brackets = match brackets {
        Some(b) if !b.is_empty() => b.clone(),
        _ => vec![dca_math::MmrBracket {
            min_notional: Fraction::zero(),
            max_notional: Fraction::from_u64(u64::MAX),
            mmr: Fraction::from_decimal_str("0.005").unwrap_or_else(|_| Fraction::zero()),
            deduction: Fraction::zero(),
        }],
    };
    dca_math::m03_m07::liquidation_price(
        st.position.direction,
        &qty,
        avg,
        &st.im_used,
        &brackets,
        &Fraction::zero(),
    )
    .ok()
    .flatten()
}

/// Per-bar liquidation guard for margin bots (FUTURES_DCA / leveraged DCA_SHORT):
/// when the bar's extreme crosses the M07 boundary, force-close at the worse of
/// open/liq (gap-through realism), cancel resting orders, mark the deal liquidated.
async fn check_liquidation(
    app: &AppState,
    rt: &mut BotRuntime,
    bar: &Bar,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    if rt.stepper.cfg.is_spot || rt.stepper.position.qty.is_zero() {
        return Ok(());
    }
    // Faz 4 (H-5): LIVE'da yerel sentez-LIQ kapanışı YOKTUR — tasfiyeyi venue
    // gerçekliğinden öğreniriz (poll_inner'daki venue-flat kontrolü). Yerel
    // hesap yalnız PAPER simülasyonunda kapanış üretir; LIVE'da liqPrice
    // gösterge olarak yaşamaya devam eder (SSE equity kartı).
    if rt.mode == "LIVE" {
        return Ok(());
    }
    // Faz 13B: hedge bacağı AYRI kernel — kendi likidasyon sınırıyla izlenir;
    // tetiklenirse YALNIZ hedge bacağı kapanır (ana bacak etkilenmez).
    if let Some(hs) = rt.hedge_stepper.as_mut() {
        if !hs.cfg.is_spot && hs.cfg.leverage > Fraction::one() {
            if let Some(hliq) = liq_price_of_stepper(hs, rt.mmr_brackets.as_ref()) {
                let hlow = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
                let hhigh = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
                let hopen = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
                let hdir = hs.position.direction;
                let hcrossed = if hdir == 1 { hlow <= hliq } else { hhigh >= hliq };
                if hcrossed {
                    let hliq_px = if hdir == 1 { Fraction::min(&hopen, &hliq) } else { Fraction::max(&hopen, &hliq) };
                    hs.cancel_pending();
                    if let Some(hfill) = hs.force_close(&hliq_px, bar.t, "LIQ").map_err(|e| e.to_string())? {
                        let _ = persist_fill(app, &rt.bot_id, &hfill).await;
                        let _ = sse.send(SseEvent {
                            event: "LIQUIDATION".into(),
                            data: json!({"botId": rt.bot_id, "leg": "hedge", "liqPrice": hliq_px.to_string(), "qty": hfill.qty.to_string(), "ts": bar.t}).to_string(),
                        });
                        crate::state::persist_event(app, &rt.bot_id, "CRITICAL", "LIQUIDATION", "hedge bacağı likide oldu (yerel simülasyon)").await;
                        if let dca_strategy::EngineTag::Hedge(h) = &mut rt.engine_kind {
                            h.hedge.as_mut().map(|l| l.force_liquidate());
                        }
                    }
                }
            }
        }
    }
    if rt.stepper.cfg.leverage <= Fraction::one() {
        return Ok(());
    }
    let Some(liq) = current_liq_price(rt) else {
        return Ok(());
    };
    let low = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
    let high = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
    let open = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
    let dir = rt.stepper.position.direction;
    let crossed = if dir == 1 { low <= liq } else { high >= liq };
    if !crossed {
        return Ok(());
    }
    let liq_px = if dir == 1 { Fraction::min(&open, &liq) } else { Fraction::max(&open, &liq) };
    let cancelled = rt.stepper.cancel_pending();
    let bot_id = rt.bot_id.clone();
    if let Some(fill) = rt.stepper.force_close(&liq_px, bar.t, "LIQ").map_err(|e| e.to_string())? {
        let mut acts = rt.engine_kind.on_fill(&fill);
        persist_fill(app, &bot_id, &fill).await?;
        let _ = sse.send(SseEvent {
            event: "FILL".into(),
            data: json!({
                "botId": bot_id,
                "fillKey": fill.fill_key,
                "orderId": fill.order_id,
                "role": fill.role,
                "side": format!("{:?}", fill.side),
                "kind": format!("{:?}", fill.kind),
                "price": fill.price.to_string(),
                "qty": fill.qty.to_string(),
                "fee": fill.fee.to_string(),
                "feeRole": format!("{}", fill.fee_role),
                "fillModel": format!("{:?}", fill.realism),
                "ts": fill.ts,
            })
            .to_string(),
        });
        let _ = sse.send(SseEvent {
            event: "LIQUIDATION".into(),
            data: json!({
                "botId": bot_id,
                "liqPrice": liq_px.to_string(),
                "qty": fill.qty.to_string(),
                "cancelledOrders": cancelled,
                "ts": bar.t,
            })
            .to_string(),
        });
        crate::state::persist_event(
            app,
            &bot_id,
            "CRITICAL",
            "LIQUIDATION",
            &format!("position force-closed at {}", liq_px.to_half_even_display(8)),
        )
        .await;
        if let dca_strategy::EngineTag::Dca(e) = &mut rt.engine_kind {
            // Faz 19 (P1-4): likidasyonda dinlenen TP emri borsadan da çekilir.
            if let Some(rt_tp) = e.resting_tp.take() {
                acts.push(dca_strategy::Action::CancelResting { id: rt_tp.order_id });
            }
            e.force_liquidate();
        }
        if let dca_strategy::EngineTag::Hedge(h) = &mut rt.engine_kind {
            // Faz 13B: ANA bacak likide oldu — yalnız ana bacak force_liquidate;
            // hedge bacağı kendi stepper'ıyla izlenmeye devam eder.
            if let Some(rt_tp) = h.main.resting_tp.take() {
                acts.push(dca_strategy::Action::CancelResting { id: rt_tp.order_id });
            }
            h.main.force_liquidate();
        }
        for a in acts {
            dispatch_action(app, rt, a, bar.t, sse).await?;
        }
    }
    Ok(())
}

/// Faz 13A-POLL: intent'leri KİLİTSİZ çalıştırır (gate wallet + place + market
/// 3× dolum sorgusu + cancel) ve sonuçları döndürür. Uygulama (muhasebe +
/// cascade) apply_intent_outcomes'ta kısa kilit altında yapılır.
async fn execute_live_intents(
    app: &Arc<AppState>,
    creds: Option<&dca_exchange::VenueCreds>,
    intents: Vec<crate::state::LiveIntent>,
) -> Vec<crate::state::IntentOutcome> {
    let mut out = Vec::new();
    let Some(creds) = creds else {
        for i in &intents {
            if let crate::state::LiveIntent::Place { id, .. } = i {
                out.push(crate::state::IntentOutcome::PlaceFailed { id: id.clone(), err: "live mode requires vault credentials".into() });
            }
        }
        return out;
    };
    for intent in intents {
        match intent {
            crate::state::LiveIntent::Place { req, id, side, kind, price, qty, role, is_close, gate } => {
                let Some(adapter) = app.adapter(&gate.venue) else {
                    out.push(crate::state::IntentOutcome::PlaceFailed { id, err: format!("no adapter for {}", gate.venue) });
                    continue;
                };
                // C-6/M-14 kapı — reduce (kapanış) muaftır; wallet ağı kilitsiz.
                if !is_close && kind != OrderKind::Reduce {
                    if let Err(e) = pre_order_gate_ctx(app, &gate, adapter, creds, price.as_ref(), &qty).await {
                        out.push(crate::state::IntentOutcome::GateBlocked { id, reason: e });
                        continue;
                    }
                }
                match adapter.place_order(creds, &req).await {
                    Ok(_) => {
                        if req.is_market {
                            // Faz 18 (P0-4/P0-5): market dolum yakalama TEK YOL —
                            // executions(): dolum kimliği borsa execId'sidir (yerel
                            // now_ms anahtarı retry/çift cascade turunda çift sayıma
                            // açıktı), her execution AYRI MarketFilled taşır (kısmi
                            // dolum taneli muhasebe). Kalan miktar MarketOpen ile
                            // kitapta tutulur; sweep order-history ile tamamlar —
                            // "st.price None → sessiz kayıp" deliği kapanmıştır.
                            let mut captured = Fraction::zero();
                            let mut exec_err: Option<String> = None;
                            for attempt in 0..3u32 {
                                match adapter.executions(creds, &req.category, &req.symbol, &id).await {
                                    Ok(v) => {
                                        for ex in v.into_iter().filter(|e| !e.exec_qty.is_zero()) {
                                            captured = captured.add(&ex.exec_qty).unwrap_or_else(|_| captured.clone());
                                            out.push(crate::state::IntentOutcome::MarketFilled {
                                                id: id.clone(), side, kind: kind.clone(), role: role.clone(),
                                                qty_filled: ex.exec_qty, price: ex.exec_price, fee: ex.exec_fee,
                                                fill_key: format!("{}-{}", ex.order_link_id, ex.exec_id),
                                            });
                                        }
                                        exec_err = None;
                                        break;
                                    }
                                    Err(e) => {
                                        exec_err = Some(e);
                                        if attempt < 2 {
                                            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                                        }
                                    }
                                }
                            }
                            // Kısmi/hiç dolum: kalan kitaba girer (sweep tamamlar).
                            // Sorgu tamamen başarısızsa bile emir KİTAPTA kalır —
                            // görünmez dolum artık imkansız (fail-closed).
                            // Faz 21 (N-4): kalan GERÇEK referans fiyatla yazılır
                            // (gate.last_close) — 0 fiyatlı kitap kaydı venue yük
                            // defterini eksikletiyor ve sweep fallback'i m04
                            // DOMAIN hatasıyla yutuyordu.
                            let remaining = qty.sub(&captured).unwrap_or_else(|_| Fraction::zero());
                            if let Some(e) = exec_err {
                                out.push(crate::state::IntentOutcome::MarketQueryFailed { id: id.clone(), err: e });
                            }
                            if !remaining.is_zero() && !remaining.is_negative() {
                                let ref_px = gate.last_close.clone().unwrap_or_else(Fraction::zero);
                                out.push(crate::state::IntentOutcome::MarketOpen {
                                    id, side, kind, role,
                                    price: ref_px,
                                    qty: remaining,
                                });
                            }
                        } else {
                            out.push(crate::state::IntentOutcome::Placed { id, side, kind, price, qty, role });
                        }
                    }
                    Err(e) => out.push(crate::state::IntentOutcome::PlaceFailed { id, err: e }),
                }
            }
            crate::state::LiveIntent::Cancel { links, venue, category, symbol } => {
                let mut cancelled = Vec::new();
                let mut failed = Vec::new();
                let Some(adapter) = app.adapter(&venue) else {
                    out.push(crate::state::IntentOutcome::Cancelled { cancelled, failed: links });
                    continue;
                };
                for link in links {
                    let resolved: Vec<String> = if link.ends_with('*') {
                        match adapter.open_orders(creds, &category, &symbol).await {
                            Ok(list) => list
                                .into_iter()
                                .map(|o| o.order_link_id)
                                .filter(|l| l.starts_with(link.trim_end_matches('*')))
                                .collect(),
                            Err(_) => Vec::new(),
                        }
                    } else {
                        vec![link.clone()]
                    };
                    for r in resolved {
                        match adapter.cancel_order(creds, &category, &symbol, &r).await {
                            Ok(()) => cancelled.push(r),
                            // 110001 "already gone" — idempotent başarı (mevcut sözleşme)
                            Err(e) if e.contains("110001") || e.to_lowercase().contains("order not exists") => cancelled.push(r),
                            Err(_) => failed.push(r),
                        }
                    }
                }
                out.push(crate::state::IntentOutcome::Cancelled { cancelled, failed });
            }
        }
    }
    out
}

/// Faz 13A-POLL: sonuçları kilit altında uygular — muhasebe + cascade.
async fn apply_intent_outcomes(
    app: &Arc<AppState>,
    rt: &mut BotRuntime,
    outcomes: Vec<crate::state::IntentOutcome>,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    for o in outcomes {
        match o {
            crate::state::IntentOutcome::Placed { id, side, kind, price, qty, role } => {
                // Faz 4.5 (C-6): resting emir yükü deftere (+) — poll-refresh arası pencere kapalı.
                if let (Some(px), Some(q)) = (price.clone(), Some(&qty)) {
                    let n = q.mul(&px).unwrap_or_else(|_| Fraction::zero());
                    let cost = if !rt.stepper.cfg.is_spot {
                        let lev = if rt.stepper.cfg.leverage.is_zero() { Fraction::one() } else { rt.stepper.cfg.leverage.clone() };
                        n.div(&lev).unwrap_or(n)
                    } else {
                        n
                    };
                    crate::state::bump_venue_live_load(app, &rt.venue, &cost, true).await;
                }
                crate::state::upsert_live_order(
                    app,
                    &rt.bot_id,
                    &id,
                    if side == dca_math::Side::Sell { "Sell" } else { "Buy" },
                    if kind == OrderKind::Reduce { "Reduce" } else { "Add" },
                    &price.clone().unwrap_or_else(Fraction::zero).to_string(),
                    &qty.to_string(),
                    &role,
                    "PENDING",
                );
                rt.live_open_orders.push(crate::state::LiveTrackedOrder {
                    id,
                    side,
                    kind,
                    price: price.unwrap_or_else(Fraction::zero),
                    qty,
                    role,
                });
            }
            crate::state::IntentOutcome::MarketFilled { id, side, kind, role, qty_filled, price, fee, fill_key } => {
                // Faz 18 (P0-4): anahtar borsa execId'si — retry/çift cascade
                // turunda aynı dolum iki kez muhasebeleştirilemez.
                let ts2 = crate::state::now_ms();
                if fill_exists(app, &rt.bot_id, &fill_key).await.unwrap_or(false) {
                    continue;
                }
                // Faz 13B: dolum, order-id prefix'ine göre DOĞRU bacak kernel'ine.
                let hedge_prefix_mf = format!("{}-hedge", rt.bot_id);
                let mf_target = if id.starts_with(&hedge_prefix_mf) {
                    rt.hedge_stepper.as_mut()
                } else {
                    Some(&mut rt.stepper)
                };
                let ingest = mf_target
                    .ok_or("hedge stepper yok")?
                    .ingest_external_fill_with_fee(
                        &id,
                        &role,
                        side,
                        kind,
                        &qty_filled,
                        &price,
                        dca_kernel::FeeRole::Taker,
                        FillRealism::MarketableTaker,
                        ts2,
                        fee.as_ref(),
                        // Faz 23 (derin inceleme): execId anahtarı KERNEL'e kadar
                        // taşınır — eski kod anahtarı yalnız ön-kontrolde kullanıp
                        // kernel'in {order_id}-{ts} anahtarına düşürüyordu; sweep
                        // yeniden sorgulayınca aynı dolum iki kez sayılıyordu.
                        Some(&fill_key),
                    );
                match ingest {
                    Ok(maybe_f) => {
                        if let Some(f) = maybe_f {
                            let _ = persist_fill(app, &rt.bot_id, &f).await;
                            let acts = rt.engine_kind.on_fill(&f);
                            let _ = sse.send(SseEvent {
                                event: "FILL".into(),
                                data: json!({
                                    "botId": rt.bot_id, "fillKey": f.fill_key, "orderId": f.order_id,
                                    "role": f.role, "side": format!("{:?}", f.side), "kind": format!("{:?}", f.kind),
                                    "price": f.price.to_string(), "qty": f.qty.to_string(), "fee": f.fee.to_string(),
                                    "feeRole": format!("{}", f.fee_role), "fillModel": format!("{:?}", f.realism), "ts": f.ts,
                                })
                                .to_string(),
                            });
                            for a in acts {
                                Box::pin(dispatch_action(app, rt, a, ts2, sse)).await?;
                            }
                        }
                        // Faz 18 (P0-5): bu dolum emrin TÜM kitap miktarını
                        // karşılamıyorsa emir kitapta KALIR — kalan miktar
                        // sweep'te tamamlanır (kısmi dolum takibi).
                        let book = rt.live_open_orders.iter().find(|o| o.id == id);
                        let covers_all = book.map(|o| qty_filled >= o.qty).unwrap_or(true);
                        let cost = book.map(|o| {
                            let n = o.qty.mul(&o.price).unwrap_or_else(|_| Fraction::zero());
                            if o.kind == OrderKind::Add && !rt.stepper.cfg.is_spot {
                                let lev = if rt.stepper.cfg.leverage.is_zero() { Fraction::one() } else { rt.stepper.cfg.leverage.clone() };
                                n.div(&lev).unwrap_or(n)
                            } else {
                                n
                            }
                        });
                        if covers_all {
                            rt.live_open_orders.retain(|o| o.id != id);
                            crate::state::remove_live_order(app, &rt.bot_id, &id);
                            if let Some(c) = cost {
                                if !c.is_zero() {
                                    crate::state::bump_venue_live_load(app, &rt.venue, &c, false).await;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        // Faz 18 (P0-4): yutulan dolum yok — CRITICAL olay yazılır,
                        // emir kitapta tutulur; sweep aynı execId anahtarıyla
                        // yeniden dener (kitap = kalıcı pending-fill kuyruğu).
                        let msg = format!(
                            "LIVE dolum işlenemedi ({id} / {fill_key}): {e} — emir kitapta tutuldu, sweep yeniden deneyecek"
                        );
                        crate::state::persist_event(app, &rt.bot_id, "CRITICAL", "FILL_INGEST", &msg).await;
                        let _ = sse.send(SseEvent {
                            event: "RISK".into(),
                            data: json!({"botId": rt.bot_id, "code": "FILL_INGEST_FAILED", "message": msg, "ts": ts2}).to_string(),
                        });
                    }
                }
            }
            crate::state::IntentOutcome::MarketOpen { id, side, kind, role, price, qty } => {
                // Faz 22 (N-14): 0 referans fiyatı patolojik — venue yük defterini
                // eksik sayıyor ve sweep fallback'i 0 fiyatla ingest edemez.
                // Kitaba girmeden önce son kapanış fiyatından türet; hâlâ yoksa
                // CRITICAL yazıp kitaba 0 fiyatla KOYMA (fail-closed; sweep
                // bundan sonra fiyat bulursa kaydı açar).
                let mut price = price;
                if price.is_zero() {
                    let derived = rt
                        .stepper
                        .last_close
                        .as_deref()
                        .and_then(|s| Fraction::from_any_str(s).ok())
                        .unwrap_or_else(Fraction::zero);
                    if !derived.is_zero() {
                        crate::state::persist_event(
                            app,
                            &rt.bot_id,
                            "WARN",
                            "ORDER",
                            &format!("LIVE kalan dolum referans fiyatı yoktu ({id}) — son kapanış ({}) ile deftere yazıldı", derived.to_half_even_display(8)),
                        )
                        .await;
                        price = derived;
                    }
                }
                if price.is_zero() {
                    crate::state::persist_event(
                        app,
                        &rt.bot_id,
                        "CRITICAL",
                        "ORDER",
                        &format!("LIVE kalan dolum takibi REFERANS FİYATSIZ — kitaba yazılmadı ({id}, kalan {qty}); sonraki poll'da fiyat bulunursa sweep açar"),
                    )
                    .await;
                    continue;
                }
                // Faz 4.5: resting partial — deftere (+) (kilit-dışı düzeltme: eski kod
                // market place'te bump(+) yapmıyordu; poll-refresh zaten düzeltiyordu).
                {
                    let n = qty.mul(&price).unwrap_or_else(|_| Fraction::zero());
                    let cost = if !rt.stepper.cfg.is_spot {
                        let lev = if rt.stepper.cfg.leverage.is_zero() { Fraction::one() } else { rt.stepper.cfg.leverage.clone() };
                        n.div(&lev).unwrap_or(n)
                    } else {
                        n
                    };
                    crate::state::bump_venue_live_load(app, &rt.venue, &cost, true).await;
                }
                crate::state::upsert_live_order(
                    app,
                    &rt.bot_id,
                    &id,
                    if side == dca_math::Side::Sell { "Sell" } else { "Buy" },
                    if kind == OrderKind::Reduce { "Reduce" } else { "Add" },
                    &price.to_string(),
                    &qty.to_string(),
                    &role,
                    "PARTIAL",
                );
                if !rt.live_open_orders.iter().any(|o| o.id == id) {
                    rt.live_open_orders.push(crate::state::LiveTrackedOrder { id, side, kind, price, qty, role });
                }
            }
            crate::state::IntentOutcome::MarketQueryFailed { id, err } => {
                let msg = format!("LIVE market fill sorgusu başarısız ({id}): {err} — dolum sweep'te order-history ile doğrulanmalı");
                crate::state::persist_event(app, &rt.bot_id, "WARN", "ORDER", &msg).await;
                let _ = sse.send(SseEvent { event: "LOG".into(), data: json!({"botId": rt.bot_id, "level": "WARN", "message": msg, "ts": crate::state::now_ms()}).to_string() });
            }
            crate::state::IntentOutcome::GateBlocked { id, reason } => {
                let msg = format!("LIVE emir gönderilmedi ({id}): {reason} — faz geri alındı, koşul düzelince yeniden denenecek");
                crate::state::persist_event(app, &rt.bot_id, "WARN", "ORDER_GATE", &msg).await;
                for a in rollback_pending_phase_after_gate_reject(app, rt) {
                    Box::pin(dispatch_action(app, rt, a, crate::state::now_ms(), sse)).await?;
                }
            }
            crate::state::IntentOutcome::PlaceFailed { id, err } => {
                let msg = format!("LIVE emir yerleştirilemedi ({id}): {err} — faz geri alındı, koşul düzelince yeniden denenecek (P0-6 kalanı)");
                crate::state::persist_event(app, &rt.bot_id, "WARN", "ORDER", &msg).await;
                // Faz 22 (P0-6 kalanı): PlaceFailed artık GateBlocked ile AYNI
                // faz-geri-alma disiplininden geçer — borsa emri reddederse
                // (minNotional, risk limiti, bakım) strateji "emir verildi"
                // varsayımında takılıp kalırdı (BasePending/Closing kilidi).
                // Reduce (kapanış) emirleri hariç: Closing'teki pozisyon P0-6
                // retry döngüsüne (3 bar) sahiptir, fazı geri almak pozisyonu
                // sahipsiz bırakırdı — orada mekanizma retry'dır.
                let is_reduce = rt
                    .live_open_orders
                    .iter()
                    .find(|o| o.id == *id)
                    .map(|o| o.kind == OrderKind::Reduce)
                    .unwrap_or(false);
                if !is_reduce {
                    for a in rollback_pending_phase_after_gate_reject(app, rt) {
                        Box::pin(dispatch_action(app, rt, a, crate::state::now_ms(), sse)).await?;
                    }
                } else {
                    crate::state::persist_event(app, &rt.bot_id, "INFO", "ORDER", &format!("kapanış emri reddi ({id}) — Closing retry döngüsünde yeniden gönderilecek")).await;
                }
            }
            crate::state::IntentOutcome::Cancelled { cancelled, failed } => {
                for link in failed {
                    let msg = format!("LIVE cancel başarısız ({link})");
                    crate::state::persist_event(app, &rt.bot_id, "WARN", "CANCEL", &msg).await;
                }
                for link in cancelled {
                    // defterden düşülecek yük — retain'den ÖNCE okunur
                    let cost = rt
                        .live_open_orders
                        .iter()
                        .find(|o| o.id == link)
                        .map(|o| {
                            let n = o.qty.mul(&o.price).unwrap_or_else(|_| Fraction::zero());
                            if o.kind == OrderKind::Add && !rt.stepper.cfg.is_spot {
                                let lev = if rt.stepper.cfg.leverage.is_zero() { Fraction::one() } else { rt.stepper.cfg.leverage.clone() };
                                n.div(&lev).unwrap_or(n)
                            } else {
                                n
                            }
                        });
                    rt.live_open_orders.retain(|o| o.id != link);
                    crate::state::remove_live_order(app, &rt.bot_id, &link);
                    if let Some(cost) = cost {
                        crate::state::bump_venue_live_load(app, &rt.venue, &cost, false).await;
                    }
                    crate::state::persist_event(app, &rt.bot_id, "INFO", "CANCEL", &format!("LIVE emir iptal edildi: {link}")).await;
                    let _ = sse.send(SseEvent { event: "ORDER".into(), data: json!({"botId": rt.bot_id, "id": link, "result": {"cancelled": true}}).to_string() });
                }
            }
        }
    }
    Ok(())
}

/// Faz 13A-POLL: kilit-dışı cascade — ≤8 tur (patolojik zincire sınır).
/// Her tur: [kısa kilit] intent'leri al → [kilitsiz] execute → [kısa kilit] apply.
async fn run_intent_cascade(
    app: &Arc<AppState>,
    bot_id: &str,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    let mut rounds = 0u8;
    loop {
        let (intents, creds) = {
            let mut rts = app.runtimes.lock().await;
            let Some(rt) = rts.get_mut(bot_id) else { return Ok(()) };
            let intents = std::mem::take(&mut rt.pending_live_intents);
            (intents, rt.credentials.clone())
        };
        if intents.is_empty() {
            break;
        }
        if rounds >= 8 {
            crate::state::persist_event(app, bot_id, "CRITICAL", "CASCADE", "intent cascade 8 turu aştı — kalan niyetler sonraki poll'a ertelendi").await;
            let mut rts = app.runtimes.lock().await;
            if let Some(rt) = rts.get_mut(bot_id) {
                rt.pending_live_intents.extend(intents);
            }
            break;
        }
        rounds += 1;
        let outcomes = execute_live_intents(app, creds.as_ref(), intents).await;
        {
            let mut rts = app.runtimes.lock().await;
            let Some(rt) = rts.get_mut(bot_id) else { return Ok(()) };
            apply_intent_outcomes(app, rt, outcomes, sse).await?;
        }
    }
    // Kaskad muhasebeyi değiştirdiyse kalıcı snapshot + UI cache tazele.
    if rounds > 0 {
        let mut rts = app.runtimes.lock().await;
        if let Some(rt) = rts.get_mut(bot_id) {
            crate::state::save_runtime_state(app, bot_id, rt);
            let snap_v = rt.snapshot_json();
            app.ui_cache.lock().await.insert(bot_id.to_string(), snap_v);
        }
    }
    Ok(())
}

/// M-14: emir-kapisi (ORDER_GATE) reddi KALICI bir yetersizliktir — strateji
/// fazını "emir verildi" varsayımından geri alıp yeni karara izin ver.
/// Dca: BasePending→Idle (yeni deal denenebilir); Grid: bootstrap geri alınır
/// (mesh yeniden kurulur); Hedge ana bacak Dca ile aynı.
pub fn rollback_pending_phase_after_gate_reject(app: &AppState, rt: &mut BotRuntime) -> Vec<dca_strategy::Action> {
    let mut out: Vec<dca_strategy::Action> = Vec::new();
    match &mut rt.engine_kind {
        dca_strategy::EngineTag::Dca(e) => {
            if e.phase == dca_strategy::dca::DcaPhase::BasePending {
                e.phase = dca_strategy::dca::DcaPhase::Idle;
            }
        }
        dca_strategy::EngineTag::Hedge(h) => {
            if h.main.phase == dca_strategy::dca::DcaPhase::BasePending {
                h.main.phase = dca_strategy::dca::DcaPhase::Idle;
            }
        }
        dca_strategy::EngineTag::Grid(g) => {
            // Faz 23 (derin inceleme): rollback mesh'i DURDURUR — eski davranış
            // yalnız `grid_bootstrapped = false` yapıyordu; tahtada kalan N−1
            // emir dururken sonraki poll aynı seviyelere YENİ mesh kuruyordu
            // (çift pozisyon + çift closer). halt_mesh resting emirleri
            // idempotent iptal eder, haritayı temizler, kapıyı kapatır.
            out.extend(g.halt_mesh());
            rt.grid_bootstrapped = false;
        }
    }
    let _ = app; // event çağıran yazar
    out
}

/// Faz 4 (C-6): tek gerçek-emir geçidi — LIVE place öncesi (1) enstrüman
/// kuralları (minNotional) ve (2) venue uygun bakiyesi (bu emrin marj maliyeti +
/// aynı venue'daki diğer LIVE botların açık-emir yükü) kontrol edilir. Başarısız
/// durumda emir GÖNDERİLMEZ; WARN ile görünür kalır ve motor akışı sürer
/// (fail-closed + fail-safe). Ölçüm: wallet.available (venue gerçekliği).
/// Faz 13B: kernel fill'leri (ana veya hedge stepper'dan) engine'e iletir —
/// idempotency + persist + SSE + cascade. Engine, order-id prefix'ine göre
/// bacak mirror'ına zaten yönlendirir (hedge.rs on_fill_leg).
async fn ingest_kernel_fills(
    app: &AppState,
    rt: &mut BotRuntime,
    fills: Vec<FillRecord>,
    bar_t: i64,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    let bot_id = rt.bot_id.clone();
    for fill in &fills {
        if fill_exists(app, &bot_id, &fill.fill_key).await.unwrap_or(false) {
            continue;
        }
        let acts = rt.engine_kind.on_fill(fill);
        let _inserted = persist_fill(app, &bot_id, fill).await;
        let _ = sse.send(SseEvent {
            event: "FILL".into(),
            data: json!({
                "botId": bot_id,
                "fillKey": fill.fill_key,
                "orderId": fill.order_id,
                "role": fill.role,
                "side": format!("{:?}", fill.side),
                "kind": format!("{:?}", fill.kind),
                "price": fill.price.to_string(),
                "qty": fill.qty.to_string(),
                "fee": fill.fee.to_string(),
                "feeRole": format!("{}", fill.fee_role),
                "fillModel": format!("{:?}", fill.realism),
                "ts": fill.ts,
            })
            .to_string(),
        });
        for a in acts {
            dispatch_action(app, rt, a, bar_t, sse).await?;
        }
    }
    Ok(())
}

async fn pre_order_gate_ctx(
    app: &AppState,
    gate: &crate::state::GateCtx,
    adapter: &dyn VenueAdapter,
    creds: &dca_exchange::VenueCreds,
    price: Option<&Fraction>,
    qty: &Fraction,
) -> Result<(), String> {
    // Faz 4.5 (C-5): hesap pozisyon-modu uyuşmazsa venue zaten reddeder —
    // emri hiç göndermeyip görünür kalmak fail-closed olanıdır.
    if gate.pos_mode_blocked {
        return Err("hesap pozisyon modu uyuşmuyor (hedge-mode'da DCA one-way emri) — Bybit'te tek-yönlü moda geçin".into());
    }
    let px = price
        .cloned()
        .or_else(|| gate.last_close.clone())
        .ok_or_else(|| "fiyat referansı yok (son kapanış beklenmeli)".to_string())?;
    let notional = qty.mul(&px).map_err(|e| e.to_string())?;
    if notional < gate.min_notional {
        return Err(format!(
            "minNotional: emir değeri {} < borsa minimumu {}",
            notional.to_half_even_display(2),
            gate.min_notional.to_half_even_display(2)
        ));
    }
    let lev = if gate.leverage.is_zero() { Fraction::one() } else { gate.leverage.clone() };
    let need = if gate.is_spot { notional } else { notional.div(&lev).map_err(|e| e.to_string())? };
    let others = app
        .venue_live_load
        .lock()
        .await
        .get(&gate.venue)
        .cloned()
        .unwrap_or_else(Fraction::zero);
    let wallet = adapter.wallet(creds, &gate.quote).await.map_err(|e| format!("wallet okunamadı: {e}"))?;
    let total_need = need.add(&others).map_err(|e| e.to_string())?;
    if total_need > wallet.available {
        return Err(format!(
            "hesap bakiyesi yetersiz: gereken {} (diğer LIVE botların açık yükü {} dahil) > venue uygun bakiye {}",
            total_need.to_half_even_display(2),
            others.to_half_even_display(2),
            wallet.available.to_half_even_display(2)
        ));
    }
    Ok(())
}

pub async fn dispatch_action(
    app: &AppState,
    rt: &mut BotRuntime,
    action: Action,
    ts: i64,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    // Faz 21 (N-2, KRİTİK): positionIdx emrin GİTTİĞİ bacağın yönünden
    // türetilir. Hedge bacağı (id "-hedge" önekli) ana bacağın TERSİdir;
    // eski kod her iki bacak için de ana bacağın yönünü kullanıyordu —
    // hedge'in Sell emirleri ana bacağın idx'iyle gidiyor, Bybit hedge-mode'da
    // yanlış pozisyona yazılıyor ya da reddediliyordu (para kaybı sınıfı).
    let leg_direction = |rt: &BotRuntime, id: &str| -> i8 {
        let is_hedge = id.starts_with(&format!("{}-hedge", rt.bot_id));
        if is_hedge {
            rt.hedge_stepper
                .as_ref()
                .map(|h| h.position.direction)
                .unwrap_or(1)
        } else {
            rt.stepper.position.direction
        }
    };
    match action {
        Action::PlaceLimit { id, side, price, qty, role } => {
            // Faz 19 (P1-4): TP de resting reduce-only limit — GRID_CLOSE ile
            // aynı kapanış muafiyetleri (GATE6 yok, reduce_only, positionIdx
            // bacak yönü) TP için de geçerlidir.
            let is_close = role == "GRID_CLOSE" || role == "TP";
            // Faz 27 (İ-1): portföy-düzeyi max-total-notional kapısı — yalnız
            // Add emirleri (BASE/SAFETY/hedge açılış). Kapanış emirleri muaf:
            // kapanış notional'i DÜŞÜRÜR, çıkış yolu hep açık kalmalı.
            if !is_close {
                let cap = crate::fleet::portfolio_cap_pct_from_env(
                    std::env::var("DCA_PORTFOLIO_MAX_NOTIONAL_PCT").ok().as_deref(),
                );
                if cap > 0 {
                    // Kilit-siz cache okuması — runtimes kilidi bu noktada
                    // ZATEN çağırıcıda; tekrar istemek deadlock üretir.
                    if let Some((nav, gross)) = crate::fleet::portfolio_cache_read(app, 30_000) {
                        let candidate = price.mul(&qty).unwrap_or_else(|_| Fraction::zero());
                        if crate::fleet::portfolio_cap_exceeded(&gross, &candidate, &nav, cap) {
                            let msg = format!(
                                "PORTFOLIO CAP: {} emri yerlestirilmedi — acik brut notional + aday {} NAV'in %{} sinirini asar (su anki brut/NAV agirligi asildi). Emir yok; seviye sonraki barda yeniden degerlendirilir. Esigi yukseltmek icin DCA_PORTFOLIO_MAX_NOTIONAL_PCT.",
                                role,
                                candidate,
                                cap
                            );
                            crate::state::persist_event(app, &rt.bot_id, "WARN", "", &msg).await;
                            let _ = sse.send(SseEvent {
                                event: "LOG".into(),
                                data: json!({"botId": rt.bot_id, "level": "WARN", "message": msg, "ts": ts}).to_string(),
                            });
                            return Ok(());
                        }
                        crate::fleet::portfolio_cache_add_candidate(app, &candidate);
                    }
                    // taze değilse: kapı bu turu atlar (fail-open)
                }
            }
            match rt.mode.as_str() {
                "PAPER" => {
                    // Faz 13B: hedge-prefix'li emirler hedge bacak stepper'ına.
                    let is_hedge = id.starts_with(&format!("{}-hedge", rt.bot_id));
                    let ref_price = rt.stepper.last_close.clone().and_then(|s| Fraction::from_any_str(&s).ok());
                    let target = if is_hedge {
                        rt.hedge_stepper.as_mut()
                    } else {
                        Some(&mut rt.stepper)
                    };
                    target
                        .ok_or_else(|| "hedge stepper yok".to_string())?
                        .submit_intent(
                            OrderIntent {
                                id: id.clone(),
                                side,
                                kind: if is_close { OrderKind::Reduce } else { OrderKind::Add },
                                price: Some(price),
                                qty,
                                decision_ts: ts,
                                role: role.to_string(),
                                placement_ref_price: ref_price,
                                quote_notional: None,
                            },
                            ts,
                        )
                        .map_err(|e| e.to_string())?;
                }
                _ => {
                    // Faz 13A-POLL: LIVE ağ çağrısı yok — intent kuyruğa alınır;
                    // gate (wallet ağı) + place, poll sonrası kilit-dışı cascade'de.
                    let gate = crate::state::GateCtx {
                        pos_mode_blocked: rt.pos_mode_blocked,
                        last_close: rt.stepper.last_close.as_deref().and_then(|s| Fraction::from_any_str(s).ok()),
                        leverage: rt.stepper.cfg.leverage.clone(),
                        is_spot: rt.stepper.cfg.is_spot,
                        quote: rt.stepper.instrument.quote.clone(),
                        min_notional: rt.stepper.instrument.min_notional.clone(),
                        venue: rt.venue.clone(),
                    };
                    let req = dca_exchange::OrderRequest {
                        category: if rt.cfg.kind.is_futures() { "linear".into() } else { "spot".into() },
                        symbol: rt.cfg.symbol.clone(),
                        side,
                        is_market: false,
                        qty: qty.clone(),
                        price: Some(price.clone()),
                        reduce_only: is_close,
                        order_link_id: id.clone(),
                        // Faz 5 (C-5): hedge modda yön eşlemesi (Buy→1/Sell→2)
                        position_idx: if rt.cfg.kind.is_futures() && rt.pos_mode_hedge {
                            // P0-1 + Faz 21 (N-2): idx, emrin gittiği BACAĞIN
                            // pozisyon yönünden gelir — hedge bacağı emirleri
                            // ana bacağın idx'ini ASLA almaz.
                            if leg_direction(rt, &id) == 1 { 1 } else { 2 }
                        } else { 0 },
                    };
                    rt.pending_live_intents.push(crate::state::LiveIntent::Place {
                        req,
                        id: id.clone(),
                        side,
                        kind: if is_close { OrderKind::Reduce } else { OrderKind::Add },
                        price: Some(price.clone()),
                        qty: qty.clone(),
                        role: role.to_string(),
                        is_close,
                        gate,
                    });
                }
            }
        }
        Action::PlaceMarket { id, side, qty, role, quote_notional } => {
            let kind = if matches!(role, "TP" | "SL" | "MANUAL" | "TRAIL") { OrderKind::Reduce } else { OrderKind::Add };
            // Faz 27 (İ-1): portföy kapısı — PlaceMarket Add emirleri (BASE,
            // hedge açılış). Reduce rolleri (TP/SL/MANUAL/TRAIL) muaf.
            // Aday notional: quote_notional (QUOTE_NOTIONAL boyutlama) yoksa
            // mark × qty ile yaklaşık son kapanıştan ölçülür.
            if kind == OrderKind::Add {
                let cap = crate::fleet::portfolio_cap_pct_from_env(
                    std::env::var("DCA_PORTFOLIO_MAX_NOTIONAL_PCT").ok().as_deref(),
                );
                if cap > 0 {
                    // Kilit-siz cache okuması (PlaceLimit kolu ile aynı desen).
                    if let Some((nav, gross)) = crate::fleet::portfolio_cache_read(app, 30_000) {
                        let candidate = match &quote_notional {
                            Some(q) => q.clone(),
                            None => rt
                                .stepper
                                .last_close
                                .as_deref()
                                .and_then(|s| Fraction::from_any_str(s).ok())
                                .and_then(|mark| mark.mul(&qty).ok())
                                .unwrap_or_else(Fraction::zero),
                        };
                        if crate::fleet::portfolio_cap_exceeded(&gross, &candidate, &nav, cap) {
                            let msg = format!(
                                "PORTFOLIO CAP: {} emri yerlestirilmedi — acik brut notional + aday {} NAV'in %{} sinirini asar. Emir yok; seviye sonraki barda yeniden degerlendirilir. Esigi yukseltmek icin DCA_PORTFOLIO_MAX_NOTIONAL_PCT.",
                                role,
                                candidate,
                                cap
                            );
                            crate::state::persist_event(app, &rt.bot_id, "WARN", "", &msg).await;
                            let _ = sse.send(SseEvent {
                                event: "LOG".into(),
                                data: json!({"botId": rt.bot_id, "level": "WARN", "message": msg, "ts": ts}).to_string(),
                            });
                            return Ok(());
                        }
                        crate::fleet::portfolio_cache_add_candidate(app, &candidate);
                    }
                    // taze değilse: kapı bu turu atlar (fail-open)
                }
            }
            match rt.mode.as_str() {
                "PAPER" => {
                    // Faz 13B: hedge-prefix'li emirler hedge bacak stepper'ına.
                    let is_hedge = id.starts_with(&format!("{}-hedge", rt.bot_id));
                    let target = if is_hedge {
                        rt.hedge_stepper.as_mut()
                    } else {
                        Some(&mut rt.stepper)
                    };
                    target
                        .ok_or_else(|| "hedge stepper yok".to_string())?
                        .submit_intent(
                            OrderIntent {
                                id: id.clone(),
                                side,
                                kind,
                                price: None,
                                qty,
                                decision_ts: ts,
                                role: role.to_string(),
                                placement_ref_price: None,
                                quote_notional,
                            },
                            ts,
                        )
                        .map_err(|e| e.to_string())?;
                }
                _ => {
                    // Faz 13A-POLL: LIVE market — intent kuyruğa; gate + place +
                    // 3× dolum sorgusu kilit-dışı cascade'de, ingest apply'da.
                    let kind_m = kind.clone();
                    let gate = crate::state::GateCtx {
                        pos_mode_blocked: rt.pos_mode_blocked,
                        last_close: rt.stepper.last_close.as_deref().and_then(|s| Fraction::from_any_str(s).ok()),
                        leverage: rt.stepper.cfg.leverage.clone(),
                        is_spot: rt.stepper.cfg.is_spot,
                        quote: rt.stepper.instrument.quote.clone(),
                        min_notional: rt.stepper.instrument.min_notional.clone(),
                        venue: rt.venue.clone(),
                    };
                    let req = dca_exchange::OrderRequest {
                        category: if rt.cfg.kind.is_futures() { "linear".into() } else { "spot".into() },
                        symbol: rt.cfg.symbol.clone(),
                        side,
                        is_market: true,
                        qty: qty.clone(),
                        price: None,
                        reduce_only: kind_m == OrderKind::Reduce,
                        order_link_id: id.clone(),
                        // Faz 5 (C-5): hedge modda yön eşlemesi (Buy→1/Sell→2)
                        position_idx: if rt.cfg.kind.is_futures() && rt.pos_mode_hedge {
                            // P0-1 + Faz 21 (N-2): idx, emrin gittiği BACAĞIN
                            // pozisyon yönünden gelir.
                            if leg_direction(rt, &id) == 1 { 1 } else { 2 }
                        } else { 0 },
                    };
                    let is_close_m = kind_m == OrderKind::Reduce;
                    rt.pending_live_intents.push(crate::state::LiveIntent::Place {
                        req,
                        id: id.clone(),
                        side,
                        kind: kind_m,
                        price: None,
                        qty: qty.clone(),
                        role: role.to_string(),
                        is_close: is_close_m,
                        gate,
                    });
                }
            }
        }
        Action::CancelResting { id } => {
            // Faz 13B: hedge bacak pending'i de temizlenir.
            let is_hedge = id.starts_with(&format!("{}-hedge", rt.bot_id));
            if id.ends_with('*') {
                let prefix = id.trim_end_matches('*');
                if !is_hedge {
                    rt.stepper.pending.retain(|o| !o.id.starts_with(prefix));
                }
                if let Some(hs) = rt.hedge_stepper.as_mut() {
                    hs.pending.retain(|o| !o.id.starts_with(prefix));
                }
            } else {
                if !is_hedge {
                    rt.stepper.pending.retain(|o| o.id != id);
                }
                if let Some(hs) = rt.hedge_stepper.as_mut() {
                    hs.pending.retain(|o| o.id != id);
                }
            }
            // Faz 13A-POLL: LIVE cancel — ağ (wildcard çözümü + cancel) kilit
            // dışı cascade'de; yerel pending listesi yukarıda senkron temizlendi.
            if rt.mode == "LIVE" {
                rt.pending_live_intents.push(crate::state::LiveIntent::Cancel {
                    links: vec![id.clone()],
                    venue: rt.venue.clone(),
                    category: if rt.cfg.kind.is_futures() { "linear".to_string() } else { "spot".to_string() },
                    symbol: rt.cfg.symbol.clone(),
                });
            }
        }
        Action::Log { level, message } => {
            crate::state::persist_event(app, &rt.bot_id, level, "", &message).await;
            let _ = sse.send(SseEvent {
                event: "LOG".into(),
                data: json!({"botId": rt.bot_id, "level": level, "message": message, "ts": ts}).to_string(),
            });
        }
    }
    Ok(())
}

/// BOT-02: INSERT OR IGNORE — aynı (bot_id, fill_key) zaten varsa sessizce
/// atlanır. Gerçekten yeni bir fill yazıldıysa true döner; duplicate ise
/// false, ki çağrıcı accounting'i tekrar applied etmesin.
async fn persist_fill(app: &AppState, bot_id: &str, f: &FillRecord) -> Result<bool, String> {
    let conn = app.db.conn.lock().map_err(|e| e.to_string())?;
    let res = conn.execute(
        "INSERT OR IGNORE INTO fills(bot_id, fill_key, order_id, role, side, kind, price, qty, fee, fee_role, realism, ts) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        rusqlite::params![
            bot_id,
            f.fill_key,
            f.order_id,
            f.role,
            format!("{:?}", f.side),
            format!("{:?}", f.kind),
            f.price.to_string(),
            f.qty.to_string(),
            f.fee.to_string(),
            format!("{}", f.fee_role),
            format!("{:?}", f.realism),
            f.ts
        ],
    )
    .map_err(|e| e.to_string())?;

    // Faz 0.9 (B-4): fills tablosu retention — her 64 dolum yazımında son 50.000 kayıt tutulur, disk dolması engellenir
    static FILL_WRITE_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    if FILL_WRITE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 64 == 0 {
        let _ = conn.execute(
            "DELETE FROM fills WHERE id < (SELECT MIN(id) FROM (SELECT id FROM fills ORDER BY id DESC LIMIT 50000))",
            [],
        );
    }
    Ok(res > 0)
}

/// BOT-02: DB'de aynı (bot_id, fill_key) zaten kayıtlı mı? Duplicate fill
/// accounting'ten önce kontrol edilir — kernel'e tekrar uygulanmaz.
async fn fill_exists(app: &AppState, bot_id: &str, fill_key: &str) -> Result<bool, String> {
    let conn = app.db.conn.lock().map_err(|e| e.to_string())?;
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM fills WHERE bot_id=?1 AND fill_key=?2", rusqlite::params![bot_id, fill_key], |r| r.get(0))
        .unwrap_or(0);
    Ok(n > 0)
}

async fn persist_equity(app: &AppState, bot_id: &str, ts: i64, eq: &Fraction) -> Result<(), String> {
    let conn = app.db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute("INSERT INTO equity_points(bot_id, ts, equity) VALUES(?1,?2,?3)", rusqlite::params![bot_id, ts, eq.to_string()])
        .map_err(|e| e.to_string())?;
    // Task 20 L6 — sınırsız büyüme kapandı: bot başına son 5000 nokta saklanır
    // (tablo write-only'ydi; 1m bot ≈ 1440 satır/gün şişmeye yol açıyordu).
    conn.execute(
        "DELETE FROM equity_points WHERE bot_id=?1 AND id < (SELECT MIN(id) FROM (SELECT id FROM equity_points WHERE bot_id=?1 ORDER BY id DESC LIMIT 5000))",
        rusqlite::params![bot_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Immediate webhook force-start (3Commas parity): opens the deal at the
/// freshest venue price instead of waiting for the next bar close. Consumes
/// the webhook flag exactly once; safe to call from the poll loop.
pub async fn force_start_if_requested(
    app: &AppState,
    rt: &mut BotRuntime,
    last_kline: Option<&Kline>,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    if !rt.webhook_start_requested || rt.status != "RUNNING" {
        return Ok(());
    }
    // Faz 0.2/0.3 — POST ile poll arası yarış penceresi kapağı: kill tüm
    // modlarda TV ENTRY'yi reddeder (v3), freeze yalnız LIVE hatta.
    if app.kill_engaged().await {
        rt.webhook_start_requested = false;
        crate::state::persist_event(app, &rt.bot_id, "WARN", crate::state::GATE_KILL, "START_DEAL reddedildi — KILL_SWITCH devrede").await;
        return Ok(());
    }
    if rt.mode == "LIVE" && app.freeze_engaged().await {
        rt.webhook_start_requested = false;
        crate::state::persist_event(app, &rt.bot_id, "WARN", crate::state::GATE_FREEZE, "START_DEAL reddedildi — FREEZE devrede (LIVE hat)").await;
        return Ok(());
    }
    let Some(k) = last_kline else { return Ok(()) };
    rt.webhook_start_requested = false;
    let bot_id = rt.bot_id.clone();
    let mark = Fraction::from_decimal_str(&k.c).map_err(|e| e.to_string())?;
    let bar = Bar { t: k.t, o: k.o.clone(), h: k.h.clone(), l: k.l.clone(), c: k.c.clone(), v: k.v.clone() };
    let o = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
    let h = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
    let l = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
    let c = mark.clone();
    let n: usize;
    match &mut rt.engine_kind {
        dca_strategy::EngineTag::Dca(e) => {
            e.request_force_start();
            let ctx = dca_strategy::BarContext {
                bar: &bar,
                open: o,
                high: h,
                low: l,
                close: c,
                position_qty: &rt.stepper.position.qty,
                position_avg: rt.stepper.position.avg_price.as_ref(),
                cash: &rt.stepper.cash,
                mark,
                signal_ok: None,
                regime: None,
                deal_open_blocked: false,
            };
            let acts = e.try_open_deal(&ctx);
            n = acts.len();
            for a in acts {
                dispatch_action(app, rt, a, bar.t, sse).await?;
            }
        }
        dca_strategy::EngineTag::Grid(g) => {
            g.force_start_pending = true;
            let acts = g.bootstrap(&mark);
            n = acts.len();
            for a in acts {
                dispatch_action(app, rt, a, bar.t, sse).await?;
            }
            rt.grid_bootstrapped = true;
        }
        dca_strategy::EngineTag::Hedge(hg) => {
            // Force path opens the MAIN leg at market; the hedge leg follows
            // its own machinery (dual) or the drawdown spawn (protective).
            hg.main.request_force_start();
            let ctx = dca_strategy::BarContext {
                bar: &bar,
                open: o,
                high: h,
                low: l,
                close: c,
                position_qty: &rt.stepper.position.qty,
                position_avg: rt.stepper.position.avg_price.as_ref(),
                cash: &rt.stepper.cash,
                mark,
                signal_ok: None,
                regime: None,
                deal_open_blocked: false,
            };
            let acts = hg.main.try_open_deal(&ctx);
            n = acts.len();
            for a in acts {
                dispatch_action(app, rt, a, bar.t, sse).await?;
            }
        }
    }
    crate::state::persist_event(app, &bot_id, "INFO", "WEBHOOK", &format!("force START_DEAL executed at market price ({n} actions)")).await;
    let _ = sse.send(SseEvent {
        event: "WEBHOOK_START".into(),
        data: json!({ "botId": bot_id, "actions": n }).to_string(),
    });
    Ok(())
}

/// Fetch latest klines and advance the bot by any new bars.
/// Faz 2.4 — tick iz çerçevesi: her poll bir `trace_id` altında koşar.
/// `persist_event` haritada iz bulursa iliştirir; yoksa NULL yazar.
/// Çıkış tek kapıdan olur (harita sızıntısı yok).
pub async fn poll_and_advance(
    app: &Arc<AppState>,
    bot_id: &str,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<usize, String> {
    // H-1: bot başına single-flight — aynı botun poll'u üst üste binmez;
    // meşgulse dürüst skip (0 bar; supervisor bir sonraki turda yine dener).
    let sf_lock: Arc<tokio::sync::Mutex<()>> = {
        let mut lock_map = app.poll_locks.lock().await;
        lock_map
            .entry(bot_id.to_string())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(())))
            .clone()
    };
    let _poll_guard = match sf_lock.try_lock() {
        Ok(g) => g,
        Err(_) => {
            crate::state::persist_event(app, bot_id, "WARN", "POLL", "önceki poll hâlâ koşuyor — bu tur atlandı (single-flight)").await;
            return Ok(0);
        }
    };
    let trace = crate::state::new_tick_trace(bot_id);
    // Faz 5 (H-8): readiness — poll BAŞLADI sayılır (hattın yaşadığının işareti);
    // hata durumunda da son deneme zamanı bilinir.
    if let Ok(mut t) = app.last_tick_at.lock() {
        *t = crate::state::now_ms();
    }
    app.tick_trace.lock().await.insert(bot_id.to_string(), trace.clone());
    // TICK kökü: izin ilk satırı (traces listesi buradan özetlenir).
    // Faz 21 (P2-9): TICK olayı events tablosuna ARTIK YAZILMAZ — her poll'da
    // bot başına bir satır üretip tabloyu şişiriyordu (12 bot × poll ≈ günde
    // milyonlarca satır); iz zaten tick_trace ring'inde görünüyor.
    let out = poll_inner(app, bot_id, sse).await;
    app.tick_trace.lock().await.remove(bot_id);
    out
}

/// H-1 (adım 1/2): Faz-1 bağlam kopyası — kilit yalnız bu kopya süresince
/// tutulur; ağ çağrıları kilitsiz Faz 2'de koşar.
struct PollCtx {
    venue: String,
    #[allow(dead_code)]
    mode: String,
    #[allow(dead_code)]
    status: String,
    interval_str: String,
    symbol: String,
    creds: Option<dca_exchange::VenueCreds>,
    is_futures: bool,
    halted: bool,
    need_leverage: bool,
    leverage: Fraction,
    need_mmr: bool,
    need_pos_mode: bool,
    need_venue_pos_check: bool,
    venue_close_ref: Option<String>,
    venue_avg_ref: Option<String>,
    /// H-1 adim 2: sinyal/pine kararları kilitsiz Faz 2'de hesaplanır.
    entry_signal: Option<dca_signals::SignalConfig>,
    pine_script: Option<String>,
    pine_inputs: std::collections::BTreeMap<String, String>,
    strategy_blocked_hint: bool,
}

async fn poll_inner(
    app: &Arc<AppState>,
    bot_id: &str,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<usize, String> {
    // Faz 4 (C-6): venue başına LIVE açık-emir yükü — rt ödünç alınmadan ÖNCE
    // ayrı kilit periyodunda hesaplanır (aynı harita üzerinde çift ödünç yok;
    // emir öncesi kapı bu defterden okur).
    {
        let rts = app.runtimes.lock().await;
        let mut loads: HashMap<String, Fraction> = HashMap::new();
        for o in rts.values() {
            if o.mode != "LIVE" {
                continue;
            }
            for od in &o.live_open_orders {
                let n = od.qty.mul(&od.price).unwrap_or_else(|_| Fraction::zero());
                let cost = if od.kind == OrderKind::Add && !o.stepper.cfg.is_spot {
                    let lev = if o.stepper.cfg.leverage.is_zero() { Fraction::one() } else { o.stepper.cfg.leverage.clone() };
                    n.div(&lev).unwrap_or(n)
                } else {
                    n
                };
                let e = loads.entry(o.venue.clone()).or_insert_with(Fraction::zero);
                *e = e.add(&cost).unwrap_or_else(|_| e.clone());
            }
        }
        *app.venue_live_load.lock().await = loads;
        // Faz 27 (İ-1): portföy kapısı anlık görüntüsü — aynı kilit altında
        // hesaplanır, dispatch'teki kapı ayrı std kilitten okur (deadlock yok).
        crate::fleet::refresh_portfolio_cache(app, &rts);
    }

    // ---------------- H-1 FAZ 1: kısa kilit — bağlam kopyası ----------------
    let ctx: PollCtx = {
        let mut rts = app.runtimes.lock().await;
        let rt = rts.get_mut(bot_id).ok_or("runtime not found")?;
        // Task 18 Ş1 + P4-3: bilinmeyen venue dürüst uyarısı (bir kez).
        if rt.venue != "BYBIT" && rt.venue != "BINANCE" && rt.venue != "OKX" && rt.venue != "PAPER" && !rt.venue_warned {
            rt.venue_warned = true;
            crate::state::persist_event(app, bot_id, "WARN", "ENGINE", &format!("bilinmeyen venue '{}' — adapter yok, poll hata verecek", rt.venue)).await;
        }
        let is_fut = rt.cfg.kind.is_futures();
        let halted = rt.mode == "LIVE" && app.live_halted().await;
        const MMR_TTL_MS: i64 = 600_000;
        let now_v = crate::state::now_ms();
        PollCtx {
            venue: rt.venue.clone(),
            mode: rt.mode.clone(),
            status: rt.status.clone(),
            interval_str: rt.interval_str.clone(),
            symbol: rt.cfg.symbol.clone(),
            creds: rt.credentials.clone(),
            is_futures: is_fut,
            halted,
            need_leverage: rt.mode == "LIVE" && is_fut && !rt.leverage_synced && !halted,
            leverage: Fraction::from_decimal_str(&rt.cfg.leverage).unwrap_or_else(|_| Fraction::one()),
            need_mmr: is_fut && (rt.mmr_brackets.is_none() || now_v - rt.mmr_brackets_at > MMR_TTL_MS),
            need_pos_mode: rt.mode == "LIVE" && is_fut && !rt.pos_mode_checked,
            need_venue_pos_check: rt.mode == "LIVE" && is_fut && !rt.stepper.position.qty.is_zero(),
            venue_close_ref: rt.stepper.last_close.clone(),
            venue_avg_ref: rt.stepper.position.avg_price.as_ref().map(|a| a.to_string()),
            entry_signal: rt.cfg.entry_signal.clone(),
            pine_script: rt.cfg.pine_script.clone(),
            pine_inputs: rt.cfg.pine_inputs.clone(),
            strategy_blocked_hint: matches!(rt.status.as_str(), "FROZEN" | "PAUSED" | "STOPPED") || halted,
        }
    };

    // ---------------- H-1 FAZ 2: KİLİTSİZ ağ fazı ----------------
    // (klines + imzalı kurulum çağrıları — runtimes kilidi serbest; UI/action
    // istekleri bu sürede yanıt verebilir. Bot-09'un "tek yavaş venue her şeyi
    // bloklar" kusurunun ana giderimi.)
    let category = if ctx.is_futures { "linear" } else { "spot" };
    // &dyn VenueAdapter ömrü `app`'e bağlıdır (runtimes kilidine değil) —
    // kilitsiz fazda doğrudan alınabilir.
    let adapter = app
        .adapter(&ctx.venue)
        .ok_or("no adapter")?;
    let iv = bybit_interval(&ctx.interval_str)?;
    // Faz 27 (P1 — KAPSAMLI_DENETİM): venue="PAPER" botları ölüydü. PAPER
    // venue'unun kendi bar deposunu üretimde besleyen YOK (inject_bars
    // yalnız test) → klines hep boş → hiç deal açılmıyordu. Belge sözleşmesi
    // "PAPER simülasyonu BYBIT piyasa verisiyle koşar" (state.rs
    // engine_instrument_for da PAPER→BYBIT çözüyor): PİYASA VERİSİ kaynağı
    // PAPER'da BYBIT adapter'ine yönlenir; emir yerleştirme zaten
    // mod=PAPER olduğu için asla borsaya gitmez (yalnız simülasyon kernel'i).
    let market_adapter: &dyn dca_exchange::VenueAdapter = if ctx.venue == "PAPER" {
        &app.bybit
    } else {
        adapter
    };
    let bars: Vec<Kline> = market_adapter
        .klines(category, &ctx.symbol, &iv, 60)
        .await
        .map_err(|e| e.to_string())?;

    let mut leverage_done = false;
    let mut leverage_info: Option<String> = None;
    let mut leverage_err: Option<String> = None;
    if ctx.need_leverage {
        if let Some(creds) = &ctx.creds {
            match adapter.set_leverage(creds, "linear", &ctx.symbol, &ctx.leverage).await {
                Ok(()) => {
                    leverage_done = true;
                    leverage_info = Some(format!("venue leverage set to {}", ctx.leverage.to_half_even_display(2)));
                }
                Err(e) => leverage_err = Some(e.to_string()),
            }
        }
    }
    let mut mmr: Option<Vec<dca_math::MmrBracket>> = None;
    if ctx.need_mmr {
        let target = if ctx.venue == "PAPER" { "BYBIT" } else { ctx.venue.as_str() };
        if let Some(a) = app.adapter(target) {
            if let Ok(b) = a.mmr_brackets(&ctx.symbol, ctx.creds.as_ref()).await {
                if !b.is_empty() {
                    mmr = Some(b);
                }
            }
        }
    }
    let mut pos_mode_result: Option<Option<String>> = None;
    let mut pos_mode_switched = false;
    if ctx.need_pos_mode {
        if let Some(creds) = &ctx.creds {
            // Hata → None: sonraki poll yeniden dener.
            let mut m = adapter.pos_mode(creds).await.unwrap_or(None);
            // Faz 5: Demo/Testnet ucunda hesap hedge modundaysa ve pozisyon
            // yoksa tek-yönlüye otomatik geç dene (mainnet'te ASLA otomatik
            // değil — kullanıcı değiştirir, denetim engel olarak kalır).
            if m.as_deref() == Some("long_short_mode") {
                if let Some(bybit) = app.bybit_demo_aware(&ctx.venue) {
                    match bybit.switch_pos_mode(creds, true).await {
                        Ok(()) => {
                            pos_mode_switched = true;
                            m = adapter.pos_mode(creds).await.unwrap_or(None);
                        }
                        Err(e) => {
                            crate::state::persist_event(app, bot_id, "WARN", "POS_MODE", &format!("Demo/Testnet otomatik tek-yönlü geçişi başarısız: {e}")).await;
                        }
                    }
                }
            }
            pos_mode_result = Some(m);
        }
    }
    let mut venue_flat: Option<bool> = None;
    if ctx.need_venue_pos_check {
        if let Some(creds) = &ctx.creds {
            if let Ok(pos) = adapter.positions(creds, &ctx.symbol).await {
                venue_flat = Some(pos.iter().all(|p| p.size.is_zero()));
            }
        }
    }

    // H-1 adim 2: sinyal/pine kararları — kilitsiz, poll başına BİR kez.
    // (Eski davranışta her bar için yeniden çekiliyordu; fetch'ler 'en son
    // kapalı' pencereye baktığından karar eşdeğerdir — 60x daha az ağ.)
    let pre = if ctx.strategy_blocked_hint {
        PreSignals { tree_ok: None, pine_out: None, sig_engine: None, sig_root: None }
    } else {
        let (tree_ok, sig_engine, sig_root) = match ctx.entry_signal.clone() {
            Some(sc) => {
                let cat = if ctx.is_futures { "linear" } else { "spot" };
                match build_live_signal_engine(app, &ctx.venue, cat, &ctx.symbol, &ctx.interval_str, &sc.root).await {
                    Ok((eng, _asof)) => {
                        let evaluated = eng.evaluate_at_time(&sc.root, crate::state::now_ms());
                        (Some(evaluated), Some(std::sync::Arc::new(eng)), Some(sc.root))
                    }
                    Err(e) => {
                        crate::state::persist_event(app, bot_id, "WARN", "SIGNAL", &format!("signal evaluation failed — fail-closed: {e}")).await;
                        (Some(false), None, None)
                    }
                }
            }
            None => (None, None, None),
        };
        let pine_out: Option<Result<dca_pine::BarVerdict, String>> = if ctx.pine_script.is_some() {
            let cat = if ctx.is_futures { "linear" } else { "spot" };
            match fetch_closed_bars(app, &ctx.venue, cat, &ctx.symbol, &ctx.interval_str).await {
                Ok(pbars) => {
                    let inputs: dca_pine::PineInputs = ctx.pine_inputs.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                    let code = ctx.pine_script.clone().unwrap_or_default();
                    match dca_pine::run_all(&code, &pbars, &inputs) {
                        Ok(res) => Some(Ok(res.verdicts.last().cloned().unwrap_or_default())),
                        Err(e) => {
                            crate::state::persist_event(app, bot_id, "WARN", "PINE", &format!("pine çalıştırma hatası — fail-closed: {e}")).await;
                            Some(Err(e))
                        }
                    }
                }
                Err(e) => {
                    crate::state::persist_event(app, bot_id, "WARN", "PINE", &format!("pine bar verisi alınamadı — fail-closed: {e}")).await;
                    Some(Err(e))
                }
            }
        } else {
            None
        };
        PreSignals { tree_ok, pine_out, sig_engine, sig_root }
    };

    // ---------------- H-1 FAZ 3: kilitli uygulama ----------------
    let mut rts = app.runtimes.lock().await;
    let rt = rts.get_mut(bot_id).ok_or("runtime not found")?;
    let live_halted = ctx.halted;
    if live_halted && !rt.halt_warned {
        rt.halt_warned = true;
        crate::state::persist_event(app, bot_id, "WARN", "HALT", "yürütme şalteri devrede (KILL/FREEZE) — LIVE hatta yeni emir üretilmiyor, açıklar korunuyor").await;
    } else if !live_halted {
        rt.halt_warned = false;
    }
    if leverage_done {
        rt.leverage_synced = true;
        if let Some(n) = leverage_info {
            crate::state::persist_event(app, bot_id, "INFO", "LEVERAGE_SYNC", &n).await;
        }
    }
    if let Some(e) = leverage_err {
        crate::state::persist_event(app, bot_id, "WARN", "LEVERAGE_SYNC", &format!("leverage sync retry next poll: {e}")).await;
    }
    if let Some(b) = mmr {
        rt.mmr_brackets = Some(b);
        rt.mmr_brackets_at = crate::state::now_ms();
    }
    if let Some(r) = pos_mode_result {
        rt.pos_mode_checked = true;
        if r.as_deref() == Some("long_short_mode") {
            // Faz 5 (C-5): hedge modda DCA positionIdx EŞLEMESİYLE çalışır —
            // Buy→1 (long), Sell→2 (short). Engelleme yerine uyum; tek-yönlü
            // hesaplarda davranış değişmez (idx=0).
            rt.pos_mode_hedge = true;
            crate::state::persist_event(app, bot_id, "INFO", "POS_MODE", "Bybit hesabı HEDGE modunda — DCA emirleri positionIdx eşlemesiyle çalışacak (Buy→1, Sell→2)").await;
        } else {
            rt.pos_mode_hedge = false;
            if pos_mode_switched {
                crate::state::persist_event(app, bot_id, "INFO", "POS_MODE", "Demo/Testnet hesabı otomatik tek-yönlü moda alındı (pozisyon yoktu; mainnet'te bu otomasyon asla denenmez)").await;
            }
        }
    }
    if venue_flat == Some(true) {
        let px = ctx
            .venue_close_ref
            .as_deref()
            .and_then(|c| Fraction::from_any_str(c).ok())
            .or_else(|| ctx.venue_avg_ref.as_deref().and_then(|a| Fraction::from_any_str(a).ok()))
            .unwrap_or_else(Fraction::zero);
        let ts = crate::state::now_ms();
        close_position_as_venue_liquidated(app, rt, &px, ts, sse).await?;
    }
    // BOT-04: henüz kapanmamış (forming) son mumu işlemeyecek —
    // sadece `bar_start + interval_ms <= now` olan kapalı barlara karar verilir.
    let interval_ms = crate::engine::iv_str_ms(&rt.interval_str).unwrap_or(3_600_000);
    let now = crate::state::now_ms();
    // C-4 warmup: yeni kurulan / restore edilen botun imleci sıfırdan başlar;
    // ilk poll aksi halde çekilen 60 geçmiş barı "yeniden yaşar" ve bugünkü
    // piyasada tarihsel kararla deal açardı (canlı testte gözlemlendi). İmleç,
    // C-4 kalıcı snapshot gelene kadar, son KAPALI bara atlanır — bot "şimdi"den
    // başlar (Freqtrade'in restart'ta son işlenen mumdan devam modelinin
    // minimal karşılığı; pozisyon uzlaştırması restore tarafındadır).
    if rt.last_bar_ts == 0 && !bars.is_empty() {
        let skipped = bars.iter().filter(|b| b.t + interval_ms <= now).count();
        if let Some(last_closed) = bars.iter().rev().find(|b| b.t + interval_ms <= now) {
            rt.last_bar_ts = last_closed.t;
            if skipped > 0 {
                crate::state::persist_event(
                    app,
                    bot_id,
                    "INFO",
                    "WARMUP",
                    &format!("warmup: {skipped} geçmiş kapalı bar karar için atlandı — bot son kapanmış mumdan devam ediyor"),
                )
                .await;
            }
        }
    }
    let mut processed = 0;
    // Faz 21 (UIX-5 + N-6): REST handler kaynaklı motor eylemleri (grid
    // CLOSE_ALL → halt_mesh + market kapanış) bar beklemeden, HER poll'da ve
    // FROZEN/PAUSED/STOPPED'ta bile akıtılır — bunlar risk-AZALTAN eylemlerdir
    // (iptal + kapanış); strateji kilidi onları tutmaz. ts = son kapanmış bar
    // (Gate 4 no-look-ahead uyumu; piyasa emri sonraki bar açılışında dolar).
    if !rt.pending_engine_actions.is_empty() {
        let ts_act = rt.last_bar_ts.max(1);
        let pending_acts = std::mem::take(&mut rt.pending_engine_actions);
        for a in pending_acts {
            dispatch_action(app, rt, a, ts_act, sse).await?;
        }
    }
    // P2-2 Freeze (+ Faz 0.2/0.3 global şalter) ve Faz 3 (H-10 kalıntısı):
    // strateji kararı işlemez (yeni safety/TP/deal yok, webhook force-start
    // yok) — ama imleç ilerler ve aşağıdaki LIVE fill sweep borsada gerçekleşen
    // dolumları kayda almaya devam eder. PAUSED/STOPPED botta da açık emirlerin
    // dolum takibi sürer (sahipsiz fill kalmaz); yalnız yeni risk üretilmez.
    let strategy_blocked = matches!(rt.status.as_str(), "FROZEN" | "PAUSED" | "STOPPED") || live_halted;
    if strategy_blocked {
        for b in &bars {
            if b.t > rt.last_bar_ts && b.t + interval_ms <= now {
                rt.last_bar_ts = b.t;
            }
        }
    } else {
        for b in &bars {
            if b.t > rt.last_bar_ts && b.t + interval_ms <= now {
                let bar = Bar { t: b.t, o: b.o.clone(), h: b.h.clone(), l: b.l.clone(), c: b.c.clone(), v: b.v.clone() };
                process_bar(app, rt, &bar, sse, &pre).await?;
                rt.last_bar_ts = b.t;
                processed += 1;
            }
        }
        // Immediate webhook force-start at the freshest price — 3Commas opens the
        // deal on signal arrival, not at the next bar close.
        if rt.webhook_start_requested {
            let lastk = bars.last();
            force_start_if_requested(app, rt, lastk, sse).await?;
        }
    }
    // Faz 3 (C-4): kalıcı runtime snapshot — motor fazı + stepper muhasebesi +
    // imleç her poll sonunda diske yazılır; restart tam durumla devam eder.
    crate::state::save_runtime_state(app, bot_id, rt);
    // Faz 13 dilim 2: Fleet/Özet anlık görüntüsünü tazele.
    {
        let snap_v = rt.snapshot_json();
        app.ui_cache.lock().await.insert(bot_id.to_string(), snap_v);
    }

    // Faz 3 (C-3): execution-kaynaklı LIVE fill uzlaştırması.
    if rt.mode == "LIVE" {
        if let Some(creds) = rt.credentials.clone() {
            match live_fill_sweep(app, rt, adapter, &creds, sse).await {
                Ok(n) if n > 0 => {
                    crate::state::persist_event(app, bot_id, "INFO", "LIVE_SYNC", &format!("LIVE fill sweep: {n} venue fill(s) ingested")).await;
                }
                Ok(_) => {}
                Err(e) => {
                    crate::state::persist_event(app, bot_id, "WARN", "LIVE_SYNC", &format!("LIVE fill sweep atlandı: {e}")).await;
                }
            }
        }
    }
    // Faz 13A-POLL: borçları bitir — LIVE intent cascade'i KİLİTSİZ koşar.
    let _ = rt;
    drop(rts);
    run_intent_cascade(app, bot_id, sse).await?;
    Ok(processed)
}

/// Faz 4 (H-5): venue kanıtlı tasfiye kapanışı — yerelde pozisyon var ama
/// venue'da yok olduğu durumda yerel aynayı dürüstçe kapatır (venue bilgisidir;
/// fiyat son mark/avg yaklaşık şeyidir ve raporda "tahmini" etiketlenir).
async fn close_position_as_venue_liquidated(
    app: &AppState,
    rt: &mut BotRuntime,
    px: &Fraction,
    ts: i64,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<(), String> {
    let cancelled = rt.stepper.cancel_pending();
    let bot_id = rt.bot_id.clone();
    if px.is_zero() {
        // Fiyat referansı bile yoksa aynayı sıfırlamadan bırakma — WARN ile
        // operatöre bırak (kapanış fiyatı olmadan muhasebe uydurulmaz).
        crate::state::persist_event(app, &bot_id, "CRITICAL", "LIQUIDATION", "venue pozisyonu yok oldu ama kapanış fiyatı referansı yok — manuel uzlaştırma gerekli").await;
        return Ok(());
    }
    // Faz 13B: venue flat — hedge bacağın kernel defteri de kapatılır.
    if let Some(hs) = rt.hedge_stepper.as_mut() {
        if !hs.position.qty.is_zero() {
            if let Ok(Some(hfill)) = hs.force_close(px, ts, "LIQ") {
                let _ = persist_fill(app, &bot_id, &hfill).await;
            }
        }
    }
    if let Some(fill) = rt.stepper.force_close(px, ts, "LIQ").map_err(|e| e.to_string())? {
        let acts = rt.engine_kind.on_fill(&fill);
        persist_fill(app, &bot_id, &fill).await?;
        let _ = sse.send(SseEvent {
            event: "LIQUIDATION".into(),
            data: json!({
                "botId": bot_id,
                "liqPrice": px.to_string(),
                "qty": fill.qty.to_string(),
                "cancelledOrders": cancelled,
                "venueConfirmed": true,
                "priceApprox": true,
                "ts": ts,
            })
            .to_string(),
        });
        crate::state::persist_event(
            app,
            &bot_id,
            "CRITICAL",
            "LIQUIDATION",
            "venue pozisyonu yok oldu — yerel ayna venue bilgisiyle kapatıldı (kapanış fiyatı son mark/avg yaklaşığı)",
        )
        .await;
        if let dca_strategy::EngineTag::Dca(e) = &mut rt.engine_kind {
            e.force_liquidate();
        }
        if let dca_strategy::EngineTag::Hedge(h) = &mut rt.engine_kind {
            h.main.force_liquidate();
            if let Some(hg) = &mut h.hedge {
                hg.force_liquidate();
            }
        }
        for a in acts {
            Box::pin(dispatch_action(app, rt, a, ts, sse)).await?;
        }
    }
    Ok(())
}

/// Faz 3 (C-3) — execution-kaynaklı LIVE fill uzlaştırması.
///
/// İLKELER (sentetik-fill dönemi kapandı):
/// 1) Fill kanıtının birincil kaynağı venue execution kayıtlarıdır; her kayıt
///    kendi `exec_id`'siyle idempotent işlenir (fill_key = order_id-exec_id).
/// 2) Cancelled / Rejected emir fill ÜRETMEZ — takip kaydı dürüstçe kapanır.
/// 3) Execution API'sini sunmayan venue'da terminal `Filled` durumu + gerçek
///    `cumExecQty`/`avgPrice`/`cumExecFee` toplanarak TEK fallback fill üretilir
///    (yalnız o emir için hiç fill yoksa — çift muhasebe engeli).
/// 4) Emir venue'da hiç bulunamazsa ve execution kanıtı yoksa fill ÜRETİLMEZ
///    (fail-closed) — eski "kaybolan emir = doldu" varsayımı kaldırıldı.
/// 5) Ücret venue'dan geliyorsa gerçek tutar kullanılır (H-4 kısmi); limit
///    dolumları Maker, market dolumları Taker etiketlenir.
async fn live_fill_sweep(
    app: &AppState,
    rt: &mut BotRuntime,
    adapter: &dyn VenueAdapter,
    creds: &dca_exchange::VenueCreds,
    sse: &broadcast::Sender<SseEvent>,
) -> Result<usize, String> {
    if rt.live_open_orders.is_empty() {
        return Ok(0);
    }
    let category = if rt.cfg.kind.is_futures() { "linear" } else { "spot" };
    // Faz 23 (derin inceleme): poll başına en fazla SWEEP_MAX_ORDERS emir —
    // runtimes kilidi bu fonksiyon boyunca tutulur (ağ çağrıları dâhil);
    // sınırsız grid mesh'i kilidi dakikalarca tutabilirdi. Dönen pencere
    // kalan emirleri sonraki poll'larda tarar.
    const SWEEP_MAX_ORDERS: usize = 20;
    let all_tracked = rt.live_open_orders.clone();
    let tracked: Vec<crate::state::LiveTrackedOrder> = if all_tracked.len() <= SWEEP_MAX_ORDERS {
        all_tracked
    } else {
        let start = rt.sweep_cursor % all_tracked.len();
        (0..SWEEP_MAX_ORDERS)
            .map(|i| all_tracked[(start + i) % all_tracked.len()].clone())
            .collect()
    };
    rt.sweep_cursor = rt.sweep_cursor.wrapping_add(tracked.len());
    let mut n = 0usize;
    for order in &tracked {
        // (1) Venue execution kayıtları — artımlı, idempotent.
        let execs = match adapter.executions(creds, category, &rt.cfg.symbol, &order.id).await {
            Ok(v) => v,
            Err(e) => {
                crate::state::persist_event(app, &rt.bot_id, "WARN", "LIVE_SYNC", &format!("execution kayıtları alınamadı ({}): {e} — sonraki sweep yeniden deneyecek", order.id)).await;
                continue;
            }
        };
        for ex in execs {
            if ex.exec_qty.is_zero() {
                continue;
            }
            let fill_key = format!("{}-{}", ex.order_link_id, ex.exec_id);
            if fill_exists(app, &rt.bot_id, &fill_key).await.unwrap_or(false) {
                continue;
            }
            let fee_role = if ex.is_maker { dca_kernel::FeeRole::Maker } else { dca_kernel::FeeRole::Taker };
            let realism = if ex.is_maker { FillRealism::RestingTouch } else { FillRealism::MarketableTaker };
            let ts = if ex.exec_time > 0 { ex.exec_time } else { crate::state::now_ms() };
            // Faz 13B: dolum, order-id prefix'ine göre DOĞRU bacak kernel'ine.
            let hedge_prefix = format!("{}-hedge", rt.bot_id);
            let ingest_target = if order.id.starts_with(&hedge_prefix) {
                rt.hedge_stepper.as_mut()
            } else {
                Some(&mut rt.stepper)
            };
            match ingest_target
                .ok_or("hedge stepper yok")?
                .ingest_external_fill_with_fee(&order.id, &order.role, order.side, order.kind.clone(), &ex.exec_qty, &ex.exec_price, fee_role, realism, ts, ex.exec_fee.as_ref(), Some(&fill_key))
            {
                Ok(Some(f)) => {
                    let acts = rt.engine_kind.on_fill(&f);
                    persist_fill(app, &rt.bot_id, &f).await?;
                    let _ = sse.send(SseEvent {
                        event: "FILL".into(),
                        data: json!({
                            "botId": rt.bot_id,
                            "fillKey": f.fill_key,
                            "orderId": f.order_id,
                            "role": f.role,
                            "side": format!("{:?}", f.side),
                            "kind": format!("{:?}", f.kind),
                            "price": f.price.to_string(),
                            "qty": f.qty.to_string(),
                            "fee": f.fee.to_string(),
                            "feeRole": format!("{}", f.fee_role),
                            "fillModel": format!("{:?}", f.realism),
                            "ts": f.ts,
                        })
                        .to_string(),
                    });
                    for a in acts {
                        Box::pin(dispatch_action(app, rt, a, ts, sse)).await?;
                    }
                    n += 1;
                }
                Ok(None) => {
                    // Çift/kırpılmış ingest — idempotent tekrar, sessiz geçmesi
                    // DOĞRU davranış (fill zaten defterde).
                }
                Err(e) => {
                    // Faz 21 (N-3): sweep tarafında da yutulan dolum yok —
                    // CRITICAL olay + emir kitapta KALIR; bir sonraki sweep
                    // aynı execId anahtarıyla yeniden dener (kitap = kuyruk).
                    let msg = format!(
                        "LIVE dolum sweep'i işlenemedi ({} / {}): {e} — emir kitapta tutuldu, sweep yeniden deneyecek",
                        order.id, ex.order_link_id
                    );
                    crate::state::persist_event(app, &rt.bot_id, "CRITICAL", "FILL_INGEST", &msg).await;
                    let _ = sse.send(SseEvent {
                        event: "RISK".into(),
                        data: json!({"botId": rt.bot_id, "code": "FILL_INGEST_FAILED", "message": msg, "ts": crate::state::now_ms()}).to_string(),
                    });
                }
            }
        }
        // (2) Terminal-durum tespiti: realtime + order-history.
        match adapter.query_order(creds, category, &rt.cfg.symbol, &order.id).await {
            Ok(Some(st)) => {
                let terminal = matches!(st.status.as_str(), "Filled" | "Cancelled" | "Rejected" | "Deactivated");
                if terminal {
                    // Faz 21 (N-3): fallback koşulu "hiç fill yok" DEĞİL —
                    // "KALAN miktar var mı". %40'ı daha önce işlendiyse yalnız
                    // kalan (cum_exec − işlenen) toplanır; executions API'si
                    // boş dönerse kalan artık kaybolmaz.
                    let mut terminal_settled = true;
                    if st.status == "Filled" && !st.cum_exec_qty.is_zero() {
                        let ingested = filled_qty_for_order(app, &rt.bot_id, &order.id).await.unwrap_or_else(|_| Fraction::zero());
                        let remaining = st.cum_exec_qty.sub(&ingested).unwrap_or_else(|_| Fraction::zero());
                        if !remaining.is_zero() && !remaining.is_negative() {
                            let ts = crate::state::now_ms();
                            let px = st.price.clone().unwrap_or_else(|| order.price.clone());
                            // Faz 21 (N-4): 0 fiyatlı ingest m04 DOMAIN hatası
                            // üretir ve dolum yutulur — fiyat yoksa ingest
                            // ETME, CRITICAL yaz, takibi AÇ (fail-closed).
                            if px.is_zero() || px.is_negative() {
                                terminal_settled = false;
                                let msg = format!(
                                    "LIVE fallback dolum için fiyat yok ({}) — kalan {} işlenemedi, emir kitapta tutuldu (fail-closed)",
                                    order.id,
                                    remaining.to_half_even_display(8)
                                );
                                crate::state::persist_event(app, &rt.bot_id, "CRITICAL", "FILL_INGEST", &msg).await;
                                let _ = sse.send(SseEvent {
                                    event: "RISK".into(),
                                    data: json!({"botId": rt.bot_id, "code": "FILL_INGEST_FAILED", "message": msg, "ts": ts}).to_string(),
                                });
                            } else {
                                let fee_role = if st.status == "Filled" { dca_kernel::FeeRole::Taker } else { dca_kernel::FeeRole::Maker };
                                let hedge_prefix_fb = format!("{}-hedge", rt.bot_id);
                                let fb_target = if order.id.starts_with(&hedge_prefix_fb) {
                                    rt.hedge_stepper.as_mut()
                                } else {
                                    Some(&mut rt.stepper)
                                };
                                let fb_ingest = fb_target
                                    .ok_or("hedge stepper yok")?
                                    .ingest_external_fill_with_fee(&order.id, &order.role, order.side, order.kind.clone(), &remaining, &px, fee_role, FillRealism::MarketableTaker, ts, st.cum_exec_fee.as_ref(), None);
                                match fb_ingest {
                                    Ok(Some(f)) => {
                                        let acts = rt.engine_kind.on_fill(&f);
                                        persist_fill(app, &rt.bot_id, &f).await?;
                                        let _ = sse.send(SseEvent {
                                            event: "FILL".into(),
                                            data: json!({
                                                "botId": rt.bot_id, "fillKey": f.fill_key, "orderId": f.order_id,
                                                "role": f.role, "side": format!("{:?}", f.side), "kind": format!("{:?}", f.kind),
                                                "price": f.price.to_string(), "qty": f.qty.to_string(), "fee": f.fee.to_string(),
                                                "feeRole": format!("{}", f.fee_role), "fillModel": format!("{:?}", f.realism), "ts": f.ts,
                                            })
                                            .to_string(),
                                        });
                                        for a in acts {
                                            Box::pin(dispatch_action(app, rt, a, ts, sse)).await?;
                                        }
                                        n += 1;
                                    }
                                    Ok(None) => {}
                                    Err(e) => {
                                        // Faz 21 (N-3): sessiz yutma yok — CRITICAL
                                        // + takip AÇIK kalır (sonraki sweep yeniden dener).
                                        terminal_settled = false;
                                        let msg = format!(
                                            "LIVE fallback dolum işlenemedi ({}): {e} — emir kitapta tutuldu",
                                            order.id
                                        );
                                        crate::state::persist_event(app, &rt.bot_id, "CRITICAL", "FILL_INGEST", &msg).await;
                                        let _ = sse.send(SseEvent {
                                            event: "RISK".into(),
                                            data: json!({"botId": rt.bot_id, "code": "FILL_INGEST_FAILED", "message": msg, "ts": ts}).to_string(),
                                        });
                                    }
                                }
                            }
                        }
                    }
                    if st.status != "Filled" {
                        crate::state::persist_event(app, &rt.bot_id, "INFO", "LIVE_SYNC", &format!("LIVE emir terminal duruma düştü (fill ÜRETİLMEDİ): {} → {}", order.id, st.status)).await;
                    }
                    // Faz 4.5 (C-6): terminal emrin açık-emir yükü defterden düşer.
                    // Faz 21 (N-3): ingest başarısızsa takip AÇILMAZ — kalan
                    // sonraki sweep'te yeniden denenir.
                    if terminal_settled {
                        {
                            let n = order.qty.mul(&order.price).unwrap_or_else(|_| Fraction::zero());
                            let cost = if order.kind == OrderKind::Add && !rt.stepper.cfg.is_spot {
                                let lev = if rt.stepper.cfg.leverage.is_zero() { Fraction::one() } else { rt.stepper.cfg.leverage.clone() };
                                n.div(&lev).unwrap_or(n)
                            } else { n };
                            crate::state::bump_venue_live_load(app, &rt.venue, &cost, false).await;
                        }
                        rt.live_open_orders.retain(|o| o.id != order.id);
                        crate::state::remove_live_order(app, &rt.bot_id, &order.id);
                    }
                }
                // Açık/Partial: tracking kalır — executions artımlı işlenmeye devam eder.
            }
            Ok(None) => {
                // realtime + history'te yok, execution kanıtı yok → fill ÜRETME.
                {
                    let n = order.qty.mul(&order.price).unwrap_or_else(|_| Fraction::zero());
                    let cost = if order.kind == OrderKind::Add && !rt.stepper.cfg.is_spot {
                        let lev = if rt.stepper.cfg.leverage.is_zero() { Fraction::one() } else { rt.stepper.cfg.leverage.clone() };
                        n.div(&lev).unwrap_or(n)
                    } else { n };
                    crate::state::bump_venue_live_load(app, &rt.venue, &cost, false).await;
                }
                rt.live_open_orders.retain(|o| o.id != order.id);
                crate::state::remove_live_order(app, &rt.bot_id, &order.id);
                crate::state::persist_event(app, &rt.bot_id, "WARN", "LIVE_SYNC", &format!("LIVE emir venue'da bulunamadı ve execution kanıtı yok ({}): fill ÜRETİLMEDİ (fail-closed), takip kapatıldı", order.id)).await;
            }
            Err(e) => {
                crate::state::persist_event(app, &rt.bot_id, "WARN", "LIVE_SYNC", &format!("emir durumu sorgulanamadı ({}): {e} — sonraki sweep yeniden deneyecek", order.id)).await;
            }
        }
    }
    Ok(n)
}

/// Faz 3 (C-3): bu emir için daha önce HERHANGİ bir fill işlendi mi?
/// (fallback toplan fill'in çift muhasebesini engeller)
async fn filled_qty_for_order(app: &AppState, bot_id: &str, order_id: &str) -> Result<Fraction, String> {
    let conn = app.db.conn.lock().map_err(|e| e.to_string())?;
    let rows: Vec<String> = {
        let mut stmt = conn
            .prepare("SELECT qty FROM fills WHERE bot_id=?1 AND order_id=?2")
            .map_err(|e| e.to_string())?;
        let list = stmt
            .query_map(rusqlite::params![bot_id, order_id], |r| r.get::<_, String>(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        list
    };
    let mut sum = Fraction::zero();
    for q in rows {
        if let Ok(v) = Fraction::from_any_str(&q) {
            sum = sum.add(&v).unwrap_or_else(|_| sum.clone());
        }
    }
    Ok(sum)
}

/// Parse interval string ("1m", "1h"...) to the Bybit interval token.
/// Unknown intervals are an explicit error — silently pretending "1h" would
/// run the bot on a timeframe it was never configured for.
pub fn bybit_interval(interval: &str) -> Result<String, String> {
    match interval {
        "1m" => Ok("1"),
        "3m" => Ok("3"),
        "5m" => Ok("5"),
        "15m" => Ok("15"),
        "30m" => Ok("30"),
        "1h" => Ok("60"),
        "2h" => Ok("120"),
        "4h" => Ok("240"),
        "1d" => Ok("D"),
        other => Err(format!(
            "desteklenmeyen bar aralığı: {other} — geçerli aralıklar: 1m, 3m, 5m, 15m, 30m, 1h, 2h, 4h, 1d"
        )),
    }
    .map(Into::into)
}

/// Structural interval validation for bot creation (422-worthy).
pub fn validate_interval(interval: &str) -> Result<(), String> {
    bybit_interval(interval).map(|_| ())
}

/* ------------------------------------------------------------------ */
/* Composite signal engine (3Commas-style entry conditions) — live     */
/* ------------------------------------------------------------------ */

pub fn iv_str_ms(iv: &str) -> Option<i64> {
    Some(match iv {
        "1m" => 60_000,
        "3m" => 180_000,
        "5m" => 300_000,
        "15m" => 900_000,
        "30m" => 1_800_000,
        "1h" => 3_600_000,
        "2h" => 7_200_000,
        "4h" => 14_400_000,
        "1d" => 86_400_000,
        _ => return None,
    })
}

pub fn tf_iv_str(tf: Timeframe) -> &'static str {
    match tf {
        Timeframe::M1 => "1m",
        Timeframe::M5 => "5m",
        Timeframe::M15 => "15m",
        Timeframe::H1 => "1h",
        Timeframe::H4 => "4h",
        Timeframe::D1 => "1d",
    }
}

fn klines_to_signal(kl: &[Kline]) -> Vec<BarData> {
    kl.iter()
        .map(|b| BarData {
            t: b.t,
            o: b.o.parse().unwrap_or(f64::NAN),
            h: b.h.parse().unwrap_or(f64::NAN),
            l: b.l.parse().unwrap_or(f64::NAN),
            c: b.c.parse().unwrap_or(f64::NAN),
            v: b.v.parse().unwrap_or(0.0),
        })
        .collect()
}

/// Task 19 — Pine strateji için KAPALI barları çek (oluşan bar düşürülür —
/// zero look-ahead) ve kernel Bar tipine çevir.
async fn fetch_closed_bars(
    app: &AppState,
    venue: &str,
    category: &str,
    symbol: &str,
    bot_iv: &str,
) -> Result<Vec<Bar>, String> {
    let ms = iv_str_ms(bot_iv).ok_or_else(|| format!("desteklenmeyen bot aralığı: {bot_iv}"))?;
    let adapter = app
        .adapter(venue)
        .ok_or("no adapter")?;
    let iv = bybit_interval(bot_iv).map_err(|e| e.to_string())?;
    let kl: Vec<Kline> = adapter.klines(category, symbol, &iv, 300).await.map_err(|e| e.to_string())?;
    let now = crate::state::now_ms();
    Ok(kl
        .into_iter()
        .filter(|b| b.t + ms <= now)
        .map(|k| Bar { t: k.t, o: k.o, h: k.h, l: k.l, c: k.c, v: k.v })
        .collect())
}

/// Fetch CLOSED bars (forming bar dropped — zero look-ahead) for every
/// timeframe referenced by the tree and precompute the indicator engine.
pub async fn build_live_signal_engine(
    app: &AppState,
    venue: &str,
    category: &str,
    symbol: &str,
    bot_iv: &str,
    root: &dca_signals::SignalNode,
) -> Result<(SignalEngine, i64), String> {
    dca_signals::validate_signal_node(root)?;
    let bot_ms = iv_str_ms(bot_iv).ok_or_else(|| format!("desteklenmeyen bot aralığı: {bot_iv}"))?;
    let bot_tf = dca_strategy::backtest::tf_from_ms(bot_ms)
        .ok_or_else(|| format!("desteklenmeyen bar aralığı ({bot_ms} ms)"))?;
    let mut tfs: Vec<Timeframe> = SignalEngine::timeframes_needed(root);
    tfs.retain(|t| *t != bot_tf);
    for t in &tfs {
        if dca_strategy::backtest::tf_ms(*t) < bot_ms {
            return Err(format!("sinyal zaman dilimi {t:?} bot barından küçük olamaz (zero look-ahead)"));
        }
    }
    let adapter = app
        .adapter(venue)
        .ok_or("no adapter")?;
    let now = crate::state::now_ms();
    let mut series: Vec<(Timeframe, Vec<BarData>)> = Vec::new();
    let mut asof = 0i64;
    for tf in std::iter::once(bot_tf).chain(tfs.into_iter()) {
        let iv = bybit_interval(tf_iv_str(tf)).map_err(|e| e.to_string())?;
        let kl: Vec<Kline> = adapter
            .klines(category, symbol, &iv, 300)
            .await
            .map_err(|e| e.to_string())?;
        let ms = dca_strategy::backtest::tf_ms(tf);
        let closed: Vec<Kline> = kl.into_iter().filter(|b| b.t + ms <= now).collect();
        if tf == bot_tf {
            asof = closed.last().map(|b| b.t).unwrap_or(0);
        }
        series.push((tf, klines_to_signal(&closed)));
    }
    let specs = SignalEngine::specs_needed(root);
    Ok((SignalEngine::new(&series, &specs), asof))
}

/// POST /api/signals/preview — evaluate a condition tree on live market data
/// and return per-leaf reports (values, met, warm-up) for the UI builder.
pub async fn signal_preview(
    app: &Arc<AppState>,
    symbol: &str,
    interval: &str,
    venue: Option<&str>,
    category: Option<&str>,
    root: &dca_signals::SignalNode,
) -> Result<serde_json::Value, String> {
    let venue = venue.unwrap_or("BYBIT");
    let category = category.unwrap_or("spot");
    let (eng, asof) = build_live_signal_engine(app, venue, category, symbol, interval, root).await?;
    let leaves = eng.inspect(root, asof);
    let met = eng.evaluate_at_time(root, asof);
    Ok(json!({ "met": met, "asof": asof, "leaves": leaves }))
}

#[allow(dead_code)]
pub fn default_instrument(kind: &StrategyKind, symbol: &str) -> Instrument {
    let mut i = Instrument::default();
    i.symbol = symbol.to_string();
    if kind.is_futures() {
        i.tick_size = Fraction::from_decimal_str("0.1").unwrap();
        i.step_size = Fraction::from_decimal_str("0.001").unwrap();
        i.maker_fee_rate = Fraction::from_decimal_str("0.0002").unwrap();
        i.taker_fee_rate = Fraction::from_decimal_str("0.00055").unwrap();
    }
    i
}

#[allow(dead_code)]
pub fn direction_of(cfg: &BotConfig) -> i8 {
    match cfg.kind.direction() {
        Some(Direction::Long) | None => 1,
        Some(Direction::Short) => -1,
    }
}

#[allow(dead_code)]
pub fn phase_str(rt: &BotRuntime) -> String {
    match &rt.engine_kind {
        dca_strategy::EngineTag::Dca(e) => format!("{:?}", e.phase),
        dca_strategy::EngineTag::Grid(_) => "Running".into(),
        dca_strategy::EngineTag::Hedge(h) => format!("{:?}", h.phase),
    }
}

#[allow(dead_code)]
pub fn side_str(s: &Side) -> &'static str {
    match s {
        Side::Buy => "Buy",
        Side::Sell => "Sell",
    }
}

#[allow(dead_code)]
pub fn kind_label(k: &StrategyKind) -> &'static str {
    k.label()
}

#[cfg(test)]
mod interval_tests {
    use super::*;

    /// Task 15-a item 12: known intervals map to their Bybit tokens; unknown
    /// intervals surface an explicit error (never a silent "60").
    #[test]
    fn bybit_interval_maps_known_and_rejects_unknown() {
        assert_eq!(bybit_interval("1m").unwrap(), "1");
        assert_eq!(bybit_interval("5m").unwrap(), "5");
        assert_eq!(bybit_interval("15m").unwrap(), "15");
        assert_eq!(bybit_interval("1h").unwrap(), "60");
        assert_eq!(bybit_interval("4h").unwrap(), "240");
        assert_eq!(bybit_interval("1d").unwrap(), "D");
        assert!(validate_interval("1h").is_ok());
        let err = bybit_interval("45m").unwrap_err();
        assert!(err.contains("desteklenmeyen bar aralığı"), "got: {err}");
        assert!(validate_interval("weekly").is_err());
    }
}
