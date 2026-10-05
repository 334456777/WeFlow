//! Reading a conversation for an export: worker threads read and parse its pages side by side while the caller
//! consumes the messages in order.
//!
//! A message cursor knows every message of the range (in order) once it is open, so page `i` is fixed before
//! anything is read. Each worker takes the next page number, reads that page from the database and turns its rows
//! into export messages; the consumer puts the pages back in order. Reading (SQLite, page decryption, building the
//! rows) and parsing are both CPU work, and reading is the larger part, so it is the part that runs in parallel.
//! The database hands each worker its own connection (see `Wcdb::set_read_connections`), so reads of one
//! database file do not wait for each other.
//!
//! How many workers help depends on the export: a format that is slow to write (ChatLab, JSON) keeps up with two,
//! a light one (TXT) needs more. So the export starts with [`INITIAL_WORKERS`] and wakes one more whenever the
//! consumer spent more than a tenth of its time waiting for pages, up to one per CPU but one (the consumer's), at
//! most [`MAX_WORKERS`]. Every worker that reads costs memory (its database connection keeps a page cache, its
//! pages wait for the consumer), so workers that are not needed stay asleep and never open a connection.
//!
//! Workers run at most [`PAGES_AHEAD_PER_WORKER`] pages per working worker ahead of the consumer, so memory stays
//! bounded however slow the consumer or one early page is. A failed page, a panicking worker or a consumer that
//! stops early stops every worker; an export never ends as if it were complete after a page went missing.

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use serde_json::Value;
use weflow_native::wcdb::Wcdb;

use super::MessageExportRequest;
use crate::error::{AppError, AppResult};
use crate::message::{CollectOptions, ExportMsg};

/// Messages per page (cursor batch). Exports sort each page by time, so the page size is part of the output order
/// when `sort_seq` and `create_time` disagree: keep it fixed.
const PAGE_SIZE: i32 = 2000;
/// How many pages per worker may be read ahead of the one the consumer is on.
const PAGES_AHEAD_PER_WORKER: usize = 2;
/// Workers reading from the start (unless the CPUs or the pages are fewer).
const INITIAL_WORKERS: usize = 2;
/// Most workers the export wakes on its own.
const MAX_WORKERS: usize = 8;
/// Pages the consumer takes before it judges whether it waited too long.
const PAGES_PER_CHECK: usize = 2;
/// Fixes the worker count (a number of threads, at least 1) instead of adapting it, for measurements.
pub const WORKERS_ENV: &str = "WEFLOW_EXPORT_WORKERS";

/// Runs `f` on the conversation's messages (oldest first), read and parsed while `f` consumes them.
pub(super) fn read_export_stream<R>(
    wcdb: &Wcdb,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
    f: impl FnOnce(&mut dyn Iterator<Item = ExportMsg>) -> R,
) -> AppResult<R> {
    let clamp = |t: i64| t.clamp(0, i32::MAX as i64) as i32;
    let (begin, finish) = (req.start.map_or(0, clamp), req.end.map_or(0, clamp));
    let cursor = wcdb
        .open_message_cursor(&req.session_id, PAGE_SIZE, true, begin, finish, false)
        .map_err(|e| AppError::native(e.to_string()))?;
    let result = read_cursor(wcdb, cursor, req, opts, f);
    let _ = wcdb.close_message_cursor(cursor);
    result
}

/// (workers reading from the start, most workers) for `pages` pages: both [`WORKERS_ENV`] when it is set; else
/// [`INITIAL_WORKERS`] growing to one per CPU but one (the consumer formats and writes), at most [`MAX_WORKERS`].
/// Never more than there are pages.
fn worker_counts(pages: usize, configured: Option<&str>, cpus: usize) -> (usize, usize) {
    let fit = |n: usize| n.min(pages).max(1);
    match configured
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
    {
        Some(n) => (fit(n), fit(n)),
        None => {
            let most = fit(cpus.saturating_sub(1).clamp(1, MAX_WORKERS));
            (INITIAL_WORKERS.min(most), most)
        }
    }
}

