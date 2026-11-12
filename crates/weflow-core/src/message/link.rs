//! Link cards: their title, address and text.

use super::xml::{
    extract_app_message_type, extract_xml_value, normalize_app_message_content, rx,
    strip_sender_prefix,
};

pub struct LinkCard {
    pub title: String,
    pub url: String,
}

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

pub fn extract_html_link_card(content: &str, local_type: i64) -> Option<LinkCard> {
    if content.is_empty() {
        return None;
    }
    let normalized = normalize_app_message_content(content);
    let is_app = local_type == 49 || normalized.contains("<appmsg") || normalized.contains("<msg>");
    if !is_app {
        return None;
    }
    let sub = extract_app_message_type(&normalized);
    if !sub.is_empty() && sub != "5" && sub != "49" {
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
    let mut title = extract_xml_value(&normalized, "title");
    if title.is_empty() {
        title = extract_xml_value(&normalized, "des");
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LinkStyle {
    Markdown,
    AppendUrl,
}

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
}
