//! Read-only inventory and cursor comparison on an existing configured account.
//! Usage: real_account_probe <isolated config.json> <private inventory.json>
//! The inventory contains conversation identifiers and dates and must remain private. Stdout contains only
//! aggregate counts. No key, identifier, message content or local account path is printed.

use anyhow::{Context, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::time::Instant;
use weflow_core::config::{expand_home, resolve_account_dir, ConfigStore};
use weflow_native::native_db::NativeAccount;

fn number(row: &Value, field: &str) -> i64 {
    row[field].as_i64().unwrap_or(0)
}

fn cursor_digest(account: &NativeAccount, cursor: i64) -> Result<(usize, String)> {
    let mut count = 0;
    let mut hash = Sha256::new();
    let result = (|| -> Result<()> {
        loop {
            let (page, more) = account.fetch_message_batch(cursor)?;
            let rows = page.as_array().context("cursor did not return rows")?;
            count += rows.len();
            for row in rows {
                hash.update(serde_json::to_vec(row)?);
            }
            if !more {
                break;
            }
        }
        Ok(())
    })();
    account.close_message_cursor(cursor)?;
    result?;
    Ok((count, weflow_native::native_db::hex(&hash.finalize())))
}

fn measure_cursors(
    account: &NativeAccount,
    cases: &Path,
    output: &Path,
    runs: usize,
    owner: &str,
) -> Result<()> {
    anyhow::ensure!(runs > 0, "runs must be positive");
    let cases: Vec<Value> = serde_json::from_slice(&std::fs::read(cases)?)?;
    let mut samples = Vec::new();
    for case in cases {
        let session = case["session"].as_str().context("case session missing")?;
        let expected = number(&case, "count") as usize;
        let warm = account.open_message_cursor(session, 2000, true, 0, 0, false)?;
        anyhow::ensure!(
            cursor_digest(account, warm)?.0 == expected,
            "warm-up count differs"
        );
        for kind in ["exact", "export", "sender"] {
            for run in 1..=runs {
                let started = Instant::now();
                let cursor = match kind {
                    "export" => account.open_export_cursor(session, 2000, 0, 0)?,
                    "sender" => account.open_message_cursor_for_senders(
                        session,
                        2000,
                        true,
                        (0, 0),
                        false,
                        (
                            &|name| weflow_core::message::is_same_wxid(name, owner),
                            true,
                        ),
                    )?,
                    _ => account.open_message_cursor(session, 2000, true, 0, 0, false)?,
                };
                let keys = started.elapsed();
                let first = Instant::now();
                let _ = account.fetch_message_page(cursor, 0)?;
                let first = first.elapsed();
                let signature = cursor_digest(account, cursor)?;
                if kind != "sender" {
                    anyhow::ensure!(signature.0 == expected, "cursor count differs");
                }
                samples.push(json!({"case": case["label"], "cursor": kind, "run": run,
                    "count": signature.0, "sha256": signature.1, "keys_ms": keys.as_secs_f64()*1000.0,
                    "first_internal_batch_ms": (keys + first).as_secs_f64()*1000.0}));
            }
        }
    }
    std::fs::write(output, serde_json::to_vec(&samples)?)?;
    println!(
        "{}",
        json!({"cursor_samples": samples.len(), "stored_locally": true})
    );
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let measured = (args.len() == 4 || args.len() == 5) && args[2] == "--measure-cursors";
    anyhow::ensure!(args.len() == 2 || (args.len() == 3 && args[2] == "--compare-bounded") || measured,
        "usage: real_account_probe <isolated config.json> <private output.json> [--compare-bounded | --measure-cursors <private cases.json> [runs]]");
    let config = ConfigStore::load(Path::new(&args[0])).context("read isolated account config")?;
    let profile = config
        .profiles
        .get(&config.current_profile)
        .context("current profile missing")?;
    let root = expand_home(
        profile
            .db_path
            .as_deref()
            .context("database path missing")?,
    );
    let root = match &profile.wxid {
        Some(owner) => resolve_account_dir(&root.to_string_lossy(), owner),
        None => root,
    };
    let account = NativeAccount::new(
        root.join("db_storage"),
        profile
            .decrypt_key
            .as_deref()
            .context("database key missing")?,
    )?
    .with_my_wxid(profile.wxid.clone());
    account.set_low_memory(true);
    account
        .test_connection()
        .context("open configured account")?;
    if measured {
        let runs = args.get(4).map_or(Ok(5), |n| n.parse::<usize>())?;
        return measure_cursors(
            &account,
            Path::new(&args[3]),
            Path::new(&args[1]),
            runs,
            profile.wxid.as_deref().unwrap_or(""),
        );
    }
    let sessions = weflow_native::native_msg::session_usernames(&account)?;
    let mut inventory = Vec::new();
    let mut total = 0i64;
    let mut mismatches = 0i64;
    let mut unusual_times = 0i64;
    let mut media = 0i64;
    let mut bounded_checks = Vec::new();
    for session in sessions {
        let mut count = 0i64;
        let mut minimum = i64::MAX;
        let mut maximum = 0i64;
        let mut broken = 0i64;
        let mut unusual = 0i64;
        let mut kinds = Vec::new();
        let mut anomalies = Vec::new();
        for table in account.message_tables(&session)? {
            let sql = format!("select count(*) as n, min(create_time) as lo, max(create_time) as hi,
                sum(case when sort_seq > 0 and sort_seq / 1000 != create_time then 1 else 0 end) as broken,
                sum(case when typeof(create_time) != 'integer' or create_time <= 0 then 1 else 0 end) as unusual
                from \"{}\"", table.table);
            let rows = account.query(&table.db, &sql, &[])?;
            let row = &rows[0];
            count += number(row, "n");
            if number(row, "n") > 0 {
                minimum = minimum.min(number(row, "lo"));
                maximum = maximum.max(number(row, "hi"));
            }
            broken += number(row, "broken");
            unusual += number(row, "unusual");
            if number(row, "broken") > 0 {
                let sql = format!("select local_id, sort_seq, create_time from \"{}\" where sort_seq > 0 and sort_seq / 1000 != create_time limit 10", table.table);
                anomalies.extend(account.query(&table.db, &sql, &[])?);
            }
            let sql = format!(
                "select local_type as kind, count(*) as n from \"{}\" group by local_type",
                table.table
            );
            for kind in account.query(&table.db, &sql, &[])? {
                if [3, 34, 43, 47].contains(&number(&kind, "kind")) {
                    media += number(&kind, "n");
                }
                kinds.push(kind);
            }
        }
        if count > 0 {
            if args.len() == 3 {
                // Compare fresh index-derived cursors before reading any complete export, which could remember
                // that this table needs exact keys and hide the bounded-range bug on subsequent commands.
                for anomaly in &anomalies {
                    let time = i32::try_from(number(anomaly, "create_time"))?;
                    let fast = account.open_export_cursor(&session, 2000, time, time)?;
                    let indexed = cursor_digest(&account, fast)?;
                    let exact =
                        account.open_message_cursor(&session, 2000, true, time, time, false)?;
                    let exact = cursor_digest(&account, exact)?;
                    bounded_checks.push(
                        json!({"session": session, "time": time, "indexed_count": indexed.0,
                        "exact_count": exact.0, "ordered_rows_match": indexed == exact}),
                    );
                }
            }
            total += count;
            mismatches += broken;
            unusual_times += unusual;
            inventory.push(
                json!({"session": session, "count": count, "first": minimum, "last": maximum,
                "derived_time_mismatches": broken, "unusual_time_rows": unusual, "kinds": kinds,
                "anomalies": anomalies}),
            );
        }
    }
    inventory.sort_by_key(|row| std::cmp::Reverse(number(row, "count")));
    let path = Path::new(&args[1]);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(
        path,
        serde_json::to_vec(&json!({"conversations": inventory, "bounded_checks": bounded_checks}))?,
    )?;
    println!(
        "{}",
        json!({"nonempty_conversations": inventory.len(), "messages": total,
        "largest_conversation_messages": inventory.first().map(|r| number(r, "count")).unwrap_or(0),
        "derived_time_mismatches": mismatches, "unusual_time_rows": unusual_times, "media_messages": media,
        "bounded_checks": bounded_checks.len(), "bounded_mismatches": bounded_checks.iter().filter(|check| check["ordered_rows_match"] == false).count()})
    );
    Ok(())
}
