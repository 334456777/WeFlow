//! Chat-message exporters ported from WeFlow's desktop app: ChatLab (+JSONL), detailed JSON,
//! Arkme JSON, Excel, TXT, WeClone CSV and HTML.
//!
//! Everything here works on already-collected [`ExportMsg`] values plus a name book for
//! contacts, so it is testable without a WeChat database. Data values (message text,
//! type names, JSON keys) follow WeFlow's desktop app; only human-facing chrome such as
//! spreadsheet headers and the HTML page follows the CLI language.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::Path;

use anyhow::{Context, Result};
use serde::ser::{SerializeMap, Serializer};
use serde::Serialize;
use serde_json::ser::Formatter;
use serde_json::{json, Map, Value};

use crate::locale;
use crate::message::*;

const HTML_STYLES: &str = include_str!("export_html.css");

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

    #[cfg(test)]
    fn from_map(map: HashMap<String, ContactInfo>) -> NameBook<'static> {
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

/// The senders of an export, in order of first appearance, and how many messages each sent (by lower-case id):
/// what the member lists need, collected message by message.
#[derive(Default)]
struct SenderTally {
    order: Vec<String>,
    seen: HashSet<String>,
    counts: HashMap<String, i64>,
}

impl SenderTally {
    fn add(&mut self, sender: &str) {
        if !sender.is_empty() && self.seen.insert(sender.to_string()) {
            self.order.push(sender.to_string());
        }
        *self.counts.entry(sender.to_lowercase()).or_insert(0) += 1;
    }
}

/// Entries of an export that are written to a file next to the output while the messages are read, then copied
/// into the output between a header and a footer that need the totals. Removed when dropped.
struct Spool {
    path: std::path::PathBuf,
    writer: Option<std::io::BufWriter<std::fs::File>>,
    entries: usize,
}

impl Spool {
    fn create(out: &Path) -> Result<Self> {
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let mut name = out.as_os_str().to_os_string();
        name.push(".part");
        let path = std::path::PathBuf::from(name);
        let file = std::fs::File::create(&path)?;
        Ok(Self {
            path,
            writer: Some(std::io::BufWriter::with_capacity(1 << 20, file)),
            entries: 0,
        })
    }

    /// Appends one entry, which `write` writes, preceded by `separator` unless it is the first.
    fn push_with(
        &mut self,
        separator: &str,
        write: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> io::Result<()>,
    ) -> Result<()> {
        use std::io::Write;
        if let Some(w) = self.writer.as_mut() {
            if self.entries > 0 {
                w.write_all(separator.as_bytes())?;
            }
            write(w)?;
        }
        self.entries += 1;
        Ok(())
    }

    /// Appends one entry, preceded by `separator` unless it is the first.
    fn push(&mut self, separator: &str, entry: &[u8]) -> Result<()> {
        use std::io::Write;
        self.push_with(separator, |w| w.write_all(entry))
    }

    /// Appends `entry` pretty-printed as one JSON value nested `indent` spaces deep (see [`write_pretty`]),
    /// preceded by `separator` unless it is the first.
    fn push_json(&mut self, separator: &str, entry: &impl Serialize, indent: usize) -> Result<()> {
        self.push_with(separator, |w| write_pretty(w, entry, indent))
    }

    /// Copies everything written so far to `out`.
    fn copy_to(&mut self, out: &mut impl std::io::Write) -> Result<()> {
        use std::io::Write;
        if let Some(mut w) = self.writer.take() {
            w.flush()?;
        }
        std::io::copy(&mut std::fs::File::open(&self.path)?, out)?;
        Ok(())
    }
}

impl Drop for Spool {
    fn drop(&mut self) {
        self.writer = None;
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Formats whose entries do not depend on each other, so a page of them can be rendered on another thread ahead of
/// the writer: [`Exporter::render_entries`] renders, [`Exporter::write_rendered`] writes them in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryFormat {
    Txt,
    Sql,
    Chatlab,
    ChatlabJsonl,
    /// Detailed JSON. Not arkme-json: its sender table numbers the senders in the order the messages show them.
    Json,
}

impl EntryFormat {
    /// Whether rendering is slow enough to be worth doing on the reading threads. A ChatLab or detailed-JSON entry
    /// resolves names and prints a JSON object, which keeps the writer behind the readers; TXT and SQL entries are
    /// cheap enough that the writer keeps up, and rendering them elsewhere only holds more text in flight (measured
    /// in issue #14).
    pub fn renders_ahead(self) -> bool {
        matches!(self, Self::Chatlab | Self::ChatlabJsonl | Self::Json)
    }

    pub fn parse(format: &str) -> Option<Self> {
        match format.to_ascii_lowercase().as_str() {
            "txt" => Some(Self::Txt),
            "sql" => Some(Self::Sql),
            "chatlab" => Some(Self::Chatlab),
            "chatlab-jsonl" => Some(Self::ChatlabJsonl),
            "json" => Some(Self::Json),
            _ => None,
        }
    }
}

/// A page of messages rendered ahead of writing (see [`Exporter::render_entries`]).
#[derive(Debug, Default)]
pub struct RenderedEntries {
    /// The entries of the page, one after another, so a page takes a few allocations rather than one or more per
    /// message.
    text: Vec<u8>,
    /// Where each entry ends in `text`, one per message, in order.
    ends: Vec<usize>,
    /// ChatLab: the sender of each message (the member list follows their first appearance).
    pub senders: Vec<String>,
    /// ChatLab: the people named inside forwarded chat histories, in order (they become members too).
    pub record_names: Vec<String>,
    /// Detailed JSON: the newest `create_time` of the page (the session header has the newest of all).
    pub latest: Option<i64>,
}

impl RenderedEntries {
    /// The entries, in order.
    fn entries(&self) -> impl Iterator<Item = &[u8]> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts
            .zip(&self.ends)
            .map(|(start, &end)| &self.text[start..end])
    }

    /// Ends the entry written to `text` since the previous one.
    fn end_entry(&mut self) {
        self.ends.push(self.text.len());
    }
}

/// The key of a detailed-JSON message's first field, whose value the writer sets (see
/// [`Exporter::render_entries`]).
const JSON_LOCAL_ID_KEY: &[u8] = b"\"localId\": ";

