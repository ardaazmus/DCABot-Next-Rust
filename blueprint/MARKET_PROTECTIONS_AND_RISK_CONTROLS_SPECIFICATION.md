# DCABOT PİYASA KORUMALARI, BİLGİLENDİRİLMİŞ RİSK VE KULLANICI ÖZERKLİĞİ SÖZLEŞMESİ
**Sistem:** DCABOT Kantitatif Çekirdek & Risk Motoru  
**Yeni İlke:** **"Kullanıcı Kendi Riskini Taşır" (Informed Risk & User Autonomy)**  
**Temel Tasarım Değişikliği:** İşlem açmayı veya bot kurulumunu zorla engelleyen (hard-block) kurallar, **açıklayıcı uyarılar (Advisory Warnings), net risk rozetleri ve kullanıcı onaylı geçersiz kılma (Override/Toggle)** mekanizmasına dönüştürülmüştür. 

---

## 1. YENİ RİSK FELSEFESİ: KATI BLOKAJ YERİNE AÇIK BİLGİLENDİRME

Geleneksel kurumsal sistemler kullanıcıyı kısıtlarken, bu yeni modelde **patron kullanıcıdır**. 
- Sistem, kullanıcının yerine karar verip işlemleri **engellemez**.
- Bunun yerine sistem; yapılan hareketin **ne işe yaradığını**, **hangi matematiksel riski doğurduğunu** ve **muhtemel sonuçlarını** bot ekranında açıkça yazar.
- Kullanıcı riski bilerek ve isteyerek üstlenir (`Acknowledge & Proceed`).

```
┌──────────────────────────────────────────────────────────────────────────┐
│                   KORUMALARIN 2 TEMEL SINIFI                             │
├──────────────────────────────────────────────────────────────────────────┤
│ 1. UYARI & GEÇERSİZ KILINABİLİR KORUMALAR (Advisory / User Override)     │
│    - Varsayılan olarak uyarır ve korur, ancak kullanıcı kapatabilir.     │
│    - Bot üzerinde ne işe yaradığı ve riski Türkçe açıkça yazar.          │
│    - Örn: M20 SL-Merdiven, Grid Komisyon Kırıcı, Seans/Makro Kara       │
│      Pencere, Açılış Salvo Tamponu, Rejim Filtresi, Çoklu Emir.          │
├──────────────────────────────────────────────────────────────────────────┤
│ 2. FİZİKİ & GÜVENLİK DEĞİŞMEZLERİ (Sistem Güvenliği)                     │
│    - Borsa API kuralları ve cüzdan güvenliği gereği esnetilemeyenler.    │
│    - Örn: Sıfır-Çekim (Zero-Withdrawal) ve Olmayan Parayı Harcamama.     │
└──────────────────────────────────────────────────────────────────────────┘
```

---

## 2. ESNETİLEN VE UYARI MODUNA ALINAN TÜM KORUMALARIN LİSTESİ

Aşağıdaki korumalar eskiden botun çalışmasını veya işlem açmasını durdururken, artık **bot ekranında ne işe yaradığı yazan, riskini açıklayan ve kullanıcının onayına bırakılan** mekanizmalara çevrilmiştir.

---

### 2.1 M20: Stop-Loss ile Merdiven Çakışması Koruması
* **Eski Davranış:** SL, güvenlik merdiveninin toplam sapmasından dar girilirse bot oluşturulamaz veya emir reddedilirdi.
* **Yeni Davranış (Uyarı Rozeti):** İşlem açılması engellenmez. Bot kartında ve sihirbazda turuncu uyarı rozeti çıkar.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Stop-Loss seviyeniz ortalama maliyete göre hesaplanır. Eğer merdiven henüz 2. kademedeyken fiyat SL seviyenize değerse, 3, 4 ve 5. kademedeki güvenlik emirleri dolamadan bot zararla stop olur."*
* **Kullanıcının Üstlendiği Risk:**
  > *"DİKKAT: 3, 4 ve 5. güvenlik emirleri için cüzdanınızda ayırdığınız sermaye hiçbir zaman kullanılamaz (ölü sermaye kilitlenir). Stratejiniz erken pes eder."*
