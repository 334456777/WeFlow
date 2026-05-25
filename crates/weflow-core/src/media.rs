use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Value};
use walkdir::WalkDir;

use crate::biz::extract_xml_value;
use crate::decrypt::{decrypt_file, detect_dat_version};

pub struct ImageEntry {
    pub path: PathBuf,
    pub relative: String,
}

pub struct VoiceEntry {
    pub path: PathBuf,
    pub relative: String,
    pub session_id: String,
}

#[derive(Debug, Clone)]
pub struct EmojiMeta {
    pub cdn_url: String,
    pub md5: String,
}

pub fn scan_image_files(account_dir: &Path) -> Vec<ImageEntry> {
    let mut entries = Vec::new();
    for sub in ["FileStorage/Image", "FileStorage/Image2"] {
        let dir = account_dir.join(sub);
        if !dir.exists() {
            continue;
        }
        for entry in WalkDir::new(&dir).into_iter().filter_map(|e| e.ok()) {
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.path().to_path_buf();
            let relative = path
                .strip_prefix(account_dir)
                .map(normalize_path)
                .unwrap_or_default();
            entries.push(ImageEntry { path, relative });
        }
    }
    entries
}

pub fn scan_voice_files(account_dir: &Path) -> Vec<VoiceEntry> {
    let mut entries = Vec::new();
    let audio_dir = account_dir.join("FileStorage/Audio");
    if !audio_dir.exists() {
        return entries;
    }
    for entry in WalkDir::new(&audio_dir).into_iter().filter_map(|e| e.ok()) {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path().to_path_buf();
        let ext = path
            .extension()
            .and_then(|e: &std::ffi::OsStr| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext != "slk" && ext != "amr" && ext != "silk" {
            continue;
        }
        let relative = path
            .strip_prefix(account_dir)
            .map(normalize_path)
            .unwrap_or_default();
        let session_id = path
            .parent()
            .and_then(|p: &Path| p.file_name())
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        entries.push(VoiceEntry { path, relative, session_id });
    }
    entries
}

pub fn export_images(
    entries: &[ImageEntry],
    xor_key: u8,
    aes_key: Option<&[u8; 16]>,
    out_dir: &Path,
    session_filter: Option<&str>,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<Vec<Value>> {
    let total = entries.len();
    let mut results = Vec::new();
    for (idx, entry) in entries.iter().enumerate() {
        progress_cb(idx, total);
        let is_dat = entry.path.extension().and_then(|e| e.to_str()).unwrap_or("") == "dat";
        if is_dat {
            let data = match fs::read(&entry.path) {
                Ok(d) => d,
                Err(_) => continue,
            };
            let version = detect_dat_version(&data);
            if version == 0 {
                // not a weflow-format .dat; skip encrypted-but-untagged files
                continue;
            }
            match decrypt_file(&entry.path, xor_key, aes_key) {
                Ok(result) => {
                    if let Some(filter) = session_filter {
                        if !entry.relative.contains(filter) {
                            continue;
                        }
                    }
                    let stem = entry.path.file_stem().and_then(|s| s.to_str()).unwrap_or("file");
                    let out_name = format!("{}{}", stem, result.ext);
                    let out_path = out_dir.join(&out_name);
                    if let Some(parent) = out_path.parent() {
                        fs::create_dir_all(parent).ok();
                    }
                    if fs::write(&out_path, &result.data).is_ok() {
                        results.push(json!({ "src": entry.relative, "out": out_name, "ext": result.ext }));
                    }
                }
                Err(_) => continue,
            }
        } else {
            // plain image file: copy as-is
            if let Some(filter) = session_filter {
                if !entry.relative.contains(filter) {
                    continue;
                }
            }
            let file_name = entry.path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
            let out_path = out_dir.join(file_name);
            if let Some(parent) = out_path.parent() {
                fs::create_dir_all(parent).ok();
            }
            if fs::copy(&entry.path, &out_path).is_ok() {
                results.push(json!({ "src": entry.relative, "out": file_name }));
            }
        }
    }
    progress_cb(total, total);
    Ok(results)
}

pub fn export_voices(
    entries: &[VoiceEntry],
    out_dir: &Path,
    session_filter: Option<&str>,
    progress_cb: &dyn Fn(usize, usize),
) -> Result<Vec<Value>> {
    let filtered: Vec<&VoiceEntry> = entries
        .iter()
        .filter(|e| {
            session_filter
                .map(|f| e.session_id.contains(f) || e.relative.contains(f))
                .unwrap_or(true)
        })
        .collect();
    let total = filtered.len();
    let mut results = Vec::new();
    for (idx, entry) in filtered.iter().enumerate() {
        progress_cb(idx, total);
        let file_name = entry.path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
        let out_path = out_dir.join(&entry.session_id).join(file_name);
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).ok();
        }
        if fs::copy(&entry.path, &out_path).is_ok() {
            results.push(json!({
                "src": entry.relative,
                "out": format!("{}/{}", entry.session_id, file_name),
                "sessionId": entry.session_id
            }));
        }
    }
    progress_cb(total, total);
    Ok(results)
}

// ── Emoji ─────────────────────────────────────────────────────────────────────

