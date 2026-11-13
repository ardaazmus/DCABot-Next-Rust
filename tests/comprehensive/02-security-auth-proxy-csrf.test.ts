import fc from "fast-check";
import { NextRequest } from "next/server";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GET as authCheckGET } from "@/app/api/auth/check/route";
import { POST as authLoginPOST } from "@/app/api/auth/login/route";
import { POST as authLogoutPOST } from "@/app/api/auth/logout/route";
import { GET as healthGET } from "@/app/api/health/route";
import { GET as streamGET } from "@/app/api/stream/route";
import {
  createSessionToken,
  isAuthRequired,
  SESSION_COOKIE_NAME,
  verifyPassword,
  verifySessionToken,
} from "@/lib/auth-session";
import { proxy } from "@/proxy";

describe("Pillar 2 — Zero-Trust Security, Session Crypto, Proxy CSRF/SSRF & Route Handlers", () => {
  const originalEnv = { ...process.env };
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    process.env = { ...originalEnv };
    delete process.env.DCA_UI_PASSWORD;
    delete process.env.DCA_REQUIRE_AUTH;
    delete process.env.DCA_API_KEY;
    delete process.env.ENGINE_API_KEY;
    delete process.env.DCA_VAULT_KEY;
    delete process.env.NEXT_PUBLIC_ENGINE_PORT;
  });

  afterEach(() => {
    process.env = { ...originalEnv };
    globalThis.fetch = originalFetch;
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  describe("Session Token Cryptography & Tamper Resistance (auth-session.ts)", () => {
    it("signs valid tokens and rejects payload/signature bit-flips, forged claims, and secret rotation", () => {
      process.env.DCA_VAULT_KEY = "test-vault-master-secret-32bytes!";
      const token = createSessionToken();
      expect(verifySessionToken(token)).toBe(true);

      const [payloadB64, sigB64] = token.split(".");
      // 1. Tamper signature
      const flippedSig = (sigB64[0] === "A" ? "B" : "A") + sigB64.slice(1);
      expect(verifySessionToken(`${payloadB64}.${flippedSig}`)).toBe(false);

      // 2. Tamper payload (extend expiry to year 2099 without key)
      const forgedPayload = Buffer.from(
        JSON.stringify({ authenticated: true, issuedAt: Date.now(), expiresAt: 4_102_444_800_000 }),
      ).toString("base64url");
      expect(verifySessionToken(`${forgedPayload}.${sigB64}`)).toBe(false);

      // 3. Rotate master secret -> old token immediately invalidated
      process.env.DCA_VAULT_KEY = "rotated-vault-master-secret-99999";
      expect(verifySessionToken(token)).toBe(false);
    });

    it("enforces strict 7-day TTL expiration boundary", () => {
      vi.useFakeTimers();
      vi.setSystemTime(new Date("2026-11-13T00:00:00Z"));
      process.env.DCA_VAULT_KEY = "ttl-test-secret";

      const token = createSessionToken();
      expect(verifySessionToken(token)).toBe(true);

      // Advance 7 days minus 1 second -> still valid
      vi.advanceTimersByTime(7 * 24 * 60 * 60 * 1000 - 1000);
      expect(verifySessionToken(token)).toBe(true);

      // Advance 2 more seconds -> expired
      vi.advanceTimersByTime(2000);
      expect(verifySessionToken(token)).toBe(false);
    });

    it("fails closed when no password is configured and handles UTF-8 / whitespace safely", () => {
      expect(isAuthRequired()).toBe(false);
      expect(verifyPassword("")).toBe(false);
      expect(verifyPassword("anything")).toBe(false);

      process.env.DCA_UI_PASSWORD = "  SüperGüvenli-Şifre!2026  ";
      expect(isAuthRequired()).toBe(true);
      expect(verifyPassword("SüperGüvenli-Şifre!2026")).toBe(true);
      expect(verifyPassword("  SüperGüvenli-Şifre!2026 ")).toBe(true);
      expect(verifyPassword("SüperGüvenli-Şifre!2025")).toBe(false);
      expect(verifyPassword("short")).toBe(false);
    });
  });

  describe("Auth API Route Handlers (/api/auth/login, /check, /logout)", () => {
    it("completes full login -> check -> logout lifecycle with strict HttpOnly + SameSite=strict cookies", async () => {
      process.env.DCA_UI_PASSWORD = "correct-horse-battery";
      process.env.DCA_VAULT_KEY = "session-signing-secret";

      // 1. Unauthenticated check
      const checkBefore = await authCheckGET(new NextRequest("http://127.0.0.1:3000/api/auth/check"));
      expect(await checkBefore.json()).toEqual({ authRequired: true, authenticated: false });

      // 2. Wrong password -> 401
      const badLogin = await authLoginPOST(
        new NextRequest("http://127.0.0.1:3000/api/auth/login", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ password: "wrong-password" }),
        }),
      );
      expect(badLogin.status).toBe(401);
      expect(badLogin.cookies.get(SESSION_COOKIE_NAME)).toBeUndefined();

      // 3. Malformed JSON -> 400
      const malformed = await authLoginPOST(
        new NextRequest("http://127.0.0.1:3000/api/auth/login", {
          method: "POST",
          body: "{not-json",
        }),
      );
      expect(malformed.status).toBe(400);

      // 4. Valid password -> 200 + sets signed dca_session cookie
      const goodLogin = await authLoginPOST(
        new NextRequest("http://127.0.0.1:3000/api/auth/login", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ password: "correct-horse-battery" }),
        }),
      );
      expect(goodLogin.status).toBe(200);
      const setCookieHeader = goodLogin.headers.get("set-cookie") ?? "";
      expect(setCookieHeader).toContain(`${SESSION_COOKIE_NAME}=`);
      expect(setCookieHeader.toLowerCase()).toContain("httponly");
      expect(setCookieHeader.toLowerCase()).toContain("samesite=strict");

      const sessionToken = goodLogin.cookies.get(SESSION_COOKIE_NAME)?.value;
      expect(sessionToken).toBeTruthy();

      // 5. Authenticated check -> true
      const checkAfter = await authCheckGET(
        new NextRequest("http://127.0.0.1:3000/api/auth/check", {
          headers: { Cookie: `${SESSION_COOKIE_NAME}=${sessionToken}` },
        }),
      );
      expect(await checkAfter.json()).toEqual({ authRequired: true, authenticated: true });

      // 6. Logout clears cookie (Max-Age=0)
      const logoutRes = await authLogoutPOST();
      expect(logoutRes.status).toBe(200);
      expect(logoutRes.cookies.get(SESSION_COOKIE_NAME)?.value).toBe("");
    });
  });

  describe("Next.js 16 Engine Proxy (src/proxy.ts): CSRF, Auth Gate, Bearer Injection & SSRF Fuzzing", () => {
    it("blocks cross-site Sec-Fetch-Site and mismatched Origin headers (CSRF defense)", () => {
      process.env.DCA_API_KEY = "server-secret-bearer";

      // Cross-site browser fetch -> 403
      const crossSite = proxy(
        new NextRequest("http://localhost:3000/api/engine/api/live/killswitch", {
          method: "POST",
          headers: { "Sec-Fetch-Site": "cross-site", Host: "localhost:3000" },
        }),
      );
      expect(crossSite.status).toBe(403);

      // Mismatched Origin vs Host -> 403
      const crossOrigin = proxy(
        new NextRequest("http://localhost:3000/api/engine/api/bots", {
          method: "POST",
          headers: { Origin: "https://evil.attacker.example", Host: "localhost:3000" },
        }),
      );
      expect(crossOrigin.status).toBe(403);

      // Malformed Origin -> 403
      const malformedOrigin = proxy(
        new NextRequest("http://localhost:3000/api/engine/api/bots", {
          method: "POST",
          headers: { Origin: "://invalid-origin", Host: "localhost:3000" },
        }),
      );
      expect(malformedOrigin.status).toBe(403);
    });

    it("enforces session cookie when DCA_REQUIRE_AUTH=1 while exempting /api/engine/api/health", () => {
      process.env.DCA_REQUIRE_AUTH = "1";
      process.env.DCA_VAULT_KEY = "vault-key-auth-test";
      process.env.DCA_API_KEY = "engine-bearer-secret";

      // Unauthenticated protected endpoint -> 401
      const unauthBots = proxy(new NextRequest("http://127.0.0.1:3000/api/engine/api/bots"));
      expect(unauthBots.status).toBe(401);

      // Unauthenticated health probe -> allowed (rewritten to 127.0.0.1:3002/api/health)
      const healthProbe = proxy(new NextRequest("http://127.0.0.1:3000/api/engine/api/health"));
      expect(healthProbe.status).toBe(200);
      expect(healthProbe.headers.get("x-middleware-rewrite")).toBe("http://127.0.0.1:3002/api/health");

      // Authenticated protected endpoint -> rewritten + server Bearer overwrites spoofed client Authorization
      const validCookie = createSessionToken();
      const authedBots = proxy(
        new NextRequest("http://127.0.0.1:3000/api/engine/api/bots?limit=10", {
          headers: {
            Cookie: `${SESSION_COOKIE_NAME}=${validCookie}`,
            Authorization: "Bearer attacker-forged-token",
            "Sec-Fetch-Site": "same-origin",
          },
        }),
      );
      expect(authedBots.status).toBe(200);
      expect(authedBots.headers.get("x-middleware-rewrite")).toBe("http://127.0.0.1:3002/api/bots?limit=10");
      expect(authedBots.headers.get("x-middleware-request-authorization")).toBe("Bearer engine-bearer-secret");
    });

    it("property: SSRF & path-traversal fuzzer never rewrites outside http://127.0.0.1:<validPort>/api/*", () => {
      process.env.DCA_API_KEY = "fuzz-key";

      const explicitPayloads = [
        "/api/engine/metrics", // Prometheus endpoint outside /api/* must be blocked!
        "/api/engine/admin/config",
        "/api/engine/api/../metrics",
        "/api/engine/api/bots/../../etc/passwd",
        "/api/engine/api/bots%00.json",
        "/api/engine/@169.254.169.254/latest/meta-data",
      ];
      for (const p of explicitPayloads) {
        const req = new NextRequest(`http://127.0.0.1:3000${p}`);
        const res = proxy(req);
        const rewrite = res.headers.get("x-middleware-rewrite");
        if (rewrite) {
          const u = new URL(rewrite);
          expect(u.protocol).toBe("http:");
          expect(u.hostname).toBe("127.0.0.1");
          expect(u.port).toBe("3002");
          expect(u.pathname.startsWith("/api/")).toBe(true);
          expect(u.pathname.includes("..")).toBe(false);
        } else {
          expect([200, 400, 403]).toContain(res.status);
        }
      }

      // Fuzz NEXT_PUBLIC_ENGINE_PORT with arbitrary strings (SSRF via env port injection)
      fc.assert(
        fc.property(fc.string({ maxLength: 30 }), (fuzzPort) => {
          process.env.NEXT_PUBLIC_ENGINE_PORT = fuzzPort;
          const res = proxy(new NextRequest("http://127.0.0.1:3000/api/engine/api/summary"));
          const rewrite = res.headers.get("x-middleware-rewrite");
          expect(rewrite).toBeTruthy();
          const u = new URL(rewrite!);
          expect(u.protocol).toBe("http:");
          expect(u.hostname).toBe("127.0.0.1");
          const portNum = Number(u.port);
          expect(portNum).toBeGreaterThan(0);
          expect(portNum).toBeLessThan(65536);
        }),
        { numRuns: 150 },
      );
    });
  });

  describe("SSE Stream Proxy (/api/stream) & Health Proxy (/api/health)", () => {
    it("blocks unauthenticated SSE stream when DCA_REQUIRE_AUTH=1 and proxies authenticated SSE with Bearer", async () => {
      process.env.DCA_REQUIRE_AUTH = "1";
      process.env.DCA_VAULT_KEY = "sse-vault-secret";
      process.env.DCA_API_KEY = "sse-engine-bearer";

      // 1. Unauthenticated -> 401
      const denied = await streamGET(new Request("http://127.0.0.1:3000/api/stream"));
      expect(denied.status).toBe(401);

      // 2. Authenticated -> proxies upstream ReadableStream, injects Bearer & forwards AbortSignal
      let capturedUrl = "";
      let capturedAuth = "";
      let capturedSignal: AbortSignal | null | undefined;
      globalThis.fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        capturedUrl = String(input);
        const h = new Headers(init?.headers);
        capturedAuth = h.get("Authorization") ?? "";
        capturedSignal = init?.signal;
        const encoder = new TextEncoder();
        const stream = new ReadableStream({
          start(ctrl) {
            ctrl.enqueue(encoder.encode("event: PING\ndata: {}\n\n"));
            ctrl.close();
          },
        });
        return new Response(stream, { status: 200, headers: { "Content-Type": "text/event-stream" } });
      }) as typeof fetch;

      const token = createSessionToken();
      const abortCtrl = new AbortController();
      const allowed = await streamGET(
        new Request("http://127.0.0.1:3000/api/stream", {
          headers: { Cookie: `${SESSION_COOKIE_NAME}=${token}` },
          signal: abortCtrl.signal,
        }),
      );
      expect(allowed.status).toBe(200);
      expect(capturedUrl).toBe("http://127.0.0.1:3002/api/stream");
      expect(capturedAuth).toBe("Bearer sse-engine-bearer");
      expect(capturedSignal).toBeDefined();
      expect(allowed.headers.get("Content-Type")).toContain("text/event-stream");
      expect(await allowed.text()).toContain("event: PING");
    });

    it("returns 502 Bad Gateway cleanly when Rust engine is unreachable on /api/stream and /api/health", async () => {
      globalThis.fetch = vi.fn(async () => {
        throw new Error("connect ECONNREFUSED 127.0.0.1:3002");
      }) as typeof fetch;

      const streamRes = await streamGET(new Request("http://127.0.0.1:3000/api/stream"));
      expect(streamRes.status).toBe(502);

      const healthRes = await healthGET();
      expect(healthRes.status).toBe(502);
      const body = await healthRes.json();
      expect(body.status).toBe("error");
      expect(body.error).toBe("engine unreachable");
    });
  });
});
