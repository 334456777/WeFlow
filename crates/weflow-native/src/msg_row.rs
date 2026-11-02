//! The rows of a conversation's `Msg_<md5>` tables (#257, #490), read by the paging, cursor and merge code of
//! [`native_msg`](crate::native_msg) in one of two shapes ([`RowShape`]):
//!
//! - [`MsgRow`]: the columns typed, the content inflated and decoded once, the sender resolved, the database named by
//!   its shard number. What the service layer reads (#491, #492).
//! - [`LegacyRow`]: the JSON object of WeChat's own columns (`select m.*`) plus `sender_username`, `is_send`,
//!   `_db_path`, `db_name` and `table_name`. The shape the desktop app reads through `weflow-wcdb-ffi`, which keeps
//!   `wcdb_api`'s.

use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::types::ValueRef;
use rusqlite::Row;
use serde_json::{json, Map, Value};

use crate::native_msg::{shard_of, MsgPos, MsgTable};

/// Cells of an integer column (`local_id`, `create_time`, `sort_seq`, …) holding another type: read as absent. WeChat
/// never writes one; a count above zero says the database is not what the reader expects.
static ODD_CELLS: AtomicU64 = AtomicU64::new(0);

/// How many cells of integer columns held another type since the program started (see [`MsgRow`]).
pub fn odd_cell_count() -> u64 {
    ODD_CELLS.load(Ordering::Relaxed)
}

/// What a row is read with besides its columns.
pub struct RowContext<'a> {
    pub table: &'a MsgTable,
    /// Bulk scans that only need the type, the times, the sender and the text leave the rest out.
    pub lite: bool,
    /// Whether a sender is the account's owner.
    pub is_me: &'a dyn Fn(&str) -> bool,
}

/// A shape of message row: the columns it selects and how it reads them.
pub trait RowShape: Sized {
    /// The select list over the message table `m` and `Name2Id` `n`, joined on the sender.
    fn columns(lite: bool) -> &'static str;
    /// The row at `row`, whose columns are [`columns`](Self::columns).
    fn read(row: &Row<'_>, ctx: &RowContext<'_>) -> rusqlite::Result<Self>;
    /// `(sort_seq, create_time, local_id)` in the chat's order: a row without a `sort_seq` (null or 0) is placed by
    /// `create_time * 1000`, and a missing integer counts as 0.
    fn key(&self) -> (i64, i64, i64);
    /// `(local_id, sort_seq)` as stored, a missing one 0: how WeChat's search index names a message.
    fn index_key(&self) -> (i64, i64);
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
#[derive(Clone, Debug, PartialEq)]
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
}

const MSG_COLUMNS: &str =
    "m.local_id, m.server_id, m.local_type, m.sort_seq, m.create_time, m.status, \
     m.compress_content, m.message_content, m.source, m.packed_info_data, n.user_name";
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

/// Bytes as text: inflated first when they are a zstd frame, then UTF-8. Bytes that are not UTF-8 lose the invalid
/// sequences when they are under a fifth of the characters, and are otherwise read one byte a character (Latin-1),
/// as the desktop app did.
pub fn bytes_to_text(bytes: &[u8]) -> String {
    let inflated;
    let bytes = if bytes.starts_with(&ZSTD_MAGIC) {
        match zstd::stream::decode_all(bytes) {
            Ok(v) => {
                inflated = v;
                &inflated[..]
            }
            Err(_) => bytes,
        }
    } else {
        bytes
    };
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

impl RowShape for MsgRow {
    fn columns(lite: bool) -> &'static str {
        if lite {
            MSG_COLUMNS_LITE
        } else {
            MSG_COLUMNS
        }
    }

    fn read(row: &Row<'_>, ctx: &RowContext<'_>) -> rusqlite::Result<Self> {
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

    fn key(&self) -> (i64, i64, i64) {
        sort_key(
            self.sort_seq.unwrap_or(0),
            self.create_time.unwrap_or(0),
            self.local_id,
        )
    }

    fn index_key(&self) -> (i64, i64) {
        (self.local_id, self.sort_seq.unwrap_or(0))
    }
}

/// A message row in the shape of WeChat's own columns (see the module).
#[derive(Clone, Debug, PartialEq)]
pub struct LegacyRow(pub Value);

/// Columns left out of lite legacy rows.
const LITE_DROP: &[&str] = &[
    "compress_content",
    "packed_info_data",
    "source",
    "WCDB_CT_source",
    "WCDB_CT_message_content",
];

/// Adds to a legacy row (`m.*` and `sender_username`) what WeChat's columns do not say: whether the owner sent it, and
/// where it is kept. Lite rows lose the columns bulk scans do not read.
pub(crate) fn finish_legacy(row: &mut Value, ctx: &RowContext<'_>) {
    let is_send = (ctx.is_me)(
        row.get("sender_username")
            .and_then(Value::as_str)
            .unwrap_or(""),
    );
    let Some(obj) = row.as_object_mut() else {
        return;
    };
    let t = ctx.table;
    obj.insert("is_send".into(), json!(if is_send { 1 } else { 0 }));
    // Text was already inflated by the reader, so the WCDB "compressed" flag does not apply.
    obj.insert("WCDB_CT_message_content".into(), Value::Null);
    obj.insert("_db_path".into(), json!(t.db.to_string_lossy()));
    obj.insert(
        "db_name".into(),
        json!(t.db.file_stem().and_then(|s| s.to_str()).unwrap_or("")),
    );
    obj.insert("table_name".into(), json!(t.table));
    if ctx.lite {
        for k in LITE_DROP {
            obj.remove(*k);
        }
    }
}

impl LegacyRow {
    pub fn into_value(self) -> Value {
        self.0
    }

    /// The rows as one JSON array, what the `Value` readers of `wcdb` return.
    pub fn array(rows: Vec<Self>) -> Value {
        Value::Array(rows.into_iter().map(Self::into_value).collect())
    }

    fn int(&self, key: &str) -> i64 {
        self.0.get(key).and_then(Value::as_i64).unwrap_or(0)
    }
}

impl RowShape for LegacyRow {
    fn columns(_lite: bool) -> &'static str {
        "m.*, n.user_name as sender_username"
    }

    fn read(row: &Row<'_>, ctx: &RowContext<'_>) -> rusqlite::Result<Self> {
        let stmt = row.as_ref();
        let mut obj = Map::with_capacity(stmt.column_count() + 5);
        for i in 0..stmt.column_count() {
            let name = stmt.column_name(i)?;
            obj.insert(
                name.to_owned(),
                crate::native_db::value_to_json(name, row.get_ref(i)?),
            );
        }
        let mut row = Value::Object(obj);
        finish_legacy(&mut row, ctx);
        Ok(Self(row))
    }

    fn key(&self) -> (i64, i64, i64) {
        sort_key(
            self.int("sort_seq"),
            self.int("create_time"),
            self.int("local_id"),
        )
    }

    fn index_key(&self) -> (i64, i64) {
        (self.int("local_id"), self.int("sort_seq"))
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
    fn rows_without_a_sort_seq_are_placed_by_their_send_time() {
        assert_eq!(sort_key(0, 10, 3), (10_000, 10, 3));
        assert_eq!(sort_key(10_005, 10, 3), (10_005, 10, 3));
    }
}
