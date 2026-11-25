//! What finding and decrypting message images needs besides the files: `.dat` / cache file naming,
//! quality tiers, WXGF (HEVC) unwrapping and JPEG sanity checks.
use std::path::{Path, PathBuf};

use chrono::{Datelike, Local, TimeZone};

use crate::message::xml::rx;
use crate::num::count_f64;

pub mod ending;

// ───────────────────────── naming ─────────────────────────

/// Whether `s` is 32 hex digits.
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

/// The base name of an image file: lower-cased, without `.dat` / `.jpg` and every variant suffix.
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

fn is_thumbnail_dat(name: &str) -> bool {
    let l = name.to_lowercase();
    l.contains("_t.dat") || l.contains(".t.dat") || l.contains("_thumb.dat")
}

/// Whether the `.dat` file is an original WeChat downloaded: its name ends in `_h`, `.h`, `_hd` or `.hd`.
pub fn is_hd_dat_path(dat_path: &str) -> bool {
    let name = file_name_lower(dat_path);
    let Some(stem) = name.strip_suffix(".dat") else {
        return false;
    };
    stem.ends_with("_h") || stem.ends_with(".h") || stem.ends_with("_hd") || stem.ends_with(".hd")
}

/// Whether the `.dat` file is a thumbnail: `_t.dat`, `.t.dat` or `_thumb.dat`.
pub fn is_t_variant_dat(dat_path: &str) -> bool {
    is_thumbnail_dat(&file_name_lower(dat_path))
}

