"use client";

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  AlertTriangle,
  Bot,
  ChartCandlestick,
  Check,
  CheckCircle2,
  Clock,
  Grid3x3,
  Info,
  Library,
  Loader2,
  Rocket,
  Save,
  ShieldCheck,
  ShieldAlert,
  ShieldHalf,
  Trash2,
  TriangleAlert,
  Workflow,
} from "lucide-react";
import {
  GateChip,
  GateRow,
  NumField,
  ReviewRow,
  SliderField,
  Stat,
  WizardSection,
  type GateState,
} from "@/components/terminal/wizard/wizard-bits";
import {
  GUARD_ROWS,
  INTERVALS,
  PINE_TEMPLATES,
  PLOT_COLORS,
  POPULAR_SYMBOLS,
  QUICK_COINS,
  SC_MODES,
  SESSION_TYPES,
  STEPS,
  STRATEGIES,
  describeTrigger,
  triggerToWire,
  type GuardKey,
} from "@/components/terminal/wizard/wizard-data";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogDescription, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { LadderTable, MeshTable } from "@/components/terminal/ladder";
import { Panel, PassChip, Spinner } from "@/components/terminal/bits";
import { TvChart, type TvMarker, type TvPlotSeries, type TvPriceLine } from "@/components/terminal/tv-chart";
import { SignalBuilder } from "@/components/terminal/signal-builder";
import {
  apiCalcDca,
  apiCalcGrid,
  apiCreateBot,
  apiDeletePineScript,
  apiKlines,
  apiMarkets,
  apiPatchPineScript,
  apiTickers,
  apiPineBacktest,
  apiPineScripts,
  apiPineValidate,
  apiSavePineScript,
  apiVaultKeys,
  EngineError,
  saveBotMeta,
  type PineBacktestResponse,
  type PineScriptItem,
  type PineValidateResponse,
} from "@/hooks/use-engine";
import { fmtFrac, fmtUsd, fracToNum } from "@/lib/format";
import { computeWizardWarnings } from "@/lib/risk-warnings";
import { cn } from "@/lib/utils";
import { toast } from "@/hooks/use-toast";
import { DEFAULT_RISK_POLICY } from "@/lib/engine-types";
import { PINE_PRESETS, SIGNAL_PRESETS } from "@/lib/bot-defaults";
import type {
  AdvisoryWarning,
  BotConfig,
  BotKind,
  CalcDcaResult,
  CalcGridResult,
  EntryTrigger,
  EntryTriggerMode,
  GuardAction,
  Instrument,
  Interval,
  Mode,
  SignalConfig,
  SignalPreviewResponse,
  Ticker24h,
  UserRiskPolicy,
  Venue,
  VaultKey,
} from "@/lib/engine-types";

/* ------------------------------------------------------------------ */


interface WizardProps {
  onCreated: (id: string) => void;
}

