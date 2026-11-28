//! 消息搜索、媒体扫描、语音数据（`media_*.db`）、微信的媒体索引中图片和视频的位置（`hardlink.db`），
//! 以及表情的查找（`emoticon.db`）。

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
use rusqlite::ToSql;
use serde_json::{json, Value};

use crate::db::Account;
use crate::msg_row::MsgRow;

/// [`Account::search_messages`] 或 [`Account::scan_media_stream`] 找到的一条消息，以及它所在的会话。
#[derive(Clone, Debug, PartialEq)]
pub struct SessionRow {
    /// 会话的用户名。
    pub session_id: String,
    /// 消息本身。
    pub row: MsgRow,
}

/// 图片消息的 `local_type`。
const IMAGE: i64 = 3;
/// 视频消息的 `local_type`。
const VIDEO: i64 = 43;
/// 按 `create_time` 匹配语音行时允许的时钟误差（秒）。
const VOICE_TIME_WINDOW: i64 = 5;

fn int(row: &Value, key: &str) -> i64 {
    row.get(key).and_then(Value::as_i64).unwrap_or(0)
}

fn text(row: &Value, key: &str) -> String {
    row.get(key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string()
}

/// 媒体索引中保存的一个文件夹名或文件名：不为空、不含路径分隔符、不是 `.` 或 `..`，拼出的路径才不会跑出账号目录。
fn is_plain_name(name: &str) -> bool {
    !name.is_empty() && name != "." && name != ".." && !name.contains(['/', '\\', ':'])
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

fn sort_key(row: &Value) -> (i64, i64, i64) {
    let create = int(row, "create_time");
    (
        match int(row, "sort_seq") {
            0 => create * 1000,
            s => s,
        },
        create,
        int(row, "local_id"),
    )
}

/// 按「最新在前，相同时按找到的先后」排序的前 `k` 行（即对全部行做稳定的最新在前排序后排在最前的几行），
/// 不保存其余的行。
struct Newest<T = Value> {
    k: usize,
    found: usize,
    heap: std::collections::BinaryHeap<Ranked<T>>,
}

struct Ranked<T> {
    key: (i64, i64, i64),
    seq: usize,
    row: T,
}

impl<T> Ranked<T> {
    /// 排序使堆顶是排在最后的行。
    fn rank(&self, other: &Self) -> std::cmp::Ordering {
        other.key.cmp(&self.key).then(self.seq.cmp(&other.seq))
    }
}

impl<T> PartialEq for Ranked<T> {
    fn eq(&self, other: &Self) -> bool {
        self.rank(other).is_eq()
    }
}
impl<T> Eq for Ranked<T> {}
impl<T> PartialOrd for Ranked<T> {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}
impl<T> Ord for Ranked<T> {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.rank(other)
    }
}

impl Newest {
    fn push(&mut self, row: Value) {
        self.push_keyed(sort_key(&row), row);
    }
}

impl<T> Newest<T> {
    fn new(k: usize) -> Self {
        Self {
            k,
            found: 0,
            heap: Default::default(),
        }
    }

    /// 加入 `row`，按 `key`（`(sort_seq, create_time, local_id)`，与 [`sort_key`] 读出的一样）排位置。
    fn push_keyed(&mut self, key: (i64, i64, i64), row: T) {
        let seq = self.found;
        self.found += 1;
        if self.k == 0 {
            return;
        }
        self.heap.push(Ranked { key, seq, row });
        if self.heap.len() > self.k {
            self.heap.pop();
        }
    }

    /// 已保存 `k` 行时排在最后的那一行的键：键比它小的行不可能进入前 `k` 行。
    fn last_kept(&self) -> Option<(i64, i64, i64)> {
        (self.k > 0 && self.heap.len() == self.k).then(|| self.heap.peek().map(|r| r.key))?
    }

    /// 加入另一个 `Newest` 保存的行，如同按它们被找到的先后接在已加入的行之后加到这里。
    /// 它丢掉的行在这里也进不了前 `k` 行。
    fn absorb(&mut self, other: Newest<T>) {
        let base = self.found;
        self.found += other.found;
        for mut r in other.heap.into_vec() {
            r.seq += base;
            self.heap.push(r);
            if self.heap.len() > self.k {
                self.heap.pop();
            }
        }
    }

    /// 保存的行，排在前面的在前。
    fn into_rows(self) -> Vec<T> {
        self.heap
            .into_sorted_vec()
            .into_iter()
            .map(|r| r.row)
            .collect()
    }
}

/// 微信自己的消息全文索引 `message/message_fts.db`。它的 FTS5 表用微信的分词器（`mmftstokenizer`），
/// 所以只用 `LIKE` 读取这些表背后普通的 `_content` 表。
struct FtsIndex {
    path: PathBuf,
    /// 每张 `_content` 表，以及其中依次存放（文字、`local_id`、`sort_seq`、会话、`create_time`）的列名（`c0`…）。
    tables: Vec<(String, [String; 5])>,
    /// 行号就是索引中 `session_id` 的那张表。
    users: String,
}

/// `create table` 语句中的列名，或者 `using fts5(…)` 参数中的列名，按顺序。
/// `tokenize = '…'` 这样的选项和表的约束不算。
fn declared_columns(sql: &str) -> Vec<String> {
    let (Some(open), Some(close)) = (sql.find('('), sql.rfind(')')) else {
        return Vec::new();
    };
    sql[open + 1..close]
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.contains('=') {
                return None;
            }
            let name = part
                .split_whitespace()
                .next()?
                .trim_matches(|c| matches!(c, '"' | '`' | '[' | ']' | '\''))
                .to_ascii_lowercase();
            let constraint = ["primary", "unique", "check", "foreign", "constraint"];
            (!constraint.contains(&name.as_str())).then_some(name)
        })
        .collect()
}

/// 微信的索引为一条消息保存的文字（就消息本身能推断的而言）：文字消息的文字，去掉别人在群里发的消息开头的
/// `sender:\n`；app 消息（链接、文件、引用等）的 `<title>`。
fn indexed_text(local_type: i64, content: &str, group: bool) -> String {
    if local_type != 1 {
        return title_of(content).unwrap_or_default();
    }
    match content.split_once(":\n") {
        Some((sender, text))
            if group && !sender.is_empty() && !sender.contains(char::is_whitespace) =>
        {
            text.to_string()
        }
        _ => content.to_string(),
    }
}

/// app 消息 XML 中第一个 `<title>` 的文字，已解码实体。用普通的字符串查找：搜索读到的每条 app 消息都要运行它。
fn title_of(xml: &str) -> Option<String> {
    let start = xml.find("<title>")? + "<title>".len();
    let raw = &xml[start..start + xml[start..].find("</title>")?];
    let raw = raw
        .strip_prefix("<![CDATA[")
        .and_then(|r| r.strip_suffix("]]>"))
        .unwrap_or(raw);
    if !raw.contains('&') {
        return Some(raw.to_string());
    }
    Some(
        raw.replace("&lt;", "<")
            .replace("&gt;", ">")
            .replace("&quot;", "\"")
            .replace("&apos;", "'")
            .replace("&amp;", "&"),
    )
}

/// [`Account::search_messages`] 的一个结果在哪里：扫描到的写成所在的表和 `local_id`，
/// 来自微信索引的写成会话、`local_id` 和 `sort_seq`。
fn place(r: &Value) -> String {
    match r.get("_session").and_then(Value::as_str) {
        Some(sid) => format!("{sid}\n{}\n{}", int(r, "local_id"), int(r, "sort_seq")),
        None => format!("{}\n{}", int(r, "_table"), int(r, "local_id")),
    }
}

/// 一个结果可能所在的表：扫描到的是它自己的表，来自索引的是它所在会话的各张表。
fn tables_of(r: &Value, tables: &[(String, crate::msg::MsgTable)]) -> Vec<usize> {
    match r.get("_session").and_then(Value::as_str) {
        Some(sid) => (0..tables.len())
            .filter(|&at| tables[at].0 == sid)
            .collect(),
        None => usize::try_from(int(r, "_table")).into_iter().collect(),
    }
}

/// 搜索在转向微信的索引之前，从最新的消息开始读一个会话的多少：每张消息表的行数，以及总共的时间。
/// 常见的词远在此前就能凑满一页；哪里都找不到的词最多也只多花这么多。不论预算是多少，结果都相同，只是快慢不同。
const NEWEST_FIRST_ROWS: usize = 2000;
#[cfg_attr(test, expect(dead_code, reason = "测试自己设定预算，不限时间"))]
const NEWEST_FIRST_TIME: std::time::Duration = std::time::Duration::from_millis(15);

