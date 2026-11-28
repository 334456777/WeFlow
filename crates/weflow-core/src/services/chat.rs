//! 基本读取以外的聊天查询：按 ID 查消息、按天统计、会话详情、导出统计、资源列表和保存的群提示。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use serde_json::{json, Map, Value};

use super::session_kind::may_be_friend;
use super::{clean_account_dir_name, ServiceHub};
use crate::chat_msg::{self, ChatMessage};
use crate::contract::{
    ChatImages, ChatStatus, ExportStats, GroupHint, ImageRef, MediaStream, MediaStreamItem,
    MessageLookup, MessageTable, MessageView, Resource, ResourceKind, ResourcePage,
    SessionDateCounts, SessionDates, SessionDetail, SessionExportStats, SessionFacts,
    SessionTables, TabCounts, TransferNames, VoiceMessages,
};
use crate::error::{AppError, AppResult};
use crate::message::appmsg::app_message_kind;
use crate::message::{collect_rows, CollectOptions, ExportMsg};
use weflow_native::media::SessionRow;
use weflow_native::msg::{table_name_for, Order};
use weflow_native::msg_row::{Columns, MsgRow};
use weflow_native::wcdb::Databases;

/// 文件消息保存时用的 `local_type`：`<type>` 为 6 的 app 消息，保存为 49 或 `(6 << 32) | 49`（#1263）。
/// 保存为 `(8 << 32) | 49` 的表情和 `(24 << 32) | 49` 的笔记不是文件。
pub(super) const FILE_APP_TYPES: [i64; 2] = [49, (6 << 32) | 49];

fn native(e: anyhow::Error) -> AppError {
    AppError::from_native(e)
}

/// `local_id` 指的消息，没有时为 `None`。
///
/// 本地 ID 只在一个消息库内唯一，所以多个消息库都有它时，先按 `create_time`（秒）、
/// 再按 `local_type`（调用方要的消息类型，如图片为 3）选出一条；仍有多条时，返回的错误中列出它们。
pub(crate) fn message_by_local_id(
    wcdb: &weflow_native::wcdb::Wcdb,
    session_id: &str,
    local_id: i64,
    create_time: Option<i64>,
    local_type: Option<i64>,
) -> AppResult<Option<MsgRow>> {
    let create_time = create_time.filter(|t| *t > 0);
    let mut rows = wcdb
        .messages_by_local_id(session_id, local_id, create_time)
        .map_err(native)?;
    if rows.len() > 1 {
        if let Some(want) = local_type {
            let of_type = |r: &MsgRow| r.local_type & 0xFFFF_FFFF == want;
            if rows.iter().any(of_type) {
                rows.retain(of_type);
            }
        }
    }
    if rows.len() > 1 {
        let candidates: Vec<Value> = rows
            .iter()
            .map(|r| {
                json!({
                    "createTime": r.create_time.unwrap_or(0),
                    "localType": r.local_type,
                    "dbName": weflow_native::msg::shard_name(r.shard),
                })
            })
            .collect();
        return Err(AppError::usage(format!(
            "local id {local_id} names {} messages in different message databases\nPick one with --create-time; details.candidates lists them.",
            rows.len()
        ))
        .with_details(json!({ "candidates": candidates })));
    }
    Ok(rows.pop())
}

