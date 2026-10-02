//! Chat-message exporters ported from the desktop app: ChatLab (+JSONL), detailed JSON,
//! Arkme JSON, Excel, TXT, WeClone CSV and HTML.
//!
//! Everything here works on already-collected [`ExportMsg`] values plus a name book for
//! contacts, so it is testable without a WeChat database. Data values (message text,
//! type names, JSON keys) follow the desktop app; only human-facing chrome such as
//! spreadsheet headers and the HTML page follows the CLI language.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};

use crate::locale;
use crate::message::*;

const HTML_STYLES: &str = include_str!("../../../electron/services/exportHtml.css");

fn chrome(zh: &'static str, en: &'static str) -> &'static str {
    locale::tr(en, zh)
}

// ─────────────────────────────── contacts / names ───────────────────────────────

#[derive(Clone, Debug, Default)]
pub struct ContactInfo {
    pub username: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
}

impl ContactInfo {
    pub fn from_value(username: &str, v: &Value) -> Self {
        let s = |keys: &[&str]| {
            keys.iter()
                .filter_map(|k| v.get(*k).and_then(Value::as_str))
                .find(|x| !x.is_empty())
                .unwrap_or("")
                .to_string()
        };
        ContactInfo {
            username: {
                let u = s(&["username", "userName"]);
                if u.is_empty() {
                    username.to_string()
                } else {
                    u
                }
            },
            nickname: s(&["nickName", "nick_name", "nickname"]),
            remark: s(&["remark"]),
            alias: s(&["alias"]),
        }
    }

    /// What WCDB's display-name lookup yields: remark, else nickname.
    pub fn display_name(&self) -> String {
        if !self.remark.is_empty() {
            self.remark.clone()
        } else if !self.nickname.is_empty() {
            self.nickname.clone()
        } else if !self.alias.is_empty() {
            self.alias.clone()
        } else {
            self.username.clone()
        }
    }
}

type Loader<'a> = Box<dyn FnMut(&str) -> Option<ContactInfo> + 'a>;

pub struct NameBook<'a> {
    cache: HashMap<String, Option<ContactInfo>>,
    loader: Loader<'a>,
}

impl<'a> NameBook<'a> {
    pub fn new(loader: impl FnMut(&str) -> Option<ContactInfo> + 'a) -> Self {
        NameBook {
            cache: HashMap::new(),
            loader: Box::new(loader),
        }
    }

    pub fn from_map(map: HashMap<String, ContactInfo>) -> NameBook<'static> {
        NameBook::new(move |u| map.get(u).cloned())
    }

    pub fn get(&mut self, username: &str) -> Option<ContactInfo> {
        if let Some(found) = self.cache.get(username) {
            return found.clone();
        }
        let loaded = (self.loader)(username);
        self.cache.insert(username.to_string(), loaded.clone());
        loaded
    }

    pub fn display_name(&mut self, username: &str) -> String {
        match self.get(username) {
            Some(c) => c.display_name(),
            None => username.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayPref {
    GroupNickname,
    Remark,
    Nickname,
}

impl DisplayPref {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim() {
            "group-nickname" => Some(Self::GroupNickname),
            "remark" => Some(Self::Remark),
            "nickname" => Some(Self::Nickname),
            _ => None,
        }
    }

    pub fn pick(self, wxid: &str, nickname: &str, remark: &str, group_nickname: &str) -> String {
        let first = |vals: &[&str]| {
            vals.iter()
                .find(|v| !v.is_empty())
                .map(|v| v.to_string())
                .unwrap_or_default()
        };
        match self {
            Self::GroupNickname => first(&[group_nickname, remark, nickname, wxid]),
            Self::Remark => first(&[remark, nickname, wxid]),
            Self::Nickname => first(&[nickname, wxid]),
        }
    }
}

pub fn normalize_group_nickname(value: &str) -> String {
    let cleaned: String = value
        .trim()
        .chars()
        .filter(|c| !(*c <= '\u{1f}' || *c == '\u{7f}'))
        .collect();
    if cleaned.is_empty() {
        return String::new();
    }
    if cleaned.chars().all(|c| ",\"'“”‘’，、".contains(c)) {
        return String::new();
    }
    cleaned
}

/// Keep only identities whose nickname is unambiguous (`buildTrustedGroupNicknameMap`).
pub fn build_trusted_group_nicknames(
    entries: impl IntoIterator<Item = (String, String)>,
) -> HashMap<String, String> {
    let mut buckets: HashMap<String, HashSet<String>> = HashMap::new();
    for (id, nick) in entries {
        let identity = id.trim().to_lowercase();
        if identity.is_empty() {
            continue;
        }
        let nickname = normalize_group_nickname(&nick);
        if nickname.is_empty() {
            continue;
        }
        buckets.entry(identity).or_default().insert(nickname);
    }
    buckets
        .into_iter()
        .filter_map(|(k, v)| {
            if v.len() == 1 {
                v.into_iter().next().map(|n| (k, n))
            } else {
                None
            }
        })
        .collect()
}

pub fn resolve_group_nickname(map: &HashMap<String, String>, candidates: &[&str]) -> String {
    let mut resolved = String::new();
    let mut seen: Vec<&str> = Vec::new();
    for c in candidates {
        let t = c.trim();
        if t.is_empty() || seen.contains(&t) {
            continue;
        }
        seen.push(t);
        let id = t.to_lowercase();
        let nick = normalize_group_nickname(map.get(&id).map(String::as_str).unwrap_or(""));
        if nick.is_empty() {
            continue;
        }
        if resolved.is_empty() {
            resolved = nick;
        } else if resolved != nick {
            return String::new();
        }
    }
    resolved
}

#[derive(Clone, Debug, Default)]
pub struct Profile {
    pub wxid: String,
    pub nickname: String,
    pub remark: String,
    pub alias: String,
    pub group_nickname: String,
    pub display_name: String,
}

// ───────────────────────────────── exporter ─────────────────────────────────

#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub id: String,
    pub display_name: String,
    pub nickname: String,
    pub remark: String,
    pub is_group: bool,
}

#[derive(Clone, Debug)]
pub struct Settings {
    pub display_pref: DisplayPref,
    pub excel_compact: bool,
    pub exported_at: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            display_pref: DisplayPref::Remark,
            excel_compact: false,
            exported_at: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0),
        }
    }
}

pub struct Exporter<'a, 'n> {
    pub session: SessionInfo,
    pub my_wxid: String,
    pub raw_my_wxid: String,
    pub my_display: String,
    pub group_nicks: HashMap<String, String>,
    /// Extra group members that never spoke (ChatLab member list).
    pub group_members: Vec<String>,
    pub names: &'a mut NameBook<'n>,
    pub settings: Settings,
}

pub struct SenderFields {
    pub role: String,
    pub wxid: String,
    pub nickname: String,
    pub remark: String,
    pub group_nickname: String,
}

impl<'a, 'n> Exporter<'a, 'n> {
    fn resolve_profile(&mut self, wxid: &str, fallback: &str, extra: &[&str]) -> Profile {
        let resolved = {
            let w = wxid.trim();
            if !w.is_empty() {
                w.to_string()
            } else if !fallback.trim().is_empty() {
                fallback.trim().to_string()
            } else {
                "unknown".to_string()
            }
        };
        let contact = self.names.get(&resolved);
        let nickname = contact
            .as_ref()
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .or_else(|| Some(fallback.to_string()).filter(|f| !f.is_empty()))
            .unwrap_or_else(|| resolved.clone());
        let remark = contact
            .as_ref()
            .map(|c| c.remark.clone())
            .unwrap_or_default();
        let alias = contact
            .as_ref()
            .map(|c| c.alias.clone())
            .unwrap_or_default();
        let username = contact
            .as_ref()
            .map(|c| c.username.clone())
            .unwrap_or_default();
        let mut candidates: Vec<&str> = vec![resolved.as_str(), username.as_str(), alias.as_str()];
        candidates.extend_from_slice(extra);
        let group_nickname = resolve_group_nickname(&self.group_nicks, &candidates);
        let display_name =
            self.settings
                .display_pref
                .pick(&resolved, &nickname, &remark, &group_nickname);
        Profile {
            wxid: resolved,
            nickname,
            remark,
            alias,
            group_nickname,
            display_name,
        }
    }

    fn sender_profile_for(&mut self, msg: &ExportMsg) -> Profile {
        let my = self.my_wxid.clone();
        let raw = self.raw_my_wxid.clone();
        let my_display = if self.my_display.is_empty() {
            my.clone()
        } else {
            self.my_display.clone()
        };
        if msg.is_send {
            self.resolve_profile(&my, &my_display, &[raw.as_str(), my.as_str()])
        } else {
            let who = if msg.sender_username.is_empty() {
                my.clone()
            } else {
                msg.sender_username.clone()
            };
            self.resolve_profile(&who, &msg.sender_username, &[])
        }
    }

    pub fn sender_fields(&mut self, msg: &ExportMsg) -> SenderFields {
        if self.session.is_group {
            let p = self.sender_profile_for(msg);
            return SenderFields {
                role: p.display_name,
                wxid: p.wxid,
                nickname: p.nickname,
                remark: p.remark,
                group_nickname: p.group_nickname,
            };
        }
        if msg.is_send {
            let nick = if self.my_display.is_empty() {
                self.my_wxid.clone()
            } else {
                self.my_display.clone()
            };
            return SenderFields {
                role: chrome("我", "Me").into(),
                wxid: self.my_wxid.clone(),
                nickname: nick,
                remark: String::new(),
                group_nickname: String::new(),
            };
        }
        let id = self.session.id.clone();
        match self.names.get(&id) {
            Some(c) => {
                let nickname = if c.nickname.is_empty() {
                    id.clone()
                } else {
                    c.nickname.clone()
                };
                let role = if !c.remark.is_empty() {
                    c.remark.clone()
                } else {
                    nickname.clone()
                };
                SenderFields {
                    role,
                    wxid: id,
                    nickname,
                    remark: c.remark,
                    group_nickname: String::new(),
                }
            }
            None => {
                let nickname = if self.session.display_name.is_empty() {
                    id.clone()
                } else {
                    self.session.display_name.clone()
                };
                SenderFields {
                    role: nickname.clone(),
                    wxid: id,
                    nickname,
                    remark: String::new(),
                    group_nickname: String::new(),
                }
            }
        }
    }

