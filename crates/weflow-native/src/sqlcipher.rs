//! 用纯 Rust 读取微信 4.x 的数据库（WCDB 写入的 SQLCipher 4 布局）。
//!
//! 布局如下，已在真实数据库上核对：
//! - 页大小 4096；每页末尾是 80 字节的保留区，即 16 字节的 IV 加 64 字节的 HMAC-SHA512
//! - 第 1 页开头的 16 字节是 KDF 盐，取代 SQLite 文件头
//! - `enc_key  = PBKDF2-HMAC-SHA512(raw_key_bytes, salt, 256000, 32)`
//! - `mac_key  = PBKDF2-HMAC-SHA512(enc_key, salt ^ 0x3a, 2, 32)`
//! - 页内容用 AES-256-CBC 加密（无填充）；`HMAC(mac_key, ciphertext || iv || pgno_le)` 必须匹配
//! - WAL 按页以相同方式加密（帧头为明文）
//!
//! `key db` 得到的密钥是 32 个原始字节，直接作为 PBKDF2 的口令，十六进制文本*不*参与派生。

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use aes::Aes256;
use anyhow::{anyhow, bail, Result};
use cbc::cipher::{block_padding::NoPadding, BlockModeDecrypt, InnerIvInit};
use hmac::{Hmac, KeyInit, Mac};
use sha2::{Digest, Sha256, Sha512};

use crate::sha512x4;

/// 微信数据库的页大小（字节），含末尾的保留区。
pub const PAGE_SIZE: usize = 4096;
const SALT_LEN: usize = 16;
const IV_LEN: usize = 16;
const HMAC_LEN: usize = 64;
pub(crate) const RESERVE: usize = IV_LEN + HMAC_LEN;
/// 第 1 页以外的页中，HMAC 在页号之前覆盖的字节数：页内容和 IV。
const SIGNED: usize = PAGE_SIZE - RESERVE + IV_LEN;
/// 从原始密钥派生页密钥的 PBKDF2 轮数；macOS x86_64 上用 DTrace 取钥时，靠它认出对原始密钥的那次派生。
pub(crate) const KDF_ITER: u32 = 256_000;
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

type Aes256CbcDec = cbc::Decryptor<Aes256>;
type HmacSha512 = Hmac<Sha512>;

/// 解析 `weflow key db` 输出的数据库密钥（64 个十六进制字符）。
///
/// # Errors
///
/// 去掉首尾空白后不是 64 个字符，或者含有非十六进制字符时返回错误。
pub fn parse_key(hex_key: &str) -> Result<[u8; 32]> {
    let hex_key = hex_key.trim();
    if hex_key.len() != 64 {
        bail!(
            "database key must be 64 hex characters, got {}",
            hex_key.len()
        );
    }
    // 切片之前先检查：多字节字符会让切片边界落在字符内部而 panic。
    if !hex_key.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("database key contains a non-hex character");
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex_key[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("database key contains a non-hex character"))?;
    }
    Ok(out)
}

/// 为一个数据库文件派生的密钥（每个文件有自己的盐）。
pub struct PageCipher {
    enc_key: [u8; 32],
    #[cfg_attr(
        not(any(test, feature = "test-fixtures")),
        expect(dead_code, reason = "只有夹具加密页时才用到")
    )]
    mac_key: [u8; 32],
    /// 派生密钥时用的盐，夹具把它写在第 1 页的开头。
    #[cfg_attr(
        not(any(test, feature = "test-fixtures")),
        expect(dead_code, reason = "只有夹具加密页时才用到派生时的盐")
    )]
    salt: Vec<u8>,
    /// 已写入 `mac_key` 的 HMAC 状态：每页克隆一份，省去重新计算密钥填充块的哈希。
    mac: HmacSha512,
    /// 同样写入了 `mac_key` 的 SHA-512 中间状态，供一次校验几页时使用（[`Self::verify_each`]）。
    mac_states: sha512x4::HmacKey,
    /// 已完成 `enc_key` 密钥编排的 AES：每次解密克隆一份，省去密钥编排。
    aes: Aes256,
}

/// 只显示类型名：每个字段都是密钥或者含有密钥。
impl std::fmt::Debug for PageCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PageCipher").finish_non_exhaustive()
    }
}

impl PageCipher {
    /// 用数据库密钥 `raw_key` 派生盐为 `salt` 的文件的密钥（PBKDF2-HMAC-SHA512，256,000 轮，有意设计得很慢）。
    pub fn derive(raw_key: &[u8; 32], salt: &[u8]) -> Self {
        let enc_key = crate::kdf::pbkdf2_hmac_sha512(raw_key, salt, KDF_ITER);
        Self::from_enc_key(enc_key, salt)
    }

    /// 已知页密钥（[`Self::derive`] 中慢的那一步的结果）时，得到数据库的全部密钥。
    fn from_enc_key(enc_key: [u8; 32], salt: &[u8]) -> Self {
        let mac_salt: Vec<u8> = salt.iter().map(|b| b ^ 0x3a).collect();
        let mac_key = crate::kdf::pbkdf2_hmac_sha512(&enc_key, &mac_salt, 2);
        #[expect(clippy::expect_used, reason = "HMAC 接受任意长度的密钥")]
        let mac =
            <HmacSha512 as KeyInit>::new_from_slice(&mac_key).expect("hmac accepts any key length");
        Self {
            aes: <Aes256 as KeyInit>::new(&enc_key.into()),
            enc_key,
            mac_states: sha512x4::HmacKey::new(&mac_key),
            mac_key,
            salt: salt.to_vec(),
            mac,
        }
    }

    /// 解密加密页 `page` 中 `blocks` 范围内的 16 字节块，写到 `out` 的相同位置，不校验这一页的 HMAC：
    /// 供需要很多页中各几个字节的读取使用。`page` 不能是第 1 页（它的页内容从盐之后开始）。CBC
    /// 解密一个块只需要它自身和前一个块（第一个块用 IV）。
    pub(crate) fn decrypt_blocks(
        &self,
        page: &[u8],
        blocks: std::ops::Range<usize>,
        out: &mut [u8],
    ) -> Result<()> {
        let Some((bytes, iv)) = block_bytes(blocks.clone())
            .filter(|_| page.len() == PAGE_SIZE && out.len() == PAGE_SIZE)
        else {
            bail!("bad block range {blocks:?}");
        };
        let Some(iv) = page[iv].first_chunk::<IV_LEN>() else {
            bail!("bad block range {blocks:?}");
        };
        let body = &mut out[bytes.clone()];
        body.copy_from_slice(&page[bytes]);
        Aes256CbcDec::inner_iv_init(self.aes.clone(), iv.into())
            .decrypt_padded::<NoPadding>(body)
            .map_err(|_| anyhow!("AES decrypt failed"))?;
        Ok(())
    }

