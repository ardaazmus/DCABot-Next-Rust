# DCABot-Next-Rust — Bağımsız Kapsamlı Denetim Raporu

**Denetim tarihi:** 12 Kasım 2026 (UTC)<br>
**İncelenen dal / başlangıç commit’i:** `arena/c0b4d989-dcabot-next-rust` / `c150fb33ddb3fbb5b48db66302a16fa0aa020dd1`<br>
**Kapsam:** Canlıya alma güvenliği ve operasyonları, Rust motoru ve borsa adaptörleri, P&L/deal verisi, Next.js UI/UX ve erişilebilirlik, dağıtım/CI ve güncel açık kaynak sektör kıyası.<br>
**Değişiklik kapsamı:** Bu turda uygulama kodu değiştirilmedi; bu belge yeni bir bağımsız denetim kaydıdır.<br>
**Önemli sınır:** Canlı borsa hesabı, gerçek API anahtarı veya gerçek emir kullanılmadı. Bu rapor yatırım tavsiyesi ya da mevzuat uygunluk değerlendirmesi değildir.

> ## Yönetici kararı: uzaktan erişim ve mainnet LIVE için **NO-GO**
>
> UI tarafında tip denetimi, lint, npm güvenlik taraması ve Next.js üretim derlemesi bu oturumda başarılı. Buna rağmen, kaynak incelemesi canlı işlem öncesi kapanması gereken iki kritik kapı gösteriyor: Compose varsayılanlarında UI kimlik doğrulamasının kapalı ve sabit gizli anahtarların etkin olması; ayrıca KILL/FREEZE devredeyken borsa fill’inden doğan yeni SAFETY/Add emirlerinin intent kaskadına gidebilmesi. Portföy notional kapısının bayat cache’de fail-open davranması ve açık LIVE emir rezervlerini cache’e almaması da gerçek maruziyeti düşük hesaplayabilir.
>
> Rust testleri, çalışan motora karşı E2E, tarayıcıda görsel/erişilebilirlik denetimi ve Docker build/boot **bu oturumda çalıştırılmadı**. Bu nedenle rapor “üretim onayı” değildir. Özellikle mainnet/uzaktan erişim, aşağıdaki P0/P1 maddeleri ve doğrulama kapıları kapanana kadar açılmamalıdır.

## 1. Kanıt standardı ve yöntem

Bu rapor üç kanıt türünü birbirinden ayırır:

- **[YENİDEN ÇALIŞTIRILDI]** — bu oturumda aynı checkout’ta gerçekten çalıştırılan komut/test sonucu.
- **[KAYNAK İNCELEMESİ]** — mevcut kaynak, ayar veya CI dosyalarında statik olarak görülen davranış; çalışan ürün davranışı olarak test edilmiş sayılmaz.
- **[TARİHSEL BELGE]** — depodaki önceki raporların kendi tarihindeki beyanı; bu oturumda yeniden üretilmiş kanıt değildir.

İnceleme; `docker-compose.yml`, `Dockerfile`, entrypoint/backup/standalone scriptleri, CI, Next proxy/auth/session katmanı, health/SSE, Rust engine/fleet/store/deals ve Bybit/OKX/Binance adaptörleri ile kokpit, filo, grafik, backtest ve ayarlar ekranlarının kaynak kodunu kapsadı. Sektör kıyası için Freqtrade, FreqUI, Hummingbot Dashboard ve Hummingbot API’nin resmi GitHub README’leri 12 Kasım 2026’da okundu; ürünlerin çalışan UI’ları kurulup kullanılmadı.

## 2. Bu oturumda bağımsız doğrulananlar

| Kontrol | Sonuç | Kanıt sınırı |
|---|---|---|
| `npm ci` | **Başarılı**; 620 paket denetlendi | Paket kurulumu; çalışma zamanı testi değil. |
| `npx tsc --noEmit --incremental false` | **Başarılı** | UI tip denetimi. |
| `npx eslint src --quiet` ve `npm run lint` | **Başarılı** | Statik lint. |
| npm audit kontrolleri | **Başarılı; 0 açıklık bildirildi** | npm advisory taraması; tüm güvenlik risklerinin yokluğu anlamına gelmez. |
| `npm run build` | **Başarılı**; Next.js 16.4.3 standalone postbuild tamamlandı | UI üretim bundle’ı; Docker imajı veya container boot testi değil. |
| Rapor sonu whitespace taraması | **Başarılı** | Yalnız yeni Markdown raporu eklendi; uygulama kaynağı değiştirilmedi. |
| Rust / Docker / tarayıcı | **Çalıştırılamadı** | Ortamda `cargo`, `rustc`, Docker ve Playwright tarayıcı binary/cache’i yoktu. |

`mini-services/dca-core` içinde statik aramayla **324 test anotasyonu** bulundu; bu, testlerin derlendiği veya geçtiği anlamına gelmez. `tests/engine_contract.mjs` çalışan motor/UI ve `.local-runtime` yapılandırmasına bağlı olduğundan burada koşturulmadı. Playwright paketi bulunmasına rağmen browser kurulumu olmadığı için E2E/görsel tur yapılmadı.

### Tarihsel denetim belgeleri hakkında

`docs/KAPSAMLI_DENETIM_2026-11-12.md` içinde önceki koşulara ait 308 Rust testi/25 sözleşme testi, daha sonraki bölümde 324 Rust testi/27 sözleşme testi ve `163/168` E2E sonucu (5 başarısız senaryo) gibi aşamalı sonuçlar yer alıyor. Bu sayılar bu oturumda yeniden çalıştırılmadı; özellikle E2E’nin son kaydı tüm testlerin geçtiğini söylemiyor. `docs/UI_UX_GORSEL_DENETIM_2026-10-30.md` 2026-10-30 tarihinde UIX-18..42’nin tarayıcıda doğrulandığını beyan ediyor; bu turda ekran görüntüsü, ekran okuyucu veya çalışan uygulama üzerinden bağımsız doğrulama yapılmadı. Bu raporun sonucu ile tarihsel beyanlar birbirinin yerine kullanılmamalıdır.

