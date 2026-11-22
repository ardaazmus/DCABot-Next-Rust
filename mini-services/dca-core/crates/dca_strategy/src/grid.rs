//! Grid suite: Spot/Futures Grid (M15 meshes, fee breaker, offset fills, trailing)
//! and Infinity Grid (M17 constant base-value conservation).

use crate::{order_id, Action, BarContext, BotConfig, EngineSnapshot, StrategyKind};
use dca_math::fraction::Fraction;
use dca_math::m15_m20::{arithmetic_mesh, fee_breaker_min_step, geometric_mesh, infinity_sell_delta};
use dca_math::m04::PositionState;
use dca_math::Instrument;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GridPhase {
    Running,
    Stopping,
    Stopped,
}

/// Faz 3 (C-4): kalıcı runtime snapshot için serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GridEngine {
    pub bot_id: String,
    pub cfg: BotConfig,
    pub inst: Instrument,
    pub kind: StrategyKind, // SpotGrid | FuturesGrid | InfinityGrid
    pub phase: GridPhase,
    pub seq: u64,
    pub mesh: Vec<Fraction>,
    /// Per-level state: last placed side (Some(Buy)/Some(Sell)) or None idle
    pub level_armed: Vec<bool>,
    pub mirror: PositionState,
    pub realized_gross: Fraction,
    pub cumulative_fees: Fraction,
    pub deals_closed: usize,
    pub wins: usize,
    pub losses: usize,
    pub grid_profit: Fraction,
    /// True once the start condition passed and mesh orders were placed.
    pub gate_open: bool,
    /// Last composite signal-tree verdict (None = no tree configured).
    pub last_signal_met: Option<bool>,
    /// Webhook force path: next bootstrap bypasses all start conditions.
    pub force_start_pending: bool,
    /// order_id → mesh level for every GRID/GRID_CLOSE order this engine has
    /// placed (bootstrap + offset re-arms). Authoritative source for on_fill
    /// offset routing (`level_of`).
    pub order_level: HashMap<String, usize>,
    /// Task 18 N-2: bu motorun bugüne kadar yerleştirdiği TÜM GRID/GRID_CLOSE
    /// emir id'leri — grid_trail mesh kaydırmasında eski seviye emirlerini
    /// tek tek idempotent iptal etmek için kullanılır.
    pub placed_ids: Vec<String>,
    /// Task 16 — g_min fee-breaker ihlali uyarısı (Permissive modda grid YİNE DE
    /// kurulur; Enforce modda new() hata döndürür). None = ihlal yok.
    pub fee_breaker_violation: Option<String>,
}

