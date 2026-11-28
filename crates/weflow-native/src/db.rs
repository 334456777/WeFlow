//! 取代闭源 `wcdb_api` 库的原生实现（纯 Rust 加内置的 SQLite）。
//!
//! 一个账号是一个 `db_storage` 目录加上原始的数据库密钥。每个数据库文件作为按需解密页的只读 SQLite 快照打开（见
//! [`crate::cipher_vfs`]），文件或它的 `-wal` 在磁盘上变化时重新打开，所以长时间运行的命令（`serve`、消息推送）
//! 能持续看到新消息。明文不写入磁盘。

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use anyhow::{anyhow, Context, Result};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::cipher_vfs::{self, CipherFile};
use crate::sqlcipher;

/// 每个连接的 SQLite 页缓存（KiB）：保留查询运行期间会再次读取的页，如 B 树的内部页。无论缓存多大，
/// 遍历整张表的读取每页都只解密一次。
const PAGE_CACHE_KIB: i64 = 16 * 1024;
/// 所有快照共用的 SQLite 堆软上限：超过后页缓存回收最旧的页。
const HEAP_SOFT_LIMIT: i64 = 1024 * 1024 * 1024;
/// 同时保持打开的快照数（最近最少使用的先关闭）。
const MAX_SNAPSHOTS: usize = 32;
/// 微信在查询期间不断改动文件时，一次查询最多运行的次数，含第一次。
const STALE_RETRIES: usize = 4;

/// 快照的一个连接，以及它的页缓存大小是否已经设置。
struct Conn {
    conn: Connection,
    sized: bool,
}

struct Snapshot {
    /// `file` 上的连接；只有一个，除非账号句柄要求更多（见 [`Account::set_read_connections`]）。
    conns: Vec<Arc<Mutex<Conn>>>,
    file: Arc<CipherFile>,
    key: [u8; 32],
    last_used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<PathBuf, Snapshot>,
    tick: u64,
}

/// 两个 ID 是否属于同一个账号（忽略首尾空白和大小写）：相等，或者其中一个是另一个加上 `_` 和 4 个字符的目录后缀。
pub fn same_identity(a: &str, b: &str) -> bool {
    let (a, b) = (a.trim().to_ascii_lowercase(), b.trim().to_ascii_lowercase());
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let suffixed = |long: &str, short: &str| {
        long.len() == short.len() + 5
            && long.starts_with(short)
            && long.as_bytes()[short.len()] == b'_'
    };
    a == b || suffixed(&a, &b) || suffixed(&b, &a)
}

/// 快照用的进程级 SQLite 设置。
fn configure_sqlite() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: 普通的配置调用，任何时候调用都有效。
    ONCE.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_soft_heap_limit64(HEAP_SOFT_LIMIT);
    });
}

/// 进程内所有账号句柄共用解密的快照：服务层每次调用都新建句柄，每次都重新解密大的消息库会占去大部分运行时间。
static SHARED_CACHE: OnceLock<Arc<Mutex<Cache>>> = OnceLock::new();

fn shared_cache() -> Arc<Mutex<Cache>> {
    SHARED_CACHE
        .get_or_init(|| Arc::new(Mutex::new(Cache::default())))
        .clone()
}

/// 用密钥以只读方式打开的一个微信账号的数据库：数据库层的每个查询都是它的方法。
pub struct Account {
    db_storage: PathBuf,
    raw_key: [u8; 32],
    cache: Arc<Mutex<Cache>>,
    /// 账号所有者的身份（wxid，可以带账号目录的后缀 `_xxxx`），用来判断哪些消息是用户自己发的。
    my_wxid: Option<String>,
    pub(crate) cursors: Mutex<crate::msg::cursor::Cursors>,
    /// 这个句柄的查询在一个快照上最多使用的连接数（见 [`Account::set_read_connections`]）。
    read_connections: std::sync::atomic::AtomicUsize,
    /// 因快照过期而重新执行的查询数（见 [`Account::stale_reruns`]）。
    stale_reruns: std::sync::atomic::AtomicUsize,
}

