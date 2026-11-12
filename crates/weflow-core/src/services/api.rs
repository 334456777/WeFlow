//! Service layer behind the HTTP API: sessions, contacts, message paging, ChatLab conversion.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::*;
use crate::api::{self, ApiExportedMedia, ApiMediaOptions};
use crate::chat_msg::{self, ChatMessage};
use crate::contract::{Contact, ContactKind, Session};
use crate::message::row::{row_field, row_int};
use crate::message::xml::rx;
use crate::sync::MutexExt;

const FRIEND_EXCLUDE_USERNAMES: [&str; 5] =
    ["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];

fn js_int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n
            .as_i64()
            .or_else(|| n.as_f64().map(crate::num::saturating_i64)),
        Value::String(s) => api::js_parse_int(s),
        _ => None,
    }
}

fn first_str(row: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(v) = row.get(*k) {
            let t = match v {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                _ => String::new(),
            };
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

fn is_enterprise_openim(username: &str) -> bool {
    let l = username.trim().to_lowercase();
    l.contains("@openim") && !l.contains("@kefu.openim")
}

fn allowed_openim_by_local_type(username: &str, local_type: Option<i64>) -> bool {
    is_enterprise_openim(username) && local_type == Some(5)
}

pub(crate) fn should_keep_session(username: &str, local_type: Option<i64>) -> bool {
    if username.is_empty() {
        return false;
    }
    let lowered = username.to_lowercase();
    if lowered.contains("@placeholder") || username.starts_with("gh_") || lowered == "weixin" {
        return false;
    }
    for prefix in [
        "qqmail",
        "fmessage",
        "medianote",
        "floatbottle",
        "newsapp",
        "brandsessionholder",
        "brandservicesessionholder",
        "notifymessage",
        "opencustomerservicemsg",
        "notification_messages",
        "userexperience_alarm",
        "helper_folders",
        "@helper_folders",
    ] {
        if username.starts_with(prefix) || username == prefix {
            return false;
        }
    }
    if username.contains("@kefu.openim") {
        return false;
    }
    if is_enterprise_openim(username) {
        return allowed_openim_by_local_type(username, local_type);
    }
    !username.contains("service_")
}

fn session_local_type(row: &Value) -> Option<i64> {
    row_field(row, &["local_type", "localType", "WCDB_CT_local_type"]).and_then(|v| js_int(Some(v)))
}

fn message_label(local_type: i64) -> &'static str {
    chat_msg::message_type_label(local_type)
}

fn summary_from_message(m: &ChatMessage) -> String {
    let raw = match m.local_type {
        1 => {
            if !m.parsed_content.is_empty() {
                m.parsed_content.clone()
            } else {
                m.raw_content.clone()
            }
        }
        3 => "[图片]".into(),
        34 => "[语音]".into(),
        43 => "[视频]".into(),
        47 => "[表情]".into(),
        42 => m
            .card_nickname
            .clone()
            .filter(|n| !n.is_empty())
            .unwrap_or_else(|| "[名片]".into()),
        48 => "[位置]".into(),
        49 => m
            .link_title()
            .filter(|t| !t.is_empty())
            .map(str::to_string)
            .or_else(|| m.file_name().filter(|t| !t.is_empty()).map(str::to_string))
            .or_else(|| Some(m.parsed_content.clone()).filter(|t| !t.is_empty()))
            .unwrap_or_else(|| "[消息]".into()),
        t => {
            if !m.parsed_content.is_empty() {
                m.parsed_content.clone()
            } else if !m.raw_content.is_empty() {
                m.raw_content.clone()
            } else {
                message_label(t).into()
            }
        }
    };
    rx(r"^\s*\[视频号\]\s*")
        .replace(&chat_msg::clean_utf16(&raw), "")
        .trim()
        .to_string()
}

impl ServiceHub {
    /// Raw configuration value (`messagePushFilterMode`, …) of the active profile.
    pub fn config_value(&self, key: &str) -> Value {
        self.config.get_key(Some(&self.profile_name), key)
    }

    pub fn open_wcdb_pub(&self) -> AppResult<weflow_native::wcdb::Wcdb> {
        self.open_wcdb()
    }

    pub fn my_wxid_pub(&self) -> String {
        self.my_wxid_cleaned()
    }

    pub fn push_avatar_dir(&self) -> PathBuf {
        self.cache_base().join("push-avatar-files")
    }

    /// Raw `id → group nickname` map of a chatroom.
    pub fn group_nicknames_pub(&self, chatroom_id: &str) -> Vec<(String, String)> {
        let Ok(wcdb) = self.open_wcdb() else {
            return Vec::new();
        };
        match wcdb.group_nicknames(chatroom_id) {
            Ok(Value::Object(m)) => m
                .into_iter()
                .map(|(k, v)| (k, v.as_str().unwrap_or("").to_string()))
                .collect(),
            _ => Vec::new(),
        }
    }

    /// `getContactAvatar`: `(avatar url, display name)`.
    pub fn chat_contact_avatar(&self, username: &str) -> Option<(Option<String>, String)> {
        if username.is_empty() {
            return None;
        }
        let wcdb = self.open_wcdb().ok()?;
        let contact = wcdb.contact(username).ok().filter(Value::is_object);
        let avatar = serde_json::to_string(&[username])
            .ok()
            .and_then(|payload| wcdb.avatar_urls(&payload).ok())
            .and_then(|m| m.get(username).and_then(Value::as_str).map(str::to_string))
            .filter(|a| !a.is_empty() && !a.contains("base64,ffd8"));
        let pick = |keys: &[&str]| {
            contact.as_ref().and_then(|c| {
                keys.iter().find_map(|k| {
                    c.get(*k)
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
            })
        };
        let name = pick(&["remark"])
            .or_else(|| pick(&["nickName", "nick_name"]))
            .or_else(|| pick(&["alias"]))
            .unwrap_or_else(|| username.to_string());
        Some((avatar, name))
    }

    pub(crate) fn wmy(&self) -> String {
        self.my_wxid_cleaned()
    }

    fn contacts_compact_for(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> Vec<Value> {
        let payload = serde_json::to_string(usernames).unwrap_or_else(|_| "[]".into());
        match wcdb.invoke_json(
            "wcdb_get_contacts_compact",
            &[weflow_native::wcdb::Arg::S(&payload)],
        ) {
            Ok(Value::Array(rows)) => rows,
            _ => Vec::new(),
        }
    }

    /// `getNewMessages`: one ascending cursor batch starting at `min_time`.
    pub fn chat_new_messages(
        &self,
        session_id: &str,
        min_time: i64,
        limit: i32,
    ) -> AppResult<Vec<ChatMessage>> {
        let wcdb = self.open_wcdb()?;
        self.new_messages_with(&wcdb, session_id, min_time, limit)
    }

    fn new_messages_with(
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
                true,
                crate::num::saturating_i32(min_time.max(0)),
                0,
                false,
            )
            .map_err(|e| AppError::native(e.to_string()))?;
        let fetched = wcdb.fetch_message_batch(cursor);
        let _ = wcdb.close_message_cursor(cursor);
        let (rows, _) = fetched.map_err(|e| AppError::native(e.to_string()))?;
        let mut msgs = chat_msg::map_msg_rows(rows, session_id, &self.wmy());
        // `normalizeMessageOrder`
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

    /// `getSessions`: filtered, typed session list. Display names come from WCDB when the
    /// desktop contact cache has none.
    pub fn chat_sessions_list(&self) -> AppResult<Vec<Session>> {
        let wcdb = self.open_wcdb()?;
        self.sessions_with(&wcdb)
    }

    fn sessions_with(&self, wcdb: &weflow_native::wcdb::Wcdb) -> AppResult<Vec<Session>> {
        let raw = wcdb
            .sessions()
            .map_err(|e| AppError::native(e.to_string()))?;
        let rows = raw.as_array().cloned().unwrap_or_default();
        if let Some(first) = rows.first() {
            if first.get("_error").is_some() || first.get("_info").is_some() {
                let detail = first
                    .get("_error")
                    .or_else(|| first.get("_info"))
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .unwrap_or_else(|| v.to_string())
                    })
                    .unwrap_or_default();
                return Err(AppError::native(format!("session table error: {detail}")));
            }
        }
        let username_keys = [
            "username",
            "user_name",
            "userName",
            "usrName",
            "UsrName",
            "talker",
            "talker_id",
            "talkerId",
        ];
        let openim_ids: Vec<String> = rows
            .iter()
            .map(|r| first_str(r, &username_keys).trim().to_string())
            .filter(|u| is_enterprise_openim(u))
            .collect();
        let mut openim_types: HashMap<String, i64> = HashMap::new();
        if !openim_ids.is_empty() {
            for row in self.contacts_compact_for(wcdb, &openim_ids) {
                let u = first_str(&row, &["username"]).trim().to_string();
                if let (false, Some(t)) = (u.is_empty(), session_local_type(&row)) {
                    openim_types.insert(u, t);
                }
            }
        }
        let my = self.wmy();
        let mut sessions: Vec<Session> = Vec::new();
        for row in &rows {
            let username = first_str(row, &username_keys);
            let mut local_type = session_local_type(row);
            if local_type.is_none() && is_enterprise_openim(&username) {
                local_type = openim_types.get(&username).copied();
            }
            if !should_keep_session(&username, local_type) {
                continue;
            }
            let sort_ts = js_int(
                ["sort_timestamp", "sortTimestamp", "sort_time", "sortTime"]
                    .iter()
                    .filter_map(|k| row.get(*k))
                    .find(|v| truthy(v)),
            )
            .unwrap_or(0);
            let last_ts = js_int(
                [
                    "last_timestamp",
                    "lastTimestamp",
                    "last_msg_time",
                    "lastMsgTime",
                ]
                .iter()
                .filter_map(|k| row.get(*k))
                .find(|v| truthy(v)),
            )
            .unwrap_or(sort_ts);
            let summary = chat_msg::clean_utf16(&first_str(
                row,
                &["summary", "digest", "last_msg", "lastMsg"],
            ));
            let last_msg_type = js_int(
                row.get("last_msg_type")
                    .filter(|v| truthy(v))
                    .or_else(|| row.get("lastMsgType").filter(|v| truthy(v))),
            )
            .unwrap_or(0);
            let hint_raw = [
                "message_count",
                "messageCount",
                "msg_count",
                "msgCount",
                "total_count",
                "totalCount",
                "n_msg",
                "nMsg",
                "message_num",
                "messageNum",
            ]
            .iter()
            .filter_map(|k| row.get(*k))
            .find(|v| !v.is_null());
            let hint = hint_raw
                .and_then(|v| {
                    v.as_f64()
                        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
                })
                .filter(|n: &f64| n.is_finite() && *n >= 0.0)
                .map(|n| crate::num::saturating_i64(n.floor()));

            let text = |key: &str| row.get(key).and_then(Value::as_str).map(str::to_string);
            sessions.push(Session {
                display_name: username.clone(),
                username,
                kind: js_int(row.get("type").filter(|v| truthy(v))).unwrap_or(0),
                unread_count: js_int(
                    ["unread_count", "unreadCount", "unreadcount"]
                        .iter()
                        .filter_map(|k| row.get(*k))
                        .find(|v| truthy(v)),
                )
                .unwrap_or(0),
                summary: if summary.is_empty() {
                    message_label(last_msg_type).to_string()
                } else {
                    summary
                },
                sort_timestamp: sort_ts,
                last_timestamp: last_ts,
                last_msg_type,
                message_count_hint: hint,
                last_msg_sender: text("last_msg_sender"),
                last_sender_display_name: text("last_sender_display_name"),
                self_wxid: my.clone(),
                avatar_url: None,
            });
        }

        // addMissingOfficialSessions
        let existing: HashSet<String> = sessions.iter().map(|s| s.username.clone()).collect();
        let mut existing = existing;
        if let Ok(Value::Array(contacts)) = wcdb.contacts() {
            for row in contacts {
                let username = first_str(&row, &["username"]).trim().to_string();
                if username.is_empty() || existing.contains(&username) {
                    continue;
                }
                let lowered = username.to_lowercase();
                let local_type =
                    row_field(&row, &["local_type", "localType", "WCDB_CT_local_type"])
                        .and_then(|v| js_int(Some(v)));
                let is_official = username.starts_with("gh_");
                let special_weixin = lowered.starts_with("weixin") && lowered != "weixin";
                let special_openim = allowed_openim_by_local_type(&username, local_type);
                if !is_official && !special_weixin && !special_openim {
                    continue;
                }
                let display = ["remark", "nick_name", "alias"]
                    .iter()
                    .map(|k| first_str(&row, &[k]))
                    .find(|s| !s.is_empty())
                    .unwrap_or_else(|| username.clone());
                existing.insert(username.clone());
                sessions.push(Session {
                    username,
                    summary: if is_official {
                        "查看公众号历史消息"
                    } else {
                        "暂无会话记录"
                    }
                    .to_string(),
                    display_name: display,
                    self_wxid: my.clone(),
                    ..Session::default()
                });
            }
        }

        // applySyntheticUnreadCounts (official accounts have no unread column)
        for s in sessions
            .iter_mut()
            .filter(|s| s.username.trim().starts_with("gh_"))
        {
            let name = s.username.clone();
            let (mut total, mut latest_snapshot) = (0i64, 0i64);
            if let Ok(Value::Array(tables)) = wcdb.message_table_stats(&name) {
                for t in tables {
                    let c = ["count", "message_count", "messageCount"]
                        .iter()
                        .filter_map(|k| t.get(*k))
                        .find_map(|v| {
                            v.as_f64()
                                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                        })
                        .unwrap_or(0.0);
                    if c.is_finite() && c > 0.0 {
                        total += crate::num::saturating_i64(c.floor());
                    }
                    let l = [
                        "last_timestamp",
                        "lastTimestamp",
                        "last_time",
                        "lastTime",
                        "max_create_time",
                        "maxCreateTime",
                    ]
                    .iter()
                    .filter_map(|k| t.get(*k))
                    .find_map(|v| {
                        v.as_f64()
                            .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
                    })
                    .unwrap_or(0.0);
                    if l.is_finite() && crate::num::saturating_i64(l) > latest_snapshot {
                        latest_snapshot = crate::num::saturating_i64(l.floor());
                    }
                }
            }
            let latest = s.last_timestamp.max(s.sort_timestamp).max(latest_snapshot);
            if latest > 0 {
                s.last_timestamp = latest;
                s.sort_timestamp = s.sort_timestamp.max(latest);
            }
            if total > 0 {
                s.message_count_hint = Some(s.message_count_hint.unwrap_or(0).max(total));
            }
            let mut unread = 0i64;
            let mut latest_msg: Option<ChatMessage> = None;
            let now = chrono::Utc::now().timestamp();
            if latest > 0 && (now - latest).abs() <= 10 * 60 {
                if let Ok(msgs) = self.new_messages_with(wcdb, &name, (latest - 1).max(0), 20) {
                    let fresh: Vec<&ChatMessage> = msgs
                        .iter()
                        .filter(|m| m.create_time >= latest && m.is_send != Some(1))
                        .collect();
                    unread = fresh.len() as i64;
                    latest_msg = fresh.last().map(|m| (*m).clone());
                }
            }
            if let Some(m) = latest_msg {
                s.summary = summary_from_message(&m);
                s.last_msg_type = m.local_type;
            }
            s.unread_count = s.unread_count.max(unread);
        }
        sessions.sort_by(|a, b| {
            let key = |s: &Session| {
                if s.sort_timestamp != 0 {
                    s.sort_timestamp
                } else {
                    s.last_timestamp
                }
            };
            key(b).cmp(&key(a))
        });

        // resolve names / avatars (WeFlow's desktop app fills these from its contact cache)
        let names: Vec<String> = sessions.iter().map(|s| s.username.clone()).collect();
        let book = self.contact_book(wcdb, &names);
        for s in sessions.iter_mut() {
            if let Some(c) = book.get(&s.username) {
                if s.display_name == s.username {
                    if let Some(n) = c.display_name.clone().filter(|n| !n.is_empty()) {
                        s.display_name = n;
                    }
                }
                if let Some(a) = c.avatar_url.clone() {
                    s.avatar_url = Some(a);
                }
            }
        }
        Ok(sessions)
    }

    /// `getContactLabelNameMap`: label id → name from the contact database (empty when it has no labels).
    fn contact_label_names(&self, wcdb: &weflow_native::wcdb::Wcdb) -> HashMap<i64, String> {
        let mut map = HashMap::new();
        let Ok(info) = wcdb.exec_query(
            "contact",
            "",
            "select name from pragma_table_info('contact_label')",
        ) else {
            return map;
        };
        let columns: Vec<String> = info
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|r| r["name"].as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        let id = crate::contact_extra::pick_column(
            &columns,
            &["label_id_", "label_id", "labelId", "labelid", "id"],
        );
        let name = crate::contact_extra::pick_column(
            &columns,
            &[
                "label_name_",
                "label_name",
                "labelName",
                "labelname",
                "name",
            ],
        );
        let (Some(id), Some(name)) = (id, name) else {
            return map;
        };
        let sql =
            format!("select \"{id}\" as label_id, \"{name}\" as label_name from contact_label");
        for row in wcdb
            .exec_query("contact", "", &sql)
            .ok()
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
        {
            let (Some(id), name) = (
                row["label_id"].as_i64(),
                first_str(&row, &["label_name"]).trim().to_string(),
            ) else {
                continue;
            };
            if id > 0 && !name.is_empty() {
                map.insert(id, name);
            }
        }
        map
    }

    /// `chat contacts --parsed` (#205): friends / groups / official accounts / former friends, newest contact first,
    /// each with its `avatarUrl`, for a contact list that shows the faces without asking for them one by one.
    pub fn chat_contacts_parsed(&self) -> AppResult<Vec<Contact>> {
        let wcdb = self.open_wcdb()?;
        let raw = wcdb
            .contacts()
            .map_err(|e| AppError::native(e.to_string()))?;
        let rows = raw.as_array().cloned().unwrap_or_default();
        let mut last_contact: HashMap<String, i64> = HashMap::new();
        if let Ok(Value::Array(sessions)) = wcdb.sessions() {
            for s in sessions {
                let u = first_str(&s, &["username", "user_name", "userName"]);
                let ts = js_int(
                    ["sort_timestamp", "sortTimestamp"]
                        .iter()
                        .filter_map(|k| s.get(*k))
                        .find(|v| truthy(v)),
                )
                .unwrap_or(0);
                if !u.is_empty() && ts != 0 {
                    last_contact.insert(u, ts);
                }
            }
        }
        let label_names = self.contact_label_names(&wcdb);
        let mut contacts: Vec<(Contact, i64)> = Vec::new();
        for row in &rows {
            let username = first_str(row, &["username"]).trim().to_string();
            if username.is_empty() {
                continue;
            }
            let local_type = row_int(row, &["local_type", "localType", "WCDB_CT_local_type"], 0);
            let quan_pin = first_str(row, &["quan_pin", "quanPin", "WCDB_CT_quan_pin"])
                .trim()
                .to_string();
            let lowered = username.to_lowercase();
            let openim = is_enterprise_openim(&username);
            if openim && !allowed_openim_by_local_type(&username, Some(local_type)) {
                continue;
            }
            let visible_weixin = lowered.starts_with("weixin") && lowered != "weixin";
            let kind = if username.ends_with("@chatroom") {
                ContactKind::Group
            } else if username.starts_with("gh_") {
                ContactKind::Official
            } else if openim
                || visible_weixin
                || (local_type == 1 && !FRIEND_EXCLUDE_USERNAMES.contains(&username.as_str()))
            {
                ContactKind::Friend
            } else if local_type == 0 && !quan_pin.is_empty() {
                ContactKind::FormerFriend
            } else {
                continue;
            };
            let remark = first_str(row, &["remark"]);
            let nick = first_str(row, &["nick_name"]);
            let alias = first_str(row, &["alias"]);
            let display = [&remark, &nick, &alias]
                .into_iter()
                .find(|s| !s.is_empty())
                .cloned()
                .unwrap_or_else(|| username.clone());
            let set = |text: String| Some(text).filter(|t| !t.is_empty());
            let labels = crate::contact_extra::contact_labels(row, &label_names);
            let t = last_contact.get(&username).copied().unwrap_or(0);
            let contact = Contact {
                display_name: display,
                remark: set(remark),
                nickname: set(nick),
                alias: set(alias),
                labels: Some(labels).filter(|l| !l.is_empty()),
                detail_description: set(crate::contact_extra::contact_signature(row)),
                region: set(crate::contact_extra::contact_region(row)),
                kind,
                avatar_url: set(first_str(row, &["big_head_url", "small_head_url"])),
                username,
            };
            contacts.push((contact, t));
        }
        contacts.sort_by(|a, b| {
            let (ta, tb) = (a.1, b.1);
            if ta != 0 && tb != 0 {
                return tb.cmp(&ta);
            }
            if ta != 0 {
                return std::cmp::Ordering::Less;
            }
            if tb != 0 {
                return std::cmp::Ordering::Greater;
            }
            crate::collate::compare_zh(&a.0.display_name, &b.0.display_name)
        });
        Ok(contacts.into_iter().map(|c| c.0).collect())
    }

    /// Display names and avatar URLs of `usernames`, each map holding the ones the account knows.
    pub(super) fn names_and_avatars_pub(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> (HashMap<String, String>, HashMap<String, String>) {
        let payload = serde_json::to_string(usernames).unwrap_or_else(|_| "[]".into());
        let to_map = |v: anyhow::Result<Value>| -> HashMap<String, String> {
            v.ok()
                .and_then(|v| v.as_object().cloned())
                .map(|m| {
                    m.into_iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string())))
                        .collect()
                })
                .unwrap_or_default()
        };
        if usernames.is_empty() {
            return (HashMap::new(), HashMap::new());
        }
        (
            to_map(wcdb.display_names(&payload)),
            to_map(wcdb.avatar_urls(&payload)),
        )
    }

    /// `downloadEmoji`: cached fetch of a sticker by CDN URL.
    async fn chat_download_emoji(&self, cdn_url: &str, md5: Option<&str>) -> AppResult<PathBuf> {
        if cdn_url.is_empty() {
            return Err(AppError::usage("invalid CDN URL"));
        }
        let key = md5
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| {
                use md5::{Digest, Md5};
                let mut h = Md5::new();
                h.update(cdn_url.as_bytes());
                h.finalize().iter().map(|b| format!("{b:02x}")).collect()
            });
        let dir = self.emoji_cache_dir();
        for ext in [".gif", ".png", ".webp", ".jpg", ".jpeg"] {
            let p = dir.join(format!("{key}{ext}"));
            if p.exists() {
                return Ok(p);
            }
        }
        let client = crate::http_client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(|e| AppError::runtime(e.to_string()))?;
        let mut url = cdn_url.to_string();
        for _ in 0..5 {
            let resp = client
                .get(&url)
                .send()
                .await
                .map_err(|e| AppError::runtime(e.to_string()))?;
            let code = resp.status().as_u16();
            if code == 301 || code == 302 {
                if let Some(loc) = resp.headers().get("location").and_then(|v| v.to_str().ok()) {
                    url = loc.to_string();
                    continue;
                }
            }
            if code != 200 {
                return Err(AppError::runtime("download failed"));
            }
            let bytes = resp
                .bytes()
                .await
                .map_err(|e| AppError::runtime(e.to_string()))?;
            if bytes.is_empty() {
                return Err(AppError::runtime("download failed"));
            }
            let ext = match crate::decrypt::detect_image_extension(&bytes) {
                ".bin" | ".wxgf" => rx(r"(?i)\.(gif|png|jpe?g|webp)(?:\?|$)")
                    .captures(&url)
                    .map(|c| format!(".{}", c[1].to_lowercase()))
                    .unwrap_or_else(|| ".gif".into()),
                e => e.to_string(),
            };
            let path = dir.join(format!("{key}{ext}"));
            std::fs::write(&path, &bytes).map_err(|e| AppError::runtime(e.to_string()))?;
            return Ok(path);
        }
        Err(AppError::runtime("download failed"))
    }

    async fn export_media_for_message(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        msg: &ChatMessage,
        talker: &str,
        safe_talker: &str,
        session_dir: &Path,
        opts: &ApiMediaOptions,
    ) -> Option<ApiExportedMedia> {
        if opts.files && is_file_message(msg.xml_type()) {
            let (source, file_name) =
                self.message_file(wcdb, msg.file_md5(), msg.file_name(), msg.create_time)?;
            let to = MediaTarget {
                session_dir,
                safe_talker,
                written: None,
            };
            return put_media(
                &to,
                ("file", "files"),
                file_name,
                MediaSource::Copy(&source),
                false,
            );
        }
        if msg.local_type == 47 && opts.emojis {
            let url = msg.emoji_cdn_url.as_deref().filter(|u| !u.is_empty())?;
            let path = self
                .chat_download_emoji(url, msg.emoji_md5.as_deref())
                .await
                .ok()?;
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_else(|| ".gif".into());
            let base = api::sanitize_file_name(
                msg.emoji_md5
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .unwrap_or(""),
                &format!("emoji_{}", msg.local_id),
            );
            let to = MediaTarget {
                session_dir,
                safe_talker,
                written: None,
            };
            return put_media(
                &to,
                ("emoji", "emojis"),
                format!("{base}{ext}"),
                MediaSource::Copy(&path),
                false,
            );
        }
        self.export_local_media(msg, talker, safe_talker, session_dir, opts, None)
    }

    /// Copies the media of `msgs` into `<out dir>/media/<out name>/{images,voices,videos,emojis,files}` and records
    /// each file on its message; the avatars asked for go to `avatars/` beside them (see [`Self::export_avatars`]).
    pub(super) async fn attach_export_media(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        msgs: &mut [crate::message::ExportMsg],
        session_id: &str,
        out: &Path,
        opts: &ApiMediaOptions,
    ) -> ExportMedia {
        use crate::message::MediaRef;
        let (session_dir, prefix) = media_dir(out);
        let wanted = |m: &crate::message::ExportMsg| match m.local_type {
            3 => opts.images,
            34 => opts.voices,
            43 => opts.videos,
            47 => opts.emojis,
            _ => opts.files && is_file_message(m.xml_type.as_deref()),
        };
        let todo: Vec<usize> = (0..msgs.len()).filter(|i| wanted(&msgs[*i])).collect();
        let in_order =
            |m: &crate::message::ExportMsg| copied_in_order(m.local_type, m.xml_type.as_deref());
        let chat_msg = |m: &crate::message::ExportMsg| ChatMessage {
            local_id: m.local_id,
            server_id: m.server_id,
            server_id_raw: m.server_id_raw.clone().unwrap_or_default(),
            local_type: m.local_type,
            create_time: m.create_time,
            image_md5: m.image_md5.clone(),
            image_dat_name: m.image_dat_name.clone(),
            video_md5: m.video_md5.clone(),
            emoji_cdn_url: m.emoji_cdn_url.clone(),
            emoji_md5: m.emoji_md5.clone(),
            // what finds a file message's file
            t49: is_file_message(m.xml_type.as_deref()).then(|| {
                Box::new(chat_msg::Type49Info {
                    xml_type: m.xml_type.clone(),
                    file_name: m.file_name.clone(),
                    file_md5: m.file_md5.clone(),
                    ..Default::default()
                })
            }),
            ..Default::default()
        };
        let safe = api::sanitize_file_name(session_id, "session");
        let total = todo.len();
        let mut results: Vec<Option<ApiExportedMedia>> = (0..total).map(|_| None).collect();
        // one counter for both passes, so the progress never steps back
        let finished = std::sync::atomic::AtomicUsize::new(0);
        {
            let local: Vec<usize> = (0..total).filter(|k| !in_order(&msgs[todo[*k]])).collect();
            let workers = std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(1, 8)
                .min(local.len().max(1));
            let next = std::sync::atomic::AtomicUsize::new(0);
            let found = std::sync::Mutex::new(Vec::<(usize, Option<ApiExportedMedia>)>::new());
            let written = ExportedFiles::default();
            let shared: &[crate::message::ExportMsg] = msgs;
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| loop {
                        let n = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&k) = local.get(n) else { break };
                        let r = self.export_local_media(
                            &chat_msg(&shared[todo[k]]),
                            session_id,
                            &safe,
                            &session_dir,
                            opts,
                            Some(&written),
                        );
                        found.lock_or_panic().push((k, r));
                        let done = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        self.emit_progress("export", "copying media", done, total);
                    });
                }
            });
            for (k, r) in found.into_inner_or_panic() {
                results[k] = r;
            }
            // An image whose own file is not on disk is still exported when another message shows the same image
            // (same md5) and that one was found first: its result is remembered (`image_state`). Which message came
            // first depends on the threads, so look again now that all of them are done.
            for &k in &local {
                if results[k].is_none() && shared[todo[k]].local_type == 3 {
                    results[k] = self.export_local_media(
                        &chat_msg(&shared[todo[k]]),
                        session_id,
                        &safe,
                        &session_dir,
                        opts,
                        Some(&written),
                    );
                }
            }
        }
        for k in 0..total {
            let m = &msgs[todo[k]];
            if in_order(m) {
                results[k] = self
                    .export_media_for_message(
                        wcdb,
                        &chat_msg(m),
                        session_id,
                        &safe,
                        &session_dir,
                        opts,
                    )
                    .await;
                let done = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                self.emit_progress("export", "copying media", done, total);
            }
        }
        let mut exported = 0usize;
        for (k, r) in results.into_iter().enumerate() {
            let Some(r) = r else { continue };
            let sub = match r.kind {
                "image" => "images",
                "voice" => "voices",
                "video" => "videos",
                "file" => "files",
                _ => "emojis",
            };
            msgs[todo[k]].media = Some(MediaRef {
                kind: r.kind,
                relative_path: format!("{prefix}/{sub}/{}", r.file_name),
            });
            exported += 1;
        }
        let avatars = match opts.avatars {
            Some(source) => {
                self.export_avatars(wcdb, msgs, session_id, &session_dir, &prefix, source)
                    .await
            }
            None => HashMap::new(),
        };
        ExportMedia {
            copied: crate::contract::MediaCopied {
                requested: total,
                exported,
                missing: total - exported,
                dir: session_dir,
            },
            avatars,
        }
    }

    /// The avatars of who speaks in `msgs` (the user, the other side of a private chat, a group's members) written
    /// to `<session_dir>/avatars/` (see [`Self::write_avatars`]).
    async fn export_avatars(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        msgs: &[crate::message::ExportMsg],
        session_id: &str,
        session_dir: &Path,
        prefix: &str,
        source: api::AvatarSource,
    ) -> HashMap<String, String> {
        let me = self.wmy();
        let is_group = session_id.ends_with("@chatroom");
        let mut users: Vec<String> = Vec::new();
        for m in msgs {
            let user = if m.is_send {
                me.as_str()
            } else if is_group {
                m.sender_username.as_str()
            } else {
                session_id
            };
            if !user.is_empty() && !users.iter().any(|u| u == user) {
                users.push(user.to_string());
            }
        }
        self.write_avatars(wcdb, users, session_dir, prefix, source)
            .await
    }

    /// The avatars of the rows of a list (`export sessions`, `export contacts` with `--media avatar`) for its HTML
    /// page, written to `media/<out name>/avatars/` beside `out` as `write_avatars` does. Returns
    /// `{ username: path from the page }` for those written.
    pub async fn export_list_avatars(
        &self,
        users: Vec<String>,
        out: &Path,
        source: api::AvatarSource,
    ) -> AppResult<HashMap<String, String>> {
        let wcdb = self.open_wcdb()?;
        let (dir, prefix) = media_dir(out);
        Ok(self
            .write_avatars(&wcdb, users, &dir, &prefix, source)
            .await)
    }

    /// The avatars of `users` written to `<dir>/avatars/`: WeChat's copy on this computer, else, with
    /// [`AvatarSource::Download`], the picture at the contact's avatar address. Returns
    /// `{ username: <prefix>/avatars/<file> }` for those written; the others keep the page's initial.
    async fn write_avatars(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        users: Vec<String>,
        dir: &Path,
        prefix: &str,
        source: api::AvatarSource,
    ) -> HashMap<String, String> {
        use futures::StreamExt as _;
        let dir = dir.join("avatars");
        if users.is_empty() || std::fs::create_dir_all(&dir).is_err() {
            return HashMap::new();
        }
        let mut pictures: Vec<(String, Vec<u8>)> = Vec::new();
        let mut elsewhere: Vec<String> = Vec::new();
        for user in users {
            match wcdb.head_image(&user) {
                Ok(Some(bytes)) => pictures.push((user, bytes)),
                _ => elsewhere.push(user),
            }
        }
        if source == api::AvatarSource::Download && !elsewhere.is_empty() {
            let urls = serde_json::to_string(&elsewhere)
                .ok()
                .and_then(|list| wcdb.avatar_urls(&list).ok())
                .unwrap_or(Value::Null);
            let wanted: Vec<(String, String)> = elsewhere
                .into_iter()
                .filter_map(|user| {
                    let url = urls.get(&user)?.as_str()?.to_string();
                    Some((user, url))
                })
                .collect();
            let total = wanted.len();
            self.emit_progress("export", "downloading avatars", 0, total);
            let fetched: Vec<(String, Option<Vec<u8>>)> = futures::stream::iter(wanted)
                .map(|(user, url)| async move { (user, avatar_picture(&url).await) })
                .buffer_unordered(5)
                .collect()
                .await;
            self.emit_progress("export", "downloading avatars", total, total);
            pictures.extend(fetched.into_iter().filter_map(|(u, b)| Some((u, b?))));
        }
        pictures
            .into_iter()
            .filter_map(|(user, bytes)| {
                let ext = crate::image::ImageFormat::of(&bytes).map_or(".jpg", |f| f.extension());
                // named by the user's id, which may hold characters a file name cannot
                let hash: String = {
                    use md5::{Digest, Md5};
                    Md5::digest(user.as_bytes())
                        .iter()
                        .take(6)
                        .map(|b| format!("{b:02x}"))
                        .collect()
                };
                let file = format!("{hash}{ext}");
                write_atomically(&dir.join(&file), &bytes).ok()?;
                Some((user, format!("{prefix}/avatars/{file}")))
            })
            .collect()
    }

    /// Images, voices and videos: everything that needs no network, so it can run on several threads (with the
    /// files they write in `written`, see [`ExportedFiles`]).
    fn export_local_media(
        &self,
        msg: &ChatMessage,
        talker: &str,
        safe_talker: &str,
        session_dir: &Path,
        opts: &ApiMediaOptions,
        written: Option<&ExportedFiles>,
    ) -> Option<ApiExportedMedia> {
        let to = MediaTarget {
            session_dir,
            safe_talker,
            written,
        };
        let put = |kind: &'static str,
                   sub: &str,
                   file_name: String,
                   source: MediaSource<'_>,
                   thumbnail: bool| {
            put_media(&to, (kind, sub), file_name, source, thumbnail)
        };
        if msg.local_type == 3 && opts.images {
            let payload = crate::services::ImagePayload {
                session_id: Some(talker.to_string()),
                image_md5: msg.image_md5.clone().filter(|m| !m.is_empty()),
                image_dat_name: msg.image_dat_name.clone().filter(|m| !m.is_empty()),
                create_time: Some(msg.create_time).filter(|t| *t != 0),
                force: true,
                prefer_file_path: true,
                ..Default::default()
            };
            let mut path = self.image_decrypt(&payload).local_path;
            if path.is_none() {
                let cached = self.image_resolve_cache(&crate::services::ImagePayload {
                    force: false,
                    ..payload
                });
                if cached.success {
                    path = cached.local_path;
                }
            }
            let path = path?;
            let thumbnail = !path.starts_with("data:") && crate::image::is_thumbnail_path(&path);
            let bytes = if let Some(rest) = path.strip_prefix("data:") {
                use base64::Engine;
                let b64 = rest.split_once(";base64,")?.1;
                base64::engine::general_purpose::STANDARD.decode(b64).ok()?
            } else {
                std::fs::read(&path).ok()?
            };
            let ext = api_detect_image_ext(&bytes);
            let base = api::sanitize_file_name(
                msg.image_md5
                    .as_deref()
                    .filter(|s| !s.is_empty())
                    .or(msg.image_dat_name.as_deref())
                    .unwrap_or(""),
                &format!("image_{}", msg.local_id),
            );
            return put(
                "image",
                "images",
                format!("{base}{ext}"),
                MediaSource::Bytes(&bytes),
                thumbnail,
            )
            .map(|mut m| {
                m.thumbnail = thumbnail;
                m
            });
        }
        if msg.local_type == 34 && opts.voices {
            let server = if msg.server_id_raw.is_empty() || msg.server_id_raw == "0" {
                msg.server_id.to_string()
            } else {
                msg.server_id_raw.clone()
            };
            let server = if server == "0" { None } else { Some(server) };
            let wav = self
                .voice_data(
                    talker,
                    &msg.local_id.to_string(),
                    Some(msg.create_time).filter(|t| *t != 0),
                    server.as_deref(),
                    None,
                )
                .ok()?;
            return put(
                "voice",
                "voices",
                format!("voice_{}.wav", msg.local_id),
                MediaSource::Bytes(&wav),
                false,
            );
        }
        if msg.local_type == 43 && opts.videos {
            let md5 = msg.video_md5.as_deref().filter(|m| !m.is_empty())?;
            let xml_md5 = crate::video::parse_video_md5(&msg.raw_content).unwrap_or_default();
            let video = self.video_file_path(&[md5, &xml_md5])?;
            let ext = video
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| format!(".{e}"))
                .unwrap_or_else(|| ".mp4".into());
            let base = api::sanitize_file_name(md5, &format!("video_{}", msg.local_id));
            return put(
                "video",
                "videos",
                format!("{base}{ext}"),
                MediaSource::Copy(&video),
                false,
            );
        }
        None
    }
}

