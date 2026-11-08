"use client";

/* Faz 2.5 — Bilgi Merkezi (v3 knowledge-view karşılığı, salt-içerik UI).
 *
 * v3: fuse.js bulanık arama + kategori sekmeleri + AI-asistan bağlantısı,
 * kaynak faq.json. Bizde fuse.js bağımlılığı YOK — yerine Türkçe-duyarsız
 * jeton-tabanlı arama (trLower fold); bağımlılık eklenmedi.
 * Sözlük de aynı görünüme alındı (Ayarlar'daki tarayıcı durur; burası
 * toplu başvuru noktası). AI bağlantısı Lab'deki YZ paneline gider
 * (onOpenAi → Backtest Lab), ayrı asistan penceresi yok.
 *
 * Faz 2.6 — 3. sekme "Sözleşme" (v3 docs-view karşılığı): dürüstlük
 * paneli + M01–M08 kartları + yol haritası durumu + docs/ listesi.
 * İçerik v3'ten KOPYALANMAZ — bu platformun ölçülen gerçekliği yazılır
 * (uymayan madde YOK rozeti alır, uydurulmaz).
 */

import { useMemo, useState } from "react";
import { BookOpen, ChevronDown, FileText, Map, Search, ShieldAlert, ShieldCheck, Sparkles, Webhook } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { cn } from "@/lib/utils";
import type { FaqEntry, GlossaryEntry } from "@/lib/engine-types";
import faqJson from "@/data/faq.json";
import glossaryJson from "@/data/glossary.json";

const FAQ = faqJson as FaqEntry[];
const GLOSSARY = glossaryJson as GlossaryEntry[];

/**
 * Türkçe-duyarsız katlama: I/ı/İ/i → i, ş→s, ğ→g, ü→u, ö→o, ç→c.
 * Not: saf `toLocaleLowerCase("tr")` KULLANILMAZ — "FUNDING"→"fundıng"
 * yapıp verideki "funding" ile eşleşmeyi bozar (I→ı). Her iki taraf da
 * aynı fold'dan geçtiği için ASCII + Türkçe sorgular birlikte çalışır.
 */
export function trLower(s: string): string {
  return s
    .toLowerCase()
    .replace(/i̇/g, "i")
    .replace(/ı/g, "i")
    .replace(/ş/g, "s")
    .replace(/ğ/g, "g")
    .replace(/ü/g, "u")
    .replace(/ö/g, "o")
    .replace(/ç/g, "c");
}

/** Jetonların TÜMÜ metinde geçmeli (AND); skor = toplam geçiş sayısı. */
function scoreHaystack(haystack: string, tokens: string[]): number {
  let score = 0;
  for (const t of tokens) {
    let idx = haystack.indexOf(t);
    if (idx < 0) return -1;
    let n = 0;
    while (idx >= 0) {
      n += 1;
      idx = haystack.indexOf(t, idx + t.length);
    }
    score += n;
  }
  return score;
}

/* ---------------- Faz 2.6 Sözleşme sekmesi verisi (statik, uydurma yok) ---------------- */

type HonestyState = "REAL" | "SIM" | "NONE";

