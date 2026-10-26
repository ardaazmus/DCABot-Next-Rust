# DCA Bot — Kapsamlı İnceleme ve Çalışabilirlik Raporu

**Tarih:** 23 Ekim 2026, Europe/Istanbul  
**Proje:** `D:\project\yeni-dca-bot`  
**Kapsam:** Yerel başlatma, kritik yürütme yollarının statik incelemesi, güvenlik, borsa sözleşmeleri, muhasebe, kalıcılık, test ve işletim eksikleri.  
**Değişiklik sınırı:** Uygulama kaynak koduna düzeltme uygulanmadı. Bu rapor oluşturuldu; çalıştırma araçlarının ürettiği önbellekler, derleme çıktıları ve `.audit-runtime/` altındaki inceleme çıktıları ayrıca vardır.

## 1. Sonuç

### Yerel çalıştırma güncellemesi — 23 Ekim 2026

İlk denetimden sonra kullanıcının yerelde çalıştırma talebiyle başlatma düzeni güncellendi. Güncel Rust kaynağı `cargo build --locked --offline --bin dca_server` ile başarıyla derlendi. Motor ayrı `.local-runtime/paper.sqlite3` veritabanıyla başlatıldı. Next.js ana sayfası ve Next.js üzerinden `/api/health` HTTP 200 verdi; `/api/bots` boş liste döndürdü. Tarayıcıda `CORE v3.0.0`, `BYBIT BAĞLI` ve `STREAM LIVE` görüldü. Gerçek hesap veritabanı veya gerçek emir kullanılmadı.

Eklenen komutlar: `npm run local:start` ve `npm run local:stop`. Başlatıcı Windows gizli arka plan süreçleri kullanır; Cargo paralelliği, Node heap'i ve SWC iş parçacıkları sınırlanır. `DCA_LOCAL_RUN=1` halinde webpack disk önbelleği kapatılır ve derleme paralelliği azaltılır. İlk HTTP isteğine soğuk sayfa derlemesi için yeterli süre tanınır. Günlükler `.local-runtime/` altındadır. Durdurma komutu da denendi ve PAPER veritabanı korundu.

Makinedeki düşük kullanılabilir sanal bellek nedeniyle birkaç başlatma denemesi ve Rust workspace test derlemesi hata verdi. Son başlatma UI/API/tarayıcı kontrollerini geçti; tüm Rust testlerinin geçtiği iddia edilmiyor. Test denemesi `.audit-runtime/local-tests.log`, başarılı başlatma `.audit-runtime/launcher-final.log` içinde kayıtlıdır. Aşağıdaki özgün denetim bulguları, yerel başlatmanın çalışmasından bağımsız olarak geçerlidir; işlem ve muhasebe mantığına düzeltme uygulanmadı.

**Proje mevcut haliyle gerçek para yöneten bir bot olarak kullanıma hazır değil.** İlk denetimde güncel kaynak kodun derlenmesi ve uçtan uca HTTP akışı doğrulanamamıştı; sonraki yerel çalıştırmanın başarılı kontrolleri yukarıdaki güncellemede kayıtlıdır. Kaynakta bulunan ciddi yürütme ve muhasebe hataları ayrıca giderilmelidir.

Öncelikli engeller:

- Yönetim API'sinde kimlik doğrulama bulunmaması ve aşırı geniş CORS.
- Borsa sınırlarına uymayan emir kimlikleri ve spot market alışlarında miktar birimi hatası.
- İptal edilmiş bir emrin gerçekleşmiş sayılabilmesi; kısmi/gecikmeli dolumların eksik takibi.
- Yeniden başlatmada açık pozisyon ve emirlerin geri yüklenmemesi.
- Spot özkaynak hesabının alınan varlığın değerini içermemesi.
- Hedge bacaklarının tek kernel pozisyonunda birleştirilmesi.
- Yüklü Next.js sürümü dahil bağımlılıkların güvenlik danışma kayıtlarıyla eşleşmesi.

Bu rapordaki **kaynakta doğrulandı** ifadesi kod akışının incelendiğini belirtir; testnet/mainnet üzerinde sömürü veya gerçek emir denemesi yapıldığı anlamına gelmez. Önerilen kabul testleri yapılacak işlerdir; bu oturumda geçmiş testler olarak sunulmamıştır.

## 2. Önce yapılan yerel çalıştırma

### 2.1 Güvenli deneme düzeni

Motor kayıtlı RUNNING botları açılışta yeniden kurduğu için mevcut `db/dca-core.sqlite3` yerine `DCA_DB_PATH` ile `.audit-runtime/audit.sqlite3` kullanıldı. İnceleme veritabanına gerçek borsa anahtarı eklenmedi ve gerçek emir gönderilmedi. `.env` içeriği rapora veya terminal çıktısına basılmadı.

Başlatma sırasında kullanılan temel komutlar:

```powershell
node node_modules/next/dist/bin/next dev --hostname 127.0.0.1 --port 3000 --webpack

# Rust kaynak derlemesi, mini-services/dca-core içinde:
cargo run --offline --bin dca_server

# Temiz hedef diziniyle tekrar; mevcut uygulama DB'sinden ayrı:
$env:DCA_DB_PATH = 'D:/project/yeni-dca-bot/.audit-runtime/audit.sqlite3'
$env:CARGO_TARGET_DIR = 'D:/project/yeni-dca-bot/.audit-runtime/target'
$env:CARGO_INCREMENTAL = '0'
cargo run --offline --bin dca_server
```

Derleme engeli sonrası mevcut `mini-services/dca-core/target/debug/dca_server.exe` de aynı ayrı veritabanıyla başlatıldı. **Bu ikili güncel kaynak doğrulaması değildir:** ikilinin dosya zamanı 22 Ekim 2026 23:03:07; `main.rs` dosyasının zamanı 23:15:37 idi.

### 2.2 Gözlenen sonuçlar

