//! 打印 SQLite 换算出的本地时间（Unix 时间 0 对应的本地时刻）和环境变量 `TZ`，不打开任何文件或账号。
fn main() -> anyhow::Result<()> {
    let db = rusqlite::Connection::open_in_memory()?;
    let local_epoch: String =
        db.query_row("select datetime(0, 'unixepoch', 'localtime')", [], |row| {
            row.get(0)
        })?;
    println!(
        "{}",
        serde_json::json!({"TZ":std::env::var("TZ").ok(),"sqlite_local_epoch":local_epoch})
    );
    Ok(())
}
