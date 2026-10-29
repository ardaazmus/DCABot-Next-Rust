# DENETİM DOĞRULAMA RAPORU — 2. TUR (Faz 21)

**Denetlenen ağaç:** `origin/master` @ **`a9e3d41`** (Faz 21-B)
**Kapsam:** `f2f5df2` _"Faz 21: dış doğrulama raporu düzeltmeleri — N-1..N-10 + P0-9/11 + P1-8/10 + P2-8/9/14 + UIX-5/14"_ + `a9e3d41` (CI ilk koşu düzeltmeleri)
**Referans:** `DENETIM_RAPORU_2026-10-28.md` (ilk denetim, 74 madde) ve `DENETIM_RAPORU_FAZ20_DOGRULAMA.md` (1. doğrulama + N-1…N-10)
**Tarih:** 2026-10-29

> **Kullanıcı haklı:** Master ilerlemiş. 1. doğrulama raporumdaki **N-1…N-10 bulgularının 9'u** ve
> **P0-9 / P0-11** kapatılmış. Aşağıda madde madde kanıtla.

---

## 0. Önceki raporla ne değişti (özet)

| Ölçüt | Faz 20 doğrulaması (`6743a3b`) | **Faz 21 doğrulaması (`a9e3d41`)** |
|---|---|---|
| DÜZELTİLDİ | 24 / 74 | **36 / 74** |
| KISMEN | 13 | **9** |
| AÇIK | 37 | **28** |
| Bilinçli kapatıldı | – | 1 (UIX-7) |
| Yeni bulgu (bu tur) | 10 (N-1…N-10) | **10'un 9'u kapandı; 3 yeni (N-11…N-13)** |
| **Kritik (P0) açık** | **2** (P0-9, P0-11) | **0** |

---

## 1. N-1 … N-10 — 1. doğrulama raporunun bulguları

| # | Bulgu | Hüküm | Kanıt (master `a9e3d41`) |
|---|---|---|---|
| **N-1** | Yeni snapshot alanları `#[serde(default)]` değil → eski snapshot çözülemiyor, bot sessiz sıfırlanıyor | **DÜZELTİLDİ** | `funding_deal` `#[serde(default)]` (`dca.rs:73`), `next_level_to_place` (`:78`), `closing_bars_waited` (`:83`). Ayrıca parse hatası artık **WARN değil CRITICAL** ve bot **`PAUSED`** açılıyor (`state.rs:895-905`: _"bot PAUSED acildi; taze baslatmak icin START…"_). İkinci yarı da kapanmış. |
| **N-2** | Hedge bacak emirleri ana bacağın `positionIdx`'ini alıyor (**para kaybı**) | **DÜZELTİLDİ** | `engine.rs:1073` `leg_direction(rt, id)` yardımcısı; iki kullanım: limit `engine.rs:1143`, market `engine.rs:1214` → `if leg_direction(rt,&id) == 1 {1} else {2}`. Yorum: _"hedge bacağı emirleri ana bacağın idx'ini ASLA almaz"_. |
| **N-3** | LIVE dolum **sweep**'i fail-open (`_ => {}` / `if let Ok(Some)`), kısmi kalan işlenmiyor | **DÜZELTİLDİ** | `engine.rs:1967-1985`: `Ok(None)` bilinçli idempotent, **`Err` → CRITICAL `FILL_INGEST` + RISK SSE**. Fallback koşulu artık "hiç fill yok" değil **"kalan miktar"**: `ingested = filled_qty_for_order(...)`, `remaining = cum_exec − ingested` (`engine.rs:1995-1999`). `terminal_settled` bayrağı ile ingest hatasında **takip AÇIK kalıyor** (`engine.rs:2072-2090`). |
| **N-4** | Kısmi kalan "fiyat 0" ile kitaba yazılıyor; 0 fiyatlı ingest m04'te patlayıp yutuluyor | **DÜZELTİLDİ (küçük kalıntı)** | `MarketOpen` artık `ref_px` taşıyor (`engine.rs:633-637`); `price.is_zero()` → CRITICAL olay (`engine.rs:809-820`); fallback'te `px <= 0` → **ingest ETME**, `terminal_settled = false`, emir kitapta (`engine.rs:2004-2020`). *Kalıntı:* 0 fiyatlı kayıt yine de `live_open_orders`'a giriyor → venue-load 0 sayıyor (küçük fail-open); N-14'e bakınız. |
| **N-5** | WFE cezası birim uyumsuz (oran ↔ getiri) | **DÜZELTİLDİ** | `certify.rs:338-343`: negatif-IS fold → `contrib = -1.0` (oranla aynı birim), fold paydada. |
| **N-6** | `CLOSE_ALL` `Closing` fazında yok sayılıyor ama Studio vaat ediyor | **DÜZELTİLDİ** | `dca.rs:808-831`: `Closing` dalında `manual_close_requested` tüketiliyor (TP iptal + MANUAL + `closing_bars_waited = 0` + log "N-6"). |
| **N-7** | TP (Reduce) %10 hacim payına takılıp kapanamıyor | **DÜZELTİLDİ** | `dca_risk/lib.rs:168-185`: `volume_share_exempt` → risk-azaltan emirler muaf (GATE7 ile aynı mantık). |
| **N-8** | Kernel `halted` arayüzde görünmüyor, RESET düğmesi yok | **DÜZELTİLDİ** | `bots.ts:124-126` (`kernelHalted`, `kernelHaltedReason`); `fleet.tsx:407-416` kırmızı **KERNEL HALT** rozeti; `studio.tsx:425-453` kırmızı şerit + **"KERNEL'İ SIFIRLA"** → `apiBotAction(id,"RESET_KERNEL")`. Ayrıca `RESET_KERNEL` artık **hedge kernel'i de** açıyor (`main.rs:1747-1752`). |
| **N-9** | Restore kapısı yalnız adet, yalnız DCA bacağı | **DÜZELTİLDİ** | `state.rs:920-955` `position_leg_mismatch(kernel, engine, tick, leg)` → **(adet, yön, ortalama ±1 tick)**; hedge bacağı ayrıca kontrol ediliyor (`state.rs:859-870`), sonuçlar birleştirilip `HALTED_RESTORE`. |
| **N-10** | Grid fee-breaker ortalamayı ölçüyor + maker×maker | **KISMEN** | **Runtime** düzeldi: `grid.rs:88-117` gerçek mesh üzerinden `validate_grid_fee_breaker(mesh, maker, **taker**)` — en dar segment + asimetrik ücret; Permissive modda `NEGATIVE_FEE_MARGIN` uyarısı. **Ama create-time 422 kapısı hâlâ eski ortalamayı kullanıyor** → **N-11**. |

