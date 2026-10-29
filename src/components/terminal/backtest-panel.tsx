"use client";

import { useMemo, useState } from "react";
import { FlaskConical, Loader2, Play, Scale } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { EquityChart } from "@/components/terminal/charts";
import { EmptyState } from "@/components/terminal/bits";
import { TvChart, type TvMarker, type TvPlotSeries } from "@/components/terminal/tv-chart";
import { apiBacktest } from "@/hooks/use-engine";
import { fmtPctNum, fmtUsd } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { BacktestResult, BotConfig, BotKind, EntryTrigger, FillRealismBreakdown, Interval } from "@/lib/engine-types";

/* Task 21 — pine plot çizgi paleti (wizard ile aynı; mavi/indigo yok). */
const PLOT_COLORS = ["#34d399", "#22d3ee", "#f59e0b", "#a78bfa", "#f43f5e", "#fb923c"];

/* Task 22 — motor-dallı no-deal kodları → TR başlık. Bilinmeyen kod aynen
   gösterilir (uydurma etiket yok); detail zaten motorun kendi cümlesidir. */
const NO_DEAL_TITLES: Record<string, string> = {
  ENTRY_TRIGGER_NEVER_MET: "Başlangıç koşulu hiç sağlanmadı",
  SIGNAL_BLOCKED: "Sinyal ağacı engelledi (fail-closed)",
  REGIME_BLOCKED: "Rejim filtresi durdurdu (ZORUNLU)",
  BREAKER_BLOCKED: "Günlük DD devre kesici blokladı",
  COOLDOWN_ACTIVE: "Cooldown beklemesi",
  CONFIG_INVALID: "Geçersiz config (sayı/SL aralığı)",
  GRID_BOOTSTRAP_TRIGGER_MISSED: "Grid başlangıç koşulu sağlanmadı",
  GRID_BOOTSTRAP_SIGNAL_MISSED: "Grid sinyal beklemede",
  GRID_BOOTSTRAP_NEVER_MET: "Grid mesh hiç kurulamadı",
};

function NoDealBox({ reasons }: { reasons: NonNullable<BacktestResult["noDealReasons"]> }) {
  return (
    <div className="rounded-xl border border-amber-500/30 bg-amber-500/5 p-3">
      <div className="text-[11px] font-bold uppercase tracking-wider text-amber-400">
        Neden deal açılmadı — motor kararı ({reasons.length} neden)
      </div>
      <ul className="mt-2 space-y-1.5">
        {reasons.map((r) => (
          <li key={r.code} className="text-xs leading-relaxed text-slate-300">
            <span className="num rounded bg-amber-500/15 px-1.5 py-0.5 font-mono text-[10px] font-bold text-amber-300">
              {r.code}
            </span>{" "}
            <span className="font-semibold text-slate-100">{NO_DEAL_TITLES[r.code] ?? r.code}</span>{" "}
            <span className="num text-slate-400">· {r.bars} bar</span>
            <div className="mt-0.5 text-slate-400">{r.detail}</div>
          </li>
        ))}
      </ul>
    </div>
  );
}

/* ---------------- Result display (shared) ---------------- */

