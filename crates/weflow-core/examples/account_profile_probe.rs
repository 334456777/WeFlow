//! Aggregate profile of a configured account, the numbers the synthetic benchmark account copies (#488, #605).
//! usage: account_profile_probe <config.json> [--calibrate <profile.json>]
//! Prints one JSON object of counts, proportions and quantiles. No conversation, user name, message content, key or
//! local path is printed: message types are WeChat's numeric codes, everything else is counted. The config is only
//! read.
//!
//! With `--calibrate`, prints nothing of the account: the numbers become a benchmark profile
//! (`weflow_native::fixture::bench::Profile`) that replaces the one in that file when some part of it moved past
//! [`SHARE_MOVED`] or [`SIZE_MOVED`] (or when the file has none), and only whether it did, how many parts moved and the
//! profile's hash are printed. The parts the account says nothing of keep the default profile's values.
#![expect(
    clippy::cast_possible_truncation,
    reason = "probe and benchmark data are small values that fit"
)]

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use weflow_core::config::{expand_home, resolve_account_dir, ConfigStore};
use weflow_core::decrypt::{decrypt_dat, parse_aes_key};
use weflow_native::fixture::bench::{Profile, Spread};
use weflow_native::native_db::NativeAccount;
use weflow_native::native_msg::{table_name_for, MsgTable};

/// Offset of the account's clock from UTC, for hours and weekdays (China Standard Time).
const TZ: i64 = 8 * 3600;
/// Image files decrypted to see what they hold, at most.
const DAT_SAMPLES: usize = 1500;
/// Groups with fewer messages say little about who speaks.
const GROUP_MIN_MESSAGES: usize = 200;
const TEXT_BUCKETS: &[i64] = &[0, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 5000];
const GAP_BUCKETS: &[i64] = &[0, 5, 60, 600, 3600, 86_400, 7 * 86_400, 30 * 86_400];
/// A share (a number in `0..=1`) that moved more than this makes a new profile.
const SHARE_MOVED: f64 = 0.02;
/// Any other number (a size, a count, a fitted exponent) that moved more than this share of itself makes one.
const SIZE_MOVED: f64 = 0.25;
/// The largest video and file the synthetic account writes, as the default profile has it.
const MEDIA_MAX: f64 = 8.0 * 1024.0 * 1024.0;

fn quantiles(mut v: Vec<i64>) -> Value {
    if v.is_empty() {
        return json!({"n": 0});
    }
    v.sort_unstable();
    let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
    json!({"n": v.len(), "sum": v.iter().sum::<i64>(), "min": v[0], "p25": at(0.25), "p50": at(0.5),
        "p75": at(0.75), "p90": at(0.9), "p99": at(0.99), "max": v[v.len() - 1]})
}

/// Counts of `values` by the bucket `[edge, next edge)` they fall in, keyed by the lower edge.
fn histogram(values: &[i64], edges: &[i64]) -> Value {
    let mut counts = vec![0u64; edges.len()];
    for v in values {
        let i = edges.iter().rposition(|e| v >= e).unwrap_or(0);
        counts[i] += 1;
    }
    let total = values.len().max(1) as f64;
    Value::Array(
        edges
            .iter()
            .zip(counts)
            .map(|(e, n)| json!({"from": e, "n": n, "share": n as f64 / total}))
            .collect(),
    )
}

fn shares<K: Ord + ToString>(counts: &BTreeMap<K, u64>) -> Value {
    let total = counts.values().sum::<u64>().max(1) as f64;
    let mut out = Map::new();
    for (k, n) in counts {
        out.insert(k.to_string(), json!({"n": n, "share": *n as f64 / total}));
    }
    Value::Object(out)
}

fn kind_of(username: &str) -> &'static str {
    if username.ends_with("@chatroom") {
        "group"
    } else if username.starts_with("gh_") {
        "official"
    } else if username.contains('@') {
        "other"
    } else {
        "private"
    }
}

fn is_emoji(c: char) -> bool {
    matches!(u32::from(c), 0x1F000..=0x1FAFF | 0x2600..=0x27BF)
}

fn is_odd_control(c: char) -> bool {
    (c.is_control() && c != '\n' && c != '\t')
        || matches!(u32::from(c), 0x200B..=0x200F | 0x2028 | 0x2029 | 0xFEFF)
}

/// The text a group message says, without the `sender:\n` WeChat writes before it.
fn group_text(content: &str) -> &str {
    match content.split_once(":\n") {
        Some((who, rest)) if who.len() < 64 && !who.contains(char::is_whitespace) => rest,
        _ => content,
    }
}

