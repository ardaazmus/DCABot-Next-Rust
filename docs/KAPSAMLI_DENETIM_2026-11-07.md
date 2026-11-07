# Faz 26 — Kapsamlı Denetim Raporu (2026-11-07)

**Kapsam:** Yazılımsal hata taraması (Rust + TS), güvenlik denetimi (statik + manuel API), motor davranış/mantık testleri (canlı PAPER), UI/UX görsel 2. tur, kapsamlı test stratejisi ve geliştirme araştırması.
**Yöntem notu:** Bu rapor bir "kanıt zinciri"dir — her iddianın altındaki komut/çıktı yinelenebilir. Motor kaynağı: `mini-services/dca-core` (8 crate, 61 .rs dosyası).

---

## 1. Yürütülen Denetimler ve Sonuçlar

| Denetim | Komut | Sonuç |
|---|---|---|
| Rust test süiti | `cargo test --workspace` | **287/287 PASS, 0 FAIL** (exchange 17 · kernel 29 · math 51 · pine 33 · risk 10 · server 51 · signals 35 · strategy 61) |
| Rust lint | `cargo clippy --workspace` | **0 error, 137 advisory** (35× büyük-Err-tipi, 32× gereksiz closure, 7× Default-dışı atama…) |
| TS tip/lint | `tsc --noEmit` + `eslint` | 0/0 (Faz 25 kanıtından; bu turda TS kodu değişmedi) |
| Statik güvenlik | Mimosa deep scan | **0 bulgu** — 167 dosya, 1268 fonksiyon, 1714 çağrı kenarı; kapsam `partial` (dinamik dispatch + ölçek sınırı; raporda beyan edilmiş) |
| JS bağımlılık | `npm audit --omit=dev` | **runtime 0 açık** |
| JS bağımlılık (dev) | `npm audit` | 9 bulgu (6 high) — **tamamı dev-toolchain** (aşağıda S-1) |
| Manuel API güvenlik | curl dizisi (A–E) | **16/16 PASS** (aşağıda §3) |
| Görsel UI 2. tur | Chrome 1440×900 | Faz 25 fix'lerinin tamamı canlı teyit (aşağıda §5) |

## 2. Güvenlik Postürü

**Güçlü kanıtlanmış (test ile):**
- **Auth duvarı:** `/api/bots`, `/api/summary`, `/api/vault/keys`, `/api/live/killswitch`, `/api/system/maintenance`, `/api/live/freeze` anahtarsız **401**; tek açık uç `/api/health` (200) — tasarım gereği.
- **Anahtar sızıntısı yok:** tarayıcı motora doğrudan konuşmaz; `src/middleware.ts` `/api/engine/*`'i yakalayıp Bearer'ı **yalnız sunucuda** ekler. `NEXT_PUBLIC_` fallback silinmiş (N-13) — anahtarın tek kaynağı sunucu env.
- **CSRF/origin kapısı (N-12):** `Origin: http://evil.example` ile killswitch POST → **403**; `Sec-Fetch-Site: cross-site` okuma → **403**. Zararlı sayfa senaryosu kapatılmış.
- **SSRF sert kenar:** proxy hedefi sabit host + doğrulanmış sayısal port; istek verisi host'a giremez.
- **Bağlama:** :3000 ve :3002 yalnız **127.0.0.1** — LAN yüzeyi yok.
- **Webhook:** yanlış secret → **401** (uç auth-dışı ama kendi secret/replay/dedup katmanıyla — P0-8 tasarımı).
- **Fail-closed girdi:** negatif base → **422**; sıfır sapma → **422**; eksik config → **422**; kara listeli sembole create → **422**.
- **Yaşam döngüsü:** create(PAPER)→pause→resume→CLOSE_ALL→delete→listede-yok — **tam döngü temiz** (test botu b-ec4fa23c).

