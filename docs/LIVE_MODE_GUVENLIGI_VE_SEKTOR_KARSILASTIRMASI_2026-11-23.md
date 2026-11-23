# Canlı Mod Güvenliği, Doğrulama Planı ve Sektör Karşılaştırması

**İnceleme tarihi:** 23 Kasım 2026
**Kapsam:** Bu depodaki DCA botunun emir yolu, PAPER/LIVE ayrımı, API anahtarı güvenliği, matematik ve fill mutabakatı, operatör akışı ve arayüzü.
**Sınır:** Bu rapor yatırım tavsiyesi, performans garantisi, mevzuat uygunluk belgesi veya OWASP/NIST sertifikası değildir. Rakip ürün özellikleri kendi ürün dokümanlarından özetlenmiştir; bu özellikler performans ya da güvenlik başarısının bağımsız kanıtı sayılmaz.

## Yönetici özeti

Bu incelemede LIVE akışına ait aşağıdaki korumalar uygulandı:

- LIVE bot oluşturma artık `PAUSED` durumunda biter. İlk gerçek/sandbox borsa emri için ayrı `START` gerekir.
- `PATCH status=RUNNING` ile LIVE başlatma yolu kapatıldı. Fleet ve Studio, LIVE başlatmayı `START`/`UNFREEZE` eylemine yönlendiriyor ve kullanıcıya açık onay gösteriyor.
- Sertifika eşiğini kullanıcı onayıyla geçme yolu kaldırıldı. LIVE sertifikası geçmeyen bot oluşturulmaz. İstemcinin gönderdiği “kaç deneme yaptım” değeri sertifika hesabında kullanılmıyor.
- Sertifika sonucu `live_certified` alanında saklanıyor. Eski DB satırları varsayılan olarak sertifikasız sayılıyor; eski LIVE botların yeniden başlatılması yeniden sertifikalandırma gerektiriyor.
- Her LIVE `START` ve `UNFREEZE` işleminde etkin adapter ortamı, ana ağ kilidi, API izinleri, çekim/transfer yasağı, gereken spot/futures kapsamı, ana ağ IP kısıtı, yeni self-check ve kill/freeze durumu yeniden kontrol ediliyor. Self-check en fazla 15 dakika geçerli.
- LIVE `ADD_FUNDS` artık RUNNING durumu, sertifika, güncel kontroller, minimum emir notional’ı ve kullanılabilir cüzdan bakiyesi kapısından geçiyor.
- Restart’tan sonra daha önce RUNNING olan LIVE botlar otomatik çalışmıyor; `PAUSED` açılıyor ve operatörün yeni START onayı gerekiyor. Anahtar eksikse bot LIVE kimliğini koruyor; yanlışlıkla PAPER diye etiketlenmiyor.
- LIVE strateji/risk parametresi yaması sertifikayı geçersiz kılabileceğinden kapalı. Değişiklik ayrı bot olarak yeniden sertifikalandırılmalı.
- Fleet, Dashboard ve Studio rozetleri PAPER, borsa Demo/Testnet ve Mainnet’i ayırıyor. PAPER ile LIVE aynı sayımda görünmüyor; etkin ortam `executionEnvironment` alanında taşınıyor.
- Bot sihirbazı üç ayrı ortam sunuyor: iç PAPER simülasyonu, borsa Demo/Testnet ve Mainnet LIVE. Kasa anahtarları tam endpoint adına göre (ör. Bybit Demo ile Bybit Testnet) etiketleniyor; bot ortamı, kasa anahtarı ve etkin endpoint birebir eşleşmeden oluşturma/başlatma reddediliyor.
- Botun seçilmiş tam endpoint ortamı DB’ye kaydediliyor. Endpoint sonradan değişirse mevcut bot otomatik olarak başka ortama yönlenmiyor; eski, ortamı bilinmeyen LIVE botlar PAUSED ve yeniden oluşturulmadan başlatılamıyor.
- Bybit’in resmi Türkiye ana makinesi `api.bybit.tr` ve dokümanda listelenen diğer bölgesel ana makineler güvenli endpoint listesine eklendi. Sandbox sınıflandırması tam host eşleşmesi kullanıyor.

