//! Moments: posts, their media and comments, the counts, and what the Moments commands download and write.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Who has posts in Moments (`sns users`), the one who posted last first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsUsers {
    pub usernames: Vec<String>,
}

/// How many Moments posts there are (`sns stats`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsStats {
    pub total_posts: i64,
    /// How many people posted them.
    pub total_friends: i64,
    /// How many of them are the account's own; null when the account's wxid is not known.
    pub my_posts: Option<i64>,
}

/// How many Moments posts one person has (`sns post-counts --user`).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserPosts {
    pub username: String,
    pub total_posts: i64,
}

/// A page of Moments posts (`sns timeline`), the newest first.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsTimeline {
    pub timeline: Vec<SnsPost>,
}

/// A Moments post.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsPost {
    /// WeChat's id of the post.
    pub id: String,
    /// The row's id in WeChat's table, in decimal.
    pub tid: String,
    pub username: String,
    /// The poster's name: the post's own, else the contact's, else the username.
    pub nickname: String,
    pub create_time: i64,
    /// The text of the post.
    pub content_desc: String,
    /// WeChat's type of the post: 1 pictures, 15 a video, others links and such.
    #[serde(rename = "type")]
    pub kind: i64,
    /// The post's XML as WeChat keeps it.
    pub raw_xml: String,
    pub media: Vec<SnsMedia>,
    /// The names of those who liked it.
    pub likes: Vec<String>,
    pub comments: Vec<SnsComment>,
    /// The key of the post's video, which opens its cover too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SnsLocation>,
}

/// A picture or video of a post: addresses on WeChat's servers, fetched and decrypted by `sns media` with `key`.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsMedia {
    pub url: String,
    pub thumb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc_idx: Option<String>,
    /// With `with-media`: the server's addresses, `url` and `thumb` being those of the files it fetched, and where
    /// it fetched them or a proxy of the server serves them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_thumb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_thumb_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_thumb_url: Option<String>,
    /// The moving part of a live photo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_photo: Option<SnsLivePhoto>,
}

/// The video of a live photo.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsLivePhoto {
    pub url: String,
    pub thumb: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc_idx: Option<String>,
    /// With `with-media`: as for the picture it moves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_thumb: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_thumb_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_thumb_url: Option<String>,
}

/// A comment on a post.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsComment {
    pub id: String,
    pub nickname: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    pub content: String,
    /// The comment it answers; empty when none.
    pub ref_comment_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_nickname: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emojis: Vec<SnsEmoji>,
}

/// A sticker in a comment.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsEmoji {
    pub url: String,
    pub md5: String,
    pub width: Option<i64>,
    pub height: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypt_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aes_key: Option<String>,
}

/// Where a post was made, as much as it says.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsLocation {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<serde_json::Number>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_address_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// A Moments image or video `sns media` fetched.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsMediaFetched {
    /// The file it was written to, with `out`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    pub content_type: String,
    pub bytes: usize,
    /// Where WeFlow keeps it; null when it was not kept.
    pub cache_path: Option<PathBuf>,
}

/// What WeChat's media server answered `sns debug-resource` for the first bytes of a resource.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsResourceProbe {
    /// Always true.
    pub success: bool,
    /// The HTTP status.
    pub status: u16,
    pub headers: SnsResourceHeaders,
}

/// The answer headers that matter for decrypting a Moments resource; null when the server left one out.
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct SnsResourceHeaders {
    pub x_enc: Option<String>,
    pub x_time: Option<String>,
    pub content_length: Option<String>,
    pub content_type: Option<String>,
}

/// The Moments sticker `sns download-emoji` saved (or found already saved).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsEmojiSaved {
    /// Always true.
    pub success: bool,
    pub local_path: PathBuf,
}

impl SnsEmojiSaved {
    pub fn at(local_path: PathBuf) -> Self {
        Self {
            success: true,
            local_path,
        }
    }
}

/// The image `sns download-image` downloaded (and decrypted when it was a .dat payload).
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum SnsImageDownload {
    Saved(SnsImageSaved),
    Inline(SnsImageInline),
}

