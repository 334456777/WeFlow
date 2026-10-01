//! Human-readable rendering of command results (the default output; `--json` / `--pretty` print JSON instead).
//!
//! Objects become aligned `key: value` lines, arrays of flat objects become tables, anything else is indented.

use serde_json::Value;

use crate::locale::tr;

const MAX_STRING: usize = 300;
const MAX_CELL: usize = 40;
const MAX_COLUMNS: usize = 8;

/// Render a value for people. The result ends with a newline.
pub fn render(value: &Value) -> String {
    let mut out = String::new();
    block(value, 0, &mut out);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Render a failure (`message`, optional details) for stderr.
pub fn render_error(message: &str, details: Option<&Value>) -> String {
    let mut out = format!("{}{message}\n", tr("error: ", "错误："));
    if let Some(details) = details.filter(|d| !d.is_null()) {
        block(details, 2, &mut out);
    }
    out
}

/// Scalars and empty collections print on one line.
fn is_scalar(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => true,
    }
}

fn scalar(value: &Value) -> String {
    match value {
        Value::Null => tr("(empty)", "（空）").to_string(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::String(s) => truncate(s, MAX_STRING),
        _ => tr("(empty)", "（空）").to_string(),
    }
}

fn truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let head: String = text.chars().take(max).collect();
    format!("{head}… ({count} {})", tr("chars", "字符"))
}

/// Terminal columns taken by `text` (CJK and emoji count double).
pub fn display_width(text: &str) -> usize {
    text.chars()
        .map(|c| match c as u32 {
            0x1100..=0x115F
            | 0x2E80..=0xA4CF
            | 0xAC00..=0xD7A3
            | 0xF900..=0xFAFF
            | 0xFE30..=0xFE6F
            | 0xFF00..=0xFF60
            | 0xFFE0..=0xFFE6
            | 0x1F300..=0x1F64F
            | 0x1F900..=0x1F9FF
            | 0x20000..=0x3FFFD => 2,
            _ => 1,
        })
        .sum()
}

fn pad(text: &str, width: usize) -> String {
    let w = display_width(text);
    format!("{text}{}", " ".repeat(width.saturating_sub(w)))
}

fn push_scalar_lines(prefix: &str, text: &str, indent: usize, out: &mut String) {
    let mut lines = text.lines();
    out.push_str(prefix);
    out.push_str(lines.next().unwrap_or(""));
    out.push('\n');
    for line in lines {
        out.push_str(&" ".repeat(indent + 2));
        out.push_str(line);
        out.push('\n');
    }
}

fn block(value: &Value, indent: usize, out: &mut String) {
    let pad_in = " ".repeat(indent);
    match value {
        Value::Object(map) if map.is_empty() => {
            out.push_str(&format!("{pad_in}{}\n", tr("(empty)", "（空）")))
        }
        Value::Object(map) => {
            let width = map
                .iter()
                .filter(|(_, v)| is_scalar(v))
                .map(|(k, _)| display_width(k))
                .max()
                .unwrap_or(0)
                .min(32);
            for (key, v) in map {
                if is_scalar(v) {
                    push_scalar_lines(
                        &format!("{pad_in}{}  ", pad(&format!("{key}:"), width + 1)),
                        &scalar(v),
                        indent,
                        out,
                    );
                } else {
                    out.push_str(&format!("{pad_in}{key}:\n"));
                    block(v, indent + 2, out);
                }
            }
        }
        Value::Array(items) if items.is_empty() => {
            out.push_str(&format!("{pad_in}{}\n", tr("(empty)", "（空）")))
        }
        Value::Array(items) => {
            if let Some(table) = table(items) {
                for line in table {
                    out.push_str(&pad_in);
                    out.push_str(&line);
                    out.push('\n');
                }
                return;
            }
            for item in items {
                if is_scalar(item) {
                    push_scalar_lines(&format!("{pad_in}- "), &scalar(item), indent, out);
                } else {
                    let mut inner = String::new();
                    block(item, indent + 2, &mut inner);
                    let rest = inner.get(indent + 2..).unwrap_or("");
                    out.push_str(&format!("{pad_in}- {rest}"));
                }
            }
        }
        other => push_scalar_lines(&pad_in, &scalar(other), indent, out),
    }
}

/// Rows of an aligned table, when every item is a flat object with few, short cells.
fn table(items: &[Value]) -> Option<Vec<String>> {
    let mut columns: Vec<&str> = Vec::new();
    for item in items {
        let map = item.as_object()?;
        for (key, v) in map {
            if !is_scalar(v) {
                return None;
            }
            if !columns.contains(&key.as_str()) {
                columns.push(key);
            }
        }
    }
    if columns.is_empty() || columns.len() > MAX_COLUMNS {
        return None;
    }
    let cell = |item: &Value, column: &str| -> Option<String> {
        let text = item
            .get(column)
            .map_or_else(|| tr("(empty)", "（空）").to_string(), scalar);
        (text.chars().count() <= MAX_CELL && !text.contains('\n')).then_some(text)
    };
    let mut rows: Vec<Vec<String>> = Vec::with_capacity(items.len());
    for item in items {
        rows.push(
            columns
                .iter()
                .map(|c| cell(item, c))
                .collect::<Option<Vec<_>>>()?,
        );
    }
    let widths: Vec<usize> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            rows.iter()
                .map(|r| display_width(&r[i]))
                .chain([display_width(c)])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: Vec<&str>| {
        let parts: Vec<String> = cells.iter().zip(&widths).map(|(c, w)| pad(c, *w)).collect();
        parts.join("  ").trim_end().to_string()
    };
    let mut lines = vec![line(columns.clone())];
    for row in &rows {
        lines.push(line(row.iter().map(String::as_str).collect()));
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn objects_align_and_nest() {
        let out = render(&json!({ "name": "a", "longer": 2, "inner": { "x": true } }));
        assert_eq!(out, "name:    a\nlonger:  2\ninner:\n  x:  true\n");
    }

    #[test]
    fn flat_object_arrays_become_tables() {
        let out = render(&json!([{ "id": 1, "name": "张三" }, { "id": 22, "name": "li" }]));
        assert_eq!(out, "id  name\n1   张三\n22  li\n");
    }

    #[test]
    fn nested_arrays_use_bullets() {
        let out = render(&json!([{ "a": 1, "b": [1, 2] }, "x"]));
        assert_eq!(out, "- a:  1\n  b:\n    - 1\n    - 2\n- x\n");
    }

    #[test]
    fn long_strings_are_cut() {
        let out = render(&json!({ "blob": "x".repeat(400) }));
        assert!(out.contains("… (400 "), "{out}");
        assert!(out.len() < 400);
    }
}
