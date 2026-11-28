//! profile：基准账号从真实账号照搬的比例和大小，为 #488 测得的默认值，以及校准得到的 profile 的读取和检查（#605）。

use std::path::Path;

use anyhow::{ensure, Context, Result};
use md5::{Digest, Md5};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::MIB;
use crate::db::hex;

/// 一种大小的分布：（中位数，第 90 百分位数，最大值）。
pub type Spread = (f64, f64, f64);

/// 基准账号从真实账号照搬的分布（#488、#605）：只有比例和大小，从不包含名字或消息。
///
/// [`Profile::default`] 是为 #488 测得的那一份（即下面的常量）；一台机器也可以用
/// `account_profile_probe --profile-out` 从真实账号校准出自己的 profile，
/// 它只留在那台机器上，由 [`Profile::load`] 读取。它的内容从不公开：基准结果用 [`Profile::hash`] 和测量的日期指代它。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    /// 测量的日期 `YYYY-MM-DD`。
    pub measured: String,
    /// 消息的类型和各列。
    pub messages: MessageProfile,
    /// 消息的发送时间。
    pub time: TimeProfile,
    /// 文字的内容。
    pub text: TextProfile,
    /// 群。
    pub groups: GroupProfile,
    /// 媒体文件。
    pub media: MediaProfile,
}

/// 消息的类型和各列的比例。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MessageProfile {
    /// 各类型的比例 `(local_type, share)`；app 消息的 `local_type` 是 `(sub << 32) | 49`。
    pub types: Vec<(i64, f64)>,
    /// 群消息中账号所有者发送的比例。
    pub sent_in_group: f64,
    /// 私聊消息中账号所有者发送的比例。
    pub sent_in_private: f64,
    /// 群消息中 @ 了某人的比例。
    pub at_in_group: f64,
    /// 带 `source`（`msgsource` XML）的消息的比例。
    pub with_source: f64,
    /// 这些 `source` 中压缩存储的比例。
    pub source_compressed: f64,
    /// `sort_seq` 与发送时间相差几天的消息的比例。
    pub sort_seq_off: f64,
    /// `sort_seq` 为 0 的消息的比例。
    pub sort_seq_missing: f64,
    /// 各类型内容的字节数（文字以外的类型是 XML 的大小）。
    pub content_bytes: Vec<(i64, Spread)>,
    /// `content_bytes` 没有列出的类型的内容字节数。
    pub content_bytes_other: Spread,
}

/// 消息的发送时间。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimeProfile {
    /// 每天各小时的消息比例（UTC+8）。
    pub hours: [f64; 24],
    /// 一周各天的消息比例，周一在前。
    pub weekdays: [f64; 7],
    /// 一阵连续消息中相邻两条的间隔秒数，写成 `(from, to, share)`。
    pub burst_gaps: Vec<(i64, i64, f64)>,
}

/// 文字的内容。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextProfile {
    /// 字符数的分布，写成 `(from, to, share)`。
    pub chars: Vec<(u32, u32, f64)>,
    /// 含 emoji 字符的文字的比例。
    pub with_emoji: f64,
    /// 含方括号写法的微信表情（如 `[微笑]`）的文字的比例。
    pub with_bracket_emoji: f64,
    /// 多于一行的文字的比例。
    pub with_newline: f64,
    /// 含控制字符的文字的比例。
    pub with_odd_control: f64,
    /// 含网址的文字的比例。
    pub with_url: f64,
}

/// 群的分布。
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupProfile {
    /// 会话中群聊的比例。
    pub share: f64,
    /// 最大的群的成员数。
    pub big_members: usize,
    /// 最大的群中发过言的成员数。
    pub big_speakers: usize,
    /// 群里谁在说话：排第 `r`（从 0 起）的成员发言的权重为 `(r + 1 + q) ^ -s`，这是其中的 `q`。
    pub speaker_q: f64,
    /// `speaker_q` 的公式中的 `s`。
    pub speaker_s: f64,
}