/// The bytes of a column and how they are stored: text, a zstd blob (unpacked here), another blob, or nothing.
fn column_bytes(value: rusqlite::types::ValueRef<'_>) -> (Vec<u8>, &'static str) {
    match value {
        rusqlite::types::ValueRef::Text(t) => (t.to_vec(), "text"),
        rusqlite::types::ValueRef::Blob(b) => match zstd::decode_all(b) {
            Ok(plain) => (plain, "zstd"),
            Err(_) => (b.to_vec(), "blob"),
        },
        rusqlite::types::ValueRef::Null => (Vec::new(), "null"),
        _ => (Vec::new(), "number"),
    }
}

fn stored_len(value: rusqlite::types::ValueRef<'_>) -> u64 {
    match value {
        rusqlite::types::ValueRef::Text(b) | rusqlite::types::ValueRef::Blob(b) => b.len() as u64,
        _ => 0,
    }
}

#[derive(Default)]
struct Text {
    lengths: Vec<i64>,
    emoji: u64,
    bracket_emoji: u64,
    newline: u64,
    control: u64,
    url: u64,
}

impl Text {
    fn add(&mut self, text: &str) {
        self.lengths.push(text.chars().count() as i64);
        self.emoji += u64::from(text.chars().any(is_emoji));
        self.bracket_emoji += u64::from(
            text.match_indices('[')
                .any(|(i, _)| text[i..].chars().take(6).any(|c| c == ']')),
        );
        self.newline += u64::from(text.contains('\n'));
        self.control += u64::from(text.chars().any(is_odd_control));
        self.url += u64::from(text.contains("http://") || text.contains("https://"));
    }

    fn report(self) -> Value {
        let n = self.lengths.len().max(1) as f64;
        json!({"chars": quantiles(self.lengths.clone()), "chars_histogram": histogram(&self.lengths, TEXT_BUCKETS),
            "with_emoji": self.emoji as f64 / n, "with_bracket_emoji": self.bracket_emoji as f64 / n,
            "with_newline": self.newline as f64 / n, "with_odd_control": self.control as f64 / n,
            "with_url": self.url as f64 / n})
    }
}

/// One conversation's messages over every shard.
#[derive(Default)]
struct Conversation {
    times: Vec<i64>,
    /// (local id, shard) of each message.
    ids: Vec<(i64, usize)>,
    /// (first, last) message time in each shard that holds some.
    ranges: BTreeMap<usize, (i64, i64)>,
    senders: HashMap<String, usize>,
}

#[derive(Default)]
struct Messages {
    total: u64,
    types: BTreeMap<i64, u64>,
    /// Messages by "<type> <storage>" of the content, and "source <storage>".
    storage: BTreeMap<String, u64>,
    content_bytes: BTreeMap<i64, Vec<i64>>,
    /// Bytes of content and of source as stored (compressed or not) and as read.
    stored: BTreeMap<&'static str, u64>,
    text: Text,
    sent: u64,
    /// Messages and those sent by the owner, in groups and in private chats.
    group: (u64, u64),
    private: (u64, u64),
    /// Text messages in groups, and those that mention someone.
    group_texts: (u64, u64),
    sort_seq_missing: u64,
    sort_seq_off: u64,
    with_source: u64,
    with_at: u64,
    status: BTreeMap<i64, u64>,
    hours: BTreeMap<i64, u64>,
    weekdays: BTreeMap<i64, u64>,
    days: HashMap<i64, i64>,
    per_shard: BTreeMap<usize, u64>,
}

