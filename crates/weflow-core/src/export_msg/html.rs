//! The HTML export: one page, the template for the CLI language with the chat in it as JSON.
//!
//! The template is `template.html` beside this file, made small by `build.mjs` into `template.zh.html` and
//! `template.en.html`, which are embedded here. It has three placeholders: `__WF_LANG__`, `__WF_TITLE__` and
//! `__WF_DATA__`, the last inside a `<script type="application/json">` element.
//!
//! The data (version 1):
//!
//! ```text
//! { v: 1, lang: "zh" | "en", title, kind: "group" | "private" | "official", me: the user's index in users (-1: none),
//!   exported: unix seconds, users: [[name, avatar path ("": none)]], msgs }
//! msgs[i] = { t: unix seconds, u: index into users (-1: none), k: kind, ...the kind's fields }
//!   0 system {x}  1 text {x}  2 picture {s,w,h}  3 voice {s,d}  4 video {s,w,h,d}  5 sticker {s,w,h}
//!   6 file {n,z,e,s}  7 link {ti,x,l,f}  8 quote {x,qn,qx,r | rq}  9 chat history {ti,it: [[name, time, text]]}
//!   10 transfer {a,x,st}  11 red packet {ti}  12 location {ti,x,la,lo}  13 contact card {n}  14 call {v,x}
//!   15 music {ti,x,l}  16 mini program {ti,f}  17 channels {ti,f}  18 announcement {x}  19 other app message {ti,x,l,f}
//! ```
//!
//! `s` is a media file's path relative to the page (missing: the file is not there); `w`, `h` its size when its
//! header gives it. A quote's `r` is the index of the message it quotes; a quote of a message that comes later in
//! the export names it by `rq` instead, and that message carries the same `id`. The messages are in the export's
//! order; the page shows times in the time zone of the computer that opens it.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Map, Value};

use super::{chrome, html_escape, write_script_json, Exporter, Spool};
use crate::locale;
use crate::message::{
    clean_system_message, decode_html_entities, extract_arkme_app_message_meta_in,
    extract_xml_value, rx, ExportMsg, MsgXml, QuotedDisplay,
};

const TEMPLATE_ZH: &str = include_str!("template.zh.html");
const TEMPLATE_EN: &str = include_str!("template.en.html");

/// The kinds of message (`k`), as the page numbers them.
#[derive(Clone, Copy)]
enum Kind {
    System = 0,
    Text = 1,
    Picture = 2,
    Voice = 3,
    Video = 4,
    Sticker = 5,
    File = 6,
    Link = 7,
    Quote = 8,
    History = 9,
    Transfer = 10,
    RedPacket = 11,
    Location = 12,
    ContactCard = 13,
    Call = 14,
    Music = 15,
    MiniProgram = 16,
    Channels = 17,
    Announcement = 18,
    App = 19,
}

/// One message of the page's data; a field the kind does not have is left out.
#[derive(Serialize, Default)]
struct PageMsg {
    t: i64,
    u: i64,
    k: u8,
    #[serde(skip_serializing_if = "Option::is_none")]
    x: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    s: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    w: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    h: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    d: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    n: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    z: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    e: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    l: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    f: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    qn: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    qx: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    r: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rq: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    it: Option<Vec<(String, Value, String)>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    a: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    st: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    la: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    lo: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    v: Option<u8>,
}

/// What the page's data needs from the messages before them, collected as they are read.
#[derive(Default)]
struct PageState {
    /// The folder the page is written in, which the media paths start from.
    dir: PathBuf,
    /// [name, avatar path] of each sender, in order of first appearance.
    users: Vec<(String, String)>,
    /// A sender's index in `users`, by [`user_key`].
    user_ix: HashMap<String, usize>,
    /// A message's index by its server id.
    ids: HashMap<u64, usize>,
    /// The server ids quoted before their message came.
    wanted: HashSet<u64>,
}

/// The key of a message's sender among the page's users: "" for the user.
fn user_key<'m>(ex: &Exporter<'_, '_>, msg: &'m ExportMsg) -> &'m str {
    if msg.is_send {
        ""
    } else if ex.session.is_group {
        &msg.sender_username
    } else {
        // the other side of a private chat; any id but ""
        "\u{1}"
    }
}

