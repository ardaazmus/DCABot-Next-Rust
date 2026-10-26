/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */

/* ---- P4-2 YZ Yardımcı ---- */

export interface AiProvider {
  id: string;
  label: string;
  models: string[];
  needsKey: boolean;
  defaultBaseUrl?: string | null;
  note: string;
}

export interface AiCallConfig {
  provider?: string;
  model?: string;
  /** Yalnız istek gövdesinde yaşar — diske yazılmaz, yanıta konmaz. */
  apiKey?: string;
  baseUrl?: string;
}

export interface AiBullet {
  title: string;
  text: string;
}

export interface AiResult {
  ok: boolean;
  verdict: "OK" | "ABSTAIN" | "ERROR";
  provider: string;
  model: string;
  bullets?: AiBullet[];
  text?: string;
  note?: string;
  elapsedMs: number;
  sources?: { glossaryCodes?: string[]; faqIds?: string[]; newsOk?: boolean };
}

export interface NewsAiResult {
  ok: boolean;
  summaryTr: string | null;
  reason: string | null;
  cached: boolean;
}
/* ---- Faz 2.3 Bot Hafızası (ölçülen bağlamın LLM özeti, ABSTAIN sözleşmeli) ---- */

export interface BotMemory {
  id: number;
  verdict: "OK" | "ABSTAIN" | "ERROR";
  content: string | null;
  model: string;
  note: string | null;
  elapsedMs: number;
  createdAt: number;
  context: Record<string, unknown>;
}

export interface BotMemoryListResult {
  botId: string;
  memories: BotMemory[];
}

export interface BotMemoryGenerateResult {
  ok: boolean;
  memory: Omit<BotMemory, "context">;
}