    fn contact_display(&mut self, username: &str) -> String {
        match self.names.get(username) {
            Some(c) => {
                if !c.remark.is_empty() {
                    c.remark
                } else if !c.nickname.is_empty() {
                    c.nickname
                } else if !c.alias.is_empty() {
                    c.alias
                } else {
                    username.to_string()
                }
            }
            None => username.to_string(),
        }
    }

    fn transfer_desc(&mut self, content: &str) -> Option<String> {
        let (payer, receiver) = transfer_parties(content)?;
        let my = self.my_wxid.clone();
        let raw = self.raw_my_wxid.clone();
        let resolve = |this: &mut Self, username: &str| -> String {
            if !my.is_empty() && (username == raw || username == my) {
                let g = resolve_group_nickname(
                    &this.group_nicks,
                    &[username, raw.as_str(), my.as_str()],
                );
                if !g.is_empty() {
                    return g;
                }
                return chrome("我", "Me").into();
            }
            let g = resolve_group_nickname(&this.group_nicks, &[username]);
            if !g.is_empty() {
                return g;
            }
            this.contact_display(username)
        };
        let payer_name = resolve(self, &payer);
        let receiver_name = resolve(self, &receiver);
        Some(format!("{payer_name} 转账给 {receiver_name}"))
    }

    fn quoted_with_names(&mut self, local_type: i64, content: &str) -> Option<QuotedDisplay> {
        if !is_text_quote_message(local_type, content) {
            return None;
        }
        let base = extract_quoted_reply_display(content)?;
        if base
            .quoted_sender
            .as_deref()
            .map_or(false, |s| !s.is_empty())
        {
            return Some(base);
        }
        let Some(user) = quoted_sender_username(content) else {
            return Some(base);
        };
        let my = self.my_wxid.clone();
        let is_self = is_same_wxid(&user, &my);
        let fallback = if is_self {
            if self.my_display.is_empty() {
                user.clone()
            } else {
                self.my_display.clone()
            }
        } else {
            user.clone()
        };
        let raw = self.raw_my_wxid.clone();
        let extra: Vec<&str> = if is_self {
            vec![raw.as_str(), my.as_str()]
        } else {
            Vec::new()
        };
        let profile = self.resolve_profile(&user, &fallback, &extra);
        let name = if !profile.display_name.is_empty() {
            profile.display_name
        } else {
            fallback
        };
        Some(QuotedDisplay {
            quoted_sender: Some(name),
            ..base
        })
    }

    /// 一条消息的展示文本，detailed JSON 与简易 txt 共用，保证两种格式写法一致：
    /// 引用消息带引用人名字，链接带 URL，转账带「谁转给谁」，系统消息统一用 `[系统: …]` 包裹，
    /// 表情 `[表情]`、语音 `[语音消息]`、图片 `[图片]`（不含媒体文件路径）。
    pub fn message_text(&mut self, msg: &ExportMsg) -> String {
        self.message_text_with(msg, Some(LinkStyle::AppendUrl), true)
    }

    /// [`message_text`](Self::message_text) 的可调版本：`link` 为链接卡片的写法（`None` 保持解析出的
    /// `[链接] 标题`，用于 HTML 这类另有链接卡片的格式），`wrap_system` 控制系统消息是否包成 `[系统: …]`
    /// （ChatLab 有独立的 system 类型、HTML 有专门的样式，不再包一层）。
    pub fn message_text_with(
        &mut self,
        msg: &ExportMsg,
        link: Option<LinkStyle>,
        wrap_system: bool,
    ) -> String {
        if let Some(text) = plain_message_text(msg) {
            return text;
        }
        self.message_text_generic(msg, link, wrap_system)
    }

    /// 完整流程（不走快速路径）。
    fn message_text_generic(
        &mut self,
        msg: &ExportMsg,
        link: Option<LinkStyle>,
        wrap_system: bool,
    ) -> String {
        let my = self.my_wxid.clone();
        let mut content = if msg.local_type == 3 {
            Some("[图片]".to_string())
        } else {
            parse_message_content(
                &msg.content,
                msg.local_type,
                Some(&my),
                Some(&msg.sender_username),
                msg.emoji_caption.as_deref(),
            )
        };
        if is_readable_system_message(msg.local_type, &msg.content) {
            let t = extract_readable_system_message_text(&msg.content);
            if !t.is_empty() {
                content = Some(t);
            }
        }
        let quoted = self.quoted_with_names(msg.local_type, &msg.content);
        if let Some(q) = &quoted {
            content = Some(build_quoted_reply_text(q));
        }
        if let (None, Some(style), true) = (&quoted, link, msg.local_type != 3) {
            if let Some(l) = format_link_card_export_text(&msg.content, msg.local_type, style) {
                content = Some(l);
            }
        }
        if let Some(c) = content.clone() {
            if is_transfer_export_content(&c) && !msg.content.is_empty() {
                if let Some(desc) = self.transfer_desc(&msg.content) {
                    content = Some(append_transfer_desc(&c, &desc));
                }
            }
        }
        // 表情有描述时写成 [表情：描述]（type 8 的表情走的是 appmsg，解析结果里没有描述）
        if let (Some("[表情]"), Some(c)) = (content.as_deref(), msg.emoji_caption.as_deref()) {
            content = Some(format_emoji_semantic_text(Some(c)));
        }
        // 内容为空的媒体消息（库里没存 XML）仍然给出占位符
        let text = content.unwrap_or_else(|| match msg.local_type {
            34 => "[语音消息]".into(),
            43 => "[视频]".into(),
            _ => String::new(),
        });
        if wrap_system && matches!(msg.local_type, 10000 | 266_287_972_401) && !text.is_empty() {
            return format!("[{}: {text}]", chrome("系统", "System"));
        }
        text
    }

    /// Text for TXT / Excel rows (formatPlainExportContent + transfer + quote + link).
    pub fn plain_row_content(&mut self, msg: &ExportMsg) -> (String, bool) {
        if let Some(m) = msg.media.as_ref().filter(|_| msg.local_type != 47) {
            return (m.relative_path.clone(), false);
        }
        let has_quote = is_text_quote_message(msg.local_type, &msg.content);
        (self.message_text(msg), has_quote)
    }

    fn sessions_members(&mut self, msgs: &[ExportMsg]) -> Vec<String> {
        let mut seen: Vec<String> = Vec::new();
        for m in msgs {
            if !m.sender_username.is_empty() && !seen.contains(&m.sender_username) {
                seen.push(m.sender_username.clone());
            }
        }
        if self.session.is_group {
            for g in self.group_members.clone() {
                if !g.is_empty() && !seen.contains(&g) {
                    seen.push(g);
                }
            }
        }
        seen
    }

    // ───────────────────────────────── ChatLab ─────────────────────────────────

    fn chatlab_meta(&self) -> (Value, Value) {
        let header = json!({ "version": "0.0.2", "exportedAt": self.settings.exported_at, "generator": "WeFlow" });
        let mut meta = Map::new();
        meta.insert("name".into(), json!(self.session.display_name));
        meta.insert("platform".into(), json!("wechat"));
        meta.insert(
            "type".into(),
            json!(if self.session.is_group {
                "group"
            } else {
                "private"
            }),
        );
        if self.session.is_group {
            meta.insert("groupId".into(), json!(self.session.id));
        }
        (header, Value::Object(meta))
    }

    pub fn build_chatlab(&mut self, msgs: &[ExportMsg]) -> Value {
        let mut out_msgs: Vec<Value> = Vec::new();
        let (chatlab, meta, members) =
            self.chatlab_parts(msgs, &mut |m| out_msgs.push(Value::Object(m)));
        json!({ "chatlab": chatlab, "meta": meta, "members": members, "messages": out_msgs })
    }