fn read_cursor<R>(
    wcdb: &Wcdb,
    cursor: i64,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
    f: impl FnOnce(&mut dyn Iterator<Item = ExportMsg>) -> R,
) -> AppResult<R> {
    let started = Instant::now();
    let (total, batch) = wcdb
        .message_cursor_len(cursor)
        .map_err(|e| AppError::native(e.to_string()))?;
    let pages = total.div_ceil(batch.max(1));
    let (initial, workers) = worker_counts(
        pages,
        std::env::var(WORKERS_ENV).ok().as_deref(),
        std::thread::available_parallelism().map_or(2, |n| n.get()),
    );
    // connections open only when a worker reads, so sleeping workers cost none
    wcdb.set_read_connections(workers);
    let gate = Gate::new(initial, workers);
    let captions: OnceLock<HashMap<String, String>> = OnceLock::new();
    let load_captions = || wcdb.emoticon_captions().unwrap_or_default();
    let page_source = PageSource {
        wcdb,
        cursor,
        req,
        opts,
        captions: &captions,
        load_captions: &load_captions,
    };
    let (tx, rx) = channel();
    std::thread::scope(|scope| {
        if pages > 0 {
            // the sticker caption table is one query over a whole database: start it now, beside the first pages
            scope.spawn(|| captions.get_or_init(load_captions));
        }
        let handles: Vec<_> = (0..workers)
            .map(|index| {
                let tx = tx.clone();
                let (gate, source) = (&gate, &page_source);
                scope.spawn(move || work(index, gate, source, pages, tx))
            })
            .collect();
        drop(tx);
        let mut stream = MessageStream::new(rx, &gate, pages, total);
        let result = f(&mut stream);
        gate.halt(); // the consumer is done (or gave up): workers stop at their next page
        let MessageStream {
            rx, error, stats, ..
        } = stream;
        drop(rx); // a worker still reading fails its send and stops
        let panicked = handles.into_iter().any(|h| h.join().is_err());
        if panicked {
            return Err(AppError::runtime("message reader thread panicked"));
        }
        if let Some(e) = error {
            return Err(e);
        }
        stats.log(gate.active(), pages, total, started.elapsed());
        Ok(result)
    })
}

/// What a worker needs to read and parse one page.
struct PageSource<'a> {
    wcdb: &'a Wcdb,
    cursor: i64,
    req: &'a MessageExportRequest,
    opts: &'a CollectOptions<'a>,
    captions: &'a OnceLock<HashMap<String, String>>,
    load_captions: &'a (dyn Fn() -> HashMap<String, String> + Sync),
}

/// One page, read and parsed.
struct ParsedPage {
    /// Rows the cursor returned (before the time and sender filters): the progress counts these.
    scanned: usize,
    msgs: Vec<ExportMsg>,
    fetch: Duration,
    parse: Duration,
}

type PageResult = (usize, AppResult<ParsedPage>);

impl PageSource<'_> {
    fn read(&self, page: usize) -> AppResult<ParsedPage> {
        let t = Instant::now();
        let rows = match self.wcdb.fetch_message_page(self.cursor, page) {
            Ok(Value::Array(rows)) => rows,
            Ok(_) => Vec::new(),
            Err(e) => return Err(AppError::native(e.to_string())),
        };
        let fetch = t.elapsed();
        let scanned = rows.len();
        let (start, end) = (self.req.start, self.req.end);
        let sender = self
            .opts
            .sender_filter
            .map(str::trim)
            .filter(|f| !f.is_empty());
        let rows: Vec<Value> = rows
            .into_iter()
            .filter(|m| {
                let ts = crate::message::get_timestamp_seconds(m);
                start.is_none_or(|s| ts >= s) && end.is_none_or(|e| ts < e)
            })
            // drop other senders' rows before parsing them
            .filter(|row| {
                sender.is_none_or(|f| {
                    crate::message::is_same_wxid(
                        &crate::message::row_sender(row, self.opts.session_id, self.opts.my_wxid),
                        f,
                    )
                })
            })
            .collect();
        let mut msgs = crate::message::collect_messages(&rows, self.opts);
        drop(rows);
        if msgs.iter().any(super::is_sticker) {
            let table = self.captions.get_or_init(self.load_captions);
            super::attach_emoji_captions(table, &mut msgs);
        }
        Ok(ParsedPage {
            scanned,
            msgs,
            fetch,
            parse: t.elapsed() - fetch,
        })
    }
}

