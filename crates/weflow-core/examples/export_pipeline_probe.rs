//! Export pipeline probe on a synthetic encrypted account; no WeChat data is opened.
//!
//! usage: export_pipeline_probe <work dir> [--messages N] [--shards N] [--format F] [--runs N]
//!                              [--mode all|full|stages|read] [--no-low-memory]
//!
//! `<work dir>/data` is wiped and rebuilt: one group with N messages (default 200000) spread evenly over the
//! shards (default 2), cycling through text, zstd-compressed link and quote appmsgs, images, stickers and system
//! messages from 50 senders. Outputs go to `<work dir>/out`. Modes:
//! - `full`: the real `export_messages` pipeline (reader, parser and writer threads), `--runs` times;
//! - `stages`: each stage alone on one thread (cursor key scan, batch fetch, parse, write), then the parser on
//!   2/4/all threads over the same pages;
//! - `read`: the conversation's range split into 1/2/4/8 parts, each read by its own cursor and thread, which
//!   shows how far reads of one database file run in parallel.
//!
//! Process CPU time (user/sys) is printed on Linux only. To compare allocators without rebuilding, run the same
//! command with `LD_PRELOAD=<path to libjemalloc.so.2>`.
use std::path::Path;
use std::time::Instant;

use serde_json::Value;
use weflow_core::config::{AppContext, ConfigStore};
use weflow_core::export_msg::{
    ContactInfo, DisplayPref, Exporter, NameBook, SessionInfo, Settings,
};
use weflow_core::message::{collect_messages, CollectOptions, ExportMsg};
use weflow_core::services::{MessageExportRequest, ServiceHub};
use weflow_native::fixture::{ContactSpec, Fixture, MsgSpec, RoomSpec, SessionSpec, T0};
use weflow_native::wcdb::Wcdb;

const ROOM: &str = "room_probe@chatroom";
const OWNER: &str = "wxid_me_ab12";
const SENDERS: i64 = 50;
/// Seconds between two synthetic messages.
const STEP: i64 = 7;
const BATCH: i32 = 2000;

struct Args {
    messages: i64,
    shards: i64,
    format: String,
    runs: usize,
    mode: String,
    low_memory: bool,
}

fn parse_args() -> anyhow::Result<(std::path::PathBuf, Args)> {
    let mut it = std::env::args().skip(1);
    let dir = it.next().ok_or_else(|| {
        anyhow::anyhow!("usage: export_pipeline_probe <work dir> [--messages N] [--shards N] [--format F] [--runs N] [--mode all|full|stages|read] [--no-low-memory]")
    })?;
    let mut args = Args {
        messages: 200_000,
        shards: 2,
        format: "chatlab".into(),
        runs: 3,
        mode: "all".into(),
        low_memory: true,
    };
    while let Some(flag) = it.next() {
        let mut value = || {
            it.next()
                .ok_or_else(|| anyhow::anyhow!("{flag} needs a value"))
        };
        match flag.as_str() {
            "--messages" => args.messages = value()?.parse()?,
            "--shards" => args.shards = value()?.parse()?,
            "--format" => args.format = value()?,
            "--runs" => args.runs = value()?.parse()?,
            "--mode" => args.mode = value()?,
            "--no-low-memory" => args.low_memory = false,
            other => anyhow::bail!("unknown option {other}"),
        }
    }
    anyhow::ensure!(
        args.messages > 0 && args.shards > 0,
        "--messages and --shards must be positive"
    );
    Ok((dir.into(), args))
}

/// (user, system) CPU seconds of this process; Linux only.
fn cpu_times() -> Option<(f64, f64)> {
    let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
    let fields: Vec<&str> = stat
        .get(stat.rfind(')')? + 2..)?
        .split_whitespace()
        .collect();
    // utime and stime are fields 14 and 15 of the file, in clock ticks (USER_HZ, 100 on Linux)
    let tick = |i: usize| fields.get(i)?.parse::<f64>().ok().map(|t| t / 100.0);
    Some((tick(11)?, tick(12)?))
}

fn timed<T>(label: &str, f: impl FnOnce() -> T) -> T {
    let (start, cpu) = (Instant::now(), cpu_times());
    let out = f();
    let wall = start.elapsed().as_secs_f64();
    match (cpu, cpu_times()) {
        (Some((u0, s0)), Some((u1, s1))) => println!(
            "{label:<40} wall {wall:>7.3}s  cpu {:>7.3}s (user {:.2}, sys {:.2})",
            (u1 - u0) + (s1 - s0),
            u1 - u0,
            s1 - s0
        ),
        _ => println!("{label:<40} wall {wall:>7.3}s"),
    }
    out
}

