//! Benchmark of the command API (`weflow serve`) on two accounts, both required: a synthetic one, whose content is
//! known, checks that the answers are right; a real one measures how fast they come.
//!
//! usage: serve_bench <work dir> --home <WEFLOW_HOME> [--weflow PATH] [--runs N] [--messages N] [--sessions N]
//!                    [--contacts N] [--keyword K] [--save FILE] [--compare FILE] [--tolerance X]
//!                    [-- <more `weflow serve` arguments>]
//!
//! Build both programs in the same profile, then run the example:
//! `cargo build --release -p weflow-cli --bin weflow --example serve_bench`, then
//! `target/release/examples/serve_bench <work dir> --home <WEFLOW_HOME>`. The `weflow` one folder above the example
//! is started unless `--weflow` names another.
//!
//! The synthetic account: `<work dir>/data` (rebuilt only when a size option changes) holds one group with
//! `--messages` messages (default 200000) over 4 message databases, every 20th an image whose `.dat` files exist for
//! the latest 1000 messages and all but the newest 50 in WeChat's own search index, `--sessions` private conversations
//! (default 500) of 30 messages each, `--contacts` contacts in all (default 3000) and 2000 Moments posts;
//! `<work dir>/home` is the WeFlow home pointing at it. Each read is made through both
//! routes, the child process one (`/chat/...`) and the in-process one (`/api/chat/...`), and its answer checked: the
//! same from both routes, and the conversations, counts, pages, search hits, group members and image bytes the
//! account holds. Last, a message is added to a private chat while `/api/push/messages` is listened to: it must
//! arrive once (the time it took is printed); it is taken out again when the server has stopped. Any failed check
//! makes the run fail.
//!
//! The real account is the one of `--home`, used as it is: point it at a copy of a real account's configuration. Its
//! conversations are picked by message count (the biggest of the 60 latest, a group, a private chat) and only
//! aggregate numbers are printed, never a conversation id, a name or a message. Each read is timed `--runs` times
//! (default 10) through both routes; the table shows the median and the 95th percentile in milliseconds as the client
//! sees them (connection included) and the size of the answer. A last row gives the server's CPU time per second
//! while `/api/push/messages` is listened to (the engine looks for new messages every second), next to the idle
//! one. Both accounts also time the reads that had no `/api/` route (issue #175), in a table of their own on the
//! synthetic one. `--save` writes the numbers to a JSON file;
//! `--compare` reads such a file, adds a column with the ratio to it and fails when a median is more than
//! `--tolerance` times (default 2) and more than 20 ms slower.
use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde_json::{json, Value};
use weflow_native::fixture::{
    ContactSpec, Fixture, FtsSpec, MsgSpec, RoomSpec, SessionSpec, SnsPostSpec, T0,
};

const TOKEN: &str = "serve-bench";
const OWNER: &str = "wxid_me_ab12";
const ROOM: &str = "room_bench@chatroom";
const SHARDS: i64 = 4;
/// Seconds between two messages of the group.
const STEP: i64 = 7;
const IMAGE_EVERY: i64 = 20;
const IMAGE_FILES_FOR_LATEST: i64 = 1000;
const PRIVATE_MESSAGES: i64 = 30;
const MEMBERS: usize = 200;
const XOR_KEY: u8 = 0x5a;
const PAGE: i64 = 50;
const AT_ONCE: i64 = 16;
const SCREEN_OF_IMAGES: usize = 20;
/// Video messages tried for the video reads, and the part of a video a player asks for first.
const VIDEOS: usize = 10;
const VIDEO_PART: usize = 1 << 20;
/// A slower median is only a regression when it is also slower by this much.
const NOISE_MS: f64 = 20.0;
/// The pushed message is this many seconds older than the group's latest one, so the group stays first.
const PUSHED_AT: i64 = 1;
/// A keyword no message holds, so a search reads every message of the conversation.
const ABSENT: &str = "qzxwvk";
/// The newest messages of the group that WeChat's index does not hold yet, so a search scans them.
const UNINDEXED: i64 = 50;
/// The reads of [`Bench::candidates`] given an `/api/` route since, by the start of their path.
const IN_API: &[&str] = &[
    "/chat/dates?",
    "/chat/date-counts?",
    "/chat/detail?",
    "/sns/",
];
/// Moments of the synthetic account, from its friends in turn.
const SNS_POSTS: i64 = 2000;
/// How long the push engine's CPU time is measured on the real account, idle and with a listener.
/// The columns a chat window shows of a message, and a list of a contact (`?fields=`).
const MESSAGE_FIELDS: &str =
    "local_id,server_id,local_type,sort_seq,create_time,sender_username,is_send,message_content,_cursor";
const CONTACT_FIELDS: &str = "username,alias,remark,nick_name,small_head_url,local_type";
const PUSH_WINDOW: Duration = Duration::from_secs(10);

struct Args {
    work: PathBuf,
    weflow: Option<PathBuf>,
    /// The WeFlow home of the real account.
    home: PathBuf,
    runs: usize,
    messages: i64,
    sessions: usize,
    contacts: usize,
    keyword: String,
    save: Option<PathBuf>,
    compare: Option<PathBuf>,
    tolerance: f64,
    serve: Vec<String>,
}

