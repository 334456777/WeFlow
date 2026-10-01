//! Read-only SQLite VFS that decrypts WeChat's SQLCipher pages on demand.
//!
//! A database snapshot is an ordinary SQLite connection whose main file is served by this VFS: a page is read from
//! disk, its HMAC checked and its body decrypted only when SQLite asks for it, and SQLite's own page cache keeps the
//! pages a query uses. So a query that touches a few pages of a 200 MB message database reads a few pages, not the
//! whole file, and memory follows the page cache instead of the database size.
//!
//! The newest committed WAL frames are copied when the snapshot is opened, so a connection sees one state of the
//! database. WeChat keeps writing while we read, though: a checkpoint can copy WAL frames that are newer than our copy
//! into the main file, and a page read after that would mix two states. The wal-index (`-shm`, plain SQLite format)
//! says how far checkpoints have gone; every read from the main file is followed by a check ([`ChangeGuard`]) and a
//! read that may be newer than the snapshot fails and marks the snapshot stale. [`crate::native_db`] then opens a
//! fresh snapshot and runs the query again.

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::fs::{self, File};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use rusqlite::{ffi, Connection, OpenFlags};

use crate::sqlcipher::{self, PageCipher, PAGE_SIZE};

const VFS_NAME: &CStr = c"weflow-cipher";
/// Prefix of the names under which snapshots are registered with the VFS (they are not paths).
const NAME_PREFIX: &str = "weflow-cipher:";
/// Pages read from the main file at once (raw, still encrypted); a full scan needs 16x fewer reads and checks.
const CHUNK_PAGES: u32 = 16;
/// Raw chunks kept per open file (`CHUNKS_KEPT * CHUNK_PAGES * 4 KiB` = 2 MiB).
const CHUNKS_KEPT: usize = 32;

/// Positional read of exactly `buf.len()` bytes.
fn read_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut done = 0;
        while done < buf.len() {
            let n = file.seek_read(&mut buf[done..], offset + done as u64)?;
            if n == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            done += n;
        }
        Ok(())
    }
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut os = path.as_os_str().to_os_string();
    os.push(suffix);
    PathBuf::from(os)
}

// ───────────────────────────── change guard ─────────────────────────────

/// What the wal-index says about checkpoints: the WAL generation (salt) and how many frames have been, or are being,
/// copied into the main file.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ShmState {
    salt: [u8; 8],
    backfilled: u32,
    attempted: u32,
}

impl ShmState {
    /// Parse the first 136 bytes of a `-shm` file (two copies of the 48-byte header, then the checkpoint info).
    /// `None` while the header is uninitialised or being rewritten (the two copies differ).
    fn parse(b: &[u8]) -> Option<Self> {
        if b.len() < 136 || b[12] == 0 || b[0..48] != b[48..96] {
            return None;
        }
        let u32_at = |i: usize| u32::from_ne_bytes(b[i..i + 4].try_into().expect("4 bytes"));
        Some(Self { salt: b[32..40].try_into().expect("8 bytes"), backfilled: u32_at(96), attempted: u32_at(128) })
    }

