//! Extended contact fields: labels, signature and region.
//!
//! Ports `getContactLabelNameMap`, `parseContactLabels`, `getContactSignature` and `getContactRegion` of
//! `chatService.ts`. WeChat keeps them in the protobuf `extra_buffer` column of the contact row (field 4
//! signature, 5 country, 6 province, 7 city, 30 label ids) plus the `contact_label` table; the region names are
//! translated to Chinese with the same lookup tables as the desktop app (`data/contact_region.json`).

use std::collections::HashMap;
use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RegionData {
    country_name_by_key: HashMap<String, String>,
    province_name_by_key: HashMap<String, String>,
    province_key_by_name: HashMap<String, String>,
    city_name_by_province_key: HashMap<String, HashMap<String, String>>,
    city_name_by_key: HashMap<String, String>,
}

fn data() -> &'static RegionData {
    static DATA: OnceLock<RegionData> = OnceLock::new();
    DATA.get_or_init(|| {
        serde_json::from_str(include_str!("../data/contact_region.json"))
            .expect("bundled region table is valid JSON")
    })
}

fn rx(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("static pattern"))
}

// ── generic helpers ──

/// `String(raw || '')`.
fn value_text(v: &Value) -> String {
    match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        Value::Bool(false) => String::new(),
        other => other.to_string(),
    }
}

/// Value of the first of `names` present in the row (exact name first, then ignoring case).
fn field<'a>(row: &'a Value, names: &[&str]) -> Option<&'a Value> {
    let obj = row.as_object()?;
    names.iter().find_map(|n| {
        obj.get(*n).or_else(|| {
            obj.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(n))
                .map(|(_, v)| v)
        })
    })
}

/// `normalizeContactRegionPart`: strips NULs and placeholder values such as `-` or `null`.
fn normalize_part(raw: &str) -> String {
    let text = raw.replace('\0', "");
    let text = text.trim();
    match text.to_lowercase().as_str() {
        "" | "-" | "--" | "—" | "null" | "undefined" | "none" => String::new(),
        _ => text.to_string(),
    }
}

fn has_cjk(s: &str) -> bool {
    s.chars().any(|c| ('\u{4e00}'..='\u{9fa5}').contains(&c))
}

// ── protobuf extra_buffer ──

fn extra_buffer_bytes(row: &Value) -> Option<Vec<u8>> {
    let text = value_text(field(row, &["extra_buffer", "extraBuffer"])?);
    let compact: String = text.split_whitespace().collect();
    if compact.len() < 2
        || compact.len() % 2 != 0
        || !compact.chars().all(|c| c.is_ascii_hexdigit())
    {
        return None;
    }
    let bytes: Vec<u8> = (0..compact.len())
        .step_by(2)
        .filter_map(|i| u8::from_str_radix(&compact[i..i + 2], 16).ok())
        .collect();
    Some(bytes).filter(|b| !b.is_empty())
}

fn read_varint(buf: &[u8], mut offset: usize) -> Option<(u64, usize)> {
    let (mut value, mut shift) = (0u64, 0u32);
    while offset < buf.len() {
        let byte = buf[offset];
        offset += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Some((value, offset));
        }
        shift += 7;
        if shift > 56 {
            return None;
        }
    }
    None
}

/// Top-level length-delimited fields numbered `target`, as trimmed UTF-8 text without NULs (empty ones dropped).
pub fn extra_buffer_strings(row: &Value, target: u64) -> Vec<String> {
    let Some(bytes) = extra_buffer_bytes(row) else {
        return Vec::new();
    };
    let mut values = Vec::new();
    let mut offset = 0;
    while offset < bytes.len() {
        let Some((tag, next)) = read_varint(&bytes, offset) else {
            break;
        };
        offset = next;
        let (field_no, wire) = (tag / 8, tag & 7);
        match wire {
            0 => match read_varint(&bytes, offset) {
                Some((_, n)) => offset = n,
                None => break,
            },
            1 => {
                if offset + 8 > bytes.len() {
                    break;
                }
                offset += 8;
            }
            2 => {
                let Some((len, n)) = read_varint(&bytes, offset) else {
                    break;
                };
                let len = len as usize;
                offset = n;
                if offset + len > bytes.len() {
                    break;
                }
                if field_no == target {
                    let text =
                        String::from_utf8_lossy(&bytes[offset..offset + len]).replace('\0', "");
                    if !text.trim().is_empty() {
                        values.push(text.trim().to_string());
                    }
                }
                offset += len;
            }
            5 => {
                if offset + 4 > bytes.len() {
                    break;
                }
                offset += 4;
            }
            _ => break,
        }
    }
    values
}

