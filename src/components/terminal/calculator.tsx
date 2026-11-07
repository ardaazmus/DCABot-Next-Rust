"use client";

import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  Calculator as CalcIcon,
  Grid3x3,
  Loader2,
  RefreshCw,
  Thermometer,
  TrendingDown,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import { LadderTable, MeshTable } from "@/components/terminal/ladder";
import { Panel, PassChip, Spinner } from "@/components/terminal/bits";
import { apiBrackets, apiCalcDca, apiCalcGrid, apiGridSuggest, apiKlines, apiMarkets } from "@/hooks/api-analysis";
import { toast } from "@/hooks/use-toast";
import { fmtFrac, fmtNum, fmtUsd, fracToNum } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { BracketsTier, CalcDcaResult, CalcGridResult } from "@/lib/engine-types";

/* ------------------------------------------------------------------ */

function NumIn({
  label,
  value,
  onChange,
  step = "0.01",
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  step?: string;
}) {
  return (
    <div className="space-y-1.5">
      <Label className="text-xs text-slate-400">{label}</Label>
      <Input
        type="number"
        step={step}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="num min-h-11 border-border-strong bg-surface text-slate-100"
      />
    </div>
  );
}

function SlideField({
  label,
  value,
  min,
  max,
  step,
  fmt,
  onChange,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  fmt?: (v: number) => string;
  onChange: (v: number) => void;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <Label className="text-xs text-slate-400">{label}</Label>
        <span className="num text-sm font-semibold text-slate-200">{fmt ? fmt(value) : value}</span>
      </div>
      <Slider value={[value]} min={min} max={max} step={step} onValueChange={(v) => onChange(v[0] ?? min)} aria-label={label} />
    </div>
  );
}

/* ---------------- Liquidation thermometer ---------------- */

function LiquidationBar({ anchor, liq, leverage }: { anchor: number; liq: number | null; leverage: number }) {
  if (!liq || liq <= 0 || anchor <= 0) {
    return (
      <div className="rounded-lg border border-border bg-surface p-3 text-xs text-slate-500">
        Likidasyon fiyatı hesaplanamadı (spot modda veya sınır dışı).
      </div>
    );
  }
  const max = Math.max(anchor, liq) * 1.15;
  const liqPct = (liq / max) * 100;
  const anchorPct = (anchor / max) * 100;
  const bufferPct = Math.max(0, ((anchor - liq) / anchor) * 100);
  return (
    <div className="rounded-lg border border-border bg-surface p-4">
      <div className="mb-2 flex items-center justify-between text-xs">
        <span className="flex items-center gap-1.5 text-slate-400">
          <Thermometer className="h-3.5 w-3.5 text-rose-400" aria-hidden />
          Likidasyon Termometresi
        </span>
        <span className="num text-slate-300">{leverage}x · tampon %{fmtNum(bufferPct, 1)}</span>
      </div>
      <div className="relative h-7 w-full overflow-hidden rounded-full bg-secondary" role="img" aria-label="Likidasyon mesafesi">
        <div
          className="absolute inset-y-0 left-0 bg-gradient-to-r from-rose-600/80 to-rose-500/40"
          style={{ width: `${liqPct}%` }}
        />
        <div className="absolute inset-y-0 w-0.5 bg-slate-100" style={{ left: `${anchorPct}%` }} />
        <span className="num absolute left-2 top-1/2 -translate-y-1/2 text-[10px] font-bold text-white/90">
          LIQ {fmtNum(liq, 2)}
        </span>
        <span
          className="num absolute top-1/2 -translate-y-1/2 whitespace-nowrap text-[10px] font-bold text-slate-100"
          style={{ left: `calc(${anchorPct}% + 6px)` }}
        >
          ▲ ANCHOR {fmtNum(anchor, 2)}
        </span>
      </div>
      <div className="mt-1.5 flex justify-between text-[10px] text-slate-400">
        <span>0</span>
        <span className="num">{fmtNum(max, 2)}</span>
      </div>
    </div>
  );
}

/* ---------------- Faz 2.7 kademe aynası (M07 venue mirror) ---------------- */

interface TierEdit {
  floor: string;
  cap: string;
  mmr: string;
  cum: string;
}

const EMPTY_TIER: TierEdit = { floor: "0", cap: "", mmr: "", cum: "" };

