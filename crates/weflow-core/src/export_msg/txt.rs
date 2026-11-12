//! The TXT export.

use std::path::Path;

use anyhow::Result;

use crate::message::time::format_timestamp;
use crate::message::xml::MsgXml;
use crate::message::*;

use super::{write_bytes, Exporter};

impl<'a, 'n> Exporter<'a, 'n> {
    /// One message of the TXT export: `<time> '<sender>'`, the text, a blank line.
    /// Appends the TXT entry of `msg` to `out`.
    pub(super) fn write_txt_entry(&mut self, msg: &ExportMsg, out: &mut Vec<u8>) {
        use std::io::Write;
        let (content, _) = self.plain_row_content(msg, &MsgXml::new(&msg.content));
        let sender = self.sender_fields(msg);
        // writing to a Vec cannot fail
        let _ = write!(
            out,
            "{} '{}'\n{}\n\n",
            format_timestamp(msg.create_time),
            sender.role,
            content
        );
    }

    pub(super) fn render_txt(&mut self, msgs: &[ExportMsg]) -> String {
        let mut out = Vec::new();
        for msg in msgs {
            self.write_txt_entry(msg, &mut out);
        }
        // the entries are written from strings, so the bytes are UTF-8
        String::from_utf8(out).unwrap_or_default()
    }

    pub fn write_txt(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let text = self.render_txt(msgs);
        write_bytes(out, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;

    #[test]
    fn txt_layout() {
        let text = with_exporter(false, |ex, msgs| ex.render_txt(msgs));
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines[0].ends_with(" 'Me'") || lines[0].ends_with(" '我'"),
            "{}",
            lines[0]
        );
        assert_eq!(lines[1], "hello \"world\", ok");
        assert!(text.contains(" 'Bobby'\nhi <b>there</b>\n\n"));
        assert!(text.contains("[图片]"));
        assert!(text.contains("[链接] Doc\nhttps://example.com/x"));
        assert!(text.contains("yes[引用 Me：hello]"));
    }
}
