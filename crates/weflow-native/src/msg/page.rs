//! 会话的分页：最新的消息、按偏移的页，以及某条消息之前或之后的页。

use anyhow::{Context, Result};

use super::MsgPos;
use super::{shard_of, typed_row_key};
use super::{MsgTable, PageFrom, Ties};
use crate::db::Account;
use crate::msg_row::{Columns, MsgRow};

/// 行的排序键（见 [`MsgPos`]）和所在消息库的分片号。
type ShardKey = ((i64, i64, i64), u32);

/// [`Account::messages`] 在会话中靠后的一页从哪里开始。
pub(super) enum OffsetPosition {
    /// 从这条消息之后开始，它是偏移跳过的最后一条消息。
    At(MsgPos),
    /// 偏移跳过了全部消息。
    Past,
    /// 读到的排序键中有不是按整数存储的值（`sort_seq` 为 null 不算）：[`typed_row_key`] 与 SQLite
    /// 对这样的值排序不同，所以按偏移逐行读取这一页，用 SQLite 的顺序（[`newest_rows`](Account::newest_rows)）。
    Unusual,
}

impl Account {
    /// [`newest_rows`](Self::newest_rows) 对一张表的 SQL：先是有 `sort_seq` 的最新 `want`（`?1`）行，
    /// `floor` 为真时只取 `?2` 及之后的；再是没有 `sort_seq` 的最新 `want` 行。
    /// 读取连接了发送者的 [`MsgRow`] 各列，给出 `columns` 时只读这些列。
    fn newest_sql(t: &MsgTable, columns: Option<&str>, floor: bool) -> String {
        let order = "order by m.sort_seq desc, m.create_time desc, m.local_id desc limit ?1";
        let floor = if floor { "m.sort_seq >= ?2 and " } else { "" };
        let indexed = format!("where {floor}m.sort_seq <> 0 {order}");
        // 没有 sort_seq 的行按 create_time * 1000 排序（见 [`MsgPos`]），它们的 sort_seq 低于任何 floor，所以另外读取
        let unsequenced = "where (m.sort_seq is null or m.sort_seq = 0) \
                           order by m.create_time desc, m.local_id desc limit ?1";
        match columns {
            None => Self::select_both_sql(t, &indexed, unsequenced),
            Some(columns) => {
                let table = &t.table;
                format!(
                    "select * from (select {columns} from \"{table}\" m {indexed}) \
                     union all select * from (select {columns} from \"{table}\" m {unsequenced})"
                )
            }
        }
    }

    /// 跳过 `offset` 条消息的一页从哪里开始：它跳过的最后一条消息的位置。
    ///
    /// [`newest_rows`](Self::newest_rows) 会读的行只按排序键的第一部分排名：它就在 `sort_seq` 索引里，
    /// 不必解密这些行所在的页。只有第一部分与跳过的最后一条或这一页的最后一条相同的行，才读出整个键：
    /// 排序键的其余部分只在这两处可能把一行移过页的边界。
    pub(super) fn offset_position(
        &self,
        tables: &[MsgTable],
        offset: usize,
        limit: usize,
    ) -> Result<OffsetPosition> {
        let want = if limit == 0 {
            -1
        } else {
            i64::try_from(offset + limit).unwrap_or(i64::MAX)
        };
        let mut firsts: Vec<i64> = Vec::new();
        for t in tables {
            if shard_of(t).is_none() {
                return Ok(OffsetPosition::Unusual);
            }
            let Some(read) = self.first_keys(t, want)? else {
                return Ok(OffsetPosition::Unusual);
            };
            firsts.extend(read);
        }
        firsts.sort_unstable_by(|a, b| b.cmp(a));
        let Some(&first) = firsts.get(offset - 1) else {
            return Ok(OffsetPosition::Past);
        };
        if let Some(&last) = usize::try_from(want).ok().and_then(|w| firsts.get(w - 1)) {
            if last != first && self.tied_keys(tables, last)?.is_none() {
                return Ok(OffsetPosition::Unusual);
            }
        }
        let Some(mut tied) = self.tied_keys(tables, first)? else {
            return Ok(OffsetPosition::Unusual);
        };
        // 列表的顺序：最新的键在前，相同的键按分片号升序
        tied.sort_unstable_by(|(a, sa), (b, sb)| b.cmp(a).then(sa.cmp(sb)));
        let above = firsts.partition_point(|&f| f > first);
        // 读出的键相同的行比排名时少：两次读取之间表发生了变化
        let Some(&((sort_seq, create_time, local_id), shard)) = tied.get(offset - 1 - above) else {
            return Ok(OffsetPosition::Unusual);
        };
        Ok(OffsetPosition::At(MsgPos {
            sort_seq,
            create_time,
            local_id,
            shard,
        }))
    }

