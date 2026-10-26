"use client";

/* Faz D — sihirbaz sunum parçaları (state yok; yalnızca prop alır). */
import { Check, CheckCircle2, Clock, MinusCircle, XCircle } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Slider } from "@/components/ui/slider";
import { cn } from "@/lib/utils";

/* ---------------- section wrapper (3Commas single-column flow) ---------------- */

export function WizardSection({
  n,
  title,
  valid,
  hidden = false,
  children,
}: {
  n: number;
  title: string;
  valid: boolean;
  hidden?: boolean;
  children: React.ReactNode;
}) {
  if (hidden) return null;
  return (
    <section id={`wz-sec-${n}`} className="rounded-xl border border-border bg-base/60 p-4" aria-label={`${n}. ${title}`}>
      <header className="mb-3 flex items-center gap-2">
        <span
          className={cn(
            "flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-[11px] font-bold",
            valid ? "bg-emerald-500 text-[#04150e]" : "bg-secondary text-slate-400",
          )}
          aria-hidden
        >
          {valid ? <Check className="h-3.5 w-3.5" aria-hidden /> : n}
        </span>
        <h3 className="text-sm font-bold text-slate-200">{`${n}. ${title}`}</h3>
        {valid && <span className="ml-auto text-[10px] font-semibold text-emerald-500">TAMAM</span>}
      </header>
      {children}
    </section>
  );
}

/* ---------------- small field helpers ---------------- */

export function NumField({ label, value, onChange }: { label: string; value: string; onChange: (v: string) => void }) {
  return (
    <div className="space-y-1.5">
      <Label className="text-xs text-slate-400">{label}</Label>
      <Input
        type="number"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        className="num min-h-11 border-border-strong bg-surface text-slate-100"
      />
    </div>
  );
}

export function SliderField({
  label,
  value,
  min,
  max,
  step,
  onChange,
  integer = false,
}: {
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  onChange: (v: number) => void;
  integer?: boolean;
}) {
  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <Label className="text-xs text-slate-400">{label}</Label>
        <span className="num text-sm font-semibold text-slate-200">{integer ? value : value.toFixed(1)}</span>
      </div>
      <Slider
        value={[value]}
        min={min}
        max={max}
        step={step}
        onValueChange={(v) => onChange(v[0] ?? min)}
        aria-label={label}
      />
    </div>
  );
}

export function Stat({ label, value, tone = "slate" }: { label: string; value: string; tone?: "emerald" | "rose" | "cyan" | "amber" | "slate" }) {
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

export function ReviewRow({ k, v }: { k: string; v: string }) {
  return (
    <>
      <dt className="text-slate-500">{k}</dt>
      <dd className="text-right text-slate-200">{v}</dd>
    </>
  );
}

/* ---------------- gate state primitives ---------------- */

export type GateState = "pass" | "fail" | "wait" | "na";

export function gateMeta(state: GateState): { icon: React.ReactNode; text: string; cls: string } {
  switch (state) {
    case "pass":
      return { icon: <CheckCircle2 className="h-3 w-3" aria-hidden />, text: "PASS", cls: "border-emerald-500/40 bg-emerald-500/10 text-emerald-400" };
    case "fail":
      return { icon: <XCircle className="h-3 w-3" aria-hidden />, text: "HATA", cls: "border-rose-500/50 bg-rose-500/10 text-rose-400" };
    case "wait":
      return { icon: <Clock className="h-3 w-3" aria-hidden />, text: "BEKLE", cls: "border-amber-500/50 bg-amber-500/10 text-amber-400" };
    default:
      return { icon: <MinusCircle className="h-3 w-3" aria-hidden />, text: "N/A", cls: "border-slate-600/50 bg-slate-600/10 text-slate-500" };
  }
}

export function GateChip({ state, label, title }: { state: GateState; label: string; title?: string }) {
  const m = gateMeta(state);
  return (
    <span
      title={title}
      className={cn("inline-flex items-center gap-1 rounded border px-2 py-0.5 text-[10px] font-semibold", m.cls)}
    >
      {m.icon}
      {label} {m.text}
    </span>
  );
}

export function GateRow({ code, desc, state, note }: { code: string; desc: string; state: GateState; note?: string }) {
  const m = gateMeta(state);
  return (
    <li className="flex items-start gap-2 text-xs">
      <span className={cn("shrink-0", m.cls.split(" ").pop())} aria-hidden>
        {m.icon}
      </span>
      <span className={cn("shrink-0 font-mono font-semibold", m.cls.split(" ").pop())}>{code}</span>
      <span className="min-w-0 flex-1 text-slate-400">
        {desc}
        {note && (
          <span className="block truncate text-[10px] text-slate-400" title={note}>
            {note}
          </span>
        )}
      </span>
      <span className={cn("shrink-0 rounded border px-1.5 py-0.5 text-[9px] font-bold", m.cls)}>{m.text}</span>
    </li>
  );
}
