"use client";

import { useEffect, useMemo, useState } from "react";
import {
  Calendar,
  CheckCircle2,
  DollarSign,
  Filter,
  History,
  RotateCcw,
  Search,
  TrendingDown,
  TrendingUp,
  X,
  XCircle,
} from "lucide-react";
import { EmptyState, KindBadge, ModeBadge, Panel, Spinner, VenueChip } from "@/components/terminal/bits";
import { apiDeals, type ClosedDeal, type DealsResponse } from "@/hooks/use-engine";
import { fmtFrac, fmtTimeShort, fmtUsd, fmtUsdSigned } from "@/lib/format";
import { cn } from "@/lib/utils";

interface ClosedDealsProps {
  selectedDate: string | null;
  onClearDate: () => void;
  className?: string;
}

type QuickDateFilter = "all" | "today" | "yesterday" | "7d" | "30d" | "custom";

function getTodayStr(): string {
  const d = new Date();
  return d.toISOString().slice(0, 10);
}

function getYesterdayStr(): string {
  const d = new Date();
  d.setDate(d.getDate() - 1);
  return d.toISOString().slice(0, 10);
}

export function ClosedDealsLedger({ selectedDate, onClearDate, className }: ClosedDealsProps) {
  const [quickFilter, setQuickFilter] = useState<QuickDateFilter>("all");
  const [customDate, setCustomDate] = useState<string>("");
  const [searchQuery, setSearchQuery] = useState<string>("");
  const [data, setData] = useState<DealsResponse | null>(null);
  const [loading, setLoading] = useState(true);

  // Compute active date parameter for API
  const activeParams = useMemo(() => {
    if (selectedDate) {
      return { date: selectedDate };
    }
    if (quickFilter === "today") {
      return { date: getTodayStr() };
    }
    if (quickFilter === "yesterday") {
      return { date: getYesterdayStr() };
    }
    if (quickFilter === "7d") {
      return { days: 7 };
    }
    if (quickFilter === "30d") {
      return { days: 30 };
    }
    if (quickFilter === "custom" && customDate) {
      return { date: customDate };
    }
    return {};
  }, [selectedDate, quickFilter, customDate]);

  useEffect(() => {
    let alive = true;
    apiDeals({
      ...activeParams,
      q: searchQuery.trim() || undefined,
      limit: 300,
    })
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
  }, [activeParams, searchQuery]);

  const deals = data?.deals ?? [];
  const summary = data?.summary;

  const handleSelectQuick = (filter: QuickDateFilter) => {
    onClearDate();
    setQuickFilter(filter);
    if (filter !== "custom") {
      setCustomDate("");
    }
  };

  const handleClearAllFilters = () => {
    onClearDate();
    setQuickFilter("all");
    setCustomDate("");
    setSearchQuery("");
  };

  const hasActiveFilters =
    quickFilter !== "all" || !!selectedDate || !!customDate || searchQuery.trim().length > 0;

  return (
    <Panel
      className={className}
      title="Kapanan İşlemler Defteri (Closed Deals)"
      icon={<History className="h-4 w-4 text-cyan-400" aria-hidden />}
      right={
        <span className="num text-xs text-slate-400">
          Toplam: <span className="font-semibold text-slate-200">{data?.total ?? 0}</span> işlem
        </span>
      }
    >
      {/* Search and Date Filter Controls */}
      <div className="mb-4 space-y-2.5 border-b border-border/60 pb-3">
        <div className="flex flex-wrap items-center justify-between gap-2.5">
          {/* Quick Date Pills */}
          <div className="flex flex-wrap items-center gap-1 rounded-lg border border-border p-0.5 text-[11px]">
            <button
              type="button"
              onClick={() => handleSelectQuick("all")}
              className={cn(
                "rounded px-2.5 py-1 font-semibold transition-colors",
                quickFilter === "all" && !selectedDate
                  ? "bg-secondary text-slate-100"
                  : "text-slate-400 hover:text-slate-200"
              )}
            >
              Tümü
            </button>
            <button
              type="button"
              onClick={() => handleSelectQuick("today")}
              className={cn(
                "rounded px-2.5 py-1 font-semibold transition-colors",
                quickFilter === "today" && !selectedDate
                  ? "bg-secondary text-slate-100"
                  : "text-slate-400 hover:text-slate-200"
              )}
            >
              Bugün
            </button>
            <button
              type="button"
              onClick={() => handleSelectQuick("yesterday")}
              className={cn(
                "rounded px-2.5 py-1 font-semibold transition-colors",
                quickFilter === "yesterday" && !selectedDate
                  ? "bg-secondary text-slate-100"
                  : "text-slate-400 hover:text-slate-200"
              )}
            >
              Dün
            </button>
            <button
              type="button"
              onClick={() => handleSelectQuick("7d")}
              className={cn(
                "rounded px-2.5 py-1 font-semibold transition-colors",
                quickFilter === "7d" && !selectedDate
                  ? "bg-secondary text-slate-100"
                  : "text-slate-400 hover:text-slate-200"
              )}
            >
              Son 7 Gün
            </button>
            <button
              type="button"
              onClick={() => handleSelectQuick("30d")}
              className={cn(
                "rounded px-2.5 py-1 font-semibold transition-colors",
                quickFilter === "30d" && !selectedDate
                  ? "bg-secondary text-slate-100"
                  : "text-slate-400 hover:text-slate-200"
              )}
            >
              Son 30 Gün
            </button>
            {selectedDate && (
              <span className="inline-flex items-center gap-1.5 rounded bg-cyan-500/20 px-2 py-0.5 font-semibold text-cyan-300">
                <span>{selectedDate}</span>
                <button
                  type="button"
                  onClick={onClearDate}
                  className="rounded p-0.5 hover:bg-cyan-500/30 hover:text-white"
                  title="Tarih seçimini kaldır"
                >
                  <X className="h-3 w-3" />
                </button>
              </span>
            )}
          </div>

          {/* Date Picker Input & Search Box */}
          <div className="flex flex-wrap items-center gap-2">
            <div className="relative flex items-center">
              <Calendar className="pointer-events-none absolute left-2.5 h-3.5 w-3.5 text-slate-500" />
              <input
                type="date"
                value={selectedDate || customDate}
                onChange={(e) => {
                  onClearDate();
                  setQuickFilter("custom");
                  setCustomDate(e.target.value);
                }}
                className="h-8 rounded-lg border border-border bg-surface pl-8 pr-2 text-xs text-slate-200 placeholder:text-slate-500 focus:border-cyan-500 focus:outline-none"
                title="Tarih seçimi"
              />
            </div>

            <div className="relative flex items-center">
              <Search className="pointer-events-none absolute left-2.5 h-3.5 w-3.5 text-slate-500" />
              <input
                type="text"
                placeholder="Bot adı veya BTCUSDT…"
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="h-8 w-44 rounded-lg border border-border bg-surface pl-8 pr-2.5 text-xs text-slate-200 placeholder:text-slate-500 focus:border-cyan-500 focus:outline-none sm:w-56"
              />
            </div>

            {hasActiveFilters && (
              <button
                type="button"
                onClick={handleClearAllFilters}
                className="inline-flex h-8 items-center gap-1 rounded-lg border border-border px-2 text-xs text-slate-400 hover:bg-secondary hover:text-slate-200"
                title="Tüm filtreleri sıfırla"
              >
                <RotateCcw className="h-3 w-3" />
                <span>Sıfırla</span>
              </button>
            )}
          </div>
        </div>

        {/* Active Filter Summary Strip */}
        <div className="flex flex-wrap items-center justify-between gap-2 rounded-lg bg-surface px-3 py-2 text-xs">
          <div className="flex items-center gap-2 text-slate-400">
            <Filter className="h-3.5 w-3.5 text-cyan-400" />
            <span>
              {selectedDate
                ? `Tarih: ${selectedDate} bazlı filtrelendi`
                : quickFilter === "today"
                  ? "Bugünün kapanan işlemleri"
                  : quickFilter === "yesterday"
                    ? "Dünün kapanan işlemleri"
                    : quickFilter === "7d"
                      ? "Son 7 günün işlemleri"
                      : quickFilter === "30d"
                        ? "Son 30 günün işlemleri"
                        : customDate
                          ? `Tarih: ${customDate}`
                          : "Tüm kapanan işlemler"}
            </span>
            {searchQuery && (
              <span className="rounded bg-cyan-500/15 px-1.5 py-0.5 text-[10px] text-cyan-300">
                Arama: &quot;{searchQuery}&quot;
              </span>
            )}
          </div>

          <div className="flex flex-wrap items-center gap-3 sm:gap-4">
            <span className="num">
              Net Kâr:{" "}
              <span
                className={cn(
                  "font-bold",
                  (summary?.totalNetPnl ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400"
                )}
              >
                {fmtUsdSigned(summary?.totalNetPnl ?? 0)}
              </span>
            </span>
            {summary?.profitFactor != null && (
              <span className="num text-slate-400">
                PF:{" "}
                <span
                  className={cn(
                    "font-semibold",
                    summary.profitFactor >= 1.5
                      ? "text-emerald-400"
                      : summary.profitFactor >= 1.0
                        ? "text-cyan-400"
                        : "text-rose-400"
                  )}
                  title="Profit Factor: Toplam Brüt Kazanç / Toplam Brüt Kayıp"
                >
                  {summary.profitFactor > 99 ? "∞" : summary.profitFactor.toFixed(2)}
                </span>
              </span>
            )}
            {summary?.avgDealProfit != null && (
              <span className="num text-slate-400">
                Ort. Deal:{" "}
                <span
                  className={cn(
                    "font-semibold",
                    summary.avgDealProfit >= 0 ? "text-emerald-400" : "text-rose-400"
                  )}
                  title="Ortalama İşlem Net Kârı"
                >
                  {fmtUsdSigned(summary.avgDealProfit)}
                </span>
              </span>
            )}
            <span className="num text-slate-400">
              Kazanma:{" "}
              <span className="font-semibold text-slate-200">%{summary?.winRate ?? 0}</span>{" "}
              ({summary?.wins ?? 0}W / {summary?.losses ?? 0}L)
            </span>
            <span className="num text-slate-400">
              Komisyon:{" "}
              <span className="font-semibold text-slate-300">
                {fmtUsd(summary?.totalFees ?? 0, 4)}
              </span>
            </span>
          </div>
        </div>
      </div>

      {/* Deals Table */}
      {loading && !data ? (
        <div className="flex h-48 items-center justify-center">
          <Spinner />
        </div>
      ) : deals.length === 0 ? (
        <EmptyState
          icon={History}
          title="Filtreye uygun kapanan işlem bulunamadı"
          hint="Seçilen tarih aralığında veya arama kriterinde kapanan deal kaydı yok. Filtreyi değiştirebilir veya sıfırlayabilirsiniz."
        />
      ) : (
        <div className="thin-scroll max-h-96 overflow-y-auto pr-1">
          <table className="w-full text-left text-xs">
            <thead>
              <tr className="border-b border-border/60 text-[10px] uppercase tracking-wider text-slate-500">
                <th className="pb-2 font-medium">Kapanış Zamanı</th>
                <th className="pb-2 font-medium">Bot & Parite</th>
                <th className="pb-2 font-medium">Çıkış Tipi</th>
                <th className="pb-2 font-medium text-right">Brüt / Gider</th>
                <th className="pb-2 font-medium text-right">Gerçekleşen Net</th>
              </tr>
            </thead>
            <tbody className="divide-y divide-border/40">
              {deals.map((deal) => {
                const isWin = deal.netPnl >= 0;
                return (
                  <tr key={deal.id} className="transition-colors hover:bg-surface-2/60">
                    {/* Timestamp */}
                    <td className="py-2.5 pr-3">
                      <div className="num font-medium text-slate-200">{deal.time}</div>
                      <div className="text-[10px] text-slate-500">{deal.date}</div>
                    </td>

                    {/* Bot Name & Symbol */}
                    <td className="py-2.5 pr-3">
                      <div className="flex flex-wrap items-center gap-1.5">
                        <span className="font-semibold text-slate-200">{deal.botName}</span>
                        <VenueChip venue={deal.venue} />
                        <KindBadge kind={deal.kind} />
                      </div>
                      <div className="num mt-0.5 text-[11px] text-slate-400">
                        {deal.symbol} · deal #{deal.id.replace("deal-", "")}
                      </div>
                    </td>

                    {/* Exit Reason / Type */}
                    <td className="py-2.5 pr-3">
                      <span
                        className={cn(
                          "inline-flex items-center gap-1 rounded px-2 py-0.5 text-[10px] font-bold uppercase",
                          deal.reason === "TP"
                            ? "bg-emerald-500/15 text-emerald-400"
                            : deal.reason === "LIQUIDATION"
                              ? "bg-rose-500/20 text-rose-300"
                              : "bg-rose-500/15 text-rose-400"
                        )}
                      >
                        {deal.reason === "TP" ? (
                          <CheckCircle2 className="h-3 w-3" />
                        ) : (
                          <XCircle className="h-3 w-3" />
                        )}
                        {deal.reason === "TP"
                          ? "KÂR AL (TP)"
                          : deal.reason === "LIQUIDATION"
                            ? "LİKİDASYON"
                            : "ZARAR DURDUR (SL)"}
                      </span>
                    </td>

                    {/* Gross / Fees */}
                    <td className="py-2.5 pr-3 text-right">
                      <div className="num text-slate-300">
                        Brüt: {deal.grossPnl >= 0 ? "+" : ""}
                        {fmtUsd(deal.grossPnl)}
                      </div>
                      <div className="num text-[10px] text-slate-500">
                        Fee: {fmtUsd(deal.fees, 4)}
                        {deal.funding !== 0 && ` · Fund: ${fmtUsd(deal.funding, 4)}`}
                      </div>
                    </td>

                    {/* Net Realized PnL */}
                    <td className="py-2.5 text-right">
                      <div
                        className={cn(
                          "num text-sm font-bold",
                          isWin ? "text-emerald-400" : "text-rose-400"
                        )}
                      >
                        {fmtUsdSigned(deal.netPnl)}
                      </div>
                      <div className="text-[10px] text-slate-500">
                        {isWin ? "Başarılı" : "Zararlı"}
                      </div>
                    </td>
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
    </Panel>
  );
}
