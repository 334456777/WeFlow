//! 读取表中每一行的几个整数列，只解密读到的字节。
//!
//! 需要每条消息某一列的查询（如一个会话有消息的日期）会让 SQLite 读取表的每个叶子页并解密整整 4 KiB，
//! 而查询要读的内容全在各单元开头的几十个字节里。这里直接遍历表的 B 树：内部页与 SQLite 读取时一样完整解密并校验；
//! 叶子页与 SQLite 的读取一样校验整页的 HMAC，所以在磁盘上损坏或被写入撕裂的页同样会让读取失败，
//! 但只解密页头、单元指针和每个单元记录开头所在的 16 字节块（CBC 解密一个块只需要它自身和前一个块），
//! 叶子页分给多个线程处理。
//!
//! 某一行要读的单元不是整数或空值、超出了记录存放在页内的部分，或者遇到 rowid 表不会有的页类型时，
//! 扫描以「不寻常」结束：调用方改用 SQL 读这张表。

use std::ops::ControlFlow;

use anyhow::{bail, Result};

use crate::cipher_vfs::CipherFile;
use crate::sqlcipher::{PageCipher, PAGE_SIZE, RESERVE};

/// 一页中存放 B 树内容的字节数（其余是 IV 和 HMAC）。
const USABLE: usize = PAGE_SIZE - RESERVE;
const BLOCK: usize = 16;
const LEAF_TABLE: u8 = 13;
const INTERIOR_TABLE: u8 = 5;

#[cfg(test)]
thread_local! {
    /// 测试设置它后，每张表都通过 SQL 读取，与扫描不读的表一样。
    pub(crate) static SQL_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// 表 `table` 的根页，以及 `columns` 在记录中的位置，供 [`int_columns`] 使用；扫描不读的表为 `None`：没有 rowid
/// 的表，或者缺少其中某一列的表。
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

/// 扫描的一部分是读完了它的页中的每一行，还是停在了它不读的内容上（见模块文档）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scan {
    Done,
    Unusual,
}

/// 扫描时每个线程至少处理的叶子页数。
const PAGES_PER_PART: usize = 512;
/// 扫描最多使用的线程数：每页的 HMAC 是它的主要工作量。
const MAX_PARTS: usize = 8;
/// 一次校验 HMAC 的叶子页数，是 CPU 并行计算的哈希数（[`crate::sha512x4`]）的倍数。
const CHECKED_TOGETHER: usize = 4 * crate::sha512x4::LANES;

/// 读取 B 树从 `file` 的第 `root` 页开始的 rowid 表中每一行的 `columns` 单元（记录中的序号，顺序不限），
/// 空值为 `None`：叶子页分成几部分，各由一个线程读取，每部分用 `start` 建一个累加器，再用 `each`
/// 把这部分页中的每一行交给它，单元按 `columns` 的顺序排列。累加器返回的顺序不定；
/// 扫描遇到不读的内容时为 `None`。
pub(crate) fn int_columns<A: Send>(
    file: &CipherFile,
    root: u32,
    columns: &[usize],
    start: impl Fn() -> A + Sync,
    each: impl Fn(&mut A, &[Option<i64>]) + Sync,
) -> Result<Option<Vec<A>>> {
    rows(file, root, columns, None, start, |acc, row| {
        each(acc, row.ints);
    })
}

/// [`rows`] 交出的文本单元。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Text<'a> {
    /// 没有要求读文本单元，或者单元为空值。
    Null,
    /// 不超过要求的长度、存放在页内且是合法 UTF-8 的文本。
    Short(&'a str),
    /// 其他情况：更长的文本、延续到溢出页或不是 UTF-8 的文本，或者其他类型的单元。
    Other,
}

/// [`rows`] 交出的一行。
pub(crate) struct Row<'a> {
    pub(crate) rowid: i64,
    /// 要求读的整数单元，按要求的顺序排列；空值为 `None`。
    pub(crate) ints: &'a [Option<i64>],
    pub(crate) text: Text<'a>,
}

