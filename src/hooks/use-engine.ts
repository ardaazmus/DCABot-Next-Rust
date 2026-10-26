"use client";

/* Faz A — barrel: `use-engine.ts` artık yalnızca yeniden-export eder.
   Gerçek kod: ./engine-client (taşıma), ./engine-stream (poll+SSE),
   ./use-bots (bot domain), ./api-analysis (tek-seferlik API'ler).
   14 tüketici import'u değişmeden çalışır. */

export * from "./engine-client";
export * from "./engine-stream";
export * from "./use-bots";
export * from "./api-analysis";
