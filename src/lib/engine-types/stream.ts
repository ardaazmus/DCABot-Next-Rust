/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */
import type { FillModel } from "./core";

/* ---- SSE stream event payloads ---- */

export interface StreamFillPayload {
  botId: string;
  fillKey: string;
  orderId: string;
  role: string;
  side: "Buy" | "Sell";
  kind: "Add" | "Reduce";
  price: string;
  qty: string;
  fee: string;
  feeRole: string;
  fillModel: FillModel;
  ts: number;
}

export interface StreamEquityPayload {
  botId: string;
  ts: number;
  equity: string;
  positionQty: string;
  avgPrice: string;
  realizedGross: string;
  cash: string;
}

export interface StreamLogPayload {
  botId: string;
  level: string;
  message: string;
  ts: number;
}

export interface StreamBotStatePayload {
  botId: string;
  status: string;
  action?: string;
}

export interface StreamOrderPayload {
  botId: string;
  id: string;
  result: string;
}
/** Task 16 — live Risk Motoru advisory event (SSE `event: "RISK"`). */
export interface StreamRiskPayload {
  botId: string;
  code: string;
  message: string;
  ts: number;
}
