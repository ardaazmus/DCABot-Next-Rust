/* Faz D — sihirbaz statik verisi + saf tetikleyici yardımcıları (state yok). */
import { Grid3x3, Infinity as InfinityIcon, ShieldHalf, TrendingDown, TrendingUp, Workflow, Zap } from "lucide-react";
import { PINE_PRESETS } from "@/lib/bot-defaults";
import type { BotKind, EntryTrigger, EntryTriggerMode, Interval, UserRiskPolicy } from "@/lib/engine-types";

export interface StrategyCard {
  key: string;
  kind: BotKind;
  name: string;
  desc: string;
  icon: React.ElementType;
  color: string;
  /** Signal Bot havuzu: motor kind'i DCA'dir, giris sinyal agaci on-yukludur. */
  signalPool?: "SPOT" | "FUTURES";
  signalPreset?: number;
  pinePreset?: number;
  presetLeverage?: number;
}

export const STRATEGIES: StrategyCard[] = [
  { key: "DCA_LONG", kind: "DCA_LONG", name: "DCA Long", desc: "Düşüşte ortalamala, TP'de kâr. Klasik spot/vadeli DCA merdiveni.", icon: TrendingUp, color: "emerald" },
  { key: "DCA_SHORT", kind: "DCA_SHORT", name: "DCA Short", desc: "Yükselişte short ortalamala, TP'de kapat. Vadeli gerektirir.", icon: TrendingDown, color: "rose" },
  { key: "FUTURES_DCA", kind: "FUTURES_DCA", name: "Futures DCA", desc: "Kaldıraçlı vadeli DCA Long — izole marjin, M07 likidasyon sınırı ve LIQ zorunlu kapanış.", icon: Zap, color: "amber" },
  { key: "SIGNAL_SPOT", kind: "DCA_LONG", name: "Signal Spot", desc: "Sinyalle baslayan spot Deal: RSI Dip agaci on-yuklu, Pine ve Webhook eklenebilir. 3Commas Signal Bot karsiligi.", icon: Workflow, color: "cyan", signalPool: "SPOT", signalPreset: 0, pinePreset: 1 },
  { key: "SIGNAL_FUTURES", kind: "FUTURES_DCA", name: "Signal Futures", desc: "Sinyalle baslayan vadeli Deal: EMA trend agaci on-yuklu, 5x izole, Pine ve Webhook eklenebilir.", icon: Workflow, color: "amber", signalPool: "FUTURES", signalPreset: 1, pinePreset: 0, presetLeverage: 5 },
  { key: "SPOT_GRID", kind: "SPOT_GRID", name: "Spot Grid", desc: "Yatay piyasada al-sat mesh'i; her seviye maker ücretiyle döner.", icon: Grid3x3, color: "cyan" },
  { key: "FUTURES_GRID", kind: "FUTURES_GRID", name: "Futures Grid", desc: "Kaldıraçlı vadeli grid — aralık stratejisi.", icon: Grid3x3, color: "cyan" },
  { key: "INFINITY_GRID", kind: "INFINITY_GRID", name: "Infinity Grid", desc: "Trendle kayan grid; aralık dışına çıkmaz (M17 delta).", icon: InfinityIcon, color: "cyan" },
  { key: "HEDGE_DUAL", kind: "HEDGE_DUAL", name: "Hedge Dual", desc: "Eş zamanlı long+short korumalı çift pozisyon (M18).", icon: ShieldHalf, color: "amber" },
  { key: "HEDGE_PROTECTIVE", kind: "HEDGE_PROTECTIVE", name: "Hedge Protective", desc: "Ana long pozisyonu belirlediğin düşüş eşiğinde otomatik short koruma bacağı açar; birleşik hedefte kapanır (M18).", icon: ShieldHalf, color: "amber" },
];

/* Task 21 — pine plot çizgi paleti (ana grafik overlay; mavi/indigo yok). */
export const PLOT_COLORS = ["#34d399", "#22d3ee", "#f59e0b", "#a78bfa", "#f43f5e", "#fb923c"];

export const INTERVALS: Interval[] = ["1m", "3m", "5m", "15m", "30m", "1h", "2h", "4h", "1d"];

