"use client";

import { useEffect, useState } from "react";
import { Power, ShieldAlert } from "lucide-react";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  AlertDialogTrigger,
} from "@/components/ui/alert-dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { apiKillSet, apiKillState } from "@/hooks/use-engine";
import { toast } from "@/hooks/use-toast";
import { cn } from "@/lib/utils";

export function KillButton({ className, onStopAll }: { className?: string; onStopAll?: () => void }) {
  const [open, setOpen] = useState(false);
  const [step, setStep] = useState(1);
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [engaged, setEngaged] = useState(false);
  const [reason, setReason] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const syncState = () => {
      apiKillState()
        .then((s) => {
          if (!alive) return;
          setEngaged(s.engaged);
          setReason(s.reason);
        })
        .catch(() => {
          // motor kapalıysa sessiz — offline banner zaten var
        });
    };
    syncState();
    const t = setInterval(syncState, 10000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  }, []);

  const reset = () => {
    setStep(1);
    setPhrase("");
    setBusy(false);
  };

  const executeEngage = async () => {
    setBusy(true);
    try {
      const s = await apiKillSet(true);
      setEngaged(s.engaged);
      setReason(s.reason);
      toast({
        title: "KILLSWITCH DEVREDE",
        description: "Yürütme hattı durdu — yeni emir ve TV sinyalleri reddedilir, açıklar korunur.",
      });
      setOpen(false);
      onStopAll?.();
    } catch (e) {
      toast({
        title: "Killswitch hatası",
        description: e instanceof Error ? e.message : "Bilinmeyen hata",
        variant: "destructive",
      });
    } finally {
      setBusy(false);
      reset();
    }
  };

  const executeRelease = async () => {
    setBusy(true);
    try {
      const s = await apiKillSet(false);
      setEngaged(s.engaged);
      toast({
        title: "Kill-switch bırakıldı",
        description: "Yürütme hattı açık — Ayarlar'dan selfcheck yenileyin (v3 UI kuralı).",
      });
      setOpen(false);
      onStopAll?.();
    } catch (e) {
      toast({
        title: "Bırakma hatası",
        description: e instanceof Error ? e.message : "Bilinmeyen hata",
        variant: "destructive",
      });
    } finally {
      setBusy(false);
      reset();
    }
  };

  return (
    <AlertDialog
      open={open}
      onOpenChange={(o) => {
        setOpen(o);
        if (!o) reset();
      }}
    >
      <AlertDialogTrigger asChild>
        <Button
          variant="outline"
          size="sm"
          className={cn(
            "min-h-11 border-rose-500/50 bg-rose-500/10 text-rose-400 hover:bg-rose-500/20 hover:text-rose-300 md:min-h-0",
            engaged && "border-rose-500 bg-rose-500/25 text-rose-200",
            className,
          )}
        >
          <Power className="h-4 w-4" aria-hidden />
          {engaged ? "KILL DEVREDE" : "KILL"}
        </Button>
      </AlertDialogTrigger>
      <AlertDialogContent className="border-rose-500/30 bg-[#151a24]">
        {!engaged ? (
          step === 1 ? (
            <>
              <AlertDialogHeader>
                <AlertDialogTitle className="flex items-center gap-2 text-rose-400">
                  <ShieldAlert className="h-5 w-5" aria-hidden />
                  Global Killswitch — 1/2
                </AlertDialogTitle>
                <AlertDialogDescription className="text-slate-400">
                  Bu işlem TÜM yürütme hattını durdurur (LIVE emirler + TV sinyalleri reddedilir; PAPER
                  simülasyonu sürer). Açık pozisyonlar kapatılmaz, botlar STOPPED&apos;e alınmaz — yalnızca yeni
                  emir üretimi durur. Devam etmek istediğinizi onaylayın.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel className="border-border-strong bg-transparent text-slate-300 hover:bg-secondary hover:text-slate-100">
                  Vazgeç
                </AlertDialogCancel>
                <AlertDialogAction
                  className="bg-amber-500/90 text-[#231400] hover:bg-amber-500"
                  onClick={(e) => {
                    e.preventDefault();
                    setStep(2);
                  }}
                >
                  Devam Et
                </AlertDialogAction>
              </AlertDialogFooter>
            </>
          ) : (
            <>
              <AlertDialogHeader>
                <AlertDialogTitle className="flex items-center gap-2 text-rose-400">
                  <ShieldAlert className="h-5 w-5" aria-hidden />
                  Global Killswitch — 2/2
                </AlertDialogTitle>
                <AlertDialogDescription className="text-slate-400">
                  Son onay: kilidi açmak için aşağıdaki alana <span className="num font-bold text-rose-400">KILL</span> yazın.
                </AlertDialogDescription>
              </AlertDialogHeader>
              <Input
                autoFocus
                value={phrase}
                onChange={(e) => setPhrase(e.target.value)}
                placeholder="KILL yazın"
                className="border-border-strong bg-surface font-mono text-slate-100 placeholder:text-slate-500"
                aria-label="Onay ifadesi"
              />
              <AlertDialogFooter>
                <AlertDialogCancel className="border-border-strong bg-transparent text-slate-300 hover:bg-secondary hover:text-slate-100">
                  Vazgeç
                </AlertDialogCancel>
                <Button
                  disabled={phrase !== "KILL" || busy}
                  className="bg-rose-500 text-white hover:bg-rose-600 disabled:opacity-40"
                  onClick={() => void executeEngage()}
                >
                  <Power className="h-4 w-4" aria-hidden />
                  {busy ? "Durduruluyor…" : "YÜRÜTMEYİ DURDUR"}
                </Button>
              </AlertDialogFooter>
            </>
          )
        ) : (
          <>
            <AlertDialogHeader>
              <AlertDialogTitle className="flex items-center gap-2 text-rose-400">
                <ShieldAlert className="h-5 w-5" aria-hidden />
                Killswitch Devrede
              </AlertDialogTitle>
              <AlertDialogDescription className="text-slate-400">
                Yürütme hattı durmuş durumda{reason ? `: ${reason}` : ""}. Bırakmak yürütmeyi yeniden açar —
                sonrasında Ayarlar&apos;dan selfcheck yenileyin.
              </AlertDialogDescription>
            </AlertDialogHeader>
            <AlertDialogFooter>
              <AlertDialogCancel className="border-border-strong bg-transparent text-slate-300 hover:bg-secondary hover:text-slate-100">
                Vazgeç
              </AlertDialogCancel>
              <Button
                disabled={busy}
                className="bg-emerald-600 text-white hover:bg-emerald-500 disabled:opacity-40"
                onClick={() => void executeRelease()}
              >
                <Power className="h-4 w-4" aria-hidden />
                {busy ? "Bırakılıyor…" : "BIRAK — HATTI AÇ"}
              </Button>
            </AlertDialogFooter>
          </>
        )}
      </AlertDialogContent>
    </AlertDialog>
  );
}
