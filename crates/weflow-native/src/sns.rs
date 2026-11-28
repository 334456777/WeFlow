//! 朋友圈（`sns.db`）：从 `TimelineObject` XML 解析出的动态，以及按用户和按年的统计。
//!
//! `SnsTimeLine(tid, user_name, content, pack_info_buf)` 每行是一条动态的 XML 文档。
//! `tid` 是无符号 64 位的雪花 ID（snowflake），按有符号整数存储，所以输出时一律写成 `tid as u64`。

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde_json::{json, Map, Value};

use crate::db::{same_identity, Account};
use crate::entities::decode_entities;

#[expect(clippy::expect_used, reason = "调用方只传入固定的正则")]
fn re(p: &str) -> Regex {
    Regex::new(p).expect("static regex")
}

static R_ID: LazyLock<Regex> = LazyLock::new(|| re(r"<TimelineObject>\s*<id>(\d+)</id>"));
static R_CREATE: LazyLock<Regex> = LazyLock::new(|| re(r"<createTime>(\d+)</createTime>"));
static R_TYPE: LazyLock<Regex> = LazyLock::new(|| re(r"<ContentObject>\s*<type>(\d+)</type>"));
static R_MEDIA: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<media>(.*?)</media>"));
static R_LIKES: LazyLock<Regex> = LazyLock::new(|| {
    re(
        r"(?is)<(like_user_list|LikeUserList|likeUserList|likeList)>(.*?)</(?:like_user_list|LikeUserList|likeUserList|likeList)>",
    )
});
static R_LIKE_ITEM: LazyLock<Regex> = LazyLock::new(|| {
    re(r"(?is)<(?:user_comment|LikeUser|likeUser)>(.*?)</(?:user_comment|LikeUser|likeUser)>")
});
static R_LIVE: LazyLock<Regex> = LazyLock::new(|| re(r"(?s)<livePhoto>(.*?)</livePhoto>"));
static R_ENC_KEY: LazyLock<Regex> = LazyLock::new(|| re(r#"(?i)<enc\s+key="(\d+)""#));

/// 解码微信用的 XML 实体（并去掉 CDATA 的包裹）。
fn decode(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("<![CDATA[")
        .and_then(|t| t.strip_suffix("]]>"))
        .unwrap_or(s);
    decode_entities(s).trim().to_string()
}

/// 每个开始标签 `<name>` 或 `<name ...>`：（属性的起点，属性的终点，标签之后的位置）。
/// 用普通的字符串查找读取：显示的每条动态的每个字段都要解析，每次调用都建一个正则时每条动态要多花一毫秒。
fn open_tags<'a>(xml: &'a str, name: &'a str) -> impl Iterator<Item = (usize, usize, usize)> + 'a {
    let open = format!("<{name}");
    xml.match_indices(open.as_str())
        .map(|(at, _)| at + open.len())
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|from| {
            let rest = &xml[from..];
            if rest.starts_with('>') {
                return Some((from, from, from + 1));
            }
            if !rest.starts_with(char::is_whitespace) {
                return None;
            }
            let end = from + rest.find('>')?;
            Some((from, end, end + 1))
        })
}

/// 第一个 `<name ...>text</name>` 中的文字（已解码）。
fn tag_text(xml: &str, name: &str) -> Option<String> {
    let close = format!("</{name}>");
    open_tags(xml, name)
        .find_map(|(_, _, inner)| xml[inner..].find(&close).map(|n| &xml[inner..inner + n]))
        .map(decode)
}

/// 第一个开始标签 `<name ...>` 的属性字符串。
fn tag_attrs(xml: &str, name: &str) -> Option<String> {
    let open = format!("<{name}");
    xml.match_indices(open.as_str()).find_map(|(at, _)| {
        let from = at + open.len();
        let rest = &xml[from..];
        if rest.starts_with('>') || rest.starts_with("/>") {
            return Some(String::new());
        }
        if !rest.starts_with(char::is_whitespace) {
            return None;
        }
        rest.find('>').map(|end| rest[..end].to_string())
    })
}

/// 属性字符串中 `name="value"`（不区分大小写）的值，已解码；为空时为 `None`。
fn attr(attrs: &str, name: &str) -> Option<String> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    // 只转换 ASCII 的大小写，每个字节的位置不变
    let lower = attrs.to_ascii_lowercase();
    let wanted = name.to_ascii_lowercase();
    lower
        .match_indices(wanted.as_str())
        .find_map(|(at, _)| {
            let before = attrs[..at].chars().next_back();
            let first = name.chars().next()?;
            // 名称前的 `\b`：单词字符与其他字符之间的边界
            if before.is_some_and(word) == word(first) {
                return None;
            }
            let rest = attrs[at + name.len()..].trim_start();
            let rest = rest.strip_prefix('=')?.trim_start();
            let value = rest.strip_prefix('"')?;
            let end = value.find('"')?;
            Some(decode(&value[..end]))
        })
        .filter(|v| !v.is_empty())
}

