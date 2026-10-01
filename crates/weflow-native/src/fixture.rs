//! Builds a synthetic WeChat account on disk (real encrypted SQLCipher databases) for tests.
//!
//! Enabled by the `test-fixtures` feature (and always for this crate's own tests). The databases use the
//! real table layouts, so everything above the file format — table lookup, sender resolution, contacts,
//! groups — is exercised exactly as it is on a real account.

use std::path::{Path, PathBuf};

use rusqlite::{params, Connection};

use crate::native_db::hex;
use crate::native_msg::table_name_for;
use crate::sqlcipher::testutil::{encrypt_db, plain_db_with, KEY, SALT};
use crate::sqlcipher::PageCipher;

pub struct SessionSpec {
    pub username: &'static str,
    pub summary: &'static str,
    pub last_timestamp: i64,
    pub unread: i64,
    pub last_msg_type: i64,
}

#[derive(Clone)]
pub struct ContactSpec {
    pub username: &'static str,
    pub local_type: i64,
    pub nick_name: &'static str,
    pub remark: &'static str,
    pub alias: &'static str,
    pub avatar: &'static str,
    pub flag: i64,
    pub chat_room_notify: i64,
    /// The `description` column.
    pub description: &'static str,
    /// Raw protobuf bytes of the `extra_buffer` column (signature, region, label ids).
    pub extra: &'static [u8],
}

impl ContactSpec {
    pub const fn new(username: &'static str, local_type: i64, nick_name: &'static str) -> Self {
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

pub struct RoomSpec {
    pub username: &'static str,
    pub owner: &'static str,
    /// (member username, group nickname or "" for none)
    pub members: &'static [(&'static str, &'static str)],
}

#[derive(Clone)]
pub struct MsgSpec {
    pub local_id: i64,
    pub server_id: i64,
    pub local_type: i64,
    /// Username of the sender; empty for none.
    pub sender: &'static str,
    pub create_time: i64,
    pub content: String,
    /// Store `content` as a zstd blob, as WCDB does for some message types.
    pub compressed: bool,
    /// The message `source` column (msgsource XML), e.g. an `<atuserlist>`.
    pub source: String,
}

impl MsgSpec {
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
        }
    }
    pub fn with_source(mut self, source: &str) -> Self {
        self.source = source.to_string();
        self
    }
    pub fn of_type(mut self, local_type: i64) -> Self {
        self.local_type = local_type;
        self
    }
    pub fn compressed(mut self) -> Self {
        self.compressed = true;
        self
    }
}

/// One Moments post for [`Fixture::sns_db`].
pub struct SnsPostSpec {
    /// Stored as-is (a negative value stands for an unsigned snowflake above `i64::MAX`).
    pub tid: i64,
    pub user: &'static str,
    pub create_time: i64,
    pub desc: &'static str,
    pub kind: i64,
    pub media: usize,
    /// (username, nickname) of each liker
    pub likes: &'static [(&'static str, &'static str)],
    /// Raw XML appended inside `<TimelineObject>` (location, comment list, ...).
    pub extra: &'static str,
}

impl SnsPostSpec {
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
            self.tid as u64, self.user, self.create_time, self.desc, self.kind, extra = self.extra
        )
    }
}

/// One `VoiceInfo` row for [`Fixture::media_db`].
pub struct VoiceSpec {
    pub chat: &'static str,
    pub create_time: i64,
    pub local_id: i64,
    pub svr_id: i64,
    pub data: Vec<u8>,
    pub index: &'static str,
}

/// One own-sticker row for [`Fixture::emoticon_db`].
pub struct EmoticonSpec {
    pub md5: &'static str,
    pub caption: &'static str,
    pub cdn_url: &'static str,
    pub extern_url: &'static str,
}

/// One row of `hardlink.db`: `kind` 1 = file, 2 = image, 3 = video, 4 = directory entry.
/// Images live in `<dir1>/<dir2>/Img/`, videos in `<dir1>/`, files in `<dir1>/`.
#[derive(Clone)]
pub struct HardlinkSpec {
    pub md5: &'static str,
    pub file_name: &'static str,
    pub kind: i64,
    pub dir1: &'static str,
    pub dir2: &'static str,
    pub size: i64,
    pub modify_time: i64,
}

