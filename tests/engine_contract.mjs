#!/usr/bin/env node
/**
 * Faz 28 (B) — motor API sözleşme hızlı süiti (PAPER, localhost).
 *
 * Kapsam: A) auth duvarı  B) fail-closed girdi doğrulama  C) yaşam döngüsü
 * D) webhook  E) UI proxy + CSRF kapısı.  Kural: yalnız loopback, PAPER,
 * gerçek borsa anahtarı YOK; test botu temizliği garantidir.
 *
 * SSRF sınırı: hedefler AÇIK sabit listesidir (BASE_ENGINE/BASE_UI);
 * path '/' ile başlamalı ve scheme içermez. Dinamik hedef yok.
 * Çalıştırma: node tests/engine_contract.mjs   (motor + UI ayakta olmalı)
 */
import { readFileSync } from "node:fs";

const BASE_ENGINE = "http://127.0.0.1:3002";
const BASE_UI = "http://127.0.0.1:3000";
const ALLOWED_BASES = new Set([BASE_ENGINE, BASE_UI]);

const KEY = (readFileSync(new URL("../.local-runtime/local-keys.env", import.meta.url), "utf8"))
  .split("\n")
  .find((l) => l.startsWith("DCA_API_KEY="))
  ?.trim()
  .split("=")
  .slice(1)
  .join("=");

if (!KEY) {
  console.error("HATA: DCA_API_KEY bulunamadı — önce npm run local:start");
  process.exit(2);
}

let pass = 0;
let fail = 0;

function check(name, cond, detail = "") {
  if (cond) {
    pass += 1;
    console.log(`PASS ${name}${detail ? "  | " + detail : ""}`);
  } else {
    fail += 1;
    console.log(`FAIL ${name}${detail ? "  | " + detail : ""}`);
  }
}

async function req(base, path, method = "GET", body, auth = false) {
  if (!ALLOWED_BASES.has(base)) throw new Error("izinli olmayan taban adres");
  if (!path.startsWith("/") || path.includes("://")) throw new Error("geçersiz yol");
  const headers = {};
  if (body !== undefined) headers["Content-Type"] = "application/json";
  if (auth) headers["Authorization"] = `Bearer ${KEY}`;
  const res = await fetch(base + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(20000),
  });
  // Tam gövde döner (list uçları JSON parse edilir); kırpma yalnız logda.
  const text = await res.text();
  return { status: res.status, text };
}

const CFG = {
  kind: "DCA_LONG", symbol: "BTCUSDT", base_order_size: "50", safety_order_size: "25",
  sizing_mode: "QUOTE_NOTIONAL", deviation_pct: "0.005", step_scale: "1.2",
  volume_scale: "1.1", max_safety_orders: 2, target_profit_pct: "1", use_net_tp: true,
  trailing_enabled: false, trailing_trigger_pct: "1", trailing_deviation_pct: "0.5",
  stop_loss_pct: null, leverage: "1", hedge_ratio: "0.5", combined_target_profit: "50",
  grid_lower: null, grid_upper: null, grid_levels: null, grid_mode: null,
  grid_trail: false, cooldown_bars: 0, entry_trigger: "INSTANT",
};
const bot = (name, symbol, cfg) => ({
  name, kind: cfg.kind, venue: "BYBIT", mode: "PAPER", symbol,
  interval: "1h", start_cash: "1000", config: cfg,
});

// ---------- A) Kimlik doğrulama ----------
console.log("=== A) Kimlik doğrulama ===");
for (const p of ["/api/bots", "/api/summary", "/api/vault/keys", "/api/live/killswitch", "/api/system/maintenance", "/api/live/freeze"]) {
  const r = await req(BASE_ENGINE, p);
  check(`anahtarsız ${p}`, r.status === 401 || r.status === 403, `status=${r.status}`);
}
{
  const r = await req(BASE_ENGINE, "/api/health");
  check("health anahtarsız 200 (tek açık uç)", r.status === 200, `status=${r.status}`);
}

