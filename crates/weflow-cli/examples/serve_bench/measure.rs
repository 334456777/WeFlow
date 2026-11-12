//! Measuring: the timed reads, their spread, the table, `--save` and `--compare`.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::{anyhow, ensure, Result};
use serde_json::{json, Value};

use super::checks::{cpu_ms, image_request, EventStream};
use super::server::{encode, field, number, request, request_with, rows};
use super::{AT_ONCE, PAGE, PUSH_WINDOW, SCREEN_OF_IMAGES, VIDEOS, VIDEO_PART};

pub(crate) struct Row {
    pub(crate) op: String,
    pub(crate) route: &'static str,
    pub(crate) spread: Spread,
    pub(crate) bytes: Option<usize>,
    pub(crate) note: String,
}

/// How the times of one read spread (#16, #257): the median and 95th percentile, the fastest and slowest, and how
/// many were timed.
#[derive(Clone, Copy)]
pub(crate) struct Spread {
    pub(crate) median: f64,
    pub(crate) p95: f64,
    pub(crate) min: f64,
    pub(crate) max: f64,
    pub(crate) n: usize,
}

impl Spread {
    fn of(mut ms: Vec<f64>) -> Self {
        ms.sort_by(f64::total_cmp);
        let n = ms.len();
        let median = if n % 2 == 1 {
            ms[n / 2]
        } else {
            (ms[n / 2 - 1] + ms[n / 2]) / 2.0
        };
        let p95 = ms[((n as f64 * 0.95).ceil() as usize).clamp(1, n) - 1];
        Self {
            median,
            p95,
            min: ms[0],
            max: ms[n - 1],
            n,
        }
    }

    /// A number measured once.
    pub(crate) fn once(ms: f64) -> Self {
        Self::of(vec![ms])
    }
}

/// The conversations the reads are about.
pub(crate) struct Targets {
    pub(crate) sessions: Vec<String>,
    pub(crate) big: String,
    pub(crate) big_messages: i64,
    pub(crate) group: Option<String>,
    pub(crate) private: Option<String>,
    /// (local id, create time): local ids repeat across message databases.
    pub(crate) one_message: Option<(i64, i64)>,
    /// The `_cursor` of the message above the page in the middle of the biggest conversation.
    pub(crate) middle_cursor: Option<String>,
    pub(crate) images: Vec<Value>,
    /// Video messages, whose files may or may not be on the computer.
    pub(crate) videos: Vec<Value>,
}

pub(crate) fn targets(addr: &str) -> Result<Targets> {
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

pub(crate) struct Bench<'a> {
    pub(crate) addr: &'a str,
    pub(crate) runs: usize,
    pub(crate) rows: Vec<Row>,
}

