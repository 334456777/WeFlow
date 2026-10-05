//! Group analytics (`electron/services/groupAnalyticsService.ts`): members panel, rankings,
//! activity, media mix, per-member analytics and exports.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::*;
use crate::chat_msg;
use crate::message::rx;

const PANEL_CACHE_TTL: Duration = Duration::from_secs(10 * 60);
const FRIEND_EXCLUDE: [&str; 5] = ["medianote", "floatbottle", "qmessage", "qqmail", "fmessage"];

#[derive(Default)]
pub(crate) struct GroupState {
    panel: HashMap<String, (Instant, i64, Vec<Value>)>,
}

#[derive(Clone, Debug, Default)]
struct MemberContact {
    remark: String,
    nick_name: String,
    alias: String,
    username: String,
    user_name: String,
    encrypt_username: String,
    encrypt_user_name: String,
    local_type: i64,
}

fn pick_str(row: &Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(v) = row.get(*k).filter(|v| !v.is_null()) {
            let t = match v {
                Value::String(s) => s.trim().to_string(),
                other => other.to_string().trim().to_string(),
            };
            if !t.is_empty() {
                return t;
            }
        }
    }
    String::new()
}

fn pick_int(row: &Value, keys: &[&str], fallback: i64) -> i64 {
    for k in keys {
        if let Some(v) = row.get(*k) {
            if v.is_null() || v.as_str() == Some("") {
                continue;
            }
            let n = v
                .as_f64()
                .or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()));
            if let Some(n) = n.filter(|n| n.is_finite()) {
                return n.floor() as i64;
            }
        }
    }
    fallback
}

fn non_neg_int(v: &Value) -> i64 {
    v.as_f64()
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .filter(|n: &f64| n.is_finite())
        .map(|n| n.floor().max(0.0) as i64)
        .unwrap_or(0)
}

pub(crate) fn build_id_candidates(values: &[&str]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for raw in values {
        let raw = raw.trim();
        if raw.is_empty() {
            continue;
        }
        if seen.insert(raw.to_string()) {
            out.push(raw.to_string());
        }
        let cleaned = clean_account_dir_name(raw);
        if !cleaned.is_empty() && cleaned != raw && seen.insert(cleaned.clone()) {
            out.push(cleaned);
        }
    }
    out
}

fn same_identity(left: &str, right: &str) -> bool {
    let l: Vec<String> = build_id_candidates(&[left])
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    let r: Vec<String> = build_id_candidates(&[right])
        .into_iter()
        .map(|s| s.to_lowercase())
        .collect();
    if l.is_empty() || r.is_empty() {
        return false;
    }
    for lc in &l {
        if r.contains(lc) {
            return true;
        }
        for rc in &r {
            if lc.starts_with(&format!("{rc}_")) || rc.starts_with(&format!("{lc}_")) {
                return true;
            }
        }
    }
    false
}

fn normalize_group_nickname(v: &str) -> String {
    let t = v.trim();
    if t.is_empty() || rx(r#"^["'@]+$"#).is_match(t) {
        String::new()
    } else {
        t.to_string()
    }
}

/// `buildTrustedGroupNicknameMap` (group analytics flavour, with candidate filtering).
fn trusted_nicknames(
    entries: &Map<String, Value>,
    candidates: &[String],
) -> HashMap<String, String> {
    let candidate_set: HashSet<String> = candidates
        .iter()
        .map(|c| c.trim().to_lowercase())
        .filter(|c| !c.is_empty())
        .collect();
    let mut buckets: HashMap<String, HashSet<String>> = HashMap::new();
    for (id, nick) in entries {
        let identity = id.trim().to_lowercase();
        if identity.is_empty() || (!candidate_set.is_empty() && !candidate_set.contains(&identity))
        {
            continue;
        }
        let nickname = normalize_group_nickname(nick.as_str().unwrap_or(""));
        if nickname.is_empty() {
            continue;
        }
        buckets.entry(identity).or_default().insert(nickname);
    }
    buckets
        .into_iter()
        .filter_map(|(k, v)| {
            if v.len() == 1 {
                v.into_iter().next().map(|n| (k, n))
            } else {
                None
            }
        })
        .collect()
}

fn resolve_nickname(map: &HashMap<String, String>, candidates: &[String]) -> String {
    let mut resolved = String::new();
    for id in candidates {
        let key = id.trim().to_lowercase();
        if key.is_empty() {
            continue;
        }
        let cand = normalize_group_nickname(map.get(&key).map(String::as_str).unwrap_or(""));
        if cand.is_empty() {
            continue;
        }
        if resolved.is_empty() {
            resolved = cand;
        } else if resolved != cand {
            return String::new();
        }
    }
    resolved
}

// ── owner detection ──

fn resolve_member_username(candidate: &Value, lookup: &HashMap<String, String>) -> Option<String> {
    let raw = candidate.as_str()?.trim();
    if raw.is_empty() {
        return None;
    }
    if let Some(v) = lookup.get(raw) {
        return Some(v.clone());
    }
    let cleaned = clean_account_dir_name(raw);
    if let Some(v) = lookup.get(&cleaned) {
        return Some(v.clone());
    }
    for part in rx(r"[,\s;|]+").split(raw).filter(|p| !p.is_empty()) {
        if let Some(v) = lookup.get(part) {
            return Some(v.clone());
        }
        if let Some(v) = lookup.get(&clean_account_dir_name(part)) {
            return Some(v.clone());
        }
    }
    if (raw.starts_with('{') || raw.starts_with('[')) && raw.len() < 4096 {
        return serde_json::from_str::<Value>(raw)
            .ok()
            .and_then(|p| extract_owner(&p, lookup, 0));
    }
    None
}

fn extract_owner(value: &Value, lookup: &HashMap<String, String>, depth: u32) -> Option<String> {
    if depth > 4 {
        return None;
    }
    match value {
        Value::Null => None,
        Value::String(_) => resolve_member_username(value, lookup),
        Value::Array(items) => items
            .iter()
            .find_map(|i| extract_owner(i, lookup, depth + 1)),
        Value::Object(row) => {
            for (key, entry) in row {
                let k = key.to_lowercase();
                if !k.contains("owner") && !k.contains("host") && !k.contains("creator") {
                    continue;
                }
                if let Value::Bool(b) = entry {
                    if *b {
                        if let Some(u) = row.get("username").filter(|u| u.is_string()) {
                            if let Some(owner) = resolve_member_username(u, lookup) {
                                return Some(owner);
                            }
                        }
                    }
                    continue;
                }
                if let Some(owner) = extract_owner(entry, lookup, depth + 1) {
                    return Some(owner);
                }
            }
            None
        }
        _ => None,
    }
}

/// CSV cell escaping (`escapeCsvValue`)
fn csv_cell(v: &str) -> String {
    if v.contains(['"', ',', '\n', '\r']) {
        format!("\"{}\"", v.replace('"', "\"\""))
    } else {
        v.to_string()
    }
}

fn simple_type_name(local_type: i64) -> String {
    match local_type {
        1 => "文本".into(),
        3 => "图片".into(),
        34 => "语音".into(),
        42 => "名片".into(),
        43 => "视频".into(),
        47 => "表情".into(),
        48 => "位置".into(),
        49 => "链接/文件".into(),
        50 => "通话".into(),
        10000 => "系统".into(),
        266287972401 => "拍一拍".into(),
        8594229559345 => "红包".into(),
        8589934592049 => "转账".into(),
        other => format!("类型({other})"),
    }
}

