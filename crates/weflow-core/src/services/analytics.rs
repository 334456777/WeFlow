//! 聊天统计：总体统计、联系人排行、时间分布和排除名单。

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Condvar, Mutex};

use indexmap::IndexMap;
use serde_json::{json, Map, Value};

use super::session_kind::is_private_session;
use super::*;
use crate::contract::{ContactRank, ExcludeCandidate, OverallStatistics, TimeDistribution};
use crate::sync::MutexExt;
use weflow_native::msg::Order;
use weflow_native::msg_row::Columns;
use weflow_native::wcdb::Databases;

/// 正在进行的聚合统计，按统计的内容登记。
///
/// 请求的统计正在进行时，等它数完并取得它的结果，而不是同时再数一遍同样的消息。统计结束后立即移除，
/// 之后的请求重新统计：不保存会因新消息而过期的结果。
#[derive(Default)]
pub(crate) struct Counting {
    running: Mutex<HashMap<String, Arc<Pending>>>,
}

#[derive(Default)]
struct Pending {
    result: Mutex<Option<Result<Value, ErrorParts>>>,
    done: Condvar,
}

/// 拆开的 [`AppError`]，让每个等待统计的请求都能得到自己的一份错误。
#[derive(Clone)]
struct ErrorParts {
    code: String,
    message: String,
    exit_code: i32,
    details: Option<Value>,
}

impl ErrorParts {
    fn of(e: &AppError) -> Self {
        Self {
            code: e.code.clone(),
            message: e.message.clone(),
            exit_code: e.exit_code,
            details: e.details.clone(),
        }
    }

    fn to_error(&self) -> AppError {
        let mut e = AppError::new(self.code.clone(), self.message.clone(), self.exit_code);
        e.details = self.details.clone();
        e
    }
}

impl Counting {
    /// `count()` 的结果；同一项统计（`key`）已经在进行时，取那一次的结果。
    fn share(&self, key: String, count: impl FnOnce() -> AppResult<Value>) -> AppResult<Value> {
        let (pending, mine) = {
            let mut running = self.running.lock_or_panic();
            match running.get(&key) {
                Some(p) => (p.clone(), false),
                None => {
                    let p = Arc::new(Pending::default());
                    running.insert(key.clone(), p.clone());
                    (p, true)
                }
            }
        };
        if mine {
            // 不论统计怎样结束（包括 panic），等待的请求都会得到回答，这项统计也会被移除
            struct Finish<'a> {
                counting: &'a Counting,
                key: &'a str,
                pending: &'a Pending,
            }
            impl Drop for Finish<'_> {
                fn drop(&mut self) {
                    let mut result = self
                        .pending
                        .result
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    if result.is_none() {
                        *result = Some(Err(ErrorParts::of(&AppError::runtime(
                            "the statistics stopped before they were counted",
                        ))));
                    }
                    drop(result);
                    self.counting
                        .running
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .remove(self.key);
                    self.pending.done.notify_all();
                }
            }
            let _finish = Finish {
                counting: self,
                key: &key,
                pending: &pending,
            };
            let result = count();
            *pending.result.lock_or_panic() =
                Some(result.as_ref().map(Value::clone).map_err(ErrorParts::of));
            return result;
        }
        let mut slot = pending.result.lock_or_panic();
        loop {
            if let Some(r) = slot.as_ref() {
                return r.clone().map_err(|e| e.to_error());
            }
            slot = pending
                .done
                .wait(slot)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
        }
    }
}

fn normalize_username(u: &str) -> String {
    u.trim().to_lowercase()
}

fn normalize_excluded(v: &Value) -> Vec<String> {
    let Some(items) = v.as_array() else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    items
        .iter()
        .filter_map(Value::as_str)
        .map(normalize_username)
        .filter(|s| !s.is_empty())
        .filter(|s| seen.insert(s.clone()))
        .collect()
}

fn num(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(crate::num::saturating_i64))
        .or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
        .unwrap_or(0)
}

/// 聚合结果中一个对象（`{"1": 5, …}`）的各项计数，保持原来的顺序；不是对象时为空。
fn counts_of(v: &Value) -> IndexMap<String, i64> {
    v.as_object()
        .into_iter()
        .flatten()
        .map(|(k, n)| (k.clone(), num(n)))
        .collect()
}

