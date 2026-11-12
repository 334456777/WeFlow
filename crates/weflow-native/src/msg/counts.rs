//! How many messages there are: per conversation, per day, and the days with messages.

use std::collections::HashMap;

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::int;
use super::MsgTable;
use crate::db::Account;

/// A quarter of an hour in seconds: the step of the local dates of messages.
const QUARTER: i64 = 900;

impl Account {
    pub fn message_count(&self, session_id: &str) -> Result<i32> {
        let mut total = 0i64;
        for t in self.message_tables(session_id)? {
            let sql = format!("select count(*) as n from \"{}\"", t.table);
            total += int(&self.query(&t.db, &sql, &[])?[0], "n");
        }
        Ok(i32::try_from(total).unwrap_or(i32::MAX))
    }

    pub fn session_message_counts(&self, session_ids: &[String]) -> Result<Value> {
        let mut out = Map::new();
        for id in session_ids {
            out.insert(id.clone(), json!(self.message_count(id)?));
        }
        Ok(Value::Object(out))
    }

    /// `{ "YYYY-MM-DD": count }` in local time, ascending.
    ///
    /// Only the time of each message is read, through `leaf_scan`, which decrypts only the bytes
    /// of each page that hold it; a table it does not read is counted through SQL.
    pub fn session_message_date_counts(&self, session_id: &str) -> Result<Value> {
        let mut counts: std::collections::BTreeMap<String, i64> = Default::default();
        for t in self.message_tables(session_id)? {
            let days = match self.date_counts_by_scan(&t)? {
                Some(days) => days,
                None => self.date_counts_by_sql(&t)?,
            };
            for (d, n) in days {
                *counts.entry(d).or_default() += n;
            }
        }
        Ok(Value::Object(
            counts.into_iter().map(|(d, n)| (d, json!(n))).collect(),
        ))
    }

    /// One table's messages with a positive time counted by local day, the times read by the leaf scan: counted by
    /// quarter of an hour, and each quarter's local date taken from SQLite's `localtime` (every time zone today is a
    /// whole number of quarters away from UTC and moves on a quarter). `None` when the scan does not read the table.
    fn date_counts_by_scan(&self, t: &MsgTable) -> Result<Option<Vec<(String, i64)>>> {
        let parts = self.read_db(
            &t.db,
            |conn| crate::leaf_scan::target(conn, &t.table, &["create_time"]),
            |file, target| {
                let Some((root, columns)) = target else {
                    return Ok(None);
                };
                crate::leaf_scan::int_columns(
                    file,
                    root,
                    &columns,
                    HashMap::<i64, i64>::new,
                    |quarters, cells| {
                        if let Some(time) = cells[0].filter(|&time| time > 0) {
                            *quarters.entry(time / QUARTER).or_default() += 1;
                        }
                    },
                )
            },
        )?;
        let Some(parts) = parts else {
            return Ok(None);
        };
        let mut quarters: HashMap<i64, i64> = HashMap::new();
        for part in parts {
            for (q, n) in part {
                *quarters.entry(q).or_default() += n;
            }
        }
        let list = serde_json::to_string(&quarters.keys().collect::<Vec<_>>())?;
        let sql = format!(
            "select value, date(value * {QUARTER}, 'unixepoch', 'localtime') from json_each(?1)"
        );
        self.with_db(&t.db, |conn| {
            let mut stmt = conn.prepare(&sql)?;
            let mut rows = stmt.query([&list])?;
            let mut days: HashMap<String, i64> = HashMap::new();
            while let Some(r) = rows.next()? {
                if let (rusqlite::types::ValueRef::Integer(q), Some(d)) =
                    (r.get_ref(0)?, r.get_ref(1)?.as_str_or_null()?)
                {
                    *days.entry(d.to_string()).or_default() +=
                        quarters.get(&q).copied().unwrap_or(0);
                }
            }
            Ok(Some(days.into_iter().collect()))
        })
    }