export function BacktestResultView({ result, symbol, interval }: { result: BacktestResult; symbol?: string; interval?: Interval }) {
  const ret = Number(result.metrics.total_return_pct);
  const m = result.metrics;
  /* Task 21 — pine plotlarını ana fiyat grafiğine çiz (Lab/Studio paylaşımlı).
     Sunucu backtest'in KULLANDIĞI bar penceresini "bars" olarak döner; plot
     değerleri bar dizinine hizalıdır → birebir üst üste biner. */
  const plotSeries = useMemo<TvPlotSeries[]>(() => {
    if (!result.pinePlots) return [];
    return Object.entries(result.pinePlots).map(([title, series], i) => ({
      title,
      color: PLOT_COLORS[i % PLOT_COLORS.length],
      values: series.map((v) => (v === "na" ? null : Number(v))),
    }));
  }, [result.pinePlots]);
  const markers = useMemo<TvMarker[] | undefined>(
    () => result.fillMarks?.map((f) => ({ time: f.t, side: f.side })),
    [result.fillMarks],
  );
  const hasPlotChart = !!result.bars && result.bars.length > 0 && (plotSeries.length > 0 || (markers?.length ?? 0) > 0);
  return (
    <div className="space-y-4">
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-4 xl:grid-cols-8">
        <MetricTile label="Toplam Getiri" value={fmtPctNum(m.total_return_pct)} tone={ret >= 0 ? "emerald" : "rose"} />
        <MetricTile label="Max DD" value={fmtPctNum(m.max_drawdown_pct)} tone="rose" />
        <MetricTile label="Win Rate" value={fmtPctNum(m.win_rate_pct)} tone="cyan" />
        {/* Faz 18 (P1-3): değer per-bar SR × 100'dür — yıllıklandırılmamış,
            okunabilirlik ölçekli; "Sharpe" etiketi kullanıcıyı yıllık
            Sharpe sanmaya itiyordu. Dürüst etiket. */}
        <MetricTile label="Per-bar SR (×100)" value={String(m.sharpe_display)} tone="slate" />
        <MetricTile label="Deal" value={String(m.deals)} tone="slate" mono={false} />
        <MetricTile label="Fill" value={String(result.fills)} tone="slate" mono={false} />
        <MetricTile label="Ücret" value={fmtUsd(m.total_fees)} tone="amber" />
        {m.total_funding != null && (
          <MetricTile
            label="Funding"
            value={fmtUsd(m.total_funding)}
            tone={Number(m.total_funding) > 0 ? "rose" : "emerald"}
          />
        )}
      </div>

      {/* Task 22 — boş sonuç artık sessiz değil: motorun engel kararları */}
      {m.deals === 0 && result.noDealReasons && result.noDealReasons.length > 0 && (
        <NoDealBox reasons={result.noDealReasons} />
      )}

      {/* Task 19 — dolum gerçekçiliği ayrıştırıcı gösterim */}
      {(m.fill_realism || result.realismCompare || (result.pineWarnings && result.pineWarnings.length > 0)) && (
        <FillRealismPanel
          realistic={m}
          realisticFills={result.fills}
          compare={result.realismCompare ?? null}
          pineWarnings={result.pineWarnings ?? []}
        />
      )}

      {/* Task 21 — fiyat grafiği: pine plot çizgileri + fill okları (bar penceresi varsa) */}
      {hasPlotChart && (
        <div className="rounded-xl border border-border bg-surface p-3">
          <div className="text-[11px] font-bold uppercase tracking-wider text-slate-400">
            Fiyat Grafiği — Pine Plotları + Fill İşaretleri
          </div>
          <div className="mt-2">
            <TvChart
              symbol={symbol ?? "?"}
              interval={interval ?? "1h"}
              data={result.bars}
              plotSeries={plotSeries.length > 0 ? plotSeries : undefined}
              markers={markers}
              height={340}
            />
          </div>
        </div>
      )}

      <div className="rounded-xl border border-border bg-surface p-3">
        <EquityChart
          points={result.equityCurve.map((p) => ({ ts: p.ts, equity: Number(p.equity) }))}
          height={260}
          color="#06B6D4"
        />
        <div className="num mt-2 flex flex-wrap justify-between gap-2 text-[11px] text-slate-400">
          <span>
            Başlangıç: <span className="text-slate-300">{fmtUsd(result.startEquity)}</span>
          </span>
          <span>
            Bitiş: <span className={cn(ret >= 0 ? "text-emerald-400" : "text-rose-400")}>{fmtUsd(result.endEquity)}</span>
          </span>
          <span>
            Bar: <span className="text-slate-300">{m.bars}</span> · Gate log: <span className="text-slate-300">{result.gateLogs}</span> · PF:{" "}
            <span className="text-slate-300">{String(m.profit_factor)}</span>
            {/* P4-3d: koşu verisi bu venue'den geldi */}
            {result.venue && (
              <>
                {" "}· Veri: <span className="font-mono font-bold text-cyan-300">{result.venue}</span>
              </>
            )}
          </span>
        </div>
      </div>
    </div>
  );
}

/**
 * Task 19 — Dolum gerçekçiliği ayrıştırıcı paneli:
 * (1) fill model etiket dağılımı (taker/resting/gap-better) + maker/taker fee ayrışı,
 * (2) GERÇEKÇİ vs İYİMSER ikinci koşu karşılaştırması — "teğet fitiller dolmuş
 * sayılsaydı ne olurdu" sorusunun dürüst cevabı; fark büyüdükçe strateji canlıda
 * o kadar zor tekrarlanır (backtest iyimserliği görünür kılınır).
 */