fn format_unix_time(ts: i64) -> String {
    use chrono::{Local, TimeZone};
    if ts <= 0 {
        return String::new();
    }
    let ms = if ts > 1_000_000_000_000 {
        ts
    } else {
        ts * 1000
    };
    match Local.timestamp_millis_opt(ms).single() {
        Some(d) => d.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => ts.to_string(),
    }
}

fn worksheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| {
            if matches!(c, '*' | '?' | ':' | '\\' | '/' | '[' | ']') {
                '_'
            } else {
                c
            }
        })
        .collect();
    let limited: String = cleaned.trim().chars().take(31).collect();
    if limited.is_empty() {
        "Sheet1".into()
    } else {
        limited
    }
}

impl ServiceHub {
    fn wrap_native<T>(&self, r: anyhow::Result<T>) -> AppResult<T> {
        r.map_err(|e| AppError::native(e.to_string()))
    }

    fn names_and_avatars(
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
        (
            to_map(wcdb.display_names(&payload)),
            to_map(wcdb.avatar_urls(&payload)),
        )
    }

    fn members_of(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
    ) -> AppResult<Vec<Value>> {
        let raw = self.wrap_native(wcdb.group_members(chatroom_id))?;
        // the DLL hands back either a bare array or `{ "members": [...] }`
        Ok(raw
            .as_array()
            .cloned()
            .or_else(|| raw.get("members").and_then(Value::as_array).cloned())
            .unwrap_or_default())
    }

    fn detect_group_owner(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
        members: &[Value],
    ) -> Option<String> {
        let mut lookup: HashMap<String, String> = HashMap::new();
        for m in members {
            let username = pick_str(m, &["username"]);
            if username.is_empty() {
                continue;
            }
            lookup.insert(clean_account_dir_name(&username), username.clone());
            lookup.insert(username.clone(), username);
        }
        if lookup.is_empty() {
            return None;
        }
        for m in members {
            if let Some(o) = extract_owner(m, &lookup, 0) {
                return Some(o);
            }
        }
        if let Ok(contact) = wcdb.contact(chatroom_id) {
            if contact.is_object() {
                if let Some(o) = extract_owner(&contact, &lookup, 0) {
                    return Some(o);
                }
            }
        }
        if let Ok(ext) = wcdb.chat_room_ext_buffer(chatroom_id) {
            let buf = pick_str(&ext, &["ext_buffer"]);
            if !buf.is_empty() {
                if let Some(o) = extract_owner(&json!({ "ext_buffer": buf }), &lookup, 0) {
                    return Some(o);
                }
            }
        }
        None
    }

