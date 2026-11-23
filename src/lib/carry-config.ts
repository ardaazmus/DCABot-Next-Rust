"use client";

import { useMemo, useSyncExternalStore } from "react";

/**
 * Faz 19 (UIX-11): Lab ↔ Sihirbaz config taşıma.
 *
 * Backtest Lab'de beğenilen koşu ayarları tek tıkla Sihirbaz'a taşınır:
 * `saveCarriedConfig` session-storage'a yazar, Sihirbaz mount'ta
 * `consumeCarriedConfig` ile TÜKETİR (okuma = silme; bayat config sürpriz
 * prefilleme yapmasın — 10 dakika tazelik penceresi).
 */

export interface CarriedBotConfig {
  source: "lab";
  ts: number;
  kind: string;
  symbol: string;
  interval: string;
  venue: string;
  startCash: string;
  /** Lab'in buildConfig() çıktısı — motorun fraksiyon sözleşmeli alanları. */
  config: Record<string, unknown>;
}

const KEY = "dcabot.carry";
const FRESH_MS = 10 * 60 * 1000;

export function saveCarriedConfig(c: Omit<CarriedBotConfig, "ts">): boolean {
  try {
    window.sessionStorage.setItem(KEY, JSON.stringify({ ...c, ts: Date.now() }));
    return true;
  } catch {
    return false;
  }
}

export function peekCarriedConfig(): CarriedBotConfig | null {
  try {
    const raw = window.sessionStorage.getItem(KEY);
    if (!raw) return null;
    const v = JSON.parse(raw) as CarriedBotConfig;
    if (!v || v.source !== "lab" || typeof v.ts !== "number") return null;
    return v;
  } catch {
    return null;
  }
}

/** Okur VE siler — taşıma tek seferliktir. Bayat (10 dk+) kayıt düşürülür. */
export function consumeCarriedConfig(): CarriedBotConfig | null {
  const v = peekCarriedConfig();
  try {
    window.sessionStorage.removeItem(KEY);
  } catch {
    /* ignore */
  }
  if (!v || Date.now() - v.ts > FRESH_MS) return null;
  return v;
}
function createCarriedConfigStore() {
  let snapshot: CarriedBotConfig | null = null;
  let initialized = false;
  const listeners = new Set<() => void>();
  return {
    subscribe(listener: () => void) {
      listeners.add(listener);
      if (!initialized) {
        snapshot = consumeCarriedConfig();
        initialized = true;
      }
      return () => { listeners.delete(listener); };
    },
    getSnapshot: () => snapshot,
    getServerSnapshot: () => null,
  };
}

/** Read one-time session storage input as an external store after hydration. */
export function useCarriedConfig() {
  const store = useMemo(() => createCarriedConfigStore(), []);
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getServerSnapshot);
}
