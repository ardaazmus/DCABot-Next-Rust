# 🔍 KAPSAMLI DENETİM RAPORU (2026-11-11)
**Proje:** DCABot-Next-Rust (dal: `arena/e76eaee8-dcabot-next-rust`)
**Denetim Kapsamı:** Kod, Finans Matematiği, UI/UX, Mantık Denetimi
**Kritik Uyarı:** Canlı para/gerçek borsa anahtarları kullanılmamıştır. README LIVE engelleri gözetilmiştir.

## 1. STATİK BULGULAR VE RİSKLER (P0-P2)

### 1.1 P0 SAFETY EMİRLERİ (Kritik)
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_strategy/src/dca.rs:575-584,725-755,800-815,939-965`
- **Etki:** TP/trailing/manuel kapanışta aktif SAFETY’ler iptal edilmiyor; sonradan dolarak pozisyonu yeniden açabilir. SL’de `:688` wildcard iptali var.
- **Öneri:** `on_fill` ve `force_liquidate` çağrı zincirine SAFETY iptal mantığı eklenmeli. Sunucu `mini-services/dca-core/crates/dca_server/src/engine.rs:1467-1496` prefix iptalini işliyor, stratejiyle senkronize edilmeli.
- **Test Sınırı:** İzole workspace'te 224 birim test geçti; ancak SAFETY iptal senaryosu `dca_strategy` testlerinde yeniden üretildi (`tp_close_does_not_cancel_safety_and_old_fill_reopens_in_cooldown`).

### 1.2 P0 OKX EMİR YÖNÜ
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_exchange/src/okx.rs:532-559`
- **Etki:** `posSide` reduce-only/pozisyon bacağı yerine emir yönünden seçiliyor. Long kapatan Sell `short`, short kapatan Buy `long` olur.
- **Öneri:** `posSide` mantığı `reduce-only` ve pozisyon bacağı bazında yeniden yapılandırılmalı.
- **Test Sınırı:** İzole workspace'te `dca_exchange` derlenmedi; statik analizle tespit edildi.

### 1.3 P0 MAINNET KİLİDİ
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_server/src/main.rs:1669-1681`, `mini-services/dca-core/crates/dca_exchange/src/okx.rs:26,37-40,65-86`
- **Etki:** `DCA_BYBIT_TESTNET=1` ile OKX dahil tüm venue’lerde mainnet kapısı geçilebilir; varsayılan gerçek OKX, demo başlığı yok.
- **Öneri:** `DCA_BYBIT_TESTNET` değişkeni sadece Bybit ile sınırlanmalı; OKX ve diğer venue’ler için bağımsız `DCA_VENUE_TESTNET` kontrolü eklenmeli.
- **Test Sınırı:** Statik analiz.

### 1.4 LOOK-AHEAD (Zamanlama)
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_strategy/src/backtest.rs:59-103,327-334`, `mini-services/dca-core/crates/dca_signals/src/engine.rs:237-240,292-301`, `mini-services/dca-core/crates/dca_server/src/engine.rs:1878-1919,2022-2026`
- **Etki:** 4h bucket başlangıcına yazılır; 00:00 kararında 03:00 içeren mum kullanılabilir. Son sinyal geride kalan tüm barlara uygulanıyor.
- **Öneri:** `resample` fonksiyonu ve `build_backtest_signal` mantığı sıfır look-ahead garantisiyle (sadece kapalı barlar) revize edilmeli.
- **Test Sınırı:** `lookahead_four_hour_signal_changes_when_future_bars_are_appended` testi ile yeniden üretildi (geçti/doğrulandı).

### 1.5 HEDGE DAVRANIŞI
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_server/src/state.rs:833-869,227-239`, `mini-services/dca-core/crates/dca_strategy/src/backtest.rs:618-621,733-740`, `mini-services/dca-core/crates/dca_strategy/src/hedge.rs:148-168,219-255`
- **Etki:** İki stepper’a aynı başlangıç parası verilip equity toplanıyor (1.000 pozisyonsuz sermaye ≈ 2.000 görünür). Combined exit brüt PnL, fee/funding hariç.
- **Öneri:** Hedge bacağı equity/DD/fee hesaplamaları ana bacakla tutarlı hale getirilmeli. `hedge.rs` dual hedge `MainOnly` başlayıp combined exit `HedgeActive` gerektirmesi doğrulandı.
- **Test Sınırı:** `hedge_dual_fractional_base_fails_to_open_hedge_leg` testi ile yeniden üretildi (geçti/doğrulandı).

### 1.6 LİKİDASYON ÇİFT SAYIM
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_strategy/src/backtest.rs:400-406`, `mini-services/dca-core/crates/dca_server/src/engine.rs:600-652`, `mini-services/dca-core/crates/dca_strategy/src/dca.rs:939-965,994-1003`
- **Etki:** LIQ fill’e `on_fill` ardından `force_liquidate` çağırır; sayaç/gross iki kez artabilir.
- **Öneri:** `force_liquidate` çağrısı `on_fill` sonrası `LIQ` dolumlarında bastırılmalı.
- **Test Sınırı:** `liquidated_close_then_force_liquidate_double_counts` testi ile yeniden üretildi (geçti/doğrulandı).