/// 从最新的消息开始读（[`NEWEST_FIRST_ROWS`]）时，停止读取的行数和时刻。
#[derive(Clone, Copy)]
struct Budget {
    rows: usize,
    until: Option<std::time::Instant>,
}

impl Budget {
    const NONE: Budget = Budget {
        rows: usize::MAX,
        until: None,
    };

    fn newest_first() -> Self {
        #[cfg(test)]
        return Budget {
            rows: TEST_NEWEST_FIRST_ROWS.with(std::cell::Cell::get),
            until: None,
        };
        #[cfg(not(test))]
        Budget {
            rows: NEWEST_FIRST_ROWS,
            until: Some(std::time::Instant::now() + NEWEST_FIRST_TIME),
        }
    }

    fn spent(&self, rows: usize) -> bool {
        rows >= self.rows || self.until.is_some_and(|u| std::time::Instant::now() >= u)
    }
}

#[cfg(test)]
thread_local! {
    /// 这个线程上运行的测试所用的 [`NEWEST_FIRST_ROWS`]（不限时间，每次运行的结果才一致）。
    static TEST_NEWEST_FIRST_ROWS: std::cell::Cell<usize> = const { std::cell::Cell::new(NEWEST_FIRST_ROWS) };
}

/// 文字消息和任意子类型的 app 消息（`(kind << 32) | 49`）：微信的索引收录的就是这些。每个比较各带一个 `+`，
/// 让 SQLite 用 `sort_seq` 索引而不是类型索引。
const SEARCHABLE_SQL: &str = "(+m.local_type = 1 or (+m.local_type & 4294967295) = 49)";

impl Account {
    /// 这个账号的微信索引；没有，或者结构不是这里认识的样子时为 `None`。
    fn fts_index(&self) -> Option<FtsIndex> {
        let path = self.db_path("message/message_fts.db");
        if !path.is_file() {
            return None;
        }
        let master = self
            .query(
                &path,
                "select name, coalesce(sql, '') as sql from sqlite_master where type = 'table'",
                &[],
            )
            .ok()?;
        let names: std::collections::HashSet<String> =
            master.iter().map(|r| text(r, "name")).collect();
        let (mut tables, mut users) = (Vec::new(), Vec::new());
        for r in &master {
            let (name, sql) = (text(r, "name"), text(r, "sql"));
            let columns = declared_columns(&sql);
            let virtual_table = sql.to_ascii_lowercase().starts_with("create virtual table");
            if virtual_table && name.starts_with("message_fts") {
                let at = |c: &str| columns.iter().position(|x| x == c).map(|i| format!("c{i}"));
                let content = format!("{name}_content");
                if let (Some(t), Some(id), Some(seq), Some(s), Some(time), true) = (
                    at("acontent"),
                    at("message_local_id"),
                    at("sort_seq"),
                    at("session_id"),
                    at("create_time"),
                    names.contains(&content),
                ) {
                    tables.push((content, [t, id, seq, s, time]));
                }
            } else if !virtual_table && columns == ["username"] {
                users.push(name);
            }
        }
        (!tables.is_empty() && users.len() == 1).then(|| FtsIndex {
            path,
            tables,
            users: users.remove(0),
        })
    }

    /// 在 `ix` 中查找 `sessions` 里的结果（给出 `below` 时只取 `sort_seq` 不大于它且不为 0 的），以
    /// `{sort_seq, create_time, local_id, _session}` 加入 `hits`；返回索引为它认识的每个会话保存的最新 `sort_seq`。
    fn search_index(
        &self,
        ix: &FtsIndex,
        sessions: &[String],
        like: &str,
        (begin, end): (i64, i64),
        below: Option<i64>,
        hits: &mut Newest,
    ) -> Result<HashMap<String, i64>> {
        let wanted: std::collections::HashSet<&str> = sessions.iter().map(String::as_str).collect();
        let mut ids = HashMap::new();
        for r in self.query(
            &ix.path,
            &format!("select rowid as id, username from \"{}\"", ix.users),
            &[],
        )? {
            let name = text(&r, "username");
            if wanted.contains(name.as_str()) {
                ids.insert(int(&r, "id"), name);
            }
        }
        let mut newest = HashMap::new();
        if ids.is_empty() {
            return Ok(newest);
        }
        let list = ids.keys().map(i64::to_string).collect::<Vec<_>>().join(",");
        let mut tops = HashMap::<i64, i64>::new();
        for (table, [t, id, seq, s, time]) in &ix.tables {
            let mut range = String::new();
            if begin > 0 {
                range.push_str(&format!(" and {time} >= {begin}"));
            }
            if end > 0 {
                range.push_str(&format!(" and {time} <= {end}"));
            }
            if let Some(below) = below {
                range.push_str(&format!(" and {seq} <= {below} and {seq} <> 0"));
            }
            // 把这些会话的行读一遍，既得出每个会话最新的 sort_seq（索引收录到哪里），也找出结果。
            // `like` 里是转成小写的关键词，LIKE 只忽略 ASCII 字母的大小写：它能匹配的，扫描时转成小写的比较也能匹配。
            // 行大致按从新到旧的顺序出来（微信加入它们的顺序），所以保存满一页之后，大部分结果不必构造成行就跳过。
            let sql = format!(
                "select {seq}, {time}, {id}, {s}, ({t} like ?1 escape '\\'{range}) from \"{table}\" \
                 where {s} in ({list}) order by rowid desc"
            );
            let mut found = Newest::new(hits.k);
            let mut table_tops = HashMap::<i64, i64>::new();
            self.with_db(&ix.path, |conn| {
                found = Newest::new(hits.k);
                table_tops.clear();
                let mut stmt = conn.prepare(&sql)?;
                let mut rows = stmt.query([like])?;
                while let Some(r) = rows.next()? {
                    let [seq, time, local_id, s, hit] = [0, 1, 2, 3, 4]
                        .map(|i| r.get::<_, Option<i64>>(i).ok().flatten().unwrap_or(0));
                    let top = table_tops.entry(s).or_insert(seq);
                    *top = (*top).max(seq);
                    if hit == 0 {
                        continue;
                    }
                    let key = (if seq == 0 { time * 1000 } else { seq }, time, local_id);
                    // 排名不高于已保存的最后一行、又在它之后找到的行，进不了前 `k` 行
                    if found.last_kept().is_some_and(|last| key <= last) {
                        continue;
                    }
                    if let Some(name) = ids.get(&s) {
                        found.push(json!({
                            "sort_seq": seq,
                            "create_time": time,
                            "local_id": local_id,
                            "_session": name,
                        }));
                    }
                }
                Ok(())
            })?;
            hits.absorb(found);
            for (s, top) in table_tops {
                let all = tops.entry(s).or_insert(top);
                *all = (*all).max(top);
            }
        }
        for (s, top) in tops {
            if let Some(name) = ids.get(&s) {
                newest.insert(name.clone(), top);
            }
        }
        Ok(newest)
    }

