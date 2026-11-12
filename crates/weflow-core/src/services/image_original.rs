//! `image download-original` (#766): has WeChat download the original of one image message. The hook `weflow serve`
//! installs in WeChat (see [`crate::image_download`]) runs a request of ours on WeChat's own thread, with WeChat's own
//! download instance, at WeChat's next download; the original then appears as `<md5>_h.dat` beside the image, which
//! `chat image-info` reports and the WebUI shows instead of the image it opened.

use std::time::{SystemTime, UNIX_EPOCH};

use super::*;

/// What the `<img>` element of a message says about its original.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageOriginal {
    /// `cdnbigimgurl`: the CDN address the original is downloaded from.
    file_id: String,
    /// `aeskey`: the key the downloaded file is decrypted with.
    aeskey: String,
    /// `hdlength`: bytes of the original.
    bytes: u64,
}

impl ImageOriginal {
    /// The original's fields of the first `<img>` element of `content`, when it has all of them: some pictures have no
    /// original, and of some messages only the thumbnail's fields are kept.
    fn parse(content: &str) -> Option<Self> {
        Some(Self {
            file_id: attribute(content, "cdnbigimgurl")?,
            aeskey: attribute(content, "aeskey")?,
            bytes: attribute(content, "hdlength")?
                .parse()
                .ok()
                .filter(|bytes| *bytes > 0)?,
        })
    }
}

/// An attribute of the `<img>` element of `content`, without the XML escapes of a URL, unless it is absent or empty.
fn attribute(content: &str, name: &str) -> Option<String> {
    crate::xml::spaced_attribute(content, "<img", name)
        .map(|value| value.trim().replace("&amp;", "&"))
        .filter(|value| !value.is_empty())
}

/// The `clientid` of a download of ours. WeChat's own is `<conversation>_<timestamp>_<number>_<kind>`, which the hook
/// matches against the whitelist by the conversation; a number of our own and the kind 3, as the requests tried so far
/// use, keep this one from clashing with a download WeChat made.
fn request_clientid(session_id: &str, now: SystemTime, number: u32) -> String {
    let seconds = now.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    format!("{session_id}_{seconds}_{number}_3")
}

impl ServiceHub {
    /// `image download-original`: leaves a request with the hook of this process for WeChat to download the original
    /// of a message's image, which WeFlow gets as `<md5>_h.dat` (`chat image-info` then reports `hd`).
    pub fn image_download_original(
        &self,
        session_id: &str,
        msg_id: &str,
        create_time: Option<i64>,
    ) -> AppResult<crate::contract::ImageOriginalRequested> {
        // the hook first: without it nothing can be asked, whatever the message says
        let Some(hook) = self.image_auto_download.as_ref() else {
            return Err(AppError::unsupported(
                "the image auto-download hook is not running in this process: `weflow serve` starts it on Windows x64, \
                 unless `serve_image_auto_download` is false",
            ));
        };
        let (payload, content) = self.image_payload_and_content(session_id, msg_id, create_time)?;
        let original = ImageOriginal::parse(&content).ok_or_else(|| {
            AppError::runtime("the message does not give this picture's original: it may have none")
        })?;
        let md5 = payload
            .image_md5
            .as_deref()
            .map(str::trim)
            .filter(|md5| !md5.is_empty())
            .ok_or_else(|| {
                AppError::runtime("the picture has no md5, so where its original goes is unknown")
            })?;
        let fullpath = self.original_dat_path(&payload, md5)?;
        let clientid = request_clientid(session_id, SystemTime::now(), rand::random());
        hook.request_original(
            &clientid,
            &original.file_id,
            &original.aeskey,
            &fullpath,
            original.bytes,
        )
        .map_err(AppError::runtime)?;
        Ok(crate::contract::ImageOriginalRequested {
            success: true,
            path: fullpath,
            bytes: original.bytes,
        })
    }

