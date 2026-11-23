//! Human-readable rendering of command results (the default output; `--json` prints JSON instead).
//!
//! Objects become aligned `key: value` lines, arrays of flat objects become tables, anything else is indented. Keys
//! are grey and values keep the terminal's colour, so the values stand out; table headers and the keys of nested
//! blocks are bold. Without colours the same words and layout remain.

use serde_json::Value;

use crate::colour::{Paint, Role};
use crate::locale::tr;

const MAX_STRING: usize = 300;
const MAX_CELL: usize = 40;
const MAX_COLUMNS: usize = 8;

/// Render a value for people. The result ends with a newline.
pub fn render(value: &Value, paint: Paint) -> String {
    let mut out = String::new();
    block(value, 0, paint, &mut out);
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// The word that opens a failure on stderr.
pub fn error_label() -> &'static str {
    tr("error:", "错误：")
}

/// The line a reason starts with in a message (see [`render_error`]).
pub fn reason_label() -> &'static str {
    tr("Reason:", "原因：")
}

/// The space after a label: one after a Latin colon, none after a full-width one, which has its own.
fn gap_after(label: &str) -> &'static str {
    if label.ends_with('：') {
        ""
    } else {
        " "
    }
}

/// `Reason: <reason>`, the line of a message that gives the cause underneath (see [`render_error`]).
pub fn reason_line(reason: &str) -> String {
    let label = reason_label();
    format!("{label}{}{reason}", gap_after(label))
}

/// `label value` on one line, the label grey; ends with a newline.
pub fn field(label: &str, value: &str, paint: Paint) -> String {
    format!(
        "{}{}{value}\n",
        paint.paint(Role::Muted, label),
        gap_after(label)
    )
}

/// Render a failure (`message`, optional details) for stderr. The first line of `message` follows the red `error:`
/// label; the lines after it say what to do and stay in the terminal's colour, except a line that gives the reason
/// ([`reason_label`]), which is grey like the details under it.
pub fn render_error(message: &str, details: Option<&Value>, paint: Paint) -> String {
    let mut out = labelled(error_label(), Role::Error, message, paint);
    if let Some(details) = details.filter(|d| !d.is_null()) {
        let mut text = String::new();
        block(details, 2, Paint::plain(), &mut text);
        for line in text.lines() {
            out.push_str(&paint.paint(Role::Muted, line));
            out.push('\n');
        }
    }
    out
}

/// `label`, shown as `role`, and the first line of `message` on one line, then the rest of `message` a line each, a
/// reason grey; ends with a newline.
pub fn labelled(label: &str, role: Role, message: &str, paint: Paint) -> String {
    let mut lines = message.lines();
    let mut out = format!(
        "{}{}{}\n",
        paint.paint(role, label),
        gap_after(label),
        lines.next().unwrap_or("")
    );
    for line in lines {
        if line.starts_with(reason_label()) {
            out.push_str(&paint.paint(Role::Muted, line));
        } else {
            out.push_str(line);
        }
        out.push('\n');
    }
    out
}

/// A scalar as shown: its text, grey when it stands for no value, and a grey note after it (the length of a
/// string cut short).
struct Cell {
    text: String,
    muted: bool,
    note: Option<String>,
}

impl Cell {
    fn plain(text: String) -> Self {
        Self {
            text,
            muted: false,
            note: None,
        }
    }

    fn none() -> Self {
        Self {
            text: tr("none", "无").to_string(),
            muted: true,
            note: None,
        }
    }

    /// Columns taken by the cell's last line, note included.
    fn width(&self) -> usize {
        let last = self.text.lines().last().unwrap_or("");
        display_width(last) + self.note.as_ref().map_or(0, |n| 1 + display_width(n))
    }

    fn fits_a_column(&self) -> bool {
        self.text.chars().count() <= MAX_CELL && !self.text.contains('\n') && self.note.is_none()
    }

