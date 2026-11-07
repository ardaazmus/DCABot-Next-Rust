//! Filo kokpiti / executive overview (v3 `GET /api/fleet/overview` karsiligi - Faz 1.4).
//!
//! v3 bilesen eslemesi (durustluk notlariyla):
//! - portfolio NAV/brut-net kaldirac/gerceklesen/unrealized/kazanc-orani/PF +
//!   bot/deal sayaclari -> Fraction ile birebir (Decimal yerine rasyonel
//!   cekirdek; para matematigi exact, oranlar 2dp HALF_EVEN display).
//! - net = realized_gross - cumulative_fees - cumulative_funding (v3 `netOf`
//!   aynen: realizedGross - feePaid - fundingPaid).
//! - allocation (venue tahsis matrisi) -> bots tablosu start_cash + calisan
//!   runtime snapshot'lari; venue RTT sondasi 2sn timeout, basarisiz -> null.
//! - macro -> ayni ic makro takvim (`macro_calendar.rs`; haber servisiyle
//!   paylasilan kara pencere, yeniden icat edilmedi).
//! - sentiment -> Fear&Greed (2.5sn timeout, hata -> null) + mevcut haber
//!   servisi agregasyonu (`get_market_news`, 5dk onbellekli).
//! - tape -> fills tablosu son 40; openDeals -> deal_id'si Some olan
//!   runtime'lar (son 20, snapshot sirasi - acilis zamani defteri yok).
//! - mark -> stepper.last_close (yerel son bar kapanisi); mark yoksa deal
//!   unrealized/margin hesaplarina GIRMEZ (v3 kurali aynen, sahte mark yok).
//!
//! TASINAMAYANLAR (hukme uygun, uydurulmadi):
//! - 24s/7g gerceklesen pencereleri: v3 `deals.closedAt` defterinden gelir;
//!   bizde deals tablosuna yazi YOK (sadece fills persist edilir) -> bu iki
//!   alan `null` + gerekceli not doner; yalniz tum-zaman neti vardir.
//! - profitFactor: trade-seviyesi kapanis defteri olmadan hesaplanamaz
//!   (bot-seviyesi netlerden PF uretmek yanlis olur) -> `null` + not.
//!   winRate motorun kendi wins/losses sayaclarindan (durust).
//! - STOPPED botlar bellekten duser (restore yalniz RUNNING/PAUSED/FROZEN);
//!   onlarin gercekleseni tabloya yazilmadigindan toplama girmez - not dusuldu.
//!
//! ASLA 500 DEGIL: her bacak ayri korunur (DB kilidi, ag, parse); eksik
//! bacak `null`/gerekceli not olur, yanit hep 200.

use axum::extract::State;
use axum::Json;
use dca_exchange::VenueAdapter;
use dca_math::fraction::Fraction;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::state::{now_ms, AppState};

const VENUES: [&str; 4] = ["BINANCE", "BYBIT", "OKX", "PAPER"];
const FG_TIMEOUT: Duration = Duration::from_millis(2500);
const RTT_TIMEOUT: Duration = Duration::from_secs(2);

/// v3 `netOf`: net = brut - fee - funding (gider-pozitif alanlar).
pub fn net_of(realized_gross: &Fraction, fees: &Fraction, funding: &Fraction) -> Fraction {
    realized_gross
        .sub(fees)
        .and_then(|x| x.sub(funding))
        .unwrap_or_else(|_| Fraction::zero())
}

/// 2dp HALF_EVEN display (v3 `d2` karsiligi).
pub fn d2(v: &Fraction) -> String {
    v.to_half_even_display(2)
}

/// NAV orani -> kaldirac (NAV=0 ise 0; sifira bolme yok).
pub fn leverage_str(notional: &Fraction, nav: &Fraction) -> String {
    if nav.is_zero() {
        return "0.00".into();
    }
    match notional.div(nav) {
        Ok(r) => d2(&r),
        Err(_) => "0.00".into(),
    }
}

/// Kazanc orani % (kapali deal sayaci uzerinden; payda 0 ise 0).
pub fn win_rate_pct(wins: usize, closed: usize) -> String {
    if closed == 0 {
        return "0.00".into();
    }
    match Fraction::from_u64(wins as u64)
        .mul(&Fraction::from_u64(100))
        .and_then(|x| x.div(&Fraction::from_u64(closed as u64)))
    {
        Ok(r) => d2(&r),
        Err(_) => "0.00".into(),
    }
}

