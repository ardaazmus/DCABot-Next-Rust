/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */

import type { BotConfig, LadderRow } from "./bots";
import type { BotKind, Interval } from "./core";
import type { FillMark, KlineBar } from "./market";

export interface CalcDcaRequest {
  anchor: string;
  base_order_size: string;
  safety_order_size: string;
  sizing_mode: "QUOTE_NOTIONAL" | "BASE_QTY";
  deviation_pct: string;
  step_scale: string;
  volume_scale: string;
  max_safety_orders: number;
  target_profit_pct: string;
  use_net_tp: boolean;
  stop_loss_pct: string | null;
  tick_size: string;
  step_size: string;
  maker_fee: string;
  taker_fee: string;
  leverage: string;
  market: "spot" | "linear";
  direction?: "long" | "short";
}

export interface CalcDcaResult {
  levels: LadderRow[];
  totalNotional: string;
  totalNotionalDisplay?: string;
  totalQty: string;
  avgAllIn: string;
  estimatedInitialMargin: string;
  estimatedFees: string;
  liquidationPrice: string | null;
  m20: { viable: boolean; firstUnreachableLevel: number | null };
  plannedSafetyNotional: string;
}

export interface CalcGridRequest {
  lower: string;
  upper: string;
  levels: number;
  grid_mode: "arithmetic" | "geometric";
  notional_per_level: string;
  tick_size: string;
  step_size: string;
  maker_fee: string;
  taker_fee: string;
}

export interface CalcGridResult {
  mesh: LadderRow[];
  feeBreakerPass: boolean;
  gMin?: string;
  totalInvested: string;
}
/** Faz 2.8 — filo tape satırı (`/api/fleet/overview` tape — son 40 fill). */
export interface FleetTapeRow {
  botId: string;
  botName: string;
  venue: string;
  side: "Buy" | "Sell";
  price: string;
  qty: string;
  fee: string;
  role: string;
  ts: number;
}

export interface FleetOverviewResult {
  ok: boolean;
  tape: FleetTapeRow[];
  portfolio?: Record<string, unknown>;
  allocation?: Record<string, unknown>[];
  openDeals?: unknown[];
}
/** Faz 2.7 — M07 kademe satırı (kesir stringler; fmtFrac/fracToNum ile görüntülenir). */
export interface BracketsTier {
  min_notional: string;
  max_notional: string;
  mmr: string;
  deduction: string;
}

export interface BracketsResult {
  venue: string;
  symbol: string;
  source: "EXCHANGE" | "ABSTAIN" | "UNSUPPORTED";
  tiers: BracketsTier[];
  reason?: string;
  at: number;
  note: string;
}
export interface BacktestMetrics {
  total_return_pct: string;
  max_drawdown_pct: string;
  win_rate_pct: string;
  profit_factor: string;
  sharpe_display: string;
  total_fees: string;
  /** Cumulative funding ledger (futures backtests; "0.000000" on spot). */
  total_funding?: string;
  bars: number;
  deals: number;
  /** Task 19 — dolum gerçekçiliği kırılımı (fill model dağılımı). */
  fill_realism?: FillRealismBreakdown;
}

/** Task 19 — fill model etiket dağılımı + maker/taker komisyon ayrışı. */
export interface FillRealismBreakdown {
  marketable_taker: number;
  resting_touch: number;
  open_gap_better: number;
  maker_fees: string;
  taker_fees: string;
}

/** Task 22 — motorun o backtest koşusunda GERÇEKTEN verdiği engel kararları.
 *  Kodlar motor-dallıdır (ENTRY_TRIGGER_NEVER_MET, SIGNAL_BLOCKED,
 *  REGIME_BLOCKED, BREAKER_BLOCKED, COOLDOWN_ACTIVE,
 *  CONFIG_INVALID, GRID_BOOTSTRAP_*); bilinmeyen kod aynen gösterilir. */
export interface NoDealReason {
  code: string;
  bars: number;
  detail: string;
}

