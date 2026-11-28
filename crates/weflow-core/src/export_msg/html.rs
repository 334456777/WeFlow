//! HTML 导出：一个页面，即按命令行的语言选择的模板，聊天记录以 JSON 写在其中。
//!
//! 模板是这个文件旁边的 `template.html`，由 `build.mjs` 压缩成 `template.zh.html` 和 `template.en.html`，
//! 嵌入这里。模板里有三个占位符：`__WF_LANG__`、`__WF_TITLE__` 和 `__WF_DATA__`，最后一个在
//! `<script type="application/json">` 元素里。
//!
//! 数据（版本 1）：
//!
//! ```text
//! { v: 1, lang: "zh" | "en", title, kind: "group" | "private" | "official", me, exported, users, msgs }
//!   me: 用户自己在 users 中的下标，-1 表示没有
//!   exported: 导出的时间，Unix 时间（秒）
//!   users: [[name, avatar]]，即名字和头像路径，没有头像时 avatar 为 ""
//! msgs[i] = { t, u, k, ...该种类的字段 }
//!   t: Unix 时间（秒）；u: 在 users 中的下标，-1 表示没有；k: 种类，各种类的编号和字段如下
//!   0 系统消息 {x}  1 文字 {x}  2 图片 {s,w,h}  3 语音 {s,d}  4 视频 {s,w,h,d}  5 表情 {s,w,h}
//!   6 文件 {n,z,e,s}  7 链接 {ti,x,l,f}  8 引用 {x,qn,qx,r | rq}  9 聊天记录 {ti,it: [[name, time, text]]}
//!   10 转账 {a,x,st}  11 红包 {ti}  12 位置 {ti,x,la,lo}  13 名片 {n}  14 通话 {v,x}
//!   15 音乐 {ti,x,l}  16 小程序 {ti,f}  17 视频号 {ti,f}  18 群公告 {x}  19 其他应用消息 {ti,x,l,f}
//! ```
//!
//! `s` 是媒体文件相对页面的路径，没有这个字段表示文件不在；`w`、`h` 是文件头里给出的尺寸，没有给出时省略。
//! 引用的 `r` 是被引用消息的下标；被引用的消息在导出中排在后面时，引用改用 `rq` 指明它，那条消息带着相同的 `id`。
//! 消息按导出的顺序排列；页面按打开它的电脑所在的时区显示时间。

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::Serialize;
use serde_json::{Map, Value};

use super::format::{html_escape, write_script_json};
use super::spool::Spool;
use super::{chrome, Exporter};
use crate::locale;
use crate::message::appmsg::extract_arkme_app_message_meta_in;
use crate::message::forward::RecordKind;
use crate::message::quote::QuotedDisplay;
use crate::message::system::clean_system_message;
use crate::message::xml::{decode_html_entities, extract_xml_value, rx, xml_text, MsgXml};
use crate::message::ExportMsg;

const TEMPLATE_ZH: &str = include_str!("template.zh.html");
const TEMPLATE_EN: &str = include_str!("template.en.html");

/// 消息的种类（`k`），编号与页面上的相同。
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

/// 页面数据中的一条消息；这种消息没有的字段省略。
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

/// 页面数据需要从前面的消息中得到的信息，在读取消息时收集。
#[derive(Default)]
struct PageState {
    /// 页面所在的文件夹，媒体路径相对它计算。
    dir: PathBuf,
    /// 每个发送者的名字和头像路径，按第一次出现的顺序排列。
    users: Vec<(String, String)>,
    /// 发送者在 `users` 中的下标，键为 [`user_key`]。
    user_ix: HashMap<String, usize>,
    /// 按服务器 ID 查消息的下标。
    ids: HashMap<u64, usize>,
    /// 在消息出现之前就被引用的服务器 ID。
    wanted: HashSet<u64>,
}

