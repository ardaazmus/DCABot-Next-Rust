//! Hedge Group engine (M18): Protective (main DCA + spawned short hedge) and
//! Dual DCA (simultaneous long+short with combined net exit `combinedExit`).
//!
//! Fill routing: every leg order id carries the leg prefix — "{bot}-main-…"
//! for the main leg, "{bot}-hedge-…" for the hedge leg — so the runner can
//! forward fills via `on_fill_leg` and each leg's own DCA machinery (safety
//! ladder, TP/SL, deal accounting) runs against its own mirror.

use crate::dca::DcaEngine;
use crate::{order_id, Action, BarContext, BotConfig, EngineSnapshot, HedgeInfo, HedgeLegInfo, StrategyKind};
use dca_kernel::{FillRecord, OrderKind};
use dca_math::fraction::Fraction;
use dca_math::m16_m18::{combined_exit_reached, hedge_qty};
use dca_math::Instrument;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HedgePhase {
    MainOnly,
    HedgeActive,
    Closing,
}

/// Faz 3 (C-4): kalıcı runtime snapshot için serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HedgeEngine {
    pub bot_id: String,
    pub cfg: BotConfig,
    pub main: DcaEngine,          // long leg (or short for inverted configs)
    pub hedge: Option<DcaEngine>, // opposite leg
    pub phase: HedgePhase,
    pub drawdown_trigger: Fraction, // e.g. 0.06 = -6% (config: hedge_drawdown_trigger_pct / 100)
    pub rho: Fraction,
    pub combined_target: Fraction,
    /// Running peak of the mark price (protective spawn reference).
    pub main_peak_mark: Option<Fraction>,
    /// Task 21 — protective spawn sayaç (dual kuruluşu da sayar; event cap'e takılmaz).
    pub spawn_count: usize,
    /// Task 21 — son işlenen bar (snapshot'ta unrealized PnL için mark kaynağı).
    pub last_mark: Option<Fraction>,
    /// Task 21 — son bardaki zirveden düşüş (protective; panel için saklanır).
    pub last_dd_from_peak: Option<Fraction>,
    /// Task 21 — son bardaki birleşik k/z (hedge bacağı varken hesaplanır).
    pub last_combined_pnl: Fraction,
    pub events: Vec<String>,
}

/// Task 21 — koruma tetikleyici parse: PERCENT string ("6" = %6) → fraction (0.06).
/// Çöp değer → dürüst default %6 (sessiz sapma yok: 422 kapısı create'te zaten var).
fn parse_dd_trigger_pct(pct: &str) -> Fraction {
    const DEFAULT: &str = "0.06";
    Fraction::from_decimal_str(pct)
        .ok()
        .filter(|f| !f.is_zero() && !f.is_negative())
        .and_then(|p| p.div(&Fraction::from_u64(100)).ok())
        .unwrap_or_else(|| Fraction::from_decimal_str(DEFAULT).unwrap_or_else(|_| Fraction::zero()))
}

impl HedgeEngine {
    pub fn new(bot_id: &str, cfg: BotConfig, inst: Instrument) -> Result<Self, String> {
        if !matches!(cfg.kind, StrategyKind::HedgeProtective | StrategyKind::HedgeDual) {
            return Err("HedgeEngine requires hedge kind".into());
        }
        let rho = Fraction::from_decimal_str(&cfg.hedge_ratio).map_err(|e| e.to_string())?;
        let combined = Fraction::from_decimal_str(&cfg.combined_target_profit).map_err(|e| e.to_string())?;
        // main leg = DCA LONG; hedge = DCA SHORT (protective). Dual: both from start.
        let mut main_cfg = cfg.clone();
        main_cfg.kind = StrategyKind::DcaLong;
        let mut hedge_cfg = cfg.clone();
        hedge_cfg.kind = StrategyKind::DcaShort;
        // Dual hedge leg is sized rho × main base (hedge_ratio semantics).
        if cfg.kind == StrategyKind::HedgeDual {
            let base = Fraction::from_decimal_str(&cfg.base_order_size).map_err(|e| e.to_string())?;
            let scaled = base.mul(&rho).map_err(|e| e.to_string())?;
            hedge_cfg.base_order_size = scaled.to_string();
        }
        let main = DcaEngine::new(&format!("{bot_id}-main"), main_cfg, inst.clone())?;
        let hedge = if cfg.kind == StrategyKind::HedgeDual {
            Some(DcaEngine::new(&format!("{bot_id}-hedge"), hedge_cfg, inst)?)
        } else {
            None
        };
        let dd = parse_dd_trigger_pct(&cfg.hedge_drawdown_trigger_pct);
        // Task 21 — dual modda hedge bacağı kuruluşta oluşur → spawn sayacı 1'den başlar.
        let spawn_count = if cfg.kind == StrategyKind::HedgeDual { 1 } else { 0 };
        Ok(Self {
            bot_id: bot_id.to_string(),
            cfg,
            main,
            hedge,
            phase: HedgePhase::MainOnly,
            drawdown_trigger: dd,
            rho,
            combined_target: combined,
            main_peak_mark: None,
            spawn_count,
            last_mark: None,
            last_dd_from_peak: None,
            last_combined_pnl: Fraction::zero(),
            events: Vec::new(),
        })
    }

