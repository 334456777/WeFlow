//! Report statistics: the two-person report (`dual_report_stats`) and "my footprint" (`footprint_stats`).
//!
//! The formulas are the desktop app's, taken from the service layer's own cursor fallbacks:
//! a *conversation* is a run of messages with gaps of at most one hour; replies are measured only for the
//! account owner; phrases are short exact-match texts; days and hours are local time.
//! The annual report's extended statistics (`annual_report_extras`) use the same definitions as the service
//! layer's cursor fallback, so both paths give the same numbers.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::Instant;

use anyhow::Result;
use serde_json::{json, Map, Value};

use crate::native_db::NativeAccount;

/// A new conversation (or footprint segment) starts after this many seconds of silence.
const GAP: i64 = 3600;
/// Replies slower than this are not counted as replies.
const MAX_RESPONSE: i64 = 86_400;
/// Text kinds that can contain typed phrases.
const TEXT: i64 = 1;
const IMAGE: i64 = 3;
const VOICE: i64 = 34;
const EMOJI: i64 = 47;
/// Phrase list sizes (overall, per side).
const PHRASES_TOP: usize = 50;
const PHRASES_SIDE: usize = 200;
/// Accounts that are never "people" in the footprint.
const NOT_PEOPLE: &[&str] = &[
    "weixin", "medianote", "floatbottle", "qmessage", "qqmail", "fmessage", "notifymessage", "newsapp",
    "brandsessionholder", "brandservicesessionholder", "opencustomerservicemsg", "notification_messages",
    "userexperience_alarm", "helper_folders", "@helper_folders",
];

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn text(row: &Value, key: &str) -> String {
    row.get(key).and_then(Value::as_str).unwrap_or("").to_string()
}

/// One message of a conversation, reduced to what the reports need.
struct Msg {
    time: i64,
    local_id: i64,
    kind: i64,
    mine: bool,
    hour: usize,
    weekday_mon: usize,
    day: String,
    day_no: i64,
    content: String,
}

fn range_sql(begin: i64, end: i64) -> String {
    let mut s = String::new();
    if begin > 0 {
        s.push_str(&format!(" and m.create_time >= {begin}"));
    }
    if end > 0 {
        s.push_str(&format!(" and m.create_time <= {end}"));
    }
    s
}

/// Phrases of one side: text of 2..=20 chars without `http`, `<` or a leading `[`, counted by exact match.
/// Kept when seen at least twice, ordered by count then first appearance.
fn phrase_list(texts: &[&str], cap: usize) -> Value {
    let mut counts: HashMap<&str, (i64, usize)> = HashMap::new();
    for (i, t) in texts.iter().enumerate() {
        counts.entry(t).or_insert((0, i)).0 += 1;
    }
    let mut v: Vec<(&str, i64, usize)> = counts.into_iter().filter(|(_, (c, _))| *c >= 2).map(|(p, (c, i))| (p, c, i)).collect();
    v.sort_by(|a, b| b.1.cmp(&a.1).then(a.2.cmp(&b.2)));
    Value::Array(v.into_iter().take(cap).map(|(p, c, _)| json!({ "phrase": p, "count": c })).collect())
}

fn is_phrase(t: &str) -> bool {
    let n = t.chars().count();
    (2..=20).contains(&n) && !t.contains("http") && !t.contains('<') && !t.starts_with('[')
}

impl NativeAccount {
    /// Chronological messages of a session inside `[begin, end]` (all shards merged).
    fn conversation(&self, session_id: &str, begin: i64, end: i64) -> Result<Vec<Msg>> {
        let range = range_sql(begin, end);
        let mut rows: Vec<((i64, i64, i64), Msg)> = Vec::new();
        for t in self.message_tables(session_id)? {
            let mine = self.my_ids(&t.db)?;
            let local = |f: &str| format!("strftime('{f}', m.create_time, 'unixepoch', 'localtime')");
            let sql = format!(
                "select m.create_time as t, m.sort_seq as s, m.local_id as l, coalesce(nullif(m.local_type, 0), 1) as k, \
                 (m.real_sender_id in ({mine})) as mine, cast({h} as integer) as h, cast({w} as integer) as w, \
                 date(m.create_time, 'unixepoch', 'localtime') as d, \
                 cast(julianday(date(m.create_time, 'unixepoch', 'localtime')) + 0.5 as integer) as dn, \
                 case when m.local_type = {TEXT} then m.message_content end as c \
                 from \"{table}\" m where m.create_time > 0{range}",
                h = local("%H"),
                w = local("%w"),
                table = t.table
            );
            for r in self.query(&t.db, &sql, &[])? {
                let time = int(&r, "t");
                let seq = match int(&r, "s") { 0 => time * 1000, s => s };
                rows.push((
                    (seq, time, int(&r, "l")),
                    Msg {
                        time,
                        local_id: int(&r, "l"),
                        kind: int(&r, "k"),
                        mine: int(&r, "mine") == 1,
                        hour: int(&r, "h").clamp(0, 23) as usize,
                        weekday_mon: ((int(&r, "w") + 6) % 7) as usize,
                        day: text(&r, "d"),
                        day_no: int(&r, "dn"),
                        content: text(&r, "c"),
                    },
                ));
            }
        }
        rows.sort_by_key(|a| a.0);
        Ok(rows.into_iter().map(|(_, m)| m).collect())
    }

