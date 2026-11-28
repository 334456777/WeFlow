//! 读取微信的 XML：正则的缓存、标签和属性的值、解析过的 `<msg>`、发送者前缀，以及被引用内容中可能带的 wxid。

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use regex::Regex;

use super::quote::QuoteInfo;
use super::quote::QuotedDisplay;
use super::quote::{format_quoted_reference_preview, quote_info_of, refermsg_xml};
use super::row::unsigned_token;
use super::system::has_sysmsg;

thread_local! {
    static RX_CACHE: RefCell<HashMap<String, Rc<Regex>>> = RefCell::new(HashMap::new());
}

/// 编译正则并缓存。这里的模式都是固定的或经过转义的，编译失败就是 bug。
///
/// 缓存交出共享的 `Rc<Regex>`：`Regex::clone` 会给克隆出的正则一个全新的（冷的）搜索缓存，
/// 用克隆时每次调用都要重新构建惰性 DFA，这会占去导出的大部分时间。
///
/// # Panics
///
/// `pattern` 不是合法的正则时。
pub fn rx(pattern: &str) -> Rc<Regex> {
    RX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(found) = cache.get(pattern) {
            return found.clone();
        }
        let compiled =
            Rc::new(Regex::new(pattern).unwrap_or_else(|e| panic!("bad regex {pattern}: {e}")));
        cache.insert(pattern.to_string(), compiled.clone());
        compiled
    })
}

/// `xml` 中第一个 `<tag>` 里的原始内容，标签不区分大小写。
fn tag_inside<'x>(xml: &'x str, tag: &str) -> Option<&'x str> {
    if tag.is_ascii() {
        crate::xml::tag_inner(xml, tag)
    } else {
        let re = rx(&format!(r"(?is)<{0}>(.*?)</{0}>", regex::escape(tag)));
        re.captures(xml)
            .map(|c| c.get(1).map_or("", |m| m.as_str()))
    }
}

/// `xml` 中第一个 `<tag>` 里的内容，去掉首尾空白和 CDATA 标记，实体保持原样。用于本身是 XML 的值（引用的
/// `<content>`、`<recorditem>`），由读它的地方解码；给人读的文字用 [`xml_text`]。
pub fn extract_xml_value(xml: &str, tag: &str) -> String {
    tag_inside(xml, tag).map_or_else(String::new, strip_cdata)
}

/// `xml` 中第一个 `<tag>` 给人读的文字（#1199）：去掉首尾空白，
/// CDATA 之外的实体解码一次（[`decode_text_entities`]），CDATA 里的文字原样保留。
pub(crate) fn xml_text(xml: &str, tag: &str) -> String {
    let Some(inside) = tag_inside(xml, tag) else {
        return String::new();
    };
    let mut text = String::with_capacity(inside.len());
    let mut rest = inside;
    while let Some(open) = rest.find("<![CDATA[") {
        text.push_str(&decode_text_entities(&rest[..open].replace("]]>", "")));
        let body = &rest[open + "<![CDATA[".len()..];
        let (cdata, after) = body.split_once("]]>").unwrap_or((body, ""));
        text.push_str(cdata);
        rest = after;
    }
    text.push_str(&decode_text_entities(&rest.replace("]]>", "")));
    text.trim().to_string()
}

/// 一遍解码 XML 文字中的实体，解码得到的内容不再参与解码（`&amp;lt;` 得到 `&lt;`）：包括
/// [`crate::xml::decode_entities`] 认识的实体和数字实体（`&#233;`、`&#xE9;`）。`&` 后面的其他内容原样保留。
pub(crate) fn decode_text_entities(s: &str) -> std::borrow::Cow<'_, str> {
    use std::borrow::Cow;
    if !s.contains('&') {
        return Cow::Borrowed(s);
    }
    // 两种实体中最长的是 `&#x10FFFF;`
    const LONGEST: usize = 9;
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let end = after.as_bytes()[..after.len().min(LONGEST)]
            .iter()
            .position(|&b| b == b';');
        let decoded = end.and_then(|end| {
            let name = &after[..end];
            match name.strip_prefix('#') {
                Some(number) => match number.strip_prefix(['x', 'X']) {
                    Some(hex) if hex.bytes().all(|b| b.is_ascii_hexdigit()) => {
                        u32::from_str_radix(hex, 16).ok()
                    }
                    Some(_) => None,
                    None if number.bytes().all(|b| b.is_ascii_digit()) => number.parse().ok(),
                    None => None,
                }
                .and_then(char::from_u32),
                // 认识的名字解码成对应的一个字符，其他的保持原样
                None => match crate::xml::decode_entities(&rest[amp..amp + end + 2]) {
                    Cow::Owned(c) if c.chars().count() == 1 => c.chars().next(),
                    _ => None,
                },
            }
            .map(|c| (c, end))
        });
        rest = match decoded {
            Some((c, end)) => {
                out.push(c);
                &after[end + 1..]
            }
            None => {
                out.push('&');
                after
            }
        };
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// 相当于 `c.replace("<![CDATA[", "").replace("]]>", "").trim()`，没有要去掉的标记时省去中间的复制。
pub(super) fn strip_cdata(c: &str) -> String {
    use crate::xml::replace_cow;
    use std::borrow::Cow;
    replace_cow(replace_cow(Cow::Borrowed(c), "<![CDATA[", ""), "]]>", "")
        .trim()
        .to_string()
}

/// `xml` 中第一个 `<tag>` 的属性 `attr`，解码成给人读的文字：实体解码一次（#1199）。
pub(crate) fn xml_attr_text(xml: &str, tag: &str, attr: &str) -> String {
    decode_text_entities(&extract_xml_attribute(xml, tag, attr)).into_owned()
}

