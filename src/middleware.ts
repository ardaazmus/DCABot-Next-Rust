import { NextRequest, NextResponse } from "next/server";

/**
 * Faz 21 (P0-9): motor proxy'si — middleware katmanı.
 *
 * Tarayıcı artık motora ASLA doğrudan konuşmaz ve API anahtarı istemci
 * paketine GİRMEZ. Tüm motor çağrıları `/api/engine/*` altına alınır;
 * middleware bunu yakalayıp (rewrites'tan ÖNCE — catch-all route'lar
 * rewrite'ı geçemediği için route-handler yaklaşımı işlemedi) Bearer'ı
 * YALNIZ burada, sunucu tarafında ekleyerek motora taşır.
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
  return process.env.ENGINE_API_KEY || process.env.NEXT_PUBLIC_ENGINE_API_KEY || "";
}

export function middleware(req: NextRequest) {
  const { pathname, search } = req.nextUrl;
  if (!pathname.startsWith("/api/engine/")) {
    return NextResponse.next();
  }
  // "/api/engine/api/health" → hedef "/api/health" (önek soyulur).
  const rest = pathname.slice("/api/engine".length) || "/";
  let target: URL;
  try {
    target = new URL(`http://${ENGINE_HOST}:${enginePort()}${rest}${search}`);
  } catch {
    return new NextResponse("bad engine endpoint", { status: 500 });
  }
  if (target.protocol !== "http:" || !ALLOWED_HOSTS.has(target.hostname)) {
    return new NextResponse("engine endpoint not allowed", { status: 403 });
  }
  const headers = new Headers(req.headers);
  headers.delete("host");
  headers.delete("content-length");
  const key = engineKey();
  if (key) headers.set("Authorization", `Bearer ${key}`);
  return NextResponse.rewrite(target, { request: { headers } });
}

export const config = {
  matcher: ["/api/engine/:path*"],
};