// ---------- B) Girdi doğrulama (fail-closed) ----------
console.log("=== B) Girdi doğrulama ===");
{
  const neg = structuredClone(CFG); neg.base_order_size = "-50";
  let r = await req(BASE_ENGINE, "/api/bots", "POST", bot("audit-neg", "BTCUSDT", neg), true);
  check("negatif base reddi", r.status === 400 || r.status === 422, `status=${r.status}`);
  const zero = structuredClone(CFG); zero.deviation_pct = "0";
  r = await req(BASE_ENGINE, "/api/bots", "POST", bot("audit-zero", "BTCUSDT", zero), true);
  check("sıfır sapma reddi", r.status === 400 || r.status === 422, `status=${r.status}`);
  r = await req(BASE_ENGINE, "/api/bots", "POST", bot("audit-bad", "BTCUSDT", { kind: "DCA_LONG" }), true);
  check("eksik config reddi", r.status === 400 || r.status === 422, `status=${r.status}`);
  // blacklist: engelli sembole create 422
  r = await req(BASE_ENGINE, "/api/blacklist", "POST", { pair: "AUDITUSDT", reason: "sözleşme testi" }, true);
  const added = r.status === 200 || r.status === 201;
  check("blacklist ekleme", added, `status=${r.status}`);
  if (added) {
    r = await req(BASE_ENGINE, "/api/bots", "POST", bot("audit-bl", "AUDITUSDT", { ...CFG, symbol: "AUDITUSDT" }), true);
    check("blacklist sembole create reddi", r.status === 400 || r.status === 422, `status=${r.status}`);
    r = await req(BASE_ENGINE, "/api/blacklist/AUDITUSDT", "DELETE", undefined, true);
    check("blacklist temizliği", r.status === 200 || r.status === 204, `status=${r.status}`);
  }
}

// ---------- C) Yaşam döngüsü ----------
console.log("=== C) Yaşam döngüsü ===");
{
  const r = await req(BASE_ENGINE, "/api/bots", "POST", bot("audit-lc", "BTCUSDT", CFG), true);
  check("create PAPER", r.status === 200 || r.status === 201, `status=${r.status}`);
  const list = await req(BASE_ENGINE, "/api/bots", "GET", undefined, true);
  const bots = list.status === 200 ? JSON.parse(list.text).bots : [];
  const created = bots.find((b) => b.name === "audit-lc");
  check("bot listede", !!created, `list_status=${list.status}`);
  if (created) {
    const id = created.id;
    let x = await req(BASE_ENGINE, `/api/bots/${id}`, "PATCH", { status: "PAUSED" }, true);
    check("pause", x.status === 200 || x.status === 204, `status=${x.status}`);
    x = await req(BASE_ENGINE, `/api/bots/${id}`, "PATCH", { status: "RUNNING" }, true);
    check("resume", x.status === 200 || x.status === 204, `status=${x.status}`);
    x = await req(BASE_ENGINE, `/api/bots/${id}/actions`, "POST", { action: "CLOSE_ALL" }, true);
    check("CLOSE_ALL", x.status === 200 || x.status === 202 || x.status === 204, `status=${x.status}`);
    x = await req(BASE_ENGINE, `/api/bots/${id}`, "DELETE", undefined, true);
    check("delete", x.status === 200 || x.status === 202 || x.status === 204, `status=${x.status}`);
    const after = await req(BASE_ENGINE, "/api/bots", "GET", undefined, true);
    const still = after.status === 200 && JSON.parse(after.text).bots.some((b) => b.id === id);
    check("delete doğrulama (listede yok)", !still);
  }
}

// ---------- D) Webhook ----------
console.log("=== D) Webhook ===");
{
  const r = await req(BASE_ENGINE, "/api/webhooks/tradingview", "POST", {
    secret: "YANLIS", botId: "b-0000", action: "START_DEAL", timestamp: 0,
  });
  check("webhook yanlış secret reddi", r.status === 401 || r.status === 403, `status=${r.status}`);
}

// ---------- E) UI proxy + CSRF kapısı ----------
console.log("=== E) UI proxy ===");
{
  const r = await req(BASE_UI, "/api/engine/api/bots");
  check("proxy Bearer sunucuda ekleniyor", r.status === 200, `status=${r.status}`);
  // Faz 29: proxy.ts varken Next plain rewrite'ı uygulamıyor — /api/bots
  // artık 404 (yol hiçbir şekilde motora çıkmıyor; 401'den de sıkı).
  const direct = await req(BASE_UI, "/api/bots");
  check("proxy dışı yol 404 (rewrite yok — motor kapalı)", direct.status === 404, `status=${direct.status}`);
  const csrf = await fetch(BASE_UI + "/api/engine/api/live/killswitch", {
    method: "POST",
    headers: { "Content-Type": "application/json", Origin: "http://evil.example" },
    body: JSON.stringify({ engaged: true, reason: "x" }),
    signal: AbortSignal.timeout(20000),
  });
  check("cross-origin killswitch → 403", csrf.status === 403, `status=${csrf.status}`);
  const xs = await fetch(BASE_UI + "/api/engine/api/bots", {
    headers: { "Sec-Fetch-Site": "cross-site" },
    signal: AbortSignal.timeout(20000),
  });
  check("cross-site okuma → 403", xs.status === 403, `status=${xs.status}`);
}

console.log("=== ÖZET ===");
console.log(`PASS: ${pass}  FAIL: ${fail}  TOPLAM: ${pass + fail}`);
process.exit(fail === 0 ? 0 : 1);
