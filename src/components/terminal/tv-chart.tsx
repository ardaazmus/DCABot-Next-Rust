"use client";

/**
 * TvChart — real TradingView charting (lightweight-charts v5) fed by the
 * engine klines proxy. Used in the wizard preview panel and the studio.
 *
 * Task 21 — backtest mode: `data` prop ile dışarıdan bar penceresi alınır
 * (fetch/refresh yok — backtest'in KULLANDIĞI barlar birebir çizilir),
 * `plotSeries` pine plot çizgilerini ana fiyat grafiğine bindirir (na → boşluk),
 * `markers` fill oklarını (Buy/Sell) mumlar üzerine koyar.
 *
 * Conventions: engine gateway fetch (engineUrl → ?XTransformPort=3002),
 * dark quantitative terminal palette (emerald long / rose short),
 * Turkish labels, chart instance stored in refs, SSR-safe (empty container
 * on the server, chart created only inside useEffect).
 */

import { useEffect, useMemo, useRef, useState } from "react";
import {
  createChart,
  CandlestickSeries,
  HistogramSeries,
  LineSeries,
  ColorType,
  CrosshairMode,
  LineStyle,
  createSeriesMarkers,
  type IChartApi,
  type ISeriesApi,
  type IPriceLine,
  type ISeriesMarkersPluginApi,
  type CandlestickData,
  type HistogramData,
  type LineData,
  type SeriesMarker,
  type UTCTimestamp,
  type WhitespaceData,
} from "lightweight-charts";
import { RefreshCw } from "lucide-react";
import type { Interval, KlineBar } from "@/lib/engine-types";
import { engineFetch } from "@/hooks/use-engine";
import { fmtFrac } from "@/lib/format";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

/* ---------------- palette ---------------- */

const UP_COLOR = "#10b981";
const DOWN_COLOR = "#f43f5e";
const VOL_UP = "rgba(16,185,129,0.6)";
const VOL_DOWN = "rgba(244,63,94,0.6)";
const GRID_COLOR = "rgba(139,147,167,0.08)";
const BORDER_COLOR = "rgba(139,147,167,0.2)";
const TEXT_COLOR = "#8b93a7";
const PANEL_BG = "#0b0f17";

const REFRESH_MS = 20_000;

/* ---------------- props contracts ---------------- */

export interface TvPriceLine {
  price: number;
  color: string;
  title: string;
  dashed?: boolean;
}

/** Task 21 — pine plot serisi (bar-dizini hizalı; null = "na" → boşluk). */
export interface TvPlotSeries {
  title: string;
  color: string;
  values: (number | null)[];
}

/** Task 21 — fill işaretçisi (mum üzerine ok). */
export interface TvMarker {
  time: number; // epoch ms
  side: "Buy" | "Sell";
  text?: string;
}

export interface TvChartProps {
  symbol: string; // "BTCUSDT"
  interval: Interval; // 9 interval ("1m"…|"1M") — engine-types INTERVALS ile aynı stringler API'yi vurur
  height?: number; // default 360
  bars?: number; // default 240
  priceLines?: TvPriceLine[]; // horizontal levels overlay (grid/DCA levels, TP, avg)
  compact?: boolean; // hides volume histogram + legend (wizard right panel)
  /** Task 21 — statik veri modu: backtest bar penceresi. Verildiğinde fetch/refresh YOK. */
  data?: KlineBar[];
  /** Task 21 — pine plot çizgileri (ana grafiğe bindirilir). */
  plotSeries?: TvPlotSeries[];
  /** Task 21 — fill okları (Buy=altında yukarı ok, Sell=üstünde aşağı ok). */
  markers?: TvMarker[];
}

/* ---------------- data shaping ---------------- */

interface KlinesResponse {
  bars: KlineBar[];
  error?: string;
}

function toCandleData(bars: KlineBar[]): CandlestickData<UTCTimestamp>[] {
  const sorted = [...bars].sort((a, b) => a.t - b.t);
  const out: CandlestickData<UTCTimestamp>[] = [];
  let lastT = -1;
  for (const b of sorted) {
    if (b.t === lastT) continue; // dedupe — lw-charts rejects duplicate times
    lastT = b.t;
    const o = Number(b.o);
    const h = Number(b.h);
    const l = Number(b.l);
    const c = Number(b.c);
    if (!Number.isFinite(o) || !Number.isFinite(h) || !Number.isFinite(l) || !Number.isFinite(c)) continue;
    out.push({ time: (b.t / 1000) as UTCTimestamp, open: o, high: h, low: l, close: c });
  }
  return out;
}