fn leak(s: String) -> &'static str {
    Box::leak(s.into_boxed_str())
}

/// (local_type, content, stored compressed) of the `i`-th synthetic message.
fn message(i: i64) -> (i64, String, bool) {
    let r = (i.wrapping_mul(2_654_435_761) >> 7) & 0xffff;
    let words: Vec<char> =
        "今天我们一起去吃饭吧好的没问题明天见哈哈哈这个文件看一下 hello world ok 收到 谢谢"
            .chars()
            .collect();
    let len = 8 + (r % 110) as usize;
    let text: String = (0..len)
        .map(|k| words[(k * 7 + r as usize) % words.len()])
        .collect();
    let who = r % SENDERS;
    let md5 = format!("{:032x}", (i % 300) as u128 * 104_729);
    match i % 20 {
        14 => (
            49,
            format!(
                "<?xml version=\"1.0\"?><msg><appmsg appid=\"\" sdkver=\"0\"><title>{text}</title><des>{text}</des>\
                 <type>5</type><url>https://example.com/a/{i}?x=1&amp;y=2</url><thumburl>https://example.com/t/{i}.jpg</thumburl>\
                 </appmsg><fromusername>wxid_m{who}</fromusername><scene>0</scene><appinfo><version>1</version><appname></appname></appinfo></msg>"
            ),
            true,
        ),
        15 => (
            3,
            format!(
                "<?xml version=\"1.0\"?><msg><img aeskey=\"0123456789abcdef0123456789abcdef\" encryver=\"1\" \
                 cdnthumburl=\"3057020100044b30490201000204{i:08x}\" cdnthumblength=\"4567\" cdnthumbheight=\"120\" \
                 cdnthumbwidth=\"90\" cdnmidimgurl=\"3057020100044b30490201000204{i:08x}\" length=\"123456\" \
                 md5=\"{:032x}\" /></msg>",
                i as u128 * 7919
            ),
            false,
        ),
        16 => (
            47,
            format!(
                "<msg><emoji fromusername=\"wxid_m{who}\" tousername=\"{ROOM}\" type=\"2\" md5=\"{md5}\" len=\"12345\" \
                 productid=\"\" androidmd5=\"{md5}\" cdnurl=\"http://example.com/emoji/{i}\" width=\"240\" height=\"240\" /></msg>"
            ),
            false,
        ),
        17 => (
            10000,
            format!("\"成员{who}\"邀请\"成员{}\"加入了群聊", (who + 1) % SENDERS),
            false,
        ),
        18 => (
            49,
            format!(
                "<?xml version=\"1.0\"?><msg><appmsg appid=\"\" sdkver=\"0\"><title>{text}</title><type>57</type>\
                 <refermsg><type>1</type><svrid>{}</svrid><fromusr>wxid_m{who}</fromusr><displayname>成员</displayname>\
                 <content>{text}</content></refermsg></appmsg></msg>",
                1000 + i - 3
            ),
            true,
        ),
        _ => (1, text, false),
    }
}

fn build(root: &Path, args: &Args) -> Fixture {
    let f = Fixture::new(root, OWNER);
    let members: Vec<&'static str> = (0..SENDERS).map(|k| leak(format!("wxid_m{k}"))).collect();
    let mut contacts: Vec<ContactSpec> = members
        .iter()
        .map(|m| ContactSpec::new(m, 1, leak(format!("昵称{m}"))))
        .collect();
    contacts.push(ContactSpec::new("wxid_me", 1, "Me"));
    contacts.push(ContactSpec::new(ROOM, 2, "Probe room"));
    let nicks: Vec<(&'static str, &'static str)> = members
        .iter()
        .map(|m| (*m, leak(format!("群昵称{m}"))))
        .collect();
    f.session_db(&[SessionSpec {
        username: ROOM,
        summary: "probe",
        last_timestamp: T0 + args.messages * STEP,
        unread: 0,
        last_msg_type: 1,
    }]);
    f.contact_db(
        &contacts,
        &[RoomSpec {
            username: ROOM,
            owner: members[0],
            members: Box::leak(nicks.into_boxed_slice()),
        }],
    );
    let per = args.messages / args.shards;
    for shard in 0..args.shards {
        let first = shard * per + 1;
        let last = if shard == args.shards - 1 {
            args.messages
        } else {
            (shard + 1) * per
        };
        let msgs: Vec<MsgSpec> = (first..=last)
            .map(|i| {
                let (local_type, content, compressed) = message(i);
                let sender = match (local_type, i % 9) {
                    (10000, _) => "",
                    (_, 0) => "wxid_me",
                    _ => members[((i * 31) % SENDERS) as usize],
                };
                let m = MsgSpec::text(i, sender, T0 + i * STEP, &content).of_type(local_type);
                if compressed {
                    m.compressed()
                } else {
                    m
                }
            })
            .collect();
        f.message_shard(shard as u32, &[(ROOM, msgs)]);
    }
    f
}

