//! 为测试在磁盘上生成合成账号（真正加密的 SQLCipher 数据库）。
//!
//! 由 `test-fixtures` feature 启用（本 crate 自己的测试总是启用）。数据库用真实的表结构，
//! 所以文件格式之上的一切（查表、解析发送者、联系人、群）都与在真实账号上一样运行。
//!
//! # Panics
//!
//! 生成器写不出要生成的内容时 panic，让请求它的测试失败。
#![expect(clippy::unwrap_used, reason = "测试数据的生成器用 panic 让测试失败")]
#![expect(
    clippy::cast_possible_wrap,
    reason = "合成账号的行、ID 和计数都只有几百"
)]

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};

use crate::db::hex;
use crate::msg::table_name_for;
use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
use crate::sqlcipher::PageCipher;

pub mod bench;

/// 一个测试自己的目录 `<parent>/<prefix>-<random>`，丢弃时连同其中的一切删除。
///
/// 测试 panic 时也会删除。设了 `WEFLOW_KEEP_TEST_DIRS` 时保留这些目录，供之后查看。
///
/// 只要还有东西在读这个目录就要持有它，并在打开了其中文件的对象之后再丢弃：与账号一起返回它的辅助函数把它放在前面，
/// 写成 `(TestDir, account)`，因为模式中后绑定的变量先丢弃。
#[derive(Debug)]
pub struct TestDir(tempfile::TempDir);

impl TestDir {
    /// 在系统的临时目录下建这个目录。
    ///
    /// # Panics
    ///
    /// 无法建目录时。
    pub fn new(prefix: &str) -> Self {
        Self::new_in(&std::env::temp_dir(), prefix)
    }

    /// 在 `parent` 下建这个目录，`parent` 必须已经存在。
    ///
    /// # Panics
    ///
    /// 无法建目录时。
    #[expect(clippy::expect_used, reason = "拿不到目录的测试以 panic 失败")]
    pub fn new_in(parent: &Path, prefix: &str) -> Self {
        let dir = tempfile::Builder::new()
            .prefix(&format!("{prefix}-"))
            .disable_cleanup(std::env::var_os("WEFLOW_KEEP_TEST_DIRS").is_some())
            .tempdir_in(parent)
            .expect("the parent directory takes a directory");
        Self(dir)
    }

    /// 目录的路径。
    pub fn path(&self) -> &Path {
        self.0.path()
    }
}

impl std::ops::Deref for TestDir {
    type Target = Path;

    fn deref(&self) -> &Path {
        self.path()
    }
}

impl AsRef<Path> for TestDir {
    fn as_ref(&self) -> &Path {
        self.path()
    }
}

/// [`Fixture::session_db`] 用的 `SessionTable` 中的一行。
#[derive(Debug)]
pub struct SessionSpec<'a> {
    /// 会话的用户名。
    pub username: &'a str,
    /// 会话列表上显示的最后一条消息。
    pub summary: &'a str,
    /// 最后一条消息的时间，也是这一行的 `sort_timestamp`。
    pub last_timestamp: i64,
    /// 未读数 `unread_count`。
    pub unread: i64,
    /// 最后一条消息的类型。
    pub last_msg_type: i64,
}

/// [`Fixture::contact_db`] 用的 `contact` 中的一行；[`ContactSpec::new`] 把其余各列留空。
#[derive(Clone, Debug)]
pub struct ContactSpec<'a> {
    /// 联系人的 wxid。
    pub username: &'a str,
    /// 微信的 `local_type`：好友为 1，曾经的好友为 0 并带拼音。
    pub local_type: i64,
    /// 昵称。
    pub nick_name: &'a str,
    /// 备注。
    pub remark: &'a str,
    /// 微信号。
    pub alias: &'a str,
    /// 头像地址，写进 `big_head_url`。
    pub avatar: &'a str,
    /// `flag` 的各位（折叠、免打扰、置顶等）。
    pub flag: i64,
    /// `chat_room_notify`：群关闭了通知时为 1。
    pub chat_room_notify: i64,
    /// `description` 列。
    pub description: &'a str,
    /// `extra_buffer` 列的原始 protobuf 字节（签名、地区、标签 ID）。
    pub extra: &'a [u8],
}

impl<'a> ContactSpec<'a> {
    /// 只有用户名、类型和昵称的联系人。
    pub const fn new(username: &'a str, local_type: i64, nick_name: &'a str) -> Self {
        Self {
            username,
            local_type,
            nick_name,
            remark: "",
            alias: "",
            avatar: "",
            flag: 0,
            chat_room_notify: 0,
            description: "",
            extra: &[],
        }
    }
}

/// [`Fixture::contact_db`] 用的一个群：它在 `chat_room` 中的行和它的成员。
#[derive(Debug)]
pub struct RoomSpec<'a> {
    /// 群的用户名 `…@chatroom`。
    pub username: &'a str,
    /// 群主的 wxid。
    pub owner: &'a str,
    /// 每个成员的（用户名，群昵称；没有群昵称时为 ""）。
    pub members: &'a [(&'a str, &'a str)],
}