export const POPULAR_SYMBOLS = [
  "BTCUSDT", "ETHUSDT", "SOLUSDT", "XRPUSDT", "DOGEUSDT", "BNBUSDT",
  "ADAUSDT", "AVAXUSDT", "SUIUSDT", "PEPEUSDT", "NEARUSDT", "LINKUSDT",
  "BTCUSDC", "ETHUSDC", "SOLUSDC",
];

export const QUICK_COINS = ["BTC", "ETH", "SOL", "XRP", "DOGE", "BNB", "AVAX", "SUI"];

export const STEPS = ["Strateji", "Borsa & Sembol", "Parametreler & Koşul", "Sermaye", "Gözden Geçir", "Risk Motoru"];

/* Task 16 — advisory guard rows (UserRiskPolicy GuardAction fields). */
export type GuardKey =
  | "session_guard"
  | "opening_salvo_buffer"
  | "gap_consolidation"
  | "macro_blackout"
  | "regime_filter"
  | "m20_sl_ladder_guard"
  | "grid_fee_breaker";

export const GUARD_ROWS: { key: GuardKey; name: string; desc: string }[] = [
  { key: "session_guard", name: "Gate 7 — Seans Koruması", desc: "Piyasa kapalıyken (gece/hafta sonu/tatil) yeni alım emirlerini bekletir." },
  { key: "opening_salvo_buffer", name: "Gate 8a — Açılış Salvo Tamponu", desc: "ABD açılışının ilk 5 dakikasında (09:30–09:35 ET, 550 bps spread) güvenlik alımlarını erteler." },
  { key: "gap_consolidation", name: "Gate 8b — Gap Konsolidasyonu", desc: "Fiyat birden çok kademeyi atlarsa tüm atlanan miktarları açılış fiyatından TEK dolumda birleştirir (tek komisyon)." },
  { key: "macro_blackout", name: "Makro Kara Pencere", desc: "FOMC faiz kararı ve TÜFE (CPI) açıklamalarının ±15 dakikasında alımı durdurur." },
  { key: "regime_filter", name: "Rejim Filtresi", desc: "Piyasa ÇÖKÜŞ veya YÜKSEK VOLATİLİTE rejimindeyken yeni deal açmayı durdurur." },
  { key: "m20_sl_ladder_guard", name: "M20 — SL / Merdiven Çakışması", desc: "Stop-Loss'un güvenlik merdiveninin toplam sapmasından dar olup olmadığını denetler." },
  { key: "grid_fee_breaker", name: "Grid Komisyon Kırıcı (g_min)", desc: "Izgara adım aralığının komisyon toplamından dar olmasını denetler." },
];

export const SESSION_TYPES: { value: UserRiskPolicy["session_type"]; label: string }[] = [
  { value: "CRYPTO_24_7", label: "Kripto 24/7" },
  { value: "US_EQUITY", label: "ABD Hissesi" },
  { value: "COMMODITY_23_5", label: "Emtia 23-5" },
];

export const SC_MODES: { mode: EntryTriggerMode; label: string; hint: string }[] = [
  { mode: "INSTANT", label: "Anında", hint: "Bot hemen işlemeye başlar (mevcut davranış)." },
  { mode: "PRICE_BELOW", label: "Fiyat Altında", hint: "Kapanış fiyatı eşiğe inince veya altına geçince başlar." },
  { mode: "PRICE_ABOVE", label: "Fiyat Üstünde", hint: "Kapanış fiyatı eşiğe çıkınca veya üstüne geçince başlar." },
  { mode: "DROP_FROM_REF", label: "Düşüş %", hint: "Referans fiyattan X% düşüş tetikler (örn. dip avı)." },
  { mode: "RSI_BELOW", label: "RSI Altında", hint: "14 periyot RSI eşiğin altına inince başlar (yalnız DCA)." },
  { mode: "SCHEDULED", label: "Zamanlanmış", hint: "Seçtiğiniz tarih/saatte otomatik başlar (yerel saat -> epoch ms)." },
  { mode: "PINE", label: "Pine Script", hint: "Pine uyumlu alt-küme stratejisi: kod giriş/çıkış sinyalleri üretir (her barda değerlendirilir)." },
];

