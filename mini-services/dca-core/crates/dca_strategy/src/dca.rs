//! DCA Long/Short engine — safety ladder (single-active-order invariant), M05
//! net TP, M10 trailing/break-even, SL, M20-validated configs, cooldown.

use crate::{order_id, Action, BarContext, BotConfig, EngineSnapshot, StrategyKind};
use dca_kernel::{FillRecord, OrderKind};
use dca_math::fraction::Fraction;
use dca_math::m02::{build_dca_plan, Direction, SizingMode, DcaLevel};
use dca_math::m03_m07::{calculate_net_take_profit, gross_take_profit_price};
use dca_math::m04::PositionState;
use dca_math::Instrument;
use serde::{Deserialize, Serialize};

/// M-18 — qty→0 abort sonrası yeniden deneme aralığı (bar sayısı).
const ABORT_BACKOFF_BARS: usize = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DcaPhase {
    Idle,
    BasePending,
    Open,
    Closing,
    Cooldown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrailingState {
    Off,
    Idle,
    Armed,
    Triggered,
}

/// Faz 19 (P1-4 kalıcı): dinlenen TP reduce-only limit emrinin motor tarafı
/// kaydı — replace kararı (fiyat/adet sapması) ve kapanışta iptal bunun
/// üzerinden yürür.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RestingTp {
    pub order_id: String,
    pub price: Fraction,
    pub qty: Fraction,
}

/// Faz 3 (C-4): kalıcı runtime snapshot için serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DcaEngine {
    pub bot_id: String,
    pub cfg: BotConfig,
    pub inst: Instrument,
    pub direction: Direction,
    pub phase: DcaPhase,
    pub seq: u64,
    pub deal_seq: u64,
    pub ladder: Vec<DcaLevel>,
    pub filled_safety: usize,
    /// Şu anda borsada bekleyen (resting) güvenlik emri sayısı —
    /// Task 16 (spec 2.8): aktif güvenlik emri sayısı = policy.active_safety_orders.
    pub resting_safety: usize,
    pub mirror: PositionState,
    pub tp_price: Option<Fraction>,
    /// Faz 19 (P1-4 kalıcı): dinlenen TP reduce-only LIMIT emrinin motor
    /// tarafı kaydı — replace kararı (fiyat/adet sapması) ve kapanışta iptal
    /// bunun üzerinden yürür. Eski snapshot'larda alan yok → default None.
    #[serde(default)]
    pub resting_tp: Option<RestingTp>,
    pub sl_price: Option<Fraction>,
    pub trailing: TrailingState,
    pub trailing_peak: Option<Fraction>,
    pub funding: Fraction,
    /// Faz 17 (P1-5): DEAL-bazlı funding — TP hesabı bunu kullanır; `funding`
    /// bot ömrü kümülatifi olarak raporlamada kalır. Deal açılışında sıfırlanır.
    pub funding_deal: Fraction,
    /// Faz 17 (P1-16): yerleştirme imleci — `filled_safety` yalnız dolum sayacı;
    /// top-up artık `ladder[next_level_to_place]`'dan devam eder (çift yerleştirme yok).
    pub next_level_to_place: usize,
    /// Faz 17 (P0-6): Closing fazında beklenen bar sayısı — 3 barı aşarsa
    /// kapanış emri yeniden gönderilir (pozisyon korumasız kalmaz).
    pub closing_bars_waited: usize,
    pub cooldown_remaining: usize,
    pub close_orders_out: usize,
    pub closes_seen: usize,
    pub closed_qty_accum: Fraction,
    pub deals_closed: usize,
    pub wins: usize,
    pub losses: usize,
    /// Faz 18 (P1-2): DEAL-bazlı komisyon toplamı — win/loss kararı net
    /// bilanço üzerinden verilir (realized_gross − deal_fees − funding_deal).
    /// Deal açılışında sıfırlanır.
    #[serde(default)]
    pub deal_fees: Fraction,
    /// M-17: bot ömrü boyunca gerçekleşen brüt PnL. Mirror deal-başı WAC
    /// defteridir ve her yeni deal'da sıfırlanır; kümülatif realizasyon
    /// burada devredilir (deals_closed ile aynı pencere).
    #[serde(default)]
    pub cum_realized_gross: Fraction,
    pub rsi_window: Vec<Fraction>,
    pub manual_close_requested: bool,
    /// True once the start condition (entry trigger) is satisfied.
    pub entry_trigger_met: bool,
    /// Last composite signal-tree verdict (None = no tree configured).
    pub last_signal_met: Option<bool>,
    /// Set by the TradingView webhook path: next gate evaluation bypasses all
    /// start conditions (3Commas external signal parity).
    pub force_start_pending: bool,
    /// Task 22 — açılamayan deal karar sayaçları. Motorun o barda GERÇEKTEN
    /// verdiği engel kararları (backtest no-deal nedenleri); canlı davranışı
    /// değiştirmez, yalnızca sayar.
    pub blocked_trigger_bars: u64,
    pub blocked_signal_bars: u64,
    pub blocked_regime_bars: u64,
    pub blocked_breaker_bars: u64,
    pub blocked_cooldown_bars: u64,
    /// M-18: base emir qty adım boyutuna yuvarlanarak sıfırlandığında (notional
    /// min_qty altı) artan honest sayaç — her bar bir kez, backtest no-deal raporu.
    #[serde(default)]
    pub blocked_size_bars: u64,
    pub invalid_config_bars: u64,
}

impl DcaEngine {
    pub fn new(bot_id: &str, cfg: BotConfig, inst: Instrument) -> Result<Self, String> {
        let direction = match cfg.kind {
            StrategyKind::DcaLong | StrategyKind::FuturesDca => Direction::Long,
            StrategyKind::DcaShort => Direction::Short,
            _ => return Err("DcaEngine requires DCA_LONG/DCA_SHORT/FUTURES_DCA".into()),
        };
        let deviation = Fraction::from_decimal_str(&cfg.deviation_pct).map_err(|e| e.to_string())?;
        let step_scale = Fraction::from_decimal_str(&cfg.step_scale).map_err(|e| e.to_string())?;
        let volume_scale = Fraction::from_decimal_str(&cfg.volume_scale).map_err(|e| e.to_string())?;
        if deviation.is_zero() || deviation.is_negative() {
            return Err("deviation must be > 0".into());
        }
        cfg.entry_trigger.validate()?;
        if let Some(sc) = &cfg.entry_signal {
            dca_signals::validate_signal_node(&sc.root)?;
        }
        let _ = (step_scale, volume_scale);
        let entry_trigger_met = matches!(cfg.entry_trigger, crate::EntryTrigger::Instant);
        Ok(Self {
            bot_id: bot_id.to_string(),
            cfg,
            inst,
            direction,
            phase: DcaPhase::Idle,
            seq: 0,
            deal_seq: 0,
            ladder: Vec::new(),
            filled_safety: 0,
            resting_safety: 0,
            mirror: PositionState::new(direction.sign() as i8),
            tp_price: None,
            resting_tp: None,
            sl_price: None,
            trailing: TrailingState::Off,
            trailing_peak: None,
            funding: Fraction::zero(),
            funding_deal: Fraction::zero(),
            next_level_to_place: 0,
            closing_bars_waited: 0,
            cooldown_remaining: 0,
            close_orders_out: 0,
            closes_seen: 0,
            closed_qty_accum: Fraction::zero(),
            deals_closed: 0,
            wins: 0,
            losses: 0,
            deal_fees: Fraction::zero(),
            cum_realized_gross: Fraction::zero(),
            rsi_window: Vec::new(),
            manual_close_requested: false,
            entry_trigger_met,
            last_signal_met: None,
            force_start_pending: false,
            blocked_trigger_bars: 0,
            blocked_signal_bars: 0,
            blocked_regime_bars: 0,
            blocked_breaker_bars: 0,
            blocked_cooldown_bars: 0,
            blocked_size_bars: 0,
            invalid_config_bars: 0,
        })
    }

    /// TradingView webhook path: request a forced deal start (bypasses gates).
    pub fn request_force_start(&mut self) {
        self.force_start_pending = true;
    }

    pub fn next_seq(&mut self) -> u64 {
        self.seq += 1;
        self.seq
    }

    fn next_seq_id(&mut self) -> String {
        self.seq += 1;
        format!("{}-{}", self.bot_id, self.seq)
    }

    fn close_str(&self, x: &Fraction, d: usize) -> String {
        x.to_half_even_display(d)
    }

    /// Compute the TP target (gross or exact-net M05) for the current position.
    fn compute_tp(&self, qty: &Fraction, avg: &Fraction) -> Result<Option<Fraction>, String> {
        if qty.is_zero() {
            return Ok(None);
        }
        let target_pct = Fraction::from_decimal_str(&self.cfg.target_profit_pct).map_err(|e| e.to_string())?;
        if self.cfg.use_net_tp {
            let target_quote = qty.mul(avg).map_err(|e| e.to_string())?.mul(&target_pct).map_err(|e| e.to_string())?;
            // Faz 19 (P1-4 kalıcı): TP artık dinlenen reduce-only LIMIT olarak
            // icra edilir — çıkış MAKER'dır; net TP hesabı da maker ücretle
            // tutarlıdır. (Faz 18'deki taker düzeltmesi piyasa-çıkış dönemine
            // aitti; resting limit mimarisinde gerçek çıkış maliyeti maker.)
            let exit_rate = self.inst.maker_fee_rate.clone();
            let p = calculate_net_take_profit(
                self.direction.sign() as i8,
                qty,
                avg,
                &self.funding_deal,
                &exit_rate,
                &target_quote,
                &self.inst.tick_size,
            )
            .map_err(|e| e.to_string())?;
            Ok(Some(p))
        } else {
            let p = gross_take_profit_price(self.direction.sign() as i8, avg, &target_pct, &self.inst.tick_size)
                .map_err(|e| e.to_string())?;
            Ok(Some(p))
        }
    }

