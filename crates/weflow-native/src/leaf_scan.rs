//! Reads of a few integer columns of every row of a table that decrypt only the bytes they read.
//!
//! A query that needs one column of every message (the dates of a conversation) makes SQLite read every leaf page of
//! the table and decrypt all of its 4 KiB, while the cells' first few dozen bytes hold all the query reads. Here the
//! table's b-tree is walked directly: interior pages are decrypted and checked as SQLite reads them; of each leaf page
//! the HMAC is checked over the whole page, as SQLite's reads do, so a page damaged on disk or torn by a write fails
//! the read the same way, but only the 16-byte blocks of its header, cell pointers and the start of each cell's
//! record are decrypted (CBC decrypts a block from it and the block before), and the leaves are shared out between
//! threads.
//!
//! A row whose wanted cells are not integers or nulls, a record longer than the wanted cells allow in the page, or a
//! page of a kind a rowid table does not have, ends the scan as unusual: the caller then reads the table through SQL.

use anyhow::{bail, Result};

use crate::cipher_vfs::CipherFile;
use crate::sqlcipher::{PageCipher, PAGE_SIZE, RESERVE};

/// Bytes of a page that hold b-tree content (the rest is the IV and HMAC).
const USABLE: usize = PAGE_SIZE - RESERVE;
const BLOCK: usize = 16;
const LEAF_TABLE: u8 = 13;
const INTERIOR_TABLE: u8 = 5;

#[cfg(test)]
thread_local! {
    /// Set by a test to read every table through SQL, as for a table the scan does not read.
    pub(crate) static SQL_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// The root page of table `table` and the places of its `columns` in a record, for [`int_columns`]; `None` for a
/// table it does not read: one without a rowid, or without one of the columns.
pub(crate) fn target(
    conn: &rusqlite::Connection,
    table: &str,
    columns: &[&str],
) -> Result<Option<(u32, Vec<usize>)>> {
    #[cfg(test)]
    if SQL_ONLY.with(std::cell::Cell::get) {
        return Ok(None);
    }
    let one =
        |sql: &str, params: &[&dyn rusqlite::ToSql]| -> Result<Option<(i64, Option<String>)>> {
            let mut stmt = conn.prepare(sql)?;
            let mut rows = stmt.query(params)?;
            Ok(match rows.next()? {
                Some(r) => Some((r.get(0)?, r.get(1)?)),
                None => None,
            })
        };
    let Some((root, sql)) = one(
        "select rootpage, sql from sqlite_master where type = 'table' and name = ?1",
        &[&table],
    )?
    else {
        return Ok(None);
    };
    let without_rowid = sql.is_some_and(|sql| {
        sql.to_ascii_lowercase()
            .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
            .filter(|w| !w.is_empty())
            .collect::<Vec<_>>()
            .windows(2)
            .any(|w| w == ["without", "rowid"])
    });
    let Ok(root) = u32::try_from(root) else {
        return Ok(None);
    };
    if without_rowid {
        return Ok(None);
    }
    let mut places = Vec::with_capacity(columns.len());
    for column in columns {
        let place = one(
            "select cid, null from pragma_table_info(?1) where name = ?2",
            &[&table, column],
        )?;
        match place.and_then(|(cid, _)| usize::try_from(cid).ok()) {
            Some(cid) => places.push(cid),
            None => return Ok(None),
        }
    }
    Ok(Some((root, places)))
}

/// Whether a part of a scan read every row of its pages, or stopped at something it does not read (see the module
/// docs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scan {
    Done,
    Unusual,
}

/// Leaf pages a thread of a scan takes at least.
const PAGES_PER_PART: usize = 512;
/// Most threads a scan uses: the HMAC of every page is most of its work.
const MAX_PARTS: usize = 8;