/// `xml` 中第一个 `<tag>` 的属性 `attr` 的原文（双引号里的值），实体保持原样；没有时为空字符串。
pub(super) fn extract_xml_attribute(xml: &str, tag: &str, attr: &str) -> String {
    if !tag.is_empty() && tag.is_ascii() && !attr.is_empty() && attr.is_ascii() {
        return crate::xml::attribute(xml, tag, attr)
            .unwrap_or_default()
            .to_string();
    }
    let re = rx(&format!(
        r#"(?i)<{}\s+[^>]*{}\s*=\s*"([^"]*)""#,
        regex::escape(tag),
        regex::escape(attr)
    ));
    re.captures(xml)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// 解码 `text` 中的实体，规则见 [`crate::xml::decode_entities`]。
pub(crate) fn decode_html_entities(text: &str) -> String {
    decode_html_entities_cow(text).into_owned()
}

/// 没有 `&` 时不复制的 [`decode_html_entities`]。
pub(super) fn decode_html_entities_cow(text: &str) -> std::borrow::Cow<'_, str> {
    crate::xml::decode_entities(text)
}

/// 规范化消息内容：转义过的 XML 解开一层，见 [`normalize_app_message_cow`]。
pub(super) fn normalize_app_message_content(content: &str) -> String {
    normalize_app_message_cow(content).into_owned()
}

/// 内容中没有转义的 XML 时不复制的 [`normalize_app_message_content`]。
///
/// 同时含有 `&lt;` 和 `&gt;` 的内容视为转义过的 XML，解开一层：每个实体只解码一次，所以转义了两次的 XML
/// 得到转义一次的（[`MsgXml::twice`] 再解开这一层），其中的 `&amp;quot;` 得到 `&quot;`，不会在属性中间变成裸的
/// `"`（#922）。
pub(super) fn normalize_app_message_cow(content: &str) -> std::borrow::Cow<'_, str> {
    let has = |entity: &str| memchr::memmem::find(content.as_bytes(), entity.as_bytes()).is_some();
    if has("&lt;") && has("&gt;") {
        return crate::xml::decode_entities(content);
    }
    std::borrow::Cow::Borrowed(content)
}

/// `content` 规范化（[`normalize_app_message_cow`]）之后是否可能含有 `word`。规范化把每个实体换成 `<>&"'`
/// 中的一个，所以结果中不含这几个字符的一段在 `content` 里原样存在：`word` 不含这几个字符时，在规范化之前搜一遍
/// `content` 就能排除。
fn normalized_may_contain(content: &str, word: &str) -> bool {
    debug_assert!(!word.contains(['<', '>', '&', '"', '\'']));
    content.contains(word)
}

/// 导出读取的一条消息的 XML：几个函数都要从内容求出的几项（规范化后的文字、`<appmsg>` 类型、是否为系统消息、
/// 引用）在第一次被问到时求出，之后共用。
///
/// 有的函数读规范化一次的内容，有的读再规范化一次的（例如引用的文字从规范化后的内容再规范化一次后读取），
/// 两者只在转义了两次的 XML 上不同，所以两份都保留：`once` 是规范化一次的内容，`twice` 是再规范化一次的。
pub struct MsgXml<'a> {
    content: &'a str,
    once: std::cell::OnceCell<std::borrow::Cow<'a, str>>,
    /// `once` 再规范化一次的结果，只在有变化时保存。
    twice: std::cell::OnceCell<Option<String>>,
    app_type_twice: std::cell::OnceCell<String>,
    /// `once` 和 `twice` 中的 `<refermsg>` 块。
    refer_once: std::cell::OnceCell<Option<String>>,
    refer_twice: std::cell::OnceCell<Option<String>>,
    /// `once` 中的引用，从 `twice` 的 `<refermsg>` 读出。
    quote_twice: std::cell::OnceCell<QuoteInfo>,
    display: std::cell::OnceCell<Option<QuotedDisplay>>,
}

/// 只显示内容的长度，不显示消息本身。
impl std::fmt::Debug for MsgXml<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MsgXml")
            .field("content_len", &self.content.len())
            .finish_non_exhaustive()
    }
}

impl<'a> MsgXml<'a> {
    /// 消息内容 `content` 的 XML，各项在用到时才求出。
    pub fn new(content: &'a str) -> Self {
        Self {
            content,
            once: Default::default(),
            twice: Default::default(),
            app_type_twice: Default::default(),
            refer_once: Default::default(),
            refer_twice: Default::default(),
            quote_twice: Default::default(),
            display: Default::default(),
        }
    }

    /// 导出读取的消息的 XML：`<appmsg>` 类型不为空时，直接用读取时求出的
    /// [`ExportMsg::xml_type`](super::ExportMsg::xml_type)，各格式和各函数不再重新求。
    pub fn of_export(msg: &'a super::ExportMsg) -> Self {
        Self {
            app_type_twice: msg
                .xml_type
                .clone()
                .map(std::cell::OnceCell::from)
                .unwrap_or_default(),
            ..Self::new(&msg.content)
        }
    }

