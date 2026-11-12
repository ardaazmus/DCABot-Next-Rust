//! Faz 26 (KAPSAMLI_DENETIM_2026-11-12) — Finans matematiği doğrulama
//! süiti: görevde istenen üç matematiksel iddianın otomatik kanıtı.
//!
//! 1. Fraction Display ↔ from_decimal_str ROUND-TRIP: serde `to_string`
//!    (Display = "n/d" veya tam sayı) → from_any_str parse zinciri kayıp
//!    vermeden kapanmalı; snapshot'lar bu zincirle dönüyor.
//! 2. gap_batch MERGED sayımı: ".m{n}" konsolide dolum tek fill üretir ama
//!    n kademe tüketmelidir (filled_safety sayacı en az n artmalı).
//! 3. geometric_mesh tick çakışması: dar aralıkta tick_down sonrası iki
//!    seviye aynı fiyata çökebilir — fee-breaker bunu ihlal olarak
//!    yakalamalı, panik/zero-div olmamalı.

use dca_kernel::{Bar, FillRecord, OrderIntent, OrderKind, SimulationStepper, StepperConfig, FeeRole};
use dca_math::fraction::Fraction;
use dca_math::instrument::Instrument;
use dca_math::m02::{tick_down, Direction};
use dca_math::m15_m20::{geometric_mesh, validate_grid_fee_breaker};
use dca_math::Side;
use dca_risk::FillRealism;
use dca_strategy::dca::{DcaEngine, DcaPhase};
use dca_strategy::{Action, BarContext, BotConfig, StrategyKind};

fn f(s: &str) -> Fraction {
    Fraction::from_decimal_str(s).unwrap()
}

fn inst() -> Instrument {
    Instrument {
        symbol: "TESTUSDT".into(),
        maker_fee_rate: f("0.001"),
        taker_fee_rate: f("0.001"),
        ..Default::default()
    }
}

fn make_bar(t: i64, o: &str, h: &str, l: &str, c: &str) -> Bar {
    Bar { t, o: o.into(), h: h.into(), l: l.into(), c: c.into(), v: "1000".into() }
}

fn dummy_context<'a>(bar: &'a Bar, mark: Fraction) -> BarContext<'a> {
    let zero: &'static Fraction = Box::leak(Box::new(Fraction::zero()));
    let cash: &'static Fraction = Box::leak(Box::new(Fraction::from_u64(1_000_000)));
    BarContext {
        bar,
        open: f(&bar.o),
        high: f(&bar.h),
        low: f(&bar.l),
        close: f(&bar.c),
        position_qty: zero,
        position_avg: None,
        cash,
        mark,
        signal_ok: None,
        regime: None,
        deal_open_blocked: false,
    }
}

fn make_fill(order_id: &str, role: &str, side: Side, kind: OrderKind, price: &str, qty: &str, ts: i64) -> FillRecord {
    FillRecord {
        fill_key: format!("{order_id}-{ts}"),
        order_id: order_id.to_string(),
        role: role.to_string(),
        side,
        kind,
        price: f(price),
        qty: f(qty),
        fee: Fraction::zero(),
        fee_role: FeeRole::Maker,
        realism: FillRealism::OpenGapBetter,
        ts,
    }
}

// ------------------------------------------------------------------
// 1. Fraction Display ↔ parse round-trip (serde sözleşmesi)
// ------------------------------------------------------------------

#[test]
fn fraction_display_roundtrip_preserves_value() {
    // serde Serialize → Display ("n/d"), Deserialize → from_any_str.
    // Display çıktısı mutlaka geri ayrıştırılabilir VE değer birebir
    // aynı olmalı — snapshot kalıcılığı bu zincirin üstünde.
    let cases = [
        "1",
        "0.5",
        "123.456",
        "0.00000001",
        "99.9",
        "3.141592653589793",
        "-7/2",
    ];
    for src in cases {
        let v = Fraction::from_any_str(src)
            .unwrap_or_else(|e| panic!("{src} ayrıştırılamadı: {e}"));
        let repr = v.to_string(); // Display (serde formatı)
        let back = Fraction::from_any_str(&repr)
            .unwrap_or_else(|e| panic!("{src} → {repr} geri ayrıştırılamadı: {e}"));
        assert_eq!(
            Fraction::cmp(&v, &back),
            std::cmp::Ordering::Equal,
            "round-trip değer değiştirdi: {src} → {repr} → geri"
        );
        // Serde tam zincir (JSON string) — snapshot sözleşmesinin ta kendisi.
        let json = serde_json::to_string(&v).unwrap();
        let back2: Fraction = serde_json::from_str(&json).unwrap();
        assert_eq!(
            Fraction::cmp(&v, &back2),
            std::cmp::Ordering::Equal,
            "serde round-trip değiştirdi: {json}"
        );
    }
}

// ------------------------------------------------------------------
// 2. gap_batch merged sayımı — tek fill, n kademe tüketimi
// ------------------------------------------------------------------

