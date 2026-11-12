//! The synthetic account: its conversations, messages, images and Moments, and the WeFlow home pointing at it.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use anyhow::{ensure, Result};
use serde_json::{json, Value};
use weflow_native::fixture::{
    ContactSpec, Fixture, FtsSpec, MsgSpec, RoomSpec, SessionSpec, SnsPostSpec, T0,
};

use super::args::{leak, Args};
use super::{
    IMAGE_EVERY, IMAGE_FILES_FOR_LATEST, MEMBERS, OWNER, PRIVATE_MESSAGES, PUSHED_AT, ROOM, SHARDS,
    SNS_POSTS, STEP, UNINDEXED, XOR_KEY,
};

/// Text of the `i`-th message: 8 to 117 characters of a small vocabulary.
pub(crate) fn text(i: i64) -> String {
    let r = ((i.wrapping_mul(2_654_435_761) >> 7) & 0xffff) as usize;
    let words: Vec<char> =
        "今天我们一起去吃饭吧好的没问题明天见哈哈哈这个文件看一下 hello world ok 收到 谢谢"
            .chars()
            .collect();
    (0..8 + r % 110)
        .map(|k| words[(k * 7 + r) % words.len()])
        .collect()
}

fn image_md5(i: i64) -> String {
    format!("{i:032x}")
}

/// A JPEG-looking file of `len` bytes, XOR-ed with the image key as an old `.dat` is.
pub(crate) fn image_dat(len: usize, seed: i64) -> Vec<u8> {
    let mut plain = vec![0xFF, 0xD8, 0xFF, 0xE0];
    plain.extend((0..len - 4).map(|k| (k as u8).wrapping_mul(7).wrapping_add(seed as u8) | 1));
    plain.iter().map(|b| b ^ XOR_KEY).collect()
}

fn group_message(i: i64, members: &[&'static str]) -> MsgSpec {
    let sender = if i % 9 == 0 {
        "wxid_me"
    } else {
        members[(i * 31) as usize % members.len()]
    };
    let at = T0 + i * STEP;
    if i % IMAGE_EVERY == 0 {
        let xml = format!(
            "<msg><img md5=\"{}\" length=\"81920\" cdnthumblength=\"6144\" /></msg>",
            image_md5(i)
        );
        return MsgSpec::text(i, sender, at, &xml).of_type(3);
    }
    MsgSpec::text(i, sender, at, &text(i))
}

/// The conversation list: the group, then the private chats ten minutes apart. With `pushed`, that private chat
/// has a new message ([`PUSHED_AT`]).
pub(crate) fn session_specs(args: &Args, pushed: Option<usize>) -> Vec<SessionSpec<'static>> {
    let last = T0 + args.messages * STEP;
    let mut sessions = vec![SessionSpec {
        username: ROOM,
        summary: "",
        last_timestamp: last,
        unread: 3,
        last_msg_type: 1,
    }];
    sessions.extend((0..args.sessions).map(|k| {
        let new = pushed == Some(k);
        SessionSpec {
            username: leak(format!("wxid_f{k}")),
            summary: "",
            last_timestamp: if new {
                last - PUSHED_AT
            } else {
                last - (k as i64 + 1) * 600
            },
            unread: new as i64,
            last_msg_type: 1,
        }
    }));
    sessions
}

/// The message database the push check adds, and removes once the server has stopped.
pub(crate) fn pushed_shard(data: &Path) -> PathBuf {
    data.join(OWNER)
        .join(format!("db_storage/message/message_{SHARDS}.db"))
}

