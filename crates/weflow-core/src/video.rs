//! Port of `electron/services/videoService.ts`: locating decrypted-by-WeChat videos on disk
//! (`<account>/msg/video/<yyyy-mm>/<md5>.mp4`) plus their cover / thumbnail images, and
//! extracting the video md5 out of a message's XML.
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use base64::Engine;
use serde_json::{json, Map, Value};

use crate::chat_msg::normalize_video_file_token;
use crate::message::rx;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PosterFormat {
    DataUrl,
    FileUrl,
}

impl PosterFormat {
    pub fn parse(s: &str) -> Self {
        if s == "fileUrl" {
            Self::FileUrl
        } else {
            Self::DataUrl
        }
    }
}

#[derive(Debug, Clone, Default)]
struct IndexEntry {
    video: Option<PathBuf>,
    cover: Option<PathBuf>,
    thumb: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct VideoInfo {
    pub video_url: Option<String>,
    pub cover_url: Option<String>,
    pub thumb_url: Option<String>,
    pub exists: bool,
}

impl VideoInfo {
    /// Same key order / omitted-undefined behaviour as the desktop `VideoInfo` object.
    pub fn to_json(&self) -> Value {
        let mut o = Map::new();
        if let Some(v) = &self.video_url {
            o.insert("videoUrl".into(), json!(v));
        }
        if let Some(v) = &self.cover_url {
            o.insert("coverUrl".into(), json!(v));
        }
        if let Some(v) = &self.thumb_url {
            o.insert("thumbUrl".into(), json!(v));
        }
        o.insert("exists".into(), json!(self.exists));
        Value::Object(o)
    }
}

/// Year-month directories, newest first (`b.localeCompare(a)` on ASCII names).
fn year_month_dirs(base: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(base)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default();
    dirs.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    dirs
}

fn build_index(base: &Path) -> HashMap<String, IndexEntry> {
    let mut index: HashMap<String, IndexEntry> = HashMap::new();
    for dir in year_month_dirs(base) {
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for full in files {
            let Some(name) = full.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            let lower = name.to_lowercase();
            if let Some(md5) = lower.strip_suffix(".mp4") {
                let e = index.entry(md5.to_string()).or_default();
                if e.video.is_none() {
                    e.video = Some(full.clone());
                }
                if let Some(base_md5) = md5.strip_suffix("_raw") {
                    let be = index.entry(base_md5.to_string()).or_default();
                    if be.video.is_none() {
                        be.video = Some(full.clone());
                    }
                }
                continue;
            }
            let Some(jpg) = lower.strip_suffix(".jpg") else {
                continue;
            };
            if let Some(base_md5) = jpg.strip_suffix("_thumb") {
                let e = index.entry(base_md5.to_string()).or_default();
                if e.thumb.is_none() {
                    e.thumb = Some(full.clone());
                }
            } else {
                let e = index.entry(jpg.to_string()).or_default();
                if e.cover.is_none() {
                    e.cover = Some(full.clone());
                }
            }
        }
    }
    let raw_keys: Vec<String> = index
        .keys()
        .filter(|k| k.ends_with("_raw"))
        .cloned()
        .collect();
    for key in raw_keys {
        let base_key = key.trim_end_matches("_raw").to_string();
        let Some(base_entry) = index.get(&base_key).cloned() else {
            continue;
        };
        let e = index.get_mut(&key).unwrap();
        if e.cover.is_none() {
            e.cover = base_entry.cover;
        }
        if e.thumb.is_none() {
            e.thumb = base_entry.thumb;
        }
    }
    index
}

/// `pathToFileURL(path).toString()`
pub fn path_to_file_url(path: &Path) -> String {
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    let text = abs.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file://");
    if !text.starts_with('/') {
        out.push('/');
    }
    for b in text.bytes() {
        match b {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'.'
            | b'_'
            | b'~'
            | b'!'
            | b'$'
            | b'&'
            | b'\''
            | b'('
            | b')'
            | b'*'
            | b'+'
            | b','
            | b';'
            | b'='
            | b':'
            | b'@'
            | b'/' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn poster_url(path: Option<&Path>, mime: &str, format: PosterFormat) -> Option<String> {
    let path = path.filter(|p| p.exists())?;
    match format {
        PosterFormat::FileUrl => Some(path_to_file_url(path)),
        PosterFormat::DataUrl => {
            let bytes = std::fs::read(path).ok()?;
            Some(format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(bytes)
            ))
        }
    }
}

fn info_for(
    video: &Path,
    cover: Option<&Path>,
    thumb: Option<&Path>,
    include_poster: bool,
    format: PosterFormat,
) -> VideoInfo {
    let mut info = VideoInfo {
        video_url: Some(video.to_string_lossy().to_string()),
        exists: true,
        ..Default::default()
    };
    if include_poster {
        info.cover_url = poster_url(cover, "image/jpeg", format);
        info.thumb_url = poster_url(thumb, "image/jpeg", format);
    }
    info
}

/// `getVideoInfo`: index lookup (md5, then the `_raw` twin), then a directory scan.
pub fn video_info(
    video_base_dir: &Path,
    md5: &str,
    include_poster: bool,
    format: PosterFormat,
) -> VideoInfo {
    let normalized = md5.trim().to_lowercase();
    if normalized.is_empty() || !video_base_dir.exists() {
        return VideoInfo::default();
    }
    let real = normalize_video_file_token(&normalized).unwrap_or_else(|| normalized.clone());
    let index = build_index(video_base_dir);
    let mut candidates = vec![real.clone()];
    match real.strip_suffix("_raw") {
        Some(base) => candidates.push(base.to_string()),
        None => candidates.push(format!("{real}_raw")),
    }
    for key in &candidates {
        let Some(entry) = index.get(key) else {
            continue;
        };
        let Some(video) = entry.video.as_ref().filter(|p| p.exists()) else {
            continue;
        };
        return info_for(
            video,
            entry.cover.as_deref(),
            entry.thumb.as_deref(),
            include_poster,
            format,
        );
    }
    // fallback scan
    for dir in year_month_dirs(video_base_dir) {
        let video = dir.join(format!("{real}.mp4"));
        if !video.exists() {
            continue;
        }
        let base = real.trim_end_matches("_raw");
        return info_for(
            &video,
            Some(&dir.join(format!("{base}.jpg"))),
            Some(&dir.join(format!("{base}_thumb.jpg"))),
            include_poster,
            format,
        );
    }
    VideoInfo::default()
}

/// `parseVideoMd5`: five strategies, in the desktop app's order.
pub fn parse_video_md5(content: &str) -> Option<String> {
    if content.is_empty() {
        return None;
    }
    let lower = |c: &regex::Captures| c[1].to_lowercase();
    if let Some(c) = rx(r#"(?i)<videomsg[^>]*\smd5\s*=\s*['"]([a-fA-F0-9]+)['"]"#).captures(content)
    {
        return Some(lower(&c));
    }
    if let Some(c) =
        rx(r#"(?i)<videomsg[^>]*\srawmd5\s*=\s*['"]([a-fA-F0-9]+)['"]"#).captures(content)
    {
        return Some(lower(&c));
    }
    // `(?<![a-z])md5\s*=` without lookbehind: require a non-letter (or start) before it
    if let Some(c) = rx(r#"(?i)(?:^|[^a-z])md5\s*=\s*['"]([a-fA-F0-9]+)['"]"#).captures(content) {
        return Some(lower(&c));
    }
    if let Some(c) = rx(r"(?i)<md5>([a-fA-F0-9]+)</md5>").captures(content) {
        return Some(lower(&c));
    }
    if let Some(c) = rx(r#"(?i)\srawmd5\s*=\s*['"]([a-fA-F0-9]+)['"]"#).captures(content) {
        return Some(lower(&c));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("weflow-video-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn md5_strategies_follow_the_desktop_priority() {
        assert_eq!(
            parse_video_md5(r#"<msg><videomsg length="1" md5="ABCD12" rawmd5="ffff"/></msg>"#)
                .as_deref(),
            Some("abcd12")
        );
        assert_eq!(
            parse_video_md5(r#"<videomsg length="1" rawmd5="EEee"/>"#).as_deref(),
            Some("eeee")
        );
        assert_eq!(
            parse_video_md5(r#"<x md5='aa11'/>"#).as_deref(),
            Some("aa11")
        );
        assert_eq!(parse_video_md5("<md5>ab12</md5>").as_deref(), Some("ab12"));
        assert_eq!(
            parse_video_md5(r#"<x newmd5="zz" rawmd5="beef"/>"#).as_deref(),
            Some("beef")
        );
        assert_eq!(
            parse_video_md5(r#"<x cdnthumbmd5="abcd"/>"#),
            None,
            "a longer attribute name never matches the bare md5 rule"
        );
        assert_eq!(parse_video_md5(""), None);
    }

    #[test]
    fn index_prefers_newest_month_and_links_raw_twins() {
        let base = tmp("idx");
        std::fs::create_dir_all(base.join("2024-01")).unwrap();
        std::fs::create_dir_all(base.join("2024-02")).unwrap();
        let md5 = "0123456789abcdef0123456789abcdef";
        std::fs::write(base.join("2024-01").join(format!("{md5}.mp4")), b"old").unwrap();
        std::fs::write(base.join("2024-02").join(format!("{md5}.mp4")), b"new").unwrap();
        std::fs::write(
            base.join("2024-02").join(format!("{md5}.jpg")),
            b"\xff\xd8cover",
        )
        .unwrap();
        std::fs::write(
            base.join("2024-02").join(format!("{md5}_thumb.jpg")),
            b"\xff\xd8thumb",
        )
        .unwrap();

        let info = video_info(&base, md5, true, PosterFormat::DataUrl);
        assert!(info.exists);
        assert!(info.video_url.as_deref().unwrap().contains("2024-02"));
        assert!(info
            .cover_url
            .as_deref()
            .unwrap()
            .starts_with("data:image/jpeg;base64,"));
        assert!(info.thumb_url.is_some());

        let info = video_info(&base, &md5.to_uppercase(), false, PosterFormat::DataUrl);
        assert!(info.exists && info.cover_url.is_none() && info.thumb_url.is_none());
        let json = info.to_json();
        assert!(json.get("coverUrl").is_none());
        assert_eq!(json["exists"], true);

        // `_raw` lookups resolve to the base file and vice versa
        let raw = format!("{md5}_raw");
        assert!(video_info(&base, &raw, true, PosterFormat::FileUrl).exists);
        let fu = video_info(&base, md5, true, PosterFormat::FileUrl);
        assert!(fu.cover_url.unwrap().starts_with("file:///"));

        assert!(
            !video_info(
                &base,
                "ffffffffffffffffffffffffffffffff",
                true,
                PosterFormat::DataUrl
            )
            .exists
        );
        assert!(!video_info(&base.join("missing"), md5, true, PosterFormat::DataUrl).exists);
    }

    #[test]
    fn file_urls_percent_encode_like_node() {
        // On Windows a rooted path gets the current drive ("file:///D:/a%20b/...").
        let url = path_to_file_url(Path::new("/a b/c#d/é.mp4"));
        assert!(url.starts_with("file:///"), "{url}");
        assert!(url.ends_with("/a%20b/c%23d/%C3%A9.mp4"), "{url}");
    }
}
