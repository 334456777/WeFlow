//! How replies quote and what forwarded chat records hold (#1242), the shapes the synthetic benchmark account copies.
//! Usage: `reply_record_probe <config.json>`
//!
//! Prints one JSON object of counts, shares and quantiles:
//! - replies (app messages of `<type>57`): how `<refermsg><content>` is written (plain text, XML escaped once, raw XML,
//!   CDATA), the `<type>` of the message quoted, the sizes, and for a quoted app message its own `<type>` and whether
//!   it quotes another message in turn;
//! - chat records (`<type>19`): how `<recorditem>` holds the record, the items per record against `<datalist count>`,
//!   and for each `datatype` of item its size, the elements and attributes it carries and the form of its `dataid`.
//!
//! No conversation, user name, message content, key or local path is printed: types are WeChat's numeric codes, an
//! element or attribute name is counted only outside CDATA and printed only when at least [`NAME_SHARE`] of the
//! items of its kind carry it, and everything else is counted. The config is only read.
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "counts of messages and quantile positions are small, non-negative values an f64 holds exactly"
)]

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use weflow_core::config::{expand_home, resolve_account_dir, ConfigStore};
use weflow_native::db::Account;

/// A name is printed when at least this share of the items of its kind carry it: a rarer one could be a word someone
/// wrote rather than a part of WeChat's format.
const NAME_SHARE: f64 = 0.01;

fn quantiles(mut v: Vec<i64>) -> Value {
    if v.is_empty() {
        return json!({"n": 0});
    }
    v.sort_unstable();
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    json!({"n": v.len(), "sum": v.iter().sum::<i64>(), "min": v[0], "p10": at(0.1), "p50": at(0.5), "p90": at(0.9),
        "p99": at(0.99), "max": v[v.len() - 1]})
}

fn shares<K: ToString>(counts: &BTreeMap<K, u64>) -> Value {
    let total = counts.values().sum::<u64>().max(1) as f64;
    let out: Map<String, Value> = counts
        .iter()
        .map(|(k, n)| (k.to_string(), json!({"n": n, "share": *n as f64 / total})))
        .collect();
    Value::Object(out)
}

/// The names carried by at least [`NAME_SHARE`] of `of` items, with the share of them that carry each.
fn common_names(counts: &BTreeMap<String, u64>, of: u64) -> Value {
    let of = of.max(1) as f64;
    let out: Map<String, Value> = counts
        .iter()
        .filter(|(_, n)| **n as f64 / of >= NAME_SHARE)
        .map(|(k, n)| (k.clone(), json!(*n as f64 / of)))
        .collect();
    Value::Object(out)
}

fn len(s: &str) -> i64 {
    i64::try_from(s.len()).unwrap_or(i64::MAX)
}

/// A column as text: a zstd blob unpacked, other bytes as they are.
fn column_text(value: rusqlite::types::ValueRef<'_>) -> String {
    match value {
        rusqlite::types::ValueRef::Text(t) => String::from_utf8_lossy(t).into_owned(),
        rusqlite::types::ValueRef::Blob(b) => {
            String::from_utf8_lossy(&zstd::decode_all(b).unwrap_or_else(|_| b.to_vec()))
                .into_owned()
        }
        _ => String::new(),
    }
}

/// The text between the first `<{name}>` and the `</{name}>` after it.
fn element<'x>(xml: &'x str, name: &str) -> Option<&'x str> {
    let open = format!("<{name}>");
    let start = xml.find(&open)? + open.len();
    let end = start + xml[start..].find(&format!("</{name}>"))?;
    Some(&xml[start..end])
}

/// The first `<type>` of an app message: the kind of card it is.
fn appmsg_type(xml: &str) -> Option<i64> {
    element(xml, "type")?.trim().parse().ok()
}

