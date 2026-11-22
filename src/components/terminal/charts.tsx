"use client";

import { useMemo, useRef, useState } from "react";
import { fmtNum, fmtTs, fmtUsd } from "@/lib/format";
import { cn } from "@/lib/utils";

/* ---------------- Allocation Donut ---------------- */

export interface DonutSlice {
  label: string;
  value: number;
  color: string;
}

export function Donut({
  slices,
  centerLabel,
  centerSub,
  size = 190,
}: {
  slices: DonutSlice[];
  centerLabel?: string;
  centerSub?: string;
  size?: number;
}) {
  const total = slices.reduce((a, s) => a + s.value, 0);
  const r = 70;
  const c = 2 * Math.PI * r;
  const arcs = slices.map((s, i) => {
    const frac = total > 0 ? s.value / total : 0;
    const prior = slices.slice(0, i).reduce((acc, p) => acc + (total > 0 ? p.value / total : 0), 0);
    return { ...s, frac, dash: frac * c, offset: prior * c };
  });

  return (
    <div className="flex flex-col items-center gap-4 sm:flex-row sm:items-center sm:gap-6">
      <div className="relative shrink-0" style={{ width: size, height: size }}>
        <svg viewBox="0 0 180 180" width={size} height={size} role="img" aria-label="Portföy dağılımı">
          <circle cx="90" cy="90" r={r} fill="none" stroke="#1d2433" strokeWidth="16" />
          {arcs.map((a) => (
            <circle
              key={a.label}
              cx="90"
              cy="90"
              r={r}
              fill="none"
              stroke={a.color}
              strokeWidth="16"
              strokeDasharray={`${a.dash} ${c - a.dash}`}
              strokeDashoffset={-a.offset}
              transform="rotate(-90 90 90)"
              strokeLinecap="butt"
            />
          ))}
        </svg>
        <div className="absolute inset-0 flex flex-col items-center justify-center">
          <div className="num text-base font-semibold text-slate-100">{centerLabel ?? "—"}</div>
          <div className="text-[10px] uppercase tracking-wider text-slate-400">{centerSub ?? ""}</div>
        </div>
      </div>
      <p className="sr-only">
        {arcs.length === 0
          ? "Portföy dağılımı: açık pozisyon yok."
          : `Portföy dağılımı: ${arcs.map((a) => `${a.label} %${fmtNum(a.frac * 100, 1)}`).join(", ")}.`}
      </p>
      <ul className="min-w-0 flex-1 space-y-1.5">
        {arcs.length === 0 && <li className="text-xs text-slate-400">Açık pozisyon yok — tahsis %0</li>}
        {arcs.map((a) => (
          <li key={a.label} className="flex items-center gap-2 text-xs">
            <span className="h-2.5 w-2.5 shrink-0 rounded-sm" style={{ background: a.color }} aria-hidden />
            <span className="font-mono text-slate-300">{a.label}</span>
            <span className="num ml-auto text-slate-400">{fmtNum(a.frac * 100, 1)}%</span>
          </li>
        ))}
      </ul>
    </div>
  );
}

/* ---------------- Equity Curve ---------------- */

export interface EquityPoint {
  ts: number;
  equity: number;
}

