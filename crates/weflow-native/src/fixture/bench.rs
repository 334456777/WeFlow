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

use aes::cipher::{BlockCipherEncrypt, KeyInit};
use anyhow::{ensure, Context, Result};
use md5::{Digest, Md5};
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::{
    message_table_schema, ContactSpec, Fixture, FtsSpec, HardlinkSpec, RoomSpec, SessionSpec,
    SnsPostSpec, VoiceSpec, NAME2ID_SCHEMA,
};
use crate::native_db::hex;
use crate::native_msg::table_name_for;
use crate::sqlcipher::testutil::{encrypt_db, encrypt_wal, plain_connection, plain_image};

/// The account's own user name, and its folder (the name and four more characters, as WeChat names it).
pub const OWNER: &str = "wxid_benchme";
pub const ACCOUNT_DIR: &str = "wxid_benchme_b3n9";
/// The image keys of the account's `.dat` files (V2: AES for the head, XOR for the tail).
pub const IMAGE_XOR_KEY: u8 = 0x5a;
pub const IMAGE_AES_KEY: &str = "b3nchm4rk1m4g3k3";
/// The newest moment of the account (2025-10-09 08:53:20 UTC): fixed, so a seed gives the same account on any day.
pub const END: i64 = 1_760_000_000;

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

// ---- the profile: shares and sizes only ----

/// `(median, 90th percentile, at most)` of a size.
pub type Spread = (f64, f64, f64);

