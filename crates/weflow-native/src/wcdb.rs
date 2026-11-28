//! 服务层使用的数据库门面。
//!
//! 每个调用都转给纯 Rust 实现的 [`Account`]（[`crate::db`]，各领域的查询在 [`crate::msg`]、[`crate::contact`]
//! 等模块中），不依赖闭源的 `wcdb_api` 库。

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::contact::distinct_usernames;
use crate::db::Account;
use crate::msg::Order;
use crate::msg_row::{Columns, MsgRow};

/// 账号数据库的一组，供 [`Wcdb::prefetch`] 使用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Databases {
    /// 会话列表，`session/session.db`。
    Sessions,
    /// 联系人，`contact/contact.db`。
    Contacts,
    /// 所有消息库（`message/message_<n>.db`、`biz_message_<n>.db`）。
    Messages,
    /// 朋友圈，`sns/sns.db`。
    Moments,
}

/// 服务层调用的一个账号的数据库句柄，打开（[`Wcdb::open`]、[`Wcdb::open_unchecked`]）之前是关闭的。
///
/// 返回 `Result` 的方法在句柄没有打开时返回错误（`WCDB is not connected`），其他错误来自所调用的 [`Account`] 方法；
/// 另有失败情形的方法单独写明。
#[derive(Debug, Default)]
pub struct Wcdb {
    native: Option<Account>,
}

impl Wcdb {
    /// 关闭的句柄。
    pub fn new() -> Self {
        Self::default()
    }

    fn account(&self) -> Result<&Account> {
        self.native
            .as_ref()
            .ok_or_else(|| anyhow!("WCDB is not connected"))
    }

    /// 打开账号：用密钥解密 `session.db`，确认密钥和路径都正确。
    ///
    /// # Errors
    ///
    /// 找不到 `session.db`、密钥的格式不对，或者密钥解不开 `session.db` 时返回错误，句柄保持关闭。
    pub fn open(&mut self, account_dir: &Path, hex_key: &str, wxid: Option<&str>) -> Result<()> {
        self.open_unchecked(account_dir, hex_key, wxid)?;
        let tested = self.account()?.test_connection();
        if tested.is_err() {
            self.native = None; // 密钥不能用的句柄保持关闭
        }
        tested
    }

    /// 不预先验证密钥的 [`open`](Self::open)：每个数据库在第一次读取时检查密钥，所以从不读 `session.db`
    /// 的命令省去它较慢的密钥派生。密钥不对时仍会失败，只是在第一次读取时才失败，报错同样是
    /// `cannot decrypt ... with the configured database key`。
    ///
    /// # Errors
    ///
    /// 找不到 `session.db` 或者密钥的格式不对时返回错误。
    pub fn open_unchecked(
        &mut self,
        account_dir: &Path,
        hex_key: &str,
        wxid: Option<&str>,
    ) -> Result<()> {
        self.native = None;
        let session_db = find_session_db(&account_dir.join("db_storage"))
            .ok_or_else(|| anyhow!("session.db not found under {}", account_dir.display()))?;
        // `session.db` 在 `<db_storage>/session/` 下，所以它的上两级目录就是 `db_storage`。
        let db_storage = session_db
            .parent()
            .and_then(Path::parent)
            .unwrap_or(account_dir);
        // 没有给出 wxid 时，用账号目录名（`<wxid>_<4 chars>`）识别所有者。
        let me = wxid.map(str::to_string).or_else(|| {
            account_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        });
        self.native = Some(Account::new(db_storage, hex_key)?.with_my_wxid(me));
        Ok(())
    }

    /// 验证用 [`open_unchecked`](Self::open_unchecked) 打开的账号的密钥（解密 `session.db`）。
    ///
    /// # Errors
    ///
    /// 句柄没有打开，或者密钥解不开 `session.db` 时返回错误。
    pub fn check_key(&self) -> Result<()> {
        self.account()?.test_connection()
    }

