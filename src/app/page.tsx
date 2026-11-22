"use client";

import { use, useEffect, useState } from "react";
import { AnimatePresence, motion, useReducedMotion } from "framer-motion";
import {
  BookOpen,
  Bot,
  Calculator,
  FlaskConical,
  LayoutDashboard,
  MoreHorizontal,
  RefreshCw,
  Settings,
  SquarePlus,
  Terminal,
  WifiOff,
  Wrench,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Drawer, DrawerContent, DrawerHeader, DrawerTitle } from "@/components/ui/drawer";
import { KillButton } from "@/components/terminal/killswitch";
import { AuthDialog } from "@/components/terminal/auth-dialog";
import {
  BacktestLazy,
  CalculatorLazy,
  DashboardView,
  FleetView,
  KnowledgeLazy,
  SettingsLazy,
  StudioView,
  WizardView,
} from "@/components/terminal/views";
import { useEngineHealth, useMaintenance } from "@/hooks/use-engine";
import { fmtDur } from "@/lib/format";
import { cn } from "@/lib/utils";

type View = "dashboard" | "fleet" | "studio" | "wizard" | "calc" | "backtest" | "knowledge" | "settings";
type PageSearchParams = Promise<{ [key: string]: string | string[] | undefined }>;

const isView = (value: unknown): value is View =>
  typeof value === "string" && ["dashboard", "fleet", "studio", "wizard", "calc", "backtest", "knowledge", "settings"].includes(value);

const NAV: { id: View; label: string; icon: React.ElementType }[] = [
  { id: "dashboard", label: "Kokpit", icon: LayoutDashboard },
  { id: "fleet", label: "Bot Filosu", icon: Bot },
  { id: "wizard", label: "Yeni Bot", icon: SquarePlus },
  { id: "calc", label: "Hesaplayıcı", icon: Calculator },
  { id: "backtest", label: "Backtest Lab", icon: FlaskConical },
  { id: "knowledge", label: "Bilgi", icon: BookOpen },
  { id: "settings", label: "Ayarlar", icon: Settings },
];

/* P1 — mobil alt bar ≤5 kuralı: 4 birincil + "Diğer" çekmecesi. Desktop gruplu. */
const PRIMARY: View[] = ["dashboard", "fleet", "wizard", "backtest"];
const SECONDARY: View[] = ["calc", "knowledge", "settings"];
const SHORT_LABEL: Record<View, string> = {
  dashboard: "Kokpit",
  fleet: "Filo",
  studio: "Stüdyo",
  wizard: "Yeni Bot",
  calc: "Hesaplayıcı",
  backtest: "Lab",
  knowledge: "Bilgi",
  settings: "Ayarlar",
};
const NAV_GROUPS: { title: string; ids: View[] }[] = [
  { title: "İzleme", ids: ["dashboard", "fleet"] },
  { title: "Kurulum", ids: ["wizard", "calc", "backtest"] },
  { title: "Sistem", ids: ["knowledge", "settings"] },
];

/**
 * Faz 20 (UIX-6): veri tazeliği rozeti — poll "sessiz atlama" yaptığında
 * ekranın ne kadar bayat kaldığı artık görünür. Taze (<15s) yeşil, kuşkulu
 * (<45s) amber, bayat (≥45s) kırmızı; motor çevrimdışıysa nötr.
 */
function DataFreshness({ lastUpdatedAt, offline }: { lastUpdatedAt: number | null; offline: boolean }) {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  if (offline) {
    return <span className="shrink-0 rounded border border-border px-2 py-0.5 font-bold tracking-wider text-slate-500">VERİ — (motor çevrimdışı)</span>;
  }
  if (lastUpdatedAt == null) {
    return <span className="shrink-0 rounded border border-border px-2 py-0.5 font-bold tracking-wider text-slate-500">VERİ — bekleniyor…</span>;
  }
  const age = Math.max(0, Math.floor((now - lastUpdatedAt) / 1000));
  const tone = age < 15 ? "border-emerald-500/30 bg-emerald-500/5 text-emerald-400" : age < 45 ? "border-amber-500/30 bg-amber-500/5 text-amber-400" : "border-rose-500/30 bg-rose-500/5 text-rose-400";
  return (
    <span className={cn("num shrink-0 rounded border px-2 py-0.5 font-bold tracking-wider", tone)} title="Son başarılı veri güncellemesi">
      VERİ {age}s ÖNCE
    </span>
  );
}

