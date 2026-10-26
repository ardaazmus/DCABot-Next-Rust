//! Makro Kara Pencere takvimi (Gate 7 — FOMC / TÜFE-CPI).
//!
//! Yüksek etkili ABD makro haber anlarında (FOMC faiz kararı 14:00 ET,
//! TÜFE/CPI açıklaması 08:30 ET) ±15 dakikalık kara pencere tanımlar.
//! Tarihler FOMC (federalreserve.gov) ve BLS'nin yayımlanmış 2025–2026
//! resmi takvimleridir — mutlak tarihler olarak saklanır (yeniden temellendirme /
//! re-basing hilesi YOK); her yıl başında güncellenmelidir (determinizm için statik).
//!
//! Zero-float ilkesi: saat dilimi dönüşümleri M21'in DST çözümleyicisi
//! (us_eastern_utc_offset_seconds) ile saniye hassasiyetinde yapılır.

use chrono::{DateTime, Duration, TimeZone, Utc};

/// Kara pencere yarı-genişliği: haber ±15 dakika (spec 2.6).
pub const BLACKOUT_HALF_WINDOW: Duration = Duration::minutes(15);

/// (yıl, ay, gün) — FOMC karar açıklaması 14:00 ET.
const FOMC_DATES: [(i32, u32, u32); 16] = [
    (2025, 1, 29), (2025, 3, 19), (2025, 5, 7), (2025, 6, 18), (2025, 7, 30), (2025, 9, 17), (2025, 10, 29), (2025, 12, 10),
    (2026, 1, 28), (2026, 3, 18), (2026, 4, 29), (2026, 6, 17), (2026, 7, 29), (2026, 9, 16), (2026, 10, 28), (2026, 12, 9),
];
/// (yıl, ay, gün) — TÜFE/CPI açıklaması 08:30 ET.
const CPI_DATES: [(i32, u32, u32); 24] = [
    (2025, 1, 15), (2025, 2, 12), (2025, 3, 12), (2025, 4, 10), (2025, 5, 13), (2025, 6, 11),
    (2025, 7, 15), (2025, 8, 12), (2025, 9, 11), (2025, 10, 15), (2025, 11, 13), (2025, 12, 10),
    (2026, 1, 13), (2026, 2, 11), (2026, 3, 11), (2026, 4, 10), (2026, 5, 12), (2026, 6, 10),
    (2026, 7, 14), (2026, 8, 12), (2026, 9, 11), (2026, 10, 13), (2026, 11, 12), (2026, 12, 10),
];

