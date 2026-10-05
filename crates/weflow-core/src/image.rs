//! Pure helpers of `electron/services/imageDecryptService.ts`: `.dat` / cache file naming,
//! quality tiers, WXGF (HEVC) unwrapping and JPEG sanity checks.
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, TimeZone};

use crate::message::rx;

// ───────────────────────── naming ─────────────────────────

pub fn looks_like_md5(s: &str) -> bool {
    s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit())
}

fn file_name_lower(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
        .unwrap_or_default()
}

fn strip_dat_variant_suffix(base: &str) -> String {
    let lower = base.to_lowercase();
    for suffix in [
        "_thumb", ".thumb", "_hd", ".hd", "_h", ".h", "_b", ".b", "_w", ".w", "_t", ".t", "_c",
        ".c",
    ] {
        if lower.ends_with(suffix) {
            return lower[..lower.len() - suffix.len()].to_string();
        }
    }
    let b = lower.as_bytes();
    if b.len() >= 2
        && (b[b.len() - 2] == b'.' || b[b.len() - 2] == b'_')
        && b[b.len() - 1].is_ascii_lowercase()
    {
        return lower[..lower.len() - 2].to_string();
    }
    lower
}

/// `normalizeDatBase`: lower-cases, drops `.dat` / `.jpg` and every variant suffix.
pub fn normalize_dat_base(name: &str) -> String {
    let mut base = name.to_lowercase();
    if base.ends_with(".dat") || base.ends_with(".jpg") {
        base.truncate(base.len() - 4);
    }
    loop {
        let stripped = strip_dat_variant_suffix(&base);
        if stripped == base {
            return base;
        }
        base = stripped;
    }
}

pub fn is_thumbnail_dat(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains("_t.dat") || l.contains(".t.dat") || l.contains("_thumb.dat")
}

pub fn is_hd_dat_path(dat_path: &str) -> bool {
    let name = file_name_lower(dat_path);
    let Some(stem) = name.strip_suffix(".dat") else {
        return false;
    };
    stem.ends_with("_h") || stem.ends_with(".h") || stem.ends_with("_hd") || stem.ends_with(".hd")
}

pub fn is_t_variant_dat(dat_path: &str) -> bool {
    is_thumbnail_dat(&file_name_lower(dat_path))
}

pub fn is_base_dat_path(dat_path: &str, base_md5: &str) -> bool {
    let base = base_md5.trim().to_lowercase();
    !base.is_empty() && file_name_lower(dat_path) == format!("{base}.dat")
}

pub fn dat_tier(dat_path: &str, base_md5: &str) -> i32 {
    if is_hd_dat_path(dat_path) {
        3
    } else if is_base_dat_path(dat_path, base_md5) {
        2
    } else if is_t_variant_dat(dat_path) {
        1
    } else {
        0
    }
}

fn stem_and_ext(path: &str) -> (String, String) {
    let raw = path.split('?').next().unwrap_or("");
    let name = Path::new(raw)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    match name.rfind('.') {
        Some(i) if i > 0 => (name[..i].to_lowercase(), name[i..].to_lowercase()),
        _ => (name.to_lowercase(), String::new()),
    }
}

/// `isHdPath` on a decrypted cache file (`…_hd.jpg`).
pub fn is_hd_path(path: &str) -> bool {
    stem_and_ext(path).0.ends_with("_hd")
}

/// `isThumbnailPath` (a plain substring test, as on the desktop).
pub fn is_thumbnail_path(path: &str) -> bool {
    let l = path.to_lowercase();
    l.contains("_thumb") || l.contains("_t") || l.contains(".t.")
}

