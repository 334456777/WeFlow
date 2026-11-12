//! The ChatLab export, as one JSON document or as JSONL lines.

#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;
#[cfg(test)]
use serde_json::Value;

use crate::message::kind::convert_message_type_in;
use crate::message::link::LinkStyle;
use crate::message::transfer::is_same_wxid;
use crate::message::xml::{rx, MsgXml};
use crate::message::*;

use super::format::write_pretty;
use super::names::{resolve_group_nickname, Profile};
use super::spool::{EntryFormat, RenderedEntries, SenderTally, Spool};
use super::Exporter;

impl<'a, 'n> Exporter<'a, 'n> {
    // ───────────────────────────────── ChatLab ─────────────────────────────────

    fn chatlab_header(&self) -> ChatlabHeader {
        ChatlabHeader {
            version: "0.0.2",
            exported_at: self.settings.exported_at,
            generator: "WeFlow",
        }
    }

    fn chatlab_meta(&self) -> ChatlabMeta<'_> {
        let is_group = self.session.is_group;
        ChatlabMeta {
            name: &self.session.display_name,
            platform: "wechat",
            kind: if is_group { "group" } else { "private" },
            group_id: is_group.then_some(self.session.id.as_str()),
        }
    }

    /// The whole ChatLab document, built in memory: what the tests hold the streamed file against.
    #[cfg(test)]
    pub(super) fn build_chatlab(&mut self, msgs: &[ExportMsg]) -> Value {
        let (members, messages) = self.chatlab_parts(msgs);
        let document = ChatlabDocument {
            chatlab: self.chatlab_header(),
            meta: self.chatlab_meta(),
            members,
            messages,
        };
        serde_json::to_value(document).unwrap_or_default()
    }

    /// The ChatLab object of one message. `member_name` is the sender's name in the member list; the names of the
    /// people in a forwarded chat history go to `record_names` (they become members too).
    pub(super) fn chatlab_message<'m>(
        &mut self,
        msg: &'m ExportMsg,
        member_name: &str,
        record_names: &mut Vec<String>,
    ) -> ChatlabMessage<'m> {
        let is_group = self.session.is_group;
        let group_nickname = if is_group {
            resolve_group_nickname(&self.group_nicks, &[msg.sender_username.as_str()])
        } else {
            String::new()
        };
        let profile = if is_group {
            self.sender_profile_for(msg)
        } else {
            Profile {
                wxid: msg.sender_username.clone(),
                nickname: member_name.to_string(),
                display_name: member_name.to_string(),
                group_nickname: group_nickname.clone(),
                ..Default::default()
            }
        };
        let x = MsgXml::new(&msg.content);
        let content = match msg.media.as_ref().filter(|_| msg.local_type == 3) {
            Some(media) => media.relative_path.clone(),
            None => self.message_text_with(msg, &x, Some(LinkStyle::Markdown), false),
        };
        let account_name = if profile.display_name.is_empty() {
            member_name.to_string()
        } else {
            profile.display_name
        };
        let group_nickname = if !profile.group_nickname.is_empty() {
            profile.group_nickname
        } else {
            group_nickname
        };
        let chat_records = msg
            .chat_record_list
            .as_ref()
            .filter(|l| !l.is_empty())
            .map(|records| {
                records
                    .iter()
                    .map(|rec| {
                        if !rec.sourcename.is_empty() {
                            record_names.push(rec.sourcename.clone());
                        }
                        chatlab_record(rec, msg.create_time)
                    })
                    .collect()
            });
        ChatlabMessage {
            sender: &msg.sender_username,
            account_name,
            group_nickname: (!group_nickname.is_empty()).then_some(group_nickname),
            timestamp: msg.create_time,
            kind: convert_message_type_in(msg.local_type, &x),
            content,
            platform_message_id: msg.platform_message_id(),
            reply_to_message_id: x.reply_to_message_id(),
            chat_records,
        }
    }

    /// The member list with the names resolved like WeFlow's desktop app does for groups.
    fn chatlab_finish_members(&mut self, mut members: Vec<ChatlabMember>) -> Vec<ChatlabMember> {
        if !self.session.is_group {
            return members;
        }
        let raw = self.raw_my_wxid.clone();
        let my = self.my_wxid.clone();
        for m in &mut members {
            let extra: Vec<&str> = if is_same_wxid(&m.platform_id, &my) {
                vec![raw.as_str(), my.as_str()]
            } else {
                Vec::new()
            };
            let p = self.resolve_profile(&m.platform_id, &m.account_name, &extra);
            if !p.display_name.is_empty() {
                m.account_name = p.display_name;
            }
            if !p.group_nickname.is_empty() {
                m.group_nickname = Some(p.group_nickname);
            }
        }
        members
    }

    /// The (resolved) members and the message objects of a ChatLab export.
    #[cfg(test)]
    fn chatlab_parts<'m>(
        &mut self,
        msgs: &'m [ExportMsg],
    ) -> (Vec<ChatlabMember>, Vec<ChatlabMessage<'m>>) {
        let mut members: Vec<ChatlabMember> = Vec::new();
        let mut member_index: HashMap<String, usize> = HashMap::new();
        for id in self.sessions_members(msgs) {
            let account_name = self.contact_display(&id);
            member_index.insert(id.clone(), members.len());
            members.push(ChatlabMember::new(id, account_name));
        }
        let mut messages = Vec::with_capacity(msgs.len());
        for msg in msgs {
            let member_name = member_index
                .get(&msg.sender_username)
                .map(|&i| members[i].account_name.clone())
                .unwrap_or_else(|| msg.sender_username.clone());
            let mut record_names: Vec<String> = Vec::new();
            messages.push(self.chatlab_message(msg, &member_name, &mut record_names));
            for name in record_names {
                if let std::collections::hash_map::Entry::Vacant(slot) = member_index.entry(name) {
                    let name = slot.key().clone();
                    slot.insert(members.len());
                    members.push(ChatlabMember::new(name.clone(), name));
                }
            }
        }
        (self.chatlab_finish_members(members), messages)
    }

    pub fn write_chatlab(&mut self, msgs: &[ExportMsg], out: &Path, jsonl: bool) -> Result<()> {
        let format = if jsonl {
            EntryFormat::ChatlabJsonl
        } else {
            EntryFormat::Chatlab
        };
        self.render_and_write(format, out, msgs, true)?;
        Ok(())
    }

    /// ChatLab: the entries go to a spool file; the member list, which comes first in the file and needs every
    /// sender, is written at the end.
    pub(super) fn write_rendered_chatlab(
        &mut self,
        out: &Path,
        jsonl: bool,
        next: &mut dyn FnMut(&mut Self) -> Option<RenderedEntries>,
        write_if_empty: bool,
    ) -> Result<usize> {
        use std::io::Write;
        let mut spool = Spool::create(out)?;
        let mut tally = SenderTally::default();
        let mut record_order: Vec<String> = Vec::new();
        let mut record_seen: HashSet<String> = HashSet::new();
        let mut count = 0usize;
        let separator = if jsonl { "\n" } else { ",\n    " };
        while let Some(mut page) = next(self) {
            for sender in &page.senders {
                tally.add(sender);
            }
            for name in std::mem::take(&mut page.record_names) {
                if record_seen.insert(name.clone()) {
                    record_order.push(name);
                }
            }
            for text in page.entries() {
                spool.push(separator, text)?;
                count += 1;
            }
        }
        if count == 0 && !write_if_empty {
            return Ok(0);
        }
        // members: the senders in order of appearance, then the rest of the group, then the people who only
        // appear inside forwarded chat histories
        let mut ids = tally.order;
        let mut known: HashSet<String> = ids.iter().cloned().collect();
        if self.session.is_group {
            for g in self.group_members.clone() {
                if !g.is_empty() && known.insert(g.clone()) {
                    ids.push(g);
                }
            }
        }
        let mut members: Vec<ChatlabMember> = Vec::with_capacity(ids.len());
        for id in ids {
            let account_name = self.contact_display(&id);
            members.push(ChatlabMember::new(id, account_name));
        }
        for name in record_order {
            if known.insert(name.clone()) {
                members.push(ChatlabMember::new(name.clone(), name));
            }
        }
        let members = self.chatlab_finish_members(members);
        let chatlab = self.chatlab_header();
        let meta = self.chatlab_meta();
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
        if jsonl {
            serde_json::to_writer(
                &mut w,
                &ChatlabJsonlHeader {
                    kind: "header",
                    chatlab: &chatlab,
                    meta: &meta,
                },
            )?;
            for member in &members {
                w.write_all(b"\n")?;
                serde_json::to_writer(
                    &mut w,
                    &Tagged {
                        kind: "member",
                        entry: member,
                    },
                )?;
            }
            if count > 0 {
                w.write_all(b"\n")?;
                spool.copy_to(&mut w)?;
            }
        } else {
            w.write_all(b"{\n  \"chatlab\": ")?;
            write_pretty(&mut w, &chatlab, 2)?;
            w.write_all(b",\n  \"meta\": ")?;
            write_pretty(&mut w, &meta, 2)?;
            w.write_all(b",\n  \"members\": ")?;
            write_pretty(&mut w, &members, 2)?;
            w.write_all(b",\n  \"messages\": ")?;
            if count == 0 {
                w.write_all(b"[]")?;
            } else {
                w.write_all(b"[\n    ")?;
                spool.copy_to(&mut w)?;
                w.write_all(b"\n  ]")?;
            }
            w.write_all(b"\n}")?;
        }
        w.flush()?;
        Ok(count)
    }
}

