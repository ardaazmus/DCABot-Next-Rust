//! dca_strategy — Bot taxonomy engines (DCA Long/Short, Grid, Infinity, Hedge)
//! driving the single-event kernel. Strategies cannot distinguish simulation
//! from live except through explicit port injection (ADR-001).

// Faz 21 (CI): i8 yön sabitleri kasıtlı i8->i8 cast (sign sözleşmesi);
// BacktestResult varyant boyut farkı rapor payload'ından; certify'in
// next_f64/total_return alanları diagnostic API yüzeyi (şimdilik okunmuyor).
#![allow(
    clippy::unnecessary_cast,
    clippy::large_enum_variant,
    dead_code,
    // Faz 21 (CI): test kurulumlarında Default + alan-ataması okunabilirliği
    // (13 test builder'ı); aralık kontrolü açık `x >= a && x <= b` biçiminde
    // kalır (clamp'in max<min panik sözleşmesi devreye girmez); certify
    // sabitlerinde tam f64 hassasiyeti bilinçlidir.
    clippy::field_reassign_with_default,
    clippy::manual_range_contains,
    clippy::excessive_precision,
    clippy::empty_line_after_doc_comments,
    clippy::unnecessary_lazy_evaluations,
)]

pub mod advisory;
pub mod backtest;
pub mod certify;
pub mod dca;
pub mod grid;
pub mod hedge;

pub use advisory::{compute_warnings, grid_fee_breaker_violation};
pub use backtest::{run_backtest, BacktestMetrics, BacktestResult};
pub use certify::{certify, certify_verdict, CertGate, CertGateVerdict, CertifyOutput};
pub use dca::{DcaEngine, DcaPhase};
pub use grid::GridEngine;
pub use hedge::HedgeEngine;

use dca_math::fraction::Fraction;
use dca_math::m02::{Direction, SizingMode};
use dca_kernel::Bar;
use dca_signals::MarketRegime;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StrategyKind {
    DcaLong,
    DcaShort,
    FuturesDca,
    SpotGrid,
    FuturesGrid,
    InfinityGrid,
    HedgeProtective,
    HedgeDual,
}

impl StrategyKind {
    pub fn direction(&self) -> Option<Direction> {
        match self {
            StrategyKind::DcaLong | StrategyKind::FuturesDca => Some(Direction::Long),
            StrategyKind::DcaShort => Some(Direction::Short),
            _ => None,
        }
    }
    pub fn is_futures(&self) -> bool {
        matches!(self, StrategyKind::DcaShort | StrategyKind::FuturesDca | StrategyKind::FuturesGrid | StrategyKind::HedgeProtective | StrategyKind::HedgeDual)
    }
    pub fn label(&self) -> &'static str {
        match self {
            StrategyKind::DcaLong => "DCA_LONG",
            StrategyKind::DcaShort => "DCA_SHORT",
            StrategyKind::FuturesDca => "FUTURES_DCA",
            StrategyKind::SpotGrid => "SPOT_GRID",
            StrategyKind::FuturesGrid => "FUTURES_GRID",
            StrategyKind::InfinityGrid => "INFINITY_GRID",
            StrategyKind::HedgeProtective => "HEDGE_PROTECTIVE",
            StrategyKind::HedgeDual => "HEDGE_DUAL",
        }
    }
}

/// Bot start condition — when trading may begin (gates every new DCA deal and
/// the initial grid bootstrap). Wire format (serde SCREAMING_SNAKE_CASE):
/// "INSTANT" | {"RSI_BELOW": 30} | {"PRICE_BELOW": "80000"} |
/// {"PRICE_ABOVE": "85000"} | {"DROP_FROM_REF": {"ref_price": "90000", "drop_pct": "0.1"}} |
/// {"SCHEDULED": {"start_ms": 1735689600000}}. Legacy "Instant"/"RsiBelow"
/// strings remain accepted via aliases.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntryTrigger {
    #[serde(alias = "Instant")]
    #[default]
    Instant,
    #[serde(alias = "RsiBelow")]
    RsiBelow(u32), // e.g. RSI < 30
    PriceBelow(String),
    PriceAbove(String),
    DropFromRef { ref_price: String, drop_pct: String },
    Scheduled { start_ms: i64 },
}

