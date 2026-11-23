//! Stickers: their addresses in the messages, and downloading them into the cache.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::message::emoji::{extract_emoji_md5, extract_emoji_url};

/// Extensions a downloaded sticker is saved under: the image formats, then `.bin` for any other response.
const CACHED_EXTENSIONS: [&str; 6] = [".gif", ".png", ".webp", ".jpg", ".jpeg", ".bin"];

/// A sticker to download: its address and the name its file is saved under.
#[derive(Debug, Clone)]
pub struct EmojiMeta {
    /// The sticker's address on WeChat's CDN.
    pub cdn_url: String,
    /// The sticker's md5 (32 lowercase hex digits), or a 16-hex-digit hash of `cdn_url` when the message has none;
    /// only hex digits, so it is safe as a file name.
    pub md5: String,
}

/// The stickers (type 47) among `messages` that have an address, parsed as the chat export parses them.
pub fn extract_emoji_urls(messages: &[weflow_native::msg_row::MsgRow]) -> Vec<EmojiMeta> {
    messages
        .iter()
        .filter(|msg| msg.local_type == 47)
        .filter_map(|msg| emoji_meta(&msg.content))
        .collect()
}

fn emoji_meta(content: &str) -> Option<EmojiMeta> {
    let cdn_url = extract_emoji_url(content)
        .map(|url| url.trim().to_string())
        .filter(|url| !url.is_empty())?;
    let md5 = extract_emoji_md5(content).unwrap_or_else(|| simple_hash(&cdn_url));
    Some(EmojiMeta { cdn_url, md5 })
}

/// What [`download_emojis`] left on disk, and how many stickers it could not download.
#[derive(Debug, Default)]
pub struct EmojiDownloads {
    /// The stickers on disk: the ones already there and the ones downloaded now, in message order.
    pub files: Vec<crate::contract::EmojiFile>,
    /// The stickers whose request or write failed; they are not in `files`.
    pub failed: usize,
}

/// Downloads each of `metas` into `cache_dir` as `<md5><ext>`, skipping the ones already there; a sticker whose
/// request or write fails is left out of `files` and counted in `failed`. `progress_cb` gets `(done, total)` before
/// each sticker and once at the end.
///
/// # Errors
///
/// When `cache_dir` cannot be created or the HTTP client cannot be built.
pub async fn download_emojis(
    metas: &[EmojiMeta],
    cache_dir: &Path,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<EmojiDownloads> {
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("create emoji cache dir {}", cache_dir.display()))?;

    let client = crate::http_client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build reqwest client")?;

    let total = metas.len();
    let mut downloads = EmojiDownloads::default();
    for (idx, meta) in metas.iter().enumerate() {
        progress_cb(idx, total);
        // skip a sticker cached already, under any picture extension
        let file = match find_cached_emoji(cache_dir, &meta.md5) {
            Some(p) => Some(crate::contract::EmojiFile {
                md5: meta.md5.clone(),
                cached: true,
                path: p.to_string_lossy().into_owned(),
                size: None,
            }),
            None => fetch_emoji(&client, meta, cache_dir).await,
        };
        match file {
            Some(file) => downloads.files.push(file),
            None => downloads.failed += 1,
        }
    }
    progress_cb(total, total);
    Ok(downloads)
}

