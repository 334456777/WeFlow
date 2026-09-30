//! Pure-Rust reader for WeChat 4.x databases (SQLCipher 4 layout, as written by WCDB).
//!
//! Layout, verified against real databases:
//! - page size 4096; every page ends with an 80-byte reserve = 16-byte IV + 64-byte HMAC-SHA512
//! - the first 16 bytes of page 1 hold the KDF salt instead of the SQLite header
//! - `enc_key  = PBKDF2-HMAC-SHA512(raw_key_bytes, salt, 256000, 32)`
//! - `mac_key  = PBKDF2-HMAC-SHA512(enc_key, salt ^ 0x3a, 2, 32)`
//! - page body is AES-256-CBC (no padding); `HMAC(mac_key, ciphertext || iv || pgno_le)` must match
//! - the WAL is encrypted the same way, page by page (frame headers stay in the clear)
//!
//! The key from `key db` is the raw 32 bytes fed to PBKDF2 as the password, *not* its hex text.

use std::collections::HashMap;

use aes::Aes256;
use anyhow::{anyhow, bail, Result};
use cbc::cipher::{block_padding::NoPadding, BlockDecryptMut, KeyIvInit};
use hmac::{Hmac, Mac};
use sha2::Sha512;

pub const PAGE_SIZE: usize = 4096;
const SALT_LEN: usize = 16;
const IV_LEN: usize = 16;
const HMAC_LEN: usize = 64;
const RESERVE: usize = IV_LEN + HMAC_LEN;
const KDF_ITER: u32 = 256_000;
const SQLITE_MAGIC: &[u8; 16] = b"SQLite format 3\0";

type Aes256CbcDec = cbc::Decryptor<Aes256>;
type HmacSha512 = Hmac<Sha512>;

/// Parse the 64-hex-character database key printed by `weflow key db`.
pub fn parse_key(hex_key: &str) -> Result<[u8; 32]> {
    let hex_key = hex_key.trim();
    if hex_key.len() != 64 {
        bail!("database key must be 64 hex characters (got {})", hex_key.len());
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex_key[i * 2..i * 2 + 2], 16)
            .map_err(|_| anyhow!("database key contains a non-hex character"))?;
    }
    Ok(out)
}

/// Keys derived for one database file (each file has its own salt).
pub struct PageCipher {
    enc_key: [u8; 32],
    mac_key: [u8; 32],
}

impl PageCipher {
    pub fn derive(raw_key: &[u8; 32], salt: &[u8]) -> Self {
        let mut enc_key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha512>(raw_key, salt, KDF_ITER, &mut enc_key);
        let mac_salt: Vec<u8> = salt.iter().map(|b| b ^ 0x3a).collect();
        let mut mac_key = [0u8; 32];
        pbkdf2::pbkdf2_hmac::<Sha512>(&enc_key, &mac_salt, 2, &mut mac_key);
        Self { enc_key, mac_key }
    }

    /// Verify and decrypt one encrypted page into `out` (`PAGE_SIZE` bytes).
    /// Page 1 comes out with the standard SQLite header restored.
    pub fn decrypt_page(&self, pgno: u32, page: &[u8], out: &mut [u8]) -> Result<()> {
        debug_assert_eq!(page.len(), PAGE_SIZE);
        debug_assert_eq!(out.len(), PAGE_SIZE);
        // A page that was never written is all zeros and is stored as such.
        if page.iter().all(|b| *b == 0) {
            out.fill(0);
            return Ok(());
        }
        let start = if pgno == 1 { SALT_LEN } else { 0 };
        let body_end = PAGE_SIZE - RESERVE;
        let iv = &page[body_end..body_end + IV_LEN];
        let stored_mac = &page[body_end + IV_LEN..];

        let mut mac = <HmacSha512 as Mac>::new_from_slice(&self.mac_key).expect("hmac accepts any key length");
        mac.update(&page[start..body_end + IV_LEN]);
        mac.update(&pgno.to_le_bytes());
        mac.verify_slice(stored_mac)
            .map_err(|_| anyhow!("HMAC check failed for page {pgno} (wrong key, or the page is corrupt)"))?;

        out.fill(0);
        if pgno == 1 {
            out[..SALT_LEN].copy_from_slice(SQLITE_MAGIC);
        }
        let body = &mut out[start..body_end];
        body.copy_from_slice(&page[start..body_end]);
        Aes256CbcDec::new(&self.enc_key.into(), iv.into())
            .decrypt_padded_mut::<NoPadding>(body)
            .map_err(|_| anyhow!("AES decrypt failed for page {pgno}"))?;
        // Keep the reserve area (IV/HMAC slot) zeroed: SQLite treats it as unused.
        Ok(())
    }
}