    /// Frames a checkpoint may have written into the main file.
    fn written(&self) -> u32 {
        self.backfilled.max(self.attempted)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct MainStamp {
    len: u64,
    mtime: Option<SystemTime>,
}

fn main_stamp(file: &File) -> Option<MainStamp> {
    let m = file.metadata().ok()?;
    Some(MainStamp { len: m.len(), mtime: m.modified().ok() })
}

/// Decides whether pages read from the main file still belong to the snapshot.
///
/// A checkpoint sets `nBackfillAttempted` before it writes any page and `nBackfill` after, and a new WAL generation
/// gets a new salt. So after a main-file read, the page is from our state when the wal-index still has our WAL's salt
/// and no checkpoint has gone past the frames we copied. When the wal-index does not describe our WAL (stale index
/// left by a closed WeChat, or no WAL at all), the fallback is "nothing changed since the snapshot was opened".
struct ChangeGuard {
    shm_path: PathBuf,
    shm: Mutex<Option<File>>,
    /// Salt and committed frame count of the WAL copy the snapshot holds.
    wal_salt: Option<[u8; 8]>,
    frames: u32,
    at_open: Option<ShmState>,
    stamp_at_open: Option<MainStamp>,
}

impl ChangeGuard {
    fn read_shm(&self) -> Option<ShmState> {
        let mut slot = self.shm.lock().ok()?;
        if slot.is_none() {
            *slot = File::open(&self.shm_path).ok();
        }
        let file = slot.as_ref()?;
        let mut buf = [0u8; 136];
        for _ in 0..3 {
            if read_at(file, &mut buf, 0).is_err() {
                *slot = None;
                return None;
            }
            if let Some(s) = ShmState::parse(&buf) {
                return Some(s);
            }
        }
        None
    }

    /// The precise rule: the wal-index describes our WAL and no checkpoint has gone past our frames.
    fn describes_our_wal(&self, now: &ShmState) -> bool {
        self.wal_salt == Some(now.salt) && now.written() <= self.frames
    }

    /// Whether a page read from the main file just now is from the snapshot's state.
    fn consistent(&self, main: &File) -> bool {
        match self.read_shm() {
            Some(now) => {
                if self.describes_our_wal(&now) {
                    return true;
                }
                // Not our WAL generation: only safe when nothing moved since we opened (and no checkpoint was running).
                self.at_open == Some(now) && now.attempted == now.backfilled && main_stamp(main) == self.stamp_at_open
            }
            None => self.at_open.is_none() && main_stamp(main) == self.stamp_at_open,
        }
    }
}

// ───────────────────────────── one snapshot ─────────────────────────────

/// One consistent, read-only view of an encrypted database file, decrypted page by page.
pub struct CipherFile {
    path: PathBuf,
    file: File,
    cipher: Arc<PageCipher>,
    main_pages: u32,
    total_pages: u32,
    /// Page 1, decrypted and patched (see [`sqlcipher::patch_header`]).
    page1: Box<[u8]>,
    /// Encrypted copy of the newest committed WAL frame of each page (taken at open).
    wal_pages: HashMap<u32, Box<[u8]>>,
    chunks: Mutex<ChunkCache>,
    guard: ChangeGuard,
    stale: AtomicBool,
    error: Mutex<Option<String>>,
    pages_decrypted: AtomicU64,
}

#[derive(Default)]
struct ChunkCache {
    chunks: HashMap<u32, Arc<[u8]>>,
    order: VecDeque<u32>,
}

impl CipherFile {
    /// Open `path` (plus its `-wal`) for on-demand reading. `cipher` is reused when the caller already derived the
    /// keys for this file (same salt), which skips the slow key derivation.
    pub fn open(path: &Path, raw_key: &[u8; 32], cipher: Option<Arc<PageCipher>>) -> Result<Self> {
        let wal_path = sibling(path, "-wal");
        let shm_path = sibling(path, "-shm");
        let mut cipher = cipher;
        let mut last_err = None;
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(20 * attempt));
            }
            // Order matters: wal-index first, then the WAL, so the WAL copy holds every frame the index counted.
            let shm = File::open(&shm_path).ok();
            let at_open = shm.as_ref().and_then(|f| {
                let mut b = [0u8; 136];
                read_at(f, &mut b, 0).ok().and_then(|_| ShmState::parse(&b))
            });
            let wal = fs::read(&wal_path).ok();
            let file = File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
            let stamp_at_open = main_stamp(&file);
            let len = stamp_at_open.map(|s| s.len).unwrap_or(0);
            let mut first = vec![0u8; PAGE_SIZE];
            read_at(&file, &mut first, 0).with_context(|| format!("{} is smaller than one page", path.display()))?;
            let c = match &cipher {
                Some(c) => c.clone(),
                None => {
                    let c = Arc::new(sqlcipher::verify_key(&first, raw_key).with_context(|| {
                        format!("cannot decrypt {} with the configured database key", path.display())
                    })?);
                    cipher = Some(c.clone());
                    c
                }
            };
            let overlay = sqlcipher::wal_overlay(wal.as_deref());
            let total_pages = sqlcipher::total_pages(len, &overlay);
            let mut wal_pages = HashMap::with_capacity(overlay.pages.len());
            if let Some(wal) = &wal {
                for (&pgno, &off) in &overlay.pages {
                    if pgno <= total_pages {
                        wal_pages.insert(pgno, wal[off..off + PAGE_SIZE].to_vec().into_boxed_slice());
                    }
                }
            }
            let mut page1 = vec![0u8; PAGE_SIZE];
            let decrypted = match wal_pages.get(&1) {
                Some(enc) => c.decrypt_page(1, enc, &mut page1),
                None => c.decrypt_page(1, &first, &mut page1),
            };
            if let Err(e) = decrypted {
                last_err = Some(e); // the WAL was being written while we copied it
                continue;
            }
            sqlcipher::patch_header(&mut page1, total_pages);

            let guard = ChangeGuard {
                shm_path: shm_path.clone(),
                shm: Mutex::new(shm),
                wal_salt: overlay.salt,
                frames: overlay.frames,
                at_open,
                stamp_at_open,
            };
            // A WAL reset between reading the index and the WAL shows up as a different salt in the WAL header.
            let wal_header_now = fs::read(&wal_path).ok().map(|w| w.get(..32).map(<[u8]>::to_vec));
            let wal_stable = wal_header_now == wal.as_ref().map(|w| w.get(..32).map(<[u8]>::to_vec));
            // A checkpoint running right now over frames we do not hold may be rewriting main-file pages: wait for it.
            // Anything else is fine to start from; the guard watches what happens next.
            let index_ok = match &at_open {
                Some(s) => guard.describes_our_wal(s) || s.attempted == s.backfilled || attempt >= 3,
                None => true,
            };
            if !wal_stable || !index_ok {
                last_err = Some(anyhow!("{} kept changing while it was being opened", path.display()));
                continue;
            }
            return Ok(Self {
                path: path.to_path_buf(),
                file,
                cipher: c,
                main_pages: (len / PAGE_SIZE as u64) as u32,
                total_pages,
                page1: page1.into_boxed_slice(),
                wal_pages,
                chunks: Mutex::default(),
                guard,
                stale: AtomicBool::new(false),
                error: Mutex::new(None),
                pages_decrypted: AtomicU64::new(0),
            });
        }
        Err(last_err.unwrap_or_else(|| anyhow!("{} kept changing while it was being opened", path.display())))
    }

    pub fn cipher(&self) -> Arc<PageCipher> {
        self.cipher.clone()
    }

    /// Whether a read found that the file moved past this snapshot (the query should be run on a fresh one).
    pub fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Relaxed)
    }

    /// The first read error, for reporting when retries do not help.
    pub fn read_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    /// Pages decrypted so far (for tests and statistics).
    pub fn pages_decrypted(&self) -> u64 {
        self.pages_decrypted.load(Ordering::Relaxed)
    }

    fn fail(&self, why: String) -> c_int {
        self.stale.store(true, Ordering::Relaxed);
        if let Ok(mut e) = self.error.lock() {
            e.get_or_insert(why);
        }
        ffi::SQLITE_IOERR_READ
    }

    /// Raw (encrypted) chunk holding `pgno`, read from disk and checked against the guard when not cached.
    fn chunk(&self, pgno: u32) -> std::result::Result<(Arc<[u8]>, u32), c_int> {
        let idx = (pgno - 1) / CHUNK_PAGES;
        let first = idx * CHUNK_PAGES + 1;
        if let Some(c) = self.chunks.lock().ok().and_then(|c| c.chunks.get(&idx).cloned()) {
            return Ok((c, first));
        }
        let count = CHUNK_PAGES.min(self.main_pages + 1 - first);
        let mut buf = vec![0u8; count as usize * PAGE_SIZE];
        if let Err(e) = read_at(&self.file, &mut buf, (first as u64 - 1) * PAGE_SIZE as u64) {
            return Err(self.fail(format!("cannot read {}: {e}", self.path.display())));
        }
        // The pages are in memory now; if no checkpoint moved past our snapshot until this moment, they are ours.
        if !self.guard.consistent(&self.file) {
            return Err(self.fail(format!("{} changed while it was being read", self.path.display())));
        }
        let chunk: Arc<[u8]> = buf.into();
        if let Ok(mut cache) = self.chunks.lock() {
            cache.chunks.insert(idx, chunk.clone());
            cache.order.push_back(idx);
            while cache.order.len() > CHUNKS_KEPT {
                if let Some(old) = cache.order.pop_front() {
                    cache.chunks.remove(&old);
                }
            }
        }
        Ok((chunk, first))
    }

    /// Decrypted page `pgno` into `out` (`PAGE_SIZE` bytes).
    fn page_into(&self, pgno: u32, out: &mut [u8]) -> std::result::Result<(), c_int> {
        if pgno == 1 {
            out.copy_from_slice(&self.page1);
            return Ok(());
        }
        let decrypted = if let Some(enc) = self.wal_pages.get(&pgno) {
            self.cipher.decrypt_page(pgno, enc, out)
        } else if pgno > self.main_pages {
            out.fill(0); // beyond the main file and not in the WAL: never written
            return Ok(());
        } else {
            let (chunk, first) = self.chunk(pgno)?;
            let i = (pgno - first) as usize * PAGE_SIZE;
            self.cipher.decrypt_page(pgno, &chunk[i..i + PAGE_SIZE], out)
        };
        self.pages_decrypted.fetch_add(1, Ordering::Relaxed);
        // A torn page (written while we read it) fails its HMAC: treat it like any other change.
        decrypted.map_err(|e| self.fail(format!("{}: {e}", self.path.display())))
    }

    /// `xRead`: `out.len()` bytes at `offset` of the plain database.
    fn read(&self, out: &mut [u8], offset: u64) -> c_int {
        let total = self.total_pages as u64 * PAGE_SIZE as u64;
        let mut done = 0usize;
        let mut scratch: Option<Vec<u8>> = None;
        while done < out.len() {
            let pos = offset + done as u64;
            if pos >= total {
                out[done..].fill(0);
                return ffi::SQLITE_IOERR_SHORT_READ;
            }
            let pgno = (pos / PAGE_SIZE as u64) as u32 + 1;
            let in_page = (pos % PAGE_SIZE as u64) as usize;
            let n = (out.len() - done).min(PAGE_SIZE - in_page);
            let result = if in_page == 0 && n == PAGE_SIZE {
                self.page_into(pgno, &mut out[done..done + PAGE_SIZE])
            } else {
                let buf = scratch.get_or_insert_with(|| vec![0u8; PAGE_SIZE]);
                self.page_into(pgno, buf).map(|_| out[done..done + n].copy_from_slice(&buf[in_page..in_page + n]))
            };
            if let Err(rc) = result {
                return rc;
            }
            done += n;
        }
        ffi::SQLITE_OK
    }

    fn size(&self) -> u64 {
        self.total_pages as u64 * PAGE_SIZE as u64
    }
}

