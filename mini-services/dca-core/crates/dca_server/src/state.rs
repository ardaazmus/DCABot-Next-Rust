//! Shared application state: DB, vault, venue adapters, bot runtimes, SSE.

use crate::store::{Db, Vault};
use dca_exchange::{
    binance::BinanceAdapter, bybit::BybitAdapter, okx::OkxAdapter, paper::PaperVenue, VenueAdapter,
    VenueCreds,
};
use dca_kernel::{SimulationStepper, StepperConfig};
use dca_math::fraction::Fraction;
use dca_math::Instrument;
use dca_strategy::{BotConfig, EngineTag, StrategyKind};
use serde_json::json;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::{broadcast, Mutex};

#[derive(Debug, Clone)]
pub struct SseEvent {
    pub event: String,
    pub data: String,
}

pub struct BotRuntime {
    pub bot_id: String,
    pub name: String,
    pub mode: String,  // PAPER | LIVE
    /// Effective exchange endpoint, distinct from the order-execution path.
    /// Values include INTERNAL_PAPER, BYBIT_DEMO, BYBIT_TESTNET, OKX_DEMO,
    /// MAINNET, and UNSUPPORTED.
    pub execution_environment: String,
    pub venue: String, // BYBIT | BINANCE | OKX | PAPER
    pub status: String,
    pub cfg: BotConfig,
    pub interval_str: String,
    pub last_bar_ts: i64,
    pub stepper: SimulationStepper,
    pub engine_kind: EngineTag,
    pub credentials: Option<VenueCreds>,
    /// True once the venue leverage has been pushed for futures bots (LIVE only).
    pub leverage_synced: bool,
    /// Grid engines: false until the start condition passes and mesh orders are placed.
    pub grid_bootstrapped: bool,
    /// TradingView webhook secret (3Commas external signal parity).
    pub webhook_secret: String,
    /// Set by POST /api/webhooks/tradingview; consumed on the next bar pass.
    pub webhook_start_requested: bool,
    /// Last composite signal-tree verdict (None = no entry_signal configured).
    pub entry_signal_met: Option<bool>,
    /// LIVE-mode tracking: resting venue limit orders placed by this bot
    /// (order_link_id + resting params) awaiting fill reconciliation. Kept
    /// OUTSIDE the kernel's `pending` so the paper-fill path never double-fills.
    pub live_open_orders: Vec<LiveTrackedOrder>,
    /// Task 16 — rejim sınıflandırma penceresi (kapalı bar kapanışları).
    pub regime_closes: Vec<f64>,
    /// Task 16 — son rejim kararı (snapshot + UI rozeti).
    pub last_regime: dca_signals::MarketRegime,
    /// Task 16 — günlük DD devre kesici durumu (UTC gün çapası + tetik).
    pub breaker_day: i64,
    pub breaker_day_start_eq: Option<dca_math::fraction::Fraction>,
    pub breaker_tripped: bool,
    /// Task 18 Ş1: bilinmeyen venue için tek seferlik WARN (poll döngüsü spam etmesin).
    pub venue_warned: bool,
    /// Faz 4.5 (C-5): Bybit pozisyon-modu denetimi — DCA ailesi one-way
    /// (positionIdx=0) ile çalışır; hesap hedge-mode'daysa venue emirleri
    /// reddeder. blocked=true iken LIVE Add emirleri gönderilmez.
    pub pos_mode_checked: bool,
    pub pos_mode_blocked: bool,
    /// Faz 13A-POLL: dispatch LIVE dalları ağ yapmaz — intent burada birikir,
    /// poll_inner'ın kilit-dışı cascade fazı çalıştırıp uygular.
    pub pending_live_intents: Vec<LiveIntent>,
    /// Faz 20 (UIX-5): REST handler kaynaklı motor eylemleri (ör. grid
    /// CLOSE_ALL → halt_mesh + market kapanış) — runner bir sonraki barda
    /// motor döngüsüyle AYNI dispatch disiplininden akıtır.
    pub pending_engine_actions: Vec<dca_strategy::Action>,
    /// Faz 22 (P1-6): funding settlement geçmişi cache'i — gerçek mutabakat
    /// penceredeki kayıtlarla yapılır; interval kayıt aralığından çıkarılır.
    pub funding_history: Vec<(i64, String)>,
    /// Son BAŞARILI funding-geçmişi fetch zamanı (0 = hiç).
    pub funding_history_ok_at: i64,
    /// Son fetch DENEMESİ (başarısızlıkta retry throttle'ı).
    pub funding_history_tried_at: i64,
    /// Faz 23: boş-geçmiş uyarısı yalnız bir kez (spam koruması).
    #[allow(dead_code)]
    pub funding_empty_warned: bool,
    /// LIVE sweep dönen-pencere imleci — her poll en fazla 20 emri ağ kilidi
    /// dışında tarar.
    pub sweep_cursor: usize,
    /// Faz 13B: hedge bacağı için AYRI kernel stepper (direction=-1) —
    /// EX-06'nın kalıcı çözümü: iki bacak artık tek kernel pozisyonunda
    /// birleşmez; her bacak kendi cash/IM/fee/pozisyon defterini tutar.
    pub hedge_stepper: Option<SimulationStepper>,
    /// Faz 5 (C-5/EX-07): hesap hedge modundaysa DCA emirleri positionIdx
    /// eşlemesiyle çalışır (Buy→1 long bacası, Sell→2 short bacası) — mod
    /// uyuşmazlığı artık engel değil, eşlemedir.
    pub pos_mode_hedge: bool,
    /// Faz 4 (H-5): venue bakım-marjı kademeleri (risk-limit aynası) —
    /// TTL'li cache; None/eski → poll yeniden çeker. Yerel likidasyon tahmini
    /// sabit 0.005 MMR yerine gerçek kademelerle hesaplanır.
    pub mmr_brackets: Option<Vec<dca_math::MmrBracket>>,
    pub mmr_brackets_at: i64,
    /// Faz 0.2/0.3 — global şalter WARN'i tek seferlik (poll spam'i yok).
    pub halt_warned: bool,
}

/// Faz 0.2/0.3 — yürütme hattı şalteri (v3 `lib/engine/killswitch.ts` karşılığı).
/// Engage → yürütme durur; açıklar korunur (fill sweep sürer). Kalıcılık:
/// her değişim append-only `events` akışına yazılır, restart'ta son olaydan
/// geri yüklenir (v3 `getKillSwitchState` ile aynı sözleşme).
#[derive(Debug, Clone, Default)]
pub struct HaltState {
    pub active: bool,
    pub reason: Option<String>,
    pub by: Option<String>,
    pub at_ms: i64,
    pub auto: bool,
}

/// `events` tablosundaki kapı adları.
pub const GATE_KILL: &str = "KILL_SWITCH";
pub const GATE_FREEZE: &str = "FREEZE";

/// Halt olay mesajı (JSON) — `parse_halt_message` ile eşleşmeli.
pub fn halt_event_message(
    action: &str,
    reason: Option<&str>,
    by: Option<&str>,
    at_ms: i64,
    auto: bool,
) -> String {
    serde_json::json!({"action": action, "reason": reason, "by": by, "at_ms": at_ms, "auto": auto})
        .to_string()
}

/// Halt mesajını çözer → (action, reason, by, at_ms, auto). Bozuk kayıt = None
/// (fail-closed: bozuk olay şalteri açmaz, restart varsayılanı kapalıdır).
pub fn parse_halt_message(
    msg: &str,
) -> Option<(String, Option<String>, Option<String>, i64, bool)> {
    let v: serde_json::Value = serde_json::from_str(msg).ok()?;
    let action = v.get("action")?.as_str()?.to_string();
    if action != "engaged" && action != "released" {
        return None;
    }
    let reason = v
        .get("reason")
        .and_then(|r| r.as_str())
        .map(|s| s.to_string());
    let by = v.get("by").and_then(|r| r.as_str()).map(|s| s.to_string());
    let at_ms = v.get("at_ms").and_then(|n| n.as_i64()).unwrap_or(0);
    let auto = v.get("auto").and_then(|b| b.as_bool()).unwrap_or(false);
    Some((action, reason, by, at_ms, auto))
}

/// Yürütme-durdurma karar matrisi (saf — birim testli):
/// kill tüm modlarda TV ENTRY dahil yürütmeyi durdurur (v3: paper tick sürer
/// ama TV ENTRY reddedilir); freeze yalnız LIVE hattı durdurur.
/// PAPER poll döngüsü her iki şalterde de sürer.
#[allow(dead_code)]
pub fn live_halted(mode: &str, kill: bool, freeze: bool) -> bool {
    mode == "LIVE" && (kill || freeze)
}