    /// `t` 中有 `sort_seq` 的最新 `want` 行和没有 `sort_seq` 的最新 `want` 行的排序键第一部分（见 [`MsgPos`]）：
    /// 前者从 `sort_seq` 索引读取，后者由 `create_time` 得出；`want` 为负数时取全部的行。
    /// 有一个不是整数时为 `None`。
    fn first_keys(&self, t: &MsgTable, want: i64) -> Result<Option<Vec<i64>>> {
        let table = &t.table;
        let sequenced = format!(
            "select sort_seq from \"{table}\" where sort_seq <> 0 order by sort_seq desc limit ?1"
        );
        let unsequenced = format!(
            "select create_time from \"{table}\" where sort_seq is null or sort_seq = 0 \
             order by create_time desc, local_id desc limit ?1"
        );
        self.with_db(&t.db, |conn| {
            // 每次重试都用新的向量：过期的快照不会留下不完整或重复的键。
            let mut firsts = Vec::new();
            for (sql, scale) in [(&sequenced, 1), (&unsequenced, 1000)] {
                let mut stmt = conn
                    .prepare(sql)
                    .with_context(|| format!("bad query: {sql}"))?;
                let mut rows = stmt.query([want])?;
                while let Some(row) = rows.next()? {
                    let rusqlite::types::ValueRef::Integer(v) = row.get_ref(0)? else {
                        return Ok(None);
                    };
                    firsts.push(v * scale);
                }
            }
            Ok(Some(firsts))
        })
    }

    /// `tables` 中排序键以 `first` 开头的行的排序键和分片号；有键不是按整数存储时为 `None`（见 [`typed_row_key`]）。
    fn tied_keys(&self, tables: &[MsgTable], first: i64) -> Result<Option<Vec<ShardKey>>> {
        let mut keys = Vec::new();
        for t in tables {
            let Some(shard) = shard_of(t) else {
                return Ok(None);
            };
            let table = &t.table;
            let sql = format!(
                "select sort_seq, create_time, local_id from \"{table}\" where sort_seq = ?1 and sort_seq <> 0 \
                 union all select sort_seq, create_time, local_id from \"{table}\" \
                 where (sort_seq is null or sort_seq = 0) and create_time * 1000 = ?1"
            );
            let read = self.with_db(&t.db, |conn| {
                let mut found = Vec::new();
                let mut stmt = conn
                    .prepare(&sql)
                    .with_context(|| format!("bad query: {sql}"))?;
                let mut rows = stmt.query([first])?;
                while let Some(row) = rows.next()? {
                    let Some(key) =
                        typed_row_key(row.get_ref(0)?, row.get_ref(1)?, row.get_ref(2)?)
                    else {
                        return Ok(None);
                    };
                    found.push((key, shard));
                }
                Ok(Some(found))
            })?;
            let Some(read) = read else {
                return Ok(None);
            };
            keys.extend(read);
        }
        Ok(Some(keys))
    }

