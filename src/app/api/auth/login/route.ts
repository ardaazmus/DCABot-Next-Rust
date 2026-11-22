import { NextRequest, NextResponse } from "next/server";
import {
  clearLoginFailures,
  createSessionToken,
  isSessionSecretConfigured,
  isUiPasswordConfigured,
  loginRateLimitDelay,
  recordLoginFailure,
  verifyPassword,
  SESSION_COOKIE_NAME,
} from "@/lib/auth-session";

export async function POST(req: NextRequest) {
  const blockedFor = loginRateLimitDelay();
  if (blockedFor > 0) {
    return NextResponse.json(
      { ok: false, error: "Çok fazla başarısız giriş denemesi; daha sonra tekrar deneyin" },
      { status: 429, headers: { "Retry-After": String(Math.ceil(blockedFor / 1000)) } },
    );
  }

  if (!isUiPasswordConfigured() || !isSessionSecretConfigured()) {
    return NextResponse.json(
      { ok: false, error: "UI parolası veya ayrı oturum imza anahtarı yapılandırılmamış" },
      { status: 503 },
    );
  }

  let body: unknown;
  try {
    body = await req.json();
  } catch {
    const delay = recordLoginFailure();
    return NextResponse.json(
      { ok: false, error: "İstek ayrıştırılamadı" },
      delay > 0
        ? { status: 429, headers: { "Retry-After": String(Math.ceil(delay / 1000)) } }
        : { status: 400 },
    );
  }

  const password = typeof body === "object" && body !== null && "password" in body
    && typeof body.password === "string"
    ? body.password
    : "";

  if (!verifyPassword(password)) {
    const delay = recordLoginFailure();
    return NextResponse.json(
      { ok: false, error: "Geçersiz erişim parolası" },
      delay > 0
        ? { status: 429, headers: { "Retry-After": String(Math.ceil(delay / 1000)) } }
        : { status: 401 },
    );
  }

  clearLoginFailures();
  const token = createSessionToken();
  const res = NextResponse.json({ ok: true, message: "Oturum açıldı" });
  res.cookies.set(SESSION_COOKIE_NAME, token, {
    httpOnly: true,
    secure: process.env.NODE_ENV === "production",
    sameSite: "strict",
    path: "/",
    maxAge: 7 * 24 * 60 * 60,
  });
  return res;
}
