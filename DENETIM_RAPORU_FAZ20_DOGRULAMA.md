# DENETİM DOĞRULAMA RAPORU — "yapıldı" denilen düzeltmeler gerçekten yapıldı mı?

**Denetlenen commit:** `origin/master` @ `6743a3b` — _"Faz 20: UX cilaları — UIX-5/6/7/8/9/10/12"_
**Referans (ilk denetimin yapıldığı ağaç):** `3ac500f`
**Fark:** 19 dosya, **+1295 / −228** satır
**Tarih:** 2026-10-29
**İlk rapor:** `DENETIM_RAPORU_2026-10-28.md` (P0-1…11, P1-1…24, P2-1…22, UIX-1…17)

---

## 0. Kapsam, yöntem ve dürüst limitler

Bu rapor **bir düzeltme doğrulamasıdır**: ilk raporda işaretlenen 74 maddenin her biri, `6743a3b`
ağacındaki **güncel koda** karşı yeniden okundu ve şu dört kategoriye ayrıldı:

| Etiket | Anlamı |
|---|---|
| **DÜZELTİLDİ** | Kök neden gerçekten kapanmış; eski davranışa dönüş yolu yok. |
| **KISMEN** | Ana senaryo kapanmış, ama aynı bulgunun ikinci yolu/kenar durumu açık. |
| **AÇIK** | Kodda değişiklik yok veya değişiklik bulgunun özüne dokunmamış. |
| **YENİ (N-x)** | Düzeltmenin kendisinden doğan yeni hata / regresyon riski. |

**Yöntem:** `git diff 3ac500f origin/master` üzerinden dosya dosya diff okuması **+** iddiaların
bağımsız `grep`/`sed` ile güncel ağaçta doğrulanması. Düzeltme iddiaları (commit mesajı, `worklog`,
`HATA_VE_EKSIK_ANALIZ_RAPORU` içindeki "PASS" ifadeleri) **kanıt olarak kabul edilmedi**; yalnız kod
okundu.

**Doğrulanamayanlar (dürüstçe):** Bu ortamda **`cargo`, `rustc` yok; `node_modules` yok.** Yani
hiçbir şey **derlenmedi, test edilmedi, çalıştırılmadı**. Rapordaki `HATA_VE_EKSIK_ANALIZ_RAPORU`
notundaki "workspace 276/276 PASS" ifadesi **bağımsız olarak teyit edilemedi**. Aşağıdaki tüm
hükümler **statik kod okumasına** dayanır. Ayrıca hiçbir uygulama kodu değiştirilmedi (depo kuralı).

---

## 1. Skor tablosu