// ── labels ──

/// Pick the first candidate column that exists (case-insensitive), returning its real name.
pub fn pick_column(columns: &[String], candidates: &[&str]) -> Option<String> {
    candidates
        .iter()
        .find_map(|c| columns.iter().find(|n| n.eq_ignore_ascii_case(c)).cloned())
}

/// Label names of a contact row: a direct label column, any `*label*`/`*tag*` column, or the label ids in
/// `extra_buffer` field 30 looked up in `label_names` (id → name).
pub fn contact_labels(row: &Value, label_names: &HashMap<i64, String>) -> Vec<String> {
    fn split(value: &Value) -> Vec<String> {
        let items: Vec<String> = match value {
            Value::Array(a) => a.iter().map(|v| value_text(v).trim().to_string()).collect(),
            other => {
                static SEP: OnceLock<Regex> = OnceLock::new();
                let text = value_text(other);
                rx(&SEP, "[；;、|]+")
                    .replace_all(text.trim(), ",")
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .collect()
            }
        };
        let mut seen = Vec::new();
        for i in items.into_iter().filter(|s| !s.is_empty()) {
            if !seen.contains(&i) {
                seen.push(i);
            }
        }
        seen
    }
    if let Some(raw) = field(
        row,
        &[
            "label_list",
            "labelList",
            "labels",
            "label_names",
            "labelNames",
            "tags",
            "tag_list",
            "tagList",
        ],
    ) {
        let direct = split(raw);
        if !direct.is_empty() {
            return direct;
        }
    }
    if let Some(obj) = row.as_object() {
        for (key, value) in obj {
            let k = key.to_lowercase();
            if !(k.contains("label") || k.contains("tag"))
                || k.contains("img")
                || k.contains("head")
            {
                continue;
            }
            let fallback = split(value);
            if !fallback.is_empty() {
                return fallback;
            }
        }
    }
    static DIGITS: OnceLock<Regex> = OnceLock::new();
    let digits = rx(&DIGITS, r"\d+");
    let mut names: Vec<String> = Vec::new();
    for text in extra_buffer_strings(row, 30) {
        for m in digits.find_iter(&text) {
            let Ok(id) = m.as_str().parse::<i64>() else {
                continue;
            };
            if id <= 0 {
                continue;
            }
            if let Some(name) = label_names.get(&id) {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
        }
    }
    names
}

// ── signature ──

pub fn contact_signature(row: &Value) -> String {
    let normalize = |raw: &str| -> String {
        let text = raw.replace('\0', "");
        let text = text.trim();
        match text.to_lowercase().as_str() {
            "-" | "--" | "—" | "null" | "undefined" | "none" => String::new(),
            _ => text.to_string(),
        }
    };
    let names = [
        "signature",
        "sign",
        "personal_signature",
        "personalSignature",
        "profile",
        "introduction",
        "detail_description",
        "detailDescription",
        "description",
        "desc",
        "contact_description",
        "contactDescription",
    ];
    if let Some(v) = field(row, &names) {
        let direct = normalize(&value_text(v));
        if !direct.is_empty() {
            return direct;
        }
    }
    if let Some(obj) = row.as_object() {
        for (key, value) in obj {
            let k = key.to_lowercase();
            let candidate = [
                "sign",
                "signature",
                "profile",
                "intro",
                "description",
                "detail",
                "desc",
            ]
            .iter()
            .any(|t| k.contains(t));
            if !candidate
                || ["avatar", "img", "head", "label", "tag"]
                    .iter()
                    .any(|t| k.contains(t))
            {
                continue;
            }
            let text = normalize(&value_text(value));
            if !text.is_empty() {
                return text;
            }
        }
    }
    extra_buffer_strings(row, 4)
        .iter()
        .map(|s| normalize(s))
        .find(|s| !s.is_empty())
        .unwrap_or_default()
}

// ── region ──

fn lookup_key(raw: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    rx(&RE, r"[^a-z0-9\u{4e00}-\u{9fa5}]+")
        .replace_all(&raw.to_lowercase(), "")
        .into_owned()
}

fn lookup_candidates(raw: &str) -> Vec<String> {
    let normalized = lookup_key(raw);
    if normalized.is_empty() {
        return Vec::new();
    }
    let mut out = vec![normalized.clone()];
    let trimmed = normalized
        .trim_end_matches(|c: char| c.is_ascii_digit())
        .to_string();
    if !trimmed.is_empty() && trimmed != normalized {
        out.push(trimmed);
    }
    out
}

fn strip_suffixes(raw: &str, suffixes: &[&str]) -> String {
    let mut text = raw.trim().to_string();
    for s in suffixes {
        if let Some(t) = text.strip_suffix(s) {
            text = t.to_string();
        }
    }
    text.trim().to_string()
}

fn chinese_province(raw: &str) -> String {
    strip_suffixes(
        raw,
        &[
            "特别行政区",
            "维吾尔自治区",
            "壮族自治区",
            "回族自治区",
            "自治区",
            "省",
            "市",
        ],
    )
}

fn chinese_city(raw: &str) -> String {
    strip_suffixes(raw, &["特别行政区", "自治州", "地区", "盟", "林区", "市"])
}

fn province_lookup_key(raw: &str) -> String {
    let d = data();
    let candidates = lookup_candidates(raw);
    for c in &candidates {
        if let Some(k) = d.province_key_by_name.get(c) {
            return k.clone();
        }
        if d.province_name_by_key.contains_key(c) {
            return c.clone();
        }
    }
    candidates.into_iter().next().unwrap_or_default()
}

fn country_name(raw: &str) -> String {
    let text = normalize_part(raw);
    if text.is_empty() {
        return text;
    }
    lookup_candidates(&text)
        .iter()
        .find_map(|c| data().country_name_by_key.get(c).cloned())
        .unwrap_or(text)
}

fn province_name(raw: &str) -> String {
    let text = normalize_part(raw);
    if text.is_empty() {
        return text;
    }
    let candidates = lookup_candidates(&text);
    if candidates.is_empty() {
        return text;
    }
    let d = data();
    let key = province_lookup_key(&text);
    let mapped = d.province_name_by_key.get(&key).cloned().or_else(|| {
        candidates
            .iter()
            .find_map(|c| d.province_name_by_key.get(c).cloned())
    });
    if let Some(m) = mapped {
        return m;
    }
    if has_cjk(&text) {
        let stripped = chinese_province(&text);
        return if stripped.is_empty() { text } else { stripped };
    }
    text
}

fn city_name(raw: &str, province_raw: &str) -> String {
    let text = normalize_part(raw);
    if text.is_empty() {
        return text;
    }
    let candidates = lookup_candidates(&text);
    if candidates.is_empty() {
        return text;
    }
    let d = data();
    let key = province_lookup_key(province_raw);
    if !key.is_empty() {
        if let Some(by_province) = d.city_name_by_province_key.get(&key) {
            if let Some(m) = candidates.iter().find_map(|c| by_province.get(c)) {
                return m.clone();
            }
        }
    }
    if let Some(m) = candidates.iter().find_map(|c| d.city_name_by_key.get(c)) {
        return m.clone();
    }
    if has_cjk(&text) {
        let stripped = chinese_city(&text);
        return if stripped.is_empty() { text } else { stripped };
    }
    text
}

fn region_text(raw: &str) -> String {
    let text = normalize_part(raw);
    if text.is_empty() {
        return text;
    }
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    let tokens: Vec<String> = rx(&SPLIT, r"[\s,，、/|·]+")
        .split(&text)
        .map(normalize_part)
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.is_empty() {
        return text;
    }
    let mut province_context = String::new();
    let mapped: Vec<String> = tokens
        .iter()
        .map(|token| {
            let country = country_name(token);
            if country != *token {
                return country;
            }
            let province = province_name(token);
            if province != *token {
                province_context = province.clone();
                return province;
            }
            let city = city_name(token, &province_context);
            if city != *token {
                return city;
            }
            token.clone()
        })
        .collect();
    mapped.join(" ").trim().to_string()
}

fn hide_country(country: &str, has_province_or_city: bool) -> bool {
    if country.is_empty() {
        return true;
    }
    match country.to_lowercase().as_str() {
        "cn" | "chn" | "china" | "中国" => has_province_or_city,
        _ => false,
    }
}

/// `"广东 深圳"`-style region of a contact row (empty when nothing is known).
pub fn contact_region(row: &Value) -> String {
    let pick_by_tokens = |tokens: &[&str]| -> String {
        let Some(obj) = row.as_object() else {
            return String::new();
        };
        for (key, value) in obj {
            let k = key.to_lowercase();
            if k.is_empty()
                || k.contains("avatar")
                || k.contains("img")
                || k.contains("head")
                || !tokens.iter().any(|t| k.contains(t))
            {
                continue;
            }
            let text = normalize_part(&value_text(value));
            if !text.is_empty() {
                return text;
            }
        }
        String::new()
    };
    let direct = |names: &[&str], tokens: &[&str]| -> String {
        let first = field(row, names)
            .map(|v| normalize_part(&value_text(v)))
            .unwrap_or_default();
        if first.is_empty() {
            pick_by_tokens(tokens)
        } else {
            first
        }
    };
    let (direct_country, direct_province, direct_city) = (
        direct(&["country", "Country"], &["country"]),
        direct(&["province", "Province"], &["province"]),
        direct(&["city", "City"], &["city"]),
    );
    let direct_region = direct(
        &["region", "Region", "location", "area"],
        &["region", "location", "area", "addr", "address"],
    );
    if !direct_region.is_empty() {
        let normalized = region_text(&direct_region);
        let parts: Vec<String> = normalized
            .split_whitespace()
            .map(normalize_part)
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() > 1 && hide_country(&parts[0], true) {
            return parts[1..].join(" ").trim().to_string();
        }
        return normalized;
    }
    let extra = |n: u64| {
        normalize_part(
            extra_buffer_strings(row, n)
                .first()
                .map(String::as_str)
                .unwrap_or(""),
        )
    };
    let pick = |direct: &str, fallback: String| {
        if direct.is_empty() {
            fallback
        } else {
            direct.to_string()
        }
    };
    let country = country_name(&pick(&direct_country, extra(5)));
    let province_raw = pick(&direct_province, extra(6));
    let province = province_name(&province_raw);
    let city = city_name(&pick(&direct_city, extra(7)), &province_raw);
    let has_province_or_city = !province.is_empty() || !city.is_empty();
    let mut parts: Vec<String> = Vec::new();
    if !hide_country(&country, has_province_or_city) {
        parts.push(country);
    }
    if !province.is_empty() {
        parts.push(province.clone());
    }
    if !city.is_empty() && city != province {
        parts.push(city);
    }
    parts.join(" ").trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Protobuf bytes of the given (field, text) pairs as the lowercase hex WeChat rows carry.
    fn buffer(fields: &[(u64, &str)]) -> String {
        let mut out = Vec::new();
        for (n, text) in fields {
            let mut tag = (n << 3) | 2;
            while tag >= 0x80 {
                out.push((tag & 0x7f) as u8 | 0x80);
                tag >>= 7;
            }
            out.push(tag as u8);
            out.push(text.len() as u8);
            out.extend_from_slice(text.as_bytes());
        }
        out.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn protobuf_strings_are_read_by_field_number() {
        let row = json!({ "extra_buffer": buffer(&[(4, "  hello  "), (5, "CN"), (4, "second"), (30, "1,2")]) });
        assert_eq!(extra_buffer_strings(&row, 4), ["hello", "second"]);
        assert_eq!(extra_buffer_strings(&row, 5), ["CN"]);
        assert_eq!(extra_buffer_strings(&row, 30), ["1,2"]);
        assert!(extra_buffer_strings(&row, 9).is_empty());
        assert!(extra_buffer_strings(&json!({}), 4).is_empty());
        assert!(extra_buffer_strings(&json!({ "extra_buffer": "zz" }), 4).is_empty());
        // varint (wire type 0) and fixed fields are skipped; truncated payloads stop the scan
        let mut hex = String::from("0801"); // field 1 varint 1
        hex.push_str(&buffer(&[(4, "kept")]));
        assert_eq!(
            extra_buffer_strings(&json!({ "extra_buffer": hex }), 4),
            ["kept"]
        );
        assert!(
            extra_buffer_strings(&json!({ "extra_buffer": "2205ab" }), 4).is_empty(),
            "length runs past the end"
        );
    }

    #[test]
    fn signature_prefers_the_description_column_then_extra_buffer() {
        assert_eq!(contact_signature(&json!({ "description": " hi \0" })), "hi");
        assert_eq!(
            contact_signature(
                &json!({ "description": "-", "extra_buffer": buffer(&[(4, "from buffer")]) })
            ),
            "from buffer"
        );
        assert_eq!(
            contact_signature(&json!({ "extra_buffer": buffer(&[(4, "null"), (4, "second")]) })),
            "second"
        );
        assert_eq!(
            contact_signature(&json!({ "remark": "x", "big_head_url": "http://sign" })),
            ""
        );
        assert_eq!(
            contact_signature(
                &json!({ "detail_text": "found by key scan", "description_img": "no" })
            ),
            "found by key scan"
        );
        assert_eq!(
            contact_signature(&json!({ "description_img": "no", "label_desc": "no" })),
            "",
            "image, label and tag columns never count"
        );
        assert_eq!(
            contact_signature(&json!({ "my_signature_text": "scanned" })),
            "scanned"
        );
    }

    #[test]
    fn labels_come_from_columns_or_extra_buffer_ids() {
        let names: HashMap<i64, String> = [
            (1, "Family".to_string()),
            (2, "Work".to_string()),
            (3, "Gym".to_string()),
        ]
        .into();
        assert_eq!(
            contact_labels(&json!({ "labels": "A；B,A、C" }), &names),
            ["A", "B", "C"]
        );
        assert_eq!(
            contact_labels(&json!({ "label_list": ["x", "x", " y "] }), &names),
            ["x", "y"]
        );
        assert_eq!(
            contact_labels(
                &json!({ "extra_buffer": buffer(&[(30, "2,1,2,99,0")]) }),
                &names
            ),
            ["Work", "Family"]
        );
        assert_eq!(
            contact_labels(&json!({ "extra_buffer": buffer(&[(30, "7")]) }), &names),
            Vec::<String>::new()
        );
        assert!(
            contact_labels(&json!({ "username": "x", "head_img_md5": "abc" }), &names).is_empty()
        );
        assert_eq!(contact_labels(&json!({ "tag_x": "t1" }), &names), ["t1"]);
    }

    #[test]
    fn column_picking_ignores_case() {
        let cols = vec!["Label_ID_".to_string(), "label_name_".to_string()];
        assert_eq!(
            pick_column(&cols, &["label_id_", "id"]).as_deref(),
            Some("Label_ID_")
        );
        assert_eq!(pick_column(&cols, &["name"]), None);
    }

    #[test]
    fn regions_are_translated_and_the_domestic_country_is_hidden() {
        let r = |country: &str, province: &str, city: &str| {
            contact_region(
                &json!({ "extra_buffer": buffer(&[(5, country), (6, province), (7, city)]) }),
            )
        };
        assert_eq!(r("CN", "Guangdong", "Shenzhen"), "广东 深圳");
        assert_eq!(r("CN", "Beijing", ""), "北京");
        assert_eq!(
            r("CN", "北京", "北京市"),
            "北京",
            "a city equal to its province appears once"
        );
        assert_eq!(r("US", "California", ""), "美国 California");
        assert_eq!(r("JP", "", ""), "日本");
        assert_eq!(r("", "", ""), "");
        assert_eq!(
            r("CN", "", ""),
            "中国",
            "a bare domestic country is kept: nothing else says where"
        );
        assert_eq!(r("-", "null", "Hangzhou"), "杭州");
    }

    #[test]
    fn direct_region_columns_win_over_extra_buffer() {
        assert_eq!(
            contact_region(
                &json!({ "region": "CN Guangdong Shenzhen", "extra_buffer": buffer(&[(5, "JP")]) })
            ),
            "广东 深圳"
        );
        assert_eq!(contact_region(&json!({ "country": "JP" })), "日本");
        assert_eq!(
            contact_region(&json!({ "province": "浙江省", "city": "杭州市" })),
            "浙江 杭州"
        );
        assert_eq!(contact_region(&json!({ "username": "x" })), "");
    }

    #[test]
    fn chinese_names_lose_their_administrative_suffixes() {
        assert_eq!(chinese_province("新疆维吾尔自治区"), "新疆");
        assert_eq!(chinese_province("上海市"), "上海");
        assert_eq!(chinese_city("延边朝鲜族自治州"), "延边朝鲜族");
        assert_eq!(lookup_candidates("Shenzhen2"), ["shenzhen2", "shenzhen"]);
        assert!(lookup_candidates("---").is_empty());
    }
}
