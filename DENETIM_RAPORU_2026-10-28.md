# DCABOT-Next-Rust — Kapsamlı Denetim Raporu
### Yazılım · Finans · Matematik · UI/UX · Mantık

**Tarih:** 28 Ekim 2026
**Kapsam:** `mini-services/dca-core` (Rust çekirdek, 9 crate) + `src/` (Next.js terminal) + `scripts/`, `tests/`, `docs/`
**Yöntem:** Tam statik kaynak okuması (satır düzeyi kanıt), sözleşme çapraz kontrolü (UI → API → kernel → strateji → adaptör), doküman ↔ kod karşılaştırması.
**Kısıt (dürüst sınır):** Bu ortamda `cargo`/`rustc` **yok**, `node_modules` **yok**. Bu nedenle `cargo test`, `tsc`, `eslint`, uçtan uca koşum **yapılamadı**. Aşağıdaki bulguların tamamı kaynak kod okumasıyla çıkarılmıştır; "canlı kanıt" diye sunulan hiçbir madde yoktur. Doğrulanması çalıştırma gerektiren maddeler bunu açıkça belirtir.

---

## 0. Yönetici Özeti

Projenin **matematik çekirdeği gerçekten iyi**: sıfır-float rasyonel aritmetik, WAC muhasebesi, MMR kademeli likidasyon formülü, fee-breaker, CSCV/DSR sertifikasyonu — bunlar sektör ortalamasının üzerinde ve bilinçli tasarlanmış. Arayüz de (kokpit, 5 adımlı sihirbaz, 7 sekmeli stüdyo, hesaplayıcı, Lab) ürünleşme açısından ciddi emek içeriyor.

Buna karşın **sistem, gerçek para yönetecek bir yürütme motoru olarak hâlâ güvenilir değil** ve bunun nedeni "eksik özellik" değil; **durum makinesi (state machine) ve mutabakat (reconciliation) katmanındaki mantık hataları**. Kritik sınıfındaki 11 bulgunun 6'sı doğrudan "bot sonsuza kadar kilitlenir / pozisyon sahipsiz kalır / dolum muhasebeye girmez" sonucunu üretiyor. Bunlar PAPER'da bile yaşanır; LIVE'da para kaybına dönüşür.

**En kritik üç cümle:**

1. **Kill-switch / freeze / pause bir botun çekirdeğini kalıcı olarak öldürüyor** — şalter bırakıldığında bot kendine gelemiyor (GATE1 interval kuralı → `halted` sticky). Yani acil durum freni, freni bıraktığınızda aracı hurdaya çeviriyor.
2. **PAPER restart'ta ana pozisyon snapshot'a yazılmıyor** — motor pozisyonu hatırlıyor, kernel unutuyor; bot "Closing" fazında hayalet pozisyonla kilitleniyor.
3. **Hedge moddaki hesapta kapanış emirleri yanlış `positionIdx` ile gidiyor** (Sell→2), yani DCA_LONG'un TP/SL'i borsa tarafından reddedilir veya yanlış bacakta işlem görür.

Bunlara ek olarak, LIVE emir hattında **dolum kaybı** (sessizce yutulan `MarketFilled`), **kısmi dolumun hiç takip edilmemesi**, **TradingView webhook'unun Bearer middleware arkasında kalıp dışarıdan çağrılamaması** ve **API anahtarının tarayıcı paketine gömülmesi** gibi maddeler var.

**Hüküm:** PAPER/backtest/Lab/hesaplayıcı: **kullanılabilir** (P0'lar düzeltildikten sonra günlük kullanım için sağlam). LIVE: **açılmamalı** — mevcut haliyle para kaybı riski yüksek, ayrıca kaybın *kaynağı* da görünmez olur çünkü muhasebe pozisyondan kopabiliyor.

| Sınıf | Adet | Anlam |
|---|---|---|
| **P0 — Kritik** | 11 | Para kaybı, kalıcı kilitlenme, güvenlik açığı; LIVE açılmadan önce zorunlu |
| **P1 — Yüksek** | 24 | Finansal doğruluk, risk kapıları, muhasebe sapması; ilk sprintte |
| **P2 — Orta** | 22 | Mimari borç, API ergonomisi, operasyon, test/CI |
| **UI/UX** | 17 | Ayrı bölümde (Bölüm 5) |

---

## 1. P0 — Kritik Bulgular

### P0-1 · Hedge modda kapanış emirlerinin bacak eşlemesi yanlış (LIVE, para kaybı)
**Kanıt:** `crates/dca_server/src/engine.rs:1051` (`PlaceLimit`) ve `engine.rs:1120` (`PlaceMarket`):
```rust
position_idx: if rt.cfg.kind.is_futures() && rt.pos_mode_hedge {
    if side == dca_math::Side::Buy { 1 } else { 2 }   // ← yalnız YÖNE bakıyor
} else { 0 },
```
**Sorun:** Bybit hedge modunda bacak **yönle değil, pozisyonla** eşleşir: long bacak = `positionIdx 1`, short bacak = `2`. Bir DCA_LONG pozisyonunu kapatmak `Sell + positionIdx=1 + reduceOnly` demektir. Kod ise her `Sell`'i koşulsuz `2`'ye gönderiyor.
**Etki:** DCA_LONG'un TP/SL/manuel kapanışı ya `reduceOnly` reddiyle döner (açık short pozisyon yok) ya da yanlış bacakta değerlendirilir. Pozisyon TP'yi geçer, kapanmaz, motor `Closing` fazına girer ve P0-6 uyarınca orada kilitlenir.
**Olması gereken:** `position_idx` **pozisyon yönüne** göre hesaplanmalı (`stepper.position.direction`), `reduce_only` emirlerde ise kapatılan bacağın idx'i kullanılmalı. Tek bir `resolve_position_idx(side, kind, hedge_mode, is_reduce, position_direction)` fonksiyonu yazıp iki yerde de onu çağırmak yeterli — ve birim testle sabitlenmeli (şu an bu harita için tek test yok).

### P0-2 · Kernel `halted` durumu kalıcı; kill/freeze/pause sonrası bot bir daha çalışmıyor
**Kanıt:** `crates/dca_kernel/src/lib.rs:206-222` (`if self.halted.is_some() { return Err(...) }`, `self.halted = Some(reason)`), `crates/dca_server/src/engine.rs:1677` (duraklatılmış/frozen botlarda imleç ilerliyor, `process_bar` çağrılmıyor), `engine.rs:1680-1689` (yalnız `b.t > rt.last_bar_ts` olan barlar işleniyor).
**Senaryo (deterministik):** 1h bot → `PAUSE` 3 saat → `START`. Kernel'in `prev_bar_ts`'i 3 saat önceki barda, sıradaki bar 3 saat sonra → `t - pt = 3 × interval ≠ interval` → `GATE1: interval gap` → `Reject` → `step()` `Halt` gibi davranıp `halted = Some(...)` yazıyor. Bu andan sonra **her** `step()` çağrısı Err döner; bot ölene kadar 20 saniyede bir `POLL` ERROR olayı yazar.
**Aynı yol:** KILL_SWITCH engage → release, FREEZE → UNFREEZE, uzun süren poll (bar atlama), borsa bakımı, klines penceresinin dışına düşen imleç.
**Ek:** `halted` snapshot'a yazılmadığı için süreç yeniden başlatıldığında "kendiliğinden" düzeliyor — yani hata determinist değil, teşhisi zor.
**Olması gereken:** (a) Veri boşluğu **Halt değil, "skip & resync"** olmalı: imleci son kapanmış bara hizala, `bar_interval_ms`'i yeniden öğren, WARN olayı üret (Freqtrade'in `ohlcv_candle_limit`/`startup_candle_count` yaklaşımı). (b) `halted` yalnız gerçek veri bozulması (negatif fiyat, OHLC ihlali) için ayrılmalı ve `POST /api/bots/:id/actions {"action":"RESET_KERNEL"}` gibi açık bir kurtarma ucu bulunmalı. (c) Şalter bırakıldığında kernel yeniden senkronize edilmeli.