/// Worker number `index`: once the export needs it, reads the next unclaimed page until there is none left or the
/// export stops.
fn work(index: usize, gate: &Gate, source: &PageSource<'_>, pages: usize, tx: Sender<PageResult>) {
    let _halt = HaltOnPanic(gate);
    if !gate.wait_until_needed(index) {
        return;
    }
    while !gate.halted() {
        let page = gate.next.fetch_add(1, Ordering::Relaxed);
        if page >= pages || !gate.wait_for_room(page) {
            break;
        }
        let parsed = source.read(page);
        let failed = parsed.is_err();
        if tx.send((page, parsed)).is_err() || failed {
            gate.halt();
            break;
        }
    }
}

/// Coordination between the workers and the consumer.
struct Gate {
    /// Next page number to hand out.
    next: AtomicUsize,
    /// Pages the consumer has taken so far.
    taken: Mutex<usize>,
    moved: Condvar,
    halted: AtomicBool,
    /// Workers allowed to read (the others sleep).
    active: AtomicUsize,
    /// Workers there are.
    workers: usize,
}

impl Gate {
    fn new(initial: usize, workers: usize) -> Self {
        let workers = workers.max(1);
        Self {
            next: AtomicUsize::new(0),
            taken: Mutex::new(0),
            moved: Condvar::new(),
            halted: AtomicBool::new(false),
            active: AtomicUsize::new(initial.clamp(1, workers)),
            workers,
        }
    }

    fn halted(&self) -> bool {
        self.halted.load(Ordering::Acquire)
    }

    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    /// Pages that may be claimed beyond the ones taken.
    fn ahead(&self) -> usize {
        self.active() * PAGES_AHEAD_PER_WORKER
    }

    /// Wakes one more worker; false when all of them already read.
    fn grow(&self) -> bool {
        let grown = self
            .active
            .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < self.workers).then_some(n + 1)
            })
            .is_ok();
        if grown {
            let _taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
            self.moved.notify_all();
        }
        grown
    }

    /// Waits until worker `index` may read; false when the export stopped first.
    fn wait_until_needed(&self, index: usize) -> bool {
        let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        while index >= self.active() && !self.halted() {
            taken = self
                .moved
                .wait(taken)
                .unwrap_or_else(PoisonError::into_inner);
        }
        !self.halted()
    }

    /// Stops the workers, waking the ones that wait for room.
    fn halt(&self) {
        self.halted.store(true, Ordering::Release);
        let _taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        self.moved.notify_all();
    }

    /// Waits until `page` is close enough to the consumer; false when the export stopped meanwhile. Pages are
    /// claimed in order, so the page the consumer waits for never waits here.
    fn wait_for_room(&self, page: usize) -> bool {
        let mut taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        while page >= *taken + self.ahead() && !self.halted() {
            taken = self
                .moved
                .wait(taken)
                .unwrap_or_else(PoisonError::into_inner);
        }
        !self.halted()
    }

    fn set_taken(&self, taken: usize) {
        *self.taken.lock().unwrap_or_else(PoisonError::into_inner) = taken;
        self.moved.notify_all();
    }
}

/// Halts the export when a worker panics, so the others do not wait for room the consumer will never make.
struct HaltOnPanic<'a>(&'a Gate);

impl Drop for HaltOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.halt();
        }
    }
}

/// Time spent per stage, summed over the workers.
#[derive(Default)]
struct Stats {
    fetch: Duration,
    parse: Duration,
    /// Time the consumer waited for the next page (the rest of its time went to formatting and writing).
    starved: Duration,
}

impl Stats {
    /// Logged with `RUST_LOG=weflow::export=debug`.
    fn log(&self, workers: usize, pages: usize, total: usize, elapsed: Duration) {
        tracing::debug!(
            target: "weflow::export",
            workers,
            pages,
            messages = total,
            elapsed_ms = elapsed.as_millis() as u64,
            fetch_ms = self.fetch.as_millis() as u64,
            parse_ms = self.parse.as_millis() as u64,
            consumer_waiting_ms = self.starved.as_millis() as u64,
            "message export read"
        );
    }
}

