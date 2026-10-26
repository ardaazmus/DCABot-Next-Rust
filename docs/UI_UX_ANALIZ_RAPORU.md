# DCABOT Terminal — UI/UX Analiz Raporu

**Yöntem:** `ui-ux-pro-max` skill'i (Quick Reference §1–10 + dataset sorguları: `fintech crypto trading terminal dark dense --design-system`, `trading dashboard dense data tables --domain chart`, `error summary validation focus --domain ux`).
**Kapsam:** `src/app/page.tsx` + `src/components/terminal/*` (18 dosya, ~11.500 satır) + `src/components/ui/*` (shadcn).
**Kod değişikliği YOK — bu rapor salt analiz + kurgu.**

Referans tasarım sistemi (dataset çıktısı): Minimalism & Swiss, dark-first, Inter, density 8/10 dashboard, motion subtle. Mevcut ürün bu çizgiye zaten yakın — öneriler ince ayar + erişilebilirlik kapatma niteliğinde.

---

## 1. Güçlü yanlar (korunacak)

| # | Bulgu | Dayanak |
|---|-------|---------|
| G1 | Emoji ikon yok, tek ikon dili (Lucide) | `no-emoji-icons` ✓; taramada emoji eşleşmesi 0 |
| G2 | Finansal sayılar tabular (`num` sınıfı) | `number-tabular` ✓ — fiyat/PnL titremesi yok |
| G3 | Renk + metin birlikte (GateChip PASS/HATA, StatusPill) | `color-not-only` ✓ |
| G4 | Yükleme ve hatalar duyuruluyor (`role=status`, `role=alert` ×3: calculator, dashboard, settings) | `aria-live-errors` kısmen ✓ |
| G5 | Nav'da `aria-current="page"`, mobil alt barda safe-area + `min-h-14` dokunma hedefi | `nav-state-active`, `safe-area` ✓ |
| G6 | Gerçek grafik motoru (lightweight-charts v5), özel SVG'lerde `role="img"` + `aria-label` | `chart-type` ✓ |
| G7 | Killswitch AlertDialog onayı | `confirmation-dialogs`, `destructive-emphasis` ✓ |
| G8 | Wizard'da `hidden` bölümlerle aşamalı açığa çıkarma, boş durumlarda hint ("Mesh boş…", "Olay bekleniyor…") | `progressive-disclosure`, `empty-states` ✓ |
| G9 | Code-split view registry — ilk yükleme küçük, görünüm başına spinner | `lazy-loading`, `bundle-splitting` ✓ |
| G10 | Formlarda görünür `Label`, secret'te `type=password` | `input-labels` ✓ |

---

## 2. Bulgular (öncelik sırasına göre)

