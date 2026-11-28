//! 按需解密微信 SQLCipher 页的只读 SQLite VFS。
//!
//! 数据库的快照是一个普通的 SQLite 连接，它的主文件由这个 VFS 提供：SQLite 请求某一页时，才从磁盘读取这一页、校验
//! HMAC 并解密页内容，查询用到的页由 SQLite 自己的页缓存保留。因此只访问 200 MB 的消息库中几页的查询只读这几页，
//! 不读整个文件，内存占用取决于页缓存，而不是数据库的大小。
//!
//! 打开快照时复制 WAL 中最新的已提交帧，所以一个连接看到的是数据库的同一个状态。但读取期间微信仍在写入：checkpoint
//! 可能把比快照的副本更新的 WAL 帧写进主文件，此后读到的页会混合两个状态。wal-index（`-shm`，SQLite 的明文格式）
//! 记录 checkpoint 进行到了哪里。每次从主文件读取后都做一次检查（`ChangeGuard`）：读到的页可能比快照新时，
//! 这次读取失败，并把快照标记为过期，[`crate::db`] 随后打开新的快照，重新执行查询。

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
use crate::shared_file::SharedFile;
use crate::sqlcipher::{self, PageCipher, PAGE_SIZE};

const VFS_NAME: &CStr = c"weflow-cipher";
/// 快照在 VFS 中登记的名字的前缀（这些名字不是路径）。
const NAME_PREFIX: &str = "weflow-cipher:";
/// 一次从主文件读取的页数（原始的、仍是加密的数据）；全表扫描时读盘和检查的次数减少到 1/16。
const CHUNK_PAGES: u32 = 16;
/// 每个打开的文件保留的原始块数（`CHUNKS_KEPT * CHUNK_PAGES * 4 KiB` = 2 MiB）。
const CHUNKS_KEPT: usize = 32;
/// 按顺序读取一个块中的页时，一次校验 HMAC 的页数（[`CipherFile::check_chunk_page`]），
/// 与 [`sha512x4`] 一次计算的哈希数相同。
#[expect(clippy::cast_possible_truncation, reason = "值为 4")]
const CHECKED_TOGETHER: u32 = sha512x4::LANES as u32;

/// 按位置读取：从 `offset` 处读满 `buf.len()` 个字节。
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

/// WAL 头的字节数。
const WAL_HEADER: usize = 32;

/// 另外打开 `path` 处的 WAL，读出它的头：无法读取时为 `None`，文件比一个头还短时为 `Some(None)`。
fn wal_header(path: &Path) -> Option<Option<[u8; WAL_HEADER]>> {
    use std::io::Read;
    let mut header = [0u8; WAL_HEADER];
    match File::open(path).ok()?.read_exact(&mut header) {
        Ok(()) => Some(Some(header)),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Some(None),
        Err(_) => None,
    }
}

// ───────────────────────────── 变化检查 ─────────────────────────────

/// wal-index 中与 checkpoint 有关的内容：WAL 的代次（盐），以及已经或正在写回主文件的帧数。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct ShmState {
    salt: [u8; 8],
    backfilled: u32,
    attempted: u32,
}

impl ShmState {
    /// 解析 `-shm` 文件的前 136 字节（48 字节的头的两份副本，之后是 checkpoint 的信息）。
    /// 头还没初始化或正在改写（两份副本不一致）时为 `None`。
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

    /// checkpoint 可能已经写进主文件的帧数。
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
    held(file).map(|h| h.stamp)
}

/// 快照一直打开着的文件的 [`main_stamp`]，文件被删除后为 `None`（#1153）。SQLite 在最后一个连接关闭时删除 WAL 和
/// wal-index，下一个连接打开时在同一路径新建，而句柄看到的仍是已删除的文件，它的大小和时间不会再变。
fn held_stamp(file: &File) -> Option<MainStamp> {
    held(file).filter(|h| !h.deleted).map(|h| h.stamp)
}

/// 从文件句柄查到的情况：大小和时间，以及文件是否已被删除。
struct Held {
    stamp: MainStamp,
    /// 文件已经没有任何名字（链接数为 0），或者在 Windows 上等句柄全部关闭后就会被删除。
    deleted: bool,
}

/// 从元数据得到 `file` 所打开的文件的 [`Held`]。
#[cfg(not(windows))]
fn held(file: &File) -> Option<Held> {
    let m = file.metadata().ok()?;
    #[cfg(unix)]
    let deleted = std::os::unix::fs::MetadataExt::nlink(&m) == 0;
    // WeFlow 只在 Unix 和 Windows 上运行，其他系统上只求能编译，不判断是否已删除
    #[cfg(not(unix))]
    let deleted = false;
    Some(Held {
        stamp: MainStamp {
            len: m.len(),
            mtime: m.modified().ok(),
        },
        deleted,
    })
}