---

## 2. P0 — kritikler: **açık kalem kalmadı**

| # | Hüküm | Not / kanıt |
|---|---|---|
| P0-1 Hedge bacağı eşlemesi | **DÜZELTİLDİ** | N-2 ile birlikte tamamlandı (bacak bazlı yön). |
| P0-2 Sticky halt | **DÜZELTİLDİ** | + N-8 (görünürlük/RESET) + hedge kernel reset. |
| P0-3 PAPER restart | **DÜZELTİLDİ** | + N-1 (serde) + N-9 (üçlü kapı). |
| P0-4 LIVE market dolumu | **DÜZELTİLDİ** | + N-3 (sweep fail-open kapandı). |
| P0-5 Kısmi dolum | **DÜZELTİLDİ** | `remaining` tabanlı fallback, `terminal_settled`. |
| P0-6 `Closing` çıkışı / `PlaceFailed` faz geri alma | **KISMEN** | `Closing` tarafı tamam (3-bar retry + N-6). **`PlaceFailed` hâlâ fazı geri almıyor**: `engine.rs:860-863` yalnız WARN yazıyor; `rollback_pending_phase_after_gate_reject` **yalnız `GateBlocked`** için çağrılıyor (`engine.rs:857`). Borsa emri reddederse (`PlaceFailed`) bot `BasePending`/`Closing`'de kalabilir. |
| P0-7 LIVE `ADD_FUNDS` | **DÜZELTİLDİ** | — |
| P0-8 Webhook auth | **DÜZELTİLDİ** | — |
| **P0-9 API anahtarı tarayıcıda** | **DÜZELTİLDİ** | `src/middleware.ts` `/api/engine/:path*` yakalayıp **sunucu tarafında Bearer** ekliyor; host/port sabit + beyaz liste (SSRF sert kenar); `engine-client.ts` artık `/api/engine/*` göreli çağrı; `NEXT_PUBLIC_ENGINE_API_KEY` istemci kodundan **silinmiş** (yalnız sunucu dosyalarında: `middleware.ts:27`, `stream/route.ts:38`). *Kalıntı riskler:* **N-12**, **N-13**. |
| P0-10 Sertifika kapısı | **KISMEN** | Kapandı: funding ortak taban, bileşik getiri, negatif-IS cezası (**N-5**). **Açık:** (a) DSR hâlâ `trials.len()` (9 sabit varyant) — kullanıcının gerçek deneme sayısını yansıtmıyor (`certify.rs:318-327`); (b) `cert_acknowledged` hâlâ **tek boolean** (`main.rs:1319`) ve UIX-10 tek-onay kutusu bunu **riskAck ile birlikte** işaretliyor (`wizard.tsx` risk-ack onCheckedChange). |
| **P0-11 `max_safety_orders` sınırsız** | **DÜZELTİLDİ** | `main.rs:1086-1100`: **422** — `max_safety_orders > 100`, `grid_levels ∉ [2,500]`. Savunma katmanı: `m02.rs:88-96` `safety_count > 1000` → `Err(DomainViolation)` (runtime yol da korunuyor). `grid.rs:76` ayrıca `.min(500)`. |