## 3. Canlıya alma açısından öncelikli bulgular

Öncelik tanımı: **P0** = dış erişim/mainnet öncesi kapanmalı; **P1** = gözetimsiz canlı kullanım öncesi kapanmalı veya işletim politikasıyla açıkça sınırlandırılmalı; **P2** = güvenlik, kullanılabilirlik ve bakım sertleştirmesi.

### P0-1 — Compose varsayılanı, UI’ı kimlik doğrulamasız yönetim API geçidine dönüştürüyor

**Kanıt: [KAYNAK İNCELEMESİ — yüksek güven]** `docker-compose.yml:11-24`, `src/proxy.ts:61-79`, `src/lib/auth-session.ts:9-13`.

- Compose `3000:3000` portunu host’un tüm arayüzlerine yayınlıyor; `DCA_UI_PASSWORD` boş, `DCA_REQUIRE_AUTH=0`.
- `DCA_API_KEY` ve `DCA_VAULT_KEY` için bilinen, sabit fallback değerleri var. Değişken boş bırakılmadığı için entrypoint’in rastgele anahtar üretme yolu (`scripts/docker-entrypoint.sh:13-32`) normal Compose varsayılanında devreye girmiyor.
- Motor yönetim API’leri Bearer anahtarı ister; ancak Next proxy bu anahtarı **sunucuda** ekler. UI auth ayarı kapalıyken UI’a erişebilen bir istemci, same-origin `/api/engine/*` üzerinden bot, kasa ve diğer yönetim uçlarına gidebilir. Yani motorun Bearer middleware’i tek başına UI’ı korumaz.
- `/api/health` ve `/metrics` motor tarafında ayrıca Bearer auth’suz bırakılmıştır; health yüzeyi bot sayısı/servis durumu gibi operasyonel bilgi taşır. `/api/health`, Next proxy’de de auth istisnasıdır. Mevcut Compose `3002`’yi host’a publish etmiyor ve proxy’nin yalnız `/api/*` yollarına izin vermesi `/metrics`’i UI proxy’sinden erişilmez kılıyor; bu nedenle `/metrics`’i public ilan etmiyorum. Engine portu ağ seviyesinde private kalmalı; health’in dış izleme erişimi ise bilinçli olarak sınırlandırılmalıdır.

**Etki:** Bu Compose ayarı LAN’a veya internete açık bir makinede kullanılırsa, bağlantı kurabilen taraf yalnızca dashboard’u görmez; Next proxy üzerinden motorun tam yetkili yönetim API’sine ulaşabilir. `DCA_ALLOW_MAINNET=1` ile canlıya alınmış bir instance’da bu durum emir ve kasa riski doğurur. HSTS başlığı tek başına TLS sağlamaz; düz HTTP portunu güvenli hale getirmez.

**Canlı öncesi kabul ölçütü:** Sabit fallback’leri kaldırın ve eksik/varsayılan secret’ta container’ı başlatmayı reddedin; birbirinden bağımsız, güçlü API/vault/UI secret’ları yönetin; `DCA_REQUIRE_AUTH=1` ve güçlü `DCA_UI_PASSWORD` zorunlu olsun. UI’ı TLS arkasına ve mümkünse VPN/özel ağa koyun; yalnız gerekli portu yayınlayın, engine `3002`’yi host’a açmayın. Erişim kuralı, login’siz UI’dan yönetim API’sine erişimin 401/403 ile reddedildiğini ve dış arayüzden portun kapalı olduğunu kanıtlamalıdır. Vault anahtarı değiştirilecekse mevcut şifreli borsa anahtarlarının yeniden şifrelenmesi/geri yüklenmesi ayrıca planlanmalıdır.

### P0-2 — KILL/FREEZE, fill-sonrası yeni Add/Safety emirlerini kesin olarak durdurmuyor olabilir

**Kanıt: [KAYNAK İNCELEMESİ — ciddi, dinamik repro yapılmadı]** `engine.rs:1951-2052,2072-2089,2160-2253`, `dca_strategy/src/dca.rs:457-538,886-945`, `engine.rs:673-795,1063-1095`.

Kaynak akışında global şalter `process_bar`/strateji kararını engelliyor; ancak LIVE fill sweep’i çalışmaya devam ediyor. Fill işlendiğinde `EngineTag::on_fill` aksiyonları `dispatch_action`’a gönderiliyor ve ardından intent kaskadı çalışıyor. Bu yollar kaskad/place aşamasında `kill_engaged`/`live_halted` durumunu yeniden kontrol etmiyor. DCA ana emir fill’i `on_base_filled` ile SAFETY/Add emirleri üretebiliyor; SAFETY fill’i de sonraki kademeyi tekrar yerleştirebiliyor. `set_killswitch` ayrıca mevcut borsa emirlerini iptal eden bir akış başlatmıyor.

**Etki:** Operatör KILL’i devreye aldıktan sonra borsada zaten bekleyen BASE/SAFETY emri fill olursa, stratejinin fill-tepki kaskadından yeni Add emirleri çıkması kaynakta mümkün görünüyor. Ayrıca daha önce gönderilmiş giriş emirleri borsada kalıp sonradan fill olabilir. Bu, butondaki “yeni emir üretimi durur” beklentisiyle tam uyumlu bir acil durdurma değildir. Bu sonuç statik akıştan çıkarılmıştır; çalışan Bybit/OKX testnet’inde henüz repro edilmemiştir.

