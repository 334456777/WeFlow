//! The WeClone CSV export.

use std::path::Path;

use anyhow::Result;

use crate::message::kind::weclone_type_name;
use crate::message::quote::is_quoted_reply_message;
use crate::message::time::format_iso_timestamp;
use crate::message::xml::extract_xml_value;
use crate::message::*;

use super::format::csv_cell;
use super::{chrome, write_bytes, Exporter};

impl<'a, 'n> Exporter<'a, 'n> {
    /// WeClone: a header, then one row per message (quote replies left out), numbered as written.
    pub(super) fn stream_weclone(
        &mut self,
        out: &Path,
        msgs: &mut dyn Iterator<Item = ExportMsg>,
    ) -> Result<usize> {
        use std::io::Write;
        let mut count = 0usize;
        let mut rows = 0usize;
        // created with the first message, so an empty range leaves no file behind
        let mut writer: Option<std::io::BufWriter<std::fs::File>> = None;
        for msg in msgs {
            if writer.is_none() {
                if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                    std::fs::create_dir_all(parent)?;
                }
                let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
                w.write_all(Self::WECLONE_HEADER.as_bytes())?;
                writer = Some(w);
            }
            if !is_quoted_reply_message(msg.local_type, &msg.content) {
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

    // ───────────────────────────────── WeClone CSV ─────────────────────────────────

    const WECLONE_HEADER: &'static str =
        "\u{feff}id,MsgSvrID,type_name,is_sender,talker,msg,src,CreateTime\r\n";

    /// One WeClone CSV row (with its line ending); `id` is the row number, starting at 1.
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
                let f = extract_xml_value(&msg.content, "filename");
                if f.is_empty() {
                    extract_xml_value(&msg.content, "title")
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

    pub(super) fn render_weclone(&mut self, msgs: &[ExportMsg]) -> String {
        let mut out = String::from(Self::WECLONE_HEADER);
        let rows = msgs
            .iter()
            .filter(|m| !is_quoted_reply_message(m.local_type, &m.content));
        for (i, msg) in rows.enumerate() {
            out.push_str(&self.weclone_row(msg, i + 1));
        }
        out
    }

    pub fn write_weclone(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let text = self.render_weclone(msgs);
        write_bytes(out, text.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;

    #[test]
    fn weclone_csv() {
        let csv = with_exporter(false, |ex, msgs| ex.render_weclone(msgs));
        assert!(csv
            .starts_with("\u{feff}id,MsgSvrID,type_name,is_sender,talker,msg,src,CreateTime\r\n"));
        let lines: Vec<&str> = csv.trim_end().split("\r\n").collect();
        // quoted reply row is filtered out
        assert_eq!(lines.len(), 1 + 4);
        assert!(lines[1].starts_with("1,111,text,1,Me Nick,\"hello \"\"world\"\", ok\","));
        assert!(lines[3].contains(",image,0,Bobby,"));
    }
}