    /// Where WeChat will write the original: the `<md5>_h.dat` beside the image, in the `Img` folder of its month.
    fn original_dat_path(&self, p: &ImagePayload, md5: &str) -> AppResult<String> {
        let account_dir = self.account_dir_only()?;
        let dir = super::image::image_month_dir(&account_dir, p.session_id.as_deref(), p.create_time)
            .ok_or_else(|| {
                AppError::runtime(
                    "the folder WeChat keeps this picture's files in is not on this computer: open the picture in \
                     WeChat first",
                )
            })?;
        Ok(dir
            .join(format!("{md5}_h.dat"))
            .to_string_lossy()
            .into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// The `<img>` element of an image message, as WeChat keeps it.
    const IMG: &str = r#"<msg><img aeskey="0123456789abcdef0123456789ABCDEF" encryver="1" cdnthumburl="http://cdn/t?m=1&amp;n=2" cdnthumblength="1024" cdnthumbheight="120" cdnthumbwidth="120" cdnmidheight="0" cdnmidwidth="0" cdnbigheight="1600" cdnbigwidth="1600" cdnbigimgurl="http://cdn/big?m=1&amp;n=2&amp;x=3" cdnmidimgurl="http://cdn/mid" length="240000" hdlength="2400000" md5="aabbccddeeff00112233445566778899"/></msg>"#;

    #[test]
    fn an_image_element_gives_the_originals_fields() {
        let original = ImageOriginal::parse(IMG).unwrap();
        assert_eq!(original.file_id, "http://cdn/big?m=1&n=2&x=3");
        assert_eq!(original.aeskey, "0123456789abcdef0123456789ABCDEF");
        assert_eq!(original.bytes, 2_400_000);
        // the thumbnail's address is not the original's
        assert!(!original.file_id.contains("/t?"));
    }

    #[test]
    fn an_image_without_the_originals_fields_has_none() {
        // only the image itself, without its original
        let small =
            r#"<msg><img aeskey="aa" cdnthumburl="http://cdn/t" length="2400" md5="aabb"/></msg>"#;
        assert_eq!(ImageOriginal::parse(small), None);
        // the size of nothing
        let empty = r#"<msg><img aeskey="aa" cdnbigimgurl="http://cdn/b" hdlength="0"/></msg>"#;
        assert_eq!(ImageOriginal::parse(empty), None);
        // `cdnthumbaeskey` is not the image's `aeskey`
        let thumb_key =
            r#"<msg><img cdnthumbaeskey="aa" cdnbigimgurl="http://cdn/b" hdlength="1"/></msg>"#;
        assert_eq!(ImageOriginal::parse(thumb_key), None);
        assert_eq!(ImageOriginal::parse("<msg></msg>"), None);
        assert_eq!(ImageOriginal::parse(""), None);
    }

    #[test]
    fn a_request_clientid_names_the_conversation_and_is_new() {
        let now = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(
            request_clientid("wxid_a", now, 42),
            "wxid_a_1700000000_42_3"
        );
        assert_eq!(
            request_clientid("1234@chatroom", UNIX_EPOCH, 0),
            "1234@chatroom_0_0_3"
        );
        // the hook matches the conversation by the part before the first `_`
        assert!(request_clientid("wxid_a", now, 1).starts_with("wxid_a_"));
    }

    #[test]
    fn a_hub_without_the_hook_says_so_before_reading_the_message() {
        let dir = weflow_native::fixture::TestDir::new("weflow-image-original");
        let hub = ServiceHub::new(
            crate::config::AppContext {
                home_dir: dir.join("home"),
                config_path: dir.join("home").join("config.json"),
                runtime_dir: dir.join("runtime"),
                version: "test".into(),
            },
            crate::config::ConfigStore::default(),
        );
        // the account is not even configured: the hook is what is missing, and it is asked first
        let err = hub
            .image_download_original("wxid_a", "3", None)
            .unwrap_err();
        assert_eq!(err.code, "unsupported");
        assert!(err.message.contains("image auto-download hook"));
    }
}
