//! M21 — TradFi market sessions & NYSE calendar engine.
//! Session types: US_EQUITY (DST-aware 09:30–16:00 ET), COMMODITY_23_5 (Sun 23:00 UTC – Fri 22:00 UTC,
//! daily 22:00–23:00 UTC pause), 24_7_CRYPTO. NYSE holidays computed via Meeus/Jones/Butcher
//! Easter algorithm — no hardcoded year tables. TradFi daily borrow interest: N * rate / 360.

use chrono::{DateTime, Datelike, Duration, TimeZone, Timelike, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionType {
    Crypto24_7,
    UsEquity,
    Commodity23_5,
}

/// Meeus/Jones/Butcher Gregorian Easter Sunday calculation.
pub fn easter_sunday(year: i32) -> (u32, u32) {
    let a = year % 19;
    let b = year / 100;
    let c = year % 100;
    let d = b / 4;
    let e = b % 4;
    let f = (b + 8) / 25;
    let g = (b - f + 1) / 3;
    let h = (19 * a + b - d - g + 15) % 30;
    let i = c / 4;
    let k = c % 4;
    let l = (32 + 2 * e + 2 * i - h - k) % 7;
    let m = (a + 11 * h + 22 * l) / 451;
    let month = ((h + l - 7 * m + 114) / 31) as u32;
    let day = (((h + l - 7 * m + 114) % 31) + 1) as u32;
    (month, day)
}

fn nth_weekday_of_month(year: i32, month: u32, weekday: u32, nth: u32) -> Option<chrono::NaiveDate> {
    use chrono::NaiveDate;
    let first = NaiveDate::from_ymd_opt(year, month, 1)?;
    let first_wd = first.weekday().num_days_from_sunday();
    let offset = (weekday + 7 - first_wd) % 7;
    let day = 1 + offset + 7 * (nth - 1);
    NaiveDate::from_ymd_opt(year, month, day)
}

fn last_weekday_of_month(year: i32, month: u32, weekday: u32) -> Option<chrono::NaiveDate> {
    use chrono::NaiveDate;
    let (next_y, next_m) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let first_next = NaiveDate::from_ymd_opt(next_y, next_m, 1)?;
    let last = first_next - Duration::days(1);
    let last_wd = last.weekday().num_days_from_sunday();
    let back = (last_wd + 7 - weekday) % 7;
    Some(last - Duration::days(back as i64))
}

/// US Eastern offset from UTC for a given date (DST: 2nd Sun Mar → 1st Sun Nov).
/// Returns offset seconds: -4h (EDT) or -5h (EST).
pub fn us_eastern_utc_offset_seconds(date: chrono::NaiveDate) -> i64 {
    let year = date.year();
    let dst_start = nth_weekday_of_month(year, 3, 0, 2).unwrap(); // 2nd Sunday March
    let dst_end = nth_weekday_of_month(year, 11, 0, 1).unwrap(); // 1st Sunday November
    if date >= dst_start && date < dst_end {
        -4 * 3600
    } else {
        -5 * 3600
    }
}

/// NYSE holidays for a year (observed: Sat->Fri, Sun->Monday rolls).
pub fn nyse_holidays(year: i32) -> Vec<chrono::NaiveDate> {
    use chrono::NaiveDate;
    let mut days: Vec<chrono::NaiveDate> = Vec::new();
    let mut push = |d: chrono::NaiveDate| days.push(d);

    // Fixed-date holidays with weekend roll
    let fixed = |m: u32, d: u32| NaiveDate::from_ymd_opt(year, m, d).unwrap();
    // New Year (Jan 1)
    push(roll(fixed(1, 1)));
    // MLK: 3rd Monday Jan
    push(nth_weekday_of_month(year, 1, 1, 3).unwrap());
    // Washington's Birthday: 3rd Monday Feb
    push(nth_weekday_of_month(year, 2, 1, 3).unwrap());
    // Good Friday (Easter - 2)
    let (em, ed) = easter_sunday(year);
    let easter = NaiveDate::from_ymd_opt(year, em, ed).unwrap();
    push(easter - Duration::days(2));
    // Memorial: last Monday May
    push(last_weekday_of_month(year, 5, 1).unwrap());
    // Juneteenth (Jun 19)
    push(roll(fixed(6, 19)));
    // Independence (Jul 4)
    push(roll(fixed(7, 4)));
    // Labor: 1st Monday Sep
    push(nth_weekday_of_month(year, 9, 1, 1).unwrap());
    // Thanksgiving: 4th Thursday Nov
    push(nth_weekday_of_month(year, 11, 4, 4).unwrap());
    // Christmas (Dec 25)
    push(roll(fixed(12, 25)));
    days.sort();
    days.dedup();
    days
}

fn roll(d: chrono::NaiveDate) -> chrono::NaiveDate {
    use chrono::Weekday;
    match d.weekday() {
        Weekday::Sat => d - Duration::days(1),
        Weekday::Sun => d + Duration::days(1),
        _ => d,
    }
}

/// Is the market open for the session type at UTC instant ts?
pub fn is_market_open(session: SessionType, ts: DateTime<Utc>) -> bool {
    match session {
        SessionType::Crypto24_7 => true,
        SessionType::Commodity23_5 => {
            let wd = ts.weekday().num_days_from_monday(); // Mon=0..Sun=6
            let (h, m) = (ts.hour(), ts.minute());
            let mins = h * 60 + m;
            let daily_pause = mins >= 22 * 60 && mins < 23 * 60;
            // Opens Sun 23:00 UTC (wd=6) → Fri 22:00 UTC (wd=4)
            match wd {
                5 => false, // Saturday closed
                6 => mins >= 23 * 60, // Sunday: open after 23:00
                0..=3 => !daily_pause, // Mon-Thu
                4 => mins < 22 * 60, // Friday: closes 22:00
                _ => false,
            }
        }
        SessionType::UsEquity => {
            let date = ts.date_naive();
            let wd = ts.weekday().num_days_from_monday();
            if wd >= 5 {
                return false;
            }
            if nyse_holidays(date.year()).contains(&date) {
                return false;
            }
            let off = us_eastern_utc_offset_seconds(date);
            let local = ts + Duration::seconds(off);
            let mins = local.hour() * 60 + local.minute();
            (9 * 60 + 30) <= mins && mins < 16 * 60
        }
    }
}

/// Is `ts` inside the opening 5-minute salvo buffer (09:30–09:35 ET)?
pub fn in_opening_salvo_buffer(ts: DateTime<Utc>) -> bool {
    let date = ts.date_naive();
    let off = us_eastern_utc_offset_seconds(date);
    let local = ts + Duration::seconds(off);
    let mins = local.hour() * 60 + local.minute();
    mins >= 9 * 60 + 30 && mins < 9 * 60 + 35
}

/// TradFi daily borrow interest: N * annual_rate / 360 (exact).
pub fn daily_borrow_interest(notional: &crate::fraction::Fraction, annual_rate: &crate::fraction::Fraction) -> Result<crate::fraction::Fraction, crate::fraction::MathError> {
    use crate::fraction::Fraction;
    notional.mul(annual_rate)?.div(&Fraction::from_u64(360))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn easter_canon() {
        // Known values: 2024-03-31, 2025-04-20, 2026-04-05
        assert_eq!(easter_sunday(2024), (3, 31));
        assert_eq!(easter_sunday(2025), (4, 20));
        assert_eq!(easter_sunday(2026), (4, 5));
    }

    #[test]
    fn us_equity_open_closed() {
        // 2026-01-15 is a Thursday (not holiday) — 15:30 UTC = 10:30 EST -> open
        let t = Utc.with_ymd_and_hms(2026, 1, 15, 15, 30, 0).unwrap();
        assert!(is_market_open(SessionType::UsEquity, t));
        // 20:00 UTC = 15:00 EST -> open
        let t2 = Utc.with_ymd_and_hms(2026, 1, 15, 20, 0, 0).unwrap();
        assert!(is_market_open(SessionType::UsEquity, t2));
        // 21:30 UTC = 16:30 EST -> closed
        let t3 = Utc.with_ymd_and_hms(2026, 1, 15, 21, 30, 0).unwrap();
        assert!(!is_market_open(SessionType::UsEquity, t3));
        // Saturday closed
        let t4 = Utc.with_ymd_and_hms(2026, 1, 17, 16, 0, 0).unwrap();
        assert!(!is_market_open(SessionType::UsEquity, t4));
        // Thanksgiving 2026 = Nov 26 (4th Thursday) closed
        let t5 = Utc.with_ymd_and_hms(2026, 11, 26, 15, 0, 0).unwrap();
        assert!(!is_market_open(SessionType::UsEquity, t5));
    }

    #[test]
    fn dst_transition() {
        // July -> EDT (-4): 13:30 UTC = 09:30 EDT open boundary
        let t = Utc.with_ymd_and_hms(2026, 7, 15, 13, 30, 0).unwrap();
        assert!(is_market_open(SessionType::UsEquity, t));
        // January -> EST (-5): 14:30 UTC = 09:30 EST open boundary
        let t2 = Utc.with_ymd_and_hms(2026, 1, 14, 14, 30, 0).unwrap();
        assert!(is_market_open(SessionType::UsEquity, t2));
    }

    #[test]
    fn commodity_235() {
        // Sunday 23:30 UTC -> open
        let t = Utc.with_ymd_and_hms(2026, 1, 18, 23, 30, 0).unwrap();
        assert_eq!(t.weekday().num_days_from_monday(), 6);
        assert!(is_market_open(SessionType::Commodity23_5, t));
        // Tuesday 22:30 UTC -> daily pause
        let t2 = Utc.with_ymd_and_hms(2026, 1, 20, 22, 30, 0).unwrap();
        assert!(!is_market_open(SessionType::Commodity23_5, t2));
        // Friday 21:59 -> open ; Friday 22:01 -> closed
        let t3 = Utc.with_ymd_and_hms(2026, 1, 23, 21, 59, 0).unwrap();
        assert!(is_market_open(SessionType::Commodity23_5, t3));
        let t4 = Utc.with_ymd_and_hms(2026, 1, 23, 22, 1, 0).unwrap();
        assert!(!is_market_open(SessionType::Commodity23_5, t4));
    }

    #[test]
    fn borrow_interest() {
        use crate::fraction::Fraction;
        let i = daily_borrow_interest(&Fraction::from_decimal_str("10000").unwrap(), &Fraction::from_decimal_str("0.05").unwrap()).unwrap();
        assert_eq!(i.to_half_even_display(4), "1.3889"); // 10000*0.05/360
    }
}