    fn show(&self, paint: Paint) -> String {
        let text = if self.muted {
            paint.paint(Role::Muted, &self.text)
        } else {
            self.text.clone()
        };
        match &self.note {
            Some(note) => format!("{text} {}", paint.paint(Role::Muted, note)),
            None => text,
        }
    }
}

/// Scalars and empty collections print on one line.
fn is_scalar(value: &Value) -> bool {
    match value {
        Value::Object(map) => map.is_empty(),
        Value::Array(items) => items.is_empty(),
        _ => true,
    }
}

fn scalar(value: &Value) -> Cell {
    match value {
        Value::Bool(b) => Cell::plain(b.to_string()),
        Value::Number(n) => Cell::plain(n.to_string()),
        Value::String(s) => truncate(s, MAX_STRING),
        Value::Null | Value::Object(_) | Value::Array(_) => Cell::none(),
    }
}

fn truncate(text: &str, max: usize) -> Cell {
    let count = text.chars().count();
    if count <= max {
        return Cell::plain(text.to_string());
    }
    let head: String = text.chars().take(max).collect();
    Cell {
        text: format!("{head}…"),
        muted: false,
        note: Some(
            tr("{count} characters in all", "共 {count} 个字符")
                .replace("{count}", &count.to_string()),
        ),
    }
}

/// Terminal columns taken by `text`: CJK and emoji count double, a combining mark, a variation selector or a skin
/// tone joined to an emoji count nothing (Unicode's East Asian width, as `unicode-width` gives it).
pub fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

/// `shown` (already painted) followed by spaces up to `width` columns, `visible` being its width unpainted.
fn padded(shown: &str, visible: usize, width: usize) -> String {
    format!("{shown}{}", " ".repeat(width.saturating_sub(visible)))
}

fn push_scalar_lines(prefix: &str, cell: &Cell, indent: usize, paint: Paint, out: &mut String) {
    let shown = cell.show(paint);
    let mut lines = shown.lines();
    out.push_str(prefix);
    out.push_str(lines.next().unwrap_or(""));
    out.push('\n');
    for line in lines {
        out.push_str(&" ".repeat(indent + 2));
        out.push_str(line);
        out.push('\n');
    }
}

fn block(value: &Value, indent: usize, paint: Paint, out: &mut String) {
    let pad_in = " ".repeat(indent);
    match value {
        Value::Object(map) if !map.is_empty() => {
            let width = map
                .iter()
                .filter(|(_, v)| is_scalar(v))
                .map(|(k, _)| display_width(k) + 1)
                .max()
                .unwrap_or(0)
                .min(33);
            for (key, v) in map {
                let label = format!("{key}:");
                if is_scalar(v) {
                    let key = padded(
                        &paint.paint(Role::Muted, &label),
                        display_width(&label),
                        width,
                    );
                    push_scalar_lines(&format!("{pad_in}{key}  "), &scalar(v), indent, paint, out);
                } else {
                    out.push_str(&format!("{pad_in}{}\n", paint.paint(Role::Strong, &label)));
                    block(v, indent + 2, paint, out);
                }
            }
        }
        Value::Array(items) if !items.is_empty() => {
            if let Some(table) = table(items, paint) {
                for line in table {
                    out.push_str(&pad_in);
                    out.push_str(&line);
                    out.push('\n');
                }
                return;
            }
            for item in items {
                if is_scalar(item) {
                    push_scalar_lines(&format!("{pad_in}- "), &scalar(item), indent, paint, out);
                } else {
                    let mut inner = String::new();
                    block(item, indent + 2, paint, &mut inner);
                    let rest = inner.get(indent + 2..).unwrap_or("");
                    out.push_str(&format!("{pad_in}- {rest}"));
                }
            }
        }
        other => push_scalar_lines(&pad_in, &scalar(other), indent, paint, out),
    }
}

