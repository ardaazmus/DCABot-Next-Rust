/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */
import type { FillModel, GuardAction, RiskSessionType } from "./core";

export interface UserRiskPolicy {
  session_guard: GuardAction;
  opening_salvo_buffer: GuardAction;
  gap_consolidation: GuardAction;
  macro_blackout: GuardAction;
  regime_filter: GuardAction;
  m20_sl_ladder_guard: GuardAction;
  grid_fee_breaker: GuardAction;
  /** 1..max_safety_orders — how many safety orders rest on the book simultaneously. */
  active_safety_orders: number;
  /** Daily drawdown circuit breaker percent; "0" = disabled, default "2". */
  daily_dd_breaker_pct: string;
  fill_model: FillModel;
  session_type: RiskSessionType;
}

export const DEFAULT_RISK_POLICY: UserRiskPolicy = {
  session_guard: "ENFORCE",
  opening_salvo_buffer: "ENFORCE",
  gap_consolidation: "ENFORCE",
  macro_blackout: "ENFORCE",
  regime_filter: "PERMISSIVE_WITH_WARNING",
  m20_sl_ladder_guard: "PERMISSIVE_WITH_WARNING",
  grid_fee_breaker: "PERMISSIVE_WITH_WARNING",
  active_safety_orders: 1,
  daily_dd_breaker_pct: "2",
  fill_model: "REALISTIC",
  session_type: "CRYPTO_24_7",
};

/** Advisory warning row — wizard live panel and engine 422/201 bodies share this shape. */
export interface AdvisoryWarning {
  code: string;
  title: string;
  message: string;
  severity: "WARN" | "INFO";
}
