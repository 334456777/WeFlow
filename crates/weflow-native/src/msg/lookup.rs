//! 按类型、本地 ID 或服务器 ID 查找消息。

use anyhow::{anyhow, Result};

use super::Order;
use crate::db::Account;
use crate::msg_row::MsgRow;

impl Account {
    /// 会话中一种 `local_type` 的全部消息，按聊天窗口的顺序，最早或最新的在前：先按 `sort_seq`（没有时用
    /// `create_time * 1000`），再按 `create_time` 和 `local_id`；两个方向下不同分片的相同键都按分片号升序。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn messages_by_type(
        &self,
        session_id: &str,
        local_type: i64,
        order: Order,
    ) -> Result<Vec<MsgRow>> {
        let mut rows: Vec<MsgRow> = Vec::new();
        for t in self.message_tables(session_id)? {
            let sql = Self::select_sql(&t, "where m.local_type = ?1");
            rows.extend(self.read_rows(&t, &sql, &[&local_type])?);
        }
        // 稳定排序：相同的键保持数据库的顺序
        match order {
            Order::OldestFirst => rows.sort_by_key(MsgRow::key),
            Order::NewestFirst => rows.sort_by_key(|r| std::cmp::Reverse(r.key())),
        }
        Ok(rows)
    }

    /// 会话中 `local_id` 为这个值的全部消息，最新的分片在前：本地 ID 只在一个 `message_N.db` 内唯一，
    /// 不同分片里的不同消息可能相同。给出 `create_time`（秒）时只保留那一刻发送的消息。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
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

    /// 按服务器 ID（十进制字符串，可能超过 2^53）查找一条消息。
    ///
    /// # Errors
    ///
    /// `server_id` 不是整数，或者某个消息库无法读取（打开、解密或查询失败）。
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
    use crate::msg::Order;

    #[test]
    fn lookups_by_type_and_id() {
        let (_dir, acct) = account("lookup");
        assert_eq!(
            texts(
                &acct
                    .messages_by_type("wxid_bob", 1, Order::OldestFirst)
                    .unwrap()
            ),
            ["hello", "third", "fourth"]
        );
        assert_eq!(
            texts(
                &acct
                    .messages_by_type("wxid_bob", 1, Order::NewestFirst)
                    .unwrap()
            ),
            ["fourth", "third", "hello"]
        );
        assert!(acct
            .messages_by_type("wxid_bob", 999, Order::NewestFirst)
            .unwrap()
            .is_empty());
        // 两个分片里都有 local_id 1：最新的分片在前
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

    /// 没有 `sort_seq` 的消息按时间排位置，与聊天窗口一致；
    /// 两个数据库都存了的消息，两个方向下都先列编号小的数据库中的那条。
    #[test]
    fn messages_by_type_follow_the_chat_order() {
        let row = |id: i64, seq: String, create: i64| (id, seq, create.to_string());
        let (_dir, acct) = crate::msg::tests::keyed_account(
            "by-type-order",
            &[
                vec![
                    row(1, "0".into(), T0 + 100),
                    row(2, (T0 * 1000).to_string(), T0),
                    row(3, ((T0 + 200) * 1000).to_string(), T0 + 200),
                ],
                vec![row(1, ((T0 + 100) * 1000).to_string(), T0 + 100)],
            ],
        );
        assert_eq!(
            texts(
                &acct
                    .messages_by_type("wxid_bob", 1, Order::OldestFirst)
                    .unwrap()
            ),
            ["0:2", "0:1", "1:1", "0:3"]
        );
        assert_eq!(
            texts(
                &acct
                    .messages_by_type("wxid_bob", 1, Order::NewestFirst)
                    .unwrap()
            ),
            ["0:3", "0:1", "1:1", "0:2"]
        );
    }
}
