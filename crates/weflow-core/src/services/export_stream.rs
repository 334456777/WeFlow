//! 为导出读取会话：工作线程并行读取、解析各页，调用方按顺序消费消息。
//!
//! 消息游标打开后就知道范围内的全部消息及其顺序，所以第 `i` 页在读取之前就已确定。每个工作线程领取下一个页号，
//! 从数据库读出这一页，把其中的行变成导出用的消息；消费方再把各页按顺序排好。读取（SQLite、数据库页的解密、
//! 构造行）和解析都是 CPU 的工作，读取占大头，所以并行的是读取。
//! 数据库给每个工作线程一个自己的连接（见 `Wcdb::set_read_connections`），读同一个数据库文件时不必互相等待。
//!
//! 需要几个工作线程取决于导出：写起来慢的格式（ChatLab、JSON）两个就跟得上，轻的格式（TXT）需要更多。
//! 所以导出从 [`INITIAL_WORKERS`] 个开始，消费方等待页的时间超过它总时间的十分之一时就再唤醒一个，
//! 最多为 CPU 数减一（留一个给消费方），且不超过 [`MAX_WORKERS`]。多唤醒的一个没能让工作线程明显更快地交出页时，
//! 就不再唤醒（见 `PageStream::adapt`）：只收集消息的消费方（带媒体的导出）不论工作线程怎样都在等待，
//! 但超过某个数量后，更多的工作线程只是在争用同样的 CPU 和数据库。这样的消费方最多有 [`COLLECT_MAX_WORKERS`]
//! 个工作线程（见 [`collect_export_messages`]）。每个读取的工作线程都要占内存（它的数据库连接有页缓存，
//! 它读好的页要等消费方取走），所以用不上的工作线程一直休眠，从不打开连接。
//!
//! 工作线程还可以替消费方完成它那一页：条目互不依赖的导出让工作线程一并渲染（见 [`read_export_pages`]），
//! 消费方只管写出。
//!
//! 每有一个工作中的线程，读取最多可以超前消费方 [`PAGES_AHEAD_PER_WORKER`] 页，所以不论消费方或前面某一页多慢，
//! 内存都有上限。某一页失败、某个工作线程 panic 或消费方提前停止时，所有工作线程都停下；
//! 缺了一页的导出决不会当作完整的导出结束。

use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Condvar, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use weflow_native::msg_row::MsgRow;
use weflow_native::wcdb::Wcdb;

use super::MessageExportRequest;
use crate::error::{AppError, AppResult};
use crate::message::{CollectOptions, ExportMsg};

/// 每页的消息数（游标的一批）。
///
/// 导出把每一页按时间排序，所以 `sort_seq` 与 `create_time` 不一致时，页的大小会影响输出的顺序：保持不变。
const PAGE_SIZE: i32 = 2000;
/// 每个工作线程最多可以超前消费方当前所在页读取的页数。
const PAGES_AHEAD_PER_WORKER: usize = 2;
/// 一开始就读取的工作线程数（CPU 或页更少时取更少的）。
const INITIAL_WORKERS: usize = 2;
/// 导出自动唤醒的工作线程的上限。
const MAX_WORKERS: usize = 8;
/// 只收集消息的消费方的工作线程上限，这样的消费方是带媒体的导出：全部读完后才复制媒体、写文件。
///
/// 在一个有 20 万条消息的真实群聊上，两个工作线程与增长检查唤醒的七个收集得一样快，内存少用约 100 MiB。
const COLLECT_MAX_WORKERS: usize = 2;
/// 消费方每取多少页判断一次等待是否太久（或者最后唤醒的工作线程带来了多少提升）。
const PAGES_PER_CHECK: usize = 3;
/// 固定工作线程数（至少为 1），不自动调整，用于测量。
const WORKERS_ENV: &str = "WEFLOW_EXPORT_WORKERS";

/// 工作线程把解析好的每一页变成交给消费方的结果。
pub(super) type Finish<'f, T> = dyn FnMut(Vec<ExportMsg>) -> T + 'f;
/// 用一个新的 [`Finish`] 运行给定的 `body`。
///
/// 每个读取的工作线程在自己的线程上调用一次，所以 [`Finish`] 可以持有不能共享的东西（如带自己的名字缓存的导出器）。
pub(super) type WithFinisher<'s, T> = dyn Fn(&mut dyn FnMut(&mut Finish<'_, T>)) + Sync + 's;