---

## 3. P1 — 24 maddenin güncel durumu

| Hüküm | Maddeler |
|---|---|
| **DÜZELTİLDİ (12)** | P1-1, P1-4, P1-5, **P1-8**, P1-9, **P1-10**, P1-12, P1-13, P1-14, P1-16, P1-17, P1-22 |
| **KISMEN (4)** | P1-2 (`profit_factor` hâlâ `"n/a"` — `backtest.rs:737`), P1-3 (hesap aynı; yalnız etiket dürüst), P1-15 (donmuş bar ✓; **spike / gelecek-ts / h==l yok**), P1-18 (fee-breaker ✓; **`n` seviye → `n+1` çizgi** hâlâ `m15_m20.rs:18,47`) |
| **AÇIK (8)** | P1-6, P1-7, P1-11, P1-19, P1-20, P1-21, P1-23, P1-24 |

**Faz 21'de yeni kapanan P1'ler:**
- **P1-8 (SO fonu)** — `kernel/lib.rs:153-184` `closer_pending_reserved()`: dolum fiyatına **daha yakın** bekleyen Add emirlerinin `notional + taker fee` toplamı rezerve edilir, GATE6 `available = cash − reserved` ile çalışır (`kernel/lib.rs:471-483`). Yani "uzak kademe yakın kademenin nakitini kilitleyemez" — 3Commas SO-fond mantığının özü. *Not:* piyasa Add yolunda rezerv yok (`kernel/lib.rs:399`); piyasa emri tanım gereği en yakın kademe olduğu için bu tutarlı sayılabilir, belgelenmeli.
- **P1-10** — pay artık `StepperConfig::max_volume_share` (tek kaynak, `kernel/lib.rs:327,370,439`); **piyasa Add'lerinde de** uygulanıyor (`kernel/lib.rs:369-384`, eskiden hiç yoktu); Reduce muaf (N-7).
- **P1-18 (kısmi)** — fee-breaker artık gerçek mesh + en dar segment + asimetrik ücret.

**Hâlâ açık olanların en önemlileri:**
- **P1-6** funding mutabakatı: `engine.rs:298-303` hâlâ sabit 8h + `crossings` çarpımı, **anlık** `ticker.funding_rate`, bar kapanış fiyatı; ticker hatası **sessiz** `Ok(())`. `funding_history()` hâlâ yalnız sertifika/backtest'te.
- **P1-11** M03 (`plan_notional`, `required_collateral`) hâlâ **çağıransız**; kurulumda stres-teminat kapısı yok.
- **P1-7** reduce overfill `violations` hâlâ kernel'de yüzeye çıkmıyor (CRITICAL/SUSPECT yok).
- **P1-19 / P1-21** merdiven doğrulaması hâlâ nominal çapa (`anchor=1000, tick=0.01`), QuoteNotional hâlâ kapanış fiyatından.
- **P1-23** Bybit `/v5/account/fee-rate` yok (ücretler gömülü). **P1-24** motor tarafında fee-guard yok.

---

## 4. P2 — 22 maddenin güncel durumu

