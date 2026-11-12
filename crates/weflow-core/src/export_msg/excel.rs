//! The Excel export.

use std::path::Path;

use anyhow::{Context, Result};

use crate::message::kind::message_type_name_in;
use crate::message::link::extract_html_link_card;
use crate::message::time::format_timestamp;
use crate::message::xml::{strip_sender_prefix, MsgXml};
use crate::message::*;

use super::{chrome, write_bytes, Exporter};

impl<'a, 'n> Exporter<'a, 'n> {
    // ─────────────────────────────────── Excel ───────────────────────────────────

    /// A sheet with more rows than this is written in constant-memory mode (no hyperlinks, rows go to disk).
    pub(super) const EXCEL_BIG: usize = 20000;

    pub fn write_excel(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        self.write_excel_rows(msgs, msgs.len() > Self::EXCEL_BIG, out)?;
        Ok(())
    }

    /// Excel written from a stream of messages (oldest first). Whether the sheet is "big" depends on the total, so
    /// the first `EXCEL_BIG + 1` messages are held back: if the stream ends before that, the small sheet is written
    /// as usual; otherwise the constant-memory sheet takes them and then the rest as they arrive.
    pub(super) fn stream_excel(
        &mut self,
        msgs: &mut dyn Iterator<Item = ExportMsg>,
        out: &Path,
    ) -> Result<usize> {
        let head: Vec<ExportMsg> = (&mut *msgs).take(Self::EXCEL_BIG + 1).collect();
        if head.is_empty() {
            return Ok(0);
        }
        if head.len() <= Self::EXCEL_BIG {
            self.write_excel(&head, out)?;
            return Ok(head.len());
        }
        self.write_excel_rows(head.into_iter().chain(msgs), true, out)
    }

    /// Writes the sheet; `big` picks the constant-memory mode (rows must then be written in order, as they are).
    /// Returns the number of messages.
    fn write_excel_rows<I>(&mut self, msgs: I, big: bool, out: &Path) -> Result<usize>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<ExportMsg>,
    {
        use rust_xlsxwriter::{Color, Format, FormatAlign, Url, Workbook};
        use std::borrow::Borrow;
        let is_group = self.session.is_group;
        let compact = self.settings.excel_compact;
        let include_group_col = !compact && is_group;
        let mut workbook = Workbook::new();
        let ws = if big {
            workbook.add_worksheet_with_constant_memory()?
        } else {
            workbook.add_worksheet()
        };
        ws.set_name(chrome("聊天记录", "Chat history"))?;
        let bold = Format::new().set_bold();
        let head = Format::new()
            .set_bold()
            .set_background_color(Color::RGB(0xE8F5E9))
            .set_align(FormatAlign::Center);
        let link_fmt = Format::new()
            .set_font_color(Color::RGB(0x0563C1))
            .set_underline(rust_xlsxwriter::FormatUnderline::Single);

        let session = self.session.clone();
        let session_contact = self.names.get(&session.id);
        let s_nick = session_contact
            .as_ref()
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| session.display_name.clone());
        let s_remark = session_contact
            .as_ref()
            .map(|c| c.remark.clone())
            .unwrap_or_default();

        ws.write_string_with_format(0, 0, chrome("会话信息", "Conversation"), &bold)?;
        ws.write_string_with_format(1, 0, chrome("微信ID", "WeChat ID"), &bold)?;
        ws.write_string(1, 1, &session.id)?;
        ws.write_string_with_format(1, 3, chrome("昵称", "Nickname"), &bold)?;
        ws.write_string(1, 4, &s_nick)?;
        if is_group {
            ws.write_string_with_format(1, 5, chrome("备注", "Remark"), &bold)?;
            ws.write_string(1, 6, &s_remark)?;
        }
        ws.write_string_with_format(2, 0, chrome("导出工具", "Exported by"), &bold)?;
        ws.write_string(2, 1, "WeFlow")?;
        ws.write_string_with_format(2, 2, chrome("导出版本", "Version"), &bold)?;
        ws.write_string(2, 3, "0.0.2")?;
        ws.write_string_with_format(2, 4, chrome("平台", "Platform"), &bold)?;
        ws.write_string(2, 5, "wechat")?;
        ws.write_string_with_format(2, 6, chrome("导出时间", "Exported at"), &bold)?;
        ws.write_string(2, 7, format_timestamp(self.settings.exported_at))?;