/// 一行消息；[`MsgSpec::text`] 生成文字消息，`with_*` 等方法修改它。
#[derive(Clone, Debug)]
pub struct MsgSpec {
    /// 行在所在表中的 ID。
    pub local_id: i64,
    /// 微信给消息的 ID（服务器 ID）。
    pub server_id: i64,
    /// 微信的消息类型。
    pub local_type: i64,
    /// 发送者的用户名；没有发送者时为空。
    pub sender: &'static str,
    /// 发送时间，Unix 秒。
    pub create_time: i64,
    /// 文字，或者非文字消息的 XML。
    pub content: String,
    /// 把 `content` 存成 zstd 压缩的 blob，与 WCDB 对部分消息类型的做法一样。
    pub compressed: bool,
    /// 消息的 `source` 列（msgsource XML），如 `<atuserlist>`。
    pub source: String,
    /// 不是微信通常的 `create_time * 1000` 时的 `sort_seq`。
    pub sort_seq: Option<i64>,
}

impl MsgSpec {
    /// 服务器 ID 为 `1000 + local_id` 的文字消息。
    pub fn text(local_id: i64, sender: &'static str, create_time: i64, content: &str) -> Self {
        Self {
            local_id,
            server_id: 1_000 + local_id,
            local_type: 1,
            sender,
            create_time,
            content: content.to_string(),
            compressed: false,
            source: String::new(),
            sort_seq: None,
        }
    }
    /// 改用这个 `sort_seq`。
    pub fn with_sort_seq(mut self, sort_seq: i64) -> Self {
        self.sort_seq = Some(sort_seq);
        self
    }
    /// 改用这个 `source`。
    pub fn with_source(mut self, source: &str) -> Self {
        self.source = source.to_string();
        self
    }
    /// 改成这个类型。
    pub fn of_type(mut self, local_type: i64) -> Self {
        self.local_type = local_type;
        self
    }
    /// 内容改为压缩存储。
    pub fn compressed(mut self) -> Self {
        self.compressed = true;
        self
    }
}

/// [`Fixture::message_fts_db`] 用的微信全文索引中的一行。
#[derive(Clone, Debug)]
pub struct FtsSpec<'a> {
    /// 被索引的文字。
    pub text: String,
    /// 消息在所在表中的 ID。
    pub local_id: i64,
    /// 消息的 `sort_seq`。
    pub sort_seq: i64,
    /// 消息的类型。
    pub local_type: i64,
    /// 会话的用户名。
    pub session: &'a str,
    /// 发送时间。
    pub create_time: i64,
}

/// [`Fixture::sns_db`] 用的一条朋友圈动态。
#[derive(Debug)]
pub struct SnsPostSpec<'a> {
    /// 原样存储的 `tid`（负数表示大于 `i64::MAX` 的无符号雪花 ID）。
    pub tid: i64,
    /// 发布者。
    pub user: &'a str,
    /// 发布时间。
    pub create_time: i64,
    /// 动态的文字。
    pub desc: &'a str,
    /// 动态的 `<ContentObject><type>`。
    pub kind: i64,
    /// 图片的张数。
    pub media: usize,
    /// 每个点赞者的（用户名，昵称）。
    pub likes: &'a [(&'a str, &'a str)],
    /// 追加在 `<TimelineObject>` 里的原始 XML（位置、评论列表等）。
    pub extra: &'a str,
}

impl SnsPostSpec<'_> {
    fn xml(&self) -> String {
        let media: String = (1..=self.media)
            .map(|i| {
                format!(
                    "<media><id>{i}</id><type>2</type><thumb type=\"1\" key=\"1\" enc_idx=\"1\" token=\"TT{i}\">http://thumb/{i}?a=1&amp;b=2</thumb>\
                     <url type=\"1\" md5=\"md5{i}\" key=\"2\" enc_idx=\"1\" token=\"TU{i}\">http://url/{i}</url></media>"
                )
            })
            .collect();
        let likes: String = self
            .likes
            .iter()
            .map(|(u, n)| {
                format!(
                    "<user_comment><username>{u}</username><nickname>{n}</nickname></user_comment>"
                )
            })
            .collect();
        format!(
            "<SnsDataItem><TimelineObject><id>{}</id><username>{}</username><createTime>{}</createTime><contentDesc>{}</contentDesc>\
             <ContentObject><type>{}</type><mediaList>{media}</mediaList></ContentObject><like_user_list>{likes}</like_user_list>{extra}</TimelineObject></SnsDataItem>",
            self.tid.cast_unsigned(), self.user, self.create_time, self.desc, self.kind, extra = self.extra
        )
    }
}

/// [`Fixture::media_db`] 用的 `VoiceInfo` 中的一行。
#[derive(Debug)]
pub struct VoiceSpec<'a> {
    /// 会话的用户名。
    pub chat: &'a str,
    /// 消息的发送时间。
    pub create_time: i64,
    /// 消息在所在表中的 ID。
    pub local_id: i64,
    /// 微信给消息的 ID（服务器 ID）。
    pub svr_id: i64,
    /// 语音数据，或者其中的一个片段。
    pub data: Vec<u8>,
    /// `data_index`：片段的位置。
    pub index: &'a str,
}

/// [`Fixture::emoticon_db`] 用的一行用户自己的表情。
#[derive(Clone, Copy, Debug)]
pub struct EmoticonSpec {
    /// 表情的 MD5。
    pub md5: &'static str,
    /// 表情的说明文字。
    pub caption: &'static str,
    /// 表情的 `cdn_url`。
    pub cdn_url: &'static str,
    /// 表情的 `extern_url`。
    pub extern_url: &'static str,
}

