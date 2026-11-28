//! 合并转发的聊天记录（`<type>19`）：`<datalist>` 中的条目，与真实账号的聊天记录一致（#1242）：
//! 文字、图片和视频，每条有自己的 `dataid`，以及微信写的各个字段。

use std::collections::HashSet;

use sha2::{Digest, Sha256};

use super::files::civil;
use super::rng::{Rng, Weighted};
use super::words::esc;
use super::world::World;
use super::{DAY, TZ};
use crate::db::hex;

/// 每份聊天记录的条目数：中位数、第 90 百分位数和微信最多保存的条数（真实账号上分别是 6、28 和 100）。
const ITEMS: (f64, f64, f64) = (6.0, 28.0, 100.0);

/// 条目的 `datatype` 和权重：文字、图片和视频的比例与真实账号一致（5962 个条目中分别占 80.7%、7.6% 和 3.2%）；
/// 那里的表情、链接、文件和嵌套的聊天记录（8.5%）在这里都写成文字。
const KINDS: &[(u8, f64)] = &[(1, 0.892), (2, 0.076), (4, 0.032)];

/// 条目的 `<sourcetime>` 的几种写法及其频率（`%-m` 表示不补前导零）。
const SOURCE_TIMES: &[(&str, f64)] = &[
    ("%Y-%m-%d %H:%M", 0.50),
    ("%Y-%m-%d %H:%M:%S", 0.13),
    ("%Y-%m-%d&#x20;%H:%M:%S", 0.18),
    ("%Y-%-m-%-d %H:%M", 0.19),
];

/// 在会话 `conv` 中于 `time` 转发的聊天记录的条目，以及条目数。
pub(super) fn items(rng: &mut Rng, world: &World, conv: usize, time: i64) -> (String, usize) {
    let members = &world.conversations[conv].members;
    let n = rng.spread(ITEMS).max(1.0) as usize;
    let kinds = Weighted::new(KINDS.iter().map(|k| k.1));
    let times = Weighted::new(SOURCE_TIMES.iter().map(|f| f.1));
    let mut ids = HashSet::new();
    let mut out = String::new();
    for _ in 0..n {
        let who = members[rng.below(members.len())];
        let sent = time - rng.between(60, 30 * DAY);
        let id = loop {
            let id = rng.hex(32);
            if ids.insert(id.clone()) {
                break id;
            }
        };
        let datatype = KINDS[kinds.draw(rng)].0;
        let htmlid = match datatype {
            1 => 0.49,
            2 => 0.37,
            _ => 0.98,
        };
        let htmlid = if rng.chance(htmlid) {
            format!(" htmlid=\"{}\"", rng.hex(32))
        } else {
            String::new()
        };
        let source = format!(
            "<sourcename>{}</sourcename><sourcetime>{}</sourcetime>\
             <sourceheadurl>https://wx.qlogo.invalid/mmhead/ver_1/{}/132</sourceheadurl>",
            esc(&world.people[who].nick),
            source_time(SOURCE_TIMES[times.draw(rng)].0, sent),
            rng.hex(130)
        );
        let local_id = match datatype {
            1 => 0.67,
            2 => 0.55,
            _ => 0.98,
        };
        let local_id = if rng.chance(local_id) {
            let id = rng.spread((30_000.0, 200_000.0, 999_999.0)).max(1.0) as i64;
            format!("<srcMsgLocalid>{id}</srcMsgLocalid>")
        } else {
            String::new()
        };
        let tail = format!(
            "{local_id}<srcMsgCreateTime>{sent}</srcMsgCreateTime><fromnewmsgid>{}</fromnewmsgid>\
             <dataitemsource><hashusername>{}</hashusername></dataitemsource>",
            rng.between(1_000_000_000_000_000_000, i64::MAX),
            hex(&Sha256::digest(world.user(who).as_bytes()))
        );
        let body = match datatype {
            2 => picture(rng, &source),
            4 => video(rng, &source),
            _ => format!(
                "<datadesc>{}</datadesc>{source}",
                esc(&world.words.message(rng))
            ),
        };
        out.push_str(&format!(
            "<dataitem datatype=\"{datatype}\" dataid=\"{id}\"{htmlid}>{body}{tail}</dataitem>"
        ));
    }
    (out, n)
}