* **Arayüz Kontrolü:** `[X] Bu riski anlıyorum, dar Stop-Loss ile devam et.`

---

### 2.2 Grid Komisyon Kırıcı ($g_{\min}$ Fee-Breaker Guard)
* **Eski Davranış:** Izgara adımı komisyon oranından ($f_{\text{buy}} + f_{\text{sell}}$) dar ise grid kurulamazdı (`Formation Rejected`).
* **Yeni Davranış (Risk Bildirimi):** Grid kurulumuna izin verilir, ancak kâr marjının negatife düşebileceği belirtilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Izgara adım aralığınız her al-sat işleminde borsaya ödenecek komisyon toplamından daha dar ayarlandı."*
* **Kullanıcının Üstlendiği Risk:**
  > *"RİSK: Her ızgara turunda brüt kâr elde etseniz bile, borsa komisyonu düşüldükten sonra net bakiyeniz erir (komisyona çalışma riski). Bu ayar yalnızca borsa komisyonsuz (zero-fee) kampanyası yapıyorsa veya borsa puanı/hacim kasmak istiyorsanız mantıklıdır."*
* **Arayüz Kontrolü:** `[ ] Negatif komisyon marjı riskini onaylıyorum.`

---

### 2.3 Gate 7: Piyasa Seansı Kapalıyken Alım Yapma (Market Session)
* **Eski Davranış:** Hisse veya Emtia piyasası kapalıyken yeni pozisyon açma (`ADD`) emri fail-closed reddedilirdi.
* **Yeni Davranış (Seans Dışı İşlem Tercihi):** Kullanıcı "Seans dışı işlem izni ver" seçeneğini aktif ederse bot emir göndermeyi dener.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"İşlem yaptığınız borsa veya piyasa şu an normal çalışma saatleri dışındadır (gece seansı, hafta sonu veya tatil)."*
* **Kullanıcının Üstlendiği Risk:**
  > *"RİSK: Seans dışı işlemlerde likidite çok düşüktür; alış-satış makası (spread) çok geniştir ve emirleriniz piyasa açıldığında çok daha kötü bir fiyattan doldurulabilir (Slippage / Fiyat kayması riski)."*
* **Arayüz Kontrolü:** `Açılış Seansı Dışı Alımlara İzin Ver (Geniş spread riskini üstlen)`

---

### 2.4 Gate 8a: Açılış Salvo Tamponu (09:30–09:35 EST)
* **Eski Davranış:** Piyasa açılışının ilk 5 dakikasında yeni güvenlik emirleri zorunlu olarak bekletilirdi.
* **Yeni Davranış (Açılış Tamponunu Kapatma Opsiyonu):** Kullanıcı isterse ilk 5 dakika korumasını kapatıp anında emir gönderebilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Piyasa açılışının ilk 5 dakikasında kurumsal açılış iğneleri yaşanır; spreadler ortalama 550 baz puana kadar açılabilir. Tampon bu açılış çılgınlığında alım yapmayı erteler."*
* **Kullanıcının Üstlendiği Risk:**
  > *"RİSK: Açılış tamponunu kapatırsanız, ilk 300 saniyedeki sahte volatilite iğnelerinde en tepeden veya en dip spread aralığından maliyetlenebilirsiniz."*
* **Arayüz Kontrolü:** `Açılış Tamponunu Devre Dışı Bırak (İlk 5 dakika içinde anında emir gönder)`

---

### 2.5 Gate 8b: Hafta Sonu Gap-Down Konsolidasyonu
* **Eski Davranış:** Piyasa hafta sonu boşluğuyla (%10 aşağıda) açıldığında atlanan seviyeler zorunlu olarak tek bir VWAP dolumda toplanırdı.
* **Yeni Davranış (Konsolidasyon Tercihi):** Kullanıcı isterse konsolidasyonu kapatıp, merdivenin her kademesini tek tek bağımsız emir olarak işletmeyi seçebilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Pazar gecesi veya pazartesi açılışında fiyat birden fazla alım seviyenizi atlarsa, tüm atlanan miktarlar açılış fiyatından tek dolumda birleştirilerek ortalama maliyet hemen tabana çekilir."*
* **Kullanıcının Üstlendiği Risk:**
  > *"Konsolidasyonu kapatırsanız; atlanan her seviye için ayrı ayrı komisyon ödersiniz ve ortalama maliyetiniz daha yavaş iyileşir."*