| Kontrol | Sonuç | Yorum |
| --- | --- | --- |
| Next.js geliştirme sunucusu | `Ready in 6.1s`; localhost:3000 dinleyicisi gözlendi | Kurulu sürüm 16.1.3 |
| Rust güncel kaynak başlatma | Başarısız | `only metadata stub found for rlib dependency core`; temiz hedefte de tekrarlandı |
| Mevcut Rust ikilisi | 0.0.0.0:3002 dinleyicisi ve ayrı DB oluşumu gözlendi | Kaynaktan eski ikili; işlevsel doğrulama sayılmaz |
| UI ve `/api/health` HTTP istekleri | Bağlantı hatası / zaman aşımı | Proxy hariç tutulunca ve sandbox dışı denemede de yanıt doğrulanamadı |
| Uygulama içi tarayıcı | `ERR_CONNECTION_TIMED_OUT` | Görsel/etkileşimli kabul testi yapılamadı |
| TypeScript | `tsc --noEmit --incremental false`: çıkış 0 | Tip kontrolü geçti |
| ESLint | `eslint src --quiet`: son denemede çıkış 0 | İlk ek denemede ortam bellek hatası görüldü; tekrar başarılı |
| Rust testleri | `cargo +1.89.0 test --workspace --offline`: başarısız | Standart kitaplık/derleme ortamı hataları; test başarı sayısı üretilemedi |
| Bağımlılık taraması | Kullanıcı onayından sonra `bun audit --json` yanıt üretti; çıkış 1 | Güvenlik eşleşmeleri var; aşağıdaki bölümde ayrıştırıldı |
| Üretim build / gerçek borsa E2E | Yapılmadı | Başarılıymış gibi değerlendirilmedi |

Rust hatasının ortam/toolchain kaynaklı olabileceği güçlü bir olasılıktır; bu çıktı tek başına kaynakta derleme hatası bulunduğunu kanıtlamaz. Yerel portun dinlemesi de API'nin veya botun doğru çalıştığını kanıtlamaz. Deneme için açılan uzun ömürlü süreçlere oturum sonunda sonlandırma gönderildi.

İnceleme çıktıları: `.audit-runtime/rust-run.log`, `.audit-runtime/cargo-test.log`, `.audit-runtime/eslint.log`, `.audit-runtime/dependency-audit.json`. Audit dosyasının başında Bun'un `.env` yükleme bildirimi bulunur; JSON gövdesi bu satırdan ayrılarak okunmalıdır.

## 3. Güvenlik bulguları

### SEC-01 — Kritik: Yönetim API'si kimlik doğrulamıyor

**Kanıt:** `mini-services/dca-core/crates/dca_server/src/main.rs:123` router kurulumu; `main.rs:185` `CorsLayer::very_permissive()`; `main.rs:188` `0.0.0.0:3002` bind. Router üzerinde yönetim istekleri için oturum/token/yetki middleware'i yok. `Caddyfile:1` ayrıca HTTP proxy açıyor.

**Etki:** Servise ağ erişimi bulunan kişi bot oluşturabilir, durum değiştirebilir, kill-switch'i bırakabilir ve kasa kayıtlarını silebilir. CORS politikası başka origin'deki sitelerin de API'yi çağırmasını kolaylaştırır. Gerçek dış erişim firewall/proxy düzenine bağlıdır; mevcut oturumda dışarıdan saldırı gösterilmedi.

**Gerekli:** Yönetim kimlik doğrulaması, yetkilendirme, açık origin listesi, cookie kullanılıyorsa CSRF koruması, TLS ve varsayılan localhost bind. Webhook ayrı yetki kapsamına alınmalı.

**Kabul:** Kimliksiz yönetim isteği 401/403; izinsiz origin reddi; kasaya/botlara erişim yetkisi testleri.

### SEC-02 — Yüksek: Kasa anahtarı şifrelediği verilerle aynı DB'de

**Kanıt:** `mini-services/dca-core/crates/dca_server/src/store.rs:186` ve `store.rs:196`: `DCA_VAULT_KEY` yoksa anahtar `settings.vault_master_key` içine yazılıyor. `store.rs:232` verilen env metnini SHA-256 ile türetiyor; üstteki base64/hex/plain açıklamasıyla birebir aynı sözleşme değil.

**Etki:** Varsayılan kurulumda DB dosyasını ele geçiren kişi şifreli API anahtarlarını çözmek için gereken master key'i de elde eder. Env anahtarının değiştirilmesi için sürümlü anahtar/yeniden şifreleme akışı görünmüyor.

**Gerekli:** Canlı modda DB dışından zorunlu güçlü anahtar, açık encoding sözleşmesi, key-id ve rotasyon prosedürü, yedeklerin şifrelenmesi ve erişim izinleri.

**Kabul:** Sadece DB yedeğiyle sırlar çözülememeli; yanlış master key kontrollü hata üretmeli; rotasyon sonrası eski kayıtlar okunabilmeli.

### SEC-03 — Kritik: OKX çekim izni kontrolü gerçek kontrol değil

**Kanıt:** `mini-services/dca-core/crates/dca_exchange/src/okx.rs:477`: `key_permissions` kimlik bilgilerini kullanmadan sabit `can_withdraw: false` ve trade izinleri döndürüyor. `mini-services/dca-core/crates/dca_server/src/main.rs:2620` kasa kabulü bu sonuca güveniyor.

**Etki:** OKX anahtarının geçerliliği ve çekim yetkisi bu yoldan doğrulanmıyor; README'deki aşılamaz zero-withdrawal iddiası OKX için sağlanmıyor.

**Gerekli:** Sağlayıcının doğrulanabilir izin bilgisi; izinler belirlenemiyorsa canlı modun reddi. Kullanıcının trade-only anahtar oluşturması doğrulamanın yerine geçmemeli.

**Kabul:** Çekim yetkili, geçersiz ve izinleri bilinmeyen anahtarların hiçbiri LIVE yetkisi kazanamamalı.

### SEC-04 — Yüksek: Sayısal girdilerde kaynak tüketimi ve taşma riski

**Kanıt:** `mini-services/dca-core/crates/dca_math/src/fraction.rs:110`: üslü onluk değerler için `BigInt::pow` 4096-bit denetiminden önce çalışıyor. `i64` üsler `u32`'ye daraltılıyor. `mini-services/dca-core/crates/dca_math/src/m02.rs:91`: dış girdiden gelen `safety_count` doğrudan `Vec::with_capacity` için kullanılıyor; hesaplayıcı bunu üst sınır olmadan geçiriyor (`routes_calc.rs:75`).

**Etki:** Küçük bir JSON gövdesi büyük bellek/CPU tüketimine yol açabilir; uç değer üsler yanlış parse veya panic oluşturabilir. Zararlı büyük girdiler bu oturumda çalıştırılmadı.

**Gerekli:** Parse öncesi uzunluk/üs/seviye sınırları, checked conversion, sınırlı iş kuyruğu, istek ve eşzamanlılık limitleri. Body limit tek başına yeterli değil.

**Kabul:** Aşırı üs ve aşırı seviye sayıları pahalı hesaplamadan önce 422 ile reddedilmeli; servis kullanılabilir kalmalı.