/// ET yerel tarih-saatini UTC'ye çevirir (M21 DST çözümü ile).
fn et_to_utc(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> Option<DateTime<Utc>> {
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let offset = crate::sessions::us_eastern_utc_offset_seconds(date);
    let naive = date.and_hms_opt(hour, minute, 0)?;
    Utc.timestamp_opt(naive.and_utc().timestamp() - offset, 0).single()
}

/// Tüm kara pencere merkezleri (UTC anı, etiket) — kronolojik.
pub fn event_times() -> Vec<(DateTime<Utc>, &'static str)> {
    let mut out = Vec::new();
    for (y, m, d) in FOMC_DATES.iter() {
        if let Some(t) = et_to_utc(*y, *m, *d, 14, 0) {
            out.push((t, "FOMC"));
        }
    }
    for (y, m, d) in CPI_DATES.iter() {
        if let Some(t) = et_to_utc(*y, *m, *d, 8, 30) {
            out.push((t, "CPI"));
        }
    }
    out.sort_by_key(|x| x.0);
    out
}

/// Verilen UTC anı bir makro kara penceresi içinde mi?
pub fn in_macro_blackout(ts: DateTime<Utc>) -> Option<(&'static str, DateTime<Utc>)> {
    for (t, label) in event_times() {
        let start = t - BLACKOUT_HALF_WINDOW;
        let end = t + BLACKOUT_HALF_WINDOW;
        if ts >= start && ts <= end {
            return Some((label, end));
        }
    }
    None
}

/// (t0, t1) milisaniye aralığı bir kara pencereyle kesişiyor mu?
/// Bar-sınıfı denetim: kapalı bar aralığı [t0, t1] pencereyi kesiyorsa
/// o bar içinde açılan pozisyonlar blackout kapsamındadır (zero look-ahead
/// dostu — karar anında barın kendi geçmiş aralığına bakılır).
pub fn range_overlaps_blackout(t0: i64, t1: i64) -> Option<(&'static str, i64)> {
    let from = DateTime::<Utc>::from_timestamp_millis(t0)?;
    let to = DateTime::<Utc>::from_timestamp_millis(t1)?;
    if to < from {
        return None;
    }
    for (t, label) in event_times() {
        let start = t - BLACKOUT_HALF_WINDOW;
        let end = t + BLACKOUT_HALF_WINDOW;
        if from <= end && to >= start {
            return Some((label, end.timestamp_millis()));
        }
    }
    None
}

/// Task 20 L5 — takvim tazeliği: son kayıtlı makro olayının UTC millis'i.
/// Takvim 2026 sonuna kadar kapsar; sonrası için Gate 7 sessizce no-op olur
/// (fail-open drift) — bu yüzden tüketici tarafı staleness'u GÖRÜNÜR kılmalı.
pub fn last_event_ts_ms() -> i64 {
    event_times().last().map(|(t, _)| t.timestamp_millis()).unwrap_or(0)
}

/// Task 20 L5 — takvim bayat mı? Son olaydan 30 gün geçtiyse koruma fiilen
/// kapalıdır ve kullanıcı uyarılmalı (sessiz koruma kaybı yok).
pub fn is_stale(now_ms: i64) -> bool {
    let last = last_event_ts_ms();
    last > 0 && now_ms > last + 30 * 86_400_000
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn fomc_jan_2025_window() {
        // 2025-01-29 14:00 ET = 19:00 UTC (EST -5). Pencere 18:45–19:15 UTC.
        let inside = Utc.with_ymd_and_hms(2025, 1, 29, 18, 50, 0).unwrap();
        let hit = in_macro_blackout(inside);
        assert!(hit.is_some(), "FOMC penceresi içinde olmalı");
        assert_eq!(hit.unwrap().0, "FOMC");
        let before = Utc.with_ymd_and_hms(2025, 1, 29, 18, 0, 0).unwrap();
        assert!(in_macro_blackout(before).is_none());
        let after = Utc.with_ymd_and_hms(2025, 1, 29, 20, 0, 0).unwrap();
        assert!(in_macro_blackout(after).is_none());
    }

    #[test]
    fn cpi_dst_window() {
        // 2025-04-10 08:30 ET = 12:30 UTC (EDT -4).
        let inside = Utc.with_ymd_and_hms(2025, 4, 10, 12, 35, 0).unwrap();
        assert_eq!(in_macro_blackout(inside).map(|x| x.0), Some("CPI"));
        // Kış: 2025-01-15 08:30 ET = 13:30 UTC (EST -5).
        let winter = Utc.with_ymd_and_hms(2025, 1, 15, 13, 35, 0).unwrap();
        assert_eq!(in_macro_blackout(winter).map(|x| x.0), Some("CPI"));
    }

    #[test]
    fn range_overlap() {
        // FOMC 2026-06-17 14:00 ET = 18:00 UTC; pencere 17:45–18:15.
        let t0 = Utc.with_ymd_and_hms(2026, 6, 17, 19, 0, 0).unwrap().timestamp_millis();
        let t1 = Utc.with_ymd_and_hms(2026, 6, 17, 20, 0, 0).unwrap().timestamp_millis();
        assert!(range_overlaps_blackout(t0, t1).is_none(), "bar pencereyi kesmemeli");
        let s0 = Utc.with_ymd_and_hms(2026, 6, 17, 17, 0, 0).unwrap().timestamp_millis();
        let s1 = Utc.with_ymd_and_hms(2026, 6, 17, 18, 0, 0).unwrap().timestamp_millis();
        let hit = range_overlaps_blackout(s0, s1);
        assert!(hit.is_some());
        assert_eq!(hit.unwrap().0, "FOMC");
    }

    #[test]
    fn event_count() {
        assert_eq!(event_times().len(), 40, "16 FOMC + 24 CPI");
        // Kronolojik sıralama
        let ts = event_times();
        for w in ts.windows(2) {
            assert!(w[0].0 <= w[1].0);
        }
    }

    #[test]
    fn staleness_boundary() {
        let last = last_event_ts_ms();
        assert!(last > 0, "takvim boş olamaz");
        // son olay + 29 gün → bayat DEĞİL; + 31 gün → bayat
        assert!(!is_stale(last + 29 * 86_400_000));
        assert!(is_stale(last + 31 * 86_400_000));
    }
}
