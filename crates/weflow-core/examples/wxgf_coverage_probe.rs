//! Read-only coverage of the built-in WXGF decoder on a configured account (#494): decrypts every image `.dat` under
//! the account's `msg/attach`, and for each WXGF picture tries the candidate streams the way the image service does,
//! counting why the ones it cannot decode fail. `decoded_jpegs_sha256`, one hash of the JPEGs of all decoded pictures,
//! tells whether two builds write the same files. `decoded_shapes` sums up the decoded pictures by the file of the
//! picture they are and by the size of their `.dat`: picture and stream sizes, bits per luma sample, chroma and
//! profile, the numbers the benchmark account's WXGF pictures copy (#1221).
//! Usage: `wxgf_coverage_probe <isolated config.json>`
//! Stdout contains only aggregate counts. No key, identifier, file name, image or local path is printed, and nothing
//! is written.

use anyhow::{Context, Result};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};
use weflow_core::config::{resolve_account_dir, ConfigStore};
use weflow_core::hevc::{self, Unsupported};
use weflow_core::image as img;

/// What one `.dat` file turned out to be.
enum Outcome {
    NotDecrypted,
    NotWxgf,
    Embedded,
    Decoded {
        candidate: usize,
        /// SHA-256 of the JPEG.
        jpeg: [u8; 32],
        samples: Option<u64>,
        took: Duration,
        shape: Shape,
    },
    Failed(Unsupported),
    NoCandidate,
}

/// What a decoded WXGF picture is like, for the benchmark account's pictures (#1221): which file of the picture it is,
/// the sizes of the file and of the stream, the picture's size and its chroma and profile.
struct Shape {
    /// `normal`, `hd` (`_h.dat`) or `thumb` (`_t.dat`).
    variant: &'static str,
    file_bytes: u64,
    stream_bytes: u64,
    width: u64,
    height: u64,
    /// `420`, `422`, `444` or `gray`, as the JPEG keeps it.
    chroma: &'static str,
    /// `general_profile_idc` of the first SPS (1 Main, 2 Main 10, 3 Main Still Picture, 4 range extensions).
    profile: Option<u8>,
}

/// (height, width, chroma) of a baseline JPEG, from its SOF0 segment.
fn jpeg_shape(jpg: &[u8]) -> Option<(u64, u64, &'static str)> {
    let sof = jpg.windows(2).position(|w| w == [0xFF, 0xC0])? + 2;
    let seg = jpg.get(sof..sof + 12)?;
    let height = u64::from(u16::from_be_bytes([seg[3], seg[4]]));
    let width = u64::from(u16::from_be_bytes([seg[5], seg[6]]));
    let chroma = match (seg[7], seg[9]) {
        (1, _) => "gray",
        (_, 0x22) => "420",
        (_, 0x21) => "422",
        (_, 0x11) => "444",
        _ => "other",
    };
    Some((height, width, chroma))
}

/// `general_profile_idc` of the first SPS in an Annex-B stream.
fn profile_idc(stream: &[u8]) -> Option<u8> {
    let at = stream.windows(5).position(|w| w == [0, 0, 1, 0x42, 0x01])? + 5;
    Some(stream.get(at + 1)? & 0x1f)
}

