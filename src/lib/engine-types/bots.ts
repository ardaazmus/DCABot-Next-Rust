/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */
import type { BotKind, BotStatus, EntryTrigger, Interval, Mode, Venue } from "./core";
import type { AdvisoryWarning, UserRiskPolicy } from "./risk-policy";
import type { SignalConfig } from "./signals";

export interface Health {
  status: string;
  service: string;
  version: string;
  math: string;
  gates: number;
  venues: string[];
  uptime_s: number;
  /** Faz 25 (UIX-28): motorun readiness'ından filo durumu (aktif/toplam). */  readiness?: {
    activeBots: number;
    totalBots: number;
    db: string;
    lastTickAgeS: number;
    lastTickMs: number;
    note?: string;
  } | null;
}

export interface Allocation {
  symbol: string;
  value: string;
  pct: number;
}

export interface Summary {
  totalEquity: string;
  totalRealizedGross: string;
  /** Vault-wide open-position mark (engine /api/summary — absent on older snapshots). */
  totalUnrealizedPnl?: string;
  totalFees: string;
  activeBots: number;
  totalBots: number;
  activeDeals: number;
  longExposure: string;
  shortExposure: string;
  allocation: Allocation[];
  engineUptimeS: number;
}

export interface BotStats {
  phase: string;
  deal_id: string | null;
  position_qty: string;
  avg_price: string;
  next_safety_price: string | null;
  next_safety_level: number | null;
  tp_price: string | null;
  sl_price: string | null;
  trailing_state: string;
  trailing_peak: string | null;
  safety_triggered: number;
  realized_gross: string;
  /** Faz 24 (UI/UX Pro Max §3.1.2): net gerçekleşen (gross − kapanış fees − funding). */
  realized_net?: string;
  cumulative_funding: string;
  deals_closed: number;
  wins: number;
  losses: number;
  /** Human-readable start condition, e.g. "PRICE ≤ 80000". */
  entry_trigger: string;
  /** True once the start condition is satisfied. */
  entry_trigger_met: boolean;
  /** Last composite signal tree evaluation (None when no entry_signal configured). */
  entry_signal_met?: boolean | null;
  /** Task 21 — Hedge motoru durumu (yalnız Hedge ailesi botlarda bulunur). */
  hedge?: HedgeInfo;
}

/* ------------------------------------------------------------------ */
/* Task 21 — Hedge motoru panel veri sözleşmesi (studio Hedge sekmesi)  */
/* ------------------------------------------------------------------ */

/** Tek hedge bacağı (main long veya hedge short) — display string numerikler. */
export interface HedgeLegInfo {
  id: string;
  phase: string;
  position_qty: string;
  avg_price: string | null;
  realized_gross: string;
  unrealized_gross: string | null;
  pnl: string;
  deals_closed: number;
  wins: number;
  losses: number;
}

/** Hedge motoru durumu — stats.hedge (yalnız Hedge ailesinde var). */
export interface HedgeInfo {
  /** "PROTECTIVE" | "DUAL" */
  mode: string;
  /** "MainOnly" | "HedgeActive" | "Closing" */
  phase: string;
  hedge_ratio: string;
  combined_target: string;
  /** Son işlenen bardaki birleşik k/z (main + hedge leg). */
  combined_pnl: string;
  /** Koruma tetikleyici eşik (fraction display, "0.06" = %6). */
  drawdown_trigger: string;
  /** Son bardaki zirveden düşüş (fraction; protective modda). */
  drawdown_from_peak: string | null;
  peak_mark: string | null;
  spawn_count: number;
  /** Hedge bacağı (protective: spawn olmadıysa null). */
  hedge_leg: HedgeLegInfo | null;
  /** Motor içi olay günlüğü (spawn/exit) — son 10 kayıt. */
  events: string[];
}

export interface LadderRow {
  level: number;
  price: string;
  priceDisplay?: string;
  qty: string;
  notional: string;
  cumDeviation?: string;
  cumDev?: string;
  avgAfter?: string;
  avgAfterDisplay?: string;
  armed?: boolean;
}

