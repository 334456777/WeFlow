//! 序列化和转义：JSON 的格式化器，以及 HTML、CSV 和 SQL 的转义。

use std::io;

use serde::Serialize;
use serde_json::ser::Formatter;

/// 把字符串里每个 `<` 写成 `\u003c` 的 `serde_json` 紧凑格式。这样 HTML 的分词器在 JSON 里完全看不到 `<`：
/// 没有会提前结束所在 `<script>` 元素的 `</script`，也没有 `<!--` 加 `<script`，后者会让元素进入双重转义状态，
/// 页面自己的 `</script>` 就不再能结束它。
struct ScriptSafe;

impl Formatter for ScriptSafe {
    // 字符串以外的 JSON 里没有 `<`；字符串里的 `<` serde_json 从不转义，所以都会经过这个方法
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

/// 把 `value` 写成可以安全放进 `<script>` 元素的紧凑 JSON（见 [`ScriptSafe`]）。
pub(super) fn write_script_json(
    writer: &mut impl io::Write,
    value: &impl Serialize,
) -> io::Result<()> {
    let mut serializer = serde_json::Serializer::with_formatter(writer, ScriptSafe);
    value.serialize(&mut serializer).map_err(io::Error::from)
}

/// `serde_json` 的美化输出（缩进两格），用于逐段写出的文档中缩进 `base` 个空格的值：
/// 第一行之后的每一行再多缩进 `base` 个空格。
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

/// 像 `serde_json::to_string_pretty` 那样美化输出 `value`，第一行之后的每一行多缩进 `indent` 个空格，
/// 用于逐段写出的文档中嵌套到这个深度的值。
pub(super) fn write_pretty(
    writer: &mut impl io::Write,
    value: &impl Serialize,
    indent: usize,
) -> io::Result<()> {
    let mut serializer = serde_json::Serializer::with_formatter(writer, NestedPretty::new(indent));
    value.serialize(&mut serializer).map_err(io::Error::from)
}

/// 转义 HTML 中的 `&`、`<`、`>`、`"` 和 `'`。
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

/// CSV 的一个单元格（`escapeCsvValue`）：含引号、逗号或换行时加上引号，其中的引号写两遍；否则原样写出。
///
/// 单元格原样保留文字，用于给程序读的 CSV（WeClone 的）。给人在电子表格里打开的 CSV 用 [`csv_text_cell`] 写文字。
pub fn csv_cell(value: &str) -> String {
    if value.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// 给人在电子表格里打开的 CSV 的一个文字单元格：文字以 `=`、`+`、`-`、`@`、制表符或回车开头且不是数字时，
/// 先在前面加一个 `'` 再写成 [`csv_cell`]，否则直接写成 [`csv_cell`]。
///
/// Excel 把这样开头的单元格当成公式，加了引号也一样；前面有 `'` 时显示文字。`-5` 这样的数字仍是数字（#1219）。
pub fn csv_text_cell(value: &str) -> String {
    if value.starts_with(['=', '+', '-', '@', '\t', '\r']) && !is_plain_number(value) {
        csv_cell(&format!("'{value}"))
    } else {
        csv_cell(value)
    }
}

/// `value` 是否为十进制数：可选的正负号、最多带一个 `.` 的数字，以及可选的指数。
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

/// 转义 SQL 字符串字面量里的单引号（写两遍）。
pub fn sql_escape(value: &str) -> String {
    value.replace('\'', "''")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn sql_escapes() {
        assert_eq!(sql_escape("it's"), "it''s");
    }

    /// Excel 会当成公式的文字前面加一个 `'`，去掉它就是原来的文字；数字和其他文字保持原样（#1219）。
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

    /// 脚本安全的 JSON 是把每个 `<` 都写成 `\u003c` 的紧凑 JSON，紧挨着转义字符的也是，读回来是同一个值。
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

    /// 消息里先有 `<!--` 再有 `<script` 时，页面的 `<script>` 元素会进入双重转义状态，
    /// 页面自己的 `</script>` 结束不了它：两者都不会出现在页面里。
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