function toVolumeData(bars: KlineBar[]): HistogramData<UTCTimestamp>[] {
  const sorted = [...bars].sort((a, b) => a.t - b.t);
  const out: HistogramData<UTCTimestamp>[] = [];
  let lastT = -1;
  for (const b of sorted) {
    if (b.t === lastT) continue;
    lastT = b.t;
    const o = Number(b.o);
    const c = Number(b.c);
    const v = Number(b.v);
    if (!Number.isFinite(c) || !Number.isFinite(o) || !Number.isFinite(v)) continue;
    out.push({
      time: (b.t / 1000) as UTCTimestamp,
      value: v,
      color: c >= o ? VOL_UP : VOL_DOWN,
    });
  }
  return out;
}

/* ---------------- component ---------------- */

export function TvChart({
  symbol,
  interval,
  height = 360,
  bars = 240,
  priceLines,
  compact = false,
  data,
  plotSeries,
  markers,
}: TvChartProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const chartRef = useRef<IChartApi | null>(null);
  const candleRef = useRef<ISeriesApi<"Candlestick"> | null>(null);
  const volumeRef = useRef<ISeriesApi<"Histogram"> | null>(null);
  const plotSeriesRef = useRef<ISeriesApi<"Line">[]>([]);
  const markersRef = useRef<ISeriesMarkersPluginApi<UTCTimestamp> | null>(null);
  const timesRef = useRef<UTCTimestamp[]>([]);
  const createdLinesRef = useRef<IPriceLine[]>([]);
  const priceLinesLatestRef = useRef<TvPriceLine[]>(priceLines ?? []);
  const heightRef = useRef(height);
  const busyRef = useRef(false);

  const [status, setStatus] = useState<"loading" | "ok" | "error">("loading");
  const [errorMsg, setErrorMsg] = useState<string | null>(null);
  const [lastClose, setLastClose] = useState<number | null>(null);
  const [hasData, setHasData] = useState(false);
  const [retryTick, setRetryTick] = useState(0);

  /* -- chart lifecycle (client only, runs once per `compact` change) -- */
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;

    const chart = createChart(el, {
      width: el.clientWidth,
      height: heightRef.current,
      autoSize: false,
      layout: {
        background: { type: ColorType.Solid, color: "transparent" },
        textColor: TEXT_COLOR,
      },
      grid: {
        vertLines: { color: GRID_COLOR },
        horzLines: { color: GRID_COLOR },
      },
      rightPriceScale: { borderColor: BORDER_COLOR },
      crosshair: { mode: CrosshairMode.Normal },
    });
    chartRef.current = chart;

    const candles = chart.addSeries(CandlestickSeries, {
      upColor: UP_COLOR,
      downColor: DOWN_COLOR,
      borderUpColor: UP_COLOR,
      borderDownColor: DOWN_COLOR,
      wickUpColor: UP_COLOR,
      wickDownColor: DOWN_COLOR,
    });
    candleRef.current = candles;

    if (!compact) {
      const vol = chart.addSeries(HistogramSeries, {
        priceScaleId: "", // separate overlay price scale
        priceFormat: { type: "volume" },
        lastValueVisible: false,
        priceLineVisible: false,
      });
      vol.priceScale().applyOptions({ scaleMargins: { top: 0.82, bottom: 0 } });
      volumeRef.current = vol;
    }

    chart.timeScale().applyOptions({ timeVisible: true, secondsVisible: false });

    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w && w > 0) chart.applyOptions({ width: Math.floor(w) });
    });
    ro.observe(el);

    return () => {
      ro.disconnect();
      chart.remove();
      chartRef.current = null;
      candleRef.current = null;
      volumeRef.current = null;
      plotSeriesRef.current = [];
      markersRef.current = null;
      timesRef.current = [];
      createdLinesRef.current = [];
    };
  }, [compact]);

  /* -- height sync -- */
  useEffect(() => {
    heightRef.current = height;
    chartRef.current?.applyOptions({ height });
  }, [height]);

  /* -- data: statik backtest penceresi veya klines fetch + 20s refresh -- */
  useEffect(() => {
    // Task 21 — statik veri modu: backtest'in KULLANDIĞI barlar dışarıdan gelir.
    // Fetch yok, refresh yok — plot dizinleri bu pencereyle birebir hizalanır.
    if (data !== undefined && data !== null) {
      const candleData = toCandleData(data);
      candleRef.current?.setData(candleData);
      volumeRef.current?.setData(toVolumeData(data));
      timesRef.current = candleData.map((d) => d.time);
      setLastClose(candleData.length > 0 ? candleData[candleData.length - 1].close : null);
      if (candleData.length === 0) {
        setErrorMsg("Grafik verisi yok — bar penceresi boş");
        setStatus("error");
        setHasData(false);
      } else {
        setErrorMsg(null);
        setHasData(true);
        setStatus("ok");
      }
      return;
    }
    if (data !== undefined) {
      // data=null → açıkça veri yok (boş durum); fetch tetiklenmez.
      return;
    }

    let alive = true;

    const load = async (initial: boolean) => {
      if (busyRef.current) return;
      busyRef.current = true;
      if (initial) setStatus("loading");
      try {
        const res = await engineFetch<KlinesResponse>(
          `/api/klines?symbol=${encodeURIComponent(symbol)}&interval=${interval}&limit=${bars}`,
        );
        if (!alive) return;
        if (res && typeof res === "object" && "error" in res && res.error) {
          throw new Error(String(res.error));
        }
        const rawBars = res?.bars ?? [];
        const candleData = toCandleData(rawBars);
        candleRef.current?.setData(candleData);
        volumeRef.current?.setData(toVolumeData(rawBars));
        timesRef.current = candleData.map((d) => d.time);
        setLastClose(candleData.length > 0 ? candleData[candleData.length - 1].close : null);
        setHasData(candleData.length > 0);
        setErrorMsg(null);
        setStatus("ok");
      } catch (e) {
        if (!alive) return;
        setErrorMsg(e instanceof Error ? e.message : "Grafik verisi alınamadı");
        setStatus("error");
      } finally {
        busyRef.current = false;
      }
    };

    void load(true);
    const t = setInterval(() => void load(false), REFRESH_MS);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, [data, symbol, interval, bars, retryTick]);

  /* -- pine plot serileri: ana grafiğe LineSeries olarak bindirilir -- */
  const plotKey = useMemo(() => JSON.stringify(plotSeries ?? []), [plotSeries]);

  useEffect(() => {
    const chart = chartRef.current;
    if (!chart) return;
    // eski çizgileri temizle (idempotent — chart yıkılmışsa sessiz geç)
    for (const s of plotSeriesRef.current) {
      try {
        chart.removeSeries(s);
      } catch {
        /* already torn down */
      }
    }
    plotSeriesRef.current = [];
    const series = plotSeries ?? [];
    const times = timesRef.current;
    if (series.length === 0 || times.length === 0) return;
    // Plot değerleri backtest bar DİZİNİNE hizalıdır; timesRef aynı pencereden
    // gelir (data modunda backtest barları, fetch modunda canlı barlar).
    for (const p of series.slice(0, 6)) {
      try {
        const line = chart.addSeries(LineSeries, {
          color: p.color,
          lineWidth: 2,
          priceLineVisible: false,
          lastValueVisible: true,
          title: p.title,
        });
        const n = Math.min(p.values.length, times.length);
        const points: (LineData<UTCTimestamp> | WhitespaceData<UTCTimestamp>)[] = [];
        for (let i = 0; i < n; i++) {
          const v = p.values[i];
          if (v === null || !Number.isFinite(v)) {
            // "na" bar → whitespace (çizgide boşluk; TV plot(na) davranışı)
            points.push({ time: times[i] });
          } else {
            points.push({ time: times[i], value: v });
          }
        }
        line.setData(points);
        plotSeriesRef.current.push(line);
      } catch {
        /* tekrar denemede chart yıkılmış olabilir */
      }
    }
  }, [plotKey, plotSeries]);

  /* -- fill işaretçileri: Buy=altında yeşil yukarı ok, Sell=üstünde kırmızı aşağı ok -- */
  const markersKey = useMemo(() => JSON.stringify(markers ?? []), [markers]);

  useEffect(() => {
    const candles = candleRef.current;
    if (!candles) return;
    const ms = markers ?? [];
    const mapped: SeriesMarker<UTCTimestamp>[] = ms.slice(0, 400).map((m) => ({
      time: (m.time / 1000) as UTCTimestamp,
      position: m.side === "Buy" ? "belowBar" : "aboveBar",
      shape: m.side === "Buy" ? "arrowUp" : "arrowDown",
      color: m.side === "Buy" ? UP_COLOR : DOWN_COLOR,
      text: m.text,
      size: 1,
    }));
    try {
      if (!markersRef.current) {
        markersRef.current = createSeriesMarkers(candles, mapped) as unknown as ISeriesMarkersPluginApi<UTCTimestamp>;
      } else {
        markersRef.current.setMarkers(mapped);
      }
    } catch {
      /* chart already torn down */
    }
  }, [markersKey, markers]);

  /* -- price lines: keep latest array, recreate on content change -- */
  useEffect(() => {
    priceLinesLatestRef.current = priceLines ?? [];
  }, [priceLines]);

  const priceLinesKey = useMemo(() => JSON.stringify(priceLines ?? []), [priceLines]);

  useEffect(() => {
    const series = candleRef.current;
    if (!series) return;
    for (const pl of createdLinesRef.current) {
      try {
        series.removePriceLine(pl);
      } catch {
        /* chart already torn down */
      }
    }
    createdLinesRef.current = priceLinesLatestRef.current.map((p) =>
      series.createPriceLine({
        price: p.price,
        color: p.color,
        lineWidth: 1,
        lineStyle: p.dashed ? LineStyle.Dashed : LineStyle.Solid,
        axisLabelVisible: true,
        title: p.title,
      }),
    );
  }, [priceLinesKey]);

  /* -- render -- */
  const levelCount = (priceLines ?? []).length;
  const plotLegend = (plotSeries ?? [])
    .map((p, idx) => ({
      title: p.title,
      color: p.color,
      key: `${p.title}-${idx}`,
      last: [...p.values].reverse().find((v) => v !== null && Number.isFinite(v)) ?? null,
    }))
    .filter((p) => p.last !== null);

  return (
    <div
      className="relative w-full overflow-hidden rounded-md border border-border/70"
      style={{ height, backgroundColor: `${PANEL_BG}66` }}
    >
      <div ref={containerRef} className="h-full w-full" />
      <span className="sr-only">
        {symbol} {interval} mum grafiği
        {lastClose !== null ? `, son kapanış ${fmtFrac(lastClose)}` : ""}
        {levelCount > 0 ? `, ${levelCount} fiyat seviyesi işaretli` : ""}
        {plotLegend.length > 0 ? `, ${plotLegend.length} plot çizgisi` : ""}
        {(markers ?? []).length > 0 ? `, ${(markers ?? []).length} işlem işareti` : ""}.
      </span>

      {!compact && status !== "error" && (
        <div className="pointer-events-none absolute left-2 top-2 z-10 flex items-center gap-1.5 rounded-md border border-border/60 bg-[#0b0f17]/80 px-2 py-1 text-[10px] font-medium text-muted-foreground backdrop-blur-sm">
          <span className="font-semibold text-slate-200">{symbol}</span>
          <span aria-hidden>·</span>
          <span>{interval}</span>
          {lastClose !== null && (
            <>
              <span aria-hidden>·</span>
              <span className="text-slate-200">{fmtFrac(lastClose)}</span>
            </>
          )}
          {levelCount > 0 && (
            <Badge
              variant="outline"
              className="ml-1 border-cyan-500/40 bg-cyan-500/10 px-1.5 py-0 text-[9px] text-cyan-300"
            >
              {levelCount} seviye
            </Badge>
          )}
          {plotLegend.length > 0 && (
            <Badge
              variant="outline"
              className="ml-1 border-violet-500/40 bg-violet-500/10 px-1.5 py-0 text-[9px] text-violet-300"
            >
              {plotLegend.length} plot
            </Badge>
          )}
          {(markers ?? []).length > 0 && (
            <Badge
              variant="outline"
              className="ml-1 border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0 text-[9px] text-emerald-300"
            >
              {(markers ?? []).length} fill
            </Badge>
          )}
        </div>
      )}

      {plotLegend.length > 0 && (
        <div className="pointer-events-none absolute right-2 top-2 z-10 flex flex-col items-end gap-0.5">
          {plotLegend.map((p) => (
            <span
              key={p.key}
              className="rounded bg-[#0b0f17]/80 px-1.5 py-0.5 font-mono text-[9px] text-slate-300 backdrop-blur-sm"
            >
              <span
                className="mr-1 inline-block h-1.5 w-3 rounded-sm align-middle"
                style={{ background: p.color }}
                aria-hidden
              />
              {p.title} <span className="num text-slate-400">{p.last}</span>
            </span>
          ))}
        </div>
      )}

      {status === "loading" && !hasData && (
        <div
          className="absolute inset-0 z-10 flex items-center justify-center bg-[#0b0f17]/50"
          role="status"
          aria-label="Grafik yükleniyor"
        >
          <div className="h-full w-full animate-pulse bg-gradient-to-b from-transparent via-slate-500/5 to-transparent" />
        </div>
      )}

      {status === "error" && (
        <div className="absolute inset-0 z-20 flex flex-col items-center justify-center gap-2 bg-[#0b0f17]/80 text-center">
          <p className={cn("text-xs font-semibold text-rose-400")}>Grafik verisi alınamadı</p>
          {errorMsg && (
            <p className="max-w-[85%] truncate text-[10px] text-muted-foreground" title={errorMsg}>
              {errorMsg}
            </p>
          )}
          {data === undefined && (
            <Button
              variant="outline"
              size="sm"
              className="h-7 text-[11px]"
              onClick={() => {
                setStatus("loading");
                setRetryTick((t) => t + 1);
              }}
            >
              <RefreshCw className="mr-1.5 h-3 w-3" aria-hidden />
              Yeniden Dene
            </Button>
          )}
        </div>
      )}
    </div>
  );
}