function FillRealismPanel({
  realistic,
  realisticFills,
  compare,
  pineWarnings,
}: {
  realistic: BacktestResult["metrics"];
  realisticFills: number;
  compare: BacktestResult["realismCompare"];
  pineWarnings: string[];
}) {
  const realism = realistic.fill_realism ?? null;
  const opt = compare?.optimisticMetrics ?? null;
  const retDelta =
    opt && compare
      ? Number(opt.total_return_pct) - Number(realistic.total_return_pct)
      : 0;
  const restingDelta =
    opt?.fill_realism && realism ? Math.max(0, opt.fill_realism.resting_touch - realism.resting_touch) : null;
  return (
    <div className="rounded-xl border border-border bg-surface p-4">
      <div className="flex items-center gap-2 text-[11px] font-bold uppercase tracking-wider text-slate-400">
        <Scale className="h-4 w-4 text-cyan-400" aria-hidden />
        Dolum Gerçekçiliği Ayrıştırıcı Analizi
      </div>
      {realism && (
        <div className="mt-3 grid grid-cols-2 gap-2 text-[11px] sm:grid-cols-5">
          <div className="rounded-lg border border-border p-2">
            <div className="text-slate-500">Marketable (Taker)</div>
            <div className="num text-base font-bold text-rose-300">{realism.marketable_taker}</div>
            <div className="text-[9px] text-slate-400">piyasa emri / çapraz limit — taker fee</div>
          </div>
          <div className="rounded-lg border border-border p-2">
            <div className="text-slate-500">Resting Touch</div>
            <div className="num text-base font-bold text-cyan-300">{realism.resting_touch}</div>
            <div className="text-[9px] text-slate-400">bekleyen limit — 1 tick penetrasyon şartı</div>
          </div>
          <div className="rounded-lg border border-border p-2">
            <div className="text-slate-500">Open Gap Better</div>
            <div className="num text-base font-bold text-emerald-300">{realism.open_gap_better}</div>
            <div className="text-[9px] text-slate-400">gap ile daha iyi fiyattan — maker fee</div>
          </div>
          <div className="rounded-lg border border-border p-2">
            <div className="text-slate-500">Maker Fee Toplamı</div>
            <div className="num text-base font-bold text-slate-200">{Number(realism.maker_fees).toFixed(4)}</div>
          </div>
          <div className="rounded-lg border border-border p-2">
            <div className="text-slate-500">Taker Fee Toplamı</div>
            <div className="num text-base font-bold text-amber-300">{Number(realism.taker_fees).toFixed(4)}</div>
          </div>
        </div>
      )}
      {opt && (
        <div className="mt-3 rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
          <div className="text-[11px] font-bold text-amber-300">Gerçekçi vs İyimser Dolum Karşılaştırması (aynı config, iki model)</div>
          <div className="mt-2 grid grid-cols-2 gap-2 text-[11px] sm:grid-cols-4">
            <div>
              <div className="text-slate-500">Fill (gerçekçi → iyimser)</div>
              <div className="num font-bold text-slate-200">
                {realisticFills} → {compare?.optimisticFills ?? "?"}
              </div>
            </div>
            <div>
              <div className="text-slate-500">Ek Resting dolumu (iyimserde)</div>
              <div className="num font-bold text-slate-200">{restingDelta === null ? "—" : `+${restingDelta}`}</div>
            </div>
            <div>
              <div className="text-slate-500">Getiri farkı (iyimser − gerçekçi)</div>
              <div className={cn("num font-bold", retDelta > 0.0001 ? "text-amber-300" : "text-slate-300")}>
                {retDelta >= 0 ? "+" : ""}
                {retDelta.toFixed(4)}%
              </div>
            </div>
            <div>
              <div className="text-slate-500">İyimser Getiri</div>
              <div className="num font-bold text-slate-200">{fmtPctNum(opt.total_return_pct)}</div>
            </div>
          </div>
          <p className="mt-2 text-[10px] leading-relaxed text-slate-400">
            İyimser modda fiyata TEĞET geçen fitiller &quot;dolmuş&quot; sayılır — canlı borsada bu emirlerin çoğu dolmaz. Gerçekçi
            koşu (varsayılan) 1 tick penetrasyon şartı arar; fark büyüdükçe backtest sonucu canlıda o kadar zor tekrarlanır.
          </p>
        </div>
      )}
      {pineWarnings.length > 0 && (
        <div className="mt-3 rounded-lg border border-border-strong bg-surface p-2 text-[10px] text-slate-400">
          <span className="font-bold text-slate-300">Pine uyarıları:</span>
          {pineWarnings.map((w, i) => (
            <div key={i}>· {w}</div>
          ))}
        </div>
      )}
    </div>
  );
}

function MetricTile({
  label,
  value,
  tone,
  mono = true,
}: {
  label: string;
  value: string;
  tone: "emerald" | "rose" | "cyan" | "amber" | "slate";
  mono?: boolean;
}) {
  const cls = {
    emerald: "text-emerald-400",
    rose: "text-rose-400",
    cyan: "text-cyan-400",
    amber: "text-amber-400",
    slate: "text-slate-200",
  }[tone];
  return (
    <div className="rounded-lg border border-border bg-surface p-3">
      <div className="truncate text-[10px] uppercase tracking-wider text-slate-400">{label}</div>
      <div className={cn("mt-1 truncate text-lg font-semibold", mono && "num", cls)}>{value}</div>
    </div>
  );
}

/* ---------------- Quick runner (Studio tab) ---------------- */

