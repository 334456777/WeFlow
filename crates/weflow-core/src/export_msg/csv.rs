//! CSV 文件：程序写出的每个 CSV 都是以字节顺序标记（BOM）开头的 UTF-8（#1075）。
//!
//! Excel 按系统代码页打开没有 BOM 的 CSV（中文 Windows 上是 GBK），其中的中文表头、名字和消息都会变成乱码；
//! 有 BOM 时 Excel 按 UTF-8 读取。

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::Path;

use anyhow::{Context, Result};

/// 每个 CSV 文件开头的 UTF-8 字节顺序标记（`EF BB BF`）。
const BOM: &str = "\u{feff}";

/// 创建要写成 CSV 的 `out`：先创建所在的文件夹，返回带缓冲的写入器，[`BOM`] 已经写入。
///
/// # Errors
///
/// 无法创建文件夹或文件，或者无法写入 BOM 时返回错误，报错里写出文件夹或文件的路径。
pub(super) fn create_csv(out: &Path) -> Result<BufWriter<File>> {
    let mut writer = super::create_output(out)?;
    writer
        .write_all(BOM.as_bytes())
        .with_context(|| format!("failed to write {}", out.display()))?;
    Ok(writer)
}

/// 写出 CSV 文件 `out`：先是 UTF-8 的 BOM，然后原样写入 `text` 的各行；先创建 `out` 所在的文件夹。
///
/// # Errors
///
/// 无法创建或写入文件夹、文件时返回错误，报错里写出文件夹或文件的路径。
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
        // 文件夹的位置上是一个文件
        let blocked = dir.path().join("blocked");
        std::fs::write(&blocked, b"").unwrap();
        let err = write_csv(&blocked.join("list.csv"), "a\n").unwrap_err();
        assert!(
            format!("{err:#}").contains(&blocked.display().to_string()),
            "{err:#}"
        );
    }
}