impl GridEngine {
    pub fn new(bot_id: &str, cfg: BotConfig, inst: Instrument, _mark: &Fraction) -> Result<Self, String> {
        let kind = cfg.kind;
        if !matches!(kind, StrategyKind::SpotGrid | StrategyKind::FuturesGrid | StrategyKind::InfinityGrid) {
            return Err("GridEngine requires a grid kind".into());
        }
        if matches!(cfg.entry_trigger, crate::EntryTrigger::RsiBelow(_)) {
            return Err(
                "grid start condition: RSI_BELOW unsupported — use INSTANT / PRICE_BELOW / PRICE_ABOVE / DROP_FROM_REF / SCHEDULED".into(),
            );
        }
        cfg.entry_trigger.validate()?;
        if let Some(sc) = &cfg.entry_signal {
            dca_signals::validate_signal_node(&sc.root)?;
        }
        let lower = Fraction::from_decimal_str(cfg.grid_lower.as_deref().unwrap_or("0")).map_err(|e| e.to_string())?;
        let upper = Fraction::from_decimal_str(cfg.grid_upper.as_deref().unwrap_or("0")).map_err(|e| e.to_string())?;
        // Faz 21 (P0-11): kademe sayısı üst sınırı 500 — sınırsız mesh tek
        // istekle motoru düşürebiliyordu (API katmanı da 422 verir; savunma).
        let levels = cfg.grid_levels.unwrap_or(10).max(2).min(500);
        if lower.is_zero() || upper.is_zero() || lower >= upper {
            return Err("grid bounds invalid: need 0 < lower < upper".into());
        }
        let mesh = match cfg.grid_mode.as_deref() {
            Some("geometric") => geometric_mesh(&lower, &upper, levels).map_err(|e| e.to_string())?,
            _ => arithmetic_mesh(&lower, &upper, levels).map_err(|e| e.to_string())?,
        };
        // Tick-align every level
        let mesh: Vec<Fraction> = mesh
            .into_iter()
            .map(|p| dca_math::m02::tick_down(&p, &inst.tick_size).unwrap_or(p))
            .collect();
        for w in mesh.windows(2) {
            if w[1] <= w[0] {
                return Err("grid bounds or tick size invalid: two adjacent levels collapse to the same price tick".into());
            }
        }
        // M15 fee breaker — Task 16 (spec 2.2) + Faz 21 (N-10/P1-18): kontrol
        // ARTIK gerçek mesh üzerinden — EN DAR segment + asimetrik gerçek
        // ücretler (maker-in / taker-out). Eski kod ortalamayı
        // ((upper/lower−1)/levels) maker×maker ile ölçüyordu: aritmetik
        // mesh'te en dar üst segment Ortalamadan dar olabilir → üretimde
        // komisyona eren tur riski görünmezdi.
        // Enforce → kurulum reddedilir; PermissiveWithWarning → grid kurulur
        // ama risk bildirimi kaydedilir (spec: "Izgarayı kurmayı engelleme!").
        let fee_ok = dca_math::m15_m20::validate_grid_fee_breaker(&mesh, &inst.maker_fee_rate, &inst.taker_fee_rate)
            .map_err(|e| e.to_string())?;
        let mut fee_breaker_violation: Option<String> = None;
        if !fee_ok {
            let g_min = fee_breaker_min_step(&inst.maker_fee_rate, &inst.taker_fee_rate).map_err(|e| e.to_string())?;
            if cfg.risk_policy.grid_fee_breaker.is_permissive() {
                fee_breaker_violation = Some(format!(
                    "NEGATIVE_FEE_MARGIN: en dar ızgara adımı komisyon toplamından (g_min {}) dar; her turda net bakiyeniz komisyona erir (kullanıcı onayıyla kuruldu)",
                    g_min.to_half_even_display(6)
                ));
            } else {
                return Err(format!(
                    "fee-breaker: en dar mesh adımı <= g_min ({}); aralığı büyütün veya kademe sayısını düşürün",
                    g_min.to_half_even_display(6)
                ));
            }
        }
        let n = mesh.len();
        Ok(Self {
            bot_id: bot_id.to_string(),
            cfg,
            inst,
            kind,
            phase: GridPhase::Running,
            seq: 0,
            mesh,
            level_armed: vec![false; n],
            mirror: PositionState::new(1),
            realized_gross: Fraction::zero(),
            cumulative_fees: Fraction::zero(),
            deals_closed: 0,
            wins: 0,
            losses: 0,
            grid_profit: Fraction::zero(),
            gate_open: false,
            last_signal_met: None,
            force_start_pending: false,
            order_level: HashMap::new(),
            placed_ids: Vec::new(),
            fee_breaker_violation,
        })
    }

    /// Bir emri yerleştirme kit'ine kaydet (id → trail-cancel sweep kaynağı).
    fn track_placed(&mut self, id: &str) {
        self.placed_ids.push(id.to_string());
    }

    /// Task 18 N-2 — İz süren grid (grid_trail = true, yalnız Spot/Futures Grid).
    ///
    /// Kapanış fiyatı mesh aralığının DIŞINA çıktığında tüm mesh aralık genişliği
    /// kadar kaydırılır: eski resting emirler idempotent iptal edilir, harita
    /// temizlenir, motor yeni aralıkta yeniden bootstrap olur.
    ///
    /// Yarış durumu belgelenmiş sınırdır: iptal ile yarışan bir dolum `level_of`
    /// eşleşemez → mirror'a pozisyon olarak yansır ama çift-kapatma üretmez
    /// (on_fill bilinmeyen id'yi offset'siz sayar — aynı davranış).