/// XML escaped once, read back.
fn unescape(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// `xml` without its CDATA sections, whose words are not names of WeChat's format.
fn without_cdata(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(start) = rest.find("<![CDATA[") {
        out.push_str(&rest[..start]);
        rest = rest[start..]
            .find("]]>")
            .map_or("", |end| &rest[start + end + 3..]);
    }
    out.push_str(rest);
    out
}

/// The names of the elements opened in `xml`, each once.
fn element_names(xml: &str) -> HashSet<String> {
    xml.match_indices('<')
        .filter_map(|(i, _)| {
            let name: String = xml[i + 1..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty() && name.len() <= 32).then_some(name)
        })
        .collect()
}

/// The names of the attributes of a start tag (`<dataitem a="1" b="2">`).
fn attribute_names(tag: &str) -> Vec<String> {
    tag.match_indices("=\"")
        .filter_map(|(i, _)| {
            let name: String = tag[..i]
                .chars()
                .rev()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            (!name.is_empty() && name.len() <= 32).then_some(name)
        })
        .collect()
}

/// The elements of an item whose form, not value, is counted ([`shape`]).
const SHAPED: &[&str] = &[
    "sourcetime",
    "srcMsgCreateTime",
    "srcMsgLocalid",
    "fromnewmsgid",
    "hashusername",
    "messageuuid",
    "datafmt",
    "duration",
    "datasize",
    "sourceheadurl",
    "cdnthumburl",
    "cdnthumbkey",
    "cdndataurl",
    "cdndatakey",
    "fullmd5",
    "thumbfullmd5",
    "thumbsize",
    "thumbwidth",
    "thumbheight",
    "filetype",
    "thumbfiletype",
];

/// The form of a value without the value. A run of up to four letters or digits keeps a `9` for each digit and an
/// `a` for each letter, so that a date shows its format; a longer run is only its class and length (`9{19}`, `a{6}`,
/// `x{40}` for letters and digits mixed), which says nothing of a hash or an address; other ASCII stays, any other
/// character is `*`, and the shape is cut at 80. A file format (`datafmt`) is kept as it is when it looks like one.
fn shape(name: &str, value: &str) -> String {
    let value = value.trim();
    if name == "datafmt"
        && (1..=8).contains(&value.len())
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '.')
    {
        return value.to_ascii_lowercase();
    }
    let mut out = String::new();
    let mut chars = value.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_ascii_alphanumeric() {
            let mut run = String::new();
            while let Some(&c) = chars.peek().filter(|c| c.is_ascii_alphanumeric()) {
                run.push(c);
                chars.next();
            }
            let class = |c: char| if c.is_ascii_digit() { '9' } else { 'a' };
            if run.len() <= 4 {
                out.extend(run.chars().map(class));
            } else {
                let kinds: HashSet<char> = run.chars().map(class).collect();
                let symbol = if kinds.len() > 1 { 'x' } else { class(c) };
                out.push_str(&format!("{symbol}{{{}}}", run.len()));
            }
        } else {
            out.push(if c.is_ascii() { c } else { '*' });
            chars.next();
        }
    }
    out.chars().take(80).collect()
}

fn attribute<'x>(tag: &'x str, name: &str) -> Option<&'x str> {
    let key = format!(" {name}=\"");
    let start = tag.find(&key)? + key.len();
    Some(&tag[start..start + tag[start..].find('"')?])
}

/// The outermost `<dataitem …>…</dataitem>` of a record, as (start tag, the whole element): an item that is a record
/// itself (`datatype="17"`) holds its own items as raw XML.
fn outer_items(xml: &str) -> Vec<(&str, &str)> {
    let mut items = Vec::new();
    let (mut depth, mut start) = (0usize, 0usize);
    let mut at = 0;
    while let Some(i) = xml[at..].find('<').map(|i| at + i) {
        let rest = &xml[i..];
        if rest.starts_with("</dataitem>") {
            if depth == 1 {
                let end = i + "</dataitem>".len();
                let tag_end = start + xml[start..].find('>').unwrap_or(0) + 1;
                items.push((&xml[start..tag_end], &xml[start..end]));
            }
            depth = depth.saturating_sub(1);
        } else if rest.starts_with("<dataitem")
            && rest[9..].starts_with([' ', '>'])
            && !rest[..rest.find('>').unwrap_or(0)].ends_with('/')
        {
            if depth == 0 {
                start = i;
            }
            depth += 1;
        }
        at = i + 1;
    }
    items
}

/// What an item says of itself: its content without the record it may hold.
fn own_part(item: &str) -> String {
    match (item.find("<recordxml>"), item.rfind("</recordxml>")) {
        (Some(a), Some(b)) if b > a => format!("{}{}", &item[..a], &item[b..]),
        _ => item.to_string(),
    }
}

