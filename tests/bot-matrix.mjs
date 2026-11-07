// DCABOT bot matrix — tum 8 kind + pine + signal agaci, canli Bybit barlariyla.
// Kullanim: bun tests/bot-matrix.mjs  (motor :3002 acik olmali)
// Sektor kurali: uydurma yok — her satir motorun dondugu gercek metrik.
// NOT: DCA ailesi dar pencerede (200 bar) sessizce issiz kalabiliyor (gozlem);
// o yuzden E2E-kaliteli 500 bar pencerede dogrulanir. Hedge'i motor backtest'i
// desteklemez — sozlesme 422'dir, matriks bunu kilitler.
const BASE = process.env.BASE_URL || "http://127.0.0.1:3002";

// Faz 28: motor auth duvari (Faz 21) sonrasi anahtar zorunlu — .local-runtime
// keys dosyasindan okunur; script disina asla yazilmaz.
import { readFileSync } from "node:fs";
const KEY = (readFileSync(new URL("../.local-runtime/local-keys.env", import.meta.url), "utf8"))
  .split(String.fromCharCode(10))
  .find((l) => l.startsWith("DCA_API_KEY="))
  ?.trim()
  .split("=")
  .slice(1)
  .join("=");
if (!KEY) {
  console.error("HATA: DCA_API_KEY bulunamadi — once npm run local:start");
  process.exit(2);
}

async function req(method, path, body, timeoutMs = 300000) {
  const ctrl = new AbortController();
  const t = setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const r = await fetch(`${BASE}${path}`, {
      method,
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${KEY}` },
      body: body ? JSON.stringify(body) : undefined,
      signal: ctrl.signal,
    });
    const j = await r.json().catch(() => ({}));
    return [r.status, j];
  } finally {
    clearTimeout(t);
  }
}

function fullConfig(kind, over = {}) {
  const e2e = over.e2e === true;
  const cfg = {
    kind, symbol: "BTCUSDT",
    base_order_size: e2e ? "100" : "200",
    safety_order_size: e2e ? "50" : "120",
    sizing_mode: "QUOTE_NOTIONAL", deviation_pct: e2e ? "0.02" : "0.01",
    step_scale: "1.5", volume_scale: "1.2", max_safety_orders: e2e ? 3 : 2,
    target_profit_pct: e2e ? "0.015" : "0.01", use_net_tp: true,
    trailing_enabled: false, trailing_trigger_pct: "0.01", trailing_deviation_pct: "0.005",
    stop_loss_pct: null, leverage: kind.includes("FUTURES") ? "5" : "1",
    hedge_ratio: "0.5", combined_target_profit: "1000000",
    hedge_drawdown_trigger_pct: "6",
    grid_lower: kind.includes("GRID") ? "76000" : null,
    grid_upper: kind.includes("GRID") ? "86000" : null,
    grid_levels: kind.includes("GRID") ? 8 : null,
    grid_mode: e2e ? "arithmetic" : "geometric", grid_trail: false, cooldown_bars: 0,
    entry_trigger: "INSTANT",
  };
  const { e2e: _drop, bars: _bars, ...rest } = over;
  return { ...cfg, ...rest };
}

const MATRIX = [
  ["DCA_LONG", { bars: 500, e2e: true }],
  ["DCA_SHORT", { bars: 500, e2e: true }],
  ["FUTURES_DCA", { bars: 300 }],
  ["SPOT_GRID", { bars: 200 }],
  ["FUTURES_GRID", { bars: 200 }],
  ["INFINITY_GRID", { bars: 200 }],
];

let pass = 0, fail = 0;
for (const [kind, over] of MATRIX) {
  const { bars } = over;
  const cfg = fullConfig(kind, over);
  const [s, j] = await req("POST", "/api/backtest", {
    kind, symbol: "BTCUSDT", interval: "1h", bars,
    start_cash: "10000", config: cfg,
  });
  const m = j.metrics || {};
  const traded = Number(m.deals) >= 1 || Number(j.fills) >= 1;
  const ok = s === 200 && traded;
  if (ok) { pass += 1; } else { fail += 1; }
  console.log(`[${ok ? "PASS" : "FAIL"}] ${kind}: status=${s} deals=${m.deals} fills=${j.fills} return=${m.total_return_pct} mdd=${m.max_drawdown_pct} win=${m.win_rate_pct} fees=${m.total_fees} funding=${m.total_funding ?? "-"}`);
}

// Hedge ailesi: Faz 28 dogrulamasi — motor artik hedge backtest'i DE
// destekliyor (200 + fill; eski sozlesme 422'ydi, kaldirildi).
for (const kind of ["HEDGE_DUAL", "HEDGE_PROTECTIVE"]) {
  const cfg = fullConfig(kind, {});
  const [s, j] = await req("POST", "/api/backtest", {
    kind, symbol: "BTCUSDT", interval: "1h", bars: 60,
    start_cash: "10000", config: cfg,
  });
  const ok = s === 200 && (Number(j.fills) >= 1 || Number(j.metrics?.deals) >= 1);
  if (ok) { pass += 1; } else { fail += 1; }
  console.log(`[${ok ? "PASS" : "FAIL"}] ${kind}: status=${s} fills=${j.fills} deals=${j.metrics?.deals}`);
}

// Pine: yeni Trend Surus sablonu gercek barlarda kosuyor mu?
const TREND = `strategy("Trend Surus")
emaLen = input.int(50, "EMA Periyodu")
atrLen = input.int(14, "ATR Periyodu")
stopMult = input.float(2, "ATR Carpan")
trend = ta.ema(close, emaLen)
atr = ta.atr(atrLen)
if close > trend
    strategy.entry("Trend", strategy.long)
if close < trend
    strategy.close_all()
strategy.exit("Trail", stop=close - stopMult * atr)
`;
{
  const [s, j] = await req("POST", "/api/pine/backtest",
    { code: TREND, symbol: "BTCUSDT", interval: "1h", bars: 200 });
  const ok = s === 200 && j.ok === true && Number(j.entries) >= 1;
  if (ok) { pass += 1; } else { fail += 1; }
  console.log(`[${ok ? "PASS" : "FAIL"}] PINE TrendSurus: status=${s} entries=${j.entries} deals=${j.metrics?.deals}`);
}

// Signal agaci: RSI Dip sablonu backtest kapisindan geciyor mu?
{
  const cfg = fullConfig("DCA_LONG", {
    entry_signal: { root: { left: { key: "Rsi", timeframe: "H1", period: 14 }, op: "Lt", right: { kind: "number", value: 90 } } },
  });
  const [s, j] = await req("POST", "/api/backtest", {
    kind: "DCA_LONG", symbol: "BTCUSDT", interval: "1h", bars: 200,
    start_cash: "10000", config: cfg,
  });
  const ok = s === 200 && Number(j.metrics?.deals) >= 1;
  if (ok) { pass += 1; } else { fail += 1; }
  console.log(`[${ok ? "PASS" : "FAIL"}] SIGNAL RSI-Dip: status=${s} deals=${j.metrics?.deals}`);
}

console.log(`\nMATRIX: ${pass} pass / ${fail} fail`);
process.exit(fail ? 1 : 0);
