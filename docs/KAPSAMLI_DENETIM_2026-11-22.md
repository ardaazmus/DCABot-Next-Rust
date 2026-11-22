# DCABOT kapsamlı kod, matematik, güvenlik ve UI/UX denetimi

> Bu denetim sonrası uygulanan düzeltmeler ve güncel açık işler için [denetim uygulama durumunu](denetim-uygulama-2026-11-22.md) inceleyin. Aşağıdaki bulgular, kaynak denetim anındaki durumun raporudur.

**Tarih:** 22 Kasım 2026
**İncelenen çalışma alanı:** `D:\project\yeni-dca-bot`
**Amaç:** Botu yerelde çalıştırmak; doğrulanabilir kod, matematik, işlem raporlama, güvenlik ve arayüz hatalarını bulup düzeltmek; kalan riskleri ve tamamlanması gereken işleri kaydetmek.

## Yönetici özeti

Yerel PAPER ortamı çalışıyor: arayüz `http://127.0.0.1:3000`, motor `http://127.0.0.1:3002`. Sağlık uç noktası `ok`, veritabanı `ok`; motor 18 bot bildiriyor ve salt okunur bot listesi isteği 18 kaydın tamamının `PAPER` modunda olduğunu doğruluyor. Kod veya test doğrulaması için canlı borsa emri gönderilmedi. Mevcut PAPER veritabanındaki botlar silinmedi, duraklatılmadı veya yeniden oluşturulmadı.

İncelemede oturum imzalama, istemci kaynaklı rate-limit kimliği, büyük sayı işleme, işlem kâr/zarar raporlaması ve başlangıç arayüzüyle ilgili doğrulanabilir hatalar bulundu ve düzeltildi. Next.js, etkilenen güvenlik aralığının dışındaki **16.4.5** sürümüne sabitlendi. npm güvenlik taraması sıfır açık bildirdi.

Yerel PAPER çalışması doğrulandı; bu sonuç LIVE/mainnet için işlem güvenliği sertifikası değildir. Canlı emir akışı, gerçek API anahtarlarıyla borsa hesabı ve mainnet yürütmesi denenmedi.

## Kapsam ve yöntem

- Next.js/React istemci ve API geçidi, Rust motoru, rasyonel matematik çekirdeği, strateji kapanış kayıtları, raporlama ve yerel servis betikleri okundu.
- Bulgu adayları erişilebilir kod yolu ve/veya tekrar üretilebilir kontrolle doğrulandı; güvenlik ve matematik düzeltmelerine regresyon testleri eklendi.
- Yerel arayüz Hesaplayıcı, Bot Filosu ve Ayarlar görünümlerinde incelendi. Risk azaltan/finansal işlem düğmelerine basılmadı.
- npm bağımlılık açık taraması kullanıcı onayıyla npm güvenlik servisine çalıştırıldı. Rust bağımlılık listesi başka bir servise gönderilmedi.
- Otomatik Codex Security derin taraması bu oturumda yönetilen dosya sistemi profili bulunmadığı için başlatılamadı. Bu nedenle repo içi kod incelemesi, testler ve yerel araçlar kullanıldı.
- Aktif PAPER veritabanını değiştiren `tests/engine_contract.mjs` çalıştırılmadı; yerel doğrulama salt okunur sağlık ve bot listesi GET istekleriyle yapıldı.

## Düzeltilen bulgular