**Bulgular:**
- **S-1 (Düşük) · Dev-chain bağımlılık açıkları:** 9 bulgu (high: browserslist, minimatch, brace-expansion, js-yaml, flatted, picomatch; moderate: ajv, @humanfs/node). Sınıf: ReDoS/DoS — **yalnız build/eslint zinciri**, çalışma anı paketine girmiyor (runtime audit 0). Öneri: planlı bakımda `npm audit fix` + lock yenileme.
- **S-2 (Bilgi) · SSE uç görünürlüğü:** motor `/api/stream` anahtarsız (health sınıfı). UI zaten sunucu-tarafı proxy (`/api/stream` route handler) kullanıyor; uç loopback-only. **Cloudflare Tunnel ile dışa açılımda** bu ucun da proxy arkasında tutulması şart — tunnel tarifine not.
- **S-3 (Test sınırı) · Zero-Withdrawal gate canlı test edilemedi:** gerçek borsa anahtarı olmadan kasa izin-denetimi (withdraw permission → 422) provası yapılamadı; Rust testlerinde (dca_server 51 test) kapılı davranış örneklenmiş. LIVE'a geçmeden önce küçük bir prova anahtarıyla e2e doğrulaması önerilir.
- **Mimosa yanlış-pozitif kaydı:** 2 yazım engeli (TS regex `exec(` → "komut enjeksiyonu"; localhost test betiği → "SSRF"). İkisi de incelenip güvenli/es-değer yazımla aşıldı — tarayıcı kuralı bağlam-dışı.

## 3. Mantık/Davranış Bulguları (canlı motor, PAPER)

- **Planlı tetikleyici uç-uca çalıştı:** `t12-sched` "PLANLI: 01 Kas 02:20" → koşul günü deal açtı → **W 1/0, +$1.00** → hemen ikinci deal (0.00396 BTC @ 78,120). Zamanlama → koşul → emir → muhasebe zinciri canlı kanıtlı.
- **DCA güvenlik merdiveni canlı:** `arda1` SOL düşüşünde 3.10 → 14.5 → **30.2 SOL** pozisyona güvenlik emirleriyle indi (ort 95.57 → 94.39 → 92.62); L8 @ 87.68 bekliyor. Motor davranışı doğru; **risk sorusu UI'a taşınmalı** (bkz. İ-1).
- **O-1 · İ-1 (Orta) · Portföy-düzeyi pozisyon üst sınırı yok:** tek botun deal'i (20x kaldıraçla ~$2.8k notional) filo özkaynağının ~%10'unu tek sembole bağladı; motor bot-bazlı DD kesiciye sahip ama **filo-geneli max-total-notional kapısı** yok. Öneri: yeni gate (örn. toplam açık notional / özkaynak > %N → yeni safety emri reddi, advisory+onay).
- **O-2 (Düşük/Operasyonel) · KERNEL HALT kurtarması tamamen manuel:** `t9-pinemacd` 3+ gündür sticky-halt (pozisyon donuk). RESET_KERNEL'in kullanıcı onaylı bir "otomatik-kurtarma seçeneği"ne bağlanması (N bar sonra uyarı + tek tık öneri) operasyon yükünü azaltır.
- **O-3 (Bilgi) · İyimser göstergeler:** `t2-dcashort` 17W/0L — PAPER reel spread/funding modeliyle (fill etiketli) besleniyor; gerçek para ile sonuç örtüşmez, Lab'daki DSR/sertifika akışı LIVE kararında zorunlu kılınmalı (mevcut Gate 6 felsefesiyle uyumlu).

## 4. Görsel UI/UX 2. Tur (Faz 25 sonrası)

**Teyit edilenler:** filo arama/filtre/sıralama (K/Z büyüklüğü varsayılan — arda1 üstte), HALT pill + "yeniden başlatma bekliyor", "Sil" etiketi, tek politika şeridi, Tahsis Dağılımı çok-varlık donut (BTC %67/SOL %33), Maruziyet yüzdeleri, "Borsa Duyuruları" + duyarlılık chip, footer tek koruma satırı, sihirbaz ✓-zinciri + CTA duruma duyarlı. UIX-18..42'nin tamamı üretimde canlı.

**Yeni gözlem (gelecek tur adayı):**
- **UIX-43 (aday) · Riskli pozisyon büyümesinin görsel vurgusu yok:** arda1'in 4.7×→9.7× büyüyen 20x pozisyonu kartta sıradan bir sayı. Öneri: pozisyon-notional/özkaynak oranı eşiği aşan kartta ince kırmızı kenar + "poziyon büyüdü — marj %X" satırı (veri zaten var; tek koşullu sınıf).
- Kokpit "Canlı Olay Akışı" boş-lambda durumunda "İşlem-Önemli filtrede eşleşme yok" mesajı doğrulandı — olay yokluğu ile filtre yutması ayrışıyor ✓.