fn parse_args() -> Result<Args> {
    let mut it = std::env::args().skip(1);
    const USAGE: &str = "usage: serve_bench <work dir> --home <WEFLOW_HOME> [--weflow PATH] [--runs N] [--messages N] [--sessions N] [--contacts N] [--keyword K] [--save FILE] [--compare FILE] [--tolerance X] [-- <serve arguments>]";
    let work = it
        .next()
        .filter(|a| !a.starts_with('-'))
        .ok_or_else(|| anyhow!(USAGE))?;
    let mut home = None;
    let mut args = Args {
        work: work.into(),
        weflow: None,
        home: PathBuf::new(),
        runs: 10,
        messages: 200_000,
        sessions: 500,
        contacts: 3000,
        keyword: "好".into(),
        save: None,
        compare: None,
        tolerance: 2.0,
        serve: Vec::new(),
    };
    while let Some(flag) = it.next() {
        if flag == "--" {
            args.serve = it.by_ref().collect();
            break;
        }
        let value = it.next().ok_or_else(|| anyhow!("{flag} needs a value"))?;
        match flag.as_str() {
            "--weflow" => args.weflow = Some(value.into()),
            "--home" => home = Some(PathBuf::from(value)),
            "--runs" => args.runs = value.parse()?,
            "--messages" => args.messages = value.parse()?,
            "--sessions" => args.sessions = value.parse()?,
            "--contacts" => args.contacts = value.parse()?,
            "--keyword" => args.keyword = value,
            "--save" => args.save = Some(value.into()),
            "--compare" => args.compare = Some(value.into()),
            "--tolerance" => args.tolerance = value.parse()?,
            _ => bail!("unknown option {flag}"),
        }
    }
    args.home = home.ok_or_else(|| {
        anyhow!("--home is required: the real account measures the speed, the synthetic one only checks the answers\n{USAGE}")
    })?;
    ensure!(args.runs > 0, "--runs must be at least 1");
    ensure!(
        args.messages >= SHARDS * PAGE * 4,
        "--messages must be at least {}",
        SHARDS * PAGE * 4
    );
    ensure!(args.sessions > 0, "--sessions must be at least 1");
    Ok(args)
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

// ---- the synthetic account ----

/// Text of the `i`-th message: 8 to 117 characters of a small vocabulary.
fn text(i: i64) -> String {
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
fn image_dat(len: usize, seed: i64) -> Vec<u8> {
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
fn session_specs(args: &Args, pushed: Option<usize>) -> Vec<SessionSpec> {
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
fn pushed_shard(data: &Path) -> PathBuf {
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
        .join(weflow_core::image::session_dir_for_storage(ROOM, |s| {
            s.to_string()
        }));
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

fn weflow_command(weflow: &Path, home: &Path) -> Command {
    let mut cmd = Command::new(weflow);
    cmd.env("WEFLOW_HOME", home)
        .env("WEFLOW_LANG", "en")
        .env_remove("WEFLOW_HTTP_TOKEN")
        .stdin(Stdio::null());
    cmd
}

/// The WeFlow home of the synthetic account, built when missing or of another size.
fn synthetic_home(weflow: &Path, args: &Args) -> Result<PathBuf> {
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

// ---- the checks on the synthetic account ----

/// The keyword of the checked search: a word of [`text`].
const CHECKED_KEYWORD: &str = "好";

struct Checks<'a> {
    addr: &'a str,
    passed: usize,
    failed: Vec<String>,
    /// How long a new message took to reach a listener of `/api/push/messages`.
    pushed_ms: Option<f64>,
}

/// An open `GET /api/push/messages` and what it sent so far.
struct EventStream {
    stream: TcpStream,
    text: String,
}

impl EventStream {
    fn open(addr: &str) -> Result<Self> {
        let mut stream = TcpStream::connect(addr)?;
        stream.write_all(
            format!(
                "GET /api/push/messages HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {TOKEN}\r\n\r\n"
            )
            .as_bytes(),
        )?;
        stream.set_read_timeout(Some(Duration::from_millis(100)))?;
        Ok(Self {
            stream,
            text: String::new(),
        })
    }

    /// Reads until `needle` arrives (true) or `wait` passes (false).
    fn wait_for(&mut self, needle: &str, wait: Duration) -> bool {
        let deadline = Instant::now() + wait;
        let mut buf = [0u8; 4096];
        while !self.text.contains(needle) && Instant::now() < deadline {
            match self.stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => self.text.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => {}
            }
        }
        self.text.contains(needle)
    }
}

/// Adds a message to the last private chat while a listener waits for it; the time it took to arrive.
fn check_push(addr: &str, data: &Path, args: &Args) -> Result<f64> {
    let mut events = EventStream::open(addr)?;
    ensure!(
        events.wait_for("event: ready", Duration::from_secs(10)),
        "no `ready` event"
    );
    // the engine takes its baseline when the first listener comes
    std::thread::sleep(Duration::from_secs(2));
    let k = args.sessions - 1;
    let friend = leak(format!("wxid_f{k}"));
    let at = T0 + args.messages * STEP - PUSHED_AT;
    let f = Fixture::open(data, OWNER);
    let begin = Instant::now();
    f.message_shard(
        SHARDS as u32,
        &[(
            friend,
            vec![MsgSpec::text(
                PRIVATE_MESSAGES + 1,
                friend,
                at,
                "pushed now",
            )],
        )],
    );
    f.session_db(&session_specs(args, Some(k)));
    let wanted = format!("\"sessionId\":\"{friend}\"");
    ensure!(
        events.wait_for(&wanted, Duration::from_secs(10)),
        "no event for the new message within 10 s"
    );
    let ms = begin.elapsed().as_secs_f64() * 1000.0;
    ensure!(
        events.text.contains("event: message.new") && events.text.contains("pushed now"),
        "the event is not a `message.new` with the message"
    );
    // two more seconds: nothing else comes
    events.wait_for("\u{0}", Duration::from_secs(2));
    ensure!(
        events.text.matches("event: message.new").count() == 1,
        "the message was pushed more than once"
    );
    Ok(ms)
}

/// CPU time the process has used so far, in milliseconds.
fn cpu_ms(pid: u32) -> Option<f64> {
    if cfg!(target_os = "linux") {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
        let fields: Vec<&str> = stat.rsplit_once(')')?.1.split_whitespace().collect();
        // utime and stime, in clock ticks (100 a second)
        let ticks = fields.get(11)?.parse::<f64>().ok()? + fields.get(12)?.parse::<f64>().ok()?;
        Some(ticks * 10.0)
    } else if cfg!(windows) {
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {pid}).TotalProcessorTime.TotalMilliseconds"),
            ])
            .output()
            .ok()?;
        String::from_utf8_lossy(&out.stdout).trim().parse().ok()
    } else {
        // macOS: [[dd-]hh:]mm:ss.ss
        let out = Command::new("ps")
            .args(["-o", "time=", "-p", &pid.to_string()])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut seconds = 0.0;
        for part in text.trim().split(':') {
            seconds = seconds * 60.0 + part.parse::<f64>().ok()?;
        }
        Some(seconds * 1000.0)
    }
}

impl Checks<'_> {
    fn check(&mut self, name: &str, outcome: Result<()>) {
        match outcome {
            Ok(()) => self.passed += 1,
            Err(err) => self.failed.push(format!("{name}: {err:#}")),
        }
    }

    /// `target` through both routes: both answer 200 and the same document (but for the `volatile` fields of its
    /// `data`); returns that `data`.
    fn both(&self, target: &str, volatile: &[&str]) -> Result<Value> {
        let sub = request(self.addr, "GET", target, "")?;
        let api = request(self.addr, "GET", &format!("/api{target}"), "")?;
        ensure!(
            sub.status == 200 && api.status == 200,
            "status {}, and {} through /api/",
            sub.status,
            api.status
        );
        let same = if volatile.is_empty() {
            sub.body == api.body
        } else {
            let strip = |body: &[u8]| -> Result<Value> {
                let mut doc: Value = serde_json::from_slice(body)?;
                if let Some(data) = doc["data"].as_object_mut() {
                    for key in volatile {
                        data.remove(*key);
                    }
                }
                Ok(doc)
            };
            strip(&sub.body)? == strip(&api.body)?
        };
        ensure!(same, "the two routes answer differently");
        sub.data()
    }
}

fn ids(data: &Value) -> Vec<i64> {
    rows(data)
        .iter()
        .filter_map(|m| number(field(m, &["local_id", "localId"])))
        .collect()
}