/// Faz 13A-POLL: dispatch LIVE dallarının ürettiği ağ niyeti — poll_inner'ın
/// kilit-dışı cascade fazı çalıştırır ve sonuçları kısa kilitte uygular.
pub enum LiveIntent {
    Place {
        req: dca_exchange::OrderRequest,
        id: String,
        side: dca_math::Side,
        kind: dca_kernel::OrderKind,
        price: Option<Fraction>,
        qty: Fraction,
        role: String,
        is_close: bool,
        gate: GateCtx,
    },
    Cancel {
        links: Vec<String>,
        venue: String,
        category: String,
        symbol: String,
    },
}

impl LiveIntent {
    #[allow(dead_code)]
    pub fn id(&self) -> &str {
        match self {
            LiveIntent::Place { id, .. } => id,
            LiveIntent::Cancel { .. } => "",
        }
    }
}

/// Faz 13A-POLL: pre_order_gate'in rt'siz girdisi — intent kuruluşunda
/// (kilit altı, ucuza) kopyalanır; wallet çağrısı execute'ta kilitsiz yapılır.
#[derive(Clone)]
pub struct GateCtx {
    pub pos_mode_blocked: bool,
    pub last_close: Option<Fraction>,
    pub leverage: Fraction,
    pub is_spot: bool,
    pub quote: String,
    pub min_notional: Fraction,
    pub venue: String,
}

/// Faz 13A-POLL: execute fazının ürettiği sonuç — apply fazı kilit altında tüketir.
pub enum IntentOutcome {
    Placed {
        id: String,
        side: dca_math::Side,
        kind: dca_kernel::OrderKind,
        price: Option<Fraction>,
        qty: Fraction,
        role: String,
    },
    /// Faz 18 (P0-4): dolum kimliği borsa execId'sidir (`fill_key`) — yerel
    /// `now_ms` anahtarı retry/çift cascade turunda çift sayıma açıktı.
    MarketFilled {
        id: String,
        side: dca_math::Side,
        kind: dca_kernel::OrderKind,
        role: String,
        qty_filled: Fraction,
        price: Fraction,
        fee: Option<Fraction>,
        fill_key: String,
    },
    MarketOpen {
        id: String,
        side: dca_math::Side,
        kind: dca_kernel::OrderKind,
        role: String,
        price: Fraction,
        qty: Fraction,
    },
    MarketQueryFailed {
        id: String,
        err: String,
    },
    GateBlocked {
        id: String,
        reason: String,
    },
    PlaceFailed {
        id: String,
        err: String,
        is_close: bool,
    },
    Cancelled {
        cancelled: Vec<String>,
        failed: Vec<String>,
    },
}

/// One LIVE resting order awaiting venue fill reconciliation.
/// Faz 22 (P2-1): Serialize/Deserialize — snapshot'a girer (restart sonrası
/// dış dolum takibi kaybolmazdı).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct LiveTrackedOrder {
    pub id: String,
    pub side: dca_math::Side,
    pub kind: dca_kernel::OrderKind,
    pub price: Fraction,
    pub qty: Fraction,
    pub role: String,
}

impl BotRuntime {
    pub fn snapshot_json(&self) -> serde_json::Value {
        let snap = self.engine_kind.snapshot();
        // Faz 24 (UI/UX Pro Max §3.1.2): net gerçekleşen (kernel tanımı:
        // gross − kapanış fees − funding) UI'ya stats içinden taşınır —
        // strategy EngineSnapshot'ında bu alan yoktur.
        let mut snap = match serde_json::to_value(&snap) {
            Ok(v) => v,
            Err(_) => serde_json::json!({}),
        };
        if let Some(obj) = snap.as_object_mut() {
            obj.insert(
                "realized_net".into(),
                serde_json::json!(self.stepper.realized_net.to_string()),
            );
            obj.insert(
                "position_direction".into(),
                serde_json::json!(self.stepper.position.direction),
            );
        }
        let warnings = dca_strategy::compute_warnings(&self.cfg, &self.stepper.instrument);
        // Faz 13B: equity'ler makro öncesi hesaplanır (json! blok kısıtı).
        let mark_now = self
            .stepper
            .last_close
            .as_deref()
            .and_then(|x| dca_math::fraction::Fraction::from_any_str(x).ok());
        let mut eq_comb = mark_now.as_ref().and_then(|m| self.stepper.equity(m).ok());
        let hedge_equity_now = self
            .hedge_stepper
            .as_ref()
            .and_then(|hs| mark_now.as_ref().and_then(|m| hs.equity(m).ok()))
            .map(|x| x.to_string());
        if let (Some(hs), Some(m)) = (self.hedge_stepper.as_ref(), mark_now.as_ref()) {
            if let Ok(he) = hs.equity(m) {
                // Arena 1.5 Düzeltmesi: hs'nin equity'si tam start_cash içerir.
                // İki bacağın start_cash'i toplanmaz; hedge bacağının net PnL katkısı eklenir.
                let hpnl = he
                    .sub(&hs.start_cash)
                    .unwrap_or_else(|_| dca_math::fraction::Fraction::zero());
                eq_comb = eq_comb.and_then(|e| e.add(&hpnl).ok());
            }
        }
        let equity_now = eq_comb.map(|x| x.to_string());
        // M-17: unrealizedNow hedge botlarda hedge bacağını da içerir (ana +
        // hedge bacak unrealized gross, son bar kapanış mark'ı üzerinde).
        let unrealized_now = (|| -> Option<String> {
            let mark = self
                .stepper
                .last_close
                .as_deref()
                .and_then(|x| dca_math::fraction::Fraction::from_any_str(x).ok())?;
            let mut u = self.stepper.position.unrealized_gross(&mark).ok()?;
            if let Some(hs) = &self.hedge_stepper {
                if let Ok(hu) = hs.position.unrealized_gross(&mark) {
                    u = u.add(&hu).ok().unwrap_or(u);
                }
            }
            Some(u.to_string())
        })();
        json!({
            "id": self.bot_id,
            "name": self.name,
            "kind": self.cfg.kind.label(),
            "venue": self.venue,
            "mode": self.mode,
            "executionEnvironment": self.execution_environment,
            "status": self.status,
            // Faz 21 (N-8): kernel halt durumu arayüze görünür — Studio/Fleet
            // kırmızı rozet + "KERNEL'İ SIFIRLA" düğmesi bununla çalışır.
            "kernelHalted": self.stepper.halted.is_some(),
            "kernelHaltedReason": self.stepper.halted.clone(),
            "symbol": self.cfg.symbol,
            "interval": self.interval_str,
            "stats": snap,
            "config": serde_json::to_value(&self.cfg).unwrap_or(serde_json::Value::Null),
            "cash": self.stepper.cash.to_string(),
            "cumulativeFees": self.stepper.cumulative_fees.to_string(),
            // Faz 28 (P1-3): funding maliyeti snapshot'a çıkar — /api/summary
            // totalFunding bunu toplar; UI totalEconomicPnL'e dahil eder.
            "cumulativeFunding": self.stepper.cumulative_funding.to_string(),
            "marginMode": !self.stepper.cfg.is_spot,
            "leverage": self.stepper.cfg.leverage.to_string(),
            "imUsed": self.stepper.im_used.to_string(),
            "liqPrice": crate::engine::current_liq_price(self).map(|p| p.to_string()),
            // SEC-05: webhookSecret snapshot'tan KALDIRILDI — hassas bir şifredir,
            // her bot detay yanıtında açık metin olarak dökülmemeli.
            "entrySignalMet": self.entry_signal_met,
            // Task 16 — risk motoru yüzeyi
            "warnings": warnings,
            "riskPolicy": serde_json::to_value(&self.cfg.risk_policy).unwrap_or(serde_json::Value::Null),
            "regime": self.last_regime.label(),
            "breakerTripped": self.breaker_tripped,
            // Faz 13 dilim 2 + 13B: değerler makro ÖNCESİ hesaplanır (json! blok
            // ifadeyi nested-JSON sanır). Hedge bot: equity = ana + hedge bacak.
            "equityNow": equity_now,
            "hedgeEquityNow": hedge_equity_now,
            "unrealizedNow": unrealized_now,
            "markPrice": self.stepper.last_close.clone(),
        })
    }
}

