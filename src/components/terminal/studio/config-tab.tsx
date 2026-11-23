"use client";

/* Faz C — Config sekmesi (Studio gövdesinden taşındı; yalnızca prop alır). */
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";
import { Loader2, Save } from "lucide-react";
import { apiPatchBot, getBotMeta, useBotDetail } from "@/hooks/use-engine";
import { Spinner } from "@/components/terminal/bits";
import { fmtNum, fracToNum } from "@/lib/format";
import { toast } from "@/hooks/use-toast";
import type { BotConfig, BotDetail } from "@/lib/engine-types";

/* ---------------- Config tab ---------------- */

export function ConfigTab({ botId, configHint }: { botId: string; configHint: BotConfig | null }) {
  const { bot, refetch } = useBotDetail(botId);
  if (!bot) return <Spinner className="h-6 w-6" />;
  return <ConfigTabForm key={botId} botId={botId} bot={bot} configHint={configHint} refetch={refetch} />;
}

function ConfigTabForm({
  botId,
  bot,
  configHint,
  refetch,
}: {
  botId: string;
  bot: BotDetail;
  configHint: BotConfig | null;
  refetch: () => void;
}) {
  const initialConfig = bot.config ?? configHint ?? null;
  const [initial] = useState(() => {
    const cfg = initialConfig;
    const tpPct = cfg ? (fracToNum(cfg.target_profit_pct) ?? 0.02) * 100 : 2;
    const slPct = cfg?.stop_loss_pct != null ? (fracToNum(cfg.stop_loss_pct) ?? 0) * 100 : null;
    return {
      tp: tpPct > 0 ? tpPct.toFixed(2) : "2.00",
      slEnabled: slPct != null || bot.stats.sl_price !== null,
      sl: slPct != null && slPct > 0 ? slPct.toFixed(2) : "",
      trailing: cfg ? cfg.trailing_enabled : bot.stats.trailing_state !== "Off",
    };
  });
  const [tp, setTp] = useState(initial.tp);
  const [slEnabled, setSlEnabled] = useState(initial.slEnabled);
  const [sl, setSl] = useState(initial.sl);
  const [trailing, setTrailing] = useState(initial.trailing);
  const [busy, setBusy] = useState(false);

  /* M20 hint source: wizard writes deviation via saveBotMeta (localStorage) at creation. */
  const [meta] = useState(() => getBotMeta(botId));

  const deviationPct = (() => {
    if (meta?.deviation_pct) return (fracToNum(meta.deviation_pct) ?? 0) * 100;
    if (configHint) return (fracToNum(configHint.deviation_pct) ?? 0) * 100;
    return null;
  })();
  const isGridBot = bot?.kind.includes("GRID") ?? false;
  const slNum = slEnabled && sl !== "" ? Number(sl) : null;
  const m20Conflict = deviationPct !== null && slNum !== null && slNum <= deviationPct;

  const save = async () => {
    setBusy(true);
    try {
      const patch: Record<string, unknown> = {
        target_profit_pct: String((Number(tp) || 0) / 100),
        trailing_enabled: trailing,
      };
      if (slEnabled && sl !== "") patch.stop_loss_pct = String(Number(sl) / 100);
      else patch.stop_loss_pct = null;
      /* 2xx only — any 4xx/5xx throws and the real engine message surfaces below. */
      await apiPatchBot(botId, patch);
      toast({ title: "Ayarlar kaydedildi", description: `PATCH /api/bots → 200 — motor yamayı uyguladı (TP ${tp}%${slEnabled ? `, SL ${sl}%` : ", SL kapalı"}${trailing ? ", trailing açık" : ""}).` });
      refetch();
    } catch (e) {
      toast({
        title: "PATCH reddedildi",
        description: e instanceof Error ? e.message : "Bilinmeyen hata",
        variant: "destructive",
      });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="grid max-w-2xl gap-4">
      {isGridBot && (
        <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-3 text-[11px] text-amber-300">
          Grid stratejisi TP/SL yaması desteklenmez — grid botlar mesh seviyeleriyle yönetilir. Bu sekmedeki kayıt motor tarafından 422 ile reddedilir.
        </div>
      )}
      <div className="space-y-1.5">
        <Label htmlFor="cfg-tp" className="text-xs text-slate-400">
          Hedef Kâr (TP) %
        </Label>
        <Input
          id="cfg-tp"
          type="number"
          step="0.1"
          min="0.1"
          value={tp}
          onChange={(e) => setTp(e.target.value)}
          className="num border-border-strong bg-surface text-slate-100"
        />
        <p className="text-[11px] text-slate-400">Net TP: ücretler düşülerek exact rational hesaplanır (M05).</p>
      </div>

      <div className="space-y-2 rounded-lg border border-border bg-surface p-3">
        <div className="flex items-center justify-between">
          <Label htmlFor="cfg-sl" className="text-xs text-slate-300">
            Stop-Loss aktif
          </Label>
          <Switch id="cfg-sl" checked={slEnabled} onCheckedChange={setSlEnabled} />
        </div>
        {slEnabled && (
          <>
            <Input
              type="number"
              step="0.1"
              min="0.1"
              placeholder="SL % (örn 5.0)"
              value={sl}
              onChange={(e) => setSl(e.target.value)}
              className="num border-border-strong bg-surface text-slate-100"
              aria-label="Stop-loss yüzdesi"
            />
            {m20Conflict ? (
              <div className="rounded-md border border-amber-500/40 bg-amber-500/10 p-2.5 text-[11px] text-amber-300">
                ⚠ M20 CONFLICT RISK: SL ({slNum}%) deviation&apos;dan ({deviationPct}%) sıkı. Motor kaydı M20 çakışmasıyla
                reddedebilir — SL&apos;i deviation üzerine çekin.
              </div>
            ) : (
              <p className="text-[11px] text-slate-400">
                SL deviation&apos;dan geniş olmalı (mevcut deviation: {deviationPct !== null ? `${deviationPct}%` : "bilinmiyor"}).
              </p>
            )}
          </>
        )}
      </div>

      <div className="space-y-3 rounded-lg border border-border bg-surface p-3">
        <div className="flex items-center justify-between">
          <div>
            <Label htmlFor="cfg-trail" className="text-xs text-slate-300">
              Trailing TP (M10)
            </Label>
            <p className="text-[11px] text-slate-400">ARMED tetikleme + geri çekilme sapması bot konfigürasyonundan gelir</p>
          </div>
          <Switch id="cfg-trail" checked={trailing} onCheckedChange={setTrailing} />
        </div>
        {trailing && (
          <p className="text-[11px] text-slate-400">
            Tetik/geri çekilme yüzdeleri canlı yamalamaz — bot oluşturulurken wizard&apos;da girilen değerlerle çalışır
            {(() => {
              const cfg = bot.config ?? configHint;
              if (!cfg) return "";
              return ` (mevcut: tetik ${fmtNum((fracToNum(cfg.trailing_trigger_pct) ?? 0) * 100, 1)}% · geri çekilme ${fmtNum((fracToNum(cfg.trailing_deviation_pct) ?? 0) * 100, 1)}%)`;
            })()}
            .
          </p>
        )}
      </div>

      <Button onClick={() => void save()} disabled={busy} className="min-h-11 bg-emerald-500/90 text-[#04150e] hover:bg-emerald-500">
        {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <Save className="h-4 w-4" aria-hidden />}
        Değişiklikleri Uygula (PATCH)
      </Button>
    </div>
  );
}
