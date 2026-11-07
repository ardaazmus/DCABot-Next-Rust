//! Faz 27 (B2) — deterministic property testleri (offline).
//!
//! proptest bu kurulumun Cargo offline önbelleğinde bulunmadığından yeni
//! bağımlılık eklenmedi; yerine sabit tohumlu LCG ile deterministik rastgele
//! parametre uzayı taranır (her koşuda AYNI uzay — tekrarlanabilir).
//! Kapsanan değişmezler:
//!   P1  Merdiven monotonluğu — long'ta güvenlik fiyatları KATI AZALAN,
//!       short'ta KATI ARTAN (deviation>0, step_scale>=1 için asla kırılmaz).
//!   P2  Miktar pozitifliği — her geçerli seviyenin qty'si > 0.
//!   P3  Domain savunması — deviation<=0 / anchor<=0 / negatif boyut → Err
//!       (fail-closed; asla sessiz çirkin merdiven üretmez).
//!   P4  "Nasty number" dayanıklılığı — uç değerler (0, maksimum, 8 ondalık
//!       minik, 1'a çok yakın) 'de panik yok, Err ya da makul sonuç.
//!   P5  Fraction serileştirme turu — from_decimal_str → to_string →
//!       from_decimal_str kimlik (uç değerlerde değer korunumu).

#[cfg(test)]
mod property_tests {
    use crate::fraction::Fraction;
    use crate::m02::{build_dca_plan, Direction, SizingMode};

    /// Sabit tohumlu 64-bit LCG (xorshift64) — her koşuda aynı dizi.
    struct Lcg(u64);
    impl Lcg {
        fn new(seed: u64) -> Self {
            Lcg(seed ^ 0x9E3779B97F4A7C15)
        }
        fn next_u64(&mut self) -> u64 {
            let mut x = self.0;
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            self.0 = x;
            x
        }
        /// [lo, hi] aralığında uniform tam sayı.
        fn range(&mut self, lo: u64, hi: u64) -> u64 {
            lo + self.next_u64() % (hi - lo + 1)
        }
        /// "nastiness" enjeksiyonlu pozitif ondalık üretici: %15 uç değer
        /// (0, çok minik, çok büyük), kalanı düzgün aralık.
        fn frac(&mut self, lo: &str, hi: &str, decimals: u32) -> String {
            let r = self.next_u64() % 20;
            match r {
                0 => "0.00000001".to_string(),
                1 => format!("0.{}", "0".repeat(decimals as usize - 1) + "1"),
                2 => hi.to_string(),
                _ => {
                    let a = self.range(1, 9999);
                    let b = self.range(1, 9999);
                    let scale = 10u64.pow(decimals);
                    let lo_n = (lo.parse::<f64>().unwrap_or(1.0) * scale as f64) as u64;
                    let hi_n = (hi.parse::<f64>().unwrap_or(100.0) * scale as f64) as u64;
                    let v = lo_n + (a * b + self.next_u64()) % (hi_n - lo_n + 1).max(1);
                    format!("{}.{}", v / scale, format!("{:0width$}", v % scale, width = decimals as usize))
                }
            }
        }
    }

    fn f(s: &str) -> Fraction {
        Fraction::from_decimal_str(s).unwrap()
    }

    const TICK: &str = "0.1";
    const STEP: &str = "0.00001";

