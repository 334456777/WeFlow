//! 在一个数据库上运行一条只读 SQL 语句，把读出的行打印成 JSON。
//! 用法：`sql_probe <account_dir> <key_file> <db path relative to db_storage> "<sql>"`
//! 行会原样打印，所以只查询汇总和表结构。
#![expect(clippy::unwrap_used, reason = "探针用 panic 报告失败")]
use weflow_native::db::Account;

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (dir, keyf, rel, sql) = (
        a.next().unwrap(),
        a.next().unwrap(),
        a.next().unwrap(),
        a.next().unwrap(),
    );
    let acct = Account::new(
        format!("{dir}/db_storage"),
        std::fs::read_to_string(keyf)?.trim(),
    )?;
    let rows = acct.query(&acct.db_path(&rel), &sql, &[])?;
    println!("{}", serde_json::to_string(&rows)?);
    Ok(())
}
