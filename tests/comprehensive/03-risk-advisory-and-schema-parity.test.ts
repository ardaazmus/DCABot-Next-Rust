import fs from "node:fs";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { PINE_PRESETS, SIGNAL_PRESETS } from "@/lib/bot-defaults";
import { consumeCarriedConfig, peekCarriedConfig, saveCarriedConfig } from "@/lib/carry-config";
import { DEFAULT_RISK_POLICY, type GuardAction, type UserRiskPolicy } from "@/lib/engine-types";
import { computeWizardWarnings } from "@/lib/risk-warnings";
import { PINE_TEMPLATES } from "@/components/terminal/wizard/wizard-data";

const ROOT = path.resolve(__dirname, "../..");

describe("Pillar 3 — Rust ↔ TypeScript Cross-Stack Contract Parity & Risk Advisory Matrix", () => {
  describe("Static Source-of-Truth Parity (Rust Crates vs Next.js TypeScript)", () => {
    it("matches every AdvisoryWarning code and exact Turkish spec text between Rust risk_policy.rs and TS risk-warnings.ts", () => {
      const rustRiskPolicy = fs.readFileSync(
        path.join(ROOT, "mini-services/dca-core/crates/dca_kernel/src/risk_policy.rs"),
        "utf8",
      );
      const rustAdvisory = fs.readFileSync(
        path.join(ROOT, "mini-services/dca-core/crates/dca_strategy/src/advisory.rs"),
        "utf8",
      );

      // Trigger all 8 non-grid warnings + 1 grid warning via computeWizardWarnings
      const allPermissive: UserRiskPolicy = {
        session_guard: "PERMISSIVE_WITH_WARNING",
        opening_salvo_buffer: "PERMISSIVE_WITH_WARNING",
        gap_consolidation: "PERMISSIVE_WITH_WARNING",
        macro_blackout: "PERMISSIVE_WITH_WARNING",
        regime_filter: "PERMISSIVE_WITH_WARNING",
        m20_sl_ladder_guard: "PERMISSIVE_WITH_WARNING",
        grid_fee_breaker: "PERMISSIVE_WITH_WARNING",
        active_safety_orders: 3,
        daily_dd_breaker_pct: "0",
        fill_model: "OPTIMISTIC",
        session_type: "US_EQUITY",
      };

      const dcaWarnings = computeWizardWarnings({
        kind: "DCA_LONG",
        stop_loss_pct: "0.01",
        max_safety_orders: 5,
        risk_policy: allPermissive,
        m20: { viable: false, firstUnreachableLevel: 1 },
      });

      const gridWarnings = computeWizardWarnings({
        kind: "SPOT_GRID",
        stop_loss_pct: null,
        max_safety_orders: 5,
        risk_policy: DEFAULT_RISK_POLICY,
        feeBreakerPass: false,
      });

      const allWarnings = [...dcaWarnings, ...gridWarnings];
      const expectedCodes = [
        "M20_SL_CONFLICT",
        "NEGATIVE_FEE_MARGIN",
        "OPENING_SALVO_DISABLED",
        "SESSION_OVERRIDE",
        "MACRO_OVERRIDE",
        "GAP_CONSOLIDATION_OFF",
        "MULTI_SAFETY_RISK",
        "DD_BREAKER_OFF",
        "OPTIMISTIC_FILL",
      ];

      expect(allWarnings.map((w) => w.code).sort()).toEqual([...expectedCodes].sort());

      for (const w of allWarnings) {
        // Every warning code emitted by UI must exist verbatim in Rust advisory.rs
        expect(rustAdvisory).toContain(`"${w.code}"`);
        // Every warning title emitted by UI must exist verbatim in Rust risk_policy.rs texts module
        expect(rustRiskPolicy).toContain(`"${w.title}"`);
      }

      // Severity contract: GAP_CONSOLIDATION_OFF is INFO; all others are WARN
      const gap = allWarnings.find((w) => w.code === "GAP_CONSOLIDATION_OFF")!;
      expect(gap.severity).toBe("INFO");
      for (const w of allWarnings.filter((x) => x.code !== "GAP_CONSOLIDATION_OFF")) {
        expect(w.severity).toBe("WARN");
      }
    });

    it("matches UserRiskPolicy default field values 1-to-1 between Rust impl Default and TS DEFAULT_RISK_POLICY", () => {
      const rustRiskPolicy = fs.readFileSync(
        path.join(ROOT, "mini-services/dca-core/crates/dca_kernel/src/risk_policy.rs"),
        "utf8",
      );

      expect(DEFAULT_RISK_POLICY).toEqual({
        session_guard: "ENFORCE",
        opening_salvo_buffer: "ENFORCE",
        gap_consolidation: "ENFORCE",
        macro_blackout: "ENFORCE",
        regime_filter: "PERMISSIVE_WITH_WARNING",
        m20_sl_ladder_guard: "PERMISSIVE_WITH_WARNING",
        grid_fee_breaker: "PERMISSIVE_WITH_WARNING",
        active_safety_orders: 1,
        daily_dd_breaker_pct: "2",
        fill_model: "REALISTIC",
        session_type: "CRYPTO_24_7",
      });

      // Verify Rust default block carries matching defaults
      expect(rustRiskPolicy).toContain("session_guard: GuardAction::Enforce");
      expect(rustRiskPolicy).toContain("regime_filter: GuardAction::PermissiveWithWarning");
      expect(rustRiskPolicy).toContain("active_safety_orders: 1");
      expect(rustRiskPolicy).toContain('daily_dd_breaker_pct: "2".into()');
      expect(rustRiskPolicy).toContain("fill_model: FillModel::Realistic");
      expect(rustRiskPolicy).toContain("session_type: RiskSessionType::Crypto24_7");

      // Default policy produces ZERO warnings when M20/FeeBreaker pass
      const clean = computeWizardWarnings({
        kind: "DCA_LONG",
        stop_loss_pct: null,
        max_safety_orders: 5,
        risk_policy: DEFAULT_RISK_POLICY,
        m20: { viable: true, firstUnreachableLevel: null },
      });
      expect(clean).toEqual([]);
    });

    it("verifies FAQ & Glossary RAG JSON datasets are synchronized and schema-valid between dca_server and Next.js UI", () => {
      const uiFaq = JSON.parse(fs.readFileSync(path.join(ROOT, "src/data/faq.json"), "utf8"));
      const srvFaq = JSON.parse(
        fs.readFileSync(path.join(ROOT, "mini-services/dca-core/crates/dca_server/data/faq.json"), "utf8"),
      );
      expect(uiFaq).toEqual(srvFaq);
      expect(uiFaq.length).toBeGreaterThan(0);

      const uiGlossary = JSON.parse(fs.readFileSync(path.join(ROOT, "src/data/glossary.json"), "utf8"));
      const srvGlossary = JSON.parse(
        fs.readFileSync(path.join(ROOT, "mini-services/dca-core/crates/dca_server/data/glossary.json"), "utf8"),
      );
      expect(uiGlossary).toEqual(srvGlossary);
      expect(uiGlossary.length).toBeGreaterThan(0);

      for (const item of uiGlossary) {
        expect(typeof item.code).toBe("string");
        expect(item.code.length).toBeGreaterThan(0);
        expect(typeof item.title).toBe("string");
      }
    });

    it("verifies all UI Pine presets & templates use only ta.* functions implemented in Rust dca_pine/src/eval.rs", () => {
      const rustPineEval = fs.readFileSync(
        path.join(ROOT, "mini-services/dca-core/crates/dca_pine/src/eval.rs"),
        "utf8",
      );

      const allScripts = [
        ...PINE_PRESETS.map((p) => ({ source: `PINE_PRESETS:${p.name}`, code: p.code })),
        ...PINE_TEMPLATES.map((p) => ({ source: `PINE_TEMPLATES:${p.name}`, code: p.code })),
      ];
      expect(allScripts.length).toBeGreaterThanOrEqual(6);

      for (const { source, code } of allScripts) {
        expect(code, `${source} must declare strategy()`).toContain("strategy(");
        const taCalls = [...code.matchAll(/\bta\.([a-zA-Z0-9_]+)\s*\(/g)].map((m) => `ta.${m[1]}`);
        for (const fnName of taCalls) {
          expect(rustPineEval, `${source} calls unsupported ${fnName}`).toContain(`"${fnName}"`);
        }
      }

      // Also verify Signal Presets have valid non-empty roots
      expect(SIGNAL_PRESETS.length).toBeGreaterThanOrEqual(4);
      for (const preset of SIGNAL_PRESETS) {
        expect(preset.tree.root).toBeTruthy();
      }
    });
  });

  describe("Combinatorial Risk Warning Domain Rules", () => {
    it("suppresses SESSION_OVERRIDE on CRYPTO_24_7 even when session_guard is PERMISSIVE_WITH_WARNING", () => {
      const cryptoPermissive: UserRiskPolicy = {
        ...DEFAULT_RISK_POLICY,
        session_guard: "PERMISSIVE_WITH_WARNING",
        session_type: "CRYPTO_24_7",
      };
      const wCrypto = computeWizardWarnings({
        kind: "DCA_LONG",
        stop_loss_pct: null,
        max_safety_orders: 5,
        risk_policy: cryptoPermissive,
      });
      expect(wCrypto.some((w) => w.code === "SESSION_OVERRIDE")).toBe(false);

      for (const session of ["US_EQUITY", "COMMODITY_23_5"] as const) {
        const wTrad = computeWizardWarnings({
          kind: "DCA_LONG",
          stop_loss_pct: null,
          max_safety_orders: 5,
          risk_policy: { ...cryptoPermissive, session_type: session },
        });
        expect(wTrad.some((w) => w.code === "SESSION_OVERRIDE")).toBe(true);
      }
    });

    it("isolates M20_SL_CONFLICT to non-Grid bots and NEGATIVE_FEE_MARGIN to Grid bots", () => {
      const m20FailInput = {
        stop_loss_pct: "0.01",
        max_safety_orders: 5,
        risk_policy: DEFAULT_RISK_POLICY,
        m20: { viable: false, firstUnreachableLevel: 1 },
        feeBreakerPass: false,
      };

      for (const dcaKind of ["DCA_LONG", "DCA_SHORT", "FUTURES_DCA", "HEDGE_DUAL", "HEDGE_PROTECTIVE"]) {
        const w = computeWizardWarnings({ ...m20FailInput, kind: dcaKind });
        expect(w.map((x) => x.code)).toContain("M20_SL_CONFLICT");
        expect(w.map((x) => x.code)).not.toContain("NEGATIVE_FEE_MARGIN");
      }

      for (const gridKind of ["SPOT_GRID", "FUTURES_GRID", "INFINITY_GRID"]) {
        const w = computeWizardWarnings({ ...m20FailInput, kind: gridKind });
        expect(w.map((x) => x.code)).toContain("NEGATIVE_FEE_MARGIN");
        expect(w.map((x) => x.code)).not.toContain("M20_SL_CONFLICT");
      }
    });

    it("exhaustively checks 2^4 GuardAction combinations without duplicate warning codes", () => {
      const actions: GuardAction[] = ["ENFORCE", "PERMISSIVE_WITH_WARNING"];
      for (const salvo of actions) {
        for (const sess of actions) {
          for (const macro of actions) {
            for (const gap of actions) {
              const w = computeWizardWarnings({
                kind: "FUTURES_DCA",
                stop_loss_pct: null,
                max_safety_orders: 4,
                risk_policy: {
                  ...DEFAULT_RISK_POLICY,
                  opening_salvo_buffer: salvo,
                  session_guard: sess,
                  macro_blackout: macro,
                  gap_consolidation: gap,
                  session_type: "US_EQUITY",
                },
              });
              const codes = w.map((x) => x.code);
              expect(new Set(codes).size).toBe(codes.length);
              expect(codes.includes("OPENING_SALVO_DISABLED")).toBe(salvo === "PERMISSIVE_WITH_WARNING");
              expect(codes.includes("SESSION_OVERRIDE")).toBe(sess === "PERMISSIVE_WITH_WARNING");
              expect(codes.includes("MACRO_OVERRIDE")).toBe(macro === "PERMISSIVE_WITH_WARNING");
              expect(codes.includes("GAP_CONSOLIDATION_OFF")).toBe(gap === "PERMISSIVE_WITH_WARNING");
            }
          }
        }
      }
    });
  });

  describe("Lab -> Wizard Config Handoff (carry-config.ts): One-Shot Consumption & 10-Min TTL", () => {
    beforeEach(() => {
      window.sessionStorage.clear();
    });

    afterEach(() => {
      vi.useRealTimers();
      window.sessionStorage.clear();
    });

    it("consumes carried config exactly once (idempotent delete-on-read) and enforces 10-minute freshness TTL", () => {
      vi.useFakeTimers();
      vi.setSystemTime(new Date("2026-11-13T10:00:00Z"));

      const ok = saveCarriedConfig({
        source: "lab",
        kind: "FUTURES_DCA",
        symbol: "ETHUSDT",
        interval: "15m",
        venue: "BYBIT",
        startCash: "5000",
        config: { target_profit_pct: "0.015", leverage: "3" },
      });
      expect(ok).toBe(true);
      expect(peekCarriedConfig()?.symbol).toBe("ETHUSDT");

      // First consume succeeds and deletes sessionStorage entry
      const first = consumeCarriedConfig();
      expect(first?.symbol).toBe("ETHUSDT");
      expect(consumeCarriedConfig()).toBeNull();

      // Re-save and let 10 minutes + 1ms pass -> stale config rejected & purged
      saveCarriedConfig({
        source: "lab",
        kind: "DCA_LONG",
        symbol: "SOLUSDT",
        interval: "1h",
        venue: "BYBIT",
        startCash: "1000",
        config: {},
      });
      vi.advanceTimersByTime(10 * 60 * 1000 + 1);
      expect(consumeCarriedConfig()).toBeNull();
      expect(peekCarriedConfig()).toBeNull();
    });

    it("gracefully discards corrupted JSON or wrong-source payloads in sessionStorage", () => {
      window.sessionStorage.setItem("dcabot.carry", "{corrupted-json");
      expect(peekCarriedConfig()).toBeNull();
      expect(consumeCarriedConfig()).toBeNull();

      window.sessionStorage.setItem("dcabot.carry", JSON.stringify({ source: "unknown", ts: Date.now() }));
      expect(peekCarriedConfig()).toBeNull();
    });
  });
});