**Canlı öncesi kabul ölçütü:** LIVE intent’i borsaya göndermeden hemen önce şalteri atomik olarak tekrar kontrol edin; şalter devreye girince henüz gönderilmemiş tüm `Add` intent’lerini iptal edin. Borsadaki bekleyen giriş emirlerini iptal edip sonucu uzlaştırın; `Reduce`/koruyucu çıkışların davranışını açık ve ayrı bir politikayla tanımlayın. Fill sweep’i muhasebe/uzlaştırma için sürdürebilir, ancak kill aktifken fill’den gelen yeni `Add` emirleri kesinlikle gönderilmemelidir. Yarış pencerelerini de kapsayan entegrasyon testleri zorunlu: (1) açık BASE emri + kill + fill, (2) açık SAFETY emri + kill + fill, (3) kill ile aynı anda intent kaskadı, (4) reduce-only çıkışın beklenen biçimde korunması/işlemesi. UI metni de gerçek semantiği anlatmalıdır.

### P1-1 — Portföy notional kapısı cache bayatken fail-open; LIVE açık emir defteri hesapta yok

**Kanıt: [KAYNAK İNCELEMESİ — yüksek güven]** `engine.rs:1263-1292`, `fleet.rs:434-523,538-578`; Compose’ta eşik `docker-compose.yml:23`.

`DCA_PORTFOLIO_MAX_NOTIONAL_PCT` varsayılanı %85; `0` açık bir opt-out. Kontrol yalnız cache 30 saniyeden yeni ise yapılıyor; `portfolio_cache_read` eski cache’te `None` döndürüyor ve `dispatch_action` bu durumda emri engellemeden devam ediyor (yorum da bu yolu fail-open diye tanımlıyor). Cache yenilemesi ana/hedge pozisyonlarını ve kernel pending Add emirlerini topluyor, fakat `rt.live_open_orders` listesini dolaşmıyor. Böylece borsada bekleyen açık Add emirlerinin rezervi eksik sayılabilir. Ek olarak `NAV <= 0` ise `portfolio_cap_exceeded` bloklamıyor.

**Etki:** Birden fazla bot veya uzun süren venue/API gecikmesinde eşik gerçek toplam maruziyeti korumayabilir. Açık emri dolmamış emir de potansiyel pozisyon olduğundan, yalnız pozisyon + kernel pending toplamı yeterli değildir.

**Canlı öncesi kabul ölçütü:** Cache’e `live_open_orders` içindeki Add emirlerini de ekleyin; aynı emri kernel pending/live order arasında çift saymamak için idempotent ortak maruziyet kaynağı kullanın. Cache bayat, venue okunamıyor, NAV sıfır/negatif veya hesaplama hatalıysa yeni Add emrini **fail-closed** engelleyin ve görünür alarm üretin. `0` opt-out’u yalnız açık ve denetlenebilir operator override ile kabul edin. Paralel bot emirleri ve restart senaryolarında toplam maruziyetin eşiği aşmadığını kilitleyen Rust/entegrasyon testleri ekleyin.

### P1-2 — Compose’ta tanımlı günlük portföy kayıp limiti kodda tüketilmiyor

**Kanıt: [KAYNAK İNCELEMESİ — yüksek güven]** `docker-compose.yml:24`; Compose dışındaki kaynaklarda `DCA_PORTFOLIO_MAX_DAILY_LOSS_PCT` eşleşmesi bulunmadı. Ayrı mekanizma `engine.rs:172-205`, `dca_kernel/src/risk_policy.rs` içindeki `daily_dd_breaker_pct`.

Compose `%10` `DCA_PORTFOLIO_MAX_DAILY_LOSS_PCT` ayarını veriyor; motor kaynaklarında bu değişkeni okuyan kod yok. Bu ayar, adı/README iddiası ne olursa olsun mevcut kodda etkin bir portföy limiti olarak doğrulanamadı. Ayrı bir **bot başına** günlük DD breaker var; kendi `risk_policy.daily_dd_breaker_pct` ayarını kullanıyor (varsayılan `%2`) ve yeni deal açılışını engelliyor. Bu, portföy-geneli günlük zarar tavanı değildir; mevcut pozisyon yönetimini sürdürür ve açık borsa giriş emirlerini otomatik iptal ettiği bu incelemede gösterilmedi.

**Canlı öncesi kabul ölçütü:** Ürün sözleşmesinde “bot başına” ve “hesap/portföy geneli” kayıp limitlerini ayırın. `%10` hesabı gerçekten uygulanacaksa ortak equity/NAV kaynağı, UTC gün reseti, açık emir iptali, restart kalıcılığı, funding/fee dahil net zarar tanımı ve alarm davranışıyla implemente edip test edin. Uygulanmayacaksa Compose değişkenini ve ilgili iddiayı kaldırın; aktifmiş gibi sunmayın.

### P1-3 — Kapanan deal defteri ve dashboard ekonomik K/Z’si güvenilir tek kaynak değil

**Kanıt: [KAYNAK İNCELEMESİ — yüksek güven]** `deals.rs:47-155,177-220`, `store.rs:65-75`, `fleet.rs:410-423`, `main.rs:1083-1156,1159-1258`, `dashboard.tsx:438-444`.

