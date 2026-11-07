# UI/UX Görsel Denetim Raporu — 2026-10-30

> **FAZ 25 — UYGULAMA TAMAM (2026-10-30, aynı gün):** UIX-18..42'nin TAMAMI uygulandı ve tarayıcıda görsel olarak doğrulandı. `tsc --noEmit` exit 0, `eslint` 0 error/0 warning. Doğrulama kanıtları: Filo (arda1 ilk sırada, "PLANLI: 01 Kas 2026 02:20:00", HALT pill + "yeniden başlatma bekliyor", politika şeridi tek satır, Sil etiketli), Studio arda1 (K/Z birinci kart kırmızı, Deal Durumu faz-bazlı "Açık deal yok (faz: Cooldown)…"), Hesaplayıcı (fiyatlar 2 ondalık, "SL ALTINDA" nötr gri + kırmızı yalnız gerçek çatışmada), Lab (haber bloğu yok, YZ tek satır, overflow yok), Ayarlar (YZ Sağlayıcısı tek panel), Sihirbaz (✓ zinciri onest, CTA "ÖNCE STRATEJİ SEÇ"), footer tek satır "KORUMA: 8 ZORUNLU KAPI · TAM-RASYONEL MUHASEBE". Nota UIX-39: grafik katmanları (AVG/TP/SL/sonraki güvenlik/LIQ) Studio'da ZATEN mevcutmuş — denetimde görünmemesinin nedeni arda1'de açık deal olmamasıydı; kod `studio.tsx chartLines`. Nota UIX-35: motorun builtin sağlayıcısı bu kurulumda kullanılamaz → düğmeler yalnız kullanılabilir sağlayıcıda görünür, yönlendirme satırı ekler.

**Kapsam:** Çalışan üretim instance'ı üzerinde canlı görsel tur (http://127.0.0.1:3000, Chrome DevTools, 1440×900 + 390×844 mobil).
**Ekranlar:** Kokpit (2 kademe), Bot Filosu (üst+orta+alt tape), Studio/arda1 (Genel, Ladder, Özet), Yeni Bot (1. adım), Hesaplayıcı, Backtest Lab (üst+alt), Bilgi, Ayarlar.
**Kanıt:** `.local-runtime/ui-audit-2026-10-30/01..07*.png` (gitignored — kalıcı gözden geçirme klasörü).
**Önceki iş:** UIX-1..17 (Faz 20–23) kapanmış; bu rapor **yeni nesil** bulguları **UIX-18**'den numaralandırır.
**Yöntem:** A11y ağacı + ekran görüntüsü çifti okundu; her bulgu ekranda gözlemlenen gerçek veriyle desteklenir (uydurma yok). Sektörel kıyas 3Commas / Pionex / Bitsgap genelgeçer desenleriyle yapıldı (kaynaklar sonda).

---

## 1. Genel Değerlendirme

**Güçlü yanlar (korusu gerekenler):** koyu terminal teması tutarlı; num/monospace veri tipografisi doğru seçim; mobil ≤5 alt-bar kuralı uygulanmış (UIX-1 mirası); veri tazeliği rozeti (UIX-6) sektörde az görülen dürüstlük sinyali; fleet tabanında "Son Dolumlar (tape)" bot kimlikli — doğru desen.

