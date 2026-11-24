//! Serializing and escaping: the JSON formatters and the escapes of HTML, CSV and SQL.

use std::io;

use serde::Serialize;
use serde_json::ser::Formatter;

/// `serde_json`'s compact format with every `<` inside a string written as `\u003c`. The HTML tokenizer then sees no
/// `<` in the JSON at all: no `</script` that would close the `<script>` element it is embedded in, and no `<!--`
/// followed by `<script` that would put the element in its double-escaped state, where the page's own `</script>`
/// no longer closes it.
struct ScriptSafe;

impl Formatter for ScriptSafe {
    // outside strings JSON has no `<`; inside them serde_json never escapes it, so it reaches this method
    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut parts = fragment.split('<');
        if let Some(first) = parts.next() {
            writer.write_all(first.as_bytes())?;
        }
        for part in parts {
            writer.write_all(b"\\u003c")?;
            writer.write_all(part.as_bytes())?;
        }
        Ok(())
    }
}

/// Writes `value` as compact JSON that is safe inside a `<script>` element (see [`ScriptSafe`]).
pub(super) fn write_script_json(
    writer: &mut impl io::Write,
    value: &impl Serialize,
) -> io::Result<()> {
    let mut serializer = serde_json::Serializer::with_formatter(writer, ScriptSafe);
    value.serialize(&mut serializer).map_err(io::Error::from)
}

/// `serde_json`'s pretty printing (two-space indent) for a value that sits `base` spaces deep in a document written
/// piece by piece: every line after the first is indented by `base` more spaces.
struct NestedPretty {
    base: usize,
    depth: usize,
    has_value: bool,
}

impl NestedPretty {
    fn new(base: usize) -> Self {
        Self {
            base,
            depth: 0,
            has_value: false,
        }
    }

    fn new_line<W: ?Sized + io::Write>(&self, writer: &mut W) -> io::Result<()> {
        const SPACES: &[u8; 32] = b"                                ";
        writer.write_all(b"\n")?;
        let mut left = self.base + 2 * self.depth;
        while left > 0 {
            let n = left.min(SPACES.len());
            writer.write_all(&SPACES[..n])?;
            left -= n;
        }
        Ok(())
    }
}

impl Formatter for NestedPretty {
    fn begin_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        writer.write_all(b"[")
    }

    fn end_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth -= 1;
        if self.has_value {
            self.new_line(writer)?;
        }
        writer.write_all(b"]")
    }

    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if !first {
            writer.write_all(b",")?;
        }
        self.new_line(writer)
    }

    fn end_array_value<W: ?Sized + io::Write>(&mut self, _writer: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }

    fn begin_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        writer.write_all(b"{")
    }

    fn end_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth -= 1;
        if self.has_value {
            self.new_line(writer)?;
        }
        writer.write_all(b"}")
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if !first {
            writer.write_all(b",")?;
        }
        self.new_line(writer)
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }

    fn end_object_value<W: ?Sized + io::Write>(&mut self, _writer: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
}

/// Writes `value` pretty-printed like `serde_json::to_string_pretty`, with every line after the first indented by
/// `indent` spaces, for a value nested that deep in a document written piece by piece.
pub(super) fn write_pretty(
    writer: &mut impl io::Write,
    value: &impl Serialize,
    indent: usize,
) -> io::Result<()> {
    let mut serializer = serde_json::Serializer::with_formatter(writer, NestedPretty::new(indent));
    value.serialize(&mut serializer).map_err(io::Error::from)
}

pub fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// A CSV cell (`escapeCsvValue`): in quotes, with its quotes doubled, when it holds a quote, a comma or a line
/// break; as it is otherwise.
///
/// The cell keeps the text exactly, for CSV a program reads (WeClone's). CSV a person opens in a spreadsheet writes
/// its text with [`csv_text_cell`].
pub fn csv_cell(value: &str) -> String {
    if value.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// A CSV cell for text a person opens in a spreadsheet: [`csv_cell`] of the text after a `'` when it starts with `=`,
/// `+`, `-`, `@`, a tab or a carriage return and is not a number, [`csv_cell`] of the text otherwise.
///
/// Excel reads a cell that starts so as a formula even in quotes; after the `'` it shows the text, and numbers such
/// as `-5` stay numbers (#1219).
pub fn csv_text_cell(value: &str) -> String {
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) && !is_plain_number(value) {
        csv_cell(&format!("'{value}"))
    } else {
        csv_cell(value)
    }
}

