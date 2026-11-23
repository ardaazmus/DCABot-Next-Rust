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
- LIVE bot sahipliği, normalize edilmiş venue/sembol çifti için kurulum, START, UNFREEZE ve RUNNING yaması sırasında seri biçimde denetleniyor; aynı piyasa için ikinci LIVE bot reddediliyor. Eski veride çakışan aktif LIVE botlar yeniden başlatmada FROZEN yapılıyor ve hesap-geneli pozisyon bu botlara atanmak yerine uzlaştırma atlanıyor. Silme, bot poll kilidini bekleyip durumu önce STOPPED yapıyor; LIVE silme ayrıca ana ve hedge pozisyonlarını, açık emir listesini ve iptal sonuçlarını doğruluyor. Doğrulama/iptal başarısızsa bot kayıtları korunuyor ve yeni risk üretemiyor.
- Engine SIGINT/SIGTERM'de yeni poll planlamasını durduruyor, süren poll/order cascade'lerini 45 saniyeye kadar drenaj ediyor, son runtime snapshot'larını yazıyor ve en son WAL checkpoint deniyor. Docker stop süresi 90 saniyeye çıkarıldı.
- Pine ADX artık Wilder DI ve DX tohumlarını doğru warmup boyunca taşıyor; ilk ADX `2×len−1` indeksinde beliriyor. Sıfır true-range piyasası sıfır ADX üretip durumu sıfırlamıyor. Supertrend son bantlarını önceki bar kapanışıyla taşıyor. Sinyal EMA'sında taşan ham toplamla SMA tohumu yerine taşmaya dayanıklı ağırlıklı ortalama kullanılıyor.
- Docker sırları data volume'dan ayrı korumalı volume'a taşındı; API, kasa, session ve UI sırları için `*_FILE` desteği ve eski `.api_key`/`.vault_key` geçişi eklendi. Üretilen sırlar loglanmıyor. Session secret kalıcı üretiliyor; test container'larında Binance spot/futures testnet varsayılanı açık. Giriş betiği POSIX `/bin/sh` ile uyumlu sinyal/çıkış yönetimi kullanıyor.
- Windows yerel süreç kayıtları ortak bir doğrulayıcıyla normalize ediliyor; eski iç içe JSON kayıtları PID/start-time/name alanlarına ayrılıyor. Durdurma yalnız kaydı ve gerçek executable/Next `start-server.js` komutunu doğrulayan PID'lere uygulanıyor. UI yeniden başlatma ve durdurma artık Next'in çalışan gerçek entrypoint'ini tanıyor.

### UI/UX ve veri gösterimi

- Dashboard SHORT maruziyetini hesaplanan `shortPct` ile gösteriyor.
- İşlem ilerleme metni LONG/SHORT yönüne göre fiyat ve safety hareketini anlatıyor; likidasyon mesafesini `<15%` durumunda “güvenli” diye etiketlemiyor.
- Hesaplayıcının SL seviyeleri ve likidasyon termometresi SHORT yönünü de hesaba katıyor.
- Silme/kapatma ve kasa anahtarı onay pencereleri onaydan sonra takılı kalmıyor.
- SSE `KILL_SWITCH`/`FREEZE` olayları veri yenilemeyi tetikliyor. Sunucu akışı `CLOSED` durumuna geçerse istemci sınırlı artan bekleme ile yeniden bağlanıyor.
- Doğrudan `?view=` yüklemesinde sunucu/istemci başlangıç görünümü eşleştirildi; Backtest'te eski piyasa verisinin yeni kategoriyle gösterilmesi engellendi.

### 23 Kasım takip denetimi: LIVE uzlaştırma ve sahiplik

