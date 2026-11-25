//! Read-only check of `image::ending` on a configured account (#1112): decrypts every image `.dat` under the account's
//! `msg/attach` and counts, by rendition and format, whether the picture is whole, has bytes after its end, or is cut
//! short. Then it imitates files WeChat is still writing: each non-thumbnail `.dat` cut to a part of its length, or
//! with its last part zero-filled, decrypted again, counting how the cut shows (a decrypt error, a picture cut short,
//! or a picture taken as whole). A few hundred WXGF files are cut too, to count how the HEVC decoder reports them.
//! With `--cache`, it counts instead how the pictures in the image cache of the configuration end (whole, bytes after
//! the end, cut short, and the zeroed-out JPEGs `is_likely_corrupted_jpeg` finds): the cache WeFlow drops a cut-short
//! JPEG from, which may be the only copy left of an image WeChat has since removed.
//! Usage: `dat_ending_probe <isolated config.json>`, `dat_ending_probe <config.json> --cache`
//! Stdout contains only aggregate counts. No key, identifier, file name, image or local path is printed, and nothing
//! is written.

use anyhow::{Context, Result};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::Instant;
use weflow_core::config::{resolve_account_dir, ConfigStore};
use weflow_core::decrypt;
use weflow_core::image::{self as img, ending, ImageFormat};

/// How much of a file a cut keeps.
#[derive(Clone, Copy)]
enum Keep {
    /// This many tenths of its length.
    Tenths(u64),
    /// All but its last this many bytes.
    AllBut(u64),
}

/// The cuts made to each file.
const CUTS: [(&str, Keep); 3] = [
    ("half", Keep::Tenths(5)),
    ("nine_tenths", Keep::Tenths(9)),
    ("all_but_4k", Keep::AllBut(4096)),
];

/// How many WXGF files are cut (decoding them is slow).
const WXGF_CUT_SAMPLE: usize = 300;

#[derive(Default)]
struct Tally {
    counts: BTreeMap<String, u64>,
    aes_sizes: BTreeMap<i32, u64>,
    xor_sizes: BTreeMap<i32, u64>,
    wxgf_cut: usize,
}

impl Tally {
    fn add(&mut self, key: String) {
        *self.counts.entry(key).or_default() += 1;
    }
}

fn variant(path: &Path) -> &'static str {
    let name = path.to_string_lossy();
    if img::is_hd_dat_path(&name) {
        "hd"
    } else if img::is_t_variant_dat(&name) {
        "thumb"
    } else {
        "normal"
    }
}

/// The kind of a decrypt error, without its text.
fn error_kind(e: &anyhow::Error) -> &'static str {
    let text = format!("{e:#}");
    if text.contains("invalid xor size") {
        "xor_size"
    } else if text.contains("invalid aes size") {
        "aes_size"
    } else if text.contains("too small") {
        "too_small"
    } else if text.contains("pkcs7") {
        "padding"
    } else if text.contains("requires an AES key") {
        "no_key"
    } else {
        "other"
    }
}

fn format_name(data: &[u8]) -> &'static str {
    match ImageFormat::of(data) {
        Some(ImageFormat::Jpeg) => "jpeg",
        Some(ImageFormat::Png) => "png",
        Some(ImageFormat::Gif) => "gif",
        Some(ImageFormat::Webp) => "webp",
        None if img::is_wxgf(data) => "wxgf",
        None => "other",
    }
}

/// How a picture ends: whole, bytes after its end (and what they look like), or cut short.
fn ending_name(data: &[u8]) -> String {
    match ending::of(data) {
        ending::Ending::At(end) if end == data.len() => "whole".into(),
        ending::Ending::At(end) => {
            let rest = &data[end..];
            let kind = if rest.iter().all(|&b| b == 0) {
                "zeros"
            } else if rest.len() >= 12 && rest[..12].windows(4).any(|w| w == b"ftyp") {
                "video"
            } else {
                "other"
            };
            let size = match rest.len() {
                0..=16 => "le16",
                17..=4096 => "le4k",
                _ => "gt4k",
            };
            format!("after_end_{kind}_{size}")
        }
        ending::Ending::CutShort => "cut_short".into(),
        ending::Ending::Unknown => "unknown".into(),
    }
}