/// 对会话的消息（从旧到新）运行 `f`，`f` 消费的同时读取和解析后面的消息。
pub(super) fn read_export_stream<R>(
    wcdb: &Wcdb,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
    f: impl FnOnce(&mut dyn Iterator<Item = ExportMsg>) -> R,
) -> AppResult<R> {
    read_stream(wcdb, req, opts, MAX_WORKERS, f)
}

/// 给只收集消息的消费方的会话消息（从旧到新）：最多 [`COLLECT_MAX_WORKERS`] 个工作线程读取，
/// 更多也不会收集得更快。
pub(super) fn collect_export_messages(
    wcdb: &Wcdb,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
) -> AppResult<Vec<ExportMsg>> {
    read_stream(wcdb, req, opts, COLLECT_MAX_WORKERS, |stream| {
        stream.collect()
    })
}

fn read_stream<R>(
    wcdb: &Wcdb,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
    max_workers: usize,
    f: impl FnOnce(&mut dyn Iterator<Item = ExportMsg>) -> R,
) -> AppResult<R> {
    let as_is: &WithFinisher<'_, Vec<ExportMsg>> = &|body| body(&mut |msgs| msgs);
    read_pages(wcdb, req, opts, max_workers, as_is, |pages| {
        f(&mut pages.flatten())
    })
}

/// 对会话的各页（从旧到新）运行 `f`：每一页在工作线程上读取、解析，并由 `with_finisher` 完成，
/// 同时 `f` 消费前面的页。
pub(super) fn read_export_pages<T: Send, R>(
    wcdb: &Wcdb,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
    with_finisher: &WithFinisher<'_, T>,
    f: impl FnOnce(&mut dyn Iterator<Item = T>) -> R,
) -> AppResult<R> {
    read_pages(wcdb, req, opts, MAX_WORKERS, with_finisher, f)
}

fn read_pages<T: Send, R>(
    wcdb: &Wcdb,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
    max_workers: usize,
    with_finisher: &WithFinisher<'_, T>,
    f: impl FnOnce(&mut dyn Iterator<Item = T>) -> R,
) -> AppResult<R> {
    let clamp = |t: i64| crate::num::saturating_i32(t.max(0));
    let (begin, finish) = (req.start.map_or(0, clamp), req.end.map_or(0, clamp));
    let cursor = wcdb
        .open_export_cursor(&req.session_id, PAGE_SIZE, begin, finish)
        .map_err(|e| AppError::native(format!("{e:#}")))?;
    let cpus = std::thread::available_parallelism().map_or(2, |n| n.get());
    let workers = (cpus, max_workers);
    let result = read_cursor(wcdb, cursor, req, opts, workers, with_finisher, f);
    let _ = wcdb.close_message_cursor(cursor);
    result
}

/// 共 `pages` 页时的（一开始读取的工作线程数，最多的工作线程数）。
///
/// 设了 [`WORKERS_ENV`] 时两者都取它；
/// 否则从 [`INITIAL_WORKERS`] 开始，最多为 CPU 数减一（留一个给格式化和写出的消费方），且不超过 `max_workers`。
/// 两者都不超过页数。
fn worker_counts(
    pages: usize,
    configured: Option<&str>,
    cpus: usize,
    max_workers: usize,
) -> (usize, usize) {
    let fit = |n: usize| n.min(pages).max(1);
    match configured
        .and_then(|v| v.trim().parse::<usize>().ok())
        .filter(|n| *n > 0)
    {
        Some(n) => (fit(n), fit(n)),
        None => {
            let most = fit(cpus.saturating_sub(1).clamp(1, max_workers.max(1)));
            (INITIAL_WORKERS.min(most), most)
        }
    }
}

