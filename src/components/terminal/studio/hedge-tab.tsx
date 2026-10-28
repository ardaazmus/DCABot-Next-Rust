"use client";

/* Faz C — Hedge sekmesi (Studio gövdesinden taşındı; yalnızca prop alır). */
import { EmptyState, RoleChip, SideTag } from "@/components/terminal/bits";
import { ShieldHalf } from "lucide-react";
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from "@/components/ui/table";
import { fmtFrac, fmtTs, fmtUsd, fmtUsdSigned } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { BotDetail, Fill, HedgeLegInfo } from "@/lib/engine-types";

/* ---------------- Task 21 — Hedge sekmesi ---------------- */

/** Görsel ilerleme çubuğu (0–100); panel amaçlı, para matematiği DEĞİLDİR. */
function MiniBar({ pct, color }: { pct: number; color: string }) {
  const w = Math.max(0, Math.min(100, pct));
  return (
    <div className="h-1.5 w-full overflow-hidden rounded-full bg-[#1f2637]" role="presentation">
      <div className="h-full rounded-full transition-all" style={{ width: `${w}%`, backgroundColor: color }} />
    </div>
  );
}

function hedgePhaseTr(p: string): string {
  return p === "MainOnly" ? "Sadece Ana Bacak" : p === "HedgeActive" ? "Hedge Aktif" : p === "Closing" ? "Kapanıyor" : p;
}

function LegCard({ title, badge, tone, leg, fallback }: { title: string; badge: string; tone: "emerald" | "rose"; leg: HedgeLegInfo | null; fallback: string }) {
  if (!leg) {
    return (
      <div className="rounded-lg border border-dashed border-border-strong bg-surface/60 p-4 text-center">
        <div className="flex items-center justify-center gap-2 text-xs font-bold text-slate-400">
          {title}
          <span className="rounded border border-border-strong px-1.5 py-0.5 text-[9px] text-slate-400">{badge}</span>
        </div>
        <p className="mt-2 text-[11px] text-slate-400">{fallback}</p>
      </div>
    );
  }
  const pnlNum = Number(leg.pnl);
  const pnlCls = pnlNum > 0 ? "text-emerald-400" : pnlNum < 0 ? "text-rose-400" : "text-slate-300";
  const toneCls = tone === "emerald" ? "border-emerald-500/40 text-emerald-300" : "border-rose-500/40 text-rose-300";
  return (
    <div className="rounded-lg border border-border bg-surface p-4">
      <div className="flex flex-wrap items-center gap-2">
        <span className="text-xs font-bold text-slate-100">{title}</span>
        <span className={cn("rounded border px-1.5 py-0.5 text-[9px] font-semibold", toneCls)}>{badge}</span>
        <span className="rounded border border-border-strong px-1.5 py-0.5 text-[9px] text-slate-400">FAZ: {leg.phase}</span>
      </div>
      <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
        <div>
          <div className="text-[10px] uppercase tracking-wider text-slate-400">POZİSYON</div>
          <div className="num text-sm font-semibold text-slate-100">{fmtFrac(leg.position_qty)}</div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wider text-slate-400">ORT. MALİYET</div>
          <div className="num text-sm font-semibold text-slate-100">{fmtFrac(leg.avg_price)}</div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wider text-slate-400">AÇIK K/Z</div>
          <div className={cn("num text-sm font-semibold", pnlCls)}>{fmtUsdSigned(leg.unrealized_gross ?? "0", 4)}</div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wider text-slate-400">GERÇEKLEŞEN (GROSS)</div>
          <div className="num text-sm font-semibold text-slate-200">{fmtUsdSigned(leg.realized_gross, 4)}</div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wider text-slate-400">BACAK K/Z (TOPLAM)</div>
          <div className={cn("num text-sm font-bold", pnlCls)}>{fmtUsdSigned(leg.pnl, 4)}</div>
        </div>
        <div>
          <div className="text-[10px] uppercase tracking-wider text-slate-400">DEAL (K/K)</div>
          <div className="num text-sm font-semibold text-slate-200">
            {leg.deals_closed} <span className="text-slate-500">({leg.wins}/{leg.losses})</span>
          </div>
        </div>
      </div>
    </div>
  );
}

/**
 * Task 21 — Hedge motoru detay paneli: bacak kartları (main/hedge), birleşik
 * çıkış ilerlemesi, koruma tetikleyici takibi, motor olay günlüğü ve hedge
 * bacağı fill geçmihi. Veri kaynağı: stats.hedge (EngineSnapshot.hedge).
 */
