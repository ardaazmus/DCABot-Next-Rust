"use client";

import { useEffect } from "react";
import { AlertTriangle, RefreshCw, Terminal } from "lucide-react";
import { Button } from "@/components/ui/button";

export default function ErrorBoundary({
  error,
  reset,
}: {
  error: Error & { digest?: string };
  reset: () => void;
}) {
  useEffect(() => {
    // Log unexpected frontend crash to console for operator inspection
    console.error("[DCABOT-UI-CRASH]", error);
  }, [error]);

  return (
    <div className="flex min-h-screen w-full flex-col items-center justify-center bg-[#0B0E14] p-4 text-slate-200">
      <div className="w-full max-w-lg rounded-xl border border-rose-500/40 bg-surface/90 p-6 shadow-2xl backdrop-blur">
        <div className="flex items-center gap-3 border-b border-rose-500/20 pb-4">
          <span className="flex h-10 w-10 items-center justify-center rounded-lg border border-rose-500/50 bg-rose-500/10 text-rose-400">
            <AlertTriangle className="h-5 w-5" />
          </span>
          <div>
            <h1 className="text-base font-extrabold tracking-wide text-rose-300">
              TERMINAL ÇALIŞMA ZAMANI HATASI
            </h1>
            <p className="text-xs text-slate-400">
              Arayüz bileşeninde beklenmedik bir istisna yakalandı
            </p>
          </div>
        </div>

        <div className="my-5 rounded-lg border border-border bg-base/80 p-3 font-mono text-xs text-rose-300/90 overflow-x-auto">
          <p className="font-bold text-slate-300">İstisna Mesajı:</p>
          <p className="mt-1 break-words">{error.message || "Bilinmeyen istemci hatası"}</p>
          {error.digest && (
            <p className="mt-2 text-[10px] text-slate-500">Digest: {error.digest}</p>
          )}
        </div>

        <div className="flex flex-col gap-2 sm:flex-row sm:justify-end">
          <Button
            variant="outline"
            className="border-border text-slate-300 hover:bg-slate-800"
            onClick={() => window.location.reload()}
          >
            <RefreshCw className="mr-2 h-4 w-4" />
            Sayfayı Yenile
          </Button>
          <Button
            className="bg-rose-600 font-bold text-white hover:bg-rose-500"
            onClick={() => reset()}
          >
            <Terminal className="mr-2 h-4 w-4" />
            Durumu Sıfırla ve Devam Et
          </Button>
        </div>
      </div>
    </div>
  );
}
