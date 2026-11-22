"use client";

/* Faz A — bot domain hook'ları (liste/detay/fills/olaylar/tape + mutasyonlar).
   Tüketiciler değişmedi (`@/hooks/use-engine` barrel üzerinden). */

import type {
  AdvisoryWarning,
  Bot,
  BotDetail,
  BotEvent,
  BotStatus,
  CreateBotBody,
  Fill,
  FleetOverviewResult,
  Summary,
} from "@/lib/engine-types";
import { engineFetch, engineJson } from "./engine-client";
import { useDebouncedValue, useEventStream, usePoll } from "./engine-stream";

/* ------------------------------------------------------------------ */
/* Bot creation metadata (deviation etc. needed for client M20 hints) */
/* ------------------------------------------------------------------ */

export interface BotMeta {
  deviation_pct: string;
}

const BOT_META_KEY = "dcabot.botmeta.v1";

function readMetaMap(): Record<string, BotMeta> {
  if (typeof window === "undefined") return {};
  try {
    return JSON.parse(window.localStorage.getItem(BOT_META_KEY) ?? "{}") as Record<string, BotMeta>;
  } catch {
    return {};
  }
}

export function saveBotMeta(id: string, meta: BotMeta): void {
  if (typeof window === "undefined") return;
  const map = readMetaMap();
  map[id] = meta;
  try {
    window.localStorage.setItem(BOT_META_KEY, JSON.stringify(map));
  } catch {
    /* ignore quota */
  }
}

export function getBotMeta(id: string): BotMeta | null {
  return readMetaMap()[id] ?? null;
}

/* ------------------------------------------------------------------ */
/* Domain hooks                                                       */
/* ------------------------------------------------------------------ */

export function useSummary(): { summary: Summary | null; error: string | null; loading: boolean; refetch: () => void } {
  const { dataVer } = useEventStream();
  const ver = useDebouncedValue(dataVer, 900);
  const poll = usePoll<Summary>("/api/summary", 5000, [ver]);
  return { summary: poll.data, error: poll.error, loading: poll.loading, refetch: poll.refetch };
}

export function useBots(): { bots: Bot[]; error: string | null; loading: boolean; refetch: () => void } {
  const { dataVer } = useEventStream();
  const ver = useDebouncedValue(dataVer, 900);
  const poll = usePoll<{ bots: Bot[] }>("/api/bots", 5000, [ver]);
  return { bots: poll.data?.bots ?? [], error: poll.error, loading: poll.loading, refetch: poll.refetch };
}

export function useBotDetail(id: string | null): {
  bot: BotDetail | null;
  error: string | null;
  loading: boolean;
  refetch: () => void;
} {
  const { dataVer } = useEventStream();
  const ver = useDebouncedValue(dataVer, 900);
  const poll = usePoll<BotDetail>(id ? `/api/bots/${id}` : null, 5000, [ver]);
  return { bot: poll.data, error: poll.error, loading: poll.loading, refetch: poll.refetch };
}

export function useFills(id: string | null, opts?: { live?: boolean }): { fills: Fill[]; error: string | null; loading: boolean; refetch: () => void } {
  const { dataVer } = useEventStream();
  const ver = useDebouncedValue(dataVer, 900);
  // Faz 23 (UI derin inceleme #9): dashboard N+1 amplifikasyonu — filodaki
  // HER bot kartı useFills açıyordu ve her FILL SSE olayı N paralel istek
  // tetikliyordu (15 bot ≈ 18 istek/dolum). Varsayılan canlı-değil modu:
  // yalnız 5 sn poll (Studio kendi canlı useFills'ini live:true ile açar).
  const live = opts?.live ?? false;
  const poll = usePoll<{ fills: Fill[] }>(id ? `/api/bots/${id}/fills` : null, 5000, live ? [ver] : []);
  return { fills: poll.data?.fills ?? [], error: poll.error, loading: poll.loading, refetch: poll.refetch };
}

/** Faz 2.8 — filo tape: `/api/fleet/overview` tape (son 40 fill, 30sn poll). */
export function useFleetTape(intervalMs = 30000): {
  tape: FleetOverviewResult["tape"];
  error: string | null;
  loading: boolean;
  refetch: () => void;
} {
  const poll = usePoll<FleetOverviewResult>("/api/fleet/overview", intervalMs);
  return { tape: poll.data?.tape ?? [], error: poll.error, loading: poll.loading, refetch: poll.refetch };
}

export function useBotEvents(id: string | null): {
  events: BotEvent[];
  error: string | null;
  loading: boolean;
  refetch: () => void;
} {
  const poll = usePoll<{ events: BotEvent[] }>(id ? `/api/bots/${id}/events` : null, 5000);
  return { events: poll.data?.events ?? [], error: poll.error, loading: poll.loading, refetch: poll.refetch };
}

/* ------------------------------------------------------------------ */
/* Bot mutations                                                      */
/* ------------------------------------------------------------------ */

export interface CreateBotResponse {
  id: string;
  status: string;
  /** Advisory warnings echoed by the engine on 201 (Task 16 wire). */
  warnings?: AdvisoryWarning[];
}

/**
 * Task 16: optional `riskAcknowledged` rides on the POST body as
 * `risk_acknowledged`. When undefined the legacy wire is kept (no field),
 * so existing callers stay byte-compatible.
 */
export async function apiCreateBot(
  body: CreateBotBody,
  riskAcknowledged?: boolean,
): Promise<CreateBotResponse> {
  const flags: Record<string, boolean> = {};
  if (riskAcknowledged !== undefined) flags.risk_acknowledged = riskAcknowledged;
  const payload = Object.keys(flags).length === 0 ? body : { ...body, ...flags };
  return engineFetch("/api/bots", engineJson("POST", payload));
}

export async function apiPatchBot(id: string, patch: Record<string, unknown>): Promise<unknown> {
  return engineFetch(`/api/bots/${id}`, engineJson("PATCH", patch));
}

export async function apiDeleteBot(id: string): Promise<unknown> {
  return engineFetch(`/api/bots/${id}`, { method: "DELETE" });
}

export async function apiBotAction(
  id: string,
  action: string,
  amount?: string,
): Promise<unknown> {
  const body: Record<string, unknown> = amount !== undefined ? { action, amount } : { action };
  return engineFetch(`/api/bots/${id}/actions`, engineJson("POST", body));
}

export async function apiStopAllBots(): Promise<{ ok: number; fail: number }> {
  const { bots } = await engineFetch<{ bots: Bot[] }>("/api/bots");
  let ok = 0;
  let fail = 0;
  await Promise.allSettled(
    bots.map(async (b) => {
      try {
        await apiPatchBot(b.id, { status: "STOPPED" as BotStatus });
        ok += 1;
      } catch {
        fail += 1;
      }
    }),
  );
  return { ok, fail };
}
