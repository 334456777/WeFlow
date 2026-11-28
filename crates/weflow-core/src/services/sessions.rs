//! 会话列表和联系人列表（`chat sessions`、`chat contacts --parsed`），按桌面版的方式筛选和分类，
//! 以及联系人的显示名和头像。

use std::collections::{HashMap, HashSet};

use serde_json::Value;

use super::session_kind::{is_official, session_kind, SessionKind, NOT_FRIENDS};
use super::*;
use crate::api;
use crate::chat_msg::{self, ChatMessage};
use crate::contract::{Contact, ContactKind, Session};
use crate::message::row::{row_field, row_int};
use weflow_native::wcdb::Databases;

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

/// 会话列表是否显示会话 `username`。
///
/// 显示所有不属于服务类（[`session_kind()`]）的会话，此外还显示文件传输助手和类型为 5 的企业微信联系人；
/// 不显示的是公众号（由列表另外添加）、微信的系统会话和文件夹以及客服。
fn should_keep_session(username: &str, local_type: Option<i64>) -> bool {
    if username == "filehelper" {
        return true;
    }
    if is_enterprise_openim(username) {
        return allowed_openim_by_local_type(username, local_type);
    }
    // 账号自己的会话和其他会话一样列出，所以不传自己的 wxid
    !username.to_lowercase().contains("@placeholder")
        && session_kind(username, "") != SessionKind::Service
}

fn session_local_type(row: &Value) -> Option<i64> {
    row_field(row, &["local_type", "localType", "WCDB_CT_local_type"]).and_then(|v| js_int(Some(v)))
}

fn message_label(local_type: i64) -> &'static str {
    chat_msg::message_type_label(local_type)
}