/// The shapes the account copies from a real one (#488, #605): shares and sizes only, never a name or a message.
/// [`Profile::default`] is the one measured for #488 (the constants below); a machine may calibrate its own from a
/// real account (`account_profile_probe --profile-out`), which stays on that machine and is read by [`Profile::load`].
/// What it holds is never shown: a benchmark result names it by [`Profile::hash`] and the day it was measured.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// The day it was measured, `YYYY-MM-DD`.
    pub measured: String,
    pub messages: MessageProfile,
    pub time: TimeProfile,
    pub text: TextProfile,
    pub groups: GroupProfile,
    pub media: MediaProfile,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageProfile {
    /// `(local_type, share)`; an app message's local type is `(its type << 32) | 49`.
    pub types: Vec<(i64, f64)>,
    pub sent_in_group: f64,
    pub sent_in_private: f64,
    pub at_in_group: f64,
    pub with_source: f64,
    pub source_compressed: f64,
    pub sort_seq_off: f64,
    pub sort_seq_missing: f64,
    /// The content size in bytes by type (the size of the XML of all but text), and of the types not listed.
    pub content_bytes: Vec<(i64, Spread)>,
    pub content_bytes_other: Spread,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeProfile {
    /// Share of messages by hour of the day (UTC+8).
    pub hours: [f64; 24],
    /// Share of messages by day of the week, Monday first.
    pub weekdays: [f64; 7],
    /// Seconds between two messages of one burst, as `(from, to, share)`.
    pub burst_gaps: Vec<(i64, i64, f64)>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextProfile {
    /// Lengths in characters, as `(from, to, share)`.
    pub chars: Vec<(u32, u32, f64)>,
    pub with_emoji: f64,
    pub with_bracket_emoji: f64,
    pub with_newline: f64,
    pub with_odd_control: f64,
    pub with_url: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupProfile {
    /// The share of the conversations that are groups.
    pub share: f64,
    /// The biggest group: members, and how many of them ever speak.
    pub big_members: usize,
    pub big_speakers: usize,
    /// Who speaks in a group: the member of rank `r` (from 0) says `(r + 1 + q) ^ -s` of it.
    pub speaker_q: f64,
    pub speaker_s: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaProfile {
    /// Which files of a picture are on the computer: `(thumbnail, image, original, share)`.
    pub image_sets: Vec<(bool, bool, bool, f64)>,
    pub thumb_bytes: Spread,
    pub image_bytes: Spread,
    pub hd_bytes: Spread,
    /// What the image file holds, as `(png, jpeg, wxgf)` shares.
    pub image_formats: (f64, f64, f64),
    pub hd_formats: (f64, f64, f64),
    pub video_bytes: Spread,
    pub video_thumb_bytes: Spread,
    pub video_thumb_here: f64,
    pub video_here: f64,
    pub file_bytes: Spread,
    pub file_here: f64,
}

impl Default for Profile {
    fn default() -> Self {
        Self {
            measured: MEASURED.to_string(),
            messages: MessageProfile {
                types: TYPES.to_vec(),
                sent_in_group: SENT_IN_GROUP,
                sent_in_private: SENT_IN_PRIVATE,
                at_in_group: AT_IN_GROUP,
                with_source: WITH_SOURCE,
                source_compressed: SOURCE_COMPRESSED,
                sort_seq_off: SORT_SEQ_OFF,
                sort_seq_missing: SORT_SEQ_MISSING,
                content_bytes: CONTENT_BYTES.to_vec(),
                content_bytes_other: CONTENT_BYTES_OTHER,
            },
            time: TimeProfile {
                hours: HOURS,
                weekdays: WEEKDAYS,
                burst_gaps: BURST_GAPS.to_vec(),
            },
            text: TextProfile {
                chars: TEXT_CHARS.to_vec(),
                with_emoji: WITH_EMOJI,
                with_bracket_emoji: WITH_BRACKET_EMOJI,
                with_newline: WITH_NEWLINE,
                with_odd_control: WITH_ODD_CONTROL,
                with_url: WITH_URL,
            },
            groups: GroupProfile {
                share: GROUP_SHARE,
                big_members: BIG_MEMBERS,
                big_speakers: BIG_SPEAKERS,
                speaker_q: SPEAKER_Q,
                speaker_s: SPEAKER_S,
            },
            media: MediaProfile {
                image_sets: IMAGE_SETS.to_vec(),
                thumb_bytes: THUMB_BYTES,
                image_bytes: IMAGE_BYTES,
                hd_bytes: HD_BYTES,
                image_formats: IMAGE_FORMATS,
                hd_formats: HD_FORMATS,
                video_bytes: VIDEO_BYTES,
                video_thumb_bytes: VIDEO_THUMB_BYTES,
                video_thumb_here: VIDEO_THUMB_HERE,
                video_here: VIDEO_HERE,
                file_bytes: FILE_BYTES,
                file_here: FILE_HERE,
            },
        }
    }
}

impl Profile {
    /// A profile saved as JSON, checked as [`check`](Self::check) does.
    ///
    /// # Errors
    /// When the file cannot be read, is not a profile, or fails the check.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            std::fs::read(path).with_context(|| format!("read the profile {}", path.display()))?;
        let profile: Self = serde_json::from_slice(&bytes).context("parse the profile")?;
        profile.check()?;
        Ok(profile)
    }

    /// What the generator needs of it: every list of shares has a positive total, every share is in `0..=1`, every
    /// range is ordered, and the biggest group has a speaker.
    ///
    /// # Errors
    /// The first part that fails, by name.
    pub fn check(&self) -> Result<()> {
        let share = |name: &str, v: f64| {
            ensure!((0.0..=1.0).contains(&v), "{name}: {v} is not a share");
            Ok(())
        };
        let total = |name: &str, shares: &mut dyn Iterator<Item = f64>| {
            let mut sum = 0.0;
            for v in shares {
                ensure!(v.is_finite() && v >= 0.0, "{name}: {v} is not a share");
                sum += v;
            }
            ensure!(sum > 0.0, "{name}: the shares add up to nothing");
            Ok(())
        };
        let m = &self.messages;
        total("messages.types", &mut m.types.iter().map(|t| t.1))?;
        for (name, v) in [
            ("messages.sent_in_group", m.sent_in_group),
            ("messages.sent_in_private", m.sent_in_private),
            ("messages.at_in_group", m.at_in_group),
            ("messages.with_source", m.with_source),
            ("messages.source_compressed", m.source_compressed),
            ("messages.sort_seq_off", m.sort_seq_off),
            ("messages.sort_seq_missing", m.sort_seq_missing),
            ("text.with_emoji", self.text.with_emoji),
            ("text.with_bracket_emoji", self.text.with_bracket_emoji),
            ("text.with_newline", self.text.with_newline),
            ("text.with_odd_control", self.text.with_odd_control),
            ("text.with_url", self.text.with_url),
            ("groups.share", self.groups.share),
            ("media.video_thumb_here", self.media.video_thumb_here),
            ("media.video_here", self.media.video_here),
            ("media.file_here", self.media.file_here),
        ] {
            share(name, v)?;
        }
        total("time.hours", &mut self.time.hours.iter().copied())?;
        ensure!(
            self.time.hours.iter().all(|&h| h > 0.0),
            "time.hours: an hour with no message"
        );
        total("time.weekdays", &mut self.time.weekdays.iter().copied())?;
        total(
            "time.burst_gaps",
            &mut self.time.burst_gaps.iter().map(|g| g.2),
        )?;
        ensure!(
            self.time.burst_gaps.iter().all(|g| g.0 < g.1),
            "time.burst_gaps: a range is not ordered"
        );
        total("text.chars", &mut self.text.chars.iter().map(|c| c.2))?;
        ensure!(
            self.text.chars.iter().all(|c| 0 < c.0 && c.0 < c.1),
            "text.chars: a range is not ordered"
        );
        total(
            "media.image_sets",
            &mut self.media.image_sets.iter().map(|s| s.3),
        )?;
        ensure!(
            1 <= self.groups.big_speakers && self.groups.big_speakers <= self.groups.big_members,
            "groups: the biggest group needs a speaker among its members"
        );
        Ok(())
    }

    /// A short name of what it holds: the first 16 hex digits of the MD5 of its JSON, each fraction written with 9
    /// significant digits, so that a profile read back from its file (whose last digit may come back off by one)
    /// has the same name.
    ///
    /// # Errors
    /// When it does not serialize, which plain data does not do.
    pub fn hash(&self) -> Result<String> {
        fn canonical(v: &Value, out: &mut String) {
            match v {
                Value::Number(n) => match (n.as_i64(), n.as_f64()) {
                    (Some(i), _) => out.push_str(&i.to_string()),
                    (None, Some(f)) => out.push_str(&format!("{f:.8e}")),
                    _ => out.push_str(&n.to_string()),
                },
                Value::Array(items) => {
                    out.push('[');
                    for item in items {
                        canonical(item, out);
                        out.push(',');
                    }
                    out.push(']');
                }
                Value::Object(fields) => {
                    out.push('{');
                    for (k, item) in fields {
                        out.push_str(k);
                        out.push(':');
                        canonical(item, out);
                        out.push(',');
                    }
                    out.push('}');
                }
                other => out.push_str(&other.to_string()),
            }
        }
        let mut text = String::new();
        canonical(
            &serde_json::to_value(self).context("serialize the profile")?,
            &mut text,
        );
        Ok(hex(&Md5::digest(text))[..16].to_string())
    }

    fn content_bytes(&self, local_type: i64) -> Spread {
        self.messages
            .content_bytes
            .iter()
            .find(|(t, _)| *t == local_type)
            .map_or(self.messages.content_bytes_other, |(_, s)| *s)
    }
}

// The default profile, measured on a real account for #488.
const MEASURED: &str = "2026-11-02";

const fn app(sub: i64) -> i64 {
    (sub << 32) | 49
}

/// Message types by share, as `(local_type, share)`; an app message's local type is `(its type << 32) | 49`.
const TYPES: &[(i64, f64)] = &[
    (1, 0.6162),
    (app(57), 0.1369),
    (47, 0.1111),
    (3, 0.0621),
    (34, 0.0305),
    (10000, 0.0183),
    (app(4), 0.0086),
    (43, 0.0041),
    (app(6), 0.0034),
    (50, 0.0019),
    (app(19), 0.0018),
    (app(62), 0.0010),
    (app(5), 0.0009),
    (app(2001), 0.0009),
    (app(51), 0.0008),
    (app(8), 0.0007),
    (app(2000), 0.0002),
    (app(24), 0.0001),
    (app(76), 0.0001),
    (app(87), 0.0001),
    (app(92), 0.0001),
    (app(1), 0.000_04),
    (app(33), 0.000_04),
    (app(36), 0.000_04),
    (app(63), 0.000_02),
    (app(3), 0.000_02),
    (app(44), 0.000_01),
    (42, 0.000_01),
    (48, 0.000_01),
];

/// Share of messages by hour of the day.
const HOURS: [f64; 24] = [
    0.0400, 0.0139, 0.0080, 0.0029, 0.0031, 0.0039, 0.0074, 0.0093, 0.0223, 0.0425, 0.0604, 0.0706,
    0.0612, 0.0483, 0.0382, 0.0338, 0.0461, 0.0551, 0.0582, 0.0687, 0.0684, 0.0649, 0.1083, 0.0643,
];
/// Share of messages by day of the week, Monday first.
const WEEKDAYS: [f64; 7] = [0.1389, 0.1371, 0.1601, 0.1940, 0.1288, 0.1259, 0.1152];
/// Text lengths in characters, as `(from, to, share)`.
const TEXT_CHARS: &[(u32, u32, f64)] = &[
    (1, 2, 0.0220),
    (2, 5, 0.2090),
    (5, 10, 0.2761),
    (10, 20, 0.3088),
    (20, 50, 0.1571),
    (50, 100, 0.0198),
    (100, 200, 0.0048),
    (200, 500, 0.0018),
    (500, 1000, 0.0005),
    (1000, 4500, 0.0003),
];
const WITH_EMOJI: f64 = 0.0563;
const WITH_BRACKET_EMOJI: f64 = 0.2064;
const WITH_NEWLINE: f64 = 0.0113;
const WITH_ODD_CONTROL: f64 = 0.0008;
const WITH_URL: f64 = 0.0013;
/// Seconds between two messages of one burst, as `(from, to, share)`.
const BURST_GAPS: &[(i64, i64, f64)] = &[
    (0, 5, 0.2695),
    (5, 60, 0.5787),
    (60, 600, 0.1189),
    (600, 3600, 0.0220),
];
/// Text and system messages longer than this are stored compressed, like every other kind.
const ZSTD_ABOVE: usize = 150;
const SENT_IN_GROUP: f64 = 0.07;
const SENT_IN_PRIVATE: f64 = 0.45;
const AT_IN_GROUP: f64 = 0.058;
const WITH_SOURCE: f64 = 0.987;
const SOURCE_COMPRESSED: f64 = 0.984;
const SORT_SEQ_OFF: f64 = 0.0008;
const SORT_SEQ_MISSING: f64 = 0.0001;
/// Who speaks in a group: the member of rank `r` (from 0) says `(r + 1 + Q) ^ -S` of it, which gives the real
/// group's 13% for the first and 91% for the first tenth.
const SPEAKER_Q: f64 = 18.5;
const SPEAKER_S: f64 = 3.8;
/// The biggest group: members, and how many of them ever speak.
const BIG_MEMBERS: usize = 369;
const BIG_SPEAKERS: usize = 251;
const GROUP_SHARE: f64 = 0.08;
/// Which files of a picture are on the computer: `(thumbnail, image, original, share)`.
const IMAGE_SETS: &[(bool, bool, bool, f64)] = &[
    (true, false, false, 0.532),
    (true, true, false, 0.437),
    (true, true, true, 0.030),
    (false, true, false, 0.001),
];
/// File sizes in bytes as `(median, 90th percentile, at most)`.
const THUMB_BYTES: (f64, f64, f64) = (5053.0, 11_639.0, 98_000.0);
const IMAGE_BYTES: (f64, f64, f64) = (46_540.0, 118_757.0, 2_200_000.0);
const HD_BYTES: (f64, f64, f64) = (120_040.0, 722_310.0, 8_200_000.0);
const VIDEO_BYTES: (f64, f64, f64) = (2_872_790.0, 14_861_110.0, 8.0 * MIB as f64);
const VIDEO_THUMB_BYTES: (f64, f64, f64) = (10_315.0, 25_880.0, 360_000.0);
const FILE_BYTES: (f64, f64, f64) = (283_610.0, 6_863_770.0, 8.0 * MIB as f64);
/// What the image file holds, as `(png, jpeg, wxgf)` shares.
const IMAGE_FORMATS: (f64, f64, f64) = (0.265, 0.015, 0.720);
const HD_FORMATS: (f64, f64, f64) = (0.567, 0.233, 0.200);
const VIDEO_THUMB_HERE: f64 = 0.9;
const VIDEO_HERE: f64 = 0.1;
const FILE_HERE: f64 = 0.35;
/// The share of the messages in the first message database; the others share the rest.
const FIRST_SHARD: f64 = 0.15;
/// Messages this close to where one database ends and the next starts go to either.
const SHARD_OVERLAP: i64 = 20 * DAY;
/// The newest messages of each database that are still in its WAL.
const WAL_MESSAGES: usize = 1500;
/// WeChat's search index holds the text messages up to the WAL ones, in this many tables.
const FTS_TABLES: usize = 5;
const SNS_PER_FRIEND: f64 = 1.1;

/// `(median, 90th percentile, largest)` of the content size in bytes by type (the size of the XML of all but text).
const CONTENT_BYTES: &[(i64, Spread)] = &[
    (3, (1334.0, 1562.0, 17_520.0)),
    (34, (554.0, 556.0, 769.0)),
    (43, (1221.0, 1413.0, 1770.0)),
    (47, (1734.0, 1981.0, 2380.0)),
    (50, (525.0, 554.0, 557.0)),
    (app(4), (10_670.0, 10_781.0, 10_999.0)),
    (app(5), (1423.0, 10_611.0, 13_142.0)),
    (app(6), (1524.0, 10_097.0, 10_658.0)),
    (app(8), (1973.0, 11_858.0, 12_019.0)),
    (app(19), (8176.0, 34_069.0, 148_365.0)),
    (app(24), (3343.0, 6499.0, 39_851.0)),
    (app(51), (10_787.0, 14_132.0, 20_352.0)),
    (app(57), (1579.0, 10_560.0, 47_816.0)),
    (app(62), (1238.0, 1262.0, 1291.0)),
    (app(87), (4943.0, 13_484.0, 39_462.0)),
    (app(92), (4716.0, 13_689.0, 14_000.0)),
    (app(2000), (3194.0, 3253.0, 3287.0)),
    (app(2001), (2298.0, 5422.0, 6906.0)),
];
const CONTENT_BYTES_OTHER: Spread = (2000.0, 6000.0, 20_000.0);

// ---- random numbers ----

/// SplitMix64: small, fast, and fixed for good, where a library's generator may change its stream in a new version.
#[derive(Clone)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed)
    }

    /// A stream of its own for one part of the account, so changing one part leaves the others as they were.
    fn fork(&self, salt: u64) -> Self {
        let mut r = Self(self.0 ^ salt.wrapping_mul(0xA24B_AED4_963E_E407));
        r.next_u64();
        r
    }

    pub fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// Uniform in `[0, 1)`.
    fn unit(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// Uniform in `0..n` (0 when `n` is 0).
    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }

    /// Uniform in `lo..hi`.
    fn between(&mut self, lo: i64, hi: i64) -> i64 {
        lo + (self.next_u64() % (hi - lo).max(1) as u64) as i64
    }

    fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len())]
    }

    /// A standard normal draw (Box–Muller).
    fn normal(&mut self) -> f64 {
        let u = 1.0 - self.unit();
        let v = self.unit();
        (-2.0 * u.ln()).sqrt() * (std::f64::consts::TAU * v).cos()
    }

    /// A log-normal draw with this median and 90th percentile, at most `max`.
    fn spread(&mut self, (median, p90, max): (f64, f64, f64)) -> f64 {
        let sigma = (p90 / median).ln() / 1.2816;
        (median * (sigma * self.normal()).exp()).min(max)
    }

    fn hex(&mut self, n: usize) -> String {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = String::with_capacity(n);
        while out.len() < n {
            let mut bits = self.next_u64();
            for _ in 0..16.min(n - out.len()) {
                out.push(DIGITS[(bits & 15) as usize] as char);
                bits >>= 4;
            }
        }
        out
    }

    /// `n` hex digits as XML carries them in URLs and keys: random runs, some of them repeated (the same key or
    /// parameter twice), which compresses about as well as WeChat's does.
    fn filler(&mut self, n: usize) -> String {
        let mut out = String::with_capacity(n);
        while out.len() < n {
            let run = (16 + self.below(48)).min(n - out.len());
            if out.len() >= 64 && self.chance(0.45) {
                let from = self.below(out.len() - run.min(out.len()));
                let copy = out[from..from + run.min(out.len() - from)].to_string();
                out.push_str(&copy);
            } else {
                let hex = self.hex(run);
                out.push_str(&hex);
            }
        }
        out.truncate(n);
        out
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n];
        for chunk in out.chunks_mut(8) {
            let b = self.next_u64().to_le_bytes();
            chunk.copy_from_slice(&b[..chunk.len()]);
        }
        out
    }
}

