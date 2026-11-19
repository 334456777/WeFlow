//! Read-only kvcomm image-key acquisition. No native helper or process access is used.
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::error::{AppError, AppResult};
use crate::keys::{clean_wxid, derive_image_keys, scan_templates, verify_derived_aes_key};

pub const DEFAULT_SCAN_BUDGET: usize = 10_000;
const TEMPLATE_LIMIT: usize = 32;
const MAX_SAMPLE_BYTES: u64 = 16 * 1024 * 1024;

/// A `kvcomm` file a code was read from, and how its name was read.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord, schemars::JsonSchema)]
pub struct CandidateSource {
    pub path: PathBuf,
    pub parsing: &'static str,
}

#[derive(Debug, Serialize)]
pub struct CodeCandidate {
    pub code: u32,
    pub sources: Vec<CandidateSource>,
}

#[derive(Debug, Serialize)]
pub struct CollectionError {
    pub path: PathBuf,
    pub kind: String,
}

#[derive(Debug, Default, Serialize)]
pub struct CodeCollection {
    pub candidates: Vec<CodeCandidate>,
    pub readable_directories: usize,
    pub errors: Vec<CollectionError>,
}

/// Compatibility parser: only `key_` filenames, only ASCII decimal underscore-delimited
/// tokens, range `1..=u32::MAX`. Every numeric token is a *loose* candidate, never a key.
fn parse_codes(name: &str) -> BTreeSet<u32> {
    let Some(name) = name.strip_prefix("key_") else {
        return BTreeSet::new();
    };
    let name = name.strip_suffix(".statistic").unwrap_or(name);
    name.split('_')
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .filter_map(|s| s.parse::<u32>().ok())
        .filter(|n| *n != 0)
        .collect()
}

/// Collect deterministically from explicit directories, preserving all distinct sources.
/// A partially readable collection retains both usable candidates and I/O diagnostics.
pub fn collect_codes(directories: &[PathBuf]) -> CodeCollection {
    let mut result = CodeCollection::default();
    let mut candidates = BTreeMap::<u32, BTreeSet<CandidateSource>>::new();
    let directories: BTreeSet<_> = directories.iter().collect();
    for dir in directories {
        let entries = match std::fs::read_dir(dir) {
            Ok(entries) => {
                result.readable_directories += 1;
                entries
            }
            Err(e) => {
                result.errors.push(CollectionError {
                    path: dir.clone(),
                    kind: format!("{:?}", e.kind()),
                });
                continue;
            }
        };
        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    result.errors.push(CollectionError {
                        path: dir.clone(),
                        kind: format!("{:?}", e.kind()),
                    });
                    continue;
                }
            };
            match entry.file_type() {
                Ok(t) if t.is_file() => {
                    let name = entry.file_name();
                    let Some(name) = name.to_str() else { continue };
                    for code in parse_codes(name) {
                        candidates.entry(code).or_default().insert(CandidateSource {
                            path: entry.path(),
                            parsing: "loose_decimal_tokens",
                        });
                    }
                }
                Err(e) => result.errors.push(CollectionError {
                    path: entry.path(),
                    kind: format!("{:?}", e.kind()),
                }),
                _ => {}
            }
        }
    }
    result
        .errors
        .sort_by(|a, b| a.path.cmp(&b.path).then_with(|| a.kind.cmp(&b.kind)));
    result.candidates = candidates
        .into_iter()
        .map(|(code, sources)| CodeCandidate {
            code,
            sources: sources.into_iter().collect(),
        })
        .collect();
    result
}

