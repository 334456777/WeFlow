//! A synthetic account as big and as varied as a real one, for the benchmark workflow (#259, #488).
//!
//! The shapes come from the aggregate profile of a real account (`account_profile_probe` in `weflow-core`): the
//! shares of message types, text lengths, hours and gaps between messages, how much of a group a few members say,
//! which files of a picture are on the computer, and so on. Only those numbers are copied, never a name or a message.
//!
//! The structure is the one WeChat writes: several `message_N.db` whose local ids start again in each database and
//! whose time ranges overlap, the newest messages of each database still in its WAL, a few `sort_seq` out of step
//! (#133) or missing, app messages of every common kind as XML compressed with zstd, and the picture, video, voice
//! and file folders with their `hardlink.db` rows. Pictures are V2 `.dat` files holding a JPEG, PNG or WXGF of the
//! real sizes; the JPEGs are headers and padding, not pictures.
//!
//! [`build`] draws everything from [`BenchSpec::seed`] with its own generator ([`Rng`]), so one seed gives the same
//! bytes on every machine and with every dependency version.
#![expect(
    clippy::cast_possible_truncation,
    reason = "the generated counts, sizes and times are small values that fit"
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
mod profile;
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

/// The account's own user name, and its folder (the name and four more characters, as WeChat names it).
pub const OWNER: &str = "wxid_benchme";
pub const ACCOUNT_DIR: &str = "wxid_benchme_b3n9";
/// The image keys of the account's `.dat` files (V2: AES for the head, XOR for the tail).
pub const IMAGE_XOR_KEY: u8 = 0x5a;
pub const IMAGE_AES_KEY: &str = "b3nchm4rk1m4g3k3";
/// The newest moment of the account (2025-10-09 08:53:20 UTC): fixed, so a seed gives the same account on any day.
const END: i64 = 1_760_000_000;

const DAY: i64 = 86_400;
/// The account's clock, UTC+8: hours, days and the month folders of its files are in it.
const TZ: i64 = 8 * 3600;
const END_DAY: i64 = (END + TZ) / DAY;
const MIB: usize = 1 << 20;

/// What to build.
#[derive(Clone, Debug)]
pub struct BenchSpec {
    pub seed: u64,
    /// Messages of the biggest group.
    pub group_messages: usize,
    /// Conversations besides the biggest group: private chats, and a few smaller groups.
    pub conversations: usize,
    /// Message databases (`message_N.db`).
    pub shards: usize,
    /// Write the files of pictures, videos and files (most of the time and the disk space a build takes).
    pub media: bool,
    /// The shapes copied from a real account.
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

/// Text and system messages longer than this are stored compressed, like every other kind.
const ZSTD_ABOVE: usize = 150;

/// The newest messages of each database that are still in its WAL.
const WAL_MESSAGES: usize = 1500;
/// WeChat's search index holds the text messages up to the WAL ones, in this many tables.
const FTS_TABLES: usize = 5;
const SNS_PER_FRIEND: f64 = 1.1;

/// Builds the account under `root` (wiped first) and returns what is in it: per conversation, its user name, kind and
/// messages; per type, the messages; the files written.
pub fn build(root: &Path, spec: &BenchSpec) -> Result<Value> {
    ensure!(spec.shards >= 1, "at least one message database");
    ensure!(
        spec.group_messages >= 1,
        "the biggest group needs a message"
    );
    let f = Fixture::new(root, ACCOUNT_DIR);
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

    // the newest messages of each database are the WAL's
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
    for (ci, c) in world.conversations.iter().enumerate() {
        let mut rng = rng.fork(1000 + ci as u64);
        if c.kind == Kind::Official {
            sessions.push((
                ci,
                c.times[0],
                49,
                format!("{}：{}", c.name, world.words.text_plain(&mut rng, 12)),
            ));
            manifest_conversations
                .push(json!({"username": c.username, "kind": c.kind.name(), "messages": 0}));
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
            let source = if rng.chance(profile.messages.with_source) {
                let s = source(&mut rng, c.kind, c.members.len(), at);
                let (bytes, z) = zstd_or_plain(s, rng.chance(profile.messages.source_compressed))?;
                (Some(bytes), z)
            } else {
                // an empty source is stored as empty text
                (Some(Vec::new()), false)
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
                content: made.indexed.clone().unwrap_or_else(|| made.summary.clone()),
            });
            if recent.len() > 30 {
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
        let cipher = &f.cipher;
        std::fs::write(
            db_dir.join(format!("message_{k}.db")),
            encrypt_db(&base, cipher),
        )?;
        if base != full {
            let salt = (0x5eed_0000 | k as u32, (spec.seed as u32) ^ 0x2468_ace0);
            std::fs::write(
                db_dir.join(format!("message_{k}.db-wal")),
                encrypt_wal(cipher, &base, &full, salt),
            )?;
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
    // a third of the members have a nickname of their own in the group
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

/// Where [`build`] puts the account under `root`.
pub fn account_dir(root: &Path) -> PathBuf {
    root.join(ACCOUNT_DIR)
}