export function HedgeTab({ bot, fills }: { bot: BotDetail; fills: Fill[] }) {
  const h = bot.stats.hedge;
  if (!h) {
    return (
      <EmptyState
        icon={ShieldHalf}
        title="Hedge verisi yok"
        hint="Bu bot eski bir motor sürümüyle açılmış olabilir — snapshot'ta hedge alanı bulunamadı."
      />
    );
  }
  const combinedNum = Number(h.combined_pnl) || 0;
  const targetNum = Number(h.combined_target) || 0;
  const combinedPct = targetNum > 0 ? (combinedNum / targetNum) * 100 : 0;
  const ddNum = h.drawdown_from_peak !== null ? Number(h.drawdown_from_peak) : null;
  const trigNum = Number(h.drawdown_trigger) || 0;
  const ddPct = ddNum !== null && trigNum > 0 ? (ddNum / trigNum) * 100 : 0;
  const hedgeFills = fills.filter((f) => f.orderId.includes("-hedge-"));
  const isDual = h.mode === "DUAL";
  // M-17: ana bacak unrealized'ı bot snapshot unrealizedNow'dan türetilir
  // (alan ana + hedge toplamıdır; hedge bacağı kartı kendi unrealized'ını
  // ayrı gösterdiğinden ana kart için hedge payı çıkarılır).
  const mainRealized = Number(bot.stats.realized_gross) || 0;
  const hedgeUnrealNum = h.hedge_leg?.unrealized_gross != null ? Number(h.hedge_leg.unrealized_gross) || 0 : null;
  const mainUnrealNum = bot.unrealizedNow != null ? Number(bot.unrealizedNow) - (hedgeUnrealNum ?? 0) : null;
  const mainLeg = {
    id: `${bot.id}-main`,
    phase: bot.stats.phase.split("/")[1] ?? bot.stats.phase,
    position_qty: bot.stats.position_qty,
    avg_price: bot.stats.avg_price || null,
    realized_gross: bot.stats.realized_gross,
    unrealized_gross: mainUnrealNum !== null ? String(mainUnrealNum) : null,
    pnl: String(mainRealized + (mainUnrealNum ?? 0)),
    deals_closed: bot.stats.deals_closed,
    wins: bot.stats.wins,
    losses: bot.stats.losses,
  };
  return (
    <div className="space-y-4" data-testid="hedge-tab">
      <div className="flex flex-wrap items-center gap-2">
        <span className="rounded border border-amber-500/40 bg-amber-500/10 px-2 py-0.5 text-[10px] font-bold text-amber-300">
          {isDual ? "HEDGE DUAL — ÇİFT BACAK" : "HEDGE PROTECTIVE — KORUMA"}
        </span>
        <span className="rounded border border-cyan-500/40 bg-cyan-500/10 px-2 py-0.5 text-[10px] font-bold text-cyan-300">FAZ: {hedgePhaseTr(h.phase)}</span>
        <span className="text-[11px] text-slate-400">ρ = {h.hedge_ratio} · spawn sayacı: {h.spawn_count}</span>
      </div>

      <div className="grid gap-3 lg:grid-cols-2">
        <LegCard
          title="ANA BACAK (DCA Long)"
          badge="MAIN"
          tone="emerald"
          leg={mainLeg}
          fallback="Ana bacak bekleniyor."
        />
        <LegCard
          title="HEDGE BACAĞI (Short)"
          badge="HEDGE"
          tone="rose"
          leg={h.hedge_leg}
          fallback={isDual ? "Hedge bacağı ilk dolumda aktifleşir." : "Koruma bacağı henüz spawn olmadı — tetikleyici bekleniyor."}
        />
      </div>

      <div className="rounded-lg border border-border bg-surface p-4">
        <div className="text-[11px] font-bold uppercase tracking-wider text-slate-400">Birleşik Çıkış (M18)</div>
        <div className="mt-2 flex flex-wrap items-baseline gap-2">
          <span className={cn("num text-xl font-bold", combinedNum >= 0 ? "text-emerald-400" : "text-rose-400")}>{fmtUsdSigned(h.combined_pnl, 4)}</span>
          <span className="text-[11px] text-slate-400">/ hedef {fmtUsd(h.combined_target)} — iki bacağın toplam k/z hedefe ulaşınca HER İKİ bacak kapanır.</span>
        </div>
        <div className="mt-2">
          <MiniBar pct={combinedPct} color={combinedNum >= 0 ? "#10b981" : "#f43f5e"} />
          <div className="num mt-1 text-right text-[10px] text-slate-400">%{combinedPct.toFixed(1)}</div>
        </div>
      </div>

      {!isDual && (
        <div className="rounded-lg border border-border bg-surface p-4">
          <div className="text-[11px] font-bold uppercase tracking-wider text-slate-400">Koruma Tetikleyici Takibi</div>
          <div className="mt-2 grid grid-cols-2 gap-2 sm:grid-cols-4">
            <div>
              <div className="text-[10px] uppercase text-slate-400">ZİRVE (MARK)</div>
              <div className="num text-sm font-semibold text-slate-100">{fmtFrac(h.peak_mark)}</div>
            </div>
            <div>
              <div className="text-[10px] uppercase text-slate-400">ZİRVEDEN DÜŞÜŞ</div>
              <div className="num text-sm font-semibold text-amber-300">{ddNum !== null ? `%${(ddNum * 100).toFixed(2)}` : "—"}</div>
            </div>
            <div>
              <div className="text-[10px] uppercase text-slate-400">TETİKLEYİCİ</div>
              <div className="num text-sm font-semibold text-slate-100">%{(trigNum * 100).toFixed(1)}</div>
            </div>
            <div>
              <div className="text-[10px] uppercase text-slate-400">SPAWN SAYACI</div>
              <div className="num text-sm font-semibold text-slate-100">{h.spawn_count}</div>
            </div>
          </div>
          <div className="mt-2">
            <MiniBar pct={ddPct} color="#f59e0b" />
            <div className="num mt-1 text-right text-[10px] text-slate-400">%{ddPct.toFixed(1)} (tetikleyiciye yakınlık)</div>
          </div>
          <p className="mt-2 text-[10px] leading-relaxed text-slate-400">
            Değerler SON İŞLENEN BARA aittir (poll döngüsü ~20 s tazelik). Tetikleyici aşılınca motor gerçek short market emri açar.
          </p>
        </div>
      )}

      <div className="rounded-lg border border-border bg-surface p-4">
        <div className="text-[11px] font-bold uppercase tracking-wider text-slate-400">Motor Olayları (spawn / exit)</div>
        {h.events.length === 0 ? (
          <p className="mt-2 text-[11px] text-slate-400">Henüz hedge olayı yok.</p>
        ) : (
          <div className="thin-scroll mt-2 max-h-40 space-y-1 overflow-y-auto">
            {h.events.map((ev, i) => (
              <div key={i} className="rounded border border-border bg-surface px-2 py-1 font-mono text-[10px] text-slate-300">
                {ev}
              </div>
            ))}
          </div>
        )}
      </div>

      <div className="rounded-lg border border-border bg-surface p-4">
        <div className="text-[11px] font-bold uppercase tracking-wider text-slate-400">Hedge Bacağı Fill Geçmihi ({hedgeFills.length})</div>
        {hedgeFills.length === 0 ? (
          <p className="mt-2 text-[11px] text-slate-400">Hedge bacağı fill yok.</p>
        ) : (
          <div className="thin-scroll mt-2 max-h-64 overflow-y-auto rounded-lg border border-border">
            <Table>
              <TableHeader className="sticky top-0 z-10 bg-surface">
                <TableRow className="hover:bg-transparent border-border">
                  <TableHead className="h-9 text-[11px] text-slate-400">ZAMAN</TableHead>
                  <TableHead className="h-9 text-[11px] text-slate-400">ROL</TableHead>
                  <TableHead className="h-9 text-[11px] text-slate-400">YÖN</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">FİYAT</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">MİKTAR</TableHead>
                  <TableHead className="h-9 text-right text-[11px] text-slate-400">ÜCRET</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {hedgeFills.map((f) => (
                  <TableRow key={f.fillKey} className="border-border hover:bg-surface-2">
                    <TableCell className="num whitespace-nowrap py-2 text-[11px] text-slate-400">{fmtTs(f.ts)}</TableCell>
                    <TableCell className="py-2"><RoleChip role={f.role} /></TableCell>
                    <TableCell className="py-2"><SideTag side={f.side} /></TableCell>
                    <TableCell className="num py-2 text-right text-slate-100">{fmtFrac(f.price)}</TableCell>
                    <TableCell className="num py-2 text-right text-slate-300">{fmtFrac(f.qty)}</TableCell>
                    <TableCell className="num py-2 text-right text-amber-400/90">{fmtFrac(f.fee, 4)}</TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        )}
      </div>
    </div>
  );
}
