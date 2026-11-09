"use client";

import { useState } from "react";
import { Lock, KeyRound, ArrowRight, AlertCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

export function AuthDialog({ onAuthenticated }: { onAuthenticated: () => void }) {
  const [password, setPassword] = useState("");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!password.trim()) return;

    setLoading(true);
    setError(null);
    try {
      const res = await fetch("/api/auth/login", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ password }),
      });
      const data = await res.json();
      if (!res.ok || !data.ok) {
        setError(data.error || "Geçersiz erişim parolası");
      } else {
        onAuthenticated();
      }
    } catch {
      setError("Sunucuya bağlanılamadı");
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/85 backdrop-blur-md p-4">
      <div className="w-full max-w-sm rounded-xl border border-slate-700 bg-surface/95 p-6 shadow-2xl">
        <div className="text-center">
          <div className="mx-auto mb-3 flex h-12 w-12 items-center justify-center rounded-xl border border-emerald-500/40 bg-emerald-500/10 text-emerald-400 shadow-md shadow-emerald-950/30">
            <Lock className="h-6 w-6" />
          </div>
          <h2 className="text-base font-extrabold tracking-wide text-slate-100">
            TERMINAL ERİŞİM KİLİDİ
          </h2>
          <p className="mt-1 text-xs text-slate-400">
            DCABOT kantitatif kontrol paneline erişmek için parolanızı girin
          </p>
        </div>

        <form onSubmit={handleSubmit} className="mt-5 space-y-3">
          <div className="relative">
            <KeyRound className="absolute left-3 top-2.5 h-4 w-4 text-slate-400" />
            <Input
              type="password"
              placeholder="Erişim Parolası"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              className="pl-9 bg-base border-slate-700 text-slate-100 placeholder:text-slate-500"
              autoFocus
            />
          </div>

          {error && (
            <div className="flex items-center gap-2 rounded border border-rose-500/40 bg-rose-500/10 p-2 text-xs text-rose-300">
              <AlertCircle className="h-4 w-4 shrink-0 text-rose-400" />
              <span>{error}</span>
            </div>
          )}

          <Button
            type="submit"
            disabled={loading || !password.trim()}
            className="w-full bg-emerald-600 font-bold text-white hover:bg-emerald-500"
          >
            {loading ? "Doğrulanıyor…" : "Giriş Yap"}
            <ArrowRight className="ml-2 h-4 w-4" />
          </Button>
        </form>

        <p className="mt-4 text-center text-[10px] text-slate-500">
          Savunma derinliği: Oturum açılmadıkça motor API çağrıları yapılamaz.
        </p>
      </div>
    </div>
  );
}
