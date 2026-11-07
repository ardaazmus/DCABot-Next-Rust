"use client";

/* Faz 25 (UIX-23): YZ sağlayıcı yapılandırması TEK yerde — Ayarlar.
   Bu store; Kokpit (haber TR özeti), Studio (hafıza özeti) ve Lab (teşhis
   sohbeti) panellerini besler. Üç ekranda da aynı alanları tekrar etmek
   yerine kullanıcı Ayarlar'dan bir kez girer.

   Güvenlik sözleşmesi değişmedi:
   - provider / model / baseUrl → localStorage (anahtar DEĞİL, risksiz tercihler)
   - apiKey → sessionStorage (yalnız bu sekme; diske yazılmaz, yanıta konmaz)
   SSR uyumu: ilk render sunucuyla aynı varsayılanları basar; gerçek değerler
   mount sonrası hydrate() ile gelir (hydration mismatch yok). */

import { useEffect } from "react";
import { create } from "zustand";
import type { AiCallConfig } from "@/lib/engine-types";

const LS_KEY = "dcabot.ai.cfg.v1";
const SS_KEY = "dcabot.ai.key";

interface AiCfgState {
  provider: string;
  model: string;
  baseUrl: string;
  apiKey: string;
  hydrated: boolean;
  hydrate: () => void;
  setProvider: (v: string) => void;
  setModel: (v: string) => void;
  setBaseUrl: (v: string) => void;
  setApiKey: (v: string) => void;
}

function readPersisted(): { provider: string; model: string; baseUrl: string } {
  if (typeof window === "undefined") return { provider: "builtin", model: "", baseUrl: "" };
  try {
    const raw = window.localStorage.getItem(LS_KEY);
    const p = raw ? (JSON.parse(raw) as Partial<{ provider: string; model: string; baseUrl: string }>) : {};
    return {
      provider: typeof p.provider === "string" ? p.provider : "builtin",
      model: typeof p.model === "string" ? p.model : "",
      baseUrl: typeof p.baseUrl === "string" ? p.baseUrl : "",
    };
  } catch {
    return { provider: "builtin", model: "", baseUrl: "" };
  }
}

function writePersisted(s: { provider: string; model: string; baseUrl: string }) {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(LS_KEY, JSON.stringify(s));
  } catch {
    /* storage kapalı olabilir — tercihler oturumluk kalır */
  }
}

export const useAiConfigStore = create<AiCfgState>((set) => ({
  provider: "builtin",
  model: "",
  baseUrl: "",
  apiKey: "",
  hydrated: false,
  hydrate: () =>
    set(() => {
      const p = readPersisted();
      let key = "";
      if (typeof window !== "undefined") {
        try {
          key = window.sessionStorage.getItem(SS_KEY) ?? "";
        } catch {
          key = "";
        }
      }
      return { ...p, apiKey: key, hydrated: true };
    }),
  setProvider: (v) =>
    set((s) => {
      writePersisted({ provider: v, model: s.model, baseUrl: s.baseUrl });
      return { provider: v };
    }),
  setModel: (v) =>
    set((s) => {
      writePersisted({ provider: s.provider, model: v, baseUrl: s.baseUrl });
      return { model: v };
    }),
  setBaseUrl: (v) =>
    set((s) => {
      writePersisted({ provider: s.provider, model: s.model, baseUrl: v });
      return { baseUrl: v };
    }),
  setApiKey: (v) =>
    set(() => {
      if (typeof window !== "undefined") {
        try {
          if (v) window.sessionStorage.setItem(SS_KEY, v);
          else window.sessionStorage.removeItem(SS_KEY);
        } catch {
          /* sessionStorage kapalı olabilir — anahtar yalnız bellekte kalır */
        }
      }
      return { apiKey: v };
    }),
}));

export function useAiConfig(): {
  provider: string;
  model: string;
  baseUrl: string;
  apiKey: string;
  setProvider: (v: string) => void;
  setModel: (v: string) => void;
  setBaseUrl: (v: string) => void;
  setApiKey: (v: string) => void;
  config: AiCallConfig;
} {
  const provider = useAiConfigStore((s) => s.provider);
  const model = useAiConfigStore((s) => s.model);
  const baseUrl = useAiConfigStore((s) => s.baseUrl);
  const apiKey = useAiConfigStore((s) => s.apiKey);
  const hydrated = useAiConfigStore((s) => s.hydrated);
  const hydrate = useAiConfigStore((s) => s.hydrate);
  const setProvider = useAiConfigStore((s) => s.setProvider);
  const setModel = useAiConfigStore((s) => s.setModel);
  const setBaseUrl = useAiConfigStore((s) => s.setBaseUrl);
  const setApiKey = useAiConfigStore((s) => s.setApiKey);

  useEffect(() => {
    if (!hydrated) hydrate();
  }, [hydrated, hydrate]);

  return {
    provider,
    model,
    baseUrl,
    apiKey,
    setProvider,
    setModel,
    setBaseUrl,
    setApiKey,
    config: {
      provider,
      model: model.trim() || undefined,
      apiKey: apiKey || undefined,
      baseUrl: baseUrl.trim() || undefined,
    },
  };
}