/// 按最后一条消息生成的公众号会话摘要（[`chat_msg::summary_line`]）。
fn summary_from_message(m: &ChatMessage) -> String {
    chat_msg::summary_line(m).unwrap_or_else(|| message_label(m.local_type).into())
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
    /// 联系人的头像地址和显示名；`username` 为空或数据库打不开时为 `None`。
    pub fn chat_contact_avatar(&self, username: &str) -> Option<(Option<String>, String)> {
        if username.is_empty() {
            return None;
        }
        let wcdb = self.open_wcdb().ok()?;
        let contact = wcdb.contact(username).ok().filter(Value::is_object);
        let avatar = wcdb
            .avatar_urls(&[username])
            .ok()
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

    fn contacts_compact_for(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
        usernames: &[String],
    ) -> Vec<Value> {
        match wcdb.contacts_compact(usernames) {
            Ok(Value::Array(rows)) => rows,
            _ => Vec::new(),
        }
    }

    /// 筛选、分类后的会话列表，显示名和头像取自微信的数据库。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误；读取数据库失败或会话表报错时返回数据库层的错误。
    pub fn chat_sessions_list(&self) -> AppResult<Vec<Session>> {
        let wcdb = self.open_wcdb()?;
        // 列表依次读会话、会话的联系人和消息
        wcdb.prefetch(&[
            Databases::Sessions,
            Databases::Contacts,
            Databases::Messages,
        ])
        .map_err(|e| AppError::native(format!("{e:#}")))?;
        self.sessions_with(&wcdb)
    }

    pub(super) fn sessions_with(
        &self,
        wcdb: &weflow_native::wcdb::Wcdb,
    ) -> AppResult<Vec<Session>> {
        let raw = wcdb
            .sessions()
            .map_err(|e| AppError::native(format!("{e:#}")))?;
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

        // 补上会话表里没有的公众号等会话（桌面版的 `addMissingOfficialSessions`）
        let existing: HashSet<String> = sessions.iter().map(|s| s.username.clone()).collect();
        let mut existing = existing;
        // 只取可能属于这几类的联系人，而不是每个联系人一行
        if let Ok(Value::Array(contacts)) = wcdb.contacts_maybe_special() {
            for row in contacts {
                let username = first_str(&row, &["username"]).trim().to_string();
                if username.is_empty() || existing.contains(&username) {
                    continue;
                }
                let lowered = username.to_lowercase();
                let local_type =
                    row_field(&row, &["local_type", "localType", "WCDB_CT_local_type"])
                        .and_then(|v| js_int(Some(v)));
                let official = is_official(&username);
                let special_weixin = lowered.starts_with("weixin") && lowered != "weixin";
                let special_openim = allowed_openim_by_local_type(&username, local_type);
                if !official && !special_weixin && !special_openim {
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
                    summary: if official {
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

        // 给公众号算出未读数（桌面版的 `applySyntheticUnreadCounts`），公众号没有未读数的列
        let official_session = |s: &Session| is_official(s.username.trim());
        let official: Vec<String> = sessions
            .iter()
            .filter(|s| official_session(s))
            .map(|s| s.username.clone())
            .collect();
        // 所有公众号的表在每个数据库的一次列表中找出，而不是为每个公众号搜一遍每个数据库的表结构；
        // 读不出表的会话保持原样，数据库列不出来时所有会话都保持原样
        let stats: Vec<Option<Value>> = match wcdb.message_table_stats_of(&official) {
            Ok(each) => each.into_iter().map(Result::ok).collect(),
            Err(_) => vec![None; official.len()],
        };
        for (s, stats) in sessions
            .iter_mut()
            .filter(|s| official_session(s))
            .zip(stats)
        {
            let name = s.username.clone();
            let (mut total, mut latest_snapshot) = (0i64, 0i64);
            if let Some(Value::Array(tables)) = stats {
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
            // 置顶或取消置顶时 `sort_timestamp` 也会变，并没有新消息（#894），所以最新的时间不取它
            let latest = s.last_timestamp.max(latest_snapshot);
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
                    unread = crate::num::count_i64(fresh.len());
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

        // 从联系人表取显示名和头像
        let names: Vec<String> = sessions.iter().map(|s| s.username.clone()).collect();
        let book = self.contact_book(wcdb, &names);
        for s in &mut sessions {
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

    /// 联系人标签：联系人数据库中标签 ID → 名称的对应表，没有标签时为空。
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

    /// `chat contacts --parsed`（#205）：好友、群、公众号和曾经的好友，最近联系的在前。
    ///
    /// 每一项都带 `avatarUrl`，联系人列表不必逐个请求头像。账号自己的那一行（微信把它存成好友，`local_type` 为 1）
    /// 不列出（#1079）。
    ///
    /// # Errors
    ///
    /// 账号没有配置好时返回配置错误，读不出联系人表时返回数据库层的错误。
    pub fn chat_contacts_parsed(&self) -> AppResult<Vec<Contact>> {
        let me = self.configured_wxid().unwrap_or_default();
        let wcdb = self.open_wcdb()?;
        let raw = wcdb
            .contacts()
            .map_err(|e| AppError::native(format!("{e:#}")))?;
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
            if username.is_empty() || weflow_native::db::same_identity(&username, &me) {
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
            } else if is_official(&username) {
                ContactKind::Official
            } else if openim
                || visible_weixin
                || (local_type == 1 && !NOT_FRIENDS.contains(&username.as_str()))
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
            let extra = crate::contact_extra::ContactRow::new(row);
            let labels = extra.labels(&label_names);
            let t = last_contact.get(&username).copied().unwrap_or(0);
            let contact = Contact {
                display_name: display,
                remark: set(remark),
                nickname: set(nick),
                alias: set(alias),
                labels: Some(labels).filter(|l| !l.is_empty()),
                detail_description: set(extra.signature()),
                region: set(extra.region()),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 列表显示人、群、账号自己的会话、文件传输助手和类型为 5 的企业微信联系人；
    /// 不显示公众号（另外添加）、微信的系统会话和文件夹，以及客服。
    #[test]
    fn the_list_keeps_chats_and_leaves_out_the_services() {
        for kept in ["wxid_a", "alice", "1@chatroom", "filehelper", "weixin123"] {
            assert!(should_keep_session(kept, Some(1)), "{kept}");
        }
        assert!(should_keep_session("corp@openim", Some(5)));
        for dropped in [
            "",
            "gh_abc",
            "weixin",
            "Weixin",
            "notifymessage",
            "brandsessionholder",
            "@placeholder_foldgroup",
            "placeholder_foldgroup",
            "abc@kefu.openim",
            "service_abc",
            "x@placeholder",
        ] {
            assert!(!should_keep_session(dropped, Some(1)), "{dropped:?}");
        }
        assert!(!should_keep_session("corp@openim", Some(1)));
    }
}
