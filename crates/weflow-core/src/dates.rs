//! `YYYY-MM-DD` dates in the machine's local time zone, as the commands take them (`--start`, `--end`).

use crate::error::{AppError, AppResult};

/// `YYYY-MM-DD` → (year, month, day), validated.
pub fn parse_date_parts(s: &str) -> Result<(i32, u32, u32), String> {
    let parts: Vec<&str> = s.splitn(3, '-').collect();
    if parts.len() != 3 {
        return Err(format!("invalid date '{s}'; use YYYY-MM-DD"));
    }
    let y: i32 = parts[0]
        .parse()
        .map_err(|_| format!("invalid year in '{s}'"))?;
    let m: u32 = parts[1]
        .parse()
        .map_err(|_| format!("invalid month in '{s}'"))?;
    let d: u32 = parts[2]
        .parse()
        .map_err(|_| format!("invalid day in '{s}'"))?;
    if !(1970..=2100).contains(&y) {
        return Err(format!("invalid year in '{s}'; use 1970-2100"));
    }
    if !(1..=12).contains(&m) {
        return Err(format!("invalid month in '{s}'; use 01-12"));
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
        return Err(format!("invalid day in '{s}'; {y}-{m:02} has {dim} days"));
    }
    Ok((y, m, d))
}

/// Unix time of 00:00 on that date in the machine's local time zone (the zone `chat dates` and the statistics use).
pub fn parse_date_local(s: &str) -> Result<i64, String> {
    let (y, m, d) = parse_date_parts(s)?;
    crate::message::local_midnight(y, m, d)
        .ok_or_else(|| format!("'{s}' does not exist in the local time zone"))
}

/// Unix time of 00:00 on the day after that date (a 23- or 25-hour day is handled by the zone rules).
pub fn parse_next_date_local(s: &str) -> Result<i64, String> {
    let (y, m, d) = parse_date_parts(s)?;
    crate::message::local_midnight_after(y, m, d)
        .ok_or_else(|| format!("'{s}' does not exist in the local time zone"))
}

/// Parses an optional `--start` / `--end` pair (`end` becomes the exclusive next-midnight bound) and rejects a start
/// that is after the end.
pub fn date_range_args(
    start: Option<&str>,
    end: Option<&str>,
) -> AppResult<(Option<i64>, Option<i64>)> {
    let b = start
        .map(parse_date_local)
        .transpose()
        .map_err(AppError::usage)?;
    let e = end
        .map(parse_next_date_local)
        .transpose()
        .map_err(AppError::usage)?;
    if let (Some(b), Some(e)) = (b, e) {
        if b >= e {
            return Err(AppError::usage(format!(
                "--start ({}) is after --end ({})",
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
            assert!(parse_date_local(bad).is_err(), "{bad}");
        }
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
