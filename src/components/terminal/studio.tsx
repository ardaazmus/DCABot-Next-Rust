"use client";

import { useEffect, useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  Activity,
  ArrowLeft,
  Bot,
  ChartCandlestick,
  Clock,
  FlaskConical,
  Grid3x3,
  History,
  Info,
  ListTree,
  Loader2,
  Percent,
  Radio,
  Save,
  Settings2,
  ShieldAlert,
  Zap,
  ShieldCheck,
  ShieldHalf,
  Sigma,
  TriangleAlert,
  Webhook,
  Workflow,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  EmptyState,
  FillModelTag,
  KindBadge,
  ModeBadge,
  Panel,
  RoleChip,
  SideTag,
  Spinner,
  StatusPill,
  VenueChip,
} from "@/components/terminal/bits";
import { LadderTable } from "@/components/terminal/ladder";
import { TvChart, type TvPriceLine } from "@/components/terminal/tv-chart";
import dynamic from "next/dynamic";
// Lazy: backtest-panel pulls the heavy chart/backtest graph — split so the
// studio chunk stays small (sandbox compile memory peaks).
const StudioBacktestRunner = dynamic(
  () => import("@/components/terminal/backtest-panel").then((m) => m.StudioBacktestRunner),
  { ssr: false, loading: () => <div className="p-6 text-xs text-slate-500">Backtest modülü yükleniyor…</div> },
);
import { apiAiProviders, apiBotAction, apiBotGrid, apiBotMemoryGenerate, apiBotMemoryList, apiBotTick, apiCalcDca, apiDryRun, apiMarkets, apiParity, apiPatchBot, apiReconcile, getBotMeta, useBotDetail, useBotEvents, useFills } from "@/hooks/use-engine";
import { ForensicsTraces } from "@/components/terminal/forensics";
import { fmtFrac, fmtNum, fmtTimeShort, fmtTs, fmtUsd, fmtUsdSigned, fracToNum, truncateMid } from "@/lib/format";
import { MICRO_FALLBACK, microOf } from "@/lib/bot-defaults";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import { DEFAULT_RISK_POLICY } from "@/lib/engine-types";
import type { GridMeshResult, AdvisoryWarning, AiProvider, BotConfig, BotDetail, BotMemory, DryRunRequest, DryRunResult, Fill, GuardAction, ParityResult, ReconcileResult, UserRiskPolicy } from "@/lib/engine-types";
import { HedgeTab } from "@/components/terminal/studio/hedge-tab";
import { ConfigTab } from "@/components/terminal/studio/config-tab";

/* ------------------------------------------------------------------ */

function StatCell({ label, value, className }: { label: string; value: React.ReactNode; className?: string }) {
  return (
    <div className={cn("min-w-0 rounded-lg border border-border bg-surface p-3", className)}>
      <div className="truncate text-[10px] uppercase tracking-wider text-slate-400">{label}</div>
      <div className="num mt-1 truncate text-sm font-semibold text-slate-100">{value}</div>
    </div>
  );
}


/* Task 16 — Risk Motoru presentation helpers (Genel tab) */

type GuardKey =
  | "session_guard"
  | "opening_salvo_buffer"
  | "gap_consolidation"
  | "macro_blackout"
  | "regime_filter"
  | "m20_sl_ladder_guard"
  | "grid_fee_breaker";

const RISK_GUARDS: { label: string; key: GuardKey }[] = [
  { label: "W1 SEANS", key: "session_guard" },
  { label: "W8a SALVO", key: "opening_salvo_buffer" },
  { label: "W8b GAP", key: "gap_consolidation" },
  { label: "MAKRO", key: "macro_blackout" },
  { label: "REJİM", key: "regime_filter" },
  { label: "M20", key: "m20_sl_ladder_guard" },
  { label: "G_MIN", key: "grid_fee_breaker" },
];

function guardBadge(act: GuardAction): { text: string; cls: string } {
  return act === "ENFORCE"
    ? { text: "ZORUNLU", cls: "border-slate-600/60 bg-slate-600/10 text-slate-300" }
    : { text: "UYARI+İZİN", cls: "border-amber-500/50 bg-amber-500/10 text-amber-300" };
}

function sessionLabel(v: UserRiskPolicy["session_type"]): string {
  return v === "US_EQUITY" ? "ABD Hissesi" : v === "COMMODITY_23_5" ? "Emtia 23-5" : "Kripto 24/7";
}


/* ------------------------------------------------------------------ */