const HONESTY: { state: HonestyState; label: string; detail: string }[] = [
  { state: "REAL", label: "Rasyonel çekirdek (M01–M20)", detail: "GERÇEK — para matematiğinde float/NaN yok; 4096-bit kesirler, DB'de n/d string." },
  { state: "REAL", label: "8 fail-closed gate + kill-switch + freeze", detail: "GERÇEK — şalterler olay akışından geri yüklenir (restart-safe)." },
  { state: "REAL", label: "Backtest + güven sertifikası", detail: "GERÇEK — trades≥5, WFE≥%50, DSR≥0.95, PBO≤%20; rozetsiz strateji canlıya alınmaz (422)." },
  { state: "REAL", label: "Paper motor", detail: "GERÇEK — gerçek fiyat, simüle emir; DCA/grid aynı çekirdek." },
  { state: "REAL", label: "Dry-run", detail: "GERÇEK — M08 tick/step hizalama, sent:false; ağa emir gitmez." },
  { state: "REAL", label: "Self-check (19 kontrol)", detail: "GERÇEK — anahtar yoksa imzalı kontroller ABSTAIN → YETERSİZ_ORTAM." },
  { state: "REAL", label: "Kasa (vault)", detail: "GERÇEK — AES-256-GCM; çekim izinli anahtar 422 ile reddedilir." },
  { state: "REAL", label: "Fleet risk (VaR/CVaR)", detail: "GERÇEK — raporlama sınırı: karekök f64 ile, estimate etiketli; para matematiği değil." },
  { state: "REAL", label: "İndikatörler + rejim + kalibrasyon", detail: "GERÇEK — sinyal motoruyla aynı fonksiyonlar (MACD quirk düzeltilmiş); kural-bazlı rejim, 90-bar RV." },
  { state: "REAL", label: "Bot hafızası", detail: "GERÇEK — ölçülen bağlam (snapshot/parite/rejim); hata/kısa yanıt → ABSTAIN." },
  { state: "REAL", label: "BNB komisyon bekçisi", detail: "GERÇEK — salt-okunur serbest BNB + 0.05 eşiği, 45 sn önbellek; süpürme düğmesi yok." },
  { state: "SIM", label: "Emirler", detail: "SİMÜLE + KAPILI — PAPER simüle; LIVE yalnız SonCert rozetiyle açılır." },
  { state: "SIM", label: "TV webhook", detail: "SİMÜLE — sinyal kuyruğa girer, paper hatta işlenir; kill/freeze LIVE'ı durdurur." },
  { state: "NONE", label: "Kapanan-deal defteri", detail: "YOK — kapanan-% / PF / 7g pencereler hesaplanamaz → null + gerekçe." },
  { state: "REAL", label: "Olay adli tıp (izler)", detail: "GERÇEK — tick başına trace_id (göçlü şema); Studio Olaylar'da iz zaman çizgisi. Span kırılımı yok." },
  { state: "NONE", label: "ML rejim/kalibrasyon", detail: "YOK — kural-bazlı rejim + RV kalibrasyon var; ML modeli yok." },
];

const MATH_CONTRACT: { id: string; title: string; one: string }[] = [
  { id: "M01", title: "Decimal / Fraction", one: "fraction.rs — tam kesirli aritmetik; float ve NaN reddedilir." },
  { id: "M02", title: "DCA planı", one: "m02.rs — preview / replay / runtime tek fonksiyon." },
  { id: "M03", title: "Bütçe", one: "m03_m07.rs — plannedGross / initialMargin / requiredCollateral + stres yolları." },
  { id: "M04", title: "Fill / WAC", one: "m04.rs — idempotent fill anahtarı, ağırlıklı ortalama maliyet." },
  { id: "M05", title: "Fee / TP", one: "Rol-esaslı komisyon; net take-profit pozitif kökü; funding işaret köprüsü." },
  { id: "M06", title: "Equity / DD", one: "Her olayda peak/DD/MDD gözlenir; özkaynak ≤ 0 iken yeni risk kapalı." },
  { id: "M07", title: "Likidasyon", one: "m03_m07.rs — long/short tam kök; %2 buffer; max kaldıraç kapalı formül." },
  { id: "M08", title: "Tick / Step", one: "Fiyat/miktar hizalaması (tickDown/qtyDown); minNotional otomatik büyütme YOK." },
];

const ROADMAP_DONE: string[] = [
  "Faz 0 — SonCert kapısı, kill-switch, freeze",
  "Faz 1 — dry-run, status, multi-status, fleet overview, ping/macro/simulate, BNB bekçisi",
  "Faz 2 — fleet risk VaR, indikatörler, bot hafızası, bilgi merkezi, sözleşme sekmesi, brackets aynası, olay adli tıp",
];

const ROADMAP_TODO: { id: string; title: string; note: string }[] = [
  { id: "2.8", title: "Piyasa istasyonu ve kartlar (P3)", note: "Yeni motor işi yok; mevcut uçlar üzerine UI gruplama." },
];

