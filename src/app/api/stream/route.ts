/**
 * C-1 / SEC-01: SSE proxy — tarayıcı EventSource'u Authorization başlığı
 * koyamaz; motor /api/stream dahil tüm yönetim uçlarında Bearer zorunlu
 * tutar. Bu route, akışı sunucu tarafında açıp aynı origin üzerinden
 * (rewrite'a inmeden) tarayıcıya aktarır. App Router route'ları
 * rewrite'lardan önce eşleşir; diğer /api/* uçları engineFetch'in
 * Bearer'ıyla rewrite'tan geçer.
 *
 * SSRF sert kenar: hedef URL tamamen sunucu tarafında, sabit ana makine +
 * doğrulanmış sayısal porttan kurulur; istekten gelen hiçbir veri URL'ye
 * girmez, protokol https/http dışına çıkamaz, ana makine beyaz listesinden
 * geçmeyen hedef bağlanmaz.
 */

export const dynamic = "force-dynamic";

/** Yalnız bu ana makinelere proxy yapılır (loopback motoru). */
const ALLOWED_HOSTS = new Set(["127.0.0.1", "localhost"]);
const ENGINE_HOST = "127.0.0.1";

/** Env port'u yalnız 2-5 haneli sayıysa kabul edilir; aksi halde 3002. */
function enginePort(): string {
  const raw = process.env.NEXT_PUBLIC_ENGINE_PORT || "";
  return /^\d{2,5}$/.test(raw) && Number(raw) > 0 && Number(raw) < 65536 ? raw : "3002";
}

export async function GET() {
  // Sabit uç nokta: istekten gelen hiçbir veri URL'ye girmez.
  let target: URL;
  try {
    target = new URL(`http://${ENGINE_HOST}:${enginePort()}/api/stream`);
  } catch {
    return new Response("bad engine endpoint", { status: 500 });
  }
  if (target.protocol !== "http:" || !ALLOWED_HOSTS.has(target.hostname)) {
    return new Response("engine endpoint not allowed", { status: 403 });
  }
  // Faz 22 (N-13): NEXT_PUBLIC_ fallback silindi — anahtarın tek kaynağı
  // sunucu env'idir (ENGINE_API_KEY); o önekli değişken istemci paketine
  // girme riski taşır ve ismin durması P0-9'un geri gelme tuzağıydı.
  const key = process.env.DCA_API_KEY || process.env.ENGINE_API_KEY || "";
  let upstream: Response;
  try {
    upstream = await fetch(target, {
      cache: "no-store",
      redirect: "error",
      headers: {
        Accept: "text/event-stream",
        ...(key ? { Authorization: `Bearer ${key}` } : {}),
      },
    });
  } catch {
    return new Response("engine unreachable", { status: 502 });
  }
  if (!upstream.ok || !upstream.body) {
    return new Response(`engine stream error: ${upstream.status}`, { status: upstream.status });
  }
  return new Response(upstream.body, {
    headers: {
      "Content-Type": "text/event-stream; charset=utf-8",
      "Cache-Control": "no-cache, no-transform",
      Connection: "keep-alive",
    },
  });
}