fn opt(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

fn meta_str(meta: &Map<String, Value>, key: &str) -> Option<String> {
    meta.get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

impl Exporter<'_, '_> {
    /// The page for messages already in memory: what the tests hold the streamed file against.
    #[cfg(test)]
    pub(super) fn render_html(&mut self, msgs: &[ExportMsg]) -> String {
        let mut state = PageState::default();
        let mut data: Vec<u8> = Vec::new();
        for (i, msg) in msgs.iter().enumerate() {
            if i > 0 {
                data.push(b',');
            }
            let item = self.page_msg(&mut state, msg, i);
            // a Vec takes every write
            let _ = write_script_json(&mut data, &item);
        }
        let mut page = Vec::new();
        let _ = self.write_page(&mut page, &state, |w| w.write_all(&data));
        String::from_utf8(page).unwrap_or_default()
    }

    /// HTML written from a stream of messages: each message's data goes to a spool file as it is read, and the page
    /// around them (whose data opens with the senders) is written at the end. With no messages nothing is written
    /// unless `write_if_empty`.
    pub(super) fn stream_html<I>(
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
        let mut spool = Spool::create(out)?;
        let mut state = PageState {
            dir: out.parent().map(Path::to_path_buf).unwrap_or_default(),
            ..Default::default()
        };
        let mut count = 0usize;
        for msg in msgs {
            let item = self.page_msg(&mut state, msg.borrow(), count);
            spool.push_with(",", |w| write_script_json(w, &item))?;
            count += 1;
        }
        if count == 0 && !write_if_empty {
            return Ok(0);
        }
        let mut w = io::BufWriter::with_capacity(1 << 20, File::create(out)?);
        self.write_page(&mut w, &state, |w| {
            spool
                .copy_to(w)
                .map_err(|e| io::Error::other(format!("{e:#}")))
        })
        .with_context(|| format!("failed to write {}", out.display()))?;
        w.flush()?;
        Ok(count)
    }

    pub fn write_html(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        self.stream_html(msgs, out, true)?;
        Ok(())
    }

    /// The page: the template with its language and title filled in, and the data, whose messages `write_msgs`
    /// writes (comma-separated).
    fn write_page<W: Write>(
        &self,
        w: &mut W,
        state: &PageState,
        write_msgs: impl FnOnce(&mut W) -> io::Result<()>,
    ) -> io::Result<()> {
        let (template, lang, page_lang) = match locale::current() {
            locale::Lang::Zh => (TEMPLATE_ZH, "zh", "zh-CN"),
            locale::Lang::En => (TEMPLATE_EN, "en", "en"),
        };
        let (head, tail) = template
            .split_once("__WF_DATA__")
            .ok_or_else(|| io::Error::other("the page template has no data placeholder"))?;
        let title = html_escape(&self.session.display_name);
        w.write_all(
            head.replacen("__WF_LANG__", page_lang, 1)
                .replacen("__WF_TITLE__", &title, 1)
                .as_bytes(),
        )?;
        let kind = if self.session.is_group {
            "group"
        } else if self.session.id.starts_with("gh_") {
            "official"
        } else {
            "private"
        };
        let me = state.user_ix.get("").map_or(-1, |&i| i as i64);
        w.write_all(b"{\"v\":1,\"lang\":")?;
        write_script_json(w, &lang)?;
        w.write_all(b",\"title\":")?;
        write_script_json(w, &self.session.display_name)?;
        write!(
            w,
            ",\"kind\":\"{kind}\",\"me\":{me},\"exported\":{},\"users\":",
            self.settings.exported_at
        )?;
        write_script_json(w, &state.users)?;
        w.write_all(b",\"msgs\":[")?;
        write_msgs(w)?;
        w.write_all(b"]}")?;
        w.write_all(tail.as_bytes())
    }

    /// The sender's index in the page's users, adding the sender on first sight.
    fn page_user(&mut self, state: &mut PageState, msg: &ExportMsg) -> i64 {
        let key = user_key(self, msg);
        if let Some(&i) = state.user_ix.get(key) {
            return i as i64;
        }
        let key = key.to_string();
        let name = if self.session.is_group {
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
        let username = if msg.is_send {
            self.my_wxid.as_str()
        } else if self.session.is_group {
            msg.sender_username.as_str()
        } else {
            self.session.id.as_str()
        };
        let avatar = self
            .settings
            .avatars
            .get(username)
            .map(|path| url_path(path))
            .unwrap_or_default();
        let i = state.users.len();
        state.users.push((name, avatar));
        state.user_ix.insert(key, i);
        i as i64
    }

    /// One message of the page's data; `index` is the number of messages before it.
    fn page_msg(&mut self, state: &mut PageState, msg: &ExportMsg, index: usize) -> PageMsg {
        let x = MsgXml::new(&msg.content);
        let mut m = PageMsg {
            t: msg.create_time,
            ..Default::default()
        };
        // a message quoted before it came carries its id
        let id = msg
            .platform_message_id()
            .and_then(|s| s.parse::<u64>().ok());
        if let Some(id) = id {
            state.ids.insert(id, index);
            if state.wanted.remove(&id) {
                m.id = Some(id.to_string());
            }
        }
        let low = msg.local_type & 0xFFFF_FFFF;
        // a quote first: one of a system message holds the system message's XML; a pat (app type 62) is a system line
        let quoted = self
            .quoted_with_names(msg.local_type, &x)
            .or_else(|| quote_of_system(&x));
        if quoted.is_none()
            && (matches!(low, 10000 | 10002)
                || x.app_type_twice() == "62"
                || x.readable_system_twice(msg.local_type))
        {
            m.u = -1;
            m.k = Kind::System as u8;
            // a pat's line keeps the entities of its XML
            m.x = Some(decode_html_entities(&self.html_message_text(msg, &x)));
            return m;
        }
        m.u = self.page_user(state, msg);
        if let Some(q) = quoted {
            m.k = Kind::Quote as u8;
            // the quoted message shows under the reply, not in it too
            m.x = opt(q.reply_text);
            m.qn = q.quoted_sender.filter(|s| !s.is_empty());
            m.qx = opt(q.quoted_preview);
            if let Some(to) = x.reply_to_message_id().and_then(|s| s.parse::<u64>().ok()) {
                match state.ids.get(&to) {
                    Some(&i) => m.r = Some(i),
                    None => {
                        state.wanted.insert(to);
                        m.rq = Some(to.to_string());
                    }
                }
            }
            return m;
        }
        if let Some(media) = &msg.media {
            let kind = match media.kind {
                "image" => Kind::Picture,
                "voice" => Kind::Voice,
                "emoji" => Kind::Sticker,
                "file" => Kind::File,
                _ => Kind::Video,
            };
            m.s = Some(url_path(&media.relative_path));
            if matches!(kind, Kind::File) {
                // its name, size and extension come from the app message, as when the file is not copied
                return self.app_msg(m, msg, &x);
            }
            let file = state.dir.join(&media.relative_path);
            match kind {
                Kind::Picture | Kind::Sticker => {
                    if let Some((w, h)) = image_size(&file) {
                        (m.w, m.h) = (Some(w), Some(h));
                    }
                }
                Kind::Video => {
                    if let Some(v) = mp4_info(&file) {
                        (m.w, m.h) = (Some(v.width), Some(v.height));
                        m.d = (v.seconds > 0).then_some(v.seconds);
                    }
                }
                _ => {}
            }
            return self.fill_kind(m, kind, msg, &x);
        }
        let kind = match low {
            3 => Kind::Picture,
            34 => Kind::Voice,
            43 => Kind::Video,
            47 => Kind::Sticker,
            42 => Kind::ContactCard,
            48 => Kind::Location,
            50 => Kind::Call,
            1 => Kind::Text,
            _ => return self.app_msg(m, msg, &x),
        };
        self.fill_kind(m, kind, msg, &x)
    }

    /// The fields of a message of a kind its local type names.
    fn fill_kind(&mut self, mut m: PageMsg, kind: Kind, msg: &ExportMsg, x: &MsgXml) -> PageMsg {
        m.k = kind as u8;
        match kind {
            Kind::Voice => {
                m.d = crate::chat_msg::parse_voice_duration_seconds(&msg.content).or(Some(1))
            }
            Kind::Video => {
                if m.d.is_none() {
                    m.d = rx(r#"(?i)playlength\s*=\s*['"]?([0-9]+)"#)
                        .captures(&msg.content)
                        .and_then(|c| c[1].parse().ok())
                        .filter(|&d: &i64| d > 0);
                }
            }
            Kind::ContactCard => {
                m.n = rx(r#"(?i)\snickname\s*=\s*"([^"]*)""#)
                    .captures(&msg.content)
                    .map(|c| c[1].to_string())
                    .and_then(opt)
                    .or_else(|| opt(self.html_message_text(msg, x)));
            }
            Kind::Location => {
                m.ti = msg.location_poiname.clone().and_then(opt);
                m.x = msg.location_label.clone().and_then(opt);
                m.la = msg.location_lat;
                m.lo = msg.location_lng;
            }
            Kind::Call => {
                let text = self.html_message_text(msg, x);
                m.v = Some(u8::from(text.contains("视频")));
                let rest = text
                    .strip_prefix('[')
                    .and_then(|t| t.split_once("] "))
                    .map_or("", |(_, r)| r.trim());
                // a length on its own says what it is
                m.x = if rx(r"^\d+(:\d{2}){1,2}$").is_match(rest) {
                    Some(format!("{}{rest}", chrome("通话时长 ", "Duration ")))
                } else {
                    opt(rest.to_string())
                };
            }
            Kind::Text => m.x = Some(self.html_message_text(msg, x)),
            _ => {}
        }
        m
    }

    /// An app message (or anything else), by the kind its XML names.
    fn app_msg(&mut self, mut m: PageMsg, msg: &ExportMsg, x: &MsgXml) -> PageMsg {
        let Some(meta) = extract_arkme_app_message_meta_in(x, msg.local_type) else {
            m.k = Kind::Text as u8;
            m.x = Some(self.html_message_text(msg, x));
            return m;
        };
        let n = x.once();
        let tag = |name: &str| opt(extract_xml_value(n, name));
        let title = tag("title");
        let kind = meta.get("appMsgKind").and_then(Value::as_str).unwrap_or("");
        let app_type = meta.get("appMsgType").and_then(Value::as_str).unwrap_or("");
        let kind = match (kind, app_type) {
            (_, "8") => Kind::Sticker,
            // a video link (4) is a link too, and a chat history can be of app type 24
            (_, "4") => Kind::Link,
            (_, "24") => Kind::History,
            ("file", _) => Kind::File,
            ("chat-record", _) => Kind::History,
            ("transfer", _) => Kind::Transfer,
            ("red-packet", _) => Kind::RedPacket,
            ("music", _) => Kind::Music,
            ("miniapp", _) => Kind::MiniProgram,
            ("finder", _) => Kind::Channels,
            ("announcement", _) => Kind::Announcement,
            ("link", _) => Kind::Link,
            _ => Kind::App,
        };
        m.k = kind as u8;
        match kind {
            Kind::File => {
                m.n = msg.file_name.clone().and_then(opt).or(title);
                m.z = msg.file_size;
                m.e = msg.file_ext.clone().and_then(opt);
            }
            Kind::History => {
                m.ti = title;
                m.it = msg.chat_record_list.as_ref().map(|items| {
                    items
                        .iter()
                        .map(|r| {
                            let time = r.sourcetime.trim();
                            let time = match time.parse::<i64>() {
                                Ok(t) => Value::from(t),
                                Err(_) => Value::from(time),
                            };
                            let text = r
                                .datadesc
                                .clone()
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| record_tag(r.datatype).to_string());
                            (r.sourcename.clone(), time, text)
                        })
                        .collect()
                });
            }
            Kind::Transfer => {
                m.a = tag("feedesc")
                    .and_then(|f| rx(r"[\d.,]+").find(&f).map(|a| a.as_str().to_string()));
                m.st = tag("paysubtype").and_then(|s| s.parse().ok()).or(Some(1));
                m.x = tag("pay_memo").or_else(|| self.transfer_desc(&msg.content));
            }
            Kind::RedPacket => m.ti = tag("receivertitle").or_else(|| tag("sendertitle")),
            Kind::Music => {
                m.ti = meta_str(&meta, "musicTitle").or(title);
                m.x = meta_str(&meta, "musicSinger").or_else(|| meta_str(&meta, "appMsgDesc"));
                m.l = meta_str(&meta, "musicUrl");
            }
            Kind::MiniProgram => {
                m.ti = title;
                m.f = meta_str(&meta, "appMsgSourceName")
                    .or_else(|| meta_str(&meta, "appMsgAppName"));
            }
            Kind::Channels => {
                m.ti = meta_str(&meta, "finderTitle").or(title);
                m.f = meta_str(&meta, "finderNickname").or_else(|| tag("nickname"));
            }
            Kind::Announcement => {
                m.x = tag("textannouncement").or_else(|| meta_str(&meta, "appMsgDesc"))
            }
            Kind::Link => {
                m.ti = meta_str(&meta, "linkTitle").or(title);
                m.x = meta_str(&meta, "appMsgDesc");
                m.l = meta_str(&meta, "linkUrl").or_else(|| tag("url"));
                m.f = meta_str(&meta, "appMsgSourceName");
            }
            Kind::Sticker => {}
            _ => {
                m.ti = title.or_else(|| opt(self.html_message_text(msg, x)));
                m.x = meta_str(&meta, "appMsgDesc");
                m.l = tag("url");
                m.f = meta_str(&meta, "appMsgSourceName")
                    .or_else(|| meta_str(&meta, "appMsgAppName"));
            }
        }
        m
    }
}

/// A quote (app type 57) of a system message, which the quote reading leaves out: the quoted text is the system
/// message's readable line.
fn quote_of_system(x: &MsgXml) -> Option<QuotedDisplay> {
    if x.app_type_twice() != "57" {
        return None;
    }
    let meta = extract_arkme_app_message_meta_in(x, 49)?;
    let quoted = meta_str(&meta, "quotedContent")?;
    if !quoted.contains("<sysmsg") {
        return None;
    }
    // a revoke says what happened in its replacemsg
    let line = opt(extract_xml_value(&quoted, "replacemsg"))
        .unwrap_or_else(|| clean_system_message(&quoted));
    Some(QuotedDisplay {
        reply_text: extract_xml_value(x.once(), "title"),
        quoted_sender: meta_str(&meta, "quotedSender"),
        quoted_preview: line,
    })
}

/// A relative path as a URL path: the ASCII characters a URL path cannot hold as they are (a space, `%`, `#`, `?`,
/// quotes, angle brackets, backslashes and control characters) percent-encoded; other text, Chinese included, kept,
/// which a page's UTF-8 carries.
pub(super) fn url_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if c.is_ascii_control() || " %#?\"'<>\\`{}|^".contains(c) {
            out.push_str(&format!("%{:02X}", c as u32));
        } else {
            out.push(c);
        }
    }
    out
}

/// The tag of a chat history's item that has no text of its own, by its data type.
fn record_tag(datatype: i64) -> &'static str {
    match datatype {
        2 => "[图片]",
        3 => "[语音]",
        4 => "[视频]",
        5 => "[链接]",
        8 => "[文件]",
        17 => "[聊天记录]",
        _ => "",
    }
}

// ───────────────────────────── sizes from media headers ─────────────────────────────

/// How much of a picture is read for its size: a JPEG's frame header comes after its metadata, which a camera's
/// thumbnail can make long.
const IMAGE_HEAD: u64 = 256 * 1024;

/// [width, height] of a PNG, GIF, WebP or JPEG from its header.
fn image_size(path: &Path) -> Option<(u32, u32)> {
    let mut b = Vec::new();
    File::open(path)
        .ok()?
        .take(IMAGE_HEAD)
        .read_to_end(&mut b)
        .ok()?;
    image_size_of(&b)
}

fn image_size_of(b: &[u8]) -> Option<(u32, u32)> {
    let u16be = |i: usize| {
        b.get(i..i + 2)
            .map(|s| u32::from(u16::from_be_bytes([s[0], s[1]])))
    };
    let u16le = |i: usize| {
        b.get(i..i + 2)
            .map(|s| u32::from(u16::from_le_bytes([s[0], s[1]])))
    };
    let u32be = |i: usize| {
        b.get(i..i + 4)
            .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    };
    let u24le = |i: usize| {
        b.get(i..i + 3)
            .map(|s| u32::from(s[0]) | u32::from(s[1]) << 8 | u32::from(s[2]) << 16)
    };
    if b.starts_with(b"\x89PNG") {
        return Some((u32be(16)?, u32be(20)?));
    }
    if b.starts_with(b"GIF") {
        return Some((u16le(6)?, u16le(8)?));
    }
    if b.starts_with(b"RIFF") && b.get(8..12) == Some(b"WEBP") {
        return match b.get(12..16)? {
            b"VP8 " => Some((u16le(26)? & 0x3fff, u16le(28)? & 0x3fff)),
            b"VP8L" => {
                let v = u32::from_le_bytes(b.get(21..25)?.try_into().ok()?);
                Some(((v & 0x3fff) + 1, ((v >> 14) & 0x3fff) + 1))
            }
            b"VP8X" => Some((u24le(24)? + 1, u24le(27)? + 1)),
            _ => None,
        };
    }
    if b.starts_with(&[0xff, 0xd8]) {
        let mut i = 2;
        while i + 9 < b.len() {
            if b[i] != 0xff {
                i += 1;
                continue;
            }
            let marker = b[i + 1];
            // a frame header (SOF0..SOF15, not DHT, JPG or DAC)
            if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                return Some((u16be(i + 7)?, u16be(i + 5)?));
            }
            i += 2 + u16be(i + 2)? as usize;
        }
    }
    None
}

