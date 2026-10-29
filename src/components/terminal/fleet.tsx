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
import { fmtFrac, fmtTimeShort, fmtUsdSigned, fracToNum, kindLabel } from "@/lib/format";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";
import type { Bot as BotRow, UserRiskPolicy } from "@/lib/engine-types";

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

function riskChipsOf(b: BotRow): { label: string; warn: boolean }[] {
  const chips: { label: string; warn: boolean }[] = [];
  const rp: UserRiskPolicy | undefined = b.riskPolicy;
  if (rp) {
    const permissive = GUARD_KEYS.filter((k) => rp[k] === "PERMISSIVE_WITH_WARNING").length;
    chips.push(
      permissive > 0
        ? { label: `${permissive} KORUMA UYARI`, warn: true }
        : { label: "KORUMA ZORUNLU", warn: false },
    );
    if (rp.daily_dd_breaker_pct === "0") chips.push({ label: "DD KESİCİ KAPALI", warn: true });
    else if (rp.fill_model === "OPTIMISTIC") chips.push({ label: "İYİMSER DOLUM", warn: true });
  }
  return chips.slice(0, 2);
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

function RiskPolicyChip({ label, warn }: { label: string; warn: boolean }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded border px-1.5 py-0.5 text-[10px] font-semibold",
        warn
          ? "border-amber-500/50 bg-amber-500/10 text-amber-300"
          : "border-border-strong bg-surface text-slate-400",
      )}
    >
      {label}
    </span>
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
  if (compact) {
    return (
      <div className={cn("num mt-1 text-[10px] font-semibold", bot.stats.entry_trigger_met ? "text-emerald-400" : "text-amber-400")}>
        {t}
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
    >
      <Clock className="h-3 w-3 shrink-0" aria-hidden />
      <span className="num truncate">BAŞLANGIÇ: {t}</span>
      {!bot.stats.entry_trigger_met && <span className="ml-auto shrink-0">KOŞUL BEKLİYOR</span>}
    </div>
  );
}

