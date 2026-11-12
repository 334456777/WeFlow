//! Reading WeChat's XML: the regex cache, tag and attribute values, the parsed `<msg>`, sender prefixes and the
//! wxids quoted content may carry.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use regex::Regex;

use super::row::unsigned_token;
use super::QuoteInfo;
use super::QuotedDisplay;
use super::{format_quoted_reference_preview, has_sysmsg, quote_info_of, refermsg_xml};

thread_local! {
    static RX_CACHE: RefCell<HashMap<String, Rc<Regex>>> = RefCell::new(HashMap::new());
}

/// Compile (and cache) a regex. Patterns here are static or escaped, so failure is a bug.
///
/// The cache hands out a shared `Rc<Regex>` on purpose: `Regex::clone` gives the clone a brand-new
/// (cold) search cache, which made every call re-run the lazy DFA construction and dominated exports.
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

pub fn extract_xml_value(xml: &str, tag: &str) -> String {
    let inner = if tag.is_ascii() {
        crate::xml::tag_inner(xml, tag)
    } else {
        let re = rx(&format!(r"(?is)<{0}>(.*?)</{0}>", regex::escape(tag)));
        re.captures(xml)
            .map(|c| c.get(1).map_or("", |m| m.as_str()))
    };
    match inner {
        Some(c) => strip_cdata(c),
        None => String::new(),
    }
}

/// `c.replace("<![CDATA[", "").replace("]]>", "").trim()`, without copying when there is nothing to remove.
fn strip_cdata(c: &str) -> String {
    use crate::xml::replace_cow;
    use std::borrow::Cow;
    replace_cow(replace_cow(Cow::Borrowed(c), "<![CDATA[", ""), "]]>", "")
        .trim()
        .to_string()
}

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

pub(crate) fn decode_html_entities(text: &str) -> String {
    decode_html_entities_cow(text).into_owned()
}

/// [`decode_html_entities`] without a copy when there is no `&`.
pub(super) fn decode_html_entities_cow(text: &str) -> std::borrow::Cow<'_, str> {
    if memchr::memchr(b'&', text.as_bytes()).is_none() {
        return std::borrow::Cow::Borrowed(text);
    }
    let decoded = text
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'");
    std::borrow::Cow::Owned(decoded)
}

pub(super) fn normalize_app_message_content(content: &str) -> String {
    normalize_app_message_cow(content).into_owned()
}

/// [`normalize_app_message_content`] without a copy when the content has no escaped markup.
pub(super) fn normalize_app_message_cow(content: &str) -> std::borrow::Cow<'_, str> {
    use crate::xml::replace_cow;
    use std::borrow::Cow;
    let has = |entity: &str| memchr::memmem::find(content.as_bytes(), entity.as_bytes()).is_some();
    if has("&lt;") && has("&gt;") {
        let s = replace_cow(Cow::Borrowed(content), "&lt;", "<");
        let s = replace_cow(s, "&gt;", ">");
        let s = replace_cow(s, "&amp;", "&");
        let s = replace_cow(s, "&quot;", "\"");
        return replace_cow(s, "&#39;", "'");
    }
    Cow::Borrowed(content)
}

/// Whether `content` can contain `word` once normalized ([`normalize_app_message_cow`]). Normalizing replaces each
/// entity with one of `<>&"'`, so a stretch of the result without those characters is in `content` as it is: when
/// `word` has none of them, one search of `content` can rule it out before anything is normalized.
fn normalized_may_contain(content: &str, word: &str) -> bool {
    debug_assert!(!word.contains(['<', '>', '&', '"', '\'']));
    content.contains(word)
}

