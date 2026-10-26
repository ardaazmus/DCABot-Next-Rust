/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */

export type BotKind =
  | "DCA_LONG"
  | "DCA_SHORT"
  | "FUTURES_DCA"
  | "SPOT_GRID"
  | "FUTURES_GRID"
  | "INFINITY_GRID"
  | "HEDGE_PROTECTIVE"
  | "HEDGE_DUAL";

export type BotStatus = "RUNNING" | "PAUSED" | "STOPPED" | "FROZEN";
export type Venue = "BYBIT" | "BINANCE" | "OKX" | "PAPER";
export type Mode = "PAPER" | "LIVE";
export type Interval = "1m" | "3m" | "5m" | "15m" | "30m" | "1h" | "2h" | "4h" | "1d";

/**
 * Bot start condition (entry trigger) wire format — mirrors the Rust
 * SCREAMING_SNAKE_CASE serde enum. Legacy "Instant" string accepted by engine.
 */
export type EntryTrigger =
  | "INSTANT"
  | { RSI_BELOW: number }
  | { PRICE_BELOW: string }
  | { PRICE_ABOVE: string }
  | { DROP_FROM_REF: { ref_price: string; drop_pct: string } }
  | { SCHEDULED: { start_ms: number } };

export type EntryTriggerMode = "INSTANT" | "PRICE_BELOW" | "PRICE_ABOVE" | "DROP_FROM_REF" | "RSI_BELOW" | "SCHEDULED" | "PINE";

/* ------------------------------------------------------------------ */
/* Risk Motoru — advisory guard policy (Task 16 frozen wire contract)  */
/* ------------------------------------------------------------------ */

/**
 * Two-class guard semantics: ENFORCE blocks the action outright,
 * PERMISSIVE_WITH_WARNING only raises an advisory warning and lets the
 * acknowledged user proceed (Zero Hard-Block philosophy).
 */
export type GuardAction = "ENFORCE" | "PERMISSIVE_WITH_WARNING";

/** Fill realism for limit wicks: REALISTIC requires 1-tick penetration, OPTIMISTIC counts tangent wicks as fills. */
export type FillModel = "REALISTIC" | "OPTIMISTIC";

/** Market-session calendar the bot trades on. */
export type RiskSessionType = "CRYPTO_24_7" | "US_EQUITY" | "COMMODITY_23_5";
