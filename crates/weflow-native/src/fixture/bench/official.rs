//! 公众号发来的消息，写在微信保存它们的 `message/biz_message_0.db` 里（#1068）：会话列表中每个公众号的文字和推送，
//! 以及微信支付的通知。
//!
//! 分布取自一个真实账号（`account_profile_probe` 的 `official` 部分）：会话列表中的每个公众号都有消息，每个几条；
//! 八分之七是文字，其余是单篇文章的推送，XML 有几 KB。那个账号没有微信支付的通知；这里的几条是为了让
//! `biz pay-records` 有内容可读。

use std::collections::{HashMap, HashSet};

use anyhow::Result;

use super::messages::{zstd_or_plain, Row, Shard};
use super::rng::Rng;
use super::words::{esc, sized, TOPICS};
use super::world::{Kind, World};
use super::{DAY, ZSTD_ABOVE};
use crate::fixture::{message_table_schema, Fixture, NAME2ID_SCHEMA};
use crate::msg::table_name_for;
use crate::sqlcipher::testutil::{encrypt_db, plain_connection, plain_image};

/// 微信支付的公众号，`biz pay-records` 读取它的通知。
pub(super) const WECHAT_PAY: &str = "gh_3dfda90e39d6";
/// 链接或推送的 app 消息的类型：49，卡片的子类型（5）在低 32 位之上。
const PUSH: i64 = (5 << 32) | 49;
/// 一个公众号的消息数：中位数、第 90 百分位数和最大值，取自真实账号。
const PER_ACCOUNT: (f64, f64, f64) = (4.0, 10.0, 30.0);
/// 公众号消息中推送所占的比例；其余是文字。
const PUSH_SHARE: f64 = 0.125;
/// 公众号文字消息的字符数：中位数、第 90 百分位数和最大值。
const TEXT_CHARS: (f64, f64, f64) = (69.0, 294.0, 1_900.0);
/// 推送的 XML 的字节数：中位数、第 90 百分位数和最大值。
const PUSH_BYTES: (f64, f64, f64) = (6_000.0, 8_400.0, 16_000.0);
/// 微信支付通知的条数。
const PAY_NOTICES: usize = 8;

/// 公众号最新的一条消息，用于它在会话列表中的行。
pub(super) struct Newest {
    pub(super) time: i64,
    pub(super) local_type: i64,
    pub(super) summary: String,
}

/// [`write()`] 写入的内容：每个会话（按序号）的消息数和最新的一条消息，以及每种类型的消息数。
#[derive(Default)]
pub(super) struct Written {
    pub(super) newest: HashMap<usize, (usize, Newest)>,
    pub(super) by_type: HashMap<i64, u64>,
}

/// 写入 `message/biz_message_0.db`，其中是 `world` 中每个公众号的消息，最新的一条在它的会话所记的时间。
pub(super) fn write(f: &Fixture, world: &World, rng: &Rng) -> Result<Written> {
    let conn = plain_connection();
    conn.execute_batch(NAME2ID_SCHEMA)?;
    conn.execute_batch("begin")?;
    let mut shard = Shard {
        conn,
        ids: HashMap::new(),
        tables: HashSet::new(),
        wal_from: (i64::MAX, usize::MAX),
        late: Vec::new(),
    };
    let mut written = Written::default();
    for (ci, c) in world.conversations.iter().enumerate() {
        if c.kind != Kind::Official {
            continue;
        }
        let mut rng = rng.fork(ci as u64);
        let account = c.members[0];
        let n = if c.username == WECHAT_PAY {
            PAY_NOTICES
        } else {
            rng.spread(PER_ACCOUNT).max(1.0) as usize
        };
        // 最新的在最后：最新的一条在会话的时间，其余的在它之前，彼此相隔几天
        let mut times = vec![c.times[0]];
        for _ in 1..n {
            let before = times[times.len() - 1] - rng.between(DAY / 2, 30 * DAY);
            times.push(before);
        }
        times.reverse();
        let table = table_name_for(&c.username);
        shard.conn.execute_batch(&message_table_schema(&table))?;
        shard.id(&c.username, true)?;
        let mut newest = None;
        for (i, &time) in times.iter().enumerate() {
            let (local_type, content, summary) = if c.username == WECHAT_PAY {
                let (xml, title) = notice(&mut rng, time, i);
                (PUSH, xml, title)
            } else if rng.chance(PUSH_SHARE) {
                let (xml, title) = push(world, &mut rng, &c.username, &c.name, time);
                (PUSH, xml, title)
            } else {
                let chars = rng.spread(TEXT_CHARS).max(1.0) as usize;
                let text = world.words.text_plain(&mut rng, chars);
                (1, text.clone(), text)
            };
            let compress = local_type != 1 || content.len() > ZSTD_ABOVE;
            let (content, compressed) = zstd_or_plain(content, compress)?;
            shard.insert(
                world,
                &table,
                &Row {
                    local_id: i as i64 + 1,
                    server_id: (rng.next_u64() >> 1) as i64,
                    local_type,
                    sort_seq: Some(time * 1000),
                    sender: Some(account),
                    time,
                    status: 3,
                    source: Some(Vec::new()),
                    source_compressed: false,
                    content,
                    compressed,
                },
            )?;
            *written.by_type.entry(local_type).or_default() += 1;
            newest = Some(Newest {
                time,
                local_type,
                summary,
            });
        }
        if let Some(newest) = newest {
            written.newest.insert(ci, (n, newest));
        }
    }
    shard.conn.execute_batch("commit")?;
    let plain = plain_image(&shard.conn);
    let rel = "message/biz_message_0.db";
    let db = f.with_cipher(rel, |cipher| encrypt_db(&plain, cipher));
    std::fs::write(f.db_storage().join(rel), db)?;
    Ok(written)
}