impl EntryTrigger {
    /// Human-readable condition description (snapshots, UI, logs).
    pub fn describe(&self) -> String {
        match self {
            EntryTrigger::Instant => "INSTANT".into(),
            EntryTrigger::RsiBelow(t) => format!("RSI < {t}"),
            EntryTrigger::PriceBelow(p) => format!("PRICE ≤ {p}"),
            EntryTrigger::PriceAbove(p) => format!("PRICE ≥ {p}"),
            EntryTrigger::DropFromRef { ref_price, drop_pct } => {
                let d = Fraction::from_decimal_str(drop_pct)
                    .ok()
                    .and_then(|x| x.mul(&Fraction::from_u64(100)).ok())
                    .map(|y| y.to_half_even_display(2))
                    .unwrap_or_else(|| drop_pct.clone());
                format!("DROP ≥ {d}% from {ref_price}")
            }
            EntryTrigger::Scheduled { start_ms } => format!("SCHEDULED @ {start_ms}ms"),
        }
    }

    /// Structural validation — 422-worthy failures surface at bot creation.
    pub fn validate(&self) -> Result<(), String> {
        match self {
            EntryTrigger::Instant => Ok(()),
            EntryTrigger::RsiBelow(t) => {
                if (1..=99).contains(t) {
                    Ok(())
                } else {
                    Err("RSI_BELOW must be in [1, 99]".into())
                }
            }
            EntryTrigger::PriceBelow(p) | EntryTrigger::PriceAbove(p) => {
                let v = Fraction::from_decimal_str(p).map_err(|e| format!("entry trigger threshold '{p}': {e}"))?;
                if v.is_zero() || v.is_negative() {
                    Err("entry trigger price threshold must be > 0".into())
                } else {
                    Ok(())
                }
            }
            EntryTrigger::DropFromRef { ref_price, drop_pct } => {
                let r = Fraction::from_decimal_str(ref_price).map_err(|e| format!("ref_price '{ref_price}': {e}"))?;
                let d = Fraction::from_decimal_str(drop_pct).map_err(|e| format!("drop_pct '{drop_pct}': {e}"))?;
                if r.is_zero() || r.is_negative() {
                    Err("DROP_FROM_REF ref_price must be > 0".into())
                } else if d.is_zero() || d.is_negative() || d >= Fraction::from_u64(1) {
                    Err("DROP_FROM_REF drop_pct must be in (0, 1), e.g. 0.1 = 10%".into())
                } else {
                    Ok(())
                }
            }
            EntryTrigger::Scheduled { start_ms } => {
                if *start_ms > 0 {
                    Ok(())
                } else {
                    Err("SCHEDULED start_ms must be > 0 (epoch ms)".into())
                }
            }
        }
    }
}