/// Draws an index with the weights it was made with.
struct Weighted(Vec<f64>);

impl Weighted {
    fn new(weights: impl IntoIterator<Item = f64>) -> Self {
        let mut total = 0.0;
        Self(
            weights
                .into_iter()
                .map(|w| {
                    total += w;
                    total
                })
                .collect(),
        )
    }

    fn draw(&self, rng: &mut Rng) -> usize {
        let x = rng.unit() * self.0.last().copied().unwrap_or(0.0);
        self.0.partition_point(|&c| c <= x).min(self.0.len() - 1)
    }
}

/// Draws a value from `(from, to, share)` ranges: a range by its share, then in it log-uniformly (short values are
/// more common than long ones inside a range too).
fn draw_range<T: Copy + Into<f64>>(rng: &mut Rng, ranges: &[(T, T, f64)]) -> f64 {
    let (from, to, _) = ranges[Weighted::new(ranges.iter().map(|r| r.2)).draw(rng)];
    let (from, to): (f64, f64) = (from.into().max(1.0), to.into());
    (from.ln() + rng.unit() * (to.ln() - from.ln())).exp()
}

// ---- words ----

const HAN: &str = "的一是了我不人在他有这个上们来到时大地为子中你说生年着就那和要她出也得里后自以会家可下而过天去能对小多然于心学么之都好看起发当没成只如事把还用第样道想作种开美总从无情己面最女但现前些所同日手又行意动方期它头经长儿回位分爱老因很给名法间知世什两次使身者被高已亲其进此话常与活正感见明问力理点文几定本公特做外孩相西果走将月十实向声车全信重三机工物气每并别真打太新比才便夫再书部水像眼等体却加电主界门利海受听表少代员许先口由死安写性马光白或住难望教命花结乐色更拉东神记处让母父应直字场平报友关放至张认接告入笑内英军候民岁往何度山觉路带万男边风解叫任金快原吃妈变通师立象数四失满战远格士音轻目条呢病始达深完今提求清王化空业思切怎非找片罗钱吗语元喜曾离飞科言干流欢约各即指合反题必该论交终林请医晚制球决传画保读运及则房早院量苦火布品近坐产答星精视五连司巴奇管类未朋且婚台夜青北队久乎越观落尽形影红爸百令周吧识步希亚术留市半热送兴造谈容极随演收首根讲整式取照办强石古华拿计您装似足双妻转诉米称丽客南领节衣站黑刻统断福城故历惊脸选包紧争另建维绝树系伤示愿持千史谁准联妇纪基买志静阿诗独复痛消社算义竟确酒需单治卡幸兰念举仅钟怕共毛句息功官待究跟穿室易游程号居考突皮哪费倒价图具刚脑永歌响商礼细专黄块脚味灵改据般破引食仍存众注笔甚某沉血备习校默务土微娘须试怀料调广苏显赛查密议底列富梦错座参八除跑亮假印设线温虽掉京初养香停际致阳纸李纳验助激够严证帝饭忘趣支春集丈木研班普导顿睡展跳获艺六波察群";
const PHRASES: &[&str] = &[
    "好的",
    "收到",
    "哈哈哈",
    "哈哈哈哈哈",
    "没问题",
    "谢谢",
    "辛苦了",
    "在吗",
    "可以",
    "不用了",
    "知道了",
    "明天",
    "今天",
    "晚上",
    "一起吃饭",
    "开会",
    "文件发我一下",
    "等一下",
    "马上到",
    "好滴",
    "嗯嗯",
    "对的",
    "是的",
    "不是",
    "我看看",
    "稍等",
    "OK",
    "ok",
    "666",
    "牛",
    "太强了",
    "真的假的",
    "笑死",
    "绝了",
    "怎么说",
    "下班了",
    "到家了吗",
    "早上好",
    "晚安",
    "周末有空吗",
    "几点",
    "在路上",
    "我也是",
    "你呢",
    "然后呢",
    "确实",
];
const ASCII_WORDS: &[&str] = &[
    "hello", "PPT", "PDF", "bug", "App", "Excel", "deadline", "3点", "2025", "OK", "QAQ", "hhh",
];
const PUNCTUATION: &[&str] = &["，", "。", "！", "？", "~", "...", " ", "，", "。"];
const EMOJI: &[&str] = &[
    "😂", "👍", "🙏", "😭", "🤣", "❤️", "😅", "🎉", "🌹", "😊", "🐶", "🔥",
];
const BRACKET_EMOJI: &[&str] = &[
    "[微笑]",
    "[捂脸]",
    "[旺柴]",
    "[呲牙]",
    "[强]",
    "[抱拳]",
    "[破涕为笑]",
    "[偷笑]",
    "[OK]",
    "[玫瑰]",
    "[让我看看]",
];
const ODD_CONTROLS: &[char] = &['\u{200b}', '\u{7}', '\r', '\u{feff}', '\u{2028}'];
const SURNAMES: &str =
    "王李张刘陈杨黄赵吴周徐孙马朱胡郭何高林罗郑梁谢宋唐许韩冯邓曹彭曾萧田董潘袁蔡蒋余";
const GIVEN: &str = "伟芳娜敏静丽强磊军洋勇艳杰娟涛明超秀霞平刚桂英华玉萍红鹏辉建文斌宇浩凯晨欣怡子涵雨轩梓萱一诺嘉";
const LATIN_NICKS: &[&str] = &[
    "Alex", "Momo", "Leo", "Coco", "Kevin", "Lily", "Jason", "Una", "Tom", "Echo", "Mia", "Sam",
];
const TOPICS: &[&str] = &[
    "家庭",
    "同学",
    "项目",
    "羽毛球",
    "读书",
    "周末",
    "租房",
    "球友",
    "摄影",
    "户外",
    "技术交流",
    "业主",
    "校友",
    "旅行",
    "打卡",
    "游戏",
    "美食",
    "跑步",
    "工作",
    "装修",
];
const GROUP_SUFFIXES: &[&str] = &["群", "交流群", "2群", "互助群", "小分队", ""];
const FILE_EXTENSIONS: &[&str] = &["pdf", "docx", "xlsx", "zip", "pptx", "txt", "mp3", "rar"];

fn chars_of(pool: &str) -> Vec<char> {
    pool.chars().collect()
}

struct Words {
    han: Vec<char>,
    surnames: Vec<char>,
    given: Vec<char>,
    profile: TextProfile,
}

impl Words {
    fn new(profile: TextProfile) -> Self {
        Self {
            han: chars_of(HAN),
            surnames: chars_of(SURNAMES),
            given: chars_of(GIVEN),
            profile,
        }
    }

