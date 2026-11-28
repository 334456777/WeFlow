//! 会话的 `Msg_<md5>` 表中的行（#257、#490），读成 [`MsgRow`]。
//!
//! [`msg`](crate::msg) 的分页、游标和合并代码读到的行：各列已按类型取出，内容只解压、解码一次，发送者已换成用户名，
//! 所在的数据库用分片号表示（#491、#492）。

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::types::ValueRef;
use rusqlite::Row;

use crate::msg::{shard_of, MsgPos, MsgTable};

/// 查询读取消息行的哪些列；其余的列读出来为空。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Columns {
    /// 全部的列。
    All,
    /// 除 `source` 以外的列，导出用不到 `source`。
    NoSource,
    /// 不含 `source` 和 `packed_info_data`，批量扫描不读这两列。
    Lite,
}

/// 整数列（`local_id`、`create_time`、`sort_seq` 等）里存了其他类型的单元格的个数，这些单元格读作缺失。
/// 微信不会写出这样的单元格；计数大于零说明数据库与读取的代码预期的不一样。
static ODD_CELLS: AtomicU64 = AtomicU64::new(0);

/// 程序启动以来，整数列中存了其他类型的单元格的个数（见 [`MsgRow`]）。
pub fn odd_cell_count() -> u64 {
    ODD_CELLS.load(Ordering::Relaxed)
}

/// 读一行时除了各列之外还要用到的信息。
pub(crate) struct RowContext<'a> {
    pub(crate) table: &'a MsgTable,
    /// 判断一个发送者是不是账号所有者。
    pub(crate) is_me: &'a dyn Fn(&str) -> bool,
}

fn sort_key(sort_seq: i64, create_time: i64, local_id: i64) -> (i64, i64, i64) {
    // 换算成毫秒会溢出的时间饱和到它所在的一端，与 SQLite 溢出成 REAL 后的排序一致
    let seq = if sort_seq == 0 {
        create_time.saturating_mul(1000)
    } else {
        sort_seq
    };
    (seq, create_time, local_id)
}

/// 一条消息行（见模块的说明）。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MsgRow {
    /// 本地 ID：行在所在表中的 ID，只在所在的分片内唯一，不同分片之间可能相同。
    pub local_id: i64,
    /// 服务器 ID：微信服务器给消息的 ID；行里没有时为 `None`。
    pub server_id: Option<i64>,
    /// 消息类型，app 消息的子类型在高 32 位（`(sub << 32) | 49`）。
    pub local_type: i64,
    /// 消息在聊天窗口中的排序位置；微信在有些行里不填（null 或 0）。
    pub sort_seq: Option<i64>,
    /// 发送时间，Unix 时间戳（秒）。
    pub create_time: Option<i64>,
    /// 微信的 `status` 列。
    pub status: Option<i64>,
    /// 发送者的用户名；`Name2Id` 里没有对应的行时为 `None`。
    pub sender: Option<String>,
    /// 是否由账号所有者发送。
    pub is_send: bool,
    /// `compress_content` 不为空时取它，否则取 `message_content`；是 zstd 数据时先解压，再解码成文字。
    pub content: String,
    /// `msgsource` 的 XML；按 [`Columns::NoSource`] 或 [`Columns::Lite`] 读的行不含它。
    pub source: Option<String>,
    /// 原样保存的 `packed_info_data`；按 [`Columns::Lite`] 读的行不含它。
    pub packed_info: Option<Vec<u8>>,
    /// 行所在消息库的分片号（见 [`shard_name`](crate::msg::shard_name)）。
    pub shard: u32,
}

// 每读一条消息就有一个：固定它的大小，只在有意时改变（#505）
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<MsgRow>() == 184);

const MSG_COLUMNS: &str =
    "m.local_id, m.server_id, m.local_type, m.sort_seq, m.create_time, m.status, \
     m.compress_content, m.message_content, m.source, m.packed_info_data, n.user_name";
