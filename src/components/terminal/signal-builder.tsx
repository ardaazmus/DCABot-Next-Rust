"use client";

/**
 * SignalBuilder — 3Commas-style visual "Deal Start Conditions" builder.
 * Produces a SignalConfig tree (dca_signals wire format, see
 * @/lib/engine-types "3Commas-style Signal Engine"). Embedded in the
 * wizard's entry-conditions section by the orchestrator.
 *
 * Conventions: dark terminal palette (emerald AND / amber OR / rose miss),
 * Turkish labels everywhere, gateway fetch (engineUrl → ?XTransformPort=3002),
 * soft client-side validation only (backend 422 stays the hard gate).
 */

import { Fragment, useEffect, useId, useMemo, useState } from "react";
import { FolderPlus, Plus, Trash2, Zap } from "lucide-react";
import type {
  CompareOp,
  IndicatorKey,
  IndicatorOutput,
  IndicatorSpec,
  Interval,
  SignalCondition,
  SignalConfig,
  SignalGroup,
  SignalNode,
  SignalPreviewResponse,
  SignalValue,
  Timeframe,
} from "@/lib/engine-types";
import { isSignalGroup } from "@/lib/engine-types";
import { engineFetch, engineJson, useDebouncedValue } from "@/hooks/use-engine";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { cn } from "@/lib/utils";

/* ================================================================== */
/* Props                                                              */
/* ================================================================== */

export interface SignalBuilderProps {
  value: SignalConfig | null; // null = signal engine off (ASAP/simple mode)
  onChange: (next: SignalConfig | null) => void;
  symbol: string;
  baseInterval: Interval; // bot execution interval
  /** Venue used for the live preview request — defaults to "BYBIT". */
  venue?: string;
  /** Optional sink for the latest live-preview outcome (wizard gate checklist consumes it). */
  onPreviewChange?: (result: SignalPreviewResponse | null, error: string | null) => void;
}

/* ================================================================== */
/* Domain tables (mirror dca_signals IndicatorSpec semantics)          */
/* ================================================================== */

const TF_BY_INTERVAL: Record<Interval, Timeframe> = {
  "1m": "M1",
  "3m": "M1",
  "5m": "M5",
  "15m": "M15",
  "30m": "M15",
  "1h": "H1",
  "2h": "H1",
  "4h": "H4",
  "1d": "D1",
};

const TIMEFRAMES: Timeframe[] = ["M1", "M5", "M15", "H1", "H4", "D1"];

type Family = IndicatorKey;

const FAMILIES: Family[] = [
  "Rsi",
  "Ema",
  "Sma",
  "Macd",
  "Bb",
  "Stoch",
  "Atr",
  "Adx",
  "Vol",
  "Price",
];

const FAMILY_LABEL: Record<Family, string> = {
  Rsi: "RSI",
  Ema: "EMA",
  Sma: "SMA",
  Macd: "MACD",
  Bb: "Bollinger",
  Stoch: "Stochastic",
  Atr: "ATR",
  Adx: "ADX",
  Vol: "Hacim",
  Price: "Fiyat",
};

const OP_OPTIONS: { value: CompareOp; label: string }[] = [
  { value: "Lt", label: "< Küçüktür" },
  { value: "Lte", label: "≤" },
  { value: "Gt", label: "> Büyüktür" },
  { value: "Gte", label: "≥" },
  { value: "CrossUp", label: "⤯ Yukarı Keser (CrossUp)" },
  { value: "CrossDown", label: "⤡ Aşağı Keser (CrossDown)" },
];

const OP_SYMBOL: Record<CompareOp, string> = {
  Lt: "<",
  Lte: "≤",
  Gt: ">",
  Gte: "≥",
  CrossUp: "⤯",
  CrossDown: "⤡",
};

const MAX_CHILDREN_PER_GROUP = 6;
const MAX_DEPTH = 3;