    /// Begin a new deal: market base order at next bar (instant) or trigger-based.
    pub fn try_open_deal(&mut self, ctx: &BarContext) -> Vec<Action> {
        let mut acts = Vec::new();
        if self.phase != DcaPhase::Idle && self.phase != DcaPhase::Cooldown {
            return acts;
        }
        // Task 16 — günlük DD devre kesici: bugün yeni deal açılışı blokludur
        // (mevcut pozisyonun TP/SL yönetimi ETKİLENMEZ — bu yalnız açılış kapısı).
        if ctx.deal_open_blocked {
            self.blocked_breaker_bars += 1;
            return acts;
        }
        if self.phase == DcaPhase::Cooldown {
            if self.cooldown_remaining > 0 {
                self.cooldown_remaining -= 1;
                self.blocked_cooldown_bars += 1;
                return acts;
            }
            self.phase = DcaPhase::Idle;
        }
        // Task 16 (spec 2.7) — Rejim filtresi: CRASH / HIGH_VOLATILITY'de yeni deal.
        // Enforce → bloklama; PermissiveWithWarning → uyarı logla ve devam et
        // (düşen bıçağı tutma riski kullanıcıya bildirildi).
        if let Some(regime) = ctx.regime {
            if regime.is_risky() {
                if self.cfg.risk_policy.regime_filter.is_permissive() {
                    acts.push(Action::Log {
                        level: "WARN",
                        message: format!(
                            "REGIME_{}: piyasa {} rejiminde — kullanıcı izniyle alım sürüyor (düşen bıçağı tutma ve uzun süre kilitlenme riski üstlenildi)",
                            regime.label(),
                            regime.label()
                        ),
                    });
                } else {
                    acts.push(Action::Log {
                        level: "INFO",
                        message: format!(
                            "REGIME_{}: piyasa {} rejiminde — yeni deal açılışı koruma modunda durduruldu (rejim filtresi ZORUNLU; sihirbazdan UYARI+İZİN'e alabilirsiniz)",
                            regime.label(),
                            regime.label()
                        ),
                    });
                    self.blocked_regime_bars += 1;
                    return acts;
                }
            }
        }
        // Webhook force-start (TradingView external signal): bypasses all gates
        // exactly once, when the engine is actually able to open a deal.
        let forced = std::mem::take(&mut self.force_start_pending);
        // Start condition (entry trigger) evaluation — gates every new deal.
        let rsi_needed = matches!(self.cfg.entry_trigger, crate::EntryTrigger::RsiBelow(_));
        let rsi_opt = if rsi_needed { crate::rsi_exact(&self.rsi_window, 14) } else { None };
        let trigger_ok = match crate::evaluate_entry_trigger(&self.cfg.entry_trigger, &ctx.close, ctx.bar.t, rsi_opt.as_ref()) {
            Ok(v) => v,
            Err(e) => {
                // Ulaşılamaz savunma: tüm Err dalları EntryTrigger::validate() ile
                // construction anında yakalanır (DcaEngine::new + API kapıları).
                acts.push(Action::Log { level: "ERROR", message: format!("entry trigger invalid: {e}") });
                return acts;
            }
        };
        // Composite signal tree (fail-closed): Some(false) blocks unless forced.
        let signal_block = matches!(ctx.signal_ok, Some(false));
        self.last_signal_met = ctx.signal_ok;
        if !forced && (!trigger_ok || signal_block) {
            if !trigger_ok {
                self.entry_trigger_met = false;
                self.blocked_trigger_bars += 1;
            }
            if signal_block {
                // Not: sinyal ağacı AND Pine birleşik hükmüdür (ctx.signal_ok) —
                // kaynak ayrımı burada bilinmez, o yüzden tarafsız ifade.
                self.blocked_signal_bars += 1;
            }
            if signal_block {
                acts.push(Action::Log { level: "INFO", message: "signal condition not met — waiting (fail-closed)".into() });
            }
            return acts;
        }
        if forced {
            acts.push(Action::Log { level: "INFO", message: "webhook START_DEAL — opening deal (conditions bypassed)".into() });
        } else if !self.entry_trigger_met {
            acts.push(Action::Log {
                level: "INFO",
                message: format!("start condition met ({}) — opening deal", self.cfg.entry_trigger.describe()),
            });
        }
        self.entry_trigger_met = true;

        self.deal_seq += 1;
        let deal_id = format!("{}-deal-{}", self.bot_id, self.deal_seq);
        // Build the ladder with a provisional anchor = current close (final VWAP locks on fill)
        let anchor = ctx.close.clone();
        // Config numeric fields MUST parse — create_bot validates them, but a
        // runtime edited/stale config would otherwise silently trade qty=1 with
        // deviation=2%. Honest fail-closed: refuse the deal with a clear log.
        let parsed = (
            Fraction::from_decimal_str(&self.cfg.base_order_size),
            Fraction::from_decimal_str(&self.cfg.safety_order_size),
            Fraction::from_decimal_str(&self.cfg.deviation_pct),
            Fraction::from_decimal_str(&self.cfg.step_scale),
            Fraction::from_decimal_str(&self.cfg.volume_scale),
        );
        let (base_qty, safety_size, deviation, step_scale, volume_scale) = match parsed {
            (Ok(a), Ok(b), Ok(c), Ok(d), Ok(e)) => (a, b, c, d, e),
            _ => {
                acts.push(Action::Log {
                    level: "ERROR",
                    message: format!(
                        "deal açılamadı: config sayısal alanları geçersiz (base='{}' safety='{}' deviation='{}' step='{}' volume='{}') — bot düzeltilmeden deal açılmaz",
                        self.cfg.base_order_size, self.cfg.safety_order_size, self.cfg.deviation_pct, self.cfg.step_scale, self.cfg.volume_scale
                    ),
                });
                self.invalid_config_bars += 1;
                self.deal_seq -= 1;
                return acts;
            }
        };
        // SL validity gate: an unparseable/out-of-range stop_loss_pct must never
        // silently open a deal WITHOUT its stop (create_bot validates; this is
        // the runtime backstop for restored/legacy snapshots).
        if let Some(sl_pct) = &self.cfg.stop_loss_pct {
            let sl_ok = Fraction::from_decimal_str(sl_pct)
                .map(|v| v > Fraction::zero() && v <= Fraction::one())
                .unwrap_or(false);
            if !sl_ok {
                acts.push(Action::Log {
                    level: "ERROR",
                    message: format!("deal açılamadı: config.stop_loss_pct '{sl_pct}' 0 < SL ≤ 1 aralığında değil — SL'siz deal açılmaz"),
                });
                self.invalid_config_bars += 1;
                self.deal_seq -= 1;
                return acts;
            }
        }
        let ladder = build_dca_plan(
            &anchor,
            &base_qty,
            &safety_size,
            &deviation,
            &step_scale,
            &volume_scale,
            self.cfg.max_safety_orders,
            self.direction,
            &self.inst.tick_size,
            &self.inst.step_size,
            self.cfg.sizing_mode,
        );
        match ladder {
            Ok(l) => self.ladder = l,
            Err(e) => {
                acts.push(Action::Log { level: "ERROR", message: format!("ladder rejected: {e}") });
                return acts;
            }
        }
        self.filled_safety = 0;
        self.resting_safety = 0;
        self.mirror = PositionState::new(self.direction.sign() as i8);
        // M-17: funding kümülatiftir (snapshot `cumulative_funding` sözleşmesi);
        // deal açılışında sıfırlanmaz — yalnızca deal-bazlı WAC mirror sıfırlanır.
        self.trailing = if self.cfg.trailing_enabled { TrailingState::Idle } else { TrailingState::Off };
        self.trailing_peak = None;
        self.manual_close_requested = false;
        let qty = match self.cfg.sizing_mode {
            SizingMode::BaseQty => base_qty,
            SizingMode::QuoteNotional => dca_math::m02::qty_down(&base_qty.div(&anchor).unwrap_or_else(|_| Fraction::zero()), &self.inst.step_size)
                .unwrap_or_else(|_| Fraction::zero()),
        };
        if qty.is_zero() {
            // M-18: base notional adım boyutunun altında (qty→0) — her tick yeniden
            // denemek CRITICAL seli + deal_seq yakımı üretir. Dürüst davranış:
            // sayaç, tek CRITICAL (sonra %100'de bir hatırlatma) ve 30 bar geri
            // çekilme; fiyat ivmesi tek başına geçerli kılabileceği için bot
            // kapanmaz, aralıklı yeniden dener.
            self.blocked_size_bars += 1;
            self.deal_seq -= 1;
            if self.blocked_size_bars == 1 || self.blocked_size_bars % 100 == 0 {
                acts.push(Action::Log {
                    level: if self.blocked_size_bars == 1 { "CRITICAL" } else { "WARN" },
                    message: format!(
                        "deal açılamadı: base notional adım boyutunun altında (qty→0) — {n}. engel; {backoff} bar sonra yeniden denenecek (base_order_size enstrüman min_qty'sını karşılamıyor)",
                        n = self.blocked_size_bars,
                        backoff = ABORT_BACKOFF_BARS,
                    ),
                });
            }
            self.cooldown_remaining = self.cfg.cooldown_bars.max(ABORT_BACKOFF_BARS);
            self.phase = DcaPhase::Cooldown;
            return acts;
        }
        let base_seq = self.next_seq();
        acts.push(Action::PlaceMarket {
            id: order_id(&self.bot_id, "BASE", base_seq),
            side: if self.direction == Direction::Long { dca_math::Side::Buy } else { dca_math::Side::Sell },
            qty,
            role: "BASE",
        });
        acts.push(Action::Log { level: "INFO", message: format!("deal {deal_id} base order dispatched") });
        self.phase = DcaPhase::BasePending;
        acts
    }

