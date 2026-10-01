//! Print table/column structure and row counts of one database (never row contents).
//! usage: schema_probe <account_dir> <key_file> <db path relative to db_storage>
use weflow_native::native_db::NativeAccount;

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (dir, keyf, rel) = (a.next().unwrap(), a.next().unwrap(), a.next().unwrap());
    let acct = NativeAccount::new(format!("{dir}/db_storage"), std::fs::read_to_string(keyf)?.trim())?;
    let db = acct.db_path(&rel);
    for t in acct.query(&db, "select name, sql from sqlite_master where type = 'table' order by name", &[])? {
        let name = t["name"].as_str().unwrap();
        if ["_content", "_data", "_idx", "_docsize", "_config"].iter().any(|s| name.ends_with(s)) {
            continue;
        }
        let n = acct
            .query(&db, &format!("select count(*) as n from \"{name}\""), &[])
            .map(|r| r[0]["n"].to_string())
            .unwrap_or_else(|_| "?".into());
        println!("{name} [{n} rows]: {}", t["sql"].as_str().unwrap_or("").split_whitespace().collect::<Vec<_>>().join(" "));
    }
    Ok(())
}