fn examine(path: &Path, xor_key: u8, aes_key: Option<&[u8; 16]>) -> Outcome {
    let Ok(decrypted) = weflow_core::decrypt::decrypt_file(path, xor_key, aes_key) else {
        return Outcome::NotDecrypted;
    };
    let data = decrypted.data;
    if !img::is_wxgf(&data) {
        return Outcome::NotWxgf;
    }
    if img::wxgf_embedded_image(&data).is_some() {
        return Outcome::Embedded;
    }
    let candidates = img::wxgf_hevc_candidates(&data);
    let mut first_failure = None;
    for (k, (_, stream)) in candidates.iter().enumerate() {
        let start = Instant::now();
        match hevc::decode_to_jpeg(stream) {
            Ok(jpg) => {
                let took = start.elapsed();
                let stem = path.file_stem().map(|s| s.to_string_lossy().to_string());
                let variant = match stem.as_deref() {
                    Some(s) if s.ends_with("_h") => "hd",
                    Some(s) if s.ends_with("_t") => "thumb",
                    _ => "normal",
                };
                let (height, width, chroma) = jpeg_shape(&jpg).unwrap_or((0, 0, "unread"));
                return Outcome::Decoded {
                    candidate: k,
                    jpeg: Sha256::digest(&jpg).into(),
                    samples: hevc::coded_luma_samples(stream),
                    took,
                    shape: Shape {
                        variant,
                        file_bytes: std::fs::metadata(path).map_or(0, |m| m.len()),
                        stream_bytes: stream.len() as u64,
                        width,
                        height,
                        chroma,
                        profile: profile_idc(stream),
                    },
                };
            }
            Err(e) => {
                first_failure.get_or_insert(e);
            }
        }
    }
    first_failure.map_or(Outcome::NoCandidate, Outcome::Failed)
}

#[derive(Default)]
struct Tally {
    files: usize,
    not_decrypted: usize,
    not_wxgf: usize,
    embedded: usize,
    decoded: usize,
    by_candidate: BTreeMap<usize, usize>,
    no_candidate: usize,
    failures: BTreeMap<String, usize>,
    max_samples: u64,
    times: Vec<Duration>,
    jpegs: Vec<[u8; 32]>,
    shapes: Vec<Shape>,
}

/// The `.dat` sizes [`shapes`] groups the pictures by, from each edge to the next.
const FILE_BUCKETS: &[u64] = &[
    0, 20_000, 50_000, 100_000, 200_000, 500_000, 1_000_000, 2_000_000,
];

/// `n`, median, 10th and 90th percentile and largest of `v`.
fn spread(mut v: Vec<u64>) -> serde_json::Value {
    v.sort_unstable();
    let at = |q: f64| {
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            clippy::cast_precision_loss,
            reason = "an index into the list"
        )]
        v.get(((v.len().saturating_sub(1)) as f64 * q).round() as usize)
            .copied()
    };
    json!({"n": v.len(), "p10": at(0.1), "p50": at(0.5), "p90": at(0.9), "max": v.last()})
}

/// The decoded pictures summed up by the file of the picture they are, and by the size of their `.dat` file: sizes,
/// bits per thousand luma samples, chroma and profile. Nothing that tells one picture from another.
fn shapes(shapes: &[Shape]) -> serde_json::Value {
    let summary = |of: &[&Shape]| {
        let mut chroma: BTreeMap<&str, usize> = BTreeMap::new();
        let mut profiles: BTreeMap<String, usize> = BTreeMap::new();
        for s in of {
            *chroma.entry(s.chroma).or_default() += 1;
            *profiles
                .entry(s.profile.map_or("unread".to_string(), |p| p.to_string()))
                .or_default() += 1;
        }
        let luma = |s: &&Shape| s.width * s.height;
        json!({
            "file_bytes": spread(of.iter().map(|s| s.file_bytes).collect()),
            "stream_bytes": spread(of.iter().map(|s| s.stream_bytes).collect()),
            "luma_samples": spread(of.iter().map(luma).collect()),
            "long_side": spread(of.iter().map(|s| s.width.max(s.height)).collect()),
            "aspect_permille": spread(of.iter().map(|s| s.width.max(s.height) * 1000 / s.width.min(s.height).max(1)).collect()),
            "stream_bits_per_kilo_luma": spread(of.iter().map(|s| s.stream_bytes * 8000 / luma(s).max(1)).collect()),
            "stream_permille_of_file": spread(of.iter().map(|s| s.stream_bytes * 1000 / s.file_bytes.max(1)).collect()),
            "chroma": chroma,
            "profile_idc": profiles,
        })
    };
    let mut by_variant = serde_json::Map::new();
    for variant in ["normal", "hd", "thumb"] {
        let of: Vec<&Shape> = shapes.iter().filter(|s| s.variant == variant).collect();
        if !of.is_empty() {
            by_variant.insert(variant.to_string(), summary(&of));
        }
    }
    let mut by_file_bytes = serde_json::Map::new();
    for (i, from) in FILE_BUCKETS.iter().enumerate() {
        let to = FILE_BUCKETS.get(i + 1).copied().unwrap_or(u64::MAX);
        let of: Vec<&Shape> = shapes
            .iter()
            .filter(|s| (*from..to).contains(&s.file_bytes))
            .collect();
        by_file_bytes.insert(
            from.to_string(),
            json!({
                "n": of.len(),
                "luma_samples": spread(of.iter().map(|s| s.width * s.height).collect()),
                "stream_bits_per_kilo_luma": spread(of.iter().map(|s| s.stream_bytes * 8000 / (s.width * s.height).max(1)).collect()),
            }),
        );
    }
    json!({"by_variant": by_variant, "by_file_bytes_from": by_file_bytes})
}