/// 同 [`int_columns`]，另外交出每一行的 rowid；`text` 给出一列和一个字节长度时，还把这一列的单元作为 [`Text`]
/// 交出：只解密不超过这个长度的文本。
pub(crate) fn rows<A: Send>(
    file: &CipherFile,
    root: u32,
    columns: &[usize],
    text: Option<(usize, usize)>,
    start: impl Fn() -> A + Sync,
    each: impl Fn(&mut A, Row<'_>) + Sync,
) -> Result<Option<Vec<A>>> {
    let Some(last) = columns.iter().copied().chain(text.map(|t| t.0)).max() else {
        bail!("no column asked for");
    };
    let Some(leaves) = leaf_pages(file, root)? else {
        return Ok(None);
    };
    let cpus = std::thread::available_parallelism().map_or(1, usize::from);
    let parts = (leaves.len() / PAGES_PER_PART).clamp(1, MAX_PARTS.min(cpus));
    let per_part = leaves.len().div_ceil(parts).max(1);
    let wanted = Wanted {
        columns,
        text,
        last,
    };
    let read = |pages: &[u32]| -> Result<Option<A>> {
        let mut acc = start();
        let mut page = Lazy::new(file.page_cipher());
        let mut values = vec![None; columns.len()];
        let mut serials: Vec<u64> = Vec::with_capacity(last + 1);
        // 一次校验几页的 HMAC（`with_checked_pages`）
        for group in pages.chunks(CHECKED_TOGETHER) {
            let scan = file.with_checked_pages(group, |_, raw| {
                // 从未写入的页不是叶子页
                let Some(raw) = raw else {
                    return ControlFlow::Break(Ok(Scan::Unusual));
                };
                page.start(raw);
                let scan = match page.bytes(0, 1) {
                    Ok(kind) if kind[0] != LEAF_TABLE => Ok(Scan::Unusual),
                    Ok(_) => leaf_rows(&mut page, &wanted, &mut serials, &mut values, &mut |row| {
                        each(&mut acc, row);
                    }),
                    Err(e) => Err(e),
                };
                match scan {
                    Ok(Scan::Done) => ControlFlow::Continue(()),
                    other => ControlFlow::Break(other),
                }
            })?;
            if let ControlFlow::Break(scan) = scan {
                scan?;
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

/// 从内部页（完整解密并校验）按顺序得到以 `root` 为根的表 B 树的叶子页；树中有 rowid 表不会有的页时为 `None`。
/// B 树的所有叶子页深度相同，所以内部页的第一个子页是叶子页时，它的所有子页都是叶子页。
fn leaf_pages(file: &CipherFile, root: u32) -> Result<Option<Vec<u32>>> {
    let mut plain = vec![0u8; PAGE_SIZE];
    let mut level = vec![root];
    // B 树是一棵树：页数超过文件的总页数说明有环
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

/// 扫描从每一行读取的内容。
struct Wanted<'a> {
    columns: &'a [usize],
    /// 文本列，以及解密的文本的最大长度。
    text: Option<(usize, usize)>,
    /// `columns` 和文本列中最靠后的一列。
    last: usize,
}

/// 读取一个叶子页中的各行。
fn leaf_rows(
    page: &mut Lazy<'_>,
    wanted: &Wanted<'_>,
    serials: &mut Vec<u64>,
    values: &mut [Option<i64>],
    each: &mut impl FnMut(Row<'_>),
) -> Result<Scan> {
    let last = wanted.last;
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
        let (rowid, m) = page.varint(at + n)?;
        // rowid 以 i64 的补码位形式保存
        let rowid = i64::from_ne_bytes(rowid.to_ne_bytes());
        let record = at + n + m;
        let local = local_payload(payload);
        let (header_len, k) = page.varint(record)?;
        let Ok(header_len) = usize::try_from(header_len) else {
            return Ok(Scan::Unusual);
        };
        if header_len > local || header_len < k {
            return Ok(Scan::Unusual);
        }
        // 直到要读的最后一个单元为止各单元的序列类型
        serials.clear();
        let mut pos = record + k;
        while serials.len() <= last && pos < record + header_len {
            let (serial, n) = page.varint(pos)?;
            serials.push(serial);
            pos += n;
        }
        if serials.len() <= last {
            // 加上这一列之前写入的行取它的默认值，而记录中没有这个值
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
        for (slot, &c) in values.iter_mut().zip(wanted.columns) {
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
                    // `size` 字节的大端补码，做符号扩展
                    let mut v = i64::from(bytes[0].cast_signed());
                    for &b in &bytes[1..] {
                        v = (v << 8) | i64::from(b);
                    }
                    Some(v)
                }
                _ => return Ok(Scan::Unusual),
            };
        }
        let text = match wanted.text {
            None => Text::Null,
            Some((c, longest)) => match serials[c] {
                0 => Text::Null,
                serial if serial >= 13 && serial % 2 == 1 => {
                    let (size, at) = (serial_size(serial).unwrap_or(usize::MAX), offsets[c]);
                    if size > longest || at + size - record > local {
                        Text::Other
                    } else {
                        match std::str::from_utf8(page.bytes(at, at + size)?) {
                            Ok(text) => Text::Short(text),
                            Err(_) => Text::Other,
                        }
                    }
                }
                _ => Text::Other,
            },
        };
        each(Row {
            rowid,
            ints: values,
            text,
        });
    }
    Ok(Scan::Done)
}

/// 按 SQLite 的算法，表的叶子单元的负载中存放在页内的字节数（其余存放在溢出页）。
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
    // 不超过 `MAX`，小于一页
    usize::try_from(local).unwrap_or(0)
}

/// 序列类型为 `serial` 的单元在记录中占的字节数；保留的类型为 `None`。
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

/// 第一次读到某个块时才解密它的叶子页。
struct Lazy<'c> {
    cipher: &'c PageCipher,
    raw: Vec<u8>,
    plain: Vec<u8>,
    /// `plain` 中哪些 16 字节块已经解密。
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

    /// 页中解密后的 `from..to` 字节。
    fn bytes(&mut self, from: usize, to: usize) -> Result<&[u8]> {
        if from > to || to > USABLE {
            bail!("a cell runs past the page: bytes {from}..{to}");
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

    /// `at` 处的 varint 和它的长度。
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