    /// Text of about `chars` characters, in the mix of words, punctuation and emoji of real messages.
    fn text(&self, rng: &mut Rng, chars: usize) -> String {
        // emoji, a second line and links count in the length, so the lengths stay the real ones
        let mut extra = String::new();
        if rng.chance(self.profile.with_bracket_emoji) {
            extra.push_str(rng.pick(BRACKET_EMOJI));
        }
        if rng.chance(self.profile.with_emoji) {
            for _ in 0..1 + rng.below(3) {
                extra.push_str(rng.pick(EMOJI));
            }
        }
        if rng.chance(self.profile.with_newline) {
            extra.push('\n');
            extra.push_str(&self.text_between(rng, 3, 23));
        }
        if rng.chance(self.profile.with_url) {
            extra.push_str(&format!(" https://example.com/s/{}", rng.hex(10)));
        }
        let chars = chars.saturating_sub(extra.chars().count());
        let mut out = String::new();
        let mut n = 0;
        while n < chars {
            let before = out.len();
            match rng.below(100) {
                0..=54 => out.push_str(rng.pick(PHRASES)),
                55..=89 => {
                    for _ in 0..1 + rng.below(3) {
                        out.push(*rng.pick(&self.han));
                    }
                }
                90..=94 => out.push_str(rng.pick(ASCII_WORDS)),
                _ => out.push_str(rng.pick(PUNCTUATION)),
            }
            if rng.chance(0.12) {
                out.push_str(rng.pick(PUNCTUATION));
            }
            n += out[before..].chars().count();
        }
        let mut out: String = out.chars().take(chars).collect();
        out.push_str(&extra);
        if out.is_empty() {
            out.push_str(rng.pick(PHRASES));
        }
        if rng.chance(self.profile.with_odd_control) {
            let at = out
                .char_indices()
                .nth(rng.below(out.chars().count()))
                .map_or(out.len(), |(i, _)| i);
            out.insert(at, *rng.pick(ODD_CONTROLS));
        }
        out
    }

    /// Like [`text`](Self::text) without emoji, links or control characters.
    fn text_plain(&self, rng: &mut Rng, chars: usize) -> String {
        (0..chars)
            .map(|_| {
                if rng.chance(0.08) {
                    '，'
                } else {
                    *rng.pick(&self.han)
                }
            })
            .collect()
    }

    /// [`text_plain`](Self::text_plain) of `lo..hi` characters.
    fn text_between(&self, rng: &mut Rng, lo: usize, hi: usize) -> String {
        let n = lo + rng.below(hi - lo);
        self.text_plain(rng, n)
    }

    /// A message's text: its length drawn from the real lengths.
    fn message(&self, rng: &mut Rng) -> String {
        let chars = draw_range(rng, &self.profile.chars) as usize;
        self.text(rng, chars.max(1))
    }

    fn person_name(&self, rng: &mut Rng) -> String {
        let mut name = match rng.below(100) {
            0..=69 => {
                let mut s = String::from(*rng.pick(&self.surnames));
                for _ in 0..1 + rng.below(2) {
                    s.push(*rng.pick(&self.given));
                }
                s
            }
            70..=84 => rng.pick(LATIN_NICKS).to_string(),
            85..=94 => self.text_between(rng, 2, 6).replace('，', ""),
            _ => format!("~{}~", rng.pick(PHRASES)),
        };
        if rng.chance(0.1) {
            name.push_str(rng.pick(EMOJI));
        }
        name
    }

    fn group_name(&self, rng: &mut Rng) -> String {
        let mut name = format!("{}{}", rng.pick(TOPICS), rng.pick(GROUP_SUFFIXES));
        if rng.chance(0.15) {
            name.push_str(rng.pick(EMOJI));
        }
        name
    }
}

/// Escapes text for XML.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// The XML `template` makes with a filler of hex digits sized so the whole is about `target` bytes.
fn sized(rng: &mut Rng, target: f64, template: impl Fn(&str) -> String) -> String {
    let base = template("").len();
    let fill = rng.filler((target as usize).saturating_sub(base));
    template(&fill)
}

// ---- people and conversations ----

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    Private,
    Group,
    Official,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Group => "group",
            Self::Official => "official",
        }
    }
}

struct Person {
    username: String,
    nick: String,
    remark: String,
    alias: String,
    avatar: String,
    local_type: i64,
}

struct Conversation {
    username: String,
    name: String,
    kind: Kind,
    /// People who may write in it (not the owner): the partner of a private chat, a group's members by how much
    /// they say (the first `speakers` say something).
    members: Vec<usize>,
    speakers: usize,
    /// Message times, oldest first.
    times: Vec<i64>,
}

struct World {
    people: Vec<Person>,
    conversations: Vec<Conversation>,
    words: Words,
}

fn unique(rng: &mut Rng, taken: &mut HashSet<String>, make: impl Fn(&mut Rng) -> String) -> String {
    loop {
        let name = make(rng);
        if taken.insert(name.clone()) {
            return name;
        }
    }
}

fn user_name(rng: &mut Rng) -> String {
    const ALNUM: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut name: String = if rng.chance(0.15) {
        // an old custom id: a letter, then letters, digits, `_` and `-`
        let mut s = String::from((b'a' + rng.below(26) as u8) as char);
        for _ in 0..5 + rng.below(14) {
            s.push(match rng.below(20) {
                0 => '_',
                1 => '-',
                _ => ALNUM[rng.below(ALNUM.len())] as char,
            });
        }
        return s;
    } else {
        "wxid_".into()
    };
    for _ in 0..14 {
        name.push(ALNUM[rng.below(ALNUM.len())] as char);
    }
    name
}

/// When the messages of a conversation of `n` messages are, between `first_day` and `last_day` (local days):
/// bursts of messages a few seconds apart, on days and hours drawn like the real ones.
fn timeline(
    rng: &mut Rng,
    n: usize,
    (first_day, last_day): (i64, i64),
    burst: f64,
    profile: &TimeProfile,
) -> Vec<i64> {
    let days: Vec<i64> = (first_day..=last_day).collect();
    // a few busy days carry much of a conversation, as on a real account
    let day_weights = Weighted::new(days.iter().map(|d| {
        let pareto = (1.0 - rng.unit()).powf(-1.0 / 1.3).min(60.0);
        profile.weekdays[(d + 3).rem_euclid(7) as usize] * pareto
    }));
    let hours = Weighted::new(profile.hours);
    let gaps = Weighted::new(profile.burst_gaps.iter().map(|g| g.2));
    let mut times = Vec::with_capacity(n);
    while times.len() < n {
        // geometric burst length with this mean
        let len = (1.0 + (1.0 - rng.unit()).ln() / (1.0 - 1.0 / burst).ln()) as usize;
        let day = days[day_weights.draw(rng)];
        let mut hour = hours.draw(rng);
        let mut t = day * DAY - TZ + hour as i64 * 3600 + rng.between(0, 3600);
        for _ in 0..len.clamp(1, n - times.len()) {
            if t > END {
                break;
            }
            times.push(t);
            let (from, to, _) = profile.burst_gaps[gaps.draw(rng)];
            t += rng.between(from, to);
            // into a quieter hour, a conversation goes on as much less often as that hour is quieter
            let now = ((t + TZ).rem_euclid(DAY) / 3600) as usize;
            if now != hour {
                if !rng.chance(profile.hours[now] / profile.hours[hour]) {
                    break;
                }
                hour = now;
            }
        }
    }
    times.sort_unstable();
    times
}

impl World {
    fn new(rng: &mut Rng, spec: &BenchSpec) -> Self {
        let profile = &spec.profile;
        let words = Words::new(profile.text.clone());
        let mut taken = HashSet::from([OWNER.to_string()]);
        let mut people = vec![Person {
            username: OWNER.into(),
            nick: "基准测试".into(),
            remark: String::new(),
            alias: String::new(),
            avatar: String::new(),
            local_type: 1,
        }];
        let mut person = |rng: &mut Rng, local_type: i64| -> usize {
            let username = unique(rng, &mut taken, user_name);
            let nick = words.person_name(rng);
            let remark = if rng.chance(0.05) {
                words.person_name(rng)
            } else {
                String::new()
            };
            let alias = if rng.chance(0.2) {
                format!(
                    "{}{}",
                    rng.pick(LATIN_NICKS).to_lowercase(),
                    rng.below(10_000)
                )
            } else {
                String::new()
            };
            let avatar = format!("https://wx.qlogo.invalid/mmhead/{}/132", rng.hex(48));
            people.push(Person {
                username,
                nick,
                remark,
                alias,
                avatar,
                local_type,
            });
            people.len() - 1
        };

        let groups = (spec.conversations as f64 * profile.groups.share).round() as usize;
        let privates = spec.conversations - groups;
        // friends are about four times the private chats on the real account; the private chats are with some of them
        let friends: Vec<usize> = (0..(privates * 4).max(profile.groups.big_members / 3))
            .map(|_| person(rng, 1))
            .collect();
        // a few contacts of the other kinds a real account has
        for _ in 0..(friends.len() / 40).max(1) {
            person(rng, 0);
        }
        person(rng, 4);

        let mut conversations = Vec::new();
        let mut room_names = HashSet::new();
        let mut group = |rng: &mut Rng, members: usize, speakers: usize, messages, days, burst| {
            let mut pool: Vec<usize> = Vec::new();
            while pool.len() < members {
                // a group mixes friends and people the owner only knows from it
                let who = if rng.chance(0.3) {
                    friends[rng.below(friends.len())]
                } else {
                    person(rng, 3)
                };
                if !pool.contains(&who) {
                    pool.push(who);
                }
            }
            Conversation {
                username: unique(rng, &mut room_names, |r| {
                    format!("{}@chatroom", r.between(10_000_000_000, 60_000_000_000))
                }),
                name: words.group_name(rng),
                kind: Kind::Group,
                members: pool,
                speakers,
                times: timeline(rng, messages, days, burst, &profile.time),
            }
        };
        conversations.push(group(
            rng,
            profile.groups.big_members,
            profile.groups.big_speakers,
            spec.group_messages,
            (END_DAY - 400, END_DAY),
            120.0,
        ));
        let window = |rng: &mut Rng| {
            let last = END_DAY - (rng.unit().powi(3) * 500.0) as i64;
            let len = rng.spread((90.0, 400.0, 600.0)).max(1.0) as i64;
            (last - len, last)
        };
        for _ in 0..groups {
            let members = rng.spread((37.0, 89.0, 500.0)).max(3.0) as usize;
            let messages = rng.spread((100.0, 700.0, 20_000.0)).max(5.0) as usize;
            let days = window(rng);
            conversations.push(group(rng, members, members, messages, days, 25.0));
        }
        let mut partners: Vec<usize> = friends.clone();
        for _ in 0..privates {
            let partner = partners.swap_remove(rng.below(partners.len()));
            let messages = rng.spread((64.0, 1040.0, 10_000.0)).max(1.0) as usize;
            let days = window(rng);
            conversations.push(Conversation {
                username: people[partner].username.clone(),
                name: people[partner].nick.clone(),
                kind: Kind::Private,
                members: vec![partner],
                speakers: 1,
                times: timeline(rng, messages, days, 10.0, &profile.time),
            });
        }
        // official accounts: in the conversation list, their messages in another database
        for _ in 0..(spec.conversations / 20).max(3) {
            let username = unique(rng, &mut taken, |r| format!("gh_{}", r.hex(12)));
            let name = format!(
                "{}{}",
                rng.pick(TOPICS),
                rng.pick(&["服务号", "订阅号", "助手"])
            );
            people.push(Person {
                username: username.clone(),
                nick: name.clone(),
                remark: String::new(),
                alias: String::new(),
                avatar: String::new(),
                local_type: 5,
            });
            let at = END - rng.between(0, 300 * DAY);
            conversations.push(Conversation {
                username,
                name,
                kind: Kind::Official,
                members: vec![people.len() - 1],
                speakers: 0,
                times: vec![at],
            });
        }
        Self {
            people,
            conversations,
            words,
        }
    }
}