// ───────────────────────────── SQLite glue ─────────────────────────────

fn registry() -> &'static Mutex<HashMap<String, Arc<CipherFile>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<CipherFile>>>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

/// Open a read-only SQLite connection on `file`.
pub fn open_connection(file: Arc<CipherFile>) -> Result<Connection> {
    register_vfs()?;
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let name = format!("{NAME_PREFIX}{}", NEXT.fetch_add(1, Ordering::Relaxed));
    registry().lock().map_err(|_| anyhow!("vfs registry poisoned"))?.insert(name.clone(), file.clone());
    let conn = Connection::open_with_flags_and_vfs(&name, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX, VFS_NAME);
    // The connection holds its own reference from `xOpen` on.
    registry().lock().map_err(|_| anyhow!("vfs registry poisoned"))?.remove(&name);
    let conn = conn.with_context(|| format!("cannot open {}", file.path.display()))?;
    Ok(conn)
}

#[repr(C)]
struct VfsFile {
    base: ffi::sqlite3_file,
    file: *const CipherFile,
}

fn default_vfs(vfs: *mut ffi::sqlite3_vfs) -> *mut ffi::sqlite3_vfs {
    // SAFETY: `pAppData` is set to the default VFS when ours is registered and never changes.
    unsafe { (*vfs).pAppData.cast() }
}

