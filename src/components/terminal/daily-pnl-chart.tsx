"use client";

import { useEffect, useMemo, useState } from "react";
import { BarChart3, Calendar, CheckCircle2, TrendingDown, TrendingUp, X } from "lucide-react";
import { Panel } from "@/components/terminal/bits";
import { apiDailyPnl, type DailyPnlBar, type DailyPnlResponse } from "@/hooks/use-engine";
import { fmtUsd, fmtUsdSigned } from "@/lib/format";
import { cn } from "@/lib/utils";

interface DailyPnlChartProps {
  selectedDate: string | null;
  onSelectDate: (date: string | null) => void;
  className?: string;
}

function formatDayLabel(dateStr: string): string {
  // "2026-11-06" -> "06 Kas" or "11-06"
  const parts = dateStr.split("-");
  if (parts.length < 3) return dateStr;
  const months = ["Oca", "Şub", "Mar", "Nis", "May", "Haz", "Tem", "Ağu", "Eyl", "Eki", "Kas", "Ara"];
  const mIdx = parseInt(parts[1], 10) - 1;
  const day = parts[2];
  if (mIdx >= 0 && mIdx < 12) {
    return `${day} ${months[mIdx]}`;
  }
  return `${parts[1]}-${parts[2]}`;
}

export function DailyPnlChart({ selectedDate, onSelectDate, className }: DailyPnlChartProps) {
  const [daysWindow, setDaysWindow] = useState<7 | 14 | 30>(14);
  const [data, setData] = useState<DailyPnlResponse | null>(null);
  const [loading, setLoading] = useState(true);
  const [hoveredDate, setHoveredDate] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    apiDailyPnl(daysWindow)
      .then((res) => {
        if (alive) {
          setData(res);
          setLoading(false);
        }
      })
      .catch(() => {
        if (alive) {
          setData(null);
          setLoading(false);
        }
      });
    return () => {
      alive = false;
    };
  }, [daysWindow]);

  const bars = data?.days ?? [];
  const summary = data?.summary;

  const { maxAbsVal } = useMemo(() => {
    if (bars.length === 0) return { maxAbsVal: 10 };
    const maxVal = Math.max(...bars.map((b) => Math.abs(b.netPnl)));
    return { maxAbsVal: maxVal > 0 ? maxVal * 1.15 : 10 };
  }, [bars]);

  const hoveredBar = useMemo(
    () => (hoveredDate ? bars.find((b) => b.date === hoveredDate) ?? null : null),
    [hoveredDate, bars]
  );

  return (
    <Panel
      className={className}
      title="Gün Bazlı Kâr / Zarar Dağılımı"
      icon={<BarChart3 className="h-4 w-4 text-emerald-400" aria-hidden />}
      right={
        <div className="flex items-center gap-2">
          {selectedDate && (
            <button
              type="button"
              onClick={() => onSelectDate(null)}
              className="inline-flex items-center gap-1 rounded-md bg-cyan-500/15 px-2 py-0.5 text-[10px] font-semibold text-cyan-300 transition-colors hover:bg-cyan-500/25"
              title="Filtreyi kaldır"
            >
              <span>Filtre: {formatDayLabel(selectedDate)}</span>
              <X className="h-3 w-3" aria-hidden />
            </button>
          )}
          <div className="flex rounded-lg border border-border p-0.5 text-[10px]">
            {([7, 14, 30] as const).map((w) => (
              <button
                key={w}
                type="button"
                onClick={() => setDaysWindow(w)}
                className={cn(
                  "rounded px-2.5 py-0.5 font-semibold transition-colors",
                  daysWindow === w ? "bg-secondary text-slate-100" : "text-slate-400 hover:text-slate-200"
                )}
              >
                {w}G
              </button>
            ))}
          </div>
        </div>
      }
    >
      {/* Summary KPI Strip */}
      <div className="mb-4 grid grid-cols-2 gap-2 sm:grid-cols-5 border-b border-border/60 pb-3 text-xs">
        <div>
          <span className="text-[10px] uppercase tracking-wider text-slate-500">Pencere Net Kâr</span>
          <div
            className={cn(
              "num text-base font-bold",
              (summary?.totalNetPnl ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400"
            )}
          >
            {fmtUsdSigned(summary?.totalNetPnl ?? 0)}
          </div>
        </div>
        <div>
          <span className="text-[10px] uppercase tracking-wider text-slate-500">Kapanan İşlem</span>
          <div className="num text-base font-bold text-slate-200">
            {summary?.totalDeals ?? 0} <span className="text-xs font-normal text-slate-500">deal</span>
          </div>
        </div>
        <div>
          <span className="text-[10px] uppercase tracking-wider text-slate-500">Kârlı / Zararlı Gün</span>
          <div className="num text-base font-bold text-slate-200">
            <span className="text-emerald-400">{summary?.profitableDays ?? 0} gün</span>
            <span className="text-slate-500"> / </span>
            <span className="text-rose-400">{summary?.unprofitableDays ?? 0} gün</span>
          </div>
        </div>
        <div>
          <span className="text-[10px] uppercase tracking-wider text-slate-500">Profit Factor (PF)</span>
          <div className="num text-base font-bold">
            {summary?.profitFactor != null ? (
              <span
                className={cn(
                  summary.profitFactor >= 1.5
                    ? "text-emerald-400"
                    : summary.profitFactor >= 1.0
                      ? "text-cyan-400"
                      : "text-rose-400"
                )}
                title="Profit Factor: seçili dönemdeki net kârlı işlem toplamı / net zararlı işlem toplamı"
              >
                {summary.profitFactor > 99 ? "∞" : summary.profitFactor.toFixed(2)}
              </span>
            ) : (
              <span className="text-slate-500">—</span>
            )}
          </div>
        </div>
        <div>
          <span className="text-[10px] uppercase tracking-wider text-slate-500">En İyi Gün</span>
          <div className="num text-base font-bold text-emerald-400">
            {summary?.bestDay ? (
              <span title={`${summary.bestDay.date} (${summary.bestDay.count} deal)`}>
                +{fmtUsd(summary.bestDay.netPnl)}{" "}
                <span className="text-[10px] font-normal text-slate-400">
                  ({formatDayLabel(summary.bestDay.date)})
                </span>
              </span>
            ) : (
              <span className="text-slate-500">—</span>
            )}
          </div>
        </div>
      </div>

      {/* Bar Chart Container */}
      {loading && !data ? (
        <div className="flex h-44 items-center justify-center text-xs text-slate-500">
          Günlük veriler yükleniyor…
        </div>
      ) : bars.length === 0 ? (
        <div className="flex h-44 items-center justify-center text-xs text-slate-500">
          Seçili aralıkta tamamlanan işlem yok.
        </div>
      ) : (
        <div className="relative">
          {/* Tooltip Hover Overlay */}
          {hoveredBar && (
            <div className="pointer-events-none absolute -top-9 left-1/2 -translate-x-1/2 z-30 flex items-center gap-2 rounded-lg border border-border bg-[#18202e] px-3 py-1.5 text-xs shadow-xl backdrop-blur-md">
              <span className="font-semibold text-slate-200">
                {formatDayLabel(hoveredBar.date)} ({hoveredBar.date}):
              </span>
              <span
                className={cn(
                  "num font-bold",
                  hoveredBar.netPnl >= 0 ? "text-emerald-400" : "text-rose-400"
                )}
              >
                {fmtUsdSigned(hoveredBar.netPnl)}
              </span>
              <span className="text-slate-400">
                · {hoveredBar.count} deal ({hoveredBar.wins}W / {hoveredBar.losses}L)
              </span>
              {hoveredBar.fees > 0 && (
                <span className="num text-slate-500">· komisyon: {fmtUsd(hoveredBar.fees, 4)}</span>
              )}
            </div>
          )}

          {/* Precision Financial Bar Chart: Two-Zone Grid */}
          <div className="flex h-40 w-full items-stretch gap-1 sm:gap-1.5">
            {bars.map((bar) => {
              const isSelected = selectedDate === bar.date;
              const isHovered = hoveredDate === bar.date;
              const isPositive = bar.netPnl >= 0;
              const hasActivity = bar.count > 0;
              const barHeightPct =
                hasActivity && maxAbsVal > 0
                  ? Math.min(100, Math.max(8, (Math.abs(bar.netPnl) / maxAbsVal) * 100))
                  : 0;

              return (
                <button
                  key={bar.date}
                  type="button"
                  onClick={() => onSelectDate(isSelected ? null : bar.date)}
                  onMouseEnter={() => setHoveredDate(bar.date)}
                  onMouseLeave={() => setHoveredDate(null)}
                  className={cn(
                    "group relative flex flex-1 flex-col items-center rounded-lg p-0.5 transition-all outline-none",
                    isSelected
                      ? "bg-cyan-500/10 ring-1 ring-cyan-400"
                      : isHovered
                        ? "bg-surface-2/80"
                        : "hover:bg-surface/50"
                  )}
                  title={`${bar.date}: ${fmtUsdSigned(bar.netPnl)} (${bar.count} deal)`}
                >
                  {/* Upper Zone (Positive Net PnL) */}
                  <div className="relative flex h-[62px] w-full flex-col justify-end items-center">
                    {hasActivity && isPositive && (
                      <>
                        <span className="num mb-1 text-[9px] font-semibold text-emerald-400 opacity-90 transition-transform group-hover:scale-110">
                          {fmtUsdSigned(bar.netPnl, 1)}
                        </span>
                        <div
                          style={{ height: `${barHeightPct}%` }}
                          className={cn(
                            "w-full max-w-[28px] rounded-t transition-all",
                            isSelected
                              ? "bg-emerald-300 shadow-[0_0_8px_rgba(52,211,153,0.5)]"
                              : "bg-emerald-500/90 hover:bg-emerald-400"
                          )}
                        />
                      </>
                    )}
                  </div>

                  {/* Zero Baseline Divider */}
                  <div className="relative flex w-full items-center justify-center py-0.5">
                    <div
                      className={cn(
                        "h-[1.5px] w-full transition-colors",
                        hasActivity
                          ? isPositive
                            ? "bg-emerald-500/60"
                            : "bg-rose-500/60"
                          : "bg-slate-700/60"
                      )}
                    />
                    {hasActivity && (
                      <span
                        className={cn(
                          "absolute h-1.5 w-1.5 rounded-full ring-2 ring-[#151a24]",
                          isPositive ? "bg-emerald-400" : "bg-rose-400"
                        )}
                      />
                    )}
                  </div>

                  {/* Lower Zone (Negative Net PnL) */}
                  <div className="relative flex h-[62px] w-full flex-col justify-start items-center">
                    {hasActivity && !isPositive && (
                      <>
                        <div
                          style={{ height: `${barHeightPct}%` }}
                          className={cn(
                            "w-full max-w-[28px] rounded-b transition-all",
                            isSelected
                              ? "bg-rose-300 shadow-[0_0_8px_rgba(251,113,133,0.5)]"
                              : "bg-rose-500/90 hover:bg-rose-400"
                          )}
                        />
                        <span className="num mt-1 text-[9px] font-semibold text-rose-400 opacity-90 transition-transform group-hover:scale-110">
                          {fmtUsdSigned(bar.netPnl, 1)}
                        </span>
                      </>
                    )}
                  </div>

                  {/* Date Label */}
                  <div
                    className={cn(
                      "mt-1 num text-[10px] font-mono leading-none select-none transition-colors",
                      isSelected
                        ? "font-bold text-cyan-300"
                        : hasActivity
                          ? "text-slate-300 font-medium"
                          : "text-slate-600"
                    )}
                  >
                    {formatDayLabel(bar.date)}
                  </div>
                </button>
              );
            })}
          </div>

          {/* Footer Guide Strip */}
          <div className="mt-3 flex flex-wrap items-center justify-between border-t border-border/40 pt-2 text-[11px] text-slate-500">
            <span className="flex items-center gap-1.5">
              <span>Bara tıklayarak ilgili günün işlemlerini alttaki defterde filtreleyebilirsiniz.</span>
            </span>
            <div className="flex items-center gap-3">
              <span className="flex items-center gap-1.5">
                <span className="h-2 w-2 rounded-sm bg-emerald-500" />
                <span className="text-slate-400">Kârlı Gün</span>
              </span>
              <span className="flex items-center gap-1.5">
                <span className="h-2 w-2 rounded-sm bg-rose-500" />
                <span className="text-slate-400">Zararlı Gün</span>
              </span>
              <span className="flex items-center gap-1.5">
                <span className="h-1.5 w-1.5 rounded-full bg-slate-700" />
                <span className="text-slate-500">İşlemsiz</span>
              </span>
            </div>
          </div>
        </div>
      )}
    </Panel>
  );
}