/// 用句柄的两次查询得到 `file` 所打开的文件的 [`Held`]：一次查时间，一次查大小和链接数。`File::metadata`
/// 另外还查询卷的序列号，这个请求要交给文件系统处理；而数据库的大小和时间在每读一个块之后都要查询（#1105）。
#[cfg(windows)]
fn held(file: &File) -> Option<Held> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FileBasicInfo, FileStandardInfo, GetFileInformationByHandleEx, FILE_BASIC_INFO,
        FILE_STANDARD_INFO,
    };
    #[expect(
        clippy::cast_possible_truncation,
        reason = "structures of 40 and 24 bytes"
    )]
    let (basic_size, standard_size) = (
        size_of::<FILE_BASIC_INFO>() as u32,
        size_of::<FILE_STANDARD_INFO>() as u32,
    );
    let mut basic = FILE_BASIC_INFO::default();
    let mut standard = FILE_STANDARD_INFO::default();
    let handle = file.as_raw_handle();
    // SAFETY: 借用 `file` 期间句柄一直打开；每个缓冲区都是可写的结构体，大小与传入的一致，
    // 类型也正是对应的信息类别要填写的结构体
    let ok = unsafe {
        GetFileInformationByHandleEx(handle, FileBasicInfo, (&raw mut basic).cast(), basic_size)
            != 0
            && GetFileInformationByHandleEx(
                handle,
                FileStandardInfo,
                (&raw mut standard).cast(),
                standard_size,
            ) != 0
    };
    if !ok {
        return None;
    }
    Some(Held {
        stamp: MainStamp {
            len: u64::try_from(standard.EndOfFile).ok()?,
            mtime: file_time(basic.LastWriteTime),
        },
        deleted: standard.DeletePending || standard.NumberOfLinks == 0,
    })
}

/// 把 Windows 的文件时间（自 1601 年起的 100 纳秒间隔数）转换成 `SystemTime`；
/// 值为 0 表示文件系统不记录这个时间，此时为 `None`。
#[cfg(windows)]
fn file_time(intervals: i64) -> Option<SystemTime> {
    /// 1970-01-01 距 1601-01-01 的 100 纳秒间隔数。
    const UNIX_EPOCH_INTERVALS: i64 = 116_444_736_000_000_000;
    if intervals == 0 {
        return None;
    }
    let since = intervals.checked_sub(UNIX_EPOCH_INTERVALS)?;
    let span = std::time::Duration::from_nanos(since.unsigned_abs().checked_mul(100)?);
    if since >= 0 {
        SystemTime::UNIX_EPOCH.checked_add(span)
    } else {
        SystemTime::UNIX_EPOCH.checked_sub(span)
    }
}

/// 判断从主文件读到的页是否仍属于快照。
///
/// checkpoint 在写入任何页之前设置 `nBackfillAttempted`，写完之后设置 `nBackfill`；WAL 每换一代，盐也随之改变。
/// 因此从主文件读取之后，只要 wal-index 中仍是快照所持 WAL 的盐，且没有 checkpoint 越过快照复制的帧，
/// 读到的页就属于快照的状态。wal-index 描述的不是快照的 WAL 时（微信退出后遗留的索引，或者根本没有 WAL），
/// 退而要求「打开快照以来什么都没有变」。
struct ChangeGuard {
    shm_path: PathBuf,
    shm: Mutex<Option<File>>,
    /// 快照所持 WAL 副本的盐和已提交的帧数。
    wal_salt: Option<[u8; 8]>,
    frames: u32,
    at_open: Option<ShmState>,
    stamp_at_open: Option<MainStamp>,
}