/// 图片或视频在微信服务器上的文件字段：先是缩略图的，然后是文件本身的。
fn files(rng: &mut Rng, thumb_md5: bool, data_bytes: f64) -> String {
    let url = |rng: &mut Rng| {
        let len = if rng.chance(0.75) { 194 } else { 178 };
        rng.hex(len)
    };
    let thumb_md5 = if thumb_md5 {
        format!("<thumbfullmd5>{}</thumbfullmd5>", rng.hex(32))
    } else {
        String::new()
    };
    format!(
        "<cdnthumburl>{}</cdnthumburl><cdnthumbkey>{}</cdnthumbkey>{thumb_md5}<thumbsize>{}</thumbsize>\
         <cdndataurl>{}</cdndataurl><cdndatakey>{}</cdndatakey><fullmd5>{}</fullmd5><datasize>{}</datasize>",
        url(rng),
        rng.hex(32),
        rng.spread((5_000.0, 13_000.0, 60_000.0)).max(1_000.0) as i64,
        url(rng),
        rng.hex(32),
        rng.hex(32),
        data_bytes as i64
    )
}

fn picture(rng: &mut Rng, source: &str) -> String {
    let format = match rng.unit() {
        x if x < 0.39 => "<datafmt>jpg</datafmt>",
        x if x < 0.60 => "<datafmt>pic</datafmt>",
        _ => "",
    };
    let size = if rng.chance(0.57) {
        format!(
            "<thumbwidth>{}</thumbwidth><thumbheight>{}</thumbheight>",
            rng.between(100, 999),
            rng.between(100, 999)
        )
    } else {
        String::new()
    };
    let thumb_md5 = rng.chance(0.91);
    let bytes = rng
        .spread((300_000.0, 1_500_000.0, 9_999_999.0))
        .max(20_000.0);
    let files = files(rng, thumb_md5, bytes);
    let types = if rng.chance(0.61) {
        "<filetype>2</filetype><thumbfiletype>1</thumbfiletype>"
    } else {
        ""
    };
    format!(
        "{format}{source}{size}{files}{types}<messageuuid>{}_</messageuuid>",
        rng.hex(32)
    )
}

fn video(rng: &mut Rng, source: &str) -> String {
    let seconds = rng.spread((20.0, 90.0, 600.0)).max(1.0) as i64;
    let bytes = rng
        .spread((3_000_000.0, 12_000_000.0, 99_999_999.0))
        .max(100_000.0);
    let files = files(rng, true, bytes);
    let types = if rng.chance(0.98) {
        "<thumbfiletype>1</thumbfiletype>"
    } else {
        ""
    };
    format!(
        "<datafmt>.mp4</datafmt>{source}<duration>{seconds}</duration>{files}{types}<messageuuid>{}_</messageuuid>",
        rng.hex(32)
    )
}

/// 按账号的时区，用 `format` 写出 `t`：支持 `%Y`、`%m`、`%d`、`%H`、`%M`、`%S`，
/// 以及不补前导零的 `%-m`、`%-d`。
fn source_time(format: &str, t: i64) -> String {
    let (y, m, d) = civil(t);
    let seconds = (t + TZ).rem_euclid(DAY);
    format
        .replace("%Y", &y.to_string())
        .replace("%-m", &m.to_string())
        .replace("%-d", &d.to_string())
        .replace("%m", &format!("{m:02}"))
        .replace("%d", &format!("{d:02}"))
        .replace("%H", &format!("{:02}", seconds / 3600))
        .replace("%M", &format!("{:02}", seconds % 3600 / 60))
        .replace("%S", &format!("{:02}", seconds % 60))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_source_time_is_written_in_the_account_s_clock() {
        // UTC+8 的 2024-01-05 01:02:03
        let t = 1_704_387_723;
        assert_eq!(source_time("%Y-%m-%d %H:%M:%S", t), "2024-01-05 01:02:03");
        assert_eq!(source_time("%Y-%-m-%-d %H:%M", t), "2024-1-5 01:02");
        assert_eq!(
            source_time("%Y-%m-%d&#x20;%H:%M:%S", t),
            "2024-01-05&#x20;01:02:03"
        );
    }
}
