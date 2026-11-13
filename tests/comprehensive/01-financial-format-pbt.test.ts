import fc from "fast-check";
import { describe, expect, it } from "vitest";
import { MICRO_FALLBACK, microOf } from "@/lib/bot-defaults";
import {
  fmtDur,
  fmtFrac,
  fmtNum,
  fmtPctFrac,
  fmtPctNum,
  fmtTimeShort,
  fmtTs,
  fmtUsd,
  fmtUsdSigned,
  fracToNum,
  kindLabel,
  truncateMid,
} from "@/lib/format";

describe("Pillar 1 — Financial Numerics & 4096-Bit Rational Wire Property-Based Suite", () => {
  describe("fracToNum: exact rational ('n/d') & 4096-bit BigInt overflow resilience", () => {
    it("resolves 4096-bit engine rationals (>308 digits) where Number(num) overflows to Infinity", () => {
      // 10^350 exceeds IEEE-754 Number.MAX_VALUE (~1.79e308), yet fits comfortably in 4096-bit Fraction (~10^1233).
      const scale350 = "1" + "0".repeat(350);
      const numStr = "8164235" + "0".repeat(348); // 81642.35 * 10^350
      const wire = `${numStr}/${scale350}`;

      expect(Number(numStr)).toBe(Infinity);
      expect(Number(scale350)).toBe(Infinity);

      const parsed = fracToNum(wire);
      expect(parsed).not.toBeNull();
      expect(parsed!).toBeCloseTo(81642.35, 6);
      expect(fmtUsd(wire)).toBe("$81,642.35");
    });

    it("preserves negative sign and sub-unit precision on 1000-digit BigInt rationals", () => {
      const den = "3" + "0".repeat(900);
      const num = "-1" + "0".repeat(900);
      const parsed = fracToNum(`${num}/${den}`);
      expect(parsed).not.toBeNull();
      expect(parsed!).toBeCloseTo(-1 / 3, 12);
      expect(fmtUsdSigned(`${num}/${den}`, 4)).toBe("-$0.3333");
    });

    it("property: scale-invariance (k*p)/(k*q) == p/q across arbitrary BigInt multipliers up to 4096 bits", () => {
      fc.assert(
        fc.property(
          fc.integer({ min: -10_000_000, max: 10_000_000 }),
          fc.integer({ min: 1, max: 10_000_000 }),
          fc.integer({ min: 1, max: 380 }),
          (p, q, zeroPow) => {
            // Multiply both numerator and denominator by 10^zeroPow (crosses 308-digit IEEE-754 cliff)
            const k = BigInt(10) ** BigInt(zeroPow);
            const scaledNum = (BigInt(p) * k).toString();
            const scaledDen = (BigInt(q) * k).toString();

            const base = fracToNum(`${p}/${q}`);
            const scaled = fracToNum(`${scaledNum}/${scaledDen}`);

            expect(base).not.toBeNull();
            expect(scaled).not.toBeNull();
            expect(Math.abs(scaled! - base!)).toBeLessThanOrEqual(1e-9 * Math.max(1, Math.abs(base!)));
          },
        ),
        { numRuns: 300 },
      );
    });

    it("rejects all malformed, zero-denominator, NaN, and Infinity inputs fail-safe as null", () => {
      const adversarial: Array<string | number | null | undefined> = [
        null,
        undefined,
        "",
        "   ",
        "NaN",
        "Infinity",
        "-Infinity",
        NaN,
        Infinity,
        -Infinity,
        "1/0",
        "-999/0",
        "0/0",
        `${"9".repeat(400)}/0`,
        "1/2/3",
        "/10",
        "10/",
        "abc/def",
        "12.34.56",
        "--5",
      ];
      for (const bad of adversarial) {
        expect(fracToNum(bad)).toBeNull();
        expect(fmtFrac(bad)).toBe("—");
        expect(fmtUsd(bad)).toBe("—");
        expect(fmtUsdSigned(bad)).toBe("—");
        expect(fmtPctFrac(bad)).toBe("—");
        expect(fmtPctNum(bad)).toBe("—");
      }
    });
  });

  describe("Display Formatting Algebraic Laws & Copy-Paste Locale Invariants", () => {
    it("property: fmtUsd & fmtUsdSigned normalize IEEE-754 signed zero (-0 / '-0') and never emit '$-X'", () => {
      // Regression for dashboard.tsx:559 `-(fracToNum("0") ?? 0)` === -0
      expect(Object.is(fracToNum(-0), 0)).toBe(true);
      expect(Object.is(fracToNum("-0"), 0)).toBe(true);
      expect(fmtUsd(-0)).toBe("$0.00");
      expect(fmtUsd("-0")).toBe("$0.00");
      expect(fmtUsd(-12.34)).toBe("-$12.34");
      expect(fmtUsd(-0.0001, 2)).toBe("$0.00");
      expect(fmtUsdSigned(-0)).toBe("+$0.00");
      expect(fmtUsdSigned("-0")).toBe("+$0.00");
      expect(fmtUsdSigned(-0.0001, 2)).toBe("+$0.00");

      fc.assert(
        fc.property(fc.double({ min: -1e12, max: 1e12, noNaN: true, noDefaultInfinity: true }), (x) => {
          const rendered = fmtUsdSigned(x, 2);
          const unsigned = fmtUsd(x, 2);
          expect(rendered).not.toContain("NaN");
          expect(rendered).not.toContain("Infinity");
          expect(rendered).not.toContain("$-");
          expect(unsigned).not.toContain("$-");
          if (x < -0.005) {
            expect(rendered.startsWith("-$")).toBe(true);
            expect(unsigned.startsWith("-$")).toBe(true);
          } else if (x > 0.005) {
            expect(rendered.startsWith("+$")).toBe(true);
            expect(unsigned.startsWith("$")).toBe(true);
          }
        }),
        { numRuns: 400 },
      );
    });

    it("property: fmtFrac uses dot-decimal (en-US) convention across all magnitudes", () => {
      // Adaptive decimal tiers: 0->2, >=1000->2, >=1->4, >=0.01->5, <0.01->8
      expect(fmtFrac("0")).toBe("0.00");
      expect(fmtFrac("84512.1")).toBe("84,512.10");
      expect(fmtFrac("12.345678")).toBe("12.3457");
      expect(fmtFrac("0.0543219")).toBe("0.05432");
      expect(fmtFrac("0.000012349")).toBe("0.00001235");

      fc.assert(
        fc.property(fc.double({ min: 0.000001, max: 999.99, noNaN: true, noDefaultInfinity: true }), (v) => {
          const out = fmtFrac(v);
          // Below 1000 there are no thousands separators; decimal separator MUST be '.' (never Turkish ',')
          expect(out).not.toContain(",");
          expect(out).toMatch(/^\d+\.\d+$/);
        }),
        { numRuns: 250 },
      );
    });

    it("property: fmtPctFrac('x') === fmtPctNum('x * 100') for exact hundredths", () => {
      fc.assert(
        fc.property(fc.integer({ min: -50_000, max: 50_000 }), (bps) => {
          const frac = (bps / 10_000).toFixed(4);
          const pct = (bps / 100).toFixed(2);
          expect(fmtPctFrac(frac, 2)).toBe(fmtPctNum(pct, 2));
        }),
        { numRuns: 300 },
      );
    });

    it("fmtDur handles negative/invalid values, zero, HH:MM:SS boundaries, and multi-day rollovers", () => {
      expect(fmtDur(null)).toBe("—");
      expect(fmtDur(-1)).toBe("—");
      expect(fmtDur(Infinity)).toBe("—");
      expect(fmtDur(0)).toBe("00:00:00");
      expect(fmtDur(59.9)).toBe("00:00:59");
      expect(fmtDur(3661)).toBe("01:01:01");
      expect(fmtDur(86399)).toBe("23:59:59");
      expect(fmtDur(86400)).toBe("1g 00:00:00");
      expect(fmtDur(2 * 86400 + 3 * 3600 + 12 * 60 + 44)).toBe("2g 03:12:44");
    });

    it("fmtTs, fmtTimeShort, fmtNum, kindLabel, and truncateMid preserve boundary contracts", () => {
      expect(fmtTs(null)).toBe("—");
      expect(fmtTs(NaN)).toBe("—");
      expect(fmtTs(1_767_225_600_000)).toContain("2026");

      expect(fmtTimeShort(null)).toBe("--:--:--");
      expect(fmtTimeShort(1_767_225_600_000)).toMatch(/^\d{2}:\d{2}:\d{2}$/);

      expect(fmtNum(NaN)).toBe("—");
      expect(fmtNum(1234.5, 2)).toBe("1,234.50");

      expect(kindLabel("DCA_LONG")).toBe("DCA Long");
      expect(kindLabel("FUTURES_DCA")).toBe("Futures DCA");
      expect(kindLabel("HEDGE_DUAL")).toBe("Hedge Dual");
      expect(kindLabel("CUSTOM_UNKNOWN")).toBe("CUSTOM_UNKNOWN");

      expect(truncateMid("", 4, 4)).toBe("");
      expect(truncateMid("short-id", 6, 4)).toBe("short-id");
      expect(truncateMid("abcdefghijklmnopqrstuvwxyz", 4, 4)).toBe("abcd…wxyz");
    });
  });

  describe("microOf: Venue Microstructure Exact String Preservation", () => {
    it("falls back to spot/linear defaults when instrument is null or has blank/invalid fields", () => {
      expect(microOf(null, "spot")).toEqual(MICRO_FALLBACK.spot);
      expect(microOf(null, "linear")).toEqual(MICRO_FALLBACK.linear);

      const partialInst = {
        symbol: "SHIB1000USDT",
        base: "SHIB1000",
        quote: "USDT",
        tick_size: "0.00000100",
        step_size: "",
        min_qty: "1",
        min_notional: "5",
        maker_fee_rate: "NaN",
        taker_fee_rate: "0.00055",
        max_leverage: "25",
      };
      const resolved = microOf(partialInst, "linear");
      // Exact string preserved (including trailing zeros, never converted through f64)
      expect(resolved.tickSize).toBe("0.00000100");
      expect(resolved.stepSize).toBe(MICRO_FALLBACK.linear.stepSize);
      expect(resolved.makerFee).toBe(MICRO_FALLBACK.linear.makerFee);
      expect(resolved.takerFee).toBe("0.00055");
    });
  });
});
