//! Message search, media scans, voice data (`media_*.db`) and emoticon lookups (`emoticon.db`).

use std::path::PathBuf;

use anyhow::Result;
use rusqlite::ToSql;
use serde_json::{json, Map, Value};

use crate::native_db::NativeAccount;

/// Message kinds the search looks into: text, app messages (links/files/quotes) and quote replies.
const SEARCHABLE: [i64; 3] = [1, 49, 244_813_135_921];
/// `scan_media_stream` codes: 1 = images, 2 = videos, anything else = both.
const IMAGE: i64 = 3;
const VIDEO: i64 = 43;
/// Voice rows are matched by `create_time` with this much clock jitter (seconds).
const VOICE_TIME_WINDOW: i64 = 5;

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn text(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

fn range_sql(begin: i64, end: i64) -> String {
    let mut s = String::new();
    if begin > 0 {
        s.push_str(&format!(" and m.create_time >= {begin}"));
    }
    if end > 0 {
        s.push_str(&format!(" and m.create_time <= {end}"));
    }
    s
}

fn sort_key(row: &Value) -> (i64, i64, i64) {
    let create = int(row, "create_time");
    (
        match int(row, "sort_seq") {
            0 => create * 1000,
            s => s,
        },
        create,
        int(row, "local_id"),
    )
}

/// The first `k` rows in "newest first, then in the order found" order (what a stable newest-first sort of every
/// row would put first), without holding the others.
struct Newest {
    k: usize,
    found: usize,
    heap: std::collections::BinaryHeap<Ranked>,
}

struct Ranked {
    key: (i64, i64, i64),
    seq: usize,
    row: Value,
}

impl Ranked {
    /// Ordered so the heap's top is the row that would come last.
    fn rank(&self, other: &Self) -> std::cmp::Ordering {
        other.key.cmp(&self.key).then(self.seq.cmp(&other.seq))
    }
}

impl PartialEq for Ranked {
    fn eq(&self, other: &Self) -> bool {
        self.rank(other).is_eq()
    }
}
impl Eq for Ranked {}
impl PartialOrd for Ranked {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Ranked {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank(other)
    }
}

impl Newest {
    fn new(k: usize) -> Self {
        Self {
            k,
            found: 0,
            heap: Default::default(),
        }
    }

    fn push(&mut self, row: Value) {
        let seq = self.found;
        self.found += 1;
        if self.k == 0 {
            return;
        }
        self.heap.push(Ranked {
            key: sort_key(&row),
            seq,
            row,
        });
        if self.heap.len() > self.k {
            self.heap.pop();
        }
    }

    /// Add the rows another `Newest` kept, as if they had been pushed here in the order they were found (after
    /// everything pushed so far). Rows it dropped could not have made the cut here either.
    fn absorb(&mut self, other: Newest) {
        let base = self.found;
        self.found += other.found;
        for mut r in other.heap.into_vec() {
            r.seq += base;
            self.heap.push(r);
            if self.heap.len() > self.k {
                self.heap.pop();
            }
        }
    }

    /// The kept rows, first one first.
    fn into_rows(self) -> Vec<Value> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|r| r.row)
            .collect()
    }
}

