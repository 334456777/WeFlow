//! ChatLab 导出：一个 JSON 文档，或者 JSONL 的多行。

#[cfg(test)]
use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use serde::Serialize;
#[cfg(test)]
use serde_json::Value;

use crate::message::forward::RecordKind;
use crate::message::kind::convert_message_type_in;
use crate::message::link::LinkStyle;
use crate::message::transfer::is_same_wxid;
use crate::message::xml::{rx, MsgXml};
use crate::message::*;

use super::format::write_pretty;
use super::names::{resolve_group_nickname, Profile};
use super::spool::{EntryFormat, RenderedEntries, SenderTally, Spool};
use super::Exporter;

impl Exporter<'_, '_> {
    // ─────────────────────────────── ChatLab 导出 ───────────────────────────────

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

    /// 在内存里构建的整个 ChatLab 文档，测试拿流式写出的文件与它比较。
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

    /// 一条消息的 ChatLab 对象。`member_name` 是发送者在成员列表中的名字；转发的聊天记录里出现的人名放进
    /// `record_names`（他们也会成为成员）。
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
        let x = MsgXml::of_export(msg);
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

    /// 群聊时按桌面版的方式确定成员列表中的名字；私聊时原样返回。
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

    /// ChatLab 导出的成员（已确定名字）和各条消息的对象。
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

    /// 把 `msgs` 以 ChatLab 格式写到 `out`：一个 JSON 文档，`jsonl` 为真时写成 JSON Lines。
    ///
    /// # Errors
    ///
    /// 无法写入文件时返回错误。
    pub fn write_chatlab(&mut self, msgs: &[ExportMsg], out: &Path, jsonl: bool) -> Result<()> {
        let format = if jsonl {
            EntryFormat::ChatlabJsonl
        } else {
            EntryFormat::Chatlab
        };
        self.render_and_write(format, out, msgs, true)?;
        Ok(())
    }

    /// ChatLab：各条目先写进暂存文件；成员列表在文件里排在最前面，但要等到所有发送者都出现，所以最后才写。
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
        // 成员：先是按出现顺序排列的发送者，然后是群里的其他人，最后是只出现在转发的聊天记录里的人
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
        let mut w = super::create_output(out)?;
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

// ──────────────────────────────── ChatLab 文档 ────────────────────────────────
//
// 下面的结构体直接序列化进输出，字段的顺序就是文件中键的顺序。

/// JSONL 的一行：先是 `_type`，然后是 `entry` 的各字段。
#[derive(Serialize)]
pub(super) struct Tagged<'a, T> {
    #[serde(rename = "_type")]
    pub(super) kind: &'static str,
    #[serde(flatten)]
    pub(super) entry: &'a T,
}

/// ChatLab 导出的 `chatlab` 文件头。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ChatlabHeader {
    version: &'static str,
    exported_at: i64,
    generator: &'static str,
}

/// ChatLab 导出的 `meta` 对象。
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

/// ChatLab JSONL 导出的第一行。
#[derive(Serialize)]
struct ChatlabJsonlHeader<'a> {
    #[serde(rename = "_type")]
    kind: &'static str,
    chatlab: &'a ChatlabHeader,
    meta: &'a ChatlabMeta<'a>,
}

/// ChatLab 成员列表中的一项。
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

/// ChatLab 导出中的一条消息。
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

/// ChatLab 消息中转发的聊天记录里的一条消息。
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

/// 整个 ChatLab 导出，供 [`Exporter::build_chatlab`] 使用。
#[cfg(test)]
#[derive(Serialize)]
struct ChatlabDocument<'a> {
    chatlab: ChatlabHeader,
    meta: ChatlabMeta<'a>,
    members: Vec<ChatlabMember>,
    messages: Vec<ChatlabMessage<'a>>,
}

/// 转发的聊天记录中一条消息的 ChatLab 记录；时间读不出来时用 `fallback_time`。
fn chatlab_record(rec: &ForwardRecord, fallback_time: i64) -> ChatlabRecord<'_> {
    let timestamp = parse_record_time(&rec.sourcetime).unwrap_or(fallback_time);
    let text = || rec.datadesc.clone().or_else(|| rec.datatitle.clone());
    // 类型代码与普通消息的相同（`convert_message_type_in`）
    let record = RecordKind::of(rec.datatype);
    let tagged = |code: i64| (code, record.tag().to_string());
    let (kind, content) = match record {
        RecordKind::Text => (0, text().unwrap_or_default()),
        RecordKind::Image => tagged(1),
        RecordKind::Voice => tagged(2),
        RecordKind::Video => tagged(3),
        RecordKind::Sticker => tagged(5),
        RecordKind::File => (
            4,
            rec.datatitle
                .as_ref()
                .map(|t| format!("[文件] {t}"))
                .unwrap_or_else(|| record.tag().into()),
        ),
        _ => (0, text().unwrap_or_else(|| record.tag().into())),
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

/// 把 `YYYY-MM-DD HH:MM:SS` 形式的本地时间读成 Unix 时间（秒）；格式不对或本地时区里没有这个时刻时为 `None`。
fn parse_record_time(value: &str) -> Option<i64> {
    use chrono::{Local, TimeZone};
    let c = rx(r"(\d{4})-(\d{2})-(\d{2}) (\d{2}):(\d{2}):(\d{2})").captures(value)?;
    let g = |i: usize| c[i].parse::<u32>().ok();
    Local
        .with_ymd_and_hms(
            i32::try_from(g(1)?).ok()?,
            g(2)?,
            g(3)?,
            g(4)?,
            g(5)?,
            g(6)?,
        )
        .single()
        .map(|d| d.timestamp())
}

#[cfg(test)]
mod tests {
    use crate::export_msg::tests::with_exporter;
    use serde_json::Value;
    use std::fs;
    use weflow_native::fixture::TestDir;

    /// 聊天记录中的一项按它自己的 `datatype` 取 ChatLab 类型，与同类的普通消息相同（#1171）。
    #[test]
    fn a_record_item_has_the_type_of_its_datatype() {
        for (datatype, kind, content) in [
            (1, 0, "收到"),
            (2, 1, "[图片]"),
            (3, 2, "[语音消息]"),
            (4, 3, "[视频]"),
            (37, 5, "[表情]"),
            (8, 4, "[文件] 收到"),
            (34, 0, "收到"),
        ] {
            let rec = crate::message::ForwardRecord {
                datatype,
                sourcename: "Bob".into(),
                datadesc: Some("收到".into()),
                datatitle: Some("收到".into()),
                ..Default::default()
            };
            let r = super::chatlab_record(&rec, 0);
            assert_eq!((r.kind, r.content.as_str()), (kind, content), "{datatype}");
        }
    }

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
            ex.write_chatlab(msgs, &out, true).unwrap();
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
