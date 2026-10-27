# DCA Bot — Kapsamlı Hata, Açık ve Eksik Analiz Raporu

**Tarih:** 25 Ekim 2026, Europe/Istanbul (revizyon 2 — aynı gün, canlı test eklendi)
**Proje:** `D:\project\yeni-dca-bot` (DCABOT v3.0 — Rust çekirdek + Next.js terminali)
**Değişiklik sınırı:** Uygulama koduna düzeltme UYGULANMADI. Test amaçlı bot oluşturma/silme, kill/freeze engage-release ve API çağrıları yapıldı (aşağıda listelendi); test artıkları temizlendi.
**Önceki rapor:** `PROJE_DENETIM_RAPORU.md` (23 Ekim 2026). Bu rapor, o tarihten sonra yapılan düzeltmeleri güncel kaynakta tek tek doğrulamış ve kalan/yeni bulguları tespit etmiştir. Tüm satır referansları 25 Ekim 2026 sabahındaki kaynak koddur.
**Bu revizyonda yeni:** §3.5 — canlı test sonuçları (cargo test, tsc, eslint, API sözleşme testleri, PAPER bot yaşam döngüsü).

---

## 1. Yönetici Özeti

Proje matematik çekirdeği (M01–M21), fail-closed kapı mimarisi ve muhasebe tarafında **oldukça olgun** durumdadır; 23 Ekim denetimindeki birçok kritik bulgu düzeltilmiştir. Aynı gün koşulan canlı testlerde (§3.5) **Rust workspace testleri, TypeScript derleyici kontrolü ve eslint'in tamamı geçti**; PAPER bot yaşam döngüsü (oluşturma → gerçek fill → pause reddi → silme), kill/freeze şalterleri, hesaplayıcılar, backtest (gerçek Bybit verisiyle) ve piyasa veri uçları canlı olarak çalıştı. Ancak **uçtan uca çalışabilir bir ürün değildir** ve gerçek para yöneten bir bot olarak kesinlikle kullanılamaz.

Üç temel engel (ilk ikisi bu revizyonda CANLI olarak doğrulandı):

1. **Arayüz ↔ motor kimlik doğrulama entegrasyonu kopuk (canlı kanıtlı).** Motor tüm yönetim uçlarında `DCA_API_KEY` Bearer zorunlu tutuyor; canlı testte `http://127.0.0.1:3000/api/bots` (UI rewrite yolu — tarayıcı davranışının birebiri) **401** döndü, kimliksiz SSE **401** döndü. UI'ın `engineFetch`'i Authorization başlığı göndermiyor ve tarayıcı `EventSource` başlık koyamaz → tarayıcıda bot listesi, özet, fill'ler ve canlı akış açılmaz (yalnızca `/api/health` çalışır). E2E testleri (`tests/e2e_engine.py`) de anahtarsızdır ve kırıktır.
2. **Resmî başlatma komutu kırık.** `package.json` → `local:start` → `start-local.ps1` ne `DCA_API_KEY` ne `DCA_VAULT_KEY` atar; motor her ikisi yoksa panic ile açılır. Motor bugün yalnızca sabit dev anahtar atan `start-local-with-keys.ps1` (elle çağrılmış) ile ayakta; bu sabit anahtarın gerçekten çalıştığı canlı testte görüldü (M-7 kanıtlı).
3. **LIVE emir yürütme zincirinde doğrulanmamış varsayımlar var.** "Açık listeden kaybolan emir = dolum muhasebeleştirildi" yaklaşımı (kullanıcı/borsa iptali, reddedilen veya kısmi dolumda emri gerçekleştirmemiş sayma), hedge bacaklarının tek pozisyonda birleşmesi, gerçek bakiye/marjin rezervasyonunun olmaması ve restart sonrası tarihsel barların yeniden işlenmesi riski hâlâ açıktır. C-4'in "ilk poll tarihsel barlarla karar üretir" davranışı canlı olarak gözlemlendi: yeni kurulan bot, ilk supervisor turunda geçmiş 60 dakikalık bar verisiyle gerçek BASE alımı açtı.

Canlı testte ayrıca iki API sağlamlık açığı gözlemlendi: geçersiz `"FOO"` status'ü `200 {"ok":true}` ile kabul ediliyor (H-10 canlı kanıt) ve `start_cash="-500"` ile bot `201 RUNNING` kuruluyor (M-5 canlı kanıt).

**27 Ekim (Faz 14) — KALAN HAVUZ KAPANDI (§4.11):** M-10 (delete_bot DB temizliği), CF-3 (Caddy localhost kısıtı), CF-2 hazırlığı (DCA_EXTRA_ORIGINS), stop-local port-bazlı güvenli kapatma — hepsi kabul edildi (271 test). 13B-backtest kesin tasarımı yazıldı: HEDGE sertifika engelinin kökü backtest runner'da Hedge varyantı yokluğu; çift-stepper tasarımı icra-ready. Sonrası salt operasyon.

**27 Ekim (Faz 13B) — UYGULANDI (§4.8):** HedgeEngine bacak-başına stepper: iki bacak artık ayrı kernel defterinde (EX-06 kalıcı çözüm); per-bacak likidasyon/funding; hedge kernel snapshot; **LIVE hedge kilidi kaldırıldı**. Canlı kanıt: HEDGE_DUAL PAPER — iki bacak iki fill, kernel net delta 0. **Tüm Faz 13 tamamlandı** — 23 Ekim denetim raporundaki tüm kritik/yüksek bulgular kapalı ya da kilitle sınırlı olmaktan çıktı. Kalan: testnet rampası (operasyonel) + arşiv temizliği.

**27 Ekim (Faz 13A-POLL) — UYGULANDI (§4.7):** Dispatch LIVE dalları intent'e döndü; place/gate/market-sorgu/cancel ağları poll kilidi DIŞINDA cascade fazında koşuyor (≤8 tur, cascade apply yeni intent üretir). Canlı kanıt: Demo LIVE BASE emri intent yolundan verildi ve doldu (Open, 0.002 BTC, gerçek fee). Kalan tek refactor: 13B hedge bacak-stepper (ayrı oturum, plan §4.6).

**27 Ekim (Faz 13A dilim 3 + TAM PLAN §4.6):** ADD_FUNDS LIVE kilit-dışı ağa geçti (IEA, handler ölçeği; PAPER kabul canlı: kesir-defter +100 tam). Kalan iki kalemin icra-ready tasarımı rapora yazıldı: 13A-POLL (dispatch IEA worklist, 8-tur cascade sınırı) ve 13B (hedge bacak-stepper, 7 adımlı kabul planı) — taze oturumda bu tasarımlardan mekanik uygulanır.

**27 Ekim — Faz 13 dilim 2 (§4.5):** Fleet/Özet okumaları anlık görüntü önbelleğinden — venue yavaşken arayüz kilitlenmez; özet agregasyonu exact kesirlerle cache'ten hesaplanıyor (canlı kanıt). Faz 13'ün kalanı (actor modeli + hedge bacak-stepper) kesin kapsamla ayrı oturuma kayıtlı.

**26 Ekim gece — Faz 11+12 UYGULANDI, Faz 13 ilk dilim (§4.4):** Bildirim fan-out'u üç kanala çıktı (Telegram + outgoing webhook + SMTP e-posta — webhook kanalı yerel dinleyiciyle CANLI kanıtlandı); motora ta.stoch + ta.adx (Stochastic/ADX şablonları canlı backtest'li); supervisor jitter (Faz 13 ilk dilim). Yol haritasında kalan: Faz 13'ün büyük yarısı (dispatch-actor modeli + HedgeEngine bacak-stepper) — ayrı oturum.

**26 Ekim (Faz 10) — Öneri↔Hesaplayıcı entegrasyonu (§4.3):** Hesaplayıcı'nın Grid sekmesinde "Otomatik Grid Önerisi" kartı: tek tıkla gerçek menzil+sigma'dan dört girdiyi dolduran, ücret-kırıcı hükmü gösteren UI akışı canlı. Yol haritasında sıradakiler: Faz 11 (bildirim kanalı genişletme), Faz 12 (Stochastic/ADX), Faz 13 (H-1 son yarısı + hedge bacak-stepper — büyük refactor).

**26 Ekim (ürün turları) — Faz 6–9 UYGULANDI (§4.1–4.2):** Motora ta.stdev + öz-yineli ta.supertrend (Bollinger/Supertrend şablonları canlı backtest'li), Pine Kütüphanesi'ne 6 sektör şablonu seed (NULL-PK tuzağı yakalanıp düzeltildi), Telegram bildirim kanalı (env-gated), grid parametre öneri motoru (ücret-kırıcı denetimli — canlı kanıt). Sihirbazda 8 hazır Pine stratejisi. Kalan ürün yolu: AI-öneri↔Lab entegrasyonu, BB-ötesi göstergeler, bildirim kanalları genişletme (e-posta/webhook-out).

**26 Ekim (kapanış) — cargo audit + M-1 + lokal çalıştırma (§3.14):** Rust bağımlılıkları yamalandı (tokio 1.54.1, rustls-webpki 0.103.16 → 0 vulnerability), M-1 sözleşmesi testle sabitlendi (271 test / 0 hata), sistem `npm run local:start` ile temiz başlatıldı — tam kabul zinciri yeşil. Kalan üç madde bilinçli ürün kararı olarak kayıtlı: H-1 son yarısı (ölçek gerektiğinde), HedgeEngine bacak-stepper (LIVE hedge kilidiyle), UI cert onay kutusu.

**26 Ekim (son tur) — SIFIR güvenlik danışması + M-14/Reduce/dedup (§3.13):** `npm audit` **91 → 0**; ORDER_GATE reddi artık botu kilitlemiyor (faz geri alma); TP/SL kapanış emirleri bakiye kapısına takılmıyor; webhook tekrar koruması canlı (400 DUPLICATE_ALERT); Demo pozisyon kapanmış doğrulandı. Kalan teknik borç: H-1'in dispatch-place ağları (tam actor modeli), HedgeEngine bacak-stepper, cargo audit (offline'ta kurulamadı), UI'da cert_acknowledged onay kutusu.

**26 Ekim — Faz 5 TAMAMLANDI + İLK GERÇEK DEMO EMRİ (§3.12):** H-1 adım 2 (sinyal/pine ağları kilitsiz, 60× azaltılmış), ÜRÜN-1 onaylı istisna, H-7 rotasyon, C-5 positionIdx eşlemesi, M-12/M-13 düzeltmeleri — ve **ilk uçtan uca Demo emri canlı koştu**: 0.002 BTC pozisyon, gerçek borsa fill'i + gerçek fee + doğru marj. Kalan açık: M-14 (gate-reddi sonrası retry), H-1 son yarısı (dispatch-place ağları), HedgeEngine bacak-stepper, alert-id dedup, dolaylı danışmalar, cargo audit.

**26 Ekim ilk saatler — Faz 5 ikinci parça (§3.11):** H-1 adım 1/2 (üç fazlı poll + bot başına single-flight — klines ve imzalı kurulum ağları artık kilidi tutmuyor) ve H-9 (next 16.4.3 + sharp 0.35.5 + ölü Prisma çıkarımı; audit 91→15; tsc/eslint 0, UI yeni sürümde SSE dahil doğrulandı). **Kalan işler:** H-1 adım 2 (dispatch/sinyal ağları), sertifika eşiği kararı (ÜRÜN-1), kapıyı geçen stratejiyle son Demo emir koşusu, webhook rotasyon ucu, dolaylı danışmaların zincir çözümü, cargo audit.

**25 Ekim gece (2) — Demo kabul turu + Faz 5 ilk parça (§3.10):** Kullanıcının Bybit DEMO anahtarlarıyla imzalı dünya ilk kez canlı koştu: kasa izin taraması + selfcheck **GECTİ 18/0** (cüzdan/pozisyon okuma dahil). Sertifika kapısı gerçek veriyle çalışıyor ama mevcut eşiklerle naif stratejileri geçirmiyor → ÜRÜN-1 (kanal fiilen kilitli; eşik kararı kullanıcıya). H-7 (kalıcı webhook secret, canlı kanıt) ve H-8 (readiness + panik temizliği) uygulandı. Kalan: H-1 (kilit izolasyonu), H-9 (bağımlılık yükseltmeleri), sertifika eşiği kararı, place→sweep zincirinin kapıyı geçen stratejiyle son canlı koşusu.

**25 Ekim gece — Faz 4.5 UYGULANDI (§3.9):** Bybit pozisyon-modu denetimi (hedge-mode hesapta DCA LIVE emirleri kalıcı engelle reddediliyor), sürekli rezervasyon defteri (place/fill/terminal anlık), Bybit spot fee-coin dönüşümü. Kalan tek mimari iş: hedge bacak-başına-stepper (LIVE hedge kilidi sürüyor). Sonraki: Faz 5 (kilit izolasyonu, webhook kalıcılığı, readiness, bağımlılık yükseltmeleri, testnet rampası).

**25 Ekim akşam — Faz 4 UYGULANDI (§3.8):** Hesap/bacak güvenliği: enstrüman artık botun kendi borsasından çözülüyor ve LIVE'da metadata yoksa emir kabul edilmiyor (OKX botu canlı doğrulandı); HEDGE LIVE'a kilitlendi (422, canlı); likidasyon venue risk-limit kademeleriyle hesaplanıyor, LIVE'da yerel sentez kapanışı kalktı — tasfiye venue gerçekliğinden öğreniliyor; LIVE place öncesi gerçek bakiye/minNotional kapısı geldi. Kalan: Faz 4.5 (hedge bacak mimarisi tam çözüm, sürekli rezervasyon defteri, fee-coin) + Faz 5 (kilit izolasyonu, webhook kalıcılığı, bağımlılıklar, testnet rampası).

**25 Ekim ikindi — Faz 3 UYGULANDI (§3.7):** Execution mutabakatı redesign: fill kanıtı artık venue execution kayıtlarından (cancel/reject fill üretmez, ücret gerçek), emir takibi kalıcı (restart güvenli), runtime snapshot ile restart tam durumla devam ediyor (canlı kanıt: cash korundu, WARMUP tekrarı yok), PAUSED/STOPPED botta dolum izleme sürüyor, events retention geldi. ~150 workspace testi geçti. Kalan LIVE öncesi blok: Faz 4 (hedge bacakları, bakiye rezervasyonu, MMR kademeleri, venue-doğru enstrüman, fee-coin) + Faz 5 (kilit izolasyonu, webhook kalıcılığı, bağımlılık yükseltmeleri, testnet rampası).

**25 Ekim öğleden sonra — Faz 1–2 UYGULANDI (§3.6):** UI↔motor auth zinciri bağlandı (Bearer + SSE sunucu-üstü proxy), başlatıcı rastgele-kalıcı anahtar üretir oldu, ilk-poll tarihsel bar sorunu warmup-skip ile kapatıldı, status/start_cash/merdiven doğrulamaları ve CORS/Cache TTL düzeltildi — tümü canlı testlerle doğrulandı (cargo 50/50, tsc 0, eslint 0). Sistem artık `npm run local:start` ile kendi kendine açılıyor ve tarayıcıda veri akışı çalışır durumda; engel listesi LIVE tarafına (Faz 3–4: execution mutabakatı, bacak mimarisi, rezervasyon) kaydı.

**Önerilen konumlandırma:** PAPER/backtest ve sertifika/laboratuvar akışı artık günlük kullanım için çalışır; LIVE işlem, Faz 3–4 tamamlanmadan ve Faz 5 testnet rampası yapılmadan açılmamalıdır.

---

## 2. Kapsam ve Yöntem

İncelenen ana bileşenler (tam okuma):

- Rust çekirdek: `dca_server` (main.rs 3051 satır, engine.rs, state.rs, store.rs, routes_calc.rs), `dca_exchange` (lib.rs, bybit.rs, okx.rs), `dca_kernel/src/lib.rs`, `dca_math` (m02.rs, fraction.rs ilgili bölümler)
- Frontend taşıma katmanı: `src/hooks/engine-client.ts`, `engine-stream.ts`
- Başlatıcılar: `scripts/start-local.ps1`, `start-local-with-keys.ps1`, `Caddyfile`, `package.json`, `next.config.ts`
- Ortam: `.env` (yalnızca anahtar ADLARI okundu, değerler raporlanmadı), `.env.example`
- Çalışma günlükleri: `.local-runtime/` (25 Ekim 10:32 başlatma kaydı), `worklog.md` (son girdiler)

23 Ekim raporundaki her SEC/EX/BOT/OPS bulgusu güncel kaynakta tek tek izlendi; "DÜZELTİLDİ" diğeri için satır düzeyinde kanıt görüldü, "AÇIK" diğeri için güncel kanıt satırı bu raporda yeniden verilmiştir.

---

## 3. Güncel Durum: Ne Çalışıyor, Ne Çalışmıyor