Bu düzeltmeler canlı kullanım için gerekli bazı emniyet boşluklarını kapatır; tek başına üretim-canlıya hazır olduğunu kanıtlamaz. Özellikle gerçek exchange demo/testnet lifecycle, kesinti/yeniden başlatma senaryoları ve sınırlı canary henüz bu çalışma kapsamında canlı borsada çalıştırılmadı.

## Üç ayrı çalışma ortamı

`PAPER`, `DEMO/TESTNET` ve `MAINNET LIVE` aynı şey değildir:

| Ortam | Emir ve bakiye | Anahtar | Uygulamadaki karşılığı |
|---|---|---|---|
| **PAPER** | Uygulamanın yerel simülatörü ve sanal bakiye. Borsaya emir göndermez. | Gerekmez | `mode=PAPER`, `executionEnvironment=INTERNAL_PAPER` |
| **Borsa Demo/Testnet** | Gerçek piyasa verisi veya borsanın sandbox kurallarıyla borsanın ayrı sanal hesabına emir gönderir. Mainnet fonu kullanılmaz; API/işlem özellikleri mainnet’ten farklı olabilir. | Demo/Testnet hesabına ait ayrı anahtar | Emir işleme yolu `LIVE`; filo ve detay rozetinde `BYBIT DEMO`, `BYBIT TESTNET` veya `OKX DEMO` gösterilir. |
| **Mainnet LIVE** | Gerçek borsa hesabına emir; gerçek fon ve zarar riski | Mainnet anahtarı | `mode=LIVE`, `executionEnvironment=MAINNET`; `DCA_ALLOW_MAINNET=1` ve yeni güvenlik kapıları gerekir. |

Uygulamada bir PAPER botunu aynı bot kimliğiyle LIVE’a çeviren tek tık yoktur. Bu, pozisyon defteri, açık emirler, ortalama maliyet ve fill geçmişini iki farklı yürütme ortamı arasında karıştırmamak için doğru emniyet sınırıdır. Yeni ortama geçişte yeni bot oluşturulmalı; mevcut gerçek pozisyon ve açık emirler operatör tarafından ayrı uzlaştırılmalıdır.