### SEC-05 — Yüksek: Kasadan silinen anahtar çalışan botta kalıyor

**Kanıt:** `mini-services/dca-core/crates/dca_server/src/main.rs:2756` yalnız DB kaydını siliyor. `state.rs:29` runtime kimlik bilgilerini tutuyor; `engine.rs:555` ve `engine.rs:619` bu kopyayla emir gönderiyor.

**Etki:** Kullanıcı anahtarı kasadan kaldırınca çalışan botun yerel yetkisi kalkmıyor. Borsada ayrıca iptal edilmedikçe kullanılmaya devam edebilir.

**Gerekli:** Anahtar kimliğine bağlı runtime yetkisi; yerel silme/devre dışı bırakmada yeni emirlerin engellenmesi; açık emir/pozisyon yönetimi için açık politika.

**Kabul:** Anahtar kaldırıldıktan sonra bağlı bot yeni emir gönderememeli; arayüz kalan pozisyonu açıkça göstermeli.

### SEC-06 — Yüksek: Caddy hedef portu istemci girdisinden seçiliyor

**Kanıt:** `Caddyfile:3` `XTransformPort=*` eşleşmesi ve `Caddyfile:7` `localhost:{query.XTransformPort}` hedefi.

**Etki:** Bu Caddy yapılandırması erişime açılırsa istemci beklenen motor dışındaki yerel HTTP servislerine de proxy üzerinden ulaşmayı deneyebilir. Port seçimi için allowlist bulunmuyor. Caddy bu oturumda başlatılmadı; dağıtım koşuluna bağlı bulgudur.

**Gerekli:** Sabit `/api` upstream'i veya yalnız gereken portları kabul eden sunucu tarafı eşleme.

**Kabul:** Başka localhost portuna yönlendirme reddedilmeli; kullanıcı kontrollü query parametresi upstream belirlememeli.

## 4. Emir yürütme ve borsa uyumsuzlukları

### EX-01 — Kritik: Oluşturulan emir kimlikleri borsa sınırlarını aşıyor

**Kanıt:** `main.rs:554` bot kimliğini `b-` + 32 hex karakter olarak üretiyor: 34 karakter. `mini-services/dca-core/crates/dca_strategy/src/lib.rs:355` emir kimliğini `{bot}-{role}-{seq}` yapıyor. BASE için ilk emir bile 41 karakter. Bybit `bybit.rs:454` bunu doğrudan `orderLinkId`, OKX `okx.rs:524` doğrudan `clOrdId` olarak gönderiyor.

**Etki:** Bybit'in 36 karakter sınırı aşılır. OKX'in 32 karakterlik alfanümerik sözleşmesinde hem uzunluk hem tireler sorun oluşturur. Hedge ekleri kimliği daha da uzatır. LIVE emirlerin reddedilmesi beklenir; bu API sözleşmesinden yapılan çıkarımdır.

**Gerekli:** Borsaya uygun, kısa, deterministik ve benzersiz dış kimlik; bot/rol/sıra eşlemesi DB'de kalmalı.

**Kabul:** Tüm stratejilerin ve yeniden başlatma sonrası emirlerin borsa kimlik kurallarına uyduğu test edilmeli.

### EX-02 — Kritik: Spot market alışında base/quote birimi karışıyor

**Kanıt:** `mini-services/dca-core/crates/dca_exchange/src/bybit.rs:454` coin miktarını `qty` olarak veriyor, `marketUnit` belirtmiyor. `okx.rs:502` coin miktarını `sz` olarak veriyor, `tgtCcy` belirtmiyor.

**Etki:** İki borsanın da spot market BUY varsayılanı quote tutarıdır. Örneğin hesaplanan 0.001 BTC miktarı 0.001 USDT harcama talebi olarak yorumlanabilir; emir reddedilir veya beklenenden farklı büyüklükte gerçekleşir.

**Gerekli:** Bybit için açık `marketUnit=baseCoin`; OKX için uygun `tgtCcy=base_ccy` veya baştan quote tutarlı açık sözleşme. BUY/SELL ayrı doğrulanmalı.

**Kabul:** Sabit fiyatlı sahte adapter/testnet senaryosunda 100 USDT bütçe beklenen coin miktarını üretmeli.

### EX-03 — Kritik: Açık listeden kaybolan emir dolmuş sayılıyor

**Kanıt:** `mini-services/dca-core/crates/dca_exchange/src/lib.rs:281` `sweep_missing_fills` sadece açık emir kimliklerini karşılaştırıyor. `engine.rs:1069` sonrasında kaybolan emrin tam miktarı ve bekleme fiyatıyla sentetik fill üretiliyor.

**Etki:** Borsa panelinden iptal, expiry, reddedilme, eksik/paginated liste veya kısmi dolum yanlış tam fill olabilir. Gerçekleşmeyen alışa karşı satış veya yanlış pozisyon kapatma emri üretilebilir.

**Gerekli:** Terminal durum + gerçek execution kayıtları; fiyat, miktar, ücret ve execution-id bazlı artımsal işleme. Liste yokluğu fill kanıtı olmamalı.

**Kabul:** Cancel/expire/reject fill üretmemeli; %30 dolup iptal edilen emir yalnız %30 muhasebeleştirilmeli.

### EX-04 — Yüksek: Market emirlerinin gecikmeli/kısmi dolumları kaybolabiliyor

**Kanıt:** `engine.rs:619` sonrası sorguda `Ok(None)` takip oluşturmadan çıkıyor. `cum_exec_qty > 0` dalı ilk kısmi miktarı işleyip çıkıyor; kalan miktarın sürekli takibi kurulmamış. Sorgu hatalarının tüm denemeleri bitince de kalıcı belirsiz-emir kaydı yok.

**Etki:** Borsada gerçekleşen pozisyon yerel aynada hiç veya eksik görünür. Limit takibi de açık listede duran kısmi dolumları EX-03 akışı nedeniyle anında işlemiyor.

**Gerekli:** Emir gönderilmeden kalıcı pending kaydı, ACK/UNKNOWN/PARTIAL/TERMINAL durum makinesi, retry ve execution cursor.

**Kabul:** ACK sonrası gecikme, ağ kesilmesi, kısmi dolum ve tekrar sorguda tam olarak bir kez muhasebe.

### EX-05 — Yüksek: Gerçek fill'ler daima maker ve tahmini ücretle işleniyor

