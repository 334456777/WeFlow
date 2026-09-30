//! Native (pure Rust + bundled SQLite) replacement for the closed `wcdb_api` library.
//!
//! An account is a `db_storage` directory plus the raw database key. Each database file is
//! decrypted (see [`crate::sqlcipher`]) into an in-memory read-only SQLite snapshot the first time
//! it is used, and re-snapshotted whenever the file or its `-wal` changes on disk, so long-running
//! commands (`serve`, message push) keep seeing new messages. Nothing plaintext is written to disk.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use rusqlite::serialize::OwnedData;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use serde_json::{json, Map, Value};

use crate::sqlcipher::{self, PageCipher};

/// Total decrypted bytes kept in memory before older snapshots are dropped.
const CACHE_BUDGET: usize = 1536 * 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
struct FileStamp {
    db_len: u64,
    db_mtime: Option<SystemTime>,
    wal_len: u64,
    wal_mtime: Option<SystemTime>,
}

struct Snapshot {
    conn: Arc<Mutex<Connection>>,
    cipher: Arc<PageCipher>,
    key: [u8; 32],
    stamp: FileStamp,
    bytes: usize,
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
    let suffixed = |long: &str, short: &str| long.len() == short.len() + 5 && long.starts_with(short) && long.as_bytes()[short.len()] == b'_';
    a == b || suffixed(&a, &b) || suffixed(&b, &a)
}

/// Decrypted snapshots are shared by every account handle in the process: the service layer opens a
/// fresh handle per call, and re-decrypting a large message database each time would dominate the run.
static SHARED_CACHE: OnceLock<Arc<Mutex<Cache>>> = OnceLock::new();

fn shared_cache() -> Arc<Mutex<Cache>> {
    SHARED_CACHE.get_or_init(|| Arc::new(Mutex::new(Cache::default()))).clone()
}

