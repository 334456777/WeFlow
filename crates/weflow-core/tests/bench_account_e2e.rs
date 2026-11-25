//! The benchmark account (#488) at a small size: the same bytes from the same seed, every message read back (those
//! still in a WAL too), and its pictures readable; its profile (#605) read back from a file as it was written.
#![expect(clippy::unwrap_used, reason = "test code fails the test by panicking")]
mod common;

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;
use weflow_core::decrypt::{decrypt_dat, parse_aes_key};
use weflow_core::export_msg::names::DisplayPref;
use weflow_core::services::MessageExportRequest;
use weflow_native::db::hex;
use weflow_native::fixture::bench::{self, BenchSpec, Profile};
use weflow_native::sqlcipher::testutil::KEY;

fn small(seed: u64) -> BenchSpec {
    BenchSpec {
        seed,
        group_messages: 3000,
        conversations: 12,
        shards: 3,
        media: true,
        profile: Profile::default(),
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
    // every database has a salt of its own, so each one opened derives its page key (#1127)
    let dbs: Vec<&[u8]> = fa
        .iter()
        .filter(|(path, _)| path.extension().is_some_and(|e| e == "db"))
        .map(|(_, bytes)| &bytes[..16])
        .collect();
    assert!(dbs.len() > 5, "{} databases", dbs.len());
    assert_eq!(dbs.iter().collect::<HashSet<_>>().len(), dbs.len());

    let c = bench::build(&root.join("c"), &small(8)).unwrap();
    assert_ne!(a["conversations"], c["conversations"]);
    let db = Path::new(bench::ACCOUNT_DIR).join("db_storage/message/message_1.db");
    assert_ne!(fa[&db], files(&root.join("c"))[&db]);
}

#[test]
fn a_profile_read_back_from_its_file_is_the_same_and_builds_the_same_account() {
    let root = common::temp_dir("bench-profile");
    let mut profile = Profile::default();
    // a calibrated profile is made of fractions that do not print short
    profile.text.with_emoji = 1.0 / 3.0;
    profile.groups.speaker_s = 4.0 / 7.0 + 3.0;
    let file = root.join("profile.json");
    std::fs::write(&file, serde_json::to_vec_pretty(&profile).unwrap()).unwrap();
    let read = Profile::load(&file).unwrap();
    assert_eq!(read.hash().unwrap(), profile.hash().unwrap());
    assert_ne!(read.hash().unwrap(), Profile::default().hash().unwrap());

    let spec = |profile: Profile| BenchSpec {
        profile,
        media: false,
        ..small(3)
    };
    let a = bench::build(&root.join("a"), &spec(profile)).unwrap();
    let b = bench::build(&root.join("b"), &spec(read)).unwrap();
    assert_eq!(a, b);
    assert_eq!(a["profile"]["hash"], b["profile"]["hash"]);
    let c = bench::build(&root.join("c"), &spec(Profile::default())).unwrap();
    assert_ne!(a["profile"]["hash"], c["profile"]["hash"]);
}

#[test]
fn a_profile_the_generator_cannot_use_is_refused() {
    let root = common::temp_dir("bench-profile-bad");
    assert!(Profile::default().check().is_ok());
    let mut bad = Profile::default();
    bad.messages.sent_in_group = 1.5;
    assert!(bad.check().is_err());
    let mut bad = Profile::default();
    bad.time.hours = [0.0; 24];
    assert!(bad.check().is_err());
    let mut bad = Profile::default();
    bad.groups.big_speakers = bad.groups.big_members + 1;
    assert!(bad.check().is_err());
    let file = root.join("bad.json");
    std::fs::write(&file, serde_json::to_vec(&bad).unwrap()).unwrap();
    assert!(Profile::load(&file).is_err());
    assert!(bench::build(
        &root.join("a"),
        &BenchSpec {
            profile: bad,
            media: false,
            ..small(3)
        }
    )
    .is_err());
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
        // as `chat messages` answers it without `--parsed`
        let page = serde_json::json!(hub
            .message_page_rows(session, 500, 0, before.as_deref(), None)
            .unwrap()
            .into_iter()
            .map(weflow_core::contract::MessageRow::from)
            .collect::<Vec<_>>());
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
fn the_official_accounts_have_messages_and_wechat_pay_its_notices() {
    let root = common::temp_dir("bench-official");
    let data = root.join("data");
    let made = bench::build(
        &data,
        &BenchSpec {
            media: false,
            ..small(4)
        },
    )
    .unwrap();
    let hub = common::hub_for_account(
        &root,
        &bench::account_dir(&data),
        &hex(&KEY),
        bench::ACCOUNT_DIR,
        |_| {},
    );
    let official: Vec<&Value> = made["conversations"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["kind"] == "official")
        .collect();
    let listed = hub.biz_accounts().unwrap()["accounts"]
        .as_array()
        .unwrap()
        .len();
    assert_eq!(listed, official.len());
    for c in &official {
        let name = c["username"].as_str().unwrap();
        let read = hub.biz_messages(name, 0, 0).unwrap().messages.len();
        assert!(read > 0, "{name}");
        assert_eq!(read as u64, c["messages"].as_u64().unwrap(), "{name}");
    }
    let pay = hub.biz_pay_records(0, 0).unwrap().records;
    assert!(!pay.is_empty());
    assert!(pay.iter().any(|r| r.method.is_some() && r.status.is_some()));
    assert!(pay.iter().any(|r| r.method.is_none()));
}

/// The text between the first `<{name}>` of `xml` and the `</{name}>` after it.
fn element<'x>(xml: &'x str, name: &str) -> Option<&'x str> {
    let start = xml.find(&format!("<{name}>"))? + name.len() + 2;
    Some(&xml[start..start + xml[start..].find(&format!("</{name}>"))?])
}

