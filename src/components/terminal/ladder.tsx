"use client";

import { AlertTriangle } from "lucide-react";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Badge } from "@/components/ui/badge";
import { EmptyState, ScrollTable } from "@/components/terminal/bits";
import { fmtFrac, fmtPctFrac, fracToNum, kindLabel } from "@/lib/format";
import type { LadderRow } from "@/lib/engine-types";

/**
 * DCA ladder table. currentLevel = engine's next_safety_level:
 * lower levels are already triggered, current is cyan "SIRADAKİ".
 * m20From: first unreachable level (M20 conflict) → rows from there render red.
 * slPrice (+dir): SL fiyatı biliniyorsa M20-beyaz bayrak yerine ayırt edilir —
 * SL ötesi seviyeler "SL ALTINDA" nötr gri gösterilir; kırmızı yalnız gerçek
 * çatışma içindir (UIX-31: varsayılan yüklemede tüm satırların kırmızı duvarı).
 */
export function LadderTable({
  rows,
  currentLevel,
  m20From,
  kind,
  slPrice,
  dir = "long",
  emptyHint,
}: {
  rows: LadderRow[];
  currentLevel?: number | null;
  m20From?: number | null;
  kind?: string;
  slPrice?: number | null;
  dir?: "long" | "short";
  emptyHint?: string;
}) {
  if (!rows || rows.length === 0) {
    return <EmptyState icon={AlertTriangle} title="Ladder verisi yok" hint={emptyHint ?? "Bu strateji için merdiven üretilmedi."} />;
  }
  const hasAvg = rows.some((r) => r.avgAfter !== undefined);
  const beyondSl = (price: number): boolean => {
    if (slPrice == null || slPrice <= 0) return false;
    return dir === "long" ? price < slPrice : price > slPrice;
  };
  return (
    <ScrollTable>
      <Table>
        <TableHeader className="sticky top-0 z-10 bg-surface">
          <TableRow className="hover:bg-transparent border-border">
            <TableHead className="h-9 text-[11px] text-slate-400">SEVİYE</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">FİYAT</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">MİKTAR</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">NOTIONAL</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">KÜM. SAPMA</TableHead>
            {hasAvg && <TableHead className="h-9 text-right text-[11px] text-slate-400">ORT. (WAC)</TableHead>}
            <TableHead className="h-9 text-right text-[11px] text-slate-400">DURUM</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((r) => {
            const cumDev = r.cumDeviation ?? r.cumDev ?? null;
            const p = fracToNum(r.price);
            const unreachable = m20From != null && r.level >= m20From;
            const slBeyond = unreachable && p !== null && beyondSl(p);
            const triggered = currentLevel != null && r.level < currentLevel;
            const isCurrent = currentLevel != null && r.level === currentLevel;
            const isGrid = kind?.includes("GRID");
            return (
              <TableRow
                key={r.level}
                className={
                  isCurrent
                    ? "border-border bg-cyan-500/10 hover:bg-cyan-500/10"
                    : unreachable && !slBeyond
                      ? "border-border bg-rose-500/5 hover:bg-rose-500/10"
                      : "border-border hover:bg-surface-2"
                }
              >
                <TableCell className="num py-2 text-slate-300">{r.level}</TableCell>
                <TableCell className="num py-2 text-right text-slate-100">{fmtFrac(r.price)}</TableCell>
                <TableCell className="num py-2 text-right text-slate-400">{fmtFrac(r.qty)}</TableCell>
                <TableCell className="num py-2 text-right text-slate-400">{fmtFrac(r.notional)}</TableCell>
                <TableCell className="num py-2 text-right text-slate-400">{fmtPctFrac(cumDev)}</TableCell>
                {hasAvg && (
                  <TableCell className="num py-2 text-right text-slate-300">
                    {/* UIX-31: fmtFrac adattivo — l'engine invia avgAfterDisplay a
                        8 decimali; si mostra alla precisione di lettura (2 per
                        i prezzi >=1000). */}
                    {r.avgAfter ? fmtFrac(r.avgAfter) : "—"}
                  </TableCell>
                )}
                <TableCell className="py-2 text-right">
                  {slBeyond ? (
                    /* UIX-31: SL tetiklenmeden dolmayacak seviye — beklenen durum,
                       alarm değil; kırmızı yerine nötr gri. */
                    <Badge variant="outline" className="border-border-strong bg-surface text-[10px] text-slate-400" title="SL bu fiyatın ötesinde — SL tetiklenmeden bu seviye dolmaz (beklenen davranış)">
                      {dir === "short" ? "SL ÜSTÜNDE" : "SL ALTINDA"}
                    </Badge>
                  ) : unreachable ? (
                    <Badge variant="outline" className="border-rose-500/40 bg-rose-500/10 text-[10px] text-rose-400">
                      <AlertTriangle className="mr-1 h-3 w-3" aria-hidden />
                      M20 ULAŞILAMAZ
                    </Badge>
                  ) : triggered ? (
                    <Badge variant="outline" className="border-emerald-500/40 bg-emerald-500/10 text-[10px] text-emerald-400">
                      TETİKLENDİ
                    </Badge>
                  ) : isCurrent || r.armed ? (
                    <Badge variant="outline" className="border-cyan-500/50 bg-cyan-500/10 text-[10px] text-cyan-300">
                      {isGrid ? "ARMED" : "SIRADAKİ"}
                    </Badge>
                  ) : (
                    <span className="text-[10px] text-slate-400">BEKLEMEDE</span>
                  )}
                </TableCell>
              </TableRow>
            );
          })}
        </TableBody>
      </Table>
    </ScrollTable>
  );
}