### P0-3 · PAPER restart'ta ana pozisyon kayboluyor → hayalet pozisyon + `Closing` kilidi
**Kanıt:** `crates/dca_server/src/state.rs:519-537` — `RuntimeSnapshot` alanları: `engine, cash, im_used, cumulative_fees, cumulative_funding, realized_net, last_bar_ts, grid_bootstrapped, leverage_synced, webhook_secret, hedge_kernel`. **`stepper.position` yok** (yalnız hedge bacağı için var: `HedgeKernelSnap.position`). `main.rs:618-620` — pozisyon uzlaştırması **yalnız LIVE** için (`if mode != "LIVE" { continue; }`).
**Etki:** Restart sonrası `engine_kind` (DcaEngine mirror) `phase=Open`, `mirror.qty=0.5` ile geri gelir; kernel ise `position.qty=0`. Sonuç:
- `equity()` kernel'den hesaplanır → arayüzde equity **nakit + 0** görünür (pozisyon değeri yok).
- TP tetiklenirse `Reduce` emri qty=0.5 ile gider; `PositionState::apply_fill` **OVERFILL** yazıp 0'a kırpar (`dca_math/src/m04.rs`) → realized 0, kernel düz.
- Engine mirror `apply_fill(reduce, r=0)` sonrası qty'yi korur (`qty = qty - 0`) → mirror hâlâ 0.5. Faz `Closing`'e geçer ve **hiçbir zaman** `qty.is_zero()` olmadığı için deal kapanmaz: bot kalıcı olarak ölü.
**Olması gereken:** `RuntimeSnapshot`'a `position: PositionState` (ve `prev_bar_ts`, `pending`, `filled_qty_this_bar`) eklenmeli; restore'da kernel önce, engine sonra yüklenmeli ve ikisinin tutarlılığı **assert edilmeli** (uyumsuzsa fail-closed: bot `HALTED_RESTORE` olarak işaretlenip kullanıcıya gösterilmeli, sessizce devam etmemeli).

### P0-4 · LIVE market dolumları sessizce kaybolabiliyor; idempotency anahtarı zamana bağlı
**Kanıt:** `engine.rs:700-742` (`IntentOutcome::MarketFilled`), `engine.rs:576-590` (dolum yakalama).
```rust
let ts2 = crate::state::now_ms();
let fk = format!("{}-{}", id, ts2);           // ← fill_key = sipariş + YEREL saat
...
if let Ok(Some(f)) = mf_target.ingest_external_fill_with_fee(...) {
```
**Sorunlar:**
1. `ingest` `Ok(None)` dönerse (ör. `price` sıfır/eksik geldiğinde `apply_fill` domain hatası, ya da `gate2` reddi) sonuç **tamamen yutulur**; ne log, ne WARN, ne yeniden deneme kuyruğu var. Yutulan dolum = borsada açık pozisyon, motorda yok.
2. `st.price` `None` ise (market emir ilk sorguda henüz fiyat yazmamış olabilir) `outcome_done` false kalır → `MarketNoTrace` üretilir, dolum **hiç** işlenmez.
3. `fill_key` yerel `now_ms` içeriyor → **aynı borsa dolumu iki farklı anahtar** üretir. Bugün çift sayım, `live_open_orders`'tan silme sırası sayesinde "genelde" önleniyor; ama retry/yeniden başlatma/çift cascade turunda koruma yok.
**Olması gereken:** Dolum kimliği **borsanın `execId`'si** olmalı (`exec_link_id-exec_id`, sweep'te zaten böyle). Ingest hatası `CRITICAL` olay + kalıcı `pending_fills` kuyruğu + her poll'da retry üretmeli; "işlenemedi" durumu asla sessiz kalmamalı. `MarketFilled` yolu tamamen `live_fill_sweep`'e devredilmeli (tek kaynak).

### P0-5 · Kısmi dolum takip edilmiyor (pozisyon eksik kapanır / eksik açılır)
**Kanıt:** `engine.rs:581-587`: `let filled = if st.cum_exec_qty < qty { st.cum_exec_qty.clone() } else { qty.clone() };` → kalan `qty - filled` **hiçbir yere yazılmıyor**, `MarketOpen` durumu da üretilmiyor (çünkü `cum_exec_qty > 0` dalına girilip `break` ediliyor).
**Etki:** Market TP emri 0.5'in 0.3'ünü doldurdu → 0.2 açık kalır; motor `Closing` fazında bekler, kalan 0.2 için emir yoktur. Aynı şekilde BASE kısmi dolarsa ladder eksik pozisyonla kurulur ve `filled_safety` sayacı yanlış ilerler.
**Olması gereken:** Her market emri için `remaining = qty - cum_exec_qty` izlenmeli; `remaining > 0` ise ya yeni bir dilim emri üretilmeli ya da emir "PARTIAL" olarak `live_open_orders`'ta kalıp sweep'te tamamlanmalı. Kapanışlarda `reduce_only` + kalan miktar **zorunlu** kapanış döngüsü olmalı (pozisyon asla sahipsiz bırakılmamalı).

### P0-6 · `Closing` fazının çıkışı yok; `PlaceFailed` fazı geri almıyor
**Kanıt:** `dca_strategy/src/dca.rs` `on_bar` → `DcaPhase::Closing => { /* awaiting close fill */ }`; `engine.rs:879-905` `rollback_pending_phase_after_gate_reject` yalnız `BasePending→Idle` geri alıyor; `IntentOutcome::PlaceFailed` (`engine.rs:770`) sadece WARN yazıyor.
**Etki:** TP/SL emri borsada reddedilirse (bakım, minNotional, `positionIdx`, risk limiti, bakiyesi kilitli) motor `Closing`'de sonsuza kadar bekler; pozisyon korumasız kalır. Ladder tarafında da aynı: SAFETY emri reddedilirse `resting_safety` sayacı şişer, sonraki top-up hiç yapılmaz.
**Olması gereken:** Emir başına **sonlu durum makinesi** (CREATED → SUBMITTED → PARTIAL → FILLED / REJECTED / EXPIRED) + `Closing` için zaman aşımı ve yeniden deneme politikası (backoff, max deneme, sonra `CRITICAL` + kullanıcıya "pozisyon açık, çıkış başarısız" alarmı). Reddedilen kapanış emri otomatik olarak daha agresif fiyatla yeniden denenmeli (reduce emirleri için GATE6 muafiyeti zaten var).

### P0-7 · LIVE `ADD_FUNDS` kill-switch'i ve tüm kapıları atlıyor; yön hatası içeriyor
**Kanıt:** `main.rs:1530-1600` — `ADD_FUNDS` LIVE dalında ağ çağrısı handler'ın **en başında**, `kill_engaged()`/`freeze_engaged()` kontrolü yok, `pre_order_gate_ctx` (minNotional + cüzdan + load) yok, ve `side: dca_math::Side::Buy` **sabit**.
**Etki:** (a) Kill-switch devredeyken bile borsaya emir gider — şalterin anlamı kalmaz. (b) `DCA_SHORT` botuna fon eklemek **alış** yapar → short pozisyonu azaltır/ters pozisyon açar. (c) Eklenen miktar sonrası `tp_price` yeniden hesaplanmaz (yalnız `manual_add_funds()` yapıyor, o da çağrılmıyor) → TP eski ortalama maliyete göre kalır, deal zararına kapanır.
**Olması beklenen:** Tüm emir üretimi **tek bir `submit_order` yolundan** geçmeli (şalter → risk kapıları → intent kuyruğu → cascade). Yön `position.direction`'a göre belirlenmeli; ekleme sonrası TP/ladder yeniden hesaplanmalı.