---

### 2.6 Makro Kara Pencere (Macro Blackout Window - CPI / FOMC)
* **Eski Davranış:** Kritik ekonomik haberlerin 15 dakika öncesi ve sonrasında yeni alımlar motor tarafından engellenirdi.
* **Yeni Davranış (Haberde Alım İzni):** Kullanıcı isterse yüksek volatilite haberlerinde de botun alım yapmasına izin verebilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Fed Faiz Kararı (FOMC) veya Enflasyon (TÜFE/CPI) gibi yüksek etkili haber anlarında piyasa her iki yöne de sert fitiller atar. Kara pencere bu anlarda alımı durdurur."*
* **Kullanıcının Üstlendiği Risk:**
  > *"RİSK: Haber anında bot alım yaparsa, birkaç saniye içinde stop olma veya merdivenin hızla tükenmesi riskiyle karşılaşırsınız."*
* **Arayüz Kontrolü:** `Haber Anında İşlem Yapmaya Devam Et`

---

### 2.7 Piyasa Rejim Filtresi (Regime Abstention - Faz 10)
* **Eski Davranış:** Piyasa `CRASH` (çöküş) veya `HIGH_VOLATILITY` rejimine girdiğinde bot yeni işlem açmayı reddederdi.
* **Yeni Davranış (Rejim Filtresini Kapatabilme):** Kullanıcı istediği rejimde alıma devam edebilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Piyasa analiz modülü trendin sert bir çöküşte veya aşırı volatilitede olduğunu tespit ettiğinde 'düşen bıçağı tutmamak' için alımı durdurur."*
* **Kullanıcının Üstlendiği Risk:**
  > *"RİSK: Çöküş rejiminde alım yapmaya devam ederseniz, trendin nerede duracağı bilinmediğinden sermayeniz uzun süre kilitli kalabilir veya likidasyona sürüklenebilirsiniz."*
* **Arayüz Kontrolü:** `Çöküş ve Yüksek Volatiliteli Rejimlerde de Alım Yap`

---

### 2.8 Tek Aktif Güvenlik Emri vs Çoklu Bekleyen Emir (Multi-Resting Safety Orders)
* **Eski Davranış:** Borsada her an kesinlikle yalnız 1 adet güvenlik emri dinlenebilirdi.
* **Yeni Davranış (Aktif Güvenlik Emri Sayısı Seçimi):** Kullanıcı 3Commas'taki gibi `activeSafetyOrders: N` (örneğin aynı anda 3 veya 5 güvenlik emrini tahtaya yaz) seçebilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Tek aktif emir kuralı, borsada sadece 1 emrin beklemesini sağlar. Böylece ani bir iğnede (flash-crash) tüm paranız tek saniyede harcanmaz."*
* **Kullanıcının Üstlendiği Risk:**
  > *"RİSK: Birden fazla aktif güvenlik emrini tahtaya yazarsanız, 1 saniyelik bir borsa çöküş iğnesinde tüm seviyeleriniz aynı anda dolar ve ani bir marj çağrısına (Margin Call) düşebilirsiniz."*
* **Arayüz Kontrolü:** `Aynı Anda Borsada Bekleyecek Güvenlik Emri Sayısı: [ 1 ] (Önerilen: 1)`

---

### 2.9 Günlük Çekilme Devre Kesicisi (Daily DD Circuit Breaker)
* **Eski Davranış:** Günlük zarar %2'yi aşınca otomatik devre kesici sistemi kilitlerdi.
* **Yeni Davranış (Ayarlanabilir veya Kapatılabilir Devre Kesici):** Eşik kullanıcı tarafından belirlenir (örn: %5, %10) veya tamamen kapatılabilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Bir gün içinde art arda gelen kayıplarda toplam kasanızın erimesini önlemek için işlemleri otomatik olarak durduran acil durum sigortasıdır."*
* **Kullanıcının Üstlendiği Risk:**
  > *"Devre kesiciyi kapatırsanız, arka arkaya gelen ters piyasa hareketlerinde bot durmaksızın zarar yazabilir ve kasanızı tamamen tüketebilir."*