pub struct AppState {
    pub db: Db,
    pub vault: Vault,
    pub bybit: BybitAdapter,
    pub binance: BinanceAdapter,
    pub okx: OkxAdapter,
    pub paper: PaperVenue,
    pub runtimes: Mutex<HashMap<String, BotRuntime>>,
    /// Faz 13 dilim 2: Fleet/Özet OKUMASI runtime kilidini beklemez.
    pub ui_cache: Mutex<HashMap<String, serde_json::Value>>,
    pub sse: broadcast::Sender<SseEvent>,
    pub started_at: i64,
    /// H-1: bot başına single-flight poll kilidi — aynı bot için üst üste
    /// poll birikmez (supervisor turu + manuel tick çakışması). Boş girişler
    /// bot silinirken kaldırılır.
    pub poll_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// LIVE bot creation/start status transitions are serialized so one
    /// venue-wide position snapshot cannot be assigned to two bot runtimes.
    pub live_ownership_lock: Arc<tokio::sync::Mutex<()>>,
    /// Faz 5 (H-8): son başarılı supervisor poll'u (ms) — readiness göstergesi.
    pub last_tick_at: std::sync::Mutex<i64>,
    /// M-3: instrument cache artık TTL'li — (cached_at_ms, liste). Tek bir ağ
    /// hatası boş listeyi restart'a kadar kalıcılaştırmaz.
    pub instruments_cache: Mutex<HashMap<String, (i64, Vec<Instrument>)>>,
    /// P2-1 ticker 24h cache: key → (cached_at_ms, rows). 30 sn TTL —
    /// instruments gibi saatlik değil, 24s değişim tazeliği için kısa tutulur.
    pub tickers_cache: Mutex<HashMap<String, (i64, Vec<dca_exchange::Ticker24h>)>>,
    /// P4-1 haber önbelleği: symbol → (cached_at_ms, yanıt). 5 dk TTL —
    /// sağlayıcı rate-limit koruması (v3 ile aynı süre).
    pub news_cache: Mutex<HashMap<String, (i64, serde_json::Value)>>,
    /// Faz 22 (P2-10 dilim): webhook rate-limit — IP bazlı sabit pencere
    /// sayaçları (pencere_başlangıcı_ms, penceredeki_istek_sayısı).
    pub webhook_rate: Mutex<HashMap<String, (i64, u32)>>,
    /// P4-1 haber HTTP istemcisi (8 sn zaman aşımı, beyaz-liste konaklar).
    pub news_http: reqwest::Client,
    /// P4-2 haber TR-özet önbelleği: news item id → özet (öğe başına bir kez).
    pub news_ai_cache: Mutex<HashMap<String, String>>,
    /// Faz 0.2 — global kill-switch (v3 engage/release; TV ENTRY dahil durdurur).
    pub kill: Mutex<HaltState>,
    /// Faz 0.3 — global freeze (v3 realOrderGate; yalnız LIVE yeni emirler durur).
    pub freeze: Mutex<HaltState>,
    /// Faz 1.5 — bakım modu bellek-ici simülasyon üst-yazımı (mod, bitiş-ms).
    /// Restart/süre-bitimi/reset ile kalkar; `settings` tablosuna yazılmaz.
    pub maint_sim: Mutex<Option<(String, i64)>>,
    /// Faz 1.6 — BNB serbest bakiye önbelleği: (cached_at_ms, free_bnb).
    /// 45 sn TTL (v3 aynen) — Ayarlar kartı poll'larken borsa yoklanmaz.
    /// Yalnız EXCHANGE başarısı yazılır; ABSTAIN önbelleğe girmez (ağ yok).
    pub bnb_cache: Mutex<Option<(i64, String)>>,
    /// Faz 4 (C-6): venue başına LIVE açık-emir yükü (marj maliyeti cinsinden).
    /// poll her turunda (runtime kilidi altında) hesaplanıp yazılır; emir öncesi
    /// kapı kilit çakışması olmadan buradan okur (ayrı kilit — deadlock yok).
    pub venue_live_load: Mutex<HashMap<String, Fraction>>,
    /// Faz 27 (İ-1): portföy kapısı anlık görüntüsü — (ts_ms, nav, gross).
    /// venue_live_load ile aynı desen: poll turunda runtime kilidi ALTINDA
    /// hesaplanır, dispatch içindeki kapı ayrı std kilitten okur (tokio
    /// kilidi yeniden istenmez — deadlock yok). Okuma tarafı 30 sn tazelik
    /// penceresi aşarsa kapı o turu atlar (fail-open; bir sonraki tick'te gelir).
    pub portfolio_cache: std::sync::Mutex<Option<(i64, Fraction, Fraction)>>,
    /// H-7 ince iş: webhook alert tekrar koruması — (botId+action+timestamp)
    /// karması → son görülme ms. 60 sn penceresi aşan ya da pencere içinde
    /// tekrar gelen aynı alert dürüstçe 400 DUPLICATE_ALERT alır.
    pub webhook_seen: Mutex<HashMap<String, i64>>,
    /// Faz 2.4 — işleyen tick izi: bot_id -> trace_id. `poll_and_advance`
    /// başında konur, sonunda kalkar; `persist_event` varsa iliştirir.
    /// Tick-dışı yazımlar (webhook/aksiyon) NULL kalır — hizası dürüsttür.
    pub tick_trace: Mutex<HashMap<String, String>>,
    /// Faz 1.6 (B-8): sembol+interval bazlı paylaşımlı kline önbelleği — rate limit ve 429 koruması
    pub kline_cache: std::sync::Mutex<HashMap<String, (i64, Vec<dca_exchange::Kline>)>>,
}

impl AppState {
    pub fn new(db: Db, vault: Vault) -> (Arc<Self>, broadcast::Receiver<SseEvent>) {
        let (paper_venue, _start_cash_unused) = (
            PaperVenue::new(Fraction::from_decimal_str("100000").unwrap()),
            (),
        );
        let (sse, rx) = broadcast::channel(2048);
        // Unit tests use explicit test/demo adapters. Never infer a safety
        // bypass from the test executable's filesystem path.
        let test_adapters = cfg!(test);
        let state = Arc::new(Self {
            db,
            vault,
            bybit: BybitAdapter::new(
                test_adapters
                    || std::env::var("DCA_BYBIT_TESTNET")
                        .map(|v| v == "1")
                        .unwrap_or(false),
            ),
            binance: if test_adapters {
                BinanceAdapter::new_testnet()
            } else {
                BinanceAdapter::new()
            },
            okx: if test_adapters {
                OkxAdapter::new_demo()
            } else {
                OkxAdapter::new()
            },
            paper: paper_venue,
            runtimes: Mutex::new(HashMap::new()),
            ui_cache: Mutex::new(HashMap::new()),
            sse,
            started_at: now_ms(),
            poll_locks: Mutex::new(HashMap::new()),
            live_ownership_lock: Arc::new(tokio::sync::Mutex::new(())),
            last_tick_at: std::sync::Mutex::new(0),
            instruments_cache: Mutex::new(HashMap::new()),
            tickers_cache: Mutex::new(HashMap::new()),
            news_cache: Mutex::new(HashMap::new()),
            webhook_rate: Mutex::new(HashMap::new()),
            news_http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(8))
                .user_agent("dcabot-news/1.0 (+internal)")
                .build()
                .expect("news http client"),
            news_ai_cache: Mutex::new(HashMap::new()),
            kill: Mutex::new(HaltState::default()),
            freeze: Mutex::new(HaltState::default()),
            maint_sim: Mutex::new(None),
            bnb_cache: Mutex::new(None),
            venue_live_load: Mutex::new(HashMap::new()),
            portfolio_cache: std::sync::Mutex::new(None),
            webhook_seen: Mutex::new(HashMap::new()),
            tick_trace: Mutex::new(HashMap::new()),
            kline_cache: std::sync::Mutex::new(HashMap::new()),
        });
        (state, rx)
    }

    /// Faz 1.6 (B-8): kline önbelleği okuma (TTL kontrolü ile)
    pub fn get_cached_klines(
        &self,
        key: &str,
        max_age_ms: i64,
    ) -> Option<Vec<dca_exchange::Kline>> {
        let cache = self.kline_cache.lock().ok()?;
        if let Some((ts, klines)) = cache.get(key) {
            if now_ms() - *ts < max_age_ms {
                return Some(klines.clone());
            }
        }
        None
    }

    /// Faz 1.6 (B-8): kline önbelleği yazma
    pub fn set_cached_klines(&self, key: String, klines: Vec<dca_exchange::Kline>) {
        if let Ok(mut cache) = self.kline_cache.lock() {
            cache.insert(key, (now_ms(), klines));
        }
    }

    /// Faz 5: venue BYBIT VE ucu demo/testnet ise adaptörü ver (otomatik
    /// pos-mode düzeltmesi yalnız bu koşulda denenir).
    pub fn bybit_demo_aware(&self, venue: &str) -> Option<&BybitAdapter> {
        if venue == "BYBIT" && self.bybit.is_demo_or_testnet() {
            Some(&self.bybit)
        } else {
            None
        }
    }

    pub fn adapter(&self, venue: &str) -> Option<&dyn VenueAdapter> {
        match venue {
            "BYBIT" => Some(&self.bybit),
            "BINANCE" => Some(&self.binance),
            "OKX" => Some(&self.okx),
            "PAPER" => Some(&self.paper),
            _ => None,
        }
    }

    /// Report the effective adapter endpoint, not environment flags that can
    /// disagree with an endpoint override. A demo flag on a mainnet URL is not
    /// a sandbox.
    pub fn is_sandbox_venue(&self, venue: &str) -> bool {
        match venue.to_ascii_uppercase().as_str() {
            "BYBIT" => self.bybit.is_demo_or_testnet(),
            "BINANCE" => self.binance.base_url == "https://testnet.binance.vision",
            "OKX" => self.okx.is_demo,
            _ => false,
        }
    }

    pub fn execution_environment(&self, venue: &str, mode: &str) -> &'static str {
        if !mode.eq_ignore_ascii_case("LIVE") {
            return "INTERNAL_PAPER";
        }
        match venue.to_ascii_uppercase().as_str() {
            "BYBIT" if self.bybit.base_url == "https://api-demo.bybit.com" => "BYBIT_DEMO",
            "BYBIT" if self.bybit.base_url == "https://api-testnet.bybit.com" => "BYBIT_TESTNET",
            "BYBIT" => "MAINNET",
            "BINANCE" if self.binance.base_url == "https://testnet.binance.vision" => "BINANCE_TESTNET",
            "BINANCE" => "MAINNET",
            "OKX" if self.okx.is_demo => "OKX_DEMO",
            "OKX" => "MAINNET",
            _ => "UNSUPPORTED",
        }
    }
}

