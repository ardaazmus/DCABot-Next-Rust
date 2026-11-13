import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { EngineError, engineFetch, engineJson, engineUrl } from "@/hooks/engine-client";
import { useDebouncedValue, useEngineStream, useEventStream, usePoll } from "@/hooks/engine-stream";

class MockEventSource {
  static instances: MockEventSource[] = [];
  url: string;
  closed = false;
  onopen: (() => void) | null = null;
  onerror: (() => void) | null = null;
  private listeners = new Map<string, Set<(e: MessageEvent<string>) => void>>();

  constructor(url: string) {
    this.url = url;
    MockEventSource.instances.push(this);
  }

  addEventListener(type: string, cb: EventListener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type)!.add(cb as unknown as (e: MessageEvent<string>) => void);
  }

  emit(type: string, data: string) {
    const set = this.listeners.get(type);
    if (!set) return;
    const evt = { data } as MessageEvent<string>;
    for (const cb of set) cb(evt);
  }

  close() {
    this.closed = true;
  }
}

describe("Pillar 4 — Deterministic Engine Fault Injection, Single-Flight Polling & SSE Stress Suite", () => {
  const originalFetch = globalThis.fetch;
  const originalEventSource = globalThis.EventSource;

  beforeEach(() => {
    MockEventSource.instances = [];
    globalThis.EventSource = MockEventSource as unknown as typeof EventSource;
    useEngineStream.setState({ events: [], connected: false, seq: 0, dataVer: 0 });
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
    globalThis.EventSource = originalEventSource;
    vi.useRealTimers();
    vi.restoreAllMocks();
  });

  describe("engineFetch: Network Chaos, HTTP 401/422/429/502 & Non-JSON Upstream Faults", () => {
    it("prefixes relative paths with /api/engine and Never exposes absolute engine URLs", async () => {
      let calledUrl = "";
      globalThis.fetch = vi.fn(async (input: RequestInfo | URL) => {
        calledUrl = String(input);
        return new Response(JSON.stringify({ ok: true }), { status: 200 });
      }) as typeof fetch;

      const res = await engineFetch<{ ok: boolean }>("api/summary", engineJson("POST", { ping: 1 }));
      expect(res.ok).toBe(true);
      expect(calledUrl).toBe("/api/engine/api/summary");
      expect(engineUrl("/api/stream")).toBe("/api/stream");
    });

    it("maps network drops to status=0 EngineError, 401 to localized auth error, and truncates huge HTML 502 pages", async () => {
      // 1. Network disconnect / DNS drop
      globalThis.fetch = vi.fn(async () => {
        throw new TypeError("Failed to fetch");
      }) as typeof fetch;
      await expect(engineFetch("/api/bots")).rejects.toMatchObject({
        name: "EngineError",
        status: 0,
      });

      // 2. HTTP 401 Unauthorized
      globalThis.fetch = vi.fn(async () => {
        return new Response(JSON.stringify({ error: "unauthorized" }), { status: 401 });
      }) as typeof fetch;
      await expect(engineFetch("/api/bots")).rejects.toMatchObject({
        name: "EngineError",
        status: 401,
      });

      // 3. Reverse proxy / Cloudflare 502 HTML page (>2KB non-JSON body)
      const hugeHtml = "<html><body>502 Bad Gateway " + "X".repeat(2500) + "</body></html>";
      globalThis.fetch = vi.fn(async () => new Response(hugeHtml, { status: 502 })) as typeof fetch;
      try {
        await engineFetch("/api/summary");
        expect.unreachable("should throw EngineError");
      } catch (e) {
        expect(e).toBeInstanceOf(EngineError);
        const err = e as EngineError;
        expect(err.status).toBe(502);
        const errBody = err.body as { error: string };
        expect(errBody.error.length).toBeLessThanOrEqual(400);
      }
    });
  });

  describe("usePoll: Single-Flight Concurrency Guard & Stale-Data Preservation Under Outage", () => {
    it("skips overlapping interval ticks while a slow request is in-flight and preserves stale data + lastUpdatedAt on failure", async () => {
      vi.useFakeTimers();
      vi.setSystemTime(new Date("2026-11-13T12:00:00Z"));

      let callCount = 0;
      let resolveSlow!: (res: Response) => void;

      globalThis.fetch = vi.fn(() => {
        callCount += 1;
        if (callCount === 1) {
          return new Promise<Response>((resolve) => {
            resolveSlow = resolve;
          });
        }
        // Second call simulates transient HTTP 429 rate-limit error
        return Promise.resolve(new Response(JSON.stringify({ error: "Too Many Requests" }), { status: 429 }));
      }) as typeof fetch;

      const { result, unmount } = renderHook(() => usePoll<{ nav: string }>("/api/summary", 1000));

      // Call #1 started immediately; advance 3 full intervals while Call #1 is still in-flight!
      expect(callCount).toBe(1);
      await act(async () => {
        await vi.advanceTimersByTimeAsync(3000);
      });
      // Single-flight busy lock MUST prevent pile-up: still only 1 fetch!
      expect(callCount).toBe(1);

      // Now complete Call #1 at t = +3000ms
      await act(async () => {
        resolveSlow(new Response(JSON.stringify({ nav: "10500.25" }), { status: 200 }));
        await Promise.resolve();
      });

      expect(result.current.data).toEqual({ nav: "10500.25" });
      expect(result.current.error).toBeNull();
      const firstSuccessTs = result.current.lastUpdatedAt;
      expect(firstSuccessTs).toBe(new Date("2026-11-13T12:00:03Z").getTime());

      // Next interval fires Call #2 which fails with 429 at t = +4000ms
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1000);
      });
      expect(callCount).toBe(2);
      // Stale-data contract: previous NAV and original lastUpdatedAt MUST remain intact!
      expect(result.current.data).toEqual({ nav: "10500.25" });
      expect(result.current.error).toContain("Too Many Requests");
      expect(result.current.lastUpdatedAt).toBe(firstSuccessTs);

      unmount();
    });
  });

  describe("SSE Stream Ring Buffer, High-Frequency Event Storm & Ref-Counted Lifecycle", () => {
    it("caps ring buffer at 200 under a 500-event storm, ignores PING for dataVer, handles malformed JSON, and closes on last unmount", () => {
      const hookA = renderHook(() => useEventStream());
      const hookB = renderHook(() => useEventStream());

      // Shared singleton EventSource across multiple mounted views
      expect(MockEventSource.instances.length).toBe(1);
      const es = MockEventSource.instances[0];
      expect(es.url).toBe("/api/stream");

      act(() => {
        es.onopen?.();
      });
      expect(hookA.result.current.connected).toBe(true);

      // 1. PING must mark connected without pushing to events or bumping dataVer
      act(() => {
        es.emit("PING", "{}");
      });
      expect(hookA.result.current.events.length).toBe(0);
      expect(hookA.result.current.dataVer).toBe(0);

      // 2. Malformed JSON frame degrades gracefully to { raw } without throwing
      act(() => {
        es.emit("RISK", "<<<corrupted-frame>>>");
      });
      expect(hookA.result.current.events[0].type).toBe("RISK");
      expect(hookA.result.current.events[0].payload).toEqual({ raw: "<<<corrupted-frame>>>" });
      // RISK is informational stream event (not in DATA_EVENT_TYPES), whereas LIQUIDATION / FUNDING / FILL bump dataVer
      const verBefore = hookA.result.current.dataVer;

      act(() => {
        es.emit("FUNDING", JSON.stringify({ botId: "b-1", expense: "0.42" }));
        es.emit("LIQUIDATION", JSON.stringify({ botId: "b-1", liqPrice: "71200" }));
        es.emit("WEBHOOK_START", JSON.stringify({ botId: "b-1" }));
      });
      expect(hookA.result.current.dataVer).toBe(verBefore + 3);

      // 3. Inject 500 rapid FILL events -> ring buffer strictly capped at 200, newest first
      act(() => {
        for (let i = 1; i <= 500; i++) {
          es.emit("FILL", JSON.stringify({ seqNo: i, side: "Buy" }));
        }
      });
      expect(hookA.result.current.events.length).toBe(200);
      expect(hookA.result.current.events[0].payload).toEqual({ seqNo: 500, side: "Buy" });
      expect(hookA.result.current.events[199].payload).toEqual({ seqNo: 301, side: "Buy" });

      // 4. Connection drop sets connected=false
      act(() => {
        es.onerror?.();
      });
      expect(hookA.result.current.connected).toBe(false);

      // 5. Unmounting first consumer keeps EventSource open; unmounting last consumer closes it
      hookA.unmount();
      expect(es.closed).toBe(false);
      hookB.unmount();
      expect(es.closed).toBe(true);
    });

    it("useDebouncedValue coalesces rapid SSE bursts into a single state update", async () => {
      vi.useFakeTimers();
      const { result, rerender } = renderHook(({ v }) => useDebouncedValue(v, 300), {
        initialProps: { v: 0 },
      });

      rerender({ v: 1 });
      rerender({ v: 2 });
      rerender({ v: 3 });
      expect(result.current).toBe(0);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(299);
      });
      expect(result.current).toBe(0);

      await act(async () => {
        await vi.advanceTimersByTimeAsync(2);
      });
      expect(result.current).toBe(3);
    });
  });
});
