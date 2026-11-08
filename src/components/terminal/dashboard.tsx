"use client";

import { useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  ArrowRight,
  Bot,
  ChartCandlestick,
  ChevronDown,
  ChevronUp,
  Coins,
  Gauge,
  Power,
  Radio,
  Rss,
  TrendingUp,
  TriangleAlert,
  Wallet,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { NewsPulse } from "@/components/terminal/news-pulse";
import { DailyPnlChart } from "@/components/terminal/daily-pnl-chart";
import { ClosedDealsLedger } from "@/components/terminal/closed-deals";
import { DealProgressBar } from "@/components/terminal/deal-progress-bar";
import { Donut, ExposureBar, type DonutSlice } from "@/components/terminal/charts";
import {
  EmptyState,
  KindBadge,
  ModeBadge,
  Panel,
  Spinner,
  StatusDot,
  StatusPill,
  VenueChip,
} from "@/components/terminal/bits";
import { apiEquity, apiKlines, apiNews, useBots, useEventStream, useFills, useHalt, useMaintenance, useSummary } from "@/hooks/use-engine";
import {
  fmtFrac,
  fmtNum,
  fmtTimeShort,
  fmtUsd,
  fmtUsdSigned,
  fracToNum,
  kindLabel,
} from "@/lib/format";
import { cn } from "@/lib/utils";

/** Faz 24 (UI/UX Pro Max raporu §3.1.1): portföy equity curve — 30 günlük
 *  kümülatif özkaynak eğrisi (motor equity_points birleşimi, örneklenmiş).
 *  Bağımlılıksız inline SVG area chart. */
function EquityCurveCard({ points }: { points: { ts: number; equity: number }[] | null }) {
  const W = 560;
  const H = 140;
  const path = useMemo(() => {
    if (!points || points.length < 2) return null;
    const vals = points.map((p) => p.equity).filter(Number.isFinite);
    if (vals.length < 2) return null;
    const min = Math.min(...vals);
    const max = Math.max(...vals);
    const span = max - min || 1;
    const step = W / (points.length - 1);
    const coords = points.map((p, i) => {
      const x = i * step;
      const y = H - 8 - ((p.equity - min) / span) * (H - 24);
      return `${i === 0 ? "M" : "L"}${x.toFixed(1)},${y.toFixed(1)}`;
    });
    const area = `${coords.join(" ")} L${W},${H} L0,${H} Z`;
    const first = vals[0];
    const last = vals[vals.length - 1];
    const up = last >= first;
    return { line: coords.join(" "), area, up, first, last };
  }, [points]);
  const stroke = path?.up ? "#34d399" : "#fb7185";
  return (
    <Panel title="Portföy Özkaynak Eğrisi (30g)" icon={<ChartCandlestick className="h-4 w-4 text-slate-500" aria-hidden />}>
      {!path ? (
        <div className="flex h-[140px] items-center justify-center text-xs text-slate-500">
          {points && points.length < 2 ? "Grafik için en az 2 günlük veri gerekli — botlar çalıştıkça dolar." : "Yükleniyor…"}
        </div>
      ) : (
        <div>
          <div className="mb-1 flex items-baseline gap-2">
            <span className={cn("num text-lg font-bold", path.up ? "text-emerald-400" : "text-rose-400")}>
              ${path.last.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
            </span>
            <span className="num text-[11px] text-slate-500">
              30g değişim: {(((path.last - path.first) / (Math.abs(path.first) || 1)) * 100).toFixed(2)}%
            </span>
          </div>
          <svg viewBox={`0 0 ${W} ${H}`} width="100%" height={H} preserveAspectRatio="none" aria-hidden="true">
            <path d={path.area} fill={stroke} opacity="0.08" />
            <path d={path.line} fill="none" stroke={stroke} strokeWidth="2" />
          </svg>
          <p className="mt-1 text-[10px] text-slate-500">Tüm botların birleşik özkaynağı (motor equity kayıtları, UTC zaman damgalı).</p>
        </div>
      )}
    </Panel>
  );
}



const DONUT_COLORS = ["#10B981", "#06B6D4", "#F59E0B", "#F43F5E", "#94A3B8", "#0EA5E9", "#D946EF"];

function StreamTypeChip({ type, payload }: { type: string; payload: Record<string, unknown> }) {
  if (type === "FILL") {
    const buy = payload.side === "Buy";
    return (
      <span
        className={cn(
          "rounded px-1.5 py-0.5 text-[10px] font-bold",
          buy ? "bg-emerald-500/15 text-emerald-400" : "bg-rose-500/15 text-rose-400",
        )}
      >
        FILL {buy ? "BUY" : "SELL"} · {String(payload.role ?? "")}
      </span>
    );
  }
  if (type === "EQUITY")
    return <span className="rounded bg-cyan-500/15 px-1.5 py-0.5 text-[10px] font-bold text-cyan-400">EQUITY</span>;
  if (type === "FUNDING") {
    const cost = Number(payload.expense ?? 0) > 0;
    return (
      <span
        className={cn(
          "rounded px-1.5 py-0.5 text-[10px] font-bold",
          cost ? "bg-amber-500/15 text-amber-400" : "bg-emerald-500/15 text-emerald-400",
        )}
      >
        FUNDING {cost ? "PAID" : "REBATE"}
      </span>
    );
  }
  if (type === "LIQUIDATION")
    return <span className="rounded bg-rose-500/15 px-1.5 py-0.5 text-[10px] font-bold text-rose-400">LIQUIDATION</span>;
  if (type === "RISK")
    return (
      <span
        title={`Risk Motoru uyarısı — ${String(payload.code ?? "")}`}
        className="inline-flex items-center gap-1 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-bold text-amber-400"
      >
        <TriangleAlert className="h-3 w-3" aria-hidden />
        RISK {String(payload.code ?? "")}
      </span>
    );
  if (type === "BOT_STATE")
    return <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] font-bold text-amber-400">BOT_STATE</span>;
  if (type === "ORDER")
    return <span className="rounded bg-slate-500/20 px-1.5 py-0.5 text-[10px] font-bold text-slate-300">ORDER</span>;
  if (type === "KILL_SWITCH") {
    const engaged = payload.engaged === true;
    return (
      <span
        className={cn(
          "inline-flex items-center gap-1 rounded px-1.5 py-0.5 text-[10px] font-bold",
          engaged ? "bg-rose-500/15 text-rose-400" : "bg-emerald-500/15 text-emerald-400",
        )}
      >
        <TriangleAlert className="h-3 w-3" aria-hidden />
        KILL {engaged ? "ENGAGED" : "RELEASED"}
      </span>
    );
  }
  if (type === "FREEZE") {
    const frozen = payload.frozen === true;
    return (
      <span
        className={cn(
          "rounded px-1.5 py-0.5 text-[10px] font-bold",
          frozen ? "bg-cyan-500/15 text-cyan-400" : "bg-emerald-500/15 text-emerald-400",
        )}
      >
        FREEZE {frozen ? "ON" : "OFF"}
      </span>
    );
  }
  if (type === "LOG") {
    const level = String(payload.level ?? "INFO");
    const cls =
      level === "ERROR"
        ? "bg-rose-500/15 text-rose-400"
        : level === "WARN"
          ? "bg-amber-500/15 text-amber-400"
          : "bg-slate-500/20 text-slate-300";
    return <span className={cn("rounded px-1.5 py-0.5 text-[10px] font-bold", cls)}>LOG {level}</span>;
  }
  return <span className="rounded bg-slate-500/20 px-1.5 py-0.5 text-[10px] font-bold text-slate-300">{type}</span>;
}

function streamMessage(type: string, p: Record<string, unknown>): string {
  switch (type) {
    case "FILL":
      return `${p.side} ${fmtFrac(String(p.qty ?? "0"))} @ ${fmtFrac(String(p.price ?? "0"))} · fee ${fmtFrac(
        String(p.fee ?? "0"),
        4,
      )} · ${String(p.fillModel ?? "")}`;
    case "EQUITY":
      return `equity ${fmtUsd(String(p.equity ?? "0"))} · qty ${fmtFrac(String(p.positionQty ?? "0"))} @ ${fmtFrac(
        String(p.avgPrice ?? "0"),
      )} · realized ${fmtUsdSigned(String(p.realizedGross ?? "0"))}`;
    case "LOG":
      return String(p.message ?? "");
    case "FUNDING":
      return `funding rate ${String(p.rate ?? "0")} · expense ${fmtUsdSigned(String(p.expense ?? "0"))} · cumulative ${fmtUsd(String(p.cumulativeFunding ?? "0"))}`;
    case "LIQUIDATION":
      return `position force-closed @ ${fmtFrac(String(p.liqPrice ?? "0"))} · ${String(p.cancelledOrders ?? 0)} resting order iptal`;
    case "RISK":
      return `${String(p.message ?? "Risk Motoru uyarısı")}${p.botId ? ` · bot ${String(p.botId).slice(0, 10)}…` : ""}`;
    case "BOT_STATE":
      return `bot ${String(p.botId ?? "").slice(0, 10)}… → ${String(p.status ?? "")}${p.action ? ` (${String(p.action)})` : ""}`;
    case "ORDER":
      return `order ${String(p.id ?? "")} → ${String(p.result ?? "")}`;
    case "KILL_SWITCH":
      return p.engaged === true
        ? `YÜRÜTME DURDURULDU — ${String(p.reason ?? "")}${p.by ? ` (by ${String(p.by)})` : ""}`
        : `Kill-switch bırakıldı${p.by ? ` (by ${String(p.by)})` : ""} — selfcheck yenileyin`;
    case "FREEZE":
      return p.frozen === true
        ? `LIVE hat donduruldu — ${String(p.reason ?? "")}`
        : "Freeze çözüldü — LIVE hat açık";
    default:
      return JSON.stringify(p).slice(0, 120);
  }
}

/* ---- Bot başına "bugünkü" aktivite — GERÇEK fills[].ts damgalarından türetilir.
   Motorun günlük Kâr/Zarar kırılımı yoktur; o yüzden bugünkü gerçekleşen Kâr YAZILMAZ,
   yalnızca bugünkü fill sayısı + komisyonu gösterilir. Veri yoksa "—". ---- */
function startOfTodayMs(): number {
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

function BotDayActivity({ botId }: { botId: string }) {
  const { fills } = useFills(botId);
  const day = useMemo(() => {
    const t0 = startOfTodayMs();
    let n = 0;
    let fee = 0;
    for (const f of fills) {
      if ((f.ts ?? 0) >= t0) {
        n += 1;
        fee += fracToNum(f.fee) ?? 0;
      }
    }
    return { n, fee };
  }, [fills]);
  if (day.n === 0) return <span className="text-slate-500">Bugün: —</span>;
  return (
    <span>
      Bugün: <span className="num text-slate-300">{day.n} fill</span> · komisyon{" "}
      <span className="num text-slate-400">{fmtUsd(day.fee.toFixed(8), 4)}</span>
    </span>
  );
}

/* Faz 2.8 — Kaynak Durumu: haber beslemeleri + venue erişimi (v3 market-center
   "Kaynak Durumu" karşılığı). Salt-okunur; veri yoksa dürüstçe boş gösterilir. */
function SourceStatus({ symbol }: { symbol: string }) {
  const { status: maint, attempted } = useMaintenance(60000);
  const [sources, setSources] = useState<string[] | null>(null);
  useEffect(() => {
    let alive = true;
    apiNews(symbol)
      .then((r) => { if (alive) setSources(r.sources ?? []); })
      .catch(() => { if (alive) setSources(null); });
    return () => { alive = false; };
  }, [symbol]);
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-1.5 rounded-xl border border-border bg-surface px-4 py-2.5 text-[11px]">
      <span className="flex items-center gap-1.5 font-bold uppercase tracking-wider text-slate-500">
        <Rss className="h-3.5 w-3.5" aria-hidden />
        Kaynak Durumu
      </span>
      <span className="text-slate-500">
        Haber:{" "}
        <span className="text-slate-300">{sources === null ? "—" : sources.length === 0 ? "kaynak yok" : sources.join(" · ")}</span>
      </span>
      {(maint?.venues ?? []).map((v) => (
        <span key={v.venue} className="num text-slate-500">
          {v.venue}{" "}
          <span className={v.ok ? "font-bold text-emerald-400" : "font-bold text-rose-400"}>
            {v.ok ? `● ${v.ms}ms` : "● erişilemiyor"}
          </span>
        </span>
      ))}
      {(!maint || (maint.venues ?? []).length === 0) && (
        <span className="text-slate-500">{attempted ? "venue yoklaması alınamadı" : "venue yoklaması bekleniyor…"}</span>
      )}
    </div>
  );
}

export function Dashboard({ onInspect, onNewBot }: { onInspect: (id: string) => void; onNewBot: () => void }) {
  const { summary, loading } = useSummary();
  const { bots } = useBots();
  const { events, connected } = useEventStream();
  const { kill, freeze } = useHalt();
  const [selectedDate, setSelectedDate] = useState<string | null>(null);
  const [showMarketNews, setShowMarketNews] = useState(false);
  // Faz 24 (UI/UX Pro Max raporu §3.1.1): portföy equity curve verisi —
  // 30 günlük motor equity_points birleşimi; her 5 dk tazelenir.
  const [equity, setEquity] = useState<{ ts: number; equity: number }[] | null>(null);
  useEffect(() => {
    let alive = true;
    const tick = () => {
      apiEquity(30)
        .then((r) => {
          if (alive) setEquity(r.points ?? []);
        })
        .catch(() => {
          if (alive) setEquity(null);
        });
    };
    tick();
    const t = setInterval(tick, 300_000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);
  /* Faz 25 (UIX-19): SSE akışı ham mühendislik günlüğüydü — dakikada bir
     tekrarlayan EQUITY tick'leri işleme-önemli olayları (FILL/RISK/TP...) ekrandan
     itiyordu. Varsayılan filtre "İşlem-önemli"; EQUITY/LOG INFO opt-in.
     Ayrıca botId taşıyan olaylara bot adı çipi eklenir (ham ID değil). */
  const [onlyAction, setOnlyAction] = useState(true);
  const nameById = useMemo(() => new Map(bots.map((b) => [b.id, b.name] as const)), [bots]);
  const botNameOf = (id: unknown): string | null => {
    const k = typeof id === "string" ? id : null;
    return k ? nameById.get(k) ?? null : null;
  };
  const shownEvents = useMemo(
    () => (onlyAction ? events.filter((e) => e.type !== "EQUITY" && !(e.type === "LOG" && e.payload.level === "INFO")) : events),
    [events, onlyAction],
  );

  /* ---- REAL 24h change: top-allocation symbol, 1d klines, close-over-close ---- */
  const topSymbol = useMemo(() => {
    let best: string | null = null;
    let bestVal = -1;
    for (const a of summary?.allocation ?? []) {
      const v = fracToNum(a.value) ?? 0;
      if (v > bestVal) {
        bestVal = v;
        best = a.symbol;
      }
    }
    return best;
  }, [summary]);

  const [chg, setChg] = useState<{ symbol: string | null; pct: number | null }>({
    symbol: null,
    pct: null,
  });

  useEffect(() => {
    if (!topSymbol) return;
    let alive = true;
    const load = async () => {
      try {
        const r = await apiKlines(topSymbol, "1d", 2);
        if (!alive) return;
        const bars = [...(r.bars ?? [])].sort((a, b) => a.t - b.t);
        let pct: number | null = null;
        if (bars.length >= 2) {
          const c1 = fracToNum(bars[0].c);
          const c2 = fracToNum(bars[1].c);
          if (c1 && c2 && c1 > 0) pct = ((c2 - c1) / c1) * 100;
        }
        setChg({ symbol: topSymbol, pct });
      } catch {
        if (alive) setChg({ symbol: topSymbol, pct: null });
      }
    };
    void load();
    const t = setInterval(() => void load(), 60000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [topSymbol]);

  /* Render-time derivation: stale symbol (mid-refetch) shows "…", missing data shows "—". */
  const chgPct: number | null =
    topSymbol !== null && chg.symbol === topSymbol && chg.pct !== null ? chg.pct : null;
  /* Faz 25 (UIX-34): yüzde de para konvansiyonuyla aynı ondalık noktayı
     kullanır (en-US) — aynı kartta "+0,83%" ile "$-281.05" karışmıyordu. */
  const chgTxt = !topSymbol
    ? "—"
    : chgPct !== null
      ? `${chgPct > 0 ? "+" : ""}${chgPct.toFixed(2)}%`
      : chg.symbol === topSymbol
        ? "—"
        : "…";

  const longPct = useMemo(() => {
    if (!summary) return 0;
    const eq = fracToNum(summary.totalEquity) ?? 0;
    const lev = fracToNum(summary.longExposure) ?? 0;
    if (eq <= 0) return 0;
    return Math.min(100, (lev / eq) * 100);
  }, [summary]);

  const shortPct = useMemo(() => {
    if (!summary) return 0;
    const eq = fracToNum(summary.totalEquity) ?? 0;
    const sh = fracToNum(summary.shortExposure) ?? 0;
    if (eq <= 0) return 0;
    return Math.min(100, (sh / eq) * 100);
  }, [summary]);

  const totalExposure = useMemo(() => {
    if (!summary) return 0;
    return (fracToNum(summary.longExposure) ?? 0) + (fracToNum(summary.shortExposure) ?? 0);
  }, [summary]);
  const totalEq = useMemo(() => (summary ? fracToNum(summary.totalEquity) ?? 0 : 0), [summary]);
  const utilPct = totalEq > 0 ? Math.min(100, (totalExposure / totalEq) * 100) : 0;
  const freeBufferPct = Math.max(0, 100 - utilPct);

  const [botFilter, setBotFilter] = useState<"all" | "open">("all");
  const openBots = useMemo(() => bots.filter((b) => b.stats.phase === "Open"), [bots]);
  const displayedBots = botFilter === "open" ? openBots : bots;

  const donutSlices: DonutSlice[] = useMemo(
    () =>
      (summary?.allocation ?? []).map((a, i) => ({
        label: a.symbol,
        value: fracToNum(a.value) ?? 0,
        color: DONUT_COLORS[i % DONUT_COLORS.length],
      })),
    [summary],
  );

  const killActive = kill?.engaged === true;
  const freezeActive = freeze?.frozen === true;

  const totalEconomicPnL = useMemo(() => {
    if (!summary) return 0;
    const realized = fracToNum(summary.totalRealizedGross) ?? 0;
    const fees = fracToNum(summary.totalFees) ?? 0;
    const unrealized = fracToNum(summary.totalUnrealizedPnl) ?? 0;
    return realized - fees + unrealized;
  }, [summary]);

  return (
    <div className="space-y-4">
      {/* Faz 0.2/0.3 — global şalter bandı (SSE + 10sn poll; restart-safe) */}
      {(killActive || freezeActive) && (
        <div
          role="alert"
          className={cn(
            "flex items-center gap-2 rounded-xl border px-4 py-2.5 text-xs font-bold",
            killActive ? "border-rose-500/40 bg-rose-500/10 text-rose-300" : "border-cyan-500/40 bg-cyan-500/10 text-cyan-300",
          )}
        >
          <TriangleAlert className="h-4 w-4 shrink-0" aria-hidden />
          {killActive
            ? `KILL-SWITCH DEVREDE — yürütme durdu (TV ENTRY dahil reddedilir)${kill?.reason ? ` · ${kill.reason}` : ""}`
            : `FREEZE DEVREDE — LIVE hatta yeni emir yok, açıklar korunuyor${freeze?.reason ? ` · ${freeze.reason}` : ""}`}
        </div>
      )}
      {/* Top metrics row */}
      <div className="grid grid-cols-1 gap-4 sm:grid-cols-2 xl:grid-cols-4">
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.25 }}
          className="rounded-xl border border-border bg-[#151a24] p-4 glow-emerald"
        >
          <div className="flex items-center gap-2 text-[11px] uppercase tracking-wider text-slate-400">
            <Wallet className="h-3.5 w-3.5" aria-hidden />
            Toplam Özkaynak
          </div>
          {loading && !summary ? (
            <div className="mt-3 flex items-center gap-2">
              <Spinner />
            </div>
          ) : (
            <>
              <div className="num mt-2 text-3xl font-bold tracking-tight text-slate-50">
                {fmtUsd(summary?.totalEquity, 2)}
              </div>
              {/* Faz 25 (UIX-41): "24s Değişim" borsa verisiydi, portföy verisi
                  gibi okunuyordu — etiket artık varlığı ve niteliği söylüyor. */}
              <div className="mt-1 text-xs text-slate-500">
                <span
                  title={chgPct !== null ? `${chg.symbol} 1d close-over-close — borsa fiyat değişimi, portföy getirisi değil` : chg.symbol === topSymbol ? "veri yok" : "yükleniyor…"}
                >
                  {topSymbol ?? "—"} 24s{" "}
                </span>
                <span
                  className={cn(
                    "num",
                    chgPct === null ? "text-slate-400" : chgPct >= 0 ? "text-emerald-400" : "text-rose-400",
                  )}
                >
                  {chgPct !== null ? chgTxt : chgTxt}
                </span>{" "}
                · Gerçekleşen{" "}
                <span className="num text-slate-400">{fmtUsdSigned(summary?.totalRealizedGross)}</span>
              </div>
            </>
          )}
        </motion.div>

        <div className="rounded-xl border border-border bg-[#151a24] p-4">
          <div className="flex items-center gap-2 text-[11px] uppercase tracking-wider text-slate-400">
            <Bot className="h-3.5 w-3.5" aria-hidden />
            Bot Filosu
          </div>
          <div className="num mt-2 text-3xl font-bold text-slate-50">
            {summary?.activeBots ?? 0}
            <span className="text-lg text-slate-500">/{summary?.totalBots ?? 0}</span>
          </div>
          <div className="mt-1 text-xs text-slate-500">
            Aktif / toplam bot · <span className="num text-cyan-400">{summary?.activeDeals ?? 0}</span> açık deal
          </div>
        </div>

        <div className="rounded-xl border border-border bg-[#151a24] p-4">
          <div className="flex items-center justify-between text-[11px] uppercase tracking-wider text-slate-400">
            <div className="flex items-center gap-2">
              <TrendingUp className="h-3.5 w-3.5" aria-hidden />
              Toplam Ekonomik K/Z
            </div>
            <span
              className={cn(
                "rounded px-1.5 py-0.5 text-[9px] font-bold",
                totalEconomicPnL >= 0 ? "bg-emerald-500/20 text-emerald-300" : "bg-rose-500/20 text-rose-300",
              )}
              title="Gerçekleşen net kâr + anlık açık pozisyonların toplamı (dürüst cüzdan etkisi)"
            >
              {totalEconomicPnL >= 0 ? "NET KÂRDA" : "DRAWDOWNDA"}
            </span>
          </div>
          <div
            className={cn(
              "num mt-2 text-3xl font-bold tracking-tight",
              totalEconomicPnL >= 0 ? "text-emerald-400" : "text-rose-400",
            )}
            title="Toplam Ekonomik K/Z: Gerçekleşen (Net) + Açık Pozisyonlar (Unrealized)"
          >
            {fmtUsdSigned(totalEconomicPnL)}
          </div>
          <div className="mt-1 text-xs text-slate-500">
            Gerçekleşen: <span className="num font-semibold text-slate-300">{fmtUsdSigned(summary?.totalRealizedGross)}</span> · Açık:{" "}
            <span
              className={cn(
                "num font-semibold",
                (fracToNum(summary?.totalUnrealizedPnl) ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400",
              )}
            >
              {fmtUsdSigned(summary?.totalUnrealizedPnl ?? "0")}
            </span>{" "}
            · Komisyon <span className="num text-slate-400">{fmtUsd(summary?.totalFees, 2)}</span>
          </div>
        </div>

        <div className="rounded-xl border border-border bg-[#151a24] p-4">
          <div className="flex items-center gap-2 text-[11px] uppercase tracking-wider text-slate-400">
            <Gauge className="h-3.5 w-3.5" aria-hidden />
            Maruziyet / Marj
          </div>
          <div className="mt-3">
            <ExposureBar longPct={longPct} shortPct={0} />
          </div>
          {/* Faz 25 (UIX-40): "L $3,230 / S $3,086" neyin değeri belirsizdi —
              açık pozisyon notional'ı + özkaynağa oranı net etiketle. */}
          <div className="num mt-2 flex justify-between text-[11px] text-slate-400" title="Açık pozisyonların notional toplamı ve özkaynağa oranı">
            <span>
              Long <span className="text-slate-200">{fmtUsd(summary?.longExposure, 0)}</span>
              <span className="text-slate-500"> · %{Math.round(longPct)}</span>
            </span>
            <span>
              Short <span className="text-slate-200">{fmtUsd(summary?.shortExposure, 0)}</span>
              <span className="text-slate-500"> · %{Math.round(shortPct)}</span>
            </span>
          </div>
          <div className="num mt-2.5 flex items-center justify-between border-t border-border/50 pt-2 text-[10px] text-slate-400">
            <span>Sermaye Kullanımı: <span className="font-semibold text-slate-200">%{Math.round(utilPct)}</span></span>
            <span>Güvenlik Tamponu: <span className="font-semibold text-emerald-400">%{Math.round(freeBufferPct)}</span></span>
          </div>
        </div>
      </div>

      {/* Finansal Performans: Gün Bazlı Kâr/Zarar Dağılımı Bar Grafiği */}
      <DailyPnlChart
        selectedDate={selectedDate}
        onSelectDate={setSelectedDate}
      />

      {/* Kapanan İşlemler Defteri (Gün & Bot Filtreli) */}
      <ClosedDealsLedger
        selectedDate={selectedDate}
        onClearDate={() => setSelectedDate(null)}
      />

      {/* Middle: allocation + fleet */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <EquityCurveCard points={equity} />
        <Panel title="Tahsis Dağılımı" icon={<Coins className="h-4 w-4 text-slate-500" aria-hidden />}>
          {/* Faz 25 (UIX-32): tek varlıkta donut bilgi taşımaz — ölü alanı
              işletme özetiyle değiştir; çok varlıkta donut kalır. */}
          {donutSlices.length <= 1 ? (
            <div className="num grid grid-cols-2 gap-3 text-xs">
              <div>
                <div className="text-slate-500">AÇIK DEAL</div>
                <div className="mt-0.5 text-lg font-semibold text-cyan-400">{summary?.activeDeals ?? 0}</div>
              </div>
              <div>
                <div className="text-slate-500">AKTİF BOT</div>
                <div className="mt-0.5 text-lg font-semibold text-slate-200">
                  {summary?.activeBots ?? 0}
                  <span className="text-sm text-slate-500">/{summary?.totalBots ?? 0}</span>
                </div>
              </div>
              <div>
                <div className="text-slate-500">LONG MARJ</div>
                <div className="mt-0.5 text-slate-200">{fmtUsd(summary?.longExposure, 0)}</div>
              </div>
              <div>
                <div className="text-slate-500">SHORT MARJ</div>
                <div className="mt-0.5 text-slate-200">{fmtUsd(summary?.shortExposure, 0)}</div>
              </div>
              <div className="col-span-2">
                <div className="text-slate-500">TAHSİS</div>
                <div className="mt-0.5 text-slate-300">
                  {donutSlices.length === 1
                    ? `${donutSlices[0].label} — tek varlık (%100)`
                    : "pozisyon yok"}
                </div>
              </div>
            </div>
          ) : (
            <Donut
              slices={donutSlices}
              centerLabel={fmtUsd(summary?.totalEquity, 0)}
              centerSub="TOPLAM"
            />
          )}
        </Panel>

        <Panel
          className="lg:col-span-2"
          title="Botlarım"
          icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />}
          right={
            <div className="flex items-center gap-2">
              <div className="flex rounded-lg border border-border p-0.5 text-[10px]">
                <button
                  type="button"
                  onClick={() => setBotFilter("all")}
                  className={cn(
                    "rounded px-2 py-0.5 font-semibold transition-colors",
                    botFilter === "all" ? "bg-secondary text-slate-100" : "text-slate-400 hover:text-slate-200"
                  )}
                >
                  Tümü ({bots.length})
                </button>
                <button
                  type="button"
                  onClick={() => setBotFilter("open")}
                  className={cn(
                    "rounded px-2 py-0.5 font-semibold transition-colors",
                    botFilter === "open" ? "bg-cyan-500/20 text-cyan-300" : "text-slate-400 hover:text-slate-200"
                  )}
                >
                  ⚡ Açık Deal ({openBots.length})
                </button>
              </div>
              <Button size="sm" variant="outline" className="min-h-9 border-border-strong text-xs text-slate-300 hover:bg-secondary" onClick={onNewBot}>
                <ArrowRight className="h-3.5 w-3.5" aria-hidden />
                Yeni Bot
              </Button>
            </div>
          }
          bodyClassName="p-3"
        >
          {displayedBots.length === 0 ? (
            <EmptyState
              icon={Bot}
              title={botFilter === "open" ? "Açık deal yok" : "Henüz bot yok"}
              hint={botFilter === "open" ? "Şu anda hiçbir bot açık bir deal yürütmüyor." : "Yeni Bot sihirbazıyla ilk DCA botunuzu başlatın."}
            />
          ) : (
            <div className="thin-scroll max-h-72 space-y-2 overflow-y-auto pr-1">
              {displayedBots.map((b) => (
                <button
                  key={b.id}
                  onClick={() => onInspect(b.id)}
                  className="flex w-full items-center gap-3 rounded-lg border border-border bg-surface p-3 text-left transition-colors hover:border-border-strong hover:bg-surface-2"
                >
                  <StatusDot status={b.status} />
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-1.5">
                      <span className="truncate text-sm font-semibold text-slate-100">{b.name}</span>
                      <KindBadge kind={b.kind} />
                      <VenueChip venue={b.venue} />
                      <ModeBadge mode={b.mode} />
                      {b.stats.phase === "Open" &&
                        b.config?.max_safety_orders != null &&
                        b.stats.safety_triggered >= b.config.max_safety_orders && (
                          <span
                            className="inline-flex items-center gap-1 rounded bg-amber-500/20 px-1.5 py-0.5 text-[10px] font-bold text-amber-300 ring-1 ring-amber-500/40"
                            title="Tüm güvenlik emirleri doldu (Kademe tükendi). Fiyat dönene veya sermaye eklenene kadar beklemede."
                          >
                            ⚠️ KADEME TÜKENDİ
                          </span>
                        )}
                    </div>
                    <div className="num mt-1 text-[11px] text-slate-400">
                      {b.symbol} · {kindLabel(b.kind)} ·{" "}
                      {b.stats.phase === "Open"
                        ? `Açık Deal: ${fmtFrac(b.stats.position_qty)} @ ${fmtFrac(b.stats.avg_price)}`
                        : "Deal bekleniyor (başlangıç koşulu izleniyor)"}{" "}
                      · SO {b.stats.safety_triggered}
                      {b.config?.max_safety_orders != null ? `/${b.config.max_safety_orders}` : ""} · TP{" "}
                      {b.stats.tp_price ? fmtFrac(b.stats.tp_price) : "—"} / SL{" "}
                      {b.stats.sl_price ? fmtFrac(b.stats.sl_price) : "—"}
                    </div>
                    <div className="num mt-0.5 text-[11px] text-slate-400">
                      Deal: {b.stats.deals_closed} kapatıldı (W {b.stats.wins} / L {b.stats.losses}) · Komisyon{" "}
                      {fmtUsd(b.cumulativeFees, 4)} · <BotDayActivity botId={b.id} />
                    </div>
                    {b.stats.phase === "Open" && (
                      <div className="mt-2 max-w-lg">
                        <DealProgressBar
                          phase={b.stats.phase}
                          safetyTriggered={b.stats.safety_triggered}
                          maxSafetyOrders={b.config?.max_safety_orders}
                          avgPrice={b.stats.avg_price}
                          tpPrice={b.stats.tp_price}
                          slPrice={b.stats.sl_price}
                          currentPrice={b.markPrice}
                          unrealizedPnl={b.unrealizedNow}
                          liqPrice={b.liqPrice}
                          marginMode={b.marginMode}
                          nextSafetyPrice={b.stats.next_safety_price}
                          nextSafetyLevel={b.stats.next_safety_level}
                          positionQty={b.stats.position_qty}
                          isShort={b.kind === "DCA_SHORT"}
                          symbol={b.symbol}
                          compact={true}
                        />
                      </div>
                    )}
                  </div>
                  <div className="hidden shrink-0 text-right sm:block">
                    <div className="text-[10px] uppercase text-slate-400">Net Kâr / Anlık</div>
                    <div
                      className={cn(
                        "num text-lg font-bold",
                        (fracToNum(b.stats.realized_gross) ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400",
                      )}
                    >
                      {fmtUsdSigned(b.stats.realized_gross)}
                    </div>
                    {b.unrealizedNow && b.unrealizedNow !== "0" && (
                      <div
                        className={cn(
                          "num text-[11px] font-semibold",
                          (fracToNum(b.unrealizedNow) ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400",
                        )}
                        title="Açık pozisyonun anlık gerçekleşmemiş kâr/zararı"
                      >
                        Açık: {fmtUsdSigned(b.unrealizedNow)}
                      </div>
                    )}
                  </div>
                  <StatusPill status={b.status} className="hidden md:inline-flex" />
                </button>
              ))}
            </div>
          )}
        </Panel>
      </div>

      {/* Bottom: live stream + killswitch */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Panel
          className="lg:col-span-2"
          title="Canlı Olay Akışı"
          icon={<Radio className="h-4 w-4 text-slate-500" aria-hidden />}
          right={
            <span className="flex items-center gap-1.5">
              <span
                className={cn(
                  "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[10px] font-semibold",
                  connected ? "border-emerald-500/40 text-emerald-400" : "border-slate-600 text-slate-500",
                )}
              >
                <StatusDot status={connected ? "RUNNING" : "STOPPED"} />
                {connected ? "STREAM LIVE" : "STREAM OFF"}
              </span>
              {/* Faz 25 (UIX-19): equity tick gürültüsü kapalı-default; isteyen
                  ham akışı açabilir. */}
              <button
                type="button"
                aria-pressed={onlyAction}
                title={onlyAction ? "Yalnız işlem-önemli olaylar (FILL, RISK, TP/SL, HALT…) — equity tick'leri gizli" : "Tüm olaylar: equity tick'leri ve INFO günlükleri dahil"}
                onClick={() => setOnlyAction((v) => !v)}
                className={cn(
                  "inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-semibold transition-colors",
                  onlyAction ? "border-cyan-500/40 bg-cyan-500/10 text-cyan-300" : "border-slate-600 text-slate-500 hover:text-slate-300",
                )}
              >
                {onlyAction ? "İŞLEM-ÖNEMLİ" : "TÜM OLAYLAR"}
              </button>
            </span>
          }
          bodyClassName="p-0"
        >
          <div className="thin-scroll max-h-96 overflow-y-auto p-3 font-mono text-xs" aria-live="polite">
            {shownEvents.length === 0 ? (
              <EmptyState
                icon={Radio}
                title={events.length === 0 ? "Olay bekleniyor…" : "Filtrede eşleşen olay yok"}
                hint={
                  events.length === 0
                    ? "SSE akışı bağlandığında fill'ler ve önemli durum değişimleri burada belirir."
                    : "Equity tick'leri ve INFO günlükleri filtreli — görmek için TÜM OLAYLAR'a geç."
                }
              />
            ) : (
              <ul className="space-y-1">
                {shownEvents.map((e) => {
                  const bn = botNameOf(e.payload.botId);
                  return (
                    <li key={e.id} className="flex items-start gap-2 rounded px-1 py-0.5 hover:bg-surface-2">
                      <span className="shrink-0 text-slate-500">{fmtTimeShort(e.at)}</span>
                      <StreamTypeChip type={e.type} payload={e.payload} />
                      {bn && (
                        <span className="shrink-0 rounded bg-secondary px-1.5 py-0.5 text-[10px] font-semibold text-slate-300" title={String(e.payload.botId ?? "")}>
                          {bn}
                        </span>
                      )}
                      <span
                        className={cn(
                          "min-w-0 flex-1 break-words",
                          e.type === "LOG" && e.payload.level === "ERROR"
                            ? "text-rose-400"
                            : e.type === "LOG" && e.payload.level === "WARN"
                              ? "text-amber-400"
                              : e.type === "RISK"
                                ? "text-amber-400"
                                : "text-slate-300",
                        )}
                      >
                        {streamMessage(e.type, e.payload)}
                      </span>
                    </li>
                  );
                })}
              </ul>
            )}
          </div>
        </Panel>

        <Panel title="Acil Durum Killswitch" icon={<Power className="h-4 w-4 text-rose-400" aria-hidden />}>
          {/* Faz 25 (UIX-27): kart belge gibiydi — gate listesi + "kullanıcı
              patron" sohbet tonu. Tehlike bölgesi tek net cümle + tek yönlendirme. */}
          <div className="flex h-full flex-col gap-3">
            <p className="text-xs leading-relaxed text-slate-300">
              Tüm botları tek seferde durdurur: açık pozisyonlar korunur, yeni emir üretimi durur.
              Onay çift aşamalıdır — önce niyet, sonra <span className="num font-bold text-rose-400">KILL</span> yazımı (sağ üst düğme).
            </p>
            <div className="rounded-lg border border-rose-500/20 bg-rose-500/5 p-3 text-[11px] leading-relaxed text-slate-400">
              <span className="font-semibold text-rose-400">Koruma katmanı:</span>{" "}
              8 zorunlu kapı (yapısal ihlalde emir reddi) · 10 piyasa koruması uyarı+onay (işlem engellemez).
              Ayrıntı: <span className="font-semibold text-slate-300">Bilgi → Sözleşme (8 Fail-Closed Gate)</span>.
            </div>
          </div>
        </Panel>
      </div>

      {/* İsteğe Bağlı Piyasa İstihbaratı ve Kaynak Durumu */}
      <div className="rounded-xl border border-border bg-[#151a24] p-3 text-xs">
        <button
          type="button"
          onClick={() => setShowMarketNews((v) => !v)}
          className="flex w-full items-center justify-between text-slate-400 hover:text-slate-200 transition-colors"
        >
          <span className="flex items-center gap-2 font-semibold">
            <Rss className="h-4 w-4 text-cyan-400" />
            Piyasa Nabzı & Kaynak Durumu (Haberler, Sentiment, Gecikme)
          </span>
          {showMarketNews ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
        </button>
        {showMarketNews && (
          <div className="mt-3 space-y-3 pt-3 border-t border-border/50">
            <NewsPulse symbol={topSymbol ?? "BTCUSDT"} />
            <SourceStatus symbol={topSymbol ?? "BTCUSDT"} />
          </div>
        )}
      </div>
    </div>
  );
}
