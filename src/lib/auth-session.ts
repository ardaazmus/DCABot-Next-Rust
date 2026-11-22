import crypto from "node:crypto";

const SESSION_COOKIE_NAME = "dca_session";

function getSecret(): string | null {
  const secret = process.env.DCA_SESSION_SECRET?.trim();
  return secret && Buffer.byteLength(secret, "utf8") >= 32 ? secret : null;
}

export function isSessionSecretConfigured(): boolean {
  return getSecret() !== null;
}

export function isAuthRequired(): boolean {
  const uiPass = process.env.DCA_UI_PASSWORD?.trim();
  const reqAuth = process.env.DCA_REQUIRE_AUTH === "1";
  return process.env.NODE_ENV === "production" || Boolean(uiPass || reqAuth);
}

export function isUiPasswordConfigured(): boolean {
  return Boolean(process.env.DCA_UI_PASSWORD?.trim());
}

export function createSessionToken(): string {
  const secret = getSecret();
  if (!secret) {
    throw new Error("Session signing secret is not configured or is too short");
  }
  const payload = JSON.stringify({
    authenticated: true,
    issuedAt: Date.now(),
    expiresAt: Date.now() + 7 * 24 * 60 * 60 * 1000,
  });
  const b64Payload = Buffer.from(payload).toString("base64url");
  const signature = crypto
    .createHmac("sha256", secret)
    .update(b64Payload)
    .digest("base64url");
  return `${b64Payload}.${signature}`;
}

export function verifySessionToken(token: string | undefined | null): boolean {
  if (!token) return false;
  const secret = getSecret();
  if (!secret) return false;
  const parts = token.split(".");
  if (parts.length !== 2) return false;
  const [b64Payload, signature] = parts;

  const expectedSignature = crypto
    .createHmac("sha256", secret)
    .update(b64Payload)
    .digest("base64url");

  // Constant-time buffer equality check
  const sigBuf = Buffer.from(signature);
  const expBuf = Buffer.from(expectedSignature);
  if (sigBuf.length !== expBuf.length || !crypto.timingSafeEqual(sigBuf, expBuf)) {
    return false;
  }

  try {
    const payload = JSON.parse(Buffer.from(b64Payload, "base64url").toString("utf-8"));
    if (
      typeof payload.expiresAt !== "number"
      || !Number.isFinite(payload.expiresAt)
      || payload.expiresAt <= Date.now()
      || typeof payload.issuedAt !== "number"
      || !Number.isFinite(payload.issuedAt)
      || payload.issuedAt > Date.now()
      || payload.authenticated !== true
    ) {
      return false;
    }
    return true;
  } catch {
    return false;
  }
}

export function verifyPassword(password: string): boolean {
  const expected = process.env.DCA_UI_PASSWORD || "";
  if (!expected) {
    return false;
  }
  const inputBuf = Buffer.from(password);
  const expBuf = Buffer.from(expected);
  if (inputBuf.length !== expBuf.length) {
    return false;
  }
  return crypto.timingSafeEqual(inputBuf, expBuf);
}

const LOGIN_WINDOW_MS = 60_000;
const LOGIN_MAX_FAILURES = 5;
const LOGIN_MAX_BACKOFF_MS = 15 * 60_000;
let loginWindowStartedAt = 0;
let loginFailures = 0;
let loginBlockedUntil = 0;
let loginBackoffMs = 60_000;

/** Process-wide limiter for this single-owner local UI. It deliberately
 * ignores spoofable forwarding headers; a multi-instance deployment should
 * place a shared limiter at its trusted edge. Returns the remaining delay. */
export function loginRateLimitDelay(now = Date.now()): number {
  if (now < loginBlockedUntil) return loginBlockedUntil - now;
  if (loginBlockedUntil > 0) {
    loginBlockedUntil = 0;
    loginWindowStartedAt = now;
    loginFailures = 0;
  }
  if (loginWindowStartedAt > 0 && now - loginWindowStartedAt >= LOGIN_WINDOW_MS) {
    loginWindowStartedAt = now;
    loginFailures = 0;
  }
  return 0;
}

export function recordLoginFailure(now = Date.now()): number {
  const blocked = loginRateLimitDelay(now);
  if (blocked > 0) return blocked;
  if (loginWindowStartedAt === 0) loginWindowStartedAt = now;
  loginFailures += 1;
  if (loginFailures < LOGIN_MAX_FAILURES) return 0;

  loginBlockedUntil = now + loginBackoffMs;
  const delay = loginBackoffMs;
  loginBackoffMs = Math.min(loginBackoffMs * 2, LOGIN_MAX_BACKOFF_MS);
  loginFailures = 0;
  loginWindowStartedAt = now;
  return delay;
}

export function clearLoginFailures(): void {
  loginWindowStartedAt = 0;
  loginFailures = 0;
  loginBlockedUntil = 0;
  loginBackoffMs = 60_000;
}

export { SESSION_COOKIE_NAME };