/* Task 19 — Pine alt-küme hazır şablonları (tek tıkla doldurulur). */
export const PINE_TEMPLATES: { name: string; code: string }[] = [
  {
    name: "EMA Kesişim",
    code: `//@version=5
strategy("EMA Kesişim")
fastLen = input.int(9, "Hızlı EMA")
slowLen = input.int(21, "Yavaş EMA")
fast = ta.ema(close, fastLen)
slow = ta.ema(close, slowLen)
if ta.crossover(fast, slow)
    strategy.entry("Long", strategy.long)
if ta.crossunder(fast, slow)
    strategy.close_all()
`,
  },
  {
    name: "RSI Dip Alımı",
    code: `//@version=5
strategy("RSI Dip")
len = input.int(14, "RSI Periyodu")
dip = input.float(30, "Dip Eşiği")
rec = input.float(55, "Toparlanma Eşiği")
r = ta.rsi(close, len)
if ta.crossover(r, dip)
    strategy.entry("Dip", strategy.long)
if r > rec
    strategy.close_all()
`,
  },
  {
    name: "Donchian Kırılım + ATR Stop",
    code: `//@version=5
strategy("Donchian Kırılım")
len = input.int(20, "Kanal Periyodu")
atrLen = input.int(14, "ATR Periyodu")
stopMult = input.float(3, "ATR Stop Çarpanı")
upper = ta.highest(high, len)[1]
lower = ta.lowest(low, len)[1]
atr = ta.atr(atrLen)
if close > upper
    strategy.entry("Kırılım", strategy.long)
if close < lower
    strategy.close_all()
strategy.exit("Stop", stop=close - stopMult * atr)
`,
  },
  // ---- Sektör-standart hazır stratejiler (motorda canlı doğrulanmış alt küme) ----
  {
    name: "MACD Sinyal Kesişimi",
    code: `//@version=5
strategy("MACD Kesişim")
fastLen = input.int(12, "Hızlı EMA")
slowLen = input.int(26, "Yavaş EMA")
sigLen = input.int(9, "Sinyal EMA")
macdLine = ta.ema(close, fastLen) - ta.ema(close, slowLen)
signal = ta.ema(macdLine, sigLen)
if ta.crossover(macdLine, signal)
    strategy.entry("MACD", strategy.long)
if ta.crossunder(macdLine, signal)
    strategy.close_all()
`,
  },
  {
    name: "RSI Dip + EMA Trend Filtresi",
    code: `//@version=5
strategy("RSI + Trend")
rsiLen = input.int(14, "RSI Periyodu")
dip = input.float(30, "Dip Eşiği")
rec = input.float(55, "Toparlanma Eşiği")
trendLen = input.int(200, "Trend EMA")
r = ta.rsi(close, rsiLen)
trend = ta.ema(close, trendLen)
longOk = close > trend
if ta.crossover(r, dip) and longOk
    strategy.entry("DipTrend", strategy.long)
if r > rec
    strategy.close_all()
`,
  },
  {
    name: "EMA Kesişim + ATR Stop",
    code: `//@version=5
strategy("EMA + ATR Stop")
fastLen = input.int(9, "Hızlı EMA")
slowLen = input.int(21, "Yavaş EMA")
atrLen = input.int(14, "ATR Periyodu")
stopMult = input.float(2.5, "ATR Stop Çarpanı")
fast = ta.ema(close, fastLen)
slow = ta.ema(close, slowLen)
atr = ta.atr(atrLen)
if ta.crossover(fast, slow)
    strategy.entry("Kesişim", strategy.long)
if ta.crossunder(fast, slow)
    strategy.close_all()
strategy.exit("ATRStop", stop=close - stopMult * atr)
`,
  },
  {
    name: "Momentum Kırılım (ROC)",
    code: `//@version=5
strategy("Momentum Kırılım")
momLen = input.int(10, "Momentum Periyodu")
thr = input.float(1.5, "Kırılım Eşiği %")
exitMom = input.float(-1, "Çıkış Eşiği %")
m = ta.mom(close, momLen) / close * 100
if m > thr
    strategy.entry("Momentum", strategy.long)
if m < exitMom
    strategy.close_all()
`,
  },
  {
    name: "Bollinger Dip (Mean-Revert)",
    code: `//@version=5
strategy("Bollinger Dip")
len = input.int(20, "Periyot")
mult = input.float(2, "Çarpan")
basis = ta.sma(close, len)
dev = ta.stdev(close, len)
lower = basis - mult * dev
if close < lower
    strategy.entry("BB Dip", strategy.long)
if ta.crossover(close, basis)
    strategy.close_all()
`,
  },
  {
    name: "Supertrend Takip",
    code: `//@version=5
strategy("Supertrend Takip")
atrLen = input.int(10, "ATR Periyodu")
mult = input.float(3, "Çarpan")
st = ta.supertrend(mult, atrLen)
if ta.crossover(close, st)
    strategy.entry("ST", strategy.long)
if ta.crossunder(close, st)
    strategy.close_all()
`,
  },
  {
    name: "Stochastic Dönüş",
    code: `//@version=5
strategy("Stochastic Dönüş")
len = input.int(14, "Stoch Periyodu")
dip = input.float(25, "Dip Eşiği")
peak = input.float(80, "Tavan Eşiği")
k = ta.stoch(close, high, low, len)
d = ta.sma(k, 3)
if ta.crossover(k, d) and k < dip
    strategy.entry("Stoch", strategy.long)
if k > peak
    strategy.close_all()
`,
  },
  {
    name: "ADX Trend + EMA Kesişim",
    code: `//@version=5
strategy("ADX + EMA")
adxLen = input.int(14, "ADX Periyodu")
adxMin = input.float(20, "Min Trend Gücü")
fast = ta.ema(close, 9)
slow = ta.ema(close, 21)
adx = ta.adx(adxLen)
if ta.crossover(fast, slow) and adx > adxMin
    strategy.entry("ADX+EMA", strategy.long)
if ta.crossunder(fast, slow)
    strategy.close_all()
`,
  },
  // Rejim sablonlari — motor alt kumesiyle uyumlu (ta.* + strategy.*).
  ...PINE_PRESETS.slice(3).map((p) => ({ name: `${p.name} [${p.regime}]`, code: p.code })),
];