/// The message database of each message of each conversation: by time, the first [`FIRST_SHARD`] of the account's
/// messages in the first, the rest shared by the others, and near each border either.
fn assign_shards(rng: &mut Rng, conversations: &[Conversation], shards: usize) -> Vec<Vec<usize>> {
    let mut all: Vec<i64> = conversations
        .iter()
        .filter(|c| c.kind != Kind::Official)
        .flat_map(|c| c.times.iter().copied())
        .collect();
    all.sort_unstable();
    let borders: Vec<i64> = (1..shards)
        .map(|k| {
            let share = FIRST_SHARD + (1.0 - FIRST_SHARD) * (k - 1) as f64 / (shards - 1) as f64;
            all[((all.len() as f64 * share) as usize).min(all.len() - 1)]
        })
        .collect();
    conversations
        .iter()
        .map(|c| {
            c.times
                .iter()
                .map(|&t| {
                    let mut shard = borders.iter().filter(|&&b| t >= b).count();
                    if let Some(k) = borders.iter().position(|&b| (t - b).abs() < SHARD_OVERLAP) {
                        shard = k + usize::from(rng.chance(0.5));
                    }
                    shard
                })
                .collect()
        })
        .collect()
}

// ---- files ----

/// A V2 `.dat` file: the head AES-encrypted with [`IMAGE_AES_KEY`], the tail XOR-ed with [`IMAGE_XOR_KEY`].
fn encrypt_dat(plain: &[u8]) -> Vec<u8> {
    let aes_len = plain.len().min(1024);
    let xor_len = (plain.len() - aes_len).min(100);
    let mut head = plain[..aes_len].to_vec();
    let pad = 16 - head.len() % 16;
    head.extend(std::iter::repeat_n(pad as u8, pad));
    let mut key = [0u8; 16];
    key.copy_from_slice(&IMAGE_AES_KEY.as_bytes()[..16]);
    let cipher = aes::Aes128::new(&key.into());
    let mut out = vec![0x07, 0x08, 0x56, 0x32, 0x08, 0x07];
    out.extend((aes_len as i32).to_le_bytes());
    out.extend((xor_len as i32).to_le_bytes());
    out.push(1);
    for block in head.as_chunks_mut::<16>().0 {
        let mut b = aes::cipher::Array::from(*block);
        cipher.encrypt_block(&mut b);
        out.extend_from_slice(&b);
    }
    out.extend_from_slice(&plain[aes_len..plain.len() - xor_len]);
    out.extend(
        plain[plain.len() - xor_len..]
            .iter()
            .map(|b| b ^ IMAGE_XOR_KEY),
    );
    out
}

/// A JPEG of `len` bytes: a JFIF header, comment segments of noise and the end marker (not a picture).
fn jpeg(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = vec![
        0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10, b'J', b'F', b'I', b'F', 0, 1, 1, 0, 0, 1, 0, 1, 0, 0,
    ];
    while out.len() + 4 + 2 < len {
        let n = (len - out.len() - 6).clamp(1, 65_533);
        out.extend([0xFF, 0xFE]);
        out.extend(((n + 2) as u16).to_be_bytes());
        out.extend(rng.bytes(n));
    }
    out.extend([0xFF, 0xD9]);
    out
}

fn crc32(chunks: &[&[u8]]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, entry) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 == 1 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            *entry = c;
        }
        t
    });
    let mut crc = !0u32;
    for b in chunks.iter().flat_map(|c| c.iter()) {
        crc = table[((crc ^ u32::from(*b)) & 0xFF) as usize] ^ (crc >> 8);
    }
    !crc
}

fn png_chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend((data.len() as u32).to_be_bytes());
    out.extend(kind);
    out.extend(data);
    out.extend(crc32(&[kind, data]).to_be_bytes());
}

/// A valid PNG of one grey pixel padded to `len` bytes with a private chunk of noise.
fn png(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    png_chunk(&mut out, b"IHDR", &[0, 0, 0, 1, 0, 0, 0, 1, 8, 0, 0, 0, 0]);
    // zlib, one stored block of the scanline (filter 0, grey 0x80), and its Adler-32
    png_chunk(
        &mut out,
        b"IDAT",
        &[
            0x78, 0x01, 0x01, 0x02, 0x00, 0xFD, 0xFF, 0x00, 0x80, 0x00, 0x83, 0x00, 0x81,
        ],
    );
    let pad = len.saturating_sub(out.len() + 12 + 12);
    if pad > 0 {
        png_chunk(&mut out, b"wfPd", &rng.bytes(pad));
    }
    png_chunk(&mut out, b"IEND", &[]);
    out
}

/// A WXGF picture the built-in decoder reads (a small HEVC frame), padded to `len` bytes with filler NAL units.
fn wxgf(len: usize) -> Vec<u8> {
    const HEVC: &[u8] = include_bytes!("../../../weflow-core/testdata/wxgf/full_range.hevc");
    let mut out = b"wxgf".to_vec();
    out.extend([0x24, 0x01, 0x00, 0x46, 0x00, 0x2e]);
    out.extend(HEVC);
    while out.len() + 6 < len {
        // filler data (NAL type 38): 0xFF bytes up to the stop bit
        let n = (len - out.len() - 6).min(60_000);
        out.extend([0x00, 0x00, 0x01, 0x4C, 0x01]);
        out.extend(std::iter::repeat_n(0xFF, n));
        out.push(0x80);
    }
    out
}

/// An MP4 of `len` bytes: a file type box and a media data box of noise.
fn mp4(rng: &mut Rng, len: usize) -> Vec<u8> {
    let mut out = vec![
        0, 0, 0, 0x18, b'f', b't', b'y', b'p', b'i', b's', b'o', b'm', 0, 0, 2, 0, b'i', b's',
        b'o', b'm', b'i', b's', b'o', b'2',
    ];
    let data = len.saturating_sub(out.len() + 8);
    out.extend(((data + 8) as u32).to_be_bytes());
    out.extend(b"mdat");
    out.extend(rng.bytes(data));
    out
}

/// The packets of a second of a 440 Hz tone the SILK SDK encoded at 24 kHz (20 ms each).
static TONE_PACKETS: std::sync::LazyLock<Vec<Vec<u8>>> = std::sync::LazyLock::new(|| {
    let stream = weflow_silk::testenc::encode_tone(24_000);
    let mut packets = Vec::new();
    let mut rest = &stream[b"#!SILK_V3".len()..];
    while let [lo, hi, tail @ ..] = rest {
        let Ok(len) = usize::try_from(i16::from_le_bytes([*lo, *hi])) else {
            break; // the -1 that ends the stream
        };
        packets.push(tail[..len].to_vec());
        rest = &tail[len..];
    }
    packets
});

/// A SILK voice of `ms` milliseconds the decoder reads (#574): the header WeChat writes, then a packet of the tone
/// per 20 ms, over again past its second. The random numbers it once drew are still drawn, so the rest of the
/// account stays the same.
fn silk(rng: &mut Rng, ms: i64) -> Vec<u8> {
    let mut out = b"\x02#!SILK_V3".to_vec();
    for packet in TONE_PACKETS.iter().cycle().take((ms / 20).max(1) as usize) {
        let n = 20 + rng.below(26);
        let _ = rng.bytes(n);
        out.extend((packet.len() as u16).to_le_bytes());
        out.extend(packet);
    }
    out
}

/// The month folder of a file sent at `t` (`YYYY-MM` in the account's clock).
fn month(t: i64) -> String {
    // days since 1970-01-01 to a civil date (Howard Hinnant's algorithm)
    let z = (t + TZ).div_euclid(DAY) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y}-{m:02}")
}