/// What an exported file holds, best last: a full rendition before a thumbnail, then the larger file.
type MediaRank = (bool, u64);

/// The files one media export writes, with the rank of what each holds. Messages that share an image share its
/// file, and the media threads export them side by side: whichever gets there first, the HD rendition replaces a
/// thumbnail, so the export does not depend on the threads' order (issue #52).
#[derive(Default)]
pub(super) struct ExportedFiles(std::sync::Mutex<HashMap<PathBuf, MediaRank>>);

/// Where the media of one conversation go.
struct MediaTarget<'a> {
    session_dir: &'a Path,
    safe_talker: &'a str,
    /// The files of the export this belongs to, when it exports several messages at once.
    written: Option<&'a ExportedFiles>,
}

/// What an exported media file is made from.
enum MediaSource<'a> {
    Bytes(&'a [u8]),
    Copy(&'a Path),
}

/// Write (or copy) one exported file below `<session_dir>/<sub>`; `file_name` may name a folder below that too
/// (`2024-01/report.pdf`). A file already there is kept, unless this export wrote it (`written`) from a lesser
/// rendition: then a full rendition (not a `thumbnail`), or a larger one of the same kind, replaces it.
fn put_media(
    to: &MediaTarget<'_>,
    (kind, sub): (&'static str, &str),
    file_name: String,
    source: MediaSource<'_>,
    thumbnail: bool,
) -> Option<ApiExportedMedia> {
    let (session_dir, safe_talker) = (to.session_dir, to.safe_talker);
    let full = file_name
        .split('/')
        .fold(session_dir.join(sub), |path, part| path.join(part));
    std::fs::create_dir_all(full.parent()?).ok()?;
    let size = match source {
        MediaSource::Bytes([]) => return None,
        MediaSource::Bytes(b) => b.len() as u64,
        MediaSource::Copy(src) => std::fs::metadata(src).ok()?.len(),
    };
    // renamed into place: the file is whole as soon as another thread sees it
    let write = || match source {
        MediaSource::Bytes(b) => write_atomically(&full, b),
        MediaSource::Copy(src) => copy_atomically(src, &full),
    };
    match to.written {
        None => {
            if !full.exists() {
                write().ok()?;
            }
        }
        Some(written) => {
            let rank = (!thumbnail, size);
            // decided and written under the lock, so a better rendition is never overwritten by a worse one
            let mut files = written
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            let keep = match files.get(&full) {
                Some(have) => *have >= rank,
                // left by an earlier export into the same folder: kept, as before
                None if full.exists() => {
                    files.insert(full.clone(), (true, u64::MAX));
                    true
                }
                None => false,
            };
            if !keep {
                write().ok()?;
                files.insert(full.clone(), rank);
            }
        }
    }
    Some(ApiExportedMedia {
        kind,
        file_name: file_name.clone(),
        full_path: full.to_string_lossy().to_string(),
        relative_path: format!("{safe_talker}/{sub}/{file_name}"),
        thumbnail: false,
    })
}

/// Where the media of the export written to `out` go, `<out dir>/media/<out name>`, and the same folder as the page
/// beside `out` names it, `media/<out name>`.
fn media_dir(out: &Path) -> (PathBuf, String) {
    let out_dir = out
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    let name = api::sanitize_file_name(
        &out.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default(),
        "export",
    );
    (out_dir.join("media").join(&name), format!("media/{name}"))
}

/// What [`ServiceHub::attach_export_media`] copied: the media messages' counts, and the avatars written as
/// `{ username: path from the export file }`.
pub(super) struct ExportMedia {
    pub copied: crate::contract::MediaCopied,
    pub avatars: HashMap<String, String>,
}

/// The picture at an avatar address: a `data:image/…;base64,` address read as it is, any other downloaded. `None`
/// when it holds no picture or cannot be fetched.
async fn avatar_picture(url: &str) -> Option<Vec<u8>> {
    let bytes = if let Some(rest) = url.strip_prefix("data:image/") {
        crate::sns::lenient_base64(rest.split_once(";base64,")?.1)
    } else {
        let client = crate::http_client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .ok()?;
        let resp = client.get(url).send().await.ok()?;
        if !resp.status().is_success() {
            return None;
        }
        resp.bytes().await.ok()?.to_vec()
    };
    crate::image::ImageFormat::of(&bytes)
        .is_some()
        .then_some(bytes)
}

/// A file message: an app message of type 6.
fn is_file_message(xml_type: Option<&str>) -> bool {
    xml_type == Some("6")
}

/// The media copied one after another after the threads, where the database is at hand: stickers (downloaded) and
/// files (found through WeChat's media index).
fn copied_in_order(local_type: i64, xml_type: Option<&str>) -> bool {
    local_type == 47 || is_file_message(xml_type)
}

/// httpService `detectImageExt`: the picture's extension, `.bmp` for a bitmap, `.jpg` for anything else.
fn api_detect_image_ext(b: &[u8]) -> &'static str {
    match crate::image::ImageFormat::of(b) {
        Some(format) => format.extension(),
        None if b.starts_with(b"BM") => ".bmp",
        None => ".jpg",
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64() != Some(0.0),
        _ => true,
    }
}

impl ServiceHub {
    /// `getLatestMessages`: the newest `limit` messages, oldest first (direct-offset read).
    pub fn chat_latest_messages(
        &self,
        session_id: &str,
        limit: usize,
    ) -> AppResult<Vec<ChatMessage>> {
        let wcdb = self.open_wcdb()?;
        let page = limit.max(1);
        let probe = (page + 1).min(500);
        let mut rows = wcdb
            .messages(session_id, crate::num::saturating_i32(probe), 0)
            .map_err(|e| AppError::native(e.to_string()))?;
        rows.truncate(page);
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

impl ServiceHub {
    /// `export media`: walks the messages of one conversation (or all of them) and copies each
    /// image / voice (as WAV) / video / sticker / file into `<out>/<session>/<kind>s/`, using the same
    /// per-message pipeline as the HTTP API (`media=1`). `media_type`: image, voice, video, emoji, file, all.
    /// `start` / `end` (Unix seconds, `end` exclusive) restrict the message time.
    /// `found` counts media *messages* (an image sent twice counts twice; `chat images` lists unique files).
    pub async fn export_media(
        &self,
        session: Option<&str>,
        out: &Path,
        media_type: &str,
        start: Option<i64>,
        end: Option<i64>,
    ) -> AppResult<crate::contract::MediaExport> {
        let wanted = |k: &str| media_type == "all" || media_type == k;
        if !["image", "voice", "video", "emoji", "file", "all"].contains(&media_type) {
            return Err(AppError::usage(format!(
                "unsupported media type: {media_type}; use image, voice, video, emoji, file or all"
            )));
        }
        std::fs::create_dir_all(out)
            .map_err(|e| AppError::runtime(format!("create {}: {e}", out.display())))?;
        let wcdb = self.open_wcdb()?;
        let sessions: Vec<String> = match session {
            Some(s) => vec![s.to_string()],
            None => self
                .sessions_with(&wcdb)?
                .into_iter()
                .map(|s| s.username)
                .collect(),
        };
        let opts = ApiMediaOptions {
            enabled: true,
            images: wanted("image"),
            voices: wanted("voice"),
            videos: wanted("video"),
            emojis: wanted("emoji"),
            files: wanted("file"),
            // a message has no avatar: `export messages` puts them on its page
            avatars: None,
        };
        // the local types of each kind: a file is an app message of type 6, which several local types carry
        let types: [(&[i64], &'static str, bool); 5] = [
            (&[3], "image", opts.images),
            (&[34], "voice", opts.voices),
            (&[43], "video", opts.videos),
            (&[47], "emoji", opts.emojis),
            (&super::chat::FILE_APP_TYPES, "file", opts.files),
        ];
        let my = self.wmy();

        // pass 1: collect the media messages so the progress bar knows the total
        self.emit_progress("media", "scanning messages", 0, sessions.len());
        let mut work: Vec<(String, &'static str, ChatMessage)> = Vec::new();
        for (i, sid) in sessions.iter().enumerate() {
            for &(codes, kind, enabled) in &types {
                if !enabled {
                    continue;
                }
                let mut seen: HashSet<(i64, i64)> = HashSet::new();
                for &code in codes {
                    let Ok(rows) = wcdb.messages_by_type(sid, code, false, 0, 0) else {
                        continue;
                    };
                    for m in chat_msg::map_msg_rows(rows, sid, &my) {
                        let in_range = start.is_none_or(|s| m.create_time >= s)
                            && end.is_none_or(|e| m.create_time < e);
                        let of_kind = kind != "file" || is_file_message(m.xml_type());
                        if m.local_type == code
                            && of_kind
                            && in_range
                            && seen.insert((m.local_id, m.create_time))
                        {
                            work.push((sid.clone(), kind, m));
                        }
                    }
                }
            }
            self.emit_progress("media", "scanning messages", i + 1, sessions.len());
        }

        // pass 2: export
        let total = work.len();
        let mut files: Vec<crate::contract::MediaFileExported> = Vec::new();
        let mut missing_by_kind: BTreeMap<&str, usize> = BTreeMap::new();
        let mut thumb_only = 0usize;
        // Images (WXGF decoded on the way), voices and videos have no network step: run them on a few
        // threads first, then walk the list in order for emojis and the report.
        let mut done: Vec<Option<Option<ApiExportedMedia>>> = (0..total).map(|_| None).collect();
        {
            let local: Vec<usize> = (0..total)
                .filter(|i| !copied_in_order(work[*i].2.local_type, work[*i].2.xml_type()))
                .collect();
            let workers = std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(1, 8)
                .min(local.len().max(1));
            let next = std::sync::atomic::AtomicUsize::new(0);
            let finished = std::sync::atomic::AtomicUsize::new(0);
            let results = std::sync::Mutex::new(Vec::<(usize, Option<ApiExportedMedia>)>::new());
            let written = ExportedFiles::default();
            std::thread::scope(|scope| {
                for _ in 0..workers {
                    scope.spawn(|| loop {
                        let k = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(&i) = local.get(k) else { break };
                        let (sid, _, msg) = &work[i];
                        let safe = api::sanitize_file_name(sid, "session");
                        let dir = out.join(&safe);
                        let r =
                            self.export_local_media(msg, sid, &safe, &dir, &opts, Some(&written));
                        results.lock_or_panic().push((i, r));
                        let n = finished.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                        self.emit_progress(
                            "media",
                            &format!("exporting {media_type} media"),
                            n,
                            total,
                        );
                    });
                }
            });
            for (i, r) in results.into_inner_or_panic() {
                done[i] = Some(r);
            }
            // see `attach_export_media`: an image found through another message's result, whatever the thread order
            for &i in &local {
                if matches!(done[i], Some(None)) && work[i].2.local_type == 3 {
                    let (sid, _, msg) = &work[i];
                    let safe = api::sanitize_file_name(sid, "session");
                    let dir = out.join(&safe);
                    done[i] =
                        Some(self.export_local_media(msg, sid, &safe, &dir, &opts, Some(&written)));
                }
            }
        }
        for (n, (sid, kind, msg)) in work.iter().enumerate() {
            let safe = api::sanitize_file_name(sid, "session");
            let session_dir = out.join(&safe);
            let (exported, in_parallel) = match done[n].take() {
                Some(r) => (r, true),
                None => (
                    self.export_media_for_message(&wcdb, msg, sid, &safe, &session_dir, &opts)
                        .await,
                    false,
                ),
            };
            match exported {
                Some(m) => {
                    if m.thumbnail {
                        thumb_only += 1;
                    }
                    files.push(crate::contract::MediaFileExported {
                        session: sid.clone(),
                        kind: m.kind.to_string(),
                        is_thumb: (m.kind == "image").then_some(m.thumbnail),
                        file_name: m.file_name,
                        path: m.full_path,
                        local_id: msg.local_id,
                        create_time: msg.create_time,
                    })
                }
                None => *missing_by_kind.entry(kind).or_default() += 1,
            }
            if !in_parallel {
                self.emit_progress(
                    "media",
                    &format!("exporting {media_type} media"),
                    n + 1,
                    total,
                );
            }
        }
        let missing: usize = missing_by_kind.values().sum();
        Ok(crate::contract::MediaExport {
            exported: files.len(),
            found: total,
            missing,
            missing_by_kind: missing_by_kind
                .into_iter()
                .map(|(k, n)| (k.to_string(), n))
                .collect(),
            thumb_only,
            note: "missing = media messages whose file is not on disk (not downloaded in WeChat) or could not be resolved; stickers need network access. thumbOnly = exported images that are only the thumbnail (open the original in WeChat, then export again for the HD image)".into(),
            sessions: sessions.len(),
            out: out.to_path_buf(),
            files,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media_dir(tag: &str) -> weflow_native::fixture::TestDir {
        weflow_native::fixture::TestDir::new(&format!("weflow-media-{tag}"))
    }

    fn put(to: &MediaTarget<'_>, bytes: &[u8], thumbnail: bool) -> Option<ApiExportedMedia> {
        put_media(
            to,
            ("image", "images"),
            "a.jpg".into(),
            MediaSource::Bytes(bytes),
            thumbnail,
        )
    }

    #[test]
    fn a_shared_image_file_ends_up_with_the_best_rendition_in_any_order() {
        for (tag, order) in [
            (
                "thumb-first",
                [(&b"thumb"[..], true), (&b"full image"[..], false)],
            ),
            (
                "full-first",
                [(&b"full image"[..], false), (&b"thumb"[..], true)],
            ),
        ] {
            let dir = media_dir(tag);
            let written = ExportedFiles::default();
            let to = MediaTarget {
                session_dir: &dir,
                safe_talker: "s",
                written: Some(&written),
            };
            for (bytes, thumbnail) in order {
                assert!(put(&to, bytes, thumbnail).is_some());
            }
            assert_eq!(
                std::fs::read(dir.join("images").join("a.jpg")).unwrap(),
                b"full image",
                "{tag}"
            );
            // of two full renditions the larger stays
            assert!(put(&to, b"full", false).is_some());
            assert!(put(&to, b"a larger full image", false).is_some());
            assert_eq!(
                std::fs::read(dir.join("images").join("a.jpg")).unwrap(),
                b"a larger full image"
            );
        }
    }

    #[test]
    fn a_file_from_an_earlier_export_is_kept_and_empty_data_is_not_written() {
        let dir = media_dir("earlier");
        std::fs::create_dir_all(dir.join("images")).unwrap();
        std::fs::write(dir.join("images").join("a.jpg"), b"earlier").unwrap();
        let written = ExportedFiles::default();
        let to = MediaTarget {
            session_dir: &dir,
            safe_talker: "s",
            written: Some(&written),
        };
        assert!(put(&to, b"a newer, larger image", false).is_some());
        assert_eq!(
            std::fs::read(dir.join("images").join("a.jpg")).unwrap(),
            b"earlier"
        );
        assert!(put(&to, b"", false).is_none());
        let names: Vec<_> = std::fs::read_dir(dir.join("images")).unwrap().collect();
        assert_eq!(names.len(), 1, "no temporary file is left behind");
    }
}
