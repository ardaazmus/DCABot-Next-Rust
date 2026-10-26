"use client";

import { Loader2, ShieldCheck, ShieldAlert, type LucideIcon } from "lucide-react";
import { Badge } from "@/components/ui/badge";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import { kindLabel } from "@/lib/format";

/* ---------------- palette tokens ---------------- */

export const ACCENT = {
  emerald: "#10B981",
  rose: "#F43F5E",
  cyan: "#06B6D4",
  amber: "#F59E0B",
  slate: "#64748B",
} as const;

export type AccentName = keyof typeof ACCENT;

export function accentText(a: AccentName): string {
  return {
    emerald: "text-emerald-400",
    rose: "text-rose-400",
    cyan: "text-cyan-400",
    amber: "text-amber-400",
    slate: "text-slate-400",
  }[a];
}

export function accentBg(a: AccentName): string {
  return {
    emerald: "bg-emerald-400",
    rose: "bg-rose-400",
    cyan: "bg-cyan-400",
    amber: "bg-amber-400",
    slate: "bg-slate-400",
  }[a];
}

/* ---------------- primitives ---------------- */

export function Spinner({ className }: { className?: string }) {
  return <Loader2 className={cn("h-4 w-4 animate-spin text-slate-500", className)} aria-label="Yükleniyor" />;
}

export function StatusDot({ status, className }: { status: string; className?: string }) {
  const color =
    status === "RUNNING"
      ? "text-emerald-400 bg-emerald-400"
      : status === "PAUSED"
        ? "text-amber-400 bg-amber-400"
        : status === "FROZEN"
          ? "text-sky-400 bg-sky-400"
          : "text-slate-500 bg-slate-500";
  return (
    <span className={cn("relative inline-flex h-2.5 w-2.5 shrink-0", color, className)} aria-hidden>
      {status === "RUNNING" && <span className="dot-ping" />}
    </span>
  );
}

export function KindBadge({ kind, className }: { kind: string; className?: string }) {
  const isLong = kind === "DCA_LONG";
  const isShort = kind === "DCA_SHORT";
  const isFuturesDca = kind === "FUTURES_DCA";
  const isGrid = kind.includes("GRID");
  const isHedge = kind.includes("HEDGE");
  const cls = isLong
    ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400"
    : isShort
      ? "border-rose-500/40 bg-rose-500/10 text-rose-400"
      : isFuturesDca
        ? "border-amber-500/40 bg-amber-500/10 text-amber-400"
        : isGrid
          ? "border-cyan-500/40 bg-cyan-500/10 text-cyan-400"
          : isHedge
            ? "border-amber-500/40 bg-amber-500/10 text-amber-400"
            : "border-slate-500/40 bg-slate-500/10 text-slate-300";
  return (
    <Badge variant="outline" className={cn("text-[10px] font-semibold tracking-wide", cls, className)}>
      {kindLabel(kind).toUpperCase()}
    </Badge>
  );
}

export function StatusPill({ status, className }: { status: string; className?: string }) {
  const cls =
    status === "RUNNING"
      ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400"
      : status === "PAUSED"
        ? "border-amber-500/40 bg-amber-500/10 text-amber-400"
        : status === "FROZEN"
          ? "border-sky-500/40 bg-sky-500/10 text-sky-300"
          : "border-slate-600/50 bg-slate-600/10 text-slate-400";
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-0.5 text-[10px] font-semibold tracking-wide",
        cls,
        className,
      )}
    >
      <StatusDot status={status} />
      {status}
    </span>
  );
}

export function VenueChip({ venue, className }: { venue: string; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center rounded border border-border-strong bg-surface px-1.5 py-0.5 text-[10px] font-mono font-semibold text-slate-300",
        className,
      )}
    >
      {venue}
    </span>
  );
}

export function ModeBadge({ mode, className }: { mode: string; className?: string }) {
  const live = mode === "LIVE";
  return (
    <span
      className={cn(
        "inline-flex items-center rounded border px-1.5 py-0.5 text-[10px] font-bold tracking-wider",
        live
          ? "border-amber-500/50 bg-amber-500/15 text-amber-300"
          : "border-emerald-500/50 bg-emerald-500/15 text-emerald-300",
        className,
      )}
    >
      <span title={live ? "LIVE — gercek borsa emri (3Commas: Real)" : "PAPER — sanal bakiye, risksiz deneme"}>
        {live ? "REAL ⚠" : "PAPER"}
      </span>
    </span>
  );
}

export function SideTag({ side, className }: { side: string; className?: string }) {
  const buy = side === "Buy";
  return (
    <span
      className={cn(
        "inline-flex rounded px-1.5 py-0.5 text-[10px] font-bold",
        buy ? "bg-emerald-500/15 text-emerald-400" : "bg-rose-500/15 text-rose-400",
        className,
      )}
    >
      {buy ? "BUY" : "SELL"}
    </span>
  );
}

const ROLE_COLORS: Record<string, string> = {
  BASE: "border-emerald-500/40 text-emerald-400",
  SAFETY: "border-cyan-500/40 text-cyan-400",
  TP: "border-emerald-500/60 text-emerald-300",
  SL: "border-rose-500/50 text-rose-400",
  MANUAL: "border-slate-500/40 text-slate-300",
  TRAIL: "border-amber-500/40 text-amber-400",
  GRID: "border-cyan-500/40 text-cyan-400",
  INF: "border-cyan-500/40 text-cyan-400",
};