async fn probe_rtt_ms(fut: impl std::future::Future<Output = Result<i64, String>>) -> Option<u64> {
    let t = Instant::now();
    match tokio::time::timeout(RTT_TIMEOUT, fut).await {
        Ok(Ok(ts)) if ts > 0 => Some(t.elapsed().as_millis() as u64),
        _ => None,
    }
}

/// alternative.me Fear & Greed (limit=1); her hatada null (fail-soft).
async fn fetch_fear_greed(app: &AppState) -> Option<Value> {
    let resp = tokio::time::timeout(FG_TIMEOUT, app.news_http.get("https://api.alternative.me/fng/?limit=1").send())
        .await
        .ok()?
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let v: Value = resp.json().await.ok()?;
    let row = v.get("data")?.as_array()?.first()?;
    let value: f64 = match row.get("value")? {
        Value::String(s) => s.parse().ok()?,
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    let label = row.get("value_classification")?.as_str()?;
    if !value.is_finite() || label.is_empty() {
        return None;
    }
    Some(json!({"value": value, "label": label}))
}

fn parse_frac(s: &str) -> Fraction {
    Fraction::from_any_str(s).unwrap_or_else(|_| Fraction::zero())
}

pub async fn fleet_overview(State(app): State<Arc<AppState>>) -> Json<Value> {
    // --- Bot tabani: bots tablosu (tum botlar, start_cash + status) + runtime snapshot'lari.
    let db_bots: Vec<(String, String, String, String, String)> = (|| {
        let conn = app.db.conn.lock().ok()?;
        let mut stmt = conn
            .prepare("SELECT id, venue, start_cash, status, name FROM bots")
            .ok()?;
        let rows = stmt
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))
            .ok()?;
        Some(rows.filter_map(|x| x.ok()).collect())
    })()
    .unwrap_or_default();
    let mut nav = Fraction::zero();
    for (_, _, sc, _, _) in &db_bots {
        nav = nav.add(&parse_frac(sc)).unwrap_or_else(|_| nav.clone());
    }
    let nav_pos = !nav.is_zero();

    // --- Runtime snapshot'lari (acik pozisyon + gerceklesen + sayaclar).
    let mut gross_notional = Fraction::zero();
    let mut net_notional = Fraction::zero();
    let mut unrealized = Fraction::zero();
    let mut realized_net_all = Fraction::zero();
    let mut wins_total: usize = 0;
    let mut closed_total: usize = 0;
    let mut active_bots: usize = 0;
    let mut paused_bots: usize = 0;
    let mut stopped_bots: usize = 0;
    let mut open_count: usize = 0;
    // venue -> (total, active, alloc_eq yok burada, open_n, open_notional, unreal, realized_net)
    let mut venue_open_n: HashMap<String, usize> = HashMap::new();
    let mut venue_open_notional: HashMap<String, Fraction> = HashMap::new();
    let mut venue_unreal: HashMap<String, Fraction> = HashMap::new();
    let mut venue_realized: HashMap<String, Fraction> = HashMap::new();
    let mut open_deals: Vec<Value> = Vec::new();

    {
        let rts = app.runtimes.lock().await;
        for rt in rts.values() {
            match rt.status.as_str() {
                "RUNNING" | "FROZEN" => active_bots += 1,
                "PAUSED" => paused_bots += 1,
                _ => stopped_bots += 1,
            }
            let snap = rt.engine_kind.snapshot();
            let fees = rt.stepper.cumulative_fees.clone();
            let funding = rt.stepper.cumulative_funding.clone();
            let gross = parse_frac(&snap.realized_gross);
            let net = net_of(&gross, &fees, &funding);
            realized_net_all = realized_net_all.add(&net).unwrap_or_else(|_| realized_net_all.clone());
            *venue_realized.entry(rt.venue.clone()).or_insert_with(Fraction::zero) = venue_realized
                .get(&rt.venue)
                .cloned()
                .unwrap_or_else(Fraction::zero)
                .add(&net)
                .unwrap_or_else(|_| venue_realized.get(&rt.venue).cloned().unwrap_or_else(Fraction::zero));
            wins_total += snap.wins;
            closed_total += snap.deals_closed;

            if snap.deal_id.is_some() {
                // Mark: yerel son bar kapanisi; yoksa hesaba girmez (v3 kurali).
                if let Some(mark_s) = rt.stepper.last_close.as_deref() {
                    if let Ok(mark) = Fraction::from_any_str(mark_s) {
                        let qty = parse_frac(&snap.position_qty);
                        let notional = qty.mul(&mark).unwrap_or_else(|_| Fraction::zero());
                        if !notional.is_zero() {
                            let is_short = matches!(rt.cfg.kind, dca_strategy::StrategyKind::DcaShort);
                            gross_notional = gross_notional.add(&notional).unwrap_or_else(|_| gross_notional.clone());
                            net_notional = if is_short {
                                net_notional.sub(&notional).unwrap_or_else(|_| net_notional.clone())
                            } else {
                                net_notional.add(&notional).unwrap_or_else(|_| net_notional.clone())
                            };
                            let upnl = rt.stepper.position.unrealized_gross(&mark).unwrap_or_else(|_| Fraction::zero());
                            unrealized = unrealized.add(&upnl).unwrap_or_else(|_| unrealized.clone());
                            *venue_open_n.entry(rt.venue.clone()).or_insert(0) += 1;
                            let vn = venue_open_notional.entry(rt.venue.clone()).or_insert_with(Fraction::zero).clone();
                            venue_open_notional.insert(rt.venue.clone(), vn.add(&notional).unwrap_or(vn));
                            let vu = venue_unreal.entry(rt.venue.clone()).or_insert_with(Fraction::zero).clone();
                            venue_unreal.insert(rt.venue.clone(), vu.add(&upnl).unwrap_or(vu));
                            open_count += 1;
                        }
                        if open_deals.len() < 20 {
                            let upnl = rt.stepper.position.unrealized_gross(&mark).unwrap_or_else(|_| Fraction::zero());
                            open_deals.push(json!({
                                "id": snap.deal_id,
                                "botId": rt.bot_id,
                                "botName": rt.name,
                                "pair": rt.cfg.symbol,
                                "side": if matches!(rt.cfg.kind, dca_strategy::StrategyKind::DcaShort) { "SHORT" } else { "LONG" },
                                "avgPrice": snap.avg_price,
                                "qty": snap.position_qty,
                                "markPrice": mark_s,
                                "unrealized": d2(&upnl),
                                "venue": rt.venue,
                                "safetyFilled": snap.safety_triggered,
                                "maxSafety": rt.cfg.max_safety_orders,
                            }));
                        }
                    }
                } else if open_deals.len() < 20 {
                    // Mark yok: seritte goster ama unrealized null, toplama girmez.
                    open_deals.push(json!({
                        "id": snap.deal_id,
                        "botId": rt.bot_id,
                        "botName": rt.name,
                        "pair": rt.cfg.symbol,
                        "side": if matches!(rt.cfg.kind, dca_strategy::StrategyKind::DcaShort) { "SHORT" } else { "LONG" },
                        "avgPrice": snap.avg_price,
                        "qty": snap.position_qty,
                        "markPrice": Value::Null,
                        "unrealized": Value::Null,
                        "venue": rt.venue,
                        "safetyFilled": snap.safety_triggered,
                        "maxSafety": rt.cfg.max_safety_orders,
                    }));
                }
            }
        }
    }

    let pct_of_nav = |x: &Fraction| -> String {
        if !nav_pos {
            return "0.00".into();
        }
        match x.mul(&Fraction::from_u64(100)).and_then(|y| y.div(&nav)) {
            Ok(r) => d2(&r),
            Err(_) => "0.00".into(),
        }
    };
    // net kaldirac mutlak deger uzerinden (v3 `netLeverage` aynen).
    let net_mag = if net_notional.to_f64_display() < 0.0 {
        net_notional.neg().unwrap_or_else(|_| Fraction::zero())
    } else {
        net_notional.clone()
    };
    let net_lev = leverage_str(&net_mag, &nav);

    let portfolio = json!({
        "navUsd": nav.to_string(),
        "grossLeverage": leverage_str(&gross_notional, &nav),
        "netLeverage": net_lev,
        "realizedPnl24h": Value::Null,
        "realizedPnl7d": Value::Null,
        "realizedPnlAllTime": d2(&realized_net_all),
        "realizedPnl24hPct": Value::Null,
        "realizedPnl7dPct": Value::Null,
        "realizedPnlAllTimePct": pct_of_nav(&realized_net_all),
        "unrealizedPnl": d2(&unrealized),
        "winRatePct": win_rate_pct(wins_total, closed_total),
        "profitFactor": Value::Null,
        "openDeals": open_count,
        "activeBots": active_bots,
        "totalBots": db_bots.len(),
        "pausedBots": paused_bots,
        "stoppedBots": stopped_bots,
    });

    // --- Venue RTT sondalari paralel (2sn timeout; PAPER yerel -> sonda yok).
    let (rtt_binance, rtt_bybit, rtt_okx) = tokio::join!(
        probe_rtt_ms(app.binance.server_time_ms()),
        probe_rtt_ms(app.bybit.server_time_ms()),
        probe_rtt_ms(app.okx.server_time_ms()),
    );
    let rtt_of = |v: &str| -> Option<u64> {
        match v {
            "BINANCE" => rtt_binance,
            "BYBIT" => rtt_bybit,
            "OKX" => rtt_okx,
            _ => None,
        }
    };

    let allocation: Vec<Value> = VENUES
        .iter()
        .map(|v| {
            let bots_here: Vec<&(String, String, String, String, String)> =
                db_bots.iter().filter(|(_, venue, _, _, _)| venue == *v).collect();
            let alloc_eq = bots_here.iter().fold(Fraction::zero(), |acc, (_, _, sc, _, _)| {
                acc.add(&parse_frac(sc)).unwrap_or(acc)
            });
            let rtt = rtt_of(v);
            let open_n = venue_open_n.get(*v).copied().unwrap_or(0);
            let open_not = venue_open_notional.get(*v).cloned().unwrap_or_else(Fraction::zero);
            let margin_util = if alloc_eq.is_zero() {
                "0.00".into()
            } else {
                match open_not.mul(&Fraction::from_u64(100)).and_then(|x| x.div(&alloc_eq)) {
                    Ok(r) => d2(&r),
                    Err(_) => "0.00".into(),
                }
            };
            json!({
                "venue": v,
                "activeBots": bots_here.iter().filter(|(_, _, _, st, _)| st == "RUNNING" || st == "FROZEN").count(),
                "totalBots": bots_here.len(),
                "allocatedEquityUsd": alloc_eq.to_string(),
                "marginUtilizationPct": margin_util,
                "openPositions": open_n,
                "openNotionalUsd": open_not.to_string(),
                "unrealizedPnl": d2(&venue_unreal.get(*v).cloned().unwrap_or_else(Fraction::zero)),
                "realizedPnl": d2(&venue_realized.get(*v).cloned().unwrap_or_else(Fraction::zero)),
                "apiRttMs": rtt,
                "failClosedState": if *v == "PAPER" { "yerel simulasyon - sonda yok" } else if rtt.is_none() { "erisilemiyor (fail-closed)" } else { "hat serbest" },
            })
        })
        .collect();

    // --- Makro: ayni ic takvim (v3 `macroBlackoutState` karsiligi).
    let now_dt = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(now_ms()).unwrap_or_else(chrono::Utc::now);
    let blackout = dca_math::macro_calendar::in_macro_blackout(now_dt);
    let next_ev = dca_math::macro_calendar::event_times()
        .into_iter()
        .find(|(t, _)| *t >= now_dt)
        .map(|(t, label)| {
            let label_tr = match label {
                "FOMC" => "FOMC faiz karari",
                "CPI" => "TUFE (CPI)",
                _ => label,
            };
            json!({"labelTr": label_tr, "label": label, "t": t.to_rfc3339()})
        });
    let macro_state = json!({
        "nextEvent": next_ev,
        "blackoutActive": blackout.is_some(),
        "blackoutLabel": blackout.map(|(l, _)| l),
    });

    // --- Duyarlilik: Fear&Greed (fail-soft) + mevcut haber servisi agregasyonu.
    let (fg, news) = tokio::join!(fetch_fear_greed(&app), super::get_market_news(&app, "BTCUSDT"));
    let news_agg = {
        let count = news.get("count").and_then(|c| c.as_u64()).unwrap_or(0);
        let sources_empty = news.get("sources").and_then(|s| s.as_array()).map(|a| a.is_empty()).unwrap_or(true);
        if count == 0 || sources_empty {
            Value::Null
        } else {
            json!({
                "mean": news.get("mean").cloned().unwrap_or(Value::Null),
                "bullPct": news.get("bull").cloned().unwrap_or(Value::Null),
                "neutralPct": news.get("neutral").cloned().unwrap_or(Value::Null),
                "bearPct": news.get("bear").cloned().unwrap_or(Value::Null),
            })
        }
    };
    let sentiment = json!({ "fearGreed": fg, "newsAggregate": news_agg });

    // --- Tape: tum deal'lerde son 40 fill (bot adi + venue ile).
    let name_of: HashMap<String, (String, String)> = {
        let mut m: HashMap<String, (String, String)> = HashMap::new();
        for (id, venue, _, _, name) in &db_bots {
            m.insert(id.clone(), (name.clone(), venue.clone()));
        }
        m
    };
    let tape: Vec<Value> = (|| {
        let conn = app.db.conn.lock().ok()?;
        let mut stmt = conn
            .prepare("SELECT bot_id, role, side, price, qty, fee, ts FROM fills ORDER BY id DESC LIMIT 40")
            .ok()?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, i64>(6)?,
                ))
            })
            .ok()?;
        let mut out = Vec::new();
        for row in rows.filter_map(|x| x.ok()) {
            let (bot_id, role, side, price, qty, fee, ts) = row;
            let (name, venue) = name_of
                .get(&bot_id)
                .cloned()
                .unwrap_or_else(|| ("bilinmiyor".into(), "bilinmiyor".into()));
            out.push(json!({
                "botId": bot_id, "botName": name, "venue": venue,
                "role": role, "side": side, "price": price, "qty": qty, "fee": fee, "ts": ts,
            }));
        }
        Some(out)
    })()
    .unwrap_or_default();

    Json(json!({
        "ok": true,
        "generatedAt": now_ms(),
        "portfolio": portfolio,
        "allocation": allocation,
        "macro": macro_state,
        "sentiment": sentiment,
        "tape": tape,
        "openDeals": open_deals,
        "notes": [
            "24s/7g gerceklesen: deals kapanis defteri bizde yazilmiyor (yalniz fills persist) -> null; tum-zaman neti bellek snapshot'larindan (realized_gross-fee-funding).",
            "profitFactor: trade-seviyesi kapanis defteri olmadan hesaplanamaz -> null; winRate motor wins/losses sayaclarindan.",
            "STOPPED botlar bellekten duser; onlarin gercekleseni toplama girmez. Mark = yerel son bar kapanisi; mark yoksa deal toplama girmez.",
            "PAPER tahsis satiri v3'te yok (3 venue); bizde PAPER botlar da oldugundan 4. satir olarak eklendi.",
        ],
    }))
}

