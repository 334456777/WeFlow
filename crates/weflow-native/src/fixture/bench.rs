//! 与真实账号同样大、同样多样的合成账号，即基准账号，供基准测试的工作流使用（#259、#488）。
//!
//! 各种分布取自一个真实账号的汇总 profile（`weflow-core` 的 `account_profile_probe`）：消息类型、文字长度、
//! 时段和消息间隔的比例，群里少数成员的发言占比，一张图片的哪些文件在电脑上，等等。
//! 只照搬这些数字，从不照搬名字或消息。
//!
//! 结构与微信写的相同：每个数据库有自己的盐，所以每打开一个都要派生它的页密钥（#1127）；有几个
//! `message_N.db`，每个数据库的本地 ID 重新从头开始，时间范围互相重叠，每个数据库最新的消息还在它的 WAL 里，
//! 少数 `sort_seq` 错位（#133）或缺失；各种常见子类型的 app 消息，XML 用 zstd 压缩；引用消息按文字或者按转义过一次的
//! XML 引用另一条消息，合并转发的聊天记录里有文字、图片和视频，与真实账号一致（#1242）；
//! 公众号的消息和微信支付的通知在 `biz_message_0.db` 里（#1068）；还有图片、视频、语音和文件的目录，
//! 以及它们在 `hardlink.db` 中的行。图片是 V2 的 `.dat` 文件，里面是与真实大小相同的 JPEG、PNG 或 WXGF；
//! JPEG 只有文件头和填充，不是真的图片，WXGF 是与真实账号的图片同样大小的完整 HEVC 图片（#1221）。
//!
//! [`build`] 用自己的生成器（`Rng`）从 [`BenchSpec::seed`] 生成一切，所以同一个种子在每台机器上、
//! 用每个版本的依赖都得到相同的字节。
#![expect(
    clippy::cast_possible_truncation,
    clippy::cast_possible_wrap,
    clippy::cast_precision_loss,
    clippy::cast_sign_loss,
    reason = "生成的计数、大小和时间都是放得下的小值（`f64` 能精确表示），而且都不是负数"
)]

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use anyhow::{ensure, Result};
use serde_json::{json, Value};

use super::{
    message_table_schema, ContactSpec, Fixture, FtsSpec, HardlinkSpec, RoomSpec, SessionSpec,
    SnsPostSpec, VoiceSpec, NAME2ID_SCHEMA,
};
use crate::msg::table_name_for;
use crate::sqlcipher::testutil::{encrypt_db, encrypt_wal, plain_connection, plain_image};

mod files;
mod messages;
mod official;
mod profile;
mod quote;
mod record;
mod rng;
mod words;
mod world;

use messages::{source, zstd_or_plain, Recent, Row, Shard, Writer, Written};
use profile::app;
pub use profile::{
    GroupProfile, MediaProfile, MessageProfile, Profile, Spread, TextProfile, TimeProfile,
};
use rng::{Rng, Weighted};
use world::{assign_shards, Conversation, Kind, World};

/// 账号所有者的用户名。
pub const OWNER: &str = "wxid_benchme";
/// 账号目录：[`OWNER`] 加上 `_` 和四个字符，与微信的命名方式相同。
pub const ACCOUNT_DIR: &str = "wxid_benchme_b3n9";
/// 账号 `.dat` 文件的图片 XOR 密钥（V2 格式的文件开头用 AES 加密，结尾用 XOR）。
pub const IMAGE_XOR_KEY: u8 = 0x5a;
/// 账号 `.dat` 文件的图片 AES 密钥，与 [`IMAGE_XOR_KEY`] 配套。
pub const IMAGE_AES_KEY: &str = "b3nchm4rk1m4g3k3";
/// 账号中最新的时刻（2025-10-09 08:53:20 UTC）：固定不变，同一个种子在哪一天生成的账号都相同。
const END: i64 = 1_760_000_000;

const DAY: i64 = 86_400;
/// 账号所在的时区 UTC+8：小时、日期和文件的月份目录都按它计算。
const TZ: i64 = 8 * 3600;
const END_DAY: i64 = (END + TZ) / DAY;
const MIB: usize = 1 << 20;
/// 引用消息可以引用的之前的消息条数：足够多，偶尔才被引用的类型（如视频）也在其中。
const QUOTABLE: usize = 200;