    /// Mesh level of a placed grid order id (None = unknown/foreign id).
    pub fn level_of(&self, order_id: &str) -> Option<usize> {
        self.order_level.get(order_id).copied()
    }

    /// Task 20 — Pine çıkış sinyali artık mesh'i DURDURUR (eski sınır:
    /// "pine exit mesh'i kapatmaz" kaldırıldı). Tüm resting grid emirleri
    /// idempotent iptal edilir, id → seviye haritası temizlenir, kapı kapanır.
    /// Yeniden giriş: pine/bootstrap kapısı tekrar açıldığında bootstrap(mark)
    /// ile mesh yeniden kurulur (backtest'te bootstrap_done=false, canlıda
    /// rt.grid_bootstrapped=false sıfırlanır — çağıranın sorumluluğu).
    pub fn halt_mesh(&mut self) -> Vec<Action> {
        let mut acts = Vec::new();
        let count = self.placed_ids.len();
        for id in self.placed_ids.drain(..) {
            acts.push(Action::CancelResting { id });
        }
        self.order_level.clear();
        self.level_armed = vec![false; self.mesh.len()];
        self.gate_open = false;
        acts.push(Action::Log {
            level: "INFO",
            message: format!(
                "pine exit: grid mesh durduruldu — {count} resting emir iptal edildi, bootstrap kapısı yeni giriş sinyali bekliyor"
            ),
        });
        acts
    }

    /// Read-only view of the full id → level map (backtest reconciliation).
    pub fn order_levels(&self) -> &HashMap<String, usize> {
        &self.order_level
    }

    fn next_seq_id(&mut self) -> String {
        self.seq += 1;
        format!("{}-{}", self.bot_id, self.seq)
    }

    /// Initial placement: buys below mark; futures may also sell above mark.
    /// Spot grids must acquire inventory before placing sells, so their upper
    /// levels are armed only after a buy opener fills and creates a close.
    /// Marks the start-condition gate as open (called only when the trigger passes).
    pub fn bootstrap(&mut self, mark: &Fraction) -> Vec<Action> {
        self.gate_open = true;
        let mut acts = Vec::new();
        let notional_per_level = Fraction::from_decimal_str(&self.cfg.base_order_size).unwrap_or_else(|_| Fraction::from_u64(100));
        let plan: Vec<(usize, dca_math::Side, Fraction, Fraction)> = self
            .mesh
            .iter()
            .enumerate()
            .filter_map(|(idx, p)| {
                let is_buy = dca_math::Fraction::cmp(p, mark) == std::cmp::Ordering::Less;
                if !is_buy && self.kind == StrategyKind::SpotGrid {
                    return None;
                }
                let qty = dca_math::m02::qty_down(&notional_per_level.div(p).unwrap_or_else(|_| Fraction::zero()), &self.inst.step_size)
                    .unwrap_or_else(|_| Fraction::zero());
                if qty.is_zero() {
                    return None;
                }
                let side = if is_buy { dca_math::Side::Buy } else { dca_math::Side::Sell };
                Some((idx, side, p.clone(), qty))
            })
            .collect();
        for (idx, side, price, qty) in plan {
            self.level_armed[idx] = true;
            let id = order_id(&self.next_seq_id(), "GRID", (idx as u64) << 32);
            self.order_level.insert(id.clone(), idx);
            self.track_placed(&id);
            acts.push(Action::PlaceLimit { id, side, price, qty, role: "GRID" });
        }
        acts
    }