    /// 保存的原始内容。
    pub fn content(&self) -> &'a str {
        self.content
    }

    /// 规范化一次的内容（[`normalize_app_message_content`]）。
    pub fn once(&self) -> &str {
        self.once
            .get_or_init(|| normalize_app_message_cow(self.content))
    }

    /// [`once`](Self::once) 再规范化一次的内容。
    pub fn twice(&self) -> &str {
        let twice = self
            .twice
            .get_or_init(|| match normalize_app_message_cow(self.once()) {
                std::borrow::Cow::Borrowed(_) => None,
                std::borrow::Cow::Owned(s) => Some(s),
            });
        twice.as_deref().unwrap_or_else(|| self.once())
    }

    /// 对 [`once`](Self::once) 求 [`extract_app_message_type`]，即在再规范化一次的内容中读类型。
    pub(crate) fn app_type_twice(&self) -> &str {
        self.app_type_twice
            .get_or_init(|| app_type_in(self.once(), self.twice()))
    }

    /// 消息是否为文字可读的系统消息：类型为 10000，或者内容在发送者之后有 `<sysmsg>`。
    pub fn readable_system_once(&self, local_type: i64) -> bool {
        local_type == 10000 || has_sysmsg(self.once())
    }

    /// 对 [`once`](Self::once) 做 [`readable_system_once`](Self::readable_system_once) 的判断，
    /// 即看再规范化一次的内容。
    pub(crate) fn readable_system_twice(&self, local_type: i64) -> bool {
        local_type == 10000 || has_sysmsg(self.twice())
    }

    /// 内容中是否可能有引用（见 [`normalized_may_contain`]）。
    fn may_quote(&self) -> bool {
        normalized_may_contain(self.content, "refermsg")
    }

    /// [`once`](Self::once) 中的 `<refermsg>` 块。
    fn refer_once(&self) -> Option<&str> {
        self.refer_once
            .get_or_init(|| {
                if self.may_quote() {
                    refermsg_xml(self.once())
                } else {
                    None
                }
            })
            .as_deref()
    }

    /// [`twice`](Self::twice) 中的 `<refermsg>` 块。
    fn refer_twice(&self) -> Option<&str> {
        // 再规范化一次没有变化时，是同一个块
        if std::ptr::eq(self.twice(), self.once()) {
            return self.refer_once();
        }
        self.refer_twice
            .get_or_init(|| {
                if self.may_quote() {
                    refermsg_xml(self.twice())
                } else {
                    None
                }
            })
            .as_deref()
    }

    /// [`once`](Self::once) 中的引用（从 [`twice`](Self::twice) 的 `<refermsg>` 读出）：被引用的是谁、说了什么。
    pub(super) fn quote_twice(&self) -> &QuoteInfo {
        self.quote_twice.get_or_init(|| {
            self.refer_twice()
                .map_or_else(QuoteInfo::empty, quote_info_of)
        })
    }

    /// 内容所回复的消息，用来写回复的文字：回复本身的文字、被引用的人和被引用内容的预览；没有引用，
    /// 或者回复和预览都为空时为 `None`。
    pub fn quoted_display(&self) -> Option<&QuotedDisplay> {
        self.display
            .get_or_init(|| {
                let refer = self.refer_once()?;
                let info = self.quote_twice();
                let reply_text = strip_sender_prefix(&xml_text(self.once(), "title"));
                let preview = info.content.clone().unwrap_or_else(|| {
                    format_quoted_reference_preview(
                        &extract_xml_value(refer, "content"),
                        &extract_xml_value(refer, "type"),
                    )
                });
                if reply_text.is_empty() && preview.is_empty() {
                    return None;
                }
                Some(QuotedDisplay {
                    reply_text,
                    quoted_sender: info.sender.clone(),
                    quoted_preview: if preview.is_empty() {
                        "[消息]".into()
                    } else {
                        preview
                    },
                })
            })
            .as_ref()
    }

    /// 内容所回复的消息的服务器 ID（`<svrid>`）。
    pub fn reply_to_message_id(&self) -> Option<String> {
        unsigned_token(&extract_xml_value(self.refer_once()?, "svrid"))
    }

    /// 内容所引用的消息的发送者 wxid：取 `<chatusr>`，没有时取 `<fromusr>`，`<fromusr>` 是群时为 `None`。
    pub fn quoted_sender_username(&self) -> Option<String> {
        let refer = self.refer_once()?;
        let chat = extract_xml_value(refer, "chatusr");
        let from = extract_xml_value(refer, "fromusr");
        if !chat.trim().is_empty() {
            return Some(chat.trim().to_string());
        }
        if from.trim().ends_with("@chatroom") {
            return None;
        }
        let f = from.trim().to_string();
        if f.is_empty() {
            None
        } else {
            Some(f)
        }
    }
}

/// 去掉群消息开头的发送者（`wxid_xxx:` 和它后面的换行）后的 `content`；`http://` 和 `1:3-1:6` 这样的文字保持原样。
/// 导出和聊天模型共用这一条规则。
pub fn strip_sender_prefix(content: &str) -> String {
    content[sender_prefix_len(content)..].to_string()
}

/// 去掉发送者行后的 `content`（`sender` 发的消息的文字，#656）。只有群里别人发的消息以写着发送者的一行开头，
/// 其他文字的第一行（如 `PS:`）保留。这一行写的是 `sender`，或者发送者就是群本身（导出时没能解析出发送者的行，
/// 发送者记为会话）时去掉。不知道发送者（`None` 或为空）时同 [`strip_sender_prefix`]。
pub fn strip_sender_line(content: &str, sender: Option<&str>) -> String {
    let Some(sender) = sender.map(str::trim).filter(|s| !s.is_empty()) else {
        return strip_sender_prefix(content);
    };
    match sender_prefix(content) {
        Some((name, end)) if name.eq_ignore_ascii_case(sender) || sender.ends_with("@chatroom") => {
            content[end..].to_string()
        }
        _ => content.to_string(),
    }
}

/// [`strip_sender_prefix`] 去掉的发送者前缀的长度，规则见 [`sender_prefix`]；没有前缀时为 0。
pub(super) fn sender_prefix_len(content: &str) -> usize {
    sender_prefix(content).map_or(0, |(_, end)| end)
}

/// [`strip_sender_prefix`] 去掉的发送者前缀中的名字，以及其后文字开始的位置。前缀是开头的空白、名字（字母、数字和
/// `_@-`）和冒号，冒号后面紧跟换行，或者名字以 `wxid_`、`gh_` 开头，而且后面不是 `//`；
/// 相当于 `^\s*([A-Za-z0-9_@-]+):(\r?\n|)` 再加这两个条件。
fn sender_prefix(content: &str) -> Option<(&str, usize)> {
    // 群消息保存为 "wxid:\ntext"（企业微信成员的是 "…@openim:\ntext"）；一行里像 "1:3-1:6" 的是正文本身，
    // 不当作前缀去掉
    let h = content.as_bytes();
    let start = crate::xml::skip_ws(content, 0);
    let name_len = h[start..]
        .iter()
        .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'@' | b'-'))
        .count();
    let colon = start + name_len;
    if name_len == 0 || h.get(colon) != Some(&b':') {
        return None;
    }
    let rest = &h[colon + 1..];
    let newline = if rest.starts_with(b"\r\n") {
        2
    } else {
        usize::from(rest.starts_with(b"\n"))
    };
    let end = colon + 1 + newline;
    let name = &content[start..colon];
    let looks_like_id = name.starts_with("wxid_") || name.starts_with("gh_");
    ((newline > 0 || looks_like_id) && !content[end..].starts_with("//")).then_some((name, end))
}

/// 消息自己的 `<appmsg><type>`（先把内容规范化一次），读法见 [`app_type_in`]；没有时为空字符串。
pub(super) fn extract_app_message_type(content: &str) -> String {
    app_type_in(content, &normalize_app_message_cow(content))
}

