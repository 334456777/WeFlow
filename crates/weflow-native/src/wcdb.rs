//! Database facade used by the service layer.
//!
//! Everything is served by the pure-Rust backend in [`crate::db`]/[`crate::msg`]; there is
//! no dependency on the closed-source `wcdb_api` library.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use serde_json::Value;

use crate::contact::distinct_usernames;
use crate::db::Account;
use crate::msg::Order;
use crate::msg_row::{Columns, MsgRow};

/// A group of an account's databases, for [`Wcdb::prefetch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Databases {
    /// The conversation list, `session/session.db`.
    Sessions,
    /// The contacts, `contact/contact.db`.
    Contacts,
    /// Every message database (`message/message_<n>.db`, `biz_message_<n>.db`).
    Messages,
    /// The Moments, `sns/sns.db`.
    Moments,
}

/// The handle of one account's databases that the service layer calls; closed until [`Wcdb::open`].
#[derive(Debug, Default)]
pub struct Wcdb {
    native: Option<Account>,
}

impl Wcdb {
    /// A closed handle.
    pub fn new() -> Self {
        Self::default()
    }

    fn account(&self) -> Result<&Account> {
        self.native
            .as_ref()
            .ok_or_else(|| anyhow!("WCDB is not connected"))
    }

    /// Open the account: decrypt `session.db` with the key to prove the key and path are right.
    pub fn open(&mut self, account_dir: &Path, hex_key: &str, wxid: Option<&str>) -> Result<()> {
        self.open_unchecked(account_dir, hex_key, wxid)?;
        let tested = self.account()?.test_connection();
        if tested.is_err() {
            self.native = None; // a handle whose key does not work stays closed
        }
        tested
    }

    /// [`open`](Self::open) without proving the key up front: each database checks the key when it is first
    /// read, so a command that never reads `session.db` skips its (slow) key derivation. A wrong key still fails,
    /// at the first read, with the same "cannot decrypt ... with the configured database key" error.
    pub fn open_unchecked(
        &mut self,
        account_dir: &Path,
        hex_key: &str,
        wxid: Option<&str>,
    ) -> Result<()> {
        self.native = None;
        let session_db = find_session_db(&account_dir.join("db_storage"))
            .ok_or_else(|| anyhow!("session.db not found under {}", account_dir.display()))?;
        // session.db lives in <db_storage>/session/, so its grandparent is db_storage.
        let db_storage = session_db
            .parent()
            .and_then(Path::parent)
            .unwrap_or(account_dir);
        // Without an explicit wxid, the account directory name (`<wxid>_<4 chars>`) identifies the owner.
        let me = wxid.map(str::to_string).or_else(|| {
            account_dir
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
        });
        self.native = Some(Account::new(db_storage, hex_key)?.with_my_wxid(me));
        Ok(())
    }

    /// Prove the key of an account opened with [`open_unchecked`](Self::open_unchecked) (decrypts `session.db`).
    pub fn check_key(&self) -> Result<()> {
        self.account()?.test_connection()
    }

    /// The key-derivation salt of `session.db` (its first 16 bytes). With the key it identifies a successful
    /// [`check_key`](Self::check_key): a different key, or a replaced database, gives a different pair.
    pub fn session_salt(&self) -> Option<[u8; 16]> {
        use std::io::Read;
        let mut salt = [0u8; 16];
        fs::File::open(self.account().ok()?.session_db())
            .ok()?
            .read_exact(&mut salt)
            .ok()?;
        Some(salt)
    }

    /// Whether the account at `account_dir` opens with `hex_key`: [`open`](Self::open), then [`close`](Self::close).
    ///
    /// # Errors
    /// As [`open`](Self::open).
    pub fn test_connection(&mut self, account_dir: &Path, hex_key: &str) -> Result<()> {
        self.open(account_dir, hex_key, None)?;
        self.close();
        Ok(())
    }

    /// Closes the handle; later calls fail with "WCDB is not connected" until it is opened again.
    pub fn close(&mut self) {
        self.native = None;
    }