/// The XML of one message as the export reads it: what its functions each worked out from the content again (the
/// normalized text, the `<appmsg>` type, the system-message check and the quote), worked out the first time one of
/// them asks, for all of them.
///
/// Some of those functions read the content as it is stored and some read it normalized once more (a quote's text,
/// for one, is read from the normalized content normalized again), which differs only for markup escaped twice; so
/// both are kept: `once` is the content normalized, `twice` that normalized again.
pub struct MsgXml<'a> {
    content: &'a str,
    once: std::cell::OnceCell<std::borrow::Cow<'a, str>>,
    /// `once` normalized again, when that changes it.
    twice: std::cell::OnceCell<Option<String>>,
    app_type_twice: std::cell::OnceCell<String>,
    /// The `<refermsg>` blocks of `once` and of `twice`.
    refer_once: std::cell::OnceCell<Option<String>>,
    refer_twice: std::cell::OnceCell<Option<String>>,
    /// The quote in `once`, which is read from the `<refermsg>` of `twice`.
    quote_twice: std::cell::OnceCell<QuoteInfo>,
    display: std::cell::OnceCell<Option<QuotedDisplay>>,
}

impl<'a> MsgXml<'a> {
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

    pub fn content(&self) -> &'a str {
        self.content
    }

    /// [`normalize_app_message_content`] of the content.
    pub fn once(&self) -> &str {
        self.once
            .get_or_init(|| normalize_app_message_cow(self.content))
    }

    /// [`normalize_app_message_content`] of [`once`](Self::once).
    pub fn twice(&self) -> &str {
        let twice = self
            .twice
            .get_or_init(|| match normalize_app_message_cow(self.once()) {
                std::borrow::Cow::Borrowed(_) => None,
                std::borrow::Cow::Owned(s) => Some(s),
            });
        twice.as_deref().unwrap_or_else(|| self.once())
    }

    /// [`extract_app_message_type`] of [`once`](Self::once).
    pub(crate) fn app_type_twice(&self) -> &str {
        self.app_type_twice
            .get_or_init(|| app_type_in(self.once(), self.twice()))
    }

    /// [`is_readable_system_message`](super::is_readable_system_message) of the content.
    pub fn readable_system_once(&self, local_type: i64) -> bool {
        local_type == 10000 || has_sysmsg(self.once())
    }

    /// [`is_readable_system_message`](super::is_readable_system_message) of [`once`](Self::once).
    pub(crate) fn readable_system_twice(&self, local_type: i64) -> bool {
        local_type == 10000 || has_sysmsg(self.twice())
    }

    /// Whether the content can hold a quote at all (see [`normalized_may_contain`]).
    fn may_quote(&self) -> bool {
        normalized_may_contain(self.content, "refermsg")
    }

    /// The `<refermsg>` block of [`once`](Self::once).
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

    /// The `<refermsg>` block of [`twice`](Self::twice).
    fn refer_twice(&self) -> Option<&str> {
        // normalizing once more changed nothing: the same block
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

    /// The quote in [`once`](Self::once): who is quoted and what they said.
    pub(super) fn quote_twice(&self) -> &QuoteInfo {
        self.quote_twice.get_or_init(|| {
            self.refer_twice()
                .map_or_else(QuoteInfo::empty, quote_info_of)
        })
    }

    /// [`extract_quoted_reply_display`](super::extract_quoted_reply_display) of the content.
    pub fn quoted_display(&self) -> Option<&QuotedDisplay> {
        self.display
            .get_or_init(|| {
                let refer = self.refer_once()?;
                let info = self.quote_twice();
                let reply_text = strip_sender_prefix(&extract_xml_value(self.once(), "title"));
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

    /// The server id (`<svrid>`) of the message the content replies to.
    pub fn reply_to_message_id(&self) -> Option<String> {
        unsigned_token(&extract_xml_value(self.refer_once()?, "svrid"))
    }

    /// [`quoted_sender_username`](super::quoted_sender_username) of the content.
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

/// `content` without the sender a group message starts with (`wxid_xxx:` and the line break after it); `http://`
/// and text such as `1:3-1:6` stay as they are. The one rule for the exports and the chat model.
pub fn strip_sender_prefix(content: &str) -> String {
    content[sender_prefix_len(content)..].to_string()
}

/// `content`, the text of a message `sender` sent, without its sender line (#656). Only a message somebody else
/// sent to a group starts with one, naming them; the first line of any other text, such as `PS:`, stays. The line
/// goes when it names `sender`, or when the sender is the group itself (the exports' sender of a row whose sender
/// was not resolved). With no sender known (none or empty) it is [`strip_sender_prefix`].
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

/// Length of the sender prefix [`strip_sender_prefix`] removes: `^\s*([A-Za-z0-9_@-]+):(\r?\n|)`, or 0.
pub(super) fn sender_prefix_len(content: &str) -> usize {
    sender_prefix(content).map_or(0, |(_, end)| end)
}

/// The name of the sender prefix [`strip_sender_prefix`] removes, and where the text after it starts.
fn sender_prefix(content: &str) -> Option<(&str, usize)> {
    // 数据库里的群消息是 "wxid:\n正文"（企业微信成员是 "…@openim:\n正文"）；同一行的 "1:3-1:6" 这类是正文本身，
    // 不能当作前缀去掉。
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

pub(super) fn extract_app_message_type(content: &str) -> String {
    app_type_in(content, &normalize_app_message_cow(content))
}

/// [`extract_app_message_type`] of `content`, given `content` normalized.
fn app_type_in(content: &str, normalized: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    // `(?is)<appmsg.*?>(.*?)</appmsg>`, minus `<refermsg…</refermsg>` and `<patMsg…</patMsg>` blocks, then `<type>`
    if let Some(c) = crate::xml::element_inner(normalized, "<appmsg", "</appmsg>") {
        let inner = crate::xml::remove_blocks(c, "<refermsg", "</refermsg>");
        let inner = crate::xml::remove_blocks(&inner, "<patmsg", "</patmsg>");
        if let Some(t) = crate::xml::tag_inner(&inner, "type") {
            return t.trim().to_string();
        }
    }
    if !normalized.contains("<appmsg") && !normalized.contains("<msg>") {
        return String::new();
    }
    // the regex needs a `<type>`, and no character but these letters' ASCII cases folds to them: images, voice and
    // stickers have none
    if crate::xml::find_ci(normalized, 0, "<type>").is_none() {
        return String::new();
    }
    rx(r"(?i)<type>(\d+)</type>")
        .captures(normalized)
        .map(|c| c[1].to_string())
        .unwrap_or_default()
}

pub(crate) fn looks_like_wxid(text: &str) -> bool {
    let t = text.trim().to_lowercase();
    if t.is_empty() {
        return false;
    }
    // `^wx[a-z0-9_-]{4,}$`
    let wx_id = t.len() >= 6
        && t.starts_with("wx")
        && t.bytes()
            .skip(2)
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'_' | b'-'));
    t.starts_with("wxid_") || wx_id
}

/// The text of a quote without wxids (`wxid_[A-Za-z0-9_-]{3,}`), the separators in front of it
/// (`^[\s:：\-]+`), runs of colons (`[:：]{2,}` → `:`), and with white space collapsed to single spaces.
pub fn sanitize_quoted_content(content: &str) -> String {
    if content.is_empty() {
        return String::new();
    }
    let is_lead = |c: char| c.is_whitespace() || matches!(c, ':' | '：' | '-');
    let without_ids = remove_wxids(content);
    let collapsed = collapse_colon_runs(without_ids.trim_start_matches(is_lead));
    // `\s` and `split_whitespace` are both Unicode White_Space
    let words: Vec<&str> = collapsed
        .trim_start_matches(is_lead)
        .split_whitespace()
        .collect();
    words.join(" ")
}

/// `wxid_[A-Za-z0-9_-]{3,}` removed from `s`, leftmost first.
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

/// `[:：]{2,}` replaced with `:` in `s`.
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

/// `([a-f0-9]{min,max})(?![a-f0-9])` for the first hex run that is at least `min` long:
/// returns the trailing `max`-capped slice of that run (what the lookahead regex matches).
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
    use crate::message::{
        dat_name_in_packed_info, extract_emoji_md5, extract_emoji_url, extract_image_md5,
        extract_loose_hex_md5, extract_video_md5, is_readable_system_message,
        normalize_image_dat_name_token,
    };

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

    #[test]
    fn sender_prefix_keeps_urls() {
        assert_eq!(strip_sender_prefix("wxid_abc:hello"), "hello");
        assert_eq!(strip_sender_prefix("http://x.y"), "http://x.y");
        assert_eq!(strip_sender_prefix("  a-b_c:\nx"), "x");
        // 正文里的 "1:3-1:6"、"4:1" 不是发送者前缀
        assert_eq!(strip_sender_prefix("1:3-1:6"), "1:3-1:6");
        assert_eq!(strip_sender_prefix("4:1"), "4:1");
        assert_eq!(strip_sender_prefix("10:1 abc"), "10:1 abc");
        // an enterprise WeChat member's id
        assert_eq!(strip_sender_prefix("123456@openim:\nhi"), "hi");
        assert_eq!(strip_sender_prefix("a@b.c:hi"), "a@b.c:hi");
    }

    #[test]
    fn only_the_line_naming_the_sender_is_a_sender_line() {
        // a group message somebody else sent (#656)
        assert_eq!(strip_sender_line("wxid_bob:\nhi", Some("wxid_bob")), "hi");
        assert_eq!(strip_sender_line("WXID_bob:\nhi", Some("wxid_bob")), "hi");
        assert_eq!(strip_sender_line("1@openim:\nhi", Some("1@openim")), "hi");
        // a sender the exports could not resolve is the group itself
        assert_eq!(strip_sender_line("wxid_bob:\nhi", Some("1@chatroom")), "hi");
        // any other message's first line is its own
        for (text, sender) in [
            ("PS:\nsee you", "wxid_bob"),
            ("TODO:\nbuy milk", "wxid_me"),
            ("wxid_carol:\nhi", "wxid_bob"),
            ("4:1", "wxid_bob"),
        ] {
            assert_eq!(strip_sender_line(text, Some(sender)), text, "{text}");
        }
        // with no sender known, the text alone decides
        assert_eq!(strip_sender_line("PS:\nsee you", None), "see you");
        assert_eq!(strip_sender_line("PS:\nsee you", Some(" ")), "see you");
    }

    #[test]
    fn app_type_ignores_refermsg() {
        let xml = "<msg><appmsg><title>t</title><type>57</type><refermsg><type>1</type></refermsg></appmsg></msg>";
        assert_eq!(extract_app_message_type(xml), "57");
    }

    /// The regexes the XML scanners replace, kept as the reference they must agree with.
    mod regex_reference {
        use super::super::{last_hex_token, normalize_app_message_content, rx};
        use crate::message::{normalize_md5, percent_decode};

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
            if let Some(c) = crate::xml::element_inner(&normalized, "<appmsg", "</appmsg>") {
                let inner = crate::xml::remove_blocks(c, "<refermsg", "</refermsg>");
                let inner = crate::xml::remove_blocks(&inner, "<patmsg", "</patmsg>");
                if let Some(t) = crate::xml::tag_inner(&inner, "type") {
                    return t.trim().to_string();
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
        // `\u{3000}` and `\u{85}` are white space, `：` is the full-width colon, `٣` is a Unicode `\d`
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
        // `\u{3000}` is white space, `é` and the combining `\u{301}` are word characters (`\b`), `²` is not
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
        // how often each scanner finds something, so that the comparison is not only of misses
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
            assert_eq!(
                decode_html_entities(&s),
                s.replace("&lt;", "<")
                    .replace("&gt;", ">")
                    .replace("&amp;", "&")
                    .replace("&quot;", "\"")
                    .replace("&#39;", "'")
                    .replace("&apos;", "'"),
                "{s:?}"
            );
        }
        assert!(
            hits.values().all(|&n| n >= 100),
            "too few matches: {hits:?}"
        );
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
        // the last attribute that ends in `md5` wins, as `[^>]*` is greedy
        assert_eq!(
            extract_xml_attribute(r#"<img md5="a" cdnthumbmd5="b">"#, "img", "md5"),
            "b"
        );
        // a `>` inside an earlier value ends the search
        assert_eq!(
            extract_xml_attribute(r#"<img a="x>" md5="m">"#, "img", "md5"),
            ""
        );
        // but a value may run past the first `>`
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
