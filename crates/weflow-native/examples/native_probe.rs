//! Read-only probe of the native backend against a real account (prints counts, never message text).
//! usage: native_probe <account_dir> <key_file>
use std::time::Instant;
use weflow_native::native_db::NativeAccount;

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let account = args.next().expect("account_dir");
    let key = std::fs::read_to_string(args.next().expect("key_file"))?;
    let acct = NativeAccount::new(format!("{account}/db_storage"), key.trim())?;

    let t = Instant::now();
    acct.test_connection()?;
    println!("session.db opened in {:?}", t.elapsed());

    let sessions = acct.sessions()?;
    let rows = sessions.as_array().unwrap();
    println!("SessionTable rows: {}", rows.len());
    for r in rows.iter().take(3) {
        println!(
            "  {} ts={}",
            r["username"]
                .as_str()
                .unwrap_or("")
                .chars()
                .take(6)
                .collect::<String>(),
            r["sort_timestamp"]
        );
    }

    let t = Instant::now();
    let msg = acct.db_path("message/message_1.db");
    let tables = acct.query(
        &msg,
        "select name from sqlite_master where type='table' and name like 'Msg_%'",
        &[],
    )?;
    println!(
        "message_1.db opened in {:?}, Msg_* tables: {}",
        t.elapsed(),
        tables.len()
    );
    let (mut total, mut zstd_ok, mut text) = (0u64, 0u64, 0u64);
    for t in tables.iter().take(10) {
        let name = t["name"].as_str().unwrap();
        let rows = acct.query(
            &msg,
            &format!("select local_type, message_content from \"{name}\" limit 200"),
            &[],
        )?;
        for r in rows {
            total += 1;
            let c = r["message_content"].as_str().unwrap_or("");
            if c.contains('<') || !c.is_empty() {
                text += 1;
            }
            if !c.is_empty() && !c.chars().all(|ch| ch.is_ascii_hexdigit()) {
                zstd_ok += 1;
            }
        }
    }
    println!("sampled {total} messages, {text} non-empty, {zstd_ok} readable (not hex fallback)");

    let contacts = acct.query(
        &acct.db_path("contact/contact.db"),
        "select count(*) as n from contact",
        &[],
    )?;
    println!("contact rows: {}", contacts[0]["n"]);
    Ok(())
}
