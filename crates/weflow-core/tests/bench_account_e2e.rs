//! The benchmark account (#488) at a small size: the same bytes from the same seed, every message read back (those
//! still in a WAL too), and its pictures readable.
#![expect(clippy::unwrap_used, reason = "test code fails the test by panicking")]
mod common;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;
use weflow_core::decrypt::{decrypt_dat, parse_aes_key};
use weflow_native::fixture::bench::{self, BenchSpec};
use weflow_native::native_db::hex;
use weflow_native::sqlcipher::testutil::KEY;

fn small(seed: u64) -> BenchSpec {
    BenchSpec {
        seed,
        group_messages: 3000,
        conversations: 12,
        shards: 3,
        media: true,
    }
}

/// Every file under `dir`, by its path below it.
fn files(dir: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<PathBuf, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read(&path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

#[test]
fn a_seed_gives_the_same_bytes_and_another_seed_other_ones() {
    let root = common::temp_dir("bench-seed");
    let a = bench::build(&root.join("a"), &small(7)).unwrap();
    let b = bench::build(&root.join("b"), &small(7)).unwrap();
    assert_eq!(a, b);
    let (fa, fb) = (files(&root.join("a")), files(&root.join("b")));
    assert!(fa.len() > 100, "{} files", fa.len());
    assert!(fa == fb, "the same seed wrote different files");

    let c = bench::build(&root.join("c"), &small(8)).unwrap();
    assert_ne!(a["conversations"], c["conversations"]);
    let db = Path::new(bench::ACCOUNT_DIR).join("db_storage/message/message_1.db");
    assert_ne!(fa[&db], files(&root.join("c"))[&db]);
}

#[test]
fn every_message_is_read_back_with_the_ones_in_the_wal() {
    let root = common::temp_dir("bench-read");
    let data = root.join("data");
    let made = bench::build(&data, &small(3)).unwrap();
    assert!(made["messages_in_wal"].as_u64().unwrap() > 0);
    let hub = common::hub_for_account(
        &root,
        &bench::account_dir(&data),
        &hex(&KEY),
        bench::ACCOUNT_DIR,
        |_| {},
    );

    let conversations: Vec<&Value> = made["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["messages"].as_u64().unwrap() > 0)
        .collect();
    let names: Vec<String> = conversations
        .iter()
        .map(|c| c["username"].as_str().unwrap().to_string())
        .collect();
    let counts = hub.chat_counts(&names).unwrap();
    for c in &conversations {
        let name = c["username"].as_str().unwrap();
        assert_eq!(
            counts[name],
            c["messages"].as_i64().unwrap(),
            "{} {name}",
            c["kind"]
        );
    }

    // the biggest group is in several databases, and the newest of its messages in a WAL
    let big = conversations[0];
    let in_shards = big["per_shard"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|n| n.as_u64().unwrap() > 0)
        .count();
    assert!(in_shards > 1);
    assert!(big["in_wal"].as_u64().unwrap() > 0);
    // paged through: every message once
    let session = big["username"].as_str().unwrap();
    let (mut seen, mut before) = (HashSet::new(), None::<String>);
    loop {
        let page = hub
            .message_page(session, 500, 0, before.as_deref(), None)
            .unwrap();
        let rows = page.as_array().unwrap();
        let Some(tail) = rows.last() else { break };
        for r in rows {
            seen.insert(r["_cursor"].as_str().unwrap().to_string());
        }
        before = tail["_cursor"].as_str().map(String::from);
    }
    assert_eq!(seen.len() as u64, big["messages"].as_u64().unwrap());
}

#[test]
fn the_pictures_decrypt_and_the_wxgf_ones_decode() {
    let root = common::temp_dir("bench-images");
    let data = root.join("data");
    bench::build(&data, &small(5)).unwrap();
    let aes = parse_aes_key(bench::IMAGE_AES_KEY).unwrap();
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    for (path, bytes) in files(&bench::account_dir(&data).join("msg/attach")) {
        assert_eq!(
            path.extension().and_then(|e| e.to_str()),
            Some("dat"),
            "{}",
            path.display()
        );
        let plain = decrypt_dat(&bytes, bench::IMAGE_XOR_KEY, Some(&aes)).unwrap();
        if plain.is_wxgf {
            let decoded = weflow_core::image::wxgf_hevc_candidates(&plain.data)
                .iter()
                .find_map(|(_, stream)| weflow_core::hevc::to_jpeg(stream));
            assert!(decoded.is_some(), "{} does not decode", path.display());
        }
        *formats.entry(plain.ext).or_default() += 1;
    }
    for ext in [".jpg", ".png", ".wxgf"] {
        assert!(formats.get(ext).is_some_and(|n| *n > 0), "{formats:?}");
    }
    assert_eq!(formats.len(), 3, "{formats:?}");
}