    /// `tables` 中最新在前的一页消息：把这一页和它前面的行全部读出、排序后取出。
    pub(super) fn newest_rows(
        &self,
        tables: &[MsgTable],
        offset: usize,
        limit: usize,
    ) -> Result<Vec<MsgRow>> {
        let want = if limit == 0 {
            -1
        } else {
            i64::try_from(offset + limit).unwrap_or(i64::MAX)
        };
        let mut rows: Vec<MsgRow> = Vec::new();
        for t in tables {
            // 直接这样查询时 SQLite 会给整张表排序（为了连接，它遍历发送者的索引）。
            // 从 sort_seq 索引读出第 `want` 新的 sort_seq，把同样的行限定在这个索引能直接给出的范围里。
            let floor = if want > 0 {
                self.sort_seq_floor(t, want)?
            } else {
                None
            };
            let sql = Self::newest_sql(t, None, floor.is_some());
            rows.extend(match floor {
                Some(floor) => self.read_rows(t, &sql, &[&want, &floor])?,
                None => self.read_rows(t, &sql, &[&want])?,
            });
        }
        rows.sort_by_key(|r| std::cmp::Reverse(r.key()));
        let it = rows.into_iter().skip(offset);
        Ok(if limit == 0 {
            it.collect()
        } else {
            it.take(limit).collect()
        })
    }

