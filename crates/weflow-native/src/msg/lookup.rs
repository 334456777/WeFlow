//! Messages found by type, by local id or by server id.

use anyhow::{anyhow, Result};

use crate::db::Account;
use crate::msg_row::MsgRow;

impl Account {
    /// Messages of one `local_type`; `limit == 0` means all.
    pub fn messages_by_type(
        &self,
        session_id: &str,
        local_type: i64,
        ascending: bool,
        limit: i32,
        offset: i32,
    ) -> Result<Vec<MsgRow>> {
        let (offset, limit) = (offset.max(0) as usize, limit.max(0) as usize);
        let want = if limit == 0 {
            -1
        } else {
            (offset + limit) as i64
        };
        let dir = if ascending { "asc" } else { "desc" };
        let mut rows: Vec<MsgRow> = Vec::new();
        for t in self.message_tables(session_id)? {
            let sql = Self::select_sql(
                &t,
                &format!("where m.local_type = ?1 order by m.sort_seq {dir}, m.create_time {dir}, m.local_id {dir} limit ?2"),
            );
            rows.extend(self.read_rows(&t, &sql, &[&local_type, &want])?);
        }
        if ascending {
            rows.sort_by_key(MsgRow::key);
        } else {
            rows.sort_by_key(|r| std::cmp::Reverse(r.key()));
        }
        let it = rows.into_iter().skip(offset);
        Ok(if limit == 0 {
            it.collect()
        } else {
            it.take(limit).collect()
        })
    }

    /// Every message of a session with this `local_id`, newest shard first: the id is only unique within one
    /// `message_N.db`, so different messages in different shards can share it. `create_time` (seconds) keeps only
    /// the one sent then.
    pub fn messages_by_local_id(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: Option<i64>,
    ) -> Result<Vec<MsgRow>> {
        let mut out = Vec::new();
        for t in self.message_tables(session_id)?.into_iter().rev() {
            out.extend(match create_time {
                Some(ct) => self.read_rows(
                    &t,
                    &Self::select_sql(&t, "where m.local_id = ?1 and m.create_time = ?2 limit 1"),
                    &[&local_id, &ct],
                )?,
                None => self.read_rows(
                    &t,
                    &Self::select_sql(&t, "where m.local_id = ?1 limit 1"),
                    &[&local_id],
                )?,
            });
        }
        Ok(out)
    }

    /// One message by server id (a decimal string, may exceed 2^53).
    pub fn message_by_server_id(
        &self,
        session_id: &str,
        server_id: &str,
    ) -> Result<Option<MsgRow>> {
        let id: i64 = server_id
            .trim()
            .parse()
            .map_err(|_| anyhow!("invalid server id: {server_id}"))?;
        self.find_one(session_id, "m.server_id = ?1", &id)
    }

    fn find_one(&self, session_id: &str, cond: &str, arg: &i64) -> Result<Option<MsgRow>> {
        for t in self.message_tables(session_id)?.into_iter().rev() {
            let sql = Self::select_sql(&t, &format!("where {cond} limit 1"));
            if let Some(r) = self.read_rows(&t, &sql, &[arg])?.into_iter().next() {
                return Ok(Some(r));
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use crate::msg::tests::account;
    use crate::msg::tests::{texts, DAY, T0};

    #[test]
    fn lookups_by_type_and_id() {
        let (_dir, acct) = account("lookup");
        assert_eq!(
            texts(&acct.messages_by_type("wxid_bob", 1, true, 0, 0).unwrap()),
            ["hello", "third", "fourth"]
        );
        assert_eq!(
            texts(&acct.messages_by_type("wxid_bob", 1, false, 2, 0).unwrap()),
            ["fourth", "third"]
        );
        assert!(acct
            .messages_by_type("wxid_bob", 999, false, 0, 0)
            .unwrap()
            .is_empty());
        // local_id 1 exists in both shards: the newest shard first
        assert_eq!(
            texts(&acct.messages_by_local_id("wxid_bob", 1, None).unwrap()),
            ["fourth", "hello"]
        );
        assert_eq!(
            texts(&acct.messages_by_local_id("wxid_bob", 1, Some(T0)).unwrap()),
            ["hello"]
        );
        assert_eq!(
            texts(&acct.messages_by_local_id("wxid_bob", 3, None).unwrap()),
            ["third"]
        );
        assert!(acct
            .messages_by_local_id("wxid_bob", 77, None)
            .unwrap()
            .is_empty());
        assert_eq!(
            acct.message_by_server_id("wxid_bob", &(DAY * 4).to_string())
                .unwrap()
                .map(|r| r.content),
            Some("voice".to_string())
        );
        assert_eq!(acct.message_by_server_id("wxid_bob", "77").unwrap(), None);
        assert!(acct.message_by_server_id("wxid_bob", "abc").is_err());
    }
}