| Hüküm | Maddeler |
|---|---|
| **DÜZELTİLDİ (4)** | P2-3, **P2-9** (TICK artık `events` tablosuna yazılmıyor — `engine.rs:1455+`), **P2-11** (`XTransformPort` çağrı yollarından kaldırıldı; kalan yalnız iki dosyada *yorum*), **P2-14** (`.github/workflows/ci.yml`: `cargo test --workspace --locked` + `npx tsc --noEmit` + `eslint`; clippy advisory) |
| **KISMEN (3)** | P2-1 (position/pending ✓; **`breaker_day`, `breaker_tripped`, `live_open_orders` hâlâ snapshot'ta yok** → restart'ta günlük DD devre kesici sıfırlanır = fail-open), P2-8 (`Lagged` artık istemciye bildiriliyor `main.rs:3740-3745`; **`Last-Event-ID` yok**, `PING` hâlâ `n % 15` olay sayacı `main.rs:3723`), P2-21 (ölü GATE8 dalı + `max_volume_share` + grid fee-breaker canlandı; `plan_notional`/`required_collateral`/`m20_validate_config`/`profit_factor` hâlâ ölü) |
| **AÇIK (15)** | P2-2, P2-4, P2-5, P2-6, P2-7, P2-10, P2-12, P2-13, P2-15, P2-16, P2-17, P2-18, P2-19, P2-20, P2-22 |

**P2-2 doğrulandı, hâlâ yanlış:** `state.rs:758` yorumu _"Secret DB'ye yazılmaz"_ diyor, ama `state.rs:586` `webhook_secret: Some(...)` ile yazıyor. Yorum tek satırda düzeltilebilir; yanlış yorum güvenlik incelemesini yanıltır.

**P2-18 kötüleşti:** iki yeni 33–50 KB rapor **depo kökünde** (`DENETIM_RAPORU_*.md`); `HATA_VE_EKSIK_ANALIZ_RAPORU` değişmeye devam ediyor. `docs/` altına taşınmalı.

---

## 5. UIX — 17 maddenin güncel durumu

| Hüküm | Maddeler |
|---|---|
| **DÜZELTİLDİ (11)** | UIX-1, 2, 3, **5**, 6, 8, 9, 10, 11, **12**, **14** |
| **Bilinçli kapatıldı (1)** | UIX-7 (`en-US` gerekçelendirildi) |
| **AÇIK (5)** | UIX-4 (birim modeli), UIX-13 (KillButton iki yerde: `page.tsx:146` + `dashboard.tsx:570`), UIX-15 (kontrast/ekran okuyucu turu), UIX-16 (`layout.tsx:33` `className="dark"` sabit), UIX-17 (TR/EN karışık) |

**Faz 21'de kapananlar:**
- **UIX-5** — `pending_engine_actions` artık **her poll'da, bar beklemeden ve `FROZEN/PAUSED/STOPPED` iken bile** akıtılıyor (`engine.rs:1749-1758`), gerekçesi yorumda: risk-azaltan eylemler strateji kilidine takılmaz. Eski "bir bar gecikme + donmuş botta hiç çalışmama" kusuru kapandı.
- **UIX-12** — `Closing` fazı artık gerçekten çalışan bir yola işaret ediyor (N-6 ile birlikte metin doğru hale geldi).
- **UIX-14** — sihirbazda `useEngineHealth` ile **motor çevrimdışıyken launch kapalı** + açıklama metni (`wizard.tsx:657-663`, `2538-2545`).

---

## 6. YENİ BULGULAR (bu tur)

### N-11 · Grid fee-breaker **iki farklı matematik** kullanıyor (create ↔ runtime) · ORTA
- **Create-time 422:** `main.rs:1149` → `dca_strategy::grid_fee_breaker_violation()` (`advisory.rs:26-41`) — hâlâ **ortalama** adım `(upper/lower−1)/levels` ve **maker×maker**.
- **Runtime (`GridEngine::new`):** `grid.rs:97` → `validate_grid_fee_breaker(mesh, maker, taker)` — **en dar segment** ve **asimetrik** ücret.

