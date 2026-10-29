//! Task 16 — Piyasa Korumaları ve Bilgilendirilmiş Risk Motoru.
//!
//! Yeni ilke: "Kullanıcı Kendi Riskini Taşır" (Informed Risk & User Autonomy).
//! Motor kullanıcıyı engellemez (Zero Hard-Block); matematiksel riski hesaplar,
//! loglar, arayüzde ne işe yaradığını ve üstlenilen riski açıklar. Kullanıcı
//! onay verdiğinde emirler kesintisiz borsaya iletilmeye devam eder.
//!
//! İki sınıf koruma vardır:
//! 1. UYARI & GEÇERSİZ KILINABİLİR KORUMALAR (Advisory / User Override):
//!    M20 SL-Merdiven, Grid Komisyon Kırıcı, Seans/Makro Kara Pencere,
//!    Açılış Salvo Tamponu, Gap Konsolidasyonu, Rejim Filtresi, Çoklu Emir,
//!    Günlük DD Kesici, Dolum Modeli.
//! 2. FİZİKİ & GÜVENLİK DEĞİŞMEZLERİ (fail-closed, esnetilemez):
//!    Zero-Withdrawal Hard Gate (HTTP 422) ve Gate 6 (olmayan parayı harcamama).
//!    Bu iki kural UserRiskPolicy ile ASLA aşılamaz.

use serde::{Deserialize, Serialize};

/// Bir korumanın çalışma modu.
/// - Enforce: Katı koruma — koşul ihlal edilirse işlem engellenir / ertelenir.
/// - PermissiveWithWarning: Kullanıcı özerkliği — uyarır, log kaydı atar ama
///   işleme izin verir (Acknowledge & Proceed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum GuardAction {
    Enforce,
    PermissiveWithWarning,
}

impl GuardAction {
    pub fn is_permissive(&self) -> bool {
        matches!(self, GuardAction::PermissiveWithWarning)
    }
    pub fn label(&self) -> &'static str {
        match self {
            GuardAction::Enforce => "ZORUNLU",
            GuardAction::PermissiveWithWarning => "UYARI+İZİN",
        }
    }
}

/// Gate 5 dolum gerçekçiliği (spec 2.10):
/// - Realistic: limit emrin dolması için fiyatın emri EN AZ 1 TICK aşması
///   zorunludur (teğet dokunuş dolmaz) — dokümanlanmış RESTING_TOUCH semantiği.
/// - Optimistic (legacy): fiyata teğet geçen fitiller dolmuş sayılır; canlı
///   borsada bu emirlerin çoğu dolmaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum FillModel {
    Realistic,
    Optimistic,
}

/// Bot'un hangi piyasa seansında işlem gördüğü (Gate 7 / 8a'nın tetikleyicisi).
/// Kripto 24/7'de seans koruması hiç tetiklenmez; ABD Hissesi / Emtia için
/// M21 takvimi (DST + NYSE tatilleri) devreye girer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RiskSessionType {
    #[serde(rename = "CRYPTO_24_7")]
    Crypto24_7,
    #[serde(rename = "US_EQUITY")]
    UsEquity,
    #[serde(rename = "COMMODITY_23_5")]
    Commodity23_5,
}

impl From<RiskSessionType> for dca_math::sessions::SessionType {
    fn from(v: RiskSessionType) -> Self {
        match v {
            RiskSessionType::Crypto24_7 => dca_math::sessions::SessionType::Crypto24_7,
            RiskSessionType::UsEquity => dca_math::sessions::SessionType::UsEquity,
            RiskSessionType::Commodity23_5 => dca_math::sessions::SessionType::Commodity23_5,
        }
    }
}

/// Kullanıcı risk politikası — bot başına tüm esnetilebilir korumaların modu.
/// Serde default'u sayesinde eski snapshot'lar/istekler policy göndermese bile
/// spec Default'u ile hayata devam eder (geriye dönük uyumluluk).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UserRiskPolicy {
    /// Gate 7 — seans kapalıyken yeni alım.
    pub session_guard: GuardAction,
    /// Gate 8a — ABD açılışı ilk 5 dakika (09:30–09:35 ET) tamponu.
    pub opening_salvo_buffer: GuardAction,
    /// Gate 8b — atlanan kademelerin tek VWAP dolumda konsolidasyonu.
    pub gap_consolidation: GuardAction,
    /// Gate 7 — FOMC/TÜFE makro kara pencere (±15 dk).
    pub macro_blackout: GuardAction,
    /// Rejim filtresi (CRASH / HIGH_VOLATILITY) — spec default: permissive.
    pub regime_filter: GuardAction,
    /// M20 SL-Merdiven çakışması — spec default: permissive (uyarı rozeti).
    pub m20_sl_ladder_guard: GuardAction,
    /// Grid g_min komisyon kırıcı — spec default: permissive (risk bildirimi).
    pub grid_fee_breaker: GuardAction,
    /// 2.8 — aynı anda borsada bekleyecek güvenlik emri sayısı (önerilen 1).
    pub active_safety_orders: u32,
    /// 2.9 — günlük DD devre kesici eşiği (yüzde). "0" = devre dışı.
    pub daily_dd_breaker_pct: String,
    /// 2.10 — Gate 5 dolum modeli.
    pub fill_model: FillModel,
    /// Bot'un piyasa seansı (Gate 7/8a'yı tetikleyen takvim).
    pub session_type: RiskSessionType,
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
            active_safety_orders: 1,
            daily_dd_breaker_pct: "2".into(),
            fill_model: FillModel::Realistic,
            session_type: RiskSessionType::Crypto24_7,
        }
    }
}