/// What one conversation's messages are, in time order: (create time, local type, `appMsgKind`). An official
/// account's article is a `link` here: the chat window calls it `official-link` on purpose.
fn kinds(messages: &Value) -> Vec<(i64, i64, Option<String>)> {
    let mut out: Vec<_> = messages
        .as_array()
        .unwrap()
        .iter()
        .map(|m| {
            (
                m["createTime"].as_i64().unwrap(),
                m["localType"].as_i64().unwrap(),
                m.get("appMsgKind")
                    .and_then(Value::as_str)
                    .map(|k| if k == "official-link" { "link" } else { k }.to_string()),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
fn replies_quote_xml_escaped_once_and_records_keep_every_item() {
    let root = common::temp_dir("bench-quotes");
    let data = root.join("data");
    let made = bench::build(
        &data,
        &BenchSpec {
            group_messages: 12_000,
            media: false,
            ..small(5)
        },
    )
    .unwrap();
    let hub = common::hub_for_account(
        &root,
        &bench::account_dir(&data),
        &hex(&KEY),
        bench::ACCOUNT_DIR,
        |_| {},
    );
    let (mut plain, mut escaped, mut items) = (0, 0, BTreeMap::<String, usize>::new());
    let (mut counts, mut exported) = (Vec::new(), Vec::new());
    for c in made["conversations"].as_array().unwrap() {
        if c["messages"].as_u64().unwrap() == 0 || c["kind"] == "official" {
            continue;
        }
        let session = c["username"].as_str().unwrap();
        let rows = hub
            .message_page_rows(session, 100_000, 0, None, None)
            .unwrap();
        for row in &rows {
            let row = serde_json::json!(weflow_core::contract::MessageRow::from(row.clone()));
            if row["localType"].as_i64().unwrap() & 0xFFFF_FFFF != 49 {
                continue;
            }
            let xml = row["content"].as_str().unwrap();
            match element(xml, "type") {
                Some("57") => {
                    let quoted = element(element(xml, "refermsg").unwrap(), "content").unwrap();
                    assert!(!quoted.contains('<'), "a quote is escaped: {quoted:.80}");
                    if quoted.contains("&lt;msg") {
                        escaped += 1;
                    } else {
                        plain += 1;
                    }
                }
                Some("19" | "24") => {
                    let record = element(xml, "recorditem").unwrap();
                    let count: usize = record
                        .split("<datalist count=\"")
                        .nth(1)
                        .and_then(|s| s.split('"').next())
                        .unwrap()
                        .parse()
                        .unwrap();
                    let ids: HashSet<&str> = record
                        .split(" dataid=\"")
                        .skip(1)
                        .map(|s| s.split('"').next().unwrap())
                        .collect();
                    assert_eq!(ids.len(), count, "every item has a dataid of its own");
                    for kind in record.split("<dataitem datatype=\"").skip(1) {
                        *items.entry(kind[..1].to_string()).or_default() += 1;
                    }
                    // a note (24) is not a record the exports list the items of
                    if element(xml, "type") == Some("19") {
                        counts.push(count);
                    }
                }
                _ => {}
            }
        }
        // the chat window and the Arkme export tell every app message the same kind, a reply's whatever it quotes
        let parsed = kinds(&serde_json::json!(hub.parsed_rows(rows, session)));
        let out = root.join(format!("{session}.arkme.json"));
        hub.export_messages(&request(session, "arkme-json"), &out)
            .unwrap();
        let arkme: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        let arkme = kinds(&arkme["messages"]);
        assert_eq!(parsed.len(), arkme.len(), "{session}");
        for (p, a) in parsed.iter().zip(&arkme) {
            assert_eq!(p, a, "{session}");
        }
        // the ChatLab export lists every item of a record
        let out = root.join(format!("{session}.chatlab.json"));
        hub.export_messages(&request(session, "chatlab"), &out)
            .unwrap();
        let chatlab: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
        exported.extend(
            chatlab["messages"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|m| m.get("chatRecords").and_then(Value::as_array).map(Vec::len)),
        );
    }
    // replies quote both ways, about as often as on a real account (3 of 5 by their text)
    assert!(plain > 0 && escaped > 0, "{plain} plain, {escaped} escaped");
    let share = f64::from(escaped) / f64::from(plain + escaped);
    assert!((0.25..0.55).contains(&share), "{share}");
    // records hold pictures and videos besides texts
    assert!(!counts.is_empty());
    assert!(
        ["1", "2", "4"]
            .iter()
            .all(|k| items.get(*k).is_some_and(|n| *n > 0)),
        "{items:?}"
    );
    counts.sort_unstable();
    exported.sort_unstable();
    assert_eq!(counts, exported);
}

fn request(session: &str, format: &str) -> MessageExportRequest {
    MessageExportRequest {
        session_id: session.into(),
        format: format.into(),
        start: None,
        end: None,
        sender: None,
        display_pref: DisplayPref::Remark,
        excel_compact: false,
    }
}

#[test]
fn the_pictures_decrypt_and_the_wxgf_ones_decode() {
    let root = common::temp_dir("bench-images");
    let data = root.join("data");
    bench::build(&data, &small(5)).unwrap();
    let aes = parse_aes_key(bench::IMAGE_AES_KEY).unwrap();
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut largest_jpeg = 0;
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
            largest_jpeg = largest_jpeg.max(decoded.map_or(0, |j| j.len()));
        }
        *formats.entry(plain.ext).or_default() += 1;
    }
    for ext in [".jpg", ".png", ".wxgf"] {
        assert!(formats.get(ext).is_some_and(|n| *n > 0), "{formats:?}");
    }
    assert_eq!(formats.len(), 3, "{formats:?}");
    // whole pictures, as a real account's are (#1221)
    assert!(largest_jpeg > 100_000, "{largest_jpeg} bytes");
}