/// Faz 27 (İ-1): portföy kapısı anlık görüntüsünü TAZELER. Yalnız runtime
/// kilidi ZATEN tutulmuşken çağrılmalı (poll turu başı) — bu fn kendisi
/// runtimes kilidini İSTEMEZ (tokio Mutex reentrant değil; dispatch içinde
/// çağrılan kapı deadlock'ı tam olarak bu yüzden cache desenine gitti).
/// NAV: bots.start_cash toplamı (db kilidi ayrı ve kısa — çakışmaz).
/// Gross: snapshot'lardan qty × son-bar-kapanışı (mark yoksa deal sayılmaz).
pub fn refresh_portfolio_cache(app: &crate::state::AppState, rts: &HashMap<String, crate::state::BotRuntime>) {
    let nav = (|| -> Option<Fraction> {
        let conn = app.db.conn.lock().ok()?;
        let mut stmt = conn.prepare("SELECT start_cash FROM bots").ok()?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0)).ok()?;
        let mut sum = Fraction::zero();
        for row in rows.filter_map(|x| x.ok()) {
            let v = Fraction::from_any_str(&row).unwrap_or_else(|_| Fraction::zero());
            sum = sum.add(&v).unwrap_or_else(|_| sum.clone());
        }
        Some(sum)
    })()
    .unwrap_or_else(Fraction::zero);

    let mut gross = Fraction::zero();
    for rt in rts.values() {
        if rt.status != "RUNNING" && rt.status != "FROZEN" && rt.status != "PAUSED" {
            continue;
        }
        let snap = rt.engine_kind.snapshot();
        if snap.deal_id.is_none() {
            continue;
        }
        if let Some(mark_s) = rt.stepper.last_close.as_deref() {
            if let Ok(mark) = Fraction::from_any_str(mark_s) {
                let qty = parse_frac(&snap.position_qty);
                let notional = qty.mul(&mark).unwrap_or_else(|_| Fraction::zero());
                gross = gross.add(&notional).unwrap_or_else(|_| gross.clone());
            }
        }
    }
    if let Ok(mut slot) = app.portfolio_cache.lock() {
        *slot = Some((crate::state::now_ms(), nav, gross));
    }
}