| Grup | Toplam | DÜZELTİLDİ | KISMEN | AÇIK |
|---|---|---|---|---|
| **P0 (kritik)** | 11 | 4 | 5 | 2 |
| **P1 (yüksek)** | 24 | 10 | 4 | 10 |
| **P2 (orta)** | 22 | 1 | 2 | 19 |
| **UIX (arayüz/UX)** | 17 | 9 (1'i bilinçli kararla kapalı) | 2 | 6 |
| **TOPLAM** | **74** | **24** | **13** | **37** |

**Yorum:** P0/P1 tarafında ciddi ve gerçek bir ilerleme var — özellikle **P0-4/P0-5 (LIVE dolum
yakalama)**, **P1-4 (resting TP)**, **P1-16 (çift emir)**, **P1-1/P1-2 (net muhasebe)** ve
**P0-2/P0-3 (halt + restore)** gerçekten kapatılmış. Ancak:

- **P0-9 (anahtarın tarayıcı paketine gömülmesi)** ve **P0-11 (`max_safety_orders` sınırsız)** —
  ilk raporun "tek istekle tüm motoru düşürebilirsin" dediği iki kalem — **hiç dokunulmadan açık**.
- Düzeltmelerin çoğu **"mutlu yol"u** kapatıyor; **hata yolunda (Err) sessiz yutma** deseni
  (fail-open) LIVE dolum sweep'inde **hâlâ duruyor** (N-3).
- Bu commit **kendi içinde 10 yeni bulgu** üretti (N-1…N-10, §6). Bunlardan **N-1** (snapshot
  uyumluluğu) ve **N-2** (hedge bacak `positionIdx`) para kaybı sınıfındadır.

---

## 2. P0 — Kritik kalemlerin tek tek hükmü

| # | Bulgu (kısa) | Hüküm | Kanıt / gerekçe |
|---|---|---|---|
| P0-1 | Hedge modda kapanış emri bacak eşlemesi | **KISMEN** | Ana bacak düzeldi: `engine.rs:1105`, `engine.rs:1177` → `position_idx = if rt.stepper.position.direction == 1 {1} else {2}` (emir yönüne göre değil, **pozisyon** yönüne göre). **Ama hedge bacağı da aynı kodu kullanıyor** ve hedge bacağının yönü ana bacağın tersi: hedge emirleri de **ana bacağın idx'ini** alıyor → N-2. |
| P0-2 | Kernel `halted` kalıcı, kill/freeze sonrası bot bir daha çalışmıyor | **DÜZELTİLDİ** | Sticky Halt / resync ayrımı: `dca_kernel/src/lib.rs:232-256` (`Halt` → kalıcı; `"interval gap"` → `INTERVAL_RESYNC` advisory + devam). `ts_ms <= 0` → `Halt` (`lib.rs:215-220`). Açık kurtarma ucu: `RESET_KERNEL` (`main.rs:1723-1729`). **Not:** RESET yalnız ana kernel'i açar (hedge kernel'in `halted`'ı durur) ve **UI'da hiçbir düğmesi yok** → N-8. |
| P0-3 | PAPER restart'ta pozisyon kaybı | **DÜZELTİLDİ** | `RuntimeSnapshot`'a `position`, `kernel_prev_bar_ts`, `pending`, `realized_fees` eklendi (`state.rs:536-566`, hepsi `#[serde(default)]`). Restore + tutarlılık kapısı: `state.rs:830-848` (kernel qty ≠ mirror qty → `halted = HALTED_RESTORE` + CRITICAL `RESTORE` olayı). **Not:** kapı yalnız **adet** karşılaştırıyor ve yalnız `EngineTag::Dca` için (N-9); ayrıca eski snapshot'lar artık **çözülemiyor** (N-1). |
| P0-4 | LIVE market dolumları sessizce kayboluyor | **KISMEN** | Yeni yol doğru: `adapter.executions(...)` + `execId` anahtarı, her execution ayrı `MarketFilled` (`engine.rs:592-616`), hata → CRITICAL `FILL_INGEST` + `RISK` SSE (`engine.rs:790-800`), `MarketNoTrace` varyantı silinmiş. **Ama sweep tarafı aynı deseni koruyor:** `engine.rs:1939` `_ => {}`, `engine.rs:1962` `if let Ok(Some(f))` → dolum işlenemezse **sessiz**. Ayrıca `engine.rs:1949-1951` "bu emir için *herhangi* bir fill var mı" kontrolü, kısmi dolumdan sonra **kalanın hiç işlenmemesine** yol açabilir (N-3). |
| P0-5 | Kısmi dolum takip edilmiyor | **KISMEN** | Kısmi dolum taneli muhasebe + `MarketOpen` ile kalan kitapta; kitap terminal duruma kadar takip ediliyor (`engine.rs:620-631`, `1942-2000`). **Ama** kalan miktar kitaba **fiyat 0** ile yazılıyor (`engine.rs:625-630` → `LiveTrackedOrder{price: 0}`), böylece venue yük defteri eksik hesaplanıyor ve sweep'in fallback dolumu **0 fiyatla** deneyebiliyor (m04 reddeder → sessiz kayıp) → N-4. |
| P0-6 | `Closing` fazının çıkışı yok / `PlaceFailed` fazı geri almıyor | **KISMEN** | İyi: 3 bar sonrası MANUAL yeniden gönderim (`dca.rs:801-826`, `closing_bars_waited`). **Ama** `PlaceFailed` hâlâ fazı geri almıyor — yalnız `GateBlocked` için `rollback_pending_phase_after_gate_reject` çağrılıyor (`engine.rs:840` yalnız `GateBlocked` için çağrılıyor; `PlaceFailed` dalı (`engine.rs:842-845`) geri alma **yapmıyor**). Ayrıca `Closing` içinde `manual_close_requested` **okunmuyor** (yalnız `Open` dalında: `dca.rs:783-796`) → N-6. |
| P0-7 | LIVE `ADD_FUNDS` şalterleri atlıyor + sabit `Buy` | **DÜZELTİLDİ** | `main.rs:1585-1636`: kill/freeze denetimi (422), yön `rt.stepper.position.direction`'dan, `position_idx` eşlemesi, `MANUAL_ADD` fill'i üzerinden TP yeniden hesaplanıyor (`dca.rs:612-627`). |
| P0-8 | Webhook kimlik doğrulama duvarının arkasında | **DÜZELTİLDİ** | Ayrı router: `main.rs:145` (`webhook_routes`) + `main.rs:210` (`auth_middleware` yalnız ana router'a) + `main.rs:216` (`.merge(webhook_routes)`). |
| P0-9 | `NEXT_PUBLIC_ENGINE_API_KEY` tarayıcı paketinde | **AÇIK** | Değişmedi: `src/hooks/engine-client.ts:35` `process.env.NEXT_PUBLIC_ENGINE_API_KEY`, `src/app/api/stream/route.ts:38`. Proxy route (`src/app/api/engine/[...path]`) **hâlâ yok** — `src/app/api` altında yalnız `stream` var. |
| P0-10 | LIVE sertifika kapısı iyimser + tek tıkla aşılabiliyor | **KISMEN** | Kapandı: (a) funding ortak taban (`certify.rs:173-186`, `main.rs:1282-1290`), (b) bileşik getiri `∏(1+r)−1` (`certify.rs:209-212`, `311-315`), (c) negatif IS fold cezası + tüm fold'lar paydada (`certify.rs:329-341`). **Açık:** (d) DSR hâlâ `trials.len()` = 9 varyantı kullanıyor (kullanıcının gerçek deneme sayısı değil), (e) `cert_acknowledged` tek boolean ile kapı hâlâ aşılıyor (`main.rs:1303-1319`) — **üstelik UIX-10 düzeltmesi bunu kötüleştirdi**: tek onay kutusu `riskAck`+`certAck`'i birlikte işaretliyor (`wizard.tsx:2403-2408`). Ayrıca ceza **birim uyumsuz** (N-5). |
| P0-11 | `max_safety_orders` üst sınırı yok → tek istekle tüm motor çöker | **AÇIK** | `dca_strategy/src/dca.rs:377` ve `:539` hâlâ `self.cfg.max_safety_orders`'ı **sınırsız** kullanıyor. `main.rs:1113`'teki `.min(1000)` yalnız **doğrulama** çağrısında; `risk_policy.validate` (`dca_kernel/src/risk_policy.rs:127-136`) yalnız `active ≤ max` çapraz kontrolü yapıyor. `grid_levels` için de üst sınır yok (`grid.rs:74` yalnız `.max(2)`). |

---

## 3. P1 — Yüksek öncelikli (24 madde)

| # | Bulgu (kısa) | Hüküm | Kanıt |
|---|---|---|---|
| P1-1 | `realized_net` yanlış tanımlı | **DÜZELTİLDİ** | `realized_fees` defteri (`kernel lib.rs:751-760`, `net_realized`), snapshot'ta kalıcı (`state.rs:536-539`), `apply_funding_expense` içinde tazeleniyor. |
| P1-2 | Win/loss brütten | **KISMEN** | Net karar: `dca.rs:912-921` + log satırı gross/fees/funding/net (`dca.rs:926-935`). **Ama `profit_factor` hâlâ `"n/a"`** (`backtest.rs:737`) — arayüzde metrik olarak duruyorsa boş söz. |
| P1-3 | `display_sharpe` "Sharpe" değil | **KISMEN** | Hesap **aynı** (`backtest.rs:857-884`, hâlâ per-bar × 100). Yalnız **etiket** dürüstleşti: `backtest-panel.tsx:83-87`, `backtest-lab.tsx:1001` "Per-bar SR (×100)". Yıllıklandırma veya DSR ile aynı tanım **yok**. |
| P1-4 | Net TP maker ücretle hesaplanıp taker icra ediliyordu | **DÜZELTİLDİ** | TP artık **resting reduce-only LIMIT**: `dca.rs:743-777` (`RestingTp`, iptal/yeniden yazım WAC değişince), `engine.rs:1051-1055` (`role == "TP"` → `is_close`, reduce_only, GATE6 muafiyeti), kernel `OrderKind::Reduce` (`kernel lib.rs:301-306` GATE7 muafiyeti). Idempotent: `needs_replace` guard'ı sayesinde her barda **çift emir yazılmıyor** (`dca.rs:756-775`). |
| P1-5 | Kümülatif funding TP'yi bozuyor | **DÜZELTİLDİ** | `funding_deal` deal açılışında sıfırlanıyor (`dca.rs:478-489`), TP onu kullanıyor (`dca.rs:216-222`). |
| P1-6 | Funding mutabakatı yaklaşık/geçmişe dönük yanlış | **AÇIK** | `engine.rs:290-345` değişmedi: hâlâ **anlık** `ticker.funding_rate`, bar kapanış fiyatı, sabit `8h`, `crossings` çarpımı. `funding_history()` yalnız sertifikada kullanılıyor. Ek: ticker hatası → **sessizce** `Ok(())` (`engine.rs:309-311`) — funding hiç uygulanmıyor, olay yazılmıyor. |
| P1-7 | Reduce overfill sessiz kırpılıyor | **AÇIK** | Kernel/engine'de `violations` yüzeye çıkarılmıyor (grep: `violations` kernel'de yok); `m04::apply_fill` `Result` dönse de CRITICAL olay/`SUSPECT` durumu üretilmiyor. |
| P1-8 | GATE6 rezervasyon yok (merdiven yarım kalıyor) | **KISMEN** | Doğru yenilik: GATE6 **dolum anında, sıralı** uygulanıyor (`kernel lib.rs:405-430` limitler, `:337-362` piyasa Add'leri, gap batch dâhil). **Ama "rezerve" defteri yok**: `reserved = Σ bekleyen Add maliyeti` hesabı yok (grep: `reserved` yalnız bir yorumda). Yani aynı nakit üç bekleyen emre "sahip" olabilir; hangi emir önce dolarsa o kazanır — sonuncusu `Wait` modunda sonsuza kadar bekler. 3Commas'ın "SO fond" mantığı hâlâ yok. |
| P1-9 | Kapılarda fail-open `unwrap_or_else(zero)` | **DÜZELTİLDİ** | GATE6 aritmetik hatası → `Halt` (`dca_risk/src/lib.rs:190-205`, iki dal). **Not:** GATE5'te `unwrap_or_else(|_| zero)` (hacim 0 → hep Defer) ve engine'de 14 adet kozmetik `unwrap_or(0)` duruyor — bunlar pozisyon/muhasebe değil, "venue yük" defteri hatası. |
| P1-10 | `max_volume_share` ölü; pay %10 sabit | **AÇIK** | `StepperConfig.max_volume_share` (`kernel lib.rs:96,110`) hâlâ **okunmuyor**; sabit iki yerde: `kernel lib.rs:279` (`volume_cap`) ve `dca_risk lib.rs:161-165`. Market emirlerde hacim kapısı yok. **Ek olarak:** bu kap Reduce/TP emirlerine de uygulanıyor → N-7. |
| P1-11 | M03 (plan notional / stres teminatı) ölü kod | **AÇIK** | `plan_notional` ve `required_collateral` için **tek bir çağıran yok** (grep: yalnız `dca_math/src/m03_m07.rs` tanımı). Kurulumda "şok altında teminat yeter mi" kapısı yok. |
| P1-12 | GATE2 negatif/absurd ücret kabul ediyor | **DÜZELTİLDİ** | `dca_risk/src/lib.rs:76-82`: negatif → Reject, `> 1%` → Reject. |
| P1-13 | Geçersiz ts → `Utc::now()` (determinizm kırılıyor) | **DÜZELTİLDİ** | Girişte kapatıldı: `kernel lib.rs:215-220` (`ts_ms <= 0` → Halt), `dca_risk lib.rs:41-43` (`t <= 0` → Halt). Alttaki `Utc::now()` fallback'leri (`dca_risk:230,272`, `kernel:311`) artık **ulaşılamaz**. |
| P1-14 | GATE8 gap koruması ölü + `consolidated_qty` yanlış | **DÜZELTİLDİ** | `gate8_gap_protection(is_safety, ts)` — ölü `ConsolidateGap` dalı ve uydurma `consolidated_qty` silindi (`dca_risk lib.rs:271`); gerçek birleştirme kernel Gate 8b'de (`kernel lib.rs:438-465`). |
| P1-15 | GATE1 OHLC bütünlüğü eksik | **KISMEN** | Kapandı: sıfır hacim → `Defer` + `FROZEN_BAR_SKIP` (`dca_risk:53-58`). **Açık:** spike (`|Δclose|/close`), `h == l` donmuş bar ve "t gelecekte" kontrolü yok. |
| P1-16 | SAFETY top-up aynı fiyata ikinci emir | **DÜZELTİLDİ** | `next_level_to_place` yerleştirme imleci (`dca.rs:72-74`), `filled_safety` yalnız sayaç; test L4 fiyatını doğruluyor (`dca.rs` testleri). |
| P1-17 | `liquidation_price` ıraksarsa `Some(0)` | **DÜZELTİLDİ** | `dca_math/src/m03_m07.rs` (+4/−2): yakınsamama → `Err(DomainViolation)`. |
| P1-18 | Grid: n seviye → n+1 çizgi; fee-breaker ortalamayı ölçüyor | **AÇIK** | `grid.rs:74-103` değişmedi (`n+1` çizgi; `(upper/lower−1)/levels` ortalaması). `validate_grid_fee_breaker` (`m15_m20.rs:102-118`) **hâlâ yalnız testlerden** çağrılıyor; üretimde `advisory.rs:26-41` ortalamayı kullanıyor. |
| P1-19 | Merdiven kurulumu nominal çapayla doğrulanıyor | **AÇIK** | `main.rs:1097-1123`: hâlâ `anchor=1000`, `tick=0.01`, `step=0.001`. SHIBUSDT gibi enstrümanlarda gerçek tick ile tamamen farklı sonuç. |
| P1-20 | Tick yuvarlaması seviyeleri birleştirince plan reddediliyor | **AÇIK** | `dca_math/src/m02.rs` bu commit'te **değişmedi** (diff listesinde yok). |
| P1-21 | QuoteNotional base qty kapanıştan hesaplanıyor | **AÇIK** | `dca.rs:326-327,400`: `anchor = ctx.close`, `qty = base/anchor` — emir sonraki bar açılışında doluyor. |
| P1-22 | Sertifika metrikleri ile Lab rozetleri farklı koşu | **DÜZELTİLDİ** | `certify(..., funding)` + `run_trial` aynı funding (`certify.rs:173-186`); LIVE kapısı `funding_ok` ile "funding alınamadıysa KALDI" diyor (`main.rs:1277`, `certify.rs:495-500`). |
| P1-23 | Bybit ücretleri koda gömülü | **AÇIK** | `dca_exchange/src/bybit.rs` bu commit'te değişmedi; `/v5/account/fee-rate` çağrısı yok (grep: 0 sonuç). |
| P1-24 | Fee guard yalnız sihirbazda | **AÇIK** | `main.rs` create akışında `target_profit_pct` ↔ komisyon karşılaştırması yok; yalnız parse/aralık kontrolü (`main.rs:1070-1078`). |

---

## 4. P2 — Orta öncelikli (22 madde, özet)

| # | Bulgu | Hüküm | Not |
|---|---|---|---|
| P2-1 | Snapshot tutarsızlıkları | **KISMEN** | `position`, `kernel_prev_bar_ts`, `pending` eklendi. `filled_qty_this_bar`, `breaker_day`, `breaker_day_start_eq`, `breaker_tripped`, `regime_closes`, `live_open_orders` **hâlâ snapshot'ta yok** → restart sonrası günlük DD devre kesici sıfırlanır (riski düşürür, yani fail-open). |
| P2-2 | `webhook_secret` yorumu kodla çelişiyor | **AÇIK** | Yorum hâlâ "Secret DB'ye yazılmaz" diyor (`state.rs:754`), oysa `save_runtime_state` yazıyor (`state.rs:582`). |
| P2-3 | Auth sabit-zamanlı değil | **DÜZELTİLDİ** | `main.rs:717-724`: uzunluk + XOR fold (sabit zamanlı yaklaşım). |
| P2-4 | Vault anahtar türetimi zayıf (SHA-256, tuz yok) | **AÇIK** | `store.rs` değişmedi. |
| P2-5 | Anahtar izinleri yalnız ekleme anında | **AÇIK** | Periyodik yeniden tarama yok. |
| P2-6 | `create_bot` içinde ağır senkron iş | **AÇIK** | `spawn_blocking` var, progress/iptal yok. |
| P2-7 | DB tek `Mutex<Connection>` | **AÇIK** | Değişmedi. |
| P2-8 | SSE yeniden başlatılabilir değil | **AÇIK** | `main.rs:3716` hâlâ `Lagged(_) => continue`; `Last-Event-ID` yok (grep: 0); `PING` hâlâ `n % 15` (olay sayacı) (`main.rs:3699`). |
| P2-9 | Olay tablosu şişmesi (her turda TICK) | **AÇIK** | `engine.rs:1421-1440` aynı. |
| P2-10 | API eksikleri (sayfalama, idempotency, rate-limit) | **AÇIK** | Değişmedi. |
| P2-11 | `XTransformPort` her isteğe ekleniyor | **AÇIK** | `engine-client.ts:25`. |
| P2-12 | `.env` ayrıştırıcısı naif | **AÇIK** | Değişmedi. |
| P2-13 | Gömülü yollar (`/home/z/my-project/...`) | **AÇIK** | Değişmedi. |
| P2-14 | CI yok | **AÇIK** | `.github/` hâlâ yok. |
| P2-15 | Rust araç zinciri sabitlenmemiş | **AÇIK** | `rust-toolchain.toml` yok. |
| P2-16 | Başlatıcılar yalnız Windows/PowerShell | **AÇIK** | Değişmedi. |
| P2-17 | Dokümantasyon kodu geçmiş | **AÇIK** | README/Caddyfile/next.config üçlüsü aynı. |
| P2-18 | Rapor/ledger dosyaları kök dizinde | **KÖTÜLEŞTİ** | `HATA_VE_EKSIK_ANALIZ_RAPORU_2026-10-25.md`'ye **+30 satır daha** eklendi (Faz 17 tablosu) — kök dizin kirliliği arttı. |
| P2-19 | `.gitignore` artıkları | **AÇIK** | Değişmedi. |
| P2-20 | E2E mimari kırılgan | **AÇIK** | Fake venue fixture'ı yok. |
| P2-21 | Ölü/tekrar kod | **KISMEN** | Ölü GATE8 dalı silindi (P1-14). Hâlâ ölü: `plan_notional`, `required_collateral`, `m20_validate_config` (`m15_m20.rs:183`), `max_volume_share`, `profit_factor` (`backtest.rs:737`), `validate_grid_fee_breaker`. |
| P2-22 | Hata mesajlarında sır sızma riski | **AÇIK** | Değişmedi. |

---

## 5. UIX — Arayüz (17 madde)

| # | Bulgu | Hüküm | Kanıt |
|---|---|---|---|
| UIX-1 | SSE istemcisi tüm olayları dinlemiyor | **DÜZELTİLDİ** | `engine-stream.ts:156-170`: `FUNDING`, `LIQUIDATION`, `WEBHOOK_START` ring buffer'a giriyor. |
| UIX-2 | `dataVer` sinyali aynı listeye bağlı | **DÜZELTİLDİ** | `engine-stream.ts:102-112`: `DATA_EVENT_TYPES` artık üç olayı da içeriyor. |
| UIX-3 | Pine TP/SL % → fraksiyon (100×) | **DÜZELTİLDİ** | `wizard.tsx` diff: `÷100` dönüşümü + "örn. 2 = %2" ipuçları. |
| UIX-4 | Birim modeli tutarsız (fraksiyon ↔ yüzde) | **AÇIK** | Alan adları/tip sözleşmesi değişmedi. |
| UIX-5 | Grid'de `CLOSE_ALL` 422 | **KISMEN** | Artık destekli: `main.rs:1665-1700` (`halt_mesh()` + market kapanış niyeti `pending_engine_actions`). **Ama** bu eylemler **yalnız `process_bar` içinde** boşaltılıyor (`engine.rs:220-223`) → bot `FROZEN/PAUSED/STOPPED` ise **hiç çalışmaz**, ayrıca en az **bir bar** (4h'lık botta 4 saat) gecikme var. |
| UIX-6 | `usePoll` sessiz atlama | **DÜZELTİLDİ** | `engine-stream.ts:20-63` (`lastUpdatedAt`) + `page.tsx:67-93` (`DataFreshness` rozeti: <15s yeşil, <45s amber, ≥45s kırmızı). |
| UIX-7 | Sayı biçimlendirme `en-US` | **KAPATILDI (bilinçli karar)** | `format.ts`'e gerekçe yorumu eklendi. |
| UIX-8 | Odak halkası yok | **DÜZELTİLDİ** | `wizard.tsx` Pine alanlarına `focus-visible:ring`. |
| UIX-9 | `microOf` hassasiyet kaybı | **DÜZELTİLDİ** | `bot-defaults.ts:44-56`: f64 dönüşümü kaldırıldı, string doğrudan taşınıyor (`Number.isFinite` doğrulaması ile). |
| UIX-10 | Çift onay kutusu | **DÜZELTİLDİ** | `wizard.tsx:2397-2415` tek kutulu "Risk Özeti — TEK ONAY" (numaralı liste). **Yan etkisi P0-10(e): sertifika kapısı da tek tikle aşılıyor.** |
| UIX-11 | Lab → Sihirbaz config taşınmıyor | **DÜZELTİLDİ** | `src/lib/carry-config.ts` (sessionStorage, 10 dk tazelik) + `backtest-lab.tsx:657-681` "SİHİRBAZA TAŞI" + wizard tüketimi. |
| UIX-12 | `Closing`'de eylem önerilmiyor | **KISMEN** | `studio.tsx:488-500` uyarı kutusu eklendi. **Ama metin yanıltıcı:** "Filo ekranındaki Tümünü Kapat ile manuel kapanış tetikleyebilirsiniz" — DCA'da `CLOSE_ALL` yalnız `Open` fazında okunan bir bayrak set ediyor (`dca.rs:783-796`), `Closing` fazında **yok sayılıyor** → N-6. |
| UIX-13 | `KillButton` header + dashboard'da tekrar | **AÇIK** | `page.tsx:146` **ve** `dashboard.tsx:570` ikisi de duruyor. |
| UIX-14 | Offline'da yazma eylemleri etkin | **AÇIK** | Wizard'da motor `offline` iken submit kilidi yok (grep: `offline` wizard'da yok). |
| UIX-15 | Kontrast/ekran okuyucu turu yapılmadı | **AÇIK** | Ölçüm yok. |
| UIX-16 | Tüm arayüz `dark` sabit | **AÇIK** | `layout.tsx:33` `<html className="dark">`. |
| UIX-17 | TR/EN karışık metin | **AÇIK** | Sözlük disiplini yok. |

---

## 6. YENİ BULGULAR — bu commit'in kendi ürettiği hatalar

### N-1 · Snapshot geriye dönük uyumsuz → güncellemede PAPER bot durumu **sessizce sıfırlanıyor** · YÜKSEK
`DcaEngine`'e eklenen üç alan `#[serde(default)]` **almamış**:
`dca_strategy/src/dca.rs:71` (`funding_deal`), `:74` (`next_level_to_place`), `:77` (`closing_bars_waited`)
— oysa aynı dosyada `resting_tp` (`:63`) ve `deal_fees` (`:88`) bilinçli olarak `#[serde(default)]` işaretli.

`RuntimeSnapshot.engine` **tüm `DcaEngine`'i** serileştiriyor (`state.rs:571-599` → `engine: rt.engine_kind.clone()`).
Dolayısıyla **eski sürümün yazdığı her snapshot** artık `serde_json::from_str::<RuntimeSnapshot>` içinde
**fail** ediyor → `state.rs:848-851` yalnız bir `WARN` yazıp botu **taze durumla** başlatıyor: faz `Idle`,
cash sıfır, merdiven yok, `deals_closed`/`wins`/`losses` yok.

**Etki:** PAPER botta açık deal + cash + istatistik sessizce kaybolur (LIVE'da kısmen venue reconcile kurtarır,
PAPER'da kurtarmaz). "Restart'ta tam durumla devam" iddiası yalnız **temiz kurulumda** doğru olur.
**Olması gereken:** üç alana da `#[serde(default)]`; ayrıca `serde_json::from_str` hatası **CRITICAL** olaya
yükseltilmeli (şu an WARN) ve bot `SUSPECT`/`PAUSED` ile açılmalı, körlemesine taze başlatılmamalı.

### N-2 · Hedge bacak emirleri **ana bacağın** `positionIdx`'ini alıyor · KRİTİK (para kaybı)
```
engine.rs:1105 / 1177:
position_idx: if rt.cfg.kind.is_futures() && rt.pos_mode_hedge {
    if rt.stepper.position.direction == 1 { 1 } else { 2 }
} else { 0 },
```
Bu kod **her iki bacak için de** çalışıyor: hedge bacağı emirleri `"-hedge"` id önekiyle aynı
`dispatch_action` yolundan geçiyor (`engine.rs:1058-1065`, `1130-1137`) ama `position_idx` hesabında
`rt.stepper` (ana bacak) kullanılıyor. Hedge bacağının yönü ana bacağın **tersi** olduğundan:

- Ana bacak long (idx 1), hedge bacak short → hedge'in `Sell` emirleri **idx 1** ile gidiyor.
- Bybit hedge-mode'da `Sell@positionIdx=1` = **long pozisyonu kapatma**; `reduceOnly=false` ile büyük olasılıkla
  **reddedilir** veya yanlış bacağa yazılır. Sonuç: hedge hiç açılmaz, ya da açılırsa yanlış bacakta kapanır.

**Olması gereken:** `let dir = if is_hedge { rt.hedge_stepper.position.direction } else { rt.stepper.position.direction };`
ve `position_idx = dir == 1 ? 1 : 2`. Ek olarak hedge emirlerinde `reduce_only` de **bacak pozisyonuna** göre
belirlenmeli (şu an `is_close` = rol tabanlı, doğru; ama idx ile tutarlı olmalı).

### N-3 · LIVE dolum **sweep**'i hâlâ fail-open (P0-4'ün ikinci yarısı) · YÜKSEK
- `engine.rs:1939` — `match ingest { Ok(Some) => {...} _ => {} }`: ingest **Err** olursa ne CRITICAL olay, ne
  `RISK` SSE, ne log. P0-4'te market yoluna eklenen görünürlük sweep'e eklenmemiş.
- `engine.rs:1950-1951` — fallback toplu dolum yalnız `fill_any_for_order(...) == false` ise üretiliyor.
  Emir **%40 dolduysa** bir fill kaydı vardır; sonra kalan %60 dolarsa ve `executions()` boş/hatalı dönerse
  **kalan asla muhasebeleşmez** → pozisyon eksik, TP/SL yanlış miktarla çalışır.
- `engine.rs:1962` — `if let Ok(Some(f)) = ...`: aynı sessiz yutma.

**Olması gereken:** tek bir `ingest` yardımcı fonksiyonu; her `Err` → CRITICAL `FILL_INGEST` + emir kitapta kalır;
fallback koşulu "hiç fill yok" değil, **"kitaptaki kalan miktar için fill yok"** olmalı (`order.qty − Σ ingested`).

### N-4 · Kısmi market dolumunun kalanı kitaba **fiyat 0** ile yazılıyor · ORTA-YÜKSEK
`engine.rs:620-631` `MarketOpen { price: Fraction::zero(), qty: remaining }` →
`engine.rs:808-829` bu sıfır fiyatı hem DB'ye (`upsert_live_order`) hem `live_open_orders`'a yazıyor.

- `cost = qty × 0 = 0` → **venue yük defteri** eksik (pre-order gate daha az yük görür, over-commit riski).
- sweep fallback'inde fiyat `st.price` yoksa `order.price = 0` (`engine.rs:1953`) → `m04::apply_fill` "fill price
  must be > 0" diye `Err` verir (`m04.rs:52-54`) → hata **yutulur** ve emir kitaptan silinir → hem pozisyon eksik
  hem takip kaybolur.

**Olması gereken:** `MarketOpen`'a gerçek referans fiyatı (`last_close` / `avgPrice`) taşı; 0 fiyatla
`LiveTrackedOrder` **oluşturulmasın** (fail-closed); fallback fill'de `px <= 0` → ingest etme + CRITICAL.

### N-5 · WFE cezası **birim uyumsuz** (oran ile getiri aynı ortalamada) · ORTA
`certify.rs:329-341`:
```rust
let contrib = if trial_is_tot[best] > 1e-12 {
    trial_oos_tot[best] / trial_is_tot[best]   // ORAN (ör. 0.8)
} else {
    trial_oos_tot[best].min(0.0)               // GETİRİ (ör. -0.05)
};
```
İki dal farklı birimde. Pratik sonucu: negatif-IS fold'un cezası ~|%5| mertebesinde kalırken iyi fold'lar
~0.8 katkı veriyor → ceza **sayısal olarak etkisiz**. Ayrıca `trial_is_tot` 1e-12'yi aşan **gürültü** pozitif
değerlerde oran patlar (payda → 0). Doğru kural: negatif IS fold → `contrib = -1.0` (ya da doğrudan
"WFE = OOS/IS, IS ≤ 0 ise -1") veya fold'u **doğrudan KALDI** say.

### N-6 · `CLOSE_ALL` `Closing` fazında **yok sayılıyor** (ve Studio bunu vaat ediyor) · ORTA
`main.rs:1682-1700` DCA için yalnız `e.request_close_all()` → `manual_close_requested = true` (`dca.rs:630-632`).
Bu bayrak **yalnız** `DcaPhase::Open` dalında okunuyor (`dca.rs:781-795`). `Closing` dalı (`dca.rs:801-826`)
bayrağa bakmıyor. Yani kullanıcı tam olarak "bot Closing'de takıldı" senaryosunda "Tümünü Kapat"a basıyor,
**hiçbir şey olmuyor** — üstelik `studio.tsx:494-498` metni tam bunu öneriyor.
**Olması gereken:** `Closing` dalında da bayrağı tüket (yeni MANUAL emri + `closing_bars_waited = 0`), ve
metni gerçeğe göre düzelt.

### N-7 · TP (reduce) emri de %10 hacim payı kapısına takılıyor → deal **kapanamaz** · ORTA-YÜKSEK
`kernel lib.rs:376-380` GATE5 tüm bekleyen emirlere uygulanıyor; `dca_risk lib.rs:161-170` payı
`bar_volume × 0.1` ile sınırlıyor ve `Reduce`/`TP` için **istisna yok**. Ayrıca GATE5'in `Defer`'i
yalnız gate log'una yazılıyor (`log_gate`), advisory üretmiyor.

**Etki:** büyük pozisyon / düşük hacimli sembolde `qty > 0.1 × bar_volume` → TP emri **her barda** `Defer` →
fiyat TP'yi geçse bile dolmaz, pozisyon açık kalır; kullanıcı "TP ulaşıldı ama kapanmadı" görür.
**Olması gereken:** risk-azaltan `Reduce` emirleri hacim payı kapısından **muaf** (GATE7 ile aynı mantık) ya da
pay yalnızca *kısmi* doldurmaya uygulanıp kalan sonraki bara devretmeli + `TP_DEFERRED` advisory'si.

### N-8 · Kernel `halted` durumu **arayüzde görünmüyor**, `RESET_KERNEL` için **düğme yok** · ORTA
`grep -rn "halted" src` → **0 sonuç**; `grep -rn "RESET_KERNEL" src` → **0 sonuç**. Backend `RESET_KERNEL`
eylemi var (`main.rs:1723`), ama kullanıcı bunu ancak curl ile çağırabilir. P0-2'nin "kill/freeze sonrası bot
bir daha çalışmıyor" şikâyeti pratikte **"görünmez şekilde duruyor"**a dönüşür: Studio'da faz rozeti `Open`
kalır, hiçbir emir gitmez.
**Olması gereken:** `snapshot_json`'a `kernelHalted` alanı; Studio/Fleet'te kırmızı rozet + "KERNEL'İ SIFIRLA"
(danger) düğmesi; `RESET_KERNEL` ayrıca **hedge kernel**'i de açmalı.

### N-9 · `HALTED_RESTORE` kapısı yalnız **adet** ve yalnız **DCA** bacağı için · ORTA
`state.rs:835-847`:
```rust
if let EngineTag::Dca(e) = &rt.engine_kind {
    let engine_qty = e.mirror.qty; let kernel_qty = rt.stepper.position.qty;
    if cmp != Equal { ... halt ... }
}
```
Karşılaştırılmıyor: **yön** (`direction`), **ortalama fiyat** (`avg_price`), **hedge bacağı**
(`hedge_stepper.position` vs `h.hedge`), grid tipi. Yanlış yönlü ama aynı adetli bir restore (short yerine long)
kapıdan **sessizce geçer** → TP/SL ters yönde çalışır.
**Olması gereken:** `(qty, direction, avg_price)` üçlüsü + her bacak için ayrı kapı; tolerans belgelenmiş
(ör. `avg` için `1 tick`) olmalı.

### N-10 · "Grid fee-breaker" hesabı **ortalama** adımı ölçüyor, ayrıca maker×maker varsayıyor · ORTA (P1-18 ile birlikte)
`advisory.rs:26-41` + `main.rs:1132-1142`: `per_level = (upper/lower − 1) / levels`, `g_min = fee_breaker_min_step(maker, maker)`.
Aritmetik mesh'te **en dar segmenti** değil ortalamayı ölçer; gerçek tur maliyeti genellikle maker-in/taker-out
(asimetrik) iken simetrik maker varsayılıyor. `validate_grid_fee_breaker` (min tabanlı, doğru) **kullanılmıyor**.
**Olması gereken:** mesh üretildikten sonra `min(P[i+1]/P[i] − 1)` ile kontrol (`validate_grid_fee_breaker`),
giriş/çıkış ücreti gerçek oranlardan.

---

## 7. "Olması gereken" — sıradaki iş listesi (öncelik sırasıyla)

**Hemen (para kaybı / çökme):**
1. **N-2** hedge `position_idx` — bacak bazlı yön. *(Tek satır; etkisi doğrudan yanlış bacağa emir.)*
2. **N-1** `#[serde(default)]` üç alana + restore hatasını CRITICAL'a yükselt.
3. **P0-11** `max_safety_orders ∈ [0,100]`, `grid_levels ∈ [2,500]`, `active_safety_orders ≤ 10` —
   hem `create_bot` 422'si hem `build_dca_plan` içinde savunma `Err`'ü.
4. **P0-9** `/app/api/engine/[...path]/route.ts` sunucu-taraflı proxy; `NEXT_PUBLIC_*` anahtarı sil.
5. **N-3/N-4** sweep'te fail-open temizliği + kısmi kalanın fiyatlı takibi.

**Kısa vade (yanlış muhasebe):**
6. **N-7** `Reduce` emirlerini hacim payı kapısından muaf tut.
7. **N-9** restore tutarlılık kapısını `(qty, yön, avg)` + hedge bacağına genişlet.
8. **P1-8** `reserved = Σ bekleyen Add maliyeti` defteri (GATE6 `available = cash − reserved`).
9. **N-6** `Closing`'de `manual_close_requested` tüketimi + Studio metninin düzeltilmesi.
10. **N-5** WFE ceza birimini tekilleştir (`-1.0` kuralı).

**Orta vade (finans/matematik doğruluğu):**
11. **P1-6** funding: `funding_history` + sembolün `fundingIntervalHour`'u; fetch hatası görünür olay.
12. **P1-10** `max_volume_share` config'ten; market emirlerde de pay.
13. **P1-18/N-10** grid: `n` çizgi = `n` seviye + `min` adım fee-breaker (`validate_grid_fee_breaker` bağlanmalı).
14. **P1-11** kurulumda stres teminatı kapısı (`required_collateral` 422).
15. **P1-19/P1-21** gerçek enstrüman metadata'sıyla ön-doğrulama; QuoteNotional'da açılış fiyatı beklentisi.
16. **P1-3** yıllıklandırılmış SR veya DSR ile aynı tanım.
17. **P1-23** `/v5/account/fee-rate` ile gerçek ücretler.
18. **P0-10(e)** sertifika istisnasını ayrı, süre sınırlı bir onaya bağla (UIX-10 birleştirmesi bunu zayıflattı).

**Arayüz / operasyon:**
19. **N-8** `halted` rozeti + `RESET_KERNEL` düğmesi (hedge kernel dâhil).
20. **UIX-5** `pending_engine_actions`'ı bar beklemeden (poll döngüsünde) dispatch et; `FROZEN` iken de çalıştır.
21. **P2-8** SSE `Last-Event-ID` + `Lagged` görünürlüğü; **P2-9** TICK'i debug'a al.
22. **UIX-13/14/16/17**, **P2-14** CI (`cargo test` + `tsc --noEmit` + `eslint`) — bu olmadan bu commit'teki
    test iddiaları her zaman "beyan" seviyesinde kalacak.

---

## 8. Bu raporda doğrulanamayanlar (açıkça)

| İddia | Durum |
|---|---|
| "workspace 276/276 test PASS" (`HATA_VE_EKSIK_ANALIZ_RAPORU`, Faz 17 notu) | **Doğrulanamadı** — bu ortamda `cargo` yok. |
| `certify` yeni testleri (WFE negatif fold) gerçekten geçiyor mu | **Doğrulanamadı** — derleme yok; kod okumasında mantık birim uyumsuz (N-5). |
| `resting_tp` PAPER'da makul fiyattan doluyor mu | **Statik olarak tutarlı** görünüyor (PlaceLimit → kernel pending → `OrderKind::Reduce`), ama gerçek FillRealism zinciri **test edilmedi**. |
| Sihirbaz/Lab akışının uçtan uca çalışması (`SİHİRBAZA TAŞI`, `DataFreshness`) | **`tsc` yok** — tip hatası riski doğrulanamadı. |
| Hedge `position_idx` hatasının venue tarafından reddedileceği | **Borsa davranışı varsayımı** (Bybit hedge-mode semantiği). Kod okuması kesin: hedge emri ana bacak yönüyle idx alıyor. |

**Öneri:** Bu raporun hükümleri `cargo test --workspace` + `cargo clippy` + `npx tsc --noEmit` çıktısıyla
birlikte **tek bir CI işinde** tekrarlanmalı; o zaman "düzeltildi" ifadesi beyan değil, kanıt olur.

---

## 9. Ek — Doğrulamada kullanılan somut komutlar (tekrar üretilebilir)

```bash
git worktree add --detach /tmp/fixed origin/master
git diff --stat 3ac500f origin/master              # 19 dosya, +1295/−228
# Örnek kanıt grepleri:
grep -n "position_idx" -A 4 mini-services/dca-core/crates/dca_server/src/engine.rs   # N-2
grep -n "funding_deal\|next_level_to_place\|closing_bars_waited" mini-services/dca-core/crates/dca_strategy/src/dca.rs  # N-1
grep -rn "plan_notional\|required_collateral" --include=*.rs mini-services            # P1-11 (çağıran yok)
grep -rn "NEXT_PUBLIC_ENGINE_API_KEY" src                                            # P0-9 (açık)
grep -n "reserved" -r mini-services/dca-core/crates                                  # P1-8 (rezervasyon yok)
grep -rn "RESET_KERNEL\|halted" src                                                  # N-8 (arayüzde yok)
```