Bybit dokümanı Demo Trading’in Testnet’ten ayrı bir hesap/anahtar kullandığını, `api-demo.bybit.com` alan adına bağlanılması gerektiğini, bazı API’lerin demo ortamında bulunmadığını ve demo emirlerinin yedi gün saklandığını söylüyor. Bu nedenle sadece “paper’da çalıştı” sonucu borsa demo veya mainnet doğrulaması sayılmaz. [Bybit Demo Trading](https://bybit-exchange.github.io/docs/v5/demo) [Bybit entegrasyon ve bölgesel endpoint rehberi](https://bybit-exchange.github.io/docs/v5/guide)

## Rakip ürünlerle özellik karşılaştırması

| Ürün | Paper/demo ayrımı | Backtest ve strateji denemesi | Emir güvenliği / operatör deneyimi |
|---|---|---|---|
| **3Commas** | Ayrı Demo Trading hesabı/API’si ve terminalde DEMO seçimi var; seçilen borsadan piyasa verisi kullanıyor. | Bu karşılaştırmada incelenen resmi sayfalar ağırlıklı olarak Demo ve terminal akışını belgeliyor. | API rehberi yalnız gerekli trade izinlerini açmayı ve withdrawal kapatmayı öneriyor. Demo’dan live’a geçişte live borsa API’si bağlanıyor. [Demo rehberi](https://help.3commas.io/en/articles/16281106-demo-trading-guide) [Terminal](https://help.3commas.io/en/articles/16281054-trading-terminal-overview) [API anahtarları](https://help.3commas.io/en/articles/16281186-create-and-connect-api-keys) |
| **Bitsgap** | Hesap düzeyinde Demo anahtarı, sanal fon ve canlı araçlara benzer bot/analitik akışı anlatılıyor; live API ayrı bağlanıyor. | Resmi Backtest rehberi geçmiş dönem ve kullanılabiliyorsa kullanıcının maker/taker ücretleriyle sonuç üretildiğini; sonucun referans amaçlı olduğunu ve slippage/yuvarlama nedeniyle canlıdan ayrılabileceğini belirtiyor. | Demo ile Live arasında görünür hesap geçişi var. [Demo Mode](https://bitsgap.com/helpdesk/article/13512068818332-How-to-use-Bitsgap-Demo-Mode) [Backtest](https://bitsgap.com/en/helpdesk/article/10023850035612-Backtest-bot-efficiency-analysis) |
| **Cryptohopper** | Exchange API anahtarı olmadan Paper Trading ve ayrı sanal bakiye sunuyor; botu paper’a alma veya yeni paper bot oluşturma belgelenmiş. | Backtester tarih aralığı/ücret ayarıyla sunuluyor; destek dokümanında beta olarak anılıyor. | Ürün dokümanı Paper ve gerçek fon akışlarının tarama sıklığı ile işlem hacmi/fill koşullarının aynı olmayabileceğini açıkça belirtiyor. [Paper Trading](https://support.cryptohopper.com/en/articles/9010197-how-do-i-enable-paper-trading) [Paper ve gerçek fon farkları](https://support.cryptohopper.com/en/articles/9173649-are-there-differences-between-paper-trading-and-real-funds) [Backtest](https://support.cryptohopper.com/en/articles/9013196-can-i-backtest-my-trading-bot) |
| **Bu repo (DCABot)** | PAPER iç simülasyonu, borsa Demo/Testnet ve Mainnet LIVE artık sihirbazda ayrı seçimler. Sandbox ve Mainnet kasa anahtarları ortam etiketiyle ayrı seçilir; PAPER anahtar istemez. | Backtest/Lab ve matematiksel strateji sertifikası var. Sertifika kârlılık garantisi değildir. | Endpoint/anahtar ortamı eşleşmesi server-side denetlenir; tam ortam botla kalıcı saklanır, değişmiş endpoint eski botu başlatmaz. Testnet/Demo ve Mainnet aynı venue için süreç başına tek etkin adapter ayarını paylaşır; farklı ortamda çalışan botlar aynı anda desteklenmiyor. |

### Bu uygulama için ürün yönü

Kullanıcı akışı artık **İç PAPER simülasyonu → Borsa Demo/Testnet → Mainnet LIVE** olarak seçim ve rozetlerde ayrılıyor. İç API’de emir motoru için `PAPER/LIVE` geriye dönük uyumlu kalıyor; etkin tam endpoint ayrı `execution_environment` alanında saklanıyor. Aynı venue için Demo/Testnet ile Mainnet’i eşzamanlı çalıştırma hâlâ desteklenmiyor; endpoint motor yapılandırmasıyla seçiliyor ve değişiklik mevcut botları güvenlik nedeniyle bloke ediyor.

## Canlı moda geçiş için doğrulama kapıları

### 1. Yerel matematik ve strateji sözleşmesi

- Tüm finansal hesapları ondalık/rasyonel türlerle yap; `f64` yuvarlamasının emir miktarı, ücret ve PnL’yi değiştirmediğini kanıtla.
- Alım/satım yönü, short/long WAC, futures kaldıraç ve marj, funding, komisyon, slippage, tick-size, qty-step, min-notional, TP/SL ve trailing için sınır ve property testleri çalıştır.
- Aynı fill/order kimliğinin ikinci kez uygulanmadığını; kısmi fill, iptal edilmiş emir, terminal durum ve geç gelen fill’in doğru muhasebeleştiğini test et.
- Backtest, PAPER ve LIVE adapter fill defterlerinin aynı olay için tutarlı olduğunu kanıtla; PAPER raporunda sanal fon ve fill modelini görünür tut.
- Sertifika için tek pencereye aşırı güvenme: ayrı eğitim/doğrulama/test pencereleri, walk-forward, farklı volatilite rejimleri, ücret/slippage duyarlılığı ve başarısız stratejiler raporu ekle.

### 2. API anahtarı ve endpoint

- Başlangıçta salt okunur izin sorgusu yap; ana ağda trade için yalnız ihtiyaç duyulan spot ya da contract/futures yetkisini iste.
- Withdrawal, dahili transfer ve benzeri fon taşıma izinlerini kapalı tut. Bu repo mevcut Bybit parser’ında `AccountTransfer` izinlerini de withdrawal hard gate’ine dahil ediyor.
- Mainnet anahtarını sabit güvenilir IP’lere bağla. Bybit’in `query-api` cevabı `readOnly`, `Spot`, `ContractTrade`, `Wallet` ve IP listesi gibi alanları sağlar; anahtar kapsamını runtime’da tekrar oku. [Bybit API key info](https://bybit-exchange.github.io/docs/v5/user/apikey-info)
- Binance’in key güvenlik rehberi trade gibi okuma dışı izinler için uygun IP kısıtlamasını tavsiye ediyor; bu repo Binance LIVE’ı hâlâ kapalı tutuyor. [Binance API key rehberi](https://www.binance.com/en/support/faq/detail/360002502072)
- Borsa endpoint’lerini HTTPS ve tam host allowlist’iyle sınırla; serbest kullanıcı URL’si/redirect ile credential sızması veya SSRF açma. Bölgesel endpoint listeleri borsanın güncel resmi dokümanından yönetilmeli.
- Tick, miktar adımı, minimum tutar ve piyasa durumu gibi enstrüman meta verisini canlı emir öncesi güncel al; veri yoksa emir üretme. Bybit sembol kuralları tick-size, qty-step ve minimum tutarı borsa metadata’sından veriyor. [Bybit Instruments Info](https://bybit-exchange.github.io/docs/v5/market/instrument)

### 3. Başlatma ve risk kapıları

- Bot oluşturma gerçek emir başlatmamalı. Bot PAUSED oluşmalı, operatör ayrı ve açık bir START onayı vermeli.
- Her START/UNFREEZE öncesi key geçerli, doğru borsa hesabına ait, doğru ürün kapsamlı ve çekim yetkisiz olmalı; mainnet ise IP allowlist’i zorunlu olmalı.
- Self-check’in PASS sonucu kısa TTL ile geçerli olmalı. Eski, başarısız, eksik veya gelecekte zaman damgalı check ile LIVE açılmamalı.
- Kill-switch/freeze, veri bayatlığı, saat farkı, sembol durumu, min qty/notional, kullanılabilir cüzdan, diğer botların ayrılmış bakiyesi, pozisyon modu ve portföy limiti birlikte değerlendirilmelidir.
- PATCH, webhook, toplu devam, restart, manuel fon ekleme ve restore gibi alternatif yollar aynı güvenlik kapısının dışından emir gönderememeli.
- Restart sonrası LIVE otomatik re-arm olmamalı. Açık order/fill/position uzlaştırması tamamlanana kadar yeni risk kapalı kalmalı.

### 4. Testnet/Demo lifecycle ve arıza enjeksiyonu

İlk gerçek borsa entegrasyon testlerini yalnız Bybit Demo/Testnet veya OKX Demo gibi ayrı sanal ortamda, mainnet anahtarından ayrı anahtarla çalıştır:

1. Emir öncesi key scope ve endpoint eşlemesi; yanlış ortam anahtarıyla fail-closed davranış.
2. Limit/market emri açma, kısmi fill, tam fill, reject, cancel, cancel-replace ve idempotent retry.
3. Ağ yanıtı kaybolduktan sonra order status, execution ve positions ile reconcile; “timeout oldu, o halde fill olmadı” varsayımı yapma.
4. Bot pause/stop, kill/freeze, süreç restart’ı ve DB restore sırasında açık emir/pozisyon sahipliğini doğrula.
5. Saat kayması, 429/rate limit, geçersiz imza, stale market data, eksik fee/funding/instrument cevabı ve borsa maintenance durumlarını enjekte et.
6. Kaldıraç, margin mode, pozisyon yönü/`positionIdx`, multi-bot aynı sembol ve diğer botların açık emir yükü çakışmasını doğrula.
7. Dashboard/Fleet’te PAPER, Demo/Testnet ve Mainnet etiketlerini ve emir sayısı/pozisyon/PnL kaynağını operatörün ayırt edebildiğini kontrol et.

### 5. Sınırlı mainnet canary

Canary yalnız önceki katmanların tamamı geçtikten sonra, operatörün onayıyla yapılır. Tek borsa, tek sembol, tek bot, küçük önceden belirlenmiş sermaye/emir tavanı, maksimum günlük kayıp, alarm ve anında kill-switch belirlenir. Başlangıçta kaldıraç ve paralel bot sayısı sınırlanır; trade-only, withdrawal’sız ve IP kısıtlı anahtar kullanılır. Emir ve fill defteri her gün borsanın order/execution/position bakiyesiyle mutabık olmalıdır. Uyumsuzluk, stale veri, beklenmeyen drawdown veya anahtar/endpoint uyuşmazlığı yeni risk açmayı otomatik kesmelidir.

**Not:** Evrensel bir “7 gün” ya da “100 fill” standardı yoktur. Ekip içi geçiş politikası olarak örneğin en az 7 gün kesintisiz demo soak ve 100 farklı fill/reconcile senaryosu belirlenebilir; bu öneri bir mevzuat veya borsa şartı değil, ölçülebilir kurum içi kabul eşiğidir. Kağıt/backtest sonucu gerçek emir güvenliğinin yerine geçmez.

## Güvenlik standartları ve bu projeye uygulanması

- **OWASP ASVS 5.0.0:** Web/API erişim kontrolü, kimlik doğrulama, input validation, dosya/secret yönetimi, TLS, logging ve hata yönetimi için doğrulanabilir kontrol listesi. Bu projede “ASVS uyumlu” iddiası yok; deploy öncesi seçilmiş ASVS gereksinimleri için kanıt toplanmalı. [OWASP ASVS](https://asvs.owasp.org/) [5.0.0 indirmeleri](https://asvs.owasp.org/downloads/)
- **NIST SSDF SP 800-218:** Güvenli gereksinim, tasarım incelemesi, bileşen yönetimi, güvenli build/test, vulnerability triage ve düzeltme süreçlerinin SDLC’ye eklenmesini önerir. [NIST SP 800-218](https://csrc.nist.gov/pubs/sp/800/218/final)
- **Least privilege ve exchange key güvenliği:** Trade için gereken izinler dışında yetki verme; withdrawal/transfer yetkisini kapalı tut; IP whitelist ve key rotation uygula; PAPER, Demo/Testnet ve Mainnet key’lerini ayır.
- **Kimlik doğrulama ve dağıtım sınırı:** Yerel loopback/proxy mimarisi dış ağa açılmadan önce güçlü kimlik doğrulama, CSRF ve origin denetimi, rate limit, audit log, secret store, backup/restore ve izleme doğrulanmalı. Yerel geliştirme varsayımı internete açık deployment için yeterli değildir.
- **İşlem güvenliği:** Emir gönderimi idempotent olmalı; borsadan gelen execution birincil fill kanıtı olmalı; timeout sonrası durum query/reconcile edilmeli; order/position sahipliği belirsizse yeni emir durmalı.

## Bu çalışmada uygulanan düzeltmeler

| Alan | Düzeltme |
|---|---|
| Başlatma | LIVE create → PAUSED; explicit START/UNFREEZE, Fleet ve Studio’da çift aşamalı kullanıcı onayı |
| Ortam seçimi | Sihirbazda iç PAPER, borsa Demo/Testnet ve Mainnet LIVE ayrı; seçilen tam endpoint `bots.execution_environment` içinde saklanır |
| Kasa | API anahtarları tam endpoint etiketiyle (Bybit Demo/Testnet, Binance Testnet, OKX Demo, Mainnet) ayrılır; botun seçimi, etkin endpoint ve anahtar etiketi birebir uyuşmalıdır; genel SANDBOX etiketli eski anahtarlar yeniden doğrulanır |
| Sertifika | Client ile sertifika başarısızlığını aşma ve istemci deneme sayısı kaldırıldı; yalnız geçerli sunucu sertifikasıyla START |
| Key/endpoint | Etkin adapter hedefini kullan; çekim/transfer yok, gerekli market trade scope’u var; mainnet key’i IP kısıtlı |
| Self-check | 15 dakikalık tazelik penceresi; eski/başarısız check ile başlatma reddedilir |
| Restart | Önceden RUNNING LIVE bot PAUSED restore edilir; credential yoksa LIVE kimliği korunur |
| Pozisyon/emir sahipliği | Futures pozisyon yönü, miktarı ve ortalama giriş fiyatı restore, START/UNFREEZE ve her poll'da yerel defterle karşılaştırılır; farkta FROZEN/fail-closed. Aynı sembolde yabancı açık emir sahiplenilmez. Doğrulanmamış venue-flat durumunda sentetik LIQ fill üretilmez. |
| Alternatif emir yolu | LIVE ADD_FUNDS, bot durumu, sertifika, self-check, key ve ortak wallet/min-notional kapısını kullanır |
| Canlı config | TP/SL/trailing patch sertifika kapısını geçersiz kılabileceğinden LIVE’da reddedilir |
| UI | Dashboard, Studio ve Fleet PAPER, Demo/Testnet ve Mainnet’i ayrı etiketler/filtreler; bot oluşturma üç seçenek sunar |
| Bölgesel endpoint | Bybit resmi bölgesel host listesi eklendi; Türkiye için `api.bybit.tr` destekleniyor |

## Doğrulama sonucu ve açık işler

- Ortam ayrımı değişikliğinde `cargo check --manifest-path mini-services/dca-core/Cargo.toml -p dca_server`: **başarılı**.
- Ortam ayrımı değişikliğinde `npx tsc --noEmit`: **başarılı**.
- Son exact-endpoint değişikliği `cargo build --manifest-path mini-services/dca-core/Cargo.toml -p dca_server` ile derlendi ve yerel motor yeniden başlatıldı: `127.0.0.1:3002/api/health` **ok**, etkin ortamlar Bybit Demo, Binance Testnet, OKX Demo ve iç PAPER; UI `127.0.0.1:3000` **HTTP 200**.
- Yerel veritabanında 18 botun tamamı `PAPER / RUNNING`; vault anahtarı yok. Bu doğrulamada gerçek veya borsa Demo/Testnet emri gönderilmedi.
- Bu son değişiklikte Rust/TypeScript test takımları yeniden çalıştırılmadı; önceki hardening revizyonunda çalıştırılmış sonuçlar bu patch için geçerli test kanıtı sayılmaz.
- 23 Kasım pozisyon/emir sahipliği takip yamasında `cargo check --manifest-path mini-services/dca-core/Cargo.toml -p dca_server` ve `npx tsc --noEmit` geçti; bu takip yaması için test takımları veya borsa emirleri çalıştırılmadı.
- `npm audit --omit=dev`: **0 bilinen production dependency açığı** (npm advisory verisine göre, tarama tarihi 23.11.2026).
- Gerçek borsa emirleri, Demo/Testnet emirleri veya mevcut bot verisi üzerinde işlem testi yapılmadı. Finansal etkili testi ancak ayrı test hesabında ve ayrıca planlanmış canary sırasında çalıştırın.
- Rust için bu çalışmada otomatik dependency advisory taraması yok. Kod içinde review ve testler yapıldı; managed Codex Security Deep Scan bu Windows çalışma alanında gerekli yönetilen filesystem izin profilini alamadığı için başlamadı. Bu sonuç “bulgu yok” anlamına gelmez.
- `live_certified=0` göç varsayılanı nedeniyle daha önce oluşturulmuş LIVE botlar yeniden START edemez. Eski sertifika olayları geçmişte kullanıcı istisnasıyla da `GECTI` kaydı üretebildiğinden güvenli backfill yapılmadı; yeniden sertifikalandırılmış yeni bot oluşturulmalı.
- Eski vault anahtarları ortam bilinmediği için `UNVERIFIED` olarak göç eder. Önceki sürümün genel `SANDBOX` etiketli anahtarları da endpoint ayrımını kanıtlayamadığından `UNVERIFIED` durumuna alınır. Ayarlar ekranında motorun etkin endpoint’inde yeniden doğrulanınca tam endpoint adına atanır. Endpoint ortamı bir venue için uygulama konfigürasyonundan geldiğinden aynı süreçte o venue’nün Demo/Testnet ve Mainnet botları birlikte koşturulamaz.
- 500 barlık sertifika tek başına istatistiksel yeterlilik değildir. Walk-forward/OOS doğrulaması, testnet soak, kesinti ve restart fault injection, alarm/notification, gerçek bakiye mutabakatı ve operasyon/runbook kanıtı tamamlanmadan sistem üretim LIVE hazır sayılmamalı.
- Pozisyon karşılaştırması hesap-geneli snapshot'ı kullanır; netting hesapta aynı yön/miktar/ortalama fiyata sahip manuel pozisyon ile bot pozisyonunu kesin ayıramaz. Spot coin envanteri mevcut `PositionSnapshot` sözleşmesinde yoktur. LIVE için botlara ayrılmış hesap/alt hesap, spot bakiye mutabakatı ve borsa sandbox kabul akışı gerekir. Pozisyon endpoint'i her LIVE futures poll'unda çağrıldığı için private API rate limit/çoklu bot ölçeği ayrıca sınanmalıdır.
- Bybit bölgesel REST endpoint listesi eklendi; farklı region hesaplarında key, demo/testnet erişimi ve websocket adresleri ayrıca doğrulanmalı.
- Daybreak Markets kaydı yalnız bilgi bağlamında tutulur; salt-okunur araştırma ürünü olduğu için emir/anahtar entegrasyonu veya DCA yürütme ortamı olarak kullanılmaz.

## Kaynaklar

- [OWASP Application Security Verification Standard (ASVS)](https://asvs.owasp.org/)
- [NIST SP 800-218, Secure Software Development Framework](https://csrc.nist.gov/pubs/sp/800/218/final)
- [Bybit V5 Integration Guidance](https://bybit-exchange.github.io/docs/v5/guide)
- [Bybit Demo Trading Service](https://bybit-exchange.github.io/docs/v5/demo)
- [Bybit API Key Information](https://bybit-exchange.github.io/docs/v5/user/apikey-info)
- [Bybit Instruments Info](https://bybit-exchange.github.io/docs/v5/market/instrument)
- [Binance API Key Security](https://www.binance.com/en/support/faq/detail/360002502072)
- [3Commas Demo Trading Guide](https://help.3commas.io/en/articles/16281106-demo-trading-guide), [Trading Terminal](https://help.3commas.io/en/articles/16281054-trading-terminal-overview), [API Key Guide](https://help.3commas.io/en/articles/16281186-create-and-connect-api-keys)
- [Bitsgap Demo Mode](https://bitsgap.com/helpdesk/article/13512068818332-How-to-use-Bitsgap-Demo-Mode), [Backtest](https://bitsgap.com/en/helpdesk/article/10023850035612-Backtest-bot-efficiency-analysis)
- [Cryptohopper Paper Trading](https://support.cryptohopper.com/en/articles/9010197-how-do-i-enable-paper-trading), [Paper vs Real Funds](https://support.cryptohopper.com/en/articles/9173649-are-there-differences-between-paper-trading-and-real-funds), [Backtesting](https://support.cryptohopper.com/en/articles/9013196-can-i-backtest-my-trading-bot)
- [Daybreak Markets](https://daybreak.markets/) — kullanıcı kaydı bilgi bağlamında; bu entegrasyon alım satım hattına dahil değildir.
