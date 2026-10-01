//! Database introspection (table lists, schemas, column lists, status), table snapshots, the
//! `hardlink.db` file lookups and the avatar blobs of `head_image.db`.
//!
//! Everything here only reads. Importing a snapshot would write into WeChat's databases and is refused.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use rusqlite::types::ValueRef;
use serde_json::{json, Map, Value};

use crate::native_db::{hex, NativeAccount};
use crate::native_stats::check_ident;

/// Columns left out of `message_meta`: the heavy payloads.
const HEAVY_MESSAGE_COLUMNS: [&str; 4] = [
    "message_content",
    "compress_content",
    "source",
    "packed_info_data",
];

fn str_of(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim()
        .to_string()
}

impl NativeAccount {
    /// Tables of a database (`kind` picks the default file, `path` overrides it), sorted by name.
    pub fn list_tables(&self, kind: &str, path: Option<&str>) -> Result<Value> {
        let db = self.resolve_db_path(kind, path)?;
        let rows = self.query(&db, "select name from sqlite_master where type = 'table' and name not like 'sqlite_%' order by name", &[])?;
        Ok(Value::Array(
            rows.iter().map(|r| r["name"].clone()).collect(),
        ))
    }

    /// `{ success, schema }` with the `CREATE TABLE` statement of one table.
    pub fn table_schema(&self, kind: &str, path: Option<&str>, table: &str) -> Result<Value> {
        check_ident(table)?;
        let db = self.resolve_db_path(kind, path)?;
        let rows = self.query(
            &db,
            "select sql from sqlite_master where type in ('table', 'view') and name = ?1",
            &[&table],
        )?;
        let sql = rows
            .first()
            .and_then(|r| r["sql"].as_str())
            .ok_or_else(|| anyhow!("table not found: {table}"))?;
        Ok(json!({ "success": true, "schema": sql }))
    }

    fn columns_of(&self, db: &Path, table: &str) -> Result<Vec<String>> {
        check_ident(table)?;
        let rows = self.query(
            db,
            "select name from pragma_table_info(?1) order by cid",
            &[&table],
        )?;
        if rows.is_empty() {
            bail!("table not found: {table}");
        }
        Ok(rows.iter().map(|r| str_of(r, "name")).collect())
    }

    /// Column names of a message table.
    pub fn message_table_columns(&self, db_path: &str, table: &str) -> Result<Value> {
        let db = self.resolve_db_path("message", Some(db_path))?;
        Ok(json!(self.columns_of(&db, table)?))
    }

    /// Message rows without their payload columns (content, compressed content, source, packed info).
    pub fn message_meta(
        &self,
        db_path: &str,
        table: &str,
        limit: i32,
        offset: i32,
    ) -> Result<Value> {
        let db = self.resolve_db_path("message", Some(db_path))?;
        let columns = self.columns_of(&db, table)?;
        let select: Vec<String> = columns
            .iter()
            .filter(|c| !HEAVY_MESSAGE_COLUMNS.contains(&c.as_str()))
            .map(|c| format!("\"{c}\""))
            .collect();
        if select.is_empty() {
            bail!("{table} has no metadata columns");
        }
        let order = if columns.iter().any(|c| c == "sort_seq") {
            "sort_seq, create_time, local_id"
        } else {
            "rowid"
        };
        let limit = if limit <= 0 { -1 } else { limit as i64 };
        let sql = format!(
            "select {} from \"{table}\" order by {order} limit ?1 offset ?2",
            select.join(", ")
        );
        Ok(Value::Array(self.query(
            &db,
            &sql,
            &[&limit, &(offset.max(0) as i64)],
        )?))
    }

