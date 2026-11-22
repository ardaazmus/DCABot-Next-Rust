"use client";

/* Faz A — tek-seferlik API yardımcıları (hesap/backtest/analiz/AI/market/vault/şalter).
   Tüketiciler değişmedi (`@/hooks/use-engine` barrel üzerinden). */

import { useCallback, useEffect, useState } from "react";
import type {
  AiCallConfig,
  AiProvider,
  AiResult,
  BacktestCertResult,
  BacktestMetrics,
  BacktestRequest,
  BacktestResult,
  BlacklistRow,
  BnbStatus,
  BotDetail,
  BotMemoryGenerateResult,
  BotMemoryListResult,
  BracketsResult,
  CalcDcaRequest,
  CalcDcaResult,
  CalcGridRequest,
  CalcGridResult,
  CalibrationResult,
  DryRunRequest,
  DryRunResult,
  FillMark,
  ForensicsEvent,
  ForensicsTrace,
  FuturesStateResult,
  GridMeshResult,
  Interval,
  KlineBar,
  MaintenanceStatus,
  MarketsResponse,
  NewsAiResult,
  NewsResult,
  OrderBookResult,
  ParityResult,
  ReconcileResult,
  RegimeResult,
  SelfCheckResult,
  TickersResponse,
  VaultKey,
  VaultKeyResponse,
} from "@/lib/engine-types";
import { engineFetch, engineJson } from "./engine-client";
import { useDebouncedValue, useEventStream, usePoll } from "./engine-stream";

/* ------------------------------------------------------------------ */
/* One-shot API helpers (mutations, searches)                          */
/* ------------------------------------------------------------------ */

export async function apiCalcDca(req: CalcDcaRequest): Promise<CalcDcaResult> {
  return engineFetch("/api/calc/dca", engineJson("POST", req));
}

export async function apiCalcGrid(req: CalcGridRequest): Promise<CalcGridResult> {
  return engineFetch("/api/calc/grid", engineJson("POST", req));
}

/** Faz 9/10 — grid parametre önerisi: pencere menzili + sigma → seviye/adım
    + ücret-kırıcı denetimi (motor: GET /api/analysis/grid-suggest). */
export interface GridSuggestResult {
  symbol: string;
  interval: string;
  bars: number;
  windowLow: string;
  windowHigh: string;
  sigmaPct: string;
  suggestedLevels: number;
  stepPct: string;
  gMinPct: string;
  feeBreakerPass: boolean;
  perLevelNotional: string;
  tickSize: string;
  stepSize: string;
  note: string;
}

export async function apiGridSuggest(
  symbol: string,
  interval: string,
  bars: number,
  investment: string,
  venue?: string,
): Promise<GridSuggestResult> {
  const p = new URLSearchParams({ symbol, interval, bars: String(bars), investment });
  if (venue) p.set("venue", venue);
  return engineFetch(`/api/analysis/grid-suggest?${p.toString()}`);
}

export async function apiBacktest(req: BacktestRequest, signal?: AbortSignal): Promise<BacktestResult> {
  return engineFetch("/api/backtest", { ...engineJson("POST", req), signal });
}

export async function apiBacktestCert(
  req: BacktestRequest & { mc_runs?: number; attempts?: number },
  signal?: AbortSignal,
): Promise<BacktestCertResult> {
  return engineFetch("/api/backtest/cert", { ...engineJson("POST", req), signal });
}

/** Faz 24 (UI/UX Pro Max §3.1.1): portföy equity curve — motor
    equity_points birleşimi. */
export async function apiEquity(days = 30, signal?: AbortSignal): Promise<{ days: number; points: { ts: number; equity: number }[]; total: number }> {
  return engineFetch(`/api/equity?days=${days}`, { ...{ signal }, ...{} } as RequestInit);
}

export interface ClosedDeal {
  id: string;
  botId: string;
  botName: string;
  symbol: string;
  venue: string;
  kind: string;
  closedTs: number;
  date: string;
  time: string;
  grossPnl: number;
  fees: number;
  funding: number;
  netPnl: number;
  wins: number;
  losses: number;
  result: "WIN" | "LOSS";
  reason: "TP" | "SL" | "LIQUIDATION" | "MANUAL" | "TRAIL" | "CLOSE";
}