fn read_table(
    account: &NativeAccount,
    part: &MsgTable,
    shard: usize,
    group: bool,
    m: &mut Messages,
    c: &mut Conversation,
) -> Result<()> {
    let (db, table) = (&part.db, &part.table);
    let names: HashMap<i64, String> = account.with_db(db, |conn| {
        let mut stmt = conn.prepare("select rowid, user_name from Name2Id")?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        Ok(rows.collect::<rusqlite::Result<_>>()?)
    })?;
    let sql = format!(
        "select local_id, local_type, sort_seq, create_time, real_sender_id, status, source, message_content from \"{table}\""
    );
    account.with_db(db, |conn| {
        let mut stmt = conn.prepare(&sql)?;
        let mut rows = stmt.query([])?;
        while let Some(r) = rows.next()? {
            let local_id: i64 = r.get(0)?;
            let kind: i64 = r.get(1)?;
            let sort_seq: Option<i64> = r.get(2)?;
            let time: i64 = r.get::<_, Option<i64>>(3)?.unwrap_or(0);
            let sender = names.get(&r.get::<_, Option<i64>>(4)?.unwrap_or(0));
            *m.stored.entry("source stored").or_default() += stored_len(r.get_ref(6)?);
            let (source, source_storage) = column_bytes(r.get_ref(6)?);
            *m.stored.entry("source read").or_default() += source.len() as u64;
            *m.storage
                .entry(format!("source {source_storage}"))
                .or_default() += 1;
            let source = String::from_utf8_lossy(&source);
            m.total += 1;
            *m.per_shard.entry(shard).or_default() += 1;
            *m.types.entry(kind).or_default() += 1;
            *m.status
                .entry(r.get::<_, Option<i64>>(5)?.unwrap_or(-1))
                .or_default() += 1;
            match sort_seq {
                None | Some(0) => m.sort_seq_missing += 1,
                Some(s) if s / 1000 != time => m.sort_seq_off += 1,
                Some(_) => {}
            }
            if !source.is_empty() {
                m.with_source += 1;
                m.with_at += u64::from(source.contains("atuserlist"));
            }
            let mine = sender.is_some_and(|s| account.is_me(s));
            m.sent += u64::from(mine);
            let by_kind = if group { &mut m.group } else { &mut m.private };
            *by_kind = (by_kind.0 + 1, by_kind.1 + u64::from(mine));
            if group && kind == 1 {
                m.group_texts = (
                    m.group_texts.0 + 1,
                    m.group_texts.1 + u64::from(source.contains("atuserlist")),
                );
            }
            let local = time + TZ;
            *m.hours.entry(local.rem_euclid(86_400) / 3600).or_default() += 1;
            *m.weekdays
                .entry((local.div_euclid(86_400) + 3).rem_euclid(7))
                .or_default() += 1;
            *m.days.entry(local.div_euclid(86_400)).or_default() += 1;
            *m.stored.entry("content stored").or_default() += stored_len(r.get_ref(7)?);
            let (bytes, storage) = column_bytes(r.get_ref(7)?);
            *m.stored.entry("content read").or_default() += bytes.len() as u64;
            *m.storage.entry(format!("{kind} {storage}")).or_default() += 1;
            m.content_bytes
                .entry(kind)
                .or_default()
                .push(bytes.len() as i64);
            if kind == 1 {
                let text = String::from_utf8_lossy(&bytes);
                m.text.add(if group { group_text(&text) } else { &text });
            }
            c.times.push(time);
            c.ids.push((local_id, shard));
            let range = c.ranges.entry(shard).or_insert((time, time));
            *range = (range.0.min(time), range.1.max(time));
            if let Some(s) = sender {
                *c.senders.entry(s.clone()).or_default() += 1;
            }
        }
        Ok(())
    })
}