/// Rows of an aligned table, when every item is a flat object with few, short cells.
fn table(items: &[Value], paint: Paint) -> Option<Vec<String>> {
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
    let cell = |item: &Value, column: &str| -> Option<Cell> {
        let cell = item.get(column).map_or_else(Cell::none, scalar);
        cell.fits_a_column().then_some(cell)
    };
    let mut rows: Vec<Vec<Cell>> = Vec::with_capacity(items.len());
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
                .map(|r| r[i].width())
                .chain([display_width(c)])
                .max()
                .unwrap_or(0)
        })
        .collect();
    let line = |cells: Vec<(String, usize)>| {
        let parts: Vec<String> = cells
            .iter()
            .zip(&widths)
            .map(|((shown, visible), w)| padded(shown, *visible, *w))
            .collect();
        parts.join("  ").trim_end().to_string()
    };
    let header = columns
        .iter()
        .map(|c| (paint.paint(Role::Strong, c), display_width(c)))
        .collect();
    let mut lines = vec![line(header)];
    for row in &rows {
        lines.push(line(
            row.iter().map(|c| (c.show(paint), c.width())).collect(),
        ));
    }
    Some(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn plain(value: Value) -> String {
        render(&value, Paint::plain())
    }

    #[test]
    fn objects_align_and_nest() {
        let out = plain(json!({ "name": "a", "longer": 2, "inner": { "x": true } }));
        assert_eq!(out, "name:    a\nlonger:  2\ninner:\n  x:  true\n");
    }

    #[test]
    fn flat_object_arrays_become_tables() {
        let out = plain(json!([{ "id": 1, "name": "张三" }, { "id": 22, "name": "li" }]));
        assert_eq!(out, "id  name\n1   张三\n22  li\n");
    }

    #[test]
    fn nested_arrays_use_bullets() {
        let out = plain(json!([{ "a": 1, "b": [1, 2] }, "x"]));
        assert_eq!(out, "- a:  1\n  b:\n    - 1\n    - 2\n- x\n");
    }

    #[test]
    fn long_strings_are_cut() {
        let out = plain(json!({ "blob": "x".repeat(400) }));
        assert!(out.contains("… 400 characters in all"), "{out}");
        assert!(out.len() < 400);
    }

    #[test]
    fn no_value_is_a_word_without_brackets() {
        let out = plain(json!({ "a": null, "b": [], "c": {} }));
        assert_eq!(out, "a:  none\nb:  none\nc:  none\n");
        assert_eq!(plain(json!([])), "none\n");
    }

    #[test]
    fn colours_do_not_shift_the_columns() {
        let paint = Paint::new(crate::colour::Depth::Extended);
        let value = json!([{ "id": 1, "name": null }, { "id": 22, "name": "li" }]);
        let strip = |text: &str| {
            let mut out = String::new();
            let mut chars = text.chars();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    chars.by_ref().find(|c| *c == 'm');
                } else {
                    out.push(c);
                }
            }
            out
        };
        assert_eq!(strip(&render(&value, paint)), plain(value));
    }

    #[test]
    fn errors_put_what_to_do_and_the_reason_on_lines_of_their_own() {
        let out = render_error(
            "cannot hook WeChat\nRestart the Mac.\nReason: boom",
            Some(&json!({ "failureKind": "not_found" })),
            Paint::plain(),
        );
        assert_eq!(
            out,
            "error: cannot hook WeChat\nRestart the Mac.\nReason: boom\n  failureKind:  not_found\n"
        );
    }

    #[test]
    fn a_painted_label_keeps_its_space_and_a_full_width_colon_takes_none() {
        let paint = Paint::new(crate::colour::Depth::Basic);
        assert_eq!(
            labelled("error:", Role::Error, "boom", paint),
            "\x1b[1;31merror:\x1b[0m boom\n"
        );
        assert_eq!(
            labelled("错误：", Role::Error, "boom", Paint::plain()),
            "错误：boom\n"
        );
        assert_eq!(
            field("网页：", "http://x", Paint::plain()),
            "网页：http://x\n"
        );
        assert_eq!(field("url:", "http://x", Paint::plain()), "url: http://x\n");
    }
}
