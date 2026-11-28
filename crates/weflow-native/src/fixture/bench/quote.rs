//! 引用消息：引用的是之前的哪一条消息，以及它的 `<refermsg>` 怎样写，与真实账号一致（#1242）。

use std::collections::VecDeque;

use super::messages::Recent;
use super::profile::Spread;
use super::rng::{Rng, Weighted};
use super::words::esc;
use super::world::World;

/// 引用消息怎样引用各种 `<type>` 的消息。
struct Quoted {
    /// 被引用的消息的类型（它的 `local_type` 的低 32 位）。
    kind: i64,
    /// 与其他类型相比被引用的频率（权重）。
    weight: f64,
    /// 引用时写成纯文字的比例：消息的文字，或者 `[视频]` 这样的摘要。
    plain: f64,
    /// 写成 XML（转义一次）的引用中，`declaration` 是以 XML 声明开头的比例，
    /// `sender_line` 是以写明发送者的一行开头的比例；其余的以 `<msg>` 开头。
    declaration: f64,
    sender_line: f64,
}

/// 一个真实账号的引用消息（36125 条，见 `weflow-core` 的 `reply_record_probe`）：文字按原文引用，
/// 其他类型大多按 XML 引用；表情和视频偶尔按摘要引用。
const QUOTED: &[Quoted] = &[
    Quoted {
        kind: 1,
        weight: 21_740.0,
        plain: 1.0,
        declaration: 0.0,
        sender_line: 0.0,
    },
    Quoted {
        kind: 3,
        weight: 3_070.0,
        plain: 0.001,
        declaration: 0.777,
        sender_line: 0.052,
    },
    Quoted {
        kind: 34,
        weight: 440.0,
        plain: 0.032,
        declaration: 0.0,
        sender_line: 0.047,
    },
    Quoted {
        kind: 43,
        weight: 336.0,
        plain: 0.42,
        declaration: 0.641,
        sender_line: 0.138,
    },
    Quoted {
        kind: 47,
        weight: 531.0,
        plain: 0.134,
        declaration: 0.015,
        sender_line: 0.0,
    },
    Quoted {
        kind: 49,
        weight: 10_007.0,
        plain: 0.001,
        declaration: 0.631,
        sender_line: 0.006,
    },
];

/// 被引用的回复中，XML 保留它所引用消息的 `<refermsg>`（写成纯文字，不带 `<msgsource>`）的比例；
/// 其余的不保留。真实账号上 8990 条被引用的回复中有 6257 条保留。
const KEEPS_ITS_QUOTE: f64 = 0.70;

/// 按引用的写法，回复相对于全部回复的大小：真实账号上每种写法的中位数和第 90 百分位数，
/// 除以全部回复的中位数和第 90 百分位数（1575 和 10549 字节）。
/// 纯文字的引用较小；引用表情或语音（以 `<msg>` 开头）稍大，很少大很多。
const PLAIN_SIZE: (f64, f64) = (0.796, 0.946);
const DECLARATION_SIZE: (f64, f64) = (1.714, 1.325);
const MSG_SIZE: (f64, f64) = (1.219, 0.287);
const SENDER_LINE_SIZE: (f64, f64) = (2.018, 1.501);

const DECLARATION: &str = "<?xml version=\"1.0\"?>\n";

/// 一条回复的 `<refermsg>`。
pub(super) struct Refer {
    /// 回复中存储的样子。
    pub(super) stored: String,
    /// 这条回复再被引用时，留在它自己的 XML 中的样子：带被引用消息的纯文字；不保留时为空。
    pub(super) when_quoted: String,
    /// 这条回复的大小相对于全部回复的大小（[`PLAIN_SIZE`] 等）。
    size: (f64, f64),
}

impl Refer {
    /// 由全部回复的大小分布，得出按这种写法引用的回复的大小分布。
    pub(super) fn sizes(&self, (median, p90, max): Spread) -> Spread {
        (median * self.size.0, p90 * self.size.1, max)
    }
}

/// `from` 在 `chat` 里发的一条回复的 `<refermsg>`，引用 `recent` 中的一条消息：
/// 按引用各类型的频率抽一种类型，取这种类型最新的一条；没有这种类型时任取一条。
pub(super) fn refermsg(
    rng: &mut Rng,
    world: &World,
    chat: &str,
    from: &str,
    time: i64,
    recent: &VecDeque<Recent>,
) -> Refer {
    let wanted = &QUOTED[Weighted::new(QUOTED.iter().map(|q| q.weight)).draw(rng)];
    let low = |r: &Recent| r.local_type & 0xFFFF_FFFF;
    let quoted = recent
        .iter()
        .rev()
        .find(|r| low(r) == wanted.kind)
        .or_else(|| recent.get(rng.below(recent.len().max(1))));
    let Some(r) = quoted else {
        let empty = element(1, 1, chat, from, "", Some(""), "", time);
        return Refer {
            when_quoted: empty.clone(),
            stored: empty,
            size: PLAIN_SIZE,
        };
    };
    let how = QUOTED.iter().find(|q| q.kind == low(r));
    let (content, size) = match how {
        Some(how) if !r.xml.is_empty() && !rng.chance(how.plain) => {
            let body = r.xml.strip_prefix(DECLARATION).unwrap_or(&r.xml);
            let x = rng.unit();
            let (xml, size) = if x < how.sender_line {
                (
                    format!("{}:\n{body}", world.user(r.sender)),
                    SENDER_LINE_SIZE,
                )
            } else if x < how.sender_line + how.declaration {
                (format!("{DECLARATION}{body}"), DECLARATION_SIZE)
            } else {
                (body.to_string(), MSG_SIZE)
            };
            (esc(&xml), size)
        }
        _ => (esc(&r.plain), PLAIN_SIZE),
    };
    let (sender, nick) = (world.user(r.sender), &world.people[r.sender].nick);
    let stored = element(
        low(r),
        r.server_id,
        chat,
        sender,
        nick,
        Some(&esc(&r.source)),
        &content,
        r.time,
    );
    let when_quoted = if rng.chance(KEEPS_ITS_QUOTE) {
        element(
            low(r),
            r.server_id,
            chat,
            sender,
            nick,
            None,
            &esc(&r.plain),
            r.time,
        )
    } else {
        String::new()
    };
    Refer {
        stored,
        when_quoted,
        size,
    }
}

/// 一个 `<refermsg>`；`source` 是转义过的 `<msgsource>`，为 `None` 时不写。
#[expect(
    clippy::too_many_arguments,
    reason = "一个 <refermsg> 的各个字段，每个只写一次"
)]
fn element(
    kind: i64,
    server_id: i64,
    chat: &str,
    sender: &str,
    nick: &str,
    source: Option<&str>,
    content: &str,
    time: i64,
) -> String {
    let source = source
        .map(|s| format!("\t\t\t<msgsource>{s}</msgsource>\n"))
        .unwrap_or_default();
    format!(
        "\t\t<refermsg>\n\t\t\t<type>{kind}</type>\n\t\t\t<svrid>{server_id}</svrid>\n\t\t\t<fromusr>{chat}</fromusr>\n\
         \t\t\t<chatusr>{sender}</chatusr>\n\t\t\t<displayname>{}</displayname>\n{source}\
         \t\t\t<content>{content}</content>\n\t\t\t<createtime>{time}</createtime>\n\t\t</refermsg>\n",
        esc(nick)
    )
}