/// 要生成的基准账号的参数。
#[derive(Clone, Debug)]
pub struct BenchSpec {
    /// 所有随机选择的起点：同一个种子生成同一个账号。
    pub seed: u64,
    /// 最大的群的消息数。
    pub group_messages: usize,
    /// 最大的群以外的会话数：私聊，以及几个较小的群。
    pub conversations: usize,
    /// 消息库（`message_N.db`）的个数。
    pub shards: usize,
    /// 是否写出图片、视频和文件（生成的大部分时间和磁盘空间都花在这里）。
    pub media: bool,
    /// 从真实账号照搬的分布。
    pub profile: Profile,
}

impl Default for BenchSpec {
    fn default() -> Self {
        Self {
            seed: 1,
            group_messages: 200_000,
            conversations: 300,
            shards: 2,
            media: true,
            profile: Profile::default(),
        }
    }
}

/// 比这更长的文字消息和系统消息压缩存储，与其他各类消息一样。
const ZSTD_ABOVE: usize = 150;

/// 每个数据库中还留在 WAL 里的最新消息的条数。
const WAL_MESSAGES: usize = 1500;
/// 微信的搜索索引收录 WAL 之前的文字消息，分在这么多张表里。
const FTS_TABLES: usize = 5;
const SNS_PER_FRIEND: f64 = 1.1;

