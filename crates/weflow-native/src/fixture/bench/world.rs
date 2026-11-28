//! 人和会话：账号与谁聊天、什么时候聊，以及消息在哪个消息库里。

use std::collections::HashSet;

use super::official::WECHAT_PAY;
use super::profile::TimeProfile;
use super::rng::{Rng, Weighted};
use super::words::{Words, LATIN_NICKS, TOPICS};
use super::{BenchSpec, DAY, END, END_DAY, OWNER, TZ};

/// 第一个消息库中的消息所占的比例；其余的由其他消息库分摊。
const FIRST_SHARD: f64 = 0.15;
/// 离一个消息库的结束处（也是下一个的开始处）这么近的消息，可能分到两个消息库中的任一个。
const SHARD_OVERLAP: i64 = 20 * DAY;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Kind {
    Private,
    Group,
    Official,
}

impl Kind {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Group => "group",
            Self::Official => "official",
        }
    }
}

pub(super) struct Person {
    pub(super) username: String,
    pub(super) nick: String,
    pub(super) remark: String,
    pub(super) alias: String,
    pub(super) avatar: String,
    pub(super) local_type: i64,
}

pub(super) struct Conversation {
    pub(super) username: String,
    pub(super) name: String,
    pub(super) kind: Kind,
    /// 可能在其中发言的人（不含账号所有者）：私聊的对方，或者按发言多少排列的群成员（前 `speakers` 个会发言）。
    pub(super) members: Vec<usize>,
    pub(super) speakers: usize,
    /// 各条消息的时间，最早的在前。
    pub(super) times: Vec<i64>,
}

pub(super) struct World {
    pub(super) people: Vec<Person>,
    pub(super) conversations: Vec<Conversation>,
    pub(super) words: Words,
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
        // 旧式的自定义 ID：一个字母开头，之后是字母、数字、`_` 和 `-`
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

/// 一个有 `n` 条消息的会话在 `first_day` 和 `last_day`（本地日期）之间的各条消息的时间：
/// 一阵阵相隔几秒的消息，日期和时段按真实账号的分布抽取。
fn timeline(
    rng: &mut Rng,
    n: usize,
    (first_day, last_day): (i64, i64),
    burst: f64,
    profile: &TimeProfile,
) -> Vec<i64> {
    let days: Vec<i64> = (first_day..=last_day).collect();
    // 少数忙碌的日子承载了一个会话的大部分消息，与真实账号一致
    let day_weights = Weighted::new(days.iter().map(|d| {
        let pareto = (1.0 - rng.unit()).powf(-1.0 / 1.3).min(60.0);
        profile.weekdays[(d + 3).rem_euclid(7) as usize] * pareto
    }));
    let hours = Weighted::new(profile.hours);
    let gaps = Weighted::new(profile.burst_gaps.iter().map(|g| g.2));
    let mut times = Vec::with_capacity(n);
    while times.len() < n {
        // 一阵消息的条数服从以 `burst` 为均值的几何分布
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
            // 进入较冷清的时段时，会话继续下去的概率按那个时段冷清的程度降低
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
    pub(super) fn new(rng: &mut Rng, spec: &BenchSpec) -> Self {
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
        // 真实账号上好友约为私聊的四倍；私聊的对象是其中的一部分
        let friends: Vec<usize> = (0..(privates * 4).max(profile.groups.big_members / 3))
            .map(|_| person(rng, 1))
            .collect();
        // 真实账号上也有的几个其他类型的联系人
        for _ in 0..(friends.len() / 40).max(1) {
            person(rng, 0);
        }
        person(rng, 4);

        let mut conversations = Vec::new();
        let mut room_names = HashSet::new();
        let mut group = |rng: &mut Rng, members: usize, speakers: usize, messages, days, burst| {
            let mut pool: Vec<usize> = Vec::new();
            while pool.len() < members {
                // 群里既有好友，也有账号所有者只在群里认识的人
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
        // 公众号和微信支付：出现在会话列表中，时间是它们最新一条消息的时间，
        // 这些消息由 `official` 写在另一个数据库里
        for i in 0..=(spec.conversations / 20).max(3) {
            let (username, name) = if i == 0 {
                taken.insert(WECHAT_PAY.to_string());
                (WECHAT_PAY.to_string(), "微信支付".to_string())
            } else {
                let username = unique(rng, &mut taken, |r| format!("gh_{}", r.hex(12)));
                let name = format!(
                    "{}{}",
                    rng.pick(TOPICS),
                    rng.pick(&["服务号", "订阅号", "助手"])
                );
                (username, name)
            };
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

/// 每个会话的每条消息所在的消息库：按时间分，账号最早的 [`FIRST_SHARD`] 比例的消息在第一个消息库，
/// 其余的由其他消息库分摊，靠近边界的消息分到边界两边的任一个。
pub(super) fn assign_shards(
    rng: &mut Rng,
    conversations: &[Conversation],
    shards: usize,
) -> Vec<Vec<usize>> {
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