### P0-8 · TradingView webhook'u kimlik doğrulama duvarının arkasında → dışarıdan çağrılamaz
**Kanıt:** `main.rs:161` route tanımı, `main.rs:206` `.route_layer(auth_middleware)` — middleware **tüm** route'lara uygulanıyor; webhook handler'ı (`main.rs:3252`) ayrıca mükemmel bir sabit-zamanlı secret + replay + dedup korumasına sahip ama oraya **hiç ulaşılamıyor**: TradingView alert gönderirken özel HTTP başlığı (`Authorization: Bearer ...`) ekleyemez.
**Etki:** Ürünün "3Commas parity" iddiasının çekirdek özelliği (dış sinyalle deal açma) pratikte çalışmıyor; sadece curl/ara proxy ile çalışır.
**Olması gereken:** Webhook route'u auth katmanından **çıkarılıp** kendi imza doğrulamasına bırakılmalı (`/api/webhooks/tradingview` sadece per-bot secret + zaman damgası + dedup ile korunur; handler zaten bunu yapıyor). Ek olarak: yanıt süresi ölçümü, IP bazlı rate-limit, ve "secret'ı UI'da göster/kopyala/rotate et" akışı (kısmen var).

### P0-9 · API anahtarı tarayıcı paketine gömülüyor (`NEXT_PUBLIC_*`)
**Kanıt:** `src/hooks/engine-client.ts:35` `process.env.NEXT_PUBLIC_ENGINE_API_KEY`, `src/app/api/stream/route.ts:38`.
**Etki:** `NEXT_PUBLIC_` ile işaretlenen her değişken **istemci JS paketine düz metin** olarak girer. Bu anahtar tüm yönetim uçlarını (bot silme, kill-switch, vault listesi) açıyor. Yerelde "kabul edilebilir" gibi görünse de Cloudflare Tunnel / `DCA_EXTRA_ORIGINS` ile dışarı açıldığı anda **her ziyaretçi** motoru tam yetkiyle kullanabilir. Ayrıca token'ın süresi, kapsamı (scope) ve iptali yok.
**Olması gereken:** Tarayıcı motora **hiç** doğrudan konuşmamalı. Next.js **route handler'ları** (`/app/api/engine/[...path]/route.ts`) sunucu tarafında Bearer ekleyip proxy'lemeli; istemci yalnız kendi oturum çerezine (httpOnly) güvenmeli. Tek kullanıcılı yerel kurulumda bile anahtar sunucuda kalmalı.

### P0-10 · LIVE sertifika kapısı sistematik olarak iyimser + tek tıkla aşılabiliyor
**Kanıt:** `dca_strategy/src/certify.rs:315-320`:
```rust
if trial_is_tot[best] > 1e-12 { wfe_sum += trial_oos_tot[best] / trial_is_tot[best]; wfe_cnt += 1; }
```
**Sorunlar:**
1. **Seçim yanlılığı:** In-sample getirisi ≤ 0 olan fold'lar WFE ortalamasından **tamamen düşülüyor**. Yani "çoğu fold'da zarar eden" bir strateji, iki şanslı fold sayesinde WFE ≥ %50 alabilir. Walk-Forward Efficiency'nin tanımı gereği bu fold'lar **cezalandırılmalıydı** (negatif katkı olarak sayılmalı), atılmamalı.
2. `trial_is_tot` **bileşik getiri değil, per-bar getirilerin aritmetik toplamı** — farklı uzunluk/kırılım oranlarında anlamsız.
3. Deneme sayısı `trials.len()` = en fazla 9 varyant; DSR'nin "deneme sayısı" kavramı kullanıcının gerçekte denediği konfigürasyon sayısını yansıtmıyor (iyimser DSR).
4. Sertifika denemeleri **funding'siz** koşarken baz koşu funding'li koşuyor (kod yorumu bunu kabul ediyor) → karşılaştırma aynı maliyet tabanında değil.
5. `cert_acknowledged=true` tek boolean ile kapı **tamamen** aşılabiliyor (`main.rs:1284-1300`); kayıt yalnız bir WARN olayı.
**Olması gereken:** WFE negatif fold'ları cezalandırmalı (veya "IS≤0 ise strateji otomatik KALDI"); bileşik getiri kullanılmalı; funding tüm koşularda aynı olmalı; `cert_acknowledged` yalnız belirli bir süre/geçmiş için geçerli, imzalı ve **LIVE açılışında tekrar onay** isteyen bir akış olmalı.

