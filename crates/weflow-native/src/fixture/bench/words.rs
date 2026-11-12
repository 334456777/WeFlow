//! Words: the text, names and topics messages and contacts are made of.

use super::profile::TextProfile;
use super::rng::{draw_range, Rng};

const HAN: &str = "的一是了我不人在他有这个上们来到时大地为子中你说生年着就那和要她出也得里后自以会家可下而过天去能对小多然于心学么之都好看起发当没成只如事把还用第样道想作种开美总从无情己面最女但现前些所同日手又行意动方期它头经长儿回位分爱老因很给名法间知世什两次使身者被高已亲其进此话常与活正感见明问力理点文几定本公特做外孩相西果走将月十实向声车全信重三机工物气每并别真打太新比才便夫再书部水像眼等体却加电主界门利海受听表少代员许先口由死安写性马光白或住难望教命花结乐色更拉东神记处让母父应直字场平报友关放至张认接告入笑内英军候民岁往何度山觉路带万男边风解叫任金快原吃妈变通师立象数四失满战远格士音轻目条呢病始达深完今提求清王化空业思切怎非找片罗钱吗语元喜曾离飞科言干流欢约各即指合反题必该论交终林请医晚制球决传画保读运及则房早院量苦火布品近坐产答星精视五连司巴奇管类未朋且婚台夜青北队久乎越观落尽形影红爸百令周吧识步希亚术留市半热送兴造谈容极随演收首根讲整式取照办强石古华拿计您装似足双妻转诉米称丽客南领节衣站黑刻统断福城故历惊脸选包紧争另建维绝树系伤示愿持千史谁准联妇纪基买志静阿诗独复痛消社算义竟确酒需单治卡幸兰念举仅钟怕共毛句息功官待究跟穿室易游程号居考突皮哪费倒价图具刚脑永歌响商礼细专黄块脚味灵改据般破引食仍存众注笔甚某沉血备习校默务土微娘须试怀料调广苏显赛查密议底列富梦错座参八除跑亮假印设线温虽掉京初养香停际致阳纸李纳验助激够严证帝饭忘趣支春集丈木研班普导顿睡展跳获艺六波察群";
pub(super) const PHRASES: &[&str] = &[
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
pub(super) const LATIN_NICKS: &[&str] = &[
    "Alex", "Momo", "Leo", "Coco", "Kevin", "Lily", "Jason", "Una", "Tom", "Echo", "Mia", "Sam",
];
pub(super) const TOPICS: &[&str] = &[
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
pub(super) const FILE_EXTENSIONS: &[&str] =
    &["pdf", "docx", "xlsx", "zip", "pptx", "txt", "mp3", "rar"];

fn chars_of(pool: &str) -> Vec<char> {
    pool.chars().collect()
}

pub(super) struct Words {
    han: Vec<char>,
    surnames: Vec<char>,
    given: Vec<char>,
    profile: TextProfile,
}

impl Words {
    pub(super) fn new(profile: TextProfile) -> Self {
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
    pub(super) fn text_plain(&self, rng: &mut Rng, chars: usize) -> String {
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
    pub(super) fn text_between(&self, rng: &mut Rng, lo: usize, hi: usize) -> String {
        let n = lo + rng.below(hi - lo);
        self.text_plain(rng, n)
    }

    /// A message's text: its length drawn from the real lengths.
    pub(super) fn message(&self, rng: &mut Rng) -> String {
        let chars = draw_range(rng, &self.profile.chars) as usize;
        self.text(rng, chars.max(1))
    }

    pub(super) fn person_name(&self, rng: &mut Rng) -> String {
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

    pub(super) fn group_name(&self, rng: &mut Rng) -> String {
        let mut name = format!("{}{}", rng.pick(TOPICS), rng.pick(GROUP_SUFFIXES));
        if rng.chance(0.15) {
            name.push_str(rng.pick(EMOJI));
        }
        name
    }
}

/// Escapes text for XML.
pub(super) fn esc(s: &str) -> String {
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
pub(super) fn sized(rng: &mut Rng, target: f64, template: impl Fn(&str) -> String) -> String {
    let base = template("").len();
    let fill = rng.filler((target as usize).saturating_sub(base));
    template(&fill)
}