- `deals` tablosu oluşturuluyor ve endpoint bu tabloyu okuyabiliyor; ancak Rust kaynaklarında tabloya `INSERT`/`UPDATE` yazıcısı bulunmadı. `/api/deals` bunun yerine `events.message LIKE '%deal closed%'` üzerinden metin ayrıştırıyor. Motorun kendi fleet notu, kapanan deal defterinin yazılmadığını ve yalnız fill’lerin kalıcı olduğunu açıkça not ediyor.
- Deal parser finansal alanları `f64`’e dönüştürüp sekiz basamağa yuvarlıyor; bu, rasyonel muhasebe çekirdeğinin kalıcı/otoritatif muhasebe sözleşmesiyle aynı hassasiyet değildir. Olay metni formatına bağlı rapor; DCA dışı stratejilerin (ör. grid) tüm kapanışlarını kapsadığı kanıtlanmış bir ledger değildir.
- `/api/summary` `totalRealizedGross`, `totalFees` ve `totalUnrealizedPnl` döndürüyor, fakat funding toplamı döndürmüyor. Kokpit `totalEconomicPnL = realizedGross - fees + unrealized` hesaplıyor; funding bu “ekonomik K/Z” headline’ında yok. Oysa kernel’in `realized_net` tanımı `gross - realized_fees - cumulative_funding` ve funding SSE’si kümülatif funding alanını taşıyor.
- Summary toplaması `rt.stepper.cumulative_fees` ve ana runtime alanlarını kullanıyor. Hedge bacağı için ayrı stepper ve ayrı cumulative fee/funding bulunduğundan, filo/summary aggregate’inin hedge bacağıyla aynı net muhasebeyi kullanıp kullanmadığı ayrıca kilit testle kanıtlanmalıdır. `BotRuntime.snapshot_json` hedge unrealized’ı birleştiriyor; bu nedenle gross/fee/funding alanlarını da aynı bacak kapsamına getirmek önemlidir.
- Filo overview notu, STOPPED botların bellekteki toplamdan düşebildiğini söylüyor. Tarihsel botların gerçekleşen K/Z’si kapanış ledger’ından gelmiyorsa kokpit toplamı zamanla eksik kalabilir.

**Etki:** UI’daki temel performans metriği finansman maliyetini ve potansiyel olarak hedge/geçmiş botları tam yansıtmayabilir; günlük K/Z/deal geçmişi eksik veya metin şemasına bağımlı olabilir. Bu, emir kararından ayrı olsa da kullanıcı güvenini ve canlı risk yönetimini etkiler.

**Canlı öncesi kabul ölçütü:** Her strateji/bacak kapanışını tek işlem sınırında, idempotent bir `deals` ledger’ına yazın; ücret, funding, gross/net, kapanış nedeni ve timestamp’i decimal string olarak saklayın. DCA/Hedge/Grid, kısmi kapanış, funding iadesi, stop/likidasyon, restart ve STOPPED/silinmiş bot testleri ekleyin. API açıkça `realizedGross`, `fees`, `funding`, `realizedNet`, `unrealized` ve `economicPnL` sözleşmesi versin; UI bu tek sunucu hesaplamasını göstersin. Ledger ile özet arasında günlük ve bot bazlı mutabakat raporu üretin.

### P1-4 — LIVE sertifika kapısı kullanıcı onayıyla istatistiksel başarısızlıkta aşılabiliyor

**Kanıt: [KAYNAK İNCELEMESİ — bilinçli ürün istisnası]** `main.rs:1290-1299,1702-1797`, `dca_strategy/src/certify.rs:514-567`.

LIVE bot açılışında motor veriyi yeniden çekip sertifika kapısını kendi içinde çalıştırıyor; bu olumlu bir server-side kontrol. Ancak `cert_acknowledged=true`, kapı `KALDI` olduğunda istatistiksel eşiklerin (örneklem, WFE, DSR, PBO ve vadeli funding geçmişi) başarısızlığını kalıcı WARN/SSE olayıyla aşabiliyor. Bu bayrak market verisi/enstrüman meta verisi alınamaması gibi daha önce fail-closed dönen kontrolleri atlamıyor; `DCA_ALLOW_MAINNET` ve venue kısıtları ayrı kapılar. Dolayısıyla bu, “mutlak fail-closed sertifika” değildir.

**Canlı öncesi karar:** İşletme politikası kullanıcı onaylı istisnaya izin veriyor mu? İzin veriyorsa güçlü ve audit edilebilir bir reason, operatör kimliği, ayrı onay/ikinci kişi ve raporlama ekleyin; `cert_acknowledged` tek boolean olarak sınırsız ve sessiz kullanıma bırakılmasın. Politika “sertifika geçmeden LIVE yok” ise override’ı kaldırın. Her iki halde de sertifikayı kârlılık garantisi olarak sunmayın.

### P1-5 — Docker üretim Node sürümü doğrulanmadı; builder ile runner farklı kaynak kullanıyor

**Kanıt: [KAYNAK İNCELEMESİ — runtime sonucu bilinmiyor]** `Dockerfile:24-46`; Next.js 16.4.3 `engines.node >=20.9.0`.

UI builder `node:22-bookworm-slim` kullanırken production runner `debian:bookworm-slim` üzerine apt `nodejs`/`npm` kuruyor. Runner’daki gerçek Node sürümü bu oturumda Docker olmadığı için ölçülemedi; Next’in manifest gereksinimi `>=20.9.0`. Bu nedenle image build başarılı olsa bile standalone server’ın üretim container’ında desteklenen Node ile açıldığı henüz kanıtlı değil.

**Kabul ölçütü:** Runner’ı da sabitlenmiş Node 22 slim imajından üretin (veya aynı major/minor runtime’ı açıkça kurun); `docker build` ardından gerçek image ile `/` ve `/api/engine/api/health` smoke testi çalıştırın. Node sürümünü CI’de basın ve beklenen aralığa uymuyorsa build’i düşürün.

### P1-6 — Entrypoint shutdown trap’i `exec` ile devreden çıkıyor; motor hazır değilken UI yine başlayabilir

**Kanıt: [KAYNAK İNCELEMESİ]** `scripts/docker-entrypoint.sh:42-61`, Compose healthcheck `docker-compose.yml:30-35`.

