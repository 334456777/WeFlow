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
//! (default 10) through both routes; the table shows the median, the 95th percentile, the fastest and the slowest in
//! milliseconds as the client sees them (connection included), how many were timed, and the size of the answer. A
//! last row gives the server's CPU time per second while `/api/push/messages` is listened to (the engine looks for
//! new messages every second), next to the idle one; below the table, the CPU time and the peak memory of the server
//! process over all of these reads (the commands it runs as child processes are not counted). Both accounts also time
//! the reads the in-process routes were chosen by (`chat dates`, `chat date-counts`, `chat statuses`,
//! `chat detail --fast` and others; issues #175, #413) through both routes, in a table of their own on the synthetic
//! one. `--save` writes the numbers to a JSON file; `--compare` reads such a file, adds a column with the ratio to it
//! and fails when a median is more than `--tolerance` times (default 2) and more than 20 ms slower.
#![expect(
    clippy::unwrap_used,
    reason = "probe code reports failure by panicking"
)]
#![expect(
    clippy::cast_possible_truncation,
    reason = "probe and benchmark data are small values that fit"
)]

use std::path::Path;
use std::time::Duration;

use anyhow::{anyhow, ensure, Context, Result};
use serde_json::{json, Value};
use weflow_native::fixture::Fixture;

mod account;
mod args;
mod checks;
mod measure;
mod server;

use account::{pushed_shard, session_specs, synthetic_home};
use args::parse_args;
use checks::{check_answers, cpu_ms, Resources};
use measure::{key, spread_cells, targets, Bench, Row, Spread};
use server::{encode, field, request, rows, start};

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
/// Moments of the synthetic account, from its friends in turn.
const SNS_POSTS: i64 = 2000;
/// How long the push engine's CPU time is measured on the real account, idle and with a listener.
/// The columns a chat window shows of a message, and a list of a contact (`?fields=`). A message's are named both
/// ways: as WeChat's columns and as `MessageRow`'s (#492), so a server answering either keeps them.
const MESSAGE_FIELDS: &str =
    "local_id,server_id,local_type,sort_seq,create_time,sender_username,is_send,message_content,\
     localId,serverId,localType,sortSeq,createTime,sender,isSend,content,_cursor";
const CONTACT_FIELDS: &str = "username,alias,remark,nick_name,small_head_url,local_type";
const PUSH_WINDOW: Duration = Duration::from_secs(10);

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
    let cpu_begin = cpu_ms(server.child.id());

    let mut bench = Bench {
        addr,
        runs: args.runs,
        rows: vec![
            Row {
                op: "server start (until it listens)".into(),
                route: "-",
                spread: Spread::once(start_ms),
                bytes: None,
                note: "once".into(),
            },
            Row {
                op: "first read after start (sessions, 50)".into(),
                route: "api",
                spread: Spread::once(first.ms),
                bytes: Some(first.body.len()),
                note: "once".into(),
            },
            Row {
                op: "first conversation opened after start (first page)".into(),
                route: "api",
                spread: Spread::once(opened.ms),
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
    // the same read asked for by many at once, around the limit of reads (one per CPU by default): a browser
    // sends at most 6 at once to one server
    let first_page = format!("/chat/messages?session-id={big}&limit={PAGE}");
    for n in [1, 6, 16, 32, 64] {
        bench.at_once("first page", "api", &first_page, n)?;
    }
    for n in [1, 6, 16, 32] {
        bench.at_once("first page", "sub", &first_page, n)?;
    }
    if !t.images.is_empty() {
        bench.images("sub", &t.images)?;
        bench.images("api", &t.images)?;
    }
    bench.videos(&t.videos)?;
    bench.push_cost(server.child.id())?;
    let resources = Resources::of(server.child.id(), cpu_begin);
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
        "reads the in-process routes were chosen by, on the synthetic account ({SNS_POSTS} moments), {} runs each",
        args.runs
    );
    println!();
    println!("| read | route | median | p95 | min | max | n | bytes | note |");
    println!("|---|---|---:|---:|---:|---:|---:|---:|---|");
    for row in &synthetic_candidates {
        println!(
            "| {} | {} | {} | {} | {} |",
            row.op,
            row.route,
            spread_cells(&row.spread),
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
    print!("| read | route | median | p95 | min | max | n | bytes |");
    if baseline.is_some() {
        print!(" vs baseline |");
    }
    println!(" note |");
    print!("|---|---|---:|---:|---:|---:|---:|---:|");
    if baseline.is_some() {
        print!("---:|");
    }
    println!("---|");
    let mut slower = Vec::new();
    for row in &bench.rows {
        print!(
            "| {} | {} | {} | {} |",
            row.op,
            row.route,
            spread_cells(&row.spread),
            row.bytes.map_or_else(|| "-".into(), |b| b.to_string())
        );
        if let Some(base) = &baseline {
            match base[key(row)]["median"].as_f64() {
                Some(before) if before > 0.0 => {
                    let ratio = row.spread.median / before;
                    print!(" {ratio:.2}× |");
                    if ratio > args.tolerance && row.spread.median - before > NOISE_MS {
                        slower.push(format!(
                            "{}: {before:.1} → {:.1} ms",
                            key(row),
                            row.spread.median
                        ));
                    }
                }
                _ => print!(" - |"),
            }
        }
        println!(" {} |", row.note);
    }

    println!();
    let shown = |v: Option<f64>, unit: &str| {
        v.map_or_else(|| "-".to_string(), |v| format!("{v:.0} {unit}"))
    };
    println!(
        "the server process over these reads (its child processes not counted): CPU {}, peak memory {}",
        shown(resources.cpu_ms, "ms"),
        shown(resources.peak_memory_mib, "MiB")
    );

    if let Some(path) = &args.save {
        let results: serde_json::Map<String, Value> = bench
            .rows
            .iter()
            .map(|r| {
                (
                    key(r),
                    json!({
                        "median": r.spread.median,
                        "p95": r.spread.p95,
                        "min": r.spread.min,
                        "max": r.spread.max,
                        "n": r.spread.n,
                        "bytes": r.bytes,
                        "note": r.note,
                    }),
                )
            })
            .collect();
        let doc = json!({
            "version": health["version"],
            "os": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "account": "real",
            "runs": args.runs,
            "resources": resources.to_json(),
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