    pub fn on_bar(&mut self, ctx: &BarContext) -> Vec<Action> {
        let mut acts = Vec::new();
        self.last_signal_met = ctx.signal_ok;
        if matches!(self.kind, StrategyKind::SpotGrid | StrategyKind::FuturesGrid)
            && self.cfg.grid_trail
            && self.gate_open
        {
            if let Some(shift) = self.trail_shift(&ctx.mark) {
                acts.extend(self.apply_trail_shift(shift, &ctx.mark));
            }
        }
        if self.kind == StrategyKind::InfinityGrid {
            // M17: sell excess when price rises keeping base value constant
            let base_value = Fraction::from_decimal_str(&self.cfg.base_order_size).unwrap_or_else(|_| Fraction::from_u64(100));
            if let Some(avg) = &self.mirror.avg_price {
                let _ = avg;
                if !self.mirror.qty.is_zero() {
                    if let Ok(dq) = infinity_sell_delta(&base_value, &self.mirror.qty, &ctx.mark, &self.inst.step_size) {
                        if !dq.is_zero() {
                            acts.push(Action::PlaceMarket {
                                id: order_id(&self.next_seq_id(), "INF", 0),
                                side: dca_math::Side::Sell,
                                qty: dq,
                                role: "TP",
                                quote_notional: None,
                            });
                        }
                    }
                }
            }
        }
        acts
    }

    /// Trail kaydırma miktarı: kapanış üst sınırın üzerindeyse +width, alt
    /// sınırın altındaysa −width; aralık içi None (kaydırma yok).
    fn trail_shift(&self, mark: &Fraction) -> Option<Fraction> {
        let n = self.mesh.len();
        if n < 2 {
            return None;
        }
        let top = &self.mesh[n - 1];
        let bottom = &self.mesh[0];
        if mark >= top {
            top.sub(bottom).ok().filter(|w| !w.is_zero())
        } else if mark <= bottom {
            bottom.sub(top).ok().filter(|w| !w.is_zero())
        } else {
            None
        }
    }

    /// Mesh'i `shift` kadar kaydır, tüm eski emirleri iptal et ve yeni
    /// aralıkta yeniden bootstrap ol.
    fn apply_trail_shift(&mut self, shift: Fraction, mark: &Fraction) -> Vec<Action> {
        let mut acts = Vec::new();
        // 1) Eski resting emirleri tek tek idempotent iptal et (kernel pending
        //    retain no-op; LIVE tarafı 110001'i idempotent sayar).
        for id in self.placed_ids.drain(..) {
            acts.push(Action::CancelResting { id });
        }
        // 2) Harita ve seviye durumunu temizle — eski id → seviye eşleşmeleri
        //    artık geçersiz (yarış dolumları bilinmeyen-id yoluna düşer).
        self.order_level.clear();
        // 3) Mesh'i kaydır ve tick hizala.
        for p in self.mesh.iter_mut() {
            *p = p.add(&shift).unwrap_or_else(|_| p.clone());
            *p = dca_math::m02::tick_down(p, &self.inst.tick_size).unwrap_or_else(|_| p.clone());
        }
        self.level_armed = vec![false; self.mesh.len()];
        // 4) Yeni aralıkta bootstrap (mark'ın altına buy, üstüne sell).
        acts.push(Action::Log {
            level: "INFO",
            message: format!(
                "grid trail: fiyat aralık dışına kapandı — mesh kaydırıldı, yeni aralık {}–{}",
                self.mesh[0].to_half_even_display(8),
                self.mesh[self.mesh.len() - 1].to_half_even_display(8)
            ),
        });
        acts.extend(self.bootstrap(mark));
        acts
    }