/// 按 CPU 数和工作线程上限算出工作线程数（见 [`worker_counts`]），读取已打开的 `cursor`。
fn read_cursor<T: Send, R>(
    wcdb: &Wcdb,
    cursor: i64,
    req: &MessageExportRequest,
    opts: &CollectOptions<'_>,
    (cpus, max_workers): (usize, usize),
    with_finisher: &WithFinisher<'_, T>,
    f: impl FnOnce(&mut dyn Iterator<Item = T>) -> R,
) -> AppResult<R> {
    let started = Instant::now();
    let stale_before = wcdb.stale_reruns();
    let (total, batch) = wcdb
        .message_cursor_len(cursor)
        .map_err(|e| AppError::native(format!("{e:#}")))?;
    let pages = total.div_ceil(batch.max(1));
    let (initial, workers) = worker_counts(
        pages,
        std::env::var(WORKERS_ENV).ok().as_deref(),
        cpus,
        max_workers,
    );
    // 工作线程读取时才打开连接，休眠的工作线程不占连接
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
            // 表情描述的表要对整个数据库查一次：现在就开始，与最初几页同时进行
            scope.spawn(|| captions.get_or_init(load_captions));
        }
        let handles: Vec<_> = (0..workers)
            .map(|index| {
                let tx = tx.clone();
                let (gate, source) = (&gate, &page_source);
                scope.spawn(move || work(index, gate, source, with_finisher, pages, tx))
            })
            .collect();
        drop(tx);
        let mut stream = PageStream::new(rx, &gate, pages, total);
        // 消费方 panic 时也要停下工作线程：作用域在 panic 继续传播前要等它们结束，而从未被唤醒的工作线程会一直等下去
        let consumer = HaltOnPanic(&gate);
        let result = f(&mut stream);
        drop(consumer);
        gate.halt(); // 消费方结束（或放弃）了：工作线程在下一页时停下
        let PageStream {
            rx, error, stats, ..
        } = stream;
        drop(rx); // 仍在读取的工作线程发送失败后停下
        let panicked = handles.into_iter().any(|h| h.join().is_err());
        if panicked {
            return Err(AppError::runtime("message reader thread panicked"));
        }
        if let Some(e) = error {
            return Err(e);
        }
        let stale_reruns = wcdb.stale_reruns().saturating_sub(stale_before);
        stats.log(gate.active(), pages, total, stale_reruns, started.elapsed());
        Ok(result)
    })
}

/// 工作线程读取、解析一页所需的东西。
struct PageSource<'a> {
    wcdb: &'a Wcdb,
    cursor: i64,
    req: &'a MessageExportRequest,
    opts: &'a CollectOptions<'a>,
    captions: &'a OnceLock<HashMap<String, String>>,
    load_captions: &'a (dyn Fn() -> HashMap<String, String> + Sync),
}

/// 读取并解析好的一页。
struct ParsedPage {
    /// 游标返回的行数（按时间和发送者筛选之前），进度按它计数。
    scanned: usize,
    msgs: Vec<ExportMsg>,
    fetch: Duration,
    parse: Duration,
}

/// 消费方拿到的一页：已在工作线程上完成。
struct Page<T> {
    scanned: usize,
    payload: T,
    fetch: Duration,
    parse: Duration,
    finish: Duration,
}

type PageResult<T> = (usize, AppResult<Page<T>>);

impl PageSource<'_> {
    fn read(&self, page: usize) -> AppResult<ParsedPage> {
        let t = Instant::now();
        // 类型化的行（#491）：它们的文字移进消息里
        let rows = self
            .wcdb
            .fetch_message_page(self.cursor, page)
            .map_err(|e| AppError::native(format!("{e:#}")))?;
        let fetch = t.elapsed();
        let scanned = rows.len();
        let (start, end) = (self.req.start, self.req.end);
        let sender = self
            .opts
            .sender_filter
            .map(str::trim)
            .filter(|f| !f.is_empty());
        let rows: Vec<MsgRow> = rows
            .into_iter()
            .filter(|m| {
                let ts = crate::message::msg_row_timestamp(m);
                start.is_none_or(|s| ts >= s) && end.is_none_or(|e| ts <= e)
            })
            // 解析前先去掉其他发送者的行
            .filter(|row| {
                sender.is_none_or(|f| {
                    crate::message::transfer::is_same_wxid(
                        &crate::message::msg_row_sender(
                            row,
                            self.opts.session_id,
                            self.opts.my_wxid,
                        ),
                        f,
                    )
                })
            })
            .collect();
        let mut msgs = crate::message::collect_rows(rows, self.opts);
        if msgs.iter().any(is_sticker) {
            let table = self.captions.get_or_init(self.load_captions);
            attach_emoji_captions(table, &mut msgs);
        }
        Ok(ParsedPage {
            scanned,
            msgs,
            fetch,
            parse: t.elapsed() - fetch,
        })
    }
}

/// 是否是表情：普通的表情消息，或类型为 8 的 app 消息（作为附件发送的表情）。
fn is_sticker(m: &ExportMsg) -> bool {
    m.local_type == 47 || m.sticker_md5.is_some()
}

/// 查出每个表情的描述，写进 `emoji_caption`，导出显示为 `[表情：caption]`。
///
/// 描述取用户自己给表情写的文字，没有时取商店表情的中文描述。对应表（md5 → 描述）一次整个读出，
/// 见 `emoticon_captions`。
fn attach_emoji_captions(table: &HashMap<String, String>, messages: &mut [ExportMsg]) {
    for m in messages.iter_mut() {
        let md5 = match m.local_type {
            47 => m.emoji_md5.clone(),
            _ => m.sticker_md5.clone(),
        };
        let Some(md5) = md5 else { continue };
        m.emoji_caption = table
            .get(&md5.to_lowercase())
            .map(|c| c.trim().to_string())
            .filter(|c| !c.is_empty());
    }
}

