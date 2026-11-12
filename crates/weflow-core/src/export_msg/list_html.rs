//! The HTML page of a list (`export sessions`, `export contacts` with `--format html`): `list.{zh,en}.html`, which
//! build.mjs makes from `list.html`, with the list as JSON in its `#wf-data`:
//!
//! `{v, lang, kind, exported, columns, rows, avatars}`: `kind` is `"sessions"` or `"contacts"`; `exported` the time
//! of the export in unix seconds; `columns` the names of the columns and `rows` a list of each row's values in that
//! order (the table the other formats write, see [`crate::export::list_table`]); `avatars` the path, from the page,
//! of each avatar written beside it, by username.

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};

use serde_json::Value;

use super::format::{html_escape, write_script_json};
use super::html::url_path;
use crate::locale;

const LIST_ZH: &str = include_str!("list.zh.html");
const LIST_EN: &str = include_str!("list.en.html");

/// What a list page shows: how it draws the rows, and its title.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    Sessions,
    Contacts,
}

impl ListKind {
    /// The name the page knows it by, which is also the export's file name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Sessions => "sessions",
            Self::Contacts => "contacts",
        }
    }

    fn title(self, lang: locale::Lang) -> &'static str {
        match (self, lang) {
            (Self::Sessions, locale::Lang::Zh) => "会话列表",
            (Self::Sessions, locale::Lang::En) => "Sessions",
            (Self::Contacts, locale::Lang::Zh) => "联系人",
            (Self::Contacts, locale::Lang::En) => "Contacts",
        }
    }
}

/// A list to write as a page.
#[derive(Debug)]
pub struct ListPage<'a> {
    pub kind: ListKind,
    pub columns: &'a [String],
    pub rows: &'a [Vec<Value>],
    /// `{ username: path from the page }` of the avatars written beside the page; the rows of the others show their
    /// name's first character.
    pub avatars: &'a HashMap<String, String>,
    /// When the list was exported, in unix seconds.
    pub exported_at: i64,
}

impl ListPage<'_> {
    /// Writes the page, in the language of the program.
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let lang = locale::current();
        let (template, lang_name, page_lang) = match lang {
            locale::Lang::Zh => (LIST_ZH, "zh", "zh-CN"),
            locale::Lang::En => (LIST_EN, "en", "en"),
        };
        let (head, tail) = template
            .split_once("__WF_DATA__")
            .ok_or_else(|| io::Error::other("the list template has no data placeholder"))?;
        w.write_all(
            head.replacen("__WF_LANG__", page_lang, 1)
                .replacen("__WF_TITLE__", &html_escape(self.kind.title(lang)), 1)
                .as_bytes(),
        )?;
        // sorted, so the same list writes the same page
        let avatars: BTreeMap<&str, String> = self
            .avatars
            .iter()
            .map(|(user, path)| (user.as_str(), url_path(path)))
            .collect();
        write!(
            w,
            "{{\"v\":1,\"lang\":\"{lang_name}\",\"kind\":\"{}\",\"exported\":{},\"columns\":",
            self.kind.name(),
            self.exported_at
        )?;
        write_script_json(w, &self.columns)?;
        w.write_all(b",\"rows\":")?;
        write_script_json(w, &self.rows)?;
        w.write_all(b",\"avatars\":")?;
        write_script_json(w, &avatars)?;
        w.write_all(b"}")?;
        w.write_all(tail.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The page's `#wf-data`, read back.
    fn data(page: &str) -> Value {
        let start = page
            .find(r#"<script id="wf-data" type="application/json">"#)
            .unwrap();
        let rest = &page[start..];
        let body = &rest[rest.find('>').unwrap() + 1..rest.find("</script>").unwrap()];
        serde_json::from_str(body).unwrap()
    }

    #[test]
    fn a_list_page_carries_the_table_and_the_avatars() {
        let columns = vec!["username".to_string(), "summary".to_string()];
        let rows = vec![
            vec![
                json!("wxid_bob"),
                json!("</script><script>alert(1)</script>"),
            ],
            vec![json!("room@chatroom"), Value::Null],
        ];
        let avatars = HashMap::from([(
            "wxid_bob".to_string(),
            "media/a b/avatars/1.jpg".to_string(),
        )]);
        let page = ListPage {
            kind: ListKind::Contacts,
            columns: &columns,
            rows: &rows,
            avatars: &avatars,
            exported_at: 1_700_000_000,
        };
        let mut out = Vec::new();
        page.write(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // a text never ends the data's script
        assert_eq!(out.matches("</script>").count(), 2, "{out}");
        assert!(!out.contains("__WF_"), "a placeholder is left");
        let d = data(&out);
        assert_eq!(d["kind"], "contacts");
        assert_eq!(d["exported"], 1_700_000_000);
        assert_eq!(d["columns"], json!(["username", "summary"]));
        assert_eq!(d["rows"].as_array().unwrap().len(), 2);
        assert_eq!(d["rows"][0][1], "</script><script>alert(1)</script>");
        assert_eq!(d["rows"][1][1], Value::Null);
        assert_eq!(d["avatars"]["wxid_bob"], "media/a%20b/avatars/1.jpg");
    }

    #[test]
    fn a_list_page_is_titled_by_its_kind() {
        let mut out = Vec::new();
        let page = ListPage {
            kind: ListKind::Sessions,
            columns: &[],
            rows: &[],
            avatars: &HashMap::new(),
            exported_at: 0,
        };
        page.write(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        let title = ListKind::Sessions.title(locale::current());
        assert!(out.contains(&format!("<title>{title}</title>")), "{out}");
        assert_eq!(data(&out)["rows"], json!([]));
    }
}