/// Whether `value` is a decimal number: an optional sign, digits with at most one `.`, and an optional exponent.
fn is_plain_number(value: &str) -> bool {
    let all_digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let (mantissa, exponent) = match unsigned.split_once(['e', 'E']) {
        Some((mantissa, exponent)) => (mantissa, Some(exponent)),
        None => (unsigned, None),
    };
    let (whole, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
    let exponent_ok = exponent.is_none_or(|e| {
        let digits = e.strip_prefix(['+', '-']).unwrap_or(e);
        !digits.is_empty() && all_digits(digits)
    });
    !(whole.is_empty() && fraction.is_empty())
        && all_digits(whole)
        && all_digits(fraction)
        && exponent_ok
}

pub fn sql_escape(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Text that Excel would read as a formula gets a `'` in front, and reads back as the same text without it;
    /// numbers and other text stay as they are (#1219).
    #[test]
    fn a_text_cell_that_starts_like_a_formula_gets_an_apostrophe() {
        for text in [
            "=1+1",
            "-_-",
            "@所有人",
            "+abc",
            "\tx",
            "\r\nx",
            "-",
            "+",
            "-.",
            "-1.2.3",
            "-1e",
            "-inf",
            "=HYPERLINK(\"http://a\",\"b\")",
        ] {
            let cell = csv_text_cell(text);
            let inner = cell
                .strip_prefix('"')
                .and_then(|c| c.strip_suffix('"'))
                .map_or_else(|| cell.clone(), |c| c.replace("\"\"", "\""));
            assert_eq!(inner.strip_prefix('\''), Some(text), "{text:?} -> {cell:?}");
        }
        for text in [
            "-5", "+1", "3.14", "-0.5", "-.5", "1e5", "-2E-3", "中文", "", "a=b", "'x", "a, b",
        ] {
            assert_eq!(csv_text_cell(text), csv_cell(text), "{text:?}");
        }
    }

    fn script_json(value: &serde_json::Value) -> String {
        let mut out = Vec::new();
        write_script_json(&mut out, value).unwrap();
        String::from_utf8(out).unwrap()
    }

    /// Script-safe JSON is compact JSON with every `<` written as `\u003c`, also next to escaped characters, and
    /// reads back as the same value.
    #[test]
    fn script_json_escapes_every_opening_angle_bracket() {
        for text in [
            "</script>",
            "a</b></c>",
            "<</",
            "\"</\"\n</",
            "<\\/",
            "</",
            "<",
            "a<b<",
            "plain",
        ] {
            let value = json!({ "b": text, "</k": [text, 1] });
            let got = script_json(&value);
            let want = serde_json::to_string(&value)
                .unwrap()
                .replace('<', "\\u003c");
            assert_eq!(got, want, "{text:?}");
            assert!(!got.contains('<'), "{got}");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&got).unwrap(),
                value
            );
        }
    }

    /// `<!--` and then `<script` in a message would put the page's `<script>` element in its double-escaped state,
    /// where the page's own `</script>` does not close it: neither reaches the page.
    #[test]
    fn script_json_never_opens_a_comment_or_a_script() {
        let text = "<!--<script>alert(1)</script>-->";
        let got = script_json(&json!({ "c": text }));
        assert_eq!(
            got,
            "{\"c\":\"\\u003c!--\\u003cscript>alert(1)\\u003c/script>-->\"}"
        );
        for tag in ["<!--", "<script", "</script"] {
            assert!(!got.contains(tag), "{tag} in {got}");
        }
    }
}