/// The parsed pages as one message iterator, in page order. It ends early when a page failed (see `error`).
struct MessageStream<'a> {
    rx: Receiver<PageResult>,
    gate: &'a Gate,
    pages: usize,
    next_page: usize,
    /// Pages that arrived before the ones they follow.
    pending: BTreeMap<usize, ParsedPage>,
    current: std::vec::IntoIter<ExportMsg>,
    error: Option<AppError>,
    total: usize,
    scanned: usize,
    stats: Stats,
    /// When the consumer took the previous page, and since the last check: pages taken, time, time waited.
    last_take: Option<Instant>,
    window: (usize, Duration, Duration),
}

impl<'a> MessageStream<'a> {
    fn new(rx: Receiver<PageResult>, gate: &'a Gate, pages: usize, total: usize) -> Self {
        Self {
            rx,
            gate,
            pages,
            next_page: 0,
            pending: BTreeMap::new(),
            current: Vec::new().into_iter(),
            error: None,
            total,
            scanned: 0,
            stats: Stats::default(),
            last_take: None,
            window: Default::default(),
        }
    }

    /// Wakes one more worker when the consumer waited for more than a tenth of its time since the last check.
    /// (The wait for the first page is how long a page takes, not a sign of too few workers.)
    /// `now` is when the page was taken.
    fn adapt(&mut self, waited: Duration, now: Instant) {
        if let Some(last) = self.last_take.replace(now) {
            self.window.0 += 1;
            self.window.1 += now - last;
            self.window.2 += waited;
        }
        let (pages, time, wait) = self.window;
        if pages >= PAGES_PER_CHECK {
            if wait * 10 > time {
                self.gate.grow();
            }
            self.window = Default::default();
        }
    }

    fn take(&mut self, page: ParsedPage, waited: Duration) {
        self.adapt(waited, Instant::now());
        self.next_page += 1;
        self.gate.set_taken(self.next_page);
        self.scanned += page.scanned;
        self.stats.fetch += page.fetch;
        self.stats.parse += page.parse;
        crate::output::progress(
            "messages",
            "reading messages",
            self.scanned,
            self.total.max(self.scanned),
        );
        self.current = page.msgs.into_iter();
    }
}

