//! Task 16 (spec 2.7) — Piyasa Rejim Filtresi: CRASH / HIGH_VOLATILITY tespiti.
//!
//! Deterministik, zero look-ahead: sınıflandırma yalnızca kendisine verilen
//! KAPALI bar kapanışlarını kullanır (karar anında gelecekteki bar yoktur).
//! Rejim bir METRİK tır (f64) — muhasebe değildir (crate felsefesi, lib.rs).
//!
//! Eşikler sabit ve belgelenmiştir:
//! - ÇÖKÜŞ (CRASH): son bar kapanışı, pencere yüksek kapanışından
//!   `CRASH_DRAWDOWN` (varsayılan %10) veya daha fazla düşükse.
//! - YÜKSEK VOLATİLİTE: pencere içindeki bar-getirilerinin örneklem standart
//!   sapması `VOL_STD_PER_BAR` (varsayılan %3/bar) üzeriyse.
//! İkisinden biri tetiklenirse rejim o isimle döner; ikisi de yoksa NORMAL.

use serde::{Deserialize, Serialize};

/// Pencere: son kaç kapanış üzerinden bakılır (deterministik sabit).
pub const REGIME_WINDOW: usize = 24;
/// Çöküş eşiği: pencere tepe kapanışından %10 düşüş.
pub const CRASH_DRAWDOWN: f64 = 0.10;
/// Yüksek volatilite eşiği: bar-başı getiri std sapması %3.
pub const VOL_STD_PER_BAR: f64 = 0.03;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum MarketRegime {
    Normal,
    Crash,
    HighVolatility,
}

impl MarketRegime {
    pub fn label(&self) -> &'static str {
        match self {
            MarketRegime::Normal => "NORMAL",
            MarketRegime::Crash => "CRASH",
            MarketRegime::HighVolatility => "HIGH_VOLATILITY",
        }
    }
    pub fn is_risky(&self) -> bool {
        !matches!(self, MarketRegime::Normal)
    }
}

/// Kapanış serisinden (kronolojik, kapalı barlar) rejim sınıflandırması.
/// Yetersiz veri (< 3 kapanış) → Normal (fail-open, koruma bilinemezse
/// uyarı üretmez; fail-closed gerektiren gate'ler ayrı katmandadır).
pub fn classify(closes: &[f64]) -> MarketRegime {
    if closes.len() < 3 {
        return MarketRegime::Normal;
    }
    let window: &[f64] = if closes.len() > REGIME_WINDOW {
        &closes[closes.len() - REGIME_WINDOW..]
    } else {
        closes
    };
    // 1) Çöküş: pencere tepesinden son kapanışa düşüş.
    let window_high = window.iter().cloned().fold(f64::MIN, f64::max);
    let last = window[window.len() - 1];
    if window_high > 0.0 {
        let dd = (window_high - last) / window_high;
        if dd >= CRASH_DRAWDOWN {
            return MarketRegime::Crash;
        }
    }
    // 2) Volatilite: basit getirilerin örneklem standart sapması.
    if window.len() >= 3 {
        let rets: Vec<f64> = window.windows(2).filter_map(|w| {
            if w[0] > 0.0 {
                Some((w[1] - w[0]) / w[0])
            } else {
                None
            }
        }).collect();
        if rets.len() >= 2 {
            let n = rets.len() as f64;
            let mean = rets.iter().sum::<f64>() / n;
            let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
            let std = var.sqrt();
            if std >= VOL_STD_PER_BAR {
                return MarketRegime::HighVolatility;
            }
        }
    }
    MarketRegime::Normal
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normal_flat_market() {
        let flat: Vec<f64> = (0..30).map(|i| 100.0 + (i % 3) as f64 * 0.1).collect();
        assert_eq!(classify(&flat), MarketRegime::Normal);
    }

    #[test]
    fn crash_detected_on_10pct_drop() {
        let mut c: Vec<f64> = (0..30).map(|i| 100.0 + i as f64 * 0.1).collect();
        c.push(88.0); // tepe ~102.9 → %14.5 düşüş
        assert_eq!(classify(&c), MarketRegime::Crash);
    }

    #[test]
    fn high_volatility_detected() {
        // ±5% salınımlar → std ~%5 > %3; çöküş yok (tepe-dip simetrik).
        let c: Vec<f64> = (0..30).map(|i| if i % 2 == 0 { 100.0 } else { 105.0 }).collect();
        assert_eq!(classify(&c), MarketRegime::HighVolatility);
    }

    #[test]
    fn too_little_data_is_normal() {
        assert_eq!(classify(&[100.0, 50.0]), MarketRegime::Normal);
        assert!(classify(&[]).is_risky() == false);
    }

    #[test]
    fn wire_names() {
        assert_eq!(serde_json::to_value(MarketRegime::Crash).unwrap(), serde_json::json!("CRASH"));
        assert_eq!(
            serde_json::to_value(MarketRegime::HighVolatility).unwrap(),
            serde_json::json!("HIGH_VOLATILITY")
        );
    }
}