const DOCS: { file: string; one: string }[] = [
  { file: "YOL_HARITASI_KALAN.md", one: "v3'ten kalan port planı — hükümlü (TAŞI/ŞARTLI/TAŞIMA)." },
  { file: "ENVANTER_H_V3_PORT.md", one: "v3 ↔ bu platform yetenek matrisi (VAR/KISMEN/YOK)." },
  { file: "DCABOT_MASTER_REWRITE_SPECIFICATION.md", one: "Yeniden yazım ana şartnamesi." },
  { file: "DCABOT_SISTEM_VE_MIMARI_ANALIZI.md", one: "Sistem ve mimari analizi." },
  { file: "ARASTIRMA_RAPORU_2025-10-21.md", one: "Araştırma raporu." },
  { file: "BYBIT_RESEARCH.md", one: "Bybit venue araştırması." },
  { file: "INDIRME-README.md", one: "Proje paketi indirme notu." },
];

const STATE_STYLE: Record<HonestyState, string> = {
  REAL: "border-emerald-500/40 bg-emerald-500/10 text-emerald-300",
  SIM: "border-amber-500/40 bg-amber-500/10 text-amber-300",
  NONE: "border-slate-600/50 bg-slate-600/10 text-slate-500",
};

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

function ContractTab() {
  return (
    <div className="space-y-6">
      <section aria-label="Dürüstlük paneli">
        <h2 className="flex items-center gap-1.5 text-sm font-bold text-slate-100">
          <ShieldCheck className="h-4 w-4 text-emerald-400" aria-hidden />
          Dürüstlük Paneli
        </h2>
        <p className="mt-1 text-[11px] text-slate-400">Bu terminalde ne GERÇEK, ne SİMÜLE, ne YOK — bölüm koşmadan etiketlenmez.</p>
        <ul className="mt-2.5 grid gap-1.5 md:grid-cols-2">
          {HONESTY.map((h) => (
            <li key={h.label} className="flex items-start justify-between gap-2.5 rounded-lg border border-border bg-surface p-2.5">
              <div className="min-w-0">
                <p className="text-xs font-semibold text-slate-200">{h.label}</p>
                <p className="mt-0.5 text-[11px] leading-relaxed text-slate-400">{h.detail}</p>
              </div>
              <span className={`shrink-0 rounded border px-1.5 py-0.5 font-mono text-[10px] font-bold ${STATE_STYLE[h.state]}`}>{h.state}</span>
            </li>
          ))}
        </ul>
      </section>
      <section aria-label="Matematik sözleşmesi M01–M08">
        <h2 className="flex items-center gap-1.5 text-sm font-bold text-slate-100">
          <FileText className="h-4 w-4 text-cyan-400" aria-hidden />
          Matematik Sözleşmesi (M01–M08)
        </h2>
        <div className="mt-2.5 grid grid-cols-1 gap-2 sm:grid-cols-2 lg:grid-cols-4">
          {MATH_CONTRACT.map((m) => (
            <div key={m.id} className="rounded-xl border border-border bg-surface p-3">
              <p className="flex items-center gap-1.5 text-xs font-bold text-slate-100">
                <span className="rounded bg-cyan-500/10 px-1.5 py-0.5 font-mono text-[10px] text-cyan-300">{m.id}</span>
                {m.title}
              </p>
              <p className="mt-1 text-[11px] leading-relaxed text-slate-400">{m.one}</p>
            </div>
          ))}
        </div>
      </section>
      <section aria-label="Yol haritası durumu">
        <h2 className="flex items-center gap-1.5 text-sm font-bold text-slate-100">
          <Map className="h-4 w-4 text-amber-400" aria-hidden />
          Yol Haritası Durumu
        </h2>
        <ul className="mt-2.5 space-y-1.5">
          {ROADMAP_DONE.map((d) => (
            <li key={d} className="flex items-start gap-2 rounded-lg border border-emerald-500/25 bg-emerald-500/[0.04] px-2.5 py-2 text-[11px] text-slate-300">
              <span className="font-mono font-bold text-emerald-400">✓</span>{d}
            </li>
          ))}
          {ROADMAP_TODO.map((t) => (
            <li key={t.id} className="flex items-start gap-2 rounded-lg border border-border bg-surface px-2.5 py-2 text-[11px]">
              <span className="shrink-0 font-mono font-bold text-slate-500">{t.id}</span>
              <span className="text-slate-300">{t.title} <span className="text-slate-500">— {t.note}</span></span>
            </li>
          ))}
        </ul>
        <p className="mt-1.5 text-[11px] text-slate-400">Taşınmayacaklar (Telegram, toz süpürme, mainnet emir UI, login, v6Shield telemetri) plana dahil değildir.</p>
      </section>
      <section aria-label="8 Fail-Closed Gate">
        <h2 className="flex items-center gap-1.5 text-sm font-bold text-slate-100">
          <ShieldCheck className="h-4 w-4 text-emerald-400" aria-hidden />
          8 Fail-Closed Gate
        </h2>
        <p className="mt-1 text-[11px] text-slate-400">
          Fail-closed ilkesi: bir gate doğrulayamazsa emir ÜRETİLMEZ. Tüm finansal hesaplamalar 4096-bit rasyonel aritmetikte yürür; float yalnızca ekran gösteriminde kullanılır.
        </p>
        <ul className="mt-2.5 space-y-1.5">
          {GATES_INFO.map(([g, desc]) => (
            <li key={g} className="flex items-start gap-2.5 rounded-lg border border-border bg-surface p-2.5 text-xs">
              <span className="shrink-0 rounded border border-emerald-500/40 bg-emerald-500/10 px-1.5 py-0.5 font-mono text-[10px] font-bold text-emerald-400">
                {g}
              </span>
              <span className="leading-relaxed text-slate-300">{desc}</span>
            </li>
          ))}
        </ul>
      </section>

      <section aria-label="Risk Felsefesi — İki Sınıf">
        <h2 className="flex items-center gap-1.5 text-sm font-bold text-slate-100">
          <ShieldAlert className="h-4 w-4 text-amber-400" aria-hidden />
          Risk Felsefesi — İki Sınıf
        </h2>
        <div className="mt-2.5 grid gap-3 sm:grid-cols-2">
          <div className="rounded-lg border border-amber-500/30 bg-amber-500/5 p-3">
            <div className="text-xs font-bold text-amber-400">1 · Uyarı &amp; Geçersiz Kılınabilir Korumalar</div>
            <p className="mt-1.5 text-[11px] leading-relaxed text-slate-400">
              M20 SL-Merdiven, Grid Komisyon Kırıcı (g_min), Seans Koruması, Açılış Salvo Tamponu, Gap Konsolidasyonu,
              Makro Kara Pencere, Rejim Filtresi, Çoklu Güvenlik Emri, Günlük DD Kesici, Dolum Modeli.
            </p>
            <p className="mt-1.5 text-[11px] leading-relaxed text-amber-300/90">
              Patron kullanıcıdır: motor emri engellemez; ne işe yaradığını ve üstlenilen riski yazar. Kullanıcı &quot;riski anladım&quot; onayı ile geçersiz kılabilir.
            </p>
          </div>
          <div className="rounded-lg border border-rose-500/30 bg-rose-500/5 p-3">
            <div className="text-xs font-bold text-rose-400">2 · Fiziksel &amp; Güvenlik Değişmezleri</div>
            <p className="mt-1.5 text-[11px] leading-relaxed text-slate-400">
              Zero-Withdrawal Hard Gate (çekim izni olan API anahtarı HTTP 422 ile reddedilir) ve Gate 6 — Olmayan Parayı Harcamama (bakiye yetersizse emir SKIP/WAIT).
            </p>
            <p className="mt-1.5 text-[11px] leading-relaxed text-rose-300/90">
              Borsa fizik kuralları ve cüzdan güvenliği gereği asla esnetilemez; hiçbir kullanıcı onayı bu ikisini aşamaz.
            </p>
          </div>
        </div>
      </section>

      <section aria-label="TradingView Webhook Sözleşmesi">
        <h2 className="flex items-center gap-1.5 text-sm font-bold text-slate-100">
          <Webhook className="h-4 w-4 text-cyan-400" aria-hidden />
          TradingView Webhook Sözleşmesi
        </h2>
        <p className="mt-1 text-[11px] leading-relaxed text-slate-400">
          TradingView alarmını (webhook sekmesi) aşağıdaki uca yönlendirin — sinyal, botun Studio → Ayarlar sekmesindeki gizli anahtarıyla eşleşirse kuyruğa girer.
        </p>
        <div className="mt-2.5 rounded-lg border border-border-strong bg-surface p-2.5 font-mono text-[11px] text-cyan-300">
          POST /api/webhooks/tradingview
        </div>
        <pre className="mt-2 overflow-x-auto rounded-lg border border-border bg-surface p-2.5 font-mono text-[10px] leading-relaxed text-slate-300">
{`{
  "secret": "tvw_<bot-gizli-anahtarı>",
  "botId": "<bot-id>",
  "action": "START_DEAL",
  "pair": "BTCUSDT",
  "timestamp": 1776672000000
}`}
        </pre>
        <ul className="mt-2.5 space-y-1.5 text-[11px] leading-relaxed text-slate-400">
          <li><span className="font-semibold text-slate-200">Secret:</span> <code className="font-mono text-[10px] text-amber-300">tvw_</code> önekli, bot başına üretilir.</li>
          <li><span className="font-semibold text-slate-200">Replay koruması:</span> timestamp sn veya ms kabul edilir; ±60 sn dışı EXPIRED_ALERT ile reddedilir.</li>
          <li><span className="font-semibold text-slate-200">Pair kapısı:</span> <code className="font-mono text-[10px]">pair</code> bot sembolüyle uyuşmazsa PAIR_MISMATCH uyarısı.</li>
          <li><span className="font-semibold text-slate-200">Fail-closed:</span> açık deal varken çifte deal asla açılmaz; bot RUNNING değilse veya FREEZE devredeyse sinyal işlenmez.</li>
        </ul>
      </section>

      <section aria-label="Belgeler">
        <h2 className="flex items-center gap-1.5 text-sm font-bold text-slate-100">
          <BookOpen className="h-4 w-4 text-slate-400" aria-hidden />
          Belgeler <span className="font-mono text-[10px] font-normal text-slate-400">docs/</span>
        </h2>
        <ul className="mt-2.5 grid gap-1.5 md:grid-cols-2">
          {DOCS.map((d) => (
            <li key={d.file} className="rounded-lg border border-border bg-surface px-2.5 py-2">
              <p className="font-mono text-[11px] text-slate-200">{d.file}</p>
              <p className="mt-0.5 text-[11px] text-slate-400">{d.one}</p>
            </li>
          ))}
        </ul>
      </section>
    </div>
  );
}

