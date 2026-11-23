# Yeniden denetim raporu 2 — uygulama durumu

**Tarih:** 23 Kasım 2026

**Rapor:** `E:\Downloads\dcabot_next_rust_dogrulama_ve_yeniden_denetim_raporu-2.md`

**Raporun incelediği HEAD:** `18083ca`

**Bu çalışma sırasında doğrulanan dal HEAD'i:** `a23a59b` (`codex/apply-audit-2026-11-22`)

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

## Açık kalan işler

1. **Canlı hesapta kabul / parity:** Backtest–Paper–Demo/Testnet senaryoları, özel WS akışı, gecikme/bağlantı kesintisi ve uzun süreli uzlaştırma testi ayrı demo/testnet hesapları gerektiriyor. Bu turda gerçek hesap kullanılmadı.
2. **Kesin LIVE pozisyon sahipliği:** Net hesap-geneli venue pozisyonu aynı yöndeki manuel pozisyondan güvenilir biçimde ayırt edilemez. Bot başına alt hesap veya venue tarafından desteklenen ayrı pozisyon sahipliği sözleşmesi gerekir; mevcut davranış belirsizlikte fail-closed.
3. **UI/erişilebilirlik:** Bileşen mimarisini bölme, renk token'larına geçiş ve gerçek mobil cihaz/ekran okuyucu/kontrast kabul turu sürüyor.
4. **DB iş modeli:** Senkron SQLite yazma modelini async writer actor'a taşıma ve iş/olay saklama süresini kararlaştırma ayrı kapsamlı işler.

Bu durum belgesi canlıya çıkış onayı değildir. İlgili daha geniş geçmiş ve önceki doğrulamalar için [ana denetim uygulama günlüğüne](denetim-uygulama-2026-11-22.md) bakın.
