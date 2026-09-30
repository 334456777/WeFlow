//! The chat service's `Message` model (`electron/services/chatService.ts`):
//! raw WCDB rows → typed messages with parsed display text, media tokens, links, quotes.
//! The exporter has its own (slightly different) model in `message.rs`; this one backs the
//! HTTP API, the chat commands and the analytics code.

use serde_json::{json, Map, Value};

use crate::message::{
    decode_message_content, extract_image_dat_name_from_row, extract_xml_value, get_timestamp_seconds, normalize_unsigned_token, parse_voip_message, row_field,
    row_int, rx,
};
use crate::services::clean_account_dir_name;

// ───────────────────────── text helpers ─────────────────────────

/// `decodeHtmlEntities` of the chat service (amp first).
pub fn decode_html(content: &str) -> String {
    content.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'")
}

pub fn clean_utf16(input: &str) -> String {
    if input.is_empty() {
        return String::new();
    }
    input.chars().filter(|c| !matches!(*c as u32, 0x00..=0x08 | 0x0B | 0x0C | 0x0E..=0x1F | 0x7F..=0x9F)).collect()
}

/// `stripSenderPrefix`: `wxid_x:` (not `http://`) plus the newline / `<br>` that follows.
pub fn strip_sender_prefix(content: &str) -> String {
    let Some(caps) = rx(r"^\s*([A-Za-z0-9_@-]+):").captures(content) else { return content.to_string() };
    let end = caps.get(0).unwrap().end();
    let rest = &content[end..];
    if rest.starts_with("//") {
        return content.to_string();
    }
    let consumed = rx(r"(?i)^\s*(?:\r?\n|<br\s*/?>)\s*").find(rest).or_else(|| rx(r"^\s*").find(rest)).map(|m| m.end()).unwrap_or(0);
    rest[consumed..].to_string()
}

pub fn extract_sender_from_content(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let normalized = clean_utf16(&decode_html(content));
    let caps = rx(r"(?i)^\s*([A-Za-z0-9_@-]{4,}):\s*(?:\r?\n|<br\s*/?>)").captures(&normalized)?;
    Some(caps[1].trim().to_string()).filter(|s| !s.is_empty())
}