    /// 校验加密的第 `pgno` 页（`PAGE_SIZE` 字节）的 HMAC：它覆盖整页，所以撕裂的页、在磁盘上损坏的页、
    /// 位置不对或密钥不对的页都会校验失败。
    pub(crate) fn verify(&self, pgno: u32, page: &[u8]) -> Result<()> {
        let start = if pgno == 1 { SALT_LEN } else { 0 };
        let body_end = PAGE_SIZE - RESERVE;
        let (Some(signed), Some(stored_mac)) = (
            page.get(start..body_end + IV_LEN),
            page.get(body_end + IV_LEN..),
        ) else {
            bail!("page {pgno} is shorter than {PAGE_SIZE} bytes");
        };
        let mut mac = self.mac.clone();
        mac.update(signed);
        mac.update(&pgno.to_le_bytes());
        mac.verify_slice(stored_mac).map_err(|_| hmac_failed(pgno))
    }

    /// 对 `pages`（页号和加密字节）中的每一页做 [`Self::verify`]，CPU 支持时一次四页（[`sha512x4`]）：
    /// 返回第一个校验失败的页的错误。剩下两三页时同样一次算四个哈希，空出的通道重复算其中的最后一页：
    /// 这样的花费与满四页时相同，比逐页算两三页少。
    pub(crate) fn verify_each(&self, pages: &[(u32, &[u8])]) -> Result<()> {
        for group in pages.chunks(sha512x4::LANES) {
            let four: [(u32, &[u8]); sha512x4::LANES] =
                std::array::from_fn(|l| group[l.min(group.len() - 1)]);
            let tags = (group.len() > 1).then(|| self.tags4(&four)).flatten();
            match tags {
                Some(tags) => {
                    for ((pgno, page), tag) in group.iter().zip(&tags) {
                        // 与 `verify` 一样比较每一个字节
                        let differ = tag
                            .iter()
                            .zip(&page[SIGNED..])
                            .fold(0u8, |d, (a, b)| d | (a ^ b));
                        if differ != 0 {
                            return Err(hmac_failed(*pgno));
                        }
                    }
                }
                None => {
                    for (pgno, page) in group {
                        self.verify(*pgno, page)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// 一次算出的四页的 HMAC；CPU 不支持时（[`sha512x4::hmac4`]），或者其中有第 1 页或大小不对的页时为
    /// `None`：第 1 页的盐不参与签名，签名的字节较少，而每个通道哈希的长度必须相同。
    fn tags4(&self, four: &[(u32, &[u8]); sha512x4::LANES]) -> Option<[[u8; HMAC_LEN]; 4]> {
        if !four
            .iter()
            .all(|(pgno, page)| *pgno != 1 && page.len() == PAGE_SIZE)
        {
            return None;
        }
        let numbers = four.map(|(pgno, _)| pgno.to_le_bytes());
        sha512x4::hmac4(
            &self.mac_states,
            four.map(|(_, page)| &page[..SIGNED]),
            std::array::from_fn(|l| &numbers[l][..]),
        )
    }

    /// 校验并解密一个加密页，写进 `out`（`PAGE_SIZE` 字节）。第 1 页解密后恢复标准的 SQLite 文件头。
    ///
    /// # Errors
    ///
    /// HMAC 校验失败（密钥不对或页已损坏）或者解密失败时返回错误。
    pub fn decrypt_page(&self, pgno: u32, page: &[u8], out: &mut [u8]) -> Result<()> {
        debug_assert_eq!(page.len(), PAGE_SIZE);
        debug_assert_eq!(out.len(), PAGE_SIZE);
        // 从未写入的页全部为 0，原样保存。
        if page.iter().all(|b| *b == 0) {
            out.fill(0);
            return Ok(());
        }
        self.verify(pgno, page)?;
        self.decrypt_verified_page(pgno, page, out)
    }

    /// 对已写入（不是全 0）、HMAC 已经校验过（[`Self::verify`]、[`Self::verify_each`]）的页做
    /// [`Self::decrypt_page`]：只解密，不再校验。
    pub(crate) fn decrypt_verified_page(
        &self,
        pgno: u32,
        page: &[u8],
        out: &mut [u8],
    ) -> Result<()> {
        let start = if pgno == 1 { SALT_LEN } else { 0 };
        let body_end = PAGE_SIZE - RESERVE;
        let Some(iv) = page.get(body_end..).and_then(<[u8]>::first_chunk::<IV_LEN>) else {
            bail!("page {pgno} is shorter than {PAGE_SIZE} bytes");
        };
        out.fill(0);
        if pgno == 1 {
            out[..SALT_LEN].copy_from_slice(SQLITE_MAGIC);
        }
        let body = &mut out[start..body_end];
        body.copy_from_slice(&page[start..body_end]);
        Aes256CbcDec::inner_iv_init(self.aes.clone(), iv.into())
            .decrypt_padded::<NoPadding>(body)
            .map_err(|_| anyhow!("AES decrypt failed for page {pgno}"))?;
        // 保留区（IV 和 HMAC 的位置）保持为 0：SQLite 把它当作未使用的空间。
        Ok(())
    }
}

/// HMAC 不匹配的页的错误。
fn hmac_failed(pgno: u32) -> anyhow::Error {
    anyhow!("HMAC check failed for page {pgno}: wrong key, or the page is corrupt")
}

/// 较快的密钥检查：派生密钥，再校验 `db` 的第 1 页。
fn verify_key(db: &[u8], raw_key: &[u8; 32]) -> Result<PageCipher> {
    if db.len() < PAGE_SIZE {
        bail!("file is smaller than one page: {} bytes", db.len());
    }
    if &db[..SQLITE_MAGIC.len()] == SQLITE_MAGIC {
        bail!("file is a plain SQLite database, not an encrypted one");
    }
    let cipher = PageCipher::derive(raw_key, &db[..SALT_LEN]);
    check_page1(&cipher, db)?;
    Ok(cipher)
}

fn check_page1(cipher: &PageCipher, db: &[u8]) -> Result<()> {
    let mut scratch = vec![0u8; PAGE_SIZE];
    cipher.decrypt_page(1, &db[..PAGE_SIZE], &mut scratch)
}

/// 正在派生或已经派生的密钥，按原始密钥和盐区分。
///
/// 表或槽的锁中毒后照常使用：持有锁的线程 panic 时不会留下改了一半的数据。表只会整项增加或整个清空，
/// 两种结果都是有效的缓存；槽只在密钥校验通过后写入一次，所以其中要么没有密钥，要么是校验过的密钥。
/// 中毒后如果改为报错，这个进程此后每次打开这个数据库都会失败。
type Derived = HashMap<([u8; 32], [u8; SALT_LEN]), Arc<Mutex<Option<Arc<PageCipher>>>>>;

static DERIVED: OnceLock<Mutex<Derived>> = OnceLock::new();

/// [`shared_cipher`] 最多保留的密钥数，超过时全部丢弃（一个账号只有几十个数据库）。
const MAX_DERIVED: usize = 1024;

/// 另一个进程交来的页密钥（[`import_keys`]），按原始密钥的 SHA-256 和盐区分。
type Imported = HashMap<([u8; 32], [u8; SALT_LEN]), [u8; 32]>;

static IMPORTED: OnceLock<Imported> = OnceLock::new();

/// [`export_keys`] 中一条记录的字节数：原始密钥的 SHA-256、盐和页密钥。
const RECORD: usize = 32 + SALT_LEN + 32;

fn fingerprint(raw_key: &[u8; 32]) -> [u8; 32] {
    Sha256::digest(raw_key).into()
}

/// [`shared_cipher`] 到目前为止派生的页密钥，交给这个进程启动的进程，供它调用 [`import_keys`]。
///
/// 页密钥与原始密钥一样能打开对应的数据库：这些字节只能经由管道交给那个进程，不作他用。
pub fn export_keys() -> Vec<u8> {
    let slots: Vec<_> = DERIVED
        .get_or_init(Default::default)
        .lock()
        // 锁中毒不会让表残缺（见 `Derived`）
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .map(|((raw_key, salt), slot)| (fingerprint(raw_key), *salt, slot.clone()))
        .collect();
    let mut out = Vec::with_capacity(slots.len() * RECORD);
    for (fingerprint, salt, slot) in slots {
        // 正在派生的密钥直接跳过，不等它
        if let Ok(slot) = slot.try_lock() {
            if let Some(cipher) = slot.as_ref() {
                out.extend_from_slice(&fingerprint);
                out.extend_from_slice(&salt);
                out.extend_from_slice(&cipher.enc_key);
            }
        }
    }
    out
}

/// 接收启动这个进程的进程用 [`export_keys`] 交来的页密钥，免得重新派生。
///
/// 只有第一次调用有效。只有确实能打开某个文件的密钥才会用于这个文件。
pub fn import_keys(bytes: &[u8]) {
    let _ = IMPORTED.set(parse_keys(bytes));
}

fn parse_keys(bytes: &[u8]) -> Imported {
    bytes
        .as_chunks::<RECORD>()
        .0
        .iter()
        .filter_map(|record| {
            let (fingerprint, rest) = record.split_first_chunk::<32>()?;
            let (salt, enc_key) = rest.split_first_chunk::<SALT_LEN>()?;
            Some(((*fingerprint, *salt), *enc_key.first_chunk::<32>()?))
        })
        .collect()
}

/// 对加密的 `db` 做 [`verify_key`]；`imported` 中有能打开它的页密钥时直接使用。
fn verify_key_with(
    imported: Option<&Imported>,
    db: &[u8],
    raw_key: &[u8; 32],
) -> Result<PageCipher> {
    let Some(&salt) = db.first_chunk::<SALT_LEN>() else {
        return verify_key(db, raw_key); // 由它报告哪里不对
    };
    if let Some(enc_key) = imported.and_then(|keys| keys.get(&(fingerprint(raw_key), salt))) {
        let cipher = PageCipher::from_enc_key(*enc_key, &salt);
        if check_page1(&cipher, db).is_ok() {
            return Ok(cipher);
        }
    }
    verify_key(db, raw_key)
}

/// `verify_key` 得到的页密钥，每个进程对每组原始密钥和盐只派生一次。
///
/// 256,000 轮约需 0.1 秒，而同一个文件会在快照丢弃后再次打开，也会被两个线程同时打开。
/// 一个线程请求另一个线程正在派生的密钥时，等它派生完，不重复派生；
/// 启动这个进程的进程交来的密钥（[`import_keys`]）完全不用派生。
///
/// # Errors
///
/// `db` 不到一页、是未加密的 SQLite 数据库，或者 `raw_key` 通不过第 1 页的校验时返回错误。
pub fn shared_cipher(db: &[u8], raw_key: &[u8; 32]) -> Result<Arc<PageCipher>> {
    if db.len() < PAGE_SIZE || &db[..SQLITE_MAGIC.len()] == SQLITE_MAGIC {
        return verify_key(db, raw_key).map(Arc::new); // 由它报告哪里不对
    }
    let mut salt = [0u8; SALT_LEN];
    salt.copy_from_slice(&db[..SALT_LEN]);
    let slot = {
        let mut all = DERIVED
            .get_or_init(Default::default)
            .lock()
            // 锁中毒不会让表残缺（见 `Derived`）
            .unwrap_or_else(|e| e.into_inner());
        if all.len() >= MAX_DERIVED && !all.contains_key(&(*raw_key, salt)) {
            all.clear();
        }
        all.entry((*raw_key, salt)).or_default().clone()
    };
    // 锁中毒后槽里要么为空，要么是校验过的密钥（见 `Derived`）
    let mut slot = slot.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(cipher) = slot.as_ref() {
        return Ok(cipher.clone());
    }
    let cipher = Arc::new(verify_key_with(IMPORTED.get(), db, raw_key)?);
    *slot = Some(cipher.clone());
    Ok(cipher)
}

/// 页内容中 `blocks` 范围内的 16 字节块所在的字节，以及解密其中第一块所用的 IV 所在的字节（前一个块；
/// 从第 0 块开始时是页的 IV）；块不全在页内容中时为 `None`。
fn block_bytes(
    blocks: std::ops::Range<usize>,
) -> Option<(std::ops::Range<usize>, std::ops::Range<usize>)> {
    let body_end = PAGE_SIZE - RESERVE;
    let (from, to) = (blocks.start.checked_mul(16)?, blocks.end.checked_mul(16)?);
    if to > body_end || from >= to {
        return None;
    }
    let iv = if from == 0 {
        body_end..body_end + IV_LEN
    } else {
        from - 16..from
    };
    Some((from..to, iv))
}

/// 一个需要采用的已提交 WAL 帧：页号，以及它的页数据在 WAL 中的字节偏移。
struct WalFrame {
    pgno: u32,
    data_offset: usize,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

/// SQLite WAL 的累积校验和：从 `(s0, s1)` 起，按 `big_endian` 给出的字节序每 8 字节累加一次 `data`。
fn wal_checksum(data: &[u8], big_endian: bool, mut s0: u32, mut s1: u32) -> (u32, u32) {
    for chunk in data.as_chunks::<8>().0 {
        let (a, b) = if big_endian {
            (be32(&chunk[0..4]), be32(&chunk[4..8]))
        } else {
            (
                u32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]),
                u32::from_le_bytes([chunk[4], chunk[5], chunk[6], chunk[7]]),
            )
        };
        s0 = s0.wrapping_add(a).wrapping_add(s1);
        s1 = s1.wrapping_add(b).wrapping_add(s0);
    }
    (s0, s1)
}

/// WAL 的内容：它的盐（头有效时）、已提交前缀的帧数（SQLite 的 wal-index 称为 `mxFrame`），以及每页最新的已提交帧。
pub(crate) struct WalOverlay {
    pub salt: Option<[u8; 8]>,
    pub frames: u32,
    /// 最后一次提交后的数据库页数（没有已提交的内容时为 `None`）。
    pub db_pages: Option<u32>,
    /// 页号 -> 这一页最新的已提交副本在 WAL 中的字节偏移。
    pub pages: HashMap<u32, usize>,
}

/// 解析 WAL（见 [`WalOverlay`]）。缺失或无效的 WAL 得到空的结果。
pub(crate) fn wal_overlay(wal: Option<&[u8]>) -> WalOverlay {
    let salt = wal.and_then(wal_salt);
    let mut overlay = WalOverlay {
        salt,
        frames: 0,
        db_pages: None,
        pages: HashMap::new(),
    };
    if let Some((frames, db_pages)) = wal.and_then(committed_wal_frames::<PAGE_SIZE>) {
        // 每帧含一页，所以 SQLite 能读的 WAL 不会有 `u32::MAX` 帧
        overlay.frames = u32::try_from(frames.len()).unwrap_or(u32::MAX);
        overlay.db_pages = Some(db_pages);
        for f in frames {
            overlay.pages.insert(f.pgno, f.data_offset); // 后面的帧覆盖前面的帧
        }
    }
    overlay
}

/// 头有效的 WAL 的盐。
fn wal_salt(wal: &[u8]) -> Option<[u8; 8]> {
    wal_header::<PAGE_SIZE>(wal)?;
    wal[16..24].try_into().ok()
}

/// 校验页大小为 `PAGE` 字节的 WAL 的头，返回（校验和是否为大端，头校验和的两个字）。
fn wal_header<const PAGE: usize>(wal: &[u8]) -> Option<(bool, u32, u32)> {
    if wal.len() < 32 {
        return None;
    }
    let magic = be32(&wal[0..4]);
    if magic != 0x377f0682 && magic != 0x377f0683 {
        return None;
    }
    let big_endian = magic & 1 == 1;
    if be32(&wal[8..12]) as usize != PAGE {
        return None;
    }
    let (h0, h1) = wal_checksum(&wal[..24], big_endian, 0, 0);
    if h0 != be32(&wal[24..28]) || h1 != be32(&wal[28..32]) {
        return None;
    }
    Some((big_endian, h0, h1))
}

/// 解析页大小为 `PAGE` 字节的 WAL，返回最后一个有效的*已提交*前缀中的帧，以及最后一次提交后的数据库页数。
/// WAL 中没有可用的内容时返回 `None`。
fn committed_wal_frames<const PAGE: usize>(wal: &[u8]) -> Option<(Vec<WalFrame>, u32)> {
    const WAL_HEADER: usize = 32;
    const FRAME_HEADER: usize = 24;
    let (big_endian, h0, h1) = wal_header::<PAGE>(wal)?;
    let salt = (&wal[16..20], &wal[20..24]);
    let (mut s0, mut s1) = (h0, h1);

    let frame_size = FRAME_HEADER + PAGE;
    let mut pending: Vec<WalFrame> = Vec::new();
    let mut committed: Vec<WalFrame> = Vec::new();
    let mut db_pages = 0u32;
    let mut pos = WAL_HEADER;
    while pos + frame_size <= wal.len() {
        let fh = &wal[pos..pos + FRAME_HEADER];
        if &fh[8..12] != salt.0 || &fh[12..16] != salt.1 {
            break;
        }
        let (c0, c1) = wal_checksum(&fh[..8], big_endian, s0, s1);
        let (c0, c1) = wal_checksum(
            &wal[pos + FRAME_HEADER..pos + frame_size],
            big_endian,
            c0,
            c1,
        );
        if c0 != be32(&fh[16..20]) || c1 != be32(&fh[20..24]) {
            break;
        }
        (s0, s1) = (c0, c1);
        let pgno = be32(&fh[0..4]);
        if pgno == 0 {
            break;
        }
        pending.push(WalFrame {
            pgno,
            data_offset: pos + FRAME_HEADER,
        });
        let commit = be32(&fh[4..8]);
        if commit != 0 {
            committed.append(&mut pending);
            db_pages = commit;
        }
        pos += frame_size;
    }
    if committed.is_empty() {
        None
    } else {
        Some((committed, db_pages))
    }
}

/// 读取方看到的数据库页数：WAL 中最后一个已提交事务之后的大小，没有时为主文件的大小。
pub fn total_pages(main_len: u64, db_pages: Option<u32>) -> u32 {
    match db_pages {
        Some(n) => n.max(1),
        // SQLite 用 32 位给页编号
        None => u32::try_from(main_len / PAGE_SIZE as u64).unwrap_or(u32::MAX),
    }
}

/// 修补解密后的第 1 页，让数据库作为独立的文件打开：使文件头里的页数生效，
/// 把 WAL 模式（第 18、19 字节）改回旧式日志，这样 SQLite 不会去找 `-wal` 文件。
///
/// # Panics
///
/// `page1` 比 100 字节的 SQLite 文件头还短时 panic。
pub fn patch_header(page1: &mut [u8], total_pages: u32) {
    page1[28..32].copy_from_slice(&total_pages.to_be_bytes());
    page1.copy_within(24..28, 92); // 文件修改计数器写进 version-valid-for，页数才会被采用
    page1[18] = 1;
    page1[19] = 1;
}

/// 把内存中的整个数据库（给出 WAL 时连同 WAL）解密成明文的 SQLite 映像。
/// 测试按需读取（[`crate::cipher_vfs`]）时以它为对照。
///
/// # Errors
///
/// 页数超出 `u32` 的范围，或者某一页的 HMAC 校验失败时返回错误。
#[cfg(any(test, feature = "test-fixtures"))]
pub fn decrypt_database(db: &[u8], wal: Option<&[u8]>, cipher: &PageCipher) -> Result<Vec<u8>> {
    let overlay = wal_overlay(wal);
    let total = total_pages(db.len() as u64, overlay.db_pages);
    let main_pages = u32::try_from(db.len() / PAGE_SIZE)?;
    let mut out = vec![0u8; total as usize * PAGE_SIZE];
    for pgno in 1..=main_pages.min(total) {
        let i = (pgno as usize - 1) * PAGE_SIZE;
        cipher.decrypt_page(pgno, &db[i..i + PAGE_SIZE], &mut out[i..i + PAGE_SIZE])?;
    }
    if let Some(wal) = wal {
        for (&pgno, &off) in &overlay.pages {
            if pgno <= total {
                let i = (pgno as usize - 1) * PAGE_SIZE;
                cipher.decrypt_page(
                    pgno,
                    &wal[off..off + PAGE_SIZE],
                    &mut out[i..i + PAGE_SIZE],
                )?;
            }
        }
    }
    patch_header(&mut out[..PAGE_SIZE], total);
    Ok(out)
}

#[cfg(any(test, feature = "test-fixtures"))]
#[expect(
    clippy::cast_possible_truncation,
    reason = "测试数据已知放得下，或者有意回绕"
)]
pub mod testutil {
    //! 测试用的加密和明文数据库映像。
    //!
    //! # Panics
    //!
    //! 构建的测试数据出错时，这些辅助函数 panic，使调用它的测试失败。

    use super::*;
    use aes::cipher::BlockModeEncrypt;
    use rusqlite::Connection;

    type Aes256CbcEnc = cbc::Encryptor<Aes256>;

    /// 测试数据库的原始数据库密钥。
    pub const KEY: [u8; 32] = [7u8; 32];
    /// 测试数据库的盐。
    pub const SALT: [u8; 16] = *b"0123456789abcdef";

    /// 完全按 SQLCipher 4 的方式加密一个明文页（只用于测试，是 `decrypt_page` 的逆操作）；
    /// 第 1 页以派生 `cipher` 时用的盐开头。
    #[expect(clippy::expect_used, reason = "测试夹具：失败说明测试的准备有误")]
    pub fn encrypt_page(cipher: &PageCipher, pgno: u32, plain: &[u8], iv: [u8; 16]) -> Vec<u8> {
        let start = if pgno == 1 { SALT_LEN } else { 0 };
        let body_end = PAGE_SIZE - RESERVE;
        let mut page = vec![0u8; PAGE_SIZE];
        let mut body = plain[start..body_end].to_vec();
        let len = body.len();
        <Aes256CbcEnc as cbc::cipher::KeyIvInit>::new(&cipher.enc_key.into(), &iv.into())
            .encrypt_padded::<NoPadding>(&mut body, len)
            .expect("a whole page body needs no padding");
        page[start..body_end].copy_from_slice(&body);
        if pgno == 1 {
            page[..SALT_LEN].copy_from_slice(&cipher.salt);
        }
        page[body_end..body_end + IV_LEN].copy_from_slice(&iv);
        let mut mac = <HmacSha512 as KeyInit>::new_from_slice(&cipher.mac_key)
            .expect("hmac accepts any key length");
        mac.update(&page[start..body_end + IV_LEN]);
        mac.update(&pgno.to_le_bytes());
        page[body_end + IV_LEN..].copy_from_slice(&mac.finalize().into_bytes());
        page
    }

    /// 每页留出 80 字节保留区的明文 SQLite 映像；`setup` 建表并写入行。
    pub fn plain_db_with(setup: impl FnOnce(&Connection)) -> Vec<u8> {
        let conn = plain_connection();
        setup(&conn);
        plain_image(&conn)
    }

    /// 用 [`plain_connection`] 建的数据库当前的明文映像。
    #[expect(clippy::expect_used, reason = "测试夹具：失败说明测试的准备有误")]
    pub fn plain_image(conn: &Connection) -> Vec<u8> {
        let data = conn
            .serialize(rusqlite::MAIN_DB)
            .expect("serialize the database");
        assert_eq!(data[20] as usize, RESERVE);
        data.to_vec()
    }

    /// 页大小为 4096 字节、留出 SQLCipher 所需保留区的空内存数据库，写入数据后用 [`plain_image`] 取得映像。
    #[expect(clippy::expect_used, reason = "测试夹具：失败说明测试的准备有误")]
    pub fn plain_connection() -> Connection {
        let conn = Connection::open_in_memory().expect("open an in-memory database");
        conn.pragma_update(None, "page_size", 4096)
            .expect("set the page size");
        let mut reserve =
            std::os::raw::c_int::try_from(RESERVE).expect("the reserve is a few dozen bytes");
        // SQLITE_FCNTL_RESERVE_BYTES = 38；只能在数据库还是空的时候设置。
        // SAFETY: 连接是打开的，`main` 是以 NUL 结尾的字面量，`reserve` 是存活的局部变量；
        // SQLITE_FCNTL_RESERVE_BYTES 通过它读写一个 `c_int`。
        let rc = unsafe {
            rusqlite::ffi::sqlite3_file_control(
                conn.handle(),
                c"main".as_ptr(),
                38,
                (&raw mut reserve).cast(),
            )
        };
        assert_eq!(rc, 0);
        conn
    }

    /// 只有一张表 `t(id integer primary key, v text)` 的明文 SQLite 映像，按顺序存放 `rows`，id 从 0 开始。
    #[expect(clippy::expect_used, reason = "测试夹具：失败说明测试的准备有误")]
    pub fn plain_db(rows: &[&str]) -> Vec<u8> {
        plain_db_with(|conn| {
            conn.execute_batch("create table t(id integer primary key, v text)")
                .expect("create the table");
            for (i, r) in rows.iter().enumerate() {
                conn.execute(
                    "insert into t values (?1, ?2)",
                    rusqlite::params![i64::try_from(i).expect("a row number"), r],
                )
                .expect("insert a row");
            }
        })
    }

    /// 大端 WAL 的 SQLite 校验和，从 `sum` 接着累加。
    pub fn wal_checksum_be(data: &[u8], sum: (u32, u32)) -> (u32, u32) {
        wal_checksum(data, true, sum.0, sum.1)
    }

    /// 代次（盐）为 `salt` 的 WAL 的 32 字节头。
    pub fn encrypted_wal_header(salt: (u32, u32)) -> Vec<u8> {
        let mut h = Vec::new();
        for v in [0x377f0683u32, 3007000, PAGE_SIZE as u32, 0, salt.0, salt.1] {
            h.extend(v.to_be_bytes());
        }
        let (c0, c1) = wal_checksum_be(&h, (0, 0));
        h.extend(c0.to_be_bytes());
        h.extend(c1.to_be_bytes());
        h
    }

    /// 只含一个已提交事务的 WAL，这个事务把明文映像 `base` 变成 `newer`：每个不同的页或新页各一帧。`base`
    /// 为空时改写每一页。
    pub fn encrypt_wal(
        cipher: &PageCipher,
        base: &[u8],
        newer: &[u8],
        salt: (u32, u32),
    ) -> Vec<u8> {
        let mut wal = encrypted_wal_header(salt);
        let mut sum = (
            u32::from_be_bytes([wal[24], wal[25], wal[26], wal[27]]),
            u32::from_be_bytes([wal[28], wal[29], wal[30], wal[31]]),
        );
        let pages = (newer.len() / PAGE_SIZE) as u32;
        let changed: Vec<u32> = (1..=pages)
            .filter(|&pgno| {
                let at = (pgno as usize - 1) * PAGE_SIZE..pgno as usize * PAGE_SIZE;
                base.get(at.clone()) != newer.get(at)
            })
            .collect();
        for (i, &pgno) in changed.iter().enumerate() {
            let at = (pgno as usize - 1) * PAGE_SIZE;
            let page = encrypt_page(cipher, pgno, &newer[at..at + PAGE_SIZE], [0x55; 16]);
            let mut fh = Vec::new();
            fh.extend(pgno.to_be_bytes());
            // 最后一帧提交事务，并给出数据库的页数
            fh.extend((if i + 1 == changed.len() { pages } else { 0 }).to_be_bytes());
            fh.extend(salt.0.to_be_bytes());
            fh.extend(salt.1.to_be_bytes());
            sum = wal_checksum_be(&page, wal_checksum_be(&fh[..8], sum));
            fh.extend(sum.0.to_be_bytes());
            fh.extend(sum.1.to_be_bytes());
            wal.extend(fh);
            wal.extend(page);
        }
        wal
    }

    /// 按 SQLCipher 4 的写法逐页加密的明文 SQLite 文件，每页有自己的 IV。
    pub fn encrypt_db(plain: &[u8], cipher: &PageCipher) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, page) in plain.chunks(PAGE_SIZE).enumerate() {
            out.extend(encrypt_page(
                cipher,
                i as u32 + 1,
                page,
                [(i as u8).wrapping_add(1); 16],
            ));
        }
        out
    }
}