/// Whether the `.dat` file is the image of `base_md5` itself, `<md5>.dat`.
pub fn is_base_dat_path(dat_path: &str, base_md5: &str) -> bool {
    let base = base_md5.trim().to_lowercase();
    !base.is_empty() && file_name_lower(dat_path) == format!("{base}.dat")
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

/// Whether a decrypted cache file is the HD rendition (`…_hd.jpg`).
pub fn is_hd_path(path: &str) -> bool {
    stem_and_ext(path).0.ends_with("_hd")
}

/// Whether `path` names a thumbnail, by a substring test of the file name.
///
/// The test is the one of WeFlow's desktop app. The folders are not looked at: a conversation or a folder named like
/// `wxid_t…` or `my_temp` does not make every image in it a thumbnail.
pub fn is_thumbnail_path(path: &str) -> bool {
    let l = file_name_lower(path);
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

fn cache_variant_suffix_from_dat(dat_path: &str) -> String {
    if is_hd_dat_path(dat_path) {
        return "_hd".into();
    }
    let lower = file_name_lower(dat_path);
    let stem = lower.strip_suffix(".dat").unwrap_or(&lower).to_string();
    let base = normalize_dat_base(&stem);
    safe_suffix(&stem[base.len().min(stem.len())..])
}

fn suffix_search_order(primary: &str, prefer_hd: bool) -> Vec<&str> {
    const FALLBACK: [&str; 11] = [
        "_hd", "_thumb", "_t", ".t", "_b", ".b", "_w", ".w", "_c", ".c", "",
    ];
    let first = if prefer_hd {
        ["_hd", primary]
    } else {
        [primary, "_hd"]
    };
    let mut ordered = Vec::with_capacity(first.len() + FALLBACK.len());
    for suffix in first.into_iter().chain(FALLBACK) {
        if !ordered.contains(&suffix) {
            ordered.push(suffix);
        }
    }
    ordered
}

fn sanitize_dir_name(s: &str) -> String {
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

/// The cache folder of a `.dat` file modified at `modified`: `YYYY-MM` of that time in local time, else of now.
pub fn month_folder(modified: Option<std::time::SystemTime>) -> String {
    let dt = modified
        .map(chrono::DateTime::<Local>::from)
        .unwrap_or_else(Local::now);
    format!("{}-{:02}", dt.year(), dt.month())
}

/// `YYYY-MM` of a message's create time in local time, else of now.
pub fn year_month_from_create_time(create_time: Option<i64>) -> String {
    let Some(raw) = create_time.filter(|t| *t > 0) else {
        return String::new();
    };
    let ms = if raw > 1_000_000_000_000 {
        raw
    } else {
        raw * 1000
    };
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => format!("{}-{:02}", d.year(), d.month()),
        None => String::new(),
    }
}

/// The attachment folder of a conversation: WeChat stores chat attachments under `md5(username)`, the username lower-cased.
/// The first of [`session_dirs_for_storage`]; empty for an empty username.
pub fn session_dir_for_storage(session_id: &str) -> String {
    session_dirs_for_storage(session_id)
        .into_iter()
        .next()
        .unwrap_or_default()
}

/// The folders under `msg/attach` a conversation's attachments may be in, the likelier first (#657).
///
/// The first is the `md5` of the whole username, then, when an account folder's suffix (`clean_account_dir_name`)
/// can be cut off it, the `md5` of what is left, the folder the desktop app looked in. A username that is already an
/// md5 is its own folder.
pub fn session_dirs_for_storage(session_id: &str) -> Vec<String> {
    let normalized = session_id.trim().to_lowercase();
    if normalized.is_empty() {
        return Vec::new();
    }
    if looks_like_md5(&normalized) {
        return vec![normalized];
    }
    let mut dirs = vec![md5_hex(&normalized)];
    let cleaned = crate::config::clean_account_dir_name(&normalized).to_lowercase();
    if !cleaned.is_empty() && cleaned != normalized {
        dirs.push(if looks_like_md5(&cleaned) {
            cleaned
        } else {
            md5_hex(&cleaned)
        });
    }
    dirs
}

fn md5_hex(text: &str) -> String {
    use md5::{Digest, Md5};
    Md5::digest(text.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Whether a `.dat` file named `file_name` may be a file of the image `base_md5` (lowercase): `<md5>.dat`, a variant
/// such as `<md5>_t.dat`, or a name that normalizes to the MD5.
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

/// Whether `path` lies in a folder of images (`img`, `image` or `msgimg`).
pub fn is_img_scoped_dat_path(path: &str) -> bool {
    rx(r"[\\/](img|image|msgimg)[\\/]").is_match(&path.to_lowercase())
}

// ───────────────────────── cache layout ─────────────────────────

const CACHE_EXTENSIONS: [&str; 5] = [".jpg", ".jpeg", ".png", ".gif", ".webp"];

/// Where the image decrypted from `dat_path` is kept in the cache `root`:
/// `<conversation>/<month>/<md5><variant suffix><ext>`, `month` being the `.dat` file's [`month_folder`].
pub fn cache_output_path(
    root: &Path,
    dat_path: &str,
    month: &str,
    ext: &str,
    session_id: Option<&str>,
) -> PathBuf {
    let lower = file_name_lower(dat_path);
    let base = lower.strip_suffix(".dat").unwrap_or(&lower).to_string();
    let normalized = normalize_dat_base(&base);
    let suffix = cache_variant_suffix_from_dat(dat_path);
    root.join(sanitize_dir_name(session_id.unwrap_or("unknown")))
        .join(month)
        .join(format!("{normalized}{suffix}{ext}"))
}

/// Every place in the cache `root` the image decrypted from `dat_path` may be kept, the most likely first: the
/// layout of [`cache_output_path`] (with `prefer_hd`, the original's suffix first), then the older layouts.
pub fn cache_output_candidates(
    root: &Path,
    dat_path: &str,
    month: &str,
    session_id: Option<&str>,
    prefer_hd: bool,
) -> Vec<PathBuf> {
    let candidates = CacheCandidates::new(root, dat_path, month, session_id, prefer_hd);
    let mut out = Vec::new();
    for folder in candidates.folders() {
        folder.each_name(|name| out.push(folder.path().join(name)));
    }
    out
}

/// [`cache_output_candidates`] as its three folders and the names tried in each, so that a lookup checks a name
/// against its folder's listing before it makes a path of it: an image tries some 135 names, and few of them exist.
#[derive(Debug)]
pub struct CacheCandidates {
    /// The base name every candidate starts with.
    normalized: String,
    /// The variant suffix of the `.dat` file.
    primary: String,
    prefer_hd: bool,
    /// The conversation's month folder, the image's own folder (an older layout) and the root (the oldest).
    paths: [PathBuf; 3],
}

/// A folder of [`CacheCandidates`].
#[derive(Clone, Copy, Debug)]
pub struct CacheFolder<'a> {
    of: &'a CacheCandidates,
    path: &'a Path,
    /// Whether the names carry every variant suffix in the order searched, rather than the three of the root.
    variants: bool,
}

impl CacheCandidates {
    /// The candidates of the image decrypted from `dat_path`, like [`cache_output_candidates`].
    pub fn new(
        root: &Path,
        dat_path: &str,
        month: &str,
        session_id: Option<&str>,
        prefer_hd: bool,
    ) -> Self {
        let lower = file_name_lower(dat_path);
        let base = lower.strip_suffix(".dat").unwrap_or(&lower);
        let normalized = normalize_dat_base(base);
        let current = root
            .join(sanitize_dir_name(session_id.unwrap_or("unknown")))
            .join(month);
        let legacy = root.join(&normalized);
        Self {
            primary: cache_variant_suffix_from_dat(dat_path),
            prefer_hd,
            paths: [current, legacy, root.to_path_buf()],
            normalized,
        }
    }

    /// The folders, in the order they are tried.
    pub fn folders(&self) -> [CacheFolder<'_>; 3] {
        let [current, legacy, root] = &self.paths;
        [(current, true), (legacy, true), (root, false)].map(|(path, variants)| CacheFolder {
            of: self,
            path,
            variants,
        })
    }
}

impl CacheFolder<'_> {
    /// The folder.
    pub fn path(&self) -> &Path {
        self.path
    }

    /// Calls `found` with each file name tried in the folder, in order; the candidates are these names joined to
    /// [`Self::path`]. The names are lower-case, as the cache writes them.
    pub fn each_name(&self, mut found: impl FnMut(&str)) {
        let normalized = &self.of.normalized;
        let mut name = String::with_capacity(normalized.len() + 12);
        let mut tried = |suffix: &str, ext: &str| {
            name.clear();
            name.push_str(normalized);
            name.push_str(suffix);
            name.push_str(ext);
            found(&name);
        };
        if self.variants {
            for suffix in suffix_search_order(&self.of.primary, self.of.prefer_hd) {
                for ext in CACHE_EXTENSIONS {
                    tried(suffix, ext);
                }
            }
        } else {
            for ext in CACHE_EXTENSIONS {
                for suffix in ["", "_t", "_hd"] {
                    tried(suffix, ext);
                }
            }
        }
    }
}

/// Whether `path` has the extension of an image the cache keeps (GIF, PNG, JPEG or WebP).
pub fn is_image_file(path: &str) -> bool {
    matches!(
        stem_and_ext(path).1.as_str(),
        ".gif" | ".png" | ".jpg" | ".jpeg" | ".webp"
    )
}

/// The MIME type of an image extension, with its dot and in any case; `None` for one it does not know.
pub fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_lowercase().as_str() {
        ".gif" => Some("image/gif"),
        ".png" => Some("image/png"),
        ".jpg" | ".jpeg" => Some("image/jpeg"),
        ".webp" => Some("image/webp"),
        _ => None,
    }
}

/// The four picture formats WeChat's files, its servers and WeFlow's caches hold, told by a file's first bytes.
///
/// Every place that names or serves a picture by its content asks here, then adds what only it knows (WXGF, BMP,
/// HEIF, the extension it falls back on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageFormat {
    /// GIF.
    Gif,
    /// PNG.
    Png,
    /// JPEG.
    Jpeg,
    /// WebP.
    Webp,
}