/// 只显示数据库所在的位置，不显示密钥。
impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account")
            .field("db_storage", &self.db_storage)
            .finish_non_exhaustive()
    }
}

/// 存放二进制内容（protobuf 之类）的 BLOB 列：一律输出为小写十六进制。
const HEX_BLOB_COLUMNS: &[&str] = &[
    "packed_info_data",
    "extra_buffer",
    "ext_buffer",
    "reserved0",
    "voice_data",
];

/// 把一个 SQLite 值转成 JSON。`HEX_BLOB_COLUMNS` 中的二进制列输出为十六进制，不解压；
/// 其他 BLOB 是 zstd 帧时先解压（WCDB 压缩了部分文本列），结果是合法的 UTF-8 时输出文本，否则输出十六进制。
/// TEXT 列按 UTF-8 解码，非法字节被替换。
pub(crate) fn value_to_json(column: &str, v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) if HEX_BLOB_COLUMNS.contains(&column) => Value::String(hex(b)),
        ValueRef::Blob(b) => Value::String(blob_to_string(b)),
    }
}

/// `bytes` 的小写十六进制。
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xf) as usize] as char);
    }
    out
}

/// 还原 WCDB 压缩存储的 BLOB：zstd 帧解压，其他内容（或解压失败的帧）原样返回。
pub(crate) fn inflate_blob(b: &[u8]) -> Cow<'_, [u8]> {
    if b.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        if let Ok(v) = zstd::stream::decode_all(b) {
            return Cow::Owned(v);
        }
    }
    Cow::Borrowed(b)
}

/// BLOB 的文本形式：先解压 zstd 帧，是合法的 UTF-8 时为文本，否则为小写十六进制。
fn blob_to_string(b: &[u8]) -> String {
    let bytes = inflate_blob(b);
    match std::str::from_utf8(&bytes) {
        Ok(s) => s.to_string(),
        Err(_) => hex(&bytes),
    }
}

impl Account {
    /// 打开账号：`db_storage` 是 `<account>/db_storage`，
    /// `hex_key` 是 `weflow key db` 输出的 64 个十六进制字符的密钥。
    ///
    /// # Errors
    ///
    /// `db_storage` 不是目录，或者 `hex_key` 不是合法的密钥时返回错误。
    pub fn new(db_storage: impl Into<PathBuf>, hex_key: &str) -> Result<Self> {
        let db_storage = db_storage.into();
        if !db_storage.is_dir() {
            return Err(anyhow!("db_storage not found: {}", db_storage.display()));
        }
        Ok(Self {
            db_storage,
            raw_key: sqlcipher::parse_key(hex_key)?,
            cache: shared_cache(),
            my_wxid: None,
            cursors: Mutex::default(),
            read_connections: std::sync::atomic::AtomicUsize::new(1),
            stale_reruns: Default::default(),
        })
    }

    /// 设置数据库快照为这个句柄的查询最多打开几个连接（至少一个）。一个连接上的查询逐个执行；连接更多时，
    /// 读同一个数据库的多个线程（并行导出）可以同时进行。每个连接各有自己的页缓存，所以内存随连接数增长。
    /// 多出的连接留在快照上，供以后的查询使用。
    pub fn set_read_connections(&self, n: usize) {
        self.read_connections
            .store(n.max(1), std::sync::atomic::Ordering::Relaxed);
    }