/// Only identities of the selected account. Sibling account names and `unknown` are excluded.
pub fn account_identities(account: &Path, configured_wxid: Option<&str>) -> BTreeSet<String> {
    configured_wxid
        .into_iter()
        .chain(account.file_name().and_then(|s| s.to_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != "unknown")
        .map(clean_wxid)
        .collect()
}

/// Full JPEG decode plus a JPEG EOI *in the XOR segment* corroborates the derived XOR.
/// Header checks alone, or samples with no XOR tail, cannot verify the pair.
fn verify_pair(path: &Path, aes: &str, xor: u8) -> bool {
    use std::io::Read;
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let mut data = Vec::new();
    if file
        .take(MAX_SAMPLE_BYTES + 1)
        .read_to_end(&mut data)
        .is_err()
        || data.len() as u64 > MAX_SAMPLE_BYTES
        || data.len() < 31
    {
        return false;
    }
    let Some(&[.., x0, x1, x2, x3]) = data.first_chunk::<14>() else {
        return false;
    };
    if crate::decrypt::detect_dat_version(&data) != 2 || i32::from_le_bytes([x0, x1, x2, x3]) < 2 {
        return false;
    }
    let Some(key) = crate::decrypt::parse_aes_key(aes) else {
        return false;
    };
    let Ok(decoded) = crate::decrypt::decrypt_dat(&data, xor, Some(&key)) else {
        return false;
    };
    if decoded.ext != ".jpg" || !decoded.data.ends_with(&[0xff, 0xd9]) {
        return false;
    }
    let mut decoder = jpeg_decoder::Decoder::new(decoded.data.as_slice());
    // Avoid allocating an unbounded pixel buffer from a malformed sample's dimensions.
    decoder.set_max_decoding_buffer_size(64 * 1024 * 1024);
    decoder.decode().is_ok_and(|pixels| !pixels.is_empty())
}

pub fn acquire_image_keys(
    directories: &[PathBuf],
    account: &Path,
    wxid: Option<&str>,
    budget: usize,
) -> AppResult<crate::contract::ImageKeys> {
    if directories.is_empty() || budget == 0 {
        return Err(AppError::usage(
            "Rust image-key acquisition needs kvcomm directories (none found; pass --kvcomm-dir) and a nonzero scan budget",
        ));
    }
    let collection = collect_codes(directories);
    let collection_diagnostics = json!({
        "readable_directories": collection.readable_directories,
        "candidate_codes": collection.candidates.len(), "errors": collection.errors,
    });
    let fail = |code, message: &str, scan: Option<Value>| {
        AppError::new(code, message, 4)
            .with_details(json!({ "collection": collection_diagnostics, "scan": scan }))
    };
    if collection.readable_directories == 0 {
        return Err(fail(
            "image_key_directory_unreadable",
            "No kvcomm directory could be read",
            None,
        ));
    }
    if collection.candidates.is_empty() {
        return Err(fail(
            "image_key_no_candidates",
            "No candidate code found in the kvcomm filenames",
            None,
        ));
    }
    let identities = account_identities(account, wxid);
    let scan = scan_templates(account, budget, TEMPLATE_LIMIT);
    let diagnostics =
        serde_json::to_value(&scan).map_err(|err| AppError::runtime(format!("{err:#}")))?;
    if scan.templates.is_empty() {
        return Err(fail(
            "image_key_no_template",
            "No valid V2 thumbnail found in the selected account",
            Some(diagnostics),
        ));
    }
    // Keyed by the *derived pair*, not by code/wxid combinations or enumeration order.
    let mut matches = BTreeMap::<(u8, String), (bool, BTreeSet<CandidateSource>)>::new();
    for identity in identities {
        for candidate in &collection.candidates {
            let (xor, aes) = derive_image_keys(candidate.code.into(), &identity);
            let templates: Vec<_> = scan
                .templates
                .iter()
                .filter(|t| verify_derived_aes_key(&aes, &t.cipher))
                .collect();
            if templates.is_empty() {
                continue;
            }
            let verified = templates
                .iter()
                .any(|t| t.tail_xor == Some(xor) && verify_pair(&t.path, &aes, xor));
            let entry = matches.entry((xor, aes)).or_default();
            entry.0 |= verified;
            entry.1.extend(candidate.sources.iter().cloned());
        }
    }
    if matches.is_empty() {
        return Err(fail(
            "image_key_verification_failed",
            "The cached codes do not match the selected account's V2 samples",
            Some(diagnostics),
        ));
    }
    // A pair that also passed full JPEG verification outranks AES-header-only coincidences.
    if matches.len() > 1 && matches.values().filter(|m| m.0).count() == 1 {
        matches.retain(|_, m| m.0);
    }
    let mut pairs = matches.into_iter();
    let (Some(((xor, aes), (verified, sources))), None) = (pairs.next(), pairs.next()) else {
        return Err(fail(
            "image_key_ambiguous",
            "Multiple distinct image-key pairs match the selected account; no pair was chosen",
            Some(diagnostics),
        ));
    };
    Ok(crate::contract::ImageKeys {
        image_xor_key: Some(xor),
        image_aes_key: Some(aes),
        verified,
        aes_verified: true,
        xor_verified: verified,
        verification: if verified {
            "jpeg_decode_and_xor_tail"
        } else {
            "aes_header_only"
        }
        .into(),
        sources: sources.into_iter().collect(),
        collection: collection_diagnostics,
        scan: diagnostics,
        saved: Vec::new(),
    })
}

/// The `xwechat` directories of Linux WeChat under the home directory.
const LINUX_XWECHAT_ROOTS: [&str; 3] = [
    ".xwechat",
    "Documents/WeChat_Data/home/.xwechat",
    "Documents/.xwechat",
];

/// The directory of macOS WeChat 4 under the home directory that holds the `net*/kvcomm` directories (#869).
const MACOS_APP_DATA: &str = "Library/Containers/com.tencent.xinWeChat/Data/Documents/app_data";

/// Whether this platform has a known `kvcomm` location, so `key image` can run without `--kvcomm-dir`.
pub const HAS_DEFAULT_KVCOMM_DIRS: bool = cfg!(any(
    windows,
    target_os = "macos",
    all(target_os = "linux", target_arch = "x86_64")
));

/// The existing `net*/kvcomm` directories (`net`, `net_1`, ...) WeChat keeps.
///
/// They are on Windows under `%APPDATA%\Tencent\xwechat`, on macOS under WeChat's `app_data` in its container, on
/// Linux x64 under the `xwechat` directories in the home directory. The home directory is the caller's under `sudo`
/// (#861). Other platforms have no known location, so callers must pass directories
/// explicitly.
pub fn default_kvcomm_dirs() -> Vec<PathBuf> {
    if cfg!(windows) {
        let Some(appdata) = std::env::var_os("APPDATA") else {
            return Vec::new();
        };
        return net_kvcomm_dirs(&Path::new(&appdata).join("Tencent").join("xwechat"));
    }
    if !HAS_DEFAULT_KVCOMM_DIRS {
        return Vec::new();
    }
    let Some(home) = crate::sudo_caller::home_dir() else {
        return Vec::new();
    };
    if cfg!(target_os = "macos") {
        return net_kvcomm_dirs(&home.join(MACOS_APP_DATA));
    }
    let dirs: BTreeSet<_> = LINUX_XWECHAT_ROOTS
        .iter()
        .flat_map(|root| net_kvcomm_dirs(&home.join(root)))
        .collect();
    dirs.into_iter().collect()
}

/// The `net*/kvcomm` directories under `xwechat`, sorted.
fn net_kvcomm_dirs(xwechat: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(xwechat) else {
        return Vec::new();
    };
    let mut dirs: Vec<_> = entries
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().starts_with("net"))
        .map(|e| e.path().join("kvcomm"))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// How many first blocks of distinct thumbnails `key image-mem` tries every window of WeChat's memory on when no
/// ciphertext is given; each one adds a decryption per window.
pub const MEMORY_SCAN_CIPHERTEXTS: usize = 3;

/// The newest V2 thumbnails of `account`, newest first: their first blocks are what `key image-mem` searches WeChat's
/// memory with, and every key it finds is checked against all of them.
///
/// # Errors
///
/// `image_key_no_template` when the account has no V2 thumbnail within the scan budget.
pub fn memory_samples(account: &Path) -> AppResult<Vec<crate::keys::ImageTemplate>> {
    let scan = scan_templates(account, DEFAULT_SCAN_BUDGET, TEMPLATE_LIMIT);
    if scan.templates.is_empty() {
        let diagnostics = serde_json::to_value(&scan).unwrap_or(Value::Null);
        return Err(AppError::new(
            "image_key_no_template",
            "No valid V2 thumbnail found in the selected account; open a few images in WeChat first",
            4,
        )
        .with_details(json!({ "scan": diagnostics })));
    }
    Ok(scan.templates)
}

/// The first blocks `key image-mem` searches WeChat's memory with: those of the newest [`MEMORY_SCAN_CIPHERTEXTS`]
/// distinct thumbnails among `samples`.
pub fn memory_ciphertexts(samples: &[crate::keys::ImageTemplate]) -> Vec<[u8; 16]> {
    let mut seen = BTreeSet::new();
    samples
        .iter()
        .map(|sample| sample.cipher)
        .filter(|cipher| seen.insert(*cipher))
        .take(MEMORY_SCAN_CIPHERTEXTS)
        .collect()
}

/// How a key found in WeChat's memory was confirmed against the account's thumbnails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoryKeyCheck {
    /// A thumbnail decodes as a whole JPEG with the key and this XOR key, read from its tail.
    DecodedWithXor(u8),
    /// The first blocks of this many distinct thumbnails (two or more) decrypt into image headers.
    FirstBlocks(usize),
}