fn uniq(ids: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for id in ids {
        let t = id.trim();
        if !t.is_empty() && !out.iter().any(|o| o == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// 查找一个 ID 时用的小写形式：ID 本身，以及去掉账号目录后缀的 ID（`clean_account_dir_name`）。
///
/// [`is_same_wxid`](crate::message::transfer::is_same_wxid) 认为是同一个账号的两个 ID 有共同的键：
/// `john` 和 `john_1a2b` 有，`john` 和 `john_smith` 没有。
fn identity_keys(raw: &str) -> Vec<String> {
    let value = raw.trim();
    if value.is_empty() {
        return Vec::new();
    }
    let lower = value.to_lowercase();
    let cleaned = clean_account_dir_name(value).to_lowercase();
    if !cleaned.is_empty() && cleaned != lower {
        vec![cleaned, lower]
    } else {
        vec![lower]
    }
}

fn member_username(member: &Value) -> String {
    if let Some(s) = member.as_str() {
        return s.trim().to_string();
    }
    [
        "username",
        "userName",
        "user_name",
        "encryptUsername",
        "encryptUserName",
        "encrypt_username",
        "originalName",
    ]
    .iter()
    .find_map(|k| {
        member
            .get(*k)
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
    })
    .unwrap_or("")
    .trim()
    .to_string()
}

fn member_list(value: &Value) -> Vec<Value> {
    value
        .as_array()
        .cloned()
        .or_else(|| value.get("members").and_then(Value::as_array).cloned())
        .unwrap_or_default()
}

fn num(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|x| {
            x.as_i64()
                .or_else(|| x.as_f64().map(crate::num::saturating_i64))
                .or_else(|| x.as_str().and_then(|s| s.parse().ok()))
        })
        .unwrap_or(0)
        .max(0)
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

/// `chat resources` 列出哪些消息。
#[derive(Clone, Debug, Default)]
pub struct ResourceQuery {
    /// 会话；为 `None` 时列出所有会话的。
    pub session_id: Option<String>,
    /// `image`、`video`、`voice`、`file` 中的几种；为空时全部列出。
    pub types: Vec<String>,
    /// Unix 秒：只列出从这时起的消息；0 表示不限。
    pub begin: i64,
    /// Unix 秒：只列出到这时为止的消息；0 表示不限。
    pub end: i64,
    /// 这一页最多的消息数。
    pub limit: usize,
    /// 上一页的 `nextCursor`。
    pub before: Option<String>,
}

/// `chat resources --type` 接受的消息种类。
const RESOURCE_TYPES: [&str; 4] = ["image", "video", "voice", "file"];

impl ServiceHub {
    fn normalize_row(&self, rows: Vec<MsgRow>, session_id: &str) -> Vec<ExportMsg> {
        let my = self.my_wxid_cleaned();
        collect_rows(
            rows,
            &CollectOptions {
                session_id,
                my_wxid: &my,
                start: None,
                end: None,
                sender_filter: None,
            },
        )
    }

    // ── 单条消息 ──

    /// `chat message --local-id`（#492：返回类型化的行）。
    ///
    /// # Errors
    ///
    /// 找不到消息时返回 `not_found`；本地 ID 对应不同消息库中的多条消息时返回用法错误（`details.candidates`
    /// 列出它们）；账号没有配置好时返回配置错误。
    pub fn chat_message_by_id(
        &self,
        session_id: &str,
        local_id: i64,
        create_time: Option<i64>,
    ) -> AppResult<MsgRow> {
        let wcdb = self.open_wcdb()?;
        message_by_local_id(&wcdb, session_id, local_id, create_time, None)?
            .ok_or_else(|| AppError::not_found("message not found"))
    }

    /// `chat message --server-id`。
    ///
    /// # Errors
    /// `server_id` 不是十进制数时返回用法错误；没有消息有这个 ID 时返回 `not_found`。
    pub fn chat_message_by_server_id(
        &self,
        session_id: &str,
        server_id: &str,
    ) -> AppResult<MsgRow> {
        // 数据库层按数字比较 ID：不是数字就是调用方写错了
        if server_id.trim().parse::<i64>().is_err() {
            return Err(AppError::usage(format!("invalid server id: {server_id}")));
        }
        self.open_wcdb()?
            .message_by_server_id(session_id, server_id)
            .map_err(native)?
            .ok_or_else(|| AppError::not_found("message not found"))
    }

    /// 不带 `parsed` 的 `chat message`：数据库的行，以及查询命令列出时的消息。
    pub fn message_lookup(&self, row: MsgRow, session_id: &str) -> MessageLookup {
        let my = self.my_wxid_cleaned();
        let message = collect_rows(
            vec![row.clone()],
            &CollectOptions {
                session_id,
                my_wxid: &my,
                start: None,
                end: None,
                sender_filter: None,
            },
        )
        .pop()
        .map(|m| MessageView::of(&m, &my));
        MessageLookup {
            row: row.into(),
            message,
        }
    }

    // ── 按天统计 ──

    /// 会话有消息的日期（本地时间），从早到晚（`chat dates`）；读不出时为空。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub fn chat_dates(&self, session_id: &str) -> AppResult<SessionDates> {
        let wcdb = self.open_wcdb()?;
        let dates = wcdb
            .message_dates(session_id)
            .ok()
            .and_then(|d| serde_json::from_value(d).ok())
            .unwrap_or_default();
        Ok(SessionDates {
            session_id: session_id.to_string(),
            dates,
        })
    }

    /// 会话每天（本地时间）的消息数（`chat date-counts`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_date_counts(&self, session_id: &str) -> AppResult<SessionDateCounts> {
        let wcdb = self.open_wcdb()?;
        let raw = wcdb
            .session_message_date_counts(session_id)
            .map_err(native)?;
        let mut counts = BTreeMap::new();
        if let Some(obj) = raw.as_object() {
            for (date, value) in obj {
                let n = value
                    .as_f64()
                    .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
                    .unwrap_or(0.0);
                if !date.is_empty() && n.is_finite() && n > 0.0 {
                    counts.insert(date.clone(), crate::num::saturating_i64(n.floor()));
                }
            }
        }
        Ok(SessionDateCounts {
            session_id: session_id.to_string(),
            counts,
        })
    }

    /// 请求的每个会话的消息数，按会话 ID 列出，顺序与请求的一致。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_counts(&self, session_ids: &[String]) -> AppResult<IndexMap<String, i64>> {
        let ids = uniq(session_ids);
        if ids.is_empty() {
            return Ok(IndexMap::new());
        }
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.session_message_counts(&ids).map_err(native)?;
        Ok(ids
            .into_iter()
            .map(|id| {
                let n = num(&raw, &id);
                (id, n)
            })
            .collect())
    }

    /// `usernames` 中每一个是否折叠、免打扰和置顶（`chat statuses`），每个只列一次，顺序与给出的一致。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_statuses(&self, usernames: &[String]) -> AppResult<IndexMap<String, ChatStatus>> {
        let ids = uniq(usernames);
        if ids.is_empty() {
            return Ok(IndexMap::new());
        }
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.contact_status(&ids).map_err(native)?;
        Ok(ids
            .into_iter()
            .map(|id| {
                let flag = |k: &str| {
                    raw.get(&id)
                        .and_then(|s| s.get(k))
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                };
                let status = ChatStatus {
                    is_folded: flag("isFolded"),
                    is_muted: flag("isMuted"),
                    is_pinned: flag("isPinned"),
                };
                (id, status)
            })
            .collect())
    }

    // ── 会话详情 ──

    /// 会话的对方是谁、有多少条消息（`chat detail --fast`）。
    ///
    /// # Errors
    /// ID 为空时返回用法错误；账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub fn chat_detail_fast(&self, session_id: &str) -> AppResult<SessionFacts> {
        let id = session_id.trim();
        if id.is_empty() {
            return Err(AppError::usage("session id cannot be empty"));
        }
        let wcdb = self.open_wcdb()?;
        let contact = wcdb
            .contact(id)
            .ok()
            .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()));
        let field = |keys: &[&str]| {
            contact
                .as_ref()
                .and_then(|c| {
                    keys.iter()
                        .find_map(|k| c.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
                })
                .map(str::to_string)
        };
        let remark = field(&["remark"]);
        let nick = field(&["nickName", "nick_name", "nickname"]);
        let alias = field(&["alias"]);
        let display = remark
            .clone()
            .or(nick.clone())
            .or(alias.clone())
            .unwrap_or_else(|| id.to_string());
        let avatar = wcdb
            .avatar_urls(&[id])
            .ok()
            .and_then(|m| m.get(id).and_then(Value::as_str).map(str::to_string))
            .filter(|u| u.starts_with("http") || u.starts_with("data:"));
        let count = wcdb.message_count(id).map(|c| c.max(0)).unwrap_or(0);
        Ok(SessionFacts {
            wxid: id.to_string(),
            display_name: display,
            remark,
            nick_name: nick,
            alias,
            avatar_url: avatar,
            message_count: count.into(),
        })
    }

    /// 会话的消息存放在哪些表里，以及最早和最晚的时间（`chat detail --extra`）。
    ///
    /// # Errors
    /// ID 为空时返回用法错误；账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub fn chat_detail_extra(&self, session_id: &str) -> AppResult<SessionTables> {
        let id = session_id.trim();
        if id.is_empty() {
            return Err(AppError::usage("session id cannot be empty"));
        }
        let wcdb = self.open_wcdb()?;
        let stats = wcdb.message_table_stats(id).unwrap_or_else(|_| json!([]));
        let mut tables = Vec::new();
        let mut first: Option<i64> = None;
        let mut last: Option<i64> = None;
        for row in stats.as_array().into_iter().flatten() {
            let db_path = row.get("db_path").and_then(Value::as_str).unwrap_or("");
            tables.push(MessageTable {
                db_name: basename(db_path),
                table_name: row
                    .get("table_name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                count: num(row, "count"),
            });
            let pick = |keys: &[&str]| {
                keys.iter()
                    .map(|k| num(row, k))
                    .find(|v| *v > 0)
                    .unwrap_or(0)
            };
            let f = pick(&[
                "first_timestamp",
                "firstTimestamp",
                "first_time",
                "firstTime",
                "min_create_time",
                "minCreateTime",
            ]);
            if f > 0 && first.is_none_or(|x| f < x) {
                first = Some(f);
            }
            let l = pick(&[
                "last_timestamp",
                "lastTimestamp",
                "last_time",
                "lastTime",
                "max_create_time",
                "maxCreateTime",
            ]);
            if l > 0 && last.is_none_or(|x| l > x) {
                last = Some(l);
            }
        }
        Ok(SessionTables {
            first_message_time: first,
            latest_message_time: last,
            message_tables: tables,
        })
    }

    /// [`Self::chat_detail_fast`] 和 [`Self::chat_detail_extra`] 合在一起（`chat detail`）；读不出表时表的部分为空。
    ///
    /// # Errors
    ///
    /// 同 [`Self::chat_detail_fast`]。
    pub fn chat_detail(&self, session_id: &str) -> AppResult<SessionDetail> {
        Ok(SessionDetail {
            facts: self.chat_detail_fast(session_id)?,
            tables: self.chat_detail_extra(session_id).unwrap_or_default(),
        })
    }

    // ── 导出统计 ──

    /// 每类联系人各有多少（`chat tab-counts`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误；读取数据库失败或回答不符合契约类型时返回数据库层的错误。
    pub fn chat_tab_counts(&self) -> AppResult<TabCounts> {
        serde_json::from_value(self.contact_type_counts()?)
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    fn friend_identities(&self, wcdb: &weflow_native::wcdb::Wcdb) -> HashSet<String> {
        let mut set = HashSet::new();
        let contacts = wcdb.contacts().unwrap_or(Value::Null);
        for row in contacts.as_array().into_iter().flatten() {
            let username = row
                .get("username")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            if !may_be_friend(username) {
                continue;
            }
            // 取这一行有的列中的第一个
            let local_type = ["local_type", "localType", "WCDB_CT_local_type"]
                .iter()
                .find_map(|k| {
                    row.get(*k).map(|v| {
                        v.as_i64()
                            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                            .unwrap_or(0)
                    })
                })
                .unwrap_or(0);
            if local_type != 1 {
                continue;
            }
            for key in identity_keys(username) {
                set.insert(key);
            }
        }
        set
    }

    fn group_session_ids(&self, wcdb: &weflow_native::wcdb::Wcdb) -> Vec<String> {
        let sessions = wcdb.sessions().unwrap_or(Value::Null);
        let mut out: Vec<String> = Vec::new();
        for row in sessions.as_array().into_iter().flatten() {
            let name = ["username", "userName", "talker", "sessionId"]
                .iter()
                .find_map(|k| row.get(*k).and_then(Value::as_str))
                .unwrap_or("")
                .trim();
            if name.ends_with("@chatroom") && !out.iter().any(|o| o == name) {
                out.push(name.to_string());
            }
        }
        out
    }

    /// `session_ids` 中各会话的导出统计：各类型的消息数，群的成员数、发言人数和自己的消息数；
    /// `include_relations` 时还有群和私聊之间的关系（群里的好友数、和对方共同的群数）。
    ///
    /// 不限时间范围时，顺带把群里自己的消息数保存为群提示。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub fn chat_export_stats(
        &self,
        session_ids: &[String],
        begin: i64,
        end: i64,
        include_relations: bool,
    ) -> AppResult<ExportStats> {
        let ids = uniq(session_ids);
        if ids.is_empty() {
            return Ok(ExportStats {
                data: IndexMap::new(),
            });
        }
        let wcdb = self.open_wcdb()?;
        // 统计依次读消息、会话和联系人
        wcdb.prefetch(&[
            Databases::Messages,
            Databases::Sessions,
            Databases::Contacts,
        ])
        .map_err(|e| AppError::native(format!("{e:#}")))?;
        let begin = crate::message::time::normalize_timestamp_seconds(crate::num::i64_f64(begin));
        let end = crate::message::time::normalize_timestamp_seconds(crate::num::i64_f64(end));
        let options = json!({
            "begin": begin,
            "end": end,
            "quick_mode": !include_relations && ids.len() > 1,
            "include_group_sender_count": true
        });
        let native_rows = wcdb
            .session_message_type_stats_batch(&ids, &options)
            .unwrap_or(Value::Null);
        let groups: Vec<String> = ids
            .iter()
            .filter(|i| i.ends_with("@chatroom"))
            .cloned()
            .collect();
        let privates: Vec<String> = ids
            .iter()
            .filter(|i| !i.ends_with("@chatroom"))
            .cloned()
            .collect();
        let member_counts = if groups.is_empty() {
            Value::Null
        } else {
            wcdb.group_member_counts(&groups).unwrap_or(Value::Null)
        };

        let mut private_mutual: HashMap<String, i64> = HashMap::new();
        let mut group_mutual: HashMap<String, i64> = HashMap::new();
        if include_relations {
            let self_set: HashSet<String> =
                identity_keys(&self.my_wxid_cleaned()).into_iter().collect();
            let mut relation_groups: Vec<String> = Vec::new();
            if !privates.is_empty() {
                relation_groups = self.group_session_ids(&wcdb);
                for g in &groups {
                    if !relation_groups.contains(g) {
                        relation_groups.push(g.clone());
                    }
                }
            } else if !groups.is_empty() {
                relation_groups = groups;
            }
            if !relation_groups.is_empty() {
                let mut private_index: HashMap<String, Vec<String>> = HashMap::new();
                for sid in &privates {
                    for key in identity_keys(sid) {
                        private_index.entry(key).or_default().push(sid.clone());
                    }
                    private_mutual.insert(sid.clone(), 0);
                }
                let friends = self.friend_identities(&wcdb);
                for gid in &relation_groups {
                    let members = wcdb.group_members(gid).unwrap_or(Value::Null);
                    let mut touched: HashSet<String> = HashSet::new();
                    let mut friend_members: HashSet<String> = HashSet::new();
                    for m in member_list(&members) {
                        let keys = identity_keys(&member_username(&m));
                        let Some(canonical) = keys.first().cloned() else {
                            continue;
                        };
                        if !self_set.contains(&canonical) && friends.contains(&canonical) {
                            friend_members.insert(canonical);
                        }
                        for key in &keys {
                            for sid in private_index.get(key).into_iter().flatten() {
                                touched.insert(sid.clone());
                            }
                        }
                    }
                    group_mutual.insert(gid.clone(), crate::num::count_i64(friend_members.len()));
                    for sid in touched {
                        *private_mutual.entry(sid).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut data = IndexMap::new();
        for id in &ids {
            let row = native_rows.get(id).unwrap_or(&Value::Null);
            let positive = |key: &str| Some(num(row, key)).filter(|n| *n > 0);
            let is_group = id.ends_with("@chatroom");
            let group = |n: i64| is_group.then_some(n);
            let my = num(row, "group_my_messages");
            if is_group && begin <= 0 && end <= 0 {
                let _ = self.set_group_hint(id, my);
            }
            let stats = SessionExportStats {
                total_messages: num(row, "total_messages"),
                voice_messages: num(row, "voice_messages"),
                image_messages: num(row, "image_messages"),
                video_messages: num(row, "video_messages"),
                emoji_messages: num(row, "emoji_messages"),
                transfer_messages: num(row, "transfer_messages"),
                red_packet_messages: num(row, "red_packet_messages"),
                call_messages: num(row, "call_messages"),
                first_timestamp: positive("first_timestamp"),
                last_timestamp: positive("last_timestamp"),
                group_my_messages: group(my),
                group_active_speakers: group(num(row, "group_sender_count")),
                group_member_count: group(num(&member_counts, id)),
                group_mutual_friends: (is_group && include_relations)
                    .then(|| group_mutual.get(id).copied().unwrap_or(0)),
                private_mutual_groups: (!is_group && include_relations)
                    .then(|| private_mutual.get(id).copied().unwrap_or(0)),
            };
            data.insert(id.clone(), stats);
        }
        Ok(ExportStats { data })
    }

    // ── 群提示：自己在群里的消息数，保存在文件里 ──

    /// 账号的群提示文件的文件名。
    fn hint_file_name(&self) -> String {
        let who = self.my_wxid_cleaned();
        let who = if who.is_empty() { "default" } else { &who };
        format!("group_my_message_counts_{who}.json")
    }

    /// 账号的群提示文件，在缓存目录中：设了 `cache_path` 时就在它下面（#1013）。
    fn hint_path(&self) -> PathBuf {
        self.cache_base().join(self.hint_file_name())
    }

    /// 主目录的 `cache/` 下账号的群提示文件：WeFlow 7.0.0 及更早版本不论 `cache_path` 怎样设置都放在这里；
    /// 这个目录就是缓存目录时为 `None`。
    pub(super) fn legacy_hint_path(&self) -> Option<PathBuf> {
        let home_cache = self.ctx.cache_dir();
        // 按系统解析后的路径比较，同一个目录换一种写法也不会被当成另一个目录
        let moved = fs::canonicalize(&home_cache).ok() != fs::canonicalize(self.cache_base()).ok();
        moved.then(|| home_cache.join(self.hint_file_name()))
    }

    /// 账号保存的群提示：先取 [`Self::legacy_hint_path`] 中的，再用缓存目录中的覆盖。
    fn read_hints(&self) -> Map<String, Value> {
        let read = |path: &Path| -> Option<Map<String, Value>> {
            serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
        };
        let mut hints = self
            .legacy_hint_path()
            .and_then(|path| read(&path))
            .unwrap_or_default();
        hints.extend(read(&self.hint_path()).unwrap_or_default());
        hints
    }

    /// 把 `count` 保存为自己在这个群里的消息数（`chat group-hint --set`），按账号存在缓存中。
    ///
    /// # Errors
    /// `chatroom_id` 不是群的 ID 时返回用法错误；文件写不进去时返回运行时错误。
    pub fn set_group_hint(&self, chatroom_id: &str, count: i64) -> AppResult<GroupHint> {
        if !chatroom_id.trim().ends_with("@chatroom") {
            return Err(AppError::usage("invalid group chat id"));
        }
        let mut hints = self.read_hints();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        hints.insert(
            chatroom_id.trim().to_string(),
            json!({ "messageCount": count.max(0), "updatedAt": now }),
        );
        let path = self.hint_path();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                AppError::runtime(format!("failed to create {}: {e}", parent.display()))
            })?;
        }
        let bytes = serde_json::to_vec_pretty(&Value::Object(hints))
            .map_err(|e| AppError::runtime(format!("{e:#}")))?;
        crate::fs_atomic::write_atomically(&path, &bytes)
            .map_err(|e| AppError::runtime(format!("failed to write {}: {e}", path.display())))?;
        // 从主目录缓存读到的群提示已一并写进缓存目录的文件，旧文件可以删掉
        if let Some(legacy) = self.legacy_hint_path() {
            let _ = fs::remove_file(legacy);
        }
        Ok(GroupHint {
            count: None,
            updated_at: Some(now),
            source: None,
        })
    }

    /// 保存的自己在这个群里的消息数（`chat group-hint`）；没有保存过时各项都省略。
    ///
    /// # Errors
    /// `chatroom_id` 不是群的 ID 时返回用法错误。
    pub fn group_hint(&self, chatroom_id: &str) -> AppResult<GroupHint> {
        if !chatroom_id.trim().ends_with("@chatroom") {
            return Err(AppError::usage("invalid group chat id"));
        }
        Ok(match self.read_hints().get(chatroom_id.trim()) {
            Some(entry) => GroupHint {
                count: entry["messageCount"].as_i64(),
                updated_at: entry["updatedAt"].as_i64(),
                source: Some("disk".into()),
            },
            None => GroupHint {
                count: None,
                updated_at: None,
                source: None,
            },
        })
    }

    // ── 资源 ──

    fn resource_type(msg: &ExportMsg) -> Option<ResourceKind> {
        match msg.local_type {
            3 => Some(ResourceKind::Image),
            43 => Some(ResourceKind::Video),
            34 => Some(ResourceKind::Voice),
            // 保存为 `(6 << 32) | 49` 的是文件；保存为 49 的只有自己说明是文件时才是（解析元数据最费时，所以放在最后）
            t if FILE_APP_TYPES.contains(&t) => {
                let is_file = t != 49
                    || msg.xml_type.as_deref() == Some("6")
                    || app_message_kind(&msg.content, t) == Some("file");
                is_file.then_some(ResourceKind::File)
            }
            _ => None,
        }
    }

    /// 聊天窗口对 `usernames` 中每一个的称呼（其中会话自己的用户名指会话本身）。
    ///
    /// 在群 `session_id` 里，用成员在群里给自己起的名字；没有时，以及在其他聊天中，依次用备注、昵称、微信号。
    /// 没有名字或读不出名字的用户名不列出。
    pub fn names_in_chat(
        &self,
        session_id: Option<&str>,
        usernames: &[String],
    ) -> HashMap<String, String> {
        use crate::export_msg::names::{build_trusted_group_nicknames, resolve_group_nickname};
        let Ok(wcdb) = self.open_wcdb() else {
            return HashMap::new();
        };
        let mut names = self.contact_names(&wcdb, usernames);
        names.retain(|username, name| username != name);
        if let Some(group) = session_id.filter(|s| s.ends_with("@chatroom")) {
            let nicknames = build_trusted_group_nicknames(self.group_nicknames_pub(group));
            for username in usernames {
                let nickname = resolve_group_nickname(&nicknames, &[username]);
                if !nickname.is_empty() {
                    names.insert(username.clone(), nickname);
                }
            }
        }
        names
    }

    /// 各会话中的图片、视频、语音和文件，按聊天中的顺序从新到旧排列（不同会话中位置相同的消息按会话 ID 排），
    /// 每页 `limit` 条（默认 300，最多 2000）。下一页从这一页的 `nextCursor` 接着读（#178）。
    ///
    /// # Errors
    /// `q` 中的类型不是 `image`、`video`、`voice`、`file` 之一，或者它的游标不是资源游标时返回用法错误；
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_resources(&self, q: &ResourceQuery) -> AppResult<Value> {
        use std::cmp::Ordering;
        use weflow_native::msg::{MsgPos, Ties};
        if let Some(unknown) = q
            .types
            .iter()
            .find(|t| !RESOURCE_TYPES.contains(&t.as_str()))
        {
            return Err(AppError::usage(format!(
                "unknown resource type: {unknown}; use image, video, voice or file"
            )));
        }
        let wcdb = self.open_wcdb()?;
        let wanted: Vec<&str> = if q.types.is_empty() {
            RESOURCE_TYPES.to_vec()
        } else {
            RESOURCE_TYPES
                .iter()
                .copied()
                .filter(|t| q.types.iter().any(|x| x == t))
                .collect()
        };
        let limit = if q.limit == 0 {
            300
        } else {
            q.limit.clamp(1, 2000)
        };
        let before = match q.before.as_deref().map(str::trim).filter(|t| !t.is_empty()) {
            Some(token) => Some(
                token
                    .split_once(':')
                    .and_then(|(pos, sid)| Some((MsgPos::parse(pos)?, sid.to_string())))
                    .filter(|(_, sid)| !sid.is_empty())
                    .ok_or_else(|| {
                        AppError::usage(format!(
                            "`{token}` is not a resource cursor; pass the `nextCursor` of a page"
                        ))
                    })?,
            ),
            None => None,
        };
        // 会话按最后一条消息的时间从新到旧排列：任何会话都不会有排在这个时间之后很多的消息
        let requested = q.session_id.as_deref().map(str::trim).unwrap_or("");
        let mut sessions: Vec<(String, i64)> = Vec::new();
        if requested.is_empty() {
            for s in wcdb
                .sessions()
                .map_err(native)?
                .as_array()
                .into_iter()
                .flatten()
            {
                let name = ["username", "userName", "talker", "sessionId"]
                    .iter()
                    .find_map(|k| s.get(*k).and_then(Value::as_str))
                    .unwrap_or("");
                if !name.is_empty() {
                    let last = num(s, "sort_timestamp").max(num(s, "sortTimestamp"));
                    sessions.push((name.to_string(), last));
                }
            }
            sessions.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        } else {
            sessions.push((requested.to_string(), i64::MAX));
        }
        let mut local_types: Vec<i64> = Vec::new();
        if wanted.contains(&"image") {
            local_types.push(3);
        }
        if wanted.contains(&"video") {
            local_types.push(43);
        }
        if wanted.contains(&"voice") {
            local_types.push(34);
        }
        if wanted.contains(&"file") {
            local_types.extend(FILE_APP_TYPES);
        }
        // 会话最后一条消息的时间以秒为单位，而消息的位置是毫秒级的发送时间再加一点
        const SLACK_SECS: i64 = 10;
        let want = limit + 1;
        let my = self.my_wxid_cleaned();
        let place = |p: &MsgPos| (p.sort_seq, p.create_time, p.local_id);
        // （位置，会话，消息，种类），按列出的顺序
        let mut items: Vec<(MsgPos, String, ExportMsg, ResourceKind)> = Vec::new();
        let tables = wcdb.message_tables_by_name().map_err(native)?;
        for (sid, last) in &sessions {
            let floor = (items.len() >= want).then(|| items[want - 1].0.sort_seq);
            let newest = last.saturating_add(SLACK_SECS);
            if floor.is_some_and(|f| newest.saturating_mul(1000) < f)
                || (q.begin > 0 && newest < q.begin)
            {
                break;
            }
            let Some(tables) = tables.get(&table_name_for(sid)) else {
                continue;
            };
            let from = before.as_ref().map(|(pos, at)| {
                let ties = match sid.as_str().cmp(at) {
                    Ordering::Greater => Ties::All,
                    _ => Ties::None,
                };
                (*pos, ties)
            });
            let opts = CollectOptions {
                session_id: sid,
                my_wxid: &my,
                start: None,
                end: None,
                sender_filter: None,
            };
            // 存在多个数据库中的消息（重新同步过的）只列一次
            let mut seen: HashSet<(i64, i64, i64)> = HashSet::new();
            let mut from = from;
            let mut found = 0;
            loop {
                // 比目前这一页更旧的消息不可能在这一页上
                let rows = wcdb
                    .typed_before(
                        tables,
                        &local_types,
                        from,
                        (q.begin, q.end),
                        floor.unwrap_or(0),
                        crate::num::count_i64(want),
                    )
                    .map_err(native)?;
                let (count, end) = (rows.len(), rows.last().map(MsgRow::pos));
                for row in rows {
                    let pos = row.pos();
                    if !seen.insert(place(&pos)) {
                        continue;
                    }
                    let Some(m) = collect_rows(vec![row], &opts).pop() else {
                        continue;
                    };
                    let Some(kind) = Self::resource_type(&m) else {
                        continue;
                    };
                    found += 1;
                    items.push((pos, sid.clone(), m, kind));
                }
                // 不是资源的行（类型为 49 的 app 消息要看内容才知道是不是文件）和已经列出的行跳过，接着往下读
                let Some(end) = end else {
                    break;
                };
                if found >= want || count < want {
                    break;
                }
                from = Some((end, Ties::None));
            }
            items.sort_by(|a, b| place(&b.0).cmp(&place(&a.0)).then_with(|| a.1.cmp(&b.1)));
            items.truncate(want);
        }
        let more = items.len() > limit;
        items.truncate(limit);
        let ids: Vec<String> = items
            .iter()
            .map(|i| i.1.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let names = self.contact_names(&wcdb, &ids);
        let page: Vec<Resource> = items
            .into_iter()
            .map(|(pos, sid, m, kind)| Resource {
                message: MessageView::of(&m, &my),
                cursor: format!("{}:{sid}", pos.token()),
                session_display_name: names.get(&sid).cloned().unwrap_or_else(|| sid.clone()),
                session_id: sid,
                resource_type: kind,
            })
            .collect();
        let next_cursor = if more {
            page.last().map(|r| r.cursor.clone())
        } else {
            None
        };
        Ok(json!(ResourcePage {
            items: page,
            has_more: more,
            next_cursor,
        }))
    }

    /// 会话中去重后的图片标识，从新到旧。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_all_images(&self, session_id: &str) -> AppResult<ChatImages> {
        let wcdb = self.open_wcdb()?;
        let rows = wcdb
            .messages_by_type(session_id, 3, Order::NewestFirst)
            .map_err(native)?;
        let msgs = self.normalize_row(rows, session_id);
        let mut images: Vec<(i64, ImageRef)> = msgs
            .into_iter()
            .filter(|m| m.local_type == 3 && (m.image_md5.is_some() || m.image_dat_name.is_some()))
            .map(|m| {
                (
                    m.create_time,
                    ImageRef {
                        image_md5: m.image_md5,
                        image_dat_name: m.image_dat_name,
                        create_time: (m.create_time > 0).then_some(m.create_time),
                    },
                )
            })
            .collect();
        images.sort_by_key(|a| std::cmp::Reverse(a.0));
        let mut seen = HashSet::new();
        let images: Vec<ImageRef> = images
            .into_iter()
            .filter(|(_, image)| {
                let key = image
                    .image_md5
                    .as_deref()
                    .or(image.image_dat_name.as_deref())
                    .unwrap_or("")
                    .to_string();
                !key.is_empty() && seen.insert(key)
            })
            .map(|(_, image)| image)
            .collect();
        Ok(ChatImages { images })
    }

    /// 会话的语音消息，从新到旧，每条一次（`chat voice-messages`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_all_voices(&self, session_id: &str) -> AppResult<VoiceMessages> {
        let wcdb = self.open_wcdb()?;
        let rows = wcdb
            .messages_by_type(session_id, 34, Order::NewestFirst)
            .map_err(native)?;
        let my = self.my_wxid_cleaned();
        let mut msgs = self.normalize_row(rows, session_id);
        msgs.sort_by_key(|a| std::cmp::Reverse(a.create_time));
        let mut seen = HashSet::new();
        let messages: Vec<MessageView> = msgs
            .into_iter()
            .filter(|m| seen.insert(format!("{}-{}-{}", m.server_id, m.local_id, m.create_time)))
            .map(|m| MessageView::of(&m, &my))
            .collect();
        Ok(VoiceMessages { messages })
    }

    /// 各会话的图片和视频消息，由数据库层的扫描分页。
    ///
    /// # Errors
    /// `media_type` 不是 `image`、`video`、`all` 之一时返回用法错误；账号没有配置好时返回配置错误，
    /// 读取数据库失败时返回数据库层的错误。
    pub fn chat_media_stream(
        &self,
        session_id: Option<&str>,
        media_type: &str,
        begin: i64,
        end: i64,
        limit: i32,
        offset: i32,
    ) -> AppResult<MediaStream> {
        let code = match media_type {
            "image" => 1,
            "video" => 2,
            "all" => 0,
            other => {
                return Err(AppError::usage(format!(
                    "unknown media type: {other}; use image, video or all"
                )))
            }
        };
        let wcdb = self.open_wcdb()?;
        let ids: Vec<String> = match session_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => vec![s.to_string()],
            None => extract_session_ids(&wcdb.sessions().map_err(native)?),
        };
        let ids = uniq(&ids);
        let (rows, has_more) = wcdb
            .scan_media_stream(
                &ids,
                code,
                crate::num::saturating_i32(begin),
                crate::num::saturating_i32(end),
                limit,
                offset,
            )
            .map_err(native)?;
        let my = self.my_wxid_cleaned();
        let names = self.contact_names(&wcdb, &ids);
        let mut items: Vec<MediaStreamItem> = Vec::new();
        for hit in rows {
            let sid = hit.session_id;
            let msgs = collect_rows(
                vec![hit.row],
                &CollectOptions {
                    session_id: &sid,
                    my_wxid: &my,
                    start: None,
                    end: None,
                    sender_filter: None,
                },
            );
            let Some(m) = msgs.into_iter().next() else {
                continue;
            };
            let kind = if m.local_type == 43 { "video" } else { "image" };
            items.push(MediaStreamItem {
                message: MessageView::of(&m, &my),
                session_display_name: names.get(&sid).cloned().unwrap_or_else(|| sid.clone()),
                session_id: sid,
                media_type: kind.into(),
            });
        }
        Ok(MediaStream {
            next_offset: usize::try_from(offset).unwrap_or(0) + items.len(),
            items,
            has_more,
        })
    }

    /// 群里的转账消息显示的付款人和收款人的名字（`chat transfer-names`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，数据库打不开时返回数据库层的错误。
    pub fn chat_transfer_names(
        &self,
        chatroom_id: &str,
        payer: &str,
        receiver: &str,
    ) -> AppResult<TransferNames> {
        use crate::export_msg::names::{build_trusted_group_nicknames, resolve_group_nickname};
        let wcdb = self.open_wcdb()?;
        let nicks = if chatroom_id.ends_with("@chatroom") {
            let v = wcdb.group_nicknames(chatroom_id).unwrap_or(Value::Null);
            let obj = v
                .get("nicknames")
                .and_then(Value::as_object)
                .or_else(|| v.as_object());
            build_trusted_group_nicknames(
                obj.map(|o| {
                    o.iter()
                        .filter_map(|(k, n)| n.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default(),
            )
        } else {
            HashMap::new()
        };
        let raw_me = self.configured_wxid().unwrap_or_default();
        let me = clean_account_dir_name(&raw_me);
        let resolve = |username: &str| -> String {
            if !raw_me.is_empty() && (username == raw_me || username == me) {
                let g = resolve_group_nickname(&nicks, &[username, raw_me.as_str()]);
                if !g.is_empty() {
                    return g;
                }
                return crate::locale::tr("Me", "我").to_string();
            }
            let g = resolve_group_nickname(&nicks, &[username]);
            if !g.is_empty() {
                return g;
            }
            match wcdb
                .contact(username)
                .ok()
                .filter(|v| v.as_object().is_some_and(|o| !o.is_empty()))
            {
                Some(c) => ["remark", "nickName", "nick_name", "alias"]
                    .iter()
                    .find_map(|k| c.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()))
                    .unwrap_or(username)
                    .to_string(),
                None => username.to_string(),
            }
        };
        Ok(TransferNames {
            payer_name: resolve(payer),
            receiver_name: resolve(receiver),
        })
    }
}

impl ServiceHub {
    /// `SessionTable` 的行，从新到旧，保持微信保存的原样。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn sessions(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.sessions()
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// `export sessions` 写出的列表：[`Self::sessions`]，每个会话在 `username` 之后加上
    /// `display_name`（联系人的备注，没有时依次用昵称、微信号和用户名）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn sessions_with_names(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let sessions = wcdb
            .sessions()
            .map_err(|err| AppError::native(format!("{err:#}")))?;
        let Value::Array(rows) = sessions else {
            return Ok(sessions);
        };
        let users: Vec<String> = rows
            .iter()
            .filter_map(|row| row["username"].as_str().map(str::to_string))
            .collect();
        let names = wcdb.display_names(&users).ok().unwrap_or(Value::Null);
        let rows = rows
            .into_iter()
            .map(|row| {
                let Value::Object(fields) = row else {
                    return row;
                };
                let name = fields
                    .get("username")
                    .and_then(Value::as_str)
                    .and_then(|user| names.get(user))
                    .cloned()
                    .unwrap_or(Value::Null);
                let mut named = Map::with_capacity(fields.len() + 1);
                for (key, value) in fields {
                    let after = key == "username";
                    named.insert(key, value);
                    if after {
                        named.insert("display_name".into(), name.clone());
                    }
                }
                Value::Object(named)
            })
            .collect();
        Ok(Value::Array(rows))
    }

    /// 会话从第 `offset` 条起的 `limit` 条消息，顺序与 `chat messages` 相同。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn messages(&self, session_id: &str, limit: i32, offset: i32) -> AppResult<Vec<MsgRow>> {
        let wcdb = self.open_wcdb()?;
        wcdb.messages(session_id, limit, offset)
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// `chat messages` 的一页，从新到旧，返回类型化的行（#492）。
    ///
    /// 从最新的消息开始，跳过 `offset` 条；或者从某个 `_cursor` 所在消息的旁边开始（`before`：更旧的；
    /// `after`：离它最近的更新的）。每一行作为 [`MessageRow`](crate::contract::MessageRow) 带有自己的 `_cursor`，
    /// 用来请求它旁边的一页。
    ///
    /// # Errors
    ///
    /// 游标无效时返回用法错误；账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn message_page_rows(
        &self,
        session_id: &str,
        limit: i32,
        offset: i32,
        before: Option<&str>,
        after: Option<&str>,
    ) -> AppResult<Vec<MsgRow>> {
        use weflow_native::msg::{MsgPos, PageFrom};
        let pos = |token: &str| {
            MsgPos::parse(token).ok_or_else(|| {
                AppError::usage(format!(
                    "`{token}` is not a message cursor; pass the `_cursor` of a message"
                ))
            })
        };
        let from = match (before, after) {
            (Some(token), _) => Some(PageFrom::Before(pos(token)?)),
            (None, Some(token)) => Some(PageFrom::After(pos(token)?)),
            (None, None) => None,
        };
        let wcdb = self.open_wcdb()?;
        match from {
            Some(from) => wcdb.messages_from(session_id, limit, from),
            None => wcdb.messages(session_id, limit, offset),
        }
        .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// `chat messages --since`：在 `seconds`（Unix 时间）或之后发送的最早的 `limit` 条消息，仍按从新到旧排列，
    /// 每条带自己的 `_cursor`：聊天窗口跳到某一天时从这里开始，再向两边翻页。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn message_page_since(
        &self,
        session_id: &str,
        limit: i32,
        seconds: i64,
    ) -> AppResult<Vec<MsgRow>> {
        use weflow_native::msg::{MsgPos, PageFrom};
        self.open_wcdb()?
            .messages_from(session_id, limit, PageFrom::After(MsgPos::at_time(seconds)))
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// 把一个会话的类型化的行转成聊天窗口显示的样子，每条带自己的 `_cursor`（#492）。
    pub fn parsed_rows(
        &self,
        rows: Vec<MsgRow>,
        session_id: &str,
    ) -> Vec<crate::contract::ParsedMessage> {
        let cursors: Vec<String> = rows.iter().map(|r| r.pos().token()).collect();
        chat_msg::map_msg_rows(rows, session_id, &self.wmy())
            .into_iter()
            .zip(cursors)
            .map(|(message, cursor)| crate::contract::ParsedMessage {
                message,
                cursor: Some(cursor),
                session_id: None,
            })
            .collect()
    }

    /// `chat message --parsed`：查到的消息，按一页消息的方式解析并带上 `_cursor`，
    /// 聊天窗口可以在那里打开它的会话（#200）。
    pub fn parsed_lookup(
        &self,
        row: MsgRow,
        session_id: &str,
    ) -> Option<crate::contract::ParsedMessage> {
        self.parsed_rows(vec![row], session_id).pop()
    }

    /// `chat search`：搜到的消息，从新到旧。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn search(
        &self,
        keyword: &str,
        session_id: Option<&str>,
        limit: i32,
        offset: i32,
        begin: i32,
        end: i32,
    ) -> AppResult<Vec<SessionRow>> {
        let wcdb = self.open_wcdb()?;
        wcdb.search(keyword, session_id, limit, offset, begin, end)
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// `chat search --parsed`：每条搜到的消息按聊天窗口的样子给出，带上 `_cursor`（从这里翻它所在的会话）
    /// 和 `_session_id`。
    pub fn parsed_hits(&self, hits: Vec<SessionRow>) -> Vec<crate::contract::ParsedMessage> {
        let my = self.wmy();
        hits.into_iter()
            .map(|hit| {
                let cursor = hit.row.pos().token();
                let table_name = table_name_for(&hit.session_id);
                crate::contract::ParsedMessage {
                    message: chat_msg::map_msg_row(hit.row, &table_name, &my),
                    cursor: Some(cursor),
                    session_id: Some(hit.session_id),
                }
            })
            .collect()
    }

    /// `contact` 表的每一行，保持微信保存的原样（不带 `--parsed` 的 `chat contacts`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn contacts(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.contacts()
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// 一个联系人的行，不认识时为 `{}`（`chat contact`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn contact(&self, username: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.contact(username)
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// `{ "private", "group", "official", "former_friend" }`：每类联系人各有多少。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn contact_type_counts(&self) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        wcdb.contact_type_counts()
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// 群的成员、他们的群昵称和人数，合成一个 JSON 对象。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读不出成员时返回数据库层的错误。
    pub fn group_members(&self, chatroom_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let members = wcdb
            .group_members(chatroom_id)
            .map_err(|err| AppError::native(format!("{err:#}")))?;
        let nicknames = wcdb.group_nicknames(chatroom_id).unwrap_or(Value::Null);
        let count = wcdb.group_member_count(chatroom_id).unwrap_or(Value::Null);
        Ok(json!({
            "chatroomId": chatroom_id,
            "members": members,
            "nicknames": nicknames,
            "count": count
        }))
    }

    /// 从 `min_time` 起按时间升序读出的一批消息（游标的一次读取）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_new_messages(
        &self,
        session_id: &str,
        min_time: i64,
        limit: i32,
    ) -> AppResult<Vec<ChatMessage>> {
        let wcdb = self.open_wcdb()?;
        self.new_messages_with(&wcdb, session_id, min_time, limit)
    }

    pub(super) fn new_messages_with(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session_id: &str,
        min_time: i64,
        limit: i32,
    ) -> AppResult<Vec<ChatMessage>> {
        let cursor = wcdb
            .open_message_cursor(
                session_id,
                limit,
                Order::OldestFirst,
                crate::num::saturating_i32(min_time.max(0)),
                0,
                Columns::All,
            )
            .map_err(|e| AppError::native(format!("{e:#}")))?;
        let fetched = wcdb.fetch_message_batch(cursor);
        let _ = wcdb.close_message_cursor(cursor);
        let (rows, _) = fetched.map_err(|e| AppError::native(format!("{e:#}")))?;
        let mut msgs = chat_msg::map_msg_rows(rows, session_id, &self.wmy());
        // 两条都有排序序号时按它排，否则依次按时间、排序序号、本地 ID 和服务器 ID 排
        msgs.sort_by(|a, b| {
            let (asq, bsq) = (a.sort_seq.max(0), b.sort_seq.max(0));
            if asq > 0 && bsq > 0 && asq != bsq {
                return asq.cmp(&bsq);
            }
            a.create_time
                .max(0)
                .cmp(&b.create_time.max(0))
                .then(asq.cmp(&bsq))
                .then(a.local_id.max(0).cmp(&b.local_id.max(0)))
                .then(a.server_id.max(0).cmp(&b.server_id.max(0)))
        });
        Ok(msgs)
    }

    /// 最新的 `limit` 条消息（1 到 500），从旧到新排列（按偏移直接读取）。
    ///
    /// 私聊中只留对方和自己发的消息，全被滤掉时保留全部。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn chat_latest_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> AppResult<Vec<ChatMessage>> {
        let wcdb = self.open_wcdb()?;
        let page = limit.clamp(1, 500);
        let rows = wcdb
            .messages(session_id, crate::num::saturating_i32(page), 0)
            .map_err(|e| AppError::native(format!("{e:#}")))?;
        let mapped = chat_msg::map_msg_rows(rows, session_id, &self.wmy());
        let is_group = session_id.contains("@chatroom");
        let visible: Vec<ChatMessage> = mapped
            .iter()
            .filter(|m| {
                is_group
                    || m.sender_username
                        .as_deref()
                        .is_none_or(|s| s.is_empty() || s == session_id)
                    || m.is_send == Some(1)
            })
            .cloned()
            .collect();
        let mut out = if visible.is_empty() && !mapped.is_empty() {
            mapped
        } else {
            visible
        };
        out.sort_by(|a, b| {
            let (asq, bsq) = (a.sort_seq.max(0), b.sort_seq.max(0));
            if asq > 0 && bsq > 0 && asq != bsq {
                return asq.cmp(&bsq);
            }
            a.create_time
                .max(0)
                .cmp(&b.create_time.max(0))
                .then(asq.cmp(&bsq))
                .then(a.local_id.max(0).cmp(&b.local_id.max(0)))
                .then(a.server_id.max(0).cmp(&b.server_id.max(0)))
        });
        Ok(out)
    }
}

pub(crate) fn extract_session_ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(session_id_from_value)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn session_id_from_value(value: &Value) -> Option<String> {
    if let Some(text) = value.as_str() {
        return Some(text.to_string());
    }
    let object = value.as_object()?;
    for key in [
        "session_id",
        "sessionId",
        "id",
        "username",
        "userName",
        "talker",
        "strUsrName",
    ] {
        if let Some(text) = object.get(key).and_then(Value::as_str) {
            if !text.trim().is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_keys_join_an_account_and_its_folder_but_not_another_id() {
        let shares = |a: &str, b: &str| {
            let keys = identity_keys(a);
            identity_keys(b).iter().any(|k| keys.contains(k))
        };
        for (a, b, same) in [
            ("john", "john_1a2b", true),
            ("WXID_ME", "wxid_me_1a2b", true),
            ("john", "john_smith", false),
            ("john_smith", "john", false),
        ] {
            assert_eq!(shares(a, b), same, "{a} {b}");
            assert_eq!(
                crate::message::transfer::is_same_wxid(a, b),
                same,
                "{a} {b}"
            );
        }
        assert!(identity_keys("  ").is_empty());
    }
}
