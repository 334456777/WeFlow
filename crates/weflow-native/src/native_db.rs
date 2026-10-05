//! Native (pure Rust + bundled SQLite) replacement for the closed `wcdb_api` library.
//!
//! An account is a `db_storage` directory plus the raw database key. Each database file is opened as a
//! read-only SQLite snapshot that decrypts pages on demand (see [`crate::cipher_vfs`]), and is reopened
//! whenever the file or its `-wal` changes on disk, so long-running commands (`serve`, message push) keep
//! seeing new messages. Nothing plaintext is written to disk.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::cipher_vfs::{self, CipherFile};
use crate::sqlcipher;

/// SQLite page cache per snapshot (KiB). It only holds pages a query has read, so it stays small for small
/// queries; queries that walk a whole message database keep every page decrypted once, as long as the soft limit
/// below allows.
const PAGE_CACHE_KIB: i64 = 1024 * 1024;
/// Page cache while the account handle is in low-memory mode ([`NativeAccount::set_low_memory`]): for long
/// one-way reads such as exports, where keeping a whole message database decrypted saves little time.
const LOW_MEMORY_CACHE_KIB: i64 = 16 * 1024;
/// Soft limit of SQLite's heap across all snapshots: above it page caches recycle their oldest pages.
const HEAP_SOFT_LIMIT: i64 = 1024 * 1024 * 1024;
/// Snapshots kept open (least recently used ones are closed first).
const MAX_SNAPSHOTS: usize = 32;
/// How often a query is run again on a fresh snapshot when WeChat changed the file under it.
const STALE_RETRIES: usize = 4;

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    db_len: u64,
    db_mtime: Option<SystemTime>,
    wal_len: u64,
    wal_mtime: Option<SystemTime>,
}

/// A snapshot's connection and the page cache size it is set to.
struct Conn {
    conn: Connection,
    cache_kib: i64,
}

struct Snapshot {
    /// Connections over `file`; one unless a handle asks for more (see [`NativeAccount::set_read_connections`]).
    conns: Vec<Arc<Mutex<Conn>>>,
    file: Arc<CipherFile>,
    key: [u8; 32],
    stamp: FileStamp,
    last_used: u64,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<PathBuf, Snapshot>,
    tick: u64,
}

/// Whether two ids are the same account: equal, or one is the other plus the `_<4 chars>` directory suffix.
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

/// Process-wide SQLite settings for the snapshots.
fn configure_sqlite() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    // SAFETY: plain configuration call, valid at any time.
    ONCE.call_once(|| unsafe {
        rusqlite::ffi::sqlite3_soft_heap_limit64(HEAP_SOFT_LIMIT);
    });
}

/// Decrypted snapshots are shared by every account handle in the process: the service layer opens a
/// fresh handle per call, and re-decrypting a large message database each time would dominate the run.
static SHARED_CACHE: OnceLock<Arc<Mutex<Cache>>> = OnceLock::new();

fn shared_cache() -> Arc<Mutex<Cache>> {
    SHARED_CACHE
        .get_or_init(|| Arc::new(Mutex::new(Cache::default())))
        .clone()
}

pub struct NativeAccount {
    db_storage: PathBuf,
    raw_key: [u8; 32],
    cache: Arc<Mutex<Cache>>,
    /// Identity of the account owner (wxid, optionally with the `_xxxx` account-directory suffix);
    /// used to tell which messages were sent by the user.
    my_wxid: Option<String>,
    pub(crate) cursors: Mutex<crate::native_msg::Cursors>,
    low_memory: std::sync::atomic::AtomicBool,
    /// Most connections a snapshot gets for this handle's queries (see [`NativeAccount::set_read_connections`]).
    read_connections: std::sync::atomic::AtomicUsize,
}

fn wal_path(db: &Path) -> PathBuf {
    let mut os = db.as_os_str().to_os_string();
    os.push("-wal");
    PathBuf::from(os)
}

fn stamp_of(db: &Path) -> Result<FileStamp> {
    let meta = fs::metadata(db).with_context(|| format!("cannot stat {}", db.display()))?;
    let wal = fs::metadata(wal_path(db)).ok();
    Ok(FileStamp {
        db_len: meta.len(),
        db_mtime: meta.modified().ok(),
        wal_len: wal.as_ref().map(|m| m.len()).unwrap_or(0),
        wal_mtime: wal.and_then(|m| m.modified().ok()),
    })
}

/// BLOB columns that carry binary payloads (protobuf and friends): always exposed as lowercase hex.
const HEX_BLOB_COLUMNS: &[&str] = &[
    "packed_info_data",
    "extra_buffer",
    "ext_buffer",
    "reserved0",
    "voice_data",
];

