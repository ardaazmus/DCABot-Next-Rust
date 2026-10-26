//! Faz 2.1 filo VaR/CVaR (v3 `GET /api/fleet/risk` + `fleet-metrics.ts`
//! karsiligi - SARTLI port).
//!
//! Matematik v3 ile birebir (isaretler ayni):
//! - w_i = nav_i / NAV; Σ_ij = ρ_ij·σ_i·σ_j (ayni sembol → 1; aksi
//!   siniflar-arasi matris); σ_p = √(wᵀΣw); VaR99 = 2.326·σ_p·NAV;
//!   CVaR99 = σ_p·NAV·φ(z)/α (φ(z)=exp(−z²/2)/√(2π), α=0.01);
//!   DR = Σw_iσ_i / σ_p. Bos/NAV=0 → durust sifir (NaN asla yok).
//!
//! SARTLILIK NOTLARI (yol haritasi 2.1 hukmu, aynen uygulandi):
//! - (a) `sqrt/exp` rasyonel cekirdekte YOK — bu uc raporlama siniridir:
//!   VaR/CVaR/σ_p/DR f64 ile hesaplanir, yanitta `"tahmin"` etiketi tasir;
//!   para matematigi degil (pozisyon degerleri Fraction exact kalir).
//!   Kilit test toleranslidir (bagil 1e-9).
//! - (b) Vol girdisi gunluk-kline RV'dir: son 90 log-getirinin RMS'i
//!   (v3 `calibrateDeviationStep` ile ayni pencere/formul — mean-centering
//!   YOK, RMS). 120 gunluk bar cekilir, <92 kapanis → pair `excluded`'a
//!   duser (v3 durustluk kurali aynen). Kaynak: BYBIT linear, dususte spot.
//! - Varlik sinifi: yerel katalog yalniz kripto bilir → bilinmeyen dahil
//!   tum semboller CRYPTO (v3 `getAssetClass` varsayilani aynen); matris
//!   yine de tam tutulur (gelecekteki siniflar icin).
//! - Mark = stepper.last_close (fleet overview ile ayni yerel mark); mark
//!   yoksa pair toplama GIRMEZ.
//!
//! ASLA 500 DEGIL: her bacak ayri korunur (kline 4sn timeout, parse
//! guard'li); basarisiz pair `excluded` + gerekce olur.

use axum::extract::State;
use axum::Json;
use dca_exchange::VenueAdapter;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

use crate::state::{now_ms, AppState};

const Z_99: f64 = 2.326;
const ALPHA: f64 = 0.01;
const SQRT_2PI: f64 = 2.5066282746310002; // √(2π)
const KLINES_TIMEOUT: Duration = Duration::from_secs(4);
const KLINES_FETCH: usize = 120;
const RV_WINDOW: usize = 90;
const MIN_CLOSES: usize = RV_WINDOW + 2; // v3: closes < window+2 → null

/// Siniflar-arasi korelasyon (v3 DEFAULT_CORRELATION aynen).
pub fn correlation(a: &str, b: &str, same_symbol: bool) -> f64 {
    if same_symbol {
        return 1.0;
    }
    match (a, b) {
        ("CRYPTO", "CRYPTO") => 0.85,
        ("EQUITY", "EQUITY") => 0.70,
        ("COMMODITY", "COMMODITY") => 0.60,
        ("CRYPTO", "EQUITY") | ("EQUITY", "CRYPTO") => 0.40,
        ("CRYPTO", "COMMODITY") | ("COMMODITY", "CRYPTO") => 0.25,
        ("EQUITY", "COMMODITY") | ("COMMODITY", "EQUITY") => 0.30,
        _ => 0.40, // bilinmeyen sinif ciftine muhafazakar orta deger (not dusuldu)
    }
}

/// Yerel varlik sinifi: katalog yalniz kripto bilir (v3 varsayilani CRYPTO).
pub fn asset_class(_symbol: &str) -> &'static str {
    "CRYPTO"
}