function DeleteBotDialog({ bot, onRemove, triggerClassName, disabled }: { bot: BotRow; onRemove: (b: BotRow) => void; triggerClassName: string; disabled?: boolean }) {
  return (
    <AlertDialog>
      <AlertDialogTrigger asChild>
        <Button size="sm" variant="outline" className={triggerClassName} disabled={disabled}>
          <Trash2 className="h-3.5 w-3.5" aria-hidden />
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
            onClick={(e) => {
              e.preventDefault();
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

export function Fleet({ onInspect, onNewBot }: { onInspect: (id: string) => void; onNewBot: () => void }) {
  const { bots, loading, refetch } = useBots();
  const [view, setView] = useState<"cards" | "table">("cards");
  const [busyId, setBusyId] = useState<string | null>(null);
  const [hedgeOnly, setHedgeOnly] = useState(false);
  const hedgeCount = bots.filter((b) => HEDGE_KINDS.has(b.kind)).length;
  const shown = hedgeOnly ? bots.filter((b) => HEDGE_KINDS.has(b.kind)) : bots;
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

  /* P2-2 Freeze: RUNNING → FROZEN (yeni emir yok, açıklar korunur), geri dönüş RUNNING. */
  const toggleFreeze = (b: BotRow) =>
    withBusy(b.id, async () => {
      const action = b.status === "FROZEN" ? "UNFREEZE" : "FREEZE";
      await apiBotAction(b.id, action);
      toast({ title: `${b.name}`, description: action === "FREEZE" ? "Donduruldu — yeni emir üretilmez, açıklar korunur." : "Çözüldü → RUNNING" });
    });

  const closeAll = (b: BotRow) =>
    withBusy(b.id, async () => {
      await apiBotAction(b.id, "CLOSE_ALL");
      toast({ title: `${b.name}`, description: "CLOSE_ALL emri gönderildi." });
    });

  const remove = (b: BotRow) =>
    withBusy(b.id, async () => {
      await apiDeleteBot(b.id);
      toast({ title: `${b.name}`, description: "Bot silindi." });
    });

  return (
    <div className="space-y-4">
      <Panel
        title={`Bot Filosu (${shown.length}${hedgeOnly ? ` hedge · toplam ${bots.length}` : ""})`}
        icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />}
        right={
          <div className="flex items-center gap-1.5">
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
        {loading && bots.length === 0 ? (
          <div className="flex items-center justify-center py-10">
            <Spinner className="h-6 w-6" />
          </div>
        ) : bots.length === 0 ? (
          <EmptyState
            icon={Bot}
            title="Filo boş"
            hint="Yeni Bot sihirbazından ilk stratejinizi kurun — DCA, Grid, Infinity veya Hedge."
          />
        ) : hedgeOnly && shown.length === 0 ? (
          <EmptyState
            icon={Bot}
            title="Hedge botu yok"
            hint="Hedge filtresini kapatın ya da sihirbazdan HEDGE_PROTECTIVE / HEDGE_DUAL botu kurun."
          />
        ) : view === "cards" ? (
          <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
            {shown.map((b, i) => (
              <motion.div
                key={b.id}
                initial={{ opacity: 0, y: 10 }}
                animate={{ opacity: 1, y: 0 }}
                transition={{ duration: 0.22, delay: i * 0.04 }}
                className="flex flex-col rounded-xl border border-border bg-surface p-4 transition-colors hover:border-border-strong"
              >
                <div className="flex items-start justify-between gap-2">
                  <div className="min-w-0">
                    <div className="truncate text-sm font-semibold text-slate-100">{b.name}</div>
                    <div className="mt-1 flex flex-wrap items-center gap-1.5">
                      <BotIdentityBadges bot={b} />
                      <RiskWarningBadge bot={b} />
                      {riskChipsOf(b).map((c) => (
                        <RiskPolicyChip key={c.label} label={c.label} warn={c.warn} />
                      ))}
                    </div>
                  </div>
                  <StatusPill status={b.status} />
                  {/* Faz 21 (N-8): kernel halt filoda GÖRÜNÜR — sessiz duran
                      bot artık kırmızı rozetle işaretlenir. */}
                  {b.kernelHalted && (
                    <span
                      className="rounded border border-rose-500/60 bg-rose-500/10 px-2 py-0.5 text-[10px] font-bold tracking-wider text-rose-400"
                      title={b.kernelHaltedReason || "Kernel kalıcı halt — RESET_KERNEL ile açılır"}
                    >
                      KERNEL HALT
                    </span>
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
                    <span className="text-slate-500">KÂR/ZARAR · PROFIT</span>
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

                <div className="mt-3 flex flex-wrap gap-1.5 border-t border-border pt-3">
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
                  <Button
                    size="sm"
                    variant="outline"
                    className="min-h-9 border-border-strong text-slate-300 hover:bg-secondary"
                    disabled={busyId === b.id}
                    onClick={() => void closeAll(b)}
                  >
                    <XCircle className="h-3.5 w-3.5" aria-hidden />
                    Kapat
                  </Button>
                  <DeleteBotDialog
                    bot={b}
                    onRemove={remove}
                    disabled={busyId === b.id}
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
                      <StatusPill status={b.status} />
                      <EntryTriggerNote bot={b} compact />
                    </TableCell>
                    <TableCell className="py-2">
                      {(b.warnings?.length ?? 0) > 0 ? (
                        <RiskWarningBadge bot={b} />
                      ) : riskChipsOf(b).length > 0 ? (
                        <div className="flex flex-wrap gap-1">
                          {riskChipsOf(b).map((c) => (
                            <RiskPolicyChip key={c.label} label={c.label} warn={c.warn} />
                          ))}
                        </div>
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
                        <Button
                          size="sm"
                          variant="outline"
                          className="min-h-8 border-border-strong px-2 text-slate-300 hover:bg-secondary"
                          disabled={busyId === b.id}
                          onClick={() => void closeAll(b)}
                        >
                          <XCircle className="h-3.5 w-3.5" aria-hidden />
                        </Button>
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
      </Panel>

      <FleetTape />

      <p className="flex items-center gap-1.5 text-[11px] text-slate-400">
        <ScrollText className="h-3.5 w-3.5" aria-hidden />
        Duraklatılmış botlar pozisyonlarını korur; yeni emir üretilmez. Silinen botların geçmişi geri alınamaz.
        Risk bütçesi için Backtest Lab&apos;deki VaR paneline bakın.
      </p>
    </div>
  );
}
