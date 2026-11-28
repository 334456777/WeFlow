//! 朋友圈：动态及其媒体和评论、计数，以及朋友圈命令下载和写出的内容。

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// 在朋友圈发过动态的人（`sns users`），最近发的在前。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsUsers {
    /// 他们的 wxid。
    pub usernames: Vec<String>,
}

/// 朋友圈有多少条动态（`sns stats`）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsStats {
    /// 本机保存的动态数。
    pub total_posts: i64,
    /// 发了这些动态的人数。
    pub total_friends: i64,
    /// 其中账号自己的动态数；不知道账号的 wxid 时为 `null`。
    pub my_posts: Option<i64>,
}

/// 一个人有多少条朋友圈动态（`sns post-counts --user`）。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserPosts {
    /// 这个人的 wxid。
    pub username: String,
    /// 本机保存的这个人的动态数。
    pub total_posts: i64,
}

/// 朋友圈动态的一页（`sns timeline`），从新到旧。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsTimeline {
    /// 这一页的动态。
    pub timeline: Vec<SnsPost>,
}

/// 一条朋友圈动态。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsPost {
    /// 微信给动态的 ID；XML 中没有时为 `tid`。
    pub id: String,
    /// 这一行在微信的表中的 ID，写成十进制。
    pub tid: String,
    /// 发布者的 wxid。
    pub username: String,
    /// 发布者在账号中的显示名：备注，没有时依次用昵称、微信号和用户名本身。
    pub nickname: String,
    /// 发布的时间；动态中没有写时为 0。
    pub create_time: i64,
    /// 动态的文字。
    pub content_desc: String,
    /// 微信的动态类型：1 为图片，15 为视频，其他为链接等；XML 中没有时为 1。
    #[serde(rename = "type")]
    pub kind: i64,
    /// 微信保存的动态 XML。
    pub raw_xml: String,
    /// 动态中的图片或视频。
    pub media: Vec<SnsMedia>,
    /// 点赞者的昵称。
    pub likes: Vec<String>,
    /// 评论，按动态 XML 中的顺序。
    pub comments: Vec<SnsComment>,
    /// 动态视频的密钥（`<enc key>`），也能解开视频的封面。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_key: Option<String>,
    /// 发布者头像的地址；没有时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
    /// 发布的地点；动态中没有写时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<SnsLocation>,
}

/// 动态中的一张图片或一段视频：微信服务器上的地址，由 `sns media` 用 `key` 取回并解密。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsMedia {
    /// 完整的图片或视频：微信服务器的地址，改用 https 并带上 `token`；带 `with-media` 时为 `resolvedUrl`，
    /// 没有时仍为微信的地址。
    pub url: String,
    /// 缩略图，写法同 `url`。
    pub thumb: String,
    /// 动态中写的文件 MD5。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    /// 微信服务器要求的访问令牌。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// 解密文件的密钥，传给 `sns media`；视频优先用动态的视频密钥（#202）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// 动态给文件写的 `enc_idx` 属性。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc_idx: Option<String>,
    /// 带 `with-media` 时：完整的图片或视频在微信服务器上的地址。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_url: Option<String>,
    /// 带 `with-media` 时：缩略图在微信服务器上的地址。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_thumb: Option<String>,
    /// 带 `with-media` 时：取回并解密的文件，写成 data URL；取不到时为 `proxyUrl`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_url: Option<String>,
    /// 带 `with-media` 时：缩略图，写法同 `resolvedUrl`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_thumb_url: Option<String>,
    /// 带 `with-media` 时：`weflow serve` 上取回并解密这个文件的路由（`/api/sns/media`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// 带 `with-media` 时：取回并解密缩略图的路由。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_thumb_url: Option<String>,
    /// 实况照片中动的部分。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub live_photo: Option<SnsLivePhoto>,
}

/// 实况照片中的视频。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsLivePhoto {
    /// 视频，写法同图片的 `url`。
    pub url: String,
    /// 视频的缩略图，写法同图片的 `thumb`。
    pub thumb: String,
    /// 微信服务器要求的访问令牌。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// 解密视频的密钥：动态的视频密钥，没有时依次为它自己的密钥和图片的密钥。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// 动态中写的视频 MD5。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md5: Option<String>,
    /// 动态给视频写的 `enc_idx` 属性。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enc_idx: Option<String>,
    /// 带 `with-media` 时：同图片的 `rawUrl`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_url: Option<String>,
    /// 带 `with-media` 时：同图片的 `rawThumb`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_thumb: Option<String>,
    /// 带 `with-media` 时：同图片的 `resolvedUrl`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_url: Option<String>,
    /// 带 `with-media` 时：同图片的 `resolvedThumbUrl`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_thumb_url: Option<String>,
    /// 带 `with-media` 时：同图片的 `proxyUrl`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_url: Option<String>,
    /// 带 `with-media` 时：同图片的 `proxyThumbUrl`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_thumb_url: Option<String>,
}