    /// Write every row of a table to `output_path` as JSON lines: a header (`table`, `schema`, `columns`)
    /// followed by one `{"row": [...]}` per row; blobs are `{"$blob": "<hex>"}`.
    /// The destination must not be inside WeChat's own `db_storage`.
    pub fn export_table_snapshot(
        &self,
        kind: &str,
        path: Option<&str>,
        table: &str,
        output_path: &str,
    ) -> Result<Value> {
        check_ident(table)?;
        let out = PathBuf::from(output_path.trim());
        if output_path.trim().is_empty() {
            bail!("output path is empty");
        }
        if out.starts_with(self.db_storage()) {
            bail!(
                "refusing to write inside WeChat's db_storage: {}",
                out.display()
            );
        }
        let db = self.resolve_db_path(kind, path)?;
        let schema = self.table_schema(kind, path, table)?["schema"].clone();
        let columns = self.columns_of(&db, table)?;
        let rows = self.with_db(&db, |conn| {
            // (re)created here: a snapshot that goes stale mid-way makes `with_db` run this again from the start
            let file = std::fs::File::create(&out)
                .with_context(|| format!("cannot create {}", out.display()))?;
            let mut w = std::io::BufWriter::new(file);
            writeln!(
                w,
                "{}",
                json!({ "table": table, "schema": schema, "columns": columns })
            )?;
            let mut stmt = conn.prepare(&format!("select * from \"{table}\""))?;
            let mut cursor = stmt.query([])?;
            let mut n = 0u64;
            while let Some(row) = cursor.next()? {
                let cells: Vec<Value> = (0..columns.len())
                    .map(|i| match row.get_ref(i) {
                        Ok(ValueRef::Null) | Err(_) => Value::Null,
                        Ok(ValueRef::Integer(v)) => json!(v),
                        Ok(ValueRef::Real(v)) => json!(v),
                        Ok(ValueRef::Text(t)) => {
                            Value::String(String::from_utf8_lossy(t).into_owned())
                        }
                        Ok(ValueRef::Blob(b)) => json!({ "$blob": hex(b) }),
                    })
                    .collect();
                writeln!(w, "{}", json!({ "row": cells }))?;
                n += 1;
            }
            w.flush()?;
            Ok(n)
        })?;
        Ok(
            json!({ "success": true, "rows": rows, "columns": columns.len(), "path": out.to_string_lossy() }),
        )
    }