/// Canonical bot configuration — all numerics as exact decimal strings (zero-float).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotConfig {
    pub kind: StrategyKind,
    pub symbol: String,
    pub base_order_size: String,
    pub safety_order_size: String,
    pub sizing_mode: SizingMode,
    pub deviation_pct: String,
    pub step_scale: String,
    pub volume_scale: String,
    pub max_safety_orders: usize,
    pub target_profit_pct: String,
    pub use_net_tp: bool,
    pub trailing_enabled: bool,
    pub trailing_trigger_pct: String,
    pub trailing_deviation_pct: String,
    pub stop_loss_pct: Option<String>,
    pub leverage: String,
    pub hedge_ratio: String,
    pub combined_target_profit: String,
    /// Task 21 — Hedge Protective koruma tetikleyici eşik (PERCENT string, "6" = %6).
    /// Task 18-b F-sentezinde tespit edilen hedge.rs:62 hardcode'unun config'e açılması.
    /// Yalnız HEDGE_PROTECTIVE spawn'ında kullanılır; serde default eski config'leri korur.
    #[serde(default = "default_hedge_dd_trigger")]
    pub hedge_drawdown_trigger_pct: String,
    pub grid_lower: Option<String>,
    pub grid_upper: Option<String>,
    pub grid_levels: Option<usize>,
    pub grid_mode: Option<String>,
    pub grid_trail: bool,
    pub cooldown_bars: usize,
    pub entry_trigger: EntryTrigger,
    /// 3Commas-style composite indicator condition tree (dca_signals).
    /// When present, the base order / grid bootstrap additionally requires the
    /// tree to evaluate TRUE on closed bars (zero look-ahead). Absent = ASAP/simple.
    #[serde(default)]
    pub entry_signal: Option<dca_signals::SignalConfig>,
    /// Task 19 — Pine Script (alt-küme) strateji kodu. Derlenir (dca_pine) ve
    /// kapalı barlar üzerinde değerlendirilir; giriş kararı signal_ok kanalına
    /// AND'lenir, çıkış kararı PINE_EXIT kapanışı üretir. Pine sinyali olan bot
    /// entry_trigger = INSTANT olmak zorundadır (Pine kendisi başlangıç koşuludur).
    #[serde(default)]
    pub pine_script: Option<String>,
    /// Task 19 — Pine input değerleri (değişken adı → string değer).
    #[serde(default)]
    pub pine_inputs: std::collections::BTreeMap<String, String>,
    /// Task 16 — Piyasa Korumaları ve Bilgilendirilmiş Risk Motoru.
    /// Absent in legacy configs → spec Default (advisory philosophy) applies.
    #[serde(default)]
    pub risk_policy: dca_kernel::UserRiskPolicy,
}

impl Default for BotConfig {
    fn default() -> Self {
        Self {
            kind: StrategyKind::DcaLong,
            symbol: "BTCUSDT".into(),
            base_order_size: "100".into(),
            safety_order_size: "100".into(),
            sizing_mode: SizingMode::QuoteNotional,
            deviation_pct: "0.02".into(),
            step_scale: "1".into(),
            volume_scale: "1".into(),
            max_safety_orders: 5,
            target_profit_pct: "0.02".into(),
            use_net_tp: true,
            trailing_enabled: false,
            trailing_trigger_pct: "0.01".into(),
            trailing_deviation_pct: "0.005".into(),
            stop_loss_pct: None,
            leverage: "1".into(),
            hedge_ratio: "0.5".into(),
            combined_target_profit: "50".into(),
            hedge_drawdown_trigger_pct: "6".into(),
            grid_lower: None,
            grid_upper: None,
            grid_levels: None,
            grid_mode: Some("arithmetic".into()),
            grid_trail: false,
            cooldown_bars: 0,
            entry_trigger: EntryTrigger::Instant,
            entry_signal: None,
            pine_script: None,
            pine_inputs: std::collections::BTreeMap::new(),
            risk_policy: dca_kernel::UserRiskPolicy::default(),
        }
    }
}

fn default_hedge_dd_trigger() -> String {
    "6".into()
}

impl BotConfig {
    /// Task 19 — Pine strateji yapısal doğrulaması (422-worthy).
    /// Derleme hatası ve entry_trigger uyumsuzluğu burada yakalanır.
    pub fn validate_pine(&self) -> Result<(), String> {
        if let Some(code) = &self.pine_script {
            if !matches!(self.entry_trigger, EntryTrigger::Instant) {
                return Err("pine_script ile birlikte entry_trigger yalnız INSTANT olabilir (Pine kendisi başlangıç koşuludur)".into());
            }
            dca_pine::validate_source(code).map_err(|e| format!("pine_script: {e}"))?;
        }
        Ok(())
    }
}

