//! WeClone CSV 导出。

use std::path::Path;

use anyhow::Result;

use crate::message::kind::weclone_type_name;
use crate::message::quote::is_text_quote_message_in;
use crate::message::time::format_iso_timestamp;
use crate::message::xml::{xml_text, MsgXml};
use crate::message::*;

use super::csv::{create_csv, write_csv};
use super::format::csv_cell;
use super::{chrome, Exporter};

impl Exporter<'_, '_> {
    /// 从消息流写出 WeClone CSV：先是表头，然后每条消息一行（引用回复不写），按写出的顺序编号。
    /// 返回读到的消息数（含没有写出的引用回复）；没有消息时不写文件。
    pub(super) fn stream_weclone(
        &mut self,
        out: &Path,
        msgs: &mut dyn Iterator<Item = ExportMsg>,
    ) -> Result<usize> {
        use std::io::Write;
        let mut count = 0usize;
        let mut rows = 0usize;
        // 读到第一条消息时才创建文件，时间范围内没有消息时不留下文件
        let mut writer: Option<std::io::BufWriter<std::fs::File>> = None;
        for msg in msgs {
            if writer.is_none() {
                let mut w = create_csv(out)?;
                w.write_all(Self::WECLONE_HEADER.as_bytes())?;
                writer = Some(w);
            }
            if !is_text_quote(&msg) {
                rows += 1;
                let row = self.weclone_row(&msg, rows);
                if let Some(w) = writer.as_mut() {
                    w.write_all(row.as_bytes())?;
                }
            }
            count += 1;
        }
        if let Some(mut w) = writer {
            w.flush()?;
        }
        Ok(count)
    }

    // ─────────────────────────────── WeClone CSV 导出 ───────────────────────────────

    /// 表头这一行；文件的 BOM 在创建文件时写在它前面（见 [`create_csv`]）。
    const WECLONE_HEADER: &'static str =
        "id,MsgSvrID,type_name,is_sender,talker,msg,src,CreateTime\r\n";

    /// WeClone CSV 的一行（含换行符）；`id` 是行号，从 1 开始。
    fn weclone_row(&mut self, msg: &ExportMsg, id: usize) -> String {
        let my = self.my_wxid.clone();
        let type_name = weclone_type_name(msg.local_type, &msg.content);
        let sender_wxid = if msg.is_send {
            my.clone()
        } else if self.session.is_group && !msg.sender_username.is_empty() {
            msg.sender_username.clone()
        } else {
            self.session.id.clone()
        };
        let mut talker = if self.my_display.is_empty() {
            chrome("我", "Me").to_string()
        } else {
            self.my_display.clone()
        };
        if self.session.is_group {
            let raw = self.raw_my_wxid.clone();
            let my_display = if self.my_display.is_empty() {
                my.clone()
            } else {
                self.my_display.clone()
            };
            let p = if msg.is_send {
                self.resolve_profile(&my, &my_display, &[raw.as_str(), my.as_str()])
            } else {
                self.resolve_profile(&sender_wxid, &sender_wxid, &[])
            };
            talker = p.display_name;
        } else if !msg.is_send {
            let contact = self.names.get(&sender_wxid);
            let nickname = contact
                .as_ref()
                .map(|c| c.nickname.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| sender_wxid.clone());
            let remark = contact
                .as_ref()
                .map(|c| c.remark.clone())
                .unwrap_or_default();
            talker = self
                .settings
                .display_pref
                .pick(&sender_wxid, &nickname, &remark, "");
        }
        let text = self.message_text(msg);
        let src = match type_name {
            "image" => msg.image_dat_name.clone().unwrap_or_default(),
            "sticker" => msg.emoji_cdn_url.clone().unwrap_or_default(),
            "file" => {
                let f = xml_text(&msg.content, "filename");
                if f.is_empty() {
                    xml_text(&msg.content, "title")
                } else {
                    f
                }
            }
            _ => String::new(),
        };
        let src = msg
            .media
            .as_ref()
            .map(|m| m.relative_path.clone())
            .unwrap_or(src);
        let cells = [
            id.to_string(),
            msg.platform_message_id().unwrap_or_default(),
            type_name.to_string(),
            if msg.is_send { "1".into() } else { "0".into() },
            talker,
            text,
            src,
            format_iso_timestamp(msg.create_time),
        ];
        let line: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
        format!("{}\r\n", line.join(","))
    }