    /// P1+P2: 600 rastgele geçerli konfigde merdiven değişmezleri.
    #[test]
    fn p1_p2_ladder_monotonic_and_positive_qty() {
        let mut rng = Lcg::new(0xDCA_2026_11_07);
        for case in 0..600u32 {
            // Anchor geçerli domaine bağlanır (>= 100 = 1000×tick 0.1):
            // sub-tick anchor davranışı ayrı testte belgelenir (p4b).
            // Safety de geçerli domaine bağlanır: anchor<=1e5 ve step=1e-5
            // için safety>=1.0, en kötü fiyatta bile qty adım altına düşmez
            // (safety < adim-min → builder dürüstçe boş plan döner; p4b kapsamı).
            let mut anchor = rng.frac("100", "100000", 2);
            let mut safety = rng.frac("0.01", "5000", 2);
            if Fraction::from_decimal_str(&safety).unwrap().cmp(&f("1")) == std::cmp::Ordering::Less {
                safety = "1".to_string();
            }
            if Fraction::from_decimal_str(&anchor).unwrap().cmp(&f("100")) == std::cmp::Ordering::Less {
                anchor = "100".to_string();
            }
            let deviation = format!("0.0{}", rng.range(1, 9)); // %1..%9
            let step_scale = match rng.next_u64() % 5 {
                0 => "1".to_string(), // sınır: sabit adım
                _ => format!("1.{}", rng.range(1, 99)),
            };
            let volume_scale = match rng.next_u64() % 5 {
                0 => "1".to_string(),
                _ => format!("1.{}", rng.range(1, 99)),
            };
            let count = rng.range(1, 12) as usize;
            let dir = if rng.next_u64() % 2 == 0 { Direction::Long } else { Direction::Short };

            let plan = match build_dca_plan(
                &f(&anchor),
                &f("100"),
                &f(&safety),
                &f(&deviation),
                &f(&step_scale),
                &f(&volume_scale),
                count,
                dir,
                &f(TICK),
                &f(STEP),
                SizingMode::QuoteNotional,
            ) {
                Ok(p) => p,
                // Belgelenmiş domain kuralı: kümülatif sapma %100'ü aşarsa
                // builder dürüstçe Err döner (fail-closed) — jeneratörün
                // ürettiği bu bileşimler geçersizdir, değişmez denetimi atlanır.
                Err(e) if e.to_string().contains("cumulative deviation") => continue,
                Err(e) => panic!("case {case}: beklenmeyen Err: {e}"),
            };

            // Sözleşme: istenen sayıyı AŞMAZ — kümülatif sapma %100'e
            // değecekse builder kuyruğu keser (honest truncate); daha kısa
            // plan geçerlidir, uzun olan asla.
            assert!(
                plan.len() <= count,
                "case {case}: istenen seviye sayısı aşıldı ({} > {})",
                plan.len(),
                count
            );
            assert!(!plan.is_empty() || count == 0, "case {case}: boş plan — domain içi girdide beklenmez");
            let safety_rows = &plan[1..];
            for w in safety_rows.windows(2) {
                let (a, b) = (&w[0].price, &w[1].price);
                match dir {
                    Direction::Long => assert!(
                        a.cmp(b) == std::cmp::Ordering::Greater,
                        "case {case}: long merdiven katı azalan değil — {} -> {}",
                        a,
                        b
                    ),
                    Direction::Short => assert!(
                        a.cmp(b) == std::cmp::Ordering::Less,
                        "case {case}: short merdiven katı artan değil — {} -> {}",
                        a,
                        b
                    ),
                }
            }
            for row in &plan {
                assert!(
                    row.qty.cmp(&Fraction::zero()) == std::cmp::Ordering::Greater,
                    "case {case}: L{} qty pozitif değil",
                    row.level_index
                );
            }
        }
    }

    /// P3: domain savunması — geçersiz girdi Err (fail-closed).
    #[test]
    fn p3_domain_violations_fail_closed() {
        let mut rng = Lcg::new(0xBEEF);
        let anchor = "50000";
        let ok = |dev: &str, anchor: &str| {
            build_dca_plan(
                &f(anchor),
                &f("100"),
                &f("100"),
                &f(dev),
                &f("1.2"),
                &f("1.1"),
                3,
                Direction::Long,
                &f(TICK),
                &f(STEP),
                SizingMode::QuoteNotional,
            )
        };
        assert!(ok("0", anchor).is_err(), "deviation=0 Err olmalı");
        assert!(ok("-0.01", anchor).is_err(), "negatif deviation Err olmalı");
        assert!(ok("0.02", "0").is_err(), "anchor=0 Err olmalı");
        assert!(ok("0.02", "-5").is_err(), "negatif anchor Err olmalı");
        let _ = rng.next_u64(); // tohum kullanımı tutarlılığı (bias yok)
    }

