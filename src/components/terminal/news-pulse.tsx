"use client";

import { useEffect, useMemo, useState } from "react";
import { Newspaper } from "lucide-react";
import { EmptyState, Panel, Spinner } from "@/components/terminal/bits";
import {
  apiAiProviders,
  apiNews,
  apiNewsAi,
} from "@/hooks/use-engine";
import type { AiCallConfig, AiProvider, NewsAiResult, NewsResult } from "@/lib/engine-types";
import { fmtTimeShort } from "@/lib/format";
import { cn } from "@/lib/utils";

/**
 * Piyasa Nabzı — açılışta (Dashboard) haber + sentiment şeridi.
 * v3'teki başlangıç görünümündeki haber/sentiment kartlarının karşılığı:
 * motorun /api/market/news ucu (duyarlılık [-1,+1] + delisting kapısı, 5 dk
 * önbellek) + öğe başına tek-cümlelik TR özet (/api/market/news-ai).
 * Salt-okunurdur; emir yetkisi yoktur. YZ anahtarı sekme-içi state'te yaşar,
 * diske yazılmaz.
 */
export function NewsPulse({ symbol }: { symbol: string }) {
  const [news, setNews] = useState<NewsResult | null>(null);
  const [failed, setFailed] = useState(false);
  const [aiSums, setAiSums] = useState<Record<string, NewsAiResult>>({});
  const [aiSumBusy, setAiSumBusy] = useState<string | null>(null);
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const [provider, setProvider] = useState("builtin");
  const [model, setModel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [baseUrl, setBaseUrl] = useState("");

  useEffect(() => {
    let alive = true;
    setFailed(false);
    const load = () => {
      apiNews(symbol)
        .then((r) => {
          if (alive) {
            setNews(r);
            setFailed(false);
          }
        })
        .catch(() => {
          if (alive) {
            setNews(null);
            setFailed(true);
          }
        });
    };
    load();
    const t = setInterval(load, 300000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [symbol]);

  useEffect(() => {
    let alive = true;
    apiAiProviders()
      .then((r) => {
        if (alive) setProviders(r.providers);
      })
      .catch(() => {
        if (alive) setProviders([]);
      });
    return () => {
      alive = false;
    };
  }, []);

  const aiConfig = useMemo<AiCallConfig>(
    () => ({
      provider,
      model: model.trim() || undefined,
      apiKey: apiKey || undefined,
      baseUrl: baseUrl.trim() || undefined,
    }),
    [provider, model, apiKey, baseUrl],
  );

  const runNewsAi = async (id: string) => {
    setAiSumBusy(id);
    try {
      const r = await apiNewsAi(id, aiConfig);
      setAiSums((m) => ({ ...m, [id]: r }));
    } catch (e) {
      setAiSums((m) => ({
        ...m,
        [id]: { ok: false, summaryTr: null, reason: e instanceof Error ? e.message : "Özet alınamadı", cached: false },
      }));
    } finally {
      setAiSumBusy(null);
    }
  };

  const mean = news?.mean ?? null;
  const meanTone =
    mean === null || Math.abs(mean) <= 0.2
      ? "bg-slate-500/15 text-slate-300"
      : mean > 0
        ? "bg-emerald-500/15 text-emerald-300"
        : "bg-rose-500/15 text-rose-300";

  return (
    <Panel
      title={`Piyasa Nabzı · ${symbol}`}
      icon={<Newspaper className="h-4 w-4 text-amber-400" aria-hidden />}
      right={
        news ? (
          <span className="flex items-center gap-2">
            <span className={cn("num rounded px-1.5 py-0.5 font-mono text-[11px] font-bold", meanTone)}>
              {(mean ?? 0) > 0 ? `+${mean}` : mean} ({news.bull}↑ {news.bear}↓)
            </span>
            <span className="text-[10px] text-slate-400">
              {news.sources.join(" · ")}
              {news.cached ? " · önbellek" : ""}
            </span>
          </span>
        ) : undefined
      }
    >
      {!news && !failed ? (
        <div className="flex items-center gap-2 py-4">
          <Spinner /> <span className="text-xs text-slate-500">Haberler yükleniyor…</span>
        </div>
      ) : !news || news.count === 0 ? (
        <EmptyState
          icon={Newspaper}
          title={failed ? "Haber servisine ulaşılamadı" : "Haber yok"}
          hint={
            failed
              ? "Motor haber uçlarına erişemedi — yürütüm etkilenmez, 5 dk sonra otomatik tekrar denenecek."
              : `${symbol} için son pencerede haber bulunamadı.`
          }
        />
      ) : (
        <>
          {news.delisting.active && (
            <p className="mb-2 rounded border border-rose-500/40 bg-rose-500/10 px-2 py-1 text-[11px] font-bold text-rose-300">
              DELISTING UYARISI: {news.delisting.headline}
            </p>
          )}
          <ul className="thin-scroll max-h-56 space-y-1 overflow-y-auto pr-1">
            {news.items.slice(0, 8).map((n) => (
              <li key={n.id}>
                <div className="flex items-start gap-2 text-[11px]">
                  <span
                    className={cn(
                      "mt-0.5 shrink-0 rounded px-1 py-0.5 font-mono text-[10px] font-bold",
                      n.sentiment > 0.2
                        ? "bg-emerald-500/15 text-emerald-300"
                        : n.sentiment < -0.2
                          ? "bg-rose-500/15 text-rose-300"
                          : "bg-slate-500/15 text-slate-400",
                    )}
                  >
                    {n.sentiment > 0 ? `+${n.sentiment}` : n.sentiment}
                  </span>
                  <a
                    href={n.url}
                    target="_blank"
                    rel="noopener noreferrer"
                    className="min-w-0 flex-1 truncate text-slate-300 hover:text-cyan-300 hover:underline"
                    title={`${n.title} — ${n.source}`}
                  >
                    {n.title}
                  </a>
                  <button
                    type="button"
                    onClick={() => void runNewsAi(n.id)}
                    disabled={aiSumBusy === n.id}
                    title="Tek cümlelik Türkçe özet üret (öğe başına bir kez, önbellekli)"
                    className="shrink-0 rounded border border-violet-500/40 bg-violet-500/10 px-1.5 py-0.5 text-[10px] font-bold text-violet-300 transition-colors hover:bg-violet-500/20 disabled:opacity-50"
                  >
                    {aiSumBusy === n.id ? "…" : n.aiSummary || aiSums[n.id]?.summaryTr ? "TR ✓" : "TR"}
                  </button>
                  <span className="hidden shrink-0 text-[10px] text-slate-400 sm:inline">
                    {n.source} · {fmtTimeShort(n.publishedAt)}
                  </span>
                </div>
                {(n.aiSummary || aiSums[n.id]?.summaryTr) && (
                  <p className="ml-12 mt-0.5 text-[11px] leading-snug text-violet-200/90">
                    <span className="mr-1 rounded bg-violet-500/15 px-1 text-[9px] font-bold text-violet-300">
                      AI ÖZETİ
                    </span>
                    {n.aiSummary || aiSums[n.id]?.summaryTr}
                  </p>
                )}
                {aiSums[n.id] && !aiSums[n.id].ok && !aiSums[n.id].summaryTr && (
                  <p className="ml-12 mt-0.5 text-[10px] text-slate-400">{aiSums[n.id].reason}</p>
                )}
              </li>
            ))}
          </ul>
          <details className="mt-2 rounded-lg border border-border bg-surface p-2">
            <summary className="cursor-pointer text-[11px] font-semibold text-slate-400 hover:text-slate-200">
              AI özet sağlayıcısı (isteğe bağlı — anahtar sekmeden çıkmaz)
            </summary>
            <div className="mt-2 grid gap-2 sm:grid-cols-2">
              <label className="block text-[11px] text-slate-400">
                Sağlayıcı
                <select
                  value={provider}
                  onChange={(e) => setProvider(e.target.value)}
                  className="mt-1 min-h-9 w-full rounded-md border border-border-strong bg-[#151a24] px-2 text-xs text-slate-200"
                >
                  {(providers.length > 0
                    ? providers
                    : [{ id: "builtin", label: "Yerleşik (paketlenmedi)" } as AiProvider]
                  ).map((p) => (
                    <option key={p.id} value={p.id}>
                      {p.label}
                    </option>
                  ))}
                </select>
              </label>
              <label className="block text-[11px] text-slate-400">
                Model (boş = varsayılan)
                <input
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder="örn. gpt-4o-mini"
                  className="mt-1 min-h-9 w-full rounded-md border border-border-strong bg-[#151a24] px-2 text-xs text-slate-200"
                />
              </label>
              <label className="block text-[11px] text-slate-400">
                API anahtarı (yalnız bu istekte gider)
                <input
                  type="password"
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
                  placeholder="sk-…"
                  autoComplete="off"
                  className="mt-1 min-h-9 w-full rounded-md border border-border-strong bg-[#151a24] px-2 text-xs text-slate-200"
                />
              </label>
              <label className="block text-[11px] text-slate-400">
                Base URL (boş = varsayılan, Ollama için gerekli)
                <input
                  value={baseUrl}
                  onChange={(e) => setBaseUrl(e.target.value)}
                  placeholder="http://localhost:11434/v1"
                  className="mt-1 min-h-9 w-full rounded-md border border-border-strong bg-[#151a24] px-2 text-xs text-slate-200"
                />
              </label>
            </div>
          </details>
        </>
      )}
    </Panel>
  );
}