// ---- messages ----

/// A file written for the account, and its `hardlink.db` row.
struct Hardlink {
    md5: String,
    file_name: String,
    kind: i64,
    dir1: String,
    dir2: String,
    size: i64,
    modify_time: i64,
}

#[derive(Default)]
struct Written {
    hardlinks: Vec<Hardlink>,
    /// (conversation, create time, local id, server id, data)
    voices: Vec<(usize, i64, i64, i64, Vec<u8>)>,
    files: BTreeMap<&'static str, (u64, u64)>,
}

impl Written {
    fn file(&mut self, what: &'static str, path: &Path, data: &[u8]) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, data).with_context(|| format!("write {}", path.display()))?;
        let entry = self.files.entry(what).or_default();
        entry.0 += 1;
        entry.1 += data.len() as u64;
        Ok(())
    }
}

/// An earlier message a reply can quote.
struct Recent {
    server_id: i64,
    sender: usize,
    local_type: i64,
    time: i64,
    content: String,
}

/// One message as it is stored.
struct Row {
    local_id: i64,
    server_id: i64,
    local_type: i64,
    sort_seq: Option<i64>,
    sender: Option<usize>,
    time: i64,
    status: i64,
    source: Option<Vec<u8>>,
    source_compressed: bool,
    content: Vec<u8>,
    compressed: bool,
}

struct Writer<'a> {
    world: &'a World,
    profile: &'a Profile,
    account_dir: &'a Path,
    media: bool,
    written: Written,
}

/// The parts of a message the generator makes.
struct Made {
    content: String,
    /// What WeChat's search index holds of it.
    indexed: Option<String>,
    summary: String,
    /// The member a group text mentions.
    mention: Option<usize>,
}

impl World {
    fn nick(&self, who: usize) -> &str {
        &self.people[who].nick
    }

    fn user(&self, who: usize) -> &str {
        &self.people[who].username
    }
}