/// chatService flavour of `extractXmlAttribute`: single or double quotes, whitespace before the name.
pub fn xml_attr(xml: &str, tag: &str, attr: &str) -> String {
    let re = rx(&format!(r#"(?i)<{}[^>]*\s{}\s*=\s*['"]([^'"]*)['"]"#, regex::escape(tag), regex::escape(attr)));
    re.captures(xml).map(|c| c[1].to_string()).unwrap_or_default()
}

fn first_non_empty(values: &[String]) -> String {
    values.iter().find(|v| !v.is_empty()).cloned().unwrap_or_default()
}

fn opt(s: String) -> Option<String> {
    (!s.is_empty()).then_some(s)
}

pub fn message_type_label(local_type: i64) -> &'static str {
    match local_type {
        1 => "[文本]",
        3 => "[图片]",
        34 => "[语音]",
        42 => "[名片]",
        43 => "[视频]",
        47 => "[动画表情]",
        48 => "[位置]",
        49 => "[链接]",
        50 => "[通话]",
        10000 => "[系统消息]",
        244813135921 => "[引用消息]",
        266287972401 => "拍一拍",
        81604378673 => "[聊天记录]",
        154618822705 => "[小程序]",
        8594229559345 => "[红包]",
        8589934592049 => "[转账]",
        34359738417 | 103079215153 | 25769803825 => "[文件]",
        _ => "[消息]",
    }
}

fn looks_like_wxid(value: &str) -> bool {
    let t = value.trim().to_lowercase();
    !t.is_empty() && (t.starts_with("wxid_") || rx(r"^wx[a-z0-9_-]{4,}$").is_match(&t))
}

fn sanitize_quoted_content(content: &str) -> String {
    crate::message::sanitize_quoted_content(content)
}

pub fn clean_system_message(content: &str) -> String {
    if content.is_empty() {
        return "[系统消息]".into();
    }
    let normalized = clean_utf16(&decode_html(content));
    let readable = extract_readable_system_message_text(&normalized);
    if !readable.is_empty() {
        return readable;
    }
    let mut cleaned = rx(r"(?i)<\?xml[^?]*\?>").replace_all(&normalized, "").to_string();
    cleaned = rx(r"<[^>]+>").replace_all(&cleaned, "").to_string();
    cleaned = rx(r"\d+\s*$").replace(&cleaned, "").to_string();
    cleaned = strip_sender_prefix(&cleaned);
    let cleaned = rx(r"\s+").replace_all(&cleaned, " ").trim().to_string();
    if cleaned.is_empty() {
        "[系统消息]".into()
    } else {
        cleaned
    }
}

fn extract_readable_system_message_text(content: &str) -> String {
    let source = rx(r"(?is)<sysmsg\b[^>]*>(.*?)</sysmsg>").captures(content).map(|c| c[1].to_string()).unwrap_or_else(|| content.to_string());
    let mut text = extract_xml_value(&source, "plain");
    if text.is_empty() {
        text = extract_xml_value(&source, "text");
    }
    rx(r"\s+").replace_all(&strip_sender_prefix(&text), " ").trim().to_string()
}

pub fn clean_pat_message(content: &str) -> String {
    if content.is_empty() {
        return "拍一拍".into();
    }
    if let Some(c) = rx(r"(?is)<title>(.*?)</title>").captures(content) {
        let title = c[1].replace("<![CDATA[", "").replace("]]>", "").trim().to_string();
        if !title.is_empty() {
            return title;
        }
    }
    if let Some(c) = rx(r"^(.+?拍了拍.+?)(?:[\r\n]|$|ງ|wxid_)").captures(content) {
        return c[1].trim().to_string();
    }
    let mut cleaned = rx(r"wxid_[a-zA-Z0-9_-]+").replace_all(content, "").to_string();
    cleaned = rx(r"[ງ໐໓ຖiht]+").replace_all(&cleaned, " ").to_string();
    cleaned = rx(r"\d{6,}").replace_all(&cleaned, "").to_string();
    cleaned = rx(r"\s+").replace_all(&cleaned, " ").trim().to_string();
    cleaned = clean_utf16(&cleaned);
    if cleaned.chars().count() > 1 && !cleaned.contains("xml") {
        return cleaned;
    }
    "拍一拍".into()
}

// ───────────────────────── type 49 ─────────────────────────

fn app_xml_type(content: &str) -> String {
    let mut xml_type = String::new();
    if let Some(c) = rx(r"(?is)<appmsg.*?>(.*?)</appmsg>").captures(content) {
        let inner = rx(r"(?is)<refermsg.*?</refermsg>").replace_all(&c[1], "").to_string();
        let inner = rx(r"(?is)<patMsg.*?</patMsg>").replace_all(&inner, "").to_string();
        if let Some(t) = rx(r"(?is)<type>(.*?)</type>").captures(&inner) {
            xml_type = t[1].trim().to_string();
        }
    }
    if xml_type.is_empty() {
        xml_type = extract_xml_value(content, "type");
    }
    xml_type
}

fn location_label_of(content: &str) -> String {
    first_non_empty(&[xml_attr(content, "location", "label"), xml_attr(content, "location", "poiname"), extract_xml_value(content, "label"), extract_xml_value(content, "poiname")])
}

/// `parseType49`: the display string for appmsg content.
pub fn parse_type49(content: &str) -> String {
    let title = extract_xml_value(content, "title");
    let ty = app_xml_type(content);
    let normalized = content.to_lowercase();
    let location_label = location_label_of(content);
    let is_finder = ty == "51" || normalized.contains("<finder") || normalized.contains("finderusername") || normalized.contains("finderobjectid");
    let is_red_packet = ty == "2001" || normalized.contains("hongbao");
    let is_music = ty == "3" || normalized.contains("<musicurl>") || normalized.contains("<playurl>") || normalized.contains("<dataurl>");

    if ty == "87" {
        let t = extract_xml_value(content, "textannouncement");
        return if t.is_empty() { "[群公告]".into() } else { format!("[群公告] {t}") };
    }
    if is_finder {
        return if title.is_empty() { "[视频号]".into() } else { format!("[视频号] {title}") };
    }
    if is_red_packet {
        return if title.is_empty() { "[红包]".into() } else { format!("[红包] {title}") };
    }
    if !location_label.is_empty() {
        return format!("[位置] {location_label}");
    }
    if is_music {
        return if title.is_empty() { "[音乐]".into() } else { format!("[音乐] {title}") };
    }
    if !title.is_empty() {
        return match ty.as_str() {
            "5" | "49" => format!("[链接] {title}"),
            "6" => format!("[文件] {title}"),
            "19" => format!("[聊天记录] {title}"),
            "33" | "36" => format!("[小程序] {title}"),
            "57" => title,
            "53" => {
                let first = title.split('\n').map(|l| l.trim()).find(|l| !l.is_empty()).unwrap_or(&title).to_string();
                format!("[接龙] {first}")
            }
            "2000" => format!("[转账] {title}"),
            "2001" => format!("[红包] {title}"),
            _ => title,
        };
    }
    match ty.as_str() {
        "6" => "[文件]",
        "19" => "[聊天记录]",
        "33" | "36" => "[小程序]",
        "2000" => "[转账]",
        "2001" => "[红包]",
        "3" => "[音乐]",
        "5" | "49" => "[链接]",
        "87" => "[群公告]",
        "53" => "[接龙]",
        _ => "[消息]",
    }
    .into()
}

/// `parseMessageContent`: the display text of any message.
pub fn parse_message_content(content: &str, local_type: i64) -> String {
    if content.is_empty() {
        return message_type_label(local_type).into();
    }
    let content = clean_utf16(&decode_html(content));
    let xml_type = extract_xml_value(&content, "type");
    let looks_app = content.contains("<appmsg") || content.contains("&lt;appmsg");
    match local_type {
        1 => strip_sender_prefix(&content),
        3 => "[图片]".into(),
        34 => "[语音消息]".into(),
        42 => "[名片]".into(),
        43 => "[视频]".into(),
        47 => "[动画表情]".into(),
        48 => {
            let label = first_non_empty(&[xml_attr(&content, "location", "label"), xml_attr(&content, "location", "poiname"), extract_xml_value(&content, "label"), extract_xml_value(&content, "poiname")]);
            if label.is_empty() {
                "[位置]".into()
            } else {
                format!("[位置] {label}")
            }
        }
        49 => parse_type49(&content),
        50 => parse_voip_message(&content),
        10000 => clean_system_message(&content),
        244813135921 => {
            let t = extract_xml_value(&content, "title");
            if t.is_empty() {
                "[引用消息]".into()
            } else {
                t
            }
        }
        266287972401 => clean_pat_message(&content),
        81604378673 => "[聊天记录]".into(),
        8594229559345 => "[红包]".into(),
        8589934592049 => "[转账]".into(),
        _ => {
            if xml_type == "87" {
                let t = extract_xml_value(&content, "textannouncement");
                return if t.is_empty() { "[群公告]".into() } else { format!("[群公告] {t}") };
            }
            if xml_type == "57" {
                let t = extract_xml_value(&content, "title");
                return if t.is_empty() { "[引用消息]".into() } else { t };
            }
            if looks_app {
                return parse_type49(&content);
            }
            let generic = extract_xml_value(&content, "title");
            if !generic.is_empty() && generic.chars().count() < 100 {
                return generic;
            }
            if content.chars().count() > 200 {
                return message_type_label(local_type).into();
            }
            let stripped = strip_sender_prefix(&content);
            if stripped.is_empty() {
                message_type_label(local_type).into()
            } else {
                stripped
            }
        }
    }
}

#[derive(Default, Clone, Debug)]
pub struct Type49Info {
    pub xml_type: Option<String>,
    pub quoted_content: Option<String>,
    pub quoted_sender: Option<String>,
    pub link_title: Option<String>,
    pub link_url: Option<String>,
    pub link_thumb: Option<String>,
    pub app_msg_kind: Option<String>,
    pub app_msg_desc: Option<String>,
    pub app_msg_app_name: Option<String>,
    pub app_msg_source_name: Option<String>,
    pub app_msg_source_username: Option<String>,
    pub app_msg_thumb_url: Option<String>,
    pub app_msg_music_url: Option<String>,
    pub app_msg_data_url: Option<String>,
    pub app_msg_location_label: Option<String>,
    pub finder_nickname: Option<String>,
    pub finder_username: Option<String>,
    pub finder_cover_url: Option<String>,
    pub finder_avatar: Option<String>,
    pub finder_duration: Option<i64>,
    pub location_lat: Option<f64>,
    pub location_lng: Option<f64>,
    pub location_poiname: Option<String>,
    pub location_label: Option<String>,
    pub music_album_url: Option<String>,
    pub music_url: Option<String>,
    pub gift_image_url: Option<String>,
    pub gift_wish: Option<String>,
    pub gift_price: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub file_ext: Option<String>,
    pub file_md5: Option<String>,
    pub transfer_payer_username: Option<String>,
    pub transfer_receiver_username: Option<String>,
    pub chat_record_title: Option<String>,
}

fn parse_js_float(s: &str) -> Option<f64> {
    crate::sns::parse_float(s)
}

fn parse_js_int(s: &str) -> Option<i64> {
    rx(r"^\s*[+-]?\d+").find(s).and_then(|m| m.as_str().trim().parse().ok())
}

/// `parseType49Message`
pub fn parse_type49_message(content: &str) -> Type49Info {
    let mut r = Type49Info::default();
    if content.is_empty() {
        return r;
    }
    let xml_type = app_xml_type(content);
    if xml_type.is_empty() {
        return r;
    }
    let ev = |t: &str| extract_xml_value(content, t);
    let title = ev("title");
    let url = ev("url");
    let desc = first_non_empty(&[ev("des"), ev("description")]);
    let app_name = ev("appname");
    let source_name = ev("sourcename");
    let source_username = ev("sourceusername");
    let thumb_url = first_non_empty(&[ev("thumburl"), ev("cdnthumburl"), ev("cover"), ev("coverurl"), ev("thumb_url")]);
    let music_url = first_non_empty(&[ev("musicurl"), ev("playurl"), ev("songalbumurl")]);
    let data_url = first_non_empty(&[ev("dataurl"), ev("lowurl")]);
    let location_label = location_label_of(content);
    let finder_username = first_non_empty(&[ev("finderusername"), ev("finder_username"), ev("finderuser")]);
    let finder_nickname = first_non_empty(&[ev("findernickname"), ev("finder_nickname")]);
    let is_finder = xml_type == "51";
    let is_red_packet = xml_type == "2001";
    let is_music = xml_type == "3";
    let is_location = !location_label.is_empty();

    r.xml_type = Some(xml_type.clone());
    r.link_title = opt(title.clone());
    r.link_url = opt(url.clone());
    r.link_thumb = opt(thumb_url.clone());
    r.app_msg_desc = opt(desc);
    r.app_msg_app_name = opt(app_name.clone());
    r.app_msg_source_name = opt(source_name.clone());
    r.app_msg_source_username = opt(source_username.clone());
    r.app_msg_thumb_url = opt(thumb_url);
    r.app_msg_music_url = opt(music_url.clone());
    r.app_msg_data_url = opt(data_url.clone());
    r.app_msg_location_label = opt(location_label.clone());
    r.finder_username = opt(finder_username);
    r.finder_nickname = opt(finder_nickname);

    if is_finder {
        r.finder_cover_url = opt(first_non_empty(&[ev("thumbUrl"), ev("coverUrl"), ev("thumburl"), ev("coverurl")]));
        r.finder_avatar = opt(ev("avatar"));
        let d = first_non_empty(&[ev("videoPlayDuration"), ev("duration")]);
        if !d.is_empty() {
            if let Some(n) = parse_js_int(&d).filter(|n| *n > 0) {
                r.finder_duration = Some(n);
            }
        }
    }
    if is_location {
        let lat = first_non_empty(&[xml_attr(content, "location", "x"), xml_attr(content, "location", "latitude")]);
        let lng = first_non_empty(&[xml_attr(content, "location", "y"), xml_attr(content, "location", "longitude")]);
        r.location_lat = parse_js_float(&lat);
        r.location_lng = parse_js_float(&lng);
        r.location_poiname = opt(first_non_empty(&[xml_attr(content, "location", "poiname"), location_label.clone()]));
        r.location_label = opt(xml_attr(content, "location", "label"));
    }
    if is_music {
        r.music_album_url = opt(ev("songalbumurl"));
        r.music_url = opt(first_non_empty(&[music_url.clone(), data_url.clone(), url.clone()]));
    }
    let is_gift = xml_type == "115";
    if is_gift {
        r.gift_wish = opt(ev("wishmessage"));
        r.gift_image_url = opt(ev("skuimgurl"));
        r.gift_price = opt(ev("skuprice"));
    }

    let kind = if is_finder {
        "finder"
    } else if is_red_packet {
        "red-packet"
    } else if is_gift {
        "gift"
    } else if is_location {
        "location"
    } else if is_music {
        "music"
    } else if xml_type == "33" || xml_type == "36" {
        "miniapp"
    } else if xml_type == "6" {
        "file"
    } else if xml_type == "19" {
        "chat-record"
    } else if xml_type == "2000" {
        "transfer"
    } else if xml_type == "87" {
        "announcement"
    } else if xml_type == "57" {
        let q = parse_quote_message(content);
        r.quoted_content = q.0;
        r.quoted_sender = q.1;
        "quote"
    } else if xml_type == "53" {
        "solitaire"
    } else if (xml_type == "5" || xml_type == "49") && (source_username.starts_with("gh_") || app_name.contains("公众号") || !source_name.is_empty()) {
        "official-link"
    } else if !url.is_empty() {
        "link"
    } else {
        "card"
    };
    r.app_msg_kind = Some(kind.into());

    match xml_type.as_str() {
        "6" => {
            let name = if title.is_empty() { ev("filename") } else { title.clone() };
            r.link_title = opt(name.clone());
            r.file_name = opt(name.clone());
            let size = first_non_empty(&[ev("totallen"), ev("filesize")]);
            if !size.is_empty() {
                r.file_size = parse_js_int(&size);
            }
            let ext = ev("fileext");
            let md5 = first_non_empty(&[ev("md5"), ev("filemd5")]);
            if !ext.is_empty() {
                r.file_ext = Some(ext);
            } else if let Some(c) = rx(r"\.([^.]+)$").captures(&name) {
                r.file_ext = Some(c[1].to_string());
            }
            if !md5.is_empty() {
                r.file_md5 = Some(md5.to_lowercase());
            }
        }
        "19" => {
            r.chat_record_title = Some(if title.is_empty() { "聊天记录".into() } else { title });
        }
        "33" | "36" => {
            r.link_title = opt(title);
            r.link_url = opt(url);
            let thumb = first_non_empty(&[ev("thumburl"), ev("cdnthumburl")]);
            if !thumb.is_empty() {
                r.link_thumb = Some(thumb);
            }
        }
        "2000" => {
            let mut t = if title.is_empty() { "[转账]".to_string() } else { title };
            let memo = ev("pay_memo");
            let fee = ev("feedesc");
            if !memo.is_empty() {
                t = memo;
            } else if !fee.is_empty() {
                t = fee;
            }
            r.link_title = Some(t);
            r.transfer_payer_username = opt(ev("payer_username"));
            r.transfer_receiver_username = opt(ev("receiver_username"));
        }
        _ => {
            r.link_title = opt(title);
            r.link_url = opt(url);
            let thumb = first_non_empty(&[ev("thumburl"), ev("cdnthumburl")]);
            if !thumb.is_empty() {
                r.link_thumb = Some(thumb);
            }
        }
    }
    r
}

fn extract_partial_quoted_text(xml: &str, full: &str) -> String {
    if xml.is_empty() || full.is_empty() {
        return String::new();
    }
    let start_char = extract_xml_value(xml, "start");
    let end_char = extract_xml_value(xml, "end");
    let start_index = parse_js_int(&extract_xml_value(xml, "startindex"));
    let end_index = parse_js_int(&extract_xml_value(xml, "endindex"));
    if !start_char.is_empty() && !end_char.is_empty() {
        if let Some(sp) = full.find(&start_char) {
            let from = sp + start_char.len().saturating_sub(1);
            if let Some(rel) = full.get(from..).and_then(|s| s.find(&end_char)) {
                let ep = from + rel;
                if ep >= sp {
                    let sliced = full[sp..ep + end_char.len()].trim().to_string();
                    if !sliced.is_empty() {
                        return sliced;
                    }
                }
            }
        }
    }
    if let (Some(s), Some(e)) = (start_index, end_index) {
        if e >= s && s >= 0 {
            let chars: Vec<char> = full.chars().collect();
            let sliced: String = chars.iter().skip(s as usize).take((e - s + 1) as usize).collect::<String>().trim().to_string();
            if !sliced.is_empty() {
                return sliced;
            }
        }
    }
    String::new()
}

fn extract_preferred_quoted_text(refer_xml: &str) -> String {
    if refer_xml.is_empty() {
        return String::new();
    }
    let mut sources = vec![decode_html(refer_xml)];
    let raw_source = extract_xml_value(refer_xml, "msgsource");
    if !raw_source.is_empty() {
        let decoded = decode_html(&raw_source);
        if !decoded.is_empty() {
            sources.push(decoded);
        }
    }
    let full = sanitize_quoted_content(&extract_xml_value(&sources[0], "content"));
    let partial = extract_partial_quoted_text(&sources[0], &full);
    if !partial.is_empty() {
        return partial;
    }
    for source in &sources {
        for tag in ["selectedcontent", "selectedtext", "selectcontent", "selecttext", "quotecontent", "quotetext", "partcontent", "parttext", "excerpt", "summary", "preview"] {
            let v = sanitize_quoted_content(&extract_xml_value(source, tag));
            if !v.is_empty() {
                return v;
            }
        }
    }
    full
}

/// `parseQuoteMessage` → (content, sender)
pub fn parse_quote_message(content: &str) -> (Option<String>, Option<String>) {
    let normalized = decode_html(content);
    let (Some(start), Some(end)) = (normalized.find("<refermsg>"), normalized.find("</refermsg>")) else { return (None, None) };
    if end + 11 > normalized.len() || end < start {
        return (None, None);
    }
    let refer_xml = &normalized[start..end + 11];
    let mut display_name = extract_xml_value(refer_xml, "displayname");
    if !display_name.is_empty() && looks_like_wxid(&display_name) {
        display_name.clear();
    }
    let refer_content = extract_xml_value(refer_xml, "content");
    let refer_type = extract_xml_value(refer_xml, "type");
    let display_content = match refer_type.as_str() {
        "1" => extract_preferred_quoted_text(refer_xml),
        "3" => "[图片]".into(),
        "34" => "[语音]".into(),
        "43" => "[视频]".into(),
        "47" => "[动画表情]".into(),
        "49" => {
            let inner = parse_type49_message(&decode_html(&refer_content));
            if inner.xml_type.as_deref() == Some("57") && inner.link_title.is_some() {
                inner.link_title.unwrap()
            } else {
                "[链接]".into()
            }
        }
        "42" => "[名片]".into(),
        "48" => "[位置]".into(),
        _ => {
            if refer_content.is_empty() || refer_content.contains("wxid_") {
                "[消息]".into()
            } else {
                sanitize_quoted_content(&refer_content)
            }
        }
    };
    (Some(display_content), opt(display_name))
}

fn parse_media_quote(content: &str) -> Option<String> {
    let normalized = decode_html(content);
    let (start, end) = (normalized.find("<refermsg>")?, normalized.find("</refermsg>")?);
    if end + 11 > normalized.len() || end < start {
        return None;
    }
    let svrid = extract_xml_value(&normalized[start..end + 11], "svrid");
    (!svrid.is_empty()).then(|| format!("__SVRID__{svrid}__"))
}

// ───────────────────────── media token helpers ─────────────────────────

pub fn normalize_video_file_token(value: &str) -> Option<String> {
    let mut text = value.trim().to_lowercase();
    if text.is_empty() {
        return None;
    }
    text = rx(r"^.*[\\/]").replace(&text, "").to_string();
    text = rx(r"(?i)\.(?:mp4|mov|m4v|avi|mkv|flv|jpg|jpeg|png|gif|dat)$").replace(&text, "").to_string();
    text = rx(r"_thumb$").replace(&text, "").to_string();
    if let Some(c) = rx(r"^([a-f0-9]{16,64})(_raw)?$").captures(&text) {
        return Some(format!("{}{}", &c[1], c.get(2).map(|m| m.as_str()).unwrap_or("")));
    }
    // `([a-f0-9]{32})(?![a-f0-9])`, then the generic 16–64 variant, emulated on hex runs
    let runs: Vec<&str> = rx(r"[a-f0-9]+").find_iter(&text).map(|m| m.as_str()).collect();
    for run in &runs {
        if run.len() >= 32 {
            return Some(run[run.len() - 32..].to_string());
        }
    }
    for run in &runs {
        if run.len() >= 16 {
            let take = run.len().min(64);
            return Some(run[run.len() - take..].to_string());
        }
    }
    None
}

fn packed_bytes(row: &Value) -> Option<Vec<u8>> {
    let raw = row_field(
        row,
        &[
            "packed_info_data",
            "packedInfoData",
            "packed_info_blob",
            "packedInfoBlob",
            "packed_info",
            "packedInfo",
            "BytesExtra",
            "bytes_extra",
            "WCDB_CT_packed_info",
            "reserved0",
            "Reserved0",
            "WCDB_CT_Reserved0",
        ],
    )?;
    let s = raw.as_str()?.trim();
    if s.is_empty() {
        return None;
    }
    let compact: String = s.split_whitespace().collect();
    if compact.len() % 2 == 0 && compact.chars().all(|c| c.is_ascii_hexdigit()) {
        let bytes: Option<Vec<u8>> = (0..compact.len() / 2).map(|i| u8::from_str_radix(&compact[i * 2..i * 2 + 2], 16).ok()).collect();
        if let Some(b) = bytes {
            return Some(b);
        }
    }
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.decode(s).ok().or_else(|| Some(crate::sns::lenient_base64(s))).filter(|b| !b.is_empty())
}

fn video_token_from_packed(row: &Value) -> Option<String> {
    let bytes = packed_bytes(row)?;
    let mut candidates: Vec<String> = Vec::new();
    let mut current = String::new();
    for b in bytes {
        if b.is_ascii_hexdigit() {
            current.push(b as char);
            continue;
        }
        if current.len() >= 16 {
            candidates.push(std::mem::take(&mut current));
        }
        current.clear();
    }
    if current.len() >= 16 {
        candidates.push(current);
    }
    candidates.iter().find(|c| c.len() == 32).or_else(|| candidates.iter().find(|c| c.len() >= 16 && c.len() <= 64)).map(|c| c.to_lowercase())
}

fn video_md5_from_content(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let md5 = xml_attr(content, "videomsg", "md5");
    if !md5.is_empty() {
        return Some(md5.to_lowercase());
    }
    let raw = xml_attr(content, "videomsg", "rawmd5");
    if !raw.is_empty() {
        return Some(raw.to_lowercase());
    }
    opt(extract_xml_value(content, "md5")).map(|m| m.to_lowercase())
}

pub fn parse_video_file_name_from_row(row: &Value, content: &str) -> Option<String> {
    if let Some(t) = video_token_from_packed(row) {
        return Some(t);
    }
    let by_col = row_field(row, &["video_md5", "videoMd5", "raw_md5", "rawMd5", "video_file_name", "videoFileName"]).and_then(|v| v.as_str()).and_then(normalize_video_file_token);
    if by_col.is_some() {
        return by_col;
    }
    video_md5_from_content(content).and_then(|m| normalize_video_file_token(&m))
}

fn parse_voice_duration_seconds(content: &str) -> Option<i64> {
    let caps = rx(r#"(?i)(voicelength|length|time|playlength)\s*=\s*['"]?([0-9]+(?:\.[0-9]+)?)['"]?"#).captures(content)?;
    let raw: f64 = caps[2].parse().ok().filter(|f: &f64| f.is_finite() && *f > 0.0)?;
    Some(if raw > 1000.0 { (raw / 1000.0).round() as i64 } else { raw.round() as i64 })
}

#[derive(Default)]
struct EmojiInfo {
    cdn_url: Option<String>,
    md5: Option<String>,
}

fn percent_decode_lossy(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() + 0 && i + 2 <= bytes.len() - 1 + 0 {
            if let Ok(v) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| s.to_string())
}

fn parse_emoji_info(content: &str) -> EmojiInfo {
    let grab = |name: &str| -> Option<String> {
        let quoted = rx(&format!(r#"(?i){name}\s*=\s*['"]([^'"]+)['"]"#)).captures(content);
        let bare = || rx(&format!(r#"(?i){name}\s*=\s*([^'"\s/>]+)"#)).captures(content);
        quoted.or_else(bare).map(|c| c[1].replace("&amp;", "&")).map(|u| if u.contains('%') { percent_decode_lossy(&u) } else { u })
    };
    let md5 = rx(r#"(?i)md5\s*=\s*['"]([a-fA-F0-9]+)['"]"#).captures(content).or_else(|| rx(r"(?i)md5\s*=\s*([a-fA-F0-9]+)").captures(content)).map(|c| c[1].to_string());
    EmojiInfo { cdn_url: grab("cdnurl"), md5 }
}

// ───────────────────────── the model ─────────────────────────

#[derive(Default, Clone, Debug)]
pub struct ChatMessage {
    pub local_id: i64,
    pub server_id: i64,
    pub server_id_raw: String,
    pub local_type: i64,
    pub create_time: i64,
    pub sort_seq: i64,
    pub is_send: Option<i64>,
    pub sender_username: Option<String>,
    pub parsed_content: String,
    pub raw_content: String,
    pub emoji_cdn_url: Option<String>,
    pub emoji_md5: Option<String>,
    pub quoted_content: Option<String>,
    pub quoted_sender: Option<String>,
    pub image_md5: Option<String>,
    pub image_dat_name: Option<String>,
    pub video_md5: Option<String>,
    pub voice_duration_seconds: Option<i64>,
    pub aes_key: Option<String>,
    pub encryp_ver: Option<i64>,
    pub cdn_thumb_url: Option<String>,
    pub card_username: Option<String>,
    pub card_nickname: Option<String>,
    pub card_avatar_url: Option<String>,
    pub location_lat: Option<f64>,
    pub location_lng: Option<f64>,
    pub location_poiname: Option<String>,
    pub location_label: Option<String>,
    pub t49: Type49Info,
    pub message_key: String,
    pub db_path: Option<String>,
    pub table_name: Option<String>,
}

impl ChatMessage {
    pub fn link_title(&self) -> Option<&str> {
        self.t49.link_title.as_deref()
    }
    pub fn file_name(&self) -> Option<&str> {
        self.t49.file_name.as_deref()
    }
    pub fn xml_type(&self) -> Option<&str> {
        self.t49.xml_type.as_deref()
    }
    pub fn app_msg_kind(&self) -> Option<&str> {
        self.t49.app_msg_kind.as_deref()
    }

    /// Full JSON view (camelCase, `undefined` fields omitted).
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        o.insert("messageKey".into(), json!(self.message_key));
        o.insert("localId".into(), json!(self.local_id));
        o.insert("serverId".into(), json!(self.server_id));
        o.insert("serverIdRaw".into(), json!(self.server_id_raw));
        o.insert("localType".into(), json!(self.local_type));
        o.insert("createTime".into(), json!(self.create_time));
        o.insert("sortSeq".into(), json!(self.sort_seq));
        o.insert("isSend".into(), self.is_send.map(Value::from).unwrap_or(Value::Null));
        o.insert("senderUsername".into(), self.sender_username.clone().map(Value::from).unwrap_or(Value::Null));
        o.insert("parsedContent".into(), json!(self.parsed_content));
        o.insert("rawContent".into(), json!(self.raw_content));
        let t = &self.t49;
        let strs: [(&str, &Option<String>); 41] = [
            ("emojiCdnUrl", &self.emoji_cdn_url),
            ("emojiMd5", &self.emoji_md5),
            ("quotedContent", &self.quoted_content),
            ("quotedSender", &self.quoted_sender),
            ("imageMd5", &self.image_md5),
            ("imageDatName", &self.image_dat_name),
            ("videoMd5", &self.video_md5),
            ("aesKey", &self.aes_key),
            ("cdnThumbUrl", &self.cdn_thumb_url),
            ("linkTitle", &t.link_title),
            ("linkUrl", &t.link_url),
            ("linkThumb", &t.link_thumb),
            ("fileName", &t.file_name),
            ("fileExt", &t.file_ext),
            ("fileMd5", &t.file_md5),
            ("xmlType", &t.xml_type),
            ("appMsgKind", &t.app_msg_kind),
            ("appMsgDesc", &t.app_msg_desc),
            ("appMsgAppName", &t.app_msg_app_name),
            ("appMsgSourceName", &t.app_msg_source_name),
            ("appMsgSourceUsername", &t.app_msg_source_username),
            ("appMsgThumbUrl", &t.app_msg_thumb_url),
            ("appMsgMusicUrl", &t.app_msg_music_url),
            ("appMsgDataUrl", &t.app_msg_data_url),
            ("appMsgLocationLabel", &t.app_msg_location_label),
            ("finderNickname", &t.finder_nickname),
            ("finderUsername", &t.finder_username),
            ("finderCoverUrl", &t.finder_cover_url),
            ("finderAvatar", &t.finder_avatar),
            ("locationPoiname", &self.location_poiname),
            ("locationLabel", &self.location_label),
            ("musicAlbumUrl", &t.music_album_url),
            ("musicUrl", &t.music_url),
            ("giftImageUrl", &t.gift_image_url),
            ("giftWish", &t.gift_wish),
            ("giftPrice", &t.gift_price),
            ("cardUsername", &self.card_username),
            ("cardNickname", &self.card_nickname),
            ("cardAvatarUrl", &self.card_avatar_url),
            ("transferPayerUsername", &t.transfer_payer_username),
            ("transferReceiverUsername", &t.transfer_receiver_username),
        ];
        for (k, v) in strs {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        for (k, v) in [
            ("voiceDurationSeconds", self.voice_duration_seconds),
            ("encrypVer", self.encryp_ver),
            ("fileSize", t.file_size),
            ("finderDuration", t.finder_duration),
        ] {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        for (k, v) in [("locationLat", self.location_lat), ("locationLng", self.location_lng)] {
            if let Some(v) = v {
                o.insert(k.into(), json!(v));
            }
        }
        if let Some(v) = &t.chat_record_title {
            o.insert("chatRecordTitle".into(), json!(v));
        }
        Value::Object(o)
    }
}

// ───────────────────────── message keys ─────────────────────────

/// JS `encodeURIComponent`
pub fn encode_uri_component(s: &str) -> String {
    let mut out = String::new();
    for b in s.trim().bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

pub struct SourceInfo {
    pub db_name: String,
    pub table_name: String,
    pub db_path: String,
}

pub fn message_source_info(row: &Value) -> SourceInfo {
    let txt = |keys: &[&str]| keys.iter().filter_map(|k| row.get(*k)).map(value_to_trimmed_string).find(|s| !s.is_empty()).unwrap_or_default();
    let db_path = txt(&["_db_path", "db_path"]);
    let explicit = txt(&["db_name"]);
    let table_name = txt(&["table_name"]);
    let db_name = if !explicit.is_empty() { explicit } else { db_base_name(&db_path) };
    SourceInfo { db_name, table_name, db_path }
}

fn db_base_name(path: &str) -> String {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("");
    match name.rfind('.') {
        Some(i) if i > 0 => name[..i].to_string(),
        _ => name.to_string(),
    }
}

/// `buildMessageKey`
#[allow(clippy::too_many_arguments)]
pub fn build_message_key(local_id: i64, server_id: i64, create_time: i64, sort_seq: i64, sender: Option<&str>, local_type: i64, src: &SourceInfo) -> String {
    let (local_id, server_id, create_time, sort_seq) = (local_id.max(0), server_id.max(0), create_time.max(0), sort_seq.max(0));
    let sender = encode_uri_component(sender.unwrap_or(""));
    let db_name = if src.db_name.is_empty() { db_base_name(&src.db_path) } else { src.db_name.clone() };
    let scope = if !src.db_path.is_empty() { src.db_path.clone() } else { db_name };
    if local_id > 0 && !scope.is_empty() && !src.table_name.is_empty() {
        return format!("{}:{}:{}", encode_uri_component(&scope), encode_uri_component(&src.table_name), local_id);
    }
    if local_id > 0 && !scope.is_empty() {
        return format!("local:{}:{local_id}:{create_time}:{sort_seq}:{sender}:{local_type}", encode_uri_component(&scope));
    }
    if server_id > 0 {
        let scoped = if scope.is_empty() { server_id.to_string() } else { format!("{}:{server_id}", encode_uri_component(&scope)) };
        return format!("server:{scoped}:{create_time}:{sort_seq}:{local_id}:{sender}:{local_type}");
    }
    format!("fallback:{}:{create_time}:{sort_seq}:{local_id}:{sender}:{local_type}", encode_uri_component(&scope))
}

// ───────────────────────── isSend / sender ─────────────────────────

fn identity_keys(raw: &str) -> Vec<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Vec::new();
    }
    let lower = value.to_lowercase();
    let cleaned = clean_account_dir_name(value).to_lowercase();
    if !cleaned.is_empty() && cleaned != lower {
        vec![cleaned, lower]
    } else {
        vec![lower]
    }
}

/// `resolveMessageIsSend`
pub fn resolve_is_send(raw: Option<i64>, sender: Option<&str>, my_wxid: &str) -> Option<i64> {
    let sender_keys = identity_keys(sender.unwrap_or(""));
    if sender_keys.is_empty() {
        return raw;
    }
    let self_keys = identity_keys(my_wxid.trim());
    if self_keys.is_empty() {
        return raw;
    }
    let matched = sender_keys.iter().any(|s| self_keys.iter().any(|m| s == m || s.starts_with(&format!("{m}_")) || m.starts_with(&format!("{s}_"))));
    if matched && raw != Some(1) {
        return Some(1);
    }
    match raw {
        None => Some(if matched { 1 } else { 0 }),
        r => r,
    }
}

fn value_to_trimmed_string(v: &Value) -> String {
    match v {
        Value::String(s) => s.trim().to_string(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn raw_is_send(row: &Value) -> Option<i64> {
    let v = row.get("computed_is_send").filter(|v| !v.is_null()).or_else(|| row.get("is_send"))?;
    if v.is_null() {
        return None;
    }
    let text = value_to_trimmed_string(v);
    rx(r"^\s*[+-]?\d+").find(&text).and_then(|m| m.as_str().trim().parse().ok())
}

/// `mapRowsToMessagesLiteForApi`
pub fn map_rows_lite(rows: &[Value], my_wxid: &str) -> Vec<ChatMessage> {
    rows.iter()
        .map(|row| {
            let local_type = row_int(row, &["local_type"], 1);
            let create_time = get_timestamp_seconds(row);
            let sort_seq = row_int(row, &["sort_seq"], if create_time > 0 { create_time * 1000 } else { 0 });
            let content = decode_message_content(row);
            let raw = raw_is_send(row);
            let from_row = row.get("sender_username").map(value_to_trimmed_string).filter(|s| !s.is_empty()).or_else(|| extract_sender_from_content(&content));
            let is_send = resolve_is_send(raw, from_row.as_deref(), my_wxid);
            let sender = from_row.or_else(|| (is_send == Some(1) && !my_wxid.is_empty()).then(|| my_wxid.to_string()));
            let src = message_source_info(row);
            let (local_id, server_id) = (row_int(row, &["local_id"], 0), row_int(row, &["server_id"], 0));
            ChatMessage {
                message_key: build_message_key(local_id, server_id, create_time, sort_seq, sender.as_deref(), local_type, &src),
                db_path: Some(src.db_path.clone()).filter(|s| !s.is_empty()),
                table_name: Some(src.table_name.clone()).filter(|s| !s.is_empty()),
                local_id,
                server_id,
                server_id_raw: row.get("server_id").map(|v| normalize_unsigned_token(&value_to_trimmed_string(v))).unwrap_or_else(|| "0".into()),
                local_type,
                create_time,
                sort_seq,
                is_send,
                sender_username: sender,
                parsed_content: String::new(),
                raw_content: content,
                ..Default::default()
            }
        })
        .collect()
}

/// `mapRowsToMessages` (the model fields the API and CLI use).
pub fn map_rows(rows: &[Value], my_wxid: &str) -> Vec<ChatMessage> {
    rows.iter()
        .map(|row| {
            let content = decode_message_content(row);
            let local_type = row_int(row, &["local_type"], 1);
            let raw = raw_is_send(row);
            let sender_raw = row.get("sender_username").map(value_to_trimmed_string).filter(|s| !s.is_empty()).or_else(|| extract_sender_from_content(&content));
            let is_send = resolve_is_send(raw, sender_raw.as_deref(), my_wxid);
            let create_time = get_timestamp_seconds(row);
            let mut m = ChatMessage {
                local_id: row_int(row, &["local_id"], 0),
                server_id: row_int(row, &["server_id"], 0),
                server_id_raw: row.get("server_id").map(|v| normalize_unsigned_token(&value_to_trimmed_string(v))).unwrap_or_else(|| "0".into()),
                local_type,
                create_time,
                sort_seq: row_int(row, &["sort_seq"], create_time),
                is_send,
                sender_username: sender_raw,
                ..Default::default()
            };

            if local_type == 47 && !content.is_empty() {
                let e = parse_emoji_info(&content);
                m.emoji_cdn_url = e.cdn_url;
                m.emoji_md5 = e.md5;
                m.cdn_thumb_url = {
                    let t = rx(r#"(?i)thumburl\s*=\s*['"]([^'"]+)['"]"#).captures(&content).map(|c| c[1].replace("&amp;", "&"));
                    t.map(|u| if u.contains('%') { percent_decode_lossy(&u) } else { u })
                };
            } else if local_type == 3 && !content.is_empty() {
                let md5 = first_non_empty(&[extract_xml_value(&content, "md5"), xml_attr(&content, "img", "md5")]);
                m.image_md5 = opt(md5);
                m.aes_key = opt(xml_attr(&content, "img", "aeskey"));
                m.encryp_ver = parse_js_int(&xml_attr(&content, "img", "encrypver"));
                m.cdn_thumb_url = opt(xml_attr(&content, "img", "cdnthumburl"));
                m.image_dat_name = extract_image_dat_name_from_row(row, &content);
                m.quoted_content = parse_media_quote(&content);
            } else if local_type == 43 {
                m.video_md5 = parse_video_file_name_from_row(row, &content);
                m.quoted_content = parse_media_quote(&content);
            } else if local_type == 34 && !content.is_empty() {
                m.voice_duration_seconds = parse_voice_duration_seconds(&content);
                m.quoted_content = parse_media_quote(&content);
            } else if local_type == 42 && !content.is_empty() {
                m.card_username = opt(xml_attr(&content, "msg", "username"));
                m.card_nickname = opt(xml_attr(&content, "msg", "nickname"));
                m.card_avatar_url = opt(first_non_empty(&[xml_attr(&content, "msg", "bigheadimgurl"), xml_attr(&content, "msg", "smallheadimgurl")]));
            } else if local_type == 48 && !content.is_empty() {
                let lat = first_non_empty(&[xml_attr(&content, "location", "x"), xml_attr(&content, "location", "latitude")]);
                let lng = first_non_empty(&[xml_attr(&content, "location", "y"), xml_attr(&content, "location", "longitude")]);
                m.location_lat = parse_js_float(&lat);
                m.location_lng = parse_js_float(&lng);
                m.location_label = opt(first_non_empty(&[xml_attr(&content, "location", "label"), extract_xml_value(&content, "label")]));
                m.location_poiname = opt(first_non_empty(&[xml_attr(&content, "location", "poiname"), extract_xml_value(&content, "poiname")]));
            } else if (local_type == 49 || local_type == 8589934592049) && !content.is_empty() {
                m.t49 = parse_type49_message(&content);
                m.quoted_content = m.t49.quoted_content.clone();
                m.quoted_sender = m.t49.quoted_sender.clone();
            } else if local_type == 244813135921 || content.contains("<type>57</type>") {
                let (c, s) = parse_quote_message(&content);
                m.quoted_content = c;
                m.quoted_sender = s;
            }

            if content.contains("<appmsg") || content.contains("&lt;appmsg") {
                let t = parse_type49_message(&content);
                let cur = &mut m.t49;
                macro_rules! fill {
                    ($f:ident) => {
                        if cur.$f.is_none() {
                            cur.$f = t.$f.clone();
                        }
                    };
                }
                fill!(xml_type);
                fill!(link_title);
                fill!(link_url);
                fill!(link_thumb);
                fill!(file_name);
                fill!(file_size);
                fill!(file_ext);
                fill!(file_md5);
                fill!(app_msg_kind);
                fill!(app_msg_desc);
                fill!(app_msg_app_name);
                fill!(app_msg_source_name);
                fill!(app_msg_source_username);
                fill!(app_msg_thumb_url);
                fill!(app_msg_music_url);
                fill!(app_msg_data_url);
                fill!(app_msg_location_label);
                fill!(finder_nickname);
                fill!(finder_username);
                fill!(finder_cover_url);
                fill!(finder_avatar);
                fill!(finder_duration);
                fill!(music_album_url);
                fill!(music_url);
                fill!(gift_image_url);
                fill!(gift_wish);
                fill!(gift_price);
                fill!(chat_record_title);
                fill!(transfer_payer_username);
                fill!(transfer_receiver_username);
                if m.location_lat.is_none() {
                    m.location_lat = t.location_lat;
                }
                if m.location_lng.is_none() {
                    m.location_lng = t.location_lng;
                }
                if m.location_poiname.is_none() {
                    m.location_poiname = t.location_poiname.clone();
                }
                if m.location_label.is_none() {
                    m.location_label = t.location_label.clone();
                }
                if m.quoted_content.is_none() {
                    m.quoted_content = t.quoted_content.clone();
                }
                if m.quoted_sender.is_none() {
                    m.quoted_sender = t.quoted_sender.clone();
                }
            }
            m.parsed_content = parse_message_content(&content, local_type);
            m.raw_content = content;
            let src = message_source_info(row);
            m.message_key = build_message_key(m.local_id, m.server_id, m.create_time, m.sort_seq, m.sender_username.as_deref(), m.local_type, &src);
            m.db_path = Some(src.db_path).filter(|s| !s.is_empty());
            m.table_name = Some(src.table_name).filter(|s| !s.is_empty());
            m
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sender_prefix_variants() {
        assert_eq!(strip_sender_prefix("wxid_a:\nhello"), "hello");
        assert_eq!(strip_sender_prefix("wxid_a:<br/>hello"), "hello");
        assert_eq!(strip_sender_prefix("wxid_a: hi"), "hi");
        assert_eq!(strip_sender_prefix("http://x"), "http://x");
        assert_eq!(extract_sender_from_content("wxid_abcd:\nhello").as_deref(), Some("wxid_abcd"));
        assert_eq!(extract_sender_from_content("wxid_abcd:hello"), None);
    }

    #[test]
    fn display_text_per_type() {
        assert_eq!(parse_message_content("", 3), "[图片]");
        assert_eq!(parse_message_content("<msg/>", 3), "[图片]");
        assert_eq!(parse_message_content("wxid_a:\nhi", 1), "hi");
        assert_eq!(parse_message_content("<msg><appmsg><title>T</title><type>5</type><url>http://x</url></appmsg></msg>", 49), "[链接] T");
        assert_eq!(parse_message_content("<appmsg><title>f.pdf</title><type>6</type></appmsg>", 49), "[文件] f.pdf");
        assert_eq!(parse_message_content("<sysmsg><plain>Bob joined</plain></sysmsg>", 10000), "Bob joined");
        assert_eq!(parse_message_content("<msg><location label=\"Bund\"/></msg>", 48), "[位置] Bund");
    }

    #[test]
    fn type49_fields_and_quotes() {
        let xml = "<msg><appmsg><title>ok</title><type>57</type><refermsg><type>1</type><displayname>Bob</displayname><content>hi there</content><svrid>9</svrid></refermsg></appmsg></msg>";
        let t = parse_type49_message(xml);
        assert_eq!(t.app_msg_kind.as_deref(), Some("quote"));
        assert_eq!(t.quoted_content.as_deref(), Some("hi there"));
        assert_eq!(t.quoted_sender.as_deref(), Some("Bob"));
        let f = parse_type49_message("<appmsg><title>a.zip</title><type>6</type><totallen>123</totallen><md5>ABC</md5></appmsg>");
        assert_eq!(f.file_name.as_deref(), Some("a.zip"));
        assert_eq!(f.file_size, Some(123));
        assert_eq!(f.file_ext.as_deref(), Some("zip"));
        assert_eq!(f.file_md5.as_deref(), Some("abc"));
    }

    #[test]
    fn is_send_uses_self_identity() {
        assert_eq!(resolve_is_send(Some(0), Some("wxid_me"), "wxid_me"), Some(1));
        assert_eq!(resolve_is_send(Some(0), Some("wxid_bob"), "wxid_me"), Some(0));
        assert_eq!(resolve_is_send(None, Some("wxid_bob"), "wxid_me"), Some(0));
        assert_eq!(resolve_is_send(None, None, "wxid_me"), None);
    }

    #[test]
    fn message_keys_follow_the_desktop_format() {
        let rows = vec![
            json!({"local_id": "5", "create_time": "10", "local_type": "1", "message_content": "x", "_db_path": "/a b/message_0.db", "table_name": "Msg_abc"}),
            json!({"local_id": "6", "create_time": "11", "local_type": "1", "message_content": "x", "_db_path": "/a/message_1.db"}),
            json!({"local_id": "0", "server_id": "77", "create_time": "12", "local_type": "1", "message_content": "x"}),
        ];
        let m = map_rows(&rows, "wxid_me");
        assert_eq!(m[0].message_key, "%2Fa%20b%2Fmessage_0.db:Msg_abc:5");
        assert!(m[1].message_key.starts_with("local:%2Fa%2Fmessage_1.db:6:11:11:"), "{}", m[1].message_key);
        assert!(m[2].message_key.starts_with("server:77:12:12:0:"), "{}", m[2].message_key);
        assert_eq!(encode_uri_component("a b/é"), "a%20b%2F%C3%A9");
    }

    #[test]
    fn rows_map_to_messages() {
        let rows = vec![
            json!({"local_id": "5", "server_id": "9007199254740993", "create_time": "1700000000", "local_type": "1", "message_content": "wxid_bob:\nhello", "sender_username": "", "is_send": "0"}),
            json!({"local_id": "6", "create_time": "1700000100", "local_type": "3", "message_content": "<msg><img md5=\"aabbccddeeff00112233445566778899\"/></msg>", "is_send": "1"}),
        ];
        let m = map_rows(&rows, "wxid_me");
        assert_eq!(m[0].sender_username.as_deref(), Some("wxid_bob"));
        assert_eq!(m[0].parsed_content, "hello");
        assert_eq!(m[0].server_id_raw, "9007199254740993");
        assert_eq!(m[1].image_md5.as_deref(), Some("aabbccddeeff00112233445566778899"));
        assert_eq!(m[1].is_send, Some(1));
        assert_eq!(m[1].sender_username, None);
        let lite = map_rows_lite(&rows, "wxid_me");
        assert_eq!(lite[1].sender_username.as_deref(), Some("wxid_me"));
        assert_eq!(lite[1].sort_seq, 1700000100 * 1000);
    }
}