| ID | Öncelik | Bulgu | Düzeltme ve doğrulama |
|---|---|---|---|
| SEC-01 | Yüksek | Oturum HMAC imzası yapılandırma yokken bilinen sabit bir varsayılan sır kullanabiliyordu. Bu, uygun koşullarda saldırganın doğrulanmış oturum token'ı üretmesine izin verirdi. | Sabit sır kaldırıldı. İmza için sunucu tarafında en az 32 baytlık anahtar gerekir; yapılandırma yoksa token üretimi durur, doğrulama fail-closed davranır ve giriş API'si 503 döndürür. Token zaman/boolean alanları da kesin doğrulanır. `npm run test:auth`: 5/5 geçti. |
| SEC-02 | Yüksek | Motor rate limiter'ı `X-Forwarded-For` değerini istemci kimliği kabul ediyor, sahte farklı değerlerle limit atlatılabiliyordu. Yeni istemciler için kova haritası da üst sınıra ulaşınca büyümeye devam edebilirdi. | Rate-limit anahtarı gerçek socket peer bilgisinden üretiliyor; gelen forwarded başlıkları kimlik için kullanılmıyor. Süresi geçmiş kovalar budanıyor ve 10.000 giriş doluysa yeni anahtar reddediliyor. İki Rust regresyon testi eklendi. |
| MATH-01 | Yüksek | Ondalık bilimsel gösterim ayrıştırıcısında `i64::MIN` üs değerinin `abs()` ile alınması taşmaya/paniğe yol açabiliyordu. Üs alma da ara sonuç boyutu denetlenmeden büyük tamsayılar üretebiliyordu. | Üs açık aralık kontrolüyle reddediliyor; `pow_u` ara adımlardaki bit uzunluğunu büyütmeden kontrol ediyor. Sınır durumları testle sabitlendi. `dca_math` testleri geçti. |
| DEP-01 | Yüksek | Kurulu Next.js 16.4.3, 16.4.4 ve öncesini etkileyen güvenlik duyurularının aralığındaydı. | `next` ve `eslint-config-next` 16.4.5'e sabitlendi; kilit dosyası güncellendi. `npm audit`: 712 bağımlılıkta 0 açık. Kaynaklar: [Next.js 16.4.5 sürüm notları](https://github.com/vercel/next.js/releases/tag/v16.4.5), [GHSA-mfr8-8qww-v76h](https://github.com/advisories/GHSA-mfr8-8qww-v76h), [GHSA-vc7v-qv4w-wrv8](https://github.com/advisories/GHSA-vc7v-qv4w-wrv8). |
| FIN-01 | Orta | İşlem P&L ayrıştırıcısı `NaN`/sonsuz değerleri rapora taşıyabiliyor; sayısal toplam ve JSON çıktısı bozulabiliyordu. | Geçersiz ve sonlu olmayan değerler ayrıştırmada eleniyor. |
| FIN-02 | Orta | Net sonucu sıfır olan işlemler kazanmış sayılıyordu. | Kazanç yalnızca net P&L `> 0` ise kazanım sayılıyor; başabaş işlem kazanım oranını şişirmiyor. UI etiketi de buna göre düzeltildi. |
| FIN-03 | Orta | Profit factor günlük toplamların oranından hesaplandığı için gün içindeki kazanç ve kayıplar birbirini götürebiliyor; sonuç işlem bazındaki gerçek PF'den farklı çıkıyordu. Çok büyük oranlar JSON `Infinity` üretebiliyordu. | PF tek tek işlem netlerinden, masraflar sonrası hesaplanıyor; raporlanabilir aralık dışındaki oran 99.99 ile sınırlandırılıyor. Günlük grafiğin açıklaması formülle uyumlu hale getirildi. |
| FIN-04 | Orta | Eksik çıkış nedeni, kâr/zarar işaretinden çıkarımla TP veya SL diye gösterilebiliyordu. Böylece kapanış sebebi yanlış raporlanıyordu. | Strateji gerçek kapanış nedenini işlem kaydına yazıyor. Raporlayıcı yalnızca açıkça bulunan TP/SL/LIQUIDATION/MANUAL/TRAIL kodlarını kullanıyor; bilinmeyen/eski kayıt “kapanış nedeni belirsiz” gösteriliyor. |
| FIN-05 | Orta | Tarih filtresine aşırı büyük gün sayısı verilmesi zaman damgası aritmetiğini taşırabiliyordu. | Gün aralığı 1–3650 ile sınırlanıp tarih aritmetiği saturating davranışa çevrildi. |
| UI-01 | Orta | `?view=calc` ile doğrudan açılışta sunucu Kokpit, tarayıcı Hesaplayıcı seçiyordu. React hydration uyuşmazlığı nedeniyle aktif gezinme öğesi hatalı kalabiliyordu. | Başlangıç görünümü Next 16 `searchParams` Promise'ından okunuyor; sunucu ve tarayıcı aynı görünümle render ediyor. Yerel tarayıcıda Hesaplayıcı ilk render'da doğru vurguyla açıldı. Kılavuz: `node_modules/next/dist/docs/01-app/03-api-reference/03-file-conventions/page.md`. |
| UI-02 | Orta | Backtest türü, kategori ve piyasa yüklenme durumu effect içinde eşzamanlı state güncellemeleriyle yönetiliyordu; kategori isteği değişiminde eski piyasa listesi bir an için gösterilebiliyordu. | Strateji seçimi kategori varsayılanını doğrudan belirliyor. Market/futures sonuçları sorgu anahtarıyla eşleştiriliyor; yeni anahtar gelene kadar eski sonuç kullanılmıyor. Dokunulan Backtest dosyasındaki lint sorunları da giderildi. |
| UI-03 | Orta | Filo üstündeki “Likidasyon Riski” filtresi yalnızca `unrealized / IM >= %50` erimesini sayarken kart aynı anda mark fiyatının tahmini tasfiye fiyatına %15'ten az uzaklığını kritik olarak işaretleyebiliyordu. Bu nedenle kartta kritik görünen bot filtre sayısında yoktu. | Filo sayacı ve filtresi iki kritik sinyali birleştiriyor: marj erimesi en az %50 veya açık marj pozisyonunda likidasyon fiyatına uzaklık %15'ten az. Kart ve filtre artık aynı kritik eşiği paylaşıyor. |
| OPS-01 | Orta | Yeniden başlatma betikleri 3000/3002 portundaki dinleyiciyi proje sahipliği doğrulanmadan durdurabiliyordu; başka yerel uygulamalar yanlışlıkla sonlandırılabilirdi. | Betikler PID, süreç başlangıç zamanı ve beklenen Next/Rust çalıştırıcısını denetliyor; doğrulanmayan dinleyiciyi durdurmayı reddediyor. Başlatma kayıtları güncelleniyor, veritabanı stop betiğinden korunuyor. PowerShell ayrıştırma kontrolleri geçti. |