/// Whether `key`, a candidate found in WeChat's memory, is the account's image AES key.
///
/// It is when a thumbnail of `samples` decodes as a whole JPEG with it and the XOR key its tail gives, or the first
/// blocks of at least two distinct
/// thumbnails decrypt into image headers. A random window of memory passes the header check of one block often
/// enough over a whole scan to need either of these.
pub fn check_memory_key(
    samples: &[crate::keys::ImageTemplate],
    key: &[u8; 16],
) -> Option<MemoryKeyCheck> {
    let matched: Vec<_> = samples
        .iter()
        .filter(|sample| weflow_native::image_key_scan::decrypts_to_image(key, &sample.cipher))
        .collect();
    let hex = weflow_native::image_key_scan::hex(key);
    if let Some(xor) = matched.iter().find_map(|sample| {
        let xor = sample.tail_xor?;
        verify_pair(&sample.path, &hex, xor).then_some(xor)
    }) {
        return Some(MemoryKeyCheck::DecodedWithXor(xor));
    }
    let distinct: BTreeSet<_> = matched.iter().map(|sample| sample.cipher).collect();
    (distinct.len() >= 2).then_some(MemoryKeyCheck::FirstBlocks(distinct.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::ImageTemplate;

    fn sample(cipher: u8) -> ImageTemplate {
        ImageTemplate {
            path: PathBuf::from(format!("{cipher}_t.dat")),
            cipher: [cipher; 16],
            tail_xor: None,
            modified: None,
        }
    }

    #[test]
    fn net_kvcomm_dirs_lists_each_net_directory_with_a_kvcomm() {
        let dir = weflow_native::fixture::TestDir::new("weflow-kvcomm-dirs");
        for name in ["net_1/kvcomm", "net/kvcomm", "other/kvcomm", "net_2"] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
        }
        assert_eq!(
            net_kvcomm_dirs(&dir),
            [
                dir.join("net").join("kvcomm"),
                dir.join("net_1").join("kvcomm")
            ]
        );
        assert!(net_kvcomm_dirs(&dir.join("missing")).is_empty());
    }

    #[test]
    fn memory_ciphertexts_are_a_few_distinct_first_blocks_newest_first() {
        let samples = [sample(1), sample(1), sample(2), sample(3), sample(4)];
        assert_eq!(
            memory_ciphertexts(&samples),
            [[1; 16], [2; 16], [3; 16]],
            "duplicates skipped, at most {MEMORY_SCAN_CIPHERTEXTS}"
        );
    }
}