#[cfg(test)]
#[expect(
    clippy::cast_possible_truncation,
    reason = "测试数据已知放得下，或者有意回绕"
)]
mod tests {
    use super::testutil::*;
    use super::*;
    use rusqlite::Connection;

    fn rows_of(image: Vec<u8>) -> Vec<String> {
        let mut conn = Connection::open_in_memory().unwrap();
        let n = image.len();
        conn.deserialize_read_exact(rusqlite::MAIN_DB, &image[..], n, true)
            .unwrap();
        let mut stmt = conn.prepare("select v from t order by id").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect()
    }

    #[test]
    fn roundtrip_decrypts_and_queries() {
        let cipher = PageCipher::derive(&KEY, &SALT);
        let enc = encrypt_db(&plain_db(&["alpha", "beta"]), &cipher);
        let derived = verify_key(&enc, &KEY).unwrap();
        assert_eq!(
            rows_of(decrypt_database(&enc, None, &derived).unwrap()),
            ["alpha", "beta"]
        );
    }

    #[test]
    fn parse_key_rejects_non_ascii_without_panicking() {
        // UTF-8 下是 64 字节，所以能通过长度检查，只有字符检查才能拦住
        for key in [
            format!("a中{}", "a".repeat(60)),
            format!("中{}", "a".repeat(61)),
            format!("🙂{}", "a".repeat(60)),
        ] {
            assert_eq!(key.len(), 64);
            let err = parse_key(&key).unwrap_err().to_string();
            assert!(err.contains("non-hex"), "{err}");
        }
        assert!(parse_key(&"g".repeat(64)).is_err());
        assert_eq!(parse_key(&"0A".repeat(32)).unwrap(), [0x0a; 32]);
    }

