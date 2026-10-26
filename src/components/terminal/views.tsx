"use client";

/**
 * Code-split view registry — every heavy terminal view is a lazy chunk.
 * Keeps the initial route compile small (sandbox-safe) and improves
 * first-load performance; each view compiles on first navigation only.
 */
import dynamic from "next/dynamic";

const viewLoading = () => (
  <div className="flex min-h-[50vh] items-center justify-center" role="status" aria-label="Görünüm yükleniyor">
    <div className="h-8 w-8 animate-spin rounded-full border-2 border-border border-t-emerald-400" />
  </div>
);

export const DashboardView = dynamic(() => import("@/components/terminal/dashboard").then((m) => m.Dashboard), {
  ssr: false,
  loading: viewLoading,
});

export const FleetView = dynamic(() => import("@/components/terminal/fleet").then((m) => m.Fleet), {
  ssr: false,
  loading: viewLoading,
});

export const StudioView = dynamic(() => import("@/components/terminal/studio").then((m) => m.Studio), {
  ssr: false,
  loading: viewLoading,
});

export const WizardView = dynamic(() => import("@/components/terminal/wizard").then((m) => m.Wizard), {
  ssr: false,
  loading: viewLoading,
});

export const CalculatorLazy = dynamic(() => import("@/components/terminal/calculator").then((m) => m.CalculatorView), {
  ssr: false,
  loading: viewLoading,
});

export const BacktestLazy = dynamic(() => import("@/components/terminal/backtest-lab").then((m) => m.BacktestLab), {
  ssr: false,
  loading: viewLoading,
});

export const SettingsLazy = dynamic(() => import("@/components/terminal/settings").then((m) => m.SettingsView), {
  ssr: false,
  loading: viewLoading,
});

export const KnowledgeLazy = dynamic(() => import("@/components/terminal/knowledge").then((m) => m.KnowledgeView), {
  ssr: false,
  loading: viewLoading,
});