    /// Offset rule (side-aware, pair-closing):
    ///   opener BUY fill at level j  → closer SELL (role GRID_CLOSE, Reduce) at j+1
    ///   opener SELL fill at level j → closer BUY  (role GRID_CLOSE, Reduce) at j-1
    ///   closer fill → re-arms the opener on the mirrored level and books the
    ///   grid step profit (qty-weighted) into grid_profit / deals_closed.
    pub fn on_fill(&mut self, fill: &dca_kernel::FillRecord, level_of: impl Fn(&str) -> Option<usize>) -> Vec<Action> {
        let mut acts = Vec::new();
        let is_add = fill.kind == dca_kernel::OrderKind::Add;
        let _ = self.mirror.apply_fill(&fill.fill_key, &fill.price, &fill.qty, is_add);
        self.cumulative_fees = self.cumulative_fees.add(&fill.fee).unwrap_or_else(|_| self.cumulative_fees.clone());
        let Some(idx) = level_of(&fill.order_id) else { return acts };
        if is_add {
            // Opener filled — place the paired closer (Reduce) at the offset level.
            let (close_idx, side) = if fill.side == dca_math::Side::Buy {
                (idx + 1, dca_math::Side::Sell)
            } else if idx > 0 {
                (idx - 1, dca_math::Side::Buy)
            } else {
                return acts;
            };
            if close_idx < self.mesh.len() {
                let p = self.mesh[close_idx].clone();
                self.level_armed[close_idx] = true;
                let id = order_id(&self.next_seq_id(), "GRID_CLOSE", (close_idx as u64) << 32);
                self.order_level.insert(id.clone(), close_idx);
                self.track_placed(&id);
                acts.push(Action::PlaceLimit { id, side, price: p, qty: fill.qty.clone(), role: "GRID_CLOSE" });
            }
        } else {
            // Closer filled — a full grid pair completed.
            let step = self.realized_pair(idx, fill.side, &fill.price, &fill.qty).unwrap_or_else(|_| Fraction::zero());
            self.grid_profit = self.grid_profit.add(&step).unwrap_or_else(|_| self.grid_profit.clone());
            if step.is_positive() {
                self.wins += 1;
            } else {
                self.losses += 1;
            }
            self.deals_closed += 1;
            // Re-arm the opener on the mirrored level.
            let (open_idx, side) = if fill.side == dca_math::Side::Sell {
                if idx == 0 {
                    return acts;
                }
                (idx - 1, dca_math::Side::Buy)
            } else if idx + 1 < self.mesh.len() {
                (idx + 1, dca_math::Side::Sell)
            } else {
                return acts;
            };
            let p = self.mesh[open_idx].clone();
            self.level_armed[open_idx] = true;
            let id = order_id(&self.next_seq_id(), "GRID", (open_idx as u64) << 32);
            self.order_level.insert(id.clone(), open_idx);
            self.track_placed(&id);
            acts.push(Action::PlaceLimit { id, side, price: p, qty: fill.qty.clone(), role: "GRID" });
        }
        acts
    }

    /// Qty-weighted grid step profit for a completed pair. A closer SELL at
    /// level idx closed the buy at idx-1; a closer BUY at idx closed the sell
    /// at idx+1.
    fn realized_pair(&self, idx: usize, side: dca_math::Side, price: &Fraction, qty: &Fraction) -> Result<Fraction, String> {
        let per_unit = match side {
            dca_math::Side::Sell => {
                if idx == 0 {
                    return Ok(Fraction::zero());
                }
                price.sub(&self.mesh[idx - 1]).map_err(|e| e.to_string())?
            }
            dca_math::Side::Buy => {
                if idx + 1 >= self.mesh.len() {
                    return Ok(Fraction::zero());
                }
                self.mesh[idx + 1].sub(price).map_err(|e| e.to_string())?
            }
        };
        per_unit.mul(qty).map_err(|e| e.to_string())
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        EngineSnapshot {
            phase: format!("{:?}", self.phase),
            deal_id: None,
            position_qty: self.mirror.qty.to_half_even_display(8),
            avg_price: self.mirror.avg_price.as_ref().map(|a| a.to_half_even_display(8)),
            next_safety_price: None,
            next_safety_level: None,
            tp_price: None,
            sl_price: None,
            trailing_state: None,
            trailing_peak: None,
            safety_triggered: 0,
            realized_gross: self.grid_profit.to_half_even_display(8),
            cumulative_funding: self.cumulative_fees.to_half_even_display(8),
            deals_closed: self.deals_closed,
            wins: self.wins,
            losses: self.losses,
            entry_trigger: self.cfg.entry_trigger.describe(),
            entry_trigger_met: self.gate_open,
            entry_signal_met: self.last_signal_met,
            hedge: None,
        }
    }
}