export function MeshTable({ rows }: { rows: LadderRow[] }) {
  if (!rows || rows.length === 0) {
    return <EmptyState icon={AlertTriangle} title="Mesh boş" hint="Grid parametrelerini kontrol edin." />;
  }
  return (
    <ScrollTable>
      <Table>
        <TableHeader className="sticky top-0 z-10 bg-surface">
          <TableRow className="hover:bg-transparent border-border">
            <TableHead className="h-9 text-[11px] text-slate-400">SEVİYE</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">FİYAT</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">MİKTAR</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">NOTIONAL</TableHead>
            <TableHead className="h-9 text-right text-[11px] text-slate-400">DURUM</TableHead>
          </TableRow>
        </TableHeader>
        <TableBody>
          {rows.map((r) => (
            <TableRow key={r.level} className="border-border hover:bg-surface-2">
                <TableCell className="num py-2 text-slate-300">{r.level}</TableCell>
                <TableCell className="num py-2 text-right text-slate-100">{fmtFrac(r.price)}</TableCell>
                <TableCell className="num py-2 text-right text-slate-400">{fmtFrac(r.qty)}</TableCell>
                <TableCell className="num py-2 text-right text-slate-400">{fmtFrac(r.notional)}</TableCell>
                <TableCell className="py-2 text-right">
                  {r.armed ? (
                  <Badge variant="outline" className="border-cyan-500/50 bg-cyan-500/10 text-[10px] text-cyan-300">
                    ARMED
                  </Badge>
                ) : (
                  <span className="text-[10px] text-slate-400">PASİF</span>
                )}
              </TableCell>
            </TableRow>
          ))}
        </TableBody>
      </Table>
    </ScrollTable>
  );
}

export function KindHint({ kind }: { kind: string }) {
  const hints: Record<string, string> = {
    DCA_LONG: "Düşüşte güvenlik emirleriyle ortalama düşürür, TP'de kâr kapatır.",
    DCA_SHORT: "Yükselişte güvenlik emirleriyle short ortalama yükseltir, TP'de kapatır.",
    FUTURES_DCA: "Kaldıraçlı vadeli DCA Long — izole marjin, M07 likidasyon sınırı ile.",
    SPOT_GRID: "Aralıkta al-sat mesh'i; her seviye maker ücretiyle döner.",
    FUTURES_GRID: "Vadeli aralık mesh'i; kaldıraçlı grid.",
    INFINITY_GRID: "Trend takip eden kayan grid — aralık dışına çıkmaz.",
    HEDGE_DUAL: "Aynı anda long + short korumalı çift pozisyon.",
    HEDGE_PROTECTIVE: "Ana pozisyonu koruyan hedge bacağı üretir.",
  };
  return <p className="text-xs text-slate-500">{hints[kind] ?? kindLabel(kind)}</p>;
}