export interface BacktestResult {
  metrics: BacktestMetrics;
  equityCurve: { ts: number; equity: string }[];
  startEquity: string;
  endEquity: string;
  fills: number;
  gateLogs: number;
  /** Task 18 N-5 — funding geçmişi alınamadığında dürüst uyarı metni. */
  fundingWarning?: string | null;
  /** Task 19 — Pine strateji uyarıları (pyramiding vb.). */
  pineWarnings?: string[];
  /** Task 20 — derleme notları (koşullu ta.* vb.). */
  pineNotes?: string[];
  /** Task 20 — plot serileri (başlık → bar-dizini hizalı display değerler, "na" = yok). */
  pinePlots?: Record<string, string[]>;
  /** Task 19 — dolum gerçekçiliği ayrıştırıcı karşılaştırma (iyimser ikinci koşu). */
  realismCompare?: {
    optimisticMetrics: BacktestMetrics;
    optimisticEndEquity: string;
    optimisticFills: number;
  } | null;
  /** Task 21 — backtest'in kullandığı TAM bar penceresi (plot hizalama kaynağı). */
  bars?: KlineBar[];
  /** P4-3d: koşunun market verisi bu venue'den geldi. */
  venue?: string;
  /** Task 21 — fill işaretçileri (fiyat grafiği okları). */
  fillMarks?: FillMark[];
  /** Task 22 — boş sonuçların makine-okunur nedenleri (motor sayaçları). */
  noDealReasons?: NoDealReason[];
}

export interface ParityResult {
  verdict: "GECTI" | "KALDI" | "YETERSIZ_VERI";
  countMatchPct: string;
  feeMatchPct: string;
  btFills: number;
  paperFills: number;
  winRateBt: string;
  bars: number;
  note: string;
}

/** P2-3 — self-check satırı + sonuç (v3 runSelfCheck karşılığı, 19 kontrol). */
export interface SelfCheckItem {
  id: number;
  group: string;
  name: string;
  status: "PASS" | "FAIL" | "ABSTAIN";
  detail: string;
  ms: number;
}

export interface SelfCheckResult {
  verdict: "GECTI" | "KALDI" | "YETERSIZ_ORTAM";
  passed: number;
  failed: number;
  abstained: number;
  hardFail: boolean;
  items: SelfCheckItem[];
  symbol: string;
  elapsedMs: number;
  note: string;
}

/** P3-1 — kara liste satırı + P3-2 rejim + P3-3 kalibrasyon. */
export interface BlacklistRow {
  pair: string;
  reason: string | null;
  createdAt: number;
}

export interface RegimeResult {
  symbol: string;
  regime: "NORMAL" | "CRASH" | "HIGH_VOLATILITY";
  blocksNewDeals: boolean;
  bars: number;
  note: string;
}

export interface CalibrationResult {
  symbol: string;
  bars: number;
  muPct: string;
  sigmaPct: string;
  q95DropPct: string;
  suggestedDeviationPct: string;
  note: string;
}
export interface ReconcileResult {
  supported: boolean;
  reason?: string;
  match?: boolean;
  local?: { side: string; qty: string };
  exchange?: { side: string; size: string; avgPrice: string }[];
  actionRequired?: boolean;
  note?: string;
}

export interface DryRunAlignment {
  field: string;
  raw: string;
  aligned: string;
  rule: string;
}

export interface DryRunResult {
  order: Record<string, string>;
  alignment: DryRunAlignment[];
  ok: boolean;
  reason?: string;
  note: string;
  sent: false;
  venue: string;
  category: string;
  symbol: string;
}

export interface DryRunRequest {
  symbol: string;
  side: "BUY" | "SELL";
  type: "MARKET" | "LIMIT" | "STOP_MARKET" | "TAKE_PROFIT_MARKET";
  quantity?: string;
  quoteOrderQty?: string;
  price?: string;
  stopPrice?: string;
  role?: string;
  venue?: string;
}

export interface BacktestRequest {
  kind: BotKind;
  symbol: string;
  interval: Interval;
  bars: number;
  start_cash: string;
  config: BotConfig;
  /** P4-3d: market verisi bu venue'den çekilir (verilmezse BYBIT). */
  venue?: string;
}

/** Backtest Certifier — WFE / DSR / PBO (CSCV) / Monte Carlo block bootstrap. */
export interface CertifyOutput {
  trials: number;
  wfe_pct: string;
  dsr: string;
  pbo_pct: string;
  blocks: number;
  combos: number;
  mc_runs: number;
  mc_prob_loss_pct: string;
  mc_return_p5_pct: string;
  mc_return_p50_pct: string;
  mc_return_p95_pct: string;
  mc_mean_mdd_pct: string;
  warnings: string[];
}

export interface CertGateView {
  key: string;
  label: string;
  value: string;
  threshold: string;
  pass: boolean;
  detail: string;
}

export interface BacktestCertResult {
  cert: CertifyOutput;
  /** Faz 0.1 — motor hükmü (Lab rozet eşikleriyle aynı kapılar). */
  verdict?: "GECTI" | "KALDI" | string;
  gates?: CertGateView[];
  base: {
    totalReturnPct: string;
    maxDrawdownPct: string;
    sharpeDisplay: string;
    deals: number;
  } | null;
}