#[cfg(test)]
mod start_condition_tests {
    use super::*;
    use crate::EntryTrigger;
    use dca_math::Instrument;

    fn cfg_with(trig: EntryTrigger) -> BotConfig {
        let mut cfg = BotConfig::default();
        cfg.kind = StrategyKind::SpotGrid;
        cfg.grid_lower = Some("90".into());
        cfg.grid_upper = Some("110".into());
        cfg.grid_levels = Some(4);
        cfg.grid_mode = Some("arithmetic".into());
        cfg.entry_trigger = trig;
        cfg
    }

    #[test]
    fn grid_rejects_rsi_condition() {
        let r = GridEngine::new("g", cfg_with(EntryTrigger::RsiBelow(30)), Instrument::default(), &Fraction::from_u64(100));
        assert!(r.is_err());
    }

    #[test]
    fn grid_bootstrap_opens_gate_only_once_flagged() {
        let mut g = GridEngine::new("g", cfg_with(EntryTrigger::PriceBelow("95".into())), Instrument::default(), &Fraction::from_u64(100))
            .expect("grid engine");
        assert!(!g.gate_open);
        assert_eq!(g.snapshot().entry_trigger, "PRICE ≤ 95");
        let acts = g.bootstrap(&Fraction::from_decimal_str("94").unwrap());
        assert!(!acts.is_empty(), "mesh orders placed");
        assert!(g.gate_open);
        assert!(g.snapshot().entry_trigger_met);
    }

    #[test]
    fn spot_grid_bootstrap_never_sells_unowned_inventory() {
        let mut cfg = cfg_with(EntryTrigger::Instant);
        cfg.base_order_size = "100".into();
        let mut grid = GridEngine::new("spot-grid-test", cfg, Instrument::default(), &Fraction::from_u64(100)).unwrap();

        let actions = grid.bootstrap(&Fraction::from_u64(100));
        assert!(!actions.is_empty(), "spot grid should seed buy levels below mark");
        assert!(actions.iter().all(|action| matches!(action, Action::PlaceLimit { side: dca_math::Side::Buy, .. })));
        assert_eq!(grid.level_armed.iter().filter(|armed| **armed).count(), actions.len());
    }

    #[test]
    fn grid_accepts_price_condition() {
        let r = GridEngine::new("g", cfg_with(EntryTrigger::Scheduled { start_ms: 1 }), Instrument::default(), &Fraction::from_u64(100));
        assert!(r.is_ok());
    }