fn safe_suffix(raw_suffix: &str) -> String {
    let safe: String = raw_suffix
        .chars()
        .filter(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-'))
        .collect();
    if safe.is_empty() {
        String::new()
    } else if safe.starts_with('_') || safe.starts_with('.') {
        safe
    } else {
        format!("_{safe}")
    }
}

pub fn cache_variant_suffix_from_dat(dat_path: &str) -> String {
    if is_hd_dat_path(dat_path) {
        return "_hd".into();
    }
    let lower = file_name_lower(dat_path);
    let stem = lower.strip_suffix(".dat").unwrap_or(&lower).to_string();
    let base = normalize_dat_base(&stem);
    safe_suffix(&stem[base.len().min(stem.len())..])
}

pub fn cache_variant_suffix_from_cached(cache_path: &str) -> String {
    let (stem, _) = stem_and_ext(cache_path);
    let base = normalize_dat_base(&stem);
    safe_suffix(&stem[base.len().min(stem.len())..])
}

pub fn cached_path_tier(cache_path: &str) -> i32 {
    if is_hd_path(cache_path) {
        return 3;
    }
    if cache_variant_suffix_from_cached(cache_path).is_empty() {
        2
    } else {
        1
    }
}

fn suffix_search_order(primary: &str, prefer_hd: bool) -> Vec<String> {
    let fallback = [
        "_hd", "_thumb", "_t", ".t", "_b", ".b", "_w", ".w", "_c", ".c", "",
    ];
    let mut ordered: Vec<String> = if prefer_hd {
        vec!["_hd".into(), primary.into()]
    } else {
        vec![primary.into(), "_hd".into()]
    };
    ordered.extend(fallback.iter().map(|s| s.to_string()));
    let mut seen = std::collections::HashSet::new();
    ordered
        .into_iter()
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

pub fn sanitize_dir_name(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) { '_' } else { c })
        .collect();
    let t = cleaned.trim();
    if t.is_empty() {
        "unknown".into()
    } else {
        t.to_string()
    }
}

/// `resolveTimeDir`: `YYYY-MM` of the file's mtime (or now).
pub fn time_dir(path: &Path) -> String {
    let dt = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .map(chrono::DateTime::<Local>::from)
        .unwrap_or_else(Local::now);
    format!("{}-{:02}", dt.year(), dt.month())
}

/// `resolveYearMonthFromCreateTime`
pub fn year_month_from_create_time(create_time: Option<i64>) -> String {
    let Some(raw) = create_time.filter(|t| *t > 0) else {
        return String::new();
    };
    let ms = if raw as f64 > 1e12 { raw } else { raw * 1000 };
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => format!("{}-{:02}", d.year(), d.month()),
        None => String::new(),
    }
}