impl UserRiskPolicy {
    /// Yapısal doğrulama — 422-worthy aralık ihlalleri (bot oluşturmada yüzeye çıkar).
    pub fn validate(&self, max_safety_orders: usize) -> Result<(), String> {
        if self.active_safety_orders == 0 {
            return Err("risk_policy.active_safety_orders en az 1 olmalı".into());
        }
        // Faz 21 (P0-11): aktif güvenlik emri üst sınırı 10 — API üzerinden
        // anlamsız derecede büyük değerle defter/mesh şişirme engellenir.
        if self.active_safety_orders > 10 {
            return Err(format!(
                "risk_policy.active_safety_orders ({}) > 10 — üst sınır 10",
                self.active_safety_orders
            ));
        }
        if self.active_safety_orders as usize > max_safety_orders.max(1) {
            return Err(format!(
                "risk_policy.active_safety_orders ({}) maksimum güvenlik emri sayısını ({max_safety_orders}) aşamaz",
                self.active_safety_orders
            ));
        }
        let dd = dca_math::fraction::Fraction::from_decimal_str(&self.daily_dd_breaker_pct)
            .map_err(|e| format!("risk_policy.daily_dd_breaker_pct ('{}'): {e}", self.daily_dd_breaker_pct))?;
        if dd.is_negative() || dd > dca_math::fraction::Fraction::from_u64(100) {
            return Err("risk_policy.daily_dd_breaker_pct [0, 100] aralığında olmalı (0 = kapalı)".into());
        }
        Ok(())
    }

    /// Günlük DD kesici aktif mi? (false = kullanıcı kapattı)
    pub fn daily_dd_breaker_active(&self) -> bool {
        match dca_math::fraction::Fraction::from_decimal_str(&self.daily_dd_breaker_pct) {
            Ok(x) => !x.is_zero(),
            Err(_) => true, // parse edilemeyen değer varsayılan "2" gibi davranır (yukarıda 422 ile yakalanır)
        }
    }
}

/// Arayüze/SSE'ye giden tek bir advisory uyarı kaydı.
/// Aynı kodlar hem create-yanıtı, hem snapshot, hem RISK SSE olayı için kullanılır.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdvisoryWarning {
    /// Makine kodu: M20_SL_CONFLICT, NEGATIVE_FEE_MARGIN, SESSION_OVERRIDE,
    /// OPENING_SALVO_DISABLED, GAP_CONSOLIDATION_OFF, MACRO_OVERRIDE,
    /// MULTI_SAFETY_RISK, DD_BREAKER_OFF, OPTIMISTIC_FILL.
    pub code: String,
    /// Türkçe kısa başlık.
    pub title: String,
    /// Türkçe risk açıklaması (spec metinleri).
    pub message: String,
    /// WARN = amber, INFO = gri.
    pub severity: String,
}

impl AdvisoryWarning {
    pub fn warn(code: &str, title: &str, message: &str) -> Self {
        Self { code: code.into(), title: title.into(), message: message.into(), severity: "WARN".into() }
    }
    pub fn info(code: &str, title: &str, message: &str) -> Self {
        Self { code: code.into(), title: title.into(), message: message.into(), severity: "INFO".into() }
    }
}