// ──────────────────────── JSON / ChatLab / HTML documents ────────────────────────
//
// The structs below are serialized straight into the output; their field order is the key order of the files.

/// A JSONL line: `_type` first, then the fields of `entry`.
#[derive(Serialize)]
pub(super) struct Tagged<'a, T> {
    #[serde(rename = "_type")]
    pub(super) kind: &'static str,
    #[serde(flatten)]
    pub(super) entry: &'a T,
}

/// The `chatlab` header of a ChatLab export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatlabHeader {
    version: &'static str,
    exported_at: i64,
    generator: &'static str,
}

/// The `meta` object of a ChatLab export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatlabMeta<'a> {
    name: &'a str,
    platform: &'static str,
    #[serde(rename = "type")]
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_id: Option<&'a str>,
}

/// The first line of a ChatLab JSONL export.
#[derive(Serialize)]
struct ChatlabJsonlHeader<'a> {
    #[serde(rename = "_type")]
    kind: &'static str,
    chatlab: &'a ChatlabHeader,
    meta: &'a ChatlabMeta<'a>,
}

/// One entry of the ChatLab member list.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatlabMember {
    platform_id: String,
    account_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_nickname: Option<String>,
}

impl ChatlabMember {
    fn new(platform_id: String, account_name: String) -> Self {
        Self {
            platform_id,
            account_name,
            group_nickname: None,
        }
    }
}