fn conversations(account: &NativeAccount) -> Result<Value> {
    let shards = account.message_dbs();
    let shard_of: HashMap<PathBuf, usize> = shards
        .iter()
        .enumerate()
        .map(|(i, p)| (p.clone(), i))
        .collect();
    // A table is named after its conversation; every conversation appears in some shard's Name2Id.
    let mut session_of: HashMap<String, String> = HashMap::new();
    for db in &shards {
        for row in account.query(db, "select user_name from Name2Id", &[])? {
            if let Some(name) = row["user_name"].as_str() {
                session_of.insert(table_name_for(name), name.to_string());
            }
        }
    }
    let mut m = Messages::default();
    let mut by_kind: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    let (mut multi_shard, mut overlapping, mut duplicated_ids, mut multi_shard_messages) =
        (0u64, 0u64, 0u64, 0u64);
    let mut gaps = Vec::new();
    let mut groups = Vec::new();
    for (table, parts) in account.message_tables_by_name()? {
        let name = session_of.get(&table).map_or("", String::as_str);
        let kind = kind_of(name);
        let mut c = Conversation::default();
        for part in &parts {
            read_table(
                account,
                part,
                shard_of[&part.db],
                kind == "group",
                &mut m,
                &mut c,
            )?;
        }
        if c.times.is_empty() {
            continue;
        }
        by_kind.entry(kind).or_default().push(c.times.len() as i64);
        if c.ranges.len() > 1 {
            multi_shard += 1;
            multi_shard_messages += c.times.len() as u64;
            let ranges: Vec<_> = c.ranges.values().collect();
            overlapping += u64::from(
                ranges
                    .windows(2)
                    .any(|w| w[1].0 < w[0].1 || w[0].0 > w[1].1),
            );
            let mut shards_of: HashMap<i64, HashSet<usize>> = HashMap::new();
            for (id, shard) in &c.ids {
                shards_of.entry(*id).or_default().insert(*shard);
            }
            duplicated_ids += shards_of.values().filter(|s| s.len() > 1).count() as u64;
        }
        let mut times = c.times.clone();
        times.sort_unstable();
        gaps.extend(times.windows(2).map(|w| w[1] - w[0]));
        if kind == "group" && c.times.len() >= GROUP_MIN_MESSAGES {
            let mut counts: Vec<usize> = c.senders.values().copied().collect();
            counts.sort_unstable_by(|a, b| b.cmp(a));
            let total: usize = counts.iter().sum::<usize>().max(1);
            let top_tenth = counts.len().div_ceil(10);
            groups.push(json!({"messages": c.times.len(), "speakers": counts.len(),
                "top1_share": counts.first().copied().unwrap_or(0) as f64 / total as f64,
                "top10pct_share": counts[..top_tenth].iter().sum::<usize>() as f64 / total as f64}));
        }
    }
    let per_day: Vec<i64> = m.days.values().copied().collect();
    let total = m.total.max(1) as f64;
    let content_bytes: Map<String, Value> = m
        .content_bytes
        .iter()
        .filter(|(_, v)| v.len() >= 20)
        .map(|(k, v)| (k.to_string(), quantiles(v.clone())))
        .collect();
    Ok(json!({
        "messages": m.total,
        "shards": shards.len(),
        "per_shard": shares(&m.per_shard),
        "conversations": by_kind.iter().map(|(k, v)| (k.to_string(), quantiles(v.clone()))).collect::<Map<_, _>>(),
        "multi_shard_conversations": multi_shard,
        "multi_shard_messages": multi_shard_messages,
        "multi_shard_overlapping_times": overlapping,
        "local_ids_in_several_shards": duplicated_ids,
        "types": shares(&m.types),
        "storage_by_type": shares(&m.storage),
        "content_bytes_by_type": content_bytes,
        "bytes": m.stored,
        "text": m.text.report(),
        "sent_share": m.sent as f64 / total,
        "sent_share_group": m.group.1 as f64 / m.group.0.max(1) as f64,
        "sent_share_private": m.private.1 as f64 / m.private.0.max(1) as f64,
        "at_share_group_text": m.group_texts.1 as f64 / m.group_texts.0.max(1) as f64,
        "sort_seq_missing_share": m.sort_seq_missing as f64 / total,
        "sort_seq_not_create_time_share": m.sort_seq_off as f64 / total,
        "with_source_share": m.with_source as f64 / total,
        "with_at_share": m.with_at as f64 / total,
        "status": shares(&m.status),
        "hour_utc8": shares(&m.hours),
        "weekday_monday0": shares(&m.weekdays),
        "messages_per_active_day": quantiles(per_day),
        "gap_seconds": histogram(&gaps, GAP_BUCKETS),
        "groups_speaking": {
            "n": groups.len(),
            "speakers": quantiles(groups.iter().map(|g| g["speakers"].as_i64().unwrap_or(0)).collect()),
            "top1_share_permille": quantiles(groups.iter().map(|g| (g["top1_share"].as_f64().unwrap_or(0.0) * 1000.0) as i64).collect()),
            "top10pct_share_permille": quantiles(groups.iter().map(|g| (g["top10pct_share"].as_f64().unwrap_or(0.0) * 1000.0) as i64).collect()),
        },
    }))
}

fn count(account: &NativeAccount, rel: &str, sql: &str) -> Value {
    let path = account.db_path(rel);
    if !path.is_file() {
        return Value::Null;
    }
    account
        .query(&path, sql, &[])
        .map_or(Value::Null, Value::Array)
}