impl ImageFormat {
    /// The format `bytes` start as: `GIF`, `\x89PNG`, `FF D8 FF`, or `RIFF` with `WEBP` at byte 8.
    pub fn of(bytes: &[u8]) -> Option<Self> {
        if bytes.starts_with(b"GIF") {
            Some(Self::Gif)
        } else if bytes.starts_with(b"\x89PNG") {
            Some(Self::Png)
        } else if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
            Some(Self::Jpeg)
        } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP".as_slice()) {
            Some(Self::Webp)
        } else {
            None
        }
    }

    /// The file extension, with its dot.
    pub fn extension(self) -> &'static str {
        match self {
            Self::Gif => ".gif",
            Self::Png => ".png",
            Self::Jpeg => ".jpg",
            Self::Webp => ".webp",
        }
    }

    /// The content type.
    pub fn mime(self) -> &'static str {
        match self {
            Self::Gif => "image/gif",
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Webp => "image/webp",
        }
    }
}

/// The extension of an image by its first bytes (needs 12 bytes, knows four formats).
pub fn detect_image_extension(b: &[u8]) -> Option<&'static str> {
    if b.len() < 12 {
        return None;
    }
    ImageFormat::of(b).map(ImageFormat::extension)
}

// ───────────────────────── JPEG sanity ─────────────────────────

