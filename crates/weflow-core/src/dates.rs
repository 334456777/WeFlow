//! `YYYY-MM-DD` dates in the machine's local time zone, as the commands take them (`--start`, `--end`).

use std::ops::RangeInclusive;

use crate::error::{AppError, AppResult};

/// The years a date or a report can be for.
pub(crate) const YEARS: RangeInclusive<i32> = 1970..=2100;

/// `YYYY-MM-DD` → (year, month, day), validated.
///
/// # Errors
/// A usage error that says which part of `s` is wrong. The errors are made here, with their own text, so that the
/// catalog of their Chinese is checked against them (#1154).
fn parse_date_parts(s: &str) -> AppResult<(i32, u32, u32)> {
    let parts: Vec<&str> = s.splitn(3, '-').collect();
    if parts.len() != 3 {
        return Err(AppError::usage(format!(
            "invalid date '{s}'; use YYYY-MM-DD"
        )));
    }
    let y: i32 = parts[0]
        .parse()
        .map_err(|_| AppError::usage(format!("invalid year in '{s}'")))?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| AppError::usage(format!("invalid month in '{s}'")))?;
    let d: u32 = parts[2]
        .parse()
        .map_err(|_| AppError::usage(format!("invalid day in '{s}'")))?;
    if !YEARS.contains(&y) {
        return Err(AppError::usage(format!(
            "invalid year in '{s}'; use 1970-2100"
        )));
    }
    if !(1..=12).contains(&m) {
        return Err(AppError::usage(format!(
            "invalid month in '{s}'; use 01-12"
        )));
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let dim = match m {
        2 => {
            if leap {
                29
            } else {
                28
            }
        }
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=dim).contains(&d) {
        return Err(AppError::usage(format!(
            "invalid day in '{s}'; {y}-{m:02} has {dim} days"
        )));
    }
    Ok((y, m, d))
}

/// The usage error of a date that the local time zone skips.
fn not_in_the_zone(s: &str) -> AppError {
    AppError::usage(format!("'{s}' does not exist in the local time zone"))
}

/// Unix time of 00:00 on that date in the machine's local time zone (the zone `chat dates` and the statistics use).
fn parse_date_local(s: &str) -> AppResult<i64> {
    let (y, m, d) = parse_date_parts(s)?;
    crate::message::time::local_midnight(y, m, d).ok_or_else(|| not_in_the_zone(s))
}

/// Unix time of 00:00 on the day after that date (a 23- or 25-hour day is handled by the zone rules).
fn parse_next_date_local(s: &str) -> AppResult<i64> {
    let (y, m, d) = parse_date_parts(s)?;
    crate::message::time::local_midnight_after(y, m, d).ok_or_else(|| not_in_the_zone(s))
}

/// Parses an optional `--start` / `--end` pair (`end` becomes the exclusive next-midnight bound) and rejects a start
/// that is after the end.
pub fn date_range_args(
    start: Option<&str>,
    end: Option<&str>,
) -> AppResult<(Option<i64>, Option<i64>)> {
    let b = start.map(parse_date_local).transpose()?;
    let e = end.map(parse_next_date_local).transpose()?;
    if let (Some(b), Some(e)) = (b, e) {
        if b >= e {
            return Err(AppError::usage(format!(
                "--start {} is after --end {}",
                start.unwrap_or(""),
                end.unwrap_or("")
            )));
        }
    }
    Ok((b, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_validated() {
        assert!(parse_date_local("2026-09-24").is_ok());
        assert!(parse_date_local("2024-02-29").is_ok());
        for bad in [
            "2026-13-40",
            "2026-02-29",
            "2026-04-31",
            "2026-00-10",
            "1969-01-01",
            "abc",
            "2026-09",
        ] {
            let err = parse_date_local(bad).unwrap_err();
            assert_eq!(err.code, "invalid_arguments", "{bad}");
        }
    }

    /// Every error of a date has its Chinese (#1154).
    #[test]
    fn the_errors_of_a_date_are_in_chinese() {
        for (bad, zh) in [
            ("2024-02-30", "'2024-02-30' 中的日期无效；2024-02 有 29 天"),
            ("2026-13-01", "'2026-13-01' 中的月份无效；请使用 01-12"),
            ("1969-01-01", "'1969-01-01' 中的年份无效；请使用 1970-2100"),
            ("abc", "日期 'abc' 无效；请使用 YYYY-MM-DD"),
            ("x-01-01", "'x-01-01' 中的年份无效"),
            ("2026-x-01", "'2026-x-01' 中的月份无效"),
            ("2026-01-x", "'2026-01-x' 中的日期无效"),
        ] {
            let err = date_range_args(Some(bad), None).unwrap_err();
            assert_eq!(
                crate::messages_zh::translate(&err.message).as_deref(),
                Some(zh),
                "{}",
                err.message
            );
        }
        assert_eq!(
            crate::messages_zh::translate("'2026-03-08' does not exist in the local time zone")
                .as_deref(),
            Some("本地时区中不存在 '2026-03-08'")
        );
    }

    #[test]
    fn range_ends_are_the_next_local_midnight() {
        let (b, e) = date_range_args(Some("2026-09-30"), Some("2026-09-30")).unwrap();
        let day = e.unwrap() - b.unwrap();
        assert!(
            (23 * 3600..=25 * 3600).contains(&day),
            "one local day: {day}"
        );
        assert_eq!(date_range_args(None, None).unwrap(), (None, None));
    }

    #[test]
    fn ranges_must_be_ordered() {
        assert!(date_range_args(Some("2026-09-01"), Some("2026-09-30")).is_ok());
        assert!(
            date_range_args(Some("2026-09-30"), Some("2026-09-30")).is_ok(),
            "the end day is inclusive"
        );
        assert!(date_range_args(Some("2026-09-30"), Some("2026-09-01")).is_err());
        assert!(date_range_args(None, Some("2026-09-01")).is_ok());
    }
}