**Kök sorun deseni (3 tekrarlayan tema):**
1. **Mühendislik dili kullanıcıya sızıyor** — modül kodları (M05/M07/M08/M17/M18/M20), gate kodları (W1/W8a/W8b/G_MIN), ham epoch, `sent:false`, "invariant", "exact rational çekirdek". Ürün kendi mimarisini kullanıcıya anlatıyor; kullanıcı işlem kararını değil mimariyi okuyor.
2. **Birinci kademede işlem kararı verisi değil, dekor var** — kokpitte SSE ham olayları, Lab'da haber akışı, her kartta aynı "3 KORUMA UYARI". Bot PnL'i (kullanıcının #1 metriği) Genel sekmesinde yok, Özet sekmesine saklanmış.
3. **Aynı konfigürasyon birden çok yerde** — YZ sağlayıcı/anahtar 3 ayrı ekranda; haber akışı 2 ayrı ekranda; gates bilgisi 3 ayrı yerde.

---

## 2. Bulgu Kataloğu

Önem: **P0** = bilgi doğruluğu/dürüstlük hatası, **P1** = yerleşim/IA hatası, **P2** = cilalama.

### P0 — Bilgi doğruluğu

**UIX-18 · Ham epoch milisaniyesi UI'a sızmış** — Filo, `t12-sched` kartı: `BAŞLANGIÇ: SCHEDULED @ 1793488800000ms`. Tarih/saat insan formatında gösterilmeli ("30 Eki 05:00"). *Kanıt: 03-filo-top.png a11y düğümü 2_351.*

**UIX-19 · Kokpit "Canlı Olay Akışı (SSE)" ham mühendislik günlüğü** — Satırlar `EQUITY equity $1,978.67 · qty 0.00966000 @ 80,797.60 · realized +$4.09`; **hangi bot olduğu yazmıyor**, aynı değerler dakika dakika tekrar akıyor (09:47/48/49'da aynı equity). Trader için işlenebilir sinyal yok; kâr alanı işgal ediyor. Çözüm: (a) olayları FILL / TP_HIT / SL_HIT / RISK / DEAL_CLOSED gibi işlem-önemli türlere filtrele, (b) her satıra bot adı, (c) equity tick'lerini ya gizle ya 5 dk'da bir özetle. *Kanıt: 02-kokpit-bottom.png.*

**UIX-20 · "Deal Durumu" metni fazla aykırı** — arda1 FAZ=`Cooldown`, DEAL ID=`—` iken metin: *"Tek aktif deal invariant: base doldu, 0 güvenlik emri tetiklendi. Sonraki güvenlik L— seviyesinde — fiyatında bekliyor."* Açık deal yokken açık-deal cümlesi gösteriliyor; "invariant" jargonu. Metin faza göre üretilmeli (Cooldown: "X bar sonra yeni base emri değerlendirilecek"). *Kanıt: 06-studio-genel-mid.png.*

**UIX-21 · Botun kendi PnL'i Genel sekmesinde yok** — 16 KPI kartında FAZ/DEAL ID/POZİSYON/TRAILING/FUNDING/NAKİT… var ama **gerçekleşen kâr/zarar ve açık deal Unrealized PnL'i yok**; `$-306.07` yalnız Özet sekmesinin 5. kartında. Sektör standardı: bot PnL birinci kademe (Bitsgap "Bot Profit", 3Commas bot kartı P&L). Çözüm: Genel KPI şeridine "K/Z (gerçekleşen)" + "Açık Deal PnL %" kartları; kötü performanslı botta negatif değer kırmızı vurgu. *Kanıt: 05-studio-genel-top.png vs 07-studio-ozet.png.*

### P1 — Yerleşim / IA

**UIX-22 · Haber akışı iki ekranda, Lab'da sonuçların üstünü kapatıyor** — "Piyasa Nabzı" (Kokpit) aynı listeyle "HABERLER · BTCUSDT" olarak Backtest Lab'ın **config ile sonuç alanı arasına** girmiş. Backtest çalıştıran kullanıcının ilgilendiği şey dağılım/equity eğrisi, Binance duyurusu değil. Çözüm: Lab'dan tamamen çıkar (Kokpit tek yer). *Kanıt: Backtest Lab üst/alt görüntüleri.*

**UIX-23 · YZ sağlayıcı yapılandırması 3 yerde dağınık** — (1) Kokpit Piyasa Nabzı altında katlanmış panel, (2) Studio Özet "Hafıza · AI Durum Özeti" (provider/model/API anahtar/base URL), (3) Lab "YZ Yardımcı · Teşhis & Sohbet". Aynı anahtar-güvenlik metni üç kez farklı ifadeyle. Çözüm: sağlayıcı/model/anahtar **Ayarlar'da tek panel**; Kokpit/Studio/Lab yalnızca "kullan" tarafı (Özet Üret / SOR) kalsın. *Kanıt: 02, 07, Lab-alt görüntüleri.*

**UIX-24 · "3 KORUMA UYARI" 13 kartın hepsinde** — Küresel piyasa advisory sayısı her bot kartına kopyalanıyor; hepsinde aynı olduğu için bilgi değeri sıfır, sarı alarm yorgunluğu yaratıyor (arda1 dahil). Çözüm: tek küresel şerit ("Piyasa koruması: 3 advisory aktif — detay") filo başına bir kez; kartta yalnızca **bota özgü** advisory sayısı sıfırdan farklıysa göster. *Kanıt: 03/04-filo.*

**UIX-25 · Filoda sıralama/filtre/arama yok** — 13 bot için tek kontrol "🛡 Hedge (2)" çipi + kart/liste geçişi. En büyük kayıp (`arda1 -$306.07`) listede gömülü; sıralama isme göre gibi. Çözüm: sıralama (K/Z, sembol, durum), durum filtresi (Running/Paused/Frozen/Halted), sembol araması; varsayılan sıra = mutlak K/Z (kayıp en üstte). *Kanıt: 03-filo-top.png.*

**UIX-26 · "RUNNING + KERNEL HALT" çelişkili rozet** — `t9-pinemacd` kartında aynı anda RUNNING (yeşil) ve KERNEL HALT (kırmızı). Kullanıcıya doğru birincil durum HALT; RUNNING alt-durum olarak "kernel yeniden başlatmayı bekliyor" açıklamasıyla verilmeli. Ayrıca HALT'lı bot için Duraklat/Dondur yerine önce "Neden durdu?" bağlantısı düşünülmeli. *Kanıt: 04-filo-mid.png.*

**UIX-27 · Killswitch kartı belge gibi yazılmış** — "Risk İlkeleri" bloğu gate listesi + "kullanıcı patron, risk kendi sorumluluğunda" geliştirici-sohbet tonu. Tehlike bölgesi kartında olması gereken: tek net cümle + KILL'e giden net yol; ilke listesi Bilgi/SSS'ye. *Kanıt: 02-kokpit-bottom.png.*

**UIX-28 · Gates/rational-core esnekliği 3 yerde tekrarlıyor** — (1) sidebar kutusu "8 FAIL-CLOSED GATES / 4096-bit RATIONAL CORE / M01–M20 MODULES", (2) footer "8 FAIL-CLOSED GATES ACTIVE" + "4096-BIT RATIONAL ARITHMETIC · ZERO FLOAT", (3) killswitch kartı "8 gate aktif". Mühendislik kimliği footer'da **tek** satırda insan diliyle tut ("Koruma katmanı: 8 kapı · tam-rasyonel muhasebe"); sidebar kutusu yerine filo özeti (açık deal sayısı, bugünkü işlem) koy. *Kanıt: tüm ekranlar.*

**UIX-29 · Sihirbaz ilerleme çipleri mantık hatası** — 1. Strateji ve 2. Borsa & Sembol seçilmeden "✓ Parametreler & Koşul" işaretli görünüyor; "✓ Risk Motoru" ikinci satıra sarıyor; sağ panelde "BOTU BAŞLAT" + "Bot Adı" 1. adımdan itibaren görünür (adım akışıyla CTA çakışıyor). Çözüm: çip durumu gerçek tamamlanmaya bağla; Risk Motoru çipi ana akışa 6. adım olarak girsin; sağ panel CTA adım içeriğine göre "Devam" ↔ "Başlat" değişsin. *Kanıt: Yeni Bot görüntüsü.*

**UIX-30 · Backtest Lab 1440px'te yatay overflow** — "SİHİRBAZA TAŞI" düğmesi viewport dışına taşıyor, sayfada yatay kaydırma çubuğu çıkıyor. Grid min-width'i düzeltilmeli. *Kanıt: Lab üst görüntüsü (sağ kenar kesik).*

**UIX-31 · Hesaplayıcı: varsayılan yükleme kırmızı duvarı + ondalık gürültü** — Varsayılan konfigde (SL %3, deviation %2, 5 güvenlik) tüm 5 ladder satırı `M20 ULAŞILAMAZ` kırmızı; kullanıcı sayfayı açar açmaz "her şey bozuk" algısı (oysa SL altı seviyelerin erişilemez olması matematiksel olarak beklenen). Fiyatlar `76975.20000000`, WAC `77682.70709220` — okunmaz. Çözüm: (a) tick hassasiyetinde biçim (76 975.20), (b) "M20" rozeti + kırmızı yalnız gerçek çatışmada; SL-altı seviye normal gri + "SL altı" notu, (c) üst başlıktaki "exact rational çekirdek (M02 ladder, M15 mesh…)" alt yazısı tek cümle insan diline. *Kanıt: Hesaplayıcı görüntüsü.*

**UIX-32 · "Tahsis Dağılımı" tek varlıkta ölü alan** — %100 BTCUSDT donut + kart altı büyük boşluk. Tek varlıkta donut bilgi taşımaz. Çözüm: kartı filo özetiyle birleştir (açık deal 8 · bugünkü fill 64 · marj kullanımı) veya donutu marj/kullanılabilir nakit ayrımı yap. *Kanıt: 02-kokpit-bottom.png.*

### P2 — Cilalama

**UIX-33 · Dil tutarlılığı** — "Botlarım · My Bots", "KÂR/ZARAR · PROFIT", "Fill'ler", "FAZ: Cooldown", "TRAILING Off", "Deal Durumu", "Ladder", "Dry-Run". Tek dil politikası: etiketler TR, terim sözlüğü Bilgi'de (Fill=Emir Dolumu, Ladder=Güvenlik Merdiveni, Cooldown=Bekleme). Öncelikli: görünür başlıklardan EN kopyasını kaldır.

**UIX-34 · Yerel ayar tutarsızlığı** — Aynı kartta `+0,83%` (virgül) ile `$-281.05` (nokta). Tüm sayısal gösterim tek locale'e (nokta) bağlanmalı.

**UIX-35 · "TR" rozeti belirsiz ve ölü** — Sağlayıcı "Yerleşik (paketlenmedi)" iken her haber satırında tıklanamaz-ölü "TR" düğmesi. Sağlayıcı yoksa gizle; varsa "Özet (TR)" yazısıyla.

**UIX-36 · Piyasa Nabzı başlığı içerikle çelişiyor** — "· BTCUSDT" diyor ama liste borsa-geneli duyuru (bStocks, TradFi sözleşmeleri). Ya başlık "Borsa Duyuruları" olsun ya duyurular sembole göre filtrelesin; sentiman rozetleri (+0.284, ↑1 0↓) açıklamasız — tooltip/bilgi ikonu.

**UIX-37 · İkon-tek silme düğmesi** — Kartta Kapat'ın yanında etiketsiz 🗑. "Kapat" (deal kapatma) ile "Sil" (bot+geçmiş) ayrımı en kritik yıkıcı ayırtım; sil düğmesine "Sil" etiketi + iki aşamalı onay (mevcut dialog var, iyi) yeterli.

**UIX-38 · Ladder boş durumu yanıltıcı** — "Bu strateji için merdiven üretilmedi" — asıl neden aktif deal olmaması. Faz-farkındalı mesaj: "Açık deal yokken merdiven üretilmez; base emri dolduğunda X seviye burada listelenir."

**UIX-39 · Studio grafiğinde strateji katmanı yok** — Grafik çıplak mum; WAC/TP/SL/güvenlik seviyeleri mumların üzerinde çizilmiyor (yalnız bayat WAC kesser çizgisi var). Pionex grid çizgilerini, 3Commas TP/SL çizgisini grafikte gösterir. Studio'nun en yüksek UX kazancı bu: fiyat çizgisi + TP + sonraki güvenlik seviyesi.

**UIX-40 · Maruziyet/Marj kartı belirsiz** — "L $3,230 / S $3,086" neyin değeri net değil (açık pozisyon notional'ı). Marj kullanım % + likidasyon mesafesi + pozisyon notional/toplam özkaynak oranı net etiketle.

**UIX-41 · Özkaynak kartında borsa fiyat karışıyor** — "24s Değişim BTCUSDT +0,83%" özkaynak kartında; bu borsa verisi, portföy verisi değil. Ayrı satır/kart ya "portföyün 24s değişimi" olarak hesaplanmış haliyle.

**UIX-42 · Sidebar kutusu Next.js dev düğmesiyle çakışıyor** (yalnız dev) — geliştirme overlay'i sidebar sistem kutusunu örtüyor; kutu içerik değişince zaten kalkacak (bkz. UIX-28).

---

## 3. Sektörel Benchmark Karşılaştırması

| Konu | 3Commas | Pionex | Bitsgap | DCABOT bugün | Kapanacak bulgu |
|---|---|---|---|---|---|
| Bot PnL birinci kademe | Bot kartında P&L | Grid profit bakiye ayırımlı | "Bot Profit" ana metrik | Yalnız Özet sekmesi | UIX-21 |
| Deal/SO ilerleme görselleşmesi | Kartta SO progress bar | Grid seviyeleri çizgili | Bot kartında ilerleme | Metin "SO 5/2" | UIX-39 |
| Grafikte strateji katmanı | TP/SL çizgileri | Grid bantları çizili | Seviye çizgileri | Yok | UIX-39 |
| Filo sıralama/filtre | Çoklu filtre+arama | Bot türü sekmeleri | Durum/tür filtresi | Tek "Hedge" çipi | UIX-25 |
| Haber akışı konumu | Yok (ayrı sinyaller) | Yok | Yok | Kokpit + Lab ortası | UIX-22 |
| Konfig yer merkezliği | Ayarlar tek yer | Uygulama içi tek yer | Ayarlar tek yer | YZ anahtarı 3 yerde | UIX-23 |
| Jargon seviyesi | Kullanıcı dili | Kullanıcı dili | Kullanıcı dili | M05/W8a/G_MIN/epoch | UIX-18/19/28/31 |
| Tehlike bölgesi tonu | Net tek cümle | Net tek cümle | Net tek cümle | Belge+sohbet tonu | UIX-27 |

Sonuç: DCABOT'un veri doğruluğu/matematik dürüstlüğü sektörün üstünde (exact-rational, fill etiketleri, mutabakat panelleri hiçbir rakipte yok); **sunum katmanı ise sektörün altında** — çünkü ürünün anlattığı şey işlem değil, mimari.

## 4. Önerilen Uygulama Sırası

1. **Dilig paketi (P0, ~yarım gün):** UIX-18 epoch formatı · UIX-20 faz-bazlı deal metni · UIX-21 Genel'e PnL kartları · UIX-26 HALT birincil durum.
2. **Gürültü temizliği (P1, ~1 gün):** UIX-19 SSE filtre+bot adı · UIX-24 tek koruma şeridi · UIX-28 tek esneklik satırı · UIX-22 Lab'dan haber çıkışı.
3. **IA konsolidasyonu (P1, ~1 gün):** UIX-23 YZ ayarları Settings'e · UIX-25 filo sıralama/filtre · UIX-29 sihirbaz çip akışı.
4. **Sunum cilası (P1/P2, ~1 gün):** UIX-31 hesaplayıcı biçim+m20 tonu · UIX-39 grafik katmanları · UIX-30 overflow · UIX-32/33/34/35/36/37/38/40/41.

Kaynaklar (benchmark): [3Commas — DCA Bot](https://3commas.io), [Bitsgap — Bot Profit dashboard](https://bitsgap.com), [Pionex — grid profit ayrımı](https://www.pionex.com/blog/what-mean-is-normal-grid-and-reverse-grid), [Pionex — bot parametre akışı](https://www.pionex.com/blog/trying-to-star-the-pionex-grid-bot-but-when-i-click-create-this-text-is-comming-on-the-screen-please-input-the-following-parameters-1-total-investment-what-ca-i-d).