## Açık kalan riskler ve işler

### P1 — LIVE fill uzlaştırması sırasında paylaşılan kilit

`mini-services/dca-core/crates/dca_server/src/engine.rs` içindeki `poll_inner`, LIVE `live_fill_sweep` venue ağ çağrısı sırasında `app.runtimes` kilidini tutuyor. Tek bir yavaş veya yanıt vermeyen borsa, diğer botların runtime state/UI güncellemelerini geciktirebilir. Dosyada kilitsiz ağ fazı mevcut olsa da bu yürütme dalı kilit altında kalıyor. Çözüm, fill sweep için gerekli durumu güvenle kopyalayıp ağ çağrısını kilit dışında yapmak ve sonuçları bot sürümü/işlem kimliğiyle yeniden uzlaştırmak; bu finansal durum eşzamanlılığını etkilediğinden ayrı, hedefli tasarım ve test gerektiriyor.

### P2 — 23 ESLint kural ihlali

Tam `npm run lint` 23 `react-hooks/set-state-in-effect` hatasıyla başarısız. Değiştirilen dosyalar ve Backtest paneli için hedefli lint temiz; kalanlar:

- `src/components/terminal/calculator.tsx` (4)
- `src/components/terminal/forensics.tsx` (1)
- `src/components/terminal/killswitch.tsx` (1)
- `src/components/terminal/settings.tsx` (3)
- `src/components/terminal/signal-builder.tsx` (2)
- `src/components/terminal/studio.tsx` (1)
- `src/components/terminal/studio/config-tab.tsx` (1)
- `src/components/terminal/tv-chart.tsx` (1)
- `src/components/terminal/wizard.tsx` (7)
- `src/components/ui/carousel.tsx` (1)
- `src/hooks/use-mobile.ts` (1)

Bu kural, effect gövdesindeki eşzamanlı state güncellemelerinin gereksiz render zinciri doğurabileceğini bildiriyor. Özellikle wizard ve calculator'daki form/strateji state'leri davranışı değişmeden önce alan bazında incelenmeli; toplu ve mekanik dönüşüm önerilmiyor.