/// Actions emitted by engines per bar; the runner forwards them to the kernel
/// (backtest/paper) or the exchange adapter (live).
#[derive(Debug, Clone)]
pub enum Action {
    PlaceLimit { id: String, side: dca_math::Side, price: Fraction, qty: Fraction, role: &'static str },
    /// Faz 22 (P1-21): `quote_notional` — QuoteNotional market emrinde dolum
    /// anında qty GERÇEK dolum fiyatından çözülür (kernel); None = qty sabit.
    PlaceMarket { id: String, side: dca_math::Side, qty: Fraction, role: &'static str, quote_notional: Option<Fraction> },
    CancelResting { id: String },
    /// Faz 28 (P1-3 — bağımsız denetim): kapanan deal defteri kaydı —
    /// kalıcı `deals` tablosuna tek işlemde idempotent yazılır (decimal
    /// string sözleşmesi). DCA/Hedge kapanışları bunu üretir.
    DealClosed {
        deal_no: usize,
        opened_ts: i64,
        closed_ts: i64,
        gross_pnl: String,
        fees: String,
        funding: String,
        net_pnl: String,
        safety_triggered: usize,
        reason: String,
    },
    Log { level: &'static str, message: String },
}

#[derive(Debug, Clone, Serialize)]
pub struct EngineSnapshot {
    pub phase: String,
    pub deal_id: Option<String>,
    pub position_qty: String,
    pub avg_price: Option<String>,
    pub next_safety_price: Option<String>,
    pub next_safety_level: Option<usize>,
    pub tp_price: Option<String>,
    pub sl_price: Option<String>,
    pub trailing_state: Option<String>,
    pub trailing_peak: Option<String>,
    pub safety_triggered: usize,
    pub realized_gross: String,
    pub cumulative_funding: String,
    pub deals_closed: usize,
    pub wins: usize,
    pub losses: usize,
    /// Active start condition (human-readable, e.g. "PRICE ≤ 80000").
    pub entry_trigger: String,
    /// True once the start condition is satisfied (Instant ⇒ always true).
    pub entry_trigger_met: bool,
    /// Last composite signal-tree verdict (None = no entry_signal configured).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entry_signal_met: Option<bool>,
    /// Task 21 — Hedge motoru durumu (yalnız Hedge ailesinde Some; JSON'da alan yok sayılır).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hedge: Option<HedgeInfo>,
}

/// Task 21 — Hedge bacağı snapshot bilgisi (studio Hedge paneli veri kaynağı).
/// Tüm numerikler display string (zero-float sözleşmesi; "n/d" kesir gösterimi serbest).
#[derive(Debug, Clone, Serialize)]
pub struct HedgeLegInfo {
    pub id: String,
    pub phase: String,
    pub position_qty: String,
    pub avg_price: Option<String>,
    pub realized_gross: String,
    pub unrealized_gross: Option<String>,
    pub pnl: String,
    pub deals_closed: usize,
    pub wins: usize,
    pub losses: usize,
}

/// Task 21 — Hedge motoru durumu (EngineSnapshot.hedge). Sadece Hedge ailesi
/// botlarda bulunur (skip_serializing_if) — eski UI sözleşmesi bozulmaz.
#[derive(Debug, Clone, Serialize)]
pub struct HedgeInfo {
    /// "PROTECTIVE" | "DUAL"
    pub mode: String,
    /// "MainOnly" | "HedgeActive" | "Closing"
    pub phase: String,
    pub hedge_ratio: String,
    pub combined_target: String,
    /// Son işlenen bardaki birleşik k/z (main + hedge leg PnL; display string).
    pub combined_pnl: String,
    /// Koruma tetikleyici eşik (fraction display, örn. "0.06" = %6).
    pub drawdown_trigger: String,
    /// Son bardaki zirveden düşüş (fraction; protective modda doldurulur).
    pub drawdown_from_peak: Option<String>,
    /// Koruma referansı: ana bacağın koşu zirvesi (mark).
    pub peak_mark: Option<String>,
    pub spawn_count: usize,
    /// Hedge bacağı (protective: spawn olmadıysa None; dual: kuruluştan beri Some).
    pub hedge_leg: Option<HedgeLegInfo>,
    /// Motor içi olay günlüğü (spawn/exit/aktif) — son 10 kayıt, en yeni sonda.
    pub events: Vec<String>,
}

/// Position + account view injected by the runner each bar (single source of truth).
pub struct BarContext<'a> {
    pub bar: &'a Bar,
    pub open: Fraction,
    pub high: Fraction,
    pub low: Fraction,
    pub close: Fraction,
    pub position_qty: &'a Fraction,
    pub position_avg: Option<&'a Fraction>,
    pub cash: &'a Fraction,
    pub mark: Fraction,
    /// Composite signal-tree verdict for this bar (None = no tree configured).
    /// `Some(false)` fail-closes the base order / grid bootstrap (Gate 4).
    pub signal_ok: Option<bool>,
    /// Task 16 — piyasa rejimi (CRASH / HIGH_VOLATILITY / NORMAL) — runner
    /// kapalı barlardan sınıflandırır; None = değerlendirilemedi.
    pub regime: Option<MarketRegime>,
    /// Task 16 — günlük DD devre kesici tetiklendi: yeni deal açılışı bugün
    /// blokludur (mevcut pozisyonun TP/SL yönetimi etkilenmez).
    pub deal_open_blocked: bool,
}

/// EX-01: kısa, belirli ve benzersiz dış emir kimliği. Bybit orderLinkId
    /// sınırı 36, OKX clOrdId sınırı 32 karakter. `bot` zaten kısaltılmış
    /// (b- + 8 hex = 10 char) olduğunda {bot}-{role}-{seq} ≈ 15-22 karakter
    /// içinde kalır ve her iki borsanın da sınırına uygun olur.
pub fn order_id(bot: &str, role: &str, seq: u64) -> String {
    format!("{bot}-{role}-{seq}")
}

/// Exact RSI (zero-float) over closes using Wilder smoothing with fractions.
pub fn rsi_exact(closes: &[Fraction], period: usize) -> Option<Fraction> {
    if closes.len() < period + 1 {
        return None;
    }
    let mut gains = Vec::new();
    let mut losses = Vec::new();
    for w in closes.windows(2) {
        let d = w[1].sub(&w[0]).ok()?;
        if d.is_positive() {
            gains.push(d);
            losses.push(Fraction::zero());
        } else if d.is_negative() {
            gains.push(Fraction::zero());
            losses.push(d.neg().ok()?);
        } else {
            gains.push(Fraction::zero());
            losses.push(Fraction::zero());
        }
    }
    let sum = |v: &[Fraction]| -> Option<Fraction> {
        v.iter().try_fold(Fraction::zero(), |acc, x| acc.add(x)).ok()
    };
    let mut avg_gain = sum(&gains[..period])?.div(&Fraction::from_u64(period as u64)).ok()?;
    let mut avg_loss = sum(&losses[..period])?.div(&Fraction::from_u64(period as u64)).ok()?;
    for i in period..gains.len() {
        avg_gain = avg_gain
            .mul(&Fraction::from_u64((period - 1) as u64)).ok()?
            .add(&gains[i]).ok()?
            .div(&Fraction::from_u64(period as u64)).ok()?;
        avg_loss = avg_loss
            .mul(&Fraction::from_u64((period - 1) as u64)).ok()?
            .add(&losses[i]).ok()?
            .div(&Fraction::from_u64(period as u64)).ok()?;
    }
    if avg_loss.is_zero() {
        return Some(Fraction::from_u64(100));
    }
    let rs = avg_gain.div(&avg_loss).ok()?;
    let hundred = Fraction::from_u64(100);
    hundred.sub(&hundred.div(&Fraction::one().add(&rs).ok()?).ok()?).ok()
}

/// Evaluate a bot start condition (entry trigger) against a bar.
/// Zero-float: all price math via exact fractions.
/// `rsi` is supplied only by engines that maintain an RSI window (DCA family);
/// None ⇒ RSI conditions evaluate to false (window not warmed up).
pub fn evaluate_entry_trigger(
    trig: &EntryTrigger,
    close: &Fraction,
    bar_t: i64,
    rsi: Option<&Fraction>,
) -> Result<bool, String> {
    Ok(match trig {
        EntryTrigger::Instant => true,
        EntryTrigger::RsiBelow(t) => match rsi {
            Some(v) => Fraction::cmp(v, &Fraction::from_u64(*t as u64)) == std::cmp::Ordering::Less,
            None => false,
        },
        EntryTrigger::PriceBelow(p) => {
            let t = Fraction::from_decimal_str(p).map_err(|e| format!("PRICE_BELOW threshold: {e}"))?;
            Fraction::cmp(close, &t) != std::cmp::Ordering::Greater
        }
        EntryTrigger::PriceAbove(p) => {
            let t = Fraction::from_decimal_str(p).map_err(|e| format!("PRICE_ABOVE threshold: {e}"))?;
            Fraction::cmp(close, &t) != std::cmp::Ordering::Less
        }
        EntryTrigger::DropFromRef { ref_price, drop_pct } => {
            let r = Fraction::from_decimal_str(ref_price).map_err(|e| format!("DROP_FROM_REF ref_price: {e}"))?;
            let d = Fraction::from_decimal_str(drop_pct).map_err(|e| format!("DROP_FROM_REF drop_pct: {e}"))?;
            if r.is_zero() || r.is_negative() {
                return Err("DROP_FROM_REF ref_price must be > 0".into());
            }
            if d.is_zero() || d.is_negative() || d >= Fraction::from_u64(1) {
                return Err("DROP_FROM_REF drop_pct must be in (0, 1), e.g. 0.1 = 10%".into());
            }
            let trigger_price = r
                .mul(&Fraction::one().sub(&d).map_err(|e| e.to_string())?)
                .map_err(|e| e.to_string())?;
            Fraction::cmp(close, &trigger_price) != std::cmp::Ordering::Greater
        }
        EntryTrigger::Scheduled { start_ms } => bar_t >= *start_ms,
    })
}

/// Unified engine wrapper: DCA, Grid or Hedge family, tagged by kind.
/// HedgeEngine carries two full DcaEngine legs — boxed to keep the enum
/// variant size difference reasonable.
/// Faz 3 (C-4): kalıcı runtime snapshot için serde.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum EngineTag {
    Dca(DcaEngine),
    Grid(GridEngine),
    Hedge(Box<HedgeEngine>),
}

impl EngineTag {
    pub fn kind(&self) -> StrategyKind {
        match self {
            EngineTag::Dca(e) => e.cfg.kind,
            EngineTag::Grid(e) => e.kind,
            EngineTag::Hedge(e) => e.cfg.kind,
        }
    }

    pub fn on_fill(&mut self, fill: &dca_kernel::FillRecord) -> Vec<Action> {
        match self {
            EngineTag::Dca(e) => e.on_fill(fill),
            EngineTag::Grid(g) => {
                // Grid offset routing: the level must be resolved BEFORE the
                // &mut on_fill borrow starts (closure captures a copy).
                let level = g.level_of(&fill.order_id);
                g.on_fill(fill, move |_| level)
            }
            EngineTag::Hedge(h) => h.on_fill_leg(fill),
        }
    }

    pub fn on_bar(&mut self, ctx: &BarContext) -> Vec<Action> {
        match self {
            EngineTag::Dca(e) => e.on_bar(ctx),
            EngineTag::Grid(g) => g.on_bar(ctx),
            EngineTag::Hedge(h) => h.process_bar(ctx),
        }
    }

    pub fn snapshot(&self) -> EngineSnapshot {
        match self {
            EngineTag::Dca(e) => e.snapshot(),
            EngineTag::Grid(g) => g.snapshot(),
            EngineTag::Hedge(h) => h.snapshot(),
        }
    }
}