/// Spec metinleri — tek kaynak (UI risk-warnings.ts ile birebir aynı).
pub mod texts {
    pub const M20_TITLE: &str = "M20 SL-Merdiven Çakışması";
    pub const M20_MSG: &str = "Stop-Loss seviyeniz güvenlik merdiveninin toplam sapmasından dar. Erken stop durumunda kalan kademelere ayrılan sermaye ölü kalır — stratejiniz erken pes eder.";
    pub const FEE_TITLE: &str = "Grid Komisyon Kırıcı";
    pub const FEE_MSG: &str = "Izgara adımı borsa alış-satış komisyon toplamından dar; her turda net bakiyeniz komisyona erir. (Sıfır komisyon kampanyası dışında zarardır.)";
    pub const SALVO_TITLE: &str = "Açılış Salvo Tamponu Kapalı";
    pub const SALVO_MSG: &str = "Açılış tamponu kapalı — ilk 300 saniyedeki sahte volatilite iğnelerinde en tepeden veya en dipten maliyetlenebilirsiniz (spread 550 bps'e kadar açılabilir).";
    pub const SESSION_TITLE: &str = "Seans Dışı İşleme İzni";
    pub const SESSION_MSG: &str = "Seans dışı alımlara izin verildi — likidite çok sığdır, spread geniştir; emirler açılışta daha kötü fiyattan dolabilir.";
    pub const MACRO_TITLE: &str = "Makro Kara Pencere Devre Dışı";
    pub const MACRO_MSG: &str = "Haber anında (FOMC/TÜFE) işlem izni verildi — ani iki yönlü fitillerde saniyeler içinde stop olma veya merdivenin hızla tükenme riski var.";
    pub const GAP_TITLE: &str = "Gap Konsolidasyonu Kapalı";
    pub const GAP_MSG: &str = "Gap konsolidasyonu kapalı — atlanan her seviye tek tek, ayrı ayrı komisyonla doldurulur; ortalama maliyet daha yavaş iyileşir.";
    pub const MULTI_TITLE: &str = "Çoklu Güvenlik Emri Riski";
    pub const DD_TITLE: &str = "Günlük Devre Kesici Kapalı";
    pub const DD_MSG: &str = "Günlük devre kesici kapalı — arka arkaya gelen ters piyasa hareketlerinde bot durmaksızın zarar yazabilir ve kasanızı tüketebilir.";
    pub const FILL_TITLE: &str = "İyimser Dolum Modeli";
    pub const FILL_MSG: &str = "İyimser dolum modu — fiyata teğet geçen fitiller emriniz dolmuş gibi sayılır; canlı borsada bu emirlerin çoğu dolmaz ve gerçek sonuçlar backtestten kötü çıkar.";
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_matches_spec() {
        let p = UserRiskPolicy::default();
        assert_eq!(p.session_guard, GuardAction::Enforce);
        assert_eq!(p.opening_salvo_buffer, GuardAction::Enforce);
        assert_eq!(p.gap_consolidation, GuardAction::Enforce);
        assert_eq!(p.macro_blackout, GuardAction::Enforce);
        assert_eq!(p.regime_filter, GuardAction::PermissiveWithWarning);
        assert_eq!(p.m20_sl_ladder_guard, GuardAction::PermissiveWithWarning);
        assert_eq!(p.grid_fee_breaker, GuardAction::PermissiveWithWarning);
        assert_eq!(p.active_safety_orders, 1);
        assert_eq!(p.daily_dd_breaker_pct, "2");
        assert_eq!(p.fill_model, FillModel::Realistic);
        assert_eq!(p.session_type, RiskSessionType::Crypto24_7);
        assert!(p.daily_dd_breaker_active());
    }

    #[test]
    fn wire_format_roundtrip() {
        let p = UserRiskPolicy {
            m20_sl_ladder_guard: GuardAction::Enforce,
            active_safety_orders: 3,
            daily_dd_breaker_pct: "0".into(),
            fill_model: FillModel::Optimistic,
            session_type: RiskSessionType::UsEquity,
            ..Default::default()
        };
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["session_guard"], "ENFORCE");
        assert_eq!(v["m20_sl_ladder_guard"], "ENFORCE");
        assert_eq!(v["regime_filter"], "PERMISSIVE_WITH_WARNING");
        assert_eq!(v["fill_model"], "OPTIMISTIC");
        assert_eq!(v["session_type"], "US_EQUITY");
        assert_eq!(v["active_safety_orders"], 3);
        assert_eq!(v["daily_dd_breaker_pct"], "0");
        let back: UserRiskPolicy = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
        assert!(!back.daily_dd_breaker_active(), "0 = kapalı");
    }

    #[test]
    fn absent_policy_fields_fill_defaults() {
        // Eski istekler risk_policy göndermez → hepsi default.
        let v: serde_json::Value = serde_json::json!({
            "session_guard": "PERMISSIVE_WITH_WARNING"
        });
        let p: UserRiskPolicy = serde_json::from_value(v).unwrap();
        assert_eq!(p.session_guard, GuardAction::PermissiveWithWarning);
        assert_eq!(p.active_safety_orders, 1);
        assert_eq!(p.fill_model, FillModel::Realistic);
    }

    #[test]
    fn validation_ranges() {
        let mut p = UserRiskPolicy::default();
        assert!(p.validate(5).is_ok());
        p.active_safety_orders = 6;
        assert!(p.validate(5).is_err(), "active_safety_orders max_safety_orders'u aşamaz");
        p.active_safety_orders = 0;
        assert!(p.validate(5).is_err());
        p.active_safety_orders = 3;
        p.daily_dd_breaker_pct = "150".into();
        assert!(p.validate(5).is_err());
        p.daily_dd_breaker_pct = "-1".into();
        assert!(p.validate(5).is_err());
        p.daily_dd_breaker_pct = "0".into();
        assert!(p.validate(5).is_ok());
        assert!(!p.daily_dd_breaker_active());
        p.daily_dd_breaker_pct = "abc".into();
        assert!(p.validate(5).is_err());
    }
}