export interface Bot {
  id: string;
  name: string;
  kind: BotKind;
  venue: Venue;
  mode: Mode;
  status: BotStatus;
  /** Faz 21 (N-8): kernel sticky-halt durumu — UI kırmızı rozet + RESET_KERNEL düğmesi. */
  kernelHalted?: boolean;
  kernelHaltedReason?: string | null;
  symbol: string;
  interval: Interval;
  stats: BotStats;
  cash: string;
  cumulativeFees: string;
  marginMode?: boolean;
  leverage?: string;
  imUsed?: string;
  liqPrice?: string | null;
  /** Açık pozisyonun son bar kapanış mark'ı üzerindeki unrealized gross PnL (ana + hedge bacak; mark yoksa null). */
  unrealizedNow?: string | null;
  /** TradingView webhook secret — present when the bot accepts external START_DEAL signals. */
  webhookSecret?: string | null;
  /** BotConfig snapshot as stored by the engine (present since 15-a wire update; optional for older snapshots). */
  config?: BotConfig | null;
  /** Advisory risk warnings echoed on the snapshot top-level `warnings` field (Task 16). */
  warnings?: AdvisoryWarning[];
  /** Risk policy echo from the snapshot top-level `riskPolicy` field (Task 16; optional for older bots). */
  riskPolicy?: UserRiskPolicy;
}

export interface BotDetail extends Bot {
  ladder: LadderRow[];
}

export type FillRole =
  | "BASE"
  | "SAFETY"
  | "TP"
  | "SL"
  | "MANUAL"
  | "TRAIL"
  | "GRID"
  | "INF";
/** Gate-5 realism tag on a single fill (engine wire values). */
export type EngineFillModel = "MarketableTaker" | "RestingTouch" | "OpenGapBetter";

export interface Fill {
  fillKey: string;
  orderId: string;
  role: string;
  side: "Buy" | "Sell";
  kind: "Add" | "Reduce";
  price: string;
  qty: string;
  fee: string;
  feeRole: string;
  fillModel: EngineFillModel;
  ts: number;
}

export interface BotEvent {
  level: "INFO" | "WARN" | "ERROR";
  gate: string;
  message: string;
  ts: number;
  /** Faz 2.4 — tick izi (göç-öncesi / tick-dışı olaylarda null). */
  traceId?: string | null;
}

/** Faz 2.4 — adli tıp iz özeti + iz olayı (v3 forensics karşılığı). */
export interface ForensicsTrace {
  traceId: string;
  eventCount: number;
  errorCount: number;
  firstTs: number;
  lastTs: number;
  totalMs: number;
  botId: string;
  symbol: string | null;
  kind: string | null;
}

export interface ForensicsEvent extends BotEvent {
  id: number;
  botId: string;
  traceId: string | null;
}

export interface BotConfig {
  kind: BotKind;
  symbol: string;
  base_order_size: string;
  safety_order_size: string;
  sizing_mode: "QUOTE_NOTIONAL" | "BASE_QTY";
  deviation_pct: string;
  step_scale: string;
  volume_scale: string;
  max_safety_orders: number;
  target_profit_pct: string;
  use_net_tp: boolean;
  trailing_enabled: boolean;
  trailing_trigger_pct: string;
  trailing_deviation_pct: string;
  stop_loss_pct: string | null;
  leverage: string;
  hedge_ratio: string;
  combined_target_profit: string;
  /** Task 21 — Hedge Protective koruma tetikleyici (PERCENT string, "6" = %6). */
  hedge_drawdown_trigger_pct?: string;
  grid_lower: string | null;
  grid_upper: string | null;
  grid_levels: number | null;
  grid_mode: "arithmetic" | "geometric";
  grid_trail: boolean;
  cooldown_bars: number;
  entry_trigger: EntryTrigger;
  /** 3Commas-style composite indicator condition tree (dca_signals). Absent = ASAP/simple only. */
  entry_signal?: SignalConfig | null;
  /** Task 19 — Pine Script (alt-küme) strateji kodu.entry_trigger = INSTANT zorunlu. */
  pine_script?: string | null;
  /** Task 19 — Pine input değerleri (değişken adı → string). */
  pine_inputs?: Record<string, string>;
  /** Risk Motoru advisory policy (Task 16). Absent in legacy snapshots — fall back to DEFAULT_RISK_POLICY. */
  risk_policy?: UserRiskPolicy;
}

export interface CreateBotBody {
  name: string;
  kind: BotKind;
  venue: Venue;
  mode: Mode;
  symbol: string;
  interval: Interval;
  start_cash: string;
  config: BotConfig;
  /** Task 16: user acknowledged the advisory risk panel (default false → engine 422 RISK_ACKNOWLEDGMENT_REQUIRED when warnings exist). */
  risk_acknowledged?: boolean;
}