/// Size and length of a video.
#[derive(Debug, PartialEq)]
struct VideoInfo {
    width: u32,
    height: u32,
    seconds: i64,
}

/// How much of an MP4's `moov` box is read: it holds the tables of every sample, a few megabytes for a long video.
const MOOV_MAX: u64 = 32 * 1024 * 1024;

/// [`VideoInfo`] of an MP4 from its `moov` box, wherever in the file it is.
fn mp4_info(path: &Path) -> Option<VideoInfo> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut at = 0u64;
    while at + 8 <= len {
        let mut head = [0u8; 16];
        f.seek(SeekFrom::Start(at)).ok()?;
        f.read_exact(&mut head[..8]).ok()?;
        let mut size = u64::from(u32::from_be_bytes(head[..4].try_into().ok()?));
        let mut header = 8;
        if size == 1 {
            f.read_exact(&mut head[8..]).ok()?;
            size = u64::from_be_bytes(head[8..].try_into().ok()?);
            header = 16;
        } else if size == 0 {
            size = len - at;
        }
        if size < header {
            return None;
        }
        if &head[4..8] == b"moov" {
            let mut moov = Vec::new();
            f.seek(SeekFrom::Start(at)).ok()?;
            f.take(size.min(MOOV_MAX)).read_to_end(&mut moov).ok()?;
            return mp4_info_of(&moov);
        }
        at += size;
    }
    None
}