**Kanıt:** `mini-services/dca-core/crates/dca_kernel/src/lib.rs:558` dış fill'i `FeeRole::Maker` ve `RestingTouch` olarak geçiriyor. Market sorgu yolu da `engine.rs:637` üzerinden aynı fonksiyona giriyor. Gerçek komisyon/fee coin'i fonksiyon sözleşmesinde yok.

**Etki:** Market dolumu taker olarak hesaplanmıyor; hesap kademesi ve farklı fee coin'i kayboluyor. Net TP, bakiye ve gerçek kâr sapıyor.

**Gerekli:** Gerçek execution fee/asset/liquidity alanlarının taşınması; fee coin dönüşümünün açık muhasebesi.

**Kabul:** Maker/taker, base/quote/üçüncü coin ücretleri borsa kayıtlarıyla tutmalı.

### EX-06 — Kritik: Hedge canlı yürütmesi bacakları ayırmıyor

**Kanıt:** `state.rs:391` hedge motoru kuruyor fakat `state.rs:409` tek `SimulationStepper` kullanıyor. `engine.rs:553` ve `engine.rs:617` tüm emirlerde `position_idx: 0`. Kernel `execute_fill` pozisyon değişimini side yerine `OrderKind::Add/Reduce` ile belirliyor (`dca_kernel/src/lib.rs:464`).

**Etki:** Long ve short Add fill'leri kernel'de aynı yönde büyüyebilir. Bybit one-way modda short açma girişimi long'u azaltabilir; hedge modda yanlış position index reddedilebilir. Strateji bacak aynaları ile kernel/venue pozisyonu uyuşmaz.

**Gerekli:** Her bacak için pozisyon ve muhasebe; venue position mode doğrulaması; uygun bacak index/side eşlemesi.

**Kabul:** Eşit long-short açıldığında net delta sıfır, iki ayrı açık bacak ve doğru margin/fee muhasebesi görülmeli.

### EX-07 — Yüksek: OKX zorunlu pozisyon modu ile emir gövdesi çelişiyor

**Kanıt:** `okx.rs:29` ve `okx.rs:670` `long_short_mode` gerektiriyor. `okx.rs:517` emir gövdesinde `posSide` bulunmuyor. Kaldıraç ayarı hatası da `engine.rs:984` içinde sadece WARN oluyor; akış emir kararlarına devam ediyor.

**Etki:** OKX long/short modunun gerektirdiği alan yok; kaldıraç/pozisyon modu doğrulanmadan işlem denemesi sürüyor. Hedef kaldıraç ile gerçek hesap kaldıracı farklı olabilir.

**Gerekli:** Bacak bazlı `posSide`, uygun leverage sözleşmesi; senkronizasyon başarısızsa yeni risk artıran emirleri engelleyen durum.

**Kabul:** Mod/leverage uyuşmazlığında hiçbir yeni açılış emri gönderilmemeli; doğru modda iki yön ayrı doğrulanmalı.

### EX-08 — Yüksek: OKX swap dolum miktarı kontrattan coin'e dönmüyor

**Kanıt:** `okx.rs:504` gönderirken coin miktarı `ctVal` ile kontrata çevriliyor. `okx.rs:613` dönüşte `accFillSz` doğrudan `cum_exec_qty`; `engine.rs:636` bunu coin miktarıyla karşılaştırıp kernel'e geçiriyor.

**Etki:** `ctVal != 1` olduğunda özellikle kısmi dolumun coin miktarı yanlış olur. `positions` yolunda da kontrat boyutu ortak tipe normalize edilmiyor.

**Gerekli:** Ortak adapter tiplerinde birim standardı; tüm quantity/position/execution alanlarında çift yönlü kontrat dönüşümü.

**Kabul:** `ctVal=0.01`, 10 kontrat siparişi ve 3 kontrat dolum için işlenen miktar 0.03 coin olmalı.

### EX-09 — Yüksek: Canlı emirler paper kernel risk kapılarını aynı yoldan geçmiyor

**Kanıt:** `engine.rs:523` ve `engine.rs:589` PAPER dalında `submit_intent`; LIVE dalında doğrudan adapter `place_order` var. LIVE nakit başlangıcı istekten gelen yerel sermaye; bu dispatch yollarında gerçek wallet/margin rezervasyonu yok.

**Etki:** Simülasyon kapılarının varlığı canlıda aynı sermaye, miktar, ücret ve portföy sınırlarının uygulandığını göstermiyor. Birden çok bot aynı hesap bakiyesini bağımsızca kullanabilir. Bazı strateji kapıları mevcut olsa da ortak emir öncesi güvenlik sınırı eksik.

**Gerekli:** Tek gerçek-emir geçidi; bakiye/margin rezervasyonu, instrument filtresi, max exposure, veri tazeliği ve halt kontrolü.

**Kabul:** İki bot aynı son bakiyeyi harcamaya çalıştığında yalnız uygun olan kabul edilmeli; eski veri ve yetersiz margin yeni risk açmamalı.

### EX-10 — Yüksek: Enstrüman bilgisi seçilen borsadan alınmıyor

**Kanıt:** `state.rs:464` çözümleyicisi venue parametresi almıyor; `state.rs:476` hep Bybit'e gidiyor. Başarısızlıkta `state.rs:487` BTC varsayılanlarıyla sembol adını değiştiriyor. Runtime instrument'i kuruluşta alınıyor.

**Etki:** OKX/Binance filtreleri yerine Bybit tick/step/fee bilgileri kullanılabilir; ağ hatası tümüyle varsayılan filtrelere düşürür. Fallback'in cache'e yazılmaması, önceden oluşturulan runtime instrument'ini kendiliğinden güncellemez.

**Gerekli:** Venue+market+symbol ile çözümleme, tazelik süresi ve canlıda veri yoksa ret. Borsaya gönderirken tick/step'e uygun exact serialization; sabit 8 ondalık yeterli varsayılmamalı.

**Kabul:** Aynı sembolün farklı venue filtreleri korunmalı; metadata yokken LIVE açılamamalı.

## 5. Muhasebe, kalıcılık ve yaşam döngüsü

### BOT-01 — Kritik: Spot equity yanlış

**Kanıt:** `dca_kernel/src/lib.rs:496` alışta notional+fee cash'ten düşülüyor. `dca_kernel/src/lib.rs:565` spot equity'yi `cash + unrealized_gross` hesaplıyor; eldeki varlığın maliyeti/değeri eklenmiyor.

**Örnek:** 1.000 USDT ile 100 USDT'lik alış, 0.1 USDT ücret ve değişmeyen fiyat için doğru equity 999.9; mevcut formül 899.9 verir.

