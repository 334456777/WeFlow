//! Where WeChat keeps that a conversation is pinned (#885): the bits of `SessionTable.status` and of `contact.flag`
//! over the conversation list, and the first conversations by their place, kind and bits.
//! Usage: `pinned_probe <config.json>`
//! No name, WeChat ID, message, key or path is printed: a conversation is its place in the list (by `sort_timestamp`),
//! its kind and a short hash of its username, so that two runs (before and after pinning one in WeChat) can be told
//! apart. The config and the databases are only read.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use anyhow::{Context, Result};
use serde_json::Value;
use weflow_core::config::{expand_home, resolve_account_dir, ConfigStore};
use weflow_native::db::Account;

/// The conversations listed one by one: the first ones, where WeChat shows the pinned.
const SHOWN: usize = 40;

fn int(row: &Value, key: &str) -> i64 {
    match &row[key] {
        Value::Number(n) => n.as_i64().unwrap_or_default(),
        Value::String(s) => s.parse().unwrap_or_default(),
        _ => 0,
    }
}

fn text<'r>(row: &'r Value, key: &str) -> &'r str {
    row[key].as_str().unwrap_or_default()
}

/// FNV-1a of the username: the same conversation has the same hash in every run, and says nothing of who it is.
fn hash(username: &str) -> String {
    let value = username.bytes().fold(0x811c_9dc5_u32, |h, b| {
        (h ^ u32::from(b)).wrapping_mul(0x0100_0193)
    });
    format!("{value:08x}")
}

fn kind(username: &str) -> &'static str {
    if username.ends_with("@chatroom") {
        "group"
    } else if username.starts_with("gh_") {
        "official"
    } else if username.starts_with("wxid_") || !username.contains('@') && username.len() > 5 {
        "private"
    } else {
        "other"
    }
}

/// How many values have each bit set, the bits no value has left out.
fn bits(values: impl IntoIterator<Item = i64>) -> BTreeMap<u32, usize> {
    let mut counts = BTreeMap::new();
    for value in values {
        for bit in 0..63 {
            if value & (1 << bit) != 0 {
                *counts.entry(bit).or_default() += 1;
            }
        }
    }
    counts
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [config] = args.as_slice() else {
        anyhow::bail!("usage: pinned_probe <config.json>");
    };
    let config = ConfigStore::load(Path::new(config)).context("read the WeFlow config")?;
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
    let me = profile.wxid.clone().unwrap_or_default();
    let root = if me.is_empty() {
        root
    } else {
        resolve_account_dir(&root.to_string_lossy(), &me)
    };
    let account = Account::new(
        root.join("db_storage"),
        profile
            .decrypt_key
            .as_deref()
            .context("database key missing")?,
    )?;

    let sessions = account.sessions()?;
    let sessions = sessions.as_array().context("sessions are rows")?;
    let columns: Vec<&String> = sessions
        .first()
        .and_then(Value::as_object)
        .map(|row| row.keys().collect())
        .unwrap_or_default();
    println!("SessionTable columns: {columns:?}");
    println!("conversations: {}", sessions.len());

    let contacts = account.query(
        &account.db_path("contact/contact.db"),
        "select username, flag from contact",
        &[],
    )?;
    let flags: HashMap<&str, i64> = contacts
        .iter()
        .map(|row| (text(row, "username"), int(row, "flag")))
        .collect();

    let statuses: Vec<i64> = sessions.iter().map(|row| int(row, "status")).collect();
    println!(
        "status values: {:?}",
        statuses
            .iter()
            .fold(BTreeMap::<i64, usize>::new(), |mut m, s| {
                *m.entry(*s).or_default() += 1;
                m
            })
    );
    println!(
        "status bits (bit: conversations): {:?}",
        bits(statuses.iter().copied())
    );
    let session_flags: Vec<i64> = sessions
        .iter()
        .map(|row| {
            flags
                .get(text(row, "username"))
                .copied()
                .unwrap_or_default()
        })
        .collect();
    println!(
        "contact.flag bits of the conversations (bit: conversations): {:?}",
        bits(session_flags.iter().copied())
    );
    println!(
        "contact.flag bits of every contact (bit: contacts): {:?}",
        bits(flags.values().copied())
    );

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_secs() as i64;
    println!("place kind     hash      status     flag          hidden sort>last sort>now");
    for (place, row) in sessions.iter().enumerate() {
        let status = int(row, "status");
        if place >= SHOWN && status == 0 {
            continue;
        }
        let username = text(row, "username");
        let flag = flags.get(username).copied().unwrap_or_default();
        let sort = int(row, "sort_timestamp");
        println!(
            "{place:>5} {:<8} {} {status:>#10x} {flag:>#12x} {:>6} {:>9} {:>8}",
            kind(username),
            hash(username),
            int(row, "is_hidden"),
            sort > int(row, "last_timestamp"),
            sort > now,
        );
    }
    Ok(())
}