    /// 紧挨着某条消息（见 [`MsgPos`]）的一页消息，最新的在前：取比它早的 `limit` 条，或者比它新、离它最近的
    /// `limit` 条（仍然最新的在前）；`limit == 0` 表示全部。每张表都经 `sort_seq` 索引从这个位置开始读，
    /// 所以长会话中靠后的一页与第一页的代价相同。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn messages_from(
        &self,
        session_id: &str,
        limit: i32,
        from: PageFrom,
    ) -> Result<Vec<MsgRow>> {
        self.page_from(&self.message_tables(session_id)?, limit, from)
    }

    /// 在一个会话的消息表上做 [`messages_from`](Self::messages_from)。
    pub(super) fn page_from(
        &self,
        tables: &[MsgTable],
        limit: i32,
        from: PageFrom,
    ) -> Result<Vec<MsgRow>> {
        let want = if limit <= 0 { -1 } else { i64::from(limit) };
        let (pos, older) = match from {
            PageFrom::Before(p) => (p, true),
            PageFrom::After(p) => (p, false),
        };
        let dir = if older { "desc" } else { "asc" };
        let mut rows: Vec<(MsgRow, u32)> = Vec::new();
        for t in tables {
            let Some(shard) = shard_of(t) else {
                continue;
            };
            // 键与这个位置相同的行，只在读取方向上排在这个位置所在数据库之后的数据库里，才算越过了它
            let cmp = match (older, shard > pos.shard, shard < pos.shard) {
                (true, true, _) => "<=",
                (true, false, _) => "<",
                (false, _, true) => ">=",
                (false, _, false) => ">",
            };
            let bound = if older { "<=" } else { ">=" };
            // 有 sort_seq 的行经它的索引读取；少数没有 sort_seq 的行按 create_time * 1000 排序
            let indexed = format!(
                "where m.sort_seq {bound} ?1 and m.sort_seq <> 0 and (m.sort_seq, m.create_time, m.local_id) {cmp} (?1, ?2, ?3) \
                 order by m.sort_seq {dir}, m.create_time {dir}, m.local_id {dir} limit ?4"
            );
            let unsequenced = format!(
                "where (m.sort_seq is null or m.sort_seq = 0) \
                 and (m.create_time * 1000, m.create_time, m.local_id) {cmp} (?1, ?2, ?3) \
                 order by m.create_time {dir}, m.local_id {dir} limit ?4"
            );
            let page = self.read_rows(
                t,
                &Self::select_both_sql(t, &indexed, &unsequenced),
                &[&pos.sort_seq, &pos.create_time, &pos.local_id, &want],
            )?;
            rows.extend(page.into_iter().map(|r| (r, shard)));
        }
        // 列表的顺序：最新的键在前，相同的键按分片号升序
        rows.sort_by(|(a, sa), (b, sb)| b.key().cmp(&a.key()).then(sa.cmp(sb)));
        if !older {
            // 离这个位置最近的较新的行在末尾
            // `want` 为负数时保留全部的行
            let skip = usize::try_from(want).map_or(0, |want| rows.len().saturating_sub(want));
            rows.drain(..skip);
        } else if let Ok(want) = usize::try_from(want) {
            rows.truncate(want);
        }
        debug_assert!(rows.iter().all(|(r, s)| {
            let k = r.key();
            if older {
                k < pos.key() || (k == pos.key() && *s > pos.shard)
            } else {
                k > pos.key() || (k == pos.key() && *s < pos.shard)
            }
        }));
        Ok(rows.into_iter().map(|(r, _)| r).collect())
    }

    /// `tables`（一个会话的表，来自 [`message_tables_by_name`](Self::message_tables_by_name)）中类型为
    /// `local_types` 之一的消息，按聊天窗口的顺序最新在前，最多 `limit` 条（为负数时取全部）。
    /// 只取列在 `before` 之后的更早的消息（排序位置相同的按它的 [`Ties`]；为 `None` 时不限），
    /// 发送时间在 `range` 的两个 Unix 秒之间（含两端，0 表示不限），
    /// 并且在聊天窗口的顺序中不早于 `floor`（即 `sort_seq` 不小于它，0 表示不限）。
    /// 每张表一条语句，每种类型一个部分，所以 `(local_type, sort_seq)` 索引按顺序给出每部分的行，
    /// 读到 `limit` 条就停止（#178）。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn typed_before(
        &self,
        tables: &[MsgTable],
        local_types: &[i64],
        before: Option<(MsgPos, Ties)>,
        range: (i64, i64),
        floor: i64,
        limit: i64,
    ) -> Result<Vec<MsgRow>> {
        let (pos, ties) = before.unwrap_or((
            MsgPos {
                sort_seq: i64::MAX,
                create_time: i64::MAX,
                local_id: i64::MAX,
                shard: 0,
            },
            Ties::None,
        ));
        let (begin, end) = range;
        let mut sent = String::new();
        if begin > 0 {
            sent.push_str(&format!(" and m.create_time >= {begin}"));
        }
        if end > 0 {
            sent.push_str(&format!(" and m.create_time <= {end}"));
        }
        let cmp = match ties {
            Ties::All => "<=",
            Ties::None => "<",
        };
        let mut rows: Vec<(MsgRow, u32)> = Vec::new();
        for t in tables {
            let Some(shard) = shard_of(t) else {
                continue;
            };
            if local_types.is_empty() {
                continue;
            }
            let table = &t.table;
            // 每种类型一个部分，各自按顺序读索引；少数没有 sort_seq 的行合成一个部分，包含所有类型；
            // 发送者在外层只连接一次（每个部分的查询计划都要花时间）
            let mut parts: Vec<String> = local_types
                .iter()
                .map(|lt| {
                    format!(
                        "select * from (select * from \"{table}\" m where m.local_type = {lt} \
                         and m.sort_seq <= ?1 and m.sort_seq >= ?5 and m.sort_seq <> 0 \
                         and (m.sort_seq, m.create_time, m.local_id) {cmp} (?1, ?2, ?3){sent} \
                         order by m.sort_seq desc, m.create_time desc, m.local_id desc limit ?4)"
                    )
                })
                .collect();
            let types = local_types
                .iter()
                .map(i64::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            parts.push(format!(
                "select * from (select * from \"{table}\" m where m.local_type in ({types}) \
                 and (m.sort_seq is null or m.sort_seq = 0) and m.create_time * 1000 >= ?5 \
                 and (m.create_time * 1000, m.create_time, m.local_id) {cmp} (?1, ?2, ?3){sent} \
                 order by m.create_time desc, m.local_id desc limit ?4)"
            ));
            let sql = format!(
                "select {} from ({}) m left join Name2Id n on n.rowid = m.real_sender_id",
                MsgRow::columns(Columns::All),
                parts.join(" union all ")
            );
            let page = self.read_rows(
                t,
                &sql,
                &[
                    &pos.sort_seq,
                    &pos.create_time,
                    &pos.local_id,
                    &limit,
                    &floor,
                ],
            )?;
            rows.extend(page.into_iter().map(|r| (r, shard)));
        }
        rows.sort_by(|(a, sa), (b, sb)| b.key().cmp(&a.key()).then(sa.cmp(sb)));
        if let Ok(limit) = usize::try_from(limit) {
            rows.truncate(limit);
        }
        Ok(rows.into_iter().map(|(r, _)| r).collect())
    }

    /// 表中第 `n` 大的 `sort_seq`：最新的 `n` 行的 `sort_seq` 都不小于它。表的行数不到 `n`，或者那一行没有
    /// `sort_seq` 时为 `None`（这时只有完整排序才准确）。
    fn sort_seq_floor(&self, t: &MsgTable, n: i64) -> Result<Option<i64>> {
        let sql = format!(
            "select sort_seq from \"{}\" order by sort_seq desc limit 1 offset ?1",
            t.table
        );
        let rows = self.query(&t.db, &sql, &[&(n - 1)])?;
        Ok(rows.first().and_then(|r| r["sort_seq"].as_i64()))
    }
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "测试用的值已知放得下，或者本来就要回绕"
)]
mod tests {
    use super::*;
    use crate::db::hex;
    use crate::fixture::TestDir;
    use crate::msg::tests::{keyed_account, texts, T0};
    use crate::msg::{table_name_for, Order};
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;
    use rusqlite::{params, Connection};