impl<'a> Writer<'a> {
    #[expect(
        clippy::too_many_arguments,
        reason = "one message's identity: conversation, sender, time and ids"
    )]
    fn make(
        &mut self,
        rng: &mut Rng,
        conv: usize,
        local_type: i64,
        sender: usize,
        time: i64,
        ids: (i64, i64),
        recent: &VecDeque<Recent>,
    ) -> Result<Made> {
        let world: &'a World = self.world;
        let c = &world.conversations[conv];
        let words = &world.words;
        let target = rng.spread(self.profile.content_bytes(local_type));
        let from = world.user(sender);
        let to = &c.username;
        let made = |content: String, summary: &str| Made {
            content,
            indexed: None,
            summary: summary.to_string(),
            mention: None,
        };
        Ok(match local_type {
            1 => {
                let mut text = words.message(rng);
                let mut mention = None;
                if c.kind == Kind::Group && rng.chance(self.profile.messages.at_in_group) {
                    let who = c.members[rng.below(c.members.len())];
                    text = format!("@{}\u{2005}{text}", world.nick(who));
                    mention = Some(who);
                }
                Made {
                    indexed: Some(text.clone()),
                    summary: text.clone(),
                    content: text,
                    mention,
                }
            }
            3 => made(self.image(rng, &c.username, time, target)?, "[图片]"),
            34 => {
                let ms = rng.spread((4000.0, 12_000.0, 60_000.0)).max(1000.0) as i64;
                let data = silk(rng, ms);
                let xml = sized(rng, target, |fill| {
                    format!(
                        "<msg><voicemsg endflag=\"1\" cancelflag=\"0\" forwardflag=\"0\" voiceformat=\"4\" voicelength=\"{ms}\" \
                         length=\"{}\" bufid=\"0\" aeskey=\"{}\" voiceurl=\"{fill}\" voicemd5=\"\" clientmsgid=\"{}\" \
                         fromusername=\"{from}\" /></msg>",
                        data.len(),
                        "0".repeat(32),
                        ids.1
                    )
                });
                self.written.voices.push((conv, time, ids.0, ids.1, data));
                made(xml, "[语音]")
            }
            43 => made(self.video(rng, from, time, target)?, "[视频]"),
            47 => {
                let md5 = rng.hex(32);
                let len = rng.between(20_000, 900_000);
                let xml = sized(rng, target, |fill| {
                    format!(
                        "<msg><emoji fromusername=\"{from}\" tousername=\"{to}\" type=\"2\" idbuffer=\"media:0_0\" md5=\"{md5}\" \
                         len=\"{len}\" productid=\"\" androidmd5=\"{md5}\" androidlen=\"{len}\" s60v3md5=\"{md5}\" s60v3len=\"{len}\" \
                         s60v5md5=\"{md5}\" s60v5len=\"{len}\" cdnurl=\"http://emoji.invalid/{fill}\" designerid=\"\" thumburl=\"\" \
                         encrypturl=\"http://emoji.invalid/e/{md5}\" aeskey=\"{md5}\" externurl=\"http://emoji.invalid/x/{md5}\" \
                         externmd5=\"{md5}\" width=\"240\" height=\"240\" tpurl=\"\" tpauthkey=\"\" attachedtext=\"\" \
                         attachedtextcolor=\"\" lensid=\"\" emojiattr=\"\" linkid=\"\" desc=\"\"></emoji>\
                         <gameext type=\"0\" content=\"0\"></gameext></msg>"
                    )
                });
                made(xml, "[动画表情]")
            }
            50 => {
                let secs = rng.between(3, 3600);
                let xml = sized(rng, target, |fill| {
                    format!(
                        "<voipmsg type=\"VoIPBubbleMsg\"><VoIPBubbleMsg><msg><![CDATA[通话时长 {:02}:{:02}]]></msg>\n\
                         <room_type>{}</room_type>\n<red_dot>false</red_dot>\n<roomid>{}</roomid>\n<roomkey>{fill}</roomkey>\n\
                         <inviteid>{}</inviteid>\n<msg_type>100</msg_type>\n<timestamp>{}</timestamp>\n<duration>0</duration>\n\
                         </VoIPBubbleMsg></voipmsg>",
                        secs / 60,
                        secs % 60,
                        ids.0 % 2,
                        ids.1 % 1_000_000,
                        ids.1 % 999_983,
                        time * 1000
                    )
                });
                made(xml, "[通话]")
            }
            10000 => {
                let other = c.members[rng.below(c.members.len())];
                let text = match rng.below(10) {
                    0..=3 => format!("\"{}\" 撤回了一条消息", world.nick(other)),
                    4 => "你撤回了一条消息".to_string(),
                    5 | 6 if c.kind == Kind::Group => {
                        let invited = c.members[rng.below(c.members.len())];
                        format!(
                            "\"{}\"邀请\"{}\"加入了群聊",
                            world.nick(other),
                            world.nick(invited)
                        )
                    }
                    7 | 8 => format!(
                        "<sysmsg type=\"revokemsg\"><revokemsg><session>{to}</session><msgid>{}</msgid><newmsgid>{}</newmsgid>\
                         <replacemsg><![CDATA[\"{}\" 撤回了一条消息]]></replacemsg></revokemsg></sysmsg>",
                        rng.between(1, 100_000),
                        ids.1,
                        esc(world.nick(other))
                    ),
                    _ => format!(
                        "你已添加了{}，现在可以开始聊天了。",
                        world.nick(other)
                    ),
                };
                made(text.clone(), &text)
            }
            42 => {
                let who = rng.below(world.people.len());
                let (user, nick) = (world.user(who), esc(world.nick(who)));
                made(
                    format!(
                        "<?xml version=\"1.0\"?>\n<msg bigheadimgurl=\"\" smallheadimgurl=\"\" username=\"{user}\" nickname=\"{nick}\" \
                         fullpy=\"\" shortpy=\"\" alias=\"\" imagestatus=\"3\" scene=\"17\" province=\"\" city=\"\" sign=\"\" sex=\"1\" \
                         certflag=\"0\" certinfo=\"\" brandIconUrl=\"\" brandHomeUrl=\"\" brandSubscriptConfigUrl=\"\" brandFlags=\"0\" \
                         regionCode=\"CN\" />\n"
                    ),
                    "[名片]",
                )
            }
            48 => {
                let place = esc(&words.text_between(rng, 4, 12));
                made(
                    format!(
                        "<?xml version=\"1.0\"?>\n<msg>\n\t<location x=\"{:.6}\" y=\"{:.6}\" scale=\"15\" label=\"{place}\" \
                         maptype=\"roadmap\" poiname=\"{place}\" poiid=\"qqmap_{}\" fromusername=\"{from}\" />\n</msg>\n",
                        22.0 + rng.unit() * 18.0,
                        100.0 + rng.unit() * 20.0,
                        rng.between(1, 1 << 40)
                    ),
                    "[位置]",
                )
            }
            t if t & 0xFFFF_FFFF == 49 => {
                self.app_message(rng, conv, t >> 32, sender, time, target, recent)?
            }
            other => made(String::new(), &other.to_string()),
        })
    }

    /// An app message (`local_type & 0xFFFFFFFF == 49`) of type `sub`.
    #[expect(
        clippy::too_many_arguments,
        reason = "one message's identity: conversation, sender, time and ids"
    )]
    fn app_message(
        &mut self,
        rng: &mut Rng,
        conv: usize,
        sub: i64,
        sender: usize,
        time: i64,
        target: f64,
        recent: &VecDeque<Recent>,
    ) -> Result<Made> {
        let world: &'a World = self.world;
        let c = &world.conversations[conv];
        let words = &world.words;
        let from = world.user(sender);
        let mut title = words.message(rng);
        let mut indexed = None;
        let mut summary = String::new();
        let inner = match sub {
            57 => {
                indexed = Some(title.clone());
                summary.clone_from(&title);
                let quoted = recent
                    .get(rng.below(recent.len()))
                    .map(|r| {
                        (
                            r.local_type,
                            r.server_id,
                            world.user(r.sender).to_string(),
                            world.nick(r.sender).to_string(),
                            r.content.clone(),
                            r.time,
                        )
                    })
                    .unwrap_or_else(|| {
                        (1, 1, from.to_string(), String::new(), String::new(), time)
                    });
                format!(
                    "\t\t<refermsg>\n\t\t\t<type>{}</type>\n\t\t\t<svrid>{}</svrid>\n\t\t\t<fromusr>{}</fromusr>\n\t\t\t<chatusr>{}</chatusr>\n\
                     \t\t\t<displayname>{}</displayname>\n\t\t\t<content>{}</content>\n\t\t\t<createtime>{}</createtime>\n\t\t</refermsg>\n",
                    quoted.0 & 0xFFFF_FFFF,
                    quoted.1,
                    c.username,
                    quoted.2,
                    esc(&quoted.3),
                    esc(&quoted.4),
                    quoted.5
                )
            }
            5 | 4 => {
                summary = format!("[链接] {title}");
                format!(
                    "\t\t<url>https://example.com/a/{}</url>\n\t\t<sourceusername>gh_{}</sourceusername>\n\t\t<sourcedisplayname>{}</sourcedisplayname>\n",
                    rng.hex(16),
                    rng.hex(12),
                    esc(rng.pick(TOPICS))
                )
            }
            6 => {
                let ext = *rng.pick(FILE_EXTENSIONS);
                let stem: String = words.text_between(rng, 2, 14).replace('，', "_");
                title = format!("{stem}.{ext}");
                summary = format!("[文件] {title}");
                let size = rng.spread(self.profile.media.file_bytes).max(100.0) as usize;
                if self.media && rng.chance(self.profile.media.file_here) {
                    let dir = month(time);
                    let path = self.account_dir.join("msg/file").join(&dir).join(&title);
                    let data = rng.bytes(size);
                    self.written.file("file", &path, &data)?;
                    self.written.hardlinks.push(Hardlink {
                        md5: hex(&Md5::digest(&data)),
                        file_name: title.clone(),
                        kind: 1,
                        dir1: dir,
                        dir2: String::new(),
                        size: size as i64,
                        modify_time: time,
                    });
                }
                format!(
                    "\t\t<appattach>\n\t\t\t<totallen>{size}</totallen>\n\t\t\t<attachid>@cdn_{}</attachid>\n\t\t\t<fileext>{ext}</fileext>\n\t\t</appattach>\n",
                    rng.hex(40)
                )
            }
            8 => {
                summary = "[动画表情]".into();
                format!(
                    "\t\t<appattach>\n\t\t\t<emoticonmd5>{}</emoticonmd5>\n\t\t</appattach>\n",
                    rng.hex(32).to_uppercase()
                )
            }
            19 | 24 => {
                title = if sub == 19 {
                    "群聊的聊天记录".into()
                } else {
                    words.text_plain(rng, 6)
                };
                summary = format!("[聊天记录] {title}");
                let n = rng.spread((10.0, 40.0, 200.0)).max(2.0) as usize;
                let items: String = (0..n)
                    .map(|_| {
                        let who = c.members[rng.below(c.members.len())];
                        format!(
                            "<dataitem datatype=\"1\" dataid=\"{}\"><datadesc>{}</datadesc><sourcename>{}</sourcename>\
                             <sourcetime>{}</sourcetime><sourceheadurl>https://wx.qlogo.invalid/{}</sourceheadurl></dataitem>",
                            rng.hex(32),
                            esc(&words.message(rng)),
                            esc(world.nick(who)),
                            time - rng.between(60, 30 * DAY),
                            rng.hex(40)
                        )
                    })
                    .collect();
                format!(
                    "\t\t<recorditem><![CDATA[<recordinfo><title>{}</title><desc>{}</desc><datalist count=\"{n}\">{items}</datalist></recordinfo>]]></recorditem>\n",
                    esc(&title),
                    esc(&words.text_plain(rng, 20))
                )
            }
            51 => {
                summary = format!("[视频号] {title}");
                format!(
                    "\t\t<finderFeed>\n\t\t\t<objectId>{}</objectId>\n\t\t\t<nickname>{}</nickname>\n\t\t\t<desc>{}</desc>\n\t\t\t<mediaCount>1</mediaCount>\n\t\t</finderFeed>\n",
                    rng.between(1, i64::MAX),
                    esc(&words.person_name(rng)),
                    esc(&words.message(rng))
                )
            }
            62 => {
                let patted = c.members[rng.below(c.members.len())];
                title = format!(
                    "\"{}\" 拍了拍 \"{}\"",
                    world.nick(sender),
                    world.nick(patted)
                );
                summary.clone_from(&title);
                format!(
                    "\t\t<patMsg>\n\t\t\t<chatUser>{}</chatUser>\n\t\t\t<records>\n\t\t\t\t<recordNum>1</recordNum>\n\t\t\t\t<record>\n\
                     \t\t\t\t\t<fromUser>{from}</fromUser>\n\t\t\t\t\t<pattedUser>{}</pattedUser>\n\t\t\t\t\t<templete><![CDATA[\"${{{from}}}\" 拍了拍 \"${{{}}}\"]]></templete>\n\
                     \t\t\t\t\t<createTime>{time}</createTime>\n\t\t\t\t</record>\n\t\t\t</records>\n\t\t</patMsg>\n",
                    c.username,
                    world.user(patted),
                    world.user(patted)
                )
            }
            87 => {
                summary = "[群公告]".into();
                format!(
                    "\t\t<textannouncement><![CDATA[{}]]></textannouncement>\n",
                    words.text_between(rng, 50, 450)
                )
            }
            2000 => {
                title = "转账".into();
                let other = c.members[rng.below(c.members.len())];
                let amount = rng.between(1, 200_000);
                summary = format!("[转账] ￥{}.{:02}", amount / 100, amount % 100);
                format!(
                    "\t\t<wcpayinfo>\n\t\t\t<paysubtype>{}</paysubtype>\n\t\t\t<feedesc>￥{}.{:02}</feedesc>\n\t\t\t<transcationid>{}</transcationid>\n\
                     \t\t\t<transferid>{}</transferid>\n\t\t\t<invalidtime>{}</invalidtime>\n\t\t\t<pay_memo>{}</pay_memo>\n\
                     \t\t\t<payer_username>{from}</payer_username>\n\t\t\t<receiver_username>{}</receiver_username>\n\t\t</wcpayinfo>\n",
                    1 + 2 * rng.below(2),
                    amount / 100,
                    amount % 100,
                    rng.hex(28),
                    rng.hex(28),
                    time + DAY,
                    esc(rng.pick(PHRASES)),
                    world.user(other)
                )
            }
            2001 => {
                title = "微信红包".into();
                summary = "[红包] 恭喜发财，大吉大利".into();
                format!(
                    "\t\t<wcpayinfo>\n\t\t\t<templateid>{}</templateid>\n\t\t\t<receivertitle><![CDATA[恭喜发财，大吉大利]]></receivertitle>\n\
                     \t\t\t<sendertitle><![CDATA[恭喜发财，大吉大利]]></sendertitle>\n\t\t\t<scenetext>微信红包</scenetext>\n\
                     \t\t\t<nativeurl>wxpay://c2cbizmessagehandler/hongbao/receivehongbao?msgtype=1&amp;channelid=1&amp;sendid={}</nativeurl>\n\
                     \t\t\t<sceneid>1002</sceneid>\n\t\t\t<invalidtime>{}</invalidtime>\n\t\t</wcpayinfo>\n",
                    rng.hex(32),
                    rng.between(1, i64::MAX),
                    time + DAY
                )
            }
            3 | 76 => {
                summary = format!("[音乐] {title}");
                format!(
                    "\t\t<url>https://example.com/m/{}</url>\n\t\t<songname>{}</songname>\n\t\t<singername>{}</singername>\n\t\t<musicurl>https://example.com/m/{}.mp3</musicurl>\n",
                    rng.hex(12),
                    esc(&title),
                    esc(&words.person_name(rng)),
                    rng.hex(12)
                )
            }
            33 | 36 => {
                summary = format!("[小程序] {title}");
                format!(
                    "\t\t<sourcedisplayname>{}</sourcedisplayname>\n\t\t<weappinfo>\n\t\t\t<username>gh_{}@app</username>\n\t\t\t<appid>wx{}</appid>\n\
                     \t\t\t<type>2</type>\n\t\t\t<pagepath>pages/index/index.html</pagepath>\n\t\t</weappinfo>\n",
                    esc(rng.pick(TOPICS)),
                    rng.hex(12),
                    rng.hex(16)
                )
            }
            _ => {
                summary = format!("[链接] {title}");
                String::new()
            }
        };
        let des = esc(&words.text_between(rng, 0, 40));
        let title = esc(&title);
        let content = sized(rng, target, |fill| {
            format!(
                "<?xml version=\"1.0\"?>\n<msg>\n\t<appmsg appid=\"\" sdkver=\"0\">\n\t\t<title>{title}</title>\n\t\t<des>{des}</des>\n\
                 \t\t<type>{sub}</type>\n\t\t<showtype>0</showtype>\n{inner}\t\t<thumburl>{fill}</thumburl>\n\t</appmsg>\n\
                 \t<fromusername>{from}</fromusername>\n\t<scene>0</scene>\n\t<appinfo>\n\t\t<version>1</version>\n\t\t<appname></appname>\n\
                 \t</appinfo>\n\t<commenturl></commenturl>\n</msg>\n"
            )
        });
        Ok(Made {
            content,
            indexed,
            summary,
            mention: None,
        })
    }

    /// A picture message: which of its files are here is drawn like the real ones.
    fn image(&mut self, rng: &mut Rng, talker: &str, time: i64, target: f64) -> Result<String> {
        let md5 = rng.hex(32);
        let p = &self.profile.media;
        let (thumb_here, image_here, hd_here, _) =
            p.image_sets[Weighted::new(p.image_sets.iter().map(|s| s.3)).draw(rng)];
        let thumb = rng.spread(p.thumb_bytes).max(800.0) as usize;
        let image = rng.spread(p.image_bytes).max(800.0) as usize;
        let hd = rng.spread(p.hd_bytes).max(4000.0) as usize;
        let (image_formats, hd_formats) = (p.image_formats, p.hd_formats);
        let has_original = hd_here || rng.chance(0.3);
        if self.media {
            let talker_dir = hex(&Md5::digest(talker.to_lowercase().as_bytes()));
            let dir = month(time);
            let folder = self
                .account_dir
                .join("msg/attach")
                .join(&talker_dir)
                .join(&dir)
                .join("Img");
            let formats = |rng: &mut Rng, (png_share, jpeg_share, _): (f64, f64, f64), len| {
                let x = rng.unit();
                if x < png_share {
                    png(rng, len)
                } else if x < png_share + jpeg_share {
                    jpeg(rng, len)
                } else {
                    wxgf(len)
                }
            };
            for (here, suffix, plain) in [
                (thumb_here, "_t", jpeg(rng, thumb)),
                (image_here, "", formats(rng, image_formats, image)),
                (hd_here, "_h", formats(rng, hd_formats, hd)),
            ] {
                if !here {
                    continue;
                }
                let file_name = format!("{md5}{suffix}.dat");
                let data = encrypt_dat(&plain);
                self.written
                    .file("image", &folder.join(&file_name), &data)?;
                self.written.hardlinks.push(Hardlink {
                    md5: md5.clone(),
                    file_name,
                    kind: 2,
                    dir1: talker_dir.clone(),
                    dir2: dir.clone(),
                    size: data.len() as i64,
                    modify_time: time,
                });
            }
        }
        let hd_attr = if has_original {
            format!(" hdlength=\"{hd}\"")
        } else {
            String::new()
        };
        let (w, h) = (120 + rng.below(200), 120 + rng.below(200));
        let (aes, origin, mid) = (rng.hex(32), rng.hex(32), rng.hex(220));
        Ok(sized(rng, target, |fill| {
            format!(
                "<?xml version=\"1.0\"?>\n<msg>\n\t<img aeskey=\"{aes}\" encryver=\"1\" cdnthumbaeskey=\"{aes}\" cdnthumburl=\"{fill}\" \
                 cdnthumblength=\"{thumb}\" cdnthumbheight=\"{h}\" cdnthumbwidth=\"{w}\" cdnmidheight=\"0\" cdnmidwidth=\"0\" \
                 cdnhdheight=\"0\" cdnhdwidth=\"0\" cdnmidimgurl=\"{mid}\" length=\"{image}\"{hd_attr} md5=\"{md5}\" \
                 hevc_mid_size=\"{image}\" originsourcemd5=\"{origin}\" />\n\t<platform_signature></platform_signature>\n\
                 \t<imgdatahash></imgdatahash>\n</msg>\n"
            )
        }))
    }

    fn video(&mut self, rng: &mut Rng, from: &str, time: i64, target: f64) -> Result<String> {
        let md5 = rng.hex(32);
        let p = &self.profile.media;
        let (thumb_here, video_here) = (p.video_thumb_here, p.video_here);
        let len = rng.spread(p.video_bytes).max(100_000.0) as usize;
        let thumb = rng.spread(p.video_thumb_bytes).max(1000.0) as usize;
        if self.media {
            let dir = month(time);
            let folder = self.account_dir.join("msg/video").join(&dir);
            if rng.chance(thumb_here) {
                let data = jpeg(rng, thumb);
                self.written.file(
                    "video thumbnail",
                    &folder.join(format!("{md5}_thumb.jpg")),
                    &data,
                )?;
            }
            if rng.chance(video_here) {
                let file_name = format!("{md5}.mp4");
                let data = mp4(rng, len);
                self.written
                    .file("video", &folder.join(&file_name), &data)?;
                self.written.hardlinks.push(Hardlink {
                    md5: md5.clone(),
                    file_name,
                    kind: 3,
                    dir1: dir,
                    dir2: String::new(),
                    size: len as i64,
                    modify_time: time,
                });
            }
        }
        let secs = rng.between(2, 300);
        let (aes, newmd5) = (rng.hex(32), rng.hex(32));
        Ok(sized(rng, target, |fill| {
            format!(
                "<?xml version=\"1.0\"?>\n<msg>\n\t<videomsg aeskey=\"{aes}\" cdnvideourl=\"{fill}\" cdnthumbaeskey=\"{aes}\" \
                 cdnthumburl=\"{aes}\" length=\"{len}\" playlength=\"{secs}\" cdnthumblength=\"{thumb}\" cdnthumbwidth=\"288\" \
                 cdnthumbheight=\"512\" fromusername=\"{from}\" md5=\"{md5}\" newmd5=\"{newmd5}\" isplaceholder=\"0\" rawmd5=\"\" \
                 rawlength=\"0\" cdnrawvideourl=\"\" cdnrawvideoaeskey=\"\" overwritenewmsgid=\"0\" originsourcemd5=\"\" isad=\"0\" />\n</msg>\n"
            )
        }))
    }
}

