import { NextRequest, NextResponse } from "next/server";
import crypto from "node:crypto";
import { isAuthRequired, verifySessionToken, SESSION_COOKIE_NAME } from "@/lib/auth-session";

/**
 * Faz 21 (P0-9): motor proxy'si — proxy katmanı.
 *
 * Tarayıcı artık motora ASLA doğrudan konuşmaz ve API anahtarı istemci
 * paketine GİRMEZ. Tüm motor çağrıları `/api/engine/*` altına alınır;
 * proxy bunu yakalayıp (rewrites'tan ÖNCE — catch-all route'lar
 * rewrite'ı geçemediği için route-handler yaklaşımı işlemedi) Bearer'ı
 * YALNIZ burada, sunucu tarafında ekleyerek motora taşır.
 *
 * Faz 29: Next 16.4.3+ `middleware.ts` dosya konvansiyonu dev derlemede
 * çalışmayı bıraktı (deprecated → proxy). Dosya `middleware.ts` →
 * `proxy.ts`, fonksiyon `middleware()` → `proxy()` olarak geçirildi;
 * davranış birebir korundu.
 *
 * SSRF sert kenar: hedef tamamen sabit ana makine + doğrulanmış sayısal
 * porttan kurulur; istekten gelen hiçbir veri hedef URL'nin host/port/şema
 * kısmına girmez (yalnız path/query aynen taşınır), protokol http loopback
 * dışına çıkamaz, ana makine beyaz listesi dışına bağlanılmaz.
 */

const ALLOWED_HOSTS = new Set(["127.0.0.1", "localhost"]);
const ENGINE_HOST = "127.0.0.1";

function enginePort(): string {
  const raw = process.env.NEXT_PUBLIC_ENGINE_PORT || "";
  return /^\d{2,5}$/.test(raw) && Number(raw) > 0 && Number(raw) < 65536 ? raw : "3002";
}

function engineKey(): string {
  // Faz 22 (N-13): NEXT_PUBLIC_ fallback SİLİNDİ — o önekli her değişken,
  // herhangi bir istemci dosyası okuduğu anda pakete girer; anahtarın tek
  // kaynağı sunucu env'idir. UI süreci için ayrılmış ENGINE_API_KEY önce gelir:
  // Next.js .env'den eski DCA_API_KEY değerini yüklemiş olsa bile, PAPER motor
  // anahtarının üstüne yazıp bot listesini 401'le boş gösteremez.
  return process.env.ENGINE_API_KEY || process.env.DCA_API_KEY || "";
}

/** N-12: proxy'ye CSRF/origin kapısı — same-origin doğrulaması.
 *  Proxy Bearer'ı sunucuda eklediği için Next uygulamasına erişebilen herkes
 *  (LAN cihazı, zararlı sayfanın fetch'i) motora tam yetkiyle yazabilirdi.
 *  Model: tarayıcı istekleri YA Sec-Fetch-Site (modern) YA Origin (POST'ta
 *  her zaman) taşır — ikisi de same-origin olmalı. İki başlık da yoksa istek
 *  tarayıcı-dışıdır (curl, sunucu-içi otomasyon, tunnel arkası) → izinli;
 *  zararlı-sayfa senaryosu bu yolla oluşamaz çünkü tarayıcı başlıkları
 *  sözleşmeyi bırakmaz. */
function originAllowed(req: NextRequest): boolean {
  const site = req.headers.get("sec-fetch-site");
  if (site) return site === "same-origin" || site === "none";
  const origin = req.headers.get("origin");
  if (!origin) return true; // tarayıcı-dışı istemci — başlık sözleşmesi yok
  try {
    const host = req.headers.get("host") ?? "";
    return new URL(origin).host === host;
  } catch {
    return false;
  }
}

export function proxy(req: NextRequest) {
  const { pathname, search } = req.nextUrl;
  const nonce = crypto.randomBytes(16).toString("base64");
  const isDevelopment = process.env.NODE_ENV !== "production";
  const csp = [
    "default-src 'self'",
    `script-src 'self' 'nonce-${nonce}' 'strict-dynamic'${isDevelopment ? " 'unsafe-eval'" : ""}`,
    // React charts and sliders set element styles dynamically; keep inline
    // styles enabled while executable scripts require a per-request nonce.
    "style-src 'self' 'unsafe-inline'",
    "font-src 'self' data:",
    "img-src 'self' data: blob:",
    `connect-src 'self'${isDevelopment ? " ws: wss:" : ""}`,
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'self'",
    "frame-ancestors 'none'",
  ].join("; ");
  const headers = new Headers(req.headers);
  headers.set("x-nonce", nonce);
  headers.set("Content-Security-Policy", csp);

  if (!pathname.startsWith("/api/engine/")) {
    const response = NextResponse.next({ request: { headers } });
    response.headers.set("Content-Security-Policy", csp);
    return response;
  }
  // Faz 22 (N-12): CSRF/origin kapısı — cross-origin yazma denemeleri 403.
  if (!originAllowed(req)) {
    return new NextResponse("cross-origin engine write rejected", { status: 403 });
  }
  // Faz 0.3 (B-3): UI Auth Guard — DCA_UI_PASSWORD veya DCA_REQUIRE_AUTH aktifse oturumsuz istekler 401.
  if (isAuthRequired() && pathname !== "/api/engine/api/health") {
    const token = req.cookies.get(SESSION_COOKIE_NAME)?.value;
    if (!verifySessionToken(token)) {
      return new NextResponse(JSON.stringify({ error: "UNAUTHORIZED: Oturum açmanız gerekmektedir" }), {
        status: 401,
        headers: { "Content-Type": "application/json" },
      });
    }
  }
  // "/api/engine/api/health" → hedef "/api/health" (önek soyulur).
  const rest = pathname.slice("/api/engine".length) || "/";
  // A01 / SSRF & Path Traversal Guard: Sadece meşru /api/* rotalarına izin verilir.
  if (!rest.startsWith("/api/") && rest !== "/api") {
    return new NextResponse("invalid engine path", { status: 400 });
  }
  if (rest.includes("..") || rest.includes("\0")) {
    return new NextResponse("path traversal attempt blocked", { status: 400 });
  }
  let target: URL;
  try {
    target = new URL(`http://${ENGINE_HOST}:${enginePort()}${rest}${search}`);
  } catch {
    return new NextResponse("bad engine endpoint", { status: 500 });
  }
  if (target.protocol !== "http:" || !ALLOWED_HOSTS.has(target.hostname)) {
    return new NextResponse("engine endpoint not allowed", { status: 403 });
  }
  headers.delete("host");
  headers.delete("content-length");
  const key = engineKey();
  if (key) headers.set("Authorization", `Bearer ${key}`);
  const response = NextResponse.rewrite(target, { request: { headers } });
  response.headers.set("Content-Security-Policy", csp);
  return response;
}

export const config = {
  matcher: [
    "/((?!api|_next/static|_next/image|favicon.ico).*)",
    "/api/engine/:path*",
  ],
};