    /// Called by runner after the base order fills; locks anchor VWAP and places
    /// ilk N güvenlik emri (Task 16 spec 2.8 — aktif güvenlik emri sayısı) + TP/SL.
    pub fn on_base_filled(&mut self, fill_price: &Fraction) -> Vec<Action> {
        let mut acts = Vec::new();
        self.phase = DcaPhase::Open;
        // Anchor = final VWAP of base order (M02 anchor mode)
        // (mirror already updated by on_fill; avg == anchor)
        self.rebuild_ladder_from_anchor(fill_price);
        // Task 16 — M20 advisory (runtime gerçekliği): gerçek anchor + gerçek
        // enstrümanla kurulmuş merdiven (self.ladder) üzerinden tarama; SL
        // merdivenden darsa kullanıcıya uyarı basılır (engellenmez).
        if let Some(sl_pct) = self.cfg.stop_loss_pct.as_deref().filter(|s| !s.is_empty()) {
            if let (Ok(sl), Some(avg), Some(anchor)) = (
                Fraction::from_decimal_str(sl_pct),
                self.mirror.avg_price.as_ref(),
                Some(fill_price),
            ) {
                if let Ok((viable, first, _)) = dca_math::m15_m20::m20_conflict_scan(
                    self.direction,
                    &sl,
                    &self.mirror.qty.clone(),
                    anchor,
                    &self.ladder,
                ) {
                    let _ = avg;
                    if !viable {
                        acts.push(Action::Log {
                            level: "WARN",
                            message: format!(
                                "M20_SL_CONFLICT: SL merdivenden dar — safety order #{} erişilemez. Erken stop durumunda kalan kademelere ayrılan sermaye ölü kalır (kullanıcı onayıyla devam ediliyor).",
                                first.unwrap_or(0)
                            ),
                        });
                    }
                }
            }
        }
        // Task 16 (spec 2.8): ilk N güvenlik emrini tahtaya yaz (single-active yerine).
        // Faz 17: yeni deal — deal-bazlı funding sıfırlanır (P1-5), yerleştirme
        // imleci N'e ayarlanır (P1-16: top-up L(N+1)'den devam eder, çift yok).
        // Faz 18 (P1-2): deal-bazlı komisyon defteri de yeni deal'da sıfırlanır.
        // Faz 19 (P1-4): yeni deal önceki deal'in dinlenen TP'sini taşımaz —
        // eski emir rölanti tahtada kalmasın diye iptal niyeti yayılır.
        self.funding_deal = Fraction::zero();
        self.deal_fees = Fraction::zero();
        if let Some(rt) = self.resting_tp.take() {
            acts.push(Action::CancelResting { id: rt.order_id });
        }
        let n = self.cfg.risk_policy.active_safety_orders.max(1) as usize;
        // Faz 17 (P1-16): yerleştirme imleci = yerleştirilen seviye sayısı;
        // top-up bundan SONRAKI seviyeye yazar (çift yerleştirme imkansız).
        self.next_level_to_place = n;
        let levels: Vec<DcaLevel> = self.ladder.iter().take(n).cloned().collect();
        for lvl in levels {
            let act = self.place_safety_order(&lvl);
            if matches!(act, Action::PlaceLimit { .. }) {
                self.resting_safety += 1;
            }
            acts.push(act);
        }
        // TP & SL
        let q = self.mirror.qty.clone();
        if let Some(avg) = &self.mirror.avg_price {
            match self.compute_tp(&q, avg) {
                Ok(tp) => self.tp_price = tp,
                Err(e) => acts.push(Action::Log { level: "ERROR", message: e }),
            }
            if let Some(sl_pct) = &self.cfg.stop_loss_pct {
                // Parse failure here is unreachable for deals opened after the
                // try_open_deal gate below, but a zero/negative SL must never
                // silently disable the stop — log loudly instead.
                let sl = Fraction::from_decimal_str(sl_pct).unwrap_or_else(|_| Fraction::zero());
                if !sl.is_zero() {
                    let trig = if self.direction == Direction::Long {
                        avg.mul(&Fraction::one().sub(&sl).unwrap_or_else(|_| Fraction::zero()))
                    } else {
                        avg.mul(&Fraction::one().add(&sl).unwrap_or_else(|_| Fraction::zero()))
                    };
                    self.sl_price = trig.ok().map(|x| if self.direction == Direction::Long {
                        dca_math::m02::tick_down(&x, &self.inst.tick_size).unwrap_or_else(|_| x)
                    } else {
                        dca_math::m02::tick_up(&x, &self.inst.tick_size).unwrap_or_else(|_| x)
                    });
                }
            }
        }
        acts
    }

    fn rebuild_ladder_from_anchor(&mut self, anchor: &Fraction) {
        let base_qty = Fraction::from_decimal_str(&self.cfg.base_order_size).unwrap_or_else(|_| Fraction::one());
        let safety_size = Fraction::from_decimal_str(&self.cfg.safety_order_size).unwrap_or_else(|_| Fraction::one());
        let deviation = Fraction::from_decimal_str(&self.cfg.deviation_pct).unwrap_or_else(|_| Fraction::from_decimal_str("0.02").unwrap());
        let step_scale = Fraction::from_decimal_str(&self.cfg.step_scale).unwrap_or_else(|_| Fraction::one());
        let volume_scale = Fraction::from_decimal_str(&self.cfg.volume_scale).unwrap_or_else(|_| Fraction::one());
        if let Ok(l) = build_dca_plan(
            anchor,
            &base_qty,
            &safety_size,
            &deviation,
            &step_scale,
            &volume_scale,
            self.cfg.max_safety_orders,
            self.direction,
            &self.inst.tick_size,
            &self.inst.step_size,
            self.cfg.sizing_mode,
        ) {
            self.ladder = l;
        }
    }

    fn place_safety_order(&mut self, level: &DcaLevel) -> Action {
        // Safety orders whose notional rounds below the instrument step size
        // are untradeable — skipping them keeps M01 domain intact instead of
        // poisoning the whole run with a zero-qty fill.
        if level.qty.is_zero() {
            return Action::Log {
                level: "WARN",
                message: format!(
                    "SAFETY L{} skipped: qty rounds to zero (notional below step_size)",
                    level.level_index
                ),
            };
        }
        let safety_seq = self.next_seq();
        Action::PlaceLimit {
            id: order_id(&self.bot_id, "SAFETY", safety_seq),
            side: if self.direction == Direction::Long { dca_math::Side::Buy } else { dca_math::Side::Sell },
            price: level.price.clone(),
            qty: level.qty.clone(),
            role: "SAFETY",
        }
    }

    /// Dynamic config patch (TP/SL/trailing) on active deal.
    pub fn patch(&mut self, target_profit_pct: Option<String>, stop_loss_pct: Option<Option<String>>, trailing_enabled: Option<bool>) -> Result<(), String> {
        if let Some(tp) = target_profit_pct {
            if Fraction::from_decimal_str(&tp).map_err(|e| e.to_string())?.is_zero() {
                return Err("TP must be > 0".into());
            }
            self.cfg.target_profit_pct = tp;
            let q = self.mirror.qty.clone();
            if let Some(avg) = &self.mirror.avg_price {
                self.tp_price = self.compute_tp(&q, avg)?;
            }
        }
        if let Some(sl) = stop_loss_pct {
            self.cfg.stop_loss_pct = sl.clone();
            self.sl_price = None;
            if let (Some(pct), Some(avg)) = (&sl, &self.mirror.avg_price) {
                let p = Fraction::from_decimal_str(pct).map_err(|e| e.to_string())?;
                let trig = if self.direction == Direction::Long {
                    avg.mul(&Fraction::one().sub(&p).map_err(|e| e.to_string())?)
                } else {
                    avg.mul(&Fraction::one().add(&p).map_err(|e| e.to_string())?)
                }
                .map_err(|e| e.to_string())?;
                self.sl_price = Some(if self.direction == Direction::Long {
                    dca_math::m02::tick_down(&trig, &self.inst.tick_size).map_err(|e| e.to_string())?
                } else {
                    dca_math::m02::tick_up(&trig, &self.inst.tick_size).map_err(|e| e.to_string())?
                });
            }
        }
        if let Some(t) = trailing_enabled {
            self.cfg.trailing_enabled = t;
            self.trailing = if t { TrailingState::Idle } else { TrailingState::Off };
        }
        Ok(())
    }

    /// M09 MANUAL_ADD: inject an extra fill at mark price and re-derive TP/ladder.
    pub fn manual_add_funds(&mut self, price: &Fraction, quote_amount: &Fraction) -> Result<Vec<Action>, String> {
        if self.phase != DcaPhase::Open {
            return Err("no open deal".into());
        }
        let qty = dca_math::m02::qty_down(&quote_amount.div(price).map_err(|e| e.to_string())?, &self.inst.step_size)
            .map_err(|e| e.to_string())?;
        if qty.is_zero() {
            return Err("amount too small".into());
        }
        let fill_key = format!("manual-{}", self.next_seq());
        self.mirror.apply_fill(&fill_key, price, &qty, true).map_err(|e| e.to_string())?;
        // TP recompute
        let q = self.mirror.qty.clone();
        if let Some(avg) = &self.mirror.avg_price {
            self.tp_price = self.compute_tp(&q, avg)?;
        }
        Ok(vec![Action::Log { level: "INFO", message: format!("MANUAL_ADD {} @ {}", qty, price) }])
    }

    pub fn request_close_all(&mut self) {
        self.manual_close_requested = true;
    }