/// `hardlink.db` 中的一行：`kind` 为 1 是文件，2 是图片，3 是视频，4 是目录项。
/// 图片在 `<dir1>/<dir2>/Img/` 下，视频和文件在 `<dir1>/` 下。
#[derive(Clone, Debug)]
pub struct HardlinkSpec<'a> {
    /// 消息中记录的文件 MD5。
    pub md5: &'a str,
    /// 文件名。
    pub file_name: &'a str,
    /// 种类：1 文件，2 图片，3 视频，4 目录项。
    pub kind: i64,
    /// 第一层文件夹的名称。
    pub dir1: &'a str,
    /// 第二层文件夹的名称；没有时为空。
    pub dir2: &'a str,
    /// 文件的大小。
    pub size: i64,
    /// 写入时间。
    pub modify_time: i64,
}

impl<'a> HardlinkSpec<'a> {
    /// 存在聊天对象目录 `talker_dir` 和月份 `month` 下的一个图片版本。
    pub fn image(md5: &'a str, file_name: &'a str, talker_dir: &'a str, month: &'a str) -> Self {
        Self {
            md5,
            file_name,
            kind: 2,
            dir1: talker_dir,
            dir2: month,
            size: 1000,
            modify_time: T0,
        }
    }

    /// 存在月份 `month` 下的视频。
    pub fn video(md5: &'a str, file_name: &'a str, month: &'a str) -> Self {
        Self {
            md5,
            file_name,
            kind: 3,
            dir1: month,
            dir2: "",
            size: 5000,
            modify_time: T0,
        }
    }

    /// 收发的文件，存为 `msg/file/<month>/<file_name>`。
    pub fn file(md5: &'a str, file_name: &'a str, month: &'a str) -> Self {
        Self {
            md5,
            file_name,
            kind: 1,
            dir1: month,
            dir2: "",
            size: 100,
            modify_time: T0,
        }
    }
}

// 与微信相同的表结构，`Fixture` 和 `bench` 共用。

const SNS_SCHEMA: &str =
    "create table SnsTimeLine(tid integer primary key desc, user_name text, content text, pack_info_buf text)";

const MEDIA_SCHEMA: &str = "create table Name2Id(user_name text primary key); create table TimeStamp(timestamp integer);
     create table VoiceInfo(chat_name_id integer, create_time integer, local_id integer, svr_id integer, voice_data blob, data_index text default '0');";

const EMOTICON_SCHEMA: &str = "create table kNonStoreEmoticonTable(type integer, md5 text, caption text, product_id text, aes_key text, thumb_url text, tp_url text, auth_key text, cdn_url text, extern_url text, extern_md5 text, encrypt_url text, designer_id text, activity_id text);
     create table kStoreEmoticonCaptionsTable(package_id_ text, md5_ text, language_ text, caption_ text);";

const HEAD_IMAGE_SCHEMA: &str =
    "create table head_image(username text primary key, md5 text, image_buffer blob, update_time integer)";

const SESSION_SCHEMA: &str = "create table SessionTable(username text primary key, type integer, unread_count integer, unread_first_msg_srv_id integer, \
     unread_first_pat_msg_local_id integer, unread_first_pat_msg_sort_seq integer, is_hidden integer, summary, draft text, \
     status integer, last_timestamp integer, sort_timestamp integer, last_clear_unread_timestamp integer, last_msg_locald_id integer, \
     last_msg_type integer, last_msg_sub_type integer, last_msg_sender text, last_sender_display_name text, last_msg_ext_type integer)";

const NAME2ID_SCHEMA: &str = "create table Name2Id(user_name text primary key, is_session integer)";

const FTS_NAMES_SCHEMA: &str = "create table fts_username_v1(username text primary key)";

fn hardlink_schema() -> String {
    let table = "(md5_hash integer, md5 text, type integer, file_name text, file_size integer, modify_time integer, dir1 integer, dir2 integer, _rowid_ integer primary key asc, extra_buffer blob)";
    format!(
        "create table dir2id(username text primary key); create table image_hardlink_info_v4{table}; \
         create table video_hardlink_info_v4{table}; create table file_hardlink_info_v4{table};"
    )
}

fn contact_schema() -> String {
    let cols = "id integer primary key, username text, local_type integer, alias text, encrypt_username text, flag integer, \
                delete_flag integer, verify_flag integer, remark text, remark_quan_pin text, remark_pin_yin_initial text, nick_name text, \
                pin_yin_initial text, quan_pin text, big_head_url text, small_head_url text, head_img_md5 text, chat_room_notify integer, \
                is_in_chat_room integer, description text, extra_buffer blob, chat_room_type integer";
    format!(
        "create table contact({cols}); create table stranger({cols}); create table name2id(username text primary key);
         create table contact_label(label_id_ integer primary key, label_name_ text, sort_order_ integer);
         create table chat_room(id integer primary key, username text, owner text, ext_buffer blob);
         create table chatroom_member(room_id integer, member_id integer, constraint room_member unique(room_id, member_id));"
    )
}