/// 消息的发送者在页面的 users 中的键：用户自己为 ""。
fn user_key<'m>(ex: &Exporter<'_, '_>, msg: &'m ExportMsg) -> &'m str {
    if msg.is_send {
        ""
    } else if ex.session.is_group {
        &msg.sender_username
    } else {
        // 私聊的对方；任何不是 "" 的键都可以
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
    /// 用已在内存中的消息生成页面，测试拿流式写出的文件与它比较。
    #[cfg(test)]
    pub(super) fn render_html(&mut self, msgs: &[ExportMsg]) -> String {
        let mut state = PageState::default();
        let mut data: Vec<u8> = Vec::new();
        for (i, msg) in msgs.iter().enumerate() {
            if i > 0 {
                data.push(b',');
            }
            let item = self.page_msg(&mut state, msg, i);
            // 写进 Vec 不会失败
            let _ = write_script_json(&mut data, &item);
        }
        let mut page = Vec::new();
        let _ = self.write_page(&mut page, &state, |w| w.write_all(&data));
        String::from_utf8(page).unwrap_or_default()
    }

    /// 从消息流写出 HTML：每条消息读到后，它的数据写进暂存文件；页面的其余部分（数据以发送者开头）最后再写。
    /// 没有消息时不写文件，除非 `write_if_empty` 为真。
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
        let mut w = super::create_output(out)?;
        self.write_page(&mut w, &state, |w| {
            spool
                .copy_to(w)
                .map_err(|e| io::Error::other(format!("{e:#}")))
        })
        .with_context(|| format!("failed to write {}", out.display()))?;
        w.flush()?;
        Ok(count)
    }

    /// 把 `msgs` 写成一个 HTML 页面 `out`，没有消息时也写。
    ///
    /// # Errors
    ///
    /// 无法写入页面或它的暂存文件时返回错误。
    pub fn write_html(&mut self, msgs: &[ExportMsg], out: &Path) -> Result<()> {
        self.stream_html(msgs, out, true)?;
        Ok(())
    }

    /// 写出页面：填好语言和标题的模板，以及数据，其中的消息由 `write_msgs` 写出（以逗号分隔）。
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
        let me = state
            .user_ix
            .get("")
            .map_or(-1, |&i| crate::num::count_i64(i));
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

    /// 发送者在页面 users 中的下标；发送者第一次出现时把它加进去。
    fn page_user(&mut self, state: &mut PageState, msg: &ExportMsg) -> i64 {
        let key = user_key(self, msg);
        if let Some(&i) = state.user_ix.get(key) {
            return crate::num::count_i64(i);
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
        crate::num::count_i64(i)
    }

    /// 页面数据中的一条消息；`index` 是它前面的消息数。
    fn page_msg(&mut self, state: &mut PageState, msg: &ExportMsg, index: usize) -> PageMsg {
        let x = MsgXml::of_export(msg);
        let mut m = PageMsg {
            t: msg.create_time,
            ..Default::default()
        };
        // 在它出现之前就被引用的消息带上自己的 ID
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
        // 先看是不是引用：引用系统消息时，引用里是那条系统消息的 XML；拍一拍（app 类型 62）显示为一行系统消息
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
            // 系统消息的文字带着 XML 里的实体，这里解码；拍一拍的文字已经解码过（#1196）
            let text = self.html_message_text(msg, &x);
            m.x = Some(if x.app_type_twice() == "62" {
                text
            } else {
                decode_html_entities(&text)
            });
            return m;
        }
        m.u = self.page_user(state, msg);
        if let Some(q) = quoted {
            m.k = Kind::Quote as u8;
            // 被引用的消息显示在回复下方，回复的文字里不再重复
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
                // 文件名、大小和扩展名取自 app 消息，与文件没有复制时相同
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

    /// 由 `local_type` 确定种类的消息：填好这个种类的字段。
    fn fill_kind(
        &mut self,
        mut m: PageMsg,
        kind: Kind,
        msg: &ExportMsg,
        x: &MsgXml<'_>,
    ) -> PageMsg {
        m.k = kind as u8;
        match kind {
            Kind::Voice => {
                m.d = crate::chat_msg::parse_voice_duration_seconds(&msg.content).or(Some(1));
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
                // 只有时长的文字前面加上「通话时长」
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

    /// app 消息（或其他消息）：按 XML 里写的种类填写字段。
    fn app_msg(&mut self, mut m: PageMsg, msg: &ExportMsg, x: &MsgXml<'_>) -> PageMsg {
        let Some(meta) = extract_arkme_app_message_meta_in(x, msg.local_type) else {
            m.k = Kind::Text as u8;
            m.x = Some(self.html_message_text(msg, x));
            return m;
        };
        let n = x.once();
        let tag = |name: &str| opt(xml_text(n, name));
        let title = tag("title");
        let kind = meta.get("appMsgKind").and_then(Value::as_str).unwrap_or("");
        let app_type = meta.get("appMsgType").and_then(Value::as_str).unwrap_or("");
        let kind = match (kind, app_type) {
            (_, "8") => Kind::Sticker,
            // 视频链接（4）也是链接；app 类型 24 也可能是聊天记录
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
            ("link" | "official-link", _) => Kind::Link,
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
                // 没有自己的播放地址时用歌曲页面，微信保存的歌曲大多是这样（#1170）
                m.l = meta_str(&meta, "musicUrl").or_else(|| tag("url"));
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
                m.x = tag("textannouncement").or_else(|| meta_str(&meta, "appMsgDesc"));
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

/// 引用了系统消息的引用（app 类型 57），读取引用的通用逻辑不处理这种：被引用的文字取那条系统消息可读的一行。
fn quote_of_system(x: &MsgXml<'_>) -> Option<QuotedDisplay> {
    if x.app_type_twice() != "57" {
        return None;
    }
    let meta = extract_arkme_app_message_meta_in(x, 49)?;
    let quoted = meta_str(&meta, "quotedContent")?;
    if !quoted.contains("<sysmsg") {
        return None;
    }
    // 撤回消息的 replacemsg 里写着发生了什么
    let line = opt(extract_xml_value(&quoted, "replacemsg"))
        .unwrap_or_else(|| clean_system_message(&quoted));
    Some(QuotedDisplay {
        reply_text: extract_xml_value(x.once(), "title"),
        quoted_sender: meta_str(&meta, "quotedSender"),
        quoted_preview: line,
    })
}

/// 把相对路径写成 URL 路径：URL 路径里不能原样出现的 ASCII 字符（空格、`%`、`#`、`?`、引号、尖括号、反斜杠、
/// 反引号、`{`、`}`、`|`、`^` 和控制字符）写成百分号编码；其他文字（包括中文）原样保留，页面的 UTF-8 能承载它们。
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

/// 聊天记录中没有文字的一项按数据类型显示的标签。
fn record_tag(datatype: i64) -> &'static str {
    match RecordKind::of(datatype) {
        // 页面的英文表里是「[语音]」，不用 `tag()` 的「[语音消息]」
        RecordKind::Voice => "[语音]",
        RecordKind::Text | RecordKind::Other => "",
        kind => kind.tag(),
    }
}

// ───────────────────────────── 从媒体文件头读尺寸 ─────────────────────────────

/// 为取图片尺寸读取的字节数：JPEG 的帧头在元数据之后，相机生成的缩略图能让元数据很长。
const IMAGE_HEAD: u64 = 256 * 1024;

/// 从文件头读出 PNG、GIF、WebP 或 JPEG 的宽和高。
fn image_size(path: &Path) -> Option<(u32, u32)> {
    let mut b = Vec::new();
    File::open(path)
        .ok()?
        .take(IMAGE_HEAD)
        .read_to_end(&mut b)
        .ok()?;
    image_size_of(&b)
}

/// 图片开头的字节 `b` 里写着的宽和高；不是认得的格式或字节不够时为 `None`。
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
            // 帧头（SOF0 到 SOF15，不含 DHT、JPG 和 DAC）
            if (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc) {
                return Some((u16be(i + 7)?, u16be(i + 5)?));
            }
            i += 2 + u16be(i + 2)? as usize;
        }
    }
    None
}

/// 视频的尺寸和时长。
#[derive(Debug, PartialEq)]
struct VideoInfo {
    width: u32,
    height: u32,
    seconds: i64,
}

/// 读取 MP4 的 `moov` box 的上限：它保存每个采样的表，长视频的有几 MB。
const MOOV_MAX: u64 = 32 * 1024 * 1024;

/// 从 MP4 的 `moov` box 读出 [`VideoInfo`]，不论它在文件的什么位置。按文件给出的大小逐个走过各个 box（大小为 0
/// 表示最后一个 box，一直到文件末尾），每个至少和它的头一样长，所以每一步都向前走；
/// 大小比头短，或者位置超出 u64 的上限时停止。
fn mp4_info(path: &Path) -> Option<VideoInfo> {
    let mut f = File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let mut at = 0u64;
    while at.checked_add(8).is_some_and(|head_end| head_end <= len) {
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
        at = at.checked_add(size)?;
    }
    None
}

/// 从 `moov` box 的字节读出 [`VideoInfo`]：第一个有尺寸的轨道，以及影片的时长。
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

    /// 页面上聊天记录各项的标签，按记录自己的 `datatype` 编号确定（#1171）。
    #[test]
    fn a_record_item_is_tagged_by_its_datatype() {
        for (datatype, tag) in [
            (1, ""),
            (2, "[图片]"),
            (3, "[语音]"),
            (4, "[视频]"),
            (5, "[链接]"),
            (8, "[文件]"),
            (17, "[聊天记录]"),
            (19, "[小程序]"),
            (37, "[表情]"),
            (34, ""),
        ] {
            assert_eq!(record_tag(datatype), tag, "{datatype}");
        }
    }

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
        // JPEG：一个 APP0 段，然后是基线帧头
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
        // mvhd（版本 0）：时间刻度 1000，时长 6500；tkhd（版本 0，92 字节）末尾是 360 x 640
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
        // 文件里的 moov 在媒体数据之后
        let dir = weflow_native::fixture::TestDir::new("weflow-mp4");
        let path = dir.join("v.mp4");
        let mdat = [24u32.to_be_bytes().as_slice(), b"mdat", &[7u8; 16]].concat();
        std::fs::write(&path, [mdat.as_slice(), &moov].concat()).unwrap();
        assert_eq!(mp4_info(&path).map(|v| v.width), Some(360));
    }

    #[test]
    fn a_box_size_that_wraps_around_ends_the_walk() {
        let dir = weflow_native::fixture::TestDir::new("weflow-mp4-bad");
        let path = dir.join("bad.mp4");
        let free = [16u32.to_be_bytes().as_slice(), b"free", &[0u8; 8]].concat();
        // 偏移量 16 处是一个 64 位的大小，会让下一个 box 回到偏移量 0
        let back_to_start = (u64::MAX - 15).to_be_bytes();
        let wrapping = [1u32.to_be_bytes().as_slice(), b"free", &back_to_start].concat();
        std::fs::write(&path, [free.as_slice(), &wrapping, &[0u8; 16]].concat()).unwrap();
        assert_eq!(mp4_info(&path), None);
        // 比自己的头还短的 64 位大小
        let short = [1u32.to_be_bytes().as_slice(), b"free", &8u64.to_be_bytes()].concat();
        std::fs::write(&path, [short.as_slice(), &[0u8; 16]].concat()).unwrap();
        assert_eq!(mp4_info(&path), None);
    }
}
