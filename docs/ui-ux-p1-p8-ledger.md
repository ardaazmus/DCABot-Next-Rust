# Ledger — P1–P8 inline uygulama (plan: in-chat bounded tasarım, onaylı)

No-git repo: worktree/commit adımları yok; ledger bu dosya. Spec: docs/UI_UX_ANALIZ_RAPORU.md + onaylı in-chat tasarım.
Ruling: TDD/verification sub-skill'leri bu ortamda yok → her task'ta `tsc --noEmit` + ilgili dosyanın derlenmesi kanıt sayılır.
Ruling: P2 token adları mevcut @theme ile çakışmayacak: base/surface/surface-2/border-strong (border, secondary, muted zaten var).
Pre-flight: task'lar arası paylaşılan arayüz yok (her biri farklı dosya/bölge; P3 sed'i P1/P2 sonrası çalışır — sıra: P6,P8,P1,P2,P3,P4,P7,P5).

Ruling: P7'de tablo zaten vardi (shadcn Table) — yeniden kurmak yerine eksik olan siralamayi (aria-sort) ekledim; kart gorunumu degismedi. Maliyet: yok.
Ruling: P5'te yuzde uydurulmadi — faz etiketli belirsiz cubuk + gercek AbortController iptali. apiBacktest/Cert'e opsiyonel signal eklendi (barrel re-export aynen gecer).
Ruling: text-slate-600 -> text-slate-500 da P3'e dahil edildi (uygulama always-dark; acik zemin yok). SVG fill renkleri bilerek disarida.
Task P6: complete (globals media query + motion.div useReducedMotion; tsc exit 0)
Task P8: complete (settings goz/kopya + role=status; tsc exit 0)
Task P1: complete (4 birincil + Diğer Drawer + desktop grup; tsc exit 0)
Task P2: complete (@theme base/surface/surface-2/border-strong; 20 dosya codemod, 0 kalan; tsc exit 0)
Task P3: complete (18 dosya micro-text slate-400; fleet istisnasi elle; tsc exit 0)
Task P4: complete (charts klavye + tablo + sr ozet; hp kaldirildi; tsc exit 0)
Task P7: complete (sirala + aria-sort; tsc exit 0)
Task P5: complete (faz cubugu + iptal + skeleton; api signal; tsc exit 0)
Final: tsc exit 0; eslint 0 error (globals.css ignored uyarisi); dev :3101 derleme temiz, 0 error.
Final: self-review (no subagent tool) — bulgu: P2'de token yazim hatasi sessiz CSS uretir riski; @theme adlari dosya + kullanim yerlerinde birebir eslesiyor (base/surface/surface-2/border-strong). Minor (deferred): ekran okuyucu turu + kontrast arac olcumu yapilmadi (rapor sinirlari).

Debug (systematic): dev overlay "global-error.js#default not in Client Manifest" — Faz 1: proje global-error YOK (builtin normal), hata Next'in kendi cumlesi (RSC bundler bug), tsc/eslint/route derlemesi temiz, ayni overlay onceki oturumda da gorulmus (webpack) -> kok neden .next Turbopack onbellegi (bayat lock + ust uste yazan surecler). Faz 3 tek hipotez, minimal test: :3101 durdur + .next sil + temiz baslat -> 200, 0 error, overlay yok. DOGRULANDI.
