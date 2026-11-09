import crypto from "node:crypto";

const SESSION_COOKIE_NAME = "dca_session";

function getSecret(): string {
  return process.env.DCA_VAULT_KEY || process.env.DCA_API_KEY || "dcabot-default-session-secret-change-in-prod";
}

export function isAuthRequired(): boolean {
  const uiPass = process.env.DCA_UI_PASSWORD?.trim();
  const reqAuth = process.env.DCA_REQUIRE_AUTH === "1";
  return Boolean(uiPass || reqAuth);
}

export function createSessionToken(): string {
  const payload = JSON.stringify({
    authenticated: true,
    issuedAt: Date.now(),
    expiresAt: Date.now() + 7 * 24 * 60 * 60 * 1000,
  });
  const b64Payload = Buffer.from(payload).toString("base64url");
  const signature = crypto
    .createHmac("sha256", getSecret())
    .update(b64Payload)
    .digest("base64url");
  return `${b64Payload}.${signature}`;
}

export function verifySessionToken(token: string | undefined | null): boolean {
  if (!token) return false;
  const parts = token.split(".");
  if (parts.length !== 2) return false;
  const [b64Payload, signature] = parts;

  const expectedSignature = crypto
    .createHmac("sha256", getSecret())
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
    if (typeof payload.expiresAt === "number" && Date.now() > payload.expiresAt) {
      return false;
    }
    return Boolean(payload.authenticated);
  } catch {
    return false;
  }
}

export function verifyPassword(password: string): boolean {
  const expected = (process.env.DCA_UI_PASSWORD || process.env.DCA_API_KEY || process.env.ENGINE_API_KEY || "").trim();
  if (!expected) {
    // If no password configured, deny access unless configured
    return false;
  }
  const inputBuf = Buffer.from(password.trim());
  const expBuf = Buffer.from(expected);
  if (inputBuf.length !== expBuf.length) {
    return false;
  }
  return crypto.timingSafeEqual(inputBuf, expBuf);
}

export { SESSION_COOKIE_NAME };
