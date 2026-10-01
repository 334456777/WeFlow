//! Chat queries beyond the basics: message lookup, per-day statistics, session details,
//! export statistics, resource listings and cached hints.

use std::collections::{HashMap, HashSet};

use serde_json::{json, Map, Value};

use super::{clean_account_dir_name, ServiceHub};
use crate::error::{AppError, AppResult};
use crate::message::{collect_messages, extract_arkme_app_message_meta, is_same_wxid, CollectOptions, ExportMsg};

const FRIEND_EXCLUDE: &[&str] = &["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];
const FILE_APP_TYPES: [i64; 4] = [49, 34_359_738_417, 103_079_215_153, 25_769_803_825];

fn native(e: anyhow::Error) -> AppError {
    AppError::native(e.to_string())
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
    ["username", "userName", "user_name", "encryptUsername", "encryptUserName", "encrypt_username", "originalName"]
        .iter()
        .find_map(|k| member.get(*k).and_then(Value::as_str).filter(|s| !s.trim().is_empty()))
        .unwrap_or("")
        .trim()
        .to_string()
}

fn member_list(value: &Value) -> Vec<Value> {
    value.as_array().cloned().or_else(|| value.get("members").and_then(Value::as_array).cloned()).unwrap_or_default()
}

fn num(v: &Value, key: &str) -> i64 {
    v.get(key)
        .and_then(|x| x.as_i64().or_else(|| x.as_f64().map(|f| f as i64)).or_else(|| x.as_str().and_then(|s| s.parse().ok())))
        .unwrap_or(0)
        .max(0)
}

fn basename(path: &str) -> String {
    path.rsplit(['/', '\\']).next().unwrap_or("").to_string()
}

#[derive(Clone, Debug, Default)]
pub struct ResourceQuery {
    pub session_id: Option<String>,
    pub types: Vec<String>,
    pub begin: i64,
    pub end: i64,
    pub limit: usize,
    pub offset: usize,
}

impl ServiceHub {
    pub(super) fn my_wxid_cleaned(&self) -> String {
        let raw = self.wxid_override.clone().or_else(|| self.profile().ok().and_then(|p| p.wxid.clone())).unwrap_or_default();
        clean_account_dir_name(&raw)
    }

    fn normalize_row(&self, rows: &[Value], session_id: &str) -> Vec<ExportMsg> {
        let my = self.my_wxid_cleaned();
        collect_messages(rows, &CollectOptions { session_id, my_wxid: &my, start: None, end: None, sender_filter: None })
    }

    // ── single messages ──

    pub fn chat_message_by_id(&self, session_id: &str, local_id: i32) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let row = wcdb.message_by_id(session_id, local_id).map_err(native)?;
        self.lookup_result(row, session_id)
    }

    pub fn chat_message_by_server_id(&self, session_id: &str, server_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let row = wcdb.message_by_server_id(session_id, server_id).map_err(native)?;
        self.lookup_result(row, session_id)
    }

    fn lookup_result(&self, row: Value, session_id: &str) -> AppResult<Value> {
        if row.is_null() || row.as_object().map_or(true, |o| o.is_empty()) {
            return Err(AppError::runtime("message not found"));
        }
        let my = self.my_wxid_cleaned();
        let message = self.normalize_row(std::slice::from_ref(&row), session_id).into_iter().next().map(|m| m.to_json(&my));
        Ok(json!({ "row": row, "message": message }))
    }

    // ── per-day statistics ──

    pub fn chat_dates(&self, session_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let dates = wcdb.message_dates(session_id).unwrap_or_else(|_| json!([]));
        Ok(json!({ "sessionId": session_id, "dates": dates }))
    }

    pub fn chat_date_counts(&self, session_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.session_message_date_counts(session_id).map_err(native)?;
        let mut counts = Map::new();
        if let Some(obj) = raw.as_object() {
            for (date, value) in obj {
                let n = value.as_f64().or_else(|| value.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0);
                if !date.is_empty() && n.is_finite() && n > 0.0 {
                    counts.insert(date.clone(), json!(n.floor() as i64));
                }
            }
        }
        Ok(json!({ "sessionId": session_id, "counts": counts }))
    }

    pub fn chat_counts(&self, session_ids: &[String]) -> AppResult<Value> {
        let ids = uniq(session_ids);
        if ids.is_empty() {
            return Ok(json!({}));
        }
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.session_message_counts(&ids).map_err(native)?;
        let mut counts = Map::new();
        for id in &ids {
            counts.insert(id.clone(), json!(num(&raw, id)));
        }
        Ok(Value::Object(counts))
    }

    pub fn chat_statuses(&self, usernames: &[String]) -> AppResult<Value> {
        let ids = uniq(usernames);
        if ids.is_empty() {
            return Ok(json!({}));
        }
        let wcdb = self.open_wcdb()?;
        let raw = wcdb.contact_status(&serde_json::to_string(&ids).unwrap()).map_err(native)?;
        let mut out = Map::new();
        for id in &ids {
            let state = raw.get(id).cloned().unwrap_or(Value::Null);
            out.insert(
                id.clone(),
                json!({
                    "isFolded": state.get("isFolded").and_then(Value::as_bool).unwrap_or(false),
                    "isMuted": state.get("isMuted").and_then(Value::as_bool).unwrap_or(false)
                }),
            );
        }
        Ok(Value::Object(out))
    }

    pub fn chat_mark_all_read(&self) -> AppResult<Value> {
        self.open_wcdb()?.mark_all_sessions_read().map_err(native)
    }

    pub fn chat_anti_revoke_sessions(&self) -> AppResult<Value> {
        let sessions = self.sessions()?;
        let kept: Vec<Value> = sessions
            .as_array()
            .map(|a| {
                a.iter()
                    .filter(|s| {
                        let name = ["username", "userName", "talker", "sessionId"].iter().find_map(|k| s.get(*k).and_then(Value::as_str)).unwrap_or("");
                        !name.starts_with("gh_")
                    })
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        Ok(Value::Array(kept))
    }

    // ── session detail ──

    pub fn chat_detail_fast(&self, session_id: &str) -> AppResult<Value> {
        let id = session_id.trim();
        if id.is_empty() {
            return Err(AppError::usage("session id cannot be empty"));
        }
        let wcdb = self.open_wcdb()?;
        let contact = wcdb.contact(id).ok().filter(|v| v.as_object().map_or(false, |o| !o.is_empty()));
        let field = |keys: &[&str]| {
            contact
                .as_ref()
                .and_then(|c| keys.iter().find_map(|k| c.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty())))
                .map(str::to_string)
        };
        let remark = field(&["remark"]);
        let nick = field(&["nickName", "nick_name", "nickname"]);
        let alias = field(&["alias"]);
        let display = remark.clone().or(nick.clone()).or(alias.clone()).unwrap_or_else(|| id.to_string());
        let avatar = wcdb
            .avatar_urls(&serde_json::to_string(&[id]).unwrap())
            .ok()
            .and_then(|m| m.get(id).and_then(Value::as_str).map(str::to_string))
            .filter(|u| u.starts_with("http") || u.starts_with("data:"));
        let count = wcdb.message_count(id).map(|c| c.max(0)).unwrap_or(0);
        let mut o = Map::new();
        o.insert("wxid".into(), json!(id));
        o.insert("displayName".into(), json!(display));
        if let Some(v) = remark { o.insert("remark".into(), json!(v)); }
        if let Some(v) = nick { o.insert("nickName".into(), json!(v)); }
        if let Some(v) = alias { o.insert("alias".into(), json!(v)); }
        if let Some(v) = avatar { o.insert("avatarUrl".into(), json!(v)); }
        o.insert("messageCount".into(), json!(count));
        Ok(Value::Object(o))
    }

    pub fn chat_detail_extra(&self, session_id: &str) -> AppResult<Value> {
        let id = session_id.trim();
        if id.is_empty() {
            return Err(AppError::usage("session id cannot be empty"));
        }
        let wcdb = self.open_wcdb()?;
        let stats = wcdb.message_table_stats(id).unwrap_or_else(|_| json!([]));
        let mut tables: Vec<Value> = Vec::new();
        let mut first: Option<i64> = None;
        let mut last: Option<i64> = None;
        for row in stats.as_array().into_iter().flatten() {
            let db_path = row.get("db_path").and_then(Value::as_str).unwrap_or("");
            tables.push(json!({
                "dbName": basename(db_path),
                "tableName": row.get("table_name").and_then(Value::as_str).unwrap_or(""),
                "count": num(row, "count")
            }));
            let pick = |keys: &[&str]| keys.iter().map(|k| num(row, k)).find(|v| *v > 0).unwrap_or(0);
            let f = pick(&["first_timestamp", "firstTimestamp", "first_time", "firstTime", "min_create_time", "minCreateTime"]);
            if f > 0 && first.map_or(true, |x| f < x) {
                first = Some(f);
            }
            let l = pick(&["last_timestamp", "lastTimestamp", "last_time", "lastTime", "max_create_time", "maxCreateTime"]);
            if l > 0 && last.map_or(true, |x| l > x) {
                last = Some(l);
            }
        }
        Ok(json!({ "firstMessageTime": first, "latestMessageTime": last, "messageTables": tables }))
    }

    pub fn chat_detail(&self, session_id: &str) -> AppResult<Value> {
        let mut detail = self.chat_detail_fast(session_id)?;
        let extra = self.chat_detail_extra(session_id).unwrap_or_else(|_| json!({ "messageTables": [] }));
        if let (Some(d), Some(e)) = (detail.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                d.insert(k.clone(), v.clone());
            }
        }
        Ok(detail)
    }

    // ── export statistics ──

    pub fn chat_tab_counts(&self) -> AppResult<Value> {
        self.contact_type_counts()
    }

    fn friend_identities(&self, wcdb: &weflow_native::wcdb::Wcdb) -> HashSet<String> {
        let mut set = HashSet::new();
        let contacts = wcdb.contacts().unwrap_or(Value::Null);
        for row in contacts.as_array().into_iter().flatten() {
            let username = row.get("username").and_then(Value::as_str).unwrap_or("").trim();
            if username.is_empty() || username.contains("@chatroom") || username.starts_with("gh_") || FRIEND_EXCLUDE.contains(&username) {
                continue;
            }
            let local_type = ["local_type", "localType", "WCDB_CT_local_type"].iter().map(|k| num(row, k)).find(|_| true).unwrap_or(0);
            let lt = ["local_type", "localType", "WCDB_CT_local_type"]
                .iter()
                .find_map(|k| row.get(*k).map(|v| v.as_i64().or_else(|| v.as_str().and_then(|s| s.parse().ok())).unwrap_or(0)))
                .unwrap_or(local_type);
            if lt != 1 {
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
            let name = ["username", "userName", "talker", "sessionId"].iter().find_map(|k| row.get(*k).and_then(Value::as_str)).unwrap_or("").trim();
            if name.ends_with("@chatroom") && !out.iter().any(|o| o == name) {
                out.push(name.to_string());
            }
        }
        out
    }

    /// `getExportSessionStats`: message-type counts plus group/private relations.
    pub fn chat_export_stats(&self, session_ids: &[String], begin: i64, end: i64, include_relations: bool) -> AppResult<Value> {
        let ids = uniq(session_ids);
        if ids.is_empty() {
            return Ok(json!({ "data": {} }));
        }
        let wcdb = self.open_wcdb()?;
        let begin = crate::message::normalize_timestamp_seconds(begin as f64);
        let end = crate::message::normalize_timestamp_seconds(end as f64);
        let options = json!({
            "begin": begin,
            "end": end,
            "quick_mode": !include_relations && ids.len() > 1,
            "include_group_sender_count": true
        });
        let native_rows = wcdb.session_message_type_stats_batch(&serde_json::to_string(&ids).unwrap(), &options.to_string()).unwrap_or(Value::Null);
        let groups: Vec<String> = ids.iter().filter(|i| i.ends_with("@chatroom")).cloned().collect();
        let privates: Vec<String> = ids.iter().filter(|i| !i.ends_with("@chatroom")).cloned().collect();
        let member_counts = if groups.is_empty() {
            Value::Null
        } else {
            wcdb.group_member_counts(&serde_json::to_string(&groups).unwrap()).unwrap_or(Value::Null)
        };

        let mut private_mutual: HashMap<String, i64> = HashMap::new();
        let mut group_mutual: HashMap<String, i64> = HashMap::new();
        if include_relations {
            let self_set: HashSet<String> = identity_keys(&self.my_wxid_cleaned()).into_iter().collect();
            let mut relation_groups: Vec<String> = Vec::new();
            if !privates.is_empty() {
                relation_groups = self.group_session_ids(&wcdb);
                for g in &groups {
                    if !relation_groups.contains(g) {
                        relation_groups.push(g.clone());
                    }
                }
            } else if !groups.is_empty() {
                relation_groups = groups.clone();
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
                        let Some(canonical) = keys.first().cloned() else { continue };
                        if !self_set.contains(&canonical) && friends.contains(&canonical) {
                            friend_members.insert(canonical);
                        }
                        for key in &keys {
                            for sid in private_index.get(key).into_iter().flatten() {
                                touched.insert(sid.clone());
                            }
                        }
                    }
                    group_mutual.insert(gid.clone(), friend_members.len() as i64);
                    for sid in touched {
                        *private_mutual.entry(sid).or_insert(0) += 1;
                    }
                }
            }
        }

        let mut data = Map::new();
        for id in &ids {
            let row = native_rows.get(id).cloned().unwrap_or(Value::Null);
            let mut o = Map::new();
            o.insert("totalMessages".into(), json!(num(&row, "total_messages")));
            o.insert("voiceMessages".into(), json!(num(&row, "voice_messages")));
            o.insert("imageMessages".into(), json!(num(&row, "image_messages")));
            o.insert("videoMessages".into(), json!(num(&row, "video_messages")));
            o.insert("emojiMessages".into(), json!(num(&row, "emoji_messages")));
            o.insert("transferMessages".into(), json!(num(&row, "transfer_messages")));
            o.insert("redPacketMessages".into(), json!(num(&row, "red_packet_messages")));
            o.insert("callMessages".into(), json!(num(&row, "call_messages")));
            let first = num(&row, "first_timestamp");
            let last = num(&row, "last_timestamp");
            if first > 0 { o.insert("firstTimestamp".into(), json!(first)); }
            if last > 0 { o.insert("lastTimestamp".into(), json!(last)); }
            if id.ends_with("@chatroom") {
                let my = num(&row, "group_my_messages");
                o.insert("groupMyMessages".into(), json!(my));
                o.insert("groupActiveSpeakers".into(), json!(num(&row, "group_sender_count")));
                o.insert("groupMemberCount".into(), json!(num(&member_counts, id)));
                if include_relations {
                    o.insert("groupMutualFriends".into(), json!(group_mutual.get(id).copied().unwrap_or(0)));
                }
                if begin <= 0 && end <= 0 {
                    let _ = self.set_group_hint(id, my);
                }
            } else if include_relations {
                o.insert("privateMutualGroups".into(), json!(private_mutual.get(id).copied().unwrap_or(0)));
            }
            data.insert(id.clone(), Value::Object(o));
        }
        Ok(json!({ "data": data }))
    }

    // ── group "my message count" hint (persisted) ──

    fn hint_path(&self) -> std::path::PathBuf {
        let who = {
            let w = self.my_wxid_cleaned();
            if w.is_empty() { "default".to_string() } else { w }
        };
        self.ctx.cache_dir().join(format!("group_my_message_counts_{who}.json"))
    }

    fn read_hints(&self) -> Map<String, Value> {
        std::fs::read_to_string(self.hint_path())
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default()
    }

    pub fn set_group_hint(&self, chatroom_id: &str, count: i64) -> AppResult<Value> {
        if !chatroom_id.trim().ends_with("@chatroom") {
            return Err(AppError::usage("invalid group chat id"));
        }
        let mut hints = self.read_hints();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
        hints.insert(chatroom_id.trim().to_string(), json!({ "messageCount": count.max(0), "updatedAt": now }));
        let path = self.hint_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::runtime(format!("failed to create {}: {e}", parent.display())))?;
        }
        std::fs::write(&path, serde_json::to_vec_pretty(&Value::Object(hints)).unwrap()).map_err(|e| AppError::runtime(format!("failed to write {}: {e}", path.display())))?;
        Ok(json!({ "updatedAt": now }))
    }

    pub fn get_group_hint(&self, chatroom_id: &str) -> AppResult<Value> {
        if !chatroom_id.trim().ends_with("@chatroom") {
            return Err(AppError::usage("invalid group chat id"));
        }
        Ok(match self.read_hints().get(chatroom_id.trim()) {
            Some(entry) => json!({ "count": entry["messageCount"], "updatedAt": entry["updatedAt"], "source": "disk" }),
            None => json!({}),
        })
    }

    // ── resources ──

    fn resource_type(msg: &ExportMsg) -> Option<&'static str> {
        match msg.local_type {
            3 => Some("image"),
            43 => Some("video"),
            34 => Some("voice"),
            // every file app type is a file; a type-49 app message only when it says so (the metadata parse is
            // the expensive part, so it comes last)
            t if FILE_APP_TYPES.contains(&t) => {
                let is_file = t != 49
                    || msg.xml_type.as_deref() == Some("6")
                    || extract_arkme_app_message_meta(&msg.content, t).is_some_and(|m| m.get("appMsgKind").and_then(Value::as_str) == Some("file"));
                is_file.then_some("file")
            }
            _ => None,
        }
    }

    fn session_display_names(&self, wcdb: &weflow_native::wcdb::Wcdb, ids: &[String]) -> HashMap<String, String> {
        let mut out = HashMap::new();
        if ids.is_empty() {
            return out;
        }
        if let Ok(map) = wcdb.display_names(&serde_json::to_string(ids).unwrap()) {
            for (k, v) in map.as_object().into_iter().flatten() {
                if let Some(name) = v.as_str().filter(|s| !s.is_empty()) {
                    out.insert(k.clone(), name.to_string());
                }
            }
        }
        out
    }

    /// `getResourceMessages`: images / videos / voice / files across sessions.
    pub fn chat_resources(&self, q: &ResourceQuery) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let all_types = ["image", "video", "voice", "file"];
        let wanted: Vec<&str> = if q.types.is_empty() { all_types.to_vec() } else { all_types.iter().copied().filter(|t| q.types.iter().any(|x| x == t)).collect() };
        let limit = if q.limit == 0 { 300 } else { q.limit.clamp(1, 2000) };
        let offset = q.offset;
        let sessions = wcdb.sessions().map_err(native)?;
        let mut session_rows: Vec<(String, i64)> = Vec::new();
        for s in sessions.as_array().into_iter().flatten() {
            let name = ["username", "userName", "talker", "sessionId"].iter().find_map(|k| s.get(*k).and_then(Value::as_str)).unwrap_or("").to_string();
            if !name.is_empty() {
                session_rows.push((name, num(s, "sort_timestamp").max(num(s, "sortTimestamp"))));
            }
        }
        session_rows.sort_by(|a, b| b.1.cmp(&a.1));
        let requested = q.session_id.clone().unwrap_or_default();
        let targets: Vec<String> = if requested.trim().is_empty() { session_rows.iter().map(|s| s.0.clone()).collect() } else { vec![requested.trim().to_string()] };
        let mut local_types: Vec<i64> = Vec::new();
        if wanted.contains(&"image") { local_types.push(3); }
        if wanted.contains(&"video") { local_types.push(43); }
        if wanted.contains(&"voice") { local_types.push(34); }
        if wanted.contains(&"file") { local_types.extend(FILE_APP_TYPES); }
        local_types.dedup();
        let ranged = q.begin > 0 || q.end > 0;
        let target_count = offset + limit;
        let per_type = if !requested.trim().is_empty() { (target_count * 2).clamp(200, 2000) } else if ranged { 140 } else { 90 };
        let max_scan = if !requested.trim().is_empty() { 1 } else if ranged { 240 } else { 80 };
        let scan: Vec<String> = targets.iter().take(max_scan).cloned().collect();
        let mut maybe_more = targets.len() > scan.len();
        let names = self.session_display_names(&wcdb, &scan);
        let my = self.my_wxid_cleaned();
        let mut items: Vec<(i64, i64, Value)> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for sid in &scan {
            for lt in &local_types {
                let rows = wcdb.messages_by_type(sid, *lt, false, per_type as i32, 0).unwrap_or(Value::Null);
                let rows = rows.as_array().cloned().unwrap_or_default();
                if rows.len() >= per_type {
                    maybe_more = true;
                }
                let msgs = collect_messages(&rows, &CollectOptions { session_id: sid, my_wxid: &my, start: None, end: None, sender_filter: None });
                for m in msgs {
                    let Some(kind) = Self::resource_type(&m) else { continue };
                    if !wanted.contains(&kind) {
                        continue;
                    }
                    if q.begin > 0 && m.create_time < q.begin {
                        continue;
                    }
                    if q.end > 0 && m.create_time > q.end {
                        continue;
                    }
                    let key = format!("{sid}:{}:{}:{}:{}", m.local_id, m.server_id, m.create_time, m.local_type);
                    if !seen.insert(key) {
                        continue;
                    }
                    let mut j = m.to_json(&my);
                    if let Some(o) = j.as_object_mut() {
                        o.insert("sessionId".into(), json!(sid));
                        o.insert("sessionDisplayName".into(), json!(names.get(sid).cloned().unwrap_or_else(|| sid.clone())));
                        o.insert("resourceType".into(), json!(kind));
                    }
                    items.push((m.create_time, m.local_id, j));
                }
            }
        }
        items.sort_by(|a, b| b.0.cmp(&a.0).then(b.1.cmp(&a.1)));
        let total = items.len();
        let start = offset.min(total);
        let stop = (start + limit).min(total);
        let page: Vec<Value> = items[start..stop].iter().map(|i| i.2.clone()).collect();
        Ok(json!({ "items": page, "total": total, "hasMore": stop < total || maybe_more }))
    }

    /// `getAllImageMessages`: de-duplicated image identifiers of a session, newest first.
    pub fn chat_all_images(&self, session_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let rows = wcdb.messages_by_type(session_id, 3, false, 0, 0).map_err(native)?;
        let msgs = self.normalize_row(rows.as_array().map(Vec::as_slice).unwrap_or(&[]), session_id);
        let mut images: Vec<(i64, Value)> = msgs
            .into_iter()
            .filter(|m| m.local_type == 3 && (m.image_md5.is_some() || m.image_dat_name.is_some()))
            .map(|m| (m.create_time, json!({ "imageMd5": m.image_md5, "imageDatName": m.image_dat_name, "createTime": if m.create_time > 0 { json!(m.create_time) } else { Value::Null } })))
            .collect();
        images.sort_by(|a, b| b.0.cmp(&a.0));
        let mut seen = HashSet::new();
        let images: Vec<Value> = images
            .into_iter()
            .filter(|(_, v)| {
                let key = v["imageMd5"].as_str().or_else(|| v["imageDatName"].as_str()).unwrap_or("").to_string();
                !key.is_empty() && seen.insert(key)
            })
            .map(|(_, v)| v)
            .collect();
        Ok(json!({ "images": images }))
    }

    /// `getAllVoiceMessages`
    pub fn chat_all_voices(&self, session_id: &str) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let rows = wcdb.messages_by_type(session_id, 34, false, 0, 0).map_err(native)?;
        let my = self.my_wxid_cleaned();
        let mut msgs = self.normalize_row(rows.as_array().map(Vec::as_slice).unwrap_or(&[]), session_id);
        msgs.sort_by(|a, b| b.create_time.cmp(&a.create_time));
        let mut seen = HashSet::new();
        let list: Vec<Value> = msgs
            .into_iter()
            .filter(|m| seen.insert(format!("{}-{}-{}", m.server_id, m.local_id, m.create_time)))
            .map(|m| m.to_json(&my))
            .collect();
        Ok(json!({ "messages": list }))
    }

    /// `getMediaStream`: image/video messages across sessions, paged by the native scanner.
    pub fn chat_media_stream(&self, session_id: Option<&str>, media_type: &str, begin: i64, end: i64, limit: i32, offset: i32) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let ids: Vec<String> = match session_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => vec![s.to_string()],
            None => crate::services::extract_session_ids(&wcdb.sessions().map_err(native)?),
        };
        let code = match media_type {
            "image" => 1,
            "video" => 2,
            _ => 0,
        };
        let (rows, has_more) = wcdb
            .scan_media_stream(&serde_json::to_string(&ids).unwrap(), code, begin as i32, end as i32, limit, offset)
            .map_err(native)?;
        let my = self.my_wxid_cleaned();
        let names = self.session_display_names(&wcdb, &ids);
        let mut items: Vec<Value> = Vec::new();
        for row in rows.as_array().into_iter().flatten() {
            let sid = ["session_id", "sessionId", "username", "talker"].iter().find_map(|k| row.get(*k).and_then(Value::as_str)).unwrap_or(session_id.unwrap_or("")).to_string();
            let msgs = collect_messages(std::slice::from_ref(row), &CollectOptions { session_id: &sid, my_wxid: &my, start: None, end: None, sender_filter: None });
            let Some(m) = msgs.into_iter().next() else { continue };
            let kind = if m.local_type == 43 { "video" } else { "image" };
            let mut j = m.to_json(&my);
            if let Some(o) = j.as_object_mut() {
                o.insert("sessionId".into(), json!(sid));
                o.insert("sessionDisplayName".into(), json!(names.get(&sid).cloned().unwrap_or_else(|| sid.clone())));
                o.insert("mediaType".into(), json!(kind));
            }
            items.push(j);
        }
        Ok(json!({ "items": items, "hasMore": has_more, "nextOffset": offset as usize + items.len() }))
    }

    /// `resolveTransferDisplayNames`
    pub fn chat_transfer_names(&self, chatroom_id: &str, payer: &str, receiver: &str) -> AppResult<Value> {
        use crate::export_msg::{build_trusted_group_nicknames, resolve_group_nickname};
        let wcdb = self.open_wcdb()?;
        let nicks = if chatroom_id.ends_with("@chatroom") {
            let v = wcdb.group_nicknames(chatroom_id).unwrap_or(Value::Null);
            let obj = v.get("nicknames").and_then(Value::as_object).or_else(|| v.as_object());
            build_trusted_group_nicknames(obj.map(|o| o.iter().filter_map(|(k, n)| n.as_str().map(|s| (k.clone(), s.to_string()))).collect::<Vec<_>>()).unwrap_or_default())
        } else {
            HashMap::new()
        };
        let raw_me = self.wxid_override.clone().or_else(|| self.profile().ok().and_then(|p| p.wxid.clone())).unwrap_or_default();
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
            match wcdb.contact(username).ok().filter(|v| v.as_object().map_or(false, |o| !o.is_empty())) {
                Some(c) => ["remark", "nickName", "nick_name", "alias"].iter().find_map(|k| c.get(*k).and_then(Value::as_str).filter(|s| !s.is_empty())).unwrap_or(username).to_string(),
                None => username.to_string(),
            }
        };
        Ok(json!({ "payerName": resolve(payer), "receiverName": resolve(receiver) }))
    }

    /// Whether two account identifiers refer to the same user.
    pub fn same_account(a: &str, b: &str) -> bool {
        is_same_wxid(a, b)
    }
}