    /// Extended annual-report statistics over `session_ids` inside `[begin, end]`:
    /// `heatmap` (weekday Monday-first × hour), `midnight` (messages between 00:00 and 05:59 per session),
    /// `conversation` (`initiated`/`received` per session), `response` (my replies, only sessions with at least
    /// 10), `peakDay` (messages per session inside `[peak_begin, peak_end]`), `topPhrases` (my phrases, at most 32)
    /// and `streak` (the longest run of consecutive days with messages in one session; the first session wins ties).
    pub fn annual_report_extras(&self, session_ids: &[String], begin: i64, end: i64, peak_begin: i64, peak_end: i64) -> Result<Value> {
        /// Sessions need this many replies of mine to take part in the response statistics.
        const MIN_REPLIES: usize = 10;
        const PHRASES: usize = 32;
        let mut heatmap = vec![vec![0i64; 24]; 7];
        let (mut midnight, mut conversation, mut response, mut peak) = (Map::new(), Map::new(), Map::new(), Map::new());
        let mut my_texts: Vec<String> = Vec::new();
        let mut streak: Option<(String, i64, String, String)> = None;
        for sid in session_ids {
            let msgs = self.conversation(sid, begin, end)?;
            if msgs.is_empty() {
                continue;
            }
            let (mut initiated, mut received, mut night, mut in_peak) = (0i64, 0i64, 0i64, 0i64);
            let mut replies: Vec<i64> = Vec::new();
            let mut last: Option<(i64, bool)> = None;
            let (mut last_day, mut run, mut run_start) = (None::<i64>, 0i64, String::new());
            let (mut best, mut best_start, mut best_end) = (0i64, String::new(), String::new());
            for m in &msgs {
                heatmap[m.weekday_mon][m.hour] += 1;
                if m.hour < 6 {
                    night += 1;
                }
                if peak_begin > 0 && peak_end > 0 && m.time >= peak_begin && m.time <= peak_end {
                    in_peak += 1;
                }
                match last {
                    Some((t, was_mine)) if m.time - t <= GAP => {
                        if !was_mine && m.mine && m.time - t > 0 && m.time - t < MAX_RESPONSE {
                            replies.push(m.time - t);
                        }
                    }
                    _ => {
                        if m.mine { initiated += 1 } else { received += 1 }
                    }
                }
                last = Some((m.time, m.mine));
                if m.kind == TEXT && m.mine {
                    let t = m.content.trim();
                    if is_phrase(t) {
                        my_texts.push(t.to_string());
                    }
                }
                if last_day != Some(m.day_no) {
                    if last_day.is_some_and(|l| m.day_no - l == 1) {
                        run += 1;
                    } else {
                        run = 1;
                        run_start = m.day.clone();
                    }
                    if run > best {
                        (best, best_start, best_end) = (run, run_start.clone(), m.day.clone());
                    }
                    last_day = Some(m.day_no);
                }
            }
            if night > 0 {
                midnight.insert(sid.clone(), json!(night));
            }
            conversation.insert(sid.clone(), json!({ "initiated": initiated, "received": received }));
            if replies.len() >= MIN_REPLIES {
                response.insert(sid.clone(), json!({ "count": replies.len(), "avg": replies.iter().sum::<i64>() as f64 / replies.len() as f64 }));
            }
            if in_peak > 0 {
                peak.insert(sid.clone(), json!(in_peak));
            }
            if best > streak.as_ref().map_or(0, |s| s.1) {
                streak = Some((sid.clone(), best, best_start, best_end));
            }
        }
        let refs: Vec<&str> = my_texts.iter().map(String::as_str).collect();
        let mut out = json!({
            "heatmap": heatmap, "midnight": midnight, "conversation": conversation, "response": response,
            "peakDay": peak, "topPhrases": phrase_list(&refs, PHRASES),
        });
        if let Some((sid, days, start, end)) = streak {
            out["streak"] = json!({ "sessionId": sid, "days": days, "startDate": start, "endDate": end });
        }
        Ok(out)
    }