fn others(account: &NativeAccount) -> Result<Value> {
    let fts_tables = account
        .query(
            &account.db_path("message/message_fts.db"),
            "select name from sqlite_master where type = 'table' and name like 'message_fts_v4_%' and name not like '%\\_%\\_%\\_%\\_%' escape '\\'",
            &[],
        )
        .unwrap_or_default();
    let mut fts_rows = 0i64;
    for t in &fts_tables {
        if let Some(name) = t["name"].as_str() {
            let rows = account.query(
                &account.db_path("message/message_fts.db"),
                &format!("select count(*) as n from \"{name}\""),
                &[],
            )?;
            fts_rows += rows[0]["n"].as_i64().unwrap_or(0);
        }
    }
    let members: Vec<i64> = account
        .query(
            &account.db_path("contact/contact.db"),
            "select count(*) as n from chatroom_member group by room_id",
            &[],
        )?
        .iter()
        .filter_map(|r| r["n"].as_i64())
        .collect();
    let voices: i64 = account
        .media_dbs()
        .iter()
        .map(|db| {
            account
                .query(db, "select count(*) as n from VoiceInfo", &[])
                .ok()
                .and_then(|r| r[0]["n"].as_i64())
                .unwrap_or(0)
        })
        .sum();
    let mut wal = Map::new();
    for entry in std::fs::read_dir(account.db_path("message"))?.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if let Some(db) = name.strip_suffix(".db-wal") {
            let size = entry.metadata()?.len();
            // WAL header 32 bytes, then frames of a 24-byte header and a 4096-byte page.
            wal.insert(db.to_string(), json!(size.saturating_sub(32) / (4096 + 24)));
        }
    }
    Ok(json!({
        "fts_tables": fts_tables.len(),
        "fts_rows": fts_rows,
        "room_members": quantiles(members),
        "voice_rows": voices,
        "wal_frames": wal,
        "sessions": count(account, "session/session.db", "select count(*) as n, sum(unread_count > 0) as unread from SessionTable"),
        "contacts_by_type": count(account, "contact/contact.db", "select local_type, count(*) as n from contact group by local_type"),
        "contacts_with": count(account, "contact/contact.db", "select sum(remark != '') as remark, sum(alias != '') as alias, sum(description != '') as description, count(*) as n from contact"),
        "sns_posts": count(account, "sns/sns.db", "select count(*) as n from SnsTimeLine"),
        "hardlinks": count(account, "hardlink/hardlink.db", "select (select count(*) from image_hardlink_info_v4) as image, (select count(*) from video_hardlink_info_v4) as video, (select count(*) from file_hardlink_info_v4) as file"),
        "emoticons": count(account, "emoticon/emoticon.db", "select count(*) as n from kNonStoreEmoticonTable"),
    }))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out);
        } else {
            out.push(path);
        }
    }
}

fn size(path: &Path) -> i64 {
    std::fs::metadata(path).map_or(0, |m| m.len() as i64)
}

fn media(account_dir: &Path, xor: u8, aes: Option<[u8; 16]>) -> Value {
    let mut files = Vec::new();
    walk(&account_dir.join("msg").join("attach"), &mut files);
    let mut by_folder: BTreeMap<String, u64> = BTreeMap::new();
    let mut variants: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    let mut combos: HashMap<String, Vec<&str>> = HashMap::new();
    let mut dats = Vec::new();
    for f in &files {
        let folder = f
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_default();
        *by_folder.entry(folder.clone()).or_default() += 1;
        let name = f
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().to_string());
        let Some(stem) = name.strip_suffix(".dat") else {
            continue;
        };
        if folder != "Img" {
            continue;
        }
        let (stem, variant) = if let Some(s) = stem.strip_suffix("_t") {
            (s, "thumb")
        } else if let Some(s) = stem.strip_suffix("_h") {
            (s, "hd")
        } else {
            (stem, "normal")
        };
        variants.entry(variant).or_default().push(size(f));
        combos
            .entry(format!("{}", f.with_file_name(stem).display()))
            .or_default()
            .push(variant);
        dats.push((f, variant));
    }
    let mut combo_counts: BTreeMap<String, u64> = BTreeMap::new();
    for mut v in combos.into_values() {
        v.sort_unstable();
        *combo_counts.entry(v.join("+")).or_default() += 1;
    }
    let step = dats.len().div_ceil(DAT_SAMPLES).max(1);
    let mut formats: BTreeMap<String, u64> = BTreeMap::new();
    for (f, variant) in dats.iter().step_by(step) {
        let format = std::fs::read(f)
            .ok()
            .and_then(|d| decrypt_dat(&d, xor, aes.as_ref()).ok())
            .map_or_else(|| "unreadable".to_string(), |r| r.ext);
        *formats.entry(format!("{variant} {format}")).or_default() += 1;
    }
    let mut video = Vec::new();
    walk(&account_dir.join("msg").join("video"), &mut video);
    let mut video_kinds: BTreeMap<&str, Vec<i64>> = BTreeMap::new();
    for f in &video {
        let name = f
            .file_name()
            .map_or(String::new(), |n| n.to_string_lossy().to_string());
        let kind = if name.ends_with("_thumb.jpg") {
            "thumb"
        } else if name.ends_with("_raw.mp4") {
            "raw"
        } else if name.ends_with(".mp4") {
            "mp4"
        } else {
            "other"
        };
        video_kinds.entry(kind).or_default().push(size(f));
    }
    let mut file = Vec::new();
    walk(&account_dir.join("msg").join("file"), &mut file);
    json!({
        "attach_files_by_folder": shares(&by_folder),
        "image_bytes": variants.into_iter().map(|(k, v)| (k.to_string(), quantiles(v))).collect::<Map<_, _>>(),
        "image_file_sets": shares(&combo_counts),
        "image_formats_sampled": shares(&formats),
        "video_bytes": video_kinds.into_iter().map(|(k, v)| (k.to_string(), quantiles(v))).collect::<Map<_, _>>(),
        "file_bytes": quantiles(file.iter().map(|f| size(f)).collect()),
    })
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let calibrate = match args.as_slice() {
        [_] => None,
        [_, flag, file] if flag == "--calibrate" => Some(PathBuf::from(file)),
        _ => {
            anyhow::bail!("usage: account_profile_probe <config.json> [--calibrate <profile.json>]")
        }
    };
    let config = ConfigStore::load(Path::new(&args[0])).context("read the WeFlow config")?;
    let profile = config
        .profiles
        .get(&config.current_profile)
        .context("current profile missing")?;
    let root = expand_home(
        profile
            .db_path
            .as_deref()
            .context("database path missing")?,
    );
    let me = profile.wxid.clone().unwrap_or_default();
    let root = if me.is_empty() {
        root
    } else {
        resolve_account_dir(&root.to_string_lossy(), &me)
    };
    let account = NativeAccount::new(
        root.join("db_storage"),
        profile
            .decrypt_key
            .as_deref()
            .context("database key missing")?,
    )?
    .with_my_wxid(Some(me));
    account
        .test_connection()
        .context("open configured account")?;
    let xor = profile
        .image_xor_key
        .and_then(|k| u8::try_from(k).ok())
        .unwrap_or(0);
    let aes = profile.image_aes_key.as_deref().and_then(parse_aes_key);
    let profile = json!({
        "conversations": conversations(&account)?,
        "databases": others(&account)?,
        "media": media(&root, xor, aes),
    });
    match calibrate {
        None => println!("{}", serde_json::to_string_pretty(&profile)?),
        Some(file) => adopt(&to_profile(&profile), &file)?,
    }
    Ok(())
}