    #[test]
    fn keys_are_derived_once_per_key_and_salt() {
        let enc = encrypt_db(&plain_db(&["x"]), &PageCipher::derive(&[3u8; 32], &SALT));
        let first = std::thread::scope(|s| {
            let all: Vec<_> = (0..4)
                .map(|_| s.spawn(|| shared_cipher(&enc, &[3u8; 32]).unwrap()))
                .collect();
            let all: Vec<_> = all.into_iter().map(|h| h.join().unwrap()).collect();
            assert!(all.iter().all(|c| Arc::ptr_eq(c, &all[0])));
            all[0].clone()
        });
        assert!(Arc::ptr_eq(
            &first,
            &shared_cipher(&enc, &[3u8; 32]).unwrap()
        ));
        // 错误的密钥会被检查出来，不会保留
        assert!(shared_cipher(&enc, &[4u8; 32]).is_err());
        assert!(shared_cipher(&enc, &[4u8; 32]).is_err());
        assert!(shared_cipher(&plain_db(&["x"]), &[3u8; 32]).is_err());
    }

    #[test]
    fn keys_handed_over_are_used_when_they_open_the_file() {
        let raw = [11u8; 32];
        let real = PageCipher::derive(&raw, &SALT);
        let enc = encrypt_db(&plain_db(&["x"]), &real);

        // 进程派生的密钥可以导出，读回来与原来相同
        shared_cipher(&enc, &raw).unwrap();
        let exported = export_keys();
        assert_eq!(exported.len() % RECORD, 0);
        let keys = parse_keys(&exported);
        assert_eq!(keys.get(&(fingerprint(&raw), SALT)), Some(&real.enc_key));
        // 导出的数据里没有原始密钥本身
        assert!(!exported.windows(32).any(|w| w == raw));

        // 交来的密钥不经派生直接使用：这里把它登记在一个派生不出它的原始密钥下
        let other = [12u8; 32];
        let mut handed = Imported::new();
        handed.insert((fingerprint(&other), SALT), real.enc_key);
        let taken = verify_key_with(Some(&handed), &enc, &other).unwrap();
        assert_eq!(taken.mac_key, real.mac_key);
        // 打不开文件的密钥会重新派生，所以错误的原始密钥仍然失败
        handed.insert((fingerprint(&raw), SALT), [0u8; 32]);
        let derived = verify_key_with(Some(&handed), &enc, &raw).unwrap();
        assert_eq!(derived.enc_key, real.enc_key);
        assert!(verify_key_with(Some(&handed), &enc, &[13u8; 32]).is_err());
        // 不完整的记录被丢弃
        assert_eq!(parse_keys(&exported[..RECORD - 1]).len(), 0);
    }

