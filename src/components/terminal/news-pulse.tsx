"use client";

import { useEffect, useState } from "react";
import { Info, Newspaper } from "lucide-react";
import { EmptyState, Panel, Spinner } from "@/components/terminal/bits";
import {
  apiAiProviders,
  apiNews,
  apiNewsAi,
  useAiConfig,
} from "@/hooks/use-engine";
import type { AiProvider, NewsAiResult, NewsResult } from "@/lib/engine-types";
import { fmtTimeShort } from "@/lib/format";
import { cn } from "@/lib/utils";

/**
 * Borsa Duyuruları — açılış (Kokpit) haber şeridi.
 * Faz 25 (UIX-36): başlık "Piyasa Nabzı · BTCUSDT"ti ama liste borsa-geneli
 * duyuruydu — çelişki giderildi. Sentiment çipine açıklama tooltip'i eklendi.
 * Faz 25 (UIX-23/35): sağlayıcı yapılandırması Ayarlar'da tek yer; TR özet
 * düğmeleri yalnız kullanılabilir bir sağlayıcı seçiliyken görünür.
 * Salt-okunurdur; emir yetkisi yoktur.
 */
export function NewsPulse({ symbol }: { symbol: string }) {
  const [news, setNews] = useState<NewsResult | null>(null);
  const [failed, setFailed] = useState(false);
  const [aiSums, setAiSums] = useState<Record<string, NewsAiResult>>({});
  const [aiSumBusy, setAiSumBusy] = useState<string | null>(null);
  const [providers, setProviders] = useState<AiProvider[]>([]);
  const ai = useAiConfig();

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

  const aiUsable = ai.provider !== "builtin" || providers.some((p) => p.id === "builtin" && p.needsKey === false && (p.models?.length ?? 0) > 0);

  const runNewsAi = async (id: string) => {
    setAiSumBusy(id);
    try {
      const r = await apiNewsAi(id, ai.config);
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
      title="Borsa Duyuruları"
      icon={<Newspaper className="h-4 w-4 text-amber-400" aria-hidden />}
      right={
        news ? (
          <span className="flex items-center gap-2">
            <span
              title={`Son haber penceresinin bileşik duyarlılık skoru (−1..+1): ${news.bull} olumlu, ${news.bear} olumsuz haber`}
              className={cn("num inline-flex items-center gap-1 rounded px-1.5 py-0.5 font-mono text-[11px] font-bold", meanTone)}
            >
              {symbol} duyarlılık {(mean ?? 0) > 0 ? `+${mean}` : mean} ({news.bull}↑ {news.bear}↓)
              <Info className="h-3 w-3 opacity-60" aria-hidden />
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
          <Spinner /> <span className="text-xs text-slate-500">Duyurular yükleniyor…</span>
        </div>
      ) : !news || news.count === 0 ? (
        <EmptyState
          icon={Newspaper}
          title={failed ? "Haber servisine ulaşılamadı" : "Duyuru yok"}
          hint={
            failed
              ? "Motor haber uçlarına erişemedi — yürütüm etkilenmez, 5 dk sonra otomatik tekrar denenecek."
              : `Son pencerede borsa duyurusu bulunamadı.`
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
                  {/* Faz 25 (UIX-35): sağlayıcı yoksa ölü TR düğmesi yerine
                      tek seferlik yönlendirme satırı. */}
                  {aiUsable ? (
                    <button
                      type="button"
                      onClick={() => void runNewsAi(n.id)}
                      disabled={aiSumBusy === n.id}
                      title="Tek cümlelik Türkçe özet üret (öğe başına bir kez, önbellekli)"
                      className="shrink-0 rounded border border-violet-500/40 bg-violet-500/10 px-1.5 py-0.5 text-[10px] font-bold text-violet-300 transition-colors hover:bg-violet-500/20 disabled:opacity-50"
                    >
                      {aiSumBusy === n.id ? "…" : n.aiSummary || aiSums[n.id]?.summaryTr ? "TR ✓" : "ÖZET (TR)"}
                    </button>
                  ) : null}
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
          {!aiUsable && (
            <p className="mt-2 text-[10px] text-slate-500">
              Türkçe özetler için Ayarlar → YZ Sağlayıcısı&apos;ndan bir sağlayıcı seçin.
            </p>
          )}
        </>
      )}
    </Panel>
  );
}