    /// Header, meta and (resolved) members of a ChatLab export; every message object goes to `sink` in order.
    fn chatlab_parts(
        &mut self,
        msgs: &[ExportMsg],
        sink: &mut dyn FnMut(Map<String, Value>),
    ) -> (Value, Value, Vec<Value>) {
        let is_group = self.session.is_group;
        let mut members: Vec<Map<String, Value>> = Vec::new();
        let mut member_index: HashMap<String, usize> = HashMap::new();
        for id in self.sessions_members(msgs) {
            let account_name = self.contact_display(&id);
            let mut m = Map::new();
            m.insert("platformId".into(), json!(id));
            m.insert("accountName".into(), json!(account_name));
            member_index.insert(id, members.len());
            members.push(m);
        }
        for msg in msgs {
            let member_name = members
                .get(
                    *member_index
                        .get(&msg.sender_username)
                        .unwrap_or(&usize::MAX),
                )
                .and_then(|m| m.get("accountName").and_then(Value::as_str))
                .map(str::to_string)
                .unwrap_or_else(|| msg.sender_username.clone());
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
                    nickname: member_name.clone(),
                    display_name: member_name.clone(),
                    group_nickname: group_nickname.clone(),
                    ..Default::default()
                }
            };
            let mut content = Some(self.message_text_with(msg, Some(LinkStyle::Markdown), false));
            if let Some(media) = msg.media.as_ref().filter(|_| msg.local_type == 3) {
                content = Some(media.relative_path.clone());
            }
            let mut m = Map::new();
            m.insert("sender".into(), json!(msg.sender_username));
            m.insert(
                "accountName".into(),
                json!(if profile.display_name.is_empty() {
                    member_name.clone()
                } else {
                    profile.display_name.clone()
                }),
            );
            let gn = if !profile.group_nickname.is_empty() {
                profile.group_nickname.clone()
            } else {
                group_nickname
            };
            if !gn.is_empty() {
                m.insert("groupNickname".into(), json!(gn));
            }
            m.insert("timestamp".into(), json!(msg.create_time));
            m.insert(
                "type".into(),
                json!(convert_message_type(msg.local_type, &msg.content)),
            );
            m.insert(
                "content".into(),
                content.map(Value::String).unwrap_or(Value::Null),
            );
            if let Some(id) = msg.platform_message_id() {
                m.insert("platformMessageId".into(), json!(id));
            }
            if let Some(r) = extract_reply_to_message_id(&msg.content) {
                m.insert("replyToMessageId".into(), json!(r));
            }
            if let Some(records) = msg.chat_record_list.as_ref().filter(|l| !l.is_empty()) {
                let mut chat_records: Vec<Value> = Vec::new();
                for rec in records {
                    let ts = parse_record_time(&rec.sourcetime).unwrap_or(msg.create_time);
                    let (rtype, rcontent) = match rec.datatype {
                        1 => (
                            0,
                            rec.datadesc
                                .clone()
                                .or(rec.datatitle.clone())
                                .unwrap_or_default(),
                        ),
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
                        _ => (
                            0,
                            rec.datadesc
                                .clone()
                                .or(rec.datatitle.clone())
                                .unwrap_or_else(|| "[消息]".into()),
                        ),
                    };
                    let who = if rec.sourcename.is_empty() {
                        "unknown".to_string()
                    } else {
                        rec.sourcename.clone()
                    };
                    chat_records.push(json!({ "sender": who, "accountName": who, "timestamp": ts, "type": rtype, "content": rcontent }));
                    if !rec.sourcename.is_empty() && !member_index.contains_key(&rec.sourcename) {
                        let mut nm = Map::new();
                        nm.insert("platformId".into(), json!(rec.sourcename));
                        nm.insert("accountName".into(), json!(rec.sourcename));
                        member_index.insert(rec.sourcename.clone(), members.len());
                        members.push(nm);
                    }
                }
                m.insert("chatRecords".into(), Value::Array(chat_records));
            }
            sink(m);
        }
        // enrich members with resolved names like the desktop app does for groups
        let mut final_members: Vec<Value> = Vec::new();
        for mut m in members {
            let id = m
                .get("platformId")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if is_group {
                let raw = self.raw_my_wxid.clone();
                let my = self.my_wxid.clone();
                let extra: Vec<&str> = if is_same_wxid(&id, &my) {
                    vec![raw.as_str(), my.as_str()]
                } else {
                    Vec::new()
                };
                let fallback = m
                    .get("accountName")
                    .and_then(Value::as_str)
                    .unwrap_or(&id)
                    .to_string();
                let p = self.resolve_profile(&id, &fallback, &extra);
                m.insert(
                    "accountName".into(),
                    json!(if p.display_name.is_empty() {
                        fallback
                    } else {
                        p.display_name
                    }),
                );
                if !p.group_nickname.is_empty() {
                    m.insert("groupNickname".into(), json!(p.group_nickname));
                }
            }
            final_members.push(Value::Object(m));
        }
        let (chatlab, meta) = self.chatlab_meta();
        (chatlab, meta, final_members)
    }

    pub fn write_chatlab(&mut self, msgs: &[ExportMsg], out: &Path, jsonl: bool) -> Result<()> {
        use std::io::Write;
        // Messages are serialized as they are built and only their text is kept; members are known afterwards.
        let mut lines: Vec<String> = Vec::with_capacity(msgs.len());
        let (chatlab, meta, members) = self.chatlab_parts(msgs, &mut |m| {
            lines.push(if jsonl {
                let mut obj = Map::new();
                obj.insert("_type".into(), json!("message"));
                obj.extend(m);
                serde_json::to_string(&Value::Object(obj)).unwrap_or_default()
            } else {
                pretty_indented(&Value::Object(m), 4)
            });
        });
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
        if jsonl {
            write!(
                w,
                "{}",
                serde_json::to_string(
                    &json!({ "_type": "header", "chatlab": chatlab, "meta": meta })
                )?
            )?;
            for member in &members {
                let mut obj = Map::new();
                obj.insert("_type".into(), json!("member"));
                if let Some(o) = member.as_object() {
                    obj.extend(o.clone());
                }
                write!(w, "\n{}", serde_json::to_string(&Value::Object(obj))?)?;
            }
            for line in &lines {
                w.write_all(b"\n")?;
                w.write_all(line.as_bytes())?;
            }
        } else {
            write!(
                w,
                "{{\n  \"chatlab\": {},\n  \"meta\": {},\n  \"members\": {},\n  \"messages\": ",
                pretty_indented(&chatlab, 2),
                pretty_indented(&meta, 2),
                pretty_indented(&Value::Array(members), 2)
            )?;
            if lines.is_empty() {
                w.write_all(b"[]")?;
            } else {
                w.write_all(b"[\n    ")?;
                for (i, line) in lines.iter().enumerate() {
                    if i > 0 {
                        w.write_all(b",\n    ")?;
                    }
                    w.write_all(line.as_bytes())?;
                }
                w.write_all(b"\n  ]")?;
            }
            w.write_all(b"\n}")?;
        }
        w.flush()?;
        Ok(())
    }

    // ─────────────────────────── detailed JSON / arkme-json ───────────────────────────

    fn weflow_header(&self, format: Option<&str>) -> Value {
        let mut h = Map::new();
        h.insert("version".into(), json!("1.0.3"));
        h.insert("exportedAt".into(), json!(self.settings.exported_at));
        h.insert("generator".into(), json!("WeFlow"));
        if let Some(f) = format {
            h.insert("format".into(), json!(f));
        }
        Value::Object(h)
    }

    /// Builds the detailed-JSON object of every message in order and hands each to `sink`, so a caller can write
    /// it out and drop it instead of holding the whole conversation as JSON values. Returns the sender profiles.
    fn json_messages(
        &mut self,
        msgs: &[ExportMsg],
        arkme: bool,
        sink: &mut dyn FnMut(Map<String, Value>, &Profile),
    ) -> HashMap<String, Profile> {
        let mut count = 0usize;
        let mut profiles: HashMap<String, Profile> = HashMap::new();
        for msg in msgs {
            let source = rx(r"(?i)<msgsource>[\s\S]*?</msgsource>")
                .find(&msg.content)
                .map(|m| m.as_str().to_string())
                .unwrap_or_default();
            let mut content = Some(self.message_text(msg));
            let quoted = self.quoted_with_names(msg.local_type, &msg.content);
            if let Some(m) = msg.media.as_ref().filter(|_| msg.local_type != 47) {
                content = Some(m.relative_path.clone());
            }
            let sender = msg.sender_username.clone();
            let nickname = self
                .names
                .get(&sender)
                .map(|c| c.nickname)
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| self.names.display_name(&sender));
            let remark = self
                .names
                .get(&sender)
                .map(|c| c.remark)
                .unwrap_or_default();
            let group_nick = resolve_group_nickname(&self.group_nicks, &[sender.as_str()]);
            let display = self
                .settings
                .display_pref
                .pick(&sender, &nickname, &remark, &group_nick);
            profiles.entry(sender.clone()).or_insert_with(|| Profile {
                wxid: sender.clone(),
                nickname: nickname.clone(),
                remark: remark.clone(),
                group_nickname: group_nick.clone(),
                display_name: display.clone(),
                ..Default::default()
            });

            let mut o = Map::new();
            o.insert("localId".into(), json!(count + 1));
            o.insert("createTime".into(), json!(msg.create_time));
            o.insert(
                "formattedTime".into(),
                json!(format_timestamp(msg.create_time)),
            );
            o.insert(
                "type".into(),
                json!(message_type_name(msg.local_type, Some(&msg.content))),
            );
            o.insert("localType".into(), json!(msg.local_type));
            o.insert(
                "content".into(),
                content.clone().map(Value::String).unwrap_or(Value::Null),
            );
            o.insert("isSend".into(), json!(if msg.is_send { 1 } else { 0 }));
            o.insert("senderUsername".into(), json!(sender));
            o.insert("senderDisplayName".into(), json!(display));
            o.insert("source".into(), json!(source));
            o.insert("senderAvatarKey".into(), json!(sender));
            if msg.local_type == 47 {
                if let Some(v) = &msg.emoji_md5 {
                    o.insert("emojiMd5".into(), json!(v));
                }
                if let Some(v) = &msg.emoji_cdn_url {
                    o.insert("emojiCdnUrl".into(), json!(v));
                }
                if let Some(v) = &msg.emoji_caption {
                    o.insert("emojiCaption".into(), json!(v));
                }
            }
            // Additive (not in the desktop export): media identifiers so exports can be matched with files on disk.
            if msg.local_type == 3 {
                if let Some(v) = &msg.image_md5 {
                    o.insert("imageMd5".into(), json!(v));
                }
                if let Some(v) = &msg.image_dat_name {
                    o.insert("imageDatName".into(), json!(v));
                }
            }
            if msg.local_type == 43 {
                if let Some(v) = &msg.video_md5 {
                    o.insert("videoMd5".into(), json!(v));
                }
            }
            if let Some(id) = msg.platform_message_id() {
                o.insert("platformMessageId".into(), json!(id));
            }
            if let Some(r) = extract_reply_to_message_id(&msg.content) {
                o.insert("replyToMessageId".into(), json!(r));
            }
            if let Some(meta) = extract_arkme_app_message_meta(&msg.content, msg.local_type) {
                let kind = meta.get("appMsgKind").and_then(Value::as_str).unwrap_or("");
                if arkme || kind == "quote" || kind == "link" {
                    for (k, v) in meta {
                        o.insert(k, v);
                    }
                }
            }
            if let Some(q) = &quoted {
                if let Some(s) = &q.quoted_sender {
                    if !s.is_empty() {
                        o.insert("quotedSender".into(), json!(s));
                    }
                }
                if !q.quoted_preview.is_empty() {
                    o.insert("quotedContent".into(), json!(q.quoted_preview));
                }
            }
            if arkme {
                if let Some(meta) = extract_arkme_contact_card_meta(&msg.content, msg.local_type) {
                    for (k, v) in meta {
                        o.insert(k, v);
                    }
                }
            }
            if msg.local_type == 48 {
                if let Some(v) = msg.location_lat {
                    o.insert("locationLat".into(), json!(v));
                }
                if let Some(v) = msg.location_lng {
                    o.insert("locationLng".into(), json!(v));
                }
                if let Some(v) = &msg.location_poiname {
                    o.insert("locationPoiname".into(), json!(v));
                }
                if let Some(v) = &msg.location_label {
                    o.insert("locationLabel".into(), json!(v));
                }
            }
            count += 1;
            sink(o, &profiles[&msg.sender_username]);
        }
        profiles
    }

    /// `{ wxid, nickname, remark, displayName, type, lastTimestamp, messageCount }` of the exported session.
    fn json_session(&mut self, count: usize, last_ts: Option<i64>) -> Map<String, Value> {
        let is_group = self.session.is_group;
        let session_contact = self.names.get(&self.session.id.clone());
        let session_nickname = session_contact
            .as_ref()
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| self.session.display_name.clone());
        let session_remark = session_contact
            .as_ref()
            .map(|c| c.remark.clone())
            .unwrap_or_default();
        let session_group_nick = if is_group {
            resolve_group_nickname(&self.group_nicks, &[self.session.id.as_str()])
        } else {
            String::new()
        };
        let session_display = self.settings.display_pref.pick(
            &self.session.id,
            &session_nickname,
            &session_remark,
            &session_group_nick,
        );
        let mut session = Map::new();
        session.insert("wxid".into(), json!(self.session.id));
        session.insert("nickname".into(), json!(session_nickname));
        session.insert("remark".into(), json!(session_remark));
        session.insert("displayName".into(), json!(session_display));
        session.insert("type".into(), json!(if is_group { "群聊" } else { "私聊" }));
        session.insert(
            "lastTimestamp".into(),
            last_ts.map(|t| json!(t)).unwrap_or(Value::Null),
        );
        session.insert("messageCount".into(), json!(count));

        session
    }

    pub fn build_json(&mut self, msgs: &[ExportMsg], arkme: bool) -> Value {
        let is_group = self.session.is_group;
        let mut out: Vec<Map<String, Value>> = Vec::new();
        let profiles = self.json_messages(msgs, arkme, &mut |m, _| out.push(m));
        out.sort_by_key(|o| o.get("createTime").and_then(Value::as_i64).unwrap_or(0));
        let last_ts = out
            .iter()
            .filter_map(|o| o.get("createTime").and_then(Value::as_i64))
            .max();
        let session = self.json_session(out.len(), last_ts);

        if !arkme {
            return json!({ "weflow": self.weflow_header(None), "session": Value::Object(session), "messages": out.into_iter().map(Value::Object).collect::<Vec<_>>() });
        }

        // arkme-json: compact messages + sender table (+ group members)
        let mut ark_state = ArkCompact::default();
        let compact: Vec<Value> = out
            .iter()
            .map(|o| Value::Object(ark_state.compact(o, &|s| profiles.get(s).cloned())))
            .collect();
        let mut ark = Map::new();
        ark.insert("weflow".into(), self.weflow_header(Some("arkme-json")));
        ark.insert("session".into(), Value::Object(session));
        ark.insert("senders".into(), Value::Array(ark_state.senders));
        ark.insert("messages".into(), Value::Array(compact));
        if is_group {
            let members = self.ark_group_members(msgs);
            ark.insert("groupMembers".into(), members);
        }
        Value::Object(ark)
    }

    /// `groupMembers` of the arkme export: every sender plus the group's member list, busiest first.
    fn ark_group_members(&mut self, msgs: &[ExportMsg]) -> Value {
        let ids: Vec<String> = {
            let mut v: Vec<String> = Vec::new();
            for m in msgs {
                if !m.sender_username.is_empty() && !v.contains(&m.sender_username) {
                    v.push(m.sender_username.clone());
                }
            }
            for g in &self.group_members {
                if !g.is_empty() && !v.contains(g) {
                    v.push(g.clone());
                }
            }
            v
        };
        let mut counts: HashMap<String, i64> = HashMap::new();
        for m in msgs {
            *counts.entry(m.sender_username.to_lowercase()).or_insert(0) += 1;
        }
        let mut members: Vec<Map<String, Value>> = Vec::new();
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
            let mut m = Map::new();
            m.insert("wxid".into(), json!(id));
            m.insert("displayName".into(), json!(display));
            m.insert("nickname".into(), json!(nickname));
            m.insert("remark".into(), json!(remark));
            m.insert("alias".into(), json!(alias));
            if !group_nick.is_empty() {
                m.insert("groupNickname".into(), json!(group_nick));
            }
            m.insert("isFriend".into(), json!(false));
            m.insert(
                "messageCount".into(),
                json!(counts.get(&id.to_lowercase()).copied().unwrap_or(0)),
            );
            members.push(m);
        }
        members.sort_by(|a, b| {
            let ca = a["messageCount"].as_i64().unwrap_or(0);
            let cb = b["messageCount"].as_i64().unwrap_or(0);
            let name = |m: &Map<String, Value>| {
                ["displayName", "wxid"]
                    .iter()
                    .filter_map(|k| m.get(*k).and_then(Value::as_str))
                    .find(|s| !s.is_empty())
                    .unwrap_or("")
                    .to_string()
            };
            cb.cmp(&ca)
                .then_with(|| crate::collate::compare_zh(&name(a), &name(b)))
        });
        Value::Array(members.into_iter().map(Value::Object).collect())
    }

    pub fn write_json(&mut self, msgs: &[ExportMsg], out: &Path, arkme: bool) -> Result<()> {
        // Written message by message (the same bytes as pretty-printing the whole value); that needs the messages
        // in time order, which collected conversations always are.
        if msgs
            .windows(2)
            .all(|w| w[0].create_time <= w[1].create_time)
        {
            return self.write_json_streaming(msgs, out, arkme);
        }
        let v = self.build_json(msgs, arkme);
        write_bytes(out, serde_json::to_string_pretty(&v)?.as_bytes())
    }

    fn write_json_streaming(&mut self, msgs: &[ExportMsg], out: &Path, arkme: bool) -> Result<()> {
        use std::io::Write;
        let is_group = self.session.is_group;
        let session =
            Value::Object(self.json_session(msgs.len(), msgs.iter().map(|m| m.create_time).max()));
        // Each message is serialized as soon as it is built; only the text is kept until the file is written.
        let mut lines: Vec<String> = Vec::with_capacity(msgs.len());
        let mut ark = ArkCompact::default();
        self.json_messages(msgs, arkme, &mut |message, profile| {
            let value = if arkme {
                let sender = message
                    .get("senderUsername")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .trim()
                    .to_string();
                Value::Object(ark.compact(&message, &|s| (s == sender).then(|| profile.clone())))
            } else {
                Value::Object(message)
            };
            lines.push(pretty_indented(&value, 4));
        });
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
        write!(
            w,
            "{{\n  \"weflow\": {},\n  \"session\": {}",
            pretty_indented(&self.weflow_header(arkme.then_some("arkme-json")), 2),
            pretty_indented(&session, 2)
        )?;
        if arkme {
            write!(
                w,
                ",\n  \"senders\": {}",
                pretty_indented(&Value::Array(std::mem::take(&mut ark.senders)), 2)
            )?;
        }
        w.write_all(b",\n  \"messages\": ")?;
        if lines.is_empty() {
            w.write_all(b"[]")?;
        } else {
            w.write_all(b"[\n    ")?;
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    w.write_all(b",\n    ")?;
                }
                w.write_all(line.as_bytes())?;
            }
            w.write_all(b"\n  ]")?;
        }
        drop(lines);
        if arkme && is_group {
            let members = self.ark_group_members(msgs);
            write!(w, ",\n  \"groupMembers\": {}", pretty_indented(&members, 2))?;
        }
        w.write_all(b"\n}")?;
        w.flush()?;
        Ok(())
    }

    // ─────────────────────────────────── TXT ───────────────────────────────────

    pub fn render_txt(&mut self, msgs: &[ExportMsg]) -> String {
        let mut out = String::new();
        for msg in msgs {
            let (content, _) = self.plain_row_content(msg);
            let sender = self.sender_fields(msg);
            out.push_str(&format!(
                "{} '{}'\n{}\n\n",
                format_timestamp(msg.create_time),
                sender.role,
                content
            ));
        }
        out
    }

    pub fn write_txt(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let text = self.render_txt(msgs);
        write_bytes(out, text.as_bytes())
    }

    // ───────────────────────────────── WeClone CSV ─────────────────────────────────

    pub fn render_weclone(&mut self, msgs: &[ExportMsg]) -> String {
        let mut out =
            String::from("\u{feff}id,MsgSvrID,type_name,is_sender,talker,msg,src,CreateTime\r\n");
        let my = self.my_wxid.clone();
        let rows: Vec<&ExportMsg> = msgs
            .iter()
            .filter(|m| !is_quoted_reply_message(m.local_type, &m.content))
            .collect();
        for (i, msg) in rows.iter().enumerate() {
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
                (i + 1).to_string(),
                msg.platform_message_id().unwrap_or_default(),
                type_name.to_string(),
                if msg.is_send { "1".into() } else { "0".into() },
                talker,
                text,
                src,
                format_iso_timestamp(msg.create_time),
            ];
            let line: Vec<String> = cells.iter().map(|c| csv_cell(c)).collect();
            out.push_str(&line.join(","));
            out.push_str("\r\n");
        }
        out
    }

    pub fn write_weclone(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let text = self.render_weclone(msgs);
        write_bytes(out, text.as_bytes())
    }

    // ─────────────────────────────────── Excel ───────────────────────────────────

    pub fn write_excel(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        use rust_xlsxwriter::{Color, Format, FormatAlign, Url, Workbook};
        let is_group = self.session.is_group;
        let compact = self.settings.excel_compact;
        let include_group_col = !compact && is_group;
        let big = msgs.len() > 20000;
        let mut workbook = Workbook::new();
        let ws = if big {
            workbook.add_worksheet_with_constant_memory()
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
            ws.write_string_with_format(3, i as u16, *h, &head)?;
        }
        let widths: Vec<f64> = if compact {
            vec![8.0, 20.0, 18.0, 12.0, 50.0]
        } else if include_group_col {
            vec![8.0, 20.0, 18.0, 25.0, 18.0, 18.0, 15.0, 12.0, 50.0]
        } else {
            vec![8.0, 20.0, 18.0, 25.0, 18.0, 15.0, 12.0, 50.0]
        };
        for (i, w) in widths.iter().enumerate() {
            ws.set_column_width(i as u16, *w)?;
        }

        let content_col = (headers.len() - 1) as u16;
        for (i, msg) in msgs.iter().enumerate() {
            let row = (i + 4) as u32;
            let (content, has_quote) = self.plain_row_content(msg);
            let s = self.sender_fields(msg);
            let type_name = message_type_name(msg.local_type, Some(&msg.content));
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
        workbook
            .save(out)
            .with_context(|| format!("failed to write {}", out.display()))?;
        Ok(())
    }

    // ─────────────────────────────────── HTML ───────────────────────────────────

    /// HTML 气泡里的文本：与其他格式同一套写法，但链接卡片、系统消息样式由 HTML 自己渲染，不再改写。
    fn html_message_text(&mut self, msg: &ExportMsg) -> String {
        if msg.content.is_empty() && msg.local_type != 47 {
            return String::new();
        }
        self.message_text_with(msg, None, false)
    }

    pub fn render_html(&mut self, msgs: &[ExportMsg]) -> String {
        let session = self.session.clone();
        let is_group = session.is_group;
        let rows: Vec<&ExportMsg> = msgs.iter().collect();
        let total = rows.len();
        let mut data: Vec<String> = Vec::with_capacity(total);
        for (i, msg) in rows.iter().enumerate() {
            let sender_name = if is_group {
                self.sender_profile_for(msg).display_name
            } else if msg.is_send {
                if self.my_display.is_empty() {
                    chrome("我", "Me").to_string()
                } else {
                    self.my_display.clone()
                }
            } else if session.display_name.is_empty() {
                session.id.clone()
            } else {
                session.display_name.clone()
            };
            let avatar_html = format!(
                "<span>{}</span>",
                html_escape(&avatar_fallback(&sender_name))
            );
            let time_text = format_timestamp(msg.create_time);
            let quoted = self.quoted_with_names(msg.local_type, &msg.content);
            let mut text = match &quoted {
                Some(q) if !q.reply_text.is_empty() => q.reply_text.clone(),
                _ => self.html_message_text(msg),
            };
            if let Some(q) = &quoted {
                if q.reply_text.is_empty() {
                    text = self.html_message_text(msg);
                }
            }
            if is_transfer_export_content(&text) && !msg.content.is_empty() {
                if let Some(desc) = self.transfer_desc(&msg.content) {
                    text = append_transfer_desc(&text, &desc);
                }
            }
            let link_card = if quoted.is_some() {
                None
            } else {
                extract_html_link_card(&msg.content, msg.local_type)
            };
            let multiline = |s: &str| {
                html_escape(s)
                    .replace("\r\n", "<br />")
                    .replace('\n', "<br />")
            };
            let text_html = if let Some(q) = &quoted {
                let sender = q
                    .quoted_sender
                    .as_ref()
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("<div class=\"quoted-sender\">{}</div>", html_escape(s)))
                    .unwrap_or_default();
                let preview = format!(
                    "<div class=\"quoted-text\">{}</div>",
                    multiline(&q.quoted_preview)
                );
                let reply = if text.is_empty() {
                    String::new()
                } else {
                    format!("<div class=\"message-text\">{}</div>", multiline(&text))
                };
                format!("<div class=\"quoted-message\">{sender}{preview}</div>{reply}")
            } else if let Some(card) = &link_card {
                format!(
                    "<div class=\"message-text\"><a class=\"message-link-card\" href=\"{}\" target=\"_blank\" rel=\"noopener noreferrer\">{}</a></div>",
                    html_attr_escape(&card.url),
                    multiline(&card.title)
                )
            } else if !text.is_empty() {
                format!("<div class=\"message-text\">{}</div>", multiline(&text))
            } else {
                String::new()
            };
            let sender_html = if is_group {
                format!(
                    "<div class=\"sender-name\">{}</div>",
                    html_escape(&sender_name)
                )
            } else {
                String::new()
            };
            let (media_html, text_html) = match &msg.media {
                Some(m) => {
                    let path = html_attr_escape(&encode_uri(&m.relative_path));
                    let html = match m.kind {
                        "image" | "emoji" => format!("<img class=\"message-media {} previewable\" src=\"{path}\" data-full=\"{path}\" alt=\"{}\" />", m.kind, m.kind),
                        "voice" => format!("<audio class=\"message-media audio\" controls src=\"{path}\"></audio>"),
                        _ => format!("<video class=\"message-media video\" controls preload=\"metadata\" src=\"{path}\"></video>"),
                    };
                    // an image message has no caption of its own
                    (
                        html,
                        if msg.local_type == 3 {
                            String::new()
                        } else {
                            text_html
                        },
                    )
                }
                None => (String::new(), text_html),
            };
            let body = format!(
                "<div class=\"message-time\">{}</div>{}<div class=\"message-content\">{}{}</div>",
                html_escape(&time_text),
                sender_html,
                media_html,
                text_html
            );
            let mut item = Map::new();
            item.insert("i".into(), json!(i + 1));
            item.insert("t".into(), json!(msg.create_time));
            item.insert("s".into(), json!(if msg.is_send { 1 } else { 0 }));
            item.insert("a".into(), json!(avatar_html));
            item.insert("b".into(), json!(body));
            if let Some(p) = msg.platform_message_id() {
                item.insert("p".into(), json!(p));
            }
            if let Some(r) = extract_reply_to_message_id(&msg.content) {
                item.insert("r".into(), json!(r));
            }
            data.push(
                serde_json::to_string(&Value::Object(item))
                    .unwrap_or_default()
                    .replace("</", "<\\/"),
            );
        }
        let title = html_escape(&session.display_name);
        let lang = match locale::current() {
            locale::Lang::Zh => "zh-CN",
            locale::Lang::En => "en",
        };
        let count_label = |n: usize| match locale::current() {
            locale::Lang::Zh => format!("共 {n} 条"),
            locale::Lang::En => format!("{n} in total"),
        };
        let messages_label = match locale::current() {
            locale::Lang::Zh => format!("{total} 条消息"),
            locale::Lang::En => format!("{total} messages"),
        };
        let scroll_js = html_script(
            &count_label(0).replace('0', "${N}"),
            chrome("暂无消息", "No messages"),
        );
        format!(
            "<!DOCTYPE html>\n<html lang=\"{lang}\">\n  <head>\n    <meta charset=\"UTF-8\" />\n    <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\" />\n    <title>{title} - {chat_history}</title>\n    <style>{styles}</style>\n  </head>\n  <body>\n    <div class=\"page\">\n      <div class=\"header\">\n        <h1 class=\"title\">{title}</h1>\n        <div class=\"meta\">\n          <span>{messages_label}</span>\n          <span>{kind}</span>\n          <span>{exported}</span>\n        </div>\n        <div class=\"controls\">\n          <input id=\"searchInput\" type=\"search\" placeholder=\"{search}\" />\n          <input id=\"timeInput\" type=\"datetime-local\" />\n          <button id=\"jumpBtn\" type=\"button\">{jump}</button>\n          <div class=\"stats\">\n            <span id=\"resultCount\">{count}</span>\n          </div>\n        </div>\n      </div>\n      <div id=\"scrollContainer\" class=\"scroll-container\"></div>\n    </div>\n    <div class=\"image-preview\" id=\"imagePreview\">\n      <img id=\"imagePreviewTarget\" alt=\"{preview}\" />\n    </div>\n    <script>\n      window.WEFLOW_DATA = [\n{data}\n];\n    </script>\n    <script>\n{scroll_js}\n    </script>\n  </body>\n</html>\n",
            lang = lang,
            title = title,
            chat_history = chrome("聊天记录", "Chat history"),
            styles = HTML_STYLES,
            messages_label = messages_label,
            kind = if is_group { chrome("群聊", "Group chat") } else { chrome("私聊", "Private chat") },
            exported = html_escape(&format_timestamp(self.settings.exported_at)),
            search = chrome("搜索消息...", "Search messages..."),
            jump = chrome("跳转", "Go"),
            count = count_label(total),
            preview = chrome("预览", "Preview"),
            data = data.join(",\n"),
            scroll_js = scroll_js,
        )
    }

    pub fn write_html(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let html = self.render_html(msgs);
        write_bytes(out, html.as_bytes())
    }

    // ─────────────────────────────────── SQL ───────────────────────────────────

    pub fn render_sql(&mut self, msgs: &[ExportMsg]) -> String {
        let mut sql = String::new();
        sql.push_str(
            "CREATE TABLE IF NOT EXISTS \"messages\" (\n\tid INTEGER PRIMARY KEY AUTOINCREMENT,\n\tsession_id TEXT,\n\tlocal_id INTEGER,\n\tserver_id TEXT,\n\tcreate_time INTEGER,\n\tsender TEXT,\n\tis_send INTEGER,\n\tlocal_type INTEGER,\n\ttype_name TEXT,\n\tcontent TEXT\n);\n\n",
        );
        for msg in msgs {
            let (content, _) = self.plain_row_content(msg);
            sql.push_str(&format!(
                "INSERT INTO \"messages\" (session_id, local_id, server_id, create_time, sender, is_send, local_type, type_name, content) VALUES ('{}', {}, {}, {}, '{}', {}, {}, '{}', '{}');\n",
                sql_escape(&self.session.id),
                msg.local_id,
                msg.platform_message_id().map(|s| format!("'{s}'")).unwrap_or_else(|| "NULL".into()),
                msg.create_time,
                sql_escape(&msg.sender_username),
                if msg.is_send { 1 } else { 0 },
                msg.local_type,
                sql_escape(message_type_name(msg.local_type, Some(&msg.content))),
                sql_escape(&content),
            ));
        }
        sql
    }

    pub fn write_sql(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let sql = self.render_sql(msgs);
        write_bytes(out, sql.as_bytes())
    }
}

