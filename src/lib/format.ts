/**
 * Display-only formatting helpers for exact engine numerics.
 * The engine emits decimal strings / "n/d" rationals; these are parsed
 * with Number() ONLY for rendering, never for money math.
 */

export function fracToNum(s: string | number | null | undefined): number | null {
  if (s === null || s === undefined) return null;
  if (typeof s === "number") return isFinite(s) ? s : null;
  const t = s.trim();
  if (t === "") return null;
  if (t.includes("/")) {
    const parts = t.split("/");
    if (parts.length !== 2) return null;
    const n = Number(parts[0]);
    const d = Number(parts[1]);
    if (!isFinite(n) || !isFinite(d) || d === 0) return null;
    return n / d;
  }
  const v = Number(t);
  return isFinite(v) ? v : null;
}

/**
 * BİLİNÇLİ TERCİH (Faz 20, UIX-7 kaydı): biçimlendirme "en-US" kalır —
 * finansal terminal gösterimi "1,234.56" konvansiyonunu kullanır ve kullanıcı
 * bu değerleri motor girişlerine (ondalık nokta = "1234.56") kopyalayabilir.
 * "tr-TR" yapılsaydı "1.234,56" ekrana gelir ve motora kopyalanınca
 * ayrıştırıcıyla çelişirdi. Arayüz dili Türkçe, SAYI konvansiyonu
 * uluslararası finansal gösterim — bu bilinçli bir tasarım kararıdır.
 */
function nf(min: number, max: number): Intl.NumberFormat {
  return new Intl.NumberFormat("en-US", {
    minimumFractionDigits: min,
    maximumFractionDigits: max,
  });
}

/** Adaptive decimals by magnitude (for prices / quantities). */
function adaptiveDecimals(v: number): number {
  const a = Math.abs(v);
  if (a === 0) return 2;
  if (a >= 1000) return 2;
  if (a >= 1) return 4;
  if (a >= 0.01) return 5;
  return 8;
}

/** Format a fraction string ("81642.3", "326569203/4000") with fixed or adaptive decimals. */
export function fmtFrac(s: string | number | null | undefined, dec?: number): string {
  const v = fracToNum(s);
  if (v === null) return "—";
  const d = dec ?? adaptiveDecimals(v);
  return nf(d, d).format(v);
}

/** USD money display: "$1,234.56". */
export function fmtUsd(s: string | number | null | undefined, dec = 2): string {
  const v = fracToNum(s);
  if (v === null) return "—";
  return "$" + nf(dec, dec).format(v);
}

/** Signed USD with explicit plus sign for positives. */
export function fmtUsdSigned(s: string | number | null | undefined, dec = 2): string {
  const v = fracToNum(s);
  if (v === null) return "—";
  const sign = v > 0 ? "+" : "";
  return sign + "$" + nf(dec, dec).format(v);
}

/** Percentage from a fraction-of-one value ("0.02" → "2.00%"). */
export function fmtPctFrac(s: string | number | null | undefined, dec = 2): string {
  const v = fracToNum(s);
  if (v === null) return "—";
  return nf(dec, dec).format(v * 100) + "%";
}

/** Percentage from a value already expressed in percent units ("-3.17" → "-3.17%"). */
export function fmtPctNum(s: string | number | null | undefined, dec = 2): string {
  const v = fracToNum(s);
  if (v === null) return "—";
  return nf(dec, dec).format(v) + "%";
}

export function fmtNum(v: number | null | undefined, dec = 2): string {
  if (v === null || v === undefined || !isFinite(v)) return "—";
  return nf(dec, dec).format(v);
}

/** Timestamp (ms) → "12 Oca 2026 14:03:22" style (tr-TR). */
export function fmtTs(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !isFinite(ms)) return "—";
  return new Date(ms).toLocaleString("tr-TR", {
    day: "2-digit",
    month: "short",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** Short time for log rows: "14:03:22". */
export function fmtTimeShort(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || !isFinite(ms)) return "--:--:--";
  return new Date(ms).toLocaleTimeString("tr-TR", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

/** Duration seconds → "2g 03:12:44" / "03:12:44". */
export function fmtDur(totalSeconds: number | null | undefined): string {
  if (totalSeconds === null || totalSeconds === undefined || !isFinite(totalSeconds) || totalSeconds < 0)
    return "—";
  const s = Math.floor(totalSeconds);
  const days = Math.floor(s / 86400);
  const h = Math.floor((s % 86400) / 3600);
  const m = Math.floor((s % 3600) / 60);
  const sec = s % 60;
  const hh = String(h).padStart(2, "0");
  const mm = String(m).padStart(2, "0");
  const ss = String(sec).padStart(2, "0");
  const core = `${hh}:${mm}:${ss}`;
  return days > 0 ? `${days}g ${core}` : core;
}

/** Human friendly kind names. */
export function kindLabel(kind: string): string {
  switch (kind) {
    case "DCA_LONG":
      return "DCA Long";
    case "DCA_SHORT":
      return "DCA Short";
    case "FUTURES_DCA":
      return "Futures DCA";
    case "SPOT_GRID":
      return "Spot Grid";
    case "FUTURES_GRID":
      return "Futures Grid";
    case "INFINITY_GRID":
      return "Infinity Grid";
    case "HEDGE_PROTECTIVE":
      return "Hedge Protective";
    case "HEDGE_DUAL":
      return "Hedge Dual";
    default:
      return kind;
  }
}

export function intervalLabel(iv: string): string {
  return iv;
}

export function truncateMid(s: string, head = 10, tail = 6): string {
  if (!s) return "";
  if (s.length <= head + tail + 3) return s;
  return `${s.slice(0, head)}…${s.slice(-tail)}`;
}