export function StudioBacktestRunner({
  kind,
  symbol,
  interval,
  config,
  startCash,
}: {
  kind: BotKind;
  symbol: string;
  interval: Interval;
  config: BotConfig;
  /** Bot's live cash (USDT) — falls back to the historical 100k sandbox default. */
  startCash?: string;
}) {
  const [bars, setBars] = useState(500);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<BacktestResult | null>(null);
  /* start-condition override */
  const [scMode, setScMode] = useState<"DEFAULT" | "PRICE_BELOW" | "PRICE_ABOVE" | "SCHEDULED">("DEFAULT");
  const [scPrice, setScPrice] = useState("");
  const [scDateTime, setScDateTime] = useState("");

  const buildTrigger = (): EntryTrigger | undefined => {
    if (scMode === "DEFAULT") return undefined;
    if (scMode === "PRICE_BELOW") return { PRICE_BELOW: scPrice || "0" };
    if (scMode === "PRICE_ABOVE") return { PRICE_ABOVE: scPrice || "0" };
    return { SCHEDULED: { start_ms: Date.parse(scDateTime) || 0 } };
  };

  const scValid =
    scMode === "DEFAULT" ||
    (scMode === "SCHEDULED" ? !Number.isNaN(Date.parse(scDateTime)) : Number(scPrice) > 0);

  const run = async () => {
    setBusy(true);
    setError(null);
    try {
      const trigger = buildTrigger();
      const r = await apiBacktest({
        kind,
        symbol,
        interval,
        bars,
        start_cash: startCash && Number(startCash) > 0 ? startCash : "100000",
        config: { ...config, kind, symbol, ...(trigger ? { entry_trigger: trigger } : {}) },
      });
      setResult(r);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Bilinmeyen hata");
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-4">
      <div className="flex flex-col gap-4 rounded-xl border border-border bg-surface p-4 sm:flex-row sm:items-end">
        <div className="flex-1 space-y-1.5">
          <div className="flex justify-between text-xs text-slate-400">
            <Label className="text-xs text-slate-400">Bar sayısı</Label>
            <span className="num text-slate-300">{bars}</span>
          </div>
          <Slider value={[bars]} min={100} max={1000} step={100} onValueChange={(v) => setBars(v[0] ?? 500)} aria-label="Bar sayısı" />
        </div>
        <div className="flex-1 space-y-1.5">
          <Label className="text-xs text-slate-400">Başlangıç Koşulu</Label>
          <div className="flex gap-1">
            {(["DEFAULT", "PRICE_BELOW", "PRICE_ABOVE", "SCHEDULED"] as const).map((m) => (
              <button
                key={m}
                onClick={() => setScMode(m)}
                aria-pressed={scMode === m}
                className={cn(
                  "min-h-9 flex-1 rounded-md border px-1.5 py-1 text-[10px] font-bold transition-colors",
                  scMode === m ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500 hover:border-border-strong",
                )}
              >
                {m === "DEFAULT" ? "BOTUN KONFİGÜ" : m === "SCHEDULED" ? "ZAMAN" : m === "PRICE_BELOW" ? "F≤" : "F≥"}
              </button>
            ))}
          </div>
          {scMode === "PRICE_BELOW" || scMode === "PRICE_ABOVE" ? (
            <Input
              type="number"
              value={scPrice}
              onChange={(e) => setScPrice(e.target.value)}
              placeholder="eşik fiyat"
              className="num min-h-9 border-border-strong bg-surface text-xs text-slate-100"
              aria-label="Koşul eşik fiyatı"
            />
          ) : scMode === "SCHEDULED" ? (
            <Input
              type="datetime-local"
              value={scDateTime}
              onChange={(e) => setScDateTime(e.target.value)}
              className="num min-h-9 border-border-strong bg-surface text-xs text-slate-100"
              aria-label="Koşul başlangıç zamanı"
            />
          ) : null}
        </div>
        <Button onClick={() => void run()} disabled={busy || !scValid} className="min-h-11 bg-cyan-500/90 text-[#02171b] hover:bg-cyan-500 sm:min-h-10">
          {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <Play className="h-4 w-4" aria-hidden />}
          {busy ? "Çalışıyor…" : "Backtest Çalıştır"}
        </Button>
      </div>

      {error && (
        <div className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">{error}</div>
      )}

      {result ? (
        <BacktestResultView result={result} symbol={symbol} interval={interval} />
      ) : (
        !busy &&
        !error && (
          <EmptyState
            icon={FlaskConical}
            title="Henüz backtest çalıştırılmadı"
            hint="Botun stratejisi, sembolü ve interval'i ile tarihsel simülasyon koşturun."
          />
        )
      )}
    </div>
  );
}

/* ---------------- Standalone quick config builder for Lab ---------------- */

export function defaultDcaConfig(kind: BotKind, symbol: string): BotConfig {
  return {
    kind,
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