/// Downloads `meta` into `cache_dir`, named after its format; `None` when the request fails, the server answers with
/// an error status, or the file cannot be written.
async fn fetch_emoji(
    client: &reqwest::Client,
    meta: &EmojiMeta,
    cache_dir: &Path,
) -> Option<crate::contract::EmojiFile> {
    let resp = client.get(&meta.cdn_url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let bytes = resp.bytes().await.ok()?;
    let ext =
        crate::image::ImageFormat::of(&bytes).map_or(".bin", crate::image::ImageFormat::extension);
    let out_path = cache_dir.join(format!("{}{}", meta.md5, ext));
    fs::write(&out_path, &bytes).ok()?;
    Some(crate::contract::EmojiFile {
        md5: meta.md5.clone(),
        cached: false,
        path: out_path.to_string_lossy().into_owned(),
        size: Some(bytes.len()),
    })
}

fn find_cached_emoji(cache_dir: &Path, md5: &str) -> Option<PathBuf> {
    CACHED_EXTENSIONS
        .iter()
        .map(|ext| cache_dir.join(format!("{md5}{ext}")))
        .find(|p| p.exists())
}

fn simple_hash(s: &str) -> String {
    let mut h: u64 = 14695981039346656037;
    for b in s.bytes() {
        h = h.wrapping_mul(1099511628211);
        h ^= u64::from(b);
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD5: &str = "0123456789abcdef0123456789abcdef";

    fn meta_of(xml: &str) -> Option<EmojiMeta> {
        extract_emoji_urls(&[emoji_row(47, xml)]).pop()
    }

    #[test]
    fn extract_emoji_urls_from_messages() {
        let xml = format!(
            r#"<msg><emoji cdnurl="https://cdn.example.com/emoji.gif" md5="{}" /></msg>"#,
            MD5.to_uppercase()
        );
        let messages = [emoji_row(47, &xml), emoji_row(1, "hello")];
        let metas = extract_emoji_urls(&messages);
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].md5, MD5);
        assert_eq!(metas[0].cdn_url, "https://cdn.example.com/emoji.gif");
    }

    #[test]
    fn an_md5_that_is_not_hex_never_reaches_the_path() {
        let url = "https://cdn.example.com/emoji?id=1";
        for md5 in [
            "../../evil",
            r"..\..\evil",
            "/abs/evil",
            "C:/evil",
            "abc123def456",
        ] {
            let xml = format!(r#"<msg><emoji cdnurl="{url}" md5="{md5}" /></msg>"#);
            let meta = meta_of(&xml).expect("the sticker has an address");
            assert_eq!(meta.md5, simple_hash(url), "md5 {md5:?}");
            assert!(meta.md5.bytes().all(|b| b.is_ascii_hexdigit()), "{meta:?}");
            let dir = Path::new("out");
            assert_eq!(
                dir.join(format!("{}.gif", meta.md5)).parent(),
                Some(dir),
                "md5 {md5:?}"
            );
        }
    }

    #[test]
    fn spaced_attributes_and_escaped_ampersands_are_read() {
        let xml = format!(
            r#"<msg><emoji cdnurl = "https://cdn.example.com/e?a=1&amp;filekey=2" md5 = "{MD5}" /></msg>"#
        );
        let meta = meta_of(&xml).expect("a spaced cdnurl is read");
        assert_eq!(meta.cdn_url, "https://cdn.example.com/e?a=1&filekey=2");
        assert_eq!(meta.md5, MD5);
    }

    #[test]
    fn only_androidmd5_takes_its_value_only_when_it_is_an_md5() {
        let url = "https://cdn.example.com/e";
        let hex = format!(r#"<msg><emoji cdnurl="{url}" androidmd5="{MD5}" /></msg>"#);
        assert_eq!(meta_of(&hex).map(|m| m.md5).as_deref(), Some(MD5));
        let bad = format!(r#"<msg><emoji cdnurl="{url}" androidmd5="../x" /></msg>"#);
        assert_eq!(meta_of(&bad).map(|m| m.md5), Some(simple_hash(url)));
    }

    #[test]
    fn a_sticker_without_an_address_is_skipped() {
        let xml = format!(r#"<msg><emoji cdnurl="" md5="{MD5}" /></msg>"#);
        assert!(meta_of(&xml).is_none());
    }

    #[test]
    fn a_sticker_saved_as_bin_counts_as_cached() {
        let dir = weflow_native::fixture::TestDir::new("weflow-emoji-cache");
        assert_eq!(find_cached_emoji(&dir, MD5), None);
        let bin = dir.join(format!("{MD5}.bin"));
        fs::write(&bin, b"not an image").unwrap();
        assert_eq!(find_cached_emoji(&dir, MD5), Some(bin));
    }

    fn emoji_row(local_type: i64, content: &str) -> weflow_native::msg_row::MsgRow {
        weflow_native::msg_row::MsgRow {
            local_id: 1,
            server_id: None,
            local_type,
            sort_seq: None,
            create_time: None,
            status: None,
            sender: None,
            is_send: false,
            content: content.into(),
            source: None,
            packed_info: None,
            shard: 0,
        }
    }

    #[test]
    fn extract_emoji_no_emoji_type_skipped() {
        let messages = [emoji_row(
            1,
            r#"<emoji cdnurl="https://x.com/e.gif" md5="aaa"/>"#,
        )];
        assert!(extract_emoji_urls(&messages).is_empty());
    }
}