/// 已有规范化后的 `normalized` 时，对 `content` 求 [`extract_app_message_type`]。
fn app_type_in(content: &str, normalized: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    // 消息自己的 XML：回复的 `<refermsg>`、拍一拍的 `<patmsg>` 和聊天记录的 `<recorditem>` 里是别的消息，
    // 各有自己的 `<type>`，被引用消息的 `</appmsg>` 还会让 `<appmsg>` 提前结束，
    // 所以找 `<appmsg>` 之前先去掉它们（#1241）
    let own = crate::xml::remove_blocks(normalized, "<refermsg", "</refermsg>");
    let own = crate::xml::remove_blocks(&own, "<patmsg", "</patmsg>");
    let own = crate::xml::remove_blocks(&own, "<recorditem", "</recorditem>");
    // 相当于 `(?is)<appmsg.*?>(.*?)</appmsg>`，再取其中第一个 `<type>`，微信写成 CDATA 的也算
    if let Some(c) = crate::xml::element_inner(&own, "<appmsg", "</appmsg>") {
        if let Some(t) = crate::xml::tag_inner(c, "type") {
            return strip_cdata(t);
        }
    }
    if !normalized.contains("<appmsg") && !normalized.contains("<msg>") {
        return String::new();
    }
    // 正则要有 `<type>` 才能匹配，而只有这几个字母的 ASCII 大小写折叠成它们：图片、语音和表情都没有 `<type>`；
    // 这里同样只读消息自己的 XML，因为被引用的消息有自己的 `<type>`（#1263）
    if crate::xml::find_ci(&own, 0, "<type>").is_none() {
        return String::new();
    }
    rx(r"(?i)<type>(\d+)</type>")
        .captures(&own)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

/// `text` 是否像 wxid（不区分大小写）：以 `wxid_` 开头，或者是 `wx` 加至少 4 个字母、数字、`_` 或 `-`。
pub(crate) fn looks_like_wxid(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return false;
    }
    // 相当于 `^wx[a-z0-9_-]{4,}$`
    let wx_id = t.len() >= 6
        && t.starts_with("wx")
        && t.bytes()
            .skip(2)
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'));
    t.starts_with("wxid_") || wx_id
}

/// 清理被引用的文字：去掉 wxid（`wxid_[A-Za-z0-9_-]{3,}`）和开头的分隔符（`^[\s:：\-]+`），
/// 连续的冒号（`[:：]{2,}`）换成 `:`，连续的空白并成一个空格。
pub fn sanitize_quoted_content(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let is_lead = |c: char| c.is_whitespace() || matches!(c, ':' | '：' | '-');
    let without_ids = remove_wxids(content);
    let collapsed = collapse_colon_runs(without_ids.trim_start_matches(is_lead));
    // `\s` 和 `split_whitespace` 都按 Unicode 的 White_Space
    let words: Vec<&str> = collapsed
        .trim_start_matches(is_lead)
        .split_whitespace()
        .collect();
    words.join(" ")
}

/// 从左到右去掉 `s` 中的 `wxid_[A-Za-z0-9_-]{3,}`。
fn remove_wxids(s: &str) -> std::borrow::Cow<'_, str> {
    let h = s.as_bytes();
    let mut out = String::new();
    let (mut kept, mut from) = (0, 0);
    while let Some(rel) = memchr::memmem::find(&h[from..], b"wxid_") {
        let at = from + rel;
        let tail = h[at + 5..]
            .iter()
            .take_while(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
            .count();
        if tail >= 3 {
            out.push_str(&s[kept..at]);
            kept = at + 5 + tail;
            from = kept;
        } else {
            from = at + 1;
        }
    }
    if kept == 0 {
        return std::borrow::Cow::Borrowed(s);
    }
    out.push_str(&s[kept..]);
    std::borrow::Cow::Owned(out)
}

/// 把 `s` 中的 `[:：]{2,}` 换成 `:`。
fn collapse_colon_runs(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if !matches!(c, ':' | '：') {
            out.push(c);
            continue;
        }
        let mut run = 1;
        while chars.next_if(|n| matches!(n, ':' | '：')).is_some() {
            run += 1;
        }
        out.push(if run >= 2 { ':' } else { c });
    }
    out
}

fn is_hex(c: char) -> bool {
    c.is_ascii_hexdigit()
}