export interface DealsSummary {
  totalNetPnl: number;
  totalGrossPnl: number;
  totalFees: number;
  totalFunding: number;
  wins: number;
  losses: number;
  winRate: number;
  profitFactor?: number;
  avgDealProfit?: number;
}

export interface DealsResponse {
  deals: ClosedDeal[];
  total: number;
  summary: DealsSummary;
}

export interface DailyPnlBar {
  date: string;
  dayTs: number;
  netPnl: number;
  grossPnl: number;
  fees: number;
  funding: number;
  count: number;
  wins: number;
  losses: number;
  winRate: number;
}

export interface DailyPnlResponse {
  days: DailyPnlBar[];
  summary: {
    totalNetPnl: number;
    totalDeals: number;
    profitableDays: number;
    unprofitableDays: number;
    neutralDays: number;
    bestDay: { date: string; netPnl: number; count: number } | null;
    worstDay: { date: string; netPnl: number; count: number } | null;
    profitFactor?: number;
  };
}

export async function apiDeals(
  params?: {
    days?: number;
    date?: string;
    bot_id?: string;
    symbol?: string;
    q?: string;
    limit?: number;
  },
  signal?: AbortSignal,
): Promise<DealsResponse> {
  const q = new URLSearchParams();
  if (params?.days != null) q.set("days", String(params.days));
  if (params?.date) q.set("date", params.date);
  if (params?.bot_id) q.set("bot_id", params.bot_id);
  if (params?.symbol) q.set("symbol", params.symbol);
  if (params?.q) q.set("q", params.q);
  if (params?.limit != null) q.set("limit", String(params.limit));
  const qs = q.toString();
  return engineFetch(`/api/deals${qs ? `?${qs}` : ""}`, { signal });
}

export async function apiDailyPnl(
  days = 30,
  params?: { bot_id?: string; symbol?: string },
  signal?: AbortSignal,
): Promise<DailyPnlResponse> {
  const q = new URLSearchParams();
  q.set("days", String(days));
  if (params?.bot_id) q.set("bot_id", params.bot_id);
  if (params?.symbol) q.set("symbol", params.symbol);
  return engineFetch(`/api/daily-pnl?${q.toString()}`, { signal });
}

export async function apiParity(body: { bot_id: string; bars?: number; interval?: string }): Promise<ParityResult> {
  return engineFetch("/api/parity", engineJson("POST", body));
}

export async function apiReconcile(botId: string): Promise<ReconcileResult> {
  return engineFetch(`/api/live/reconcile?bot_id=${encodeURIComponent(botId)}`);
}

/** Faz 1.1 — 0-risk emir provası (M08 hizalama, sent:false; ağa emir gitmez). */
export async function apiDryRun(req: DryRunRequest): Promise<DryRunResult> {
  return engineFetch("/api/live/dryrun", engineJson("POST", req));
}

/** P2-3 — 19 kontrol self-check (sembol: imzalı pozisyon bacağı için). */
export async function apiSelfCheck(symbol?: string): Promise<SelfCheckResult> {
  return engineFetch("/api/live/selfcheck", engineJson("POST", symbol ? { symbol } : {}));
}

/** P3-1 — kara liste: listele / ekle-güncelle / kaldır. */
export async function apiBlacklist(): Promise<{ blacklist: BlacklistRow[] }> {
  return engineFetch("/api/blacklist");
}

export async function apiBlacklistAdd(pair: string, reason?: string): Promise<BlacklistRow> {
  return engineFetch("/api/blacklist", engineJson("POST", reason ? { pair, reason } : { pair }));
}

export async function apiBlacklistRemove(pair: string): Promise<{ removed: string }> {
  return engineFetch(`/api/blacklist/${encodeURIComponent(pair)}`, { method: "DELETE" });
}

/** P3-5 — bakım sentineli (60 sn yoklama). */
export async function apiMaintenance(): Promise<MaintenanceStatus> {
  return engineFetch("/api/system/maintenance");
}

export async function apiMaintenanceSet(quiet: boolean, note?: string): Promise<{ quiet: boolean; note: string }> {
  return engineFetch("/api/system/maintenance", engineJson("POST", note ? { quiet, note } : { quiet }));
}