Entrypoint Rust motorunu arka plana alıp shell’e `TERM/INT` trap’i kuruyor, fakat sonrasında `exec node /app/server.js` yapıyor. `exec` shell’i Node süreciyle değiştirdiği için shell trap’inin container durdurulurken motoru graceful kapatacağı garanti görünmüyor. Ayrıca 30 saniyelik readiness döngüsü sonunda motor sağlıklı değilse script hata verip çıkmıyor; UI’ı başlatıyor. Compose healthcheck yalnızca Next UI `/` yanıtını kontrol ediyor, Rust engine readiness’ini doğrulamıyor.

**Kabul ölçütü:** `tini` veya gerçek bir supervisor kullanın; sinyali her iki child’a iletip ikisinin de kapanmasını bekleyin. 30 saniye sonunda engine hazır değilse container startup başarısız olsun. Readiness healthcheck hem UI’ı hem motorun DB/tick readiness durumunu kapsasın; liveness ile readiness’i ayırın. SIGTERM, motor çökmesi, UI çökmesi ve açılışta geç/başarısız DB senaryolarını container testiyle sınayın.

### P1-7 — SQLite backup tutarlılığı, saklama yeri ve otomatik çalışma doğrulanmamış

**Kanıt: [KAYNAK İNCELEMESİ]** `scripts/backup-db.mjs:11-59`, `package.json`, `docker-compose.yml:28-35`.

Backup script’i SQLite ana dosyasını, `-wal` ve `-shm` dosyalarını sırayla `copyFileSync` ile alıyor; aktif yazım sırasında tutarlı atomik snapshot garantisi yok. Repoda bu script’i çağıran package script’i, Compose servisi veya scheduler bulunmadı; backup’ın periyodik çalıştığı gösterilmedi. Script’in hedefi proje kökü altındaki `backups/`; container içinde paketlenip/çalıştırıldığı ayrıca doğrulanmadı. Çalıştırıldığı bir kurulumda hedef dizin kalıcı volume’a bağlanmalıdır (mevcut Compose yalnız `/app/data`’yı mount ediyor).

**Kabul ölçütü:** SQLite online backup API (`sqlite3_backup`/uygun destek) veya checkpoint/lock kontrollü snapshot kullanın. Yedeği kalıcı volume dışına veya şifreli off-host storage’a alın, planlı iş/başarısızlık alarmı kurun, retention’ı test edin. En az bir temiz ortamda restore drill yapıp RPO/RTO’yu belgeleyin; yalnız “backup dosyası oluştu” yeterli kabul edilmemelidir.

### P1-8 — CI temel statik kapıları içeriyor; üretim ve güvenlik kapıları zorunlu değil

**Kanıt: [KAYNAK İNCELEMESİ]** `.github/workflows/ci.yml:7-57`, `package.json`.

Pull request CI’sinde Rust workspace testi ve UI npm ci/TypeScript/ESLint kapıları var; Clippy `continue-on-error: true`. Bu oturumda workflow’ların GitHub’da çalışıp geçtiği ayrıca kontrol edilmedi. CI dosyasında Next production build, Docker build/boot, Playwright browser testleri, npm audit/dependency scan ve engine sözleşme E2E görünmüyor. `npm test` sözleşme testi yerel `.local-runtime`/çalışan servis gerektirdiği için CI’ye eklenmesi ayrıca tasarlanmalıdır.

**Kabul ölçütü:** PR’e production build, `npm audit`/uygun dependency taraması, `cargo fmt --check`, Clippy uyarı politikasına göre zorunlu kapı, Rust test, Docker build ve testnet/PAPER mock’lu API/E2E smoke ekleyin. Flaky gerçek-borsa testlerini zorunlu CI yerine deterministik adapter mock’ları ve ayrı gece koşusu olarak ayırın. Fail durumunda canlı imaj yayınlanmamalıdır.

### P2-1 — “Per-IP” rate limiter aslında bütün yönetim API’si için tek global bucket

**Kanıt: [KAYNAK İNCELEMESİ — yüksek güven]** `main.rs:51-57,257-269`.

Limiter çağrısı her request’te `l.check("gateway")` kullanıyor; `check` anahtarı `_key` olarak alıp tüm istekleri tek bucket’ta tutuyor. Gerçek sayaç 10 saniyede 400 istek; middleware yorumunda ise 100/10 saniye ve burst 20 yazıyor. Böylece çok sayıda normal panel/poll isteği ortak limiti tüketebilir; IP bazlı koruma da sağlanmaz.

**Öneri:** Ya tüm kullanıcılar için adil, belgelenmiş global kapasite uygulayın; ya reverse proxy’den doğrulanmış client-IP anahtarıyla per-IP limit ve yönetim endpoint’lerinde ayrı quota kurun. Proxy arkasında spoof edilebilir `X-Forwarded-For` değerini doğrudan güvenmeyin. Yük testi, 429 ve dashboard normal kullanım testi ekleyin; yorum ve kod limitlerini eşitleyin.

### P2-2 — Docker build context filtresi yok ve container root olarak çalışıyor

**Kanıt: [KAYNAK İNCELEMESİ]** Kök dizinde `.dockerignore` bulunmuyor; `Dockerfile:31-33` `COPY . .` yapıyor ve Dockerfile’da `USER` yönergesi yok.

`.dockerignore` olmayınca `node_modules`, `.next`, `target`, `.git`, local runtime dosyaları veya varsa `.env`/SQLite/backup gibi Git dışı dosyalar build context’e girebilir. Bunların tümünün mevcut checkout’ta bulunduğu iddia edilmiyor; fakat Docker build context bunları filtrelemiyor. Builder aşamaları final image’dan ayrı olsa da, gereksiz context, cache ve hassas dosyaların build hattına taşınma riskini artırır. Final runner varsayılan root kullanıcısıyla çalışır.