/// 第 `index` 个工作线程：导出需要它时，不断领取、读取、解析并完成下一个没人领的页，直到没有剩下的页或导出停止。
fn work<T>(
    index: usize,
    gate: &Gate,
    source: &PageSource<'_>,
    with_finisher: &WithFinisher<'_, T>,
    pages: usize,
    tx: Sender<PageResult<T>>,
) {
    let _halt = HaltOnPanic(gate);
    if !gate.wait_until_needed(index) {
        return;
    }
    with_finisher(&mut |finish| {
        while !gate.halted() {
            let page = gate.next.fetch_add(1, Ordering::Relaxed);
            if page >= pages || !gate.wait_for_room(page) {
                break;
            }
            let done = source.read(page).map(|p| {
                let t = Instant::now();
                let payload = finish(p.msgs);
                Page {
                    scanned: p.scanned,
                    payload,
                    fetch: p.fetch,
                    parse: p.parse,
                    finish: t.elapsed(),
                }
            });
            let failed = done.is_err();
            gate.delivered.fetch_add(1, Ordering::Relaxed);
            if tx.send((page, done)).is_err() || failed {
                gate.halt();
                break;
            }
        }
    });
}

/// 工作线程与消费方之间的协调。
struct Gate {
    /// 下一个要分出去的页号。
    next: AtomicUsize,
    /// 消费方到目前为止取走的页数。
    taken: Mutex<usize>,
    moved: Condvar,
    halted: AtomicBool,
    /// 允许读取的工作线程数，其余的休眠。
    active: AtomicUsize,
    /// 自消费方上次查看以来工作线程交出的页数（按发送的时刻计，不论顺序），反映它们读得多快。
    ///
    /// 消费方取走的页则不同：要等最早的那一页，而且来自超前读好的部分。
    delivered: AtomicUsize,
    /// 工作线程的总数。
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
            delivered: AtomicUsize::new(0),
            workers,
        }
    }

    fn halted(&self) -> bool {
        self.halted.load(Ordering::Acquire)
    }

    fn active(&self) -> usize {
        self.active.load(Ordering::Acquire)
    }

    /// 在已取走的页之外还可以领取的页数。
    fn ahead(&self) -> usize {
        self.active() * PAGES_AHEAD_PER_WORKER
    }

    /// 再唤醒一个工作线程；全部都已在读取时返回 false。
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

    /// 等到第 `index` 个工作线程可以读取；导出先停止时返回 false。
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

    /// 停止工作线程，并唤醒所有在等待的工作线程（等空位的和休眠的）。
    fn halt(&self) {
        self.halted.store(true, Ordering::Release);
        let _taken = self.taken.lock().unwrap_or_else(PoisonError::into_inner);
        self.moved.notify_all();
    }

    /// 等到 `page` 离消费方足够近；期间导出停止时返回 false。
    ///
    /// 页是按顺序领取的，所以消费方正在等的那一页从不在这里等待。
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

/// 持有它的线程（工作线程或消费方）panic 时停止导出，免得其他工作线程一直等待不会再来的空位或唤醒。
struct HaltOnPanic<'a>(&'a Gate);

impl Drop for HaltOnPanic<'_> {
    fn drop(&mut self) {
        if std::thread::panicking() {
            self.0.halt();
        }
    }
}

/// 各阶段花费的时间，所有工作线程的总和。
#[derive(Default)]
struct Stats {
    fetch: Duration,
    parse: Duration,
    /// 在工作线程上渲染的时间（由工作线程渲染条目的导出）。
    finish: Duration,
    /// 消费方等待下一页的时间，其余时间用于格式化和写出。
    starved: Duration,
}

impl Stats {
    /// 设 `RUST_LOG=weflow::export=debug` 时写进日志。
    fn log(&self, workers: usize, pages: usize, total: usize, stale: usize, elapsed: Duration) {
        tracing::debug!(
            target: "weflow::export",
            workers,
            pages,
            messages = total,
            // 因读取期间微信改了数据库而重新运行的页读取次数
            stale_reruns = stale,
            elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            fetch_ms = u64::try_from(self.fetch.as_millis()).unwrap_or(u64::MAX),
            parse_ms = u64::try_from(self.parse.as_millis()).unwrap_or(u64::MAX),
            render_ms = u64::try_from(self.finish.as_millis()).unwrap_or(u64::MAX),
            consumer_waiting_ms = u64::try_from(self.starved.as_millis()).unwrap_or(u64::MAX),
            "message export read"
        );
    }
}