#[derive(Default)]
struct Replies {
    n: u64,
    forms: BTreeMap<String, u64>,
    quoted_types: BTreeMap<i64, u64>,
    form_by_quoted_type: BTreeMap<i64, BTreeMap<String, u64>>,
    quoted_bytes_by_type: BTreeMap<i64, Vec<i64>>,
    reply_bytes: Vec<i64>,
    reply_bytes_by_form: BTreeMap<String, Vec<i64>>,
    quoted_app_types: BTreeMap<i64, u64>,
    /// Of the quoted messages written as XML: those that quote a message in turn, end with `</msg>`, are escaped
    /// once more inside, begin with a sender's name.
    quoting_in_turn: u64,
    ending_in_msg: u64,
    escaped_inside: u64,
    after_a_sender: u64,
    refermsg_names: BTreeMap<String, u64>,
    /// The forms of the values of `<refermsg>`'s fields other than the content.
    refermsg_shapes: BTreeMap<&'static str, BTreeMap<String, u64>>,
}

/// The fields of `<refermsg>` whose form, not value, is counted ([`shape`]).
const REFERMSG_SHAPED: &[&str] = &["svrid", "createtime", "msgsource", "strid"];

impl Replies {
    fn add(&mut self, xml: &str) {
        self.n += 1;
        self.reply_bytes.push(len(xml));
        let Some(refer) = element(xml, "refermsg") else {
            *self.forms.entry("no refermsg".into()).or_default() += 1;
            return;
        };
        for name in element_names(&without_cdata(refer)) {
            *self.refermsg_names.entry(name).or_default() += 1;
        }
        for name in REFERMSG_SHAPED {
            if let Some(value) = element(refer, name) {
                *self
                    .refermsg_shapes
                    .entry(name)
                    .or_default()
                    .entry(shape(name, value))
                    .or_default() += 1;
            }
        }
        let quoted_type = appmsg_type(refer).unwrap_or(-1);
        *self.quoted_types.entry(quoted_type).or_default() += 1;
        let content = element(refer, "content").map(str::trim);
        let form = match content {
            None => "no content",
            Some("") => "empty",
            Some(c) if c.starts_with("<![CDATA[") => "cdata",
            Some(c) if c.starts_with('<') => "raw xml",
            Some(c) if c.starts_with("&lt;?xml") => "escaped, from <?xml",
            Some(c) if c.starts_with("&lt;msg") => "escaped, from <msg",
            Some(c) if c.starts_with("&lt;") => "escaped, from another element",
            Some(c) if c.contains("&lt;msg") || c.contains("&lt;?xml") => "escaped, after a line",
            Some(_) => "plain text",
        };
        *self.forms.entry(form.into()).or_default() += 1;
        *self
            .form_by_quoted_type
            .entry(quoted_type)
            .or_default()
            .entry(form.into())
            .or_default() += 1;
        self.reply_bytes_by_form
            .entry(form.into())
            .or_default()
            .push(len(xml));
        let Some(content) = content else { return };
        self.quoted_bytes_by_type
            .entry(quoted_type)
            .or_default()
            .push(len(content));
        if !form.starts_with("escaped") {
            return;
        }
        let quoted = unescape(content);
        self.after_a_sender += u64::from(!quoted.starts_with('<'));
        self.ending_in_msg += u64::from(quoted.trim_end().ends_with("</msg>"));
        self.quoting_in_turn += u64::from(quoted.contains("<refermsg>"));
        self.escaped_inside += u64::from(quoted.contains("&lt;"));
        if quoted_type == 49 {
            *self
                .quoted_app_types
                .entry(appmsg_type(&quoted).unwrap_or(-1))
                .or_default() += 1;
        }
    }

    fn report(&self) -> Value {
        let escaped: u64 = self
            .forms
            .iter()
            .filter(|(k, _)| k.starts_with("escaped"))
            .map(|(_, n)| n)
            .sum();
        json!({
            "n": self.n,
            "forms": shares(&self.forms),
            "quoted_types": shares(&self.quoted_types),
            "form_by_quoted_type": self.form_by_quoted_type.iter().map(|(k, v)| (k.to_string(), shares(v))).collect::<Map<_, _>>(),
            "quoted_bytes_by_type": self.quoted_bytes_by_type.iter().map(|(k, v)| (k.to_string(), quantiles(v.clone()))).collect::<Map<_, _>>(),
            "reply_bytes": quantiles(self.reply_bytes.clone()),
            "reply_bytes_by_form": self.reply_bytes_by_form.iter().map(|(k, v)| (k.clone(), quantiles(v.clone()))).collect::<Map<_, _>>(),
            "quoted_app_types": shares(&self.quoted_app_types),
            "escaped": {
                "n": escaped,
                "quoting_in_turn": self.quoting_in_turn,
                "ending_in_msg": self.ending_in_msg,
                "escaped_inside": self.escaped_inside,
                "after_a_sender": self.after_a_sender,
            },
            "refermsg_elements": common_names(&self.refermsg_names, self.n),
            "refermsg_shapes": common_shapes(&self.refermsg_shapes, self.n),
        })
    }
}

