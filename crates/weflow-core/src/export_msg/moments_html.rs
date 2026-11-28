//! 朋友圈的 HTML 页面（`sns export --format html`）：build.mjs 由 `moments.html` 生成的 `moments.{zh,en}.html`，
//! 动态以 JSON 写在其中的 `#wf-data` 里：
//!
//! `{v, lang, title, exported, keyword, users, posts}`：`exported` 是导出的 Unix 时间（秒）；
//! `keyword` 是筛选动态用的关键词（没有时省略）；`users` 是发动态的人，每项是 `[名字, 头像路径]`（路径相对页面，
//! 没有保存头像时为 `""`）；`posts` 从新到旧排列，每条是 `{t, u, text, loc, media, likes, cmts}`：
//! Unix 时间（秒）、发布者在 `users` 中的下标、文字、位置、图片和视频、点赞者的名字、评论。
//! 图片和视频的每项是 `{k, s, url, live}`：`k` 为 `"img"` 或 `"vid"`，`s` 是副本的路径（没有保存副本时为 `""`），
//! `url` 是微信的地址，`live` 是实况照片的视频；评论的每项是 `[名字, 文字]` 或 `[名字, 文字, 被回复者的名字]`。
//! 没有内容的字段省略。

use std::collections::HashMap;
use std::io::{self, Write};

use serde::ser::{SerializeSeq, Serializer};
use serde::Serialize;

use super::format::{html_escape, write_script_json};
use super::html::url_path;
use crate::locale;

const MOMENTS_ZH: &str = include_str!("moments.zh.html");
const MOMENTS_EN: &str = include_str!("moments.en.html");

/// 页面绘制的一条动态。
#[derive(Debug, Default, Serialize)]
pub struct MomentPost {
    /// 发布的时间，Unix 时间（秒）。
    pub t: i64,
    /// 发布者：在 [`MomentsPage`] 的发布者列表中的下标（见 [`MomentsPage::user`]）。
    pub u: usize,
    /// 动态的文字。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// 发布的位置，写成一行。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub loc: String,
    /// 动态的图片或视频。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<MomentMedia>,
    /// 点赞者的名字。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub likes: Vec<String>,
    /// 评论，按顺序排列。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cmts: Vec<MomentComment>,
}

/// 动态中一项媒体的种类。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MomentMediaKind {
    /// 图片，页面上为 `img`。
    #[serde(rename = "img")]
    Picture,
    /// 视频，页面上为 `vid`。
    #[serde(rename = "vid")]
    Video,
}

/// 动态中的一张图片或一个视频。
#[derive(Debug, Serialize)]
pub struct MomentMedia {
    k: MomentMediaKind,
    /// 副本相对页面的路径；没有保存副本时为空，页面改为提供 `url`。
    s: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    live: Option<String>,
}

impl MomentMedia {
    /// 一项媒体；`saved` 和 `live` 是导出写出的副本相对页面的路径。
    pub fn new(kind: MomentMediaKind, saved: Option<&str>, url: &str, live: Option<&str>) -> Self {
        Self {
            k: kind,
            s: saved
                .filter(|s| !s.is_empty())
                .map(url_path)
                .unwrap_or_default(),
            url: url.to_string(),
            live: live.filter(|s| !s.is_empty()).map(url_path),
        }
    }
}

/// 一条评论：谁写的、写了什么、回复谁；在页面上写成 `[名字, 文字]` 或 `[名字, 文字, 被回复者的名字]`。
#[derive(Debug)]
pub struct MomentComment {
    /// 评论者的名字。
    pub name: String,
    /// 评论的文字；只有表情的评论为空，页面显示为 `[表情]`。
    pub text: String,
    /// 被回复者的名字；没有回复谁时为 `None` 或空字符串。
    pub to: Option<String>,
}

impl Serialize for MomentComment {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let to = self.to.as_deref().filter(|t| !t.is_empty());
        let mut seq = serializer.serialize_seq(Some(if to.is_some() { 3 } else { 2 }))?;
        seq.serialize_element(&self.name)?;
        seq.serialize_element(&self.text)?;
        if let Some(to) = to {
            seq.serialize_element(to)?;
        }
        seq.end()
    }
}

/// 要写成页面的朋友圈导出中的动态。
#[derive(Debug, Default)]
pub struct MomentsPage {
    /// 筛选动态用的关键词。
    pub keyword: Option<String>,
    /// 导出动态的时间，Unix 时间（秒）。
    pub exported_at: i64,
    /// 各条动态，按页面列出的顺序排列。
    pub posts: Vec<MomentPost>,
    users: Vec<(String, String)>,
    by_username: HashMap<String, usize>,
}

impl MomentsPage {
    /// 还没有动态的页面：动态按 `keyword` 筛选，导出时间为 `exported_at`（Unix 时间，秒）。
    pub fn new(keyword: Option<String>, exported_at: i64) -> Self {
        Self {
            keyword,
            exported_at,
            ..Self::default()
        }
    }

