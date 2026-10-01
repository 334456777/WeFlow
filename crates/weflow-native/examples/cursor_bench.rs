//! Time a full lite-cursor scan of one conversation (no content is printed).
//! usage: cursor_bench <account_dir> <key_file> <session id> [lite=1|0]
use std::time::Instant;
use weflow_native::native_db::NativeAccount;

fn main() -> anyhow::Result<()> {
    let mut a = std::env::args().skip(1);
    let (dir, keyf, sid) = (a.next().unwrap(), a.next().unwrap(), a.next().unwrap());
    let lite = a.next().map_or(true, |v| v != "0");
    let acct = NativeAccount::new(format!("{dir}/db_storage"), std::fs::read_to_string(keyf)?.trim())?.with_my_wxid(Some("wxid_example".into()));
    let t = Instant::now();
    acct.test_connection()?;
    let cursor = acct.open_message_cursor(&sid, 10_000, true, 0, 0, lite)?;
    println!("open cursor: {:.2}s", t.elapsed().as_secs_f64());
    let (mut rows, mut bytes) = (0usize, 0usize);
    let t = Instant::now();
    loop {
        let (batch, more) = acct.fetch_message_batch(cursor)?;
        let list = batch.as_array().cloned().unwrap_or_default();
        rows += list.len();
        bytes += list.iter().map(|r| r.to_string().len()).sum::<usize>();
        if !more || list.is_empty() {
            break;
        }
    }
    println!("fetched {rows} rows ({} MB of JSON) in {:.2}s", bytes / 1_000_000, t.elapsed().as_secs_f64());
    Ok(())
}