    fn member_contact_lookup(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> HashMap<String, MemberContact> {
        let mut lookup: HashMap<String, MemberContact> = HashMap::new();
        let refs: Vec<&str> = usernames.iter().map(String::as_str).collect();
        let candidates = build_id_candidates(&refs);
        for batch in candidates.chunks(200) {
            let payload = serde_json::to_string(batch).unwrap();
            let Ok(Value::Array(rows)) = wcdb.invoke_json(
                "wcdb_get_contacts_compact",
                &[weflow_native::wcdb::Arg::S(&payload)],
            ) else {
                continue;
            };
            for row in rows {
                let c = MemberContact {
                    remark: pick_str(&row, &["remark", "WCDB_CT_remark"]),
                    nick_name: pick_str(&row, &["nick_name", "nickName", "WCDB_CT_nick_name"]),
                    alias: pick_str(&row, &["alias", "WCDB_CT_alias"]),
                    username: pick_str(&row, &["username", "WCDB_CT_username"]),
                    user_name: pick_str(&row, &["user_name", "userName", "WCDB_CT_user_name"]),
                    encrypt_username: pick_str(
                        &row,
                        &[
                            "encrypt_username",
                            "encryptUsername",
                            "WCDB_CT_encrypt_username",
                        ],
                    ),
                    encrypt_user_name: pick_str(
                        &row,
                        &[
                            "encrypt_user_name",
                            "encryptUserName",
                            "WCDB_CT_encrypt_user_name",
                        ],
                    ),
                    local_type: pick_int(
                        &row,
                        &["local_type", "localType", "WCDB_CT_local_type"],
                        0,
                    ),
                };
                for key in build_id_candidates(&[
                    &c.username,
                    &c.user_name,
                    &c.encrypt_username,
                    &c.encrypt_user_name,
                    &c.alias,
                ]) {
                    lookup
                        .entry(key.to_lowercase())
                        .or_insert_with(|| c.clone());
                }
            }
        }
        lookup
    }

    fn message_count_lookup(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
    ) -> HashMap<String, i64> {
        let mut lookup = HashMap::new();
        let (b, e) = normalize_range(0, 0);
        let Ok(data) = wcdb.group_stats(chatroom_id, b, e) else {
            return lookup;
        };
        let Some(senders) = data
            .pointer(&format!(
                "/sessions/{}/senders",
                escape_pointer(chatroom_id)
            ))
            .and_then(Value::as_object)
        else {
            return lookup;
        };
        let id_map = data
            .get("idMap")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        for (sender_id, raw) in senders {
            let username = id_map
                .get(sender_id)
                .and_then(Value::as_str)
                .unwrap_or(sender_id)
                .trim()
                .to_string();
            if username.is_empty() {
                continue;
            }
            let count = non_neg_int(raw);
            for key in build_id_candidates(&[&username]) {
                let entry = lookup.entry(key.to_lowercase()).or_insert(0);
                if count > *entry {
                    *entry = count;
                }
            }
        }
        lookup
    }

    fn panel_fresh(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
        include_counts: bool,
    ) -> AppResult<Vec<Value>> {
        let members = self.members_of(wcdb, chatroom_id)?;
        if members.is_empty() {
            return Ok(Vec::new());
        }
        let usernames: Vec<String> = members
            .iter()
            .map(|m| pick_str(m, &["username"]))
            .filter(|u| !u.is_empty())
            .collect();
        if usernames.is_empty() {
            return Ok(Vec::new());
        }
        let (names, _) = self.names_and_avatars(wcdb, &usernames);
        let contact_lookup = self.member_contact_lookup(wcdb, &usernames);
        let owner = self.detect_group_owner(wcdb, chatroom_id, &members);
        let counts = if include_counts {
            self.message_count_lookup(wcdb, chatroom_id)
        } else {
            HashMap::new()
        };

        let mut nick_candidates: Vec<String> = Vec::new();
        {
            let mut vals: Vec<&str> = Vec::new();
            let owned: Vec<String> = members
                .iter()
                .map(|m| pick_str(m, &["username"]))
                .chain(members.iter().map(|m| pick_str(m, &["originalName"])))
                .collect();
            vals.extend(owned.iter().map(String::as_str));
            for c in contact_lookup.values() {
                vals.extend([
                    c.username.as_str(),
                    c.user_name.as_str(),
                    c.encrypt_username.as_str(),
                    c.encrypt_user_name.as_str(),
                    c.alias.as_str(),
                ]);
            }
            nick_candidates.extend(build_id_candidates(&vals));
        }
        let nick_map = match wcdb.group_nicknames(chatroom_id) {
            Ok(Value::Object(m)) => trusted_nicknames(
                &m,
                &nick_candidates
                    .iter()
                    .map(|c| c.to_string())
                    .collect::<Vec<_>>(),
            ),
            _ => HashMap::new(),
        };
        let my_wxid = clean_account_dir_name(
            &self
                .wxid_override
                .clone()
                .or_else(|| self.profile().ok().and_then(|p| p.wxid.clone()))
                .unwrap_or_default(),
        );

        let mut data: Vec<(Value, bool, bool, i64, String)> = Vec::new();
        for member in &members {
            let wxid = pick_str(member, &["username"]);
            if wxid.is_empty() {
                continue;
            }
            let original = pick_str(member, &["originalName"]);
            let contact = build_id_candidates(&[&wxid, &original])
                .iter()
                .find_map(|id| contact_lookup.get(&id.to_lowercase()))
                .cloned();
            let (nickname, remark, alias) = contact
                .as_ref()
                .map(|c| (c.nick_name.clone(), c.remark.clone(), c.alias.clone()))
                .unwrap_or_default();
            let normalized_wxid = clean_account_dir_name(&wxid);
            let c = contact.clone().unwrap_or_default();
            let mut lookup_candidates = build_id_candidates(&[
                &wxid,
                &original,
                &c.username,
                &c.user_name,
                &c.encrypt_username,
                &c.encrypt_user_name,
                &alias,
            ]);
            if normalized_wxid == my_wxid {
                lookup_candidates.push(my_wxid.clone());
            }
            let group_nickname = resolve_nickname(&nick_map, &lookup_candidates);
            let display_name = names
                .get(&wxid)
                .filter(|n| !n.is_empty())
                .cloned()
                .unwrap_or_else(|| wxid.clone());
            let lw = wxid.to_lowercase();
            let is_friend = !(lw.is_empty()
                || lw.contains("@chatroom")
                || lw.starts_with("gh_")
                || FRIEND_EXCLUDE.contains(&lw.as_str()))
                && contact.as_ref().is_some_and(|c| c.local_type == 1);
            let is_owner = owner.as_deref() == Some(wxid.as_str());
            let count = lookup_candidates
                .iter()
                .filter_map(|id| counts.get(&id.to_lowercase()))
                .copied()
                .max()
                .unwrap_or(0);
            let mut o = Map::new();
            o.insert("username".into(), json!(wxid));
            o.insert("displayName".into(), json!(display_name));
            o.insert("nickname".into(), json!(nickname));
            o.insert("alias".into(), json!(alias));
            o.insert("remark".into(), json!(remark));
            o.insert("groupNickname".into(), json!(group_nickname));
            if let Some(a) = member.get("avatarUrl").filter(|v| !v.is_null()) {
                o.insert("avatarUrl".into(), a.clone());
            }
            o.insert("isOwner".into(), json!(is_owner));
            o.insert("isFriend".into(), json!(is_friend));
            o.insert("messageCount".into(), json!(count));
            data.push((Value::Object(o), is_owner, is_friend, count, display_name));
        }
        if include_counts && !my_wxid.is_empty() {
            if let Some((_, _, _, count, _)) = data.iter().find(|(v, ..)| {
                clean_account_dir_name(v["username"].as_str().unwrap_or("")) == my_wxid
            }) {
                let _ = self.set_group_hint(chatroom_id, *count);
            }
        }
        data.sort_by(|a, b| {
            b.1.cmp(&a.1)
                .then(b.2.cmp(&a.2))
                .then(b.3.cmp(&a.3))
                .then_with(|| crate::collate::compare_zh(&a.4, &b.4))
        });
        Ok(data.into_iter().map(|d| d.0).collect())
    }

    /// `getGroupMembersPanelData`: returns `(entries, fromCache, updatedAt)`.
    pub fn group_members_panel(
        &self,
        chatroom_id: &str,
        force_refresh: bool,
        include_counts: bool,
    ) -> AppResult<(Vec<Value>, bool, i64)> {
        let chatroom_id = chatroom_id.trim();
        if chatroom_id.is_empty() {
            return Err(AppError::usage("chatroom id must not be empty"));
        }
        let wxid = clean_account_dir_name(
            &self
                .wxid_override
                .clone()
                .or_else(|| self.profile().ok().and_then(|p| p.wxid.clone()))
                .unwrap_or_default(),
        );
        let key = format!(
            "{wxid}::{chatroom_id}::{}",
            if include_counts { "full" } else { "members" }
        );
        if !force_refresh {
            if let Some((at, updated, data)) = self.group_state.lock().unwrap().panel.get(&key) {
                if at.elapsed() < PANEL_CACHE_TTL {
                    return Ok((data.clone(), true, *updated));
                }
            }
        }
        let wcdb = self.open_wcdb()?;
        let data = self.panel_fresh(&wcdb, chatroom_id, include_counts)?;
        let updated = chrono::Utc::now().timestamp_millis();
        let mut st = self.group_state.lock().unwrap();
        st.panel
            .insert(key, (Instant::now(), updated, data.clone()));
        if st.panel.len() > 80 {
            let mut keys: Vec<(String, Instant)> =
                st.panel.iter().map(|(k, v)| (k.clone(), v.0)).collect();
            keys.sort_by_key(|(_, t)| *t);
            let remove = st.panel.len() - 80;
            for (k, _) in keys.into_iter().take(remove) {
                st.panel.remove(&k);
            }
        }
        Ok((data, false, updated))
    }

    /// `getGroupChats`
    pub fn group_chats(&self) -> AppResult<Vec<Value>> {
        let wcdb = self.open_wcdb()?;
        let sessions = self.wrap_native(wcdb.sessions())?;
        let ids: Vec<String> = sessions
            .as_array()
            .cloned()
            .unwrap_or_default()
            .iter()
            .map(|r| pick_str(r, &["username", "user_name", "userName"]))
            .filter(|u| u.contains("@chatroom"))
            .collect();
        let counts = wcdb
            .group_member_counts(&serde_json::to_string(&ids).unwrap())
            .ok()
            .and_then(|v| v.as_object().cloned())
            .unwrap_or_default();
        let book = self.contact_book(&wcdb, &ids);
        let mut groups: Vec<(Value, i64)> = ids
            .iter()
            .map(|id| {
                let c = book.get(id);
                let display = c
                    .and_then(|c| c.display_name.clone())
                    .filter(|d| !d.is_empty())
                    .unwrap_or_else(|| id.clone());
                let count = counts.get(id).and_then(Value::as_i64).unwrap_or(0);
                let mut o = Map::new();
                o.insert("username".into(), json!(id));
                o.insert("displayName".into(), json!(display));
                o.insert("memberCount".into(), json!(count));
                if let Some(a) = c.and_then(|c| c.avatar_url.clone()) {
                    o.insert("avatarUrl".into(), json!(a));
                }
                (Value::Object(o), count)
            })
            .collect();
        groups.sort_by_key(|a| std::cmp::Reverse(a.1));
        Ok(groups.into_iter().map(|g| g.0).collect())
    }

    /// `getGroupMembers` (full member records)
    pub fn group_members_full(&self, chatroom_id: &str) -> AppResult<Vec<Value>> {
        let wcdb = self.open_wcdb()?;
        self.group_members_full_with(&wcdb, chatroom_id)
    }

    fn group_members_full_with(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
    ) -> AppResult<Vec<Value>> {
        let members = self.members_of(wcdb, chatroom_id)?;
        let usernames: Vec<String> = members
            .iter()
            .map(|m| pick_str(m, &["username"]))
            .filter(|u| !u.is_empty())
            .collect();
        let (names, _) = self.names_and_avatars(wcdb, &usernames);
        let mut contacts: HashMap<String, MemberContact> = HashMap::new();
        for u in &usernames {
            let c = wcdb.contact(u).ok().filter(Value::is_object);
            let mc = match c {
                Some(c) => MemberContact {
                    remark: pick_str(&c, &["remark"]),
                    nick_name: pick_str(&c, &["nickName", "nick_name"]),
                    alias: pick_str(&c, &["alias"]),
                    username: pick_str(&c, &["username"]),
                    user_name: pick_str(&c, &["userName", "user_name"]),
                    encrypt_username: pick_str(&c, &["encryptUsername", "encrypt_username"]),
                    encrypt_user_name: pick_str(&c, &["encryptUserName"]),
                    local_type: 0,
                },
                None => MemberContact::default(),
            };
            contacts.insert(u.clone(), mc);
        }
        let mut vals: Vec<String> = members
            .iter()
            .map(|m| pick_str(m, &["username"]))
            .chain(members.iter().map(|m| pick_str(m, &["originalName"])))
            .collect();
        for c in contacts.values() {
            vals.extend([
                c.username.clone(),
                c.user_name.clone(),
                c.encrypt_username.clone(),
                c.encrypt_user_name.clone(),
                c.alias.clone(),
            ]);
        }
        let refs: Vec<&str> = vals.iter().map(String::as_str).collect();
        let cands = build_id_candidates(&refs);
        let nick_map = match wcdb.group_nicknames(chatroom_id) {
            Ok(Value::Object(m)) => trusted_nicknames(&m, &cands),
            _ => HashMap::new(),
        };
        let my_wxid = clean_account_dir_name(
            &self
                .wxid_override
                .clone()
                .or_else(|| self.profile().ok().and_then(|p| p.wxid.clone()))
                .unwrap_or_default(),
        );
        let owner = self.detect_group_owner(wcdb, chatroom_id, &members);
        Ok(members
            .iter()
            .map(|m| {
                let wxid = pick_str(m, &["username"]);
                let contact = contacts.get(&wxid).cloned().unwrap_or_default();
                let normalized = clean_account_dir_name(&wxid);
                let mut lookup = build_id_candidates(&[
                    &wxid,
                    &pick_str(m, &["originalName"]),
                    &contact.username,
                    &contact.user_name,
                    &contact.encrypt_username,
                    &contact.encrypt_user_name,
                    &contact.alias,
                ]);
                if normalized == my_wxid {
                    lookup.push(my_wxid.clone());
                }
                let mut o = Map::new();
                o.insert("username".into(), json!(wxid));
                o.insert(
                    "displayName".into(),
                    json!(names
                        .get(&wxid)
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| wxid.clone())),
                );
                o.insert("nickname".into(), json!(contact.nick_name));
                o.insert("alias".into(), json!(contact.alias));
                o.insert("remark".into(), json!(contact.remark));
                o.insert(
                    "groupNickname".into(),
                    json!(resolve_nickname(&nick_map, &lookup)),
                );
                if let Some(a) = m.get("avatarUrl").filter(|v| !v.is_null()) {
                    o.insert("avatarUrl".into(), a.clone());
                }
                o.insert(
                    "isOwner".into(),
                    json!(owner.as_deref() == Some(wxid.as_str())),
                );
                Value::Object(o)
            })
            .collect())
    }

