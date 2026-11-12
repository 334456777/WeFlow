//! Serializing and escaping: the JSON formatters and the escapes of HTML, CSV and SQL.

use std::io;

use serde::Serialize;
use serde_json::ser::Formatter;

/// serde_json's compact format with every `</` inside a string written as `<\/`, so the JSON cannot close the
/// `<script>` element it is embedded in.
struct ScriptSafe;

impl Formatter for ScriptSafe {
    // `<` and `/` are never escaped, so a `</` of the text always lies within one fragment
    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut rest = fragment;
        while let Some(at) = rest.find("</") {
            writer.write_all(&rest.as_bytes()[..=at])?;
            writer.write_all(b"\\")?;
            rest = &rest[at + 1..];
        }
        writer.write_all(rest.as_bytes())
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

/// serde_json's pretty printing (two-space indent) for a value that sits `base` spaces deep in a document written
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
pub fn csv_cell(value: &str) -> String {
    if value.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn sql_escape(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Script-safe JSON is compact JSON with every `</` written as `<\/`, also next to escaped characters.
    #[test]
    fn script_json_escapes_every_closing_tag() {
        for text in [
            "</script>",
            "a</b></c>",
            "<</",
            "\"</\"\n</",
            "<\\/",
            "</",
            "plain",
        ] {
            let value = json!({ "b": text, "</k": [text, 1] });
            let mut out = Vec::new();
            write_script_json(&mut out, &value).unwrap();
            let want = serde_json::to_string(&value).unwrap().replace("</", "<\\/");
            assert_eq!(String::from_utf8(out).unwrap(), want, "{text:?}");
        }
    }
}