pub fn extract_emoji_urls(messages: &Value) -> Vec<EmojiMeta> {
    let Some(items) = messages.as_array() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for msg in items {
        let msg_type = msg
            .get("type")
            .or_else(|| msg.get("msgType"))
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if msg_type != 47 {
            continue;
        }
        let content = msg
            .get("content")
            .or_else(|| msg.get("rawContent"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if content.is_empty() {
            continue;
        }
        // Try attribute-style: cdnurl="..."
        let cdn_url = extract_attr(content, "cdnurl")
            .or_else(|| extract_attr(content, "thumburl"))
            .or_else(|| {
                let v = extract_xml_value(content, "cdnurl");
                if v.is_empty() { None } else { Some(v) }
            })
            .unwrap_or_default();
        if cdn_url.is_empty() {
            continue;
        }
        let md5 = extract_attr(content, "md5")
            .or_else(|| {
                let v = extract_xml_value(content, "md5");
                if v.is_empty() { None } else { Some(v) }
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
) -> Result<Vec<Value>> {
    fs::create_dir_all(cache_dir)
        .with_context(|| format!("create emoji cache dir {}", cache_dir.display()))?;

    let client = reqwest::Client::builder()
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
            results.push(json!({ "md5": meta.md5, "cached": true, "path": p.to_string_lossy() }));
            continue;
        }
        match client.get(&meta.cdn_url).send().await {
            Ok(resp) if resp.status().is_success() => {
                let bytes = match resp.bytes().await {
                    Ok(b) => b,
                    Err(_) => continue,
                };
                let ext = guess_image_ext(&bytes);
                let out_path = cache_dir.join(format!("{}{}", meta.md5, ext));
                if fs::write(&out_path, &bytes).is_ok() {
                    results.push(json!({
                        "md5": meta.md5,
                        "cached": false,
                        "path": out_path.to_string_lossy(),
                        "size": bytes.len()
                    }));
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

fn guess_image_ext(bytes: &[u8]) -> &'static str {
    if bytes.len() >= 4 && bytes[..3] == [0xFF, 0xD8, 0xFF] {
        return ".jpg";
    }
    if bytes.len() >= 4 && bytes[..4] == [0x89, 0x50, 0x4E, 0x47] {
        return ".png";
    }
    if bytes.len() >= 6 && bytes[..6] == [0x47, 0x49, 0x46, 0x38, 0x39, 0x61] {
        return ".gif";
    }
    if bytes.len() >= 6 && bytes[..6] == [0x47, 0x49, 0x46, 0x38, 0x37, 0x61] {
        return ".gif";
    }
    if bytes.len() >= 12
        && bytes[..4] == [0x52, 0x49, 0x46, 0x46]
        && bytes[8..12] == [0x57, 0x45, 0x42, 0x50]
    {
        return ".webp";
    }
    ".bin"
}

fn extract_attr(xml: &str, attr: &str) -> Option<String> {
    let needle = format!("{}=\"", attr);
    let start = xml.find(&needle)? + needle.len();
    let end = xml[start..].find('"')? + start;
    let val = xml[start..end].trim().to_string();
    if val.is_empty() { None } else { Some(val) }
}

fn simple_hash(s: &str) -> String {
    let mut h: u64 = 14695981039346656037;
    for b in s.bytes() {
        h = h.wrapping_mul(1099511628211);
        h ^= b as u64;
    }
    format!("{h:016x}")
}

fn normalize_path(p: &Path) -> String {
    p.components()
        .map(|c| c.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use std::time::{SystemTime, UNIX_EPOCH};

    use serde_json::json;

    use super::*;

    fn temp_dir(prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn extract_emoji_urls_from_messages() {
        let xml = r#"<msg><emoji cdnurl="https://cdn.example.com/emoji.gif" md5="abc123def456" /></msg>"#;
        let messages = json!([
            { "type": 47, "content": xml },
            { "type": 1, "content": "hello" }
        ]);
        let metas = extract_emoji_urls(&messages);
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].md5, "abc123def456");
        assert!(metas[0].cdn_url.contains("cdn.example.com"));
    }

    #[test]
    fn scan_image_files_finds_files() {
        let dir = temp_dir("weflow-media-scan");
        let img_dir = dir.join("FileStorage/Image/2024-01");
        fs::create_dir_all(&img_dir).unwrap();
        fs::write(img_dir.join("pic.dat"), b"dummy").unwrap();
        fs::write(img_dir.join("pic_t.jpg"), b"thumb").unwrap();

        let entries = scan_image_files(&dir);
        assert_eq!(entries.len(), 2);
        let names: Vec<&str> = entries
            .iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap())
            .collect();
        assert!(names.contains(&"pic.dat"));
        assert!(names.contains(&"pic_t.jpg"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_voice_files_finds_slk() {
        let dir = temp_dir("weflow-voice-scan");
        let audio_dir = dir.join("FileStorage/Audio/wxid_abc");
        fs::create_dir_all(&audio_dir).unwrap();
        fs::write(audio_dir.join("voice.slk"), b"SILK").unwrap();
        fs::write(audio_dir.join("other.txt"), b"text").unwrap();

        let entries = scan_voice_files(&dir);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].session_id, "wxid_abc");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_emoji_no_emoji_type_skipped() {
        let messages = json!([{ "type": 1, "content": r#"<emoji cdnurl="https://x.com/e.gif" md5="aaa"/>"# }]);
        assert!(extract_emoji_urls(&messages).is_empty());
    }
}