fn map_get<'a>(v: &'a Value, key: &str) -> Option<&'a Value> {
    v.get(key)
}

impl ServiceHub {
    pub(super) fn fresh_config(&self) -> ConfigStore {
        if !self.ctx.config_path.exists() {
            return self.config.clone();
        }
        ConfigStore::load(&self.ctx.config_path).unwrap_or_else(|_| self.config.clone())
    }

    /// 把当前档案的一个配置键写入配置文件。
    fn set_config_value(&self, key: &str, value: Value) -> AppResult<()> {
        ConfigStore::update_or(
            &self.ctx.config_path,
            || self.config.clone(),
            |cfg| cfg.set_key(Some(&self.profile_name), key, value),
        )
    }

    fn excluded_list(&self) -> Vec<String> {
        normalize_excluded(
            &self
                .fresh_config()
                .get_key(Some(&self.profile_name), "analytics_excluded_usernames"),
        )
    }

    /// 统计时排除的会话（`analytics excluded`）。
    ///
    /// # Errors
    ///
    /// 不会返回错误。
    pub fn analytics_excluded_usernames(&self) -> AppResult<Vec<String>> {
        Ok(self.excluded_list())
    }

    /// 保存统计时排除的会话（`analytics excluded --set`），返回保存的名单。
    ///
    /// 用户名去掉首尾空白、转成小写，去掉空项和重复项后保存。
    ///
    /// # Errors
    ///
    /// 写入配置文件失败时返回错误。
    pub fn analytics_set_excluded_usernames(&self, usernames: &[String]) -> AppResult<Vec<String>> {
        let normalized = normalize_excluded(&json!(usernames));
        self.set_config_value("analytics_excluded_usernames", json!(normalized))?;
        Ok(normalized)
    }

