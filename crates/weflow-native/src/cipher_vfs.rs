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
//! says how far checkpoints have gone; every read from the main file is followed by a check (`ChangeGuard`) and a
//! read that may be newer than the snapshot fails and marks the snapshot stale. [`crate::db`] then opens a
//! fresh snapshot and runs the query again.

use std::collections::{HashMap, VecDeque};
use std::ffi::{c_char, c_int, c_void, CStr};
use std::fs::{self, File};
use std::ops::ControlFlow;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use anyhow::{anyhow, Context, Result};
use rusqlite::{ffi, Connection, OpenFlags};

use crate::sha512x4;
use crate::sqlcipher::{self, PageCipher, PAGE_SIZE};

const VFS_NAME: &CStr = c"weflow-cipher";
/// Prefix of the names under which snapshots are registered with the VFS (they are not paths).
const NAME_PREFIX: &str = "weflow-cipher:";
/// Pages read from the main file at once (raw, still encrypted); a full scan needs 16x fewer reads and checks.
const CHUNK_PAGES: u32 = 16;
/// Raw chunks kept per open file (`CHUNKS_KEPT * CHUNK_PAGES * 4 KiB` = 2 MiB).
const CHUNKS_KEPT: usize = 32;
/// Pages of a chunk read in order whose HMACs are checked at once ([`CipherFile::check_chunk_page`]): as many as
/// [`sha512x4`] hashes at once.
#[expect(clippy::cast_possible_truncation, reason = "four")]
const CHECKED_TOGETHER: u32 = sha512x4::LANES as u32;

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

/// Bytes in a WAL header.
const WAL_HEADER: usize = 32;

/// The header of the WAL at `path`, read on its own: `None` when the file cannot be read, `Some(None)` when it is
/// shorter than a header.
fn wal_header(path: &Path) -> Option<Option<[u8; WAL_HEADER]>> {
    use std::io::Read;
    let mut header = [0u8; WAL_HEADER];
    match File::open(path).ok()?.read_exact(&mut header) {
        Ok(()) => Some(Some(header)),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Some(None),
        Err(_) => None,
    }
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
        let u32_at = |i: usize| b.get(i..)?.first_chunk().copied().map(u32::from_ne_bytes);
        Some(Self {
            salt: *b.get(32..)?.first_chunk()?,
            backfilled: u32_at(96)?,
            attempted: u32_at(128)?,
        })
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
    Some(MainStamp {
        len: m.len(),
        mtime: m.modified().ok(),
    })
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
                self.at_open == Some(now)
                    && now.attempted == now.backfilled
                    && main_stamp(main) == self.stamp_at_open
            }
            None => self.at_open.is_none() && main_stamp(main) == self.stamp_at_open,
        }
    }

    /// [`Self::consistent`] for a read, which runs on every chunk read from the main file. When there was no
    /// wal-index as the snapshot was opened, the snapshot is the main file as it was then (with the WAL copied at
    /// open): while the file's size and time are still those, a page read from it is the snapshot's, and the
    /// wal-index is not looked for. Looking for a file that is not there is a system call on every read, and on
    /// Windows one that costs tens of microseconds.
    fn consistent_read(&self, main: &File) -> bool {
        (self.at_open.is_none() && main_stamp(main) == self.stamp_at_open) || self.consistent(main)
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
    /// The WAL as it was at open, for [`Self::unchanged`].
    wal: WalAtOpen,
    chunks: Mutex<ChunkCache>,
    guard: ChangeGuard,
    stale: AtomicBool,
    error: Mutex<Option<String>>,
    pages_decrypted: AtomicU64,
}

/// The encrypted bytes of a page as a snapshot holds them ([`CipherFile::with_checked_pages`]).
enum RawPage<'a> {
    /// Its copy in the WAL.
    Wal(&'a [u8]),
    /// The `i`th page of a chunk of the main file.
    Chunk(Arc<Chunk>, usize),
}

impl RawPage<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Wal(bytes) => bytes,
            Self::Chunk(chunk, i) => chunk.page(*i),
        }
    }
}

/// Pages read from the main file at once ([`CHUNK_PAGES`], fewer at its end), still encrypted, and which of them
/// have had their HMAC checked: the bytes do not change once read, so each page is checked once while the chunk is
/// kept.
struct Chunk {
    bytes: Box<[u8]>,
    /// Bit `i` set once page `i` was checked.
    checked: AtomicU16,
}

impl Chunk {
    fn new(bytes: Box<[u8]>) -> Self {
        Self {
            bytes,
            checked: AtomicU16::new(0),
        }
    }

    /// The encrypted bytes of its `i`th page.
    fn page(&self, i: usize) -> &[u8] {
        &self.bytes[i * PAGE_SIZE..(i + 1) * PAGE_SIZE]
    }

    fn is_checked(&self, i: usize) -> bool {
        self.checked.load(Ordering::Relaxed) & (1 << i) != 0
    }

    /// Notes that the `i`th page's HMAC matched. Relaxed is enough: the bytes were in place before the chunk was
    /// shared, and the bit says nothing about other memory.
    fn mark_checked(&self, i: usize) {
        self.checked.fetch_or(1 << i, Ordering::Relaxed);
    }
}

/// A snapshot's WAL as it was when the snapshot was opened.
enum WalAtOpen {
    /// Open, and its size and time then.
    Held(File, Option<MainStamp>),
    /// There was none at this path.
    Missing(PathBuf),
}