export function describeTrigger(m: EntryTriggerMode, price: string, ref: string, dropPct: number, rsi: number, dtLocal: string): string {
  switch (m) {
    case "INSTANT":
      return "ANINDA";
    case "PINE":
      return "PINE STRATEJİ";
    case "PRICE_BELOW":
      return price ? `FİYAT ≤ ${price}` : "FİYAT ≤ ?";
    case "PRICE_ABOVE":
      return price ? `FİYAT ≥ ${price}` : "FİYAT ≥ ?";
    case "DROP_FROM_REF":
      return ref ? `DÜŞÜŞ ≥ ${dropPct}% (${ref})` : `DÜŞÜŞ ≥ ${dropPct}%`; 
    case "RSI_BELOW":
      return `RSI < ${rsi}`;
    case "SCHEDULED": {
      const ms = Date.parse(dtLocal);
      return Number.isNaN(ms) ? "ZAMANLANMIŞ ?" : `ZAMANLANMIŞ ${new Date(ms).toLocaleString("tr-TR", { dateStyle: "short", timeStyle: "short" })}`;
    }
  }
}

export function triggerToWire(m: EntryTriggerMode, price: string, ref: string, dropPct: number, rsi: number, dtLocal: string): EntryTrigger {
  switch (m) {
    case "INSTANT":
      return "INSTANT";
    case "PINE":
      // Pine stratejisi signal_ok kanalından gating yapar — tetikleyici INSTANT'tır
      // (motor pine_script + INSTANT kombinasyonunu doğrular; başkası 422 döner).
      return "INSTANT";
    case "PRICE_BELOW":
      return { PRICE_BELOW: price };
    case "PRICE_ABOVE":
      return { PRICE_ABOVE: price };
    case "DROP_FROM_REF":
      return { DROP_FROM_REF: { ref_price: ref, drop_pct: String(dropPct / 100) } };
    case "RSI_BELOW":
      return { RSI_BELOW: rsi };
    case "SCHEDULED":
      return { SCHEDULED: { start_ms: Date.parse(dtLocal) } };
  }
}