    #[test]
    fn newest_pages_are_exact_when_sort_seq_ties_cross_the_page_boundary() {
        let dir = TestDir::new("weflow-msg-ties");
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let table = table_name_for("wxid_bob");
        // 60 行：sort_seq 只有 6 个不同的值（每个 10 行），有的行没有 sort_seq；create_time 不全相同
        let plain = plain_db_with(move |c: &Connection| {
            c.execute_batch("create table Name2Id(user_name text primary key, is_session integer)")
                .unwrap();
            c.execute_batch(&format!(
                "create table \"{table}\"(local_id integer primary key autoincrement, server_id integer, local_type integer, \
                 sort_seq integer, real_sender_id integer, create_time integer, status integer, compress_content, \
                 message_content, source text, packed_info_data blob)"
            ))
            .unwrap();
            c.execute_batch(&format!(
                "create index \"{table}_SORTSEQ\" on \"{table}\"(sort_seq)"
            ))
            .unwrap();
            for i in 0..60i64 {
                let seq: Option<i64> = if i % 13 == 0 {
                    None
                } else {
                    Some((i % 6) * 1000)
                };
                c.execute(
                    &format!("insert into \"{table}\"(local_id, local_type, sort_seq, real_sender_id, create_time, message_content) values (?1, 1, ?2, 1, ?3, ?4)"),
                    params![i + 1, seq, T0 + (i * 7) % 11, format!("m{i}")],
                )
                .unwrap();
            }
        });
        let cipher = PageCipher::derive(&KEY, &SALT);
        std::fs::write(
            dir.join("message/message_0.db"),
            encrypt_db(&plain, &cipher),
        )
        .unwrap();
        let acct = Account::new(dir.path(), &hex(&KEY)).unwrap();
        // 参照：全部的行按键排序后再取这一页。没有 sort_seq（null 或 0）的行按 create_time * 1000 排，
        // 所以在这里是最新的行
        let t = acct.message_tables("wxid_bob").unwrap().remove(0);
        let reference = |limit: i32, offset: i32| -> Vec<String> {
            let sql = Account::select_sql(&t, "");
            let mut rows = acct.read_rows(&t, &sql, &[]).unwrap();
            rows.sort_by_key(|r| std::cmp::Reverse(r.key()));
            let page: Vec<MsgRow> = rows
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .collect();
            texts(&page)
        };
        assert_eq!(acct.messages("wxid_bob", 0, 0).unwrap().len(), 60);
        for (limit, offset) in [
            (1, 0),
            (5, 0),
            (10, 0),
            (7, 3),
            (10, 10),
            (25, 30),
            (40, 15),
            (50, 5),
            (100, 0),
        ] {
            let page = texts(&acct.messages("wxid_bob", limit, offset).unwrap());
            assert_eq!(
                page,
                reference(limit, offset),
                "limit {limit} offset {offset}"
            );
            assert_eq!(
                page.len(),
                (limit as usize).min(60 - (offset as usize).min(60))
            );
        }

        // 从游标往两个方向翻页，经过相同的键和没有 sort_seq 的行时，列出的顺序也相同
        let all = acct.messages("wxid_bob", 0, 0).unwrap();
        let pos = |row: &MsgRow| {
            let create_time = row.create_time.unwrap();
            MsgPos {
                sort_seq: row
                    .sort_seq
                    .filter(|s| *s != 0)
                    .unwrap_or(create_time * 1000),
                create_time,
                local_id: row.local_id,
                shard: row.shard,
            }
        };
        assert!(all.iter().all(|r| r.pos() == pos(r)));
        let mut back = texts(&acct.messages("wxid_bob", 7, 0).unwrap());
        while back.len() < 60 {
            let last = &all[back.len() - 1];
            let page = acct
                .messages_from("wxid_bob", 7, PageFrom::Before(pos(last)))
                .unwrap();
            assert!(!page.is_empty(), "stuck at {}", back.len());
            back.extend(texts(&page));
        }
        assert_eq!(back, reference(60, 0));
        assert!(acct
            .messages_from("wxid_bob", 7, PageFrom::Before(pos(&all[59])))
            .unwrap()
            .is_empty());
        for at in [1, 9, 30, 59] {
            // 列在第 `at` 行之前的 7 行
            let page = acct
                .messages_from("wxid_bob", 7, PageFrom::After(pos(&all[at])))
                .unwrap();
            let from = at.saturating_sub(7);
            assert_eq!(
                texts(&page),
                reference((at - from) as i32, from as i32),
                "after row {at}"
            );
        }
        assert_eq!(MsgPos::parse(&pos(&all[3]).token()), Some(pos(&all[3])));
        assert_eq!(MsgPos::parse("1.2.3"), None);
        assert_eq!(MsgPos::parse("1.2.3.4.5"), None);
    }