/// Faz 27 (İ-1): kapının okuma tarafı — tazelik penceresi içindeyse (nav, gross).
pub fn portfolio_cache_read(app: &crate::state::AppState, max_age_ms: i64) -> Option<(Fraction, Fraction)> {
    let slot = app.portfolio_cache.lock().ok()?;
    let (ts, nav, gross) = slot.as_ref()?;
    if crate::state::now_ms() - ts > max_age_ms {
        return None;
    }
    Some((nav.clone(), gross.clone()))
}

/// Faz 27 (İ-1): portföy-düzeyi max-total-notional kapısı — saf çekirdek.
///
/// Kural: (mevcut brüt açık notional + aday emir notional) / NAV × 100,
/// cap_pct'i aşıyorsa emir yerleştirilmez (fail-closed). NAV = bots
/// tablosundaki start_cash toplamı (fleet_overview ile aynı temel —
/// unrealized PnL geri besleme döngüsü yaratmasın diye bilinçli tercih).
///
/// Dürüstlük kuralları:
/// - NAV <= 0 → ölçülebilir temel yok, engelleme yok (create tarafındaki
///   capital-guard zaten NAV'sız bot üretmez; burada çifte kilide gerek yok).
/// - cap_pct <= 0 → kapı kapalı (explicit opt-out).
/// - aday notional <= 0 → engelleme yok (reduce/kapanış sınıfı; savunma).
pub fn portfolio_cap_exceeded(gross: &Fraction, candidate: &Fraction, nav: &Fraction, cap_pct: i64) -> bool {
    if cap_pct <= 0 {
        return false;
    }
    if nav.is_zero() || nav.is_negative() {
        return false;
    }
    if candidate.is_negative() || candidate.is_zero() {
        return false;
    }
    let total = gross.add(candidate).unwrap_or_else(|_| gross.clone());
    // total × 100 > nav × cap_pct — bölmeden rasyonel karşılaştırma.
    match total.mul(&Fraction::from_i64(100)) {
        Ok(lhs) => match nav.mul(&Fraction::from_i64(cap_pct)) {
            Ok(rhs) => lhs.cmp(&rhs) == std::cmp::Ordering::Greater,
            Err(_) => false,
        },
        Err(_) => true, // üst taşma → güvenli tarafta reddet
    }
}