impl AppState {
    pub async fn kill_engaged(&self) -> bool {
        self.kill.lock().await.active
    }

    pub async fn freeze_engaged(&self) -> bool {
        self.freeze.lock().await.active
    }

    /// LIVE yürütme durdu mu (kill veya freeze — v3 realOrderGate karşılığı).
    pub async fn live_halted(&self) -> bool {
        self.kill.lock().await.active || self.freeze.lock().await.active
    }

    pub async fn kill_json(&self) -> serde_json::Value {
        let k = self.kill.lock().await;
        serde_json::json!({"engaged": k.active, "reason": k.reason, "by": k.by, "at": k.at_ms, "auto": k.auto})
    }

    pub async fn freeze_json(&self) -> serde_json::Value {
        let f = self.freeze.lock().await;
        serde_json::json!({"frozen": f.active, "reason": f.reason, "by": f.by, "at": f.at_ms})
    }

    /// Restart-safe geri yükleme: her kapı için SON olayı uygula (v3
    /// `getKillSwitchState` ile aynı sözleşme — doğruluk kaynağı olay akışı).
    /// Olay yoksa/bozuksa kapalı kalır (fail-closed varsayılan).
    pub async fn rehydrate_halts(app: &Arc<AppState>) {
        let rows: Vec<(String, String, i64)> = (|| {
            let conn = app.db.conn.lock().ok()?;
            let mut stmt = conn
                .prepare(
                    "SELECT gate, message, ts FROM events WHERE id IN (\
                       SELECT MAX(id) FROM events WHERE gate IN ('KILL_SWITCH','FREEZE') GROUP BY gate\
                     ) ORDER BY id DESC",
                )
                .ok()?;
            let mapped = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).ok()?;
            Some(mapped.filter_map(|r| r.ok()).collect())
        })()
        .unwrap_or_default();
        let mut kill_done = false;
        let mut freeze_done = false;
        for (gate, msg, ts) in rows {
            if gate == GATE_KILL && !kill_done {
                kill_done = true;
                if let Some((action, reason, by, at_ms, auto)) = parse_halt_message(&msg) {
                    if action == "engaged" {
                        *app.kill.lock().await = HaltState {
                            active: true,
                            reason,
                            by,
                            at_ms: if at_ms > 0 { at_ms } else { ts },
                            auto,
                        };
                    }
                }
            } else if gate == GATE_FREEZE && !freeze_done {
                freeze_done = true;
                if let Some((action, reason, by, at_ms, _)) = parse_halt_message(&msg) {
                    if action == "engaged" {
                        *app.freeze.lock().await = HaltState {
                            active: true,
                            reason,
                            by,
                            at_ms: if at_ms > 0 { at_ms } else { ts },
                            auto: false,
                        };
                    }
                }
            }
            if kill_done && freeze_done {
                break;
            }
        }
    }
}

pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

pub async fn persist_event(app: &AppState, bot_id: &str, level: &str, gate: &str, message: &str) {
    let trace: Option<String> = app.tick_trace.lock().await.get(bot_id).cloned();
    persist_event_traced(app, bot_id, level, gate, message, trace.as_deref()).await;
}

/// Faz 2.4 — izli yazım. `trace_id=None` tick-dışı / göç-öncesi anlamındadır.
/// Sütun eski DB'lerde göçle gelir (`store.rs`); göçsüz DB'de yazım
/// başarısız olur ve stderr'e düşer (sessiz kayıp yok).
pub async fn persist_event_traced(
    app: &AppState,
    bot_id: &str,
    level: &str,
    gate: &str,
    message: &str,
    trace_id: Option<&str>,
) {
    let conn = app.db.conn.lock().map(|c| c);
    if let Ok(conn) = conn {
        // Task 18 N-4: sessiz yutma yerine stderr — SQLITE busy/lock kaybı
        // gözlenebilir olsun (event akışı audit için kritik).
        if let Err(e) = conn.execute(
            "INSERT INTO events(bot_id, level, gate, message, ts, trace_id) VALUES(?1,?2,?3,?4,?5,?6)",
            rusqlite::params![bot_id, level, gate, message, now_ms(), trace_id],
        ) {
            eprintln!("[db] event yazımı başarısız (bot={bot_id}, gate={gate}): {e}");
        }
        // M-9: her 64 yazimda bir global retention — son 200k olay dişinda
        // silinir (bot başına TICK spam'i tabloyu sonsuz büyütmesin).
        static EVENT_WRITE_COUNT: std::sync::atomic::AtomicU64 =
            std::sync::atomic::AtomicU64::new(0);
        if EVENT_WRITE_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 64 == 0 {
            let _ = conn.execute(
                "DELETE FROM events WHERE id < (SELECT MIN(id) FROM (SELECT id FROM events ORDER BY id DESC LIMIT 200000))",
                [],
            );
        }
    }
}

/// Faz 2.4 — tick iz çerçevesi: `tr-<bot8>-<ts>` üretir, haritaya koyar.
pub fn new_tick_trace(bot_id: &str) -> String {
    let short: String = bot_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    format!(
        "tr-{}-{}",
        if short.is_empty() { "bot" } else { &short },
        now_ms()
    )
}

#[allow(dead_code)]
pub async fn persist_bot_stats(app: &AppState, bot_id: &str, stats_json: &str) {
    if let Ok(conn) = app.db.conn.lock() {
        if let Err(e) = conn.execute(
            "UPDATE bots SET stats=?1 WHERE id=?2",
            rusqlite::params![stats_json, bot_id],
        ) {
            eprintln!("[db] stats yazımı başarısız (bot={bot_id}): {e}");
        }
    }
}