    // ── sessions and messages ──

    /// The rows of `SessionTable`, newest first (see [`Account::sessions`]).
    pub fn sessions(&self) -> Result<Value> {
        self.account()?.sessions()
    }

    /// A newest-first page of a conversation's messages across the message databases (see [`Account::messages`]).
    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> Result<Vec<MsgRow>> {
        self.account()?.messages(session_id, limit, offset)
    }

    /// See [`Account::messages_from`](crate::db::Account::messages_from).
    pub fn messages_from(
        &self,
        session_id: &str,
        limit: i32,
        from: crate::msg::PageFrom,
    ) -> Result<Vec<MsgRow>> {
        self.account()?.messages_from(session_id, limit, from)
    }

    /// See [`Account::messages_where`].
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

    /// See [`Account::message_tables_by_name`].
    pub fn message_tables_by_name(
        &self,
    ) -> Result<std::collections::HashMap<String, Vec<crate::msg::MsgTable>>> {
        self.account()?.message_tables_by_name()
    }

    /// See [`Account::typed_before`].
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

    /// Opens the databases a chat page reads, at the same time (see
    /// [`Account::warm`](crate::db::Account::warm)); returns how many opened.
    pub fn warm_chat_databases(&self) -> Result<usize> {
        let account = self.account()?;
        Ok(account.warm(&account.chat_databases()))
    }

    /// Opens the databases of `groups` side by side, ahead of a read that goes through them one after another: the
    /// first open of each derives its key (PBKDF2, about 130 ms that one thread does alone), so opened as the read
    /// reaches each they add up, and opened here they overlap (#1100). Those already open cost nothing (see
    /// [`Account::prefetch`](crate::db::Account::prefetch)).
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

    /// How many messages a conversation has (see [`Account::message_count`]).
    pub fn message_count(&self, session_id: &str) -> Result<i32> {
        self.account()?.message_count(session_id)
    }

    /// The days a conversation has messages on, `YYYY-MM-DD` in local time, oldest first (see
    /// [`Account::message_dates`]).
    pub fn message_dates(&self, session_id: &str) -> Result<Value> {
        self.account()?.message_dates(session_id)
    }

    /// `{ username: count }`: how many messages each of `session_ids` has (see
    /// [`Account::session_message_counts`]).
    pub fn session_message_counts(&self, session_ids: &[String]) -> Result<Value> {
        self.account()?.session_message_counts(session_ids)
    }

    /// `{ "YYYY-MM-DD": count }`: a conversation's messages by local day (see
    /// [`Account::session_message_date_counts`]).
    pub fn session_message_date_counts(&self, session_id: &str) -> Result<Value> {
        self.account()?.session_message_date_counts(session_id)
    }

    /// Every message of one `local_type` (see [`Account::messages_by_type`]).
    pub fn messages_by_type(
        &self,
        session_id: &str,
        local_type: i64,
        order: Order,
    ) -> Result<Vec<MsgRow>> {
        self.account()?
            .messages_by_type(session_id, local_type, order)
    }

    /// Every message with this `local_id` (one per message database at most), optionally sent at `create_time`.
    pub fn messages_by_local_id(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: Option<i64>,
    ) -> Result<Vec<MsgRow>> {
        self.account()?
            .messages_by_local_id(session_id, local_id, create_time)
    }

    /// The message of a conversation with the server id `svrid`, a decimal string that may pass 2^53 (see
    /// [`Account::message_by_server_id`]).
    pub fn message_by_server_id(&self, session_id: &str, svrid: &str) -> Result<Option<MsgRow>> {
        self.account()?.message_by_server_id(session_id, svrid)
    }

    /// Opens a cursor over a conversation's messages between `begin` and `end` (seconds; 0 is unbounded) and returns
    /// its id (see [`Account::open_message_cursor`]).
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

    /// The cursor of an export (see [`Account::open_export_cursor`]).
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