/// How many bytes of `data` are zero, counted 255 at a time in a `u8`: the compiler turns that into vector compares
/// and adds, where a count of the whole a byte at a time stays one byte a step. Cached images are read whole for it.
fn count_zeros(data: &[u8]) -> usize {
    data.chunks(255)
        .map(|chunk| usize::from(chunk.iter().fold(0u8, |n, &b| n + u8::from(b == 0))))
        .sum()
}

/// Whether a JPEG is likely corrupt: an encoder that was fed garbage emits an almost all-zero scan.
pub fn is_likely_corrupted_jpeg(data: &[u8]) -> bool {
    if data.len() < 4096 {
        return false;
    }
    let zero_ratio = count_f64(count_zeros(data)) / count_f64(data.len());
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
    count_f64(count_zeros(scan)) / count_f64(scan.len()) >= 0.985
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

/// The HEVC streams to try for a WXGF blob: VPS-delimited groups (largest first), all NAL units, raw payload.
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

/// Whether `buf` is a WXGF image, WeChat's HEVC container: it starts with `wxgf`.
pub fn is_wxgf(buf: &[u8]) -> bool {
    buf.len() >= 20 && &buf[0..4] == b"wxgf"
}

/// The image bytes of `buf`, converted from WXGF when it is a WXGF blob.
///
/// When the data is still an undecoded WXGF blob, the second value says why the built-in decoder ([`crate::hevc`])
/// could not convert it (`None`: it is an image, or was not WXGF). The candidates are tried in order; the reason
/// given is the first one's, the stream the others are cut from.
pub fn unwrap_wxgf(buf: Vec<u8>) -> (Vec<u8>, Option<crate::hevc::Unsupported>) {
    if !is_wxgf(&buf) {
        return (buf, None);
    }
    if let Some(inner) = wxgf_embedded_image(&buf) {
        return (inner.to_vec(), None);
    }
    let candidates = wxgf_hevc_candidates(&buf);
    let mut failure = None;
    for (_, data) in &candidates {
        match crate::hevc::decode_to_jpeg(data) {
            Ok(jpg) => return (jpg, None),
            Err(e) => {
                failure.get_or_insert(e);
            }
        }
    }
    let fallback = candidates
        .first()
        .map(|(_, d)| d.clone())
        .unwrap_or_else(|| buf[4..].to_vec());
    (
        fallback,
        Some(failure.unwrap_or(crate::hevc::Unsupported::NoPicture)),
    )
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
            format!("{MD5}f.txt"),
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
        assert!(is_hd_path(&format!("/c/{MD5}_hd.png")));
        assert!(!is_hd_path(&format!("/c/{MD5}.png")));
        assert!(is_thumbnail_path(&format!("/c/{MD5}_t.jpg")));
        assert!(is_thumbnail_path(&format!("/c/{MD5}.t.jpg")));
        assert!(
            !is_thumbnail_path(&format!("/my_temp/Images/wxid_tom/{MD5}.jpg")),
            "only the file name tells a thumbnail"
        );
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
            "2024-05",
            Some("wxid:bob"),
            true,
        );
        assert_eq!(
            c[0].file_name().unwrap().to_string_lossy(),
            format!("{MD5}_hd.jpg")
        );
        assert!(
            c[0].starts_with("/cache/Images/wxid_bob/2024-05"),
            "{:?}",
            c[0]
        );
        assert!(c
            .iter()
            .any(|p| p == &root.join(MD5).join(format!("{MD5}_hd.webp"))));
        assert!(c.iter().any(|p| p == &root.join(format!("{MD5}_t.gif"))));
        let out = cache_output_path(
            root,
            &format!("/nonexistent/{MD5}_t.dat"),
            "2024-05",
            ".png",
            Some("a/b"),
        );
        let out = out.to_string_lossy().replace('\\', "/");
        assert!(out.contains("/a_b/2024-05/"));
        assert!(out.ends_with(&format!("{MD5}_t.png")));
    }

    /// The candidates made one path at a time, every suffix list deduplicated through a set.
    fn cache_output_candidates_each(
        root: &Path,
        dat_path: &str,
        month: &str,
        session_id: Option<&str>,
        prefer_hd: bool,
    ) -> Vec<PathBuf> {
        let lower = file_name_lower(dat_path);
        let base = lower.strip_suffix(".dat").unwrap_or(&lower).to_string();
        let normalized = normalize_dat_base(&base);
        let primary = cache_variant_suffix_from_dat(dat_path);
        let mut suffixes: Vec<String> = if prefer_hd {
            vec!["_hd".into(), primary]
        } else {
            vec![primary, "_hd".into()]
        };
        suffixes.extend(
            [
                "_hd", "_thumb", "_t", ".t", "_b", ".b", "_w", ".w", "_c", ".c", "",
            ]
            .map(String::from),
        );
        let mut seen = std::collections::HashSet::new();
        suffixes.retain(|s| seen.insert(s.clone()));
        let current = root
            .join(sanitize_dir_name(session_id.unwrap_or("unknown")))
            .join(month);
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

    /// `.dat` file names as WeChat writes them and as they may be: an MD5 in any case, a variant suffix or a junk
    /// one, or none.
    fn dat_names() -> impl proptest::strategy::Strategy<Value = String> {
        use proptest::prelude::*;
        let stem = prop_oneof![
            Just(MD5.to_string()),
            Just(MD5.to_uppercase()),
            "[0-9a-fA-F]{0,40}",
            "[a-zA-Z0-9_.é-]{0,12}",
        ];
        let suffix = prop_oneof![
            Just(String::new()),
            proptest::sample::select(vec![
                "_h", ".h", "_hd", ".hd", "_t", ".t", "_thumb", ".thumb", "_b", "_w", ".c", "_x",
                "_hd_t", "_W", "-1",
            ])
            .prop_map(String::from),
            "[._-]?[a-zA-Z0-9]{0,3}",
        ];
        let ext = prop_oneof![Just(".dat"), Just(".DAT"), Just(""), Just(".jpg")];
        (stem, suffix, ext).prop_map(|(stem, suffix, ext)| format!("{stem}{suffix}{ext}"))
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(256))]

        /// For any `.dat` file and conversation, the candidates made from the folders and their names are those made
        /// one path at a time, in the same order, and each name is the lower-case file name of its path.
        #[test]
        fn any_dat_file_has_the_candidates_made_one_path_at_a_time(
            name in dat_names(),
            session in proptest::option::of("[a-zA-Z0-9_@:/ .-]{0,16}"),
            prefer_hd in proptest::bool::ANY,
        ) {
            let root = Path::new("/cache/Images");
            let dat = format!("/attach/abc/2024-05/Img/{name}");
            let session = session.as_deref();
            proptest::prop_assert_eq!(
                cache_output_candidates(root, &dat, "2024-05", session, prefer_hd),
                cache_output_candidates_each(root, &dat, "2024-05", session, prefer_hd)
            );
            let candidates = CacheCandidates::new(root, &dat, "2024-05", session, prefer_hd);
            for folder in candidates.folders() {
                let mut names = Vec::new();
                folder.each_name(|n| names.push(n.to_string()));
                for n in names {
                    let path = folder.path().join(&n);
                    proptest::prop_assert_eq!(path.parent(), Some(folder.path()));
                    proptest::prop_assert_eq!(
                        path.file_name().map(|f| f.to_string_lossy().to_lowercase()),
                        Some(n)
                    );
                }
            }
        }
    }

    #[test]
    fn session_dirs_are_md5_of_the_username() {
        assert_eq!(
            session_dir_for_storage("WXID_bob"),
            "8a7b11f2fd24e19a60664a5fe5d56342"
        );
        assert_eq!(session_dir_for_storage(MD5), MD5);
        assert_eq!(session_dir_for_storage(""), "");
        // a username ending like an account folder is looked for whole first, then cut as the desktop app did (#657)
        assert_eq!(
            session_dirs_for_storage("name_1998"),
            [md5_hex("name_1998"), md5_hex("name")]
        );
        assert_eq!(
            session_dirs_for_storage("wxid_abc_1a2b"),
            [md5_hex("wxid_abc_1a2b"), md5_hex("wxid_abc")]
        );
        assert_eq!(
            session_dirs_for_storage(&format!("{MD5}_1a2b")),
            [md5_hex(&format!("{MD5}_1a2b")), MD5.to_string()]
        );
        assert_eq!(session_dirs_for_storage("123@chatroom").len(), 1);
        assert!(session_dirs_for_storage("  ").is_empty());
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
    fn formats_are_told_by_their_first_bytes() {
        let told = |bytes: &[u8]| ImageFormat::of(bytes).map(|f| (f.extension(), f.mime()));
        assert_eq!(told(b"GIF89a"), Some((".gif", "image/gif")));
        assert_eq!(told(b"\x89PNG\r\n\x1a\n"), Some((".png", "image/png")));
        assert_eq!(told(&[0xff, 0xd8, 0xff]), Some((".jpg", "image/jpeg")));
        assert_eq!(told(b"RIFF\0\0\0\0WEBPVP8 "), Some((".webp", "image/webp")));
        // a RIFF file that is no WebP, a start cut short, and no picture at all
        for other in [
            b"RIFF\0\0\0\0WAVEfmt ".as_slice(),
            b"RIFF\0\0\0\0WEB",
            &[0xff, 0xd8],
            b"wxgf",
            b"BM",
            b"",
        ] {
            assert_eq!(told(other), None, "{other:?}");
        }
    }

    #[test]
    fn corrupted_jpegs_are_mostly_zero() {
        let mut good = vec![0xff, 0xd8, 0xff, 0xe0];
        good.extend((0..6000).map(|i| u8::try_from(i % 251 + 1).unwrap()));
        assert!(!is_likely_corrupted_jpeg(&good));
        let mut bad = vec![0xff, 0xd8, 0xff, 0xe0];
        bad.extend(vec![0u8; 6000]);
        assert!(is_likely_corrupted_jpeg(&bad));
        assert!(
            !is_likely_corrupted_jpeg(&bad[..100]),
            "small files are never flagged"
        );
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(256))]

        /// For any bytes, mostly zero or hardly any, of any length around the 255 of a step, the zeros counted a step
        /// at a time are those counted one by one.
        #[test]
        fn any_bytes_have_the_zeros_counted_one_by_one(
            data in proptest::collection::vec(
                proptest::prop_oneof![
                    3 => proptest::strategy::Just(0u8),
                    1 => proptest::num::u8::ANY,
                ],
                0..2000,
            ),
            dense in proptest::bool::ANY,
        ) {
            let data: Vec<u8> = if dense { data } else { data.iter().map(|b| b.wrapping_add(1)).collect() };
            proptest::prop_assert_eq!(count_zeros(&data), data.iter().filter(|&&b| b == 0).count());
        }
    }

    #[test]
    fn wxgf_with_an_embedded_jpeg_is_unwrapped_as_it_is() {
        let mut blob = b"wxgf".to_vec();
        blob.extend([1u8; 20]);
        blob.extend([0xff, 0xd8, 0xff, 0xe0, 9, 9, 9, 9, 9, 9, 9, 9]);
        blob.extend([7u8; 32]);
        let (out, failure) = unwrap_wxgf(blob);
        assert_eq!(failure, None);
        assert_eq!(&out[..3], &[0xff, 0xd8, 0xff]);
        let (plain, failure) = unwrap_wxgf(vec![1, 2, 3]);
        assert_eq!(plain, vec![1, 2, 3]);
        assert_eq!(failure, None);
    }

    #[test]
    fn a_wxgf_blob_the_built_in_decoder_cannot_read_says_why() {
        let mut blob = b"wxgf".to_vec();
        blob.extend([0x11u8; 1300]);
        let (out, failure) = unwrap_wxgf(blob);
        assert_eq!(failure, Some(crate::hevc::Unsupported::NoPicture));
        assert_eq!(
            out,
            vec![0x11u8; 1300],
            "the stream after the magic is left as it is"
        );
        let mut blob = b"wxgf".to_vec();
        blob.extend([0x24u8, 0x01, 0x00, 0x46, 0x00, 0x2e]);
        blob.extend(include_bytes!("../testdata/wxgf/full_range.hevc"));
        let (jpg, failure) = unwrap_wxgf(blob);
        assert_eq!((failure, &jpg[..3]), (None, &[0xff, 0xd8, 0xff][..]));
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