export function Wizard({ onCreated }: WizardProps) {

  /* step 1 */
  const [kind, setKind] = useState<BotKind | null>(null);
  const [cardKey, setCardKey] = useState<string | null>(null);
  /* step 2 */
  const [venue, setVenue] = useState<Venue>("BYBIT");
  const [mode, setMode] = useState<Mode>("PAPER");
  const [symbolQuery, setSymbolQuery] = useState("");
  const [listOpen, setListOpen] = useState(false);
  const [symbol, setSymbol] = useState<string | null>(null);
  const [instrument, setInstrument] = useState<Instrument | null>(null);
  const [intervalSel, setIntervalSel] = useState<Interval>("1h");
  const [instruments, setInstruments] = useState<Instrument[]>([]);
  const [marketsBusy, setMarketsBusy] = useState(false);
  /** P2-1: 24h istatistik haritası (sembol → ticker). Sessizce boş kalabilir. */
  const [tickers, setTickers] = useState<Record<string, Ticker24h>>({});
  const [quoteFilter, setQuoteFilter] = useState<"ALL" | "USDT" | "USDC">("USDT");
  /* step 3 */
  const [baseSize, setBaseSize] = useState("500");
  const [safetySize, setSafetySize] = useState("500");
  const [sizingMode, setSizingMode] = useState<"QUOTE_NOTIONAL" | "BASE_QTY">("QUOTE_NOTIONAL");
  const [deviationPct, setDeviationPct] = useState(2); // percent
  const [stepScale, setStepScale] = useState("1.3");
  const [volumeScale, setVolumeScale] = useState("1.2");
  const [maxSafety, setMaxSafety] = useState(5);
  const [targetProfitPct, setTargetProfitPct] = useState(2); // percent
  const [netTp, setNetTp] = useState(true);
  const [trailing, setTrailing] = useState(false);
  const [trailTrigPct, setTrailTrigPct] = useState(1);
  const [trailDevPct, setTrailDevPct] = useState(0.5);
  const [slEnabled, setSlEnabled] = useState(false);
  const [slPct, setSlPct] = useState(5);
  const [leverage, setLeverage] = useState(1);
  /* step 3 — hedge (M18) real inputs */
  const [hedgeRatio, setHedgeRatio] = useState("0.5");
  const [combinedTp, setCombinedTp] = useState("50");
  /* Task 21 — Hedge Protective koruma tetikleyici (PERCENT, "6" = %6). */
  const [hedgeDdTrigger, setHedgeDdTrigger] = useState("6");
  /* step 3 — start condition */
  const [scMode, setScMode] = useState<EntryTriggerMode>("INSTANT");
  const [scPrice, setScPrice] = useState("");
  const [scRef, setScRef] = useState("");
  const [scDropPct, setScDropPct] = useState(10);
  const [scRsi, setScRsi] = useState(30);
  const [scDateTime, setScDateTime] = useState("");
  const [scMarket, setScMarket] = useState<number | null>(null);
  /* Task 19 — Pine strateji editörü durumu */
  const [pineCode, setPineCode] = useState("");
  const [pineInfo, setPineInfo] = useState<PineValidateResponse | null>(null);
  const [pineErr, setPineErr] = useState<string | null>(null);
  const [pineInputs, setPineInputs] = useState<Record<string, string>>({});
  const [pineBusy, setPineBusy] = useState<"idle" | "validate" | "bt">("idle");
  const [pineBt, setPineBt] = useState<PineBacktestResponse | null>(null);
  const [pineBtErr, setPineBtErr] = useState<string | null>(null);
  /* Task 20 — opsiyonel bot TP/SL (pine çıkışına EK; boş = çıkış yalnız pine'dan) */
  const [pineTp, setPineTp] = useState("");
  const [pineSl, setPineSl] = useState("");
  /* Task 21 — Pine strateji kütüphanesi (kaydet/yükle/sil) */
  const [libOpen, setLibOpen] = useState(false);
  const [libScripts, setLibScripts] = useState<PineScriptItem[] | null>(null);
  const [libBusy, setLibBusy] = useState(false);
  const [libName, setLibName] = useState("");
  const [libLoadedId, setLibLoadedId] = useState<string | null>(null);
  const [libErr, setLibErr] = useState<string | null>(null);
  const [libConfirmDelete, setLibConfirmDelete] = useState<string | null>(null);
  /** 3Commas-style composite signal tree (null = signal engine off). */
  const [signalCfg, setSignalCfg] = useState<SignalConfig | null>(null);
  /* live signal preview sink (fed back by SignalBuilder via onPreviewChange) */
  const [signalPreview, setSignalPreview] = useState<SignalPreviewResponse | null>(null);
  const [signalPreviewError, setSignalPreviewError] = useState<string | null>(null);
  const handleSignalPreview = useCallback((r: SignalPreviewResponse | null, err: string | null) => {
    setSignalPreview(r);
    setSignalPreviewError(err);
  }, []);
  /* step 4 */
  const [startCash, setStartCash] = useState("100000");
  const [anchor, setAnchor] = useState("");
  const [anchorFetched, setAnchorFetched] = useState<number | null>(null);
  const [gridLower, setGridLower] = useState("");
  const [gridUpper, setGridUpper] = useState("");
  const [gridLevels, setGridLevels] = useState(10);
  const [gridMode, setGridMode] = useState<"arithmetic" | "geometric">("arithmetic");
  /* Task 18 N-2 — İz süren grid: fiyat mesh aralığının dışında kapanırsa motor
     mesh'i aralık genişliği kadar kaydırır (eski emirler idempotent iptal,
     yeni aralıkta bootstrap). Infinity Grid'in kendisi zaten iz sürer — bu
     anahtar Spot/Futures Grid için. */
  const [gridTrail, setGridTrail] = useState(false);
  const [gridNotional, setGridNotional] = useState("100");
  /* calc */
  const [calcDca, setCalcDca] = useState<CalcDcaResult | null>(null);
  const [calcGrid, setCalcGrid] = useState<CalcGridResult | null>(null);
  const [calcBusy, setCalcBusy] = useState(false);
  const [calcError, setCalcError] = useState<string | null>(null);
  /* step 5 */
  const [botName, setBotName] = useState("");
  const [launching, setLaunching] = useState(false);
  const [launchError, setLaunchError] = useState<string | null>(null);
  /* step 6 — Risk Motoru (Task 16): advisory policy + user acknowledgment */
  const [riskPolicy, setRiskPolicy] = useState<UserRiskPolicy>(() => ({ ...DEFAULT_RISK_POLICY }));
  const [riskAck, setRiskAck] = useState(false);
  /** Warnings echoed by an engine 422 RISK_ACKNOWLEDGMENT_REQUIRED body. */
  const [serverWarnings, setServerWarnings] = useState<AdvisoryWarning[]>([]);
  /** ÜRÜN-1: LIVE sertifika kapısı KALDI — bilinçli-onay istisnası akışı. */
  const [certGateFailed, setCertGateFailed] = useState<string | null>(null);
  const [certAck, setCertAck] = useState(false);
  /* LIVE vault check */
  const [vaultKeys, setVaultKeys] = useState<VaultKey[] | null>(null);
  const [vaultError, setVaultError] = useState<string | null>(null);

  const isGridKind = kind !== null && kind.includes("GRID");
  const isShort = kind === "DCA_SHORT";
  const isFuturesDca = kind === "FUTURES_DCA";
  const isHedgeKind = kind === "HEDGE_PROTECTIVE" || kind === "HEDGE_DUAL";
  const marketType: "spot" | "linear" = isShort || kind === "FUTURES_GRID" || isFuturesDca || leverage > 1 ? "linear" : "spot";
  const category: "spot" | "linear" = isShort || kind === "FUTURES_GRID" || isFuturesDca ? "linear" : "spot";

  /* ---- markets fetch once strategy kind or venue is selected ---- */
  useEffect(() => {
    if (!kind) return;
    let alive = true;
    setMarketsBusy(true);
    apiMarkets(category, undefined, venue)
      .then((r) => {
        if (alive) setInstruments(r.instruments);
      })
      .catch(() => {
        if (alive) setInstruments([]);
      });
    /* P2-1: 24h ticker'ları ayrı uctan — hata sessiz (liste yine de çalışır). */
    apiTickers(category, venue)
      .then((r) => {
        if (!alive) return;
        const m: Record<string, Ticker24h> = {};
        for (const t of r.tickers ?? []) m[t.symbol] = t;
        setTickers(m);
      })
      .catch(() => {
        if (alive) setTickers({});
      })
      .finally(() => {
        if (alive) setMarketsBusy(false);
      });
    return () => {
      alive = false;
    };
  }, [kind, category, venue]);

  const filteredInstruments = useMemo(() => {
    let list = instruments;
    if (quoteFilter !== "ALL") {
      list = list.filter((i) => i.quote === quoteFilter || i.symbol.endsWith(quoteFilter));
    }
    const q = symbolQuery.trim().toUpperCase();
    if (q) {
      list = list.filter((i) => i.symbol.includes(q) || i.base.includes(q));
    }
    return [...list].sort((a, b) => {
      const aPop = POPULAR_SYMBOLS.indexOf(a.symbol);
      const bPop = POPULAR_SYMBOLS.indexOf(b.symbol);
      if (aPop !== -1 && bPop !== -1) return aPop - bPop;
      if (aPop !== -1) return -1;
      if (bPop !== -1) return 1;
      return a.symbol.localeCompare(b.symbol);
    });
  }, [instruments, symbolQuery, quoteFilter]);

  /* ---- anchor from last kline when symbol set ---- */
  const fetchAnchor = useCallback(async () => {
    if (!symbol) return;
    try {
      const r = await apiKlines(symbol, intervalSel, 1);
      const c = r.bars?.[0]?.c;
      if (c) {
        const v = fracToNum(c) ?? 0;
        setAnchorFetched(v);
        setAnchor((prev) => (prev === "" || prev === "0" ? String(v) : prev));
      }
    } catch {
      setAnchorFetched(null);
    }
  }, [symbol, intervalSel]);

  useEffect(() => {
    void fetchAnchor();
    // reset grid bounds when symbol changes
    setGridLower("");
    setGridUpper("");
  }, [symbol, intervalSel]);

  /* ---- grid bounds default around anchor ---- */
  useEffect(() => {
    if (isGridKind && anchorFetched) {
      setGridLower((p) => (p === "" ? String(Number((anchorFetched * 0.9).toPrecision(8))) : p));
      setGridUpper((p) => (p === "" ? String(Number((anchorFetched * 1.1).toPrecision(8))) : p));
    }
  }, [isGridKind, anchorFetched]);

  /* ---- live calc (debounced) ---- */
  const calcTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  useEffect(() => {
    if (!kind) return;
    if (calcTimer.current) clearTimeout(calcTimer.current);
    setCalcBusy(true);
    calcTimer.current = setTimeout(async () => {
      try {
        setCalcError(null);
        if (!isGridKind) {
          const a = fracToNum(anchor);
          if (!a || a <= 0) {
            setCalcDca(null);
            setCalcError("Geçerli bir çapa (anchor) fiyatı gerekli.");
            return;
          }
          const r = await apiCalcDca({
            anchor: String(a),
            base_order_size: baseSize || "0",
            safety_order_size: safetySize || "0",
            sizing_mode: sizingMode,
            deviation_pct: String(deviationPct / 100),
            step_scale: stepScale || "1",
            volume_scale: volumeScale || "1",
            max_safety_orders: maxSafety,
            target_profit_pct: String(targetProfitPct / 100),
            use_net_tp: netTp,
            stop_loss_pct: slEnabled ? String(slPct / 100) : null,
            tick_size: instrument ? String(fracToNum(instrument.tick_size) ?? 0.01) : "0.01",
            step_size: instrument ? String(fracToNum(instrument.step_size) ?? 0.00001) : "0.00001",
            maker_fee: instrument ? String(fracToNum(instrument.maker_fee_rate) ?? 0.001) : "0.001",
            taker_fee: instrument ? String(fracToNum(instrument.taker_fee_rate) ?? 0.001) : "0.001",
            leverage: String(leverage),
            market: marketType,
          });
          setCalcDca(r);
          setCalcGrid(null);
        } else {
          const lo = fracToNum(gridLower);
          const hi = fracToNum(gridUpper);
          if (!lo || !hi || lo >= hi) {
            setCalcGrid(null);
            setCalcError("Grid alt sınır < üst sınır olmalı.");
            return;
          }
          const r = await apiCalcGrid({
            lower: String(lo),
            upper: String(hi),
            levels: gridLevels,
            grid_mode: gridMode,
            notional_per_level: gridNotional || "0",
            tick_size: instrument ? String(fracToNum(instrument.tick_size) ?? 0.01) : "0.01",
            step_size: instrument ? String(fracToNum(instrument.step_size) ?? 0.00001) : "0.00001",
            maker_fee: instrument ? String(fracToNum(instrument.maker_fee_rate) ?? 0.001) : "0.001",
            taker_fee: instrument ? String(fracToNum(instrument.taker_fee_rate) ?? 0.001) : "0.001",
          });
          setCalcGrid(r);
          setCalcDca(null);
        }
      } catch (e) {
        setCalcDca(null);
        setCalcGrid(null);
        setCalcError(e instanceof Error ? e.message : "Hesaplama hatası");
      } finally {
        setCalcBusy(false);
      }
    }, 500);
    return () => {
      if (calcTimer.current) clearTimeout(calcTimer.current);
    };
  }, [kind, anchor, baseSize, safetySize, sizingMode, deviationPct, stepScale, volumeScale, maxSafety, targetProfitPct, netTp, slEnabled, slPct, leverage, isGridKind, gridLower, gridUpper, gridLevels, gridMode, gridTrail, gridNotional, instrument, marketType]);

  /* ---- start condition: live market price for on-screen evaluation ---- */
  useEffect(() => {
    if (!symbol || scMode === "INSTANT" || scMode === "PINE") return;
    let alive = true;
    apiKlines(symbol, intervalSel, 1)
      .then((r) => {
        if (!alive) return;
        const c = r.bars?.[0]?.c;
        const v = c ? fracToNum(c) : null;
        setScMarket(v);
        // prefill thresholds with the live price on first load
        setScPrice((p) => (p === "" && v ? String(Number(v.toPrecision(8))) : p));
        setScRef((p) => (p === "" && v ? String(Number(v.toPrecision(8))) : p));
      })
      .catch(() => {
        if (alive) setScMarket(null);
      });
    return () => {
      alive = false;
    };
  }, [symbol, intervalSel, scMode]);

  /* ---- start condition validation (declared before canNext) ---- */
  const triggerValid = useMemo(() => {
    switch (scMode) {
      case "INSTANT":
        return true;
      case "PINE":
        return pineInfo?.ok === true && pineCode.trim().length > 0;
      case "PRICE_BELOW":
      case "PRICE_ABOVE":
        return Number(scPrice) > 0;
      case "DROP_FROM_REF":
        return Number(scRef) > 0 && scDropPct > 0 && scDropPct <= 90;
      case "RSI_BELOW":
        return scRsi >= 1 && scRsi <= 99;
      case "SCHEDULED":
        return !Number.isNaN(Date.parse(scDateTime)) && Date.parse(scDateTime) > 0;
    }
  }, [scMode, scPrice, scRef, scDropPct, scRsi, scDateTime, pineInfo, pineCode]);

  /* ---- per-section validity (3Commas single-page flow) ---- */
  const sec1Valid = kind !== null;
  const sec2Valid = symbol !== null && instrument !== null;
  /* Hedge contract (backend 15-a): hedge_ratio ∈ (0,1], combined_target_profit ∈ [1,200].
     Task 21: koruma tetikleyici (HEDGE_PROTECTIVE) (0, 100] — motor 422 kapısıyla aynı kural. */
  const hedgeValid =
    !isHedgeKind ||
    (Number(hedgeRatio) >= 0.05 &&
      Number(hedgeRatio) <= 1 &&
      Number(combinedTp) >= 1 &&
      Number(combinedTp) <= 200 &&
      (kind !== "HEDGE_PROTECTIVE" || (Number(hedgeDdTrigger) > 0 && Number(hedgeDdTrigger) <= 100)));
  /* Task 16: SL ≤ deviation artık engellemez — M20 advisory uyarısı üretir
     (Bölüm 6'da onay kutusu). Sadece sayısal geçerlilik burada tutulur. */
  const sec3Valid =
    (Number(baseSize) > 0 || isGridKind) &&
    deviationPct > 0 &&
    targetProfitPct > 0 &&
    hedgeValid &&
    triggerValid;
  const sec4Valid =
    // Sermaye her zaman gerçek ve pozitif olmalı — boş girdi sessizce 100000'e
    // düşmez; BOTU BAŞLAT kilitli kalır (Gate 6 dürüstlük kuralı).
    Number.isFinite(Number(startCash)) &&
    Number(startCash) > 0 &&
    (isGridKind
      ? (fracToNum(gridLower) ?? 0) > 0 &&
        (fracToNum(gridUpper) ?? 0) > (fracToNum(gridLower) ?? 0) &&
        gridLevels >= 2 &&
        Number(gridNotional) > 0 &&
        !!calcGrid?.feeBreakerPass
      : (fracToNum(anchor) ?? 0) > 0 && !!calcDca);
  const sec5Valid = botName.trim().length > 0;

  const m20Warn = slEnabled && slPct <= deviationPct;

  /* Task 21 — pine plot serileri + fill işaretleri (plotları ana grafiğe çizmek için).
     Plot değerleri backtest bar DİZİNİNE hizalıdır; sunucu aynı pencereyi "bars" alanında döner. */
  const pinePlotSeries = useMemo<TvPlotSeries[]>(() => {
    if (!pineBt?.pinePlots) return [];
    return Object.entries(pineBt.pinePlots).map(([title, series], i) => ({
      title,
      color: PLOT_COLORS[i % PLOT_COLORS.length],
      values: series.map((s) => (s === "na" ? null : Number(s))),
    }));
  }, [pineBt]);
  const pineMarkers = useMemo<TvMarker[] | undefined>(() => {
    if (!pineBt?.fillMarks) return undefined;
    return pineBt.fillMarks.map((m) => ({ time: m.t, side: m.side }));
  }, [pineBt]);

  /* Task 21 — Pine kitaplık işlemleri (kaydet/yükle/sil — motor pine_scripts tablosu). */
  const refreshLib = useCallback(async () => {
    try {
      const r = await apiPineScripts();
      setLibScripts(r.scripts);
    } catch (e) {
      setLibErr(e instanceof EngineError ? (e.body as { error?: string })?.error || e.message : String(e));
    }
  }, []);

  const openLib = useCallback(() => {
    setLibOpen(true);
    setLibErr(null);
    setLibConfirmDelete(null);
    void refreshLib();
  }, [refreshLib]);

  const loadFromLib = useCallback((s: PineScriptItem) => {
    setPineCode(s.code);
    setPineInputs({ ...(s.inputs ?? {}) });
    setPineInfo(null);
    setPineErr(null);
    setPineBt(null);
    setPineBtErr(null);
    setLibName(s.name);
    setLibLoadedId(s.id);
    setLibOpen(false);
    toast({ title: "Kitaplıktan yüklendi", description: `${s.name} — doğrulayıp backtest alabilirsiniz.` });
  }, []);

  const saveToLib = useCallback(async () => {
    if (!pineCode.trim()) return;
    const name = libName.trim() || `Strateji ${new Date().toLocaleDateString("tr-TR")}`;
    setLibBusy(true);
    setLibErr(null);
    try {
      const r = await apiSavePineScript({
        name,
        code: pineCode,
        inputs: Object.fromEntries(Object.entries(pineInputs).filter(([, v]) => v.trim() !== "")),
      });
      setLibLoadedId(r.id);
      setLibName(name);
      await refreshLib();
      toast({ title: "Kitaplığa kaydedildi", description: `${name} derlendi ve saklandı.` });
    } catch (e) {
      setLibErr(e instanceof EngineError ? (e.body as { error?: string })?.error || e.message : String(e));
    } finally {
      setLibBusy(false);
    }
  }, [pineCode, pineInputs, libName, refreshLib]);

  const updateInLib = useCallback(async () => {
    if (!libLoadedId || !pineCode.trim()) return;
    setLibBusy(true);
    setLibErr(null);
    try {
      await apiPatchPineScript(libLoadedId, {
        name: libName.trim() || undefined,
        code: pineCode,
        inputs: Object.fromEntries(Object.entries(pineInputs).filter(([, v]) => v.trim() !== "")),
      });
      await refreshLib();
      toast({ title: "Kitaplık kaydı güncellendi", description: libName.trim() });
    } catch (e) {
      setLibErr(e instanceof EngineError ? (e.body as { error?: string })?.error || e.message : String(e));
    } finally {
      setLibBusy(false);
    }
  }, [libLoadedId, pineCode, pineInputs, libName, refreshLib]);

  const deleteFromLib = useCallback(
    async (id: string) => {
      setLibBusy(true);
      try {
        await apiDeletePineScript(id);
        if (libLoadedId === id) setLibLoadedId(null);
        await refreshLib();
        toast({ title: "Kayıt silindi" });
      } catch (e) {
        setLibErr(e instanceof EngineError ? (e.body as { error?: string })?.error || e.message : String(e));
      } finally {
        setLibBusy(false);
        setLibConfirmDelete(null);
      }
    },
    [libLoadedId, refreshLib],
  );

  /* ---- Task 16: Risk Motoru — live advisory warnings + acknowledgment gate ---- */

  /** GuardAction değişince motorun 422 uyarı anlık görüntüsü geçersizleşir. */
  const setGuard = useCallback((key: GuardKey, value: GuardAction) => {
    setRiskPolicy((p) => {
      const next = { ...p };
      next[key] = value;
      return next;
    });
    setServerWarnings([]);
  }, []);

  const patchRiskPolicy = useCallback((patch: Partial<UserRiskPolicy>) => {
    setRiskPolicy((p) => ({ ...p, ...patch }));
    setServerWarnings([]);
  }, []);

  /* active_safety_orders her zaman 1..max(max_safety_orders, 1) aralığına kelepirlenir. */
  useEffect(() => {
    setRiskPolicy((p) => {
      const maxSo = Math.max(maxSafety, 1);
      const clamped = Math.min(Math.max(1, Math.round(p.active_safety_orders ?? 1)), maxSo);
      return clamped === p.active_safety_orders ? p : { ...p, active_safety_orders: clamped };
    });
  }, [maxSafety]);

  const wizardWarnings = useMemo(
    () =>
      computeWizardWarnings({
        kind: kind ?? "DCA_LONG",
        stop_loss_pct: slEnabled ? String(slPct / 100) : null,
        max_safety_orders: maxSafety,
        risk_policy: riskPolicy,
        m20: !isGridKind && calcDca ? calcDca.m20 : undefined,
        feeBreakerPass: isGridKind ? calcGrid?.feeBreakerPass : undefined,
      }),
    [kind, slEnabled, slPct, maxSafety, riskPolicy, isGridKind, calcDca, calcGrid],
  );

  /* Motor 422 uyarı anlık görüntüsü canlı listeyle kod bazında birleştirilir. */
  const displayWarnings = useMemo(() => {
    const byCode = new Map<string, AdvisoryWarning>();
    for (const w of [...wizardWarnings, ...serverWarnings]) {
      if (!byCode.has(w.code)) byCode.set(w.code, w);
    }
    return [...byCode.values()];
  }, [wizardWarnings, serverWarnings]);

  const needsAck = displayWarnings.length > 0;
  const sec6Valid = !needsAck || riskAck;
  const canLaunch = sec1Valid && sec2Valid && sec3Valid && sec4Valid && sec5Valid && sec6Valid;
  const secValid = [sec1Valid, sec2Valid, sec3Valid, sec4Valid, sec5Valid, sec6Valid];

  /* ---- LIVE mode: verify a vault key exists for the selected venue ---- */
  useEffect(() => {
    if (mode !== "LIVE" || !venue) {
      setVaultKeys(null);
      setVaultError(null);
      return;
    }
    let alive = true;
    apiVaultKeys()
      .then((r) => {
        if (!alive) return;
        setVaultKeys(r.keys ?? []);
        setVaultError(null);
      })
      .catch((e) => {
        if (!alive) return;
        setVaultKeys(null);
        setVaultError(e instanceof Error ? e.message : "vault okunamadı");
      });
    return () => {
      alive = false;
    };
  }, [mode, venue]);

  const vaultVenueOk =
    mode === "LIVE" && vaultKeys !== null
      ? vaultKeys.some((k) => k.venue.toUpperCase() === venue.toUpperCase())
      : null;

  const scMetNow = useMemo<boolean | null>(() => {
    if (scMode === "INSTANT") return null;
    if (scMode === "RSI_BELOW") return null; // bar kapanışında hesaplanır
    if (scMode === "PINE") return null; // pine kararı bar kapanışında motor'da üretilir
    if (scMode === "SCHEDULED") {
      const ms = Date.parse(scDateTime);
      return Number.isNaN(ms) ? null : Date.now() >= ms;
    }
    if (scMarket === null) return null;
    if (scMode === "PRICE_BELOW") return Number(scPrice) > 0 ? scMarket <= Number(scPrice) : null;
    if (scMode === "PRICE_ABOVE") return Number(scPrice) > 0 ? scMarket >= Number(scPrice) : null;
    if (scMode === "DROP_FROM_REF")
      return Number(scRef) > 0 ? scMarket <= Number(scRef) * (1 - scDropPct / 100) : null;
    return null;
  }, [scMode, scPrice, scRef, scDropPct, scDateTime, scMarket]);

  /* ---- real certifier chips: FEE GUARD + CAPITAL G6 (live ladder math) ---- */
  const guard = useMemo(() => {
    const feeRate = instrument
      ? Math.max(fracToNum(instrument.maker_fee_rate) ?? 0, fracToNum(instrument.taker_fee_rate) ?? 0)
      : null;
    const cash = fracToNum(startCash);

    if (isGridKind) {
      const invested = fracToNum(calcGrid?.totalInvested ?? null);
      const feeGuardState: GateState = calcGrid ? (calcGrid.feeBreakerPass ? "pass" : "fail") : "wait";
      const feeDetail = calcGrid
        ? calcGrid.feeBreakerPass
          ? "motor fee-breaker: seviye spread'i maker ücretini karşılıyor"
          : "motor fee-breaker: spread maker ücretini karşılamıyor"
        : "calc/grid bekleniyor";
      const capitalState: GateState =
        cash !== null && cash > 0 ? (invested === null ? "wait" : invested <= cash ? "pass" : "fail") : "wait";
      const capitalDetail =
        cash === null || cash <= 0
          ? "başlangıç sermayesi girilmemiş"
          : invested === null
            ? "toplam yatırım bekleniyor"
            : `toplam yatırım ${fmtUsd(invested)} ${invested <= cash ? "≤" : ">"} nakit ${fmtUsd(cash)}`;
      return { feeGuardState, feeDetail, capitalState, capitalDetail };
    }

    const total = fracToNum(calcDca?.totalNotional ?? null);
    let feeGuardState: GateState = "wait";
    let feeDetail: string;
    if (calcBusy) {
      feeDetail = "calc/dca çalışıyor…";
    } else if (!total || total <= 0 || !feeRate) {
      feeDetail = "merdiven hesabı veya enstrüman ücreti yok";
    } else {
      const profit = total * (targetProfitPct / 100);
      const feeEst = total * feeRate * 2;
      feeGuardState = profit > feeEst ? "pass" : "fail";
      feeDetail = `beklenen kâr ${fmtUsd(profit)} vs tahmini fee×2 ${fmtUsd(feeEst)} (fee ${(feeRate * 100).toFixed(3)}% × 2 güvenlik çarpanı)`;
    }

    let capitalState: GateState = "wait";
    let capitalDetail: string;
    if (cash === null || cash <= 0) {
      capitalDetail = "başlangıç sermayesi girilmemiş";
    } else if (total === null) {
      capitalDetail = "toplam emir maliyeti bekleniyor";
    } else {
      capitalState = total <= cash ? "pass" : "fail";
      capitalDetail = `toplam emir maliyeti (base+safety) ${fmtUsd(total)} ${total <= cash ? "≤" : ">"} nakit ${fmtUsd(cash)}`;
    }
    return { feeGuardState, feeDetail, capitalState, capitalDetail };
  }, [isGridKind, calcDca, calcGrid, calcBusy, instrument, targetProfitPct, startCash]);

  const buildConfig = (): BotConfig => ({
    kind: (kind ?? "DCA_LONG") as BotKind,
    symbol: symbol ?? "BTCUSDT",
    base_order_size: baseSize || "0",
    safety_order_size: safetySize || "0",
    sizing_mode: sizingMode,
    deviation_pct: String(deviationPct / 100),
    step_scale: stepScale || "1",
    volume_scale: volumeScale || "1",
    max_safety_orders: maxSafety,
    target_profit_pct: String(targetProfitPct / 100),
    use_net_tp: netTp,
    trailing_enabled: trailing,
    trailing_trigger_pct: String(trailTrigPct / 100),
    trailing_deviation_pct: String(trailDevPct / 100),
    stop_loss_pct: slEnabled ? String(slPct / 100) : null,
    leverage: String(leverage),
    hedge_ratio: hedgeRatio || "0.5",
    combined_target_profit: combinedTp || "50",
    hedge_drawdown_trigger_pct: hedgeDdTrigger || "6",
    grid_lower: isGridKind ? (gridLower || null) : null,
    grid_upper: isGridKind ? (gridUpper || null) : null,
    grid_levels: isGridKind ? gridLevels : null,
    grid_mode: gridMode,
    grid_trail: gridTrail,
    cooldown_bars: 0,
    entry_trigger: triggerToWire(scMode, scPrice, scRef, scDropPct, scRsi, scDateTime),
    entry_signal: signalCfg,
    pine_script: scMode === "PINE" && pineCode.trim() ? pineCode : null,
    pine_inputs:
      scMode === "PINE"
        ? Object.fromEntries(Object.entries(pineInputs).filter(([, v]) => v.trim() !== ""))
        : {},
    risk_policy: riskPolicy,
  });

  const launch = async () => {
    if (!kind || !symbol) return;
    setLaunching(true);
    setLaunchError(null);
    try {
      const cfg = buildConfig();
      /* Task 16: uyarı yoksa onay gereksiz (true gönderilir); uyarı varsa kutu işaretliyse true,
         işaretsizse false → motor 422 RISK_ACKNOWLEDGMENT_REQUIRED ile uyarı listesini döner. */
      const res = await apiCreateBot(
        {
          name: botName.trim(),
          kind,
          venue,
          mode,
          symbol,
          interval: intervalSel,
          // Task 18 Ş3: ölü "|| 100000" fallback'i kaldırıldı — sec4Valid
          // (start_cash > 0) launch öncesi bu alanı zaten zorunlu kılar.
          start_cash: startCash,
          config: cfg,
        },
        needsAck ? riskAck : true,
        certGateFailed && certAck ? true : undefined,
      );
      saveBotMeta(res.id, { deviation_pct: cfg.deviation_pct });
      try {
        window.sessionStorage.setItem(`dcabot.cfg.${res.id}`, JSON.stringify(cfg));
      } catch {
        /* ignore */
      }
      setServerWarnings([]);
      toast({ title: "Bot başlatıldı", description: `${botName.trim()} → RUNNING (201)` });
      onCreated(res.id);
    } catch (e) {
      /* Task 16: RISK_ACKNOWLEDGMENT_REQUIRED özel akışı — kırmızı alert yerine
         uyarı paneli + onay kutusu; diğer 422'ler eski kırmızı akışta kalır. */
      if (e instanceof EngineError && e.status === 422) {
        const body = (e.body ?? {}) as { code?: string; warnings?: AdvisoryWarning[]; error?: string };
        if (body.code === "RISK_ACKNOWLEDGMENT_REQUIRED") {
          setServerWarnings(Array.isArray(body.warnings) ? body.warnings : []);
          setRiskAck(false);
          toast({
            title: "Risk onayı gerekli",
            description: "Motor uyarıları onaylanmadan launch kabul edilmiyor — listeyi okuyup onay kutusunu işaretleyin.",
          });
          return;
        }
        if (body.code === "CERT_GATE_FAILED") {
          /* ÜRÜN-1: sertifika kapısı KALDI — bilinçli-onay istisnası sunulur
             (motor onayı kalıcı WARN olayına yazar). Kullanıcı istemeyebilir:
             kutu işaretsizse launch tekrarında yine 422 döner. */
          setCertGateFailed(typeof body.error === "string" ? body.error : "LIVE sertifika kapısı KALDI");
          setCertAck(false);
          toast({
            title: "LIVE sertifika kapısı: KALDI",
            description: "Strateji sertifikasyon eşiklerini geçemedi — istisna için bilinçli onay kutusunu işaretleyin veya Lab'de config'inizi güçlendirin.",
          });
          return;
        }
      }
      setLaunchError(e instanceof Error ? e.message : "Bilinmeyen hata");
      toast({ title: "Launch reddedildi (422)", description: e instanceof Error ? e.message : "", variant: "destructive" });
    } finally {
      setLaunching(false);
    }
  };

  /* ---------------------------------------------------------------- */

  /* ---- real gate checklist rows (section 5) ---- */
  const vaultKeysForVenue = (vaultKeys ?? []).filter((k) => k.venue.toUpperCase() === venue.toUpperCase());
  const gateRows: { code: string; desc: string; state: GateState; note: string }[] = [
    {
      code: "INS",
      desc: "Enstrüman verisi çekildi (tick/step/fee)",
      state: instrument ? "pass" : marketsBusy ? "wait" : "fail",
      note: instrument
        ? `${instrument.symbol} · tick ${fmtFrac(instrument.tick_size)} · taker ${fmtFrac(instrument.taker_fee_rate, 4)}${tickers[instrument.symbol] ? ` · 24s %${tickers[instrument.symbol].change_pct_24h}` : ""}`
        : marketsBusy
          ? "markets fetch sürüyor…"
          : "enstrüman seçilmedi veya fetch başarısız",
    },
    {
      code: "TICK",
      desc: "Hesap çağrısı (calc/dca · calc/grid) başarılı",
      state: calcBusy ? "wait" : calcError ? "fail" : calcDca || calcGrid ? "pass" : "wait",
      note: calcBusy ? "hesaplanıyor…" : (calcError ?? (calcDca || calcGrid ? "motor merdiven/mesh üretti" : "hesap bekleniyor")),
    },
    {
      code: "G6",
      desc: "Sermaye — toplam emir maliyeti ≤ kullanılabilir nakit",
      state: guard.capitalState,
      note: guard.capitalDetail,
    },
    {
      code: "FEE",
      desc: isGridKind ? "Grid fee-guard (motor fee-breaker)" : "Fee guard — beklenen kâr > tahmini fee × 2",
      state: guard.feeGuardState,
      note: guard.feeDetail,
    },
    {
      code: "KOŞUL",
      desc: "Başlangıç koşulu geçerli (yerel validasyon)",
      state: triggerValid ? "pass" : "fail",
      note:
        describeTrigger(scMode, scPrice, scRef, scDropPct, scRsi, scDateTime) +
        (scMode === "PINE"
          ? pineInfo?.ok
            ? ` · derlendi (${pineInfo.functions.length} fonksiyon, ${pineInfo.inputs.length} input)`
            : " · kod derlenmedi — Doğrula düğmesine basın"
          : scMetNow === true
            ? " · şu an SAĞLANIR"
            : scMetNow === false
              ? " · şu an SAĞLANMAZ"
              : scMode === "RSI_BELOW"
                ? " · bar kapanışında değerlendirilir"
                : ""),
    },
    {
      code: "SİNYAL",
      desc: "Sinyal ağacı (entry_signal)",
      state: !signalCfg
        ? "pass"
        : signalPreviewError
          ? "fail"
          : signalPreview
            ? signalPreview.met
              ? "pass"
              : "wait"
            : "wait",
      note: !signalCfg
        ? "PASS — ASAP (sinyal motoru kapalı)"
        : signalPreviewError
          ? `önizleme reddi: ${signalPreviewError}`
          : signalPreview
            ? signalPreview.met
              ? "ağaç şu an sağlanıyor (canlı önizleme)"
              : "ağaç sağlanmasını bekliyor (canlı önizleme)"
            : "önizleme değerleniyor…",
    },
    {
      code: "VAULT",
      desc: "LIVE anahtarı kasada mevcut",
      state:
        mode === "PAPER"
          ? "na"
          : vaultError
            ? "wait"
            : vaultKeys === null
              ? "wait"
              : vaultVenueOk
                ? "pass"
                : "fail",
      note:
        mode === "PAPER"
          ? "N/A — PAPER modunda vault anahtarı gerekmez"
          : vaultError
            ? `vault okunamadı: ${vaultError}`
            : vaultKeys === null
              ? "vault sorgulanıyor…"
              : vaultVenueOk
                ? `${vaultKeysForVenue.length} anahtar hazır (${vaultKeysForVenue.map((k) => k.label).join(", ")})`
                : "VAULT ANAHTARI EKSİK — Ayarlar > API Key Vault'tan bu venue için anahtar ekleyin",
    },
    {
      code: "MESH",
      desc: "Grid mesh ladder üretildi",
      state: isGridKind ? ((calcGrid?.mesh.length ?? 0) > 0 ? "pass" : "wait") : "na",
      note: isGridKind
        ? (calcGrid?.mesh.length ?? 0) > 0
          ? `${calcGrid?.mesh.length} mesh seviyesi üretildi`
          : "mesh bekleniyor — aralık/notional girin"
        : "N/A — DCA stratejilerinde mesh yok",
    },
  ];

  /* ---- chart overlay: planned levels on the live TradingView chart ---- */
  const chartLines = useMemo<TvPriceLine[]>(() => {
    const lines: TvPriceLine[] = [];
    if (isGridKind) {
      for (const m of calcGrid?.mesh ?? []) {
        const p = fracToNum(m.priceDisplay ?? m.price) ?? 0;
        if (p > 0) lines.push({ price: p, color: "#38bdf8", title: `L${m.level}`, dashed: true });
      }
      return lines.slice(0, 16);
    }
    for (const l of (calcDca?.levels ?? []).slice(0, 9)) {
      const p = fracToNum(l.priceDisplay ?? l.price) ?? 0;
      if (p > 0)
        lines.push({
          price: p,
          color: l.level === 0 ? "#10b981" : "#f59e0b",
          title: l.level === 0 ? "BASE" : `S${l.level}`,
          dashed: l.level > 0,
        });
    }
    const avg = fracToNum(calcDca?.avgAllIn ?? null);
    if (avg && avg > 0) lines.push({ price: avg, color: "#22d3ee", title: "AVG", dashed: false });
    return lines;
  }, [isGridKind, calcDca, calcGrid]);

  return (
    <div className="grid grid-cols-1 gap-4 lg:grid-cols-[minmax(0,1fr)_400px] xl:grid-cols-[minmax(0,1fr)_440px]">
      {/* ============ LEFT: 3Commas-style single-column sections ============ */}
      <div className="min-w-0 space-y-4">
        <nav className="flex flex-wrap items-center gap-1.5" aria-label="Bölüm gezinmesi">
          {STEPS.map((label, i) => {
            const n = i + 1;
            const ok = secValid[i];
            return (
              <button
                key={label}
                onClick={() => document.getElementById(`wz-sec-${n}`)?.scrollIntoView({ behavior: "smooth", block: "start" })}
                className={cn(
                  "flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-[11px] font-semibold transition-colors",
                  ok
                    ? "border-emerald-500/50 bg-emerald-500/10 text-emerald-300"
                    : "border-border bg-surface text-slate-500 hover:border-border-strong",
                )}
              >
                <span
                  className={cn(
                    "flex h-4 w-4 items-center justify-center rounded-full text-[9px]",
                    ok ? "bg-emerald-500 text-[#04150e]" : "bg-secondary text-slate-500",
                  )}
                >
                  {ok ? <Check className="h-2.5 w-2.5" aria-hidden /> : n}
                </span>
                {label}
              </button>
            );
          })}
        </nav>

        {/* ---------------- SECTION 1: strategy ---------------- */}
        <WizardSection n={1} title="Strateji" valid={sec1Valid}>
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2 xl:grid-cols-3">
            {STRATEGIES.map((s) => {
              const Icon = s.icon;
              const sel = s.key === cardKey;
              const tone = {
                emerald: "hover:border-emerald-500/50",
                rose: "hover:border-rose-500/50",
                cyan: "hover:border-cyan-500/50",
                amber: "hover:border-amber-500/50",
              }[s.color] ?? "hover:border-border-strong";
              return (
                <button
                  key={s.key}
                  onClick={() => {
                    setKind(s.kind);
                    setCardKey(s.key);
                    if (s.kind === "FUTURES_DCA") setLeverage(5);
                    else if (s.presetLeverage) setLeverage(s.presetLeverage);
                    // Signal Bot havuzu: sinyal agaci on-yukle (yoksa); Pine sablonu oner (uzerine yazma).
                    if (s.signalPreset !== undefined && !signalCfg) {
                      const preset = SIGNAL_PRESETS[s.signalPreset];
                      if (preset) {
                        setSignalCfg(structuredClone(preset.tree));
                        toast({ title: `${s.name} sinyali yuklendi`, description: `${preset.name} (${preset.regime}) — Parametreler & Kosul'da duzenleyebilirsiniz.` });
                      }
                    }
                    if (s.pinePreset !== undefined && !pineCode.trim()) {
                      const pine = PINE_PRESETS[s.pinePreset];
                      if (pine) setPineCode(pine.code);
                    }
                  }}
                  aria-pressed={sel}
                  className={cn(
                    "rounded-xl border p-4 text-left transition-all",
                    sel ? "border-emerald-500/60 bg-emerald-500/5 glow-emerald" : cn("border-border bg-[#151a24]", tone),
                  )}
                >
                  <div className="flex items-center justify-between">
                    <Icon
                      className={cn(
                        "h-6 w-6",
                        s.color === "emerald" ? "text-emerald-400" : s.color === "rose" ? "text-rose-400" : s.color === "cyan" ? "text-cyan-400" : "text-amber-400",
                      )}
                      aria-hidden
                    />
                    {sel && <CheckCircle2 className="h-5 w-5 text-emerald-400" aria-hidden />}
                  </div>
                  <div className="mt-3 text-sm font-bold text-slate-100">{s.name}</div>
                  <p className="mt-1 text-xs leading-relaxed text-slate-500">{s.desc}</p>
                  <div className="num mt-2 text-[10px] text-slate-400">{s.signalPool ? `SIGNAL HAVUZU · motor: ${s.kind}` : s.kind}</div>
                </button>
              );
            })}
          </div>
        </WizardSection>

        {/* ---------------- SECTION 2: venue + symbol ---------------- */}
        <WizardSection n={2} title="Borsa & Sembol" valid={sec2Valid} hidden={!kind}>
          <div className="space-y-4">
            <div className="grid gap-4 lg:grid-cols-2">
              <Panel title="Borsa (Venue)" icon={<ShieldCheck className="h-4 w-4 text-slate-500" aria-hidden />}>
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-4">
                  {(["BYBIT", "BINANCE", "OKX", "PAPER"] as Venue[]).map((v) => (
                    <button
                      key={v}
                      onClick={() => {
                        setVenue(v);
                        if (v === "PAPER" && mode === "LIVE") {
                          setMode("PAPER");
                          toast({ title: "Mod PAPER'a çekildi", description: "PAPER venue ile LIVE mod kullanılamaz." });
                        }
                      }}
                      aria-pressed={venue === v}
                      className={cn(
                        "min-h-11 rounded-lg border px-2 py-2 text-xs font-bold transition-colors",
                        venue === v
                          ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300"
                          : "border-border bg-surface text-slate-400 hover:border-border-strong",
                      )}
                    >
                      {v}
                    </button>
                  ))}
                </div>
                <div className="mt-4 flex items-center gap-2">
                  <button
                    onClick={() => setMode("PAPER")}
                    className={cn(
                      "flex-1 rounded-lg border px-3 py-2.5 text-xs font-bold transition-colors",
                      mode === "PAPER" ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300" : "border-border text-slate-500",
                    )}
                  >
                    PAPER ✓
                  </button>
                  <button
                    onClick={() => {
                      if (venue === "PAPER") {
                        toast({
                          title: "LIVE seçilemez",
                          description: "PAPER venue ile LIVE mod kullanılamaz — önce gerçek bir borsa (BYBIT/BINANCE/OKX) seçin.",
                          variant: "destructive",
                        });
                        return;
                      }
                      setMode("LIVE");
                    }}
                    disabled={venue === "PAPER"}
                    aria-disabled={venue === "PAPER"}
                    className={cn(
                      "flex-1 rounded-lg border px-3 py-2.5 text-xs font-bold transition-colors",
                      mode === "LIVE"
                        ? "border-amber-500/60 bg-amber-500/10 text-amber-300"
                        : "border-border text-slate-500",
                      venue === "PAPER" && "cursor-not-allowed opacity-40",
                    )}
                    title="LIVE — gercek borsa emri (3Commas: Real)"
                  >
                    REAL ⚠
                  </button>
                </div>
                {venue === "PAPER" && (
                  <p className="mt-1.5 text-[11px] text-amber-400">PAPER venue ile LIVE mod kullanılamaz — simülasyon hesabı gerçek emir göndermez.</p>
                )}
                <p className="mt-2 text-[11px] text-slate-400">
                  {mode === "PAPER"
                    ? "Kağıt modu: emirler simülasyon çekirdeğinde doldurulur (Gate 5 realizm etiketleriyle)."
                    : "LIVE mod: gerçek emirler gönderilir. Vault'ta doğrulanmış anahtar gerekir — dikkatli olun."}
                </p>
              </Panel>

              <Panel title="Sembol & Periyot" icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />}>
                <div className="flex items-center justify-between">
                  <Label className="text-xs text-slate-400">Enstrüman ara</Label>
                  <div className="flex items-center gap-1">
                    {(["USDT", "USDC", "ALL"] as const).map((q) => (
                      <button
                        key={q}
                        type="button"
                        onClick={() => setQuoteFilter(q)}
                        className={cn(
                          "rounded px-2 py-0.5 text-[10px] font-mono font-medium transition-colors",
                          quoteFilter === q
                            ? "border border-cyan-500/50 bg-cyan-500/20 text-cyan-300"
                            : "border border-border-strong bg-surface text-slate-400 hover:text-slate-200",
                        )}
                      >
                        {q === "ALL" ? "Tümü" : q}
                      </button>
                    ))}
                  </div>
                </div>
                <div className="relative">
                  <Input
                    value={symbolQuery}
                    onChange={(e) => {
                      setSymbolQuery(e.target.value);
                      setListOpen(true);
                    }}
                    onFocus={() => setListOpen(true)}
                    onBlur={() => setTimeout(() => setListOpen(false), 150)}
                    placeholder={category === "linear" ? "örn. BTC, ETH, SOL... (linear vadeli)" : "örn. BTC, ETH, SOL... (spot)"}
                    className="mt-1.5 border-border-strong bg-surface font-mono text-slate-100 placeholder:text-slate-500"
                    aria-label="Sembol ara"
                  />
                  {listOpen && filteredInstruments.length > 0 && (
                    <div className="absolute z-20 mt-1 max-h-64 w-full overflow-y-auto rounded-lg border border-border-strong bg-surface shadow-xl">
                      <div className="num sticky top-0 border-b border-border-strong bg-surface px-3 py-1.5 font-mono text-[10px] text-slate-400">
                        {venue} {category.toUpperCase()} · {filteredInstruments.length} / {instruments.length} çift
                      </div>
                      {filteredInstruments.slice(0, 40).map((i) => (
                        <button
                          key={i.symbol}
                          type="button"
                          onMouseDown={(e) => e.preventDefault()}
                          onClick={() => {
                            setSymbol(i.symbol);
                            setInstrument(i);
                            setSymbolQuery("");
                            setListOpen(false);
                          }}
                          className={cn(
                            "flex w-full items-center justify-between px-3 py-2 font-mono text-xs transition-colors hover:bg-[#1a2233]",
                            symbol === i.symbol ? "text-cyan-300" : "text-slate-200",
                          )}
                        >
                          <span>{i.symbol}</span>
                          <span className="flex items-center gap-1.5 text-[10px] text-slate-400">
                            {(() => {
                              const t = tickers[i.symbol];
                              if (!t) return null;
                              const pct = fracToNum(t.change_pct_24h);
                              return (
                                <span
                                  className={cn(
                                    "font-mono",
                                    pct !== null && pct < 0 ? "text-rose-400" : "text-emerald-400",
                                  )}
                                >
                                  {pct !== null && pct >= 0 ? "+" : ""}
                                  {t.change_pct_24h}%
                                </span>
                              );
                            })()}
                            {i.quote} · {fmtFrac(i.max_leverage)}x
                          </span>
                        </button>
                      ))}
                    </div>
                  )}
                </div>
                {/* Hızlı Seçim Popüler Coinler */}
                <div className="mt-2 flex flex-wrap items-center gap-1.5">
                  <span className="text-[10px] text-slate-400">Popüler:</span>
                  {QUICK_COINS.map((c) => {
                    const qAsset = quoteFilter === "ALL" ? "USDT" : quoteFilter;
                    const targetSymbol = `${c}${qAsset}`;
                    const isSelected = symbol?.startsWith(c);
                    return (
                      <button
                        key={c}
                        type="button"
                        onClick={() => {
                          setSymbolQuery(c);
                          const found = instruments.find((i) => i.symbol === targetSymbol || i.base === c);
                          if (found) {
                            setSymbol(found.symbol);
                            setInstrument(found);
                          }
                        }}
                        className={cn(
                          "rounded border px-2 py-0.5 font-mono text-[11px] transition-colors",
                          isSelected
                            ? "border-cyan-500/60 bg-cyan-500/20 text-cyan-200"
                            : "border-[#252f42] bg-[#141a26] text-slate-300 hover:border-cyan-500/40 hover:text-cyan-300",
                        )}
                      >
                        {c}
                      </button>
                    );
                  })}
                  {symbolQuery && (
                    <button
                      type="button"
                      onClick={() => setSymbolQuery("")}
                      className="ml-auto text-[10px] text-slate-400 hover:text-slate-300 underline"
                    >
                      Aramayı temizle
                    </button>
                  )}
                </div>
                <div className="mt-3">
                  <Label className="text-xs text-slate-400">Periyot</Label>
                  <Select value={intervalSel} onValueChange={(v) => setIntervalSel(v as Interval)}>
                    <SelectTrigger className="mt-1.5 min-h-11 w-full border-border-strong bg-surface text-slate-100">
                      <SelectValue />
                    </SelectTrigger>
                    <SelectContent className="border-border-strong bg-[#151a24]">
                      {INTERVALS.map((iv) => (
                        <SelectItem key={iv} value={iv} className="font-mono text-slate-200">
                          {iv}
                        </SelectItem>
                      ))}
                    </SelectContent>
                  </Select>
                </div>
                <p className="mt-2 text-[11px] text-slate-400">
                  Borsa: <span className="font-mono text-emerald-400">{venue}</span> · Kategori: <span className="font-mono text-cyan-400">{category}</span>
                  {isShort && " — short strateji linear (vadeli) gerektirir"}
                </p>
              </Panel>
            </div>

          </div>
        </WizardSection>

        {/* ---------------- SECTION 3: parameters + start condition ---------------- */}
        <WizardSection n={3} title="Parametreler & Başlangıç Koşulu" valid={sec3Valid} hidden={!kind}>
          <div className="grid gap-4 lg:grid-cols-2">
            {!isGridKind ? (
              <Panel title="DCA Parametreleri" icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />}>
                <div className="grid grid-cols-2 gap-3">
                  <NumField label={sizingMode === "QUOTE_NOTIONAL" ? "Base Emir (USDT)" : "Base Emir (miktar)"} value={baseSize} onChange={setBaseSize} />
                  <NumField label="Güvenlik Emri" value={safetySize} onChange={setSafetySize} />
                </div>
                <div className="mt-3 flex gap-2">
                  {(["QUOTE_NOTIONAL", "BASE_QTY"] as const).map((m) => (
                    <button
                      key={m}
                      onClick={() => setSizingMode(m)}
                      className={cn(
                        "flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold",
                        sizingMode === m ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500",
                      )}
                    >
                      {m}
                    </button>
                  ))}
                </div>

                <div className="mt-5 space-y-5">
                  <SliderField label="Deviation (sapma) %" value={deviationPct} min={0.5} max={10} step={0.1} onChange={setDeviationPct} />
                  <SliderField label="Maks. Güvenlik Emri" value={maxSafety} min={0} max={10} step={1} onChange={(v) => setMaxSafety(Math.round(v))} integer />
                  <SliderField label="Hedef Kâr (TP) %" value={targetProfitPct} min={0.2} max={10} step={0.1} onChange={setTargetProfitPct} />
                  <div className="grid grid-cols-2 gap-3">
                    <NumField label="Step Scale" value={stepScale} onChange={setStepScale} />
                    <NumField label="Volume Scale" value={volumeScale} onChange={setVolumeScale} />
                  </div>
                </div>
              </Panel>
            ) : (
              <Panel title="Grid Parametreleri" icon={<Grid3x3 className="h-4 w-4 text-slate-500" aria-hidden />}>
                <div className="grid grid-cols-2 gap-3">
                  <NumField label="Alt Sınır" value={gridLower} onChange={setGridLower} />
                  <NumField label="Üst Sınır" value={gridUpper} onChange={setGridUpper} />
                </div>
                <div className="mt-3 flex gap-2">
                  {(["arithmetic", "geometric"] as const).map((m) => (
                    <button
                      key={m}
                      onClick={() => setGridMode(m)}
                      className={cn(
                        "flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold",
                        gridMode === m ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500",
                      )}
                    >
                      {m}
                    </button>
                  ))}
                </div>
                <div className="mt-4">
                  <SliderField label="Seviye Sayısı" value={gridLevels} min={2} max={60} step={1} onChange={(v) => setGridLevels(Math.round(v))} integer />
                </div>
                <div className="mt-4">
                  <NumField label="Seviye Başına Notional (USDT)" value={gridNotional} onChange={setGridNotional} />
                </div>
                <div className="mt-4 flex items-center justify-between rounded-lg border border-border bg-surface p-3">
                  <div>
                    <div className="text-xs font-semibold text-slate-300">İz Süren Grid (Trail)</div>
                    <div className="text-[11px] text-slate-400">Fiyat mesh dışında kapanırsa ızgara bir genişlik kayar — trendi takip eder</div>
                  </div>
                  <Switch checked={gridTrail} onCheckedChange={setGridTrail} aria-label="Grid trail" />
                </div>
                <p className="mt-3 text-[11px] text-slate-400">
                  Fee-breaker: seviyeler arası spread maker ücretini karşılamıyorsa motor launch'ı reddeder.
                </p>
              </Panel>
            )}

            <Panel title="Risk & Çıkış" icon={<ShieldAlert className="h-4 w-4 text-slate-500" aria-hidden />}>
              <div className="space-y-4">
                <div className="flex items-center justify-between rounded-lg border border-border bg-surface p-3">
                  <div>
                    <div className="text-xs font-semibold text-slate-300">Net TP (M05)</div>
                    <div className="text-[11px] text-slate-400">Ücret + funding sonrası exact kâr hedefi</div>
                  </div>
                  <Switch checked={netTp} onCheckedChange={setNetTp} aria-label="Net TP" />
                </div>

                <div className="flex items-center justify-between rounded-lg border border-border bg-surface p-3">
                  <div>
                    <div className="text-xs font-semibold text-slate-300">Stop-Loss</div>
                    <div className="text-[11px] text-slate-400">M20 devriye: SL ≤ deviation ise çakışma</div>
                  </div>
                  <Switch checked={slEnabled} onCheckedChange={setSlEnabled} aria-label="Stop-loss" />
                </div>
                {slEnabled && (
                  <SliderField label="SL %" value={slPct} min={0.5} max={30} step={0.1} onChange={setSlPct} />
                )}
                {m20Warn && (
                  <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-300">
                    <AlertTriangle className="mr-1 inline h-3.5 w-3.5" aria-hidden />
                    M20 SL-ÇAKIŞMASI: SL ({slPct}%) ≤ deviation ({deviationPct}%). İşlem engellenmez —
                    Bölüm 6&apos;daki uyarı paneline M20_SL_CONFLICT eklenir; &quot;riski anladım&quot; onayıyla
                    dar Stop-Loss ile devam edebilirsiniz (erken stop durumunda kalan kademelerin sermayesi ölü kalır).
                  </div>
                )}

                <div className="flex items-center justify-between rounded-lg border border-border bg-surface p-3">
                  <div>
                    <div className="text-xs font-semibold text-slate-300">Trailing TP (M10)</div>
                    <div className="text-[11px] text-slate-400">ARMED → peak takip → geri çekilmede çıkış</div>
                  </div>
                  <Switch checked={trailing} onCheckedChange={setTrailing} aria-label="Trailing" />
                </div>
                {trailing && (
                  <div className="grid grid-cols-2 gap-3">
                    <SliderField label="Tetik %" value={trailTrigPct} min={0.2} max={5} step={0.1} onChange={setTrailTrigPct} />
                    <SliderField label="Geri Çekilme %" value={trailDevPct} min={0.1} max={3} step={0.1} onChange={setTrailDevPct} />
                  </div>
                )}

                <SliderField
                  label={marketType === "linear" ? (isFuturesDca ? "Kaldıraç (izole marjin)" : "Kaldıraç (linear)") : "Kaldıraç (spot: 1x)"}
                  value={leverage}
                  min={isFuturesDca ? 2 : 1}
                  max={marketType === "linear" ? 50 : 1}
                  step={1}
                  onChange={(v) => setLeverage(Math.round(v))}
                  integer
                />
                {isFuturesDca && (
                  <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] leading-relaxed text-amber-300/90">
                    M07 likidasyon aktif: pozisyon marjin sınırına ulaşınca LIQ zorunlu kapanışı çalışır, bekleyen emirler iptal edilir ve deal kayıp olarak kapanır.
                  </div>
                )}
              </div>
            </Panel>

            {/* M18 hedge — real engine inputs (backend: hedge_ratio ∈ (0,1], combined_target_profit > 0) */}
            {isHedgeKind && (
              <Panel title="Hedge Parametreleri (M18)" icon={<ShieldHalf className="h-4 w-4 text-slate-500" aria-hidden />}>
                <div className="grid grid-cols-2 gap-3">
                  <div className="space-y-1.5">
                    <Label htmlFor="hedge-ratio" className="text-xs text-slate-400">
                      Hedge Oranı ρ (0.05–1)
                    </Label>
                    <Input
                      id="hedge-ratio"
                      type="number"
                      min="0.05"
                      max="1"
                      step="0.05"
                      value={hedgeRatio}
                      onChange={(e) => setHedgeRatio(e.target.value)}
                      className="num min-h-11 border-border-strong bg-surface text-slate-100"
                      aria-label="Hedge oranı (0.05 ile 1 arası)"
                    />
                  </div>
                  <div className="space-y-1.5">
                    <Label htmlFor="hedge-combined-tp" className="text-xs text-slate-400">
                      Birleşik Kâr Hedefi % (1–200)
                    </Label>
                    <Input
                      id="hedge-combined-tp"
                      type="number"
                      min="1"
                      max="200"
                      step="1"
                      value={combinedTp}
                      onChange={(e) => setCombinedTp(e.target.value)}
                      className="num min-h-11 border-border-strong bg-surface text-slate-100"
                      aria-label="Birleşik kâr hedefi yüzdesi"
                    />
                  </div>
                </div>
                {!hedgeValid && (
                  <p className="mt-2 text-[11px] text-rose-400">
                    Geçersiz hedge girdisi: ρ 0.05–1 aralığında, birleşik hedef 1–200, koruma tetikleyici (0, 100] arasında olmalı — motor 422 döner.
                  </p>
                )}
                {kind === "HEDGE_PROTECTIVE" && (
                  <div className="mt-3 space-y-1.5">
                    <Label htmlFor="hedge-dd-trigger" className="text-xs text-slate-400">
                      Koruma Tetikleyici % (0–100 — zirveden düşüş eşiği)
                    </Label>
                    <Input
                      id="hedge-dd-trigger"
                      type="number"
                      min="0.5"
                      max="100"
                      step="0.5"
                      value={hedgeDdTrigger}
                      onChange={(e) => setHedgeDdTrigger(e.target.value)}
                      className="num min-h-11 border-border-strong bg-surface text-slate-100"
                      aria-label="Koruma tetikleyici düşüş yüzdesi"
                    />
                    <p className="text-[10px] leading-relaxed text-slate-400">
                      Ana pozisyon, koşu zirvesinden bu kadar düşünce motor GERÇEK bir short koruma emri açar (rho × pozisyon).
                      Varsayılan %6; artık config'ten yönetilir.
                    </p>
                  </div>
                )}
                <p className="mt-3 text-[11px] leading-relaxed text-slate-400">
                  ρ, karşı bacağın (koruma pozisyonunun) notional oranıdır; birleşik kâr hedefi iki bacağın toplam
                  kâr hedefidir. Motor gerçek HedgeEngine ile çift pozisyon açar (M18). Bacak bazlı k/z, birleşik
                  k/z ve spawn geçmişi stüdyodaki Hedge sekmesinde canlı izlenir.
                </p>
              </Panel>
            )}

            <Panel className="lg:col-span-2" title="Başlangıç Koşulu (Entry Trigger)" icon={<Clock className="h-4 w-4 text-slate-500" aria-hidden />}>
              <div className="grid grid-cols-2 gap-2 sm:grid-cols-3 lg:grid-cols-6">
                {SC_MODES.map((m) => (
                  <button
                    key={m.mode}
                    onClick={() => setScMode(m.mode)}
                    aria-pressed={scMode === m.mode}
                    className={cn(
                      "min-h-11 rounded-lg border px-2 py-2 text-[11px] font-bold transition-colors",
                      scMode === m.mode ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300" : "border-border bg-surface text-slate-400 hover:border-border-strong",
                    )}
                  >
                    {m.label}
                  </button>
                ))}
              </div>
              <p className="mt-2 text-[11px] text-slate-400">{SC_MODES.find((m) => m.mode === scMode)?.hint}</p>

              {scMode === "PRICE_BELOW" && (
                <div className="mt-3 grid gap-3 sm:grid-cols-2">
                  <NumField label="Başlama Eşiği (fiyat ≤)" value={scPrice} onChange={setScPrice} />
                  <div className="flex items-end pb-1 text-[11px] text-slate-400">
                    Her yeni deal bu koşula bağlıdır — bar kapanışında değerlendirilir.
                  </div>
                </div>
              )}
              {scMode === "PRICE_ABOVE" && (
                <div className="mt-3 grid gap-3 sm:grid-cols-2">
                  <NumField label="Başlama Eşiği (fiyat ≥)" value={scPrice} onChange={setScPrice} />
                  <div className="flex items-end pb-1 text-[11px] text-slate-400">
                    Kırılım yukarı onaylanınca işleme girer — bar kapanışında değerlendirilir.
                  </div>
                </div>
              )}
              {scMode === "DROP_FROM_REF" && (
                <div className="mt-3 grid gap-3 sm:grid-cols-2">
                  <NumField label="Referans Fiyat" value={scRef} onChange={setScRef} />
                  <SliderField label="Düşüş %" value={scDropPct} min={1} max={90} step={0.5} onChange={setScDropPct} />
                </div>
              )}
              {scMode === "RSI_BELOW" && (
                <div className="mt-3">
                  <SliderField label="RSI Eşiği (14 periyot)" value={scRsi} min={5} max={90} step={1} onChange={(v) => setScRsi(Math.round(v))} integer />
                </div>
              )}
              {scMode === "SCHEDULED" && (
                <div className="mt-3">
                  <Label className="text-xs text-slate-400">Başlangıç Zamanı</Label>
                  <Input
                    type="datetime-local"
                    value={scDateTime}
                    onChange={(e) => setScDateTime(e.target.value)}
                    className="num mt-1.5 min-h-11 border-border-strong bg-surface text-slate-100"
                    aria-label="Başlangıç zamanı"
                  />
                </div>
              )}
              {scMode === "PINE" && (
                <div className="mt-3 space-y-3">
                  {/* şablonlar + kitaplık */}
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-[11px] font-semibold uppercase tracking-wide text-slate-400">Şablonlar:</span>
                    {PINE_TEMPLATES.map((t) => (
                      <button
                        key={t.name}
                        onClick={() => {
                          setPineCode(t.code);
                          setPineInfo(null);
                          setPineErr(null);
                          setPineBt(null);
                          setPineBtErr(null);
                          setPineInputs({});
                          setLibLoadedId(null);
                          setLibName("");
                        }}
                        className="min-h-9 rounded-md border border-border bg-surface px-3 text-[11px] font-semibold text-slate-300 transition-colors hover:border-emerald-500/50 hover:text-emerald-300"
                      >
                        {t.name}
                      </button>
                    ))}
                    <span className="grow" />
                    <Button
                      variant="outline"
                      size="sm"
                      className="min-h-9 border-border-strong text-[11px] text-slate-200 hover:bg-secondary"
                      onClick={openLib}
                    >
                      <Library className="mr-1.5 h-3.5 w-3.5" aria-hidden />
                      Kitaplık{libLoadedId ? " ✓" : ""}
                    </Button>
                  </div>
                  {/* kod editörü */}
                  <div>
                    <Label className="text-xs text-slate-400">Pine Strateji Kodu (alt-küme)</Label>
                    <textarea
                      value={pineCode}
                      onChange={(e) => {
                        setPineCode(e.target.value);
                        setPineInfo(null);
                      }}
                      spellCheck={false}
                      rows={12}
                      aria-label="Pine strateji kodu"
                      className="mt-1.5 w-full rounded-lg border border-border-strong bg-surface p-3 font-mono text-[12px] leading-relaxed text-emerald-100 outline-none focus:border-emerald-500/50"
                      placeholder={'//@version=5\nstrategy("Stratejim")\nif ta.crossover(ta.ema(close, 9), ta.ema(close, 21))\n    strategy.entry("L", strategy.long)'}
                    />
                  </div>
                  {/* Task 20 — opsiyonel bot TP/SL (pine çıkışına ek) */}
                  <div className="grid gap-2 sm:grid-cols-2">
                    <div>
                      <Label className="text-xs text-slate-400">Bot TP % (opsiyonel — pine çıkışına ek)</Label>
                      <Input
                        value={pineTp}
                        onChange={(e) => setPineTp(e.target.value)}
                        placeholder="boş = TP kapalı (çıkış yalnız pine'dan)"
                        className="num mt-1.5 min-h-11 border-border-strong bg-surface text-slate-100"
                        aria-label="Pine backtest bot hedef kâr yüzdesi"
                      />
                    </div>
                    <div>
                      <Label className="text-xs text-slate-400">Bot SL % (opsiyonel — pine çıkışına ek)</Label>
                      <Input
                        value={pineSl}
                        onChange={(e) => setPineSl(e.target.value)}
                        placeholder="boş = SL yok"
                        className="num mt-1.5 min-h-11 border-border-strong bg-surface text-slate-100"
                        aria-label="Pine backtest bot stop-loss yüzdesi"
                      />
                    </div>
                  </div>
                  {/* eylemler */}
                  <div className="flex flex-wrap gap-2">
                    <Button
                      variant="outline"
                      disabled={pineBusy !== "idle" || !pineCode.trim()}
                      className="min-h-11 border-border-strong text-xs text-slate-200 hover:bg-secondary"
                      onClick={async () => {
                        setPineBusy("validate");
                        setPineErr(null);
                        try {
                          const r = await apiPineValidate(pineCode);
                          setPineInfo(r);
                          setPineInputs((p) => {
                            const next = { ...p };
                            for (const k of r.inputs) if (!(k in next)) next[k] = "";
                            return next;
                          });
                          if (pineBtErr) setPineBtErr(null);
                        } catch (e) {
                          setPineInfo(null);
                          setPineErr(e instanceof EngineError ? (e.body as { error?: string })?.error || e.message : String(e));
                        } finally {
                          setPineBusy("idle");
                        }
                      }}
                    >
                      {pineBusy === "validate" ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : <ShieldCheck className="h-3.5 w-3.5" aria-hidden />}
                      Doğrula
                    </Button>
                    <Button
                      variant="outline"
                      disabled={pineBusy !== "idle" || !pineCode.trim() || !symbol}
                      className="min-h-11 border-border-strong text-xs text-slate-200 hover:bg-secondary"
                      onClick={async () => {
                        setPineBusy("bt");
                        setPineBtErr(null);
                        setPineBt(null);
                        try {
                          const r = await apiPineBacktest({
                            code: pineCode,
                            // boş input değeri = scriptin default'u — evaluator'a "" gönderilmez
                            inputs: Object.fromEntries(Object.entries(pineInputs).filter(([, v]) => v.trim() !== "")),
                            symbol: symbol ?? "BTCUSDT",
                            interval: intervalSel,
                            bars: 400,
                            // Task 20 — opsiyonel bot TP/SL (pine çıkışına ek)
                            ...(pineTp.trim() ? { target_profit_pct: pineTp.trim() } : {}),
                            ...(pineSl.trim() ? { stop_loss_pct: pineSl.trim() } : {}),
                          });
                          setPineBt(r);
                        } catch (e) {
                          setPineBtErr(e instanceof EngineError ? (e.body as { error?: string })?.error || e.message : String(e));
                        } finally {
                          setPineBusy("idle");
                        }
                      }}
                    >
                      {pineBusy === "bt" ? <Loader2 className="h-3.5 w-3.5 animate-spin" aria-hidden /> : <ChartCandlestick className="h-3.5 w-3.5" aria-hidden />}
                      Backtest (400 bar)
                    </Button>
                  </div>
                  {/* doğrulama sonucu */}
                  {pineErr && (
                    <div className="rounded-md border border-rose-500/40 bg-rose-500/10 p-3 text-xs text-rose-300">
                      <span className="font-bold">Derleme hatası:</span> {pineErr}
                    </div>
                  )}
                  {pineInfo?.ok && (
                    <div className="rounded-md border border-emerald-500/40 bg-emerald-500/10 p-3 text-xs text-emerald-300">
                      <div className="font-bold">
                        Derlendi{pineInfo.decl ? ` — ${pineInfo.decl.kind}: "${pineInfo.decl.title}"` : ""} · {pineInfo.functions.length} fonksiyon
                        {pineInfo.decl?.pyramiding ? ` · pyramiding=${pineInfo.decl.pyramiding} (desteklenmiyor — 1 olarak işlenir)` : ""}
                      </div>
                      {(pineInfo.notes?.length ?? 0) > 0 && (
                        <div className="mt-2 rounded border border-amber-500/40 bg-amber-500/10 p-2 text-[11px] text-amber-300">
                          {pineInfo.notes!.map((n, i) => (
                            <div key={i} className="leading-relaxed">
                              ⚠ {n}
                            </div>
                          ))}
                        </div>
                      )}
                      {pineInfo.inputs.length > 0 && (
                        <div className="mt-2 grid gap-2 sm:grid-cols-3">
                          {pineInfo.inputs.map((k) => (
                            <div key={k}>
                              <Label className="text-[10px] text-emerald-200/70">{k}</Label>
                              <Input
                                value={pineInputs[k] ?? ""}
                                onChange={(e) => setPineInputs((p) => ({ ...p, [k]: e.target.value }))}
                                className="num min-h-9 border-border-strong bg-surface text-slate-100"
                                aria-label={`Pine input ${k}`}
                              />
                            </div>
                          ))}
                        </div>
                      )}
                      <div className="mt-2 text-[10px] leading-relaxed text-emerald-200/60">
                        Kullanılan fonksiyonlar: {pineInfo.functions.join(", ") || "—"} · ta.*[n] geçmişi desteklenir · Pyramiding yok (açık pozisyon
                        varken entry yoksayılır) · ta.* koşullu blok içinde seriyi kaydırabilir (TV uyarısı gibi derleme notu düşer).
                      </div>
                    </div>
                  )}
                  {/* pine backtest sonucu */}
                  {pineBtErr && (
                    <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-xs text-amber-300">{pineBtErr}</div>
                  )}
                  {pineBt && (
                    <div className="rounded-md border border-border-strong bg-surface p-3">
                      <div className="text-[11px] font-bold uppercase tracking-wide text-slate-400">
                        Pine Backtest — {symbol ?? "?"} · {intervalSel} · {pineBt.metrics.bars} bar (gerçekçi dolum modeli)
                      </div>
                      <div className="mt-2 grid grid-cols-2 gap-2 text-[11px] sm:grid-cols-4">
                        <div className="rounded border border-border p-2">
                          <div className="text-slate-500">Giriş Emri</div>
                          <div className="num font-bold text-slate-100">{pineBt.entries}</div>
                        </div>
                        <div className="rounded border border-border p-2">
                          <div className="text-slate-500">Kapanan Deal</div>
                          <div className="num font-bold text-slate-100">{pineBt.metrics.deals}</div>
                        </div>
                        <div className="rounded border border-border p-2">
                          <div className="text-slate-500">Getiri</div>
                          <div className={cn("num font-bold", Number(pineBt.metrics.total_return_pct) >= 0 ? "text-emerald-400" : "text-rose-400")}>
                            {pineBt.metrics.total_return_pct}%
                          </div>
                        </div>
                        <div className="rounded border border-border p-2">
                          <div className="text-slate-500">Max DD</div>
                          <div className="num font-bold text-amber-300">{pineBt.metrics.max_drawdown_pct}%</div>
                        </div>
                      </div>
                      {pineBt.metrics.fill_realism && (
                        <div className="mt-2 text-[10px] text-slate-400">
                          Dolum modeli: {pineBt.metrics.fill_realism.marketable_taker} taker · {pineBt.metrics.fill_realism.resting_touch} resting ·{" "}
                          {pineBt.metrics.fill_realism.open_gap_better} gap-better · maker fee {pineBt.metrics.fill_realism.maker_fees} / taker fee{" "}
                          {pineBt.metrics.fill_realism.taker_fees}
                        </div>
                      )}
                      {pineBt.pineWarnings.length > 0 && (
                        <div className="mt-2 rounded border border-amber-500/30 bg-amber-500/5 p-2 text-[10px] text-amber-300">
                          {pineBt.pineWarnings.map((w, i) => (
                            <div key={i}>· {w}</div>
                          ))}
                        </div>
                      )}
                      {(pineBt.pineNotes?.length ?? 0) > 0 && (
                        <div className="mt-2 rounded border border-amber-500/40 bg-amber-500/10 p-2 text-[10px] text-amber-300">
                          <span className="font-bold">Derleme notları:</span>
                          {pineBt.pineNotes!.map((n, i) => (
                            <div key={i} className="leading-relaxed">
                              ⚠ {n}
                            </div>
                          ))}
                        </div>
                      )}
                      {/* Task 21 — plotlar ANA GRAFİKTE: backtest'in kendi barları + pine plot çizgileri + fill okları. */}
                      {pineBt.bars && pineBt.bars.length > 0 && (
                        <div className="mt-3">
                          <div className="text-[10px] font-bold uppercase tracking-wide text-slate-400">
                            Fiyat Grafiği — Pine Plotları + Fill İşaretleri (backtest bar penceresi)
                          </div>
                          <div className="mt-1.5">
                            <TvChart
                              symbol={symbol ?? "?"}
                              interval={intervalSel}
                              data={pineBt.bars}
                              plotSeries={pinePlotSeries.length > 0 ? pinePlotSeries : undefined}
                              markers={pineMarkers}
                              height={320}
                            />
                          </div>
                        </div>
                      )}
                      {/* Task 20 — sparkline fallback (bar penceresi yoksa plotlar hâlâ görünür) */}
                      {!(pineBt.bars && pineBt.bars.length > 0) && pineBt.pinePlots && Object.keys(pineBt.pinePlots).length > 0 && (
                        <div className="mt-3">
                          <div className="text-[10px] font-bold uppercase tracking-wide text-slate-400">Plot Serileri</div>
                          <div className="mt-1.5 grid gap-2 sm:grid-cols-2">
                            {Object.entries(pineBt.pinePlots).map(([title, series]) => {
                              const nums = series.map((s) => (s === "na" ? null : Number(s)));
                              const valid = nums.filter((n): n is number => n !== null && Number.isFinite(n));
                              if (valid.length < 2) {
                                return (
                                  <div key={title} className="rounded border border-border p-2 text-[10px] text-slate-400">
                                    {title}: çizilebilir veri yok
                                  </div>
                                );
                              }
                              const mn = Math.min(...valid);
                              const mx = Math.max(...valid);
                              const span = mx - mn || 1;
                              const W = 240;
                              const H = 44;
                              const pts = nums
                                .map((n, i) => (n === null ? null : `${((i / (nums.length - 1)) * W).toFixed(1)},${(H - ((n - mn) / span) * (H - 4) - 2).toFixed(1)}`))
                                .filter(Boolean)
                                .join(" ");
                              return (
                                <div key={title} className="rounded border border-border p-2">
                                  <div className="flex items-baseline justify-between text-[10px]">
                                    <span className="text-slate-400">{title}</span>
                                    <span className="num text-slate-500">
                                      {String(valid[valid.length - 1])} (min {String(mn)} / max {String(mx)})
                                    </span>
                                  </div>
                                  <svg viewBox={`0 0 ${W} ${H}`} className="mt-1 h-11 w-full" role="img" aria-label={`${title} plot serisi`}>
                                    <polyline points={pts} fill="none" stroke="#34d399" strokeWidth="1.5" />
                                  </svg>
                                </div>
                              );
                            })}
                          </div>
                        </div>
                      )}
                      <div className="mt-2 flex items-center gap-2 text-[10px] leading-relaxed text-slate-400">
                        {pineBt.botStopsApplied ? (
                          <span className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-emerald-300">Bot TP/SL EKTE</span>
                        ) : (
                          <span className="rounded border border-border-strong px-1.5 py-0.5">Bot TP/SL kapalı</span>
                        )}
                        <span>
                          {pineBt.botStopsApplied
                            ? "Çıkışlar pine + bot TP/SL'den gelir (hangisi önce vurursa)."
                            : "Çıkışlar yalnız Pine kodundan gelir. Bot olarak kurduğunuzda Bölüm 3'teki TP/SL ek olarak uygulanır."}
                        </span>
                      </div>
                    </div>
                  )}
                  <div className="rounded-md border border-border-strong bg-surface p-3 text-[11px] leading-relaxed text-slate-400">
                    <span className="font-semibold text-slate-400">Desteklenen alt küme:</span> ta.sma/ema/rma/rsi/tr/atr/highest/lowest/
                    change/mom/crossover/crossunder/cross/sum/cum/barssince · math.abs/max/min/round/floor/ceil/pow · input.float/int/bool/
                    string · strategy.entry/close/close_all/exit(stop=,limit=) · if/else, ternary, [n] geçmişi, and/or/not. Tam PineScript
                    DEĞİLDİR — bilinmeyen fonksiyonlar derlemede reddedilir (sessiz sahte sinyal yok).
                  </div>
                  {/* Task 21 — Pine strateji kitaplığı (kaydet/yükle/sil) */}
                  <Dialog open={libOpen} onOpenChange={setLibOpen}>
                    <DialogContent className="max-w-lg border-border-strong bg-surface text-slate-100">
                      <DialogHeader>
                        <DialogTitle className="text-sm">Pine Strateji Kitaplığı</DialogTitle>
                        <DialogDescription className="text-xs text-slate-400">
                          Derlenen stratejiler motorda saklanır — derlemeyen kod kütüphaneye giremez (sessiz bozuk kayıt yok).
                        </DialogDescription>
                      </DialogHeader>
                      <div className="thin-scroll max-h-64 space-y-1.5 overflow-y-auto">
                        {libScripts === null && <div className="py-4 text-center text-xs text-slate-500">Yükleniyor…</div>}
                        {libScripts?.length === 0 && (
                          <div className="py-4 text-center text-xs text-slate-500">Kitaplık boş — kodu yazıp aşağıdan kaydedin.</div>
                        )}
                        {libScripts?.map((s) => (
                          <div key={s.id} className="flex items-center gap-2 rounded-md border border-border bg-surface px-2.5 py-2">
                            <div className="min-w-0 flex-1">
                              <div className="truncate text-xs font-semibold text-slate-200">
                                {s.name}
                                {libLoadedId === s.id && <span className="ml-1.5 text-[9px] text-emerald-400">✓ yüklü</span>}
                              </div>
                              <div className="text-[10px] text-slate-400">
                                {s.code.split("\n").length} satır · {Object.keys(s.inputs ?? {}).length} input
                                {s.updatedAt ? ` · ${new Date(s.updatedAt).toLocaleDateString("tr-TR")}` : ""}
                              </div>
                            </div>
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={libBusy}
                              className="min-h-9 border-emerald-500/40 text-[11px] text-emerald-300 hover:bg-emerald-500/10"
                              onClick={() => loadFromLib(s)}
                            >
                              Yükle
                            </Button>
                            {libConfirmDelete === s.id ? (
                              <Button
                                size="sm"
                                variant="outline"
                                disabled={libBusy}
                                className="min-h-9 border-rose-500/50 px-2 text-[11px] text-rose-300 hover:bg-rose-500/10"
                                onClick={() => void deleteFromLib(s.id)}
                              >
                                Onayla
                              </Button>
                            ) : (
                              <Button
                                size="sm"
                                variant="outline"
                                disabled={libBusy}
                                className="min-h-9 border-border-strong px-2 text-slate-400 hover:text-rose-300"
                                onClick={() => setLibConfirmDelete(s.id)}
                                aria-label={`${s.name} kaydını sil`}
                              >
                                <Trash2 className="h-3.5 w-3.5" aria-hidden />
                              </Button>
                            )}
                          </div>
                        ))}
                        {libErr && <div className="rounded-md border border-rose-500/40 bg-rose-500/10 p-2 text-[11px] text-rose-300">{libErr}</div>}
                      </div>
                      <div className="space-y-2 border-t border-border pt-3">
                        <Label className="text-[11px] text-slate-400">Editördeki kodu kaydet</Label>
                        <div className="flex flex-wrap gap-2">
                          <Input
                            value={libName}
                            onChange={(e) => setLibName(e.target.value)}
                            placeholder="Strateji adı (örn. EMA 9/21 Kripto)"
                            className="min-h-10 min-w-40 flex-1 border-border-strong bg-surface text-xs text-slate-100"
                            aria-label="Kitaplık strateji adı"
                          />
                          <Button
                            size="sm"
                            disabled={libBusy || !pineCode.trim()}
                            className="min-h-10 bg-emerald-500/90 text-[#04110c] hover:bg-emerald-500"
                            onClick={() => void saveToLib()}
                          >
                            {libBusy ? <Loader2 className="mr-1 h-3.5 w-3.5 animate-spin" aria-hidden /> : <Save className="mr-1 h-3.5 w-3.5" aria-hidden />}
                            Kaydet
                          </Button>
                          {libLoadedId && (
                            <Button
                              size="sm"
                              variant="outline"
                              disabled={libBusy || !pineCode.trim()}
                              className="min-h-10 border-border-strong text-[11px] text-slate-300 hover:bg-secondary"
                              onClick={() => void updateInLib()}
                            >
                              Güncelle
                            </Button>
                          )}
                        </div>
                      </div>
                    </DialogContent>
                  </Dialog>
                </div>
              )}

              {/* live on-screen evaluation */}
              {scMetNow !== null && (
                <div
                  className={cn(
                    "mt-3 flex items-center gap-2 rounded-md border p-3 text-xs",
                    scMetNow ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300" : "border-amber-500/40 bg-amber-500/10 text-amber-300",
                  )}
                >
                  {scMetNow ? <CheckCircle2 className="h-3.5 w-3.5" aria-hidden /> : <Clock className="h-3.5 w-3.5" aria-hidden />}
                  {scMetNow ? "Koşul şu an SAĞLANIR — bot launch sonrası ilk barda işleme girer." : "Koşul şu an SAĞLANMAZ — bot koşul sağlanana kadar bekler (KOŞUL BEKLİYOR)."}
                </div>
              )}
              {scMode === "RSI_BELOW" && (
                <div className="mt-3 rounded-md border border-border-strong bg-surface p-3 text-[11px] text-slate-400">
                  RSI 14 periyot kapanış penceresiyle exact-fraction hesaplanır; ilk ~14 barda koşul sağlanmaz (ısınma penceresi).
                </div>
              )}
            </Panel>

            {/* 3Commas-style composite signal engine (AND/OR tree) */}
            <Panel className="lg:col-span-2" title="Sinyal Motoru — Gösterge Koşul Ağacı (AND/OR)" icon={<Workflow className="h-4 w-4 text-slate-500" aria-hidden />}>
              <SignalBuilder
                value={signalCfg}
                onChange={setSignalCfg}
                symbol={symbol ?? "BTCUSDT"}
                baseInterval={intervalSel}
                venue={venue}
                onPreviewChange={handleSignalPreview}
              />
            </Panel>
          </div>
        </WizardSection>

        {/* ---------------- SECTION 4: capital ---------------- */}
        <WizardSection n={4} title="Sermaye" valid={sec4Valid} hidden={!kind}>
          <div className="space-y-4">
            <div className="grid gap-3 sm:grid-cols-3">
              <NumField label="Başlangıç Sermayesi (USDT)" value={startCash} onChange={setStartCash} />
              {!isGridKind && (
                <NumField
                  label={`Çapa Fiyat (anchor)${anchorFetched ? ` — piyasa: ${anchorFetched}` : ""}`}
                  value={anchor}
                  onChange={setAnchor}
                />
              )}
              <div className="flex items-end">
                <Button variant="outline" className="min-h-11 w-full border-border-strong text-xs text-slate-300 hover:bg-secondary" onClick={() => void fetchAnchor()}>
                  {anchorFetched ? "Piyasa Fiyatını Yenile" : "Piyasa Fiyatını Getir"}
                </Button>
              </div>
            </div>

            {calcError && (
              <div className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">{calcError}</div>
            )}

            {calcBusy && !calcDca && !calcGrid && (
              <div className="flex justify-center py-8">
                <Spinner className="h-6 w-6" />
              </div>
            )}

            {!isGridKind && calcDca && (
              <div className="grid gap-4 xl:grid-cols-3">
                <Panel className="xl:col-span-2" title="Sermaye Merdiveni (calc/dca)" icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />} bodyClassName="p-0 px-2 py-2">
                  <LadderTable rows={calcDca.levels} m20From={calcDca.m20.firstUnreachableLevel} />
                </Panel>
                <div className="space-y-3">
                  <div className="grid grid-cols-2 gap-3">
                    <Stat label="Toplam Notional" value={fmtUsd(calcDca.totalNotionalDisplay ?? calcDca.totalNotional)} />
                    <Stat label="Toplam Miktar" value={fmtFrac(calcDca.totalQty)} />
                    <Stat label="Ort. Giriş (WAC)" value={fmtFrac(calcDca.avgAllIn)} />
                    <Stat label="Tahmini Ücret" value={fmtUsd(calcDca.estimatedFees, 4)} tone="amber" />
                    <Stat label="Başlangıç Marjı" value={fmtUsd(calcDca.estimatedInitialMargin)} tone="cyan" />
                    <Stat
                      label="Likidasyon (linear)"
                      value={calcDca.liquidationPrice ? fmtFrac(calcDca.liquidationPrice) : "—"}
                      tone={calcDca.liquidationPrice ? "rose" : "slate"}
                    />
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <PassChip ok={calcDca.m20.viable} label="M20 SL-CONFLICT" />
                    <GateChip
                      state={guard.feeGuardState}
                      label="FEE GUARD"
                      title={`FEE GUARD matematiği — ${guard.feeDetail}`}
                    />
                    <GateChip
                      state={guard.capitalState}
                      label="CAPITAL G6"
                      title={`CAPITAL G6 matematiği — ${guard.capitalDetail}`}
                    />
                  </div>
                </div>
              </div>
            )}

            {isGridKind && calcGrid && (
              <div className="grid gap-4 xl:grid-cols-3">
                <Panel className="xl:col-span-2" title="Grid Mesh (calc/grid)" icon={<Grid3x3 className="h-4 w-4 text-slate-500" aria-hidden />} bodyClassName="p-0 px-2 py-2">
                  <MeshTable rows={calcGrid.mesh} />
                </Panel>
                <div className="space-y-3">
                  <div className="grid grid-cols-2 gap-3">
                    <Stat label="Toplam Yatırım" value={fmtUsd(calcGrid.totalInvested)} />
                    <Stat label="Seviye" value={String(calcGrid.mesh.length)} />
                  </div>
                  <div className="flex flex-wrap gap-2">
                    <PassChip ok={calcGrid.feeBreakerPass} label="FEE-BREAKER" />
                    <GateChip
                      state={guard.capitalState}
                      label="CAPITAL G6"
                      title={`CAPITAL G6 matematiği — ${guard.capitalDetail}`}
                    />
                  </div>
                  {!calcGrid.feeBreakerPass && (
                    <div className="rounded-md border border-rose-500/40 bg-rose-500/10 p-3 text-xs text-rose-300">
                      Fee-breaker başarısız: seviye spread&apos;i maker ücreti karşılamıyor. Notional&apos;ı artırın veya
                      seviye sayısını düşürün.
                    </div>
                  )}
                </div>
              </div>
            )}
          </div>
        </WizardSection>

        {/* ---------------- SECTION 5: review ---------------- */}
        <WizardSection n={5} title="Gözden Geçir" valid={sec5Valid} hidden={!kind}>
          <div className="grid gap-4 lg:grid-cols-2">
            <Panel title="Konfigürasyon Özeti" icon={<Bot className="h-4 w-4 text-slate-500" aria-hidden />}>
              <dl className="num grid grid-cols-2 gap-x-4 gap-y-2 text-xs">
                <ReviewRow k="Ad" v={botName || "—"} />
                <ReviewRow k="Strateji" v={kind ?? "—"} />
                    {cardKey?.startsWith("SIGNAL") && (
                      <ReviewRow k="Havuz" v={`Signal Bot · ${cardKey === "SIGNAL_FUTURES" ? "vadeli (linear)" : "spot"} · sinyal-gated DCA (3Commas Signal karsiligi)`} />
                    )}
                <ReviewRow k="Borsa" v={`${venue} · ${mode}`} />
                <ReviewRow k="Sembol / Periyot" v={`${symbol ?? "—"} · ${intervalSel}`} />
                {!isGridKind ? (
                  <>
                    <ReviewRow k="Base / Safety" v={`${baseSize} / ${safetySize}`} />
                    <ReviewRow k="Sizing" v={sizingMode} />
                    <ReviewRow k="Deviation" v={`${deviationPct}%`} />
                    <ReviewRow k="Max Safety" v={String(maxSafety)} />
                    <ReviewRow k="Step / Vol Scale" v={`${stepScale} / ${volumeScale}`} />
                    <ReviewRow k="TP %" v={`${targetProfitPct}% (net: ${netTp ? "açık" : "kapalı"})`} />
                    <ReviewRow k="SL" v={slEnabled ? `${slPct}%` : "kapalı"} />
                    <ReviewRow k="Trailing" v={trailing ? `${trailTrigPct}% / ${trailDevPct}%` : "kapalı"} />
                    <ReviewRow k="Kaldıraç" v={`${leverage}x`} />
                    {isHedgeKind && <ReviewRow k="Hedge ρ / Birleşik TP" v={`${hedgeRatio} · ${combinedTp}%`} />}
                    {kind === "HEDGE_PROTECTIVE" && <ReviewRow k="Koruma Tetikleyici" v={`zirveden %${hedgeDdTrigger} düşüşte short bacağı`} />}
                    <ReviewRow k="Başlangıç Koşulu" v={describeTrigger(scMode, scPrice, scRef, scDropPct, scRsi, scDateTime)} />
                    <ReviewRow k="Sermaye" v={fmtUsd(startCash)} />
                  </>
                ) : (
                  <>
                    <ReviewRow k="Grid Aralığı" v={`${gridLower} – ${gridUpper}`} />
                    <ReviewRow k="Mod / Seviye" v={`${gridMode} · ${gridLevels}`} />
                    <ReviewRow k="Seviye Notional" v={gridNotional} />
                    <ReviewRow k="Başlangıç Koşulu" v={describeTrigger(scMode, scPrice, scRef, scDropPct, scRsi, scDateTime)} />
                    <ReviewRow k="Sermaye" v={fmtUsd(startCash)} />
                  </>
                )}
              </dl>
            </Panel>

            <Panel title="Gate Kontrol Listesi" icon={<ShieldCheck className="h-4 w-4 text-slate-500" aria-hidden />}>
              <p className="mb-2 text-[11px] text-slate-400">Rozetler canlı state'ten hesaplanır — koşulsuz yeşil yoktur.</p>
              <ul className="space-y-1.5">
                {gateRows.map((r) => (
                  <GateRow key={r.code} code={r.code} desc={r.desc} state={r.state} note={r.note} />
                ))}
              </ul>
            </Panel>

            <Panel className="lg:col-span-2" title="Bot Adı & Launch" icon={<Rocket className="h-4 w-4 text-slate-500" aria-hidden />}>
              <p className="text-xs text-slate-500">
                Bot adı ve BOTU BAŞLAT butonu sağdaki <span className="font-semibold text-slate-300">Açılacak Deal</span> panelinde — canlı grafik ve deal
                önizlemesiyle birlikte.
              </p>
            </Panel>
          </div>
        </WizardSection>

        {/* ---------------- SECTION 6: Risk Motoru & Piyasa Korumaları (Task 16) ---------------- */}
        <WizardSection n={6} title="Risk Motoru & Piyasa Korumaları" valid={sec6Valid} hidden={!kind}>
          <div className="space-y-4">
            <p className="text-[11px] leading-relaxed text-slate-400">
              Zero Hard-Block: motor sizi engellemez — koruma tetiklenince uyarı verir ve onayınızla izin verir
              (Kullanıcı Kendi Riskini Taşır). Değişmezler (Zero-Withdrawal, Gate 6 sermaye) bu panelde esnetilemez.
            </p>

            {/* a) 7 advisory guard — ZORUNLU / UYARI + İZİN */}
            <div className="space-y-2">
              {GUARD_ROWS.map((g) => {
                const val = riskPolicy[g.key];
                return (
                  <div
                    key={g.key}
                    className="flex flex-col gap-2 rounded-lg border border-border bg-surface p-3 sm:flex-row sm:items-center"
                  >
                    <div className="min-w-0 flex-1">
                      <div className="text-xs font-semibold text-slate-200">{g.name}</div>
                      <div className="mt-0.5 text-[11px] leading-snug text-slate-400">{g.desc}</div>
                    </div>
                    <div className="flex shrink-0 gap-1.5" role="group" aria-label={`${g.name} modu`}>
                      <button
                        onClick={() => setGuard(g.key, "ENFORCE")}
                        aria-pressed={val === "ENFORCE"}
                        className={cn(
                          "min-h-11 w-full rounded-lg border px-3 text-center transition-colors sm:w-auto",
                          val === "ENFORCE"
                            ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300"
                            : "border-border bg-surface text-slate-500 hover:border-border-strong",
                        )}
                      >
                        <span className="block text-[11px] font-bold leading-tight">ZORUNLU</span>
                        <span className="block text-[9px] leading-tight opacity-70">Engeller</span>
                      </button>
                      <button
                        onClick={() => setGuard(g.key, "PERMISSIVE_WITH_WARNING")}
                        aria-pressed={val === "PERMISSIVE_WITH_WARNING"}
                        className={cn(
                          "min-h-11 w-full rounded-lg border px-3 text-center transition-colors sm:w-auto",
                          val === "PERMISSIVE_WITH_WARNING"
                            ? "border-amber-500/60 bg-amber-500/10 text-amber-300"
                            : "border-border bg-surface text-slate-500 hover:border-border-strong",
                        )}
                      >
                        <span className="block text-[11px] font-bold leading-tight">UYARI + İZİN</span>
                        <span className="block text-[9px] leading-tight opacity-70">Engellemez</span>
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>

            {/* b + c) aktif güvenlik emri sayısı + günlük devre kesici */}
            <div className="grid gap-3 sm:grid-cols-2">
              <div className="space-y-1.5">
                <Label htmlFor="risk-active-so" className="text-xs text-slate-400">
                  Aynı Anda Bekleyecek Güvenlik Emri Sayısı <span className="text-slate-500">(Önerilen: 1)</span>
                </Label>
                <Input
                  id="risk-active-so"
                  type="number"
                  min={1}
                  max={Math.max(maxSafety, 1)}
                  step={1}
                  value={riskPolicy.active_safety_orders}
                  onChange={(e) => {
                    const n = Math.round(Number(e.target.value));
                    patchRiskPolicy({ active_safety_orders: Number.isFinite(n) && n > 0 ? n : 1 });
                  }}
                  className="num min-h-11 border-border-strong bg-surface text-slate-100"
                  aria-label="Aynı anda bekleyecek güvenlik emri sayısı"
                />
                <p className="text-[10px] text-slate-400">
                  Maks. güvenlik emri: {maxSafety} — üst sınır otomatik kelepirlenir.
                </p>
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="risk-dd-breaker" className="text-xs text-slate-400">
                  Günlük Devre Kesici Eşiği: % <span className="text-slate-500">(0 = Devre Dışı)</span>
                </Label>
                <Input
                  id="risk-dd-breaker"
                  type="number"
                  min={0}
                  max={100}
                  step={0.5}
                  value={riskPolicy.daily_dd_breaker_pct}
                  onChange={(e) =>
                    patchRiskPolicy({ daily_dd_breaker_pct: e.target.value === "" ? "0" : e.target.value })
                  }
                  className="num min-h-11 border-border-strong bg-surface text-slate-100"
                  aria-label="Günlük devre kesici yüzdesi"
                />
                <p className="text-[10px] text-slate-400">
                  Günlük drawdown bu eşiği aşınca bot yeni emir üretmeyi keser. Default: %2.
                </p>
              </div>
            </div>

            {/* d) dolum gerçekçiliği */}
            <div className="space-y-1.5">
              <Label className="text-xs text-slate-400">Dolum Gerçekçiliği</Label>
              <div className="grid grid-cols-2 gap-2">
                {(
                  [
                    { v: "REALISTIC", t: "GERÇEKÇİ", s: "fitil 1 tick aşmalı" },
                    { v: "OPTIMISTIC", t: "İYİMSER", s: "teğet fitil dolar" },
                  ] as const
                ).map((o) => (
                  <button
                    key={o.v}
                    onClick={() => patchRiskPolicy({ fill_model: o.v })}
                    aria-pressed={riskPolicy.fill_model === o.v}
                    className={cn(
                      "min-h-11 rounded-lg border px-2 py-2 text-center transition-colors",
                      riskPolicy.fill_model === o.v
                        ? o.v === "REALISTIC"
                          ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300"
                          : "border-amber-500/60 bg-amber-500/10 text-amber-300"
                        : "border-border bg-surface text-slate-500 hover:border-border-strong",
                    )}
                  >
                    <span className="block text-[11px] font-bold leading-tight">{o.t}</span>
                    <span className="block text-[9px] leading-tight opacity-70">{o.s}</span>
                  </button>
                ))}
              </div>
            </div>

            {/* e) piyasa seansı */}
            <div className="space-y-1.5">
              <Label className="text-xs text-slate-400">Piyasa Seansı</Label>
              <div className="grid grid-cols-3 gap-2">
                {SESSION_TYPES.map((s) => (
                  <button
                    key={s.value}
                    onClick={() => patchRiskPolicy({ session_type: s.value })}
                    aria-pressed={riskPolicy.session_type === s.value}
                    className={cn(
                      "min-h-11 rounded-lg border px-2 py-2 text-[11px] font-bold transition-colors",
                      riskPolicy.session_type === s.value
                        ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300"
                        : "border-border bg-surface text-slate-500 hover:border-border-strong",
                    )}
                  >
                    {s.label}
                  </button>
                ))}
              </div>
              <p className="text-[10px] text-slate-400">
                Seans takvimi Gate 7 ve Gate 8a tarafından kullanılır — Kripto 24/7 dışında seans koruması ZORUNLU
                modda piyasa kapalıyken alımı bekletir.
              </p>
            </div>

            {/* f) canlı uyarı paneli + kullanıcı onayı */}
            <div
              className={cn(
                "rounded-lg border p-3",
                needsAck ? "border-amber-500/50 bg-amber-500/5" : "border-border bg-surface",
              )}
              role="status"
            >
              <div
                className={cn(
                  "flex items-center gap-2 text-xs font-bold",
                  needsAck ? "text-amber-300" : "text-slate-400",
                )}
              >
                {needsAck ? <TriangleAlert className="h-4 w-4" aria-hidden /> : <ShieldCheck className="h-4 w-4" aria-hidden />}
                STRATEJİ RİSK VE UYARI BİLDİRİMİ
                {needsAck && (
                  <span className="rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-300">
                    {displayWarnings.length} UYARI
                  </span>
                )}
              </div>
              {displayWarnings.length === 0 ? (
                <p className="mt-2 text-[11px] text-slate-400">
                  Aktif uyarı yok — tüm korumalar ZORUNLU modda ve riskli modlar kapalı. Onay kutusu gerekmez.
                </p>
              ) : (
                <>
                  <ul className="mt-2 space-y-1.5">
                    {displayWarnings.map((w) => (
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
                  <label
                    htmlFor="risk-ack"
                    className="mt-3 flex cursor-pointer items-start gap-2 rounded-md border border-amber-500/30 bg-surface p-2.5"
                  >
                    <Checkbox
                      id="risk-ack"
                      checked={riskAck}
                      onCheckedChange={(v) => setRiskAck(v === true)}
                      className="mt-0.5 border-amber-500/60 data-[state=checked]:border-amber-500 data-[state=checked]:bg-amber-500 data-[state=checked]:text-[#1c1204]"
                      aria-label="Riskleri okudum ve onaylıyorum"
                    />
                    <span className="text-[11px] leading-snug text-amber-200/90">
                      Yukarıdaki riskleri okudum, anladım ve sorumluluğu üstleniyorum. (Kullanıcı Kendi Riskini Taşır)
                    </span>
                  </label>
                  {serverWarnings.length > 0 && (
                    <p className="mt-1.5 text-[10px] text-slate-400">
                      Motor 422 yanıtı uyarı listesini doğruladı — onaydan sonra BOTU BAŞLAT&apos;a tekrar basın.
                    </p>
                  )}
                </>
              )}
              {certGateFailed && (
                <>
                  <div className="rounded-md border border-rose-500/30 bg-rose-500/5 p-2.5">
                    <p className="text-[11px] font-semibold text-rose-200">
                      LIVE sertifika kapısı: KALDI
                    </p>
                    <p className="mt-1 line-clamp-3 text-[10px] leading-snug text-rose-200/70" title={certGateFailed}>
                      {certGateFailed}
                    </p>
                  </div>
                  <label
                    htmlFor="cert-ack"
                    className="mt-2 flex cursor-pointer items-start gap-2 rounded-md border border-rose-500/30 bg-surface p-2.5"
                  >
                    <Checkbox
                      id="cert-ack"
                      checked={certAck}
                      onCheckedChange={(v) => setCertAck(v === true)}
                      className="mt-0.5 border-rose-500/60 data-[state=checked]:border-rose-500 data-[state=checked]:bg-rose-500 data-[state=checked]:text-white"
                      aria-label="Sertifikasız stratejiyi bilinçli onaylıyorum"
                    />
                    <span className="text-[11px] leading-snug text-rose-200/90">
                      Sertifikasyon eşiklerini geçemediğimi biliyorum; bu stratejiyi canlıya bilinçli olarak almakta ısrar ediyorum ve tüm riski üstleniyorum. (Onay kalıcı olarak olay kaydına yazılır.)
                    </span>
                  </label>
                  <p className="mt-1.5 text-[10px] text-slate-400">
                    Öneri: Backtest Lab&apos;de örneklem/WFE/DSR eşiklerini geçen bir config bulun — onaysız geçiş her zaman daha güvenlidir.
                  </p>
                </>
              )}
            </div>
          </div>
        </WizardSection>
      </div>

      {/* ============ RIGHT: sticky live preview (chart + upcoming deal) ============ */}
      <aside className="min-w-0 space-y-4 lg:sticky lg:top-20 lg:self-start" aria-label="Canlı önizleme">
        <Panel title="Canlı Grafik" icon={<ChartCandlestick className="h-4 w-4 text-slate-500" aria-hidden />} bodyClassName="p-2">
          {symbol ? (
            <TvChart symbol={symbol} interval={intervalSel} compact height={280} bars={180} priceLines={chartLines} />
          ) : (
            <div className="flex h-[280px] flex-col items-center justify-center gap-2 text-center">
              <ChartCandlestick className="h-8 w-8 text-slate-700" aria-hidden />
              <p className="max-w-[230px] text-xs text-slate-500">
                Grafik için 1-2. bölümlerden strateji ve sembol seçin — planlanan seviyeler mumların üzerine çizilir.
              </p>
            </div>
          )}
          {chartLines.length > 0 && (
            <div className="flex flex-wrap gap-1 px-1 pt-2">
              <span className="rounded bg-secondary px-1.5 py-0.5 text-[10px] text-slate-400">{chartLines.length} planlanan seviye grafikte işaretli</span>
            </div>
          )}
        </Panel>

        <Panel title="Açılacak Deal" icon={<Rocket className="h-4 w-4 text-slate-500" aria-hidden />}>
          {!kind || !symbol ? (
            <p className="text-xs text-slate-500">Strateji ve sembol seçildiğinde deal önizlemesi burada canlı hesaplanır.</p>
          ) : isGridKind ? (
            <div className="grid grid-cols-2 gap-2">
              <Stat label="Aralık" value={gridLower && gridUpper ? `${gridLower} – ${gridUpper}` : "—"} />
              <Stat label="Seviye" value={String(gridLevels)} />
              <Stat label="Seviye Notional" value={fmtUsd(fracToNum(gridNotional) ?? 0)} />
              <Stat label="Toplam Yatırım" value={calcGrid ? fmtUsd(calcGrid.totalInvested) : "—"} />
              <Stat label="Mod" value={gridMode} />
              <Stat label="Fee-Breaker" value={calcGrid ? (calcGrid.feeBreakerPass ? "GEÇTİ" : "KALDI") : "—"} tone={calcGrid?.feeBreakerPass ? "emerald" : "rose"} />
            </div>
          ) : (
            <div className="grid grid-cols-2 gap-2">
              <Stat label="Base Emir" value={fmtUsd(fracToNum(baseSize) ?? 0)} />
              <Stat label="Toplam Notional" value={calcDca ? fmtUsd(calcDca.totalNotionalDisplay ?? calcDca.totalNotional) : "—"} />
              <Stat label="Toplam Miktar" value={calcDca ? fmtFrac(calcDca.totalQty) : "—"} />
              <Stat label="Ort. Giriş (WAC)" value={calcDca ? fmtFrac(calcDca.avgAllIn) : "—"} />
              <Stat label="Güvenlik Emri" value={String(maxSafety)} />
              <Stat label="TP Hedefi" value={`${targetProfitPct}%${netTp ? " (net)" : ""}`} tone="emerald" />
              <Stat label="Tahmini Ücret" value={calcDca ? fmtUsd(calcDca.estimatedFees, 4) : "—"} tone="amber" />
              <Stat
                label={isFuturesDca ? "Başlangıç Marjı" : "Kaldıraç"}
                value={isFuturesDca ? (calcDca ? fmtUsd(calcDca.estimatedInitialMargin) : "—") : `${leverage}x`}
                tone="cyan"
              />
            </div>
          )}

          {/* entry status chips */}
          <div className="mt-3 flex flex-wrap gap-1.5">
            <span className="rounded border border-border-strong bg-surface px-2 py-1 font-mono text-[10px] text-slate-300">
              {describeTrigger(scMode, scPrice, scRef, scDropPct, scRsi, scDateTime)}
            </span>
            {scMetNow === true && <span className="rounded bg-emerald-500/15 px-2 py-1 text-[10px] font-bold text-emerald-300">KOŞUL SAĞLANIR</span>}
            {scMetNow === false && <span className="rounded bg-amber-500/15 px-2 py-1 text-[10px] font-bold text-amber-300">KOŞUL BEKLİYOR</span>}
            {signalCfg && <span className="rounded bg-cyan-500/15 px-2 py-1 text-[10px] font-bold text-cyan-300">SİNYAL MOTORU AKTİF</span>}
            {calcBusy && <span className="rounded bg-secondary px-2 py-1 text-[10px] text-slate-400">HESAPLANIYOR…</span>}
          </div>

          {launchError && (
            <div className="mt-3 rounded-lg border border-rose-500/50 bg-rose-500/10 p-3">
              <div className="flex items-center gap-2 text-sm font-bold text-rose-400">
                <AlertTriangle className="h-4 w-4" aria-hidden />
                Motor reddetti (HTTP 422)
              </div>
              <pre className="thin-scroll mt-2 max-h-32 overflow-auto whitespace-pre-wrap font-mono text-xs text-rose-300">{launchError}</pre>
            </div>
          )}

          <div className="mt-3 space-y-2.5">
            <div className="space-y-1.5">
              <Label htmlFor="bot-name" className="text-xs text-slate-400">
                Bot Adı
              </Label>
              <Input
                id="bot-name"
                value={botName}
                onChange={(e) => setBotName(e.target.value)}
                placeholder={`${symbol ?? "BTC"} ${kind ?? "DCA"} ${mode}`}
                className="border-border-strong bg-surface text-slate-100 placeholder:text-slate-500"
              />
            </div>
            <Tooltip>
              <TooltipTrigger asChild>
                <span className={cn("block", needsAck && !riskAck && "cursor-not-allowed")}>
                  <Button
                    onClick={() => void launch()}
                    disabled={launching || !canLaunch}
                    aria-label="Botu başlat"
                    className="min-h-11 w-full bg-emerald-500 px-6 font-bold text-[#04150e] hover:bg-emerald-400"
                  >
                    {launching ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <Rocket className="h-4 w-4" aria-hidden />}
                    {launching ? "Launch ediliyor…" : "BOTU BAŞLAT"}
                  </Button>
                </span>
              </TooltipTrigger>
              {needsAck && !riskAck && (
                <TooltipContent side="top" className="border-amber-500/50 bg-[#1c1608] text-xs text-amber-200">
                  Önce riskleri onaylayın
                </TooltipContent>
              )}
            </Tooltip>
            {!canLaunch && (
              <p className="text-center text-[10px] text-slate-400">
                {needsAck && !riskAck
                  ? "Önce 6. bölümdeki onay kutusuyla riskleri onaylayın."
                  : "Tüm bölümler geçerli olunca aktif olur."}
              </p>
            )}
          </div>
        </Panel>
      </aside>
    </div>
  );
}

