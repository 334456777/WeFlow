//! Read-only coverage of the built-in WXGF decoder on a configured account (#494): decrypts every image `.dat` under
//! the account's `msg/attach`, and for each WXGF picture tries the candidate streams the way the image service does,
//! counting why the ones it cannot decode fail. `decoded_jpegs_sha256`, one hash of the JPEGs of all decoded pictures,
//! tells whether two builds write the same files.
//! Usage: wxgf_coverage_probe <isolated config.json>
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
    },
    Failed(Unsupported),
    NoCandidate,
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
                return Outcome::Decoded {
                    candidate: k,
                    jpeg: Sha256::digest(&jpg).into(),
                    samples: hevc::coded_luma_samples(stream),
                    took: start.elapsed(),
                }
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
                        } => {
                            t.decoded += 1;
                            *t.by_candidate.entry(candidate).or_default() += 1;
                            t.max_samples = t.max_samples.max(samples.unwrap_or(0));
                            t.times.push(took);
                            t.jpegs.push(jpeg);
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
            "threads": threads,
            "elapsed_s": started.elapsed().as_secs_f64(),
        }))?
    );
    Ok(())
}
