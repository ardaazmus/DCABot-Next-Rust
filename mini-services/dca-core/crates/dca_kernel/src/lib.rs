// Faz 21 (CI): test builder'ları Default + alan-ataması ile kurulur (okunabilirlik).
#![cfg_attr(test, allow(clippy::field_reassign_with_default))]

//! dca_kernel — Single Event Kernel (Backtest = Paper = Live, ADR-001).
//! SimulationStepper drives bar-by-bar execution through the 8 fail-closed gates.
//! Task 16: korumalar UserRiskPolicy ile ZORUNLU veya UYARI+İZİN modunda çalışır
//! (advisory override); fiziksel değişmezler (G1 veri bütünlüğü, G2 fee, G6 sermaye)
//! esnetilemez.

pub mod risk_policy;

use dca_math::fraction::{Fraction, MathError};
use dca_math::m04::PositionState;
use dca_math::{Instrument, Side};
use dca_risk::{
    gate1_bar_integrity, gate2_fee_guard, gate3_slippage, gate4_no_lookahead, gate5_limit_realism,
    gate6_capital_guard, gate7_session_guard, gate8_gap_protection, CapitalShortfallMode, FillRealism,
    GateVerdict,
};
use serde::{Deserialize, Serialize};
use std::fmt;

pub use risk_policy::{AdvisoryWarning, FillModel, GuardAction, RiskSessionType, UserRiskPolicy};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bar {
    pub t: i64,
    pub o: String,
    pub h: String,
    pub l: String,
    pub c: String,
    pub v: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OrderKind {
    Add,
    Reduce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderIntent {
    pub id: String,
    pub side: Side,
    pub kind: OrderKind,
    pub price: Option<Fraction>, // None => market
    pub qty: Fraction,
    /// MARKET orders carry a decision timestamp (Gate 4 eligibility).
    pub decision_ts: i64,
    /// Role label for audit (BASE, SAFETY, TP, SL, HEDGE, MANUAL).
    pub role: String,
    /// Market reference price AT PLACEMENT TIME (Gate 5 marketable check).
    pub placement_ref_price: Option<Fraction>,
    /// Faz 22 (P1-21): QuoteNotional market emri — dolum anında qty GERÇEK
    /// dolum fiyatından çözülür (qty = notional / fill_price); plan-anı qty
    /// alanı yalnız tahmin/kapı girdisi olarak kalır. Böylece harcanan quote
    /// planla birebir tutar (kapanış ↔ açılış drifti artık miktara sızamaz).
    #[serde(default)]
    pub quote_notional: Option<Fraction>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FillRecord {
    pub fill_key: String,
    pub order_id: String,
    pub role: String,
    pub side: Side,
    pub kind: OrderKind,
    pub price: Fraction,
    pub qty: Fraction,
    pub fee: Fraction,
    pub fee_role: FeeRole,
    pub realism: FillRealism,
    pub ts: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeeRole {
    Maker,
    Taker,
}

impl fmt::Display for FeeRole {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FeeRole::Maker => write!(f, "MAKER"),
            FeeRole::Taker => write!(f, "TAKER"),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct GateLog {
    pub ts: i64,
    pub gate: String,
    pub verdict: String,
    pub detail: String,
}

#[derive(Debug, Clone)]
pub struct StepperConfig {
    pub session_type: dca_math::sessions::SessionType,
    pub capital_shortfall_mode: CapitalShortfallMode,
    pub slippage_bps: Fraction,
    pub max_volume_share: Fraction,
    pub is_spot: bool,
    /// Linear futures leverage (margin mode when is_spot == false). 1x = full notional.
    pub leverage: Fraction,
    /// Task 16 — kullanıcı risk politikası (advisory override'lar buradan okunur).
    pub policy: UserRiskPolicy,
}

impl Default for StepperConfig {
    fn default() -> Self {
        Self {
            session_type: dca_math::sessions::SessionType::Crypto24_7,
            capital_shortfall_mode: CapitalShortfallMode::Wait,
            slippage_bps: Fraction::from_u64(5),
            max_volume_share: Fraction::from_decimal_str("0.1").unwrap(),
            is_spot: true,
            leverage: Fraction::one(),
            policy: UserRiskPolicy::default(),
        }
    }
}

/// Unified kernel stepper: identical logic for backtest, paper replay, live shadow.
pub struct SimulationStepper {
    pub instrument: Instrument,
    pub cfg: StepperConfig,
    pub position: PositionState,
    pub start_cash: Fraction,
    pub cash: Fraction,
    /// Initial margin committed to the open position (margin mode only).
    pub im_used: Fraction,
    pub cumulative_fees: Fraction,
    pub cumulative_funding: Fraction,
    /// Faz 18 (P1-1): yalnız KAPANAN (reduce) miktarın komisyonu — giriş
    /// komisyonları bunu şişirmez. realized_net tanımının bileşeni.
    pub realized_fees: Fraction,
    pub realized_net: Fraction,
    pub pending: Vec<OrderIntent>,
    pub fills: Vec<FillRecord>,
    pub gate_logs: Vec<GateLog>,
    pub halted: Option<String>,
    pub prev_bar_ts: Option<i64>,
    pub bar_interval_ms: i64,
    filled_qty_this_bar: Fraction,
    current_bar_ts: i64,
    pub last_close: Option<String>,
    /// Drain cursor: loglar işlendi olarak işaretlendi (runtime RISK SSE tüketicisi).
    gate_log_cursor: usize,
}

/// Faz 21 (P1-8): yakın-kademe SO-fonu — `fill_price` ile piyas arasında
/// (Buy merdiveni: daha YÜKSEK fiyatlı; Sell: daha DÜŞÜK) bekleyen Add
/// emirlerinin notional+taker-fee toplamı. Bu emir doluma erişmeden önce
/// yakın kademeler dolacağı için nakit onlara ayrılmalıdır; aksi halde uzak
/// bir kademe yakının sermayesini "ilk gelen alır" mantığıyla kilitler.
fn closer_pending_reserved(
    pending: &[OrderIntent],
    fill_price: &Fraction,
    is_buy_ladder: bool,
    taker_fee_rate: &Fraction,
    leverage: &Fraction,
    is_spot: bool,
) -> Result<Fraction, String> {
    let mut acc = Fraction::zero();
    for p in pending {
        if p.kind != OrderKind::Add {
            continue;
        }
        let Some(pp) = &p.price else { continue };
        let closer = if is_buy_ladder {
            Fraction::cmp(pp, fill_price) == std::cmp::Ordering::Greater
        } else {
            Fraction::cmp(pp, fill_price) == std::cmp::Ordering::Less
        };
        if !closer {
            continue;
        }
        let notional = p.qty.mul(pp).map_err(|e| e.to_string())?;
        let fee = notional.mul(taker_fee_rate).map_err(|e| e.to_string())?;
        let principal = if is_spot {
            notional
        } else {
            let effective_leverage = if leverage.is_positive() { leverage.clone() } else { Fraction::one() };
            notional.div(&effective_leverage).map_err(|e| e.to_string())?
        };
        let required = principal.add(&fee).map_err(|e| e.to_string())?;
        acc = acc.add(&required).map_err(|e| e.to_string())?;
    }
    Ok(acc)
}

impl SimulationStepper {
    pub fn new(instrument: Instrument, start_cash: Fraction, cfg: StepperConfig) -> Self {
        let dir = 1_i8; // engine-specific; strategies set their own position dir
        Self {
            instrument,
            cfg,
            position: PositionState::new(dir),
            start_cash: start_cash.clone(),
            cash: start_cash,
            im_used: Fraction::zero(),
            cumulative_fees: Fraction::zero(),
            cumulative_funding: Fraction::zero(),
            realized_fees: Fraction::zero(),
            realized_net: Fraction::zero(),
            pending: Vec::new(),
            fills: Vec::new(),
            gate_logs: Vec::new(),
            halted: None,
            prev_bar_ts: None,
            bar_interval_ms: 0,
            filled_qty_this_bar: Fraction::zero(),
            current_bar_ts: 0,
            last_close: None,
            gate_log_cursor: 0,
        }
    }

    pub fn set_position_direction(&mut self, dir: i8) {
        self.position.direction = dir;
    }

    fn log_gate(&mut self, ts: i64, gate: &str, v: &GateVerdict) {
        let (verdict, detail) = match v {
            GateVerdict::Pass => ("PASS".to_string(), String::new()),
            GateVerdict::Defer(d) => ("DEFER".to_string(), d.clone()),
            GateVerdict::Reject(d) => ("REJECT".to_string(), d.clone()),
            GateVerdict::Halt(d) => ("HALT".to_string(), d.clone()),
        };
        self.gate_logs.push(GateLog { ts, gate: gate.to_string(), verdict, detail });
    }

    /// Advisory kaydı: koruma kullanıcı izniyle aşıldı (Acknowledge & Proceed).
    /// Runtime'da RISK SSE olayına, backtest'te gate-log satırına dönüşür.
    fn log_advisory(&mut self, ts: i64, gate: &str, code: &str, detail: &str) {
        self.gate_logs.push(GateLog {
            ts,
            gate: gate.to_string(),
            verdict: format!("ADVISORY:{code}"),
            detail: detail.to_string(),
        });
    }

    /// Son drain'den bu yana eklenen gate logları (runtime RISK SSE tüketicisi).
    pub fn drain_new_gate_logs(&mut self) -> Vec<GateLog> {
        let out: Vec<GateLog> = self.gate_logs[self.gate_log_cursor.min(self.gate_logs.len())..].to_vec();
        self.gate_log_cursor = self.gate_logs.len();
        out
    }

    /// Dış modüller için (backtest breaker/rejim uyarıları) advisory kaydı.
    pub fn log_advisory_pub(&mut self, ts: i64, gate: &str, code: &str, detail: &str) {
        self.log_advisory(ts, gate, code, detail);
    }

    /// Advance one closed bar. Order: Gate1 → fills of resting orders (G5/G6) →
    /// session/gap gates on pending adds → market fills from previous decisions.
    pub fn step(&mut self, bar: &Bar, ts_ms: i64, macro_blackout: bool) -> Result<Vec<FillRecord>, String> {
        if self.halted.is_some() {
            return Err(format!("kernel halted: {}", self.halted.clone().unwrap()));
        }
        // Faz 18 (P1-13): deterministik zaman — karar zaman damgası geçersizse
        // (sistem saati sıfırlandı/geriye sardı) sessiz devam etmeyiz.
        if ts_ms <= 0 {
            let reason = format!("kernel: invalid decision timestamp {ts_ms}");
            self.halted = Some(reason.clone());
            return Err(reason);
        }
        // Gate 1 — data integrity (HALT on violation)
        let o = Fraction::from_decimal_str(&bar.o).map_err(|e| e.to_string())?;
        let h = Fraction::from_decimal_str(&bar.h).map_err(|e| e.to_string())?;
        let l = Fraction::from_decimal_str(&bar.l).map_err(|e| e.to_string())?;
        let c = Fraction::from_decimal_str(&bar.c).map_err(|e| e.to_string())?;
        let v = Fraction::from_decimal_str(&bar.v).map_err(|e| e.to_string())?;

        let verdict = gate1_bar_integrity(self.prev_bar_ts, bar.t, &o, &h, &l, &c, &v, self.bar_interval_ms);
        self.log_gate(ts_ms, "GATE1", &verdict);
        match &verdict {
            GateVerdict::Halt(reason) => {
                // P0-2: Halt yalnız GERÇEK veri bozulmasına (OHLC/negatif/monotonik)
                // ayrıldı — kalıcıdır, RESET_KERNEL action ile açılır.
                self.halted = Some(reason.clone());
                return Err(reason.clone());
            }
            GateVerdict::Reject(reason) if reason.contains("interval gap") => {
                // P0-2: veri boşluğu (PAUSE→START, borsa bakımı, uzun kesinti)
                // artık sticky halt DEĞİL — "skip & resync": imleç son kapanmış
                // bara hizalanır, interval yeniden öğrenilir, bu bar işlenir
                // (Freqtrade startup/resync yaklaşımı). Uygulama GATE1 resync'i
                // kendisi loglar (WARN advisory).
                self.log_gate(ts_ms, "GATE1", &GateVerdict::Pass);
                self.log_advisory(ts_ms, "GATE1", "INTERVAL_RESYNC", &format!(
                    "veri boşluğu saptandı ({}) — imleç son kapanmış bara hizalandı, işlem sürüyor", reason));
            }
            GateVerdict::Reject(reason) => {
                self.halted = Some(reason.clone());
                return Err(reason.clone());
            }
            _ => {}
        }
        // Faz 22 (P1-15): GELECEK-ZAMAN barı — karar saati (ts_ms) bar
        // kapanışından önceyse veri henüz kapanmamıştır; işlenmez, imleç
        // İLERLETİLMEZ (bar saat döndüğünde tekrar değerlendirilir).
        // (defense-in-depth: engine zaten `b.t + interval <= now` filtreler.)
        if bar.t > ts_ms {
            self.log_advisory(
                ts_ms,
                "GATE1",
                "FUTURE_BAR_SKIP",
                &format!("bar kapanışı ({}) karar saatinden ({}) ileride — bar henüz işlenmedi", bar.t, ts_ms),
            );
            return Ok(vec![]);
        }
        // Spike tespiti için ÖNCEKİ kapanış yakalanır (last_close hemen altta
        // bu barın kapanışına döner).
        let prev_close = self.last_close.clone();
        if let Some(pt) = self.prev_bar_ts {
            self.bar_interval_ms = bar.t - pt;
        }
        self.prev_bar_ts = Some(bar.t);
        self.filled_qty_this_bar = Fraction::zero();
        self.current_bar_ts = bar.t;
        self.last_close = Some(bar.c.clone());

        // Faz 18 (P1-15): donmuş/likiditesiz bar (hacim 0) — imleç ilerletildi;
        // dolum kararı yok, dinlenen emirler sonraki likit bara devrediyor.
        if matches!(verdict, GateVerdict::Defer(_)) {
            self.log_advisory(
                ts_ms,
                "GATE1",
                "FROZEN_BAR_SKIP",
                "sıfır hacimli bar — dolum kararı uygulanmadı, emirler sonraki likit bara devredildi",
            );
            return Ok(vec![]);
        }
        // NOT (P1-15, bilinçli): h == l tek başına dolum engelleyici DEĞİL —
        // sabit fiyatlı barlar meşrudur (stabilcoin, seyrek piyasa) ve dolum
        // gerçekçiliğini GATE5 penetrasyon kuralları, donmuşluğu hacim-0
        // kontrolü zaten yönetir. (İlk turda eklenmişti, backtest'in düz
        // bar'lı verisiyle çelişti — kaldırıldı.)
        // Faz 22 (P1-15): FİYAT SPIKE'ı — tek barda %50+ kapanış sıçraması
        // (kirli veri / fitil); o barın dolum kararı verilmez, emirler
        // sonraki normal bara devreder (kalıcı halt DEĞİL).
        if let Some(pc_str) = prev_close {
            if let Ok(pc) = Fraction::from_any_str(&pc_str) {
                if let Ok(diff) = c.sub(&pc).and_then(|d| d.abs()) {
                    if let Ok(ratio) = diff.div(&pc) {
                        if Fraction::cmp(&ratio, &Fraction::from_decimal_str("0.5").unwrap_or_else(|_| Fraction::zero()))
                            == std::cmp::Ordering::Greater
                        {
                            self.log_advisory(
                                ts_ms,
                                "GATE1",
                                "PRICE_SPIKE_SKIP",
                                &format!(
                                    "tek barda %{} fiyat sıçraması ({} → {}) — bar fitil/kirli veri sayıldı, dolum kararı uygulanmadı",
                                    ratio.mul(&Fraction::from_u64(100)).map(|x| x.to_half_even_display(1)).unwrap_or_else(|_| "?".into()),
                                    pc.to_half_even_display(4),
                                    c.to_half_even_display(4)
                                ),
                            );
                            return Ok(vec![]);
                        }
                    }
                }
            }
        }

        let mut new_fills = Vec::new();

        // Task 16 — Makro kara pencere (Gate 7): çağıran parametresi + dahili
        // FOMC/TÜFE takvimi (bar aralığı [t-interval, t] pencereyle kesişiyorsa).
        let bar_from = bar.t - self.bar_interval_ms.max(1);
        let calendar_hit = dca_math::macro_calendar::range_overlaps_blackout(bar_from, bar.t);
        let macro_eff = macro_blackout || calendar_hit.is_some();

        // Task 16 Gate 8b — aynı barda açılış fiyatından doldurulacak SAFETY
        // emirleri burada toplanır: Enforce → TEK sentetik VWAP dolum;
        // Permissive → tek tek dolum (her seviyeye ayrı komisyon) + uyarı.
        let mut gap_batch: Vec<OrderIntent> = Vec::new();
        let mut gap_batch_qty = Fraction::zero();
        let gap_consolidate_on = self.cfg.policy.gap_consolidation == GuardAction::Enforce;
        // Faz 21 (P1-10): hacim payı kapısı konfigürasyondan (ölü alan canlandı).
        let volume_cap = v.mul(&self.cfg.max_volume_share).unwrap_or_else(|_| Fraction::zero());

        // Intrabar evaluation of resting limit orders (penetration + realism + capital)
        let remaining = std::mem::take(&mut self.pending);
        'orders: for (order_index, order) in remaining.iter().cloned().enumerate() {
            let Some(target) = &order.price else {
                // market orders scheduled for this bar open (Gate 4 eligible)
                if order.decision_ts > bar.t {
                    self.pending.push(order);
                    continue;
                }
                // Gate 7 session guard on market adds (policy-aware)
                let g7 = gate7_session_guard(self.cfg.session_type, ts_ms, macro_eff, order.kind == OrderKind::Reduce);
                self.log_gate(ts_ms, "GATE7", &g7);
                if let GateVerdict::Reject(r) = g7 {
                    if order.kind == OrderKind::Reduce {
                        continue; // risk-reducing exits are never blocked by G7 anyway
                    }
                    // Advisory override (Acknowledge & Proceed): kullanıcı riski
                    // üstlendi — emir kesintisiz borsaya iletilir.
                    let session_closed = !dca_math::sessions::is_market_open(
                        dca_math::sessions::SessionType::from(self.cfg.policy.session_type),
                        chrono::DateTime::from_timestamp_millis(ts_ms).unwrap_or_else(chrono::Utc::now),
                    );
                    let permissive = (session_closed && self.cfg.policy.session_guard.is_permissive())
                        || (!session_closed && macro_eff && self.cfg.policy.macro_blackout.is_permissive());
                    if permissive {
                        let code = if session_closed { "SESSION_CLOSED" } else { "MACRO_BLACKOUT" };
                        self.log_advisory(
                            ts_ms,
                            "GATE7",
                            code,
                            &format!("{r} — kullanıcı izniyle aşıldı (Acknowledge & Proceed); likidite sığ, spread geniş — emir borsaya iletiliyor"),
                        );
                        // proceed to fill below
                    } else {
                        continue; // Enforce: fail-closed (eski davranış)
                    }
                }
                let fill_price = match gate3_slippage(order.side, &o, &self.cfg.slippage_bps) {
                    Ok(p) => p,
                    Err(e) => {
                        self.halted = Some(e.clone());
                        return Err(e);
                    }
                };
                // Faz 22 (P1-21): QuoteNotional market emri — qty GERÇEK dolum
                // fiyatından (açılış ± slippage) çözülür; plan-anı qty tahmini
                // atılır. Kapanış ↔ açılış drifti artık harcanan quote'a
                // sızmaz (3Commas quote-spend semantiği).
                let mut order = order;
                if let Some(qn) = order.quote_notional.clone() {
                    if !qn.is_zero() && !fill_price.is_zero() {
                        let solved = dca_math::m02::qty_down(&qn.div(&fill_price).unwrap_or_else(|_| Fraction::zero()), &self.instrument.step_size)
                            .unwrap_or_else(|_| Fraction::zero());
                        if solved.is_zero() {
                            self.log_advisory(
                                ts_ms,
                                "M02",
                                "QUOTE_QTY_BELOW_STEP",
                                &format!("quote notional {} dolum fiyatında adım boyutunun altında — emir bu bar atlandı", qn.to_half_even_display(8)),
                            );
                            continue;
                        }
                        order.qty = solved;
                    }
                }
                // Faz 21 (P1-10): piyasa Add'ler de hacim payı kapısından geçer
                // (eski kodda piyasa yolunda kapı hiç yoktu); risk-azaltan
                // Reduce muaf (N-7). Kapıyı aşan emir sonraki bara devreder.
                // Not (P1-8, bilinçli): piyasa Add için yakın-kademe rezervasyonu
                // YOKTUR — market emri tanım gereği en yakın kademedir (kendi
                // dolumu anında tahsis edilir), rezervlenecek "daha yakın bekleyen"
                // kavramı yalnız LIMIT merdiveninde anlamlıdır.
                if order.kind == OrderKind::Add {
                    let cap = v.mul(&self.cfg.max_volume_share).unwrap_or_else(|_| Fraction::zero());
                    let total = self
                        .filled_qty_this_bar
                        .add(&order.qty)
                        .unwrap_or_else(|_| Fraction::zero());
                    if Fraction::cmp(&cap, &total) == std::cmp::Ordering::Less {
                        let verdict = GateVerdict::Defer("GATE5: market add exceeds bar volume share cap".into());
                        self.log_gate(ts_ms, "GATE5", &verdict);
                        self.pending.push(order);
                        continue;
                    }
                }
                // Faz 18 (P1-8): planlı piyasa Add'leri de sermaye kapısından
                // geçer — dolum öncesi GERÇEK kalan bakiye kontrol edilir.
                // Wait modunda emir sonraki bara devreder, Skip modunda atlanır.
                if order.kind == OrderKind::Add {
                    let fee = order
                        .qty
                        .mul(&fill_price)
                        .map_err(|e| e.to_string())?
                        .mul(&self.instrument.taker_fee_rate)
                        .map_err(|e| e.to_string())?;
                    let g6 = gate6_capital_guard(
                        &order.qty,
                        &fill_price,
                        &fee,
                        &self.cfg.leverage,
                        self.cfg.is_spot,
                        &self.cash,
                        &self.cfg.capital_shortfall_mode,
                    );
                    self.log_gate(ts_ms, "GATE6", &g6);
                    match g6 {
                        GateVerdict::Pass => {}
                        GateVerdict::Defer(_) => {
                            self.pending.push(order);
                            continue;
                        }
                        _ => continue,
                    }
                }
                if let Some(f) = self.execute_fill(&order, &fill_price, FeeRole::Taker, FillRealism::MarketableTaker, bar.t, None)? {
                    new_fills.push(f);
                }
                continue;
            };

            // Gate 5 — realism & penetration (dolum modeli: REALISTIC/OPTIMISTIC)
            // Faz 21 (N-7): risk-azaltan (Reduce/TP/SL) emirler hacim payı
            // kapısından muaf; Faz 21 (P1-10): pay konfigürasyondan gelir.
            let placement_ref = order.placement_ref_price.clone().unwrap_or_else(|| target.clone());
            let (realism, g5) = gate5_limit_realism(
                order.side,
                target,
                &placement_ref,
                &o,
                &h,
                &l,
                &order.qty,
                &v,
                &self.filled_qty_this_bar,
                self.cfg.policy.fill_model == FillModel::Optimistic,
                &self.cfg.max_volume_share,
                order.kind == OrderKind::Reduce,
            );
            self.log_gate(ts_ms, "GATE5", &g5);
            match g5 {
                GateVerdict::Pass => {}
                GateVerdict::Defer(_) => {
                    self.pending.push(order);
                    continue;
                }
                _ => {
                    self.pending.push(order);
                    continue;
                }
            }
            // Gap-better fills at open price
            let fill_price = if realism == FillRealism::OpenGapBetter {
                o.clone()
            } else {
                target.clone()
            };
            let fee_role = if realism == FillRealism::MarketableTaker { FeeRole::Taker } else { FeeRole::Maker };
            // Gate 6 — capital guard (adds only) — FİZİKSEL DEĞİŞMEZ, esnetilemez.
            // Faz 18 (P1-8): defter aynı barda sıralı dolumlarla tutarlıdır —
            // her dolum bakiyeyi anında düşer, sonraki emrin G6'sı GERÇEK kalan
            // bakiyeyi görür. Beklemedeki (sonraki bara kalan) emirler kendi
            // dolum anlarında yeniden kontrol edilir.
            // Faz 21 (P1-8): yakın-kademe SO-fonu — bu emirden piyasaya DAHA
            // YAKIN bekleyen Add'lerin maliyeti rezerve edilir; nakit yetmezse
            // dolum sırasını yakın kademeler kazanır (uzak kademe Wait'te
            // bekler — kör ilk-gelen-alır değil, öncelikli tahsis).
            if order.kind == OrderKind::Add {
                let fee = order.qty.mul(&fill_price).map_err(|e| e.to_string())?.mul(&self.instrument.taker_fee_rate).map_err(|e| e.to_string())?;
                // self.pending contains already-deferred orders; remaining
                // contains later orders that have not yet been visited. Both
                // groups must reserve closer Add capital for this fill.
                let mut reserve_candidates = self.pending.clone();
                reserve_candidates.extend(remaining.iter().skip(order_index + 1).cloned());
                let reserved = closer_pending_reserved(
                    &reserve_candidates,
                    &fill_price,
                    order.side == Side::Buy,
                    &self.instrument.taker_fee_rate,
                    &self.cfg.leverage,
                    self.cfg.is_spot,
                )?;
                let available = self.cash.sub(&reserved).unwrap_or_else(|_| Fraction::zero());
                let g6 = gate6_capital_guard(
                    &order.qty,
                    &fill_price,
                    &fee,
                    &self.cfg.leverage,
                    self.cfg.is_spot,
                    &available,
                    &self.cfg.capital_shortfall_mode,
                );
                self.log_gate(ts_ms, "GATE6", &g6);
                match g6 {
                    GateVerdict::Pass => {}
                    GateVerdict::Defer(_) => {
                        self.pending.push(order);
                        continue 'orders;
                    }
                    _ => continue 'orders,
                }
            }
            // Gate 8 — opening salvo tamponu (policy-aware). Faz 18 (P1-14):
            // gap-through birleştirme kaldırıldı; gerçek birleştirme aşağıdaki
            // Gate 8b gap_batch yolunda gerçek adet + sentetik VWAP ile yapılır.
            if let dca_risk::Gate8Outcome::DeferOpeningSalvo = gate8_gap_protection(order.role == "SAFETY", ts_ms) {
                if order.role == "SAFETY" && self.cfg.policy.opening_salvo_buffer.is_permissive() {
                    self.log_advisory(
                        ts_ms,
                        "GATE8",
                        "OPENING_SALVO_BYPASS",
                        "Açılış salvo tamponu kullanıcı tarafından devre dışı — ilk 5 dakika (09:30–09:35 ET, spread 550 bps'e kadar) içinde emir gönderiliyor; sahte fitil riski üstlenildi",
                    );
                    // proceed
                } else {
                    self.pending.push(order);
                    continue;
                }
            }
            // Task 16 Gate 8b — gap-through SAFETY adds batch (aynı bar, aynı açılış fiyatı)
            if realism == FillRealism::OpenGapBetter && order.kind == OrderKind::Add && order.role == "SAFETY" {
                // Gate 5 hacim payı kuralını tampon dahil yeniden uygula (Arena 2 Gap Batch düzeltmesi)
                let candidate_batch = gap_batch_qty.add(&order.qty).unwrap_or_else(|_| gap_batch_qty.clone());
                let total = self
                    .filled_qty_this_bar
                    .add(&candidate_batch)
                    .unwrap_or_else(|_| Fraction::zero());
                if Fraction::cmp(&volume_cap, &total) == std::cmp::Ordering::Less {
                    self.pending.push(order);
                    continue;
                }
                gap_batch_qty = candidate_batch;
                gap_batch.push(order);
                continue;
            }
            if let Some(f) = self.execute_fill(&order, &fill_price, fee_role, realism, bar.t, None)? {
                new_fills.push(f);
            }
        }
        // Gate 8b flush — toplanan gap dolumlarını işle
        if !gap_batch.is_empty() {
            let n = gap_batch.len();
            if n >= 2 && gap_consolidate_on {
                // Tek sentetik VWAP dolum: tüm atlanan miktar açılış fiyatından,
                // tek maker komisyonla. order_id ".m{n}" eki motorun kademe
                // sayacını doğru ilerletmesini sağlar (DcaEngine.on_fill).
                let total_qty = gap_batch.iter().fold(Fraction::zero(), |acc, ord| {
                    acc.add(&ord.qty).unwrap_or_else(|_| acc.clone())
                });
                let side = gap_batch[0].side;
                let merged_id = format!("{}.m{n}", gap_batch[0].id);
                self.log_advisory(
                    ts_ms,
                    "GATE8",
                    "GAP_CONSOLIDATED",
                    &format!("Fiyat {n} kademeyi aştı — tüm atlanan miktar açılış fiyatından TEK dolumda konsolide edildi (tek komisyon, ortalama maliyet hemen tabana çekildi)"),
                );
                let merged = OrderIntent {
                    id: merged_id,
                    side,
                    kind: OrderKind::Add,
                    price: Some(o.clone()),
                    qty: total_qty.clone(),
                    decision_ts: ts_ms,
                    role: "SAFETY".into(),
                    placement_ref_price: Some(o.clone()),
                quote_notional: None,
                };
                // Faz 18 (P1-8): konsolide gap dolumu da sermaye kapısından geçer
                // (gerçek ücret: maker — tek komisyon semantiği).
                let fee = total_qty
                    .mul(&o)
                    .map_err(|e| e.to_string())?
                    .mul(&self.instrument.maker_fee_rate)
                    .map_err(|e| e.to_string())?;
                let g6 = gate6_capital_guard(
                    &total_qty,
                    &o,
                    &fee,
                    &self.cfg.leverage,
                    self.cfg.is_spot,
                    &self.cash,
                    &self.cfg.capital_shortfall_mode,
                );
                self.log_gate(ts_ms, "GATE6", &g6);
                match g6 {
                    GateVerdict::Pass => {
                        if let Some(f) = self.execute_fill(&merged, &o, FeeRole::Maker, FillRealism::OpenGapBetter, bar.t, None)? {
                            new_fills.push(f);
                        }
                    }
                    GateVerdict::Defer(_) => {
                        for ord in gap_batch {
                            self.pending.push(ord);
                        }
                    }
                    _ => {}
                }
            } else {
                if n >= 2 {
                    self.log_advisory(
                        ts_ms,
                        "GATE8",
                        "GAP_NO_CONSOLIDATION",
                        &format!("Gap konsolidasyonu kapalı — atlanan {n} seviye tek tek dolduruluyor; her seviye için ayrı komisyon ödenecek, ortalama maliyet daha yavaş iyileşir"),
                    );
                }
                for order in gap_batch {
                    // Faz 18 (P1-8): tek tek gap dolumu da sermaye kapısından geçer.
                    let fee = order
                        .qty
                        .mul(&o)
                        .map_err(|e| e.to_string())?
                        .mul(&self.instrument.maker_fee_rate)
                        .map_err(|e| e.to_string())?;
                    let g6 = gate6_capital_guard(
                        &order.qty,
                        &o,
                        &fee,
                        &self.cfg.leverage,
                        self.cfg.is_spot,
                        &self.cash,
                        &self.cfg.capital_shortfall_mode,
                    );
                    self.log_gate(ts_ms, "GATE6", &g6);
                    match g6 {
                        GateVerdict::Pass => {
                            if let Some(f) = self.execute_fill(&order, &o, FeeRole::Maker, FillRealism::OpenGapBetter, bar.t, None)? {
                                new_fills.push(f);
                            }
                        }
                        GateVerdict::Defer(_) => self.pending.push(order),
                        _ => {}
                    }
                }
            }
        }
        Ok(new_fills)
    }

    fn execute_fill(
        &mut self,
        order: &OrderIntent,
        price: &Fraction,
        fee_role: FeeRole,
        realism: FillRealism,
        ts: i64,
        fill_key_override: Option<&str>,
    ) -> Result<Option<FillRecord>, String> {
        self.execute_fill_inner(order, price, fee_role, realism, ts, None, fill_key_override)
    }

    /// Faz 3 (H-4): `fee_override` — venue execution kaydından gelen GERÇEK
    /// ücret. None → yerel instrument oranı (mevcut sözleşme). Venue ücreti
    /// doğrulanmış bir kayıtsa G2 yerel-oran kontrolünden muaftır (sıfır
    /// ücretli dolum — voucher/promosyon — gerçek olabilir).
    #[allow(clippy::too_many_arguments)]
    fn execute_fill_inner(
        &mut self,
        order: &OrderIntent,
        price: &Fraction,
        fee_role: FeeRole,
        realism: FillRealism,
        ts: i64,
        fee_override: Option<&Fraction>,
        // Faz 23 (derin inceleme): LIVE dış-dolumlar için borsa execId bazlı
        // anahtar — olmadan kernel `{order_id}-{ts}` yeniden üretir, execId
        // hiç muhasebe defterine ulaşmaz; sweep yeniden sorgulayınca aynı
        // dolum İKİ KEZ sayılır (m04 seen_fill_keys farklı anahtar görür).
        fill_key_override: Option<&str>,
    ) -> Result<Option<FillRecord>, String> {
        let rate = match fee_role {
            FeeRole::Maker => self.instrument.maker_fee_rate.clone(),
            FeeRole::Taker => self.instrument.taker_fee_rate.clone(),
        };
        // Gate 2 — fee guard (venue-verified fee'de muaf: borsa kaydı öncelikli)
        let g2 = if fee_override.is_some() { GateVerdict::Pass } else { gate2_fee_guard(&rate, false) };
        self.log_gate(ts, "GATE2", &g2);
        if matches!(g2, GateVerdict::Reject(_)) {
            return Ok(None);
        }
        let notional = order.qty.mul(price).map_err(|e| e.to_string())?;
        let fee = match fee_override {
            Some(f) => f.clone(),
            None => notional.mul(&rate).map_err(|e| e.to_string())?,
        };
        let is_add = order.kind == OrderKind::Add;
        let fill_key = match fill_key_override {
            Some(k) => k.to_string(),
            None => format!("{}-{}", order.id, ts),
        };
        let margin_mode = !self.cfg.is_spot;
        let lev = if self.cfg.leverage.is_zero() { Fraction::one() } else { self.cfg.leverage.clone() };
        let realized_before = self.position.realized_gross.clone();
        let avg_before = self.position.avg_price.clone();
        let qty_before = self.position.qty.clone();
        // BOT-02: idempotent fill — aynı fill_key zaten işlenmişse accounting'i
        // atla (PositionState::apply_fill false döner).
        let state_changed = self.position
            .apply_fill(&fill_key, price, &order.qty, is_add)
            .map_err(|e: MathError| e.to_string())?;
        if !state_changed {
            return Ok(None);
        }
        // Faz 22 (P1-7): reduce overfill artık YÜZEYE ÇIKAR — m04 OVERFILL'i
        // kırpıp violations'a yazıyordu ama kimse okumuyordu; pozisyon "eksik
        // kapanmış" olarak sessizce sürüyordu. Advisory defterine CRITICAL
        // seviyesinde yazılır (runtime RISK SSE tüketicisi görür).
        if !self.position.violations.is_empty()
            && self.position.violations.iter().any(|v| v.contains(&fill_key))
        {
            let last = self.position.violations.last().cloned().unwrap_or_default();
            self.log_advisory(ts, "M04", "OVERFILL", &format!("reduce overfill tespit edildi: {last}"));
        }
        if margin_mode {
            // Linear futures isolated-margin cash flow:
            //  open  : cash -= IM(notional/L) + fee ; im_used += IM
            //  close : cash += released(closed*avg_before/L) + realized_pnl - fee ; im_used -= released
            if is_add {
                let im = notional.div(&lev).map_err(|e| e.to_string())?;
                self.cash = self.cash.sub(&im.add(&fee).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
                self.im_used = self.im_used.add(&im).map_err(|e| e.to_string())?;
            } else {
                let closed = Fraction::max(&Fraction::min(&order.qty, &qty_before), &Fraction::zero());
                let released = if let Some(avg) = &avg_before {
                    closed.mul(avg).map_err(|e| e.to_string())?.div(&lev).map_err(|e| e.to_string())?
                } else {
                    Fraction::zero()
                };
                let pnl_delta = self.position.realized_gross.sub(&realized_before).map_err(|e| e.to_string())?;
                let credit = released.add(&pnl_delta).map_err(|e| e.to_string())?.sub(&fee).map_err(|e| e.to_string())?;
                self.cash = self.cash.add(&credit).map_err(|e| e.to_string())?;
                self.im_used = self.im_used.sub(&released).unwrap_or_else(|_| Fraction::zero());
            }
        } else if is_add {
            // Cash movement: buy adds cost+fee, sell adds proceeds-fee
            self.cash = self.cash.sub(&notional.add(&fee).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        } else {
            // Faz 23 (derin inceleme): SPOT reduce'da kredi CLAMP'li — m04
            // azaltmayı min(qty, açık) ile kırpar ama eski kod TAM order
            // notional'ı nakde yazıyordu; bayat reduce 5 birimlik emir, 1
            // birimlik pozisyona +439 hayali nakit ekliyordu. Marjin kolu
            // zaten closed=min kullanıyordu; spot kolu da aynı sözleşmede.
            let closed = Fraction::max(&Fraction::min(&order.qty, &qty_before), &Fraction::zero());
            let credit_notional = closed.mul(price).map_err(|e| e.to_string())?;
            self.cash = self
                .cash
                .add(&credit_notional.sub(&fee).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
        }
        self.cumulative_fees = self.cumulative_fees.add(&fee).map_err(|e| e.to_string())?;
        // Faz 18 (P1-1): realized_fees — yalnız kapanış (reduce) dolumlarının
        // komisyonu birikir; açık pozisyonun giriş komisyonu buraya girmez.
        if !is_add {
            self.realized_fees = self.realized_fees.add(&fee).map_err(|e| e.to_string())?;
        }
        // M-17: `realized_net` kalıcı alan — net_realized() ile aynı tanımda
        // tutulur; eski davranışta hiç güncellenmiyor, persist hep 0 yazıyordu.
        // Faz 18 (P1-1) tanım: gross − realized_fees − cumulative_funding
        // (bot ömrü komisyonu ve funding'in tamamı değil, yalnız kapanışa
        // düşenler netleştirilir; funding tarafı kümulative kalır — funding
        // pozisyon başınadır ve kapanışla netleşir).
        self.realized_net = self
            .position
            .realized_gross
            .sub(&self.realized_fees)
            .and_then(|x| x.sub(&self.cumulative_funding))
            .unwrap_or_else(|_| self.realized_net.clone());
        self.filled_qty_this_bar = self.filled_qty_this_bar.add(&order.qty).unwrap_or_else(|_| self.filled_qty_this_bar.clone());
        let rec = FillRecord {
            fill_key,
            order_id: order.id.clone(),
            role: order.role.clone(),
            side: order.side,
            kind: order.kind.clone(),
            price: price.clone(),
            qty: order.qty.clone(),
            fee,
            fee_role,
            realism,
            ts,
        };
        self.fills.push(rec.clone());
        Ok(Some(rec))
    }

    /// Submit a decision taken at bar close t (orders eligible from t+1).
    pub fn submit_intent(&mut self, mut order: OrderIntent, decision_ts: i64) -> Result<(), String> {
        let g4 = gate4_no_lookahead(decision_ts + 1, decision_ts);
        self.log_gate(decision_ts, "GATE4", &g4);
        if !matches!(g4, GateVerdict::Pass) {
            return Err("GATE4 rejected".into());
        }
        order.decision_ts = decision_ts + 1;
        self.pending.push(order);
        Ok(())
    }

    /// LIVE reconciliation inject (M16): a resting venue order that vanished
    /// from the venue's open-order book has been filled there — synthesize the
    /// fill at the resting limit price and run the exact same accounting path
    /// (position/cash/fees/ledger) as kernel-generated fills.
    /// EX-05: `fee_role` çağrıcıdan alınır — market dolumu Taker, limit dolumu
    /// Maker olarak doğru ücretle muhasebeleştirilir.
    /// Idempotent per call: the caller must stop tracking the order after a
    /// successful ingest.
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_external_fill(
        &mut self,
        order_id: &str,
        role: &str,
        side: Side,
        kind: OrderKind,
        qty: &Fraction,
        price: &Fraction,
        fee_role: FeeRole,
        realism: FillRealism,
        ts: i64,
    ) -> Result<Option<FillRecord>, String> {
        self.ingest_external_fill_with_fee(order_id, role, side, kind, qty, price, fee_role, realism, ts, None, None)
    }

    /// Faz 3 (C-3/H-4): fill kanıtı venue execution kaydıysa ücret de oradan
    /// gelir. `venue_fee = None` → yerel instrument oranı (eski sözleşme).
    /// Not: ücretin quote cinsinde olduğu varsayımı belgelidir (Bybit linear /
    /// USDT-quote spot); farklı fee-coin (H-4 kalanı) Faz 4'te ayrıştırılacak.
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn ingest_external_fill_with_fee(
        &mut self,
        order_id: &str,
        role: &str,
        side: Side,
        kind: OrderKind,
        qty: &Fraction,
        price: &Fraction,
        fee_role: FeeRole,
        realism: FillRealism,
        ts: i64,
        venue_fee: Option<&Fraction>,
        fill_key_override: Option<&str>,
    ) -> Result<Option<FillRecord>, String> {
        let order = OrderIntent {
            id: order_id.to_string(),
            side,
            kind,
            price: Some(price.clone()),
            qty: qty.clone(),
            decision_ts: ts,
            role: role.to_string(),
            placement_ref_price: Some(price.clone()),
            quote_notional: None,
        };
        self.execute_fill_inner(&order, price, fee_role, realism, ts, venue_fee, fill_key_override)
    }

    pub fn net_realized(&self) -> Result<Fraction, MathError> {
        // Faz 18 (P1-1): realized_net tanımı — gross − realized_fees (yalnız
        // kapanış komisyonu) − funding. execute_fill_inner'daki kalıcı alanla
        // aynı formül.
        self.position
            .realized_gross
            .sub(&self.realized_fees)
            .and_then(|x| x.sub(&self.cumulative_funding))}

    pub fn equity(&self, mark: &Fraction) -> Result<Fraction, MathError> {
        if self.cfg.is_spot {
            // BOT-01: spot equity = nakit + eldeki varlığın mark üzerindeki değeri.
            // Eski formül (cash + unrealized_gross) alış sonrası notional+fee kadar
            // cash'ten düşülüp varlık değeri eklenmediği için equity'yi aşağı raporlardı.
            // Yeni: cash + qty * mark (pozisyonun anlık piyasa değeri).
            self.position.qty.mul(mark).and_then(|pos_value| self.cash.add(&pos_value))
        } else {
            let u = self.position.unrealized_gross(mark)?;
            // margin mode: cash excludes committed IM — add it back
            self.cash.add(&self.im_used).and_then(|x| x.add(&u))}
    }

    /// Free collateral = cash (margin mode) — what G6 checks against new IM.
    pub fn free_margin(&self) -> &Fraction {
        &self.cash
    }

    /// Settle one funding expense on an open margin position (M05 sign bridge).
    /// `expense > 0` = cost (long pays when rate > 0), `expense < 0` = rebate.
    /// Cash moves by -expense; the cumulative funding ledger grows by +expense.
    /// Spot mode: no-op (no funding on spot pairs).
    pub fn apply_funding_expense(&mut self, expense: &Fraction) {
        if self.cfg.is_spot || expense.is_zero() {
            return;
        }
        let cash_delta = expense.neg().unwrap_or_else(|_| Fraction::zero());
        self.cash = self.cash.add(&cash_delta).unwrap_or_else(|_| self.cash.clone());
        self.cumulative_funding = self
            .cumulative_funding
            .add(expense)
            .unwrap_or_else(|_| self.cumulative_funding.clone());
        // Faz 18 (P1-1): funding büyüdükçe realized_net de güncel kalsın
        // (sonraki dolumu beklemeden doğru raporlansın).
        self.realized_net = self
            .position
            .realized_gross
            .sub(&self.realized_fees)
            .and_then(|x| x.sub(&self.cumulative_funding))
            .unwrap_or_else(|_| self.realized_net.clone());
    }

    /// Cancel every resting order (used on liquidation / manual close-all).
    pub fn cancel_pending(&mut self) -> usize {
        let n = self.pending.len();
        self.pending.clear();
        n
    }

    /// Force-close the entire position at `price` (liquidation). Cancels all
    /// resting orders first. Returns the closing FillRecord, if a position existed.
    pub fn force_close(&mut self, price: &Fraction, ts: i64, role: &str) -> Result<Option<FillRecord>, String> {
        self.cancel_pending();
        if self.position.qty.is_zero() {
            return Ok(None);
        }
        let qty = self.position.qty.clone();
        let order = OrderIntent {
            id: format!("LIQ-{}", ts),
            side: if self.position.direction == 1 { Side::Sell } else { Side::Buy },
            kind: OrderKind::Reduce,
            price: Some(price.clone()),
            qty,
            decision_ts: ts,
            role: role.to_string(),
            placement_ref_price: Some(price.clone()),
            quote_notional: None,
        };
        self.execute_fill(&order, price, FeeRole::Taker, FillRealism::MarketableTaker, ts, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use dca_math::Fraction as F;

    fn f(s: &str) -> Fraction {
        F::from_decimal_str(s).unwrap()
    }

    fn add_intent(price: &str, qty: &str, side: Side) -> OrderIntent {
        OrderIntent {
            id: "reserve-test".into(),
            side,
            kind: OrderKind::Add,
            price: Some(f(price)),
            qty: f(qty),
            decision_ts: 1,
            role: "BASE".into(),
            placement_ref_price: Some(f(price)),
            quote_notional: None,
        }
    }

    #[test]
    fn closer_reservation_uses_future_orders_and_futures_margin() {
        let buy_ladder = vec![add_intent("95", "1", Side::Buy), add_intent("85", "1", Side::Buy)];
        let reserved = closer_pending_reserved(&buy_ladder, &f("90"), true, &f("0.01"), &f("5"), false).unwrap();
        // Only the 95 order is closer: initial margin 95/5 + fee 0.95.
        assert_eq!(reserved.to_string(), "399/20");

        let spot_reserved = closer_pending_reserved(&buy_ladder[..1], &f("90"), true, &f("0.01"), &f("5"), true).unwrap();
        assert_eq!(spot_reserved.to_string(), "1919/20");
    }

    fn inst() -> Instrument {
        Instrument {
            symbol: "TESTUSDT".into(),
            maker_fee_rate: f("0.001"),
            taker_fee_rate: f("0.001"),
            ..Default::default()
        }
    }

    fn bar(t: i64, o: &str, h: &str, l: &str, c: &str) -> Bar {
        Bar { t, o: o.into(), h: h.into(), l: l.into(), c: c.into(), v: "1000".into() }
    }

    fn margin_cfg(lev: &str) -> StepperConfig {
        StepperConfig {
            is_spot: false,
            leverage: f(lev),
            ..Default::default()
        }
    }

    fn buy_limit(st: &mut SimulationStepper, id: &str, price: &str, qty: &str, ts: i64) {
        st.submit_intent(
            OrderIntent {
                id: id.into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: Some(f(price)),
                qty: f(qty),
                decision_ts: ts,
                role: "BASE".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            ts,
        )
        .unwrap();
    }

    #[test]
    fn margin_open_deducts_im_only_not_full_notional() {
        // 10x isolated: buying 100 notional costs 10 IM + fee, not 100.
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(1);
        buy_limit(&mut st, "o1", "100", "1", 1000);
        // Bar opens at 100 -> marketable taker fill at 100
        let fills = st.step(&bar(1100, "100", "100.5", "99.5", "100"), 1100, false).unwrap();
        assert_eq!(fills.len(), 1);
        // fee = 100*0.001 = 0.1 (taker); cash = 1000 - 10 - 0.1 = 989.9
        assert_eq!(st.cash.to_half_even_display(4), "989.9000");
        assert_eq!(st.im_used.to_half_even_display(4), "10.0000");
        // equity = cash + im + uPnL(0 at mark=avg) = 999.9
        let eq = st.equity(&f("100")).unwrap();
        assert_eq!(eq.to_half_even_display(4), "999.9000");
    }

    #[test]
    fn margin_gate6_checks_im_not_notional() {
        // 10x: 1 unit @ 100 needs 10 IM; cash 50 is enough (spot would need 100).
        let mut st = SimulationStepper::new(inst(), f("50"), margin_cfg("10"));
        st.set_position_direction(1);
        buy_limit(&mut st, "o1", "99", "1", 1000);
        let fills = st.step(&bar(1100, "100", "100.5", "98", "100"), 1100, false).unwrap();
        assert_eq!(fills.len(), 1, "G6 must pass: IM 9.9 + fee <= 50");
        // And a 1x spot-cash check would have rejected the same order.
    }

    #[test]
    fn invalid_decision_timestamp_halts() {
        // Faz 18 (P1-13): karar zaman damgası <= 0 → kernel durur.
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(1);
        assert!(st.step(&bar(1100, "100", "100.5", "99.5", "100"), 0, false).is_err());
        assert!(st.halted.is_some());
    }

    #[test]
    fn frozen_bar_defers_fills_without_halting() {
        // Faz 18 (P1-15): hacim 0 bar → dolum yok, emir bekler, kernel ayakta.
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(1);
        buy_limit(&mut st, "o1", "99", "1", 1000);
        let frozen = Bar {
            t: 1100,
            o: "100".into(),
            h: "100.5".into(),
            l: "99.5".into(),
            c: "100".into(),
            v: "0".into(),
        };
        let fills = st.step(&frozen, 1100, false).unwrap();
        assert!(fills.is_empty(), "donmuş barda dolum olmamalı");
        assert!(st.halted.is_none(), "donmuş bar kalıcı bozulma değildir");
        // Sonraki likit bar emri doldurur (low 98 < 99 penetre eder).
        let fills = st.step(&bar(1200, "100", "100.5", "98", "100"), 1200, false).unwrap();
        assert_eq!(fills.len(), 1);
    }

    #[test]
    fn market_add_respects_capital_guard() {
        // Faz 18 (P1-8): planlı piyasa Add — bakiye (~100.15 gerekli) yetmezse
        // dolmaz; Wait modunda bekler, Skip modunda atlanır. Halcını asla eksiye
        // çekmez.
        let mut st = SimulationStepper::new(inst(), f("50"), StepperConfig { is_spot: true, ..Default::default() });
        st.set_position_direction(1);
        st.submit_intent(
            OrderIntent {
                id: "m1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: None,
                qty: f("1"),
                decision_ts: 1000,
                role: "BASE".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        let fills = st.step(&bar(1100, "100", "100.5", "99.5", "100"), 1100, false).unwrap();
        assert!(fills.is_empty(), "sermayesi olmayan piyasa Add dolmamalı");
        assert!(st.cash.is_positive() || st.cash.is_zero());
        assert!(!st.halted.is_some());
    }

    #[test]
    fn gap_flush_respects_capital_guard() {
        // Faz 18 (P1-8): gap_batch dolumu — bireysel G6'dan geçen iki seviye
        // birlikte bakiyeyi aşamaz; konsolidasyon moduna göre ya ikisi de
        // bekler ya yalnız ilki dolar. Nakit asla negatif olmaz.
        let mut st = SimulationStepper::new(inst(), f("100"), StepperConfig { is_spot: true, ..Default::default() });
        st.set_position_direction(1);
        for (id, p) in [("s1", "95"), ("s2", "94")] {
            st.submit_intent(
                OrderIntent {
                    id: id.into(),
                    side: Side::Buy,
                    kind: OrderKind::Add,
                    price: Some(f(p)),
                    qty: f("1"),
                    decision_ts: 1000,
                    role: "SAFETY".into(),
                    placement_ref_price: Some(f("100")),
                    quote_notional: None,
                },
                1000,
            )
            .unwrap();
        }
        // Açılış 94'ün altına gap'ledi (90) — her iki seviye OpenGapBetter.
        let fills = st.step(&bar(1100, "90", "91", "88", "89"), 1100, false).unwrap();
        assert!(fills.len() <= 1, "100 bakiye iki ~90'lık dolumu karşılayamaz");
        assert!(!st.cash.is_negative());
    }

    #[test]
    fn margin_close_credits_released_margin_plus_pnl() {
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(1);
        buy_limit(&mut st, "o1", "100", "1", 1000);
        st.step(&bar(1100, "100", "100.5", "99.5", "100"), 1100, false).unwrap();
        // Close full 1 @ 110 (market reduce next bar)
        st.submit_intent(
            OrderIntent {
                id: "tp1".into(),
                side: Side::Sell,
                kind: OrderKind::Reduce,
                price: None,
                qty: f("1"),
                decision_ts: 1100,
                role: "TP".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1100,
        )
        .unwrap();
        let fills = st.step(&bar(1200, "110", "111", "109", "110"), 1200, false).unwrap();
        assert_eq!(fills.len(), 1);
        // Market sell fills at open with 5bps slippage: 110*0.9995 = 109.945
        // released = 100/10 = 10; pnl = 1*(109.945-100) = 9.945; fee = 0.109945
        // cash = 989.9 + 10 + 9.945 - 0.109945 = 1009.735055
        assert_eq!(st.cash.to_half_even_display(4), "1009.7351");
        assert!(st.im_used.is_zero());
        assert!(st.position.qty.is_zero());
        let eq = st.equity(&f("110")).unwrap();
        assert_eq!(eq.to_half_even_display(4), "1009.7351");
    }

    #[test]
    fn realized_net_nets_closed_fees_and_funding_only() {
        // Faz 18 (P1-1): realized_net = gross − realized_fees (yalnız reduce
        // komisyonu) − funding. Giriş (add) komisyonu defteri şişirmez.
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(1);
        buy_limit(&mut st, "o1", "100", "1", 1000);
        st.step(&bar(1100, "100", "100.5", "99.5", "100"), 1100, false).unwrap();
        // Açılış komisyonu 0.1 ödendi ama realized_fees hâlâ sıfır.
        assert!(st.realized_fees.is_zero(), "giriş komisyonu realized_fees'a girmemeli");
        st.apply_funding_expense(&f("0.05"));
        // gross 0, funding 0.05 → net −0.05 (dolum beklemeden güncel).
        assert_eq!(st.realized_net.to_half_even_display(4), "-0.0500");
        // 1 @ 110 market reduce: gross 9.945, reduce fee 0.109945.
        st.submit_intent(
            OrderIntent {
                id: "tp1".into(),
                side: Side::Sell,
                kind: OrderKind::Reduce,
                price: None,
                qty: f("1"),
                decision_ts: 1100,
                role: "TP".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1100,
        )
        .unwrap();
        st.step(&bar(1200, "110", "111", "109", "110"), 1200, false).unwrap();
        // net = 9.945 − 0.109945 − 0.05 = 9.785055
        assert_eq!(st.realized_fees.to_half_even_display(4), "0.1099");
        assert_eq!(st.realized_net.to_half_even_display(4), "9.7851");
    }

    #[test]
    fn force_close_flattens_and_cancels_pending() {
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(1);
        buy_limit(&mut st, "o1", "100", "1", 1000);
        st.step(&bar(1100, "100", "100.5", "99.5", "100"), 1100, false).unwrap();
        buy_limit(&mut st, "o2", "90", "1", 1100); // resting safety
        st.step(&bar(1200, "100", "100.5", "95", "100"), 1200, false).unwrap();
        assert_eq!(st.pending.len(), 1);
        let fill = st.force_close(&f("50"), 1300, "LIQ").unwrap().unwrap();
        assert_eq!(fill.role, "LIQ");
        assert!(st.position.qty.is_zero());
        assert!(st.pending.is_empty());
        // cash = 989.9 + released 10 + pnl (50-100)*1 = -50 ... - fee 0.05 => 949.85
        assert_eq!(st.cash.to_half_even_display(4), "949.8500");
    }

    #[test]
    fn margin_short_equity_stays_wallet_plus_upnl() {
        // Short via sell-to-open at 10x: margin IM reserved, equity = cash+im+u.
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(-1);
        st.submit_intent(
            OrderIntent {
                id: "s1".into(),
                side: Side::Sell,
                kind: OrderKind::Add,
                price: Some(f("100")),
                qty: f("1"),
                decision_ts: 1000,
                role: "BASE".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        st.step(&bar(1100, "100", "100.5", "99.5", "100"), 1100, false).unwrap();
        // cash = 1000 - 10 - 0.1 = 989.9 ; im = 10
        assert_eq!(st.cash.to_half_even_display(4), "989.9000");
        // Price drops to 90: uPnL = 1*(100-90) = +10 -> equity 1009.9
        let eq = st.equity(&f("90")).unwrap();
        assert_eq!(eq.to_half_even_display(4), "1009.9000");
        // Price rises to 110: uPnL = -10 -> equity 989.9
        let eq2 = st.equity(&f("110")).unwrap();
        assert_eq!(eq2.to_half_even_display(4), "989.9000");
    }

    #[test]
    fn funding_expense_moves_cash_and_ledger() {
        // Open long at 10x, then settle funding: rate 0.0001 * notional 100 = 0.01 cost.
        let mut st = SimulationStepper::new(inst(), f("1000"), margin_cfg("10"));
        st.set_position_direction(1);
        buy_limit(&mut st, "o1", "100", "1", 1000);
        st.step(&bar(1100, "100", "100.5", "99.5", "100"), 1100, false).unwrap();
        let cash_before = st.cash.clone();
        st.apply_funding_expense(&f("0.01"));
        // cash = 989.9 - 0.01 = 989.89 ; ledger +0.01
        assert_eq!(st.cash.to_half_even_display(4), "989.8900");
        assert_eq!(st.cumulative_funding.to_half_even_display(6), "0.010000");
        // Rebate (negative expense) flips the sign: cash in, ledger decreases.
        st.apply_funding_expense(&f("-0.004"));
        assert_eq!(st.cash.to_half_even_display(4), "989.8940");
        assert_eq!(st.cumulative_funding.to_half_even_display(6), "0.006000");
        // Equity reflects the funding drain: 989.894 + 10 (IM) + 0 = 999.894
        let eq = st.equity(&f("100")).unwrap();
        assert_eq!(eq.to_half_even_display(4), "999.8940");
        // Spot mode: absolute no-op.
        let mut spot = SimulationStepper::new(inst(), f("1000"), StepperConfig::default());
        let spot_cash = spot.cash.clone();
        spot.apply_funding_expense(&f("5"));
        assert_eq!(spot.cash, spot_cash);
        assert!(spot.cumulative_funding.is_zero());
        let _ = cash_before;
    }

    #[test]
    fn resting_buy_fills_on_penetration_with_fees() {
        let mut st = SimulationStepper::new(inst(), f("10000"), StepperConfig::default());
        st.set_position_direction(1);
        // Decision at t=1000: rest a buy limit at 98 (below open 100)
        st.submit_intent(
            OrderIntent {
                id: "o1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: Some(f("98")),
                qty: f("1"),
                decision_ts: 1000,
                role: "BASE".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        // Bar t=1100: low 97.5 penetrates 98 -> fill at 98 maker
        let fills = st.step(&bar(1100, "100", "100.5", "97.5", "99"), 1100, false).unwrap();
        assert_eq!(fills.len(), 1);
        assert_eq!(fills[0].fee_role, FeeRole::Maker);
        assert_eq!(fills[0].price.to_half_even_display(2), "98.00");
        // fee = 98*0.001 = 0.098
        assert_eq!(fills[0].fee.to_half_even_display(4), "0.0980");
        // cash = 10000 - 98 - 0.098 = 9901.902
        assert_eq!(st.cash.to_half_even_display(4), "9901.9020");
        assert_eq!(st.position.avg_price.unwrap().to_string(), "98");
    }

    #[test]
    fn no_penetration_defers() {
        let mut st = SimulationStepper::new(inst(), f("10000"), StepperConfig::default());
        st.submit_intent(
            OrderIntent {
                id: "o1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: Some(f("90")),
                qty: f("1"),
                decision_ts: 1000,
                role: "BASE".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        let fills = st.step(&bar(1100, "100", "100.5", "97.5", "99"), 1100, false).unwrap();
        assert!(fills.is_empty());
        assert_eq!(st.pending.len(), 1);
    }

    #[test]
    fn market_order_slippage_applied() {
        let mut st = SimulationStepper::new(inst(), f("10000"), StepperConfig::default());
        st.set_position_direction(1);
        st.submit_intent(
            OrderIntent {
                id: "m1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: None,
                qty: f("1"),
                decision_ts: 1000,
                role: "BASE".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        let fills = st.step(&bar(1100, "100", "101", "99", "100.5"), 1100, false).unwrap();
        assert_eq!(fills.len(), 1);
        // 100 * (1 + 5/10000) = 100.05
        assert_eq!(fills[0].price.to_half_even_display(4), "100.0500");
        assert_eq!(fills[0].fee_role, FeeRole::Taker);
    }

    #[test]
    fn gate4_lookahead_enforced() {
        let mut st = SimulationStepper::new(inst(), f("10000"), StepperConfig::default());
        // decision_ts=1000, order eligible from 1001; bar t=1000 should NOT fill market order
        st.submit_intent(
            OrderIntent {
                id: "m1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: None,
                qty: f("1"),
                decision_ts: 1000,
                role: "BASE".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        // pending order decision_ts=1001 > bar.t=1000 -> deferred
        let fills = st.step(&bar(1000, "100", "101", "99", "100.5"), 1000, false).unwrap();
        assert!(fills.is_empty());
        assert_eq!(st.pending.len(), 1);
    }

    #[test]
    fn sell_realizes_and_cash_increases() {
        let mut st = SimulationStepper::new(inst(), f("0"), StepperConfig::default());
        st.set_position_direction(1);
        // Seed position manually: buy 1 @100
        st.position.apply_fill("seed", &f("100"), &f("1"), true).unwrap();
        st.submit_intent(
            OrderIntent {
                id: "tp1".into(),
                side: Side::Sell,
                kind: OrderKind::Reduce,
                price: Some(f("110")),
                qty: f("1"),
                decision_ts: 1000,
                role: "TP".into(),
                placement_ref_price: Some(f("108")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        let fills = st.step(&bar(1100, "108", "111", "107", "110"), 1100, false).unwrap();
        assert_eq!(fills.len(), 1);
        // realized = 110-100 = 10
        assert_eq!(st.position.realized_gross.to_string(), "10");
        // cash = 110 - 0.11 = 109.89
        assert_eq!(st.cash.to_half_even_display(2), "109.89");
    }

    #[test]
    fn gate6_insufficient_cash_skips_add() {
        let mut cfg = StepperConfig::default();
        cfg.capital_shortfall_mode = CapitalShortfallMode::Skip;
        let mut st = SimulationStepper::new(inst(), f("50"), cfg);
        st.set_position_direction(1);
        st.submit_intent(
            OrderIntent {
                id: "o1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: Some(f("98")),
                qty: f("1"),
                decision_ts: 1000,
                role: "SAFETY".into(),
                placement_ref_price: Some(f("100")),
                quote_notional: None,
            },
            1000,
        )
        .unwrap();
        let fills = st.step(&bar(1100, "100", "100.5", "97.5", "99"), 1100, false).unwrap();
        assert!(fills.is_empty());
        assert!(st.gate_logs.iter().any(|g| g.gate == "GATE6" && g.verdict == "REJECT"));
    }

    #[test]
    fn ohlc_violation_halts_kernel() {
        let mut st = SimulationStepper::new(inst(), f("10000"), StepperConfig::default());
        let err = st.step(&bar(1000, "100", "90", "95", "97"), 1000, false);
        assert!(err.is_err());
        assert!(st.halted.is_some());
    }

    #[test]
    fn ingest_external_fill_accounts_like_kernel_fill() {
        // LIVE reconciliation path: venue filled a resting order — synthesize a
        // maker fill at the resting limit price through the same accounting.
        let mut st = SimulationStepper::new(inst(), f("1000"), StepperConfig::default());
        st.set_position_direction(1);
        let rec = st
            .ingest_external_fill("o1", "BASE", Side::Buy, OrderKind::Add, &f("1"), &f("99"), FeeRole::Maker, FillRealism::RestingTouch, 1200)
            .unwrap()
            .expect("fill synthesized");
        assert_eq!(rec.order_id, "o1");
        assert_eq!(rec.role, "BASE");
        assert_eq!(rec.realism, FillRealism::RestingTouch);
        assert_eq!(rec.fee_role, FeeRole::Maker);
        assert_eq!(st.position.qty.to_string(), "1");
        assert_eq!(st.position.avg_price.as_ref().unwrap().to_string(), "99");
        // spot cash: 1000 - 99 - fee(99*0.001=0.099) = 900.901
        assert_eq!(st.cash.to_half_even_display(3), "900.901");
        // Reduce path: closing the position credits cash with realized pnl.
        let rec2 = st
            .ingest_external_fill("o2", "TP", Side::Sell, OrderKind::Reduce, &f("1"), &f("110"), FeeRole::Maker, FillRealism::RestingTouch, 1300)
            .unwrap()
            .expect("close fill");
        assert_eq!(rec2.realism, FillRealism::RestingTouch);
        assert_eq!(rec2.fee_role, FeeRole::Maker);
        assert!(st.position.qty.is_zero());
        assert_eq!(st.position.realized_gross.to_string(), "11");
    }

    /// Faz 3 (H-4): venue execution kaydından gelen GERÇEK ücret muhasebeye girer;
    /// yerel instrument oranı değil.
    #[test]
    fn venue_fee_overrides_local_rate() {
        let mut st = SimulationStepper::new(inst(), f("1000"), StepperConfig::default());
        st.set_position_direction(1);
        // instrument oranı 0.001 → yerel fee 0.099 olurdu; venue 0.02 bildiriyor.
        let venue = f("0.02");
        let rec = st
            .ingest_external_fill_with_fee("v1", "BASE", Side::Buy, OrderKind::Add, &f("1"), &f("99"), FeeRole::Taker, FillRealism::MarketableTaker, 1200, Some(&venue), None)
            .unwrap()
            .expect("fill");
        assert_eq!(rec.fee.to_string(), "1/50", "venue fee (0.02) kullanılmalı");
        assert_eq!(st.cumulative_fees.to_string(), "1/50");
        // cash = 1000 - 99 - 0.02 = 900.98
        assert_eq!(st.cash.to_half_even_display(3), "900.980");
    }

    /// Faz 3 (H-4): venue sıfır ücret bildirebilir (voucher) — G2 reddetmez,
    /// muhasebe ücretsiz dolumla devam eder.
    #[test]
    fn venue_zero_fee_is_verified_and_allowed() {
        let mut st = SimulationStepper::new(inst(), f("1000"), StepperConfig::default());
        st.set_position_direction(1);
        let zero = f("0");
        let rec = st
            .ingest_external_fill_with_fee("v2", "BASE", Side::Buy, OrderKind::Add, &f("1"), &f("50"), FeeRole::Maker, FillRealism::RestingTouch, 1300, Some(&zero), None)
            .unwrap()
            .expect("zero-fee fill must pass (venue-verified)");
        assert!(rec.fee.is_zero());
        assert_eq!(st.cash.to_half_even_display(2), "950.00");
    }

    /// Faz 3 (C-3): aynı (order, exec) fill'i ikinci kez işlenmez — execution
    /// tabanlı idempotency çekirdekte fill_key dedup'ıyla sağlanır.
    #[test]
    fn duplicate_exec_fill_is_idempotent() {
        let mut st = SimulationStepper::new(inst(), f("1000"), StepperConfig::default());
        st.set_position_direction(1);
        let venue = f("0.01");
        let one = st.ingest_external_fill_with_fee("v3", "SAFETY", Side::Buy, OrderKind::Add, &f("1"), &f("10"), FeeRole::Maker, FillRealism::RestingTouch, 1400, Some(&venue), Some("v3-1400")).unwrap();
        assert!(one.is_some());
        // Faz 23: execId anahtarı artık kernel'e taşınıyor — aynı anahtarla
        // gelen ikinci dolum idempotent yoksayılır (test bu sözleşmeyi tutar).
        let two = st.ingest_external_fill_with_fee("v3", "SAFETY", Side::Buy, OrderKind::Add, &f("1"), &f("10"), FeeRole::Maker, FillRealism::RestingTouch, 1400, Some(&venue), Some("v3-1400")).unwrap();
        assert!(two.is_none(), "duplicate fill_key (order+ts) yoksayılmalı");
        assert_eq!(st.position.qty.to_string(), "1");
        assert_eq!(st.cumulative_fees.to_string(), "1/100");
    }

    /* ---------------- Task 16 — Risk Motoru davranış testleri ---------------- */

    fn submit_limit(st: &mut SimulationStepper, id: &str, price: &str, qty: &str, ts: i64) {
        st.submit_intent(
            OrderIntent {
                id: id.into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: Some(F::from_decimal_str(price).unwrap()),
                qty: F::from_decimal_str(qty).unwrap(),
                decision_ts: ts,
                role: "SAFETY".into(),
                placement_ref_price: Some(F::from_decimal_str("100").unwrap()),
                quote_notional: None,
            },
            ts,
        )
        .unwrap();
    }

    fn bar_at(t: i64, o: &str, h: &str, l: &str, c: &str) -> Bar {
        Bar { t, o: o.into(), h: h.into(), l: l.into(), c: c.into(), v: "1000".into() }
    }

    /// REALISTIC model: teğet dokunuş (low == limit) DOLMAZ; OPTIMISTIC dolar.
    #[test]
    fn fill_model_realistic_vs_optimistic() {
        // low == limit tam teğet.
        let b = bar_at(1000, "100", "100.5", "98", "99");
        // REALISTIC: dolmaz (penetrasyon yok).
        let mut st = SimulationStepper::new(inst(), f("1000"), StepperConfig::default());
        submit_limit(&mut st, "o1", "98", "1", 900);
        let fills = st.step(&b, 1000, false).unwrap();
        assert!(fills.is_empty(), "REALISTIC: teğet dokunuş dolmamalı");
        // OPTIMISTIC: dolar.
        let mut st = SimulationStepper::new(
            inst(),
            f("1000"),
            StepperConfig { policy: UserRiskPolicy { fill_model: FillModel::Optimistic, ..Default::default() }, ..Default::default() },
        );
        submit_limit(&mut st, "o1", "98", "1", 900);
        let fills = st.step(&b, 1000, false).unwrap();
        assert_eq!(fills.len(), 1, "OPTIMISTIC: teğet dokunuş dolmalı");
    }

    /// Gate 7 advisory: seans kapalıyken ENFORCE reddeder, PERMISSIVE uyarıyla geçirir.
    #[test]
    fn gate7_session_enforce_vs_permissive() {
        // 2026-01-17 Cumartesi 16:00 UTC → UsEquity kapalı.
        let t_sat = 1768656000000_i64; // 2026-01-17 16:00 UTC
        let b = bar_at(t_sat, "100", "101", "99", "100");
        // ENFORCE: market add düşürülür.
        let mut st = SimulationStepper::new(
            inst(),
            f("1000"),
            StepperConfig {
                session_type: dca_math::sessions::SessionType::UsEquity,
                policy: UserRiskPolicy { session_type: RiskSessionType::UsEquity, ..Default::default() },
                ..Default::default()
            },
        );
        st.submit_intent(
            OrderIntent {
                id: "m1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: None,
                qty: f("1"),
                decision_ts: t_sat - 1,
                role: "BASE".into(),
                placement_ref_price: None,
            quote_notional: None,
            },
            t_sat - 1,
        )
        .unwrap();
        let fills = st.step(&b, t_sat, false).unwrap();
        assert!(fills.is_empty(), "ENFORCE: seans dışı market add düşürülmeli");
        // PERMISSIVE: uyarı + dolum.
        let mut st = SimulationStepper::new(
            inst(),
            f("1000"),
            StepperConfig {
                session_type: dca_math::sessions::SessionType::UsEquity,
                policy: UserRiskPolicy {
                    session_type: RiskSessionType::UsEquity,
                    session_guard: GuardAction::PermissiveWithWarning,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        st.submit_intent(
            OrderIntent {
                id: "m1".into(),
                side: Side::Buy,
                kind: OrderKind::Add,
                price: None,
                qty: f("1"),
                decision_ts: t_sat - 1,
                role: "BASE".into(),
                placement_ref_price: None,
            quote_notional: None,
            },
            t_sat - 1,
        )
        .unwrap();
        let fills = st.step(&b, t_sat, false).unwrap();
        assert_eq!(fills.len(), 1, "PERMISSIVE: seans dışı alım kullanıcı izniyle iletilir");
        let advisories = st.drain_new_gate_logs();
        assert!(
            advisories.iter().any(|g| g.verdict.contains("SESSION_CLOSED")),
            "SESSION_CLOSED advisory loglanmalı: {advisories:?}"
        );
    }

    /// Gate 8b konsolidasyon: aynı barda 2 gap SAFETY dolumu → Enforce'ta TEK
    /// ".m2" dolum; Permissive'te iki ayrı dolum + GAP_NO_CONSOLIDATION uyarısı.
    #[test]
    fn gate8_gap_consolidation_modes() {
        // Bar açılışı her iki limitin altında → her ikisi de OpenGapBetter.
        let b = bar_at(2000, "95", "96", "94", "95.5");
        let run = |policy: UserRiskPolicy| -> (Vec<FillRecord>, Vec<GateLog>) {
            let mut st = SimulationStepper::new(inst(), f("100000"), StepperConfig { policy, ..Default::default() });
            submit_limit(&mut st, "s1", "99", "1", 1900);
            submit_limit(&mut st, "s2", "97", "1", 1900);
            let fills = st.step(&b, 2000, false).unwrap();
            let logs = st.drain_new_gate_logs();
            (fills, logs)
        };
        // ENFORCE (default): tek konsolide dolum.
        let (fills, logs) = run(UserRiskPolicy::default());
        assert_eq!(fills.len(), 1, "konsolidasyon: TEK sentetik dolum");
        assert!(fills[0].order_id.ends_with(".m2"), "merged id eki: {}", fills[0].order_id);
        assert_eq!(fills[0].qty.to_string(), "2");
        assert_eq!(fills[0].price.to_string(), "95", "açılış fiyatından");
        assert!(logs.iter().any(|g| g.verdict.contains("GAP_CONSOLIDATED")));
        // PERMISSIVE: iki ayrı dolum (ayrı komisyon).
        let (fills, logs) = run(UserRiskPolicy { gap_consolidation: GuardAction::PermissiveWithWarning, ..Default::default() });
        assert_eq!(fills.len(), 2, "konsolidasyon kapalı: her seviye ayrı");
        assert!(fills.iter().all(|f2| !f2.order_id.contains(".m")));
        assert!(logs.iter().any(|g| g.verdict.contains("GAP_NO_CONSOLIDATION")));
    }
}