### 1.7 ÜCRET VE NET TP
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_strategy/src/dca.rs:884-898,489-496,952-963,209-230`, `mini-services/dca-core/crates/dca_kernel/src/lib.rs:824-841`
- **Etki:** Giriş fee’si sıfırlanabilir ve deal neti/TP’de dışlanır. 100 alış, %0,1 giriş/çıkış, %1 net hedef: 101,11 satışta net 0,90889. Kernel `realized_net` yalnız kapanış ücretini düşer.
- **Öneri:** Giriş fee’si `realized_net` ve TP hesaplamasına dahil edilmeli.
- **Test Sınırı:** `net_tp_omits_entry_fee` ve `base_fee_is_erased_and_win_classification_can_flip` testleri ile yeniden üretildi (geçti/doğrulandı).

### 1.8 HESAPLAYICI / MESH
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_server/src/routes_calc.rs:63,90-128`, `mini-services/dca-core/crates/dca_strategy/src/grid.rs:84-98`, `mini-services/dca-core/crates/dca_math/src/m15_m20.rs:28-58,134-177`
- **Etki:** Yön hep Long, liquidation WAC yerine anchor kullanır (örnek 85,929648 yerine 90,954774). Grid fee-breaker maker×maker/ticklenmemiş fiyat. 100–101 geometrik mesh `[100,100,101]` olabilir; base–ilk safety SL çakışması taranmaz.
- **Öneri:** `routes_calc.rs` yön parametresi eklenmeli; `m15_m20.rs` mesh oluşturma ve SL çakışma taraması düzeltilmeli.
- **Test Sınırı:** `geometric_grid_integer_anchors_create_duplicate_levels` ve `m20_does_not_test_stop_before_first_safety` testleri ile yeniden üretildi (geçti/doğrulandı).