    /// Per-bar decision pass.
    pub fn on_bar(&mut self, ctx: &BarContext) -> Vec<Action> {
        let mut acts = Vec::new();
        // Maintain RSI window (exact)
        self.rsi_window.push(ctx.close.clone());
        if self.rsi_window.len() > 200 {
            self.rsi_window.remove(0);
        }

        match self.phase {
            DcaPhase::Idle | DcaPhase::Cooldown => {
                acts.extend(self.try_open_deal(ctx));
            }
            DcaPhase::BasePending => {
                // wait for fill event (runner calls on_base_filled)
            }
            DcaPhase::Open => {
                let mark = &ctx.mark;
                // SL check (mark-based trigger)
                if let Some(sl) = &self.sl_price {
                    let hit = if self.direction == Direction::Long {
                        dca_math::Fraction::cmp(mark, sl) != std::cmp::Ordering::Greater
                    } else {
                        dca_math::Fraction::cmp(mark, sl) != std::cmp::Ordering::Less
                    };
                    if hit {
                        // Faz 19 (P1-4): SL piyasa ile kapanır — dinlenen TP emri
                        // geride kalmamalı (bayat TP yeni pozisyonu kapatabilir).
                        if let Some(rt) = self.resting_tp.take() {
                            acts.push(Action::CancelResting { id: rt.order_id });
                        }
                        acts.push(Action::PlaceMarket {
                            id: order_id(&self.next_seq_id(), "SL", 0),
                            side: if self.direction == Direction::Long { dca_math::Side::Sell } else { dca_math::Side::Buy },
                            qty: self.mirror.qty.clone(),
                            role: "SL",
                        });
                        self.phase = DcaPhase::Closing;
                        self.close_orders_out += 1;
                        acts.push(Action::CancelResting { id: format!("{}-SAFETY-*", self.bot_id) });
                        return acts;
                    }
                }
                // Trailing machine (M10)
                if self.trailing != TrailingState::Off && !self.mirror.qty.is_zero() {
                    if let (Some(trigger_pct), Some(dev_pct)) = (
                        Fraction::from_decimal_str(&self.cfg.trailing_trigger_pct).ok(),
                        Fraction::from_decimal_str(&self.cfg.trailing_deviation_pct).ok(),
                    ) {
                        if let Some(avg) = &self.mirror.avg_price {
                            match self.trailing {
                                TrailingState::Idle => {
                                    let activation = if self.direction == Direction::Long {
                                        avg.mul(&Fraction::one().add(&trigger_pct).unwrap_or_else(|_| avg.clone()))
                                    } else {
                                        avg.mul(&Fraction::one().sub(&trigger_pct).unwrap_or_else(|_| avg.clone()))
                                    };
                                    let armed = if self.direction == Direction::Long {
                                        dca_math::Fraction::cmp(mark, &activation.unwrap_or_else(|_| avg.clone())) != std::cmp::Ordering::Less
                                    } else {
                                        dca_math::Fraction::cmp(mark, &activation.unwrap_or_else(|_| avg.clone())) != std::cmp::Ordering::Greater
                                    };
                                    if armed {
                                        self.trailing = TrailingState::Armed;
                                        self.trailing_peak = Some(mark.clone());
                                    }
                                }
                                TrailingState::Armed => {
                                    let peak = self.trailing_peak.clone().unwrap_or_else(|| mark.clone());
                                    let better = if self.direction == Direction::Long {
                                        dca_math::Fraction::cmp(mark, &peak) == std::cmp::Ordering::Greater
                                    } else {
                                        dca_math::Fraction::cmp(mark, &peak) == std::cmp::Ordering::Less
                                    };
                                    if better {
                                        self.trailing_peak = Some(mark.clone());
                                    } else {
                                        let drop = if self.direction == Direction::Long {
                                            peak.mul(&Fraction::one().sub(&dev_pct).unwrap_or_else(|_| peak.clone()))
                                        } else {
                                            peak.mul(&Fraction::one().add(&dev_pct).unwrap_or_else(|_| peak.clone()))
                                        };
                                        let exit = if self.direction == Direction::Long {
                                            dca_math::Fraction::cmp(mark, &drop.unwrap_or_else(|_| peak.clone())) != std::cmp::Ordering::Greater
                                        } else {
                                            dca_math::Fraction::cmp(mark, &drop.unwrap_or_else(|_| peak.clone())) != std::cmp::Ordering::Less
                                        };
                                        if exit {
                                            // Faz 19 (P1-4): trailing kapanışı — dinlenen TP iptal.
                                            if let Some(rt) = self.resting_tp.take() {
                                                acts.push(Action::CancelResting { id: rt.order_id });
                                            }
                                            acts.push(Action::PlaceMarket {
                                                id: order_id(&self.next_seq_id(), "TRAIL", 0),
                                                side: if self.direction == Direction::Long { dca_math::Side::Sell } else { dca_math::Side::Buy },
                                                qty: self.mirror.qty.clone(),
                                                role: "TP",
                                            });
                                            self.phase = DcaPhase::Closing;
                                            self.close_orders_out += 1;
                                            self.trailing = TrailingState::Triggered;
                                            return acts;
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
                // Faz 19 (P1-4 kalıcı): TP ARTIK RESTING reduce-only LIMIT'tir.
                // Eski akış mark TP'ye değince piyasa emri açıyordu: gerçek dolum
                // bir sonraki bar açılışı ± slippage olur, "net TP" hedefi
                // sistematik olarak tutmazdı. Yeni akış: hedef fiyat borsada
                // reduce-only LIMIT olarak bekler; dolum gerçekçi modelden gelir
                // (Maker, 1-tick penetrasyon + hacim payı). WAC değişince
                // (safety dolumu → yeni TP/adet) iptal + yeniden yazılır.
                if let Some(tp) = self.tp_price.clone() {
                    if self.mirror.qty.is_zero() {
                        if let Some(rt) = self.resting_tp.take() {
                            acts.push(Action::CancelResting { id: rt.order_id });
                        }
                    } else {
                        let needs_replace = match &self.resting_tp {
                            None => true,
                            Some(rt) => rt.price != tp || rt.qty != self.mirror.qty,
                        };
                        if needs_replace {
                            if let Some(old) = self.resting_tp.take() {
                                acts.push(Action::CancelResting { id: old.order_id });
                            }
                            let id = order_id(&self.next_seq_id(), "TPL", 0);
                            acts.push(Action::PlaceLimit {
                                id: id.clone(),
                                side: if self.direction == Direction::Long { dca_math::Side::Sell } else { dca_math::Side::Buy },
                                price: tp.clone(),
                                qty: self.mirror.qty.clone(),
                                role: "TP",
                            });
                            self.resting_tp = Some(RestingTp {
                                order_id: id,
                                price: tp.clone(),
                                qty: self.mirror.qty.clone(),
                            });
                        }
                    }
                } else if let Some(rt) = self.resting_tp.take() {
                    // TP hedefi kalktı (config/patch değişimi) — dinlenen emri çek.
                    acts.push(Action::CancelResting { id: rt.order_id });
                }
                // Manual close
                if self.manual_close_requested && !self.mirror.qty.is_zero() {
                    // Faz 19 (P1-4): manuel kapanış — dinlenen TP iptal edilir.
                    if let Some(rt) = self.resting_tp.take() {
                        acts.push(Action::CancelResting { id: rt.order_id });
                    }
                    acts.push(Action::PlaceMarket {
                        id: order_id(&self.next_seq_id(), "MANUAL", 0),
                        side: if self.direction == Direction::Long { dca_math::Side::Sell } else { dca_math::Side::Buy },
                        qty: self.mirror.qty.clone(),
                        role: "MANUAL",
                    });
                    self.phase = DcaPhase::Closing;
                    self.close_orders_out += 1;
                    self.manual_close_requested = false;
                    return acts;
                }
            }
            DcaPhase::Closing => {
                // Faz 17 (P0-6): kapanış emri 3 bar dolmadıysa yeniden gönder —
                // pozisyon korumasız kalmaz (bakım/reddetme senaryoları).
                self.closing_bars_waited += 1;
                if self.closing_bars_waited > 3 && !self.mirror.qty.is_zero() {
                    // Faz 19 (P1-4): savunmacı — Closing'e giren yollar TP'yi
                    // iptal eder; kalan bir kayıt varsa burada da düşülür.
                    if let Some(rt) = self.resting_tp.take() {
                        acts.push(Action::CancelResting { id: rt.order_id });
                    }
                    acts.push(Action::PlaceMarket {
                        id: order_id(&self.next_seq_id(), "MANUAL", 0),
                        side: if self.direction == Direction::Long { dca_math::Side::Sell } else { dca_math::Side::Buy },
                        qty: self.mirror.qty.clone(),
                        role: "MANUAL",
                    });
                    self.close_orders_out += 1;
                    self.closing_bars_waited = 0;
                    acts.push(Action::Log {
                        level: "WARN",
                        message: format!(
                            "kapanış emri 3 barda dolmadı — kalan {} için MANUAL kapanış yeniden gönderildi (P0-6)",
                            self.mirror.qty.to_half_even_display(8)
                        ),
                    });
                }
            }
        }
        acts
    }

    /// Runner forwards every execution fill here (single source of truth mirror).
    pub fn on_fill(&mut self, fill: &FillRecord) -> Vec<Action> {
        let mut acts = Vec::new();
        let is_add = fill.kind == OrderKind::Add;
        if fill.role == "SAFETY" {
            // ".m{n}" konsolide dolum sayacı on_fill SAFETY kolunda işlenir;
            // burada yalnız ham fill sayacı tutulur.
            self.filled_safety += 1;
        }
        // Faz 18 (P1-2): deal komisyon defteri — her dolumun ücreti deal
        // bazında birikir; win/loss kararı net bilanço üzerinden verilir.
        self.deal_fees = self
            .deal_fees
            .add(&fill.fee)
            .unwrap_or_else(|_| self.deal_fees.clone());
        let before_qty = self.mirror.qty.clone();
        let _ = self.mirror.apply_fill(&fill.fill_key, &fill.price, &fill.qty, is_add);

        // Track deal completion via closing fills
        if !is_add {
            self.closed_qty_accum = self.closed_qty_accum.add(&fill.qty).unwrap_or_else(|_| self.closed_qty_accum.clone());
        }
        if before_qty.is_zero() && is_add && fill.role == "BASE" && self.phase == DcaPhase::BasePending {
            acts.extend(self.on_base_filled(&fill.price));
            return acts;
        }
        // Safety chain: next single active order
        if is_add && fill.role == "SAFETY" && self.phase == DcaPhase::Open {
            // Task 16 — konsolide gap dolumu: ".m{n}" eki, bu dolumun n kademeyi
            // birden tükettiğini söyler (Gate 8b tek VWAP dolum).
            let merged_levels = fill
                .order_id
                .rsplit('.')
                .next()
                .and_then(|sfx| sfx.strip_prefix('m'))
                .and_then(|n| n.parse::<usize>().ok())
                .unwrap_or(1);
            self.filled_safety += merged_levels.saturating_sub(1);
            // n bekleyen emir tek dolumda tükendi → bekleyen sayacı n kadar düşer.
            self.resting_safety = self.resting_safety.saturating_sub(merged_levels);
            // Top-up: bekleyen sayısını policy.active_safety_orders'a tamamla.
            let want = self.cfg.risk_policy.active_safety_orders.max(1) as usize;
            // Faz 17 (P1-16): top-up YERLEŞTİRME imleciyle —  yalnız
            // dolum sayacı; böylece L2 tahtadayken L2 tekrar yazılmaz (çift emir yok).
            while self.resting_safety < want && self.next_level_to_place < self.ladder.len() {
                let lvl = self.ladder[self.next_level_to_place].clone();
                let act = self.place_safety_order(&lvl);
                if matches!(act, Action::PlaceLimit { .. }) {
                    self.resting_safety += 1;
                }
                acts.push(act);
                self.next_level_to_place += 1;
            }
            // TP recomputation after each fill (WAC changed)
            let q = self.mirror.qty.clone();
            if let Some(avg) = &self.mirror.avg_price {
                if let Ok(tp) = self.compute_tp(&q, avg) {
                    self.tp_price = tp;
                }
            }
        }
        // Deal fully closed?
        // Faz 19 (P1-4): dinlenen TP dolumu fazı Closing'e ALMADAN flat yapar
        // (resting TP iken faz Open kalır) — kapanış tespiti her iki fazı görür.
        if !is_add && self.mirror.qty.is_zero() && matches!(self.phase, DcaPhase::Closing | DcaPhase::Open) {
            // Faz 19 (P1-4): pozisyon kapandı — dinlenen TP (kısmi dolum
            // artığı olabilir) mutlaka geri çekilir.
            if let Some(rt) = self.resting_tp.take() {
                acts.push(Action::CancelResting { id: rt.order_id });
            }
            let realized = self.mirror.realized_gross.clone();
            // M-17: mirror bir sonraki deal'da sıfırlanır — kapanış realizasyonunu
            // kümülatif deftere devretmeden kaybedilmez.
            self.cum_realized_gross = self
                .cum_realized_gross
                .add(&realized)
                .unwrap_or_else(|_| self.cum_realized_gross.clone());
            // Faz 18 (P1-2): net bilanço — deal komisyonları ve deal funding
            // düşülerek kazanma/kaybetme kararı verilir (brüt PnL değil).
            // Aritmetik hatada brüt değerle devam edilir (karar yine kaydedilir).
            let net = realized
                .sub(&self.deal_fees)
                .and_then(|x| x.sub(&self.funding_deal))
                .unwrap_or_else(|_| realized.clone());
            if dca_math::Fraction::cmp(&net, &Fraction::zero()) == std::cmp::Ordering::Greater {
                self.wins += 1;
            } else {
                self.losses += 1;
            }
            self.deals_closed += 1;
            self.phase = DcaPhase::Cooldown;
            self.cooldown_remaining = self.cfg.cooldown_bars;
            self.close_orders_out = 0;
            self.closes_seen = 0;
            self.closed_qty_accum = Fraction::zero();
            self.closing_bars_waited = 0;
            acts.push(Action::Log {
                level: "INFO",
                message: format!(
                    "deal closed gross={} fees={} funding={} net={} wins={} losses={}",
                    realized.to_half_even_display(8),
                    self.deal_fees.to_half_even_display(8),
                    self.funding_deal.to_half_even_display(8),
                    net.to_half_even_display(8),
                    self.wins,
                    self.losses
                ),
            });
        }
        acts
    }

    pub fn apply_funding(&mut self, expense: &Fraction) {
        self.funding = self.funding.add(expense).unwrap_or_else(|_| self.funding.clone());
        self.funding_deal = self.funding_deal.add(expense).unwrap_or_else(|_| self.funding_deal.clone());
    }

    /// Margin call: the position was force-closed at the liquidation boundary.
    /// Mirror is already flat (runner force_close); reset deal bookkeeping.
    pub fn force_liquidate(&mut self) {
        // M-17: liquidasyon öncesi mirror'da birikmiş realizasyonu kümülatif
        // deftere devret (mirror sıfırlansa da bot ömrü k/z'si korunur).
        self.cum_realized_gross = self
            .cum_realized_gross
            .add(&self.mirror.realized_gross.clone())
            .unwrap_or_else(|_| self.cum_realized_gross.clone());
        self.losses += 1;
        self.deals_closed += 1;
        self.phase = DcaPhase::Cooldown;
        self.cooldown_remaining = self.cfg.cooldown_bars;
        self.close_orders_out = 0;
        self.closes_seen = 0;
        self.closed_qty_accum = Fraction::zero();
        self.filled_safety = 0;
        self.resting_safety = 0;
        // Faz 19 (P1-4): likidasyonda dinlenen TP kaydı düşülür — borsa tarafı
        // iptali çağıran (engine.rs) CancelResting ile yayılır.
        self.resting_tp = None;
        self.tp_price = None;
        self.sl_price = None;
        self.trailing = TrailingState::Off;
        self.trailing_peak = None;
        self.ladder.clear();
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            phase: format!("{:?}", self.phase),
            deal_id: if self.phase == DcaPhase::Idle || self.phase == DcaPhase::Cooldown {
                None
            } else {
                Some(format!("{}-deal-{}", self.bot_id, self.deal_seq))
            },
            position_qty: self.mirror.qty.to_half_even_display(8),
            avg_price: self.mirror.avg_price.as_ref().map(|a| self.close_str(a, 8)),
            // Faz 18 (P1-16 kalıntı): "sıradaki güvenlik" gösterimi yerleştirme
            // imlecini izler — dolum sayacı (filled_safety) top-up sonrası
            // merdiven imlecinden sapabilir (çoklu resting + konsolide dolum).
            next_safety_price: self
                .ladder
                .get(self.next_level_to_place)
                .map(|l| self.close_str(&l.price, 8)),
            next_safety_level: self.ladder.get(self.next_level_to_place).map(|l| l.level_index),
            tp_price: self.tp_price.as_ref().map(|p| self.close_str(p, 8)),
            sl_price: self.sl_price.as_ref().map(|p| self.close_str(p, 8)),
            trailing_state: Some(format!("{:?}", self.trailing)),
            trailing_peak: self.trailing_peak.as_ref().map(|p| self.close_str(p, 8)),
            safety_triggered: self.filled_safety,
            // M-17: bot ömrü kümülatif realizasyon (deals_closed ile aynı pencere);
            // mirror deal-başı olduğundan artık raporlanmaz.
            realized_gross: self.cum_realized_gross.to_half_even_display(8),
            cumulative_funding: self.funding.to_half_even_display(8),
            deals_closed: self.deals_closed,
            wins: self.wins,
            losses: self.losses,
            entry_trigger: self.cfg.entry_trigger.describe(),
            entry_trigger_met: self.entry_trigger_met,
            entry_signal_met: self.last_signal_met,
            hedge: None,
        }
    }
}

#[cfg(test)]
mod start_condition_tests {
    use super::*;
    use crate::{evaluate_entry_trigger, EntryTrigger};
    use dca_kernel::Bar;
    use dca_math::Instrument;

    fn bar(t: i64, c: &str) -> Bar {
        Bar { t, o: c.into(), h: c.into(), l: c.into(), c: c.into(), v: "10".into() }
    }

    fn ctx<'a>(bar: &'a Bar, close: Fraction) -> BarContext<'a> {
        // leak tiny constants for the &'a Fraction fields (test-only)
        let qty: &'static Fraction = Box::leak(Box::new(Fraction::zero()));
        let cash: &'static Fraction = Box::leak(Box::new(Fraction::from_u64(100_000)));
        let mark = close.clone();
        BarContext {
            bar,
            open: close.clone(),
            high: close.clone(),
            low: close.clone(),
            close,
            position_qty: qty,
            position_avg: None,
            cash,
            mark,
            signal_ok: None,
            regime: None,
            deal_open_blocked: false,
        }
    }

    fn engine_with(trig: EntryTrigger) -> DcaEngine {
        let mut cfg = BotConfig::default();
        cfg.entry_trigger = trig;
        DcaEngine::new("t-bot", cfg, Instrument::default()).expect("engine")
    }

    #[test]
    fn evaluate_semantics() {
        let f = |s: &str| Fraction::from_decimal_str(s).unwrap();
        // Instant
        assert!(evaluate_entry_trigger(&EntryTrigger::Instant, &f("1"), 0, None).unwrap());
        // PRICE_BELOW: ≤ is inclusive
        let pb = EntryTrigger::PriceBelow("100".into());
        assert!(evaluate_entry_trigger(&pb, &f("99.99"), 0, None).unwrap());
        assert!(evaluate_entry_trigger(&pb, &f("100"), 0, None).unwrap());
        assert!(!evaluate_entry_trigger(&pb, &f("100.01"), 0, None).unwrap());
        // PRICE_ABOVE: ≥ is inclusive
        let pa = EntryTrigger::PriceAbove("100".into());
        assert!(evaluate_entry_trigger(&pa, &f("100.01"), 0, None).unwrap());
        assert!(!evaluate_entry_trigger(&pa, &f("99.99"), 0, None).unwrap());
        // DROP_FROM_REF: 100000 −10% → 90000 trigger, inclusive
        let dr = EntryTrigger::DropFromRef { ref_price: "100000".into(), drop_pct: "0.1".into() };
        assert!(evaluate_entry_trigger(&dr, &f("90000"), 0, None).unwrap());
        assert!(!evaluate_entry_trigger(&dr, &f("90000.000001"), 0, None).unwrap());
        assert!(evaluate_entry_trigger(&dr, &f("89999"), 0, None).unwrap());
        // invalid drop_pct rejected
        let bad = EntryTrigger::DropFromRef { ref_price: "100".into(), drop_pct: "1".into() };
        assert!(evaluate_entry_trigger(&bad, &f("50"), 0, None).is_err());
        // SCHEDULED: bar time ≥ start_ms
        let sc = EntryTrigger::Scheduled { start_ms: 1_700_000_000_000 };
        assert!(!evaluate_entry_trigger(&sc, &f("1"), 1_699_999_999_999, None).unwrap());
        assert!(evaluate_entry_trigger(&sc, &f("1"), 1_700_000_000_000, None).unwrap());
        // RSI_BELOW: None window ⇒ false; supplied value compared strictly
        let rsi_trig = EntryTrigger::RsiBelow(30);
        assert!(!evaluate_entry_trigger(&rsi_trig, &f("1"), 0, None).unwrap());
        assert!(evaluate_entry_trigger(&rsi_trig, &f("1"), 0, Some(&f("29.9"))).unwrap());
        assert!(!evaluate_entry_trigger(&rsi_trig, &f("1"), 0, Some(&f("30"))).unwrap());
    }

    #[test]
    fn dca_gate_blocks_then_opens() {
        let mut e = engine_with(EntryTrigger::PriceBelow("50000".into()));
        let b = bar(1_700_000_000_000, "60000");
        let acts = e.try_open_deal(&ctx(&b, Fraction::from_decimal_str("60000").unwrap()));
        assert!(acts.is_empty(), "condition not met: no orders");
        assert_eq!(e.phase, DcaPhase::Idle);
        assert!(!e.entry_trigger_met);
        assert_eq!(e.blocked_trigger_bars, 1, "no-deal sayacı: trigger");
        assert_eq!(e.snapshot().entry_trigger, "PRICE ≤ 50000");
        assert!(!e.snapshot().entry_trigger_met);

        // price touches the threshold (inclusive) → BASE dispatched
        let acts = e.try_open_deal(&ctx(&b, Fraction::from_decimal_str("50000").unwrap()));
        assert!(acts.iter().any(|a| matches!(a, Action::PlaceMarket { role: "BASE", .. })));
        assert_eq!(e.phase, DcaPhase::BasePending);
        assert!(e.entry_trigger_met);
        assert!(e.snapshot().entry_trigger_met);
    }

    #[test]
    fn dca_scheduled_future_blocks_then_time_opens() {
        let t0 = 1_700_000_000_000i64;
        let mut e = engine_with(EntryTrigger::Scheduled { start_ms: t0 });
        let early = bar(t0 - 3_600_000, "100");
        assert!(e.try_open_deal(&ctx(&early, Fraction::from_u64(100))).is_empty());
        assert!(!e.entry_trigger_met);
        let late = bar(t0 + 3_600_000, "100");
        let acts = e.try_open_deal(&ctx(&late, Fraction::from_u64(100)));
        assert!(acts.iter().any(|a| matches!(a, Action::PlaceMarket { role: "BASE", .. })));
        assert!(e.entry_trigger_met);
    }

    #[test]
    fn invalid_trigger_rejected_at_construction() {
        let mut cfg = BotConfig::default();
        cfg.entry_trigger = EntryTrigger::PriceBelow("0".into());
        assert!(DcaEngine::new("x", cfg, Instrument::default()).is_err());
        let mut cfg = BotConfig::default();
        cfg.entry_trigger = EntryTrigger::Scheduled { start_ms: -5 };
        assert!(DcaEngine::new("x", cfg, Instrument::default()).is_err());
    }

    #[test]
    fn wire_format_roundtrip() {
        // legacy plain string
        let v: serde_json::Value = serde_json::json!("Instant");
        let t: EntryTrigger = serde_json::from_value(v).unwrap();
        assert_eq!(t, EntryTrigger::Instant);
        // SCREAMING names
        let t: EntryTrigger = serde_json::from_value(serde_json::json!({"PRICE_BELOW": "80000"})).unwrap();
        assert_eq!(t, EntryTrigger::PriceBelow("80000".into()));
        let t: EntryTrigger = serde_json::from_value(serde_json::json!({"RSI_BELOW": 30})).unwrap();
        assert_eq!(t, EntryTrigger::RsiBelow(30));
        let t: EntryTrigger =
            serde_json::from_value(serde_json::json!({"DROP_FROM_REF": {"ref_price": "90000", "drop_pct": "0.15"}})).unwrap();
        assert_eq!(t, EntryTrigger::DropFromRef { ref_price: "90000".into(), drop_pct: "0.15".into() });
        let t: EntryTrigger = serde_json::from_value(serde_json::json!({"SCHEDULED": {"start_ms": 123}})).unwrap();
        assert_eq!(t, EntryTrigger::Scheduled { start_ms: 123 });
        // serialize → canonical SCREAMING names
        let s = serde_json::to_value(EntryTrigger::PriceAbove("5".into())).unwrap();
        assert_eq!(s, serde_json::json!({"PRICE_ABOVE": "5"}));
        let s = serde_json::to_value(EntryTrigger::Instant).unwrap();
        assert_eq!(s, serde_json::json!("INSTANT"));
    }
}

#[cfg(test)]
mod signal_gate_tests {
    use super::*;
    use crate::BarContext;
    use dca_kernel::Bar;
    use dca_math::Instrument;

    fn bar(t: i64, c: &str) -> Bar {
        Bar { t, o: c.into(), h: c.into(), l: c.into(), c: c.into(), v: "10".into() }
    }

    fn ctx_with_signal<'a>(bar: &'a Bar, close: Fraction, signal_ok: Option<bool>) -> BarContext<'a> {
        let qty: &'static Fraction = Box::leak(Box::new(Fraction::zero()));
        let cash: &'static Fraction = Box::leak(Box::new(Fraction::from_u64(100_000)));
        let mark = close.clone();
        BarContext {
            bar,
            open: close.clone(),
            high: close.clone(),
            low: close.clone(),
            close,
            position_qty: qty,
            position_avg: None,
            cash,
            mark,
            signal_ok,
            regime: None,
            deal_open_blocked: false,
        }
    }

    #[test]
    fn signal_false_blocks_deal_fail_closed() {
        let mut e = DcaEngine::new("sg", BotConfig::default(), Instrument::default()).unwrap();
        let b = bar(1_000, "100");
        let acts = e.try_open_deal(&ctx_with_signal(&b, Fraction::from_u64(100), Some(false)));
        assert!(acts.iter().all(|a| !matches!(a, Action::PlaceMarket { .. })));
        assert_eq!(e.phase, DcaPhase::Idle);
        assert_eq!(e.last_signal_met, Some(false));
        assert_eq!(e.blocked_signal_bars, 1, "no-deal sayacı: sinyal");
    }

    #[test]
    fn signal_true_opens_deal() {
        let mut e = DcaEngine::new("sg", BotConfig::default(), Instrument::default()).unwrap();
        let b = bar(1_000, "100");
        let acts = e.try_open_deal(&ctx_with_signal(&b, Fraction::from_u64(100), Some(true)));
        assert!(acts.iter().any(|a| matches!(a, Action::PlaceMarket { .. })));
        assert_eq!(e.last_signal_met, Some(true));
    }

    /* ---------------- Task 16 — Risk Motoru motor testleri ---------------- */

    use dca_signals::MarketRegime;

    fn ctx_full<'a>(
        bar: &'a Bar,
        close: Fraction,
        regime: Option<MarketRegime>,
        blocked: bool,
    ) -> BarContext<'a> {
        let qty: &'static Fraction = Box::leak(Box::new(Fraction::zero()));
        let cash: &'static Fraction = Box::leak(Box::new(Fraction::from_u64(1_000_000)));
        BarContext {
            bar,
            open: close.clone(),
            high: close.clone(),
            low: close.clone(),
            close: close.clone(),
            position_qty: qty,
            position_avg: None,
            cash,
            mark: close,
            signal_ok: None,
            regime,
            deal_open_blocked: blocked,
        }
    }