pub struct NativeAccount {
    db_storage: PathBuf,
    raw_key: [u8; 32],
    cache: Arc<Mutex<Cache>>,
    /// Identity of the account owner (wxid, optionally with the `_xxxx` account-directory suffix);
    /// used to tell which messages were sent by the user.
    my_wxid: Option<String>,
    pub(crate) cursors: Mutex<crate::native_msg::Cursors>,
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
const HEX_BLOB_COLUMNS: &[&str] = &["packed_info_data", "extra_buffer", "ext_buffer", "reserved0", "voice_data"];

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
    bytes.iter().map(|x| format!("{x:02x}")).collect()
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
        Ok(Self { db_storage, raw_key: sqlcipher::parse_key(hex_key)?, cache: shared_cache(), my_wxid: None, cursors: Mutex::default() })
    }

    /// Set the account owner's wxid (see [`NativeAccount::is_me`]).
    pub fn with_my_wxid(mut self, wxid: Option<String>) -> Self {
        self.my_wxid = wxid.map(|w| w.trim().to_string()).filter(|w| !w.is_empty());
        self
    }

    /// Whether `sender` is the account owner. Message databases store the bare wxid while the account
    /// directory is `<wxid>_<4 chars>`, so a bare id matches its suffixed form and the other way round.
    pub fn is_me(&self, sender: &str) -> bool {
        self.my_wxid.as_deref().is_some_and(|me| same_identity(sender, me))
    }

    pub fn db_storage(&self) -> &Path {
        &self.db_storage
    }

    /// Absolute path of a database given relative to `db_storage` (`session/session.db`).
    pub fn db_path(&self, rel: &str) -> PathBuf {
        self.db_storage.join(rel)
    }

    /// Open (or refresh) the snapshot of `path` and run `f` on it.
    pub fn with_db<T>(&self, path: &Path, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let conn = self.snapshot(path)?;
        let guard = conn.lock().map_err(|_| anyhow!("database snapshot lock poisoned"))?;
        f(&guard)
    }

    fn snapshot(&self, path: &Path) -> Result<Arc<Mutex<Connection>>> {
        let stamp = stamp_of(path)?;
        let mut cache = self.cache.lock().map_err(|_| anyhow!("snapshot cache lock poisoned"))?;
        cache.tick += 1;
        let tick = cache.tick;
        let mut known_cipher = None;
        if let Some(entry) = cache.entries.get_mut(path) {
            if entry.key == self.raw_key {
                if entry.stamp == stamp {
                    entry.last_used = tick;
                    return Ok(entry.conn.clone());
                }
                known_cipher = Some(entry.cipher.clone()); // same file => same salt => reuse the slow KDF result
            }
        }

        // `stamp` was taken before reading: a change that lands while we read makes the next call reload.
        let (data, bytes, cipher) = self.load_plain(path, known_cipher)?;
        let mut conn = Connection::open_in_memory()?;
        conn.deserialize(rusqlite::MAIN_DB, data, true)
            .with_context(|| format!("failed to open decrypted snapshot of {}", path.display()))?;
        let conn = Arc::new(Mutex::new(conn));
        cache.entries.insert(
            path.to_path_buf(),
            Snapshot { conn: conn.clone(), cipher, key: self.raw_key, stamp, bytes, last_used: tick },
        );
        Self::evict(&mut cache, path);
        Ok(conn)
    }

    /// Read db + wal consistently (retry if the WAL was reset by a checkpoint in between) and decrypt the
    /// main file page by page straight into a buffer owned by SQLite, so a large database is held once.
    fn load_plain(&self, path: &Path, cipher: Option<Arc<PageCipher>>) -> Result<(OwnedData, usize, Arc<PageCipher>)> {
        use std::io::{BufReader, Read, Seek, SeekFrom};
        let wal_file = wal_path(path);
        let mut last_err = None;
        for _ in 0..4 {
            let wal_before = fs::read(&wal_file).ok();
            let mut file = fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
            let len = file.metadata()?.len();
            let mut first = vec![0u8; sqlcipher::PAGE_SIZE];
            file.read_exact(&mut first).with_context(|| format!("{} is smaller than one page", path.display()))?;
            let cipher = match &cipher {
                Some(c) => c.clone(),
                None => Arc::new(sqlcipher::verify_key(&first, &self.raw_key).with_context(|| {
                    format!("cannot decrypt {} with the configured database key", path.display())
                })?),
            };
            let plan = sqlcipher::plan(len, wal_before.as_deref());
            let bytes = plan.plain_len();
            // SAFETY: `sqlite3_malloc64` memory is what `OwnedData` (and SQLITE_DESERIALIZE_FREEONCLOSE) expects;
            // the slice covers exactly the allocation and is dropped before the buffer is handed over.
            let ptr = std::ptr::NonNull::new(unsafe { rusqlite::ffi::sqlite3_malloc64(bytes as u64) }.cast::<u8>())
                .ok_or_else(|| anyhow!("out of memory reading {}", path.display()))?;
            let data = unsafe { OwnedData::from_raw_nonnull(ptr, bytes) };
            file.seek(SeekFrom::Start(0))?;
            // SAFETY: see above; `data` owns `bytes` bytes and is not aliased while this slice lives.
            let out = unsafe { std::slice::from_raw_parts_mut(ptr.as_ptr(), bytes) };
            let decrypted = sqlcipher::decrypt_into(out, BufReader::with_capacity(1 << 20, &mut file), &plan, wal_before.as_deref(), &cipher);
            let wal_after = fs::read(&wal_file).ok();
            let stable = match (&wal_before, &wal_after) {
                (Some(a), Some(b)) => a.len() >= 32 && b.len() >= 32 && a[..32] == b[..32],
                (None, None) => true,
                _ => false,
            };
            match decrypted {
                Ok(()) if stable => return Ok((data, bytes, cipher)),
                Ok(()) => last_err = Some(anyhow!("{} kept changing while it was being read", path.display())),
                Err(e) => last_err = Some(e), // a page changed mid-read: try again (`data` is freed on drop)
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow!("{} kept changing while it was being read", path.display())))
    }

    fn evict(cache: &mut Cache, keep: &Path) {
        loop {
            let total: usize = cache.entries.values().map(|s| s.bytes).sum();
            if total <= CACHE_BUDGET {
                return;
            }
            let victim = cache
                .entries
                .iter()
                .filter(|(p, _)| p.as_path() != keep)
                .min_by_key(|(_, s)| s.last_used)
                .map(|(p, _)| p.clone());
            match victim {
                Some(p) => {
                    cache.entries.remove(&p);
                }
                None => return,
            }
        }
    }

    /// Run a read-only query on `path` and return every row as a JSON object.
    pub fn query(&self, path: &Path, sql: &str, params: &[&dyn rusqlite::ToSql]) -> Result<Vec<Value>> {
        self.with_db(path, |conn| {
            let mut stmt = conn.prepare(sql).with_context(|| format!("bad query: {sql}"))?;
            let names: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
            let mut rows = stmt.query(params)?;
            let mut out = Vec::new();
            while let Some(row) = rows.next()? {
                let mut obj = Map::new();
                for (i, name) in names.iter().enumerate() {
                    obj.insert(name.clone(), value_to_json(name, row.get_ref(i)?));
                }
                out.push(Value::Object(obj));
            }
            Ok(out)
        })
    }

    pub fn session_db(&self) -> PathBuf {
        self.db_path("session/session.db")
    }

    /// Probe the account: decrypt `session.db` and read its schema.
    pub fn test_connection(&self) -> Result<()> {
        self.with_db(&self.session_db(), |conn| {
            conn.query_row("select count(*) from sqlite_master", [], |r| r.get::<_, i64>(0))?;
            Ok(())
        })
    }

    /// Rows of `SessionTable`, newest first — the same columns WeChat stores.
    pub fn sessions(&self) -> Result<Value> {
        let rows = self.query(&self.session_db(), "select * from SessionTable order by sort_timestamp desc", &[])?;
        Ok(Value::Array(rows))
    }
}