        let headers: Vec<&str> = if compact {
            vec![
                chrome("序号", "No."),
                chrome("时间", "Time"),
                chrome("发送者身份", "Sender"),
                chrome("消息类型", "Message type"),
                chrome("内容", "Content"),
            ]
        } else if include_group_col {
            vec![
                chrome("序号", "No."),
                chrome("时间", "Time"),
                chrome("发送者昵称", "Sender nickname"),
                chrome("发送者微信ID", "Sender WeChat ID"),
                chrome("发送者备注", "Sender remark"),
                chrome("群昵称", "Group nickname"),
                chrome("发送者身份", "Sender"),
                chrome("消息类型", "Message type"),
                chrome("内容", "Content"),
            ]
        } else {
            vec![
                chrome("序号", "No."),
                chrome("时间", "Time"),
                chrome("发送者昵称", "Sender nickname"),
                chrome("发送者微信ID", "Sender WeChat ID"),
                chrome("发送者备注", "Sender remark"),
                chrome("发送者身份", "Sender"),
                chrome("消息类型", "Message type"),
                chrome("内容", "Content"),
            ]
        };
        for (i, h) in headers.iter().enumerate() {
            ws.write_string_with_format(3, u16::try_from(i)?, *h, &head)?;
        }
        let widths: Vec<f64> = if compact {
            vec![8.0, 20.0, 18.0, 12.0, 50.0]
        } else if include_group_col {
            vec![8.0, 20.0, 18.0, 25.0, 18.0, 18.0, 15.0, 12.0, 50.0]
        } else {
            vec![8.0, 20.0, 18.0, 25.0, 18.0, 15.0, 12.0, 50.0]
        };
        for (i, w) in widths.iter().enumerate() {
            ws.set_column_width(u16::try_from(i)?, *w)?;
        }

        let content_col = u16::try_from(headers.len() - 1)?;
        let mut count = 0usize;
        for (i, item) in msgs.into_iter().enumerate() {
            let msg: &ExportMsg = item.borrow();
            count += 1;
            let row = u32::try_from(i + 4)?;
            let x = MsgXml::new(&msg.content);
            let (content, has_quote) = self.plain_row_content(msg, &x);
            let s = self.sender_fields(msg);
            let type_name = message_type_name_in(msg.local_type, &x);
            ws.write_number(row, 0, (i + 1) as f64)?;
            ws.write_string(row, 1, format_timestamp(msg.create_time))?;
            if compact {
                ws.write_string(row, 2, &s.role)?;
                ws.write_string(row, 3, type_name)?;
            } else if include_group_col {
                ws.write_string(row, 2, &s.nickname)?;
                ws.write_string(row, 3, &s.wxid)?;
                ws.write_string(row, 4, &s.remark)?;
                ws.write_string(row, 5, &s.group_nickname)?;
                ws.write_string(row, 6, &s.role)?;
                ws.write_string(row, 7, type_name)?;
            } else {
                ws.write_string(row, 2, &s.nickname)?;
                ws.write_string(row, 3, &s.wxid)?;
                ws.write_string(row, 4, &s.remark)?;
                ws.write_string(row, 5, &s.role)?;
                ws.write_string(row, 6, type_name)?;
            }
            let link = if has_quote {
                None
            } else {
                extract_html_link_card(&msg.content, msg.local_type)
            };
            let mut wrote_link = false;
            if let Some(card) = link {
                if !big && card.url.len() < 2000 {
                    let title: String = strip_sender_prefix(card.title.trim());
                    let title = if title.is_empty() {
                        card.url.clone()
                    } else {
                        title
                    };
                    wrote_link = ws
                        .write_url_with_options(
                            row,
                            content_col,
                            Url::new(card.url.as_str()),
                            title.as_str(),
                            card.url.as_str(),
                            Some(&link_fmt),
                        )
                        .is_ok();
                }
            }
            if !wrote_link {
                let clipped: String = content.chars().take(32000).collect();
                ws.write_string(row, content_col, clipped)?;
            }
        }
        // Built in memory, then written: rust_xlsxwriter panics (an unwrap) when writing a constant-memory sheet to
        // the file fails, for instance on a full disk, while writing to memory cannot fail.
        let bytes = workbook
            .save_to_buffer()
            .with_context(|| format!("failed to build {}", out.display()))?;
        write_bytes(out, &bytes)?;
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;
    use std::fs;
    use weflow_native::fixture::TestDir;

    #[test]
    fn excel_writes_file() {
        let dir = TestDir::new("weflow-xlsx");
        let out = dir.join("chat.xlsx");
        with_exporter(true, |ex, msgs| ex.write_excel(msgs, &out).unwrap());
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[..2], b"PK");
        assert!(bytes.len() > 2000);
    }
}
