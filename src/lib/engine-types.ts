/**
 * Exact wire types for the dca-core Rust engine (port 3002 via gateway).
 * ALL financial numerics arrive as decimal strings or "n/d" fraction strings.
 * NEVER use parseFloat on them for money math — format via src/lib/format.ts.
 */

/* Faz B — barrel: gerçek tipler ./engine-types/ altındadır.
   18 tüketici import'u değişmeden çalışır. */

export * from "./engine-types/core";
export * from "./engine-types/risk-policy";
export * from "./engine-types/signals";
export * from "./engine-types/bots";
export * from "./engine-types/market";
export * from "./engine-types/analysis";
export * from "./engine-types/ai";
export * from "./engine-types/knowledge";
export * from "./engine-types/vault";
export * from "./engine-types/stream";
