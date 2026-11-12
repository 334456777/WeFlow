//! The message databases and a conversation's tables in them, the SQL that reads them, and their rows.

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};

use super::is_message_db;
use super::page::OffsetPosition;
use super::{table_name_for, MsgTable, PageFrom};
use crate::db::Account;
use crate::msg_row::{Columns, MsgRow, RowContext};

impl Account {
    /// The message databases in `message/`, ascending by number: `message_<n>.db`, then `biz_message_<n>.db`.
    pub fn message_dbs(&self) -> Vec<PathBuf> {
        let dir = self.db_path("message");
        let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let n = is_message_db(e.file_name().to_str()?)?;
                Some((n, e.path()))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, p)| p).collect()
    }

    /// Every shard that holds a table for `session_id`, oldest shard first.
    pub fn message_tables(&self, session_id: &str) -> Result<Vec<MsgTable>> {
        let table = table_name_for(session_id);
        let mut out = Vec::new();
        let dbs = self.message_dbs();
        self.prefetch(&dbs);
        for db in dbs {
            let hit = self.with_db(&db, |c| {
                Ok(c.query_row(
                    "select 1 from sqlite_master where type='table' and name=?1",
                    [&table],
                    |_| Ok(()),
                )
                .is_ok())
            })?;
            if hit {
                out.push(MsgTable {
                    db,
                    table: table.clone(),
                });
            }
        }
        Ok(out)
    }

    /// `select` of the [`MsgRow`] columns over one message table with the sender resolved from `Name2Id`.
    pub(crate) fn select_sql(t: &MsgTable, tail: &str) -> String {
        Self::select_sql_of(t, tail, Columns::All)
    }

    /// [`select_sql`](Self::select_sql) reading `which` columns.
    pub(super) fn select_sql_of(t: &MsgTable, tail: &str, which: Columns) -> String {
        format!(
            "select {} from \"{}\" m left join Name2Id n on n.rowid = m.real_sender_id {tail}",
            MsgRow::columns(which),
            t.table
        )
    }

    /// The rows of [`select_sql`](Self::select_sql) with `indexed` (rows with a `sort_seq`), then those with
    /// `unsequenced` (rows without one), as one statement: a second one per table costs its own preparation.
    pub(super) fn select_both_sql(t: &MsgTable, indexed: &str, unsequenced: &str) -> String {
        format!(
            "select * from ({}) union all select * from ({})",
            Self::select_sql(t, indexed),
            Self::select_sql(t, unsequenced)
        )
    }

    /// The rows of `sql` over `t`, whose select list is [`MsgRow::columns`].
    pub(crate) fn read_rows(
        &self,
        t: &MsgTable,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<MsgRow>> {
        let out = std::cell::RefCell::new(Vec::new());
        self.read_rows_each(
            t,
            sql,
            params,
            || out.borrow_mut().clear(),
            |row| out.borrow_mut().push(row),
        )?;
        Ok(out.into_inner())
    }

    /// [`read_rows`](Self::read_rows), each row handed to `each` as it is read. A query read again on a fresh
    /// snapshot (see `with_db`) starts over: `start` runs before every attempt.
    pub(crate) fn read_rows_each(
        &self,
        t: &MsgTable,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
        mut start: impl FnMut(),
        mut each: impl FnMut(MsgRow),
    ) -> Result<()> {
        let is_me = |sender: &str| self.is_me(sender);
        let ctx = RowContext {
            table: t,
            is_me: &is_me,
        };
        self.with_db(&t.db, |conn| {
            start();
            let mut stmt = conn
                .prepare(sql)
                .with_context(|| format!("bad query: {sql}"))?;
            let mut rows = stmt.query(params)?;
            while let Some(row) = rows.next()? {
                each(MsgRow::read(row, &ctx)?);
            }
            Ok(())
        })
    }

    /// The messages of `session_id` sent at `since` or later that `condition` keeps (SQL over the message table
    /// `m`), oldest first in each table: at most `limit` of each table, the tables one after the other.
    pub fn messages_where(
        &self,
        session_id: &str,
        since: i64,
        condition: &str,
        limit: usize,
    ) -> Result<Vec<MsgRow>> {
        let limit = i64::try_from(limit).unwrap_or(i64::MAX);
        let tail = format!(
            "where m.create_time >= ?1 and ({condition}) \
             order by m.create_time asc, m.sort_seq asc, m.local_id asc limit ?2"
        );
        let mut out = Vec::new();
        for t in self.message_tables(session_id)? {
            let sql = Self::select_sql(&t, &tail);
            out.extend(self.read_rows(&t, &sql, &[&since.max(0), &limit])?);
        }
        Ok(out)
    }

    /// Newest-first page of messages across all shards.
    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Vec<MsgRow>> {
        let (offset, limit) = (offset.max(0) as usize, limit.max(0) as usize);
        let tables = self.message_tables(session_id)?;
        if offset > 0 {
            // A page deep into a long conversation: reading every row before it costs what the offset does (issue
            // #486), so find the last message it skips from the sort keys alone and page on from there.
            match self.offset_position(&tables, offset, limit)? {
                OffsetPosition::At(pos) => {
                    let page = self.page_from(
                        &tables,
                        i32::try_from(limit).unwrap_or(i32::MAX),
                        PageFrom::Before(pos),
                    )?;
                    // a page row whose time is not an integer: SQLite and the sort key order it differently
                    if page.iter().all(|r| r.create_time.is_some()) {
                        return Ok(page);
                    }
                }
                OffsetPosition::Past => return Ok(Vec::new()),
                OffsetPosition::Unusual => {}
            }
        }
        self.newest_rows(&tables, offset, limit)
    }

    /// Every message table of the account by its name ([`table_name_for`] a conversation), oldest shard first: one
    /// read of each database, for a listing across conversations (issue #178).
    pub fn message_tables_by_name(&self) -> Result<HashMap<String, Vec<MsgTable>>> {
        let dbs = self.message_dbs();
        self.prefetch(&dbs);
        let mut out: HashMap<String, Vec<MsgTable>> = HashMap::new();
        for db in dbs {
            let names = self.with_db(&db, |c| {
                let mut stmt = c.prepare(
                    "select name from sqlite_master where type = 'table' and name like 'Msg\\_%' escape '\\'",
                )?;
                let names = stmt
                    .query_map([], |r| r.get::<_, String>(0))?
                    .collect::<rusqlite::Result<Vec<_>>>()?;
                Ok(names)
            })?;
            for table in names {
                out.entry(table.clone()).or_default().push(MsgTable {
                    db: db.clone(),
                    table,
                });
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::TestDir;
    use crate::msg::shard_name;
    use crate::msg::tests::{account, texts};
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;

    #[test]
    fn biz_message_databases_are_read_with_numbers_of_their_own() {
        let root = TestDir::new("weflow-msg-biz");
        let f = crate::fixture::Fixture::new(&root, "wxid_me_ab12");
        let spec = |text| vec![crate::fixture::MsgSpec::text(1, "gh_a", 100, text)];
        f.message_shard(0, &[("gh_a", spec("plain"))]);
        f.biz_message_db(0, &[("gh_a", spec("biz"))]);
        let acct = Account::new(f.db_storage(), &f.key_hex()).unwrap();
        assert_eq!(acct.message_dbs().len(), 2);
        assert_eq!(acct.message_tables("gh_a").unwrap().len(), 2);
        let rows = acct.messages("gh_a", 0, 0).unwrap();
        let names: Vec<String> = rows.iter().map(|r| shard_name(r.shard)).collect();
        assert_eq!(rows.len(), 2);
        assert!(names.contains(&"message_0".to_string()), "{names:?}");
        assert!(names.contains(&"biz_message_0".to_string()), "{names:?}");
    }

    #[test]
    fn shards_are_discovered_and_merged_newest_first() {
        let (_dir, acct) = account("merge");
        assert_eq!(
            acct.message_dbs().len(),
            2,
            "message_fts.db must not count as a shard"
        );
        assert_eq!(acct.message_tables("wxid_bob").unwrap().len(), 2);
        assert!(acct.message_tables("wxid_nobody").unwrap().is_empty());
        assert_eq!(acct.message_count("wxid_bob").unwrap(), 5);
        assert_eq!(
            texts(&acct.messages("wxid_bob", 3, 0).unwrap()),
            ["voice", "fourth", "third"]
        );
        assert_eq!(
            texts(&acct.messages("wxid_bob", 10, 3).unwrap()),
            ["img", "hello"]
        );
        assert!(acct.messages("wxid_bob", 10, 5).unwrap().is_empty());
    }

    #[test]
    fn messages_where_keeps_what_the_condition_does_from_a_time_on() {
        let (_dir, acct) = account("where");
        let all: Vec<MsgRow> = acct.messages("wxid_bob", 0, 0).unwrap();
        let since = all.iter().filter_map(|m| m.create_time).min().unwrap() + 1;
        let kept: Vec<MsgRow> = acct
            .messages_where("wxid_bob", since, "m.local_type = 1", 1000)
            .unwrap();
        let mut want: Vec<i64> = all
            .iter()
            .filter(|m| m.local_type == 1 && m.create_time.unwrap_or(0) >= since)
            .map(|m| m.local_id)
            .collect();
        let mut got: Vec<i64> = kept.iter().map(|m| m.local_id).collect();
        assert!(!got.is_empty());
        want.sort_unstable();
        got.sort_unstable();
        assert_eq!(got, want);
        // at most `limit` of each table, the oldest
        let one: Vec<MsgRow> = acct.messages_where("wxid_bob", 0, "1", 1).unwrap();
        let tables = acct.message_tables("wxid_bob").unwrap().len();
        assert!(!one.is_empty() && one.len() <= tables);
    }

    /// A cell of an integer column that holds another type is read as absent, and counted.
    #[test]
    fn odd_integer_cells_are_absent_and_counted() {
        use crate::fixture::T0;
        use crate::msg_row::odd_cell_count;
        let root = TestDir::new("weflow-odd-cells");
        let fixture = crate::fixture::Fixture::new(&root, "wxid_synthetic_ab12");
        let table = table_name_for("wxid_bob");
        let plain = plain_db_with(|conn| {
            conn.execute_batch(&format!(
                "create table Name2Id(user_name text);
                 insert into Name2Id values ('wxid_bob');
                 create table \"{table}\"(local_id integer primary key, server_id, local_type integer, sort_seq,
                     create_time integer, status integer, real_sender_id integer, compress_content, message_content,
                     source, packed_info_data);
                 insert into \"{table}\"(local_id, server_id, local_type, sort_seq, create_time, real_sender_id,
                     message_content) values (1, 'not a number', 1, 1.5, {T0}, 1, 'odd');"
            ))
            .unwrap();
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            fixture.db_storage().join("message/message_0.db"),
            encrypt_db(&plain, &cipher),
        )
        .unwrap();
        let acct = Account::new(fixture.db_storage(), &fixture.key_hex()).unwrap();
        let before = odd_cell_count();
        let rows: Vec<MsgRow> = acct.messages("wxid_bob", 0, 0).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!((rows[0].server_id, rows[0].sort_seq), (None, None));
        assert_eq!(rows[0].key(), (T0 * 1000, T0, 1));
        assert!(odd_cell_count() >= before + 2);
    }
}