**Etki:** DD, günlük devre kesici, backtest equity, performans ve sertifika sonuçları sapar. Bu, doğrudan formülden çıkan bir hatadır; gerçek işlemle denenmedi.

**Gerekli/Kabul:** Spot equity serbest quote + mark üzerinden envanter değeri olmalı; alış/satış anında yalnız ücret kadar equity kaybı testi yapılmalı.

### BOT-02 — Yüksek: Fill idempotensi bütün muhasebeyi kapsamıyor

**Kanıt:** `dca_kernel/src/lib.rs:472` `PositionState::apply_fill` dönüşündeki bool kullanılmıyor; aynı fill pozisyona tekrar uygulanmasa bile cash/fee güncellemeleri sürüyor. `fill_key` order-id + yerel timestamp (`lib.rs:465`); DB `store.rs:35` üzerinde unique fill constraint yok.

**Etki:** Aynı fill'in tekrar işlenmesi nakit ve ücretleri iki kez değiştirebilir. Farklı yerel zamanla tekrar gelen aynı venue execution yeni fill gibi görülebilir.

**Gerekli/Kabul:** Venue execution-id ile DB seviyesinde uniqueness; pozisyon, cash, fee ve fill kaydının atomik/idempotent güncellenmesi; duplicate testinde tüm toplamlar değişmemeli.

### BOT-03 — Kritik: Restart açık pozisyonu ve emirleri geri yüklemiyor

**Kanıt:** `main.rs:193` restore config ve başlangıç sermayesinden runtime kuruyor. `state.rs:438` `last_bar_ts: 0`; `state.rs:447` `live_open_orders: Vec::new()`. Kernel pozisyonu, strategy phase/seq ve açık emir snapshot'ı geri yüklenmiyor.

**Etki:** Borsada açık işlem varken bot düz pozisyon sanabilir, yeni deal açabilir, mevcut emirleri takip edemeyebilir. Eksik LIVE anahtarında PAPER'a düşmek açık gerçek pozisyonu yönetmez.

**Gerekli/Kabul:** Kalıcı snapshot + execution replay + borsa uzlaştırması; önce RECOVERING. Süreç açık pozisyonda öldürülüp açıldığında quantity/fee/cash/emirler ve kimlik sayaçları korunmalı; uzlaştırma bitmeden yeni risk açılmamalı.

### BOT-04 — Kritik: Geçmiş ve henüz kapanmamış mumlar LIVE kararına giriyor

**Kanıt:** `engine.rs:989` son 60 mum alınıyor; `engine.rs:1006` yalnız timestamp cursor'uyla hepsi işleniyor. İlk cursor sıfır. Bybit `bybit.rs:313` kline dönüşünde kapanmamış mum filtrelemiyor. İşlenen mumun aynı timestamp ile sonraki güncellemeleri atlanıyor.

**Etki:** İlk poll geçmiş mumlar için bugünkü borsaya emir gönderebilir. Devamında oluşan mum bir kez işlenir ve gerçek kapanışı kaçabilir; SL/TP/sinyal davranışı sapar. Sinyal yardımcılarında kapalı mum filtresi bulunması bu ana döngüyü düzeltmez.

**Gerekli/Kabul:** Kapalı mumlar için zaman/eşik kontrolü, ayrı warmup ve execution akışı, kalıcı cursor ve gap backfill. Başlatma geçmişteki emirleri gerçek piyasaya göndermemeli; her kapalı mum bir kez işlenmeli.

### BOT-05 — Kritik: ADD_FUNDS gerçek emir oluşturmadan pozisyonu büyütüyor

**Kanıt:** `main.rs:878` `manual_add_funds` çağrısının action sonucunu kullanmıyor. `dca_strategy/src/dca.rs:521` yalnız strategy mirror'a sentetik fill uyguluyor; venue emri, stepper cash ve DB fill akışı yok.

**Etki:** Gerçekte alınmayan coin TP hesabına eklenebilir; strategy mirror/kernel/borsa üçü ayrışır. LIVE için özellikle tehlikeli.

**Gerekli/Kabul:** Kullanıcının kastı sermaye ekleme mi ek alım mı açık olmalı; ek alım gerçek execution sonrası muhasebeleştirilmeli. Başarısız emir hiçbir pozisyonu değiştirmemeli.

### BOT-06 — Yüksek: TP/SL patch'i kalıcı ve atomik değil

**Kanıt:** `main.rs:786` sadece DcaEngine'i patch'liyor; `rt.cfg` ve DB config güncellenmiyor. `dca.rs:484` negatif TP'yi sıfır kontrolüyle kaçırıyor; SL parse/hesap hatasından önce config ve `sl_price` değişiyor. Oluşturma yolundaki SL aralık kontrolü burada yok.

**Etki:** API'nin 422 döndüğü patch bile eski korumayı değiştirebilir; gösterilen config ile kullanılan config ve restart sonrası config farklı olabilir. `Option<Option<String>>` için JSON null ile SL silme sözleşmesi ayrıca test edilmeli.

**Gerekli/Kabul:** Önce tam doğrula, sonra tek işlemle strategy/runtime/DB güncelle; reddedilen patch'te hiçbir durum değişmemeli ve restart patch'i korumalı.

### BOT-07 — Kritik: PAUSE/STOP/DELETE açık emir ve pozisyonu sahipsiz bırakabiliyor

**Kanıt:** `main.rs:816` runtime'ı doğrudan siliyor. `main.rs:830` PAUSE/STOP sadece status değiştiriyor. Supervisor `main.rs:98` yalnız RUNNING/FROZEN botları poll ediyor.

**Etki:** Borsadaki resting emirler dolmaya devam edebilirken fill takibi ve yerel SL/TP durur. CLOSE_ALL yalnız bayrak kurduğu için PAUSED/STOPPED botta yeni karar geçişi olmadan sonuçlanmayabilir.

**Gerekli/Kabul:** Yeni girişleri durdurma ile risk azaltma/uzlaştırmayı ayır; silmeyi güvenli terminal duruma bağla. Pause/stop sonrası fill'ler ve mevcut pozisyon izlenmeli; close komutu durumdan bağımsız güvenli tamamlanmalı.

### BOT-08 — Yüksek: Manuel tick STOP/PAUSE sınırını aşabiliyor

**Kanıt:** `main.rs:2231` tick endpoint'i doğrudan `poll_and_advance` çağırıyor. `engine.rs:945` içinde STOPPED/PAUSED için erken ret yok; yalnız FROZEN/halt özel dalı bulunuyor. `main.rs:790` status patch'i de serbest string kabul ediyor.

