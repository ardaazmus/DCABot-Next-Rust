"use client";

/* Faz A — yoklama + canlı akış altyapısı (usePoll, SSE store, debounce, health).
   Tüketiciler değişmedi (`@/hooks/use-engine` barrel üzerinden). */

import { useCallback, useEffect, useRef, useState } from "react";
import { create } from "zustand";
import type { Health } from "@/lib/engine-types";
import { engineFetch, engineUrl } from "./engine-client";

/* ------------------------------------------------------------------ */
/* Generic polling hook                                               */
/* ------------------------------------------------------------------ */

interface PollState<T> {
  data: T | null;
  error: string | null;
  loading: boolean;
  /** Faz 20 (UIX-6): son başarılı yanıtın epoch ms'i — ekranın ne kadar
   * taze olduğunu UI göstermek için. null = hiç başarılı yanıt yok. */
  lastUpdatedAt: number | null;
}

export function usePoll<T>(
  path: string | null,
  intervalMs: number,
  refreshKeys: ReadonlyArray<string | number | boolean | null | undefined> = [],
): { data: T | null; error: string | null; loading: boolean; lastUpdatedAt: number | null; refetch: () => void } {
  const [state, setState] = useState<PollState<T>>({
    data: null,
    error: null,
    loading: path !== null,
    lastUpdatedAt: null,
  });
  const [tick, setTick] = useState(0);
  const busy = useRef(false);

  const refetch = useCallback(() => setTick((t) => t + 1), []);

  useEffect(() => {
    if (!path) {
      setState({ data: null, error: null, loading: false, lastUpdatedAt: null });
      return;
    }
    let alive = true;
    const load = async (initial: boolean) => {
      if (busy.current) return;
      busy.current = true;
      if (initial) setState((s) => ({ ...s, loading: true }));
      try {
        const data = await engineFetch<T>(path);
        // Faz 20 (UIX-6): "sessiz atlama" artık tamamen sessiz değil — son
        // başarılı güncelleme zamanı tutulur; tüketici stale rozeti gösterebilir.
        if (alive) setState({ data, error: null, loading: false, lastUpdatedAt: Date.now() });
      } catch (e) {
        if (alive)
          setState((s) => ({
            data: s.data,
            error: e instanceof Error ? e.message : "Bilinmeyen hata",
            loading: false,
            lastUpdatedAt: s.lastUpdatedAt,
          }));
      } finally {
        busy.current = false;
      }
    };
    void load(true);
    const t = setInterval(() => void load(false), intervalMs);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [path, intervalMs, tick, ...refreshKeys]);

  return { ...state, refetch };
}

/* ------------------------------------------------------------------ */
/* SSE stream store (single shared EventSource, ring buffer of 200)   */
/* ------------------------------------------------------------------ */

export interface StreamEvt {
  id: number;
  type: string;
  payload: Record<string, unknown>;
  at: number;
}

interface EngineStreamState {
  events: StreamEvt[];
  connected: boolean;
  seq: number;
  dataVer: number;
  attach: () => void;
  detach: () => void;
}

const RING_SIZE = 200;
// Faz 18 (UIX-1): FUNDING (pozisyon maliyetini değiştirir), LIQUIDATION
// (pozisyon/bot durumu değişir), WEBHOOK_START (yeni deal açılır) — hepsi
// ekrandaki veriyi bozar; polling'i tetiklemek için dataVer'a katılır.
const DATA_EVENT_TYPES = new Set([
  "FILL",
  "EQUITY",
  "BOT_STATE",
  "ORDER",
  "LOG",
  "FUNDING",
  "LIQUIDATION",
  "WEBHOOK_START",
]);

export const useEngineStream = create<EngineStreamState>((set) => ({
  events: [],
  connected: false,
  seq: 0,
  dataVer: 0,
  attach: () => undefined,
  detach: () => undefined,
}));

let es: EventSource | null = null;
let refCount = 0;
let evtId = 0;

function pushEvent(type: string, payload: Record<string, unknown>) {
  evtId += 1;
  const evt: StreamEvt = { id: evtId, type, payload, at: Date.now() };
  useEngineStream.setState((s) => ({
    events: [evt, ...s.events].slice(0, RING_SIZE),
    seq: s.seq + 1,
    dataVer: DATA_EVENT_TYPES.has(type) ? s.dataVer + 1 : s.dataVer,
  }));
}

function ensureStream() {
  if (es) return;
  try {
    es = new EventSource(engineUrl("/api/stream"));
  } catch {
    es = null;
    useEngineStream.setState({ connected: false });
    return;
  }
  const handle = (type: string) => (e: MessageEvent<string>) => {
    useEngineStream.setState({ connected: true });
    let payload: Record<string, unknown> = {};
    try {
      payload = e.data ? (JSON.parse(e.data) as Record<string, unknown>) : {};
    } catch {
      payload = { raw: e.data };
    }
    if (type === "PING") return;
    pushEvent(type, payload);
  };
  // Faz 18 (UIX-1): motorun yaydığı tüm olay tipleri dinlenir — FUNDING,
  // LIQUIDATION ve WEBHOOK_START eksikti; dinlenmeyen olay ring buffer'a
  // düşmez, ekranda görünmez ve dataVer tetiklenmezdi.
  for (const t of [
    "FILL",
    "EQUITY",
    "LOG",
    "BOT_STATE",
    "ORDER",
    "RISK",
    "FUNDING",
    "LIQUIDATION",
    "WEBHOOK_START",
    "KILL_SWITCH",
    "FREEZE",
    "PING",
  ]) {
    es.addEventListener(t, handle(t) as EventListener);
  }
  es.onopen = () => useEngineStream.setState({ connected: true });
  es.onerror = () => useEngineStream.setState({ connected: false });
}

function teardownStreamIfIdle() {
  if (refCount <= 0 && es) {
    es.close();
    es = null;
    useEngineStream.setState({ connected: false });
  }
}

/** Mount this once per view that needs the live stream. */
export function useEventStream(): {
  events: StreamEvt[];
  connected: boolean;
  seq: number;
  dataVer: number;
} {
  const events = useEngineStream((s) => s.events);
  const connected = useEngineStream((s) => s.connected);
  const seq = useEngineStream((s) => s.seq);
  const dataVer = useEngineStream((s) => s.dataVer);

  useEffect(() => {
    refCount += 1;
    ensureStream();
    return () => {
      refCount -= 1;
      teardownStreamIfIdle();
    };
  }, []);

  return { events, connected, seq, dataVer };
}

/** Debounced value helper (used to throttle SSE-triggered refetches). */
export function useDebouncedValue<T>(value: T, delayMs: number): T {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setDebounced(value), delayMs);
    return () => clearTimeout(t);
  }, [value, delayMs]);
  return debounced;
}

export function useEngineHealth(intervalMs = 10000): {
  health: Health | null;
  error: string | null;
  loading: boolean;
  lastUpdatedAt: number | null;
  refetch: () => void;
} {
  const poll = usePoll<Health>("/api/health", intervalMs);
  return {
    health: poll.data,
    error: poll.error,
    loading: poll.loading,
    lastUpdatedAt: poll.lastUpdatedAt,
    refetch: poll.refetch,
  };
}
