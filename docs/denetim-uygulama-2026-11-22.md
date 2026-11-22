# Denetim notları: uygulama ve kalan işler

**Tarih:** 22 Kasım 2026
**Kaynak rapor:** `E:\Downloads\dcabot_next_rust_kapsamli_denetim_ve_gelistirme_raporu.md`
**Repo:** `D:\project\yeni-dca-bot`

Bu belge kaynak rapordaki maddelerin repo üzerindeki uygulama durumunu kaydeder. Uygulama işi, dört fazlı yol haritasının tamamlandığı anlamına gelmez; canlı hesapta denenmeyen maddeler ayrıca belirtilmiştir.

## Uygulanan düzeltmeler

### Güvenlik ve yerel çalıştırma

- Oturum imzasındaki sabit yedek sır kaldırıldı; `DCA_SESSION_SECRET` yoksa oturum üretimi ve doğrulaması fail-closed çalışıyor. Üretim UI parolası zorunlu ve API/kasa anahtarından ayrı.
- Giriş denemeleri için bellek içi hız sınırı ve artan bekleme eklendi.
- Motor rate limiter'ı artık istemcinin verdiği `X-Forwarded-For` değerini kimlik kabul etmiyor; tablo üst sınırı korunuyor.
- Webhook doğrulama hataları geçersiz girdide DB'ye kontrolsüz olay yazmıyor; borsa anahtarının silinmesi çalışan runtime kimlik bilgisini de temizliyor.
- AI URL çıkışında özel/yerel hedefler, DNS rebinding, yönlendirme ve proxy kullanımı engellendi; Gemini anahtarı URL yerine başlıkta taşınıyor.
- Binance, Bybit ve OKX için izin verilen venue adresleri, demo/testnet varsayılanları ve ana ağ kapıları sıkılaştırıldı; anahtar izinleri eksik veya okunamazsa reddediliyor.
- Yerel başlatma, yeniden başlatma ve durdurma betikleri süreç sahipliğini doğruluyor; canlı ağa çıkmamak için PAPER ve demo/testnet ayarlarını koruyor.
- Next.js güvenlik aralığının dışındaki 16.4.5 sürümüne sabitlendi. Önceki onay kapsamında yapılan npm audit 0 açık bildirdi.

### Rust çekirdeği, emirler ve matematik

- Pine `and`/`or` öncelik/işleç eşlemesi düzeltildi; parser derinliği, token sayısı ve bozuk adlandırılmış argümanlar sınırlandı.
- LIVE flat-pozisyon algılaması fill uzlaştırmasından önce pozisyonu likidasyon diye kapatmıyor. Takip edilen emirler iptal/uzlaştırma turunda korunuyor; bakiye ve yerel pozisyon uyuşmazlığında bot donduruluyor. Sweep sonrası snapshot ve UI önbelleği yenileniyor.
- OKX swap kontratları `ctVal` ile coin miktarına çevriliyor; komisyon işareti, `posSide`, `size` ve geçersiz kontrat değeri fail-closed ele alınıyor. OKX'in 32 karakterlik alfanümerik `clOrdId` sınırına uyum için motor emir kimliği adaptör içinde geri çevrilebilir kodlanıyor; [OKX API kılavuzu](https://my.okx.com/docs-v5/en/) ile doğrulandı.
- Bybit execution kayıtları yerel `orderLinkId` filtresinden geçiyor. Binance REST emir durumları motorun ortak enum'una dönüştürülüyor.
- `PlaceFailed` sonucu, emrin kapanış olup olmadığını taşıyor; başarısız kapanış yeni giriş fazını geri almıyor.
- SpotGrid başlangıçta envantersiz satış emri üretmiyor. PAPER Pine ve sinyal stratejileri de Bybit'in halka açık piyasa verisini kullanıyor.
- VaR kesir ayrıştırması, yakın kademe nakit/marj rezervasyonu, M07 funding tamponu ve DCA safety sonrası SL/TP yenilemesi düzeltildi.
- Hedge leg yapılandırma hatası panic üretmiyor; birleşik kâr hesabı deal ücret/funding giderlerini düşüyor; sıfır cooldown'da kapanış yeniden kurulumdan önce tanınıyor ve tamamlanmış Protective hedge temizleniyor.
- Sertifikasyon trial'ları farklı uzunluktaysa ortak uzunluğa kırpılıyor.
- Yeniden başlatmada borsa hesap-geneli açık emir listesinden yalnız bot kimliğiyle başlayan emirler alınıyor. Bilinen BASE/SAFETY/TP/SL/TRAIL/MANUAL/GRID rollerinin `Add`/`Reduce` anlamı korunuyor. Botun kendi emir kimliği çözülemiyorsa bot FROZEN oluyor.
- Terminal dolum yedeğinde borsa toplam komisyonundan önceden işlenmiş komisyon çıkarılıyor; kalan dolum komisyonu ikinci kez yazılmıyor.

### UI/UX ve veri gösterimi