    /// A cursor over one member's messages only (see `Account::open_message_cursor_for_senders`).
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

    /// Returns the next batch of rows and whether more remain.
    pub fn fetch_message_batch(&self, cursor: i64) -> Result<(Vec<MsgRow>, bool)> {
        self.account()?.fetch_message_batch(cursor)
    }

    /// (messages, batch size) of a cursor (see [`Account::message_cursor_len`]).
    pub fn message_cursor_len(&self, cursor: i64) -> Result<(usize, usize)> {
        self.account()?.message_cursor_len(cursor)
    }

    /// Batch number `page` of a cursor, without moving it (see [`Account::fetch_message_page`]).
    pub fn fetch_message_page(&self, cursor: i64, page: usize) -> Result<Vec<MsgRow>> {
        self.account()?.fetch_message_page(cursor, page)
    }

    /// Closes a cursor and frees what it holds (see [`Account::close_message_cursor`]).
    pub fn close_message_cursor(&self, cursor: i64) -> Result<()> {
        self.account()?.close_message_cursor(cursor)
    }

    /// Connections a database may open for this handle's queries (see [`Account::set_read_connections`]).
    pub fn set_read_connections(&self, n: usize) {
        if let Some(a) = &self.native {
            a.set_read_connections(n);
        }
    }

    /// Queries run again on a fresh snapshot so far (see [`Account::stale_reruns`]).
    pub fn stale_reruns(&self) -> usize {
        self.native.as_ref().map_or(0, Account::stale_reruns)
    }

    // ── statistics of conversations ──

    /// The compact contact rows of `usernames`: names, alias, avatar and type (see [`Account::contacts_compact`]).
    pub fn contacts_compact(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .contacts_compact(&distinct_usernames(usernames))
    }

    /// The messages of `session_ids` between `begin` and `end`, by direction, type and time (see
    /// [`Account::aggregate_stats`]).
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

    /// The totals the annual report starts from, of `session_ids` between `begin` and `end` (see
    /// [`Account::annual_report_stats`]).
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

    /// The first and last message time of the table `table` of the database `db` (see
    /// [`Account::message_table_time_range`]).
    pub fn message_table_time_range(&self, db: &str, table: &str) -> Result<Value> {
        self.account()?.message_table_time_range(db, table)
    }

    // ── search, contacts, groups, statistics, Moments and media ──

    /// The messages whose text holds `keyword`, newest first, each hit with its conversation (see
    /// [`Account::search_messages`]).
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

    /// One contact's row, `{}` when it is unknown (see [`Account::contact`]).
    pub fn contact(&self, username: &str) -> Result<Value> {
        self.account()?.contact(username)
    }

    /// Every row of `contact` (see [`Account::contacts`]).
    pub fn contacts(&self) -> Result<Value> {
        self.account()?.contacts()
    }

    /// `{ "private", "group", "official", "former_friend" }`: how many contacts of each kind (see
    /// [`Account::contact_type_counts`]).
    pub fn contact_type_counts(&self) -> Result<Value> {
        self.account()?.contact_type_counts()
    }

    /// How many members a group has (see [`Account::group_member_count`]).
    pub fn group_member_count(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_member_count(chatroom_id)
    }

    /// A group's members as WeFlow's desktop app lists them (see [`Account::group_members`]).
    pub fn group_members(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_members(chatroom_id)
    }

    /// `{ username: group nickname }` of a group's members (see [`Account::group_nicknames`]).
    pub fn group_nicknames(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.group_nicknames(chatroom_id)
    }

    /// Messages, sent and received, and the last time, of each of `session_ids` (see
    /// [`Account::session_totals`]).
    pub fn session_totals(&self, session_ids: &[String], begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .session_totals(session_ids, i64::from(begin), i64::from(end))
    }

    /// A group's statistics between `begin` and `end`: messages by sender, by hour and by type (see
    /// [`Account::group_stats`]).
    pub fn group_stats(&self, chatroom_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .group_stats(chatroom_id, i64::from(begin), i64::from(end))
    }

