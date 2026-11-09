import { Terminal } from "lucide-react";

export default function Loading() {
  return (
    <div className="flex min-h-screen w-full flex-col items-center justify-center bg-[#0B0E14] text-slate-200">
      <div className="flex flex-col items-center gap-4">
        <div className="relative flex h-16 w-16 items-center justify-center rounded-2xl border border-emerald-500/30 bg-emerald-500/10 shadow-lg shadow-emerald-950/40">
          <Terminal className="h-8 w-8 text-emerald-400 animate-pulse" />
          <span className="absolute -top-1 -right-1 flex h-3 w-3">
            <span className="absolute inline-flex h-full w-full animate-ping rounded-full bg-emerald-400 opacity-75" />
            <span className="relative inline-flex h-3 w-3 rounded-full bg-emerald-500" />
          </span>
        </div>
        <div className="text-center">
          <p className="text-sm font-extrabold tracking-widest text-slate-100">DCABOT TERMINAL</p>
          <p className="mt-1 text-xs text-slate-400">Kantitatif motor bağlantısı kuruluyor…</p>
        </div>
      </div>
    </div>
  );
}
