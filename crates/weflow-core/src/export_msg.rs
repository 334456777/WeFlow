//! Chat-message exporters ported from WeFlow's desktop app: ChatLab (+JSONL), detailed JSON,
//! Arkme JSON, Excel, TXT, WeClone CSV and HTML.
//!
//! Everything here works on already-collected [`ExportMsg`] values plus a name book for
//! contacts, so it is testable without a WeChat database. Data values (message text,
//! type names, JSON keys) follow WeFlow's desktop app; only human-facing chrome such as
//! spreadsheet headers and the HTML page follows the CLI language.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

use crate::locale;
use crate::message::emoji::format_emoji_semantic_text;
use crate::message::link::{format_link_card_export_text, LinkStyle};
use crate::message::quote::{build_quoted_reply_text, is_text_quote_message_in, QuotedDisplay};
use crate::message::system::extract_readable_system_message_text;
use crate::message::transfer::{
    append_transfer_desc, is_same_wxid, is_transfer_export_content, transfer_parties,
};
use crate::message::xml::{strip_sender_line, MsgXml};
use crate::message::*;

mod arkme;
mod chatlab;
mod excel;
mod format;
mod html;
mod json;
mod list_html;
mod moments_html;
pub mod names;
pub(crate) mod spool;
mod sql;
mod txt;
mod weclone;

use chatlab::Tagged;

use format::write_pretty;
use names::{resolve_group_nickname, DisplayPref, NameBook, Profile};
use spool::{EntryFormat, RenderedEntries};

pub use format::csv_cell;
pub use list_html::{ListKind, ListPage};
pub use moments_html::{MomentComment, MomentMedia, MomentMediaKind, MomentPost, MomentsPage};

fn chrome(zh: &'static str, en: &'static str) -> &'static str {
    locale::tr(en, zh)
}

/// Messages rendered at a time when an export is rendered and written on one thread.
const RENDER_CHUNK: usize = 2000;

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
    /// The avatars on the HTML page: `{ username: path from the export file }`.
    pub avatars: HashMap<String, String>,
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
            avatars: HashMap::new(),
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

    // ─────────────────────────────────── HTML ───────────────────────────────────

    /// HTML 页面里一条消息的文本（见 [`html`]）：与其他格式同一套写法，但链接卡片、系统消息由页面自己渲染。
    fn html_message_text(&mut self, msg: &ExportMsg, x: &MsgXml) -> String {
        if msg.content.is_empty() && msg.local_type != 47 {
            return String::new();
        }
        self.message_text_with(msg, x, None, false)
    }
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
        1 => strip_sender_line(c, Some(&msg.sender_username)),
        3 => "[图片]".into(),
        34 => "[语音消息]".into(),
        43 => "[视频]".into(),
        _ => format_emoji_semantic_text(msg.emoji_caption.as_deref()),
    })
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

#[cfg(test)]
mod tests {
    use super::names::ContactInfo;
    use super::*;
    use serde_json::{json, Map, Value};
    use weflow_native::fixture::TestDir;
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

    pub(super) fn with_exporter<R>(
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
            // a first line that names somebody else is the text's own (#656)
            (1, "PS:\nsee you", None),
            (1, "wxid_carol:\nhi", None),
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
            if ty == 1 {
                let expected = content.strip_prefix("wxid_bob:\n").unwrap_or(content);
                assert_eq!(fast, expected, "{content:?}");
            }
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
        let dir = TestDir::new("weflow-rendered");
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
    }

    #[test]
    fn streamed_exports_are_byte_identical_to_serializing_the_whole_value() {
        let dir = TestDir::new("weflow-json-stream");
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
    }

    /// The page is the template with its placeholders filled in: the title escaped, the data as JSON that cannot
    /// close its `<script>` element, one entry for each message.
    #[test]
    fn html_page_holds_the_data() {
        let html = with_exporter(false, |ex, msgs| ex.render_html(msgs));
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(!html.contains("__WF_"), "every placeholder filled in");
        assert!(html.contains("<title>Bobby</title>"));
        let open = "<script id=\"wf-data\" type=\"application/json\">";
        let start = html.find(open).unwrap() + open.len();
        let end = start + html[start..].find("</script>").unwrap();
        let data: Value = serde_json::from_str(&html[start..end]).unwrap();
        assert_eq!(data["v"], 1);
        assert_eq!(data["kind"], "private");
        assert_eq!(data["title"], "Bobby");
        let msgs = data["msgs"].as_array().unwrap();
        assert_eq!(msgs.len(), 5);
        // the user and the other side, each once
        let users = data["users"].as_array().unwrap();
        assert_eq!(users.len(), 2);
        let me = data["me"].as_i64().unwrap();
        assert_eq!(users[usize::try_from(me).unwrap()][0], "Me Nick");
        assert!(
            msgs.iter()
                .any(|m| m["x"].as_str().is_some_and(|x| x.contains("<b>there</b>"))),
            "text stays text in the data: the page escapes it"
        );
        assert!(
            !html.contains("<b>there</b>"),
            "and no `<` of it is markup in the file"
        );
        // text, text, a picture whose file is not there, a link, a quote of the first message
        let kinds: Vec<i64> = msgs.iter().map(|m| m["k"].as_i64().unwrap()).collect();
        assert_eq!(kinds, [1, 1, 2, 7, 8]);
        assert!(msgs[2].get("s").is_none());
        assert_eq!(msgs[3]["ti"], "Doc");
        assert_eq!(msgs[3]["l"], "https://example.com/x");
        assert_eq!(msgs[4]["r"], 0);
    }

    /// A quote of a message that comes later in the export names it by id, and that message carries the id.
    #[test]
    fn html_quote_of_a_later_message_goes_by_id() {
        let html = with_exporter(false, |ex, msgs| {
            ex.render_html(&[msgs[4].clone(), msgs[0].clone()])
        });
        let open = "<script id=\"wf-data\" type=\"application/json\">";
        let start = html.find(open).unwrap() + open.len();
        let end = start + html[start..].find("</script>").unwrap();
        let data: Value = serde_json::from_str(&html[start..end]).unwrap();
        let msgs = data["msgs"].as_array().unwrap();
        assert!(msgs[0].get("r").is_none());
        assert_eq!(msgs[0]["rq"], "111");
        assert_eq!(msgs[1]["id"], "111");
    }

    /// Streaming: every format can be written from an iterator; Excel switches to the constant-memory sheet
    /// only once there are more than `EXCEL_BIG` messages.
    #[test]
    fn streamed_exports_cover_every_format_and_both_excel_sizes() {
        let dir = TestDir::new("weflow-stream");
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
            let dir = TestDir::new("weflow-lossless");
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