/// Task 18 N-4 — kritik bot durum yazımı: tek retry + stderr. SQLITE
/// busy/lock durumunda status kaybı, restart'ta STOPPED bir botu eski
/// RUNNING durumuyla diriltebilir — bu yüzden başarısızlık sessiz kalmaz.
pub fn persist_status(app: &AppState, bot_id: &str, status: &str) {
    let write = |c: &rusqlite::Connection| {
        c.execute(
            "UPDATE bots SET status=?1 WHERE id=?2",
            rusqlite::params![status, bot_id],
        )
    };
    match app.db.conn.lock() {
        Ok(conn) => {
            if let Err(e) = write(&conn) {
                eprintln!("[db] status yazımı yeniden deneniyor (bot={bot_id}, status={status}): {e}");
                if let Err(e2) = write(&conn) {
                    eprintln!("[db] KRİTİK: status yazımı başarısız (bot={bot_id}, status={status}): {e2}");
                }
            }
        }
        Err(e) => eprintln!("[db] KRİTİK: db kilidi alınamadı — status yazılamadı (bot={bot_id}, status={status}): {e}"),
    }
}

/// Faz 3 (C-4): kalici runtime snapshot — motor fazi (EngineTag serde) +
/// stepper muhasebesi + imlec. Restart sonrasi bot kaldigi yerden devam eder;
/// "yarim restore" (faz sifir, cash sifir) donemi kapanir. LIVE restore'da
/// pozisyon uzlastirmasi snapshot'tan SONRA venue gercekligiyle ezer.
#[derive(serde::Serialize, serde::Deserialize)]
pub struct RuntimeSnapshot {
    pub engine: EngineTag,
    pub cash: Fraction,
    pub im_used: Fraction,
    pub cumulative_fees: Fraction,
    pub cumulative_funding: Fraction,
    /// Faz 18 (P1-1): kapanış komisyonları defteri — realized_net tanımının
    /// bileşeni; eski snapshot'larla uyum için default.
    #[serde(default)]
    pub realized_fees: Fraction,
    pub realized_net: Fraction,
    pub last_bar_ts: i64,
    pub grid_bootstrapped: bool,
    pub leverage_synced: bool,
    /// Faz 5 (H-7): webhook secret kalıcı — restart sonrası TradingView
    /// alert'leri geçerliliğini korur (yoksa üretilmiş yeni secret kullanılır;
    /// eski snapshot'larla uyum için Option+default).
    #[serde(default)]
    pub webhook_secret: Option<String>,
    /// Faz 13B: hedge bacağı kernel snapshot'ı (EX-06 kalıcı çözüm).
    #[serde(default)]
    pub hedge_kernel: Option<HedgeKernelSnap>,
    /// P0-3: ANA kernel pozisyonu — restore sonrası kernel/mirror ayrışması
    /// (hayalet pozisyon + Closing kilidi) tam olarak bu eksikten doğuyordu.
    #[serde(default)]
    pub position: Option<dca_math::m04::PositionState>,
    #[serde(default)]
    pub kernel_prev_bar_ts: Option<i64>,
    /// P0-3: bekleyen emirler de snapshot'a girer (kapanış emri kaybı olmasın).
    #[serde(default)]
    pub pending: Vec<dca_kernel::OrderIntent>,
    /// Faz 22 (P2-1 kalanı): günlük DD devre kesici durumu + açık emir defteri
    /// — restart'ta kesici sıfırlanır ve dış dolum takibi kaybolurdu (fail-open).
    #[serde(default)]
    pub breaker_day: i64,
    #[serde(default)]
    pub breaker_day_start_eq: Option<String>,
    #[serde(default)]
    pub breaker_tripped: bool,
    #[serde(default)]
    pub live_open_orders: Vec<LiveTrackedOrder>,
    /// Faz 22: rejim sınıflandırma penceresi — restart'ta sıfırlanırsa
    /// sınıflandırıcı warm-up'a döner (200 kapanış penceresi).
    #[serde(default)]
    pub regime_closes: Vec<f64>,
}

/// Faz 13B: hedge bacağı kernel defteri — ana bacaktan bağımsız restore.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct HedgeKernelSnap {
    pub cash: Fraction,
    pub im_used: Fraction,
    pub cumulative_fees: Fraction,
    pub cumulative_funding: Fraction,
    pub realized_net: Fraction,
    pub position: dca_math::m04::PositionState,
    /// Faz 23 (derin inceleme): hedge bacağının BEKLEYEN emirleri — eski
    /// snapshot atıyordu; restart sonrası resting_safety sayaçları emirleri
    /// kitapta sanıp ladder sessizce tıkanıyordu.
    #[serde(default)]
    pub pending: Vec<dca_kernel::OrderIntent>,
}

/// Faz 3 (C-4): runtime snapshot'i bots.runtime_state'a yazar (UPSERT).
/// Yazim hatasi stderr'e duser — sessiz kayip yok; bot akisi etkilenmez.
pub fn save_runtime_state(app: &AppState, bot_id: &str, rt: &BotRuntime) {
    let snap = RuntimeSnapshot {
        engine: rt.engine_kind.clone(),
        cash: rt.stepper.cash.clone(),
        im_used: rt.stepper.im_used.clone(),
        cumulative_fees: rt.stepper.cumulative_fees.clone(),
        cumulative_funding: rt.stepper.cumulative_funding.clone(),
        realized_fees: rt.stepper.realized_fees.clone(),
        realized_net: rt.stepper.realized_net.clone(),
        last_bar_ts: rt.last_bar_ts,
        grid_bootstrapped: rt.grid_bootstrapped,
        leverage_synced: rt.leverage_synced,
        webhook_secret: Some(rt.webhook_secret.clone()),
        hedge_kernel: rt.hedge_stepper.as_ref().map(|hs| HedgeKernelSnap {
            cash: hs.cash.clone(),
            im_used: hs.im_used.clone(),
            cumulative_fees: hs.cumulative_fees.clone(),
            cumulative_funding: hs.cumulative_funding.clone(),
            realized_net: hs.realized_net.clone(),
            position: hs.position.clone(),
            pending: hs.pending.clone(),
        }),
        position: Some(rt.stepper.position.clone()),
        kernel_prev_bar_ts: rt.stepper.prev_bar_ts,
        pending: rt.stepper.pending.clone(),
        breaker_day: rt.breaker_day,
        breaker_day_start_eq: rt.breaker_day_start_eq.as_ref().map(|x| x.to_string()),
        breaker_tripped: rt.breaker_tripped,
        live_open_orders: rt.live_open_orders.clone(),
        regime_closes: rt.regime_closes.clone(),
    };
    let json = match serde_json::to_string(&snap) {
        Ok(j) => j,
        Err(e) => {
            eprintln!("[db] runtime snapshot serilestirilemedi (bot={bot_id}): {e}");
            return;
        }
    };
    match app.db.conn.lock() {
        Ok(conn) => {
            if let Err(e) = conn.execute(
                "UPDATE bots SET runtime_state=?1 WHERE id=?2",
                rusqlite::params![json, bot_id],
            ) {
                eprintln!("[db] runtime snapshot yazilamadi (bot={bot_id}): {e}");
            }
        }
        Err(e) => eprintln!("[db] runtime snapshot: db kilidi alinamadi (bot={bot_id}): {e}"),
    }
}

/// Faz 3 (C-4): restore icin snapshot JSON'unu okur (yoksa/bozuksa None).
pub fn load_runtime_state_json(app: &AppState, bot_id: &str) -> Option<String> {
    let conn = app.db.conn.lock().ok()?;
    conn.query_row(
        "SELECT runtime_state FROM bots WHERE id=?1",
        [bot_id],
        |r| r.get::<_, Option<String>>(0),
    )
    .ok()
    .flatten()
}

/// Faz 3 (H-3): LIVE emir takip kaydi — place aninda PENDING yazilir, fill
/// uzlastirmasi state gunceller, terminal durumda satir duser. Restart sonrasi
/// takip bu tablodan geri yuklenir (bellek-only donem kapandi).
#[allow(clippy::too_many_arguments)]
pub fn upsert_live_order(
    app: &AppState,
    bot: &str,
    order_id: &str,
    side: &str,
    kind: &str,
    price: &str,
    qty: &str,
    role: &str,
    state: &str,
) {
    if let Ok(conn) = app.db.conn.lock() {
        let _ = conn.execute(
            "INSERT INTO live_orders(bot_id, order_id, side, kind, price, qty, role, state, updated_at) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9) ON CONFLICT(bot_id, order_id) DO UPDATE SET state=excluded.state, updated_at=excluded.updated_at, price=excluded.price, qty=excluded.qty, role=excluded.role",
            rusqlite::params![bot, order_id, side, kind, price, qty, role, state, now_ms()],
        );
    }
}