fn main() -> Result<()> {
    let config_path = std::env::args()
        .nth(1)
        .context("usage: wxgf_coverage_probe <isolated config.json>")?;
    let config = ConfigStore::load(Path::new(&config_path))?;
    let profile = config.profile(None).context("no profile")?;
    let xor_key = profile
        .image_xor_key
        .and_then(|k| u8::try_from(k).ok())
        .context("no image XOR key configured")?;
    let aes_key = profile
        .image_aes_key
        .as_deref()
        .and_then(weflow_core::decrypt::parse_aes_key);
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

    let started = Instant::now();
    let tally = Mutex::new(Tally {
        files: files.len(),
        ..Tally::default()
    });
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let chunk = files.len().div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for part in files.chunks(chunk) {
            let tally = &tally;
            let aes_key = aes_key.as_ref();
            s.spawn(move || {
                for path in part {
                    let outcome = examine(path, xor_key, aes_key);
                    let mut t = tally.lock().unwrap_or_else(PoisonError::into_inner);
                    match outcome {
                        Outcome::NotDecrypted => t.not_decrypted += 1,
                        Outcome::NotWxgf => t.not_wxgf += 1,
                        Outcome::Embedded => t.embedded += 1,
                        Outcome::NoCandidate => t.no_candidate += 1,
                        Outcome::Decoded {
                            candidate,
                            jpeg,
                            samples,
                            took,
                            shape,
                        } => {
                            t.decoded += 1;
                            *t.by_candidate.entry(candidate).or_default() += 1;
                            t.max_samples = t.max_samples.max(samples.unwrap_or(0));
                            t.times.push(took);
                            t.jpegs.push(jpeg);
                            t.shapes.push(shape);
                        }
                        Outcome::Failed(e) => *t.failures.entry(format!("{e:?}")).or_default() += 1,
                    }
                }
            });
        }
    });
    let mut t = tally.into_inner().unwrap_or_else(PoisonError::into_inner);
    t.times.sort();
    // whatever order the threads found them in: compares two builds without showing any image
    t.jpegs.sort_unstable();
    let digest: String = Sha256::digest(t.jpegs.concat())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let ms = |percent: usize| {
        t.times
            .get(t.times.len().saturating_sub(1) * percent / 100)
            .map_or(0.0, |d| d.as_secs_f64() * 1000.0)
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&json!({
            "dat_files": t.files,
            "not_decrypted": t.not_decrypted,
            "not_wxgf": t.not_wxgf,
            "wxgf_with_embedded_image": t.embedded,
            "wxgf_decoded": t.decoded,
            "decoded_by_candidate_index": t.by_candidate,
            "wxgf_without_candidate_stream": t.no_candidate,
            "wxgf_not_decoded_by_reason": t.failures,
            "largest_decoded_luma_samples": t.max_samples,
            "decoded_jpegs_sha256": digest,
            "decode_ms": { "median": ms(50), "p99": ms(99), "max": ms(100) },
            "decoded_shapes": shapes(&t.shapes),
            "threads": threads,
            "elapsed_s": started.elapsed().as_secs_f64(),
        }))?
    );
    Ok(())
}