/// Gunluk RV: son 90 log-getirinin RMS'i (v3 `calibrateDeviationStep`
/// formulu: mean-centering yok). <92 kapanis veya <90 getiri → None.
pub fn realized_vol_daily(closes: &[f64]) -> Option<f64> {
    if closes.len() < MIN_CLOSES {
        return None;
    }
    let mut rets = Vec::with_capacity(closes.len() - 1);
    for w in closes.windows(2) {
        if w[0] <= 0.0 || w[1] <= 0.0 || !w[0].is_finite() || !w[1].is_finite() {
            continue; // bozuk mum — atla (v3 kapı-1 ruhu)
        }
        rets.push((w[1] / w[0]).ln());
    }
    if rets.len() < RV_WINDOW {
        return None;
    }
    let tail = &rets[rets.len() - RV_WINDOW..];
    let mean_sq: f64 = tail.iter().map(|r| r * r).sum::<f64>() / RV_WINDOW as f64;
    if !mean_sq.is_finite() {
        return None;
    }
    Some(mean_sq.sqrt())
}

pub struct FleetPosition {
    pub symbol: String,
    pub asset_class: String,
    pub nav: f64,
    pub vol: f64,
}

pub struct FleetRiskResult {
    pub nav_total: f64,
    pub weights: Vec<(String, f64)>,
    pub sigma_p: f64,
    pub var99: f64,
    pub cvar99: f64,
    pub wavg_vol: f64,
    pub dr: f64,
}

fn zeroed(symbols: &[String]) -> FleetRiskResult {
    FleetRiskResult {
        nav_total: 0.0,
        weights: symbols.iter().map(|s| (s.clone(), 0.0)).collect(),
        sigma_p: 0.0,
        var99: 0.0,
        cvar99: 0.0,
        wavg_vol: 0.0,
        dr: 0.0,
    }
}

/// Parametrik filo riski (f64 — raporlama tahmini; para matematigi degil).
pub fn compute_fleet_risk(positions: &[FleetPosition]) -> FleetRiskResult {
    if positions.is_empty() {
        return zeroed(&[]);
    }
    let nav_total: f64 = positions.iter().map(|p| p.nav).sum();
    if !(nav_total > 0.0) {
        return zeroed(&positions.iter().map(|p| p.symbol.clone()).collect::<Vec<_>>());
    }
    let w: Vec<f64> = positions.iter().map(|p| p.nav / nav_total).collect();
    let vols: Vec<f64> = positions.iter().map(|p| p.vol).collect();
    let mut variance = 0.0;
    for (i, pi) in positions.iter().enumerate() {
        for (j, pj) in positions.iter().enumerate() {
            let rho = correlation(
                &pi.asset_class,
                &pj.asset_class,
                pi.symbol == pj.symbol,
            );
            variance += w[i] * w[j] * rho * vols[i] * vols[j];
        }
    }
    if !(variance >= 0.0) || variance == 0.0 {
        // Negatif/NaN varyans uretilemez — sifir don (v3 durust-sifir kurali).
        // variance == 0 (tum vol sifir) → sigma 0, DR 0.
        let mut r = zeroed(&positions.iter().map(|p| p.symbol.clone()).collect::<Vec<_>>());
        r.nav_total = nav_total;
        r.weights = positions.iter().zip(w.iter()).map(|(p, &x)| (p.symbol.clone(), x)).collect();
        return r;
    }
    let sigma_p = variance.sqrt();
    let var99 = Z_99 * sigma_p * nav_total;
    let phi = (-Z_99 * Z_99 / 2.0).exp() / SQRT_2PI;
    let cvar99 = sigma_p * nav_total * phi / ALPHA;
    let wavg_vol: f64 = w.iter().zip(vols.iter()).map(|(a, b)| a * b).sum();
    let dr = if sigma_p == 0.0 { 0.0 } else { wavg_vol / sigma_p };
    FleetRiskResult {
        nav_total,
        weights: positions.iter().zip(w.iter()).map(|(p, &x)| (p.symbol.clone(), x)).collect(),
        sigma_p,
        var99,
        cvar99,
        wavg_vol,
        dr,
    }
}

fn f12(x: f64) -> String {
    if !x.is_finite() {
        return "0.000000000000".into();
    }
    format!("{x:.12}")
}

fn f8(x: f64) -> String {
    if !x.is_finite() {
        return "0.00000000".into();
    }
    format!("{x:.8}")
}

