//! The Excel export.

use std::path::Path;

use anyhow::{Context, Result};

use crate::message::kind::message_type_name_in;
use crate::message::link::extract_html_link_card;
use crate::message::time::format_timestamp;
use crate::message::xml::{strip_sender_prefix, MsgXml};
use crate::message::*;

use super::{chrome, write_bytes, Exporter};

/// Most rows an Excel sheet holds (Excel's limit): the messages that do not fit go on in another sheet.
const SHEET_ROWS: u32 = 1_048_576;
/// Rows above the messages of every sheet: the conversation's details, then the column headers.
const HEAD_ROWS: u32 = 4;

impl<'a, 'n> Exporter<'a, 'n> {
    // ─────────────────────────────────── Excel ───────────────────────────────────

    /// A sheet with more rows than this is written in constant-memory mode (no hyperlinks, rows go to disk).
    pub(super) const EXCEL_BIG: usize = 20000;

    /// Writes `msgs` to `out` as an Excel workbook, a new sheet every 1 048 576 rows.
    ///
    /// # Errors
    /// When the workbook cannot be written.
    pub fn write_excel(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        self.write_excel_rows(msgs, msgs.len() > Self::EXCEL_BIG, out, SHEET_ROWS)?;
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
        self.write_excel_rows(head.into_iter().chain(msgs), true, out, SHEET_ROWS)
    }

    /// Writes the workbook; `big` picks the constant-memory mode (rows must then be written in order, as they are).
    /// A sheet holds `sheet_rows` rows ([`SHEET_ROWS`] but in tests): the messages that do not fit go on in another
    /// sheet, under the same rows on top, numbered on. Returns the number of messages.
    fn write_excel_rows<I>(
        &mut self,
        msgs: I,
        big: bool,
        out: &Path,
        sheet_rows: u32,
    ) -> Result<usize>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<ExportMsg>,
    {
        use rust_xlsxwriter::{Color, Format, Url, Workbook};
        use std::borrow::Borrow;
        let is_group = self.session.is_group;
        let compact = self.settings.excel_compact;
        let include_group_col = !compact && is_group;
        let mut workbook = Workbook::new();
        let link_fmt = Format::new()
            .set_font_color(Color::RGB(0x0563C1))
            .set_underline(rust_xlsxwriter::FormatUnderline::Single);

        let session = self.session.clone();
        let session_contact = self.names.get(&session.id);
        let nickname = session_contact
            .as_ref()
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| session.display_name.clone());
        let remark = is_group.then(|| {
            session_contact
                .as_ref()
                .map(|c| c.remark.clone())
                .unwrap_or_default()
        });

        let headers: Vec<&'static str> = if compact {
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
        let widths: Vec<f64> = if compact {
            vec![8.0, 20.0, 18.0, 12.0, 50.0]
        } else if include_group_col {
            vec![8.0, 20.0, 18.0, 25.0, 18.0, 18.0, 15.0, 12.0, 50.0]
        } else {
            vec![8.0, 20.0, 18.0, 25.0, 18.0, 15.0, 12.0, 50.0]
        };
        let top = SheetTop {
            session_id: session.id,
            nickname,
            remark,
            exported_at: format_timestamp(self.settings.exported_at),
            headers,
            widths,
        };