    /// Overview of the account's databases and of the snapshot cache.
    pub fn db_status(&self) -> Result<Value> {
        let root = self.db_storage().to_path_buf();
        let mut dbs = Vec::new();
        let mut stack = vec![(root.clone(), 0)];
        while let Some((dir, depth)) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let p = entry.path();
                if p.is_dir() {
                    if depth < 3 {
                        stack.push((p, depth + 1));
                    }
                } else if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("db")) {
                    let wal = std::fs::metadata(format!("{}-wal", p.display()))
                        .map(|m| m.len())
                        .unwrap_or(0);
                    let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    let rel = p
                        .strip_prefix(&root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/");
                    dbs.push(json!({ "path": rel, "size": size, "walSize": wal }));
                }
            }
        }
        dbs.sort_by(|a, b| a["path"].as_str().cmp(&b["path"].as_str()));
        let (snapshots, bytes, budget) = self.cache_stats();
        Ok(json!({
            "connected": true,
            "backend": "native",
            "readOnly": true,
            "dbStorage": root.to_string_lossy(),
            "databases": dbs,
            "cache": { "snapshots": snapshots, "bytes": bytes, "budgetBytes": budget },
        }))
    }

    /// Tables of a media database (`media_N.db`) with their columns and row counts.
    pub fn media_schema_summary(&self, db_path: &str) -> Result<Value> {
        let db = self.resolve_db_path("message", Some(db_path))?;
        let mut tables = Vec::new();
        for name in self.query(&db, "select name from sqlite_master where type = 'table' and name not like 'sqlite_%' order by name", &[])? {
            let name = str_of(&name, "name");
            if check_ident(&name).is_err() {
                continue;
            }
            let columns = self.columns_of(&db, &name).unwrap_or_default();
            let rows = self.query(&db, &format!("select count(*) as n from \"{name}\""), &[]).ok().and_then(|r| r[0]["n"].as_i64()).unwrap_or(0);
            tables.push(json!({ "name": name, "columns": columns, "rows": rows }));
        }
        let has = |n: &str| tables.iter().any(|t| t["name"] == n);
        Ok(
            json!({ "path": db.to_string_lossy(), "hasVoiceInfo": has("VoiceInfo"), "hasName2Id": has("Name2Id"), "tables": tables }),
        )
    }

    /// `{ username: <lowercase hex of the avatar image> }` for the usernames that have one in `head_image.db`.
    pub fn head_image_buffers(&self, usernames: &[String]) -> Result<Value> {
        let db = self.resolve_db_path("head_image", None)?;
        let mut map = Map::new();
        for chunk in usernames.chunks(400) {
            let marks = vec!["?"; chunk.len()].join(",");
            let params: Vec<&dyn rusqlite::ToSql> =
                chunk.iter().map(|u| u as &dyn rusqlite::ToSql).collect();
            self.with_db(&db, |conn| {
                let mut stmt = conn.prepare(&format!(
                    "select username, image_buffer from head_image where username in ({marks})"
                ))?;
                let mut rows = stmt.query(params.as_slice())?;
                while let Some(row) = rows.next()? {
                    let name: String = row.get(0)?;
                    if let Ok(ValueRef::Blob(b)) = row.get_ref(1) {
                        if !b.is_empty() {
                            map.insert(name, Value::String(hex(b)));
                        }
                    }
                }
                Ok(())
            })?;
        }
        Ok(Value::Object(map))
    }

    // ── hardlink.db: md5 → file on disk ──

    fn hardlink_dirs(&self, db: &Path) -> Result<HashMap<i64, String>> {
        let rows = self.query(db, "select rowid, username from dir2id", &[])?;
        Ok(rows
            .iter()
            .filter_map(|r| Some((r["rowid"].as_i64()?, str_of(r, "username"))))
            .collect())
    }

    /// Where the image with this message md5 lives: `<account>/msg/attach/<dir1>/<dir2>/Img/<file_name>`.
    /// Several renditions can share an md5; the HD one wins, then the original, then thumbnails.
    pub fn resolve_image_hardlink(&self, md5: &str, account_dir: Option<&str>) -> Result<Value> {
        let md5 = md5.trim().to_ascii_lowercase();
        if md5.is_empty() {
            bail!("md5 is empty");
        }
        let db = self.resolve_db_path("hardlink", None)?;
        let dirs = self.hardlink_dirs(&db)?;
        let rows = self.query(
            &db,
            "select md5, type, file_name, file_size, modify_time, dir1, dir2 from image_hardlink_info_v4 where md5 = ?1 and type <> 4",
            &[&md5],
        )?;
        let best = rows
            .iter()
            .min_by_key(|r| {
                (
                    image_rank(&str_of(r, "file_name")),
                    -r["modify_time"].as_i64().unwrap_or(0),
                )
            })
            .ok_or_else(|| anyhow!("no hardlink record for md5 {md5}"))?;
        let account = account_dir
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.account_dir());
        let name = |key: &str| {
            dirs.get(&best[key].as_i64().unwrap_or(0))
                .cloned()
                .unwrap_or_default()
        };
        let file_name = str_of(best, "file_name");
        let full = account
            .join("msg/attach")
            .join(name("dir1"))
            .join(name("dir2"))
            .join("Img")
            .join(&file_name);
        Ok(hardlink_data(best, &md5, &file_name, &full, None))
    }

    /// The video file name behind a message md5: `<account>/msg/video/<dir1>/<file_name>`;
    /// `resolved_md5` is the file name without its extension (what the video folder uses).
    pub fn resolve_video_hardlink_md5(&self, md5: &str, db_path: Option<&str>) -> Result<Value> {
        let md5 = md5.trim().to_ascii_lowercase();
        if md5.is_empty() {
            bail!("md5 is empty");
        }
        let db = self.resolve_db_path("hardlink", db_path)?;
        let dirs = self.hardlink_dirs(&db)?;
        let rows = self.query(
            &db,
            "select md5, type, file_name, file_size, modify_time, dir1, dir2 from video_hardlink_info_v4 where md5 = ?1 order by modify_time desc",
            &[&md5],
        )?;
        let best = rows
            .first()
            .ok_or_else(|| anyhow!("no hardlink record for md5 {md5}"))?;
        let file_name = str_of(best, "file_name");
        let month = dirs
            .get(&best["dir1"].as_i64().unwrap_or(0))
            .cloned()
            .unwrap_or_default();
        let full = self
            .account_dir()
            .join("msg/video")
            .join(month)
            .join(&file_name);
        let stem = Path::new(&file_name)
            .file_stem()
            .map(|s| s.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        Ok(hardlink_data(best, &md5, &file_name, &full, Some(stem)))
    }

    /// `requests` is `[{ md5, account_dir? }]`; the result is `[{ index, md5, success, data | error }]`.
    pub fn resolve_image_hardlink_batch(&self, requests: &Value) -> Result<Value> {
        batch(requests, |req| {
            self.resolve_image_hardlink(&str_of(req, "md5"), Some(&str_of(req, "account_dir")))
        })
    }

    /// `requests` is `[{ md5, db_path? }]`; same result shape as the image variant.
    pub fn resolve_video_hardlink_md5_batch(&self, requests: &Value) -> Result<Value> {
        batch(requests, |req| {
            self.resolve_video_hardlink_md5(&str_of(req, "md5"), Some(&str_of(req, "db_path")))
        })
    }
}

