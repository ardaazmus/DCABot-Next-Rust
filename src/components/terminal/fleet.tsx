"use client";

import { useMemo, useState } from "react";
import { motion } from "framer-motion";
import {
  ArrowDown,
  ArrowUp,
  Bot,
  Clock,
  Eye,
  LayoutGrid,
  List,
  Pause,
  Play,
  ScrollText,
  Snowflake,
  Trash2,
  TriangleAlert,
  XCircle,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
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
  KindBadge,
  ModeBadge,
  Panel,
  Spinner,
  StatusPill,
  VenueChip,
} from "@/components/terminal/bits";
import { apiBotAction, apiDeleteBot, apiPatchBot, useBots, useFleetTape } from "@/hooks/use-engine";
import { fmtFrac, fmtTimeShort, fmtTs, fmtUsd, fmtUsdSigned, fracToNum, kindLabel } from "@/lib/format";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import { CRITICAL_LIQUIDATION_DISTANCE_PCT, DealProgressBar } from "@/components/terminal/deal-progress-bar";
import type { Bot as BotRow, Mode } from "@/lib/engine-types";

/* Task 16 — compact risk-policy chips for fleet cards/table (max 2). */
const GUARD_KEYS = [
  "session_guard",
  "opening_salvo_buffer",
  "gap_consolidation",
  "macro_blackout",
  "regime_filter",
  "m20_sl_ladder_guard",
  "grid_fee_breaker",
] as const;

/// Faz 27 (UIX-43): izole marj erimesi rozeti — unrealized / IM oranı.
/// %50 eşik: M07 likidasyon ~%100 IM erimesinde tetiklendiğinden, erimenin
/// yarısı "likidasyona yakın" operasyonel uyarıdır (arda1 vakası dersi).
function MarginErodeBadge({ bot }: { bot: BotRow }) {
  const im = fracToNum(bot.imUsed) ?? 0;
  if (!bot.marginMode || im <= 0) return null;
  const unreal = fracToNum(bot.unrealizedNow) ?? 0;
  const erode = unreal < 0 ? (-unreal / im) * 100 : 0;
  const danger = erode >= 50;
  return (
    <div
      className={cn(
        "num mt-2 rounded-md border px-2 py-1.5 text-[10px] font-semibold",
        danger
          ? "border-rose-500/50 bg-rose-500/10 text-rose-300"
          : "border-border bg-surface text-slate-400",
      )}
      title="Açık deal unrealized / komisye marj (IM) — %100 erime M07 likidasyonudur"
    >
      MARJ KULLANIMI {fmtUsd(bot.imUsed, 2)} · ERİME %{erode.toFixed(1)}
      {danger && " — LİKİDASYONA YAKIN (M07)"}
    </div>
  );
}

function hasLiquidationRisk(bot: BotRow): boolean {
  if (bot.stats.phase !== "Open" || !bot.marginMode) return false;

  const im = fracToNum(bot.imUsed) ?? 0;
  const unreal = fracToNum(bot.unrealizedNow) ?? 0;
  const marginErodeDanger = im > 0 && unreal < 0 && (-unreal / im) * 100 >= 50;

  const liq = fracToNum(bot.liqPrice) ?? 0;
  const current = fracToNum(bot.markPrice) ?? 0;
  let criticalPriceDistance = false;
  if (liq > 0 && current > 0) {
    const isShort = bot.stats.position_direction === -1 || (bot.stats.position_direction == null && bot.kind === "DCA_SHORT");
    const distancePct = Math.max(0, isShort ? ((liq - current) / current) * 100 : ((current - liq) / current) * 100);
    criticalPriceDistance = distancePct < CRITICAL_LIQUIDATION_DISTANCE_PCT;
  }

  return marginErodeDanger || criticalPriceDistance;
}

function RiskWarningBadge({ bot, className }: { bot: BotRow; className?: string }) {
  const n = bot.warnings?.length ?? 0;
  if (n === 0) return null;
  return (
    <span
      title={bot.warnings?.[0]?.title}
      className={cn(
        "inline-flex items-center gap-1 rounded border border-amber-500/50 bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-bold text-amber-300",
        className,
      )}
    >
      <TriangleAlert className="h-3 w-3" aria-hidden />
      {n} RİSK
    </span>
  );
}

/* Faz 25 (UIX-24): "3 KORUMA UYARI" 13 kartın hepsinde aynıydı — bilgi değeri
   sıfır, sarı alarm yorgunluğu üretiyordu. Politika filo-ortaktır; artık TEK
   şeritte söylenir, kartta yalnız bota özgü RiskWarningBadge kalır. */
function GlobalPolicyStrip({ bots }: { bots: BotRow[] }) {
  const rp = bots[0]?.riskPolicy;
  if (!rp) return null;
  const permissive = GUARD_KEYS.filter((k) => rp[k] === "PERMISSIVE_WITH_WARNING").length;
  return (
    <div className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-border bg-surface px-3 py-2 text-[11px] text-slate-400" role="note">
      <span className="font-semibold text-slate-300">Piyasa koruması — tüm filoda aynı politika:</span>
      <span className="num">
        {permissive > 0 ? `${permissive} koruma uyarı+izin` : "tüm korumalar ZORUNLU"} · DD kesici{" "}
        {rp.daily_dd_breaker_pct === "0" ? <span className="text-amber-400">KAPALI</span> : `%${rp.daily_dd_breaker_pct}`} · dolum{" "}
        {rp.fill_model === "REALISTIC" ? "GERÇEKÇİ" : "İYİMSER"}
      </span>
      <span className="text-slate-500">bota özgü ayrıntı: İncele → Genel</span>
    </div>
  );
}

/* Faz 2.8 — hedge-grup filtresi: v3 hedge-grup listesinin karşılığı (istemci-taraflı). */
const HEDGE_KINDS = new Set(["HEDGE_PROTECTIVE", "HEDGE_DUAL"]);

/* R3 (denetim): kart + tablo görünümünün ortak parçaları — tek sahip, iki kullanım. */
function BotIdentityBadges({ bot }: { bot: BotRow }) {
  return (
    <>
      <KindBadge kind={bot.kind} />
      <VenueChip venue={bot.venue} />
      <ModeBadge mode={bot.mode} />
    </>
  );
}