/// `chat_room` 行的 `ext_buffer`：写入每个有群昵称的 `(member, nickname)`。
fn room_ext_buffer<'a>(members: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<u8> {
    let mut ext = vec![0x08, 0x01];
    for (m, nick) in members.into_iter().filter(|(_, n)| !n.is_empty()) {
        let mut inner = vec![0x0a, u8::try_from(m.len()).unwrap()];
        inner.extend(m.as_bytes());
        inner.extend([0x12, u8::try_from(nick.len()).unwrap()]);
        inner.extend(nick.as_bytes());
        ext.extend([0x0a, u8::try_from(inner.len()).unwrap()]);
        ext.extend(inner);
    }
    ext
}

fn fts_table_schema(g: usize) -> String {
    format!(
        "create virtual table message_fts_v4_{g} using fts5(acontent, message_local_id unindexed, \
         sort_seq unindexed, local_type unindexed, session_id unindexed, sender_id unindexed, \
         create_time unindexed)"
    )
}

fn message_table_schema(table: &str) -> String {
    format!(
        "create table \"{table}\"(local_id integer primary key autoincrement, server_id integer, local_type integer, sort_seq integer, \
         real_sender_id integer, create_time integer, status integer, upload_status integer, download_status integer, server_seq integer, \
         origin_source integer, source text, message_content, compress_content, packed_info_data blob, \
         WCDB_CT_message_content integer default null, WCDB_CT_source integer default null); \
         create index \"{table}_SORTSEQ\" on \"{table}\"(sort_seq); \
         create index \"{table}_TYPE_SEQ\" on \"{table}\"(local_type, sort_seq); \
         create index \"{table}_SERVERID\" on \"{table}\"(server_id); \
         create index \"{table}_SENDERID\" on \"{table}\"(real_sender_id)"
    )
}

/// 磁盘上的合成账号，由它的方法写入各个数据库。
#[derive(Debug)]
pub struct Fixture {
    /// 账号目录所在的目录。
    pub root: PathBuf,
    /// 账号目录，其中有 `db_storage`。
    pub account_dir: PathBuf,
    salts: Salts,
}

/// 加密 [`Fixture`] 的数据库所用的盐。
#[derive(Debug)]
enum Salts {
    /// 每个数据库都用 [`SALT`]：读取方只派生一次页密钥就能读全部数据库，测试因此更快。
    Shared(Box<PageCipher>),
    /// 每个数据库各有自己的盐，由它的路径和这个种子得出，与微信写的一样：
    /// 读取方每打开一个数据库都要派生一次页密钥（#1127）。
    OwnPerDatabase(u64),
}

/// [`Salts::OwnPerDatabase`] 给 `rel`（相对于 `db_storage`，用 `/` 分隔）处的数据库的盐：
/// 种子和路径的 SHA-256 的前 16 字节。
fn own_salt(seed: u64, rel: &str) -> [u8; 16] {
    let digest = Sha256::new()
        .chain_update(seed.to_le_bytes())
        .chain_update(rel.as_bytes())
        .finalize();
    let mut salt = [0u8; 16];
    salt.copy_from_slice(&digest[..16]);
    salt
}

impl Fixture {
    /// 建立 `<root>/<account>/db_storage/...`；先清空 `root`。
    pub fn new(root: &Path, account: &str) -> Self {
        let _ = std::fs::remove_dir_all(root);
        let account_dir = root.join(account);
        for d in [
            "session",
            "contact",
            "message",
            "sns",
            "emoticon",
            "hardlink",
            "head_image",
        ] {
            std::fs::create_dir_all(account_dir.join("db_storage").join(d)).unwrap();
        }
        Self {
            root: root.to_path_buf(),
            account_dir,
            salts: Salts::Shared(Box::new(PageCipher::derive(&KEY, &SALT))),
        }
    }

    /// 之前由 [`new`](Self::new) 在 `root` 下建立的账号，用来修改其中的部分数据库。
    pub fn open(root: &Path, account: &str) -> Self {
        Self {
            root: root.to_path_buf(),
            account_dir: root.join(account),
            salts: Salts::Shared(Box::new(PageCipher::derive(&KEY, &SALT))),
        }
    }

    /// 同一个账号，但每个数据库有自己的盐，由 `seed` 和数据库的路径得出，与微信写的一样：
    /// 读取方每打开一个数据库都要派生一次页密钥，每次约 130 毫秒，而 [`new`](Self::new) 的共用盐只派生一次（#1127）。
    #[must_use]
    pub fn with_own_salts(self, seed: u64) -> Self {
        Self {
            salts: Salts::OwnPerDatabase(seed),
            ..self
        }
    }

    /// 用 `rel`（相对于 `db_storage`，用 `/` 分隔）处数据库的页密钥（[`PageCipher`]）运行 `f`。
    pub(crate) fn with_cipher<R>(&self, rel: &str, f: impl FnOnce(&PageCipher) -> R) -> R {
        match &self.salts {
            Salts::Shared(cipher) => f(cipher),
            Salts::OwnPerDatabase(seed) => f(&PageCipher::derive(&KEY, &own_salt(*seed, rel))),
        }
    }

    /// 十六进制的数据库密钥，即 `decrypt_key` 接受的格式。
    pub fn key_hex(&self) -> String {
        hex(&KEY)
    }