export default function Page({ searchParams }: { searchParams: PageSearchParams }) {
  const query = use(searchParams);
  const requestedView = Array.isArray(query.view) ? query.view[0] : query.view;
  const initialView: View = isView(requestedView) ? requestedView : "dashboard";
  const initialBotId = typeof query.botId === "string" ? query.botId : null;
  const [view, setView] = useState<View>(initialView);
  const [botId, setBotId] = useState<string | null>(initialBotId);
  const [authLocked, setAuthLocked] = useState(false);
  /* P1 + P6 */
  const [moreOpen, setMoreOpen] = useState(false);
  const reduceMotion = useReducedMotion();
  const { health, error, lastUpdatedAt, refetch } = useEngineHealth(10000);
  const { status: maint } = useMaintenance(60000);

  const online = health?.status === "ok";

  // Faz 0.3 (B-3): Oturum / Kimlik doğrulama kontrolü
  useEffect(() => {
    fetch("/api/auth/check")
      .then((r) => r.json())
      .then((d) => {
        if (d.authRequired && !d.authenticated) {
          setAuthLocked(true);
        }
      })
      .catch(() => {});
  }, []);

  // Faz 1.1 (P1): URL Senkronizasyonu & Tarayıcı Geri/İleri (popstate) Desteği
  useEffect(() => {
    if (typeof window === "undefined") return;

    const onPopState = () => {
      const p = new URLSearchParams(window.location.search);
      const pv = (p.get("view") as View) || "dashboard";
      const pb = p.get("botId");
      setView(pv);
      setBotId(pv === "studio" ? pb : null);
    };
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  const syncUrl = (v: View, id: string | null) => {
    if (typeof window === "undefined") return;
    const params = new URLSearchParams();
    if (v !== "dashboard") params.set("view", v);
    if (v === "studio" && id) params.set("botId", id);
    const qs = params.toString();
    const newUrl = qs ? `?${qs}` : window.location.pathname;
    if (window.location.search !== (qs ? `?${qs}` : "")) {
      window.history.pushState(null, "", newUrl);
    }
  };

  const openStudio = (id: string) => {
    setBotId(id);
    setView("studio");
    syncUrl("studio", id);
  };
  const go = (v: View) => {
    if (v !== "studio") setBotId(null);
    setMoreOpen(false);
    setView(v);
    syncUrl(v, null);
  };

  const navActive = (id: View): boolean => (id === "fleet" && view === "studio") || view === id;

  return (
    <div className="flex min-h-screen w-full flex-col bg-base">
      {/* ---------------- Header ---------------- */}
      <header className="sticky top-0 z-40 border-b border-border bg-surface/95 backdrop-blur">
        <div className="flex items-center gap-3 px-4 py-3 md:px-6">
          <button className="flex items-center gap-2.5 text-left" onClick={() => go("dashboard")} aria-label="DCABOT ana sayfa">
            <span className="flex h-9 w-9 items-center justify-center rounded-lg border border-emerald-500/40 bg-emerald-500/10">
              <Terminal className="h-4.5 w-4.5 text-emerald-400" aria-hidden />
            </span>
            <span className="leading-tight">
              <span className="block text-sm font-extrabold tracking-wide text-slate-50">DCABOT</span>
              <span className="hidden text-[10px] font-medium uppercase tracking-widest text-slate-400 sm:block">
                Quantitative Terminal {health?.version ? `v${health.version}` : ""}
              </span>
            </span>
          </button>

          <div className="ml-auto flex items-center gap-2">
            <span
              className={cn(
                "hidden items-center gap-1.5 rounded-full border px-2.5 py-1 text-[10px] font-bold tracking-wider sm:inline-flex",
                online ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400" : "border-rose-500/50 bg-rose-500/10 text-rose-400",
              )}
              aria-live="polite"
            >
              <span className={cn("relative inline-flex h-2 w-2 rounded-full", online ? "bg-emerald-400 text-emerald-400" : "bg-rose-400 text-rose-400")}>
                {online && <span className="dot-ping" />}
              </span>
              {online ? `CORE v${health?.version ?? "—"}` : "CORE OFFLINE"}
            </span>
            <KillButton onStopAll={refetch} />
          </div>
        </div>
      </header>

      {/* ---------------- Offline banner ---------------- */}
      {error && (
        <div className="flex flex-wrap items-center gap-3 border-b border-rose-500/30 bg-rose-500/10 px-4 py-2.5 md:px-6">
          <WifiOff className="h-4 w-4 text-rose-400" aria-hidden />
          <div className="min-w-0 flex-1 text-xs text-rose-300">
            <span className="font-bold">ENGINE OFFLINE</span> — dca-core (port 3002) yanıt vermiyor: {error}
          </div>
          <Button size="sm" variant="outline" className="min-h-9 border-rose-500/40 text-rose-300 hover:bg-rose-500/10" onClick={refetch}>
            <RefreshCw className="h-3.5 w-3.5" aria-hidden />
            Yeniden Dene
          </Button>
        </div>
      )}

      {/* ---------------- Bakım bandı (P3-5 sentinel) ---------------- */}
      {online && maint && maint.mode !== "NORMAL" && (
        <div
          className={maint.mode === "QUIET" ? "flex flex-wrap items-center gap-3 border-b border-amber-500/30 bg-amber-500/10 px-4 py-2.5 md:px-6" : "flex flex-wrap items-center gap-3 border-b border-orange-500/30 bg-orange-500/10 px-4 py-2.5 md:px-6"}
          role="status"
        >
          <Wrench className={maint.mode === "QUIET" ? "h-4 w-4 text-amber-400" : "h-4 w-4 text-orange-400"} aria-hidden />
          <div className="min-w-0 flex-1 text-xs">
            <span className={maint.mode === "QUIET" ? "font-bold text-amber-300" : "font-bold text-orange-300"}>
              {maint.mode === "QUIET" ? "BAKIM SESSİZLİĞİ" : "BORSA ERİŞİMİ AKSIYOR"}
            </span>{" "}
            <span className="text-slate-400">
              {maint.mode === "QUIET"
                ? maint.note || "Operatör sessizlik bayrağı açık — yeni canlı emirler için temkinli olun."
                : maint.venues.filter((v) => !v.ok).map((v) => `${v.venue} (${v.error ?? "erişilemiyor"})`).join(" · ")}
            </span>
          </div>
        </div>
      )}

      {/* ---------------- Body ---------------- */}
      <div className="flex w-full flex-1">
        {/* Sidebar (desktop) */}
        <aside className="sticky top-[61px] hidden h-[calc(100vh-61px)] w-56 shrink-0 flex-col border-r border-border bg-surface p-3 md:flex">
          <nav className="space-y-4" aria-label="Ana gezinme">
            {NAV_GROUPS.map((g) => (
              <div key={g.title}>
                <div className="mb-1 px-3 text-[10px] font-bold uppercase tracking-widest text-slate-400">{g.title}</div>
                <div className="space-y-1">
                  {g.ids.map((id) => {
                    const n = NAV.find((x) => x.id === id)!;
                    const Icon = n.icon;
                    return (
                      <button
                        key={n.id}
                        onClick={() => go(n.id)}
                        aria-current={navActive(n.id) ? "page" : undefined}
                        className={cn(
                          "flex min-h-11 w-full items-center gap-3 rounded-lg px-3 text-sm font-medium transition-colors",
                          navActive(n.id)
                            ? "bg-emerald-500/10 text-emerald-300"
                            : "text-slate-400 hover:bg-surface-2 hover:text-slate-200",
                        )}
                      >
                        <Icon className="h-4.5 w-4.5" aria-hidden />
                        {n.label}
                      </button>
                    );
                  })}
                </div>
              </div>
            ))}
          </nav>
          {/* Faz 25 (UIX-28): burada modül kodlarıydı ("FAIL-CLOSED GATES /
              RATIONAL CORE / M01–M20") — tüm UI'da 3 kez tekrarlanıyordu.
              Artık: operasyonel filo özeti + koruma tek yerde, insan diliyle. */}
          <div className="mt-auto rounded-lg border border-border bg-surface p-3 text-[10px] leading-relaxed text-slate-400">
            <div className="num text-emerald-500/80">
              BOT {online && health?.readiness ? `${health.readiness.activeBots}/${health.readiness.totalBots} AKTİF` : "—"}
            </div>
            <div className="num">{online && health ? "tick hattı canlı" : "motor çevrimdışı"}</div>
            <div className="num mt-1 text-slate-500">Koruma: {health?.gates ?? "—"} zorunlu kapı</div>
          </div>
        </aside>

        {/* Main */}
        <main className="min-w-0 flex-1 p-4 pb-24 md:p-6 md:pb-8">
          <AnimatePresence mode="wait">
            <motion.div
              key={`${view}-${botId ?? "none"}`}
              initial={{ opacity: 0, y: 10 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: -8 }}
              transition={{ duration: reduceMotion ? 0 : 0.18 }}
            >
              {view === "dashboard" && <DashboardView onInspect={openStudio} onNewBot={() => go("wizard")} />}
              {view === "fleet" && <FleetView onInspect={openStudio} onNewBot={() => go("wizard")} />}
              {view === "studio" && botId && <StudioView botId={botId} onBack={() => go("fleet")} />}
              {view === "wizard" && <WizardView onCreated={openStudio} />}
              {view === "calc" && <CalculatorLazy />}
              {view === "backtest" && <BacktestLazy onGoWizard={() => go("wizard")} />}
              {view === "knowledge" && <KnowledgeLazy onOpenAi={() => go("backtest")} />}
              {view === "settings" && <SettingsLazy />}
            </motion.div>
          </AnimatePresence>
        </main>
      </div>

      {/* ---------------- Footer (sticky bottom status bar) ---------------- */}
      <footer className="mt-auto mb-16 border-t border-border bg-surface md:mb-0" aria-label="Durum çubuğu">
        <div className="thin-scroll flex items-center gap-x-4 gap-y-1 overflow-x-auto px-4 py-2 text-[10px] md:px-6">
          {/* Faz 20 (UIX-6): veri tazeliği — poll sessiz atlayınca ekranın
              ne kadar bayat olduğu artık görünür. */}
          <DataFreshness lastUpdatedAt={lastUpdatedAt} offline={!online} />
          <span className="num shrink-0 text-slate-500">
            UPTIME <span className="text-slate-300">{online ? fmtDur(health?.uptime_s) : "—"}</span>
          </span>
          {/* Faz 25 (UIX-28): iki mühendislik rozeti tüm UI'da tekrar ediyordu
              → tek satırda birleşti, insan diliyle. */}
          <span className="shrink-0 rounded border border-border-strong bg-surface px-2 py-0.5 font-bold tracking-wider text-slate-300">
            {online && health ? `KORUMA: ${health.gates} ZORUNLU KAPI · TAM-RASYONEL MUHASEBE` : "KORUMA — (motor çevrimdışı)"}
          </span>
          <span className="ml-auto flex shrink-0 items-center gap-1.5">
            <span
              className={cn(
                "inline-flex h-1.5 w-1.5 rounded-full",
                online && health?.venues.includes("BYBIT") ? "bg-emerald-400" : "bg-rose-400",
              )}
              aria-hidden
            />
            <span className={cn("font-bold tracking-wider", online ? "text-emerald-400" : "text-rose-400")}>
              BYBIT {online ? "BAĞLI" : "ÇEVRİMDIŞI"}
            </span>
          </span>
        </div>
      </footer>

      {/* ---------------- Mobile bottom nav ---------------- */}
      <nav
        className="fixed inset-x-0 bottom-0 z-40 flex items-stretch justify-around border-t border-border bg-surface/97 pb-[env(safe-area-inset-bottom)] backdrop-blur md:hidden"
        aria-label="Mobil gezinme"
      >
        {PRIMARY.map((id) => {
          const n = NAV.find((x) => x.id === id)!;
          const Icon = n.icon;
          return (
            <button
              key={n.id}
              onClick={() => go(n.id)}
              aria-current={navActive(n.id) ? "page" : undefined}
              className={cn(
                "flex min-h-14 min-w-14 flex-1 flex-col items-center justify-center gap-0.5 py-1.5 text-[11px] font-semibold",
                navActive(n.id) ? "text-emerald-400" : "text-slate-400",
              )}
            >
              <Icon className="h-5 w-5" aria-hidden />
              <span className="px-0.5 leading-tight">{SHORT_LABEL[n.id]}</span>
            </button>
          );
        })}
        <button
          onClick={() => setMoreOpen(true)}
          aria-expanded={moreOpen}
          aria-haspopup="dialog"
          className="flex min-h-14 min-w-14 flex-1 flex-col items-center justify-center gap-0.5 py-1.5 text-[11px] font-semibold text-slate-400"
        >
          <MoreHorizontal className="h-5 w-5" aria-hidden />
          <span className="px-0.5 leading-tight">Diğer</span>
        </button>
      </nav>
      {/* P1 — Diğer çekmecesi (mobil, 2. seviye) */}
      <Drawer open={moreOpen} onOpenChange={setMoreOpen}>
        <DrawerContent className="border-border bg-surface">
          <DrawerHeader className="text-left">
            <DrawerTitle className="text-sm text-slate-200">Diğer bölümler</DrawerTitle>
          </DrawerHeader>
          <div className="space-y-1 px-4 pb-8">
            {SECONDARY.map((id) => {
              const n = NAV.find((x) => x.id === id)!;
              const Icon = n.icon;
              return (
                <button
                  key={n.id}
                  onClick={() => go(n.id)}
                  aria-current={navActive(n.id) ? "page" : undefined}
                  className={cn(
                    "flex min-h-12 w-full items-center gap-3 rounded-lg px-3 text-sm font-medium",
                    navActive(n.id) ? "bg-emerald-500/10 text-emerald-300" : "text-slate-300",
                  )}
                >
                  <Icon className="h-5 w-5" aria-hidden />
                  {n.label}
                </button>
              );
            })}
          </div>
        </DrawerContent>
      </Drawer>

      {authLocked && (
        <AuthDialog
          onAuthenticated={() => {
            setAuthLocked(false);
            refetch();
          }}
        />
      )}
    </div>
  );
}