    #[test]
    fn wrong_key_is_rejected() {
        let cipher = PageCipher::derive(&KEY, &SALT);
        let enc = encrypt_db(&plain_db(&["x"]), &cipher);
        assert!(verify_key(&enc, &[9u8; 32]).is_err());
    }

    #[test]
    fn tampered_page_fails_hmac() {
        let cipher = PageCipher::derive(&KEY, &SALT);
        let mut enc = encrypt_db(&plain_db(&["x"]), &cipher);
        enc[100] ^= 1;
        assert!(verify_key(&enc, &KEY).is_err());
    }

    /// 性质测试用的密钥，只派生一次：派生是其中慢的部分。
    static PROPERTY_CIPHER: std::sync::LazyLock<PageCipher> =
        std::sync::LazyLock::new(|| PageCipher::derive(&KEY, &SALT));

    /// 一页的明文字节：任意的页内容，保留区与 SQLite 留下的一样为空。
    fn plain_page() -> impl proptest::strategy::Strategy<Value = Vec<u8>> {
        use proptest::prelude::*;
        prop::collection::vec(any::<u8>(), PAGE_SIZE - RESERVE).prop_map(|mut page| {
            page.resize(PAGE_SIZE, 0);
            page
        })
    }

    proptest::proptest! {
        #![proptest_config(crate::property_cases(256))]

        /// 任何页都解密回加密前的字节；第 1 页的盐换成 SQLite 文件头（#502）。
        #[test]
        fn any_page_decrypts_to_its_plain_bytes(
            plain in plain_page(),
            pgno in 1u32..=1_000_000,
            iv in proptest::prelude::any::<[u8; IV_LEN]>(),
        ) {
            let page = encrypt_page(&PROPERTY_CIPHER, pgno, &plain, iv);
            let mut out = vec![0xaa; PAGE_SIZE];
            proptest::prop_assert!(PROPERTY_CIPHER.decrypt_page(pgno, &page, &mut out).is_ok());
            let start = if pgno == 1 { SALT_LEN } else { 0 };
            let body_end = PAGE_SIZE - RESERVE;
            proptest::prop_assert_eq!(&out[start..body_end], &plain[start..body_end]);
            proptest::prop_assert!(out[body_end..].iter().all(|&b| b == 0));
            if pgno == 1 {
                proptest::prop_assert_eq!(&out[..SALT_LEN], SQLITE_MAGIC);
            }
        }

        /// 改动了任何一个字节的页，或者被当作另一页读取的页，都校验失败，不会解密成别的字节（#502）。
        /// 第 1 页开头的盐不参与签名：密钥由它派生，换了盐每一页都会失败。
        #[test]
        fn any_changed_or_moved_page_fails_its_check(
            plain in plain_page(),
            pgno in 1u32..=1_000_000,
            iv in proptest::prelude::any::<[u8; IV_LEN]>(),
            at in 0..PAGE_SIZE,
            flip in 1u8..=255,
            other in 1u32..=1_000_000,
        ) {
            let page = encrypt_page(&PROPERTY_CIPHER, pgno, &plain, iv);
            let mut out = vec![0; PAGE_SIZE];
            if pgno != 1 || at >= SALT_LEN {
                let mut changed = page.clone();
                changed[at] ^= flip;
                proptest::prop_assert!(PROPERTY_CIPHER.decrypt_page(pgno, &changed, &mut out).is_err());
            }
            if other != pgno {
                proptest::prop_assert!(PROPERTY_CIPHER.decrypt_page(other, &page, &mut out).is_err());
            }
        }
    }