    #[test]
    fn offset_pages_are_the_pages_of_reading_every_row() {
        // 同一张表内相同的 sort_seq、没有 sort_seq 的行（null 或 0）、负的 sort_seq，
        // 以及第二个数据库中键与第一个数据库完全相同的行
        let row = |i: i64, salt: i64| -> (i64, String, String) {
            let seq = match i {
                _ if i % 9 == 0 => "null".to_string(),
                _ if i % 11 == 0 => "0".to_string(),
                _ if i % 7 == 0 => "-5".to_string(),
                _ => (T0 * 1000 + (i * salt) % 8 * 1000).to_string(),
            };
            (i, seq, (T0 + (i * 5) % 13).to_string())
        };
        let first: Vec<_> = (1..=40).map(|i| row(i, 1)).collect();
        let second: Vec<_> = (1..=30)
            .map(|i| if i <= 10 { row(i, 1) } else { row(i, 3) })
            .collect();
        let (_dir, acct) = keyed_account("offset", &[first, second]);
        let tables = acct.message_tables("wxid_bob").unwrap();
        assert_eq!(tables.len(), 2);
        for limit in [0, 1, 3, 7, 50] {
            for offset in 0..=72 {
                if offset > 0 {
                    assert!(
                        !matches!(
                            acct.offset_position(&tables, offset, limit).unwrap(),
                            OffsetPosition::Unusual
                        ),
                        "limit {limit} offset {offset}"
                    );
                }
                assert_eq!(
                    acct.messages("wxid_bob", limit as i32, offset as i32)
                        .unwrap(),
                    acct.newest_rows(&tables, offset, limit).unwrap(),
                    "limit {limit} offset {offset}"
                );
            }
        }
    }