/// 相当于 `([a-f0-9]{min,max})(?![a-f0-9])`：在第一段长度至少为 `min` 的十六进制中，
/// 取末尾最多 `max` 个字符，即带前瞻的正则匹配到的部分。
pub(super) fn last_hex_token(text: &str, min: usize, max: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if is_hex(chars[i]) {
            let start = i;
            while i < chars.len() && is_hex(chars[i]) {
                i += 1;
            }
            let len = i - start;
            if len >= min {
                let take = len.min(max);
                return Some(chars[i - take..i].iter().collect());
            }
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::message::emoji::{extract_emoji_md5, extract_emoji_url, extract_loose_hex_md5};
    use crate::message::media::{
        dat_name_in_packed_info, extract_image_md5, extract_video_md5,
        normalize_image_dat_name_token,
    };
    use crate::message::system::is_readable_system_message;

    #[test]
    fn xml_value_strips_cdata() {
        let xml = "<msg><title><![CDATA[ hello ]]></title></msg>";
        assert_eq!(extract_xml_value(xml, "title"), "hello");
        assert_eq!(
            extract_xml_attribute(
                r#"<location x="1.5" y="2" poiname="P"/>"#,
                "location",
                "poiname"
            ),
            "P"
        );
    }

    /// 给人读的文字只在 CDATA 之外解码一次实体（#1199）。
    #[test]
    fn xml_text_decodes_entities_once_outside_cdata() {
        for (inside, want) in [
            ("A &amp; B", "A & B"),
            ("&amp;amp; &amp;lt;", "&amp; &lt;"),
            ("Caf&#233; &#xE9;&#XE9;", "Café éé"),
            ("&lt;b&gt; &quot;q&quot; &apos;", "<b> \"q\" '"),
            ("&#xZZ; &#; &nbsp; & a;b", "&#xZZ; &#; &nbsp; & a;b"),
            ("&#1114112; &#55296;", "&#1114112; &#55296;"),
            (" <![CDATA[ a &amp; b ]]> ", "a &amp; b"),
            ("x &amp; <![CDATA[&amp;]]> &amp; y", "x & &amp; & y"),
            ("中&amp;文", "中&文"),
        ] {
            let xml = format!("<msg><title>{inside}</title></msg>");
            assert_eq!(xml_text(&xml, "title"), want, "{inside:?}");
        }
        assert_eq!(xml_text("<msg/>", "title"), "");
        assert_eq!(
            xml_attr_text(
                r#"<location label="Caf&#233; &amp; Bar"/>"#,
                "location",
                "label"
            ),
            "Café & Bar"
        );
        // 本身是 XML 的字段的值保留实体，由读它的地方解码
        assert_eq!(
            extract_xml_value(
                "<refermsg><content>&lt;msg&gt;</content></refermsg>",
                "content"
            ),
            "&lt;msg&gt;"
        );
    }

    #[test]
    fn sender_prefix_keeps_urls() {
        assert_eq!(strip_sender_prefix("wxid_abc:hello"), "hello");
        assert_eq!(strip_sender_prefix("http://x.y"), "http://x.y");
        assert_eq!(strip_sender_prefix("  a-b_c:\nx"), "x");
        // 正文中的 "1:3-1:6" 和 "4:1" 不是发送者前缀
        assert_eq!(strip_sender_prefix("1:3-1:6"), "1:3-1:6");
        assert_eq!(strip_sender_prefix("4:1"), "4:1");
        assert_eq!(strip_sender_prefix("10:1 abc"), "10:1 abc");
        // 企业微信成员的 ID
        assert_eq!(strip_sender_prefix("123456@openim:\nhi"), "hi");
        assert_eq!(strip_sender_prefix("a@b.c:hi"), "a@b.c:hi");
    }

    #[test]
    fn only_the_line_naming_the_sender_is_a_sender_line() {
        // 群里别人发的消息（#656）
        assert_eq!(strip_sender_line("wxid_bob:\nhi", Some("wxid_bob")), "hi");
        assert_eq!(strip_sender_line("WXID_bob:\nhi", Some("wxid_bob")), "hi");
        assert_eq!(strip_sender_line("1@openim:\nhi", Some("1@openim")), "hi");
        // 导出没能解析出的发送者记为群本身
        assert_eq!(strip_sender_line("wxid_bob:\nhi", Some("1@chatroom")), "hi");
        // 其他消息的第一行属于正文
        for (text, sender) in [
            ("PS:\nsee you", "wxid_bob"),
            ("TODO:\nbuy milk", "wxid_me"),
            ("wxid_carol:\nhi", "wxid_bob"),
            ("4:1", "wxid_bob"),
        ] {
            assert_eq!(strip_sender_line(text, Some(sender)), text, "{text}");
        }
        // 不知道发送者时只按文字判断
        assert_eq!(strip_sender_line("PS:\nsee you", None), "see you");
        assert_eq!(strip_sender_line("PS:\nsee you", Some(" ")), "see you");
    }

    #[test]
    fn app_type_ignores_refermsg() {
        let xml = "<msg><appmsg><title>t</title><type>57</type><refermsg><type>1</type></refermsg></appmsg></msg>";
        assert_eq!(extract_app_message_type(xml), "57");
    }

    /// app 消息的类型是它自己的 `<type>`：写在被引用的 app 消息的整段 XML 或聊天记录的条目之后时也是，
    /// 写成 CDATA 时也是（#1241）。
    #[test]
    fn the_app_type_is_the_messages_own() {
        let quoted_file =
            "&lt;msg&gt;&lt;appmsg&gt;&lt;title&gt;a.pdf&lt;/title&gt;&lt;type&gt;6&lt;/type&gt;\
            &lt;/appmsg&gt;&lt;/msg&gt;";
        for (xml, ty) in [
            (
                format!(
                    "<msg><appmsg><title>好</title><refermsg><type>49</type><content>{quoted_file}</content>\
                     </refermsg><type>57</type></appmsg></msg>"
                ),
                "57",
            ),
            (
                "<msg><appmsg><title>记录</title><recorditem>&lt;recordinfo&gt;&lt;refermsgitem&gt;\
                 &lt;type&gt;1&lt;/type&gt;&lt;/refermsgitem&gt;&lt;/recordinfo&gt;</recorditem><type>19</type>\
                 </appmsg></msg>"
                    .to_string(),
                "19",
            ),
            (
                "<msg><appmsg><title>红包</title><type><![CDATA[2001]]></type></appmsg></msg>".to_string(),
                "2001",
            ),
        ] {
            assert_eq!(extract_app_message_type(&xml), ty, "{xml}");
            assert_eq!(MsgXml::new(&xml).app_type_twice(), ty, "{xml}");
            assert_eq!(regex_reference::extract_app_message_type(&xml), ty, "{xml}");
        }
    }

    /// XML 扫描函数所替代的正则，留作参考实现，扫描函数的结果必须与它们一致。
    mod regex_reference {
        use super::super::{last_hex_token, normalize_app_message_content, rx};
        use crate::message::emoji::{normalize_md5, percent_decode};

        pub fn attribute(xml: &str, tag: &str, attr: &str) -> String {
            rx(&format!(
                r#"(?i)<{}\s+[^>]*{}\s*=\s*"([^"]*)""#,
                regex::escape(tag),
                regex::escape(attr)
            ))
            .captures(xml)
            .map(|c| c[1].to_string())
            .unwrap_or_default()
        }

        pub fn strip_sender_prefix(content: &str) -> String {
            let re = rx(r"^\s*([A-Za-z0-9_@-]+):(\r?\n|)");
            if let Some(c) = re.captures(content) {
                let whole = c.get_match();
                let looks_like_id = c[1].starts_with("wxid_") || c[1].starts_with("gh_");
                if (!c[2].is_empty() || looks_like_id) && !content[whole.end()..].starts_with("//")
                {
                    return content[whole.end()..].to_string();
                }
            }
            content.to_string()
        }

        pub fn is_readable_system_message(content: &str) -> bool {
            let normalized = normalize_app_message_content(content);
            rx(r"(?i)<sysmsg\b").is_match(&strip_sender_prefix(&normalized))
        }

        pub fn sysmsg_body(content: &str) -> Option<String> {
            rx(r"(?is)<sysmsg\b[^>]*>(.*?)</sysmsg>")
                .captures(content)
                .map(|c| c[1].to_string())
        }

        pub fn loose_hex_md5(content: &str) -> Option<String> {
            rx(r"(?i)(?:emoji|sticker|md5)[^a-fA-F0-9]{0,32}([a-fA-F0-9]{32})")
                .captures(content)
                .or_else(|| rx(r"(?i)([a-fA-F0-9]{32})").captures(content))
                .and_then(|c| normalize_md5(&c[1]))
        }

        pub fn emoji_url(content: &str) -> Option<String> {
            if let Some(c) = rx(r#"(?i)cdnurl\s*=\s*['"]([^'"]+)['"]"#).captures(content) {
                return Some(percent_decode(&c[1].replace("&amp;", "&")));
            }
            rx(r"(?i)cdnurl[^>]*>([^<]+)")
                .captures(content)
                .map(|c| c[1].to_string())
        }

        pub fn emoji_md5(content: &str) -> Option<String> {
            rx(r#"(?i)md5\s*=\s*['"]([a-fA-F0-9]{32})['"]"#)
                .captures(content)
                .or_else(|| rx(r"(?i)md5\s*=\s*([a-fA-F0-9]{32})").captures(content))
                .or_else(|| rx(r"(?i)<md5>([a-fA-F0-9]{32})</md5>").captures(content))
                .and_then(|c| normalize_md5(&c[1]))
        }

        pub fn spaced_md5(content: &str, open: &str) -> Option<String> {
            let attr = format!(r#"(?i){open}[^>]*\smd5\s*=\s*['"]([a-fA-F0-9]+)['"]"#);
            if let Some(c) = rx(&attr).captures(content) {
                return Some(c[1].to_lowercase());
            }
            rx(r"(?i)<md5>([^<]+)</md5>")
                .captures(content)
                .map(|c| c[1].to_lowercase())
        }

        pub fn all(re: &str, s: &str, group: usize) -> Vec<String> {
            rx(re)
                .captures_iter(s)
                .map(|c| c[group].to_string())
                .collect()
        }

        pub fn extract_app_message_type(content: &str) -> String {
            if content.is_empty() {
                return String::new();
            }
            let normalized = normalize_app_message_content(content);
            let mut own = normalized.clone();
            for block in [
                r"(?is)<refermsg.*?</refermsg>",
                r"(?is)<patmsg.*?</patmsg>",
                r"(?is)<recorditem.*?</recorditem>",
            ] {
                own = rx(block).replace_all(&own, "").to_string();
            }
            if let Some(c) = rx(r"(?is)<appmsg.*?>(.*?)</appmsg>").captures(&own) {
                if let Some(t) = rx(r"(?is)<type>(.*?)</type>").captures(&c[1]) {
                    return t[1]
                        .replace("<![CDATA[", "")
                        .replace("]]>", "")
                        .trim()
                        .to_string();
                }
            }
            if !normalized.contains("<appmsg") && !normalized.contains("<msg>") {
                return String::new();
            }
            rx(r"(?i)<type>(\d+)</type>")
                .captures(&normalized)
                .map(|c| c[1].to_string())
                .unwrap_or_default()
        }

        pub fn looks_like_wxid(text: &str) -> bool {
            let t = text.trim().to_lowercase();
            if t.is_empty() {
                return false;
            }
            t.starts_with("wxid_") || rx(r"^wx[a-z0-9_-]{4,}$").is_match(&t)
        }

        pub fn sanitize_quoted_content(content: &str) -> String {
            if content.is_empty() {
                return String::new();
            }
            let mut r = rx(r"wxid_[A-Za-z0-9_-]{3,}")
                .replace_all(content, "")
                .to_string();
            r = rx(r"^[\s:：\-]+").replace(&r, "").to_string();
            r = rx(r"[:：]{2,}").replace_all(&r, ":").to_string();
            r = rx(r"^[\s:：\-]+").replace(&r, "").to_string();
            rx(r"\s+").replace_all(&r, " ").trim().to_string()
        }

        pub fn normalize_image_dat_name_token(value: &str) -> Option<String> {
            let mut text = value.trim().to_string();
            if text.is_empty() {
                return None;
            }
            text = text.replace("&amp;", "&");
            text = percent_decode(&text);
            if let Some(c) = rx(r"(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat").captures(&text) {
                return Some(c[1].to_lowercase());
            }
            let no_query = text.split(['?', '#']).next().unwrap_or("").to_string();
            let base = rx(r"^.*[\\/]").replace(&no_query, "").to_string();
            let base = rx(r"(?i)\.(?:t\.)?dat$")
                .replace(&base, "")
                .trim()
                .to_string();
            if base.is_empty() {
                return None;
            }
            let cdn_token = if base.contains('_') {
                base.split('_').next().unwrap_or("").to_string()
            } else {
                base
            };
            if rx(r"^[a-fA-F0-9]{16,64}$").is_match(&cdn_token) {
                return Some(cdn_token.to_lowercase());
            }
            if let Some(t) = last_hex_token(&cdn_token, 32, 32) {
                return Some(t.to_lowercase());
            }
            last_hex_token(&cdn_token, 16, 64).map(|t| t.to_lowercase())
        }

        pub fn dat_name_in_packed_info(bytes: &[u8]) -> Option<String> {
            let text: String = bytes
                .iter()
                .map(|b| {
                    if (0x20..=0x7e).contains(b) {
                        *b as char
                    } else {
                        ' '
                    }
                })
                .collect();
            if let Some(c) = rx(r"(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat").captures(&text) {
                return Some(c[1].to_lowercase());
            }
            rx(r"([0-9a-fA-F]{16,})")
                .captures(&text)
                .map(|c| c[1].to_lowercase())
        }
    }

    #[test]
    fn the_regex_free_helpers_match_the_regexes_on_random_snippets() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        // `\u{3000}` 和 `\u{85}` 是空白，`：` 是全角冒号，`٣` 是 Unicode 的 `\d`
        const TOKENS: &[&str] = &[
            "wxid_",
            "wxid_ab",
            "WXID_",
            "wx",
            "abc",
            "-",
            "_",
            ":",
            "：",
            " ",
            "\n",
            "\u{3000}",
            "\u{85}",
            "<type>",
            "<TYPE>",
            "</type>",
            "</Type>",
            "57",
            "٣",
            "<msg>",
            "<appmsg>",
            ".dat",
            ".t.dat",
            ".DAT",
            ".T.Dat",
            "0123456789abcdef",
            "ABCDEF01",
            "deadbeef",
            "/",
            "\\",
            "?",
            "#",
            "%2F",
            "&amp;",
            "x",
            "中",
            "\u{1}",
            "\u{7f}",
            "<msg><type>57</type>",
            "<type>5</type>",
            "<TYPE>٣</type>",
            "<msg><appmsg><type>",
        ];
        let mut rng = StdRng::seed_from_u64(547);
        let mut hits = [0usize; 5];
        for _ in 0..40_000 {
            let s: String = (0..rng.random_range(0..12))
                .map(|_| TOKENS[rng.random_range(0..TOKENS.len())])
                .collect();
            let typed = extract_app_message_type(&s);
            hits[0] += usize::from(!typed.is_empty());
            assert_eq!(
                typed,
                regex_reference::extract_app_message_type(&s),
                "{s:?}"
            );
            let wx = looks_like_wxid(&s);
            hits[1] += usize::from(wx && !s.trim().starts_with("wxid_"));
            assert_eq!(wx, regex_reference::looks_like_wxid(&s), "{s:?}");
            let sanitized = sanitize_quoted_content(&s);
            hits[2] += usize::from(sanitized.len() != s.len());
            assert_eq!(
                sanitized,
                regex_reference::sanitize_quoted_content(&s),
                "{s:?}"
            );
            let token = normalize_image_dat_name_token(&s);
            hits[3] += usize::from(token.is_some());
            assert_eq!(
                token,
                regex_reference::normalize_image_dat_name_token(&s),
                "{s:?}"
            );
            let packed = dat_name_in_packed_info(s.as_bytes());
            hits[4] += usize::from(packed.is_some());
            assert_eq!(
                packed,
                regex_reference::dat_name_in_packed_info(s.as_bytes()),
                "{s:?}"
            );
        }
        assert!(hits.iter().all(|&n| n >= 100), "{hits:?}");
    }

    #[test]
    fn xml_scanners_match_the_regexes_on_random_snippets() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        // `\u{3000}` 是空白，`é` 和组合字符 `\u{301}` 是单词字符（`\b`），`²` 不是
        const TOKENS: &[&str] = &[
            "<img",
            "<IMG ",
            "<videomsg",
            "<location",
            "<appattach",
            " md5",
            "MD5",
            "md5",
            "cdnthumbmd5",
            "=",
            " = ",
            "\"",
            "'",
            ">",
            "<",
            "/",
            " ",
            "\n",
            "\r\n",
            "\u{3000}",
            "<md5>",
            "</md5>",
            "</MD5>",
            "0123456789abcdef0123456789ABCDEF",
            "deadbeef",
            "x",
            "X=",
            "cdnurl",
            "CdnUrl",
            "&amp;",
            "&lt;",
            "&gt;",
            "%41",
            "emoji",
            "sticker",
            "中",
            "é",
            "\u{301}",
            "²",
            "<sysmsg",
            "<SysMsg ",
            "</sysmsg>",
            "wxid_ab",
            "gh_",
            "a-b",
            ":",
            "//",
            "<dataitem",
            "</dataitem>",
            "<recorditem>",
            "</RecordItem>",
            "<![CDATA[",
            "]]>",
            "_",
            " md5=\"",
            " x=\"",
            "cdnurl=\"",
            "=\"1.5\"",
            "\"0123456789abcdef0123456789abcdef\"",
            "wxid_ab:",
            "gh_x:\n",
            "a-b:\r\n",
            "1@openim:\n",
            "@",
            "<DataItem datatype=\"1\">",
            "<msgsource>",
            "</MsgSource>",
        ];
        let mut rng = StdRng::seed_from_u64(521);
        // 统计每个扫描函数找到内容的次数，免得比较的全是没找到的情况
        let mut hits = std::collections::BTreeMap::<&str, usize>::new();
        let mut hit = |name, found: bool| *hits.entry(name).or_default() += usize::from(found);
        for _ in 0..40_000 {
            let s: String = (0..rng.random_range(0..16))
                .map(|_| TOKENS[rng.random_range(0..TOKENS.len())])
                .collect();
            for (tag, attr) in [
                ("img", "md5"),
                ("location", "x"),
                ("appattach", "md5"),
                ("IMG", "cdnurl"),
            ] {
                let found = extract_xml_attribute(&s, tag, attr);
                assert_eq!(
                    found,
                    regex_reference::attribute(&s, tag, attr),
                    "{tag} {attr} in {s:?}"
                );
                hit("attribute", !found.is_empty());
            }
            hit("sender prefix", sender_prefix_len(&s) > 0);
            hit("sysmsg", crate::xml::has_open(&s, "sysmsg"));
            hit("loose md5", extract_loose_hex_md5(&s).is_some());
            hit("emoji url", extract_emoji_url(&s).is_some());
            hit("image md5", extract_image_md5(&s).is_some());
            hit(
                "dataitem",
                crate::xml::elements(&s, "dataitem").next().is_some(),
            );
            hit(
                "recorditem",
                crate::xml::tag_inners(&s, "recorditem").next().is_some(),
            );
            hit(
                "cdata",
                crate::xml::between(&s, "<![CDATA[", "]]>").next().is_some(),
            );
            hit(
                "msgsource",
                crate::xml::tag_element(&s, "msgsource").is_some(),
            );
            assert_eq!(
                strip_sender_prefix(&s),
                regex_reference::strip_sender_prefix(&s),
                "{s:?}"
            );
            assert_eq!(
                is_readable_system_message(1, &s),
                regex_reference::is_readable_system_message(&s),
                "{s:?}"
            );
            assert_eq!(
                crate::xml::elements(&s, "sysmsg")
                    .next()
                    .map(|(_, b)| b.to_string()),
                regex_reference::sysmsg_body(&s),
                "{s:?}"
            );
            assert_eq!(
                extract_loose_hex_md5(&s),
                regex_reference::loose_hex_md5(&s),
                "{s:?}"
            );
            assert_eq!(
                extract_emoji_url(&s),
                regex_reference::emoji_url(&s),
                "{s:?}"
            );
            assert_eq!(
                extract_emoji_md5(&s),
                regex_reference::emoji_md5(&s).or_else(|| regex_reference::loose_hex_md5(&s)),
                "{s:?}"
            );
            assert_eq!(
                extract_image_md5(&s),
                regex_reference::spaced_md5(&s, "<img"),
                "{s:?}"
            );
            if !s.is_empty() {
                assert_eq!(
                    extract_video_md5(&s),
                    regex_reference::spaced_md5(&s, "<videomsg"),
                    "{s:?}"
                );
            }
            let pairs = |it: &mut dyn Iterator<Item = (&str, &str)>| -> Vec<String> {
                it.map(|(a, b)| format!("{a}|{b}")).collect()
            };
            assert_eq!(
                pairs(&mut crate::xml::elements(&s, "dataitem")),
                rx(r"(?is)<dataitem\b([^>]*)>(.*?)</dataitem>")
                    .captures_iter(&s)
                    .map(|c| format!("{}|{}", &c[1], &c[2]))
                    .collect::<Vec<_>>(),
                "{s:?}"
            );
            assert_eq!(
                crate::xml::tag_inners(&s, "recorditem").collect::<Vec<_>>(),
                regex_reference::all(r"(?is)<recorditem>(.*?)</recorditem>", &s, 1),
                "{s:?}"
            );
            assert_eq!(
                crate::xml::between(&s, "<![CDATA[", "]]>").collect::<Vec<_>>(),
                regex_reference::all(r"(?s)<!\[CDATA\[(.*?)\]\]>", &s, 1),
                "{s:?}"
            );
            assert_eq!(
                crate::xml::tag_element(&s, "msgsource"),
                rx(r"(?i)<msgsource>[\s\S]*?</msgsource>")
                    .find(&s)
                    .map(|m| m.as_str()),
                "{s:?}"
            );
            assert_eq!(
                crate::xml::text_inner(&s, "md5"),
                rx(r"(?i)<md5>([^<]+)</md5>")
                    .captures(&s)
                    .map(|c| c.get(1).unwrap().as_str()),
                "{s:?}"
            );
        }
        assert!(
            hits.values().all(|&n| n >= 100),
            "too few matches: {hits:?}"
        );
    }

    #[test]
    fn entities_are_decoded_once() {
        for (input, want) in [
            ("&amp;lt;b&amp;gt;", "&lt;b&gt;"),
            ("&lt;b&gt;", "<b>"),
            ("&amp;quot;&amp;#39;&amp;apos;", "&quot;&#39;&apos;"),
            ("a &amp; b", "a & b"),
        ] {
            assert_eq!(decode_html_entities(input), want, "{input:?}");
        }
    }

    #[test]
    fn normalizing_unescapes_one_level() {
        assert_eq!(
            normalize_app_message_content(
                "&lt;title t=&quot;a &amp;quot; b&quot;&gt;x &amp;lt;y&amp;gt;&lt;/title&gt;"
            ),
            "<title t=\"a &quot; b\">x &lt;y&gt;</title>"
        );
        // 没有转义的 XML 时内容保持原样
        assert_eq!(normalize_app_message_content("a &amp; b"), "a &amp; b");
        let x = MsgXml::new("&lt;a&gt;&amp;lt;b&amp;gt;&lt;/a&gt;");
        assert_eq!(x.once(), "<a>&lt;b&gt;</a>");
        assert_eq!(x.twice(), "<a><b></a>");
    }

    #[test]
    fn a_word_missing_from_the_content_is_missing_once_normalized() {
        use rand::{rngs::StdRng, RngExt, SeedableRng};
        const TOKENS: &[&str] = &[
            "refer", "msg", "refermsg", "emoticon", "EMOTICON", "md5", "MD5", "&lt;", "&gt;",
            "&amp;", "&quot;", "&#39;", "&", "lt;", "gt;", "amp;", "<", ">", ";", "#39", "x",
        ];
        let mut rng = StdRng::seed_from_u64(546);
        let mut found = [0usize; 2];
        for _ in 0..40_000 {
            let s: String = (0..rng.random_range(0..10))
                .map(|_| TOKENS[rng.random_range(0..TOKENS.len())])
                .collect();
            let normalized = normalize_app_message_content(&s);
            if normalized.contains("refermsg") {
                found[0] += 1;
                assert!(normalized_may_contain(&s, "refermsg"), "{s:?}");
            }
            if crate::xml::find_ci(&normalized, 0, "emoticonmd5").is_some() {
                found[1] += 1;
                assert!(crate::xml::find_ci(&s, 0, "emoticonmd5").is_some(), "{s:?}");
            }
        }
        assert!(found.iter().all(|&n| n >= 100), "{found:?}");
    }

    #[test]
    fn xml_scanners_keep_the_regex_quirks() {
        // 以 `md5` 结尾的最后一个属性胜出，因为 `[^>]*` 是贪婪的
        assert_eq!(
            extract_xml_attribute(r#"<img md5="a" cdnthumbmd5="b">"#, "img", "md5"),
            "b"
        );
        // 前面的值里的 `>` 会结束搜索
        assert_eq!(
            extract_xml_attribute(r#"<img a="x>" md5="m">"#, "img", "md5"),
            ""
        );
        // 但要取的值本身可以越过第一个 `>`
        assert_eq!(
            extract_xml_attribute(r#"<img md5="m>n">"#, "img", "md5"),
            "m>n"
        );
        assert_eq!(
            extract_image_md5(r#"<img aesmd5="ab" md5='CD"><md5>EF</md5>"#).as_deref(),
            Some("cd")
        );
        assert_eq!(
            extract_image_md5("<msg><md5> Ab </md5></msg>").as_deref(),
            Some(" ab ")
        );
        assert!(is_readable_system_message(
            1,
            "wxid_a:\n<sysmsg type=\"x\">"
        ));
        assert!(!is_readable_system_message(1, "<sysmsgé>"));
        assert!(is_readable_system_message(1, "<SYSMSG²>"));
    }
}