/**
 * Bakım marjı kademeleri: venue'den al (salt-okunur ayna) ya da elle gir
 * (yalnız bu oturumda, kaydedilmez). Kademe yoksa likidasyon tahmini
 * yürütülmez algısı YANLIŞ olur diye panel dürüst not taşır: hesaplayıcı
 * tahmini tek-kademe (%0.5) kullanır, bu tablo tahmine katılmaz.
 */
function BracketsPanel({ symbol }: { symbol: string }) {
  const [venue, setVenue] = useState<"BYBIT" | "BINANCE">("BYBIT");
  const [tiers, setTiers] = useState<BracketsTier[] | null>(null);
  const [source, setSource] = useState<string | null>(null);
  const [reason, setReason] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [editing, setEditing] = useState(false);
  const [manual, setManual] = useState<TierEdit[]>([{ ...EMPTY_TIER }]);

  const fetchTiers = async () => {
    setBusy(true);
    setReason(null);
    try {
      const r = await apiBrackets(venue, symbol);
      setSource(r.source);
      if (r.source === "EXCHANGE") {
        setTiers(r.tiers);
        setEditing(false);
      } else {
        setTiers(null);
        setReason(r.reason ?? (r.source === "ABSTAIN" ? "Anahtar yok ya da venue reddetti — sahte kademe üretilmez." : "Bu venue desteklenmiyor."));
      }
    } catch (e) {
      setTiers(null);
      setSource(null);
      setReason(e instanceof Error ? e.message : "Kademe alınamadı");
    } finally {
      setBusy(false);
    }
  };

  const setCell = (i: number, k: keyof TierEdit, v: string) =>
    setManual((t) => t.map((row, j) => (j === i ? { ...row, [k]: v } : row)));

  const rows: (BracketsTier | TierEdit)[] | null = editing ? manual : tiers;
  const mmrPct = (m: string) => {
    const v = fracToNum(m);
    return v === null ? "—" : `%${fmtNum(v * 100, 3)}`;
  };

  return (
    <Panel
      title="Bakım Marjı Kademeleri (M07)"
      icon={<Thermometer className="h-4 w-4 text-slate-500" aria-hidden />}
      right={source === "EXCHANGE" ? <PassChip ok label={`M07 · ${tiers?.length ?? 0} kademe`} /> : undefined}
    >
      <div className="flex flex-wrap items-center gap-2">
        {(["BYBIT", "BINANCE"] as const).map((v) => (
          <button
            key={v}
            onClick={() => { setVenue(v); setTiers(null); setSource(null); setReason(null); }}
            className={cn("rounded-lg border px-3 py-1.5 font-mono text-[11px] font-bold", venue === v ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500")}
          >
            {v}
          </button>
        ))}
        <Button size="sm" className="min-h-9" onClick={() => void fetchTiers()} disabled={busy}>
          {busy ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : venue === "BYBIT" ? "Venue'den al (anahtarsız)" : "Venue'den al (imzalı)"}
        </Button>
        <Button size="sm" variant="outline" className="min-h-9 border-border-strong" onClick={() => setEditing((e) => !e)}>
          {editing ? "Tabloya dön" : "Elle gir"}
        </Button>
      </div>
      {reason && <p role="alert" className="mt-2 rounded-lg border border-amber-500/40 bg-amber-500/10 p-2.5 text-[11px] text-amber-300">{reason}</p>}
      {rows && rows.length > 0 && (
        <div className="mt-2 overflow-x-auto rounded-lg border border-border">
          <table className="w-full min-w-[520px] text-left text-[11px]">
            <thead>
              <tr className="border-b border-border text-slate-500">
                <th className="px-2 py-1.5 font-semibold">#</th>
                <th className="px-2 py-1.5 font-semibold">Taban (dahil)</th>
                <th className="px-2 py-1.5 font-semibold">Tavan (hariç)</th>
                <th className="px-2 py-1.5 font-semibold">MMR</th>
                <th className="px-2 py-1.5 font-semibold">Kümülatif</th>
                {editing && <th className="px-2 py-1.5" />}
              </tr>
            </thead>
            <tbody>
              {rows.map((t, i) => {
                const floor = editing ? (t as TierEdit).floor : (t as BracketsTier).min_notional;
                const cap = editing ? (t as TierEdit).cap : (t as BracketsTier).max_notional;
                const mmr = editing ? (t as TierEdit).mmr : (t as BracketsTier).mmr;
                const cum = editing ? (t as TierEdit).cum : (t as BracketsTier).deduction;
                return (
                  <tr key={i} className="num border-b border-[#161c28] last:border-0">
                    <td className="px-2 py-1.5 text-slate-500">{i + 1}</td>
                    <td className="px-2 py-1.5 text-slate-200">
                      {editing
                        ? <Input value={floor} onChange={(e) => setCell(i, "floor", e.target.value)} inputMode="decimal" aria-label={`${i + 1}. kademe taban`} className="h-8 border-border-strong bg-surface font-mono text-[11px]" />
                        : fmtUsd(floor, 0)}
                    </td>
                    <td className="px-2 py-1.5 text-slate-200">
                      {editing
                        ? <Input value={cap} onChange={(e) => setCell(i, "cap", e.target.value)} inputMode="decimal" placeholder="boş = ∞" aria-label={`${i + 1}. kademe tavan`} className="h-8 border-border-strong bg-surface font-mono text-[11px]" />
                        : (cap === "" ? "∞" : fmtUsd(cap, 0))}
                    </td>
                    <td className="px-2 py-1.5 text-slate-200">
                      {editing
                        ? <Input value={mmr} onChange={(e) => setCell(i, "mmr", e.target.value)} inputMode="decimal" placeholder="0.005" aria-label={`${i + 1}. kademe oran`} className="h-8 border-border-strong bg-surface font-mono text-[11px]" />
                        : mmrPct(mmr)}
                    </td>
                    <td className="px-2 py-1.5 text-slate-200">
                      {editing
                        ? <Input value={cum} onChange={(e) => setCell(i, "cum", e.target.value)} inputMode="decimal" placeholder="0" aria-label={`${i + 1}. kademe kümülatif`} className="h-8 border-border-strong bg-surface font-mono text-[11px]" />
                        : fmtUsd(cum, 0)}
                    </td>
                    {editing && manual.length > 1 && (
                      <td className="px-2 py-1.5">
                        <button onClick={() => setManual((m) => m.filter((_, j) => j !== i))} aria-label={`${i + 1}. kademeyi sil`} className="text-[11px] text-rose-400 hover:text-rose-300">sil</button>
                      </td>
                    )}
                  </tr>
                );
              })}
            </tbody>
          </table>
        </div>
      )}
      {editing && (
        <div className="mt-2 flex flex-wrap gap-2">
          <Button size="sm" variant="outline" className="min-h-9 border-border-strong" onClick={() => setManual((m) => [...m, { ...EMPTY_TIER }])}>
            Kademe ekle
          </Button>
          <span className="self-center text-[10px] text-slate-400">Elle girilen kademeler yalnızca bu oturumda durur — kaydedilmez, tahmine katılmaz.</span>
        </div>
      )}
      <p className="mt-2 text-[10px] leading-relaxed text-slate-400">
        Hesaplayıcı likidasyon tahmini tek-kademe (%0.5 MMR) kullanır — bu tablo operatör referansıdır, tahmine katılmaz. Kademe yoksa sahte kademe üretilmez.
      </p>
    </Panel>
  );
}

/* ---------------- DCA tab ---------------- */

interface MicroStructure {
  tickSize: string;
  stepSize: string;
  makerFee: string;
  takerFee: string;
}

const DEFAULT_MICRO: MicroStructure = {
  tickSize: "0.01",
  stepSize: "0.00001",
  makerFee: "0.001",
  takerFee: "0.001",
};

function DcaCalc({
  marketPrice,
  marketPriceSeq,
  micro,
  market,
  symbol,
  onMarketChange,
}: {
  marketPrice: string | null;
  marketPriceSeq: number;
  micro: MicroStructure;
  market: "spot" | "linear";
  symbol: string;
  onMarketChange: (m: "spot" | "linear") => void;
}) {
  const [anchor, setAnchor] = useState("100");
  const [base, setBase] = useState("100");
  const [safety, setSafety] = useState("100");
  const [deviation, setDeviation] = useState(2);
  const [stepScale, setStepScale] = useState(1.3);
  const [volumeScale, setVolumeScale] = useState(1.2);
  const [maxSafety, setMaxSafety] = useState(5);
  const [tp, setTp] = useState(2);
  const [netTp, setNetTp] = useState(true);
  const [sl, setSl] = useState(3);
  const [slOn, setSlOn] = useState(true);
  const [leverage, setLeverage] = useState(1);
  const [res, setRes] = useState<CalcDcaResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  /* "Piyasa Fiyatı" fetch → anchor input'una yansır (seq her fetch'te artar). */
  useEffect(() => {
    if (marketPrice) setAnchor(marketPrice);
  }, [marketPrice, marketPriceSeq]);

  useEffect(() => {
    if (timer.current) clearTimeout(timer.current);
    setBusy(true);
    timer.current = setTimeout(async () => {
      try {
        setErr(null);
        const r = await apiCalcDca({
          anchor: anchor || "0",
          base_order_size: base || "0",
          safety_order_size: safety || "0",
          sizing_mode: "QUOTE_NOTIONAL",
          deviation_pct: String(deviation / 100),
          step_scale: String(stepScale),
          volume_scale: String(volumeScale),
          max_safety_orders: maxSafety,
          target_profit_pct: String(tp / 100),
          use_net_tp: netTp,
          stop_loss_pct: slOn ? String(sl / 100) : null,
          tick_size: micro.tickSize,
          step_size: micro.stepSize,
          maker_fee: micro.makerFee,
          taker_fee: micro.takerFee,
          leverage: String(leverage),
          market,
        });
        setRes(r);
      } catch (e) {
        setRes(null);
        setErr(e instanceof Error ? e.message : "Hesaplama hatası");
      } finally {
        setBusy(false);
      }
    }, 400);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [anchor, base, safety, deviation, stepScale, volumeScale, maxSafety, tp, netTp, sl, slOn, leverage, market, micro]);

  const m20From = res?.m20.firstUnreachableLevel ?? null;

  return (
    <div className="grid gap-4 xl:grid-cols-3">
      <Panel title="DCA Parametreleri" icon={<CalcIcon className="h-4 w-4 text-slate-500" aria-hidden />}>
        <div className="space-y-4">
          <div className="grid grid-cols-3 gap-2">
            <NumIn label="Anchor" value={anchor} onChange={setAnchor} />
            <NumIn label="Base" value={base} onChange={setBase} />
            <NumIn label="Safety" value={safety} onChange={setSafety} />
          </div>
          <SlideField label="Deviation %" value={deviation} min={0.5} max={10} step={0.1} fmt={(v) => `${v.toFixed(1)}%`} onChange={setDeviation} />
          <SlideField label="Step Scale" value={stepScale} min={1} max={3} step={0.05} fmt={(v) => v.toFixed(2)} onChange={setStepScale} />
          <SlideField label="Volume Scale" value={volumeScale} min={1} max={3} step={0.05} fmt={(v) => v.toFixed(2)} onChange={setVolumeScale} />
          <SlideField label="Maks. Güvenlik" value={maxSafety} min={0} max={10} step={1} onChange={(v) => setMaxSafety(Math.round(v))} />
          <SlideField label="Hedef Kâr %" value={tp} min={0.2} max={10} step={0.1} fmt={(v) => `${v.toFixed(1)}%`} onChange={setTp} />
          <div className="flex items-center justify-between rounded-lg border border-border bg-surface p-2.5">
            <span className="text-xs text-slate-300">Net TP (ücret sonrası)</span>
            <Switch checked={netTp} onCheckedChange={setNetTp} aria-label="Net TP" />
          </div>
          <div className="flex items-center justify-between rounded-lg border border-border bg-surface p-2.5">
            <span className="text-xs text-slate-300">Stop-Loss</span>
            <Switch checked={slOn} onCheckedChange={setSlOn} aria-label="Stop-loss" />
          </div>
          {slOn && <SlideField label="SL %" value={sl} min={0.5} max={30} step={0.1} fmt={(v) => `${v.toFixed(1)}%`} onChange={setSl} />}
          {slOn && deviation > 0 && sl <= deviation && (
            /* Faz 25 (UIX-31): "M20 CONFLICT RISK" → insan dili. */
            <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-2.5 text-[11px] text-amber-300">
              <AlertTriangle className="mr-1 inline h-3.5 w-3.5" aria-hidden />
              SL %{sl}, güvenlik merdiveninden önce tetikleniyor (sapma %{deviation}) — Risk Motoru bölümünde onay istenecek.
            </div>
          )}
          <div className="flex gap-2">
            <button
              onClick={() => onMarketChange("spot")}
              className={cn("flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold", market === "spot" ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500")}
            >
              SPOT
            </button>
            <button
              onClick={() => onMarketChange("linear")}
              className={cn("flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold", market === "linear" ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500")}
            >
              LINEAR
            </button>
          </div>
          {market === "linear" && <SlideField label="Kaldıraç" value={leverage} min={1} max={50} step={1} fmt={(v) => `${v}x`} onChange={(v) => setLeverage(Math.round(v))} />}
        </div>
      </Panel>

      <div className="space-y-4 xl:col-span-2">
        {busy && !res && (
          <div className="flex justify-center py-10">
            <Spinner className="h-6 w-6" />
          </div>
        )}
        {err && <div className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">{err}</div>}
        {res && (
          <>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              <MiniStat label="Toplam Notional" value={fmtUsd(res.totalNotionalDisplay ?? res.totalNotional)} />
              <MiniStat label="Toplam Miktar" value={fmtFrac(res.totalQty)} />
              <MiniStat label="Ort. Giriş (WAC)" value={fmtFrac(res.avgAllIn)} tone="cyan" />
              <MiniStat label="Tahmini Ücret" value={fmtUsd(res.estimatedFees, 4)} tone="amber" />
            </div>
            <Panel
              title="Ladder + WAC Trajektorisi"
              icon={<CalcIcon className="h-4 w-4 text-slate-500" aria-hidden />}
              right={
                busy ? (
                  <Loader2 className="h-4 w-4 animate-spin text-slate-500" aria-hidden />
                ) : (
                  <PassChip ok={res.m20.viable} label="M20" />
                )
              }
              bodyClassName="p-0 px-2 py-2"
            >
              <LadderTable
                rows={res.levels}
                m20From={m20From}
                /* Faz 25 (UIX-31): SL altı seviyeler artık kırmızı alarm DEĞİL —
                    "SL ALTINDA" nötr (beklenen davranış); kırmızı yalnız gerçek
                    çatışma için. */
                slPrice={slOn ? (fracToNum(anchor) ?? 0) * (1 - sl / 100) : null}
                dir="long"
              />
              {m20From !== null && (
                <p className="px-2 pb-1 pt-2 text-[11px] text-rose-400">
                  <AlertTriangle className="mr-1 inline h-3 w-3" aria-hidden />
                  L{m20From}&apos;den itibaren seviyeler SL öncesinde erişilemez: SL altındakiler nötr &quot;SL ALTINDA&quot;, geride kalan gerçek çatışmalar kırmızı işaretlenir.
                </p>
              )}
            </Panel>
            {market === "linear" && <LiquidationBar anchor={fracToNum(anchor) ?? 0} liq={res.liquidationPrice ? fracToNum(res.liquidationPrice) : null} leverage={leverage} />}
            {market === "linear" && <BracketsPanel symbol={symbol} />}
          </>
        )}
      </div>
    </div>
  );
}

/* ---------------- Grid tab ---------------- */

function GridCalc({ micro }: { micro: MicroStructure }) {
  const [lower, setLower] = useState("90");
  const [upper, setUpper] = useState("110");
  const [levels, setLevels] = useState(10);
  const [mode, setMode] = useState<"arithmetic" | "geometric">("arithmetic");
  const [notional, setNotional] = useState("100");
  const [res, setRes] = useState<CalcGridResult | null>(null);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);
  /* Faz 10 — Otomatik grid önerisi (motor: /api/analysis/grid-suggest). */
  const [gsSymbol, setGsSymbol] = useState("BTCUSDT");
  const [gsInvestment, setGsInvestment] = useState("1000");
  const [gsBusy, setGsBusy] = useState(false);
  const [gsInfo, setGsInfo] = useState<string | null>(null);
  const [gsErr, setGsErr] = useState<string | null>(null);

  const runGridSuggest = async () => {
    setGsBusy(true);
    setGsErr(null);
    try {
      const r = await apiGridSuggest(gsSymbol.trim().toUpperCase() || "BTCUSDT", "1h", 500, gsInvestment || "1000");
      setLower(r.windowLow);
      setUpper(r.windowHigh);
      setLevels(Math.min(60, Math.max(2, r.suggestedLevels)));
      setNotional(r.perLevelNotional);
      setGsInfo(
        `σ %{${r.sigmaPct}} · adım %{${r.stepPct}} · g_min %{${r.gMinPct}} · ücret-kırıcı ${r.feeBreakerPass ? "GEÇTİ" : "KALDI"}`,
      );
    } catch (e) {
      setGsInfo(null);
      setGsErr(e instanceof Error ? e.message : "Öneri çalışmadı");
    } finally {
      setGsBusy(false);
    }
  };

  useEffect(() => {
    if (timer.current) clearTimeout(timer.current);
    setBusy(true);
    timer.current = setTimeout(async () => {
      try {
        setErr(null);
        const lo = fracToNum(lower) ?? 0;
        const hi = fracToNum(upper) ?? 0;
        if (lo <= 0 || hi <= lo) {
          setRes(null);
          setErr("Alt sınır > 0 ve üst > alt olmalı.");
          return;
        }
        const r = await apiCalcGrid({
          lower: String(lo),
          upper: String(hi),
          levels,
          grid_mode: mode,
          notional_per_level: notional || "0",
          tick_size: micro.tickSize,
          step_size: micro.stepSize,
          maker_fee: micro.makerFee,
          taker_fee: micro.takerFee,
        });
        setRes(r);
      } catch (e) {
        setRes(null);
        setErr(e instanceof Error ? e.message : "Hesaplama hatası");
      } finally {
        setBusy(false);
      }
    }, 400);
    return () => {
      if (timer.current) clearTimeout(timer.current);
    };
  }, [lower, upper, levels, mode, notional, micro]);

  return (
    <div className="grid gap-4 xl:grid-cols-3">
      <Panel title="Grid Parametreleri" icon={<Grid3x3 className="h-4 w-4 text-slate-500" aria-hidden />}>
        <div className="space-y-4">
          {/* Faz 10 — otomatik öneri: menzil + sigma → dört girdiyi doldurur */}
          <div className="rounded-lg border border-cyan-500/30 bg-cyan-500/5 p-2.5 space-y-2">
            <p className="text-[11px] font-bold text-cyan-300">Otomatik Grid Önerisi</p>
            <div className="grid grid-cols-2 gap-2">
              <div className="space-y-1">
                <Label className="text-[10px] text-slate-400">Sembol</Label>
                <Input
                  value={gsSymbol}
                  onChange={(e) => setGsSymbol(e.target.value.toUpperCase())}
                  placeholder="BTCUSDT"
                  className="num h-8 text-xs"
                  aria-label="Öneri sembolü"
                />
              </div>
              <div className="space-y-1">
                <Label className="text-[10px] text-slate-400">Toplam Yatırım (USDT)</Label>
                <Input
                  value={gsInvestment}
                  onChange={(e) => setGsInvestment(e.target.value)}
                  inputMode="decimal"
                  className="num h-8 text-xs"
                  aria-label="Toplam yatırım"
                />
              </div>
            </div>
            <Button
              type="button"
              onClick={() => void runGridSuggest()}
              disabled={gsBusy}
              className="h-8 w-full text-[11px] font-bold"
              title="Son 500 barın menzili ve oynaklığından alt/üst/seviye/notional öner — girdileri otomatik doldurur"
            >
              {gsBusy ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : <Grid3x3 className="h-3.5 w-3.5" aria-hidden />}
              Otomatik Öner
            </Button>
            {gsInfo && <p className="num text-[10px] text-cyan-200/80">{gsInfo}</p>}
            {gsErr && <p className="text-[10px] text-rose-400">{gsErr}</p>}
          </div>
          <div className="grid grid-cols-2 gap-2">
            <NumIn label="Alt Sınır" value={lower} onChange={setLower} />
            <NumIn label="Üst Sınır" value={upper} onChange={setUpper} />
          </div>
          <div className="flex gap-2">
            {(["arithmetic", "geometric"] as const).map((m) => (
              <button
                key={m}
                onClick={() => setMode(m)}
                className={cn(
                  "flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold",
                  mode === m ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500",
                )}
              >
                {m}
              </button>
            ))}
          </div>
          <SlideField label="Seviye Sayısı" value={levels} min={2} max={60} step={1} onChange={(v) => setLevels(Math.round(v))} />
          <NumIn label="Seviye Başına Notional" value={notional} onChange={setNotional} />
        </div>
      </Panel>

      <div className="space-y-4 xl:col-span-2">
        {busy && !res && (
          <div className="flex justify-center py-10">
            <Spinner className="h-6 w-6" />
          </div>
        )}
        {err && <div className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">{err}</div>}
        {res && (
          <>
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-4">
              <MiniStat label="Toplam Yatırım" value={fmtUsd(res.totalInvested)} />
              <MiniStat label="Mesh Seviyesi" value={String(res.mesh.length)} />
              <MiniStat
                label="Adım Aralığı"
                value={
                  res.mesh.length > 1
                    ? `${fmtNum((((fracToNum(upper) ?? 0) - (fracToNum(lower) ?? 0)) / (res.mesh.length - 1)), 4)}`
                    : "—"
                }
                tone="cyan"
              />
              <MiniStat label="Fee-Breaker" value={res.feeBreakerPass ? "PASS" : "FAIL"} tone={res.feeBreakerPass ? "emerald" : "rose"} />
            </div>
            <Panel
              title="Grid Mesh"
              icon={<Grid3x3 className="h-4 w-4 text-slate-500" aria-hidden />}
              right={busy ? <Loader2 className="h-4 w-4 animate-spin text-slate-500" aria-hidden /> : undefined}
              bodyClassName="p-0 px-2 py-2"
            >
              <MeshTable rows={res.mesh} />
            </Panel>
          </>
        )}
      </div>
    </div>
  );
}

function MiniStat({ label, value, tone = "slate" }: { label: string; value: string; tone?: "emerald" | "rose" | "cyan" | "amber" | "slate" }) {
  const cls = {
    emerald: "text-emerald-400",
    rose: "text-rose-400",
    cyan: "text-cyan-400",
    amber: "text-amber-400",
    slate: "text-slate-100",
  }[tone];
  return (
    <div className="rounded-lg border border-border bg-surface p-3">
      <div className="truncate text-[10px] uppercase tracking-wider text-slate-400">{label}</div>
      <div className={cn("num mt-1 truncate text-sm font-semibold", cls)}>{value}</div>
    </div>
  );
}

/* ---------------- main ---------------- */

export function CalculatorView() {
  const [symbolInput, setSymbolInput] = useState("BTCUSDT");
  const [symbol, setSymbol] = useState("BTCUSDT");
  const [market, setMarket] = useState<"spot" | "linear">("spot");
  const [micro, setMicro] = useState<MicroStructure>(DEFAULT_MICRO);
  const [microLive, setMicroLive] = useState(false);
  const [dcaAnchor, setDcaAnchor] = useState<string | null>(null);
  const [anchorSeq, setAnchorSeq] = useState(0);
  const [fetching, setFetching] = useState(false);

  const applySymbol = () => {
    const s = symbolInput.trim().toUpperCase() || "BTCUSDT";
    setSymbolInput(s);
    setSymbol(s);
  };

  /* Real instrument micro-structure (tick/step/fees) for symbol+category —
     defaults stay as fallback when the fetch fails. */
  useEffect(() => {
    let alive = true;
    apiMarkets(market, symbol)
      .then((r) => {
        if (!alive) return;
        const inst = r.instruments.find((i) => i.symbol === symbol) ?? r.instruments[0] ?? null;
        if (inst) {
          setMicro({
            tickSize: String(fracToNum(inst.tick_size) ?? 0.01),
            stepSize: String(fracToNum(inst.step_size) ?? 0.00001),
            makerFee: String(fracToNum(inst.maker_fee_rate) ?? 0.001),
            takerFee: String(fracToNum(inst.taker_fee_rate) ?? 0.001),
          });
          setMicroLive(true);
        }
      })
      .catch(() => {
        if (alive) setMicroLive(false);
      });
    return () => {
      alive = false;
    };
  }, [symbol, market]);

  const fetchMarket = async () => {
    setFetching(true);
    try {
      const r = await apiKlines(symbol, "1h", 1);
      const c = r.bars?.[0]?.c;
      const v = c ? fracToNum(c) : null;
      if (v) {
        setDcaAnchor(String(v));
        setAnchorSeq((n) => n + 1);
      } else {
        toast({ title: "Fiyat alınamadı", description: `${symbol} için kline verisi boş döndü — eski çapa korunuyor.`, variant: "destructive" });
      }
    } catch {
      toast({ title: "Fiyat alınamadı", description: `${symbol} klines isteği başarısız — eski çapa korunuyor.`, variant: "destructive" });
    } finally {
      setFetching(false);
    }
  };

  /* Fetch the live price on mount and on symbol change — anchor follows. */
  useEffect(() => {
    let alive = true;
    setFetching(true);
    apiKlines(symbol, "1h", 1)
      .then((r) => {
        if (!alive) return;
        const c = r.bars?.[0]?.c;
        const v = c ? fracToNum(c) : null;
        if (v) {
          setDcaAnchor(String(v));
          setAnchorSeq((n) => n + 1);
        }
      })
      .catch(() => {
        if (alive) toast({ title: "Fiyat alınamadı", description: `${symbol} klines isteği başarısız — eski çapa korunuyor.`, variant: "destructive" });
      })
      .finally(() => {
        if (alive) setFetching(false);
      });
    return () => {
      alive = false;
    };
  }, [symbol]);

  return (
    <div className="space-y-4">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="flex flex-wrap items-center gap-2 text-xs text-slate-500">
          <TrendingDown className="h-3.5 w-3.5 shrink-0 text-cyan-400" aria-hidden />
          {/* Faz 25 (UIX-31): modül kodları kullanıcıya sızmıyordu — tek cümle. */}
          Sandbox hesaplayıcı — motorla birebir aynı hesap çekirdeği (tam-rasyonel aritmetik).
          {dcaAnchor && (
            <span className="rounded border border-border-strong bg-surface px-2 py-0.5 font-mono text-[10px] text-slate-400">
              {symbol} 1h close: {dcaAnchor}
            </span>
          )}
          <span
            className="rounded border border-border-strong bg-surface px-2 py-0.5 font-mono text-[10px] text-slate-400"
            title={microLive ? `${symbol} (${market}) enstrüman verisi canlı çekildi` : "Enstrüman verisi alınamadı — varsayılan mikro-yapı kullanılıyor"}
          >
            {microLive ? "canlı" : "varsayılan"} tick {micro.tickSize} · step {micro.stepSize} · fee {micro.makerFee}/{micro.takerFee}
          </span>
        </p>
        <div className="flex items-center gap-2">
          <Input
            value={symbolInput}
            onChange={(e) => setSymbolInput(e.target.value.toUpperCase())}
            onBlur={applySymbol}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                applySymbol();
                (e.target as HTMLInputElement).blur();
              }
            }}
            placeholder="BTCUSDT"
            aria-label="Sembol"
            className="num h-9 w-36 border-border-strong bg-surface font-mono text-xs text-slate-100"
          />
          <Button variant="outline" size="sm" className="min-h-9 border-border-strong text-xs text-slate-300 hover:bg-secondary" onClick={() => void fetchMarket()} disabled={fetching}>
            {fetching ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : <RefreshCw className="h-3.5 w-3.5" aria-hidden />}
            Piyasa Fiyatı
          </Button>
        </div>
      </div>

      <Tabs defaultValue="dca">
        <TabsList className="gap-1 border-b border-border bg-transparent pb-1 rounded-none">
          <TabsTrigger value="dca" className="min-h-10 gap-1.5 rounded-md border border-transparent px-4 text-xs text-slate-400 data-[state=active]:border-border-strong data-[state=active]:bg-[#151a24] data-[state=active]:text-slate-100">
            <CalcIcon className="h-3.5 w-3.5" aria-hidden />
            DCA Ladder
          </TabsTrigger>
          <TabsTrigger value="grid" className="min-h-10 gap-1.5 rounded-md border border-transparent px-4 text-xs text-slate-400 data-[state=active]:border-border-strong data-[state=active]:bg-[#151a24] data-[state=active]:text-slate-100">
            <Grid3x3 className="h-3.5 w-3.5" aria-hidden />
            Grid Mesh
          </TabsTrigger>
        </TabsList>
        <TabsContent value="dca" className="mt-4">
          <DcaCalc marketPrice={dcaAnchor} marketPriceSeq={anchorSeq} micro={micro} market={market} symbol={symbol} onMarketChange={setMarket} />
        </TabsContent>
        <TabsContent value="grid" className="mt-4">
          <GridCalc micro={micro} />
        </TabsContent>
      </Tabs>
    </div>
  );
}