    /// `session.db` 派生密钥用的盐（文件的前 16 字节）。它与密钥一起标识一次成功的 [`check_key`](Self::check_key)：
    /// 换了密钥或数据库被替换，这一对值就会不同。
    pub fn session_salt(&self) -> Option<[u8; 16]> {
        use std::io::Read;
        let mut salt = [0u8; 16];
        fs::File::open(self.account().ok()?.session_db())
            .ok()?
            .read_exact(&mut salt)
            .ok()?;
        Some(salt)
    }

    /// `account_dir` 处的账号能否用 `hex_key` 打开：先 [`open`](Self::open)，再 [`close`](Self::close)。
    ///
    /// # Errors
    ///
    /// 与 [`open`](Self::open) 相同。
    pub fn test_connection(&mut self, account_dir: &Path, hex_key: &str) -> Result<()> {
        self.open(account_dir, hex_key, None)?;
        self.close();
        Ok(())
    }

    /// 关闭句柄；此后的调用返回 `WCDB is not connected`，直到重新打开。
    pub fn close(&mut self) {
        self.native = None;
    }

    // ── 会话和消息 ──

    /// `SessionTable` 的各行，最新的在前（见 [`Account::sessions`]）。
    pub fn sessions(&self) -> Result<Value> {
        self.account()?.sessions()
    }

    /// 跨各个消息库的一页会话消息，最新的在前（见 [`Account::messages`]）。
    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Vec<MsgRow>> {
        self.account()?.messages(session_id, limit, offset)
    }

    /// 见 [`Account::messages_from`](crate::db::Account::messages_from)。
    pub fn messages_from(
        &self,
        session_id: &str,
        limit: i32,
        from: crate::msg::PageFrom,
    ) -> Result<Vec<MsgRow>> {
        self.account()?.messages_from(session_id, limit, from)
    }

    /// 见 [`Account::messages_where`]。
    pub fn messages_where(
        &self,
        session_id: &str,
        since: i64,
        condition: &str,
        limit: usize,
    ) -> Result<Vec<MsgRow>> {
        self.account()?
            .messages_where(session_id, since, condition, limit)
    }