### P0-11 · `max_safety_orders` üst sınırı yok → tek API çağrısıyla tüm motorun çökmesi
**Kanıt:** `dca_math/src/m02.rs:90` `Vec::with_capacity(safety_count)`; `main.rs:1096` doğrulama için `.min(1000)` kullanıyor ama **runtime** `self.cfg.max_safety_orders`'ı doğrudan kullanıyor (`dca_strategy/src/dca.rs` `try_open_deal`); `create_bot` bu alan için üst sınır koymuyor; `routes_calc.rs:77` `/api/calc/dca` da aynı alanı doğrulamasız alıyor.
**Etki:** `max_safety_orders = 10^9` olan bir bot (veya hesaplayıcı isteği) `with_capacity` ile doğrudan bellek ayırmaya çalışır → süreç abort → **tüm botlar** durur. Tek süreç mimarisi olduğu için yıkıcı.
**Olması gereken:** `max_safety_orders ∈ [0, 100]` (3Commas 50'de sınırlar), `active_safety_orders ∈ [1, 10]`, `grid_levels ∈ [2, 500]`; `build_dca_plan` içinde de savunma amaçlı `debug_assert` + `Err` dönüşü.

---

## 2. P1 — Yüksek Öncelikli (Finans ve Matematik)

### 2.1 Muhasebe ve PnL

**P1-1 · `realized_net` yanlış tanımlı.** `dca_kernel/src/lib.rs:527` `realized_net = realized_gross − cumulative_fees` — `cumulative_fees` **bot ömrü boyunca** ödenen tüm komisyonları içerir (açık pozisyonun giriş komisyonları dâhil) ve **funding düşülmemiştir**. Bu değer arayüzde ve snapshot'ta "net gerçekleşen" olarak dolaşıyor. *Olması gereken:* `realized_net = realized_gross − realized_fees − cumulative_funding` ve `realized_fees` yalnız kapanan miktara düşen komisyonlar olmalı (deal başına ayrı defter).

**P1-2 · Kazanma oranı komisyon/funding'siz brütten hesaplanıyor.** `dca.rs` `on_fill` → `realized_gross > 0 ⇒ win`. Gerçekte %0.1 komisyonla kapanan küçük kâr **kayıp**tır. *Olması gereken:* win/loss net (fee + funding düşülmüş) PnL'e göre; ayrıca `profit_factor` gerçekten hesaplanmalı (`backtest.rs:737` hâlâ `"n/a"` — arayüzde metrik olarak gösteriliyorsa boş söz).

**P1-3 · `display_sharpe` "Sharpe" değil.** `backtest.rs:876-884`: per-bar ortalama/std × **100**. Yıllıklandırılmamış, ölçeklenmiş bir sayı; Lab'de "Sharpe" etiketiyle gösterilirse kullanıcı yorumlayamaz. *Olması gereken:* ya yıllıklandırılmış (bar sayısı/yıl ile √) ya da açıkça "per-bar SR (ölçekli)" etiketi; DSR içinde kullanılan SR ile **aynı** tanım olmalı.

**P1-4 · Net TP hedefi maker komisyonla hesaplanıyor, çıkış taker icra ediliyor.** `dca.rs::compute_tp` → `exit_rate = inst.maker_fee_rate`, ama TP tetiklendiğinde `PlaceMarket` ile taker olarak çıkılır (`dispatch_action`). Ayrıca TP fiyatı "mark ≥ tp" ile **piyasa emrine** dönüşüyor; gerçek fill bir sonraki barın açılışı ± slippage. Yani "net TP" hedefi sistematik olarak tutmaz. *Olması gereken:* net TP hesabında taker ücret + beklenen slippage; ya da gerçekten **resting reduce limit emri** olarak borsaya yazılmalı (Bybit/OKX bunu destekler) ve dolum sweep'ten gelmeli.

**P1-5 · Kümülatif funding deal'lar arasında sıfırlanmıyor.** `dca.rs` `self.funding` deal açılışında sıfırlanmıyor (yorumda "M-17 kümülatif" deniyor) ve `compute_tp` bunu doğrudan TP fiyatına katıyor. 10. deal'de önceki 9 deal'in funding'i yeni TP'yi yukarı iter → bot **kâr realize edemez**, TP ulaşılamaz olur. *Olması gereken:* TP hesabı **deal-bazlı** funding kullanmalı; bot ömrü kümülatifi ayrı bir raporlama alanı olmalı.

**P1-6 · Funding mutabakatı yaklaşık ve geçmişe dönük yanlış.** `engine.rs:286-380`: (a) sınır geçişinde **şu anki** `ticker.funding_rate` uygulanıyor (geçmişteki gerçek oran değil, çoğu borsada bu "predicted" orandır); (b) fiyat olarak barın kapanışı kullanılıyor (sınır anındaki mark değil); (c) 8 saat sabit (`FUNDING_INTERVAL_MS`), 4 saatlik sembollerde yanlış; (d) `crossings > 1` durumunda **aynı** oran `crossings` kez çarpılıyor. Ayrıca `rate == 0` ise "settlement yok" sayılıp sessizce geçiliyor. *Olması gereken:* Backtest'te olduğu gibi `adapter.funding_history()` (bu uç zaten var, sertifika kapısı kullanıyor) ile **gerçekleşmiş** oranlar kullanılmalı; sembolün `fundingIntervalHour`'u enstrüman meta verisinden okunmalı.

**P1-7 · WAC eksiltmede (`reduce`) overfill "kırpılıyor" ama sessiz.** `m04.rs` `apply_fill(is_add=false)`: qty > açık miktar ise `violations`'a yazıp `min`'e kırpıyor. Kernel'de bu durum yalnız bir Vec'te kalıyor, hiçbir uyarı/olay üretilmiyor. Borsada fazla dolum olmuşsa (ör. manuel müdahale, iki bot aynı hesap) pozisyon **gerçekte** fazladır ama motorda değildir. *Olması gereken:* `violations` boş değilse `CRITICAL` olay + bot `SUSPECT` durumuna alınmalı, otomatik devam etmemeli.

### 2.2 Risk Kapıları (Gate 1–8)

**P1-8 · GATE6'da rezervasyon yok → merdiven kaçınılmaz olarak yarım kalır.** `dca_kernel/src/lib.rs:354` (`gate6_capital_guard(..., &self.cash, ...)`) yalnız **anlık nakit**e bakıyor; tahtada bekleyen SAFETY emirlerinin toplam yükümlülüğünü rezerve etmiyor. `active_safety_orders > 1` iken 3 emir aynı nakdi "sahiplenir"; fiyat düşünce ilk ikisi dolar, üçüncüsü `Defer` → **sonsuza kadar** bekler (`Wait` modunda). Bu, 3Commas'ın "SO fond" mantığının yokluğudur. *Olması gereken:* `reserved = Σ(bekleyen Add emirlerinin maliyeti)`; `gate6.available = cash − reserved`; ve `active_safety_orders` kadar emir yerleştirirken toplam maliyet **plan notionalı** ile karşılaştırılmalı (M03 `required_collateral` bunun için yazılmış — bkz. P1-11).

**P1-9 · Kapılarda fail-open'a dönüşen `unwrap_or_else(zero)` desenleri.** `dca_risk/src/lib.rs:168-173` (gate6): `qty.mul(price).unwrap_or(0)` → taşma/hata durumunda `notional = 0` ⇒ `required = 0` ⇒ **PASS**. `lib.rs:143-150` (gate5): `bar_volume.mul(0.1).unwrap_or(0)` → `cap = 0` ⇒ her emir `Defer` (fail-closed, iyi) ama sessiz. `lib.rs:255` (M07): yakınsamazsa `Ok(Some(0))` döndürüyor → likidasyon fiyatı **0** → long için hiç tetiklenmez (fail-open). *Olması gereken:* aritmetik hata = `Halt`, asla `Pass`.

**P1-10 · `max_volume_share` ayarı ölü; hacim payı %10 sabit ve yalnız limit emirlerde.** `StepperConfig::max_volume_share` (`lib.rs:96`) hiçbir yerde okunmuyor; `gate5` içinde `0.1` sabiti gömülü, `step()` içindeki `volume_cap` da aynı sabiti kullanıyor. Market emirlerde (BASE girişleri, TP/SL çıkışları) hacim kapısı **hiç** uygulanmıyor. *Olması gereken:* tek kaynak config; market emirler için de bar hacmine göre parçalama (iceberg/tranche).

**P1-11 · M03 (plan notional / stres teminatı) tamamen ölü kod.** `plan_notional`, `required_collateral`, `m20_validate_config` — yalnız testlerde kullanılıyor (grep ile doğrulandı). Yani "8 kapı + stres teminatı" iddiasının **stres teminatı** ayağı üretimde yok; sermaye yeterliliği yalnız tek-emir bazında GATE6 ile bakılıyor. Ayrıca `plan_notional`'ın `QuoteNotional` dalı hatalı: `n = n.mul(1)` ile aynı safety boyutunu tekrar tekrar topluyor. *Olması gereken:* Bot kurulumunda "-%30 şok altında merdiven tamamen dolarsa teminat yeter mi?" kapısı (422 + uyarı), ve formülün düzeltilmesi.

**P1-12 · GATE2 negatif ve absurd ücret oranlarını kabul ediyor.** `gate2_fee_guard` yalnız `rate == 0` kontrolü yapıyor; `-0.001` (rebate) veya `5` (absurd) geçer. *Olması gereken:* `0 < rate ≤ 0.01` aralık kontrolü (negatif rebate ayrı bir `FeeRole::Rebate` olarak modellenmeli).

**P1-13 · GATE7/GATE8 zaman damgası geçersizse `Utc::now()`'a düşüyor.** `dca_risk/src/lib.rs:201`, `lib.rs:244` (`from_timestamp_millis(ts_ms).unwrap_or_else(|| chrono::Utc::now())`). Bu, "deterministik, tekrar üretilebilir backtest" iddiasını kırar: aynı bar dizisi farklı günlerde koşarsa farklı seans hükmü üretebilir. *Olması gereken:* geçersiz ts = `Halt` (veri hatası), asla "şimdi".

**P1-14 · GATE8 gap koruması fiilen ölü.** `dca_kernel/src/lib.rs:358` `gate8_gap_protection(order.role == "SAFETY", ts_ms, &o, &[])` — `pending_level_prices` **her zaman boş dizi**; `all_gapped` boş listede `true` döner ama `!is_empty()` koşulu yüzünden hiçbir zaman `ConsolidateGap` üretilmez. Ayrıca `ConsolidateGap.consolidated_qty` **miktar değil, adet** sayıyor (`q.add(&Fraction::one())`). Gerçek gap konsolidasyonu `step()` içinde ayrı bir yoldan (gap_batch) yapılıyor; yani aynı iş iki yerde, biri ölü ve yanlış. *Olması gereken:* tek yol; `consolidated_qty` gerçek toplam miktar olmalı ve API'de "adet" olarak sunulmalı.

**P1-15 · GATE1 OHLC bütünlüğü eksik.** Sıfır hacim, `h == l` (donmuş bar), önceki bara göre aşırı sıçrama (spike) ve `t` gelecekte olma durumları kontrol edilmiyor. Borsa API'sinin boş/donmuş veri döndürmesi gerçek bir senaryo; motor bunu gerçek fiyat sanıp emir üretir. *Olması gereken:* hacim > 0, `t ≤ now + interval`, ve `|Δclose|/close > eşik` → `Reject + resync`.

### 2.3 Matematik Çekirdeği (M01–M21)

**P1-16 · DCA güvenlik emri "top-up"ı aynı fiyata **ikinci** emir yazıyor.** `dca.rs:753-765`: SAFETY dolumu sonrası `filled_safety` **hem dolum sayacı hem yerleştirme imleci** olarak kullanılıyor:
```rust
self.filled_safety += 1;                       // dolum
...
self.resting_safety = self.resting_safety.saturating_sub(merged_levels);
while self.resting_safety < want && self.filled_safety < self.ladder.len() {
    let lvl = self.ladder[self.filled_safety].clone();   // ← L2 zaten tahtada!
    ...
    self.filled_safety += 1;                   // ← yerleştirme sayacı gibi
}
```
`on_base_filled` ilk N seviyeyi (L1..LN) tahtaya yazmıştır. L1 dolunca `filled_safety = 1` olur ve top-up **`ladder[1]` = L2**'yi yeniden yerleştirir → **aynı fiyatta iki emir** → fiyat oraya gelince pozisyon iki katı büyür. Test (`multi_resting_safety_orders_top_up`) yalnız "emir sayısı 1" diye bakıyor, fiyatı kontrol etmiyor; bu yüzden hata testten kaçmış. Ayrıca `snapshot.safety_triggered = filled_safety` artık "dolan + yerleştirilen" karışımı → arayüzde yanlış sayı.
*Olması gereken:* iki ayrı sayaç: `filled_levels` (dolum) ve `next_level_to_place` (yerleştirme imleci); hiçbir seviye iki kez yerleştirilmemeli (idempotent `level → order_id` eşlemesi).

**P1-17 · `liquidation_price` bracket iterasyonu ıraksayabiliyor.** `m03_m07.rs:255` iterasyon 8 turda yakınsamazsa **`Some(0)`** döner. Likidasyon fiyatı 0 = hiç tetiklenmez. *Olması gereken:* yakınsamama = `Err` → "likidasyon hesaplanamadı" WARN + pozisyonu **kapat** (fail-closed), ya da son adayın gerçek kökünü döndürüp `bracket_uncertain` bayrağı koy.

**P1-18 · Grid: seviye sayısı ve fee-breaker aritmetiği hatalı.** `geometric_mesh`/`arithmetic_mesh` `n` seviye için **n+1** fiyat üretiyor (`0..=n`) → `grid_levels = 10` diyen kullanıcı 11 çizgi alır, arayüzdeki sayı ile motorun sayısı uyuşmaz. Fee-breaker ise `(upper/lower − 1)/levels` (ortalama adım) kullanıyor (`grid.rs:82-84`) — aritmetik mesh'te adımların **mutlak** boyutu eşit olduğu için üst segmentlerdeki bağıl adım çok daha küçüktür; yani kontrol **en dar segmenti değil ortalamayı** ölçer ve geçmesi gereken bir grid'i geçirir/kalması gerekeni geçirir. *Olması gereken:* komşu çiftler üzerinden `min(P[i+1]/P[i] − 1)` ile kontrol; `n` seviye = `n` çizgi (veya UI'da "n+1 çizgi" açıkça yazılmalı).

**P1-19 · Merdiven kurulumu nominal çapayla doğrulanıyor, gerçek fiyatla çalışıyor.** `main.rs:1080-1099`: kurulum doğrulaması sabit `anchor=1000`, `tick=0.01`, `step=0.001` ile yapılıyor. Gerçek piyasada (ör. SHIBUSDT, tick 1e-8) aynı config ya geçersizdir ya da tamamen farklı davranır; `build_dca_plan` tick yuvarlaması sonrası "not below previous tick level" hatası verebilir ve **bot hiç deal açamaz** (her bar `ERROR ladder rejected`). *Olması gereken:* doğrulama gerçek enstrüman metadata'sı + canlı fiyatla (create anında `/api/markets` zaten çağrılıyor).

**P1-20 · `build_dca_plan` tick yuvarlamasında seviyeleri birleştirirse hata veriyor.** Küçük `deviation_pct × step_scale` ve büyük `tick_size` kombinasyonunda iki seviye aynı fiyata düşer → `DomainViolation`. Doğru davranış (aynı fiyatlı seviyeleri birleştirmek) yerine tüm plan reddediliyor. *Olması gereken:* birleştir + `merged_levels` metadata'sı (zaten `.m{n}` mekanizması var).

**P1-21 · QuoteNotional base emir miktarı bar kapanışından hesaplanıyor.** `dca.rs` `try_open_deal`: `qty = qty_down(base_qty / anchor)`; `anchor = ctx.close`, ama emir bir sonraki barın açılışında ± slippage ile doluyor. Volatil sembollerde BASE notional ±%2 sapar. Minor ama `base_order_size`'ı "tam olarak şu kadar dolarlık al" diye düşünen kullanıcı için şaşırtıcı.

**P1-22 · Sertifika metrikleri ile Lab rozetleri aynı koşuyu kullanmıyor.** `certify` denemeleri funding'siz (`run_trial` → `&[]`), baz koşu funding'li; LIVE kapısı `funding_ok` bayrağına bakıyor. Sonuç: kullanıcı Lab'de "GECTI" görüp LIVE'da "KALDI" alabiliyor (veya tersi). *Olması gereken:* tek `RunConfig` (funding dâhil/hariç, slippage, fill model) her iki yolda da aynı olsun ve yanıtta **hangi maliyet varsayımlarıyla** hesaplandığı yazılsın.

**P1-23 · Bybit ücret oranları koda gömülü.** `bybit.rs:166-168`: linear `0.0002/0.00055`, spot `0.001/0.001`. VIP kademeleri, promosyonlar ve "maker rebate" hesaplanmıyor; `/v5/account/fee-rate` mevcut ve çağrılmıyor. Spot alımlarda ücret **base coin** cinsinden kesiliyor; `executions()` bunu quote'a çeviriyor (`bybit.rs:580-590`, iyi) ama `ingest` yolunda fee zaten quote varsayılıyor (`lib.rs` yorumu: "ücretin quote cinsinde olduğu varsayımı belgelidir") → spot BASE dolumlarında komisyon **sistematik olarak eksik/yanlış** muhasebelenir.

**P1-24 · Kâr hedefi + komisyon "fee guard"ı yalnız sihirbazda.** `wizard.tsx:700-707` fee guard hesabı yalnız UI'de; motor tarafında `target_profit_pct`'in komisyonları aşıp aşmadığını denetleyen bir kapı yok (`compute_warnings`'e eklenmeli). API'den kurulan bot için bu koruma tamamen yok.

---

## 3. P2 — Mimari, API, Güvenlik, Operasyon

**P2-1 · Snapshot tutarsızlıkları.** `RuntimeSnapshot`'ta `prev_bar_ts`, `pending` (resting emirler), `filled_qty_this_bar`, `breaker_day/breaker_tripped`, `regime_closes`, `filled_safety` yer almıyor (engine serde içinde kısmen taşınıyor). Sonuç: restart sonrası günlük DD kesici **sıfırdan** çapalanıyor (gün içi kayıp koruması atlanabilir), rejim filtresi 200 bar ısınmaya düşüyor.

**P2-2 · `webhook_secret` yorumu kodla çelişiyor.** `state.rs:529-533` yorumu "Secret DB'ye yazılmaz" diyor ama `save_runtime_state` içinde `webhook_secret: Some(rt.webhook_secret.clone())` ile `bots.runtime_state`'e yazılıyor. Doküman/kod uyumsuzluğu; güvenlik açısından da secret'ın DB'de düz metin durması demek.

**P2-3 · Kimlik doğrulama sabit-zamanlı değil.** `main.rs:707` `&auth[7..] != api_key.as_str()` (byte-by-byte early-exit). Webhook handler'ı sabit-zamanlı yazılmış (`main.rs:3260`) — aynı titizlik API anahtarına da uygulanmalı. Ayrıca anahtar tek ve global; scope/expiry/rotation yok.

**P2-4 · Vault anahtar türetimi zayıf.** `store.rs:243` `derive_32 = SHA-256(env)`. Tuz yok, iterasyon yok, AAD yok; `DCA_VAULT_KEY` zayıf bir parola ise offline kaba kuvvetle kırılabilir (launcher 64 hex üretiyor, o yüzden yerelde risk düşük). Anahtar rotasyonu/`re-encrypt` ucu yok. *Öneri:* Argon2id + salt (DB'de saklanan), `aad = key_id`, rotasyon ucu.

**P2-5 · Anahtar izinleri yalnız ekleme anında doğrulanıyor.** `add_key` → `key_permissions` + `WithdrawalGate`. Kullanıcı borsada sonradan çekim iznini açarsa motor bunu **asla** fark etmez. *Öneri:* periyodik (6 saat) izin yeniden taraması + izin değişirse botu `FREEZE`.

**P2-6 · `create_bot` içinde ağır senkron iş.** 500 bar klines + tam `certify` (9 varyant × backtest + 100 MC) `spawn_blocking` içinde ama istek **dakikalarca** sürebilir; UI'da progress/iptal yok. Ayrıca aynı anda 3 kullanıcı bot kurarsa CPU tavanı vurulur.

**P2-7 · DB tek `Mutex<Connection>`.** Tüm okuma/yazmalar serileşiyor; `poll_inner` her bot için tüm LIVE botların yükünü yeniden hesaplıyor (`engine.rs:1413-1436`) → O(n²). 20+ botta gecikme kaçınılmaz. *Öneri:* WAL + ayrı okuma bağlantısı havuzu (`r2d2`/`deadpool`), yük defterini artımlı tut.

**P2-8 · SSE akışı yeniden başlatılabilir değil.** `sse_handler` (`main.rs:3612`) `broadcast::RecvError::Lagged` durumunda sessizce `continue` → istemci **veri kaybını bilmez**; `Last-Event-ID` desteği yok. `PING` sayacı zamana değil **olay sayısına** bağlı (`n % 15`), yani sessiz piyasada hiç ping gitmez.

**P2-9 · Olay tablosu şişmesi.** `poll_and_advance` her turda `TICK` olayı yazıyor (`engine.rs:1364-1374`); 10 bot × 20 s → günde ~430k satır; retention 200k'da kırpıyor (`state.rs:470-476`) yani **son birkaç saatten fazlası kayboluyor** (forensics için kötü). *Öneri:* TICK'i debug seviyesine al / ayrı tablo + daha uzun retention.

**P2-10 · API eksikleri.** Liste uçlarında sayfalama yok (`/fills`, `/events`, `/bots`), `POST /bots` için idempotency anahtarı yok, rate-limit yok, `If-Match`/versiyon kontrolü yok (eşzamanlı PATCH kaybı).

**P2-11 · `XTransformPort` query parametresi her isteğe ekleniyor.** `engine-client.ts:22` — sandbox gateway artefaktı; gerçek dağıtımda anlamsız gürültü ve loglarda kafa karışıklığı. Ayrıca `NEXT_PUBLIC_ENGINE_PORT` istemciye gömülü.

**P2-12 · `.env` ayrıştırıcısı naif.** `main.rs:50-66`: tırnak/kaçış/yorum desteği yok, `export` yok, BOM yok; `if var(k).is_err()` mantığı boş değerleri de "tanımsız" sayar. `dotenvy` kullanılmalı.

**P2-13 · Sabit/gömülü yollar.** `main.rs:69-73` DB adayları arasında `/home/z/my-project/db/...` var; `bybit.rs:31` env ile override edilebilir (iyi). Docker/dağıtım için yol konfigürasyonu tek yerde toplanmalı.

**P2-14 · CI yok.** `.github/` dizini mevcut değil; `cargo test`, `tsc --noEmit`, `eslint`, `pytest tests/e2e_engine.py` hiçbir otomatik kapıdan geçmiyor. `tests/e2e_engine.py` `E2E_API_KEY` çevre değişkeni verilmezse boş Bearer ile **401** alır (varsayılanı boş) — suite yanıltıcı şekilde "kırık" görünür.

**P2-15 · Rust araç zinciri sabitlenmemiş.** `rust-toolchain.toml` yok, `Cargo.lock` var; `cargo build --offline --locked` kullanılıyor ama sürüm drift'i garantilenmiyor.

**P2-16 · Başlatıcı betikleri yalnız Windows/PowerShell.** `package.json` `local:start` → `powershell -File scripts/start-local.ps1`. Linux/macOS'ta proje **hiç çalışmıyor**; README de bunu "Windows üzerinde yerel çalıştırma" diye kabul ediyor. Docker Compose veya çapraz platform `make`/`just` hedefleri olmalı.

**P2-17 · Dokümantasyon kodu geçmiş.** `Caddyfile` `localhost:81` dinliyor, README "Arayüz: http://127.0.0.1:3000" diyor; `next.config.ts` rewrite'ı 3002'ye gidiyor; rapor "Binance ikincil" diyor ama `create_bot` LIVE'da Binance'i 422 ile reddediyor. Yeni gelen bir geliştirici için üç farklı "gerçek" var.

**P2-18 · Rapor/ledger dosyaları depoda.** `HATA_VE_EKSIK_ANALIZ_RAPORU_2026-10-25.md` (117 KB), `worklog.md` (181 KB), `PROJE_DENETIM_RAPORU.md` — kaynak ağacını kirletiyor; `docs/` altında arşivlenmeli.

**P2-19 · `.gitignore` artıkları.** `test`, `prompt`, `3100`, `local-*`, `dev.out.log` gibi desenler yanlış pozitif üretiyor (`test` klasörünü de yok sayar — `tests/` ayrı ama `test` deseni kafa karıştırıcı).

**P2-20 · `E2E` testleri mimari olarak kırılgan.** Python suite gateway portu, sabit sembol, gerçek Bybit verisi ve canlı motor bekliyor; mock/fixture katmanı yok. Kernel için birim testler iyi ama **entegrasyon** için deterministik bir "fake venue" (kaydedilmiş klines + sahte adapter) hedeflenmeli.

**P2-21 · Ölü/tekrar kod.** `plan_notional`, `required_collateral`, `m20_validate_config`, `gate8`'in `pending_level_prices` dalı, `max_volume_share`, `profit_factor` — hepsi "tanımlı ama kullanılmıyor/çalışmıyor". `#![deny(dead_code)]` veya `cargo udeps` CI'ya girerse bunlar yakalanır.

**P2-22 · Hata mesajlarında sır sızma riski.** `bybit.rs` hata mesajları `retCode + msg` döndürüyor; `pre_order_gate_ctx` cüzdan hatasını doğrudan SSE'ye yazıyor. Sembol/bakiye bilgisi UI'da görünür (tek kullanıcı için sorun değil, çok kullanıcılı senaryoda değil).

---

## 4. Mantık / Durum Makinesi Bulguları (özet tablo)

| # | Akış | Bozuk davranış | Sonuç |
|---|---|---|---|
| L1 | PAUSE→START, KILL→release, FREEZE→unfreeze | GATE1 interval gap → sticky halt | Bot kalıcı ölür |
| L2 | Restart (PAPER) | `stepper.position` snapshot'ta yok | Hayalet pozisyon, `Closing` kilidi |
| L3 | TP/SL place reddi | `Closing`'den çıkış yok, `PlaceFailed` rollback yok | Pozisyon korumasız kalır |
| L4 | Market kısmi dolum | Kalan miktar izlenmiyor | Pozisyon eksik kapanır |
| L5 | Market dolum ingest hatası | `if let Ok(Some(f))` sessiz yutma | Dolum kaybolur |
| L6 | Hedge modda kapanış | `positionIdx` yönle eşleniyor | Emir reddi / yanlış bacak |
| L7 | `ADD_FUNDS` LIVE | Şalter+kapı atlanıyor, yön sabit Buy | Şalter işlevsiz; short'ta ters işlem |
| L8 | SAFETY top-up | Aynı seviye iki kez yerleştiriliyor | Pozisyon 2× büyür |
| L9 | Grid `CLOSE_ALL` | `422 close-all only on DCA engines` | Kullanıcı grid pozisyonunu kapatamıyor |
| L10 | START (STOPPED bot) | `stepper.halted` / `phase=Closing` sıfırlanmıyor | "Başlat" işe yaramaz |
| L11 | Webhook START_DEAL | Handler'a ulaşılamıyor (P0-8) | Özellik ölü |
| L12 | GATE6 `Wait` modu | `Defer` sonsuz | Merdiven tamamlanmaz, sessiz |
| L13 | `active_safety_orders` > ladder | `while ... filled_safety < ladder.len()` | Erken kesilir, sessiz |
| L14 | Funding `rate == 0` | Erken `return Ok(())` | Settlement hiç kaydedilmez |
| L15 | `equity()` LIVE uzlaştırma | Venue flat → yerel pozisyon "tahmini" fiyatla kapatılıyor | PnL uydurma (raporlanıyor ama muhasebeye giriyor) |

---

## 5. UI / UX Bulguları

**UIX-1 · SSE istemcisi sunucunun tüm olaylarını dinlemiyor.** `src/hooks/engine-stream.ts:136`:
```ts
for (const t of ["FILL","EQUITY","LOG","BOT_STATE","ORDER","RISK","PING","KILL_SWITCH","FREEZE"]) ...
```
Sunucu ayrıca **`FUNDING`, `LIQUIDATION`, `WEBHOOK_START`** yayınlıyor (`grep 'event: "'` ile doğrulandı). Kokpitte bu olaylar için **hazır çipler var** (`dashboard.tsx:44-75`: FUNDING PAID/REBATE, LIQUIDATION) — yani ölü kod. Kullanıcı funding ödemesini ve likidasyonu canlı akışta **hiç görmüyor**.

**UIX-2 · `dataVer` sinyali de aynı listeye bağlı.** `DATA_EVENT_TYPES = new Set(["FILL","EQUITY","BOT_STATE","ORDER","LOG"])` → FUNDING/LIQUIDATION geldiğinde panel refetch tetiklenmez; ekran bayat kalır.

**UIX-3 · Pine backtest TP/SL alanları % soruyor, motora fraksiyon gidiyor (100×).** `wizard.tsx:1586` etiket: **"Bot TP %"**, değer `pineTp.trim()` olarak aynen gönderiliyor (`wizard.tsx:1650`), backend `cfg.target_profit_pct = tp` (fraksiyon sözleşmesi, `main.rs:1817`). Kullanıcı "2" yazar → hedef **%200** olur → TP hiç tetiklenmez, backtest sonucu sessizce yanlış. Aynı hata SL'de. **Düzeltme:** `String(Number(pineTp)/100)` ve alan yanında fraksiyon ipucu.

**UIX-4 · Birim modeli tutarsız (fraksiyon ↔ yüzde).** Aynı `BotConfig` içinde: `deviation_pct`, `target_profit_pct`, `trailing_*`, `stop_loss_pct` = **fraksiyon**; `daily_dd_breaker_pct`, `hedge_drawdown_trigger_pct` = **yüzde**; `combined_target_profit` = **mutlak quote**. Sihirbaz bunu kullanıcıdan gizliyor (`/100` dönüşümleri) ama API'den bot kuran, docta örnek veren veya Studio'da alan düzenleyen herkes için **tuzak**. Kanıt: README'de bile "deviation_pct KESİR olarak yazılır" uyarısı var. *Öneri:* alan adlarını `*_frac` / `*_pct` olarak ayır ya da tek bir `Fraction` tipi + UI'da tek dönüşüm noktası; API'de iki birimi de kabul edip açıkça reddet.

**UIX-5 · `CLOSE_ALL` grid'de 422 dönüyor** (`main.rs:1634`) — Studio'da grid botu için "Tümünü Kapat" butonu varsa kullanıcı anlaşılmaz bir hata alır; yoksa da pozisyonu kapatmanın yolu yok. Grid için "durdur + mesh iptal + market kapat" akışı gerekli.

**UIX-6 · `usePoll` "sessiz atlama" yapıyor.** `engine-stream.ts:31-40`: `busy.current` true ise istek atlanır, kullanıcıya **hiçbir gösterge** gitmez; "son güncelleme" zamanı hiçbir yerde yok. Yavaş motorda ekran donmuş gibi görünür. *Öneri:* `lastUpdatedAt` + `stale` rozeti.

**UIX-7 · Sayı biçimlendirme yerelleştirilmemiş.** `lib/format.ts` `Intl.NumberFormat("en-US")` — arayüz Türkçe ama `$1,234.56` gösteriliyor. Türk kullanıcı için binlik/ondalık ayırıcı ters. Bilinçli tercih ise yazılmalı; değilse `tr-TR`.

**UIX-8 · Ham `<Input>`'larda görünür odak halkası yok.** `className="... outline-none focus:border-emerald-500/50"` (ör. `wizard.tsx:1577`) — klavye kullanıcısı için sınır rengi değişimi tek ipucu; düşük kontrast. `focus-visible:ring-2 ring-emerald-500/60` eklenmeli.

**UIX-9 · `microOf` hassasiyet kaybı.** `bot-defaults.ts:52-58` `String(fracToNum(...))` — 1e-8 `step_size` `"1e-8"` olarak gider (motor `from_decimal_str` e-notasyonunu desteklediği için çalışır) ama `0.00000001` yerine bilimsel gösterim; ayrıca f64 dönüşümü "sıfır-float" iddiasını UI katmanında deler. Kesirli string'i (`"1/100000000"`) doğrudan taşımak daha güvenli.

**UIX-10 · Sihirbaz "onay kutusu" akışı çift yönlü.** `risk_acknowledged` ve `cert_acknowledged` ayrı iki onay; kullanıcı iki farklı modal/kutu ile karşılaşıyor ve hangi riski kabul ettiğini metinden ayırt etmesi zor (`wizard.tsx:795-830`). Tek bir "Risk Özeti" ekranı + tek onay önerilir.

**UIX-11 · Lab ↔ Studio ↔ Sihirbaz arasında config taşınmıyor.** Sihirbazda backtest yapılıp "bu konfigürasyonla bot kur" akışı yok (config yalnız `sessionStorage`'a yazılıyor: `wizard.tsx:786`). Sektör standardı: Lab'de beğenilen konfigürasyon → tek tıkla bot.

**UIX-12 · Studio'da `phase` rozeti "Closing"de iken kullanıcıya eylem önerilmiyor.** P0-6 ile birleşince kullanıcı sonsuz "Closing" görür, hiçbir buton çözmez.

**UIX-13 · Mobil alt bar doğru kurgulanmış (4+Drawer, `page.tsx:52-56`)** — bu iyi; ancak `KillButton` header'da her ekranda tekrar ediyor ve `dashboard.tsx`'te de ayrıca var → yanlışlıkla basma riski artıyor (onay diyaloğu var mı kontrol edilmeli).

**UIX-14 · Empty/error durumları tutarsız.** Bazı panellerde `EmptyState`, bazılarında ham `"—"`; motor kapalıyken (`CORE OFFLINE`) formların çoğu hâlâ etkin, kullanıcı submit edip "Çekirdeğe ulaşılamıyor" alıyor. Global "offline" modunda yazma eylemleri devre dışı bırakılmalı.

**UIX-15 · Grafik erişilebilirliği kısmen çözülmüş** (`docs/ui-ux-p1-p8-ledger.md`'de P4 "klavye + tablo + sr özet" tamamlandı deniyor) — ancak kontrast ölçümü ve ekran okuyucu turu **yapılmadığı** aynı belgede kabul edilmiş. `text-slate-400` üzerinde `#0B0E14` zemin ≈ 7:1 (iyi), `text-slate-500` ≈ 4.6:1 (sınırda, küçük puntoda riskli).

**UIX-16 · Tüm arayüz `dark` sabit.** `layout.tsx` `<html className="dark">`; `next-themes` bağımlılığı kurulu ama kullanılmıyor. Açık tema desteği ya kaldırılmalı (bağımlılık temizliği) ya tamamlanmalı.

**UIX-17 · Türkçe/İngilizce karışık metin.** Bilinçli ("motor terimleri İngilizce" kuralı `bot-defaults.ts` başında yazılı) — ama hata mesajları motor tarafından Türkçe, bazı UI metinleri İngilizce (`"no deals"`, `"ORDER"`), bazıları melez. Tek sözlük dosyası (`TERM`) var, kullanımı tutarsız.

---

## 6. İyi Olan / Korunması Gereken

Bunlar bozulmamalı; düzeltmeler sırasında referans alınmalı:

1. **M01 rasyonel aritmetik** (`fraction.rs`) — big-int, gcd normalizasyonu, half-even görüntüleme, kesin `floor_div`/`ceil_div`. Testler anlamlı (0.1+0.2, `1/3×3`, bit limiti).
2. **M07 likidasyon formülü** — izole marj, kademeli MMR + deduction, long/short simetrisi doğru türetilmiş ve testli.
3. **M05 net TP türetimi** — funding ve çıkış komisyonunu kapatan fiyat formülü matematiksel olarak doğru (sadece ücret türü/icra varsayımı hatalı, P1-4).
4. **M15 fee-breaker** kavramı ve `integer_nth_root` (kayan noktasız tam kök) — nadir ve değerli.
5. **Sertifikasyon istatistiği** — CSCV/PBO, DSR (Bailey–López de Prado çerçevesi), blok bootstrap Monte Carlo, `inv_norm_cdf`/`norm_cdf` doğru uygulanmış. Metodolojik hatalar (P0-10) düzeltilirse sektörde ender bir özellik.
6. **Zero-withdrawal hard gate** (anahtar ekleme anında 422) ve **vault'un DB dışı anahtar** modeli.
7. **Webhook handler'ının savunma katmanları** (sabit-zamanlı secret, replay penceresi, dedup, pair eşleşmesi) — sadece middleware'e takılıyor.
8. **Fill idempotency** çekirdeği (`INSERT OR IGNORE` + `seen_fill_keys`) ve `execId` tabanlı sweep tasarımı.
9. **Kill/freeze karar matrisi**, olay akışından rehydrate (restart-safe).
10. **UI kurgusu**: 7 sekmeli stüdyo, 5 adımlı sihirbaz, mobil 4+Drawer, `usePoll` tek uçuş koruması, `aria-live` durum rozetleri.

---

## 7. Öncelikli Düzeltme Planı

### Faz A — "Bot ölmesin" (1–2 gün, LIVE'dan bağımsız, en yüksek getiri)
1. **P0-2**: GATE1 interval gap → `Reject` yerine **resync + WARN**; `halted`'ı yalnız gerçek veri bozulmasına indir; `POST /actions {"action":"RESET_KERNEL"}` ekle.
2. **P0-3**: `RuntimeSnapshot`'a tam kernel durumu (`position`, `prev_bar_ts`, `pending`, `filled_qty_this_bar`); restore'da tutarlılık kontrolü.
3. **P0-6**: `Closing` için zaman aşımı + retry; `PlaceFailed` → faz rollback + yeniden deneme kuyruğu.
4. **L8/P1-16**: `filled_levels` / `next_level_to_place` ayrımı + seviye başına tek emir; testte **fiyat** doğrulaması.
5. **P0-11 / P1-19**: config aralık sınırları, gerçek enstrümanla merdiven doğrulaması.

### Faz B — "Muhasebe yalan söylemesin" (2–3 gün)
6. **P1-1/P1-2/P1-3**: `realized_net` (deal-bazlı fee + funding), net win-rate, `profit_factor`, dürüst Sharpe etiketi.
7. **P1-5**: deal-bazlı funding; kümülatif ayrı alan.
8. **P1-6**: gerçek funding geçmişi + `fundingIntervalHour` metadata.
9. **P1-9/P1-12/P1-13/P1-14/P1-15**: kapılarda fail-open temizliği, ölü GATE8 dalının kaldırılması, deterministik zaman.
10. **P1-11**: M03 stres teminat kapısını gerçekten bağla (veya "yok" diye dokümandan çıkar).

### Faz C — "LIVE'a hazırlık" (3–5 gün)
11. **P0-1**: `position_idx` pozisyon yönünden; hedge-mode entegrasyon testi.
12. **P0-4/P0-5**: dolum kimliği = `execId`, `pending_fills` kuyruğu, kısmi dolum takibi, **tek** dolum yolu.
13. **P0-7**: `ADD_FUNDS`'ı tek `submit_order` yoluna bağla (şalter + kapılar + yön + TP yenileme).
14. **P0-8**: webhook'u auth'tan çıkar, rate-limit ekle.
15. **P0-9**: tarayıcıdan motora doğrudan erişimi kaldır, sunucu tarafı proxy + httpOnly oturum.
16. **P0-10**: WFE negatif fold cezası, bileşik getiri, ortak maliyet tabanı, `cert_acknowledged`'ı süreli/signed yap.
17. **P1-8**: rezervasyon defteri (GATE6 available = cash − reserved).
18. **P2-4/P2-5**: Argon2id + izin periyodik taraması.

### Faz D — Ürün/UX ve operasyon (paralel)
19. **UIX-1/UIX-2** (SSE olay listesi + dataVer), **UIX-3** (Pine TP/SL 100×), **UIX-4** (birim modeli), **UIX-5** (grid CLOSE_ALL), **UIX-6** (stale göstergesi).
20. **P2-14** CI (cargo test + tsc + eslint + e2e smoke), **P2-16** çapraz platform başlatıcı, **P2-17** doküman-kod senkronu, **P2-18** arşiv temizliği.

### Faz E — Mimari (orta vade)
21. Emir başına FSM + tek `OrderRouter` (tüm yollar oradan geçer).
22. Event-sourced ledger: `fills`, `funding_settlements`, `fee_ledger` ayrı, deal başına mutabakat; günlük "venue ↔ local" reconciliation raporu.
23. Adaptör sözleşmesine **`reconcile()`** (venue open orders + positions + executions → tek snapshot) ve "drift" alarmı.
24. Çoklu süreç/çoklu bot ölçeklenmesi: SQLite yerine Postgres, bot başına actor.

---

## 8. Doğrulama Önerileri (bu ortamda yapılamayanlar)

Aşağıdakiler çalıştırma gerektirir; rapor bunları **iddia etmiyor**, öneriyor:

```bash
# 1) Çekirdek testler (mevcut iddia: 271 test)
cd mini-services/dca-core && cargo test --workspace --offline --locked

# 2) Frontend kalite kapıları
npx tsc --noEmit --incremental false
npx eslint src --quiet

# 3) Hedefli regresyon testleri (bu raporun bulguları için YAZILMASI GEREKEN)
#    - gate1_gap_resync_does_not_halt
#    - pause_resume_preserves_kernel_cursor
#    - snapshot_roundtrip_restores_position
#    - safety_topup_never_places_duplicate_price
#    - hedge_mode_close_uses_position_direction_idx
#    - market_partial_fill_remaining_is_tracked
#    - cert_wfe_penalizes_negative_is_folds
#    - grid_fee_breaker_uses_min_step

# 4) Uçtan uca (E2E_API_KEY gerekir)
E2E_API_KEY=$DCA_API_KEY python3 tests/e2e_engine.py
```

**Ek öneri:** `proptest`/`quickcheck` ile `Fraction` ve `build_dca_plan` için özellik testleri (monotonluk, tick hizası, toplam notional), ayrıca "fake venue" (kaydedilmiş klines + sahte adapter) ile deterministik entegrasyon testi.

---

## 9. Sonuç

Bu proje **"iyi bir matematik çekirdeği + eksik bir yürütme çekirdeği"** durumunda. Matematik ve istatistik katmanı düzeltme değil, **incelik** istiyor; asıl iş yürütme/durum makinesi katmanında: *her emir bir sonlu durum makinesi olmalı, her dolum borsanın kimliğiyle muhasebeye girmeli, her kapanış yolu sonlu olmalı, her şalter geri alınabilir olmalı.*

Faz A tamamlandığında sistem **güvenilir bir PAPER/backtest laboratuvarı** olur (bugün bile yakın). Faz B+C tamamlandığında **küçük sermayeyle testnet → canlı** rampası savunulabilir hale gelir. Bugünkü haliyle LIVE açmak, kaybın *miktarını* değil, kaybın *nereden geldiğini* bilmenizi bile engelleyecek bir muhasebe sapması riski taşıyor.