/// `resolveSessionDirForStorage`: WeChat stores chat attachments under `md5(username)`.
pub fn session_dir_for_storage(session_id: &str, clean_account: impl Fn(&str) -> String) -> String {
    use md5::{Digest, Md5};
    let normalized = session_id.trim().to_lowercase();
    if normalized.is_empty() {
        return String::new();
    }
    if looks_like_md5(&normalized) {
        return normalized;
    }
    let cleaned = clean_account(&normalized).to_lowercase();
    if looks_like_md5(&cleaned) {
        return cleaned;
    }
    let src = if cleaned.is_empty() {
        &normalized
    } else {
        &cleaned
    };
    Md5::digest(src.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

pub fn is_hardlink_candidate_name(file_name: &str, base_md5: &str) -> bool {
    let lower = file_name.trim().to_lowercase();
    let Some(base) = lower.strip_suffix(".dat") else {
        return false;
    };
    if base == base_md5 {
        return true;
    }
    if base.starts_with(&format!("{base_md5}_")) || base.starts_with(&format!("{base_md5}.")) {
        return true;
    }
    if base.len() == base_md5.len() + 1 && base.starts_with(base_md5) {
        return true;
    }
    normalize_dat_base(base) == base_md5
}

pub fn is_img_scoped_dat_path(path: &str) -> bool {
    rx(r"[\\/](img|image|msgimg)[\\/]").is_match(&path.to_lowercase())
}

// ───────────────────────── cache layout ─────────────────────────

pub const CACHE_EXTENSIONS: [&str; 5] = [".jpg", ".jpeg", ".png", ".gif", ".webp"];

pub fn cache_output_path(
    root: &Path,
    dat_path: &str,
    ext: &str,
    session_id: Option<&str>,
) -> PathBuf {
    let lower = file_name_lower(dat_path);
    let base = lower.strip_suffix(".dat").unwrap_or(&lower).to_string();
    let normalized = normalize_dat_base(&base);
    let suffix = cache_variant_suffix_from_dat(dat_path);
    root.join(sanitize_dir_name(session_id.unwrap_or("unknown")))
        .join(time_dir(Path::new(dat_path)))
        .join(format!("{normalized}{suffix}{ext}"))
}

pub fn cache_output_candidates(
    root: &Path,
    dat_path: &str,
    session_id: Option<&str>,
    prefer_hd: bool,
) -> Vec<PathBuf> {
    let lower = file_name_lower(dat_path);
    let base = lower.strip_suffix(".dat").unwrap_or(&lower).to_string();
    let normalized = normalize_dat_base(&base);
    let primary = cache_variant_suffix_from_dat(dat_path);
    let suffixes = suffix_search_order(&primary, prefer_hd);
    let current = root
        .join(sanitize_dir_name(session_id.unwrap_or("unknown")))
        .join(time_dir(Path::new(dat_path)));
    let legacy = root.join(&normalized);
    let mut out = Vec::new();
    for dir in [&current, &legacy] {
        for suffix in &suffixes {
            for ext in CACHE_EXTENSIONS {
                out.push(dir.join(format!("{normalized}{suffix}{ext}")));
            }
        }
    }
    for ext in CACHE_EXTENSIONS {
        out.push(root.join(format!("{normalized}{ext}")));
        out.push(root.join(format!("{normalized}_t{ext}")));
        out.push(root.join(format!("{normalized}_hd{ext}")));
    }
    out
}

pub fn is_image_file(path: &str) -> bool {
    matches!(
        stem_and_ext(path).1.as_str(),
        ".gif" | ".png" | ".jpg" | ".jpeg" | ".webp"
    )
}

pub fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        ".gif" => Some("image/gif"),
        ".png" => Some("image/png"),
        ".jpg" | ".jpeg" => Some("image/jpeg"),
        ".webp" => Some("image/webp"),
        _ => None,
    }
}

/// `detectImageExtension` of the service (needs 12 bytes, knows four formats).
pub fn detect_image_extension(b: &[u8]) -> Option<&'static str> {
    if b.len() < 12 {
        return None;
    }
    if b[0] == 0x47 && b[1] == 0x49 && b[2] == 0x46 {
        return Some(".gif");
    }
    if b[0] == 0x89 && b[1] == 0x50 && b[2] == 0x4e && b[3] == 0x47 {
        return Some(".png");
    }
    if b[0] == 0xff && b[1] == 0xd8 && b[2] == 0xff {
        return Some(".jpg");
    }
    if &b[0..4] == b"RIFF" && &b[8..12] == b"WEBP" {
        return Some(".webp");
    }
    None
}

// ───────────────────────── JPEG sanity ─────────────────────────

/// `isLikelyCorruptedJpegBuffer`: an encoder that was fed garbage emits an almost all-zero scan.
pub fn is_likely_corrupted_jpeg(data: &[u8]) -> bool {
    if data.len() < 4096 {
        return false;
    }
    let zero_ratio = data.iter().filter(|b| **b == 0).count() as f64 / data.len() as f64;
    if zero_ratio >= 0.985 {
        return true;
    }
    let has_lavc = data.len() >= 24 && data[..24].windows(4).any(|w| w == b"Lavc");
    if !has_lavc {
        return false;
    }
    let Some(sos) = (2..data.len() - 1).find(|&i| data[i] == 0xff && data[i + 1] == 0xda) else {
        return zero_ratio >= 0.95;
    };
    if sos + 4 >= data.len() {
        return zero_ratio >= 0.95;
    }
    let sos_len = ((data[sos + 2] as usize) << 8) | data[sos + 3] as usize;
    let scan_start = sos + 2 + sos_len;
    if scan_start + 2 >= data.len() {
        return zero_ratio >= 0.95;
    }
    let Some(eoi) = (scan_start..=data.len() - 2)
        .rev()
        .find(|&i| data[i] == 0xff && data[i + 1] == 0xd9)
    else {
        return zero_ratio >= 0.95;
    };
    if eoi <= scan_start {
        return zero_ratio >= 0.95;
    }
    let scan = &data[scan_start..eoi];
    if scan.len() < 1024 {
        return zero_ratio >= 0.95;
    }
    scan.iter().filter(|b| **b == 0).count() as f64 / scan.len() as f64 >= 0.985
}