// ─────────────────────────────── small helpers ───────────────────────────────

/// Turns detailed-JSON message objects into arkme-json's compact form and numbers the senders on first sight.
#[derive(Default)]
struct ArkCompact {
    sender_ids: HashMap<String, usize>,
    senders: Vec<Value>,
}

impl ArkCompact {
    const KEEP: &'static [&'static str] = &[
        "localId",
        "createTime",
        "formattedTime",
        "type",
        "localType",
        "content",
        "isSend",
        "senderID",
        "source",
        "platformMessageId",
        "replyToMessageId",
        "locationLat",
        "locationLng",
        "locationPoiname",
        "locationLabel",
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

    /// `profile_of(sender)` supplies the sender's name data the first time that sender appears.
    fn compact(
        &mut self,
        o: &Map<String, Value>,
        profile_of: &dyn Fn(&str) -> Option<Profile>,
    ) -> Map<String, Value> {
        let sender = {
            let s = o
                .get("senderUsername")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if s.is_empty() {
                "unknown".to_string()
            } else {
                s
            }
        };
        let sid = match self.sender_ids.get(&sender) {
            Some(i) => *i,
            None => {
                let id = self.senders.len() + 1;
                self.sender_ids.insert(sender.clone(), id);
                let p = profile_of(&sender).unwrap_or_default();
                let mut s = Map::new();
                s.insert("senderID".into(), json!(id));
                s.insert("wxid".into(), json!(sender));
                s.insert(
                    "displayName".into(),
                    json!(if p.display_name.is_empty() {
                        sender.clone()
                    } else {
                        p.display_name.clone()
                    }),
                );
                s.insert(
                    "nickname".into(),
                    json!(if !p.nickname.is_empty() {
                        p.nickname.clone()
                    } else if !p.display_name.is_empty() {
                        p.display_name.clone()
                    } else {
                        sender.clone()
                    }),
                );
                if !p.remark.is_empty() {
                    s.insert("remark".into(), json!(p.remark));
                }
                if !p.group_nickname.is_empty() {
                    s.insert("groupNickname".into(), json!(p.group_nickname));
                }
                self.senders.push(Value::Object(s));
                id
            }
        };
        let mut c = Map::new();
        for key in Self::KEEP {
            if *key == "senderID" {
                c.insert((*key).into(), json!(sid));
            } else if let Some(v) = o.get(*key) {
                c.insert((*key).into(), v.clone());
            }
        }
        c
    }
}

