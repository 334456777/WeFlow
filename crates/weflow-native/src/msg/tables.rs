//! 消息库和会话在其中的表、读取这些表的 SQL，以及读出的行。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::OptionalExtension;

use super::is_message_db;
use super::page::OffsetPosition;
use super::{table_name_for, MsgTable, PageFrom};
use crate::db::Account;
use crate::msg_row::{Columns, MsgRow, RowContext};

impl Account {
    /// `message/` 下的消息库，按编号升序：先是 `message_<n>.db`，然后是 `biz_message_<n>.db`。
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

    /// 含有 `session_id` 的表的全部分片，最早的分片在前。
    ///
    /// # Errors
    ///
    /// 某个分片的表结构读不出来时返回错误（微信改动了数据库时，以 `with_db` 重读之后为准）：
    /// 只有能读出、但没有这张表的分片才被略过。
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
                .optional()?
                .is_some())
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

    /// 从一张消息表读取 [`MsgRow`] 各列的 `select` 语句，发送者从 `Name2Id` 查出。
    pub(crate) fn select_sql(t: &MsgTable, tail: &str) -> String {
        Self::select_sql_of(t, tail, Columns::All)
    }

    /// 读取 `which` 指定的列的 [`select_sql`](Self::select_sql)。
    pub(super) fn select_sql_of(t: &MsgTable, tail: &str, which: Columns) -> String {
        format!(
            "select {} from \"{}\" m left join Name2Id n on n.rowid = m.real_sender_id {tail}",
            MsgRow::columns(which),
            t.table
        )
    }

    /// 把 `indexed`（有 `sort_seq` 的行）和 `unsequenced`（没有 `sort_seq` 的行）两个
    /// [`select_sql`](Self::select_sql) 的结果按先后合成一条语句：每张表多一条语句就要多准备一次。
    pub(super) fn select_both_sql(t: &MsgTable, indexed: &str, unsequenced: &str) -> String {
        format!(
            "select * from ({}) union all select * from ({})",
            Self::select_sql(t, indexed),
            Self::select_sql(t, unsequenced)
        )
    }

    /// 在 `t` 上运行 `sql` 读出的行，`sql` 的 select 列表须是 [`MsgRow::columns`]。
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

    /// 逐行交给 `each` 的 [`read_rows`](Self::read_rows)。查询在新的快照上重读时（见 `with_db`）从头开始：
    /// 每次尝试之前都先运行 `start`。
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

    /// `session_id` 在 `since` 及之后发送、`condition`（消息表为 `m` 的 SQL 条件）保留的消息，每张表内最早的在前：
    /// 每张表最多 `limit` 条，各表依次排列。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败），包括 `condition` 不是有效的 SQL。
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

    /// 合并全部分片后最新在前的一页消息：跳过 `offset` 条，最多取 `limit` 条（`limit` 不大于 0 时取全部）。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Vec<MsgRow>> {
        let (offset, limit) = (
            usize::try_from(offset).unwrap_or(0),
            usize::try_from(limit).unwrap_or(0),
        );
        let tables = self.message_tables(session_id)?;
        if offset > 0 {
            // 长会话中靠后的一页：逐行读完它前面的行，代价与偏移成正比（#486），
            // 所以只凭排序键找出它跳过的最后一条消息，再从那里往后翻页。
            match self.offset_position(&tables, offset, limit)? {
                OffsetPosition::At(pos) => {
                    let page = self.page_from(
                        &tables,
                        i32::try_from(limit).unwrap_or(i32::MAX),
                        PageFrom::Before(pos),
                    )?;
                    // 页中有时间不是整数的行时，SQLite 与排序键对它排序不同，改用 `newest_rows`
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

    /// `session_ids` 中每个会话的 [`message_tables`](Self::message_tables)，顺序与 `session_ids` 相同。
    /// 多个会话在一次列出的全部分片的表中查找（[`message_tables_by_name`](Self::message_tables_by_name)），
    /// 每个分片的表结构只读一次，不是每个会话读一次。
    pub(crate) fn message_tables_of(&self, session_ids: &[String]) -> Result<Vec<Vec<MsgTable>>> {
        match session_ids {
            [] => Ok(Vec::new()),
            [one] => Ok(vec![self.message_tables(one)?]),
            many => {
                let by_name = self.message_tables_by_name()?;
                Ok(many
                    .iter()
                    .map(|sid| {
                        by_name
                            .get(&table_name_for(sid))
                            .cloned()
                            .unwrap_or_default()
                    })
                    .collect())
            }
        }
    }

    /// 账号的全部消息表，按表名（会话的 [`table_name_for`]）分组，最早的分片在前：每个数据库只读一次，
    /// 用于跨会话的列表（#178）。
    ///
    /// # Errors
    ///
    /// 某个分片的表结构读不出来。
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

    /// 表结构读不出来的分片是错误，不当作没有这个会话的分片：否则导出会悄无声息地漏掉整个数据库。
    #[test]
    fn a_shard_whose_schema_cannot_be_read_fails_the_lookup() {
        let dir = TestDir::new("weflow-msg-unreadable-schema");
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let plain = plain_db_with(|c| {
            // 表足够多，表结构要占第一页之外的页
            for i in 0..300 {
                let table = table_name_for(&format!("wxid_{i}"));
                c.execute_batch(&format!(
                    "create table \"{table}\"(local_id integer primary key, message_content text)"
                ))
                .unwrap();
            }
        });
        let mut enc = encrypt_db(&plain, &PageCipher::derive(&KEY, &SALT));
        assert!(enc.len() > 2 * crate::sqlcipher::PAGE_SIZE);
        // 除第一页外每一页的 HMAC 都校验失败，如同写到一半的页
        for page in enc.chunks_mut(crate::sqlcipher::PAGE_SIZE).skip(1) {
            page[100] ^= 0xff;
        }
        std::fs::write(dir.join("message/message_0.db"), &enc).unwrap();
        let acct = Account::new(dir.path(), &crate::db::hex(&KEY)).unwrap();
        assert!(acct.message_tables("wxid_1").is_err());
        assert!(acct.message_tables("wxid_nobody").is_err());
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
        // 每张表最多 `limit` 条，取最早的
        let one: Vec<MsgRow> = acct.messages_where("wxid_bob", 0, "1", 1).unwrap();
        let tables = acct.message_tables("wxid_bob").unwrap().len();
        assert!(!one.is_empty() && one.len() <= tables);
    }

    /// 整数列中存了其他类型的单元格读作缺失，并且被计数。
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

    #[test]
    fn the_tables_of_several_conversations_are_those_of_each() {
        let (_dir, acct) = account("tables-of");
        let ids: Vec<String> = ["wxid_bob", "wxid_nobody", "wxid_bob"]
            .map(String::from)
            .to_vec();
        let places = |ts: &[MsgTable]| -> Vec<(PathBuf, String)> {
            ts.iter().map(|t| (t.db.clone(), t.table.clone())).collect()
        };
        let got = acct.message_tables_of(&ids).unwrap();
        assert_eq!(got.len(), ids.len());
        for (sid, ts) in ids.iter().zip(&got) {
            assert_eq!(
                places(ts),
                places(&acct.message_tables(sid).unwrap()),
                "{sid}"
            );
        }
        assert_eq!(got[0].len(), 2, "both shards, oldest first");
        assert_eq!(
            places(&acct.message_tables_of(&ids[..1]).unwrap()[0]),
            places(&got[0])
        );
        assert!(acct.message_tables_of(&[]).unwrap().is_empty());
    }
}
