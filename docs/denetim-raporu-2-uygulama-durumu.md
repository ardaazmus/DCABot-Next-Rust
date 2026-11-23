# Yeniden denetim raporu 2 — uygulama durumu

**Tarih:** 23 Kasım 2026

**Rapor:** `E:\Downloads\dcabot_next_rust_dogrulama_ve_yeniden_denetim_raporu-2.md`

**Raporun incelediği HEAD:** `18083ca`

**İlk takip doğrulamasındaki dal HEAD'i:** `a23a59b` (`codex/apply-audit-2026-11-22`)

Rapor daha eski bir commit'i denetliyor. Bu nedenle maddeler rapordaki eski satır durumuna göre değil, mevcut dalın koduna göre tekrar kontrol edildi.

## Kritik ve yüksek öncelikli regresyonlar

| Rapor bulgusu | Güncel durum | Kanıt / not |
|---|---|---|
| Docker'da `DCA_SESSION_SECRET` yokluğu nedeniyle login 503 | **Düzeltilmiş** | `scripts/docker-entrypoint.sh` kalıcı session secret üretimini ve `DCA_SESSION_SECRET_FILE` desteğini içeriyor; `docker-compose.yml` file tabanlı aktarımı destekliyor. |
| Pozisyon yokken SL yamasının 422 vermesi ve config'i kirletmesi | **Düzeltilmiş** | DCA yaması önce önerilen değerleri doğruluyor, açık pozisyon yoksa SL fiyatı üretmiyor; reddedilen yama eski state'i koruyor. |
| `deps` yol metninin mainnet kapısını atlatması | **Düzeltilmiş** | Venue ana ağ istisnası çalışma dosyası yoluna göre değil, test derleme kapsamına göre sınırlandırılmış. |
| Runtime mutex tutulurken sıralı cancel/fill HTTP çağrıları | **Düzeltilmiş** | Ağ istekleri runtime kilidi bırakıldıktan sonra çalışıyor; yerel sonuçlar daha sonra state'e uygulanıyor. |

## Orta ve düşük öncelikli maddeler

Raporun §2.2 bölümündeki bulgular da güncel kaynakta kontrol edildi. Binance emir durumları `open_orders` ve tek emir sorgusunda ortak enum'a normalize ediliyor; anahtar silme bekleyen engine action ve LIVE intent kuyruklarını temizliyor; OKX client ID uzunluk sınırı, kısmi terminal fill ve kontrat değeri ele alınıyor; fee yedeği, credentials temizliği, flat-bot UI matematiği, SSE yeniden bağlanması ve ilgili form doğrulamaları sonraki commit'lerde sertleştirildi. Hedge-only venue düzleşmesinde engine, hedge muhasebesine sahte likidasyon yazmak yerine uyuşmazlıkta fail-closed davranıyor.

Rapordaki **Bybit execution geçmişinin tek `limit=100` sayfasında kesilebilmesi** ise güncel dalda halen gerçek bir açıklıktı ve bu değişiklikte kapatıldı:

- `nextPageCursor` son sayfaya kadar izleniyor; tekrar eden, çok uzun veya 20 sayfayı aşan cursor'da kısmi dolum geçmişi başarı gibi döndürülmüyor.
- Cursor, sorgu parametresi olarak güvenli biçimde kodlanıyor. Aynı kodlama mevcut enstrüman sayfalamasında da kullanılıyor.
- Execution listesi biçimi, kimlik, miktar ve fiyat hatalıysa sessizce sıfır fill üretmek yerine hata veriyor; eşleşmeyen `orderLinkId` kayıtları dışarıda kalıyor.
- Sayfa boyutu Bybit'in `1–100` sınırına göre 100. API sözleşmesi: [Bybit Get Trade History](https://bybit-exchange.github.io/docs/v5/order/execution).

## Bu turdaki doğrulama

- `cargo check --locked --offline --manifest-path mini-services/dca-core/Cargo.toml -p dca_exchange -p dca_server` geçti.
- Değiştirilen `bybit.rs` için `rustfmt --edition 2021 --check` geçti.
- Yerel UI `http://127.0.0.1:3000` çalışıyor; engine `http://127.0.0.1:3002/api/health` ve DB health `ok`.
- Yerel bot listesindeki 18 botun tamamı `PAPER / RUNNING`.
- Son cursor-kodlama değişikliği sonrası tam Rust test workspace'i çalıştırılmadı. Gerçek borsa hesabı, özel endpoint veya emir kullanılmadı.

## 23 Kasım ek takip: muhasebe tekilleştirme ve oturum kapatma

