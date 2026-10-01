//! Run one read-only SQL statement against a database and print the rows as JSON.
//! usage: sql_probe <account_dir> <key_file> <db path relative to db_storage> "<sql>"
//! Only select aggregates/structure here: rows are printed verbatim.
use weflow_native::native_db::NativeAccount;

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (dir, keyf, rel, sql) = (
        a.next().unwrap(),
        a.next().unwrap(),
        a.next().unwrap(),
        a.next().unwrap(),
    );
    let acct = NativeAccount::new(
        format!("{dir}/db_storage"),
        std::fs::read_to_string(keyf)?.trim(),
    )?;
    let rows = acct.query(&acct.db_path(&rel), &sql, &[])?;
    println!("{}", serde_json::to_string(&rows)?);
    Ok(())
}