    /// spec 2.8: active_safety_orders = 3 → base fill sonrası 3 safety tahtada;
    /// biri dolunca tamına kadar yeni emir konur.
    #[test]
    fn multi_resting_safety_orders_top_up() {
        let mut cfg = BotConfig::default();
        cfg.risk_policy.active_safety_orders = 3;
        cfg.max_safety_orders = 5;
        let mut e = DcaEngine::new("ms", cfg, Instrument::default()).unwrap();
        let b = bar(1_000, "100");
        let acts = e.try_open_deal(&ctx_full(&b, Fraction::from_u64(100), None, false));
        let base = acts.iter().find_map(|a| match a {
            Action::PlaceMarket { id, role: "BASE", .. } => Some(id.clone()),
            _ => None,
        }).expect("base order");
        // Base fill simülasyonu.
        let fill = FillRecord {
            fill_key: "f0".into(),
            order_id: base,
            role: "BASE".into(),
            side: dca_math::Side::Buy,
            kind: OrderKind::Add,
            price: Fraction::from_u64(100),
            qty: Fraction::from_u64(1),
            fee: Fraction::zero(),
            fee_role: dca_kernel::FeeRole::Taker,
            realism: dca_risk::FillRealism::MarketableTaker,
            ts: 1,
        };
        let acts = e.on_fill(&fill);
        let safety_placed = acts.iter().filter(|a| matches!(a, Action::PlaceLimit { role: "SAFETY", .. })).count();
        assert_eq!(safety_placed, 3, "3 aktif güvenlik emri tahtaya yazılmalı");
        assert_eq!(e.resting_safety, 3);
        // L1 dolunca yalnız 1 yeni emir (top-up).
        let l1_id = acts.iter().find_map(|a| match a {
            Action::PlaceLimit { id, role: "SAFETY", .. } => Some(id.clone()),
            _ => None,
        }).expect("L1 id");
        let fill1 = FillRecord {
            fill_key: "f1".into(),
            order_id: l1_id,
            role: "SAFETY".into(),
            side: dca_math::Side::Buy,
            kind: OrderKind::Add,
            price: Fraction::from_u64(98),
            qty: Fraction::from_u64(1),
            fee: Fraction::zero(),
            fee_role: dca_kernel::FeeRole::Maker,
            realism: dca_risk::FillRealism::RestingTouch,
            ts: 2,
        };
        let acts = e.on_fill(&fill1);
        let new_safety = acts.iter().filter(|a| matches!(a, Action::PlaceLimit { role: "SAFETY", .. })).count();
        assert_eq!(new_safety, 1, "top-up: bekleyen sayısı 3'e tamamlanmalı");
        assert_eq!(e.resting_safety, 3);
        // Faz 17 (P1-16): filled_safety yalnız DOLUM sayacı; yerleştirme imleci
        // ayrı — L1 dolunca top-up L2'yi TEKRAR yazmaz, L4'ü yazar (fiyat kanıtı).
        assert_eq!(e.filled_safety, 1, "filled_safety yalnız dolum sayacı");
        assert_eq!(e.next_level_to_place, 4, "yerleştirme imleci L4'ü göstermeli");
        let l4_price = e.ladder[3].price.to_string();
        let placed_price = acts.iter().find_map(|a| match a {
            Action::PlaceLimit { price, .. } => Some(price.to_string()),
            _ => None,
        }).expect("top-up emri");
        assert_eq!(placed_price, l4_price, "top-up çift fiyat yerleştirmemeli (eski bug: L2 tekrar)");
    }