**Etki:** Durdurulmuş bot API üzerinden tekrar işlem kararına sokulabilir; geçersiz status değerleri kalıcılaşabilir.

**Gerekli/Kabul:** Tek enum ve geçiş matrisi; hem otomatik hem manuel yürütmede aynı server-side durum denetimi. STOPPED/PAUSED tick'i yeni emir üretememeli.

### BOT-09 — Yüksek: Global runtime kilidi ağ istekleri boyunca tutuluyor

**Kanıt:** `engine.rs:950` bütün runtime haritasını kilitliyor; aynı kilitle kline, kaldıraç, sinyal, emir ve fill sorgularını await ediyor. Supervisor her 20 saniyede yeniden task oluşturuyor (`main.rs:94`).

**Etki:** Tek yavaş venue tüm botların kontrolünü ve haritaya erişen UI/action isteklerini bekletebilir. Tick süresi periyodu aşarsa işler birikebilir.

**Gerekli/Kabul:** Bot bazlı kilit/actor, bot başına single-flight, sınırlı kuyruk ve öncelikli risk azaltma. Bir venue time-out olurken diğer bot ve durdurma komutu yanıt vermeli.

### BOT-10 — Yüksek: Kill/freeze kalıcılığı ve risk azaltma davranışı eksik

**Kanıt:** `state.rs:267` son 20 kill/freeze olayını birlikte okuyor; bir tür diğerini pencere dışına itebilir. Parse/DB hatalarında varsayılan active=false kalıyor. `engine.rs:997` halt sırasında strateji kararları tamamen atlanıyor.

**Etki:** Restart'ta önceki halt kaybolabilir. Halt halinde yeni emir engellenirken mevcut pozisyonun bot tarafı TP/SL kararı da durur; açık emirler yine dolabilir. Bunun acil durum davranışı olarak açıkça tanımlanması gerekir.

**Gerekli/Kabul:** Her şalterin güvenilir son durum kaydı, hata halinde açıkça güvenli yürütme politikası; yeni risk ve reduce-only yollarını ayır. Karışık 100 olay ve bozuk kayıtla restart testleri yapılmalı.

### BOT-11 — Yüksek: Kalıcılık hataları işlem başarısından ayrılmıyor

**Kanıt:** `state.rs:350` `persist_status` sonuç döndürmüyor; iki yazma da başarısız olsa handler başarılı dönebilir. `engine.rs:646` market fill DB yazma hatasını yok sayıyor. Emir gönderimi, durum güncelleme ve kayıt yazımı arasında atomik recovery kaydı yok.

**Etki:** Kullanıcının durdurduğunu sandığı bot restart'ta eski durumda dönebilir; gerçek dolum kayıt dışı kalabilir.

**Gerekli/Kabul:** Kalıcı command/order günlüğü, başarısız yazımda kontrollü duruş, transaction/outbox ve kurtarma. DB kilit/disk-dolu testinde sahte başarı dönmemeli.

### BOT-12 — Yüksek: Canlı likidasyon modeli tek sabit MMR kullanıyor

**Kanıt:** `engine.rs:379` çevresindeki `current_liq_price` tek `mmr=0.005`, deduction=0 dilimi kullanıyor. `engine.rs:431` ve `engine.rs:459` LIVE/PAPER ayırmadan yerel forced-close fill'i işleyebiliyor.

**Etki:** Ayrı bracket API'sinin varlığı gerçek runtime hesabının venue risk kademesini kullandığı anlamına gelmiyor. Yerel sentetik LIQ fill gerçek borsa tasfiye kaydı yerine geçemez; yanlış eşikte muhasebe kapanabilir.

**Gerekli/Kabul:** Venue margin mode/bracket/mark/position bilgisiyle hesap; LIVE tasfiyesini borsa execution'ından öğrenme. Risk kademesi geçişleri ve yerel tahmin/borsa gerçekliği ayrımı test edilmeli.

### BOT-13 — Orta: İlk safety büyüklüğüne hacim çarpanı erken uygulanıyor

**Kanıt:** `dca_math/src/m02.rs:95` `current_size` safety tutarıyla başlıyor; `m02.rs:115` ilk seviyeden önce `volume_scale` ile çarpılıyor. Aynı fonksiyon açıklaması ilk safety'nin verilen safety tutarı olduğunu belirtiyor.

**Etki:** Safety=100 ve volume_scale=2 için ilk emir 200, ikinci 400 olur; ilk safety=100 sözleşmesi bekleniyorsa bütçe iki katına kayar. Ürün sözleşmesi netleştirilmeli; mevcut yorumla uygulama çelişiyor.

**Gerekli/Kabul:** İlk safety ve sonraki çarpım sırası tek tanımda sabitlenmeli; hesaplayıcı, runtime ve belge aynı 100/200/400 örneğini doğrulamalı.

## 6. Bağımlılık güvenliği

Kullanıcı, paket adları ve sürümlerinin npm güvenlik servisine gönderilmesini açıkça onayladıktan sonra tarama tamamlandı. İlk ağ denemesi DNS hatası vermişti; onay sonrası yanıt alınan taramanın çıkış 1 olması bulgu bulunduğunu gösterir. Ham çıktı `.audit-runtime/dependency-audit.json` içindedir.

**Sonuç:** 26 paket başlığında 106 danışma kaydı; aynı advisory URL'leri tekilleştirildiğinde **91 farklı danışma kaydı**: **5 critical, 46 high, 34 moderate, 6 low**. Bunlar 91 doğrulanmış uygulama sömürüsü veya 106 ayrı kurulu paket demek değildir; aynı advisory farklı sürüm aralıkları için tekrarlanabilir. Dev bağımlılıkları ve erişilmeyen özellikler ayrıca ayrıştırılmalıdır.

Kurulu sürümler doğrudan okundu: **Next.js 16.1.3, React 19.2.3**.