export function Studio({ botId, onBack }: { botId: string; onBack: () => void }) {
  const { bot, loading } = useBotDetail(botId);
  const { fills, loading: fillsLoading } = useFills(botId);
  const { events, loading: eventsLoading } = useBotEvents(botId);
  // Wizard-created bots stash their config in sessionStorage; read lazily per mount.
  const [configHint] = useState<BotConfig | null>(() => {
    if (typeof window === "undefined") return null;
    try {
      const raw = window.sessionStorage.getItem(`dcabot.cfg.${botId}`);
      return raw ? (JSON.parse(raw) as BotConfig) : null;
    } catch {
      return null;
    }
  });

  /* Task 18 S1 — GERÇEK M20 vurgusu: motorun kendi /api/calc/dca taramasını botun
     canlı konfigürasyonuyla (merdiven 1. seviye fiyatı çapa) çalıştırır; viable
     değilse firstUnreachableLevel'dan itibaren satırlar "M20 ULAŞILAMAZ" rozetli.
     Grid türlerinde DCA merdiveni yoktur → vurgu yok. */
  const [m20From, setM20From] = useState<number | null>(null);
  /* Parite + Mutabakat (v3 portu P1): Özet sekmesinde rozetler. */
  const [parity, setParity] = useState<ParityResult | null>(null);
  const [parityBusy, setParityBusy] = useState(false);
  const [parityErr, setParityErr] = useState<string | null>(null);
  const [recon, setRecon] = useState<ReconcileResult | null>(null);
  const [reconBusy, setReconBusy] = useState(false);
  const [reconErr, setReconErr] = useState<string | null>(null);
  /* Faz 2.3 bot hafızası — ölçülen bağlamın LLM özeti (ABSTAIN sözleşmeli). */
  const [memories, setMemories] = useState<BotMemory[]>([]);
  const [memBusy, setMemBusy] = useState(false);
  const [memGenBusy, setMemGenBusy] = useState(false);
  const [memErr, setMemErr] = useState<string | null>(null);
  const [memProviders, setMemProviders] = useState<AiProvider[]>([]);
  const [memProvider, setMemProvider] = useState("builtin");
  const [memModel, setMemModel] = useState("");
  const [memKey, setMemKey] = useState("");
  const [memBase, setMemBase] = useState("");
  const loadMemories = async () => {
    setMemBusy(true);
    setMemErr(null);
    try {
      const r = await apiBotMemoryList(botId, 20);
      setMemories(r.memories);
    } catch (e) {
      setMemErr(e instanceof Error ? e.message : "Hafıza okunamadı");
    } finally {
      setMemBusy(false);
    }
  };
  const runMemory = async () => {
    setMemGenBusy(true);
    setMemErr(null);
    try {
      await apiBotMemoryGenerate(botId, {
        provider: memProvider,
        model: memModel.trim() || undefined,
        apiKey: memKey || undefined,
        baseUrl: memBase.trim() || undefined,
      });
      await loadMemories();
    } catch (e) {
      setMemErr(e instanceof Error ? e.message : "Özet üretilemedi");
    } finally {
      setMemGenBusy(false);
    }
  };
  /* Faz 1.1 dry-run — 0-risk emir provası (bot sembolüyle ön-doldurulur). */
  const [drySide, setDrySide] = useState<"BUY" | "SELL">("BUY");
  const [dryType, setDryType] = useState<DryRunRequest["type"]>("LIMIT");
  const [dryQty, setDryQty] = useState("0.001");
  const [dryPrice, setDryPrice] = useState("");
  const [dryRun, setDryRun] = useState<DryRunResult | null>(null);
  const [dryBusy, setDryBusy] = useState(false);
  const [dryErr, setDryErr] = useState<string | null>(null);
  const runDry = async () => {
    setDryBusy(true);
    setDryErr(null);
    try {
      setDryRun(
        await apiDryRun({
          symbol: bot?.symbol ?? "",
          side: drySide,
          type: dryType,
          quantity: dryQty.trim() ? dryQty.trim() : undefined,
          price: dryPrice.trim() ? dryPrice.trim() : undefined,
          role: "base",
        }),
      );
    } catch (e) {
      setDryErr(e instanceof Error ? e.message : "Prova çalışmadı");
    } finally {
      setDryBusy(false);
    }
  };
  /* P3-8 grid mesh + manuel tick (yalnız grid botlarında anlamlı). */
  const [grid, setGrid] = useState<GridMeshResult | null>(null);
  const [gridBusy, setGridBusy] = useState(false);
  const [gridErr, setGridErr] = useState<string | null>(null);
  const [tickBusy, setTickBusy] = useState(false);
  const [tickInfo, setTickInfo] = useState<string | null>(null);
  const runGrid = async () => {
    setGridBusy(true);
    setGridErr(null);
    try {
      setGrid(await apiBotGrid(botId));
    } catch (e) {
      setGridErr(e instanceof Error ? e.message : "Mesh okunamadı");
    } finally {
      setGridBusy(false);
    }
  };
  const runTick = async () => {
    setTickBusy(true);
    try {
      const r = await apiBotTick(botId);
      setTickInfo(r.newBars > 0 ? `${r.newBars} yeni bar işlendi` : "yeni bar yok — durum taze");
    } catch (e) {
      setTickInfo(e instanceof Error ? e.message : "Tick çalışmadı");
    } finally {
      setTickBusy(false);
    }
  };

  const runParity = async () => {
    setParityBusy(true);
    setParityErr(null);
    try {
      setParity(await apiParity({ bot_id: botId, bars: 500 }));
    } catch (e) {
      setParityErr(e instanceof Error ? e.message : "Bilinmeyen hata");
    } finally {
      setParityBusy(false);
    }
  };
  const runReconcile = async () => {
    setReconBusy(true);
    setReconErr(null);
    try {
      setRecon(await apiReconcile(botId));
    } catch (e) {
      setReconErr(e instanceof Error ? e.message : "Bilinmeyen hata");
    } finally {
      setReconBusy(false);
    }
  };
  useEffect(() => {
    void loadMemories();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [botId]);
  useEffect(() => {
    let alive = true;
    apiAiProviders()
      .then((r) => { if (alive) setMemProviders(r.providers); })
      .catch(() => { if (alive) setMemProviders([]); });
    return () => { alive = false; };
  }, []);
  useEffect(() => {
    let dead = false;
    const cfg = bot?.config ?? configHint ?? null;
    if (!bot || !cfg || cfg.kind.includes("GRID")) return;
    // Ladder/avg fiyatları "n/d" rasyonel string olarak gelir; calc endpoint'i
    // ondalık bekler → fracToNum ile çevir (Task 18 S1 fix).
    const anchorRow = (bot.ladder ?? []).find((r) => r.level === 1);
    const anchorNum = anchorRow ? fracToNum(anchorRow.price) : null;
    const avgNum = bot.stats.avg_price && bot.stats.avg_price !== "0" ? fracToNum(bot.stats.avg_price) : null;
    const anchor = anchorNum !== null && anchorNum > 0 ? String(anchorNum) : avgNum !== null && avgNum > 0 ? String(avgNum) : null;
    const sl = cfg.stop_loss_pct;
    if (!anchor || !sl) return;
    const isFutures = cfg.kind.includes("FUTURES");
    const category = isFutures ? "linear" : "spot";
    // Mikro-yapi BORSADAN gelir (sabit degil): enstruman yoksa kategori fallback'i.
    apiMarkets(category, bot.symbol)
      .then((m) => microOf(m.instruments?.[0] ?? null, category))
      .catch(() => MICRO_FALLBACK[category])
      .then((micro) => {
        if (dead) return;
        return apiCalcDca({
          anchor,
          base_order_size: cfg.base_order_size,
          safety_order_size: cfg.safety_order_size,
          sizing_mode: cfg.sizing_mode ?? "QUOTE_NOTIONAL",
          deviation_pct: cfg.deviation_pct,
          step_scale: cfg.step_scale,
          volume_scale: cfg.volume_scale,
          max_safety_orders: cfg.max_safety_orders,
          target_profit_pct: cfg.target_profit_pct,
          use_net_tp: cfg.use_net_tp ?? true,
          stop_loss_pct: sl,
          tick_size: micro.tickSize,
          step_size: micro.stepSize,
          maker_fee: micro.makerFee,
          taker_fee: micro.takerFee,
          leverage: cfg.leverage ?? "1",
          market: category,
        });
      })
      .then((r) => {
        if (!dead && r) setM20From(r.m20.viable ? null : r.m20.firstUnreachableLevel);
      })
      .catch(() => {
        if (!dead) setM20From(null);
      });
    return () => {
      dead = true;
    };
  }, [bot, configHint]);

  if (loading && !bot) {
    return (
      <div className="flex items-center justify-center py-24">
        <Spinner className="h-8 w-8" />
      </div>
    );
  }
  if (!bot) {
    return (
      <EmptyState
        icon={Bot}
        title="Bot bulunamadı"
        hint="Bot silinmiş olabilir. Filoya dönüp listeyi yenileyin."
        className="py-24"
      />
    );
  }

  const s = bot.stats;
  const isGrid = bot.kind.includes("GRID");
  const isHedge = bot.kind.includes("HEDGE");

  /* Task 16: Risk Motoru — policy echo (legacy bots → engine defaults) + advisory warnings. */
  const rp: UserRiskPolicy = bot.riskPolicy ?? DEFAULT_RISK_POLICY;
  const riskWarnings: AdvisoryWarning[] = bot.warnings ?? [];

  /* Backtest tab: derive the runner config from the REAL bot (engine snapshot
     → wizard hint) instead of a hardcoded fallback. */
  const backtestConfig: BotConfig = (() => {
    const cfg = bot.config ?? configHint;
    if (cfg) return { ...cfg, kind: bot.kind, symbol: bot.symbol };
    return defaultBotConfig(bot.kind, bot.symbol);
  })();
  const botCashNum = fracToNum(bot.cash);

  /* TradingView chart overlay: live bot levels (avg/TP/SL/next safety/liq).
     Plain derivation (no hook — this runs after the early returns above). */
  const chartLines: TvPriceLine[] = (() => {
    const lines: TvPriceLine[] = [];
    const push = (v: string | null | undefined, color: string, title: string, dashed = false) => {
      const p = fracToNum(v ?? null);
      if (p && p > 0) lines.push({ price: p, color, title, dashed });
    };
    push(s.avg_price, "#22d3ee", "AVG");
    push(s.tp_price, "#10b981", "TP", true);
    push(s.sl_price, "#f43f5e", "SL", true);
    push(s.next_safety_price, "#f59e0b", `S${s.next_safety_level ?? ""}`, true);
    push(bot.liqPrice, "#f43f5e", "LIQ", true);
    return lines.slice(0, 8);
  })();

  return (
    <motion.div initial={{ opacity: 0, x: 16 }} animate={{ opacity: 1, x: 0 }} transition={{ duration: 0.22 }} className="space-y-4">
      <div className="flex flex-wrap items-center gap-3">
        <Button variant="outline" size="sm" className="min-h-10 border-border-strong text-slate-300 hover:bg-secondary" onClick={onBack}>
          <ArrowLeft className="h-4 w-4" aria-hidden />
          Filoya Dön
        </Button>
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-2">
            <h2 className="truncate text-lg font-bold text-slate-50">{bot.name}</h2>
            <KindBadge kind={bot.kind} />
            <VenueChip venue={bot.venue} />
            <ModeBadge mode={bot.mode} />
            <StatusPill status={bot.status} />
          </div>
          <div className="num mt-0.5 text-[11px] text-slate-400">
            {bot.id} · {bot.symbol} · {bot.interval}
          </div>
        </div>
      </div>

      <Tabs defaultValue="genel">
        <TabsList className="flex h-auto w-full flex-wrap gap-1 border-b border-border bg-transparent p-0 pb-1 rounded-none">
          {([
            ["genel", "Genel", <Bot key="i" className="h-3.5 w-3.5" />],
            ...(isHedge ? ([["hedge", "Hedge", <ShieldHalf key="ih" className="h-3.5 w-3.5" />]] as [string, string, React.ReactNode][]) : []),
            ["fills", "Fill'ler", <ListTree key="i2" className="h-3.5 w-3.5" />],
            ["ladder", "Ladder", <ListTree key="i3" className="h-3.5 w-3.5" />],
            ["ayarlar", "Ayarlar", <Settings2 key="i4" className="h-3.5 w-3.5" />],
            ["olaylar", "Olaylar", <Radio key="i5" className="h-3.5 w-3.5" />],
            ["backtest", "Backtest", <FlaskConical key="i6" className="h-3.5 w-3.5" />],
            ["ozet", "Özet", <Sigma key="i7" className="h-3.5 w-3.5" />],
          ] as [string, string, React.ReactNode][]).map(([v, label, icon]) => (
            <TabsTrigger
              key={String(v)}
              value={String(v)}
              className="min-h-10 gap-1.5 rounded-md border border-transparent px-3 text-xs text-slate-400 data-[state=active]:border-border-strong data-[state=active]:bg-[#151a24] data-[state=active]:text-slate-100"
            >
              {icon}
              {String(label)}
            </TabsTrigger>
          ))}
        </TabsList>

        {/* ---------- Genel ---------- */}
        <TabsContent value="genel" className="mt-4 space-y-4">
          {/* Faz 21 (N-8): kernel sticky-halt artık GÖRÜNÜR — kırmızı şerit +
              açma düğmesi. Eski davranışta bot "görünmez şekilde duruyor",
              faz rozeti Open kalıyor, hiçbir emir gitmiyordu. */}
          {bot.kernelHalted && (
            <div className="rounded-lg border border-rose-500/50 bg-rose-500/10 p-3" role="alert">
              <div className="flex items-center gap-2 text-xs font-bold text-rose-300">
                <TriangleAlert className="h-4 w-4" aria-hidden />
                KERNEL DURDURULDU (GERÇEK VERİ BOZULMASI)
              </div>
              <p className="mt-1.5 text-[11px] leading-snug text-rose-200/80">
                {bot.kernelHaltedReason || "Kernel kalıcı halt durumunda — hiçbir emir gönderilmiyor."} Reset
                sonrası bot bir sonraki kapalı bardan warmup ile devam eder; gerekirse pozisyonu Filo&apos;dan
                manuel kapatabilirsiniz.
              </p>
              <Button
                size="sm"
                variant="outline"
                className="mt-2 min-h-9 border-rose-500/60 text-xs text-rose-300 hover:bg-rose-500/10"
                onClick={() => {
                  void apiBotAction(bot.id, "RESET_KERNEL")
                    .then(() => toast({ title: "Kernel sıfırlandı", description: "Bir sonraki kapalı bardan devam eder." }))
                    .catch((e) => toast({ title: "Sıfırlama başarısız", description: String(e) }));
                }}
              >
                KERNEL&apos;İ SIFIRLA
              </Button>
            </div>
          )}
          {riskWarnings.length > 0 && (
            <div className="rounded-lg border border-amber-500/50 bg-amber-500/5 p-3" role="status">
              <div className="flex items-center gap-2 text-xs font-bold text-amber-300">
                <TriangleAlert className="h-4 w-4" aria-hidden />
                RİSK MOTORU UYARILARI ({riskWarnings.length})
              </div>
              <ul className="mt-2 space-y-1.5">
                {riskWarnings.map((w) => (
                  <li key={w.code} className="flex items-start gap-2 text-[11px] leading-snug">
                    {w.severity === "WARN" ? (
                      <TriangleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0 text-amber-400" aria-hidden />
                    ) : (
                      <Info className="mt-0.5 h-3.5 w-3.5 shrink-0 text-slate-500" aria-hidden />
                    )}
                    <span className={w.severity === "WARN" ? "text-amber-200/90" : "text-slate-400"}>
                      <span className="font-semibold">{w.title}:</span> {w.message}
                    </span>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
            <StatCell label="FAZ" value={<span className="text-cyan-300">{s.phase}</span>} />
            <StatCell label="DEAL ID" value={<span title={s.deal_id ?? ""}>{truncateMid(s.deal_id ?? "—", 12, 6)}</span>} />
            <StatCell label="POZİSYON" value={fmtFrac(s.position_qty)} />
            <StatCell label="ORTALAMA (WAC)" value={fmtFrac(s.avg_price)} />
            <StatCell label="TP FİYATI" value={<span className="text-emerald-400">{fmtFrac(s.tp_price)}</span>} />
            <StatCell label="SL FİYATI" value={s.sl_price ? <span className="text-rose-400">{fmtFrac(s.sl_price)}</span> : "—"} />
            <StatCell label="SONRAKİ GÜVENLİK" value={<span className="text-cyan-400">L{s.next_safety_level ?? "—"} @ {fmtFrac(s.next_safety_price)}</span>} />
            <StatCell label="TRAILING" value={s.trailing_state} />
            <StatCell label="TRAILING PEAK" value={fmtFrac(s.trailing_peak)} />
            <StatCell label="GÜVENLİK TETİK" value={String(s.safety_triggered)} />
            <StatCell label="FUNDING" value={fmtUsdSigned(s.cumulative_funding, 4)} />
            <StatCell label="NAKİT" value={fmtUsd(bot.cash)} />
            {bot.marginMode && (
              <>
                <StatCell label="MARJİN MODU" value={<span className="text-amber-400">{`İZOLE ${fmtNum(fracToNum(bot.leverage), 0)}x`}</span>} />
                <StatCell label="KOMİSYE MARJİN (IM)" value={<span className="text-amber-300">{fmtUsd(bot.imUsed)}</span>} />
                <StatCell
                  label="LİKİDASYON (M07)"
                  value={bot.liqPrice ? <span className="text-rose-400">{fmtFrac(bot.liqPrice)}</span> : "—"}
                />
              </>
            )}
            <StatCell
              label="BAŞLANGIÇ KOŞULU"
              value={
                <span
                  className={
                    s.entry_trigger_met
                      ? s.entry_trigger === "INSTANT"
                        ? "text-slate-100"
                        : "text-emerald-400"
                      : "text-amber-400"
                  }
                >
                  {s.entry_trigger}
                  {s.entry_trigger !== "INSTANT" && (s.entry_trigger_met ? " ✓" : " · bekliyor")}
                </span>
              }
            />
          </div>
          {/* Faz 20 (UIX-12): Closing fazı artık kör bekleme değil — P0-6
              koruması ve kullanıcının elindeki manuel tetik açıkça söylenir. */}
          {s.phase === "Closing" && (
            <div className="mt-3 flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-2.5" role="status">
              <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-400" aria-hidden />
              <p className="text-[11px] leading-snug text-amber-200/90">
                <span className="font-semibold">Deal kapanıyor:</span> kapanış emri piyasada. Motor 3 bar içinde dolum görmezse kapanışı
                kendisi yeniden gönderir (P0-6 koruması) — pozisyon sahipsiz kalmaz. Acele ederseniz Filo ekranındaki
                <span className="font-semibold"> &quot;Tümünü Kapat&quot;</span> ile manuel kapanış da tetikleyebilirsiniz.
              </p>
            </div>
          )}
          <Panel title="Risk Motoru (Piyasa Korumaları)" icon={<ShieldAlert className="h-4 w-4 text-slate-500" aria-hidden />}>
            <div className="flex flex-wrap items-center gap-1.5">
              {RISK_GUARDS.map((g) => {
                const b = guardBadge(rp[g.key]);
                return (
                  <span
                    key={g.key}
                    className="inline-flex items-center gap-1 rounded border border-border bg-surface px-1.5 py-0.5 text-[10px] font-semibold"
                  >
                    <span className="text-slate-500">{g.label}</span>
                    <span className={cn("rounded px-1 py-0.5 text-[9px] font-bold", b.cls)}>{b.text}</span>
                  </span>
                );
              })}
            </div>
            <div className="mt-3 grid grid-cols-2 gap-3 sm:grid-cols-4">
              <StatCell
                label="GÜNLÜK DD KESİCİ"
                value={rp.daily_dd_breaker_pct === "0" ? <span className="text-amber-400">KAPALI</span> : `%${rp.daily_dd_breaker_pct}`}
              />
              <StatCell label="GÜVENLİK EMİRLERİ" value={`${rp.active_safety_orders} AKTİF`} />
              <StatCell
                label="DOLUM"
                value={rp.fill_model === "REALISTIC" ? "GERÇEKÇİ" : <span className="text-amber-400">İYİMSER</span>}
              />
              <StatCell
                label="SEANS"
                value={sessionLabel(rp.session_type)}
                className={rp.session_type !== "CRYPTO_24_7" ? "ring-1 ring-inset ring-amber-500/30" : undefined}
              />
            </div>
            {!bot.riskPolicy && (
              <p className="mt-2 text-[10px] text-slate-400">
                Bu snapshot&apos;ta risk politikası alanı yok (eski bot) — motor varsayılanları gösteriliyor.
              </p>
            )}
          </Panel>
          {!s.entry_trigger_met && (
            <div className="flex items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-300">
              <Clock className="h-3.5 w-3.5 shrink-0" aria-hidden />
              Bot başlangıç koşulunu bekliyor ({s.entry_trigger}). Koşul bir bar kapanışında sağlandığında ilk deal otomatik açılır — beklerken pozisyon yok, ücret yok.
            </div>
          )}
          {s.entry_signal_met != null && (
            <div
              className={cn(
                "flex items-center gap-2 rounded-md border p-3 text-xs",
                s.entry_signal_met ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300" : "border-amber-500/40 bg-amber-500/10 text-amber-300",
              )}
            >
              <Workflow className="h-3.5 w-3.5 shrink-0" aria-hidden />
              {s.entry_signal_met
                ? "Sinyal ağacı şu an SAĞLANIYOR — yeni deal açılışına izin verildi."
                : "Sinyal ağacı şu an SAĞLANMIYOR — yeni deal açılışı fail-closed bekliyor (zero look-ahead, kapanmış barlar)."}
            </div>
          )}
          <Panel title="Piyasa Grafiği (TradingView)" icon={<ChartCandlestick className="h-4 w-4 text-slate-500" aria-hidden />} bodyClassName="p-2">
            <TvChart symbol={bot.symbol} interval={bot.interval} height={360} bars={240} priceLines={chartLines} />
            {chartLines.length > 0 && (
              <div className="flex flex-wrap gap-1.5 px-1 pt-2">
                {chartLines.map((l) => (
                  <span key={l.title + String(l.price)} className="rounded bg-secondary px-1.5 py-0.5 font-mono text-[10px] text-slate-400">
                    <span className="inline-block h-2 w-2 rounded-sm align-middle" style={{ background: l.color }} aria-hidden /> {l.title} {l.price}
                  </span>
                ))}
              </div>
            )}
          </Panel>
          <Panel title="Deal Durumu" icon={<Percent className="h-4 w-4 text-slate-500" aria-hidden />}>
            <p className="text-xs leading-relaxed text-slate-400">
              {isHedge
                ? `Hedge motoru (${s.hedge?.mode === "DUAL" ? "çift bacak" : "koruma"} modu): ana bacak ${s.position_qty === "0" ? "pozisyonsuz" : "pozisyonda"}, hedge bacağı ${s.hedge?.hedge_leg ? "AÇIK" : "yok"}. Birleşik k/z ${fmtUsdSigned(s.hedge?.combined_pnl ?? "0", 4)} · hedef ${fmtUsd(s.hedge?.combined_target ?? "0")}. Bacak kartları, tetikleyici takibi ve spawn geçmihi Hedge sekmesinde.`
                : isGrid
                  ? "Grid stratejisi: mesh seviyeleri ARMED durumda bekler; fiyat dokunuşu offset fill üretir."
                  : `Tek aktif deal invariant: base doldu, ${s.safety_triggered} güvenlik emri tetiklendi. Sonraki güvenlik L${
                      s.next_safety_level ?? "—"
                    } seviyesinde ${fmtFrac(s.next_safety_price)} fiyatında bekliyor. Net TP hedefi ${fmtFrac(s.tp_price)}.`}
            </p>
          </Panel>
        </TabsContent>

        {/* ---------- Hedge (Task 21) ---------- */}
        {isHedge && (
          <TabsContent value="hedge" className="mt-4">
            <HedgeTab bot={bot} fills={fills} />
          </TabsContent>
        )}

        {/* ---------- Fill'ler ---------- */}
        <TabsContent value="fills" className="mt-4">
          {fillsLoading && fills.length === 0 ? (
            <div className="flex justify-center py-10">
              <Spinner className="h-6 w-6" />
            </div>
          ) : fills.length === 0 ? (
            <EmptyState icon={ListTree} title="Fill yok" hint="Emirler dolduğunda burada listelenir." />
          ) : (
            <div className="thin-scroll max-h-96 overflow-y-auto rounded-lg border border-border">
              <Table>
                <TableHeader className="sticky top-0 z-10 bg-surface">
                  <TableRow className="hover:bg-transparent border-border">
                    <TableHead className="h-9 text-[11px] text-slate-400">ZAMAN</TableHead>
                    <TableHead className="h-9 text-[11px] text-slate-400">ROL</TableHead>
                    <TableHead className="h-9 text-[11px] text-slate-400">YÖN</TableHead>
                    <TableHead className="h-9 text-right text-[11px] text-slate-400">FİYAT</TableHead>
                    <TableHead className="h-9 text-right text-[11px] text-slate-400">MİKTAR</TableHead>
                    <TableHead className="h-9 text-right text-[11px] text-slate-400">ÜCRET</TableHead>
                    <TableHead className="h-9 text-[11px] text-slate-400">MODEL</TableHead>
                    <TableHead className="h-9 text-[11px] text-slate-400">ORDER</TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {fills.map((f) => (
                    <TableRow key={f.fillKey} className="border-border hover:bg-surface-2">
                      <TableCell className="num whitespace-nowrap py-2 text-[11px] text-slate-400">{fmtTs(f.ts)}</TableCell>
                      <TableCell className="py-2">
                        <RoleChip role={f.role} />
                      </TableCell>
                      <TableCell className="py-2">
                        <SideTag side={f.side} />
                      </TableCell>
                      <TableCell className="num py-2 text-right text-slate-100">{fmtFrac(f.price)}</TableCell>
                      <TableCell className="num py-2 text-right text-slate-300">{fmtFrac(f.qty)}</TableCell>
                      <TableCell className="num py-2 text-right text-amber-400/90">
                        {fmtFrac(f.fee, 4)} <span className="text-[9px] text-slate-400">{f.feeRole}</span>
                      </TableCell>
                      <TableCell className="py-2">
                        <FillModelTag model={f.fillModel} />
                      </TableCell>
                      <TableCell className="num py-2 text-[10px] text-slate-400" title={f.orderId}>
                        {truncateMid(f.orderId, 10, 4)}
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </TabsContent>

        {/* ---------- Ladder ---------- */}
        <TabsContent value="ladder" className="mt-4 space-y-3">
          <div className="flex flex-wrap items-center gap-2 text-xs text-slate-500">
            <span>
              {isHedge ? "Ana bacak (long) DCA merdiveni" : isGrid ? "Grid mesh" : "DCA güvenlik merdiveni"} — {bot.ladder?.length ?? 0} seviye
              {s.sl_price ? " · SL aktif (M20 denetimi creation'da yapıldı)" : ""}
            </span>
          </div>
          <LadderTable rows={bot.ladder ?? []} currentLevel={s.next_safety_level} m20From={m20From} kind={bot.kind} />
        </TabsContent>

        {/* ---------- Ayarlar ---------- */}
        <TabsContent value="ayarlar" className="mt-4 space-y-4">
          <Panel title="TradingView Webhook (3Commas Sinyali)" icon={<Webhook className="h-4 w-4 text-slate-500" aria-hidden />}>
            <p className="text-xs leading-relaxed text-slate-400">
              TradingView alert mesajını aşağıdaki URL&apos;e yönlendirin (webhook sekmesi). Payload:
              <code className="ml-1 rounded bg-secondary px-1 py-0.5 font-mono text-[10px] text-cyan-300">
                {`{"secret":"<gizli>","botId":"${botId}","action":"START_DEAL","timestamp":<ms>}`}
              </code>
              . Replay koruması ±60 sn; deal açıkken sinyal yoksayılır (DEAL_ALREADY_ACTIVE, fail-closed).
            </p>
            <div className="mt-3 flex flex-col gap-2 sm:flex-row sm:items-center">
              <Input
                readOnly
                value={bot.webhookSecret ?? ""}
                className="num min-h-10 flex-1 border-border-strong bg-surface font-mono text-xs text-slate-300"
                aria-label="Webhook gizli anahtarı"
              />
              <Button
                variant="outline"
                size="sm"
                className="min-h-10 border-border-strong text-xs text-slate-300 hover:bg-secondary"
                onClick={async () => {
                  const secret = bot.webhookSecret ?? "";
                  try {
                    await navigator.clipboard.writeText(secret);
                    toast({ title: "Kopyalandı", description: "Webhook secret panoya alındı." });
                  } catch {
                    toast({
                      title: "Kopyalanamadı",
                      description: "Tarayıcı panoya erişemedi — gizli anahtarı el ile seçip kopyalayın.",
                      variant: "destructive",
                    });
                  }
                }}
              >
                SECRET KOPYALA
              </Button>
            </div>
            <div className="mt-2 flex flex-col gap-2 sm:flex-row sm:items-center">
              <Input
                readOnly
                value={`/api/webhooks/tradingview`}
                className="num min-h-10 flex-1 border-border-strong bg-surface font-mono text-xs text-cyan-300"
                aria-label="Webhook endpoint yolu"
              />
              <Button
                variant="outline"
                size="sm"
                className="min-h-10 border-border-strong text-xs text-slate-300 hover:bg-secondary"
                onClick={async () => {
                  try {
                    await navigator.clipboard.writeText("/api/webhooks/tradingview");
                    toast({ title: "Kopyalandı", description: "Webhook endpoint yolu panoya alındı." });
                  } catch {
                    toast({
                      title: "Kopyalanamadı",
                      description: "Tarayıcı panoya erişemedi — endpoint yolunu el ile kopyalayın.",
                      variant: "destructive",
                    });
                  }
                }}
              >
                URL KOPYALA
              </Button>
            </div>
            <p className="mt-2 text-[11px] text-slate-400">
              Endpoint, terminalinizin kök adresine göre görevidir (aynı origin + port). Örnek tam URL: <code className="font-mono text-cyan-300">https://&lt;terminal-adresi&gt;/api/webhooks/tradingview</code>
            </p>
          </Panel>
          <ConfigTab botId={botId} configHint={configHint} />
        </TabsContent>

        {/* ---------- Olaylar ---------- */}
        <TabsContent value="olaylar" className="mt-4 space-y-4">
          <ForensicsTraces botId={botId} />
          {eventsLoading && events.length === 0 ? (
            <div className="flex justify-center py-10">
              <Spinner className="h-6 w-6" />
            </div>
          ) : events.length === 0 ? (
            <EmptyState icon={Radio} title="Olay kaydı yok" />
          ) : (
            <div className="thin-scroll max-h-96 space-y-1 overflow-y-auto rounded-lg border border-border bg-surface p-3 font-mono text-xs">
              {events.map((e, i) => (
                <div key={`${e.ts}-${i}`} className="flex items-start gap-2 rounded px-1 py-0.5 hover:bg-surface-2">
                  <span className="shrink-0 text-slate-500">{fmtTimeShort(e.ts)}</span>
                  <span
                    className={cn(
                      "shrink-0 font-bold",
                      e.level === "ERROR" ? "text-rose-400" : e.level === "WARN" ? "text-amber-400" : "text-emerald-400",
                    )}
                  >
                    {e.level}
                  </span>
                  {e.gate && <span className="shrink-0 rounded bg-secondary px-1 text-[10px] text-cyan-400">{e.gate}</span>}
                  <span className="min-w-0 flex-1 break-words text-slate-300">{e.message}</span>
                </div>
              ))}
            </div>
          )}
        </TabsContent>

        {/* ---------- Backtest ---------- */}
        <TabsContent value="backtest" className="mt-4 space-y-3">          {isHedge && (
            <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-300">
              Motor backtest'i Hedge ailesini desteklemiyor (<code className="font-mono">DcaEngine requires DCA_LONG/DCA_SHORT/FUTURES_DCA</code> → 422).
              Hedge stratejisini PAPER modda küçük sermayeyle canlı test edin; backtest sonucu üretilemez.
            </div>
          )}
          {/* Task 18 S5: konfigürasyon kaynağı motor örneği/sihirbaz hint'i değilse
             dürüstçe söyle — sessiz varsayılanlarla koşan backtest yanıltıcı olur. */}
          {!(bot.config ?? configHint) && (
            <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-300">
              Bu botun motor örneğinden okunan bir konfigürasyonu yok (eski/legacy kayıt) —
              backtest, gösterim amaçlı varsayılan parametrelerle koşuyor. Gerçek bot
              davranışını temsil etmez; parametreleri Ayarlar sekmesinden kaydedin.
            </div>
          )}
          <StudioBacktestRunner
            kind={bot.kind}
            symbol={bot.symbol}
            interval={bot.interval}
            config={backtestConfig}
            startCash={botCashNum !== null && botCashNum > 0 ? String(botCashNum) : undefined}
          />
        </TabsContent>

        {/* ---------- Özet ---------- */}
        <TabsContent value="ozet" className="mt-4 space-y-4">
          <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 xl:grid-cols-6">
            <StatCell label="KAPANAN DEAL" value={String(s.deals_closed)} />
            <StatCell label="KAZANAN" value={<span className="text-emerald-400">{s.wins}</span>} />
            <StatCell label="KAYBEDEN" value={<span className="text-rose-400">{s.losses}</span>} />
            <StatCell
              label="WIN RATE"
              value={s.deals_closed > 0 ? `${Math.round((s.wins / s.deals_closed) * 100)}%` : "—"}
            />
            <StatCell label="REALIZED GROSS" value={<span className="text-slate-100">{fmtUsdSigned(s.realized_gross)}</span>} />
            <StatCell label="KÜMÜLATİF ÜCRET" value={<span className="text-amber-400">{fmtUsd(bot.cumulativeFees, 4)}</span>} />
          </div>
          <div className="grid gap-3 lg:grid-cols-2">
            <Panel title="Parite · Backtest ↔ Paper" icon={<Activity className="h-4 w-4 text-slate-500" aria-hidden />}>
              <div className="flex items-center justify-between gap-2">
                <p className="text-[11px] text-slate-400">Son 500 barda fill adedi + fill-başı ücret eşleşmesi.</p>
                <Button size="sm" onClick={runParity} disabled={parityBusy} className="min-h-9 shrink-0">
                  {parityBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Kontrol Et"}
                </Button>
              </div>
              {parityErr && <p className="mt-2 text-xs text-rose-400">{parityErr}</p>}
              {parity && (
                <div className="num mt-2 space-y-1.5 text-xs">
                  <div className="flex items-center gap-2">
                    <span
                      className={cn(
                        "rounded px-1.5 py-0.5 font-mono text-[11px] font-bold",
                        parity.verdict === "GECTI" && "bg-emerald-500/15 text-emerald-300",
                        parity.verdict === "KALDI" && "bg-rose-500/15 text-rose-300",
                        parity.verdict === "YETERSIZ_VERI" && "bg-amber-500/15 text-amber-300",
                      )}
                    >
                      {parity.verdict === "GECTI" ? "GEÇTİ" : parity.verdict === "KALDI" ? "KALDI" : "YETERSİZ VERİ"}
                    </span>
                    <span className="text-slate-400">
                      adet %{parity.countMatchPct} · ücret %{parity.feeMatchPct}
                    </span>
                  </div>
                  <div className="flex flex-wrap gap-x-4 gap-y-1 text-slate-400">
                    <span>
                      backtest fill: <span className="text-slate-200">{parity.btFills}</span>
                    </span>
                    <span>
                      paper fill: <span className="text-slate-200">{parity.paperFills}</span>
                    </span>
                    <span>
                      win-rate (bt): <span className="text-slate-200">%{parity.winRateBt}</span>
                    </span>
                  </div>
                  <p className="text-[10px] leading-relaxed text-slate-400">{parity.note}</p>
                </div>
              )}
            </Panel>
            <Panel title="Mutabakat · Borsa ↔ Yerel" icon={<ShieldCheck className="h-4 w-4 text-slate-500" aria-hidden />}>
              <div className="flex items-center justify-between gap-2">
                <p className="text-[11px] text-slate-400">Canlı pozisyon borsa ile denk mi? (raporlama — otomatik durdurma yok)</p>
                <Button size="sm" onClick={runReconcile} disabled={reconBusy} className="min-h-9 shrink-0">
                  {reconBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Mutabakat"}
                </Button>
              </div>
              {reconErr && <p className="mt-2 text-xs text-rose-400">{reconErr}</p>}
              {recon && !recon.supported && <p className="mt-2 text-xs text-slate-400">{recon.reason}</p>}
              {recon?.supported && (
                <div className="num mt-2 space-y-1.5 text-xs">
                  <div className="flex items-center gap-2">
                    <span
                      className={cn(
                        "rounded px-1.5 py-0.5 font-mono text-[11px] font-bold",
                        recon.match ? "bg-emerald-500/15 text-emerald-300" : "bg-rose-500/15 text-rose-300",
                      )}
                    >
                      {recon.match ? "DENK" : "TUTARSIZ"}
                    </span>
                    <span className="text-slate-400">
                      yerel {recon.local?.side} {recon.local?.qty}
                    </span>
                  </div>
                  {(recon.exchange ?? []).map((p, i) => (
                    <div key={i} className="text-slate-400">
                      borsa {p.side} {p.size} @ {p.avgPrice}
                    </div>
                  ))}
                  {recon.note && <p className="text-[10px] leading-relaxed text-slate-400">{recon.note}</p>}
                </div>
              )}
            </Panel>
            <Panel title="Hafıza · AI Durum Özeti" icon={<History className="h-4 w-4 text-slate-500" aria-hidden />}>
              <p className="text-[11px] text-slate-400">Ölçülen bağlamın Türkçe özeti — model yalnız motor verisini görür, anahtar gövdede yaşar (saklanmaz). Veri yoksa içerik üretilmez.</p>
              <div className="mt-2 grid grid-cols-2 gap-2">
                <div>
                  <Label className="text-[10px] text-slate-400">Sağlayıcı</Label>
                  <select value={memProvider} onChange={(e) => setMemProvider(e.target.value)} className="mt-1 w-full rounded border border-border-strong bg-surface px-2 py-1.5 text-xs text-slate-200" aria-label="Hafıza sağlayıcısı">
                    {(memProviders.length > 0 ? memProviders : [{ id: "builtin", label: "Yerleşik" } as AiProvider]).map((p) => (
                      <option key={p.id} value={p.id}>{p.label}</option>
                    ))}
                  </select>
                </div>
                <div>
                  <Label className="text-[10px] text-slate-400">Model (boş → varsayılan)</Label>
                  <Input value={memModel} onChange={(e) => setMemModel(e.target.value)} placeholder="gpt-4o-mini" className="mt-1 border-border-strong bg-surface text-xs" aria-label="Hafıza modeli" />
                </div>
                <div>
                  <Label className="text-[10px] text-slate-400">API anahtarı (saklanmaz)</Label>
                  <Input type="password" value={memKey} onChange={(e) => setMemKey(e.target.value)} placeholder="••••" className="mt-1 border-border-strong bg-surface text-xs" aria-label="Hafıza API anahtarı" />
                </div>
                <div>
                  <Label className="text-[10px] text-slate-400">Base URL (yerel için)</Label>
                  <Input value={memBase} onChange={(e) => setMemBase(e.target.value)} placeholder="http://localhost:11434" className="mt-1 border-border-strong bg-surface text-xs" aria-label="Hafıza base URL" />
                </div>
              </div>
              <div className="mt-2 flex items-center justify-between gap-2">
                <span className="text-[10px] text-slate-400">{memories.length > 0 ? `${memories.length} kayıt` : memBusy ? "yükleniyor…" : "henüz kayıt yok"}</span>
                <Button size="sm" onClick={() => void runMemory()} disabled={memGenBusy} className="min-h-9 shrink-0">
                  {memGenBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Özet Üret"}
                </Button>
              </div>
              {memErr && <p className="mt-2 text-xs text-rose-400">{memErr}</p>}
              {memories.length > 0 && (() => {
                const latest = memories[0];
                return (
                  <div className="mt-2 space-y-1.5 text-xs">
                    <div className="flex items-center gap-2">
                      <span
                        className={cn(
                          "rounded px-1.5 py-0.5 font-mono text-[11px] font-bold",
                          latest.verdict === "OK" && "bg-emerald-500/15 text-emerald-300",
                          latest.verdict === "ABSTAIN" && "bg-amber-500/15 text-amber-300",
                          latest.verdict === "ERROR" && "bg-rose-500/15 text-rose-300",
                        )}
                      >
                        {latest.verdict === "OK" ? "ÖZET" : latest.verdict === "ABSTAIN" ? "ÇEKİMSER" : "HATA"}
                      </span>
                      <span className="text-slate-400">{latest.model} · {fmtTs(latest.createdAt)}</span>
                    </div>
                    {latest.content
                      ? <p className="whitespace-pre-line leading-relaxed text-slate-300">{latest.content}</p>
                      : <p className="leading-relaxed text-slate-500">{latest.note ?? "İçerik üretilmedi."}</p>}
                    {latest.content && latest.note && <p className="text-[10px] leading-relaxed text-slate-400">{latest.note}</p>}
                    {memories.length > 1 && (
                      <div className="space-y-1 border-t border-border pt-1.5">
                        {memories.slice(1, 6).map((m) => (
                          <div key={m.id} className="flex items-center gap-2 text-[11px] text-slate-400">
                            <span className={cn("font-mono font-bold", m.verdict === "OK" ? "text-emerald-400/80" : m.verdict === "ABSTAIN" ? "text-amber-400/80" : "text-rose-400/80")}>{m.verdict}</span>
                            <span>{fmtTs(m.createdAt)}</span>
                            <span className="truncate">{m.model}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })()}
            </Panel>
            <Panel title="Dry-Run · 0-Risk Emir Provası" icon={<Zap className="h-4 w-4 text-slate-500" aria-hidden />}>
              <p className="text-[11px] text-slate-400">M08 tick/step/minNotional hizalaması — ağa emir gitmez (sent:false).</p>
              <div className="mt-2 grid grid-cols-2 gap-2">
                <div>
                  <Label className="text-[10px] text-slate-400">Yön</Label>
                  <select value={drySide} onChange={(e) => setDrySide(e.target.value as "BUY" | "SELL")} className="mt-1 w-full rounded border border-border-strong bg-surface px-2 py-1.5 text-xs text-slate-200" aria-label="Dry-run yönü">
                    <option value="BUY">BUY</option>
                    <option value="SELL">SELL</option>
                  </select>
                </div>
                <div>
                  <Label className="text-[10px] text-slate-400">Tip</Label>
                  <select value={dryType} onChange={(e) => setDryType(e.target.value as DryRunRequest["type"])} className="mt-1 w-full rounded border border-border-strong bg-surface px-2 py-1.5 text-xs text-slate-200" aria-label="Dry-run emir tipi">
                    <option value="LIMIT">LIMIT</option>
                    <option value="MARKET">MARKET</option>
                    <option value="STOP_MARKET">STOP_MARKET</option>
                    <option value="TAKE_PROFIT_MARKET">TAKE_PROFIT_MARKET</option>
                  </select>
                </div>
                <div>
                  <Label className="text-[10px] text-slate-400">Miktar (coin)</Label>
                  <Input value={dryQty} onChange={(e) => setDryQty(e.target.value)} placeholder="0.001" className="num mt-1 border-border-strong bg-surface text-xs" aria-label="Dry-run miktarı" />
                </div>
                <div>
                  <Label className="text-[10px] text-slate-400">Fiyat (LIMIT)</Label>
                  <Input value={dryPrice} onChange={(e) => setDryPrice(e.target.value)} placeholder="boş → MARKET ticker" className="num mt-1 border-border-strong bg-surface text-xs" aria-label="Dry-run fiyatı" />
                </div>
              </div>
              <div className="mt-2 flex items-center justify-between gap-2">
                <span className="num text-[10px] text-slate-400">{bot?.symbol}</span>
                <Button size="sm" onClick={() => void runDry()} disabled={dryBusy} className="min-h-9 shrink-0">
                  {dryBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "0-Risk Dene"}
                </Button>
              </div>
              {dryErr && <p className="mt-2 text-xs text-rose-400">{dryErr}</p>}
              {dryRun && (
                <div className="num mt-2 space-y-1.5 text-xs">
                  <div className="flex items-center gap-2">
                    <span className={dryRun.ok ? "rounded bg-emerald-500/15 px-1.5 py-0.5 font-mono text-[11px] font-bold text-emerald-300" : "rounded bg-rose-500/15 px-1.5 py-0.5 font-mono text-[11px] font-bold text-rose-300"}>
                      {dryRun.ok ? "GÖNDERİLEBİLİR" : "GÖNDERİLEMEZ"}
                    </span>
                    <span className="text-slate-500">sent:false · {dryRun.venue} {dryRun.category}</span>
                  </div>
                  {dryRun.alignment.map((a, i) => (
                    <div key={i} className="text-slate-400">
                      {a.field}: {a.raw} → <span className="text-slate-200">{a.aligned}</span> <span className="text-slate-500">({a.rule})</span>
                    </div>
                  ))}
                  <div className="break-all text-slate-400">{JSON.stringify(dryRun.order)}</div>
                  {dryRun.reason && <p className="text-rose-400">{dryRun.reason}</p>}
                  <p className="text-[10px] leading-relaxed text-slate-400">{dryRun.note}</p>
                </div>
              )}
            </Panel>
          </div>
          {/* P3-8 grid mesh + manuel tick — yalnız grid botlarında */}
          {isGrid && (
            <div className="grid gap-3 lg:grid-cols-2">
              <Panel title="Grid Mesh · Canlı Seviyeler" icon={<Grid3x3 className="h-4 w-4 text-slate-500" aria-hidden />}>
                <div className="flex items-center justify-between gap-2">
                  <p className="text-[11px] text-slate-400">Motor mesh'i + armed seviyeler (bekleyen emir).</p>
                  <Button size="sm" onClick={() => void runGrid()} disabled={gridBusy} className="min-h-9 shrink-0">
                    {gridBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Mesh"}
                  </Button>
                </div>
                {gridErr && <p className="mt-2 text-xs text-rose-400">{gridErr}</p>}
                {grid && (
                  <div className="num mt-2 space-y-1.5 text-xs">
                    <div className="flex items-center gap-2">
                      <span className={cn("rounded px-1.5 py-0.5 font-mono text-[11px] font-bold", grid.bootstrapped ? "bg-emerald-500/15 text-emerald-300" : "bg-amber-500/15 text-amber-300")}>
                        {grid.bootstrapped ? "KURULU" : "BAŞLANGIÇ BEKLİYOR"}
                      </span>
                      <span className="text-slate-400">{grid.levels} seviye · {grid.armed.filter(Boolean).length} armed</span>
                    </div>
                    <div className="flex max-h-40 flex-wrap gap-1 overflow-y-auto">
                      {grid.mesh.map((m, i) => (
                        <span key={i} className={cn("rounded border px-1.5 py-0.5 font-mono text-[10px]", grid.armed[i] ? "border-emerald-500/50 bg-emerald-500/10 text-emerald-300" : "border-border-strong text-slate-500")}>{m}</span>
                      ))}
                    </div>
                    <p className="text-[10px] leading-relaxed text-slate-400">{grid.note}</p>
                  </div>
                )}
              </Panel>
              <Panel title="Manuel Tick" icon={<Zap className="h-4 w-4 text-slate-500" aria-hidden />}>
                <p className="text-[11px] text-slate-400">Motor zaten periyodik poll yapar; bu düğme turu hemen koşturur.</p>
                <div className="mt-2 flex items-center gap-2">
                  <Button size="sm" onClick={() => void runTick()} disabled={tickBusy} className="min-h-9 shrink-0">
                    {tickBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : "Tick Çalıştır"}
                  </Button>
                  {tickInfo && <span className="text-xs text-slate-400">{tickInfo}</span>}
                </div>
              </Panel>
            </div>
          )}
          <Panel title="Hesap Özeti" icon={<History className="h-4 w-4 text-slate-500" aria-hidden />}>
            <div className="num grid grid-cols-2 gap-3 text-xs sm:grid-cols-4">
              <div>
                <div className="text-slate-500">NAKİT</div>
                <div className="mt-0.5 text-slate-200">{fmtUsd(bot.cash)}</div>
              </div>
              <div>
                <div className="text-slate-500">POZİSYON DEĞERİ</div>
                <div className="mt-0.5 text-slate-200">
                  {fmtUsd(String((fracToNum(s.position_qty) ?? 0) * (fracToNum(s.avg_price) ?? 0)))}
                </div>
              </div>
              <div>
                <div className="text-slate-500">FUNDING</div>
                <div className="mt-0.5 text-slate-200">{fmtUsdSigned(s.cumulative_funding, 4)}</div>
              </div>
              <div>
                <div className="text-slate-500">GÜVENLİK TETİK</div>
                <div className="mt-0.5 text-slate-200">{s.safety_triggered}</div>
              </div>
            </div>
          </Panel>
        </TabsContent>
      </Tabs>
    </motion.div>
  );
}

function defaultBotConfig(kind: string, symbol: string): BotConfig {
  return {
    kind: kind as BotConfig["kind"],
    symbol,
    base_order_size: "500",
    safety_order_size: "500",
    sizing_mode: "QUOTE_NOTIONAL",
    deviation_pct: "0.02",
    step_scale: "1.3",
    volume_scale: "1.2",
    max_safety_orders: 5,
    target_profit_pct: "0.02",
    use_net_tp: true,
    trailing_enabled: false,
    trailing_trigger_pct: "0.01",
    trailing_deviation_pct: "0.005",
    stop_loss_pct: null,
    leverage: "1",
    hedge_ratio: "0.5",
    combined_target_profit: "50",
    grid_lower: null,
    grid_upper: null,
    grid_levels: null,
    grid_mode: "arithmetic",
    grid_trail: false,
    cooldown_bars: 0,
    entry_trigger: "INSTANT",
  };
}