/// The raw chunks of a file read last, at most [`CHUNKS_KEPT`]; `order` lists each chunk of `chunks` once, the
/// oldest first.
#[derive(Default)]
struct ChunkCache {
    chunks: HashMap<u32, Arc<Chunk>>,
    order: VecDeque<u32>,
}

impl ChunkCache {
    /// Keep chunk `idx`, dropping the oldest beyond [`CHUNKS_KEPT`]. Threads that missed the same chunk at once each
    /// read it and keep it: the later copy replaces the earlier one in its place.
    fn keep(&mut self, idx: u32, chunk: Arc<Chunk>) {
        if self.chunks.insert(idx, chunk).is_none() {
            self.order.push_back(idx);
        }
        while self.order.len() > CHUNKS_KEPT {
            if let Some(old) = self.order.pop_front() {
                self.chunks.remove(&old);
            }
        }
    }
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
            // The main file's stamp first, so that a change from here on shows (`ChangeGuard`, `unchanged`).
            let file =
                File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
            let stamp_at_open = main_stamp(&file);
            let len = stamp_at_open.map(|s| s.len).unwrap_or(0);
            // Order matters: wal-index first, then the WAL, so the WAL copy holds every frame the index counted. The
            // WAL is stamped before it is read, and kept open to be stamped again.
            let shm = File::open(&shm_path).ok();
            let at_open = shm.as_ref().and_then(|f| {
                let mut b = [0u8; 136];
                read_at(f, &mut b, 0)
                    .ok()
                    .and_then(|()| ShmState::parse(&b))
            });
            let wal_file = File::open(&wal_path).ok();
            let wal_stamp_at_open = wal_file.as_ref().and_then(main_stamp);
            let wal = wal_file.as_ref().and_then(|mut f| {
                let mut bytes = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut bytes)
                    .ok()
                    .map(|_| bytes)
            });
            let mut first = vec![0u8; PAGE_SIZE];
            read_at(&file, &mut first, 0)
                .with_context(|| format!("{} is smaller than one page", path.display()))?;
            let c = match &cipher {
                Some(c) => c.clone(),
                None => {
                    let c = sqlcipher::shared_cipher(&first, raw_key).with_context(|| {
                        format!(
                            "cannot decrypt {} with the configured database key",
                            path.display()
                        )
                    })?;
                    cipher = Some(c.clone());
                    c
                }
            };
            let overlay = sqlcipher::wal_overlay(wal.as_deref());
            let total_pages = sqlcipher::total_pages(len, overlay.db_pages);
            let mut wal_pages = HashMap::with_capacity(overlay.pages.len());
            if let Some(wal) = &wal {
                for (&pgno, &off) in &overlay.pages {
                    if pgno <= total_pages {
                        wal_pages
                            .insert(pgno, wal[off..off + PAGE_SIZE].to_vec().into_boxed_slice());
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
            let wal_stable = wal_header(&wal_path)
                == wal
                    .as_deref()
                    .map(|w| w.first_chunk::<WAL_HEADER>().copied());
            // A checkpoint running right now over frames we do not hold may be rewriting main-file pages: wait for it.
            // Anything else is fine to start from; the guard watches what happens next.
            let index_ok = match &at_open {
                Some(s) => {
                    guard.describes_our_wal(s) || s.attempted == s.backfilled || attempt >= 3
                }
                None => true,
            };
            if !wal_stable || !index_ok {
                last_err = Some(anyhow!(
                    "{} kept changing while it was being opened",
                    path.display()
                ));
                continue;
            }
            return Ok(Self {
                path: path.to_path_buf(),
                file,
                cipher: c,
                main_pages: u32::try_from(len / PAGE_SIZE as u64).with_context(|| {
                    format!("{} has more pages than SQLite can number", path.display())
                })?,
                total_pages,
                page1: page1.into_boxed_slice(),
                wal_pages,
                wal: match wal_file {
                    Some(file) => WalAtOpen::Held(file, wal_stamp_at_open),
                    None => WalAtOpen::Missing(wal_path),
                },
                chunks: Mutex::default(),
                guard,
                stale: AtomicBool::new(false),
                error: Mutex::new(None),
                pages_decrypted: AtomicU64::new(0),
            });
        }
        Err(last_err.unwrap_or_else(|| {
            anyhow!("{} kept changing while it was being opened", path.display())
        }))
    }

    /// Whether the main file and its WAL are as they were when the snapshot was opened, by their sizes and
    /// modification times: one that changed means WeChat wrote since, and a fresh snapshot shows it. They are asked
    /// through the handles the snapshot holds, which is a call that opens nothing (on Windows, opening a file to ask
    /// for its size costs tens of microseconds, and a snapshot is checked before every read); a WAL there was not
    /// then is looked for by its name.
    pub fn unchanged(&self) -> bool {
        main_stamp(&self.file) == self.guard.stamp_at_open
            && match &self.wal {
                WalAtOpen::Held(file, stamp) => main_stamp(file) == *stamp,
                WalAtOpen::Missing(path) => fs::metadata(path).is_err(),
            }
    }

    /// The keys that decrypt the file's pages, to open its next snapshot without deriving them again.
    pub fn cipher(&self) -> Arc<PageCipher> {
        self.cipher.clone()
    }

    /// Whether a read found that the file moved past this snapshot (the query should be run on a fresh one).
    pub fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Relaxed)
    }

    /// Whether the files moved since this snapshot was opened, judged more strictly than a read is: also when the
    /// main file's size or time changed under a wal-index that still looks right. Only for after an error that no
    /// read reported (a schema made of half-written pages decrypts fine and only SQLite finds it malformed), where a
    /// needless rerun is cheap; on every read it would rerun whenever WeChat checkpoints frames we already hold.
    pub fn changed_since_open(&self) -> bool {
        !self.guard.consistent(&self.file) || main_stamp(&self.file) != self.guard.stamp_at_open
    }

    /// The first read error, for reporting when retries do not help.
    pub fn read_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    /// Pages decrypted so far.
    #[cfg(test)]
    fn pages_decrypted(&self) -> u64 {
        self.pages_decrypted.load(Ordering::Relaxed)
    }

    fn fail(&self, why: String) -> c_int {
        self.stale.store(true, Ordering::Relaxed);
        if let Ok(mut e) = self.error.lock() {
            e.get_or_insert(why);
        }
        ffi::SQLITE_IOERR_READ
    }

    /// Raw (encrypted) chunk holding `pgno`, read from disk and checked against the guard when not cached, and the
    /// number of its first page.
    fn chunk(&self, pgno: u32) -> std::result::Result<(Arc<Chunk>, u32), c_int> {
        let idx = (pgno - 1) / CHUNK_PAGES;
        let first = idx * CHUNK_PAGES + 1;
        if let Some(c) = self
            .chunks
            .lock()
            .ok()
            .and_then(|c| c.chunks.get(&idx).cloned())
        {
            return Ok((c, first));
        }
        let count = CHUNK_PAGES.min(self.main_pages + 1 - first);
        let mut buf = vec![0u8; count as usize * PAGE_SIZE];
        if let Err(e) = read_at(&self.file, &mut buf, (first as u64 - 1) * PAGE_SIZE as u64) {
            return Err(self.fail(format!("cannot read {}: {e}", self.path.display())));
        }
        // The pages are in memory now; if no checkpoint moved past our snapshot until this moment, they are ours.
        if !self.guard.consistent_read(&self.file) {
            return Err(self.fail(format!(
                "{} changed while it was being read",
                self.path.display()
            )));
        }
        let chunk = Arc::new(Chunk::new(buf.into()));
        if let Ok(mut cache) = self.chunks.lock() {
            cache.keep(idx, chunk.clone());
        }
        Ok((chunk, first))
    }

    /// The encrypted bytes of page `pgno` as the snapshot holds it (its WAL copy, else from the main file, checked
    /// against the guard like any read), its HMAC checked, handed to `f` to decrypt what it needs; `None` for a page
    /// never written. A page that fails its HMAC fails the read as a torn one does (see [`page_into`](Self::page_into)).
    pub(crate) fn with_checked_page<T>(
        &self,
        pgno: u32,
        f: impl FnOnce(&[u8]) -> T,
    ) -> Result<Option<T>> {
        let failed = |e: anyhow::Error| {
            let why = format!("{}: {e}", self.path.display());
            self.fail(why.clone());
            anyhow!(why)
        };
        if let Some(enc) = self.wal_pages.get(&pgno) {
            if enc.iter().all(|b| *b == 0) {
                return Ok(None);
            }
            self.cipher.verify(pgno, enc).map_err(failed)?;
            self.pages_decrypted.fetch_add(1, Ordering::Relaxed);
            return Ok(Some(f(enc)));
        }
        if pgno == 0 || pgno > self.main_pages {
            return Ok(None);
        }
        let (chunk, first) = self
            .chunk(pgno)
            .map_err(|_| anyhow!(self.read_error().unwrap_or_else(|| "read failed".into())))?;
        let enc = chunk.page((pgno - first) as usize);
        if enc.iter().all(|b| *b == 0) {
            return Ok(None);
        }
        self.check_chunk_page(&chunk, first, pgno).map_err(failed)?;
        self.pages_decrypted.fetch_add(1, Ordering::Relaxed);
        Ok(Some(f(enc)))
    }

    /// Checks the HMAC of page `pgno`, a written one, in `chunk` (whose first page is page `first`), unless it was
    /// checked before. When a page next to it was checked already, the pages look read in order, as a table scan
    /// reads its leaves: the next pages of the chunk in that direction, up to four with this one, are checked
    /// together with it ([`PageCipher::verify_each`]), for the reads to come. Otherwise it is checked alone: a lookup
    /// that reads one page of the chunk would waste the others. The error is this page's; a neighbour that fails is
    /// checked again when it is read.
    fn check_chunk_page(&self, chunk: &Chunk, first: u32, pgno: u32) -> Result<()> {
        let at = |p: u32| (p - first) as usize;
        if chunk.is_checked(at(pgno)) {
            return Ok(());
        }
        // pages of the chunk: `first..last`, at most `CHUNK_PAGES`
        let last = first + CHUNK_PAGES.min(self.main_pages + 1 - first);
        let checked = |p: u32| (first..last).contains(&p) && chunk.is_checked(at(p));
        let group = if pgno > first && checked(pgno - 1) {
            pgno..(pgno + CHECKED_TOGETHER).min(last)
        } else if checked(pgno + 1) {
            (pgno + 1).saturating_sub(CHECKED_TOGETHER).max(first)..pgno + 1
        } else {
            pgno..pgno + 1
        };
        let pages: Vec<(u32, &[u8])> = group
            .filter(|&p| {
                p == pgno || (!chunk.is_checked(at(p)) && chunk.page(at(p)).iter().any(|b| *b != 0))
            })
            .map(|p| (p, chunk.page(at(p))))
            .collect();
        if pages.len() > 1 && self.cipher.verify_each(&pages).is_ok() {
            for (p, _) in &pages {
                chunk.mark_checked(at(*p));
            }
            return Ok(());
        }
        self.cipher.verify(pgno, chunk.page(at(pgno)))?;
        chunk.mark_checked(at(pgno));
        Ok(())
    }

    /// [`Self::with_checked_page`] for each of `pgnos` in turn, their HMACs checked together first (four at a time
    /// where the CPU can, see [`PageCipher::verify_each`]): `f` gets each page's encrypted bytes, `None` for a page
    /// never written, until it breaks. When a page fails its HMAC the pages are taken one by one instead, so `f` has
    /// had the pages before it, as with [`Self::with_checked_page`].
    pub(crate) fn with_checked_pages<B>(
        &self,
        pgnos: &[u32],
        mut f: impl FnMut(u32, Option<&[u8]>) -> ControlFlow<B>,
    ) -> Result<ControlFlow<B>> {
        let mut held: Vec<(u32, Option<RawPage<'_>>)> = Vec::with_capacity(pgnos.len());
        for &pgno in pgnos {
            let raw = if let Some(enc) = self.wal_pages.get(&pgno) {
                Some(RawPage::Wal(enc))
            } else if pgno == 0 || pgno > self.main_pages {
                None
            } else {
                let (chunk, first) = self.chunk(pgno).map_err(|_| {
                    anyhow!(self.read_error().unwrap_or_else(|| "read failed".into()))
                })?;
                Some(RawPage::Chunk(chunk, (pgno - first) as usize))
            };
            // a page that was never written is all zeros, and has no HMAC
            held.push((pgno, raw.filter(|r| r.bytes().iter().any(|b| *b != 0))));
        }
        let checked_before =
            |raw: &RawPage<'_>| matches!(raw, RawPage::Chunk(c, i) if c.is_checked(*i));
        let unchecked: Vec<(u32, &[u8])> = held
            .iter()
            .filter_map(|(pgno, raw)| raw.as_ref().map(|r| (*pgno, r)))
            .filter(|(_, raw)| !checked_before(raw))
            .map(|(pgno, raw)| (pgno, raw.bytes()))
            .collect();
        let together = self.cipher.verify_each(&unchecked).is_ok();
        let written = held.iter().filter(|(_, raw)| raw.is_some()).count();
        if together {
            self.pages_decrypted
                .fetch_add(written as u64, Ordering::Relaxed);
        }
        for (pgno, raw) in &held {
            if let Some(raw) = raw {
                if !together {
                    if !checked_before(raw) {
                        if let Err(e) = self.cipher.verify(*pgno, raw.bytes()) {
                            let why = format!("{}: {e}", self.path.display());
                            self.fail(why.clone());
                            return Err(anyhow!(why));
                        }
                    }
                    self.pages_decrypted.fetch_add(1, Ordering::Relaxed);
                }
                if let RawPage::Chunk(chunk, i) = raw {
                    chunk.mark_checked(*i);
                }
            }
            if let ControlFlow::Break(b) = f(*pgno, raw.as_ref().map(RawPage::bytes)) {
                return Ok(ControlFlow::Break(b));
            }
        }
        Ok(ControlFlow::Continue(()))
    }

    /// Page `pgno` decrypted and checked, as SQLite gets it.
    pub(crate) fn page(&self, pgno: u32, out: &mut [u8]) -> Result<()> {
        self.page_into(pgno, out)
            .map_err(|_| anyhow!(self.read_error().unwrap_or_else(|| "read failed".into())))
    }

    /// Pages of the database as the snapshot sees it.
    pub(crate) fn page_count(&self) -> u32 {
        self.total_pages
    }

    /// The keys of the file.
    pub(crate) fn page_cipher(&self) -> &PageCipher {
        &self.cipher
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
            let enc = chunk.page((pgno - first) as usize);
            if enc.iter().all(|b| *b == 0) {
                // never written: stored as zeros, and without an HMAC
                out.fill(0);
                Ok(())
            } else {
                self.check_chunk_page(&chunk, first, pgno)
                    .and_then(|()| self.cipher.decrypt_verified_page(pgno, enc, out))
            }
        };
        self.pages_decrypted.fetch_add(1, Ordering::Relaxed);
        // A torn page (written while we read it) fails its HMAC: treat it like any other change.
        decrypted.map_err(|e| self.fail(format!("{}: {e}", self.path.display())))
    }

    /// `xRead`: `out.len()` bytes at `offset` of the plain database.
    fn read(&self, out: &mut [u8], offset: u64) -> c_int {
        let mut done = 0usize;
        let mut scratch: Option<Vec<u8>> = None;
        while done < out.len() {
            let Some((pgno, in_page, n)) = page_part(
                offset.saturating_add(done as u64),
                out.len() - done,
                self.total_pages,
            ) else {
                out[done..].fill(0);
                return ffi::SQLITE_IOERR_SHORT_READ;
            };
            let result = if in_page == 0 && n == PAGE_SIZE {
                self.page_into(pgno, &mut out[done..done + PAGE_SIZE])
            } else {
                let buf = scratch.get_or_insert_with(|| vec![0u8; PAGE_SIZE]);
                self.page_into(pgno, buf)
                    .map(|()| out[done..done + n].copy_from_slice(&buf[in_page..in_page + n]))
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

/// The part of a read that one page holds: from byte `pos` of the file, with `remaining` bytes still to read, (its
/// page number, the offset in that page, how many bytes); `None` past the last of `total_pages` pages.
fn page_part(pos: u64, remaining: usize, total_pages: u32) -> Option<(u32, usize, usize)> {
    if pos >= u64::from(total_pages) * PAGE_SIZE as u64 {
        return None;
    }
    // below `total_pages`, a `u32`
    let pgno = u32::try_from(pos / PAGE_SIZE as u64).ok()? + 1;
    let in_page = usize::try_from(pos % PAGE_SIZE as u64).ok()?;
    Some((pgno, in_page, remaining.min(PAGE_SIZE - in_page)))
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
    registry()
        .lock()
        .map_err(|_| anyhow!("vfs registry poisoned"))?
        .insert(name.clone(), file.clone());
    let conn = Connection::open_with_flags_and_vfs(
        &name,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        VFS_NAME,
    );
    // The connection holds its own reference from `xOpen` on.
    registry()
        .lock()
        .map_err(|_| anyhow!("vfs registry poisoned"))?
        .remove(&name);
    let conn = conn.with_context(|| format!("cannot open {}", file.path.display()))?;
    Ok(conn)
}

#[repr(C)]
struct VfsFile {
    base: ffi::sqlite3_file,
    file: *const CipherFile,
}

// SQLite sees a `sqlite3_file *`: the base must come first (#505)
const _: () = assert!(std::mem::offset_of!(VfsFile, base) == 0);

fn default_vfs(vfs: *mut ffi::sqlite3_vfs) -> *mut ffi::sqlite3_vfs {
    // SAFETY: `pAppData` is set to the default VFS when ours is registered and never changes.
    unsafe { (*vfs).pAppData.cast() }
}

/// Our names are not paths; they never reach the default VFS.
///
/// # Safety
///
/// `name` is null or a NUL-terminated string that stays put while the result is used: SQLite's name of a file,
/// read during the call that gets it.
unsafe fn our_name(name: *const c_char) -> Option<&'static str> {
    if name.is_null() {
        return None;
    }
    // SAFETY: not null, and NUL-terminated as the caller promises
    unsafe { CStr::from_ptr(name) }
        .to_str()
        .ok()
        .filter(|n| n.starts_with(NAME_PREFIX))
}

fn register_vfs() -> Result<()> {
    static DONE: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    // SAFETY: the default VFS SQLite returns is checked for null and lives as long as the process; the new VFS is
    // leaked and names a `'static` string, so both outlive every connection; `DONE` registers it only once.
    DONE.get_or_init(|| unsafe {
        let default = ffi::sqlite3_vfs_find(std::ptr::null());
        if default.is_null() {
            return Err("SQLite has no default VFS".into());
        }
        let d = &*default;
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the file struct is a few dozen bytes"
        )]
        let vfs = Box::new(ffi::sqlite3_vfs {
            iVersion: 2,
            szOsFile: d.szOsFile.max(size_of::<VfsFile>() as c_int),
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
            rc => Err(format!(
                "cannot register the decrypting SQLite VFS: code {rc}"
            )),
        }
    })
    .clone()
    .map_err(|e| anyhow!(e))
}

unsafe extern "C" fn x_open(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    file: *mut ffi::sqlite3_file,
    flags: c_int,
    out_flags: *mut c_int,
) -> c_int {
    // SAFETY: SQLite hands `xOpen` a `sqlite3_file` of `szOsFile` bytes to fill
    unsafe { (*file).pMethods = std::ptr::null() };
    // SAFETY: SQLite's name of the file, or null
    if let Some(n) = unsafe { our_name(name) } {
        let found = if flags & ffi::SQLITE_OPEN_MAIN_DB != 0 {
            registry().lock().ok().and_then(|r| r.get(n).cloned())
        } else {
            None
        };
        let Some(cf) = found else {
            return ffi::SQLITE_CANTOPEN;
        };
        let f = file.cast::<VfsFile>();
        // SAFETY: `szOsFile` is at least a `VfsFile`, whose `sqlite3_file` comes first
        unsafe {
            (*f).file = Arc::into_raw(cf);
            (*f).base.pMethods = &IO_METHODS;
        }
        if !out_flags.is_null() {
            // SAFETY: SQLite's out-parameter, not null
            unsafe {
                *out_flags = (flags & !(ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE))
                    | ffi::SQLITE_OPEN_READONLY;
            }
        }
        return ffi::SQLITE_OK;
    }
    // Temporary files (sorting, materialised views) go to the OS VFS.
    let d = default_vfs(vfs);
    // SAFETY: the default VFS lives as long as the process; its `xOpen` gets SQLite's arguments as they came
    unsafe {
        match (*d).xOpen {
            Some(open) => open(d, name, file, flags, out_flags),
            None => ffi::SQLITE_CANTOPEN,
        }
    }
}

unsafe extern "C" fn x_delete(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    sync_dir: c_int,
) -> c_int {
    // SAFETY: SQLite's name of the file
    if unsafe { our_name(name) }.is_some() {
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    // SAFETY: the default VFS lives as long as the process; its `xDelete` gets SQLite's arguments as they came
    unsafe {
        (*d).xDelete
            .map_or(ffi::SQLITE_IOERR_DELETE, |f| f(d, name, sync_dir))
    }
}

unsafe extern "C" fn x_access(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    flags: c_int,
    out: *mut c_int,
) -> c_int {
    // SAFETY: SQLite's name of the file
    if unsafe { our_name(name) }.is_some() {
        // SAFETY: SQLite's out-parameter
        unsafe { *out = 0 }; // no journal, no WAL: a snapshot is a single immutable file
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    // SAFETY: the default VFS lives as long as the process; its `xAccess` gets SQLite's arguments as they came
    unsafe {
        (*d).xAccess
            .map_or(ffi::SQLITE_IOERR_ACCESS, |f| f(d, name, flags, out))
    }
}

unsafe extern "C" fn x_full_pathname(
    vfs: *mut ffi::sqlite3_vfs,
    name: *const c_char,
    n_out: c_int,
    out: *mut c_char,
) -> c_int {
    // SAFETY: SQLite's name of the file
    if let Some(n) = unsafe { our_name(name) } {
        let bytes = n.as_bytes();
        if bytes.len() + 1 > n_out.max(0) as usize {
            return ffi::SQLITE_CANTOPEN;
        }
        // SAFETY: `out` has room for `n_out` bytes (SQLite's buffer), and the name and its NUL fit
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, bytes.len());
            *out.add(bytes.len()) = 0;
        }
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    // SAFETY: the default VFS lives as long as the process; its `xFullPathname` gets SQLite's arguments as they came
    unsafe {
        (*d).xFullPathname
            .map_or(ffi::SQLITE_CANTOPEN, |f| f(d, name, n_out, out))
    }
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

/// The snapshot a file opened by [`x_open`] reads.
///
/// # Safety
///
/// `f` is a file [`x_open`] opened as one of ours and [`io_close`] has not closed: SQLite calls a file's methods only
/// between the two.
unsafe fn cipher_file<'a>(f: *mut ffi::sqlite3_file) -> &'a CipherFile {
    // SAFETY: `x_open` put a live `Arc<CipherFile>` there, which `io_close` gives back only after the last call
    unsafe { &*(*f.cast::<VfsFile>()).file }
}

unsafe extern "C" fn io_close(f: *mut ffi::sqlite3_file) -> c_int {
    let vf = f.cast::<VfsFile>();
    // SAFETY: SQLite closes a file `x_open` opened as ours, once; the `Arc` it holds is given back once and forgotten
    unsafe {
        if !(*vf).file.is_null() {
            drop(Arc::from_raw((*vf).file));
            (*vf).file = std::ptr::null();
        }
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_read(
    f: *mut ffi::sqlite3_file,
    buf: *mut c_void,
    amt: c_int,
    offset: ffi::sqlite3_int64,
) -> c_int {
    if amt < 0 || offset < 0 {
        return ffi::SQLITE_IOERR_READ;
    }
    // SAFETY: SQLite reads a file of ours, open
    let cf = unsafe { cipher_file(f) };
    // SAFETY: SQLite's buffer of `amt` bytes, not negative, for this call only
    let out = unsafe { std::slice::from_raw_parts_mut(buf.cast::<u8>(), amt as usize) };
    catch_unwind(AssertUnwindSafe(|| cf.read(out, offset as u64))).unwrap_or(ffi::SQLITE_IOERR_READ)
}

unsafe extern "C" fn io_write(
    _: *mut ffi::sqlite3_file,
    _: *const c_void,
    _: c_int,
    _: ffi::sqlite3_int64,
) -> c_int {
    ffi::SQLITE_READONLY
}

unsafe extern "C" fn io_truncate(_: *mut ffi::sqlite3_file, _: ffi::sqlite3_int64) -> c_int {
    ffi::SQLITE_READONLY
}

unsafe extern "C" fn io_sync(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_file_size(
    f: *mut ffi::sqlite3_file,
    size: *mut ffi::sqlite3_int64,
) -> c_int {
    // SAFETY: SQLite asks a file of ours, open, and passes its out-parameter
    unsafe { *size = cipher_file(f).size() as ffi::sqlite3_int64 };
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_lock(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_check_reserved_lock(_: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    // SAFETY: SQLite's out-parameter
    unsafe { *out = 0 };
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_file_control(_: *mut ffi::sqlite3_file, _: c_int, _: *mut c_void) -> c_int {
    ffi::SQLITE_NOTFOUND
}

#[expect(clippy::cast_possible_truncation, reason = "PAGE_SIZE is 4096")]
unsafe extern "C" fn io_sector_size(_: *mut ffi::sqlite3_file) -> c_int {
    PAGE_SIZE as c_int
}

unsafe extern "C" fn io_device_characteristics(_: *mut ffi::sqlite3_file) -> c_int {
    ffi::SQLITE_IOCAP_IMMUTABLE
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "test values are known to fit, or are meant to wrap"
)]
mod tests {
    use super::*;
    use crate::sqlcipher::testutil::{
        encrypt_db, encrypt_page, encrypt_wal, encrypted_wal_header, plain_db, wal_checksum_be,
        KEY, SALT,
    };

    fn temp_dir(tag: &str) -> crate::fixture::TestDir {
        crate::fixture::TestDir::new(&format!("weflow-vfs-{tag}"))
    }

    fn rows(conn: &Connection) -> Vec<String> {
        let mut stmt = conn.prepare("select v from t order by id").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
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
        let values: Vec<String> = (0..3000)
            .map(|i| format!("{i:05}{}", "x".repeat(200)))
            .collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let enc = encrypt_db(&plain_db(&refs), &cipher);
        let pages = (enc.len() / PAGE_SIZE) as u64;
        assert!(pages > 2 * CHUNK_PAGES as u64);
        let path = dir.join("a.db");
        fs::write(&path, &enc).unwrap();
        let (cf, conn) = open(&path);
        let one: String = conn
            .query_row("select v from t where id = 1234", [], |r| r.get(0))
            .unwrap();
        assert_eq!(one, values[1234]);
        assert!(
            cf.pages_decrypted() < 10,
            "a point lookup decrypted {} of {pages} pages",
            cf.pages_decrypted()
        );
        assert_eq!(rows(&conn), values);
        let full = sqlcipher::decrypt_database(&enc, None, &cipher).unwrap();
        let mut mem = Connection::open_in_memory().unwrap();
        mem.deserialize_read_exact(rusqlite::MAIN_DB, &full[..], full.len(), true)
            .unwrap();
        assert_eq!(rows(&mem), values);
        assert!(!cf.is_stale());
    }

    #[test]
    fn a_chunk_kept_twice_is_listed_once() {
        let chunk = |n: u8| Arc::new(Chunk::new(vec![n; 4].into()));
        let mut cache = ChunkCache::default();
        // two threads missed chunk 0 at once
        cache.keep(0, chunk(0));
        cache.keep(0, chunk(1));
        for idx in 1..CHUNKS_KEPT as u32 {
            cache.keep(idx, chunk(2));
        }
        assert_eq!(cache.chunks.len(), CHUNKS_KEPT);
        assert_eq!(cache.order.len(), CHUNKS_KEPT);
        assert_eq!(
            cache.chunks.get(&0).map(|c| c.bytes[0]),
            Some(1),
            "the later copy"
        );
        // one more pushes out the oldest, and only it
        cache.keep(CHUNKS_KEPT as u32, chunk(3));
        assert!(!cache.chunks.contains_key(&0));
        assert_eq!(
            (cache.chunks.len(), cache.order.len()),
            (CHUNKS_KEPT, CHUNKS_KEPT)
        );
        assert!(cache.order.iter().all(|idx| cache.chunks.contains_key(idx)));
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

    /// A WAL holding one committed transaction that rewrites page 1 of `plain` only (one frame).
    fn wal_page_1(cipher: &PageCipher, plain: &[u8], salt: (u32, u32)) -> Vec<u8> {
        let mut wal = encrypted_wal_header(salt);
        let first_page = encrypt_page(cipher, 1, &plain[..PAGE_SIZE], [7; 16]);
        let sum = (
            u32::from_be_bytes(wal[24..28].try_into().unwrap()),
            u32::from_be_bytes(wal[28..32].try_into().unwrap()),
        );
        let pages = (plain.len() / PAGE_SIZE) as u32;
        let mut fh = Vec::new();
        fh.extend(1u32.to_be_bytes());
        fh.extend(pages.to_be_bytes());
        fh.extend(salt.0.to_be_bytes());
        fh.extend(salt.1.to_be_bytes());
        let s = wal_checksum_be(&first_page, wal_checksum_be(&fh[..8], sum));
        fh.extend(s.0.to_be_bytes());
        fh.extend(s.1.to_be_bytes());
        wal.extend(fh);
        wal.extend(first_page);
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
        fs::write(
            sibling(&path, "-wal"),
            encrypt_wal(&cipher, &[], &plain_db(&["new"]), (1, 2)),
        )
        .unwrap();
        let (_, conn) = open(&path);
        assert_eq!(rows(&conn), ["new"]);
    }

    /// Many rows, so a scan reads the main file chunk by chunk after the open.
    fn big(tag: &str) -> Vec<u8> {
        let values: Vec<String> = (0..2000)
            .map(|i| format!("{tag}{i:05}{}", "y".repeat(300)))
            .collect();
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
        let wal = wal_page_1(&cipher, &plain, (1, 2));
        fs::write(sibling(&path, "-wal"), &wal).unwrap();
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 1)).unwrap();

        // a checkpoint within our frames is fine
        let (cf, conn) = open(&path);
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());

        // a checkpoint of frames we do not hold (written = 5 > frames = 1): the next main-file read fails
        let (cf, conn) = open(&path);
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 5)).unwrap();
        assert!(conn
            .prepare("select v from t order by id")
            .and_then(|mut s| s
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>())
            .is_err());
        assert!(cf.is_stale());

        // a new WAL generation (salt changed): stale as well
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 1)).unwrap();
        let (cf, conn) = open(&path);
        fs::write(sibling(&path, "-shm"), shm((2, 9), 0, 0)).unwrap();
        assert!(conn
            .query_row("select count(v) from t", [], |r| r.get::<_, i64>(0))
            .is_err());
        assert!(cf.is_stale());
    }

    #[test]
    fn a_main_file_rewritten_under_a_matching_index_is_noticed_after_an_error() {
        // a read cannot tell: the wal-index still describes our WAL and no checkpoint passed our frame
        let dir = temp_dir("rewritten");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let plain = big("e");
        let mut enc = encrypt_db(&plain, &cipher);
        fs::write(&path, &enc).unwrap();
        fs::write(sibling(&path, "-wal"), wal_page_1(&cipher, &plain, (1, 2))).unwrap();
        fs::write(sibling(&path, "-shm"), shm((1, 2), 0, 0)).unwrap();
        let (cf, conn) = open(&path);
        assert!(!cf.changed_since_open());
        enc.extend(vec![0u8; PAGE_SIZE]);
        fs::write(&path, &enc).unwrap();
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());
        // but when SQLite finds the pages it read malformed, the caller can tell the file moved and start over
        assert!(cf.changed_since_open());
    }

    #[test]
    fn a_snapshot_opened_without_an_index_reads_while_the_main_file_stays_as_it_was() {
        let dir = temp_dir("no-index");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let mut enc = encrypt_db(&big("d"), &cipher);
        fs::write(&path, &enc).unwrap();
        let (cf, conn) = open(&path);
        // WeChat starting makes a wal-index of its own; the main file, the snapshot, is as it was
        fs::write(sibling(&path, "-shm"), shm((5, 6), 0, 0)).unwrap();
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());
        // once the main file changes (a checkpoint), a read from it fails
        fs::remove_file(sibling(&path, "-shm")).unwrap();
        let (cf, conn) = open(&path);
        enc.extend(vec![0u8; PAGE_SIZE]);
        fs::write(&path, &enc).unwrap();
        assert!(conn
            .query_row("select count(v) from t", [], |r| r.get::<_, i64>(0))
            .is_err());
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
        assert!(conn
            .query_row("select count(v) from t", [], |r| r.get::<_, i64>(0))
            .is_err());
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
        assert!(conn
            .query_row("select count(v) from t", [], |r| r.get::<_, i64>(0))
            .is_err());
        assert!(cf.is_stale());
        assert!(
            cf.read_error().unwrap().contains("HMAC"),
            "{:?}",
            cf.read_error()
        );
    }

    #[test]
    fn pages_read_in_order_are_checked_a_few_at_once_and_each_still_fails_alone() {
        let dir = temp_dir("in-order");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let clean = encrypt_db(&big("f"), &cipher);
        assert!(clean.len() / PAGE_SIZE > 64);
        // page 21, the fifth of the second chunk (17..=32), damaged
        let bad = 21u32;
        let mut enc = clean.clone();
        enc[(bad as usize - 1) * PAGE_SIZE + 100] ^= 0xff;
        fs::write(&path, &enc).unwrap();
        let cf = CipherFile::open(&path, &KEY, None).unwrap();
        let mut out = vec![0u8; PAGE_SIZE];
        let mut want = vec![0u8; PAGE_SIZE];
        let mut read = |pgno: u32| {
            let at = (pgno as usize - 1) * PAGE_SIZE;
            cipher
                .decrypt_page(pgno, &clean[at..at + PAGE_SIZE], &mut want)
                .unwrap();
            cf.page(pgno, &mut out)
                .map(|()| assert_eq!(out, want, "page {pgno}"))
        };
        let checked = |pgno: u32| {
            let (chunk, first) = cf.chunk(pgno).unwrap();
            chunk.is_checked((pgno - first) as usize)
        };

        // in order: the first of a chunk alone, then four at once
        for pgno in 2..=8 {
            read(pgno).unwrap();
        }
        assert!((2..=10).all(checked), "2 alone, then 3..=6, 7..=10");
        assert!(!checked(11));
        // up to the damaged page: the groups that hold it fail, so the pages before it are checked one by one
        for pgno in 9..bad {
            read(pgno).unwrap();
        }
        assert!(!checked(bad) && !checked(bad + 1));
        let err = read(bad).unwrap_err().to_string();
        assert!(err.contains(&format!("page {bad}")), "{err}");
        assert!(cf.is_stale(), "as any torn page");
        // the pages after it read as before
        for pgno in bad + 1..=bad + 4 {
            read(pgno).unwrap();
        }

        // a lookup reads one page of a chunk: only it is checked
        read(40).unwrap();
        assert!(checked(40) && !checked(39) && !checked(41));
        // backwards from there: four at once, below it
        read(39).unwrap();
        assert!((36..=40).all(checked) && !checked(35));
    }

    #[test]
    fn a_wal_header_is_read_without_the_rest() {
        let dir = temp_dir("wal-header");
        let path = dir.join("a.db-wal");
        assert_eq!(wal_header(&path), None, "no file");
        fs::write(&path, [1u8; 10]).unwrap();
        assert_eq!(wal_header(&path), Some(None), "shorter than a header");
        let wal: Vec<u8> = (0..100).collect();
        fs::write(&path, &wal).unwrap();
        assert_eq!(
            wal_header(&path),
            Some(wal.first_chunk::<WAL_HEADER>().copied())
        );
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
        let n: i64 = conn
            .query_row(
                "select count(*) from (select v from t order by substr(v, 3) desc)",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(n, 2000);
        assert!(
            conn.execute("insert into t values (99999, 'x')", [])
                .is_err(),
            "snapshots are read-only"
        );
    }
}

/// Proofs of `cargo kani` (#501).
#[cfg(kani)]
mod proofs {
    use super::*;

    /// Any read position gives a part of one page inside the file, of at least a byte and no more than asked, at
    /// the very position asked; or nothing past the end.
    #[kani::proof]
    fn any_read_position_is_inside_one_page() {
        let pos: u64 = kani::any();
        let remaining: usize = kani::any_where(|&n: &usize| n > 0);
        let total_pages: u32 = kani::any();
        match page_part(pos, remaining, total_pages) {
            Some((pgno, in_page, n)) => {
                assert!(pgno >= 1 && pgno <= total_pages);
                assert!(
                    in_page < PAGE_SIZE && n >= 1 && n <= remaining && in_page + n <= PAGE_SIZE
                );
                assert!(u64::from(pgno - 1) * PAGE_SIZE as u64 + in_page as u64 == pos);
            }
            None => assert!(pos >= u64::from(total_pages) * PAGE_SIZE as u64),
        }
    }
}
