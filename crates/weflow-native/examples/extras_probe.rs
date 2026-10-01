//! Exercise the introspection, hardlink and avatar functions on a real account (counts only, no content).
//! usage: extras_probe <account_dir> <key_file> <wxid>
use serde_json::Value;
use weflow_native::wcdb::Wcdb;

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (dir, keyf, wxid) = (a.next().unwrap(), a.next().unwrap(), a.next().unwrap());
    let mut db = Wcdb::new();
    db.open(std::path::Path::new(&dir), std::fs::read_to_string(keyf)?.trim(), Some(&wxid))?;

    let status = db.db_status()?;
    println!("db_status: {} databases, cache {}", status["databases"].as_array().map_or(0, Vec::len), status["cache"]);
    let tables = db.list_tables("session", "")?;
    println!("list_tables(session): {}", tables.as_array().map_or(0, Vec::len));
    println!("table_schema ok: {}", db.table_schema("contact", "", "contact")?["schema"].as_str().is_some_and(|s| s.starts_with("CREATE TABLE")));
    let shard = db.list_message_dbs()?[0].as_str().unwrap().to_string();
    let message_tables = db.list_tables("message", &shard)?;
    let table = message_tables.as_array().unwrap().iter().filter_map(Value::as_str).find(|t| t.starts_with("Msg_")).unwrap().to_string();
    println!("message_table_columns: {}", db.message_table_columns(&shard, &table)?.as_array().map_or(0, Vec::len));
    println!("message_meta rows: {}", db.message_meta(&shard, &table, 5, 0)?.as_array().map_or(0, Vec::len));
    let media = db.list_media_dbs()?;
    if let Some(m) = media.as_array().and_then(|l| l.first()).and_then(Value::as_str) {
        let s = db.media_schema_summary(m)?;
        println!("media_schema_summary: voice table {}, {} tables", s["hasVoiceInfo"], s["tables"].as_array().map_or(0, Vec::len));
    }
    let out = std::env::temp_dir().join("probe_snapshot.jsonl");
    let snap = db.export_table_snapshot("contact", "", "contact", &out.to_string_lossy())?;
    println!("export_table_snapshot: {} rows x {} columns, {} bytes", snap["rows"], snap["columns"], std::fs::metadata(&out)?.len());
    let _ = std::fs::remove_file(&out);

    let contacts = db.contacts()?;
    let names: Vec<String> = contacts.as_array().unwrap().iter().filter_map(|c| c["username"].as_str().map(str::to_string)).collect();
    let avatars = db.head_image_buffers(&serde_json::to_string(&names)?)?;
    let jpeg = avatars.as_object().unwrap().values().filter(|v| v.as_str().is_some_and(|h| h.starts_with("ffd8ff") || h.starts_with("89504e47"))).count();
    println!("head_image_buffers: {} avatars, {} look like JPEG/PNG", avatars.as_object().unwrap().len(), jpeg);

    // hardlinks: resolve what the hardlink database lists and check the files exist
    let rows = db.exec_query("hardlink", "", "select md5, type from image_hardlink_info_v4 where type <> 4 limit 400")?;
    let reqs: Vec<Value> = rows.as_array().unwrap().iter().map(|r| serde_json::json!({ "md5": r["md5"], "account_dir": dir })).collect();
    let resolved = db.resolve_image_hardlink_batch(&Value::Array(reqs).to_string())?;
    let list = resolved.as_array().unwrap();
    let ok = list.iter().filter(|r| r["success"] == true).count();
    let exist = list.iter().filter(|r| r["data"]["full_path"].as_str().is_some_and(|p| std::path::Path::new(p).exists())).count();
    println!("resolve_image_hardlink_batch: {} requests, {} resolved, {} files exist", list.len(), ok, exist);
    let rows = db.exec_query("hardlink", "", "select md5 from video_hardlink_info_v4")?;
    let reqs: Vec<Value> = rows.as_array().unwrap().iter().map(|r| serde_json::json!({ "md5": r["md5"] })).collect();
    let resolved = db.resolve_video_hardlink_md5_batch(&Value::Array(reqs).to_string())?;
    let list = resolved.as_array().unwrap();
    let exist = list.iter().filter(|r| r["data"]["full_path"].as_str().is_some_and(|p| std::path::Path::new(p).exists())).count();
    println!("resolve_video_hardlink_md5_batch: {} requests, {} resolved, {} files exist", list.len(), list.iter().filter(|r| r["success"] == true).count(), exist);
    println!("import refused: {}", db.import_table_snapshot("contact", "", "contact", "x").unwrap_err());
    Ok(())
}