    /// 文字中含有 `keyword`（不区分大小写）的消息，最新的在前，从第 `offset` 条起取 `limit` 条；
    /// `limit` 不大于 0 时取 50 条，给出 `session_id` 时只在这个会话里搜索。
    ///
    /// 匹配的文字与微信自己的搜索相同：索引（`message_fts.db`）收录了的消息从索引读取；
    /// 索引还没收录的消息（没有索引时是全部消息）按从消息推断出的同样的文字匹配（`indexed_text`）。
    ///
    /// # Errors
    ///
    /// 读取会话列表或某个消息库失败（打开、解密或查询失败）。微信的索引读不了时不算错误，改为扫描全部消息。
    pub fn search_messages(
        &self,
        keyword: &str,
        session_id: Option<&str>,
        limit: i32,
        offset: i32,
        begin: i64,
        end: i64,
    ) -> Result<Vec<SessionRow>> {
        let needle = keyword.trim().to_lowercase();
        if needle.is_empty() {
            return Ok(Vec::new());
        }
        // 搜索要读会话列表、每个消息库和索引。在这里一起打开，它们耗时的密钥派生就同时进行，
        // 而不是搜索读到哪个才派生哪个、一个接一个（#1100）；已经打开的不再花时间。
        let mut dbs = vec![self.session_db(), self.db_path("message/message_fts.db")];
        dbs.extend(self.message_dbs());
        dbs.retain(|p| p.is_file());
        self.prefetch(&dbs);
        let sessions: Vec<String> = match session_id.map(str::trim).filter(|s| !s.is_empty()) {
            Some(s) => vec![s.to_string()],
            None => crate::msg::session_usernames(self)?,
        };
        let like = format!(
            "%{}%",
            needle
                .replace('\\', "\\\\")
                .replace('%', "\\%")
                .replace('_', "\\_")
        );
        let (offset, limit) = (
            usize::try_from(offset).unwrap_or(0),
            usize::try_from(limit)
                .ok()
                .filter(|&limit| limit > 0)
                .unwrap_or(50),
        );
        let want = offset + limit;
        let mut tables = Vec::new();
        for (sid, ts) in sessions.iter().zip(self.message_tables_of(&sessions)?) {
            for t in ts {
                tables.push((sid.clone(), t));
            }
        }
        let index = self.fts_index();
        // 已删除但索引里还有的消息不列出：前 `k` 个结果里有这样的消息时，多排一些结果
        let mut k = want;
        let page: Vec<Value> = loop {
            let ranked = self.search_ranked(
                index.as_ref(),
                &tables,
                &sessions,
                &needle,
                &like,
                (begin, end),
                k,
            )?;
            let gone = self.deleted_hits(&ranked, &tables)?;
            let kept: Vec<Value> = ranked
                .iter()
                .filter(|r| !gone.contains(&place(r)))
                .cloned()
                .collect();
            if gone.is_empty() || kept.len() >= want || ranked.len() < k {
                break kept.into_iter().skip(offset).take(limit).collect();
            }
            k += want - kept.len();
        };

        // 然后读这一页的整行：扫描到的按表读，来自索引的按会话读（索引用 local_id 和 sort_seq 标识消息，
        // 不知道它在哪个数据库）
        let mut wanted: std::collections::BTreeMap<usize, Vec<String>> = Default::default();
        for r in &page {
            for at in tables_of(r, &tables) {
                wanted
                    .entry(at)
                    .or_default()
                    .push(int(r, "local_id").to_string());
            }
        }
        // 每行只读一次，按结果可能给出的两种位置都能找到它
        let mut rows: Vec<Option<SessionRow>> = Vec::new();
        let mut whole = HashMap::new();
        for (at, ids) in wanted {
            let (sid, t) = &tables[at];
            let sql = Self::select_sql(t, &format!("where m.local_id in ({})", ids.join(",")));
            for row in self.read_rows(t, &sql, &[])? {
                let (id, seq) = row.index_key();
                whole.insert(format!("{sid}\n{id}\n{seq}"), rows.len());
                whole.insert(format!("{at}\n{id}"), rows.len());
                rows.push(Some(SessionRow {
                    session_id: sid.clone(),
                    row,
                }));
            }
        }
        Ok(page
            .iter()
            .filter_map(|r| rows.get_mut(*whole.get(&place(r))?)?.take())
            .collect())
    }

    /// `ranked` 中来自索引、但消息已不在所在会话里的结果。
    fn deleted_hits(
        &self,
        ranked: &[Value],
        tables: &[(String, crate::msg::MsgTable)],
    ) -> Result<std::collections::HashSet<String>> {
        let mut wanted: std::collections::BTreeMap<usize, Vec<String>> = Default::default();
        let mut gone = std::collections::HashSet::new();
        for r in ranked.iter().filter(|r| r.get("_session").is_some()) {
            gone.insert(place(r));
            for at in tables_of(r, tables) {
                wanted
                    .entry(at)
                    .or_default()
                    .push(int(r, "local_id").to_string());
            }
        }
        for (at, ids) in wanted {
            let (sid, t) = &tables[at];
            let sql = format!(
                "select local_id, sort_seq from \"{}\" where local_id in ({})",
                t.table,
                ids.join(",")
            );
            for r in self.query(&t.db, &sql, &[])? {
                gone.remove(&format!(
                    "{sid}\n{}\n{}",
                    int(&r, "local_id"),
                    int(&r, "sort_seq")
                ));
            }
        }
        Ok(gone)
    }

    /// 最新在前的前 `want` 个结果。来自索引的结果是 `{sort_seq, create_time, local_id, _session}`，
    /// 扫描到的结果给出所在的表（`_table`）。
    ///
    /// 只搜一个会话时，先从最新的消息开始读，与没有索引时的搜索一样；有索引时只读到 [`Budget`] 限定的行数和时间为止，
    /// 常见的词不读索引就能凑满一页。更早的消息从索引读取，索引还没收录的消息靠扫描。
    #[expect(clippy::too_many_arguments, reason = "一次搜索的各项参数，原样往下传")]
    fn search_ranked(
        &self,
        index: Option<&FtsIndex>,
        tables: &[(String, crate::msg::MsgTable)],
        sessions: &[String],
        needle: &str,
        like: &str,
        (begin, end): (i64, i64),
        want: usize,
    ) -> Result<Vec<Value>> {
        let mut hits = Newest::new(want);
        // line 为 Some 时，sort_seq 大于它的消息（以及没有 sort_seq 的消息）都已读过
        let mut line = None;
        if index.is_none() || sessions.len() == 1 {
            let budget = if index.is_some() {
                Budget::newest_first()
            } else {
                Budget::NONE
            };
            let mut cut = None;
            for (at, (sid, t)) in tables.iter().enumerate() {
                let (found, stopped) = self.scan_table(
                    at,
                    sid,
                    t,
                    needle,
                    like,
                    (begin, end),
                    "",
                    want,
                    budget,
                    true,
                )?;
                hits.absorb(found);
                cut = cut.max(stopped);
            }
            let Some(l) = cut else {
                return Ok(hits.into_rows());
            };
            // line 以上的结果已经齐全；line 以下的结果重新查找
            let rows = hits.into_rows();
            let above = |r: &Value| int(r, "sort_seq") == 0 || int(r, "sort_seq") > l;
            if rows.iter().filter(|r| above(r)).count() >= want {
                return Ok(rows.into_iter().filter(above).collect());
            }
            hits = Newest::new(want);
            rows.into_iter().filter(above).for_each(|r| hits.push(r));
            line = Some(l);
        }
        // 索引读不了时不用索引，改为扫描全部消息
        let mut covered = HashMap::new();
        if let Some(ix) = index {
            let mut found = Newest::new(want);
            if let Ok(newest) =
                self.search_index(ix, sessions, like, (begin, end), line, &mut found)
            {
                covered = newest;
                hits.absorb(found);
            }
        }
        for (at, (sid, t)) in tables.iter().enumerate() {
            // 索引已收录的、上面已经读过的部分不再读
            let mut cond = covered
                .get(sid)
                .map_or(String::new(), |top| format!(" and m.sort_seq > {top}"));
            if let Some(l) = line {
                cond.push_str(&format!(" and m.sort_seq <= {l}"));
            }
            let (found, _) = self.scan_table(
                at,
                sid,
                t,
                needle,
                like,
                (begin, end),
                &cond,
                want,
                Budget::NONE,
                line.is_none(),
            )?;
            hits.absorb(found);
        }
        Ok(hits.into_rows())
    }

