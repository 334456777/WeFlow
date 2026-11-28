//! 消息的数量：每个会话的、每天的，以及有消息的日期。

use std::collections::HashMap;

use anyhow::Result;
use serde_json::{json, Map, Value};

use super::int;
use super::MsgTable;
use crate::db::Account;

/// 一刻钟的秒数：换算消息的本地日期时的步长。
const QUARTER: i64 = 900;

impl Account {
    /// 会话的各个消息表的行数之和，最多为 `i32::MAX`。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn message_count(&self, session_id: &str) -> Result<i32> {
        self.count_rows(&self.message_tables(session_id)?)
    }

    /// `{ username: count }`：`session_ids` 中每个会话的 [`Self::message_count`]。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn session_message_counts(&self, session_ids: &[String]) -> Result<Value> {
        let mut out = Map::new();
        for (id, tables) in session_ids.iter().zip(self.message_tables_of(session_ids)?) {
            out.insert(id.clone(), json!(self.count_rows(&tables)?));
        }
        Ok(Value::Object(out))
    }

    /// `tables` 的行数之和，最多为 `i32::MAX`。
    fn count_rows(&self, tables: &[MsgTable]) -> Result<i32> {
        let mut total = 0i64;
        for t in tables {
            let sql = format!("select count(*) as n from \"{}\"", t.table);
            total += int(&self.query(&t.db, &sql, &[])?[0], "n");
        }
        Ok(i32::try_from(total).unwrap_or(i32::MAX))
    }

    /// 按本地时间每天的消息数 `{ "YYYY-MM-DD": count }`，按日期升序。
    ///
    /// 只读取每条消息的时间：经 `leaf_scan` 读取，它只解密每页中存放时间的字节；它读不了的表改用 SQL 统计。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败），包括扫描到的页的 HMAC 校验失败。
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

    /// 用 `leaf_scan` 读出的时间，按本地日期统计一张表中时间为正的消息：先按一刻钟计数，每一刻钟的本地日期再由
    /// SQLite 的 `localtime` 给出（现今每个时区与 UTC 都相差整数个一刻钟，时区的切换也在一刻钟的整点）。
    /// 扫描读不了这张表时为 `None`。
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

    /// 用 SQL 做 [`date_counts_by_scan`](Self::date_counts_by_scan) 的统计，用于扫描读不了的表。
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

    /// 至少有一条消息的日期 `["YYYY-MM-DD", ...]`，按日期升序。
    ///
    /// # Errors
    ///
    /// 同 [`session_message_date_counts`](Self::session_message_date_counts)。
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

    /// 只有一个数据库、一个会话的账号：`fill`（建表之后运行的 SQL）把行写进表 `t`；
    /// 加密后的文件写入磁盘之前先交给 `damage`。
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

    /// 跨一年多的 6000 行：B 树有好几层，每 40 行中有一行的文字长到要用溢出页，有的行时间为 null、0 或负数。
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
        // 发送者有账号所有者、其他人、未知的和没有的；类型有 null、0 和其他值
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
        // 账号所有者的文字：重复的短语、前后有空格的、长的、在溢出页上的、像压缩数据的 blob、null；
        // 其他人的文字和其他类型的消息；缺少类型的行
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
        // 文件中最后一个表叶页属于这个会话：它的行是最后写入的
        let (_dir, acct) = scanned_account("damaged", MANY_ROWS, |plain, file| {
            let leaf = (0..plain.len() / crate::sqlcipher::PAGE_SIZE)
                .rev()
                .find(|&p| p > 0 && plain[p * crate::sqlcipher::PAGE_SIZE] == 13)
                .expect("a leaf page");
            // 改动页内远在扫描所解密的单元格开头字节之后的一个字节
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
        // 十天里每 6 分 13 秒一条消息，每个午夜前后的一刻钟里都有消息
        let texts: Vec<String> = (0..2400).map(|k| format!("m{k}")).collect();
        let msgs: Vec<(i64, i64, i64, i64, &str)> = texts
            .iter()
            .enumerate()
            .map(|(k, t)| {
                let k = i64::try_from(k).unwrap();
                (k + 1, k * 373 - 7, 1, 1, t.as_str())
            })
            .collect();
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            dir.join("message/message_0.db"),
            encrypt_db(&shard(&msgs, None), &cipher),
        )
        .unwrap();
        let acct = Account::new(dir.path(), &hex(&KEY)).unwrap();
        let sqlite = Connection::open_in_memory().unwrap();
        let mut want = Map::new();
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