    /// Two-person report statistics for `friend` (see the module docs for the definitions).
    pub fn dual_report_stats(&self, friend: &str, begin: i64, end: i64) -> Result<Value> {
        let msgs = self.conversation(friend, begin, end)?;
        let (mut words, mut images, mut voices, mut emojis) = (0i64, 0i64, 0i64, 0i64);
        let mut heatmap = vec![vec![0i64; 24]; 7];
        let mut monthly: BTreeMap<String, i64> = BTreeMap::new();
        let (mut initiated, mut received) = (0i64, 0i64);
        let mut responses: Vec<i64> = Vec::new();
        let mut last: Option<(i64, bool)> = None;
        let (mut mine_texts, mut their_texts): (Vec<&str>, Vec<&str>) = (Vec::new(), Vec::new());
        // longest run of consecutive local days
        let (mut last_day, mut run, mut run_start) = (None::<i64>, 0i64, String::new());
        let (mut best, mut best_start, mut best_end) = (0i64, String::new(), String::new());

        for m in &msgs {
            match m.kind {
                IMAGE => images += 1,
                VOICE => voices += 1,
                EMOJI => emojis += 1,
                TEXT => {
                    let t = m.content.trim();
                    words += t.chars().count() as i64;
                    if is_phrase(t) {
                        if m.mine { mine_texts.push(t) } else { their_texts.push(t) }
                    }
                }
                _ => {}
            }
            heatmap[m.weekday_mon][m.hour] += 1;
            if m.day.len() >= 7 {
                *monthly.entry(m.day[..7].to_string()).or_default() += 1;
            }
            match last {
                Some((t, was_mine)) if m.time - t <= GAP => {
                    if !was_mine && m.mine {
                        let rt = m.time - t;
                        if rt > 0 && rt < MAX_RESPONSE {
                            responses.push(rt);
                        }
                    }
                }
                _ => {
                    if m.mine { initiated += 1 } else { received += 1 }
                }
            }
            last = Some((m.time, m.mine));
            if last_day != Some(m.day_no) {
                if last_day.is_some_and(|l| m.day_no - l == 1) {
                    run += 1;
                } else {
                    run = 1;
                    run_start = m.day.clone();
                }
                if run > best {
                    (best, best_start, best_end) = (run, run_start.clone(), m.day.clone());
                }
                last_day = Some(m.day_no);
            }
        }

        let all_texts: Vec<&str> = mine_texts.iter().chain(their_texts.iter()).copied().collect();
        let mut out = json!({
            "counts": { "total": msgs.len(), "words": words, "image": images, "voice": voices, "emoji": emojis },
            // empty on purpose: the service layer then tallies the stickers itself
            "emojis": [],
            "phrases": phrase_list(&all_texts, PHRASES_TOP),
            "myPhrases": phrase_list(&mine_texts, PHRASES_SIDE),
            "friendPhrases": phrase_list(&their_texts, PHRASES_SIDE),
            "heatmap": heatmap,
            "initiative": { "initiated": initiated, "received": received },
            "monthly": monthly,
        });
        if !responses.is_empty() {
            let sum: i64 = responses.iter().sum();
            out["response"] = json!({
                "avg": (sum as f64 / responses.len() as f64).round() as i64,
                "fastest": responses.iter().min(), "slowest": responses.iter().max(), "count": responses.len()
            });
        }
        if best > 0 {
            out["streak"] = json!({ "days": best, "startDate": best_start, "endDate": best_end });
        }
        Ok(out)
    }

    /// Whether a message's `source` XML lists the account owner (or everyone) in `<atuserlist>`.
    fn mentions_me(&self, source: &str) -> bool {
        let lower = source.to_ascii_lowercase();
        let Some(start) = lower.find("<atuserlist") else { return false };
        let Some(open_end) = lower[start..].find('>') else { return false };
        let body_start = start + open_end + 1;
        let Some(len) = lower[body_start..].find("</atuserlist>") else { return false };
        let body = &source[body_start..body_start + len];
        let body = body.trim().trim_start_matches("<![CDATA[").trim_end_matches("]]>");
        body.split(',').map(|t| t.trim().trim_start_matches('@')).any(|t| t.eq_ignore_ascii_case("notify@all") || self.is_me(t))
    }

