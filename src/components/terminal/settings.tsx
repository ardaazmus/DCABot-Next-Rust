"use client";

import { useCallback, useEffect, useState } from "react";
import {
  BadgeCheck,
  Ban,
  BookOpen,
  CheckCircle2,
  Coins,
  Copy,
  Cpu,
  Download,
  Eye,
  EyeOff,
  KeyRound,
  ListChecks,
  Loader2,
  ShieldAlert,
  ShieldCheck,
  Sparkles,
  Trash2,
  Webhook,
  Wrench,
  XCircle,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
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
import { EmptyState, Panel, Spinner, VenueChip } from "@/components/terminal/bits";
import { apiAddVaultKey, apiAiProviders, apiBlacklist, apiBlacklistAdd, apiBlacklistRemove, apiDeleteVaultKey, apiMaintenance, apiMaintenanceSet, apiSelfCheck, apiVaultKeys, useAiConfig, useBnb, useEngineHealth } from "@/hooks/use-engine";
import { toast } from "@/hooks/use-toast";
import { fmtTs } from "@/lib/format";
import { cn } from "@/lib/utils";
import type { AiProvider, BlacklistRow, BnbStatus, FaqEntry, GlossaryEntry, MaintenanceStatus, SelfCheckResult, VaultKey } from "@/lib/engine-types";
import glossaryJson from "@/data/glossary.json";
import faqJson from "@/data/faq.json";

const GLOSSARY = glossaryJson as GlossaryEntry[];
const FAQ = faqJson as FaqEntry[];

const GATES_INFO: [string, string][] = [
  ["G1", "OHLC Monotonluk — H>L, O/C aralık içinde; bozuk veri varsa bot HALT'e geçer (fail-closed)."],
  ["G2", "Fee Guard / Fee-Breaker — grid spread'i maker ücretini karşılamıyorsa yapı reddedilir."],
  ["G3", "Slippage gerçekçiliği — fill fiyatı emir fiyatından kötüleşemez, sapma modeli denetlenir."],
  ["G4", "No-lookahead — backtest yalnızca bar kapanışına kadar bilgi kullanabilir."],
  ["G5", "Realism tagging — her fill MARKETABLE (taker) / RESTING (maker) / GAP-BETTER olarak etiketlenir."],
  ["G6", "Capital guard — bakiye yetersizse emir SKIP/WAIT; negatif nakit oluşamaz."],
  ["G7", "Seans takvimi (M21) — borsa kapalıyken emir üretimi durur; takvim verisi yoksa fail-closed."],
  ["G8", "Opening salvo / gap konsolidasyonu — bar açılış boşlukları tek fill'e konsolide edilir."],
];

export function SettingsView() {
  const { health } = useEngineHealth(15000);
  const [keys, setKeys] = useState<VaultKey[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [venue, setVenue] = useState("BYBIT");
  const [label, setLabel] = useState("");
  const [apiKey, setApiKey] = useState("");
  const [apiSecret, setApiSecret] = useState("");
  /* P8 — secret görünürlüğü + kopyalama onayı (yanlış anahtar hatalarını kapatır). */
  const [showSecret, setShowSecret] = useState(false);
  const [secretMsg, setSecretMsg] = useState<string | null>(null);
  /* P4-3: OKX passphrase (yalnız OKX — diğer venue'lerde gizli). */
  const [apiPass, setApiPass] = useState("");
  const [busy, setBusy] = useState(false);
  const [gateError, setGateError] = useState<string | null>(null);
  /* P2-3 self-check */
  const [sc, setSc] = useState<SelfCheckResult | null>(null);
  const [scBusy, setScBusy] = useState(false);
  const [scError, setScError] = useState<string | null>(null);
  const runSelfCheck = async () => {
    setScBusy(true);
    setScError(null);
    try {
      setSc(await apiSelfCheck("BTCUSDT"));
    } catch (e) {
      setScError(e instanceof Error ? e.message : "Self-check çalışmadı");
    } finally {
      setScBusy(false);
    }
  };
  /* Faz 25 (UIX-23): ortak YZ sağlayıcı config — tek sahibi bu panel.
     provider/model/baseUrl localStorage'da; anahtar sessionStorage'da. */
  const ai = useAiConfig();
  const [aiProviders, setAiProviders] = useState<AiProvider[]>([]);
  useEffect(() => {
    let alive = true;
    apiAiProviders()
      .then((r) => {
        if (alive) setAiProviders(r.providers);
      })
      .catch(() => {
        if (alive) setAiProviders([]);
      });
    return () => {
      alive = false;
    };
  }, []);
  const aiCfgNote = (() => {
    const info = aiProviders.find((p) => p.id === ai.provider) ?? null;
    if (!info) return null;
    if (info.id === "builtin") return info.note || "Yerleşik sağlayıcı bu kurulumda paketlenmemiş — dış sağlayıcı seçin.";
    return info.note;
  })();
  /* P3-1 kara liste */
  const [bl, setBl] = useState<BlacklistRow[]>([]);  const [blBusy, setBlBusy] = useState(false);
  const [blError, setBlError] = useState<string | null>(null);
  const [blPair, setBlPair] = useState("");
  const [blReason, setBlReason] = useState("");
  const loadBl = useCallback(async () => {
    try {
      const r = await apiBlacklist();
      setBl(r.blacklist ?? []);
      setBlError(null);
    } catch (e) {
      setBlError(e instanceof Error ? e.message : "Kara liste okunamadı");
    }
  }, []);
  useEffect(() => {
    void loadBl();
  }, [loadBl]);
  const addBl = async () => {
    if (!blPair.trim()) return;
    setBlBusy(true);
    setBlError(null);
    try {
      await apiBlacklistAdd(blPair, blReason.trim() || undefined);
      toast({ title: "Kara listeye eklendi", description: `${blPair.trim().toUpperCase()} — bu sembole yeni bot kurulamaz.` });
      setBlPair("");
      setBlReason("");
      await loadBl();
    } catch (e) {
      setBlError(e instanceof Error ? e.message : "Eklenemedi");
    } finally {
      setBlBusy(false);
    }
  };
  /* P3-5 bakım bayrağı */
  const [maint, setMaint] = useState<MaintenanceStatus | null>(null);
  const [maintBusy, setMaintBusy] = useState(false);
  const [maintNote, setMaintNote] = useState("");
  const loadMaint = useCallback(async () => {
    try {
      const r = await apiMaintenance();
      setMaint(r);
      setMaintNote(r.note ?? "");
    } catch {
      /* motor kapalıysa sessiz */
    }
  }, []);
  useEffect(() => {
    void loadMaint();
  }, [loadMaint]);
  const setQuiet = async (quiet: boolean) => {
    setMaintBusy(true);
    try {
      const r = await apiMaintenanceSet(quiet, maintNote.trim() || undefined);
      toast({ title: quiet ? "Bakım sessizliği açıldı" : "Bakım sessizliği kapatıldı", description: r.note || undefined });
      await loadMaint();
    } finally {
      setMaintBusy(false);
    }
  };
  const removeBl = async (pair: string) => {
    setBlBusy(true);
    try {
      await apiBlacklistRemove(pair);
      toast({ title: "Kara listeden çıkarıldı", description: pair });
      await loadBl();
    } catch (e) {
      setBlError(e instanceof Error ? e.message : "Kaldırılamadı");
    } finally {
      setBlBusy(false);
    }
  };

  const load = useCallback(async () => {
    try {
      const r = await apiVaultKeys();
      setKeys(r.keys ?? []);
      setLoadError(null);
    } catch (e) {
      setKeys([]);
      setLoadError(e instanceof Error ? e.message : "Kasa okunamadı");
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const addKey = async () => {
    setBusy(true);
    setGateError(null);
    try {
      const r = await apiAddVaultKey({ venue, label: label.trim() || "default", api_key: apiKey.trim(), api_secret: apiSecret.trim(), passphrase: venue === "OKX" ? apiPass : undefined });
      toast({ title: "Anahtar kasaya eklendi", description: `${r.venue} · ${r.maskedKey} — zero-withdrawal gate doğrulandı.` });
      setApiKey("");
      setApiSecret("");
      setApiPass("");
      setLabel("");
      void load();
    } catch (e) {
      const msg = e instanceof Error ? e.message : "Bilinmeyen hata";
      setGateError(msg);
      toast({ title: "Vault reddedildi", description: msg.slice(0, 140), variant: "destructive" });
    } finally {
      setBusy(false);
    }
  };

  const removeKey = async (id: string) => {
    try {
      await apiDeleteVaultKey(id);
      toast({ title: "Anahtar silindi" });
      void load();
    } catch (e) {
      toast({ title: "Silme hatası", description: e instanceof Error ? e.message : "", variant: "destructive" });
    }
  };

  return (
    <div className="grid gap-4 xl:grid-cols-2">
      {/* API Key Vault */}
      <div className="space-y-4">
        <Panel title="API Key Vault (AES-256-GCM)" icon={<KeyRound className="h-4 w-4 text-slate-500" aria-hidden />}>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label className="text-xs text-slate-400">Borsa</Label>
              <Select value={venue} onValueChange={setVenue}>
                <SelectTrigger className="min-h-11 w-full border-border-strong bg-surface text-slate-100">
                  <SelectValue />
                </SelectTrigger>                  <SelectContent className="border-border-strong bg-[#151a24]">
                    <SelectItem value="BYBIT" className="font-mono">BYBIT</SelectItem>
                    <SelectItem value="BINANCE" className="font-mono">BINANCE</SelectItem>
                    <SelectItem value="OKX" className="font-mono">OKX</SelectItem>
                  </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="vk-label" className="text-xs text-slate-400">
                Etiket
              </Label>
              <Input id="vk-label" value={label} onChange={(e) => setLabel(e.target.value)} placeholder="main-trading" className="min-h-11 border-border-strong bg-surface text-slate-100 placeholder:text-slate-500" />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="vk-key" className="text-xs text-slate-400">
                API Key
              </Label>
              <Input id="vk-key" value={apiKey} onChange={(e) => setApiKey(e.target.value)} className="num min-h-11 border-border-strong bg-surface font-mono text-slate-100" placeholder="XXXXXXXX" />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="vk-secret" className="text-xs text-slate-400">
                API Secret
              </Label>
              <div className="flex gap-2">
                <Input id="vk-secret" type={showSecret ? "text" : "password"} value={apiSecret} onChange={(e) => setApiSecret(e.target.value)} className="num min-h-11 border-border-strong bg-surface font-mono text-slate-100" placeholder="••••••••" autoComplete="off" />
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setShowSecret((v) => !v)}
                  aria-pressed={showSecret}
                  aria-label={showSecret ? "Secret'i gizle" : "Secret'i göster"}
                  className="min-h-11 shrink-0 border-border-strong px-3 text-slate-300 hover:text-slate-100"
                >
                  {showSecret ? <EyeOff className="h-4 w-4" aria-hidden /> : <Eye className="h-4 w-4" aria-hidden />}
                </Button>
                <Button
                  type="button"
                  variant="outline"
                  disabled={!apiSecret}
                  onClick={() => {
                    void (async () => {
                      try {
                        await navigator.clipboard.writeText(apiSecret);
                        setSecretMsg("Kopyalandı — kimseyle paylaşmayın.");
                      } catch {
                        setSecretMsg("Kopyalanamadı — metni elle seçin.");
                      }
                    })();
                  }}
                  aria-label="Secret'i panoya kopyala"
                  className="min-h-11 shrink-0 border-border-strong px-3 text-slate-300 hover:text-slate-100"
                >
                  <Copy className="h-4 w-4" aria-hidden />
                </Button>
              </div>
              <p role="status" className="min-h-4 text-[11px] text-emerald-400">{secretMsg ?? ""}</p>
            </div>
            {venue === "OKX" && (
              <div className="space-y-1.5 sm:col-span-2">
                <Label htmlFor="vk-pass" className="text-xs text-slate-400">
                  Passphrase <span className="font-bold text-amber-300">(OKX zorunlu)</span>
                </Label>
                <Input id="vk-pass" type="password" value={apiPass} onChange={(e) => setApiPass(e.target.value)} className="num min-h-11 border-border-strong bg-surface font-mono text-slate-100" placeholder="API oluştururken verilen parola" autoComplete="off" />
                <p className="text-[10px] text-slate-400">AES-256-GCM ile şifreli saklanır, asla görüntülenmez; yalnız imza anında çözülür.</p>
              </div>
            )}
          </div>
          <p className="mt-2 text-[11px] text-slate-400">
            ZERO-WITHDRAWAL GATE: kaydedilen anahtar çekim izni içermemelidir. Withdraw yetkisi tespit edilirse motor 422
            ile reddeder — bu kasaya hiçbir anahtar çekim yetkisiyle giremez.
          </p>
          <Button onClick={() => void addKey()} disabled={busy || !apiKey.trim() || !apiSecret.trim() || (venue === "OKX" && !apiPass)} className="mt-3 min-h-11 w-full bg-emerald-500/90 text-[#04150e] hover:bg-emerald-500 sm:w-auto sm:px-6">
            {busy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <ShieldCheck className="h-4 w-4" aria-hidden />}
            Kasaya Ekle & Doğrula
          </Button>

          {gateError && (
            <div className="mt-3 flex items-start gap-3 rounded-lg border border-rose-500/60 bg-rose-500/10 p-4 glow-rose">
              <ShieldAlert className="mt-0.5 h-6 w-6 shrink-0 text-rose-400" aria-hidden />
              <div className="min-w-0">
                <div className="text-sm font-bold text-rose-400">ZERO-WITHDRAWAL GATE VIOLATION</div>
                <pre className="thin-scroll mt-1 max-h-32 overflow-auto whitespace-pre-wrap font-mono text-xs text-rose-300">{gateError}</pre>
              </div>
            </div>
          )}
        </Panel>

        <Panel title={`Kayıtlı Anahtarlar (${keys.length})`} icon={<BadgeCheck className="h-4 w-4 text-slate-500" aria-hidden />} bodyClassName="p-3">
          {loading ? (
            <div className="flex justify-center py-6">
              <Spinner className="h-6 w-6" />
            </div>
          ) : (
            <>
              {loadError && (
                <div role="alert" className="mb-2 flex items-center gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 p-2.5 text-[11px] text-amber-300">
                  <ShieldAlert className="h-3.5 w-3.5 shrink-0" aria-hidden />
                  Kasa okunamadı: {loadError}
                </div>
              )}
              {keys.length === 0 ? (
                !loadError && (
                  <EmptyState
                    icon={KeyRound}
                    title="Kasa boş"
                    hint="LIVE moda geçmeden önce doğrulanmış bir API anahtarı ekleyin — motor LIVE emirler için kasadaki zero-withdrawal anahtarını zorunlu kılar."
                  />
                )
              ) : (
                <ul className="thin-scroll max-h-72 space-y-2 overflow-y-auto pr-1">
                  {keys.map((k) => (
                    <li key={k.id} className="flex flex-wrap items-center gap-2 rounded-lg border border-border bg-surface p-3">
                      <VenueChip venue={k.venue} />
                      <span className="text-sm font-semibold text-slate-100">{k.label}</span>
                      <span className="num rounded bg-secondary px-2 py-0.5 font-mono text-xs text-slate-300" title="Maskelenmiş API anahtarı">
                        {k.maskedKey}
                      </span>
                      <span className="ml-auto flex flex-wrap gap-1">
                        <PermChip ok={k.permissions.spot_trade} label="Spot" />
                        <PermChip ok={k.permissions.contract_trade} label="Contract" />
                        {k.permissions.read_only && (
                          <span className="rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-semibold text-amber-400">
                            READ-ONLY ⚠
                          </span>
                        )}
                        {k.permissions.can_withdraw ? (
                          <span className="inline-flex items-center gap-0.5 rounded border border-rose-500/50 bg-rose-500/15 px-1.5 py-0.5 text-[10px] font-bold text-rose-300" title="Bu anahtar çekim iznine sahip — zero-withdrawal gate ihlali">
                            WITHDRAW AÇIK
                          </span>
                        ) : (
                          <span className="inline-flex items-center gap-0.5 rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-400" title="Çekim izni yok — zero-withdrawal gate sağlandı">
                            zero-withdrawal ✓
                          </span>
                        )}
                      </span>
                      {k.note && <span className="w-full truncate text-[11px] text-slate-400" title={k.note}>Not: {k.note}</span>}
                      {k.createdAt && (
                        <span className="w-full text-[10px] text-slate-400">Eklenme: {fmtTs(Date.parse(k.createdAt))}</span>
                      )}
                      <AlertDialog>
                    <AlertDialogTrigger asChild>
                      <Button size="sm" variant="outline" className="min-h-9 border-rose-500/30 px-2 text-rose-400 hover:bg-rose-500/10">
                        <Trash2 className="h-3.5 w-3.5" aria-hidden />
                      </Button>
                    </AlertDialogTrigger>
                    <AlertDialogContent className="border-border-strong bg-[#151a24]">
                      <AlertDialogHeader>
                        <AlertDialogTitle className="text-slate-100">Anahtar silinsin mi?</AlertDialogTitle>
                        <AlertDialogDescription className="text-slate-400">
                          {k.label} ({k.venue}) kasadan kalıcı olarak kaldırılacak. Bu anahtarı kullanan LIVE botlar emir
                          gönderemez hale gelir.
                        </AlertDialogDescription>
                      </AlertDialogHeader>
                      <AlertDialogFooter>
                        <AlertDialogCancel className="border-border-strong bg-transparent text-slate-300 hover:bg-secondary hover:text-slate-100">
                          Vazgeç
                        </AlertDialogCancel>
                        <AlertDialogAction
                          className="bg-rose-500 text-white hover:bg-rose-600"
                          onClick={(e) => {
                            e.preventDefault();
                            void removeKey(k.id);
                          }}
                        >
                          Sil
                        </AlertDialogAction>
                      </AlertDialogFooter>
                    </AlertDialogContent>
                  </AlertDialog>
                    </li>
                  ))}
                </ul>
              )}
            </>
          )}
        </Panel>
      </div>

      {/* System card */}
      <div className="space-y-4">
        <Panel title="Sistem" icon={<Cpu className="h-4 w-4 text-slate-500" aria-hidden />}>
          <div className="num grid grid-cols-2 gap-3 text-xs">
            <div>
              <div className="text-slate-500">SÜRÜM</div>
              <div className="mt-0.5 text-slate-100">{health?.version ?? "—"}</div>
            </div>
            <div>
              <div className="text-slate-500">SERVİS</div>
              <div className="mt-0.5 text-slate-100">{health?.service ?? "—"}</div>
            </div>
            <div className="col-span-2">
              <div className="text-slate-500">MATEMATİK ÇEKİRDEĞİ</div>
              {/* Sadece canlı sağlık yanıtını göster — çevrimdışı sahte veri yok */}
              <div className={`mt-0.5 ${health?.math ? "text-emerald-400" : "text-slate-500"}`}>
                {health?.math ?? "— (motor çevrimdışı)"}
              </div>
            </div>
            <div className="col-span-2">
              <div className="text-slate-500">VENUE&apos;LER</div>
              <div className="mt-0.5 flex gap-1.5">
                {health?.venues ? (
                  health.venues.map((v) => <VenueChip key={v} venue={v} />)
                ) : (
                  <span className="text-xs text-slate-500">— (motor çevrimdışı)</span>
                )}
              </div>
            </div>
          </div>
        </Panel>
        {/* Faz 25 (UIX-23): YZ sağlayıcı yapılandırması tek yer — Kokpit TR özeti,
            Studio hafıza özeti ve Lab teşhisi bu ayarları paylaşır. Anahtar
            yalnız bu sekmede (sessionStorage) yaşar; diske yazılmaz. */}
        <Panel title="YZ Sağlayıcısı (Özetler & Teşhis)" icon={<Sparkles className="h-4 w-4 text-violet-400" aria-hidden />}>
          <p className="text-[11px] leading-relaxed text-slate-400">
            Haber TR özeti, bot hafıza özeti ve YZ teşhisi ortak bu ayarları kullanır. API anahtarı
            <span className="font-semibold text-slate-300"> yalnız bu tarayıcı sekmesinde</span> tutulur — diske yazılmaz, motora istek gövdesi dışında taşınmaz.
          </p>
          <div className="mt-3 grid gap-2 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label className="text-xs text-slate-400">Sağlayıcı</Label>
              <Select value={ai.provider} onValueChange={(v) => { ai.setProvider(v); ai.setModel(""); }}>
                <SelectTrigger className="min-h-10 border-border-strong bg-surface text-xs text-slate-200" aria-label="YZ sağlayıcısı">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent className="border-border-strong bg-[#151a24]">
                  {(aiProviders.length > 0
                    ? aiProviders
                    : [{ id: "builtin", label: "Yerleşik (paketlenmedi)", models: [], needsKey: false, note: "" } as AiProvider]
                  ).map((p) => (
                    <SelectItem key={p.id} value={p.id} className="text-xs text-slate-200">
                      {p.label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs text-slate-400">Model (boş = sağlayıcı varsayılanı)</Label>
              <Input
                value={ai.model}
                onChange={(e) => ai.setModel(e.target.value)}
                placeholder="örn. gpt-4o-mini"
                aria-label="YZ modeli"
                className="min-h-10 border-border-strong bg-surface text-xs text-slate-100"
              />
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs text-slate-400">API anahtarı (yalnız bu sekme)</Label>
              <Input
                type="password"
                value={ai.apiKey}
                onChange={(e) => ai.setApiKey(e.target.value)}
                placeholder="sk-…"
                autoComplete="off"
                aria-label="YZ API anahtarı"
                className="min-h-10 border-border-strong bg-surface text-xs text-slate-100"
              />
            </div>
            <div className="space-y-1.5">
              <Label className="text-xs text-slate-400">Base URL (Ollama gibi yerel sağlayıcılar için)</Label>
              <Input
                value={ai.baseUrl}
                onChange={(e) => ai.setBaseUrl(e.target.value)}
                placeholder="http://localhost:11434/v1"
                aria-label="YZ base URL"
                className="min-h-10 border-border-strong bg-surface text-xs text-slate-100"
              />
            </div>
          </div>
          {aiCfgNote && <p className="mt-2 text-[10px] leading-relaxed text-slate-400">{aiCfgNote}</p>}
        </Panel>
        {/* P3-1: Kara Liste — bu sembollere yeni bot kurulamaz */}
        <Panel title="Kara Liste" icon={<Ban className="h-4 w-4 text-slate-500" aria-hidden />}>
          <p className="text-[11px] leading-relaxed text-slate-400">
            Listedeki sembole yeni bot kurulamaz (motor 422 ile reddeder). Mevcut botlar etkilenmez.
            Her değişiklik olay günlüğüne yazılır.
          </p>
          <div className="mt-3 flex flex-col gap-2 sm:flex-row">
            <Input
              value={blPair}
              onChange={(e) => setBlPair(e.target.value.toUpperCase())}
              placeholder="BTCUSDT"
              aria-label="Engellenecek sembol"
              className="num h-11 flex-1"
            />
            <Input
              value={blReason}
              onChange={(e) => setBlReason(e.target.value)}
              placeholder="Neden? (örn. düşük hacim)"
              aria-label="Engelleme nedeni"
              className="h-11 flex-1"
            />
            <Button onClick={() => void addBl()} disabled={blBusy || !blPair.trim()} className="min-h-11">
              {blBusy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : "Engelle"}
            </Button>
          </div>
          {blError && <p className="mt-2 text-xs text-rose-400">{blError}</p>}
          {bl.length > 0 ? (
            <ul className="mt-3 space-y-1.5">
              {bl.map((r) => (
                <li key={r.pair} className="flex items-center gap-2 rounded-md border border-slate-700/60 bg-slate-800/40 px-2.5 py-2 text-xs">
                  <Ban className="h-3.5 w-3.5 shrink-0 text-rose-400" aria-hidden />
                  <span className="num font-bold text-slate-200">{r.pair}</span>
                  {r.reason && <span className="min-w-0 flex-1 truncate text-slate-400">{r.reason}</span>}
                  <button
                    type="button"
                    onClick={() => void removeBl(r.pair)}
                    disabled={blBusy}
                    className="ml-auto inline-flex min-h-9 min-w-9 items-center justify-center rounded-md text-slate-400 transition-colors hover:bg-slate-700 hover:text-slate-100 disabled:opacity-50"
                    aria-label={`${r.pair} engelini kaldır`}
                  >
                    <Trash2 className="h-3.5 w-3.5" aria-hidden />
                  </button>
                </li>
              ))}
            </ul>
          ) : (
            <p className="mt-3 text-xs text-slate-500">Liste boş — her sembole bot kurulabilir.</p>
          )}
        </Panel>
        {/* P3-5: Bakım Sessizliği */}
        <Panel title="Bakım Sessizliği" icon={<Wrench className="h-4 w-4 text-slate-500" aria-hidden />}>
          <p className="text-[11px] leading-relaxed text-slate-400">
            Borsa tarafında bakım/kesinti varken üst bantta uyarı gösterir. Bilgilendiricidir — botları
            durdurmaz; durdurma için Kill-switch veya Dondur kullanılır. Durum:{" "}
            <span className={`font-bold ${maint?.mode === "NORMAL" ? "text-emerald-400" : "text-amber-400"}`}>{maint?.mode ?? "—"}</span>
          </p>
          <div className="mt-3 flex flex-col gap-2 sm:flex-row">
            <Input
              value={maintNote}
              onChange={(e) => setMaintNote(e.target.value)}
              placeholder="Not (örn. Bybit 02:00 bakımı)"
              aria-label="Bakım notu"
              className="h-11 flex-1"
            />
            {maint?.quiet ? (
              <Button onClick={() => void setQuiet(false)} disabled={maintBusy} variant="outline" className="min-h-11">
                Sessizliği Kapat
              </Button>
            ) : (
              <Button onClick={() => void setQuiet(true)} disabled={maintBusy} className="min-h-11">
                Sessizliği Aç
              </Button>
            )}
          </div>
        </Panel>
        {/* P2-3: Canlı Self-Check (19 kontrol) */}
        <Panel title="Canlı Self-Check" icon={<ListChecks className="h-4 w-4 text-slate-500" aria-hidden />}> 
          <p className="text-[11px] leading-relaxed text-slate-400">
            Motor + canlı borsa bağlantısını 19 maddede denetler: venue erişimi, anahtar izinleri, sembol
            kuralları, bakiye kapısı ve hız limiti. Anahtar yokken imzalı kontroller atlanır (YETERSİZ_ORTAM).
          </p>
          <div className="mt-3 flex flex-wrap items-center gap-2">
            <button
              type="button"
              onClick={() => void runSelfCheck()}
              disabled={scBusy}
              className="inline-flex min-h-11 items-center gap-2 rounded-md bg-cyan-500/90 px-4 text-sm font-semibold text-[#04151a] transition-colors hover:bg-cyan-400 disabled:opacity-50"
            >
              {scBusy ? <Loader2 className="h-4 w-4 animate-spin" aria-hidden /> : <ListChecks className="h-4 w-4" aria-hidden />}
              {scBusy ? "Denetleniyor…" : "Self-Check Çalıştır"}
            </button>
            {sc && (
              <span
                className={`rounded border px-2 py-1 text-xs font-bold ${sc.verdict === "GECTI" ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400" : sc.verdict === "YETERSIZ_ORTAM" ? "border-amber-500/40 bg-amber-500/10 text-amber-400" : "border-rose-500/40 bg-rose-500/10 text-rose-400"}`}
              >
                {sc.verdict === "GECTI" ? "✓ GEÇTİ" : sc.verdict === "YETERSIZ_ORTAM" ? "… YETERSİZ ORTAM" : "✕ KALDI"}
                <span className="num ml-1 font-normal opacity-80">{sc.passed} geçti · {sc.failed} kaldı · {sc.abstained} atlandı · {sc.elapsedMs}ms</span>
              </span>
            )}
          </div>
          {scError && <p className="mt-2 text-xs text-rose-400">{scError}</p>}
          {sc && sc.note && <p className="mt-2 text-[11px] leading-relaxed text-slate-400">{sc.note}</p>}
          {sc && (
            <div className="mt-3 max-h-72 space-y-3 overflow-y-auto pr-1">
              {["Yerel", "Ortam", "İmza/Bybit", "İmza/Binance"].map((g) => {
                const items = sc.items.filter((i) => i.group === g);
                if (items.length === 0) return null;
                return (
                  <div key={g}>
                    <div className="mb-1 text-[10px] font-bold uppercase tracking-wider text-slate-400">{GROUP_LABEL[g] ?? g}</div>
                    <ul className="space-y-1">
                      {items.map((i) => (
                        <li key={i.id} className="flex items-start gap-2 text-xs">
                          <span className={`mt-0.5 shrink-0 rounded border px-1.5 py-0.5 text-[10px] font-bold ${i.status === "PASS" ? "border-emerald-500/40 bg-emerald-500/10 text-emerald-400" : i.status === "FAIL" ? "border-rose-500/40 bg-rose-500/10 text-rose-400" : "border-slate-600/50 bg-slate-600/10 text-slate-400"}`}>
                            {i.status === "PASS" ? "✓" : i.status === "FAIL" ? "✕" : "…"}
                          </span>
                          <div className="min-w-0">
                            <span className="num text-slate-300">{i.id}</span>
                            <span className="ml-1.5 text-slate-400">{i.name}</span>
                            {i.detail && <div className="truncate text-[11px] text-slate-400" title={i.detail}>{i.detail}</div>}
                          </div>
                          <span className="num ml-auto shrink-0 text-[10px] text-slate-400">{i.ms}ms</span>
                        </li>
                      ))}
                    </ul>
                  </div>
                );
              })}
            </div>
          )}
        </Panel>
        <Panel title="BNB Komisyon Bekçisi" icon={<Coins className="h-4 w-4 text-slate-500" aria-hidden />}>
          <BnbCard />
        </Panel>
        <Panel title="Proje Paketi" icon={<Cpu className="h-4 w-4 text-slate-500" aria-hidden />}>
          <p className="text-[11px] leading-relaxed text-slate-400">
            Tüm proje tek ZIP&apos;te: Rust çekirdek (6 crate), Next.js terminal UI, Bybit araştırma dokümanı,
            roadmap spec&apos;i, E2E test süiti ve konfigürasyonlar. Build çıktıları (target/, node_modules/) hariçtir.
          </p>
          <a
            href="/dcabot-v3.0.zip"
            download="dcabot-v3.0.zip"
            className="mt-3 inline-flex min-h-11 w-full items-center justify-center gap-2 rounded-md bg-emerald-500/90 px-6 text-sm font-semibold text-[#04150e] transition-colors hover:bg-emerald-500 sm:w-auto"
          >
            <Download className="h-4 w-4" aria-hidden />
            dcabot-v3.0.zip indir (~0.5 MB)
          </a>
        </Panel>

        <Panel title="Risk Felsefesi — İki Sınıf" icon={<ShieldAlert className="h-4 w-4 text-slate-500" aria-hidden />}>
          <div className="grid gap-3 sm:grid-cols-2">
            <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
              <div className="text-xs font-bold text-amber-400">1 · Uyarı &amp; Geçersiz Kılınabilir Korumalar</div>
              <p className="mt-1.5 text-[11px] leading-relaxed text-slate-400">
                M20 SL-Merdiven, Grid Komisyon Kırıcı (g_min), Seans Koruması, Açılış Salvo Tamponu, Gap Konsolidasyonu,
                Makro Kara Pencere, Rejim Filtresi, Çoklu Güvenlik Emri, Günlük DD Kesici, Dolum Modeli.
              </p>
              <p className="mt-1.5 text-[11px] leading-relaxed text-amber-300/90">
                Patron kullanıcıdır: motor emri engellemez; ne işe yaradığını ve üstlenilen riski yazar. Kullanıcı
                &quot;riski anladım&quot; onayı ile (Acknowledge &amp; Proceed) geçersiz kılabilir.
              </p>
            </div>
            <div className="rounded-lg border border-rose-500/30 bg-rose-500/5 p-3">
              <div className="text-xs font-bold text-rose-400">2 · Fiziksel &amp; Güvenlik Değişmezleri</div>
              <p className="mt-1.5 text-[11px] leading-relaxed text-slate-400">
                Zero-Withdrawal Hard Gate (çekim izni olan API anahtarı HTTP 422 ile reddedilir) ve Gate 6 — Olmayan
                Parayı Harcamama (bakiye yetersizse emir SKIP/WAIT).
              </p>
              <p className="mt-1.5 text-[11px] leading-relaxed text-rose-300/90">
                Borsa fizik kuralları ve cüzdan güvenliği gereği asla esnetilemez; hiçbir kullanıcı onayı bu ikisini
                aşamaz.
              </p>
            </div>
          </div>
        </Panel>

        <Panel title="Sözlük & SSS — Hata Kodları ve Yardım" icon={<BookOpen className="h-4 w-4 text-slate-500" aria-hidden />}>
          <GlossaryBrowser />
        </Panel>
        <Panel title="TradingView Webhook Kurulumu" icon={<Webhook className="h-4 w-4 text-slate-500" aria-hidden />}>
          <p className="text-[11px] leading-relaxed text-slate-400">
            TradingView alarmını (webhook sekmesi) aşağıdaki uca yönlendirin — sinyal, botun
            Studio → Ayarlar sekmesindeki gizli anahtarıyla eşleşirse kuyruğa girer, deal bir sonraki
            bar değerlendirmesinde açılır. Bot başına gizli anahtar Studio&apos;da gösterilir; burada
            secret yazılmaz, gösterilmez.
          </p>
          <div className="num mt-2.5 rounded-lg border border-border-strong bg-surface p-2.5 font-mono text-[11px] text-cyan-300">
            POST /api/webhooks/tradingview
          </div>
          <pre className="num mt-2 overflow-x-auto rounded-lg border border-border bg-surface p-2.5 font-mono text-[10px] leading-relaxed text-slate-300">
{`{
  "secret": "tvw_<bot-gizli-anahtarı>",
  "botId": "<bot-id>",
  "action": "START_DEAL",
  "pair": "BTCUSDT",
  "timestamp": 1776672000000
}`}
          </pre>
          <ul className="mt-2.5 space-y-1.5 text-[11px] leading-relaxed text-slate-400">
            <li><span className="font-semibold text-slate-200">Secret:</span> <code className="font-mono text-[10px] text-amber-300">tvw_</code> önekli, bot başına üretilir (Studio → bot → Ayarlar sekmesi).</li>
            <li><span className="font-semibold text-slate-200">Replay koruması:</span> timestamp sn veya ms kabul edilir; ±60 sn dışı EXPIRED_ALERT ile reddedilir.</li>
            <li><span className="font-semibold text-slate-200">Pair kapısı:</span> <code className="font-mono text-[10px]">pair</code> bot sembolüyle uyuşmazsa PAIR_MISMATCH + WARN olayı.</li>
            <li><span className="font-semibold text-slate-200">Fail-closed:</span> açık deal varken sinyal DEAL_ALREADY_ACTIVE ile yoksayılır (çifte deal asla açılmaz); bot RUNNING değilse, KILL devredeyse veya LIVE hatta FREEZE devredeyse sinyal işlenmez.</li>
          </ul>
        </Panel>
        <Panel title="8 Fail-Closed Gate" icon={<ShieldCheck className="h-4 w-4 text-slate-500" aria-hidden />}>
          <ul className="space-y-2">
            {GATES_INFO.map(([g, desc]) => (
              <li key={g} className="flex items-start gap-2 text-xs">
                <span className="num mt-0.5 shrink-0 rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-bold text-emerald-400">
                  {g}
                </span>
                <span className="leading-relaxed text-slate-400">{desc}</span>
              </li>
            ))}
          </ul>
          <div className="mt-3 rounded-lg border border-cyan-500/25 bg-cyan-500/5 p-3 text-[11px] leading-relaxed text-slate-400">
            <span className="font-semibold text-cyan-400">Fail-closed ilkesi:</span> bir gate doğrulayamazsa emir ÜRETİLMEZ.
            Tüm finansal hesaplamalar 4096-bit rasyonel aritmetikte yürür; float yalnızca ekran gösteriminde kullanılır.
            Gate 7/8a/8b ve M20/g_min artık <span className="text-amber-400">RiskPolicy</span> ile bot başına
            ZORUNLU veya UYARI+İZİN modunda çalışır (sihirbaz Bölüm 6&apos;dan seçilir).
          </div>
        </Panel>
      </div>
    </div>
  );
}

/* P4-4: sözlük + SSS tarayıcı (v3 help/glossary karşılığı — salt-okunur). */
function GlossaryBrowser() {
  const [q, setQ] = useState("");
  const [tab, setTab] = useState<"glossary" | "faq">("glossary");
  const [openId, setOpenId] = useState<string | null>(null);
  const needle = q.trim().toLowerCase();
  const glossHits = GLOSSARY.filter((g) =>
    !needle ||
    g.code.toLowerCase().includes(needle) ||
    g.title.toLowerCase().includes(needle) ||
    g.plainCause.toLowerCase().includes(needle) ||
    g.exchange.toLowerCase().includes(needle),
  );
  const faqHits = FAQ.filter((f) =>
    !needle ||
    f.question.toLowerCase().includes(needle) ||
    f.answer.toLowerCase().includes(needle) ||
    f.category.toLowerCase().includes(needle) ||
    f.keywords.some((k) => k.toLowerCase().includes(needle)),
  );
  return (
    <div>
      <div className="flex gap-1.5">
        {([{ v: "glossary", l: `SÖZLÜK (${glossHits.length})` }, { v: "faq", l: `SSS (${faqHits.length})` }] as const).map((o) => (
          <button
            key={o.v}
            onClick={() => { setTab(o.v); setOpenId(null); }}
            aria-pressed={tab === o.v}
            className={cn(
              "min-h-10 flex-1 rounded-lg border px-2 py-2 text-[11px] font-bold",
              tab === o.v ? "border-cyan-500/60 bg-cyan-500/10 text-cyan-300" : "border-border text-slate-500 hover:border-border-strong",
            )}
          >
            {o.l}
          </button>
        ))}
      </div>
      <Input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder={tab === "glossary" ? "kod / başlık ara — örn. -2010" : "soru ara — örn. sapma adımı"}
        aria-label="Sözlükte ara"
        className="mt-2 min-h-11 border-border-strong bg-surface text-slate-100"
      />
      {tab === "glossary" ? (
        <ul className="mt-2 max-h-72 space-y-1.5 overflow-y-auto">
          {glossHits.map((g) => {
            const open = openId === g.code;
            return (
              <li key={g.code} className="rounded-lg border border-border bg-surface">
                <button onClick={() => setOpenId(open ? null : g.code)} aria-expanded={open} className="flex w-full items-center gap-2 px-2.5 py-2 text-left">
                  <span className="num shrink-0 rounded border border-amber-500/40 bg-amber-500/10 px-1.5 py-0.5 font-mono text-[10px] font-bold text-amber-300">{g.code}</span>
                  <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-slate-200">{g.title}</span>
                  <span className="shrink-0 text-[9px] font-bold tracking-wider text-slate-400">{g.exchange} · {g.severity}</span>
                </button>
                {open && (
                  <div className="space-y-1.5 px-2.5 pb-2.5 text-[11px] leading-relaxed">
                    <p className="text-slate-300"><span className="font-bold text-slate-400">Neden: </span>{g.plainCause}</p>
                    <p className="text-slate-400"><span className="font-bold text-slate-500">Bot eylemi: </span>{g.botAction}</p>
                    {g.operatorSteps && g.operatorSteps.length > 0 && (
                      <ol className="list-decimal space-y-0.5 pl-4 text-slate-500">
                        {g.operatorSteps.map((s, i) => <li key={i}>{s}</li>)}
                      </ol>
                    )}
                  </div>
                )}
              </li>
            );
          })}
          {glossHits.length === 0 && <li className="text-[11px] text-slate-400">Eşleşme yok.</li>}
        </ul>
      ) : (
        <ul className="mt-2 max-h-72 space-y-1.5 overflow-y-auto">
          {faqHits.map((f) => {
            const open = openId === f.id;
            return (
              <li key={f.id} className="rounded-lg border border-border bg-surface">
                <button onClick={() => setOpenId(open ? null : f.id)} aria-expanded={open} className="flex w-full items-center gap-2 px-2.5 py-2 text-left">
                  <span className="min-w-0 flex-1 truncate text-[11px] font-semibold text-slate-200">{f.question}</span>
                  <span className="shrink-0 text-[9px] font-bold tracking-wider text-slate-400">{f.category}</span>
                </button>
                {open && <p className="px-2.5 pb-2.5 text-[11px] leading-relaxed text-slate-400">{f.answer}</p>}
              </li>
            );
          })}
          {faqHits.length === 0 && <li className="text-[11px] text-slate-400">Eşleşme yok.</li>}
        </ul>
      )}
      <p className="mt-2 text-[10px] leading-tight text-slate-400">YZ Yardımcı&apos;nın RAG bağlamı bu iki listeden beslenir; burası salt-okunur yardımdır.</p>
    </div>
  );
}

/* Faz 1.6: BNB komisyon bekçisi kartı (v3 bnb-sentinel karşılığı, GET only). */
function BnbCard() {
  const { status } = useBnb(60000);
  const b: BnbStatus | null = status;
  const alert = b?.alert === true;
  const abstain = b?.source === "ABSTAIN";
  return (
    <div>
      <p className="text-[11px] leading-relaxed text-slate-400">
        Binance spot cüzdandaki serbest BNB — komisyon indirimi için eşik{" "}
        <span className="num font-bold text-slate-200">0.05</span> BNB. Altındayken komisyon base
        asset&apos;ten kesilir (toz birikimi). Salt-okunur; süpürme düğmesi yok.
      </p>
      <div className="mt-2.5 flex flex-wrap items-center gap-2">
        <span
          className={`rounded border px-2 py-1 text-xs font-bold ${!b ? "border-slate-600/50 bg-slate-600/10 text-slate-400" : alert ? "border-amber-500/40 bg-amber-500/10 text-amber-400" : abstain ? "border-slate-600/50 bg-slate-600/10 text-slate-400" : "border-emerald-500/40 bg-emerald-500/10 text-emerald-400"}`}
        >
          {!b ? "… okunuyor" : alert ? `⚠ DÜŞÜK — ${b.freeBnb ?? "?"} BNB` : abstain ? "… ANAHTARSIZ (ABSTAIN)" : `✓ YETERLİ — ${b.freeBnb ?? "?"} BNB`}
        </span>
        {b?.cached && <span className="text-[10px] text-slate-400">önbellek (45 sn)</span>}
      </div>
      {b?.reason && <p className="mt-1.5 text-[11px] text-slate-400">{b.reason}</p>}
    </div>
  );
}

/* P2-3: self-check grup başlıkları */
const GROUP_LABEL: Record<string, string> = {
  "Yerel": "Yerel (ağ yok)",
  "Ortam": "Ortam (halka açık)",
  "İmza/Bybit": "İmzalı — Bybit",
  "İmza/Binance": "İmzalı — Binance",
};

function PermChip({ ok, label }: { ok: boolean; label: string }) {
  return ok ? (
    <span className="inline-flex items-center gap-0.5 rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-semibold text-emerald-400">
      <CheckCircle2 className="h-3 w-3" aria-hidden />
      {label}
    </span>
  ) : (
    <span className="inline-flex items-center gap-0.5 rounded border border-slate-600/50 bg-slate-600/10 px-1.5 py-0.5 text-[10px] font-semibold text-slate-400">
      <XCircle className="h-3 w-3" aria-hidden />
      {label}
    </span>
  );
}
