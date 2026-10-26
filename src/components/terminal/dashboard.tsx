"use client";

import { useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  ArrowRight,
  Bot,
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
import { KillButton } from "@/components/terminal/killswitch";
import { NewsPulse } from "@/components/terminal/news-pulse";
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
import { apiKlines, apiNews, useBots, useEventStream, useFills, useHalt, useMaintenance, useSummary } from "@/hooks/use-engine";
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
  const chgTxt = !topSymbol
    ? "—"
    : chgPct !== null
      ? `${chgPct > 0 ? "+" : ""}${chgPct.toLocaleString("tr-TR", { maximumFractionDigits: 2, minimumFractionDigits: 2 })}%`
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
              <div className="mt-1 text-xs text-slate-500">
                24s Değişim{" "}
                <span
                  title={chgPct !== null ? `${chg.symbol} 1d close-over-close` : chg.symbol === topSymbol ? "veri yok" : "yükleniyor…"}
                  className={cn(
                    "num",
                    chgPct === null ? "text-slate-400" : chgPct >= 0 ? "text-emerald-400" : "text-rose-400",
                  )}
                >
                  {chgPct !== null ? `${chg.symbol} ${chgTxt}` : chgTxt}
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
          <div className="flex items-center gap-2 text-[11px] uppercase tracking-wider text-slate-400">
            <TrendingUp className="h-3.5 w-3.5" aria-hidden />
            Toplam Kâr/Zarar · Profit
          </div>
          <div
            className={cn(
              "num mt-2 text-3xl font-bold tracking-tight",
              (fracToNum(summary?.totalRealizedGross) ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400",
            )}
          >
            {fmtUsdSigned(summary?.totalRealizedGross)}
          </div>
          <div className="mt-1 text-xs text-slate-500">
            Gerçekleşen (gross) · Gerçekleşmemiş{" "}
            <span className="num text-slate-400">{fmtUsdSigned(summary?.totalUnrealizedPnl ?? "0")}</span>
            {" "}· Komisyon <span className="num text-slate-400">{fmtUsd(summary?.totalFees, 4)}</span>
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
          <div className="num mt-2 flex justify-between text-[11px] text-slate-400">
            <span>L {fmtUsd(summary?.longExposure, 0)}</span>
            <span>S {fmtUsd(summary?.shortExposure, 0)}</span>
          </div>
        </div>
      </div>

      {/* Piyasa Nabzı — açılışta haber + sentiment (v3 başlangıç görünümü karşılığı) */}
      <NewsPulse symbol={topSymbol ?? "BTCUSDT"} />

      {/* Faz 2.8: Kaynak Durumu — haber beslemeleri + venue erişimi (v3 market-center karşılığı) */}
      <SourceStatus symbol={topSymbol ?? "BTCUSDT"} />

      {/* Middle: allocation + fleet */}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-3">
        <Panel title="Tahsis Dağılımı" icon={<Coins className="h-4 w-4 text-slate-500" aria-hidden />}>
          <Donut
            slices={donutSlices}
            centerLabel={fmtUsd(summary?.totalEquity, 0)}
            centerSub="TOPLAM"
          />
        </Panel>

        <Panel
          className="lg:col-span-2"
          title="Botlarım · My Bots"
          icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />}
          right={
            <Button size="sm" variant="outline" className="min-h-9 border-border-strong text-xs text-slate-300 hover:bg-secondary" onClick={onNewBot}>
              <ArrowRight className="h-3.5 w-3.5" aria-hidden />
              Yeni Bot
            </Button>
          }
          bodyClassName="p-3"
        >
          {bots.length === 0 ? (
            <EmptyState icon={Bot} title="Henüz bot yok" hint="Yeni Bot sihirbazıyla ilk DCA botunuzu başlatın." />
          ) : (
            <div className="thin-scroll max-h-72 space-y-2 overflow-y-auto pr-1">
              {bots.map((b) => (
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
                  </div>
                  <div className="hidden shrink-0 text-right sm:block">
                    <div className="text-[10px] uppercase text-slate-400">Kâr/Zarar · Profit</div>
                    <div
                      className={cn(
                        "num text-lg font-bold",
                        (fracToNum(b.stats.realized_gross) ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400",
                      )}
                    >
                      {fmtUsdSigned(b.stats.realized_gross)}
                    </div>
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
          title="Canlı Olay Akışı (SSE)"
          icon={<Radio className="h-4 w-4 text-slate-500" aria-hidden />}
          right={
            <span
              className={cn(
                "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[10px] font-semibold",
                connected ? "border-emerald-500/40 text-emerald-400" : "border-slate-600 text-slate-500",
              )}
            >
              <StatusDot status={connected ? "RUNNING" : "STOPPED"} />
              {connected ? "STREAM LIVE" : "STREAM OFF"}
            </span>
          }
          bodyClassName="p-0"
        >
          <div className="thin-scroll max-h-96 overflow-y-auto p-3 font-mono text-xs" aria-live="polite">
            {events.length === 0 ? (
              <EmptyState icon={Radio} title="Olay bekleniyor…" hint="SSE akışı bağlandığında fill'ler ve equity güncellemeleri burada belirir." />
            ) : (
              <ul className="space-y-1">
                {events.map((e) => (
                  <li key={e.id} className="flex items-start gap-2 rounded px-1 py-0.5 hover:bg-surface-2">
                    <span className="shrink-0 text-slate-500">{fmtTimeShort(e.at)}</span>
                    <StreamTypeChip type={e.type} payload={e.payload} />
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
                ))}
              </ul>
            )}
          </div>
        </Panel>

        <Panel title="Acil Durum Killswitch" icon={<Power className="h-4 w-4 text-rose-400" aria-hidden />}>
          <div className="flex h-full flex-col justify-between gap-4">
            <p className="text-xs leading-relaxed text-slate-400">
              Tüm botları tek seferde durdurur. Killswitch, <span className="text-rose-400">çift aşamalı onay</span> gerektirir:
              önce niyet, sonra <span className="num">KILL</span> ifadesi. Açık pozisyonlar korunur; yalnızca yeni emir
              üretimi durur.
            </p>
            <div className="rounded-lg border border-rose-500/20 bg-rose-500/5 p-3 text-[11px] text-slate-400">
              <div className="mb-1 flex items-center gap-1.5 font-semibold text-rose-400">
                <TrendingUp className="h-3.5 w-3.5" aria-hidden />
                Risk İlkeleri
              </div>
              <ul className="list-inside list-disc space-y-0.5">
                <li>8 gate aktif — yapısal ihlalde emir reddi (Gate 6/Zero-Withdrawal değişmez)</li>
                <li>10 piyasa koruması advisory: uyarı + onay, işlem engellemez (SIFIR İŞLEM ENGELİ)</li>
                <li>RISK olayları canlı akışta rozetle bildirilir — kullanıcı patron, risk kendi sorumluluğunda</li>
              </ul>
            </div>
            <KillButton className="w-full justify-center" />
          </div>
        </Panel>
      </div>
    </div>
  );
}