/// Writes `entry`, a detailed-JSON message rendered ahead, with `local_id` as the value of its `localId`.
fn write_json_entry(w: &mut impl io::Write, entry: &[u8], local_id: usize) -> io::Result<()> {
    // `localId` is the first field, so the first match is the key and not text inside a later value
    let Some(key) = memchr::memmem::find(entry, JSON_LOCAL_ID_KEY) else {
        return w.write_all(entry);
    };
    let start = key + JSON_LOCAL_ID_KEY.len();
    let digits = entry[start..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count();
    w.write_all(&entry[..start])?;
    write!(w, "{local_id}")?;
    w.write_all(&entry[start + digits..])
}

/// Messages rendered at a time when an export is rendered and written on one thread.
const RENDER_CHUNK: usize = 2000;

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

struct SenderFields {
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

    fn sender_fields(&mut self, msg: &ExportMsg) -> SenderFields {
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

    fn quoted_with_names(&mut self, local_type: i64, x: &MsgXml) -> Option<QuotedDisplay> {
        if !is_text_quote_message_in(local_type, x) {
            return None;
        }
        let base = x.quoted_display()?.clone();
        if base.quoted_sender.as_deref().is_some_and(|s| !s.is_empty()) {
            return Some(base);
        }
        let Some(user) = x.quoted_sender_username() else {
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
    fn message_text(&mut self, msg: &ExportMsg) -> String {
        self.message_text_in(msg, &MsgXml::new(&msg.content))
    }

    /// [`message_text`](Self::message_text) given the message's [`MsgXml`].
    fn message_text_in(&mut self, msg: &ExportMsg, x: &MsgXml) -> String {
        self.message_text_with(msg, x, Some(LinkStyle::AppendUrl), true)
    }

    /// [`message_text`](Self::message_text) 的可调版本：`link` 为链接卡片的写法（`None` 保持解析出的
    /// `[链接] 标题`，用于 HTML 这类另有链接卡片的格式），`wrap_system` 控制系统消息是否包成 `[系统: …]`
    /// （ChatLab 有独立的 system 类型、HTML 有专门的样式，不再包一层）。
    fn message_text_with(
        &mut self,
        msg: &ExportMsg,
        x: &MsgXml,
        link: Option<LinkStyle>,
        wrap_system: bool,
    ) -> String {
        if let Some(text) = plain_message_text(msg) {
            return text;
        }
        self.message_text_generic(msg, x, link, wrap_system)
    }

    /// 完整流程（不走快速路径）。
    fn message_text_generic(
        &mut self,
        msg: &ExportMsg,
        x: &MsgXml,
        link: Option<LinkStyle>,
        wrap_system: bool,
    ) -> String {
        let my = self.my_wxid.clone();
        let mut content = if msg.local_type == 3 {
            Some("[图片]".to_string())
        } else {
            parse_message_content_in(
                x,
                msg.local_type,
                Some(&my),
                Some(&msg.sender_username),
                msg.emoji_caption.as_deref(),
            )
        };
        if x.readable_system_once(msg.local_type) {
            let t = extract_readable_system_message_text(&msg.content);
            if !t.is_empty() {
                content = Some(t);
            }
        }
        let quoted = self.quoted_with_names(msg.local_type, x);
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
    fn plain_row_content(&mut self, msg: &ExportMsg, x: &MsgXml) -> (String, bool) {
        if let Some(m) = msg.media.as_ref().filter(|_| msg.local_type != 47) {
            return (m.relative_path.clone(), false);
        }
        let has_quote = is_text_quote_message_in(msg.local_type, x);
        (self.message_text_in(msg, x), has_quote)
    }

    #[cfg(test)]
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
    fn build_chatlab(&mut self, msgs: &[ExportMsg]) -> Value {
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
    fn chatlab_message<'m>(
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

    /// Renders `msgs` a chunk at a time on this thread and writes them (see [`write_rendered`](Self::write_rendered)).
    fn render_and_write<I>(
        &mut self,
        format: EntryFormat,
        out: &Path,
        msgs: I,
        write_if_empty: bool,
    ) -> Result<usize>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<ExportMsg>,
    {
        let mut msgs = msgs.into_iter();
        self.write_rendered(
            format,
            out,
            &mut |ex: &mut Self| {
                let chunk: Vec<I::Item> = msgs.by_ref().take(RENDER_CHUNK).collect();
                (!chunk.is_empty()).then(|| ex.render_entries(format, &chunk))
            },
            write_if_empty,
        )
    }

    /// The entries of `msgs` in `format`, without writing them: what a worker thread prepares for
    /// [`write_rendered`](Self::write_rendered).
    pub fn render_entries<M: std::borrow::Borrow<ExportMsg>>(
        &mut self,
        format: EntryFormat,
        msgs: &[M],
    ) -> RenderedEntries {
        let mut out = RenderedEntries {
            ends: Vec::with_capacity(msgs.len()),
            ..Default::default()
        };
        // detailed JSON keeps no sender profiles; json_message wants somewhere to put them
        let mut profiles: HashMap<String, Profile> = HashMap::new();
        for msg in msgs.iter().map(std::borrow::Borrow::borrow) {
            match format {
                EntryFormat::Txt => self.write_txt_entry(msg, &mut out.text),
                EntryFormat::Sql => self.write_sql_insert(msg, &mut out.text),
                EntryFormat::Json => {
                    out.latest = out.latest.max(Some(msg.create_time));
                    // the message's place in the whole export is known only when it is written, which sets
                    // `localId` (see write_json_entry)
                    let m = self.json_message(msg, false, 0, &mut profiles);
                    // a Vec takes every write, and the documents here have only string keys
                    let _ = write_pretty(&mut out.text, &m, 4);
                }
                EntryFormat::Chatlab | EntryFormat::ChatlabJsonl => {
                    out.senders.push(msg.sender_username.clone());
                    let member_name = if msg.sender_username.is_empty() {
                        String::new()
                    } else {
                        self.contact_display(&msg.sender_username)
                    };
                    let m = self.chatlab_message(msg, &member_name, &mut out.record_names);
                    if format == EntryFormat::ChatlabJsonl {
                        let start = out.text.len();
                        let line = Tagged {
                            kind: "message",
                            entry: &m,
                        };
                        // a line that cannot be serialized is left empty
                        if serde_json::to_writer(&mut out.text, &line).is_err() {
                            out.text.truncate(start);
                        }
                    } else {
                        let _ = write_pretty(&mut out.text, &m, 4);
                    }
                }
            }
            out.end_entry();
        }
        out
    }

    /// Writes the pages `next` returns (in order, until `None`) as one export; `next` gets this exporter, so it can
    /// render the page itself or hand over one rendered elsewhere. Returns the number of entries; with none, no file
    /// is written unless `write_if_empty`.
    pub fn write_rendered(
        &mut self,
        format: EntryFormat,
        out: &Path,
        next: &mut dyn FnMut(&mut Self) -> Option<RenderedEntries>,
        write_if_empty: bool,
    ) -> Result<usize> {
        match format {
            EntryFormat::Txt | EntryFormat::Sql => {
                self.write_rendered_entries(format, out, next, write_if_empty)
            }
            EntryFormat::Chatlab | EntryFormat::ChatlabJsonl => self.write_rendered_chatlab(
                out,
                format == EntryFormat::ChatlabJsonl,
                next,
                write_if_empty,
            ),
            EntryFormat::Json => self.write_rendered_json(out, next, write_if_empty),
        }
    }

    /// Detailed JSON: the entries go to a spool file, numbered as they are written; the session header, which comes
    /// first in the file and has the totals, is written at the end.
    fn write_rendered_json(
        &mut self,
        out: &Path,
        next: &mut dyn FnMut(&mut Self) -> Option<RenderedEntries>,
        write_if_empty: bool,
    ) -> Result<usize> {
        let mut spool = Spool::create(out)?;
        let mut last_ts: Option<i64> = None;
        let mut count = 0usize;
        while let Some(page) = next(self) {
            last_ts = last_ts.max(page.latest);
            for text in page.entries() {
                count += 1;
                spool.push_with(",\n    ", |w| write_json_entry(w, text, count))?;
            }
        }
        if count == 0 && !write_if_empty {
            return Ok(0);
        }
        self.write_json_file(out, &mut spool, count, last_ts, None)?;
        Ok(count)
    }

    /// TXT / SQL: a header, then the entries.
    fn write_rendered_entries(
        &mut self,
        format: EntryFormat,
        out: &Path,
        next: &mut dyn FnMut(&mut Self) -> Option<RenderedEntries>,
        write_if_empty: bool,
    ) -> Result<usize> {
        use std::io::Write;
        let header = if format == EntryFormat::Sql {
            Self::SQL_HEADER
        } else {
            ""
        };
        let open = || -> Result<std::io::BufWriter<std::fs::File>> {
            if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
                std::fs::create_dir_all(parent)?;
            }
            let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
            w.write_all(header.as_bytes())?;
            Ok(w)
        };
        let mut count = 0usize;
        // created with the first entry, so an empty range leaves no file behind
        let mut writer: Option<std::io::BufWriter<std::fs::File>> = None;
        while let Some(page) = next(self) {
            for text in page.entries() {
                if writer.is_none() {
                    writer = Some(open()?);
                }
                if let Some(w) = writer.as_mut() {
                    w.write_all(text)?;
                }
                count += 1;
            }
        }
        if writer.is_none() && write_if_empty {
            writer = Some(open()?);
        }
        if let Some(mut w) = writer {
            w.flush()?;
        }
        Ok(count)
    }

    /// ChatLab: the entries go to a spool file; the member list, which comes first in the file and needs every
    /// sender, is written at the end.
    fn write_rendered_chatlab(
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

    // ─────────────────────────── detailed JSON / arkme-json ───────────────────────────

    fn weflow_header(&self, format: Option<&'static str>) -> WeflowHeader {
        WeflowHeader {
            version: "1.0.3",
            exported_at: self.settings.exported_at,
            generator: "WeFlow",
            format,
        }
    }

    /// The detailed-JSON object of one message; `index` is the number of messages before it. Records the sender's
    /// profile in `profiles` (the first one seen wins).
    fn json_message<'m>(
        &mut self,
        msg: &'m ExportMsg,
        arkme: bool,
        index: usize,
        profiles: &mut HashMap<String, Profile>,
    ) -> JsonMessage<'m> {
        let source = crate::xml::tag_element(&msg.content, "msgsource").unwrap_or("");
        let x = MsgXml::new(&msg.content);
        let content = match msg.media.as_ref().filter(|_| msg.local_type != 47) {
            Some(m) => m.relative_path.clone(),
            None => self.message_text_in(msg, &x),
        };
        let quoted = self.quoted_with_names(msg.local_type, &x);
        let sender = msg.sender_username.as_str();
        let contact = self.names.get(sender);
        let nickname = contact
            .as_ref()
            .map(|c| c.nickname.clone())
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| {
                contact
                    .as_ref()
                    .map_or_else(|| sender.to_string(), ContactInfo::display_name)
            });
        let remark = contact.map(|c| c.remark).unwrap_or_default();
        let group_nick = resolve_group_nickname(&self.group_nicks, &[sender]);
        let display = self
            .settings
            .display_pref
            .pick(sender, &nickname, &remark, &group_nick);
        if !profiles.contains_key(sender) {
            profiles.insert(
                sender.to_string(),
                Profile {
                    wxid: sender.to_string(),
                    nickname,
                    remark,
                    group_nickname: group_nick,
                    display_name: display.clone(),
                    ..Default::default()
                },
            );
        }

        // later keys replace the value of an earlier one in place (the quoted sender and text of a quote)
        let mut extra = Map::new();
        // detailed JSON keeps the app-message fields of quotes and links only
        let meta = if arkme {
            extract_arkme_app_message_meta_in(&x, msg.local_type)
        } else {
            quote_or_link_app_message_meta_in(&x, msg.local_type)
        };
        if let Some(meta) = meta {
            extra.extend(meta);
        }
        if let Some(q) = &quoted {
            if let Some(s) = q.quoted_sender.as_ref().filter(|s| !s.is_empty()) {
                extra.insert("quotedSender".into(), json!(s));
            }
            if !q.quoted_preview.is_empty() {
                extra.insert("quotedContent".into(), json!(q.quoted_preview));
            }
        }
        if arkme {
            if let Some(meta) = extract_arkme_contact_card_meta(&msg.content, msg.local_type) {
                extra.extend(meta);
            }
        }

        let (emoji, image, video, location) = (
            msg.local_type == 47,
            msg.local_type == 3,
            msg.local_type == 43,
            msg.local_type == 48,
        );
        JsonMessage {
            local_id: index + 1,
            create_time: msg.create_time,
            formatted_time: format_timestamp(msg.create_time),
            kind: message_type_name_in(msg.local_type, &x),
            local_type: msg.local_type,
            content,
            is_send: u8::from(msg.is_send),
            sender_username: sender,
            sender_display_name: display,
            source,
            sender_avatar_key: sender,
            emoji_md5: msg.emoji_md5.as_deref().filter(|_| emoji),
            emoji_cdn_url: msg.emoji_cdn_url.as_deref().filter(|_| emoji),
            emoji_caption: msg.emoji_caption.as_deref().filter(|_| emoji),
            image_md5: msg.image_md5.as_deref().filter(|_| image),
            image_dat_name: msg.image_dat_name.as_deref().filter(|_| image),
            video_md5: msg.video_md5.as_deref().filter(|_| video),
            platform_message_id: msg.platform_message_id(),
            reply_to_message_id: x.reply_to_message_id(),
            extra,
            location_lat: msg.location_lat.filter(|_| location),
            location_lng: msg.location_lng.filter(|_| location),
            location_poiname: msg.location_poiname.as_deref().filter(|_| location),
            location_label: msg.location_label.as_deref().filter(|_| location),
        }
    }

    /// `{ wxid, nickname, remark, displayName, type, lastTimestamp, messageCount }` of the exported session.
    fn json_session(&mut self, count: usize, last_ts: Option<i64>) -> JsonSession {
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
        JsonSession {
            wxid: self.session.id.clone(),
            nickname: session_nickname,
            remark: session_remark,
            display_name: session_display,
            kind: if is_group { "群聊" } else { "私聊" },
            last_timestamp: last_ts,
            message_count: count,
        }
    }

    /// The whole detailed-JSON (or arkme-json) document, with the messages sorted by time.
    #[cfg(test)]
    fn json_document<'m>(&mut self, msgs: &'m [ExportMsg], arkme: bool) -> JsonDocument<'m> {
        let mut profiles: HashMap<String, Profile> = HashMap::new();
        let mut messages: Vec<JsonMessage<'m>> = Vec::with_capacity(msgs.len());
        for (count, msg) in msgs.iter().enumerate() {
            messages.push(self.json_message(msg, arkme, count, &mut profiles));
        }
        messages.sort_by_key(|m| m.create_time);
        let last_ts = messages.iter().map(|m| m.create_time).max();
        let session = self.json_session(messages.len(), last_ts);
        if !arkme {
            return JsonDocument {
                weflow: self.weflow_header(None),
                session,
                senders: None,
                messages: JsonMessages::Detailed(messages),
                group_members: None,
            };
        }

        // arkme-json: compact messages + sender table (+ group members)
        let mut ark = ArkCompact::default();
        let compact: Vec<ArkMessage<'m>> = messages
            .into_iter()
            .map(|m| ark.compact(m, &|s| profiles.get(s).cloned()))
            .collect();
        let group_members = self.session.is_group.then(|| self.ark_group_members(msgs));
        JsonDocument {
            weflow: self.weflow_header(Some("arkme-json")),
            session,
            senders: Some(ark.senders),
            messages: JsonMessages::Arkme(compact),
            group_members,
        }
    }

    /// The whole document, built in memory: what the tests hold the streamed file against.
    #[cfg(test)]
    fn build_json(&mut self, msgs: &[ExportMsg], arkme: bool) -> Value {
        serde_json::to_value(self.json_document(msgs, arkme)).unwrap_or_default()
    }

    /// `groupMembers` of the arkme export: every sender plus the group's member list, busiest first.
    #[cfg(test)]
    fn ark_group_members(&mut self, msgs: &[ExportMsg]) -> Vec<ArkGroupMember> {
        let mut tally = SenderTally::default();
        for m in msgs {
            tally.add(&m.sender_username);
        }
        self.ark_group_members_of(tally)
    }

    /// [`ark_group_members`](Self::ark_group_members) from a tally of the senders (see [`SenderTally`]).
    fn ark_group_members_of(&mut self, tally: SenderTally) -> Vec<ArkGroupMember> {
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

    pub fn write_json(&mut self, msgs: &[ExportMsg], out: &Path, arkme: bool) -> Result<()> {
        // written message by message in the order given, as a streaming export writes them
        if arkme {
            self.stream_arkme_json(msgs, out, true)?;
        } else {
            self.render_and_write(EntryFormat::Json, out, msgs, true)?;
        }
        Ok(())
    }

    /// arkme-json written from a stream of messages (oldest first). Each message is serialized as soon as it is
    /// read and put in a spool file; the header (which has the totals) and the sender table and member list (which
    /// have the senders) are written around it at the end. With no messages nothing is written unless
    /// `write_if_empty`.
    fn stream_arkme_json<I>(&mut self, msgs: I, out: &Path, write_if_empty: bool) -> Result<usize>
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

    /// Writes the detailed-JSON file, or with `arkme` (its sender table and a tally of the senders) the arkme-json
    /// file, around the `count` messages in `spool`.
    fn write_json_file(
        &mut self,
        out: &Path,
        spool: &mut Spool,
        count: usize,
        last_ts: Option<i64>,
        arkme: Option<(&[ArkSender], SenderTally)>,
    ) -> Result<()> {
        use std::io::Write;
        let session = self.json_session(count, last_ts);
        let header = self.weflow_header(arkme.is_some().then_some("arkme-json"));
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
        w.write_all(b"{\n  \"weflow\": ")?;
        write_pretty(&mut w, &header, 2)?;
        w.write_all(b",\n  \"session\": ")?;
        write_pretty(&mut w, &session, 2)?;
        let tally = match arkme {
            Some((senders, tally)) => {
                w.write_all(b",\n  \"senders\": ")?;
                write_pretty(&mut w, &senders, 2)?;
                Some(tally)
            }
            None => None,
        };
        w.write_all(b",\n  \"messages\": ")?;
        if count == 0 {
            w.write_all(b"[]")?;
        } else {
            w.write_all(b"[\n    ")?;
            spool.copy_to(&mut w)?;
            w.write_all(b"\n  ]")?;
        }
        if let Some(tally) = tally.filter(|_| self.session.is_group) {
            let members = self.ark_group_members_of(tally);
            w.write_all(b",\n  \"groupMembers\": ")?;
            write_pretty(&mut w, &members, 2)?;
        }
        w.write_all(b"\n}")?;
        w.flush()?;
        Ok(())
    }

    // ─────────────────────────────────── TXT ───────────────────────────────────

    /// Writes an export from a stream of messages (oldest first) without holding them all: the file is built as the
    /// messages arrive. Returns how many messages were read; with none, no file is written.
    pub fn write_streamed(
        &mut self,
        format: &str,
        out: &Path,
        msgs: &mut dyn Iterator<Item = ExportMsg>,
    ) -> Result<usize> {
        if let Some(format) = EntryFormat::parse(format) {
            return self.render_and_write(format, out, msgs, false);
        }
        match format {
            "weclone" => return self.stream_weclone(out, msgs),
            "arkme-json" => return self.stream_arkme_json(msgs, out, false),
            "html" => return self.stream_html(msgs, out, false),
            "excel" | "xlsx" => return self.stream_excel(msgs, out),
            _ => {}
        }
        anyhow::bail!("unsupported streamed export format: {format}")
    }

    /// WeClone: a header, then one row per message (quote replies left out), numbered as written.
    fn stream_weclone(
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

    /// One message of the TXT export: `<time> '<sender>'`, the text, a blank line.
    /// Appends the TXT entry of `msg` to `out`.
    fn write_txt_entry(&mut self, msg: &ExportMsg, out: &mut Vec<u8>) {
        use std::io::Write;
        let (content, _) = self.plain_row_content(msg, &MsgXml::new(&msg.content));
        let sender = self.sender_fields(msg);
        // writing to a Vec cannot fail
        let _ = write!(
            out,
            "{} '{}'\n{}\n\n",
            format_timestamp(msg.create_time),
            sender.role,
            content
        );
    }

    fn render_txt(&mut self, msgs: &[ExportMsg]) -> String {
        let mut out = Vec::new();
        for msg in msgs {
            self.write_txt_entry(msg, &mut out);
        }
        // the entries are written from strings, so the bytes are UTF-8
        String::from_utf8(out).unwrap_or_default()
    }

    pub fn write_txt(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let text = self.render_txt(msgs);
        write_bytes(out, text.as_bytes())
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

    fn render_weclone(&mut self, msgs: &[ExportMsg]) -> String {
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

    // ─────────────────────────────────── Excel ───────────────────────────────────

    /// A sheet with more rows than this is written in constant-memory mode (no hyperlinks, rows go to disk).
    const EXCEL_BIG: usize = 20000;

    pub fn write_excel(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        self.write_excel_rows(msgs, msgs.len() > Self::EXCEL_BIG, out)?;
        Ok(())
    }

    /// Excel written from a stream of messages (oldest first). Whether the sheet is "big" depends on the total, so
    /// the first `EXCEL_BIG + 1` messages are held back: if the stream ends before that, the small sheet is written
    /// as usual; otherwise the constant-memory sheet takes them and then the rest as they arrive.
    fn stream_excel(
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

    // ─────────────────────────────────── HTML ───────────────────────────────────

    /// HTML 气泡里的文本：与其他格式同一套写法，但链接卡片、系统消息样式由 HTML 自己渲染，不再改写。
    fn html_message_text(&mut self, msg: &ExportMsg, x: &MsgXml) -> String {
        if msg.content.is_empty() && msg.local_type != 47 {
            return String::new();
        }
        self.message_text_with(msg, x, None, false)
    }

    /// The `window.WEFLOW_DATA` item of one message; `index` is the number of messages before it.
    fn html_item(&mut self, msg: &ExportMsg, index: usize) -> HtmlItem {
        let is_group = self.session.is_group;
        let sender_name = if is_group {
            self.sender_profile_for(msg).display_name
        } else if msg.is_send {
            if self.my_display.is_empty() {
                chrome("我", "Me").to_string()
            } else {
                self.my_display.clone()
            }
        } else if self.session.display_name.is_empty() {
            self.session.id.clone()
        } else {
            self.session.display_name.clone()
        };
        let avatar_html = format!(
            "<span>{}</span>",
            html_escape(&avatar_fallback(&sender_name))
        );
        let time_text = format_timestamp(msg.create_time);
        let x = MsgXml::new(&msg.content);
        let quoted = self.quoted_with_names(msg.local_type, &x);
        let mut text = match &quoted {
            Some(q) if !q.reply_text.is_empty() => q.reply_text.clone(),
            _ => self.html_message_text(msg, &x),
        };
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
        HtmlItem {
            local_id: index + 1,
            create_time: msg.create_time,
            is_send: u8::from(msg.is_send),
            avatar_html,
            body_html: body,
            platform_message_id: msg.platform_message_id(),
            reply_to_message_id: x.reply_to_message_id(),
        }
    }

    /// The whole page around the data: `data_marker` stands where the items go.
    fn html_document(&self, total: usize, data_marker: &str) -> String {
        let is_group = self.session.is_group;
        let title = html_escape(&self.session.display_name);
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
            data = data_marker,
            scroll_js = scroll_js,
        )
    }

    /// The whole page, built in memory: what the tests hold the streamed file against.
    #[cfg(test)]
    fn render_html(&mut self, msgs: &[ExportMsg]) -> String {
        let mut data: Vec<u8> = Vec::new();
        for (i, msg) in msgs.iter().enumerate() {
            if i > 0 {
                data.extend_from_slice(b",\n");
            }
            let item = self.html_item(msg, i);
            // a Vec takes every write, and an item has only string keys
            let _ = write_script_json(&mut data, &item);
        }
        self.html_document(msgs.len(), &String::from_utf8(data).unwrap_or_default())
    }

    /// HTML written from a stream of messages (oldest first): the items go to a spool file as they are read, the
    /// page around them (which shows the total) is written at the end. With no messages nothing is written unless
    /// `write_if_empty`.
    fn stream_html<I>(&mut self, msgs: I, out: &Path, write_if_empty: bool) -> Result<usize>
    where
        I: IntoIterator,
        I::Item: std::borrow::Borrow<ExportMsg>,
    {
        use std::borrow::Borrow;
        use std::io::Write;
        let mut spool = Spool::create(out)?;
        let mut count = 0usize;
        for msg in msgs {
            let item = self.html_item(msg.borrow(), count);
            spool.push_with(",\n", |w| write_script_json(w, &item))?;
            count += 1;
        }
        if count == 0 && !write_if_empty {
            return Ok(0);
        }
        const MARKER: &str = "\u{0}WEFLOW_DATA\u{0}";
        let page = self.html_document(count, MARKER);
        let (head, tail) = page
            .split_once(MARKER)
            .context("the page template has no data placeholder")?;
        let mut w = std::io::BufWriter::with_capacity(1 << 20, std::fs::File::create(out)?);
        w.write_all(head.as_bytes())?;
        spool.copy_to(&mut w)?;
        w.write_all(tail.as_bytes())?;
        w.flush()?;
        Ok(count)
    }

    pub fn write_html(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        self.stream_html(msgs, out, true)?;
        Ok(())
    }

    // ─────────────────────────────────── SQL ───────────────────────────────────

    const SQL_HEADER: &'static str = "CREATE TABLE IF NOT EXISTS \"messages\" (\n\tid INTEGER PRIMARY KEY AUTOINCREMENT,\n\tsession_id TEXT,\n\tlocal_id INTEGER,\n\tserver_id TEXT,\n\tcreate_time INTEGER,\n\tsender TEXT,\n\tis_send INTEGER,\n\tlocal_type INTEGER,\n\ttype_name TEXT,\n\tcontent TEXT\n);\n\n";

    /// Appends the SQL `INSERT` of `msg` to `out`.
    fn write_sql_insert(&mut self, msg: &ExportMsg, out: &mut Vec<u8>) {
        use std::io::Write;
        let x = MsgXml::new(&msg.content);
        let (content, _) = self.plain_row_content(msg, &x);
        // writing to a Vec cannot fail
        let _ = writeln!(
            out,
            "INSERT INTO \"messages\" (session_id, local_id, server_id, create_time, sender, is_send, local_type, type_name, content) VALUES ('{}', {}, {}, {}, '{}', {}, {}, '{}', '{}');",
            sql_escape(&self.session.id),
            msg.local_id,
            msg.platform_message_id().map(|s| format!("'{s}'")).unwrap_or_else(|| "NULL".into()),
            msg.create_time,
            sql_escape(&msg.sender_username),
            if msg.is_send { 1 } else { 0 },
            msg.local_type,
            sql_escape(message_type_name_in(msg.local_type, &x)),
            sql_escape(&content),
        );
    }

    fn render_sql(&mut self, msgs: &[ExportMsg]) -> String {
        let mut sql = Vec::from(Self::SQL_HEADER);
        for msg in msgs {
            self.write_sql_insert(msg, &mut sql);
        }
        // the inserts are written from strings, so the bytes are UTF-8
        String::from_utf8(sql).unwrap_or_default()
    }

    pub fn write_sql(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        let sql = self.render_sql(msgs);
        write_bytes(out, sql.as_bytes())
    }
}

// ──────────────────────── JSON / ChatLab / HTML documents ────────────────────────
//
// The structs below are serialized straight into the output; their field order is the key order of the files.

/// A JSONL line: `_type` first, then the fields of `entry`.
#[derive(Serialize)]
struct Tagged<'a, T> {
    #[serde(rename = "_type")]
    kind: &'static str,
    #[serde(flatten)]
    entry: &'a T,
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
struct ChatlabMessage<'m> {
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

/// The `weflow` header of a detailed-JSON / arkme-json export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WeflowHeader {
    version: &'static str,
    exported_at: i64,
    generator: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    format: Option<&'static str>,
}

/// The `session` object of a detailed-JSON / arkme-json export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonSession {
    wxid: String,
    nickname: String,
    remark: String,
    display_name: String,
    #[serde(rename = "type")]
    kind: &'static str,
    last_timestamp: Option<i64>,
    message_count: usize,
}

/// One message of the detailed-JSON export.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonMessage<'m> {
    local_id: usize,
    create_time: i64,
    formatted_time: String,
    #[serde(rename = "type")]
    kind: &'static str,
    local_type: i64,
    content: String,
    is_send: u8,
    sender_username: &'m str,
    sender_display_name: String,
    source: &'m str,
    sender_avatar_key: &'m str,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji_md5: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji_cdn_url: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    emoji_caption: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_md5: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_dat_name: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    video_md5: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platform_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<String>,
    /// The app-message and contact-card fields ([`extract_arkme_app_message_meta`],
    /// [`extract_arkme_contact_card_meta`]) and the quoted sender and text: which keys appear depends on the message
    /// XML, so they stay a JSON map, in the order they were found.
    #[serde(flatten)]
    extra: Map<String, Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_lat: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_lng: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_poiname: Option<&'m str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    location_label: Option<&'m str>,
}

/// One message of the arkme-json export: a [`JsonMessage`] with the sender replaced by its number in the sender
/// table and only the keys arkme keeps.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArkMessage<'m> {
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

/// The keys of an arkme-json message after the location, in [`ArkTail::KEYS`] order: the sticker fields and the
/// kept keys of [`JsonMessage::extra`].
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

/// One entry of the arkme-json sender table.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArkSender {
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

/// One entry of `groupMembers` in the arkme-json export of a group.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ArkGroupMember {
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
    /// The name members with the same number of messages are ordered by.
    fn sort_name(&self) -> &str {
        if self.display_name.is_empty() {
            &self.wxid
        } else {
            &self.display_name
        }
    }
}

/// The messages of a detailed-JSON or arkme-json document.
#[cfg(test)]
#[derive(Serialize)]
#[serde(untagged)]
enum JsonMessages<'m> {
    Detailed(Vec<JsonMessage<'m>>),
    Arkme(Vec<ArkMessage<'m>>),
}

/// A whole detailed-JSON or arkme-json export (`senders` and `groupMembers` are arkme's).
#[cfg(test)]
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct JsonDocument<'m> {
    weflow: WeflowHeader,
    session: JsonSession,
    #[serde(skip_serializing_if = "Option::is_none")]
    senders: Option<Vec<ArkSender>>,
    messages: JsonMessages<'m>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group_members: Option<Vec<ArkGroupMember>>,
}

/// One item of the HTML page's `window.WEFLOW_DATA`, which the page script renders.
#[derive(Serialize)]
struct HtmlItem {
    #[serde(rename = "i")]
    local_id: usize,
    #[serde(rename = "t")]
    create_time: i64,
    #[serde(rename = "s")]
    is_send: u8,
    #[serde(rename = "a")]
    avatar_html: String,
    #[serde(rename = "b")]
    body_html: String,
    #[serde(rename = "p", skip_serializing_if = "Option::is_none")]
    platform_message_id: Option<String>,
    #[serde(rename = "r", skip_serializing_if = "Option::is_none")]
    reply_to_message_id: Option<String>,
}

/// serde_json's compact format with every `</` inside a string written as `<\/`, so the JSON cannot close the
/// `<script>` element it is embedded in.
struct ScriptSafe;

impl Formatter for ScriptSafe {
    // `<` and `/` are never escaped, so a `</` of the text always lies within one fragment
    fn write_string_fragment<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        fragment: &str,
    ) -> io::Result<()> {
        let mut rest = fragment;
        while let Some(at) = rest.find("</") {
            writer.write_all(&rest.as_bytes()[..=at])?;
            writer.write_all(b"\\")?;
            rest = &rest[at + 1..];
        }
        writer.write_all(rest.as_bytes())
    }
}