    fn group_stats_data(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
        start: i64,
        end: i64,
    ) -> AppResult<Value> {
        let (b, e) = normalize_range(start, end);
        let data = self.wrap_native(wcdb.group_stats(chatroom_id, b, e))?;
        if data.is_null() {
            return Err(AppError::native("group aggregation failed"));
        }
        Ok(data)
    }

    /// `getGroupMessageRanking`
    pub fn group_message_ranking(
        &self,
        chatroom_id: &str,
        limit: usize,
        start: i64,
        end: i64,
    ) -> AppResult<Vec<Value>> {
        let wcdb = self.open_wcdb()?;
        let d = self.group_stats_data(&wcdb, chatroom_id, start, end)?;
        let Some(senders) = d
            .pointer(&format!(
                "/sessions/{}/senders",
                escape_pointer(chatroom_id)
            ))
            .and_then(Value::as_object)
        else {
            return Ok(Vec::new());
        };
        let id_map = d
            .get("idMap")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let mut rankings: Vec<(String, i64)> = senders
            .iter()
            .map(|(id, c)| {
                (
                    id_map
                        .get(id)
                        .and_then(Value::as_str)
                        .unwrap_or(id)
                        .to_string(),
                    c.as_i64()
                        .unwrap_or_else(|| c.as_f64().unwrap_or(0.0) as i64),
                )
            })
            .collect();
        rankings.sort_by_key(|a| std::cmp::Reverse(a.1));
        rankings.truncate(limit);
        let names_in: Vec<String> = rankings.iter().map(|r| r.0.clone()).collect();
        let (names, avatars) = self.names_and_avatars(&wcdb, &names_in);
        Ok(rankings
            .into_iter()
            .map(|(username, count)| {
                let mut member = Map::new();
                member.insert("username".into(), json!(username));
                member.insert(
                    "displayName".into(),
                    json!(names
                        .get(&username)
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| username.clone())),
                );
                if let Some(a) = avatars.get(&username).filter(|a| !a.is_empty()) {
                    member.insert("avatarUrl".into(), json!(a));
                }
                json!({ "member": member, "messageCount": count })
            })
            .collect())
    }

    /// `getGroupActiveHours`
    pub fn group_active_hours(&self, chatroom_id: &str, start: i64, end: i64) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let d = self.group_stats_data(&wcdb, chatroom_id, start, end)?;
        let hourly = d.get("hourly");
        let mut dist = Map::new();
        for i in 0..24 {
            let v = hourly
                .and_then(|h| h.get(i.to_string()).or_else(|| h.get(i)))
                .and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)))
                .unwrap_or(0);
            dist.insert(i.to_string(), json!(v));
        }
        Ok(json!({ "hourlyDistribution": dist }))
    }

    /// `getGroupMediaStats`
    pub fn group_media_stats(&self, chatroom_id: &str, start: i64, end: i64) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let d = self.group_stats_data(&wcdb, chatroom_id, start, end)?;
        let raw = d
            .get("typeCounts")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let main_types = [1i64, 3, 34, 43, 47, 49];
        let names = [
            (1, "文本"),
            (3, "图片"),
            (34, "语音"),
            (43, "视频"),
            (47, "表情包"),
            (49, "链接/文件"),
        ];
        let mut counts: HashMap<i64, i64> = HashMap::new();
        let mut others = 0i64;
        for (k, c) in raw {
            let t = crate::api::js_parse_int(&k).unwrap_or(0);
            let c = c
                .as_i64()
                .unwrap_or_else(|| c.as_f64().unwrap_or(0.0) as i64);
            if main_types.contains(&t) {
                *counts.entry(t).or_insert(0) += c;
            } else {
                others += c;
            }
        }
        let mut media: Vec<(i64, String, i64)> = main_types
            .iter()
            .map(|t| {
                (
                    *t,
                    names
                        .iter()
                        .find(|(k, _)| k == t)
                        .map(|(_, n)| n.to_string())
                        .unwrap(),
                    counts.get(t).copied().unwrap_or(0),
                )
            })
            .filter(|(_, _, c)| *c > 0)
            .collect();
        if others > 0 {
            media.push((-1, "其他".into(), others));
        }
        media.sort_by_key(|a| std::cmp::Reverse(a.2));
        let total: i64 = media.iter().map(|m| m.2).sum();
        Ok(
            json!({ "typeCounts": media.iter().map(|(t, n, c)| json!({ "type": t, "name": n, "count": c })).collect::<Vec<_>>(), "total": total }),
        )
    }

    fn row_sender(&self, row: &Value, my_wxid: &str) -> String {
        let raw_is_send = ["computed_is_send", "is_send", "isSend", "WCDB_CT_is_send"]
            .iter()
            .filter_map(|k| row.get(*k))
            .find(|v| !v.is_null());
        if let Some(v) = raw_is_send {
            let t = v
                .as_str()
                .map(str::to_string)
                .unwrap_or_else(|| v.to_string());
            if crate::api::js_parse_int(&t) == Some(1) && !my_wxid.is_empty() {
                return my_wxid.to_string();
            }
        }
        let direct = pick_str(
            row,
            &[
                "sender_username",
                "senderUsername",
                "sender",
                "WCDB_CT_sender_username",
            ],
        );
        if !direct.is_empty() {
            return direct;
        }
        if let Some(obj) = row.as_object() {
            for (k, v) in obj {
                let kl = k.to_lowercase();
                if matches!(
                    kl.as_str(),
                    "sender_username" | "senderusername" | "sender" | "wcdb_ct_sender_username"
                ) {
                    let t = v.as_str().unwrap_or("").trim();
                    if !t.is_empty() {
                        return t.to_string();
                    }
                }
            }
        }
        let raw = pick_str(
            row,
            &["StrContent", "message_content", "content", "msg_content"],
        );
        if !raw.is_empty() {
            if let Some(c) =
                rx(r"(?i)^\s*([a-zA-Z0-9_@-]{4,}):\s*(?:\r?\n|<br\s*/?>)").captures(&raw)
            {
                return c[1].trim().to_string();
            }
        }
        String::new()
    }

    /// A cursor over the messages `member` may have sent: only their rows (and rows with an unresolved sender) are
    /// read, which keeps a per-member scan of a 200,000-message group to that member's share of the work.
    fn member_cursor(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
        member: &str,
        batch: i32,
        ascending: bool,
        (start, end): (i64, i64),
    ) -> AppResult<i64> {
        let norm = |v: i64| {
            if v <= 0 {
                0
            } else if v > 10_000_000_000 {
                (v / 1000) as i32
            } else {
                v as i32
            }
        };
        let (b, e) = (norm(start), norm(end));
        let ok = |name: &str| same_identity(member, name);
        let mine = same_identity(member, &self.my_wxid_cleaned());
        match wcdb.open_message_cursor_for_senders(
            chatroom_id,
            batch,
            ascending,
            (b, e),
            true,
            (&ok, mine),
        ) {
            Ok(c) if c != 0 => Ok(c),
            _ => self.wrap_native(wcdb.open_message_cursor_for_senders(
                chatroom_id,
                batch,
                ascending,
                (b, e),
                false,
                (&ok, mine),
            )),
        }
    }

    /// `getGroupMemberAnalytics`
    pub fn group_member_analytics(
        &self,
        chatroom_id: &str,
        member: &str,
        start: i64,
        end: i64,
    ) -> AppResult<Value> {
        use chrono::{Datelike, Local, TimeZone, Timelike};
        let wcdb = self.open_wcdb()?;
        let (chatroom_id, member) = (chatroom_id.trim(), member.trim());
        let my_wxid = self.my_wxid_cleaned();
        let cursor = self.member_cursor(&wcdb, chatroom_id, member, 10000, true, (start, end))?;

        let mut match_cache: HashMap<String, bool> = HashMap::new();
        let mut total = 0i64;
        let (mut text, mut image, mut voice, mut video, mut emoji, mut other) =
            (0i64, 0i64, 0i64, 0i64, 0i64, 0i64);
        let mut sent = 0i64;
        let (mut first, mut last): (Option<i64>, Option<i64>) = (None, None);
        let mut type_counts: BTreeMap<i64, i64> = BTreeMap::new();
        let mut hourly = [0i64; 24];
        let mut days: HashSet<(i32, u32, u32)> = HashSet::new();
        let mut phrases: HashMap<String, i64> = HashMap::new();
        let mut emojis: HashMap<String, i64> = HashMap::new();
        let mut phrase_order: Vec<String> = Vec::new();
        let mut emoji_order: Vec<String> = Vec::new();

        let result = (|| -> AppResult<()> {
            loop {
                let (rows, has_more) = self.wrap_native(wcdb.fetch_message_batch(cursor))?;
                let rows = rows.as_array().cloned().unwrap_or_default();
                if rows.is_empty() {
                    break;
                }
                for row in &rows {
                    let is_send_raw = ["computed_is_send", "is_send", "isSend", "WCDB_CT_is_send"]
                        .iter()
                        .filter_map(|k| row.get(*k))
                        .find(|v| !v.is_null());
                    let is_send = is_send_raw.is_some_and(|v| {
                        crate::api::js_parse_int(
                            &v.as_str()
                                .map(str::to_string)
                                .unwrap_or_else(|| v.to_string()),
                        ) == Some(1)
                    });
                    let mut sender = self.row_sender(row, &my_wxid);
                    if is_send {
                        sender = my_wxid.clone();
                    }
                    if sender.is_empty() {
                        continue;
                    }
                    let key = sender.trim().to_lowercase();
                    let matched = *match_cache
                        .entry(key)
                        .or_insert_with(|| same_identity(member, &sender));
                    if !matched {
                        continue;
                    }
                    let msg_type = crate::api::js_parse_int(&pick_str(
                        row,
                        &["Type", "type", "local_type", "msg_type"],
                    ))
                    .unwrap_or(0);
                    let create_time = crate::api::js_parse_int(&pick_str(
                        row,
                        &["CreateTime", "create_time", "createTime", "msg_time"],
                    ))
                    .unwrap_or(0);
                    let mut content = pick_str(
                        row,
                        &["StrContent", "message_content", "content", "msg_content"],
                    );
                    if !content.is_empty() {
                        content = rx(r"(?i)^\s*([a-zA-Z0-9_@-]{4,}):\s*(?:\r?\n|<br\s*/?>)")
                            .replace(&content, "")
                            .to_string();
                    }
                    total += 1;
                    if msg_type == 1 || msg_type == 244813135921 {
                        text += 1;
                        if !content.is_empty() {
                            let t = content.trim().to_string();
                            if !t.is_empty() && t.chars().count() <= 20 {
                                let e = phrases.entry(t.clone()).or_insert(0);
                                if *e == 0 {
                                    phrase_order.push(t.clone());
                                }
                                *e += 1;
                            }
                            for m in rx(r"\[.*?\]").find_iter(&t) {
                                let e = emojis.entry(m.as_str().to_string()).or_insert(0);
                                if *e == 0 {
                                    emoji_order.push(m.as_str().to_string());
                                }
                                *e += 1;
                            }
                        }
                    } else if msg_type == 3 {
                        image += 1;
                    } else if msg_type == 34 {
                        voice += 1;
                    } else if msg_type == 43 {
                        video += 1;
                    } else if msg_type == 47 {
                        emoji += 1;
                    } else {
                        other += 1;
                    }
                    sent += 1;
                    *type_counts.entry(msg_type).or_insert(0) += 1;
                    if create_time > 0 {
                        first = Some(first.map_or(create_time, |f| f.min(create_time)));
                        last = Some(last.map_or(create_time, |l| l.max(create_time)));
                        if let Some(d) = Local.timestamp_opt(create_time, 0).single() {
                            hourly[d.hour() as usize] += 1;
                            days.insert((d.year(), d.month0(), d.day()));
                        }
                    }
                }
                if !has_more {
                    break;
                }
            }
            Ok(())
        })();
        let _ = wcdb.close_message_cursor(cursor);
        result?;

        let top =
            |counts: &HashMap<String, i64>, order: &[String], n: usize| -> Vec<(String, i64)> {
                let mut v: Vec<(String, i64)> =
                    order.iter().map(|k| (k.clone(), counts[k])).collect();
                v.sort_by_key(|a| std::cmp::Reverse(a.1)); // stable: ties keep first-seen order like JS Map + sort
                v.truncate(n);
                v
            };
        let mut dist = Map::new();
        for (i, c) in hourly.iter().enumerate() {
            dist.insert(i.to_string(), json!(c));
        }
        let mut tc = Map::new();
        for (k, v) in &type_counts {
            tc.insert(k.to_string(), json!(v));
        }
        Ok(json!({
            "statistics": {
                "totalMessages": total, "textMessages": text, "imageMessages": image, "voiceMessages": voice,
                "videoMessages": video, "emojiMessages": emoji, "otherMessages": other,
                "sentMessages": sent, "receivedMessages": 0,
                "firstMessageTime": first, "lastMessageTime": last,
                "activeDays": days.len(), "messageTypeCounts": tc
            },
            "timeDistribution": dist,
            "commonPhrases": top(&phrases, &phrase_order, 10).into_iter().map(|(p, c)| json!({ "phrase": p, "count": c })).collect::<Vec<_>>(),
            "commonEmojis": top(&emojis, &emoji_order, 10).into_iter().map(|(e, c)| json!({ "emoji": e, "count": c })).collect::<Vec<_>>()
        }))
    }

    fn collect_member_messages(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        chatroom_id: &str,
        member: &str,
        start: i64,
        end: i64,
    ) -> AppResult<Vec<chat_msg::ChatMessage>> {
        let my_wxid = self.my_wxid_cleaned();
        let cursor = self.member_cursor(wcdb, chatroom_id, member, 800, true, (start, end))?;
        let mut out = Vec::new();
        let mut cache: HashMap<String, bool> = HashMap::new();
        let matches = |s: &str, cache: &mut HashMap<String, bool>| -> bool {
            let key = s.trim().to_lowercase();
            if key.is_empty() {
                return false;
            }
            *cache.entry(key).or_insert_with(|| same_identity(member, s))
        };
        let result = (|| -> AppResult<()> {
            loop {
                let (rows, has_more) = self.wrap_native(wcdb.fetch_message_batch(cursor))?;
                let rows = rows.as_array().cloned().unwrap_or_default();
                if rows.is_empty() {
                    break;
                }
                for row in &rows {
                    let sender = self.row_sender(row, &my_wxid);
                    if !sender.is_empty() && !matches(&sender, &mut cache) {
                        continue;
                    }
                    let Some(msg) = chat_msg::map_rows(std::slice::from_ref(row), &my_wxid)
                        .into_iter()
                        .next()
                    else {
                        continue;
                    };
                    if matches(msg.sender_username.as_deref().unwrap_or(""), &mut cache) {
                        out.push(msg);
                    }
                }
                if !has_more {
                    break;
                }
            }
            Ok(())
        })();
        let _ = wcdb.close_message_cursor(cursor);
        result?;
        Ok(out)
    }

    /// `getGroupMemberMessages`: one page of a member's messages (newest first).
    pub fn group_member_messages(
        &self,
        chatroom_id: &str,
        member: &str,
        start: i64,
        end: i64,
        limit: usize,
        cursor_in: usize,
    ) -> AppResult<Value> {
        let (chatroom_id, member) = (chatroom_id.trim(), member.trim());
        if chatroom_id.is_empty() {
            return Err(AppError::usage("chatroom id must not be empty"));
        }
        if member.is_empty() {
            return Err(AppError::usage("member id must not be empty"));
        }
        let limit = limit.clamp(1, 100);
        let wcdb = self.open_wcdb()?;
        let my_wxid = self.my_wxid_cleaned();
        let batch_size = (limit * 4).max(240) as i32;
        let db_cursor = self.member_cursor(
            &wcdb,
            chatroom_id,
            member,
            batch_size,
            false,
            (start.max(0), end.max(0)),
        )?;
        let mut matched: Vec<chat_msg::ChatMessage> = Vec::new();
        let mut cache: HashMap<String, bool> = HashMap::new();
        let mut consumed = 0usize;
        let mut cursor = cursor_in;
        let mut has_more = false;
        let result = (|| -> AppResult<()> {
            while matched.len() < limit {
                let (rows, batch_more) = self.wrap_native(wcdb.fetch_message_batch(db_cursor))?;
                let rows = rows.as_array().cloned().unwrap_or_default();
                if rows.is_empty() {
                    has_more = false;
                    break;
                }
                let mut start_index = 0usize;
                if cursor > consumed {
                    let skip = (cursor - consumed).min(rows.len());
                    consumed += skip;
                    start_index = skip;
                    if start_index >= rows.len() {
                        if !batch_more {
                            has_more = false;
                            break;
                        }
                        continue;
                    }
                }
                for (index, row) in rows.iter().enumerate().skip(start_index) {
                    consumed += 1;
                    let sender = self.row_sender(row, &my_wxid);
                    let is_match = |s: &str, cache: &mut HashMap<String, bool>| -> bool {
                        let key = s.trim().to_lowercase();
                        !key.is_empty()
                            && *cache.entry(key).or_insert_with(|| same_identity(member, s))
                    };
                    if !sender.is_empty() && !is_match(&sender, &mut cache) {
                        continue;
                    }
                    let Some(msg) = chat_msg::map_rows(std::slice::from_ref(row), &my_wxid)
                        .into_iter()
                        .next()
                    else {
                        continue;
                    };
                    if !is_match(msg.sender_username.as_deref().unwrap_or(""), &mut cache) {
                        continue;
                    }
                    matched.push(msg);
                    if matched.len() >= limit {
                        cursor = consumed;
                        has_more = index < rows.len() - 1 || batch_more;
                        break;
                    }
                }
                if matched.len() >= limit {
                    break;
                }
                cursor = consumed;
                if !batch_more {
                    has_more = false;
                    break;
                }
            }
            Ok(())
        })();
        let _ = wcdb.close_message_cursor(db_cursor);
        result?;
        Ok(
            json!({ "messages": matched.iter().map(|m| m.to_json()).collect::<Vec<_>>(), "hasMore": has_more, "nextCursor": cursor }),
        )
    }

    /// `exportGroupMemberMessages`: CSV (BOM) or XLSX depending on the extension.
    pub fn group_export_member_messages(
        &self,
        chatroom_id: &str,
        member: &str,
        out: &Path,
        start: i64,
        end: i64,
    ) -> AppResult<Value> {
        let (chatroom_id, member) = (chatroom_id.trim(), member.trim());
        if chatroom_id.is_empty() {
            return Err(AppError::usage("chatroom id must not be empty"));
        }
        if member.is_empty() {
            return Err(AppError::usage("member id must not be empty"));
        }
        let wcdb = self.open_wcdb()?;
        let export_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let (names, _) =
            self.names_and_avatars(&wcdb, &[chatroom_id.to_string(), member.to_string()]);
        let group_name = names
            .get(chatroom_id)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| chatroom_id.to_string());
        let default_member = names
            .get(member)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| member.to_string());
        let (mut member_display, mut alias, mut remark, mut group_nick) = (
            default_member.clone(),
            String::new(),
            String::new(),
            String::new(),
        );
        if let Ok(members) = self.group_members_full_with(&wcdb, chatroom_id) {
            if let Some(m) = members
                .iter()
                .find(|m| same_identity(m["username"].as_str().unwrap_or(""), member))
            {
                member_display = pick_str(m, &["displayName"]);
                if member_display.is_empty() {
                    member_display = default_member;
                }
                alias = pick_str(m, &["alias"]);
                remark = pick_str(m, &["remark"]);
                group_nick = pick_str(m, &["groupNickname"]);
            }
        }
        let collected =
            self.collect_member_messages(&wcdb, chatroom_id, member, start.max(0), end.max(0))?;
        let records: Vec<[String; 5]> = collected
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let content = {
                    let p = m.parsed_content.trim();
                    if !p.is_empty() {
                        p.to_string()
                    } else {
                        m.raw_content.trim().to_string()
                    }
                };
                [
                    (i + 1).to_string(),
                    format_unix_time(m.create_time),
                    m.sender_username.clone().unwrap_or_default(),
                    simple_type_name(m.local_type),
                    content,
                ]
            })
            .collect();
        if let Some(parent) = out.parent() {
            std::fs::create_dir_all(parent).map_err(|e| AppError::runtime(e.to_string()))?;
        }
        let ext = out
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let meta = [
            "导出工具",
            "WeFlow",
            "导出版本",
            "0.0.2",
            "平台",
            "wechat",
            "导出时间",
            &export_time,
        ];
        if ext == "csv" {
            let mut rows: Vec<Vec<String>> = vec![
                vec!["会话信息".into()],
                [
                    "群聊ID",
                    chatroom_id,
                    "",
                    "群聊名称",
                    &group_name,
                    "成员wxid",
                    member,
                    "",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                [
                    "成员显示名",
                    &member_display,
                    "成员备注",
                    &remark,
                    "群昵称",
                    &group_nick,
                    "微信号",
                    &alias,
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                meta.iter().map(|s| s.to_string()).collect(),
                ["序号", "时间", "发送者wxid", "消息类型", "内容"]
                    .iter()
                    .map(|s| s.to_string())
                    .collect(),
            ];
            rows.extend(records.iter().map(|r| r.to_vec()));
            let lines: Vec<String> = rows
                .iter()
                .map(|r| r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","))
                .collect();
            std::fs::write(out, format!("\u{feff}{}", lines.join("\n")))
                .map_err(|e| AppError::runtime(e.to_string()))?;
        } else {
            use rust_xlsxwriter::{Format, FormatAlign, Workbook};
            let bold = Format::new()
                .set_bold()
                .set_font_name("Calibri")
                .set_font_size(11);
            let wrap = Format::new().set_text_wrap().set_align(FormatAlign::Top);
            let mut wb = Workbook::new();
            let ws = wb.add_worksheet();
            let x = |e: rust_xlsxwriter::XlsxError| AppError::runtime(e.to_string());
            ws.set_name(worksheet_name("成员消息记录")).map_err(x)?;
            ws.write_with_format(0, 0, "会话信息", &bold).map_err(x)?;
            ws.set_row_height(0, 24).map_err(x)?;
            ws.write_with_format(1, 0, "群聊ID", &bold).map_err(x)?;
            ws.merge_range(1, 1, 1, 2, chatroom_id, &Format::new())
                .map_err(x)?;
            ws.write_with_format(1, 3, "群聊名称", &bold).map_err(x)?;
            ws.write(1, 4, group_name.as_str()).map_err(x)?;
            ws.write_with_format(1, 5, "成员wxid", &bold).map_err(x)?;
            ws.merge_range(1, 6, 1, 7, member, &Format::new())
                .map_err(x)?;
            for (i, (label, value)) in [
                ("成员显示名", &member_display),
                ("成员备注", &remark),
                ("群昵称", &group_nick),
                ("微信号", &alias),
            ]
            .iter()
            .enumerate()
            {
                ws.write_with_format(2, (i * 2) as u16, *label, &bold)
                    .map_err(x)?;
                ws.write(2, (i * 2 + 1) as u16, value.as_str()).map_err(x)?;
            }
            for (i, pair) in meta.chunks(2).enumerate() {
                ws.write_with_format(3, (i * 2) as u16, pair[0], &bold)
                    .map_err(x)?;
                ws.write(3, (i * 2 + 1) as u16, pair[1]).map_err(x)?;
            }
            for (i, h) in ["序号", "时间", "发送者wxid", "消息类型", "内容"]
                .iter()
                .enumerate()
            {
                ws.write_with_format(4, i as u16, *h, &bold).map_err(x)?;
            }
            ws.set_row_height(4, 22).map_err(x)?;
            for (col, w) in [10.0, 22.0, 30.0, 16.0, 90.0, 16.0, 20.0, 24.0]
                .iter()
                .enumerate()
            {
                ws.set_column_width(col as u16, *w).map_err(x)?;
            }
            for (i, r) in records.iter().enumerate() {
                let row = (5 + i) as u32;
                ws.write_number_with_format(row, 0, (i + 1) as f64, &wrap)
                    .map_err(x)?;
                for (c, cell) in r.iter().enumerate().take(5).skip(1) {
                    ws.write_with_format(row, c as u16, cell.as_str(), &wrap)
                        .map_err(x)?;
                }
            }
            wb.save(out).map_err(x)?;
        }
        Ok(json!({ "success": true, "count": records.len() }))
    }

    /// `exportGroupMembers`
    pub fn group_export_members(&self, chatroom_id: &str, out: &Path) -> AppResult<Value> {
        let wcdb = self.open_wcdb()?;
        let export_time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
        let (names, _) = self.names_and_avatars(&wcdb, &[chatroom_id.to_string()]);
        let group_name = names
            .get(chatroom_id)
            .filter(|n| !n.is_empty())
            .cloned()
            .unwrap_or_else(|| chatroom_id.to_string());
        let session_remark = wcdb
            .contact(chatroom_id)
            .ok()
            .map(|c| pick_str(&c, &["remark"]))
            .unwrap_or_default();
        let members = self.members_of(&wcdb, chatroom_id)?;
        if members.is_empty() {
            return Err(AppError::runtime("the group has no members"));
        }
        let usernames: Vec<String> = members
            .iter()
            .map(|m| pick_str(m, &["username"]))
            .filter(|u| !u.is_empty())
            .collect();
        let (display, _) = self.names_and_avatars(&wcdb, &usernames);
        let mut contacts: HashMap<String, MemberContact> = HashMap::new();
        for u in &usernames {
            let mc = match wcdb.contact(u).ok().filter(Value::is_object) {
                Some(c) => MemberContact {
                    remark: pick_str(&c, &["remark"]),
                    nick_name: pick_str(&c, &["nickName", "nick_name"]),
                    alias: pick_str(&c, &["alias"]),
                    username: pick_str(&c, &["username"]),
                    user_name: pick_str(&c, &["userName", "user_name"]),
                    encrypt_username: pick_str(&c, &["encryptUsername", "encrypt_username"]),
                    encrypt_user_name: pick_str(&c, &["encryptUserName"]),
                    local_type: 0,
                },
                None => MemberContact::default(),
            };
            contacts.insert(u.clone(), mc);
        }
        let mut vals: Vec<String> = members
            .iter()
            .map(|m| pick_str(m, &["username"]))
            .chain(members.iter().map(|m| pick_str(m, &["originalName"])))
            .collect();
        for c in contacts.values() {
            vals.extend([
                c.username.clone(),
                c.user_name.clone(),
                c.encrypt_username.clone(),
                c.encrypt_user_name.clone(),
                c.alias.clone(),
            ]);
        }
        let refs: Vec<&str> = vals.iter().map(String::as_str).collect();
        let nick_map = match wcdb.group_nicknames(chatroom_id) {
            Ok(Value::Object(m)) => trusted_nicknames(&m, &build_id_candidates(&refs)),
            _ => HashMap::new(),
        };
        let my_wxid = clean_account_dir_name(
            &self
                .wxid_override
                .clone()
                .or_else(|| self.profile().ok().and_then(|p| p.wxid.clone()))
                .unwrap_or_default(),
        );
        let mut rows: Vec<[String; 5]> = Vec::new();
        for m in &members {
            let wxid = pick_str(m, &["username"]);
            let c = contacts.get(&wxid).cloned().unwrap_or_default();
            let fallback = display.get(&wxid).cloned().unwrap_or_default();
            let nick = if !c.nick_name.is_empty() {
                c.nick_name.clone()
            } else {
                fallback
            };
            let mut cands = build_id_candidates(&[
                &wxid,
                &pick_str(m, &["originalName"]),
                &c.username,
                &c.user_name,
                &c.encrypt_username,
                &c.encrypt_user_name,
                &c.alias,
            ]);
            if clean_account_dir_name(&wxid) == my_wxid {
                cands.push(my_wxid.clone());
            }
            rows.push([
                nick,
                c.remark.clone(),
                resolve_nickname(&nick_map, &cands),
                wxid,
                c.alias.clone(),
            ]);
        }
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent).map_err(|e| AppError::runtime(e.to_string()))?;
        }
        let ext = out
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let meta = [
            "导出工具",
            "WeFlow",
            "导出版本",
            "0.0.2",
            "平台",
            "wechat",
            "导出时间",
            &export_time,
        ];
        let header = ["微信昵称", "微信备注", "群昵称", "wxid", "微信号"];
        if ext == "csv" {
            let mut all: Vec<Vec<String>> = vec![
                vec!["会话信息".into()],
                [
                    "微信ID",
                    chatroom_id,
                    "",
                    "昵称",
                    &group_name,
                    "备注",
                    &session_remark,
                    "",
                ]
                .iter()
                .map(|s| s.to_string())
                .collect(),
                meta.iter().map(|s| s.to_string()).collect(),
                header.iter().map(|s| s.to_string()).collect(),
            ];
            all.extend(rows.iter().map(|r| r.to_vec()));
            let lines: Vec<String> = all
                .iter()
                .map(|r| r.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","))
                .collect();
            std::fs::write(out, format!("\u{feff}{}", lines.join("\n")))
                .map_err(|e| AppError::runtime(e.to_string()))?;
        } else {
            use rust_xlsxwriter::{Format, FormatAlign, Workbook};
            let bold = Format::new()
                .set_bold()
                .set_font_name("Calibri")
                .set_font_size(11);
            let wrap = Format::new().set_text_wrap().set_align(FormatAlign::Top);
            let mut wb = Workbook::new();
            let ws = wb.add_worksheet();
            let x = |e: rust_xlsxwriter::XlsxError| AppError::runtime(e.to_string());
            ws.set_name(worksheet_name("群成员列表")).map_err(x)?;
            ws.write_with_format(0, 0, "会话信息", &bold).map_err(x)?;
            ws.set_row_height(0, 25).map_err(x)?;
            ws.write_with_format(1, 0, "微信ID", &bold).map_err(x)?;
            ws.merge_range(1, 1, 1, 2, chatroom_id, &Format::new())
                .map_err(x)?;
            ws.write_with_format(1, 3, "昵称", &bold).map_err(x)?;
            ws.write(1, 4, group_name.as_str()).map_err(x)?;
            ws.write_with_format(1, 5, "备注", &bold).map_err(x)?;
            ws.merge_range(1, 6, 1, 7, session_remark.as_str(), &Format::new())
                .map_err(x)?;
            ws.set_row_height(1, 20).map_err(x)?;
            for (i, pair) in meta.chunks(2).enumerate() {
                ws.write_with_format(2, (i * 2) as u16, pair[0], &bold)
                    .map_err(x)?;
                ws.write(2, (i * 2 + 1) as u16, pair[1]).map_err(x)?;
            }
            ws.set_row_height(2, 20).map_err(x)?;
            for (i, h) in header.iter().enumerate() {
                ws.write_with_format(3, i as u16, *h, &bold).map_err(x)?;
            }
            ws.set_row_height(3, 22).map_err(x)?;
            for (col, w) in [28.0, 28.0, 28.0, 36.0, 28.0, 18.0, 24.0, 22.0]
                .iter()
                .enumerate()
            {
                ws.set_column_width(col as u16, *w).map_err(x)?;
            }
            for (i, r) in rows.iter().enumerate() {
                for (c, cell) in r.iter().enumerate().take(5) {
                    ws.write_with_format((4 + i) as u32, c as u16, cell.as_str(), &wrap)
                        .map_err(x)?;
                }
            }
            wb.save(out).map_err(x)?;
        }
        Ok(json!({ "success": true, "count": members.len() }))
    }
}

/// JSON-pointer escaping of a path segment.
fn escape_pointer(s: &str) -> String {
    s.replace('~', "~0").replace('/', "~1")
}
