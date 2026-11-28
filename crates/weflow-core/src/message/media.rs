//! 图片、视频、文件和位置：文件名、MD5，以及它们的 XML 中的信息。

use super::emoji::{extract_loose_hex_md5, percent_decode};
use super::xml::{
    extract_xml_attribute, extract_xml_value, last_hex_token, normalize_app_message_content, rx,
    xml_attr_text, xml_text, MsgXml,
};

/// 从 `<imgname>`、CDN 地址这类值中取出图片 `.dat` 文件名的十六进制部分（小写）：先找 `<hex>[.t].dat`；
/// 没有时去掉查询串、目录和扩展名，在 `_` 前面的部分中取 16～64 位的十六进制。取不出时为 `None`。
pub(super) fn normalize_image_dat_name_token(value: &str) -> Option<String> {
    let mut text = value.trim().to_string();
    if text.is_empty() {
        return None;
    }
    text = text.replace("&amp;", "&");
    text = percent_decode(&text);
    if let Some(name) = hex_dat_name(&text) {
        return Some(name);
    }
    let no_query = text.split(['?', '#']).next().unwrap_or("").to_string();
    let base = rx(r"^.*[\\/]").replace(&no_query, "").to_string();
    // 下面的正则要有 `.dat` 才能匹配；只有这几个字母的 ASCII 大小写折叠成它们，所以先按 ASCII 查一遍不会漏掉
    let base = if crate::xml::find_ci(&base, 0, ".dat").is_some() {
        rx(r"(?i)\.(?:t\.)?dat$").replace(&base, "").to_string()
    } else {
        base
    };
    let base = base.trim().to_string();
    if base.is_empty() {
        return None;
    }
    let cdn_token = if base.contains('_') {
        base.split('_').next().unwrap_or("").to_string()
    } else {
        base
    };
    // 相当于 `^[a-fA-F0-9]{16,64}$`
    if (16..=64).contains(&cdn_token.len()) && cdn_token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(cdn_token.to_lowercase());
    }
    if let Some(t) = last_hex_token(&cdn_token, 32, 32) {
        return Some(t.to_lowercase());
    }
    last_hex_token(&cdn_token, 16, 64).map(|t| t.to_lowercase())
}

/// 图片消息的 XML 中 `.dat` 文件的名字：取元素 `<imgname>`、`<cdnmidimgurl>`、`<cdnthumburl>` 和 `<img>`
/// 的同名属性中第一个不为空的，再按 [`normalize_image_dat_name_token`] 取出。
pub(crate) fn extract_image_dat_name(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let candidates = [
        extract_xml_value(content, "imgname"),
        extract_xml_value(content, "cdnmidimgurl"),
        extract_xml_value(content, "cdnthumburl"),
        extract_xml_attribute(content, "img", "imgname"),
        extract_xml_attribute(content, "img", "cdnmidimgurl"),
        extract_xml_attribute(content, "img", "cdnthumburl"),
    ];
    let pick = candidates
        .into_iter()
        .find(|c| !c.is_empty())
        .unwrap_or_default();
    normalize_image_dat_name_token(&pick)
}

/// 行的 `packed_info` 中图片 `.dat` 文件的名字（小写）：把其中可打印的字节当作文字，取 `<hex>[.t].dat` 的名字，
/// 没有时取第一段至少 16 位的十六进制数字。
pub(crate) fn dat_name_in_packed_info(bytes: &[u8]) -> Option<String> {
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
    if let Some(name) = hex_dat_name(&text) {
        return Some(name);
    }
    // 相当于 `([0-9a-fA-F]{16,})`：第一段至少 16 位的十六进制数字
    let h = text.as_bytes();
    let mut i = 0;
    while i < h.len() {
        let run = h[i..].iter().take_while(|b| b.is_ascii_hexdigit()).count();
        if run >= 16 {
            return Some(text[i..i + run].to_lowercase());
        }
        i += run.max(1);
    }
    None
}

/// `(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat`：`text` 中 `.dat` 文件名的十六进制部分，转成小写。
fn hex_dat_name(text: &str) -> Option<String> {
    // 正则要有 `.dat` 才能匹配；只有这几个字母的 ASCII 大小写折叠成它们，所以先按 ASCII 查一遍不会漏掉
    crate::xml::find_ci(text, 0, ".dat")?;
    rx(r"(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat")
        .captures(text)
        .map(|c| c[1].to_lowercase())
}

/// 图片的 MD5（小写）：`<img>` 的 `md5` 属性，没有时取 `<md5>` 元素。
pub fn extract_image_md5(content: &str) -> Option<String> {
    crate::xml::spaced_hex_attribute(content, "<img", "md5")
        .map(str::to_ascii_lowercase)
        .or_else(|| crate::xml::text_inner(content, "md5").map(str::to_lowercase))
}

/// 视频的 MD5（小写）：`<videomsg>` 的 `md5` 属性，没有时取 `<md5>` 元素。
pub(super) fn extract_video_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    crate::xml::spaced_hex_attribute(content, "<videomsg", "md5")
        .map(str::to_ascii_lowercase)
        .or_else(|| crate::xml::text_inner(content, "md5").map(str::to_lowercase))
}