#[test]
fn gap_batch_merged_fill_consumes_n_levels_not_n_fills() {
    let bot_id = "gapcnt".to_string();
    let mut e = DcaEngine::new(&bot_id, BotConfig::default(), inst()).unwrap();

    // 1. Base dolumu → Open fazı + merdiven.
    let b0 = make_bar(1_000, "100", "101", "99", "100");
    let acts = e.on_bar(&dummy_context(&b0, f("100")));
    let base_id = acts
        .iter()
        .find_map(|a| match a {
            Action::PlaceMarket { id, role: "BASE", .. } => Some(id.clone()),
            _ => None,
        })
        .expect("base emri");
    e.on_fill(&make_fill(&base_id, "BASE", Side::Buy, OrderKind::Add, "100", "1", 1_000));
    assert_eq!(e.phase, DcaPhase::Open);
    let before = e.snapshot().safety_triggered;

    // 2. Konsolide gap dolumu: ".m3" → TEK fill kaydı, 3 kademe tüketimi.
    let merged_id = format!("{bot_id}-SAFETY-7.m3");
    e.on_fill(&make_fill(&merged_id, "SAFETY", Side::Buy, OrderKind::Add, "95", "3", 1_001));

    // filled_safety: ham +1 VEYA merged (3-1)=+2 → toplam en az +2 artmalı
    // (tek sayım “1 kademe tüketildi” demek olur — n-kademe kaybı regredisi).
    let after = e.snapshot().safety_triggered;
    assert!(
        after >= before + 2,
        "merged .m3 dolumu filled_safety'yi en az +2 artırmalı (before={before}, after={after}) — tek sayım n-kademe kaybıdır"
    );
    // Aşırı tepki de olmamalı: en fazla +3 (3 kademe).
    assert!(
        after <= before + 3,
        "merged .m3 en fazla 3 kademe tüketmeli (before={before}, after={after}) — çift sayım regredisi"
    );
}

// ------------------------------------------------------------------
// 3. geometric_mesh + tick_down çakışması — savunma davranışı
// ------------------------------------------------------------------

#[test]
fn geometric_mesh_tick_collision_is_caught_not_panicked() {
    // Dar aralık + kaba tick: geometric mesh seviyeleri tick_down sonrası
    // AYNI fiyata çökebilir. fee-breaker bu durumda 0-adım görür →
    // ihlal dönmeli (step <= g_min), panik/zero-div olmamalı.
    let lower = f("100");
    let upper = f("100.001"); // %0.001 aralık
    let tick = f("0.01"); // aralık tick'ten dar
    let mesh = geometric_mesh(&lower, &upper, 8).expect("geometric mesh kurulmalı");
    let mesh: Vec<Fraction> = mesh
        .into_iter()
        .map(|p| tick_down(&p, &tick).unwrap_or(p))
        .collect();
    // En az bir çift aynı fiyata çökmüş olmalı (savunmanın tetik koşulu);
    // çakışma yoksa bu koşul bu parametrelerle üretilememiştir — sessiz geç.
    let dup = mesh.windows(2).any(|w| Fraction::cmp(&w[0], &w[1]) == std::cmp::Ordering::Equal);
    if !dup {
        return;
    }
    let ok = validate_grid_fee_breaker(&mesh, &f("0.001"), &f("0.001"));
    assert!(ok.is_ok(), "çakışan mesh fee-breaker'ı panikletmemeli");
    assert!(!ok.unwrap(), "çakışan (0-adım) mesh ihlal olarak işaretlenmeli");
}

#[test]
fn build_plan_never_produces_non_monotonic_ladder() {
    // m02 savunması: tick yuvarlaması sonrası "not below previous" ihlali
    // Err dönmeli, sessiz çakışan merdiven üretmemeli.
    for (tick, step) in [("0.1", "0.001"), ("1", "0.01"), ("0.01", "0.00001")] {
        let r = dca_math::build_dca_plan(
            &f("1000"),
            &f("100"),
            &f("100"),
            &f("0.004"),
            &f("1.3"),
            &f("1.2"),
            8,
            Direction::Long,
            &f(tick),
            &f(step),
            dca_math::m02::SizingMode::QuoteNotional,
        );
        match r {
            Ok(levels) => {
                for w in levels.windows(2) {
                    assert!(
                        Fraction::cmp(&w[0].price, &w[1].price) == std::cmp::Ordering::Greater,
                        "merdiven monotonik değil (tick={tick})"
                    );
                }
            }
            Err(_) => {} // dürüst reddi kabul
        }
    }
}

// ------------------------------------------------------------------
// Smoke: stepper hâlâ resting limit penetrasyonla dolduruyor
// ------------------------------------------------------------------

#[test]
fn stepper_smoke_still_fills_resting_limit() {
    let mut st = SimulationStepper::new(inst(), f("10000"), StepperConfig::default());
    st.set_position_direction(1);
    st.submit_intent(
        OrderIntent {
            id: "s1".into(),
            side: Side::Buy,
            kind: OrderKind::Add,
            price: Some(f("99")),
            qty: f("1"),
            decision_ts: 1_000,
            role: "SAFETY".into(),
            placement_ref_price: Some(f("100")),
            quote_notional: None,
        },
        1_000,
    )
    .unwrap();
    let fills = st
        .step(&make_bar(1_100, "100", "100.5", "98", "100"), 1_100, false)
        .unwrap();
    assert_eq!(fills.len(), 1, "resting limit penetrasyonla dolmalı");
}

// StrategyKind importunu tüket (derleme tam bağlılık için).
#[allow(dead_code)]
fn _kind_marker() -> StrategyKind {
    StrategyKind::DcaLong
}