    /// 账号的 `db_storage` 目录。
    pub fn db_storage(&self) -> PathBuf {
        self.account_dir.join("db_storage")
    }

    fn write(&self, rel: &str, plain: Vec<u8>) {
        let encrypted = self.with_cipher(rel, |cipher| encrypt_db(&plain, cipher));
        std::fs::write(self.db_storage().join(rel), encrypted).unwrap();
    }

    /// 写入含这些动态的 `sns/sns.db`。
    pub fn sns_db(&self, posts: &[SnsPostSpec<'_>]) {
        let rows: Vec<(i64, &str, String)> =
            posts.iter().map(|p| (p.tid, p.user, p.xml())).collect();
        self.write(
            "sns/sns.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch(SNS_SCHEMA).unwrap();
                for (tid, user, xml) in &rows {
                    c.execute(
                        "insert into SnsTimeLine(tid, user_name, content) values (?1, ?2, ?3)",
                        params![tid, user, xml],
                    )
                    .unwrap();
                }
            }),
        );
    }

    /// 写入含这些语音的 `message/media_<n>.db`。
    pub fn media_db(&self, n: u32, voices: &[VoiceSpec<'_>]) {
        /// 各项依次是聊天的 ID、发送时间、本地 ID、服务器 ID、数据和聊天的用户名。
        type VoiceRow<'a> = (i64, i64, i64, i64, Vec<u8>, &'a str);
        let rows: Vec<VoiceRow<'_>> = {
            let mut names: Vec<&str> = Vec::new();
            voices
                .iter()
                .map(|v| {
                    if !names.contains(&v.chat) {
                        names.push(v.chat);
                    }
                    (
                        names.iter().position(|c| *c == v.chat).unwrap() as i64 + 1,
                        v.create_time,
                        v.local_id,
                        v.svr_id,
                        v.data.clone(),
                        v.index,
                    )
                })
                .collect()
        };
        let mut names: Vec<&str> = Vec::new();
        for v in voices {
            if !names.contains(&v.chat) {
                names.push(v.chat);
            }
        }
        self.write(
            &format!("message/media_{n}.db"),
            plain_db_with(move |c: &Connection| {
                c.execute_batch(MEDIA_SCHEMA).unwrap();
                for (i, name) in names.iter().enumerate() {
                    c.execute(
                        "insert into Name2Id(rowid, user_name) values (?1, ?2)",
                        params![i as i64 + 1, name],
                    )
                    .unwrap();
                }
                for (chat, ct, local, svr, data, idx) in &rows {
                    c.execute(
                        "insert into VoiceInfo values (?1, ?2, ?3, ?4, ?5, ?6)",
                        params![chat, ct, local, svr, data, idx],
                    )
                    .unwrap();
                }
            }),
        );
    }

    /// 写入 `emoticon/emoticon.db`：`own` 是用户自己的表情，`store` 是商店表情说明表中的（md5，语言，说明文字）。
    pub fn emoticon_db(
        &self,
        own: &[EmoticonSpec],
        store: &[(&'static str, &'static str, &'static str)],
    ) {
        let own: Vec<(&str, &str, &str, &str)> = own
            .iter()
            .map(|e| (e.md5, e.caption, e.cdn_url, e.extern_url))
            .collect();
        let store = store.to_vec();
        self.write(
            "emoticon/emoticon.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch(EMOTICON_SCHEMA).unwrap();
                for (md5, caption, cdn, ext) in &own {
                    c.execute("insert into kNonStoreEmoticonTable(type, md5, caption, cdn_url, extern_url) values (1, ?1, ?2, ?3, ?4)", params![md5, caption, cdn, ext]).unwrap();
                }
                for (md5, lang, caption) in &store {
                    c.execute("insert into kStoreEmoticonCaptionsTable values ('pkg', ?1, ?2, ?3)", params![md5, lang, caption]).unwrap();
                }
            }),
        );
    }

    /// 写入 `hardlink/hardlink.db`：图片、视频、文件三张表，以及 `dir2id` 中的目录名。
    pub fn hardlink_db(&self, rows: &[HardlinkSpec<'_>]) {
        let rows = rows.to_vec();
        self.write(
            "hardlink/hardlink.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch(&hardlink_schema()).unwrap();
                fn id<'a>(name: &'a str, dirs: &mut Vec<&'a str>, ids: &mut std::collections::HashMap<&'a str, i64>) -> i64 {
                    if name.is_empty() {
                        return 0;
                    }
                    *ids.entry(name).or_insert_with(|| {
                        dirs.push(name);
                        dirs.len() as i64
                    })
                }
                let mut dir_list: Vec<&str> = Vec::new();
                let mut dir_ids = std::collections::HashMap::new();
                for r in &rows {
                    let (d1, d2) = (id(r.dir1, &mut dir_list, &mut dir_ids), id(r.dir2, &mut dir_list, &mut dir_ids));
                    let t = match r.kind {
                        2 | 4 => "image_hardlink_info_v4",
                        3 => "video_hardlink_info_v4",
                        _ => "file_hardlink_info_v4",
                    };
                    c.prepare_cached(&format!("insert into {t}(md5, type, file_name, file_size, modify_time, dir1, dir2) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)"))
                        .unwrap()
                        .execute(params![r.md5, r.kind, r.file_name, r.size, r.modify_time, d1, d2])
                        .unwrap();
                }
                for (i, d) in dir_list.iter().enumerate() {
                    c.execute("insert into dir2id(rowid, username) values (?1, ?2)", params![i as i64 + 1, d]).unwrap();
                }
            }),
        );
    }

    /// 写入 `head_image/head_image.db`：每个头像一项（用户名，图片的字节）。
    pub fn head_image_db(&self, avatars: &[(&'static str, &[u8])]) {
        let avatars: Vec<(&'static str, Vec<u8>)> =
            avatars.iter().map(|(u, b)| (*u, b.to_vec())).collect();
        self.write(
            "head_image/head_image.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch(HEAD_IMAGE_SCHEMA).unwrap();
                for (u, b) in &avatars {
                    c.execute(
                        "insert into head_image values (?1, 'md5', ?2, 1)",
                        params![u, b],
                    )
                    .unwrap();
                }
            }),
        );
    }

    /// 写入含这些会话的 `session/session.db`。
    pub fn session_db(&self, sessions: &[SessionSpec<'_>]) {
        let rows: Vec<(String, String, i64, i64, i64)> = sessions
            .iter()
            .map(|s| {
                (
                    s.username.to_string(),
                    s.summary.to_string(),
                    s.last_timestamp,
                    s.unread,
                    s.last_msg_type,
                )
            })
            .collect();
        self.write(
            "session/session.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch(SESSION_SCHEMA).unwrap();
                for (u, summary, ts, unread, t) in &rows {
                    c.execute(
                        "insert into SessionTable(username, type, unread_count, is_hidden, summary, last_timestamp, sort_timestamp, last_msg_type) \
                         values (?1, 0, ?2, 0, ?3, ?4, ?4, ?5)",
                        params![u, unread, summary, ts, t],
                    )
                    .unwrap();
                }
            }),
        );
    }

    /// 写入含这些联系人和群的 `contact/contact.db`。
    pub fn contact_db(&self, contacts: &[ContactSpec<'_>], rooms: &[RoomSpec<'_>]) {
        self.contact_db_with_labels(contacts, rooms, &[]);
    }

    /// 与 [`contact_db`](Self::contact_db) 相同，另在 `contact_label` 中写入（标签 ID，名称）的行。
    pub fn contact_db_with_labels(
        &self,
        contacts: &[ContactSpec<'_>],
        rooms: &[RoomSpec<'_>],
        labels: &[(i64, &'static str)],
    ) {
        let labels = labels.to_vec();
        let contacts: Vec<(i64, ContactSpec<'_>)> = contacts
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, c)| (i as i64 + 1, c))
            .collect();
        let ids: std::collections::HashMap<&str, i64> =
            contacts.iter().map(|(id, c)| (c.username, *id)).collect();
        /// 各项依次是群的行号、用户名、群主、成员（用户名，群昵称）和成员的行号。
        type RoomRow<'a> = (i64, &'a str, &'a str, &'a [(&'a str, &'a str)], Vec<i64>);
        let rooms: Vec<RoomRow<'_>> = rooms
            .iter()
            .map(|r| {
                let rid = *ids
                    .get(r.username)
                    .unwrap_or_else(|| panic!("room {} needs a contact row", r.username));
                let members = r
                    .members
                    .iter()
                    .map(|(m, _)| {
                        *ids.get(m)
                            .unwrap_or_else(|| panic!("member {m} needs a contact row"))
                    })
                    .collect();
                (rid, r.username, r.owner, r.members, members)
            })
            .collect();
        self.write(
            "contact/contact.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch(&contact_schema()).unwrap();
                for (id, name) in &labels {
                    c.execute("insert into contact_label(label_id_, label_name_, sort_order_) values (?1, ?2, ?1)", params![id, name]).unwrap();
                }
                for (id, ct) in &contacts {
                    let extra: Option<&[u8]> = Some(ct.extra).filter(|e| !e.is_empty());
                    c.execute(
                        "insert into contact(id, username, local_type, alias, flag, remark, nick_name, quan_pin, big_head_url, chat_room_notify, description, extra_buffer) \
                         values (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
                        params![id, ct.username, ct.local_type, ct.alias, ct.flag, ct.remark, ct.nick_name, ct.nick_name.to_lowercase(), ct.avatar, ct.chat_room_notify, ct.description, extra],
                    )
                    .unwrap();
                    c.execute("insert into name2id(rowid, username) values (?1, ?2)", params![id, ct.username]).unwrap();
                }
                for (rid, user, owner, members, member_ids) in &rooms {
                    let ext = room_ext_buffer(members.iter().copied());
                    c.execute("insert into chat_room(id, username, owner, ext_buffer) values (?1,?2,?3,?4)", params![rid, user, owner, ext]).unwrap();
                    for mid in member_ids {
                        c.execute("insert into chatroom_member(room_id, member_id) values (?1, ?2)", params![rid, mid]).unwrap();
                    }
                }
            }),
        );
    }

    /// 按微信的写法写入 `message/message_fts.db`：每组一张 FTS5 表（这里用 SQLite 自带的分词器，微信用的是
    /// `mmftstokenizer`），还有一张用户名表，它的行号就是各行的 `session_id`。
    pub fn message_fts_db(&self, groups: &[Vec<FtsSpec<'_>>]) {
        let groups = groups.to_vec();
        self.write(
            "message/message_fts.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch(FTS_NAMES_SCHEMA).unwrap();
                let mut ids: std::collections::HashMap<&str, i64> = Default::default();
                for (g, rows) in groups.iter().enumerate() {
                    c.execute_batch(&fts_table_schema(g)).unwrap();
                    for r in rows {
                        let next = ids.len() as i64 + 1;
                        let id = *ids.entry(r.session).or_insert_with(|| {
                            c.execute(
                                "insert into fts_username_v1(rowid, username) values (?1, ?2)",
                                params![next, r.session],
                            )
                            .unwrap();
                            next
                        });
                        c.prepare_cached(&format!(
                            "insert into message_fts_v4_{g} values (?1, ?2, ?3, ?4, ?5, 0, ?6)"
                        ))
                        .unwrap()
                        .execute(params![
                            r.text,
                            r.local_id,
                            r.sort_seq,
                            r.local_type,
                            id,
                            r.create_time
                        ])
                        .unwrap();
                    }
                }
            }),
        );
    }

    /// 写入 `message/message_<n>.db`，其中是 `sessions` 中每个会话的消息。每次调用都重写整个文件，
    /// 不与已有的内容合并：一个分片的全部会话要在一次调用里给出。
    pub fn message_shard(&self, n: u32, sessions: &[(&str, Vec<MsgSpec>)]) {
        self.message_db(&format!("message/message_{n}.db"), sessions);
    }

    /// 写入 `message/biz_message_<n>.db` 的 [`message_shard`](Self::message_shard)，
    /// 微信在这个数据库里保存公众号和服务通知的消息。
    pub fn biz_message_db(&self, n: u32, sessions: &[(&str, Vec<MsgSpec>)]) {
        self.message_db(&format!("message/biz_message_{n}.db"), sessions);
    }

    fn message_db(&self, rel: &str, sessions: &[(&str, Vec<MsgSpec>)]) {
        let sessions: Vec<(String, Vec<MsgSpec>)> = sessions
            .iter()
            .map(|(s, m)| (s.to_string(), m.clone()))
            .collect();
        self.write(
            rel,
            plain_db_with(move |c: &Connection| {
                c.execute_batch(NAME2ID_SCHEMA).unwrap();
                let mut next_id = 0i64;
                let mut ids: std::collections::HashMap<String, i64> = Default::default();
                let mut id_of = |c: &Connection, name: &str| -> i64 {
                    *ids.entry(name.to_string()).or_insert_with(|| {
                        next_id += 1;
                        c.execute("insert into Name2Id(rowid, user_name, is_session) values (?1, ?2, 0)", params![next_id, name]).unwrap();
                        next_id
                    })
                };
                for (session, msgs) in &sessions {
                    id_of(c, session);
                    let table = table_name_for(session);
                    c.execute_batch(&message_table_schema(&table)).unwrap();
                    for m in msgs {
                        let sender = if m.sender.is_empty() { 0 } else { id_of(c, m.sender) };
                        let content: Box<dyn rusqlite::ToSql> =
                            if m.compressed { Box::new(zstd::encode_all(m.content.as_bytes(), 1).unwrap()) } else { Box::new(m.content.clone()) };
                        c.execute(
                            &format!(
                                "insert into \"{table}\"(local_id, server_id, local_type, sort_seq, real_sender_id, create_time, status, message_content, source) \
                                 values (?1,?2,?3,?4,?5,?6,2,?7,?8)"
                            ),
                            params![m.local_id, m.server_id, m.local_type, m.sort_seq.unwrap_or(m.create_time * 1000), sender, m.create_time, content, m.source],
                        )
                        .unwrap();
                    }
                }
            }),
        );
    }
}