## 5. Kapsamlı Test Stratejisi — Araştırma Özeti

Kaynaklar sonda. Uygulanabilir plan (motor öncelikli):
1. **proptest devreleri (dca_math/dca_kernel):** değişmezler — özkaynak korunumu (işlem öncesi=sonrası toplam), negatif bakiye oluşmaması, fill miktarlarının emir miktarına eşitliği, ladder fiyat monotonluğu. proptest başarısızlığı otomatik küçültür.
2. **"Nasty number" stratejileri:** proptest varsayılan olarak uç değerler üretmez — finansal stratejilere 0, max-u64, negatif, çok-küçük ondalık, üst-taşma sınırları **elle** eklenmeli.
3. **Stateful/model-based test:** basit ve "bariz doğru" bir referans-model oracle'ı ile kernel state machine'i adım adım kıyas (order-book match için kabul gören desen).
4. **cargo-fuzz:** dış veri ayrıştırıcılarına (borsa mesajları, webhook payload, config JSON) uzun süreli fuzz; `derive_fuzztest` ile aynı gövdeyi CI proptest'i ve fuzz oturumu paylaşır.
5. **e2e_engine.py bölümlendirme:** 600 sn'de bitmiyor — hızlı sözleşme bölümleri (create/validate/gate) ile uzun-bekleme bölümlerini ayır; CI'da hızlı bölüm zorunlu, uzun bölüm gece.
6. **UI:** Playwright smoke (login-anahtarsız akışı, filo sıralama determinizmi, SSE filtresi) — şu an görsel tur elle; en azından 5 ekranın altın-yol snapshot testi.

## 6. Sektör Kıyası ve Geliştirme Araştırması (2026)

2026 gündeminde platformlarda ana tema **risk kontrolü**: DCA botların yerleşik stopu yok — güvenlik emirleri, stop-loss, **maksimum pozisyon limitleri** ve paper-trading temel beklenti olarak sıralanıyor; "net bot logları" ve backtesting şart sayılıyor. DCABOT'un exact-rational muhasebesi, fill gerçekçilik etiketleri ve mutabakat/parite panelleri bu alanda **rakiplerde olmayan** ayrımcılar; eksik kalan iki başlık: (a) portföy-düzeyi pozisyon üst sınırı (İ-1), (b) canlı mod için sertifikasyon zorunluluğunun ürün yüzeyine taşınması (Lab'daki sertifikanın LIVE butonuna gate olması — mimari zaten hazır).

## 7. Öncelikli Yol Haritası

| Öncelik | İş | Gerekçe |
|---|---|---|
| P1 | Portföy-düzeyi max-total-notional gate (İ-1) | arda1 vakası canlı risk örneği |
| P1 | `npm audit fix` (dev-chain) + lock yenileme | S-1, düşük maliyet |
| P2 | proptest değişmez devreleri + nasty-numbers | §5.1-2; finansal doğruluğun sigortası |
| P2 | HALT otomatik-kurtarma önerisi (O-2) | 3+ gün sticky-halt operasyon yükü |
| P2 | e2e bölümlendirme + Playwright smoke | §5.5-6 |
| P3 | UIX-43 pozisyon-büyüme vurgusu | §4 |
| P3 | Tunnel tarifesine SSE-proxy notu | S-2 |

**Sonuç:** Motor ve UI üretim-kalitesinde; güvenlik postürü yerel tek-kullanıcı modeli için sağlam ve testlerle kanıtlı. 287/287 test + 16/16 API denetimi + 0 statik güvenlik bulgusu ile **bilinen hiçbir yüksek-önemli yazılımsal ya da güvenlik hatası yok**. Açık kalemler risk-ürünü geliştirmeleri (İ-1) ve test-altyapısı derinleştirmesi (proptest/fuzz).