/// 在 `root` 下生成账号（先清空 `root`），返回其中的内容：每个会话的用户名、种类和消息数，每种类型的消息数，
/// 以及写出的文件。
///
/// # Errors
///
/// `spec` 不合法（没有消息库、最大的群没有消息，或者 profile 没有通过检查），或者写入失败。
pub fn build(root: &Path, spec: &BenchSpec) -> Result<Value> {
    ensure!(spec.shards >= 1, "at least one message database");
    ensure!(
        spec.group_messages >= 1,
        "the biggest group needs a message"
    );
    let f = Fixture::new(root, ACCOUNT_DIR).with_own_salts(spec.seed);
    let rng = Rng::new(spec.seed);
    let world = World::new(&mut rng.fork(1), spec);
    let shard_of = assign_shards(&mut rng.fork(2), &world.conversations, spec.shards);
    write_contacts(&f, &world);
    let profile = &spec.profile;
    profile.check()?;
    let mut writer = Writer {
        world: &world,
        profile,
        account_dir: &f.account_dir,
        media: spec.media,
        written: Written::default(),
    };

    // 每个数据库最新的消息放进 WAL
    let mut shards: Vec<Shard> = (0..spec.shards)
        .map(|k| -> Result<Shard> {
            let mut times: Vec<(i64, usize)> = world
                .conversations
                .iter()
                .zip(&shard_of)
                .enumerate()
                .filter(|(_, (c, _))| c.kind != Kind::Official)
                .flat_map(|(ci, (c, s))| {
                    c.times
                        .iter()
                        .zip(s)
                        .filter(|(_, &sk)| sk == k)
                        .map(move |(&t, _)| (t, ci))
                })
                .collect();
            times.sort_unstable();
            let wal = WAL_MESSAGES.min(times.len() / 10);
            let conn = plain_connection();
            conn.set_prepared_statement_cache_capacity(64);
            conn.execute_batch(NAME2ID_SCHEMA)?;
            conn.execute_batch("begin")?;
            Ok(Shard {
                conn,
                ids: HashMap::new(),
                tables: HashSet::new(),
                wal_from: times
                    .get(times.len() - wal)
                    .copied()
                    .unwrap_or((i64::MAX, usize::MAX)),
                late: Vec::new(),
            })
        })
        .collect::<Result<_>>()?;

    let types = Weighted::new(profile.messages.types.iter().map(|t| t.1));
    let mut by_type: BTreeMap<i64, u64> = BTreeMap::new();
    let mut fts: Vec<Vec<FtsSpec<'_>>> = (0..FTS_TABLES).map(|_| Vec::new()).collect();
    let mut sessions = Vec::new();
    let mut manifest_conversations = Vec::new();
    let official = official::write(&f, &world, &rng.fork(5))?;
    for (local_type, n) in &official.by_type {
        *by_type.entry(*local_type).or_default() += n;
    }
    for (ci, c) in world.conversations.iter().enumerate() {
        let mut rng = rng.fork(1000 + ci as u64);
        if c.kind == Kind::Official {
            let (messages, newest) = &official.newest[&ci];
            sessions.push((ci, newest.time, newest.local_type, newest.summary.clone()));
            manifest_conversations
                .push(json!({"username": c.username, "kind": c.kind.name(), "messages": messages}));
            continue;
        }
        let table = table_name_for(&c.username);
        let speakers =
            Weighted::new((0..c.speakers.max(1)).map(|r| {
                (r as f64 + 1.0 + profile.groups.speaker_q).powf(-profile.groups.speaker_s)
            }));
        let sent = if c.kind == Kind::Group {
            profile.messages.sent_in_group
        } else {
            profile.messages.sent_in_private
        };
        let mut next_local: Vec<i64> = vec![0; spec.shards];
        let mut recent: VecDeque<Recent> = VecDeque::new();
        let mut per_shard = vec![0u64; spec.shards];
        let mut wal_rows = 0u64;
        let mut last = (0, 1, String::new());
        let mut same_second = (0i64, 0i64);
        for (&time, &k) in c.times.iter().zip(&shard_of[ci]) {
            let local_type = profile.messages.types[types.draw(&mut rng)].0;
            let local_type = if c.kind == Kind::Private && local_type == app(87) {
                1
            } else {
                local_type
            };
            let sender = if local_type == 10000 {
                None
            } else if rng.chance(sent) {
                Some(0)
            } else {
                Some(c.members[speakers.draw(&mut rng).min(c.members.len() - 1)])
            };
            let shard = &mut shards[k];
            if shard.tables.insert(table.clone()) {
                shard.conn.execute_batch(&message_table_schema(&table))?;
                shard.id(&c.username, true)?;
            }
            next_local[k] += 1;
            let local_id = next_local[k];
            let server_id = (rng.next_u64() >> 1) as i64;
            let made = writer.make(
                &mut rng,
                ci,
                local_type,
                sender.unwrap_or(0),
                time,
                (local_id, server_id),
                &recent,
            )?;
            same_second = if same_second.0 == time {
                (time, same_second.1 + 1)
            } else {
                (time, 0)
            };
            let sort_seq = if rng.chance(profile.messages.sort_seq_missing) {
                Some(0)
            } else if rng.chance(profile.messages.sort_seq_off) {
                Some((time + rng.between(-30 * DAY, 30 * DAY)) * 1000)
            } else {
                Some(time * 1000 + same_second.1)
            };
            let in_group_from_other = c.kind == Kind::Group && sender.is_some_and(|s| s != 0);
            let content = if in_group_from_other && !made.content.is_empty() {
                format!(
                    "{}:\n{}",
                    world.people[sender.unwrap_or(0)].username,
                    made.content
                )
            } else {
                made.content.clone()
            };
            let compress = !matches!(local_type, 1 | 10000) || content.len() > ZSTD_ABOVE;
            let (content_bytes, compressed) = zstd_or_plain(content, compress)?;
            let at = made.mention.map(|m| world.user(m));
            let source_text = if rng.chance(profile.messages.with_source) {
                source(&mut rng, c.kind, c.members.len(), at)
            } else {
                String::new()
            };
            let source = if source_text.is_empty() {
                // 空的 source 存成空文字
                (Some(Vec::new()), false)
            } else {
                let compress = rng.chance(profile.messages.source_compressed);
                let (bytes, z) = zstd_or_plain(source_text.clone(), compress)?;
                (Some(bytes), z)
            };
            let status = if sender == Some(0) {
                2
            } else if rng.chance(0.153) {
                3
            } else {
                4
            };
            let row = Row {
                local_id,
                server_id,
                local_type,
                sort_seq,
                sender,
                time,
                status,
                source: source.0,
                source_compressed: source.1,
                content: content_bytes,
                compressed,
            };
            let late = (time, ci) >= shard.wal_from;
            if late {
                shard.late.push((table.clone(), row));
                wal_rows += 1;
            } else {
                shard.insert(&world, &table, &row)?;
                if let Some(text) = &made.indexed {
                    fts[ci % FTS_TABLES].push(FtsSpec {
                        text: text.clone(),
                        local_id,
                        sort_seq: sort_seq.unwrap_or(0),
                        local_type,
                        session: &c.username,
                        create_time: time,
                    });
                }
            }
            per_shard[k] += 1;
            *by_type.entry(local_type).or_default() += 1;
            recent.push_back(Recent {
                server_id,
                sender: sender.unwrap_or(0),
                local_type,
                time,
                plain: made.indexed.clone().unwrap_or_else(|| made.summary.clone()),
                xml: match &made.quoted_as {
                    Some(xml) => xml.clone(),
                    None if local_type == 1 => String::new(),
                    None => made.content.clone(),
                },
                source: source_text,
            });
            if recent.len() > QUOTABLE {
                recent.pop_front();
            }
            let summary = match sender {
                Some(s) if c.kind == Kind::Group && s != 0 => {
                    format!("{}: {}", world.people[s].nick, made.summary)
                }
                _ => made.summary,
            };
            last = (time, local_type, summary);
        }
        sessions.push((ci, last.0, last.1, last.2));
        manifest_conversations.push(json!({
            "username": c.username,
            "kind": c.kind.name(),
            "messages": c.times.len(),
            "per_shard": per_shard,
            "in_wal": wal_rows,
        }));
    }

    let db_dir = f.db_storage().join("message");
    let mut wal_messages = 0;
    for (k, mut shard) in shards.into_iter().enumerate() {
        shard.conn.execute_batch("commit")?;
        let base = plain_image(&shard.conn);
        shard.conn.execute_batch("begin")?;
        let late = std::mem::take(&mut shard.late);
        wal_messages += late.len();
        for (table, row) in &late {
            shard.insert(&world, table, row)?;
        }
        shard.conn.execute_batch("commit")?;
        let full = plain_image(&shard.conn);
        let (db, wal) = f.with_cipher(&format!("message/message_{k}.db"), |cipher| {
            let wal = (base != full).then(|| {
                let salt = (0x5eed_0000 | k as u32, (spec.seed as u32) ^ 0x2468_ace0);
                encrypt_wal(cipher, &base, &full, salt)
            });
            (encrypt_db(&base, cipher), wal)
        });
        std::fs::write(db_dir.join(format!("message_{k}.db")), db)?;
        if let Some(wal) = wal {
            std::fs::write(db_dir.join(format!("message_{k}.db-wal")), wal)?;
        }
    }

    let summaries: Vec<String> = sessions.iter().map(|s| s.3.clone()).collect();
    let mut rng_sessions = rng.fork(3);
    let session_specs: Vec<SessionSpec<'_>> = sessions
        .iter()
        .zip(&summaries)
        .map(|((ci, t, ty, _), summary)| SessionSpec {
            username: &world.conversations[*ci].username,
            summary,
            last_timestamp: *t,
            unread: if rng_sessions.chance(0.02) {
                rng_sessions.between(1, 30)
            } else {
                0
            },
            last_msg_type: ty & 0xFFFF_FFFF,
        })
        .collect();
    f.session_db(&session_specs);
    f.message_fts_db(&fts);
    let voices: Vec<VoiceSpec<'_>> = std::mem::take(&mut writer.written.voices)
        .into_iter()
        .map(|(ci, create_time, local_id, svr_id, data)| VoiceSpec {
            chat: &world.conversations[ci].username,
            create_time,
            local_id,
            svr_id,
            data,
            index: "0",
        })
        .collect();
    f.media_db(0, &voices);
    let links: Vec<HardlinkSpec<'_>> = writer
        .written
        .hardlinks
        .iter()
        .map(|h| HardlinkSpec {
            md5: &h.md5,
            file_name: &h.file_name,
            kind: h.kind,
            dir1: &h.dir1,
            dir2: &h.dir2,
            size: h.size,
            modify_time: h.modify_time,
        })
        .collect();
    f.hardlink_db(&links);
    write_sns(&f, &world, &mut rng.fork(4));
    f.emoticon_db(&[], &[]);
    f.head_image_db(&[]);

    let files: serde_json::Map<String, Value> = writer
        .written
        .files
        .iter()
        .map(|(k, (n, bytes))| (k.to_string(), json!({"files": n, "bytes": bytes})))
        .collect();
    Ok(json!({
        "seed": spec.seed,
        "group_messages": spec.group_messages,
        "conversations_asked": spec.conversations,
        "shards": spec.shards,
        "media": spec.media,
        "profile": {"measured": profile.measured, "hash": profile.hash()?},
        "account_dir": ACCOUNT_DIR,
        "messages": by_type.values().sum::<u64>(),
        "messages_in_wal": wal_messages,
        "by_type": by_type.iter().map(|(k, v)| (k.to_string(), json!(v))).collect::<serde_json::Map<_, _>>(),
        "voices": voices.len(),
        "hardlinks": links.len(),
        "files": files,
        "people": world.people.len(),
        "conversations": manifest_conversations,
    }))
}

