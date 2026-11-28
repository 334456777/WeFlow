//! 列表的 HTML 页面（带 `--format html` 的 `export sessions`、`export contacts`）：build.mjs 由 `list.html` 生成的
//! `list.{zh,en}.html`，列表以 JSON 写在其中的 `#wf-data` 里：
//!
//! `{v, lang, kind, exported, columns, rows, avatars}`：`kind` 为 `"sessions"` 或 `"contacts"`；`exported` 是导出的
//! Unix 时间（秒）；`columns` 是列名，`rows` 是各行按这个顺序给出的值（即其他格式写出的表格，见
//! [`crate::export::list_table`]）；`avatars` 按用户名给出写在页面旁边的每个头像相对页面的路径。

use std::collections::{BTreeMap, HashMap};
use std::io::{self, Write};

use serde_json::Value;

use super::format::{html_escape, write_script_json};
use super::html::url_path;
use crate::locale;

const LIST_ZH: &str = include_str!("list.zh.html");
const LIST_EN: &str = include_str!("list.en.html");

/// 列表页面显示什么：决定各行的画法和页面的标题。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKind {
    /// 会话（`export sessions`）。
    Sessions,
    /// 联系人（`export contacts`）。
    Contacts,
}

impl ListKind {
    /// 页面用来识别它的名字，也是导出文件的文件名。
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

/// 要写成页面的列表。
#[derive(Debug)]
pub struct ListPage<'a> {
    /// 哪一种列表。
    pub kind: ListKind,
    /// 列名，与导出的其他格式中的列名相同。
    pub columns: &'a [String],
    /// 各行，每列一个值。
    pub rows: &'a [Vec<Value>],
    /// 写在页面旁边的头像：`{ 用户名: 相对页面的路径 }`；没有头像的行显示名字的第一个字。
    pub avatars: &'a HashMap<String, String>,
    /// 导出列表的时间，Unix 时间（秒）。
    pub exported_at: i64,
}

impl ListPage<'_> {
    /// 按程序的语言写出页面。
    ///
    /// # Errors
    ///
    /// 写入 `w` 失败时返回错误。
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
        // 排好序，同一个列表总是写出相同的页面
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

    /// 读回页面的 `#wf-data`。
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
        // 文字不会提前结束数据所在的 script 元素
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