impl Bench<'_> {
    /// Keeps a measured row and shows it on stderr while the others run.
    fn push(&mut self, row: Row) {
        eprintln!(
            "{:<45} {:<3} median {:>8.1} ms  {}",
            row.op, row.route, row.spread.median, row.note
        );
        self.rows.push(row);
    }

    /// Times `runs` requests of one read through the given route.
    pub(crate) fn read(&mut self, op: &str, route: &'static str, target: &str) -> Result<()> {
        self.read_n(op, route, target, self.runs, "")
    }

    /// [`read`](Self::read) asking for a compressed answer: its size is the compressed one.
    pub(crate) fn read_encoded(
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
        let spread = Spread::of(ms);
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
            spread,
            bytes: Some(bytes),
            note,
        });
        Ok(())
    }

    pub(crate) fn both(&mut self, op: &str, target: &str) -> Result<()> {
        self.read(op, "sub", target)?;
        self.read(op, "api", target)
    }

    /// The reads the in-process routes were chosen by (issues #175, #413), through both routes.
    pub(crate) fn candidates(&mut self, t: &Targets) -> Result<()> {
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
            ("biz pay-records".into(), "/biz/pay-records".into()),
            ("report annual years".into(), "/report/annual/years".into()),
            ("config profiles".into(), "/config/profiles".into()),
            ("cache list".into(), "/cache/list".into()),
            ("db test".into(), "/db/test".into()),
        ];
        for kind in ["image", "video", "voice", "file"] {
            reads.push((
                format!("chat resources, all conversations, {kind}"),
                format!("/chat/resources?type={kind}"),
            ));
        }
        // the page after the first 3,000, from the cursor the pages before it end with
        let mut far = None;
        for limit in [2000, 1000] {
            let from = far
                .as_deref()
                .map(|c| format!("&before={}", encode(c)))
                .unwrap_or_default();
            let page = request(
                self.addr,
                "GET",
                &format!("/chat/resources?limit={limit}{from}"),
                "",
            )?
            .data()?;
            far = page["nextCursor"].as_str().map(String::from);
            if far.is_none() {
                break;
            }
        }
        if let Some(cursor) = far {
            reads.push((
                "chat resources, all conversations, far page".into(),
                format!("/chat/resources?before={}", encode(&cursor)),
            ));
        }
        for (op, target) in &reads {
            self.both(op, target)?;
        }
        // the statistics count every chat on each read: slow, so read once each
        for route in ["sub", "api"] {
            for (op, target) in [
                ("analytics overall", "/analytics/overall"),
                ("analytics rankings", "/analytics/rankings"),
                ("analytics time", "/analytics/time"),
            ] {
                self.read_n(op, route, target, 1, "")?;
            }
        }
        Ok(())
    }

    /// `AT_ONCE` pages of the big conversation requested at the same time: the wall time of all of them.
    pub(crate) fn pages_at_once(&mut self, route: &'static str, big: &str) -> Result<()> {
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
        let spread = Spread::of(walls);
        self.push(Row {
            op: format!("{AT_ONCE} pages at once (all of them)"),
            route,
            spread,
            bytes: None,
            note: if failed > 0 {
                format!("{failed} of {} not 200", self.runs * AT_ONCE as usize)
            } else {
                String::new()
            },
        });
        Ok(())
    }

    /// `n` requests of one read at the same time (issue #157): the wall time of all of them, and how many were
    /// refused (`429`) or failed otherwise.
    pub(crate) fn at_once(
        &mut self,
        op: &str,
        route: &'static str,
        target: &str,
        n: usize,
    ) -> Result<()> {
        let path = if route == "api" {
            format!("/api{target}")
        } else {
            target.to_string()
        };
        let (mut walls, mut busy, mut failed) = (Vec::with_capacity(self.runs), 0, 0);
        let addr = self.addr;
        for _ in 0..self.runs {
            let begin = Instant::now();
            let statuses: Vec<u16> = std::thread::scope(|s| {
                let handles: Vec<_> = (0..n)
                    .map(|_| s.spawn(|| request(addr, "GET", &path, "").map_or(0, |r| r.status)))
                    .collect();
                handles.into_iter().map(|h| h.join().unwrap_or(0)).collect()
            });
            walls.push(begin.elapsed().as_secs_f64() * 1000.0);
            busy += statuses.iter().filter(|s| **s == 429).count();
            failed += statuses.iter().filter(|s| **s != 200 && **s != 429).count();
        }
        let spread = Spread::of(walls);
        let all = self.runs * n;
        let answered = (all - busy - failed) as f64 / self.runs as f64;
        let mut note = format!("{:.0} answered a second", answered * 1000.0 / spread.median);
        if busy > 0 {
            note.push_str(&format!("; {busy} of {all} refused (429)"));
        }
        if failed > 0 {
            note.push_str(&format!("; {failed} of {all} failed"));
        }
        self.push(Row {
            op: format!("{op}, {n} at once (all of them)"),
            route,
            spread,
            bytes: None,
            note,
        });
        Ok(())
    }

    /// What the push engine costs: the server's CPU time per second, idle and then with a listener of
    /// `/api/push/messages` (from its second second on, after the baseline), and the events that came meanwhile.
    pub(crate) fn push_cost(&mut self, pid: u32) -> Result<()> {
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
            spread: Spread::once(listening),
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
    pub(crate) fn images(&mut self, route: &'static str, images: &[Value]) -> Result<()> {
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
            let spread = Spread::of(ms);
            this.push(Row {
                op: op.into(),
                route,
                spread,
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
        let spread = Spread::of(walls);
        self.push(Row {
            op: format!("{} images at once (all of them)", images.len()),
            route,
            spread,
            bytes: None,
            note: format!("{ok}/{} ok", self.runs * images.len()),
        });
        Ok(())
    }
}

impl Bench<'_> {
    /// Videos as a player reads them through `/api/`: the start of each, then a part from its middle, as when the
    /// viewer skips ahead. Only the videos whose file is on the computer count.
    pub(crate) fn videos(&mut self, videos: &[Value]) -> Result<()> {
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
            let spread = Spread::of(ms);
            self.push(Row {
                op: op.into(),
                route: "api",
                spread,
                bytes: None,
                note: note.clone(),
            });
        }
        Ok(())
    }
}

/// The median, p95, min, max and n columns of a table row.
pub(crate) fn spread_cells(s: &Spread) -> String {
    format!(
        "{:.1} | {:.1} | {:.1} | {:.1} | {}",
        s.median, s.p95, s.min, s.max, s.n
    )
}

pub(crate) fn key(row: &Row) -> String {
    format!("{} [{}]", row.op, row.route)
}
