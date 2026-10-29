"use client";

import { useEffect, useMemo, useRef, useState } from "react";
import { BadgeCheck, FlaskConical, Loader2, Play, Send, ShieldAlert, Sparkles, X } from "lucide-react";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Panel } from "@/components/terminal/bits";
import { BacktestResultView, defaultDcaConfig } from "@/components/terminal/backtest-panel";
import { apiAiDiagnose, apiAiProviders, apiBacktest, apiBacktestCert, apiCalibration, apiFuturesState, apiMarkets, apiNews, apiNewsAi, apiOrderbook, apiRegime, useBots } from "@/hooks/use-engine";
import { cn } from "@/lib/utils";
import { DEFAULT_RISK_POLICY } from "@/lib/engine-types";
import { saveCarriedConfig } from "@/lib/carry-config";
import type { AiCallConfig, AiProvider, AiResult, BacktestCertResult, BacktestResult, BotKind, CalibrationResult, FillModel, FuturesStateResult, Instrument, Interval, NewsAiResult, NewsResult, OrderBookResult, RegimeResult, Venue } from "@/lib/engine-types";

const INTERVALS: Interval[] = ["1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "1d"];

function certBadge(label: string, value: string, tone: "good" | "warn" | "bad" | "info", hint: string) {
  return (
    <div className="rounded-lg border border-border bg-surface p-3">
      <div className="flex items-center justify-between gap-2">
        <span className="text-[10px] font-bold tracking-wider text-slate-400">{label}</span>
        <span
          className={cn(
            "num rounded px-1.5 py-0.5 font-mono text-xs font-bold",
            tone === "good" && "bg-emerald-500/15 text-emerald-300",
            tone === "warn" && "bg-amber-500/15 text-amber-300",
            tone === "bad" && "bg-rose-500/15 text-rose-300",
            tone === "info" && "bg-cyan-500/15 text-cyan-300",
          )}
        >
          {value}
        </span>
      </div>
      <p className="mt-1 text-[10px] leading-tight text-slate-400">{hint}</p>
    </div>
  );
}

export function BacktestLab({ onGoWizard }: { onGoWizard?: () => void } = {}) {
  const [kind, setKind] = useState<BotKind>("DCA_LONG");
  const [symbol, setSymbol] = useState("BTCUSDT");
  const [interval, setInterval] = useState<Interval>("1h");
  /* Borsa-içi sembol seçici: venue+kategori Sembol hücresinin içinden; ayrı alan yok. */
  const [venue, setVenue] = useState<Venue>("BYBIT");
  const [category, setCategory] = useState<"spot" | "linear">("spot");
  const [instruments, setInstruments] = useState<Instrument[]>([]);
  const [marketsBusy, setMarketsBusy] = useState(false);
  const [dropOpen, setDropOpen] = useState(false);
  const [bars, setBars] = useState(500);
  const [startCash, setStartCash] = useState("100000");
  const [deviation, setDeviation] = useState(2);
  const [maxSafety, setMaxSafety] = useState(5);
  const [tp, setTp] = useState(2);
  const [baseSize, setBaseSize] = useState("500");
  const [leverage, setLeverage] = useState("5");
  /* Task 16: Gate 5 dolum gerçekçiliği — REALISTIC (1-tick penetrasyon şartı) vs OPTIMISTIC (teğet fitil dolar). */
  const [fillModel, setFillModel] = useState<FillModel>("REALISTIC");

  /* Tür değişince kategori önerisi (kullanıcı elle değiştirebilir). */
  useEffect(() => {
    setCategory(kind === "DCA_SHORT" || kind === "FUTURES_DCA" ? "linear" : "spot");
  }, [kind]);

  /* Borsa listesi: venue+kategoriye göre canlı enstrümanlar (Bybit Linear ~500 çift). */
  useEffect(() => {
    let alive = true;
    setMarketsBusy(true);
    apiMarkets(category, undefined, venue)
      .then((r) => {
        if (alive) setInstruments(r.instruments);
      })
      .catch(() => {
        if (alive) setInstruments([]);
      })
      .finally(() => {
        if (alive) setMarketsBusy(false);
      });
    return () => {
      alive = false;
    };
  }, [venue, category]);

  const selectedInstrument = useMemo(
    () => instruments.find((i) => i.symbol === symbol.trim().toUpperCase()) ?? null,
    [instruments, symbol],
  );

  const symbolMatches = useMemo(() => {
    const q = symbol.trim().toUpperCase();
    if (!q) return instruments.slice(0, 8);
    return instruments.filter((i) => i.symbol.includes(q) || i.base.includes(q)).slice(0, 50);
  }, [instruments, symbol]);

  /* P3-2 rejim rozeti — sembol/periyot değişince motor sınıflandırıcısı sorulur. */
  const [regime, setRegime] = useState<RegimeResult | null>(null);
  useEffect(() => {
    let alive = true;
    const sym = symbol.trim().toUpperCase() || "BTCUSDT";
    apiRegime(sym, interval, 300)
      .then((r) => { if (alive) setRegime(r); })
      .catch(() => { if (alive) setRegime(null); });
    return () => { alive = false; };
  }, [symbol, interval]);
  /* P3-3 kalibrasyon — oynaklıktan sapma adımı önerisi. */
  const [cal, setCal] = useState<CalibrationResult | null>(null);
  const [calBusy, setCalBusy] = useState(false);
  const [calError, setCalError] = useState<string | null>(null);
  const runCalibration = async () => {
    setCalBusy(true);
    setCalError(null);
    try {
      const r = await apiCalibration(symbol.trim().toUpperCase() || "BTCUSDT", interval, 500);
      setCal(r);
      const sug = parseFloat(r.suggestedDeviationPct);
      if (Number.isFinite(sug)) setDeviation(Math.min(10, Math.max(0.5, Math.round(sug * 10) / 10)));
    } catch (e) {
      setCalError(e instanceof Error ? e.message : "Kalibrasyon çalışmadı");
    } finally {
      setCalBusy(false);
    }
  };

  /* P3-6 orderbook L2 — venue+kategori değişince tazelenir. */
  const [ob, setOb] = useState<OrderBookResult | null>(null);
  useEffect(() => {
    let alive = true;
    const sym = symbol.trim().toUpperCase() || "BTCUSDT";
    apiOrderbook(sym, venue, category, 25)
      .then((r) => { if (alive) setOb(r); })
      .catch(() => { if (alive) setOb(null); });
    return () => { alive = false; };
  }, [symbol, venue, category]);

  /* P4-3d vadeli durum kartı — yalnız FUTURES_DCA seçiliyken (BYBIT/OKX tam kart; BINANCE dürüst 422 → kart gizlenir). */
  const [fs, setFs] = useState<FuturesStateResult | null>(null);
  useEffect(() => {
    if (kind !== "FUTURES_DCA") { setFs(null); return; }
    let alive = true;
    const sym = symbol.trim().toUpperCase() || "BTCUSDT";
    apiFuturesState(sym, venue)
      .then((r) => { if (alive) setFs(r); })
      .catch(() => { if (alive) setFs(null); });
    return () => { alive = false; };
  }, [kind, symbol, venue]);

  /* P4-1 haberler — sembol değişince tazelenir. */
  const [news, setNews] = useState<NewsResult | null>(null);
  /* P4-2 haber TR-özetleri — öğe kimliğine göre (sekme içi, kalıcı değil). */
  const [aiSums, setAiSums] = useState<Record<string, NewsAiResult>>({});
  const [aiSumBusy, setAiSumBusy] = useState<string | null>(null);
  /* P4-2 YZ sağlayıcı yapılandırması — sekme içi state; anahtar diske yazılmaz. */
  const [aiProviders, setAiProviders] = useState<AiProvider[]>([]);
  const [aiProvider, setAiProvider] = useState("builtin");
  const [aiModel, setAiModel] = useState("");
  const [aiKey, setAiKey] = useState("");
  const [aiBaseUrl, setAiBaseUrl] = useState("");
  useEffect(() => {
    let alive = true;
    apiAiProviders()
      .then((r) => { if (alive) setAiProviders(r.providers); })
      .catch(() => { if (alive) setAiProviders([]); });
    return () => { alive = false; };
  }, []);
  const aiConfig = useMemo<AiCallConfig>(
    () => ({
      provider: aiProvider,
      model: aiModel.trim() || undefined,
      apiKey: aiKey || undefined,
      baseUrl: aiBaseUrl.trim() || undefined,
    }),
    [aiProvider, aiModel, aiKey, aiBaseUrl],
  );
  const runNewsAi = async (id: string) => {
    setAiSumBusy(id);
    try {
      const r = await apiNewsAi(id, aiConfig);
      setAiSums((m) => ({ ...m, [id]: r }));
    } catch (e) {
      setAiSums((m) => ({ ...m, [id]: { ok: false, summaryTr: null, reason: e instanceof Error ? e.message : "Özet alınamadı", cached: false } }));
    } finally {
      setAiSumBusy(null);
    }
  };
  useEffect(() => {
    let alive = true;
    const sym = symbol.trim().toUpperCase() || "BTCUSDT";
    apiNews(sym)
      .then((r) => { if (alive) setNews(r); })
      .catch(() => { if (alive) setNews(null); });
    return () => { alive = false; };
  }, [symbol]);

  /* P4-2 YZ Yardımcı paneli — sohbet + bot teşhisi (tümü salt-okunur bağlam). */
  const [aiMode, setAiMode] = useState<"chat" | "diagnose">("chat");
  const [aiBot, setAiBot] = useState("");
  const [aiMsg, setAiMsg] = useState("");
  const [aiHist, setAiHist] = useState<{ role: string; content: string }[]>([]);
  const [aiRes, setAiRes] = useState<AiResult | null>(null);
  const [aiBusy, setAiBusy] = useState(false);
  const [aiErr, setAiErr] = useState<string | null>(null);
  const { bots } = useBots();
  const askAi = async () => {
    setAiBusy(true);
    setAiErr(null);
    try {
      const sym = symbol.trim().toUpperCase() || "BTCUSDT";
      const body = aiMode === "chat"
        ? { mode: "chat" as const, message: aiMsg, history: aiHist.slice(-6), symbol: sym, config: aiConfig }
        : { mode: "diagnose" as const, bot_id: aiBot || undefined, symbol: sym, config: aiConfig };
      if (aiMode === "chat" && !aiMsg.trim()) throw new Error("Soru boş");
      if (aiMode === "diagnose" && !aiBot) throw new Error("Teşhis için bot seçin");
      const r = await apiAiDiagnose(body);
      setAiRes(r);
      if (aiMode === "chat" && r.ok && r.text) {
        setAiHist((h) => [...h.slice(-7), { role: "user", content: aiMsg }, { role: "assistant", content: r.text ?? "" }]);
        setAiMsg("");
      }
    } catch (e) {
      setAiRes(null);
      setAiErr(e instanceof Error ? e.message : "YZ çağrısı başarısız");
    } finally {
      setAiBusy(false);
    }
  };
  const aiProviderInfo = aiProviders.find((p) => p.id === aiProvider) ?? null;

  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<BacktestResult | null>(null);
  /* P5 — faz çubuğu + iptal: tek uçuşan istek, istemci faz etiketleri (yüzde uydurulmaz). */
  const [op, setOp] = useState<{ kind: "backtest" | "cert"; phase: string } | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const PHASES = ["Kline çekiliyor…", "Motor koşuyor (SAF-G2 + kapılar)…", "Skorlar hesaplanıyor…"];
  const startOp = (kind: "backtest" | "cert") => {
    abortRef.current?.abort();
    const ctrl = new AbortController();
    abortRef.current = ctrl;
    let i = 0;
    setOp({ kind, phase: PHASES[0] });
    /* Not: `setInterval` adı periyot state'iyle gölgelenir — globale açıkça inilir. */
    const t = window.setInterval(() => {
      i = (i + 1) % PHASES.length;
      setOp((o) => (o && o.kind === kind ? { ...o, phase: PHASES[i] } : o));
    }, 5000);
    return { ctrl, stop: () => window.clearInterval(t) };
  };
  const endOp = (ctrl: AbortController) => {
    if (abortRef.current === ctrl) {
      abortRef.current = null;
      setOp(null);
    }
  };
  const [certBusy, setCertBusy] = useState(false);
  const [certError, setCertError] = useState<string | null>(null);
  const [cert, setCert] = useState<BacktestCertResult | null>(null);

  const buildConfig = () => {
    /* P4-3d: koşu bu venue'ün market verisiyle yapılır (run/runCert'te venue parametresi). */
    const config = defaultDcaConfig(kind, symbol.trim().toUpperCase() || "BTCUSDT");
    config.deviation_pct = String(deviation / 100);
    config.max_safety_orders = maxSafety;
    config.target_profit_pct = String(tp / 100);
    config.base_order_size = baseSize || "500";
    if (kind === "FUTURES_DCA") config.leverage = leverage || "5";
    config.risk_policy = { ...DEFAULT_RISK_POLICY, fill_model: fillModel };
    return config;
  };

  const run = async () => {
    const { ctrl, stop } = startOp("backtest");
    setBusy(true);
    setError(null);
    try {
      const config = buildConfig();
      const r = await apiBacktest(
        {
          kind,
          symbol: symbol.trim().toUpperCase() || "BTCUSDT",
          interval,
          bars,
          start_cash: startCash || "100000",
          config,
          venue,
        },
        ctrl.signal,
      );
      if (!ctrl.signal.aborted) setResult(r);
    } catch (e) {
      if (ctrl.signal.aborted) {
        setError("Koşu iptal edildi — yarım sonuç diske yazılmadı.");
      } else {
        setResult(null);
        setError(e instanceof Error ? e.message : "Bilinmeyen hata");
      }
    } finally {
      stop();
      setBusy(false);
      endOp(ctrl);
    }
  };

  const runCert = async () => {
    const { ctrl, stop } = startOp("cert");
    setCertBusy(true);
    setCertError(null);
    try {
      const config = buildConfig();
      const r = await apiBacktestCert(
        {
          kind,
          symbol: symbol.trim().toUpperCase() || "BTCUSDT",
          interval,
          bars,
          start_cash: startCash || "100000",
          config,
          mc_runs: 200,
          venue,
        },
        ctrl.signal,
      );
      if (!ctrl.signal.aborted) setCert(r);
    } catch (e) {
      if (ctrl.signal.aborted) {
        setCertError("Sertifika koşusu iptal edildi.");
      } else {
        setCert(null);
        setCertError(e instanceof Error ? e.message : "Bilinmeyen hata");
      }
    } finally {
      stop();
      setCertBusy(false);
      endOp(ctrl);
    }
  };

  return (
    <div className="space-y-4">
      <Panel title="Backtest Lab" icon={<FlaskConical className="h-4 w-4 text-slate-500" aria-hidden />}>
        <div className="grid gap-4 lg:grid-cols-4">
          <div className="space-y-1.5">
            <Label className="text-xs text-slate-400">Strateji</Label>
            <div className="flex gap-1.5">
              {(["DCA_LONG", "DCA_SHORT", "FUTURES_DCA"] as BotKind[]).map((k) => (
                <button
                  key={k}
                  onClick={() => setKind(k)}
                  className={cn(
                    "min-h-11 flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold",
                    kind === k
                      ? k === "DCA_LONG"
                        ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300"
                        : k === "FUTURES_DCA"
                          ? "border-amber-500/60 bg-amber-500/10 text-amber-300"
                          : "border-rose-500/60 bg-rose-500/10 text-rose-300"
                      : "border-border text-slate-500 hover:border-border-strong",
                  )}
                >
                  {k === "DCA_LONG" ? "DCA LONG" : k === "FUTURES_DCA" ? "FUT. DCA" : "DCA SHORT"}
                </button>
              ))}
            </div>
          </div>
          <div className="space-y-1.5">
            <div className="flex items-center justify-between">
              <Label htmlFor="bt-symbol" className="text-xs text-slate-400">
                Sembol & Borsa
              </Label>
              <span className="num font-mono text-[10px] text-slate-400">
                {marketsBusy ? "yükleniyor…" : `${instruments.length} çift`}
              </span>
            </div>
            {/* Borsa listesi: ayrı alan yok — venue+kategori bu hücrenin içinden */}
            <div className="flex gap-1">
              {(["BYBIT", "BINANCE", "OKX"] as Venue[]).map((v) => (
                <button
                  key={v}
                  type="button"
                  onClick={() => setVenue(v)}
                  aria-pressed={venue === v}
                  className={cn(
                    "min-h-9 flex-1 rounded-md border px-1.5 py-1 text-[10px] font-bold transition-colors",
                    venue === v
                      ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300"
                      : "border-border text-slate-500 hover:border-border-strong",
                  )}
                >
                  {v}
                </button>
              ))}
              {(["spot", "linear"] as const).map((c) => (
                <button
                  key={c}
                  type="button"
                  onClick={() => setCategory(c)}
                  aria-pressed={category === c}
                  title={c === "linear" ? "Vadeli (Bybit Linear ~500 çift)" : "Spot"}
                  className={cn(
                    "min-h-9 flex-1 rounded-md border px-1.5 py-1 text-[10px] font-bold uppercase transition-colors",
                    category === c
                      ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300"
                      : "border-border text-slate-500 hover:border-border-strong",
                  )}
                >
                  {c === "linear" ? "LINEAR" : "SPOT"}
                </button>
              ))}
            </div>
            <div className="relative">
              <Input
                id="bt-symbol"
                value={symbol}
                onChange={(e) => {
                  setSymbol(e.target.value);
                  setDropOpen(true);
                }}
                onFocus={() => setDropOpen(true)}
                onBlur={() => setTimeout(() => setDropOpen(false), 150)}
                className="num min-h-11 border-border-strong bg-surface font-mono text-slate-100"
                placeholder="ör. BTC, ETH, SOL…"
                autoComplete="off"
              />
              {dropOpen && symbolMatches.length > 0 && (
                <div className="absolute z-20 mt-1 max-h-56 w-full overflow-y-auto rounded-lg border border-border-strong bg-surface shadow-xl">
                  {symbolMatches.map((i) => (
                    <button
                      key={i.symbol}
                      type="button"
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => {
                        setSymbol(i.symbol);
                        setDropOpen(false);
                      }}
                      className={cn(
                        "flex w-full items-center justify-between px-3 py-2 font-mono text-xs transition-colors hover:bg-[#1a2233]",
                        i.symbol === symbol.trim().toUpperCase() ? "text-emerald-300" : "text-slate-200",
                      )}
                    >
                      <span>{i.symbol}</span>
                      <span className="text-[10px] text-slate-400">
                        {i.quote} · {i.max_leverage}x
                      </span>
                    </button>
                  ))}
                </div>
              )}
            </div>
            {/* P3-6 derinlik (L2 anlık görüntü) */}
            {ob && (
              <div className="num flex flex-wrap items-center gap-x-2 gap-y-1 font-mono text-[10px] text-slate-400" title={ob.note}>
                <span className="text-slate-500">Derinlik:</span>
                <span className="text-emerald-300">{ob.bestBid}</span>
                <span className="text-slate-500">/</span>
                <span className="text-rose-300">{ob.bestAsk}</span>
                <span className="text-slate-500">spread {ob.spreadBps}bp</span>
                <span
                  className={cn(
                    "rounded px-1 py-0.5 font-bold",
                    parseFloat(ob.imbalance) > 0.2 && "bg-emerald-500/15 text-emerald-300",
                    parseFloat(ob.imbalance) < -0.2 && "bg-rose-500/15 text-rose-300",
                    Math.abs(parseFloat(ob.imbalance)) <= 0.2 && "bg-slate-500/15 text-slate-300",
                  )}
                  title="+ alım baskısı / − satış baskısı"
                >
                  {parseFloat(ob.imbalance) > 0 ? `+${ob.imbalance}` : ob.imbalance}
                </span>
              </div>
            )}
            {/* P3-2 rejim rozeti */}
            {regime && (
              <div className="flex items-center gap-1.5 text-[10px]">
                <span className="text-slate-500">Rejim:</span>
                <span
                  title={regime.note}
                  className={cn(
                    "num rounded px-1.5 py-0.5 font-mono font-bold",
                    regime.regime === "NORMAL" && "bg-emerald-500/15 text-emerald-300",
                    regime.regime === "HIGH_VOLATILITY" && "bg-amber-500/15 text-amber-300",
                    regime.regime === "CRASH" && "bg-rose-500/15 text-rose-300",
                  )}
                >
                  {regime.regime === "NORMAL" ? "✓ SAKİN" : regime.regime === "HIGH_VOLATILITY" ? "~ HAREKETLİ" : "✕ ÇÖKÜŞ"}
                </span>
                {regime.blocksNewDeals && <span className="text-amber-400/90">yeni deal blokeli</span>}
              </div>
            )}
            {selectedInstrument ? (
              <div className="num flex flex-wrap gap-1 font-mono text-[10px] text-slate-400">
                <span className="rounded bg-[#1a2233] px-1.5 py-0.5">tick {selectedInstrument.tick_size}</span>
                <span className="rounded bg-[#1a2233] px-1.5 py-0.5">step {selectedInstrument.step_size}</span>
                <span className="rounded bg-[#1a2233] px-1.5 py-0.5">minNot {selectedInstrument.min_notional}</span>
                <span className="rounded bg-amber-500/10 px-1.5 py-0.5 text-amber-300">
                  maxLev {selectedInstrument.max_leverage}x
                </span>
              </div>
            ) : (
              instruments.length > 0 && symbolMatches.length === 0 && (
                <p className="text-[10px] text-slate-400">Listede yok — serbest sembol olarak denenecek.</p>
              )
            )}
          </div>
          <div className="space-y-1.5">
            <Label className="text-xs text-slate-400">Periyot</Label>
            <Select value={interval} onValueChange={(v) => setInterval(v as Interval)}>
              <SelectTrigger className="min-h-11 w-full border-border-strong bg-surface text-slate-100">
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
          <div className="space-y-1.5">
            <Label htmlFor="bt-cash" className="text-xs text-slate-400">
              Başlangıç Sermayesi
            </Label>
            <Input
              id="bt-cash"
              type="number"
              value={startCash}
              onChange={(e) => setStartCash(e.target.value)}
              className="num min-h-11 border-border-strong bg-surface text-slate-100"
            />
          </div>
          {kind === "FUTURES_DCA" && (
            <div className="space-y-1.5">
              <Label htmlFor="bt-lev" className="text-xs text-amber-400/90">
                Kaldıraç (izole marjin)
              </Label>
              <Input
                id="bt-lev"
                type="number"
                min={1}
                max={selectedInstrument ? Number(selectedInstrument.max_leverage) || 50 : 50}
                value={leverage}
                onChange={(e) => setLeverage(e.target.value)}
                className="num min-h-11 border-amber-500/40 bg-surface text-amber-200"
              />
            </div>
          )}
          <div className="space-y-1.5">
            <Label className="text-xs text-slate-400">Dolum Modeli (Gate 5)</Label>
            <div className="flex gap-1.5">
              {([
                { v: "REALISTIC" as FillModel, l: "GERÇEKÇİ", t: "Fitil limiti en az 1 tick aşmalı — teğet dokunuş dolmaz (gerçekçi)." },
                { v: "OPTIMISTIC" as FillModel, l: "İYİMSER", t: "Teğet dokunuş dolmuş sayılır — canlıda bu emirlerin çoğu dolmaz." },
              ]).map((o) => (
                <button
                  key={o.v}
                  onClick={() => setFillModel(o.v)}
                  title={o.t}
                  aria-pressed={fillModel === o.v}
                  className={cn(
                    "min-h-11 flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold",
                    fillModel === o.v
                      ? o.v === "REALISTIC"
                        ? "border-emerald-500/60 bg-emerald-500/10 text-emerald-300"
                        : "border-amber-500/60 bg-amber-500/10 text-amber-300"
                      : "border-border text-slate-500 hover:border-border-strong",
                  )}
                >
                  {o.l}
                </button>
              ))}
            </div>
          </div>
        </div>

        <div className="mt-4 grid gap-4 lg:grid-cols-4">
          <div className="space-y-2 lg:col-span-2">
            <div className="flex justify-between text-xs text-slate-400">
              <Label className="text-xs text-slate-400">Bar sayısı (tarihsel derinlik)</Label>
              <span className="num text-slate-300">{bars}</span>
            </div>
            <Slider value={[bars]} min={100} max={1000} step={100} onValueChange={(v) => setBars(v[0] ?? 500)} aria-label="Bar sayısı" />
          </div>
          <div className="space-y-2">
            <div className="flex items-center justify-between text-xs text-slate-400">
              <Label className="text-xs text-slate-400">Deviation %</Label>
              <span className="flex items-center gap-1.5">
                <span className="num text-slate-300">{deviation.toFixed(1)}%</span>
                <button
                  type="button"
                  onClick={() => void runCalibration()}
                  disabled={calBusy}
                  title="Son 500 barın oynaklığından (2σ) başlangıç adımı öner — kaydırıcıyı otomatik ayarlar"
                  className="rounded border border-cyan-500/40 bg-cyan-500/10 px-1.5 py-0.5 text-[10px] font-bold text-cyan-300 transition-colors hover:bg-cyan-500/20 disabled:opacity-50"
                >
                  {calBusy ? "…" : "Öner"}
                </button>
              </span>
            </div>
            {calError && <p className="text-[10px] text-rose-400">{calError}</p>}
            {cal && (
              <p className="num text-[10px] text-slate-400" title={cal.note}>
                σ %{cal.sigmaPct} · %95 düşüş %{cal.q95DropPct} · öneri %{cal.suggestedDeviationPct} ({cal.bars} bar)
              </p>
            )}
            <Slider value={[deviation]} min={0.5} max={10} step={0.1} onValueChange={(v) => setDeviation(v[0] ?? 2)} aria-label="Deviation" />
          </div>
          <div className="space-y-2">
            <div className="flex justify-between text-xs text-slate-400">
              <Label className="text-xs text-slate-400">Hedef Kâr %</Label>
              <span className="num text-slate-300">{tp.toFixed(1)}%</span>
            </div>
            <Slider value={[tp]} min={0.2} max={10} step={0.1} onValueChange={(v) => setTp(v[0] ?? 2)} aria-label="Hedef kâr" />
          </div>
        </div>

        <div className="mt-4 grid gap-4 lg:grid-cols-4">
          <div className="space-y-2">
            <div className="flex justify-between text-xs text-slate-400">
              <Label className="text-xs text-slate-400">Maks. Güvenlik</Label>
              <span className="num text-slate-300">{maxSafety}</span>
            </div>
            <Slider value={[maxSafety]} min={0} max={10} step={1} onValueChange={(v) => setMaxSafety(v[0] ?? 5)} aria-label="Maks güvenlik" />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="bt-base" className="text-xs text-slate-400">
              Base Emir (USDT)
            </Label>
            <Input
              id="bt-base"
              type="number"
              value={baseSize}
              onChange={(e) => setBaseSize(e.target.value)}
              className="num min-h-11 border-border-strong bg-surface text-slate-100"
            />
          </div>
          <div className="flex items-end gap-2 lg:col-span-2">
            <Button onClick={() => void run()} disabled={busy} className="min-h-11 flex-1 bg-cyan-500/90 font-bold text-[#02171b] hover:bg-cyan-500 lg:w-36">
              {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <Play className="h-4 w-4" aria-hidden />}
              {busy ? "Koşuyor…" : "BACKTEST"}
            </Button>
            <Button
              onClick={() => void runCert()}
              disabled={certBusy}
              variant="outline"
              className="min-h-11 flex-1 border-amber-500/50 bg-amber-500/10 font-bold text-amber-300 hover:bg-amber-500/20 hover:text-amber-200 lg:w-44"
            >
              {certBusy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <BadgeCheck className="h-4 w-4" aria-hidden />}
              {certBusy ? "Sertifikalanıyor…" : "SERTİFİKALA"}
            </Button>
            {/* Faz 19 (UIX-11): Lab'de beğenilen koşuyu tek tıkla Sihirbaz'a taşı —
                config session-storage'a yazılır, Sihirbaz mount'ta tüketip
                alanları prefill eder (tek seferlik, 10 dk tazelik penceresi). */}
            {onGoWizard && (
              <Button
                onClick={() => {
                  const ok = saveCarriedConfig({
                    source: "lab",
                    kind,
                    symbol: symbol.trim().toUpperCase() || "BTCUSDT",
                    interval,
                    venue,
                    startCash: startCash || "100000",
                    config: buildConfig() as unknown as Record<string, unknown>,
                  });
                  if (ok) onGoWizard();
                }}
                variant="outline"
                title="Bu ayarlarla Sihirbaz'ı aç (alanlar otomatik doldurulur)"
                className="min-h-11 flex-1 border-violet-500/50 bg-violet-500/10 font-bold text-violet-300 hover:bg-violet-500/20 hover:text-violet-200 lg:w-44"
              >
                <Send className="h-4 w-4" aria-hidden />
                SİHİRBAZA TAŞI
              </Button>
            )}
          </div>
        </div>

        {/* P5 — faz çubuğu + iptal + skeleton (uzun koşuda kör bekleme yok). */}
        {(busy || certBusy) && op && (
          <div className="mt-3 rounded-lg border border-border bg-surface p-3" role="status">
            <div className="flex items-center gap-2">
              <Loader2 className="h-4 w-4 shrink-0 animate-spin text-cyan-400" aria-hidden />
              <span className="min-w-0 flex-1 truncate text-xs text-slate-300">
                {op.kind === "backtest" ? "Backtest: " : "Sertifika: "}{op.phase}
              </span>
              <Button
                type="button"
                variant="outline"
                onClick={() => abortRef.current?.abort()}
                className="min-h-9 shrink-0 border-rose-500/50 px-3 text-xs text-rose-300 hover:bg-rose-500/10"
              >
                <X className="h-3.5 w-3.5" aria-hidden />
                İptal
              </Button>
            </div>
            <div
              role="progressbar"
              aria-label={op.kind === "backtest" ? "Backtest ilerlemesi" : "Sertifika ilerlemesi"}
              aria-valuetext={op.phase}
              className="mt-2 h-1.5 overflow-hidden rounded-full bg-secondary"
            >
              <div className="h-full w-1/3 animate-pulse rounded-full bg-cyan-400" />
            </div>
          </div>
        )}
        {(busy || certBusy) && !result && !cert && (
          <div className="mt-3 space-y-2" aria-hidden>
            <Skeleton className="h-14 w-full bg-secondary" />
            <Skeleton className="h-28 w-full bg-secondary" />
          </div>
        )}

        {/* P3-7 vadeli durum kartı */}
        {kind === "FUTURES_DCA" && fs && (
          <div className="num mt-3 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-amber-500/25 bg-amber-500/5 px-3 py-2 font-mono text-[11px] text-slate-300" title={fs.note}>
            <span className="font-bold text-amber-300">VADELİ {fs.symbol}</span>
            <span>maxLev {fs.maxLeverage ?? "—"}x</span>
            <span>tick {fs.tickSize ?? "—"}</span>
            <span>minNot {fs.minNotional ?? "—"}</span>
            {fs.live.supported ? (
              <span className={fs.live.positions && fs.live.positions.length > 0 ? "text-rose-300" : "text-emerald-300"}>
                {fs.live.positions && fs.live.positions.length > 0
                  ? `açık: ${fs.live.positions.map((p) => `${p.side} ${p.size} @ ${p.avgPrice} (${p.leverage}x)`).join(" · ")}`
                  : "açık pozisyon yok"}
              </span>
            ) : (
              <span className="text-slate-500">anahtar yok — pozisyon bacağı atlandı</span>
            )}
          </div>
        )}
        {/* P4-1 haberler */}
        {news && news.count > 0 && (
          <div className="mt-3 rounded-lg border border-border bg-surface p-3">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[10px] font-bold uppercase tracking-wider text-slate-400">Haberler · {news.symbol}</span>
              <span className={cn("num rounded px-1.5 py-0.5 font-mono text-[11px] font-bold", news.mean > 0.2 ? "bg-emerald-500/15 text-emerald-300" : news.mean < -0.2 ? "bg-rose-500/15 text-rose-300" : "bg-slate-500/15 text-slate-300")}>
                {news.mean > 0 ? `+${news.mean}` : news.mean} ({news.bull}↑ {news.bear}↓)
              </span>
              <span className="text-[10px] text-slate-400">{news.sources.join(" · ")}{news.cached ? " · önbellek" : ""}</span>
            </div>
            {news.delisting.active && (
              <p className="mt-1.5 rounded border border-rose-500/40 bg-rose-500/10 px-2 py-1 text-[11px] font-bold text-rose-300">
                DELISTING UYARISI: {news.delisting.headline}
              </p>
            )}
            <ul className="mt-2 max-h-40 space-y-1 overflow-y-auto">
              {news.items.slice(0, 8).map((n) => (
                <li key={n.id}>
                  <div className="flex items-start gap-2 text-[11px]">
                    <span className={cn("mt-0.5 shrink-0 rounded px-1 py-0.5 font-mono text-[10px] font-bold", n.sentiment > 0.2 ? "bg-emerald-500/15 text-emerald-300" : n.sentiment < -0.2 ? "bg-rose-500/15 text-rose-300" : "bg-slate-500/15 text-slate-400")}>
                      {n.sentiment > 0 ? `+${n.sentiment}` : n.sentiment}
                    </span>
                    <a href={n.url} target="_blank" rel="noopener noreferrer" className="min-w-0 flex-1 truncate text-slate-300 hover:text-cyan-300 hover:underline" title={`${n.title} — ${n.source}`}>
                      {n.title}
                    </a>
                    <button
                      type="button"
                      onClick={() => void runNewsAi(n.id)}
                      disabled={aiSumBusy === n.id}
                      title="Tek cümlelik Türkçe özet üret (öğe başına bir kez, önbellekli)"
                      className="shrink-0 rounded border border-violet-500/40 bg-violet-500/10 px-1.5 py-0.5 text-[10px] font-bold text-violet-300 transition-colors hover:bg-violet-500/20 disabled:opacity-50"
                    >
                      {aiSumBusy === n.id ? "…" : (n.aiSummary || aiSums[n.id]?.summaryTr) ? (aiSums[n.id]?.cached || n.aiSummary ? "TR ✓" : "TR") : "TR"}
                    </button>
                    <span className="shrink-0 text-[10px] text-slate-400">{n.source}</span>
                  </div>
                  {(n.aiSummary || aiSums[n.id]?.summaryTr) && (
                    <p className="ml-12 mt-0.5 text-[11px] leading-snug text-violet-200/90">
                      <span className="mr-1 rounded bg-violet-500/15 px-1 text-[9px] font-bold text-violet-300">AI ÖZETİ</span>
                      {n.aiSummary || aiSums[n.id]?.summaryTr}
                    </p>
                  )}
                  {aiSums[n.id] && !aiSums[n.id].ok && !aiSums[n.id].summaryTr && (
                    <p className="ml-12 mt-0.5 text-[10px] text-slate-400">{aiSums[n.id].reason}</p>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )}
        <p className="mt-3 text-[11px] text-slate-400">
          Simülasyon Gate 4 (no-lookahead) ve Gate 5 (realism: MARKETABLE / RESTING / GAP-BETTER fill etiketleri) ile koşar;
          equity eğrisi exact rational hesaplanır, yalnızca gösterim katmanında ondalık gösterilir.
        </p>
      </Panel>

      {/* P4-2 YZ Yardımcı — salt-okunur teşhis kopyilotu (emir yetkisi yok) */}
      <Panel title="YZ Yardımcı · Teşhis & Sohbet" icon={<Sparkles className="h-4 w-4 text-violet-400" aria-hidden />}>
        <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
          <div className="space-y-1.5">
            <Label className="text-xs text-slate-400">Sağlayıcı</Label>
            <Select value={aiProvider} onValueChange={(v) => { setAiProvider(v); setAiModel(""); }}>
              <SelectTrigger className="min-h-11 w-full border-border-strong bg-surface text-slate-100">
                <SelectValue placeholder="Sağlayıcı seç" />
              </SelectTrigger>
              <SelectContent className="border-border-strong bg-[#151a24]">
                {(aiProviders.length > 0 ? aiProviders : [{ id: "builtin", label: "Yerleşik (paketlenmedi)", models: [] as string[], needsKey: false, note: "" }]).map((p) => (
                  <SelectItem key={p.id} value={p.id} className="text-slate-200">
                    {p.label}{p.needsKey ? " · anahtarlı" : ""}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="ai-model" className="text-xs text-slate-400">Model{aiProviderInfo && aiProviderInfo.models.length > 0 ? ` (varsayılan ${aiProviderInfo.models[0]})` : ""}</Label>
            <Input
              id="ai-model"
              value={aiModel}
              onChange={(e) => setAiModel(e.target.value)}
              placeholder={aiProviderInfo?.models[0] ?? "model"}
              className="num min-h-11 border-border-strong bg-surface text-slate-100"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="ai-key" className="text-xs text-slate-400">API anahtarı (yalnız bu sekmede)</Label>
            <Input
              id="ai-key"
              type="password"
              value={aiKey}
              onChange={(e) => setAiKey(e.target.value)}
              placeholder={aiProviderInfo?.needsKey ? "sk-…" : "gerekmez"}
              disabled={!aiProviderInfo?.needsKey && aiProvider !== "ollama"}
              autoComplete="off"
              className="num min-h-11 border-border-strong bg-surface text-slate-100"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="ai-base" className="text-xs text-slate-400">Base URL (Ollama)</Label>
            <Input
              id="ai-base"
              value={aiBaseUrl}
              onChange={(e) => setAiBaseUrl(e.target.value)}
              placeholder={aiProviderInfo?.defaultBaseUrl ?? "http://localhost:11434"}
              disabled={aiProvider !== "ollama"}
              className="num min-h-11 border-border-strong bg-surface text-slate-100"
            />
          </div>
        </div>
        {aiProviderInfo?.note && <p className="mt-1.5 text-[10px] text-slate-400">{aiProviderInfo.note} Anahtar diske yazılmaz, yanıta konmaz.</p>}
        <div className="mt-3 flex gap-1.5">
          {([{ v: "chat", l: "SOHBET" }, { v: "diagnose", l: "BOT TEŞHİSİ" }] as const).map((o) => (
            <button
              key={o.v}
              onClick={() => { setAiMode(o.v); setAiRes(null); setAiErr(null); }}
              aria-pressed={aiMode === o.v}
              className={cn(
                "min-h-10 flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold",
                aiMode === o.v
                  ? "border-violet-500/60 bg-violet-500/10 text-violet-300"
                  : "border-border text-slate-500 hover:border-border-strong",
              )}
            >
              {o.l}
            </button>
          ))}
        </div>
        {aiMode === "diagnose" ? (
          <div className="mt-2 space-y-1.5">
            <Label className="text-xs text-slate-400">Bot (son 120 olay şelale olarak verilir)</Label>
            <Select value={aiBot} onValueChange={setAiBot}>
              <SelectTrigger className="min-h-11 w-full border-border-strong bg-surface text-slate-100">
                <SelectValue placeholder="Teşhis edilecek bot" />
              </SelectTrigger>
              <SelectContent className="border-border-strong bg-[#151a24]">
                {bots.map((b) => (
                  <SelectItem key={b.id} value={b.id} className="text-slate-200">
                    {b.name} · {b.symbol}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          </div>
        ) : (
          <div className="mt-2 space-y-1.5">
            <Label htmlFor="ai-msg" className="text-xs text-slate-400">Soru (sözlük + SSS + haber bağlamıyla yanıtlanır)</Label>
            <Input
              id="ai-msg"
              value={aiMsg}
              onChange={(e) => setAiMsg(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter") void askAi(); }}
              placeholder="örn. -2010 hatası ne demek?"
              className="min-h-11 border-border-strong bg-surface text-slate-100"
            />
          </div>
        )}
        <div className="mt-2 flex items-center gap-2">
          <Button onClick={() => void askAi()} disabled={aiBusy} className="min-h-10 flex-1 bg-violet-500/90 font-bold text-[#0d0518] hover:bg-violet-500">
            {aiBusy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <Send className="h-4 w-4" aria-hidden />}
            {aiBusy ? "Soruyor…" : aiMode === "chat" ? "SOR" : "TEŞHİS ET"}
          </Button>
          {aiRes && (
            <span className={cn("rounded px-2 py-1 font-mono text-[11px] font-bold", aiRes.verdict === "OK" ? "bg-emerald-500/15 text-emerald-300" : aiRes.verdict === "ABSTAIN" ? "bg-amber-500/15 text-amber-300" : "bg-rose-500/15 text-rose-300")}>
              {aiRes.verdict} · {aiRes.elapsedMs}ms
            </span>
          )}
        </div>
        {aiErr && <p className="mt-2 text-xs text-rose-400">{aiErr}</p>}
        {aiRes && !aiRes.ok && aiRes.note && (
          <p className="mt-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-2.5 text-[11px] leading-relaxed text-amber-300/90">{aiRes.note}</p>
        )}
        {aiRes?.bullets && aiRes.bullets.length > 0 && (
          <ol className="mt-2 space-y-1.5">
            {aiRes.bullets.map((b, i) => (
              <li key={i} className="rounded-lg border border-border bg-surface p-2.5 text-[11px] leading-relaxed">
                <span className="font-bold text-violet-300">{i + 1}. {b.title}: </span>
                <span className="text-slate-300">{b.text}</span>
              </li>
            ))}
          </ol>
        )}
        {aiRes?.text && (
          <p className="mt-2 rounded-lg border border-border bg-surface p-2.5 text-[11px] leading-relaxed text-slate-300">{aiRes.text}</p>
        )}
        {aiRes?.sources && ((aiRes.sources.faqIds?.length ?? 0) + (aiRes.sources.glossaryCodes?.length ?? 0) > 0 || aiRes.sources.newsOk) && (
          <p className="mt-1.5 text-[10px] text-slate-400">
            kaynaklar:{aiRes.sources.faqIds?.map((f) => ` SSS:${f}`).join("")}{aiRes.sources.glossaryCodes?.map((g) => ` SÖZLÜK:${g}`).join("")}{aiRes.sources.newsOk ? " +haber" : ""} · model: {aiRes.model}
          </p>
        )}
        <p className="mt-2 text-[10px] leading-tight text-slate-400">
          Salt-okunur kum havuzu: emir açma/iptal yetkisi yok; emir isteyen sorular otomatik reddedilir. Anahtar yalnız istek gövdesinde taşınır — Ayarlar&apos;a ya da diske yazılmaz.
        </p>
      </Panel>

      {certError && <div className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">Sertifika hatası: {certError}</div>}

      {cert && (
        <Panel title="Sertifika — WFE / DSR / PBO / Monte Carlo" icon={<BadgeCheck className="h-4 w-4 text-amber-400" aria-hidden />}>
          {(() => {
            const c = cert.cert;
            const wfe = parseFloat(c.wfe_pct);
            const dsr = parseFloat(c.dsr);
            const pbo = parseFloat(c.pbo_pct);
            const verdict = cert.verdict;
            const gates = cert.gates ?? [];
            return (
              <>
                {/* Faz 0.1 — motor hükmü: bu rozet yeşilse strateji LIVE kapısından geçer. */}
                {verdict && (
                <div
                  role="status"
                  className={cn(
                    "rounded-lg border px-3 py-2 text-xs font-bold",
                    verdict === "GECTI"
                      ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-300"
                      : "border-rose-500/40 bg-rose-500/10 text-rose-300",
                  )}
                >
                  SERTİFİKA HÜKMÜ: {verdict === "GECTI" ? "GECTI ✓ — LIVE kapısından geçer" : "KALDI — sertifikasız strateji canlıya eklenemez"}
                  {gates.length > 0 && (
                    <span className="mt-1 block text-[11px] font-normal opacity-90">
                      {gates.map((g) => `${g.pass ? "✓" : "✗"} ${g.label} (eşik ${g.threshold}, değer ${g.value})`).join(" · ")}
                    </span>
                  )}
                </div>
              )}
              <div className="space-y-3">
                <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
                  {certBadge(
                    "WFE — WALK-FORWARD",
                    `${c.wfe_pct}%`,
                    !Number.isFinite(wfe) ? "info" : wfe >= 50 ? "good" : wfe >= 0 ? "warn" : "bad",
                    "OOS/IS getiri oranı (folding ort.); ≥%50 sağlıklı"
                  )}
                  {certBadge(
                    "DSR — DEFLATED SHARPE",
                    c.dsr,
                    dsr >= 0.95 ? "good" : dsr >= 0.7 ? "warn" : "bad",
                    `${c.trials} deneme üzerinden şişirilmiş Sharpe olasılığı`
                  )}
                  {certBadge(
                    "PBO — OVERFIT OLASILIĞI",
                    `${c.pbo_pct}%`,
                    !Number.isFinite(pbo) ? "info" : pbo <= 20 ? "good" : pbo <= 40 ? "warn" : "bad",
                    `CSCV ${c.blocks} blok × ${c.combos} kombinasyon`
                  )}
                  {certBadge(
                    "MC — KAYIP OLASILIĞI",
                    `${c.mc_prob_loss_pct}%`,
                    parseFloat(c.mc_prob_loss_pct) <= 25 ? "good" : parseFloat(c.mc_prob_loss_pct) <= 50 ? "warn" : "bad",
                    `${c.mc_runs} blok-bootstrap yolu`
                  )}
                </div>
                <div className="grid gap-3 sm:grid-cols-4">
                  {certBadge("MC P5 GETİRİ", `${c.mc_return_p5_pct}%`, parseFloat(c.mc_return_p5_pct) < 0 ? "bad" : "good", "Kötü senaryo (%5 dilim)")}
                  {certBadge("MC MEDYAN", `${c.mc_return_p50_pct}%`, "info", "Tipik yol getirisi")}
                  {certBadge("MC P95 GETİRİ", `${c.mc_return_p95_pct}%`, parseFloat(c.mc_return_p95_pct) > 0 ? "good" : "warn", "İyi senaryo (%95 dilim)")}
                  {certBadge("MC ORT. MDD", `${c.mc_mean_mdd_pct}%`, parseFloat(c.mc_mean_mdd_pct) <= 15 ? "good" : parseFloat(c.mc_mean_mdd_pct) <= 30 ? "warn" : "bad", "Yol bazlı ortalama maks. düşüş")}
                </div>
                {cert.base && (
                  <div className="flex flex-wrap gap-2 text-[11px] text-slate-400">
                    <span className="rounded bg-surface px-2 py-1">Temel: getiri {cert.base.totalReturnPct}%</span>
                    <span className="rounded bg-surface px-2 py-1">MDD {cert.base.maxDrawdownPct}%</span>
                    <span className="rounded bg-surface px-2 py-1">Per-bar SR (×100) {cert.base.sharpeDisplay}</span>
                    <span className="rounded bg-surface px-2 py-1">Deal {cert.base.deals}</span>
                  </div>
                )}
                {c.warnings.length > 0 && (
                  <div className="flex items-start gap-2 rounded-lg border border-amber-500/30 bg-amber-500/5 p-2.5 text-[11px] text-amber-300/90">
                    <ShieldAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" aria-hidden />
                    <span>{c.warnings.join(" · ")}</span>
                  </div>
                )}
                <p className="text-[10px] leading-tight text-slate-400">
                  WFE: walk-forward OOS/IS getiri oranı · DSR: Bailey–López de Prado deflated Sharpe (deneme varyansı ile söndürülmüş) · PBO: CSCV kombinatoryal aşırı-uyum olasılığı · MC: dairesel blok bootstrap, xorshift deterministik tohum.
                </p>
              </div>
              </>
            );
          })()}
        </Panel>
      )}

      {error && <div className="rounded-lg border border-rose-500/40 bg-rose-500/10 p-3 text-sm text-rose-300">{error}</div>}

      {result && <BacktestResultView result={result} symbol={symbol.trim().toUpperCase() || "BTCUSDT"} interval={interval} />}
    </div>
  );
}
