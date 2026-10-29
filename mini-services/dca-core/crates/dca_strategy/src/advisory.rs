//! Task 16 — Advisory uyarı hesaplayıcı (tek kaynak).
//!
//! create_bot yanıtı, bot snapshot'ı ve event kayıtları AYNI fonksiyondan
//! beslenir — UI'daki canlı hesap (risk-warnings.ts) yalnızca ön-render'dır;
//! motorun kararı bu modüldür.
//!
//! Kural: her uyarı bir "RiskMotoru" gerçeğidir ve SPEC metinleriyle yazılır.
//! warnings boş değilse create akışı risk_acknowledged ister (UI checkbox).

use crate::BotConfig;
use dca_kernel::risk_policy::{texts, AdvisoryWarning, GuardAction};
use dca_kernel::FillModel;
use dca_math::fraction::Fraction;
use dca_math::m02::SizingMode;
use dca_math::m15_m20::m20_conflict_scan;
use dca_math::Instrument;

/// Grid türü mü? (fee-breaker uyarısı yalnız grid ailesinde anlamlı)
fn is_grid(kind: crate::StrategyKind) -> bool {
    matches!(kind, crate::StrategyKind::SpotGrid | crate::StrategyKind::FuturesGrid | crate::StrategyKind::InfinityGrid)
}

/// Grid fee-breaker ihlali: Some((en_dar_segment, g_min)) — ihlal yoksa None.
/// Faz 22 (N-11): TEK MATEMATİK — create-time uyarısı artık GridEngine::new
/// ile AYNI yolu yürür: gerçek mesh üretilir (aritmetik/geometrik), tick'e
/// hizalanır ve `validate_grid_fee_breaker` ile EN DAR segment + asimetrik
/// ücretler (maker-in / taker-out) denetlenir. Eski "ortalama adım ×
/// maker×maker" hesabı iki kapının farklı hüküm vermesine yol açıyordu.
pub fn grid_fee_breaker_violation(cfg: &BotConfig, inst: &Instrument) -> Option<(Fraction, Fraction)> {
    let lower = Fraction::from_decimal_str(cfg.grid_lower.as_deref().unwrap_or("0")).ok()?;
    let upper = Fraction::from_decimal_str(cfg.grid_upper.as_deref().unwrap_or("0")).ok()?;
    if lower.is_zero() || upper.is_zero() || lower >= upper {
        return None; // yapısal hata ayrıca 422 ile yüzeye çıkar
    }
    let levels = cfg.grid_levels.unwrap_or(10).max(2).min(500);
    let mesh = match cfg.grid_mode.as_deref() {
        Some("geometric") => dca_math::m15_m20::geometric_mesh(&lower, &upper, levels).ok()?,
        _ => dca_math::m15_m20::arithmetic_mesh(&lower, &upper, levels).ok()?,
    };
    let mesh: Vec<Fraction> = mesh
        .into_iter()
        .map(|p| dca_math::m02::tick_down(&p, &inst.tick_size).unwrap_or(p))
        .collect();
    let ok = dca_math::m15_m20::validate_grid_fee_breaker(&mesh, &inst.maker_fee_rate, &inst.taker_fee_rate).ok()?;
    if ok {
        return None;
    }
    let g_min = dca_math::m15_m20::fee_breaker_min_step(&inst.maker_fee_rate, &inst.taker_fee_rate).ok()?;
    // En dar segmenti raporla (uyarı metni gerçek darboğazı göstersin).
    let tightest = mesh
        .windows(2)
        .filter_map(|w| w[1].div(&w[0]).ok()?.sub(&Fraction::one()).ok())
        .min()?;
    Some((tightest, g_min))
}