    /// Grid re-arm lifecycle: every placed id (bootstrap + offsets) resolves to
    /// a mesh level; an opener fill emits a registered GRID_CLOSE closer; the
    /// closer fill re-arms the opener and books grid_profit/deals_closed.
    #[test]
    fn grid_levels_resolve_and_offset_rearm_works() {
        let mut g = GridEngine::new("g", cfg_with(EntryTrigger::Instant), Instrument::default(), &Fraction::from_u64(100)).expect("grid");
        let acts = g.bootstrap(&Fraction::from_u64(100));
        assert!(!acts.is_empty(), "mesh orders placed");
        for a in &acts {
            if let Action::PlaceLimit { id, .. } = a {
                assert!(g.level_of(id).is_some(), "bootstrap id {id} must resolve to a level");
            }
        }
        let buy = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceLimit { id, side: dca_math::Side::Buy, price, qty, .. } => {
                    Some((id.clone(), price.clone(), qty.clone()))
                }
                _ => None,
            })
            .expect("bootstrap contains a buy opener");
        let opener_level = g.level_of(&buy.0).unwrap();

        // Opener BUY fill → paired GRID_CLOSE sell at level+1.
        let opener_fill = dca_kernel::FillRecord {
            fill_key: "f1".into(),
            order_id: buy.0.clone(),
            role: "GRID".into(),
            side: dca_math::Side::Buy,
            kind: dca_kernel::OrderKind::Add,
            price: buy.1.clone(),
            qty: buy.2.clone(),
            fee: Fraction::zero(),
            fee_role: dca_kernel::FeeRole::Maker,
            realism: dca_risk::FillRealism::RestingTouch,
            ts: 1,
        };
        let level = g.level_of(&buy.0);
        let offset_acts = g.on_fill(&opener_fill, move |_| level);
        assert_eq!(offset_acts.len(), 1, "buy opener fill must place exactly one closer");
        let (close_id, close_price, close_qty) = match &offset_acts[0] {
            Action::PlaceLimit { id, side: dca_math::Side::Sell, price, qty, role } => {
                assert_eq!(*role, "GRID_CLOSE");
                assert!(g.level_of(id).is_some(), "offset id must resolve");
                assert_eq!(g.level_of(id).unwrap(), opener_level + 1);
                (id.clone(), price.clone(), qty.clone())
            }
            other => panic!("expected PlaceLimit closer, got {other:?}"),
        };

        // Closer fill → re-armed opener at the original level + profit booked.
        let closer_fill = dca_kernel::FillRecord {
            fill_key: "f2".into(),
            order_id: close_id.clone(),
            role: "GRID_CLOSE".into(),
            side: dca_math::Side::Sell,
            kind: dca_kernel::OrderKind::Reduce,
            price: close_price,
            qty: close_qty,
            fee: Fraction::zero(),
            fee_role: dca_kernel::FeeRole::Maker,
            realism: dca_risk::FillRealism::RestingTouch,
            ts: 2,
        };
        assert_eq!(g.deals_closed, 0);
        let close_level = g.level_of(&close_id);
        let rearm_acts = g.on_fill(&closer_fill, move |_| close_level);
        assert_eq!(g.deals_closed, 1, "pair completion must close a deal");
        assert_eq!(g.wins, 1);
        assert!(!g.grid_profit.is_zero(), "grid step profit must be booked");
        assert_eq!(rearm_acts.len(), 1, "closer fill must re-arm the opener");
        match &rearm_acts[0] {
            Action::PlaceLimit { id, side: dca_math::Side::Buy, role, .. } => {
                assert_eq!(*role, "GRID");
                assert_eq!(g.level_of(id), Some(opener_level), "opener re-armed on its original level");
            }
            other => panic!("expected PlaceLimit re-arm, got {other:?}"),
        }
    }

    /// Task 18 N-2 — İz süren grid: kapanış aralığın üstünde → mesh bir genişlik
    /// yukarı kayar, eski emirler iptal edilir, yeni aralıkta bootstrap olur.
    #[test]
    fn grid_trail_shifts_mesh_up_on_breakout() {
        let mut cfg = cfg_with(EntryTrigger::Instant);
        cfg.grid_trail = true;
        let mut g = GridEngine::new("g", cfg, Instrument::default(), &Fraction::from_u64(100)).expect("grid");
        g.bootstrap(&Fraction::from_u64(100));
        let old_top = g.mesh[g.mesh.len() - 1].clone();
        let placed = g.placed_ids.len();
        assert!(placed > 0, "bootstrap must register placed ids");

        let bar = crate::Bar { t: 1, o: "120".into(), h: "121".into(), l: "119".into(), c: "120".into(), v: "10".into() };
        let ctx = BarContext {
            bar: &bar,
            open: Fraction::from_u64(120),
            high: Fraction::from_u64(121),
            low: Fraction::from_u64(119),
            close: Fraction::from_u64(120),
            position_qty: &Fraction::zero(),
            position_avg: None,
            cash: &Fraction::from_u64(10_000),
            mark: Fraction::from_u64(120),
            signal_ok: None,
            regime: None,
            deal_open_blocked: false,
        };
        let acts = g.on_bar(&ctx);

        // Cancel'lar: yerleştirilen her emir için biri + INFO log.
        let cancels = acts.iter().filter(|a| matches!(a, Action::CancelResting { .. })).count();
        assert_eq!(cancels, placed, "every placed id must be cancelled exactly once");
        assert!(acts.iter().any(|a| matches!(a, Action::Log { level: "INFO", .. })));

        // Mesh bir genişlik yukarı kaydı: yeni alt sınır eski üst sınıra ulaştı
        // (tick hizalaması drift'i ≤ 1 tick).
        let tick = Instrument::default().tick_size;
        let new_bottom = g.mesh[0].clone();
        assert!(
            new_bottom >= old_top,
            "mesh must move up: new bottom {new_bottom} >= old top {old_top}"
        );
        let drift = new_bottom.sub(&old_top).unwrap();
        assert!(drift <= tick, "tick-alignment drift must be within one tick");
        assert!(!g.order_level.is_empty(), "re-bootstrap must register new ids");
        assert!(g.level_armed.iter().any(|a| *a), "re-bootstrap must re-arm levels");
    }

    /// Trail KAPALIYKEN (default) aralık dışı kapanış mesh'i kaydırmaz — eski
    /// davranış korunur.
    #[test]
    fn grid_trail_disabled_keeps_mesh_stationary() {
        let mut g = GridEngine::new("g", cfg_with(EntryTrigger::Instant), Instrument::default(), &Fraction::from_u64(100)).expect("grid");
        g.bootstrap(&Fraction::from_u64(100));
        let before = g.mesh.clone();
        let bar = crate::Bar { t: 1, o: "120".into(), h: "121".into(), l: "119".into(), c: "120".into(), v: "10".into() };
        let ctx = BarContext {
            bar: &bar,
            open: Fraction::from_u64(120),
            high: Fraction::from_u64(121),
            low: Fraction::from_u64(119),
            close: Fraction::from_u64(120),
            position_qty: &Fraction::zero(),
            position_avg: None,
            cash: &Fraction::from_u64(10_000),
            mark: Fraction::from_u64(120),
            signal_ok: None,
            regime: None,
            deal_open_blocked: false,
        };
        let acts = g.on_bar(&ctx);
        assert!(acts.is_empty(), "no trail → on_bar must be inert");
        assert_eq!(g.mesh, before, "mesh must not move without grid_trail");
    }

    /// EngineTag dispatch (runtime path): on_fill routes grid fills through
    /// the engine's own level registry without an external map.
    #[test]
    fn engine_tag_on_fill_routes_grid_offsets() {
        let g = GridEngine::new("g", cfg_with(EntryTrigger::Instant), Instrument::default(), &Fraction::from_u64(100)).expect("grid");
        let mut tag = crate::EngineTag::Grid(g);
        let acts = match &mut tag {
            crate::EngineTag::Grid(g) => g.bootstrap(&Fraction::from_u64(100)),
            _ => unreachable!(),
        };
        let buy = acts
            .iter()
            .find_map(|a| match a {
                Action::PlaceLimit { id, side: dca_math::Side::Buy, price, qty, .. } => {
                    Some((id.clone(), price.clone(), qty.clone()))
                }
                _ => None,
            })
            .expect("bootstrap buy");
        let fill = dca_kernel::FillRecord {
            fill_key: "f1".into(),
            order_id: buy.0,
            role: "GRID".into(),
            side: dca_math::Side::Buy,
            kind: dca_kernel::OrderKind::Add,
            price: buy.1,
            qty: buy.2,
            fee: Fraction::zero(),
            fee_role: dca_kernel::FeeRole::Maker,
            realism: dca_risk::FillRealism::RestingTouch,
            ts: 1,
        };
        let offset_acts = tag.on_fill(&fill);
        assert_eq!(offset_acts.len(), 1, "EngineTag::on_fill must produce the grid closer");
        assert!(matches!(&offset_acts[0], Action::PlaceLimit { role: "GRID_CLOSE", .. }));
    }
}