// ---- --calibrate ----

fn num(v: &Value) -> Option<f64> {
    v.as_f64().filter(|x| x.is_finite())
}

/// `(median, 90th percentile, largest)` of quantiles, the largest at most `cap`.
fn spread(q: &Value, cap: f64) -> Option<Spread> {
    let (p50, p90, max) = (num(&q["p50"])?, num(&q["p90"])?, num(&q["max"])?);
    (q["n"].as_u64()? >= 20 && p50 > 0.0).then_some((p50, p90.max(p50), max.max(p90).min(cap)))
}

/// The shares of `shares` (`{"key": {"share": x}}`) over the keys `keys`, renormalized; None when they hold none.
fn picked(shares: &Value, keys: impl Iterator<Item = String>) -> Option<Vec<f64>> {
    let v: Vec<f64> = keys
        .map(|k| num(&shares[&k]["share"]).unwrap_or(0.0))
        .collect();
    let total: f64 = v.iter().sum();
    (total > 0.0).then(|| v.iter().map(|x| x / total).collect())
}

/// `(q, s)` of the speaker weights `(r + 1 + q) ^ -s` that give a group of `speakers` the share of the first speaker
/// and of the first tenth closest to `top1` and `top10`.
fn fit_speakers(speakers: usize, top1: f64, top10: f64) -> (f64, f64) {
    let tenth = speakers.div_ceil(10);
    let mut best = (f64::INFINITY, 0.0, 0.0);
    for qi in 0..=120 {
        let q = f64::from(qi) * 0.5;
        for si in 5..=80 {
            let s = f64::from(si) * 0.1;
            let w: Vec<f64> = (0..speakers)
                .map(|r| (r as f64 + 1.0 + q).powf(-s))
                .collect();
            let total: f64 = w.iter().sum();
            let err = (w[0] / total - top1).powi(2)
                + (w[..tenth].iter().sum::<f64>() / total - top10).powi(2);
            if err < best.0 {
                best = (err, q, s);
            }
        }
    }
    (best.1, best.2)
}