/// 动态中的一张图片或一段视频（`<media>`），或者实况照片会动的部分（`<livePhoto>`），各字段是 XML 中的原值。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnsMedia {
    /// `<url>` 的文字：图片或视频的地址。
    pub url: String,
    /// `<thumb>` 的文字：缩略图的地址。
    pub thumb: String,
    /// `token` 属性。`token`、`key`、`enc_idx` 先取 `<url>` 的，没有时取 `<thumb>` 的；`md5` 只取 `<url>` 的。
    pub token: Option<String>,
    /// `key` 属性：解密文件用的密钥。
    pub key: Option<String>,
    /// `<url>` 的 `md5` 属性。
    pub md5: Option<String>,
    /// `enc_idx` 属性。
    pub enc_idx: Option<String>,
    /// 实况照片的 `<livePhoto>`。
    pub live_photo: Option<Box<SnsMedia>>,
}

impl SnsMedia {
    /// 从一段 XML 片段读出的媒体。
    fn of(frag: &str) -> Self {
        let (url_attrs, thumb_attrs) = (
            tag_attrs(frag, "url").unwrap_or_default(),
            tag_attrs(frag, "thumb").unwrap_or_default(),
        );
        let pick = |name: &str| attr(&url_attrs, name).or_else(|| attr(&thumb_attrs, name));
        Self {
            url: tag_text(frag, "url").unwrap_or_default(),
            thumb: tag_text(frag, "thumb").unwrap_or_default(),
            token: pick("token"),
            key: pick("key"),
            md5: attr(&url_attrs, "md5"),
            enc_idx: pick("enc_idx"),
            live_photo: None,
        }
    }
}

/// 一条朋友圈动态，从它在 `SnsTimeLine` 中的行读出。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SnsPost {
    /// 微信给动态的 ID；XML 里没有时取 `tid`。
    pub id: String,
    /// 行的 ID：十进制的无符号雪花 ID。
    pub tid: String,
    /// 行的 `user_name`；为空时取 XML 的 `<username>`。
    pub username: String,
    /// `<createTime>`；没有时为 0。
    pub create_time: i64,
    /// `<contentDesc>`：动态的文字。
    pub content_desc: String,
    /// `<ContentObject><type>`；没有时为 1。
    pub kind: i64,
    /// 行里保存的动态 XML 原文。
    pub raw_xml: String,
    /// `<mediaList>` 中的各个 `<media>`，按原顺序。
    pub media: Vec<SnsMedia>,
    /// 点赞者的昵称。
    pub likes: Vec<String>,
    /// 解密动态视频的 `<enc key>`。
    pub video_key: Option<String>,
}

/// 动态的 `<createTime>`；没有时为 `0`。
fn post_time(xml: &str) -> i64 {
    R_CREATE
        .captures(xml)
        .and_then(|c| c[1].parse::<i64>().ok())
        .unwrap_or(0)
}