- Dashboard SHORT maruziyetini hesaplanan `shortPct` ile gösteriyor.
- İşlem ilerleme metni LONG/SHORT yönüne göre fiyat ve safety hareketini anlatıyor; likidasyon mesafesini `<15%` durumunda “güvenli” diye etiketlemiyor.
- Hesaplayıcının SL seviyeleri ve likidasyon termometresi SHORT yönünü de hesaba katıyor.
- Silme/kapatma ve kasa anahtarı onay pencereleri onaydan sonra takılı kalmıyor.
- SSE `KILL_SWITCH`/`FREEZE` olayları veri yenilemeyi tetikliyor. Sunucu akışı `CLOSED` durumuna geçerse istemci sınırlı artan bekleme ile yeniden bağlanıyor.
- Doğrudan `?view=` yüklemesinde sunucu/istemci başlangıç görünümü eşleştirildi; Backtest'te eski piyasa verisinin yeni kategoriyle gösterilmesi engellendi.

## Doğrulama

- `cargo test --locked --offline --workspace --quiet`: geçti (359 test).
- `npx tsc --noEmit`: geçti.
- `npm run test:auth`: geçti (8 test).
- Değiştirilen lint temiz UI dosyalarında hedefli ESLint: geçti.
- Güncel motor ve UI yerelde PAPER kipinde çalışıyor: `http://127.0.0.1:3000/` HTTP 200, `http://127.0.0.1:3002/api/health` `ok`, API bot listesi 18 botun tamamını `PAPER` gösteriyor; 3000/3002 portları dinliyor.
- `npm run lint`: hâlâ 23 `react-hooks/set-state-in-effect` hatası veriyor. Bunlar Calculator, Forensics, Kill Switch, Settings, Signal Builder, Studio, TV Chart, Wizard, Carousel ve `use-mobile` dosyalarında; tümüne tek tek state/effect tasarım işi gerekiyor.
- Tam Rust workspace, gerçek borsa anahtarları veya LIVE emirleriyle denenmedi. Yerel kontrol PAPER modundadır; bu LIVE/mainnet işlem onayı değildir.

## Açık kalan öncelikli işler

1. **P1 — Runtime kilidi altında borsa ağı:** `live_fill_sweep` ve bazı funding/ticker yolları `app.runtimes` kilidi açıkken ağ çağrısı yapıyor. Aynı venue'deki yavaş yanıt diğer bot ve API işlerini geciktirebilir. Durum kopyası + sürüm kontrollü yeniden uzlaştırma ile kilitsiz ağ fazı tasarlanmalı.
2. **P1 — Graceful shutdown:** Supervisor/engine kapanışında poll döngülerinin ve in-flight order cascade'lerinin drenajı, tüm snapshot'ların yazılması ve son WAL checkpoint'i sıralanmalı.
3. **P1 — Restore pozisyon sahipliği:** Açık emirler bot bazında filtrelendi; ancak borsa pozisyon endpoint'i aynı sembolde hesap toplamını döndürür. Birden çok LIVE bot aynı venue/sembolü paylaşınca pozisyonu hangi bota ait olduğu hâlâ güvenilir biçimde çıkarılamıyor; bu kombinasyon güvenli attribution kuralı eklenmeden LIVE'da kullanılmamalı.
4. **P1 — Backtest/PAPER/LIVE parity:** Kaynak rapordaki hedge funding, funding imleci, hedge likidasyonu, grid bootstrap referansı, bar içi işlem sırası, gerçek MMR kademeleri ve birleşik hedge equity/fee eşlemesi ayrı parity testi ve tasarım gerektiriyor.
5. **P1 — Pine gösterge doğruluğu:** EMA uzun pencere sayısal kararlılığı, `ta.stoch` argümanları, ADX warmup ve Supertrend önceki kapanış kullanımı bu değişiklik paketinde yeniden yazılmadı.
6. **P1 — Docker sırları:** Giriş betiği otomatik UI parolasını stdout'a yazıyor; API/vault anahtarlarını DB ile aynı data volume içinde saklayabiliyor. Docker volume/secrets geçişi mevcut kurulumların anahtarlarını koruyacak migrasyon gerektirdiğinden burada değiştirilmedi.
7. **P2 — UI lint ve mimari:** 23 effect lint hatası, tek-route ekran yapısı, büyük `wizard.tsx`/`studio.tsx`, kullanılmayan npm bağımlılıkları, erişilebilirlik ve mobil drawer yol haritası sürüyor.
8. **P2 — Daha geniş dosya/işlem güvenliği:** SQLite writer actor/yedek, limit emir fiyat-adım hassasiyeti, sayaç/harita büyüme sınırları ve gerçek borsa sandbox kabul testleri eklenmeli.

Özel borsa hesabı endpoint'lerine veya ana ağa işlem isteği gönderilmedi. PAPER motoru kamuya açık piyasa verisi çekebilir; hiçbir canlı emir gönderilmedi. Bu doküman denetim notlarının tümünü tamamlanmış saymaz; yukarıdaki P1 maddeleri ayrı uygulama işi olarak açık bırakılmıştır.