/// Our names are not paths; they never reach the default VFS.
unsafe fn our_name(name: *const c_char) -> Option<&'static str> {
    if name.is_null() {
        return None;
    }
    CStr::from_ptr(name).to_str().ok().filter(|n| n.starts_with(NAME_PREFIX))
}

fn register_vfs() -> Result<()> {
    static DONE: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    DONE.get_or_init(|| unsafe {
        let default = ffi::sqlite3_vfs_find(std::ptr::null());
        if default.is_null() {
            return Err("SQLite has no default VFS".into());
        }
        let d = &*default;
        let vfs = Box::new(ffi::sqlite3_vfs {
            iVersion: 2,
            szOsFile: d.szOsFile.max(std::mem::size_of::<VfsFile>() as c_int),
            mxPathname: d.mxPathname,
            pNext: std::ptr::null_mut(),
            zName: VFS_NAME.as_ptr(),
            pAppData: default.cast(),
            xOpen: Some(x_open),
            xDelete: Some(x_delete),
            xAccess: Some(x_access),
            xFullPathname: Some(x_full_pathname),
            // The OS VFSes ignore their `sqlite3_vfs*` argument in these, so they can be shared as they are.
            xDlOpen: d.xDlOpen,
            xDlError: d.xDlError,
            xDlSym: d.xDlSym,
            xDlClose: d.xDlClose,
            xRandomness: d.xRandomness,
            xSleep: d.xSleep,
            xCurrentTime: d.xCurrentTime,
            xGetLastError: d.xGetLastError,
            xCurrentTimeInt64: d.xCurrentTimeInt64,
            xSetSystemCall: None,
            xGetSystemCall: None,
            xNextSystemCall: None,
        });
        match ffi::sqlite3_vfs_register(Box::leak(vfs), 0) {
            ffi::SQLITE_OK => Ok(()),
            rc => Err(format!("cannot register the decrypting SQLite VFS (code {rc})")),
        }
    })
    .clone()
    .map_err(|e| anyhow!(e))
}