* **Arayüz Kontrolü:** `Günlük Devre Kesici Eşiği: [ %5 ] (0 = Devre Dışı)`

---

### 2.10 Gate 5: Fitil Dokunuşu & Penetrasyon Kuralı
* **Eski Davranış:** Limit emrin dolması için fiyatın emri en az 1 tick aşması zorunluydu (`RESTING_TOUCH`).
* **Yeni Davranış (Simülasyon / Dolum Modu Seçimi):** Kullanıcı `realistic` yerine `legacy / optimistic` modunu seçebilir.
* **Botta Ne İşe Yaradığı Yazar:**
  > *"Gerçekçi modda, fiyat limit emrinize sadece dokunup döndüyse emir dolmuş sayılmaz; sıranın size gelmesi için fiyatın emri aşması gerekir."*
* **Kullanıcının Üstlendiği Risk:**
  > *"İyimser modu seçerseniz, fiyata teğet geçen fitillerde emriniz dolmuş gibi görünür; ancak canlı borsada bu emirlerin çoğu dolmaz ve gerçek sonuçlarınız backtestten kötü çıkar."*

---

## 3. DEĞİŞTİRİLEMEYEN FİZİKİ VE GÜVENLİK DEĞİŞMEZLERİ

Kullanıcı kendi riskini taşır, ancak **fizik kurallarını ve borsa güvenlik protokollerini çiğneyemez**. Aşağıdaki 2 kural uyarıya dönüştürülemez; fail-closed kalmak zorundadır:

1. **Zero-Withdrawal Hard Gate (Para Çekme Yetkisi Yasağı - HTTP 422):**
   - *Neden Değiştirilemez?* API anahtarında para çekme yetkisi varsa, bot sistemindeki herhangi bir güvenlik açığında kullanıcının tüm cüzdanı boşaltılabilir. Hiçbir kullanıcı "ben paramın çalınması riskini alıyorum" diyemez. Çekim izni olan anahtar **asla kabul edilmez**.
2. **Gate 6 (Nakit/Marj Yetersizliği - Olmayan Parayı Harcamama):**
   - *Neden Değiştirilemez?* Cüzdanda 100 USDT varken borsa 500 USDT'lik spot alım emrini kabul etmez (bakiye yetersiz hatası verir). Olmayan bakiye ile emir gönderilemeyeceğinden yetersiz bakiyede emir atlama (`SKIP`) veya bekleme (`WAIT`) mantığı fiziki bir zorunluluktur.

---

## 4. NEXT.JS ARAYÜZ (UI) TASARIMI: UYARI VE RİSK ROZETLERİ

Kullanıcının riski bilinçli üstlenebilmesi için bot ekranlarında aşağıdaki UI bileşenleri yer alır:

```
┌──────────────────────────────────────────────────────────────────────────┐
│ ⚠️  STRATEJİ RİSK VE UYARI BİLDİRİMİ                                     │
├──────────────────────────────────────────────────────────────────────────┤
│ 🔸 [M20 SL Çakışması]: Stop-Loss (%3), 2. güvenlik seviyenizden (%4)      │
│    daha dar. Fiyat oraya inemeden bot stop olacak. Sermayenin %60'ı      │
│    ölü kalacak.                                                          │
│                                                                          │
│ 🔸 [Açılış Seansı]: ABD Hisse açılış tamponu devre dışı bırakıldı.        │
│    İlk 5 dakika spread genişlemesinde kötü fiyattan alım riski var.     │
│                                                                          │
│ [X] Yukarıdaki riskleri okudum, anladım ve sorumluluğu üstleniyorum.     │
│                                           [ BOTU YİNE DE ÇALIŞTIR ]      │
└──────────────────────────────────────────────────────────────────────────┘
```