/// Turn one SQLite value into JSON. zstd blobs (WCDB compresses some text columns) are inflated;
/// other blobs become text when the column is a text column, hex when it is a binary one.
fn value_to_json(column: &str, v: ValueRef<'_>) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => json!(i),
        ValueRef::Real(f) => json!(f),
        ValueRef::Text(t) => Value::String(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(b) if HEX_BLOB_COLUMNS.contains(&column) => Value::String(hex(b)),
        ValueRef::Blob(b) => Value::String(blob_to_string(b)),
    }
}

pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0xf) as usize] as char);
    }
    out
}

/// Text form of a BLOB: inflate zstd frames, then UTF-8 if valid, otherwise lowercase hex.
pub fn blob_to_string(b: &[u8]) -> String {
    let inflated;
    let bytes = if b.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) {
        match zstd::stream::decode_all(b) {
            Ok(v) => {
                inflated = v;
                &inflated[..]
            }
            Err(_) => b,
        }
    } else {
        b
    };
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => hex(bytes),
    }
}

impl NativeAccount {
    /// `db_storage` is `<account>/db_storage`; `hex_key` is the 64-hex key from `weflow key db`.
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
            low_memory: Default::default(),
            read_connections: std::sync::atomic::AtomicUsize::new(1),
        })
    }

    /// Low-memory mode for long one-way reads (exports): the snapshots this handle uses keep a small page cache,
    /// so a pass over a whole message database does not keep all of it decrypted in memory.
    pub fn set_low_memory(&self, on: bool) {
        self.low_memory
            .store(on, std::sync::atomic::Ordering::Relaxed);
    }

    /// How many connections a database snapshot may open for this handle's queries (at least one). Queries on one
    /// connection run one at a time; with more, threads that read the same database (a parallel export) run side by
    /// side. Every connection keeps its own page cache, so memory grows with the count. Extra connections stay with
    /// the snapshot for later queries.
    pub fn set_read_connections(&self, n: usize) {
        self.read_connections
            .store(n.max(1), std::sync::atomic::Ordering::Relaxed);
    }

    /// Set the account owner's wxid (see [`NativeAccount::is_me`]).
    pub fn with_my_wxid(mut self, wxid: Option<String>) -> Self {
        self.my_wxid = wxid.map(|w| w.trim().to_string()).filter(|w| !w.is_empty());
        self
    }

    /// Whether `sender` is the account owner. Message databases store the bare wxid while the account
    /// directory is `<wxid>_<4 chars>`, so a bare id matches its suffixed form and the other way round.
    pub fn is_me(&self, sender: &str) -> bool {
        self.my_wxid
            .as_deref()
            .is_some_and(|me| same_identity(sender, me))
    }

    pub fn db_storage(&self) -> &Path {
        &self.db_storage
    }

    /// Account directory (`db_storage`'s parent): where `msg/`, `cache/` and friends live.
    pub fn account_dir(&self) -> PathBuf {
        self.db_storage
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| self.db_storage.clone())
    }

    /// (number of open snapshots, bytes of decrypted pages they cache, the soft limit in bytes).
    pub fn cache_stats(&self) -> (usize, usize, usize) {
        let Ok(c) = self.cache.lock() else {
            return (0, 0, HEAP_SOFT_LIMIT as usize);
        };
        let mut used = 0usize;
        for conn in c.entries.values().flat_map(|s| &s.conns) {
            if let Ok(c) = conn.lock() {
                let (mut cur, mut hi) = (0, 0);
                // SAFETY: a valid connection handle and two out-parameters.
                unsafe {
                    rusqlite::ffi::sqlite3_db_status(
                        c.conn.handle(),
                        rusqlite::ffi::SQLITE_DBSTATUS_CACHE_USED,
                        &mut cur,
                        &mut hi,
                        0,
                    )
                };
                used += cur.max(0) as usize;
            }
        }
        (c.entries.len(), used, HEAP_SOFT_LIMIT as usize)
    }

    /// Connections the snapshot of `path` holds (0 when it is not open).
    #[cfg(test)]
    pub(crate) fn snapshot_connections(&self, path: &Path) -> usize {
        self.cache
            .lock()
            .unwrap()
            .entries
            .get(path)
            .map_or(0, |s| s.conns.len())
    }

    /// Absolute path of a database given relative to `db_storage` (`session/session.db`).
    pub fn db_path(&self, rel: &str) -> PathBuf {
        self.db_storage.join(rel)
    }

    /// Open (or refresh) the snapshot of `path` and run `f` on it.
    ///
    /// When WeChat changes the file under the snapshot while `f` reads it, `f` fails, and it is run again on a fresh
    /// snapshot; so `f` must be safe to repeat (a pure query, or one that starts its output over).
    pub fn with_db<T>(
        &self,
        path: &Path,
        mut f: impl FnMut(&Connection) -> Result<T>,
    ) -> Result<T> {
        let mut attempt = 0;
        let cache_kib = if self.low_memory.load(std::sync::atomic::Ordering::Relaxed) {
            LOW_MEMORY_CACHE_KIB
        } else {
            PAGE_CACHE_KIB
        };
        loop {
            let (conn, file) = self.snapshot(path)?;
            let result = {
                let mut guard = conn
                    .lock()
                    .map_err(|_| anyhow!("database snapshot lock poisoned"))?;
                if guard.cache_kib != cache_kib {
                    guard.conn.pragma_update(None, "cache_size", -cache_kib)?;
                    guard.cache_kib = cache_kib;
                }
                f(&guard.conn)
            };
            match result {
                Err(e) if file.is_stale() => {
                    self.forget(path, &file);
                    attempt += 1;
                    if attempt >= STALE_RETRIES {
                        let why = file.read_error().unwrap_or_else(|| e.to_string());
                        return Err(e.context(format!(
                            "{} kept changing while it was being read ({why})",
                            path.display()
                        )));
                    }
                }
                other => return other,
            }
        }
    }

    /// Drop the cached snapshot of `path` if it is still `file`.
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

    /// Open the snapshots of `paths` that are not open yet, several at once: each new file costs a slow key
    /// derivation, so a command that needs every message shard waits for one instead of all of them in a row.
    /// Errors are left for the real call to report.
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
        let stamp = stamp_of(path)?;
        let known_cipher = {
            let mut cache = self
                .cache
                .lock()
                .map_err(|_| anyhow!("snapshot cache lock poisoned"))?;
            cache.tick += 1;
            let tick = cache.tick;
            match cache.entries.get_mut(path) {
                Some(entry) if entry.key == self.raw_key => {
                    if entry.stamp == stamp && !entry.file.is_stale() {
                        entry.last_used = tick;
                        let file = entry.file.clone();
                        // a connection nobody else holds; else a new one while the limit allows; else the least busy
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
                    Some(entry.file.cipher()) // same file => same salt => reuse the slow KDF result
                }
                _ => None,
            }
        };

        // Opened without holding the cache lock, so other files can be opened meanwhile. `stamp` was taken
        // before opening: a change that lands meanwhile makes the next call reopen.
        let file = Arc::new(CipherFile::open(path, &self.raw_key, known_cipher)?);
        let conn = cipher_vfs::open_connection(file.clone())?;
        configure_sqlite();
        let conn = Arc::new(Mutex::new(Conn { conn, cache_kib: 0 })); // sized by `with_db`
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
                stamp,
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

    /// One more connection over the snapshot `file` of `path` (no key derivation: the file is already open), kept
    /// with the snapshot while it has fewer than `limit`.
    fn add_connection(
        &self,
        path: &Path,
        file: Arc<CipherFile>,
        limit: usize,
    ) -> Result<(Arc<Mutex<Conn>>, Arc<CipherFile>)> {
        let conn = cipher_vfs::open_connection(file.clone())?;
        let conn = Arc::new(Mutex::new(Conn { conn, cache_kib: 0 })); // sized by `with_db`
        let mut cache = self
            .cache
            .lock()
            .map_err(|_| anyhow!("snapshot cache lock poisoned"))?;
        // the snapshot may have been replaced meanwhile: then this connection only serves the query that asked
        if let Some(entry) = cache
            .entries
            .get_mut(path)
            .filter(|e| Arc::ptr_eq(&e.file, &file) && e.conns.len() < limit)
        {
            entry.conns.push(conn.clone());
        }
        Ok((conn, file))
    }

    /// Run a read-only query on `path` and return every row as a JSON object.
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

    /// Like [`query`](Self::query), but hands each row to `each` as it is read instead of collecting them.
    /// `start` runs before the first row, and again if the snapshot went stale and the rows are read once more
    /// from the beginning (see [`with_db`](Self::with_db)): it must reset whatever `each` accumulated.
    pub fn query_each(
        &self,
        path: &Path,
        sql: &str,
        params: &[&dyn rusqlite::ToSql],
        mut start: impl FnMut(),
        mut each: impl FnMut(Value),
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
                each(Value::Object(obj));
            }
            Ok(())
        })
    }

    pub fn session_db(&self) -> PathBuf {
        self.db_path("session/session.db")
    }

    /// Probe the account: decrypt `session.db` and read its schema.
    pub fn test_connection(&self) -> Result<()> {
        self.with_db(&self.session_db(), |conn| {
            conn.query_row("select count(*) from sqlite_master", [], |r| {
                r.get::<_, i64>(0)
            })?;
            Ok(())
        })
    }

    /// Rows of `SessionTable`, newest first — the same columns WeChat stores.
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
}