impl HardlinkSpec {
    /// An image rendition stored under the talker directory `talker_dir` and month `month`.
    pub fn image(
        md5: &'static str,
        file_name: &'static str,
        talker_dir: &'static str,
        month: &'static str,
    ) -> Self {
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

    pub fn video(md5: &'static str, file_name: &'static str, month: &'static str) -> Self {
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
}

pub struct Fixture {
    pub root: PathBuf,
    pub account_dir: PathBuf,
    cipher: PageCipher,
}

impl Fixture {
    /// Create `<root>/<account>/db_storage/...`; `root` is wiped first.
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
            cipher: PageCipher::derive(&KEY, &SALT),
        }
    }

    pub fn key_hex(&self) -> String {
        hex(&KEY)
    }

    pub fn db_storage(&self) -> PathBuf {
        self.account_dir.join("db_storage")
    }

    fn write(&self, rel: &str, plain: Vec<u8>) {
        std::fs::write(
            self.db_storage().join(rel),
            encrypt_db(&plain, &self.cipher),
        )
        .unwrap();
    }

    pub fn sns_db(&self, posts: &[SnsPostSpec]) {
        let rows: Vec<(i64, &'static str, String)> =
            posts.iter().map(|p| (p.tid, p.user, p.xml())).collect();
        self.write(
            "sns/sns.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch("create table SnsTimeLine(tid integer primary key desc, user_name text, content text, pack_info_buf text)").unwrap();
                for (tid, user, xml) in &rows {
                    c.execute("insert into SnsTimeLine(tid, user_name, content) values (?1, ?2, ?3)", params![tid, user, xml]).unwrap();
                }
            }),
        );
    }

    pub fn media_db(&self, n: u32, voices: &[VoiceSpec]) {
        let rows: Vec<(i64, i64, i64, i64, Vec<u8>, &'static str)> = {
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
        let mut names: Vec<&'static str> = Vec::new();
        for v in voices {
            if !names.contains(&v.chat) {
                names.push(v.chat);
            }
        }
        self.write(
            &format!("message/media_{n}.db"),
            plain_db_with(move |c: &Connection| {
                c.execute_batch(
                    "create table Name2Id(user_name text primary key); create table TimeStamp(timestamp integer);
                     create table VoiceInfo(chat_name_id integer, create_time integer, local_id integer, svr_id integer, voice_data blob, data_index text default '0');",
                )
                .unwrap();
                for (i, name) in names.iter().enumerate() {
                    c.execute("insert into Name2Id(rowid, user_name) values (?1, ?2)", params![i as i64 + 1, name]).unwrap();
                }
                for (chat, ct, local, svr, data, idx) in &rows {
                    c.execute("insert into VoiceInfo values (?1, ?2, ?3, ?4, ?5, ?6)", params![chat, ct, local, svr, data, idx]).unwrap();
                }
            }),
        );
    }

    /// `store` rows are (md5, language, caption) of the sticker-store caption table.
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
                c.execute_batch(
                    "create table kNonStoreEmoticonTable(type integer, md5 text, caption text, product_id text, aes_key text, thumb_url text, tp_url text, auth_key text, cdn_url text, extern_url text, extern_md5 text, encrypt_url text, designer_id text, activity_id text);
                     create table kStoreEmoticonCaptionsTable(package_id_ text, md5_ text, language_ text, caption_ text);",
                )
                .unwrap();
                for (md5, caption, cdn, ext) in &own {
                    c.execute("insert into kNonStoreEmoticonTable(type, md5, caption, cdn_url, extern_url) values (1, ?1, ?2, ?3, ?4)", params![md5, caption, cdn, ext]).unwrap();
                }
                for (md5, lang, caption) in &store {
                    c.execute("insert into kStoreEmoticonCaptionsTable values ('pkg', ?1, ?2, ?3)", params![md5, lang, caption]).unwrap();
                }
            }),
        );
    }

    /// `hardlink/hardlink.db` with the image/video/file tables and the `dir2id` directory names.
    pub fn hardlink_db(&self, rows: &[HardlinkSpec]) {
        let rows = rows.to_vec();
        self.write(
            "hardlink/hardlink.db",
            plain_db_with(move |c: &Connection| {
                let table = "(md5_hash integer, md5 text, type integer, file_name text, file_size integer, modify_time integer, dir1 integer, dir2 integer, _rowid_ integer primary key asc, extra_buffer blob)";
                c.execute_batch(&format!(
                    "create table dir2id(username text primary key); create table image_hardlink_info_v4{table}; \
                     create table video_hardlink_info_v4{table}; create table file_hardlink_info_v4{table};"
                ))
                .unwrap();
                fn id(name: &'static str, dirs: &mut Vec<&'static str>) -> i64 {
                    if name.is_empty() {
                        return 0;
                    }
                    if !dirs.contains(&name) {
                        dirs.push(name);
                    }
                    dirs.iter().position(|d| *d == name).unwrap() as i64 + 1
                }
                let mut dir_list: Vec<&'static str> = Vec::new();
                for r in &rows {
                    let (d1, d2) = (id(r.dir1, &mut dir_list), id(r.dir2, &mut dir_list));
                    let t = match r.kind {
                        2 | 4 => "image_hardlink_info_v4",
                        3 => "video_hardlink_info_v4",
                        _ => "file_hardlink_info_v4",
                    };
                    c.execute(
                        &format!("insert into {t}(md5, type, file_name, file_size, modify_time, dir1, dir2) values (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
                        params![r.md5, r.kind, r.file_name, r.size, r.modify_time, d1, d2],
                    )
                    .unwrap();
                }
                for (i, d) in dir_list.iter().enumerate() {
                    c.execute("insert into dir2id(rowid, username) values (?1, ?2)", params![i as i64 + 1, d]).unwrap();
                }
            }),
        );
    }

    /// `head_image/head_image.db`: `(username, image bytes)` per avatar.
    pub fn head_image_db(&self, avatars: &[(&'static str, &[u8])]) {
        let avatars: Vec<(&'static str, Vec<u8>)> =
            avatars.iter().map(|(u, b)| (*u, b.to_vec())).collect();
        self.write(
            "head_image/head_image.db",
            plain_db_with(move |c: &Connection| {
                c.execute_batch("create table head_image(username text primary key, md5 text, image_buffer blob, update_time integer)").unwrap();
                for (u, b) in &avatars {
                    c.execute("insert into head_image values (?1, 'md5', ?2, 1)", params![u, b]).unwrap();
                }
            }),
        );
    }

    pub fn session_db(&self, sessions: &[SessionSpec]) {
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
                c.execute_batch(
                    "create table SessionTable(username text primary key, type integer, unread_count integer, unread_first_msg_srv_id integer, \
                     unread_first_pat_msg_local_id integer, unread_first_pat_msg_sort_seq integer, is_hidden integer, summary, draft text, \
                     status integer, last_timestamp integer, sort_timestamp integer, last_clear_unread_timestamp integer, last_msg_locald_id integer, \
                     last_msg_type integer, last_msg_sub_type integer, last_msg_sender text, last_sender_display_name text, last_msg_ext_type integer)",
                )
                .unwrap();
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

    pub fn contact_db(&self, contacts: &[ContactSpec], rooms: &[RoomSpec]) {
        self.contact_db_with_labels(contacts, rooms, &[]);
    }

    /// Like [`contact_db`](Self::contact_db) with `(label id, name)` rows in `contact_label`.
    pub fn contact_db_with_labels(
        &self,
        contacts: &[ContactSpec],
        rooms: &[RoomSpec],
        labels: &[(i64, &'static str)],
    ) {
        let labels = labels.to_vec();
        let contacts: Vec<(i64, ContactSpec)> = contacts
            .iter()
            .cloned()
            .enumerate()
            .map(|(i, c)| (i as i64 + 1, c))
            .collect();
        let ids: std::collections::HashMap<&str, i64> =
            contacts.iter().map(|(id, c)| (c.username, *id)).collect();
        let rooms: Vec<(
            i64,
            &'static str,
            &'static str,
            &'static [(&'static str, &'static str)],
            Vec<i64>,
        )> = rooms
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
                let cols = "id integer primary key, username text, local_type integer, alias text, encrypt_username text, flag integer, \
                            delete_flag integer, verify_flag integer, remark text, remark_quan_pin text, remark_pin_yin_initial text, nick_name text, \
                            pin_yin_initial text, quan_pin text, big_head_url text, small_head_url text, head_img_md5 text, chat_room_notify integer, \
                            is_in_chat_room integer, description text, extra_buffer blob, chat_room_type integer";
                c.execute_batch(&format!(
                    "create table contact({cols}); create table stranger({cols}); create table name2id(username text primary key);
                     create table contact_label(label_id_ integer primary key, label_name_ text, sort_order_ integer);
                     create table chat_room(id integer primary key, username text, owner text, ext_buffer blob);
                     create table chatroom_member(room_id integer, member_id integer, constraint room_member unique(room_id, member_id));"
                ))
                .unwrap();
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
                    let mut ext = vec![0x08, 0x01];
                    for (m, nick) in members.iter().filter(|(_, n)| !n.is_empty()) {
                        let mut inner = vec![0x0a, m.len() as u8];
                        inner.extend(m.as_bytes());
                        inner.extend([0x12, nick.len() as u8]);
                        inner.extend(nick.as_bytes());
                        ext.extend([0x0a, inner.len() as u8]);
                        ext.extend(inner);
                    }
                    c.execute("insert into chat_room(id, username, owner, ext_buffer) values (?1,?2,?3,?4)", params![rid, user, owner, ext]).unwrap();
                    for mid in member_ids {
                        c.execute("insert into chatroom_member(room_id, member_id) values (?1, ?2)", params![rid, mid]).unwrap();
                    }
                }
            }),
        );
    }

    /// Add `msgs` of `session` to `message/message_<n>.db` (created when missing, merged when present
    /// is not supported: one call per session per shard, several sessions per shard are fine).
    pub fn message_shard(&self, n: u32, sessions: &[(&str, Vec<MsgSpec>)]) {
        let sessions: Vec<(String, Vec<MsgSpec>)> = sessions
            .iter()
            .map(|(s, m)| (s.to_string(), m.clone()))
            .collect();
        self.write(
            &format!("message/message_{n}.db"),
            plain_db_with(move |c: &Connection| {
                c.execute_batch("create table Name2Id(user_name text primary key, is_session integer)").unwrap();
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
                    c.execute_batch(&format!(
                        "create table \"{table}\"(local_id integer primary key autoincrement, server_id integer, local_type integer, sort_seq integer, \
                         real_sender_id integer, create_time integer, status integer, upload_status integer, download_status integer, server_seq integer, \
                         origin_source integer, source text, message_content, compress_content, packed_info_data blob, \
                         WCDB_CT_message_content integer default null, WCDB_CT_source integer default null)"
                    ))
                    .unwrap();
                    for m in msgs {
                        let sender = if m.sender.is_empty() { 0 } else { id_of(c, m.sender) };
                        let content: Box<dyn rusqlite::ToSql> =
                            if m.compressed { Box::new(zstd::encode_all(m.content.as_bytes(), 1).unwrap()) } else { Box::new(m.content.clone()) };
                        c.execute(
                            &format!(
                                "insert into \"{table}\"(local_id, server_id, local_type, sort_seq, real_sender_id, create_time, status, message_content, source) \
                                 values (?1,?2,?3,?4,?5,?6,2,?7,?8)"
                            ),
                            params![m.local_id, m.server_id, m.local_type, m.create_time * 1000, sender, m.create_time, content, m.source],
                        )
                        .unwrap();
                    }
                }
            }),
        );
    }
}

/// Base timestamp of the standard world (2023-11-14 22:13:20 UTC).
pub const T0: i64 = 1_700_000_000;
pub const DAY: i64 = 86_400;

impl Fixture {
    /// A small but complete account: `wxid_me` talking to `wxid_bob` (two message shards, one image,
    /// one zstd-compressed message) and to the group `room1@chatroom` (owner bob, nicknames, 3 members).
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
                ContactSpec::new("wxid_carol", 1, "Carol"),
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
