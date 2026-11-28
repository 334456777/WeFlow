//! Arkme JSON 导出：带发送者表、消息写得更紧凑的详细 JSON。

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::Result;
use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::{Map, Value};

use crate::message::*;

use super::json::JsonMessage;
use super::names::{resolve_group_nickname, Profile};
use super::spool::{SenderTally, Spool};
use super::Exporter;

impl Exporter<'_, '_> {
    /// 对 `msgs` 的发送者调用 [`ark_group_members_of`](Self::ark_group_members_of)。
    #[cfg(test)]
    pub(super) fn ark_group_members(&mut self, msgs: &[ExportMsg]) -> Vec<ArkGroupMember> {
        let mut tally = SenderTally::default();
        for m in msgs {
            tally.add(&m.sender_username);
        }
        self.ark_group_members_of(tally)
    }

    /// 由发送者的统计（见 [`SenderTally`]）得出 arkme 导出的 `groupMembers`：每个发送者加上群成员列表，
    /// 发言多的在前，消息数相同的按名字排序。
    pub(super) fn ark_group_members_of(&mut self, tally: SenderTally) -> Vec<ArkGroupMember> {
        let SenderTally { order, counts, .. } = tally;
        let ids: Vec<String> = {
            let mut v = order;
            let mut seen: HashSet<String> = v.iter().cloned().collect();
            for g in &self.group_members {
                if !g.is_empty() && seen.insert(g.clone()) {
                    v.push(g.clone());
                }
            }
            v
        };
        let mut members: Vec<ArkGroupMember> = Vec::with_capacity(ids.len());
        for id in ids {
            let contact = self.names.get(&id);
            let nickname = contact
                .as_ref()
                .map(|c| c.nickname.clone())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| id.clone());
            let remark = contact
                .as_ref()
                .map(|c| c.remark.clone())
                .unwrap_or_default();
            let alias = contact
                .as_ref()
                .map(|c| c.alias.clone())
                .unwrap_or_default();
            let group_nick =
                resolve_group_nickname(&self.group_nicks, &[id.as_str(), alias.as_str()]);
            let display = self
                .settings
                .display_pref
                .pick(&id, &nickname, &remark, &group_nick);
            let message_count = counts.get(&id.to_lowercase()).copied().unwrap_or(0);
            members.push(ArkGroupMember {
                wxid: id,
                display_name: display,
                nickname,
                remark,
                alias,
                group_nickname: (!group_nick.is_empty()).then_some(group_nick),
                is_friend: false,
                message_count,
            });
        }
        members.sort_by(|a, b| {
            b.message_count
                .cmp(&a.message_count)
                .then_with(|| crate::collate::compare_zh(a.sort_name(), b.sort_name()))
        });
        members
    }

    /// 从消息流（从旧到新）写出 arkme-json。每条消息读到后立即序列化，放进暂存文件；
    /// 文件头（含总数）、发送者表和成员列表（含发送者）最后再写在它的前后。
    /// 没有消息时不写文件，除非 `write_if_empty` 为真。
    pub(super) fn stream_arkme_json<I>(
        &mut self,
        msgs: I,
        out: &Path,
        write_if_empty: bool,
    ) -> Result<usize>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<ExportMsg>,
    {
        use std::borrow::Borrow;
        let is_group = self.session.is_group;
        let mut spool = Spool::create(out)?;
        let mut ark = ArkCompact::default();
        let mut profiles: HashMap<String, Profile> = HashMap::new();
        let mut tally = SenderTally::default();
        let mut last_ts: Option<i64> = None;
        let mut count = 0usize;
        for item in msgs {
            let msg: &ExportMsg = item.borrow();
            let message = self.json_message(msg, true, count, &mut profiles);
            let sender = msg.sender_username.trim();
            let compact = ark.compact(message, &|s| {
                (s == sender)
                    .then(|| profiles.get(&msg.sender_username).cloned())
                    .flatten()
            });
            spool.push_json(",\n    ", &compact, 4)?;
            last_ts = last_ts.max(Some(msg.create_time));
            if is_group {
                tally.add(&msg.sender_username);
            }
            count += 1;
        }
        if count == 0 && !write_if_empty {
            return Ok(0);
        }
        self.write_json_file(out, &mut spool, count, last_ts, Some((&ark.senders, tally)))?;
        Ok(count)
    }
}

/// arkme-json 导出中的一条消息：发送者换成它在发送者表中的编号、只保留 arkme 需要的键的 [`JsonMessage`]。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArkMessage<'m> {
    local_id: usize,
    create_time: i64,
    formatted_time: String,
    #[serde(rename = "type")]
    kind: &'static str,
    local_type: i64,
    content: String,
    is_send: u8,
    #[serde(rename = "senderID")]
    sender_id: usize,
    source: &'m str,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_poiname: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_label: Option<&'m str>,
    #[serde(flatten)]
    tail: ArkTail<'m>,
}

/// arkme-json 消息中位于位置字段之后的键，按 [`ArkTail::KEYS`] 的顺序排列：表情的字段和 [`JsonMessage::extra`]
/// 中保留的键。
struct ArkTail<'m> {
    emoji_md5: Option<&'m str>,
    emoji_cdn_url: Option<&'m str>,
    emoji_caption: Option<&'m str>,
    extra: Map<String, Value>,
}

