//! Moments (`sns.db`): timeline posts parsed from their `TimelineObject` XML, plus user and year statistics.
//!
//! `SnsTimeLine(tid, user_name, content, pack_info_buf)` holds one XML document per post. `tid` is an
//! unsigned 64-bit snowflake stored as a signed integer, so it is always printed as `tid as u64`.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;
use serde_json::{json, Map, Value};

use crate::native_db::{same_identity, NativeAccount};

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

/// Decode the XML entities WeChat uses (and unwrap CDATA).
fn decode(s: &str) -> String {
    let s = s.trim();
    let s = s
        .strip_prefix("<![CDATA[")
        .and_then(|t| t.strip_suffix("]]>"))
        .unwrap_or(s);
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
        .trim()
        .to_string()
}

/// Each opening `<name>` or `<name ...>` tag: (where its attributes begin, where they end, just after the tag).
/// Read with plain string search: a post is parsed for every field of every one shown, and a regex built per
/// call cost a millisecond a post.
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

/// Text of the first `<name ...>text</name>`.
fn tag_text(xml: &str, name: &str) -> Option<String> {
    let close = format!("</{name}>");
    open_tags(xml, name)
        .find_map(|(_, _, inner)| xml[inner..].find(&close).map(|n| &xml[inner..inner + n]))
        .map(decode)
}

/// Attribute string of the first opening `<name ...>` tag.
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