Sonuç: iki kapı **farklı hüküm** verebilir.
- Aritmetik mesh + geniş üst segment → ortalama geçer (create OK), en dar segment kalır (runtime **reddeder**) → kullanıcı botu kurar, **çalışma anında** kurulum hatası alır (UI'da "bot açıldı ama mesh yok").
- Ters yönde: create 422 alan bir grid, runtime'da geçerdi.
**Olması gereken:** tek kaynak — create-time kapısı da **mesh üretip** `validate_grid_fee_breaker` çağırsın (grid.rs'teki kod `dca_strategy`'de zaten var; `advisory::grid_fee_breaker_violation` bu fonksiyonu sarmalamalı).

### N-12 · `/api/engine` proxy'si **kimlik doğrulaması olmadan** tam yetkili · ORTA-YÜKSEK
`src/middleware.ts` her `/api/engine/*` isteğine sunucu tarafında `Authorization: Bearer <ENGINE_API_KEY>` ekliyor. Anahtar artık tarayıcıya sızmıyor (**P0-9 doğru çözüm**), ama proxy'nin kendisi **oturum/CSRF kontrolü yapmıyor**: Next uygulamasına erişebilen herkes (yerel ağda başka bir cihaz, tünellenmiş port, ileride bir hosting) motora **tam yönetim yetkisiyle** erişir — bot silme, kill-switch, vault listesi dâhil.
Ayrıca `SameSite`/`Origin` kontrolü yok → kullanıcı tarayıcısındaki zararlı bir sayfa `fetch("http://127.0.0.1:3000/api/engine/api/bots/...", {...})` ile yazma deneyebilir (basit istek kısıtı yalnız bazı content-type'larda korur; `text/plain` + JSON gövde + CORS preflight'sız yol denenebilir).
**Olması gereken:** (1) middleware'de `Origin`/`Sec-Fetch-Site` doğrulaması + yalnız `same-origin`; (2) yazma metotları için CSRF token veya `X-Requested-With` zorunluluğu; (3) varsayılan `HOST` bind'i `127.0.0.1` ile sınırlı tutulmalı ve README'de açıkça yazılmalı; (4) tercihen panel oturumu (cookie + HttpOnly) eklenmeli.

### N-13 · Middleware anahtarı hâlâ `NEXT_PUBLIC_ENGINE_API_KEY`'ten okuyor (gelecekteki sızıntı tuzağı) · DÜŞÜK-ORTA
`src/middleware.ts:27`: `return process.env.ENGINE_API_KEY || process.env.NEXT_PUBLIC_ENGINE_API_KEY || "";`
`NEXT_PUBLIC_` önekli bir değişken, **herhangi bir istemci dosyası** okuduğu anda paketine girer. Şu an istemcide okunmuyor (kontrol edildi: `src/**` içinde yalnız `middleware.ts` ve `stream/route.ts`). Ama isim durdukça **P0-9'un geri gelme riski** canlı kalır; üstelik değişkenin `.env`'de bu adla durması, "bu anahtar tarayıcıya gömülür" varsayımını normalleştiriyor.
**Olması gereken:** `NEXT_PUBLIC_` fallback'ini sil (yalnız `ENGINE_API_KEY`), `.env.example`/README'de adı `ENGINE_API_KEY` olarak düzelt, ve CI'da `grep -r "NEXT_PUBLIC_ENGINE_API_KEY" src` negatif testi.

*(N-14, düşük: `MarketOpen` 0-fiyat patolojisinde kayıt yine `live_open_orders`'a 0 fiyatla giriyor → `bump_venue_live_load` 0 ekliyor. Görünürlük var (CRITICAL), ama defter eksik sayıyor. Öneri: 0 fiyatlı kaydı kitaba koyma, `ref_px`'i son kapanıştan türet.)*

---

## 7. Kalan iş listesi (öncelik)

**Yüksek:**
1. **P1-6** funding: `funding_history` + `fundingIntervalHour`; fetch hatası = görünür olay (sessiz `Ok(())` yok).
2. **N-12** proxy'ye origin/CSRF kapısı.
3. **P0-6 kalanı** `PlaceFailed` → faz geri alma (GateBlocked ile aynı `rollback_pending_phase_after_gate_reject`).
4. **P1-11** kurulumda stres-teminat kapısı (M03'ü canlandır; `QuoteNotional` dalındaki `n.mul(1)` hatasını da düzelt).
5. **P2-1 kalanı** `breaker_day/tripped`, `live_open_orders` snapshot'a → restart'ta günlük DD kesici kaybolmasın.
6. **P0-10 kalanı** DSR `trials` sayısı (gerçek deneme sayısı) + sertifika istisnasını ayrı/süreli onaya bağla.

**Orta:**
7. **N-11** grid fee-breaker tek kaynak.
8. **P1-7** reduce overfill → CRITICAL + `SUSPECT`.
9. **P1-15** GATE1: spike (`|Δclose|/close > eşik`), `t > now + interval`, `h == l`.
10. **P1-19/P1-21** gerçek enstrüman metadata'sı + açılış fiyatı beklentisi.
11. **N-13** `NEXT_PUBLIC_` fallback temizliği + CI grep kapısı.
12. **P1-18 kalanı** `n` seviye → `n+1` çizgi belirsizliği (UI'da "n+1 çizgi" yaz veya mesh'i n çizgiye indir).
13. **P1-23/P1-24** gerçek ücretler + motor tarafı fee-guard.

**Düşük:** P2-4 (Argon2id), P2-5 (izin yeniden taraması), P2-6/7 (DB/perf), P2-8 `Last-Event-ID` + zaman tabanlı PING, P2-10 (sayfalama/idempotency/rate-limit), P2-12/13/15/16/17, P2-18 (raporları `docs/`'a taşı — bu turda **kötüleşti**), P2-19/20/22, UIX-4/13/15/16/17.

---

## 8. Bu turda **doğrulanamayanlar** (değişmedi)

| İddia | Durum |
|---|---|
| `cargo test --workspace`, `tsc --noEmit`, `eslint` gerçekten geçiyor mu | **Doğrulanamadı** — bu sandbox'ta `cargo`/`node_modules` yok; GitHub Actions çalıştırma kayıtlarına API iznim yetmiyor (`403 Resource not accessible`). `.github/workflows/ci.yml` **var ve gerçek kapılar içeriyor** — yani iddia artık *kanıtlanabilir*, benim ortamımda değil. |
| Faz 21'in "testler güncellendi" beyanı | **Doğrulanamadı** (aynı neden). CI yeşilse kapanır. |
| N-2 düzeltmesinin venue tarafındaki karşılığı | Kod doğru (bacak yönü); Bybit hedge-mode semantiği **varsayım**. |

**Not:** Bu, 1. turdaki "hiçbir şey derlenmiyor" durumundan **yapısal olarak daha iyi**: CI eklendiği için bundan sonraki düzeltmeler beyan değil, koşan kapı olabilir. Tek eksik: CI'ın **gerçekten koştuğunu** dışarıdan görebilmek (rozet/`gh run list`).

---

## 9. Sonuç

- 1. turda "para kaybı" sınıfında işaretlediğim **N-2 (hedge positionIdx)** ve **N-1 (snapshot uyumsuzluğu)** kapandı; **P0-9** ve **P0-11** gerçekten çözüldü (proxy + sunucu tarafı anahtar; 422 + savunma katmanı).
- 1. turdaki 10 yeni bulgunun **9'u tam, 1'i kısmen** kapandı.
- **Kritik (P0) açık kalem: 0.** Bununla birlikte **P0-10 kısmen** ve **P1-6 (funding) / P1-11 (stres teminatı)** hâlâ finansal doğruluk açısından en zayıf halkalar.
- Bu tur **P0-9 düzeltmesinin kendi yan etkisini** ortaya çıkardı: **N-12** (kimlik doğrulamasız tam yetkili proxy) — çözüm doğru yönde ama yarım; yerel kullanımda risk düşük, ağa açıldığında yüksek.

---

### Ek — tekrar üretilebilir doğrulama komutları

```bash
git worktree add --detach /tmp/m21 origin/master          # a9e3d41
cd /tmp/m21
grep -n "leg_direction" -A 20 mini-services/dca-core/crates/dca_server/src/engine.rs     # N-2
grep -n "serde(default)" -B1 mini-services/dca-core/crates/dca_strategy/src/dca.rs       # N-1
grep -n "FILL_INGEST" mini-services/dca-core/crates/dca_server/src/engine.rs             # N-3/N-4
grep -n "position_leg_mismatch" -A 30 mini-services/dca-core/crates/dca_server/src/state.rs  # N-9
grep -n "closer_pending_reserved" -A 25 mini-services/dca-core/crates/dca_kernel/src/lib.rs  # P1-8
sed -n 1,60p src/middleware.ts                                                            # P0-9 / N-12 / N-13
grep -n "max_safety_orders > 100" -A 6 mini-services/dca-core/crates/dca_server/src/main.rs   # P0-11
cat .github/workflows/ci.yml                                                              # P2-14
grep -n "grid_fee_breaker_violation" main.rs; grep -n "validate_grid_fee_breaker" grid.rs  # N-11
```