/// The benchmark profile of the account's numbers, from the default one: what the account says nothing of stays.
fn to_profile(p: &Value) -> Profile {
    let mut out = Profile::default();
    let (c, d, m) = (&p["conversations"], &p["databases"], &p["media"]);
    out.measured = (chrono::Utc::now() + chrono::Duration::seconds(TZ))
        .format("%Y-%m-%d")
        .to_string();

    let msg = &mut out.messages;
    if let Some(types) = c["types"].as_object() {
        let mut v: Vec<(i64, f64)> = types
            .iter()
            .filter_map(|(k, x)| Some((k.parse().ok()?, num(&x["share"])?)))
            .filter(|(_, share)| *share >= 0.000_01)
            .collect();
        v.sort_unstable_by_key(|t| t.0);
        if !v.is_empty() {
            msg.types = v;
        }
    }
    let share_of = |key: &str| num(&c[key]).filter(|x| (0.0..=1.0).contains(x));
    for (field, key) in [
        (&mut msg.sent_in_group, "sent_share_group"),
        (&mut msg.sent_in_private, "sent_share_private"),
        (&mut msg.at_in_group, "at_share_group_text"),
        (&mut msg.with_source, "with_source_share"),
        (&mut msg.sort_seq_off, "sort_seq_not_create_time_share"),
        (&mut msg.sort_seq_missing, "sort_seq_missing_share"),
    ] {
        if let Some(x) = share_of(key) {
            *field = x;
        }
    }
    let storage = &c["storage_by_type"];
    let source_n = |kind: &str| {
        storage[format!("source {kind}")]["n"]
            .as_f64()
            .unwrap_or(0.0)
    };
    if source_n("zstd") + source_n("text") > 0.0 {
        msg.source_compressed = source_n("zstd") / (source_n("zstd") + source_n("text"));
    }
    if let Some(sizes) = c["content_bytes_by_type"].as_object() {
        let mut v: Vec<(i64, Spread)> = sizes
            .iter()
            .filter_map(|(k, q)| Some((k.parse().ok()?, spread(q, f64::MAX)?)))
            .filter(|(t, _)| *t != 1)
            .collect();
        v.sort_unstable_by_key(|t| t.0);
        if !v.is_empty() {
            msg.content_bytes = v;
        }
    }

    let time = &mut out.time;
    if let Some(v) = picked(&c["hour_utc8"], (0..24).map(|h| h.to_string())) {
        // every hour keeps a little: the generator divides by an hour's share
        for (h, x) in time.hours.iter_mut().zip(v) {
            *h = x.max(0.0001);
        }
    }
    if let Some(v) = picked(&c["weekday_monday0"], (0..7).map(|d| d.to_string())) {
        time.weekdays.iter_mut().zip(v).for_each(|(w, x)| *w = x);
    }
    if let Some(buckets) = c["gap_seconds"].as_array() {
        // the gaps within a burst: those under an hour
        let v: Vec<(i64, i64, f64)> = buckets
            .windows(2)
            .filter_map(|w| {
                Some((
                    w[0]["from"].as_i64()?,
                    w[1]["from"].as_i64()?,
                    num(&w[0]["share"])?,
                ))
            })
            .filter(|g| g.1 <= 3600)
            .collect();
        let total: f64 = v.iter().map(|g| g.2).sum();
        if total > 0.0 {
            time.burst_gaps = v.into_iter().map(|(a, b, x)| (a, b, x / total)).collect();
        }
    }

    let text = &mut out.text;
    let t = &c["text"];
    if let Some(buckets) = t["chars_histogram"].as_array() {
        let longest = t["chars"]["max"].as_u64().unwrap_or(4500).clamp(1001, 4500) as u32;
        let v: Vec<(u32, u32, f64)> = buckets
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                let from = b["from"].as_u64()?.max(1) as u32;
                let to = buckets
                    .get(i + 1)
                    .and_then(|n| n["from"].as_u64())
                    .map_or(longest, |n| n as u32);
                Some((from, to.max(from + 1), num(&b["share"])?))
            })
            .filter(|r| r.2 > 0.0)
            .collect();
        if !v.is_empty() {
            text.chars = v;
        }
    }
    for (field, key) in [
        (&mut text.with_emoji, "with_emoji"),
        (&mut text.with_bracket_emoji, "with_bracket_emoji"),
        (&mut text.with_newline, "with_newline"),
        (&mut text.with_odd_control, "with_odd_control"),
        (&mut text.with_url, "with_url"),
    ] {
        if let Some(x) = num(&t[key]).filter(|x| (0.0..=1.0).contains(x)) {
            *field = x;
        }
    }

    let groups = &mut out.groups;
    let kinds = &c["conversations"];
    let (g, pr) = (
        kinds["group"]["n"].as_f64().unwrap_or(0.0),
        kinds["private"]["n"].as_f64().unwrap_or(0.0),
    );
    if g + pr > 0.0 {
        groups.share = g / (g + pr);
    }
    let speaking = &c["groups_speaking"];
    if let (Some(speakers), Some(top1), Some(top10)) = (
        speaking["speakers"]["p50"].as_u64(),
        num(&speaking["top1_share_permille"]["p50"]),
        num(&speaking["top10pct_share_permille"]["p50"]),
    ) {
        if speakers >= 10 {
            (groups.speaker_q, groups.speaker_s) =
                fit_speakers(speakers as usize, top1 / 1000.0, top10 / 1000.0);
        }
    }
    if let (Some(members), Some(speakers)) = (
        d["room_members"]["max"].as_u64(),
        speaking["speakers"]["max"].as_u64(),
    ) {
        if speakers >= 1 {
            groups.big_members = members.max(speakers) as usize;
            groups.big_speakers = speakers as usize;
        }
    }

    let media = &mut out.media;
    let sets = &m["image_file_sets"];
    let set_keys = [
        ("thumb", (true, false, false)),
        ("normal+thumb", (true, true, false)),
        ("hd+normal+thumb", (true, true, true)),
        ("normal", (false, true, false)),
    ];
    if let Some(v) = picked(sets, set_keys.iter().map(|(k, _)| (*k).to_string())) {
        media.image_sets = set_keys
            .iter()
            .zip(v)
            .map(|((_, (t, i, h)), x)| (*t, *i, *h, x))
            .collect();
    }
    let images = &m["image_bytes"];
    for (field, key) in [
        (&mut media.thumb_bytes, "thumb"),
        (&mut media.image_bytes, "normal"),
        (&mut media.hd_bytes, "hd"),
    ] {
        if let Some(s) = spread(&images[key], f64::MAX) {
            *field = s;
        }
    }
    let formats = &m["image_formats_sampled"];
    for (field, variant) in [
        (&mut media.image_formats, "normal"),
        (&mut media.hd_formats, "hd"),
    ] {
        if let Some(v) = picked(
            formats,
            [".png", ".jpg", ".wxgf"]
                .iter()
                .map(|e| format!("{variant} {e}")),
        ) {
            *field = (v[0], v[1], v[2]);
        }
    }
    let videos = &m["video_bytes"];
    if let Some(s) = spread(&videos["mp4"], MEDIA_MAX) {
        media.video_bytes = s;
    }
    if let Some(s) = spread(&videos["thumb"], f64::MAX) {
        media.video_thumb_bytes = s;
    }
    if let Some(s) = spread(&m["file_bytes"], MEDIA_MAX) {
        media.file_bytes = s;
    }
    let type_n = |t: i64| c["types"][t.to_string()]["n"].as_f64().unwrap_or(0.0);
    let here = |files: Option<u64>, messages: f64| {
        files
            .filter(|_| messages >= 20.0)
            .map(|f| (f as f64 / messages).min(1.0))
    };
    if let Some(x) = here(videos["thumb"]["n"].as_u64(), type_n(43)) {
        media.video_thumb_here = x;
    }
    if let Some(x) = here(videos["mp4"]["n"].as_u64(), type_n(43)) {
        media.video_here = x;
    }
    if let Some(x) = here(m["file_bytes"]["n"].as_u64(), type_n((6 << 32) | 49)) {
        media.file_here = x;
    }
    out
}

