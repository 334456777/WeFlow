//! CSV files: UTF-8 that starts with a byte order mark, for every CSV the program writes (#1075).
//!
//! Excel opens a CSV without the mark in the system's code page (GBK on a Chinese Windows), so every Chinese header,
//! name and message in it is garbled; with the mark it reads the file as UTF-8.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};

/// The UTF-8 byte order mark (`EF BB BF`) every CSV file starts with.
const BOM: &str = "\u{feff}";

/// `out` created to be written as a CSV file: buffered, its folder made first, the [`BOM`] already written.
///
/// # Errors
///
/// When the folder or the file cannot be made, or the mark cannot be written; the error names the folder or file.
pub(super) fn create_csv(out: &Path) -> Result<BufWriter<File>> {
    let mut writer = super::create_output(out)?;
    writer
        .write_all(BOM.as_bytes())
        .with_context(|| format!("failed to write {}", out.display()))?;
    Ok(writer)
}

/// Writes the CSV file `out`: the UTF-8 byte order mark, then `text`, its lines as they are; the folder of `out` is
/// made first.
///
/// # Errors
///
/// When the folder or the file cannot be made or written; the error names the folder or file.
pub fn write_csv(out: &Path, text: &str) -> Result<()> {
    let mut writer = create_csv(out)?;
    writer
        .write_all(text.as_bytes())
        .and_then(|()| writer.flush())
        .with_context(|| format!("failed to write {}", out.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use weflow_native::fixture::TestDir;

    #[test]
    fn a_csv_file_starts_with_the_utf8_byte_order_mark() {
        let dir = TestDir::new("csv-bom");
        let out = dir.path().join("in").join("a folder").join("list.csv");
        write_csv(&out, "名字,备注\n张三,\"a, b\"\n").unwrap();
        let bytes = std::fs::read(&out).unwrap();
        assert_eq!(bytes[..3], [0xEF, 0xBB, 0xBF]);
        assert_eq!(
            std::str::from_utf8(&bytes[3..]).unwrap(),
            "名字,备注\n张三,\"a, b\"\n"
        );
    }

    #[test]
    fn a_csv_file_that_cannot_be_made_names_it() {
        let dir = TestDir::new("csv-blocked");
        // a file where the folder should be
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"").unwrap();
        let err = write_csv(&blocked.join("list.csv"), "a\n").unwrap_err();
        assert!(
            format!("{err:#}").contains(&blocked.display().to_string()),
            "{err:#}"
        );
    }
}
