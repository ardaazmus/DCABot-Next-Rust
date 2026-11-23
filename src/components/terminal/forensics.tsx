"use client";

/* Faz 2.4 — Studio "İzler" görünümü (v3 forensics şelale karşılığı).
 *
 * traces modu → bu botun izleri (istemcide botId filtresi) → iz seçimi →
 * olay zaman çizgisi (kökten offset barları). v3'ün alt-sistem span
 * kırılımı bizde YOK — barlar ham olaylardır, uydurma hiyerarşi çizilmez.
 * İzsiz eski olaylar listede görünmez (dürüst not düşülür).
 */

import { useCallback, useEffect, useState } from "react";
import { GitBranch, Loader2 } from "lucide-react";
import { apiForensicsTrace, apiForensicsTraces } from "@/hooks/use-engine";
import { fmtTimeShort } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { ForensicsEvent, ForensicsTrace } from "@/lib/engine-types";

function levelColor(level: string): string {
  return level === "ERROR" ? "bg-rose-400" : level === "WARN" ? "bg-amber-400" : "bg-emerald-400";
}

export function ForensicsTraces({ botId }: { botId: string }) {
  return <ForensicsTracesForBot key={botId} botId={botId} />;
}

function ForensicsTracesForBot({ botId }: { botId: string }) {
  const [traces, setTraces] = useState<ForensicsTrace[] | null>(null);
  const [openId, setOpenId] = useState<string | null>(null);
  const [events, setEvents] = useState<ForensicsEvent[] | null>(null);
  const [eventsBusy, setEventsBusy] = useState(false);
  const [totalMs, setTotalMs] = useState(0);
  const [loadError, setLoadError] = useState<string | null>(null);

  const loadTraces = useCallback(async () => {
    try {
      const r = await apiForensicsTraces(50);
      setTraces((r.traces ?? []).filter((t) => t.botId === botId));
      setLoadError(null);
    } catch (e) {
      setLoadError(e instanceof Error ? e.message : "İzler okunamadı");
    }
  }, [botId]);

  useEffect(() => {
    let alive = true;
    apiForensicsTraces(50)
      .then((r) => {
        if (!alive) return;
        setTraces((r.traces ?? []).filter((t) => t.botId === botId));
        setLoadError(null);
      })
      .catch((e) => {
        if (alive) setLoadError(e instanceof Error ? e.message : "İzler okunamadı");
      });
    return () => {
      alive = false;
    };
  }, [botId]);

  const openTrace = async (id: string) => {
    if (openId === id) {
      setOpenId(null);
      setEvents(null);
      return;
    }
    setOpenId(id);
    setEventsBusy(true);
    try {
      const r = await apiForensicsTrace(id);
      setEvents(r.events ?? []);
      const evs = r.events ?? [];
      setTotalMs(evs.length >= 2 ? Math.max(0, evs[evs.length - 1].ts - evs[0].ts) : 0);
    } catch {
      setEvents([]);
    } finally {
      setEventsBusy(false);
    }
  };

  if (traces === null && !loadError) {
    return (
      <div className="flex items-center gap-2 py-4 text-xs text-slate-500" role="status">
        <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> İzler yükleniyor…
      </div>
    );
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <p className="flex items-center gap-1.5 text-[11px] font-bold uppercase tracking-wider text-slate-400">
          <GitBranch className="h-3.5 w-3.5" aria-hidden /> Tick İzleri
        </p>
        <button onClick={() => void loadTraces()} className="text-[11px] text-cyan-400 hover:text-cyan-300">
          Yenile
        </button>
      </div>
      {loadError && <p className="text-[11px] text-rose-400">{loadError}</p>}
      {traces !== null && traces.length === 0 && (
        <p className="rounded-lg border border-border bg-surface p-3 text-[11px] leading-relaxed text-slate-400">
          Bu botta henüz izli tick yok — izler bot çalışırken her poll&apos;da yazılır (göç-öncesi
          olaylar izsizdir, aşağıdaki kayıtlarda görünür).
        </p>
      )}
      {traces !== null && traces.length > 0 && (
        <ul className="max-h-64 space-y-1.5 overflow-y-auto">
          {traces.map((t) => {
            const open = openId === t.traceId;
            return (
              <li key={t.traceId} className="overflow-hidden rounded-lg border border-border bg-surface">
                <button
                  onClick={() => void openTrace(t.traceId)}
                  aria-expanded={open}
                  className="flex w-full items-center gap-2 px-2.5 py-2 text-left"
                >
                  <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-slate-200">{t.traceId}</span>
                  <span className="num shrink-0 text-[10px] text-slate-400">{t.eventCount} olay</span>
                  {t.errorCount > 0 && (
                    <span className="shrink-0 rounded border border-rose-500/40 bg-rose-500/10 px-1.5 py-0.5 font-mono text-[10px] font-bold text-rose-400">
                      {t.errorCount} hata
                    </span>
                  )}
                  <span className="num shrink-0 text-[10px] text-slate-400">{t.totalMs}ms</span>
                </button>
                {open && (
                  <div className="border-t border-border px-2.5 py-2">
                    {eventsBusy ? (
                      <p className="flex items-center gap-2 py-2 text-[11px] text-slate-400">
                        <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> Olaylar yükleniyor…
                      </p>
                    ) : events !== null && events.length > 0 ? (
                      <ol className="space-y-1">
                        {events.map((e) => {
                          const off = totalMs > 0 ? Math.min(100, Math.max(0, ((e.ts - events[0].ts) / totalMs) * 100)) : 0;
                          return (
                            <li key={e.id} className="flex items-center gap-2 text-[11px]">
                              <span className="num w-14 shrink-0 text-slate-500">{fmtTimeShort(e.ts)}</span>
                              <span className="relative h-3 min-w-0 flex-1 rounded bg-[#1a2233]" aria-hidden>
                                <span
                                  className={cn("absolute top-0 h-3 w-1.5 rounded-full", levelColor(e.level))}
                                  style={{ left: `${off}%` }}
                                />
                              </span>
                              <span className="shrink-0 rounded bg-secondary px-1 font-mono text-[10px] text-cyan-400">{e.gate}</span>
                              <span className="min-w-0 flex-1 truncate text-slate-400" title={e.message}>{e.message}</span>
                            </li>
                          );
                        })}
                      </ol>
                    ) : (
                      <p className="py-1 text-[11px] text-slate-400">Olay yok.</p>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}