/// How many numbers moved past [`SHARE_MOVED`] or [`SIZE_MOVED`] from `old` to `new`, as JSON; a list of a
/// different length counts as one.
fn moved(old: &Value, new: &Value) -> usize {
    match (old, new) {
        (Value::Object(a), Value::Object(b)) => b
            .iter()
            .map(|(k, v)| a.get(k).map_or(1, |o| moved(o, v)))
            .sum(),
        (Value::Array(a), Value::Array(b)) if a.len() == b.len() => {
            a.iter().zip(b).map(|(x, y)| moved(x, y)).sum()
        }
        (Value::Number(a), Value::Number(b)) => {
            let (a, b) = (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0));
            let shares = (0.0..=1.0).contains(&a) && (0.0..=1.0).contains(&b);
            let limit = if shares {
                SHARE_MOVED
            } else {
                SIZE_MOVED * a.abs().max(b.abs())
            };
            usize::from((a - b).abs() > limit)
        }
        (a, b) => usize::from(a != b),
    }
}

/// Writes `new` to `file` unless the profile there is close enough, and says which, without a number of the account.
fn adopt(new: &Profile, file: &Path) -> Result<()> {
    new.check().context("the calibrated profile")?;
    let mut fresh = serde_json::to_value(new)?;
    let old = Profile::load(file).ok();
    if let Some(old) = &old {
        let mut kept = serde_json::to_value(old)?;
        // the day it was measured is not a change
        kept["measured"] = Value::Null;
        fresh["measured"] = Value::Null;
        let n = moved(&kept, &fresh);
        if n == 0 {
            println!("unchanged: the profile {} still fits", old.hash()?);
            return Ok(());
        }
        println!("{n} parts moved");
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(new)?)?;
    std::fs::rename(&tmp, file)?;
    println!("written: the profile {} of {}", new.hash()?, new.measured);
    Ok(())
}
