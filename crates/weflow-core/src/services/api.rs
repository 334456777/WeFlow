//! Service layer behind the HTTP API: sessions, contacts, message paging, ChatLab conversion.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::*;
use crate::api::{self, ApiError, ApiExportedMedia, ApiMediaOptions, Params};
use crate::chat_msg::{self, ChatMessage};
use crate::message::{row_field, row_int, rx};

const FRIEND_EXCLUDE_USERNAMES: [&str; 5] = ["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];

fn js_int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f as i64)),
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
        42 => m.card_nickname.clone().filter(|n| !n.is_empty()).unwrap_or_else(|| "[名片]".into()),
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
    rx(r"^\s*\[视频号\]\s*").replace(&chat_msg::clean_utf16(&raw), "").trim().to_string()
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
        let Ok(wcdb) = self.open_wcdb() else { return Vec::new() };
        match wcdb.group_nicknames(chatroom_id) {
            Ok(Value::Object(m)) => m.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or("").to_string())).collect(),
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
        let avatar = wcdb
            .avatar_urls(&serde_json::to_string(&[username]).unwrap())
            .ok()
            .and_then(|m| m.get(username).and_then(Value::as_str).map(str::to_string))
            .filter(|a| !a.is_empty() && !a.contains("base64,ffd8"));
        let pick = |keys: &[&str]| contact.as_ref().and_then(|c| keys.iter().find_map(|k| c.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)));
        let name = pick(&["remark"]).or_else(|| pick(&["nickName", "nick_name"])).or_else(|| pick(&["alias"])).unwrap_or_else(|| username.to_string());
        Some((avatar, name))
    }

    fn wmy(&self) -> String {
        self.my_wxid_cleaned()
    }

    fn contacts_compact_for(&self, wcdb: &weflow_native::wcdb::Wcdb, usernames: &[String]) -> Vec<Value> {
        let payload = serde_json::to_string(usernames).unwrap_or_else(|_| "[]".into());
        match wcdb.invoke_json("wcdb_get_contacts_compact", &[weflow_native::wcdb::Arg::S(&payload)]) {
            Ok(Value::Array(rows)) => rows,
            _ => Vec::new(),
        }
    }

    /// `getNewMessages`: one ascending cursor batch starting at `min_time`.
    pub fn chat_new_messages(&self, session_id: &str, min_time: i64, limit: i32) -> AppResult<Vec<ChatMessage>> {
        let wcdb = self.open_wcdb()?;
        self.new_messages_with(&wcdb, session_id, min_time, limit)
    }

    fn new_messages_with(&self, wcdb: &weflow_native::wcdb::Wcdb, session_id: &str, min_time: i64, limit: i32) -> AppResult<Vec<ChatMessage>> {
        let cursor = wcdb.open_message_cursor(session_id, limit, true, min_time.clamp(0, i32::MAX as i64) as i32, 0, false).map_err(|e| AppError::native(e.to_string()))?;
        let fetched = wcdb.fetch_message_batch(cursor);
        let _ = wcdb.close_message_cursor(cursor);
        let (rows, _) = fetched.map_err(|e| AppError::native(e.to_string()))?;
        let mut msgs = chat_msg::map_rows(&rows.as_array().cloned().unwrap_or_default(), &self.wmy());
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
    pub fn chat_sessions_list(&self) -> AppResult<Vec<Value>> {
        let wcdb = self.open_wcdb()?;
        self.sessions_with(&wcdb)
    }

    fn sessions_with(&self, wcdb: &weflow_native::wcdb::Wcdb) -> AppResult<Vec<Value>> {
        let raw = wcdb.sessions().map_err(|e| AppError::native(e.to_string()))?;
        let rows = raw.as_array().cloned().unwrap_or_default();
        if let Some(first) = rows.first() {
            if first.get("_error").is_some() || first.get("_info").is_some() {
                let detail = first.get("_error").or_else(|| first.get("_info")).map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string())).unwrap_or_default();
                return Err(AppError::native(format!("session table error: {detail}")));
            }
        }
        let username_keys = ["username", "user_name", "userName", "usrName", "UsrName", "talker", "talker_id", "talkerId"];
        let openim_ids: Vec<String> = rows.iter().map(|r| first_str(r, &username_keys).trim().to_string()).filter(|u| is_enterprise_openim(u)).collect();
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
        let mut sessions: Vec<Map<String, Value>> = Vec::new();
        for row in &rows {
            let username = first_str(row, &username_keys);
            let mut local_type = session_local_type(row);
            if local_type.is_none() && is_enterprise_openim(&username) {
                local_type = openim_types.get(&username).copied();
            }
            if !should_keep_session(&username, local_type) {
                continue;
            }
            let sort_ts = js_int(["sort_timestamp", "sortTimestamp", "sort_time", "sortTime"].iter().filter_map(|k| row.get(*k)).find(|v| truthy(v))).unwrap_or(0);
            let last_ts = js_int(["last_timestamp", "lastTimestamp", "last_msg_time", "lastMsgTime"].iter().filter_map(|k| row.get(*k)).find(|v| truthy(v))).unwrap_or(sort_ts);
            let summary = chat_msg::clean_utf16(&first_str(row, &["summary", "digest", "last_msg", "lastMsg"]));
            let last_msg_type = js_int(row.get("last_msg_type").filter(|v| truthy(v)).or_else(|| row.get("lastMsgType").filter(|v| truthy(v)))).unwrap_or(0);
            let hint_raw = ["message_count", "messageCount", "msg_count", "msgCount", "total_count", "totalCount", "n_msg", "nMsg", "message_num", "messageNum"].iter().filter_map(|k| row.get(*k)).find(|v| !v.is_null());
            let hint = hint_raw.and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))).filter(|n: &f64| n.is_finite() && *n >= 0.0).map(|n| n.floor() as i64);

            let mut o = Map::new();
            o.insert("username".into(), json!(username));
            o.insert("type".into(), json!(js_int(row.get("type").filter(|v| truthy(v))).unwrap_or(0)));
            o.insert("unreadCount".into(), json!(js_int(["unread_count", "unreadCount", "unreadcount"].iter().filter_map(|k| row.get(*k)).find(|v| truthy(v))).unwrap_or(0)));
            o.insert("summary".into(), json!(if summary.is_empty() { message_label(last_msg_type).to_string() } else { summary }));
            o.insert("sortTimestamp".into(), json!(sort_ts));
            o.insert("lastTimestamp".into(), json!(last_ts));
            o.insert("lastMsgType".into(), json!(last_msg_type));
            if let Some(h) = hint {
                o.insert("messageCountHint".into(), json!(h));
            }
            o.insert("displayName".into(), json!(username));
            if let Some(v) = row.get("last_msg_sender").filter(|v| !v.is_null()) {
                o.insert("lastMsgSender".into(), v.clone());
            }
            if let Some(v) = row.get("last_sender_display_name").filter(|v| !v.is_null()) {
                o.insert("lastSenderDisplayName".into(), v.clone());
            }
            o.insert("selfWxid".into(), json!(my));
            sessions.push(o);
        }

        // addMissingOfficialSessions
        let existing: HashSet<String> = sessions.iter().filter_map(|s| s["username"].as_str().map(str::to_string)).collect();
        let mut existing = existing;
        if let Ok(Value::Array(contacts)) = wcdb.contacts() {
            for row in contacts {
                let username = first_str(&row, &["username"]).trim().to_string();
                if username.is_empty() || existing.contains(&username) {
                    continue;
                }
                let lowered = username.to_lowercase();
                let local_type = row_field(&row, &["local_type", "localType", "WCDB_CT_local_type"]).and_then(|v| js_int(Some(v)));
                let is_official = username.starts_with("gh_");
                let special_weixin = lowered.starts_with("weixin") && lowered != "weixin";
                let special_openim = allowed_openim_by_local_type(&username, local_type);
                if !is_official && !special_weixin && !special_openim {
                    continue;
                }
                let display = ["remark", "nick_name", "alias"].iter().map(|k| first_str(&row, &[k])).find(|s| !s.is_empty()).unwrap_or_else(|| username.clone());
                let mut o = Map::new();
                o.insert("username".into(), json!(username));
                o.insert("type".into(), json!(0));
                o.insert("unreadCount".into(), json!(0));
                o.insert("summary".into(), json!(if is_official { "查看公众号历史消息" } else { "暂无会话记录" }));
                o.insert("sortTimestamp".into(), json!(0));
                o.insert("lastTimestamp".into(), json!(0));
                o.insert("lastMsgType".into(), json!(0));
                o.insert("displayName".into(), json!(display));
                o.insert("selfWxid".into(), json!(my));
                existing.insert(username);
                sessions.push(o);
            }
        }

        // applySyntheticUnreadCounts (official accounts have no unread column)
        for s in sessions.iter_mut().filter(|s| s["username"].as_str().map_or(false, |u| u.trim().starts_with("gh_"))) {
            let name = s["username"].as_str().unwrap_or("").to_string();
            let (mut total, mut latest_snapshot) = (0i64, 0i64);
            if let Ok(Value::Array(tables)) = wcdb.message_table_stats(&name) {
                for t in tables {
                    let c = ["count", "message_count", "messageCount"].iter().filter_map(|k| t.get(*k)).find_map(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0);
                    if c.is_finite() && c > 0.0 {
                        total += c.floor() as i64;
                    }
                    let l = ["last_timestamp", "lastTimestamp", "last_time", "lastTime", "max_create_time", "maxCreateTime"].iter().filter_map(|k| t.get(*k)).find_map(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(0.0);
                    if l.is_finite() && l as i64 > latest_snapshot {
                        latest_snapshot = l.floor() as i64;
                    }
                }
            }
            let latest = s["lastTimestamp"].as_i64().unwrap_or(0).max(s["sortTimestamp"].as_i64().unwrap_or(0)).max(latest_snapshot);
            if latest > 0 {
                s.insert("lastTimestamp".into(), json!(latest));
                let sort = s["sortTimestamp"].as_i64().unwrap_or(0).max(latest);
                s.insert("sortTimestamp".into(), json!(sort));
            }
            if total > 0 {
                let hint = s.get("messageCountHint").and_then(Value::as_i64).unwrap_or(0).max(total);
                s.insert("messageCountHint".into(), json!(hint));
            }
            let mut unread = 0i64;
            let mut latest_msg: Option<ChatMessage> = None;
            let now = chrono::Utc::now().timestamp();
            if latest > 0 && (now - latest).abs() <= 10 * 60 {
                if let Ok(msgs) = self.new_messages_with(wcdb, &name, (latest - 1).max(0), 20) {
                    let fresh: Vec<&ChatMessage> = msgs.iter().filter(|m| m.create_time >= latest && m.is_send != Some(1)).collect();
                    unread = fresh.len() as i64;
                    latest_msg = fresh.last().map(|m| (*m).clone());
                }
            }
            if let Some(m) = latest_msg {
                let summary = summary_from_message(&m);
                s.insert("summary".into(), json!(summary));
                s.insert("lastMsgType".into(), json!(m.local_type));
            }
            let cur = s["unreadCount"].as_i64().unwrap_or(0);
            s.insert("unreadCount".into(), json!(cur.max(unread)));
        }
        sessions.sort_by(|a, b| {
            let key = |s: &Map<String, Value>| {
                let sort = s["sortTimestamp"].as_i64().unwrap_or(0);
                if sort != 0 {
                    sort
                } else {
                    s["lastTimestamp"].as_i64().unwrap_or(0)
                }
            };
            key(b).cmp(&key(a))
        });

        // resolve names / avatars (the desktop app fills these from its contact cache)
        let names: Vec<String> = sessions.iter().filter_map(|s| s["username"].as_str().map(str::to_string)).collect();
        let book = self.contact_book(wcdb, &names);
        for s in sessions.iter_mut() {
            let u = s["username"].as_str().unwrap_or("").to_string();
            if let Some(c) = book.get(&u) {
                if s["displayName"].as_str() == Some(u.as_str()) {
                    if let Some(n) = c.display_name.clone().filter(|n| !n.is_empty()) {
                        s.insert("displayName".into(), json!(n));
                    }
                }
                if let Some(a) = c.avatar_url.clone() {
                    let summary_pos = s.get("displayName").cloned();
                    let _ = summary_pos;
                    s.insert("avatarUrl".into(), json!(a));
                }
            }
        }
        Ok(sessions.into_iter().map(Value::Object).collect())
    }

    /// `getContacts`: friends / groups / official accounts / former friends, newest contact first.
    /// Labels, signature and region need the extended contact columns and are not filled.
    pub fn chat_contacts_list(&self) -> AppResult<Vec<Value>> {
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.contacts().map_err(|e| AppError::native(e.to_string()))?;
        let rows = raw.as_array().cloned().unwrap_or_default();
        let mut last_contact: HashMap<String, i64> = HashMap::new();
        if let Ok(Value::Array(sessions)) = wcdb.sessions() {
            for s in sessions {
                let u = first_str(&s, &["username", "user_name", "userName"]);
                let ts = js_int(["sort_timestamp", "sortTimestamp"].iter().filter_map(|k| s.get(*k)).find(|v| truthy(v))).unwrap_or(0);
                if !u.is_empty() && ts != 0 {
                    last_contact.insert(u, ts);
                }
            }
        }
        let mut contacts: Vec<(Value, i64)> = Vec::new();
        for row in &rows {
            let username = first_str(row, &["username"]).trim().to_string();
            if username.is_empty() {
                continue;
            }
            let local_type = row_int(row, &["local_type", "localType", "WCDB_CT_local_type"], 0);
            let quan_pin = first_str(row, &["quan_pin", "quanPin", "WCDB_CT_quan_pin"]).trim().to_string();
            let lowered = username.to_lowercase();
            let openim = is_enterprise_openim(&username);
            if openim && !allowed_openim_by_local_type(&username, Some(local_type)) {
                continue;
            }
            let visible_weixin = lowered.starts_with("weixin") && lowered != "weixin";
            let kind = if username.ends_with("@chatroom") {
                "group"
            } else if username.starts_with("gh_") {
                "official"
            } else if openim || visible_weixin || (local_type == 1 && !FRIEND_EXCLUDE_USERNAMES.contains(&username.as_str())) {
                "friend"
            } else if local_type == 0 && !quan_pin.is_empty() {
                "former_friend"
            } else {
                continue;
            };
            let remark = first_str(row, &["remark"]);
            let nick = first_str(row, &["nick_name"]);
            let alias = first_str(row, &["alias"]);
            let display = [&remark, &nick, &alias].into_iter().find(|s| !s.is_empty()).cloned().unwrap_or_else(|| username.clone());
            let mut o = Map::new();
            o.insert("username".into(), json!(username));
            o.insert("displayName".into(), json!(display));
            for (k, v) in [("remark", &remark), ("nickname", &nick), ("alias", &alias)] {
                if !v.is_empty() {
                    o.insert(k.into(), json!(v));
                }
            }
            o.insert("type".into(), json!(kind));
            let t = last_contact.get(&username).copied().unwrap_or(0);
            contacts.push((Value::Object(o), t));
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
            a.0["displayName"].as_str().unwrap_or("").to_lowercase().cmp(&b.0["displayName"].as_str().unwrap_or("").to_lowercase())
        });
        Ok(contacts.into_iter().map(|c| c.0).collect())
    }

    // ── HTTP API endpoints ──

    /// `GET /api/v1/sessions`
    pub fn api_sessions(&self, params: &Params) -> Result<Value, ApiError> {
        let keyword = params.get("keyword").map(|k| k.trim().to_string()).unwrap_or_default();
        let limit = api::parse_int_param(params.get("limit").map(String::as_str), 100, 1, 10000) as usize;
        let format = params.get("format").map(|f| f.trim().to_lowercase()).unwrap_or_default();
        let sessions = self.chat_sessions_list().map_err(|e| ApiError::new(500, e.message))?;
        let mut filtered: Vec<Value> = sessions;
        if !keyword.is_empty() {
            let k = keyword.to_lowercase();
            filtered.retain(|s| s["username"].as_str().unwrap_or("").to_lowercase().contains(&k) || s["displayName"].as_str().map_or(false, |d| !d.is_empty() && d.to_lowercase().contains(&k)));
        }
        filtered.truncate(limit);
        if format == "chatlab" {
            let items: Vec<Value> = filtered
                .iter()
                .map(|s| {
                    let u = s["username"].as_str().unwrap_or("");
                    let mut o = Map::new();
                    o.insert("id".into(), json!(u));
                    o.insert("name".into(), json!(s["displayName"].as_str().filter(|n| !n.is_empty()).unwrap_or(u)));
                    o.insert("platform".into(), json!("wechat"));
                    o.insert("type".into(), json!(api::api_session_type(u)));
                    if let Some(c) = s.get("messageCountHint").and_then(Value::as_i64).filter(|c| *c != 0) {
                        o.insert("messageCount".into(), json!(c));
                    }
                    o.insert("lastMessageAt".into(), s["lastTimestamp"].clone());
                    Value::Object(o)
                })
                .collect();
            return Ok(json!({ "sessions": items }));
        }
        let items: Vec<Value> = filtered
            .iter()
            .map(|s| {
                json!({
                    "username": s["username"], "displayName": s["displayName"], "type": s["type"],
                    "sessionType": api::api_session_type(s["username"].as_str().unwrap_or("")),
                    "lastTimestamp": s["lastTimestamp"], "unreadCount": s["unreadCount"]
                })
            })
            .collect();
        Ok(json!({ "success": true, "count": items.len(), "sessions": items }))
    }

    /// `GET /api/v1/contacts`
    pub fn api_contacts(&self, params: &Params) -> Result<Value, ApiError> {
        let keyword = params.get("keyword").map(|k| k.trim().to_lowercase()).unwrap_or_default();
        let limit = api::parse_int_param(params.get("limit").map(String::as_str), 100, 1, 10000) as usize;
        let contacts = self.chat_contacts_list().map_err(|e| ApiError::new(500, e.message))?;
        let mut filtered = contacts;
        if !keyword.is_empty() {
            filtered.retain(|c| ["username", "nickname", "remark", "displayName"].iter().any(|k| c.get(*k).and_then(Value::as_str).map_or(false, |v| !v.is_empty() && v.to_lowercase().contains(&keyword))));
        }
        filtered.truncate(limit);
        Ok(json!({ "success": true, "count": filtered.len(), "contacts": filtered }))
    }

    /// `GET /api/v1/group-members`
    pub fn api_group_members(&self, params: &Params) -> Result<Value, ApiError> {
        let chatroom_id = params.get("chatroomId").filter(|v| !v.trim().is_empty()).or_else(|| params.get("talker")).map(|v| v.trim().to_string()).unwrap_or_default();
        let include_counts = api::parse_bool_param(params, &["includeMessageCounts", "withCounts"], false);
        let force = api::parse_bool_param(params, &["forceRefresh"], false);
        if chatroom_id.is_empty() {
            return Err(ApiError::new(400, "Missing chatroomId"));
        }
        let (entries, from_cache, updated_at) = self.group_members_panel(&chatroom_id, force, include_counts).map_err(|e| ApiError::new(500, e.message))?;
        let members: Vec<Value> = entries
            .iter()
            .map(|m| {
                let s = |k: &str| m.get(k).and_then(Value::as_str).unwrap_or("").to_string();
                let mut o = Map::new();
                o.insert("wxid".into(), json!(s("username")));
                o.insert("displayName".into(), m.get("displayName").cloned().unwrap_or(Value::Null));
                o.insert("nickname".into(), json!(s("nickname")));
                o.insert("remark".into(), json!(s("remark")));
                o.insert("alias".into(), json!(s("alias")));
                o.insert("groupNickname".into(), json!(s("groupNickname")));
                if let Some(a) = m.get("avatarUrl") {
                    o.insert("avatarUrl".into(), a.clone());
                }
                o.insert("isOwner".into(), json!(m.get("isOwner").and_then(Value::as_bool).unwrap_or(false)));
                o.insert("isFriend".into(), json!(m.get("isFriend").and_then(Value::as_bool).unwrap_or(false)));
                o.insert("messageCount".into(), json!(m.get("messageCount").and_then(Value::as_i64).unwrap_or(0)));
                Value::Object(o)
            })
            .collect();
        Ok(json!({ "success": true, "chatroomId": chatroom_id, "count": members.len(), "fromCache": from_cache, "updatedAt": updated_at, "members": members }))
    }

    /// `fetchMessagesBatch`: cursor paging with offset skipping; returns the raw rows.
    fn fetch_rows_batch(&self, wcdb: &weflow_native::wcdb::Wcdb, talker: &str, offset: usize, limit: usize, start: i64, end: i64, ascending: bool, lite: bool) -> Result<(Vec<Value>, bool), ApiError> {
        let batch_size = limit.clamp(500, 2000) as i32;
        let norm = |t: i64| -> i32 { (if t > 10_000_000_000 { t / 1000 } else { t }).clamp(0, i32::MAX as i64) as i32 };
        let cursor = wcdb.open_message_cursor(talker, batch_size, ascending, norm(start), norm(end), lite).map_err(|e| ApiError::new(500, e.to_string()))?;
        let mut collected: Vec<Value> = Vec::new();
        let mut has_more = true;
        let mut skipped = 0usize;
        let mut reached_limit = false;
        let result = (|| -> Result<(), ApiError> {
            while collected.len() < limit && has_more {
                let (rows, more) = wcdb.fetch_message_batch(cursor).map_err(|e| ApiError::new(500, e.to_string()))?;
                let rows = rows.as_array().cloned().unwrap_or_default();
                if rows.is_empty() {
                    has_more = false;
                    break;
                }
                has_more = more;
                let mut rows = rows;
                if skipped < offset {
                    let remaining = offset - skipped;
                    if remaining >= rows.len() {
                        skipped += rows.len();
                        continue;
                    }
                    rows.drain(..remaining);
                    skipped = offset;
                }
                let capacity = limit - collected.len();
                if rows.len() > capacity {
                    collected.extend(rows.into_iter().take(capacity));
                    reached_limit = true;
                    break;
                }
                collected.extend(rows);
            }
            Ok(())
        })();
        let _ = wcdb.close_message_cursor(cursor);
        result?;
        Ok((collected, has_more || reached_limit))
    }

    /// `backfillMissingSenderUsernames` for group chats (detail lookups are capped like the desktop app).
    fn backfill_senders(&self, wcdb: &weflow_native::wcdb::Wcdb, talker: &str, messages: &mut [ChatMessage]) {
        if !talker.ends_with("@chatroom") {
            return;
        }
        let my = self.wmy();
        let targets: Vec<usize> = messages.iter().enumerate().filter(|(_, m)| m.sender_username.as_deref().map_or(true, |s| s.trim().is_empty())).map(|(i, _)| i).collect();
        if targets.is_empty() {
            return;
        }
        if targets.len() > 120 {
            for i in targets {
                if messages[i].sender_username.is_none() && messages[i].is_send == Some(1) && !my.is_empty() {
                    messages[i].sender_username = Some(my.clone());
                }
            }
            return;
        }
        let (mut attempted, mut hydrated, mut miss) = (0usize, 0usize, 0usize);
        for i in targets {
            if attempted >= 80 || (miss >= 36 && hydrated == 0) {
                break;
            }
            let local_id = messages[i].local_id;
            if local_id > 0 {
                attempted += 1;
                match wcdb.message_by_id(talker, local_id as i32) {
                    Ok(row) if row.is_object() => {
                        if let Some(h) = chat_msg::map_rows(std::slice::from_ref(&row), &my).into_iter().next() {
                            if let Some(s) = h.sender_username.clone().filter(|s| !s.is_empty()) {
                                messages[i].sender_username = Some(s);
                            }
                            if messages[i].is_send.is_none() {
                                messages[i].is_send = h.is_send;
                            }
                            if messages[i].raw_content.is_empty() {
                                messages[i].raw_content = h.raw_content;
                            }
                        }
                        if messages[i].sender_username.is_some() {
                            hydrated += 1;
                            miss = 0;
                        } else {
                            miss += 1;
                        }
                    }
                    _ => miss += 1,
                }
            }
            if messages[i].sender_username.is_none() && messages[i].is_send == Some(1) && !my.is_empty() {
                messages[i].sender_username = Some(my.clone());
            }
        }
    }

    fn convert_to_chatlab(&self, wcdb: &weflow_native::wcdb::Wcdb, messages: &[ChatMessage], talker: &str, talker_name: &str, media: &HashMap<i64, ApiExportedMedia>, base_url: &str) -> Value {
        let is_group = talker.ends_with("@chatroom");
        let my = self.wmy();
        let my_norm = api::normalize_account_id(&my).to_lowercase();
        let senders: Vec<String> = {
            let mut seen = HashSet::new();
            messages.iter().filter_map(|m| m.sender_username.clone()).filter(|s| !s.is_empty() && seen.insert(s.clone())).collect()
        };
        let (names, _) = self.names_and_avatars_pub(wcdb, &senders);
        let group_nicks: HashMap<String, String> = if is_group {
            match wcdb.group_nicknames(talker) {
                Ok(Value::Object(m)) => crate::export_msg::build_trusted_group_nicknames(m.into_iter().map(|(k, v)| (k, v.as_str().unwrap_or("").to_string()))),
                _ => HashMap::new(),
            }
        } else {
            HashMap::new()
        };
        let mut member_order: Vec<String> = Vec::new();
        let mut members: HashMap<String, Map<String, Value>> = HashMap::new();
        for m in messages {
            let info = api::resolve_chatlab_sender_info(m, talker, &my, is_group, &names, &group_nicks);
            if !members.contains_key(&info.sender) {
                let mut o = Map::new();
                o.insert("platformId".into(), json!(info.sender));
                o.insert("accountName".into(), json!(info.account_name));
                if let Some(g) = info.group_nickname {
                    o.insert("groupNickname".into(), json!(g));
                }
                member_order.push(info.sender.clone());
                members.insert(info.sender, o);
            }
        }
        // avatars
        let lookup: Vec<String> = {
            let mut seen = HashSet::new();
            member_order
                .iter()
                .filter(|s| !s.starts_with("unknown_sender_"))
                .flat_map(|s| {
                    let n = s.trim().to_string();
                    let c = api::normalize_account_id(&n);
                    if !c.is_empty() && c != n {
                        vec![n, c]
                    } else {
                        vec![n]
                    }
                })
                .filter(|s| !s.is_empty() && seen.insert(s.clone()))
                .collect()
        };
        let mut avatar_map: HashMap<String, String> = HashMap::new();
        if !lookup.is_empty() {
            if let Ok(Value::Object(m)) = wcdb.avatar_urls(&serde_json::to_string(&lookup).unwrap()) {
                for (u, url) in m {
                    let (u, url) = (u.trim().to_string(), url.as_str().unwrap_or("").trim().to_string());
                    if u.is_empty() || url.is_empty() {
                        continue;
                    }
                    let cleaned = api::normalize_account_id(&u);
                    for k in [u.clone(), u.to_lowercase(), cleaned.clone(), cleaned.to_lowercase()] {
                        if !k.is_empty() {
                            avatar_map.insert(k, url.clone());
                        }
                    }
                }
            }
        }
        let my_avatar = if my.is_empty() {
            None
        } else {
            let list: Vec<String> = {
                let mut v = vec![my.clone(), clean_account_dir_name(&my), "self".to_string()];
                v.dedup();
                v
            };
            wcdb.avatar_urls(&serde_json::to_string(&list).unwrap()).ok().and_then(|m| list.iter().find_map(|k| m.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)))
        };
        for sender in &member_order {
            if sender.starts_with("unknown_sender_") {
                continue;
            }
            let norm = api::normalize_account_id(sender).to_lowercase();
            let is_self = !my_norm.is_empty() && !norm.is_empty() && norm == my_norm;
            let candidates: Vec<&str> = if is_self { vec![sender.as_str(), my.as_str()] } else { vec![sender.as_str()] };
            let url = is_self.then(|| my_avatar.clone()).flatten().or_else(|| {
                candidates.iter().find_map(|c| {
                    let n = c.trim();
                    if n.is_empty() {
                        return None;
                    }
                    let cleaned = api::normalize_account_id(n);
                    avatar_map.get(n).or_else(|| avatar_map.get(&n.to_lowercase())).or_else(|| avatar_map.get(&cleaned)).or_else(|| avatar_map.get(&cleaned.to_lowercase())).cloned()
                })
            });
            if let (Some(u), Some(m)) = (url, members.get_mut(sender)) {
                m.insert("avatar".into(), json!(u));
            }
        }
        let group_avatar = if is_group { wcdb.avatar_urls(&serde_json::to_string(&[talker]).unwrap()).ok().and_then(|m| m.get(talker).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string)) } else { None };

        let chat_messages: Vec<Value> = messages
            .iter()
            .map(|m| {
                let info = api::resolve_chatlab_sender_info(m, talker, &my, is_group, &names, &group_nicks);
                let quote = api::extract_api_quote_info(m);
                let mut o = Map::new();
                o.insert("sender".into(), json!(info.sender));
                o.insert("accountName".into(), json!(info.account_name));
                if let Some(g) = info.group_nickname {
                    o.insert("groupNickname".into(), json!(g));
                }
                o.insert("timestamp".into(), json!(m.create_time));
                o.insert("type".into(), json!(api::map_message_type(m)));
                o.insert("content".into(), api::message_content(m, quote.as_ref()).map(Value::from).unwrap_or(Value::Null));
                let sid = api::message_server_id(m);
                if !sid.is_empty() {
                    o.insert("platformMessageId".into(), json!(sid));
                }
                if let Some(id) = quote.as_ref().and_then(|q| q.reply_to_message_id.clone()) {
                    o.insert("replyToMessageId".into(), json!(id));
                }
                if let Some(md) = media.get(&m.local_id) {
                    o.insert("mediaPath".into(), json!(format!("{}/api/v1/media/{}", base_url.trim_end_matches('/'), md.relative_path)));
                }
                Value::Object(o)
            })
            .collect();

        let mut meta = Map::new();
        meta.insert("name".into(), json!(talker_name));
        meta.insert("platform".into(), json!("wechat"));
        meta.insert("type".into(), json!(api::api_session_type(talker)));
        if is_group {
            meta.insert("groupId".into(), json!(talker));
            if let Some(a) = group_avatar {
                meta.insert("groupAvatar".into(), json!(a));
            }
        }
        if !my.is_empty() {
            meta.insert("ownerId".into(), json!(my));
        }
        json!({
            "chatlab": { "version": "0.0.2", "exportedAt": chrono::Utc::now().timestamp(), "generator": "WeFlow" },
            "meta": meta,
            "members": member_order.iter().filter_map(|s| members.get(s).cloned()).map(Value::Object).collect::<Vec<_>>(),
            "messages": chat_messages
        })
    }

    pub(super) fn names_and_avatars_pub(&self, wcdb: &weflow_native::wcdb::Wcdb, usernames: &[String]) -> (HashMap<String, String>, HashMap<String, String>) {
        let payload = serde_json::to_string(usernames).unwrap_or_else(|_| "[]".into());
        let to_map = |v: anyhow::Result<Value>| -> HashMap<String, String> { v.ok().and_then(|v| v.as_object().cloned()).map(|m| m.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_string()))).collect()).unwrap_or_default() };
        if usernames.is_empty() {
            return (HashMap::new(), HashMap::new());
        }
        (to_map(wcdb.display_names(&payload)), to_map(wcdb.avatar_urls(&payload)))
    }

    /// `GET /api/v1/messages`
    pub async fn api_messages(&self, params: &Params, base_url: &str) -> Result<Value, ApiError> {
        let talker = params.get("talker").map(|t| t.trim().to_string()).unwrap_or_default();
        let limit = api::parse_int_param(params.get("limit").map(String::as_str), 100, 1, 10000) as usize;
        let offset = api::parse_int_param(params.get("offset").map(String::as_str), 0, 0, i64::MAX) as usize;
        let keyword = params.get("keyword").map(|k| k.trim().to_string()).unwrap_or_default();
        let chatlab = api::parse_bool_param(params, &["chatlab"], false);
        let format_param = params.get("format").map(|f| f.trim().to_lowercase()).unwrap_or_default();
        let format = if format_param.is_empty() { if chatlab { "chatlab".to_string() } else { "json".to_string() } } else { format_param };
        let media_opts = api::parse_media_options(params);
        if talker.is_empty() {
            return Err(ApiError::new(400, "Missing required parameter: talker"));
        }
        if format != "json" && format != "chatlab" {
            return Err(ApiError::new(400, "Invalid format, supported: json/chatlab"));
        }
        let start = api::parse_time_param(params.get("start").map(String::as_str), false);
        let end = api::parse_time_param(params.get("end").map(String::as_str), true);
        let wcdb = self.open_wcdb().map_err(|e| ApiError::new(500, e.message))?;
        let my = self.wmy();
        let (mut messages, has_more);
        if !keyword.is_empty() {
            let rows = wcdb.search(&keyword, Some(&talker), (limit + 1) as i32, offset as i32, start as i32, end as i32).map_err(|e| ApiError::new(500, e.to_string()))?;
            let rows = rows.as_array().cloned().unwrap_or_default();
            let mut mapped = chat_msg::map_rows(&rows, &my);
            has_more = mapped.len() > limit;
            if has_more {
                mapped.truncate(limit);
            }
            messages = mapped;
            if talker.ends_with("@chatroom") {
                for m in messages.iter_mut() {
                    if m.local_id > 0 && (m.sender_username.is_none() || m.is_send.is_none()) {
                        if let Ok(row) = wcdb.message_by_id(&talker, m.local_id as i32) {
                            if row.is_object() {
                                if let Some(d) = chat_msg::map_rows(std::slice::from_ref(&row), &my).into_iter().next() {
                                    let (parsed, raw) = (m.parsed_content.clone(), m.raw_content.clone());
                                    *m = d;
                                    if !parsed.is_empty() {
                                        m.parsed_content = parsed;
                                    }
                                    if !raw.is_empty() {
                                        m.raw_content = raw;
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            let (rows, more) = self.fetch_rows_batch(&wcdb, &talker, offset, limit, start, end, false, !media_opts.enabled)?;
            messages = if media_opts.enabled { chat_msg::map_rows(&rows, &my) } else { chat_msg::map_rows_lite(&rows, &my) };
            self.backfill_senders(&wcdb, &talker, &mut messages);
            has_more = more;
        }

        let media_map = if media_opts.enabled { self.export_media_for_messages(&wcdb, &messages, &talker, &media_opts).await } else { HashMap::new() };
        let (names, _) = self.names_and_avatars_pub(&wcdb, std::slice::from_ref(&talker));
        let talker_name = names.get(&talker).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| talker.clone());
        let media_info = json!({ "enabled": media_opts.enabled, "exportPath": self.api_media_dir(), "count": media_map.len() });
        if format == "chatlab" {
            let mut data = self.convert_to_chatlab(&wcdb, &messages, &talker, &talker_name, &media_map, base_url);
            data.as_object_mut().unwrap().insert("media".into(), media_info);
            return Ok(data);
        }
        let api_messages: Vec<Value> = messages.iter().map(|m| api::to_api_message(m, media_map.get(&m.local_id), base_url)).collect();
        Ok(json!({ "success": true, "talker": talker, "count": api_messages.len(), "hasMore": has_more, "media": media_info, "messages": api_messages }))
    }

    /// `GET /api/v1/sessions/:id/messages` (ChatLab Pull)
    pub async fn api_pull_messages(&self, session_id: &str, params: &Params, base_url: &str) -> Result<Value, ApiError> {
        const PULL_MAX: i64 = 5000;
        let limit = api::parse_int_param(params.get("limit").map(String::as_str), PULL_MAX, 1, PULL_MAX) as usize;
        let offset = api::parse_int_param(params.get("offset").map(String::as_str), 0, 0, i64::MAX) as usize;
        let start = params.get("since").filter(|s| !s.is_empty()).map(|s| api::parse_time_param(Some(s), false)).unwrap_or(0);
        let end = params.get("end").filter(|s| !s.is_empty()).map(|s| api::parse_time_param(Some(s), true)).unwrap_or(0);
        let wcdb = self.open_wcdb().map_err(|e| ApiError::new(500, e.message))?;
        let (rows, has_more) = self.fetch_rows_batch(&wcdb, session_id, offset, limit, start, end, true, true)?;
        let mut messages = chat_msg::map_rows_lite(&rows, &self.wmy());
        self.backfill_senders(&wcdb, session_id, &mut messages);
        let (names, _) = self.names_and_avatars_pub(&wcdb, &[session_id.to_string()]);
        let talker_name = names.get(session_id).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| session_id.to_string());
        let mut data = self.convert_to_chatlab(&wcdb, &messages, session_id, &talker_name, &HashMap::new(), base_url);
        let last_ts = messages.last().map(|m| m.create_time);
        let mut sync = Map::new();
        sync.insert("hasMore".into(), json!(has_more));
        if has_more {
            if let Some(ts) = last_ts.filter(|t| *t != 0) {
                sync.insert("nextSince".into(), json!(ts));
            }
            sync.insert("nextOffset".into(), json!(offset + messages.len()));
        }
        sync.insert("watermark".into(), json!(chrono::Utc::now().timestamp()));
        data.as_object_mut().unwrap().insert("sync".into(), Value::Object(sync));
        Ok(data)
    }

    /// `downloadEmoji`: cached fetch of a sticker by CDN URL.
    pub async fn chat_download_emoji(&self, cdn_url: &str, md5: Option<&str>) -> AppResult<PathBuf> {
        if cdn_url.is_empty() {
            return Err(AppError::usage("invalid CDN URL"));
        }
        let key = md5.filter(|m| !m.is_empty()).map(str::to_string).unwrap_or_else(|| {
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
        let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().map_err(|e| AppError::runtime(e.to_string()))?;
        let mut url = cdn_url.to_string();
        for _ in 0..5 {
            let resp = client.get(&url).send().await.map_err(|e| AppError::runtime(e.to_string()))?;
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
            let bytes = resp.bytes().await.map_err(|e| AppError::runtime(e.to_string()))?;
            if bytes.is_empty() {
                return Err(AppError::runtime("download failed"));
            }
            let ext = match crate::decrypt::detect_image_extension(&bytes) {
                ".bin" | ".wxgf" => rx(r"(?i)\.(gif|png|jpe?g|webp)(?:\?|$)").captures(&url).map(|c| format!(".{}", c[1].to_lowercase())).unwrap_or_else(|| ".gif".into()),
                e => e.to_string(),
            };
            let path = dir.join(format!("{key}{ext}"));
            std::fs::write(&path, &bytes).map_err(|e| AppError::runtime(e.to_string()))?;
            return Ok(path);
        }
        Err(AppError::runtime("download failed"))
    }

    /// `exportMediaForMessages`: copies message media into `api-media/<session>/…`.
    /// Images, videos and stickers are supported; voice needs the SILK decoder.
    pub(super) async fn export_media_for_messages(&self, wcdb: &weflow_native::wcdb::Wcdb, messages: &[ChatMessage], talker: &str, opts: &ApiMediaOptions) -> HashMap<i64, ApiExportedMedia> {
        let mut map = HashMap::new();
        if !opts.enabled || messages.is_empty() {
            return map;
        }
        let safe_talker = api::sanitize_file_name(talker, "session");
        let session_dir = self.api_media_dir().join(&safe_talker);
        let _ = std::fs::create_dir_all(&session_dir);
        for msg in messages {
            if let Some(m) = self.export_media_for_message(wcdb, msg, &safe_talker, &session_dir, opts).await {
                map.insert(msg.local_id, m);
            }
        }
        map
    }

    async fn export_media_for_message(&self, wcdb: &weflow_native::wcdb::Wcdb, msg: &ChatMessage, safe_talker: &str, session_dir: &Path, opts: &ApiMediaOptions) -> Option<ApiExportedMedia> {
        let _ = wcdb;
        let put = |kind: &'static str, sub: &str, file_name: String, bytes: Option<&[u8]>, copy_from: Option<&Path>| -> Option<ApiExportedMedia> {
            let dir = session_dir.join(sub);
            std::fs::create_dir_all(&dir).ok()?;
            let full = dir.join(&file_name);
            if !full.exists() {
                match (bytes, copy_from) {
                    (Some(b), _) => std::fs::write(&full, b).ok()?,
                    (None, Some(src)) => {
                        std::fs::copy(src, &full).ok()?;
                    }
                    _ => return None,
                }
            }
            Some(ApiExportedMedia { kind, file_name: file_name.clone(), full_path: full.to_string_lossy().to_string(), relative_path: format!("{safe_talker}/{sub}/{file_name}") })
        };
        if msg.local_type == 3 && opts.images {
            let found = self.resolve_message_image(msg)?;
            let ext = crate::decrypt::detect_image_extension(&found).to_string();
            let ext = if ext == ".bin" { ".jpg".to_string() } else { ext };
            let base = api::sanitize_file_name(msg.image_md5.as_deref().filter(|s| !s.is_empty()).or(msg.image_dat_name.as_deref()).unwrap_or(""), &format!("image_{}", msg.local_id));
            return put("image", "images", format!("{base}{ext}"), Some(&found), None);
        }
        if msg.local_type == 43 && opts.videos {
            let md5 = msg.video_md5.as_deref().filter(|m| !m.is_empty())?;
            let video = self.video_file_path(md5)?;
            let ext = video.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_else(|| ".mp4".into());
            let base = api::sanitize_file_name(md5, &format!("video_{}", msg.local_id));
            return put("video", "videos", format!("{base}{ext}"), None, Some(&video));
        }
        if msg.local_type == 47 && opts.emojis {
            let url = msg.emoji_cdn_url.as_deref().filter(|u| !u.is_empty())?;
            let path = self.chat_download_emoji(url, msg.emoji_md5.as_deref()).await.ok()?;
            let ext = path.extension().and_then(|e| e.to_str()).map(|e| format!(".{e}")).unwrap_or_else(|| ".gif".into());
            let base = api::sanitize_file_name(msg.emoji_md5.as_deref().filter(|s| !s.is_empty()).unwrap_or(""), &format!("emoji_{}", msg.local_id));
            return put("emoji", "emojis", format!("{base}{ext}"), None, Some(&path));
        }
        None
    }

    /// Finds and decrypts the `.dat` file of an image message (by md5 / dat name).
    fn resolve_message_image(&self, msg: &ChatMessage) -> Option<Vec<u8>> {
        let (account_dir, _, _) = self.connection_inputs().ok()?;
        let profile = self.profile().ok()?;
        let xor_key = profile.image_xor_key.map(|k| k as u8).unwrap_or(0);
        let aes: Option<[u8; 16]> = profile.image_aes_key.as_deref().and_then(|k| {
            if k.len() < 32 {
                return None;
            }
            let mut arr = [0u8; 16];
            for i in 0..16 {
                arr[i] = u8::from_str_radix(&k[i * 2..i * 2 + 2], 16).ok()?;
            }
            Some(arr)
        });
        let tokens: Vec<String> = [msg.image_md5.clone(), msg.image_dat_name.clone()].into_iter().flatten().map(|t| t.to_lowercase()).filter(|t| !t.is_empty()).collect();
        if tokens.is_empty() {
            return None;
        }
        for entry in crate::media::scan_image_files(&account_dir) {
            let name = entry.path.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
            if !tokens.iter().any(|t| name.starts_with(t.as_str())) {
                continue;
            }
            if name.ends_with(".dat") {
                if let Ok(res) = crate::decrypt::decrypt_file(&entry.path, xor_key, aes.as_ref()) {
                    return Some(res.data);
                }
            } else if let Ok(bytes) = std::fs::read(&entry.path) {
                return Some(bytes);
            }
        }
        None
    }
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::String(s) => !s.is_empty(),
        Value::Number(n) => n.as_f64().map_or(true, |f| f != 0.0),
        _ => true,
    }
}
