//! The build time `build.rs` stamps into `weflow --version`, worked out here rather than by the external `date`, which
//! Windows does not have outside Git Bash.
//!
//! The file is included by `build.rs` and by `tests/build_script.rs` (`#[path]`), as build scripts take no part in
//! `cargo test`.

use std::time::{SystemTime, UNIX_EPOCH};

/// `time` in UTC as `YYYY-MM-DD HH:MMZ`, or `None` for a time before 1970.
pub fn utc_stamp(time: SystemTime) -> Option<String> {
    let secs = time.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let minutes_of_day = secs % 86_400 / 60;
    let (year, month, day) = civil_from_days(secs / 86_400);
    Some(format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}Z",
        minutes_of_day / 60,
        minutes_of_day % 60
    ))
}

/// The Gregorian `(year, month, day)` of the day `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`): it counts
/// from 0000-03-01 in eras of 400 years (146097 days) so that the leap day is the last day of a year.
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let shifted = days + 719_468;
    let era = shifted / 146_097;
    let day_of_era = shifted % 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_from_march = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_from_march + 2) / 5 + 1;
    let month = if month_from_march < 10 {
        month_from_march + 3
    } else {
        month_from_march - 9
    };
    let year = year_of_era + era * 400 + u64::from(month <= 2);
    (year, month, day)
}