- LIVE futures pozisyonları restart'ta, START/UNFREEZE öncesinde ve çalışan poll'larda (ayrıca açık yerel maruziyeti olan durmuş botlarda) kalıcı ana/hedge defteriyle karşılaştırılıyor. Boş ve durdurulmuş botlar gereksiz private API çağrısı yapmıyor. Yön, miktar ve ortalama giriş fiyatı (en fazla bir enstrüman tick'i fark) eşleşmezse borsa pozisyonu yerel bota mal edilmiyor; bot `FROZEN` oluyor ve yeni risk kesiliyor. Pozisyon endpoint'i hata verirse o poll'da yeni risk açılmıyor.
- Aynı sembolde bota ait olmayan açık venue emirleri restore/START/UNFREEZE aşamasında sahiplenilmiyor; bot donduruluyor veya başlatma reddediliyor.
- Pozisyonun venue'da kaybolması artık tahmini fiyatla sentetik `LIQ` fill üretmiyor. Doğrulanmış execution/close kanıtı yoksa yerel defter korunup bot manuel uzlaştırma için donduruluyor.
- Fill uzlaştırma kilidi yalnız henüz yerel deftere işlenmemiş execution/cumulative miktarı için devrede; daha önce kaydedilmiş kısmi fill açık emir bitene kadar stratejiyi durdurmuyor.
- LIVE fill sorgusu ve venue-flat iptal sorguları kısa runtime snapshot kilidi bırakıldıktan sonra, strateji uygulama kilidi alınmadan önce yürütülüyor; bu nedenle eski rapordaki “REST çağrısı global runtime kilidi altında” bulgusu güncel kodda kapanmıştır.
- `/api/live/reconcile` miktarı tam kesirle ve ortalama fiyatı bir tick toleransıyla karşılaştırıyor; SHORT yerel `Short` yönü venue `Sell` yönüne eşleniyor.
- Bu, mutlak hesap sahipliği kanıtı değildir: net pozisyon endpoint'i aynı yön/miktar/fiyatla oluşmuş manuel pozisyonu ayırt edemez. Spot envanteri de mevcut adapter sözleşmesinde bot bazında ölçülmüyor. Güvenli LIVE kullanımı için bot başına ayrılmış alt hesap/alt hesap benzeri izolasyon ve spot envanter mutabakatı hâlâ gereklidir. Pozisyon kontrolü çalışan LIVE futures botları için poll başına private API çağrısı ekler; bu yük borsa rate limit planına dahil edilmelidir.

## Doğrulama

- `cargo test --locked --offline --workspace --quiet`: geçti (361 test).
- `npx tsc --noEmit`: geçti.
- `npm run test:auth`: geçti (8 test).
- Değiştirilen lint temiz UI dosyalarında hedefli ESLint: geçti.
- Güncel motor ve UI yerelde PAPER kipinde çalışıyor: `http://127.0.0.1:3000/` HTTP 200, `http://127.0.0.1:3002/api/health` `ok`, API bot listesi 18 botun tamamını `PAPER` gösteriyor; 3000/3002 portları dinliyor.
- İzole Docker image/container denemeleri geçti: Compose config ve Docker build, shell syntax, health/login, `*_FILE` sırları, eski sır geçişi, sırların loglanmaması ve graceful stop kontrol edildi. Test container'ları ve yalnız bu teste ait volume'lar temizlendi; proje volume'larına dokunulmadı.
- Son kodla yerel engine yeniden derlenip aynı PAPER DB ile başlatıldı: UI HTTP 200, engine health `ok`, 18 botun tamamı `PAPER` ve RUNNING. Yeniden başlatma öncesi DB `VACUUM INTO` ile tutarlı anlık görüntü olarak yedeklendi; UI süreci çalışır kaldı.
- 23 Kasım takip yamasında `cargo check --manifest-path mini-services/dca-core/Cargo.toml -p dca_server` ve `npx tsc --noEmit` başarılıdır. Bu takip yamasında test takımları çalıştırılmadı ve gerçek borsa hesabı/emri kullanılmadı.
- Güncel yamayla `npm run local:start` başarıyla tamamlandı: UI `http://127.0.0.1:3000/` HTTP 200, engine `http://127.0.0.1:3002/api/health` `ok`; API 18 botun tamamını `PAPER / RUNNING` gösterdi. Etkin venue ortamları Bybit Demo, Binance Testnet, OKX Demo ve iç PAPER; hiçbir LIVE emir gönderilmedi.
- `npm run lint`: hâlâ 23 `react-hooks/set-state-in-effect` hatası veriyor. Bunlar Calculator, Config Tab, Forensics, Kill Switch, Settings, Signal Builder, Studio, TV Chart, Wizard, Carousel ve `use-mobile` dosyalarında; tümüne tek tek state/effect tasarım işi gerekiyor.
- Tam Rust workspace, gerçek borsa anahtarları veya LIVE emirleriyle denenmedi. Yerel kontrol PAPER modundadır; bu LIVE/mainnet işlem onayı değildir.

## Açık kalan öncelikli işler

1. **P1 — Hesap-geneli pozisyonun kesin sahipliği:** Mismatch durumunda artık fail-closed koruma var; ancak borsa net pozisyonu aynı yön/miktar/fiyatla oluşmuş manuel pozisyondan ayıramaz. Spot envanteri için de bot bazlı sahiplik sözleşmesi yok. Ayrılmış hesap/alt hesap ve spot mutabakatı gerekir.
2. **P1 — Backtest/PAPER/LIVE parity:** Kaynak rapordaki hedge funding ve funding imleci, hedge likidasyonu, grid bootstrap referansı, bar içi işlem sırası, gerçek MMR kademeleri ve birleşik hedge equity/fee eşlemesi ayrı tasarım ve parity testleri gerektiriyor.
3. **P2 — UI lint ve mimari:** 23 `react-hooks/set-state-in-effect` hatası (Calculator, Config Tab, Forensics, Kill Switch, Settings, Signal Builder, Studio, TV Chart, Wizard, Carousel ve `use-mobile`), tek-route ekran yapısı, büyük `wizard.tsx`/`studio.tsx`, kullanılmayan npm bağımlılıkları, erişilebilirlik ve mobil drawer yol haritası sürüyor.
4. **P2 — Daha geniş dosya/işlem güvenliği:** SQLite writer actor/yedek, limit emir fiyat-adım hassasiyeti, sayaç/harita büyüme sınırları ve gerçek borsa sandbox kabul testleri eklenmeli.

Özel borsa hesabı endpoint'lerine veya ana ağa işlem isteği gönderilmedi. PAPER motoru kamuya açık piyasa verisi çekebilir; hiçbir canlı emir gönderilmedi. Docker kontrolleri izole sahte sırlarla yapıldı; kullanıcı volume'ları korunarak test volume'ları silindi. Bu doküman denetim notlarının tümünü tamamlanmış saymaz; yukarıdaki maddeler ayrı geliştirme işi olarak açık bırakılmıştır.