/** Family defaults — reset spec on family change (brief table). */
function defaultSpec(family: Family, tf: Timeframe): IndicatorSpec {
  switch (family) {
    case "Rsi":
      return { key: "Rsi", timeframe: tf, period: 14 };
    case "Ema":
      return { key: "Ema", timeframe: tf, period: 20 };
    case "Sma":
      return { key: "Sma", timeframe: tf, period: 20 };
    case "Macd":
      return { key: "Macd", timeframe: tf, fast: 12, slow: 26, signal: 9, output: "macd" };
    case "Bb":
      return { key: "Bb", timeframe: tf, period: 20, std_dev: 2, output: "lower" };
    case "Stoch":
      return { key: "Stoch", timeframe: tf, period: 14, d_period: 3, output: "k" };
    case "Atr":
      return { key: "Atr", timeframe: tf, period: 14, output: "atr_abs" };
    case "Adx":
      return { key: "Adx", timeframe: tf, period: 14, output: "default" };
    case "Vol":
      return { key: "Vol", timeframe: tf, period: 20, output: "ratio" };
    case "Price":
      return { key: "Price", timeframe: tf, output: "close" };
  }
}

/** Right-side indicator default — EMA(50) on the left side's timeframe. */
function defaultRightIndicator(tf: Timeframe): IndicatorSpec {
  return { ...defaultSpec("Ema", tf), period: 50 };
}

function defaultCondition(tf: Timeframe): SignalCondition {
  return {
    left: { key: "Rsi", timeframe: tf, period: 14 },
    op: "Lt",
    right: { kind: "number", value: 30 },
  };
}

function defaultConfig(tf: Timeframe): SignalConfig {
  return { root: { op: "And", children: [defaultCondition(tf)] } };
}

/* -- compact numeric param fields per family -- */

interface ParamField {
  field: "period" | "fast" | "slow" | "signal" | "std_dev" | "d_period";
  label: string;
  min: number;
  max: number;
  step?: number;
}

const PARAM_FIELDS: Record<Family, ParamField[]> = {
  Rsi: [{ field: "period", label: "Periyot", min: 2, max: 100 }],
  Ema: [{ field: "period", label: "Periyot", min: 2, max: 500 }],
  Sma: [{ field: "period", label: "Periyot", min: 2, max: 500 }],
  Macd: [
    { field: "fast", label: "Hızlı", min: 2, max: 200 },
    { field: "slow", label: "Yavaş", min: 2, max: 200 },
    { field: "signal", label: "Sinyal", min: 2, max: 200 },
  ],
  Bb: [
    { field: "period", label: "Periyot", min: 2, max: 500 },
    { field: "std_dev", label: "Std Sapma", min: 0.5, max: 5, step: 0.1 },
  ],
  Stoch: [
    { field: "period", label: "Periyot", min: 2, max: 500 },
    { field: "d_period", label: "D Periyodu", min: 1, max: 20 },
  ],
  Atr: [{ field: "period", label: "Periyot", min: 2, max: 500 }],
  Adx: [{ field: "period", label: "Periyot", min: 2, max: 100 }],
  Vol: [{ field: "period", label: "Periyot", min: 2, max: 500 }],
  Price: [],
};

/** Output select shown only where the family has multiple outputs. */
const OUTPUT_OPTIONS: Partial<Record<Family, { value: IndicatorOutput; label: string }[]>> = {
  Macd: [
    { value: "macd", label: "macd" },
    { value: "signal", label: "sinyal" },
    { value: "hist", label: "histogram" },
  ],
  Bb: [
    { value: "lower", label: "alt" },
    { value: "mid", label: "orta" },
    { value: "upper", label: "üst" },
    { value: "bandwidth", label: "bant genişliği %" },
  ],
  Stoch: [
    { value: "k", label: "%K" },
    { value: "d", label: "%D" },
  ],
  Atr: [
    { value: "atr_abs", label: "mutlak" },
    { value: "atr_pct", label: "%" },
  ],
  Price: [
    { value: "close", label: "kapanış" },
    { value: "change_pct", label: "değişim %" },
  ],
};

/* -- one-click presets (apply replaces the whole tree) -- */

function cond(
  left: IndicatorSpec,
  op: CompareOp,
  right: SignalValue,
): SignalCondition {
  return { left, op, right };
}
function num(v: number): SignalValue {
  return { kind: "number", value: v };
}
function ind(spec: IndicatorSpec): SignalValue {
  return { kind: "indicator", value: spec };
}
function grp(op: "And" | "Or", children: SignalNode[]): SignalGroup {
  return { op, children };
}

interface Preset {
  label: string;
  build: (tf: Timeframe) => SignalGroup;
}