/// The ids of a page, newest first: `count` of them from `from` down, as far as 1.
fn down(from: i64, count: i64) -> Vec<i64> {
    (0..count).map(|k| from - k).filter(|i| *i > 0).collect()
}

fn page_is(data: Value, expected: Vec<i64>) -> Result<()> {
    let got = ids(&data);
    ensure!(
        got == expected,
        "{} messages from {:?} to {:?}, expected {} from {:?} to {:?}",
        got.len(),
        got.first(),
        got.last(),
        expected.len(),
        expected.first(),
        expected.last()
    );
    Ok(())
}

/// Reads through both routes and checks their answers against what the synthetic account holds.
fn check_answers<'a>(addr: &'a str, data: &Path, args: &Args) -> Checks<'a> {
    let n = args.messages;
    let room = encode(ROOM);
    let mut c = Checks {
        addr,
        passed: 0,
        failed: Vec::new(),
        pushed_ms: None,
    };

    let r = c.both("/chat/sessions", &[]).and_then(|d| {
        let all = rows(&d);
        ensure!(
            all.len() == args.sessions + 1,
            "{} conversations, expected {}",
            all.len(),
            args.sessions + 1
        );
        let first = field(&all[0], &["username"]).and_then(Value::as_str);
        ensure!(
            first == Some(ROOM),
            "the group is not the latest conversation"
        );
        Ok(())
    });
    c.check("conversations, newest first", r);
    let r = c.both("/chat/sessions?limit=50", &[]).and_then(|d| {
        ensure!(rows(&d).len() == 50, "{} conversations", rows(&d).len());
        Ok(())
    });
    c.check("conversations, limit 50", r);

    let r = c
        .both(
            &format!("/chat/counts?sessions={room}&sessions=wxid_f0"),
            &[],
        )
        .and_then(|d| {
            ensure!(
                number(d.get(ROOM)) == Some(n)
                    && number(d.get("wxid_f0")) == Some(PRIVATE_MESSAGES),
                "counts {d}"
            );
            Ok(())
        });
    c.check("message counts", r);

    let page =
        |offset: i64| format!("/chat/messages?session-id={room}&limit={PAGE}&offset={offset}");
    let r = c
        .both(
            &format!("/chat/messages?session-id={room}&limit={PAGE}"),
            &[],
        )
        .and_then(|d| page_is(d, down(n, PAGE)));
    c.check("messages, first page", r);
    let r = c
        .both(&page(n / 2), &[])
        .and_then(|d| page_is(d, down(n - n / 2, PAGE)));
    c.check("messages, page in the middle", r);
    let r = c
        .both(&page(n - 10), &[])
        .and_then(|d| page_is(d, down(10, PAGE)));
    c.check("messages, last page (10 left)", r);
    let r = c.both(&page(n), &[]).and_then(|d| page_is(d, Vec::new()));
    c.check("messages, past the end", r);
    let r = c
        .both(&format!("/chat/latest?session-id={room}&limit=20"), &[])
        .and_then(|d| page_is(d, down(n, 20)));
    c.check("latest 20", r);

    // the cursor of message i: its sort key and its database
    let per = n / SHARDS;
    let cursor = |i: i64| {
        let at = T0 + i * STEP;
        format!("{}.{at}.{i}.{}", at * 1000, ((i - 1) / per).min(SHARDS - 1))
    };
    let r = c
        .both(&format!("/chat/messages?session-id={room}&limit=5"), &[])
        .and_then(|d| {
            for m in rows(&d) {
                let i = number(field(&m, &["local_id"])).unwrap_or(0);
                ensure!(
                    m["_cursor"] == cursor(i).as_str(),
                    "message {i}: cursor {}",
                    m["_cursor"]
                );
            }
            Ok(())
        });
    c.check("messages carry their cursor", r);
    let from_cursor = |key: &str, i: i64| {
        format!(
            "/chat/messages?session-id={room}&limit={PAGE}&{key}={}",
            encode(&cursor(i))
        )
    };
    let r = c
        .both(&from_cursor("before", n - n / 2 + 1), &[])
        .and_then(|d| page_is(d, down(n - n / 2, PAGE)));
    c.check("messages before a cursor in the middle", r);
    let r = c
        .both(&from_cursor("before", per + 1), &[])
        .and_then(|d| page_is(d, down(per, PAGE)));
    c.check("messages before a cursor, into the previous database", r);
    let r = c
        .both(&from_cursor("after", n / 2), &[])
        .and_then(|d| page_is(d, down(n / 2 + PAGE, PAGE)));
    c.check("messages after a cursor", r);

    let id = n - 10;
    let at = T0 + id * STEP;
    let r = c
        .both(
            &format!("/chat/message?session-id={room}&local-id={id}&create-time={at}"),
            &[],
        )
        .and_then(|d| {
            let m = &d["message"];
            ensure!(
                number(m.get("localId")) == Some(id) && number(m.get("createTime")) == Some(at),
                "got message {} at {}",
                m["localId"],
                m["createTime"]
            );
            Ok(())
        });
    c.check("one message", r);

    let r = c.both("/chat/contact?username=wxid_f3", &[]).and_then(|d| {
        let nick = field(&d, &["nick_name", "nickName"]).and_then(Value::as_str);
        ensure!(nick == Some("好友3"), "nickname {nick:?}");
        Ok(())
    });
    c.check("one contact", r);
    let r = c.both("/chat/contacts", &[]).and_then(|d| {
        // the friends and the other contacts, `wxid_me` and the group
        let expected = args.contacts.max(args.sessions) + 2;
        ensure!(
            rows(&d).len() == expected,
            "{} contacts, expected {expected}",
            rows(&d).len()
        );
        Ok(())
    });
    c.check("contacts", r);

    let r = c
        .both(
            &format!(
                "/chat/search?keyword={}&session-id={room}&limit=20",
                encode(CHECKED_KEYWORD)
            ),
            &[],
        )
        .and_then(|d| {
            let hits = rows(&d);
            ensure!(!hits.is_empty() && hits.len() <= 20, "{} hits", hits.len());
            for m in &hits {
                let content = m["message_content"].as_str().unwrap_or("");
                ensure!(
                    content.contains(CHECKED_KEYWORD),
                    "a hit without the keyword"
                );
                ensure!(
                    m["_session_id"].as_str() == Some(ROOM),
                    "a hit of another conversation"
                );
            }
            // the newest messages with the keyword, none skipped
            let expected: Vec<i64> = (1..=n)
                .rev()
                .filter(|i| i % IMAGE_EVERY != 0 && text(*i).contains(CHECKED_KEYWORD))
                .take(20)
                .collect();
            page_is(d, expected)
        });
    c.check("search in the conversation", r);

    let r = c
        .both(
            &format!(
                "/chat/search?keyword={}&session-id={room}&limit=20",
                encode(ABSENT)
            ),
            &[],
        )
        .and_then(|d| page_is(d, Vec::new()));
    c.check("search for a word found nowhere", r);

    let r = c
        .both(
            &format!("/group/members?chatroom-id={room}"),
            &["updatedAt", "fromCache"],
        )
        .and_then(|d| {
            let expected = MEMBERS.min(args.sessions);
            ensure!(
                number(d.get("count")) == Some(expected as i64)
                    && d["members"].as_array().map(Vec::len) == Some(expected),
                "{} members, expected {expected}",
                d["count"]
            );
            Ok(())
        });
    c.check("group members", r);

    for prefix in ["", "/api"] {
        let r = (|| -> Result<()> {
            let pages: Vec<Result<Value>> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..AT_ONCE)
                    .map(|k| {
                        let target = format!("{prefix}{}", page(k * PAGE));
                        s.spawn(move || request(addr, "GET", &target, "")?.data())
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|_| Err(anyhow!("thread panicked"))))
                    .collect()
            });
            let mut got = Vec::new();
            for p in pages {
                got.extend(ids(&p?));
            }
            ensure!(
                got == down(n, AT_ONCE * PAGE),
                "the pages do not join up into the latest {} messages",
                AT_ONCE * PAGE
            );
            Ok(())
        })();
        c.check(
            &format!(
                "{AT_ONCE} pages at once through {}",
                if prefix.is_empty() {
                    "the child process"
                } else {
                    "/api/"
                }
            ),
            r,
        );
    }

    let latest_image = n - n % IMAGE_EVERY;
    let r = (|| -> Result<()> {
        for k in 0..5 {
            let i = latest_image - k * IMAGE_EVERY;
            let img = json!({ "session-id": ROOM, "msg-id": i.to_string(), "create-time": T0 + i * STEP });
            let plain: Vec<u8> = image_dat(81_920, i).iter().map(|b| b ^ XOR_KEY).collect();
            let child = image_request(addr, "sub", &img, false)?;
            ensure!(child.status == 200, "image {i}: status {}", child.status);
            ensure!(
                child.body.windows(plain.len()).any(|w| w == plain),
                "image {i}: the decrypted bytes are not in the answer"
            );
            // the /api/ route answers the file itself
            let own = image_request(addr, "api", &img, false)?;
            ensure!(
                own.status == 200 && own.body == plain,
                "image {i}: /api/ answers {} with {} bytes",
                own.status,
                own.body.len()
            );
        }
        Ok(())
    })();
    c.check("images, decrypted, through both routes", r);

    // last: it adds a message, removed again once the server has stopped
    let r = check_push(addr, data, args).map(|ms| c.pushed_ms = Some(ms));
    c.check("a new message is pushed once", r);
    c
}