### P2 — CSP gevşekliği

`next.config.ts` CSP'si üretim dahil `script-src 'unsafe-inline' 'unsafe-eval'` içeriyor; `connect-src` ayrıca tüm HTTP/HTTPS hedeflerine izin veriyor. Bu, XSS halinde script kısıtlamasını zayıflatır. Üretim CSP'si nonce/strict-dynamic veya doğrulanmış hash yaklaşımıyla sıkılaştırılmalı; geliştirme sunucusunun gereksinimleri ayrı tutulmalı ve Next'in RSC/hydration akışına karşı build/browser doğrulaması yapılmalı.

### P2 — Ters proxy arkasında rate-limit kimliği

Motor yalnızca loopback'e bağlı olduğundan, Next proxy'den gelen tüm uzak istemciler motor için aynı socket peer (`127.0.0.1`) görünür. İstemci `X-Forwarded-For` değerinin güvenilmemesi rate-limit atlatmasını kapatıyor; çok kullanıcılı dağıtımda ise meşru kullanıcılar aynı kovayı paylaşabilir. Böyle bir dağıtım gerekiyorsa Next proxy'nin istemci IP'sini güvenilir kaynaktan alıp motorun yalnız proxy'den kabul ettiği, istemcinin taklit edemeyeceği dahili kimlik aktarımı ayrıca tasarlanmalı. Tek kullanıcılı yerel modda bu pratik engel değildir.

### P3 — Format denetimi

`cargo fmt --all -- --check` çalışma alanında çok sayıda mevcut biçim farkı nedeniyle başarısız oldu. Tüm Rust çalışma alanına otomatik format uygulanmadı; bu, denetim dışı dosyalarda büyük ve ilgisiz diff oluşturacaktı. Değiştirilen Rust dosyalarının formatı bir sonraki ayrı Rust bakım işinde izole biçimde ele alınmalı.

## Doğrulama ve mevcut çalışma durumu

- `npm run test:auth`: **5 geçti, 0 kaldı**.
- `cargo test --locked --offline --workspace -- --quiet`: **335 geçti, 0 kaldı**.
- `cargo build --locked --offline --bin dca_server`: geçti.
- `npx tsc --noEmit`: geçti.
- Değiştirilen sayfa, kimlik doğrulama, backtest, filo ve işlem raporlama dosyaları için `npx eslint ...`: geçti.
- `npm audit --json`: **0 bilgi/düşük/orta/yüksek/kritik açık** (toplam 712 bağımlılık).
- `npm run lint`: **23 hata**; açık kalan listede.
- Yerel sağlık: `status=ok`, `db=ok`, `version=3.0.0`, `18/18` bot.
- Salt okunur `/api/engine/api/bots`: **18 kayıt, tamamı PAPER**.
- UI ekranında görülen PAPER kayıtlarında tükenmiş kademe ve kritik marj/likidasyon uyarıları var. Bunlar mevcut yerel PAPER verisinin durumudur; denetim sırasında bot eylemiyle değiştirilmedi.
- `tests/engine_contract.mjs` çalıştırılmadı; mevcut PAPER veritabanında bot yaratıp/sildiği için bu denetimde güvenli salt-okunur smoke testi tercih edildi.
- `npm install` çıktısı `@parcel/watcher`, `@swc/core` ve `unrs-resolver` lifecycle script'lerinin paket yöneticisi onayı beklediğini bildirdi. Next dev sunucusu webpack ile açıldı ve yerel UI smoke testi geçti; tam üretim `npm run build` bu oturumda çalıştırılmadı.

## Kaynaklar

- [Next.js 16.4.5 sürümü](https://github.com/vercel/next.js/releases/tag/v16.4.5)
- [GHSA-mfr8-8qww-v76h](https://github.com/advisories/GHSA-mfr8-8qww-v76h)
- [GHSA-vc7v-qv4w-wrv8](https://github.com/advisories/GHSA-vc7v-qv4w-wrv8)
- [Next.js `page` dosyası ve `searchParams` kılavuzu](https://nextjs.org/docs/app/api-reference/file-conventions/page)