/// 0 = HD (`_h`, `_hd`, `.h`), 1 = big (`_b`), 2 = plain original, 3 = thumbnail variants.
fn image_rank(file_name: &str) -> u8 {
    let lower = file_name.to_ascii_lowercase();
    let base = lower.strip_suffix(".dat").unwrap_or(&lower);
    if ["_h", ".h", "_hd", ".hd"].iter().any(|s| base.ends_with(s)) {
        0
    } else if base.ends_with("_b") || base.ends_with(".b") {
        1
    } else if ["_t", ".t", "_thumb", ".thumb"]
        .iter()
        .any(|s| base.ends_with(s))
    {
        3
    } else {
        2
    }
}

fn hardlink_data(
    row: &Value,
    md5: &str,
    file_name: &str,
    full: &Path,
    resolved: Option<String>,
) -> Value {
    let mut o = Map::new();
    o.insert("md5".into(), json!(md5));
    if let Some(r) = resolved {
        o.insert("resolved_md5".into(), json!(r));
    }
    o.insert("file_name".into(), json!(file_name));
    o.insert("full_path".into(), json!(full.to_string_lossy()));
    for k in ["type", "file_size", "modify_time", "dir1", "dir2"] {
        o.insert(k.into(), row[k].clone());
    }
    Value::Object(o)
}