    /// [`date_counts_by_scan`](Self::date_counts_by_scan) through SQL, for a table the scan does not read.
    fn date_counts_by_sql(&self, t: &MsgTable) -> Result<Vec<(String, i64)>> {
        let sql = format!(
            "select date(q * {QUARTER}, 'unixepoch', 'localtime') as d, sum(n) as n from \
             (select create_time / {QUARTER} as q, count(*) as n from \"{}\" where create_time > 0 group by q) \
             group by d",
            t.table
        );
        Ok(self
            .query(&t.db, &sql, &[])?
            .into_iter()
            .filter_map(|r| Some((r["d"].as_str()?.to_string(), int(&r, "n"))))
            .collect())
    }

    /// `["YYYY-MM-DD", ...]` days that have at least one message, ascending.
    pub fn message_dates(&self, session_id: &str) -> Result<Value> {
        let counts = self.session_message_date_counts(session_id)?;
        Ok(Value::Array(
            counts
                .as_object()
                .map(|o| o.keys().map(|k| json!(k)).collect())
                .unwrap_or_default(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::hex;
    use crate::fixture::TestDir;
    use crate::msg::table_name_for;
    use crate::msg::tests::{account, shard, T0};
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;
    use rusqlite::Connection;

    /// One conversation in one database whose rows `fill` (SQL run after the schema) writes into table `t`; the
    /// encrypted file is handed to `damage` before it is written.
    fn scanned_account(
        tag: &str,
        fill: &str,
        damage: impl FnOnce(&[u8], &mut Vec<u8>),
    ) -> (TestDir, Account) {
        let dir = TestDir::new(&format!("weflow-scan-{tag}"));
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let table = table_name_for("wxid_bob");
        let fill = fill.replace("\"t\"", &format!("\"{table}\""));
        let plain = plain_db_with(move |c: &Connection| {
            c.execute_batch(&format!(
                "create table Name2Id(user_name text primary key, is_session integer);
                 create table \"{table}\"(local_id integer primary key, server_id integer, local_type integer,
                     sort_seq integer, real_sender_id integer, create_time, status integer, message_content text);
                 {fill}"
            ))
            .unwrap();
        });
        let mut file = encrypt_db(&plain, &PageCipher::derive(&KEY, &SALT));
        damage(&plain, &mut file);
        std::fs::write(dir.join("message/message_0.db"), file).unwrap();
        let acct = Account::new(dir.path(), &hex(&KEY)).unwrap();
        (dir, acct)
    }

    /// 6,000 rows over more than a year: a b-tree of several levels, every 40th row's text long enough to overflow,
    /// and times that are null, zero or negative.
    const MANY_ROWS: &str =
        "with recursive n(i) as (select 1 union all select i + 1 from n where i < 6000)
        insert into \"t\"(local_id, local_type, sort_seq, create_time, message_content)
        select i, 1, i * 1000,
               case when i % 97 = 0 then null when i % 89 = 0 then 0 when i % 83 = 0 then -5
                    else 1700000000 + (i * 7919) % 34000000 end,
               case when i % 40 = 0 then printf('%.6000c', 'x') else printf('m%d', i) end
        from n;";

    fn sorted(mut days: Vec<(String, i64)>) -> Vec<(String, i64)> {
        days.sort();
        days
    }

    #[test]
    fn date_counts_from_the_leaf_scan_are_those_of_sql() {
        let (_dir, acct) = scanned_account("many", MANY_ROWS, |_, _| {});
        let t = &acct.message_tables("wxid_bob").unwrap()[0];
        let scanned = acct
            .date_counts_by_scan(t)
            .unwrap()
            .expect("the scan reads the table");
        assert!(scanned.len() > 300, "{} days", scanned.len());
        assert_eq!(sorted(scanned), sorted(acct.date_counts_by_sql(t).unwrap()));
    }

    #[test]
    fn statistics_from_the_leaf_scan_are_those_of_sql() {
        // senders of the owner, of others, unknown and none; types null, zero and others
        let fill = format!(
            "{MANY_ROWS}
             insert into Name2Id(rowid, user_name) values (1, 'wxid_me'), (2, 'wxid_bob'), (3, 'wxid_carol');
             update \"t\" set real_sender_id = case when local_id % 11 = 0 then null else local_id % 5 end,
                 local_type = case when local_id % 13 = 0 then null when local_id % 17 = 0 then 0
                                   else 1 + local_id % 4 end;"
        );
        let (_dir, acct) = scanned_account("stats", &fill, |_, _| {});
        let acct = acct.with_my_wxid(Some("wxid_me".into()));
        let ids = vec!["wxid_bob".to_string()];
        let read = |acct: &Account| {
            let (b, e) = (1_705_000_000, 1_720_000_000);
            (
                acct.aggregate_stats(&ids, 0, 0).unwrap(),
                acct.annual_report_stats(&ids, b, e).unwrap(),
                acct.session_totals(&ids, b, e).unwrap(),
                acct.available_years(&ids).unwrap(),
                acct.group_stats("wxid_bob", 0, e).unwrap(),
            )
        };
        let scanned = read(&acct);
        assert!(scanned.0["total"].as_i64().unwrap() > 5000);
        assert!(scanned.0["sent"].as_i64().unwrap() > 0);
        crate::leaf_scan::SQL_ONLY.with(|off| off.set(true));
        let by_sql = read(&acct);
        crate::leaf_scan::SQL_ONLY.with(|off| off.set(false));
        assert_eq!(scanned, by_sql);
    }

    #[test]
    fn annual_extras_from_the_leaf_scan_are_those_of_sql() {
        // the owner's texts: repeated phrases, ones padded with spaces, long ones, ones on overflow pages, a
        // compressed-looking blob, a null; texts of others and of other types; a missing type
        let fill = format!(
            "{MANY_ROWS}
             insert into Name2Id(rowid, user_name) values (1, 'wxid_me'), (2, 'wxid_bob');
             update \"t\" set real_sender_id = case when local_id % 3 = 0 then 2 else 1 end,
                 local_type = case when local_id % 13 = 0 then null when local_id % 7 = 0 then 3 else 1 end,
                 sort_seq = case when local_id % 19 = 0 then 0 else sort_seq end;
             update \"t\" set message_content = case
                 when local_id % 40 = 0 then message_content
                 when local_id % 41 = 0 then printf('%.300c', 'y')
                 when local_id % 43 = 0 then '   p' || (local_id % 5) || '   '
                 when local_id % 47 = 0 then x'6869'
                 when local_id % 53 = 0 then null
                 else 'p' || (local_id % 37) end;"
        );
        let (_dir, acct) = scanned_account("extras", &fill, |_, _| {});
        let acct = acct.with_my_wxid(Some("wxid_me".into()));
        let ids = vec!["wxid_bob".to_string()];
        let read = |acct: &Account| {
            (
                acct.annual_report_extras(&ids, 0, 0, 1_701_000_000, 1_701_086_399)
                    .unwrap(),
                acct.annual_report_extras(&ids, 1_705_000_000, 1_720_000_000, 0, 0)
                    .unwrap(),
            )
        };
        let scanned = read(&acct);
        assert!(!scanned.0["topPhrases"].as_array().unwrap().is_empty());
        crate::leaf_scan::SQL_ONLY.with(|off| off.set(true));
        let by_sql = read(&acct);
        crate::leaf_scan::SQL_ONLY.with(|off| off.set(false));
        assert_eq!(scanned, by_sql);
    }

    #[test]
    fn a_time_that_is_not_an_integer_is_counted_through_sql() {
        for (tag, time) in [
            ("text", "'1700000000'"),
            ("real", "1700000000.5"),
            ("blob", "x'31'"),
        ] {
            let fill =
                format!("{MANY_ROWS} update \"t\" set create_time = {time} where local_id = 3000;");
            let (_dir, acct) = scanned_account(tag, &fill, |_, _| {});
            let t = &acct.message_tables("wxid_bob").unwrap()[0];
            assert!(acct.date_counts_by_scan(t).unwrap().is_none(), "{tag}");
            let counts = acct.session_message_date_counts("wxid_bob").unwrap();
            let by_sql: Map<String, Value> = sorted(acct.date_counts_by_sql(t).unwrap())
                .into_iter()
                .map(|(d, n)| (d, json!(n)))
                .collect();
            assert_eq!(counts, Value::Object(by_sql), "{tag}");
        }
    }

    #[test]
    fn a_leaf_page_damaged_on_disk_fails_the_date_counts() {
        // the last table leaf page of the file is one of the conversation's: its rows were written last
        let (_dir, acct) = scanned_account("damaged", MANY_ROWS, |plain, file| {
            let leaf = (0..plain.len() / crate::sqlcipher::PAGE_SIZE)
                .rev()
                .find(|&p| p > 0 && plain[p * crate::sqlcipher::PAGE_SIZE] == 13)
                .expect("a leaf page");
            // a byte of the page well past the cells' first bytes the scan decrypts
            file[leaf * crate::sqlcipher::PAGE_SIZE + 3000] ^= 0x40;
        });
        let err = acct.session_message_date_counts("wxid_bob").unwrap_err();
        assert!(format!("{err:#}").contains("HMAC"), "{err:#}");
    }

    #[test]
    fn dates_and_counts() {
        let (_dir, acct) = account("dates");
        let counts = acct.session_message_date_counts("wxid_bob").unwrap();
        assert_eq!(counts.as_object().unwrap().len(), 5);
        assert!(counts.as_object().unwrap().values().all(|n| *n == 1));
        let dates = acct.message_dates("wxid_bob").unwrap();
        let dates: Vec<&str> = dates
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap())
            .collect();
        assert_eq!(dates.len(), 5);
        assert!(
            dates.windows(2).all(|w| w[0] < w[1]),
            "ascending: {dates:?}"
        );
        assert_eq!(
            acct.session_message_counts(&["wxid_bob".into(), "x".into()])
                .unwrap(),
            json!({"wxid_bob": 5, "x": 0})
        );
    }

    #[test]
    fn days_are_counted_as_the_local_date_of_each_message() {
        let dir = TestDir::new("weflow-msg-days");
        std::fs::create_dir_all(dir.join("message")).unwrap();
        // a message every 6 min 13 s for ten days, so midnights fall between the quarters' own messages
        let texts: Vec<String> = (0..2400).map(|k| format!("m{k}")).collect();
        let msgs: Vec<(i64, i64, i64, i64, &str)> = texts
            .iter()
            .enumerate()
            .map(|(k, t)| (k as i64 + 1, k as i64 * 373 - 7, 1, 1, t.as_str()))
            .collect();
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            dir.join("message/message_0.db"),
            encrypt_db(&shard(&msgs, None), &cipher),
        )
        .unwrap();
        let acct = Account::new(dir.path(), &hex(&KEY)).unwrap();
        let sqlite = Connection::open_in_memory().unwrap();
        let mut want = serde_json::Map::new();
        for (_, offset, ..) in &msgs {
            let day: String = sqlite
                .query_row(
                    "select date(?1, 'unixepoch', 'localtime')",
                    [T0 + offset],
                    |r| r.get(0),
                )
                .unwrap();
            let n = want.entry(day).or_insert(json!(0));
            *n = json!(n.as_i64().unwrap() + 1);
        }
        assert!(want.len() >= 10);
        assert_eq!(
            acct.session_message_date_counts("wxid_bob").unwrap(),
            Value::Object(want)
        );
    }
}
