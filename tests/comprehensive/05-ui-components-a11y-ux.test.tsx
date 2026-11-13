import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { AuthDialog } from "@/components/terminal/auth-dialog";
import { Donut, EquityChart } from "@/components/terminal/charts";
import { DealProgressBar } from "@/components/terminal/deal-progress-bar";
import { KillButton } from "@/components/terminal/killswitch";
import { KnowledgeView, trLower } from "@/components/terminal/knowledge";

describe("Pillar 5 — Terminal UI/UX, WCAG Accessibility & Safety Confirmation Suite", () => {
  const originalFetch = globalThis.fetch;

  beforeEach(() => {
    globalThis.fetch = vi.fn(async () => new Response(JSON.stringify({ engaged: false, reason: null }), { status: 200 })) as typeof fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    vi.restoreAllMocks();
  });

  describe("Charts & WCAG Accessibility (Donut + EquityChart)", () => {
    it("renders Donut empty state and accessible sr-only allocation summary", () => {
      const { rerender } = render(<Donut slices={[]} centerLabel="$0.00" centerSub="NAKİT" />);
      // Both visible legend item and sr-only summary announce empty allocation
      expect(screen.getAllByText(/Açık pozisyon yok/i).length).toBe(2);
      expect(screen.getByRole("img", { name: "Portföy dağılımı" })).toBeInTheDocument();

      rerender(
        <Donut
          slices={[
            { label: "BTCUSDT", value: 750, color: "#10b981" },
            { label: "ETHUSDT", value: 250, color: "#06b6d4" },
          ]}
          centerLabel="$1,000.00"
          centerSub="2 PARİTE"
        />,
      );
      expect(screen.getByText("BTCUSDT")).toBeInTheDocument();
      expect(screen.getByText("75.0%")).toBeInTheDocument();
      expect(screen.getByText("25.0%")).toBeInTheDocument();
    });

    it("supports flat-line zero-span equity, keyboard arrow navigation, and accessible data-table toggle", () => {
      // 1. Fewer than 2 points -> graceful empty message
      const { rerender } = render(<EquityChart points={[{ ts: 1_760_000_000_000, equity: 1000 }]} />);
      expect(screen.getByText(/en az 2 nokta gerekli/i)).toBeInTheDocument();

      // 2. Flat-line segment + gain -> zero-division guard + keyboard navigation
      const points = [
        { ts: 1_760_000_000_000, equity: 1000 },
        { ts: 1_760_003_600_000, equity: 1000 },
        { ts: 1_760_007_200_000, equity: 1050.5 },
      ];
      rerender(<EquityChart points={points} />);

      const chartRegion = screen.getByRole("img", { name: /Özkaynak eğrisi/i });
      expect(chartRegion).toHaveAttribute("tabindex", "0");

      // Keyboard navigation updates live ARIA status region: Home -> End -> Escape
      fireEvent.keyDown(chartRegion, { key: "Home" });
      expect(screen.getByRole("status").textContent).toContain("$1,000.00");

      fireEvent.keyDown(chartRegion, { key: "End" });
      expect(screen.getByRole("status").textContent).toContain("$1,050.50");

      fireEvent.keyDown(chartRegion, { key: "Escape" });
      expect(screen.getByRole("status").textContent).toContain("3 nokta");

      // Toggle accessible tabular view
      const tableBtn = screen.getByRole("button", { name: /Veri tablosu/i });
      expect(tableBtn).toHaveAttribute("aria-expanded", "false");
      fireEvent.click(tableBtn);
      expect(tableBtn).toHaveAttribute("aria-expanded", "true");
      expect(screen.getByRole("table")).toBeInTheDocument();
    });
  });

  describe("AuthDialog UX & Error States", () => {
    it("disables submit on blank input, displays server 401 message, and invokes onAuthenticated on valid login", async () => {
      const onAuthenticated = vi.fn();
      let shouldSucceed = false;

      globalThis.fetch = vi.fn(async () => {
        if (!shouldSucceed) {
          return new Response(JSON.stringify({ ok: false, error: "Geçersiz erişim parolası" }), { status: 401 });
        }
        return new Response(JSON.stringify({ ok: true }), { status: 200 });
      }) as typeof fetch;

      render(<AuthDialog onAuthenticated={onAuthenticated} />);

      const input = screen.getByPlaceholderText("Erişim Parolası");
      const submitBtn = screen.getByRole("button", { name: /Giriş Yap/i });
      expect(submitBtn).toBeDisabled();

      // Wrong password attempt
      fireEvent.change(input, { target: { value: "yanlis-sifre" } });
      expect(submitBtn).not.toBeDisabled();
      fireEvent.click(submitBtn);

      await waitFor(() => {
        expect(screen.getByText("Geçersiz erişim parolası")).toBeInTheDocument();
      });
      expect(onAuthenticated).not.toHaveBeenCalled();

      // Valid password attempt
      shouldSucceed = true;
      fireEvent.change(input, { target: { value: "dogru-sifre" } });
      fireEvent.click(submitBtn);

      await waitFor(() => {
        expect(onAuthenticated).toHaveBeenCalledTimes(1);
      });
    });
  });

  describe("KillButton: Two-Step Destructive Action Guard", () => {
    it("never fires killswitch POST on first click or wrong confirmation phrase; requires exact 'KILL' string", async () => {
      const postCalls: unknown[] = [];
      globalThis.fetch = vi.fn(async (input: RequestInfo | URL, init?: RequestInit) => {
        const url = String(input);
        if (init?.method === "POST" && url.includes("killswitch")) {
          postCalls.push(JSON.parse(String(init.body)));
          return new Response(JSON.stringify({ engaged: true, reason: "manual" }), { status: 200 });
        }
        return new Response(JSON.stringify({ engaged: false, reason: null }), { status: 200 });
      }) as typeof fetch;

      const onStopAll = vi.fn();
      render(<KillButton onStopAll={onStopAll} />);

      // Step 0 -> Open dialog
      fireEvent.click(screen.getByRole("button", { name: /^KILL$/i }));
      expect(await screen.findByText(/Global Killswitch — 1\/2/i)).toBeInTheDocument();
      expect(postCalls.length).toBe(0);

      // Step 1 -> Advance to Step 2
      fireEvent.click(screen.getByRole("button", { name: /Devam Et/i }));
      expect(await screen.findByText(/Global Killswitch — 2\/2/i)).toBeInTheDocument();

      const engageBtn = screen.getByRole("button", { name: /YÜRÜTMEYİ DURDUR/i });
      expect(engageBtn).toBeDisabled();

      // Lowercase "kill" must NOT unlock the button
      const confirmInput = screen.getByLabelText("Onay ifadesi");
      fireEvent.change(confirmInput, { target: { value: "kill" } });
      expect(engageBtn).toBeDisabled();
      expect(postCalls.length).toBe(0);

      // Exact uppercase "KILL" unlocks and engages
      fireEvent.change(confirmInput, { target: { value: "KILL" } });
      expect(engageBtn).not.toBeDisabled();
      fireEvent.click(engageBtn);

      await waitFor(() => {
        expect(postCalls.length).toBe(1);
        expect(onStopAll).toHaveBeenCalledTimes(1);
      });
    });
  });

  describe("Knowledge Center: Turkish Diacritic Fold (trLower) & Search UX", () => {
    it("folds Turkish dotted/dotless I (İ/I/ı/i) and diacritics (ş/ğ/ü/ö/ç) without breaking English terms like FUNDING", () => {
      expect(trLower("FUNDING")).toBe("funding");
      expect(trLower("İNDİKATÖR ÇAKIŞMASI")).toBe("indikator cakismasi");
      expect(trLower("Güvenlik Şalteri Öğesi")).toBe("guvenlik salteri ogesi");

      const onOpenAi = vi.fn();
      render(<KnowledgeView onOpenAi={onOpenAi} />);

      const searchInput = screen.getByRole("textbox", { name: "Bilgi merkezinde ara" });
      fireEvent.change(searchInput, { target: { value: "FUNDING" } });
      // Should still match lowercase "funding" entries rather than failing on Turkish 'fundıng'
      expect(screen.queryByText(/Eşleşen kayıt bulunamadı/i)).not.toBeInTheDocument();

      // Non-existent query shows honest empty state + AI assistant handoff
      fireEvent.change(searchInput, { target: { value: "xyznonexistenttoken999" } });
      const aiBtns = screen.getAllByRole("button", { name: /YZ Yardımcı/i });
      expect(aiBtns.length).toBeGreaterThan(0);
      fireEvent.click(aiBtns[0]);
      expect(onOpenAi).toHaveBeenCalledTimes(1);
    });
  });

  describe("DealProgressBar: Long/Short Deal Geometry, Liquidation Tiers & Bag-Holding State", () => {
    it("renders Idle/Closing states cleanly and computes Long TP progress + critical liquidation narrative without contradiction", () => {
      const { rerender } = render(<DealProgressBar phase="Idle" safetyTriggered={0} />);
      expect(screen.getByText(/Aktif deal yok/i)).toBeInTheDocument();

      rerender(<DealProgressBar phase="Closing" safetyTriggered={2} />);
      expect(screen.getByText(/Deal Kapanıyor/i)).toBeInTheDocument();

      // Active leveraged Long deal halfway from AVG (80,000) to TP (82,000) at mark 81,000, all 3 SOs exhausted
      rerender(
        <DealProgressBar
          phase="Open"
          safetyTriggered={3}
          maxSafetyOrders={3}
          avgPrice="80000"
          tpPrice="82000"
          currentPrice="81000"
          liqPrice="72000"
          marginMode={true}
          positionQty="0.5"
          unrealizedPnl="500"
        />,
      );
      expect(screen.getByText(/%50 tamamlandı/i)).toBeInTheDocument();
      expect(screen.getByText(/BAG HOLDING/i)).toBeInTheDocument();
      // Liquidation distance: (81000 - 72000) / 81000 = 11.11% (<15% -> KRİTİK LİKİDASYON RİSKİ)
      expect(screen.getAllByText(/KRİTİK LİKİDASYON RİSKİ/i).length).toBeGreaterThan(0);
      expect(screen.getByText(/kritik bölge/i)).toBeInTheDocument();
      // Must NEVER claim "güvenli mesafe bulunmaktadır" while in critical liquidation zone
      expect(screen.queryByText(/güvenli mesafe bulunmaktadır/i)).not.toBeInTheDocument();
    });

    it("renders DCA_SHORT deals with upward (+%) liquidation & safety-order geometry instead of Long-only downward labels", () => {
      render(
        <DealProgressBar
          phase="Open"
          isShort={true}
          marginMode={true}
          safetyTriggered={1}
          maxSafetyOrders={3}
          avgPrice="80000"
          tpPrice="78000"
          currentPrice="81000"
          liqPrice="90000"
          nextSafetyPrice="83000"
          nextSafetyLevel={2}
          ladder={[
            { level: 1, price: "81000", qty: "0.0012", notional: "100" },
            { level: 2, price: "83000", qty: "0.0018", notional: "150" },
          ]}
        />,
      );

      // Short liquidation triggers when market RISES (+11.1% to 90,000), never when it falls
      expect(screen.getByText(/Piyasa bu oranda yükseldiğinde marjin tükenir/i)).toBeInTheDocument();
      expect(screen.queryByText(/Piyasa bu oranda düştüğünde marjin tükenir/i)).not.toBeInTheDocument();

      // Short next SO triggers on upward price move (+2.47% to 83,000) via automatic sell
      expect(screen.getByText(/yükselirse otomatik satış \(SO\) yapacak/i)).toBeInTheDocument();
      expect(screen.queryByText(/düşerse otomatik alım yapacak/i)).not.toBeInTheDocument();
    });
  });
});