const PRESETS: Preset[] = [
  {
    label: "RSI Dibi",
    build: (tf) => grp("And", [cond({ key: "Rsi", timeframe: tf, period: 14 }, "Lt", num(30))]),
  },
  {
    label: "RSI + Hacim",
    build: (tf) =>
      grp("And", [
        cond({ key: "Rsi", timeframe: tf, period: 14 }, "Lt", num(35)),
        cond({ key: "Vol", timeframe: tf, period: 20, output: "ratio" }, "Gt", num(1.5)),
      ]),
  },
  {
    label: "MACD Yükseliş",
    build: (tf) =>
      grp("And", [
        cond(
          { key: "Macd", timeframe: tf, fast: 12, slow: 26, signal: 9, output: "macd" },
          "CrossUp",
          ind({ key: "Macd", timeframe: tf, fast: 12, slow: 26, signal: 9, output: "signal" }),
        ),
      ]),
  },
  {
    label: "EMA Altın Kesişim",
    build: (tf) =>
      grp("And", [
        cond({ key: "Ema", timeframe: tf, period: 20 }, "CrossUp", ind({ key: "Ema", timeframe: tf, period: 50 })),
      ]),
  },
  {
    label: "Bollinger Dönüşü",
    build: (tf) =>
      grp("And", [
        cond(
          { key: "Price", timeframe: tf, output: "close" },
          "CrossUp",
          ind({ key: "Bb", timeframe: tf, period: 20, std_dev: 2, output: "lower" }),
        ),
      ]),
  },
  {
    label: "Stokastik Dönüş",
    build: (tf) =>
      grp("And", [
        cond(
          { key: "Stoch", timeframe: tf, period: 14, d_period: 3, output: "k" },
          "CrossUp",
          ind({ key: "Stoch", timeframe: tf, period: 14, d_period: 3, output: "d" }),
        ),
        cond({ key: "Stoch", timeframe: tf, period: 14, d_period: 3, output: "k" }, "Lt", num(20)),
      ]),
  },
];

/* ================================================================== */
/* Immutable tree ops (root is always a SignalGroup)                  */
/* ================================================================== */

function updateNodeAt(
  root: SignalGroup,
  path: number[],
  fn: (n: SignalNode) => SignalNode,
): SignalGroup {
  if (path.length === 0) {
    const next = fn(root);
    return isSignalGroup(next) ? next : { op: "And", children: [next] };
  }
  const [head, ...rest] = path;
  return {
    op: root.op,
    children: root.children.map((c, i) => {
      if (i !== head) return c;
      if (rest.length === 0) return fn(c);
      return isSignalGroup(c) ? updateNodeAt(c, rest, fn) : c;
    }),
  };
}

/* ================================================================== */
/* describe util — mirrors Rust describe() semantics textually        */
/* ================================================================== */

function fmtMetric(v: number | null | undefined): string {
  if (v === null || v === undefined || !Number.isFinite(v)) return "—";
  if (Math.abs(v) < 1e-9) return "0"; // trim float dust
  return new Intl.NumberFormat("en-US", { maximumFractionDigits: 4 }).format(v);
}

function describeIndicator(s: IndicatorSpec): string {
  switch (s.key) {
    case "Rsi":
      return `RSI(${s.period ?? 14},${s.timeframe})`;
    case "Ema":
      return `EMA(${s.period ?? 20},${s.timeframe})`;
    case "Sma":
      return `SMA(${s.period ?? 20},${s.timeframe})`;
    case "Macd":
      return `MACD(${s.fast ?? 12},${s.slow ?? 26},${s.signal ?? 9},${s.timeframe}).${s.output ?? "macd"}`;
    case "Bb":
      return `BB(${s.period ?? 20},${s.std_dev ?? 2},${s.timeframe}).${s.output ?? "lower"}`;
    case "Stoch":
      return `STOCH(${s.period ?? 14},${s.d_period ?? 3},${s.timeframe}).${s.output ?? "k"}`;
    case "Atr":
      return `ATR(${s.period ?? 14},${s.timeframe}).${s.output ?? "atr_abs"}`;
    case "Adx":
      return `ADX(${s.period ?? 14},${s.timeframe})`;
    case "Vol":
      return `VOL(${s.period ?? 20},${s.timeframe}).${s.output ?? "ratio"}`;
    case "Price":
      return `PRICE(${s.timeframe}).${s.output ?? "close"}`;
    default:
      return String(s.key);
  }
}