    #[test]
    fn offset_pages_over_keys_of_other_types_read_every_row() {
        for (tag, seq, create) in [
            ("numeric-text", "'1.5'", "1"),
            ("real", "1.5", "1"),
            ("text", "'x'", "1"),
            ("blob", "x'31'", "1"),
            ("null-time", "1000", "null"),
        ] {
            let mut rows: Vec<(i64, String, String)> = (1..=6)
                .map(|i| (i, (i * 1000).to_string(), i.to_string()))
                .collect();
            rows.push((7, seq.to_string(), create.to_string()));
            let (_dir, acct) = keyed_account(&format!("unusual-{tag}"), &[rows]);
            let tables = acct.message_tables("wxid_bob").unwrap();
            // 不是整数的时间在影响结果的地方能被发现：在页里，或者与页两端的键相同
            assert_eq!(
                matches!(
                    acct.offset_position(&tables, 3, 0).unwrap(),
                    OffsetPosition::Unusual
                ),
                tag != "null-time",
                "{tag}"
            );
            for offset in 1..=8 {
                assert_eq!(
                    acct.messages("wxid_bob", 2, offset as i32).unwrap(),
                    acct.newest_rows(&tables, offset, 2).unwrap(),
                    "{tag} offset {offset}"
                );
            }
        }
    }

    /// 给 [`keyed_account`] 的一个数据库的行：`sort_seq` 有相同的、缺失的（null 或 0）、负的和正的，
    /// `create_time` 也有相同的；每个数据库的本地 ID 都从 1 开始，所以彼此重复。
    fn shard_rows() -> impl proptest::strategy::Strategy<Value = Vec<(i64, String, String)>> {
        use proptest::prelude::*;
        prop::collection::vec((0u8..10, 0i64..6, 0i64..8), 0..30).prop_map(|cells| {
            (1i64..)
                .zip(cells)
                .map(|(id, (kind, seq, time))| {
                    let seq = match kind {
                        0 => "null".to_string(),
                        1 => "0".to_string(),
                        2 => "-5".to_string(),
                        _ => (T0 * 1000 + seq * 1000).to_string(),
                    };
                    (id, seq, (T0 + time).to_string())
                })
                .collect()
        })
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(24))]

        /// 任意偏移的任意一页，以及游标读出的每一批，都与读出全部的行再排序得到的结果相同（#502）。
        #[test]
        fn any_page_is_the_page_of_reading_every_row(
            shards in proptest::collection::vec(shard_rows(), 1..=3),
            pages in proptest::collection::vec((0usize..100, 0usize..25), 1..6),
            batch in 1i32..40,
        ) {
            let (_dir, acct) = keyed_account("any-page", &shards);
            let tables = acct.message_tables("wxid_bob").unwrap();
            for (offset, limit) in pages {
                proptest::prop_assert_eq!(
                    acct.messages("wxid_bob", limit as i32, offset as i32).unwrap(),
                    acct.newest_rows(&tables, offset, limit).unwrap(),
                    "offset {} limit {}",
                    offset,
                    limit
                );
            }
            let every = acct.newest_rows(&tables, 0, 0).unwrap();
            proptest::prop_assert_eq!(every.len(), shards.iter().map(Vec::len).sum::<usize>());
            let cursor = acct.open_message_cursor("wxid_bob", batch, Order::NewestFirst, 0, 0, Columns::All).unwrap();
            let mut read = Vec::new();
            loop {
                let (rows, more) = acct.fetch_message_batch(cursor).unwrap();
                read.extend(rows);
                if !more {
                    break;
                }
            }
            acct.close_message_cursor(cursor).unwrap();
            proptest::prop_assert_eq!(read, every);
        }
    }
}