fn write_contacts(f: &Fixture, world: &World) {
    let mut contacts: Vec<ContactSpec<'_>> = world
        .people
        .iter()
        .map(|p| ContactSpec {
            remark: &p.remark,
            alias: &p.alias,
            avatar: &p.avatar,
            ..ContactSpec::new(&p.username, p.local_type, &p.nick)
        })
        .collect();
    let groups: Vec<&Conversation> = world
        .conversations
        .iter()
        .filter(|c| c.kind == Kind::Group)
        .collect();
    for c in &groups {
        contacts.push(ContactSpec::new(&c.username, 2, &c.name));
    }
    // 约三成成员在群里有自己的群昵称
    let mut rng = Rng::new(0x9e0b);
    let nicknames: Vec<Vec<String>> = groups
        .iter()
        .map(|c| {
            c.members
                .iter()
                .map(|_| {
                    if rng.chance(0.3) {
                        world.words.person_name(&mut rng)
                    } else {
                        String::new()
                    }
                })
                .collect()
        })
        .collect();
    let members: Vec<Vec<(&str, &str)>> = groups
        .iter()
        .zip(&nicknames)
        .map(|(c, nicks)| {
            std::iter::once((OWNER, ""))
                .chain(
                    c.members
                        .iter()
                        .zip(nicks)
                        .map(|(m, n)| (world.people[*m].username.as_str(), n.as_str())),
                )
                .collect()
        })
        .collect();
    let rooms: Vec<RoomSpec<'_>> = groups
        .iter()
        .zip(&members)
        .map(|(c, m)| RoomSpec {
            username: &c.username,
            owner: &world.people[c.members[0]].username,
            members: m,
        })
        .collect();
    f.contact_db_with_labels(&contacts, &rooms, &[(1, "同事"), (2, "家人"), (3, "朋友")]);
}

