//! Where the time of a search in one conversation goes, on an existing configured account (issue #153).
//! Usage: search_probe <isolated config.json> [keyword]
//! Searches the conversation with the most messages. Stdout holds only aggregate counts and timings: no key,
//! identifier, message content or local path is printed.

use anyhow::{Context, Result};
use std::path::Path;
use std::time::Instant;
use weflow_core::config::{expand_home, resolve_account_dir, ConfigStore};
use weflow_native::native_db::NativeAccount;

const KINDS: &str = "1,49,244813135921";
const PAGE: usize = 50;

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

fn ms(t: Instant) -> f64 {
    t.elapsed().as_secs_f64() * 1000.0
}

/// The text of `message_content`: zstd frames inflated, then UTF-8.
fn content(v: rusqlite::types::ValueRef<'_>) -> (String, bool) {
    match v {
        rusqlite::types::ValueRef::Text(t) => (String::from_utf8_lossy(t).into_owned(), false),
        rusqlite::types::ValueRef::Blob(b) => (
            zstd::stream::decode_all(b)
                .map(|d| String::from_utf8_lossy(&d).into_owned())
                .unwrap_or_default(),
            true,
        ),
        _ => (String::new(), false),
    }
}

/// The light scan: only the sort key and the text, matched here; `newest_first` stops at a page of hits.
/// Returns (ms, hits, rows read, blobs inflated, ms inflating).
fn light_scan(
    account: &NativeAccount,
    db: &Path,
    table: &str,
    needle: &str,
    newest_first: bool,
) -> Result<(f64, usize, usize, usize, f64)> {
    let order = if newest_first {
        " order by sort_seq desc"
    } else {
        ""
    };
    scan(
        account,
        db,
        &light_sql(table, "local_type", "", order),
        needle,
        newest_first,
    )
}

/// The light query; `kind` is `local_type`, or `+local_type` to keep SQLite off an index on it.
fn light_sql(table: &str, kind: &str, more: &str, order: &str) -> String {
    format!(
        "select sort_seq, local_id, message_content from \"{table}\" where {kind} in ({KINDS}) \
         and (typeof(message_content) = 'blob' or message_content like ?1){more}{order}"
    )
}

