//! Times as a reader says them, in the local time zone: `11:16` today, `昨天` / `yesterday`, `11-20` this year,
//! `2025-10-07` before it.

use chrono::{DateTime, Datelike, Local, NaiveDate, TimeZone};

use super::layout::Style;

/// The local time of `seconds` since the Unix epoch; none for 0 or less, which stand for no time.
pub(crate) fn local(seconds: i64) -> Option<DateTime<Local>> {
    (seconds > 0)
        .then(|| Local.timestamp_opt(seconds, 0).single())
        .flatten()
}

/// `day` as a word when it is today or yesterday.
fn day_word(day: NaiveDate, style: &Style) -> Option<&'static str> {
    let today = style.now.date_naive();
    if day == today {
        Some(style.tr("today", "今天"))
    } else if today.pred_opt() == Some(day) {
        Some(style.tr("yesterday", "昨天"))
    } else {
        None
    }
}

/// A short time for a list: the clock today, `昨天` yesterday, the month and day this year, else the date.
pub(crate) fn short(seconds: i64, style: &Style) -> String {
    let Some(time) = local(seconds) else {
        return String::new();
    };
    let day = time.date_naive();
    if day == style.now.date_naive() {
        time.format("%H:%M").to_string()
    } else if let Some(word) = day_word(day, style) {
        word.to_string()
    } else if day.year() == style.now.year() {
        time.format("%m-%d").to_string()
    } else {
        time.format("%Y-%m-%d").to_string()
    }
}

/// A time with its clock: `11:16` today, `昨天 23:40`, `11-20 09:05` this year, else `2025-09-01 08:30`.
pub(crate) fn stamp(seconds: i64, style: &Style) -> String {
    let Some(time) = local(seconds) else {
        return String::new();
    };
    let day = time.date_naive();
    let clock = time.format("%H:%M");
    if day == style.now.date_naive() {
        clock.to_string()
    } else if let Some(word) = day_word(day, style) {
        format!("{word} {clock}")
    } else if day.year() == style.now.year() {
        time.format("%m-%d %H:%M").to_string()
    } else {
        time.format("%Y-%m-%d %H:%M").to_string()
    }
}

/// The clock of `seconds`: `11:16`.
pub(crate) fn clock(seconds: i64) -> String {
    local(seconds).map_or_else(String::new, |t| t.format("%H:%M").to_string())
}

/// The date and clock of `seconds`: `2025-10-07 22:49`.
pub(crate) fn full(seconds: i64) -> String {
    local(seconds).map_or_else(String::new, |t| t.format("%Y-%m-%d %H:%M").to_string())
}

/// The date of `seconds`: `2025-10-07`.
pub(crate) fn date(seconds: i64) -> String {
    local(seconds).map_or_else(String::new, |t| t.format("%Y-%m-%d").to_string())
}

/// The heading of a day of messages: `2026-11-23 周一`, with `今天` or `昨天` for those days.
pub(crate) fn day_heading(day: NaiveDate, style: &Style) -> String {
    let weekday = match style.lang {
        weflow_core::locale::Lang::Zh => ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
            [day.weekday().num_days_from_monday() as usize],
        weflow_core::locale::Lang::En => ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"]
            [day.weekday().num_days_from_monday() as usize],
    };
    let word = day_word(day, style).map_or_else(String::new, |w| format!(" {w}"));
    format!("{} {weekday}{word}", day.format("%Y-%m-%d"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::views::layout::tests::style;
    use weflow_core::locale::Lang;

    fn at(y: i32, m: u32, d: u32, h: u32, min: u32) -> i64 {
        Local
            .with_ymd_and_hms(y, m, d, h, min, 0)
            .unwrap()
            .timestamp()
    }

    #[test]
    fn a_short_time_says_the_day_as_a_reader_would() {
        let zh = style(Lang::Zh, 80);
        assert_eq!(short(at(2026, 11, 23, 11, 16), &zh), "11:16");
        assert_eq!(short(at(2026, 11, 22, 23, 40), &zh), "昨天");
        assert_eq!(short(at(2026, 11, 20, 9, 5), &zh), "11-20");
        assert_eq!(short(at(2025, 10, 7, 22, 49), &zh), "2025-10-07");
        assert_eq!(short(0, &zh), "");
        let en = style(Lang::En, 80);
        assert_eq!(short(at(2026, 11, 22, 23, 40), &en), "yesterday");
        assert_eq!(stamp(at(2026, 11, 22, 23, 40), &zh), "昨天 23:40");
        assert_eq!(stamp(at(2026, 11, 20, 9, 5), &zh), "11-20 09:05");
        assert_eq!(full(at(2025, 10, 7, 22, 49)), "2025-10-07 22:49");
    }

    #[test]
    fn a_day_heading_names_its_weekday() {
        let zh = style(Lang::Zh, 80);
        let day = |d| NaiveDate::from_ymd_opt(2026, 11, d).unwrap();
        assert_eq!(day_heading(day(23), &zh), "2026-11-23 周一 今天");
        assert_eq!(day_heading(day(22), &zh), "2026-11-22 周日 昨天");
        assert_eq!(day_heading(day(20), &style(Lang::En, 80)), "2026-11-20 Fri");
    }
}