impl Iterator for MessageStream<'_> {
    type Item = ExportMsg;

    fn next(&mut self) -> Option<ExportMsg> {
        loop {
            if let Some(m) = self.current.next() {
                return Some(m);
            }
            if self.error.is_some() || self.next_page >= self.pages {
                return None;
            }
            let mut waited = Duration::ZERO;
            while !self.pending.contains_key(&self.next_page) {
                let since = Instant::now();
                let received = self.rx.recv();
                waited += since.elapsed();
                match received {
                    Ok((i, Ok(page))) => {
                        self.pending.insert(i, page);
                    }
                    Ok((_, Err(e))) => {
                        self.error = Some(e);
                        return None;
                    }
                    Err(_) => {
                        self.error = Some(AppError::runtime(
                            "the message readers stopped before the end of the conversation",
                        ));
                        return None;
                    }
                }
            }
            self.stats.starved += waited;
            if let Some(page) = self.pending.remove(&self.next_page) {
                self.take(page, waited);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn worker_counts_follow_cpus_pages_and_the_override() {
        // start with two, grow to one per CPU but the consumer's, capped, never more than the pages
        assert_eq!(worker_counts(100, None, 16), (INITIAL_WORKERS, MAX_WORKERS));
        assert_eq!(worker_counts(100, None, 4), (2, 3));
        assert_eq!(worker_counts(100, None, 2), (1, 1));
        assert_eq!(worker_counts(100, None, 1), (1, 1));
        assert_eq!(worker_counts(2, None, 16), (2, 2));
        assert_eq!(worker_counts(1, None, 16), (1, 1));
        assert_eq!(worker_counts(0, None, 16), (1, 1));
        // the override fixes the count, above the CPU count but not above the page count; nonsense is ignored
        assert_eq!(worker_counts(100, Some(" 12 "), 4), (12, 12));
        assert_eq!(worker_counts(3, Some("12"), 4), (3, 3));
        assert_eq!(worker_counts(100, Some("0"), 4), (2, 3));
        assert_eq!(worker_counts(100, Some("many"), 4), (2, 3));
    }

    #[test]
    fn sleeping_workers_start_when_the_export_grows_or_stops() {
        let gate = Gate::new(1, 3);
        assert!(gate.wait_until_needed(0));
        std::thread::scope(|s| {
            let second = s.spawn(|| gate.wait_until_needed(1));
            let third = s.spawn(|| gate.wait_until_needed(2));
            std::thread::sleep(Duration::from_millis(20));
            assert!(!second.is_finished() && !third.is_finished());
            assert!(gate.grow());
            assert!(second.join().unwrap());
            assert!(!third.is_finished());
            gate.halt();
            assert!(!third.join().unwrap());
        });
        assert!(gate.grow());
        assert!(!gate.grow(), "all three already read");
        assert_eq!(gate.active(), 3);
    }

    #[test]
    fn a_waiting_consumer_wakes_more_workers_a_busy_one_does_not() {
        let gate = Gate::new(1, 4);
        let (_tx, rx) = channel();
        let mut stream = MessageStream::new(rx, &gate, 100, 0);
        // the clock is passed in: a sleeping test would depend on how the runner schedules it
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        // busy: 10 ms between pages, nothing waited
        for k in 0..5 {
            stream.adapt(Duration::ZERO, at(k * 10));
        }
        assert_eq!(gate.active(), 1);
        // starved: half of the time between pages went to waiting
        for k in 5..7 {
            stream.adapt(Duration::from_millis(5), at(k * 10));
        }
        assert_eq!(gate.active(), 2);
    }

    #[test]
    fn workers_wait_for_room_and_wake_up_when_halted() {
        // one working worker: two pages ahead of the consumer
        let gate = Gate::new(1, 1);
        assert!(gate.wait_for_room(0));
        assert!(gate.wait_for_room(1));
        std::thread::scope(|s| {
            let waiter = s.spawn(|| gate.wait_for_room(2));
            std::thread::sleep(Duration::from_millis(20));
            assert!(
                !waiter.is_finished(),
                "page 2 must wait until page 0 is taken"
            );
            gate.set_taken(1);
            assert!(waiter.join().unwrap());
            let stuck = s.spawn(|| gate.wait_for_room(10));
            std::thread::sleep(Duration::from_millis(20));
            gate.halt();
            assert!(!stuck.join().unwrap());
        });
    }

    fn page(scanned: usize, ids: &[i64]) -> ParsedPage {
        ParsedPage {
            scanned,
            msgs: ids
                .iter()
                .map(|&local_id| ExportMsg {
                    local_id,
                    ..Default::default()
                })
                .collect(),
            fetch: Duration::ZERO,
            parse: Duration::ZERO,
        }
    }

    #[test]
    fn the_stream_puts_pages_back_in_order() {
        let gate = Gate::new(4, 4);
        let (tx, rx) = channel();
        for (i, ids) in [(2, &[5][..]), (0, &[1, 2]), (3, &[]), (1, &[3, 4])] {
            tx.send((i, Ok(page(ids.len(), ids)))).unwrap();
        }
        drop(tx);
        let mut stream = MessageStream::new(rx, &gate, 4, 5);
        let ids: Vec<i64> = stream.by_ref().map(|m| m.local_id).collect();
        assert_eq!(ids, [1, 2, 3, 4, 5]);
        assert!(stream.error.is_none());
        assert_eq!(*gate.taken.lock().unwrap(), 4);
    }

    #[test]
    fn the_stream_stops_on_a_failed_page_or_a_missing_one() {
        let gate = Gate::new(4, 4);
        let (tx, rx) = channel();
        tx.send((0, Ok(page(1, &[1])))).unwrap();
        tx.send((1, Err(AppError::native("synthetic read failure"))))
            .unwrap();
        tx.send((2, Ok(page(1, &[3])))).unwrap();
        let mut stream = MessageStream::new(rx, &gate, 3, 3);
        let ids: Vec<i64> = stream.by_ref().map(|m| m.local_id).collect();
        assert_eq!(ids, [1]);
        assert!(stream.error.is_some());

        // every worker gone while a page is still missing: not a complete export
        let (tx, rx) = channel();
        tx.send((1, Ok(page(1, &[2])))).unwrap();
        drop(tx);
        let mut stream = MessageStream::new(rx, &gate, 2, 2);
        assert_eq!(stream.by_ref().count(), 0);
        assert!(stream.error.is_some());
    }
}