/// 按页序排列的完成的页；某一页失败时提前结束（见 `error`）。
struct PageStream<'a, T> {
    rx: Receiver<PageResult<T>>,
    gate: &'a Gate,
    pages: usize,
    next_page: usize,
    /// 比前面的页先到的页。
    pending: BTreeMap<usize, Page<T>>,
    error: Option<AppError>,
    total: usize,
    scanned: usize,
    stats: Stats,
    /// 消费方取上一页的时刻；`window` 是自上次检查以来取走的页数、经过的时间和等待的时间。
    last_take: Option<Instant>,
    window: (usize, Duration, Duration),
    growth: Growth,
}

/// 导出是否还唤醒工作线程。
#[derive(Clone, Copy, Debug, PartialEq)]
enum Growth {
    /// 第一次检查：工作线程同时开始并超前读取，页来得比之后快。只测量，然后转为 `Open`。
    Warming,
    /// 消费方等待时再唤醒一个工作线程。
    Open,
    /// 刚唤醒了一个工作线程；下一次检查（每个工作中的线程一页）留给它启动（打开连接、预热页缓存），不作评判。
    /// `before` 是唤醒它之前每交出一页的时间。
    Starting { before: Duration },
    /// 再下一次检查（每个工作中的线程两页）看出新的工作线程带来了多少提升。
    Judging { before: Duration },
    /// 最后唤醒的工作线程不划算：不再唤醒。
    Stopped,
}

impl<'a, T> PageStream<'a, T> {
    fn new(rx: Receiver<PageResult<T>>, gate: &'a Gate, pages: usize, total: usize) -> Self {
        Self {
            rx,
            gate,
            pages,
            next_page: 0,
            pending: BTreeMap::new(),
            error: None,
            total,
            scanned: 0,
            stats: Stats::default(),
            last_take: None,
            window: Default::default(),
            growth: Growth::Warming,
        }
    }

    /// 自上次检查以来消费方等待的时间超过十分之一时，再唤醒一个工作线程，直到唤醒的工作线程不再划算。
    ///
    /// 从 n - 1 个增加到 n 个工作线程，每交出一页的时间最多缩短 1/n；缩短得不到这个值的一半时，
    /// 更多的读取线程不会读得更快（CPU 或数据库已经忙不过来），只会多占内存。等待第一页的时间是读一页本身要花的时间，
    /// 不说明工作线程太少。`now` 是取走这一页的时刻。
    fn adapt(&mut self, waited: Duration, now: Instant) {
        match self.last_take.replace(now) {
            Some(last) => {
                self.window.0 += 1;
                self.window.1 += now - last;
                self.window.2 += waited;
            }
            // 从第一页开始计时：在它之前交出的页不算
            None => {
                self.gate.delivered.store(0, Ordering::Relaxed);
            }
        }
        let (pages, time, wait) = self.window;
        let check_after = match self.growth {
            Growth::Starting { .. } => self.gate.active(),
            Growth::Judging { .. } => 2 * self.gate.active(),
            _ => PAGES_PER_CHECK,
        };
        if pages < check_after.max(PAGES_PER_CHECK) {
            return;
        }
        let delivered = self.gate.delivered.swap(0, Ordering::Relaxed);
        let per_page = time / u32::try_from(delivered.max(1)).unwrap_or(u32::MAX);
        tracing::trace!(
            target: "weflow::export",
            workers = self.gate.active(),
            taken = pages,
            delivered,
            window_us = u64::try_from(time.as_micros()).unwrap_or(u64::MAX),
            waited_us = u64::try_from(wait.as_micros()).unwrap_or(u64::MAX),
            per_page_us = u64::try_from(per_page.as_micros()).unwrap_or(u64::MAX),
            growth = ?self.growth,
            "worker check"
        );
        self.window = Default::default();
        self.growth = match self.growth {
            Growth::Warming => {
                self.growth = Growth::Open;
                return;
            }
            Growth::Starting { before } => Growth::Judging { before },
            Growth::Judging { before } => {
                let gain = before.saturating_sub(per_page);
                let possible =
                    before / u32::try_from(self.gate.active().max(1)).unwrap_or(u32::MAX);
                if gain * 2 < possible {
                    Growth::Stopped
                } else {
                    Growth::Open
                }
            }
            other => other,
        };
        if self.growth == Growth::Open && wait * 10 > time && self.gate.grow() {
            self.growth = Growth::Starting { before: per_page };
        }
    }