        let per_sheet = usize::try_from(sheet_rows.saturating_sub(HEAD_ROWS).max(1))?;
        let content_col = u16::try_from(top.headers.len() - 1)?;
        let mut ws = top.add_sheet(&mut workbook, 0, big)?;
        let mut count = 0usize;
        for (i, item) in msgs.into_iter().enumerate() {
            let msg: &ExportMsg = item.borrow();
            count += 1;
            if i > 0 && i % per_sheet == 0 {
                ws = top.add_sheet(&mut workbook, i / per_sheet, big)?;
            }
            let row = HEAD_ROWS + u32::try_from(i % per_sheet)?;
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

/// What every sheet of an Excel export has above its messages: the conversation's details, then the column headers.
struct SheetTop {
    session_id: String,
    nickname: String,
    /// The group's remark (a group only).
    remark: Option<String>,
    exported_at: String,
    headers: Vec<&'static str>,
    widths: Vec<f64>,
}

impl SheetTop {
    /// Adds sheet number `index` (from 0) to `workbook`, in constant-memory mode when `big`, with the rows on top.
    fn add_sheet<'w>(
        &self,
        workbook: &'w mut rust_xlsxwriter::Workbook,
        index: usize,
        big: bool,
    ) -> Result<&'w mut rust_xlsxwriter::Worksheet> {
        use rust_xlsxwriter::{Color, Format, FormatAlign};
        let ws = if big {
            workbook.add_worksheet_with_constant_memory()?
        } else {
            workbook.add_worksheet()
        };
        let name = chrome("聊天记录", "Chat history");
        match index {
            0 => ws.set_name(name)?,
            _ => ws.set_name(format!("{name} ({})", index + 1))?,
        };
        let bold = Format::new().set_bold();
        let head = Format::new()
            .set_bold()
            .set_background_color(Color::RGB(0xE8F5E9))
            .set_align(FormatAlign::Center);
        ws.write_string_with_format(0, 0, chrome("会话信息", "Conversation"), &bold)?;
        ws.write_string_with_format(1, 0, chrome("微信ID", "WeChat ID"), &bold)?;
        ws.write_string(1, 1, &self.session_id)?;
        ws.write_string_with_format(1, 3, chrome("昵称", "Nickname"), &bold)?;
        ws.write_string(1, 4, &self.nickname)?;
        if let Some(remark) = &self.remark {
            ws.write_string_with_format(1, 5, chrome("备注", "Remark"), &bold)?;
            ws.write_string(1, 6, remark)?;
        }
        ws.write_string_with_format(2, 0, chrome("导出工具", "Exported by"), &bold)?;
        ws.write_string(2, 1, "WeFlow")?;
        ws.write_string_with_format(2, 2, chrome("导出版本", "Version"), &bold)?;
        ws.write_string(2, 3, "0.0.2")?;
        ws.write_string_with_format(2, 4, chrome("平台", "Platform"), &bold)?;
        ws.write_string(2, 5, "wechat")?;
        ws.write_string_with_format(2, 6, chrome("导出时间", "Exported at"), &bold)?;
        ws.write_string(2, 7, &self.exported_at)?;
        for (i, h) in self.headers.iter().enumerate() {
            ws.write_string_with_format(HEAD_ROWS - 1, u16::try_from(i)?, *h, &head)?;
        }
        for (i, w) in self.widths.iter().enumerate() {
            ws.set_column_width(u16::try_from(i)?, *w)?;
        }
        Ok(ws)
    }
}

#[cfg(test)]
mod tests {
    use super::HEAD_ROWS;
    use crate::export_msg::tests::with_exporter;
    use std::fs;
    use std::io::Read;
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

    /// The XML of the part `name` of the workbook at `path`.
    fn part(path: &std::path::Path, name: &str) -> String {
        let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        let mut xml = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
        xml
    }

    /// The value of the cell `cell` (`A5`) in a sheet's XML.
    fn cell_value<'x>(sheet: &'x str, cell: &str) -> Option<&'x str> {
        let at = sheet.find(&format!("<c r=\"{cell}\""))?;
        let rest = &sheet[at..];
        let rest = &rest[..rest.find("</c>")?];
        let value = &rest[rest.find("<v>")? + 3..];
        Some(&value[..value.find("</v>")?])
    }

    #[test]
    fn messages_beyond_a_sheet_go_on_in_the_next_sheets() {
        // the five messages of the fixture, two to a sheet: three sheets, in either mode
        for big in [false, true] {
            let dir = TestDir::new("weflow-xlsx-sheets");
            let out = dir.join("chat.xlsx");
            let count = with_exporter(false, |ex, msgs| {
                ex.write_excel_rows(msgs, big, &out, HEAD_ROWS + 2).unwrap()
            });
            assert_eq!(count, 5);
            assert_eq!(part(&out, "xl/workbook.xml").matches("<sheet ").count(), 3);
            let first_row = HEAD_ROWS + 1;
            for (sheet, numbers) in [(1, &[1, 2][..]), (2, &[3, 4]), (3, &[5])] {
                let xml = part(&out, &format!("xl/worksheets/sheet{sheet}.xml"));
                let rows = usize::try_from(HEAD_ROWS).unwrap() + numbers.len();
                assert_eq!(
                    xml.matches("<row ").count(),
                    rows,
                    "sheet {sheet}, big {big}"
                );
                assert!(
                    xml.contains(&format!("<c r=\"A{HEAD_ROWS}\"")),
                    "headers on sheet {sheet}"
                );
                for (k, n) in numbers.iter().enumerate() {
                    let cell = format!("A{}", first_row + u32::try_from(k).unwrap());
                    assert_eq!(
                        cell_value(&xml, &cell),
                        Some(n.to_string().as_str()),
                        "sheet {sheet}, big {big}"
                    );
                }
            }
        }
    }
}
