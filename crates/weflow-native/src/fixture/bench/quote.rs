//! Replies: which earlier message a reply quotes and how its `<refermsg>` writes it, as on a real account (#1242).

use std::collections::VecDeque;

use super::messages::Recent;
use super::profile::Spread;
use super::rng::{Rng, Weighted};
use super::words::esc;
use super::world::World;

/// How replies quote each `<type>` of message.
struct Quoted {
    /// The quoted message's type (the low 32 bits of its `local_type`).
    kind: i64,
    /// How often it is quoted, against the other kinds.
    weight: f64,
    /// The share of its quotes written as plain text: its words, or a summary such as `[视频]`.
    plain: f64,
    /// Of the quotes written as its XML (escaped once), the share beginning with the XML declaration and the share
    /// beginning with a line naming its sender; the others begin with `<msg>`.
    declaration: f64,
    sender_line: f64,
}

/// The replies of a real account (36,125, `reply_record_probe` in `weflow-core`): text is quoted as itself, the
/// others mostly as their XML; a sticker or a video is now and then quoted by its summary.
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

/// The share of quoted replies whose XML keeps the `<refermsg>` of what they quote, by its plain text and without
/// its `<msgsource>`; the others leave it out (on a real account, 6,257 of the 8,990 quoted replies keep it).
const KEEPS_ITS_QUOTE: f64 = 0.70;

/// How big a reply is against replies on the whole, by how it quotes: the median and the 90th percentile of each form
/// on a real account over those of all its replies (1,575 and 10,549 bytes). A plain quote is small; a quote of a
/// sticker or a voice message (from `<msg>`) is a little bigger and rarely much bigger.
const PLAIN_SIZE: (f64, f64) = (0.796, 0.946);
const DECLARATION_SIZE: (f64, f64) = (1.714, 1.325);
const MSG_SIZE: (f64, f64) = (1.219, 0.287);
const SENDER_LINE_SIZE: (f64, f64) = (2.018, 1.501);

const DECLARATION: &str = "<?xml version=\"1.0\"?>\n";

/// A reply's `<refermsg>`.
pub(super) struct Refer {
    /// As the reply stores it.
    pub(super) stored: String,
    /// As it stays in the reply's own XML when that reply is quoted in turn: with the quoted message's plain text, or
    /// empty when it is left out.
    pub(super) when_quoted: String,
    /// The size of the reply against the size of replies on the whole ([`PLAIN_SIZE`] and the others).
    size: (f64, f64),
}

impl Refer {
    /// The sizes of a reply quoting this way, out of the sizes of replies on the whole.
    pub(super) fn sizes(&self, (median, p90, max): Spread) -> Spread {
        (median * self.size.0, p90 * self.size.1, max)
    }
}

/// The `<refermsg>` of a reply in `chat` by `from`, quoting one of the `recent` messages: the newest of a kind drawn
/// as replies quote kinds, else any of them.
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

/// One `<refermsg>`; `source` is its `<msgsource>`, escaped, or `None` for none.
#[expect(
    clippy::too_many_arguments,
    reason = "the fields of one <refermsg>, each written once"
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
