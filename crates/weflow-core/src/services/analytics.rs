//! Chat analytics (`electron/services/analyticsService.ts`): overall statistics, contact
//! rankings, time distribution and the exclusion list.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};

use super::*;

const AGGREGATE_TTL: Duration = Duration::from_secs(5 * 60);

#[derive(Default)]
pub(crate) struct AnalyticsState {
    cache: Option<(String, Value, Instant)>,
}

fn normalize_username(u: &str) -> String {
    u.trim().to_lowercase()
}

fn normalize_excluded(v: &Value) -> Vec<String> {
    let Some(items) = v.as_array() else { return Vec::new() };
    let mut seen = HashSet::new();
    items.iter().filter_map(Value::as_str).map(normalize_username).filter(|s| !s.is_empty()).filter(|s| seen.insert(s.clone())).collect()
}

fn is_private_session(username: &str, cleaned_wxid: &str) -> bool {
    if username.is_empty() || username.to_lowercase() == cleaned_wxid.to_lowercase() {
        return false;
    }
    if username.contains("@chatroom") || username == "filehelper" || username.starts_with("gh_") || username.to_lowercase() == "weixin" {
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
        "placeholder_foldgroup",
        "@helper_folders",
        "@placeholder_foldgroup",
    ] {
        if username.starts_with(prefix) || username == prefix {
            return false;
        }
    }
    !(username.contains("@kefu.openim") || username.contains("@openim") || username.contains("service_"))
}

fn num(v: &Value) -> i64 {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(0)
}

fn map_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
}

impl ServiceHub {
    fn analytics_cache_path(&self) -> PathBuf {
        self.cache_base().join("analytics_cache.json")
    }

    fn fresh_config(&self) -> crate::config::ConfigStore {
        crate::config::ConfigStore::load(&self.ctx.config_path).unwrap_or_else(|_| self.config.clone())
    }

    /// Persists one key of the active profile to the config file.
    pub fn set_config_value(&self, key: &str, value: Value) -> AppResult<()> {
        let mut cfg = self.fresh_config();
        cfg.set_key(Some(&self.profile_name), key, value)?;
        cfg.save(&self.ctx.config_path).map_err(|e| AppError::config(e.to_string()))
    }

    fn excluded_list(&self) -> Vec<String> {
        normalize_excluded(&self.fresh_config().get_key(Some(&self.profile_name), "analyticsExcludedUsernames"))
    }

    /// `getExcludedUsernames`
    pub fn analytics_excluded_usernames(&self) -> AppResult<Vec<String>> {
        Ok(self.excluded_list())
    }

    /// `setExcludedUsernames`: normalises, persists and drops the aggregate caches.
    pub fn analytics_set_excluded_usernames(&self, usernames: &[String]) -> AppResult<Vec<String>> {
        let normalized = normalize_excluded(&json!(usernames));
        self.set_config_value("analyticsExcludedUsernames", json!(normalized))?;
        self.analytics_clear_cache()?;
        Ok(normalized)
    }

    /// `clearCache`
    pub fn analytics_clear_cache(&self) -> AppResult<()> {
        self.analytics_state.lock().unwrap().cache = None;
        let _ = std::fs::remove_file(self.analytics_cache_path());
        Ok(())
    }

    fn private_sessions(&self, wcdb: &weflow_native::wcdb::Wcdb, cleaned_wxid: &str, excluded: Option<&HashSet<String>>) -> AppResult<Vec<String>> {
        let raw = wcdb.sessions().map_err(|e| AppError::native(e.to_string()))?;
        let rows = raw.as_array().cloned().unwrap_or_default();
        let own;
        let excluded = match excluded {
            Some(e) => e,
            None => {
                own = self.excluded_list().into_iter().collect::<HashSet<_>>();
                &own
            }
        };
        Ok(rows
            .iter()
            .map(|r| ["username", "user_name", "userName"].iter().filter_map(|k| r.get(*k).and_then(Value::as_str)).find(|s| !s.is_empty()).unwrap_or("").to_string())
            .filter(|u| is_private_session(u, cleaned_wxid))
            .filter(|u| excluded.is_empty() || !excluded.contains(&normalize_username(u)))
            .collect())
    }