impl ChangeGuard {
    fn read_shm(&self) -> Option<ShmState> {
        let mut slot = self.shm.lock().ok()?;
        // 句柄打开期间被删除的 wal-index 已经不是微信在更新的那一个（#1153）
        if slot
            .as_ref()
            .is_none_or(|file| held(file).is_some_and(|h| h.deleted))
        {
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

    /// 精确的规则：wal-index 描述的是快照的 WAL，且没有 checkpoint 越过快照的帧。
    fn describes_our_wal(&self, now: &ShmState) -> bool {
        self.wal_salt == Some(now.salt) && now.written() <= self.frames
    }

    /// 刚从主文件读到的页是否属于快照的状态。
    fn consistent(&self, main: &File) -> bool {
        match self.read_shm() {
            Some(now) => {
                if self.describes_our_wal(&now) {
                    return true;
                }
                // 不是快照的那一代 WAL：只有打开以来什么都没变（且没有正在进行的 checkpoint）时才安全。
                self.at_open == Some(now)
                    && now.attempted == now.backfilled
                    && main_stamp(main) == self.stamp_at_open
            }
            None => self.at_open.is_none() && main_stamp(main) == self.stamp_at_open,
        }
    }

    /// 读取时用的 [`Self::consistent`]，每从主文件读一个块都要运行。打开快照时没有 wal-index 的，
    /// 快照就是当时的主文件（加上打开时复制的 WAL）：只要文件的大小和时间没变，从中读到的页就属于快照，不去找
    /// wal-index。查找不存在的文件在每次读取时都是一次系统调用，在 Windows 上要花几十微秒。
    fn consistent_read(&self, main: &File) -> bool {
        (self.at_open.is_none() && main_stamp(main) == self.stamp_at_open) || self.consistent(main)
    }
}

// ───────────────────────────── 一个快照 ─────────────────────────────

/// 加密数据库文件的一个一致的只读视图，逐页解密。
pub struct CipherFile {
    path: PathBuf,
    /// 主文件，由快照所有连接的线程共同读取。
    file: SharedFile,
    cipher: Arc<PageCipher>,
    main_pages: u32,
    total_pages: u32,
    /// 解密并修补过的第 1 页（见 [`sqlcipher::patch_header`]）。
    page1: Box<[u8]>,
    /// 每页在 WAL 中最新的已提交帧的加密副本（打开时复制）。
    wal_pages: HashMap<u32, Box<[u8]>>,
    /// 打开时的 WAL，供 [`Self::unchanged`] 使用。
    wal: WalAtOpen,
    chunks: Mutex<ChunkCache>,
    guard: ChangeGuard,
    stale: AtomicBool,
    error: Mutex<Option<String>>,
    pages_decrypted: AtomicU64,
}

/// 只显示文件和页数：页是解密后的数据，`cipher` 含有密钥。
impl std::fmt::Debug for CipherFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CipherFile")
            .field("path", &self.path)
            .field("main_pages", &self.main_pages)
            .field("total_pages", &self.total_pages)
            .field("wal_pages", &self.wal_pages.len())
            .field("stale", &self.stale.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

/// 快照所持的一页的加密字节（[`CipherFile::with_checked_pages`]）。
enum RawPage<'a> {
    /// 这一页在 WAL 中的副本。
    Wal(&'a [u8]),
    /// 主文件某个块中的第 `i` 页。
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

/// 一次从主文件读取的、仍是加密的页（[`CHUNK_PAGES`] 页，文件末尾的块可能更少），以及其中哪些页已经校验过
/// HMAC：字节读入后不再变化，所以块保留期间每页只校验一次。
struct Chunk {
    bytes: Box<[u8]>,
    /// 第 `i` 页校验过后置位第 `i` 位。
    checked: AtomicU16,
}

impl Chunk {
    fn new(bytes: Box<[u8]>) -> Self {
        Self {
            bytes,
            checked: AtomicU16::new(0),
        }
    }

    /// 块中第 `i` 页的加密字节。
    fn page(&self, i: usize) -> &[u8] {
        &self.bytes[i * PAGE_SIZE..(i + 1) * PAGE_SIZE]
    }

    fn is_checked(&self, i: usize) -> bool {
        self.checked.load(Ordering::Relaxed) & (1 << i) != 0
    }

    /// 记下第 `i` 页的 HMAC 已经匹配。用 `Relaxed` 就够了：块被共享之前字节已经就位，这一位也不涉及其他内存。
    fn mark_checked(&self, i: usize) {
        self.checked.fetch_or(1 << i, Ordering::Relaxed);
    }
}

/// 快照打开时它的 WAL 的状态。
enum WalAtOpen {
    /// 已打开，带有打开时的大小和时间。
    Held(SharedFile, Option<MainStamp>),
    /// 这个路径上当时没有 WAL。
    Missing(PathBuf),
}

/// 一个文件最近读取的原始块，最多 [`CHUNKS_KEPT`] 个；`order` 按从旧到新的顺序列出 `chunks` 中的每个块，各一次。
#[derive(Default)]
struct ChunkCache {
    chunks: HashMap<u32, Arc<Chunk>>,
    order: VecDeque<u32>,
}

impl ChunkCache {
    /// 保留第 `idx` 个块，超过 [`CHUNKS_KEPT`] 个时丢弃最旧的。几个线程同时未命中同一个块时，各自读取并保留它：
    /// 后读到的副本替换先前的副本，在淘汰顺序中的位置不变。
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
    /// 打开 `path`（连同它的 `-wal`）供按需读取。调用方已经为这个文件派生过密钥（盐相同）时传入 `cipher` 复用，
    /// 省去较慢的密钥派生。
    ///
    /// # Errors
    ///
    /// 主文件无法读取或不到一页、`raw_key` 解不开第 1 页、页数超出 SQLite 能编号的范围，
    /// 或者 5 次尝试中文件一直在变化（WAL 被写入或重置、checkpoint 正在进行）时返回错误。
    pub fn open(path: &Path, raw_key: &[u8; 32], cipher: Option<Arc<PageCipher>>) -> Result<Self> {
        let wal_path = sibling(path, "-wal");
        let shm_path = sibling(path, "-shm");
        let mut cipher = cipher;
        let mut last_err = None;
        for attempt in 0..5 {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(20 * attempt));
            }
            // 先取主文件的文件戳，此后的变化都能被发现（`ChangeGuard`、`unchanged`）。
            let file =
                File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
            let stamp_at_open = main_stamp(&file);
            let len = stamp_at_open.map(|s| s.len).unwrap_or(0);
            // 顺序不能变：先读 wal-index，再读 WAL，这样 WAL 的副本含有索引统计到的每一帧。
            // WAL 在读取之前先取文件戳，并保持打开，以便之后再取一次。
            let shm = File::open(&shm_path).ok();
            let at_open = shm.as_ref().and_then(|f| {
                let mut b = [0u8; 136];
                read_at(f, &mut b, 0)
                    .ok()
                    .and_then(|()| ShmState::parse(&b))
            });
            let wal_file = File::open(&wal_path).ok();
            let wal_stamp_at_open = wal_file.as_ref().and_then(held_stamp);
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
                last_err = Some(e); // 复制 WAL 时它正在被写入
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
            // 读索引和读 WAL 之间 WAL 被重置的话，WAL 头里的盐会不同。
            let wal_stable = wal_header(&wal_path)
                == wal
                    .as_deref()
                    .map(|w| w.first_chunk::<WAL_HEADER>().copied());
            // 正在处理快照不持有的帧的 checkpoint 可能在改写主文件的页：等它完成（第 4 次尝试起不再等）。
            // 其他情况都可以从这里开始，之后的变化由 `ChangeGuard` 监视。
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
                file: SharedFile::new(file),
                cipher: c,
                main_pages: u32::try_from(len / PAGE_SIZE as u64).with_context(|| {
                    format!("{} has more pages than SQLite can number", path.display())
                })?,
                total_pages,
                page1: page1.into_boxed_slice(),
                wal_pages,
                wal: match wal_file {
                    Some(file) => WalAtOpen::Held(SharedFile::new(file), wal_stamp_at_open),
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

    /// 按大小和修改时间判断主文件和它的 WAL 是否与快照打开时相同：有变化说明微信此后写过，新的快照能看到这些写入。
    /// 大小和时间通过快照持有的句柄查询，不打开任何文件（在 Windows 上，为查询大小打开一个文件要花几十微秒，
    /// 而每次读取之前都要检查快照）；打开时不存在的 WAL 按名字查找。此后被删除的 WAL 算作有变化，
    /// 因为它的路径上可能已经有了新的 WAL（#1153）。
    pub fn unchanged(&self) -> bool {
        self.file.with(main_stamp) == self.guard.stamp_at_open
            && match &self.wal {
                WalAtOpen::Held(file, stamp) => stamp.is_some() && file.with(held_stamp) == *stamp,
                WalAtOpen::Missing(path) => fs::metadata(path).is_err(),
            }
    }

    /// 解密这个文件的页所用的密钥，供打开下一个快照时使用，不必重新派生。
    pub fn cipher(&self) -> Arc<PageCipher> {
        self.cipher.clone()
    }

    /// 是否有读取发现文件已经比这个快照新（查询应该在新的快照上重新执行）。
    pub fn is_stale(&self) -> bool {
        self.stale.load(Ordering::Relaxed)
    }

    /// 快照打开以来文件是否变化，判断比读取时更严格：wal-index 看起来仍然正确、但主文件的大小或时间变了，也算变化。
    /// 只用于读取没有报告的错误之后（由半写入的页组成的 schema 能正常解密，只有 SQLite 才发现它格式错误），
    /// 这时多重新执行一次代价很小；如果每次读取都这样判断，微信每次对快照已持有的帧做 checkpoint 都会造成重新执行。
    pub fn changed_since_open(&self) -> bool {
        self.file.with(|file| {
            !self.guard.consistent(file) || main_stamp(file) != self.guard.stamp_at_open
        })
    }

    /// 第一个读取错误，重试也无济于事时用来报告。
    pub fn read_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    /// 到目前为止解密过的页数。
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

    /// 含有第 `pgno` 页的原始（加密的）块，以及块中第一页的页号；
    /// 块不在缓存中时从磁盘读取，并用 `ChangeGuard` 检查。
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
        let at = (u64::from(first) - 1) * PAGE_SIZE as u64;
        // 页一经读入就在内存中；只要到读完的那一刻还没有 checkpoint 越过快照，它们就属于快照。
        let read = self
            .file
            .with(|file| read_at(file, &mut buf, at).map(|()| self.guard.consistent_read(file)));
        let consistent = match read {
            Ok(consistent) => consistent,
            Err(e) => return Err(self.fail(format!("cannot read {}: {e}", self.path.display()))),
        };
        if !consistent {
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

    /// 校验快照所持的第 `pgno` 页的 HMAC，再把它的加密字节交给 `f` 解密所需的部分；从未写入的页为 `None`。
    /// 快照所持的页是它在 WAL 中的副本，没有时从主文件读取（与任何读取一样用 `ChangeGuard` 检查）。HMAC
    /// 校验失败的页与撕裂的页一样使读取失败（见 [`page_into`](Self::page_into)）。
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

    /// 校验 `chunk`（第一页是第 `first` 页）中已写入的第 `pgno` 页的 HMAC，之前校验过的不再校验。
    /// 相邻的页已经校验过时，说明页是按顺序读取的（全表扫描读叶子页就是这样）：
    /// 块中沿这个方向接下来的几页（连同这一页最多四页）与它一起校验（[`PageCipher::verify_each`]），
    /// 供接下来的读取使用。否则只校验这一页：只读块中一页的查找用不到其他页，一起校验是浪费。
    /// 返回的错误是这一页的；相邻的页校验失败时，等读到它时再校验一次。
    fn check_chunk_page(&self, chunk: &Chunk, first: u32, pgno: u32) -> Result<()> {
        let at = |p: u32| (p - first) as usize;
        if chunk.is_checked(at(pgno)) {
            return Ok(());
        }
        // 块中的页：`first..last`，最多 `CHUNK_PAGES` 页
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

    /// 依次对 `pgnos` 中的每一页做 [`Self::with_checked_page`]，但先一起校验它们的 HMAC（CPU 支持时一次四页，
    /// 见 [`PageCipher::verify_each`]）：`f` 依次得到每页的加密字节，从未写入的页为 `None`，直到 `f` 返回
    /// `ControlFlow::Break`。有一页的 HMAC 校验失败时改为逐页处理，这样 `f` 已经得到了它之前的页，与
    /// [`Self::with_checked_page`] 一致。
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
            // 从未写入的页全部为 0，也没有 HMAC
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

    /// 解密并校验过的第 `pgno` 页，与 SQLite 得到的相同。
    pub(crate) fn page(&self, pgno: u32, out: &mut [u8]) -> Result<()> {
        self.page_into(pgno, out)
            .map_err(|_| anyhow!(self.read_error().unwrap_or_else(|| "read failed".into())))
    }

    /// 快照所见的数据库页数。
    pub(crate) fn page_count(&self) -> u32 {
        self.total_pages
    }

    /// 这个文件的密钥。
    pub(crate) fn page_cipher(&self) -> &PageCipher {
        &self.cipher
    }

    /// 把解密后的第 `pgno` 页写进 `out`（`PAGE_SIZE` 字节）。
    fn page_into(&self, pgno: u32, out: &mut [u8]) -> std::result::Result<(), c_int> {
        if pgno == 1 {
            out.copy_from_slice(&self.page1);
            return Ok(());
        }
        let decrypted = if let Some(enc) = self.wal_pages.get(&pgno) {
            self.cipher.decrypt_page(pgno, enc, out)
        } else if pgno > self.main_pages {
            out.fill(0); // 超出主文件，也不在 WAL 中：从未写入
            return Ok(());
        } else {
            let (chunk, first) = self.chunk(pgno)?;
            let enc = chunk.page((pgno - first) as usize);
            if enc.iter().all(|b| *b == 0) {
                // 从未写入：保存为全 0，也没有 HMAC
                out.fill(0);
                Ok(())
            } else {
                self.check_chunk_page(&chunk, first, pgno)
                    .and_then(|()| self.cipher.decrypt_verified_page(pgno, enc, out))
            }
        };
        self.pages_decrypted.fetch_add(1, Ordering::Relaxed);
        // 撕裂的页（读取时正在被写入）HMAC 校验会失败：与其他变化一样处理。
        decrypted.map_err(|e| self.fail(format!("{}: {e}", self.path.display())))
    }

    /// `xRead`：明文数据库中从 `offset` 开始的 `out.len()` 个字节。
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
        u64::from(self.total_pages) * PAGE_SIZE as u64
    }
}

/// 一次读取中落在同一页里的部分：从文件的第 `pos` 字节起、还剩 `remaining` 字节要读时，返回（页号，页内偏移，
/// 字节数）；超出 `total_pages` 页的末尾时为 `None`。
fn page_part(pos: u64, remaining: usize, total_pages: u32) -> Option<(u32, usize, usize)> {
    if pos >= u64::from(total_pages) * PAGE_SIZE as u64 {
        return None;
    }
    // 小于 `total_pages`，所以在 `u32` 的范围内
    let pgno = u32::try_from(pos / PAGE_SIZE as u64).ok()? + 1;
    let in_page = usize::try_from(pos % PAGE_SIZE as u64).ok()?;
    Some((pgno, in_page, remaining.min(PAGE_SIZE - in_page)))
}

// ───────────────────────────── 与 SQLite 的衔接 ─────────────────────────────

fn registry() -> &'static Mutex<HashMap<String, Arc<CipherFile>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<CipherFile>>>> = OnceLock::new();
    REGISTRY.get_or_init(Mutex::default)
}

/// 在 `file` 上打开一个只读的 SQLite 连接。
///
/// # Errors
///
/// VFS 注册失败，或者 SQLite 打不开连接时返回错误。
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
    // 连接从 `xOpen` 起持有自己的引用，登记表中的这一项不再需要。
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

// SQLite 把它当作 `sqlite3_file *`：`base` 必须在最前面（#505）
const _: () = assert!(std::mem::offset_of!(VfsFile, base) == 0);

fn default_vfs(vfs: *mut ffi::sqlite3_vfs) -> *mut ffi::sqlite3_vfs {
    // SAFETY: 注册我们的 VFS 时 `pAppData` 设为默认 VFS，此后不再改变。
    unsafe { (*vfs).pAppData.cast() }
}

/// `name` 是我们登记的名字时返回它。这些名字不是路径，不会交给默认 VFS。
///
/// # Safety
///
/// `name` 为空指针，或者指向以 NUL 结尾、在使用结果期间保持不变的字符串：如 SQLite 传入的文件名，
/// 在收到它的那次调用中读取。
unsafe fn our_name(name: *const c_char) -> Option<&'static str> {
    if name.is_null() {
        return None;
    }
    // SAFETY: 不为空，且按调用方的承诺以 NUL 结尾
    unsafe { CStr::from_ptr(name) }
        .to_str()
        .ok()
        .filter(|n| n.starts_with(NAME_PREFIX))
}

fn register_vfs() -> Result<()> {
    static DONE: OnceLock<std::result::Result<(), String>> = OnceLock::new();
    // SAFETY: SQLite 返回的默认 VFS 检查过不为空，且一直有效到进程结束；新的 VFS 被有意泄漏，名字是 `'static`
    // 字符串，所以两者都比任何连接存活得久；`DONE` 保证只注册一次。
    DONE.get_or_init(|| unsafe {
        let default = ffi::sqlite3_vfs_find(std::ptr::null());
        if default.is_null() {
            return Err("SQLite has no default VFS".into());
        }
        let d = &*default;
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_possible_wrap,
            reason = "文件结构体只有几十字节"
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
            // 系统的 VFS 在这些函数里不使用 `sqlite3_vfs*` 参数，所以可以原样共用。
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
    // SAFETY: SQLite 交给 `xOpen` 的是一个大小为 `szOsFile` 字节、等待填写的 `sqlite3_file`
    unsafe { (*file).pMethods = std::ptr::null() };
    // SAFETY: 这是 SQLite 传入的文件名，或者为空指针
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
        // SAFETY: `szOsFile` 不小于 `VfsFile` 的大小，而 `VfsFile` 的 `sqlite3_file` 在最前面
        unsafe {
            (*f).file = Arc::into_raw(cf);
            (*f).base.pMethods = &raw const IO_METHODS;
        }
        if !out_flags.is_null() {
            // SAFETY: SQLite 的输出参数，不为空
            unsafe {
                *out_flags = (flags & !(ffi::SQLITE_OPEN_READWRITE | ffi::SQLITE_OPEN_CREATE))
                    | ffi::SQLITE_OPEN_READONLY;
            }
        }
        return ffi::SQLITE_OK;
    }
    // 临时文件（排序、物化视图）交给系统的 VFS。
    let d = default_vfs(vfs);
    // SAFETY: 默认 VFS 一直有效到进程结束；它的 `xOpen` 收到的是 SQLite 原样传来的参数
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
    // SAFETY: 这是 SQLite 传入的文件名
    if unsafe { our_name(name) }.is_some() {
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    // SAFETY: 默认 VFS 一直有效到进程结束；它的 `xDelete` 收到的是 SQLite 原样传来的参数
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
    // SAFETY: 这是 SQLite 传入的文件名
    if unsafe { our_name(name) }.is_some() {
        // SAFETY: SQLite 的输出参数
        unsafe { *out = 0 }; // 没有日志，也没有 WAL：快照是一个不变的单独文件
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    // SAFETY: 默认 VFS 一直有效到进程结束；它的 `xAccess` 收到的是 SQLite 原样传来的参数
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
    // SAFETY: 这是 SQLite 传入的文件名
    if let Some(n) = unsafe { our_name(name) } {
        let bytes = n.as_bytes();
        if bytes.len() + 1 > usize::try_from(n_out).unwrap_or(0) {
            return ffi::SQLITE_CANTOPEN;
        }
        // SAFETY: `out` 是 SQLite 的缓冲区，能容纳 `n_out` 字节，名字连同结尾的 NUL 放得下
        unsafe {
            std::ptr::copy_nonoverlapping(bytes.as_ptr().cast::<c_char>(), out, bytes.len());
            *out.add(bytes.len()) = 0;
        }
        return ffi::SQLITE_OK;
    }
    let d = default_vfs(vfs);
    // SAFETY: 默认 VFS 一直有效到进程结束；它的 `xFullPathname` 收到的是 SQLite 原样传来的参数
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

/// [`x_open`] 打开的文件所读的快照。
///
/// # Safety
///
/// `f` 是 [`x_open`] 作为我们的文件打开、且 [`io_close`] 还没有关闭的文件：SQLite 只在这两者之间调用文件的方法。
unsafe fn cipher_file<'a>(f: *mut ffi::sqlite3_file) -> &'a CipherFile {
    // SAFETY: `x_open` 在这里放了一个有效的 `Arc<CipherFile>`，`io_close` 在最后一次调用之后才释放它
    unsafe { &*(*f.cast::<VfsFile>()).file }
}

unsafe extern "C" fn io_close(f: *mut ffi::sqlite3_file) -> c_int {
    let vf = f.cast::<VfsFile>();
    // SAFETY: SQLite 对 `x_open` 作为我们的文件打开的文件只关闭一次；其中的 `Arc` 只释放一次，指针随即清空
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
    // SAFETY: SQLite 读取的是我们的、已打开的文件
    let cf = unsafe { cipher_file(f) };
    // SAFETY: SQLite 的缓冲区，有 `amt` 字节（`amt` 不为负），只在这次调用中使用
    let out = unsafe {
        std::slice::from_raw_parts_mut(buf.cast::<u8>(), usize::try_from(amt).unwrap_or(0))
    };
    catch_unwind(AssertUnwindSafe(|| cf.read(out, offset.cast_unsigned())))
        .unwrap_or(ffi::SQLITE_IOERR_READ)
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
    // SAFETY: SQLite 查询的是我们的、已打开的文件，并传入它的输出参数
    unsafe {
        let len = cipher_file(f).size();
        *size = ffi::sqlite3_int64::try_from(len).unwrap_or(ffi::sqlite3_int64::MAX);
    }
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_lock(_: *mut ffi::sqlite3_file, _: c_int) -> c_int {
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_check_reserved_lock(_: *mut ffi::sqlite3_file, out: *mut c_int) -> c_int {
    // SAFETY: SQLite 的输出参数
    unsafe { *out = 0 };
    ffi::SQLITE_OK
}

unsafe extern "C" fn io_file_control(_: *mut ffi::sqlite3_file, _: c_int, _: *mut c_void) -> c_int {
    ffi::SQLITE_NOTFOUND
}

#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    reason = "PAGE_SIZE 为 4096"
)]
unsafe extern "C" fn io_sector_size(_: *mut ffi::sqlite3_file) -> c_int {
    PAGE_SIZE as c_int
}

unsafe extern "C" fn io_device_characteristics(_: *mut ffi::sqlite3_file) -> c_int {
    ffi::SQLITE_IOCAP_IMMUTABLE
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "测试数据已知放得下，或者有意回绕"
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
        // 大到能占好几个块：约 3000 行，每行 200 字节
        let values: Vec<String> = (0..3000)
            .map(|i| format!("{i:05}{}", "x".repeat(200)))
            .collect();
        let refs: Vec<&str> = values.iter().map(String::as_str).collect();
        let enc = encrypt_db(&plain_db(&refs), &cipher);
        let pages = (enc.len() / PAGE_SIZE) as u64;
        assert!(pages > 2 * u64::from(CHUNK_PAGES));
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
        // 两个线程同时未命中第 0 个块
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
        // 再加一个就挤出最旧的块，并且只挤出它
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

    /// 只含一个已提交事务的 WAL，这个事务只改写 `plain` 的第 1 页（一帧）。
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

    /// wal-index 的头（两份副本）加上 checkpoint 的信息。
    fn shm(salt: (u32, u32), backfilled: u32, attempted: u32) -> Vec<u8> {
        let mut b = vec![0u8; 32768];
        let mut hdr = [0u8; 48];
        hdr[0..4].copy_from_slice(&3007000u32.to_ne_bytes());
        hdr[12] = 1; // `isInit`：头已经初始化
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

    /// 行数多的数据库，打开之后扫描时要一块一块地读主文件。
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
        // 快照的 WAL：盐为 (1,2)，一个只改写第 1 页的已提交事务（frames = 1）
        let wal = wal_page_1(&cipher, &plain, (1, 2));
        fs::write(sibling(&path, "-wal"), &wal).unwrap();
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 1)).unwrap();

        // 没有越过快照的帧的 checkpoint 没有影响
        let (cf, conn) = open(&path);
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());

        // checkpoint 处理了快照不持有的帧（written = 5 > frames = 1）：下一次读主文件失败
        let (cf, conn) = open(&path);
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 5)).unwrap();
        assert!(conn
            .prepare("select v from t order by id")
            .and_then(|mut s| s
                .query_map([], |r| r.get::<_, String>(0))?
                .collect::<rusqlite::Result<Vec<_>>>())
            .is_err());
        assert!(cf.is_stale());

        // WAL 换了一代（盐变了）：同样过期
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
        // 读取时看不出来：wal-index 仍然描述快照的 WAL，也没有 checkpoint 越过快照的帧
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
        // 但 SQLite 发现读到的页格式错误时，调用方能看出文件变了，从头再来
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
        // 微信启动时新建自己的 wal-index；主文件（也就是快照）保持原样
        fs::write(sibling(&path, "-shm"), shm((5, 6), 0, 0)).unwrap();
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());
        // 主文件一旦变化（checkpoint），从中读取就会失败
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
        // 索引属于另一代 WAL，但什么都没有变：按后备的规则可以读取
        let dir = temp_dir("closed");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        fs::write(&path, encrypt_db(&big("b"), &cipher)).unwrap();
        fs::write(sibling(&path, "-shm"), shm((5, 6), 3, 3)).unwrap();
        let (cf, conn) = open(&path);
        assert_eq!(rows(&conn).len(), 2000);
        assert!(!cf.is_stale());
    }

    /// 微信关闭时删除 WAL 和 wal-index，再次打开时在原路径新建：快照持有的句柄看到的是已删除的 WAL，
    /// 大小和时间与原来相同，但快照已经不是数据库最新的状态（#1153）。新的 WAL 与旧的大小相同，
    /// 只有「是另一个文件」这一点能看出变化。
    #[test]
    fn a_wal_made_again_at_its_path_is_a_change() {
        let dir = temp_dir("wal-again");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let plain = big("f");
        fs::write(&path, encrypt_db(&plain, &cipher)).unwrap();
        fs::write(sibling(&path, "-wal"), wal_page_1(&cipher, &plain, (1, 2))).unwrap();
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 1)).unwrap();
        let (cf, _conn) = open(&path);
        assert!(cf.unchanged());

        fs::remove_file(sibling(&path, "-wal")).unwrap();
        fs::remove_file(sibling(&path, "-shm")).unwrap();
        // WAL 被删除、没有新建：数据库仍是原来的主文件，只是会多打开一次快照
        assert!(!cf.unchanged());

        let again = wal_page_1(&cipher, &plain, (3, 4));
        fs::write(sibling(&path, "-wal"), &again).unwrap();
        fs::write(sibling(&path, "-shm"), shm((3, 4), 0, 0)).unwrap();
        assert!(!cf.unchanged());
        // 接下来打开的快照持有新的 WAL，看不到变化
        let (cf, _conn) = open(&path);
        assert!(cf.unchanged());
    }

