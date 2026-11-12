//! Pictures, videos, files and locations: the names and MD5s of their files and what their XML says.

use super::emoji::{extract_loose_hex_md5, percent_decode};
use super::xml::{
    decode_html_entities, extract_xml_attribute, extract_xml_value, last_hex_token,
    normalize_app_message_content, rx, MsgXml,
};

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
    // the regex below needs `.dat`, and no character but these letters' ASCII cases folds to them
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
    // `^[a-fA-F0-9]{16,64}$`
    if (16..=64).contains(&cdn_token.len()) && cdn_token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Some(cdn_token.to_lowercase());
    }
    if let Some(t) = last_hex_token(&cdn_token, 32, 32) {
        return Some(t.to_lowercase());
    }
    last_hex_token(&cdn_token, 16, 64).map(|t| t.to_lowercase())
}

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

/// The name of an image's `.dat` file in a row's packed info: its printable bytes as text, a `<hex>[.t].dat` name
/// or else a long run of hex digits.
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
    // `([0-9a-fA-F]{16,})`: the first run of at least 16 hex digits
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

/// `(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat`: the hex name of a `.dat` file in `text`, lowercased.
fn hex_dat_name(text: &str) -> Option<String> {
    // the regex needs `.dat`, and no character but these letters' ASCII cases folds to them
    crate::xml::find_ci(text, 0, ".dat")?;
    rx(r"(?i)([0-9a-fA-F]{8,})(?:\.t)?\.dat")
        .captures(text)
        .map(|c| c[1].to_lowercase())
}

pub fn extract_image_md5(content: &str) -> Option<String> {
    crate::xml::spaced_hex_attribute(content, "<img", "md5")
        .map(str::to_ascii_lowercase)
        .or_else(|| crate::xml::text_inner(content, "md5").map(str::to_lowercase))
}

pub(super) fn extract_video_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    crate::xml::spaced_hex_attribute(content, "<videomsg", "md5")
        .map(str::to_ascii_lowercase)
        .or_else(|| crate::xml::text_inner(content, "md5").map(str::to_lowercase))
}

pub(super) struct FileMeta {
    pub xml_type: Option<String>,
    pub file_name: Option<String>,
    pub file_size: Option<i64>,
    pub file_ext: Option<String>,
    pub file_md5: Option<String>,
}

/// The file an app message carries ([`FileMeta`]), read from its [`MsgXml`].
pub(super) fn extract_file_app_message_meta_in(x: &MsgXml) -> Option<FileMeta> {
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
            .map(|k| extract_xml_value(normalized, k))
            .find(|v| !v.is_empty())
            .unwrap_or_default()
    };
    let raw_name = first(&["filename", "title"]);
    let raw_ext = extract_xml_value(normalized, "fileext");
    let raw_size = first(&["totallen", "datasize", "filesize"]);
    let mut raw_md5 = extract_xml_value(normalized, "md5");
    if raw_md5.is_empty() {
        raw_md5 = extract_xml_attribute(normalized, "appattach", "md5");
    }
    if raw_md5.is_empty() {
        raw_md5 = extract_loose_hex_md5(normalized).unwrap_or_default();
    }
    let size = raw_size.trim().parse::<i64>().ok().filter(|n| *n > 0);
    let non_empty = |s: String| {
        let t = decode_html_entities(&s).trim().to_string();
        if t.is_empty() {
            None
        } else {
            Some(t)
        }
    };
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

/// (latitude, longitude, label, poi name) of a location message.
type LocationMeta = (Option<f64>, Option<f64>, Option<String>, Option<String>);

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
        let a = extract_xml_attribute(&n, "location", "poiname");
        if !a.is_empty() {
            a
        } else {
            let b = extract_xml_value(&n, "poiname");
            if !b.is_empty() {
                b
            } else {
                extract_xml_value(&n, "poiName")
            }
        }
    };
    let label = pick(
        extract_xml_attribute(&n, "location", "label"),
        extract_xml_value(&n, "label"),
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

pub(super) fn location_text(content: &str) -> String {
    let n = normalize_app_message_content(content);
    let first = |vals: Vec<String>| vals.into_iter().find(|v| !v.is_empty()).unwrap_or_default();
    let poiname = first(vec![
        extract_xml_attribute(&n, "location", "poiname"),
        extract_xml_value(&n, "poiname"),
        extract_xml_value(&n, "poiName"),
    ]);
    let label = first(vec![
        extract_xml_attribute(&n, "location", "label"),
        extract_xml_value(&n, "label"),
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