| Öncelikli kayıt | Uygulanabilirlik / yapılacak iş |
| --- | --- |
| [Windows sunucularda Next.js RCE — GHSA-p293-qw3h-jr36](https://github.com/advisories/GHSA-p293-qw3h-jr36) | Resmî kayıt Windows dosya sistemi ve Cache Components kullanılmayan App/Pages Router koşulunu belirtiyor. Yerel ortam Windows, config'te Cache Components açık değil; 16.1.3 etkilenen aralıkta. Sömürü yapılmadı. Bu kayıt için düzeltilen sürüm 16.3.3; bütün diğer kayıtlar için bu sürüm tek başına yeterli kabul edilmemeli. |
| [SVG image optimization RCE — GHSA-2464-fhr4-w9xw](https://github.com/advisories/GHSA-2464-fhr4-w9xw) | Resmî kayıt glibc Linux ve image optimization koşulunu belirtiyor; mevcut Windows koşusuyla aynı etki varsayılmamalı. Kayıttaki düzeltilen Next 16 sürümü 16.4.1; sharp için belirtilen alt sınır 0.35.5. Dağıtım hedefi ve erişilen image yolları incelenmeli. |
| [AVIF image optimization RCE — GHSA-2xp9-vwfh-vxw4](https://github.com/advisories/GHSA-2xp9-vwfh-vxw4) | Audit eşleşmesi; erişim ve platform koşulları ayrıca doğrulanmalı. |
| [use cache typed arrays RCE — GHSA-673q-mjm6-r5xr](https://github.com/advisories/GHSA-673q-mjm6-r5xr) | Audit eşleşmesi; ilgili özelliğin kullanıldığı bu incelemede kanıtlanmadı. |
| [Auth.js email normalization — GHSA-7rqj-j65f-68wh](https://github.com/advisories/GHSA-7rqj-j65f-68wh) | next-auth eşleşmesi; projede aktif e-posta giriş akışı doğrulanmadı. API'deki mevcut kimlik doğrulama eksikliğinden ayrı konu. |

**Gerekli:** Uygulanabilirlik matrisi, production/dev ayrımı, güvenli ve uyumlu sürümlere planlı yükseltme, kilit dosyasıyla temiz kurulum ve tekrar audit. Bu oturumda paket yükseltme veya otomatik audit fix yapılmadı. Rust tarafında `cargo audit` çalıştırılmadı; Rust bağımlılıkları için açık yok sonucu çıkarılamaz.

## 7. Ürün, test ve işletim eksikleri

### OPS-01 — Yüksek: Taşınabilir ve yeniden üretilebilir başlatma eksik

`package.json:5` script'leri `tee`, `cp` ve POSIX env ataması kullanıyor. `mini-services/dca-core/run.sh:6` `/home/z/my-project/db` dizinine bağlı; varsa release ikilisini kaynak güncelliğini denetlemeden çalıştırıyor. Bu Windows checkout'unda kaynaklardan eski ikili gerçekten mevcut.

Gerekli: Desteklenen işletim sistemi, sabit toolchain, lock'lu kurulum, platforma uygun start/build komutları, commit/build kimlikli binary ve tek belgelenmiş geliştirme akışı. Güncel kaynak için temiz build CI'da zorunlu olmalı.

### OPS-02 — Yüksek: Başarılı build'in tip güvenliği garanti edilmiyor

`next.config.ts:7` `ignoreBuildErrors: true`. Bu oturumda bağımsız TypeScript ve lint kontrolleri geçti, fakat ayar gelecekte tip hatalı üretim çıktısını engellemez. ESLint yapılandırması çok sayıda denetimi kapatıyor.

Gerekli: Ayrı zorunlu typecheck/lint/build adımları; Rust workspace testleri ve borsa sözleşme testleri. Mevcut kalite kontrollerinin geçmesi yürütme/muhasebe doğruluğu yerine geçmez.

### OPS-03 — Orta: Sağlık endpoint'i işlem yapabilirliği ölçmüyor

`main.rs:281` her zaman `status: ok` üretiyor. DB yazılabilirliği, son başarılı tick/fill, market veri gecikmesi, uzlaştırma sapması, aktif anahtar, leverage sync ve kuyruk durumu yanıtı etkilemiyor.

Gerekli: Liveness/readiness ayrımı; venue ve bot bazlı heartbeat, drift ve alarm göstergeleri. Süreç açık ama emir hattı bozukken yeşil durum gösterilmemeli.

### OPS-04 — Orta: Market cache hatayı kalıcılaştırıyor

`main.rs:1390` market listesinde hata boş listeye çevrilip süre sınırı olmadan cache'e ekleniyor. Başarısızlıkta hata kaynağı ile gerçek boş piyasa listesi ayrılmıyor.

Gerekli: TTL, başarısız yanıtı başarı olarak cache'lememe, veri zaman damgası ve uygun HTTP hata kodları. İlk istek başarısız, ikincisi başarılı senaryosu restart gerektirmemeli.

### OPS-05 — Orta: TradingView sırları restart'ta değişiyor; tekrar koruması sınırlı

`state.rs:416` civarında webhook secret yeniden üretiliyor; `state.rs:137` bot snapshot'ında açığa çıkıyor. `main.rs:2472` yalnız 60 saniyelik zaman penceresi uyguluyor; kalıcı alert-id/nonce deduplikasyonu bulunmuyor.

Gerekli: Yetkili ayrı secret görüntüleme/rotasyon işlemi, restart'ta kararlı entegrasyon ve alert-id deduplikasyonu. Aynı uyarı bir deal kapandıktan sonra pencere içinde tekrar yürütülmemeli.

### OPS-06 — Orta: Finansal kayıt büyümesi ve geri dönüş planı eksik

`store.rs:35` sonrası fill/event/equity tablolarında sorgu indeksleri ve fill uniqueness tanımlanmıyor. Her tick/mum olay yazıyor. Paket script'i `db:push` otomatik `--accept-data-loss` içeriyor; ayrıca Prisma şeması ile Rust DB'sinin ayrı sorumlulukları belgelenmeli.

Gerekli: `(bot_id, ts)`/execution kimliği indeksleri, retention/archiving, disk doluluk alarmı, SQLite backup+WAL tutarlılığı, geri yükleme tatbikatı ve üretim verisi için güvenli migration yolu.

### OPS-07 — Orta: Test kapsamı ve belgelerdeki başarı iddiaları yeniden doğrulanmalı

README 60 test ve altı crate anlatırken mevcut workspace sekiz crate içeriyor; tests dizininde E2E ve canlıya yakın senaryolar var, kök package script'inde tek bir standart test komutu yok. Bu incelemede mevcut README test iddiaları yeniden üretilemedi.

Gerekli: Offline unit/contract testleri, açıkça ayrılmış testnet E2E, doğrulanabilir CI kanıtı, kapsam ve destek matrisi. Testnet rampası önerisi tek başına üretime hazır olma kanıtı değil.

## 8. Çalışabilir bir bot için tamamlanması gerekenler

### Aşama A — Canlı işlem öncesi engeller

- [ ] Kimlik doğrulama/yetki/CORS/TLS ve sabit proxy upstream tamamlanmalı.
- [ ] Audit eşleşmeleri uygulanabilirlikleriyle ele alınmalı; özellikle Windows Next.js kaydı kapatılmalı.
- [ ] Kasa master key'i DB'den ayrılmalı; izinleri bilinmeyen anahtarla LIVE reddedilmeli.
- [ ] Güncel kaynak temiz build ile başlamalı; UI → Next rewrite → motor health akışı doğrulanmalı.
- [ ] EX-01/02 kimlik ve miktar birimi sözleşmeleri düzeltilmeli.
- [ ] BOT-01/02 spot equity ve fill idempotensi düzeltilmeli.
- [ ] Emir/fill ve restart recovery kalıcı hale gelmeli; LIVE uzlaştırılmadan açılmamalı.
- [ ] Hedge ve OKX bacak/kontrat dönüşümleri doğrulanana kadar ilgili LIVE stratejiler kapalı tutulmalı.

### Aşama B — İşlevsel bütünlük

- [ ] Gerçek execution feed veya güvenilir REST cursor; partial/cancel/reject/retry akışları.
- [ ] Ortak emir öncesi risk geçidi ve hesap seviyesinde margin/bütçe rezervasyonu.
- [ ] Kapalı mum, warmup, gap-backfill ve kalıcı cursor düzeni.
- [ ] Strategy/runtime/kernel/venue mutabakatı; drift'te yeni risk engeli.
- [ ] STOP/PAUSE/FREEZE/KILL/CLOSE/DELETE durum matrisi ve reduce-only önceliği.
- [ ] TP/SL patch kalıcılığı; ADD_FUNDS için açık ve gerçek execution temelli sözleşme.
- [ ] Bot başına yürütme izolasyonu; sınırlı iş kuyruğu, timeout, backoff ve hata sınıfları.
- [ ] Borsa tarafı koruyucu stop/TP veya motor kapanınca korumayı sürdüren açık bir mekanizma.

### Aşama C — İşletim ve kabul

- [ ] Readiness, son tick/execution yaşı, bakiye/pozisyon drift alarmı, hata oranı.
- [ ] Şifreli backup; restore tatbikatı; disk doluluğu ve DB yazma hatası testi.
- [ ] Frozen lock ile kurulum; toolchain pinleme; test/typecheck/lint/build/audit CI.
- [ ] Her venue ve strateji için PAPER/TESTNET/LIVE destek durumu; unsupported işlem server-side ret.
- [ ] Operatör runbook: bağlantı kopması, borsa bakımı, anahtar iptali, emir belirsizliği ve acil durdurma.
- [ ] Gerçek kullanım öncesi sınırlandırılmış testnet kabul koşulları ve ölçülen sonuçlar.

## 9. Önerilen zorunlu regresyon senaryoları

| Senaryo | Beklenen invariant |
| --- | --- |
| Spot alış, fiyat sabit | Equity yalnız gerçek ücret kadar azalır |
| Aynı execution iki kez | Cash, position, fee ve DB fill sayısı ikinci kez değişmez |
| %30 fill + cancel | Yalnız %30 pozisyon; kalan %70 yeni fill sayılmaz |
| Place ACK + query None/timeout | Emir kalıcı UNKNOWN/PENDING; kaybolmaz, kör tekrar yok |
| Cancel ile fill yarışı | Gerçek execution kaçmaz; aynı fill tekrar uygulanmaz |
| Açık pozisyonda restart | Emir/pozisyon/cash/seq korunur; reconcile öncesi açılış yok |
| STOP/PAUSE sonrası tick | Yeni risk açılmaz; mevcut risk izlenir |
| Global halt + açık pozisyon | Tanımlanmış reduce-only korumaları ve fill kaydı devam eder |
| Anahtarın kasadan kaldırılması | Bağlı runtime yeni emir gönderemez |
| İlk poll, 60 tarihî mum | Gerçek piyasaya tarihî karar emri gönderilmez |
| Henüz kapanmamış mum | Kalıcı strateji kararı olarak işlenmez |
| Metadata/leverage/permission alınamıyor | LIVE risk artışı reddedilir |
| Eşit hedge bacakları | Ayrı pozisyonlar, sıfır net delta, doğru IM ve ücret |
| OKX kontrat kısmi fill | ctVal ile doğru coin dönüşümü |
| İki bot, yetersiz ortak bakiye | Hesap seviyesinde toplam bütçe aşılmaz |
| DB busy/disk full | Sahte başarı yok; güvenli, gözlenebilir hata durumu |
| Kötü origin/kimliksiz çağrı | Yönetim işlemi reddedilir |
| Aşırı sayısal girdi | Erken 422; bellek/CPU tüketimi sınırlı |
| Tekrarlanan webhook + restart | Kararlı secret politikası, idempotent alert işleme |

## 10. Kaynaklar ve incelemenin sınırları

Dosya konumları aksi belirtilmedikçe proje köküne göredir. Borsa sözleşmesi karşılaştırmalarında kullanılan güncel birincil kaynaklar:

- [Bybit V5 Place Order](https://bybit-exchange.github.io/docs/v5/order/create-order): orderLinkId, spot market miktar birimi ve positionIdx.
- [OKX V5 API](https://www.okx.com/docs-v5/en/): clOrdId, tgtCcy ve long/short posSide sözleşmeleri.
- [OKX işlem rehberi](https://www.okx.com/docs-v5/trick_en/): istemci emir kimliği kuralları.
- Bağımlılık bölümündeki GitHub güvenlik danışmaları ve onaylı npm audit yanıtı.

Bu çalışma kritik execution, adapter, risk, persistence ve UI taşıma yollarını kapsayan kapsamlı bir kod incelemesidir; her satırın formal doğrulaması veya tüm özelliklerin başarılı dinamik testi değildir. AI/Pine/sinyal modüllerinin bütün algoritmik sınırları, tüm matematik formülleri, gerçek borsa fee/margin davranışı ve üretim dağıtımı uçtan uca doğrulanmadı. Kaynakta gösterilmeyen bir risk, kanıtlanmış açık olarak listelenmedi.

Çalışma dizininde `.git` bulunmadığından `git status`/diff ile başlangıç ve bitiş karşılaştırması yapılamadı. Uygulama kodunda düzeltme yapılmadı; rapordaki onarım ve test maddeleri sonraki uygulama çalışmasının kapsamıdır.