/// 媒体文件的分布。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaProfile {
    /// 一张图片的哪些文件在电脑上：`(thumbnail, image, original, share)`，即缩略图、普通图、原图和比例。
    pub image_sets: Vec<(bool, bool, bool, f64)>,
    /// 缩略图 `.dat` 文件的大小。
    pub thumb_bytes: Spread,
    /// 普通图 `.dat` 文件的大小。
    pub image_bytes: Spread,
    /// 原图 `.dat` 文件的大小。
    pub hd_bytes: Spread,
    /// 普通图文件里各格式的比例 `(png, jpeg, wxgf)`。
    pub image_formats: (f64, f64, f64),
    /// 原图文件里各格式的比例，写法同 `image_formats`。
    pub hd_formats: (f64, f64, f64),
    /// 视频的大小。
    pub video_bytes: Spread,
    /// 视频缩略图的大小。
    pub video_thumb_bytes: Spread,
    /// 缩略图在电脑上的视频的比例。
    pub video_thumb_here: f64,
    /// 在电脑上的视频的比例。
    pub video_here: f64,
    /// 收发的文件的大小。
    pub file_bytes: Spread,
    /// 在电脑上的文件的比例。
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
    /// 读取保存成 JSON 的 profile，并像 [`check`](Self::check) 那样检查。
    ///
    /// # Errors
    /// 文件读不了、不是 profile，或者没有通过检查时。
    pub fn load(path: &Path) -> Result<Self> {
        let bytes =
            std::fs::read(path).with_context(|| format!("read the profile {}", path.display()))?;
        let profile: Self = serde_json::from_slice(&bytes).context("parse the profile")?;
        profile.check()?;
        Ok(profile)
    }

    /// 检查生成器对它的要求：每组比例的总和为正，每个比例都在 `0..=1` 之内，
    /// 每个范围都是有序的，最大的群至少有一个人发言。
    ///
    /// # Errors
    /// 第一个不满足要求的部分，错误中写明它的名称。
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

    /// 代表其内容的短名称：它的 JSON 的 MD5 的前 16 位十六进制数字，其中每个小数写成 9 位有效数字，
    /// 这样从文件读回的 profile（最后一位可能差 1）名称也相同。
    ///
    /// # Errors
    /// 无法序列化时；只含普通数据的 profile 不会这样。
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

// 默认的 profile，为 #488 在一个真实账号上测得。
const MEASURED: &str = "2026-11-02";

pub(super) const fn app(sub: i64) -> i64 {
    (sub << 32) | 49
}

/// 各消息类型的比例 `(local_type, share)`；app 消息的 `local_type` 是 `(sub << 32) | 49`。
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

/// 每天各小时的消息比例。
const HOURS: [f64; 24] = [
    0.0400, 0.0139, 0.0080, 0.0029, 0.0031, 0.0039, 0.0074, 0.0093, 0.0223, 0.0425, 0.0604, 0.0706,
    0.0612, 0.0483, 0.0382, 0.0338, 0.0461, 0.0551, 0.0582, 0.0687, 0.0684, 0.0649, 0.1083, 0.0643,
];
/// 一周各天的消息比例，周一在前。
const WEEKDAYS: [f64; 7] = [0.1389, 0.1371, 0.1601, 0.1940, 0.1288, 0.1259, 0.1152];
/// 文字的字符数分布 `(from, to, share)`。
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
/// 一阵连续消息中相邻两条的间隔秒数 `(from, to, share)`。
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
/// 群里谁在说话：排第 `r`（从 0 起）的成员发言的权重为 `(r + 1 + Q) ^ -S`，
/// 得出的比例与真实的群一样：第一名占 13%，前十分之一的成员占 91%。
const SPEAKER_Q: f64 = 18.5;
const SPEAKER_S: f64 = 3.8;
/// 最大的群：成员数，以及其中发过言的人数。
const BIG_MEMBERS: usize = 369;
const BIG_SPEAKERS: usize = 251;
const GROUP_SHARE: f64 = 0.08;
/// 一张图片的哪些文件在电脑上：`(thumbnail, image, original, share)`。
const IMAGE_SETS: &[(bool, bool, bool, f64)] = &[
    (true, false, false, 0.532),
    (true, true, false, 0.437),
    (true, true, true, 0.030),
    (false, true, false, 0.001),
];
/// 文件的字节数 `(median, 90th percentile, at most)`：中位数、第 90 百分位数和最大值。
const THUMB_BYTES: (f64, f64, f64) = (5053.0, 11_639.0, 98_000.0);
const IMAGE_BYTES: (f64, f64, f64) = (46_540.0, 118_757.0, 2_200_000.0);
const HD_BYTES: (f64, f64, f64) = (120_040.0, 722_310.0, 8_200_000.0);
const VIDEO_BYTES: (f64, f64, f64) = (2_872_790.0, 14_861_110.0, 8.0 * MIB as f64);
const VIDEO_THUMB_BYTES: (f64, f64, f64) = (10_315.0, 25_880.0, 360_000.0);
const FILE_BYTES: (f64, f64, f64) = (283_610.0, 6_863_770.0, 8.0 * MIB as f64);
/// 普通图文件里各格式的比例 `(png, jpeg, wxgf)`。
const IMAGE_FORMATS: (f64, f64, f64) = (0.265, 0.015, 0.720);
const HD_FORMATS: (f64, f64, f64) = (0.567, 0.233, 0.200);
const VIDEO_THUMB_HERE: f64 = 0.9;
const VIDEO_HERE: f64 = 0.1;
const FILE_HERE: f64 = 0.35;

/// 各类型内容字节数的 `(median, 90th percentile, largest)`（文字以外的类型是 XML 的大小）。
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