/// app 消息带的文件，各字段与 [`super::ExportMsg`] 的同名字段相同。
pub(super) struct FileMeta {
    pub xml_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub file_ext: Option<String>,
    pub file_md5: Option<String>,
}

/// 从 app 消息的 [`MsgXml`] 中读出它带的文件（[`FileMeta`]）；不像 app 消息或没有 `<type>` 时为 `None`。
pub(super) fn extract_file_app_message_meta_in(x: &MsgXml<'_>) -> Option<FileMeta> {
    let normalized = x.once();
    if normalized.is_empty() || (!normalized.contains("<appmsg") && !normalized.contains("<msg>")) {
        return None;
    }
    let xml_type = x.app_type_twice().to_string();
    if xml_type.is_empty() {
        return None;
    }
    let first = |keys: &[&str]| {
        keys.iter()
            .map(|k| xml_text(normalized, k))
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    };
    let raw_name = first(&["filename", "title"]);
    let raw_ext = xml_text(normalized, "fileext");
    let raw_size = first(&["totallen", "datasize", "filesize"]);
    let mut raw_md5 = extract_xml_value(normalized, "md5");
    if raw_md5.is_empty() {
        raw_md5 = extract_xml_attribute(normalized, "appattach", "md5");
    }
    if raw_md5.is_empty() {
        raw_md5 = extract_loose_hex_md5(normalized).unwrap_or_default();
    }
    let size = raw_size.trim().parse::<i64>().ok().filter(|n| *n > 0);
    // `xml_text` 已经解码过实体（#1199）
    let non_empty = |s: String| (!s.is_empty()).then_some(s);
    Some(FileMeta {
        xml_type: Some(xml_type),
        file_name: non_empty(raw_name),
        file_size: size,
        file_ext: non_empty(raw_ext),
        file_md5: if raw_md5.len() == 32 && raw_md5.chars().all(|c| c.is_ascii_hexdigit()) {
            Some(raw_md5.to_lowercase())
        } else {
            None
        },
    })
}

/// 位置消息的（纬度，经度，地点名称，标签）。
type LocationMeta = (Option<f64>, Option<f64>, Option<String>, Option<String>);

/// 位置消息（`local_type` 48）的坐标、地点名称和标签：坐标取 `<location>` 的 `x`、`y` 属性（或 `latitude`、
/// `longitude`）；四项都没有时为 `None`。
pub(super) fn extract_location_meta(content: &str, local_type: i64) -> Option<LocationMeta> {
    if content.is_empty() || local_type != 48 {
        return None;
    }
    let n = normalize_app_message_content(content);
    let pick = |a: String, b: String| if !a.is_empty() { a } else { b };
    let raw_lat = pick(
        extract_xml_attribute(&n, "location", "x"),
        extract_xml_attribute(&n, "location", "latitude"),
    );
    let raw_lng = pick(
        extract_xml_attribute(&n, "location", "y"),
        extract_xml_attribute(&n, "location", "longitude"),
    );
    let poiname = {
        let a = xml_attr_text(&n, "location", "poiname");
        if !a.is_empty() {
            a
        } else {
            let b = xml_text(&n, "poiname");
            if !b.is_empty() {
                b
            } else {
                xml_text(&n, "poiName")
            }
        }
    };
    let label = pick(
        xml_attr_text(&n, "location", "label"),
        xml_text(&n, "label"),
    );
    let lat = raw_lat.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let lng = raw_lng.trim().parse::<f64>().ok().filter(|v| v.is_finite());
    let poi = if poiname.is_empty() {
        None
    } else {
        Some(poiname)
    };
    let lab = if label.is_empty() { None } else { Some(label) };
    if lat.is_none() && lng.is_none() && poi.is_none() && lab.is_none() {
        return None;
    }
    Some((lat, lng, poi, lab))
}

/// 位置消息的文字 `[位置] <地点名称> <标签> (<纬度>,<经度>)`：没有的部分不写，标签与地点名称相同时只写一次；
/// 都没有时为 `[位置]`。
pub(super) fn location_text(content: &str) -> String {
    let n = normalize_app_message_content(content);
    let first = |vals: Vec<String>| vals.into_iter().find(|v| !v.is_empty()).unwrap_or_default();
    let poiname = first(vec![
        xml_attr_text(&n, "location", "poiname"),
        xml_text(&n, "poiname"),
        xml_text(&n, "poiName"),
    ]);
    let label = first(vec![
        xml_attr_text(&n, "location", "label"),
        xml_text(&n, "label"),
    ]);
    let lat = first(vec![
        extract_xml_attribute(&n, "location", "x"),
        extract_xml_attribute(&n, "location", "latitude"),
    ]);
    let lng = first(vec![
        extract_xml_attribute(&n, "location", "y"),
        extract_xml_attribute(&n, "location", "longitude"),
    ]);
    let mut parts: Vec<String> = Vec::new();
    if !poiname.is_empty() {
        parts.push(poiname.clone());
    }
    if !label.is_empty() && label != poiname {
        parts.push(label);
    }
    if !lat.is_empty() && !lng.is_empty() {
        parts.push(format!("({lat},{lng})"));
    }
    if parts.is_empty() {
        "[位置]".into()
    } else {
        format!("[位置] {}", parts.join(" "))
    }
}