### P1 — Mobil alt bar 7 öğe (HIGH · Navigasyon)
`page.tsx:36-44` — 7 view'un tamamı mobil alta diziliyor (`bottom-nav-limit` ≤5). `text-[9px]` + `max-w-[72px] truncate` ile "Bot Filosu", "Hesaplayıcı", "Backtest Lab" kısalıyor; keşfedilebilirlik düşüyor, yanlış dokunma artıyor.
**Kurgu:** 4 birincil (Kokpit · Filo · Yeni Bot · Lab) + "Diğer" (Hesaplayıcı/Bilgi/Ayarlar sheet'i). Desktop sidebar'da da aynı gruplama (İzleme / Kurulum / Sistem başlıkları — `nav-hierarchy`).

### P2 — 362 ham hex, token yok (HIGH · Stil/Tutarlılık)
`#0f1420 #10141d #1f2637 #2a3347 #0d1119` view'lere dağılmış; `bits.tsx`'te `ACCENT` token'ı var ama yüzey/border token'ı yok. Aynı "kart" 3–4 farklı border hex'i kullanıyor (`elevation-consistent`, `color-semantic` ihlali).
**Kurgu:** `surface / surface-2 / border / border-strong / text-1/2/3` token'ları (Tailwind `@theme` veya `globals.css`), ACCENT'i `success/danger/warn/info` semantiğine genişlet. Yeni koddaki ham hex lint ile yasaklanır.

### P3 — Mikro metin + kontrast riski (HIGH · Tipografi/Erişilebilirlik)
~190 `text-[9px]/[10px]/[11px]` kullanımı (wizard 72, settings 43, studio 48). İkincil metinlerde `text-slate-500` (#64748B) + koyu zemin ≈ **3.3:1** — küçük metinde 4.5 barajının altında (`color-accessible-pairs`).
**Kurgu:** Kural — mobilde gövde ≥12px, 10px yalnızca uppercase micro-label; küçük metinlerde `slate-500 → slate-400`. `line-length` 60–75 karakter kontrolü settings/bilgi metinlerinde.

### P4 — Grafiklerde hover-only + tablo alternatifi yok (HIGH · Grafik)
`charts.tsx:157-158` equity eğrisi yalnızca `onMouseMove`; klavye odağı (`tabIndex`), dokunmatik değer okuma ve veri-tablosu alternatifi yok (`tooltip-keyboard`, `data-table`, `screen-reader-summary` eksik). Donut (`:38`) aynı durumda.
**Kurgu:** Grafiğe "Veri tablosu" toggle'ı (sıralanabilir, `aria-sort`), klavyeyle gezilebilir noktalar (ok tuşları + `aria-valuetext`), `sr-only` Türkçe özet ("Son 30 gün: +X%, maks. düşüş Y%"). lightweight-charts (tv-chart) için konteynere `aria-label` + son fiyatın metin karşılığı (zaten header'da varsa yeterli — doğrulanacak).

### P5 — Uzun koşularda spinner + iptal yok (MEDIUM · Geri bildirim)
`backtest-lab.tsx`'te progress/cancel eşleşmesi yok; `ui/skeleton.tsx` mevcut ama terminalde **hiç kullanılmıyor**. Dakikalar süren backtest/cert koşusunda kullanıcı kör bekliyor (`progressive-loading`, `loading-states`, `timeout-feedback`).
**Kurgu:** Koşu fazlarını SSE'den ilerleme çubuğuna bağla (kline → koşu → skor), "İptal" butonu (AbortController), sonuç alanına skeleton.

### P6 — `prefers-reduced-motion` yok (MEDIUM · Hareket)
`dot-ping` + `animate-spin` globalde koşulsuz. Dataset motion kuralı: reduced-motion'da statik son durum.
**Kurgu:** `globals.css`'e tek media query (ping/spin/smooth-scroll kapat), dataset'in scroll-reveal deseni uygulanırsa `matchMedia` guard'ı.

### P7 — Filo/pano listeleri semantik tablo değil (MEDIUM · Veri)
`calculator.tsx` dışında `<table>` yok; filo satırları div'lerden kurulu. Ekran okuyucu satır/sütun bağlamını kaybediyor; sıralama varsa `aria-sort` yok (`data-table`, `sortable-table`).
**Kurgu:** Filo ve tahsis listelerini gerçek `<table>`'a taşı (görünüm aynı kalır), sıralanabilir sütunlar + `aria-sort`. Satır aksiyonları (Duraklat/Sil) menüde toplanırsa `overflow-menu` de çözülür.

### P8 — Secret'te göz simgesi yok (MEDIUM · Form)
`settings.tsx:248` `type=password` doğru ama göster/gizle düğmesi yok (`password-toggle`). Yapıştırma engeli yok — iyi (`accessible-authentication`), `autoComplete="off"` API secret için bilinçli istisna olarak not düşülmeli.
**Kurgu:** Göz toggle + "kopyalandı" onayı (`success-feedback`).

### P9 — Wizard 2448 satır tek dosya (LOW · Bakım, UX'e dolaylı)
Sekme/validasyon mantığı ile sunum iç içe; hata gösterimi GateChip satırlarında ama **submit sonrası odak yönetimi** (özet → ilk hatalı alan, `focus-management`/`error-summary`) kodda doğrulanamadı.
**Kurgu:** Bölme (adım bileşenleri + `useWizardState`), en üste odaklanabilir hata özeti + her madde hatalı alana linkli (dataset Result 1 deseni aynen).

### P10 — Küçükler (LOW)
- `KindBadge`/`StatusPill` renkleri `bits.tsx`'te tutarlı ama `sky-400` (FROZEN) token dışı — P2 ile çözülür.
- Toast'lar shadcn (canlı bölge varsayılanı iyi) — yıkıcı işlem sonrası "Geri al" yok (`undo-support`, örn. bot silme).
- `tv-chart` 529 satırda grafik + veri çekme + boş-durum bir arada; P9 tipi bölme adayı.

---

## 3. Önerilen yol haritası

| Faz | İş | Etki |
|-----|----|------|
| A (1–2 gün) | P1 mobil 5'li nav + P6 reduced-motion + P8 göz toggle + P3 metin kuralı | Yüksek görünürlük, düşük risk |
| B (3–5 gün) | P2 token sistemi (codemod ile hex → token) + P7 filo `<table>` | Tutarlılık + erişilebilirlik temeli |
| C (1 hafta) | P4 grafik erişilebilirliği (tablo toggle + klavye) + P5 backtest ilerleme/iptal/skeleton | En derin UX kazancı |
| D (sürekli) | P9/P10 bölmeler + wizard hata özeti odak yönetimi | Bakım + form a11y |

## 4. Doğrulanamayanlar (bilinçli sınırlar)
- Gerçek cihazda 375px/landscape davranışı (kodda `md:` kırılımları var, gözle test edilmedi).
- Ekran okuyucu turu (VoiceOver/TalkBack) yapılmadı.
- Wizard submit-sonrası odak akışı (P9) — kodda iz bulunamadı, çalışır test gerekir.
- Renk kontrastı hesapları hex matematiğine dayanır; `color-accessible-pairs` için araç ölçümü önerilir.