/// Faz 27 (İ-1): kapı eşiği çözümü — env `DCA_PORTFOLIO_MAX_NOTIONAL_PCT`.
/// Unset/geçersiz → 85 (kapı AÇIK varsayılan); "0" → kapalı (explicit opt-out);
/// 1..=1000 arası geçerli; 1000 üstü geçersiz sayılır (85'e düşer).
pub fn portfolio_cap_pct_from_env(raw: Option<&str>) -> i64 {
    match raw {
        None => 85,
        Some(s) => match s.trim().parse::<i64>() {
            Ok(0) => 0,
            Ok(n) if (1..=1000).contains(&n) => n,
            _ => 85,
        },
    }
}

#[cfg(test)]
mod fleet_tests {
    use super::*;

    fn f(s: &str) -> Fraction {
        Fraction::from_decimal_str(s).unwrap()
    }

    #[test]
    fn net_of_matches_v3_formula() {
        // v3: net = realizedGross - feePaid - fundingPaid (rasyonel gosterim
        // integral olmayinca kesir biciminde olabilir; parasal karsilastirma d2 ile).
        assert_eq!(d2(&net_of(&f("10.5"), &f("0.3"), &f("0.2"))), "10.00");
        assert_eq!(d2(&net_of(&f("-2"), &f("0.1"), &f("0"))), "-2.10");
        // funding iadesi (short rebate) neti buyutur
        assert_eq!(d2(&net_of(&f("5"), &f("0.5"), &f("-0.5"))), "5.00");
    }

