/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */

export interface Instrument {
  symbol: string;
  base: string;
  quote: string;
  tick_size: string;
  step_size: string;
  min_qty: string;
  min_notional: string;
  maker_fee_rate: string;
  taker_fee_rate: string;
  max_leverage: string;
}

export interface MarketsResponse {
  instruments: Instrument[];
  cached: boolean;
}

/** P2-1 — 24h istatistik satırı (sembol tarayıcı). change_pct_24h yüzde puan string.
 *  Wire format motordaki snake_case'i korur (Instrument ile aynı konvansiyon). */
export interface Ticker24h {
  symbol: string;
  last: string;
  change_pct_24h: string;
  high_24h: string;
  low_24h: string;
  volume_24h: string;
  turnover_24h: string;
}

export interface TickersResponse {
  tickers: Ticker24h[];
  cached: boolean;
}

export interface KlineBar {
  t: number;
  o: string;
  h: string;
  l: string;
  c: string;
  v: string;
}

/** Task 21 — backtest fill işaretçisi (fiyat grafiği üzerinde ok). */
export interface FillMark {
  t: number;
  price: string;
  side: "Buy" | "Sell";
  role: string;
}
export interface MaintenanceVenue {
  venue: string;
  ok: boolean;
  serverTimeMs?: number;
  error?: string;
  ms: number;
}

export interface MaintenanceStatus {
  mode: "NORMAL" | "DEGRADED" | "QUIET";
  quiet: boolean;
  note: string;
  venues: MaintenanceVenue[];
  checkedAt: number;
}

/** Faz 1.6 — BNB komisyon bekçisi (v3 bnb-sentinel karşılığı, GET only). */
export interface BnbStatus {
  freeBnb: string | null;
  threshold: string;
  alert: boolean;
  source: "EXCHANGE" | "ABSTAIN";
  reason?: string;
  cached: boolean;
  at: number;
  note: string;
}

export interface OrderBookLevel {
  price: string;
  qty: string;
}

export interface OrderBookResult {
  symbol: string;
  bids: OrderBookLevel[];
  asks: OrderBookLevel[];
  bestBid: string;
  bestAsk: string;
  spreadBps: string;
  imbalance: string;
  ts: number;
  note: string;
}

export interface FuturesStateResult {
  symbol: string;
  venue: string;
  maxLeverage: string | null;
  tickSize: string | null;
  minNotional: string | null;
  live: { supported: boolean; positions?: { side: string; size: string; avgPrice: string; leverage: string }[]; note?: string };
  note: string;
}

export interface GridMeshResult {
  id: string;
  kind: string;
  bootstrapped: boolean;
  mesh: string[];
  armed: boolean[];
  levels: number;
  note: string;
}

export interface NewsItem {
  id: string;
  title: string;
  url: string;
  publishedAt: number;
  source: string;
  sentiment: number;
  impact: "LOW" | "MEDIUM" | "HIGH" | "CRITICAL";
  /** P4-2: öğe başına TR özet (news-ai önbelleğinden gelirse dolar). */
  aiSummary?: string | null;
}
export interface NewsResult {
  symbol: string;
  items: NewsItem[];
  count: number;
  mean: number;
  bull: number;
  bear: number;
  neutral: number;
  delisting: { active: boolean; headline: string | null; url: string | null; keyword?: string };
  sources: string[];
  cached: boolean;
  note: string;
}