/// The value of `name="value"` (any case) in an attribute string, decoded; `None` when empty.
fn attr(attrs: &str, name: &str) -> Option<String> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    // ASCII lowering keeps every byte where it was
    let lower = attrs.to_ascii_lowercase();
    let wanted = name.to_ascii_lowercase();
    lower
        .match_indices(wanted.as_str())
        .find_map(|(at, _)| {
            let before = attrs[..at].chars().next_back();
            let first = name.chars().next()?;
            // `\b` before the name: between a word character and another kind
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

/// `{ url, thumb, token, key, md5, encIdx }` for a `<media>`/`<livePhoto>` fragment.
fn media_item(frag: &str) -> Value {
    let (url_attrs, thumb_attrs) = (
        tag_attrs(frag, "url").unwrap_or_default(),
        tag_attrs(frag, "thumb").unwrap_or_default(),
    );
    let pick = |name: &str| attr(&url_attrs, name).or_else(|| attr(&thumb_attrs, name));
    let mut o = Map::new();
    o.insert(
        "url".into(),
        json!(tag_text(frag, "url").unwrap_or_default()),
    );
    o.insert(
        "thumb".into(),
        json!(tag_text(frag, "thumb").unwrap_or_default()),
    );
    for (k, v) in [
        ("token", pick("token")),
        ("key", pick("key")),
        ("md5", attr(&url_attrs, "md5")),
        ("encIdx", pick("enc_idx")),
    ] {
        if let Some(v) = v {
            o.insert(k.into(), json!(v));
        }
    }
    Value::Object(o)
}

/// `<createTime>` of a post, `0` when it has none.
fn post_time(xml: &str) -> i64 {
    R_CREATE
        .captures(xml)
        .and_then(|c| c[1].parse::<i64>().ok())
        .unwrap_or(0)
}

/// One timeline row → the post object the service layer expects.
pub fn parse_post(tid: i64, user_name: &str, xml: &str) -> Value {
    let tid_text = (tid as u64).to_string();
    let id = R_ID
        .captures(xml)
        .map(|c| c[1].to_string())
        .unwrap_or_else(|| tid_text.clone());
    let create_time = post_time(xml);
    let post_type = R_TYPE
        .captures(xml)
        .and_then(|c| c[1].parse::<i64>().ok())
        .unwrap_or(1);
    let username = if user_name.is_empty() {
        tag_text(xml, "username").unwrap_or_default()
    } else {
        user_name.to_string()
    };

    let media: Vec<Value> = R_MEDIA
        .captures_iter(xml)
        .map(|c| {
            let frag = &c[1];
            let mut item = media_item(frag);
            if let Some(live) = R_LIVE.captures(frag) {
                item["livePhoto"] = media_item(&live[1]);
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

    let mut post = json!({
        "id": id, "tid": tid_text, "username": username, "nickname": "", "createTime": create_time,
        "contentDesc": tag_text(xml, "contentDesc").unwrap_or_default(), "type": post_type,
        "rawXml": xml, "media": media, "likes": likes, "comments": []
    });
    if let Some(key) = R_ENC_KEY.captures(xml) {
        post["videoKey"] = json!(key[1].to_string());
    }
    post
}

/// Usernames that liked the post (from `<like_user_list>`).
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

impl NativeAccount {
    fn sns_db(&self) -> std::path::PathBuf {
        self.db_path("sns/sns.db")
    }

    /// Every post `(tid, username, xml)` — cheap: one row per post.
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

    /// Newest-first page of posts. `keyword` matches the post text; `start`/`end` are seconds (`0` = unbounded).
    pub fn sns_timeline(
        &self,
        limit: i32,
        offset: i32,
        usernames: &[String],
        keyword: Option<&str>,
        start: i64,
        end: i64,
    ) -> Result<Value> {
        let needle = keyword
            .map(|k| k.trim().to_lowercase())
            .filter(|k| !k.is_empty());
        // only the time and the text of every post, to filter and order them; the page alone is parsed whole
        let mut posts: Vec<(i64, u64, String, String)> = self
            .sns_rows(usernames)?
            .into_iter()
            .map(|(tid, user, xml)| (post_time(&xml), tid as u64, user, xml))
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
            .skip(offset.max(0) as usize)
            .map(|(_, tid, user, xml)| parse_post(tid as i64, &user, &xml));
        Ok(Value::Array(if limit > 0 {
            it.take(limit as usize).collect()
        } else {
            it.collect()
        }))
    }

    /// Distinct usernames that have posts, most recent poster first.
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

    /// `{ username: posts }`: the poster of each post is its `user_name`, or the `<username>` of its XML when
    /// that is empty, as in [`parse_post`]. Counted from the table, without parsing the posts.
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

    /// `{ total_posts, total_friends, my_posts }` (`my_posts` only when `my_wxid` is given).
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

    /// Year statistics of the account owner: own post count and types, friends who liked the most,
    /// and friends whose posts the owner liked the most.
    pub fn sns_annual_stats(&self, begin: i64, end: i64) -> Result<Value> {
        let mut total = 0i64;
        let mut types: BTreeMap<i64, i64> = BTreeMap::new();
        let (mut likers, mut liked): (HashMap<String, i64>, HashMap<String, i64>) =
            Default::default();
        for (tid, user, xml) in self.sns_rows(&[])? {
            let t = post_time(&xml);
            if (begin > 0 && t < begin) || (end > 0 && t > end) {
                continue;
            }
            let mine = self.is_me(&user);
            let likes = liker_usernames(&xml);
            if mine {
                total += 1;
                *types
                    .entry(parse_post(tid, &user, &xml)["type"].as_i64().unwrap_or(1))
                    .or_default() += 1;
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
        assert_eq!(p["id"], "14000000000000000001");
        assert_eq!(
            p["tid"], "14446744073709551616",
            "a negative stored tid is the unsigned snowflake"
        );
        assert_eq!(
            (
                p["username"].as_str(),
                p["createTime"].as_i64(),
                p["type"].as_i64()
            ),
            (Some("wxid_bob"), Some(1_700_000_500), Some(1))
        );
        assert_eq!(p["contentDesc"], "hello & <moments>");
        let media = p["media"].as_array().unwrap();
        assert_eq!(media.len(), 2);
        assert_eq!(media[0]["url"], "http://u/1");
        assert_eq!(
            media[0]["thumb"], "http://t/1?a=1&b=2",
            "entities in URLs are decoded"
        );
        assert_eq!(
            (
                media[0]["token"].as_str(),
                media[0]["key"].as_str(),
                media[0]["md5"].as_str(),
                media[0]["encIdx"].as_str()
            ),
            (Some("TU"), Some("7"), Some("abc"), Some("2"))
        );
        assert_eq!(media[0]["livePhoto"]["url"], "http://l/1");
        assert_eq!(
            (
                media[0]["livePhoto"]["token"].as_str(),
                media[0]["livePhoto"]["key"].as_str()
            ),
            (Some("LT"), Some("5"))
        );
        assert!(media[1].get("token").is_none() && media[1].get("livePhoto").is_none());
        assert_eq!(p["likes"], json!(["Carol", "Me"]));
        assert_eq!(p["videoKey"], "2105122989");
        assert_eq!(p["rawXml"], XML);
        assert_eq!(liker_usernames(XML), ["wxid_carol", "wxid_me"]);
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
            r#" md5"#,
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
        assert_eq!(
            (
                p["id"].as_str(),
                p["createTime"].as_i64(),
                p["media"].as_array().unwrap().len()
            ),
            (Some("7"), Some(0), 0)
        );
        let p = parse_post(
            7,
            "",
            "<TimelineObject><username>wxid_y</username></TimelineObject>",
        );
        assert_eq!(p["username"], "wxid_y");
    }
}

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::fixture::{Fixture, DAY, T0};

    fn account(tag: &str) -> NativeAccount {
        let root = std::env::temp_dir().join(format!("weflow-sns-{}-{tag}", std::process::id()));
        let f = Fixture::standard(&root);
        NativeAccount::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()))
    }

    fn descs(v: &Value) -> Vec<&str> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|p| p["contentDesc"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn timeline_is_newest_first_with_paging_and_filters() {
        let a = account("timeline");
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
        assert_eq!(all[0]["tid"], (-5i64 as u64).to_string());
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
        assert_eq!(trip["media"].as_array().unwrap().len(), 2);
        assert_eq!(trip["media"][0]["thumb"], "http://thumb/1?a=1&b=2");
        assert_eq!(trip["media"][0]["token"], "TU1");
        assert_eq!(trip["likes"], json!(["Me Nick", "Carol"]));
        assert_eq!(a.sns_timeline(0, 0, &[], None, 0, 1).unwrap(), json!([]));
    }

    #[test]
    fn users_and_export_stats() {
        let a = account("stats");
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
        // the posts of each poster, as the parsed timeline has them
        let mut from_timeline = BTreeMap::<String, i64>::new();
        for p in a
            .sns_timeline(0, 0, &[], None, 0, 0)
            .unwrap()
            .as_array()
            .unwrap()
        {
            *from_timeline
                .entry(p["username"].as_str().unwrap().to_string())
                .or_default() += 1;
        }
        assert_eq!(from_timeline.values().sum::<i64>(), 5);
        assert_eq!(a.sns_post_counts().unwrap(), json!(from_timeline));
    }

    #[test]
    fn annual_stats_rank_likers_and_liked_friends() {
        let a = account("annual");
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