### Kodlama Şartı (Zod Şeması & State)
- Eskiden formda `throw new Error(...)` veya `form.setError(...)` ile engellenen kurallar, artık `warnings: WarningItem[]` listesine yazılır.
- Kullanıcı onay kutusunu işaretlediği anda (`acknowledged: true`) buton aktif hale gelir ve konfigürasyon kaydedilir.

---

## 5. RUST MOTORU İÇİN YENİ ENUM VE SÖZLEŞME

Rust tarafında strateji konfigürasyonlarına her koruma için bir `RiskMode` enum'ı eklenir:

```rust
// crates/dca_kernel/src/risk_policy.rs
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuardAction {
    /// Katı Koruma: Koşul ihlal edilirse işlemi engeller / erteler.
    Enforce,
    /// Kullanıcı Özerkliği: Uyarır, log kaydı atar ama işleme izin verir.
    PermissiveWithWarning,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserRiskPolicy {
    pub session_guard: GuardAction,         // Gate 7
    pub opening_salvo_buffer: GuardAction,  // Gate 8a
    pub gap_consolidation: GuardAction,     // Gate 8b
    pub macro_blackout: GuardAction,        // Gate 7 haber
    pub regime_filter: GuardAction,         // Faz 10 çöküş
    pub m20_sl_ladder_guard: GuardAction,   // M20 SL çakışması
    pub grid_fee_breaker: GuardAction,      // g_min komisyon kırıcı
}

impl Default for UserRiskPolicy {
    fn default() -> Self {
        Self {
            session_guard: GuardAction::Enforce,
            opening_salvo_buffer: GuardAction::Enforce,
            gap_consolidation: GuardAction::Enforce,
            macro_blackout: GuardAction::Enforce,
            regime_filter: GuardAction::PermissiveWithWarning,
            m20_sl_ladder_guard: GuardAction::PermissiveWithWarning,
            grid_fee_breaker: GuardAction::PermissiveWithWarning,
        }
    }
}
```

---

## 6. ÖZET TABLO: ENGELLEME VS BİLGİLENDİRİLMİŞ UYARI

| Koruma Adı | Katı Modda Davranış | Yeni Uyarı Modunda Davranış | Botta Görünen Risk Açıklaması |
| :--- | :--- | :--- | :--- |
| **M20 SL-Merdiven** | Bot kurulumunu engeller. | İşlemi açar, uyarı rozeti basar. | *"SL seviyeniz merdivenden önce çalışacak; kalan sermaye ölü kalır."* |
| **Grid Komisyon Kırıcı** | Izgara kurulumunu iptal eder. | Izgarayı kurar, uyarı verir. | *"Izgara adımı borsa komisyonundan dar; net bakiyeniz her turda erir."* |
| **Gate 7 Seans Saati** | Seans dışı emirleri reddeder. | Seans dışı da alım dener. | *"Seans kapalıyken likidite sığdır; yüksek fiyat kayması (slippage) yaşanabilir."* |
| **Gate 8a Açılış Tamponu**| İlk 5 dakika alımı bekletir. | İlk 5 dakika anında alır. | *"Açılışın ilk 300 saniyesinde spreadler %5.5'e kadar açılabilir."* |
| **Makro Kara Pencere** | Haber anında alımı durdurur. | Haber anında da alım yapar. | *"Kritik haber anındaki ani iğnelerde anında stop olma riski yüksektir."* |
| **Piyasa Rejim Filtresi** | Çöküşte yeni işlem açmaz. | Çöküşte de alım yapar. | *"Piyasa çöküş rejiminde; düşen bıçağı tutma ve uzun süre kilitlenme riski."* |
| **Çoklu Aktif Güvenlik** | Sadece 1 safety emri bekler. | Kullanıcı $N$ adet safety yazar. | *"Ani bir flash-crash iğnesinde tüm seviyeler dolarak marj çağrısına yol açabilir."* |
| **Günlük DD Kesici** | Günlük %2 zararda botu kilitler.| Eşiği yükseltir veya kapatır. | *"Art arda gelen ters piyasada bot durmaksızın zarar yazıp kasanızı tüketebilir."* |