impl NativeAccount {
    /// Newest-first messages whose text contains `keyword` (case-insensitive), from one session or all.
    /// Each row is a message row plus `_session_id`. Compressed contents are decoded before matching.
    pub fn search_messages(
        &self,
        keyword: &str,
        session_id: Option<&str>,
        limit: i32,
        offset: i32,
        begin: i64,
        end: i64,
    ) -> Result<Value> {
        let needle = keyword.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(json!([]));
        }
        let sessions: Vec<String> = match session_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => vec![s.to_string()],
            None => crate::native_msg::session_usernames(self)?,
        };
        let like = format!(
            "%{}%",
            needle
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let kinds = SEARCHABLE
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let range = range_sql(begin, end);
        let (offset, limit) = (
            offset.max(0) as usize,
            if limit > 0 { limit as usize } else { 50 },
        );
        let mut hits = Newest::new(offset + limit);
        for sid in &sessions {
            for t in self.message_tables(sid)? {
                // plain text is pre-filtered in SQL; blobs (zstd) can only be matched after decoding
                let sql = format!(
                    "select m.*, n.user_name as sender_username from \"{table}\" m left join Name2Id n on n.rowid = m.real_sender_id \
                     where m.local_type in ({kinds}){range} and (typeof(m.message_content) = 'blob' or m.message_content like ?1 escape '\\')",
                    table = t.table
                );
                // rows are matched as they are read: a short keyword can match most of a big conversation
                let table_hits = std::cell::RefCell::new(Newest::new(offset + limit));
                self.query_each(
                    &t.db,
                    &sql,
                    &[&like],
                    || *table_hits.borrow_mut() = Newest::new(offset + limit),
                    |mut r| {
                        if text(&r, "message_content").to_lowercase().contains(&needle) {
                            self.finish_row(&mut r, &t, false);
                            r["_session_id"] = json!(sid);
                            table_hits.borrow_mut().push(r);
                        }
                    },
                )?;
                hits.absorb(table_hits.into_inner());
            }
        }
        Ok(Value::Array(
            hits.into_rows()
                .into_iter()
                .skip(offset)
                .take(limit)
                .collect(),
        ))
    }

    /// Image/video messages across sessions, newest first; returns the page and whether more remain.
    pub fn scan_media_stream(
        &self,
        session_ids: &[String],
        media_type: i32,
        begin: i64,
        end: i64,
        limit: i32,
        offset: i32,
    ) -> Result<(Value, bool)> {
        let kinds = match media_type {
            1 => vec![IMAGE],
            2 => vec![VIDEO],
            _ => vec![IMAGE, VIDEO],
        };
        let kinds = kinds
            .iter()
            .map(i64::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let range = range_sql(begin, end);
        let offset = offset.max(0) as usize;
        // `limit <= 0` asks for everything after `offset`
        let k = if limit > 0 {
            offset + limit as usize
        } else {
            usize::MAX
        };
        let mut rows = Newest::new(k);
        for sid in session_ids {
            for t in self.message_tables(sid)? {
                let sql = format!(
                    "select m.*, n.user_name as sender_username from \"{}\" m left join Name2Id n on n.rowid = m.real_sender_id \
                     where m.local_type in ({kinds}){range}",
                    t.table
                );
                let table_rows = std::cell::RefCell::new(Newest::new(k));
                self.query_each(
                    &t.db,
                    &sql,
                    &[],
                    || *table_rows.borrow_mut() = Newest::new(k),
                    |mut r| {
                        self.finish_row(&mut r, &t, false);
                        r["session_id"] = json!(sid);
                        table_rows.borrow_mut().push(r);
                    },
                )?;
                rows.absorb(table_rows.into_inner());
            }
        }
        let found = rows.found;
        let page: Vec<Value> = rows.into_rows().into_iter().skip(offset).collect();
        let more = limit > 0 && found > offset + limit as usize;
        Ok((Value::Array(page), more))
    }

    // ── voice ──

    /// `message/media_<n>.db` shards.
    pub fn media_dbs(&self) -> Vec<PathBuf> {
        let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(self.db_path("message"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                let n = name
                    .strip_prefix("media_")?
                    .strip_suffix(".db")?
                    .parse()
                    .ok()?;
                Some((n, e.path()))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, p)| p).collect()
    }

    /// Voice bytes as lowercase hex (several fragments are concatenated in `data_index` order); `""` when absent.
    ///
    /// Matching: the exact server id first, then `local_id` with `create_time` within ±5 s. `candidates` are the
    /// chat names (sender, session, self) the row may belong to; an empty list accepts any chat.
    pub fn voice_data(
        &self,
        create_time: i64,
        local_id: i64,
        svr_id: i64,
        candidates: &[String],
    ) -> Result<String> {
        for db in self.media_dbs() {
            let chat_filter = if candidates.is_empty() {
                String::new()
            } else {
                let sql = format!(
                    "select rowid as id from Name2Id where user_name in ({})",
                    vec!["?"; candidates.len()].join(",")
                );
                let params: Vec<&dyn ToSql> = candidates.iter().map(|c| c as &dyn ToSql).collect();
                let ids: Vec<String> = self
                    .query(&db, &sql, &params)?
                    .iter()
                    .map(|r| int(r, "id").to_string())
                    .collect();
                if ids.is_empty() {
                    continue;
                }
                format!(" and chat_name_id in ({})", ids.join(","))
            };
            let order = " order by cast(data_index as integer), data_index";
            let tiers: [(bool, String, Vec<i64>); 2] = [
                (svr_id > 0, format!("svr_id = ?1{chat_filter}{order}"), vec![svr_id]),
                (
                    local_id > 0,
                    format!("local_id = ?1 and abs(create_time - ?2) <= {VOICE_TIME_WINDOW}{chat_filter}{order}"),
                    vec![local_id, create_time],
                ),
            ];
            for (enabled, cond, args) in tiers {
                if !enabled {
                    continue;
                }
                let params: Vec<&dyn ToSql> = args.iter().map(|a| a as &dyn ToSql).collect();
                let rows = self.query(
                    &db,
                    &format!("select voice_data from VoiceInfo where {cond}"),
                    &params,
                )?;
                let hex: String = rows.iter().map(|r| text(r, "voice_data")).collect();
                if !hex.is_empty() {
                    return Ok(hex);
                }
            }
        }
        Ok(String::new())
    }

    /// `[{ "hex": ... }]`, one entry per request (`{}` when no voice was found).
    pub fn voice_data_batch(&self, requests: &Value) -> Result<Value> {
        let mut out = Vec::new();
        for req in requests.as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let svr = match &req["svr_id"] {
                Value::Number(n) => n.as_i64().unwrap_or(0),
                Value::String(s) => s.trim().parse().unwrap_or(0),
                _ => 0,
            };
            let candidates: Vec<String> = req["candidates"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let hex = self.voice_data(
                int(req, "create_time"),
                int(req, "local_id"),
                svr,
                &candidates,
            )?;
            out.push(if hex.is_empty() {
                json!({})
            } else {
                json!({ "hex": hex })
            });
        }
        Ok(Value::Array(out))
    }

    // ── emoticons ──

    fn emoticon_db(&self) -> PathBuf {
        self.db_path("emoticon/emoticon.db")
    }

    /// Download URL of a sticker by content md5 (case-insensitive): CDN first, then external, preview, thumbnail.
    pub fn emoticon_cdn_url(&self, md5: &str) -> Result<String> {
        let rows = self.query(&self.emoticon_db(), "select cdn_url, extern_url, tp_url, thumb_url from kNonStoreEmoticonTable where lower(md5) = lower(?1)", &[&md5.trim()])?;
        Ok(rows
            .iter()
            .flat_map(|r| {
                ["cdn_url", "extern_url", "tp_url", "thumb_url"]
                    .into_iter()
                    .map(|k| text(r, k))
            })
            .find(|u| !u.is_empty())
            .unwrap_or_default())
    }

    /// Caption of a sticker: the user's own sticker text, else the store package caption.
    pub fn emoticon_caption(&self, md5: &str) -> Result<String> {
        let own = self.query(&self.emoticon_db(), "select caption from kNonStoreEmoticonTable where lower(md5) = lower(?1) and caption <> ''", &[&md5.trim()])?;
        if let Some(c) = own.first() {
            return Ok(text(c, "caption"));
        }
        let store = self.query(&self.emoticon_db(), "select caption_ from kStoreEmoticonCaptionsTable where lower(md5_) = lower(?1) and caption_ <> '' order by (language_ like 'zh%') desc", &[&md5.trim()])?;
        Ok(store
            .first()
            .map(|c| text(c, "caption_"))
            .unwrap_or_default())
    }

    // ── small diagnostics ──

    /// `["<path to message_N.db>", ...]`.
    pub fn list_message_dbs(&self) -> Value {
        Value::Array(
            self.message_dbs()
                .iter()
                .map(|p| json!(p.to_string_lossy()))
                .collect(),
        )
    }

    pub fn list_media_dbs(&self) -> Value {
        Value::Array(
            self.media_dbs()
                .iter()
                .map(|p| json!(p.to_string_lossy()))
                .collect(),
        )
    }

    /// `[{ db_path, table_name }]` of the shards holding a session's messages.
    pub fn message_tables_json(&self, session_id: &str) -> Result<Value> {
        Ok(Value::Array(
            self.message_tables(session_id)?
                .into_iter()
                .map(|t| {
                    let mut o = Map::new();
                    o.insert("db_path".into(), json!(t.db.to_string_lossy()));
                    o.insert("table_name".into(), json!(t.table));
                    Value::Object(o)
                })
                .collect(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, DAY, T0};

    fn account(tag: &str) -> NativeAccount {
        let root = std::env::temp_dir().join(format!("weflow-media-{}-{tag}", std::process::id()));
        let f = Fixture::standard(&root);
        NativeAccount::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()))
    }

    fn contents(v: &Value) -> Vec<String> {
        v.as_array()
            .unwrap()
            .iter()
            .map(|r| r["message_content"].as_str().unwrap().trim().to_string())
            .collect()
    }

    #[test]
    fn newest_keeps_what_a_stable_newest_first_sort_puts_first() {
        // small key ranges so ties (and the "order found" tie-break) are common; a fixed LCG keeps it reproducible
        let mut x: u64 = 42;
        let mut next = |m: u64| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            ((x >> 33) % m) as i64
        };
        for round in 0..200 {
            let rows: Vec<Value> = (0..next(60) as usize)
                .map(|i| json!({ "sort_seq": next(4) * 1000, "create_time": next(3), "local_id": next(3), "i": i }))
                .collect();
            let mut sorted = rows.clone();
            sorted.sort_by(|a, b| sort_key(b).cmp(&sort_key(a)));
            for k in [0usize, 1, 3, 10, 100] {
                let mut top = Newest::new(k);
                rows.iter().cloned().for_each(|r| top.push(r));
                assert_eq!(top.found, rows.len());
                assert_eq!(
                    top.into_rows(),
                    sorted.iter().take(k).cloned().collect::<Vec<_>>(),
                    "round {round} k {k}"
                );
                // the same rows found table by table, each table ranked on its own first
                let mut merged = Newest::new(k);
                for chunk in rows.chunks(7) {
                    let mut local = Newest::new(k);
                    chunk.iter().cloned().for_each(|r| local.push(r));
                    merged.absorb(local);
                }
                assert_eq!(merged.found, rows.len());
                assert_eq!(
                    merged.into_rows(),
                    sorted.iter().take(k).cloned().collect::<Vec<_>>(),
                    "merged, round {round} k {k}"
                );
            }
        }
    }

    #[test]
    fn search_finds_plain_and_compressed_text_across_sessions() {
        let a = account("search");
        // "later, compressed" is a zstd blob: it can only be found after decoding
        assert_eq!(
            contents(
                &a.search_messages("COMPRESSED", Some("wxid_bob"), 50, 0, 0, 0)
                    .unwrap()
            ),
            ["later, compressed"]
        );
        assert_eq!(
            contents(&a.search_messages("hello", None, 50, 0, 0, 0).unwrap()),
            ["hello"]
        );
        let ok = a.search_messages("ok", None, 50, 0, 0, 0).unwrap();
        assert_eq!(
            contents(&ok),
            ["wxid_quiet:\nok"],
            "group text keeps its sender prefix in the raw row"
        );
        assert_eq!(ok[0]["_session_id"], "room1@chatroom");
        assert_eq!(ok[0]["sender_username"], "wxid_quiet");
        // the image message's XML is type 3, which is never searched
        assert_eq!(
            a.search_messages("aabbccdd", None, 50, 0, 0, 0).unwrap(),
            json!([])
        );
        assert_eq!(
            a.search_messages("  ", None, 50, 0, 0, 0).unwrap(),
            json!([])
        );
        // LIKE wildcards in the keyword are literals
        assert_eq!(
            a.search_messages("%", None, 50, 0, 0, 0).unwrap(),
            json!([])
        );
        assert_eq!(
            a.search_messages("h_llo", None, 50, 0, 0, 0).unwrap(),
            json!([])
        );
    }

    #[test]
    fn search_orders_pages_and_filters_by_time() {
        let a = account("page");
        let all = a
            .search_messages("e", Some("wxid_bob"), 50, 0, 0, 0)
            .unwrap(); // hello, later, compressed, see you
        assert_eq!(
            contents(&all),
            ["see you", "later, compressed", "hello"],
            "newest first"
        );
        assert_eq!(
            contents(
                &a.search_messages("e", Some("wxid_bob"), 1, 1, 0, 0)
                    .unwrap()
            ),
            ["later, compressed"]
        );
        assert_eq!(
            contents(
                &a.search_messages("e", Some("wxid_bob"), 50, 0, (T0 + 3 * DAY) as i64, 0)
                    .unwrap()
            ),
            ["see you", "later, compressed"]
        );
        assert_eq!(
            a.search_messages("e", Some("wxid_bob"), 50, 0, 0, T0 as i64 - 1)
                .unwrap(),
            json!([])
        );
    }

    #[test]
    fn media_stream_pages_images_and_videos() {
        let a = account("media");
        let ids = vec!["wxid_bob".to_string(), "room1@chatroom".to_string()];
        let (rows, more) = a.scan_media_stream(&ids, 1, 0, 0, 10, 0).unwrap();
        assert_eq!(
            rows.as_array().unwrap().len(),
            1,
            "one image in the fixture"
        );
        assert_eq!(rows[0]["session_id"], "wxid_bob");
        assert_eq!(rows[0]["local_type"], 3);
        assert!(!more);
        assert_eq!(
            a.scan_media_stream(&ids, 2, 0, 0, 10, 0).unwrap().0,
            json!([]),
            "no videos"
        );
        let (both, _) = a.scan_media_stream(&ids, 0, 0, 0, 10, 0).unwrap();
        assert_eq!(both.as_array().unwrap().len(), 1);
        let (page, more) = a.scan_media_stream(&ids, 1, 0, 0, 1, 1).unwrap();
        assert_eq!((page, more), (json!([]), false));
    }

    #[test]
    fn voice_is_found_by_server_id_or_by_local_id_and_time_window() {
        let a = account("voice");
        let bob = vec!["wxid_bob".to_string()];
        assert_eq!(
            a.voice_data(T0 + 500, 6, 9_000_000_000_001, &bob).unwrap(),
            "01020304",
            "exact server id"
        );
        assert_eq!(
            a.voice_data(T0 + 500, 6, 0, &bob).unwrap(),
            "01020304",
            "local id + time"
        );
        assert_eq!(
            a.voice_data(T0 + 503, 6, 0, &bob).unwrap(),
            "01020304",
            "within the +-5 s window"
        );
        assert_eq!(
            a.voice_data(T0 + 520, 6, 0, &bob).unwrap(),
            "",
            "outside the window"
        );
        assert_eq!(
            a.voice_data(T0 + 600, 7, 9_000_000_000_002, &bob).unwrap(),
            "aabbcc",
            "fragments are joined in data_index order"
        );
        // the candidate list decides whose chat is searched
        assert_eq!(
            a.voice_data(T0 + 700, 8, 9_000_000_000_003, &bob).unwrap(),
            ""
        );
        assert_eq!(
            a.voice_data(T0 + 700, 8, 9_000_000_000_003, &["wxid_me".to_string()])
                .unwrap(),
            "09"
        );
        assert_eq!(
            a.voice_data(T0 + 700, 8, 9_000_000_000_003, &[]).unwrap(),
            "09",
            "no candidates: any chat"
        );
        assert_eq!(
            a.voice_data(T0, 0, 0, &[]).unwrap(),
            "",
            "no key at all matches nothing"
        );
    }

    #[test]
    fn voice_batch_mixes_hits_and_misses() {
        let a = account("vbatch");
        let req = json!([
            {"session_id": "wxid_bob", "create_time": T0 + 500, "local_id": 6, "svr_id": 9_000_000_000_001i64, "candidates": ["wxid_bob"]},
            {"session_id": "wxid_bob", "create_time": T0 + 600, "local_id": 7, "svr_id": "9000000000002", "candidates": ["wxid_bob", "wxid_me"]},
            {"session_id": "wxid_bob", "create_time": 1, "local_id": 99, "svr_id": 5, "candidates": []}
        ]);
        assert_eq!(
            a.voice_data_batch(&req).unwrap(),
            json!([{"hex": "01020304"}, {"hex": "aabbcc"}, {}])
        );
        assert_eq!(a.voice_data_batch(&json!("nonsense")).unwrap(), json!([]));
    }

    #[test]
    fn emoticon_urls_and_captions() {
        let a = account("emoji");
        assert_eq!(
            a.emoticon_cdn_url("aabbccddeeff00112233445566778899")
                .unwrap(),
            "http://cdn/e1",
            "md5 is case-insensitive, CDN first"
        );
        assert_eq!(
            a.emoticon_cdn_url("11223344556677889900AABBCCDDEEFF")
                .unwrap(),
            "http://ext/e2",
            "falls back to the external url"
        );
        assert_eq!(
            a.emoticon_cdn_url("ffffffffffffffffffffffffffffffff")
                .unwrap(),
            ""
        );
        assert_eq!(
            a.emoticon_caption("AABBCCDDEEFF00112233445566778899")
                .unwrap(),
            "smile"
        );
        assert_eq!(
            a.emoticon_caption("deadbeefdeadbeefdeadbeefdeadbeef")
                .unwrap(),
            "商店表情",
            "store captions prefer Chinese"
        );
        assert_eq!(
            a.emoticon_caption("ffffffffffffffffffffffffffffffff")
                .unwrap(),
            ""
        );
    }

    #[test]
    fn diagnostics_list_shards() {
        let a = account("diag");
        assert_eq!(a.list_message_dbs().as_array().unwrap().len(), 2);
        assert_eq!(a.list_media_dbs().as_array().unwrap().len(), 1);
        let t = a.message_tables_json("wxid_bob").unwrap();
        assert_eq!(t.as_array().unwrap().len(), 2);
        assert!(t[0]["table_name"].as_str().unwrap().starts_with("Msg_"));
    }
}
