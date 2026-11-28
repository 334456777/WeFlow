//! 链接卡片：标题、地址和文字。

use super::appmsg::kind_of_app_type;
use super::xml::{
    extract_app_message_type, extract_xml_value, normalize_app_message_content, rx,
    strip_sender_prefix, xml_text,
};

/// 链接消息的卡片。
pub struct LinkCard {
    /// 标题；消息没有标题时为地址。
    pub title: String,
    /// `http:` 或 `https:` 地址。
    pub url: String,
}

/// 把 `raw` 规范化成可以打开的 `http:` 或 `https:` 地址：`&amp;` 换成 `&`，以 `//` 开头的和像域名的补上
/// `https://`；其他协议（如 `javascript:`）、含空格的和太短的为空字符串。
pub(super) fn normalize_http_link_url(raw: &str) -> String {
    let value = raw.trim().replace("&amp;", "&");
    if value.is_empty() {
        return String::new();
    }
    let parse_http = |candidate: &str| -> String {
        let lower = candidate.to_ascii_lowercase();
        if (lower.starts_with("http://") || lower.starts_with("https://"))
            && candidate.len() > 8
            && !candidate.contains(' ')
        {
            candidate.to_string()
        } else {
            String::new()
        }
    };
    if let Some(rest) = value.strip_prefix("//") {
        return parse_http(&format!("https://{rest}"));
    }
    let direct = parse_http(&value);
    if !direct.is_empty() {
        return direct;
    }
    let has_scheme = rx(r"^[a-zA-Z][a-zA-Z0-9+.-]*:").is_match(&value);
    let domain_like = rx(r"^[a-zA-Z0-9.-]+\.[a-zA-Z]{2,}(?:[/:?#].*)?$").is_match(&value);
    if !has_scheme && domain_like {
        return parse_http(&format!("https://{value}"));
    }
    String::new()
}

/// 链接消息的卡片：`<type>` 在表中算作链接（4、5、49）或没有 `<type>` 的 app 消息，取第一个能打开的地址；
/// 标题依次取 `<title>`、`<des>` 和地址，去掉发送者前缀。没有能打开的地址时为 `None`。
pub fn extract_html_link_card(content: &str, local_type: i64) -> Option<LinkCard> {
    if content.is_empty() {
        return None;
    }
    let normalized = normalize_app_message_content(content);
    let is_app = local_type == 49 || normalized.contains("<appmsg") || normalized.contains("<msg>");
    if !is_app {
        return None;
    }
    // 表中算作链接的 `<type>`（4、5、49）都有卡片，与种类的判定一致（#1191）
    let sub = extract_app_message_type(&normalized);
    if !sub.is_empty() && kind_of_app_type(&sub) != Some("link") {
        return None;
    }
    let url = [
        "url",
        "shareurlopen",
        "shareurloriginal",
        "shareurl",
        "shorturl",
        "dataurl",
        "lowurl",
        "streamvideoweburl",
        "weburl",
    ]
    .iter()
    .map(|k| normalize_http_link_url(&extract_xml_value(&normalized, k)))
    .find(|u| !u.is_empty())?;
    let mut title = xml_text(&normalized, "title");
    if title.is_empty() {
        title = xml_text(&normalized, "des");
    }
    if title.is_empty() {
        title = url.clone();
    }
    let title = strip_sender_prefix(&title);
    let title = if title.is_empty() { url.clone() } else { title };
    Some(LinkCard { title, url })
}

fn link_card_display_title(card: &LinkCard) -> String {
    let t = strip_sender_prefix(card.title.trim());
    if !t.is_empty() {
        t
    } else if !card.url.is_empty() {
        card.url.clone()
    } else {
        "链接".into()
    }
}

/// 链接写进导出文字的方式。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LinkStyle {
    /// `[标题](地址)`，ChatLab 用。
    Markdown,
    /// `[链接] 标题`，下一行是地址；标题就是地址时只写 `[链接]`。
    AppendUrl,
}

/// 链接消息在导出中的文字，按 `style` 写；不是链接或没有能打开的地址时为 `None`。
pub fn format_link_card_export_text(
    content: &str,
    local_type: i64,
    style: LinkStyle,
) -> Option<String> {
    let card = extract_html_link_card(content, local_type)?;
    if card.url.is_empty() {
        return None;
    }
    let title = link_card_display_title(&card);
    Some(match style {
        LinkStyle::Markdown => format!("[{title}]({})", card.url),
        LinkStyle::AppendUrl => {
            let prefix = if !title.is_empty() && title != card.url {
                format!("[链接] {title}")
            } else {
                "[链接]".to_string()
            };
            format!("{prefix}\n{}", card.url)
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn link_cards() {
        let xml = "<msg><appmsg><title>Hello</title><type>5</type><url>https://example.com/a?b=1&amp;c=2</url></appmsg></msg>";
        let c = extract_html_link_card(xml, 49).unwrap();
        assert_eq!(c.url, "https://example.com/a?b=1&c=2");
        assert_eq!(
            format_link_card_export_text(xml, 49, LinkStyle::Markdown).unwrap(),
            "[Hello](https://example.com/a?b=1&c=2)"
        );
        assert_eq!(
            normalize_http_link_url("example.com/x"),
            "https://example.com/x"
        );
        assert_eq!(normalize_http_link_url("javascript:alert(1)"), "");
    }

    /// 表中算作链接的每个 `<type>`（4、5、49）不论存为 `local_type` 49 还是 `(<type> << 32) | 49` 都有卡片：
    /// 导出写出它的地址，Arkme 带它的标题和地址。其他 `<type>` 没有卡片（#1191）。
    #[test]
    fn every_link_type_has_a_card() {
        use super::super::appmsg::extract_arkme_app_message_meta_in;
        use super::super::xml::MsgXml;

        let types = [
            "3", "76", "4", "5", "49", "6", "8", "19", "24", "33", "36", "51", "53", "57", "87",
            "2000", "2001",
        ];
        let mut links = 0;
        for ty in types {
            let xml = format!(
                "<msg><appmsg><title>T</title><type>{ty}</type><url>https://a.example/b</url></appmsg></msg>"
            );
            let kept_as: i64 = (ty.parse::<i64>().unwrap() << 32) | 49;
            for local_type in [49, kept_as] {
                let at = format!("<type> {ty} as {local_type}");
                let card = extract_html_link_card(&xml, local_type);
                if kind_of_app_type(ty) != Some("link") {
                    assert!(card.is_none(), "{at}");
                    continue;
                }
                assert_eq!(
                    card.map(|c| (c.title, c.url)),
                    Some(("T".into(), "https://a.example/b".into())),
                    "{at}"
                );
                assert_eq!(
                    format_link_card_export_text(&xml, local_type, LinkStyle::Markdown).as_deref(),
                    Some("[T](https://a.example/b)"),
                    "{at}"
                );
                assert_eq!(
                    format_link_card_export_text(&xml, local_type, LinkStyle::AppendUrl).as_deref(),
                    Some("[链接] T\nhttps://a.example/b"),
                    "{at}"
                );
                let meta = extract_arkme_app_message_meta_in(&MsgXml::new(&xml), local_type)
                    .unwrap_or_default();
                let field = |k: &str| meta.get(k).and_then(|v| v.as_str()).map(str::to_owned);
                assert_eq!(
                    (field("linkTitle"), field("linkUrl")),
                    (Some("T".into()), Some("https://a.example/b".into())),
                    "{at}"
                );
                links += 1;
            }
        }
        assert_eq!(links, 6, "4, 5 and 49, kept either way");
    }
}
