//! The time of a zip entry: the local date and time, as a zip keeps it (#1215).
//!
//! A zip entry's time is an MS-DOS date and time, which has no time zone: unzip tools and file managers read it as
//! the local time of the computer. The zip crate's default is the current time in UTC, which put every extracted
//! file hours off.

use std::time::SystemTime;

use chrono::{Datelike as _, Timelike as _};

/// `time` as a zip entry's last modification: its local date and time. The zip's default (1980-01-01 00:00) when it
/// is outside what a zip can hold, the years 1980 to 2107.
pub(crate) fn local_zip_time(time: SystemTime) -> zip::DateTime {
    let local: chrono::DateTime<chrono::Local> = time.into();
    let part = |value: u32| u8::try_from(value).unwrap_or(u8::MAX);
    u16::try_from(local.year())
        .ok()
        .and_then(|year| {
            zip::DateTime::from_date_and_time(
                year,
                part(local.month()),
                part(local.day()),
                part(local.hour()),
                part(local.minute()),
                part(local.second()),
            )
            .ok()
        })
        .unwrap_or_default()
}

/// The last modification of the file at `path` as [`local_zip_time`]; the current time when it cannot be read.
pub(crate) fn file_zip_time(path: &std::path::Path) -> zip::DateTime {
    local_zip_time(
        std::fs::metadata(path)
            .and_then(|m| m.modified())
            .unwrap_or_else(|_| SystemTime::now()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, UNIX_EPOCH};

    /// The zip time of a moment is its local date and time, whatever the time zone of the computer running the test.
    #[test]
    fn a_zip_time_is_the_local_time() {
        for secs in [1_700_000_000_u64, 1_000_000_000, 4_102_444_800] {
            let time = UNIX_EPOCH + Duration::from_secs(secs);
            let local: chrono::DateTime<chrono::Local> = time.into();
            let zip = local_zip_time(time);
            assert_eq!(
                (zip.year(), zip.month(), zip.day(), zip.hour(), zip.minute()),
                (
                    u16::try_from(local.year()).unwrap(),
                    u8::try_from(local.month()).unwrap(),
                    u8::try_from(local.day()).unwrap(),
                    u8::try_from(local.hour()).unwrap(),
                    u8::try_from(local.minute()).unwrap(),
                ),
                "{secs}"
            );
        }
        // before 1980 a zip cannot hold it: the zip's own default
        assert_eq!(local_zip_time(UNIX_EPOCH), zip::DateTime::default());
    }
}