/// 一篇文章的推送：大小接近抽到的值的 XML，以及它的标题。
fn push(world: &World, rng: &mut Rng, account: &str, name: &str, time: i64) -> (String, String) {
    let (title_chars, digest_chars) = (8 + rng.below(16), 20 + rng.below(60));
    let title = format!(
        "{}{}",
        rng.pick(TOPICS),
        world.words.text_plain(rng, title_chars)
    );
    let digest = world.words.text_plain(rng, digest_chars);
    let key = rng.hex(32);
    let target = rng.spread(PUSH_BYTES);
    let (title_x, digest_x, name_x) = (esc(&title), esc(&digest), esc(name));
    let xml = sized(rng, target, |fill| {
        let url =
            format!("http://mp.weixin.qq.com/s?__biz={key}&amp;mid={time}&amp;idx=1&amp;sn={fill}");
        format!(
            "<msg>\n<appmsg appid=\"\" sdkver=\"0\">\n<title><![CDATA[{title_x}]]></title>\n<des><![CDATA[{digest_x}]]></des>\n\
             <type>5</type>\n<url><![CDATA[{url}]]></url>\n<thumburl><![CDATA[http://mmbiz.qpic.cn/mmbiz_jpg/{key}/0]]></thumburl>\n\
             <mmreader>\n<category type=\"20\" count=\"1\">\n<name><![CDATA[{name_x}]]></name>\n<topnew>\n\
             <cover><![CDATA[http://mmbiz.qpic.cn/mmbiz_jpg/{key}/640]]></cover>\n<width>0</width>\n<height>0</height>\n\
             <digest><![CDATA[{digest_x}]]></digest>\n</topnew>\n<item>\n<itemshowtype>0</itemshowtype>\n\
             <title><![CDATA[{title_x}]]></title>\n<url><![CDATA[{url}]]></url>\n\
             <cover><![CDATA[http://mmbiz.qpic.cn/mmbiz_jpg/{key}/640]]></cover>\n<digest><![CDATA[{digest_x}]]></digest>\n\
             <pub_time>{time}</pub_time>\n</item>\n</category>\n<publisher>\n<username><![CDATA[{account}]]></username>\n\
             <nickname><![CDATA[{name_x}]]></nickname>\n</publisher>\n</mmreader>\n</appmsg>\n\
             <fromusername><![CDATA[{account}]]></fromusername>\n</msg>"
        )
    });
    (xml, title)
}

/// 一条微信支付通知（数值是编的）和它的标题：序号为偶数的是带顶行的支付凭证卡片，其余的是几行带标签的文字。
fn notice(rng: &mut Rng, time: i64, i: usize) -> (String, String) {
    const MERCHANTS: &[&str] = &[
        "街角便利店",
        "楼下面馆",
        "城市地铁",
        "周末书店",
        "山野户外用品",
    ];
    const METHODS: &[&str] = &["零钱", "零钱通", "信用卡"];
    let amount = format!("{}.{:02}", 1 + rng.below(400), rng.below(100));
    let merchant = *rng.pick(MERCHANTS);
    let method = *rng.pick(METHODS);
    if i.is_multiple_of(2) {
        let title = format!("已支付¥{amount}");
        let xml = format!(
            "<msg><appmsg appid=\"\" sdkver=\"0\"><title><![CDATA[{title}]]></title>\
             <des><![CDATA[使用{method}支付¥{amount}]]></des><type>5</type><mmreader>\
             <template_header><title><![CDATA[微信支付凭证]]></title><pub_time>{time}</pub_time>\
             <display_name><![CDATA[{merchant}]]></display_name></template_header>\
             <template_detail><template_show_type>1</template_show_type><line_content><topline>\
             <key><word><![CDATA[使用{method}支付]]></word><color><![CDATA[#888888]]></color></key>\
             <value><word><![CDATA[¥{amount}]]></word></value></topline></line_content></template_detail>\
             </mmreader></appmsg><fromusername><![CDATA[{WECHAT_PAY}]]></fromusername></msg>"
        );
        (xml, title)
    } else {
        let title = "微信支付凭证".to_string();
        let xml = format!(
            "<msg><appmsg appid=\"\" sdkver=\"0\"><title>{title}</title>\
             <des><![CDATA[付款金额¥{amount}\n支付方式{method}\n收款方{merchant}\n交易状态支付成功]]></des>\
             <type>5</type></appmsg><fromusername>{WECHAT_PAY}</fromusername></msg>"
        );
        (xml, title)
    }
}