/// Faz 4.5 (C-6): surekli rezervasyon defteri — place/cancel aninda anlik
/// guncellenir (poll-refresh periyodik duzeltir; place araligindaki cift
/// harcama penceresini kapatir). add=true -> yuk ekler, false -> duserur.
pub async fn bump_venue_live_load(app: &AppState, venue: &str, cost: &Fraction, add: bool) {
    let mut loads = app.venue_live_load.lock().await;
    let cur = loads.get(venue).cloned().unwrap_or_else(Fraction::zero);
    let next = if add { cur.add(cost) } else { cur.sub(cost) }.unwrap_or(cur);
    loads.insert(venue.to_string(), next);
}

/// Faz 3 (H-3): terminal emir takip kaydini duserur.
pub fn remove_live_order(app: &AppState, bot: &str, order_id: &str) {
    if let Ok(conn) = app.db.conn.lock() {
        let _ = conn.execute(
            "DELETE FROM live_orders WHERE bot_id=?1 AND order_id=?2",
            rusqlite::params![bot, order_id],
        );
    }
}

/// Faz 3 (H-3): restart'ta geri yuklenecek aktif (PENDING/PARTIAL) takip kayitlari.
pub fn load_live_orders(app: &AppState, bot: &str) -> Vec<LiveTrackedOrder> {
    let Ok(conn) = app.db.conn.lock() else {
        return Vec::new();
    };
    let Ok(mut stmt) = conn.prepare("SELECT order_id, side, kind, price, qty, role FROM live_orders WHERE bot_id=?1 AND state IN ('PENDING','PARTIAL') ORDER BY id") else { return Vec::new() };
    let Ok(rows) = stmt.query_map([bot], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
            r.get::<_, String>(5)?,
        ))
    }) else {
        return Vec::new();
    };
    rows.filter_map(|x| x.ok())
        .filter_map(|(id, side, kind, price, qty, role)| {
            let side = if side == "Sell" {
                dca_math::Side::Sell
            } else {
                dca_math::Side::Buy
            };
            let kind = if kind == "Reduce" {
                dca_kernel::OrderKind::Reduce
            } else {
                dca_kernel::OrderKind::Add
            };
            Some(LiveTrackedOrder {
                id,
                side,
                kind,
                price: Fraction::from_any_str(&price).ok()?,
                qty: Fraction::from_any_str(&qty).ok()?,
                role,
            })
        })
        .collect()
}