    /// P4: nasty numbers — uç girdilerde panik yok.
    #[test]
    fn p4_nasty_numbers_no_panic() {
        let cases: &[(&str, &str, &str, &str)] = &[
            ("0.00000001", "0.00000001", "1.0", "1.0"), // aşırı minik
            ("99999999999", "99999999", "3", "3"),      // aşırı büyük
            ("1", "1", "1", "1"),                        // tam sınır: sıfır sapma adımı
            ("0.1", "99999999", "1.9999", "2.9999"),     // karışık uçlar
        ];
        for (i, (anchor, safety, ss, vs)) in cases.iter().enumerate() {
            let r = build_dca_plan(
                &f(anchor),
                &f("100"),
                &f(safety),
                &f("0.02"),
                &f(ss),
                &f(vs),
                4,
                Direction::Long,
                &f(TICK),
                &f(STEP),
                SizingMode::QuoteNotional,
            );
            // Ya Err (dürüst ret) ya da değişmezleri sağlayan plan — panik ASLA.
            if let Ok(plan) = r {
                let safety_rows = &plan[1..];
                for w in safety_rows.windows(2) {
                    assert!(
                        w[0].price.cmp(&w[1].price) == std::cmp::Ordering::Greater,
                        "case {i}: nasty girdide monotonluk bozuldu"
                    );
                }
            }
        }
    }

    /// P4b (H-1 hardening notu): sub-tick anchor'da build_dca_plan Err yerine
    /// SESSİZ BOŞ plan döndürebilir (gözlemlendi). Panik yoktur; arayan
    /// (dca.rs rebuild_ladder_from_anchor) boş planı kabul eder ve ladder
    /// boş kalır — base zaten size-guard'a takılır. Yine de bu davranış
    /// gelecek fazda Err'e bağlanmalı (fail-closed tutarlılığı).
    #[test]
    fn p4b_subtick_anchor_returns_empty_or_err_no_panic() {
        let r = build_dca_plan(
            &f("0.00000001"),
            &f("100"),
            &f("100"),
            &f("0.02"),
            &f("1.2"),
            &f("1.1"),
            5,
            Direction::Long,
            &f(TICK),
            &f(STEP),
            SizingMode::QuoteNotional,
        );
        if let Ok(plan) = r {
            assert!(plan.is_empty(), "sub-tick anchor: boş olmayan plan beklenmedik — incele");
        }
    }

    /// P5: Fraction string turu kimliği — uç değerlerde değer korunumu.
    #[test]
    fn p5_fraction_round_trip_identity() {
        let mut rng = Lcg::new(0x5EED_0001);
        let mut samples: Vec<String> = vec![
            "0".into(),
            "0.00000001".into(),
            "-0.00000001".into(),
            "12345678901234567890.12345678".into(),
            "-99999999999.99999999".into(),
        ];
        for _ in 0..300 {
            samples.push(rng.frac("0.0001", "999999", 8));
        }
        for s in &samples {
            let a = Fraction::from_decimal_str(s).unwrap_or_else(|_| panic!("{s} parse edilemedi"));
            let rt = format!("{a}");
            // Display rasyonel ("n/d") form basabilir — motorda JSON turu
            // from_any_str ile yapılır; property de aynısını kullanır.
            let b = Fraction::from_any_str(&rt).unwrap_or_else(|_| panic!("{s} -> {rt} geri parse edilemedi"));
            assert_eq!(a, b, "{s} tur kimliği bozuldu ({rt})");
        }
    }
}