export function KnowledgeView({ onOpenAi }: { onOpenAi: () => void }) {
  const [query, setQuery] = useState("");
  const [tab, setTab] = useState<"faq" | "glossary" | "contract">("faq");
  const [category, setCategory] = useState("Tümü");
  const [openId, setOpenId] = useState<string | null>(null);

  const categories = useMemo(() => ["Tümü", ...Array.from(new Set(FAQ.map((f) => f.category)))], []);

  const faqRows = useMemo(() => {
    const tokens = trLower(query.trim()).split(/\s+/).filter((t) => t.length >= 2);
    const scored = FAQ.map((f) => {
      const hay = trLower([f.question, f.answer, f.category, ...(f.keywords ?? [])].join("\n"));
      const s = tokens.length === 0 ? 0 : scoreHaystack(hay, tokens);
      return { f, s };
    }).filter((r) => r.s >= 0);
    scored.sort((a, b) => b.s - a.s);
    const rows = scored.map((r) => r.f);
    return category === "Tümü" ? rows : rows.filter((r) => r.category === category);
  }, [query, category]);

  const glossRows = useMemo(() => {
    const tokens = trLower(query.trim()).split(/\s+/).filter((t) => t.length >= 2);
    const scored = GLOSSARY.map((g) => {
      const hay = trLower([g.code, g.title, g.plainCause, g.botAction, g.exchange].join("\n"));
      const s = tokens.length === 0 ? 0 : scoreHaystack(hay, tokens);
      return { g, s };
    }).filter((r) => r.s >= 0);
    scored.sort((a, b) => b.s - a.s);
    return scored.map((r) => r.g);
  }, [query]);

  const empty = tab !== "contract" && (tab === "faq" ? faqRows.length === 0 : glossRows.length === 0);

  return (
    <div className="space-y-5">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h1 className="flex items-center gap-2 text-lg font-semibold tracking-tight text-slate-50">
            <BookOpen className="h-5 w-5 text-cyan-400" aria-hidden />
            Bilgi Merkezi
          </h1>
          <p className="mt-0.5 text-xs text-slate-400">
            DCA ilkeleri, kaldıraç basamakları, fonlama, rozetler, hata kodları — kayıtlı bilgi, uydurma yok
          </p>
        </div>
        <Button variant="outline" size="sm" className="shrink-0 border-cyan-500/40 text-cyan-300 hover:bg-cyan-500/10" onClick={onOpenAi}>
          <Sparkles className="h-4 w-4" aria-hidden />
          Soruyu YZ Yardımcı&apos;ya sor
        </Button>
      </div>

      <div className="space-y-3">
        <div className="flex gap-1.5">
          {([{ v: "faq", l: `SSS (${faqRows.length})` }, { v: "glossary", l: `SÖZLÜK (${glossRows.length})` }, { v: "contract", l: "SÖZLEŞME" }] as const).map((o) => (
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
        {tab !== "contract" && (
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500" aria-hidden />
            <Input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Ara: funding, kaldıraç, kill-switch, DSR, -2010… (en az 2 harf)"
              className="h-10 border-border-strong bg-surface pl-9 text-sm text-slate-100"
              aria-label="Bilgi merkezinde ara"
            />
          </div>
        )}
        {tab === "faq" && (
          <div className="flex flex-wrap gap-1.5" role="tablist" aria-label="Kategoriler">
            {categories.map((c) => (
              <button
                key={c}
                type="button"
                role="tab"
                aria-selected={category === c}
                onClick={() => setCategory(c)}
                className={cn(
                  "rounded-full border px-3 py-1.5 text-xs transition-colors",
                  category === c
                    ? "border-cyan-500/50 bg-cyan-500/10 text-cyan-300"
                    : "border-border text-slate-400 hover:border-border-strong hover:text-slate-200",
                )}
              >
                {c}
              </button>
            ))}
          </div>
        )}
      </div>

      {tab === "contract" ? (
        <ContractTab />
      ) : empty ? (
        <div className="rounded-xl border border-dashed border-border-strong bg-surface/50 p-10 text-center">
          <Search className="mx-auto mb-2 h-7 w-7 text-slate-700" aria-hidden />
          <div className="text-sm font-medium text-slate-300">&quot;{query}&quot; için sonuç yok</div>
          <p className="mt-1 text-xs text-slate-500">Farklı bir anahtar kelime deneyin veya soruyu YZ Yardımcı&apos;ya sorun.</p>
        </div>
      ) : tab === "faq" ? (
        <div className="space-y-2">
          {faqRows.map((f) => {
            const open = openId === f.id;
            return (
              <div key={f.id} className={cn("overflow-hidden rounded-xl border transition-colors", open ? "border-cyan-500/40 bg-cyan-500/[0.04]" : "border-border bg-surface")}>
                <button
                  type="button"
                  onClick={() => setOpenId(open ? null : f.id)}
                  aria-expanded={open}
                  className="flex min-h-12 w-full items-center justify-between gap-3 px-4 py-3 text-left"
                >
                  <span className="min-w-0">
                    <span className="block text-sm font-medium text-slate-100">{f.question}</span>
                    <span className="mt-1 inline-block rounded border border-border-strong px-1.5 font-mono text-[10px] text-slate-400">{f.category}</span>
                  </span>
                  <ChevronDown className={cn("h-4 w-4 shrink-0 text-slate-400 transition-transform", open && "rotate-180")} aria-hidden />
                </button>
                {open && (
                  <div className="border-t border-border px-4 py-3">
                    <p className="text-[13px] leading-relaxed text-slate-300">{f.answer}</p>
                    {(f.keywords?.length ?? 0) > 0 && (
                      <div className="mt-2.5 flex flex-wrap gap-1">
                        {(f.keywords ?? []).slice(0, 6).map((k) => (
                          <span key={k} className="rounded bg-[#1a2233] px-1.5 py-0.5 font-mono text-[10px] text-slate-400">#{k}</span>
                        ))}
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      ) : (
        <ul className="space-y-1.5">
          {glossRows.map((g) => {
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
                    {(g.operatorSteps?.length ?? 0) > 0 && (
                      <ol className="list-decimal space-y-0.5 pl-4 text-slate-500">
                        {(g.operatorSteps ?? []).map((s, i) => <li key={i}>{s}</li>)}
                      </ol>
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      )}

      <p className="text-center text-[11px] text-slate-400">
        {FAQ.length} SSS + {GLOSSARY.length} sözlük kaydı · Türkçe-duyarsız arama (fuse.js yok) · kaynak: data/faq.json + data/glossary.json
      </p>
    </div>
  );
}