unsafe extern "C" fn x_open(vfs: *mut ffi::sqlite3_vfs, name: *const c_char, file: *mut ffi::sqlite3_file, flags: c_int, out_flags: *mut c_int) -> c_int {
    (*file).pMethods = std::ptr::null();
    if let Some(n) = our_name(name) {
        let found = if flags & ffi::SQLITE_OPEN_MAIN_DB != 0 { registry().lock().ok().and_then(|r| r.get(n).cloned()) } else { None };
        let Some(cf) = found else { return ffi::SQLITE_CANTOPEN };
        let f = file.cast::<VfsFile>();
        (*f).file = Arc::into_raw(cf);
        (*f).base.pMethods = &IO_METHODS;
        if !out_flags.is_null() {
            *out_flags = (flags & !(ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE)) | ffi::SQLITE_OPEN_READONLY;
        }
        return ffi::SQLITE_OK;
    }
    // Temporary files (sorting, materialised views) go to the OS VFS.
    let d = default_vfs(vfs);
    match (*d).xOpen {
        Some(open) => open(d, name, file, flags, out_flags),
        None => ffi::SQLITE_CANTOPEN,
    }
}

unsafe extern "C" fn x_delete(vfs: *mut ffi::sqlite3_vfs, name: *const c_char, sync_dir: c_int) -> c_int {
    if our_name(name).is_some() {
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    (*d).xDelete.map_or(ffi::SQLITE_IOERR_DELETE, |f| f(d, name, sync_dir))
}

unsafe extern "C" fn x_access(vfs: *mut ffi::sqlite3_vfs, name: *const c_char, flags: c_int, out: *mut c_int) -> c_int {
    if our_name(name).is_some() {
        *out = 0; // no journal, no WAL: a snapshot is a single immutable file
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    (*d).xAccess.map_or(ffi::SQLITE_IOERR_ACCESS, |f| f(d, name, flags, out))
}

unsafe extern "C" fn x_full_pathname(vfs: *mut ffi::sqlite3_vfs, name: *const c_char, n_out: c_int, out: *mut c_char) -> c_int {
    if let Some(n) = our_name(name) {
        let bytes = n.as_bytes();
        if bytes.len() + 1 > n_out.max(0) as usize {
            return ffi::SQLITE_CANTOPEN;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, bytes.len());
        *out.add(bytes.len()) = 0;
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    (*d).xFullPathname.map_or(ffi::SQLITE_CANTOPEN, |f| f(d, name, n_out, out))
}

static IO_METHODS: ffi::sqlite3_io_methods = ffi::sqlite3_io_methods {
    iVersion: 1,
    xClose: Some(io_close),
    xRead: Some(io_read),
    xWrite: Some(io_write),
    xTruncate: Some(io_truncate),
    xSync: Some(io_sync),
    xFileSize: Some(io_file_size),
    xLock: Some(io_lock),
    xUnlock: Some(io_lock),
    xCheckReservedLock: Some(io_check_reserved_lock),
    xFileControl: Some(io_file_control),
    xSectorSize: Some(io_sector_size),
    xDeviceCharacteristics: Some(io_device_characteristics),
    xShmMap: None,
    xShmLock: None,
    xShmBarrier: None,
    xShmUnmap: None,
    xFetch: None,
    xUnfetch: None,
};

unsafe fn cipher_file<'a>(f: *mut ffi::sqlite3_file) -> &'a CipherFile {
    &*(*f.cast::<VfsFile>()).file
}

unsafe extern "C" fn io_close(f: *mut ffi::sqlite3_file) -> c_int {
    let vf = f.cast::<VfsFile>();
    if !(*vf).file.is_null() {
        drop(Arc::from_raw((*vf).file));
        (*vf).file = std::ptr::null();
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_read(f: *mut ffi::sqlite3_file, buf: *mut c_void, amt: c_int, offset: ffi::sqlite3_int64) -> c_int {
    if amt < 0 || offset < 0 {
        return ffi::SQLITE_IOERR_READ;
    }
    let cf = cipher_file(f);
    let out = std::slice::from_raw_parts_mut(buf.cast::<u8>(), amt as usize);
    catch_unwind(AssertUnwindSafe(|| cf.read(out, offset as u64))).unwrap_or(ffi::SQLITE_IOERR_READ)
}

unsafe extern "C" fn io_write(_: *mut ffi::sqlite3_file, _: *const c_void, _: c_int, _: ffi::sqlite3_int64) -> c_int {
    ffi::SQLITE_READONLY
}

unsafe extern "C" fn io_truncate(_: *mut ffi::sqlite3_file, _: ffi::sqlite3_int64) -> c_int {
    ffi::SQLITE_READONLY
}

unsafe extern "C" fn io_sync(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_file_size(f: *mut ffi::sqlite3_file, size: *mut ffi::sqlite3_int64) -> c_int {
    *size = cipher_file(f).size() as ffi::sqlite3_int64;
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_lock(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_check_reserved_lock(_: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    *out = 0;
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_file_control(_: *mut ffi::sqlite3_file, _: c_int, _: *mut c_void) -> c_int {
    ffi::SQLITE_NOTFOUND
}

unsafe extern "C" fn io_sector_size(_: *mut ffi::sqlite3_file) -> c_int {
    PAGE_SIZE as c_int
}

unsafe extern "C" fn io_device_characteristics(_: *mut ffi::sqlite3_file) -> c_int {
    ffi::SQLITE_IOCAP_IMMUTABLE
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sqlcipher::testutil::{encrypt_db, encrypt_page, plain_db, KEY, SALT};

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("weflow-vfs-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn rows(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("select v from t order by id").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    fn open(path: &Path) -> (Arc<CipherFile>, Connection) {
        let cf = Arc::new(CipherFile::open(path, &KEY, None).unwrap());
        let conn = open_connection(cf.clone()).unwrap();
        (cf, conn)
    }

    #[test]
    fn reads_the_same_rows_as_a_full_decryption_and_only_the_pages_it_needs() {
        let dir = temp_dir("rows");
        let cipher = PageCipher::derive(&KEY, &SALT);
        // big enough for several chunks: ~3000 rows of 200 bytes
        let values: Vec<String> = (0..3000).map(|i| format!("{i:05}{}", "x".repeat(200))).collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let enc = encrypt_db(&plain_db(&refs), &cipher);
        let pages = (enc.len() / PAGE_SIZE) as u64;
        assert!(pages > 2 * CHUNK_PAGES as u64);
        let path = dir.join("a.db");
        fs::write(&path, &enc).unwrap();
        let (cf, conn) = open(&path);
        let one: String = conn.query_row("select v from t where id = 1234", [], |r| r.get(0)).unwrap();
        assert_eq!(one, values[1234]);
        assert!(cf.pages_decrypted() < 10, "a point lookup decrypted {} of {pages} pages", cf.pages_decrypted());
        assert_eq!(rows(&conn), values);
        let full = sqlcipher::decrypt_database(&enc, None, &cipher).unwrap();
        let mut mem = Connection::open_in_memory().unwrap();
        mem.deserialize_read_exact(rusqlite::MAIN_DB, &full[..], full.len(), true).unwrap();
        assert_eq!(rows(&mem), values);
        assert!(!cf.is_stale());
    }

    #[test]
    fn wrong_key_and_plain_files_are_refused() {
        let dir = temp_dir("key");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        fs::write(&path, encrypt_db(&plain_db(&["x"]), &cipher)).unwrap();
        assert!(CipherFile::open(&path, &[9u8; 32], None).is_err());
        fs::write(&path, plain_db(&["x"])).unwrap();
        assert!(CipherFile::open(&path, &KEY, None).is_err());
    }

    fn wal_header(salt: (u32, u32)) -> Vec<u8> {
        let mut h = Vec::new();
        for v in [0x377f0683u32, 3007000, PAGE_SIZE as u32, 0, salt.0, salt.1] {
            h.extend(v.to_be_bytes());
        }
        let (c0, c1) = super::sqlcipher_checksum(&h);
        h.extend(c0.to_be_bytes());
        h.extend(c1.to_be_bytes());
        h
    }

    /// A WAL holding one committed transaction that rewrites every page of `newer`.
    fn wal_with(cipher: &PageCipher, newer: &[u8], salt: (u32, u32)) -> Vec<u8> {
        let mut wal = wal_header(salt);
        let mut sum = (u32::from_be_bytes(wal[24..28].try_into().unwrap()), u32::from_be_bytes(wal[28..32].try_into().unwrap()));
        let n = (newer.len() / PAGE_SIZE) as u32;
        for pgno in 1..=n {
            let page = encrypt_page(cipher, pgno, &newer[(pgno as usize - 1) * PAGE_SIZE..pgno as usize * PAGE_SIZE], [0x55; 16]);
            let mut fh = Vec::new();
            fh.extend(pgno.to_be_bytes());
            fh.extend((if pgno == n { n } else { 0 }).to_be_bytes());
            fh.extend(salt.0.to_be_bytes());
            fh.extend(salt.1.to_be_bytes());
            let s = super::sqlcipher_checksum_from(&fh[..8], sum);
            let s = super::sqlcipher_checksum_from(&page, s);
            fh.extend(s.0.to_be_bytes());
            fh.extend(s.1.to_be_bytes());
            sum = s;
            wal.extend(fh);
            wal.extend(page);
        }
        wal
    }

    /// A wal-index header (both copies) plus checkpoint info.
    fn shm(salt: (u32, u32), backfilled: u32, attempted: u32) -> Vec<u8> {
        let mut b = vec![0u8; 32768];
        let mut hdr = [0u8; 48];
        hdr[0..4].copy_from_slice(&3007000u32.to_ne_bytes());
        hdr[12] = 1; // isInit
        hdr[32..36].copy_from_slice(&salt.0.to_be_bytes());
        hdr[36..40].copy_from_slice(&salt.1.to_be_bytes());
        b[0..48].copy_from_slice(&hdr);
        b[48..96].copy_from_slice(&hdr);
        b[96..100].copy_from_slice(&backfilled.to_ne_bytes());
        b[128..132].copy_from_slice(&attempted.to_ne_bytes());
        b
    }

    #[test]
    fn wal_frames_override_the_main_file() {
        let dir = temp_dir("wal");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        fs::write(&path, encrypt_db(&plain_db(&["old"]), &cipher)).unwrap();
        fs::write(sibling(&path, "-wal"), wal_with(&cipher, &plain_db(&["new"]), (1, 2))).unwrap();
        let (_, conn) = open(&path);
        assert_eq!(rows(&conn), ["new"]);
    }

    /// Many rows, so a scan reads the main file chunk by chunk after the open.
    fn big(tag: &str) -> Vec<u8> {
        let values: Vec<String> = (0..2000).map(|i| format!("{tag}{i:05}{}", "y".repeat(300))).collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        plain_db(&refs)
    }

    #[test]
    fn a_checkpoint_past_the_snapshot_marks_it_stale() {
        let dir = temp_dir("stale");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let plain = big("a");
        fs::write(&path, encrypt_db(&plain, &cipher)).unwrap();
        // our WAL: salt (1,2), one committed transaction rewriting page 1 only (frames = 1)
        let mut wal = wal_header((1, 2));
        let first_page = encrypt_page(&cipher, 1, &plain[..PAGE_SIZE], [7; 16]);
        let sum = (u32::from_be_bytes(wal[24..28].try_into().unwrap()), u32::from_be_bytes(wal[28..32].try_into().unwrap()));
        let pages = (plain.len() / PAGE_SIZE) as u32;
        let mut fh = Vec::new();
        fh.extend(1u32.to_be_bytes());
        fh.extend(pages.to_be_bytes());
        fh.extend(1u32.to_be_bytes());
        fh.extend(2u32.to_be_bytes());
        let s = super::sqlcipher_checksum_from(&fh[..8], sum);
        let s = super::sqlcipher_checksum_from(&first_page, s);
        fh.extend(s.0.to_be_bytes());
        fh.extend(s.1.to_be_bytes());
        wal.extend(fh);
        wal.extend(first_page);
        fs::write(sibling(&path, "-wal"), &wal).unwrap();
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 1)).unwrap();

        // a checkpoint within our frames is fine
        let (cf, conn) = open(&path);
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());

        // a checkpoint of frames we do not hold (written = 5 > frames = 1): the next main-file read fails
        let (cf, conn) = open(&path);
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 5)).unwrap();
        assert!(conn.prepare("select v from t order by id").and_then(|mut s| s.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<Vec<_>>>()).is_err());
        assert!(cf.is_stale());

        // a new WAL generation (salt changed): stale as well
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 1)).unwrap();
        let (cf, conn) = open(&path);
        fs::write(sibling(&path, "-shm"), shm((2, 9), 0, 0)).unwrap();
        assert!(conn.query_row("select count(v) from t", [], |r| r.get::<_, i64>(0)).is_err());
        assert!(cf.is_stale());
    }

    #[test]
    fn a_stale_index_of_a_closed_wechat_still_reads() {
        // the index belongs to another WAL generation and nothing moves: the fallback accepts it
        let dir = temp_dir("closed");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        fs::write(&path, encrypt_db(&big("b"), &cipher)).unwrap();
        fs::write(sibling(&path, "-shm"), shm((5, 6), 3, 3)).unwrap();
        let (cf, conn) = open(&path);
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());
    }

    #[test]
    fn a_torn_page_marks_the_snapshot_stale() {
        let dir = temp_dir("torn");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let mut enc = encrypt_db(&big("c"), &cipher);
        let (cf, conn) = {
            fs::write(&path, &enc).unwrap();
            open(&path)
        };
        let last = enc.len() - PAGE_SIZE + 100;
        enc[last] ^= 0xff;
        fs::write(&path, &enc).unwrap();
        assert!(conn.query_row("select count(v) from t", [], |r| r.get::<_, i64>(0)).is_err());
        assert!(cf.is_stale());
        assert!(cf.read_error().is_some());
    }

    #[test]
    fn a_torn_page_without_a_visible_change_fails_its_hmac() {
        let dir = temp_dir("hmac");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let mut enc = encrypt_db(&big("e"), &cipher);
        let last = enc.len() - PAGE_SIZE + 100;
        enc[last] ^= 0xff;
        fs::write(&path, &enc).unwrap();
        let (cf, conn) = open(&path);
        assert!(conn.query_row("select count(v) from t", [], |r| r.get::<_, i64>(0)).is_err());
        assert!(cf.is_stale());
        assert!(cf.read_error().unwrap().contains("HMAC"), "{:?}", cf.read_error());
    }

    #[test]
    fn temp_files_still_work() {
        // a sort too big for memory spills to a temporary file of the OS VFS
        let dir = temp_dir("temp");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        fs::write(&path, encrypt_db(&big("d"), &cipher)).unwrap();
        let (_, conn) = open(&path);
        conn.pragma_update(None, "cache_size", 10).unwrap();
        conn.pragma_update(None, "temp_store", 1).unwrap();
        let n: i64 = conn.query_row("select count(*) from (select v from t order by substr(v, 3) desc)", [], |r| r.get(0)).unwrap();
        assert_eq!(n, 2000);
        assert!(conn.execute("insert into t values (99999, 'x')", []).is_err(), "snapshots are read-only");
    }
}

/// WAL checksum helpers for the tests (same algorithm as SQLite's, big-endian words).
#[cfg(test)]
fn sqlcipher_checksum(data: &[u8]) -> (u32, u32) {
    sqlcipher_checksum_from(data, (0, 0))
}

#[cfg(test)]
fn sqlcipher_checksum_from(data: &[u8], (mut s0, mut s1): (u32, u32)) -> (u32, u32) {
    for c in data.chunks_exact(8) {
        let a = u32::from_be_bytes(c[0..4].try_into().unwrap());
        let b = u32::from_be_bytes(c[4..8].try_into().unwrap());
        s0 = s0.wrapping_add(a).wrapping_add(s1);
        s1 = s1.wrapping_add(b).wrapping_add(s0);
    }
    (s0, s1)
}