/// How a decrypted, possibly cut file shows: a decrypt error, a WXGF the decoder rejects or converts, or the ending
/// of a picture.
fn cut_outcome(bytes: &[u8], xor_key: u8, aes_key: Option<&[u8; 16]>, decode_wxgf: bool) -> String {
    match decrypt::decrypt_dat(bytes, xor_key, aes_key) {
        Err(e) => format!("error_{}", error_kind(&e)),
        Ok(d) if img::is_wxgf(&d.data) => {
            if !decode_wxgf {
                return "wxgf_not_decoded".into();
            }
            match img::unwrap_wxgf(d.data) {
                (_, Some(reason)) if reason.is_damaged() => "wxgf_damaged".into(),
                (_, Some(_)) => "wxgf_unsupported".into(),
                (jpg, None) => format!("wxgf_converted_{}", ending_name(&jpg)),
            }
        }
        Ok(d) => format!("{}_{}", format_name(&d.data), ending_name(&d.data)),
    }
}

fn examine(path: &Path, xor_key: u8, aes_key: Option<&[u8; 16]>, tally: &Mutex<Tally>) {
    let lock = || tally.lock().unwrap_or_else(PoisonError::into_inner);
    let rendition = variant(path);
    let Ok(bytes) = std::fs::read(path) else {
        lock().add(format!("{rendition}/unreadable"));
        return;
    };
    let version = decrypt::detect_dat_version(&bytes);
    if version != 0 && bytes.len() >= 15 {
        let size = |at: usize| {
            i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let mut t = lock();
        *t.aes_sizes.entry(size(6)).or_default() += 1;
        *t.xor_sizes.entry(size(10)).or_default() += 1;
    }
    let data = match decrypt::decrypt_dat(&bytes, xor_key, aes_key) {
        Ok(d) => d.data,
        Err(e) => {
            lock().add(format!("{rendition}/v{version}/error_{}", error_kind(&e)));
            return;
        }
    };
    let format = format_name(&data);
    let end = if format == "wxgf" || format == "other" {
        String::new()
    } else {
        ending_name(&data)
    };
    // zero bytes the file ends in: a file set aside at its full size and not written to the end would
    let zeros = bytes.iter().rev().take_while(|&&b| b == 0).count();
    let zeros = match zeros {
        0 => "0",
        1..=15 => "1_15",
        16..=255 => "16_255",
        _ => "ge256",
    };
    {
        let mut t = lock();
        t.add(format!("{rendition}/v{version}/{format}/{end}"));
        t.add(format!("zero_tail/{rendition}/{format}/{zeros}"));
    }
    // a JPEG or PNG the WXGF container is taken to embed: how it ends, and for one cut short whether the HEVC stream
    // of the same container decodes
    if let Some(inner) = img::wxgf_embedded_image(&data) {
        let mut key = format!("wxgf_embedded/{rendition}/{}", ending_name(inner));
        if ending::is_cut_short(inner) {
            let decodes = img::wxgf_hevc_candidates(&data)
                .iter()
                .any(|(_, stream)| weflow_core::hevc::decode_to_jpeg(stream).is_ok());
            key.push_str(if decodes {
                "/hevc_decodes"
            } else {
                "/hevc_fails"
            });
        }
        lock().add(key);
    }

    if rendition == "thumb" || format == "other" {
        return;
    }
    let decode_wxgf = format == "wxgf" && {
        let mut t = lock();
        t.wxgf_cut += 1;
        t.wxgf_cut <= WXGF_CUT_SAMPLE
    };
    if format == "wxgf" && !decode_wxgf {
        return;
    }
    let len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    for (name, keep) in CUTS {
        let keep = match keep {
            Keep::Tenths(n) => len * n / 10,
            Keep::AllBut(n) => len.saturating_sub(n),
        };
        let keep = usize::try_from(keep).unwrap_or(bytes.len());
        let prefix = cut_outcome(&bytes[..keep], xor_key, aes_key, decode_wxgf);
        let mut zeroed = bytes.clone();
        zeroed[keep..].fill(0);
        let zero_filled = cut_outcome(&zeroed, xor_key, aes_key, decode_wxgf);
        let mut t = lock();
        t.add(format!("cut/{format}/v{version}/{name}/prefix/{prefix}"));
        t.add(format!(
            "cut/{format}/v{version}/{name}/zero_filled/{zero_filled}"
        ));
    }
}

/// The ten most common values and how many files have the others.
fn top(map: &BTreeMap<i32, u64>) -> serde_json::Value {
    let mut v: Vec<(i32, u64)> = map.iter().map(|(&k, &n)| (k, n)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let rest: u64 = v.iter().skip(10).map(|(_, n)| n).sum();
    json!({
        "distinct": v.len(),
        "top": v.iter().take(10).map(|(k, n)| json!([k, n])).collect::<Vec<_>>(),
        "others": rest,
    })
}

/// How the pictures under `root`, the image cache, end. For a JPEG dropped as cut short, whether the account still
/// holds a `.dat` of the same image that decrypts to a whole picture (`dats`: the account's `.dat` files by their
/// normalized base name), from which it is decrypted again.
fn scan_cache(
    root: &Path,
    dats: &BTreeMap<String, Vec<PathBuf>>,
    xor_key: u8,
    aes_key: Option<&[u8; 16]>,
) -> serde_json::Value {
    let mut counts: BTreeMap<String, u64> = BTreeMap::new();
    let files = walkdir::WalkDir::new(root)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file());
    for entry in files {
        let Ok(data) = std::fs::read(entry.path()) else {
            *counts.entry("unreadable".into()).or_default() += 1;
            continue;
        };
        let format = format_name(&data);
        let mut key = format!("{format}/{}", ending_name(&data));
        if format == "jpeg" {
            let dropped = ending::is_cut_short(&data);
            key.push_str(if dropped { "/dropped" } else { "/kept" });
            if img::is_likely_corrupted_jpeg(&data) {
                key.push_str("/zeroed");
            }
            if dropped {
                // whether a decoder reads it to the end: one that is really cut short fails or stops early
                let mut decoder = jpeg_decoder::Decoder::new(data.as_slice());
                key.push_str(match decoder.decode() {
                    Ok(_) => "/decodes",
                    Err(_) => "/does_not_decode",
                });
                let name = entry.file_name().to_string_lossy().to_lowercase();
                key.push_str(if name.contains("_hd.") {
                    "/hd"
                } else if name.contains("_t.") || name.contains("_thumb.") {
                    "/thumb"
                } else {
                    "/normal"
                });
                let base = img::normalize_dat_base(name.split('.').next().unwrap_or(""));
                let whole_dat = dats.get(&base).is_some_and(|paths| {
                    paths.iter().any(|p| {
                        decrypt::decrypt_file(p, xor_key, aes_key).is_ok_and(|d| {
                            let (data, failure) = img::unwrap_wxgf(d.data);
                            failure.is_none() && !ending::is_cut_short(&data)
                        })
                    })
                });
                key.push_str(match (dats.contains_key(&base), whole_dat) {
                    (_, true) => "/whole_dat_on_disk",
                    (true, false) => "/no_whole_dat",
                    (false, _) => "/no_dat",
                });
            }
        }
        *counts.entry(key).or_default() += 1;
    }
    json!({ "cache_files": counts.values().sum::<u64>(), "counts": counts })
}

fn main() -> Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .context("usage: dat_ending_probe <isolated config.json> [--cache]")?;
    let config = ConfigStore::load(Path::new(&config_path))?;
    let profile = config.profile(None).context("no profile")?;
    let xor_key = profile
        .image_xor_key
        .and_then(|k| u8::try_from(k).ok())
        .context("no image XOR key configured")?;
    let aes_key = profile
        .image_aes_key
        .as_deref()
        .and_then(decrypt::parse_aes_key);
    let account = resolve_account_dir(
        profile.db_path.as_deref().unwrap_or(""),
        profile.wxid.as_deref().unwrap_or(""),
    );
    let files: Vec<PathBuf> = walkdir::WalkDir::new(account.join("msg").join("attach"))
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .map(walkdir::DirEntry::into_path)
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("dat")))
        .collect();

    if std::env::args().nth(2).as_deref() == Some("--cache") {
        let mut dats: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
        for path in &files {
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_lowercase())
                .unwrap_or_default();
            dats.entry(img::normalize_dat_base(&name))
                .or_default()
                .push(path.clone());
        }
        // `cache_path`, else `cache` beside the configuration file
        let base = profile
            .cache_path
            .as_deref()
            .filter(|p| !p.trim().is_empty())
            .map_or_else(
                || Path::new(&config_path).with_file_name("cache"),
                PathBuf::from,
            );
        let report = scan_cache(&base.join("Images"), &dats, xor_key, aes_key.as_ref());
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }

    let started = Instant::now();
    let tally = Mutex::new(Tally::default());
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let chunk = files.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for part in files.chunks(chunk) {
            let tally = &tally;
            let aes_key = aes_key.as_ref();
            s.spawn(move || {
                for path in part {
                    examine(path, xor_key, aes_key, tally);
                }
            });
        }
    });
    let t = tally.into_inner().unwrap_or_else(PoisonError::into_inner);
    let report = json!({
        "files": files.len(),
        "seconds": started.elapsed().as_secs(),
        "counts": t.counts,
        "header_aes_size": top(&t.aes_sizes),
        "header_xor_size": top(&t.xor_sizes),
        "wxgf_cut_sample": t.wxgf_cut.min(WXGF_CUT_SAMPLE),
    });
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