/// Cheap key check: derive the keys and verify page 1 of `db`.
pub fn verify_key(db: &[u8], raw_key: &[u8; 32]) -> Result<PageCipher> {
    if db.len() < PAGE_SIZE {
        bail!("file is smaller than one page ({} bytes)", db.len());
    }
    if &db[..SQLITE_MAGIC.len()] == SQLITE_MAGIC {
        bail!("file is a plain (unencrypted) SQLite database");
    }
    let cipher = PageCipher::derive(raw_key, &db[..SALT_LEN]);
    let mut scratch = vec![0u8; PAGE_SIZE];
    cipher.decrypt_page(1, &db[..PAGE_SIZE], &mut scratch)?;
    Ok(cipher)
}

/// One committed WAL frame worth applying: (page number, byte offset of its page data in the WAL).
struct WalFrame {
    pgno: u32,
    data_offset: usize,
}

fn be32(b: &[u8]) -> u32 {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

fn wal_checksum(data: &[u8], big_endian: bool, mut s0: u32, mut s1: u32) -> (u32, u32) {
    for chunk in data.chunks_exact(8) {
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

/// Parse a WAL and return the frames of its last valid *committed* prefix, plus the database
/// size in pages after the last commit. Returns `None` when the WAL holds nothing usable.
fn committed_wal_frames(wal: &[u8]) -> Option<(Vec<WalFrame>, u32)> {
    const WAL_HEADER: usize = 32;
    const FRAME_HEADER: usize = 24;
    if wal.len() < WAL_HEADER {
        return None;
    }
    let magic = be32(&wal[0..4]);
    if magic != 0x377f0682 && magic != 0x377f0683 {
        return None;
    }
    let big_endian = magic & 1 == 1;
    if be32(&wal[8..12]) as usize != PAGE_SIZE {
        return None;
    }
    let (h0, h1) = wal_checksum(&wal[..24], big_endian, 0, 0);
    if h0 != be32(&wal[24..28]) || h1 != be32(&wal[28..32]) {
        return None;
    }
    let salt = (&wal[16..20], &wal[20..24]);
    let (mut s0, mut s1) = (h0, h1);

    let frame_size = FRAME_HEADER + PAGE_SIZE;
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
        let (c0, c1) = wal_checksum(&wal[pos + FRAME_HEADER..pos + frame_size], big_endian, c0, c1);
        if c0 != be32(&fh[16..20]) || c1 != be32(&fh[20..24]) {
            break;
        }
        (s0, s1) = (c0, c1);
        let pgno = be32(&fh[0..4]);
        if pgno == 0 {
            break;
        }
        pending.push(WalFrame { pgno, data_offset: pos + FRAME_HEADER });
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

/// What a decryption will produce: sizes, and the committed WAL frames that override main-file pages.
pub struct Plan {
    main_pages: u32,
    wal_frames: Option<(Vec<WalFrame>, u32)>,
    /// Pages of the plain image (main file, or the size after the last committed WAL transaction).
    pub total_pages: u32,
}

impl Plan {
    /// Size in bytes of the plain image.
    pub fn plain_len(&self) -> usize {
        self.total_pages as usize * PAGE_SIZE
    }
}

/// Work out the plain image size for a main file of `main_len` bytes and its WAL (when present).
pub fn plan(main_len: u64, wal: Option<&[u8]>) -> Plan {
    let main_pages = (main_len / PAGE_SIZE as u64) as u32;
    let wal_frames = wal.and_then(committed_wal_frames);
    let total_pages = match &wal_frames {
        Some((_, n)) => (*n).max(1),
        None => main_pages,
    };
    Plan { main_pages, wal_frames, total_pages }
}

/// Decrypt the main file (read sequentially from `reader`, one page at a time) and apply the WAL, writing the
/// plain image straight into `out` (`plan.plain_len()` bytes). No whole-file buffer is needed.
///
/// The image is patched to rollback-journal mode so it opens as a stand-alone snapshot.
pub fn decrypt_into(out: &mut [u8], mut reader: impl std::io::Read, plan: &Plan, wal: Option<&[u8]>, cipher: &PageCipher) -> Result<()> {
    assert_eq!(out.len(), plan.plain_len(), "output buffer must match the plan");
    out.fill(0);
    let mut page = vec![0u8; PAGE_SIZE];
    for pgno in 1..=plan.main_pages {
        reader.read_exact(&mut page)?;
        if pgno <= plan.total_pages {
            let dst = &mut out[(pgno as usize - 1) * PAGE_SIZE..pgno as usize * PAGE_SIZE];
            cipher.decrypt_page(pgno, &page, dst)?;
        }
    }
    if let (Some((frames, _)), Some(wal)) = (&plan.wal_frames, wal) {
        // Later frames override earlier ones, so let the last write for each page win.
        let mut last: HashMap<u32, usize> = HashMap::new();
        for f in frames {
            last.insert(f.pgno, f.data_offset);
        }
        for (pgno, off) in last {
            if pgno > plan.total_pages {
                continue;
            }
            let dst = &mut out[(pgno as usize - 1) * PAGE_SIZE..pgno as usize * PAGE_SIZE];
            cipher.decrypt_page(pgno, &wal[off..off + PAGE_SIZE], dst)?;
        }
    }

    // Page 1 header: rewrite the page count and downgrade WAL mode (bytes 18/19) to legacy so the
    // in-memory snapshot needs no -wal file.
    out[28..32].copy_from_slice(&plan.total_pages.to_be_bytes());
    let change_counter: [u8; 4] = out[24..28].try_into().expect("4 bytes");
    out[92..96].copy_from_slice(&change_counter); // version-valid-for: makes the header page count authoritative
    out[18] = 1;
    out[19] = 1;
    Ok(())
}

/// Decrypt a whole database held in memory (plus its WAL, when given) into a plain SQLite image.
pub fn decrypt_database(db: &[u8], wal: Option<&[u8]>, cipher: &PageCipher) -> Result<Vec<u8>> {
    let plan = plan(db.len() as u64, wal);
    let mut plain = vec![0u8; plan.plain_len()];
    decrypt_into(&mut plain, std::io::Cursor::new(db), &plan, wal, cipher)?;
    Ok(plain)
}

#[cfg(any(test, feature = "test-fixtures"))]
pub mod testutil {
    use super::*;
    use aes::cipher::BlockEncryptMut;
    use rusqlite::Connection;

    type Aes256CbcEnc = cbc::Encryptor<Aes256>;

    pub const KEY: [u8; 32] = [7u8; 32];
    pub const SALT: [u8; 16] = *b"0123456789abcdef";

    /// Encrypt one plaintext page exactly as SQLCipher 4 does (test-only mirror of `decrypt_page`).
    pub fn encrypt_page(cipher: &PageCipher, pgno: u32, plain: &[u8], iv: [u8; 16]) -> Vec<u8> {
        let start = if pgno == 1 { SALT_LEN } else { 0 };
        let body_end = PAGE_SIZE - RESERVE;
        let mut page = vec![0u8; PAGE_SIZE];
        let mut body = plain[start..body_end].to_vec();
        let len = body.len();
        Aes256CbcEnc::new(&cipher.enc_key.into(), &iv.into())
            .encrypt_padded_mut::<NoPadding>(&mut body, len)
            .unwrap();
        page[start..body_end].copy_from_slice(&body);
        if pgno == 1 {
            page[..SALT_LEN].copy_from_slice(&SALT);
        }
        page[body_end..body_end + IV_LEN].copy_from_slice(&iv);
        let mut mac = <HmacSha512 as Mac>::new_from_slice(&cipher.mac_key).unwrap();
        mac.update(&page[start..body_end + IV_LEN]);
        mac.update(&pgno.to_le_bytes());
        page[body_end + IV_LEN..].copy_from_slice(&mac.finalize().into_bytes());
        page
    }

    /// A plain SQLite image whose pages leave the 80-byte reserve free; `setup` creates the schema/rows.
    pub fn plain_db_with(setup: impl FnOnce(&Connection)) -> Vec<u8> {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "page_size", 4096).unwrap();
        let mut reserve: std::os::raw::c_int = RESERVE as _;
        // SQLITE_FCNTL_RESERVE_BYTES = 38; only legal while the database is still empty.
        let rc = unsafe {
            rusqlite::ffi::sqlite3_file_control(conn.handle(), c"main".as_ptr(), 38, (&mut reserve as *mut std::os::raw::c_int).cast())
        };
        assert_eq!(rc, 0);
        setup(&conn);
        let data = conn.serialize(rusqlite::MAIN_DB).unwrap();
        assert_eq!(data[20] as usize, RESERVE);
        data.to_vec()
    }

    pub fn plain_db(rows: &[&str]) -> Vec<u8> {
        plain_db_with(|conn| {
            conn.execute_batch("create table t(id integer primary key, v text)").unwrap();
            for (i, r) in rows.iter().enumerate() {
                conn.execute("insert into t values (?1, ?2)", rusqlite::params![i as i64, r]).unwrap();
            }
        })
    }

    pub fn encrypt_db(plain: &[u8], cipher: &PageCipher) -> Vec<u8> {
        let mut out = Vec::new();
        for (i, page) in plain.chunks(PAGE_SIZE).enumerate() {
            out.extend(encrypt_page(cipher, i as u32 + 1, page, [(i as u8).wrapping_add(1); 16]));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::*;
    use super::*;
    use rusqlite::Connection;

    fn rows_of(image: Vec<u8>) -> Vec<String> {
        let mut conn = Connection::open_in_memory().unwrap();
        let n = image.len();
        conn.deserialize_read_exact(rusqlite::MAIN_DB, &image[..], n, true).unwrap();
        let mut stmt = conn.prepare("select v from t order by id").unwrap();
        stmt.query_map([], |r| r.get::<_, String>(0)).unwrap().map(|r| r.unwrap()).collect()
    }

    #[test]
    fn roundtrip_decrypts_and_queries() {
        let cipher = PageCipher::derive(&KEY, &SALT);
        let enc = encrypt_db(&plain_db(&["alpha", "beta"]), &cipher);
        let derived = verify_key(&enc, &KEY).unwrap();
        assert_eq!(rows_of(decrypt_database(&enc, None, &derived).unwrap()), ["alpha", "beta"]);
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

    /// Append one WAL frame carrying the already-encrypted `page`; returns the running checksum.
    fn push_frame(wal: &mut Vec<u8>, sum: (u32, u32), pgno: u32, commit: u32, salts: (u32, u32), page: &[u8]) -> (u32, u32) {
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

        // Same schema, different row: page 2 (the table's root leaf) is what changes.
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
            sum = push_frame(&mut wal, sum, pgno, commit, salts, &encrypt_page(&cipher, pgno, pg, [0x55; 16]));
        }
        // A later transaction that never committed (commit == 0): must not be applied.
        let pg = &newest[..PAGE_SIZE];
        push_frame(&mut wal, sum, 1, 0, salts, &encrypt_page(&cipher, 1, pg, [0x66; 16]));

        assert_eq!(rows_of(decrypt_database(&enc_main, None, &derived).unwrap()), ["old"]);
        assert_eq!(rows_of(decrypt_database(&enc_main, Some(&wal), &derived).unwrap()), ["new"]);
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
        // Frame written under a different salt pair (leftover from before a WAL reset).
        push_frame(&mut wal, sum, 1, 1, (9, 9), &encrypt_page(&cipher, 1, pg, [3; 16]));
        assert_eq!(rows_of(decrypt_database(&enc, Some(&wal), &derived).unwrap()), ["kept"]);
    }
}