fn request(format: &str) -> MessageExportRequest {
    MessageExportRequest {
        session_id: ROOM.into(),
        format: format.into(),
        start: None,
        end: None,
        sender: None,
        display_pref: DisplayPref::Remark,
        excel_compact: false,
    }
}

/// Every page of the conversation's cursor over `[begin, end]` (`0` = unbounded), in order.
fn read_pages(wcdb: &Wcdb, begin: i32, end: i32) -> anyhow::Result<Vec<Vec<Value>>> {
    let cursor = wcdb.open_message_cursor(ROOM, BATCH, true, begin, end, false)?;
    let pages = fetch_all(wcdb, cursor);
    let _ = wcdb.close_message_cursor(cursor);
    pages
}

/// Rows of the conversation's cursor over `[begin, end]`, each page dropped as soon as it is read (what a
/// streaming reader costs, without holding the pages).
fn count_rows(wcdb: &Wcdb, begin: i32, end: i32) -> anyhow::Result<usize> {
    let cursor = wcdb.open_message_cursor(ROOM, BATCH, true, begin, end, false)?;
    let mut rows = 0;
    let result = loop {
        match wcdb.fetch_message_batch(cursor) {
            Ok((page, more)) => {
                rows += page.as_array().map_or(0, Vec::len);
                if !more {
                    break Ok(rows);
                }
            }
            Err(e) => break Err(e),
        }
    };
    let _ = wcdb.close_message_cursor(cursor);
    result
}

fn fetch_all(wcdb: &Wcdb, cursor: i64) -> anyhow::Result<Vec<Vec<Value>>> {
    let mut pages = Vec::new();
    loop {
        let (page, more) = wcdb.fetch_message_batch(cursor)?;
        match page {
            Value::Array(rows) if !rows.is_empty() => pages.push(rows),
            _ => break,
        }
        if !more {
            break;
        }
    }
    Ok(pages)
}

fn full(root: &Path, fixture: &Fixture, args: &Args) -> anyhow::Result<()> {
    let ctx = AppContext {
        home_dir: root.join("home"),
        config_path: root.join("home/config.json"),
        runtime_dir: root.join("runtime"),
        version: "probe".into(),
    };
    std::fs::create_dir_all(&ctx.home_dir)?;
    let hub = ServiceHub::new(
        ctx,
        ConfigStore::default(),
        None,
        Some(fixture.account_dir.to_string_lossy().to_string()),
        Some(fixture.key_hex()),
        Some(OWNER.into()),
    );
    let out = root.join("out");
    // first run pays the key derivations and remembers the verified key: keep it out of the numbers
    timed("full pipeline warm-up (txt)", || {
        hub.export_messages(&request("txt"), &out.join("warm-up.txt"))
    })?;
    for run in 1..=args.runs {
        let result = timed(&format!("full pipeline {} #{run}", args.format), || {
            hub.export_messages(
                &request(&args.format),
                &out.join(format!("full.{}", args.format)),
            )
        })?;
        anyhow::ensure!(
            result["count"].as_i64() == Some(args.messages),
            "exported {} messages, expected {}",
            result["count"],
            args.messages
        );
    }
    Ok(())
}