    /// 一张消息表在 `cond` 条件下按 [`indexed_text`] 匹配到的结果：先读有 `sort_seq` 的行，最新的在前，
    /// 最多读到 `budget` 为止；`unsequenced` 为真时再读少数没有 `sort_seq` 的行。预算让读取停下时，
    /// 还返回第一个没读的行的 `sort_seq`：比它大的行都读过了，不大于它的行没有全部读过。
    #[expect(
        clippy::too_many_arguments,
        reason = "一次搜索的各项参数，以及它在这张表里读到哪里"
    )]
    fn scan_table(
        &self,
        at: usize,
        sid: &str,
        t: &crate::msg::MsgTable,
        needle: &str,
        like: &str,
        (begin, end): (i64, i64),
        cond: &str,
        want: usize,
        budget: Budget,
        unsequenced: bool,
    ) -> Result<(Newest, Option<i64>)> {
        let group = sid.ends_with("@chatroom");
        let range = range_sql(begin, end);
        // 只读排序键和文字来匹配。纯文字先在 SQL 里筛选，app 消息的 XML 也一样（标题原样写在里面，
        // 除非关键词含有 XML 会写成实体的字符），blob（zstd）只能解码之后再筛选
        let xml_escaped = if needle.contains(['&', '<', '>', '"', '\'']) {
            " or +m.local_type <> 1"
        } else {
            ""
        };
        let light = format!(
            "select m.sort_seq, m.create_time, m.local_id, m.local_type, m.message_content from \"{table}\" m \
             where {SEARCHABLE_SQL}{range}{cond} and (typeof(m.message_content) = 'blob'{xml_escaped} \
             or m.message_content like ?1 escape '\\')",
            table = t.table
        );
        // 有 sort_seq 的行经它的索引从新到旧读取：保存满一页之后，sort_seq 更小的行进不了这一页，它后面的行也一样；
        // 少数没有 sort_seq 的行按 create_time * 1000 排序，另外读取
        let mut table_hits = Newest::new(want);
        let mut stopped = None;
        let mut tails = vec![(" and m.sort_seq <> 0 order by m.sort_seq desc", true)];
        if unsequenced {
            tails.push((" and (m.sort_seq is null or m.sort_seq = 0)", false));
        }
        for (tail, newest_first) in tails {
            let found = std::cell::RefCell::new(Newest::new(want));
            // 已读的行数，以及因预算没读的第一行的 sort_seq
            let read = std::cell::Cell::new((0usize, None));
            self.query_until(
                &t.db,
                &format!("{light}{tail}"),
                &[&like],
                || {
                    *found.borrow_mut() = Newest::new(want);
                    read.set((0, None));
                },
                |mut r| {
                    if newest_first {
                        if found
                            .borrow()
                            .last_kept()
                            .is_some_and(|last| int(&r, "sort_seq") < last.0)
                        {
                            return false;
                        }
                        let (n, _) = read.get();
                        if budget.spent(n) {
                            read.set((n, Some(int(&r, "sort_seq"))));
                            return false;
                        }
                        read.set((n + 1, None));
                    }
                    let kind = int(&r, "local_type");
                    let matched = indexed_text(kind, &text(&r, "message_content"), group)
                        .to_lowercase()
                        .contains(needle);
                    if matched {
                        r["message_content"] = Value::Null;
                        r["_table"] = json!(at);
                        found.borrow_mut().push(r);
                    }
                    true
                },
            )?;
            if newest_first {
                stopped = read.get().1;
            }
            table_hits.absorb(found.into_inner());
        }
        Ok((table_hits, stopped))
    }

    /// 跨会话的图片和视频消息，最新的在前；返回这一页，以及后面是否还有。
    ///
    /// `media_type` 为 1 时只取图片，为 2 时只取视频，其他值两者都取。每个会话只给出这些类型最新的
    /// `offset + limit + 1` 行，经 `(local_type, sort_seq)` 索引读取（[`typed_before`](Self::typed_before)），
    /// 所以一页的代价取决于这一页之前的行数，而不是账号里全部的图片和视频。排序键相同的行按 `session_ids`
    /// 的顺序，再按分片的顺序排列。`limit <= 0` 表示取 `offset` 之后的全部行。
    ///
    /// # Errors
    ///
    /// 某个消息库无法读取（打开、解密或查询失败）。
    pub fn scan_media_stream(
        &self,
        session_ids: &[String],
        media_type: i32,
        begin: i64,
        end: i64,
        limit: i32,
        offset: i32,
    ) -> Result<(Vec<SessionRow>, bool)> {
        let kinds: &[i64] = match media_type {
            1 => &[IMAGE],
            2 => &[VIDEO],
            _ => &[IMAGE, VIDEO],
        };
        let offset = usize::try_from(offset).unwrap_or(0);
        let page_end = usize::try_from(limit)
            .ok()
            .filter(|&l| l > 0)
            .map(|l| offset.saturating_add(l));
        // 每个会话多取一行，就知道后面是否还有；-1 表示全部读取
        let want = page_end.map_or(-1, |e| {
            i64::try_from(e.saturating_add(1)).unwrap_or(i64::MAX)
        });
        let mut rows: Newest<SessionRow> = Newest::new(page_end.unwrap_or(usize::MAX));
        for (sid, tables) in session_ids.iter().zip(self.message_tables_of(session_ids)?) {
            for row in self.typed_before(&tables, kinds, None, (begin, end), 0, want)? {
                rows.push_keyed(
                    row.key(),
                    SessionRow {
                        session_id: sid.clone(),
                        row,
                    },
                );
            }
        }
        let more = page_end.is_some_and(|e| rows.found > e);
        let page: Vec<SessionRow> = rows.into_rows().into_iter().skip(offset).collect();
        Ok((page, more))
    }

    // ── 语音 ──

    /// `message/media_<n>.db` 的各个分片，按编号升序。
    pub fn media_dbs(&self) -> Vec<PathBuf> {
        let mut found: Vec<(u32, PathBuf)> = std::fs::read_dir(self.db_path("message"))
            .into_iter()
            .flatten()
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().to_ascii_lowercase();
                let n = name
                    .strip_prefix("media_")?
                    .strip_suffix(".db")?
                    .parse()
                    .ok()?;
                Some((n, e.path()))
            })
            .collect();
        found.sort();
        found.into_iter().map(|(_, p)| p).collect()
    }

    /// 语音数据的小写十六进制（有多个片段时按 `data_index` 的顺序拼接）；没有时为 `""`。
    ///
    /// 匹配顺序：先按服务器 ID 精确匹配，再按 `local_id` 加上相差不超过 5 秒的 `create_time` 匹配。
    /// `candidates` 是这一行可能属于的聊天的用户名（发送者、会话、自己）；列表为空时接受任何聊天。
    ///
    /// # Errors
    ///
    /// 某个 `media_<n>.db` 无法读取（打开、解密或查询失败）。
    pub fn voice_data(
        &self,
        create_time: i64,
        local_id: i64,
        svr_id: i64,
        candidates: &[String],
    ) -> Result<String> {
        for db in self.media_dbs() {
            let chat_filter = if candidates.is_empty() {
                String::new()
            } else {
                let sql = format!(
                    "select rowid as id from Name2Id where user_name in ({})",
                    vec!["?"; candidates.len()].join(",")
                );
                let params: Vec<&dyn ToSql> = candidates.iter().map(|c| c as &dyn ToSql).collect();
                let ids: Vec<String> = self
                    .query(&db, &sql, &params)?
                    .iter()
                    .map(|r| int(r, "id").to_string())
                    .collect();
                if ids.is_empty() {
                    continue;
                }
                format!(" and chat_name_id in ({})", ids.join(","))
            };
            let order = " order by cast(data_index as integer), data_index";
            let tiers: [(bool, String, Vec<i64>); 2] = [
                (svr_id > 0, format!("svr_id = ?1{chat_filter}{order}"), vec![svr_id]),
                (
                    local_id > 0,
                    format!("local_id = ?1 and abs(create_time - ?2) <= {VOICE_TIME_WINDOW}{chat_filter}{order}"),
                    vec![local_id, create_time],
                ),
            ];
            for (enabled, cond, args) in tiers {
                if !enabled {
                    continue;
                }
                let params: Vec<&dyn ToSql> = args.iter().map(|a| a as &dyn ToSql).collect();
                let rows = self.query(
                    &db,
                    &format!("select voice_data from VoiceInfo where {cond}"),
                    &params,
                )?;
                let hex: String = rows.iter().map(|r| text(r, "voice_data")).collect();
                if !hex.is_empty() {
                    return Ok(hex);
                }
            }
        }
        Ok(String::new())
    }

    /// `[{ "hex": ... }]`：每个请求一项（没有找到语音时为 `{}`）。
    ///
    /// # Errors
    ///
    /// 同 [`voice_data`](Self::voice_data)。
    pub fn voice_data_batch(&self, requests: &Value) -> Result<Value> {
        let mut out = Vec::new();
        for req in requests.as_array().map(Vec::as_slice).unwrap_or(&[]) {
            let svr = match &req["svr_id"] {
                Value::Number(n) => n.as_i64().unwrap_or(0),
                Value::String(s) => s.trim().parse().unwrap_or(0),
                _ => 0,
            };
            let candidates: Vec<String> = req["candidates"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.as_str().map(str::to_string))
                        .collect()
                })
                .unwrap_or_default();
            let hex = self.voice_data(
                int(req, "create_time"),
                int(req, "local_id"),
                svr,
                &candidates,
            )?;
            out.push(if hex.is_empty() {
                json!({})
            } else {
                json!({ "hex": hex })
            });
        }
        Ok(Value::Array(out))
    }

    // ── 微信的媒体索引 `hardlink/hardlink.db` ──

    /// 微信媒体索引列出的全部图片文件，按消息的 md5（小写）分组，每个 md5 下最新的在前，路径相对于账号目录：
    /// `msg/attach/<dir1>/<dir2>/Img/<file name>`。同一张图片的几种版本（缩略图、普通图、原图）共用它的 md5；
    /// 类型为 4 的行不对应文件。没有这个索引的账号列出的是空表。
    ///
    /// 一次读完整张表：微信没有给 md5 建索引，查一个 md5 也要读完整张表。
    ///
    /// # Errors
    ///
    /// 无法读取 `hardlink.db`（打开、解密或查询失败）。
    pub fn image_hardlink_index(&self) -> Result<HashMap<String, Vec<PathBuf>>> {
        let db = self.db_path("hardlink/hardlink.db");
        if !db.is_file() {
            return Ok(HashMap::new());
        }
        let sql = "select h.md5, h.file_name, a.username as dir1, b.username as dir2 from image_hardlink_info_v4 h \
                   left join dir2id a on a.rowid = h.dir1 left join dir2id b on b.rowid = h.dir2 \
                   where h.type <> 4 order by h.modify_time desc";
        let mut index: HashMap<String, Vec<PathBuf>> = HashMap::new();
        for row in self.query(&db, sql, &[])? {
            let parts = [
                text(&row, "dir1"),
                text(&row, "dir2"),
                text(&row, "file_name"),
            ];
            let md5 = text(&row, "md5").trim().to_ascii_lowercase();
            if md5.is_empty() || !parts.iter().all(|p| is_plain_name(p)) {
                continue;
            }
            let [dir1, dir2, name] = &parts;
            let path: PathBuf = ["msg", "attach", dir1, dir2, "Img", name].iter().collect();
            index.entry(md5).or_default().push(path);
        }
        Ok(index)
    }

    /// 微信媒体索引为一个消息 md5 列出的视频文件，最新的在前，路径相对于账号目录：`msg/video/<dir1>/<file name>`。
    /// 文件名可以与 md5 不同。
    ///
    /// # Errors
    ///
    /// 无法读取 `hardlink.db`（打开、解密或查询失败）。
    pub fn video_hardlinks(&self, md5: &str) -> Result<Vec<PathBuf>> {
        let db = self.db_path("hardlink/hardlink.db");
        let md5 = md5.trim().to_ascii_lowercase();
        if md5.is_empty() || !db.is_file() {
            return Ok(Vec::new());
        }
        let sql = "select h.file_name, a.username as dir1 from video_hardlink_info_v4 h \
                   left join dir2id a on a.rowid = h.dir1 \
                   where h.md5 = ?1 order by h.modify_time desc";
        let rows = self.query(&db, sql, &[&md5])?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let (dir1, name) = (text(row, "dir1"), text(row, "file_name"));
                (is_plain_name(&dir1) && is_plain_name(&name))
                    .then(|| ["msg", "video", &dir1, &name].iter().collect())
            })
            .collect())
    }

    /// 微信媒体索引为一个消息 md5 列出的文件（收发的文档、压缩包等），最新的在前，路径相对于账号目录：
    /// `msg/file/<dir1>/<file name>`，`dir1` 是月份（`2024-01`）。
    ///
    /// # Errors
    ///
    /// 无法读取 `hardlink.db`（打开、解密或查询失败）。
    pub fn file_hardlinks(&self, md5: &str) -> Result<Vec<PathBuf>> {
        let db = self.db_path("hardlink/hardlink.db");
        let md5 = md5.trim().to_ascii_lowercase();
        if md5.is_empty() || !db.is_file() {
            return Ok(Vec::new());
        }
        let sql = "select h.file_name, a.username as dir1 from file_hardlink_info_v4 h \
                   left join dir2id a on a.rowid = h.dir1 \
                   where h.md5 = ?1 order by h.modify_time desc";
        let rows = self.query(&db, sql, &[&md5])?;
        Ok(rows
            .iter()
            .filter_map(|row| {
                let (dir1, name) = (text(row, "dir1"), text(row, "file_name"));
                (is_plain_name(&dir1) && is_plain_name(&name))
                    .then(|| ["msg", "file", &dir1, &name].iter().collect())
            })
            .collect())
    }

    // ── 表情 ──

    fn emoticon_db(&self) -> PathBuf {
        self.db_path("emoticon/emoticon.db")
    }

    /// 按内容的 md5（不区分大小写）查表情的下载地址：依次取 CDN 地址、外部地址、预览图、缩略图。
    ///
    /// # Errors
    ///
    /// 无法读取 `emoticon.db`（打开、解密或查询失败）。
    pub fn emoticon_cdn_url(&self, md5: &str) -> Result<String> {
        let rows = self.query(&self.emoticon_db(), "select cdn_url, extern_url, tp_url, thumb_url from kNonStoreEmoticonTable where lower(md5) = lower(?1)", &[&md5.trim()])?;
        Ok(rows
            .iter()
            .flat_map(|r| {
                ["cdn_url", "extern_url", "tp_url", "thumb_url"]
                    .into_iter()
                    .map(|k| text(r, k))
            })
            .find(|u| !u.is_empty())
            .unwrap_or_default())
    }

    /// 全部表情的说明文字，以小写的 md5 为键：先取用户自己表情的文字，没有时取商店表情包的说明（中文优先）。
    /// 扫描两张表：逐个表情按 `lower(md5) = lower(?)` 查找用不上索引。
    ///
    /// # Errors
    ///
    /// 无法读取 `emoticon.db`（打开、解密或查询失败）。
    pub fn emoticon_captions(&self) -> Result<HashMap<String, String>> {
        let mut out = HashMap::new();
        let own = self.query(
            &self.emoticon_db(),
            "select md5, caption from kNonStoreEmoticonTable where caption <> ''",
            &[],
        )?;
        for r in &own {
            out.entry(text(r, "md5").to_lowercase())
                .or_insert_with(|| text(r, "caption"));
        }
        let store = self.query(&self.emoticon_db(), "select md5_, caption_ from kStoreEmoticonCaptionsTable where caption_ <> '' order by (language_ like 'zh%') desc", &[])?;
        for r in &store {
            out.entry(text(r, "md5_").to_lowercase())
                .or_insert_with(|| text(r, "caption_"));
        }
        Ok(out)
    }
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    reason = "测试用的值已知放得下，或者本来就要回绕"
)]
mod tests {
    use super::*;
    use crate::fixture::{Fixture, TestDir, DAY, T0};