    /// 见 [`Account::message_tables_by_name`]。
    pub fn message_tables_by_name(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<crate::msg::MsgTable>>> {
        self.account()?.message_tables_by_name()
    }

    /// 见 [`Account::typed_before`]。
    pub fn typed_before(
        &self,
        tables: &[crate::msg::MsgTable],
        local_types: &[i64],
        before: Option<(crate::msg::MsgPos, crate::msg::Ties)>,
        range: (i64, i64),
        floor: i64,
        limit: i64,
    ) -> Result<Vec<MsgRow>> {
        self.account()?
            .typed_before(tables, local_types, before, range, floor, limit)
    }

    /// 同时打开聊天页面要读的数据库（见 [`Account::warm`](crate::db::Account::warm)），返回打开成功的个数。
    pub fn warm_chat_databases(&self) -> Result<usize> {
        let account = self.account()?;
        Ok(account.warm(&account.chat_databases()))
    }

    /// 在逐个读取 `groups` 中的数据库之前，先并行打开它们：每个库第一次打开时要派生密钥（PBKDF2，约 130 ms，
    /// 只能在一个线程上算），读到哪个才打开哪个时这些时间会累加，在这里打开则相互重叠（#1100）。
    /// 已经打开的不花时间（见 [`Account::prefetch`](crate::db::Account::prefetch)）。
    pub fn prefetch(&self, groups: &[Databases]) -> Result<()> {
        let account = self.account()?;
        let mut paths = Vec::new();
        for group in groups {
            match group {
                Databases::Sessions => paths.push(account.session_db()),
                Databases::Contacts => paths.push(account.db_path("contact/contact.db")),
                Databases::Messages => paths.extend(account.message_dbs()),
                Databases::Moments => paths.push(account.db_path("sns/sns.db")),
            }
        }
        paths.retain(|p| p.is_file());
        account.prefetch(&paths);
        Ok(())
    }

    /// 会话的消息数（见 [`Account::message_count`]）。
    pub fn message_count(&self, session_id: &str) -> Result<i32> {
        self.account()?.message_count(session_id)
    }

    /// 会话中有消息的日期，按本地时间写成 `YYYY-MM-DD`，最早的在前（见 [`Account::message_dates`]）。
    pub fn message_dates(&self, session_id: &str) -> Result<Value> {
        self.account()?.message_dates(session_id)
    }

    /// `{ username: count }`：`session_ids` 中每个会话的消息数（见 [`Account::session_message_counts`]）。
    pub fn session_message_counts(&self, session_ids: &[String]) -> Result<Value> {
        self.account()?.session_message_counts(session_ids)
    }

    /// `{ "YYYY-MM-DD": count }`：会话按本地日期统计的消息数（见 [`Account::session_message_date_counts`]）。
    pub fn session_message_date_counts(&self, session_id: &str) -> Result<Value> {
        self.account()?.session_message_date_counts(session_id)
    }

    /// 某一种 `local_type` 的全部消息（见 [`Account::messages_by_type`]）。
    pub fn messages_by_type(
        &self,
        session_id: &str,
        local_type: i64,
        order: Order,
    ) -> Result<Vec<MsgRow>> {
        self.account()?
            .messages_by_type(session_id, local_type, order)
    }

    /// `local_id` 为这个值的全部消息（每个消息库最多一条），给出 `create_time` 时只取这个时间发送的。
    pub fn messages_by_local_id(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: Option<i64>,
    ) -> Result<Vec<MsgRow>> {
        self.account()?
            .messages_by_local_id(session_id, local_id, create_time)
    }

    /// 会话中服务器 ID 为 `svrid` 的消息，
    /// `svrid` 是可能超过 2^53 的十进制字符串（见 [`Account::message_by_server_id`]）。
    pub fn message_by_server_id(&self, session_id: &str, svrid: &str) -> Result<Option<MsgRow>> {
        self.account()?.message_by_server_id(session_id, svrid)
    }

    /// 在会话中 `begin` 到 `end` 之间（秒；0 表示不限）的消息上打开游标，
    /// 返回游标的 ID（见 [`Account::open_message_cursor`]）。
    pub fn open_message_cursor(
        &self,
        session_id: &str,
        batch_size: i32,
        order: Order,
        begin: i32,
        end: i32,
        columns: Columns,
    ) -> Result<i64> {
        self.account()?
            .open_message_cursor(session_id, batch_size, order, begin, end, columns)
    }

    /// 导出用的游标（见 [`Account::open_export_cursor`]）。
    pub fn open_export_cursor(
        &self,
        session_id: &str,
        batch_size: i32,
        begin: i32,
        end: i32,
    ) -> Result<i64> {
        self.account()?
            .open_export_cursor(session_id, batch_size, begin, end)
    }

    /// 只含 `senders` 选中的发送者的消息的游标（见 `Account::open_message_cursor_for_senders`）。
    pub fn open_message_cursor_for_senders(
        &self,
        session_id: &str,
        batch_size: i32,
        order: Order,
        range: (i32, i32),
        columns: Columns,
        senders: crate::msg::SenderFilter<'_>,
    ) -> Result<i64> {
        self.account()?
            .open_message_cursor_for_senders(session_id, batch_size, order, range, columns, senders)
    }

    /// 返回下一批行，以及是否还有剩余。
    pub fn fetch_message_batch(&self, cursor: i64) -> Result<(Vec<MsgRow>, bool)> {
        self.account()?.fetch_message_batch(cursor)
    }

    /// 游标的消息数和每批的大小（见 [`Account::message_cursor_len`]）。
    pub fn message_cursor_len(&self, cursor: i64) -> Result<(usize, usize)> {
        self.account()?.message_cursor_len(cursor)
    }

    /// 游标的第 `page` 批，不移动游标（见 [`Account::fetch_message_page`]）。
    pub fn fetch_message_page(&self, cursor: i64, page: usize) -> Result<Vec<MsgRow>> {
        self.account()?.fetch_message_page(cursor, page)
    }

    /// 关闭游标，释放它持有的资源（见 [`Account::close_message_cursor`]）。
    pub fn close_message_cursor(&self, cursor: i64) -> Result<()> {
        self.account()?.close_message_cursor(cursor)
    }

    /// 设置数据库为这个句柄的查询最多打开的连接数（见 [`Account::set_read_connections`]）。
    pub fn set_read_connections(&self, n: usize) {
        if let Some(a) = &self.native {
            a.set_read_connections(n);
        }
    }

    /// 到目前为止在新快照上重新执行的查询数（见 [`Account::stale_reruns`]）。
    pub fn stale_reruns(&self) -> usize {
        self.native.as_ref().map_or(0, Account::stale_reruns)
    }

    // ── 会话的统计 ──

    /// `usernames` 的精简联系人行：名字、微信号、头像和类型（见 [`Account::contacts_compact`]）。
    pub fn contacts_compact(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .contacts_compact(&distinct_usernames(usernames))
    }

    /// `session_ids` 在 `begin` 到 `end` 之间的消息，按方向、类型和时间统计（见 [`Account::aggregate_stats`]）。
    pub fn aggregate_stats(
        &self,
        session_ids: &[impl AsRef<str>],
        begin: i32,
        end: i32,
    ) -> Result<Value> {
        self.account()?.aggregate_stats(
            &distinct_usernames(session_ids),
            i64::from(begin),
            i64::from(end),
        )
    }

    /// 年度报告所依据的总数：`session_ids` 在 `begin` 到 `end` 之间的统计（见 [`Account::annual_report_stats`]）。
    pub fn annual_report_stats(
        &self,
        session_ids: &[impl AsRef<str>],
        begin: i32,
        end: i32,
    ) -> Result<Value> {
        self.account()?.annual_report_stats(
            &distinct_usernames(session_ids),
            i64::from(begin),
            i64::from(end),
        )
    }

    /// 数据库 `db` 中表 `table` 的第一条和最后一条消息的时间（见 [`Account::message_table_time_range`]）。
    pub fn message_table_time_range(&self, db: &str, table: &str) -> Result<Value> {
        self.account()?.message_table_time_range(db, table)
    }

    // ── 搜索、联系人、群、统计、朋友圈和媒体 ──

    /// 文字含有 `keyword` 的消息，最新的在前，每条结果带上它所在的会话（见 [`Account::search_messages`]）。
    pub fn search(
        &self,
        keyword: &str,
        session_id: Option<&str>,
        limit: i32,
        offset: i32,
        begin: i32,
        end: i32,
    ) -> Result<Vec<crate::media::SessionRow>> {
        self.account()?.search_messages(
            keyword,
            session_id,
            limit,
            offset,
            i64::from(begin),
            i64::from(end),
        )
    }

    /// 一个联系人的行，未知时为 `{}`（见 [`Account::contact`]）。
    pub fn contact(&self, username: &str) -> Result<Value> {
        self.account()?.contact(username)
    }

    /// `contact` 表的全部行（见 [`Account::contacts`]）。
    pub fn contacts(&self) -> Result<Value> {
        self.account()?.contacts()
    }

    /// `contact` 表中可能是公众号、`weixin…` 或 `@openim` 账号的行（见 [`Account::contacts_maybe_special`]）。
    pub fn contacts_maybe_special(&self) -> Result<Value> {
        self.account()?.contacts_maybe_special()
    }

    /// `{ "private", "group", "official", "former_friend" }`：
    /// 每种联系人的个数（见 [`Account::contact_type_counts`]）。
    pub fn contact_type_counts(&self) -> Result<Value> {
        self.account()?.contact_type_counts()
    }

    /// 群的成员数（见 [`Account::group_member_count`]）。
    pub fn group_member_count(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_member_count(chatroom_id)
    }

    /// 群成员，与桌面版列出的相同（见 [`Account::group_members`]）。
    pub fn group_members(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_members(chatroom_id)
    }

    /// 群成员的 `{ username: group nickname }`（见 [`Account::group_nicknames`]）。
    pub fn group_nicknames(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_nicknames(chatroom_id)
    }

    /// `session_ids` 中每个会话的消息数、发出和收到的消息数，以及最后一条的时间（见 [`Account::session_totals`]）。
    pub fn session_totals(&self, session_ids: &[String], begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .session_totals(session_ids, i64::from(begin), i64::from(end))
    }

    /// 群在 `begin` 到 `end` 之间的统计：按发送者、按小时和按类型的消息数（见 [`Account::group_stats`]）。
    pub fn group_stats(&self, chatroom_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .group_stats(chatroom_id, i64::from(begin), i64::from(end))
    }

    /// `session_ids` 中任一会话有消息的年份（本地时间），最新的在前（见 [`Account::available_years`]）。
    pub fn available_years(&self, session_ids: &[String]) -> Result<Value> {
        self.account()?.available_years(session_ids)
    }

    /// 与 `session_id` 的聊天在 `begin` 到 `end` 之间的双人报告统计（见 [`Account::dual_report_stats`]）。
    pub fn dual_report_stats(&self, session_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .dual_report_stats(session_id, i64::from(begin), i64::from(end))
    }

    /// `options` 给出的时间范围和会话内的足迹（见 [`Account::footprint_stats`]）。
    pub fn footprint_stats(&self, options: &Value) -> Result<Value> {
        self.account()?.footprint_stats(options)
    }

    /// 一页朋友圈动态，最新的在前，只含 `usernames` 中发布者的动态（为空时不限）。
    pub fn sns_timeline(
        &self,
        limit: i32,
        offset: i32,
        usernames: &[String],
        keyword: Option<&str>,
        start: i32,
        end: i32,
    ) -> Result<Vec<crate::sns::SnsPost>> {
        self.account()?.sns_timeline(
            limit,
            offset,
            usernames,
            keyword,
            i64::from(start),
            i64::from(end),
        )
    }

    /// 账号所有者在 `begin` 到 `end` 之间的朋友圈：动态及其类型、给这些动态点赞最多的人，
    /// 以及所有者点赞最多的是谁（见 [`Account::sns_annual_stats`]）。
    pub fn sns_annual_stats(&self, begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .sns_annual_stats(i64::from(begin), i64::from(end))
    }

    /// 发过朋友圈的人，最近发过的在前（见 [`Account::sns_usernames`]）。
    pub fn sns_usernames(&self) -> Result<Value> {
        self.account()?.sns_usernames()
    }

    /// 每个发布者的 `{ username: posts }`（见 [`Account::sns_post_counts`]）。
    pub fn sns_post_counts(&self) -> Result<Value> {
        self.account()?.sns_post_counts()
    }

    /// 朋友圈的 `{ total_posts, total_friends, my_posts }`（见 [`Account::sns_export_stats`]）。
    pub fn sns_export_stats(&self, my_wxid: Option<&str>) -> Result<Value> {
        self.account()?.sns_export_stats(my_wxid)
    }

    /// 在一个数据库上执行只读语句，以 JSON 对象返回各行（见 [`Account::exec_query`]）。
    pub fn exec_query(&self, kind: &str, path: &str, sql: &str) -> Result<Value> {
        self.account()?.exec_query(kind, Some(path), sql)
    }

    /// `session_ids` 这些会话中的图片、视频消息，最新的在前，以及是否还有更多。
    pub fn scan_media_stream(
        &self,
        session_ids: &[String],
        media_type: i32,
        begin: i32,
        end: i32,
        limit: i32,
        offset: i32,
    ) -> Result<(Vec<crate::media::SessionRow>, bool)> {
        self.account()?.scan_media_stream(
            session_ids,
            media_type,
            i64::from(begin),
            i64::from(end),
            limit,
            offset,
        )
    }

    /// `usernames` 的 `{ username: name }`：
    /// 依次取备注、昵称、微信号，都没有时用用户名（见 [`Account::display_names`]）。
    pub fn display_names(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .display_names(&distinct_usernames(usernames))
    }

    /// 本机保存的 `username` 的头像（见 `Account::head_image`）。
    pub fn head_image(&self, username: &str) -> Result<Option<Vec<u8>>> {
        self.account()?.head_image(username)
    }

    /// `usernames` 中有头像的联系人的 `{ username: url }`（见 [`Account::avatar_urls`]）。
    pub fn avatar_urls(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?.avatar_urls(&distinct_usernames(usernames))
    }

    /// `chatroom_ids` 这些群的 `{ chatroom: count }`（见 [`Account::group_member_counts`]）。
    pub fn group_member_counts(&self, chatroom_ids: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .group_member_counts(&distinct_usernames(chatroom_ids))
    }

    /// `usernames` 的每个会话是否折叠、免打扰、置顶（见 [`Account::contact_status`]）。
    pub fn contact_status(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .contact_status(&distinct_usernames(usernames))
    }

    /// `usernames` 中有微信号的联系人的 `{ username: alias }`（见 [`Account::contact_alias_map`]）。
    pub fn contact_alias_map(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .contact_alias_map(&distinct_usernames(usernames))
    }

    /// 群的 `{ "ext_buffer": "<hex>" }`，未知时为 `{}`（见 [`Account::chat_room_ext_buffer`]）。
    pub fn chat_room_ext_buffer(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.chat_room_ext_buffer(chatroom_id)
    }

    /// 会话的每张表一行：所在的数据库、表名、消息数，
    /// 以及第一条和最后一条消息的时间（见 [`Account::message_table_stats`]）。
    pub fn message_table_stats(&self, session_id: &str) -> Result<Value> {
        self.account()?.message_table_stats(session_id)
    }

    /// 按顺序对几个会话各做一次 [`message_table_stats`](Self::message_table_stats)，
    /// 每个数据库只列一次表，从中找出各会话的表（见 [`Account::message_table_stats_of`]）。
    pub fn message_table_stats_of(&self, session_ids: &[String]) -> Result<Vec<Result<Value>>> {
        self.account()?.message_table_stats_of(session_ids)
    }

    /// 年度报告除总数以外要读的内容：热力图、深夜消息、谁先开启会话、我的回复、最高峰的一天、
    /// 我的常用语和最长的连续天数（见 [`Account::annual_report_extras`]）。
    pub fn annual_report_extras(
        &self,
        session_ids: &[impl AsRef<str>],
        begin: i32,
        end: i32,
        peak_begin: i32,
        peak_end: i32,
    ) -> Result<Value> {
        self.account()?.annual_report_extras(
            &distinct_usernames(session_ids),
            i64::from(begin),
            i64::from(end),
            i64::from(peak_begin),
            i64::from(peak_end),
        )
    }

    /// 微信的媒体索引中图片所在的位置，按消息的 md5 区分（见 `Account::image_hardlink_index`）。
    pub fn image_hardlink_index(&self) -> Result<std::collections::HashMap<String, Vec<PathBuf>>> {
        self.account()?.image_hardlink_index()
    }

    /// 微信的媒体索引中 md5 为 `md5` 的消息的视频所在的位置（见 `Account::video_hardlinks`）。
    pub fn video_hardlinks(&self, md5: &str) -> Result<Vec<PathBuf>> {
        self.account()?.video_hardlinks(md5)
    }

    /// 微信的媒体索引中 md5 为 `md5` 的消息的文件所在的位置（见 `Account::file_hardlinks`）。
    pub fn file_hardlinks(&self, md5: &str) -> Result<Vec<PathBuf>> {
        self.account()?.file_hardlinks(md5)
    }

    /// md5 为 `md5` 的表情的下载地址（见 [`Account::emoticon_cdn_url`]）。
    pub fn emoticon_cdn_url(&self, md5: &str) -> Result<String> {
        self.account()?.emoticon_cdn_url(md5)
    }

    /// 所有表情的说明文字，以小写的 md5 为键（见 `Account::emoticon_captions`）。
    pub fn emoticon_captions(&self) -> Result<std::collections::HashMap<String, String>> {
        self.account()?.emoticon_captions()
    }

    /// 语音消息的音频，写成小写十六进制，没有时为 `""`（见 [`Account::voice_data`]）。
    pub fn voice_data(
        &self,
        create_time: i32,
        local_id: i32,
        svr_id: i64,
        candidates: &[impl AsRef<str>],
    ) -> Result<String> {
        self.account()?.voice_data(
            i64::from(create_time),
            i64::from(local_id),
            svr_id,
            &distinct_usernames(candidates),
        )
    }

    /// 数组 `requests` 请求的每条语音消息的音频，
    /// 按请求的顺序写成 `[{ "hex": … }]`（见 [`Account::voice_data_batch`]）。
    pub fn voice_data_batch(&self, requests: &Value) -> Result<Value> {
        self.account()?.voice_data_batch(requests)
    }

    /// 在 `options` 给出的范围内，每个会话按类型统计的消息数，以及第一条和最后一条消息的时间（见
    /// [`Account::session_message_type_stats_batch`]）。
    pub fn session_message_type_stats_batch(
        &self,
        session_ids: &[impl AsRef<str>],
        options: &Value,
    ) -> Result<Value> {
        self.account()?
            .session_message_type_stats_batch(&distinct_usernames(session_ids), options)
    }
}

/// `root` 下的 `session/session.db`，没有时为它下面第一个 `session.db`（不区分大小写）。
/// 搜索时跳过无法列出的目录，不进入指向目录的链接（符号链接、目录联接），所以指回上层的链接不会让它绕圈。
/// 每个目录中的条目按名字排序，同一棵目录树总是得到同一个文件。
fn find_session_db(root: &Path) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    let direct = root.join("session/session.db");
    if direct.exists() {
        return Some(direct);
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<fs::DirEntry> = entries.flatten().collect();
        // 栈后进先出：子目录按名字倒序入栈，名字排在最前的最先搜索
        entries.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for entry in entries {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_dir() {
                stack.push(entry.path());
            } else if entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.eq_ignore_ascii_case("session.db"))
            {
                return Some(entry.path());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::TestDir;

    #[test]
    fn session_db_is_found_below_the_root() {
        let root = TestDir::new("weflow-find-session");
        assert_eq!(find_session_db(&root.join("missing")), None);
        fs::create_dir_all(root.join("a/b")).unwrap();
        fs::create_dir_all(root.join("c")).unwrap();
        assert_eq!(find_session_db(root.path()), None);
        fs::write(root.join("a/b/Session.db"), b"").unwrap();
        fs::write(root.join("c/session.db"), b"").unwrap();
        assert_eq!(
            find_session_db(root.path()),
            Some(root.join("a/b/Session.db")),
            "the first by name"
        );
        fs::create_dir_all(root.join("session")).unwrap();
        fs::write(root.join("session/session.db"), b"").unwrap();
        assert_eq!(
            find_session_db(root.path()),
            Some(root.join("session/session.db"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_search_passes_folders_it_cannot_list_and_does_not_follow_links() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let root = TestDir::new("weflow-find-session-links");
        let outside = TestDir::new("weflow-find-session-outside");
        fs::write(outside.join("session.db"), b"").unwrap();
        fs::create_dir_all(root.join("a")).unwrap();
        // 一个指回上层的链接，一个指向树外目录的链接
        symlink(root.path(), root.join("a/up")).unwrap();
        symlink(outside.path(), root.join("a/out")).unwrap();
        assert_eq!(find_session_db(root.path()), None);

        // 按名字最先搜索，但无法列出
        let locked = root.join("a/locked");
        fs::create_dir_all(&locked).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("b/session.db"), b"").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let unlistable = fs::read_dir(&locked).is_err(); // root 用户照样能列出
        let found = find_session_db(root.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if unlistable {
            assert_eq!(found, Some(root.join("b/session.db")));
        }
    }
}