/// The cells `columns` (indexes into the record, in any order) of every row of the rowid table whose b-tree starts at
/// page `root` of `file`, `None` for a null: each part of the leaf pages, read by a thread of its own, starts an
/// accumulator with `start` and hands it every row of its pages with `each`, the cells in the order of `columns`.
/// The accumulators come back in no set order; `None` when the scan met something it does not read.
pub(crate) fn int_columns<A: Send>(
    file: &CipherFile,
    root: u32,
    columns: &[usize],
    start: impl Fn() -> A + Sync,
    each: impl Fn(&mut A, &[Option<i64>]) + Sync,
) -> Result<Option<Vec<A>>> {
    let Some(&last) = columns.iter().max() else {
        bail!("no column asked for");
    };
    let Some(leaves) = leaf_pages(file, root)? else {
        return Ok(None);
    };
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let parts = (leaves.len() / PAGES_PER_PART).clamp(1, MAX_PARTS.min(cpus));
    let per_part = leaves.len().div_ceil(parts).max(1);
    let read = |pages: &[u32]| -> Result<Option<A>> {
        let mut acc = start();
        let mut page = Lazy::new(file.page_cipher());
        let mut values = vec![None; columns.len()];
        let mut serials: Vec<u64> = Vec::with_capacity(last + 1);
        for &pgno in pages {
            let scan = file
                .with_checked_page(pgno, |raw| -> Result<Scan> {
                    page.start(raw);
                    if page.bytes(0, 1)?[0] != LEAF_TABLE {
                        return Ok(Scan::Unusual);
                    }
                    leaf_rows(
                        &mut page,
                        columns,
                        last,
                        &mut serials,
                        &mut values,
                        &mut |v| each(&mut acc, v),
                    )
                })?
                .transpose()?;
            if scan != Some(Scan::Done) {
                return Ok(None);
            }
        }
        Ok(Some(acc))
    };
    let parts: Vec<Result<Option<A>>> = if parts == 1 {
        vec![read(&leaves)]
    } else {
        std::thread::scope(|s| {
            let running: Vec<_> = leaves
                .chunks(per_part)
                .map(|pages| s.spawn(move || read(pages)))
                .collect();
            running
                .into_iter()
                .map(|h| {
                    h.join()
                        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
                })
                .collect()
        })
    };
    let mut out = Vec::with_capacity(parts.len());
    for part in parts {
        match part? {
            Some(acc) => out.push(acc),
            None => return Ok(None),
        }
    }
    Ok(Some(out))
}

/// The leaf pages of the table b-tree at `root`, in order, from its interior pages (decrypted and checked whole);
/// `None` when it holds a page a rowid table does not have. Every leaf of a b-tree is at the same depth, so the
/// children of an interior page are leaves when its first child is one.
fn leaf_pages(file: &CipherFile, root: u32) -> Result<Option<Vec<u32>>> {
    let mut plain = vec![0u8; PAGE_SIZE];
    let mut level = vec![root];
    // a b-tree is a tree: more pages than the file has means a loop
    let mut budget = file.page_count() as usize;
    loop {
        if level.iter().any(|&p| p <= 1) || level.len() > budget {
            return Ok(None);
        }
        budget -= level.len();
        let Some(first) = file.with_checked_page(level[0], |raw| raw.to_vec())? else {
            return Ok(None);
        };
        let mut head = Lazy::new(file.page_cipher());
        head.start(&first);
        match head.bytes(0, 1)?[0] {
            LEAF_TABLE => return Ok(Some(level)),
            INTERIOR_TABLE => {}
            _ => return Ok(None),
        }
        let mut next = Vec::new();
        for &pgno in &level {
            file.page(pgno, &mut plain)?;
            if plain[0] != INTERIOR_TABLE {
                return Ok(None);
            }
            let cells = usize::from(u16::from_be_bytes([plain[3], plain[4]]));
            let Some(pointers) = plain.get(12..12 + 2 * cells) else {
                return Ok(None);
            };
            for p in pointers.as_chunks::<2>().0 {
                let at = usize::from(u16::from_be_bytes(*p));
                let Some(child) = plain.get(at..at + 4) else {
                    return Ok(None);
                };
                next.push(u32::from_be_bytes([child[0], child[1], child[2], child[3]]));
            }
            next.push(u32::from_be_bytes([
                plain[8], plain[9], plain[10], plain[11],
            ]));
        }
        level = next;
    }
}