    fn account(tag: &str) -> (TestDir, Account) {
        let root = TestDir::new(&format!("weflow-media-{tag}"));
        let f = Fixture::standard(&root);
        let acct = Account::new(f.db_storage(), &f.key_hex())
            .unwrap()
            .with_my_wxid(Some("wxid_me_ab12".into()));
        (root, acct)
    }

    fn contents(hits: &[SessionRow]) -> Vec<String> {
        hits.iter()
            .map(|h| h.row.content.trim().to_string())
            .collect()
    }

    #[test]
    fn newest_keeps_what_a_stable_newest_first_sort_puts_first() {
        // 键的取值范围小，相同的键（以及按找到的先后排序）才常见；用固定种子的线性同余生成器（LCG）让结果可以重现
        let mut x: u64 = 42;
        let mut next = |m: u64| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            i64::try_from((x >> 33) % m).unwrap()
        };
        for round in 0..200 {
            let rows: Vec<Value> = (0..usize::try_from(next(60)).unwrap())
                .map(|i| json!({ "sort_seq": next(4) * 1000, "create_time": next(3), "local_id": next(3), "i": i }))
                .collect();
            let mut sorted = rows.clone();
            sorted.sort_by_key(|a| std::cmp::Reverse(sort_key(a)));
            for k in [0usize, 1, 3, 10, 100] {
                let mut top = Newest::new(k);
                rows.iter().cloned().for_each(|r| top.push(r));
                assert_eq!(top.found, rows.len());
                assert_eq!(
                    top.into_rows(),
                    sorted.iter().take(k).cloned().collect::<Vec<_>>(),
                    "round {round} k {k}"
                );
                // 同样的行逐表找到，每张表先单独排名
                let mut merged = Newest::new(k);
                for chunk in rows.chunks(7) {
                    let mut local = Newest::new(k);
                    chunk.iter().cloned().for_each(|r| local.push(r));
                    merged.absorb(local);
                }
                assert_eq!(merged.found, rows.len());
                assert_eq!(
                    merged.into_rows(),
                    sorted.iter().take(k).cloned().collect::<Vec<_>>(),
                    "merged, round {round} k {k}"
                );
            }
        }
    }

    #[test]
    fn search_finds_plain_and_compressed_text_across_sessions() {
        let (_dir, a) = account("search");
        // `later, compressed` 是 zstd 压缩的 blob：解码之后才能找到
        assert_eq!(
            contents(
                &a.search_messages("COMPRESSED", Some("wxid_bob"), 50, 0, 0, 0)
                    .unwrap()
            ),
            ["later, compressed"]
        );
        assert_eq!(
            contents(&a.search_messages("hello", None, 50, 0, 0, 0).unwrap()),
            ["hello"]
        );
        let ok = a.search_messages("ok", None, 50, 0, 0, 0).unwrap();
        assert_eq!(
            contents(&ok),
            ["wxid_quiet:\nok"],
            "group text keeps its sender prefix in the raw row"
        );
        assert_eq!(ok[0].session_id, "room1@chatroom");
        assert_eq!(ok[0].row.sender.as_deref(), Some("wxid_quiet"));
        // 但搜索的文字里不含这个前缀
        assert!(a
            .search_messages("quiet", None, 50, 0, 0, 0)
            .unwrap()
            .is_empty());
        // 图片消息（类型 3）的 XML 从不参与搜索
        assert!(a
            .search_messages("aabbccdd", None, 50, 0, 0, 0)
            .unwrap()
            .is_empty());
        assert!(a
            .search_messages("  ", None, 50, 0, 0, 0)
            .unwrap()
            .is_empty());
        // 关键词里 LIKE 的通配符按字面匹配
        assert!(a
            .search_messages("%", None, 50, 0, 0, 0)
            .unwrap()
            .is_empty());
        assert!(a
            .search_messages("h_llo", None, 50, 0, 0, 0)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_orders_pages_and_filters_by_time() {
        let (_dir, a) = account("page");
        let all = a
            .search_messages("e", Some("wxid_bob"), 50, 0, 0, 0)
            .unwrap(); // 含 e 的消息：`hello`、`later, compressed`、`see you`
        assert_eq!(
            contents(&all),
            ["see you", "later, compressed", "hello"],
            "newest first"
        );
        assert_eq!(
            contents(
                &a.search_messages("e", Some("wxid_bob"), 1, 1, 0, 0)
                    .unwrap()
            ),
            ["later, compressed"]
        );
        assert_eq!(
            contents(
                &a.search_messages("e", Some("wxid_bob"), 50, 0, T0 + 3 * DAY, 0)
                    .unwrap()
            ),
            ["see you", "later, compressed"]
        );
        assert!(a
            .search_messages("e", Some("wxid_bob"), 50, 0, 0, T0 - 1)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn search_pages_are_those_of_a_sort_of_every_hit() {
        use crate::fixture::{Fixture, MsgSpec, SessionSpec};
        let root = TestDir::new("weflow-search-ties");
        let f = Fixture::new(&root, "wxid_me_ab12");
        f.session_db(&[SessionSpec {
            username: "wxid_bob",
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        // 两个数据库；库内和库间都有相同的 sort_seq，还有没有 sort_seq 的行、压缩的文字、app 消息（只按标题匹配），
        // 以及从不参与搜索的类型
        let mut all = Vec::new();
        for shard in 0..2u32 {
            let msgs: Vec<MsgSpec> = (1..=60i64)
                .map(|i| {
                    let fruit = ["apple", "Apple pie", "banana"][(i % 3) as usize];
                    let mut m = MsgSpec::text(
                        i,
                        "wxid_bob",
                        T0 + (i * 7) % 40,
                        &format!("msg {shard}-{i} {fruit}"),
                    )
                    .with_sort_seq(if i % 9 == 0 {
                        0
                    } else {
                        (i % 15) * 1000 + 5
                    });
                    if i % 4 == 0 {
                        m = m.compressed();
                    }
                    if i % 5 == 0 {
                        // 链接消息，它的描述不能被匹配到
                        m = m.of_type((5 << 32) | 49);
                        m.content = format!(
                            "<msg><appmsg><title>{}</title><des>apple pie banana</des></appmsg></msg>",
                            m.content
                        );
                    }
                    if i % 11 == 0 {
                        m = m.of_type(3);
                    }
                    m
                })
                .collect();
            all.extend(msgs.iter().map(|m| (shard, m.clone())));
            f.message_shard(shard, &[("wxid_bob", msgs)]);
        }
        let a = Account::new(f.db_storage(), &f.key_hex()).unwrap();
        let shown = |m: &MsgSpec| match m.local_type {
            1 => Some(m.content.clone()),
            k if k & 0xffff_ffff == 49 => m
                .content
                .split_once("<title>")
                .and_then(|(_, t)| t.split_once("</title>"))
                .map(|(t, _)| t.to_string()),
            _ => None,
        };
        let expected = |keyword: &str, limit: usize, offset: usize, begin: i64, end: i64| {
            let mut hits: Vec<_> = all
                .iter()
                .filter(|(_, m)| {
                    shown(m).is_some_and(|t| t.to_lowercase().contains(keyword))
                        && (begin == 0 || m.create_time >= begin)
                        && (end == 0 || m.create_time <= end)
                })
                .map(|(shard, m)| {
                    let seq = match m.sort_seq.unwrap() {
                        0 => m.create_time * 1000,
                        s => s,
                    };
                    ((seq, m.create_time, m.local_id), *shard)
                })
                .collect();
            // 最新的在前；相同的键按数据库的顺序
            hits.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            hits.into_iter()
                .skip(offset)
                .take(limit)
                .map(|((_, _, id), shard)| (format!("message_{shard}"), id))
                .collect::<Vec<_>>()
        };
        for keyword in ["apple", "banana", "pie", "msg", "zzz"] {
            // 在每个位置结束的页，有的结束在一串相同的 sort_seq 中间
            let pages = (1..=14).map(|limit| (limit, 0)).chain([
                (50, 0),
                (5, 5),
                (7, 3),
                (4, 9),
                (1, 30),
                (200, 0),
                (10, 100),
            ]);
            for (limit, offset) in pages {
                for (begin, end) in [(0, 0), (T0 + 10, T0 + 30)] {
                    let got: Vec<(String, i64)> = a
                        .search_messages(
                            &keyword.to_uppercase(),
                            Some("wxid_bob"),
                            limit as i32,
                            offset as i32,
                            begin,
                            end,
                        )
                        .unwrap()
                        .iter()
                        .map(|h| {
                            assert_eq!(h.session_id, "wxid_bob");
                            (format!("message_{}", h.row.shard), h.row.local_id)
                        })
                        .collect();
                    assert_eq!(
                        got,
                        expected(keyword, limit, offset, begin, end),
                        "{keyword} limit {limit} offset {offset} from {begin} to {end}"
                    );
                }
            }
        }
        // 结果带着整行，文字已解码
        let first = &a.search_messages("pie", None, 1, 0, 0, 0).unwrap()[0].row;
        assert!(first.content.ends_with("Apple pie"));
        assert_eq!(first.sender.as_deref(), Some("wxid_bob"));
    }

    #[test]
    fn search_reads_wechats_index_and_scans_what_it_does_not_hold() {
        use crate::fixture::{Fixture, FtsSpec, MsgSpec, SessionSpec};
        let root = TestDir::new("weflow-search-fts");
        let f = Fixture::new(&root, "wxid_me_ab12");
        let room = "room9@chatroom";
        f.session_db(&[room, "wxid_bob"].map(|username| SessionSpec {
            username,
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }));
        // 索引收录了群里 sort_seq 不超过这个值的消息，没有更新的，也没有 bob 的
        const INDEXED_UP_TO: i64 = 40_000;
        let mut all = Vec::new();
        let mut groups = vec![Vec::new(), Vec::new()];
        for shard in 0..2u32 {
            let msgs: Vec<MsgSpec> = (1..=25i64)
                .map(|i| {
                    let fruit = ["apple", "Apple pie", "banana"][(i % 3) as usize];
                    // 两个数据库的 sort_seq 增长快慢不同，库间有少数相同的 sort_seq
                    let seq = if shard == 0 { i * 2000 } else { i * 3000 + 1000 };
                    let said = format!("msg {shard}-{i} {fruit}");
                    let mut m =
                        MsgSpec::text(i, "wxid_alice", T0 + seq / 1000, &format!("wxid_alice:\n{said}"))
                            .with_sort_seq(seq);
                    if i % 4 == 0 {
                        m = m.compressed();
                    }
                    if i % 5 == 0 {
                        // 引用消息：标题是它的文字，被引用的消息不算
                        m = m.of_type((57 << 32) | 49);
                        m.content = format!(
                            "wxid_alice:\n<msg><appmsg><title>{said}</title><refermsg><content>banana pie</content>\
                             </refermsg></appmsg></msg>"
                        );
                    }
                    m
                })
                .collect();
            for m in msgs.iter().filter(|m| m.sort_seq.unwrap() <= INDEXED_UP_TO) {
                let said = m.content.split_once(":\n").unwrap().1;
                // 微信保存去掉发送者后的文字；app 消息保存标题再加一点别的
                let text = match m.local_type {
                    1 => said.to_string(),
                    _ => format!("{} indexed", indexed_text(m.local_type, said, false)),
                };
                // 索引的分组与数据库的编号不对应
                groups[1 - shard as usize].push(FtsSpec {
                    text,
                    local_id: m.local_id,
                    sort_seq: m.sort_seq.unwrap(),
                    local_type: m.local_type,
                    session: room,
                    create_time: m.create_time,
                });
            }
            all.extend(msgs.iter().map(|m| (room, shard, m.clone())));
            let mut sessions = vec![(room, msgs)];
            if shard == 0 {
                let bob: Vec<MsgSpec> = (100..110i64)
                    .map(|i| {
                        MsgSpec::text(i, "wxid_bob", T0 + i, &format!("bob {i} apple"))
                            .with_sort_seq((i - 100) * 4000 + 500)
                    })
                    .collect();
                all.extend(bob.iter().map(|m| ("wxid_bob", 0, m.clone())));
                sessions.push(("wxid_bob", bob));
            }
            f.message_shard(shard, &sessions);
        }
        // 被索引之后又删除的消息
        groups[0].push(FtsSpec {
            text: "deleted apple".into(),
            local_id: 77,
            sort_seq: 39_500,
            local_type: 1,
            session: room,
            create_time: T0 + 39,
        });
        f.message_fts_db(&groups);
        let a = Account::new(f.db_storage(), &f.key_hex()).unwrap();

        let shown = |session: &str, m: &MsgSpec| -> String {
            let said = m
                .content
                .split_once(":\n")
                .map_or(m.content.as_str(), |(_, s)| s);
            let fallback = indexed_text(m.local_type, &m.content, session == room);
            match (
                session == room && m.sort_seq.unwrap() <= INDEXED_UP_TO,
                m.local_type,
            ) {
                (true, 1) => said.to_string(),
                (true, _) => format!("{fallback} indexed"),
                (false, _) => fallback,
            }
        };
        let expected = |keyword: &str,
                        session: Option<&str>,
                        limit: usize,
                        offset: usize,
                        begin: i64,
                        end: i64| {
            let mut hits: Vec<_> = all
                .iter()
                .filter(|(s, _, m)| {
                    session.is_none_or(|x| x == *s)
                        && shown(s, m).to_lowercase().contains(keyword)
                        && (begin == 0 || m.create_time >= begin)
                        && (end == 0 || m.create_time <= end)
                })
                .map(|(s, shard, m)| {
                    (
                        m.sort_seq.unwrap(),
                        s.to_string(),
                        format!("message_{shard}"),
                        m.local_id,
                    )
                })
                .collect();
            // 最新的在前：先按 sort_seq，再按 local_id（这里的时间随 sort_seq 增长）
            hits.sort_by_key(|h| std::cmp::Reverse((h.0, h.3)));
            hits.into_iter()
                .skip(offset)
                .take(limit)
                .map(|(_, s, db, id)| (s, db, id))
                .collect::<Vec<_>>()
        };
        // 一个会话先从最新的消息开始读，每张表读到预算的行数为止，然后读索引：预算为 0 时全部由索引给出；
        // 不限预算时搜索一个会话从不读索引。从消息读取时 app 消息只按标题匹配，
        // 所以索引文字中标题之后的词（`indexed`）在一个会话里只能在读取停下的位置以下找到。
        for budget in (0..=12).chain([usize::MAX]) {
            TEST_NEWEST_FIRST_ROWS.with(|b| b.set(budget));
            for keyword in ["apple", "pie", "indexed", "msg", "alice", "banana", "zzz"] {
                for session in [Some(room), None] {
                    if keyword == "indexed" && session.is_some() && budget != 0 {
                        if budget == usize::MAX {
                            assert!(a
                                .search_messages(keyword, session, 50, 0, 0, 0)
                                .unwrap()
                                .is_empty());
                        }
                        continue;
                    }
                    for (limit, offset) in
                        (1..=8)
                            .map(|l| (l, 0))
                            .chain([(5, 5), (3, 10), (50, 0), (10, 40)])
                    {
                        for (begin, end) in [(0, 0), (T0 + 10, T0 + 45)] {
                            let got: Vec<_> = a
                                .search_messages(
                                    &keyword.to_uppercase(),
                                    session,
                                    limit as i32,
                                    offset as i32,
                                    begin,
                                    end,
                                )
                                .unwrap()
                                .iter()
                                .map(|h| {
                                    (
                                        h.session_id.clone(),
                                        format!("message_{}", h.row.shard),
                                        h.row.local_id,
                                    )
                                })
                                .collect();
                            assert_eq!(
                            got,
                            expected(keyword, session, limit, offset, begin, end),
                            "{keyword} in {session:?}, limit {limit} offset {offset}, from {begin} to {end}, \
                             budget {budget}"
                        );
                        }
                    }
                }
            }
        }
        // 与扫描得到的一样是整行
        TEST_NEWEST_FIRST_ROWS.with(|b| b.set(0));
        let first = &a
            .search_messages("indexed", Some(room), 1, 0, 0, 0)
            .unwrap()[0]
            .row;
        assert_eq!(first.sender.as_deref(), Some("wxid_alice"));
        assert!(first.content.contains("<refermsg>"));
    }

    #[test]
    fn media_stream_pages_images_and_videos() {
        let (_dir, a) = account("media");
        let ids = vec!["wxid_bob".to_string(), "room1@chatroom".to_string()];
        let (rows, more) = a.scan_media_stream(&ids, 1, 0, 0, 10, 0).unwrap();
        assert_eq!(rows.len(), 1, "one image in the fixture");
        assert_eq!(rows[0].session_id, "wxid_bob");
        assert_eq!(rows[0].row.local_type, 3);
        assert!(!more);
        assert!(
            a.scan_media_stream(&ids, 2, 0, 0, 10, 0)
                .unwrap()
                .0
                .is_empty(),
            "no videos"
        );
        let (both, _) = a.scan_media_stream(&ids, 0, 0, 0, 10, 0).unwrap();
        assert_eq!(both, rows);
        let (page, more) = a.scan_media_stream(&ids, 1, 0, 0, 1, 1).unwrap();
        assert!(page.is_empty() && !more);
    }

    /// `scan_media_stream` 的每一页，都是把全部符合的行稳定地按最新在前排序（会话按请求的顺序，再按分片）后的一段；
    /// 其中有跨会话和跨分片的相同键、没有 `sort_seq` 的行、混入的其他类型，以及时间范围。
    #[test]
    fn media_stream_pages_are_slices_of_every_row_sorted() {
        use crate::fixture::{Fixture, MsgSpec};
        let root = TestDir::new("weflow-media-stream-pages");
        let f = Fixture::new(&root, "wxid_me_ab12");
        // 用固定种子的线性同余生成器（LCG）让结果可以重现；取值范围小，相同的键才常见
        let mut x: u64 = 7;
        let mut next = |m: u64| {
            x = x
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            i64::try_from((x >> 33) % m).expect("small")
        };
        let sessions = ["wxid_bob", "room1@chatroom", "wxid_carol"];
        for shard in 0..2u32 {
            let per_session: Vec<(&str, Vec<MsgSpec>)> = sessions
                .iter()
                .map(|&sid| {
                    let msgs = (1..=40)
                        .map(|local_id| {
                            let create_time = T0 + next(6) * 60;
                            let local_type =
                                [IMAGE, VIDEO, 1, 34][usize::try_from(next(4)).expect("small")];
                            let spec = MsgSpec::text(
                                local_id,
                                "wxid_bob",
                                create_time,
                                &format!("{sid}/{shard}/{local_id}"),
                            )
                            .of_type(local_type);
                            // 少数行没有 sort_seq，其余的落在粗的网格上，让不同表的键相同
                            match next(5) {
                                0 => spec.with_sort_seq(0),
                                _ => spec.with_sort_seq(create_time * 1000 + next(3)),
                            }
                        })
                        .collect();
                    (sid, msgs)
                })
                .collect();
            f.message_shard(shard, &per_session);
        }
        let a = Account::new(f.db_storage(), &f.key_hex()).unwrap();
        let ids: Vec<String> = sessions.iter().map(|s| (*s).to_string()).collect();

        for (media_type, kinds) in [(1, "3"), (2, "43"), (0, "3,43")] {
            for (begin, end) in [(0, 0), (T0 + 60, T0 + 240)] {
                let mut every: Vec<SessionRow> = Vec::new();
                for (sid, tables) in ids.iter().zip(a.message_tables_of(&ids).unwrap()) {
                    for t in &tables {
                        let sql = Account::select_sql(
                            t,
                            &format!("where m.local_type in ({kinds}){}", range_sql(begin, end)),
                        );
                        every.extend(a.read_rows(t, &sql, &[]).unwrap().into_iter().map(|row| {
                            SessionRow {
                                session_id: sid.clone(),
                                row,
                            }
                        }));
                    }
                }
                every.sort_by_key(|p| std::cmp::Reverse(p.row.key()));
                assert!(
                    every.len() > 20,
                    "enough rows to page through: {}",
                    every.len()
                );
                for (limit, offset) in [
                    (1, 0),
                    (5, 0),
                    (5, 3),
                    (7, 14),
                    (50, 0),
                    (3, every.len() - 2),
                    (4, every.len() + 1),
                    (0, 0),
                    (0, 9),
                ] {
                    let (page, more) = a
                        .scan_media_stream(
                            &ids,
                            media_type,
                            begin,
                            end,
                            i32::try_from(limit).unwrap(),
                            i32::try_from(offset).unwrap(),
                        )
                        .unwrap();
                    let want: Vec<&SessionRow> = every
                        .iter()
                        .skip(offset)
                        .take(if limit == 0 { usize::MAX } else { limit })
                        .collect();
                    let got: Vec<&SessionRow> = page.iter().collect();
                    assert_eq!(
                        got, want,
                        "type {media_type}, range ({begin}, {end}), limit {limit}, offset {offset}"
                    );
                    assert_eq!(
                        more,
                        limit > 0 && every.len() > offset + limit,
                        "more: type {media_type}, limit {limit}, offset {offset}"
                    );
                }
            }
        }
    }

    #[test]
    fn voice_is_found_by_server_id_or_by_local_id_and_time_window() {
        let (_dir, a) = account("voice");
        let bob = vec!["wxid_bob".to_string()];
        assert_eq!(
            a.voice_data(T0 + 500, 6, 9_000_000_000_001, &bob).unwrap(),
            "01020304",
            "exact server id"
        );
        assert_eq!(
            a.voice_data(T0 + 500, 6, 0, &bob).unwrap(),
            "01020304",
            "local id + time"
        );
        assert_eq!(
            a.voice_data(T0 + 503, 6, 0, &bob).unwrap(),
            "01020304",
            "within the +-5 s window"
        );
        assert_eq!(
            a.voice_data(T0 + 520, 6, 0, &bob).unwrap(),
            "",
            "outside the window"
        );
        assert_eq!(
            a.voice_data(T0 + 600, 7, 9_000_000_000_002, &bob).unwrap(),
            "aabbcc",
            "fragments are joined in data_index order"
        );
        // 候选列表决定在谁的聊天里查找
        assert_eq!(
            a.voice_data(T0 + 700, 8, 9_000_000_000_003, &bob).unwrap(),
            ""
        );
        assert_eq!(
            a.voice_data(T0 + 700, 8, 9_000_000_000_003, &["wxid_me".to_string()])
                .unwrap(),
            "09"
        );
        assert_eq!(
            a.voice_data(T0 + 700, 8, 9_000_000_000_003, &[]).unwrap(),
            "09",
            "no candidates: any chat"
        );
        assert_eq!(
            a.voice_data(T0, 0, 0, &[]).unwrap(),
            "",
            "no key at all matches nothing"
        );
    }

    #[test]
    fn voice_batch_mixes_hits_and_misses() {
        let (_dir, a) = account("vbatch");
        let req = json!([
            {"session_id": "wxid_bob", "create_time": T0 + 500, "local_id": 6, "svr_id": 9_000_000_000_001i64, "candidates": ["wxid_bob"]},
            {"session_id": "wxid_bob", "create_time": T0 + 600, "local_id": 7, "svr_id": "9000000000002", "candidates": ["wxid_bob", "wxid_me"]},
            {"session_id": "wxid_bob", "create_time": 1, "local_id": 99, "svr_id": 5, "candidates": []}
        ]);
        assert_eq!(
            a.voice_data_batch(&req).unwrap(),
            json!([{"hex": "01020304"}, {"hex": "aabbcc"}, {}])
        );
        assert_eq!(a.voice_data_batch(&json!("nonsense")).unwrap(), json!([]));
    }

    #[test]
    fn emoticon_urls_and_captions() {
        let (_dir, a) = account("emoji");
        assert_eq!(
            a.emoticon_cdn_url("aabbccddeeff00112233445566778899")
                .unwrap(),
            "http://cdn/e1",
            "md5 is case-insensitive, CDN first"
        );
        assert_eq!(
            a.emoticon_cdn_url("11223344556677889900AABBCCDDEEFF")
                .unwrap(),
            "http://ext/e2",
            "falls back to the external url"
        );
        assert_eq!(
            a.emoticon_cdn_url("ffffffffffffffffffffffffffffffff")
                .unwrap(),
            ""
        );
        let all = a.emoticon_captions().unwrap();
        assert_eq!(
            all.get("aabbccddeeff00112233445566778899")
                .map(String::as_str),
            Some("smile"),
            "keyed by the lower-case md5"
        );
        assert_eq!(
            all.get("deadbeefdeadbeefdeadbeefdeadbeef")
                .map(String::as_str),
            Some("商店表情"),
            "store captions prefer Chinese"
        );
        assert_eq!(all.get("ffffffffffffffffffffffffffffffff"), None);
    }

    #[test]
    fn the_media_index_names_the_files_of_an_md5_newest_first() {
        use crate::fixture::HardlinkSpec;
        use std::path::Path;
        let root = TestDir::new("weflow-media-hardlink");
        let f = Fixture::standard(&root);
        let a = Account::new(f.db_storage(), &f.key_hex()).unwrap();
        // 没有索引：结果为空，也不报错
        assert!(a.image_hardlink_index().unwrap().is_empty());
        assert!(a
            .video_hardlinks("11223344556677889900aabbccddeeff")
            .unwrap()
            .is_empty());

        let (image, video, talker) = (
            "aabbccddeeff00112233445566778899",
            "11223344556677889900aabbccddeeff",
            "d41d8cd98f00b204e9800998ecf8427e",
        );
        f.hardlink_db(&[
            HardlinkSpec::image(
                image,
                "0123456789abcdef0123456789abcdef_t.dat",
                talker,
                "2023-11",
            ),
            HardlinkSpec {
                modify_time: T0 + DAY,
                ..HardlinkSpec::image(
                    image,
                    "0123456789abcdef0123456789abcdef_h.dat",
                    talker,
                    "2023-12",
                )
            },
            // 类型为 4 的行不对应文件
            HardlinkSpec {
                kind: 4,
                ..HardlinkSpec::image(image, "0", talker, "2023-11")
            },
            // 会跑出所在文件夹的文件名不采用
            HardlinkSpec::image(image, "../escape.dat", talker, "2023-11"),
            // 以大写保存的 md5 用小写也能找到
            HardlinkSpec::image(
                "FFEEDDCCBBAA99887766554433221100",
                "ffeeddccbbaa99887766554433221100.dat",
                talker,
                "2023-11",
            ),
            HardlinkSpec::video(video, "ffeeddccbbaa99887766554433221100.mp4", "2023-11"),
        ]);
        let image_path = |month: &str, name: &str| {
            Path::new("msg/attach")
                .join(talker)
                .join(month)
                .join("Img")
                .join(name)
        };
        let index = a.image_hardlink_index().unwrap();
        assert_eq!(
            index[image],
            [
                image_path("2023-12", "0123456789abcdef0123456789abcdef_h.dat"),
                image_path("2023-11", "0123456789abcdef0123456789abcdef_t.dat"),
            ]
        );
        assert_eq!(
            index["ffeeddccbbaa99887766554433221100"],
            [image_path(
                "2023-11",
                "ffeeddccbbaa99887766554433221100.dat"
            )]
        );
        // 每种媒体在各自的表里
        assert_eq!(index.len(), 2);
        assert_eq!(
            a.video_hardlinks(&video.to_uppercase()).unwrap(),
            [Path::new(
                "msg/video/2023-11/ffeeddccbbaa99887766554433221100.mp4"
            )]
        );
        assert!(a.video_hardlinks(image).unwrap().is_empty());
        assert!(a.video_hardlinks(" ").unwrap().is_empty());
    }
}