    #[test]
    fn leverage_zero_nav_is_fail_closed() {
        assert_eq!(leverage_str(&f("100"), &Fraction::zero()), "0.00");
        assert_eq!(leverage_str(&f("50"), &f("100")), "0.50");
    }

    #[test]
    fn win_rate_empty_is_zero() {
        assert_eq!(win_rate_pct(0, 0), "0.00");
        assert_eq!(win_rate_pct(3, 4), "75.00");
    }

    // ---- Faz 27 (İ-1): portföy notional kapısı ----

    #[test]
    fn portfolio_cap_blocks_over_threshold() {
        // gross 7000 + aday 1350 = 8350; NAV 10000, cap %80 → 8350 > 8000 → blok
        assert!(portfolio_cap_exceeded(&f("7000"), &f("1350"), &f("10000"), 80));
        // aday 1000 → 8000 == 8000 → sınırda, engelleme yok (eşitlik serbest)
        assert!(!portfolio_cap_exceeded(&f("7000"), &f("1000"), &f("10000"), 80));
        // altında rahat
        assert!(!portfolio_cap_exceeded(&f("1000"), &f("500"), &f("10000"), 80));
    }

    #[test]
    fn portfolio_cap_edge_and_nasty_numbers() {
        // NAV sıfır → ölçülemez temel, engelleme yok (capital-guard'ın sahası)
        assert!(!portfolio_cap_exceeded(&f("7000"), &f("1000"), &Fraction::zero(), 80));
        assert!(!portfolio_cap_exceeded(&f("7000"), &f("1000"), &f("-100"), 80));
        // cap 0 → explicit opt-out
        assert!(!portfolio_cap_exceeded(&f("99999"), &f("99999"), &f("1"), 0));
        // cap negatif (saldiri girdisi) → opt-out değil, kapatma yorumu yine geçersiz:
        // negatif cap ile "her şey bloklansın" denemez — false döner (kapı kapalı).
        assert!(!portfolio_cap_exceeded(&f("99999"), &f("1"), &f("100"), -50));
        // aday sıfır/negatif → reduce sınıfı, engelleme yok
        assert!(!portfolio_cap_exceeded(&f("7000"), &Fraction::zero(), &f("10000"), 80));
        assert!(!portfolio_cap_exceeded(&f("7000"), &f("-50"), &f("10000"), 80));
        // minik ondalık hassasiyet: gross 79.9999 + 0.0001 = 80.00 (sınır, geçer)
        assert!(!portfolio_cap_exceeded(&f("79.9999"), &f("0.0001"), &f("100"), 80));
        // minik aşım: 79.9999 + 0.0002 → blok
        assert!(portfolio_cap_exceeded(&f("79.9999"), &f("0.0002"), &f("100"), 80));
        // makro: yüz milyonlarca
        assert!(portfolio_cap_exceeded(&f("900000000"), &f("200000000"), &f("1000000000"), 85));
        assert!(!portfolio_cap_exceeded(&f("800000000"), &f("40000000"), &f("1000000000"), 85));
    }

    #[test]
    fn portfolio_cap_env_parsing() {
        assert_eq!(portfolio_cap_pct_from_env(None), 85);
        assert_eq!(portfolio_cap_pct_from_env(Some(" 60 ")), 60);
        assert_eq!(portfolio_cap_pct_from_env(Some("0")), 0);
        assert_eq!(portfolio_cap_pct_from_env(Some("1000")), 1000);
        // geçersizler varsayılana düşer — sessiz kapatma YOK
        assert_eq!(portfolio_cap_pct_from_env(Some("abc")), 85);
        assert_eq!(portfolio_cap_pct_from_env(Some("-5")), 85);
        assert_eq!(portfolio_cap_pct_from_env(Some("2000")), 85);
        assert_eq!(portfolio_cap_pct_from_env(Some("")), 85);
    }
}
