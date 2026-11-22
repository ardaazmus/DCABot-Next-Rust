import assert from "node:assert/strict";
import crypto from "node:crypto";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import ts from "typescript";

const authSource = readFileSync(new URL("../src/lib/auth-session.ts", import.meta.url), "utf8");
const authModule = ts.transpileModule(authSource, {
  compilerOptions: { module: ts.ModuleKind.ESNext, target: ts.ScriptTarget.ES2022 },
}).outputText;
const authSession = await import(`data:text/javascript;base64,${Buffer.from(authModule).toString("base64")}`);

function withEnv(values, fn) {
  const names = ["DCA_SESSION_SECRET", "DCA_VAULT_KEY", "DCA_API_KEY", "ENGINE_API_KEY", "DCA_UI_PASSWORD", "DCA_REQUIRE_AUTH", "NODE_ENV"];
  const previous = Object.fromEntries(names.map((name) => [name, process.env[name]]));
  try {
    for (const name of names) delete process.env[name];
    Object.assign(process.env, values);
    return fn();
  } finally {
    for (const name of names) {
      if (previous[name] === undefined) delete process.env[name];
      else process.env[name] = previous[name];
    }
  }
}

test("session signing fails closed when no private secret is configured", () => {
  withEnv({}, () => {
    authSession.clearLoginFailures();
    assert.equal(authSession.isSessionSecretConfigured(), false);
    assert.equal(authSession.verifySessionToken(null), false);
    assert.throws(() => authSession.createSessionToken());
  });
});

test("session signing rejects secrets shorter than 32 bytes", () => {
  withEnv({ DCA_SESSION_SECRET: "short-secret" }, () => {
    assert.equal(authSession.isSessionSecretConfigured(), false);
    assert.throws(() => authSession.createSessionToken());
  });
});

test("a token signed with the former public fallback is rejected", () => {
  withEnv({ DCA_VAULT_KEY: "v".repeat(64), DCA_API_KEY: "a".repeat(64) }, () => {
    const payload = Buffer.from(JSON.stringify({
      authenticated: true,
      issuedAt: Date.now(),
      expiresAt: Date.now() + 60_000,
    })).toString("base64url");
    const signature = crypto.createHmac(
      "sha256",
      "dcabot-default-session-secret-change-in-prod",
    ).update(payload).digest("base64url");

    assert.equal(authSession.verifySessionToken(`${payload}.${signature}`), false);
  });
});

test("a configured high-entropy secret signs a verifiable session", () => {
  withEnv({ DCA_SESSION_SECRET: "s".repeat(64) }, () => {
    const token = authSession.createSessionToken();
    assert.equal(authSession.verifySessionToken(token), true);
  });
});

test("expired and non-boolean authenticated payloads are rejected", () => {
  withEnv({ DCA_SESSION_SECRET: "s".repeat(64) }, () => {
    for (const payloadValue of [
      { authenticated: true, issuedAt: Date.now() - 1_000, expiresAt: Date.now() - 1 },
      { authenticated: "true", issuedAt: Date.now() - 1_000, expiresAt: Date.now() + 60_000 },
    ]) {
      const payload = Buffer.from(JSON.stringify(payloadValue)).toString("base64url");
      const signature = crypto.createHmac("sha256", "s".repeat(64)).update(payload).digest("base64url");
      assert.equal(authSession.verifySessionToken(`${payload}.${signature}`), false);
    }
  });
});

test("production requires UI authentication even without an opt-in flag", () => {
  withEnv({ NODE_ENV: "production" }, () => {
    assert.equal(authSession.isAuthRequired(), true);
  });
  withEnv({ NODE_ENV: "development" }, () => {
    assert.equal(authSession.isAuthRequired(), false);
  });
});

test("the UI password cannot fall back to exchange or engine API keys", () => {
  withEnv({ DCA_API_KEY: "shared-api-key", ENGINE_API_KEY: "engine-key" }, () => {
    assert.equal(authSession.isUiPasswordConfigured(), false);
    assert.equal(authSession.verifyPassword("shared-api-key"), false);
    assert.equal(authSession.verifyPassword("engine-key"), false);
  });
});

test("login failures are throttled and the lockout backs off", () => {
  authSession.clearLoginFailures();
  for (let i = 0; i < 4; i += 1) assert.equal(authSession.recordLoginFailure(1_000 + i), 0);
  assert.equal(authSession.recordLoginFailure(1_004), 60_000);
  assert.equal(authSession.loginRateLimitDelay(2_000), 59_004);
  assert.equal(authSession.loginRateLimitDelay(61_004), 0);
  for (let i = 0; i < 4; i += 1) assert.equal(authSession.recordLoginFailure(62_000 + i), 0);
  assert.equal(authSession.recordLoginFailure(62_004), 120_000);
  authSession.clearLoginFailures();
});