    /// spec 2.7: CRASH rejiminde ENFORCE bloklar, PERMISSIVE uyarıyla geçirir.
    #[test]
    fn regime_filter_enforce_vs_permissive() {
        let b = bar(1_000, "100");
        // ENFORCE (açıkça seçilmiş): deal açılmaz.
        let mut cfg = BotConfig::default();
        cfg.risk_policy.regime_filter = dca_kernel::GuardAction::Enforce;
        let mut e = DcaEngine::new("rg", cfg, Instrument::default()).unwrap();
        let acts = e.try_open_deal(&ctx_full(&b, Fraction::from_u64(100), Some(MarketRegime::Crash), false));
        assert!(!acts.iter().any(|a| matches!(a, Action::PlaceMarket { role: "BASE", .. })));
        assert!(acts.iter().any(|a| matches!(a, Action::Log { level: "INFO", .. })));
        assert_eq!(e.blocked_regime_bars, 1, "no-deal sayacı: rejim");
        // PERMISSIVE: uyarı + deal açılır.
        let mut cfg = BotConfig::default();
        cfg.risk_policy.regime_filter = dca_kernel::GuardAction::PermissiveWithWarning;
        let mut e = DcaEngine::new("rg2", cfg, Instrument::default()).unwrap();
        let acts = e.try_open_deal(&ctx_full(&b, Fraction::from_u64(100), Some(MarketRegime::Crash), false));
        assert!(acts.iter().any(|a| matches!(a, Action::Log { level: "WARN", .. })), "uyarı loglanmalı");
        assert!(acts.iter().any(|a| matches!(a, Action::PlaceMarket { role: "BASE", .. })), "deal açılmalı");
    }