/// 动态的 `<ContentObject><type>`；没有时为 `1`。
fn post_type(xml: &str) -> i64 {
    R_TYPE
        .captures(xml)
        .and_then(|c| c[1].parse::<i64>().ok())
        .unwrap_or(1)
}

/// 把 `SnsTimeLine` 的一行解析成动态。
pub fn parse_post(tid: i64, user_name: &str, xml: &str) -> SnsPost {
    let tid_text = tid.cast_unsigned().to_string();
    let id = R_ID
        .captures(xml)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| tid_text.clone());
    let username = if user_name.is_empty() {
        tag_text(xml, "username").unwrap_or_default()
    } else {
        user_name.to_string()
    };

    let media: Vec<SnsMedia> = R_MEDIA
        .captures_iter(xml)
        .map(|c| {
            let frag = &c[1];
            let mut item = SnsMedia::of(frag);
            if let Some(live) = R_LIVE.captures(frag) {
                item.live_photo = Some(Box::new(SnsMedia::of(&live[1])));
            }
            item
        })
        .collect();
    let likes: Vec<String> = R_LIKES
        .captures(xml)
        .map(|l| {
            R_LIKE_ITEM
                .captures_iter(&l[2])
                .filter_map(|i| tag_text(&i[1], "nickname"))
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default();

    SnsPost {
        id,
        tid: tid_text,
        username,
        create_time: post_time(xml),
        content_desc: tag_text(xml, "contentDesc").unwrap_or_default(),
        kind: post_type(xml),
        raw_xml: xml.to_string(),
        media,
        likes,
        video_key: R_ENC_KEY.captures(xml).map(|key| key[1].to_string()),
    }
}

/// 点赞者的用户名（取自 `<like_user_list>`）。
fn liker_usernames(xml: &str) -> Vec<String> {
    R_LIKES
        .captures(xml)
        .map(|l| {
            R_LIKE_ITEM
                .captures_iter(&l[2])
                .filter_map(|i| tag_text(&i[1], "username"))
                .filter(|n| !n.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

fn top(counts: HashMap<String, i64>, n: usize) -> Value {
    let mut v: Vec<(String, i64)> = counts.into_iter().collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Value::Array(
        v.into_iter()
            .take(n)
            .map(|(u, c)| json!({ "username": u, "count": c }))
            .collect(),
    )
}

impl Account {
    fn sns_db(&self) -> std::path::PathBuf {
        self.db_path("sns/sns.db")
    }

    /// 每条动态的 `(tid, username, xml)`（`usernames` 不为空时只取这些用户的），开销小：每条动态只读一行。
    fn sns_rows(&self, usernames: &[String]) -> Result<Vec<(i64, String, String)>> {
        let rows = if usernames.is_empty() {
            self.query(
                &self.sns_db(),
                "select tid, user_name, content from SnsTimeLine",
                &[],
            )?
        } else {
            let sql = format!(
                "select tid, user_name, content from SnsTimeLine where user_name in ({})",
                vec!["?"; usernames.len()].join(",")
            );
            let params: Vec<&dyn rusqlite::ToSql> = usernames
                .iter()
                .map(|u| u as &dyn rusqlite::ToSql)
                .collect();
            self.query(&self.sns_db(), &sql, &params)?
        };
        Ok(rows
            .into_iter()
            .map(|r| {
                (
                    r["tid"].as_i64().unwrap_or(0),
                    r["user_name"].as_str().unwrap_or("").to_string(),
                    r["content"].as_str().unwrap_or("").to_string(),
                )
            })
            .collect())
    }

    /// 最新在前的一页动态：跳过 `offset` 条，最多取 `limit` 条（不大于 0 时取全部）。
    /// `usernames` 不为空时只取这些用户的动态，`keyword` 匹配动态的文字（不区分大小写），
    /// `start`、`end` 为秒（`0` 表示不限）。
    ///
    /// # Errors
    ///
    /// 无法读取 `sns.db`（打开、解密或查询失败）。
    pub fn sns_timeline(
        &self,
        limit: i32,
        offset: i32,
        usernames: &[String],
        keyword: Option<&str>,
        start: i64,
        end: i64,
    ) -> Result<Vec<SnsPost>> {
        let needle = keyword
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty());
        // 每条动态只取时间和文字，用来筛选和排序；只有这一页的动态才完整解析
        let mut posts: Vec<(i64, u64, String, String)> = self
            .sns_rows(usernames)?
            .into_iter()
            .map(|(tid, user, xml)| (post_time(&xml), tid.cast_unsigned(), user, xml))
            .filter(|(t, _, _, xml)| {
                (start <= 0 || *t >= start)
                    && (end <= 0 || *t <= end)
                    && needle.as_ref().is_none_or(|n| {
                        tag_text(xml, "contentDesc")
                            .unwrap_or_default()
                            .to_lowercase()
                            .contains(n.as_str())
                    })
            })
            .collect();
        posts.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let it = posts
            .into_iter()
            .skip(usize::try_from(offset).unwrap_or(0))
            .map(|(_, tid, user, xml)| parse_post(tid.cast_signed(), &user, &xml));
        Ok(if limit > 0 {
            it.take(usize::try_from(limit).unwrap_or(usize::MAX))
                .collect()
        } else {
            it.collect()
        })
    }

    /// 发过动态的用户名（不重复），最近发过的在前。
    ///
    /// # Errors
    ///
    /// 无法读取 `sns.db`（打开、解密或查询失败）。
    pub fn sns_usernames(&self) -> Result<Value> {
        let mut last: BTreeMap<String, i64> = BTreeMap::new();
        for (tid, user, xml) in self.sns_rows(&[])? {
            if user.is_empty() {
                continue;
            }
            let t = post_time(&xml);
            let e = last.entry(user).or_insert(0);
            *e = (*e).max(t.max(tid.clamp(0, 1)));
        }
        let mut v: Vec<(String, i64)> = last.into_iter().collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        Ok(Value::Array(v.into_iter().map(|(u, _)| json!(u)).collect()))
    }

    /// `{ username: posts }`：每条动态的发布者是它的 `user_name`，为空时取 XML 的 `<username>`，与 [`parse_post`]
    /// 一致。直接从表中计数，不解析动态。
    ///
    /// # Errors
    ///
    /// 无法读取 `sns.db`（打开、解密或查询失败）。
    pub fn sns_post_counts(&self) -> Result<Value> {
        let mut counts: BTreeMap<String, i64> = BTreeMap::new();
        for (_, user, xml) in self.sns_rows(&[])? {
            let user = if user.is_empty() {
                tag_text(&xml, "username").unwrap_or_default()
            } else {
                user
            };
            let user = user.trim();
            if !user.is_empty() {
                *counts.entry(user.to_string()).or_default() += 1;
            }
        }
        Ok(json!(counts))
    }

    /// `{ total_posts, total_friends, my_posts }`（只有给出 `my_wxid` 时才有 `my_posts`）。
    ///
    /// # Errors
    ///
    /// 无法读取 `sns.db`（打开、解密或查询失败）。
    pub fn sns_export_stats(&self, my_wxid: Option<&str>) -> Result<Value> {
        let rows = self.sns_rows(&[])?;
        let users: BTreeSet<&str> = rows
            .iter()
            .map(|r| r.1.as_str())
            .filter(|u| !u.is_empty())
            .collect();
        let mut o = json!({ "total_posts": rows.len(), "total_friends": users.len() });
        if let Some(me) = my_wxid.filter(|m| !m.trim().is_empty()) {
            o["my_posts"] = json!(rows.iter().filter(|r| same_identity(&r.1, me)).count());
        }
        Ok(o)
    }

    /// 账号所有者的年度朋友圈统计：自己的动态数和各类型的数量、给自己点赞最多的好友，
    /// 以及自己点赞最多的好友。`begin`、`end` 为秒，0 表示不限。
    ///
    /// # Errors
    ///
    /// 无法读取 `sns.db`（打开、解密或查询失败）。
    pub fn sns_annual_stats(&self, begin: i64, end: i64) -> Result<Value> {
        let mut total = 0i64;
        let mut types: BTreeMap<i64, i64> = BTreeMap::new();
        let (mut likers, mut liked): (HashMap<String, i64>, HashMap<String, i64>) =
            Default::default();
        for (_, user, xml) in self.sns_rows(&[])? {
            let t = post_time(&xml);
            if (begin > 0 && t < begin) || (end > 0 && t > end) {
                continue;
            }
            let mine = self.is_me(&user);
            let likes = liker_usernames(&xml);
            if mine {
                total += 1;
                *types.entry(post_type(&xml)).or_default() += 1;
                for l in likes.iter().filter(|l| !self.is_me(l)) {
                    *likers.entry(l.clone()).or_default() += 1;
                }
            } else if !user.is_empty() && likes.iter().any(|l| self.is_me(l)) {
                *liked.entry(user).or_default() += 1;
            }
        }
        Ok(json!({
            "totalPosts": total,
            "typeCounts": types.into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>(),
            "topLikers": top(likers, 10), "topLiked": top(liked, 10)
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const XML: &str = "<SnsDataItem><TimelineObject><id>14000000000000000001</id><username>wxid_bob</username><createTime>1700000500</createTime>\
        <contentDesc>hello &amp; &lt;moments&gt;</contentDesc><location city=\"Shanghai\" latitude=\"31.2\"/>\
        <ContentObject><type>1</type><mediaList>\
        <media><id>1</id><thumb type=\"1\" key=\"9\" enc_idx=\"1\" token=\"TT\">http://t/1?a=1&amp;b=2</thumb><url type=\"1\" md5=\"abc\" key=\"7\" enc_idx=\"2\" token=\"TU\">http://u/1</url>\
        <livePhoto><url key=\"5\" token=\"LT\">http://l/1</url><thumb>http://l/t</thumb></livePhoto></media>\
        <media><id>2</id><url>http://u/2</url><thumb>http://t/2</thumb></media></mediaList></ContentObject>\
        <enc key=\"2105122989\"/>\
        <like_user_list><user_comment><username>wxid_carol</username><nickname>Carol</nickname></user_comment>\
        <user_comment><username>wxid_me</username><nickname>Me</nickname></user_comment></like_user_list></TimelineObject></SnsDataItem>";

    #[test]
    fn parses_text_type_media_live_photo_likes_and_video_key() {
        let p = parse_post(-4_000_000_000_000_000_000, "wxid_bob", XML);
        assert_eq!(p.id, "14000000000000000001");
        assert_eq!(
            p.tid, "14446744073709551616",
            "a negative stored tid is the unsigned snowflake"
        );
        assert_eq!(
            (p.username.as_str(), p.create_time, p.kind),
            ("wxid_bob", 1_700_000_500, 1)
        );
        assert_eq!(p.content_desc, "hello & <moments>");
        let media = &p.media;
        assert_eq!(media.len(), 2);
        assert_eq!(media[0].url, "http://u/1");
        assert_eq!(
            media[0].thumb, "http://t/1?a=1&b=2",
            "entities in URLs are decoded"
        );
        assert_eq!(
            (
                media[0].token.as_deref(),
                media[0].key.as_deref(),
                media[0].md5.as_deref(),
                media[0].enc_idx.as_deref()
            ),
            (Some("TU"), Some("7"), Some("abc"), Some("2"))
        );
        let live = media[0].live_photo.as_deref().unwrap();
        assert_eq!(live.url, "http://l/1");
        assert_eq!(
            (live.token.as_deref(), live.key.as_deref()),
            (Some("LT"), Some("5"))
        );
        assert!(media[1].token.is_none() && media[1].live_photo.is_none());
        assert_eq!(p.likes, ["Carol", "Me"]);
        assert_eq!(p.video_key.as_deref(), Some("2105122989"));
        assert_eq!(p.raw_xml, XML);
        assert_eq!(liker_usernames(XML), ["wxid_carol", "wxid_me"]);
    }

    #[test]
    fn text_and_nicknames_decode_each_entity_once() {
        assert_eq!(
            decode(" <![CDATA[it&#39;s &amp;lt; &AMP;]]> "),
            "it's &lt; &"
        );
        let p = parse_post(
            1,
            "wxid_bob",
            "<TimelineObject><contentDesc>it&#39;s &amp;amp;</contentDesc>\
             <like_user_list><user_comment><username>wxid_a</username><nickname>A&#39;&amp;B</nickname></user_comment></like_user_list></TimelineObject>",
        );
        assert_eq!(p.content_desc, "it's &amp;");
        assert_eq!(p.likes, ["A'&B"]);
    }

    #[test]
    fn tags_and_attributes_are_read_as_the_regexes_read_them() {
        let samples = [
            XML,
            "<url>a</url><url>b</url>",
            "<urls>no</urls><url >yes</url>",
            "<url/><url>after empty</url>",
            "<url key=\"1\"/>",
            "<url\tkey=\"1\">tab</url>",
            "<url>open only",
            "<url key=\"no close\"",
            "<urlx>x</urlx><url\nmd5=\"m\">新</url>",
            "<thumb><url>nested</url></thumb>",
            "<url/x><url>y</url>",
            "",
        ];
        let attr_samples = [
            r#" key="1" token="T""#,
            r#" xkey="1" key="2""#,
            r#" KEY = "3""#,
            r#" key="" key="4""#,
            r#" enc_idx="1" idx="2""#,
            r#" 键key="5" key="6""#,
            r#" key=5 key="6""#,
            r#" key="&amp;""#,
            r" md5",
            "",
        ];
        for name in ["url", "thumb", "contentDesc", "id"] {
            let text = Regex::new(&format!(
                r"(?s)<{0}(?:\s[^>]*)?>(.*?)</{0}>",
                regex::escape(name)
            ))
            .unwrap();
            let attrs = Regex::new(&format!(r"<{}(\s[^>]*)?/?>", regex::escape(name))).unwrap();
            for s in samples {
                assert_eq!(
                    tag_text(s, name),
                    text.captures(s).map(|c| decode(&c[1])),
                    "{name} in {s:?}"
                );
                assert_eq!(
                    tag_attrs(s, name),
                    attrs
                        .captures(s)
                        .map(|c| c.get(1).map_or("", |m| m.as_str()).to_string()),
                    "{name} in {s:?}"
                );
            }
        }
        for name in ["key", "token", "md5", "enc_idx", "idx"] {
            let re =
                Regex::new(&format!(r#"(?i)\b{}\s*=\s*"([^"]*)""#, regex::escape(name))).unwrap();
            for s in attr_samples {
                let want = re
                    .captures(s)
                    .map(|c| decode(&c[1]))
                    .filter(|v| !v.is_empty());
                assert_eq!(attr(s, name), want, "{name} in {s:?}");
            }
        }
    }

    #[test]
    fn malformed_or_empty_xml_still_yields_a_post() {
        let p = parse_post(7, "wxid_x", "");
        assert_eq!((p.id.as_str(), p.create_time, p.media.len()), ("7", 0, 0));
        let p = parse_post(
            7,
            "",
            "<TimelineObject><username>wxid_y</username></TimelineObject>",
        );
        assert_eq!(p.username, "wxid_y");
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::fixture::{Fixture, TestDir, DAY, T0};

    fn account(tag: &str) -> (TestDir, Account) {
        let root = TestDir::new(&format!("weflow-sns-{tag}"));
        let f = Fixture::standard(&root);
        let acct = Account::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()));
        (root, acct)
    }

    fn descs(posts: &[SnsPost]) -> Vec<&str> {
        posts.iter().map(|p| p.content_desc.as_str()).collect()
    }

    #[test]
    fn timeline_is_newest_first_with_paging_and_filters() {
        let (_dir, a) = account("timeline");
        let all = a.sns_timeline(0, 0, &[], None, 0, 0).unwrap();
        assert_eq!(
            descs(&all),
            [
                "old style",
                "carol video",
                "second post",
                "my post",
                "bob trip"
            ]
        );
        assert_eq!(all[0].tid, (-5i64).cast_unsigned().to_string());
        assert_eq!(
            descs(&a.sns_timeline(2, 1, &[], None, 0, 0).unwrap()),
            ["carol video", "second post"]
        );
        assert_eq!(
            descs(
                &a.sns_timeline(0, 0, &["wxid_me".into()], None, 0, 0)
                    .unwrap()
            ),
            ["second post", "my post"]
        );
        assert_eq!(
            descs(&a.sns_timeline(0, 0, &[], Some("TRIP"), 0, 0).unwrap()),
            ["bob trip"],
            "keyword is case-insensitive"
        );
        assert_eq!(
            descs(
                &a.sns_timeline(0, 0, &[], None, T0 + DAY, T0 + 2 * DAY)
                    .unwrap()
            ),
            ["carol video", "second post"]
        );
        let trip = &a.sns_timeline(0, 0, &[], Some("trip"), 0, 0).unwrap()[0];
        assert_eq!(trip.media.len(), 2);
        assert_eq!(trip.media[0].thumb, "http://thumb/1?a=1&b=2");
        assert_eq!(trip.media[0].token.as_deref(), Some("TU1"));
        assert_eq!(trip.likes, ["Me Nick", "Carol"]);
        assert!(a.sns_timeline(0, 0, &[], None, 0, 1).unwrap().is_empty());
    }

    #[test]
    fn users_and_export_stats() {
        let (_dir, a) = account("stats");
        assert_eq!(
            a.sns_usernames().unwrap(),
            json!(["wxid_bob", "wxid_carol", "wxid_me"]),
            "most recent poster first"
        );
        assert_eq!(
            a.sns_export_stats(None).unwrap(),
            json!({"total_posts": 5, "total_friends": 3})
        );
        assert_eq!(
            a.sns_export_stats(Some("wxid_me_ab12")).unwrap(),
            json!({"total_posts": 5, "total_friends": 3, "my_posts": 2})
        );
        assert_eq!(
            a.sns_export_stats(Some("wxid_nobody")).unwrap()["my_posts"],
            0
        );
        // 解析出的时间线中每个发布者的动态数
        let mut from_timeline = BTreeMap::<String, i64>::new();
        for p in a.sns_timeline(0, 0, &[], None, 0, 0).unwrap() {
            *from_timeline.entry(p.username).or_default() += 1;
        }
        assert_eq!(from_timeline.values().sum::<i64>(), 5);
        assert_eq!(a.sns_post_counts().unwrap(), json!(from_timeline));
    }

    #[test]
    fn annual_stats_rank_likers_and_liked_friends() {
        let (_dir, a) = account("annual");
        let v = a.sns_annual_stats(0, 0).unwrap();
        assert_eq!(v["totalPosts"], 2, "only the owner's own posts");
        assert_eq!(v["typeCounts"], json!({"1": 1, "2": 1}));
        assert_eq!(
            v["topLikers"],
            json!([{"username": "wxid_bob", "count": 2}, {"username": "wxid_carol", "count": 1}])
        );
        assert_eq!(
            v["topLiked"],
            json!([{"username": "wxid_bob", "count": 1}, {"username": "wxid_carol", "count": 1}])
        );
        let narrow = a.sns_annual_stats(T0 + DAY, 0).unwrap();
        assert_eq!(narrow["totalPosts"], 1);
        assert_eq!(
            narrow["topLikers"],
            json!([{"username": "wxid_bob", "count": 1}])
        );
        assert_eq!(
            narrow["topLiked"],
            json!([{"username": "wxid_carol", "count": 1}])
        );
    }
}
