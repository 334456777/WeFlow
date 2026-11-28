//! TXT 导出。

use std::path::Path;

use anyhow::Result;

use crate::message::time::format_timestamp;
use crate::message::xml::MsgXml;
use crate::message::*;

use super::{write_bytes, Exporter};

impl Exporter<'_, '_> {
    /// 把 `msg` 的 TXT 条目追加到 `out`：一行 `<时间> '<发送者>'`，然后是文字和一个空行。
    pub(super) fn write_txt_entry(&mut self, msg: &ExportMsg, out: &mut Vec<u8>) {
        use std::io::Write;
        let (content, _) = self.plain_row_content(msg, &MsgXml::of_export(msg));
        let sender = self.sender_fields(msg);
        // 写进 Vec 不会失败
        let _ = write!(
            out,
            "{} '{}'\n{}\n\n",
            format_timestamp(msg.create_time),
            sender.role,
            content
        );
    }

    /// 整个 TXT 导出的文字。
    pub(super) fn render_txt(&mut self, msgs: &[ExportMsg]) -> String {
        let mut out = Vec::new();
        for msg in msgs {
            self.write_txt_entry(msg, &mut out);
        }
        // 各条目都由字符串写成，字节一定是 UTF-8
        String::from_utf8(out).unwrap_or_default()
    }

    /// 把 `msgs` 写成纯文本文件 `out`。
    ///
    /// # Errors
    ///
    /// 无法写入文件时返回错误。
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