**Öneri:** `.dockerignore` ekleyip `.git`, `node_modules`, `.next`, Rust `target`, `.local-runtime`, `.env*`, anahtar/DB/backup dosyalarını açıkça hariç tutun. Final image’da non-root kullanıcı, dar dosya izinleri ve yazılabilir yalnız `/app/data` kullanın; ownership’i image build’de ayarlayın. Build log/cache’lerinde secret sızıntısı kontrolü yapın.

## 4. Güçlü kontroller ve canlı sınırları

**Kaynakta mevcut olan olumlu kontroller [KAYNAK İNCELEMESİ]:**

- UI motor API anahtarını tarayıcıya taşımak yerine sunucu tarafı `/api/engine/*` proxy’sinde Bearer ekliyor; path traversal/SSRF için sabit loopback hedef ve origin kontrolü var (`src/proxy.ts`). UI auth etkinleştirilirse proxy’de oturum kontrolü yapılıyor.
- Motor yönetim API’leri Bearer auth, gövde boyut limiti ve rate-limit katmanı altında; SSE auth’lı; TradingView webhook’u kendi secret, ±60 saniye replay, dedup ve gövde limiti kontrollerine sahip (`main.rs`).
- Vault’a anahtar eklemeden önce venue izinleri sorgulanıyor; çekim yetkisi `ZERO_WITHDRAWAL_HARD_FAIL` ile reddediliyor ve anahtar/passphrase şifrelenerek saklanıyor (`main.rs`/`dca_exchange`). Bu, çekimsiz ve IP allowlist’li API anahtarı kullanma ihtiyacını ortadan kaldırmaz.
- Bybit/OKX mainnet geçidi için `DCA_ALLOW_MAINNET=1` açıkça gerekir; LIVE bot oluşturma Bybit ve OKX ile sınırlıdır; Binance LIVE kapalıdır. LIVE başlangıcında market verisi, enstrüman metası ve futures funding sertifikasyon kontrolü var. Compose varsayılanı testnet/demo ve mainnet kilitli; bu doğru bir başlangıç, fakat tek başına P0 güvenlik yapılandırmasını çözmez.
- `GET /api/health` DB durumu, son tick yaşı ve aktif/total bot bilgisini döndürüyor. Bunun gerçek healthcheck ve alarm hattına bağlanması gerekir.

Bu kontroller statik olarak görüldü; Rust derlemesi, gerçek venue cevapları, secret rotasyonu, restore ve LIVE order akışı bu oturumda çalıştırılmadı. “Zero withdrawal”, “fail-closed” veya “4096-bit rational” gibi ürün açıklamaları tüm işletim risklerinin sıfır olduğu anlamına gelmez.

## 5. UI/UX denetimi ve sektör kıyası

### 5.1 Mevcut kaynak kodunda doğrulanan iyileşmeler

Önceki UX belgelerinin bazı bulguları güncel kaynakta kapanmış görünüyor; bu **statik kaynak doğrulamasıdır**, ekran/görsel regresyon testi değildir:

| Alan | Mevcut kaynakta görülen | Sınır |
|---|---|---|
| Mobil gezinme | 4 birincil öğe + “Diğer” çekmecesi; erişilebilir gezinme durumu ve safe-area | Gerçek 375/390 px cihazda dokunma/taşma testi yapılmadı. |
| Filo tarama | Arama, durum filtresi, kart/tablo modu, sıralama ve `aria-sort` | Büyük bot sayısında performans ve ekran okuyucu turu yok. |
| Grafik | Equity grafiğinde klavye gezinmesi ve tablo alternatifi bulunuyor | Grafik kütüphanesinin tüm etkileşimleri, mobil/touch ve ekran okuyucu davranışı test edilmedi. |
| Hareket | `prefers-reduced-motion` CSS’i var; ana sayfa `useReducedMotion` kullanıyor | Hareketli tüm bileşenler cihaz ayarında görsel olarak kontrol edilmedi. |
| Backtest | `AbortController`, işlem ilerleme göstergesi ve iptal akışı var | Motorun iptal edilebilirliği/UI ile senkronu canlı browser testinde doğrulanmadı. |
| API secret | Ayarlarda göster/gizle ve panoya kopyalama kontrolleri var | Clipboard hata/izin ve screen-reader bildirimi test edilmedi. |
| Veri tazeliği | Kokpitte son başarılı güncelleme yaşı görünür | Ekranın tamamında her metrik için kaynak/zaman bilgisi aynı derecede görünür değil. |

### 5.2 Bu turdaki UX bulguları

**P1 — Performans/K/Z sunumu, tasarımdan önce doğruluk:** Kokpit headline’ı `totalEconomicPnL` adını ve net K/Z açıklamasını kullanıyor; formül funding’i içermiyor. Üstte “K/Z” kartı, finance olmayan kullanıcıya fonlama atlanmış olsa da tam ekonomik sonuç izlenimi verebilir. Kapanış ledger’ı olmaması da günlük performans/deal geçmişini zayıflatıyor. Önce tek net muhasebe sözleşmesini düzeltin, sonra KPI hiyerarşisini cilalayın.

**P2 — Küçük metin ve düşük kontrast:** `src/components/terminal` içinde statik aramayla `text-[9px]` 47, `text-[10px]` 259, `text-[11px]` 251 kullanım bulundu (toplam 557 class kullanımı; her biri mutlaka içerik hatası demek değildir). `#64748b` slate-500’ün `#0b0e14` ve `#10141d` zeminlerindeki kontrastı sırasıyla **4.06:1** ve **3.87:1** olarak hesaplandı. Normal boyutlu metin için WCAG AA 4.5:1 hedefinin altında kalan bu eşleşmeler, özellikle 9–11 px bilgi metninde okunabilirlik sorunu yaratabilir. Renk/zemin çiftlerini tüm gerçek UI token’larında otomatik tarayın; küçük boyutu yalnız dekoratif/micro-label için bırakın; anlamlı metinde daha açık ton ve en az 12 px taban ölçeğini değerlendirin.