    fn wal_header(salt1: u32, salt2: u32) -> Vec<u8> {
        let mut h = Vec::new();
        for v in [0x377f0683u32, 3007000, PAGE_SIZE as u32, 0, salt1, salt2] {
            h.extend(v.to_be_bytes());
        }
        let (c0, c1) = wal_checksum(&h, true, 0, 0);
        h.extend(c0.to_be_bytes());
        h.extend(c1.to_be_bytes());
        h
    }

    /// 追加一个带有已加密的 `page` 的 WAL 帧，返回累积的校验和。
    fn push_frame(
        wal: &mut Vec<u8>,
        sum: (u32, u32),
        pgno: u32,
        commit: u32,
        salts: (u32, u32),
        page: &[u8],
    ) -> (u32, u32) {
        let mut fh = Vec::new();
        fh.extend(pgno.to_be_bytes());
        fh.extend(commit.to_be_bytes());
        fh.extend(salts.0.to_be_bytes());
        fh.extend(salts.1.to_be_bytes());
        let s = wal_checksum(&fh[..8], true, sum.0, sum.1);
        let s = wal_checksum(page, true, s.0, s.1);
        fh.extend(s.0.to_be_bytes());
        fh.extend(s.1.to_be_bytes());
        wal.extend(fh);
        wal.extend(page);
        s
    }

