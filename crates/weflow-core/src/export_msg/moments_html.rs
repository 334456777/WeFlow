//! The HTML page of Moments (`sns export --format html`): `moments.{zh,en}.html`, which build.mjs makes from
//! `moments.html`, with the posts as JSON in its `#wf-data`:
//!
//! `{v, lang, title, exported, keyword, users, posts}`: `exported` the time of the export in unix seconds; `keyword`
//! the words the posts were picked by (left out when none); `users` the posters as `[name, avatar path]` (the path
//! from the page, `""` when no avatar was saved); `posts` newest first, each `{t, u, text, loc, media, likes, cmts}`:
//! the time in unix seconds, the poster's index in `users`, the text, the place, the pictures and videos as `{k:
//! "img" | "vid", s, url, live}` (`s` the path of the copy, `""` when none was saved; `url` WeChat's address; `live` a
//! live photo's video), the names of those who liked it, and the comments as `[name, text]` or `[name, text, name
//! answered]`. A field a post has nothing in is left out.

use std::collections::HashMap;
use std::io::{self, Write};

use serde::ser::{SerializeSeq, Serializer};
use serde::Serialize;

use super::html::url_path;
use super::{html_escape, write_script_json};
use crate::locale;

const MOMENTS_ZH: &str = include_str!("moments.zh.html");
const MOMENTS_EN: &str = include_str!("moments.en.html");

/// A post as the page draws it.
#[derive(Debug, Default, Serialize)]
pub struct MomentPost {
    /// When it was posted, in unix seconds.
    pub t: i64,
    /// The poster: an index into [`MomentsPage`]'s users (see [`MomentsPage::user`]).
    pub u: usize,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// Where it was posted, as one line.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub loc: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub media: Vec<MomentMedia>,
    /// The names of those who liked it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub likes: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cmts: Vec<MomentComment>,
}

/// What a medium of a post is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum MomentMediaKind {
    #[serde(rename = "img")]
    Picture,
    #[serde(rename = "vid")]
    Video,
}

/// A picture or video of a post.
#[derive(Debug, Serialize)]
pub struct MomentMedia {
    k: MomentMediaKind,
    /// The copy's path from the page; empty when none was saved, and the page offers `url` instead.
    s: String,
    url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    live: Option<String>,
}

impl MomentMedia {
    /// `saved` and `live` are the paths of the copies from the page, as the export wrote them.
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

/// A comment: who wrote it, what, and whom it answers; on the page `[name, text]` or `[name, text, name answered]`.
#[derive(Debug)]
pub struct MomentComment {
    pub name: String,
    /// Empty for a comment of only a sticker, which the page shows as `[表情]`.
    pub text: String,
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

/// The posts of a Moments export, to write as a page.
#[derive(Debug, Default)]
pub struct MomentsPage {
    /// The words the posts were picked by.
    pub keyword: Option<String>,
    /// When the posts were exported, in unix seconds.
    pub exported_at: i64,
    pub posts: Vec<MomentPost>,
    users: Vec<(String, String)>,
    by_username: HashMap<String, usize>,
}

impl MomentsPage {
    /// A page without posts yet: those picked by `keyword`, exported at `exported_at` (unix seconds).
    pub fn new(keyword: Option<String>, exported_at: i64) -> Self {
        Self {
            keyword,
            exported_at,
            ..Self::default()
        }
    }

    /// The index of the poster `username`, added as `name` with its avatar's path (from the page, as the export wrote
    /// it) the first time.
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

    /// Writes the page, in the language of the program.
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

    /// The page's `#wf-data`, read back.
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
                    text: "".into(),
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
        // a name never ends the data's script
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
        // what a post has nothing in is left out
        assert_eq!(d["posts"][1], json!({ "t": 1_699_000_000, "u": 1 }));
    }
}