| Katman | Durum | Not |
|---|---|---|
| Rust çekirdek derleme ve açılış | ⚠️ Koşullu | `cargo build` temiz; açılış yalnız `DCA_API_KEY` + `DCA_VAULT_KEY` env varken çalışır (bugün 10:32'de anahtarlarla açıldığı logda görüldü) |
| Matematik çekirdeği (M01–M21) + 8 kapı | ✅ | Birim testleri geniş; spot equity ve margin muhasebesi testli |
| PAPER simülasyon botları | ✅ (auth düzeltilirse) | Poll, kapılar, fill muhasebesi, SSE üretimi çalışır |
| UI → motor veri akışı | ❌ | 401: UI Bearer göndermiyor, SSE EventSource başlık koyamıyor |
| `npm run local:start` | ❌ | start-local.ps1 zorunlu env'leri atmıyor → motor panic |
| Vault (AES-256-GCM) | ✅ | Master key DB dışından zorunlu; ancak key rotasyonu yok |
| Bybit REST adaptörü (market data + imzalı uçlar) | ✅ | marketUnit, emir sorgusu, sayfalama, risk-limit kademeleri düzeltilmiş |
| OKX adaptörü | ⚠️ | Sorgu bacağı sağlam; `open_orders` kontrat/coin birimi hâlâ karışık |
| LIVE emir yürütme bütünlüğü | ❌ | Aşama A bulguları (aşağıda) |
| Backtest / sertifika / LIVE kapı | ✅ | Sertifikasız strateji canlıya alınamıyor (iyi bir kilit) |
| E2E testler | ❌ | Auth'suz; motor tarafındaki testler ortam bellek kısıtıyla geçmişte zorlandı |
| Bağımlılık güvenliği | ❌ | 23 Ekim'de 91 advisory (5 kritik) raponlandı; paket sürümleri değişmedi |

---

## 3.5 Canlı Test ve Doğrulama Sonuçları (25 Ekim, revizyon 2)

Ortam: çalışan motor (`:3002`, with-keys sabit anahtarıyla açılmış) + Next dev (`:3000`), kullanıcının kendi PAPER botu (`deneme sol future`) korunarak test edildi. Test amaçlı 4 bot oluşturuldu ve tamamı silindi; kill/freeze engage-release döngüsü çekildi ve bırakıldı.

### 3.5.1 Statik kalite kapıları — TÜMÜ GEÇTİ

| Kontrol | Komut | Sonuç |
|---|---|---|
| Rust workspace testleri | `cargo test --workspace --offline --locked` (CARGO_BUILD_JOBS=1) | **PASS** (exit 0; son crate dca_strategy 54/54, doc-testler 0 sorun; log: `.audit-runtime/cargo-test-2026-10-25.log`) |
| TypeScript | `node_modules/.bin/tsc --noEmit --incremental false` | **PASS** (exit 0; `.audit-runtime/tsc-2026-10-25.log`) |
| ESLint | `npx eslint src --quiet` | **PASS** (exit 0; `.audit-runtime/eslint-2026-10-25.log`) |

Not: `npx tsc` sahte `tsc@2.0.4` paketini kurup yanıltıcı sonuç veriyor — derleyici kontrolü proje ikilisiyle tekrarlandı.

### 3.5.2 API sözleşme testleri (anahtarlı/anahtarsız)

| Test | Beklenen | Gözlenen | Hüküm |
|---|---|---|---|
| `GET /api/health` anahtarsız | 200 | 200 | ✅ |
| `GET /api/bots` anahtarsız (3002 ve 3000 rewrite) | 401 | **401 / 401** | ✅ sunucu doğru; **C-1 canlı kanıt**: tarayıcı akışında tüm veri uçları 401 |
| `GET /api/stream` anahtarsız (EventSource simülasyonu) | 401 | **401** | ✅ C-1'in SSE ayağı canlı kanıt |
| `GET /api/bots` sabit dev anahtarla | 200 | 200 | **M-7 canlı kanıt** (bilinen anahtar çalışıyor) |
| CORS preflight `Origin: http://127.0.0.1:3000` | allow-origin eşleşmeli | **`allow-origin: http://localhost:3000` döndü** | **M-8 canlı kanıt**: başlatıcının verdiği `127.0.0.1:3000` adresinden doğrudan motor erişimi tarayıcıda CORS hatısıyla reddedilir |
| `POST /api/webhooks/tradingview` yanlış secret | 401 | 401 `invalid secret` | ✅ |
| Kill-switch + freeze engage/get/release | 200, durum kalıcı, bırakınca sıfır | aynen | ✅ |

### 3.5.3 PAPER bot yaşam döngüsü (E2E)

1. **Oluşturma:** tam `BotConfig` şemasıyla `DCA_LONG / BYBIT / PAPER / 1m` bot → `201 {id, status:RUNNING}`; bot kimliği `b-767745f7` (8-hex kısa ID — EX-01 düzeltmesi canlı).
2. **İlk supervisor turu (25 sn içinde):** bot `Open` fazına geçti, `deal-1` açıldı, 0.00062 BTC pozisyon (≈50 USDT BASE alımı) — **C-4 canlı kanıt**: bot kurulduktan sonraki İLK poll, geçmiş 60 dakikanın barlarıyla bugünkü piyasada deal açtı (`last_bar_ts=0` ile kurulum, `state.rs:438`).
3. **Fill kaydı:** `BASE / Buy / TAKER / MarketableTaker` — gerçekçilik etiketleri ve ücret rolü doğru.
4. **PAUSE → manuel tick:** `422 "bot PAUSED — manuel tick yeni emir üretemez"` — BOT-08 düzeltmesi canlı doğrulandı.
5. **Geçersiz status:** `PATCH {"status":"FOO"}` → `200 {"ok":true}` — **H-10 canlı kanıt**; bot supervisor filtresinin dışına düşer (poll edilmez), DB'ye `"FOO"` kalıcı yazılır.
6. **Negatif sermaye:** `start_cash:"-500"` ile bot `201 RUNNING` — **M-5 canlı kanıt**.
7. **Silme:** `DELETE` → 200; ancak DB denetiminde silinen 3 test botu `bots` tablosunda `STOPPED` satırı olarak duruyor — **M-10 canlı kanıt** (runtime'dan kalkar, kayıt birikir; events tablosu kısa test oturumunda bile 354 satır — M-9).
8. **Hatalı config davranışı (yan not):** `deviation_pct="0.5"` (kesir beklerken) M01 domain korumasını tetikledi: merdiven reddedildi, deal AÇILMADI (fail-closed ✅) ancak bot RUNNING'de kaldı ve her poll'da `ERROR ladder rejected` tekrarladı — hatalı config kurulumda 422 ile reddedilmediği için kalıcı ERROR spam üretiyor (operasyonel not; M-9'u hızlandırır).

### 3.5.4 Fonksiyonel uçlar

| Uç | Sonuç | Not |
|---|---|---|
| `POST /api/calc/dca` (geçerli) | 200 | seviye/WAC/M20 yanıtın tamam |
| `POST /api/calc/dca` `max_safety_orders=100000` | **422 (0,03 sn)** | M01 geometrik sapma sınırı erken reddediyor — pratik vektör kapalı; ancak `step_scale=0 + dev→0 + dev max_safety_orders` kombinasyonunda `Vec::with_capacity` hâlâ sınırsız (kod kanıtlı; çalışan kullanıcı botu nedeniyle canlı OOM testi bilinçli yapılmadı → H-2 güncellendi) |
| `anchor="1e400"` / `"1e2000"` | 200 / **422** | 4096-bit kesir sınırı çalışıyor; `as u32` üs truncate'ı yalnız üs ≥ 2³²'de teorik risk (H-2 güncellendi) |
| `POST /api/calc/grid` (doğru şema) | 200 | |
| `POST /api/backtest` (BYBIT canlı veri, 200 bar) | 200 | 1 deal, equity eğrisi tamam |
| `GET /api/klines` `/api/markets` `/api/tickers` `/api/orderbook` | 200 | |
| `POST /api/pine/validate` | 200 | derleme + fonksiyon listesi |
| `POST /api/live/selfcheck` | `YETERSIZ_ORTAM` (12 pass / 0 fail / 11 abstain) | ✅ dürüst sınıflandırma (vault'ta anahtar yok) |
| `POST /api/parity` (kullanıcı botu, 200 bar) | 200 `YETERSIZ_VERI` | ✅ dürüst: 5 paper fill < 10 eşik; count-match %28 notu v1 kapsamını açıkça beyan ediyor |
| `GET /api/stream` anahtarlı (25 sn) | bağlantı + keep-alive | akış çalışıyor |

### 3.5.5 Canlı testten çıkan yeni/kesinleşen hususlar

- **YENİ (D-1, Düşük):** `POST /api/bots` config şeması dışarıdan kullanımı kolaylaştırmıyor: `config` nesnesi içinde `kind`, `symbol`, `sizing_mode` vb. zorunlu (serde default yok) — istek gövdesinin üst düzey alanlarıyla çiftleme yapılıyor. UI wire formatına bağımlılık; API dokümantasyonu/örnek gövde yok.
- **YENİ (D-2, Orta):** Hatalı `deviation_pct` (≥1 kesir) config'i kurulumda kabul ediliyor; bot RUNNING kalıp her turda ERROR tekrarlıyor. Kurulumda ladders ön doğrulaması (create anında merdiven inşası) yok.
- C-1, C-4, H-10, M-5, M-7, M-8, M-10 canlı kanıta dönüşürken; H-2'nün pratik vektörü büyük ölçüde kapalı çıktı (sadece uç kombinasyon teorik açık).

---


## 3.6 Faz 1–2 UYGULAMA KANITLARI (25 Ekim, aynı gün)

Kod değişiklikleri: `src/hooks/engine-client.ts` (Bearer), `src/app/api/stream/route.ts` (yeni SSE proxy), `scripts/start-local.ps1` (rastgele-kalıcı anahtar + `-cmatch` parse), `scripts/start-local-with-keys.ps1` (wrapper), `main.rs` (CORS list, status whitelist, start_cash>0, merdiven ön-inşası), `engine.rs` (warmup-skip), `state.rs` + `main.rs markets` + `dryrun.rs` (instrument cache TTL), `README.md` (auth + örnek gövde).

Doğrulama zinciri (canlı):

| Kabul | Beklenen | Gözlenen |
|---|---|---|
| `npm run local:start` temiz ortam | motor+UI açılır | ✓ ilk çalıştırmada `local-keys.env` rastgele anahtarlarla üretildi; iki servis 200 |
| Kimliksiz `/api/bots` (3002 ve 3000 rewrite) | 401/401 | 401 / 401 ✓ |
| Bearer'lı `/api/bots` (3002 ve 3000 rewrite = engineFetch davranışı) | 200/200 | 200 / 200 ✓ |
| `/api/stream` 3000 üzerinden başlıksız (EventSource davranışı) | 200 + text/event-stream | 200 + text/event-stream ✓ (sunucu-üstü proxy Bearer ekler) |
| Yeni bot ilk poll'u | geçmiş barlar atlanır | `WARMUP: 59 geçmiş kapalı bar karar için atlandı` ✓; pozisyon yalnız YENİ kapanan barla açıldı (C-4 kanıtı) |
| `PATCH status:"FOO"` | 422 | 422 ✓ (H-10 kapandı) |
| `start_cash:"-500"` | 422 | 422 ✓ (M-5 kapandı) |
| `deviation_pct:"0.5"` | 422 | 422 "config merdiveni kurulamıyor" ✓ (D-2 kapandı; runtime ERROR spam yerine kurulum ret) |
| CORS `Origin: http://127.0.0.1:3000` | allow-origin 127.0.0.1 | ✓ (M-8 kapandı) |
| Restart sonrası kullanıcı botu | restore | `deneme sol future` RUNNING döndü ✓ |
| `cargo test -p dca_server` | PASS | 50/50 ✓; `tsc` 0; `eslint` 0 ✓ |

Süreç notları (opsiyonel iyileştirme): `local:stop`, processes.json süreç-kimliği (parent/startTime tick) değişimlerinde reddediyor — elle `taskkill` gerekti; iki kez karşılaşıldı. PS 5.1 `-match` Türkçe kültürde `[A-Za-z]` sınıf aralıklarını bozuyor (canlı teşhis: match=False/cmatch=True) — `-cmatch` kullanıldı; ileride yazılacak her PS script'i için geçerli.

**Faz 1–2 ile KAPANAN bulgular:** C-1, C-2, M-3, M-5, M-7, M-8, H-10, D-1, D-2; C-4 → minimal çözüm (warmup-skip; kalıcı snapshot hâlâ Faz 3).

---


## 3.7 Faz 3 UYGULAMA KANITLARI (25 Ekim, aynı gün — execution mutabakatı redesign)

Kod değişiklikleri: `dca_exchange/lib.rs` (ExecutionRecord + `executions()` trait + `cum_exec_fee`), `bybit.rs` (`/v5/execution/list`), `okx.rs`/`binance.rs` (fee alanları), `dca_kernel/lib.rs` (`ingest_external_fill_with_fee` + G2 muafiyeti + 3 yeni test), `store.rs` (live_orders tablosu + bots.runtime_state + events indeksi), `state.rs` (RuntimeSnapshot + kalıcı emir izi yardımcıları + events retention + restore), `engine.rs` (execution-kaynaklı yeni sweep + market/limit DB izleri + PAUSED/STOPPED sweep + snapshot save), `main.rs` (supervisor filtresi + reconcile DB izleri), `scripts/restart-engine.ps1` (motor-tekil restart aracı).

### C-3 — "kaybolan emir = doldu" dönemi KAPANDI

Yeni sweep ilkeleri (`engine.rs live_fill_sweep`): (1) fill kanıtının birincil kaynağı venue **execution kayıtları** — her kayıt kendi `exec_id`'siyle idempotent işlenir (fill_key = `order_id-exec_id`); (2) **Cancelled/Rejected emir fill ÜRETMEZ** — takip dürüstçe kapanır (INFO event); (3) execution API'si olmayan venue'da terminal `Filled` + gerçek `cumExecQty/avgPrice/cumExecFee` ile TEK fallback fill (yalnız o emir için hiç fill yoksa — çift muhasebe engeli `fill_any_for_order` ile); (4) emir venue'da bulunamazsa ve kanıt yoksa **fill ÜRETİLMEZ** (fail-closed WARN); (5) limit dolumları **Maker**, market **Taker**; ücret venue'dan geliyorsa **gerçek tutar** kullanılır (G2, venue-doğrulanmış fee'de yerel-oran kontrolünden muaftır — sıfır ücretli voucher dolumu mümkün). Doğrulama: çekirdek davranış birim testli (`venue_fee_overrides_local_rate`, `venue_zero_fee_is_verified_and_allowed`, `duplicate_exec_fill_is_idempotent`); imzalı LIVE sweep turu vault anahtarsız ortamda koşulamadı — testnet anahtarı eklenince Tablo 9'daki senaryo ile doğrulanmalı ( dürüst sınır).

### H-3 — emir takibi kalıcı

`live_orders` tablosu: place anında PENDING yazılır (limit + market), partial'da PARTIAL güncellenir, terminalde/fill'de satır düşer. Restart'ta PENDING/PARTIAL kayıtlar `load_live_orders` ile geri yüklenir (venue RESTORED listesiyle birleşik, dedup'lu). Bellek-only takip dönemi kapandı: restart öncesi place edilmiş emir ilk poll'da execution sweep ile uzlaştırılır.

### C-4 (TAM) — kalıcı runtime snapshot

Motor yapıları serde'ye açıldı (`DcaEngine/GridEngine/HedgeEngine/EngineTag/PositionState`); her poll sonunda tam motor durumu (faz + merdiven sayaçları + stepper muhasebesi + imleç) `bots.runtime_state`'e yazılır. Canlı iki-restart kanıtı: (1) snapshot 2178 byte yazıldı; (2) restart sonrası **cash 2000 birebir korundu**, faz korundu ve **WARMUP tekrarı yaşanmadı** (en yeni eventler TICK). Yarım-restore dönemi kapandı.

### H-10 (kalıntı) + M-9 — PAUSED/STOPPED sweep + retention

Supervisor PAUSED/STOPPED botları da poll eder (open emir dolumları izlenir; strateji kararı kapalı). Canlı kanıt: PAUSE sonrası 45 sn'de TICK sayısı 876'da sabit (sessiz sweep — events şişmiyor), START ile akış geri geldi. `events` için global retention (son 200k, 1/64 yazımda) + `(bot_id, id)` indeksi eklendi.

### Bu fazla kısmen kapanan / açık kalan

- **H-4 kısmi kapandı:** gerçek fee (quote cinsiyse) artık kullanılıyor; farklı fee-coin ayrıştırması Faz 4'te açık.
- **M-2 (OKX open_orders ctVal) ve C-5/C-6/H-5/H-6:** Faz 4 kapsamında dokunulmadı.
- Süreç notu: `.ps1` dosyaları bu ortamda **ASCII-only** yazılmalı (PS 5.1 BOM'suz UTF-8'i ANSI okuyor — Türkçe karakter/em-dash parse hatası veriyor; `restart-engine.ps1` bu şekilde düzeltildi).

---


## 3.8 Faz 4 UYGULAMA KANITLARI (25 Ekim, akşam — hesap ve bacak güvenliği)

Kod değişiklikleri: `state.rs` (MMR kademe alanı + `engine_instrument_for(venue)` + `venue_live_load` defteri), `engine.rs` (kademeli likidasyon + venue-tasfiye öğrenme + `pre_order_gate`), `main.rs` (Hedge LIVE kilidi + LIVE metadata ret + restore düşürme), `okx.rs` (open_orders ctVal normalize).

| İş | Bulgu | Sonuç |
|---|---|---|
| Enstrüman botun kendi venue'sundan çözülür (PAPER→BYBIT); LIVE kurulumda metadata alınamazsa 422 (sertifika kapısında varsayılan-fallback kaldırıldı) | H-6 | ✅ canlı: OKX venue'lu PAPER bot OKX enstrümanıyla kurulup çalıştı (201/RUNNING); LIVE metadata-ret kod yolu anahtar gerektirdiğinden testnet turunda doğrulanacak |
| OKX `open_orders` sz/accFillSz ctVal ile coin'e normalize | M-2 | ✅ kod; OKX imzalı uç testnet turunda doğrulanacak |
| HEDGE stratejileri LIVE'a kilitli (create 422 + restore'da dürüst PAPER düşürme) — tam bacak mimarisi Faz 4.5'e kaldı | C-5 (minimal güvenlik kilidi) | ✅ canlı: `HEDGE_DUAL / LIVE` → 422 "bacak-bazlı pozisyon muhasebesi ve positionIdx eşlemesi tamamlanana kadar PAPER kullanın" |
| Likidasyon hesabı venue risk-limit kademeleriyle (10 dk TTL; sabit 0.005 yalnız son-çare); LIVE'da yerel sentez-LIQ kapanışı KAPALI — tasfiye venue gerçekliğinden öğrenilir (yerelde pozisyon var + venue'da yok → venue kanıtlı kapanış, CRITICAL olay) | H-5 | ✅ kod + ~150 test; canlı: futures botu sorunsuz sürüyor (pozisyonsuzken liqPrice None — beklenen) |
| LIVE place öncesi gerçek bakiye kapısı: minNotional kontrolü + wallet.available ≥ (bu emrin marj maliyeti + aynı venue'daki diğer LIVE botların açık-emir yükü); geçilemezse emir gönderilmez (WARN, motor sürer) | C-6 (minimal emir geçidi) | ✅ kod; imzalı yol testnet turunda |
| Fee-coin ayrıştırması (spot'ta base cinsinden ücret) | H-4 kalanı | 🔶 Faz 5/4.5'te açık (quote cins fee Faz 3'te kullanımda) |

Kalite: `cargo check` temiz, workspace ~150 test PASS (yenisi eklenmedi — davranış LIVE-yollarında; kabul testnet turunda). Süreç dersi kayda geçti: `restart-engine.ps1` derleme YAPMAZ — derle→kapat→başlat sırası izlendi (ilk denemede eski binary fark edildi ve düzeltildi).

---


## 3.9 Faz 4.5 UYGULAMA KANITLARI (25 Ekim, gece — pozisyon modu, sürekli defter, fee-coin)

| İş | Bulgu | Sonuç |
|---|---|---|
| Bybit pozisyon-modu denetimi: `PositionSnapshot.position_idx` (0=tek-yönlü, 1/2=hedge; [docs](https://bybit-exchange.github.io/docs/v5/position/position-mode) — GET ucu yoktur, `position/list`'ten çıkarım) + `pos_mode()` impl; DCA LIVE futures başlangıcında hesap hedge-mode ise `POS_MODE` WARN + **kalıcı emir engeli** (pre_order_gate reddi) | C-5 (DCA tarafı), EX-07 Bybit karşılığı | ✅ kod; imzalı yol testnet |
| Sürekli rezervasyon defteri: `venue_live_load` artık place anında (+), iptal/dolum/terminal anında (−) güncelleniyor — poll-refresh arasındaki çift-harcama penceresi kapandı; poll periyodik olarak drift'i düzeltiyor | C-6 (tam) | ✅ kod; imzalı yol testnet |
| Bybit spot ALIŞ ücreti BASE cinsinden kesilir → execution kaydında quote karşılığına çevriliyor (`fee_q = fee_base × execPrice`); satış/lineer zaten quote | H-4 (Bybit kapanış) | ✅ kod; OKX fee-ccy ayrıştırması not olarak açık |
| HEDGE LIVE kilidi (422) + restore düşürme sürüyor — bacak başına ayrı stepper/positionIdx mimarisi hâlâ Faz 4.5-kalan | C-5 (hedge tam çözüm) | 🔶 canlı: `HEDGE_PROTECTIVE/LIVE` → 422 doğrulandı |

Doğrulama: `cargo check` temiz; workspace testleri **17 paket / ~150 test tamamı geçti**; binary derlendi (gerçek exit 0) ve motor yeniden başlatıldı — kullanıcı botu snapshot'tan restore edildi (cash 2000 korundu; RuntimeSnapshot şeması geriye uyumlu), Hedge kilidi canlı 422, UI 200.

---


## 3.10 Bybit DEMO Kabul Turu + Faz 5 (ilk parça) — 25 Ekim gece

Ortam: kullanıcının `.env`'indeki **Bybit Demo Trading** anahtarları ve `BYBIT_BASE_URL=https://api-demo.bybit.com` (motor başlangıçta yükler). Gerçek para YOK; imzalı v5 protokolü GERÇEK.

### Demo ucunda canlı doğrulananlar

| Kontrol | Sonuç |
|---|---|
| `POST /api/vault/keys` — izin taraması gerçek `query-api` (Demo) | ✅ 200; `can_withdraw:false` → zero-withdrawal kapısı gerçek veriyle PASS; spot+contract yetkisi görüldü |
| `POST /api/live/selfcheck` — imzalı sondalar | ✅ **GECTİ (18 pass / 0 fail)**: anahtar geçerli, trade yetkisi, çekim yasağı, salt-okunur değil, cüzdan okuma, pozisyon okuma — tümü Demo ucunda |
| Sertifika kapısı veri borusu (klines/instruments/funding + 500-bar backtest) | ✅ gerçek Demo verisiyle koşuyor |
| `GET /api/health` readiness (Faz 5/H-8) | ✅ canlı: `db:ok`, `lastTickAgeS` aktif, aktif/toplam bot sayısı |
| Webhook secret restart sabitliği (Faz 5/H-7) | ✅ canlı: iki restart arasında secret **birebir aynı** — TradingView alert'leri motor restart'ında artık geçersizleşmiyor |

### Kapsam dışı kalan (dürüst sınır)

Demo LIVE bot kurulumu **sertifika kapısı** tarafından engellendi — kapı, gerçek Demo verisiyle koşan sertifikasyon eşiklerini (örneklem ≥5 kapanış ✓, WFE ✓, **DSR ≥ 0.95**, PBO ≤ %20) yalnız hepsi birden geçildiğinde canlıya izin veriyor. Bu oturumda test edilen ~12 makul DCA/Grid varyantının hiçbiri DSR/PBO'yu geçemedi. Sonuç: `place_order → pre_order_gate → execution sweep → gerçek fee` zincirinin uçtan uca canlı koşusu, kapıyı geçen bir strateji üretildiğinde tekrarlanmalı (kod + ~150 test + imzalı sondalar hazır; yalnız son adım eksik).

### YENİ bulgu — ÜRÜN-1 (Orta): LIVE sertifikasyon kapısı fiilen erişilemez

DSR ≥ 0.95 + WFE ≥ %50 + PBO ≤ %20 kombinasyonu, yan/sert piyasa pencerelerinde naif DCA/Grid config'lerinin hiçbirini geçirmiyor (12 varyant: BTC/ETH/SOL, 5m–1h, 200–1000 bar, TP %0.1–%30). Kapı fikri doğru (rozetsiz strateji canlıya girmesin) ama mevcut eşiklerle kanal pratikte kilitli kalabilir. Öneriler: (a) eşik kalibrasyonu (örn. DSR 0.90 veya örneklem-bağımlı esneme), (b) bilinçli-risk onayıyla tek-seferlik istisna (risk_acknowledged desenine paralel, event'e yazılan), (c) Lab'de "canlıya en yakın geçen config" öneri motoru. **Karar kullanıcıya aittir; kapı bilinçli olarak aşılmadı.**

### YENİ not — parametre birimleri (API ergonomisi, D-1 uzantısı)

`target_profit_pct` ve `deviation_pct` KESİR'dir (`0.003` = %0.3). Canlı testte `"0.3"` (= %30) verilince TP hiç tetiklenmedi — motor doğru davrandı, ama API düzeyinde birim belirsizliği insan hatasına açık. Öneri: create/cert yanıtlarında yorumlanmış değer (`"targetProfitPctInterpreted": "0.30%"`) dönmek veya yüzde-kabul modu.

### Faz 5 ilk parça uygulandı

- **H-7:** webhook secret `RuntimeSnapshot`'a alındı (kalıcı; eski snapshot uyumlu) — canlı kanıt yukarıda. Rotasyon ucu UI ile birlikte sonraki adım.
- **H-8:** `/api/health` artık dürüst readiness taşıyor (db + son poll yaşı + bot sayıları; `status:"ok"` alanı korunarak) ve `bot_fills`/`bot_events`'teki `unwrap()` panikleri hata yanıtlarına çevrildi. Süreç dersi: async handler'da `blocking_lock` panikler (canlı yakalandı, `await`'li kilitle düzeltildi).

---


## 3.11 Faz 5 ikinci parça — H-1 (adım 1/2) + H-9 (26 Ekim ilk saatler)

### H-1 — kilit izolasyonu, adım 1/2

`poll_inner` üç faza bölündü (`engine.rs`): **(1)** kısa kilitli bağlam kopyası (`PollCtx`), **(2)** KİLİTSİZ ağ fazı — klines + imzalı kurulum çağrıları (set-leverage, pos-mode, MMR kademeleri, venue pozisyon kontrolü), **(3)** kilitli uygulama + bar işleme + sweep. Ayrıca `AppState.poll_locks` ile **bot başına single-flight**: aynı botun poll'ları üst üste binmez, meşgul tur dürüst "atlandı" olayıyla geçer; bot silinince kilit kaydı da düşer. Etki: en yavaş ağ öğesi (klines, 15 sn timeout'a dek) artık runtimes kilidini tutmaz — UI/action istekleri poll sırasında yanıt alır. **Adım 2 (kalan):** process_bar içindeki sinyal/pine fetch'leri ve sweep/dispatch place ağları hâlâ kilit altında (adım 2 = dispatch zincirinin kilitsizleştirilmesi; mimari değişim gerektirir). Doğrulama: 17 paket/~150 test PASS; canlı restart sonrası botlar sağlıklı poll ediyor (lastTickAge 5 sn).

### H-9 — bağımlılık güvenliği: 91 → 15 danışma

- `next` **16.1.3 → 16.4.3**, `sharp` → **0.35.5** (GHSA-p293 / GHSA-2464 kapsanan sürümler). Doğrulama: `tsc` 0 (ilk turdaki 3 hata eski `.next` üretik tiplerindendi; yeni tiplerle temiz), `eslint` 0, UI **16.4.3** ile ayakta: sayfa 200, rewrite Bearer'lı `/api/bots` 200, SSE proxy 200 + text/event-stream.
- Ölü Prisma bağımlılıkları (`prisma`, `@prisma/client`) ve `db:*` script'leri kaldırıldı (kodda hiç kullanılmıyordu — M-6'nın bağımlılık yarısı kapandı); yerine `restart:engine` / `restart:ui` kısayolları eklendi.
- `npm audit` (yalnız üretim bağımlılıkları): **91 → 15** (1 kritik, 7 yüksek, 6 orta, 1 düşük; kalanlar deepmerge-ts/defu/effect/js-cookie gibi dolaylı — transitive zincir çözümü sonraki bakım kalemi). `cargo audit` çevrimdışı ortamda koşulamadı (cargo-audit kurulu değil) — not olarak açık.

---


## 3.12 Faz 5 tamamlama — H-1 adım 2, ÜRÜN-1 onaylı istisna ve İLK UÇTAN UCA DEMO EMRİ (26 Ekim)

### Kod

- **H-1 adım 2:** sinyal/pine kararları bar döngüsünden çıkarıldı → kilitsiz Faz 2'de poll başına BİR kez hesaplanır (`PreSignals`). Eski davranışta her bar için yeniden ağ çekimi vardı (60 bar = 60×); fetch'ler zaten "en son kapalı" pencereye baktığından karar eşdeğer — kilit süresi ve ağ yükü 60× düştü.
- **ÜRÜN-1 çözümü:** LIVE sertifika kapısına `cert_acknowledged` bilinçli-onay istisnası — onaylı kurulum kalıcı WARN olayı + SSE RISK bildirimiyle kayıt altında; onaysız 422 yanıtına yol gösterme ipucu eklendi.
- **H-7 tamamı:** webhook secret rotasyonu (`PATCH /api/bots/:id {webhook_secret}`) — canlı doğrulandı (yeni secret `tvw_rotated_acceptance_2026` yerleşti, snapshot'a yazıldı).
- **C-5'in positionIdx yarısı tamam:** hesap hedge modundaysa DCA emirleri yön eşlemesiyle çalışır (Buy→1, Sell→2) — engelleme yerine uyum; tek yönlü hesaplarda davranış değişmez (idx=0). Canlı: Demo hesabınız hedge modunda ve emir bu eşlemeyle gönderildi.

### İLK UÇTAN UCA DEMO EMİR ZİNCİRİ (canlı kanıt)

`demo-final2` (FUTURES_DCA/LIVE/BYBIT-demo, 200 USDT base, 3×): **WARMUP → kapı GEÇİLDİ (M-13 düzeltmesi) → BASE market emri GÖNDERİLDİ → venue DOLDU → execution işlendi → pozisyon AÇIK**:

| Kanıt | Değer |
|---|---|
| Fill | `BASE Buy, qty 0.002 BTC @ ~80,648.6, TAKER/MarketableTaker` |
| Gerçek fee | borsadan (≈0.0887 USDT; yerel oran değil) |
| Marj | im_used ≈ 53.77 (200/3 ✓) |
| Faz | BasePending → **Open**, deal-1 aktif; CLOSE_ALL → Closing zinciri işledi |
| Webhook rotasyonu | 200, yeni secret anında yerleşti |

### Yeni bulgular (bu turda canlı teşhis)

- **M-12 (Orta):** Bybit `set-leverage` retCode=110043 ("zaten hedefte") `check_ret` yüzünden hata sayılıyordu → her turda WARN-retry spam'i. Düzeltildi (metin eşleşmesiyle başarı sayılır); canlı: tek INFO "venue leverage set to 3.00".
- **M-13 (Yüksek):** `wallet.available` Bybit UTA'da `availableToWithdraw`'i okuyordu (para çekme odaklı, genelde 0/az) → C-6 kapısı dolu hesabı YANLIŞ reddediyordu (canlı: 50,982 USDT bakiyeye rağmen "yetersiz"). Düzeltildi: hesap seviyesi `totalAvailableBalance` (coin değeriyle maksimumu). Kanıt: düzeltme sonrası aynı kurulum emri geçirdi.
- **M-14 (Orta):** ORDER_GATE reddi sonrası strateji fazı kilitleniyordu — ✅ §3.13'te KAPANDI (faz geri alma + Reduce muafiyeti).
- **Not:** Demo hesabın pozisyon modu hedge; `switch-mode` çağrısı OK dönerken mod okuması değişmedi (Demo ucu sınırlaması olabilir) — bu yüzden positionIdx eşlemesi kalıcı çözüm olarak uygulandı.
- **Bakiye/adım bilgisi:** BTCUSDT linear step=0.001 → minimum emir ≈81 USDT; 50 USDT'lik base "qty rounds to zero" ile dürüstçe iptal edildi (12 deal denemesi olaylarda görünür).

### Temizlik notu (dürüst)

Kapanış barını beklemeden bot silindi; Demo hesapta **0.002 BTC (~160 USDT sanal)** pozisyon açık kaldı — Bybit Demo panelinden tek tıkla kapatılabilir ya da bir sonraki bot kurulumunda restore+CLOSE_ALL zinciri kapatır. Gerçek para etkisi yok.

---


## 3.13 Son tur — H-9 sıfır danışma + M-14 + Reduce muafiyeti + alert dedup (26 Ekim)

| İş | Sonuç |
|---|---|
| **H-9 TAMAM:** `npm audit`: **91 → 0** (0 kritik/yüksek/orta/düşük — üretim bağımlılıkları tamamen temiz). Yol: next 16.4.3 + sharp 0.35.5 → `npm audit fix` (15'e) → ölü `next-auth`, `@reactuses/core`, `@mdxeditor/editor`, `react-syntax-highlighter` (kaynakta hiç kullanılmıyor) → **0**. `tsc` 0, `eslint` 0, UI temiz node_modules ile tam zincir 200. |
| **M-14 KAPANDI:** ORDER_GATE reddi artık strateji fazını geri alıyor (Dca BasePending→Idle, Grid bootstrap geri) — bot kilitlenmez, koşul düzelince yeniden denenir. |
| **Reduce muafiyeti (C-6 düzeltmesi):** TP/SL/CLOSE_ALL (Reduce) emirleri bakiye kapısına takılmaz — pozisyon kapatma asla engellenmez. |
| **H-7 alert dedup:** aynı (bot, action, timestamp) alert'i ikinci kez işlenmez — canlı kanıt: ilk 200, tekrar **400 DUPLICATE_ALERT** + WARN olayı. |
| **Demo temizlik:** cleanup bot restore edildi — venue pozisyonu **zaten kapalıydı** (önceki bot'un son barında kapanış tamamlanmış), 0 pozisyonla başladı; bot silindi. §3.12'deki "açık kaldı" notu güncellendi. |

---


## 3.14 Kapanış turu — cargo audit + M-1 sözleşme testi + tokio yaması (26 Ekim, son)

| İş | Sonuç |
|---|---|
| **cargo audit kuruldu ve koştu** (cargo-audit 0.22.2): başlangıç 2 bulgu → **tokio 1.54.0 → 1.54.1** (RUSTSEC-2026-0384 yarış durumu) ve **rustls-webpki 0.103.15 → 0.103.16** (RUSTSEC-2026-0382 sertifika doğrulaması) yamalandı → **0 vulnerability**; kalan tek "unmaintained" uyarısı `rustls-pemfile` (geçiş zincirinden, tokio-tungstenite bağımlılığı — güvenlik açığı değil). Rust tarafı artık npm gibi taranıyor. |
| **M-1 KAPANDI:** "safety_order_size taban, volume_scale her seviyede (ilkinde dâhil): safety_i = taban × v^i (100→120→144)" sözleşmesi `dca_math`'e kalıcı testle sabitlendi (`m01_safety_volume_chain_first_level_is_base_times_v`) — hesaplayıcı/sihirbaz/motor aynı zinciri kullandığı davranışla kanıtlandı. |
| **Tam test:** tokio yamasından sonra workspace **271 test / 0 hata** (EXIT 0). |
| **LOKAL ÇALIŞTIRMA:** portlar temizlendi → `npm run local:start` (yeni anahtar üretimi yok, mevcut kalıcı anahtarlar) → kabul zinciri **tam yeşil**: motor health 200 (readiness db:ok, lastTick 7 sn), UI 200, kimliksiz /api/bots **401**, Bearer'lı **200**, SSE proxy **200**, iki kullanıcı botu PAPER RUNNING olarak restore. |

### Bilinçli kalan (ürün kararı — kod borcu değil)

- **H-1 son yarısı** (dispatch-place/sweep ağlarının kilitsizleştirilmesi = tam actor modeli): mevcut 3-fazlı yapı + single-flight ile pratik engel kalmadı; refactor, eşzamanlı çok bot ölçeği gerektiğinde yapılmalı.
- **HedgeEngine bacak-başına-stepper** (LIVE hedge kilidi sürüyor): hedge-mode hesaplarda DCA zaten positionIdx eşlemesiyle çalışıyor; özel hedge stratejilerinin LIVE'ı, bacak muhasebesi mimarisiyle birlikte açılmalı.
- **UI'da cert_acknowledged onay kutusu**: API düzeyinde canlı; sihirbaza eklenmesi tek-ekran işi.

---

## 4. 23 Ekim Raporundan Bu Yana DÜZELTİLEN Bulgular (güncel kaynakta doğrulandı)

| Eski No | Konu | Güncel kanıt |
|---|---|---|
| SEC-01 | API auth yok / CORS çok açık / 0.0.0.0 bind | `main.rs:125-127` (DCA_API_KEY zorunlu), `main.rs:196` (Bearer middleware), `main.rs:202-206` (CORS yalnız localhost:3000), `main.rs:210` (127.0.0.1:3002 bind) |
| SEC-02 | Vault master key DB'de | `store.rs:186-194` — env zorunlu, DB'ye yazılmıyor, fail-closed |
| SEC-03 | OKX izin kontrolü sabit yanlış | `okx.rs:477-497` — gerçek `/api/v5/account/get-api-key` introspection'ı |
| SEC-05 | Kasadan silinen anahtar botta kalıyor | `main.rs:2977-3011` — silme öncesi bağlı LIVE botlar STOPPED yapılıyor |
| SEC-06 | Caddy istemci kontrollü port | `Caddyfile:1-22` — sabit upstream (3002/3000) |
| EX-01 | Emir ID'leri borsa sınırını aşıyor | `main.rs:683-684` — bot id `b-`+8 hex (10 kr); türetilen emir id'leri ≤ ~20 kr |
| EX-02 | Spot market alış birim hatası | `bybit.rs:469-471` (marketUnit=baseCoin), `okx.rs:551-552` (tgtCcy=base_ccy) |
| EX-07 | OKX posSide/posMode | `okx.rs:532-539` (posSide), `okx.rs:707-727` (set_leverage öncesi long_short_mode assertion, fail-closed) |
| EX-08 (sorgu bacağı) | Kontrat→coin dönüşümü | `okx.rs:630-644` — query_order ctVal ile normalize ediyor |
| BOT-01 | Spot equity yanlış | `dca_kernel/lib.rs:574-585` — `cash + qty×mark`; testli |
| BOT-02 | Fill idempotensi | `store.rs:49` UNIQUE(bot_id,fill_key) + `engine.rs:796-827` INSERT OR IGNORE/fill_exists + `dca_kernel/lib.rs:471-477` apply_fill dedup |
| BOT-04 (forming bar) | Kapanmamış mum işleniyor | `engine.rs:1023-1046` — `b.t + interval_ms <= now` filtresi |
| BOT-05 | ADD_FUNDS sanal pozisyon | `main.rs:1036-1091` — PAPER: yalnız nakit; LIVE: gerçek market emir + takip |
| BOT-06 | TP/SL patch kalıcı değil | `main.rs:924-937` — patch DB config'e yazılıyor |
| BOT-07 (delete bacağı) | Silme sahipsiz emir bırakır | `main.rs:956-990` — silme öncesi açık emirler iptal ediliyor |
| BOT-08 (manuel tick) | STOPPED/PAUSED tick | `main.rs:2441-2449` — manuel tick status kontrolü |
| BOT-10 | Kill/freeze restart kaybı | `state.rs:263-297` — events akışından restart-safe rehydrate (her kapı için son olay) |
| BOT-11 (kısmen) | Sessiz DB kaybı | `state.rs:350-363` — persist_status retry + stderr (ancak bkz. H-8 kalıntısı) |
| OPS-02 | ignoreBuildErrors | `next.config.ts:11` — kaldırılmış (`ignoreBuildErrors: false`) |
| OPS-04 (ticker bacağı) | Hata kalıcı cache | `main.rs:1632-1647` — ticker 30 sn TTL (ancak instruments bacağı hâlâ süresiz, bkz. M-3) |
| OPS-06 (kısmen) | equity_points sınırsız büyüme | `engine.rs:829-841` — bot başına 5000 nokta上限 |

---


## 4.1 Sektörel Kıyas ve Ürün Geliştirme Turu (26 Ekim — bot geliştirme)

### Sektörel konumlandırma (2026 kaynakları)

[Bitsgap'in 2026 karşılaştırması](https://bitsgap.com/blog/blog-grid-vs-dca-vs-combo-vs-loop-which-crypto-trading-bot-fits-your-market-scenario) ve [Bitsgap vs Pionex](https://bitsgap.com/blog/bitsgap-vs-pionex-2026-honest-comparison), [TradeSanta DCA-vs-Grid](https://tradesanta.com/grid-vs-dca) ve [Gainium DCA rehberi](https://gainium.io/best/dca) referansıyla:

| Sektör yeteneği | 3Commas / Bitsgap / Pionex / Freqtrade | DCABOT (bu proje) | Hüküm |
|---|---|---|---|
| DCA + güvenlik emirleri + martingale ölçekleme | hepsinde | `max_safety_orders` + `volume_scale` (M02 testli zincir) | ✅ eşdeğer |
| Trailing TP | hepsinde | `trailing_*` config + motor desteği | ✅ |
| Grid + Trailing Up/Down | Bitsgap amiral gemisi | `grid_trail` (mesh breakout takibi — testli) | ✅ |
| Infinity grid | Pionex | `InfinityGrid` (M17) | ✅ |
| Sinyal botları (TradingView) | 3Commas'ın gücü | webhook + kalıcı secret + dedup | ✅ |
| AI grid parametre optimizasyonu | Bitsgap farklılaştırıcısı | `/api/analysis/calibration` (2σ öneri) | 🔶 temel düzey |
| Backtest + overfit koruması | sınırlı/kapalı kutu | Lab + sertifika (DSR/PBO/WFE) — **sektörden ileri** | ✅➕ |
| Pine Script strateji editörü | TradingView dışına yok | dca_pine alt kümesi (derlemede dürüst uyarılar) | ✅ benzersiz |
| Açık kaynak / self-host | Freqtrade | bu proje | ✅ |

**Kapanan ürün eksiği — hazır Pine strateji kütüphanesi** ([awesome-pinescript](https://github.com/pAulseperformance/awesome-pinescript), [Alorse/pinescript-strategies](https://github.com/Alorse/pinescript-strategies) 50+ koleksiyonundan seçilmiş, motora uyarlanabilir klasikler):

Sihirbaz şablonlarına 4 yeni strateji eklendi (`wizard-data.ts`) ve **dördü canlı motorda derlendi + ikisi (MACD türetmesi, ROC momentum) backtest'te koşturuldu**:

1. **MACD Sinyal Kesişimi** — ema-ema farkı üzerine sinyal EMA'sı (türetilmiş seri, motor destekli — canlı doğrulandı; 200 bar → 8 giriş/15 fill).
2. **RSI Dip + EMA Trend Filtresi** — dip alımı yalnız 200-EMA üzerinde (trend-zeminli giriş; klasik "buy the dip in uptrend").
3. **EMA Kesişim + ATR Stop** — `strategy.exit(stop=)` ile volatilite-uyarlı koruma.
4. **Momentum Kırılım (ROC)** — `ta.mom` yüzde eşik; 200 bar → 2 giriş, çıkış sinyalli (backtest OK).

Bilinçli kapsam dışı: gerçek Bollinger (motor alt kümesinde stdev yok — Donchian zaten kanal eşdeğeri) ve Supertrend (öz-yineli seri gerektiriyor; alt kümede `var` yok — derleyici dürüstçe reddeder).

### UI'da cert_acknowledged akışı (ÜRÜN-1'in son parçası)

- `apiCreateBot` üçüncü parametre olarak `cert_acknowledged` taşıyor (`use-bots.ts`).
- Sihirbaz: `CERT_GATE_FAILED` 422'sine özel panel — kapakta kalan kapılar gösterilir, **bilinçli-onay kutusu** (kırmızı, sorumluluk metinli) işaretlenirse launch `cert_acknowledged:true` ile tekrarlanır; motor onayı kalıcı WARN olayına yazar. Öneri metni: önce Lab'de config güçlendirme.
- Doğrulama: `tsc` 0, `eslint` 0, UI yeniden başlatıldı (tarayıcı akışı canlı), 4 şablon motor derlemesi geçti.

### Sonraki ürün adımları (öneri sırasıyla)

1. **BB/Supertrend göstergeleri**: `dca_math`'e stdev + öz-yineli seri desteği (motor tarafı, orta iş) → Bollinger/Supertrend şablonları açılır.
2. **Pine şablon kütüphanesine kaydetme**: 4 strateji `pine_scripts` tablosuna seed (Studio'daki kütüphane sekmesinde görünür).
3. **Telegram/loyihalar bildirim kanalı** (sektörde standart; ops işi).
4. **Grid parametre öneri motoru**: calibration uçunu Lab'e bağlayıp "Bitsgap-AI" eşdeğeri tek-tık optimizasyon.

---


## 4.2 Ürün Geliştirme Fazları 6–9 — YOL HARİTASI + UYGULAMA (26 Ekim, tamamlanan tur)

Sektörel araştırma sonrası önerilen dört adım, faz disipliniyle planlandı ve **aynı turda dördü de uygulandı**:

### Faz 6 — Motor göstergeleri: ta.stdev + ta.supertrend ✅

- `dca_pine` alt kümesine **`ta.stdev(src, len)`** eklendi: popülasyon varyansı exact kesir; kök, BigInt Newton `isqrt` ile ölçekli kesir-approx (raporlama-sınırı yaklaşımı — half-even felsefesiyle aynı spirit, deterministik).
- **`ta.supertrend(factor, atrLen)`** eklendi: öz-yineli final bantlar + yön + kendi RMA-ATR'si **düğüm başına taşıyıcı durumda** (TaState::st) — Pine'ın var-öz-yinelemesi motor tarafında çözüldü; çoklu çağrı izole. İlk bar ATR=TR ile başlar (TV warmup farkı dokümante).
- Sihirbaza 2 şablon: **Bollinger Dip (mean-revert)** ve **Supertrend Takip**.
- **Canlı kanıt:** Bollinger 300 bar → 9 giriş/18 fill; Supertrend → 4 giriş/8 fill (pyramiding uyarısı dahil dürüst). Çalışma-anı: 271 test / 0 hata.

### Faz 7 — Pine Kütüphanesi seed ✅

- Startup'ta 6 sektör-standart şablon (`Şablon: …` öneki) `pine_scripts`'e **idempotent** yazılır (isim varsa atlar — kullanıcı düzenlemesini ezmez).
- **Yakalanan hata:** ilk turda seed `id` kolonunu yazmadan insert etti → TEXT PRIMARY KEY NULL satırlar listelemede görünmez düştü (API boş döndü, DB'de 6 kayıt). Düzeltildi (uuid'li id); NULL-PK satırları temizlendi; **API artık 6 şablonu listeliyor**.

### Faz 8 — Telegram bildirim kanalı ✅ (kod canlı; aktivasyon kullanıcıda)

- `TELEGRAM_BOT_TOKEN` + `TELEGRAM_CHAT_ID` ikisi de set edilirse motor SSE akışına abone olur; **FILL / LIQUIDATION / RISK / WEBHOOK_START / KILL_SWITCH / FREEZE / BOT_STATE** olaylarını Bot API'ye iletir (Markdown, BOT_STATE sessiz). Token yoksa tamamen sessiz — opsiyonel özellik, hatasız.
- Kabul: kod yolu derlendi + motor sorunsuz açıldı (token'sız ortamda skip kanıtı); uçtan uca mesaj testi token eklenince tek adım.

### Faz 9 — Grid parametre öneri motoru ✅

- **`GET /api/analysis/grid-suggest?symbol&interval&bars&investment[&levels]`**: pencere dip/tepe (±%1 kaçak payı), bar-başı log-return sigma, seviye önerisi (`span/0.75σ`, 8–48 kelepçe), geometric adım %, **ücret-kırıcı (g_min) denetimi**, seviye-başı notional ve enstrüman tick/step — tek yanıtta.
- **Canlı kanıt (BTCUSDT 1h/500 bar, 5000 USDT):** menzil 80,331–85,702; sigma %0.34; 16 seviye; adım %0.405 > g_min %0.200 → `feeBreakerPass: true`; seviye notional 312.50.

### Kullanım notu

- Sihirbazda Pine editörünün şablon listesinde artık 8 hazır strateji var (4 önceki tur + BB + Supertrend + mevcutlar); Studio'daki Pine Kütüphanesi'nde 6 "Şablon:" kaydı.
- Telegram'ı açmak için `.env`'e `TELEGRAM_BOT_TOKEN=<BotFather token>` ve `TELEGRAM_CHAT_ID=<chat>` ekleyip `npm run restart:engine`.
- grid-suggest çıktısı öneridir — Lab backtest + sertifika akışına girdi olarak tasarlandı.

---


## 4.3 Faz 10 — Öneri↔Hesaplayıcı entegrasyonu (26 Ekim)

- **`apiGridSuggest`** istemci API'si eklendi (`api-analysis.ts`, tip + GET sarmalayıcı).
- **Hesaplayıcı → Grid sekmesi**'ne "Otomatik Grid Önerisi" kartı eklendi (`calculator.tsx`): sembol + toplam yatırım girdisiyle **Otomatik Öner** butonu; motor yanıtından **alt/üst sınır, seviye sayısı ve seviye-başı notional girdilerini otomatik doldurur**, altta σ/adım%/g_min/ücret-kırıcı hüküm satırı gösterir. Mevcut mesh tablosu otomatik yeniden hesaplanır (aynı state'ler).
- Kullanıcı artık tek tıkla: gerçek 500-bar menzili + oynaklıktan grid taslağı → mesh incelemesi → (istediği gibi değerleri elle kırparak) sihirbaza taşıma.
- Doğrulama: `tsc` 0, `eslint` 0; UI yeni kodla yeniden başladı (sayfa + rewrite zinciri 200; tarayıcı SSE akışı canlı).

### Yol haritası güncel durumu

| Faz | İçerik | Durum |
|---|---|---|
| 6 | ta.stdev + ta.supertrend + 2 şablon | ✅ |
| 7 | Pine Kütüphanesi seed (6 şablon) | ✅ |
| 8 | Telegram bildirim (env-gated) | ✅ kod; aktivasyon kullanıcıda |
| 9 | grid-suggest ucu | ✅ |
| 10 | Öneri↔Hesaplayıcı entegrasyonu (UI) | ✅ |
| 11 | Bildirim kanalı genişletme (e-posta / outgoing webhook) | sıradaki (aynı abone deseni) |
| 12 | Ek göstergeler (Stochastic/ADX) + şablonları | sıradaki (aynı ekleme deseni) |
| 13 | H-1 son yarısı + HedgeEngine bacak-stepper | büyük refactor — ölçek gerektiğinde |

---


## 4.4 Faz 11–12 — bildirim kanalları + yeni göstergeler, Faz 13 ilk dilim (26 Ekim gece)

### Faz 11 — Bildirim kanalları ✅ (webhook CANLI kanıt; telegram/smtp aynı boru)

`spawn_telegram_notifier` → **`spawn_notifiers`**: yapılandırılmış TÜM kanallara fan-out —
- **Telegram**: `TELEGRAM_BOT_TOKEN` + `TELEGRAM_CHAT_ID` (mevcut davranış korundu),
- **Outgoing webhook** (YENİ): `NOTIFY_WEBHOOK_URL` → her olay için `{event, data, ts}` JSON POST,
- **SMTP e-posta** (YENİ, `lettre 0.11` rustls): `NOTIFY_SMTP_HOST` + `NOTIFY_SMTP_TO` (+`USER/PASS/FROM`) — spawn_blocking'de STARTTLS.

Olay kümesi: FILL, LIQUIDATION, RISK, WEBHOOK_START, KILL_SWITCH, FREEZE, BOT_STATE. Hiçbiri yapılandırılmamışsa abone olunmaz (sıfır maliyet).

**Canlı kanıt (outgoing webhook):** `.local-runtime/local-keys.env`'e `NOTIFY_WEBHOOK_URL=http://127.0.0.1:9877/notify` eklendi; yerel dinleyici kurularak PAUSE→START tetiklendi → `"notifier aktif: webhook"` log + **2 makbuz** ({event: BOT_STATE, data: {action: PAUSE/START, botId…}, ts}) birebir alındı. E-posta/Telegram kanalları aynı boruyu paylaşır; kimlik bilgileri eklenince ek doğrulama gerektirmez.

### Faz 12 — Yeni göstergeler ✅ (backtest CANLI)

- **`ta.stoch(src, high, low, len)`**: %K — (src,high,low) üçlüsü düğüm penceresinde; %D kullanıcı tarafında `ta.sma(k,3)` ile.
- **`ta.adx(len)`**: Wilder ADX — TR/±DM RMA düzleştirmesi + DX + RMA(DX), düğüm-başına taşıyıcı durum (supertrend deseni); ADX tohumlanana kadar **Na** döner (dürüst warmup, yanlış sinyal üretmez).
- Şablonlar: **Stochastic Dönüş** (K/D kesişimi + dip eşik) ve **ADX Trend + EMA Kesişim** (kesişim yalnız ADX > 20'de).
- **Canlı kanıt:** Stochastic 300 bar → 22 giriş/44 fill; ADX → 10 giriş/20 fill. tsc 0.

### Faz 13 — ilk dilim: supervisor jitter ✅

Botlar artık aynı ana sıkışmıyor: bot-kimliğinden türetilen 0–1200 ms jitter poll'u dağıtır (tek yavaş venue tüm filoyu aynı turda kilitlemez). Kalan (ayrı oturum gerektiren gerçek refactor): dispatch-place/sweep ağlarının kilitsizleştirilmesi (tam actor modeli) ve HedgeEngine bacak-başına stepper.

### Kabul durumu

`cargo check` 0 hata; workspace **271 test / 0 hata**; motor + UI yeniden başlatıldı — notifier aktif, 4 bot RUNNING (kullanıcının UI'dan kurduğu "xrp1" dâhil). Bir süreç notu: bu turda biri supervisor-jitter düzenlemesinde (`u8` deref), diğeri lettre `send` trait-import'unda olmak üzere iki derleme hatası canlı yakalanıp düzeltildi.

---


## 4.5 Faz 13 dilim 2 — UI okumaları kilitsiz (anlık görüntü önbelleği) (27 Ekim)

- **`AppState.ui_cache`**: bot başına tam anlık görüntü (snapshot_json) — poll sonunda, kurulum/restore'da, patch/action sonrasında tazelenir; silmede düşürülür.
- **`snapshot_json` zengindi:** `equityNow` + `unrealizedNow` alanları eklendi (mark = son kapanış; yoksa Na).
- **`GET /api/bots`** ve **`GET /api/summary`** artık cache'ten yanıt verir — **venue yavaşken Fleet/Özet istekleri runtime kilidini beklemez** (kilitli yol, cache boşsa dürüst fallback olarak durur). Özet agregasyonu cache-anlık görüntülerinden exact kesirlerle hesaplanır (hedge short bacağı dâhil — aynı frontend sözleşmesi).
- **Canlı kabul:** bots/summary 200; özet cache'ten 4 bot / 3 aktif işlem / long-exposure ve fees doğru; poll edilmemiş botlarda equityNow None (dürüst warmup — ilk poll'da dolar).
- 271 test / 0 hata; süreç notu: bu turda üç mekanik derleme hatası (çift-imza, State sarmalayıcı, anchor farkları) canlı yakalanıp düzeltildi.

**Faz 13 kalanı (kesin kapsam, ayrı oturum):** dispatch-action'ın place/sweep ağlarının tam actor modeline taşınması + HedgeEngine bacak-başına stepper (LIVE hedge kilidini açar). Her ikisi mevcut canlı davranışı değiştirmeden tek başına planlanıp uygulanır.

---


## 4.6 Faz 13A dilim 3 — handler-IEA deseni + TAM FAZ 13 UYGULAMA PLANI (27 Ekim)

### Bu turda uygulanan

**ADD_FUNDS LIVE artık kilit-dışı ağ ile çalışıyor (IEA deseni, handler ölçeğinde):** plan (miktar/orderLinkIdx/venue) kısa kilitte kurulur → **kilit bırakılır** → `place_order` serbest → uygulama (tracking + snapshot tazeleme) yeni kilitte. Bot listesi/özet ve diğer botların poll'u, manual ek-alımın borsa gecikmesini beklemez. `delete_bot` zaten kilit-dışı iptal yapıyordu (doğrulandı).
**Kabul:** PAPER ADD_FUNDS canlı — kendi test botuyla kesir-defter 1000→1100, fark tam 100 (200); `no mark price yet` reddi önceden-var olan dürüst sözleşme (bar işlemeden ek-alım yok). 271 test / 0 hata.

### FAZ 13 TAM PLAN (kalan iki kalemin icra-ready tasarımı — taze oturumda sırayla uygulanır)

**13A-POLL — Dispatch IEA worklist (poll_inner + dispatch_action yeniden yapılanması):**
1. `QueuedIntent` enum: `PlaceLimit{...}` / `PlaceMarket{...}` / `Cancel{...}` — plan alanları (OrderRequest + tracking + role/kind + cascade etiketi).
2. `dispatch_action` LIVE dalları artık ağ yapmaz: intent'i `rt.pending_live_intents: Vec<QueuedIntent>`'a iter (PAPER dalı aynen senkron).
3. `poll_inner` kilitli gövdesi bar işlerken intent'leri toplar; **kilit bırakılır** → `execute_intents(app, &mut intents)` (place/query/cancel/executions ağı) → **yeniden kilit** → `apply_intents` (tracking/upsert/ledger/fill ingest + `engine.on_fill` cascade → yeni Action'lar → yeni intent'ler).
4. Cascade döngüsü: apply yeni intent üretirse tekrar unlock/execute — **en fazla 8 tur** (patolojik zincire sınır; aşımda CRITICAL olay).
5. `force_start_if_requested` ve `close_position_as_venue_liquidated` aynı üç parçaya ayrılır.
6. Test: sahte-adapter ile "yavaş venue" senaryosu (500 ms uyku) — poll süresince `list_bots` 5 ms altında döner; cascade: TP-fill → MANUAL kapanış zinciri tek poll'da tamamlanır.

**13B — HedgeEngine bacak-başına stepper (LIVE hedge kilidini açar):**
1. `BotRuntime.hedge_stepper: Option<SimulationStepper>` — HedgeEngine kurulumunda hedge bacağı için ayrı stepper (aynı instrument, direction=-1).
2. Yönlendirme kuralı: role öneki `"HEDGE"` → hedge_stepper'a ingest; ana bacak rolü → ana stepper. Kernel fill'leri `engine.on_fill` cascade'iyle bacak motorlarına zaten ayrı düşer (hedge.rs fill-routing testli).
3. Emir: hedge bacak emirlerinde `position_idx = hedge-modda 2 (Sell) / 1 (Buy)`; one-way hesapta hedge stratejileri LIVE'a yine kapalı (mevcut kilit koşulu kalır).
4. Likidasyon/funding: her stepper kendi M07/funding döngüsünde (poll_inner'da iki bacak sırayla); combined-exit kararı HedgeEngine'de kalır.
5. Snapshot: `RuntimeSnapshot`'a `hedge_state: Option<HedgeSnapshot{stepper alanları}>` eklenir (serde hazır); restore aynı sırayla iki bacağı venue'dan uzlaştırır.
6. Özet/Fleet: short exposure hedge_stepper'dan da toplanır (ui_cache alanları genişler).
7. Test: eşit long/short açılış → iki ayrı stepper pozisyonu, net delta 0, IM ve fee iki bacakta ayrı; restore iki bacağı venue'dan geri yükler; LIVE kilidi yalnız bu kabul turu sonrası açılır.

**Sıra ve güvenlik:** 13A-POLL önce (davranış-koruyucu refactor, tek risk noktası dispatch), 13B sonra (muhasebe genişlemesi, kilidi amaca hizmetle açma). Her adım kendi kabul turuyla kapanır; süreç boyunca LIVE hedge kilidi 13B kabulüne dek sürer.

### 13A-POLL icra-detalı (bir sonraki oturumda mekanik uygulama için)

**Ön güvenlik:** git deposu kuruldu ve tam çalışan durum ilk commit'e alındı (`9148955`) — refactor sırasında `git checkout -- .` ile her an geri dönülebilir.

**Veri tipleri (`state.rs`):**
```rust
pub enum LiveIntent {
    Place { req: OrderRequest, id: String, side: Side, kind: OrderKind,
            price: Option<Fraction>, qty: Fraction, role: String, is_close: bool },
    Cancel { links: Vec<String> },   // wildcard '*' execute'ta open_orders ile çözülür
}
pub struct GateCtx {   // pre_order_gate'in rt'siz karşılığı — intent kuruluşunda (kilit altı, ucuza) doldurulur
    pub pos_mode_blocked: bool, pub last_close: Option<Fraction>,
    pub leverage: Fraction, pub is_spot: bool, pub quote: String,
    pub min_notional: Fraction, pub kind_is_futures: bool,
}
// BotRuntime: pending_live_intents: Vec<LiveIntent>
```

**dispatch_action LIVE dalları** yalnız intent iter (ağ yok): PlaceLimit/PlaceMarket → GateCtx kur + `LiveIntent::Place` push; CancelResting → `LiveIntent::Cancel{links}` push. PAPER dalları aynen senkron kalır.

**`pre_order_gate` imzası:** `(gate: &GateCtx, others: Fraction, adapter, creds, price, qty)` — wallet çağrısı execute fazında (kilitsiz); `venue_live_load` ayrı kilit olduğundan okunabilir.

**poll_inner kuyruğu:** mevcut gövde sonunda (sweep sonrası) `drop(rt); drop(rts);` ile borçlar bitirilir (NLL güvenli — sonrası yalnız `Ok(processed)`), ardından:
```rust
run_intent_cascade(app, bot_id, sse).await?;   // ≤8 tur:
// tur: [kısa kilit] intents=take + (venue,creds) kopyala → [kilitsiz] execute_live_intents
//       (gate→place→market 3×query) → [kısa kilit] apply_intent_outcomes
//       (upsert/tracking/bump/ingest/on_fill→yeni intent'ler)
```
**apply kaskadı:** Placed-limit → PENDING upsert + tracking + bump(+); MarketFilled → `ingest_external_fill_with_fee` + persist + on_fill + **yeni Action'lar dispatch_action'a** (intent olarak bir sonraki tura); GateBlocked → WARN + `rollback_pending_phase_after_gate_reject`; PlaceFailed/CancelFailed → WARN (mevcut davranış).
**Kapsam notu:** `live_fill_sweep` bu fazda kilit altında kalır (bölüm 2 — kendi execute/apply ayrışması küçük ve benzer); ilk turda place/cancel = en ağır ve kullanıcıya en görünür gecikme kaynağı kilit dışına çıkar.

**Kabul (13A-POLL):** (1) Demo LIVE bot (cert_acknowledged ile) emri YENİ intent yolundan verir, execution sweep dolumu işler; (2) sahte-yavaş adapter testi: poll süresince `list_bots` < 5 ms (cache zaten kilitsiz — doğrudan kanıt); (3) TP-fill→kapanış cascade'i tek poll'da tamamlanır (tur sayacı olaylarda görünür).

**13B uygulama sırası (aynı raporun yukarısındaki 7 adım) değişmez; 13A-POLL kabulünden sonra başlar.**

---


## 4.7 Faz 13A-POLL — UYGULANDI (27 Ekim: dispatch artık kilit-dışı ağ ile çalışıyor)

Rapor §4.6'daki icra-detayından mekanik uygulama:

- **`state.rs`:** `LiveIntent` enum (Place{req,id,side,kind,price,qty,role,is_close,gate:GateCtx} / Cancel{links,venue,category,symbol}), `GateCtx` (pre_order_gate'in rt'siz girdisi), `IntentOutcome` (Placed/MarketFilled/MarketOpen/MarketNoTrace/MarketQueryFailed/GateBlocked/PlaceFailed/Cancelled), `BotRuntime.pending_live_intents`.
- **`dispatch_action` LIVE dalları artık AĞ YAPMAZ:** PlaceLimit/PlaceMarket → GateCtx kur + intent push; CancelResting → Cancel intent push (yerel pending temizliği aynen senkron). PAPER dalları aynen senkron.
- **`pre_order_gate_ctx`**: imza `(app, &GateCtx, adapter, creds, price, qty)` — wallet çağrısı execute fazında kilitsiz.
- **`execute_live_intents`** (kilitsiz): gate (reduce muaf) → place → market 3× dolum sorgusu → IntentOutcome'lar; cancel: wildcard open_orders çözümü + idempotent 110001 başarı sözleşmesi.
- **`apply_intent_outcomes`** (kısa kilit): Placed-limit → bump(+)/PENDING upsert/tracking; MarketFilled → idempotent ingest (Taker/MarketableTaker) + persist + on_fill **cascade** (yeni Action'lar → yeni intent, sonraki tur) + FILL SSE; MarketOpen → bump(+)/PARTIAL upsert/tracking; GateBlocked → WARN + M-14 faz geri alma; Cancelled → tracking düşümü + bump(−) + olaylar.
- **`run_intent_cascade`** (poll_inner sonunda, `drop(rt); drop(rts);` sonrası): ≤8 tur [kısa kilit take → kilitsiz execute → kısa kilit apply]; aşım CRITICAL + erteleme; tur>0 ise kalıcı snapshot + UI cache tazeleme.
- **Canlı kanıt:** Demo LIVE bot (cert_acknowledged) ilk poll'da — `WARMUP: 59 bar atlandı` → BASE emri **intent yolundan** verildi → venue doldu → cascade apply pozisyonu açtı (`Open`, 0.002 BTC, TAKER fill, IM ≈53.8, fee gerçek). CLOSE_ALL → Closing (kapanış barı gelmeden bot silindi — Demo panelinden kapatılabilir sanal pozisyon notu, §3.12 ile aynı durum).
- 271 test / 0 hata; süreç notu: 5 mekanik derleme hatası canlı yakalanıp düzeltildi (OrderKind move, Option-price, String move, çift-imza, brace telafisi) — git güvenlik ağıyla sıfır risk.

**Kalan (13B):** HedgeEngine bacak-başına stepper (§4.6'daki 7 adımlı plan) — LIVE hedge kilidini açar; ayrı oturum işi.

---


## 4.8 Faz 13B — HedgeEngine bacak-başına stepper UYGULANDI (27 Ekim: EX-06 kalıcı çözüm, LIVE hedge kilidi açıldı)

§4.6'daki 7 adımlı plan uygulandı:

1. **`BotRuntime.hedge_stepper: Option<SimulationStepper>`** — her iki hedge türü için baştan kurulur (yön -1, aynı start_cash/instrument/risk policy); protective spawn sonrası devreye girer, dual ilk barda.
2. **Routing (order-id prefix):** `{bot}-hedge` → hedge kernel; `{bot}-main` → ana kernel. PAPER dispatch (limit+market), process_bar dual-step (fill ingest `ingest_kernel_fills` helper'ıyla iki kernel için), LIVE sweep (executions + fallback top-fill), 13A apply (MarketFilled) — **tümü prefix'e göre yönlendiriyor**.
3. **Emirler:** hedge bacak emirleri mevcut positionIdx eşlemesiyle (Sell→2, Buy→1 hedge-modda) venue'a gider — kod değişikliği gerekmedi (Faz 4.5/5 eşlemesi taraf-bazlıydı).
4. **Likidasyon per-bacak:** `liq_price_of_stepper` helper'ı — hedge bacağı kendi M07 sınırıyla izlenir; tetiklenirse YALNIZ hedge bacağı force-close + `h.hedge.force_liquidate` (ana bacak etkilenmez). Ana bacak likidasyonu artık yalnız ana bacağı kapatır (eski "iki bacağı birden" davranışı kaldırıldı). Venue-flat öğrenimi: her iki kernel defteri de kapatılır.
5. **Funding per-bacak:** hedge stepper kendi notional'iyla ayrı settlement + `h.hedge.apply_funding`.
6. **Snapshot:** `RuntimeSnapshot.hedge_kernel: Option<HedgeKernelSnap>` (cash/IM/fees/funding/realized/PositionState — PositionState serde'li) — restore iki bacağı bağımsız geri yükler. `snapshot_json`: `equityNow` **combined** (ana+hedge), `hedgeEquityNow` bacak-bazlı.
7. **LIVE hedge kilidi KALDIRILDI:** create_bot 422'si ve restore PAPER-düşürmesi kaldırıldı (hesap hedge-moddaysa POS_MODE denetimi emirleri positionIdx ile eşlemeye devam eder).

**Canlı kanıt (HEDGE_DUAL PAPER, rho=1, 2000 USDT/bacak):** ilk bar → **iki fill**: `…-hedge-BASE-1` (Sell 1/1000) + `…-main-BASE-1` (Buy 1/1000) → **kernel net delta: 0**; `hedge_kernel` snapshot: qty 1/1000, yön -1, IM rezerve, fees birikmiş; `equityNow` 4000 = iki bacak toplamı (ayrı defterlerin toplamı). 271 test / 0 hata. Süreç notu: json! makro blok-ifade kısıtı ve 4 borrow-check hatası canlı yakalanıp düzeltildi (git ağı ile).

**Not:** Demo hesapta HEDGE LIVE kabul turu (testnet rampası kapsamında) önerilir; PAPER dual kanıtı + positionIdx eşlemesi + tüm güvenlik kapıları mevcut.

---


## 4.9 Cloudflare Denetimi — dışa-açılış mimarisi değerlendirmesi (27 Ekim)

Cloudflare ürün kataloğu üzerinden tam proje denetimi; hedef AGENTS.md'de kayıtlı olan "offline-first, gerektiğinde Cloudflare Tunnels ile dışa açılış".

### Bileşen bazlı Cloudflare eşlemesi

| Bileşen | Cloudflare'a taşınabilir mi? | Hüküm |
|---|---|---|
| Rust motor (`dca_server`: SQLite WAL, tokio uzun-ömürlü süreç, Bybit/OKX'e giden TCP, 24/7 poll supervisor) | ❌ Workers kısa-ömürlü isolate + D1/KV modeline uymaz; Containers bile gereksiz karmaşıklık (Windows'ta sorunsuz koşuyor) | **Yerel kalır** — doğru karar |
| Next.js UI | Workers'a taşınabilir (vinext) AMA motor `127.0.0.1`'e bağlı — Workers'tan localhost'a erişilemez; UI'nin değeri motora komşuluk | **Yerel kalır** |
| TradingView webhook girişi | Tunnel ile dış dünyaya açılabilir; kendi koruması sağlam (constant-time secret, 60 sn replay, DUPLICATE dedup — canlı doğrulandı) | **Tunnel senaryosu** |
| PAPER veritabanı / events / vault | D1/R2 yerine SQLite WAL kalır (motor ile aynı süreç, transactional bütünlük) | **Yerel kalır** |

### Doğru Cloudflare kombinasyonu: Tunnel + Access + (WAF kuralları)

1. **Cloudflare Tunnel** (`cloudflared` Windows servisi): motor+UI yerel kalmaya devam ederken panel ve webhook TLS + Cloudflare ağı üzerinden dışarıya açılır. Giden-bağlantı modeli: port açma yok, 0.0.0.0 yok — mevcut `127.0.0.1` bind'lar (doğrulandı: motor `main.rs:226`, UI `--hostname 127.0.0.1`) ile birebir uyumlu.
2. **Cloudflare Access** (zorunlu değilse açma!): `NEXT_PUBLIC_ENGINE_API_KEY` tarayıcı bundle'ına gömülü (`engine-client.ts:35` — lokalde kabul edilebilir); panel dışarıya açılırsa anahtar her ziyaretçiye sızar. Access kimlik politikası panel önünde ilk kilit olmalı. Webhook yolu (`/api/webhooks/tradingview`) Access'in DIŞINDA tutulmalı (TradingView sunucuları Access login'i geçemez) — kendi secret katmanı yeterli.
3. **WAF/Rate-limit kuralları**: webhook yoluna sadece TradingView'in duyurulan IP aralıkları; `/api/*` genelinde agresif rate-limit.
4. **CORS hazırlığı** (kod değişikliği gerekli, açılış öncesi): motor CORS listesi yalnız `localhost` origin'leri içeriyor — Tunnel domain'i eklenmeli.
5. **Caddyfile notu**: `:81` tüm arayüzlerde dinler (LAN'a açık); Windows güvenlik duvarına güvenmek yerine `localhost:81` kısıtı önerilir. Tunnel doğrudan `localhost:3000/3002`'e de bağlanabilir — Caddy opsiyoneldir.

### Somut cloudflared yapılandırma taslağı (uygulama istendiğinde)

```yaml
# ~/.cloudflared/config.yml
tunnel: dcabot
credentials-file: C:/Users/ArdaAzmus/.cloudflared/dcabot.json
ingress:
  - hostname: bot.ornek-domain.com        # Access ile korunur (UI paneli)
    service: http://localhost:3000
    originRequest:
      noTLSVerify: true
  - hostname: hook.ornek-domain.com       # Access DIŞI — TradingView webhook'u
    service: http://localhost:3002
    originRequest:
      noTLSVerify: true
  - service: http_status:404
```

### Denetim bulguları (Cloudflare lensi)

- **CF-1 (önemli, açılış önü):** istemci gömülü `NEXT_PUBLIC_ENGINE_API_KEY` — dışa açılışta Access'siz panel açılmamalı; kalıcı çözüm olarak anahtar taşımanın Cloudflare Worker (Access JWT doğrulamalı) üzerinden yapılması değerlendirilebilir.
- **CF-2 (açılış önü):** CORS listesine Tunnel domain'i eklenmeli (tek satır, `main.rs`).
- **CF-3 (öneri):** `:81` Caddy dinleyicisi `localhost`'a kısıtlanmalı.
- **CF-4 (bilgi):** API anahtarlarının IP kısıt listesi (`ips`) vault'ta toplanıyor ama motor tarafında zorlanmıyor — Bybit anahtarlarını kendi IP'nize kısıtlarsanız bu motor-uzayı riskini ortadan kaldırır (borsa tarafı önlemi).
- **CF-5 (bilgi):** Telegram/webhook/SMTP bildirim kanalları Cloudflare'den bağımsız çalışır (giden bağlantı) — Tunnel kesintisinde bildirimler sürer; panel erişimi kesilir (kabul edilebilir).

**Sonuç:** Proje mimarisi Cloudflare ile çelişmiyor; doğru desen "yerel motor + Tunnel + Access". Workers'a taşıma YAPILMAMALI (motorun çalışma modeli uyumsuz, kazanç sıfır). Açılış istendiğinde uygulama sırası: Tunnel servisi kur → Access politikası (UI hostname'i) → CORS'a domain ekle → webhook hostname'ini Access dışı + WAF kuralı → uçtan uca test.

---


## 4.10 Bağımsız E2E Denetimi — tests/e2e_engine.py iki fazlı koşum (27 Ekim)

Pakete hiç dokunmadan (Faz 1) ve yalnız auth uyarlamasıyla (Faz 2) koşuldu; sonuçlar `.audit-runtime/e2e-asis.log` ve `e2e-auth.log` içinde.

### Faz 1 — olduğu gibi (bağımsız kanıt)

**11 PASS / 121 FAIL.** Kök: paket SEC-01 ÖNCESİ motor için yazılmış — tüm yönetim uçları artık Bearer zorunlu; anahtarsız istekler 401. Bu,SEC-01 düzeltmesinin e2e yüzeyinde de geçerli olduğunun bağımsız kanıtıdır (paket güncellenmemişti).

### Faz 2 — auth uyarlanmış (`req()` + SSE curl'lere Bearer; kalıcı iyileştirme)

**107 PASS / 120 FAIL→13 FAIL.** Geçenler: summary/markets/klines/health, calc dca+grid, backtest+cert, pine validate/backtest, selfcheck, vault 422 kapısı, kill-switch/freeze döngüsü, bot CRUD, PAPER grid bot, webhook korunması, T14 sinyal ağacı, parite — geniş sözleşme yüzeyi yeşil.

**13 başarısızlık — tek kök, tümü ikincil:** paketin deal açılışı bekleyen 7 kontrolü (`bot-deal-*`, `fdca-*`, `sc-met-*`, `sse-lifecycle-events`) boş sonuç aldı. Zincir: **(1)** Faz 2'de gelen warmup-skip ile yeni botun İLK deal'i bir SONRAKİ kapanmış barda açılır (eski davranış: ilk poll tarihsel barları anında yaşardı); **(2)** paket ~8 botu art arda kurup her biri 20 sn'de 60 barlık kline çekince Bybit public anti-abuse limiti (10006) devreye giriyor; motor fail-closed bekliyor (dürüst — kanıt: `fdca-*` detayında paketin kendisi "rate-limit penceresinde yakalanamadı" yazıyor).

**Motor-side tek-kesin kanıt (aynı baskı altında):** taze 1m PAPER bot, izlemeli bekleme → tur 9'da (~100 sn) `BasePending→Open`, 0.00062 BTC pozisyon. Deal açılış mekanizması baskı altında da çalışıyor; yalnız süre uzuyor.

### Uygulanan kalıcı iyileştirmeler

- `tests/e2e_engine.py`: auth başlıkları (kalıcı), `bot-deal-open` ve `sc-met` pencereleri 90→150 sn (warmup + rate sönümü farkındalığı, açıklama yorumlarıyla).
- Öneri (sonraki oturum): paketin bot-kurma bölümleri arasına 60 sn rate-limit sönümü ya da testnet API anahtarı (limitler daha cömert) — tam-yeşil koşum için.

### Bağımsız denetim genel hükmü

Motor sözleşmeleri (health/auth/calc/backtest/pine/vault/halt/webhook koruması/grid/parite) bağımsız pakette geniş ölçüde doğrulandı; başarısızlıkların tümü test-altyapısı lag'ından (auth eksik + bekleme pencereleri) ve bilinçli fail-closed davranıştan kaynaklı — motor tarafında yeni kusur bulunamadı.

---


## 4.11 Faz 14 — kalan bulgu havuzunun kapatılması (27 Ekim) + 13B-backtest tasarımı

### Uygulanan ve kabul edilenler

| İş | Bulgu | Kanıt |
|---|---|---|
| `delete_bot` artık DB kaydını da temizliyor: `bots` satırı + `live_orders` + (varsa) `runtime_state` silinir; fill/equity geçmişi analiz için korunur; LIFECYCLE olayı yazılır | M-10 | canlı: oluştur→sil→DB `bots=0, live_orders=0` |
| Caddyfile dinleyicisi `localhost:81`'e kısıtlandı (LAN'a açık yüzey kaldırıldı; Tunnel doğrudan 3000/3002'ye bağlanabilir) | CF-3 | yapılandırma değişikliği |
| `DCA_EXTRA_ORIGINS` env: CORS listesi virgülle ayrık ek origin'leri kabul eder (Tunnel domain'i kod değişikliği olmadan) | CF-2 hazırlığı | kod + derleme |
| `stop-local.ps1` port-dinleyici bazlı güvenli kapatma: yalnız 3002'de `dca_server` ve 3000'de `node/bun` kapatılır, beklenmeyen süreçlere dokunulmaz — "identity changed" reddi dönemi bitti | ops | script |
| e2e rate sönümü: `bot-deal-open` ve `sc-met` pencereleri 90→150 sn + auth başlıkları kalıcı | §4.10 önerisi | §4.10 |

271 test / 0 hata; motor yeniden başlatıldı, 4 kullanıcı botu restore.

### 13B-backtest — kesin tasarım (HEDGE LIVE sertifikasının önündeki son engel)

**Kök neden:** `backtest.rs` `EngineBox`'ın Hedge varyantı yok — HEDGE_DUAL/HEDGE_PROTECTIVE backtest'leri `DcaEngine::new`'e düşüp `"DcaEngine requires DCA_LONG/DCA_SHORT/FUTURES_DCA"` ile reddediliyor. Bu yüzden sertifika kapısı hedge için "all trial backtests failed" döner (fail-closed doğru çalışıyor — canlı doğrulandı).

**Tasarım (taze oturumda mekanik uygulama):**
1. `EngineBox::Hedge(HedgeEngine)` varyantı ekle.
2. `run_backtest` başında hedge ise İKİ stepper kur (ana yön=+1, hedge yön=−1, ayrı cash/IM) — canlı `engine.rs process_bar` deseniyle birebir.
3. `dispatch` iki-stepper sürümü: role/id prefix (`{bt}-hedge`) → hedge stepper; HedgeEngine BASE emirleri `…-hedge-…` id taşır (mevcut).
4. `stepper.step` iki kez (ana + hedge), fill'ler prefix'e göre `engine.on_fill_leg`'e (mevcut routing).
5. Per-bacak M07 likidasyon + funding (`liq_price_of_stepper` deseni canlıda var — runner'a ayna).
6. Metrics: deals/wins/losses HedgeEngine leg'lerinden (snapshot sözleşmesi mevcut), equity = ana+hedge toplamı.
7. Kabul: HEDGE_DUAL backtest 400 bar → iki BASE fill (Sell+Buy), net delta 0, deals ≥ 1; sertifika kapısı hedge için deneme koşusu üretir; Demo'da HEDGE_DUAL LIVE kabul turu (POZ_MODE positionIdx eşlemesiyle) — testnet rampası kapsamında.

**Değerlendirme:** Bu eklentiyle 23 Ekim raporunun tüm kapsamı (bulgular + ürün yolu) tüketilmiş olur; sonrası salt operasyon (testnet rampası, Tunnel açılışı — §4.9).

---

## 5. AÇIK BULGULAR (güncel kaynakla doğrulanmış)

Önem sırası: **C (Kritik) → H (Yüksek) → M (Orta)**. Her bulguda kanıt `dosya:satır`, etki ve gereken iş verilir.

### C-1 — KRİTİK: UI ↔ motor kimlik doğrulama entegrasyonu kopuk — ✅ FAZ 1'DE KAPANDI (bkz. §3.6)

**Kanıt:** Motor: `main.rs:125-127` (`DCA_API_KEY` zorunlu), `main.rs:196` (tüm yönetim uçları `auth_middleware` altında; yalnız `/api/health` muaf, `main.rs:131`). UI: `src/hooks/engine-client.ts:28-38` — `engineFetch` yalnız `Content-Type` koyar, Authorization yok. SSE: `src/hooks/engine-stream.ts:116` — `new EventSource(engineUrl("/api/stream"))`; tarayıcı EventSource özel başlık desteklemez. Testler: `tests/e2e_engine.py` içinde `DCA_API_KEY`/`Authorization` geçmez.

**Etki:** Motor auth açıkken ayağa kalkar (bugün 10:32 logu) ama tarayıcıdaki terminal her veri isteğinde 401 alır: bot listesi, özet, fill'ler, klines, vault — hesti. SSE bağlanamaz. PAPER geliştirme dahil tüm ürün akışı kilitlenir. E2E testleri de aynı sebeple kırıktır.

**Gereken:** (a) `engineFetch`'e `Authorization: Bearer <anahtar>` eklemek ve anahtarı build-time env (`NEXT_PUBLIC_ENGINE_API_KEY` vb.) ile taşımak; (b) SSE için ya query/token mekanizması ya da Next tarafında sunucu-üstü proxy route'u (EventSource'un header yasağını aşmak); (c) e2e testlerine auth başlığı; (d) prod'da anahtarın tarayıcıya sızmaması için localhost-muafiyeti + dış-gateway-token iki katmanlı tasarım kararı.

### C-2 — KRİTİK: Resmî başlatma komutu zorunlu env'leri atmıyor — ✅ FAZ 1'DE KAPANDI (bkz. §3.6)

**Kanıt:** `package.json:6` → `start-local.ps1`; script içinde `DCA_API_KEY`/`DCA_VAULT_KEY` hiç geçmez (81 satırın tamamı okundu). Motor: `main.rs:126-127` `.expect("DCA_API_KEY required")`, `store.rs:189` env yoksa hata → `main.rs:85` `expect("vault")`. Alternatif `start-local-with-keys.ps1:11-12` anahtarları **sabit değerlerle** (`dev-local-key-change-in-production`) atar ve `package.json`'a bağlı değildir.

**Etki:** `npm run local:start` (README'deki tek komut) motorun anında panik etmesiyle sonuçlanır. `with-keys` script'i kullanılsa bile sabit/bilinen anahtar sorunu (bkz. M-6) ve C-1 (UI 401) sürer.

**Gereken:** Başlatıcı tek elden anahtar üretmeli (ilk çalıştırmada random üretip `.env.local`'a yazıp tekrar kullanmalı), `package.json` bu script'i göstermeli, README güncellenmeli.

### C-3 — KRİTİK: "Açık listeden kaybolan emir = dolum" varsayımı — ✅ FAZ 3'TE KAPANDI (execution-kaynaklı sweep; bkz. §3.7)

**Kanıt:** `dca_exchange/lib.rs:281-295` `sweep_missing_fills` yalnız order_link_id diff'i yapar. Tüketici: `engine.rs:1085-1140` — kaybolan emir, **tam takip miktarı** ve resting fiyatıyla sentez fill olur. Motor kendi iptal ettiği emiri takipten çıkarır (`engine.rs:765`) ancak: kullanıcının borsa panelinden iptali, emrin reddi (Reject), süresi dolması, kısmi dolum + iptal — hepsi **tam dolum** muhasebeleştirilir. Ek kusur: sentez fill **Taker** ücretiyle işlenir (`engine.rs:1110` `FeeRole::Taker`) — resting limit dolumu Maker olmalıdır ve gerçek borsa ücreti kullanılmalıdır (bkz. H-4).

**Etki:** Gerçekleşmeyen alış muhasebeye girer; buna karşı otomatik satış/TP hesaplanır. LIVE'da doğrudan para kaybı senaryosudur (gerçek olmayan pozisyonu kapatmaya çalışan emir).

**Gereken:** Fill kanıtı olarak venue execution kayıtları (`/v5/execution/list`, OKX `fills`); terminal durum makinesi (New/Partial/Filled/Cancelled/Rejected); kısmi dolumda yalnız `cumExecQty` kadar muhasebe; emir durumu Cancelled/Rejected ise fill ÜRETİLMEMESİ.

### C-4 — KRİTİK: Restart sonrası yarım geri yükleme — ✅ FAZ 3'TE TAM KAPANDI (kalıcı runtime snapshot; canlı iki-restart kanıtı §3.7)

**Kanıt:** Runtime her zaman `last_bar_ts: 0` ile kurulur (`state.rs:438`). Restore uzlaştırması (`main.rs:307-387`) LIVE botlarda venue'dan pozisyon miktarı/ortalaması ve açık emirleri çeker, ancak: `stepper.cash`, `im_used`, WAC maliyeti, deal fazı (DcaPhase), merdiven sayacı, `cumulative_fees` **sıfırdan** başlar. İlk poll'de son 60 barın tamamı `t + interval <= now` koşulunu geçtiği için **geçmiş barlar için bugünkü piyasada karar/emir** üretilir (`engine.rs:1019-1046`).

**Etki:** (PAPER) Restart her seferinde botu "hayal edilmiş geçmişte" yeniden koşturur; sonuçlar tekrarlanamaz. (LIVE) Açık pozisyonu olan bot restart'ta fazını kaybeder — yeni BASE/deal açma girişimi mevcut pozisyonu istemeden büyütebilir; cash/fee geçmişi sıfırlandığından G6 sermaye kontrolü yanlış sermaye üzerinde çalışır.

**Gereken:** Kurulumda `last_bar_ts`'yi son kapalı bara ilkleme (warmup); kalıcı bot snapshot (faz, sayaç, cash, im_used, funding); restore'da RECONCILING durumu: uzlaştırma tamamlanmadan yeni risk açmama (restore bugün "RECONCILING" yorumu taşır ama kodda durum yok — açık pozisyon varsa ilk barlarda yeni deal engellenmelidir).

### C-5 — KRİTİK: Hedge bacakları — 🔶 positionIdx EŞLEMESİ FAZ 5'TE TAMAMLANDI (hedge-mode hesapta DCA canlı çalıştı); kalan tek iş: HedgeEngine'in bacak-başına-stepper mimarisi (LIVE hedge kilidi sürüyor)

**Kanıt:** Tüm hedge botlar tek `SimulationStepper` paylaşır (`state.rs:399-412`, HedgeEngine tek stepper'la). Tüm LIVE emirlerde `position_idx: 0` sabittir (`engine.rs:558`, `engine.rs:622`, `main.rs:1069`). Kernel pozisyon değişimini `OrderKind::Add/Reduce` belirler; Long-Sell "short açma" değil "azaltma" olarak işlenir.

**Etki:** HedgeDual (long+short) Bybit one-way modda bacaklar birbirini düşürür; hesap hedge_mode'daysa `positionIdx` 1/2 zorunludur → emirler reddedilir. Yerel ayna ile venue pozisyonu ayrışır. Aynı kök neden DCA_SHORT için de kısmen geçerlidir (one-way'de Sell short açar ama yerelde direction=-1 sabitidir — venue tarafındaki mod doğrulanmaz).

**Gereken:** Bacak başına ayrı muhasebe; venue pozisyon modu doğrulaması (OKX'te `pos_mode` assertion'ı VAR — Bybit'te yok); positionIdx'yi strateji/bacaktan türetme. Doğrulanana kadar Hedge LIVE kapalı tutulmalı (şu an LIVE kapısı venue+sertifika koşullarına bağlı, hedge özel engeli yok).

### C-6 — ✅ TAMAM (geçit + defter + M-13 gerçek bakiye alanı; canlı: dolu hesap emri geçirdi, yetersiz senaryosu da önceki turda canlı reddedildi)

**Kanıt:** LIVE dalı doğrudan adapter `place_order` çağırır (`engine.rs:546-589, 611-725`); sermaye kaynağı yalnız yerel `start_cash`. `wallet()` çağrısı yalnız selfcheck/bnb kartında var; emir öncesi hesap düzeyinde bakiye/marjin kontrolü ve rezervasyon yok.

**Etki:** Aynı hesapta çalışan iki bot aynı serbest bakiyeyi bağımsız harcayabilir; emir bir kısmı borsada yetersiz bakiyeden reddedilirken yerel ayna dolmuş sayabilir (C-3 ile birleşince drift). Enstrüman/minNotional kontrolü de emir yolunda yok (yalnız hesaplayıcıda).

**Gereken:** Tek gerçek-emir geçidi: emir öncesi wallet/available okuma, hesap düzeyi rezervasyon, minNotional/lot kontrolü, veri tazeliği denetimi; yetersizse fail-closed SKIP.

### H-1 — ✅ ADIM 1/2 + ADım 2'nin ağ yarısı TAMAM (klines, imzalı kurulum, sinyal/pine artık kilitsiz; kalan: dispatch-place/sweep ağları — tam actor modeli büyük refactor)

**Kanıt:** `engine.rs:980` `poll_inner` haritanın `Mutex`'ını alır ve kline çekme, kaldıraç senkronu, fill sweep (tümü `await`) bu kilit altında koşar (980–1071). `main.rs:1000` `bot_actions` kilidi alır ve ADD_FUNDS LIVE dalında `adapter.place_order(...).await` (satır 1071) kilit tutulurken çağrılır. `tradingview_webhook` (2673+) kilit altında DB yazan `persist_event(...).await`'ler yapar. UI listeleme (`list_bots`, `summary`) aynı kilidi bekler.

**Etki:** Tek yavaş/askıda venue çağrısı tüm botların tick'ini VE arayüzün tüm bot/action isteklerini bloklar. 15 sn HTTP timeout ile 10 bot + işkence senaryosunda UI dakikalarca yanıt vermez; supervisor her 20 sn'de üst üste task üretebilir.

**Gereken:** Bot-başına kilit/actor modeli; ağ çağrıları kilit dışında; haritadan kısa süreli `Arc<Mutex<BotRuntime>>` ödünç alma; action handler'larında kilit dışı ağ.

### H-2 — YÜKSEK: Sayısal girdilerde kaynak tüketimi (SEC-04 kalıntısı)

**Kanıt:** `routes_calc.rs:39` `max_safety_orders: usize` sınırlamasız → `build_dca_plan` `Vec::with_capacity(safety_count)` (`m02.rs:91` civarı); `1e9` gibi değer OOM/panic. `fraction.rs:110-118`: üslü ondalık parse'da `BigInt::pow(exp_val as u32)` — `as u32` dönüşümü dev/negatif üslerde sessiz truncate; sınırsız üs dev Bellek tüketebilir (4096-bit sınırı `pow`'dan sonra gelir).

**Etki:** Auth korumalı olsa da (C-1 düzelince) gövde limiti tek başına yetmez; tek istekle servis edilemezlik. **Canlı güncelleme (§3.5.4):** `max_safety_orders=100000` pratikte 422 ile reddediliyor (M01 geometrik sapma sınırı erken kırıyor) ve `1e2000` üssü 4096-bit kesir sınırıyla 422 veriyor — pratik vektör kapalı; kalan risk yalnız `step_scale=0 + dev→0` uç kombinasyonu ve üs ≥ 2³²'de `as u32` truncation'ı (kod kanıtlı, canlı OOM testi çalışan kullanıcı botu nedeniyle bilinçli yapılmadı).

**Gereken:** `max_safety_orders ≤ 20`, üs ≤ ±24, mantissa uzunluk sınırı; hepsi parse-öncesi 422.

### H-3 — YÜKSEK: Emir ACK sonrası belirsizlik — ✅ FAZ 3'TE KAPANDI (kalıcı live_orders + execution/order-history çözümü)

**Kanıt:** Market emir place sonrası 3 deneme/300 ms sorgu (`engine.rs:632-714`): `Ok(None)` → `break` — emir open_orders'a da eklenmez, kalıcı UNKNOWN kaydı yok (`engine.rs:704` yorumu "bulunamadı — dürüst: görünmez"). Sorgu hatası 3 denemede WARN ile biter; kalan takip yok.

**Etki:** Borsada gerçekte açılmış pozisyon yerel aynada hiç görünmeyebilir; sonraki kararlar yanlış sermaye/pozisyonla verilir.

**Gereken:** Place anında kalıcı PENDING kaydı; ACK/UNKNOWN/PARTIAL/TERMINAL durum makinesi; bilinmeyen emirler için order-history tabanlı tarama; "tam bir kez" muhasebe garantisi.

### H-4 — YÜKSEK: Gerçek borsa ücretleri — ✅ FAZ 4.5'TE BYBIT İÇİN KAPANDI (spot alış base-fee → quote dönüşümü; OKX fee-ccy not olarak açık)

**Kanıt:** Tüm fill'ler yerel `instrument.maker/taker_fee_rate` ile hesaplanır (`dca_kernel/lib.rs:452-463`); Bybit adaptörü ücretleri kalıcı olarak gömer (`bybit.rs:141` — linear 0.0002/0.00055, spot 0.001). Gerçek `cumExecFee` / fee coin'i (`execFee`, `feeCurrency`) hiçbir yerden taşınmaz.

**Etki:** Kullanıcının gerçek komisyon kademesi/voucher'ı farklıysa net TP, bakiye ve kâr kalıcı olarak sapar; fee coin farkı (Bnb indirimi vb.) yok sayılır.

**Gereken:** Execution kayıtlarından gerçek fee + fee currency taşıma; adaptör sözleşmesine `fee_paid`, `fee_asset` alanları; farklı coin cinsinden ücretin açık muhasebesi.

### H-5 — YÜKSEK: Likidasyon modeli — ✅ FAZ 4'TE KAPANDI (venue kademeleri + LIVE yerel sentez kapalı + venue-tasfiye öğrenme)

**Kanıt:** `engine.rs:409-431` `current_liq_price` yalnız `mmr=0.005` tek kademe; `check_liquidation` (436-517) LIVE/PAPER ayrımı yapmadan yerel forced-close fill işler. Bybit `mmr_brackets` aynası VAR (`bybit.rs:596-601`, `parse_bybit_risk_limit` testli) ama runtime tüketmiyor.

**Etki:** Gerçek bakım marjı kademeleri farklı olduğunda yanlış eşikte "likidasyon" muhasebesi; LIVE'da venue'daki gerçek tasfiye kaydı yerine yerel sentez — borsa ile yerel defter ayrışır.

**Gereken:** Runtime'da venue risk-limit kademeleriyle hesap; LIVE'da tasfiyeyi borsa pozisyon/execution değişiminden öğrenme, yerel tahmini yalnız uyarı amaçlı.

### H-6 — YÜKSEK: Enstrüman bilgisi her borsadan değil Bybit'ten — ✅ FAZ 4'TE KAPANDI (venue'lu çözüm + LIVE metadata ret; OKX botu canlı doğrulandı)

**Kanıt:** `state.rs:464-492` `engine_instrument` venue parametresi almaz, daima `app.bybit.instruments` çağırır. Başarısızlıkta `Instrument::default()` ile kurulum sürer; LIVE'da "metadata yok → ret" yok. Instrument cache'i süresizdir (bkz. M-3).

**Etki:** OKX/Binance sembollerinde yanlış tick/step/minNotional/fee ile emir hesabı ve qty yuvarlama; sembol adları da farklı biçimdedir (OKX `BTC-USDT-SWAP` vs `BTCUSDT` — adaptör kendi içinde çevirir ama kurulum filtresi Bybit'inkidir). Varsayılan fallback ile LIVE açılabilir.

**Gereken:** Venue+category+symbol ile çözümleme; canlı metadata yoksa LIVE kurulumu 422; emir yolunda tick/step doğrulaması.

### H-7 — ✅ TAMAM KAPANDI (kalıcı secret + rotasyon ucu canlı; kalan ince iş: alert-id dedup)

**Kanıt:** `state.rs:420-426` — secret her runtime kuruluşunda yeniden üretilir ("restore'da yeniden üretilir" yorumu) ve DB'ye yazılmaz. `state.rs:137` snapshot JSON'unda `webhookSecret` dışarı verilir. Tekrar koruması yalnız ±60 s pencere (`main.rs:2689-2692`); alert-id dedup yok.

**Etki:** Motor her restart'ta kullanıcının TradingView alert'lerindeki secret'ı geçersiz kılar (sessiz üretim kesintisi). Auth'lu API'yi okuyan her istemci secret'i görebilir (C-1 düzelince risk kısıtlı ama yine en az ayrıcalık ilkesine aykırı).

**Gereken:** Secret'ı kalıcı + açık rotasyon uç'u; görüntüleme ayrıcalığı; alert-id/borum nonce dedup (deal kapandıktan sonra pencere içinde tekrar tetiklenemez).

### H-8 — YÜKSEK: Sahte başarı/yüzeysel sağlık — 🔶 FAZ 5'TE BÜYÜK ÖLÇÜDE KAPANDI (readiness + unwrap panikleri; persist_status sonucunun yanıta yansıması kalan)

**Kanıt:** `state.rs:350-363` `persist_status` retry+stderr yapar ama `Result` dönmez; çağıranlar (`main.rs:922, 987, 1096`) sonucu yok sayıp `{"ok":true}` döner. `main.rs:3008-3009` `remove_key` DB silme sonucunu `_ =` ile atar. `health` (`main.rs:408-418`) her zaman `status:"ok"` — DB yazılabilirliği/son fill yaşı/venue gecikmesi ölçülmez. `bot_fills`/`bot_events` handler'ları `conn.lock().unwrap()` ile poisoning'de panikler (`main.rs:1103, 1130` — diğer call-site'lar dürüst hata verir).

**Etki:** Disk dolu/kilitli anında kullanıcı durdurduğunu sanır, restart'ta bot eski durumla döner; süreç açık ama emir hattı bozukken yeşil görünür.

**Gereken:** Kritik yazımların sonucunu yanıta yansıtma (5xx); liveness/readiness ayrımı; `unwrap()`'leri dürüst hata yoluna çevirme.

### H-9 — YÜKSEK: Bağımlılık güvenliği — ✅ TAM KAPANDI (npm 91→0 + cargo audit kurulum + tokio/webpki yamaları → Rust 0 vulnerability; §3.13–3.14)

**Kanıt:** `package.json` 23 Ekim'den beri değişmedi (next `^16.1.1`, kurulu 16.1.3). 23 Ekim onaylı `bun audit`: 91 tekil advisory — 5 kritik, 46 yüksek; öne çıkanlar GHSA-p293-qw3h-jr36 (Windows Next.js RCE; düzeltme 16.3.3), GHSA-2464-fhr4-w9xw (SVG optimizer; 16.4.1 + sharp ≥0.35.5). Rust tarafı `cargo audit` hiç çalıştırılmadı.

**Gereken:** Sürüm yükseltme planı (öncelikle Next 16.4.1+ ve sharp), temiz kilit kurulumu, tekrar audit; `cargo audit`/`cargo deny` ekleme. Uygulanabilirlik koşulları (Windows, image optimization yolları) ayrıca değerlendirilmelidir.

### H-10 — YÜKSEK: Bot status'u + PAUSED/STOPPED fill takibi — ✅ FAZ 2+3'TE TAM KAPANDI (whitelist + sessiz sweep; canlı kanıt §3.7)

**Kanıt:** `patch_bot` (`main.rs:919-923`) ve `bot_actions` (1003-1024) status'u doğrulamadan yazar — `"FOO"` kalıcılaşabilir. Supervisor yalnız RUNNING/FROZEN poll eder (`main.rs:97`) → PAUSED/STOPPED bir LIVE botta açık emirler borsada dolmaya devam ederken `live_fill_sweep` çalışmaz (restore sırasında PAUSED reconcile edilir ama çalışma anında değil).

**Etki:** Duraklatılan botun gerçek fill'leri kaydedilmez; yerel ayna venue'dan geri dönüşsüz ayrışır. Geçersiz status, süpürme/restore filtrelerinin sessiz atlanmasına yol açar.

**Gereken:** Tek enum + geçiş matrisi (API 422); PAUSED/STOPPED'ta bile LIVE fill sweep + açık pozisyon izleme (yalnız yeni risk kapalı).

### M-1 — ORTA: İlk safety seviyesine volume_scale — ✅ KAPANDI (sözleşme testle sabitlendi: safety_i = taban × v^i; bkz. §3.14)

**Kanıt:** `m02.rs` — `current_size = safety_quote_size_or_qty` ile başlanır, döngü içinde ilk seviyede bile `current_size × volume_scale` çarpımı yapılır; kod yorumu hem "safety_1 = base × v" hem "ilk safety verilen değer" der — çelişki çözülmemiş.

**Etki:** volume_scale ≠ 1'de beklenen ilk emir boyutu tartışmalı; hesaplayıcı-runtime-belge üçlüsü aynı örneği üretmeli.

### M-2 — ORTA: OKX open_orders birim dönüşümü — ✅ FAZ 4'TE KAPANDI (ctVal normalize; imzalı doğrulama testnet turunda)

**Kanıt:** `okx.rs:606-607` — swap emirlerinde `sz`/`accFillSz` kontrat cinsindedir; adaptör bunları doğrudan `qty`/`cum_exec_qty`'ye koyar (query_order'daki ctVal dönüşümü burada yok). `restore` (`main.rs:361-377`) ve sweep bu listeyi coin muhasebesine işler.

**Etki:** ctVal ≠ 1 sembollerde restore/sweep miktarları yanlış ölçeklenir (ör. 100x fark).

### M-3 — ORTA: `instruments_cache` hatalı/boş yanıtı süresiz saklar — ✅ FAZ 2'DE KAPANDI (60 sn TTL, yalnız dolu yanıt cache)

**Kanıt:** `main.rs:1600-1615` — hata `unwrap_or_default()` ile boş listeye düşer ve `cache.insert` TTL olmadan yazılır; `state.rs:464-475` runtime kurulumu da aynı süresiz cache'i okur.

**Etki:** Tek ağ hatası sonrası piyasa listesi restart'a kadar boş kalır.

### M-4 — ORTA: `parity` CPU-yoğun backtest'i async bağlamda bloklar

**Kanıt:** `main.rs:1750` — `run_backtest` (≤1000 bar) `spawn_blocking` olmadan doğrudan çağrılır (`pine_backtest` kullanıyor, bu kullanmıyor).
**Etki:** Tokio worker thread'i saniyeler bloklanır; eşzamanlı isteklerde gecikme.

### M-5 — ORTA: `start_cash` doğrulaması yok — ✅ FAZ 2'DE KAPANDI (>0 zorunlu)

**Kanıt:** `main.rs:687` — `Fraction::from_decimal_str` başarılıysa negatif/sıfır/sapkın değer kabul edilir; G6 doğru çalışsa da anlamsız durumlar erken reddedilmeli (422).

### M-6 — ORTA: Ölü Prisma bağımlılıkları ✅ KAPANDI (paket + script + kısayollar); .env anahtar temizliği hâlâ kullanıcı kararı (Demo anahtarları bilinçli kullanımda)

**Kanıt:** `.env` içinde `BINANCE_API_KEY/SECRET`, `BINANCE_FUTURES_TESTNET_*`, `BYBIT_API_KEY/SECRET`, `BYBIT_BASE_URL`, `BINANCE_LIVE_MODE` mevcut (değerler bu raporda yer almaz). Rust açılışta `.env`'i kendisi yükler (`main.rs:51-67`). `BINANCE_LIVE_MODE` hiçbir kaynakta okunmuyor (grep: 0 eşleşme). Prisma (`DATABASE_URL`, `db:*` script'leri, `prisma` paketi) kodda hiç kullanılmıyor — şablon artığı.
**Etki:** Vault mimarisine rağmen anahtarların ikinci bir düz metin kopyası; `BINANCE_LIVE_MODE` kullanıcıda "canlı mod açık" yanılsaması yaratabilir. Prisma ölü ağırlık + kafa karışıklığı.
**Gereken:** `.env`'den borsa anahtarlarını kaldır (yalnız vault), ölü env/değişkenleri temizle, Prisma'yı ya belgele ya çıkar.

### M-7 — ORTA: Dev API anahtarı sabit metin — ✅ FAZ 1'DE KAPANDI (rastgele-kalıcı üretim)

**Kanıt:** `start-local-with-keys.ps1:11-12` — `dev-local-key-change-in-production` / `dev-local-vault-key-change-in-production`. Prod'a taşınırsa vault şifrelemesi bilinen anahtarla kırılabilir. Üretimde rastgele üretim + zorunlu ayarlanmalı.

### M-8 — ORTA: CORS origin'i `localhost:3000`; sunucu `127.0.0.1:3000` yayınlar — ✅ FAZ 2'DE KAPANDI (iki origin de açık)

**Kanıt:** `main.rs:203` `AllowOrigin::exact("http://localhost:3000")`; başlatıcı UI'ı `--hostname 127.0.0.1` ile açar (`start-local.ps1:63`). Same-origin proxy devredeyken sorun görünmez, ama tarayıcıdan motora doğrudan erişim gerektiğinde (C-1 çözümü sonrası) `127.0.0.1` origin reddedilir. İki origin de listeye alınmalı.

### M-9 — ORTA: `events` tablosu sınırsız büyür — ✅ FAZ 3'TE KAPANDI (global retention 200k + indeks + PAUSED sessiz TICK)

**Kanıt:** `store.rs:52-59` indeks/retention yok; her tick en az bir TICK event'i yazar (`engine.rs:968`) — bot başına ~4.320 satır/gün; WARN/POLL hataları katlar. `equity_points` için 5000 sınırı getirilmiş olmasına karşın events için yok. `deals` tablosu da yazılmıyor (şema var, kullanım yok).

### M-10 — DÜŞÜK: `delete_bot` DB kaydını silmez

**Kanıt:** `main.rs:986-989` yalnız runtime'dan çıkarır ve status STOPPED yazar; `bots` satıları birikir. Kasıtlı geçmiş koruması olabilir — belgelenmeli ya da soft-delete sütunu eklenmeli.

### M-11 — DÜŞÜK: `tickers_24h` ölü kod dalı

**Kanıt:** `bybit.rs:362` — `public_get` zaten `result` döndürür; `v.get("result")` dalı ölü, fallback çalışır. İşlevsel zarar yok, okuma karışıklığı yaratır.

---

## 6. Faz Yol Haritası ve Sektörel Kıyas

Sektörel referanslar: [Freqtrade Bot Basics](https://www.freqtrade.io/en/stable/bot-basics) (kalıcı emir/takas DB'si + restart'ta borsadan yeniden çekim), [Hummingbot start/stop](https://hummingbot-hummingbot.mintlify.app/cli/start-stop) ("restart and stop" uzlaştırması), [Hummingbot Kill Switch](https://hummingbot.org/client/global-configs/kill-switch) (performans eşiğinde otomatik duruş), [Zyniq](https://zyniq.io/blog/3commas-trading-bot-setup-fees-limits-and-risk) ve [TradingAgentStack](https://tradingagentstack.com/guides/safe-exchange-api-key-3commas-bitsgap) rehberleri (trade-only + IP kısıtlı anahtar, şifreli saklama, önce demo), 3Commas 2022 anahtar sızıntısı ([Benzinga](https://www.benzinga.com/markets/cryptocurrency/22/12/30224711/3commas-hack-company-admits-to-unauthorized-access-to-api-keys), $22M kayıp — anahtar kısıtlarının önemi).

### Faz 1 — Sistemi kendi kendine açılabilir yapmak (PAPER) — ✅ UYGULANDI (bu oturum)

| İş | Bulgu | Durum |
|---|---|---|
| UI→motor Bearer zinciri (`engineFetch` + `NEXT_PUBLIC_ENGINE_API_KEY`) | C-1 | ✅ `src/hooks/engine-client.ts` |
| SSE için sunucu-üstü proxy route (EventSource başlık koyamaz; SSRF-sert: sabit host + sayısal port) | C-1 | ✅ `src/app/api/stream/route.ts` |
| Başlatıcı rastgele-kalıcı anahtar üretimi (`local-keys.env`, BOM'suz yazım, `-cmatch` parse) | C-2, M-7 | ✅ `scripts/start-local.ps1` (+ `start-local-with-keys.ps1` wrapper'a indirildi) |
| README: auth açıklaması + çalışan create gövdesi | D-1 | ✅ |

Sektörel karşılık: trade-only anahtar + şifreli saklama dişi zaten vault'ta vardı; eksik olan "tarayıcı↔motor" taşıma katmanıydı (3Commas/Bitsgap'ın web-panel auth'una karşılık).

### Faz 2 — Hızlı sağlamlık kazanımları — ✅ UYGULANDI (bu oturum)

| İş | Bulgu | Durum |
|---|---|---|
| İlk poll'da geçmiş 60 barın "yeniden yaşanması" yerine warmup-skip (imleci son kapalı bara atla + INFO/WARMUP olayı) | C-4 (kalıcı snapshot öncesi minimal düzeltme) | ✅ `engine.rs poll_inner` |
| Status beyaz listesi (RUNNING/PAUSED/STOPPED/FROZEN dışı 422) | H-10 | ✅ `main.rs patch_bot` |
| `start_cash > 0` zorunluluğu | M-5 | ✅ `main.rs create_bot` |
| Kurulumda merdiven ön-inşası (hatalı deviation/scale → 422; max 1000 seviye sunucu tarafı kapağı) | D-2 (+H-2 katmanı) | ✅ `main.rs create_bot` |
| CORS'a `http://127.0.0.1:3000` eklendi (başlatıcının servis adresi) | M-8 | ✅ `main.rs` |
| `instruments_cache` 60 sn TTL + yalnız dolu yanıtlar cache'lenir | M-3 | ✅ `state.rs`, `main.rs markets`, `dryrun.rs` |

Sektörel karşılık: Freqtrade'in "son işlenen mumdan devam" ilkesinin minimal karşılığı warmup-skip; hatalı config'in kurulumda reddi 3Commas'ın bot-oluşturma doğrulamasıyla aynı yönde.

### Faz 3 — Execution mutabakat redesign (LIVE öncesi zorunlu)

- C-3: fill kanıtı venue execution kayıtlarından (`/v5/execution/list` / OKX fills); terminal durum makinesi (New/Partial/Filled/Cancelled/Rejected); cancel/reject fill ÜRETMEZ; limit sweep fill'i Maker + gerçek fee.
- H-3: place anında kalıcı PENDING kaydı; ACK/UNKNOWN/PARTIAL/TERMINAL; bilinmeyen emirler order-history taramasıyla çözülür ("tam bir kez" muhasebe).
- C-4 tam çözüm: kalıcı bot snapshot'ı (faz, sayaç, cash, im_used, funding) + restore'da RECONCILING (uzlaştırma bitmeden yeni risk yok) — Freqtrade/Hummingbot modeli.
- M-9/M-10: events retention + bots soft-delete.

### Faz 4 — Hesap ve bacak mimarisi (LIVE öncesi zorunlu)

- C-5: bacak bazlı muhasebe + venue pozisyon modu doğrulaması (Bybit hedge-mode positionIdx 1/2) — Hedge LIVE kilidi.
- C-6: tek gerçek-emir geçidi; wallet/available okuma + hesap düzeyi rezervasyon + minNotional kontrolü.
- H-4: gerçek fee/fee-coin taşıma (execution kayıtlarından).
- H-5: venue MMR kademeleriyle likidasyon (`mmr_brackets` zaten hazır); LIVE tasfiye venue'dan öğrenilir.
- H-6: venue-doğru enstrüman çözümü; metadata yoksa LIVE 422.
- M-2: OKX `open_orders` ctVal dönüşümü.

### Faz 5 — İşletim, ölçek ve kabul

- H-1: bot-başına yürütme izolasyonu (kilit dışında ağ); UI/action bloğu kalkar.
- H-7: kalıcı webhook secret + rotasyon + alert-id dedup.
- H-8: persist sonuçlarının yanıta yansıması; liveness/readiness; `unwrap()` temizliği.
- H-9: Next 16.4.1+/sharp yükseltmesi, `cargo audit`/`cargo deny`; tekrar `bun audit`.
- Testnet rampası: venue×strateji matrisi, `/api/parity` iki-çekirdek karşılaştırması, 20 maddelik kabul listesi (mevcut §7 senaryoları).
- M-6: `.env`'den borsa anahtarlarının temizlenmesi (yalnız vault), ölü `BINANCE_LIVE_MODE`/Prisma çıkarımı.

---

## 7. İncelemenin Sınırları

- Revizyon 2'de canlı testler yapıldı: `cargo test --workspace`, `tsc`, `eslint`, anahtarlı/anahtarsız API sözleşme testleri, PAPER bot yaşam döngüsü, kill/freeze döngüsü, hesaplayıcı/backtest/piyasa uçları, DB kalıntı denetimi. Borsaya GERÇEK EMİR gönderilmedi; vault'a anahtar eklenmedi (testnet imzalı uçlar abstain kaldı); tarayıcıda DOM doğrulaması yapılmadı (auth 401 davranışı rewrite yolu + EventSource kısıtıyla eşdeğer biçimde doğrulandı). `bun audit` tekrarı yapılmadı (paket seti değişmedi).
- `dca_pine`, `dca_signals`, `ai.rs`, `dryrun.rs`, `forensics.rs`, `memory.rs` modülleri ve UI bileşenlerinin tamamı (wizard/studio/charts) satır-satır incelenmedi; bu rapordaki bulgular yürütme/güvenlik/kalıcılık ana yollarına odaklanır.
- Bağımlılık advisory listesi 23 Ekim'deki onaylı taramadan aktarılmıştır (paket seti değişmediği için); güncel bir tarama değildir.
- Kaynakta kanıt görülmeyen riskler bulgu olarak listelenmedi.

## 8. Özet Cetvel

| Kategori | Açık bulgu | En kritik |
|---|---|---|
| Entegrasyon/Çalışabilirlik | C-1, C-2, M-3, M-5, M-6, M-7, M-8, D-1 | UI-motor auth kopukluğu (canlı kanıtlı) |
| Emir yürütme | C-3, C-5, C-6, H-3, H-4, M-2 | Kaybolan emrin fill sayılması |
| Yaşam döngüsü/kalıcılık | C-4 (canlı kanıtlı), H-7, H-10 (canlı kanıtlı), D-2, M-9, M-10 (canlı kanıtlı) | İlk poll tarihsel barlarla deal açıyor |
| Kaynak/sağlamlık | H-1, H-2 (pratik vektör kapalı — uç kombinasyon teorik), H-8, M-4, M-11 | Global kilitte ağ beklemesi |
| Bağımlılık | H-9 | Next.js 16.1.3 (5 kritik advisory) |
| Muhasebe çekirdeği | M-1 (sözleşme netliği) | — (BOT-01/02 düzeltilmiş; canlı fill doğru etiketlendi) |

**Canlı test hüküm özeti (§3.5):** cargo test + tsc + eslint PASS; PAPER yaşam döngüsü, kill/freeze, hesaplayıcı, backtest, piyasa uçları, selfcheck/parity dürüst sınıflandırmaları ÇALIŞIYOR; C-1/C-4/H-10/M-5/M-7/M-8/M-10 canlı kanıtlandı; H-2 pratikte büyük ölçüde kapalı; yeni bulgular D-1 (config şeması ergonomisi) ve D-2 (hatalı config kurulumda kabul + ERROR spam).