fn build(data: &Path, args: &Args) -> Result<Fixture> {
    let f = Fixture::new(data, OWNER);
    let friends: Vec<&'static str> = (0..args.sessions)
        .map(|k| leak(format!("wxid_f{k}")))
        .collect();
    let mut contacts: Vec<ContactSpec> = friends
        .iter()
        .enumerate()
        .map(|(k, u)| ContactSpec::new(u, 1, leak(format!("好友{k}"))))
        .collect();
    contacts.extend(
        (0..args.contacts.saturating_sub(args.sessions))
            .map(|k| ContactSpec::new(leak(format!("wxid_c{k}")), 1, leak(format!("联系人{k}")))),
    );
    contacts.push(ContactSpec::new("wxid_me", 1, "Me"));
    contacts.push(ContactSpec::new(ROOM, 2, "Bench group"));
    let members: Vec<&'static str> = friends.iter().copied().take(MEMBERS).collect();
    let nicks: Vec<(&'static str, &'static str)> = members
        .iter()
        .enumerate()
        .map(|(k, m)| {
            (
                *m,
                if k % 3 == 0 {
                    leak(format!("群昵称{k}"))
                } else {
                    ""
                },
            )
        })
        .collect();
    f.contact_db(
        &contacts,
        &[RoomSpec {
            username: ROOM,
            owner: members[0],
            members: Box::leak(nicks.into_boxed_slice()),
        }],
    );

    let last = T0 + args.messages * STEP;
    f.session_db(&session_specs(args, None));

    let per = args.messages / SHARDS;
    for shard in 0..SHARDS {
        let first = shard * per + 1;
        let end = if shard == SHARDS - 1 {
            args.messages
        } else {
            (shard + 1) * per
        };
        let mut convs: Vec<(&str, Vec<MsgSpec>)> = vec![(
            ROOM,
            (first..=end).map(|i| group_message(i, &members)).collect(),
        )];
        if shard == SHARDS - 1 {
            for (k, u) in friends.iter().enumerate() {
                let newest = last - (k as i64 + 1) * 600;
                let msgs = (1..=PRIVATE_MESSAGES)
                    .map(|j| {
                        let at = newest - (PRIVATE_MESSAGES - j) * 60;
                        let sender = if j % 2 == 0 { "wxid_me" } else { u };
                        MsgSpec::text(j, sender, at, &text(j + k as i64))
                    })
                    .collect();
                convs.push((u, msgs));
            }
        }
        f.message_shard(shard as u32, &convs);
    }
    // WeChat's own index of the group, which has not caught up with its newest messages
    let indexed: Vec<FtsSpec> = (1..=args.messages - UNINDEXED)
        .filter(|i| i % IMAGE_EVERY != 0)
        .map(|i| {
            let m = group_message(i, &members);
            FtsSpec {
                text: m.content,
                local_id: i,
                sort_seq: m.create_time * 1000,
                local_type: 1,
                session: ROOM,
                create_time: m.create_time,
            }
        })
        .collect();
    f.message_fts_db(&[indexed]);
    // the friends' moments, newest first by tid as WeChat keeps them
    let posts: Vec<SnsPostSpec> = (0..SNS_POSTS)
        .map(|k| SnsPostSpec {
            tid: SNS_POSTS - k,
            user: friends[k as usize % friends.len()],
            create_time: last - k * 3600,
            desc: leak(text(k)),
            kind: 1,
            media: (k % 4) as usize,
            likes: &[],
            extra: "",
        })
        .collect();
    f.sns_db(&posts);

    let dir = f
        .account_dir
        .join("msg/attach")
        .join(weflow_core::image::session_dir_for_storage(ROOM));
    let from = (args.messages - IMAGE_FILES_FOR_LATEST).max(1);
    for i in (from..=args.messages).filter(|i| i % IMAGE_EVERY == 0) {
        let month = weflow_core::image::year_month_from_create_time(Some(T0 + i * STEP));
        let img = dir.join(month).join("Img");
        std::fs::create_dir_all(&img)?;
        let md5 = image_md5(i);
        std::fs::write(img.join(format!("{md5}.dat")), image_dat(81_920, i))?;
        std::fs::write(img.join(format!("{md5}_t.dat")), image_dat(6_144, i))?;
    }
    Ok(f)
}

pub(crate) fn weflow_command(weflow: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(weflow);
    cmd.env("WEFLOW_HOME", home)
        .env("WEFLOW_LANG", "en")
        .env_remove("WEFLOW_HTTP_TOKEN")
        .stdin(Stdio::null());
    cmd
}

/// The WeFlow home of the synthetic account, built when missing or of another size.
pub(crate) fn synthetic_home(weflow: &Path, args: &Args) -> Result<PathBuf> {
    let data = args.work.join("data");
    let home = args.work.join("home");
    let size = json!({
        "messages": args.messages,
        "sessions": args.sessions,
        "contacts": args.contacts,
        "layout": 3,
    });
    let stamp = args.work.join("data.json");
    let built = std::fs::read(&stamp)
        .ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    // a pushed message left by a run that stopped early changes the answers: build again
    if built.as_ref() != Some(&size) || !data.exists() || pushed_shard(&data).exists() {
        let start = Instant::now();
        let _ = std::fs::remove_file(&stamp);
        let f = build(&data, args)?;
        let _ = std::fs::remove_dir_all(&home);
        for (key, value) in [
            ("db_path", f.account_dir.to_string_lossy().to_string()),
            ("wxid", OWNER.to_string()),
            ("decrypt_key", f.key_hex()),
            ("image_xor_key", XOR_KEY.to_string()),
        ] {
            let out = weflow_command(weflow, &home)
                .args(["config", "set", key, &value])
                .output()?;
            ensure!(
                out.status.success(),
                "config set {key} failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        std::fs::write(&stamp, serde_json::to_vec(&size)?)?;
        eprintln!(
            "built the synthetic account in {:.1} s",
            start.elapsed().as_secs_f64()
        );
    }
    Ok(home)
}