/// Writes `value` as compact JSON that is safe inside a `<script>` element (see [`ScriptSafe`]).
fn write_script_json(writer: &mut impl io::Write, value: &impl Serialize) -> io::Result<()> {
    let mut serializer = serde_json::Serializer::with_formatter(writer, ScriptSafe);
    value.serialize(&mut serializer).map_err(io::Error::from)
}

// ─────────────────────────────── small helpers ───────────────────────────────

/// Turns detailed-JSON messages into arkme-json's compact form and numbers the senders on first sight.
#[derive(Default)]
struct ArkCompact {
    sender_ids: HashMap<String, usize>,
    senders: Vec<ArkSender>,
}

impl ArkCompact {
    /// `profile_of(sender)` supplies the sender's name data the first time that sender appears.
    fn compact<'m>(
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
                // without a nickname, the name shown stands in for it
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

/// serde_json's pretty printing (two-space indent) for a value that sits `base` spaces deep in a document written
/// piece by piece: every line after the first is indented by `base` more spaces.
struct NestedPretty {
    base: usize,
    depth: usize,
    has_value: bool,
}

impl NestedPretty {
    fn new(base: usize) -> Self {
        Self {
            base,
            depth: 0,
            has_value: false,
        }
    }

    fn new_line<W: ?Sized + io::Write>(&self, writer: &mut W) -> io::Result<()> {
        const SPACES: &[u8; 32] = b"                                ";
        writer.write_all(b"\n")?;
        let mut left = self.base + 2 * self.depth;
        while left > 0 {
            let n = left.min(SPACES.len());
            writer.write_all(&SPACES[..n])?;
            left -= n;
        }
        Ok(())
    }
}

impl Formatter for NestedPretty {
    fn begin_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        writer.write_all(b"[")
    }