/// Instantiate a BotRuntime from config + persist to DB.
#[allow(clippy::too_many_arguments)]
pub async fn create_runtime(
    app: &Arc<AppState>,
    id: &str,
    name: &str,
    mode: &str,
    venue: &str,
    status: &str,
    cfg: BotConfig,
    interval: &str,
    start_cash: Fraction,
    execution_environment: &str,
    credentials: Option<VenueCreds>,
) -> Result<(), String> {
    // Faz 4 (H-6): enstrüman botun kendi venue'sundan çözülür (PAPER bots
    // BYBIT piyasa verisiyle koştuğundan PAPER da BYBIT'ten çözülür).
    let mut inst = engine_instrument_for(&cfg, venue, app).await;
    // Faz 22 (P1-23): LIVE botlarda hesap-bazlı GERÇEK ücretler — enstrüman
    // varsayılanları VIP/komisyon kademesini yansıtmaz; venue destekliyorsa
    // (/v5/account/fee-rate) maker/taker burada override edilir.
    if mode == "LIVE" {
        if let (Some(creds), Some(adapter)) = (credentials.as_ref(), app.adapter(venue)) {
            let category = if cfg.kind.is_futures() {
                "linear"
            } else {
                "spot"
            };
            match adapter.account_fee_rate(creds, category, &cfg.symbol).await {
                Ok(Some((maker, taker))) if maker.is_positive() && taker.is_positive() => {
                    persist_event(
                        app,
                        id,
                        "INFO",
                        "FEES",
                        &format!(
                            "hesap-bazlı gerçek ücretler uygulandı: maker={} taker={} (enstrüman varsayılanı yerine)",
                            maker.to_half_even_display(6),
                            taker.to_half_even_display(6)
                        ),
                    )
                    .await;
                    inst.maker_fee_rate = maker;
                    inst.taker_fee_rate = taker;
                }
                Ok(_) => {
                    persist_event(app, id, "INFO", "FEES", "venue hesap ücreti döndürmedi — enstrüman varsayılan ücretleri kullanılıyor").await;
                }
                Err(e) => {
                    persist_event(app, id, "WARN", "FEES", &format!("hesap ücreti alınamadı: {e} — enstrüman varsayılan ücretleri kullanılıyor")).await;
                }
            }
        }
    }
    let kind = cfg.kind;
    let engine = match kind {
        StrategyKind::SpotGrid | StrategyKind::FuturesGrid | StrategyKind::InfinityGrid => {
            // grid needs a reference mark; use last close from klines or config grid mid
            let mark = match (&cfg.grid_lower, &cfg.grid_upper) {
                (Some(lo), Some(hi)) => {
                    let lo = Fraction::from_decimal_str(lo).map_err(|e| e.to_string())?;
                    let hi = Fraction::from_decimal_str(hi).map_err(|e| e.to_string())?;
                    lo.add(&hi)
                        .map_err(|e| e.to_string())?
                        .div(&Fraction::from_u64(2))
                        .map_err(|e| e.to_string())?
                }
                _ => Fraction::from_u64(1),
            };
            EngineTag::Grid(dca_strategy::GridEngine::new(
                id,
                cfg.clone(),
                inst.clone(),
                &mark,
            )?)
        }
        StrategyKind::HedgeProtective | StrategyKind::HedgeDual => EngineTag::Hedge(Box::new(
            dca_strategy::HedgeEngine::new(id, cfg.clone(), inst.clone())?,
        )),
        _ => EngineTag::Dca(dca_strategy::DcaEngine::new(id, cfg.clone(), inst.clone())?),
    };
    let stepper = SimulationStepper::new(
        inst.clone(),
        start_cash.clone(),
        StepperConfig {
            // Task 16: seans tipi artık kullanıcının risk politikasından gelir
            // (Gate 7/8a gerçek piyasa takvimiyle tetiklenir).
            session_type: cfg.risk_policy.session_type.into(),
            capital_shortfall_mode: dca_exchange_gate_mode(),
            is_spot: !kind.is_futures(),
            leverage: Fraction::from_decimal_str(&cfg.leverage).unwrap_or_else(|_| Fraction::one()),
            policy: cfg.risk_policy.clone(),
            ..Default::default()
        },
    );
    // Short engines sell-to-open: mirror position direction must be -1 for
    // correct WAC/uPnL math (long futures and spot stay +1).
    let mut stepper = stepper;
    if kind.direction() == Some(dca_math::m02::Direction::Short) {
        stepper.set_position_direction(-1);
    }
    // Faz 13B: hedge bacak stepper'ı — her iki hedge türü için baştan kurulur
    // (protective spawn sonrası devreye girer; dual ilk barda). Bacak defteri
    // bağımsızdır: aynı start_cash, yön -1 (sell-to-open).
    let hedge_stepper = if matches!(
        kind,
        StrategyKind::HedgeProtective | StrategyKind::HedgeDual
    ) {
        let mut hs = SimulationStepper::new(
            inst.clone(),
            start_cash.clone(),
            StepperConfig {
                session_type: cfg.risk_policy.session_type.into(),
                capital_shortfall_mode: dca_exchange_gate_mode(),
                is_spot: !kind.is_futures(),
                leverage: Fraction::from_decimal_str(&cfg.leverage)
                    .unwrap_or_else(|_| Fraction::one()),
                policy: cfg.risk_policy.clone(),
                ..Default::default()
            },
        );
        hs.set_position_direction(-1);
        Some(hs)
    } else {
        None
    };
    let is_grid_kind = matches!(
        kind,
        StrategyKind::SpotGrid | StrategyKind::FuturesGrid | StrategyKind::InfinityGrid
    );
    // TradingView webhook secret — stable per bot session (persisted bots get a
    // fresh secret on restore; the UI surfaces it in bot settings).
    // Task 20 L4 — webhook secret CSPRNG: DefaultHasher (SipHash sabit seed +
    // zaman girdisi) CSPRNG DEĞİLDİ ve tahmin edilebilirdi. uuid v4 (OsRng
    // destekli) — vault deseninin aynısı. Faz 22 (P2-2): YORUM DÜZELTİLDİ —
    // eski metin "Secret DB'ye yazılmaz" diyordu ama save_runtime_state
    // webhook_secret'i RuntimeSnapshot'a YAZAR (restart sonrası TradingView
    // alert'leri geçerliliğini korur). Yanlış yorum güvenlik incelemesini
    // yanıltırdı; gerçek davranış: secret snapshot'ta kalıcıdır, DB'de ayrı
    // bir sütun tutulmaz.
    let webhook_secret = format!("tvw_{}", uuid::Uuid::new_v4().simple());
    if let Some(sc) = &cfg.entry_signal {
        dca_signals::validate_signal_node(&sc.root)?;
    }
    let rt = BotRuntime {
        bot_id: id.to_string(),
        name: name.to_string(),
        mode: mode.to_string(),
        execution_environment: execution_environment.to_string(),
        venue: venue.to_string(),
        status: status.to_string(),
        cfg,
        interval_str: interval.to_string(),
        last_bar_ts: 0,
        stepper,
        engine_kind: engine,
        credentials,
        leverage_synced: mode != "LIVE" || !kind.is_futures(),
        grid_bootstrapped: !is_grid_kind,
        webhook_secret,
        webhook_start_requested: false,
        entry_signal_met: None,
        live_open_orders: Vec::new(),
        regime_closes: Vec::new(),
        last_regime: dca_signals::MarketRegime::Normal,
        breaker_day: -1,
        breaker_day_start_eq: None,
        breaker_tripped: false,
        venue_warned: false,
        halt_warned: false,
        pos_mode_checked: false,
        pos_mode_blocked: false,
        pending_live_intents: Vec::new(),
        pending_engine_actions: Vec::new(),
        funding_history: Vec::new(),
        funding_history_ok_at: 0,
        funding_history_tried_at: 0,
        funding_empty_warned: false,
        sweep_cursor: 0,
        hedge_stepper,
        pos_mode_hedge: false,
        mmr_brackets: None,
        mmr_brackets_at: 0,
    };
    // Faz 3 (C-4): kalici snapshot varsa motor fazi + muhasebe geri yuklenir.
    // Bozuk/eski snapshot taze kurulumla duser (gorunur WARN); LIVE pozisyon
    // uzlastirmasi (reconcile_restored_bots) bundan SONRA venue gercekligiyle
    // stepper pozisyonunu ezer.
    let mut rt = rt;
    if let Some(json) = load_runtime_state_json(app, id) {
        match serde_json::from_str::<RuntimeSnapshot>(&json) {
            Ok(snap) => {
                rt.engine_kind = snap.engine;
                rt.stepper.cash = snap.cash;
                rt.stepper.im_used = snap.im_used;
                rt.stepper.cumulative_fees = snap.cumulative_fees;
                rt.stepper.cumulative_funding = snap.cumulative_funding;
                // Faz 18 (P1-1): eski snapshot'larda alan yok → default 0.
                rt.stepper.realized_fees = snap.realized_fees;
                rt.stepper.realized_net = snap.realized_net;
                rt.last_bar_ts = snap.last_bar_ts;
                rt.grid_bootstrapped = snap.grid_bootstrapped;
                rt.leverage_synced = snap.leverage_synced;
                if let Some(sec) = snap.webhook_secret {
                    if !sec.trim().is_empty() {
                        rt.webhook_secret = sec;
                    }
                }
                // Faz 13B: hedge bacak kernel defteri geri yüklenir.
                if let (Some(hk), Some(hs)) = (snap.hedge_kernel, rt.hedge_stepper.as_mut()) {
                    hs.cash = hk.cash;
                    hs.im_used = hk.im_used;
                    hs.cumulative_fees = hk.cumulative_fees;
                    hs.cumulative_funding = hk.cumulative_funding;
                    hs.realized_net = hk.realized_net;
                    hs.position = hk.position;
                    // Faz 23 (derin inceleme): hedge bacağının bekleyen emirleri
                    // de geri yüklenir — ladder tıkanması kapandı.
                    hs.pending = hk.pending;
                }
                // P0-3: ANA kernel pozisyonu + bekleyen emirler geri yüklenir —
                // restart sonrası kernel/engine ayrışması (hayalet pozisyon) kapanır.
                if let Some(pos) = snap.position {
                    rt.stepper.position = pos;
                }
                rt.stepper.prev_bar_ts = snap.kernel_prev_bar_ts.or(Some(snap.last_bar_ts));
                rt.stepper.pending = snap.pending.clone();
                // Faz 22 (P2-1 kalanı): günlük DD kesici + açık emir defteri
                // restore — restart'ta kesici sıfırlanmıyor, dış dolum takibi
                // sürüyor (eski davranış fail-open'dı).
                rt.breaker_day = snap.breaker_day;
                rt.breaker_day_start_eq = snap
                    .breaker_day_start_eq
                    .as_deref()
                    .and_then(|s| dca_math::fraction::Fraction::from_any_str(s).ok());
                rt.breaker_tripped = snap.breaker_tripped;
                rt.live_open_orders = snap.live_open_orders.clone();
                rt.regime_closes = snap.regime_closes;
                // Faz 18 (P0-3 kalanı) + Faz 21 (N-9): restore TUTARLILIK KAPISI —
                // kernel pozisyonu ile engine mirror (adet, YÖN, ortalama fiyat)
                // üçlüsü uyuşmuyorsa sessiz devam ETME: kernel fail-closed HALT
                // ile durdurulur, CRITICAL olay yazılır. Eski kapı yalnız adet
                // bakıyordu — ters yönlü ama aynı adetli restore (short yerine
                // long) kapıdan sessiz geçip TP/SL'i ters çalıştırıyordu.
                // Hedge bacağı için de AYRI kapı uygulanır.
                let tick = rt.stepper.instrument.tick_size.clone();
                let mut restore_mismatch: Option<String> = None;
                {
                    let kernel_main = rt.stepper.position.clone();
                    let engine_main = match &rt.engine_kind {
                        EngineTag::Dca(e) => Some(e.mirror.clone()),
                        EngineTag::Hedge(h) => Some(h.main.mirror.clone()),
                        EngineTag::Grid(_) => None,
                    };
                    if let Some(em) = engine_main {
                        if let Some(detail) = position_leg_mismatch(&kernel_main, &em, &tick, "ana")
                        {
                            restore_mismatch = Some(detail);
                        }
                    }
                    // Hedge bacağı: kernel hedge_stepper ↔ engine hedge leg mirror.
                    if let (Some(hs), EngineTag::Hedge(h)) =
                        (rt.hedge_stepper.as_ref(), &rt.engine_kind)
                    {
                        match &h.hedge {
                            Some(leg) => {
                                if let Some(detail) =
                                    position_leg_mismatch(&hs.position, &leg.mirror, &tick, "hedge")
                                {
                                    let combined = match restore_mismatch {
                                        Some(prev) => format!("{prev}; {detail}"),
                                        None => detail,
                                    };
                                    restore_mismatch = Some(combined);
                                }
                            }
                            None => {
                                if !hs.position.qty.is_zero() {
                                    let detail = format!(
                                        "HALTED_RESTORE: kernel hedge bacağında pozisyon var ({}) ama engine'de hedge bacağı yok — uzlaştırma gerekli",
                                        hs.position.qty.to_half_even_display(8)
                                    );
                                    restore_mismatch = Some(match restore_mismatch {
                                        Some(prev) => format!("{prev}; {detail}"),
                                        None => detail,
                                    });
                                }
                            }
                        }
                    }
                }
                if let Some(reason) = restore_mismatch {
                    rt.stepper.halted = Some(reason.clone());
                    persist_event(app, id, "CRITICAL", "RESTORE", &reason).await;
                }
            }
            Err(e) => {
                // Faz 21 (N-1): çözülemeyen snapshot artık SESSİZ taze başlangıç
                // değil — CRITICAL olay + bot PAUSED açılır (fail-closed).
                // Kullanıcı durumu görür ve START ile bilinçli devam eder.
                rt.status = "PAUSED".into();
                let _ = persist_event(
                    app,
                    id,
                    "CRITICAL",
                    "RESTORE",
                    &format!("runtime snapshot cozulemedi ({e}) — bot PAUSED acildi; taze baslatmak icin START, kalici durumu geri yuklemek icin snapshot'i kontrol edin"),
                )
                .await;
            }
        }
    }
    // Faz 13 dilim 2: kurulum/restore sonrası cache anında dolu olsun.
    let snap_v = rt.snapshot_json();
    app.runtimes.lock().await.insert(id.to_string(), rt);
    app.ui_cache.lock().await.insert(id.to_string(), snap_v);
    Ok(())
}