    #[test]
    fn wal_committed_frames_override_main_and_uncommitted_tail_is_ignored() {
        let cipher = PageCipher::derive(&KEY, &SALT);
        let old = plain_db(&["old"]);
        let enc_main = encrypt_db(&old, &cipher);
        let derived = verify_key(&enc_main, &KEY).unwrap();

        // 表结构相同、行不同：变化的是第 2 页（表的根叶子页）。
        let newer = plain_db(&["new"]);
        assert_eq!(newer.len(), old.len());
        let newest = plain_db(&["never-committed"]);

        let salts = (0xdead_beef, 0x0102_0304);
        let mut wal = wal_header(salts.0, salts.1);
        let mut sum = (be32(&wal[24..28]), be32(&wal[28..32]));
        let db_pages = (old.len() / PAGE_SIZE) as u32;
        for pgno in 1..=db_pages {
            let pg = &newer[(pgno as usize - 1) * PAGE_SIZE..pgno as usize * PAGE_SIZE];
            let commit = if pgno == db_pages { db_pages } else { 0 };
            sum = push_frame(
                &mut wal,
                sum,
                pgno,
                commit,
                salts,
                &encrypt_page(&cipher, pgno, pg, [0x55; 16]),
            );
        }
        // 之后一个从未提交的事务（commit == 0）：不能采用。
        let pg = &newest[..PAGE_SIZE];
        push_frame(
            &mut wal,
            sum,
            1,
            0,
            salts,
            &encrypt_page(&cipher, 1, pg, [0x66; 16]),
        );

        assert_eq!(
            rows_of(decrypt_database(&enc_main, None, &derived).unwrap()),
            ["old"]
        );
        assert_eq!(
            rows_of(decrypt_database(&enc_main, Some(&wal), &derived).unwrap()),
            ["new"]
        );
    }

