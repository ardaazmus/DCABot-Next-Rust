/* Faz A — motor HTTP istemcisi (engineFetch + engineJson + EngineError).
   Saf modül: hook yok, yalnızca taşıma. Tüketiciler değişmedi
   (`@/hooks/use-engine` barrel üzerinden). */

export const ENGINE_PORT =
  (typeof process !== "undefined" && process.env.NEXT_PUBLIC_ENGINE_PORT) || "3002";

/* ------------------------------------------------------------------ */
/* engineFetch — gateway routed fetch, never absolute URLs            */
/* ------------------------------------------------------------------ */

export class EngineError extends Error {
  status: number;
  body: unknown;
  constructor(message: string, status: number, body: unknown) {
    super(message);
    this.name = "EngineError";
    this.status = status;
    this.body = body;
  }
}

export function engineUrl(path: string): string {
  const sep = path.includes("?") ? "&" : "?";
  return `${path}${sep}XTransformPort=${ENGINE_PORT}`;
}

/**
 * C-1: motor SEC-01'den beri tüm yönetim uçlarında Bearer zorunlu tutuyor.
 * Anahtar build/dev-time env'den gelir (start-local.ps1 üretir, tarayıcıya
 * NEXT_PUBLIC_ önekiyle gömülür — yalnız localhost geliştirme kapsamında).
 * SSE özel durumu: EventSource özel başlık koyamaz → /api/stream route
 * handler'ı sunucu tarafında aynı başlığı ekler.
 */
const ENGINE_API_KEY = process.env.NEXT_PUBLIC_ENGINE_API_KEY || "";

export function engineAuthHeaders(): Record<string, string> {
  return ENGINE_API_KEY ? { Authorization: `Bearer ${ENGINE_API_KEY}` } : {};
}

export async function engineFetch<T>(path: string, init?: RequestInit): Promise<T> {
  let res: Response;
  try {
    res = await fetch(engineUrl(path), {
      cache: "no-store",
      ...init,
      headers: {
        "Content-Type": "application/json",
        ...engineAuthHeaders(),
        ...(init?.headers ?? {}),
      },
    });
  } catch {
    throw new EngineError("Çekirdeğe ulaşılamıyor (ağ hatası)", 0, null);
  }
  const text = await res.text();
  let body: unknown = null;
  try {
    body = text ? JSON.parse(text) : null;
  } catch {
    body = { error: text.slice(0, 400) };
  }
  if (!res.ok) {
    const msg =
      body && typeof body === "object" && "error" in body
        ? String((body as { error: unknown }).error)
        : `HTTP ${res.status}`;
    throw new EngineError(msg, res.status, body);
  }
  return body as T;
}

export const engineJson = (method: string, body: unknown): RequestInit => ({
  method,
  body: JSON.stringify(body),
});