/// The image of a message (`{"session-id", "msg-id", "create-time"}`), or its thumbnail, through one route: `POST`
/// with the file in the answer for the child process, `GET` answering the file itself for `/api/`.
fn image_request(addr: &str, route: &str, img: &Value, thumb: bool) -> Result<Reply> {
    if route == "api" {
        let mut query = Vec::new();
        for key in ["session-id", "msg-id", "create-time"] {
            match &img[key] {
                Value::String(s) => query.push(format!("{key}={}", encode(s))),
                Value::Number(n) => query.push(format!("{key}={n}")),
                _ => {}
            }
        }
        if thumb {
            query.push("thumb=true".into());
        }
        request(
            addr,
            "GET",
            &format!("/api/chat/image-data?{}", query.join("&")),
            "",
        )
    } else {
        let mut body = img.clone();
        if thumb {
            body["thumb"] = json!(true);
        }
        request(
            addr,
            "POST",
            "/chat/image-data?return=file",
            &body.to_string(),
        )
    }
}

// ---- the server and its requests ----

struct Server {
    child: Child,
    addr: String,
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Starts `weflow serve` on a free port; returns it with the milliseconds it took to listen.
fn start(weflow: &Path, home: &Path, extra: &[String]) -> Result<(Server, f64)> {
    let begin = Instant::now();
    let mut child = weflow_command(weflow, home)
        .args(["--json", "serve", "--port", "0", "--api-token", TOKEN])
        .args(extra)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("cannot start {}", weflow.display()))?;
    let mut lines = BufReader::new(child.stderr.take().unwrap()).lines();
    let started = loop {
        let line = lines
            .next()
            .ok_or_else(|| anyhow!("weflow serve exited before it started listening"))??;
        if let Ok(event) = serde_json::from_str::<Value>(&line) {
            if event["type"] == "server_started" {
                break event;
            }
        }
    };
    let ms = begin.elapsed().as_secs_f64() * 1000.0;
    // keep reading stderr so the server never blocks on a full pipe; nothing of it is printed
    std::thread::spawn(move || for _ in lines {});
    let addr = started["url"]
        .as_str()
        .ok_or_else(|| anyhow!("server_started without a url"))?
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    Ok((Server { child, addr }, ms))
}

struct Reply {
    status: u16,
    body: Vec<u8>,
    ms: f64,
    /// The size of the whole file, of a `206` answer.
    content_range_total: Option<usize>,
}

impl Reply {
    fn data(&self) -> Result<Value> {
        ensure!(self.status == 200, "status {}", self.status);
        let doc: Value = serde_json::from_slice(&self.body)?;
        Ok(doc["data"].clone())
    }
}

fn dechunk(mut raw: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    while let Some(end) = raw.windows(2).position(|w| w == b"\r\n") {
        let size = std::str::from_utf8(&raw[..end])
            .ok()
            .and_then(|s| usize::from_str_radix(s.trim(), 16).ok())
            .unwrap_or(0);
        if size == 0 || raw.len() < end + 2 + size {
            break;
        }
        out.extend_from_slice(&raw[end + 2..end + 2 + size]);
        raw = &raw[(end + 4 + size).min(raw.len())..];
    }
    out
}

fn request(addr: &str, method: &str, target: &str, body: &str) -> Result<Reply> {
    request_with(addr, method, target, body, "")
}