// ───────────────────────── WXGF ─────────────────────────

fn extract_hevc_nalus(buf: &[u8]) -> Vec<&[u8]> {
    let mut starts: Vec<(usize, usize)> = Vec::new(); // (start, prefix length)
    let mut i = 4usize;
    while i + 3 < buf.len() {
        let p4 = buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 0 && buf[i + 3] == 1;
        let p3 = buf[i] == 0 && buf[i + 1] == 0 && buf[i + 2] == 1;
        if p4 || p3 {
            let len = if p4 { 4 } else { 3 };
            starts.push((i, len));
            i += len;
            continue;
        }
        i += 1;
    }
    let mut units = Vec::new();
    for (idx, (start, plen)) in starts.iter().enumerate() {
        let end = starts.get(idx + 1).map(|s| s.0).unwrap_or(buf.len());
        let payload_start = start + plen;
        if payload_start >= end {
            continue;
        }
        let payload = &buf[payload_start..end];
        if payload.len() < 2 || payload[0] & 0x80 != 0 {
            continue;
        }
        units.push(payload);
    }
    units
}

fn merge_nalus(units: &[&[u8]]) -> Vec<u8> {
    let mut out = Vec::new();
    for u in units {
        if u.len() < 2 {
            continue;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(u);
    }
    out
}

/// `buildWxgfHevcCandidates`: VPS-delimited groups (largest first), all NAL units, raw payload.
pub fn wxgf_hevc_candidates(buf: &[u8]) -> Vec<(String, Vec<u8>)> {
    let units = extract_hevc_nalus(buf);
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    let mut add = |name: String, data: Vec<u8>| {
        if data.len() < 100 || out.iter().any(|(_, d)| *d == data) {
            return;
        }
        out.push((name, data));
    };
    let nal_type = |u: &[u8]| (u[0] >> 1) & 0x3f;
    let vps: Vec<usize> = units
        .iter()
        .enumerate()
        .filter(|(_, u)| u.len() >= 2 && nal_type(u) == 32)
        .map(|(i, _)| i)
        .collect();
    let mut groups: Vec<(usize, Vec<u8>)> = Vec::new();
    for (gi, &start) in vps.iter().enumerate() {
        let end = vps.get(gi + 1).copied().unwrap_or(units.len());
        let group = &units[start..end];
        if !group
            .iter()
            .any(|u| u.len() >= 2 && matches!(nal_type(u), 19 | 20 | 1))
        {
            continue;
        }
        groups.push((gi, merge_nalus(group)));
    }
    groups.sort_by_key(|a| std::cmp::Reverse(a.1.len()));
    for (gi, data) in groups {
        add(format!("group_{gi}"), data);
    }
    add("scan_all_nalus".into(), merge_nalus(&units));
    add("raw_skip4".into(), buf[4.min(buf.len())..].to_vec());
    out
}

/// Embedded JPEG/PNG inside the first 4 KiB of a WXGF blob.
pub fn wxgf_embedded_image(buf: &[u8]) -> Option<&[u8]> {
    if buf.len() < 20 || &buf[0..4] != b"wxgf" {
        return None;
    }
    let limit = (buf.len() - 12).min(4096);
    for i in 4..limit {
        if buf[i] == 0xff && buf[i + 1] == 0xd8 && buf[i + 2] == 0xff {
            return Some(&buf[i..]);
        }
        if buf[i] == 0x89 && buf[i + 1] == 0x50 && buf[i + 2] == 0x4e && buf[i + 3] == 0x47 {
            return Some(&buf[i..]);
        }
    }
    None
}

pub fn is_wxgf(buf: &[u8]) -> bool {
    buf.len() >= 20 && &buf[0..4] == b"wxgf"
}

fn ffmpeg_binary() -> String {
    std::env::var("FFMPEG_PATH")
        .ok()
        .filter(|p| !p.trim().is_empty())
        .unwrap_or_else(|| "ffmpeg".into())
}

/// `convertHevcToJpg`: nine ffmpeg attempts (hevc / h265 / autodetect × frame 0, 1, 5).
pub fn convert_hevc_to_jpg(hevc: &[u8]) -> Option<Vec<u8>> {
    use std::process::{Command, Stdio};
    let dir = std::env::temp_dir().join("weflow_hevc");
    std::fs::create_dir_all(&dir).ok()?;
    let uid = format!(
        "{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    );
    let input = dir.join(format!("hevc_{uid}.hevc"));
    let output = dir.join(format!("hevc_{uid}.jpg"));
    std::fs::write(&input, hevc).ok()?;
    let ffmpeg = ffmpeg_binary();
    let mut result = None;
    'attempts: for fmt in [Some("hevc"), Some("h265"), None] {
        for frame in [None, Some(1), Some(5)] {
            let _ = std::fs::remove_file(&output);
            let mut cmd = Command::new(&ffmpeg);
            cmd.args(["-hide_banner", "-loglevel", "error", "-y"]);
            if let Some(f) = fmt {
                cmd.args(["-f", f]);
            }
            cmd.arg("-i").arg(&input);
            if let Some(n) = frame {
                cmd.args(["-vf", &format!("select=eq(n\\,{n})")]);
            }
            cmd.args(["-vframes", "1", "-q:v", "2", "-f", "image2"])
                .arg(&output)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            let Ok(mut child) = cmd.spawn() else {
                break 'attempts;
            };
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            let status = loop {
                match child.try_wait() {
                    Ok(Some(s)) => break Some(s),
                    Ok(None) if std::time::Instant::now() < deadline => {
                        std::thread::sleep(std::time::Duration::from_millis(2))
                    }
                    _ => {
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                }
            };
            if status.is_some_and(|s| s.success()) {
                if let Ok(buf) = std::fs::read(&output) {
                    if !buf.is_empty() && !is_likely_corrupted_jpeg(&buf) {
                        result = Some(buf);
                        break 'attempts;
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_file(&input);
    let _ = std::fs::remove_file(&output);
    result
}

/// `unwrapWxgf`: returns the image bytes and whether the data is still an undecoded WXGF blob.
pub fn unwrap_wxgf(buf: Vec<u8>) -> (Vec<u8>, bool) {
    if !is_wxgf(&buf) {
        return (buf, false);
    }
    if let Some(inner) = wxgf_embedded_image(&buf) {
        return (inner.to_vec(), false);
    }
    let candidates = wxgf_hevc_candidates(&buf);
    for (_, data) in &candidates {
        if let Some(jpg) = convert_hevc_to_jpg(data).filter(|j| !j.is_empty()) {
            return (jpg, false);
        }
    }
    let fallback = candidates
        .first()
        .map(|(_, d)| d.clone())
        .unwrap_or_else(|| buf[4..].to_vec());
    (fallback, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MD5: &str = "0123456789abcdef0123456789abcdef";

    #[test]
    fn dat_names_normalize_like_the_desktop_app() {
        assert_eq!(normalize_dat_base(&format!("{MD5}_h.dat")), MD5);
        assert_eq!(normalize_dat_base(&format!("{MD5}_t")), MD5);
        assert_eq!(normalize_dat_base(&format!("{MD5}.thumb.DAT")), MD5);
        assert_eq!(
            normalize_dat_base(&format!("{MD5}_hd_t")),
            MD5,
            "suffixes are stripped repeatedly"
        );
        assert_eq!(
            normalize_dat_base(&format!("{MD5}_x")),
            MD5,
            "any single trailing letter variant"
        );
        assert!(is_hd_dat_path(&format!("/a/{MD5}_h.dat")));
        assert!(is_hd_dat_path(&format!("/a/{MD5}.hd.dat")));
        assert!(!is_hd_dat_path(&format!("/a/{MD5}_t.dat")));
        assert!(is_t_variant_dat(&format!("/a/{MD5}_t.dat")));
        assert!(is_base_dat_path(&format!("/a/{MD5}.dat"), MD5));
        assert_eq!(dat_tier(&format!("{MD5}_h.dat"), MD5), 3);
        assert_eq!(dat_tier(&format!("{MD5}.dat"), MD5), 2);
        assert_eq!(dat_tier(&format!("{MD5}_t.dat"), MD5), 1);
        assert_eq!(dat_tier(&format!("{MD5}_b.dat"), MD5), 0);
    }

    #[test]
    fn hardlink_candidate_names() {
        for ok in [
            format!("{MD5}.dat"),
            format!("{MD5}_h.dat"),
            format!("{MD5}.t.dat"),
            format!("{MD5}x.dat"),
            format!("{}.DAT", MD5.to_uppercase()),
        ] {
            assert!(is_hardlink_candidate_name(&ok, MD5), "{ok}");
        }
        for bad in [
            format!("{MD5}.jpg"),
            "other.dat".to_string(),
            format!("{}f.txt", MD5),
        ] {
            assert!(!is_hardlink_candidate_name(&bad, MD5), "{bad}");
        }
    }

    #[test]
    fn cache_suffixes_and_tiers() {
        assert_eq!(
            cache_variant_suffix_from_dat(&format!("/x/{MD5}_h.dat")),
            "_hd"
        );
        assert_eq!(
            cache_variant_suffix_from_dat(&format!("/x/{MD5}_t.dat")),
            "_t"
        );
        assert_eq!(cache_variant_suffix_from_dat(&format!("/x/{MD5}.dat")), "");
        assert_eq!(
            cache_variant_suffix_from_dat(&format!("/x/{MD5}.t.dat")),
            ".t"
        );
        assert_eq!(
            cache_variant_suffix_from_cached(&format!("/c/{MD5}_hd.jpg?v=1")),
            "_hd"
        );
        assert_eq!(cached_path_tier(&format!("/c/{MD5}_hd.jpg")), 3);
        assert_eq!(cached_path_tier(&format!("/c/{MD5}.jpg")), 2);
        assert_eq!(cached_path_tier(&format!("/c/{MD5}_t.jpg")), 1);
        assert!(is_hd_path(&format!("/c/{MD5}_hd.png")));
        assert!(!is_hd_path(&format!("/c/{MD5}.png")));
        assert_eq!(
            suffix_search_order("_t", false)[..3],
            ["_t", "_hd", "_thumb"]
        );
        assert_eq!(
            suffix_search_order("_t", true)[..3],
            ["_hd", "_t", "_thumb"]
        );
        assert_eq!(suffix_search_order("", false)[0], "");
    }

    #[test]
    fn candidate_list_covers_current_legacy_and_flat_layouts() {
        let root = Path::new("/cache/Images");
        let c = cache_output_candidates(
            root,
            &format!("/nonexistent/{MD5}_h.dat"),
            Some("wxid:bob"),
            true,
        );
        assert_eq!(
            c[0].file_name().unwrap().to_string_lossy(),
            format!("{MD5}_hd.jpg")
        );
        assert!(c[0].starts_with("/cache/Images/wxid_bob"), "{:?}", c[0]);
        assert!(c
            .iter()
            .any(|p| p == &root.join(MD5).join(format!("{MD5}_hd.webp"))));
        assert!(c.iter().any(|p| p == &root.join(format!("{MD5}_t.gif"))));
        let out = cache_output_path(
            root,
            &format!("/nonexistent/{MD5}_t.dat"),
            ".png",
            Some("a/b"),
        );
        let out = out.to_string_lossy().replace('\\', "/");
        assert!(out.contains("/a_b/"));
        assert!(out.ends_with(&format!("{MD5}_t.png")));
    }

    #[test]
    fn session_dirs_are_md5_of_the_username() {
        let clean = |s: &str| s.to_string();
        assert_eq!(
            session_dir_for_storage("WXID_bob", clean),
            "8a7b11f2fd24e19a60664a5fe5d56342"
        );
        assert_eq!(session_dir_for_storage(MD5, clean), MD5);
        assert_eq!(session_dir_for_storage("", clean), "");
    }

    #[test]
    fn year_month_uses_seconds_or_milliseconds() {
        let a = year_month_from_create_time(Some(1_700_000_000));
        let b = year_month_from_create_time(Some(1_700_000_000_000));
        assert_eq!(a, b);
        assert!(a.starts_with("2023-1"));
        assert_eq!(year_month_from_create_time(None), "");
        assert_eq!(year_month_from_create_time(Some(0)), "");
    }

    #[test]
    fn image_signatures() {
        let mut jpg = vec![0xff, 0xd8, 0xff, 0xe0];
        jpg.extend([0u8; 12]);
        assert_eq!(detect_image_extension(&jpg), Some(".jpg"));
        assert_eq!(detect_image_extension(&jpg[..8]), None, "needs 12 bytes");
        let mut webp = b"RIFF\0\0\0\0WEBP".to_vec();
        webp.push(0);
        assert_eq!(detect_image_extension(&webp), Some(".webp"));
        assert_eq!(detect_image_extension(b"wxgf\0\0\0\0\0\0\0\0"), None);
    }

    #[test]
    fn corrupted_jpegs_are_mostly_zero() {
        let mut good = vec![0xff, 0xd8, 0xff, 0xe0];
        good.extend((0..6000).map(|i| (i % 251 + 1) as u8));
        assert!(!is_likely_corrupted_jpeg(&good));
        let mut bad = vec![0xff, 0xd8, 0xff, 0xe0];
        bad.extend(vec![0u8; 6000]);
        assert!(is_likely_corrupted_jpeg(&bad));
        assert!(
            !is_likely_corrupted_jpeg(&bad[..100]),
            "small files are never flagged"
        );
    }

    #[test]
    fn wxgf_with_an_embedded_jpeg_is_unwrapped_without_ffmpeg() {
        let mut blob = b"wxgf".to_vec();
        blob.extend([1u8; 20]);
        blob.extend([0xff, 0xd8, 0xff, 0xe0, 9, 9, 9, 9, 9, 9, 9, 9]);
        blob.extend([7u8; 32]);
        let (out, still) = unwrap_wxgf(blob);
        assert!(!still);
        assert_eq!(&out[..3], &[0xff, 0xd8, 0xff]);
        let (plain, flag) = unwrap_wxgf(vec![1, 2, 3]);
        assert_eq!(plain, vec![1, 2, 3]);
        assert!(!flag);
    }

    #[test]
    fn hevc_candidates_group_by_vps() {
        // wxgf header, then two VPS-delimited groups; the second is larger.
        let mut blob = b"wxgf".to_vec();
        blob.extend([0xAAu8; 8]); // container bytes before the first start code
        let nal = |t: u8, len: usize| -> Vec<u8> {
            let mut v = vec![0, 0, 0, 1, t << 1, 1];
            v.extend(vec![0x11; len]);
            v
        };
        blob.extend(nal(32, 4));
        blob.extend(nal(19, 40));
        blob.extend(nal(32, 4));
        blob.extend(nal(19, 200));
        let c = wxgf_hevc_candidates(&blob);
        assert_eq!(c[0].0, "group_1", "largest group first");
        assert!(c.iter().any(|(n, _)| n == "scan_all_nalus"));
        assert!(c.iter().any(|(n, _)| n == "raw_skip4"));
        assert!(c.iter().all(|(_, d)| d.len() >= 100));
    }
}