const MSG_COLUMNS_NO_SOURCE: &str =
    "m.local_id, m.server_id, m.local_type, m.sort_seq, m.create_time, m.status, \
     m.compress_content, m.message_content, null, m.packed_info_data, n.user_name";
const MSG_COLUMNS_LITE: &str =
    "m.local_id, m.server_id, m.local_type, m.sort_seq, m.create_time, m.status, \
     m.compress_content, m.message_content, null, null, n.user_name";

fn int_cell(v: ValueRef<'_>) -> Option<i64> {
    match v {
        ValueRef::Integer(i) => Some(i),
        ValueRef::Null => None,
        ValueRef::Real(_) | ValueRef::Text(_) | ValueRef::Blob(_) => {
            ODD_CELLS.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

/// zstd 帧开头的字节（小端的 `0xFD2FB528`）。
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// zstd 帧声明的大小不超过这个值时，[`inflate`] 才按声明的大小预先分配空间。
const INFLATE_RESERVE_MAX: u64 = 16 << 20;

thread_local! {
    /// 留给下一条消息复用的解压上下文，不像 `zstd::stream::decode_all` 那样每条消息新建、释放一个上下文和读缓冲区。
    static DCTX: std::cell::RefCell<Option<zstd::zstd_safe::DCtx<'static>>> =
        const { std::cell::RefCell::new(None) };
}

/// 解压 `bytes`（以 [`ZSTD_MAGIC`] 开头）；不是有效的 zstd 数据时为 `None`。
fn inflate(bytes: &[u8]) -> Option<Vec<u8>> {
    // 输入不恰好在某一帧的结尾结束时交给 `decode_all`，由它处理错误和截断
    DCTX.with(|slot| {
        inflate_with(
            slot.borrow_mut()
                .get_or_insert_with(zstd::zstd_safe::DCtx::create),
            bytes,
        )
    })
    .or_else(|| zstd::stream::decode_all(bytes).ok())
}

/// 用 `dctx` 解压 `bytes` 的每一帧；输入不恰好在某一帧的结尾结束时为 `None`。
fn inflate_with(dctx: &mut zstd::zstd_safe::DCtx<'static>, bytes: &[u8]) -> Option<Vec<u8>> {
    use zstd::zstd_safe::{InBuffer, OutBuffer, ResetDirective};
    dctx.reset(ResetDirective::SessionOnly).ok()?;
    let reserve = zstd::zstd_safe::get_frame_content_size(bytes)
        .ok()
        .flatten()
        .filter(|&n| n <= INFLATE_RESERVE_MAX)
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(bytes.len().saturating_mul(4));
    let mut out = Vec::with_capacity(reserve.max(64));
    let mut input = InBuffer::around(bytes);
    loop {
        if out.len() == out.capacity() {
            out.reserve(out.capacity());
        }
        let (read, written) = (input.pos(), out.len());
        let hint = {
            let mut output = OutBuffer::around_pos(&mut out, written);
            dctx.decompress_stream(&mut output, &mut input).ok()?
        };
        if input.pos() == bytes.len() && hint == 0 {
            return Some(out);
        }
        if input.pos() == read && out.len() == written && out.len() < out.capacity() {
            return None; // 输出还有空间却没有进展：输入在帧的中间结束
        }
    }
}

/// 把字节读成文字：是 zstd 帧时先解压，再按 UTF-8 解码。不是有效的 UTF-8 时，
/// 无效的序列少于字符数的五分之一就删掉这些序列，否则每个字节读作一个字符（Latin-1）。
fn bytes_to_text(bytes: &[u8]) -> String {
    let inflated = if bytes.starts_with(&ZSTD_MAGIC) {
        inflate(bytes)
    } else {
        None
    };
    let inflated = match inflated.map(String::from_utf8) {
        // 解压出的缓冲区直接作为文字，不再复制
        Some(Ok(text)) => return text,
        Some(Err(e)) => Some(e.into_bytes()),
        None => None,
    };
    let bytes = inflated.as_deref().unwrap_or(bytes);
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.to_owned();
    }
    let lossy = String::from_utf8_lossy(bytes);
    let replaced = lossy.chars().filter(|c| *c == '\u{FFFD}').count();
    #[expect(clippy::cast_precision_loss, reason = "一条消息的字符数，远小于 2^53")]
    let mostly_text = (replaced as f64) < lossy.chars().count() as f64 * 0.2;
    if mostly_text {
        lossy.replace('\u{FFFD}', "")
    } else {
        bytes.iter().map(|b| char::from(*b)).collect()
    }
}

fn text_cell(v: ValueRef<'_>) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Text(t) | ValueRef::Blob(t) => Some(bytes_to_text(t)),
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(f) => Some(f.to_string()),
    }
}

impl MsgRow {
    /// 行在会话顺序中的位置，用作翻页的起点。
    pub fn pos(&self) -> MsgPos {
        let (sort_seq, create_time, local_id) = self.key();
        MsgPos {
            sort_seq,
            create_time,
            local_id,
            shard: self.shard,
        }
    }

    /// 读取 `which` 指定的列时的 select 列表：消息表为 `m`，按发送者连接的 `Name2Id` 为 `n`，不读的列写成 `null`。
    pub(crate) fn columns(which: Columns) -> &'static str {
        match which {
            Columns::All => MSG_COLUMNS,
            Columns::NoSource => MSG_COLUMNS_NO_SOURCE,
            Columns::Lite => MSG_COLUMNS_LITE,
        }
    }

    /// 读出 `row` 这一行，它的列是 [`columns`](Self::columns) 给出的列表。
    pub(crate) fn read(row: &Row<'_>, ctx: &RowContext<'_>) -> rusqlite::Result<Self> {
        let sender = text_cell(row.get_ref(10)?);
        let content = match text_cell(row.get_ref(6)?) {
            Some(c) if !c.is_empty() => c,
            _ => text_cell(row.get_ref(7)?).unwrap_or_default(),
        };
        let packed_info = match row.get_ref(9)? {
            ValueRef::Blob(b) | ValueRef::Text(b) => Some(b.to_vec()),
            _ => None,
        };
        Ok(Self {
            local_id: int_cell(row.get_ref(0)?).unwrap_or(0),
            server_id: int_cell(row.get_ref(1)?),
            local_type: int_cell(row.get_ref(2)?).unwrap_or(0),
            sort_seq: int_cell(row.get_ref(3)?),
            create_time: int_cell(row.get_ref(4)?),
            status: int_cell(row.get_ref(5)?),
            is_send: (ctx.is_me)(sender.as_deref().unwrap_or("")),
            sender,
            content,
            source: text_cell(row.get_ref(8)?),
            packed_info,
            shard: shard_of(ctx.table).unwrap_or(0),
        })
    }

    /// 聊天窗口中的排序键 `(sort_seq, create_time, local_id)`：没有 `sort_seq`（null 或 0）的行按
    /// `create_time * 1000` 排，缺失的整数算作 0。
    pub(crate) fn key(&self) -> (i64, i64, i64) {
        sort_key(
            self.sort_seq.unwrap_or(0),
            self.create_time.unwrap_or(0),
            self.local_id,
        )
    }

    /// 按原值取的 `(local_id, sort_seq)`，缺失时为 0：微信的搜索索引用它标识一条消息。
    pub(crate) fn index_key(&self) -> (i64, i64) {
        (self.local_id, self.sort_seq.unwrap_or(0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_become_text_whether_compressed_or_not() {
        assert_eq!(bytes_to_text("你好".as_bytes()), "你好");
        let packed = zstd::stream::encode_all("<msg>x</msg>".as_bytes(), 3).unwrap();
        assert!(packed.starts_with(&ZSTD_MAGIC));
        assert_eq!(bytes_to_text(&packed), "<msg>x</msg>");
        // 文字里个别的无效字节被删掉；大部分不是 UTF-8 的字节每个读作一个字符
        assert_eq!(bytes_to_text(b"abcdefghij\xff"), "abcdefghij");
        assert_eq!(bytes_to_text(&[0xff, 0xfe, 0x41]), "\u{ff}\u{fe}A");
    }

    #[test]
    fn inflating_with_a_kept_context_gives_what_streaming_gives() {
        // 参照：每一帧都经 `decode_all` 流式解压，每条消息新建一个上下文
        let streamed = |bytes: &[u8]| -> String {
            let inflated = zstd::stream::decode_all(bytes).ok();
            let bytes = inflated.as_deref().unwrap_or(bytes);
            match std::str::from_utf8(bytes) {
                Ok(s) => s.to_owned(),
                Err(_) => bytes_to_text(bytes),
            }
        };
        let text = "<msg><appmsg>引用</appmsg></msg>".repeat(50);
        let declared = zstd::bulk::compress(text.as_bytes(), 3).unwrap();
        assert!(zstd::zstd_safe::get_frame_content_size(&declared)
            .unwrap()
            .is_some());
        let undeclared = {
            let mut e = zstd::stream::Encoder::new(Vec::new(), 3).unwrap();
            e.include_contentsize(false).unwrap();
            std::io::Write::write_all(&mut e, text.as_bytes()).unwrap();
            e.finish().unwrap()
        };
        assert_eq!(
            zstd::zstd_safe::get_frame_content_size(&undeclared).unwrap(),
            None
        );
        let two_frames = [declared.clone(), declared.clone()].concat();
        let not_utf8 = zstd::bulk::compress(b"abcdefghij\xff", 3).unwrap();
        let mostly_not_utf8 = zstd::bulk::compress(&[0xff, 0xfe, 0x41], 3).unwrap();
        let corrupt = [&declared[..declared.len() / 2], b"junk"].concat();
        let truncated = undeclared[..undeclared.len() / 2].to_vec();
        let trailing = [&declared[..], b"junk"].concat();
        for bytes in [
            &declared,
            &undeclared,
            &two_frames,
            &not_utf8,
            &mostly_not_utf8,
            &corrupt,
            &truncated,
            &trailing,
        ] {
            assert_eq!(bytes_to_text(bytes), streamed(bytes));
        }
        // 完整的帧用保留的上下文解压，其余的退回流式解压
        let mut dctx = zstd::zstd_safe::DCtx::create();
        for whole in [&declared, &undeclared, &two_frames] {
            assert!(inflate_with(&mut dctx, whole).is_some());
        }
        for broken in [&corrupt, &truncated, &trailing] {
            assert!(inflate_with(&mut dctx, broken).is_none());
        }
        assert_eq!(bytes_to_text(&two_frames), text.repeat(2));
        assert_eq!(bytes_to_text(&not_utf8), "abcdefghij");
    }

    #[test]
    fn rows_without_a_sort_seq_are_placed_by_their_send_time() {
        assert_eq!(sort_key(0, 10, 3), (10_000, 10, 3));
        assert_eq!(sort_key(10_005, 10, 3), (10_005, 10, 3));
    }
}

/// `cargo kani` 的证明（#501）。
#[cfg(kani)]
mod proofs {
    use super::*;

    /// 不论时间是多少，没有 `sort_seq`（为 0）的消息都按 `create_time` 排：早的排在前面，不会溢出；
    /// 有 `sort_seq` 的消息按 `sort_seq` 排。
    #[kani::proof]
    fn messages_without_a_sort_seq_are_in_time_order() {
        let (t1, t2, a, b): (i64, i64, i64, i64) = kani::any();
        kani::assume(t1 < t2);
        assert!(sort_key(0, t1, a) < sort_key(0, t2, b));
        let seq: i64 = kani::any_where(|&s: &i64| s != 0);
        assert!(sort_key(seq, t1, a).0 == seq);
    }
}