    /// Task 21 — olay günlüğü cap'i (sonsuz büyüme yok; snapshot son 10'u taşır).
    fn push_event(&mut self, msg: String) {
        if self.events.len() >= 30 {
            self.events.remove(0);
        }
        self.events.push(msg);
    }

    /// Per-bar decision pass (EngineTag::Hedge dispatch):
    /// (1) main leg DCA machinery, (2) hedge leg DCA machinery (dual only),
    /// (3) protective spawn evaluation (mark drawdown from running peak),
    /// (4) M18 combined exit on both legs' current PnL.
    /// All leg actions are merged in order; the runner dispatches them.
    pub fn process_bar(&mut self, ctx: &BarContext) -> Vec<Action> {
        let mut acts = Vec::new();
        // Task 21 — panel snapshot'ı için mark saklanır (snapshot mark-temelli unrealized hesabı).
        self.last_mark = Some(ctx.mark.clone());
        // 1) main leg (DCA long) — its own gates / TP / SL / trailing.
        acts.extend(self.main.on_bar(ctx));
        // 2) hedge leg (DCA short) — dual runs from the first bar; protective
        //    only after the spawn below.
        if let Some(h) = self.hedge.as_mut() {
            acts.extend(h.on_bar(ctx));
        }
        // 3) protective spawn: price drawdown from the running peak crosses the
        //    trigger (zero-float fraction math).
        if self.cfg.kind == StrategyKind::HedgeProtective && self.hedge.is_none() && !ctx.mark.is_zero() {
            let peak = match &self.main_peak_mark {
                Some(p) if Fraction::cmp(p, &ctx.mark) == std::cmp::Ordering::Greater => p.clone(),
                _ => ctx.mark.clone(),
            };
            self.main_peak_mark = Some(peak.clone());
            let dd = peak
                .sub(&ctx.mark)
                .unwrap_or_else(|_| Fraction::zero())
                .div(&peak)
                .unwrap_or_else(|_| Fraction::zero());
            // Task 21 — panel için son düşüş saklanır (spawn öncesi takip).
            self.last_dd_from_peak = Some(dd.clone());
            if !dd.is_zero() {
                let qty = self.main.mirror.qty.clone();
                acts.extend(self.on_main_drawdown(&dd, &qty, &ctx.mark));
            }
        }
        // 4) combined exit (M18): leg PnL = mirror realized + unrealized at mark.
        if self.hedge.is_some() {
            let pnl_main = self.leg_pnl(&self.main, &ctx.mark);
            let pnl_hedge = self
                .hedge
                .as_ref()
                .map(|h| self.leg_pnl(h, &ctx.mark))
                .unwrap_or_else(Fraction::zero);
            // Task 21 — birleşik k/z panel için saklanır (son işlenen bar).
            self.last_combined_pnl = pnl_main.add(&pnl_hedge).unwrap_or_else(|_| pnl_main.clone());
            acts.extend(self.combined_exit_check(&pnl_main, &pnl_hedge));
        }
        acts
    }