/// [`request`] with more header lines, each ending in CRLF; the body comes back as sent (compressed or not).
fn request_with(
    addr: &str,
    method: &str,
    target: &str,
    body: &str,
    headers: &str,
) -> Result<Reply> {
    let begin = Instant::now();
    let mut stream = TcpStream::connect(addr)?;
    let mut req = format!(
        "{method} {target} HTTP/1.1\r\nHost: {addr}\r\nAuthorization: Bearer {TOKEN}\r\nConnection: close\r\nContent-Length: {}\r\n{headers}",
        body.len()
    );
    if !body.is_empty() {
        req.push_str("Content-Type: application/json\r\n");
    }
    req.push_str("\r\n");
    req.push_str(body);
    stream.write_all(req.as_bytes())?;
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw)?;
    let ms = begin.elapsed().as_secs_f64() * 1000.0;
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| anyhow!("no HTTP head in the answer to {method}"))?;
    let head = String::from_utf8_lossy(&raw[..split]).to_ascii_lowercase();
    let status = head
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    let mut body = raw[split + 4..].to_vec();
    if head.contains("\r\ntransfer-encoding: chunked") {
        body = dechunk(&body);
    }
    let content_range_total = head
        .lines()
        .find_map(|l| l.strip_prefix("content-range: bytes "))
        .and_then(|r| r.rsplit_once('/'))
        .and_then(|(_, total)| total.trim().parse().ok());
    Ok(Reply {
        status,
        body,
        ms,
        content_range_total,
    })
}

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// The list in a command's `data`: the value itself or its first array field.
fn rows(data: &Value) -> Vec<Value> {
    match data {
        Value::Array(a) => a.clone(),
        Value::Object(o) => o
            .values()
            .find_map(|v| v.as_array().cloned())
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

fn field<'a>(row: &'a Value, names: &[&str]) -> Option<&'a Value> {
    names.iter().find_map(|n| row.get(*n))
}

fn number(v: Option<&Value>) -> Option<i64> {
    v.and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok()))
}

// ---- measuring ----

struct Row {
    op: String,
    route: &'static str,
    median: f64,
    p95: f64,
    bytes: Option<usize>,
    note: String,
}

fn median_p95(mut ms: Vec<f64>) -> (f64, f64) {
    ms.sort_by(f64::total_cmp);
    let n = ms.len();
    let median = if n % 2 == 1 {
        ms[n / 2]
    } else {
        (ms[n / 2 - 1] + ms[n / 2]) / 2.0
    };
    let p95 = ms[((n as f64 * 0.95).ceil() as usize).clamp(1, n) - 1];
    (median, p95)
}

/// The conversations the reads are about.
struct Targets {
    sessions: Vec<String>,
    big: String,
    big_messages: i64,
    group: Option<String>,
    private: Option<String>,
    /// (local id, create time): local ids repeat across message databases.
    one_message: Option<(i64, i64)>,
    /// The `_cursor` of the message above the page in the middle of the biggest conversation.
    middle_cursor: Option<String>,
    images: Vec<Value>,
    /// Video messages, whose files may or may not be on the computer.
    videos: Vec<Value>,
}

fn targets(addr: &str) -> Result<Targets> {
    let list = request(addr, "GET", "/api/chat/sessions?limit=60", "")?.data()?;
    let sessions: Vec<String> = rows(&list)
        .iter()
        .filter_map(|s| {
            field(s, &["username", "userName"])?
                .as_str()
                .map(String::from)
        })
        .collect();
    ensure!(!sessions.is_empty(), "the account has no conversations");
    let query: Vec<String> = sessions
        .iter()
        .map(|s| format!("sessions={}", encode(s)))
        .collect();
    let counts = request(
        addr,
        "GET",
        &format!("/api/chat/counts?{}", query.join("&")),
        "",
    )?
    .data()?;
    let counts = counts.get("counts").unwrap_or(&counts);
    let mut ranked: Vec<(i64, String)> = sessions
        .iter()
        .filter_map(|s| Some((number(counts.get(s))?, s.clone())))
        .collect();
    ranked.sort_by(|a, b| b.cmp(a));
    let (big_messages, big) = ranked
        .first()
        .cloned()
        .ok_or_else(|| anyhow!("no message counts"))?;
    let group = ranked
        .iter()
        .find(|(n, s)| *n > 0 && s.ends_with("@chatroom"))
        .map(|(_, s)| s.clone());
    let private = ranked
        .iter()
        .find(|(n, s)| *n > 0 && !s.ends_with("@chatroom") && !s.starts_with("gh_"))
        .map(|(_, s)| s.clone());

    let mut one_message = None;
    let mut images = Vec::new();
    let mut videos = Vec::new();
    for (_, sid) in ranked.iter().take(15) {
        let page = request(
            addr,
            "GET",
            &format!("/api/chat/messages?session-id={}&limit=300", encode(sid)),
            "",
        )?;
        let Ok(data) = page.data() else { continue };
        for m in rows(&data) {
            let id = number(field(&m, &["local_id", "localId"]));
            let at = number(field(&m, &["create_time", "createTime"]));
            if sid == &big && one_message.is_none() {
                one_message = id.zip(at);
            }
            let found = json!({ "session-id": sid, "msg-id": id.map(|i| i.to_string()), "create-time": at });
            match number(field(&m, &["local_type", "localType"])) {
                Some(3) if id.is_some() => images.push(found),
                Some(43) if id.is_some() => videos.push(found),
                _ => {}
            }
        }
        if images.len() >= SCREEN_OF_IMAGES && videos.len() >= VIDEOS {
            break;
        }
    }
    images.truncate(SCREEN_OF_IMAGES);
    videos.truncate(VIDEOS);
    // the cursor of the message just above the middle page, to ask for that page from it
    let middle_cursor = request(
        addr,
        "GET",
        &format!(
            "/api/chat/messages?session-id={}&limit=1&offset={}",
            encode(&big),
            (big_messages / 2 - 1).max(0)
        ),
        "",
    )?
    .data()
    .ok()
    .and_then(|d| d[0]["_cursor"].as_str().map(String::from));
    Ok(Targets {
        sessions,
        big,
        big_messages,
        group,
        private,
        one_message,
        images,
        videos,
        middle_cursor,
    })
}

struct Bench<'a> {
    addr: &'a str,
    runs: usize,
    rows: Vec<Row>,
}

