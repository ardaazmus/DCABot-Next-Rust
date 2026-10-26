/* Faz B — engine-types bölünmesi. Tüketiciler değişmedi (`@/lib/engine-types` barrel). */

/* ---- P4-4 yardım sözlükleri ---- */

export interface GlossaryEntry {
  code: string;
  exchange: string;
  title: string;
  severity: string;
  plainCause: string;
  botAction: string;
  operatorSteps?: string[];
}

export interface FaqEntry {
  id: string;
  category: string;
  question: string;
  answer: string;
  keywords: string[];
}
