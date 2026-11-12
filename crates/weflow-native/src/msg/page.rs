//! Pages of a conversation: the newest messages, pages by offset and pages before or after a message.

use anyhow::{Context, Result};

use super::MsgPos;
use super::{shard_of, typed_row_key};
use super::{MsgTable, PageFrom, Ties};
use crate::db::Account;
use crate::msg_row::{Columns, MsgRow};

/// A row's sort key (see [`MsgPos`]) and the number of its `message_<n>.db`.
type ShardKey = ((i64, i64, i64), u32);

/// Where [`Account::messages`] starts a page deep into a conversation.
pub(super) enum OffsetPosition {
    /// After this message, the last one the offset skips.
    At(MsgPos),
    /// The offset skips every message.
    Past,
    /// A sort key read is not stored as an integer (a `sort_seq` may also be null): [`typed_row_key`] and SQLite order
    /// such values differently, so only reading the rows lists them as before.
    Unusual,
}

impl Account {
    /// The SQL of [`newest_rows`](Self::newest_rows) for one table: its `want` (`?1`) newest rows with a `sort_seq`,
    /// at `?2` or later when `floor`, then its `want` newest without one. The [`MsgRow`] columns with the sender joined,
    /// or only `columns`.
    fn newest_sql(t: &MsgTable, columns: Option<&str>, floor: bool) -> String {
        let order = "order by m.sort_seq desc, m.create_time desc, m.local_id desc limit ?1";
        let floor = if floor { "m.sort_seq >= ?2 and " } else { "" };
        let indexed = format!("where {floor}m.sort_seq <> 0 {order}");
        // rows without a sort_seq are keyed by create_time * 1000 (see [`MsgPos`]): below any floor, they are read
        // apart
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

    /// Where a page `offset` messages deep starts: the place of the last message it skips.
    ///
    /// The rows [`newest_rows`](Self::newest_rows) would read are ranked by the first part of their sort key alone:
    /// the `sort_seq` index holds it, so their pages are not decrypted. Only the rows tied on it with the last skipped
    /// one, or with the page's last one, are read, for their whole key: those two places are where the rest of the
    /// key can move a row across the page's ends.
    pub(super) fn offset_position(
        &self,
        tables: &[MsgTable],
        offset: usize,
        limit: usize,
    ) -> Result<OffsetPosition> {
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
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
        // the listing's order: newest key first, equal keys by ascending database
        tied.sort_unstable_by(|(a, sa), (b, sb)| b.cmp(a).then(sa.cmp(sb)));
        let above = firsts.partition_point(|&f| f > first);
        // fewer tied rows than ranked ones: the table changed between the reads
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

    /// The first part of the sort key (see [`MsgPos`]) of the `want` newest rows of `t` with a `sort_seq`, from its
    /// index, and of its `want` newest without one, from their `create_time`; all of them when `want` is negative.
    /// `None` when one is not an integer.
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
            // Each retry owns a fresh vector: a stale snapshot cannot leave partial or duplicate keys behind.
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

    /// The sort keys and databases of the rows of `tables` whose key starts with `first`; `None` when a key is not
    /// stored as integers (see [`typed_row_key`]).
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

    /// Newest-first page of the messages of `tables`, from the rows before it and the page read in full and sorted.
    pub(super) fn newest_rows(
        &self,
        tables: &[MsgTable],
        offset: usize,
        limit: usize,
    ) -> Result<Vec<MsgRow>> {
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
        };
        let mut rows: Vec<MsgRow> = Vec::new();
        for t in tables {
            // Asked as is, SQLite sorts the whole table (it walks the sender index for the join). The `want`-th
            // newest sort_seq, read from the sort_seq index, bounds the same rows to a range that index serves.
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

    /// A newest-first page of messages next to a message (see [`MsgPos`]): `limit` of the older ones, or of the newer
    /// ones closest to it, still listed newest first; `limit == 0` means all of them. Each table is read from the
    /// position on through its `sort_seq` index, so a page deep into a long conversation costs what the first does.
    pub fn messages_from(
        &self,
        session_id: &str,
        limit: i32,
        from: PageFrom,
    ) -> Result<Vec<MsgRow>> {
        self.page_from(&self.message_tables(session_id)?, limit, from)
    }

    /// [`messages_from`](Self::messages_from) over the message tables of a conversation.
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
            // a row with the position's own key is past it only in a database listed after the position's
            let cmp = match (older, shard > pos.shard, shard < pos.shard) {
                (true, true, _) => "<=",
                (true, false, _) => "<",
                (false, _, true) => ">=",
                (false, _, false) => ">",
            };
            let bound = if older { "<=" } else { ">=" };
            // rows with a sort_seq, through its index; the few without one are keyed by create_time * 1000
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
        // the listing's order: newest key first, equal keys by ascending database
        rows.sort_by(|(a, sa), (b, sb)| b.key().cmp(&a.key()).then(sa.cmp(sb)));
        if !older {
            // the newer rows closest to the position are at the end
            // a negative `want` keeps every row
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

    /// Messages of `tables` (one conversation's, from [`message_tables_by_name`](Self::message_tables_by_name)) of
    /// the `local_types`, newest first in the chat's order, at most `limit` (all when negative): those listed after
    /// `before` (older; with its [`Ties`]), all when `None`, sent between the Unix seconds of `range` (inclusive; 0
    /// for no bound) and at `floor` or later in the chat's order (its `sort_seq`; 0 for no bound). One statement a
    /// table, of one part a type, so the `(local_type, sort_seq)` index gives each part's rows in order and the
    /// limit stops its read (issue #178).
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
            // a part a type, so each reads its index in order; the few rows without a sort_seq in one part of all
            // the types; the senders joined once, outside (each part takes its time to plan)
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

    /// The `n`-th largest `sort_seq` of a table: every one of its `n` newest rows has a `sort_seq` at least this big.
    /// `None` when the table has fewer rows, or that row has no `sort_seq` (then only a full sort is exact).
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
    reason = "test values are known to fit, or are meant to wrap"
)]
mod tests {
    use super::*;
    use crate::db::hex;
    use crate::fixture::TestDir;
    use crate::msg::table_name_for;
    use crate::msg::tests::{keyed_account, texts, T0};
    use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
    use crate::sqlcipher::PageCipher;
    use rusqlite::{params, Connection};

    #[test]
    fn newest_pages_are_exact_when_sort_seq_ties_cross_the_page_boundary() {
        let dir = TestDir::new("weflow-msg-ties");
        std::fs::create_dir_all(dir.join("message")).unwrap();
        let table = table_name_for("wxid_bob");
        // 60 rows: sort_seq takes only 6 distinct values (10 rows each), some rows have none; create_time varies
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
        // every row sorted by its key, then the page taken: rows without a sort_seq (null or 0) are keyed by
        // create_time * 1000, so here they are the newest ones
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

        // pages from a cursor, both ways, list the same order through the ties and the rows without a sort_seq
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
            // the 7 rows listed just before row `at`
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
        // sort_seq ties within a table, rows without one (null or 0), negative ones, and rows of the second database
        // with the very keys of the first one's
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
            // a time that is not an integer is found where it matters: in the page or tied at its ends
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

    /// The rows of one database for [`keyed_account`]: `sort_seq` tied, missing (null or 0), negative or not, and
    /// `create_time` tied; every database's local ids start at 1, so they share them.
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

        /// Any page of any offset, and the batches of a cursor, are the rows of reading every row and sorting them
        /// (#502).
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
            let cursor = acct.open_message_cursor("wxid_bob", batch, false, 0, 0, false).unwrap();
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