fn batch(requests: &Value, mut one: impl FnMut(&Value) -> Result<Value>) -> Result<Value> {
    let list = requests
        .as_array()
        .ok_or_else(|| anyhow!("requests must be an array"))?;
    Ok(Value::Array(
        list.iter()
            .enumerate()
            .map(|(index, req)| {
                let md5 = str_of(req, "md5").to_ascii_lowercase();
                match one(req) {
                    Ok(data) => json!({ "index": index, "md5": md5, "success": true, "data": data }),
                    Err(e) => json!({ "index": index, "md5": md5, "success": false, "error": format!("{e:#}") }),
                }
            })
            .collect(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, HardlinkSpec};

    fn account(tag: &str) -> (NativeAccount, Fixture) {
        let root = std::env::temp_dir().join(format!("weflow-meta-{}-{tag}", std::process::id()));
        let f = Fixture::standard(&root);
        f.hardlink_db(&[
            HardlinkSpec::image(
                "aabbccddeeff00112233445566778899",
                "0123456789abcdef0123456789abcdef.dat",
                "d41d8cd98f00b204e9800998ecf8427e",
                "2023-11",
            ),
            HardlinkSpec::image(
                "aabbccddeeff00112233445566778899",
                "0123456789abcdef0123456789abcdef_h.dat",
                "d41d8cd98f00b204e9800998ecf8427e",
                "2023-11",
            ),
            HardlinkSpec {
                kind: 4,
                ..HardlinkSpec::image(
                    "aabbccddeeff00112233445566778899",
                    "0",
                    "d41d8cd98f00b204e9800998ecf8427e",
                    "2023-11",
                )
            },
            HardlinkSpec::video(
                "11223344556677889900aabbccddeeff",
                "ffeeddccbbaa99887766554433221100.mp4",
                "2023-11",
            ),
        ]);
        f.head_image_db(&[
            ("wxid_bob", &[0xff, 0xd8, 0xff, 0x01]),
            ("wxid_carol", &[1, 2, 3]),
        ]);
        let acct = NativeAccount::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()));
        (acct, f)
    }

    #[test]
    fn tables_columns_and_schema() {
        let (a, _f) = account("tables");
        let tables = a.list_tables("session", None).unwrap();
        assert!(tables
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "SessionTable"));
        assert!(a
            .list_tables("hardlink", None)
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t == "image_hardlink_info_v4"));
        let schema = a.table_schema("session", None, "SessionTable").unwrap();
        assert!(
            schema["schema"]
                .as_str()
                .unwrap()
                .starts_with("CREATE TABLE SessionTable"),
            "{schema}"
        );
        assert!(a.table_schema("session", None, "Nope").is_err());
        assert!(a.table_schema("session", None, "x; drop table y").is_err());

        let shard = a.message_dbs()[0].to_string_lossy().to_string();
        let table = a.message_tables("wxid_bob").unwrap()[0].table.clone();
        let cols = a.message_table_columns(&shard, &table).unwrap();
        assert!(
            cols.as_array().unwrap().iter().any(|c| c == "local_id")
                && cols
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|c| c == "message_content")
        );
        assert!(a.message_table_columns(&shard, "Msg_nothing").is_err());
        assert!(a.list_tables("session", Some("../../x.db")).is_err());
    }

    #[test]
    fn message_meta_leaves_the_payload_out_and_pages() {
        let (a, _f) = account("meta");
        let first = a.message_tables("wxid_bob").unwrap().remove(0);
        let db = first.db.to_string_lossy().to_string();
        let all = a.message_meta(&db, &first.table, 100, 0).unwrap();
        let rows = all.as_array().unwrap();
        assert!(!rows.is_empty());
        assert!(rows[0].get("local_id").is_some() && rows[0].get("create_time").is_some());
        assert!(
            rows[0].get("message_content").is_none() && rows[0].get("compress_content").is_none()
        );
        let times: Vec<i64> = rows
            .iter()
            .map(|r| r["create_time"].as_i64().unwrap())
            .collect();
        assert!(
            times.windows(2).all(|w| w[0] <= w[1]),
            "oldest first: {times:?}"
        );
        let page = a.message_meta(&db, &first.table, 1, 1).unwrap();
        assert_eq!(page.as_array().unwrap().len(), 1);
        assert_eq!(page[0]["local_id"], rows[1]["local_id"]);
    }

    #[test]
    fn snapshots_are_written_outside_db_storage_only() {
        let (a, f) = account("snap");
        let out = f.root.join("snap.jsonl");
        let r = a
            .export_table_snapshot("session", None, "SessionTable", &out.to_string_lossy())
            .unwrap();
        assert_eq!(r["success"], true);
        assert_eq!(r["rows"], 3);
        let text = std::fs::read_to_string(&out).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 4, "header + 3 rows");
        let head: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(head["table"], "SessionTable");
        assert_eq!(
            head["columns"].as_array().unwrap().len() as i64,
            r["columns"].as_i64().unwrap()
        );
        let row: Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(
            row["row"].as_array().unwrap().len(),
            head["columns"].as_array().unwrap().len()
        );

        let inside = a.db_storage().join("copy.jsonl");
        assert!(a
            .export_table_snapshot("session", None, "SessionTable", &inside.to_string_lossy())
            .is_err());
        assert!(!inside.exists());
        assert!(a
            .export_table_snapshot("session", None, "SessionTable", " ")
            .is_err());
    }

    #[test]
    fn status_lists_databases_and_cache() {
        let (a, _f) = account("status");
        a.test_connection().unwrap();
        let s = a.db_status().unwrap();
        assert_eq!(
            (s["connected"].clone(), s["readOnly"].clone()),
            (json!(true), json!(true))
        );
        let paths: Vec<&str> = s["databases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["path"].as_str().unwrap())
            .collect();
        assert!(
            paths.contains(&"session/session.db") && paths.contains(&"hardlink/hardlink.db"),
            "{paths:?}"
        );
        assert!(s["cache"]["snapshots"].as_i64().unwrap() >= 1);
    }

    #[test]
    fn media_schema_summary_counts_voice_rows() {
        let (a, _f) = account("media");
        let media = a.media_dbs()[0].to_string_lossy().to_string();
        let s = a.media_schema_summary(&media).unwrap();
        assert_eq!(
            (s["hasVoiceInfo"].clone(), s["hasName2Id"].clone()),
            (json!(true), json!(true))
        );
        let voice = s["tables"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "VoiceInfo")
            .unwrap();
        assert_eq!(voice["rows"], 4);
        assert!(voice["columns"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c == "voice_data"));
    }

    #[test]
    fn avatars_come_back_as_hex() {
        let (a, _f) = account("avatar");
        let m = a
            .head_image_buffers(&["wxid_bob".into(), "wxid_carol".into(), "wxid_nobody".into()])
            .unwrap();
        assert_eq!(m["wxid_bob"], "ffd8ff01");
        assert_eq!(m["wxid_carol"], "010203");
        assert!(m.get("wxid_nobody").is_none());
        assert_eq!(a.head_image_buffers(&[]).unwrap(), json!({}));
    }

    #[test]
    fn image_hardlinks_prefer_hd_and_build_the_attach_path() {
        let (a, f) = account("img");
        let r = a
            .resolve_image_hardlink("AABBCCDDEEFF00112233445566778899", None)
            .unwrap();
        assert_eq!(
            r["file_name"], "0123456789abcdef0123456789abcdef_h.dat",
            "HD rendition wins"
        );
        let expected = f.account_dir.join("msg/attach/d41d8cd98f00b204e9800998ecf8427e/2023-11/Img/0123456789abcdef0123456789abcdef_h.dat");
        assert_eq!(r["full_path"], expected.to_string_lossy().as_ref());
        let elsewhere = a
            .resolve_image_hardlink("aabbccddeeff00112233445566778899", Some("/data/acct"))
            .unwrap();
        assert!(elsewhere["full_path"]
            .as_str()
            .unwrap()
            .starts_with("/data/acct/msg/attach/"));
        assert!(a
            .resolve_image_hardlink("00000000000000000000000000000000", None)
            .is_err());
        assert!(a.resolve_image_hardlink("  ", None).is_err());

        let batch = a.resolve_image_hardlink_batch(&json!([{ "md5": "aabbccddeeff00112233445566778899" }, { "md5": "ffffffffffffffffffffffffffffffff" }, { "md5": "" }])).unwrap();
        let rows = batch.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(
            (rows[0]["index"].clone(), rows[0]["success"].clone()),
            (json!(0), json!(true))
        );
        assert_eq!(rows[1]["success"], false);
        assert!(rows[1]["error"]
            .as_str()
            .unwrap()
            .contains("no hardlink record"));
        assert_eq!(rows[2]["success"], false);
        assert!(a
            .resolve_image_hardlink_batch(&json!({"md5": "x"}))
            .is_err());
    }

    #[test]
    fn video_hardlinks_resolve_the_file_stem() {
        let (a, f) = account("video");
        let r = a
            .resolve_video_hardlink_md5("11223344556677889900AABBCCDDEEFF", None)
            .unwrap();
        assert_eq!(r["resolved_md5"], "ffeeddccbbaa99887766554433221100");
        assert_eq!(
            r["full_path"],
            f.account_dir
                .join("msg/video/2023-11/ffeeddccbbaa99887766554433221100.mp4")
                .to_string_lossy()
                .as_ref()
        );
        assert!(a
            .resolve_video_hardlink_md5("deadbeefdeadbeefdeadbeefdeadbeef", None)
            .is_err());
        let rows = a
            .resolve_video_hardlink_md5_batch(
                &json!([{ "md5": "11223344556677889900aabbccddeeff" }, { "md5": "nope" }]),
            )
            .unwrap();
        assert_eq!(
            rows[0]["data"]["resolved_md5"],
            "ffeeddccbbaa99887766554433221100"
        );
        assert_eq!(rows[1]["success"], false);
    }
}