fn write_sns(f: &Fixture, world: &World, rng: &mut Rng) {
    let friends: Vec<usize> = (0..world.people.len())
        .filter(|&p| world.people[p].local_type == 1)
        .collect();
    let n = (friends.len() as f64 * SNS_PER_FRIEND) as usize;
    let authors = Weighted::new((0..friends.len()).map(|r| 1.0 / (r as f64 + 1.0)));
    let descs: Vec<String> = (0..n).map(|_| world.words.message(rng)).collect();
    let likes: Vec<Vec<(&str, &str)>> = (0..n)
        .map(|_| {
            (0..rng.below(12))
                .map(|_| {
                    let p = &world.people[friends[rng.below(friends.len())]];
                    (p.username.as_str(), p.nick.as_str())
                })
                .collect()
        })
        .collect();
    let posts: Vec<SnsPostSpec<'_>> = (0..n)
        .map(|i| SnsPostSpec {
            tid: 13_000_000_000_000_000_000u64.wrapping_sub(i as u64 * 7_919) as i64,
            user: &world.people[friends[authors.draw(rng)]].username,
            create_time: END - i as i64 * (2 * 365 * DAY / n.max(1) as i64),
            desc: &descs[i],
            kind: if rng.chance(0.1) { 15 } else { 1 },
            media: rng.below(10),
            likes: &likes[i],
            extra: "",
        })
        .collect();
    f.sns_db(&posts);
}

/// [`build`] 在 `root` 下生成的账号目录。
pub fn account_dir(root: &Path) -> PathBuf {
    root.join(ACCOUNT_DIR)
}