**P2 — Görsel/a11y onayı eksik:** Bu oturumda 375/390 px mobil, tablet ve desktop ekranları, klavye-only akış, VoiceOver/TalkBack, dialog focus dönüşü, ekran okuyucu grafikleri, yüksek kontrast ve loading/error durumları çalıştırılmadı. `tsc`/lint başarıları bu kontrollerin yerine geçmez. Playwright + axe benzeri otomatik taramayı; gerçek klavye ve ekran okuyucu kısa turunu; 390×844, 768 px ve 1440×900 screenshot karşılaştırmasını release kapısına ekleyin.

**P0-2 ile bağlantılı yıkıcı işlem dili:** KILL butonunda iki aşamalı onay ve açık açıklama var; bu iyi bir desen. Ancak backend mevcut Add emirlerini iptal etmiyor ve fill-event sonrası Add emir kaskadı riski taşıyor. “Açıkları korur/yeni emir yok” anlatımı, gerçek davranışla uzlaştırılmadan acil durdurma kontrolü olarak güven vermemelidir. KILL’in “strateji sinyallerini durdurma”, “bekleyen emirleri iptal etme” ve “pozisyon kapatma” davranışlarını ayrı, doğrulanmış eylemler halinde gösterin.

### 5.3 Sektör araştırması — resmi repo README’leri (12.11.2026)