    fn end_array<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth -= 1;
        if self.has_value {
            self.new_line(writer)?;
        }
        writer.write_all(b"]")
    }

    fn begin_array_value<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if !first {
            writer.write_all(b",")?;
        }
        self.new_line(writer)
    }

    fn end_array_value<W: ?Sized + io::Write>(&mut self, _writer: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }

    fn begin_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth += 1;
        self.has_value = false;
        writer.write_all(b"{")
    }

    fn end_object<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        self.depth -= 1;
        if self.has_value {
            self.new_line(writer)?;
        }
        writer.write_all(b"}")
    }

    fn begin_object_key<W: ?Sized + io::Write>(
        &mut self,
        writer: &mut W,
        first: bool,
    ) -> io::Result<()> {
        if !first {
            writer.write_all(b",")?;
        }
        self.new_line(writer)
    }

    fn begin_object_value<W: ?Sized + io::Write>(&mut self, writer: &mut W) -> io::Result<()> {
        writer.write_all(b": ")
    }

    fn end_object_value<W: ?Sized + io::Write>(&mut self, _writer: &mut W) -> io::Result<()> {
        self.has_value = true;
        Ok(())
    }
}

/// Writes `value` pretty-printed like `serde_json::to_string_pretty`, with every line after the first indented by
/// `indent` spaces, for a value nested that deep in a document written piece by piece.
fn write_pretty(
    writer: &mut impl io::Write,
    value: &impl Serialize,
    indent: usize,
) -> io::Result<()> {
    let mut serializer = serde_json::Serializer::with_formatter(writer, NestedPretty::new(indent));
    value.serialize(&mut serializer).map_err(io::Error::from)
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
    // the cause goes into the message itself: only the outermost context reaches the user
    fs::write(out, bytes).map_err(|e| anyhow::anyhow!("failed to write {}: {e}", out.display()))
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
    use weflow_native::msg_row::MsgRow;

    fn row(
        local_id: i64,
        create_time: i64,
        local_type: i64,
        content: &str,
        sender: Option<&str>,
    ) -> MsgRow {
        MsgRow {
            local_id,
            create_time: Some(create_time),
            local_type,
            content: content.into(),
            sender: sender.map(str::to_string),
            ..MsgRow::default()
        }
    }

    fn fixture() -> (Vec<ExportMsg>, HashMap<String, ContactInfo>) {
        let rows = vec![
            MsgRow {
                server_id: Some(111),
                is_send: true,
                ..row(1, 1700000000, 1, r#"hello "world", ok"#, None)
            },
            MsgRow {
                server_id: Some(222),
                ..row(2, 1700000060, 1, "wxid_bob:hi <b>there</b>", Some("wxid_bob"))
            },
            row(3, 1700000120, 3, "", Some("wxid_bob")),
            row(4, 1700000180, 49, "<msg><appmsg><title>Doc</title><type>5</type><url>https://example.com/x</url></appmsg></msg>", Some("wxid_bob")),
            MsgRow {
                server_id: Some(555),
                ..row(5, 1700000240, 49, "<msg><appmsg><title>yes</title><type>57</type><refermsg><type>1</type><displayname>Me</displayname><content>hello</content><svrid>111</svrid></refermsg></appmsg></msg>", Some("wxid_bob"))
            },
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
        exporter_with(contacts, group, |ex| f(ex, &msgs))
    }

    /// The exporter of the chat with `wxid_bob` (or a group of that id), knowing `contacts`.
    fn exporter_with<R>(
        contacts: HashMap<String, ContactInfo>,
        group: bool,
        f: impl FnOnce(&mut Exporter<'_, '_>) -> R,
    ) -> R {
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
        f(&mut ex)
    }

    /// detailed JSON 与简易 txt 共用 `message_text`：各类消息的写法在两种格式里一致。
    #[test]
    fn message_text_is_uniform_across_message_kinds() {
        let quote_inner = "&lt;msg&gt;&lt;img md5=\"x\"&gt;&lt;extcommoninfo&gt;&lt;refermsg&gt;&lt;svrid&gt;9&lt;/svrid&gt;&lt;/refermsg&gt;&lt;/extcommoninfo&gt;&lt;/msg&gt;";
        let rows = vec![
            row(1, 1700000000, 47, r#"wxid_bob:<msg><emoji md5="a"/></msg>"#, Some("wxid_bob")),
            row(2, 1700000001, 34, "", Some("wxid_bob")),
            row(3, 1700000002, 10000, r#""Bob" 撤回了一条消息"#, None),
            // 图片里夹带 <refermsg>：不是文字引用，仍然是 [图片]
            row(4, 1700000003, 3, "wxid_bob:\n<msg><img md5=\"x\"/><extcommoninfo><refermsg><svrid>9</svrid></refermsg></extcommoninfo></msg>", Some("wxid_bob")),
            // appmsg type 8：以附件形式发送的表情
            row(5, 1700000004, 34359738417, "wxid_bob:\n<msg><appmsg><title /><type>8</type></appmsg></msg>", Some("wxid_bob")),
            // 引用一条「带引用的图片」：被引用内容是（转义后的）XML，且里面还嵌着一层 <refermsg>
            row(6, 1700000005, 244813135921, &format!("wxid_bob:\n<msg><appmsg><title>好看</title><type>57</type><refermsg><type>3</type><content>wxid_me:\n{quote_inner}</content><displayname>Me Nick</displayname><svrid>1</svrid></refermsg></appmsg></msg>"), Some("wxid_bob")),
            // 正文里的 "4:1" 不是发送者前缀
            row(7, 1700000006, 1, "4:1", Some("wxid_bob")),
        ];
        let msgs = collect_rows(
            rows,
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
            row(1, 1700000000, 10000, r#""Bob" 撤回了一条消息"#, None),
            row(2, 1700000001, 34, "", Some("wxid_bob")),
            row(3, 1700000002, 49, "wxid_bob:\n<msg><appmsg><title>Doc</title><type>5</type><url>https://example.com/x</url></appmsg></msg>", Some("wxid_bob")),
            row(4, 1700000003, 103079215153, "wxid_bob:\n<msg><appmsg><title /><des>预览</des><type>24</type><recorditem><![CDATA[<recordinfo><datalist><dataitem datatype=\"1\"><datadesc>笔记&#x0A;全文</datadesc></dataitem></datalist></recordinfo>]]></recorditem></appmsg></msg>", Some("wxid_bob")),
            row(5, 1700000004, 244813135921, "wxid_bob:\n<msg><appmsg><title>好的</title><type>57</type><refermsg><type>1</type><displayname>Me</displayname><content>hello</content><svrid>1</svrid></refermsg></appmsg></msg>", Some("wxid_bob")),
        ];
        let mut msgs = collect_rows(
            rows,
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
            assert_eq!(
                ex.plain_row_content(m, &MsgXml::new(&m.content)).0,
                ex.message_text(m)
            );
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
            let full = ex.message_text_generic(
                &msg,
                &MsgXml::new(&msg.content),
                Some(LinkStyle::AppendUrl),
                true,
            );
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

    /// Pages rendered by separate exporters (as the reading threads of an export do, each with its own name cache)
    /// and written in order give the file one exporter writes alone, including the ChatLab members that only appear
    /// inside forwarded chat histories.
    #[test]
    fn entries_rendered_in_pages_by_other_exporters_write_the_same_file() {
        use crate::message::ForwardRecord;
        let dir = std::env::temp_dir().join(format!("weflow-rendered-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let with_records = |msgs: &[ExportMsg]| {
            let mut all = msgs.to_vec();
            for (i, people) in [&["Alice", "Carol"][..], &["Dave"], &["Alice", ""]]
                .iter()
                .enumerate()
            {
                all.push(ExportMsg {
                    local_id: 9000 + i as i64,
                    create_time: 1_700_000_900 + i as i64,
                    local_type: 49,
                    sender_username: if i == 1 { "wxid_me" } else { "wxid_bob" }.into(),
                    chat_record_list: Some(
                        people
                            .iter()
                            .map(|who| ForwardRecord {
                                datatype: 1,
                                sourcename: who.to_string(),
                                sourcetime: "2023-11-14 22:20:00".into(),
                                datadesc: Some(format!("from {who}")),
                                ..Default::default()
                            })
                            .collect(),
                    ),
                    ..Default::default()
                });
            }
            all
        };
        for group in [false, true] {
            for (name, format) in [
                ("txt", EntryFormat::Txt),
                ("sql", EntryFormat::Sql),
                ("chatlab", EntryFormat::Chatlab),
                ("chatlab-jsonl", EntryFormat::ChatlabJsonl),
                ("json", EntryFormat::Json),
            ] {
                let alone = dir.join(format!("{group}-alone.{name}"));
                let paged = dir.join(format!("{group}-paged.{name}"));
                let written = with_exporter(group, |ex, msgs| {
                    ex.write_streamed(name, &alone, &mut with_records(msgs).into_iter())
                        .unwrap()
                });
                // uneven pages, each rendered by an exporter of its own
                let all = with_exporter(group, |_, msgs| with_records(msgs));
                let mut pages: Vec<RenderedEntries> = Vec::new();
                let mut rest = &all[..];
                for size in [1, 3, 2].iter().cycle() {
                    if rest.is_empty() {
                        break;
                    }
                    let (page, tail) = rest.split_at((*size).min(rest.len()));
                    pages.push(with_exporter(group, |ex, _| {
                        ex.render_entries(format, page)
                    }));
                    rest = tail;
                }
                let mut pages = pages.into_iter();
                let count = with_exporter(group, |ex, _| {
                    ex.write_rendered(format, &paged, &mut |_| pages.next(), false)
                        .unwrap()
                });
                assert_eq!(count, written, "{name}, group={group}");
                assert_eq!(
                    std::fs::read(&paged).unwrap(),
                    std::fs::read(&alone).unwrap(),
                    "{name}, group={group}"
                );
            }
        }
        let lab = std::fs::read_to_string(dir.join("true-alone.chatlab")).unwrap();
        for who in ["Alice", "Carol", "Dave"] {
            assert!(
                lab.contains(&format!("\"platformId\": \"{who}\"")),
                "{who} is a member"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
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

    /// Script-safe JSON is compact JSON with every `</` written as `<\/`, also next to escaped characters.
    #[test]
    fn script_json_escapes_every_closing_tag() {
        for text in [
            "</script>",
            "a</b></c>",
            "<</",
            "\"</\"\n</",
            "<\\/",
            "</",
            "plain",
        ] {
            let value = json!({ "b": text, "</k": [text, 1] });
            let mut out = Vec::new();
            write_script_json(&mut out, &value).unwrap();
            let want = serde_json::to_string(&value).unwrap().replace("</", "<\\/");
            assert_eq!(String::from_utf8(out).unwrap(), want, "{text:?}");
        }
    }

    #[test]
    fn sql_inserts() {
        let sql = with_exporter(false, |ex, msgs| ex.render_sql(msgs));
        assert!(sql.contains("CREATE TABLE IF NOT EXISTS \"messages\""));
        assert_eq!(sql.matches("INSERT INTO").count(), 5);
        assert!(sql.contains("hello \"world\", ok"));
    }

    /// Streaming: every format can be written from an iterator; Excel switches to the constant-memory sheet
    /// only once there are more than `EXCEL_BIG` messages.
    #[test]
    fn streamed_exports_cover_every_format_and_both_excel_sizes() {
        let dir = std::env::temp_dir().join(format!("weflow-stream-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for format in [
            "txt",
            "sql",
            "weclone",
            "json",
            "arkme-json",
            "chatlab",
            "chatlab-jsonl",
            "html",
            "excel",
        ] {
            let out = dir.join(format!("{format}.out"));
            let n = with_exporter(true, |ex, msgs| {
                ex.write_streamed(format, &out, &mut msgs.iter().cloned())
                    .unwrap()
            });
            assert_eq!(n, 5, "{format}");
            assert!(std::fs::metadata(&out).unwrap().len() > 0, "{format}");
            let none = dir.join(format!("{format}.none"));
            let n = with_exporter(true, |ex, _| {
                ex.write_streamed(format, &none, &mut std::iter::empty())
                    .unwrap()
            });
            assert_eq!(n, 0, "{format}");
            assert!(
                !none.exists(),
                "{format}: nothing is written for no messages"
            );
        }
        // one message more than the small sheet takes: the constant-memory sheet gets all of them
        let big = dir.join("big.xlsx");
        let n = with_exporter(false, |ex, msgs| {
            let template = msgs[1].clone();
            let mut many = (0..=Exporter::EXCEL_BIG).map(|i| {
                let mut m = template.clone();
                m.local_id = i as i64;
                m.create_time = 1_700_000_000 + i as i64;
                m
            });
            ex.write_streamed("excel", &big, &mut many).unwrap()
        });
        assert_eq!(n, Exporter::EXCEL_BIG + 1);
        assert!(std::fs::metadata(&big).unwrap().len() > 0);
        let leftovers = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".part"))
            .count();
        assert_eq!(leftovers, 0);
        let _ = std::fs::remove_dir_all(&dir);
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
    /// A text message of the chat with `wxid_bob` as the property test makes it: any text (control characters,
    /// emoji, any length up to 20000 characters), any server id that is not negative (WeChat's are below 2^63, and
    /// one of 0 or less is none), sent or received.
    fn text_message() -> impl proptest::strategy::Strategy<Value = (String, i64, bool)> {
        use proptest::prelude::*;
        let text = prop_oneof![
            4 => any::<String>(),
            1 => prop::collection::vec(any::<char>(), 0..20_000).prop_map(String::from_iter),
        ];
        (text, 0..=i64::MAX, any::<bool>())
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(64))]

        /// Any text, time and server id of text messages comes back unchanged from the JSON, ChatLab and ChatLab
        /// JSONL files, as the streamed writers write them (#502).
        #[test]
        fn text_messages_come_back_from_the_files_unchanged(
            messages in proptest::collection::vec(text_message(), 1..6),
            first in 0i64..4_000_000_000,
        ) {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("weflow-lossless-{}-{n}", std::process::id()));
            fs::create_dir_all(&dir).unwrap();
            let rows: Vec<MsgRow> = (1i64..)
                .zip(&messages)
                .map(|(i, (text, server_id, is_send))| MsgRow {
                    server_id: Some(*server_id),
                    is_send: *is_send,
                    ..row(i, first + i * 60, 1, text, (!*is_send).then_some("wxid_bob"))
                })
                .collect();
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
            proptest::prop_assert_eq!(msgs.len(), messages.len());
            let (json, lab, jsonl) = (dir.join("m.json"), dir.join("m.chatlab.json"), dir.join("m.jsonl"));
            exporter_with(fixture().1, false, |ex| {
                ex.write_json(&msgs, &json, false).unwrap();
                ex.write_chatlab(&msgs, &lab, false).unwrap();
                ex.write_chatlab(&msgs, &jsonl, true).unwrap();
            });
            let read = |f: &Path| -> Value { serde_json::from_str(&fs::read_to_string(f).unwrap()).unwrap() };
            let (json, lab) = (read(&json), read(&lab));
            let lines: Vec<Value> = fs::read_to_string(&jsonl)
                .unwrap()
                .lines()
                .map(|l| serde_json::from_str::<Value>(l).unwrap())
                .filter(|l| l["_type"] == "message")
                .collect();
            let _ = fs::remove_dir_all(&dir);
            for (i, (text, server_id, is_send)) in messages.iter().enumerate() {
                let time = first + (i as i64 + 1) * 60;
                let platform = (*server_id > 0).then(|| server_id.to_string());
                let sender = if *is_send { "wxid_me" } else { "wxid_bob" };
                let j = &json["messages"][i];
                proptest::prop_assert_eq!(j["content"].as_str(), Some(text.as_str()), "json {}", i);
                proptest::prop_assert_eq!(j["createTime"].as_i64(), Some(time));
                proptest::prop_assert_eq!(j["platformMessageId"].as_str(), platform.as_deref());
                for (format, m) in [("chatlab", &lab["messages"][i]), ("jsonl", &lines[i])] {
                    proptest::prop_assert_eq!(m["content"].as_str(), Some(text.as_str()), "{} {}", format, i);
                    proptest::prop_assert_eq!(m["timestamp"].as_i64(), Some(time), "{}", format);
                    proptest::prop_assert_eq!(m["sender"].as_str(), Some(sender), "{}", format);
                    proptest::prop_assert_eq!(m["platformMessageId"].as_str(), platform.as_deref(), "{}", format);
                }
            }
        }
    }
}
