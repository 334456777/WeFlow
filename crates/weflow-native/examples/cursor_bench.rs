//! 测量用游标完整扫描一个会话的耗时，不打印任何内容。默认只读精简的列（`Columns::Lite`），`lite=0` 时读全部的列。
//! 用法：`cursor_bench <account_dir> <key_file> <session id> [lite=1|0] [your wxid]`
#![expect(clippy::unwrap_used, reason = "探针用 panic 报告失败")]
use std::time::Instant;
use weflow_native::db::Account;
use weflow_native::msg::Order;
use weflow_native::msg_row::Columns;

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (dir, keyf, sid) = (a.next().unwrap(), a.next().unwrap(), a.next().unwrap());
    let columns = if a.next().is_none_or(|v| v != "0") {
        Columns::Lite
    } else {
        Columns::All
    };
    let my_wxid = a.next();
    let acct = Account::new(
        format!("{dir}/db_storage"),
        std::fs::read_to_string(keyf)?.trim(),
    )?
    .with_my_wxid(my_wxid);
    let t = Instant::now();
    acct.test_connection()?;
    let cursor = acct.open_message_cursor(&sid, 10_000, Order::OldestFirst, 0, 0, columns)?;
    println!("open cursor: {:.2}s", t.elapsed().as_secs_f64());
    let (mut rows, mut bytes) = (0usize, 0usize);
    let t = Instant::now();
    loop {
        let (list, more) = acct.fetch_message_batch(cursor)?;
        rows += list.len();
        bytes += list.iter().map(|r| r.content.len()).sum::<usize>();
        if !more || list.is_empty() {
            break;
        }
    }
    println!(
        "fetched {rows} rows ({} MB of text) in {:.2}s",
        bytes / 1_000_000,
        t.elapsed().as_secs_f64()
    );
    Ok(())
}