function describeValue(v: SignalValue): string {
  return v.kind === "number" ? fmtMetric(v.value) : describeIndicator(v.value);
}

function describeCondition(c: SignalCondition): string {
  return `${describeIndicator(c.left)} ${OP_SYMBOL[c.op]} ${describeValue(c.right)}`;
}

function describeNode(n: SignalNode): string {
  if (isSignalGroup(n)) {
    return `(${n.children.map(describeNode).join(n.op === "And" ? " AND " : " OR ")})`;
  }
  return describeCondition(n);
}

/* ================================================================== */
/* Small building blocks                                              */
/* ================================================================== */

/** Numeric input with local draft — typing never blocked, clamped on blur. */
function NumInput(props: {
  id: string;
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number | "any";
  className?: string;
  onCommit: (v: number) => void;
}) {
  const { id, label, value, min, max, step = 1, className, onCommit } = props;
  const [draft, setDraft] = useState<string>(String(value));

  useEffect(() => {
    setDraft(String(value));
  }, [value]);

  const commit = () => {
    const n = Number(draft.replace(",", "."));
    const next = Number.isFinite(n) ? Math.min(max, Math.max(min, n)) : value;
    onCommit(next);
    setDraft(String(next));
  };

  return (
    <div className="flex flex-col gap-0.5">
      <Label htmlFor={id} className="text-[9px] font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </Label>
      <Input
        id={id}
        type="number"
        inputMode="decimal"
        min={min}
        max={max}
        step={step}
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => {
          if (e.key === "Enter") (e.target as HTMLInputElement).blur();
        }}
        className={cn("h-7 px-1.5 text-xs tabular-nums", className)}
      />
    </div>
  );
}

