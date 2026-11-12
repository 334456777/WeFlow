//! The checks on the synthetic account: the answers of each read through both routes, the push of a new message,
//! and the resources the server takes.

use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};
use weflow_native::fixture::{Fixture, MsgSpec, T0};

use super::account::{image_dat, session_specs, text};
use super::args::{leak, Args};
use super::server::{encode, field, number, request, rows, Reply};
use super::{
    ABSENT, AT_ONCE, IMAGE_EVERY, MEMBERS, OWNER, PAGE, PRIVATE_MESSAGES, PUSHED_AT, ROOM, SHARDS,
    STEP, TOKEN, XOR_KEY,
};

/// The keyword of the checked search: a word of [`text`].
const CHECKED_KEYWORD: &str = "好";

pub(crate) struct Checks<'a> {
    addr: &'a str,
    pub(crate) passed: usize,
    pub(crate) failed: Vec<String>,
    /// How long a new message took to reach a listener of `/api/push/messages`.
    pub(crate) pushed_ms: Option<f64>,
}

/// An open `GET /api/push/messages` and what it sent so far.
pub(crate) struct EventStream {
    stream: TcpStream,
    pub(crate) text: String,
}

impl EventStream {
    pub(crate) fn open(addr: &str) -> Result<Self> {
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
    pub(crate) fn wait_for(&mut self, needle: &str, wait: Duration) -> bool {
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

/// The most memory the process has held so far (its peak resident set, or peak working set on Windows), in MiB; `None`
/// where that cannot be read (macOS gives only the current size).
fn peak_memory_mib(pid: u32) -> Option<f64> {
    if cfg!(target_os = "linux") {
        let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
        let kib: f64 = status
            .lines()
            .find_map(|l| l.strip_prefix("VmHWM:"))?
            .trim()
            .trim_end_matches("kB")
            .trim()
            .parse()
            .ok()?;
        Some(kib / 1024.0)
    } else if cfg!(windows) {
        let out = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!("(Get-Process -Id {pid}).PeakWorkingSet64"),
            ])
            .output()
            .ok()?;
        let bytes: f64 = String::from_utf8_lossy(&out.stdout).trim().parse().ok()?;
        Some(bytes / (1024.0 * 1024.0))
    } else {
        None
    }
}

/// What the server process itself used over the timed reads of the real account: the commands it ran as child
/// processes are not counted.
pub(crate) struct Resources {
    pub(crate) cpu_ms: Option<f64>,
    pub(crate) peak_memory_mib: Option<f64>,
}

impl Resources {
    pub(crate) fn of(pid: u32, cpu_begin: Option<f64>) -> Self {
        Self {
            cpu_ms: cpu_begin.zip(cpu_ms(pid)).map(|(begin, end)| end - begin),
            peak_memory_mib: peak_memory_mib(pid),
        }
    }

    pub(crate) fn to_json(&self) -> Value {
        json!({ "serverCpuMs": self.cpu_ms, "serverPeakMemoryMiB": self.peak_memory_mib })
    }
}

/// CPU time the process has used so far, in milliseconds.
pub(crate) fn cpu_ms(pid: u32) -> Option<f64> {
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
pub(crate) fn check_answers<'a>(addr: &'a str, data: &Path, args: &Args) -> Checks<'a> {
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
                let i = number(field(&m, &["local_id", "localId"])).unwrap_or(0);
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
                let content = field(m, &["message_content", "content"])
                    .and_then(Value::as_str)
                    .unwrap_or("");
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
                        s.spawn(move || loop {
                            // the child-process route answers `429` at once over `--max-reads` (one per CPU), as
                            // documented: on a machine with fewer CPUs than pages, the client tries again later
                            let reply = request(addr, "GET", &target, "")?;
                            if reply.status == 429 && prefix.is_empty() {
                                std::thread::sleep(Duration::from_millis(200));
                                continue;
                            }
                            break reply.data();
                        })
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
pub(crate) fn image_request(addr: &str, route: &str, img: &Value, thumb: bool) -> Result<Reply> {
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