#[derive(Default)]
struct Kind {
    n: u64,
    bytes: Vec<i64>,
    elements: BTreeMap<String, u64>,
    attributes: BTreeMap<String, u64>,
    /// The forms of the values of [`SHAPED`] elements and of the `htmlid` attribute.
    shapes: BTreeMap<&'static str, BTreeMap<String, u64>>,
}

/// Each name's common forms, as [`common_names`] has them.
fn common_shapes(shapes: &BTreeMap<&'static str, BTreeMap<String, u64>>, of: u64) -> Value {
    let out: Map<String, Value> = shapes
        .iter()
        .map(|(name, forms)| ((*name).to_string(), common_names(forms, of)))
        .collect();
    Value::Object(out)
}

#[derive(Default)]
struct Records {
    n: u64,
    forms: BTreeMap<String, u64>,
    items: Vec<i64>,
    count_matches: u64,
    without_count: u64,
    by_datatype: BTreeMap<String, Kind>,
    dataid_lengths: BTreeMap<usize, u64>,
    dataid_hex: u64,
    without_dataid: u64,
    repeated_dataid: u64,
}

impl Records {
    fn add(&mut self, xml: &str) {
        self.n += 1;
        let Some(holder) = element(xml, "recorditem") else {
            *self.forms.entry("no recorditem".into()).or_default() += 1;
            return;
        };
        let trimmed = holder.trim();
        let (form, record) = if let Some(inner) = trimmed.strip_prefix("<![CDATA[") {
            (
                "cdata",
                inner.strip_suffix("]]>").unwrap_or(inner).to_string(),
            )
        } else if trimmed.starts_with("&lt;") {
            ("escaped", unescape(trimmed))
        } else {
            ("raw", trimmed.to_string())
        };
        *self.forms.entry(form.into()).or_default() += 1;
        let items = outer_items(&record);
        self.items
            .push(i64::try_from(items.len()).unwrap_or(i64::MAX));
        let count = record
            .find("<datalist")
            .and_then(|i| attribute(&record[i..i + record[i..].find('>').unwrap_or(0)], "count"))
            .and_then(|c| c.parse::<usize>().ok());
        match count {
            Some(c) => self.count_matches += u64::from(c == items.len()),
            None => self.without_count += 1,
        }
        let mut seen = HashSet::new();
        for (tag, item) in items {
            let datatype = attribute(tag, "datatype").unwrap_or("none").to_string();
            let kind = self.by_datatype.entry(datatype).or_default();
            kind.n += 1;
            kind.bytes.push(len(item));
            let own = own_part(item);
            for name in element_names(&without_cdata(&own)) {
                *kind.elements.entry(name).or_default() += 1;
            }
            for name in attribute_names(tag) {
                *kind.attributes.entry(name).or_default() += 1;
            }
            for name in SHAPED {
                if let Some(value) = element(&own, name) {
                    *kind
                        .shapes
                        .entry(name)
                        .or_default()
                        .entry(shape(name, value))
                        .or_default() += 1;
                }
            }
            if let Some(id) = attribute(tag, "htmlid") {
                *kind
                    .shapes
                    .entry("htmlid")
                    .or_default()
                    .entry(shape("htmlid", id))
                    .or_default() += 1;
            }
            match attribute(tag, "dataid") {
                Some(id) => {
                    *self.dataid_lengths.entry(id.len()).or_default() += 1;
                    self.dataid_hex += u64::from(id.chars().all(|c| c.is_ascii_hexdigit()));
                    self.repeated_dataid += u64::from(!seen.insert(id.to_string()));
                }
                None => self.without_dataid += 1,
            }
        }
    }

