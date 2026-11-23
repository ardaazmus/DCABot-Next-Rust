import crypto from "node:crypto";
import { isIP } from "node:net";

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
  // Hash both inputs first so timingSafeEqual always receives fixed-size
  // buffers. Preserve exact password bytes; trimming only one side changes
  // the configured credential and creates surprising whitespace behavior.
  const inputDigest = crypto.createHash("sha256").update(inputBuf).digest();
  const expectedDigest = crypto.createHash("sha256").update(expBuf).digest();
  const digestMatches = crypto.timingSafeEqual(inputDigest, expectedDigest);
  return digestMatches && inputBuf.length === expBuf.length;
}

const LOGIN_WINDOW_MS = 60_000;
const LOGIN_MAX_FAILURES = 5;
const LOGIN_MAX_BACKOFF_MS = 15 * 60_000;
const LOGIN_STATE_RETENTION_MS = LOGIN_MAX_BACKOFF_MS + LOGIN_WINDOW_MS;
const MAX_LOGIN_CLIENTS = 4096;
const OVERFLOW_LOGIN_CLIENT = "__capacity__";

type LoginAttemptState = {
  windowStartedAt: number;
  failures: number;
  blockedUntil: number;
  backoffMs: number;
  lastSeenAt: number;
};

const loginAttempts = new Map<string, LoginAttemptState>();

/**
 * Return a rate-limit identity only from an explicitly configured trusted
 * ingress header. Forwarding headers are otherwise attacker-controlled.
 */
export function loginClientKey(headers: Headers): string {
  const headerName = process.env.DCA_TRUSTED_CLIENT_IP_HEADER?.trim().toLowerCase();
  if (!headerName || !/^[a-z0-9-]{1,128}$/.test(headerName)) return "single-owner";

  const raw = headers.get(headerName)?.trim();
  if (!raw || raw.length > 64 || raw.includes(",")) return "single-owner";
  const version = isIP(raw);
  if (version === 0) return "single-owner";

  if (version === 4) return `ip:${raw}`;
  try {
    // WHATWG URL canonicalizes equivalent IPv6 spellings into one bucket.
    const normalized = new URL(`http://[${raw}]/`).hostname.toLowerCase();
    return `ip:${normalized}`;
  } catch {
    return "single-owner";
  }
}

function getLoginAttemptState(clientKey: string, now: number): LoginAttemptState {
  let state = loginAttempts.get(clientKey);
  if (!state && loginAttempts.size >= MAX_LOGIN_CLIENTS - 1) {
    for (const [key, candidate] of loginAttempts) {
      if (
        now >= candidate.blockedUntil
        && now - candidate.lastSeenAt >= LOGIN_STATE_RETENTION_MS
      ) {
        loginAttempts.delete(key);
      }
    }
  }

  let bucketKey = clientKey;
  if (!state && loginAttempts.size >= MAX_LOGIN_CLIENTS) bucketKey = OVERFLOW_LOGIN_CLIENT;
  state = loginAttempts.get(bucketKey);
  if (!state) {
    state = {
      windowStartedAt: 0,
      failures: 0,
      blockedUntil: 0,
      backoffMs: 60_000,
      lastSeenAt: now,
    };
    loginAttempts.set(bucketKey, state);
  }
  state.lastSeenAt = now;
  return state;
}

/** Process-local per-client limiter. Returns the remaining delay. */
export function loginRateLimitDelay(now = Date.now(), clientKey = "single-owner"): number {
  const state = getLoginAttemptState(clientKey, now);
  if (now < state.blockedUntil) return state.blockedUntil - now;
  if (state.blockedUntil > 0) {
    state.blockedUntil = 0;
    state.windowStartedAt = now;
    state.failures = 0;
  }
  if (state.windowStartedAt > 0 && now - state.windowStartedAt >= LOGIN_WINDOW_MS) {
    state.windowStartedAt = now;
    state.failures = 0;
  }
  return 0;
}

export function recordLoginFailure(now = Date.now(), clientKey = "single-owner"): number {
  const blocked = loginRateLimitDelay(now, clientKey);
  if (blocked > 0) return blocked;
  const state = getLoginAttemptState(clientKey, now);
  if (state.windowStartedAt === 0) state.windowStartedAt = now;
  state.failures += 1;
  if (state.failures < LOGIN_MAX_FAILURES) return 0;

  state.blockedUntil = now + state.backoffMs;
  const delay = state.backoffMs;
  state.backoffMs = Math.min(state.backoffMs * 2, LOGIN_MAX_BACKOFF_MS);
  state.failures = 0;
  state.windowStartedAt = now;
  return delay;
}

export function clearLoginFailures(clientKey?: string): void {
  if (clientKey === undefined) {
    loginAttempts.clear();
    return;
  }
  // Do not clear the shared capacity bucket when an overflow client succeeds:
  // that would let any one client erase the throttling shared by other clients.
  loginAttempts.delete(clientKey);
}

export { SESSION_COOKIE_NAME };