    fn private_sessions(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        cleaned_wxid: &str,
        excluded: Option<&HashSet<String>>,
    ) -> AppResult<Vec<String>> {
        let raw = wcdb
            .sessions()
            .map_err(|e| AppError::native(format!("{e:#}")))?;
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
            .map(|r| {
                ["username", "user_name", "userName"]
                    .iter()
                    .filter_map(|k| r.get(*k).and_then(Value::as_str))
                    .find(|s| !s.is_empty())
                    .unwrap_or("")
                    .to_string()
            })
            .filter(|u| is_private_session(u, cleaned_wxid))
            .filter(|u| excluded.is_empty() || !excluded.contains(&normalize_username(u)))
            .collect())
    }

    /// `session_ids` 中各会话在 `begin` 到 `end` 之间的消息，按收发方向、类型和时间统计。
    fn aggregate_raw(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session_ids: &[String],
        begin: i64,
        end: i64,
    ) -> AppResult<Value> {
        let (b, e) = normalize_range(begin, end);
        wcdb.aggregate_stats(session_ids, b, e)
            .map_err(|err| AppError::native(format!("{err:#}")))
    }

    /// 数据库层的聚合结果为空时的后备：用 `open_message_cursor` 逐批读出消息再统计。
    fn aggregate_by_cursor(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        session_ids: &[String],
        begin: i64,
        end: i64,
    ) -> AppResult<Value> {
        use chrono::{Datelike, Local, TimeZone, Timelike};
        let (mut total, mut sent, mut received, mut first, mut last) =
            (0i64, 0i64, 0i64, 0i64, 0i64);
        let mut type_counts: BTreeMap<i64, i64> = BTreeMap::new();
        let mut hourly: BTreeMap<u32, i64> = BTreeMap::new();
        let mut weekday: BTreeMap<u32, i64> = BTreeMap::new();
        let mut daily: BTreeMap<String, i64> = BTreeMap::new();
        let mut monthly: BTreeMap<String, i64> = BTreeMap::new();
        let mut sessions = Map::new();
        for session_id in session_ids {
            let Ok(cursor) = wcdb.open_message_cursor(
                session_id,
                500,
                Order::OldestFirst,
                crate::num::saturating_i32(begin.max(0)),
                crate::num::saturating_i32(end.max(0)),
                Columns::Lite,
            ) else {
                continue;
            };
            let (mut s_total, mut s_sent, mut s_recv, mut s_last) = (0i64, 0i64, 0i64, 0i64);
            loop {
                let Ok((rows, more)) = wcdb.fetch_message_batch(cursor) else {
                    break;
                };
                for row in &rows {
                    let create_time = row.create_time.unwrap_or(0);
                    if create_time == 0
                        || (begin > 0 && create_time < begin)
                        || (end > 0 && create_time > end)
                    {
                        continue;
                    }
                    let local_type = if row.local_type == 0 {
                        1
                    } else {
                        row.local_type
                    };
                    let is_send = row.is_send;
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
                        *weekday
                            .entry(d.weekday().num_days_from_sunday())
                            .or_insert(0) += 1;
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
        let obj = |m: &BTreeMap<u32, i64>| -> Value {
            Value::Object(m.iter().map(|(k, v)| (k.to_string(), json!(v))).collect())
        };
        Ok(json!({
            "total": total, "sent": sent, "received": received, "firstTime": first, "lastTime": last,
            "typeCounts": type_counts.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<Map<_, _>>(),
            "hourly": obj(&hourly), "weekday": obj(&weekday),
            "daily": daily.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
            "monthly": monthly.iter().map(|(k, v)| (k.clone(), json!(v))).collect::<Map<_, _>>(),
            "sessions": sessions
        }))
    }

    /// 数据库层的聚合结果，为空时逐批读出消息统计。
    ///
    /// 每次都从数据库读取，所以包含新消息；统计期间到来的同一项统计的请求共用这次统计（见 [`Counting`]）。
    fn aggregate_with_fallback(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        ids: &[String],
        begin: i64,
        end: i64,
    ) -> AppResult<Value> {
        let key = format!("{begin}\u{1f}{end}\u{1f}{}", ids.join("\u{1f}"));
        self.analytics_counting
            .share(key, || match self.aggregate_raw(wcdb, ids, begin, end) {
                Ok(d) if num(&d["total"]) > 0 => Ok(d),
                _ => self.aggregate_by_cursor(wcdb, ids, begin, end),
            })
    }

    /// 只统计 [`aggregate_with_fallback`](Self::aggregate_with_fallback) 结果中的 `sessions`，
    /// 不做其余部分所需的按类型和时间分组；数据库层的计数为空时逐批读出消息统计。
    fn session_totals(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        ids: &[String],
        begin: i64,
        end: i64,
    ) -> AppResult<Value> {
        let (b, e) = normalize_range(begin, end);
        match wcdb.session_totals(ids, b, e) {
            Ok(d) if num(&d["total"]) > 0 => Ok(d),
            _ => self.aggregate_by_cursor(wcdb, ids, begin, end),
        }
    }

    fn analytics_connect(&self) -> AppResult<(weflow_native::wcdb::Wcdb, String)> {
        let profile = self.profile()?;
        let wxid = profile
            .wxid
            .clone()
            .filter(|w| !w.is_empty())
            .ok_or_else(|| AppError::config("wxid is not configured"))?;
        let wcdb = self.open_wcdb()?;
        // 每项统计都先读会话列表，再读消息
        wcdb.prefetch(&[Databases::Sessions, Databases::Messages])
            .map_err(|e| AppError::native(format!("{e:#}")))?;
        Ok((wcdb, clean_account_dir_name(&wxid)))
    }

    fn alias_map(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> HashMap<String, String> {
        if usernames.is_empty() {
            return HashMap::new();
        }
        wcdb.contact_alias_map(usernames)
            .ok()
            .and_then(|v| v.as_object().cloned())
            .map(|m| {
                m.into_iter()
                    .filter_map(|(k, v)| {
                        v.as_str()
                            .filter(|a| !a.is_empty())
                            .map(|a| (k, a.to_string()))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// 可以在统计中排除的私聊，带显示名、头像和微信号；已排除而不在会话列表里的用户名也列出。
    ///
    /// # Errors
    ///
    /// 账号没有配置好（档案、wxid、数据目录或密钥）时返回配置错误，读取数据库失败时返回数据库层的错误。
    pub fn analytics_exclude_candidates(&self) -> AppResult<Vec<ExcludeCandidate>> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let excluded: HashSet<String> = self.excluded_list().into_iter().collect();
        let mut usernames: Vec<String> =
            self.private_sessions(&wcdb, &cleaned, Some(&HashSet::new()))?;
        for name in &excluded {
            if !usernames.contains(name) {
                usernames.push(name.clone());
            }
        }
        if usernames.is_empty() {
            return Ok(Vec::new());
        }
        let (names, avatars) = self.names_and_avatars(&wcdb, &usernames);
        let aliases = self.alias_map(&wcdb, &usernames);
        Ok(usernames
            .iter()
            .map(|u| {
                let alias = aliases.get(u).cloned().unwrap_or_default();
                let wechat_id = if !alias.is_empty() {
                    alias
                } else if !u.starts_with("wxid_") {
                    u.clone()
                } else {
                    String::new()
                };
                ExcludeCandidate {
                    username: u.clone(),
                    display_name: names
                        .get(u)
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| u.clone()),
                    avatar_url: avatars.get(u).cloned(),
                    wechat_id,
                }
            })
            .collect())
    }

    /// 排除名单以外的全部私聊的总体统计（`analytics overall`）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好（档案、wxid、数据目录或密钥）时返回配置错误，读取数据库失败时返回数据库层的错误，
    /// 没有私聊会话时返回运行时错误。
    pub fn analytics_overall_statistics(&self) -> AppResult<OverallStatistics> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let usernames = self.private_sessions(&wcdb, &cleaned, None)?;
        if usernames.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        self.emit_progress("analytics", "aggregating…", 30, 100);
        let d = self.aggregate_with_fallback(&wcdb, &usernames, 0, 0)?;
        let type_count = |t: i64| d["typeCounts"].get(t.to_string()).map(num).unwrap_or(0);
        let total = num(&d["total"]);
        let text = type_count(1) + type_count(244813135921);
        let (image, voice, video, emoji) = (
            type_count(3),
            type_count(34),
            type_count(43),
            type_count(47),
        );
        let other = (total - text - image - voice - video - emoji).max(0);
        // 有消息的天数，由聚合结果逐日计数（桌面版按每月 20 天估算，#307）
        let active_days = crate::num::count_i64(d["daily"].as_object().map_or(0, |m| m.len()));
        let opt = |v: i64| (v != 0).then_some(v);
        Ok(OverallStatistics {
            total_messages: total,
            text_messages: text,
            image_messages: image,
            voice_messages: voice,
            video_messages: video,
            emoji_messages: emoji,
            other_messages: other,
            sent_messages: num(&d["sent"]),
            received_messages: num(&d["received"]),
            first_message_time: opt(num(&d["firstTime"])),
            last_message_time: opt(num(&d["lastTime"])),
            active_days,
            message_type_counts: counts_of(&d["typeCounts"]),
        })
    }

    /// `begin` 到 `end` 之间消息最多的 `limit` 个私聊（`analytics rankings`），不含排除名单；`limit` 为 0 时列出全部。
    ///
    /// # Errors
    ///
    /// 同 [`analytics_overall_statistics`](Self::analytics_overall_statistics)。
    pub fn analytics_contact_rankings(
        &self,
        limit: usize,
        begin: i64,
        end: i64,
    ) -> AppResult<Vec<ContactRank>> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let usernames = self.private_sessions(&wcdb, &cleaned, None)?;
        if usernames.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        let d = self.session_totals(&wcdb, &usernames, begin, end)?;
        let sessions = d
            .get("sessions")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let names: Vec<String> = sessions.keys().cloned().collect();
        let (display, avatars) = self.names_and_avatars(&wcdb, &names);
        let aliases = self.alias_map(&wcdb, &names);
        let mut rankings: Vec<ContactRank> = names
            .iter()
            .map(|u| {
                let stat = &sessions[u];
                let alias = aliases.get(u).cloned().unwrap_or_default();
                let wechat_id = if !alias.is_empty() {
                    alias
                } else if !u.starts_with("wxid_") {
                    u.clone()
                } else {
                    String::new()
                };
                let last = num(&stat["lastTime"]);
                ContactRank {
                    username: u.clone(),
                    display_name: display
                        .get(u)
                        .filter(|n| !n.is_empty())
                        .cloned()
                        .unwrap_or_else(|| u.clone()),
                    avatar_url: avatars.get(u).cloned(),
                    wechat_id,
                    message_count: num(&stat["total"]),
                    sent_count: num(&stat["sent"]),
                    received_count: num(&stat["received"]),
                    last_message_time: (last != 0).then_some(last),
                }
            })
            .collect();
        rankings.sort_by_key(|r| std::cmp::Reverse(r.message_count));
        if limit > 0 {
            rankings.truncate(limit);
        }
        Ok(rankings)
    }

    /// 排除名单以外的全部私聊的消息按小时、星期和月份的分布（`analytics time`）。
    ///
    /// # Errors
    ///
    /// 同 [`analytics_overall_statistics`](Self::analytics_overall_statistics)。
    pub fn analytics_time_distribution(&self) -> AppResult<TimeDistribution> {
        let (wcdb, cleaned) = self.analytics_connect()?;
        let usernames = self.private_sessions(&wcdb, &cleaned, None)?;
        if usernames.is_empty() {
            return Err(AppError::runtime("no message sessions found"));
        }
        let d = self.aggregate_with_fallback(&wcdb, &usernames, 0, 0)?;
        // 数据库层给出 0 = 星期日……6 = 星期六，界面要的是 1 = 星期一……7 = 星期日
        let weekday = counts_of(&d["weekday"])
            .into_iter()
            .map(|(k, v)| {
                let n: i64 = crate::api::js_parse_int(&k).unwrap_or(0);
                ((if n == 0 { 7 } else { n }).to_string(), v)
            })
            .collect();
        let hourly = (0..24)
            .map(|i| {
                let n = map_get(&d["hourly"], &i.to_string()).map(num).unwrap_or(0);
                (i.to_string(), n)
            })
            .collect();
        Ok(TimeDistribution {
            hourly_distribution: hourly,
            weekday_distribution: weekday,
            monthly_distribution: counts_of(&d["monthly"]),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Barrier;

    #[test]
    fn requests_that_meet_share_one_count_and_later_ones_count_again() {
        let counting = Counting::default();
        let counted = AtomicUsize::new(0);
        let (started, release) = (Barrier::new(2), Barrier::new(2));
        let results: Vec<AppResult<Value>> = std::thread::scope(|s| {
            let first = s.spawn(|| {
                counting.share("k".into(), || {
                    counted.fetch_add(1, Ordering::SeqCst);
                    started.wait();
                    release.wait();
                    Ok(json!(1))
                })
            });
            // 第一个请求正在统计时，第二个请求到来
            started.wait();
            let second = s.spawn(|| counting.share("k".into(), || panic!("counted twice")));
            while counting
                .running
                .lock_or_panic()
                .get("k")
                .map(Arc::strong_count)
                != Some(3)
            {
                std::thread::yield_now();
            }
            release.wait();
            vec![first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(counted.load(Ordering::SeqCst), 1);
        assert!(results.iter().all(|r| r.as_ref().ok() == Some(&json!(1))));
        // 统计结束后什么都不保留
        assert!(counting.running.lock_or_panic().is_empty());
        assert_eq!(
            counting.share("k".into(), || Ok(json!(2))).unwrap(),
            json!(2)
        );
        let failed = counting.share("k".into(), || Err(AppError::native("broken")));
        assert_eq!(failed.unwrap_err().message, "broken");
    }

    #[test]
    fn a_count_that_panics_still_answers_who_waits() {
        let counting = Counting::default();
        let (started, release) = (Barrier::new(2), Barrier::new(2));
        let waiter = std::thread::scope(|s| {
            let first = s.spawn(|| {
                counting.share("k".into(), || {
                    started.wait();
                    release.wait();
                    panic!("the count broke")
                })
            });
            started.wait();
            let second = s.spawn(|| counting.share("k".into(), || Ok(json!(0))));
            while counting
                .running
                .lock_or_panic()
                .get("k")
                .map(Arc::strong_count)
                != Some(3)
            {
                std::thread::yield_now();
            }
            release.wait();
            assert!(first.join().is_err());
            second.join().unwrap()
        });
        assert!(waiter.is_err());
        assert!(counting.running.lock_or_panic().is_empty());
    }
}
