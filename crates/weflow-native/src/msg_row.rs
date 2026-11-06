//! The rows of a conversation's `Msg_<md5>` tables (#257, #490), read by the paging, cursor and merge code of
//! [`native_msg`](crate::native_msg) as [`MsgRow`]s: the columns typed, the content inflated and decoded once, the
//! sender resolved, the database named by its shard number (#491, #492).

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::types::ValueRef;
use rusqlite::Row;

use crate::native_msg::{shard_of, MsgPos, MsgTable};

/// Which columns of a message row a query reads; the others come back empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Columns {
    All,
    /// All but `source`, which exports do not use.
    NoSource,
    /// Neither `source` nor `packed_info_data`, which bulk scans do not read.
    Lite,
}

/// Cells of an integer column (`local_id`, `create_time`, `sort_seq`, …) holding another type: read as absent. WeChat
/// never writes one; a count above zero says the database is not what the reader expects.
static ODD_CELLS: AtomicU64 = AtomicU64::new(0);

/// How many cells of integer columns held another type since the program started (see [`MsgRow`]).
pub fn odd_cell_count() -> u64 {
    ODD_CELLS.load(Ordering::Relaxed)
}

/// What a row is read with besides its columns.
pub(crate) struct RowContext<'a> {
    pub(crate) table: &'a MsgTable,
    /// Whether a sender is the account's owner.
    pub(crate) is_me: &'a dyn Fn(&str) -> bool,
}

fn sort_key(sort_seq: i64, create_time: i64, local_id: i64) -> (i64, i64, i64) {
    let seq = if sort_seq == 0 {
        create_time * 1000
    } else {
        sort_seq
    };
    (seq, create_time, local_id)
}

/// A message row (see the module).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MsgRow {
    pub local_id: i64,
    pub server_id: Option<i64>,
    /// The type, with the sub-type of an app message in the high 32 bits (`(sub << 32) | 49`).
    pub local_type: i64,
    /// The place in the chat's order; WeChat leaves it null or 0 on some rows.
    pub sort_seq: Option<i64>,
    /// Unix seconds.
    pub create_time: Option<i64>,
    pub status: Option<i64>,
    /// The sender's username; none when `Name2Id` has no row for it.
    pub sender: Option<String>,
    pub is_send: bool,
    /// `compress_content` when it holds anything, else `message_content`: inflated (zstd) and decoded as text.
    pub content: String,
    /// The `msgsource` XML; left out of lite rows.
    pub source: Option<String>,
    /// `packed_info_data` as stored; left out of lite rows.
    pub packed_info: Option<Vec<u8>>,
    /// The `<n>` of the `message_<n>.db` holding the row.
    pub shard: u32,
}

// one per message read: its size changes only on purpose (#505)
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

/// zstd frames start with these bytes (`0xFD2FB528`, little-endian).
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// Largest size a zstd frame may declare for [`inflate`] to reserve it up front.
const INFLATE_RESERVE_MAX: u64 = 16 << 20;

thread_local! {
    /// A decompression context kept for the next message, instead of a context and a read buffer made and freed
    /// for each one (`zstd::stream::decode_all`).
    static DCTX: std::cell::RefCell<Option<zstd::zstd_safe::DCtx<'static>>> =
        const { std::cell::RefCell::new(None) };
}

/// `bytes` (starting with [`ZSTD_MAGIC`]) inflated, or `None` when they are not valid zstd.
fn inflate(bytes: &[u8]) -> Option<Vec<u8>> {
    // anything but a clean end of every frame goes the old way, so errors and truncation come out as they did
    DCTX.with(|slot| {
        inflate_with(
            slot.borrow_mut()
                .get_or_insert_with(zstd::zstd_safe::DCtx::create),
            bytes,
        )
    })
    .or_else(|| zstd::stream::decode_all(bytes).ok())
}

/// Every frame of `bytes` inflated with `dctx`; `None` unless the input ends right where a frame does.
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
            return None; // no progress with room to spare: the input ends inside a frame
        }
    }
}

/// Bytes as text: inflated first when they are a zstd frame, then UTF-8. Bytes that are not UTF-8 lose the invalid
/// sequences when they are under a fifth of the characters, and are otherwise read one byte a character (Latin-1).
pub fn bytes_to_text(bytes: &[u8]) -> String {
    let inflated = if bytes.starts_with(&ZSTD_MAGIC) {
        inflate(bytes)
    } else {
        None
    };
    let inflated = match inflated.map(String::from_utf8) {
        // the inflated buffer becomes the text as it is
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
    if (replaced as f64) < lossy.chars().count() as f64 * 0.2 {
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
    /// Where the row is in its conversation's order, to page from.
    pub fn pos(&self) -> MsgPos {
        let (sort_seq, create_time, local_id) = self.key();
        MsgPos {
            sort_seq,
            create_time,
            local_id,
            shard: self.shard,
        }
    }

    /// The select list over the message table `m` and `Name2Id` `n`, joined on the sender; lite rows leave out the
    /// source and the packed info, which bulk scans do not read.
    pub(crate) fn columns(lite: bool) -> &'static str {
        Self::columns_of(if lite { Columns::Lite } else { Columns::All })
    }

    /// The select list of [`columns`](Self::columns) for `which` columns.
    pub(crate) fn columns_of(which: Columns) -> &'static str {
        match which {
            Columns::All => MSG_COLUMNS,
            Columns::NoSource => MSG_COLUMNS_NO_SOURCE,
            Columns::Lite => MSG_COLUMNS_LITE,
        }
    }

    /// The row at `row`, whose columns are [`columns`](Self::columns).
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

    /// `(sort_seq, create_time, local_id)` in the chat's order: a row without a `sort_seq` (null or 0) is placed by
    /// `create_time * 1000`, and a missing integer counts as 0.
    pub(crate) fn key(&self) -> (i64, i64, i64) {
        sort_key(
            self.sort_seq.unwrap_or(0),
            self.create_time.unwrap_or(0),
            self.local_id,
        )
    }

    /// `(local_id, sort_seq)` as stored, a missing one 0: how WeChat's search index names a message.
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
        // a stray invalid byte in text is dropped; bytes that are mostly not UTF-8 are read one a character
        assert_eq!(bytes_to_text(b"abcdefghij\xff"), "abcdefghij");
        assert_eq!(bytes_to_text(&[0xff, 0xfe, 0x41]), "\u{ff}\u{fe}A");
    }

    #[test]
    fn inflating_with_a_kept_context_gives_what_streaming_gave() {
        // what bytes_to_text did before it kept a context: stream every frame
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
        // whole frames go through the kept context; the rest fall back to streaming
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
