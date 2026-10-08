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

#[derive(Debug, Clone, Serialize, PartialEq, Eq, PartialOrd, Ord)]
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
/// tokens, range 1..=u32::MAX. Every numeric token is a *loose* candidate, never a key.
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
    if crate::decrypt::detect_dat_version(&data) != 2
        || i32::from_le_bytes(data[10..14].try_into().unwrap()) < 2
    {
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
) -> AppResult<Value> {
    if directories.is_empty() || budget == 0 {
        return Err(AppError::usage(
            "Rust image-key acquisition needs kvcomm directories and a nonzero scan budget",
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
    let diagnostics = serde_json::to_value(&scan).unwrap();
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
    if matches.len() != 1 {
        return Err(fail(
            "image_key_ambiguous",
            "Multiple distinct image-key pairs match the selected account; no pair was chosen",
            Some(diagnostics),
        ));
    }
    let ((xor, aes), (verified, sources)) = matches.into_iter().next().unwrap();
    Ok(json!({
        "image_xor_key": xor, "image_aes_key": aes, "method": "rust_kvcomm",
        "verified": verified, "aes_verified": true, "xor_verified": verified,
        "verification": if verified { "jpeg_decode_and_xor_tail" } else { "aes_header_only" },
        "sources": sources, "collection": collection_diagnostics, "scan": diagnostics,
    }))
}