fn stages(root: &Path, wcdb: &Wcdb, args: &Args) -> anyhow::Result<()> {
    let opts = CollectOptions {
        session_id: ROOM,
        my_wxid: "wxid_me",
        start: None,
        end: None,
        sender_filter: None,
    };
    for run in 1..=args.runs.min(2) {
        let cursor = timed(&format!("reader: open cursor (key scan) #{run}"), || {
            wcdb.open_message_cursor(ROOM, BATCH, true, 0, 0, false)
        })?;
        let pages = timed(&format!("reader: fetch every batch #{run}"), || {
            fetch_all(wcdb, cursor)
        });
        let _ = wcdb.close_message_cursor(cursor);
        let pages = pages?;
        let msgs: Vec<ExportMsg> = timed(&format!("parser: collect_messages #{run}"), || {
            pages
                .iter()
                .flat_map(|p| collect_messages(p, &opts))
                .collect()
        });
        anyhow::ensure!(
            msgs.len() as i64 == args.messages,
            "parsed {} messages",
            msgs.len()
        );
        timed("drop the raw pages", || drop(pages));

        let pages = read_pages(wcdb, 0, 0)?;
        let all = std::thread::available_parallelism().map_or(4, |n| n.get());
        for workers in [2, 4, all] {
            timed(
                &format!("parser: collect_messages on {workers} threads"),
                || {
                    let next = std::sync::atomic::AtomicUsize::new(0);
                    std::thread::scope(|s| {
                        for _ in 0..workers {
                            s.spawn(|| loop {
                                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                let Some(page) = pages.get(i) else { break };
                                std::hint::black_box(collect_messages(page, &opts));
                            });
                        }
                    });
                },
            );
        }
        drop(pages);

        for format in [args.format.as_str(), "txt"] {
            let mut names = NameBook::new(|u: &str| {
                wcdb.contact(u)
                    .ok()
                    .filter(Value::is_object)
                    .map(|v| ContactInfo::from_value(u, &v))
            });
            let mut exporter = Exporter {
                session: SessionInfo {
                    id: ROOM.into(),
                    display_name: "Probe room".into(),
                    nickname: String::new(),
                    remark: String::new(),
                    is_group: true,
                },
                my_wxid: "wxid_me".into(),
                raw_my_wxid: OWNER.into(),
                my_display: "Me".into(),
                group_nicks: Default::default(),
                group_members: Vec::new(),
                names: &mut names,
                settings: Settings::default(),
            };
            let out = root.join("out").join(format!("stage.{format}"));
            timed(&format!("writer: write_streamed {format} #{run}"), || {
                exporter.write_streamed(format, &out, &mut msgs.iter().cloned())
            })?;
        }
    }
    Ok(())
}

fn parallel_read(wcdb: &Wcdb, args: &Args) -> anyhow::Result<()> {
    let span = args.messages * STEP;
    // opens every shard's snapshot (and, with --no-low-memory, fills the page caches), so the rows below compare
    // the same warm state
    timed("reader: warm-up pass", || count_rows(wcdb, 0, 0))?;
    for parts in [1i64, 2, 4, 8] {
        let rows = timed(&format!("reader: range split over {parts} threads"), || {
            std::thread::scope(|s| {
                let handles: Vec<_> = (0..parts)
                    .map(|k| {
                        // disjoint inclusive ranges covering every message
                        let begin = T0 + k * span / parts + 1;
                        let end = T0 + (k + 1) * span / parts;
                        s.spawn(move || count_rows(wcdb, begin as i32, end as i32))
                    })
                    .collect();
                handles.into_iter().try_fold(0usize, |sum, h| {
                    anyhow::Ok(sum + h.join().expect("reader thread panicked")?)
                })
            })
        })?;
        anyhow::ensure!(rows as i64 == args.messages, "read {rows} rows");
    }
    Ok(())
}

fn main() -> anyhow::Result<()> {
    let (root, args) = parse_args()?;
    std::fs::create_dir_all(root.join("out"))?;
    let fixture = timed("build the synthetic account", || {
        build(&root.join("data"), &args)
    });
    if matches!(args.mode.as_str(), "all" | "full") {
        full(&root, &fixture, &args)?;
    }
    if matches!(args.mode.as_str(), "all" | "stages" | "read") {
        let mut wcdb = Wcdb::new();
        wcdb.open_unchecked(&fixture.account_dir, &fixture.key_hex(), Some(OWNER))?;
        // exports run in low-memory mode (16 MiB page cache per database)
        wcdb.set_low_memory(args.low_memory);
        if args.mode != "read" {
            stages(&root, &wcdb, &args)?;
        }
        if args.mode != "stages" {
            parallel_read(&wcdb, &args)?;
        }
    }
    Ok(())
}