pub async fn fleet_risk(State(app): State<Arc<AppState>>) -> Json<Value> {
    // Acik pozisyonlar: runtime snapshot'lari, sembol bazinda grupla.
    // nav = Σ |qty| × mark (mark = yerel last_close; yoksa dislanir).
    let mut qty_by_symbol: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut uncounted: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    {
        let rts = app.runtimes.lock().await;
        for rt in rts.values() {
            let snap = rt.engine_kind.snapshot();
            if snap.deal_id.is_none() {
                continue;
            }
            let sym = rt.cfg.symbol.to_uppercase();
            let mark = rt
                .stepper
                .last_close
                .as_deref()
                .and_then(|s| s.parse::<f64>().ok())
                .filter(|m| *m > 0.0 && m.is_finite());
            let qty = snap
                .position_qty
                .split('/')
                .next()
                .and_then(|s| s.trim().parse::<f64>().ok())
                .map(|q| q.abs())
                .unwrap_or(0.0);
            match (mark, qty > 0.0) {
                (Some(m), true) => {
                    *qty_by_symbol.entry(sym).or_insert(0.0) += qty * m;
                }
                _ => {
                    *uncounted.entry(sym).or_insert(0) += 1;
                }
            }
        }
    }
    let mut symbols: Vec<String> = qty_by_symbol.keys().cloned().collect();
    symbols.sort();

    // Gunluk RV: sembol basina 120 gunluk bar (BYBIT linear → dususte spot), 4sn timeout.
    let vols = futures_util::future::join_all(symbols.iter().map(|sym| {
        let app = app.clone();
        let sym = sym.clone();
        async move {
            let fetch = async {
                let linear = tokio::time::timeout(
                    KLINES_TIMEOUT,
                    app.bybit.klines("linear", &sym, "D", KLINES_FETCH),
                )
                .await
                .ok()?
                .ok()?;
                if linear.len() >= MIN_CLOSES {
                    return Some(linear);
                }
                // Dusuş: spot kanali.
                tokio::time::timeout(KLINES_TIMEOUT, app.bybit.klines("spot", &sym, "D", KLINES_FETCH))
                    .await
                    .ok()?
                    .ok()
            };
            let kl = fetch.await.unwrap_or_default();
            let closes: Vec<f64> = kl.iter().filter_map(|k| k.c.parse::<f64>().ok()).collect();
            (sym, closes.len(), realized_vol_daily(&closes))
        }
    }))
    .await;

    let mut positions: Vec<FleetPosition> = Vec::new();
    let mut excluded: Vec<String> = Vec::new();
    let mut excluded_reasons: Vec<Value> = Vec::new();
    for (sym, n_bars, rv) in vols {
        let nav = qty_by_symbol.get(&sym).copied().unwrap_or(0.0);
        match (nav > 0.0, rv) {
            (true, Some(v)) if v.is_finite() => positions.push(FleetPosition {
                symbol: sym.clone(),
                asset_class: asset_class(&sym).into(),
                nav,
                vol: v,
            }),
            _ => {
                excluded.push(sym.clone());
                let reason = if !(nav > 0.0) {
                    "nominal yok (mark/qty cozule transportemedi)"
                } else if n_bars < MIN_CLOSES {
                    "yetersiz gunluk bar (<92)"
                } else {
                    "RV uretilemedi (bozuk/negatif kapanislar)"
                };
                excluded_reasons.push(json!({"symbol": sym, "reason": reason}));
            }
        }
    }
    // Ayni sembol hem sayilan hem sayilamayan botlarda olabilir (coklu runtime):
    // sayilan varsa pair positions'ta kalir, sayilamayan bot adedi ayrica aciklanir.
    let mut partial: Vec<Value> = Vec::new();
    for (sym, n) in uncounted.iter() {
        if positions.iter().any(|p| &p.symbol == sym) {
            partial.push(json!({"symbol": sym, "uncountedRuntimes": n,
                "reason": "bu sembolde mark/qty cozule transportemeyen acik deal'li bot var — nav eksik sayilmis olabilir"}));
        } else if !excluded.contains(sym) {
            excluded.push(sym.clone());
            excluded_reasons.push(json!({"symbol": sym, "reason": "mark yok (last_close henuz uretilmedi) — hesaba girmedi"}));
        }
    }
    excluded.sort();

    let r = compute_fleet_risk(&positions);
    let dr_label = if r.dr > 1.0 {
        "Çok varlıklı çeşitlendirme riski azaltıyor (DR > 1)"
    } else {
        "Çeşitlendirme kazancı yok (DR ≤ 1)"
    };
    Json(json!({
        "generatedAt": now_ms(),
        "estimate": true,
        "positions": positions.iter().map(|p| json!({
            "symbol": p.symbol,
            "assetClass": p.asset_class,
            "navUsd": p.nav.to_string(),
            "volDaily": format!("{:.8}", p.vol),
        })).collect::<Vec<_>>(),
        "excluded": excluded,
        "excludedReasons": excluded_reasons,
        "partialCoverage": partial,
        "result": {
            "navTotal": r.nav_total.to_string(),
            "weights": r.weights.iter().map(|(s, x)| json!({"symbol": s, "weight": f12(*x)})).collect::<Vec<_>>(),
            "sigmaPortfolio": f12(r.sigma_p),
            "var99Usd": f8(r.var99),
            "cvar99Usd": f8(r.cvar99),
            "weightedAvgVol": f12(r.wavg_vol),
            "diversificationRatio": f12(r.dr),
            "zScore": "2.326",
        },
        "interpretation": {"drLabel": dr_label},
        "notes": [
            "TAHMIN: sqrt/exp f64 ile hesaplandi (rasyonel cekirdekte yok); para matematigi degil, raporlama gostergesidir.",
            "Vol = son 90 gunluk log-getirinin RMS'i (v3 calibrateDeviationStep ile ayni pencere/formul). Kaynak: BYBIT linear gunluk, dususte spot.",
            "Mark = yerel last_close (fleet overview ile ayni); mark yoksa pair hesaba girmez.",
            "Yerel katalog yalniz kripto bilir (bilinmeyen→CRYPTO, v3 varsayilani); siniflar-arasi korelasyon v3 matrisiyle aynen.",
        ],
    }))
}

