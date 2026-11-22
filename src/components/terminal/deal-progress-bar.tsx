"use client";

import { useMemo } from "react";
import {
  AlertTriangle,
  ArrowDownRight,
  ArrowRight,
  ArrowUpRight,
  CheckCircle2,
  Flag,
  Info,
  Layers,
  MapPin,
  ShieldAlert,
  ShieldCheck,
  Target,
  Zap,
} from "lucide-react";
import { fmtFrac, fmtUsdSigned, fracToNum } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { LadderRow } from "@/lib/engine-types";

export const CRITICAL_LIQUIDATION_DISTANCE_PCT = 15;

export interface DealProgressBarProps {
  phase: string; // "Open" | "Idle" | "Closing"
  safetyTriggered: number;
  maxSafetyOrders?: number | null;
  avgPrice?: string | null;
  tpPrice?: string | null;
  slPrice?: string | null;
  currentPrice?: string | null;
  unrealizedPnl?: string | null;
  liqPrice?: string | null;
  marginMode?: boolean;
  nextSafetyPrice?: string | null;
  nextSafetyLevel?: number | null;
  positionQty?: string | null;
  ladder?: LadderRow[] | null;
  isShort?: boolean;
  symbol?: string;
  compact?: boolean;
  className?: string;
}

export function DealProgressBar({
  phase,
  safetyTriggered,
  maxSafetyOrders = 5,
  avgPrice,
  tpPrice,
  slPrice,
  currentPrice,
  unrealizedPnl,
  liqPrice,
  marginMode = false,
  nextSafetyPrice,
  nextSafetyLevel,
  positionQty,
  ladder,
  isShort = false,
  compact = false,
  className,
}: DealProgressBarProps) {
  const isOpen = phase === "Open";
  const maxSo = Math.max(1, maxSafetyOrders ?? 5);
  const isBagHolding = safetyTriggered >= maxSo;

  /* Anlık Fiyat (Mark Price): Doğrudan markPrice veya PnL / Pozisyon / Avg türetimi */
  const curNum = useMemo(() => {
    if (currentPrice) {
      const p = fracToNum(currentPrice);
      if (p && p > 0) return p;
    }
    const avg = fracToNum(avgPrice) ?? 0;
    const un = fracToNum(unrealizedPnl) ?? 0;
    const qty = fracToNum(positionQty) ?? 0;
    if (avg > 0 && qty > 0 && un !== 0) {
      return isShort ? avg - un / qty : avg + un / qty;
    }
    return avg > 0 ? avg : null;
  }, [currentPrice, avgPrice, unrealizedPnl, positionQty, isShort]);

  const metrics = useMemo(() => {
    const avg = fracToNum(avgPrice) ?? 0;
    const tp = fracToNum(tpPrice) ?? 0;
    const liq = fracToNum(liqPrice) ?? 0;
    const nextSo = fracToNum(nextSafetyPrice) ?? 0;
    const sl = fracToNum(slPrice) ?? 0;
    const cur = curNum ?? avg;

    if (!isOpen || avg <= 0) {
      return null;
    }

    // 1. Şimdiki Fiyat -> TP Mesafesi (%)
    let distToTpPct: number | null = null;
    let progressToTpPct = 0;
    if (tp > 0 && cur > 0) {
      if (isShort) {
        distToTpPct = cur > tp ? ((cur - tp) / cur) * 100 : 0;
        const totalSpan = avg - tp;
        const currentDelta = avg - cur;
        progressToTpPct = totalSpan > 0 ? Math.min(100, Math.max(0, (currentDelta / totalSpan) * 100)) : 0;
      } else {
        distToTpPct = tp > cur ? ((tp - cur) / cur) * 100 : 0;
        const totalSpan = tp - avg;
        const currentDelta = cur - avg;
        progressToTpPct = totalSpan > 0 ? Math.min(100, Math.max(0, (currentDelta / totalSpan) * 100)) : 0;
      }
    }

    // 2. Şimdiki Fiyat -> Başabaş (AVG) Sapması (%)
    let devFromAvgPct = 0;
    let isProfitable = false;
    if (cur > 0 && avg > 0) {
      if (isShort) {
        devFromAvgPct = ((avg - cur) / avg) * 100;
        isProfitable = cur <= avg;
      } else {
        devFromAvgPct = ((cur - avg) / avg) * 100;
        isProfitable = cur >= avg;
      }
    }

    // 3. Şimdiki Fiyat -> Likidasyon Mesafesi (%)
    let liqDistFromCurPct: number | null = null;
    let liqTier: { label: string; color: string; border: string; bg: string } | null = null;
    if (marginMode && liq > 0 && cur > 0) {
      const rawDist = isShort ? ((liq - cur) / cur) * 100 : ((cur - liq) / cur) * 100;
      const clampedDist = Math.max(0, rawDist);
      liqDistFromCurPct = clampedDist;
      liqTier =
        clampedDist >= 30
          ? { label: "Güvenli", color: "text-emerald-400", border: "border-emerald-500/40", bg: "bg-emerald-500/10" }
          : clampedDist >= CRITICAL_LIQUIDATION_DISTANCE_PCT
            ? { label: "Dikkat", color: "text-amber-400", border: "border-amber-500/40", bg: "bg-amber-500/10" }
            : { label: "KRİTİK LİKİDASYON RİSKİ", color: "text-rose-400 animate-pulse font-black", border: "border-rose-500/60", bg: "bg-rose-500/20" };
    }

    // 4. Şimdiki Fiyat -> Sonraki Güvenlik Emri Mesafesi (%)
    let nextSoDistPct: number | null = null;
    if (nextSo > 0 && cur > 0) {
      nextSoDistPct = isShort ? Math.max(0, ((nextSo - cur) / cur) * 100) : Math.max(0, ((cur - nextSo) / cur) * 100);
    }

    // 5. Şimdiki Fiyat -> Stop Loss Mesafesi (%)
    let slDistPct: number | null = null;
    if (sl > 0 && cur > 0) {
      slDistPct = isShort ? Math.max(0, ((sl - cur) / cur) * 100) : Math.max(0, ((cur - sl) / cur) * 100);
    }

    // 6. Şimdiki Fiyat ile TÜM Güvenlik Kademeleri (SO) Arasındaki İlişki Analizi
    const ladderRelations = (ladder ?? []).map((row, idx) => {
      const rowPrice = fracToNum(row.price) ?? 0;
      const levelNum = row.level ?? idx + 1;
      const isFilled = levelNum <= safetyTriggered;
      const isNext = levelNum === safetyTriggered + 1;

      // Şimdiki fiyata göre mesafe oranı (%)
      let distPct = 0;
      if (cur > 0 && rowPrice > 0) {
        if (isShort) {
          // Short: kademe fiyatı şimdiki fiyattan yukarıda olmalı
          distPct = ((rowPrice - cur) / cur) * 100;
        } else {
          // Long: kademe fiyatı şimdiki fiyattan aşağıda olmalı
          distPct = ((cur - rowPrice) / cur) * 100;
        }
      }

      return {
        level: levelNum,
        price: rowPrice,
        priceStr: row.price,
        isFilled,
        isNext,
        distPct,
        notional: row.notional,
      };
    });

    return {
      avg,
      tp,
      liq,
      nextSo,
      sl,
      cur,
      distToTpPct,
      progressToTpPct: Math.round(progressToTpPct),
      devFromAvgPct,
      isProfitable,
      liqDistFromCurPct,
      liqTier,
      nextSoDistPct,
      slDistPct,
      ladderRelations,
    };
  }, [isOpen, avgPrice, tpPrice, liqPrice, nextSafetyPrice, slPrice, curNum, ladder, safetyTriggered, isShort, marginMode]);

  const formatMovePct = (pct: number, digits = 2) => {
    const movesUp = (pct >= 0) === isShort;
    return `${movesUp ? "+" : "-"}%${Math.abs(pct).toFixed(digits)}`;
  };

  if (!isOpen || !metrics) {
    return (
      <div className={cn("flex items-center gap-2 rounded-lg border border-border/60 bg-surface/60 px-3 py-2 text-xs text-slate-500", className)}>
        <span className="h-2 w-2 rounded-full bg-slate-600" />
        <span>{phase === "Closing" ? "Deal Kapanıyor: Emir piyasada, dolum bekleniyor…" : "Aktif deal yok — başlangıç koşulu izleniyor"}</span>
      </div>
    );
  }

  /* Compact Mode (Dashboard & Fleet Cards) */
  if (compact) {
    return (
      <div className={cn("space-y-2.5 rounded-lg border border-border/80 bg-surface/90 p-2.5 text-xs", className)}>
        {/* Top: Status, Price & Liq Badge */}
        <div className="flex flex-wrap items-center justify-between gap-1.5">
          <div className="flex items-center gap-1.5">
            <span className="flex h-2 w-2 rounded-full bg-cyan-400 animate-pulse" />
            <span className="font-bold text-cyan-300">
              📍 Şimdiki: <span className="num font-black text-slate-100">{fmtFrac(metrics.cur)}</span>
            </span>
            <span
              className={cn(
                "num rounded px-1.5 py-0.2 text-[10px] font-bold",
                metrics.isProfitable ? "bg-emerald-500/15 text-emerald-400" : "bg-rose-500/15 text-rose-400"
              )}
            >
              {metrics.devFromAvgPct >= 0 ? `+${metrics.devFromAvgPct.toFixed(2)}%` : `${metrics.devFromAvgPct.toFixed(2)}%`}
            </span>
          </div>

          <div className="flex items-center gap-2">
            {marginMode && metrics.liqTier && (
              <span className={cn("rounded border px-1.5 py-0.5 text-[9px] font-bold", metrics.liqTier.border, metrics.liqTier.bg, metrics.liqTier.color)}>
                Liq: %{metrics.liqDistFromCurPct?.toFixed(1)} ({metrics.liqTier.label})
              </span>
            )}
            {unrealizedPnl && (
              <span
                className={cn(
                  "num rounded px-1.5 py-0.5 text-[10px] font-bold",
                  metrics.isProfitable ? "bg-emerald-500/15 text-emerald-400" : "bg-rose-500/15 text-rose-400"
                )}
              >
                Açık: {fmtUsdSigned(unrealizedPnl)}
              </span>
            )}
          </div>
        </div>

        {/* Multi-point bar: AVG -> Current Pin -> TP */}
        <div className="space-y-1">
          <div className="flex items-center justify-between text-[10px] text-slate-400">
            <span className="flex items-center gap-1">
              <Target className="h-3 w-3 text-cyan-400" />
              <span>AVG: {fmtFrac(metrics.avg)}</span>
            </span>
            {metrics.distToTpPct != null && (
              <span className="flex items-center gap-1 text-emerald-400">
                <Flag className="h-3 w-3" />
                <span>TP: {fmtFrac(metrics.tp)}</span>
                <span className="font-semibold">(%{metrics.distToTpPct.toFixed(2)} kaldı)</span>
              </span>
            )}
          </div>

          {/* Visual Track */}
          <div className="relative h-2 w-full overflow-hidden rounded-full bg-slate-800">
            <div
              style={{ width: `${Math.max(4, metrics.progressToTpPct)}%` }}
              className={cn(
                "h-full rounded-full transition-all duration-300",
                metrics.isProfitable
                  ? "bg-gradient-to-r from-cyan-500 via-emerald-500 to-emerald-400"
                  : "bg-gradient-to-r from-rose-500 via-amber-500 to-cyan-500"
              )}
            />
          </div>
        </div>

        {/* Footer: SO Stepper & Next SO distance */}
        <div className="flex items-center justify-between border-t border-border/40 pt-1.5 text-[10px] text-slate-400">
          <div className="flex items-center gap-1.5">
            <span className="font-medium">SO {safetyTriggered}/{maxSo}:</span>
            <div className="flex items-center gap-1">
              {Array.from({ length: maxSo }).map((_, idx) => {
                const isFilled = idx < safetyTriggered;
                return (
                  <div
                    key={idx}
                    title={`Kademe #${idx + 1} (${isFilled ? "Doldu" : "Bekliyor"})`}
                    className={cn(
                      "h-1.5 w-1.5 rounded-sm transition-all",
                      isFilled ? "bg-cyan-400 shadow-[0_0_4px_rgba(6,182,212,0.6)]" : "bg-slate-700 border border-slate-600/50"
                    )}
                  />
                );
              })}
            </div>
            {isBagHolding && (
              <span className="rounded bg-rose-500/20 px-1 py-0.2 text-[9px] font-bold text-rose-300">
                ⚠️ Kademe Bitti
              </span>
            )}
          </div>
          {metrics.nextSo > 0 && !isBagHolding && (
            <span className="text-amber-300/90 font-medium">
              Sonraki SO: {fmtFrac(metrics.nextSo)} ({metrics.nextSoDistPct == null ? "—" : formatMovePct(metrics.nextSoDistPct, 1)})
            </span>
          )}
        </div>
      </div>
    );
  }

  /* Full / Studio Mode: Comprehensive Visual Deal & Risk Tracker */
  return (
    <div className={cn("space-y-4 rounded-xl border border-border bg-[#0d1117] p-4 text-xs shadow-sm", className)}>
      {/* 1. Üst Ana Fiyat & Durum Bilgi Kartları */}
      <div className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
        {/* Şimdiki Fiyat (Mark) */}
        <div className="rounded-lg border border-cyan-500/40 bg-cyan-950/20 p-3 shadow-[0_0_15px_rgba(6,182,212,0.08)]">
          <div className="flex items-center justify-between text-[10px] font-bold uppercase tracking-wider text-cyan-300">
            <span className="flex items-center gap-1.5">
              <span className="h-2 w-2 rounded-full bg-cyan-400 animate-pulse" />
              📍 ŞİMDİKİ FİYAT
            </span>
            <span
              className={cn(
                "num rounded px-1.5 py-0.5 text-[10px] font-bold",
                metrics.isProfitable ? "bg-emerald-500/20 text-emerald-300" : "bg-rose-500/20 text-rose-300"
              )}
            >
              {metrics.devFromAvgPct >= 0 ? `+${metrics.devFromAvgPct.toFixed(2)}%` : `${metrics.devFromAvgPct.toFixed(2)}%`}
            </span>
          </div>
          <div className="num mt-1.5 text-xl font-black text-cyan-100">{fmtFrac(metrics.cur)}</div>
          <div className="mt-1 text-[10px] text-slate-400">
            {metrics.devFromAvgPct === 0
              ? "Maliyet seviyesinde (Başabaş)"
              : metrics.isProfitable
                ? isShort ? "Maliyetin altında (Kârda)" : "Maliyetin üzerinde (Kârda)"
                : isShort ? "Maliyetin üzerinde (Zararda)" : "Maliyetin altında (Zararda)"}
          </div>
        </div>

        {/* Başabaş (AVG Entry) */}
        <div className="rounded-lg border border-border/70 bg-surface/90 p-3">
          <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-slate-400">
            <span className="flex items-center gap-1 text-blue-400">
              <Target className="h-3 w-3" /> BAŞABAŞ (AVG)
            </span>
          </div>
          <div className="num mt-1.5 text-xl font-bold text-slate-100">{fmtFrac(metrics.avg)}</div>
          <div className="mt-1 text-[10px] text-slate-400">Ağırlıklı pozisyon maliyeti</div>
        </div>

        {/* Hedef (Take Profit) */}
        <div className="rounded-lg border border-border/70 bg-surface/90 p-3">
          <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-slate-400">
            <span className="flex items-center gap-1 text-emerald-400">
              <Flag className="h-3 w-3" /> HEDEF (TP)
            </span>
            {metrics.distToTpPct != null && (
              <span className="rounded bg-emerald-500/15 px-1.5 py-0.5 text-[9px] font-bold text-emerald-400">
                %{metrics.distToTpPct.toFixed(2)} kaldı
              </span>
            )}
          </div>
          <div className="num mt-1.5 text-xl font-bold text-emerald-400">{fmtFrac(metrics.tp)}</div>
          <div className="mt-1 text-[10px] text-slate-400">
            İlerleme: %{metrics.progressToTpPct} tamamlandı
          </div>
        </div>

        {/* Likidasyon Seviyesi ve Riski */}
        <div
          className={cn(
            "rounded-lg border p-3",
            marginMode && metrics.liqTier ? metrics.liqTier.border : "border-border/70 bg-surface/90",
            marginMode && metrics.liqTier ? metrics.liqTier.bg : ""
          )}
        >
          <div className="flex items-center justify-between text-[10px] font-semibold uppercase tracking-wider text-slate-400">
            <span className="flex items-center gap-1 text-rose-400">
              {marginMode ? <ShieldAlert className="h-3 w-3" /> : <ShieldCheck className="h-3 w-3 text-emerald-400" />}
              LİKİDASYON (LIQ)
            </span>
            {marginMode && metrics.liqTier && (
              <span className={cn("rounded px-1.5 py-0.5 text-[9px] font-bold", metrics.liqTier.color)}>
                {metrics.liqTier.label}
              </span>
            )}
          </div>
          <div className="num mt-1.5 text-xl font-bold">
            {marginMode && metrics.liq > 0 ? (
              <span className="text-rose-400">{fmtFrac(metrics.liq)}</span>
            ) : (
              <span className="text-slate-400">Yok (Spot)</span>
            )}
          </div>
          <div className="mt-1 text-[10px] text-slate-400">
            {marginMode && metrics.liqDistFromCurPct != null ? (
              <span className={cn("font-bold", metrics.liqTier?.color)}>
                Şimdiki Fiyata Mesafe: {formatMovePct(metrics.liqDistFromCurPct, 1)}
              </span>
            ) : (
              "Spot işlemde tasfiye riski bulunmaz"
            )}
          </div>
        </div>
      </div>

      {/* 2. ŞİMDİKİ FİYAT İLE SO'LAR VE LİKİDASYON ARASINDAKİ YÜZDESEL İLİŞKİ HARİTASI */}
      <div className="rounded-xl border border-border/80 bg-surface/80 p-4 space-y-3">
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border/50 pb-2">
          <div className="flex items-center gap-2">
            <Layers className="h-4 w-4 text-cyan-400" />
            <span className="text-xs font-bold text-slate-100">
              ŞİMDİKİ FİYAT İLE GÜVENLİK KADEMELERİ (SO) VE LİKİDASYON İLİŞKİ HARİTASI
            </span>
          </div>
          <div className="flex items-center gap-2 text-[11px] text-slate-400">
            <span>Referans Fiyat:</span>
            <span className="num font-bold text-cyan-300">{fmtFrac(metrics.cur)}</span>
          </div>
        </div>

        {/* Karşılaştırma Matrisi (Yüzdesel Oranlar) */}
        <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
          {/* A. Likidasyon Seviyesi İlişkisi */}
          <div className="flex flex-col justify-between rounded-lg border border-rose-500/30 bg-rose-500/5 p-2.5">
            <div>
              <div className="flex items-center justify-between text-[10px] font-semibold text-rose-300">
                <span className="flex items-center gap-1">
                  <ShieldAlert className="h-3 w-3" />
                  LİKİDASYON SEVİYESİ
                </span>
                {marginMode && metrics.liqTier && (
                  <span className={cn("text-[9px] font-bold", metrics.liqTier.color)}>
                    {metrics.liqTier.label}
                  </span>
                )}
              </div>
              <div className="num mt-1 text-sm font-bold text-slate-100">
                {marginMode && metrics.liq > 0 ? fmtFrac(metrics.liq) : "—"}
              </div>
            </div>
            <div className="mt-2 border-t border-rose-500/20 pt-1.5 text-[11px]">
              <div className="flex items-center justify-between">
                <span className="text-slate-400">Şimdiki Fiyata Oran:</span>
                <span className="num font-black text-rose-400">
                  {marginMode && metrics.liqDistFromCurPct != null
                    ? formatMovePct(metrics.liqDistFromCurPct)
                    : "—"}
                </span>
              </div>
              <div className="mt-0.5 text-[9px] text-slate-400 leading-snug">
                {marginMode
                  ? `Piyasa bu oranda ${isShort ? "yükseldiğinde" : "düştüğünde"} marjin tükenir ve tasfiye olur`
                  : "Spot modunda likidasyon yoktur"}
              </div>
            </div>
          </div>

          {/* B. Güvenlik Kademeleri (SO) İlişkisi */}
          {metrics.ladderRelations.length > 0 ? (
            metrics.ladderRelations.slice(0, 3).map((so) => (
              <div
                key={so.level}
                className={cn(
                  "flex flex-col justify-between rounded-lg border p-2.5 transition-all",
                  so.isFilled
                    ? "border-cyan-500/40 bg-cyan-500/5"
                    : so.isNext
                      ? "border-amber-500/50 bg-amber-500/10 shadow-[0_0_10px_rgba(245,158,11,0.15)]"
                      : "border-border/60 bg-surface/50"
                )}
              >
                <div>
                  <div className="flex items-center justify-between text-[10px] font-semibold">
                    <span className={cn("flex items-center gap-1", so.isFilled ? "text-cyan-300" : so.isNext ? "text-amber-300 font-bold" : "text-slate-400")}>
                      <Zap className="h-3 w-3" />
                      KADEME #{so.level} (SO{so.level})
                    </span>
                    <span
                      className={cn(
                        "rounded px-1.5 py-0.2 text-[9px] font-bold",
                        so.isFilled
                          ? "bg-cyan-500/20 text-cyan-300"
                          : so.isNext
                            ? "bg-amber-500/20 text-amber-300 animate-pulse"
                            : "bg-slate-800 text-slate-400"
                      )}
                    >
                      {so.isFilled ? "✓ Doldu" : so.isNext ? "⏳ Sırada" : "Bekliyor"}
                    </span>
                  </div>
                  <div className="num mt-1 text-sm font-bold text-slate-100">{fmtFrac(so.price)}</div>
                </div>
                <div className="mt-2 border-t border-border/40 pt-1.5 text-[11px]">
                  <div className="flex items-center justify-between">
                    <span className="text-slate-400">Şimdiki Fiyata Oran:</span>
                    <span
                      className={cn(
                        "num font-black",
                        so.isFilled
                          ? "text-cyan-300"
                          : so.isNext
                            ? "text-amber-300"
                            : "text-slate-300"
                      )}
                    >
                      {so.isFilled
                        ? `${isShort ? "Satıldı" : "Alındı"} (${formatMovePct(so.distPct)})`
                        : formatMovePct(so.distPct)}
                    </span>
                  </div>
                  <div className="mt-0.5 text-[9px] text-slate-400 leading-snug">
                    {so.isFilled
                      ? `Pozisyona dahil edildi, ortalamayı ${isShort ? "yukarı taşıdı" : "aşağı çekti"}`
                      : so.isNext
                        ? `Fiyat %${Math.abs(so.distPct).toFixed(2)} daha ${isShort ? "yükselirse otomatik satış" : "düşerse otomatik alım"} yapacak`
                        : `Daha derin ${isShort ? "yükselişlerde" : "düşüşlerde"} devreye girecek kademe`}
                  </div>
                </div>
              </div>
            ))
          ) : (
            /* Eğer ladder dizisi boşsa (Örn: basitleştirilmiş özet), sıradaki SO kartı gösterilir */
            <div className="flex flex-col justify-between rounded-lg border border-amber-500/40 bg-amber-500/5 p-2.5">
              <div>
                <div className="flex items-center justify-between text-[10px] font-semibold text-amber-300">
                  <span className="flex items-center gap-1">
                    <Zap className="h-3 w-3" />
                    SONRAKİ SO KADEMESİ
                  </span>
                  <span className="rounded bg-amber-500/20 px-1.5 py-0.2 text-[9px] font-bold text-amber-300">
                    Sırada
                  </span>
                </div>
                <div className="num mt-1 text-sm font-bold text-slate-100">
                  {metrics.nextSo > 0 ? fmtFrac(metrics.nextSo) : "Tüm Kademeler Doldu"}
                </div>
              </div>
              <div className="mt-2 border-t border-amber-500/20 pt-1.5 text-[11px]">
                <div className="flex items-center justify-between">
                  <span className="text-slate-400">Şimdiki Fiyata Oran:</span>
                  <span className="num font-black text-amber-400">
                    {metrics.nextSoDistPct != null ? formatMovePct(metrics.nextSoDistPct) : "—"}
                  </span>
                </div>
                <div className="mt-0.5 text-[9px] text-slate-400 leading-snug">
                  Fiyat bu oranda {isShort ? "yükseldiğinde" : "düştüğünde"} bir sonraki güvenlik emri tetiklenir
                </div>
              </div>
            </div>
          )}
        </div>

        {/* 3. Görsel Eksen Spektrumu */}
        <div className="relative pt-6 pb-2">
          {/* Track Bar */}
          <div className="relative h-3 w-full rounded-full bg-slate-800/90 shadow-inner overflow-hidden">
            {/* Drawdown vs Profit Fill */}
            <div
              style={{ width: `${Math.max(3, Math.min(100, metrics.progressToTpPct))}%` }}
              className={cn(
                "h-full rounded-full transition-all duration-300",
                metrics.isProfitable
                  ? "bg-gradient-to-r from-cyan-500 via-emerald-500 to-emerald-400 shadow-[0_0_12px_rgba(16,185,129,0.5)]"
                  : "bg-gradient-to-r from-rose-500 via-amber-500 to-cyan-500"
              )}
            />
          </div>

          {/* Critical Markers across the Spectrum */}
          <div className={cn("mt-2.5 flex items-center justify-between text-[11px]", isShort && "flex-row-reverse")}>
            {/* Left: Liquidation */}
            <div className={cn("space-y-0.5", isShort ? "text-right" : "text-left")}>
              <div className={cn("flex items-center gap-1 font-semibold text-rose-400", isShort && "justify-end")}>
                <ShieldAlert className="h-3 w-3" />
                <span>{marginMode && metrics.liq > 0 ? "Likidasyon" : "Alt Taban"}</span>
              </div>
              <div className="num font-mono text-[10px] text-slate-300">
                {marginMode && metrics.liq > 0 ? fmtFrac(metrics.liq) : "—"}
              </div>
              {marginMode && metrics.liqDistFromCurPct != null && (
                <div className={cn("text-[9px] font-bold", metrics.liqTier?.color)}>
                  {formatMovePct(metrics.liqDistFromCurPct, 1)} mesafe
                </div>
              )}
            </div>

            {/* Next Safety Order */}
            {metrics.nextSo > 0 && !isBagHolding && (
              <div className="space-y-0.5 text-center">
                <div className="flex items-center justify-center gap-1 font-semibold text-amber-300">
                  <AlertTriangle className="h-3 w-3" />
                  <span>Sonraki SO{nextSafetyLevel != null ? `#${nextSafetyLevel}` : ""}</span>
                </div>
                <div className="num font-mono text-[10px] text-slate-200">{fmtFrac(metrics.nextSo)}</div>
                {metrics.nextSoDistPct != null && (
                  <div className="text-[9px] text-amber-300/90 font-bold">
                    {formatMovePct(metrics.nextSoDistPct, 1)} tetik mesafesi
                  </div>
                )}
              </div>
            )}

            {/* Center: Current Price & AVG */}
            <div className="space-y-0.5 text-center">
              <div className="flex items-center justify-center gap-1 font-bold text-cyan-300">
                <MapPin className="h-3 w-3 text-cyan-400 animate-pulse" />
                <span>ŞİMDİKİ FİYAT</span>
              </div>
              <div className="num font-mono text-[11px] font-black text-cyan-100">{fmtFrac(metrics.cur)}</div>
              <div className="text-[9px] text-slate-400">
                AVG: {fmtFrac(metrics.avg)} ({metrics.devFromAvgPct >= 0 ? `+${metrics.devFromAvgPct.toFixed(2)}%` : `${metrics.devFromAvgPct.toFixed(2)}%`})
              </div>
            </div>

            {/* Right: Take Profit (TP) */}
            <div className={cn("space-y-0.5", isShort ? "text-left" : "text-right")}>
              <div className={cn("flex items-center gap-1 font-semibold text-emerald-400", !isShort && "justify-end")}>
                <Flag className="h-3 w-3" />
                <span>Hedef (TP)</span>
              </div>
              <div className="num font-mono text-[10px] font-bold text-emerald-300">{fmtFrac(metrics.tp)}</div>
              {metrics.distToTpPct != null && (
                <div className="text-[9px] font-bold text-emerald-400">
                  %{metrics.distToTpPct.toFixed(2)} kaldı
                </div>
              )}
            </div>
          </div>
        </div>

        {/* 4. Safety Order Kademe Stepper'ı & Doğal Dil Anlatısı */}
        <div className="flex flex-wrap items-center justify-between gap-2 border-t border-border/50 pt-3 text-[11px]">
          <div className="flex items-center gap-2">
            <span className="font-semibold text-slate-300">
              GÜVENLİK KADEMELERİ ({safetyTriggered}/{maxSo}):
            </span>
            <div className="flex items-center gap-1.5">
              {Array.from({ length: maxSo }).map((_, idx) => {
                const isFilled = idx < safetyTriggered;
                const isNext = idx === safetyTriggered;
                return (
                  <div
                    key={idx}
                    title={`Kademe #${idx + 1} (${isFilled ? "Dolduruldu" : isNext ? "Sırada Bekliyor" : "Planlandı"})`}
                    className={cn(
                      "flex h-4 items-center justify-center rounded px-1.5 text-[9px] font-bold transition-all",
                      isFilled
                        ? "bg-cyan-500/20 text-cyan-300 border border-cyan-500/50 shadow-[0_0_6px_rgba(6,182,212,0.4)]"
                        : isNext
                          ? "bg-amber-500/20 text-amber-300 border border-amber-500/50 animate-pulse"
                          : "bg-slate-800 text-slate-500 border border-slate-700/60"
                    )}
                  >
                    SO{idx + 1}
                  </div>
                );
              })}
            </div>
            {isBagHolding && (
              <span className="rounded bg-rose-500/20 border border-rose-500/40 px-2 py-0.5 text-[10px] font-bold text-rose-300 animate-pulse">
                ⚠️ TÜM KADEMELER TÜKENDİ (BAG HOLDING)
              </span>
            )}
          </div>

          {/* Anlatı Teşhisi (Narrative Diagnostic) */}
          <div className="flex items-center gap-1.5 text-[11px] text-slate-400">
            <Info className="h-3.5 w-3.5 text-cyan-400 shrink-0" />
            <span>
              {metrics.isProfitable
                ? `Fiyat maliyetin %${Math.abs(metrics.devFromAvgPct).toFixed(2)} ${isShort ? "altında" : "üzerinde"} ($${fmtFrac(metrics.cur)}). TP hedefine %${metrics.distToTpPct?.toFixed(2) ?? "0"} kaldı.${
                    marginMode && metrics.liqDistFromCurPct != null ? ` Likidasyon fiyatına %${metrics.liqDistFromCurPct.toFixed(1)} mesafe var (${metrics.liqTier?.label ?? "durum belirsiz"}).` : ""
                  }`
                : `Fiyat maliyetin %${Math.abs(metrics.devFromAvgPct).toFixed(2)} ${isShort ? "üzerinde" : "altında"} seyrediyor ($${fmtFrac(metrics.cur)}).${
                    metrics.nextSo > 0 && !isBagHolding
                      ? ` Sonraki kademe (SO${nextSafetyLevel ?? safetyTriggered + 1}) fiyat %${metrics.nextSoDistPct?.toFixed(2)} ${isShort ? "yükseldiğinde" : "düştüğünde"} ($${fmtFrac(metrics.nextSo)}) tetiklenecek.`
                      : " Tüm güvenlik kademeleri devrede."
                  }${
                    marginMode && metrics.liqDistFromCurPct != null ? ` Likidasyon fiyatına %${metrics.liqDistFromCurPct.toFixed(1)} mesafe var (${metrics.liqTier?.label ?? "durum belirsiz"}).` : ""
                  }`}
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