function EntryTriggerNote({ bot, compact }: { bot: BotRow; compact?: boolean }) {
  const t = bot.stats.entry_trigger;
  if (!t || t === "INSTANT") return null;
  /* Faz 25 (UIX-18): motor "SCHEDULED @ 1793488800000ms" diye ham epoch
     basıyordu — insan tarihine çevrilir; ham değer title'da kalır.
     (String.match — salt biçimlendirme, hiçbir kabuk/komut çağrısı yok.) */
  const sched = t.trim().match(/^(SCHEDULED|SIGNAL)\s*@\s*(\d+)ms$/);
  const shown = sched ? `${sched[1] === "SCHEDULED" ? "PLANLI" : "SİNYAL"}: ${fmtTs(Number(sched[2]))}` : t;
  const rawTitle = sched ? `Ham değer: ${t}` : undefined;
  if (compact) {
    return (
      <div className={cn("num mt-1 text-[10px] font-semibold", bot.stats.entry_trigger_met ? "text-emerald-400" : "text-amber-400")} title={rawTitle}>
        {shown}
        {!bot.stats.entry_trigger_met && " · bekliyor"}
      </div>
    );
  }
  return (
    <div
      className={cn(
        "mt-2 flex items-center gap-1.5 rounded-md border px-2 py-1.5 text-[10px] font-semibold",
        bot.stats.entry_trigger_met
          ? "border-emerald-500/40 bg-emerald-500/5 text-emerald-300"
          : "border-amber-500/40 bg-amber-500/5 text-amber-300",
      )}
      title={rawTitle}
    >
      <Clock className="h-3 w-3 shrink-0" aria-hidden />
      <span className="num truncate">BAŞLANGIÇ: {shown}</span>
      {!bot.stats.entry_trigger_met && <span className="ml-auto shrink-0">KOŞUL BEKLİYOR</span>}
    </div>
  );
}