/** Faz 2.7 — venue kademe aynası (talep üzerine; önbellek yok, salt-okunur). */
export async function apiBrackets(venue: string, symbol: string): Promise<BracketsResult> {
  const q = new URLSearchParams({ venue, symbol });
  return engineFetch(`/api/calc/brackets?${q.toString()}`);
}

/** Faz 2.4 — olay adli tıp (3 mod; salt-okunur). */
export async function apiForensicsTraces(limit = 50): Promise<{ traces: ForensicsTrace[] }> {
  return engineFetch(`/api/events/forensics?mode=traces&limit=${limit}`);
}

export async function apiForensicsTrace(traceId: string): Promise<{ trace: { traceId: string; rootTs: number; totalMs: number; eventCount: number; errorCount: number; spanCount: number } | null; events: ForensicsEvent[] }> {
  return engineFetch(`/api/events/forensics?trace_id=${encodeURIComponent(traceId)}`);
}

/** Faz 1.6 — BNB komisyon bekçisi (salt-okunur, 60 sn poll; motor 45 sn önbellekli). */
export async function apiBnb(): Promise<BnbStatus> {
  return engineFetch("/api/live/bnb");
}

export function useBnb(intervalMs = 60000) {
  const [status, setStatus] = useState<BnbStatus | null>(null);
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const s = await apiBnb();
        if (alive) setStatus(s);
      } catch {
        /* motor kapalıysa sessiz — offline banner zaten var */
      }
    };
    void load();
    const id = setInterval(() => void load(), intervalMs);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [intervalMs]);
  return { status };
}

