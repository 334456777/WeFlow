//! People and conversations: who the account talks with, when, and in which message database.

use std::collections::HashSet;

use super::profile::TimeProfile;
use super::rng::{Rng, Weighted};
use super::words::{Words, LATIN_NICKS, TOPICS};
use super::{BenchSpec, DAY, END, END_DAY, OWNER, TZ};

/// The share of the messages in the first message database; the others share the rest.
const FIRST_SHARD: f64 = 0.15;
/// Messages this close to where one database ends and the next starts go to either.
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
    /// People who may write in it (not the owner): the partner of a private chat, a group's members by how much
    /// they say (the first `speakers` say something).
    pub(super) members: Vec<usize>,
    pub(super) speakers: usize,
    /// Message times, oldest first.
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