impl Bench<'_> {
    /// Keeps a measured row and shows it on stderr while the others run.
    fn push(&mut self, row: Row) {
        eprintln!(
            "{:<45} {:<3} median {:>8.1} ms  {}",
            row.op, row.route, row.median, row.note
        );
        self.rows.push(row);
    }

    /// Times `runs` requests of one read through the given route.
    fn read(&mut self, op: &str, route: &'static str, target: &str) -> Result<()> {
        self.read_n(op, route, target, self.runs, "")
    }

    /// [`read`](Self::read) asking for a compressed answer: its size is the compressed one.
    fn read_encoded(
        &mut self,
        op: &str,
        route: &'static str,
        target: &str,
        encoding: &str,
    ) -> Result<()> {
        let header = format!("Accept-Encoding: {encoding}\r\n");
        self.read_n(op, route, target, self.runs, &header)
    }

    /// [`read`](Self::read) `runs` times, with more header lines.
    fn read_n(
        &mut self,
        op: &str,
        route: &'static str,
        target: &str,
        runs: usize,
        headers: &str,
    ) -> Result<()> {
        let path = if route == "api" {
            format!("/api{target}")
        } else {
            target.to_string()
        };
        let mut ms = Vec::with_capacity(runs);
        let mut bytes = 0;
        let mut statuses = BTreeMap::<u16, usize>::new();
        for _ in 0..runs {
            let r = request_with(self.addr, "GET", &path, "", headers)?;
            ms.push(r.ms);
            bytes = r.body.len();
            *statuses.entry(r.status).or_default() += 1;
        }
        let (median, p95) = median_p95(ms);
        let mut note = if statuses.keys().eq([200].iter()) {
            String::new()
        } else {
            format!("status {statuses:?}")
        };
        if runs == 1 {
            note = [note.as_str(), "once"].join(if note.is_empty() { "" } else { "; " });
        }
        self.push(Row {
            op: op.into(),
            route,
            median,
            p95,
            bytes: Some(bytes),
            note,
        });
        Ok(())
    }

    fn both(&mut self, op: &str, target: &str) -> Result<()> {
        self.read(op, "sub", target)?;
        self.read(op, "api", target)
    }

    /// The reads that had no `/api/` route (issue #175): through the child process route, to tell whether one is
    /// worth adding, and through both for those added.
    fn candidates(&mut self, t: &Targets) -> Result<()> {
        let big = encode(&t.big);
        let statuses: Vec<String> = t
            .sessions
            .iter()
            .map(|s| format!("usernames={}", encode(s)))
            .collect();
        let mut reads = vec![
            (
                "chat dates".to_string(),
                format!("/chat/dates?session-id={big}"),
            ),
            (
                "chat date-counts".into(),
                format!("/chat/date-counts?session-id={big}"),
            ),
            (
                format!("chat statuses of {} conversations", t.sessions.len()),
                format!("/chat/statuses?{}", statuses.join("&")),
            ),
            (
                "chat detail --fast".into(),
                format!("/chat/detail?session-id={big}&fast=true"),
            ),
            ("chat tab-counts".into(), "/chat/tab-counts".into()),
            (
                "chat resources, all conversations".into(),
                "/chat/resources".into(),
            ),
            (
                "chat resources, all conversations, far page".into(),
                "/chat/resources?offset=3000".into(),
            ),
            (
                "chat resources, one conversation".into(),
                format!("/chat/resources?session={big}"),
            ),
            ("group list".into(), "/group/list".into()),
            ("sns timeline, 20".into(), "/sns/timeline".into()),
            (
                "sns timeline, 20, far page".into(),
                "/sns/timeline?offset=1000".into(),
            ),
            ("sns users".into(), "/sns/users".into()),
            ("sns stats --fast".into(), "/sns/stats?fast=true".into()),
            (
                "sns post-counts --prefer-cache".into(),
                "/sns/post-counts?prefer-cache=true".into(),
            ),
        ];
        for kind in ["image", "video", "voice", "file"] {
            reads.push((
                format!("chat resources, all conversations, {kind}"),
                format!("/chat/resources?type={kind}"),
            ));
        }
        for (op, target) in &reads {
            if IN_API.iter().any(|p| target.starts_with(p)) {
                self.both(op, target)?;
            } else {
                self.read(op, "sub", target)?;
            }
        }
        // the aggregate of the statistics: computed once, then read from its cache
        self.read_n(
            "analytics overall --force",
            "sub",
            "/analytics/overall?force=true",
            1,
            "",
        )?;
        self.read("analytics overall (cached)", "sub", "/analytics/overall")?;
        self.read("analytics rankings (cached)", "sub", "/analytics/rankings")?;
        self.read("analytics time (cached)", "sub", "/analytics/time")
    }

    /// `AT_ONCE` pages of the big conversation requested at the same time: the wall time of all of them.
    fn pages_at_once(&mut self, route: &'static str, big: &str) -> Result<()> {
        let prefix = if route == "api" { "/api" } else { "" };
        let mut walls = Vec::with_capacity(self.runs);
        let mut failed = 0;
        let addr = self.addr;
        for _ in 0..self.runs {
            let begin = Instant::now();
            let statuses: Vec<u16> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..AT_ONCE)
                    .map(|k| {
                        let target = format!(
                            "{prefix}/chat/messages?session-id={}&limit={PAGE}&offset={}",
                            encode(big),
                            k * PAGE
                        );
                        s.spawn(move || request(addr, "GET", &target, "").map_or(0, |r| r.status))
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap_or(0)).collect()
            });
            walls.push(begin.elapsed().as_secs_f64() * 1000.0);
            failed += statuses.iter().filter(|s| **s != 200).count();
        }
        let (median, p95) = median_p95(walls);
        self.push(Row {
            op: format!("{AT_ONCE} pages at once (all of them)"),
            route,
            median,
            p95,
            bytes: None,
            note: if failed > 0 {
                format!("{failed} of {} not 200", self.runs * AT_ONCE as usize)
            } else {
                String::new()
            },
        });
        Ok(())
    }

    /// What the push engine costs: the server's CPU time per second, idle and then with a listener of
    /// `/api/push/messages` (from its second second on, after the baseline), and the events that came meanwhile.
    fn push_cost(&mut self, pid: u32) -> Result<()> {
        let rate = |begin: f64, end: f64| (end - begin) / PUSH_WINDOW.as_secs_f64();
        let cpu = || cpu_ms(pid).ok_or_else(|| anyhow!("cannot read the server's CPU time"));
        let start = cpu()?;
        std::thread::sleep(PUSH_WINDOW);
        let idle = rate(start, cpu()?);
        let mut events = EventStream::open(self.addr)?;
        ensure!(
            events.wait_for("event: ready", Duration::from_secs(10)),
            "no `ready` event on the real account"
        );
        std::thread::sleep(Duration::from_secs(2));
        let start = cpu()?;
        events.wait_for("\u{0}", PUSH_WINDOW);
        let listening = rate(start, cpu()?);
        let pushed = events.text.matches("\nevent: message.").count();
        self.push(Row {
            op: "push engine, CPU ms per second while listened to".into(),
            route: "api",
            median: listening,
            p95: listening,
            bytes: None,
            note: format!(
                "idle {idle:.1}; {pushed} events in {} s",
                PUSH_WINDOW.as_secs()
            ),
        });
        Ok(())
    }

    /// The images of one screen through one route: each alone (first time and again), then all of them at the same
    /// time.
    fn images(&mut self, route: &'static str, images: &[Value]) -> Result<()> {
        let each = |op: &str, thumb: bool, this: &mut Self| -> Result<()> {
            let mut ms = Vec::new();
            let mut ok = 0;
            let mut bytes = 0;
            for img in images {
                let r = image_request(this.addr, route, img, thumb)?;
                ms.push(r.ms);
                if r.status == 200 {
                    ok += 1;
                    bytes += r.body.len();
                }
            }
            // the mean size of one
            let bytes = bytes / ok.max(1);
            let (median, p95) = median_p95(ms);
            this.push(Row {
                op: op.into(),
                route,
                median,
                p95,
                bytes: Some(bytes),
                note: format!("{ok}/{} ok", images.len()),
            });
            Ok(())
        };
        each("image, first time (each)", false, self)?;
        each("image, again (each)", false, self)?;
        each("thumbnail of the image (each)", true, self)?;
        let mut walls = Vec::new();
        let mut ok = 0;
        let addr = self.addr;
        for _ in 0..self.runs {
            let begin = Instant::now();
            let statuses: Vec<u16> = std::thread::scope(|s| {
                let handles: Vec<_> = images
                    .iter()
                    .map(|img| {
                        s.spawn(move || {
                            image_request(addr, route, img, false).map_or(0, |r| r.status)
                        })
                    })
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap_or(0)).collect()
            });
            walls.push(begin.elapsed().as_secs_f64() * 1000.0);
            ok += statuses.iter().filter(|s| **s == 200).count();
        }
        let (median, p95) = median_p95(walls);
        self.push(Row {
            op: format!("{} images at once (all of them)", images.len()),
            route,
            median,
            p95,
            bytes: None,
            note: format!("{ok}/{} ok", self.runs * images.len()),
        });
        Ok(())
    }
}