    fn take(&mut self, page: Page<T>, waited: Duration) -> T {
        self.adapt(waited, Instant::now());
        self.next_page += 1;
        self.gate.set_taken(self.next_page);
        self.scanned += page.scanned;
        self.stats.fetch += page.fetch;
        self.stats.parse += page.parse;
        self.stats.finish += page.finish;
        crate::output::progress(
            "messages",
            "reading messages",
            self.scanned,
            self.total.max(self.scanned),
        );
        page.payload
    }
}

impl<T> Iterator for PageStream<'_, T> {
    type Item = T;

    fn next(&mut self) -> Option<T> {
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
        let page = self.pending.remove(&self.next_page)?;
        Some(self.take(page, waited))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use weflow_native::msg::Order;
    use weflow_native::msg_row::Columns;

    #[test]
    fn worker_counts_follow_cpus_pages_and_the_override() {
        // 从两个开始，最多增长到 CPU 数减一（留一个给消费方），有上限，且不超过页数
        let most = MAX_WORKERS;
        assert_eq!(
            worker_counts(100, None, 16, most),
            (INITIAL_WORKERS, MAX_WORKERS)
        );
        assert_eq!(worker_counts(100, None, 4, most), (2, 3));
        assert_eq!(worker_counts(100, None, 2, most), (1, 1));
        assert_eq!(worker_counts(100, None, 1, most), (1, 1));
        assert_eq!(worker_counts(2, None, 16, most), (2, 2));
        assert_eq!(worker_counts(1, None, 16, most), (1, 1));
        assert_eq!(worker_counts(0, None, 16, most), (1, 1));
        // 覆盖值固定线程数，可以超过 CPU 数但不超过页数；无效的值被忽略
        assert_eq!(worker_counts(100, Some(" 12 "), 4, most), (12, 12));
        assert_eq!(worker_counts(3, Some("12"), 4, most), (3, 3));
        assert_eq!(worker_counts(100, Some("0"), 4, most), (2, 3));
        assert_eq!(worker_counts(100, Some("many"), 4, most), (2, 3));
    }

    #[test]
    fn a_collecting_consumer_gets_fewer_workers_unless_overridden() {
        let most = COLLECT_MAX_WORKERS;
        assert_eq!(worker_counts(100, None, 16, most), (2, 2));
        assert_eq!(worker_counts(100, None, 2, most), (1, 1));
        assert_eq!(worker_counts(1, None, 16, most), (1, 1));
        // 覆盖值用于测量：它仍然固定线程数
        assert_eq!(worker_counts(100, Some("4"), 16, most), (4, 4));
    }

    #[test]
    fn a_consumer_that_panics_stops_the_sleeping_workers() {
        use std::panic::{catch_unwind, AssertUnwindSafe};
        use weflow_native::fixture::{ContactSpec, Fixture, MsgSpec, SessionSpec, T0};

        let root = weflow_native::fixture::TestDir::new("weflow-consumer-panic");
        let fixture = Fixture::new(&root, "wxid_me_ab12");
        fixture.session_db(&[SessionSpec {
            username: "wxid_bob",
            summary: "",
            last_timestamp: T0 + 20_000,
            unread: 0,
            last_msg_type: 1,
        }]);
        fixture.contact_db(&[ContactSpec::new("wxid_bob", 1, "Bob")], &[]);
        // 十个游标页：够八个工作线程用，其中只有前两个在读取
        let msgs: Vec<MsgSpec> = (1..=20_000)
            .map(|i| MsgSpec::text(i, "wxid_bob", T0 + i, "hi"))
            .collect();
        fixture.message_shard(0, &[("wxid_bob", msgs)]);
        let account = fixture.db_storage().parent().unwrap().to_path_buf();
        let key = fixture.key_hex();

        let (done, finished) = channel();
        std::thread::spawn(move || {
            let mut wcdb = Wcdb::new();
            wcdb.open(&account, &key, Some("wxid_me_ab12")).unwrap();
            let req = MessageExportRequest {
                session_id: "wxid_bob".into(),
                format: "txt".into(),
                start: None,
                end: None,
                sender: None,
                display_pref: crate::export_msg::names::DisplayPref::Remark,
                excel_compact: false,
            };
            let opts = CollectOptions {
                session_id: "wxid_bob",
                my_wxid: "wxid_me",
                start: None,
                end: None,
                sender_filter: None,
            };
            let cursor = wcdb
                .open_message_cursor(
                    "wxid_bob",
                    PAGE_SIZE,
                    Order::OldestFirst,
                    0,
                    0,
                    Columns::All,
                )
                .unwrap();
            let as_is: &WithFinisher<'_, Vec<ExportMsg>> = &|body| body(&mut |msgs| msgs);
            // 16 个 CPU：两个工作线程读取，六个休眠，直到消费方需要它们。消费方比这两个慢，所以其余的都不会被唤醒，
            // 而且它在最后一页之后才失败（就像最后一次保存失败的写出方）：这时两个工作线程已经结束，
            // 没有失败的发送能让休眠的那些停下。
            let panicked = catch_unwind(AssertUnwindSafe(|| {
                read_cursor(
                    &wcdb,
                    cursor,
                    &req,
                    &opts,
                    (16, MAX_WORKERS),
                    as_is,
                    |pages| {
                        for _ in pages {
                            std::thread::sleep(Duration::from_millis(100));
                        }
                        panic!("the consumer failed after the last page");
                    },
                )
            }))
            .is_err();
            let _ = done.send(panicked);
        });
        // panic 让 `Gate` 停止，从而唤醒休眠的工作线程：导出结束，并把 panic 传下去
        let panicked = finished.recv_timeout(Duration::from_secs(60));
        assert_eq!(panicked, Ok(true), "the export hung or swallowed the panic");
    }

    #[test]
    fn pages_read_after_wechat_changed_the_database_are_read_again() {
        use weflow_native::fixture::{ContactSpec, Fixture, MsgSpec, SessionSpec, T0};

        let root = weflow_native::fixture::TestDir::new("weflow-stale-export");
        let fixture = Fixture::new(&root, "wxid_me_ab12");
        fixture.session_db(&[SessionSpec {
            username: "wxid_bob",
            summary: "",
            last_timestamp: T0 + 40_000,
            unread: 0,
            last_msg_type: 1,
        }]);
        fixture.contact_db(&[ContactSpec::new("wxid_bob", 1, "Bob")], &[]);
        // 二十个游标页，数据库远大于导出的页缓存
        let text = |i: i64| format!("message {i} {}", "x".repeat(1000));
        let msgs: Vec<MsgSpec> = (1..=40_000)
            .map(|i| MsgSpec::text(i, "wxid_bob", T0 + i, &text(i)))
            .collect();
        fixture.message_shard(0, &[("wxid_bob", msgs)]);
        let shard = fixture.db_storage().join("message/message_0.db");

        let mut wcdb = Wcdb::new();
        let account = fixture.db_storage().parent().unwrap().to_path_buf();
        wcdb.open(&account, &fixture.key_hex(), Some("wxid_me_ab12"))
            .unwrap();
        let req = MessageExportRequest {
            session_id: "wxid_bob".into(),
            format: "txt".into(),
            start: None,
            end: None,
            sender: None,
            display_pref: crate::export_msg::names::DisplayPref::Remark,
            excel_compact: false,
        };
        let opts = CollectOptions {
            session_id: "wxid_bob",
            my_wxid: "wxid_me",
            start: None,
            end: None,
            sender_filter: None,
        };
        let as_is: &WithFinisher<'_, Vec<ExportMsg>> = &|body| body(&mut |msgs| msgs);
        let mut changes = 0u64;
        // 两个工作线程和一个只收集的消费方：工作线程是瓶颈，所以微信写入时它们多半正在查询。
        // 改动之后开始的查询打开新的快照；改动时正在读取的查询失败并重新运行。
        // 在繁忙的单个 CPU 上，改动发生时可能没有查询在运行：这时再检查一次导出，最多五次
        for _ in 0..5 {
            let cursor = wcdb
                .open_message_cursor(
                    "wxid_bob",
                    PAGE_SIZE,
                    Order::OldestFirst,
                    0,
                    0,
                    Columns::All,
                )
                .unwrap();
            let before = wcdb.stale_reruns();
            let got = read_cursor(&wcdb, cursor, &req, &opts, (16, 2), as_is, |pages| {
                let mut got: Vec<(i64, String)> = Vec::new();
                for (n, page) in pages.enumerate() {
                    if n % 3 == 0 && n < 15 {
                        changes += 1;
                        std::fs::File::options()
                            .write(true)
                            .open(&shard)
                            .unwrap()
                            .set_modified(
                                std::time::SystemTime::now() + Duration::from_secs(3600 + changes),
                            )
                            .unwrap();
                    }
                    got.extend(page.into_iter().map(|m| (m.local_id, m.content)));
                }
                got
            })
            .unwrap();
            let _ = wcdb.close_message_cursor(cursor);
            assert_eq!(got.len(), 40_000, "every message once");
            assert!(
                got.iter()
                    .zip(1..)
                    .all(|((id, content), i)| *id == i && *content == text(i)),
                "in order and unchanged"
            );
            if wcdb.stale_reruns() > before {
                break;
            }
        }
        let reruns = wcdb.stale_reruns();
        assert!(reruns > 0, "no query was reading when the database changed");
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

    /// 每隔 `every` 毫秒取一页，共取 `pages` 页（每页都恰好及时交出），每页之前等待 `waited` 毫秒。
    ///
    /// 时钟由参数传入：靠休眠的测试会受运行机器调度的影响。
    fn feed(
        stream: &mut PageStream<'_, Vec<ExportMsg>>,
        clock: &mut Instant,
        pages: usize,
        every: u64,
        waited: u64,
    ) {
        for _ in 0..pages {
            *clock += Duration::from_millis(every);
            stream.gate.delivered.fetch_add(1, Ordering::Relaxed);
            stream.adapt(Duration::from_millis(waited), *clock);
        }
    }

    #[test]
    fn a_waiting_consumer_wakes_more_workers_a_busy_one_does_not() {
        let gate = Gate::new(1, 4);
        let (_tx, rx) = channel();
        let mut stream = PageStream::new(rx, &gate, 100, 0);
        let mut clock = Instant::now();
        // 忙：每页 10 毫秒，没有等待（多一页：第一页只是开始计时）
        feed(&mut stream, &mut clock, 1 + 2 * PAGES_PER_CHECK, 10, 0);
        assert_eq!(gate.active(), 1);
        // 饥饿：一半的时间在等待
        feed(&mut stream, &mut clock, PAGES_PER_CHECK, 10, 5);
        assert_eq!(gate.active(), 2);
    }

    #[test]
    fn workers_stop_growing_once_one_more_does_not_pay_off() {
        let gate = Gate::new(1, 8);
        let (_tx, rx) = channel();
        let mut stream = PageStream::new(rx, &gate, 1000, 0);
        let mut clock = Instant::now();
        feed(&mut stream, &mut clock, 1 + 2 * PAGES_PER_CHECK, 12, 6);
        assert_eq!(gate.active(), 2);
        // 新的工作线程启动（每个工作线程一页，至少一次检查的页数：不作评判），之后在每个工作线程两页的检查中，
        // 每页的时间减半：已是最好的情况，所以继续增长
        feed(&mut stream, &mut clock, PAGES_PER_CHECK.max(2), 12, 6);
        feed(&mut stream, &mut clock, PAGES_PER_CHECK.max(2 * 2), 6, 3);
        assert_eq!(gate.active(), 3);
        // 第三个工作线程没有缩短每页 6 毫秒的时间（最多可以缩短三分之一）：它是最后一个
        feed(&mut stream, &mut clock, PAGES_PER_CHECK.max(3), 6, 3);
        feed(&mut stream, &mut clock, PAGES_PER_CHECK.max(2 * 3), 6, 3);
        assert_eq!(stream.growth, Growth::Stopped);
        // 此后不论消费方等待多久
        feed(&mut stream, &mut clock, 10 * PAGES_PER_CHECK, 6, 5);
        assert_eq!(gate.active(), 3);
    }

    #[test]
    fn workers_wait_for_room_and_wake_up_when_halted() {
        // 一个工作中的线程：可以超前消费方两页
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

    fn page(scanned: usize, ids: &[i64]) -> Page<Vec<ExportMsg>> {
        Page {
            scanned,
            payload: ids
                .iter()
                .map(|&local_id| ExportMsg {
                    local_id,
                    ..Default::default()
                })
                .collect(),
            fetch: Duration::ZERO,
            parse: Duration::ZERO,
            finish: Duration::ZERO,
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
        let mut stream = PageStream::new(rx, &gate, 4, 5);
        let ids: Vec<i64> = stream.by_ref().flatten().map(|m| m.local_id).collect();
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
        let mut stream = PageStream::new(rx, &gate, 3, 3);
        let ids: Vec<i64> = stream.by_ref().flatten().map(|m| m.local_id).collect();
        assert_eq!(ids, [1]);
        assert!(stream.error.is_some());

        // 还缺一页时所有工作线程都已退出：不是完整的导出
        let (tx, rx) = channel();
        tx.send((1, Ok(page(1, &[2])))).unwrap();
        drop(tx);
        let mut stream = PageStream::new(rx, &gate, 2, 2);
        assert_eq!(stream.by_ref().flatten().count(), 0);
        assert!(stream.error.is_some());
    }
}