/// [`Fixture::standard`] 的基准时间，2023-11-14 22:13:20 UTC。
pub const T0: i64 = 1_700_000_000;
/// 一天的秒数。
pub const DAY: i64 = 86_400;

impl Fixture {
    /// 小而完整的账号：`wxid_me` 与 `wxid_bob` 的私聊（两个消息分片、一张图片、一条 zstd 压缩的消息），
    /// 群 `room1@chatroom`（群主是 bob，有群昵称，3 个成员），以及朋友圈、语音和表情的数据库。
    pub fn standard(root: &Path) -> Self {
        let f = Self::new(root, "wxid_me_ab12");
        f.contact_db(
            &[
                ContactSpec::new("wxid_me", 1, "Me Nick"),
                ContactSpec {
                    remark: "Bobby",
                    alias: "bobby_id",
                    avatar: "https://example.com/bob.png",
                    ..ContactSpec::new("wxid_bob", 1, "Bob")
                },
                // 在会话列表中置顶（#885）
                ContactSpec {
                    flag: 1 << 11,
                    ..ContactSpec::new("wxid_carol", 1, "Carol")
                },
                ContactSpec::new("wxid_quiet", 3, "Quiet"),
                ContactSpec::new("gh_news", 5, "News"),
                ContactSpec::new("medianote", 1, "File Helper"),
                ContactSpec {
                    flag: 1 << 28,
                    chat_room_notify: 1,
                    ..ContactSpec::new("room1@chatroom", 2, "Project Room")
                },
            ],
            &[RoomSpec {
                username: "room1@chatroom",
                owner: "wxid_bob",
                members: &[
                    ("wxid_me", ""),
                    ("wxid_bob", "Bob in room"),
                    ("wxid_quiet", ""),
                ],
            }],
        );
        f.session_db(&[
            SessionSpec {
                username: "wxid_bob",
                summary: "see you",
                last_timestamp: T0 + 4 * DAY,
                unread: 2,
                last_msg_type: 1,
            },
            SessionSpec {
                username: "room1@chatroom",
                summary: "ok",
                last_timestamp: T0 + 2 * DAY,
                unread: 0,
                last_msg_type: 1,
            },
            SessionSpec {
                username: "gh_news",
                summary: "daily",
                last_timestamp: T0,
                unread: 0,
                last_msg_type: 49,
            },
        ]);
        f.message_shard(
            0,
            &[
                (
                    "wxid_bob",
                    vec![
                        MsgSpec::text(1, "wxid_bob", T0, "hello"),
                        MsgSpec::text(2, "wxid_me", T0 + 60, "hi bob"),
                        MsgSpec::text(
                            3,
                            "wxid_bob",
                            T0 + DAY,
                            "<msg><img md5=\"aabbccddeeff00112233445566778899\"/></msg>",
                        )
                        .of_type(3),
                    ],
                ),
                (
                    "room1@chatroom",
                    vec![
                        MsgSpec::text(1, "wxid_bob", T0 + 10, "wxid_bob:\nwelcome"),
                        MsgSpec::text(2, "wxid_me", T0 + 20, "thanks"),
                        MsgSpec::text(3, "wxid_quiet", T0 + 2 * DAY, "wxid_quiet:\nok"),
                    ],
                ),
            ],
        );
        f.message_shard(
            1,
            &[(
                "wxid_bob",
                vec![
                    MsgSpec::text(4, "wxid_me", T0 + 3 * DAY, "later, compressed").compressed(),
                    MsgSpec::text(5, "wxid_bob", T0 + 4 * DAY, "see you"),
                ],
            )],
        );
        f.sns_db(&[
            SnsPostSpec {
                tid: 1001,
                user: "wxid_bob",
                create_time: T0 + 100,
                desc: "bob trip",
                kind: 1,
                media: 2,
                likes: &[("wxid_me", "Me Nick"), ("wxid_carol", "Carol")],
                extra: "",
            },
            SnsPostSpec {
                tid: 1002,
                user: "wxid_me",
                create_time: T0 + 200,
                desc: "my post",
                kind: 2,
                media: 0,
                likes: &[("wxid_bob", "Bob"), ("wxid_carol", "Carol")],
                extra: "",
            },
            SnsPostSpec {
                tid: 1003,
                user: "wxid_me",
                create_time: T0 + DAY,
                desc: "second post",
                kind: 1,
                media: 1,
                likes: &[("wxid_bob", "Bob")],
                extra: "",
            },
            SnsPostSpec {
                tid: 1004,
                user: "wxid_carol",
                create_time: T0 + 2 * DAY,
                desc: "carol video",
                kind: 15,
                media: 1,
                likes: &[("wxid_me", "Me Nick")],
                extra: "",
            },
            SnsPostSpec {
                tid: -5,
                user: "wxid_bob",
                create_time: T0 + 3 * DAY,
                desc: "old style",
                kind: 1,
                media: 0,
                likes: &[],
                extra: "",
            },
        ]);
        f.media_db(
            0,
            &[
                VoiceSpec {
                    chat: "wxid_bob",
                    create_time: T0 + 500,
                    local_id: 6,
                    svr_id: 9_000_000_000_001,
                    data: vec![1, 2, 3, 4],
                    index: "0",
                },
                VoiceSpec {
                    chat: "wxid_bob",
                    create_time: T0 + 600,
                    local_id: 7,
                    svr_id: 9_000_000_000_002,
                    data: vec![0xaa, 0xbb],
                    index: "0",
                },
                VoiceSpec {
                    chat: "wxid_bob",
                    create_time: T0 + 600,
                    local_id: 7,
                    svr_id: 9_000_000_000_002,
                    data: vec![0xcc],
                    index: "1",
                },
                VoiceSpec {
                    chat: "wxid_me",
                    create_time: T0 + 700,
                    local_id: 8,
                    svr_id: 9_000_000_000_003,
                    data: vec![9],
                    index: "0",
                },
            ],
        );
        f.emoticon_db(
            &[
                EmoticonSpec {
                    md5: "AABBCCDDEEFF00112233445566778899",
                    caption: "smile",
                    cdn_url: "http://cdn/e1",
                    extern_url: "http://ext/e1",
                },
                EmoticonSpec {
                    md5: "11223344556677889900aabbccddeeff",
                    caption: "",
                    cdn_url: "",
                    extern_url: "http://ext/e2",
                },
            ],
            &[
                ("deadbeefdeadbeefdeadbeefdeadbeef", "en", "store caption"),
                ("deadbeefdeadbeefdeadbeefdeadbeef", "zh_CN", "商店表情"),
            ],
        );
        f
    }
}