    /// The local years, newest first, in which any of `session_ids` has messages (see
    /// [`Account::available_years`]).
    pub fn available_years(&self, session_ids: &[String]) -> Result<Value> {
        self.account()?.available_years(session_ids)
    }

    /// The dual report's statistics of the chat with `session_id` between `begin` and `end` (see
    /// [`Account::dual_report_stats`]).
    pub fn dual_report_stats(&self, session_id: &str, begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .dual_report_stats(session_id, i64::from(begin), i64::from(end))
    }

    /// The footprint over the range and conversations `options` gives (see [`Account::footprint_stats`]).
    pub fn footprint_stats(&self, options: &Value) -> Result<Value> {
        self.account()?.footprint_stats(options)
    }

    /// Newest-first page of Moments posts, of the posters `usernames` (all when empty).
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

    /// The owner's Moments between `begin` and `end`: posts and their types, who liked them most and whose posts the
    /// owner liked most (see [`Account::sns_annual_stats`]).
    pub fn sns_annual_stats(&self, begin: i32, end: i32) -> Result<Value> {
        self.account()?
            .sns_annual_stats(i64::from(begin), i64::from(end))
    }

    /// Who has Moments posts, the one who posted last first (see [`Account::sns_usernames`]).
    pub fn sns_usernames(&self) -> Result<Value> {
        self.account()?.sns_usernames()
    }

    /// `{ username: posts }` of every poster (see [`Account::sns_post_counts`]).
    pub fn sns_post_counts(&self) -> Result<Value> {
        self.account()?.sns_post_counts()
    }

    /// `{ total_posts, total_friends, my_posts }` of the Moments (see [`Account::sns_export_stats`]).
    pub fn sns_export_stats(&self, my_wxid: Option<&str>) -> Result<Value> {
        self.account()?.sns_export_stats(my_wxid)
    }

    /// The rows of a read-only statement on one database, as JSON objects (see [`Account::exec_query`]).
    pub fn exec_query(&self, kind: &str, path: &str, sql: &str) -> Result<Value> {
        self.account()?.exec_query(kind, Some(path), sql)
    }

    /// Image/video messages of the conversations `session_ids`, newest first, and whether more remain.
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

    /// `{ username: name }` of `usernames`: the remark, else the nickname, else the alias, else the username (see
    /// [`Account::display_names`]).
    pub fn display_names(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .display_names(&distinct_usernames(usernames))
    }

    /// The avatar kept on this computer for `username` (see `Account::head_image`).
    pub fn head_image(&self, username: &str) -> Result<Option<Vec<u8>>> {
        self.account()?.head_image(username)
    }

    /// `{ username: url }` of the contacts of `usernames` that have an avatar (see [`Account::avatar_urls`]).
    pub fn avatar_urls(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?.avatar_urls(&distinct_usernames(usernames))
    }

    /// `{ chatroom: count }` of the groups `chatroom_ids` (see [`Account::group_member_counts`]).
    pub fn group_member_counts(&self, chatroom_ids: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .group_member_counts(&distinct_usernames(chatroom_ids))
    }

    /// Whether each conversation of `usernames` is folded, muted and pinned (see [`Account::contact_status`]).
    pub fn contact_status(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .contact_status(&distinct_usernames(usernames))
    }

    /// `{ username: alias }` of the contacts of `usernames` with a WeChat ID (see [`Account::contact_alias_map`]).
    pub fn contact_alias_map(&self, usernames: &[impl AsRef<str>]) -> Result<Value> {
        self.account()?
            .contact_alias_map(&distinct_usernames(usernames))
    }

    /// `{ "ext_buffer": "<hex>" }` of a group, `{}` when it is unknown (see [`Account::chat_room_ext_buffer`]).
    pub fn chat_room_ext_buffer(&self, chatroom_id: &str) -> Result<Value> {
        self.account()?.chat_room_ext_buffer(chatroom_id)
    }