    /// `msgs` 的 CSV 文字：先是表头，然后是各行，不含文件的 BOM。
    pub(super) fn render_weclone(&mut self, msgs: &[ExportMsg]) -> String {
        let mut out = String::from(Self::WECLONE_HEADER);
        let rows = msgs.iter().filter(|m| !is_text_quote(m));
        for (i, msg) in rows.enumerate() {
            out.push_str(&self.weclone_row(msg, i + 1));
        }
        out
    }

    /// 把 `msgs` 写成 WeClone 的 CSV 文件 `out`，不写引用回复。
    ///
    /// # Errors
    ///
    /// 无法写入文件时返回错误。
    pub fn write_weclone(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        write_csv(out, &self.render_weclone(msgs))
    }
}

/// 是否为引用回复，WeClone 不写这种消息。XML 里带 `<refermsg>` 的图片或视频不算。
fn is_text_quote(msg: &ExportMsg) -> bool {
    is_text_quote_message_in(msg.local_type, &MsgXml::of_export(msg))
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;
    use crate::message::{collect_rows, CollectOptions};
    use weflow_native::fixture::TestDir;
    use weflow_native::msg_row::MsgRow;

    #[test]
    fn weclone_csv() {
        let csv = with_exporter(false, |ex, msgs| ex.render_weclone(msgs));
        assert!(csv.starts_with("id,MsgSvrID,type_name,is_sender,talker,msg,src,CreateTime\r\n"));
        let lines: Vec<&str> = csv.trim_end().split("\r\n").collect();
        // 引用回复的一行被去掉
        assert_eq!(lines.len(), 1 + 4);
        assert!(lines[1].starts_with("1,111,text,1,Me Nick,\"hello \"\"world\"\", ok\","));
        assert!(lines[3].contains(",image,0,Bobby,"));
    }

    /// 不论一次写出还是随消息到来逐行写出，文件都是 BOM 加上渲染出的文字（#1075）。
    #[test]
    fn a_weclone_file_starts_with_the_byte_order_mark() {
        let dir = TestDir::new("weclone-bom");
        let (whole, streamed) = (dir.join("whole.csv"), dir.join("streamed.csv"));
        let text = with_exporter(false, |ex, msgs| {
            ex.write_weclone(msgs, &whole).unwrap();
            ex.stream_weclone(&streamed, &mut msgs.iter().cloned())
                .unwrap();
            ex.render_weclone(msgs)
        });
        for out in [whole, streamed] {
            let bytes = std::fs::read(&out).unwrap();
            assert_eq!(bytes[..3], [0xEF, 0xBB, 0xBF], "{}", out.display());
            assert_eq!(bytes[3..], *text.as_bytes(), "{}", out.display());
        }
    }

    #[test]
    fn images_and_videos_that_carry_a_refermsg_are_rows() {
        let row = |local_id: i64, local_type: i64, content: &str| MsgRow {
            local_id,
            create_time: Some(1_700_000_000 + local_id),
            local_type,
            content: content.into(),
            sender: Some("wxid_bob".into()),
            ..MsgRow::default()
        };
        let refer = "<extcommoninfo><refermsg><svrid>9</svrid></refermsg></extcommoninfo>";
        let rows = vec![
            row(1, 3, &format!("<msg><img md5=\"x\"/>{refer}</msg>")),
            row(2, 43, &format!("<msg><videomsg md5=\"y\"/>{refer}</msg>")),
            row(3, 49, "<msg><appmsg><title>yes</title><type>57</type><refermsg><type>1</type><content>hi</content><svrid>9</svrid></refermsg></appmsg></msg>"),
        ];
        let msgs = collect_rows(
            rows,
            &CollectOptions {
                session_id: "wxid_bob",
                my_wxid: "wxid_me",
                start: None,
                end: None,
                sender_filter: None,
            },
        );
        let csv = with_exporter(false, |ex, _| ex.render_weclone(&msgs));
        let lines: Vec<&str> = csv.trim_end().split("\r\n").collect();
        // 图片和视频留下，引用回复去掉
        assert_eq!(lines.len(), 1 + 2, "{csv}");
        assert!(
            lines[1].starts_with("1,") && lines[1].contains(",image,"),
            "{csv}"
        );
        assert!(
            lines[2].starts_with("2,") && lines[2].contains(",video,"),
            "{csv}"
        );
    }
}
