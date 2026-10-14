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

/// (sort_seq, local_id) of a message, the key that matches it to WeChat's index.
type Key = (i64, i64);

/// The rows of WeChat's full-text index for one conversation: key, local_type, text.
fn fts_rows(
    account: &NativeAccount,
    fts: &Path,
    table: &str,
    session: &rusqlite::types::Value,
) -> Result<Vec<(Key, i64, String)>> {
    account.with_db(fts, |conn| {
        let mut stmt = conn.prepare(&format!(
            "select c2, c1, c3, c0 from \"{table}\" where c4 = ?1"
        ))?;
        let rows = stmt
            .query_map([session], |r| {
                Ok((
                    (
                        r.get::<_, Option<i64>>(0)?.unwrap_or(0),
                        r.get::<_, Option<i64>>(1)?.unwrap_or(0),
                    ),
                    r.get::<_, Option<i64>>(2)?.unwrap_or(0),
                    content(r.get_ref(3)?).0,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    })
}

/// WeChat's index `message/message_fts.db` against the messages of conversation `big`: its shape, which rows it
/// holds and how they map to the messages, how its text compares, and how fast a LIKE over its plain `_content`
/// tables is. Only counts and timings are printed; table names are replaced by their position.
fn fts_probe(
    account: &NativeAccount,
    big: &str,
    tables: &[weflow_native::native_msg::MsgTable],
    keyword: &str,
) -> Result<()> {
    let fts = account.db_path("message/message_fts.db");
    anyhow::ensure!(fts.is_file(), "no message_fts.db");
    let names: Vec<String> = account
        .query(
            &fts,
            "select name from sqlite_master where type = 'table' order by name",
            &[],
        )?
        .iter()
        .filter_map(|r| r["name"].as_str().map(str::to_string))
        .collect();
    // the tables beside the index groups: columns and rows
    for (i, name) in names.iter().enumerate() {
        let is_group = name.starts_with("message_fts_")
            && ["_content", "_data", "_idx", "_docsize", "_config", "aux"]
                .iter()
                .any(|s| name.ends_with(s));
        if is_group || name.contains("img") {
            continue;
        }
        let cols: Vec<String> = account
            .query(&fts, &format!("pragma table_info(\"{name}\")"), &[])?
            .iter()
            .filter_map(|r| r["name"].as_str().map(str::to_string))
            .collect();
        let n = account.query(&fts, &format!("select count(*) as n from \"{name}\""), &[])?;
        let kind = if name.ends_with("_range") {
            "…_range"
        } else {
            name.as_str()
        };
        // a name of only letters and underscores tells nothing private; others are named by kind
        let shown = if kind
            .chars()
            .all(|c| c.is_ascii_alphabetic() || c == '_' || c == '…')
        {
            kind.to_string()
        } else {
            format!("table #{i}")
        };
        println!("  {shown}: columns {cols:?}, {} rows", n[0]["n"]);
    }

    // which value of c4 (session_id) is this conversation: its user name, or a Name2Id row of a message database
    let mut ids = std::collections::BTreeSet::new();
    for t in tables {
        for r in account.query(
            &t.db,
            "select rowid as id from Name2Id where user_name = ?1",
            &[&big],
        )? {
            if let Some(id) = r["id"].as_i64() {
                ids.insert(id);
            }
        }
    }
    let contents: Vec<&String> = names
        .iter()
        .filter(|n| n.starts_with("message_fts_") && n.ends_with("_content"))
        .collect();
    let mut keys: Vec<(usize, rusqlite::types::Value)> = Vec::new();
    for (g, content) in contents.iter().enumerate() {
        let cols: Vec<String> = account
            .query(&fts, &format!("pragma table_info(\"{content}\")"), &[])?
            .iter()
            .filter_map(|r| r["name"].as_str().map(str::to_string))
            .collect();
        let shape = account.query(
            &fts,
            &format!(
                "select count(*) as n, max(c2) as max_seq, sum(typeof(c4) = 'text') as text_ids, \
                 sum(typeof(c4) = 'integer') as int_ids, count(distinct c4) as sessions from \"{content}\""
            ),
            &[],
        )?;
        let by_name = account.query(
            &fts,
            &format!("select count(*) as n from \"{content}\" where c4 = ?1"),
            &[&big],
        )?[0]["n"]
            .as_i64()
            .unwrap_or(0);
        let mut by_id = Vec::new();
        for id in &ids {
            let n = account.query(
                &fts,
                &format!("select count(*) as n from \"{content}\" where c4 = ?1"),
                &[id],
            )?[0]["n"]
                .as_i64()
                .unwrap_or(0);
            by_id.push(n);
            if n > 0 {
                keys.push((g, rusqlite::types::Value::Integer(*id)));
            }
        }
        if by_name > 0 {
            keys.push((g, rusqlite::types::Value::Text(big.to_string())));
        }
        println!(
            "group {g}: columns {cols:?}; {}; this conversation by name {by_name}, by Name2Id rows {by_id:?}",
            serde_json::to_string(&shape[0])?
        );
    }

    // the messages, keyed as the index keys them
    let kinds: Vec<i64> = KINDS.split(',').map(|k| k.parse().unwrap()).collect();
    let mut messages = std::collections::HashMap::<Key, (i64, String)>::new();
    // every message, with the table it is in
    let mut all = std::collections::HashMap::<Key, (i64, String, usize)>::new();
    let mut max_msg_seq = 0;
    let mut max_local_id = vec![0i64; tables.len()];
    for (ti, t) in tables.iter().enumerate() {
        account.with_db(&t.db, |conn| {
            let mut stmt = conn.prepare(&format!(
                "select sort_seq, local_id, local_type, message_content from \"{}\"",
                t.table
            ))?;
            let mut it = stmt.query([])?;
            while let Some(r) = it.next()? {
                let seq = r.get::<_, Option<i64>>(0)?.unwrap_or(0);
                max_msg_seq = max_msg_seq.max(seq);
                let kind = r.get::<_, Option<i64>>(2)?.unwrap_or(0);
                let key = (seq, r.get::<_, Option<i64>>(1)?.unwrap_or(0));
                max_local_id[ti] = max_local_id[ti].max(key.1);
                let text = content(r.get_ref(3)?).0;
                if kinds.contains(&kind) {
                    messages.insert(key, (kind, text.clone()));
                }
                all.insert(key, (kind, text, ti));
            }
            Ok(())
        })?;
    }
    let files: Vec<String> = tables
        .iter()
        .map(|t| {
            t.db.file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_default()
        })
        .collect();
    println!("message tables in {files:?}, newest local_id {max_local_id:?}");
    let mut indexed = std::collections::HashMap::<Key, (i64, String)>::new();
    let mut max_fts_seq = 0;
    for (g, key) in &keys {
        let mut by_table = std::collections::BTreeMap::<String, usize>::new();
        for (k, kind, text) in fts_rows(account, &fts, contents[*g], key)? {
            max_fts_seq = max_fts_seq.max(k.0);
            let at = all
                .get(&k)
                .map_or("no message".to_string(), |m| files[m.2].clone());
            *by_table.entry(at).or_default() += 1;
            indexed.insert(k, (kind, text));
        }
        // the short numbers in the group's name, which may say which database it indexes
        let numbers: Vec<&str> = contents[*g]
            .split('_')
            .filter(|s| !s.is_empty() && s.len() <= 3 && s.chars().all(|c| c.is_ascii_digit()))
            .collect();
        println!("group {g} (numbers in its name {numbers:?}): rows by message table {by_table:?}");
    }
    // the `_range` table: what it holds for this conversation
    if let Some(range) = names.iter().find(|n| n.ends_with("_range")) {
        for (_, key) in &keys {
            let r = account.with_db(&fts, |conn| {
                Ok(conn.query_row(
                    &format!(
                        "select count(*), min(start_local_id), max(end_local_id), max(db_time_stamp) from \"{range}\" where session_id = ?1"
                    ),
                    [key],
                    |r| {
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, Option<i64>>(1)?,
                            r.get::<_, Option<i64>>(2)?,
                            r.get::<_, Option<i64>>(3)?.is_some(),
                        ))
                    },
                )?)
            })?;
            println!(
                "  _range for this conversation: rows {}, start {:?}, end {:?}, has a time {}",
                r.0, r.1, r.2, r.3
            );
        }
        let types = account.query(
            &fts,
            &format!("select typeof(session_id) as t, count(*) as n from \"{range}\" group by 1"),
            &[],
        )?;
        println!(
            "  _range session_id types: {}",
            serde_json::to_string(&types)?
        );
    }
    // rows only in the index: messages of other types, or none at all
    let mut extra = std::collections::BTreeMap::<&str, usize>::new();
    let mut titles = std::collections::BTreeMap::<i64, (usize, usize)>::new();
    for (k, (kind, text)) in &indexed {
        let Some((_, xml, _)) = all.get(k) else {
            *extra.entry("no message").or_default() += 1;
            continue;
        };
        if !messages.contains_key(k) {
            *extra
                .entry("message of a type not searched today")
                .or_default() += 1;
        }
        // is the indexed text the message's <title>?
        let escaped = text
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;");
        let entry = titles.entry(*kind).or_default();
        entry.1 += 1;
        if xml.contains(&format!("<title>{text}</title>"))
            || xml.contains(&format!("<title>{escaped}</title>"))
            || xml.contains(&format!("<title><![CDATA[{text}]]></title>"))
        {
            entry.0 += 1;
        }
    }
    println!("  rows only in the index: {extra:?}");
    // the indexed text against the <title> and <des> of the message XML, entities decoded
    let tag = |xml: &str, name: &str| -> Option<String> {
        let open = format!("<{name}>");
        let start = xml.find(&open)? + open.len();
        let end = xml[start..].find(&format!("</{name}>"))? + start;
        let raw = xml[start..end].trim();
        let raw = raw
            .strip_prefix("<![CDATA[")
            .and_then(|r| r.strip_suffix("]]>"))
            .unwrap_or(raw);
        Some(
            raw.replace("&lt;", "<")
                .replace("&gt;", ">")
                .replace("&quot;", "\"")
                .replace("&apos;", "'")
                .replace("&amp;", "&"),
        )
    };
    let mut how = std::collections::BTreeMap::<(bool, &str), usize>::new();
    for (k, (kind, text)) in &indexed {
        if *kind == 1 {
            continue;
        }
        let Some((_, xml, _)) = all.get(k) else {
            continue;
        };
        let (title, des) = (tag(xml, "title"), tag(xml, "des"));
        let what = match (&title, &des) {
            _ if text.is_empty() => "empty",
            (Some(t), _) if t == text => "title",
            (Some(t), Some(d)) if format!("{t}{d}") == *text || format!("{t} {d}") == *text => {
                "title and des"
            }
            (Some(t), _) if text.starts_with(t.as_str()) => "starts with the title",
            (Some(t), _) if t.contains(text.as_str()) => "part of the title",
            (None, _) => "no title",
            _ => "other",
        };
        *how.entry((*kind == 244_813_135_921, what)).or_default() += 1;
    }
    println!("  indexed text of non-text messages (is a quote, how): {how:?}");
    // a table of user names in the index database: is session_id its row?
    for (i, name) in names.iter().enumerate() {
        let cols: Vec<String> = account
            .query(&fts, &format!("pragma table_info(\"{name}\")"), &[])?
            .iter()
            .filter_map(|r| r["name"].as_str().map(str::to_string))
            .collect();
        if cols == ["username"] {
            let row = account.query(
                &fts,
                &format!("select rowid as id from \"{name}\" where username = ?1"),
                &[&big],
            )?;
            let id = row.first().and_then(|r| r["id"].as_i64());
            println!(
                "  table #{i} (username): this conversation's row {}; it is the session_id that matched: {}",
                if id.is_some() { "found" } else { "missing" },
                keys.iter()
                    .any(|(_, k)| id.is_some_and(|id| *k == rusqlite::types::Value::Integer(id)))
            );
        }
    }
    println!("  indexed text that is the message's <title>, by type (title, all): {titles:?}");
    println!(
        "searchable messages {}, rows in the index {}; newest sort_seq: messages {max_msg_seq}, index {max_fts_seq}",
        messages.len(),
        indexed.len()
    );
    let kinds_of = |rows: &std::collections::HashMap<Key, (i64, String)>| {
        let mut by = std::collections::BTreeMap::<i64, usize>::new();
        for (kind, _) in rows.values() {
            *by.entry(*kind).or_default() += 1;
        }
        by
    };
    println!(
        "  by type: messages {:?}, index {:?}",
        kinds_of(&messages),
        kinds_of(&indexed)
    );
    let (mut both, mut only_msg, mut only_msg_newer, mut only_fts) = (0, 0, 0, 0);
    let mut only_msg_kinds = std::collections::BTreeMap::<i64, usize>::new();
    let mut same = std::collections::BTreeMap::<(i64, &str), usize>::new();
    for (k, (kind, text)) in &messages {
        match indexed.get(k) {
            Some((_, indexed_text)) => {
                both += 1;
                let tail = text.split_once(":\n").map_or("", |(_, t)| t);
                let how = if indexed_text == text {
                    "equal"
                } else if !tail.is_empty() && indexed_text == tail {
                    "equal without the sender prefix"
                } else if text.contains(indexed_text.as_str()) {
                    "part of the message"
                } else if indexed_text.is_empty() {
                    "empty in the index"
                } else {
                    "different"
                };
                *same.entry((*kind, how)).or_default() += 1;
            }
            None => {
                only_msg += 1;
                *only_msg_kinds.entry(*kind).or_default() += 1;
                if k.0 > max_fts_seq {
                    only_msg_newer += 1;
                }
            }
        }
    }
    for k in indexed.keys() {
        if !messages.contains_key(k) {
            only_fts += 1;
        }
    }
    println!(
        "  matched {both}; only in the messages {only_msg} (by type {only_msg_kinds:?}, newer than the index {only_msg_newer}); only in the index {only_fts}"
    );
    println!("  text of matched rows (type, how): {same:?}");

    // the hits of a search, by today's rule (the message text contains the word) and by the index's text
    for word in [keyword, "qzxwvk"] {
        let hit = |rows: &std::collections::HashMap<Key, (i64, String)>| -> std::collections::HashSet<Key> {
            rows.iter()
                .filter(|(_, (_, t))| t.to_lowercase().contains(word))
                .map(|(k, _)| *k)
                .collect()
        };
        let (a, b) = (hit(&messages), hit(&indexed));
        let mut a_only = std::collections::BTreeMap::<i64, usize>::new();
        for k in a.difference(&b) {
            *a_only.entry(messages[k].0).or_default() += 1;
        }
        println!(
            "  word #{}: hits today {}, in the index {}, both {}; only today by type {a_only:?}; only in the index {}",
            if word == keyword { 1 } else { 2 },
            a.len(),
            b.len(),
            a.intersection(&b).count(),
            b.difference(&a).count()
        );
        // a LIKE over the plain tables, newest first, as a search would run it
        let like = format!("%{word}%");
        let mut runs = Vec::new();
        let mut found = 0;
        for _ in 0..3 {
            let t = Instant::now();
            found = 0;
            for (g, key) in &keys {
                account.with_db(&fts, |conn| {
                    let mut stmt = conn.prepare(&format!(
                        "select c2, c1 from \"{}\" where c4 = ?1 and c0 like ?2 order by c2 desc",
                        contents[*g]
                    ))?;
                    found += stmt
                        .query_map(rusqlite::params![key, like], |_| Ok(()))?
                        .count();
                    Ok(())
                })?;
            }
            runs.push(ms(t));
        }
        println!(
            "    LIKE over the index: median {:.1} ms (first {:.1} ms), {found} rows",
            median(runs.clone()),
            runs[0]
        );
    }
    if let Some((g, _)) = keys.first() {
        let plan = account.query(
            &fts,
            &format!(
                "explain query plan select c2, c1 from \"{}\" where c4 = ?1 and c0 like ?2 order by c2 desc",
                contents[*g]
            ),
            &[&big, &"%x%"],
        )?;
        let plan: Vec<String> = plan
            .iter()
            .map(|r| {
                r["detail"]
                    .as_str()
                    .unwrap_or("")
                    .replace(contents[*g].as_str(), "T")
            })
            .collect();
        println!("  plan: {}", plan.join(" / "));
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    anyhow::ensure!(
        !args.is_empty(),
        "usage: search_probe <isolated config.json> [keyword] [fts]"
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
    if args.get(2).map(String::as_str) == Some("fts") {
        return fts_probe(&account, &big, &tables, &keyword);
    }
    if args.get(2).map(String::as_str) == Some("time") {
        // only the search itself, in this process, as `/api/` runs it
        for (name, word) in [("common", keyword.as_str()), ("absent", "qzxwvk")] {
            for session in [Some(big.as_str()), None] {
                let mut times = Vec::new();
                let mut found = 0;
                for _ in 0..6 {
                    let t = Instant::now();
                    let page = account.search_messages(word, session, 20, 0, 0, 0)?;
                    times.push(ms(t));
                    found = page.as_array().map_or(0, Vec::len);
                }
                println!(
                    "{name} word, {}: first {:.1} ms, then median {:.1} ms, {found} hits",
                    if session.is_some() {
                        "one conversation"
                    } else {
                        "all"
                    },
                    times[0],
                    median(times[1..].to_vec())
                );
            }
        }
        return Ok(());
    }

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