    /// One row for each table of a conversation: its database, its name, its messages and their first and last time
    /// (see [`Account::message_table_stats`]).
    pub fn message_table_stats(&self, session_id: &str) -> Result<Value> {
        self.account()?.message_table_stats(session_id)
    }

    /// [`message_table_stats`](Self::message_table_stats) of each of several conversations, in their order, their
    /// tables found in one listing of each database (see [`Account::message_table_stats_of`]).
    pub fn message_table_stats_of(&self, session_ids: &[String]) -> Result<Vec<Result<Value>>> {
        self.account()?.message_table_stats_of(session_ids)
    }

    /// What the annual report reads besides the totals: the heatmap, the midnight messages, who opened the
    /// conversations, my replies, the peak day, my phrases and the longest streak (see
    /// [`Account::annual_report_extras`]).
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

    /// Where WeChat's media index puts the images, by message md5 (see `Account::image_hardlink_index`).
    pub fn image_hardlink_index(&self) -> Result<std::collections::HashMap<String, Vec<PathBuf>>> {
        self.account()?.image_hardlink_index()
    }

    /// Where WeChat's media index puts the video of a message md5 (see `Account::video_hardlinks`).
    pub fn video_hardlinks(&self, md5: &str) -> Result<Vec<PathBuf>> {
        self.account()?.video_hardlinks(md5)
    }

    /// Where WeChat's media index puts the file of a message md5 (see `Account::file_hardlinks`).
    pub fn file_hardlinks(&self, md5: &str) -> Result<Vec<PathBuf>> {
        self.account()?.file_hardlinks(md5)
    }

    /// The download address of the sticker `md5` (see [`Account::emoticon_cdn_url`]).
    pub fn emoticon_cdn_url(&self, md5: &str) -> Result<String> {
        self.account()?.emoticon_cdn_url(md5)
    }

    /// Every sticker caption, keyed by lower-case md5 (see `Account::emoticon_captions`).
    pub fn emoticon_captions(&self) -> Result<std::collections::HashMap<String, String>> {
        self.account()?.emoticon_captions()
    }

    /// A voice message's audio as lowercase hex, `""` when there is none (see [`Account::voice_data`]).
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

    /// The audio of each voice message the array `requests` asks for, `[{ "hex": … }]` in its order (see
    /// [`Account::voice_data_batch`]).
    pub fn voice_data_batch(&self, requests: &Value) -> Result<Value> {
        self.account()?.voice_data_batch(requests)
    }

    /// Each conversation's messages by type, and their first and last time, within the range of `options` (see
    /// [`Account::session_message_type_stats_batch`]).
    pub fn session_message_type_stats_batch(
        &self,
        session_ids: &[impl AsRef<str>],
        options: &Value,
    ) -> Result<Value> {
        self.account()?
            .session_message_type_stats_batch(&distinct_usernames(session_ids), options)
    }
}

/// `session/session.db` of `root`, else the first `session.db` (any case) under it. The search goes on past a folder
/// it cannot list, and does not enter links to folders (symbolic links, junctions), so a link back up the tree cannot
/// send it round in circles. Each folder's entries are taken in name order, so the same tree gives the same file.
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
        // popped last first: the subfolders in reverse name order, so the first by name is searched first
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
        // a link back up the tree, and one to a folder outside it
        symlink(root.path(), root.join("a/up")).unwrap();
        symlink(outside.path(), root.join("a/out")).unwrap();
        assert_eq!(find_session_db(root.path()), None);

        // searched first by name, but it cannot be listed
        let locked = root.join("a/locked");
        fs::create_dir_all(&locked).unwrap();
        fs::create_dir_all(root.join("b")).unwrap();
        fs::write(root.join("b/session.db"), b"").unwrap();
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
        let unlistable = fs::read_dir(&locked).is_err(); // root lists it anyway
        let found = find_session_db(root.path());
        fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
        if unlistable {
            assert_eq!(found, Some(root.join("b/session.db")));
        }
    }
}