impl ArkTail<'_> {
    const KEYS: &'static [&'static str] = &[
        "appMsgType",
        "appMsgKind",
        "appMsgDesc",
        "appMsgAppName",
        "appMsgSourceName",
        "appMsgSourceUsername",
        "appMsgThumbUrl",
        "quotedContent",
        "quotedSender",
        "quotedType",
        "linkTitle",
        "linkUrl",
        "linkThumb",
        "emojiMd5",
        "emojiCdnUrl",
        "emojiCaption",
        "finderTitle",
        "finderDesc",
        "finderUsername",
        "finderNickname",
        "finderCoverUrl",
        "finderAvatar",
        "finderDuration",
        "finderObjectId",
        "finderUrl",
        "musicTitle",
        "musicUrl",
        "musicDataUrl",
        "musicAlbumUrl",
        "musicCoverUrl",
        "musicSinger",
        "musicAppName",
        "musicSourceName",
        "musicDuration",
        "cardKind",
        "contactCardWxid",
        "contactCardNickname",
        "contactCardAlias",
        "contactCardRemark",
        "contactCardGender",
        "contactCardProvince",
        "contactCardCity",
        "contactCardSignature",
        "contactCardAvatar",
    ];
}

impl Serialize for ArkTail<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        for &key in Self::KEYS {
            let sticker = match key {
                "emojiMd5" => Some(self.emoji_md5),
                "emojiCdnUrl" => Some(self.emoji_cdn_url),
                "emojiCaption" => Some(self.emoji_caption),
                _ => None,
            };
            match sticker {
                Some(Some(value)) => map.serialize_entry(key, value)?,
                Some(None) => {}
                None => {
                    if let Some(value) = self.extra.get(key) {
                        map.serialize_entry(key, value)?;
                    }
                }
            }
        }
        map.end()
    }
}

/// arkme-json 发送者表中的一项。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArkSender {
    #[serde(rename = "senderID")]
    sender_id: usize,
    wxid: String,
    display_name: String,
    nickname: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    remark: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_nickname: Option<String>,
}

/// 群聊的 arkme-json 导出中 `groupMembers` 的一项。
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct ArkGroupMember {
    wxid: String,
    display_name: String,
    nickname: String,
    remark: String,
    alias: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_nickname: Option<String>,
    is_friend: bool,
    message_count: i64,
}

impl ArkGroupMember {
    /// 消息数相同的成员按这个名字排序：显示的名字，为空时用 wxid。
    fn sort_name(&self) -> &str {
        if self.display_name.is_empty() {
            &self.wxid
        } else {
            &self.display_name
        }
    }
}

/// 把详细 JSON 的消息转成 arkme-json 的紧凑形式，发送者第一次出现时给它编号。
#[derive(Default)]
pub(super) struct ArkCompact {
    sender_ids: HashMap<String, usize>,
    pub(super) senders: Vec<ArkSender>,
}

impl ArkCompact {
    /// 把一条消息转成紧凑形式；发送者第一次出现时，由 `profile_of(sender)` 提供它的名字资料。
    pub(super) fn compact<'m>(
        &mut self,
        message: JsonMessage<'m>,
        profile_of: &dyn Fn(&str) -> Option<Profile>,
    ) -> ArkMessage<'m> {
        let sender = match message.sender_username.trim() {
            "" => "unknown",
            s => s,
        };
        let sender_id = match self.sender_ids.get(sender) {
            Some(&id) => id,
            None => {
                let id = self.senders.len() + 1;
                self.sender_ids.insert(sender.to_string(), id);
                let p = profile_of(sender).unwrap_or_default();
                let display_name = if p.display_name.is_empty() {
                    sender.to_string()
                } else {
                    p.display_name
                };
                // 没有昵称时用显示的名字代替
                let nickname = if p.nickname.is_empty() {
                    display_name.clone()
                } else {
                    p.nickname
                };
                self.senders.push(ArkSender {
                    sender_id: id,
                    wxid: sender.to_string(),
                    display_name,
                    nickname,
                    remark: (!p.remark.is_empty()).then_some(p.remark),
                    group_nickname: (!p.group_nickname.is_empty()).then_some(p.group_nickname),
                });
                id
            }
        };
        let JsonMessage {
            local_id,
            create_time,
            formatted_time,
            kind,
            local_type,
            content,
            is_send,
            source,
            emoji_md5,
            emoji_cdn_url,
            emoji_caption,
            platform_message_id,
            reply_to_message_id,
            extra,
            location_lat,
            location_lng,
            location_poiname,
            location_label,
            ..
        } = message;
        ArkMessage {
            local_id,
            create_time,
            formatted_time,
            kind,
            local_type,
            content,
            is_send,
            sender_id,
            source,
            platform_message_id,
            reply_to_message_id,
            location_lat,
            location_lng,
            location_poiname,
            location_label,
            tail: ArkTail {
                emoji_md5,
                emoji_cdn_url,
                emoji_caption,
                extra,
            },
        }
    }
}