/// The rows of one leaf page.
fn leaf_rows(
    page: &mut Lazy,
    columns: &[usize],
    last: usize,
    serials: &mut Vec<u64>,
    values: &mut [Option<i64>],
    each: &mut impl FnMut(&[Option<i64>]),
) -> Result<Scan> {
    let header = page.bytes(0, 8)?;
    let cells = usize::from(u16::from_be_bytes([header[3], header[4]]));
    let pointers: Vec<usize> = page
        .bytes(8, 8 + 2 * cells)?
        .as_chunks::<2>()
        .0
        .iter()
        .map(|p| usize::from(u16::from_be_bytes(*p)))
        .collect();
    for at in pointers {
        let (payload, n) = page.varint(at)?;
        let (_rowid, m) = page.varint(at + n)?;
        let record = at + n + m;
        let local = local_payload(payload);
        let (header_len, k) = page.varint(record)?;
        let Ok(header_len) = usize::try_from(header_len) else {
            return Ok(Scan::Unusual);
        };
        if header_len > local || header_len < k {
            return Ok(Scan::Unusual);
        }
        // the serial types of the cells up to the last one wanted
        serials.clear();
        let mut pos = record + k;
        while serials.len() <= last && pos < record + header_len {
            let (serial, n) = page.varint(pos)?;
            serials.push(serial);
            pos += n;
        }
        if serials.len() <= last {
            // a row written before the column was added holds its default, which the record does not say
            return Ok(Scan::Unusual);
        }
        let mut offsets = Vec::with_capacity(serials.len());
        let mut offset = record + header_len;
        for &serial in serials.iter() {
            offsets.push(offset);
            let Some(size) = serial_size(serial) else {
                return Ok(Scan::Unusual);
            };
            offset += size;
        }
        for (slot, &c) in values.iter_mut().zip(columns) {
            let (serial, at) = (serials[c], offsets[c]);
            *slot = match serial {
                0 => None,
                8 => Some(0),
                9 => Some(1),
                1..=6 => {
                    let size = serial_size(serial).unwrap_or(0);
                    if at + size - record > local {
                        return Ok(Scan::Unusual);
                    }
                    let bytes = page.bytes(at, at + size)?;
                    // big-endian two's complement of `size` bytes, sign-extended
                    let mut v = i64::from(bytes[0] as i8);
                    for &b in &bytes[1..] {
                        v = (v << 8) | i64::from(b);
                    }
                    Some(v)
                }
                _ => return Ok(Scan::Unusual),
            };
        }
        each(values);
    }
    Ok(Scan::Done)
}

/// Bytes of a table leaf cell's payload stored in the page itself (the rest goes to overflow pages), as SQLite
/// computes it.
fn local_payload(payload: u64) -> usize {
    const MAX: u64 = USABLE as u64 - 35;
    const MIN: u64 = (USABLE as u64 - 12) * 32 / 255 - 23;
    let local = if payload <= MAX {
        payload
    } else {
        let k = MIN + (payload - MIN) % (USABLE as u64 - 4);
        if k <= MAX {
            k
        } else {
            MIN
        }
    };
    // at most `MAX`, below a page
    usize::try_from(local).unwrap_or(0)
}

/// The size in the record of a cell of serial type `serial`; `None` for the reserved types.
fn serial_size(serial: u64) -> Option<usize> {
    Some(match serial {
        0 | 8 | 9 => 0,
        1 => 1,
        2 => 2,
        3 => 3,
        4 => 4,
        5 => 6,
        6 | 7 => 8,
        10 | 11 => return None,
        n => usize::try_from((n - 12) / 2).ok()?,
    })
}

/// A leaf page whose blocks are decrypted when first read.
struct Lazy<'c> {
    cipher: &'c PageCipher,
    raw: Vec<u8>,
    plain: Vec<u8>,
    /// Which 16-byte blocks of `plain` are decrypted.
    done: [u64; 4],
}

impl<'c> Lazy<'c> {
    fn new(cipher: &'c PageCipher) -> Self {
        Self {
            cipher,
            raw: vec![0; PAGE_SIZE],
            plain: vec![0; PAGE_SIZE],
            done: [0; 4],
        }
    }

    fn start(&mut self, raw: &[u8]) {
        self.raw.copy_from_slice(raw);
        self.done = [0; 4];
    }

    fn has(&self, block: usize) -> bool {
        self.done[block / 64] & (1 << (block % 64)) != 0
    }

    /// Bytes `from..to` of the page, decrypted.
    fn bytes(&mut self, from: usize, to: usize) -> Result<&[u8]> {
        if from > to || to > USABLE {
            bail!("a cell runs past the page ({from}..{to})");
        }
        let (mut block, end) = (from / BLOCK, to.div_ceil(BLOCK));
        while block < end {
            if self.has(block) {
                block += 1;
                continue;
            }
            let run = block;
            while block < end && !self.has(block) {
                self.done[block / 64] |= 1 << (block % 64);
                block += 1;
            }
            self.cipher
                .decrypt_blocks(&self.raw, run..block, &mut self.plain)?;
        }
        Ok(&self.plain[from..to])
    }

    /// The varint at `at` and its length.
    fn varint(&mut self, at: usize) -> Result<(u64, usize)> {
        let bytes = self.bytes(at, (at + 9).min(USABLE))?;
        let mut v = 0u64;
        for (i, &b) in bytes.iter().enumerate() {
            if i == 8 {
                return Ok(((v << 8) | u64::from(b), 9));
            }
            v = (v << 7) | u64::from(b & 0x7f);
            if b & 0x80 == 0 {
                return Ok((v, i + 1));
            }
        }
        bail!("a varint runs past the page")
    }
}