    /// `getAggregateStats` with the numeric-id retry of the WCDB wrapper.
    fn aggregate_raw(&self, wcdb: &weflow_native::wcdb::Wcdb, session_ids: &[String], begin: i64, end: i64) -> AppResult<Value> {
        let (b, e) = normalize_range(begin, end);
        let call = |ids: &[String]| -> AppResult<Value> {
            let numeric = !ids.is_empty() && ids.iter().all(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_digit()));
            let payload = if numeric { json!(ids.iter().filter_map(|i| i.parse::<u64>().ok()).collect::<Vec<_>>()) } else { json!(ids) };
            wcdb.invoke_json("wcdb_get_aggregate_stats", &[weflow_native::wcdb::Arg::S(&payload.to_string()), weflow_native::wcdb::Arg::I32(b), weflow_native::wcdb::Arg::I32(e)])
                .map_err(|err| AppError::native(err.to_string()))
        };
        let mut result = call(session_ids)?;
        if num(&result["total"]) == 0 {
            if let Some(id_map) = result.get("idMap").and_then(Value::as_object) {
                let reverse: HashMap<&str, &str> = id_map.iter().filter_map(|(id, name)| name.as_str().filter(|n| !n.is_empty()).map(|n| (n, id.as_str()))).collect();
                let numeric_ids: Vec<String> = session_ids.iter().filter_map(|s| reverse.get(s.as_str())).filter(|id| id.chars().all(|c| c.is_ascii_digit()) && !id.is_empty()).map(|s| s.to_string()).collect();
                if !numeric_ids.is_empty() {
                    if let Ok(retry) = call(&numeric_ids) {
                        result = retry;
                    }
                }
            }
        }
        Ok(result)
    }

    /// `computeAggregateByCursor`: cursor-based fallback when the native aggregate is empty.
    fn aggregate_by_cursor(&self, wcdb: &weflow_native::wcdb::Wcdb, session_ids: &[String], begin: i64, end: i64) -> AppResult<Value> {
        use chrono::{Datelike, Local, TimeZone, Timelike};
        let my = self.my_wxid_cleaned().to_lowercase();
        let (mut total, mut sent, mut received, mut first, mut last) = (0i64, 0i64, 0i64, 0i64, 0i64);
        let mut type_counts: BTreeMap<i64, i64> = BTreeMap::new();
        let mut hourly: BTreeMap<u32, i64> = BTreeMap::new();
        let mut weekday: BTreeMap<u32, i64> = BTreeMap::new();
        let mut daily: BTreeMap<String, i64> = BTreeMap::new();
        let mut monthly: BTreeMap<String, i64> = BTreeMap::new();
        let mut sessions = Map::new();
        for session_id in session_ids {
            let Ok(cursor) = wcdb.open_message_cursor(session_id, 500, true, begin.clamp(0, i32::MAX as i64) as i32, end.clamp(0, i32::MAX as i64) as i32, false) else { continue };
            let (mut s_total, mut s_sent, mut s_recv, mut s_last) = (0i64, 0i64, 0i64, 0i64);
            loop {
                let Ok((rows, more)) = wcdb.fetch_message_batch(cursor) else { break };
                let Some(rows) = rows.as_array() else { break };
                for row in rows {
                    let create_time = ["create_time", "createTime", "create_time_ms"].iter().filter_map(|k| row.get(*k)).map(num).find(|n| *n != 0).unwrap_or(0);
                    if create_time == 0 || (begin > 0 && create_time < begin) || (end > 0 && create_time > end) {
                        continue;
                    }
                    let local_type = ["local_type", "type"].iter().filter_map(|k| row.get(*k)).map(num).find(|n| *n != 0).unwrap_or(1);
                    let raw = row.get("computed_is_send").filter(|v| !v.is_null()).or_else(|| row.get("is_send")).or_else(|| row.get("isSend")).filter(|v| !v.is_null());
                    let mut is_send = raw.map_or(false, |v| v.as_str().map_or(v.as_i64() == Some(1) || v.as_bool() == Some(true), |s| s == "1"));
                    if raw.is_none() {
                        if let Some(sender) = ["sender_username", "senderUsername", "sender"].iter().filter_map(|k| row.get(*k).and_then(Value::as_str)).find(|s| !s.is_empty()) {
                            if !my.is_empty() {
                                let s = sender.to_lowercase();
                                is_send = s == my || s.starts_with(&format!("{my}_"));
                            }
                        }
                    }
                    total += 1;
                    s_total += 1;
                    *type_counts.entry(local_type).or_insert(0) += 1;
                    if is_send {
                        sent += 1;
                        s_sent += 1;
                    } else {
                        received += 1;
                        s_recv += 1;
                    }
                    if first == 0 || create_time < first {
                        first = create_time;
                    }
                    last = last.max(create_time);
                    s_last = s_last.max(create_time);
                    if let Some(d) = Local.timestamp_opt(create_time, 0).single() {
                        let month = format!("{}-{:02}", d.year(), d.month());
                        *hourly.entry(d.hour()).or_insert(0) += 1;
                        *weekday.entry(d.weekday().num_days_from_sunday()).or_insert(0) += 1;
                        *daily.entry(format!("{month}-{:02}", d.day())).or_insert(0) += 1;
                        *monthly.entry(month).or_insert(0) += 1;
                    }
                }
                if !more {
                    break;
                }
            }
            let _ = wcdb.close_message_cursor(cursor);
            if s_total > 0 {
                sessions.insert(session_id.clone(), json!({ "total": s_total, "sent": s_sent, "received": s_recv, "lastTime": s_last }));
            }
        }
        let obj = |m: &BTreeMap<u32, i64>| -> Value { Value::Object(m.iter().map(|(k, v)| (k.to_string(), json!(v))).collect()) };
        Ok(json!({
            "total": total, "sent": sent, "received": received, "firstTime": first, "lastTime": last,
            "typeCounts": type_counts.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>(),
            "hourly": obj(&hourly), "weekday": obj(&weekday),
            "daily": daily.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
            "monthly": monthly.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
            "sessions": sessions, "idMap": {}
        }))
    }

    fn aggregate_key(ids: &[String], begin: i64, end: i64) -> String {
        if ids.is_empty() {
            return format!("{begin}-{end}-0-empty");
        }
        let mut uniq: Vec<String> = ids.iter().cloned().collect::<HashSet<_>>().into_iter().collect();
        uniq.sort();
        use sha1::{Digest, Sha1};
        let mut h = Sha1::new();
        h.update(uniq.join("|").as_bytes());
        let hash: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
        format!("{begin}-{end}-{}-{}", uniq.len(), &hash[..12])
    }

    /// `getAggregateWithFallback`: memory cache → file cache → native aggregate → cursor scan.
    /// Returns `(data, source)`.
    fn aggregate_with_fallback(&self, wcdb: &weflow_native::wcdb::Wcdb, ids: &[String], begin: i64, end: i64, force: bool) -> AppResult<(Value, &'static str)> {
        let key = Self::aggregate_key(ids, begin, end);
        if force {
            self.analytics_state.lock().unwrap().cache = None;
        }
        if !force {
            if let Some((k, data, at)) = &self.analytics_state.lock().unwrap().cache {
                if *k == key && at.elapsed() < AGGREGATE_TTL {
                    return Ok((data.clone(), "cache"));
                }
            }
            if let Ok(raw) = std::fs::read_to_string(self.analytics_cache_path()) {
                if let Ok(v) = serde_json::from_str::<Value>(&raw) {
                    if v["key"].as_str() == Some(key.as_str()) && v.get("data").is_some() {
                        let data = v["data"].clone();
                        self.analytics_state.lock().unwrap().cache = Some((key.clone(), data.clone(), Instant::now()));
                        return Ok((data, "file-cache"));
                    }
                }
            }
        }
        let (data, source) = match self.aggregate_raw(wcdb, ids, begin, end) {
            Ok(d) if num(&d["total"]) > 0 => (d, "dll"),
            _ => (self.aggregate_by_cursor(wcdb, ids, begin, end)?, "cursor"),
        };
        self.analytics_state.lock().unwrap().cache = Some((key.clone(), data.clone(), Instant::now()));
        let _ = std::fs::create_dir_all(self.cache_base());
        let _ = std::fs::write(self.analytics_cache_path(), serde_json::to_string(&json!({ "key": key, "data": data, "updatedAt": chrono::Utc::now().timestamp_millis() })).unwrap_or_default());
        Ok((data, source))
    }

    fn analytics_connect(&self) -> AppResult<(weflow_native::wcdb::Wcdb, String)> {
        let profile = self.profile()?;
        let wxid = self.wxid_override.clone().or_else(|| profile.wxid.clone()).filter(|w| !w.is_empty()).ok_or_else(|| AppError::config("wxid is not configured"))?;
        let wcdb = self.open_wcdb()?;
        Ok((wcdb, clean_account_dir_name(&wxid)))
    }

    fn alias_map(&self, wcdb: &weflow_native::wcdb::Wcdb, usernames: &[String]) -> HashMap<String, String> {
        if usernames.is_empty() {
            return HashMap::new();
        }
        wcdb.contact_alias_map(&serde_json::to_string(usernames).unwrap())
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|m| m.into_iter().filter_map(|(k, v)| v.as_str().filter(|a| !a.is_empty()).map(|a| (k, a.to_string()))).collect())
            .unwrap_or_default()
    }

    /// `getExcludeCandidates`
    pub fn analytics_exclude_candidates(&self) -> AppResult<Vec<Value>> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let excluded: HashSet<String> = self.excluded_list().into_iter().collect();
        let mut usernames: Vec<String> = self.private_sessions(&wcdb, &cleaned, Some(&HashSet::new()))?;
        for name in &excluded {
            if !usernames.contains(name) {
                usernames.push(name.clone());
            }
        }
        if usernames.is_empty() {
            return Ok(Vec::new());
        }
        let (names, avatars) = self.names_and_avatars_pub(&wcdb, &usernames);
        let aliases = self.alias_map(&wcdb, &usernames);
        Ok(usernames
            .iter()
            .map(|u| {
                let alias = aliases.get(u).cloned().unwrap_or_default();
                let wechat_id = if !alias.is_empty() { alias } else if !u.starts_with("wxid_") { u.clone() } else { String::new() };
                let mut o = Map::new();
                o.insert("username".into(), json!(u));
                o.insert("displayName".into(), json!(names.get(u).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| u.clone())));
                if let Some(a) = avatars.get(u) {
                    o.insert("avatarUrl".into(), json!(a));
                }
                o.insert("wechatId".into(), json!(wechat_id));
                Value::Object(o)
            })
            .collect())
    }

    /// `getOverallStatistics`
    pub fn analytics_overall_statistics(&self, force: bool) -> AppResult<Value> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let usernames = self.private_sessions(&wcdb, &cleaned, None)?;
        if usernames.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        self.emit_progress("analytics", "aggregating…", 30, 100);
        let (d, _) = self.aggregate_with_fallback(&wcdb, &usernames, 0, 0, force)?;
        let type_count = |t: i64| d["typeCounts"].get(t.to_string()).map(num).unwrap_or(0);
        let total = num(&d["total"]);
        let text = type_count(1) + type_count(244813135921);
        let (image, voice, video, emoji) = (type_count(3), type_count(34), type_count(43), type_count(47));
        let other = (total - text - image - voice - video - emoji).max(0);
        let active_months = d["monthly"].as_object().map_or(0, |m| m.len()) as i64;
        let opt = |v: i64| if v != 0 { json!(v) } else { Value::Null };
        Ok(json!({
            "totalMessages": total, "textMessages": text, "imageMessages": image, "voiceMessages": voice,
            "videoMessages": video, "emojiMessages": emoji, "otherMessages": other,
            "sentMessages": num(&d["sent"]), "receivedMessages": num(&d["received"]),
            "firstMessageTime": opt(num(&d["firstTime"])), "lastMessageTime": opt(num(&d["lastTime"])),
            "activeDays": active_months * 20,
            "messageTypeCounts": d["typeCounts"]
        }))
    }

    /// `getContactRankings`
    pub fn analytics_contact_rankings(&self, limit: usize, begin: i64, end: i64) -> AppResult<Vec<Value>> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let usernames = self.private_sessions(&wcdb, &cleaned, None)?;
        if usernames.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        let (d, _) = self.aggregate_with_fallback(&wcdb, &usernames, begin, end, false)?;
        let mut sessions = d.get("sessions").and_then(Value::as_object).cloned().unwrap_or_default();
        if let Some(id_map) = d.get("idMap").and_then(Value::as_object) {
            if !sessions.is_empty() && sessions.keys().all(|k| !k.is_empty() && k.chars().all(|c| c.is_ascii_digit())) {
                sessions = sessions.into_iter().map(|(id, stat)| (id_map.get(&id).and_then(Value::as_str).unwrap_or(&id).to_string(), stat)).collect();
            }
        }
        let names: Vec<String> = sessions.keys().cloned().collect();
        let (display, avatars) = self.names_and_avatars_pub(&wcdb, &names);
        let aliases = self.alias_map(&wcdb, &names);
        let mut rankings: Vec<(i64, Value)> = names
            .iter()
            .map(|u| {
                let stat = &sessions[u];
                let alias = aliases.get(u).cloned().unwrap_or_default();
                let wechat_id = if !alias.is_empty() { alias } else if !u.starts_with("wxid_") { u.clone() } else { String::new() };
                let mut o = Map::new();
                o.insert("username".into(), json!(u));
                o.insert("displayName".into(), json!(display.get(u).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| u.clone())));
                if let Some(a) = avatars.get(u) {
                    o.insert("avatarUrl".into(), json!(a));
                }
                o.insert("wechatId".into(), json!(wechat_id));
                o.insert("messageCount".into(), json!(num(&stat["total"])));
                o.insert("sentCount".into(), json!(num(&stat["sent"])));
                o.insert("receivedCount".into(), json!(num(&stat["received"])));
                let last = num(&stat["lastTime"]);
                o.insert("lastMessageTime".into(), if last != 0 { json!(last) } else { Value::Null });
                (num(&stat["total"]), Value::Object(o))
            })
            .collect();
        rankings.sort_by(|a, b| b.0.cmp(&a.0));
        rankings.truncate(limit);
        Ok(rankings.into_iter().map(|r| r.1).collect())
    }

    /// `getTimeDistribution`
    pub fn analytics_time_distribution(&self) -> AppResult<Value> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let usernames = self.private_sessions(&wcdb, &cleaned, None)?;
        if usernames.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        let (d, _) = self.aggregate_with_fallback(&wcdb, &usernames, 0, 0, false)?;
        // native: 0 = Sunday … 6 = Saturday; the UI wants 1 = Monday … 7 = Sunday
        let mut weekday = Map::new();
        if let Some(w) = d.get("weekday").and_then(Value::as_object) {
            for (k, v) in w {
                let n: i64 = crate::api::js_parse_int(k).unwrap_or(0);
                weekday.insert((if n == 0 { 7 } else { n }).to_string(), v.clone());
            }
        }
        let mut hourly = Map::new();
        for i in 0..24 {
            hourly.insert(i.to_string(), json!(map_get(&d["hourly"], &i.to_string()).map(num).unwrap_or(0)));
        }
        Ok(json!({ "hourlyDistribution": hourly, "weekdayDistribution": weekday, "monthlyDistribution": d["monthly"] }))
    }
}