/// The image written to `out`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsImageSaved {
    pub url: String,
    pub out: PathBuf,
    /// The image's extension, with its dot.
    pub ext: String,
    /// Its size in bytes.
    pub size: usize,
}

/// The image in base64, without `out`.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsImageInline {
    pub url: String,
    /// The image's extension, with its dot.
    pub ext: String,
    /// Its size in bytes.
    pub size: usize,
    /// A `data:image/...;base64,` URL.
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A post as `sns timeline` builds it writes out and reads back into [`SnsPost`] the same, every field of it.
    #[test]
    fn a_moments_post_reads_into_the_contract_and_back_the_same() {
        let xml = r#"<TimelineObject><id>1395</id><createTime>1700000000</createTime>
            <contentDesc>at the river</contentDesc>
            <location city="Shanghai" poiName="Bund" latitude="31.2" longitude="121"/>
            <ContentObject><type>1</type><mediaList>
            <media><url md5="aa11" token="t1" key="k1" enc_idx="1">http://mmsns.qpic.cn/a</url><thumb token="t1">http://mmsns.qpic.cn/a/150</thumb>
              <livePhoto><url token="t2" key="k2" md5="bb22" enc_idx="1">http://mmsns.qpic.cn/v</url><thumb>http://mmsns.qpic.cn/v/150</thumb></livePhoto></media>
            </mediaList></ContentObject><enc key="987"/>
            <like_user_list><user_comment><nickname>Carol</nickname></user_comment></like_user_list>
            <comment_user_list>
              <user_comment><comment_id>1</comment_id><username>wxid_carol</username><nickname>Carol</nickname><content>nice</content><ref_comment_id>0</ref_comment_id></user_comment>
              <user_comment><comment_id>2</comment_id><username>wxid_bob</username><nickname>Bob</nickname><content></content><ref_comment_id>1</ref_comment_id><ref_username>wxid_carol</ref_username>
                <emojiinfo><url>http://emoji.qpic.cn/e</url><md5>cc33</md5><width>120</width></emojiinfo></user_comment>
            </comment_user_list></TimelineObject>"#;
        let contact = crate::sns::CachedContact {
            display_name: Some("Bobby".into()),
            avatar_url: Some("https://wx.qlogo.cn/b".into()),
        };
        let built = crate::sns::enrich_post(
            weflow_native::sns::parse_post(2001, "wxid_bob", xml),
            Some(&contact),
        );
        let post = serde_json::to_value(&built).unwrap();
        let typed: SnsPost = serde_json::from_value(post.clone()).unwrap();
        assert_eq!(serde_json::to_value(&typed).unwrap(), post);
        // the test has every kind of field to lose
        assert!(typed.video_key.is_some() && typed.avatar_url.is_some());
        let media = &typed.media[0];
        assert!(media.key.is_some() && media.enc_idx.is_some() && media.live_photo.is_some());
        let location = typed.location.as_ref().unwrap();
        assert_eq!(location.longitude.as_ref().unwrap().as_i64(), Some(121));
        assert_eq!(typed.comments.len(), 2);
        assert!(
            !typed.comments[1].emojis.is_empty() && typed.comments[1].emojis[0].height.is_none()
        );
        assert_eq!(typed.likes, ["Carol"]);
        // what `with-media` adds to a picture and its live photo
        let fetched = json!({
            "url": "/sns/media/a", "thumb": "/sns/media/a.t", "key": "k1", "rawUrl": "http://mmsns.qpic.cn/a",
            "rawThumb": "http://mmsns.qpic.cn/a/150", "resolvedUrl": "/sns/media/a", "proxyThumbUrl": "/p/a.t",
            "livePhoto": { "url": "/sns/media/v", "thumb": "", "rawUrl": "http://mmsns.qpic.cn/v", "proxyUrl": "/p/v" },
        });
        let media: SnsMedia = serde_json::from_value(fetched.clone()).unwrap();
        assert_eq!(serde_json::to_value(media).unwrap(), fetched);
        // and a field none of them knows fails rather than goes missing from the answer
        assert!(
            serde_json::from_value::<SnsMedia>(json!({ "url": "", "thumb": "", "size": 1 }))
                .is_err()
        );
    }
}
