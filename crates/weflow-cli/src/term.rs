//! The terminal: sizes in words, columns and yes/no answers. Colours are `weflow_core::colour`'s.

use weflow_core::error::{AppError, AppResult};

fn is_yes(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Whether a question can be put to the user: stdin and stderr are both a terminal.
pub(crate) fn can_ask() -> bool {
    use std::io::IsTerminal;
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

/// Writes `question` on stderr and reads the answer from stdin: only `y` or `yes` is a yes, anything else, Enter
/// included, is a no.
pub(crate) fn ask_yes(question: &str) -> AppResult<bool> {
    use std::io::Write;
    let mut err = std::io::stderr();
    let _ = err.write_all(question.as_bytes());
    let _ = err.flush();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .map_err(|e| AppError::runtime(format!("failed to read the answer: {e}")))?;
    Ok(is_yes(&answer))
}

/// `1536` → `1.5 KB`: binary units, one decimal below 10.
pub(crate) fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else if value < 10.0 {
        format!("{value:.1} {}", UNITS[unit])
    } else {
        format!("{value:.0} {}", UNITS[unit])
    }
}

/// Pads the cells of `rows` into columns two spaces apart (CJK counts double); empty columns are left out. A cell
/// of several values (one per line, `\n` between them) takes as many lines, each starting under its column's
/// header (issue #258).
pub(crate) fn align_columns(rows: &[Vec<String>]) -> Vec<String> {
    use weflow_core::render::display_width;
    let columns = rows.iter().map(Vec::len).max().unwrap_or(0);
    let widths: Vec<usize> = (0..columns)
        .map(|i| {
            rows.iter()
                .filter_map(|r| r.get(i))
                .flat_map(|c| c.lines())
                .map(display_width)
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut out = Vec::new();
    for row in rows {
        let cells: Vec<Vec<&str>> = row.iter().map(|c| c.lines().collect()).collect();
        let height = cells.iter().map(Vec::len).max().unwrap_or(0).max(1);
        for line in 0..height {
            let text = cells
                .iter()
                .zip(&widths)
                .filter(|&(_, &w)| w > 0)
                .map(|(cell, &w)| {
                    let part = cell.get(line).copied().unwrap_or("");
                    format!("{part}{}", " ".repeat(w - display_width(part)))
                })
                .collect::<Vec<_>>()
                .join("  ");
            out.push(text.trim_end().to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_y_or_yes_confirms() {
        for answer in ["y\n", "Y\n", "yes\r\n", " YES "] {
            assert!(is_yes(answer), "{answer:?}");
        }
        for answer in ["\n", "", "n\n", "N", "no", "ye", "yy", "是"] {
            assert!(!is_yes(answer), "{answer:?}");
        }
    }

    #[test]
    fn sizes_are_readable() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(85 << 20), "85 MB");
        assert_eq!(format_bytes((12 << 30) / 10), "1.2 GB");
    }

    #[test]
    fn columns_line_up_and_skip_empty_ones() {
        let rows = vec![
            vec![String::new(), "图片".to_string(), "1 KB".to_string()],
            vec![String::new(), "keys".to_string(), "20 MB".to_string()],
        ];
        assert_eq!(align_columns(&rows), ["图片  1 KB", "keys  20 MB"]);
    }

    #[test]
    fn a_cell_of_several_values_takes_a_line_each_under_its_column() {
        let row = |cells: &[&str]| cells.iter().map(|c| c.to_string()).collect::<Vec<_>>();
        let rows = vec![
            row(&["部分", "大小", "路径"]),
            row(&["网页版本", "3 MB", "/a/web/1\n/a/web/longer/2"]),
            row(&["图片", "1 KB", "/a/images"]),
        ];
        assert_eq!(
            align_columns(&rows),
            [
                "部分      大小  路径",
                "网页版本  3 MB  /a/web/1",
                "                /a/web/longer/2",
                "图片      1 KB  /a/images",
            ]
        );
    }
}