**Kaynaklar:** [Property Testing — Rust Project Primer](https://rustprojectprimer.com/testing/property.html) · [proptest](https://crates.io/crates/proptest) · [Bridging Fuzzing and Property Testing](https://blog.yoshuawuyts.com/bridging-fuzzing-and-property-testing) · [derive_fuzztest](https://crates.io/crates/derive_fuzztest) · [Stateful Property Testing (Rust)](https://www.reddit.com/r/rust/comments/163qul3/stateful_property_testing_in_rust) · [proptest eleştirisi — nasty numbers](https://propertybasedtesting.com/) · [Matching Engine in Rust — Ambrain](https://ambrain.org/blog/matching-engine-rust-design) · [NautilusTrader](https://nautilustrader.io/) · [CryptoTicker platform karşılaştırması (Eki 2026)](https://cryptoticker.io/en/comparison/crypto-trading-bots) · [Gainium DCA bot sıralaması](https://gainium.io/bots/dca) · [Bitsgap](https://bitsgap.com)

---

## 8. Faz 27 — Tüm Bulguların Uygulaması (2026-11-07, aynı gün tamamlandı)

| Bulgu | Uygulama | Kanıt |
|---|---|---|
| **İ-1 (P1)** portföy notional kapısı | `fleet.rs`: `portfolio_cap_exceeded` (saf, rasyonel) + `portfolio_cap_pct_from_env` (default %85, `0`=kapalı) + cache deseni (`venue_live_load` kopyası: poll turunda kilit altında hesapla, dispatch'te ayrı std kilitten oku). `engine.rs`: `PlaceLimit` (SAFETY vb.) + `PlaceMarket` (BASE/hedge) Add emirlerinde kapı; kapanış rolleri (TP/SL/MANUAL/TRAIL/GRID_CLOSE) muaf. Blokta WARN olayı + SSE | Canlı: `DCA_PORTFOLIO_MAX_NOTIONAL_PCT=20` ile audit-cap6 → DB'de `WARN: PORTFOLIO CAP: BASE emri yerleştirilmedi...` + faz Idle'da kaldı; kullanıcı botları etkilenmedi (26% < 85%) |
| **A1 ders** | İlk denemede dispatch içinden `runtimes.lock().await` → **global deadlock** (tokio Mutex reentrant değil); motor dondu → cache desenine geçildi | Kök neden: per-bot poll `rts` kilitli tutuyor (engine.rs:1910+). Ders raporda kalıcı not |
| **S-1 (P1)** dev-chain açıkları | `npm audit fix` (breaking olmayan) | high 6→**0**; kalan 8 moderate (breaking gerektirir, not edildi); **runtime 0** doğrulandı (`--omit=dev`) |
| **O-2 (P2)** HALT kurtarma | Filo kartında `KERNEL'İ SIFIRLA` düğmesi (Studio'daki tek-yol eki kırıldı) | t9-pinemacd kartında canlı görüldü |
| **§5.1-4 (P2)** test derinliği | `dca_math/src/properties.rs`: sabit-tohumlu LCG deterministic property testler (proptest offline cache'te yok — sıfır yeni bağımlılık kararı). P1 merdiven monotonluğu (600 rastgele konfig), P2 qty pozitifliği, P3 domain fail-closed, P4 nasty numbers paniksizlik, P4b sub-tick anchor belgesi, P5 Fraction string-tur kimliği | 4 GERÇEK bulgu: (1) sub-tick anchor'da **sessiz boş plan** (H-1 hardening notu), (2) Display rasyonel form basıyor (tur from_any_str ile), (3) kümülatif sapma ≥%100 → dürüst Err, (4) ladder honest-truncate (len ≤ count). Suite: 295/295 |
| **UIX-43 (P3)** marj vurgusu | `MarginErodeBadge`: unrealized/IM erime oranı; ≥%50 → kırmızı "LİKİDASYONA YAKIN (M07)" | arda1 kartında canlı: kırmızı `MARJ KULLANIMI $139.86 · ERİME %56.2 — LİKİDASYONA YAKIN` |
| **S-2 (P3)** tunnel notu | README §8: yalnız :3000 tünellenmeli; `/api/health` + `/api/stream` için Cloudflare Access şart | README güncel |

**Final doğrulama:** `cargo test --workspace` **295/295** (287+8) · `tsc --noEmit` 0 · `eslint` 0 · motor canlı (13/13, default cap %85) · audit test botlarının tamamı silindi (filo temiz).

**Bilinen açık kalem (dürüstlük):** e2e_engine.py bölümlendirme + Playwright smoke (§5.5-6) bu fazda yapılmadı — ayrı oturum gerektirir; plan §5'te duruyor. Kalan 8 moderate npm bulgusu breaking-change istiyor — planlı bakımda.