export function useMaintenance(intervalMs = 60000) {
  const [status, setStatus] = useState<MaintenanceStatus | null>(null);
  /* R2 (denetim): ilk yoklama sonuçlandı mı — sonuçsuz bekleyiş ile hatayı ayırt eder. */
  const [attempted, setAttempted] = useState(false);
  const [tick, setTick] = useState(0);
  const refetch = useCallback(() => setTick((t) => t + 1), []);
  useEffect(() => {
    let alive = true;
    const load = async () => {
      try {
        const s = await apiMaintenance();
        if (alive) setStatus(s);
      } catch {
        /* motor kapalıysa sessiz — offline banner zaten var */
      } finally {
        if (alive) setAttempted(true);
      }
    };
    void load();
    const id = setInterval(() => void load(), intervalMs);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [intervalMs, tick]);
  return { status, attempted, refetch };
}

/** P3-6 — orderbook L2 (REST anlık görüntü + imbalance). */
export async function apiOrderbook(symbol: string, venue = "BYBIT", category = "linear", limit = 25): Promise<OrderBookResult> {
  const q = new URLSearchParams({ symbol, venue, category, limit: String(limit) });
  return engineFetch(`/api/orderbook?${q.toString()}`);
}

/** P3-7 — vadeli durum kartı (kurallar + canlı pozisyon, salt-okunur). */
export async function apiFuturesState(symbol: string, venue = "BYBIT"): Promise<FuturesStateResult> {
  const q = new URLSearchParams({ symbol, venue });
  return engineFetch(`/api/live/futures-state?${q.toString()}`);
}

/** P3-8 — grid mesh + manuel tick (v3 grid-live/deals-tick karşılığı). */
export async function apiBotGrid(id: string): Promise<GridMeshResult> {
  return engineFetch(`/api/bots/${encodeURIComponent(id)}/grid`);
}

export async function apiBotTick(id: string): Promise<{ id: string; newBars: number; snapshot: BotDetail }> {
  return engineFetch(`/api/bots/${encodeURIComponent(id)}/tick`, engineJson("POST", {}));
}

/** P4-1 — haber motoru (duyarlılık + delisting kapısı, 5 dk önbellek). */
export async function apiNews(symbol: string): Promise<NewsResult> {
  return engineFetch(`/api/market/news?symbol=${encodeURIComponent(symbol)}`);
}

/* ---- P4-2 YZ Yardımcı (teşhis/sohbet + haber TR-özeti) ---- */

export async function apiAiProviders(): Promise<{ providers: AiProvider[] }> {
  return engineFetch("/api/ai/providers");
}

export async function apiAiDiagnose(body: {
  mode: "diagnose" | "chat";
  bot_id?: string;
  message?: string;
  history?: { role: string; content: string }[];
  symbol?: string;
  config?: AiCallConfig;
}): Promise<AiResult> {
  return engineFetch("/api/ai/diagnose", engineJson("POST", body));
}

export async function apiNewsAi(id: string, config?: AiCallConfig): Promise<NewsAiResult> {
  return engineFetch("/api/market/news-ai", engineJson("POST", { id, config }));
}

/* ---- Faz 2.3 Bot Hafızası ---- */

export async function apiBotMemoryList(id: string, limit = 20): Promise<BotMemoryListResult> {
  return engineFetch(`/api/bots/${encodeURIComponent(id)}/memory?limit=${limit}`);
}

export async function apiBotMemoryGenerate(id: string, config?: AiCallConfig): Promise<BotMemoryGenerateResult> {
  return engineFetch(`/api/bots/${encodeURIComponent(id)}/memory`, engineJson("POST", config ? { config } : {}));
}

/** P3-2 — rejim rozeti (motor sınıflandırıcısı). */
export async function apiRegime(symbol: string, interval = "1h", bars = 300): Promise<RegimeResult> {
  const q = new URLSearchParams({ symbol, interval, bars: String(bars) });
  return engineFetch(`/api/analysis/regime?${q.toString()}`);
}

/** P3-3 — kalibrasyon: oynaklıktan sapma adımı önerisi. */
export async function apiCalibration(symbol: string, interval = "1h", bars = 500): Promise<CalibrationResult> {
  return engineFetch("/api/analysis/calibration", engineJson("POST", { symbol, interval, bars }));
}

/* Task 19 — Pine Script (alt-küme) doğrulama + backtest */
export interface PineValidateResponse {
  ok: boolean;
  decl: { kind: "strategy" | "indicator"; title: string; pyramiding?: number | null } | null;
  inputs: string[];
  functions: string[];
  plots: string[];
  /** Task 20 — derleme-anı dürüst notlar (koşullu ta.*, pyramiding uyarısı) */
  notes?: string[];
}

export interface PineBacktestResponse {
  ok: boolean;
  decl: { kind: "strategy" | "indicator"; title: string; pyramiding?: number | null } | null;
  inputs: string[];
  metrics: BacktestMetrics;
  equityCurve: { ts: number; equity: string }[];
  startEquity: string;
  endEquity: string;
  entries: number;
  fills: number;
  pineWarnings: string[];
  /** Task 20 — derleme notları + plot serileri + bot TP/SL durumu */
  pineNotes?: string[];
  pinePlots?: Record<string, string[]>;
  botStopsApplied?: boolean;
  /** Task 21 — backtest'in kullandığı TAM bar penceresi (plot hizalama kaynağı). */
  bars?: KlineBar[];
  /** Task 21 — fill işaretçileri (fiyat grafiği okları). */
  fillMarks?: FillMark[];
}

export interface PineBacktestRequest {
  code: string;
  inputs?: Record<string, string>;
  symbol: string;
  interval: Interval;
  bars?: number;
  start_cash?: string;
  kind?: "DCA_LONG" | "DCA_SHORT";
  /** Task 20 — opsiyonel bot TP/SL (pine çıkışına ek; verilmezse çıkış yalnız pine'dan) */
  target_profit_pct?: string;
  stop_loss_pct?: string;
}

export async function apiPineValidate(code: string): Promise<PineValidateResponse> {
  return engineFetch("/api/pine/validate", engineJson("POST", { code }));
}

export async function apiPineBacktest(req: PineBacktestRequest): Promise<PineBacktestResponse> {
  return engineFetch("/api/pine/backtest", engineJson("POST", req));
}

/* ------------------------------------------------------------------ */
/* Task 21 — Pine strateji kütüphanesi (kaydet/yükle/sil)              */
/* ------------------------------------------------------------------ */

/** Kütüphane kaydı — motor pine_scripts tablosundan döner. */
export interface PineScriptItem {
  id: string;
  name: string;
  code: string;
  /** Kaydedilen input değerleri (değişken adı → string). */
  inputs?: Record<string, string>;
  notes?: string | null;
  createdAt?: string;
  updatedAt?: string;
}

export async function apiPineScripts(): Promise<{ scripts: PineScriptItem[] }> {
  return engineFetch("/api/pine/scripts");
}

export async function apiSavePineScript(body: {
  name: string;
  code: string;
  inputs?: Record<string, string>;
  notes?: string | null;
}): Promise<{ ok: boolean; id: string; name: string; notes?: string[] }> {
  return engineFetch("/api/pine/scripts", engineJson("POST", body));
}

export async function apiPatchPineScript(
  id: string,
  body: { name?: string; code?: string; inputs?: Record<string, string>; notes?: string | null },
): Promise<{ ok: boolean }> {
  return engineFetch(`/api/pine/scripts/${id}`, engineJson("PATCH", body));
}

export async function apiDeletePineScript(id: string): Promise<{ ok: boolean }> {
  return engineFetch(`/api/pine/scripts/${id}`, { method: "DELETE" });
}

export async function apiMarkets(category: "spot" | "linear", symbol?: string, venue?: string): Promise<MarketsResponse> {
  const params = new URLSearchParams({ category });
  if (symbol) params.set("symbol", symbol);
  if (venue) params.set("venue", venue);
  return engineFetch(`/api/markets?${params.toString()}`);
}

export async function apiKlines(symbol: string, interval: Interval, limit: number): Promise<{ bars: KlineBar[] }> {
  return engineFetch(`/api/klines?symbol=${encodeURIComponent(symbol)}&interval=${interval}&limit=${limit}`);
}

/** P2-1 — 24h istatistik listesi (sembol tarayıcı beslemesi, 30sn motor-cache). */
export async function apiTickers(
  category: "spot" | "linear",
  venue?: string,
): Promise<TickersResponse> {
  const params = new URLSearchParams({ category });
  if (venue) params.set("venue", venue);
  return engineFetch(`/api/tickers?${params.toString()}`);
}

export async function apiVaultKeys(): Promise<{ keys: VaultKey[] }> {
  return engineFetch("/api/vault/keys");
}

export async function apiAddVaultKey(body: {
  venue: string;
  label: string;
  api_key: string;
  api_secret: string;
  /** P4-3: OKX için zorunlu (diğer venue'lerde yoksayılır). */
  passphrase?: string;
}): Promise<VaultKeyResponse> {
  return engineFetch("/api/vault/keys", engineJson("POST", body));
}

export async function apiDeleteVaultKey(id: string): Promise<unknown> {
  return engineFetch(`/api/vault/keys/${id}`, { method: "DELETE" });
}

/* ---- Faz 0.2/0.3 — global yürütme şalterleri (v3 killswitch/freeze karşılığı) ---- */

export interface KillSwitchState {
  engaged: boolean;
  reason: string | null;
  by: string | null;
  /** epoch ms (restart-safe olay akışından). */
  at: number | null;
  auto: boolean;
}

export interface FreezeState {
  frozen: boolean;
  reason: string | null;
  by: string | null;
  at: number | null;
}

export async function apiKillState(): Promise<KillSwitchState> {
  return engineFetch("/api/live/killswitch");
}

export async function apiKillSet(engaged: boolean, reason?: string): Promise<KillSwitchState> {
  return engineFetch("/api/live/killswitch", engineJson("POST", reason ? { engaged, reason } : { engaged }));
}

export async function apiFreezeState(): Promise<FreezeState> {
  return engineFetch("/api/live/freeze");
}

export async function apiFreezeSet(frozen?: boolean, reason?: string): Promise<FreezeState> {
  const body: Record<string, unknown> = {};
  if (frozen !== undefined) body.frozen = frozen;
  if (reason) body.reason = reason;
  return engineFetch("/api/live/freeze", engineJson("POST", body));
}

/** Şalter durumları (10sn poll + SSE tetiklemeli tazeleme). */
export function useHalt(): {
  kill: KillSwitchState | null;
  freeze: FreezeState | null;
  refetch: () => void;
} {
  const { dataVer } = useEventStream();
  const ver = useDebouncedValue(dataVer, 900);
  const killPoll = usePoll<KillSwitchState>("/api/live/killswitch", 10000, [ver]);
  const freezePoll = usePoll<FreezeState>("/api/live/freeze", 10000, [ver]);
  return {
    kill: killPoll.data,
    freeze: freezePoll.data,
    refetch: () => {
      killPoll.refetch();
      freezePoll.refetch();
    },
  };
}