    /// spec 2.9: deal_open_blocked=true iken yeni deal açılmaz.
    #[test]
    fn daily_breaker_blocks_deal_open_only() {
        let mut e = DcaEngine::new("bk", BotConfig::default(), Instrument::default()).unwrap();
        let b = bar(1_000, "100");
        let acts = e.try_open_deal(&ctx_full(&b, Fraction::from_u64(100), None, true));
        assert!(acts.is_empty(), "kesici tetikliyken deal açılmamalı");
        assert_eq!(e.phase, DcaPhase::Idle);
        assert_eq!(e.blocked_breaker_bars, 1, "no-deal sayacı: kesici");
    }

    #[test]
    fn webhook_force_bypasses_signal_block() {
        let mut e = DcaEngine::new("sg", BotConfig::default(), Instrument::default()).unwrap();
        e.request_force_start();
        let b = bar(1_000, "100");
        let acts = e.try_open_deal(&ctx_with_signal(&b, Fraction::from_u64(100), Some(false)));
        assert!(acts.iter().any(|a| matches!(a, Action::PlaceMarket { .. })));
        assert!(!e.force_start_pending, "force flag must be consumed exactly once");
    }

    #[test]
    fn entry_signal_wire_roundtrip_and_validation() {
        let cfg_json = serde_json::json!({
            "kind": "DCA_LONG", "symbol": "BTCUSDT", "base_order_size": "100",
            "safety_order_size": "100", "sizing_mode": "QUOTE_NOTIONAL",
            "deviation_pct": "0.02", "step_scale": "1", "volume_scale": "1",
            "max_safety_orders": 3, "target_profit_pct": "0.02", "use_net_tp": true,
            "trailing_enabled": false, "trailing_trigger_pct": "0.01",
            "trailing_deviation_pct": "0.005", "stop_loss_pct": null,
            "leverage": "1", "hedge_ratio": "0.5", "combined_target_profit": "50",
            "grid_lower": null, "grid_upper": null, "grid_levels": null,
            "grid_mode": "arithmetic", "grid_trail": false, "cooldown_bars": 0,
            "entry_trigger": "INSTANT",
            "entry_signal": { "root": { "op": "And", "children": [
                { "left": { "key": "Rsi", "timeframe": "M15", "period": 14 }, "op": "Lt",
                  "right": { "kind": "number", "value": 30 } },
                { "left": { "key": "Price", "timeframe": "M15", "output": "close" }, "op": "Gt",
                  "right": { "kind": "indicator", "value": { "key": "Ema", "timeframe": "H1", "period": 200 } } }
            ]}}
        });
        let cfg: BotConfig = serde_json::from_value(cfg_json).unwrap();
        assert!(cfg.entry_signal.is_some());
        // missing entry_signal field → None (backward compatible)
        let old: BotConfig = serde_json::from_value(serde_json::json!({
            "kind": "DCA_LONG", "symbol": "BTCUSDT", "base_order_size": "100",
            "safety_order_size": "100", "sizing_mode": "QUOTE_NOTIONAL",
            "deviation_pct": "0.02", "step_scale": "1", "volume_scale": "1",
            "max_safety_orders": 3, "target_profit_pct": "0.02", "use_net_tp": true,
            "trailing_enabled": false, "trailing_trigger_pct": "0.01",
            "trailing_deviation_pct": "0.005", "stop_loss_pct": null,
            "leverage": "1", "hedge_ratio": "0.5", "combined_target_profit": "50",
            "grid_lower": null, "grid_upper": null, "grid_levels": null,
            "grid_mode": "arithmetic", "grid_trail": false, "cooldown_bars": 0,
            "entry_trigger": "INSTANT"
        }))
        .unwrap();
        assert!(old.entry_signal.is_none());
        // invalid tree rejected at engine construction
        let mut bad = BotConfig::default();
        bad.entry_signal = Some(dca_signals::SignalConfig {
            root: dca_signals::SignalNode::Condition(dca_signals::SignalCondition {
                left: dca_signals::IndicatorSpec {
                    key: dca_signals::IndicatorKey::Rsi,
                    timeframe: dca_signals::Timeframe::M15,
                    period: Some(0), // invalid: period 0
                    fast: None,
                    slow: None,
                    signal: None,
                    std_dev: None,
                    d_period: None,
                    output: None,
                },
                op: dca_signals::CompareOp::Lt,
                right: dca_signals::SignalValue::Number(30.0),
            }),
        });
        assert!(DcaEngine::new("bad", bad, Instrument::default()).is_err());
    }
}

/// M-17 regresyon: mirror deal-başı WAC defteridir; kümülatif realizasyon
/// `cum_realized_gross`'ta bot ömrü boyunca taşınır. deals_closed ile
/// realized_gross artık aynı pencereyi raporlar.
#[cfg(test)]
mod m17_realized_ledger_tests {
    use super::*;
    use dca_kernel::{Bar, OrderKind};
    use dca_math::{Instrument, Side};
    use dca_risk::FillRealism;

    fn bar(t: i64, c: &str) -> Bar {
        Bar { t, o: c.into(), h: c.into(), l: c.into(), c: c.into(), v: "10".into() }
    }