impl Bench<'_> {
    /// Videos as a player reads them through `/api/`: the start of each, then a part from its middle, as when the
    /// viewer skips ahead. Only the videos whose file is on the computer count.
    fn videos(&mut self, videos: &[Value]) -> Result<()> {
        let query = |v: &Value| {
            ["session-id", "msg-id", "create-time"]
                .iter()
                .filter_map(|k| match &v[*k] {
                    Value::String(s) => Some(format!("{k}={}", encode(s))),
                    Value::Number(n) => Some(format!("{k}={n}")),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("&")
        };
        let part = |v: &Value, first: usize| {
            request_with(
                self.addr,
                "GET",
                &format!("/api/chat/video-data?{}", query(v)),
                "",
                &format!("Range: bytes={first}-{}\r\n", first + VIDEO_PART - 1),
            )
        };
        let (mut start, mut middle, mut thumbs, mut sizes) =
            (Vec::new(), Vec::new(), Vec::new(), 0);
        for v in videos {
            let r = part(v, 0)?;
            if r.status != 206 && r.status != 200 {
                continue;
            }
            start.push(r.ms);
            let total = r.content_range_total.unwrap_or(r.body.len());
            sizes += total;
            middle.push(part(v, total / 2)?.ms);
            let t = request(
                self.addr,
                "GET",
                &format!("/api/chat/video-data?{}&thumb=true", query(v)),
                "",
            )?;
            thumbs.push(t.ms);
        }
        if start.is_empty() {
            return Ok(());
        }
        let found = start.len();
        let note = format!(
            "{found} of {} on disk, {} MB each on average",
            videos.len(),
            sizes / found / (1 << 20)
        );
        for (op, ms) in [
            ("video, its first 1 MB (each)", start),
            ("video, 1 MB from its middle (each)", middle),
            ("thumbnail of the video (each)", thumbs),
        ] {
            let (median, p95) = median_p95(ms);
            self.push(Row {
                op: op.into(),
                route: "api",
                median,
                p95,
                bytes: None,
                note: note.clone(),
            });
        }
        Ok(())
    }
}

fn key(row: &Row) -> String {
    format!("{} [{}]", row.op, row.route)
}

fn main() -> Result<()> {
    let args = parse_args()?;
    std::fs::create_dir_all(&args.work)?;
    let weflow = match &args.weflow {
        Some(p) => p.clone(),
        None => {
            let exe = std::env::current_exe()?;
            let dir = exe
                .parent()
                .and_then(Path::parent)
                .ok_or_else(|| anyhow!("cannot find the folder of the example"))?;
            dir.join(format!("weflow{}", std::env::consts::EXE_SUFFIX))
        }
    };
    ensure!(
        weflow.exists(),
        "{} is missing: build it with `cargo build --release -p weflow-cli --bin weflow` or pass --weflow",
        weflow.display()
    );
    let synthetic = synthetic_home(&weflow, &args)?;
    let (server, _) = start(&weflow, &synthetic, &args.serve)?;
    let data = args.work.join("data");
    let checks = check_answers(&server.addr, &data, &args);
    let (passed, failed, pushed_ms) = (checks.passed, checks.failed, checks.pushed_ms);
    // the reads without an `/api/` route, on an account whose size is known
    let mut synthetic_candidates = Bench {
        addr: &server.addr,
        runs: args.runs,
        rows: Vec::new(),
    };
    synthetic_candidates.candidates(&targets(&server.addr)?)?;
    let synthetic_candidates = synthetic_candidates.rows;
    drop(server);
    // take the pushed message out again (the server had its database open); left behind, it makes the next
    // run build the account again
    if pushed_shard(&data).exists() && std::fs::remove_file(pushed_shard(&data)).is_ok() {
        Fixture::open(&data, OWNER).session_db(&session_specs(&args, None));
    }
    eprintln!(
        "synthetic account: {passed} of {} checks passed",
        passed + failed.len()
    );

    let (server, start_ms) = start(&weflow, &args.home, &args.serve)?;
    let addr = server.addr.clone();
    let addr = addr.as_str();
    let health: Value = serde_json::from_slice(&request(addr, "GET", "/health", "")?.body)?;
    let first = request(addr, "GET", "/api/chat/sessions?limit=50", "")?;
    ensure!(
        first.status == 200,
        "the first read failed with status {}: is the account configured?",
        first.status
    );
    // then the page opens the conversation on top: its first page, before anything else read its databases
    let top = rows(&first.data()?)
        .first()
        .and_then(|s| field(s, &["username", "userName"])?.as_str().map(encode))
        .ok_or_else(|| anyhow!("the account has no conversations"))?;
    let opened = request(
        addr,
        "GET",
        &format!("/api/chat/messages?session-id={top}&limit={PAGE}"),
        "",
    )?;
    ensure!(
        opened.status == 200,
        "the first conversation failed with status {}",
        opened.status
    );
    let t = targets(addr)?;

    let mut bench = Bench {
        addr,
        runs: args.runs,
        rows: vec![
            Row {
                op: "server start (until it listens)".into(),
                route: "-",
                median: start_ms,
                p95: start_ms,
                bytes: None,
                note: "once".into(),
            },
            Row {
                op: "first read after start (sessions, 50)".into(),
                route: "api",
                median: first.ms,
                p95: first.ms,
                bytes: Some(first.body.len()),
                note: "once".into(),
            },
            Row {
                op: "first conversation opened after start (first page)".into(),
                route: "api",
                median: opened.ms,
                p95: opened.ms,
                bytes: Some(opened.body.len()),
                note: "once".into(),
            },
        ],
    };
    let big = encode(&t.big);
    bench.both("sessions, 50", "/chat/sessions?limit=50")?;
    bench.both("sessions, all", "/chat/sessions")?;
    let first_page = format!("/chat/messages?session-id={big}&limit={PAGE}");
    bench.both("messages, first page", &first_page)?;
    bench.read_encoded("messages, first page, gzip", "api", &first_page, "gzip")?;
    bench.read_encoded("messages, first page, br", "api", &first_page, "br")?;
    let shown = format!("{first_page}&fields={MESSAGE_FIELDS}");
    bench.read("messages, first page, the fields shown", "api", &shown)?;
    bench.read_encoded(
        "messages, first page, the fields shown, br",
        "api",
        &shown,
        "br",
    )?;
    bench.both(
        "messages, page in the middle",
        &format!(
            "/chat/messages?session-id={big}&limit={PAGE}&offset={}",
            t.big_messages / 2
        ),
    )?;
    if let Some(cursor) = &t.middle_cursor {
        bench.both(
            "messages, page in the middle, from a cursor",
            &format!(
                "/chat/messages?session-id={big}&limit={PAGE}&before={}",
                encode(cursor)
            ),
        )?;
    }
    bench.both(
        "latest 20 (polling)",
        &format!("/chat/latest?session-id={big}&limit=20"),
    )?;
    if let Some((id, at)) = t.one_message {
        bench.both(
            "one message",
            &format!("/chat/message?session-id={big}&local-id={id}&create-time={at}"),
        )?;
    }
    if let Some(private) = &t.private {
        bench.both(
            "one contact",
            &format!("/chat/contact?username={}", encode(private)),
        )?;
    }
    bench.both("contacts, all", "/chat/contacts")?;
    // the size of an answer (issue #155): compressed, or only the columns a page uses
    let contacts = "/chat/contacts";
    bench.read_encoded("contacts, all, gzip", "api", contacts, "gzip")?;
    bench.read_encoded("contacts, all, br", "api", contacts, "br")?;
    let contacts = format!("{contacts}?fields={CONTACT_FIELDS}");
    bench.read("contacts, all, the fields of a list", "api", &contacts)?;
    bench.read_encoded(
        "contacts, all, the fields of a list, br",
        "api",
        &contacts,
        "br",
    )?;
    let counts: Vec<String> = t
        .sessions
        .iter()
        .map(|s| format!("sessions={}", encode(s)))
        .collect();
    bench.both(
        &format!("counts of {} conversations", t.sessions.len()),
        &format!("/chat/counts?{}", counts.join("&")),
    )?;
    bench.both(
        "search in the conversation",
        &format!(
            "/chat/search?keyword={}&session-id={big}&limit=20",
            encode(&args.keyword)
        ),
    )?;
    // the worst case: every message of the conversation is read
    bench.both(
        "search in the conversation, a word found nowhere",
        &format!(
            "/chat/search?keyword={}&session-id={big}&limit=20",
            encode(ABSENT)
        ),
    )?;
    if let Some(group) = &t.group {
        bench.both(
            "group members",
            &format!("/group/members?chatroom-id={}", encode(group)),
        )?;
    }
    bench.candidates(&t)?;
    bench.pages_at_once("sub", &t.big)?;
    bench.pages_at_once("api", &t.big)?;
    if !t.images.is_empty() {
        bench.images("sub", &t.images)?;
        bench.images("api", &t.images)?;
    }
    bench.videos(&t.videos)?;
    bench.push_cost(server.child.id())?;
    drop(server);

    let baseline = match &args.compare {
        Some(path) => {
            let doc: Value = serde_json::from_slice(
                &std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?,
            )?;
            Some(doc["results"].clone())
        }
        None => None,
    };

    println!(
        "weflow {} on {} {}, {} logical CPUs",
        health["version"].as_str().unwrap_or("?"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        std::thread::available_parallelism().map_or(0, |n| n.get()),
    );
    println!(
        "answers on the synthetic account ({} group messages, {} conversations, {} contacts): {passed} of {} checks passed",
        args.messages,
        args.sessions + 1,
        args.contacts.max(args.sessions),
        passed + failed.len()
    );
    for f in &failed {
        println!("  FAILED {f}");
    }
    if let Some(ms) = pushed_ms {
        println!("  a new message reached the listener after {ms:.0} ms");
    }
    println!(
        "reads that had no /api/ route, on the synthetic account ({SNS_POSTS} moments), {} runs each",
        args.runs
    );
    println!();
    println!("| read | route | median | p95 | bytes | note |");
    println!("|---|---|---:|---:|---:|---|");
    for row in &synthetic_candidates {
        println!(
            "| {} | {} | {:.1} | {:.1} | {} | {} |",
            row.op,
            row.route,
            row.median,
            row.p95,
            row.bytes.map_or_else(|| "-".into(), |b| b.to_string()),
            row.note
        );
    }
    println!();
    println!(
        "speed on the real account, {} runs each, milliseconds as the client sees them",
        args.runs
    );
    println!(
        "biggest of the {} latest conversations: {} messages; {} image messages found for the image reads",
        t.sessions.len(),
        t.big_messages,
        t.images.len()
    );
    println!();
    print!("| read | route | median | p95 | bytes |");
    if baseline.is_some() {
        print!(" vs baseline |");
    }
    println!(" note |");
    print!("|---|---|---:|---:|---:|");
    if baseline.is_some() {
        print!("---:|");
    }
    println!("---|");
    let mut slower = Vec::new();
    for row in &bench.rows {
        print!(
            "| {} | {} | {:.1} | {:.1} | {} |",
            row.op,
            row.route,
            row.median,
            row.p95,
            row.bytes.map_or_else(|| "-".into(), |b| b.to_string())
        );
        if let Some(base) = &baseline {
            match base[key(row)]["median"].as_f64() {
                Some(before) if before > 0.0 => {
                    let ratio = row.median / before;
                    print!(" {ratio:.2}× |");
                    if ratio > args.tolerance && row.median - before > NOISE_MS {
                        slower.push(format!("{}: {before:.1} → {:.1} ms", key(row), row.median));
                    }
                }
                _ => print!(" - |"),
            }
        }
        println!(" {} |", row.note);
    }

    if let Some(path) = &args.save {
        let results: serde_json::Map<String, Value> = bench
            .rows
            .iter()
            .map(|r| {
                (
                    key(r),
                    json!({ "median": r.median, "p95": r.p95, "bytes": r.bytes, "note": r.note }),
                )
            })
            .collect();
        let doc = json!({
            "version": health["version"],
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "account": "real",
            "runs": args.runs,
            "results": results,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&doc)?)?;
    }
    let mut problems = Vec::new();
    if !failed.is_empty() {
        problems.push(format!(
            "{} checks failed on the synthetic account:\n  {}",
            failed.len(),
            failed.join("\n  ")
        ));
    }
    if !slower.is_empty() {
        problems.push(format!(
            "slower than the baseline by more than {}× and {NOISE_MS} ms:\n  {}",
            args.tolerance,
            slower.join("\n  ")
        ));
    }
    ensure!(problems.is_empty(), "{}", problems.join("\n"));
    Ok(())
}