    /// `{ summary, private_sessions, private_segments, mentions, mention_groups, diagnostics }`.
    pub fn footprint_stats(&self, options: &Value) -> Result<Value> {
        let started = Instant::now();
        let pick = |keys: &[&str]| keys.iter().find_map(|k| options.get(*k));
        let num = |keys: &[&str]| pick(keys).and_then(Value::as_i64).unwrap_or(0).max(0);
        let (begin, end) = (num(&["begin", "beginTimestamp"]), num(&["end", "endTimestamp"]));
        let (mention_limit, private_limit) = (num(&["mention_limit", "mentionLimit"]) as usize, num(&["private_limit", "privateLimit"]) as usize);
        let list = |key: &str| -> Vec<String> {
            options.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
        };
        let (mut private_ids, mut group_ids) = (list("private_session_ids"), list("group_session_ids"));
        if private_ids.is_empty() && group_ids.is_empty() {
            for u in crate::native_msg::session_usernames(self)? {
                if u.ends_with("@chatroom") {
                    group_ids.push(u);
                } else if !u.starts_with("gh_") && !NOT_PEOPLE.contains(&u.as_str()) {
                    private_ids.push(u);
                }
            }
        }

        // private chats
        let mut sessions: Vec<Value> = Vec::new();
        let mut segments: Vec<Value> = Vec::new();
        for sid in &private_ids {
            let msgs = self.conversation(sid, begin, end)?;
            if msgs.is_empty() {
                continue;
            }
            let (mut inc, mut out, mut first_in, mut first_reply) = (0i64, 0i64, 0i64, 0i64);
            let (mut latest, mut anchor) = (0i64, 0i64);
            let mut seg: Option<Segment> = None;
            let mut seg_no = 0i64;
            let mut last_ts = 0i64;
            for m in &msgs {
                if seg.is_none() || (last_ts > 0 && m.time - last_ts > GAP) {
                    if let Some(s) = seg.take() {
                        segments.push(s.finish(sid));
                    }
                    seg_no += 1;
                    seg = Some(Segment::new(seg_no, m.time, m.local_id));
                }
                let s = seg.as_mut().expect("segment exists");
                if m.mine {
                    out += 1;
                    if first_in > 0 && m.time >= first_in && first_reply <= 0 {
                        first_reply = m.time;
                    }
                    s.out += 1;
                    if s.first_in > 0 && m.time >= s.first_in && s.first_reply <= 0 {
                        s.first_reply = m.time;
                    }
                } else {
                    inc += 1;
                    if first_in <= 0 || m.time < first_in {
                        first_in = m.time;
                    }
                    s.inc += 1;
                    if s.first_in <= 0 || m.time < s.first_in {
                        s.first_in = m.time;
                    }
                }
                if latest <= 0 || m.time > latest || (m.time == latest && m.local_id > anchor) {
                    (latest, anchor) = (m.time, m.local_id);
                }
                s.end = m.time;
                last_ts = m.time;
            }
            if let Some(s) = seg.take() {
                segments.push(s.finish(sid));
            }
            sessions.push(json!({
                "session_id": sid, "incoming_count": inc, "outgoing_count": out, "replied": inc > 0 && out > 0,
                "first_incoming_ts": first_in, "first_reply_ts": first_reply, "latest_ts": latest,
                "anchor_local_id": anchor, "anchor_create_time": latest
            }));
        }
        sessions.sort_by(|a, b| int(b, "latest_ts").cmp(&int(a, "latest_ts")).then(text(a, "session_id").cmp(&text(b, "session_id"))));
        segments.sort_by(|a, b| int(a, "start_ts").cmp(&int(b, "start_ts")).then(text(a, "session_id").cmp(&text(b, "session_id"))).then(int(a, "segment_index").cmp(&int(b, "segment_index"))));
        let private_truncated = private_limit > 0 && sessions.len() > private_limit;
        if private_truncated {
            sessions.truncate(private_limit);
            let keep: HashSet<String> = sessions.iter().map(|s| text(s, "session_id")).collect();
            segments.retain(|s| keep.contains(&text(s, "session_id")));
        }

        // group @-mentions
        let range = range_sql(begin, end);
        let mut mentions: Vec<Value> = Vec::new();
        for sid in &group_ids {
            for t in self.message_tables(sid)? {
                let sql = format!(
                    "select m.local_id as local_id, m.create_time as create_time, m.message_content as message_content, m.source as source, \
                     n.user_name as sender_username from \"{}\" m left join Name2Id n on n.rowid = m.real_sender_id \
                     where m.local_type = {TEXT}{range} and (typeof(m.source) = 'blob' or m.source like '%atuserlist%')",
                    t.table
                );
                for r in self.query(&t.db, &sql, &[])? {
                    let content = text(&r, "message_content");
                    if !(content.contains('@') || content.contains('\u{ff20}')) || !self.mentions_me(&text(&r, "source")) {
                        continue;
                    }
                    mentions.push(json!({
                        "session_id": sid, "local_id": int(&r, "local_id"), "create_time": int(&r, "create_time"),
                        "sender_username": text(&r, "sender_username"), "message_content": content, "source": text(&r, "source")
                    }));
                }
            }
        }
        mentions.sort_by(|a, b| int(b, "create_time").cmp(&int(a, "create_time")).then(int(b, "local_id").cmp(&int(a, "local_id"))));
        let mention_truncated = mention_limit > 0 && mentions.len() > mention_limit;
        if mention_truncated {
            mentions.truncate(mention_limit);
        }
        let mut groups: HashMap<String, (i64, i64)> = HashMap::new();
        for m in &mentions {
            let g = groups.entry(text(m, "session_id")).or_default();
            g.0 += 1;
            g.1 = g.1.max(int(m, "create_time"));
        }
        let mut mention_groups: Vec<(String, i64, i64)> = groups.into_iter().map(|(s, (c, l))| (s, c, l)).collect();
        mention_groups.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.0.cmp(&b.0)));

        let inbound = sessions.iter().filter(|s| int(s, "incoming_count") > 0).count();
        let replied = sessions.iter().filter(|s| s["replied"] == true).count();
        let outbound = sessions.iter().filter(|s| int(s, "outgoing_count") > 0).count();
        Ok(json!({
            "summary": {
                "private_inbound_people": inbound, "private_replied_people": replied, "private_outbound_people": outbound,
                "private_reply_rate": if inbound > 0 { replied as f64 / inbound as f64 } else { 0.0 },
                "mention_count": mentions.len(), "mention_group_count": mention_groups.len()
            },
            "private_sessions": sessions, "private_segments": segments, "mentions": mentions,
            "mention_groups": mention_groups.into_iter().map(|(s, c, l)| json!({ "session_id": s, "count": c, "latest_ts": l })).collect::<Vec<_>>(),
            "diagnostics": {
                "truncated": private_truncated || mention_truncated, "private_truncated": private_truncated, "mention_truncated": mention_truncated,
                "scanned_dbs": self.message_dbs().len(), "elapsed_ms": started.elapsed().as_millis() as u64,
                "private_session_count": private_ids.len(), "group_session_count": group_ids.len(), "fallback_used": false
            }
        }))
    }
}