    /// 发布者 `username` 的下标；第一次出现时以名字 `name` 和头像路径（导出写出的、相对页面的路径）加入。
    pub fn user(&mut self, username: &str, name: &str, avatar: Option<&str>) -> usize {
        if let Some(&at) = self.by_username.get(username) {
            return at;
        }
        let at = self.users.len();
        let avatar = avatar
            .filter(|a| !a.is_empty())
            .map(url_path)
            .unwrap_or_default();
        self.users.push((name.to_string(), avatar));
        self.by_username.insert(username.to_string(), at);
        at
    }

    /// 按程序的语言写出页面。
    ///
    /// # Errors
    ///
    /// 写入 `w` 失败时返回错误。
    pub fn write(&self, w: &mut impl Write) -> io::Result<()> {
        let lang = locale::current();
        let (template, lang_name, page_lang, title) = match lang {
            locale::Lang::Zh => (MOMENTS_ZH, "zh", "zh-CN", "朋友圈"),
            locale::Lang::En => (MOMENTS_EN, "en", "en", "Moments"),
        };
        let (head, tail) = template
            .split_once("__WF_DATA__")
            .ok_or_else(|| io::Error::other("the Moments template has no data placeholder"))?;
        w.write_all(
            head.replacen("__WF_LANG__", page_lang, 1)
                .replacen("__WF_TITLE__", &html_escape(title), 1)
                .as_bytes(),
        )?;
        write!(
            w,
            "{{\"v\":1,\"lang\":\"{lang_name}\",\"title\":\"{title}\",\"exported\":{}",
            self.exported_at
        )?;
        if let Some(keyword) = self.keyword.as_deref().filter(|k| !k.is_empty()) {
            w.write_all(b",\"keyword\":")?;
            write_script_json(w, &keyword)?;
        }
        w.write_all(b",\"users\":")?;
        write_script_json(w, &self.users)?;
        w.write_all(b",\"posts\":")?;
        write_script_json(w, &self.posts)?;
        w.write_all(b"}")?;
        w.write_all(tail.as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{json, Value};

    /// 读回页面的 `#wf-data`。
    fn data(page: &str) -> Value {
        let open = r#"<script id="wf-data" type="application/json">"#;
        let start = page.find(open).unwrap() + open.len();
        let end = start + page[start..].find("</script>").unwrap();
        serde_json::from_str(&page[start..end]).unwrap()
    }

    #[test]
    fn a_moments_page_carries_the_posts() {
        let mut page = MomentsPage {
            keyword: Some("trip".into()),
            exported_at: 1_700_000_100,
            ..Default::default()
        };
        let bob = page.user("wxid_bob", "</script>Bob", Some("media/avatar 1.jpg"));
        assert_eq!(page.user("wxid_bob", "Bob again", None), bob);
        let amy = page.user("wxid_amy", "Amy", None);
        page.posts.push(MomentPost {
            t: 1_700_000_000,
            u: bob,
            text: "a\nb".into(),
            loc: "West Lake".into(),
            media: vec![
                MomentMedia::new(
                    MomentMediaKind::Picture,
                    Some("media/1.jpg"),
                    "https://x/1",
                    Some("media/1.mp4"),
                ),
                MomentMedia::new(MomentMediaKind::Video, None, "https://x/2", None),
            ],
            likes: vec!["Amy".into()],
            cmts: vec![
                MomentComment {
                    name: "Amy".into(),
                    text: "nice".into(),
                    to: None,
                },
                MomentComment {
                    name: "Bob".into(),
                    text: String::new(),
                    to: Some("Amy".into()),
                },
            ],
        });
        page.posts.push(MomentPost {
            t: 1_699_000_000,
            u: amy,
            ..Default::default()
        });
        let mut out = Vec::new();
        page.write(&mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // 名字不会提前结束数据所在的 script 元素
        assert_eq!(out.matches("</script>").count(), 2, "{out}");
        assert!(!out.contains("__WF_"), "a placeholder is left");
        let d = data(&out);
        assert_eq!(d["keyword"], "trip");
        assert_eq!(d["exported"], 1_700_000_100);
        assert_eq!(
            d["users"],
            json!([["</script>Bob", "media/avatar%201.jpg"], ["Amy", ""]])
        );
        let p = &d["posts"][0];
        assert_eq!(
            (p["t"].clone(), p["u"].clone()),
            (json!(1_700_000_000), json!(0))
        );
        assert_eq!(p["text"], "a\nb");
        assert_eq!(
            p["media"][0],
            json!({ "k": "img", "s": "media/1.jpg", "url": "https://x/1", "live": "media/1.mp4" })
        );
        assert_eq!(
            p["media"][1],
            json!({ "k": "vid", "s": "", "url": "https://x/2" })
        );
        assert_eq!(p["cmts"], json!([["Amy", "nice"], ["Bob", "", "Amy"]]));
        // 动态中没有内容的字段省略
        assert_eq!(d["posts"][1], json!({ "t": 1_699_000_000, "u": 1 }));
    }
}