    fn ctx<'a>(bar: &'a Bar, close: Fraction) -> BarContext<'a> {
        let qty: &'static Fraction = Box::leak(Box::new(Fraction::zero()));
        let cash: &'static Fraction = Box::leak(Box::new(Fraction::from_u64(1_000_000)));
        BarContext {
            bar,
            open: close.clone(),
            high: close.clone(),
            low: close.clone(),
            close: close.clone(),
            position_qty: qty,
            position_avg: None,
            cash,
            mark: close,
            signal_ok: None,
            regime: None,
            deal_open_blocked: false,
        }
    }

    fn fill(id: &str, role: &str, side: Side, kind: OrderKind, price: &str, qty: &str, ts: i64) -> FillRecord {
        FillRecord {
            fill_key: format!("{id}-{ts}"),
            order_id: id.to_string(),
            role: role.to_string(),
            side,
            kind,
            price: Fraction::from_decimal_str(price).unwrap(),
            qty: Fraction::from_decimal_str(qty).unwrap(),
            fee: Fraction::zero(),
            fee_role: dca_kernel::FeeRole::Maker,
            realism: FillRealism::MarketableTaker,
            ts,
        }
    }

    /// Bir deal'i uçtan uca yürüt: BASE fill + request_close_all + MANUAL reduce fill.
    fn run_deal(e: &mut DcaEngine, base_px: &str, close_px: &str, qty: &str, t: i64) {
        let acts = e.on_bar(&ctx(&bar(t, base_px), Fraction::from_decimal_str(base_px).unwrap()));
        let base_id = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, role: "BASE", .. } => Some(id.clone()),
                _ => None,
            })
            .expect("base order");
        let (side, kind) = match e.direction {
            Direction::Long => (Side::Buy, OrderKind::Add),
            Direction::Short => (Side::Sell, OrderKind::Add),
        };
        e.on_fill(&fill(&base_id, "BASE", side, kind, base_px, qty, t));
        assert_eq!(e.phase, DcaPhase::Open);
        e.request_close_all();
        let acts = e.on_bar(&ctx(&bar(t + 1, close_px), Fraction::from_decimal_str(close_px).unwrap()));
        // close_px TP eşiğinin üstündeyse motor TP, altındaysa MANUAL rolüyle kapatır.
        let (close_id, close_role) = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, role, .. } if matches!(*role, "MANUAL" | "TP") => {
                    Some((id.clone(), *role))
                }
                _ => None,
            })
            .expect("close order (MANUAL/TP)");
        let (side, kind) = match e.direction {
            Direction::Long => (Side::Sell, OrderKind::Reduce),
            Direction::Short => (Side::Buy, OrderKind::Reduce),
        };
        e.on_fill(&fill(&close_id, close_role, side, kind, close_px, qty, t + 1));
        assert_eq!(e.phase, DcaPhase::Cooldown);
    }

    #[test]
    fn realized_gross_is_cumulative_across_deals() {
        let mut e = DcaEngine::new("m17", BotConfig::default(), Instrument::default()).unwrap();
        // Deal 1: 100'te aç, 90'da kapat → realized = (90-100)*1 = -10
        run_deal(&mut e, "100", "90", "1", 1_000);
        assert_eq!(e.deals_closed, 1);
        assert_eq!(e.losses, 1);
        assert_eq!(e.snapshot().realized_gross, "-10.00000000");

        // Deal 2: mirror sıfırlandı ama kümülatif korundu (M-17 bulgusu).
        run_deal(&mut e, "90", "95", "1", 2_000);
        assert_eq!(e.deals_closed, 2);
        assert_eq!(e.wins, 1);
        // -10 + (95-90)*1 = -5
        assert_eq!(e.snapshot().realized_gross, "-5.00000000");
    }

    #[test]
    fn deal_win_decision_is_net_of_fees_and_funding() {
        // Faz 18 (P1-2): brüt +0.5 ancak komisyon 1.2 + funding 0.1 →
        // NET kayıp. Win/loss kararı brüt PnL'e göre verilemezdi.
        let mut e = DcaEngine::new("p12", BotConfig::default(), Instrument::default()).unwrap();
        let acts = e.on_bar(&ctx(&bar(1_000, "100"), Fraction::from_decimal_str("100").unwrap()));
        let base_id = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, role: "BASE", .. } => Some(id.clone()),
                _ => None,
            })
            .expect("base order");
        let mut bf = fill(&base_id, "BASE", Side::Buy, OrderKind::Add, "100", "1", 1_000);
        bf.fee = Fraction::from_decimal_str("0.6").unwrap();
        e.on_fill(&bf);
        e.apply_funding(&Fraction::from_decimal_str("0.1").unwrap());
        e.request_close_all();
        let acts = e.on_bar(&ctx(&bar(1_001, "100.5"), Fraction::from_decimal_str("100.5").unwrap()));
        let (close_id, close_role) = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, role, .. } if matches!(*role, "MANUAL" | "TP") => Some((id.clone(), *role)),
                _ => None,
            })
            .expect("close order (MANUAL/TP)");
        let mut cf = fill(&close_id, close_role, Side::Sell, OrderKind::Reduce, "100.5", "1", 1_001);
        cf.fee = Fraction::from_decimal_str("0.6").unwrap();
        e.on_fill(&cf);
        assert_eq!(e.deals_closed, 1);
        assert_eq!(e.losses, 1, "net = +0.5 brüt − 1.2 komisyon − 0.1 funding = −0.8 → kayıp");
        assert_eq!(e.wins, 0);
        // Kümülatif realizasyon brüt olarak raporlanmaya devam eder (M-17).
        assert_eq!(e.snapshot().realized_gross, "0.50000000");
    }

    #[test]
    fn force_liquidate_carries_mirror_realized_to_cumulative() {
        let mut e = DcaEngine::new("m17l", BotConfig::default(), Instrument::default()).unwrap();
        let acts = e.on_bar(&ctx(&bar(1_000, "100"), Fraction::from_u64(100)));
        let base_id = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, role: "BASE", .. } => Some(id.clone()),
                _ => None,
            })
            .expect("base order");
        e.on_fill(&fill(&base_id, "BASE", Side::Buy, OrderKind::Add, "100", "1", 1_000));
        // Faz 19: flat'a indiren reduce artık Open fazında da deal'i kapatır
        // (resting TP dolumu) — force_liquidate'in taşıma sözleşmesi KISMİ
        // realizasyonla test edilir: 0.5 @ 110 → gross 5, pozisyon 0.5 kalır.
        e.on_fill(&fill("seed-reduce", "TP", Side::Sell, OrderKind::Reduce, "110", "0.5", 1_001));
        assert_eq!(e.mirror.realized_gross.to_string(), "5");
        assert_eq!(e.mirror.qty.to_half_even_display(8), "0.50000000");
        e.cum_realized_gross = Fraction::zero();
        e.force_liquidate();
        assert_eq!(e.snapshot().realized_gross, "5.00000000");
        assert_eq!(e.deals_closed, 1);
    }

    #[test]
    fn resting_tp_limit_replaces_and_fills_realistically() {
        // Faz 19 (P1-4 kalıcı): TP artık dinlenen reduce-only LIMIT'tir.
        // 1) Deal açılır açılmaz mark TP'ye değse bile PİYASA emri AÇILMAZ;
        //    TPL limit emri tahtaya yazılır.
        // 2) Safety dolumu WAC'ı değiştirir → eski TPL iptal + yeni TPL.
        // 3) Yüksek penetrasyonlu barda dolum MAKER rolüyle gelir, deal kapanır
        //    ve dinlenen TP kaydı düşülür.
        let mut e = DcaEngine::new("p114", BotConfig::default(), Instrument::default()).unwrap();
        let acts = e.on_bar(&ctx(&bar(1_000, "100"), Fraction::from_decimal_str("100").unwrap()));
        let base_id = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, role: "BASE", .. } => Some(id.clone()),
                _ => None,
            })
            .expect("base order");
        e.on_fill(&fill(&base_id, "BASE", Side::Buy, OrderKind::Add, "100", "1", 1_000));
        assert!(e.tp_price.is_some());

        // TP üstünde bir bar: piyasa emri YOK, TPL yerleşir (bir sonraki on_bar'da).
        let acts = e.on_bar(&ctx(&bar(1_001, "105"), Fraction::from_decimal_str("105").unwrap()));
        let tpl = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceLimit { id, price, role: "TP", .. } => Some((id.clone(), price.clone())),
                _ => None,
            })
            .expect("TPL reduce-only limit yerleşmeli");
        assert!(matches!(e.resting_tp, Some(RestingTp { .. })));
        let _ = tpl;

        // Safety dolumu WAC'ı düşürür → TP değişir → CancelResting + yeni TPL.
        e.on_fill(&fill("s1", "SAFETY", Side::Buy, OrderKind::Add, "95", "1", 1_002));
        let acts = e.on_bar(&ctx(&bar(1_003, "96"), Fraction::from_decimal_str("96").unwrap()));
        assert!(
            acts.iter().any(|a| matches!(a, Action::CancelResting { .. })),
            "WAC değişince eski TPL iptal edilmeli"
        );
        assert!(
            acts.iter().any(|a| matches!(a, Action::PlaceLimit { role: "TP", .. })),
            "yeni TPL yeniden yazılmalı"
        );

        // Yüksek TP'yi 1-tick aşıyor (Maker penetrasyon dolumu) → deal kapanır.
        let tp_now = e.tp_price.clone().expect("tp var");
        let touch = tp_now.add(&Fraction::from_decimal_str("0.01").unwrap_or_else(|_| Fraction::one())).unwrap_or_else(|_| tp_now.clone());
        let close_id = e
            .resting_tp
            .as_ref()
            .map(|rt| rt.order_id.clone())
            .expect("resting tp kayıtlı");
        let cf = fill(&close_id, "TP", Side::Sell, OrderKind::Reduce, &touch.to_half_even_display(8), "2", 1_004);
        let acts = e.on_fill(&cf);
        assert_eq!(e.phase, DcaPhase::Cooldown, "dinlenen TP dolumu deal'i kapatmalı");
        assert!(e.mirror.qty.is_zero());
        assert!(e.resting_tp.is_none(), "deal kapanınca TP kaydı düşülmeli");
        assert!(
            acts.iter().any(|a| matches!(a, Action::CancelResting { .. })),
            "kapanışta artık TPL iptal niyeti yayılmalı"
        );
        assert_eq!(e.wins, 1, "net pozitif deal kazanç");
    }

    #[test]
    fn funding_is_cumulative_across_deals() {
        let mut e = DcaEngine::new("m17f", BotConfig::default(), Instrument::default()).unwrap();
        e.apply_funding(&Fraction::from_decimal_str("1.5").unwrap());
        run_deal(&mut e, "100", "95", "1", 1_000);
        // Yeni deal açıldı; funding sıfırlanmamalı (M-17: kümülatif sözleşme).
        assert_eq!(e.snapshot().cumulative_funding, "1.50000000");
    }

    #[test]
    fn legacy_runtime_state_without_cum_field_deserializes() {
        let e = DcaEngine::new("m17s", BotConfig::default(), Instrument::default()).unwrap();
        let mut v = serde_json::to_value(&e).unwrap();
        v.as_object_mut().unwrap().remove("cum_realized_gross");
        let e2: DcaEngine = serde_json::from_value(v).unwrap();
        assert!(e2.cum_realized_gross.is_zero(), "eski persist kayıtları default 0 ile açılmalı");
    }

    /// M-18 regresyon: base notional step altına yuvarlanırsa (qty→0) motor
    /// her tick yeniden denemez — 30 bar cooldown'a girer, ilk denemede tek
    /// CRITICAL basar, sayaç her engeli sayar; backoff sonrası yeniden dener.
    #[test]
    fn qty_zero_abort_backs_off_instead_of_ticking_loop() {
        // step_size'ı kasıtlı büyüt (1 BTC): base_order_size 100 → qty 0 → abort.
        let mut inst = Instrument::default();
        inst.step_size = Fraction::from_decimal_str("1").unwrap();
        let mut e = DcaEngine::new("m18", BotConfig::default(), inst).unwrap();

        // 1. deneme: CRITICAL + cooldown (100 USDT base @ 1000 → qty 0.1 → 0).
        let acts = e.on_bar(&ctx(&bar(1_000, "1000"), Fraction::from_decimal_str("1000").unwrap()));
        assert_eq!(e.blocked_size_bars, 1);
        assert_eq!(e.phase, DcaPhase::Cooldown);
        assert_eq!(e.cooldown_remaining, ABORT_BACKOFF_BARS);
        assert_eq!(acts.iter().filter(|a| matches!(a, Action::Log { level: "CRITICAL", .. })).count(), 1);
        // deal_seq abort'ta geri alınır — gerçek denemeye harcanmaz.
        assert_eq!(e.deal_seq, 0);

        // 2..ABORT_BACKOFF_BARS. bar: cooldown bloklar, CRITICAL YOK, sayaç sabit.
        for i in 0..ABORT_BACKOFF_BARS {
            let acts = e.on_bar(&ctx(&bar(2_000 + i as i64, "100"), Fraction::from_u64(100)));
            assert!(acts.is_empty(), "cooldown bloğu sessiz olmalı (iter {i})");
            assert_eq!(e.blocked_size_bars, 1, "backoff sırasında sayaç artmamalı");
        }

        // Backoff bitti: yeniden deneme — 2. engel sayılır, CRITICAL BASILMAZ.
        let acts = e.on_bar(&ctx(&bar(9_999, "1000"), Fraction::from_decimal_str("1000").unwrap()));
        assert_eq!(e.blocked_size_bars, 2);
        assert_eq!(e.phase, DcaPhase::Cooldown);
        assert!(acts.iter().all(|a| !matches!(a, Action::Log { level: "CRITICAL", .. })), "tekrar eden engel CRITICAL spam üretmemeli");

        // İkinci backoff'u boşalt, sonra fiyat düşer (LONG için qty artar) → açılır.
        for i in 0..ABORT_BACKOFF_BARS {
            assert!(e.on_bar(&ctx(&bar(11_000 + i as i64, "100"), Fraction::from_u64(100))).is_empty());
        }
        let acts = e.on_bar(&ctx(&bar(20_000, "50"), Fraction::from_decimal_str("50").unwrap()));
        assert!(acts.iter().any(|a| matches!(a, Action::PlaceMarket { role: "BASE", .. })), "backoff sonrası geçerli boyutta deal açılmalı");
        assert_eq!(e.phase, DcaPhase::BasePending);
    }
}