    /// 读主文件之后检查的是在原路径新建的 wal-index，而不是快照持有的、已删除的那个：已删除的仍然描述快照的
    /// WAL，而微信正在把新的 WAL checkpoint 进主文件（#1153）。
    #[test]
    fn a_wal_index_made_again_at_its_path_is_the_one_read() {
        let dir = temp_dir("shm-again");
        let cipher = PageCipher::derive(&KEY, &SALT);
        let path = dir.join("a.db");
        let plain = big("g");
        fs::write(&path, encrypt_db(&plain, &cipher)).unwrap();
        fs::write(sibling(&path, "-wal"), wal_page_1(&cipher, &plain, (1, 2))).unwrap();
        fs::write(sibling(&path, "-shm"), shm((1, 2), 1, 1)).unwrap();
        let (cf, conn) = open(&path);
        fs::remove_file(sibling(&path, "-shm")).unwrap();
        fs::write(sibling(&path, "-shm"), shm((3, 4), 2, 2)).unwrap();
        assert!(conn
            .query_row("select count(v) from t", [], |r| r.get::<_, i64>(0))
            .is_err());
        assert!(cf.is_stale());
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
        // 损坏第 21 页，即第二个块（17..=32）中的第五页
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

        // 按顺序读：块的第一页单独校验，之后一次四页
        for pgno in 2..=8 {
            read(pgno).unwrap();
        }
        assert!((2..=10).all(checked), "2 alone, then 3..=6, 7..=10");
        assert!(!checked(11));
        // 读到损坏的页之前：含有它的组校验失败，所以它前面的页逐页校验
        for pgno in 9..bad {
            read(pgno).unwrap();
        }
        assert!(!checked(bad) && !checked(bad + 1));
        let err = read(bad).unwrap_err().to_string();
        assert!(err.contains(&format!("page {bad}")), "{err}");
        assert!(cf.is_stale(), "as any torn page");
        // 它后面的页照常读取
        for pgno in bad + 1..=bad + 4 {
            read(pgno).unwrap();
        }

        // 查找只读块中的一页：只校验这一页
        read(40).unwrap();
        assert!(checked(40) && !checked(39) && !checked(41));
        // 从那里倒着读：一次校验四页，即这一页和它前面的三页
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
        // 内存放不下的排序会溢出到系统 VFS 的临时文件
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

/// `cargo kani` 的证明（#501）。
#[cfg(kani)]
mod proofs {
    use super::*;

    /// 任何读取位置都得到文件内某一页中的一段：至少一个字节、不超过请求的长度，起点正是请求的位置；
    /// 超出末尾时什么都不返回。
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
