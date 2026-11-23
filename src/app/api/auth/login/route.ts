import { NextRequest, NextResponse } from "next/server";
import {
  clearLoginFailures,
  createSessionToken,
  isSessionSecretConfigured,
  isUiPasswordConfigured,
  loginClientKey,
  loginRateLimitDelay,
  recordLoginFailure,
  verifyPassword,
  SESSION_COOKIE_NAME,
} from "@/lib/auth-session";

const MAX_LOGIN_BODY_BYTES = 4096;
const MAX_PASSWORD_BYTES = 1024;

async function readBoundedJson(req: NextRequest): Promise<{ ok: true; value: unknown } | { ok: false }> {
  const reader = req.body?.getReader();
  if (!reader) return { ok: false };

  const chunks: Uint8Array[] = [];
  let total = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      total += value.byteLength;
      if (total > MAX_LOGIN_BODY_BYTES) {
        await reader.cancel();
        return { ok: false };
      }
      chunks.push(value);
    }
    const body = new Uint8Array(total);
    let offset = 0;
    for (const chunk of chunks) {
      body.set(chunk, offset);
      offset += chunk.byteLength;
    }
    const text = new TextDecoder("utf-8", { fatal: true }).decode(body);
    return { ok: true, value: JSON.parse(text) as unknown };
  } catch {
    return { ok: false };
  } finally {
    reader.releaseLock();
  }
}

export async function POST(req: NextRequest) {
  // Require a non-simple browser request. Cross-origin HTML forms cannot
  // submit this media type, so they cannot consume the client's lockout.
  const contentType = req.headers.get("content-type")?.split(";", 1)[0].trim().toLowerCase();
  if (contentType !== "application/json") {
    return NextResponse.json({ ok: false, error: "application/json gerekli" }, { status: 415 });
  }

  const limiterKey = loginClientKey(req.headers);
  const blockedFor = loginRateLimitDelay(Date.now(), limiterKey);
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

  const parsed = await readBoundedJson(req);
  if (!parsed.ok) {
    return NextResponse.json({ ok: false, error: "İstek ayrıştırılamadı veya çok büyük" }, { status: 400 });
  }
  const body = parsed.value;

  const password = typeof body === "object" && body !== null && "password" in body
    && typeof body.password === "string"
    ? body.password
    : "";

  if (Buffer.byteLength(password, "utf8") > MAX_PASSWORD_BYTES) {
    return NextResponse.json({ ok: false, error: "Parola çok uzun" }, { status: 400 });
  }

  if (!verifyPassword(password)) {
    const delay = recordLoginFailure(Date.now(), limiterKey);
    return NextResponse.json(
      { ok: false, error: "Geçersiz erişim parolası" },
      delay > 0
        ? { status: 429, headers: { "Retry-After": String(Math.ceil(delay / 1000)) } }
        : { status: 401 },
    );
  }

  clearLoginFailures(limiterKey);
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