/// One message of a ChatLab export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ChatlabMessage<'m> {
    sender: &'m str,
    account_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_nickname: Option<String>,
    timestamp: i64,
    #[serde(rename = "type")]
    kind: i64,
    content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    chat_records: Option<Vec<ChatlabRecord<'m>>>,
}

/// One message of a forwarded chat history inside a ChatLab message.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatlabRecord<'m> {
    sender: &'m str,
    account_name: &'m str,
    timestamp: i64,
    #[serde(rename = "type")]
    kind: i64,
    content: String,
}

/// A whole ChatLab export, for [`Exporter::build_chatlab`].
#[cfg(test)]
#[derive(Serialize)]
struct ChatlabDocument<'a> {
    chatlab: ChatlabHeader,
    meta: ChatlabMeta<'a>,
    members: Vec<ChatlabMember>,
    messages: Vec<ChatlabMessage<'a>>,
}

/// The ChatLab record of one message in a forwarded chat history; `fallback_time` stands in for an unreadable time.
fn chatlab_record(rec: &ForwardRecord, fallback_time: i64) -> ChatlabRecord<'_> {
    let timestamp = parse_record_time(&rec.sourcetime).unwrap_or(fallback_time);
    let text = || rec.datadesc.clone().or_else(|| rec.datatitle.clone());
    let (kind, content) = match rec.datatype {
        1 => (0, text().unwrap_or_default()),
        3 => (1, "[图片]".to_string()),
        8 | 49 => (
            4,
            rec.datatitle
                .as_ref()
                .map(|t| format!("[文件] {t}"))
                .unwrap_or_else(|| "[文件]".into()),
        ),
        34 => (2, "[语音消息]".to_string()),
        43 => (3, "[视频]".to_string()),
        47 => (5, "[表情]".to_string()),
        _ => (0, text().unwrap_or_else(|| "[消息]".into())),
    };
    let who = if rec.sourcename.is_empty() {
        "unknown"
    } else {
        rec.sourcename.as_str()
    };
    ChatlabRecord {
        sender: who,
        account_name: who,
        timestamp,
        kind,
        content,
    }
}

fn parse_record_time(value: &str) -> Option<i64> {
    use chrono::{Local, TimeZone};
    let c = rx(r"(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})").captures(value)?;
    let g = |i: usize| c[i].parse::<u32>().ok();
    Local
        .with_ymd_and_hms(g(1)? as i32, g(2)?, g(3)?, g(4)?, g(5)?, g(6)?)
        .single()
        .map(|d| d.timestamp())
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;
    use serde_json::Value;
    use std::fs;
    use weflow_native::fixture::TestDir;

    #[test]
    fn chatlab_shape() {
        let v = with_exporter(false, |ex, msgs| ex.build_chatlab(msgs));
        assert_eq!(v["chatlab"]["version"], "0.0.2");
        assert_eq!(v["meta"]["platform"], "wechat");
        assert_eq!(v["meta"]["type"], "private");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 5);
        assert_eq!(msgs[0]["sender"], "wxid_me");
        assert_eq!(msgs[0]["type"], 0);
        assert_eq!(msgs[0]["platformMessageId"], "111");
        assert_eq!(msgs[2]["type"], 1);
        assert_eq!(msgs[3]["content"], "[Doc](https://example.com/x)");
        assert_eq!(msgs[4]["type"], 25);
        assert_eq!(msgs[4]["replyToMessageId"], "111");
        let members = v["members"].as_array().unwrap();
        assert!(members
            .iter()
            .any(|m| m["platformId"] == "wxid_bob" && m["accountName"] == "Bobby"));
    }

    #[test]
    fn chatlab_jsonl_lines() {
        let dir = TestDir::new("weflow-jsonl");
        let out = dir.join("a.jsonl");
        with_exporter(false, |ex, msgs| {
            ex.write_chatlab(msgs, &out, true).unwrap()
        });
        let text = fs::read_to_string(&out).unwrap();
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines[0]["_type"], "header");
        assert!(lines.iter().filter(|l| l["_type"] == "member").count() >= 2);
        assert_eq!(lines.iter().filter(|l| l["_type"] == "message").count(), 5);
    }
}