/// JSON text of `value`, pretty-printed, with every line after the first indented by `spaces`.
fn pretty_indented(value: &Value, spaces: usize) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_default()
        .replace('\n', &format!("\n{}", " ".repeat(spaces)))
}

/// JavaScript's `encodeURI`: everything except letters, digits and `;,/?:@&=+$-_.!~*'()#` is percent-encoded.
fn encode_uri(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for b in path.bytes() {
        if b.is_ascii_alphanumeric() || b";,/?:@&=+$-_.!~*'()#".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

fn avatar_fallback(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_else(|| "?".into())
}

/// 文本、图片、语音、视频、表情这几种最常见的消息（约占 9 成）不含 XML，不需要引用 / 链接 / 转账 / 系统消息的
/// 解析（每一步都要把内容规范化、跑正则），直接给出结果；内容里带 appmsg / sysmsg / refermsg 的仍走完整流程。
/// 结果与完整流程一致（有测试对照）。
fn plain_message_text(msg: &ExportMsg) -> Option<String> {
    if !matches!(msg.local_type, 1 | 3 | 34 | 43 | 47) {
        return None;
    }
    let c = msg.content.as_str();
    if ["appmsg", "sysmsg", "refermsg"]
        .iter()
        .any(|k| c.contains(k))
    {
        return None;
    }
    Some(match msg.local_type {
        1 => strip_sender_prefix(c),
        3 => "[图片]".into(),
        34 => "[语音消息]".into(),
        43 => "[视频]".into(),
        _ => format_emoji_semantic_text(msg.emoji_caption.as_deref()),
    })
}

pub fn html_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn html_attr_escape(value: &str) -> String {
    html_escape(value).replace('`', "&#96;")
}

pub fn csv_cell(value: &str) -> String {
    if value.contains(['"', ',', '\r', '\n']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

pub fn sql_escape(value: &str) -> String {
    value.replace('\'', "''")
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

fn write_bytes(out: &Path, bytes: &[u8]) -> Result<()> {
    if let Some(parent) = out.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
    }
    fs::write(out, bytes).with_context(|| format!("failed to write {}", out.display()))
}

/// Client-side script of the exported HTML page (chunked renderer, search, jump).
fn html_script(count_template: &str, empty_label: &str) -> String {
    let (prefix, suffix) = count_template
        .split_once("${N}")
        .unwrap_or((count_template, ""));
    format!(
        r#"      class ChunkedRenderer {{
        constructor(container, data, renderItem) {{
          this.container = container; this.data = data; this.renderItem = renderItem;
          this.batchSize = 100; this.rendered = 0; this.loading = false;
          this.list = document.createElement('div'); this.list.className = 'message-list'; this.container.appendChild(this.list);
          this.sentinel = document.createElement('div'); this.sentinel.className = 'load-sentinel'; this.container.appendChild(this.sentinel);
          this.renderBatch();
          this.observer = new IntersectionObserver((entries) => {{ if (entries[0].isIntersecting && !this.loading) this.renderBatch(); }}, {{ root: this.container, rootMargin: '600px' }});
          this.observer.observe(this.sentinel);
        }}
        renderBatch() {{
          if (this.rendered >= this.data.length) return;
          this.loading = true;
          const end = Math.min(this.rendered + this.batchSize, this.data.length);
          const fragment = document.createDocumentFragment();
          for (let i = this.rendered; i < end; i++) {{
            const wrapper = document.createElement('div');
            wrapper.innerHTML = this.renderItem(this.data[i], i);
            if (wrapper.firstElementChild) fragment.appendChild(wrapper.firstElementChild);
          }}
          this.list.appendChild(fragment); this.rendered = end; this.loading = false;
        }}
        setData(newData) {{
          this.data = newData; this.rendered = 0; this.list.innerHTML = ''; this.container.scrollTop = 0;
          if (this.data.length === 0) {{ this.list.innerHTML = '<div class="empty">{empty}</div>'; return; }}
          this.renderBatch();
        }}
        scrollToTime(timestamp) {{
          const idx = this.data.findIndex(item => item.t >= timestamp);
          if (idx === -1) return;
          while (this.rendered <= idx) this.renderBatch();
          const el = this.list.children[idx];
          if (el) {{ el.scrollIntoView({{ behavior: 'smooth', block: 'center' }}); el.classList.add('highlight'); setTimeout(() => el.classList.remove('highlight'), 2500); }}
        }}
      }}
      const searchInput = document.getElementById('searchInput');
      const timeInput = document.getElementById('timeInput');
      const jumpBtn = document.getElementById('jumpBtn');
      const resultCount = document.getElementById('resultCount');
      const imagePreview = document.getElementById('imagePreview');
      const imagePreviewTarget = document.getElementById('imagePreviewTarget');
      const container = document.getElementById('scrollContainer');
      let allData = window.WEFLOW_DATA || [];
      let currentList = allData;
      const renderItem = (item) => {{
        const me = item.s === 1;
        const pid = item.p ? ` data-platform-message-id="${{item.p}}"` : '';
        const rid = item.r ? ` data-reply-to-message-id="${{item.r}}"` : '';
        return `<div class="message ${{me ? 'sent' : 'received'}}" data-index="${{item.i}}"${{pid}}${{rid}}><div class="message-row"><div class="avatar">${{item.a}}</div><div class="bubble">${{item.b}}</div></div></div>`;
      }};
      const renderer = new ChunkedRenderer(container, currentList, renderItem);
      const updateCount = () => {{ resultCount.textContent = `{prefix}${{currentList.length}}{suffix}`; }};
      let searchTimeout;
      searchInput.addEventListener('input', () => {{
        clearTimeout(searchTimeout);
        searchTimeout = setTimeout(() => {{
          const keyword = searchInput.value.trim().toLowerCase();
          currentList = keyword ? allData.filter(item => item.b.toLowerCase().includes(keyword)) : allData;
          renderer.setData(currentList); updateCount();
        }}, 300);
      }});
      jumpBtn.addEventListener('click', () => {{
        const value = timeInput.value; if (!value) return;
        renderer.scrollToTime(Math.floor(new Date(value).getTime() / 1000));
      }});
      updateCount();"#,
        prefix = prefix,
        suffix = suffix,
        empty = empty_label,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Vec<ExportMsg>, HashMap<String, ContactInfo>) {
        let rows = vec![
            json!({"local_id": "1", "server_id": "111", "create_time": "1700000000", "local_type": "1", "message_content": "hello \"world\", ok", "is_send": "1"}),
            json!({"local_id": "2", "server_id": "222", "create_time": "1700000060", "local_type": "1", "message_content": "wxid_bob:hi <b>there</b>", "sender_username": "wxid_bob", "is_send": "0"}),
            json!({"local_id": "3", "create_time": "1700000120", "local_type": "3", "message_content": "", "sender_username": "wxid_bob", "is_send": "0"}),
            json!({"local_id": "4", "create_time": "1700000180", "local_type": "49", "message_content": "<msg><appmsg><title>Doc</title><type>5</type><url>https://example.com/x</url></appmsg></msg>", "sender_username": "wxid_bob"}),
            json!({"local_id": "5", "server_id": "555", "create_time": "1700000240", "local_type": "49", "message_content": "<msg><appmsg><title>yes</title><type>57</type><refermsg><type>1</type><displayname>Me</displayname><content>hello</content><svrid>111</svrid></refermsg></appmsg></msg>", "sender_username": "wxid_bob"}),
        ];
        let msgs = collect_messages(
            &rows,
            &CollectOptions {
                session_id: "wxid_bob",
                my_wxid: "wxid_me",
                start: None,
                end: None,
                sender_filter: None,
            },
        );
        let mut contacts = HashMap::new();
        contacts.insert(
            "wxid_bob".into(),
            ContactInfo {
                username: "wxid_bob".into(),
                nickname: "Bob".into(),
                remark: "Bobby".into(),
                alias: "bb".into(),
            },
        );
        contacts.insert(
            "wxid_me".into(),
            ContactInfo {
                username: "wxid_me".into(),
                nickname: "Me Nick".into(),
                remark: String::new(),
                alias: String::new(),
            },
        );
        (msgs, contacts)
    }

    fn with_exporter<R>(
        group: bool,
        f: impl FnOnce(&mut Exporter<'_, '_>, &[ExportMsg]) -> R,
    ) -> R {
        let (msgs, contacts) = fixture();
        let mut names = NameBook::from_map(contacts);
        let mut ex = Exporter {
            session: SessionInfo {
                id: "wxid_bob".into(),
                display_name: "Bobby".into(),
                nickname: "Bob".into(),
                remark: "Bobby".into(),
                is_group: group,
            },
            my_wxid: "wxid_me".into(),
            raw_my_wxid: "wxid_me".into(),
            my_display: "Me Nick".into(),
            group_nicks: HashMap::new(),
            group_members: Vec::new(),
            names: &mut names,
            settings: Settings {
                exported_at: 1_700_001_000,
                ..Default::default()
            },
        };
        f(&mut ex, &msgs)
    }

    /// detailed JSON 与简易 txt 共用 `message_text`：各类消息的写法在两种格式里一致。
    #[test]
    fn message_text_is_uniform_across_message_kinds() {
        let quote_inner = "&lt;msg&gt;&lt;img md5=\"x\"&gt;&lt;extcommoninfo&gt;&lt;refermsg&gt;&lt;svrid&gt;9&lt;/svrid&gt;&lt;/refermsg&gt;&lt;/extcommoninfo&gt;&lt;/msg&gt;";
        let rows = vec![
            json!({"local_id": "1", "create_time": "1700000000", "local_type": "47", "message_content": "wxid_bob:<msg><emoji md5=\"a\"/></msg>", "sender_username": "wxid_bob"}),
            json!({"local_id": "2", "create_time": "1700000001", "local_type": "34", "message_content": "", "sender_username": "wxid_bob"}),
            json!({"local_id": "3", "create_time": "1700000002", "local_type": "10000", "message_content": "\"Bob\" 撤回了一条消息"}),
            // 图片里夹带 <refermsg>：不是文字引用，仍然是 [图片]
            json!({"local_id": "4", "create_time": "1700000003", "local_type": "3", "message_content": "wxid_bob:\n<msg><img md5=\"x\"/><extcommoninfo><refermsg><svrid>9</svrid></refermsg></extcommoninfo></msg>", "sender_username": "wxid_bob"}),
            // appmsg type 8：以附件形式发送的表情
            json!({"local_id": "5", "create_time": "1700000004", "local_type": "34359738417", "message_content": "wxid_bob:\n<msg><appmsg><title /><type>8</type></appmsg></msg>", "sender_username": "wxid_bob"}),
            // 引用一条「带引用的图片」：被引用内容是（转义后的）XML，且里面还嵌着一层 <refermsg>
            json!({"local_id": "6", "create_time": "1700000005", "local_type": "244813135921", "message_content": format!("wxid_bob:\n<msg><appmsg><title>好看</title><type>57</type><refermsg><type>3</type><content>wxid_me:\n{quote_inner}</content><displayname>Me Nick</displayname><svrid>1</svrid></refermsg></appmsg></msg>"), "sender_username": "wxid_bob"}),
            // 正文里的 "4:1" 不是发送者前缀
            json!({"local_id": "7", "create_time": "1700000006", "local_type": "1", "message_content": "4:1", "sender_username": "wxid_bob"}),
        ];
        let msgs = collect_messages(
            &rows,
            &CollectOptions {
                session_id: "room@chatroom",
                my_wxid: "wxid_me",
                start: None,
                end: None,
                sender_filter: None,
            },
        );
        let mut names = NameBook::from_map(HashMap::new());
        let mut ex = Exporter {
            session: SessionInfo {
                id: "room@chatroom".into(),
                display_name: "Room".into(),
                nickname: "Room".into(),
                remark: String::new(),
                is_group: true,
            },
            my_wxid: "wxid_me".into(),
            raw_my_wxid: "wxid_me".into(),
            my_display: "Me Nick".into(),
            group_nicks: HashMap::new(),
            group_members: Vec::new(),
            names: &mut names,
            settings: Settings::default(),
        };
        let t: Vec<String> = msgs.iter().map(|m| ex.message_text(m)).collect();
        assert_eq!(t[0], "[表情]");
        assert_eq!(t[1], "[语音消息]");
        assert!(
            t[2] == "[系统: \"Bob\" 撤回了一条消息]" || t[2] == "[System: \"Bob\" 撤回了一条消息]",
            "{}",
            t[2]
        );
        assert_eq!(t[3], "[图片]");
        assert_eq!(t[4], "[表情]");
        assert_eq!(t[5], "好看[引用 Me Nick：[图片]]");
        assert_eq!(t[6], "4:1");
    }

    /// 各格式共用同一套文本：SQL / Excel 行 / 导出器 txt 与 JSON 一致；ChatLab 链接用 Markdown、
    /// 系统消息不再包裹；WeClone 丢弃引用消息。
    #[test]
    fn formats_share_message_text() {
        let rows = vec![
            json!({"local_id": "1", "create_time": "1700000000", "local_type": "10000", "message_content": "\"Bob\" 撤回了一条消息"}),
            json!({"local_id": "2", "create_time": "1700000001", "local_type": "34", "message_content": "", "sender_username": "wxid_bob"}),
            json!({"local_id": "3", "create_time": "1700000002", "local_type": "49", "message_content": "wxid_bob:\n<msg><appmsg><title>Doc</title><type>5</type><url>https://example.com/x</url></appmsg></msg>", "sender_username": "wxid_bob"}),
            json!({"local_id": "4", "create_time": "1700000003", "local_type": "103079215153", "message_content": "wxid_bob:\n<msg><appmsg><title /><des>预览</des><type>24</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><datadesc>笔记&#x0A;全文</datadesc></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>", "sender_username": "wxid_bob"}),
            json!({"local_id": "5", "create_time": "1700000004", "local_type": "244813135921", "message_content": "wxid_bob:\n<msg><appmsg><title>好的</title><type>57</type><refermsg><type>1</type><displayname>Me</displayname><content>hello</content><svrid>1</svrid></refermsg></appmsg></msg>", "sender_username": "wxid_bob"}),
        ];
        let mut msgs = collect_messages(
            &rows,
            &CollectOptions {
                session_id: "wxid_bob",
                my_wxid: "wxid_me",
                start: None,
                end: None,
                sender_filter: None,
            },
        );
        msgs.push({
            let mut m = msgs[1].clone();
            m.local_type = 47;
            m.emoji_caption = Some("打call".into());
            m
        });
        let mut names = NameBook::from_map(HashMap::new());
        let mut ex = Exporter {
            session: SessionInfo {
                id: "wxid_bob".into(),
                display_name: "Bob".into(),
                nickname: "Bob".into(),
                remark: String::new(),
                is_group: false,
            },
            my_wxid: "wxid_me".into(),
            raw_my_wxid: "wxid_me".into(),
            my_display: "Me".into(),
            group_nicks: HashMap::new(),
            group_members: Vec::new(),
            names: &mut names,
            settings: Settings::default(),
        };
        let sys = |t: &str| format!("[{}: {t}]", chrome("系统", "System"));
        // 导出器 txt / SQL（plain_row_content）与 JSON（message_text）一致
        for m in &msgs {
            assert_eq!(ex.plain_row_content(m).0, ex.message_text(m));
        }
        let sql = ex.render_sql(&msgs);
        for want in [
            sys("\"Bob\" 撤回了一条消息"),
            "[语音消息]".into(),
            "[链接] Doc".into(),
            "https://example.com/x".into(),
            "[笔记]\n笔记\n全文".into(),
            "好的[引用 Me：hello]".into(),
            "[表情：打call]".into(),
        ] {
            assert!(
                sql.contains(&want.replace('\'', "''")),
                "sql: {want}\n{sql}"
            );
        }
        // ChatLab：链接 Markdown，系统消息不包裹
        let chatlab = ex.build_chatlab(&msgs).to_string();
        assert!(
            chatlab.contains("[Doc](https://example.com/x)"),
            "{chatlab}"
        );
        assert!(chatlab.contains("\\\"Bob\\\" 撤回了一条消息"), "{chatlab}");
        assert!(!chatlab.contains(&sys("\"Bob\"")), "{chatlab}");
        // WeClone：引用消息不进训练数据，其余同样的写法
        let weclone = ex.render_weclone(&msgs);
        assert!(!weclone.contains("好的"), "{weclone}");
        assert!(weclone.contains("[语音消息]"), "{weclone}");
    }

    /// 常见消息类型的快速路径与完整流程结果一致。
    #[test]
    fn plain_message_text_matches_the_full_path() {
        let samples: Vec<(i64, &str, Option<&str>)> = vec![
            (1, "hello", None),
            (1, "wxid_bob:\nhi", None),
            (1, "1:3-1:6", None),
            (1, "", None),
            (1, "  带空格  ", None),
            (1, "[转账] 看起来像转账的文字", None),
            (1, "<凡人修仙传>看了100多集", None),
            (3, "wxid_bob:\n<msg><img md5=\"x\"/></msg>", None),
            (3, "", None),
            (34, "wxid_bob:\n<msg><voicemsg length=\"1\"/></msg>", None),
            (34, "", None),
            (
                43,
                "wxid_bob:\n<msg><videomsg playlength=\"3\"/></msg>",
                None,
            ),
            (43, "", None),
            (47, "wxid_bob:\n<msg><emoji md5=\"a\"/></msg>", None),
            (47, "", None),
            (
                47,
                "wxid_bob:\n<msg><emoji md5=\"a\"/></msg>",
                Some("打call"),
            ),
        ];
        let mut names = NameBook::from_map(HashMap::new());
        let mut ex = Exporter {
            session: SessionInfo {
                id: "room@chatroom".into(),
                display_name: "Room".into(),
                nickname: "Room".into(),
                remark: String::new(),
                is_group: true,
            },
            my_wxid: "wxid_me".into(),
            raw_my_wxid: "wxid_me".into(),
            my_display: "Me".into(),
            group_nicks: HashMap::new(),
            group_members: Vec::new(),
            names: &mut names,
            settings: Settings::default(),
        };
        for (ty, content, caption) in samples {
            let msg = ExportMsg {
                local_type: ty,
                content: content.into(),
                sender_username: "wxid_bob".into(),
                emoji_caption: caption.map(str::to_string),
                ..Default::default()
            };
            let fast = plain_message_text(&msg)
                .unwrap_or_else(|| panic!("no fast path: {ty} {content:?}"));
            // 完整流程：内容里夹一个 appmsg 字样会关掉快速路径，所以改走 message_text_generic
            let full = ex.message_text_generic(&msg, Some(LinkStyle::AppendUrl), true);
            assert_eq!(fast, full, "type {ty}, content {content:?}");
        }
        // 含 appmsg / sysmsg / refermsg 的内容不走快速路径
        for k in ["appmsg", "sysmsg", "refermsg"] {
            let msg = ExportMsg {
                local_type: 1,
                content: format!("<{k}>"),
                ..Default::default()
            };
            assert!(plain_message_text(&msg).is_none(), "{k}");
        }
    }

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
        let dir = std::env::temp_dir().join(format!("weflow-jsonl-{}", std::process::id()));
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
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn detailed_json_and_arkme() {
        let v = with_exporter(false, |ex, msgs| ex.build_json(msgs, false));
        assert_eq!(v["weflow"]["generator"], "WeFlow");
        assert_eq!(v["session"]["type"], "私聊");
        assert_eq!(v["session"]["messageCount"], 5);
        let m = v["messages"].as_array().unwrap();
        assert_eq!(m[0]["formattedTime"].as_str().unwrap().len(), 19);
        assert_eq!(m[0]["type"], "文本消息");
        assert_eq!(m[3]["appMsgKind"], "link");
        assert_eq!(m[3]["linkUrl"], "https://example.com/x");
        assert_eq!(m[4]["quotedSender"], "Me");

        let a = with_exporter(true, |ex, msgs| ex.build_json(msgs, true));
        assert_eq!(a["weflow"]["format"], "arkme-json");
        let senders = a["senders"].as_array().unwrap();
        assert_eq!(senders[0]["senderID"], 1);
        assert!(a["messages"][0].get("senderID").is_some());
        assert!(a["messages"][0].get("senderUsername").is_none());
        assert!(a["groupMembers"].as_array().unwrap().len() >= 2);
    }

    /// The JSONL a whole `build_chatlab` value turns into (header, members, messages).
    fn chatlab_jsonl_reference(export: &Value) -> String {
        let mut lines = vec![serde_json::to_string(
            &json!({ "_type": "header", "chatlab": export["chatlab"], "meta": export["meta"] }),
        )
        .unwrap()];
        for (key, ty) in [("members", "member"), ("messages", "message")] {
            for item in export[key].as_array().into_iter().flatten() {
                let mut obj = Map::new();
                obj.insert("_type".into(), json!(ty));
                obj.extend(item.as_object().unwrap().clone());
                lines.push(serde_json::to_string(&Value::Object(obj)).unwrap());
            }
        }
        lines.join("\n")
    }

    #[test]
    fn streamed_exports_are_byte_identical_to_serializing_the_whole_value() {
        let dir = std::env::temp_dir().join(format!("weflow-json-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let read = |f: &Path| std::fs::read_to_string(f).unwrap();
        for group in [false, true] {
            let file = dir.join(format!("g{group}.json"));
            let (detailed, arkme) = (
                file.with_extension("detailed"),
                file.with_extension("arkme"),
            );
            let (lab, jsonl) = (file.with_extension("lab"), file.with_extension("jsonl"));
            with_exporter(group, |ex, msgs| {
                ex.write_json(msgs, &detailed, false).unwrap();
                ex.write_json(msgs, &arkme, true).unwrap();
                ex.write_chatlab(msgs, &lab, false).unwrap();
                ex.write_chatlab(msgs, &jsonl, true).unwrap();
            });
            assert_eq!(
                read(&detailed),
                with_exporter(group, |ex, msgs| serde_json::to_string_pretty(
                    &ex.build_json(msgs, false)
                )
                .unwrap()),
                "detailed, group={group}"
            );
            assert_eq!(
                read(&arkme),
                with_exporter(group, |ex, msgs| serde_json::to_string_pretty(
                    &ex.build_json(msgs, true)
                )
                .unwrap()),
                "arkme, group={group}"
            );
            assert_eq!(
                read(&lab),
                with_exporter(group, |ex, msgs| serde_json::to_string_pretty(
                    &ex.build_chatlab(msgs)
                )
                .unwrap()),
                "chatlab, group={group}"
            );
            assert_eq!(
                read(&jsonl),
                with_exporter(group, |ex, msgs| chatlab_jsonl_reference(
                    &ex.build_chatlab(msgs)
                )),
                "chatlab-jsonl, group={group}"
            );
        }
        // no messages: empty arrays print like serde_json does
        let (empty_json, empty_ark, empty_lab) = (
            dir.join("e.detailed"),
            dir.join("e.arkme"),
            dir.join("e.lab"),
        );
        with_exporter(false, |ex, _| {
            ex.write_json(&[], &empty_json, false).unwrap();
            ex.write_json(&[], &empty_ark, true).unwrap();
            ex.write_chatlab(&[], &empty_lab, false).unwrap();
        });
        assert_eq!(
            read(&empty_json),
            with_exporter(false, |ex, _| serde_json::to_string_pretty(
                &ex.build_json(&[], false)
            )
            .unwrap())
        );
        assert_eq!(
            read(&empty_ark),
            with_exporter(false, |ex, _| serde_json::to_string_pretty(
                &ex.build_json(&[], true)
            )
            .unwrap())
        );
        assert_eq!(
            read(&empty_lab),
            with_exporter(false, |ex, _| serde_json::to_string_pretty(
                &ex.build_chatlab(&[])
            )
            .unwrap())
        );
        let _ = std::fs::remove_dir_all(dir);
    }

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

    #[test]
    fn html_contains_data() {
        let html = with_exporter(false, |ex, msgs| ex.render_html(msgs));
        assert!(html.contains("window.WEFLOW_DATA = ["));
        assert!(html.contains("&lt;b&gt;there&lt;/b&gt;"));
        assert!(html.contains("message-link-card"));
        assert!(!html.contains("<b>there</b>"));
        assert!(html.contains("<title>Bobby - "));
    }

    #[test]
    fn sql_inserts() {
        let sql = with_exporter(false, |ex, msgs| ex.render_sql(msgs));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"messages\""));
        assert_eq!(sql.matches("INSERT INTO").count(), 5);
        assert!(sql.contains("hello \"world\", ok"));
    }

    #[test]
    fn excel_writes_file() {
        let dir = std::env::temp_dir().join(format!("weflow-xlsx-{}", std::process::id()));
        let out = dir.join("chat.xlsx");
        fs::create_dir_all(&dir).unwrap();
        with_exporter(true, |ex, msgs| ex.write_excel(msgs, &out).unwrap());
        let bytes = fs::read(&out).unwrap();
        assert_eq!(&bytes[..2], b"PK");
        assert!(bytes.len() > 2000);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn group_nickname_rules() {
        let map = build_trusted_group_nicknames(vec![
            ("WXID_A".into(), "Alpha".into()),
            ("wxid_a".into(), "Alpha".into()),
            ("wxid_b".into(), "X".into()),
            ("wxid_b".into(), "Y".into()),
            ("wxid_c".into(), "，，".into()),
        ]);
        assert_eq!(resolve_group_nickname(&map, &["wxid_a"]), "Alpha");
        assert_eq!(resolve_group_nickname(&map, &["wxid_b"]), "");
        assert_eq!(resolve_group_nickname(&map, &["wxid_c"]), "");
        assert_eq!(DisplayPref::GroupNickname.pick("w", "n", "r", "g"), "g");
        assert_eq!(DisplayPref::Remark.pick("w", "n", "", "g"), "n");
        assert_eq!(DisplayPref::parse("nickname"), Some(DisplayPref::Nickname));
    }

    #[test]
    fn transfer_description_uses_names() {
        let (mut msgs, contacts) = fixture();
        msgs.push(ExportMsg {
            local_id: 9,
            create_time: 1_700_000_300,
            local_type: 49,
            content: "<msg><appmsg><title>转账</title><type>2000</type><wcpayinfo><feedesc>￥2.00</feedesc><paysubtype>1</paysubtype><payer_username>wxid_me</payer_username><receiver_username>wxid_bob</receiver_username></wcpayinfo></appmsg></msg>".into(),
            sender_username: "wxid_me".into(),
            is_send: true,
            ..Default::default()
        });
        let mut names = NameBook::from_map(contacts);
        let mut ex = Exporter {
            session: SessionInfo {
                id: "wxid_bob".into(),
                display_name: "Bobby".into(),
                nickname: "Bob".into(),
                remark: "Bobby".into(),
                is_group: false,
            },
            my_wxid: "wxid_me".into(),
            raw_my_wxid: "wxid_me".into(),
            my_display: "Me Nick".into(),
            group_nicks: HashMap::new(),
            group_members: Vec::new(),
            names: &mut names,
            settings: Settings::default(),
        };
        let txt = ex.render_txt(&msgs);
        assert!(
            txt.contains("[转账] (") && txt.contains("转账给 Bobby"),
            "{txt}"
        );
    }
}