/// M20 çakışma taraması: oransal anchor (1) ve base (1) ile merdiveni kurar,
/// SL'in hangi kademeyi öldürdüğünü bulur. Geçersiz sayı girdileri uyarıyı
/// bastırır (yapısal 422 ayrı katmanda).
fn m20_conflict(cfg: &BotConfig, direction: dca_math::Direction) -> Option<usize> {
    let sl = cfg.stop_loss_pct.as_deref().filter(|s| !s.is_empty())?;
    let anchor = Fraction::from_decimal_str("1").ok()?;
    let base_qty = Fraction::from_decimal_str("1").ok()?;
    let safety = Fraction::from_decimal_str(&cfg.safety_order_size).ok()?;
    let dev = Fraction::from_decimal_str(&cfg.deviation_pct).ok()?;
    let step_scale = Fraction::from_decimal_str(&cfg.step_scale).ok()?;
    let vol_scale = Fraction::from_decimal_str(&cfg.volume_scale).ok()?;
    let sl_f = Fraction::from_decimal_str(sl).ok()?;
    // Oransal tarama: anchor=1 ölçeğinde; tick/step yuvarlamayı bozmamak için
    // yeterince küçük sabitler (eski create-bot ön taramasıyla aynı yaklaşım).
    let tick = Fraction::from_decimal_str("0.001").ok()?;
    let step = Fraction::from_decimal_str("0.00001").ok()?;
    let levels = dca_math::build_dca_plan(
        &anchor, &base_qty, &safety, &dev, &step_scale, &vol_scale, cfg.max_safety_orders,
        direction, &tick, &step, SizingMode::BaseQty,
    )
    .ok()?;
    let (viable, first, _) = m20_conflict_scan(direction, &sl_f, &base_qty, &anchor, &levels).ok()?;
    if viable {
        None
    } else {
        first
    }
}