export function RoleChip({ role, className }: { role: string; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex rounded border bg-transparent px-1.5 py-0.5 text-[10px] font-semibold tracking-wide",
        ROLE_COLORS[role] ?? "border-slate-500/40 text-slate-300",
        className,
      )}
    >
      {role}
    </span>
  );
}

const FILL_MODEL_INFO: Record<string, { label: string; desc: string }> = {
  MarketableTaker: {
    label: "MARKETABLE",
    desc: "Emir fiyatlandı ve piyasa tarafına geçti — taker ücreti ile anında doldu (Gate 5).",
  },
  RestingTouch: {
    label: "RESTING",
    desc: "Bekleyen limit emir fiyat dokunuşuyla doldu — maker ücreti (Gate 5).",
  },
  OpenGapBetter: {
    label: "GAP-BETTER",
    desc: "Bar açılışı emir fiyatından daha iyi açıldı — açılış boşluğu fill'i (Gate 8).",
  },
};

export function FillModelTag({ model, className }: { model: string; className?: string }) {
  const info = FILL_MODEL_INFO[model] ?? { label: model.toUpperCase(), desc: model };
  const cls =
    model === "MarketableTaker"
      ? "bg-amber-500/10 text-amber-400 border-amber-500/30"
      : model === "RestingTouch"
        ? "bg-emerald-500/10 text-emerald-400 border-emerald-500/30"
        : "bg-cyan-500/10 text-cyan-400 border-cyan-500/30";
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className={cn("cursor-help rounded border px-1.5 py-0.5 text-[10px] font-semibold", cls, className)}>
          {info.label}
        </span>
      </TooltipTrigger>
      <TooltipContent side="top" className="max-w-64 border-border-strong bg-surface text-xs text-slate-300">
        {info.desc}
      </TooltipContent>
    </Tooltip>
  );
}

/* ---------------- layout helpers ---------------- */

export function Metric({
  label,
  value,
  sub,
  tone = "slate",
  mono = true,
  className,
}: {
  label: string;
  value: React.ReactNode;
  sub?: React.ReactNode;
  tone?: AccentName;
  mono?: boolean;
  className?: string;
}) {
  return (
    <div className={cn("min-w-0", className)}>
      <div className="truncate text-[11px] uppercase tracking-wider text-slate-400">{label}</div>
      <div className={cn("mt-1 truncate text-lg font-semibold", mono && "num", accentText(tone))}>{value}</div>
      {sub !== undefined && <div className="mt-0.5 truncate text-xs text-slate-500">{sub}</div>}
    </div>
  );
}

export function Panel({
  title,
  icon,
  right,
  children,
  className,
  bodyClassName,
}: {
  title: React.ReactNode;
  /** Rendered as-is; pass JSX like <Bot className="h-4 w-4 text-slate-500" />. */
  icon?: React.ReactNode;
  right?: React.ReactNode;
  children: React.ReactNode;
  className?: string;
  bodyClassName?: string;
}) {
  return (
    <section
      className={cn("rounded-xl border border-border bg-[#151a24]", className)}
      aria-label={typeof title === "string" ? title : undefined}
    >
      <header className="flex min-h-11 items-center justify-between gap-2 border-b border-border px-4 py-2">
        <div className="flex items-center gap-2 text-sm font-semibold text-slate-200">
          {icon}
          {title}
        </div>
        {right}
      </header>
      <div className={cn("p-4", bodyClassName)}>{children}</div>
    </section>
  );
}

export function EmptyState({
  icon: Icon,
  title,
  hint,
  className,
}: {
  icon: LucideIcon;
  title: string;
  hint?: string;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col items-center justify-center gap-2 py-10 text-center", className)}>
      <div className="flex h-12 w-12 items-center justify-center rounded-full border border-border-strong bg-surface">
        <Icon className="h-5 w-5 text-slate-500" aria-hidden />
      </div>
      <div className="text-sm font-medium text-slate-300">{title}</div>
      {hint && <div className="max-w-sm text-xs text-slate-500">{hint}</div>}
    </div>
  );
}

export function PassChip({ ok, label, className }: { ok: boolean; label: string; className?: string }) {
  return ok ? (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded border border-emerald-500/40 bg-emerald-500/10 px-2 py-0.5 text-[10px] font-semibold text-emerald-400",
        className,
      )}
    >
      <ShieldCheck className="h-3 w-3" aria-hidden />
      {label} PASS
    </span>
  ) : (
    <span
      className={cn(
        "inline-flex items-center gap-1 rounded border border-amber-500/50 bg-amber-500/10 px-2 py-0.5 text-[10px] font-semibold text-amber-400",
        className,
      )}
    >
      <ShieldAlert className="h-3 w-3" aria-hidden />
      {label} RISK
    </span>
  );
}

/** Scrollable table wrapper: sticky header + custom thin scrollbar. */
export function ScrollTable({ children, className }: { children: React.ReactNode; className?: string }) {
  return (
    <div className={cn("thin-scroll max-h-96 overflow-y-auto rounded-lg border border-border", className)}>
      {children}
    </div>
  );
}