export function EquityChart({
  points,
  height = 260,
  color = "#06B6D4",
  startLabel,
}: {
  points: EquityPoint[];
  height?: number;
  color?: string;
  startLabel?: string;
}) {
  const wrapRef = useRef<HTMLDivElement>(null);
  const [hoverIdx, setHoverIdx] = useState<number | null>(null);
  /* P4 — klavye gezinmesi + veri tablosu (hover-only degil). */
  const [kbIdx, setKbIdx] = useState<number | null>(null);
  const [showTable, setShowTable] = useState(false);

  const geo = useMemo(() => {
    if (points.length < 2) return null;
    const W = 800;
    const H = 300;
    const padL = 64;
    const padR = 12;
    const padT = 14;
    const padB = 26;
    const xs = points.map((_, i) => padL + (i / (points.length - 1)) * (W - padL - padR));
    const values = points.map((p) => p.equity);
    let min = Math.min(...values);
    let max = Math.max(...values);
    if (min === max) {
      min -= 1;
      max += 1;
    }
    const pad = (max - min) * 0.08;
    min -= pad;
    max += pad;
    const ys = points.map((p) => padT + (1 - (p.equity - min) / (max - min)) * (H - padT - padB));
    const line = points.map((_, i) => `${i === 0 ? "M" : "L"}${xs[i].toFixed(2)},${ys[i].toFixed(2)}`).join(" ");
    const area = `${line} L${xs[xs.length - 1].toFixed(2)},${H - padB} L${xs[0].toFixed(2)},${H - padB} Z`;
    const gridVals = [0, 0.25, 0.5, 0.75, 1].map((f) => min + f * (max - min));
    return { W, H, padL, padR, padT, padB, xs, ys, line, area, min, max, gridVals };
  }, [points]);

  if (!geo) {
    return (
      <div className="flex h-40 items-center justify-center text-xs text-slate-500" style={{ height }}>
        Grafik için yeterli veri yok (en az 2 nokta gerekli)
      </div>
    );
  }

  const onMove = (clientX: number) => {
    const el = wrapRef.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const rel = (clientX - rect.left) / rect.width;
    const svgX = rel * geo.W;
    let best = 0;
    let bestD = Infinity;
    for (let i = 0; i < geo.xs.length; i++) {
      const d = Math.abs(geo.xs[i] - svgX);
      if (d < bestD) {
        bestD = d;
        best = i;
      }
    }
    setHoverIdx(best);
  };

  const first = points[0];
  const last = points[points.length - 1];
  const gid = `eq-grad-${color.replace("#", "")}`;
  const activeIdx = kbIdx ?? hoverIdx;
  const activePt = activeIdx !== null ? points[activeIdx] : null;
  const chg = last && first ? last.equity - first.equity : 0;
  const chgPct = first && first.equity !== 0 ? (chg / first.equity) * 100 : 0;
  const summary = `Özkaynak eğrisi: ${points.length} nokta, başlangıç ${fmtUsd(first?.equity ?? 0)}, bitiş ${fmtUsd(last?.equity ?? 0)}, değişim %${fmtNum(chgPct, 2)}.`;

  return (
    <div
      ref={wrapRef}
      className="relative w-full select-none"
      tabIndex={0}
      role="group"
      aria-label={`${summary} Ok tuşlarıyla nokta gezin.`}
      onMouseMove={(e) => onMove(e.clientX)}
      onMouseLeave={() => setHoverIdx(null)}
      onTouchMove={(e) => {
        if (e.touches.length > 0) onMove(e.touches[0].clientX);
      }}
      onTouchEnd={() => setHoverIdx(null)}
      onKeyDown={(e) => {
        if (e.key === "ArrowRight") {
          setKbIdx((i) => Math.min(points.length - 1, (i ?? -1) + 1));
          e.preventDefault();
        } else if (e.key === "ArrowLeft") {
          setKbIdx((i) => Math.max(0, (i ?? points.length) - 1));
          e.preventDefault();
        } else if (e.key === "Home") {
          setKbIdx(0);
          e.preventDefault();
        } else if (e.key === "End") {
          setKbIdx(points.length - 1);
          e.preventDefault();
        } else if (e.key === "Escape") {
          setKbIdx(null);
        }
      }}
      onBlur={() => setKbIdx(null)}
    >
      <span className="sr-only" role="status">
        {activePt ? `${fmtTs(activePt.ts)}: ${fmtUsd(activePt.equity)}` : summary}
      </span>
      <svg viewBox={`0 0 ${geo.W} ${geo.H}`} width="100%" height={height} preserveAspectRatio="none" aria-hidden="true">
        <defs>
          <linearGradient id={gid} x1="0" y1="0" x2="0" y2="1">
            <stop offset="0%" stopColor={color} stopOpacity="0.28" />
            <stop offset="100%" stopColor={color} stopOpacity="0.02" />
          </linearGradient>
        </defs>
        {geo.gridVals.map((v, i) => {
          const y = geo.padT + (1 - i / 4) * (geo.H - geo.padT - geo.padB);
          return (
            <g key={i}>
              <line x1={geo.padL} x2={geo.W - geo.padR} y1={y} y2={y} stroke="#1f2637" strokeWidth="1" />
              <text x={geo.padL - 6} y={y + 3.5} textAnchor="end" fontSize="10" fill="#5b6679" fontFamily="var(--font-jetbrains-mono), monospace">
                {fmtNum(v, 0)}
              </text>
            </g>
          );
        })}
        <path d={geo.area} fill={`url(#${gid})`} />
        <path d={geo.line} fill="none" stroke={color} strokeWidth="2" />
        {activeIdx !== null && (
          <g>
            <line
              x1={geo.xs[activeIdx]}
              x2={geo.xs[activeIdx]}
              y1={geo.padT}
              y2={geo.H - geo.padB}
              stroke="#3a4661"
              strokeWidth="1"
              strokeDasharray="4 3"
            />
            <circle cx={geo.xs[activeIdx]} cy={geo.ys[activeIdx]} r="4" fill={color} stroke="#0b0e14" strokeWidth="2" />
          </g>
        )}
      </svg>
      <div className="pointer-events-none absolute inset-x-0 bottom-1 flex justify-between px-2 text-[10px] text-slate-400">
        <span className="num">{startLabel ?? fmtTs(first?.ts).split(" ").slice(0, 3).join(" ")}</span>
        <span className="num">{fmtTs(last?.ts).split(" ").slice(0, 3).join(" ")}</span>
      </div>
      {activePt && activeIdx !== null && (
        <div
          className="pointer-events-none absolute z-20 -translate-x-1/2 -translate-y-full rounded-md border border-border-strong bg-surface/95 px-3 py-2 text-xs shadow-xl"
          style={{
            left: `${(geo.xs[activeIdx] / geo.W) * 100}%`,
            top: `${(geo.ys[activeIdx] / geo.H) * height - 8}px`,
          }}
        >
          <div className="num font-semibold text-slate-100">{fmtUsd(activePt.equity)}</div>
          <div className="mt-0.5 text-[10px] text-slate-400">{fmtTs(activePt.ts)}</div>
        </div>
      )}
      <div className="mt-2">
        <button
          type="button"
          onClick={() => setShowTable((v) => !v)}
          aria-expanded={showTable}
          className="rounded-md border border-border px-2.5 py-1.5 text-[11px] font-semibold text-slate-400 hover:text-slate-200"
        >
          {showTable ? "Grafiğe dön" : "Veri tablosu"}
        </button>
      </div>
      {showTable && (
        <div className="thin-scroll mt-2 max-h-56 overflow-auto rounded-md border border-border">
          <table className="w-full text-xs">
            <caption className="sr-only">Özkaynak değerleri tablosu</caption>
            <thead className="sticky top-0 bg-surface">
              <tr className="text-left text-slate-400">
                <th scope="col" className="px-3 py-1.5 font-semibold">Zaman</th>
                <th scope="col" className="px-3 py-1.5 text-right font-semibold">Özkaynak</th>
              </tr>
            </thead>
            <tbody>
              {points.map((p, i) => (
                <tr key={p.ts} className="border-t border-border text-slate-300">
                  <td className="num px-3 py-1">{fmtTs(p.ts)}</td>
                  <td className="num px-3 py-1 text-right">{fmtUsd(p.equity)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </div>
  );
}

/* ---------------- Exposure bar (long vs short) ---------------- */

export function ExposureBar({ longPct, shortPct }: { longPct: number; shortPct: number }) {
  const total = longPct + shortPct;
  const lp = total > 0 ? (longPct / total) * 100 : 0;
  const sp = total > 0 ? (shortPct / total) * 100 : 0;
  return (
    <div className="space-y-1.5" role="img" aria-label={`Uzun ${fmtNum(lp, 1)}% / Kısa ${fmtNum(sp, 1)}% maruziyet`}>
      <div className="flex h-2.5 w-full overflow-hidden rounded-full bg-secondary">
        <div className="h-full bg-emerald-500/80 transition-all" style={{ width: `${lp}%` }} />
        <div className={cn("h-full bg-rose-500/80 transition-all", lp === 0 && sp === 0 && "hidden")} style={{ width: `${sp}%` }} />
      </div>
      <div className="flex justify-between text-[10px] text-slate-400">
        <span className="text-emerald-400">LONG {fmtNum(lp, 1)}%</span>
        <span className="text-rose-400">SHORT {fmtNum(sp, 1)}%</span>
      </div>
    </div>
  );
}