/// Bot konfigürasyonunun TÜM advisory uyarılarını üretir (spec sırasıyla).
pub fn compute_warnings(cfg: &BotConfig, inst: &Instrument) -> Vec<AdvisoryWarning> {
    let mut out = Vec::new();
    let p = &cfg.risk_policy;

    // 1) Dinamik: M20 SL-Merdiven çakışması (DCA ailesi, SL set edilmişse)
    if let Some(direction) = cfg.kind.direction() {
        if let Some(first) = m20_conflict(cfg, direction) {
            out.push(AdvisoryWarning::warn(
                "M20_SL_CONFLICT",
                texts::M20_TITLE,
                &format!(
                    "Stop-Loss seviyeniz güvenlik merdiveninin toplam sapmasından dar; safety order #{first} erişilemez. Erken stop durumunda kalan kademelere ayrılan sermaye ölü kalır — stratejiniz erken pes eder."
                ),
            ));
        }
    }

    // 2) Dinamik: Grid komisyon kırıcı ihlali
    if is_grid(cfg.kind) {
        if let Some((per_level, g_min)) = grid_fee_breaker_violation(cfg, inst) {
            out.push(AdvisoryWarning::warn(
                "NEGATIVE_FEE_MARGIN",
                texts::FEE_TITLE,
                &format!(
                    "Izgara adım aralığınız ({}) borsa komisyon toplamından (g_min {}) dar; her al-sat turunda net bakiyeniz komisyona erir. (Sıfır komisyon kampanyası dışında zarardır.)",
                    per_level.to_half_even_display(6),
                    g_min.to_half_even_display(6)
                ),
            ));
        }
    }

    // 3) Politika bildirimleri (statik, seçilen moddan üretilir)
    if p.opening_salvo_buffer.is_permissive() {
        out.push(AdvisoryWarning::warn("OPENING_SALVO_DISABLED", texts::SALVO_TITLE, texts::SALVO_MSG));
    }
    if p.session_guard.is_permissive() && p.session_type != dca_kernel::RiskSessionType::Crypto24_7 {
        out.push(AdvisoryWarning::warn("SESSION_OVERRIDE", texts::SESSION_TITLE, texts::SESSION_MSG));
    }
    if p.macro_blackout.is_permissive() {
        out.push(AdvisoryWarning::warn("MACRO_OVERRIDE", texts::MACRO_TITLE, texts::MACRO_MSG));
    }
    if p.gap_consolidation.is_permissive() {
        out.push(AdvisoryWarning::info("GAP_CONSOLIDATION_OFF", texts::GAP_TITLE, texts::GAP_MSG));
    }
    if p.active_safety_orders > 1 {
        out.push(AdvisoryWarning::warn(
            "MULTI_SAFETY_RISK",
            texts::MULTI_TITLE,
            &format!(
                "Aynı anda {} güvenlik emri borsada bekleyecek — 1 saniyelik flash-crash iğnesinde tüm seviyeler aynı anda dolarak marj çağrısına (Margin Call) düşebilirsiniz.",
                p.active_safety_orders
            ),
        ));
    }
    if !p.daily_dd_breaker_active() {
        out.push(AdvisoryWarning::warn("DD_BREAKER_OFF", texts::DD_TITLE, texts::DD_MSG));
    }
    if p.fill_model == FillModel::Optimistic {
        out.push(AdvisoryWarning::warn("OPTIMISTIC_FILL", texts::FILL_TITLE, texts::FILL_MSG));
    }
    let _ = GuardAction::Enforce; // (enum referansı — lint sessizliği değil, api yüzeyi)
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inst() -> Instrument {
        Instrument {
            symbol: "TESTUSDT".into(),
            maker_fee_rate: Fraction::from_decimal_str("0.001").unwrap(),
            taker_fee_rate: Fraction::from_decimal_str("0.001").unwrap(),
            ..Default::default()
        }
    }

    #[test]
    fn default_policy_no_warnings() {
        let cfg = BotConfig::default();
        let w = compute_warnings(&cfg, &inst());
        assert!(w.is_empty(), "default policy + SL yok → uyarı yok, got: {w:?}");
    }

    #[test]
    fn m20_conflict_warns_permissive() {
        let mut cfg = BotConfig::default();
        cfg.stop_loss_pct = Some("0.005".into()); // %0.5 SL, %2 deviation → çakışma
        let w = compute_warnings(&cfg, &inst());
        assert!(w.iter().any(|x| x.code == "M20_SL_CONFLICT"), "got: {w:?}");
    }

    #[test]
    fn wide_sl_no_warning() {
        let mut cfg = BotConfig::default();
        cfg.stop_loss_pct = Some("0.5".into()); // %50 SL — merdivenden geniş
        let w = compute_warnings(&cfg, &inst());
        assert!(!w.iter().any(|x| x.code == "M20_SL_CONFLICT"), "got: {w:?}");
    }

    #[test]
    fn grid_fee_violation_warns() {
        let mut cfg = BotConfig::default();
        cfg.kind = crate::StrategyKind::SpotGrid;
        cfg.grid_lower = Some("100".into());
        cfg.grid_upper = Some("100.05".into()); // toplam %0.05 / 10 seviye = adım %0.005 < %0.2 g_min
        cfg.grid_levels = Some(10);
        let w = compute_warnings(&cfg, &inst());
        assert!(w.iter().any(|x| x.code == "NEGATIVE_FEE_MARGIN"), "got: {w:?}");
    }

    #[test]
    fn policy_notices_match_choices() {
        let mut cfg = BotConfig::default();
        cfg.risk_policy.opening_salvo_buffer = GuardAction::PermissiveWithWarning;
        cfg.risk_policy.macro_blackout = GuardAction::PermissiveWithWarning;
        cfg.risk_policy.active_safety_orders = 3;
        cfg.risk_policy.daily_dd_breaker_pct = "0".into();
        cfg.risk_policy.fill_model = FillModel::Optimistic;
        cfg.risk_policy.session_type = dca_kernel::RiskSessionType::UsEquity;
        cfg.risk_policy.session_guard = GuardAction::PermissiveWithWarning;
        let warns = compute_warnings(&cfg, &inst());
        let codes: Vec<&str> = warns.iter().map(|x| x.code.as_str()).collect();
        for c in [
            "OPENING_SALVO_DISABLED",
            "MACRO_OVERRIDE",
            "MULTI_SAFETY_RISK",
            "DD_BREAKER_OFF",
            "OPTIMISTIC_FILL",
            "SESSION_OVERRIDE",
        ] {
            assert!(codes.contains(&c), "{c} eksik: {codes:?}");
        }
    }

    #[test]
    fn wire_shape() {
        let w = AdvisoryWarning::warn("X_Y", "Başlık", "Mesaj");
        let v = serde_json::to_value(&w).unwrap();
        assert_eq!(v["code"], "X_Y");
        assert_eq!(v["severity"], "WARN");
    }
}