    /// Current PnL of one leg: realized gross + unrealized gross at the mark
    /// (zero-float; both from the leg's own WAC mirror).
    fn leg_pnl(&self, leg: &DcaEngine, mark: &Fraction) -> Fraction {
        let realized = leg.mirror.realized_gross.clone();
        let unreal = leg.mirror.unrealized_gross(mark).unwrap_or_else(|_| Fraction::zero());
        realized.add(&unreal).unwrap_or(realized)
    }

    /// Protective spawn: when main leg drawdown from peak reaches the threshold
    /// a REAL short market order is dispatched (the hedge mirror fills when the
    /// fill arrives via on_fill_leg — no synthetic accounting).
    pub fn on_main_drawdown(&mut self, drawdown: &Fraction, main_qty: &Fraction, _mark: &Fraction) -> Vec<Action> {
        let mut acts = Vec::new();
        if self.cfg.kind != StrategyKind::HedgeProtective {
            return acts;
        }
        if self.phase != HedgePhase::MainOnly || self.hedge.is_some() {
            return acts;
        }
        if Fraction::cmp(drawdown, &self.drawdown_trigger) == std::cmp::Ordering::Less {
            return acts;
        }
        if main_qty.is_zero() {
            return acts; // nothing to protect yet
        }
        let Ok(hq) = hedge_qty(main_qty, &self.rho) else { return acts };
        if hq.is_zero() {
            return acts;
        }
        let mut hedge_cfg = self.cfg.clone();
        hedge_cfg.kind = StrategyKind::DcaShort;
        let mut h = DcaEngine::new(&format!("{}-hedge", self.bot_id), hedge_cfg, self.main.inst.clone())
            .expect("hedge engine");
        let seq = h.next_seq();
        let id = format!("{}-hedge-{}", self.bot_id, seq);
        h.phase = crate::dca::DcaPhase::BasePending;
        self.hedge = Some(h);
        self.phase = HedgePhase::HedgeActive;
        self.spawn_count += 1;
        self.push_event(format!(
            "HEDGE SPAWN: main dd=-{}%; short qty={} (rho={})",
            drawdown.to_half_even_display(4),
            hq.to_half_even_display(8),
            self.rho.to_half_even_display(4)
        ));
        acts.push(Action::PlaceMarket {
            id: order_id(&id, "BASE", 0),
            side: dca_math::Side::Sell,
            qty: hq,
            role: "BASE",
            quote_notional: None,
        });
        acts.push(Action::Log { level: "WARN", message: "hedge leg spawned (protective)".into() });
        acts
    }

    /// Combined exit check (M18): Pi_main + Pi_hedge >= T → close both legs.
    pub fn combined_exit_check(&mut self, pnl_main: &Fraction, pnl_hedge: &Fraction) -> Vec<Action> {
        let mut acts = Vec::new();
        // Faz 23 (derin inceleme): Closing artık ÖLÜM DEĞİL — her iki bacak
        // düzleşip deal döngüleri Cooldown'a düşünce faz geri açılır. Eski
        // davranışta Closing kalıcıydı: ikinci döngüde combined exit hiç
        // çalışmaz, bacaklar birleşik kâr hedefi OLMADAN işlemeye devam
        // ederdi (bot ömrü boyunca tek çıkış).
        if self.phase == HedgePhase::Closing {
            let main_flat = self.main.mirror.qty.is_zero() && self.main.phase == crate::dca::DcaPhase::Cooldown;
            let hedge_flat = match &self.hedge {
                Some(h) => h.mirror.qty.is_zero() && h.phase == crate::dca::DcaPhase::Cooldown,
                None => true,
            };
            if main_flat && hedge_flat {
                self.phase = if self.cfg.kind == StrategyKind::HedgeDual {
                    HedgePhase::HedgeActive
                } else {
                    HedgePhase::MainOnly
                };
                self.push_event("COMBINED EXIT complete — cycle re-armed".into());
            }
        }
        if self.phase != HedgePhase::HedgeActive {
            return acts;
        }
        if let Ok(reached) = combined_exit_reached(pnl_main, pnl_hedge, &self.combined_target) {
            if reached {
                self.main.request_close_all();
                if let Some(h) = &mut self.hedge {
                    h.request_close_all();
                }
                self.phase = HedgePhase::Closing;
                self.push_event("COMBINED EXIT triggered".into());
                acts.push(Action::Log { level: "INFO", message: "combined exit: closing both legs".into() });
            }
        }
        acts
    }