fn scan(
    account: &NativeAccount,
    db: &Path,
    sql: &str,
    needle: &str,
    newest_first: bool,
) -> Result<(f64, usize, usize, usize, f64)> {
    let like = format!("%{needle}%");
    let t = Instant::now();
    let (mut hits, mut rows, mut blobs, mut inflating) = (0, 0, 0, 0.0);
    account.with_db(db, |conn| {
        (hits, rows, blobs, inflating) = (0, 0, 0, 0.0);
        let mut stmt = conn.prepare(sql)?;
        let mut it = stmt.query([&like])?;
        while let Some(r) = it.next()? {
            rows += 1;
            let i = Instant::now();
            let (text, blob) = content(r.get_ref(2)?);
            if blob {
                blobs += 1;
                inflating += ms(i);
            }
            if text.to_lowercase().contains(needle) {
                hits += 1;
                if newest_first && hits >= PAGE {
                    break;
                }
            }
        }
        Ok(())
    })?;
    Ok((ms(t), hits, rows, blobs, inflating))
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        !args.is_empty(),
        "usage: search_probe <isolated config.json> [keyword]"
    );
    let keyword = args.get(1).map_or("好", String::as_str).to_lowercase();
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
    account
        .test_connection()
        .context("open configured account")?;

    let mut biggest = (0, String::new());
    for s in weflow_native::native_msg::session_usernames(&account)? {
        let n = account.message_count(&s).unwrap_or(0);
        if n > biggest.0 {
            biggest = (n, s);
        }
    }
    let (count, big) = biggest;
    let tables = account.message_tables(&big)?;
    println!("conversation: {count} messages in {} tables", tables.len());

    // what the tables hold
    for (i, t) in tables.iter().enumerate() {
        let start = Instant::now();
        let row = account.query(
            &t.db,
            &format!(
                "select count(*) as n, sum(local_type in ({KINDS})) as searchable, \
                 sum(local_type in ({KINDS}) and typeof(message_content) = 'blob') as blobs, \
                 sum(case when local_type in ({KINDS}) and typeof(message_content) = 'blob' then length(message_content) end) as blob_bytes, \
                 sum(case when local_type in ({KINDS}) and typeof(message_content) = 'text' then length(message_content) end) as text_chars \
                 from \"{}\"",
                t.table
            ),
            &[],
        )?;
        println!(
            "table {i}: {} (counted in {:.1} ms)",
            serde_json::to_string(&row[0])?,
            ms(start)
        );
    }

    // the search as it is (warm: the first run decrypts the pages)
    let mut times = Vec::new();
    let mut found = 0;
    for _ in 0..4 {
        let t = Instant::now();
        let page = account.search_messages(&keyword, Some(&big), PAGE as i32, 0, 0, 0)?;
        times.push(ms(t));
        found = page.as_array().map_or(0, Vec::len);
    }
    println!(
        "search_messages: first {:.1} ms, then median {:.1} ms, {found} hits",
        times[0],
        median(times[1..].to_vec())
    );

    // the same rows, only the key and the text
    for newest_first in [false, true] {
        let mut runs = Vec::new();
        let mut totals = (0, 0, 0, 0.0);
        for _ in 0..3 {
            let mut total = 0.0;
            totals = (0, 0, 0, 0.0);
            for t in &tables {
                let (took, hits, rows, blobs, inflating) =
                    light_scan(&account, &t.db, &t.table, &keyword, newest_first)?;
                total += took;
                totals.0 += hits;
                totals.1 += rows;
                totals.2 += blobs;
                totals.3 += inflating;
            }
            runs.push(total);
        }
        println!(
            "light scan{}: median {:.1} ms; {} hits, {} rows read, {} blobs inflated in {:.1} ms",
            if newest_first {
                ", newest first, stopping at a page"
            } else {
                ""
            },
            median(runs),
            totals.0,
            totals.1,
            totals.2,
            totals.3
        );
    }

    // how SQLite runs the newest-first query, and the same kept off an index on local_type
    let biggest_table = tables.last().context("no tables")?;
    for kind in ["local_type", "+local_type"] {
        let sql = light_sql(&biggest_table.table, kind, "", " order by sort_seq desc");
        let plan = account.query(
            &biggest_table.db,
            &format!("explain query plan {sql}"),
            &[&"%x%"],
        )?;
        let plan: Vec<String> = plan
            .iter()
            .map(|r| {
                r["detail"]
                    .as_str()
                    .unwrap_or("")
                    .replace(&biggest_table.table, "T")
            })
            .collect();
        let mut runs = Vec::new();
        let mut last = (0, 0);
        for _ in 0..3 {
            let mut total = 0.0;
            last = (0, 0);
            for t in &tables {
                let (took, hits, rows, _, _) = scan(
                    &account,
                    &t.db,
                    &light_sql(&t.table, kind, "", " order by sort_seq desc"),
                    &keyword,
                    true,
                )?;
                total += took;
                last.0 += hits;
                last.1 += rows;
            }
            runs.push(total);
        }
        println!(
            "newest first with `{kind}`: median {:.1} ms, {} hits, {} rows read; plan: {}",
            median(runs),
            last.0,
            last.1,
            plan.join(" / ")
        );
    }

    // the whole table in parts at once, as a keyword found nowhere needs
    let parts = std::thread::available_parallelism().map_or(4, |n| n.get());
    account.set_read_connections(parts);
    let mut runs = Vec::new();
    let mut hits = 0;
    for _ in 0..3 {
        let t = Instant::now();
        hits = 0;
        for table in &tables {
            let bounds = account.query(
                &table.db,
                &format!(
                    "select min(local_id) as lo, max(local_id) as hi from \"{}\"",
                    table.table
                ),
                &[],
            )?;
            let (lo, hi) = (
                bounds[0]["lo"].as_i64().unwrap_or(0),
                bounds[0]["hi"].as_i64().unwrap_or(0),
            );
            let step = (hi - lo) / parts as i64 + 1;
            let found: Vec<usize> = std::thread::scope(|s| {
                let all: Vec<_> = (0..parts as i64)
                    .map(|p| {
                        let (account, keyword) = (&account, &keyword);
                        s.spawn(move || {
                            let more = format!(
                                " and local_id between {} and {}",
                                lo + p * step,
                                lo + (p + 1) * step - 1
                            );
                            scan(
                                account,
                                &table.db,
                                &light_sql(&table.table, "local_type", &more, ""),
                                keyword,
                                false,
                            )
                            .map(|r| r.1)
                            .unwrap_or(0)
                        })
                    })
                    .collect();
                all.into_iter().map(|h| h.join().unwrap()).collect()
            });
            hits += found.iter().sum::<usize>();
        }
        runs.push(ms(t));
    }
    account.set_read_connections(1);
    println!(
        "whole tables in {parts} parts at once: median {:.1} ms, {hits} hits",
        median(runs)
    );

    // WeChat's own full-text index: its shape only
    let fts = account.db_path("message/message_fts.db");
    if fts.is_file() {
        let rows = account.query(
            &fts,
            "select type, name, coalesce(sql, '') as sql from sqlite_master",
            &[],
        )?;
        let mut shapes = std::collections::BTreeMap::<String, usize>::new();
        for r in &rows {
            // segments that may be hashes or ids are left out
            let name = r["name"]
                .as_str()
                .unwrap_or("")
                .split('_')
                .map(|seg| {
                    if seg.len() > 12 || seg.chars().any(|c| !c.is_ascii_alphabetic()) {
                        "#"
                    } else {
                        seg
                    }
                })
                .collect::<Vec<_>>()
                .join("_");
            let sql = r["sql"].as_str().unwrap_or("").to_lowercase();
            let using = sql
                .split_once("using")
                .map(|(_, u)| u.trim().chars().take(300).collect::<String>())
                .unwrap_or_default();
            *shapes
                .entry(format!(
                    "{} {name} {using}",
                    r["type"].as_str().unwrap_or("")
                ))
                .or_default() += 1;
        }
        println!("message_fts.db: {} objects", rows.len());
        for (shape, n) in shapes {
            println!("  {n} × {shape}");
        }
    }
    Ok(())
}
