use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::biz::extract_xml_value;

#[derive(Debug, Clone)]
pub struct EmojiMeta {
    pub cdn_url: String,
    pub md5: String,
}

pub fn extract_emoji_urls(messages: &[weflow_native::msg_row::MsgRow]) -> Vec<EmojiMeta> {
    let mut out = Vec::new();
    for msg in messages {
        if msg.local_type != 47 {
            continue;
        }
        let content = msg.content.as_str();
        if content.is_empty() {
            continue;
        }
        // Try attribute-style: cdnurl="..."
        let cdn_url = extract_attr(content, "cdnurl")
            .or_else(|| extract_attr(content, "thumburl"))
            .or_else(|| {
                let v = extract_xml_value(content, "cdnurl");
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            })
            .unwrap_or_default();
        if cdn_url.is_empty() {
            continue;
        }
        let md5 = extract_attr(content, "md5")
            .or_else(|| {
                let v = extract_xml_value(content, "md5");
                if v.is_empty() {
                    None
                } else {
                    Some(v)
                }
            })
            .unwrap_or_else(|| simple_hash(&cdn_url));
        out.push(EmojiMeta { cdn_url, md5 });
    }
    out
}

pub async fn download_emojis(
    metas: &[EmojiMeta],
    cache_dir: &Path,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<Vec<crate::contract::EmojiFile>> {
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("create emoji cache dir {}", cache_dir.display()))?;

    let client = crate::http_client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .context("build reqwest client")?;

    let total = metas.len();
    let mut results = Vec::new();
    for (idx, meta) in metas.iter().enumerate() {
        progress_cb(idx, total);
        // Skip if already cached (any extension)
        let existing = find_cached_emoji(cache_dir, &meta.md5);
        if let Some(p) = existing {
            results.push(crate::contract::EmojiFile {
                md5: meta.md5.clone(),
                cached: true,
                path: p.to_string_lossy().into_owned(),
                size: None,
            });
            continue;
        }
        match client.get(&meta.cdn_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let bytes = match resp.bytes().await {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let ext = crate::image::ImageFormat::of(&bytes)
                    .map_or(".bin", crate::image::ImageFormat::extension);
                let out_path = cache_dir.join(format!("{}{}", meta.md5, ext));
                if fs::write(&out_path, &bytes).is_ok() {
                    results.push(crate::contract::EmojiFile {
                        md5: meta.md5.clone(),
                        cached: false,
                        path: out_path.to_string_lossy().into_owned(),
                        size: Some(bytes.len()),
                    });
                }
            }
            _ => continue,
        }
    }
    progress_cb(total, total);
    Ok(results)
}

fn find_cached_emoji(cache_dir: &Path, md5: &str) -> Option<PathBuf> {
    for ext in &[".gif", ".png", ".webp", ".jpg", ".jpeg"] {
        let p = cache_dir.join(format!("{md5}{ext}"));
        if p.exists() {
            return Some(p);
        }
    }
    None
}

fn extract_attr(xml: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = xml.find(&needle)? + needle.len();
    let end = xml[start..].find('"')? + start;
    let val = xml[start..end].trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

fn simple_hash(s: &str) -> String {
    let mut h: u64 = 14695981039346656037;
    for b in s.bytes() {
        h = h.wrapping_mul(1099511628211);
        h ^= b as u64;
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_emoji_urls_from_messages() {
        let xml =
            r#"<msg><emoji cdnurl="https://cdn.example.com/emoji.gif" md5="abc123def456" /></msg>"#;
        let messages = [emoji_row(47, xml), emoji_row(1, "hello")];
        let metas = extract_emoji_urls(&messages);
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].md5, "abc123def456");
        assert!(metas[0].cdn_url.contains("cdn.example.com"));
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