/** Family select + compact params + output select + timeframe select. */
function SpecEditor(props: {
  idBase: string;
  spec: IndicatorSpec;
  compact?: boolean;
  onSpecChange: (next: IndicatorSpec) => void;
}) {
  const { idBase, spec, compact = false, onSpecChange } = props;
  const family = spec.key;
  const fields = PARAM_FIELDS[family];
  const outputs = OUTPUT_OPTIONS[family];
  const triggerCls = "h-7 text-[11px]";

  return (
    <div className="flex flex-wrap items-end gap-x-1.5 gap-y-1.5">
      <Select value={family} onValueChange={(v) => onSpecChange(defaultSpec(v as Family, spec.timeframe))}>
        <SelectTrigger
          size="sm"
          className={cn(triggerCls, compact ? "w-[92px]" : "w-[110px]")}
          aria-label="Gösterge ailesi"
        >
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {FAMILIES.map((f) => (
            <SelectItem key={f} value={f} className="text-xs">
              {FAMILY_LABEL[f]}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>

      {fields.map((pf) => (
        <NumInput
          key={pf.field}
          id={`${idBase}-${pf.field}`}
          label={pf.label}
          value={spec[pf.field] ?? pf.min}
          min={pf.min}
          max={pf.max}
          step={pf.step}
          className="w-[60px]"
          onCommit={(v) => onSpecChange({ ...spec, [pf.field]: v })}
        />
      ))}

      {outputs && (
        <Select
          value={spec.output ?? outputs[0].value}
          onValueChange={(v) => onSpecChange({ ...spec, output: v as IndicatorOutput })}
        >
          <SelectTrigger size="sm" className={cn(triggerCls, "w-[104px]")} aria-label="Çıktı">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {outputs.map((o) => (
              <SelectItem key={o.value} value={o.value} className="text-xs">
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      )}

      <Select value={spec.timeframe} onValueChange={(v) => onSpecChange({ ...spec, timeframe: v as Timeframe })}>
        <SelectTrigger size="sm" className={cn(triggerCls, "w-[62px]")} aria-label="Zaman dilimi">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          {TIMEFRAMES.map((t) => (
            <SelectItem key={t} value={t} className="text-xs">
              {t}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  );
}

/** AND/OR gate pill between sibling cards — clicking toggles the group op. */
function GatePill({ isAnd, onToggle }: { isAnd: boolean; onToggle: () => void }) {
  return (
    <div className="my-1 flex items-center gap-2 pl-1">
      <div className="flex flex-col items-center">
        <span className="h-1.5 w-px bg-border" aria-hidden />
        <button
          type="button"
          onClick={onToggle}
          aria-label={isAnd ? "Mantık kapısını VEYA'ya çevir" : "Mantık kapısını VE'ye çevir"}
          title="VE ↔ VEYA"
          className={cn(
            "rounded-full border px-2.5 py-0.5 text-[10px] font-bold tracking-widest transition-colors",
            isAnd
              ? "border-emerald-500/50 bg-emerald-500/10 text-emerald-400 hover:bg-emerald-500/20"
              : "border-amber-500/50 bg-amber-500/10 text-amber-400 hover:bg-amber-500/20",
          )}
        >
          {isAnd ? "VE" : "VEYA"}
        </button>
        <span className="h-1.5 w-px bg-border" aria-hidden />
      </div>
    </div>
  );
}

/* ================================================================== */
/* Recursive node view                                                */
/* ================================================================== */

interface NodeViewProps {
  node: SignalNode;
  path: number[];
  depth: number;
  isRoot: boolean;
  baseTf: Timeframe;
  canDelete: boolean; // parent group has >1 children
  onNodeChange: (path: number[], next: SignalNode) => void;
  onDelete: (path: number[]) => void;
  onToggleGroupOp: (path: number[]) => void;
  onAddCondition: (groupPath: number[]) => void;
  onAddNestedGroup: () => void;
}

function NodeView(props: NodeViewProps) {
  const { node, path, depth, isRoot, baseTf, canDelete } = props;

  if (!isSignalGroup(node)) {
    return (
      <ConditionRow
        cond={node}
        path={path}
        baseTf={baseTf}
        canDelete={canDelete}
        onNodeChange={props.onNodeChange}
        onDelete={props.onDelete}
      />
    );
  }

  const isAnd = node.op === "And";
  const canNestGroup = isRoot && depth + 1 <= MAX_DEPTH;

  return (
    <div
      className={cn(
        "space-y-1 rounded-md border border-border/50 border-l-2 py-2 pl-3 pr-2",
        isAnd ? "border-l-emerald-500/70" : "border-l-amber-500/70",
      )}
    >
      <div className="mb-1 flex flex-wrap items-center gap-1.5">
        <span
          className={cn(
            "inline-flex items-center rounded-full border px-2 py-0.5 text-[10px] font-semibold tracking-wide",
            isAnd
              ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400"
              : "border-amber-500/40 bg-amber-500/10 text-amber-400",
          )}
        >
          {isAnd ? "VE — tümü sağlanmalı" : "VEYA — herhangi biri"}
        </span>
        <span className="grow" aria-hidden />
        {isRoot && (
          <Button
            variant="outline"
            size="sm"
            className="h-6 gap-1 px-2 text-[11px]"
            onClick={props.onAddNestedGroup}
            disabled={!canNestGroup}
            title="Yeni alt grup (2 varsayılan koşul)"
          >
            <FolderPlus className="h-3 w-3" aria-hidden />
            Grup Ekle
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          className="h-6 gap-1 px-2 text-[11px]"
          onClick={() => props.onAddCondition(path)}
          disabled={node.children.length >= MAX_CHILDREN_PER_GROUP}
          title={`Grup başına en fazla ${MAX_CHILDREN_PER_GROUP} koşul`}
        >
          <Plus className="h-3 w-3" aria-hidden />
          Koşul
        </Button>
      </div>

      {node.children.map((child, i) => (
        <Fragment key={i}>
          {i > 0 && <GatePill isAnd={isAnd} onToggle={() => props.onToggleGroupOp(path)} />}
          <NodeView
            node={child}
            path={[...path, i]}
            depth={depth + 1}
            isRoot={false}
            baseTf={baseTf}
            canDelete={node.children.length > 1}
            onNodeChange={props.onNodeChange}
            onDelete={props.onDelete}
            onToggleGroupOp={props.onToggleGroupOp}
            onAddCondition={props.onAddCondition}
            onAddNestedGroup={props.onAddNestedGroup}
          />
        </Fragment>
      ))}
    </div>
  );
}

/* ================================================================== */
/* Condition (leaf) row                                               */
/* ================================================================== */

function ConditionRow(props: {
  cond: SignalCondition;
  path: number[];
  baseTf: Timeframe;
  canDelete: boolean;
  onNodeChange: (path: number[], next: SignalNode) => void;
  onDelete: (path: number[]) => void;
}) {
  const { cond, path, canDelete, onNodeChange, onDelete } = props;
  const idBase = useId();
  const isCross = cond.op === "CrossUp" || cond.op === "CrossDown";
  const crossMismatch =
    isCross && cond.right.kind === "indicator" && cond.right.value.timeframe !== cond.left.timeframe;

  const setRight = (right: SignalValue) => onNodeChange(path, { ...cond, right });

  const switchRightSide = (kind: "number" | "indicator") => {
    if (cond.right.kind === kind) return;
    if (kind === "number") {
      setRight(num(cond.left.key === "Rsi" ? 30 : 1));
    } else {
      setRight(ind(defaultRightIndicator(cond.left.timeframe)));
    }
  };

  return (
    <div className="rounded-md border border-border/50 bg-card/40 p-2">
      <div className="flex flex-wrap items-end gap-x-2 gap-y-2">
        <SpecEditor
          idBase={`${idBase}-l`}
          spec={cond.left}
          onSpecChange={(s) => onNodeChange(path, { ...cond, left: s })}
        />

        <Select
          value={cond.op}
          onValueChange={(v) => onNodeChange(path, { ...cond, op: v as CompareOp })}
        >
          <SelectTrigger size="sm" className="h-7 w-[152px] text-[11px]" aria-label="Operatör">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {OP_OPTIONS.map((o) => (
              <SelectItem key={o.value} value={o.value} className="text-xs">
                {o.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>

        <div className="flex items-end gap-1.5">
          <div
            role="group"
            aria-label="Sağ taraf türü"
            className="inline-flex h-7 overflow-hidden rounded-md border border-border/60"
          >
            <button
              type="button"
              aria-pressed={cond.right.kind === "number"}
              onClick={() => switchRightSide("number")}
              className={cn(
                "px-2 text-[11px] font-medium transition-colors",
                cond.right.kind === "number"
                  ? "bg-primary/15 text-primary"
                  : "text-muted-foreground hover:bg-muted/40",
              )}
            >
              Sayı
            </button>
            <button
              type="button"
              aria-pressed={cond.right.kind === "indicator"}
              onClick={() => switchRightSide("indicator")}
              className={cn(
                "px-2 text-[11px] font-medium transition-colors",
                cond.right.kind === "indicator"
                  ? "bg-primary/15 text-primary"
                  : "text-muted-foreground hover:bg-muted/40",
              )}
            >
              Gösterge
            </button>
          </div>

          {cond.right.kind === "number" ? (
            <NumInput
              id={`${idBase}-r`}
              label="Değer"
              value={cond.right.value}
              min={-1e12}
              max={1e12}
              step="any"
              className="w-[96px]"
              onCommit={(v) => setRight(num(v))}
            />
          ) : (
            <SpecEditor
              idBase={`${idBase}-ri`}
              spec={cond.right.value}
              compact
              onSpecChange={(s) => setRight(ind(s))}
            />
          )}
        </div>

        <Button
          variant="ghost"
          size="icon"
          className="h-7 w-7 shrink-0 text-muted-foreground hover:text-rose-400"
          disabled={!canDelete}
          aria-label="Koşulu sil"
          title={canDelete ? "Koşulu sil" : "Her grup en az 1 koşul içermelidir"}
          onClick={() => onDelete(path)}
        >
          <Trash2 className="h-3.5 w-3.5" aria-hidden />
        </Button>
      </div>

      {crossMismatch && (
        <p className="mt-1.5 text-[10px] text-amber-400">
          Kesişim: sağ taraf aynı zaman diliminde olmalı
        </p>
      )}
    </div>
  );
}

/* ================================================================== */
/* Main builder                                                       */
/* ================================================================== */

export function SignalBuilder({ value, onChange, symbol, baseInterval, venue = "BYBIT", onPreviewChange }: SignalBuilderProps) {
  const baseTf = TF_BY_INTERVAL[baseInterval];
  const switchId = useId();
  const engineOn = value !== null;

  /** Root is ALWAYS a group — wrap bare incoming conditions for rendering. */
  const root: SignalGroup | null = useMemo(() => {
    if (!value) return null;
    return isSignalGroup(value.root) ? value.root : { op: "And", children: [value.root] };
  }, [value]);

  const emit = (nextRoot: SignalGroup) => onChange({ root: nextRoot });

  const handleNodeChange = (path: number[], next: SignalNode) => {
    if (!root) return;
    emit(updateNodeAt(root, path, () => next));
  };

  const handleDelete = (path: number[]) => {
    if (!root) return;
    const parentPath = path.slice(0, -1);
    const idx = path[path.length - 1];
    emit(
      updateNodeAt(root, parentPath, (n) => {
        if (!isSignalGroup(n) || n.children.length <= 1) return n; // min 1 leaf per group
        return { op: n.op, children: n.children.filter((_, i) => i !== idx) };
      }),
    );
  };

  const handleToggleGroupOp = (path: number[]) => {
    if (!root) return;
    emit(
      updateNodeAt(root, path, (n) =>
        isSignalGroup(n) ? { op: n.op === "And" ? "Or" : "And", children: n.children } : n,
      ),
    );
  };

  const handleAddCondition = (groupPath: number[]) => {
    if (!root) return;
    emit(
      updateNodeAt(root, groupPath, (n) => {
        if (!isSignalGroup(n) || n.children.length >= MAX_CHILDREN_PER_GROUP) return n;
        return { op: n.op, children: [...n.children, defaultCondition(baseTf)] };
      }),
    );
  };

  const handleAddNestedGroup = () => {
    if (!root) return;
    if (root.children.length >= MAX_CHILDREN_PER_GROUP) return;
    emit({
      op: root.op,
      children: [
        ...root.children,
        { op: "And", children: [defaultCondition(baseTf), defaultCondition(baseTf)] },
      ],
    });
  };

  /* ---------------- live preview (debounced 800ms) ---------------- */

  const previewKey = useMemo(
    () => JSON.stringify({ symbol, interval: baseInterval, venue, node: root }),
    [symbol, baseInterval, venue, root],
  );
  const debouncedPreviewKey = useDebouncedValue(previewKey, 800);

  const [preview, setPreview] = useState<SignalPreviewResponse | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);

  useEffect(() => {
    if (!engineOn) {
      setPreview(null);
      setPreviewError(null);
      onPreviewChange?.(null, null);
      return;
    }
    let parsed: { symbol: string; interval: Interval; venue: string; node: SignalNode };
    try {
      parsed = JSON.parse(debouncedPreviewKey) as { symbol: string; interval: Interval; venue: string; node: SignalNode };
    } catch {
      return;
    }
    if (!parsed.node) return;

    let alive = true;
    setPreviewLoading(true);
    (async () => {
      try {
        const res = await engineFetch<SignalPreviewResponse>(
          "/api/signals/preview",
          engineJson("POST", {
            symbol: parsed.symbol,
            interval: parsed.interval,
            venue: parsed.venue,
            node: parsed.node,
          }),
        );
        if (!alive) return;
        if (res && typeof res === "object" && "error" in res && typeof res.error === "string" && res.error) {
          setPreview(null);
          setPreviewError(res.error);
          onPreviewChange?.(null, res.error);
        } else {
          setPreview(res);
          setPreviewError(null);
          onPreviewChange?.(res, null);
        }
      } catch (e) {
        if (!alive) return;
        setPreview(null);
        setPreviewError(e instanceof Error ? e.message : "canlı önizleme yok");
        onPreviewChange?.(null, e instanceof Error ? e.message : "canlı önizleme yok");
      } finally {
        if (alive) setPreviewLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, [debouncedPreviewKey, engineOn, onPreviewChange]);

  /* ---------------- render ---------------- */

  return (
    <div className="space-y-3">
      {/* 1 — header row */}
      <div className="flex flex-wrap items-center gap-2">
        <Switch
          id={switchId}
          checked={engineOn}
          onCheckedChange={(c) => onChange(c ? defaultConfig(baseTf) : null)}
        />
        <Label htmlFor={switchId} className="text-xs font-semibold">
          Sinyal Motoru
        </Label>
        {engineOn && root && (
          <Badge
            variant="outline"
            className={cn(
              "text-[9px] font-semibold tracking-wide",
              root.op === "And"
                ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400"
                : "border-amber-500/40 bg-amber-500/10 text-amber-400",
            )}
          >
            KÖK: {root.op === "And" ? "VE" : "VEYA"}
          </Badge>
        )}
        {!engineOn && (
          <p className="text-xs text-muted-foreground">
            Sinyal motoru kapalı — bot anında (ASAP) veya basit koşulla başlar.
          </p>
        )}
      </div>

      {engineOn && root && (
        <>
          {/* 6 — one-click 3Commas presets */}
          <div className="flex flex-wrap items-center gap-1.5">
            <span className="text-[10px] font-semibold tracking-widest text-muted-foreground">
              HAZIR ŞABLONLAR
            </span>
            {PRESETS.map((p) => (
              <button
                key={p.label}
                type="button"
                onClick={() => onChange({ root: p.build(baseTf) })}
                className="inline-flex items-center gap-1 rounded-full border border-border/60 bg-card/50 px-2 py-0.5 text-[10px] font-medium text-slate-300 transition-colors hover:border-amber-500/40 hover:bg-amber-500/10 hover:text-amber-300"
                title="Tüm koşul ağacının üzerine yazar"
              >
                <Zap className="h-3 w-3" aria-hidden />
                {p.label}
              </button>
            ))}
          </div>

          {/* 2–5 — the tree */}
          <div className="max-h-[560px] overflow-y-auto pr-1">
            <NodeView
              node={root}
              path={[]}
              depth={1}
              isRoot
              baseTf={baseTf}
              canDelete={false}
              onNodeChange={handleNodeChange}
              onDelete={handleDelete}
              onToggleGroupOp={handleToggleGroupOp}
              onAddCondition={handleAddCondition}
              onAddNestedGroup={handleAddNestedGroup}
            />
          </div>

          {/* 8 — summary line */}
          <p
            className="truncate font-mono text-[10px] text-muted-foreground"
            title={describeNode(root)}
          >
            {describeNode(root)}
          </p>

          {/* 7 — live preview chip */}
          <div className="rounded-md border border-border/50 bg-muted/20 p-2">
            <div className="flex flex-wrap items-center gap-2">
              <span className="text-[10px] font-semibold tracking-widest text-muted-foreground">
                CANLI ÖNİZLEME
              </span>
              {previewLoading && !preview && (
                <span className="text-[10px] text-muted-foreground">değerleniyor…</span>
              )}
              {!previewLoading && !preview && !previewError && (
                <span className="text-[10px] text-muted-foreground">bekleniyor…</span>
              )}
              {previewError && !preview && (
                <span className="text-[10px] text-muted-foreground" title={previewError}>
                  canlı önizleme yok
                </span>
              )}
              {preview && (
                <Badge
                  variant="outline"
                  className={cn(
                    "text-[9px] font-semibold tracking-wide",
                    preview.met
                      ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400"
                      : "border-rose-500/40 bg-rose-500/10 text-rose-400",
                  )}
                >
                  {preview.met ? "KOŞUL SAĞLANIYOR" : "BEKLENİYOR"}
                </Badge>
              )}
            </div>
            {preview?.leaves && preview.leaves.length > 0 && (
              <ul className="mt-1.5 space-y-1">
                {preview.leaves.map((leaf, i) => (
                  <li key={i} className="flex flex-wrap items-center gap-x-2 gap-y-0.5 text-[10px]">
                    <span className="font-mono text-slate-300">{leaf.desc}</span>
                    <span className="font-mono text-muted-foreground">
                      {fmtMetric(leaf.left_value)} → {fmtMetric(leaf.right_value)}
                    </span>
                    {leaf.warmup ? (
                      <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1 text-[9px] font-semibold text-amber-400">
                        ısınıyor
                      </span>
                    ) : leaf.met ? (
                      <span className="rounded border border-emerald-500/40 bg-emerald-500/10 px-1 text-[9px] font-semibold text-emerald-400">
                        ✓ sağlandı
                      </span>
                    ) : (
                      <span className="rounded border border-rose-500/40 bg-rose-500/10 px-1 text-[9px] font-semibold text-rose-400">
                        ✗ değil
                      </span>
                    )}
                  </li>
                ))}
              </ul>
            )}
          </div>
        </>
      )}
    </div>
  );
}
