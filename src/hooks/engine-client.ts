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

/**
 * SSE özel durumu: EventSource özel başlık koyamaz → /api/stream route
 * handler'ı sunucu tarafında Bearer'ı ekler (istekten gelen hiçbir veri
 * hedef URL'ye girmez).
 */
export function engineUrl(path: string): string {
  void path;
  return "/api/stream";
}

/**
 * Faz 21 (P0-9): motor anahtarı TARAYICIYA GİRMEZ. Tüm çağrılar aynı origin
 * altındaki sunucu proxy'sinden (/api/engine/*) geçer; Bearer yalnız
 * sunucuda env'den eklenir. Eski `NEXT_PUBLIC_ENGINE_API_KEY` gömülüsü
 * kaldırıldı — dışa açılan her ziyaretçi motoru tam yetkiyle kullanamalı.
 */
export async function engineFetch<T>(path: string, init?: RequestInit): Promise<T> {
  const clean = path.startsWith("/") ? path : `/${path}`;
  let res: Response;
  try {
    res = await fetch(`/api/engine${clean}`, {
      cache: "no-store",
      ...init,
      headers: {
        "Content-Type": "application/json",
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