fn source(rng: &mut Rng, kind: Kind, members: usize, at: Option<&str>) -> String {
    let signature = format!("V1_{}|v1_{}", rng.hex(8), rng.hex(8));
    match kind {
        Kind::Group => format!(
            "<msgsource>{}<sec_msg_node><uuid>{}_</uuid><risk-file-flag /><risk-file-md5-list /></sec_msg_node>\
             <alnode><fr>1</fr></alnode><pua>1</pua><eggIncluded>1</eggIncluded><silence>{}</silence><membercount>{}</membercount><signature>{signature}</signature>\
             <tmp_node><publisher-id></publisher-id></tmp_node></msgsource>",
            at.map(|a| format!("<atuserlist><![CDATA[,{a}]]></atuserlist>"))
                .unwrap_or_default(),
            rng.hex(32),
            rng.below(2),
            members + 1
        ),
        _ => format!(
            "<msgsource><alnode><fr>1</fr></alnode><bizflag>0</bizflag><pua>1</pua><eggIncluded>1</eggIncluded><signature>{signature}</signature><tmp_node><publisher-id></publisher-id></tmp_node></msgsource>"
        ),
    }
}

fn zstd_or_plain(data: String, compress: bool) -> Result<(Vec<u8>, bool)> {
    Ok(if compress {
        (zstd::encode_all(data.as_bytes(), 1)?, true)
    } else {
        (data.into_bytes(), false)
    })
}

/// One message database being written: its tables, its `Name2Id` rows and the rows left for its WAL.
struct Shard {
    conn: Connection,
    ids: HashMap<String, i64>,
    tables: HashSet<String>,
    /// Messages from this one on go to the WAL.
    wal_from: (i64, usize),
    late: Vec<(String, Row)>,
}

impl Shard {
    fn id(&mut self, username: &str, session: bool) -> Result<i64> {
        if let Some(id) = self.ids.get(username) {
            return Ok(*id);
        }
        let id = self.ids.len() as i64 + 1;
        self.conn.execute(
            "insert into Name2Id(rowid, user_name, is_session) values (?1, ?2, ?3)",
            params![id, username, i64::from(session)],
        )?;
        self.ids.insert(username.to_string(), id);
        Ok(id)
    }

    fn insert(&mut self, world: &World, table: &str, row: &Row) -> Result<()> {
        let sender = match row.sender {
            Some(p) => self.id(world.user(p), false)?,
            None => 0,
        };
        // compressed content is a blob, the rest text
        let text = std::str::from_utf8(&row.content).unwrap_or_default();
        let content: &dyn rusqlite::ToSql = if row.compressed { &row.content } else { &text };
        let source_text;
        let source: &dyn rusqlite::ToSql = match &row.source {
            Some(s) if row.source_compressed => s,
            Some(s) => {
                source_text = String::from_utf8_lossy(s).into_owned();
                &source_text
            }
            None => &rusqlite::types::Null,
        };
        self.conn
            .prepare_cached(&format!(
                "insert into \"{table}\"(local_id, server_id, local_type, sort_seq, real_sender_id, create_time, status, \
                 message_content, source) values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)"
            ))?
            .execute(params![
                row.local_id,
                row.server_id,
                row.local_type,
                row.sort_seq,
                sender,
                row.time,
                row.status,
                content,
                source
            ])?;
        Ok(())
    }
}

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