    /// Runner forwards every execution fill here; routed to the owning leg by
    /// the "{bot}-main" / "{bot}-hedge" order-id prefix (each leg keeps its own
    /// WAC mirror + deal accounting). The first hedge-leg base fill activates
    /// the hedge phase (dual: order placed at construction).
    pub fn on_fill_leg(&mut self, fill: &FillRecord) -> Vec<Action> {
        let hedge_prefix = format!("{}-hedge", self.bot_id);
        let main_prefix = format!("{}-main", self.bot_id);
        if fill.order_id.starts_with(&hedge_prefix) {
            let acts = match &mut self.hedge {
                Some(h) => h.on_fill(fill),
                None => Vec::new(),
            };
            if self.phase == HedgePhase::MainOnly && fill.kind == OrderKind::Add && fill.role == "BASE" {
                self.phase = HedgePhase::HedgeActive;
                self.push_event("HEDGE LEG ACTIVE (dual base filled)".into());
            }
            acts
        } else if fill.order_id.starts_with(&main_prefix) {
            self.main.on_fill(fill)
        } else {
            // Unknown/legacy id: default to the main leg.
            self.main.on_fill(fill)
        }
    }

    /// Task 21 — hedge bacağı panel bilgisi (studio Hedge sekmesi veri sözleşmesi).
    /// unrealized PnL SON İŞLENEN BARIN mark'ı ile hesaplanır (sokak ışığı:
    /// poll döngüsü bar sonrası çalışır; canlıda ~20s tazelik).
    /// M-17: realized panelde bot ömrü kümülatifi (deals_closed ile aynı pencere);
    /// M18 combined exit matematiği leg_pnl üzerinden deal-bazlı mirror'la devam eder.
    fn leg_info(&self, id: &str, leg: &DcaEngine) -> HedgeLegInfo {
        let realized = leg.cum_realized_gross.clone();
        let unreal = self
            .last_mark
            .as_ref()
            .and_then(|m| leg.mirror.unrealized_gross(m).ok());
        let pnl = unreal
            .as_ref()
            .and_then(|u| realized.add(u).ok())
            .unwrap_or_else(|| realized.clone());
        HedgeLegInfo {
            id: id.to_string(),
            phase: format!("{:?}", leg.phase),
            position_qty: leg.mirror.qty.to_half_even_display(8),
            avg_price: leg.mirror.avg_price.as_ref().map(|a| a.to_half_even_display(8)),
            realized_gross: realized.to_half_even_display(8),
            unrealized_gross: unreal.as_ref().map(|u| u.to_half_even_display(8)),
            pnl: pnl.to_half_even_display(8),
            deals_closed: leg.deals_closed,
            wins: leg.wins,
            losses: leg.losses,
        }
    }

    fn hedge_info(&self) -> HedgeInfo {
        HedgeInfo {
            mode: if self.cfg.kind == StrategyKind::HedgeDual { "DUAL".into() } else { "PROTECTIVE".into() },
            phase: format!("{:?}", self.phase),
            hedge_ratio: self.rho.to_half_even_display(4),
            combined_target: self.combined_target.to_half_even_display(8),
            combined_pnl: self.last_combined_pnl.to_half_even_display(8),
            drawdown_trigger: self.drawdown_trigger.to_half_even_display(6),
            drawdown_from_peak: self.last_dd_from_peak.as_ref().map(|d| d.to_half_even_display(6)),
            peak_mark: self.main_peak_mark.as_ref().map(|p| p.to_half_even_display(8)),
            spawn_count: self.spawn_count,
            hedge_leg: self.hedge.as_ref().map(|h| self.leg_info(&format!("{}-hedge", self.bot_id), h)),
            events: self.events.iter().rev().take(10).rev().cloned().collect(),
        }
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        let mut s = self.main.snapshot();
        s.phase = format!("{:?}/{}", self.phase, s.phase);
        s.hedge = Some(self.hedge_info());
        s
    }
}

