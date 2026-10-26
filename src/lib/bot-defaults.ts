/**
 * DCABOT merkezi varsayilanlar + 3Commas sektor sozlugu + strateji sablonlari.
 *
 * Sektor kurali: 3Commas'ta motor terimleri INGILIZCE kalir
 * (Deal, Safety Order, Take Profit, Trailing, Grid, Hedge, Signal, Webhook,
 *  Paper/Real, LONG/SHORT), aciklamalar TURKCE'dir. Bu dosya tek kaynaktir —
 * ekranlara sabit gommeyin, buradan import edin.
 */

import type { Instrument, SignalConfig } from "@/lib/engine-types";
import { fracToNum } from "@/lib/format";

/* ------------------------------------------------------------------ */
/* Motor baglantisi — env override'li, varsayilan 3002 (gateway sozlesmesi) */
/* ------------------------------------------------------------------ */

export const ENGINE_PORT =
  (typeof process !== "undefined" && process.env.NEXT_PUBLIC_ENGINE_PORT) || "3002";

/* ------------------------------------------------------------------ */
/* Mikro-yapi (tick/step/fee): borsadan gelir, sabitler SADECE fallback'tir. */
/* Spot BTC varsayilan; linear'da maker/taker daha dusuktur (Bybit v5).    */
/* ------------------------------------------------------------------ */

export interface MicroStructure {
  tickSize: string;
  stepSize: string;
  makerFee: string;
  takerFee: string;
}

/** Kategori bazli fallback — hicbir enstruman verisi yokken son care. */
export const MICRO_FALLBACK: Record<"spot" | "linear", MicroStructure> = {
  spot: { tickSize: "0.1", stepSize: "0.00001", makerFee: "0.001", takerFee: "0.001" },
  linear: { tickSize: "0.1", stepSize: "0.001", makerFee: "0.0002", takerFee: "0.00055" },
};

/** Enstruman varsa onun degerleri, yoksa kategori fallback'i. */
export function microOf(
  instrument: Instrument | null,
  market: "spot" | "linear",
): MicroStructure {
  const fb = MICRO_FALLBACK[market];
  if (!instrument) return fb;
  return {
    tickSize: String(fracToNum(instrument.tick_size) ?? Number(fb.tickSize)),
    stepSize: String(fracToNum(instrument.step_size) ?? Number(fb.stepSize)),
    makerFee: String(fracToNum(instrument.maker_fee_rate) ?? Number(fb.makerFee)),
    takerFee: String(fracToNum(instrument.taker_fee_rate) ?? Number(fb.takerFee)),
  };
}

/* ------------------------------------------------------------------ */
/* Sihirbaz / hesaplayici ortak varsayilanlari (tek kaynak)               */
/* ------------------------------------------------------------------ */

export const BOT_DEFAULTS = {
  baseSize: "500",
  safetySize: "500",
  deviationPct: 2,
  stepScale: "1.3",
  volumeScale: "1.2",
  maxSafety: 5,
  targetProfitPct: 2,
  leverageFutures: 5,
  leverageSpot: 1,
  gridLevels: 10,
  startCash: "100000",
  backtestBars: 500,
  hedgeRatio: "0.5",
  combinedTp: "50",
  hedgeDdTrigger: "6",
  interval: "1h",
  symbol: "BTCUSDT",
} as const;

/* ------------------------------------------------------------------ */
/* 3Commas sektor sozlugu: terim EN kalir, aciklama TR'dir.               */
/* UI'da dogrudan string gommek yerine t() kullanin.                      */
/* ------------------------------------------------------------------ */