| Ürün/proje | Resmi README’de görülen desen | DCABot için uygulanabilir ders | Kanıt sınırı |
|---|---|---|---|
| [Freqtrade](https://github.com/freqtrade/freqtrade) | Dry-run ile başlamayı özellikle öneriyor; SQLite persistence, backtesting, optimizasyon, WebUI/Telegram ve performans/K/Z raporlarını listeliyor. | Paper/testnet → gözlem → sınırlı LIVE aşamalarını ürün akışına yerleştirin; dashboard’da gerçekleşen performansı tek ve kalıcı ledger’dan gösterin. | Repo README’si okundu; çalışan Freqtrade UI/hesap akışı karşılaştırmalı test edilmedi. |
| [FreqUI](https://github.com/freqtrade/frequi) | Vue/Nuxt UI; doğru çalışması için Freqtrade API’si gerekir; API/CORS/proxy yapılandırması ayrı bir entegrasyon sorumluluğudur. | UI ile motor arasındaki sınırı koruyun; DCABot’un server-side proxy yaklaşımı iyi mimari temel, fakat auth varsayılanı mutlaka açık olmalı. | README mimari notu; görsel/ürün UX incelemesi değil. |
| [Hummingbot Dashboard](https://github.com/hummingbot/dashboard) | Çoklu bot orkestrasyonu, backtest/optimizasyon, paper/live deploy, performans izleme ve kimlik bilgisi yönetimi öne çıkıyor. README auth’un varsayılan kapalı olduğunu ve örnek `admin/abc` türü zayıf kimlik bilgilerini de söylüyor. | Filo ve deploy akışları ürün benchmark’ı olabilir; ancak bu projenin güvensiz auth varsayılanları örnek alınmamalı. | Fonksiyon listesi README’den; canlı UI/işlem davranışı test edilmedi. |
| [Hummingbot API](https://github.com/hummingbot/hummingbot-api) / [Deploy](https://github.com/hummingbot/deploy) | Üretimde Tailscale/özel overlay önerisi, API’yi public portta açmama, API/DB için loopback veya seçilmiş arayüz ve ayrı güçlü kimlik bilgileri yönlendirmesi. | Tek kullanıcı terminalini VPN/localhost/TLS arkasında tutun; `3000` portunu doğrudan internete açmayın; health/metrics erişimini ayrı sınırlandırın. | Operasyonel öneriler resmi README’den; bağımsız güvenlik sertifikası olarak yorumlanmamalı. |

`docs/UI_UX_GORSEL_DENETIM_2026-10-30.md` içindeki 3Commas/Pionex/Bitsgap kıyas bağlantıları tarihsel kaynaklardır; bu turda bu ürünlerin güncel arayüzü, fiyatlandırması veya benchmark ölçümü açılıp doğrulanmadı. Bu nedenle güncel sektör UI kanıtı olarak sunulmuyor.

## 6. Uygulama sırası ve kabul kapıları

### Faz A — Güvenli erişim sınırı (P0, ilk iş)

1. Compose’daki sabit secret fallback’lerini kaldır; boş/varsayılan secret’ta startup fail olsun. API, vault ve UI için farklı güçlü secret kullan; yönetilen secret saklama ve rotasyon prosedürü yaz.
2. `DCA_REQUIRE_AUTH=1`; güçlü, benzersiz UI parolası; session/vault/API secret ilişkisini ve parola rotasyonunu test et.
3. `3000` portunu localhost, VPN arayüzü veya TLS reverse proxy arkasına bağla. Engine `3002`’yi dışarı yayınlama; metrics’i yalnız monitor ağından erişilebilir yap.
4. LAN/uzak ağdan login’siz API isteğinin reddedildiğini, dış firewall’da yalnız TLS/VPN portunun açık olduğunu kanıtla.

### Faz B — Şalter ve risk kapıları (P0/P1)

1. KILL/FREEZE devredeyken kaskad ve fill tepkilerinden Add emir çıkmadığını garantile; mevcut borsa giriş emirlerini iptal et ve uzlaştır; reduce-only çıkış davranışını belgeleyip test et.
2. Portföy notional hesabına LIVE açık emir rezervlerini ekle; cache/venue/NAV belirsizliğinde Add’i fail-closed engelle.
3. `DCA_PORTFOLIO_MAX_DAILY_LOSS_PCT` ayarını gerçek hesap-geneli net kayıp kesicisine bağla ya da kaldır; bot başına breaker’dan açıkça ayır.
4. `cert_acknowledged` istisnasının operatör politikasını karara bağla; gerekiyorsa ikinci onay/reason ve audit zorunlu kıl.

### Faz C — Muhasebe ve karar ekranları (P1)

1. DCA/Hedge/Grid için kalıcı, idempotent, decimal-string deal ledger’ı tamamla; kapanışları, fees ve funding’i atomik yaz.
2. Summary, fleet, deal history ve dashboard aynı sunucu tarafı net P&L sözleşmesini kullansın; funding ve hedge bacaklarını açıkça kapsasın.
3. STOPPED botlar ve geçmiş kapanışlar toplamda kaybolmasın; API’nin eksik veri durumunu `null`/kapsama uyarısı ile belirt.
4. Ledger ↔ bot snapshot ↔ equity mutabakat testleri; TP/SL/likidasyon/manuel kapatma/funding iadesi ve restart sonrası tekrar sayım testleri ekle.

### Faz D — Paketleme, dayanıklılık ve CI (P1)

1. Node 22 builder/runtime eşleşmesi; `.dockerignore`; non-root runner; `tini`/supervisor; gerçek engine readiness healthcheck.
2. Rust + Next + Docker imajını CI’de üret; testnet/PAPER mock E2E, güvenlik/dependency taraması ve migration/restore testlerini PR kapısı yap.
3. SQLite online backup’ı kalıcı/off-host sakla; yedek şifrele; periyodik restore drill ve RPO/RTO alarmı kur.
4. Venue erişim hatası, stale tick, order reconciliation/fill ingest hatası, kill state, günlük DD ve disk doluluğu için ölçülebilir alarm/incident runbook ekle. `/metrics` ve health’i public internetten uzak tut.

### Faz E — Kademeli canlıya geçiş

1. Kod/CI ve testnet’te her strateji türü ve her LIVE destekli venue için emir-place/cancel/fill/restart mutabakatını doğrula. Binance LIVE kapalı kalmalıdır; kaynak bunu fill reconciliation doğrulanmadığı için kısıtlıyor.
2. Paper ve venue demo ortamında; gerçek borsa API anahtarlarında çekim izni kapalı, mümkünse IP allowlist’li, ayrı subaccount ve yalnız gerekli spot/contract trade izinleri kullan.
3. Demo’da kill, network kopması, venue timeout/rate-limit, kısmi fill, duplicate execution, restart, stale DB ve backup restore tatbikatı yap.
4. Mainnet’e ancak P0/P1 kabul testleri geçtikten sonra `DCA_ALLOW_MAINNET=1` gibi açık operatör onayıyla, tek bot/düşük notional canary ve sürekli izleme altında geç. Kademeli büyüt; ilk anomali/uzlaştırma farkında yeni Add emirlerini durdur ve incident runbook’u uygula.

## 7. Go / No-Go matrisi

| Kullanım | Karar | Gerekçe |
|---|---|---|
| Yerel geliştirme ve statik UI doğrulaması | **Koşullu GO** | UI build/type/lint bu turda geçti; Rust motoru ve tarayıcı E2E bu turda doğrulanmadı. |
| Paper/testnet’i yalnız localhost/VPN’de deneme | **Önce secret/auth ve testnet smoke** | Compose varsayılanlarını üretim gibi kullanmayın; engine readiness ve shutdown doğrulanmalı. |
| LAN’a/internete açık mevcut Compose varsayılanı | **NO-GO** | UI auth kapalı, sabit API/vault anahtarları etkin ve port tüm host arayüzlerine yayınlanıyor. |
| Gözetimsiz Bybit/OKX mainnet LIVE | **NO-GO** | KILL sonrası fill/Add yolu, portföy cap fail-open/eksik emir rezervi, daily cap belirsizliği, P&L ledger eksikleri ve Docker/Rust/E2E doğrulama boşluğu. |
| Binance LIVE | **Kapalı kalmalı** | Mevcut kaynak açıkça LIVE kanalını kapalı tutuyor; bu oturumda adaptör fill mutabakatı doğrulanmadı. |

## 8. Dış kaynaklar ve inceleme sınırları

- [Freqtrade GitHub](https://github.com/freqtrade/freqtrade) — README: dry-run, backtesting, SQLite persistence, WebUI/Telegram, performans raporları.
- [FreqUI GitHub](https://github.com/freqtrade/frequi) — README: Vue/Nuxt UI ve çalışan Freqtrade API gereksinimi.
- [Hummingbot Dashboard GitHub](https://github.com/hummingbot/dashboard) — README: orchestration, backtesting/optimization, paper/live deploy, performance ve credentials; auth varsayılanlarına dair uyarı.
- [Hummingbot API GitHub](https://github.com/hummingbot/hummingbot-api) — README: private networking/Tailscale, public port açmama ve bind-address tavsiyeleri.
- [Hummingbot Deploy GitHub](https://github.com/hummingbot/deploy) — Docker tabanlı kurulum akışı.
- Tarihsel iç benchmark: `docs/UI_UX_GORSEL_DENETIM_2026-10-30.md`; bu rapordaki ticari ürün kıyasları güncel UI testi olarak kullanılmadı.

**Nihai not:** Bu denetim kod ve yapılandırma risklerini önceliklendirir; kaynak metinler tek başına kriptografik/finansal doğruluk veya canlı borsa davranışının kanıtı değildir. Mainnet kararı için P0/P1 maddelerinin düzeltilmesi, Rust/Docker/E2E kapılarının gerçek çalıştırılması ve operatörün kontrollü demo/canary tatbikatı gerekir.