    fn report(&self) -> Value {
        let items: u64 = self.by_datatype.values().map(|k| k.n).sum();
        let datatypes: BTreeMap<&String, u64> =
            self.by_datatype.iter().map(|(k, v)| (k, v.n)).collect();
        json!({
            "n": self.n,
            "forms": shares(&self.forms),
            "items_per_record": quantiles(self.items.clone()),
            "count_matches": self.count_matches,
            "without_count": self.without_count,
            "datatypes": shares(&datatypes),
            "by_datatype": self.by_datatype.iter().map(|(k, v)| (k.clone(), json!({
                "n": v.n,
                "bytes": quantiles(v.bytes.clone()),
                "elements": common_names(&v.elements, v.n),
                "attributes": common_names(&v.attributes, v.n),
                "shapes": common_shapes(&v.shapes, v.n),
            }))).collect::<Map<_, _>>(),
            "dataid": {
                "lengths": shares(&self.dataid_lengths),
                "hex_only": self.dataid_hex,
                "without": self.without_dataid,
                "repeated_in_a_record": self.repeated_dataid,
                "items": items,
            },
        })
    }
}

fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: reply_record_probe <config.json>")?;
    let config = ConfigStore::load(Path::new(&path)).context("read the WeFlow config")?;
    let profile = config
        .profiles
        .get(&config.current_profile)
        .context("current profile missing")?;
    let root = expand_home(
        profile
            .db_path
            .as_deref()
            .context("database path missing")?,
    );
    let me = profile.wxid.clone().unwrap_or_default();
    let root = if me.is_empty() {
        root
    } else {
        resolve_account_dir(&root.to_string_lossy(), &me)
    };
    let account = Account::new(
        root.join("db_storage"),
        profile
            .decrypt_key
            .as_deref()
            .context("database key missing")?,
    )?;
    account
        .test_connection()
        .context("open configured account")?;
    let (mut replies, mut records) = (Replies::default(), Records::default());
    for (_, parts) in account.message_tables_by_name()? {
        for part in &parts {
            account.with_db(&part.db, |conn| {
                let sql = format!(
                    "select message_content from \"{}\" where local_type & 4294967295 = 49",
                    part.table
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query([])?;
                while let Some(r) = rows.next()? {
                    let xml = column_text(r.get_ref(0)?);
                    match appmsg_type(&xml) {
                        Some(57) => replies.add(&xml),
                        Some(19) => records.add(&xml),
                        _ => {}
                    }
                }
                Ok(())
            })?;
        }
    }
    let out = json!({"replies": replies.report(), "records": records.report()});
    println!("{}", serde_json::to_string_pretty(&out)?);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nested_record_is_one_item_of_the_outer_one() {
        let xml = "<datalist count=\"2\"><dataitem datatype=\"17\" dataid=\"a\"><recordxml><recordinfo><datalist>\
                   <dataitem datatype=\"1\" dataid=\"b\"><datadesc>x</datadesc></dataitem></datalist></recordinfo>\
                   </recordxml></dataitem><dataitem datatype=\"2\" dataid=\"c\"><datafmt>jpg</datafmt></dataitem>\
                   </datalist>";
        let items = outer_items(xml);
        assert_eq!(items.len(), 2, "{items:?}");
        assert_eq!(attribute(items[0].0, "dataid"), Some("a"));
        assert!(!element_names(&own_part(items[0].1)).contains("datadesc"));
        assert_eq!(attribute_names(items[1].0), ["datatype", "dataid"]);
    }

    #[test]
    fn a_shape_keeps_the_form_and_drops_the_value() {
        assert_eq!(shape("sourcetime", "2024-1-5 09:30"), "9999-9-9 99:99");
        assert_eq!(shape("hashusername", "ab12名"), "aa99*");
        assert_eq!(shape("hashusername", "3f9c0a1b2e"), "x{10}");
        assert_eq!(shape("svrid", "1234567890123456789"), "9{19}");
        assert_eq!(
            shape("sourceheadurl", "https://h.example/p_1/abc12345"),
            "a{5}://a.a{7}/a_9/x{8}"
        );
        assert_eq!(shape("datafmt", "JPG"), "jpg");
        assert_eq!(shape("datafmt", "a file"), "a aaaa");
    }

    #[test]
    fn words_in_cdata_are_not_names() {
        let names = element_names(&without_cdata("<a><![CDATA[<word>]]></a><b/>"));
        assert_eq!(names, HashSet::from(["a".to_string(), "b".to_string()]));
    }
}
