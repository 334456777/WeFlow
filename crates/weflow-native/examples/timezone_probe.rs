//! Synthetic SQLite local-time probe. No files or accounts are opened.
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
