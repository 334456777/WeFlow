//! Times: local midnights and the formats of timestamps.

use super::xml::rx;
use chrono::{Local, TimeZone};

/// Unix time of 00:00 on a calendar date in the machine's local time zone. A date whose midnight does not exist
/// (a daylight-saving jump at 00:00) starts at the first moment that does.
pub fn local_midnight(year: i32, month: u32, day: u32) -> Option<i64> {
    let midnight = chrono::NaiveDate::from_ymd_opt(year, month, day)?.and_hms_opt(0, 0, 0)?;
    (0..=3 * 60)
        .find_map(|minutes| {
            Local
                .from_local_datetime(&(midnight + chrono::Duration::minutes(minutes)))
                .earliest()
        })
        .map(|t| t.timestamp())
}

/// Unix time of 00:00 on the day after a calendar date, local time zone.
pub fn local_midnight_after(year: i32, month: u32, day: u32) -> Option<i64> {
    let next = chrono::NaiveDate::from_ymd_opt(year, month, day)?.succ_opt()?;
    local_midnight(
        chrono::Datelike::year(&next),
        chrono::Datelike::month(&next),
        chrono::Datelike::day(&next),
    )
}

/// `YYYY-MM-DD HH:MM:SS` in the local time zone (same as WeFlow's desktop app).
pub fn format_timestamp(ts: i64) -> String {
    match Local.timestamp_opt(ts, 0).single() {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => String::new(),
    }
}

pub fn format_iso_timestamp(ts: i64) -> String {
    match chrono::DateTime::<chrono::Utc>::from_timestamp(ts, 0) {
        Some(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        None => String::new(),
    }
}

pub fn normalize_timestamp_seconds(value: f64) -> i64 {
    if !value.is_finite() || value <= 0.0 {
        return 0;
    }
    let mut n = crate::num::saturating_i64(value.floor());
    while n > 10_000_000_000 {
        n /= 1000;
    }
    n
}

pub(super) fn parse_compact_datetime(raw: &str) -> i64 {
    let raw = raw.trim();
    if !rx(r"^\d{8}(?:\d{4}(?:\d{2})?)?$").is_match(raw) {
        return 0;
    }
    let num = |a: usize, b: usize| raw[a..b].parse::<u32>().unwrap_or(0);
    let year = num(0, 4) as i32;
    let month = num(4, 6);
    let day = num(6, 8);
    let hour = if raw.len() >= 12 { num(8, 10) } else { 0 };
    let minute = if raw.len() >= 12 { num(10, 12) } else { 0 };
    let second = if raw.len() >= 14 { num(12, 14) } else { 0 };
    if !(1990..=2200).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || hour > 23
        || minute > 59
        || second > 59
    {
        return 0;
    }
    match Local
        .with_ymd_and_hms(year, month, day, hour, minute, second)
        .single()
    {
        Some(dt) => dt.timestamp().max(0),
        None => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_midnights_round_trip_through_the_local_formatter() {
        for (y, m, d) in [(2026, 9, 24), (2024, 2, 29), (2026, 12, 31), (2026, 1, 1)] {
            let start = local_midnight(y, m, d).unwrap();
            assert_eq!(
                format_timestamp(start),
                format!("{y}-{m:02}-{d:02} 00:00:00")
            );
            let next = local_midnight_after(y, m, d).unwrap();
            assert!(
                (23 * 3600..=25 * 3600).contains(&(next - start)),
                "a day is 23 to 25 hours long: {}",
                next - start
            );
            assert!(format_timestamp(next).ends_with("00:00:00"));
        }
        assert!(local_midnight(2026, 2, 30).is_none());
    }
}
