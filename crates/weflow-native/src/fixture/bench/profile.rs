//! The profile: the shares and sizes the account copies from a real one, the default measured for #488, and
//! loading and checking a calibrated one (#605).

use std::path::Path;

use anyhow::{ensure, Context, Result};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::MIB;
use crate::db::hex;

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

    pub(super) fn content_bytes(&self, local_type: i64) -> Spread {
        self.messages
            .content_bytes
            .iter()
            .find(|(t, _)| *t == local_type)
            .map_or(self.messages.content_bytes_other, |(_, s)| *s)
    }
}

// The default profile, measured on a real account for #488.
const MEASURED: &str = "2026-11-02";

pub(super) const fn app(sub: i64) -> i64 {
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