/// [`VideoInfo`] from the bytes of a `moov` box: the first track with a size, and the movie's length.
fn mp4_info_of(b: &[u8]) -> Option<VideoInfo> {
    let u32at = |i: usize| {
        b.get(i..i + 4)
            .map(|s| u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    };
    let find = |from: usize, what: &[u8]| {
        b.get(from..)?
            .windows(4)
            .position(|w| w == what)
            .map(|p| p + from)
    };
    let (mut width, mut height) = (0, 0);
    let mut from = 0;
    while let Some(i) = find(from, b"tkhd") {
        from = i + 4;
        let Some(size) = i.checked_sub(4).and_then(u32at) else {
            continue;
        };
        let end = i - 4 + size as usize;
        if size < 84 || end > b.len() {
            continue;
        }
        let (w, h) = (u32at(end - 8)? >> 16, u32at(end - 4)? >> 16);
        if w > 0 && h > 0 {
            (width, height) = (w, h);
            break;
        }
    }
    if width == 0 {
        return None;
    }
    let mut seconds = 0;
    if let Some(mv) = find(0, b"mvhd") {
        let v1 = b.get(mv + 4) == Some(&1);
        let scale = u32at(mv + if v1 { 24 } else { 16 })?;
        let length = if v1 {
            b.get(mv + 28..mv + 36)
                .map(|s| u64::from_be_bytes(s.try_into().unwrap_or_default()))?
        } else {
            u64::from(u32at(mv + 20)?)
        };
        if scale > 0 {
            seconds = i64::try_from(length / u64::from(scale)).unwrap_or(0);
        }
    }
    Some(VideoInfo {
        width,
        height,
        seconds,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_paths_keep_their_text() {
        assert_eq!(
            url_path("media/群聊 #1/images/a%b?.jpg"),
            "media/群聊%20%231/images/a%25b%3F.jpg"
        );
        assert_eq!(
            url_path("media/chat/voices/voice_4.wav"),
            "media/chat/voices/voice_4.wav"
        );
    }

    #[test]
    fn image_sizes_from_headers() {
        let mut png = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
        png.extend_from_slice(&640u32.to_be_bytes());
        png.extend_from_slice(&480u32.to_be_bytes());
        assert_eq!(image_size_of(&png), Some((640, 480)));
        let gif = [b"GIF89a".as_slice(), &[0x20, 0x01, 0x10, 0x00]].concat();
        assert_eq!(image_size_of(&gif), Some((288, 16)));
        // a JPEG: an APP0 segment, then a baseline frame header
        let jpeg = [
            [0xff, 0xd8, 0xff, 0xe0, 0x00, 0x04, 0x00, 0x00].as_slice(),
            &[0xff, 0xc0, 0x00, 0x11, 0x08, 0x01, 0xe0, 0x02, 0x80, 0x03],
        ]
        .concat();
        assert_eq!(image_size_of(&jpeg), Some((640, 480)));
        assert_eq!(image_size_of(b"not a picture"), None);
        assert_eq!(image_size_of(&png[..18]), None, "cut short");
    }

    #[test]
    fn video_size_and_length_from_moov() {
        // mvhd (version 0): time scale 1000, duration 6500; tkhd (version 0, 92 bytes) ending in 360 x 640
        let mut mvhd = vec![0u8; 108];
        mvhd[..4].copy_from_slice(&108u32.to_be_bytes());
        mvhd[4..8].copy_from_slice(b"mvhd");
        mvhd[20..24].copy_from_slice(&1000u32.to_be_bytes());
        mvhd[24..28].copy_from_slice(&6500u32.to_be_bytes());
        let mut tkhd = vec![0u8; 92];
        tkhd[..4].copy_from_slice(&92u32.to_be_bytes());
        tkhd[4..8].copy_from_slice(b"tkhd");
        tkhd[84..88].copy_from_slice(&(360u32 << 16).to_be_bytes());
        tkhd[88..92].copy_from_slice(&(640u32 << 16).to_be_bytes());
        let body = [mvhd, tkhd].concat();
        let moov = [
            u32::try_from(body.len() + 8)
                .unwrap()
                .to_be_bytes()
                .as_slice(),
            b"moov",
            &body,
        ]
        .concat();
        assert_eq!(
            mp4_info_of(&moov),
            Some(VideoInfo {
                width: 360,
                height: 640,
                seconds: 6
            })
        );
        // in a file, after the media data
        let dir = weflow_native::fixture::TestDir::new("weflow-mp4");
        let path = dir.join("v.mp4");
        let mdat = [24u32.to_be_bytes().as_slice(), b"mdat", &[7u8; 16]].concat();
        std::fs::write(&path, [mdat.as_slice(), &moov].concat()).unwrap();
        assert_eq!(mp4_info(&path).map(|v| v.width), Some(360));
    }
}