    /// 这个句柄的查询中，因读取期间微信改动了文件而在新快照上重新执行的次数（见 [`with_db`](Self::with_db)）。
    pub fn stale_reruns(&self) -> usize {
        self.stale_reruns.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 设置账号所有者的 wxid（见 [`Account::is_me`]）。
    pub fn with_my_wxid(mut self, wxid: Option<String>) -> Self {
        self.my_wxid = wxid.map(|w| w.trim().to_string()).filter(|w| !w.is_empty());
        self
    }

    /// `sender` 是否为账号所有者。消息库中保存的是裸 wxid，而账号目录名是 `<wxid>_<4 chars>`，
    /// 所以裸 wxid 与带后缀的形式互相匹配。
    pub fn is_me(&self, sender: &str) -> bool {
        self.my_wxid
            .as_deref()
            .is_some_and(|me| same_identity(sender, me))
    }

    /// 账号的 `db_storage` 目录。
    pub fn db_storage(&self) -> &Path {
        &self.db_storage
    }

    /// `path` 的快照持有的连接数（没有打开时为 0）。
    #[cfg(test)]
    pub(crate) fn snapshot_connections(&self, path: &Path) -> usize {
        self.cache
            .lock()
            .expect("lock poisoned")
            .entries
            .get(path)
            .map_or(0, |s| s.conns.len())
    }

    /// 相对 `db_storage` 给出的数据库（如 `session/session.db`）的绝对路径。
    pub fn db_path(&self, rel: &str) -> PathBuf {
        self.db_storage.join(rel)
    }

    /// 打开（或刷新）`path` 的快照，在其上运行 `f`。
    ///
    /// `f` 读取期间微信改动了快照下的文件时，`f` 失败，并在新的快照上重新运行；所以 `f` 必须可以重复执行（纯查询，
    /// 或者每次都从头开始输出的查询）。
    ///
    /// # Errors
    ///
    /// 快照无法打开、`f` 返回错误，或者文件在 `STALE_RETRIES` 次运行中一直在变化时返回错误。
    pub fn with_db<T>(
        &self,
        path: &Path,
        mut f: impl FnMut(&Connection) -> Result<T>,
    ) -> Result<T> {
        self.read_db(path, |c| f(c), |_, v| Ok(v))
    }

    /// 分两步的 [`with_db`](Self::with_db)，供自己遍历文件页的读取使用（见 [`crate::leaf_scan`]）：`prepare`
    /// 在快照的连接上运行，然后 `read` 拿 `prepare` 的结果在快照的文件上运行，这期间连接空出来供其他查询使用。
    /// 微信改动了文件时，两步都在新的快照上重新运行。
    pub(crate) fn read_db<P, T>(
        &self,
        path: &Path,
        mut prepare: impl FnMut(&Connection) -> Result<P>,
        mut read: impl FnMut(&CipherFile, P) -> Result<T>,
    ) -> Result<T> {
        let mut attempt = 0;
        loop {
            let (conn, file) = self.snapshot(path)?;
            let result = {
                let mut guard = conn
                    .lock()
                    .map_err(|_| anyhow!("database snapshot lock poisoned"))?;
                // 放在闭包里：这个 pragma 会读 schema，所以与 `f` 一样可能遇到过期的快照。
                let mut run = || {
                    if !guard.sized {
                        guard
                            .conn
                            .pragma_update(None, "cache_size", -PAGE_CACHE_KIB)?;
                        guard.sized = true;
                    }
                    prepare(&guard.conn)
                };
                run()
            };
            let result = result.and_then(|prepared| read(&file, prepared));
            match result {
                Err(e) if file.is_stale() || file.changed_since_open() => {
                    self.forget(path, &file);
                    self.stale_reruns
                        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    attempt += 1;
                    if attempt >= STALE_RETRIES {
                        let why = file.read_error().unwrap_or_else(|| e.to_string());
                        return Err(e.context(format!(
                            "{} kept changing while it was being read: {why}",
                            path.display()
                        )));
                    }
                }
                other => return other,
            }
        }
    }

    /// 缓存中 `path` 的快照仍是 `file` 时把它丢弃。
    fn forget(&self, path: &Path, file: &Arc<CipherFile>) {
        if let Ok(mut cache) = self.cache.lock() {
            if cache
                .entries
                .get(path)
                .is_some_and(|s| Arc::ptr_eq(&s.file, file))
            {
                cache.entries.remove(path);
            }
        }
    }

    /// 同时打开 `paths` 中还没打开的快照：每个新文件都要做一次较慢的密钥派生，
    /// 这样需要每个消息分片的命令只等一次派生，而不是一个接一个地等全部。出错时不报告，留给实际的调用报告。
    pub fn prefetch(&self, paths: &[PathBuf]) {
        let missing: Vec<&PathBuf> = {
            let Ok(cache) = self.cache.lock() else { return };
            paths
                .iter()
                .filter(|p| {
                    !cache
                        .entries
                        .get(p.as_path())
                        .is_some_and(|s| s.key == self.raw_key)
                })
                .collect()
        };
        if missing.len() < 2 {
            return;
        }
        std::thread::scope(|s| {
            for p in missing {
                s.spawn(move || {
                    let _ = self.snapshot(p);
                });
            }
        });
    }

    fn snapshot(&self, path: &Path) -> Result<(Arc<Mutex<Conn>>, Arc<CipherFile>)> {
        let known_cipher = {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| anyhow!("snapshot cache lock poisoned"))?;
            cache.tick += 1;
            let tick = cache.tick;
            match cache.entries.get_mut(path) {
                Some(entry) if entry.key == self.raw_key => {
                    // 通过快照持有的句柄查询：不重新打开文件
                    if entry.file.unchanged() && !entry.file.is_stale() {
                        entry.last_used = tick;
                        let file = entry.file.clone();
                        // 先找没有别人持有的连接；没有就在上限内新开一个；否则用最空闲的
                        if let Some(free) = entry.conns.iter().find(|c| Arc::strong_count(c) == 1) {
                            return Ok((free.clone(), file));
                        }
                        let limit = self
                            .read_connections
                            .load(std::sync::atomic::Ordering::Relaxed);
                        if entry.conns.len() >= limit {
                            if let Some(least) =
                                entry.conns.iter().min_by_key(|c| Arc::strong_count(c))
                            {
                                return Ok((least.clone(), file));
                            }
                        }
                        drop(cache);
                        return self.add_connection(path, file, limit);
                    }
                    Some(entry.file.cipher()) // 同一个文件 => 同一个盐 => 复用较慢的 KDF 的结果
                }
                _ => None,
            }
        };

        // 打开时不持有缓存的锁，这期间其他文件也可以打开。快照打开文件时取文件戳：
        // 这期间发生的变化会让下一次调用重新打开。
        let file = Arc::new(CipherFile::open(path, &self.raw_key, known_cipher)?);
        let conn = cipher_vfs::open_connection(file.clone())?;
        configure_sqlite();
        let conn = Arc::new(Mutex::new(Conn { conn, sized: false })); // 由 `read_db` 设置页缓存大小
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("snapshot cache lock poisoned"))?;
        cache.tick += 1;
        let tick = cache.tick;
        cache.entries.insert(
            path.to_path_buf(),
            Snapshot {
                conns: vec![conn.clone()],
                file: file.clone(),
                key: self.raw_key,
                last_used: tick,
            },
        );
        while cache.entries.len() > MAX_SNAPSHOTS {
            let victim = cache
                .entries
                .iter()
                .filter(|(p, _)| p.as_path() != path)
                .min_by_key(|(_, s)| s.last_used)
                .map(|(p, _)| p.clone());
            match victim {
                Some(p) => cache.entries.remove(&p),
                None => break,
            };
        }
        Ok((conn, file))
    }

    /// 在 `path` 的快照 `file` 上再开一个连接（不用派生密钥：文件已经打开），
    /// 快照的连接少于 `limit` 个时把它留在快照上。
    fn add_connection(
        &self,
        path: &Path,
        file: Arc<CipherFile>,
        limit: usize,
    ) -> Result<(Arc<Mutex<Conn>>, Arc<CipherFile>)> {
        let conn = cipher_vfs::open_connection(file.clone())?;
        let conn = Arc::new(Mutex::new(Conn { conn, sized: false })); // 由 `read_db` 设置页缓存大小
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("snapshot cache lock poisoned"))?;
        // 这期间快照可能已被替换：那样这个连接只为请求它的这次查询服务
        if let Some(entry) = cache
            .entries
            .get_mut(path)
            .filter(|e| Arc::ptr_eq(&e.file, &file) && e.conns.len() < limit)
        {
            entry.conns.push(conn.clone());
        }
        Ok((conn, file))
    }

    /// 在 `path` 上执行只读查询，把每一行作为 JSON 对象返回。
    ///
    /// # Errors
    ///
    /// 快照无法打开、SQL 有误或执行失败时返回错误，重新执行的规则见 [`with_db`](Self::with_db)。
    pub fn query(
        &self,
        path: &Path,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
    ) -> Result<Vec<Value>> {
        let out = std::cell::RefCell::new(Vec::new());
        self.query_each(
            path,
            sql,
            params,
            || out.borrow_mut().clear(),
            |row| out.borrow_mut().push(row),
        )?;
        Ok(out.into_inner())
    }

    /// 与 [`query`](Self::query) 相同，但每读到一行就交给 `each`，不收集起来。
    /// `start` 在第一行之前运行；快照过期、从头重新读取各行时再运行一次（见 [`with_db`](Self::with_db)）：
    /// 它必须重置 `each` 累积的内容。
    ///
    /// # Errors
    ///
    /// 与 [`query`](Self::query) 相同。
    pub fn query_each(
        &self,
        path: &Path,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
        start: impl FnMut(),
        mut each: impl FnMut(Value),
    ) -> Result<()> {
        self.query_until(path, sql, params, start, |row| {
            each(row);
            true
        })
    }

    /// 与 [`query_each`](Self::query_each) 相同，但 `each` 返回 `false` 后停止读取。
    ///
    /// # Errors
    ///
    /// 与 [`query`](Self::query) 相同。
    pub fn query_until(
        &self,
        path: &Path,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
        mut start: impl FnMut(),
        mut each: impl FnMut(Value) -> bool,
    ) -> Result<()> {
        self.with_db(path, |conn| {
            start();
            let mut stmt = conn
                .prepare(sql)
                .with_context(|| format!("bad query: {sql}"))?;
            let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let mut rows = stmt.query(params)?;
            while let Some(row) = rows.next()? {
                let mut obj = Map::with_capacity(names.len());
                for (i, name) in names.iter().enumerate() {
                    obj.insert(name.clone(), value_to_json(name, row.get_ref(i)?));
                }
                if !each(Value::Object(obj)) {
                    break;
                }
            }
            Ok(())
        })
    }

    /// 账号的 `db_storage` 下的 `session/session.db`。
    pub fn session_db(&self) -> PathBuf {
        self.db_path("session/session.db")
    }

    /// 检查账号能否读取：解密 `session.db`，读出它的 schema。
    ///
    /// # Errors
    ///
    /// `session.db` 无法打开或解密（如密钥不对）时返回错误。
    pub fn test_connection(&self) -> Result<()> {
        self.with_db(&self.session_db(), |conn| {
            conn.query_row("select count(*) from sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })?;
            Ok(())
        })
    }

    /// 聊天页面要读的数据库：会话、联系人（含头像的地址）、微信的媒体索引、微信的搜索索引、消息和语音，
    /// 只列出存在的文件。
    pub fn chat_databases(&self) -> Vec<PathBuf> {
        let mut all: Vec<PathBuf> = [
            "session/session.db",
            "contact/contact.db",
            "hardlink/hardlink.db",
            "message/message_fts.db",
        ]
        .iter()
        .map(|rel| self.db_path(rel))
        .collect();
        all.extend(self.message_dbs());
        all.extend(self.media_dbs());
        all.retain(|p| p.is_file());
        all
    }

    /// 同时打开这些数据库，在读取需要之前派生好它们的密钥（见 [`sqlcipher::shared_cipher`]）；返回打开成功的个数。
    pub fn warm(&self, paths: &[PathBuf]) -> usize {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (next, opened) = (AtomicUsize::new(0), AtomicUsize::new(0));
        let workers = std::thread::available_parallelism()
            .map_or(4, |n| n.get())
            .min(paths.len());
        std::thread::scope(|s| {
            for _ in 0..workers {
                s.spawn(|| {
                    while let Some(path) = paths.get(next.fetch_add(1, Ordering::Relaxed)) {
                        let schema = self.with_db(path, |conn| {
                            Ok(
                                conn.query_row("select count(*) from sqlite_master", [], |r| {
                                    r.get::<_, i64>(0)
                                })?,
                            )
                        });
                        if schema.is_ok() {
                            opened.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                });
            }
        });
        opened.into_inner()
    }

    /// `SessionTable` 的各行，最新的在前，列与微信保存的相同。
    ///
    /// # Errors
    ///
    /// `session.db` 无法读取时返回错误。
    pub fn sessions(&self) -> Result<Value> {
        let rows = self.query(
            &self.session_db(),
            "select * from SessionTable order by sort_timestamp desc",
            &[],
        )?;
        Ok(Value::Array(rows))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_is_lowercase_two_digits_per_byte() {
        assert_eq!(super::hex(&[]), "");
        assert_eq!(super::hex(&[0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        let all: Vec<u8> = (0..=255).collect();
        let expected: String = all.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(super::hex(&all), expected);
    }

    #[test]
    fn a_database_written_after_its_snapshot_opened_is_read_anew() {
        use crate::fixture::{Fixture, SessionSpec, TestDir, T0};
        let root = TestDir::new("weflow-snapshot-fresh");
        let f = Fixture::new(&root, "wxid_me_ab12");
        let session = |username: &'static str| SessionSpec {
            username,
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        };
        f.session_db(&[session("wxid_bob")]);
        let acct = super::Account::new(f.db_storage(), &f.key_hex()).unwrap();
        let count = || acct.sessions().unwrap().as_array().map_or(0, Vec::len);
        assert_eq!(count(), 1);
        assert_eq!(
            count(),
            1,
            "the snapshot is kept while the file is as it was"
        );
        // 之后微信在原文件上写入数据库：在文件系统时钟的同一个刻度内两次大小相同的写入，
        // 无论按路径还是按句柄看都一样，所以把时间设到第一次写入之后
        f.session_db(&[session("wxid_bob"), session("wxid_carol")]);
        let db = acct.db_path("session/session.db");
        let later =
            std::fs::metadata(&db).unwrap().modified().unwrap() + std::time::Duration::from_secs(2);
        std::fs::File::options()
            .write(true)
            .open(&db)
            .unwrap()
            .set_modified(later)
            .unwrap();
        assert_eq!(count(), 2);
    }

    #[test]
    fn the_databases_of_a_chat_page_open_at_once() {
        use crate::fixture::{Fixture, SessionSpec, TestDir, T0};
        let root = TestDir::new("weflow-warm");
        let f = Fixture::new(&root, "wxid_me_ab12");
        f.session_db(&[SessionSpec {
            username: "wxid_bob",
            summary: "",
            last_timestamp: T0,
            unread: 0,
            last_msg_type: 1,
        }]);
        for shard in 0..2 {
            f.message_shard(shard, &[]);
        }
        let acct = super::Account::new(f.db_storage(), &f.key_hex()).unwrap();
        let all = acct.chat_databases();
        for rel in [
            "session/session.db",
            "message/message_0.db",
            "message/message_1.db",
        ] {
            assert!(all.contains(&acct.db_path(rel)), "{rel} in {all:?}");
        }
        assert!(all.iter().all(|p| p.is_file()));
        assert_eq!(acct.warm(&all), all.len());
        // 打不开的数据库被跳过
        let missing = acct.db_path("message/message_9.db");
        assert_eq!(acct.warm(&[missing]), 0);
    }
}