#[cfg(test)]
mod hedge_tests {
    use super::*;
    use dca_kernel::Bar;
    use dca_math::Instrument;

    fn hedge_cfg(kind: StrategyKind, rho: &str, combined: &str) -> BotConfig {
        let mut cfg = BotConfig::default();
        cfg.kind = kind;
        cfg.hedge_ratio = rho.into();
        cfg.combined_target_profit = combined.into();
        // Keep per-leg TP far away so only the combined exit can close legs.
        cfg.target_profit_pct = "100".into();
        cfg
    }

    fn bar(t: i64, c: &str) -> Bar {
        Bar { t, o: c.into(), h: c.into(), l: c.into(), c: c.into(), v: "100000".into() }
    }

    fn ctx<'a>(bar: &'a Bar, mark: Fraction) -> BarContext<'a> {
        let qty: &'static Fraction = Box::leak(Box::new(Fraction::zero()));
        let cash: &'static Fraction = Box::leak(Box::new(Fraction::from_u64(1_000_000)));
        BarContext {
            bar,
            open: mark.clone(),
            high: mark.clone(),
            low: mark.clone(),
            close: mark.clone(),
            position_qty: qty,
            position_avg: None,
            cash,
            mark,
            signal_ok: None,
            regime: None,
            deal_open_blocked: false,
        }
    }

    fn fill(order_id: &str, role: &str, side: dca_math::Side, kind: OrderKind, price: &str, qty: &str) -> FillRecord {
        FillRecord {
            fill_key: format!("{order_id}-f"),
            order_id: order_id.into(),
            role: role.into(),
            side,
            kind,
            price: Fraction::from_any_str(price).unwrap(),
            qty: Fraction::from_any_str(qty).unwrap(),
            fee: Fraction::zero(),
            fee_role: dca_kernel::FeeRole::Taker,
            realism: dca_risk::FillRealism::MarketableTaker,
            ts: 1,
        }
    }

    fn base_order_id(acts: &[Action], side: dca_math::Side) -> String {
        acts.iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, side: s, role: "BASE", .. } if *s == side => Some(id.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {side:?} BASE order in {acts:?}"))
    }

    /// (1) HEDGE_DUAL: the first bar dispatches BOTH legs' base orders (long+short).
    #[test]
    fn hedge_dual_first_bar_opens_both_legs() {
        let mut e = HedgeEngine::new("hb", hedge_cfg(StrategyKind::HedgeDual, "0.5", "1000000"), Instrument::default())
            .expect("hedge engine");
        let b = bar(1, "100");
        let acts = e.process_bar(&ctx(&b, Fraction::from_u64(100)));
        let longs = acts.iter().filter(|a| matches!(a, Action::PlaceMarket { side: dca_math::Side::Buy, role: "BASE", .. })).count();
        let shorts = acts.iter().filter(|a| matches!(a, Action::PlaceMarket { side: dca_math::Side::Sell, role: "BASE", .. })).count();
        assert_eq!(longs, 1, "main long base");
        assert_eq!(shorts, 1, "hedge short base");
        // Hedge leg is rho-sized: 0.5 × 100 / 100 = 0.5
        let short_qty = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { side: dca_math::Side::Sell, qty, role: "BASE", .. } => Some(qty.to_string()),
                _ => None,
            })
            .expect("short qty");
        assert_eq!(short_qty, "1/2", "dual hedge leg sized rho × base (zero-float)");
        // Fill both bases → phase activates.
        let long_id = base_order_id(&acts, dca_math::Side::Buy);
        let short_id = base_order_id(&acts, dca_math::Side::Sell);
        e.on_fill_leg(&fill(&long_id, "BASE", dca_math::Side::Buy, OrderKind::Add, "100", "1"));
        e.on_fill_leg(&fill(&short_id, "BASE", dca_math::Side::Sell, OrderKind::Add, "100", "1/2"));
        assert_eq!(e.phase, HedgePhase::HedgeActive);
    }

    /// (2) HEDGE_PROTECTIVE: -6%+ drawdown from peak spawns the hedge leg with
    /// a REAL short market order and a HEDGE SPAWN event.
    #[test]
    fn hedge_protective_spawns_on_drawdown() {
        let mut e = HedgeEngine::new("hp", hedge_cfg(StrategyKind::HedgeProtective, "1", "1000000"), Instrument::default())
            .expect("hedge engine");
        assert!(e.snapshot().phase.contains("MainOnly"), "pre-spawn phase must be MainOnly");
        // Bar 1: open the main long.
        let b1 = bar(1, "100");
        let acts = e.process_bar(&ctx(&b1, Fraction::from_u64(100)));
        let long_id = base_order_id(&acts, dca_math::Side::Buy);
        e.on_fill_leg(&fill(&long_id, "BASE", dca_math::Side::Buy, OrderKind::Add, "100", "1"));
        // Bar 2: mild dip (-5%) — below trigger, no spawn.
        let b2 = bar(2, "95");
        let acts2 = e.process_bar(&ctx(&b2, Fraction::from_decimal_str("95").unwrap()));
        assert!(e.hedge.is_none(), "5% dd must not spawn");
        assert!(acts2.iter().all(|a| !matches!(a, Action::PlaceMarket { role: "BASE", side: dca_math::Side::Sell, .. })));
        // Bar 3: -7% from the peak (100) → spawn.
        let b3 = bar(3, "93");
        let acts3 = e.process_bar(&ctx(&b3, Fraction::from_decimal_str("93").unwrap()));
        assert!(e.hedge.is_some(), "7% dd must spawn the hedge leg");
        assert_eq!(e.phase, HedgePhase::HedgeActive);
        assert!(e.events.iter().any(|ev| ev.contains("HEDGE SPAWN")), "HEDGE SPAWN event recorded");
        assert!(
            acts3.iter().any(|a| matches!(a, Action::PlaceMarket { side: dca_math::Side::Sell, role: "BASE", qty, .. } if *qty == Fraction::from_u64(1))),
            "spawn must dispatch a real short market order, got {acts3:?}"
        );
        // The spawn order id must route to the hedge leg.
        let hedge_base_id = acts3
            .iter()
            .find_map(|a| match a {
                Action::PlaceMarket { id, side: dca_math::Side::Sell, role: "BASE", .. } => Some(id.clone()),
                _ => None,
            })
            .expect("hedge base id");
        assert!(hedge_base_id.starts_with("hp-hedge"), "hedge order id must carry the leg prefix");
        e.on_fill_leg(&fill(&hedge_base_id, "BASE", dca_math::Side::Sell, OrderKind::Add, "93", "1"));
        assert!(e.snapshot().phase.contains("HedgeActive"));
    }

    /// (3) Combined exit: when main + hedge PnL crosses the combined target,
    /// both legs request a close on the next pass.
    #[test]
    fn hedge_combined_exit_closes_both_legs() {
        // rho 0.5 → hedge short 0.5 vs main long 1: mark 120 ⇒ +20 −10 = +10 ≥ 5.
        let mut e = HedgeEngine::new("hx", hedge_cfg(StrategyKind::HedgeDual, "0.5", "5"), Instrument::default())
            .expect("hedge engine");
        let b = bar(1, "100");
        let acts = e.process_bar(&ctx(&b, Fraction::from_u64(100)));
        let long_id = base_order_id(&acts, dca_math::Side::Buy);
        let short_id = base_order_id(&acts, dca_math::Side::Sell);
        e.on_fill_leg(&fill(&long_id, "BASE", dca_math::Side::Buy, OrderKind::Add, "100", "1"));
        e.on_fill_leg(&fill(&short_id, "BASE", dca_math::Side::Sell, OrderKind::Add, "100", "1/2"));
        // Mark 90: main −10, hedge +5 → −5 < 5: no exit.
        let b2 = bar(2, "90");
        let _ = e.process_bar(&ctx(&b2, Fraction::from_decimal_str("90").unwrap()));
        assert_eq!(e.phase, HedgePhase::HedgeActive, "no premature combined exit");
        // Mark 120: +20 −10 = +10 ≥ 5 → combined exit fires.
        let b3 = bar(3, "120");
        let acts3 = e.process_bar(&ctx(&b3, Fraction::from_decimal_str("120").unwrap()));
        assert_eq!(e.phase, HedgePhase::Closing);
        assert!(e.events.iter().any(|ev| ev.contains("COMBINED EXIT")));
        assert!(acts3.iter().any(|a| matches!(a, Action::Log { message, .. } if message.contains("combined exit"))));
        // Next pass: both legs emit their MANUAL close market orders.
        let b4 = bar(4, "120");
        let acts4 = e.process_bar(&ctx(&b4, Fraction::from_decimal_str("120").unwrap()));
        let closes: Vec<&Action> = acts4.iter().filter(|a| matches!(a, Action::PlaceMarket { role: "MANUAL", .. })).collect();
        assert_eq!(closes.len(), 2, "both legs must emit close orders, got {acts4:?}");
        let has_buy_close = acts4.iter().any(|a| matches!(a, Action::PlaceMarket { side: dca_math::Side::Buy, role: "MANUAL", .. }));
        let has_sell_close = acts4.iter().any(|a| matches!(a, Action::PlaceMarket { side: dca_math::Side::Sell, role: "MANUAL", .. }));
        assert!(has_buy_close && has_sell_close, "long leg sells, short leg buys");
    }

    /// (4) Fill routing: legs only react to their own prefixed fills; unknown
    /// / kernel-generated ids (e.g. LIQ) default to the main leg.
    #[test]
    fn hedge_fill_routing_by_prefix() {
        let mut e = HedgeEngine::new("hr", hedge_cfg(StrategyKind::HedgeDual, "0.5", "1000000"), Instrument::default())
            .expect("hedge engine");
        // Unknown prefix → main leg (documented default routing).
        e.on_fill_leg(&fill("hr-unknown-1", "BASE", dca_math::Side::Buy, OrderKind::Add, "100", "1"));
        assert_eq!(e.main.mirror.qty.to_string(), "1");
        // Main-prefixed fill reaches the main leg.
        e.on_fill_leg(&fill("hr-main-2", "BASE", dca_math::Side::Buy, OrderKind::Add, "100", "1"));
        assert_eq!(e.main.mirror.qty.to_string(), "2");
        // Hedge-prefixed fill opens the hedge leg and activates the phase.
        e.on_fill_leg(&fill("hr-hedge-1", "BASE", dca_math::Side::Sell, OrderKind::Add, "100", "1/2"));
        let h = e.hedge.as_ref().expect("hedge leg");
        assert_eq!(h.mirror.qty.to_string(), "1/2");
        assert_eq!(e.phase, HedgePhase::HedgeActive);
    }

    /// (5) Task 21 — snapshot hedge bacakları + birleşik k/z + spawn sayacını
    /// DÜRÜST taşımalı (eski davranış: hedge bacağı telin üzerinde yoktu).
    #[test]
    fn hedge_snapshot_exposes_legs_and_combined_pnl() {
        let mut e = HedgeEngine::new("hs", hedge_cfg(StrategyKind::HedgeDual, "0.5", "5"), Instrument::default())
            .expect("hedge engine");
        let b = bar(1, "100");
        let acts = e.process_bar(&ctx(&b, Fraction::from_u64(100)));
        let long_id = base_order_id(&acts, dca_math::Side::Buy);
        let short_id = base_order_id(&acts, dca_math::Side::Sell);
        e.on_fill_leg(&fill(&long_id, "BASE", dca_math::Side::Buy, OrderKind::Add, "100", "1"));
        e.on_fill_leg(&fill(&short_id, "BASE", dca_math::Side::Sell, OrderKind::Add, "100", "1/2"));
        // Mark 90: main −10, hedge +5 → birleşik −5.
        let b2 = bar(2, "90");
        let _ = e.process_bar(&ctx(&b2, Fraction::from_decimal_str("90").unwrap()));
        let snap = e.snapshot();
        let h = snap.hedge.as_ref().expect("hedge info must ride the snapshot");
        assert_eq!(h.mode, "DUAL");
        assert_eq!(h.phase, "HedgeActive");
        assert_eq!(h.spawn_count, 1, "dual construction counts as the initial spawn");
        assert_eq!(h.hedge_ratio, "0.5000");
        let leg = h.hedge_leg.as_ref().expect("hedge leg must be exposed");
        assert_eq!(leg.position_qty, "0.50000000");
        assert_eq!(leg.avg_price.as_deref(), Some("100.00000000"));
        // main leg: −10 (unrealized) → pnl "-10.00000000"; hedge: +5.
        let expected_combined = Fraction::from_decimal_str("-5").unwrap();
        let got = Fraction::from_any_str(&h.combined_pnl).expect("combined pnl parseable");
        assert_eq!(got, expected_combined, "combined pnl = main(-10) + hedge(+5)");
        assert_eq!(h.combined_target, "5.00000000");
        // Tetikleyici default %6 → 0.06 fraction gösterimi.
        assert_eq!(h.drawdown_trigger, "0.060000");
        assert!(h.hedge_leg.as_ref().unwrap().pnl.starts_with("5"), "hedge leg pnl +5, got {}", h.hedge_leg.as_ref().unwrap().pnl);
        assert!(snap.phase.contains("HedgeActive"), "phase prefix korunmalı");
    }

    /// (6) Task 21 — drawdown tetikleyici artık config'ten gelir: "3" → %3,
    /// çöp → default %6, "0"/negatif → default %6 (kapalı-açık tuzağı yok).
    #[test]
    fn hedge_drawdown_trigger_from_config() {
        let mut cfg = hedge_cfg(StrategyKind::HedgeProtective, "1", "1000000");
        cfg.hedge_drawdown_trigger_pct = "3".into();
        let e = HedgeEngine::new("ht", cfg, Instrument::default()).expect("engine");
        assert_eq!(e.drawdown_trigger.to_string(), "3/100");
        // Çöp → default 6/100.
        let mut bad = hedge_cfg(StrategyKind::HedgeProtective, "1", "1000000");
        bad.hedge_drawdown_trigger_pct = "abc".into();
        let e2 = HedgeEngine::new("ht2", bad, Instrument::default()).expect("engine");
        assert_eq!(e2.drawdown_trigger.to_string(), "3/50");
        // %3 tetikleyiciyle −3% düşüş spawn ETMELİ (default %6 ile etmezdi).
        let mut cfg3 = hedge_cfg(StrategyKind::HedgeProtective, "1", "1000000");
        cfg3.hedge_drawdown_trigger_pct = "3".into();
        let mut e3 = HedgeEngine::new("ht3", cfg3, Instrument::default()).expect("engine");
        let b1 = bar(1, "100");
        let acts1 = e3.process_bar(&ctx(&b1, Fraction::from_u64(100)));
        let long_id = base_order_id(&acts1, dca_math::Side::Buy);
        e3.on_fill_leg(&fill(&long_id, "BASE", dca_math::Side::Buy, OrderKind::Add, "100", "1"));
        let b2 = bar(2, "97"); // tam −3%
        let acts2 = e3.process_bar(&ctx(&b2, Fraction::from_decimal_str("97").unwrap()));
        assert!(e3.hedge.is_some(), "−3% dd must spawn with trigger=3%");
        assert_eq!(e3.spawn_count, 1);
        assert!(e3.snapshot().hedge.as_ref().unwrap().drawdown_from_peak.is_some(), "dd_from_peak panel için taşınır");
        assert!(
            acts2.iter().any(|a| matches!(a, Action::PlaceMarket { side: dca_math::Side::Sell, role: "BASE", .. })),
            "spawn short market order"
        );
        // Protective + spawn öncesi: hedge_leg None dönmeli (dürüst boş durum).
        let mut cfg4 = hedge_cfg(StrategyKind::HedgeProtective, "1", "1000000");
        cfg4.hedge_drawdown_trigger_pct = "6".into();
        let mut e4 = HedgeEngine::new("ht4", cfg4, Instrument::default()).expect("engine");
        let bb1 = bar(1, "100");
        let _ = e4.process_bar(&ctx(&bb1, Fraction::from_u64(100)));
        let h4 = e4.snapshot().hedge.expect("hedge info");
        assert_eq!(h4.mode, "PROTECTIVE");
        assert!(h4.hedge_leg.is_none(), "spawn öncesi hedge bacağı yok");
        assert_eq!(h4.spawn_count, 0);
    }
}