/// A private-chat segment under construction.
struct Segment {
    index: i64,
    start: i64,
    end: i64,
    inc: i64,
    out: i64,
    first_in: i64,
    first_reply: i64,
    anchor_local_id: i64,
}

impl Segment {
    fn new(index: i64, start: i64, anchor_local_id: i64) -> Self {
        Self { index, start, end: start, inc: 0, out: 0, first_in: 0, first_reply: 0, anchor_local_id }
    }

    fn finish(self, session_id: &str) -> Value {
        let mut o = Map::new();
        o.insert("session_id".into(), json!(session_id));
        o.insert("segment_index".into(), json!(self.index));
        o.insert("start_ts".into(), json!(self.start));
        o.insert("end_ts".into(), json!(self.end));
        o.insert("duration_sec".into(), json!((self.end - self.start).max(0)));
        o.insert("incoming_count".into(), json!(self.inc));
        o.insert("outgoing_count".into(), json!(self.out));
        o.insert("message_count".into(), json!(self.inc + self.out));
        o.insert("replied".into(), json!(self.inc > 0 && self.out > 0));
        o.insert("first_incoming_ts".into(), json!(self.first_in));
        o.insert("first_reply_ts".into(), json!(self.first_reply));
        o.insert("latest_ts".into(), json!(self.end));
        o.insert("anchor_local_id".into(), json!(self.anchor_local_id));
        o.insert("anchor_create_time".into(), json!(self.start));
        Value::Object(o)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, MsgSpec, SessionSpec, DAY, T0};