export const TERM: Record<string, { en: string; tr: string }> = {
  deal: { en: "Deal", tr: "Botun actigi pozisyon dongusu; Take Profit veya Stop ile kapanir." },
  safetyOrder: { en: "Safety Order", tr: "Fiyat terse gidince ortalamayi duzeltmek icin atilan guvenlik emri." },
  baseOrder: { en: "Base Order", tr: "Deal'i baslatan ilk emir." },
  takeProfit: { en: "Take Profit", tr: "Deal'i karla kapatan hedef fiyat (ucretler dusulmus net TP)." },
  trailing: { en: "Trailing Take Profit", tr: "Kar hedefini fiyatla birlikte surukleyen dinamik cikis." },
  stopLoss: { en: "Stop-Loss", tr: "Zarari sinirlayan cikis; merdivenden darsa M20 calisir." },
  grid: { en: "Grid", tr: "Belirli araliga dizilmis alis/satis mesh emirleri." },
  hedge: { en: "Hedge", tr: "Ana pozisyonu ters yonlu koruma bacagiyla sigortalama." },
  signal: { en: "Signal", tr: "Disaridan (TradingView/Pine/gosterge) gelen baslatma sinyali." },
  webhook: { en: "Webhook", tr: "TradingView alert'ini bota baglayan URL + secret." },
  paper: { en: "Paper", tr: "Sanal bakiyeyle risksiz deneme modu." },
  live: { en: "Live", tr: "Gercek borsa anahtariyla gercek emir modu." },
  averaging: { en: "Averaging", tr: "Ortalama maliyet duzeltme (DCA merdiveni)." },
  drawdown: { en: "Drawdown", tr: "Zirveden dusus; gunluk kesici buna baglidir." },
  funding: { en: "Funding", tr: "Vadeli kontratta periyodik tasiyici odemesi (long/short arasi)." },
  liquidation: { en: "Liquidation", tr: "Teminat yetmeyince pozisyonun zorla kapatilmasi." },
};

/* ------------------------------------------------------------------ */
/* Hazir sinyal agaclari (rejim etiketli, 3Commas tarzı Deal Start).      */
/* Hepsi fail-closed: kosul saglanmazsa deal ACILMAZ.                     */
/* ------------------------------------------------------------------ */

export interface SignalPreset {
  name: string;
  regime: string;
  desc: string;
  tree: SignalConfig;
}

export const SIGNAL_PRESETS: SignalPreset[] = [
  {
    name: "RSI Dip (Yatay/Dusus)",
    regime: "RANGE",
    desc: "RSI asiri satimdan donerken baslatir; yatay ve dusen piyasada dipten Deal acar.",
    tree: {
      root: {
        op: "And",
        children: [
          { left: { key: "Rsi", timeframe: "H1", period: 14 }, op: "Lt", right: { kind: "number", value: 35 } },
          { left: { key: "Price", timeframe: "H1", output: "close" }, op: "Gt", right: { kind: "indicator", value: { key: "Ema", timeframe: "H1", period: 200 } } },
        ],
      },
    },
  },
  {
    name: "EMA Trend Onayi (Yukselis)",
    regime: "TREND_UP",
    desc: "Kisa EMA uzun EMA'yi yukari keserken baslatir; trendin erken asamasinda giris.",
    tree: {
      root: {
        left: { key: "Ema", timeframe: "H1", period: 9 },
        op: "CrossUp",
        right: { kind: "indicator", value: { key: "Ema", timeframe: "H1", period: 21 } },
      },
    },
  },
  {
    name: "MACD Momentum (Kirilim)",
    regime: "BREAKOUT",
    desc: "MACD sinyal cizgisini yukari keserken baslatir; momentum kirilimlarinda giris.",
    tree: {
      root: {
        left: { key: "Macd", timeframe: "H1", output: "macd" },
        op: "CrossUp",
        right: { kind: "indicator", value: { key: "Macd", timeframe: "H1", output: "signal" } },
      },
    },
  },
  {
    name: "Asiri Alimdan Kacis (Dususten Korunma)",
    regime: "TREND_DOWN",
    desc: "RSI asiri alimda iken girisi engeller; short botlar icin ideal start filtresi.",
    tree: {
      root: {
        left: { key: "Rsi", timeframe: "H1", period: 14 },
        op: "Gt",
        right: { kind: "number", value: 60 },
      },
    },
  },
];

