//! Excel 导出。

use std::path::Path;

use anyhow::{Context, Result};

use crate::message::kind::message_type_name_in;
use crate::message::link::extract_html_link_card;
use crate::message::time::format_timestamp;
use crate::message::xml::{strip_sender_prefix, MsgXml};
use crate::message::*;

use super::{chrome, write_bytes, Exporter};

/// 一个 Excel 工作表最多容纳的行数（Excel 的上限）：放不下的消息接着写到下一个工作表。
const SHEET_ROWS: u32 = 1_048_576;
/// 每个工作表中消息上方的行数：先是会话的信息，然后是列的表头。
const HEAD_ROWS: u32 = 4;

impl Exporter<'_, '_> {
    // ──────────────────────────────── Excel 导出 ────────────────────────────────

    /// 行数超过这个数的工作表以常量内存模式写出（没有超链接，行写到磁盘上）。
    pub(super) const EXCEL_BIG: usize = 20000;

    /// 把 `msgs` 写成 Excel 工作簿 `out`，每个工作表写满 1048576 行（Excel 的上限）后换下一个。
    ///
    /// # Errors
    ///
    /// 无法写出工作簿时返回错误。
    pub fn write_excel(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        self.write_excel_rows(msgs, msgs.len() > Self::EXCEL_BIG, out, SHEET_ROWS)?;
        Ok(())
    }

    /// 从消息流（从旧到新）写出 Excel。工作表是否算「大」取决于总数，所以先留下前 `EXCEL_BIG + 1` 条消息：
    /// 消息流在这之前结束时照常写普通工作表；否则把它们写进常量内存的工作表，其余的消息到来时接着写。
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

    /// 写出工作簿；`big` 为真时用常量内存模式（这时各行必须按顺序写，这里正是按顺序写的）。一个工作表容纳
    /// `sheet_rows` 行（除测试外都是 [`SHEET_ROWS`]）：放不下的消息接着写到下一个工作表，上方的行相同，序号接着编。
    /// 返回消息数。
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
            let x = MsgXml::of_export(msg);
            let (content, has_quote) = self.plain_row_content(msg, &x);
            let s = self.sender_fields(msg);
            let type_name = message_type_name_in(msg.local_type, &x);
            ws.write_number(row, 0, crate::num::count_f64(i + 1))?;
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
                ws.write_string(row, content_col, crate::export::fit_cell(&content))?;
            }
        }
        // 先在内存里生成再写出：常量内存的工作表写到文件失败时（例如磁盘已满），rust_xlsxwriter 会 panic（一处
        // unwrap），而写到内存不会失败
        let bytes = workbook
            .save_to_buffer()
            .with_context(|| format!("failed to build {}", out.display()))?;
        write_bytes(out, &bytes)?;
        Ok(count)
    }
}

/// Excel 导出的每个工作表在消息上方的内容：先是会话的信息，然后是列的表头。
struct SheetTop {
    session_id: String,
    nickname: String,
    /// 群的备注（只有群聊有）。
    remark: Option<String>,
    exported_at: String,
    headers: Vec<&'static str>,
    widths: Vec<f64>,
}

impl SheetTop {
    /// 向 `workbook` 添加第 `index` 个工作表（从 0 开始），`big` 为真时用常量内存模式，并写好上方的行。
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

    /// `path` 处工作簿中部件 `name` 的 XML。
    fn part(path: &std::path::Path, name: &str) -> String {
        let mut zip = zip::ZipArchive::new(fs::File::open(path).unwrap()).unwrap();
        let mut xml = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut xml).unwrap();
        xml
    }

    /// 工作表 XML 中单元格 `cell`（如 `A5`）的值。
    fn cell_value<'x>(sheet: &'x str, cell: &str) -> Option<&'x str> {
        let at = sheet.find(&format!("<c r=\"{cell}\""))?;
        let rest = &sheet[at..];
        let rest = &rest[..rest.find("</c>")?];
        let value = &rest[rest.find("<v>")? + 3..];
        Some(&value[..value.find("</v>")?])
    }

    #[test]
    fn messages_beyond_a_sheet_go_on_in_the_next_sheets() {
        // `fixture` 的五条消息，每个工作表两条：不论哪种模式都是三个工作表
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
