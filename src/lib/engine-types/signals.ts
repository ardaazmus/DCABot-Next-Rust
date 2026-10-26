/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */

/* ------------------------------------------------------------------ */
/* 3Commas-style Signal Engine (dca_signals crate wire types)          */
/* ------------------------------------------------------------------ */

export type Timeframe = "M1" | "M5" | "M15" | "H1" | "H4" | "D1";

export type IndicatorKey =
  | "Rsi"
  | "Ema"
  | "Sma"
  | "Macd"
  | "Bb"
  | "Stoch"
  | "Atr"
  | "Adx"
  | "Vol"
  | "Price";

export type IndicatorOutput =
  | "default"
  | "macd"
  | "signal"
  | "hist"
  | "lower"
  | "mid"
  | "upper"
  | "bandwidth"
  | "k"
  | "d"
  | "atr_abs"
  | "atr_pct"
  | "ratio"
  | "close"
  | "change_pct";

/**
 * Indicator definition leaf — mirrors Rust dca_signals::IndicatorSpec.
 * Numeric fields arrive as numbers (signal layer is float metrics, not accounting).
 */
export interface IndicatorSpec {
  key: IndicatorKey;
  timeframe: Timeframe;
  period?: number;
  fast?: number;
  slow?: number;
  signal?: number;
  std_dev?: number;
  d_period?: number;
  output?: IndicatorOutput;
}

export type CompareOp = "Lt" | "Lte" | "Gt" | "Gte" | "CrossUp" | "CrossDown";

/** {"kind":"number","value":30} | {"kind":"indicator","value":{...IndicatorSpec}} */
export type SignalValue =
  | { kind: "number"; value: number }
  | { kind: "indicator"; value: IndicatorSpec };

export interface SignalCondition {
  left: IndicatorSpec;
  op: CompareOp;
  right: SignalValue;
}

export interface SignalGroup {
  op: "And" | "Or";
  children: SignalNode[];
}

/** Untagged union on the wire: condition has left/op/right, group has op/children. */
export type SignalNode = SignalCondition | SignalGroup;

export interface SignalConfig {
  root: SignalNode;
}

export function isSignalGroup(n: SignalNode): n is SignalGroup {
  return "children" in n;
}

/** Per-leaf evaluation report from POST /api/signals/preview. */
export interface SignalLeafReport {
  desc: string;
  tf: string;
  left_value: number | null;
  right_value: number | null;
  met: boolean;
  warmup: boolean;
}

export interface SignalPreviewResponse {
  met: boolean;
  asof: number;
  leaves: SignalLeafReport[];
  error?: string;
}