/* ------------------------------------------------------------------ */
/* Pine strateji sablonlari — motor alt kumesiyle uyumlu SADECE sunlar:  */
/* ta.sma/ema/rma/rsi/tr/atr/highest/lowest/change/mom/crossover/       */
/* crossunder/cross/sum/cum/barssince + strategy.entry/close/close_all/ */
/* exit. Bollinger/Supertrend YOKTUR (motor reddeder).                   */
/* ------------------------------------------------------------------ */

export interface PinePreset {
  name: string;
  regime: string;
  desc: string;
  code: string;
}

export const PINE_PRESETS: PinePreset[] = [
  {
    name: "EMA Kesisim (Trend)",
    regime: "TREND",
    desc: "9/21 EMA kesisimi giris, ters kesisim cikis. Trend piyasalarin klasik cozumu.",
    code: `//@version=5
strategy("EMA Kesisim")
fastLen = input.int(9, "Hizli EMA")
slowLen = input.int(21, "Yavas EMA")
fast = ta.ema(close, fastLen)
slow = ta.ema(close, slowLen)
if ta.crossover(fast, slow)
    strategy.entry("Long", strategy.long)
if ta.crossunder(fast, slow)
    strategy.close_all()
`,
  },
  {
    name: "RSI Dip Alimi (Yatay)",
    regime: "RANGE",
    desc: "RSI dibe vurup toparlaninca giris, asiri alimda cikis. Yatay piyasada ortalamaya donus.",
    code: `//@version=5
strategy("RSI Dip")
len = input.int(14, "RSI Periyodu")
dip = input.float(30, "Dip Esigi")
rec = input.float(55, "Toparlanma Esigi")
r = ta.rsi(close, len)
if ta.crossover(r, dip)
    strategy.entry("Dip", strategy.long)
if r > rec
    strategy.close_all()
`,
  },
  {
    name: "Donchian Kirilim + ATR Stop (Kirilim)",
    regime: "BREAKOUT",
    desc: "Kanal kirilimi giris, ATR stop cikis. Sıkısan piyasalarin patlama anini yakalar.",
    code: `//@version=5
strategy("Donchian Kirilim")
len = input.int(20, "Kanal Periyodu")
atrLen = input.int(14, "ATR Periyodu")
stopMult = input.float(3, "ATR Stop Carpan")
upper = ta.highest(high, len)[1]
lower = ta.lowest(low, len)[1]
atr = ta.atr(atrLen)
if close > upper
    strategy.entry("Kirilim", strategy.long)
if close < lower
    strategy.close_all()
strategy.exit("Stop", stop=close - stopMult * atr)
`,
  },
  {
    name: "Kanal Donusu + RSI Filtresi (Yatay Korumali)",
    regime: "RANGE",
    desc: "En dusuk kanal seviyesinden donus + RSI asiri satim onayi; dususte erken girisi filtreler.",
    code: `//@version=5
strategy("Kanal Donusu")
len = input.int(20, "Kanal Periyodu")
rsiLen = input.int(14, "RSI Periyodu")
rsiCap = input.float(40, "RSI Ust Sinir")
floor = ta.lowest(low, len)[1]
r = ta.rsi(close, rsiLen)
if close > floor and r < rsiCap
    strategy.entry("Donus", strategy.long)
if r > 60
    strategy.close_all()
`,
  },
  {
    name: "ATR Trailing Cikisli Trend Surus (Trend)",
    regime: "TREND",
    desc: "EMA uzeri giris, ATR ile suruklenen stop cikis; kara gecince stop maliyete cekilir.",
    code: `//@version=5
strategy("Trend Surus")
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
`,
  },
  {
    name: "Momentum Onayli Giris (Dusuk Volatilite)",
    regime: "LOW_VOL",
    desc: "Fiyat degisimi pozitife donup RSI 50 uzerine cikinca giris; olu piyasada bekler.",
    code: `//@version=5
strategy("Momentum Onay")
momLen = input.int(10, "Momentum Periyodu")
rsiLen = input.int(14, "RSI Periyodu")
m = ta.mom(close, momLen)
r = ta.rsi(close, rsiLen)
if m > 0 and r > 50
    strategy.entry("Mom", strategy.long)
if m < 0 and r < 50
    strategy.close_all()
`,
  },
];