fn dca_exchange_gate_mode() -> dca_risk::CapitalShortfallMode {
    dca_risk::CapitalShortfallMode::Wait
}

/// Faz 21 (N-9): restore bacak tutarlılığı — (adet, yön, ortalama fiyat)
/// üçlüsü. Ortalama fiyat için tolerans 1 tick (persist string yuvarlaması).
/// Uyuşmazlıkta insan-okur gerekçe döner; uyumda None.
fn position_leg_mismatch(
    kernel: &dca_math::m04::PositionState,
    engine: &dca_math::m04::PositionState,
    tick: &Fraction,
    leg: &str,
) -> Option<String> {
    let fmt = |v: &Fraction| v.to_half_even_display(8);
    if Fraction::cmp(&kernel.qty, &engine.qty) != std::cmp::Ordering::Equal {
        return Some(format!(
            "HALTED_RESTORE: {leg} bacak MİKTAR kernel={} ↔ engine={} — uzlaştırma gerekli (POST /actions {{\"action\":\"RESET_KERNEL\"}} ile açılır; PAPER'da pozisyon mirror'a hizalanır)",
            fmt(&kernel.qty),
            fmt(&engine.qty)
        ));
    }
    if kernel.direction != engine.direction {
        return Some(format!(
            "HALTED_RESTORE: {leg} bacak YÖN kernel={} ↔ engine={} — uzlaştırma gerekli (POST /actions {{\"action\":\"RESET_KERNEL\"}})",
            kernel.direction,
            engine.direction
        ));
    }
    let tick_up = tick.clone();
    let tick_dn = tick.neg().unwrap_or_else(|_| tick.clone());
    let avg_ok = match (&kernel.avg_price, &engine.avg_price) {
        (None, None) => true,
        (Some(k), Some(e)) => {
            let d = k.sub(e).unwrap_or_else(|_| Fraction::zero());
            Fraction::cmp(&d, &tick_up) != std::cmp::Ordering::Greater
                && Fraction::cmp(&d, &tick_dn) != std::cmp::Ordering::Less
        }
        _ => false,
    };
    if !avg_ok {
        return Some(format!(
            "HALTED_RESTORE: {leg} bacak ORTALAMA kernel={} ↔ engine={} (±1 tick tolerans) — uzlaştırma gerekli (POST /actions {{\"action\":\"RESET_KERNEL\"}})",
            kernel.avg_price.as_ref().map(|v| fmt(v)).unwrap_or_else(|| "yok".into()),
            engine.avg_price.as_ref().map(|v| fmt(v)).unwrap_or_else(|| "yok".into())
        ));
    }
    None
}

/// Faz 4 (H-6): botun VENUE'undan enstrüman çözümü — OKX/BINANCE botu artık
/// Bybit mikro-yapısıyla değil kendi borsasının tick/step/fee'siyle çalışır.
/// PAPER simülasyonu BYBIT piyasa verisiyle koştuğundan PAPER → BYBIT çözülür.
pub(crate) async fn engine_instrument_for(
    cfg: &BotConfig,
    venue: &str,
    app: &Arc<AppState>,
) -> Instrument {
    let category = if cfg.kind.is_futures() {
        "linear"
    } else {
        "spot"
    };
    let venue = if venue == "PAPER" { "BYBIT" } else { venue };
    let cache_key = format!("{venue}:{category}:{}", cfg.symbol);
    {
        let cache = app.instruments_cache.lock().await;
        if let Some((at, list)) = cache.get(&cache_key) {
            if now_ms() - *at < INSTRUMENTS_CACHE_TTL_MS {
                if let Some(i) = list.first() {
                    return i.clone();
                }
            }
        }
    }
    let fetched = match app.adapter(venue) {
        Some(a) => a.instruments(category, Some(&cfg.symbol)).await.ok(),
        None => None,
    };
    match fetched.and_then(|l| l.into_iter().next()) {
        Some(i) => {
            let mut cache = app.instruments_cache.lock().await;
            cache.insert(cache_key, (now_ms(), vec![i.clone()]));
            i
        }
        // Honest fallback: a failed fetch returns defaults for THIS call only —
        // the fallback is never cached, so the next poll retries the live fetch
        // instead of trading on BTC-default microstructure until restart.
        None => {
            let mut i = Instrument::default();
            i.symbol = cfg.symbol.clone();
            i
        }
    }
}

/// M-3: instrument cache TTL — venue hataları/haberleşme kesintileri sonrası
/// bayat mikro-yapıyla uzun süre çalışmamak için.
pub const INSTRUMENTS_CACHE_TTL_MS: i64 = 60_000;

/// create_bot ack-flow için enstrüman çözümü (cache'li — create_runtime'da
/// ikinci çağrı bedava). Task 16: grid fee-breaker ön-denetimi bunu kullanır.
pub(crate) async fn resolve_instrument(
    cfg: &BotConfig,
    venue: &str,
    app: &Arc<AppState>,
) -> Instrument {
    engine_instrument_for(cfg, venue, app).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Task 15-a item 9f-(4): HEDGE_DUAL runtimes are real HedgeEngines — the
    /// snapshot phase exposes the hedge lifecycle ("MainOnly/...") instead of
    /// the DCA phases, and the snapshot carries the bot's config object
    /// (Task 15-a item 4 — the UI seeds TP/SL from it).
    #[test]
    fn halt_message_roundtrip_and_reject_garbage() {
        let m = halt_event_message("engaged", Some("test"), Some("UI"), 123, false);
        let (action, reason, by, at_ms, auto) = parse_halt_message(&m).expect("parse");
        assert_eq!(action, "engaged");
        assert_eq!(reason.as_deref(), Some("test"));
        assert_eq!(by.as_deref(), Some("UI"));
        assert_eq!(at_ms, 123);
        assert!(!auto);
        assert!(parse_halt_message("bozuk").is_none());
        assert!(parse_halt_message("{\"action\":\"patladı\"}").is_none());
    }

    #[test]
    fn live_halt_matrix_matches_v3_semantics() {
        // kill: LIVE durur; freeze: LIVE durur; PAPER her ikisinde de sürer.
        assert!(live_halted("LIVE", true, false));
        assert!(live_halted("LIVE", false, true));
        assert!(live_halted("LIVE", true, true));
        assert!(!live_halted("LIVE", false, false));
        assert!(!live_halted("PAPER", true, false));
        assert!(!live_halted("PAPER", false, true));
        assert!(!live_halted("PAPER", false, false));
    }

    #[tokio::test]
    async fn create_runtime_hedge_dual_snapshot_has_hedge_phase_and_config() {
        std::env::set_var("DCA_VAULT_KEY", "test-test-test-test-test-test-test-test");
        let dir = std::env::temp_dir().join(format!("dca-core-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let db_path = dir.join("state-test.sqlite3");
        let db = Db::open(db_path.to_str().unwrap()).expect("db");
        let vault = Vault::new(&db).expect("vault");
        let (app, _rx) = AppState::new(db, vault);
        // Pre-seed the instrument cache so the test is deterministic offline.
        let inst = Instrument::default();
        app.instruments_cache
            .lock()
            .await
            .insert("spot:BTCUSDT".into(), (now_ms(), vec![inst]));

        let mut cfg = BotConfig::default();
        cfg.kind = StrategyKind::HedgeDual;
        cfg.symbol = "BTCUSDT".into();
        create_runtime(
            &app,
            "b-hd-test",
            "HD Test",
            "PAPER",
            "PAPER",
            "RUNNING",
            cfg,
            "1h",
            Fraction::from_u64(100_000),
            "INTERNAL_PAPER",
            None,
        )
        .await
        .expect("hedge runtime");

        let rts = app.runtimes.lock().await;
        let rt = rts.get("b-hd-test").expect("runtime registered");
        let v = rt.snapshot_json();
        let phase = v["stats"]["phase"].as_str().expect("phase string");
        assert!(
            phase.contains("MainOnly"),
            "hedge snapshot phase must expose the hedge lifecycle, got: {phase}"
        );
        assert!(
            matches!(rt.engine_kind, dca_strategy::EngineTag::Hedge(_)),
            "HEDGE_DUAL must not fall back to a DCA engine"
        );
        assert!(
            v["config"].is_object(),
            "snapshot must carry the bot config for the UI"
        );
        assert_eq!(v["config"]["kind"].as_str(), Some("HEDGE_DUAL"));
        drop(rts);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