    #[test]
    fn wal_with_bad_checksum_or_foreign_salt_is_ignored() {
        let cipher = PageCipher::derive(&KEY, &SALT);
        let plain = plain_db(&["kept"]);
        let enc = encrypt_db(&plain, &cipher);
        let derived = verify_key(&enc, &KEY).unwrap();
        let salts = (1u32, 2u32);
        let mut wal = wal_header(salts.0, salts.1);
        let sum = (be32(&wal[24..28]), be32(&wal[28..32]));
        let other = plain_db(&["stale"]);
        let pg = &other[..PAGE_SIZE];
        // 用另一对盐写入的帧（WAL 重置之前遗留的）。
        push_frame(
            &mut wal,
            sum,
            1,
            1,
            (9, 9),
            &encrypt_page(&cipher, 1, pg, [3; 16]),
        );
        assert_eq!(
            rows_of(decrypt_database(&enc, Some(&wal), &derived).unwrap()),
            ["kept"]
        );
    }
}

/// `cargo kani` 的证明（#501）：覆盖各证明所述大小以内的每一种输入。
#[cfg(kani)]
mod proofs {
    use super::*;

    /// 从任意字节读取 WAL 头都不会 panic，且只有 32 字节或更长时才可能读出。
    #[kani::proof]
    fn any_bytes_are_read_as_a_wal_header_or_refused() {
        let bytes: [u8; 40] = kani::any();
        let len: usize = kani::any_where(|&n| n <= bytes.len());
        if wal_header::<PAGE_SIZE>(&bytes[..len]).is_some() {
            assert!(len >= 32);
        }
    }

    /// 读取不超过两帧的任何 WAL 的已提交帧都不会 panic：每帧都完整地在 WAL 内，位于头和前一帧之后，页号不为 0，
    /// 提交后的大小也不为 0。页大小用 8 字节而不是 4096 字节：遍历和边界与页大小无关，
    /// 而 4096 字节的校验和对求解器来说太大。
    #[kani::proof]
    #[kani::unwind(12)]
    fn any_wal_gives_committed_frames_inside_it() {
        const PAGE: usize = 8;
        const LEN: usize = 32 + 2 * (24 + PAGE) + 8;
        let bytes: [u8; LEN] = kani::any();
        let len: usize = kani::any_where(|&n| n <= LEN);
        if let Some((frames, db_pages)) = committed_wal_frames::<PAGE>(&bytes[..len]) {
            assert!(db_pages != 0);
            assert!(!frames.is_empty() && frames.len() <= 2);
            let mut end = 32;
            // 上面断言了最多 2 帧：给求解器一个能展开的上界
            for f in frames.iter().take(2) {
                assert!(f.pgno != 0);
                assert!(f.data_offset == end + 24);
                assert!(f.data_offset + PAGE <= len);
                end = f.data_offset + PAGE;
            }
        }
    }

    /// 页数不会溢出：是最后一次提交的大小（至少一页），没有时是主文件的整页数，最多为 32 位能编号的页数。
    #[kani::proof]
    fn total_pages_are_the_commit_or_the_main_file() {
        let main_len: u64 = kani::any();
        let db_pages: Option<u32> = kani::any();
        let n = total_pages(main_len, db_pages);
        match db_pages {
            Some(p) => assert!(n == p.max(1)),
            None => assert!(u64::from(n) == (main_len / PAGE_SIZE as u64).min(u64::from(u32::MAX))),
        }
    }

    /// 修补任何 100 字节或更长的第 1 页都不会 panic：只有页数、version-valid-for 和两个格式字节改变，其他不变。
    #[kani::proof]
    #[kani::unwind(130)]
    fn page_1_is_patched_in_place() {
        let mut page: [u8; 128] = kani::any();
        let before = page;
        let len: usize = kani::any_where(|&n: &usize| (100..=128).contains(&n));
        let total: u32 = kani::any();
        patch_header(&mut page[..len], total);
        assert!(page[28..32] == total.to_be_bytes());
        assert!(page[92..96] == before[24..28]);
        assert!(page[18] == 1 && page[19] == 1);
        for i in 0..page.len() {
            if !(18..20).contains(&i) && !(28..32).contains(&i) && !(92..96).contains(&i) {
                assert!(page[i] == before[i]);
            }
        }
    }

    /// 任何块范围要么被拒绝，要么对应页内容中的整块，且 IV 是页内的 16 字节：第一块的前一块，
    /// 从第 0 块开始时是页自己的 IV。
    #[kani::proof]
    fn any_block_range_stays_inside_the_page() {
        let (start, end): (usize, usize) = kani::any();
        if let Some((bytes, iv)) = block_bytes(start..end) {
            let body_end = PAGE_SIZE - RESERVE;
            assert!(bytes.start < bytes.end && bytes.end <= body_end);
            assert!(bytes.start % 16 == 0 && bytes.end % 16 == 0);
            assert!(iv.end - iv.start == IV_LEN && iv.end <= PAGE_SIZE);
            assert!(if bytes.start == 0 {
                iv.start == body_end
            } else {
                iv.end == bytes.start
            });
        }
    }
}