    fn world(tag: &str, sessions: &[SessionSpec], shard: &[(&str, Vec<MsgSpec>)]) -> NativeAccount {
        // tests run in parallel and several build the same world: every call gets its own directory
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("weflow-report-{}-{tag}-{n}", std::process::id()));
        let f = Fixture::new(&root, "wxid_me_ab12");
        f.session_db(sessions);
        f.message_shard(0, shard);
        NativeAccount::new(f.db_storage(), &f.key_hex()).unwrap().with_my_wxid(Some("wxid_me_ab12".into()))
    }

    fn session(username: &'static str) -> SessionSpec {
        SessionSpec { username, summary: "", last_timestamp: T0, unread: 0, last_msg_type: 1 }
    }

    fn dual() -> NativeAccount {
        let m = |id, who: &'static str, dt: i64, text: &str| MsgSpec::text(id, who, T0 + dt, text);
        world(
            "dual",
            &[session("wxid_pal")],
            &[(
                "wxid_pal",
                vec![
                    m(1, "wxid_pal", 0, "ok"),
                    m(2, "wxid_me", 10, "ok"),
                    m(3, "wxid_pal", 20, "haha"),
                    m(4, "wxid_me", 30, "haha"),
                    m(5, "wxid_me", 7_200, "ok"),
                    m(6, "wxid_pal", 7_210, "ok"),
                    m(7, "wxid_me", DAY, "ok"),
                    m(8, "wxid_pal", DAY + 100, "<img/>").of_type(3),
                    m(9, "wxid_pal", 2 * DAY, "").of_type(34),
                    m(10, "wxid_me", 2 * DAY + 5, "").of_type(47),
                ],
            )],
        )
    }

    #[test]
    fn dual_report_counts_words_and_kinds() {
        let v = dual().dual_report_stats("wxid_pal", 0, 0).unwrap();
        assert_eq!(v["counts"], json!({"total": 10, "words": 18, "image": 1, "voice": 1, "emoji": 1}));
        assert_eq!(v["emojis"], json!([]), "the service layer tallies stickers itself");
        let heat = v["heatmap"].as_array().unwrap();
        assert_eq!((heat.len(), heat[0].as_array().unwrap().len()), (7, 24));
        let total: i64 = heat.iter().flat_map(|r| r.as_array().unwrap()).map(|n| n.as_i64().unwrap()).sum();
        assert_eq!(total, 10);
        let monthly: i64 = v["monthly"].as_object().unwrap().values().map(|n| n.as_i64().unwrap()).sum();
        assert_eq!(monthly, 10);
        assert!(v["monthly"].as_object().unwrap().keys().all(|k| k.len() == 7));
    }

    #[test]
    fn dual_report_initiative_and_response_follow_the_one_hour_rule() {
        let v = dual().dual_report_stats("wxid_pal", 0, 0).unwrap();
        // conversations start at messages 1 (pal), 5 (me), 7 (me), 9 (pal)
        assert_eq!(v["initiative"], json!({"initiated": 2, "received": 2}));
        // my replies to pal inside a conversation: 10 s, 10 s and 5 s
        assert_eq!(v["response"], json!({"avg": 8, "fastest": 5, "slowest": 10, "count": 3}));
    }

    #[test]
    fn dual_report_phrases_and_streak() {
        let v = dual().dual_report_stats("wxid_pal", 0, 0).unwrap();
        assert_eq!(v["phrases"], json!([{"phrase": "ok", "count": 5}, {"phrase": "haha", "count": 2}]));
        assert_eq!(v["myPhrases"], json!([{"phrase": "ok", "count": 3}]), "haha was typed once: below the 2-occurrence floor");
        assert_eq!(v["friendPhrases"], json!([{"phrase": "ok", "count": 2}]));
        assert_eq!(v["streak"]["days"], 3);
        let (s, e) = (v["streak"]["startDate"].as_str().unwrap(), v["streak"]["endDate"].as_str().unwrap());
        assert!(s.len() == 10 && e.len() == 10 && s < e, "{s} .. {e}");
    }

    #[test]
    fn dual_report_respects_the_range_and_handles_empty_conversations() {
        let a = dual();
        let v = a.dual_report_stats("wxid_pal", T0 + DAY, 0).unwrap();
        assert_eq!(v["counts"]["total"], 4);
        let none = a.dual_report_stats("nobody", 0, 0).unwrap();
        assert_eq!(none["counts"]["total"], 0);
        assert!(none.get("response").is_none() && none.get("streak").is_none());
        assert_eq!(none["initiative"], json!({"initiated": 0, "received": 0}));
    }

    #[test]
    fn phrase_rule_filters_links_markup_and_length() {
        assert!(is_phrase("ok") && is_phrase("好的呀"));
        assert!(!is_phrase("a") && !is_phrase(&"x".repeat(21)));
        assert!(!is_phrase("see http://x") && !is_phrase("<b>hi</b>") && !is_phrase("[Smile]"));
        assert_eq!(phrase_list(&["b", "a", "b", "a", "c"], 10), json!([{"phrase": "b", "count": 2}, {"phrase": "a", "count": 2}]), "ties keep first-seen order");
        assert_eq!(phrase_list(&["a", "a", "a", "b", "b"], 1), json!([{"phrase": "a", "count": 3}]));
    }

    fn footprint() -> NativeAccount {
        let at = |who: &str| format!("<msgsource><atuserlist>{who}</atuserlist></msgsource>");
        world(
            "footprint",
            &["wxid_pal", "wxid_lone", "wxid_quiet", "gh_x", "medianote", "g1@chatroom", "g2@chatroom"].map(session),
            &[
                (
                    "wxid_pal",
                    vec![
                        MsgSpec::text(1, "wxid_pal", T0, "a"),
                        MsgSpec::text(2, "wxid_me", T0 + 60, "b"),
                        MsgSpec::text(3, "wxid_pal", T0 + DAY, "c"),
                        MsgSpec::text(4, "wxid_me", T0 + DAY + 30, "d"),
                        MsgSpec::text(5, "wxid_pal", T0 + 3 * DAY, "e"),
                    ],
                ),
                ("wxid_lone", vec![MsgSpec::text(1, "wxid_lone", T0 + 5, "x"), MsgSpec::text(2, "wxid_lone", T0 + 6, "y")]),
                ("wxid_quiet", vec![MsgSpec::text(1, "wxid_me", T0 + 70, "hello?")]),
                (
                    "g1@chatroom",
                    vec![
                        MsgSpec::text(1, "wxid_bob", T0 + 10, "wxid_bob:\n@Me hello").with_source(&at("wxid_me")),
                        MsgSpec::text(2, "wxid_bob", T0 + 20, "wxid_bob:\n@Carol hi").with_source(&at("wxid_carol")),
                        MsgSpec::text(3, "wxid_quiet", T0 + 100, "wxid_quiet:\n@all meeting").with_source(&at("notify@all")),
                        MsgSpec::text(4, "wxid_bob", T0 + 150, "no at sign here").with_source(&at("wxid_me")),
                        MsgSpec::text(5, "wxid_bob", T0 + 200, "wxid_bob:\n@Me again").with_source(&at("wxid_me_ab12,wxid_carol")),
                    ],
                ),
                ("g2@chatroom", vec![MsgSpec::text(1, "wxid_bob", T0 + 50, "@Me there").with_source(&at("wxid_me"))]),
            ],
        )
    }

    #[test]
    fn footprint_private_sessions_summary_and_segments() {
        let v = footprint().footprint_stats(&json!({"myWxid": "wxid_me_ab12"})).unwrap();
        let ids: Vec<&str> = v["private_sessions"].as_array().unwrap().iter().map(|s| s["session_id"].as_str().unwrap()).collect();
        assert_eq!(ids, ["wxid_pal", "wxid_quiet", "wxid_lone"], "people only, newest activity first; gh_/medianote are not people");
        let pal = &v["private_sessions"][0];
        assert_eq!((pal["incoming_count"].as_i64(), pal["outgoing_count"].as_i64(), pal["replied"].as_bool()), (Some(3), Some(2), Some(true)));
        assert_eq!((pal["first_incoming_ts"].as_i64(), pal["first_reply_ts"].as_i64(), pal["latest_ts"].as_i64()), (Some(T0), Some(T0 + 60), Some(T0 + 3 * DAY)));
        assert_eq!(pal["anchor_local_id"], 5);
        assert_eq!(v["private_sessions"][2]["replied"], false);
        assert_eq!(v["summary"]["private_inbound_people"], 2);
        assert_eq!(v["summary"]["private_replied_people"], 1);
        assert_eq!(v["summary"]["private_outbound_people"], 2);
        assert_eq!(v["summary"]["private_reply_rate"], 0.5);

        let segs: Vec<&Value> = v["private_segments"].as_array().unwrap().iter().filter(|s| s["session_id"] == "wxid_pal").collect();
        assert_eq!(segs.len(), 3, "gaps of more than an hour split the chat");
        assert_eq!((segs[0]["message_count"].as_i64(), segs[0]["duration_sec"].as_i64(), segs[0]["replied"].as_bool()), (Some(2), Some(60), Some(true)));
        assert_eq!((segs[2]["message_count"].as_i64(), segs[2]["replied"].as_bool(), segs[2]["segment_index"].as_i64()), (Some(1), Some(false), Some(3)));
        assert_eq!(segs[1]["anchor_local_id"], 3);
    }

    #[test]
    fn footprint_mentions_use_the_atuserlist_and_count_at_all() {
        let v = footprint().footprint_stats(&json!({"myWxid": "wxid_me_ab12"})).unwrap();
        let got: Vec<(String, i64)> = v["mentions"].as_array().unwrap().iter().map(|m| (m["session_id"].as_str().unwrap().to_string(), m["local_id"].as_i64().unwrap())).collect();
        // newest first; @Carol is not me; "no at sign here" has no @ in its text
        assert_eq!(got, [("g1@chatroom".into(), 5), ("g1@chatroom".into(), 3), ("g2@chatroom".into(), 1), ("g1@chatroom".into(), 1)]);
        assert_eq!(v["mentions"][0]["sender_username"], "wxid_bob");
        assert_eq!((v["summary"]["mention_count"].as_i64(), v["summary"]["mention_group_count"].as_i64()), (Some(4), Some(2)));
        assert_eq!(v["mention_groups"], json!([{"session_id": "g1@chatroom", "count": 3, "latest_ts": T0 + 200}, {"session_id": "g2@chatroom", "count": 1, "latest_ts": T0 + 50}]));
    }

    #[test]
    fn footprint_options_limit_scope_and_truncate() {
        let a = footprint();
        let v = a.footprint_stats(&json!({"begin": T0 + 60, "end": T0 + 150, "mention_limit": 1, "private_limit": 1})).unwrap();
        // in [T0+60, T0+150] only g1#3 (@all at T0+100) is a mention; g2#1 (T0+50) is before the range
        assert_eq!(v["mentions"].as_array().unwrap().len(), 1);
        assert_eq!(v["diagnostics"]["mention_truncated"], false, "the limit was not exceeded");
        // pal (my reply at T0+60) and quiet (T0+70) are active in range, so a limit of 1 truncates
        assert_eq!(v["private_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(v["diagnostics"]["private_truncated"], true);
        assert_eq!(v["diagnostics"]["truncated"], true);
        let scoped = a.footprint_stats(&json!({"private_session_ids": ["wxid_lone"], "group_session_ids": []})).unwrap();
        assert_eq!(scoped["private_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(scoped["mentions"], json!([]), "explicit scope: no groups were asked for");
        let cut = a.footprint_stats(&json!({"private_limit": 1})).unwrap();
        assert_eq!(cut["private_sessions"].as_array().unwrap().len(), 1);
        assert_eq!(cut["diagnostics"]["private_truncated"], true);
        assert!(cut["private_segments"].as_array().unwrap().iter().all(|s| s["session_id"] == "wxid_pal"));
    }

    #[test]
    fn mention_parsing_edge_cases() {
        let a = account_for_mentions();
        assert!(a.mentions_me("<msgsource><atuserlist><![CDATA[wxid_me, wxid_x]]></atuserlist></msgsource>"));
        assert!(a.mentions_me("<ATUSERLIST>@wxid_me</ATUSERLIST>"));
        assert!(a.mentions_me("<atuserlist>notify@all</atuserlist>"));
        assert!(!a.mentions_me("<atuserlist>wxid_other</atuserlist>"));
        assert!(!a.mentions_me("<msgsource><other/></msgsource>"));
        assert!(!a.mentions_me(""));
        assert!(!a.mentions_me("<atuserlist>wxid_me"), "unterminated list");
    }

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn annual_extras_follow_the_service_layer_definitions() {
        let a = dual();
        let v = a.annual_report_extras(&ids(&["wxid_pal"]), 0, 0, 0, 0).unwrap();
        let heat: i64 = v["heatmap"].as_array().unwrap().iter().flat_map(|r| r.as_array().unwrap()).map(|n| n.as_i64().unwrap()).sum();
        assert_eq!((v["heatmap"].as_array().unwrap().len(), heat), (7, 10));
        assert_eq!(v["conversation"], json!({"wxid_pal": {"initiated": 2, "received": 2}}));
        assert_eq!(v["response"], json!({}), "3 replies is below the 10-reply floor");
        assert_eq!(v["topPhrases"], json!([{"phrase": "ok", "count": 3}]), "only my phrases, seen at least twice");
        assert_eq!((v["streak"]["sessionId"].clone(), v["streak"]["days"].clone()), (json!("wxid_pal"), json!(3)));
        assert!(v["streak"]["startDate"].as_str().unwrap() < v["streak"]["endDate"].as_str().unwrap());
        assert_eq!(v["peakDay"], json!({}), "no peak window asked for");
        assert!(v["midnight"].as_object().unwrap().values().all(|n| n.as_i64().unwrap() <= 10));

        let peak = a.annual_report_extras(&ids(&["wxid_pal"]), 0, 0, T0, T0 + 100).unwrap();
        assert_eq!(peak["peakDay"], json!({"wxid_pal": 4}));
        let ranged = a.annual_report_extras(&ids(&["wxid_pal"]), T0 + DAY, 0, 0, 0).unwrap();
        assert_eq!(ranged["conversation"], json!({"wxid_pal": {"initiated": 1, "received": 1}}));
        let none = a.annual_report_extras(&ids(&["nobody"]), 0, 0, 0, 0).unwrap();
        assert!(none.get("streak").is_none() && none["conversation"] == json!({}) && none["topPhrases"] == json!([]));
    }

    #[test]
    fn annual_extras_response_needs_ten_replies_and_streak_ties_go_to_the_first_session() {
        let mut quick = Vec::new();
        for i in 0..10 {
            quick.push(MsgSpec::text(i * 2 + 1, "wxid_fast", T0 + i * 7_200, "ping"));
            quick.push(MsgSpec::text(i * 2 + 2, "wxid_me", T0 + i * 7_200 + 30, "pong"));
        }
        let a = world(
            "annual-response",
            &[session("wxid_fast"), session("wxid_slow")],
            &[
                ("wxid_fast", quick),
                ("wxid_slow", vec![MsgSpec::text(1, "wxid_slow", T0, "hi"), MsgSpec::text(2, "wxid_me", T0 + 20, "yo"), MsgSpec::text(3, "wxid_slow", T0 + DAY, "again")]),
            ],
        );
        let v = a.annual_report_extras(&ids(&["wxid_slow", "wxid_fast"]), 0, 0, 0, 0).unwrap();
        assert_eq!(v["response"], json!({"wxid_fast": {"count": 10, "avg": 30.0}}));
        assert_eq!(v["conversation"]["wxid_fast"], json!({"initiated": 0, "received": 10}));
        // wxid_slow spans 2 days; wxid_fast spans at most 2 (18 hours) and slow is listed first, so slow wins
        assert_eq!((v["streak"]["sessionId"].clone(), v["streak"]["days"].clone()), (json!("wxid_slow"), json!(2)));
        let tie = a.annual_report_extras(&ids(&["wxid_fast", "wxid_slow"]), 0, T0 + 2 * 3_600, 0, 0).unwrap();
        assert_eq!(tie["streak"]["sessionId"], "wxid_fast", "a tie keeps the first session");
    }

    #[test]
    fn annual_stats_add_month_of_year_counts_per_session() {
        let a = dual();
        let v = a.annual_report_stats(&ids(&["wxid_pal"]), 0, 0).unwrap();
        let plain = a.aggregate_stats(&ids(&["wxid_pal"]), 0, 0).unwrap();
        assert_eq!(v["total"], plain["total"]);
        assert!(plain["sessions"]["wxid_pal"].get("monthly").is_none(), "the plain aggregate keeps its shape");
        let monthly = v["sessions"]["wxid_pal"]["monthly"].as_object().unwrap();
        assert!(monthly.keys().all(|k| (1..=12).contains(&k.parse::<i64>().unwrap())), "{monthly:?}");
        assert_eq!(monthly.values().map(|n| n.as_i64().unwrap()).sum::<i64>(), 10);
        assert_eq!((v["sessions"]["wxid_pal"]["sent"].clone(), v["sessions"]["wxid_pal"]["received"].clone()), (json!(5), json!(5)));
    }

    fn account_for_mentions() -> NativeAccount {
        world("mentions", &[], &[]).with_my_wxid(Some("wxid_me".into()))
    }
}