#[cfg(test)]
mod risk_tests {
    use super::*;

    fn approx(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-9 * b.abs().max(1.0)
    }

    #[test]
    fn single_position_matches_closed_form() {
        // Tek pozisyon: σ_p = σ, DR = 1, VaR = 2.326·σ·NAV.
        let p = vec![FleetPosition { symbol: "BTCUSDT".into(), asset_class: "CRYPTO".into(), nav: 10_000.0, vol: 0.03 }];
        let r = compute_fleet_risk(&p);
        assert!(approx(r.sigma_p, 0.03), "sigma {}", r.sigma_p);
        assert!(approx(r.var99, 2.326 * 0.03 * 10_000.0), "var {}", r.var99);
        let phi = (-Z_99 * Z_99 / 2.0).exp() / SQRT_2PI;
        assert!(approx(r.cvar99, 0.03 * 10_000.0 * phi / 0.01), "cvar {}", r.cvar99);
        assert!(approx(r.dr, 1.0), "dr {}", r.dr);
        assert!(approx(r.nav_total, 10_000.0));
    }

    #[test]
    fn empty_and_zero_nav_are_honest_zeros() {
        let r = compute_fleet_risk(&[]);
        assert_eq!(r.nav_total, 0.0);
        assert_eq!(r.var99, 0.0);
        assert!(r.weights.is_empty());
        let z = vec![FleetPosition { symbol: "BTCUSDT".into(), asset_class: "CRYPTO".into(), nav: 0.0, vol: 0.05 }];
        let r2 = compute_fleet_risk(&z);
        assert_eq!(r2.sigma_p, 0.0);
        assert_eq!(r2.weights[0].1, 0.0);
    }

    #[test]
    fn two_cryptos_use_085_correlation() {
        // w=(0.5,0.5), σ=(0.02,0.04), ρ=0.85 → σ_p² = .25·(.0004+.0016+2·.85·.0008).
        let ps = vec![
            FleetPosition { symbol: "BTCUSDT".into(), asset_class: "CRYPTO".into(), nav: 5_000.0, vol: 0.02 },
            FleetPosition { symbol: "ETHUSDT".into(), asset_class: "CRYPTO".into(), nav: 5_000.0, vol: 0.04 },
        ];
        let r = compute_fleet_risk(&ps);
        let expect: f64 = (0.25f64 * (0.0004 + 0.0016 + 2.0 * 0.85 * 0.0008)).sqrt();
        assert!(approx(r.sigma_p, expect), "got {} want {expect}", r.sigma_p);
        assert!(r.dr > 1.0, "farkli vol'ler DR>1 vermeli, got {}", r.dr);
    }

    #[test]
    fn rv_matches_v3_rms_formula() {
        // Sabit +%1 gunluk getiri: RV = |ln(1.01)|.
        let mut c = 100.0;
        let mut closes = vec![c];
        for _ in 0..119 {
            c *= 1.01;
            closes.push(c);
        }
        let rv = realized_vol_daily(&closes).unwrap();
        assert!((rv - 1.01_f64.ln()).abs() < 1e-12, "rv {rv}");
        assert!(realized_vol_daily(&closes[..50]).is_none()); // <92 kapanis
        assert!(realized_vol_daily(&[]).is_none());
    }
}