/// 动态下的一条评论。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsComment {
    /// 微信给评论的 ID；动态中没有时为生成的 `cmt_…`。
    pub id: String,
    /// 评论者的昵称。
    pub nickname: String,
    /// 评论者的 wxid；动态中没有写时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// 评论的文字；只有表情的评论为空。
    pub content: String,
    /// 所回复的评论；没有时为空。
    pub ref_comment_id: String,
    /// 所回复的人的 wxid；不是回复，或动态中没有写时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_username: Option<String>,
    /// 所回复的人的昵称：动态中写的，没有时按 `refUsername` 从同一条动态的评论中找；找不到时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ref_nickname: Option<String>,
    /// 评论中的表情；没有时省略。
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emojis: Vec<SnsEmoji>,
}

/// 评论中的一个表情。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsEmoji {
    /// 不加密的地址；只有 `encryptUrl` 时为空。
    pub url: String,
    /// 表情的 MD5；评论中没有写时为空。
    pub md5: String,
    /// 宽度，单位为像素；评论中没有写时为 `null`。
    pub width: Option<i64>,
    /// 高度，单位为像素；评论中没有写时为 `null`。
    pub height: Option<i64>,
    /// 加密文件的地址；没有时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypt_url: Option<String>,
    /// `encryptUrl` 的 AES 密钥；没有时省略。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aes_key: Option<String>,
}

/// 发布动态的地点，动态中写了多少就有多少。
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SnsLocation {
    /// 纬度，单位为度，按动态中写的数字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latitude: Option<serde_json::Number>,
    /// 经度，单位为度，按动态中写的数字。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub longitude: Option<serde_json::Number>,
    /// 城市。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub city: Option<String>,
    /// 国家。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// 地点的名称。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_name: Option<String>,
    /// 地点的地址。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_address: Option<String>,
    /// 地址的名称（`poiAddressName`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poi_address_name: Option<String>,
    /// 动态为这个地点显示的标签。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// `sns media` 取回的一张朋友圈图片或一段视频。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsMediaFetched {
    /// 带 `out` 时写入的文件；不带时省略。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub out: Option<String>,
    /// 文件的 MIME 类型。
    pub content_type: String,
    /// 文件的字节数，有密钥时为解密后的大小。
    pub bytes: usize,
    /// WeFlow 缓存它的位置；没有缓存时为 `null`。
    pub cache_path: Option<PathBuf>,
}

/// 微信的媒体服务器对 `sns debug-resource` 请求资源前几个字节的回答。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsResourceProbe {
    /// HTTP 状态码。
    pub status: u16,
    /// 与解密有关的回答头。
    pub headers: SnsResourceHeaders,
}

/// 与解密朋友圈资源有关的回答头；服务器没有给出的为 `null`。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub struct SnsResourceHeaders {
    /// `X-Enc` 头。
    pub x_enc: Option<String>,
    /// `X-Time` 头。
    pub x_time: Option<String>,
    /// `Content-Length` 头。
    pub content_length: Option<String>,
    /// `Content-Type` 头。
    pub content_type: Option<String>,
}

/// `sns download-emoji` 保存（或发现已经保存过）的朋友圈表情。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SnsEmojiSaved {
    /// WeFlow 缓存中的文件。
    pub local_path: PathBuf,
}

impl SnsEmojiSaved {
    /// 保存在 `local_path` 的表情的回答。
    pub fn at(local_path: PathBuf) -> Self {
        Self { local_path }
    }
}

/// `sns download-image` 下载的图片；是 `.dat` 格式时用图片密钥解密。
#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum SnsImageDownload {
    /// 写进了 `out`。
    Saved(SnsImageSaved),
    /// 不带 `out` 时放在回答中。
    Inline(SnsImageInline),
}

/// 写进 `out` 的图片。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsImageSaved {
    /// 下载的地址。
    pub url: String,
    /// 写出的文件。
    pub out: PathBuf,
    /// 图片的扩展名，带点。
    pub ext: String,
    /// 图片的字节数。
    pub size: usize,
}

/// 不带 `out` 时以 base64 放在回答中的图片。
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SnsImageInline {
    /// 下载的地址。
    pub url: String,
    /// 图片的扩展名，带点。
    pub ext: String,
    /// 图片的字节数。
    pub size: usize,
    /// `data:image/...;base64,` 形式的 URL。
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// `sns timeline` 生成的动态写出后再读回 [`SnsPost`]，每个字段都不变。
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
        // 测试数据含有每一种可能丢失的字段
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
        // `with-media` 给图片和它的实况照片加上的字段
        let fetched = json!({
            "url": "/sns/media/a", "thumb": "/sns/media/a.t", "key": "k1", "rawUrl": "http://mmsns.qpic.cn/a",
            "rawThumb": "http://mmsns.qpic.cn/a/150", "resolvedUrl": "/sns/media/a", "proxyThumbUrl": "/p/a.t",
            "livePhoto": { "url": "/sns/media/v", "thumb": "", "rawUrl": "http://mmsns.qpic.cn/v", "proxyUrl": "/p/v" },
        });
        let media: SnsMedia = serde_json::from_value(fetched.clone()).unwrap();
        assert_eq!(serde_json::to_value(media).unwrap(), fetched);
        // 契约类型不认识的字段让读取失败，而不是从回答中丢掉
        assert!(
            serde_json::from_value::<SnsMedia>(json!({ "url": "", "thumb": "", "size": 1 }))
                .is_err()
        );
    }
}