### 1.9 PORTFÖY
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_server/src/fleet.rs:434-466`, `mini-services/dca-core/crates/dca_server/src/engine.rs:1254-1281,1357-1393`
- **Etki:** NAV başlangıç nakitleri toplamı; gross hedge ve bekleyen emir rezervlerini dışlar. Adaylar aynı statik cache’e karşı bağımsız ölçülür; toplam limit aşılabilir. Reddedilen market BASE/cache yokluğu `Ok(())` ile düşerek `BasePending`’de kalabilir.
- **Öneri:** NAV hesaplamasına gross hedge ve rezervler dahil edilmeli; cache kontrolü atomik hale getirilmeli.
- **Test Sınırı:** Statik analiz.

### 1.10 PATCH / SİLME
- **Dosya/Satır:** `mini-services/dca-core/crates/dca_server/src/main.rs:1907-1917`, `mini-services/dca-core/crates/dca_server/src/state.rs:270`, `mini-services/dca-core/crates/dca_server/src/main.rs:1953-2003`
- **Etki:** Engine patch sonrası DB’ye eski `rt.cfg` yazılır; snapshot da eski. LIVE delete iptal hatalarını yutabilir ve açık pozisyonu kapatmadan runtime/DB kaldırabilir.
- **Öneri:** Patch sonrası `rt.cfg` güncellemesi ve LIVE delete hata yönetimi revize edilmeli.
- **Test Sınırı:** Statik analiz.

## 2. FİNANS MATEMATİĞİ
- **Fraction::Display vs from_decimal_str:** `Fraction::Display` kesirli değerleri `n/d` olarak yazarken `from_decimal_str` yalnız ondalık kabul ediyor. Çağrı zincirleri (özellikle `mini-services/dca-core/crates/dca_math/src/instrument.rs` ve `dca_strategy/src/dca.rs`) incelendi.
- **Test Sınırı:** `fraction_display_vs_from_decimal_str` testi ile yeniden üretildi (başarısız/doğrulandı).
- **Gap Batch:** `mini-services/dca-core/crates/dca_kernel/src/lib.rs:824-841` `filled_qty_this_bar`’ın tekrar sayılması ipucu doğrulandı.
- **Test Sınırı:** `gap_batch_counts_previous_filled_twice` testi ile yeniden üretildi (geçti/doğrulandı).

## 3. UI/UX DENETİMİ (Playwright & Next.js 16)
- **Tarayıcı:** Masaüstü (1440x900) ve Mobil (375x812) ekranlar.
- **Locale:** `tr-TR`
- **Sonuç:** Konsol veya sayfa hatası yok.
- **Bulgular:**
  - **1x Hedge Dual:** Spot piyasa seçiliyor (`GET /api/engine/api/markets?category=spot&venue=BYBIT`), calc `market:"spot"`. Short `linear` ama calc POST yön alanı içermiyor (`mini-services/dca-core/crates/dca_server/src/routes_calc.rs` ve `src/components/terminal/wizard.tsx:233-237,392-410`).
  - **Mobil Footer:** Taşma `overflow-x-auto` içindedir; sayfa-geneli taşma olarak sunulmamaktadır (`src/app/page.tsx` footer ve mobil nav).
  - **Studio:** Short/hedge `kind.includes("FUTURES")` ile spot sınıflandırabilir (`src/components/terminal/studio.tsx:334-359`).
- **API Mock:** `/tmp/dca-ui-audit/audit.mjs` ile test edildi; sonuç `/tmp/dca-ui-audit/ui-result.json` (hata yok).
- **Kimlik Denetimi:** `src/app/api/auth/login/route.ts`, `src/app/api/auth/check/route.ts`, `src/app/api/auth/logout/route.ts`, `src/proxy.ts`, `src/lib/auth-session.ts` ve `src/components/terminal/auth-dialog.tsx` incelendi. `DCA_UI_PASSWORD` veya `DCA_REQUIRE_AUTH` aktifse oturumsuz istekler 401. SSE akışı kimlik doğrulaması (`mini-services/dca-core/crates/dca_server/src/main.rs:276-304`) ve Next.js proxy (`src/proxy.ts:70-79`) tutarlı.

## 4. DAĞITIM VE GÜVENLİK
- **Docker:** `docker-compose.yml:11-22` UI auth varsayılan kapalı/sabit API-vault anahtarları.
- **Proxy:** `src/proxy.ts:48-78` auth isteğe bağlı.
- **Vault Anahtarı:** Oturum HMAC sırrı (`src/lib/auth-session.ts:5-6,15-52`).
- **SSE:** `src/app/api/stream/route.ts:27-50` UI auth kontrolü olmadan SSE açar (Next.js proxy üzerinden motor kontrolü ile korunur).
- **Restart:** `scripts/docker-entrypoint.sh:10` rastgele vault anahtarı restart sonrası eski kasayı çözemeyebilir.
- **Öneri:** Vault anahtarı kalıcı depolama/env ile sabitlenmeli; SSE route `src/app/api/stream/route.ts` Next.js proxy ile tutarlı auth kontrolü eklemeli.

## 5. TEST SONUÇLARI (İZOLE WORKSPACE)
- **Crate Testleri:** `dca_kernel` (29), `dca_math` (56), `dca_pine` (33), `dca_risk` (10), `dca_signals` (35), `dca_strategy` (61) — Toplam **224 geçti, 0 başarısız**.
- **Not:** Vendor sürümleri farklı, `chrono` manifesti sadeleştirildi, lint’ler bastırıldı. `dca_server` ve `dca_exchange` derlenmedi.
- **Regresyon Testleri:**
  - `lookahead_four_hour_signal_changes_when_future_bars_are_appended`: Geçti (Doğrulandı)
  - `base_fee_is_erased_and_win_classification_can_flip`: Geçti (Doğrulandı)
  - `net_tp_omits_entry_fee`: Geçti (Doğrulandı)
  - `tp_close_does_not_cancel_safety_and_old_fill_reopens_in_cooldown`: Geçti (Doğrulandı)
  - `liquidated_close_then_force_liquidate_double_counts`: Geçti (Doğrulandı)
  - `geometric_grid_integer_anchors_create_duplicate_levels`: Geçti (Doğrulandı)
  - `m20_does_not_test_stop_before_first_safety`: Geçti (Doğrulandı)
  - `hedge_dual_fractional_base_fails_to_open_hedge_leg`: Geçti (Doğrulandı)
  - `gap_batch_counts_previous_filled_twice`: Geçti (Doğrulandı)
  - `fraction_display_vs_from_decimal_str`: Başarısız (Doğrulandı - `1/2` ondalık değil)

## 6. SONUÇ
Proje; finans matematiği, UI/UX ve mantık açısından kapsamlı denetlenmiştir. P0 seviyesindeki SAFETY, OKX yönü, mainnet kilidi, look-ahead, hedge, likidasyon çift sayımı, ücret/net TP, hesaplayıcı/mesh, portföy, patch/silme bulguları önceliklendirilmiştir. Next.js derleme, TypeScript, lint ve npm audit başarılıdır. Playwright testi gerçek şemaya uygun haber fixture’ı ve `locale: "tr-TR"` ile tekrarlanmış; hata tespit edilmemiştir.

**İşlem Durumu:** Next.js sunucusu (PID 2315) ve sahte motor (PID 8008) durdurulmuştur. Denetim tamamlanmıştır.

---
*Bu rapor, gerçek anahtar/borsa/emir kullanmadan, statik analiz ve izole test ortamı sonuçlarına dayanarak hazırlanmıştır.*