function DeleteBotDialog({ bot, onRemove, triggerClassName, disabled, withLabel = false }: { bot: BotRow; onRemove: (b: BotRow) => void; triggerClassName: string; disabled?: boolean; withLabel?: boolean }) {
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        {/* Faz 25 (UIX-37): "Kapat" (deal kapatma) ile "Sil" (bot+geçmiş) en
            kritik yıkıcı ayırtım — sil düğmesi kartta etiketli, tabloda
            başlıklı ikon. */}
        <Button size="sm" variant="outline" className={triggerClassName} disabled={disabled} title="Botu ve geçmişini kalıcı sil">
          <Trash2 className="h-3.5 w-3.5" aria-hidden />
          {withLabel && "Sil"}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent className="border-border-strong bg-[#151a24]">
        <AlertDialogHeader>
          <AlertDialogTitle className="text-slate-100">Bot silinsin mi?</AlertDialogTitle>
          <AlertDialogDescription className="text-slate-400">
            <span className="font-semibold text-slate-200">{bot.name}</span> ({kindLabel(bot.kind)} · {bot.symbol}) ve
            tüm geçmişi kalıcı olarak silinecek.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel className="border-border-strong bg-transparent text-slate-300 hover:bg-secondary hover:text-slate-100">
            Vazgeç
          </AlertDialogCancel>
          <AlertDialogAction
            className="bg-rose-500 text-white hover:bg-rose-600"
            onClick={() => {
              void onRemove(bot);
            }}
          >
            Sil
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

function CloseAllDialog({ bot, onClose, triggerClassName, disabled }: { bot: import("@/lib/engine-types").Bot; onClose: (b: import("@/lib/engine-types").Bot) => void; triggerClassName: string; disabled?: boolean }) {
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button size="sm" variant="outline" className={triggerClassName} disabled={disabled} title="Tüm açık emirleri iptal et ve pozisyonu kapat">
          <XCircle className="h-3.5 w-3.5" aria-hidden />
          Kapat
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent className="border-border-strong bg-[#151a24]">
        <AlertDialogHeader>
          <AlertDialogTitle className="text-slate-100">Tüm emirleri kapat?</AlertDialogTitle>
          <AlertDialogDescription className="text-slate-400">
            <span className="font-semibold text-slate-200">{bot.name}</span> ({kindLabel(bot.kind)} · {bot.symbol}) için tüm açık limit emirleri iptal edilecek ve pozisyon pazarda kapatılmaya çalışılacak. Bu işlem geri alınamaz.
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel className="border-border-strong bg-transparent text-slate-300 hover:bg-secondary hover:text-slate-100">
            Vazgeç
          </AlertDialogCancel>
          <AlertDialogAction
            className="bg-rose-500 text-white hover:bg-rose-600"
            onClick={() => {
              void onClose(bot);
            }}
          >
            Onayla ve Kapat
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

export function FleetTape() {
  const { tape, loading } = useFleetTape();
  return (
    <Panel
      title={`Son Dolumlar (tape${tape.length > 0 ? ` · ${tape.length}` : ""})`}
      icon={<ScrollText className="h-4 w-4 text-slate-500" aria-hidden />}
    >
      {loading && tape.length === 0 ? (
        <div className="flex items-center justify-center py-6">
          <Spinner className="h-5 w-5" />
        </div>
      ) : tape.length === 0 ? (
        <p className="py-4 text-center text-xs text-slate-500">Henüz dolum yok — deal açılıp kapandıkça son 40 fill burada listelenir.</p>
      ) : (
        <div className="thin-scroll max-h-64 space-y-1 overflow-y-auto">
          {tape.map((t, i) => (
            <div
              key={`${t.ts}-${t.botId}-${i}`}
              title={`rol: ${t.role} · komisyon: ${fmtFrac(t.fee)}`}
              className="num flex items-center gap-2 rounded px-2 py-1 text-[11px] hover:bg-surface-2"
            >
              <span className="shrink-0 text-slate-500">{fmtTimeShort(t.ts)}</span>
              <span className="min-w-0 flex-1 truncate text-slate-200">{t.botName}</span>
              <VenueChip venue={t.venue} />
              <span
                className={t.side === "Buy" ? "shrink-0 font-semibold text-emerald-400" : "shrink-0 font-semibold text-rose-400"}
              >
                {t.side === "Buy" ? "ALIM" : "SATIM"}
              </span>
              <span className="shrink-0 text-slate-300">{fmtFrac(t.qty)} @ {fmtFrac(t.price)}</span>
              <span className="hidden shrink-0 text-slate-500 xl:inline">kom {fmtFrac(t.fee)}</span>
            </div>
          ))}
        </div>
      )}
    </Panel>
  );
}

export function Fleet({ onInspect, onNewBot }: { onInspect: (id: string) => void; onNewBot: (mode?: Mode) => void }) {
  const { bots, loading, refetch } = useBots();
  const [view, setView] = useState<"cards" | "table">("cards");
  const [busyId, setBusyId] = useState<string | null>(null);
  // Faz 24 (UI/UX Pro Max raporu §3.2.1): toplu seçim — checkbox ile çoklu
  // bot başlat/duraklat. Seçim kart/tablo ortak id kümesi olarak tutulur.
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [closeAllDialogId, setCloseAllDialogId] = useState<string | null>(null);
  const [confirmation, setConfirmation] = useState<{
    title: string;
    description: string;
    confirmLabel: string;
    tone: "danger" | "normal";
    onConfirm: () => void | Promise<void>;
  } | null>(null);
  const [hedgeOnly, setHedgeOnly] = useState(false);
  /* Faz 25 (UIX-25): filo arama + durum filtresi + kart sıralaması.
     Varsayılan sıra = K/Z büyüklüğü (kayıp/kâr en büyük en üstte) —
     sektör standardı (Bitsgap/3Commas) ve en büyük kayıp listede gömülü kalmasın. */
  const [query, setQuery] = useState("");
  const [statusF, setStatusF] = useState<"ALL" | "RUNNING" | "PAUSED" | "FROZEN" | "STOPPED">("ALL");
  const [modeF, setModeF] = useState<Mode | "ALL">("ALL");
  type SmartFilter = "ALL" | "OPEN_DEAL" | "BAG_HOLDING" | "RISK_MARGIN" | "HALT_WARN" | "STOPPED";
  const [smartF, setSmartF] = useState<SmartFilter>("ALL");
  const [cardSort, setCardSort] = useState<"impact" | "pnl-asc" | "pnl-desc" | "name" | "symbol">("impact");
  const modeCounts = useMemo(
    () => ({
      ALL: bots.length,
      PAPER: bots.filter((b) => b.mode === "PAPER").length,
      LIVE: bots.filter((b) => b.mode === "LIVE").length,
    }),
    [bots],
  );
  const changeModeFilter = (nextMode: Mode | "ALL") => {
    if (nextMode !== modeF) setSelected(new Set());
    setModeF(nextMode);
  };
  const hedgeCount = bots.filter((b) => HEDGE_KINDS.has(b.kind)).length;
  const openDealsCount = useMemo(() => bots.filter((b) => b.stats.phase === "Open").length, [bots]);
  const bagHoldingCount = useMemo(() => {
    return bots.filter(
      (b) =>
        b.stats.phase === "Open" &&
        b.config?.max_safety_orders != null &&
        b.stats.safety_triggered >= b.config.max_safety_orders,
    ).length;
  }, [bots]);
  const riskMarginCount = useMemo(() => {
    return bots.filter(hasLiquidationRisk).length;
  }, [bots]);
  const haltWarnCount = useMemo(() => bots.filter((b) => b.kernelHalted || (b.warnings?.length ?? 0) > 0).length, [bots]);
  const stoppedCount = useMemo(() => bots.filter((b) => b.status === "STOPPED").length, [bots]);
  const runningBots = useMemo(() => bots.filter((b) => b.status === "RUNNING"), [bots]);
  const pausedBots = useMemo(() => bots.filter((b) => b.status === "PAUSED"), [bots]);

  const shown = useMemo(() => {
    const q = query.trim().toLocaleLowerCase("tr");
    let list = bots;
    if (modeF !== "ALL") list = list.filter((b) => b.mode === modeF);
    if (hedgeOnly) list = list.filter((b) => HEDGE_KINDS.has(b.kind));
    if (smartF === "OPEN_DEAL") list = list.filter((b) => b.stats.phase === "Open");
    else if (smartF === "BAG_HOLDING") {
      list = list.filter(
        (b) =>
          b.stats.phase === "Open" &&
          b.config?.max_safety_orders != null &&
          b.stats.safety_triggered >= b.config.max_safety_orders,
      );
    }
    else if (smartF === "RISK_MARGIN") {
      list = list.filter(hasLiquidationRisk);
    } else if (smartF === "HALT_WARN") {
      list = list.filter((b) => b.kernelHalted || (b.warnings?.length ?? 0) > 0);
    } else if (smartF === "STOPPED") {
      list = list.filter((b) => b.status === "STOPPED");
    }
    if (statusF !== "ALL") list = list.filter((b) => b.status === statusF);
    if (q) list = list.filter((b) => b.name.toLocaleLowerCase("tr").includes(q) || b.symbol.toLocaleLowerCase("tr").includes(q));
    if (view === "cards") {
      const pnl = (b: BotRow) => fracToNum(b.stats.realized_gross) ?? 0;
      list = [...list].sort((a, b) => {
        switch (cardSort) {
          case "impact":
            return Math.abs(pnl(b)) - Math.abs(pnl(a));
          case "pnl-asc":
            return pnl(a) - pnl(b);
          case "pnl-desc":
            return pnl(b) - pnl(a);
          case "name":
            return a.name.localeCompare(b.name, "tr");
          case "symbol":
            return a.symbol.localeCompare(b.symbol, "tr") || a.name.localeCompare(b.name, "tr");
        }
      });
    }
    return list;
  }, [bots, modeF, hedgeOnly, smartF, statusF, query, view, cardSort]);
  /* P7 — tablo sıralaması (aria-sort). Kart görünümü etkilenmez. */
  const [sortKey, setSortKey] = useState<"name" | "status" | "pnl" | null>(null);
  const [sortDir, setSortDir] = useState<1 | -1>(1);
  const sorted = useMemo(() => {
    if (!sortKey) return shown;
    const val = (b: BotRow): string | number =>
      sortKey === "name" ? b.name.toLocaleLowerCase("tr") : sortKey === "status" ? b.status : (fracToNum(b.stats.realized_gross) ?? 0);
    return [...shown].sort((a, b) => {
      const va = val(a);
      const vb = val(b);
      const c = typeof va === "number" && typeof vb === "number" ? va - vb : String(va).localeCompare(String(vb), "tr");
      return c * sortDir;
    });
  }, [shown, sortKey, sortDir]);
  const toggleSort = (key: "name" | "status" | "pnl") => {
    if (sortKey !== key) {
      setSortKey(key);
      setSortDir(1);
    } else if (sortDir === 1) {
      setSortDir(-1);
    } else {
      setSortKey(null);
      setSortDir(1);
    }
  };
  const sortTh = (key: "name" | "status" | "pnl", label: string, numeric = false) => (
    <button
      type="button"
      onClick={() => toggleSort(key)}
      aria-label={`${label} sütununa göre sırala`}
      className="inline-flex items-center gap-1 font-semibold hover:text-slate-200"
    >
      {label}
      {sortKey === key ? (
        sortDir === 1 ? (
          <ArrowUp className="h-3 w-3" aria-hidden />
        ) : (
          <ArrowDown className="h-3 w-3" aria-hidden />
        )
      ) : (
        <span className="text-slate-600" aria-hidden>
          ↕
        </span>
      )}
    </button>
  );

  const withBusy = async (id: string, fn: () => Promise<void>) => {
    setBusyId(id);
    try {
      await fn();
      refetch();
    } catch (e) {
      toast({
        title: "İşlem başarısız",
        description: e instanceof Error ? e.message : "Bilinmeyen hata",
        variant: "destructive",
      });
    } finally {
      setBusyId(null);
    }
  };

  const togglePause = (b: BotRow) =>
    withBusy(b.id, async () => {
      const target = b.status === "RUNNING" ? "PAUSED" : "RUNNING";
      await apiPatchBot(b.id, { status: target });
      toast({ title: `${b.name}`, description: `Durum → ${target}` });
    });

  /* Faz 24 (UI/UX Pro Max raporu §3.2.3): PANIC — açık pozisyonu marketten
     kapat (CLOSE_ALL) ve botu durdur (STOP). İki aşamalı onaylı. */
  const panicClose = (b: BotRow) => {
    setConfirmation({
      title: `${b.name} için PANIC işlemi başlatılsın mı?`,
      description: "Açık pozisyon marketten kapatılacak ve bot durdurulacak. Emir gönderildikten sonra fiyat kayması oluşabilir.",
      confirmLabel: "Marketten Kapat ve Durdur",
      tone: "danger",
      onConfirm: async () => {
      try {
        await apiBotAction(b.id, "CLOSE_ALL");
        await apiPatchBot(b.id, { status: "STOPPED" });
        toast({ title: `${b.name} PANIC`, description: "Market çıkışı gönderildi, bot STOPPED." });
        refetch();
      } catch (e) {
        toast({ title: "PANIC başarısız", description: String(e) });
      }
      },
    });
  };

  /* Faz 24 (UI/UX Pro Max raporu §3.2.1): toplu eylem — seçili botlara
     tek tek apiBotAction uygular (motor serial işler), bitince refetch. */
  const bulkAction = (action: string, label: string) => {
    const ids = Array.from(selected);
    if (ids.length === 0) return;
    setConfirmation({
      title: `${label} onayı`,
      description: `Bu işlem ${ids.length} seçili bota uygulanacak.`,
      confirmLabel: label,
      tone: "normal",
      onConfirm: async () => {
      let ok = 0;
      for (const bid of ids) {
        try {
          await apiBotAction(bid, action);
          ok += 1;
        } catch {
          /* tek bot hatası toplu akışı bozmaz — olay kaydı motor tarafında */
        }
      }
      setSelected(new Set());
      refetch();
      toast({ title: label, description: `${ok}/${ids.length} bot uygulandı.` });
      },
    });
  };

  /* P2-2 Freeze: RUNNING → FROZEN (yeni emir yok, açıklar korunur), geri dönüş RUNNING. */
  const toggleFreeze = (b: BotRow) =>
    withBusy(b.id, async () => {
      const action = b.status === "FROZEN" ? "UNFREEZE" : "FREEZE";
      await apiBotAction(b.id, action);
      toast({ title: `${b.name}`, description: action === "FREEZE" ? "Donduruldu — yeni emir üretilmez, açıklar korunur." : "Çözüldü → RUNNING" });
    });

  const closeAll = (b: BotRow) => {
    setCloseAllDialogId(b.id);
  };

  const executeCloseAll = async (b: BotRow) => {
    setCloseAllDialogId(null);
    await withBusy(b.id, async () => {
      await apiBotAction(b.id, "CLOSE_ALL");
      toast({ title: `${b.name}`, description: "CLOSE_ALL emri gönderildi — açık emirler iptal ediliyor, pozisyon kapatılıyor." });
    });
  };

  const remove = (b: BotRow) =>
    withBusy(b.id, async () => {
      await apiDeleteBot(b.id);
      toast({ title: `${b.name}`, description: "Bot silindi." });
    });

  const handleBulkPauseAll = async () => {
    if (runningBots.length === 0) return;
    const targets = [...runningBots];
    setConfirmation({
      title: "Çalışan botlar duraklatılsın mı?",
      description: `${targets.length} bot duraklatılacak. Açık pozisyonlar korunur; yeni emir üretilmez.`,
      confirmLabel: "Tümünü Duraklat",
      tone: "danger",
      onConfirm: async () => {
        let ok = 0;
        for (const b of targets) {
          try {
            await apiPatchBot(b.id, { status: "PAUSED" });
            ok++;
          } catch {
            /* devam et */
          }
        }
        refetch();
        toast({ title: "Toplu Duraklatma", description: `${ok}/${targets.length} bot duraklatıldı.` });
      },
    });
  };

  const handleBulkResumeAll = async () => {
    if (pausedBots.length === 0) return;
    const targets = [...pausedBots];
    setConfirmation({
      title: "Duraklatılmış botlar başlatılsın mı?",
      description: `${targets.length} bot yeniden RUNNING durumuna alınacak. Giriş kapıları emir açmayı engelleyebilir.`,
      confirmLabel: "Tümünü Başlat",
      tone: "normal",
      onConfirm: async () => {
        let ok = 0;
        for (const b of targets) {
          try {
            await apiPatchBot(b.id, { status: "RUNNING" });
            ok++;
          } catch {
            /* devam et */
          }
        }
        refetch();
        toast({ title: "Toplu Başlatma", description: `${ok}/${targets.length} bot başlatıldı.` });
      },
    });
  };

  return (
    <div className="space-y-4">
      <Panel
        title={`Bot Filosu (${shown.length}${shown.length !== bots.length ? ` / ${bots.length}` : ""})`}
        icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />}
        right={
          <div className="flex flex-wrap items-center justify-end gap-1.5">
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Ara: bot / sembol"
              aria-label="Filo arama (bot adı veya sembol)"
              className="h-9 w-36 rounded-lg border border-border-strong bg-surface px-2.5 font-mono text-[11px] text-slate-200 placeholder:text-slate-500 focus:outline-none focus:ring-1 focus:ring-emerald-500/40"
            />
            <select
              value={statusF}
              onChange={(e) => setStatusF(e.target.value as typeof statusF)}
              aria-label="Durum filtresi"
              className="h-9 rounded-lg border border-border-strong bg-surface px-2 text-[11px] font-semibold text-slate-300"
            >
              <option value="ALL">Durum: Tümü</option>
              <option value="RUNNING">Running</option>
              <option value="PAUSED">Paused</option>
              <option value="FROZEN">Frozen</option>
              <option value="STOPPED">Stopped</option>
            </select>
            {view === "cards" && (
              <select
                value={cardSort}
                onChange={(e) => setCardSort(e.target.value as typeof cardSort)}
                aria-label="Kart sıralaması"
                className="h-9 rounded-lg border border-border-strong bg-surface px-2 text-[11px] font-semibold text-slate-300"
              >
                <option value="impact">Sırala: K/Z büyüklüğü</option>
                <option value="pnl-desc">K/Z: kâr → zarar</option>
                <option value="pnl-asc">K/Z: zarar → kâr</option>
                <option value="name">Ad (A-Z)</option>
                <option value="symbol">Sembol</option>
              </select>
            )}
            <button
              aria-pressed={hedgeOnly}
              title="Yalnız hedge botları göster (HEDGE_PROTECTIVE + HEDGE_DUAL)"
              onClick={() => setHedgeOnly((v) => !v)}
              className={hedgeOnly
                ? "num rounded-lg border border-amber-500/50 bg-amber-500/10 px-2.5 py-1 text-[11px] font-semibold text-amber-300"
                : "num rounded-lg border border-border-strong px-2.5 py-1 text-[11px] font-semibold text-slate-400 hover:text-slate-200"}
            >
              🛡 Hedge{hedgeCount > 0 ? ` (${hedgeCount})` : ""}
            </button>
          <div className="flex items-center gap-1 rounded-lg border border-border-strong p-0.5">
            <button
              aria-label="Kart görünümü"
              onClick={() => setView("cards")}
              className={cn(
                "flex h-9 w-9 items-center justify-center rounded-md transition-colors",
                view === "cards" ? "bg-secondary text-slate-100" : "text-slate-500 hover:text-slate-300",
              )}
            >
              <LayoutGrid className="h-4 w-4" aria-hidden />
            </button>
            <button
              aria-label="Tablo görünümü"
              onClick={() => setView("table")}
              className={cn(
                "flex h-9 w-9 items-center justify-center rounded-md transition-colors",
                view === "table" ? "bg-secondary text-slate-100" : "text-slate-500 hover:text-slate-300",
              )}
            >
              <List className="h-4 w-4" aria-hidden />
            </button>
          </div>
          </div>
        }
      >
        {/* Faz 24: toplu eylem araç çubuğu — yalnız seçim varken görünür */}
        {selected.size > 0 && (
          <div className="mb-3 flex flex-wrap items-center justify-between gap-2 rounded-xl border border-emerald-500/40 bg-emerald-500/10 p-3 shadow-md">
            <div className="flex items-center gap-2">
              <span className="flex h-6 w-6 items-center justify-center rounded-full bg-emerald-500/20 text-xs font-bold text-emerald-300">
                {selected.size}
              </span>
              <span className="text-xs font-bold text-emerald-200">bot seçili</span>
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              <Button size="sm" variant="outline" className="min-h-8 border-emerald-500/50 text-xs text-emerald-300 hover:bg-emerald-500/20" onClick={() => bulkAction("START", "Toplu Başlat")}>
                <Play className="mr-1 h-3.5 w-3.5" aria-hidden /> Başlat
              </Button>
              <Button size="sm" variant="outline" className="min-h-8 border-amber-500/50 text-xs text-amber-300 hover:bg-amber-500/20" onClick={() => bulkAction("PAUSE", "Toplu Duraklat")}>
                <Pause className="mr-1 h-3.5 w-3.5" aria-hidden /> Duraklat
              </Button>
              <Button size="sm" variant="outline" className="min-h-8 border-cyan-500/50 text-xs text-cyan-300 hover:bg-cyan-500/20" onClick={() => bulkAction("FREEZE", "Toplu Dondur")}>
                <Snowflake className="mr-1 h-3.5 w-3.5" aria-hidden /> Dondur
              </Button>
              <Button size="sm" variant="outline" className="min-h-8 border-rose-500/50 text-xs text-rose-300 hover:bg-rose-500/20" onClick={() => bulkAction("CLOSE_ALL", "Toplu Kapat")}>
                <XCircle className="mr-1 h-3.5 w-3.5" aria-hidden /> Kapat
              </Button>
              <Button size="sm" variant="ghost" className="min-h-8 text-xs text-slate-400 hover:text-slate-200" onClick={() => setSelected(new Set())}>
                Seçimi temizle
              </Button>
            </div>
          </div>
        )}
        <div className="mb-3 flex flex-wrap items-center justify-between gap-3 rounded-xl border border-border bg-surface/60 px-3 py-2.5">
          <div className="min-w-44">
            <div className="text-[10px] font-bold uppercase tracking-widest text-slate-300">Çalışma modu</div>
            <p className="mt-0.5 text-[10px] text-slate-500">Filtre botların mevcut modunu değiştirmez.</p>
          </div>
          <div role="group" aria-label="Filo çalışma modu filtresi" className="flex flex-wrap items-center gap-1.5">
            {([
              { mode: "ALL", label: "Tümü", count: modeCounts.ALL },
              { mode: "PAPER", label: "PAPER", count: modeCounts.PAPER },
              { mode: "LIVE", label: "Gerçek · LIVE", count: modeCounts.LIVE },
            ] as const).map((option) => {
              const active = modeF === option.mode;
              const tone = option.mode === "PAPER"
                ? active
                  ? "border-emerald-500/60 bg-emerald-500/15 text-emerald-300"
                  : "border-emerald-500/25 text-emerald-400/80 hover:bg-emerald-500/5"
                : option.mode === "LIVE"
                  ? active
                    ? "border-amber-500/60 bg-amber-500/15 text-amber-300"
                    : "border-amber-500/25 text-amber-400/80 hover:bg-amber-500/5"
                  : active
                    ? "border-slate-400 bg-secondary text-slate-100"
                    : "border-border text-slate-400 hover:text-slate-200";
              return (
                <button
                  key={option.mode}
                  type="button"
                  aria-pressed={active}
                  onClick={() => changeModeFilter(option.mode)}
                  className={cn("rounded-lg border px-3 py-1.5 text-[11px] font-bold transition-colors", tone)}
                >
                  {option.label} <span className="num ml-1 opacity-80">{option.count}</span>
                </button>
              );
            })}
          </div>
        </div>
        {/* Faz 25: Akıllı durum filtre çipleri & Hızlı Toplu Yönetim */}
        <div className="mb-3 flex flex-wrap items-center justify-between gap-2 text-xs">
          <div className="flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              onClick={() => setSmartF("ALL")}
              className={cn(
                "rounded-lg border px-2.5 py-1 text-[11px] font-semibold transition-colors",
                smartF === "ALL" ? "border-slate-400 bg-secondary text-slate-100" : "border-border text-slate-400 hover:text-slate-200"
              )}
            >
              Tümü ({bots.length})
            </button>
            <button
              type="button"
              onClick={() => setSmartF("OPEN_DEAL")}
              className={cn(
                "rounded-lg border px-2.5 py-1 text-[11px] font-semibold transition-colors",
                smartF === "OPEN_DEAL"
                  ? "border-cyan-500/60 bg-cyan-500/15 text-cyan-300"
                  : "border-border text-slate-400 hover:text-slate-200",
              )}
            >
              ⚡ Açık Deal ({openDealsCount})
            </button>
            <button
              type="button"
              onClick={() => setSmartF("BAG_HOLDING")}
              className={cn(
                "rounded-lg border px-2.5 py-1 text-[11px] font-semibold transition-colors",
                smartF === "BAG_HOLDING"
                  ? "border-amber-500/60 bg-amber-500/15 text-amber-300"
                  : bagHoldingCount > 0
                    ? "border-amber-500/30 text-amber-400/90 hover:bg-amber-500/5"
                    : "border-border text-slate-400 hover:text-slate-200",
              )}
            >
              ⚠️ Kademe Tükendi ({bagHoldingCount})
            </button>
            <button
              type="button"
              onClick={() => setSmartF("RISK_MARGIN")}
              className={cn(
                "rounded-lg border px-2.5 py-1 text-[11px] font-semibold transition-colors",
                smartF === "RISK_MARGIN"
                  ? "border-rose-500/60 bg-rose-500/15 text-rose-300"
                  : riskMarginCount > 0
                    ? "border-rose-500/30 text-rose-400/90 hover:bg-rose-500/5"
                    : "border-border text-slate-400 hover:text-slate-200"
              )}
            >
              ⚠️ Likidasyon Riski ({riskMarginCount})
            </button>
            <button
              type="button"
              onClick={() => setSmartF("HALT_WARN")}
              className={cn(
                "rounded-lg border px-2.5 py-1 text-[11px] font-semibold transition-colors",
                smartF === "HALT_WARN"
                  ? "border-amber-500/60 bg-amber-500/15 text-amber-300"
                  : haltWarnCount > 0
                    ? "border-amber-500/30 text-amber-400/90 hover:bg-amber-500/5"
                    : "border-border text-slate-400 hover:text-slate-200"
              )}
            >
              🛑 Halt / Uyarı ({haltWarnCount})
            </button>
            <button
              type="button"
              onClick={() => setSmartF("STOPPED")}
              className={cn(
                "rounded-lg border px-2.5 py-1 text-[11px] font-semibold transition-colors",
                smartF === "STOPPED" ? "border-slate-400 bg-secondary text-slate-100" : "border-border text-slate-400 hover:text-slate-200"
              )}
            >
              Duran ({stoppedCount})
            </button>
          </div>

          {/* 3Commas benchmark: Hızlı Toplu Yönetim Düğmeleri */}
          <div className="flex flex-wrap items-center gap-1.5">
            <button
              type="button"
              onClick={() => {
                if (selected.size === shown.length && shown.length > 0) {
                  setSelected(new Set());
                } else {
                  setSelected(new Set(shown.map((b) => b.id)));
                }
              }}
              className="rounded-lg border border-border bg-surface px-2.5 py-1 text-[11px] font-semibold text-slate-300 transition-colors hover:bg-secondary hover:text-white"
            >
              {selected.size === shown.length && shown.length > 0 ? "Seçimi Kaldır" : `Tümünü Seç (${shown.length})`}
            </button>
            {runningBots.length > 0 && (
              <button
                type="button"
                onClick={handleBulkPauseAll}
                className="inline-flex items-center gap-1 rounded-lg border border-amber-500/40 bg-amber-500/10 px-2.5 py-1 text-[11px] font-semibold text-amber-300 transition-colors hover:bg-amber-500/20"
                title="Çalışan tüm botları duraklat"
              >
                <Pause className="h-3 w-3" /> Tümünü Duraklat ({runningBots.length})
              </button>
            )}
            {pausedBots.length > 0 && (
              <button
                type="button"
                onClick={handleBulkResumeAll}
                className="inline-flex items-center gap-1 rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-2.5 py-1 text-[11px] font-semibold text-emerald-300 transition-colors hover:bg-emerald-500/20"
                title="Duraklatılmış tüm botları başlat"
              >
                <Play className="h-3 w-3" /> Tümünü Başlat ({pausedBots.length})
              </button>
            )}
          </div>
        </div>

        {loading && bots.length === 0 ? (
          <div className="flex items-center justify-center py-10">
            <Spinner className="h-6 w-6" />
          </div>
        ) : bots.length === 0 ? (
          <>
            <EmptyState
              icon={Bot}
              title="Henüz bot yok"
              hint="Simülasyon veya gerçek borsa modu için ayrı bir bot oluşturun."
            />
            <div className="flex flex-wrap justify-center gap-2 pb-3">
              <Button type="button" variant="outline" className="border-emerald-500/40 text-emerald-300 hover:bg-emerald-500/10" onClick={() => onNewBot("PAPER")}>
                PAPER bot oluştur
              </Button>
              <Button type="button" variant="outline" className="border-amber-500/40 text-amber-300 hover:bg-amber-500/10" onClick={() => onNewBot("LIVE")}>
                Gerçek (LIVE) bot kur
              </Button>
            </div>
          </>
        ) : shown.length === 0 ? (
          <>
            <EmptyState
              icon={Bot}
              title={modeF !== "ALL" && modeCounts[modeF] === 0 ? `${modeF === "PAPER" ? "PAPER" : "Gerçek (LIVE)"} bot yok` : "Eşleşen bot yok"}
              hint={modeF !== "ALL" && modeCounts[modeF] === 0
                ? "Bu modda henüz bot yok. Yeni bot oluşturma ekranı seçtiğiniz modla açılır."
                : "Arama, mod, durum filtresi veya Hedge çipini gevşetin."}
            />
            {modeF !== "ALL" && modeCounts[modeF] === 0 && (
              <div className="flex flex-col items-center gap-2 pb-3">
                <Button
                  type="button"
                  variant="outline"
                  className={modeF === "LIVE" ? "border-amber-500/40 text-amber-300 hover:bg-amber-500/10" : "border-emerald-500/40 text-emerald-300 hover:bg-emerald-500/10"}
                  onClick={() => onNewBot(modeF)}
                >
                  {modeF === "PAPER" ? "PAPER bot oluştur" : "Gerçek (LIVE) bot kur"}
                </Button>
                {modeF === "LIVE" && <p className="max-w-md text-center text-[10px] text-amber-400/80">LIVE mod gerçek borsa emirleri kullanır; sihirbazdaki anahtar ve risk kontrolleri uygulanır.</p>}
              </div>
            )}
          </>
        ) : (
          <>
            <GlobalPolicyStrip bots={bots} />
            {view === "cards" ? (
            <div className="mt-3 grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
            {shown.map((b, i) => (
              <motion.div
                key={b.id}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.22, delay: i * 0.04 }}
                className={cn(
                  "relative flex flex-col rounded-xl border bg-surface p-4 transition-colors hover:border-border-strong",
                  selected.has(b.id) ? "border-emerald-500/60 ring-1 ring-emerald-500/30" : "border-border",
                )}
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="flex items-start gap-2.5 min-w-0">
                    <input
                      type="checkbox"
                      aria-label={`${b.name} seç`}
                      checked={selected.has(b.id)}
                      onChange={(e) =>
                        setSelected((s) => {
                          const next = new Set(s);
                          if (e.target.checked) next.add(b.id);
                          else next.delete(b.id);
                          return next;
                        })
                      }
                      className="mt-0.5 h-4 w-4 rounded border-border-strong bg-secondary text-emerald-500 focus:ring-emerald-500/40"
                    />
                    <div className="min-w-0">
                      <div className="truncate text-sm font-semibold text-slate-100">{b.name}</div>
                      <div className="mt-1 flex flex-wrap items-center gap-1.5">
                        <BotIdentityBadges bot={b} />
                        <RiskWarningBadge bot={b} />
                        {b.stats.phase === "Open" &&
                          b.config?.max_safety_orders != null &&
                          b.stats.safety_triggered >= b.config.max_safety_orders && (
                            <span
                              className="inline-flex items-center gap-1 rounded bg-amber-500/20 px-1.5 py-0.5 text-[10px] font-bold text-amber-300 ring-1 ring-amber-500/40"
                              title="Tüm güvenlik emirleri doldu (Kademe tükendi). Fiyat dönene veya Add Funds eklenene kadar beklemede."
                            >
                              ⚠️ KADEME TÜKENDİ
                            </span>
                          )}
                      </div>
                    </div>
                  </div>
                  {/* Faz 25 (UIX-26): KERNEL HALT birincil durumdur — RUNNING
                      (yeşil) ile birlikte göstermek çelişkiydi. HALT varken
                      durum rozeti HALT olur, RUNNING alt notta kalır. */}
                  {b.kernelHalted ? (
                    <div className="flex shrink-0 flex-col items-end gap-0.5">
                      <span
                        className="rounded border border-rose-500/60 bg-rose-500/10 px-2 py-0.5 text-[10px] font-bold tracking-wider text-rose-400"
                        title={b.kernelHaltedReason || "Kernel kalıcı halt — RESET_KERNEL ile açılır"}
                      >
                        KERNEL HALT
                      </span>
                      <span className="text-[9px] font-semibold tracking-wide text-slate-500" title="Yürütücü durdu; sıfırlanana kadar emir üretilmez">
                        yeniden başlatma bekliyor
                      </span>
                    </div>
                  ) : (
                    <StatusPill status={b.status} />
                  )}
                </div>

                <EntryTriggerNote bot={b} />

                <div className="num mt-3 grid grid-cols-2 gap-x-3 gap-y-2 text-xs">
                  <div>
                    <span className="text-slate-500">POZİSYON</span>
                    <div className="text-slate-200">{fmtFrac(b.stats.position_qty)}</div>
                  </div>
                  <div>
                    <span className="text-slate-500">ORTALAMA</span>
                    <div className="text-slate-200">{fmtFrac(b.stats.avg_price)}</div>
                  </div>
                  <div>
                    <span className="text-slate-500">SONRAKİ GÜVENLİK</span>
                    <div className="text-cyan-400">
                      L{b.stats.next_safety_level ?? "—"} @ {fmtFrac(b.stats.next_safety_price)}
                    </div>
                  </div>
                  <div>
                    <span className="text-slate-500">TP FİYATI</span>
                    <div className="text-emerald-400">{fmtFrac(b.stats.tp_price)}</div>
                  </div>
                  <div>
                    <span className="text-slate-500">DEAL W/L</span>
                    <div className="text-slate-300">
                      {b.stats.wins}/{b.stats.losses}
                    </div>
                  </div>
                  <div>
                    <span className="text-slate-500">KÂR/ZARAR</span>
                    <div
                      className={cn(
                        "font-semibold",
                        (fracToNum(b.stats.realized_gross) ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400",
                      )}
                    >
                      {fmtUsdSigned(b.stats.realized_gross)}
                    </div>
                  </div>
                </div>

                {/* 3Commas tarzı görsel SO Stepper & TP mesafe ilerleme çubuğu */}
                <div className="mt-2.5">
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
                    isShort={b.stats.position_direction === -1 || (b.stats.position_direction == null && b.kind === "DCA_SHORT")}
                    symbol={b.symbol}
                    compact={true}
                  />
                </div>

                {/* Faz 25 (UIX-43) / Faz 27: kaldıraçlı botta marj erimesi
                    vurgusu — izole marjda M07 likidasyonu ~%100 IM erimesi;
                    %50 eşiği aşan kartta kırmızı uyarı (arda1 vakası dersi). */}
                <MarginErodeBadge bot={b} />

                <div className="mt-3 flex flex-wrap gap-1.5 border-t border-border pt-3">
                  {/* Faz 27 (O-2): KERNEL HALT kurtarması yalnız Studio'da
                      değildi — filo kartında tek tık; sticky-halt bekleyen bot
                      için operasyon yükünü düşürür. */}
                  {b.kernelHalted && (
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-h-9 flex-1 border-rose-500/60 text-rose-300 hover:bg-rose-500/10"
                      disabled={busyId === b.id}
                      title={b.kernelHaltedReason || "Kernel kalıcı halt — sıfırlanınca bot sonraki kapalı bardan warmup ile devam eder"}
                      onClick={() =>
                        void withBusy(b.id, async () => {
                          await apiBotAction(b.id, "RESET_KERNEL");
                          toast({ title: `${b.name} kernel sıfırlandı`, description: "Bir sonraki kapalı bardan warmup ile devam eder." });
                        })
                      }
                    >
                      <Play className="h-3.5 w-3.5" aria-hidden />
                      KERNEL&apos;İ SIFIRLA
                    </Button>
                  )}
                  {/* Faz 24: toplu seçim checkbox'ı */}
                  <label className="flex min-h-9 items-center gap-1.5 rounded-md border border-border px-2.5" aria-label={`${b.name} toplu seçim`}>
                    <input
                      type="checkbox"
                      checked={selected.has(b.id)}
                      onChange={(e) =>
                        setSelected((s) => {
                          const next = new Set(s);
                          if (e.target.checked) next.add(b.id);
                          else next.delete(b.id);
                          return next;
                        })
                      }
                      className="h-3.5 w-3.5 accent-emerald-500"
                    />
                  </label>
                  <Button size="sm" className="min-h-9 flex-1 bg-emerald-500/90 text-[#04150e] hover:bg-emerald-500" onClick={() => onInspect(b.id)}>
                    <Eye className="h-3.5 w-3.5" aria-hidden />
                    İncele
                  </Button>
                  {b.status === "RUNNING" ? (
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-h-9 border-amber-500/40 text-amber-400 hover:bg-amber-500/10 hover:text-amber-300"
                      disabled={busyId === b.id}
                      onClick={() => void togglePause(b)}
                    >
                      <Pause className="h-3.5 w-3.5" aria-hidden />
                      Duraklat
                    </Button>
                  ) : b.status === "PAUSED" ? (
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-h-9 border-emerald-500/40 text-emerald-400 hover:bg-emerald-500/10"
                      disabled={busyId === b.id}
                      onClick={() => void togglePause(b)}
                    >
                      <Play className="h-3.5 w-3.5" aria-hidden />
                      Devam
                    </Button>
                  ) : b.status === "FROZEN" ? (
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-h-9 border-sky-500/40 text-sky-300 hover:bg-sky-500/10"
                      disabled={busyId === b.id}
                      onClick={() => void toggleFreeze(b)}
                    >
                      <Snowflake className="h-3.5 w-3.5" aria-hidden />
                      Çöz
                    </Button>
                  ) : b.status === "STOPPED" ? (
                    // Faz 23 (UI derin inceleme #8): STOPPED bot ölü uçtu —
                    // UI'da START yolu hiç yoktu (yalnız apiStopAllBots bunları
                    // üretiyordu). Devam = patch status RUNNING.
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-h-9 border-emerald-500/40 text-emerald-400 hover:bg-emerald-500/10"
                      disabled={busyId === b.id}
                      onClick={() =>
                        void apiPatchBot(b.id, { status: "RUNNING" })
                          .then(() => toast({ title: `${b.name} devam ettirildi`, description: "RUNNING" }))
                          .catch((e) => toast({ title: "Devam ettirilemedi", description: String(e) }))
                      }
                    >
                      <Play className="h-3.5 w-3.5" aria-hidden />
                      Devam
                    </Button>
                  ) : null}
                  {b.status === "RUNNING" ? (
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-h-9 border-sky-500/40 text-sky-300 hover:bg-sky-500/10 hover:text-sky-200"
                      disabled={busyId === b.id}
                      onClick={() => void toggleFreeze(b)}
                    >
                      <Snowflake className="h-3.5 w-3.5" aria-hidden />
                      Dondur
                    </Button>
                  ) : null}
                  {/* Faz 24 (UI/UX Pro Max raporu §3.2.3): PANIC — tek tıkla
                      market çıkışı + bot durdurma (onaylı). */}
                  {b.status === "RUNNING" && (
                    <Button
                      size="sm"
                      variant="outline"
                      className="min-h-9 border-rose-500/60 bg-rose-500/10 font-bold text-rose-300 hover:bg-rose-500/20"
                      disabled={busyId === b.id}
                      title="Pozisyonu marketten kapat ve botu durdur"
                      onClick={() => void panicClose(b)}
                    >
                      PANIC
                    </Button>
                  )}
                  <CloseAllDialog
                    bot={b}
                    onClose={executeCloseAll}
                    disabled={busyId === b.id || closeAllDialogId !== null && closeAllDialogId !== b.id}
                    triggerClassName="min-h-9 border-border-strong text-slate-300 hover:bg-secondary"
                  />
                  <DeleteBotDialog
                    bot={b}
                    onRemove={remove}
                    disabled={busyId === b.id}
                    withLabel
                    triggerClassName="min-h-9 border-rose-500/30 text-rose-400 hover:bg-rose-500/10"
                  />
                </div>
              </motion.div>
            ))}
          </div>
            ) : (
              <div className="thin-scroll max-h-[32rem] overflow-y-auto rounded-lg border border-border">
            <Table>
              <TableHeader className="sticky top-0 z-10 bg-surface">
                <TableRow className="hover:bg-transparent border-border">
                  <TableHead className="h-9 text-[11px] text-slate-400" aria-sort={sortKey === "name" ? (sortDir === 1 ? "ascending" : "descending") : "none"}>{sortTh("name", "AD")}</TableHead>
                  <TableHead className="h-9 text-[11px] text-slate-400">STRATEJİ</TableHead>
                  <TableHead className="h-9 text-[11px] text-slate-400" aria-sort={sortKey === "status" ? (sortDir === 1 ? "ascending" : "descending") : "none"}>{sortTh("status", "DURUM")}</TableHead>
                  <TableHead className="h-9 text-[11px] text-slate-400">RİSK</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">POZİSYON</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">ORT.</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">TP</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">W/L</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400" aria-sort={sortKey === "pnl" ? (sortDir === 1 ? "ascending" : "descending") : "none"}>{sortTh("pnl", "KÂR/ZARAR")}</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">İŞLEM</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {sorted.map((b) => (
                  <TableRow key={b.id} className="border-border hover:bg-surface-2">
                    <TableCell className="py-2">
                      <button className="text-left text-xs font-semibold text-slate-100 hover:text-emerald-400" onClick={() => onInspect(b.id)}>
                        {b.name}
                      </button>
                      <div className="num text-[10px] text-slate-400">{b.symbol}</div>
                    </TableCell>
                    <TableCell className="py-2">
                      <BotIdentityBadges bot={b} />
                    </TableCell>
                    <TableCell className="py-2">
                      {/* UIX-26: HALT birincil — RUNNING'e yeşil rozetle eşitlenmez */}
                      {b.kernelHalted ? (
                        <span
                          className="inline-flex rounded border border-rose-500/60 bg-rose-500/10 px-2 py-0.5 text-[10px] font-bold tracking-wider text-rose-400"
                          title={b.kernelHaltedReason || "Kernel kalıcı halt — RESET_KERNEL ile açılır"}
                        >
                          KERNEL HALT
                        </span>
                      ) : (
                        <StatusPill status={b.status} />
                      )}
                      <EntryTriggerNote bot={b} compact />
                    </TableCell>
                    <TableCell className="py-2">
                      {(b.warnings?.length ?? 0) > 0 ? (
                        <RiskWarningBadge bot={b} />
                      ) : (
                        <span className="text-[10px] text-slate-400">—</span>
                      )}
                    </TableCell>
                    <TableCell className="num py-2 text-right text-slate-200">{fmtFrac(b.stats.position_qty)}</TableCell>
                    <TableCell className="num py-2 text-right text-slate-300">{fmtFrac(b.stats.avg_price)}</TableCell>
                    <TableCell className="num py-2 text-right text-emerald-400">{fmtFrac(b.stats.tp_price)}</TableCell>
                    <TableCell className="num py-2 text-right text-slate-300">
                      {b.stats.wins}/{b.stats.losses}
                    </TableCell>
                    <TableCell
                      className={cn(
                        "num py-2 text-right font-semibold",
                        (fracToNum(b.stats.realized_gross) ?? 0) >= 0 ? "text-emerald-400" : "text-rose-400",
                      )}
                    >
                      {fmtUsdSigned(b.stats.realized_gross)}
                    </TableCell>
                    <TableCell className="py-2">
                      <div className="flex justify-end gap-1">
                        <Button size="sm" variant="outline" className="min-h-8 border-border-strong px-2 text-slate-300 hover:bg-secondary" onClick={() => onInspect(b.id)}>
                          <Eye className="h-3.5 w-3.5" aria-hidden />
                        </Button>
                        <CloseAllDialog
                          bot={b}
                          onClose={executeCloseAll}
                          disabled={busyId === b.id || closeAllDialogId !== null && closeAllDialogId !== b.id}
                          triggerClassName="min-h-8 border-border-strong px-2 text-slate-300 hover:bg-secondary"
                        />
                        <DeleteBotDialog
                          bot={b}
                          onRemove={remove}
                          disabled={busyId === b.id}
                          triggerClassName="min-h-8 border-rose-500/30 px-2 text-rose-400 hover:bg-rose-500/10"
                        />
                      </div>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
              </div>
            )}
          </>
        )}
      </Panel>

      <FleetTape />

      <AlertDialog open={confirmation !== null} onOpenChange={(open) => { if (!open) setConfirmation(null); }}>
        <AlertDialogContent className="border-border-strong bg-[#151a24]">
          <AlertDialogHeader>
            <AlertDialogTitle className="text-slate-100">{confirmation?.title}</AlertDialogTitle>
            <AlertDialogDescription className="text-slate-400">{confirmation?.description}</AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel className="border-border-strong bg-transparent text-slate-300 hover:bg-secondary hover:text-slate-100">
              Vazgeç
            </AlertDialogCancel>
            <AlertDialogAction
              className={confirmation?.tone === "danger" ? "bg-rose-500 text-white hover:bg-rose-600" : "bg-emerald-600 text-white hover:bg-emerald-500"}
              onClick={() => {
                const action = confirmation?.onConfirm;
                setConfirmation(null);
                if (action) void action();
              }}
            >
              {confirmation?.confirmLabel ?? "Onayla"}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <p className="flex items-center gap-1.5 text-[11px] text-slate-400">
        <ScrollText className="h-3.5 w-3.5" aria-hidden />
        Duraklatılmış botlar pozisyonlarını korur; yeni emir üretilmez. Silinen botların geçmişi geri alınamaz.
        Risk bütçesi için Backtest Lab&apos;deki VaR paneline bakın.
      </p>
    </div>
  );
}