`2ff94d1` tabanından sonra yerel PAPER panelinde kapalı işlem listesinde olay akışı ile kalıcı `deals` tablosunun aynı kapanışları iki kez gösterdiği doğrulandı. Tekrarlanan kayıtlar kâr/zarar toplamlarını ve işlem istatistiklerini şişirebiliyordu. Güncel kodda:

- Yeni kapanış olayları kararlı `deal_no` ve gerçek `closed_ts` taşıyor; kalıcı defter yazımı aynı bot ve işlem numarası için tek SQL ifadesiyle idempotent hale getirildi.
- Eski kayıtlarda kimlik bulunmadığında yalnız bot, brüt PnL, komisyon, funding ve net PnL eşleşmesi karşılıklı olarak tekilse ve zaman farkı altı saati aşmıyorsa bir kayıt tutuluyor. Eşleşme belirsizse işlem gizlenmiyor.
- Yerel `GET /api/engine/api/deals` toplamı 155 çift kaynaklı satırdan 142 tekil kapanışa indi; 13 kopya özet dışı kaldı.
- `POST /api/auth/logout` için same-origin JSON/Fetch Metadata kapısı eklendi. Daha önce route bulunmasına rağmen panelde oturum kapatma eylemi yoktu; kimlik doğrulama zorunluysa artık başlıkta erişilebilir bir **Çıkış** düğmesi var. Başarılı çıkış kök sayfayı yeniden yükleyerek bellekteki ekran durumunu da temizliyor.
- Yerel entegrasyon kontrolü, Next'in `localhost` ve `127.0.0.1` adlarını normalize edebildiğini gösterdi. Kaynak kontrolü bu nedenle gerçek gelen `Host` değerini de karşılaştırıyor; aynı-origin loopback isteği `200`, cross-origin isteği `403` döndü.

Bu takipte doğrulama:

- `npm run test:auth`: 11 test geçti; logout kaynak kontrolleri ve red durumları test edildi.
- Yerel logout route kontrolü: same-origin POST `200`, cross-origin POST `403`.
- `npm test`: API sözleşme testlerinin 27'si geçti.
- `npx tsc --noEmit` ve `npm run lint` geçti.
- `cargo test --locked --offline -p dca_server -p dca_strategy`: sunucu 75, strateji 70 + 10 yeniden üretim + 5 finans testi geçti.
- `cargo test --locked --offline --workspace` geçti.
- Yerel UI `http://127.0.0.1:3000` ve engine `http://127.0.0.1:3002/api/health` yanıt verdi; `health=ok`, DB `ok`. 18/18 bot `PAPER / RUNNING`; kapalı işlem API'si 142 kayıt döndürdü.
- Yerel arayüz gözle kontrol edildi. Bu doğrulama gerçek mobil cihaz/ekran okuyucu kabul testi değildir.
- Gerçek borsa özel endpoint'i çağrılmadı ve emir gönderilmedi.
- Codex Security Standard tarayıcısı 262 dosyalık envanterin keşif aşamasında 0/6 yüzeyde ilerledi ve rapor üretemedi; takılı kalan iki çalışma sonlandırıldı. Bu ek takip bulguları kaynak kodu incelemesi ve çalıştırılan testlere dayanır, tamamlanmış bağımsız tarama sertifikası değildir.

## Açık kalan işler

1. **Canlı hesapta kabul / parity:** Backtest–Paper–Demo/Testnet senaryoları, özel WS akışı, gecikme/bağlantı kesintisi ve uzun süreli uzlaştırma testi ayrı demo/testnet hesapları gerektiriyor. Bu turda gerçek hesap kullanılmadı.
2. **Kesin LIVE pozisyon sahipliği:** Net hesap-geneli venue pozisyonu aynı yöndeki manuel pozisyondan güvenilir biçimde ayırt edilemez. Bot başına alt hesap veya venue tarafından desteklenen ayrı pozisyon sahipliği sözleşmesi gerekir; mevcut davranış belirsizlikte fail-closed.
3. **UI/erişilebilirlik:** Bileşen mimarisini bölme, renk token'larına geçiş ve gerçek mobil cihaz/ekran okuyucu/kontrast kabul turu sürüyor.
4. **DB iş modeli:** Senkron SQLite yazma modelini async writer actor'a taşıma ve iş/olay saklama süresini kararlaştırma ayrı kapsamlı işler.

Bu durum belgesi canlıya çıkış onayı değildir. İlgili daha geniş geçmiş ve önceki doğrulamalar için [ana denetim uygulama günlüğüne](denetim-uygulama-2026-11-22.md) bakın.
