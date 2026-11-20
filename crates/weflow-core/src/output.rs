//! What a command prints: its JSON answer, events and warnings on stderr, and the progress bar. Lines that say what
//! is under way are in [`crate::status`].

use serde::Serialize;
use serde_json::Value;

use crate::colour::{Paint, Role, Stream};
use crate::error::ErrorPayload;
use crate::sync::MutexExt;

/// The JSON answer of a command: `{ "success": true, "data": … }` or `{ "success": false, "error": … }`.
#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum CliResponse<T: Serialize> {
    /// The command did its work.
    Success {
        /// Always true.
        success: bool,
        /// What it answered; left out when it answers nothing.
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<T>,
    },
    /// The command failed.
    Failure {
        /// Always false.
        success: bool,
        /// What went wrong.
        error: ErrorPayload,
    },
}

/// The answer of a command that did its work.
pub fn success<T: Serialize>(data: T) -> CliResponse<T> {
    CliResponse::Success {
        success: true,
        data: Some(data),
    }
}

/// The answer of a command that failed.
pub fn failure(error: ErrorPayload) -> CliResponse<Value> {
    CliResponse::Failure {
        success: false,
        error,
    }
}

static JSON_OUTPUT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// `--json`: print JSON (results on stdout, events on stderr) instead of text for people.
pub fn set_json_output(on: bool) {
    JSON_OUTPUT.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether this run prints JSON (see [`set_json_output`]).
pub fn json_output() -> bool {
    JSON_OUTPUT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Writes `text` to stdout and flushes it. Output that nothing reads any more is dropped without an error, where
/// `print!` would panic: the reader of a pipe has exited (`weflow chat sessions | head -1`), or the terminal hung up,
/// as when it is closed while `weflow serve` runs in it (#989).
///
/// # Errors
/// When stdout cannot take the text for another reason, such as a full disk under `> file`.
pub fn print(text: &str) -> std::io::Result<()> {
    write_to(&mut std::io::stdout().lock(), text)
}

/// [`print()`] on stderr.
///
/// # Errors
/// As for [`print()`].
pub fn eprint(text: &str) -> std::io::Result<()> {
    write_to(&mut std::io::stderr().lock(), text)
}

/// Writes `text` to `out` and flushes it, dropping it when nothing reads `out` any more; see [`print()`].
fn write_to(out: &mut impl std::io::Write, text: &str) -> std::io::Result<()> {
    match out.write_all(text.as_bytes()).and_then(|()| out.flush()) {
        Err(err) if reader_gone(&err) => Ok(()),
        written => written,
    }
}

/// Whether a write failed because nothing reads the stream: a pipe without a reader, or on Unix a terminal that hung
/// up, which fails writes with EIO.
fn reader_gone(err: &std::io::Error) -> bool {
    #[cfg(unix)]
    let hung_up = err.raw_os_error() == Some(libc::EIO);
    #[cfg(not(unix))]
    let hung_up = false;
    hung_up || err.kind() == std::io::ErrorKind::BrokenPipe
}

/// A status event on stderr: one JSON line with `--json`, lines for people otherwise.
pub fn event(event: Value) {
    if json_output() {
        crate::status::break_line();
        // an event that cannot be written has nowhere else to go
        let _ = eprint(&format!("{event}\n"));
    } else {
        let text = human_event(&event, Paint::of(Stream::Stderr));
        if !text.is_empty() {
            crate::status::say(&text);
        }
    }
}

/// A warning on stderr: `warning: <message>` for people, with what to do on the lines after it; a `warning` event
/// with `--json`.
pub fn warn(message: &str) {
    event(serde_json::json!({ "type": "warning", "message": message }));
}

/// The text of `event` for people; empty for an event that is said another way (a pending line, the result).
fn human_event(event: &Value, paint: Paint) -> String {
    use crate::render::{labelled, render};
    let text = |key: &str| event[key].as_str().unwrap_or("").to_string();
    let enabled = |key: &str| event[key].as_bool() == Some(true);
    let field = |label: &str, value: &str| crate::render::field(label, value, paint);
    match event["type"].as_str().unwrap_or("") {
        "warning" | "key_status" => labelled(
            tr("warning:", "警告："),
            Role::Warning,
            &text("message"),
            paint,
        ),
        // the command shows a pending line until it stops
        "auto_download_started" => String::new(),
        "insight" => format!(
            "{}\n{}",
            paint.paint(Role::Strong, tr("Insight record", "洞察记录")),
            render(&event["record"], paint)
        ),
        "request" => {
            let status = event["status"].as_u64().unwrap_or(0);
            let role = match status {
                500.. => Role::Error,
                400..=499 => Role::Warning,
                _ => Role::Success,
            };
            format!(
                "{} {} {} {} {}\n",
                paint.paint(
                    Role::Muted,
                    &chrono::Local::now().format("%H:%M:%S").to_string()
                ),
                text("method"),
                text("path"),
                paint.paint(role, &status.to_string()),
                paint.paint(
                    Role::Muted,
                    &format!("{:.1} ms", event["ms"].as_f64().unwrap_or(0.0))
                )
            )
        }
        "server_started" => {
            let mut services = vec![tr("HTTP API", "HTTP API")];
            for (key, en, zh) in [
                ("insight", "insight", "洞察"),
                ("imageAutoDownload", "image auto download", "原图自动下载"),
            ] {
                if enabled(key) {
                    services.push(tr(en, zh));
                }
            }
            let mut out = labelled(
                tr("Server started:", "服务已启动："),
                Role::Success,
                &text("url"),
                paint,
            );
            out.push_str(&field(
                tr("Running:", "运行中："),
                &services.join(tr(", ", "、")),
            ));
            if let Some(token) = event["generatedToken"].as_str() {
                out.push_str(&field(tr("Access token:", "访问令牌："), token));
                out.push_str(&paint.paint(
                    Role::Muted,
                    tr(
                        "The token is made up for this run; set http_api_token or pass --api-token to keep one.",
                        "令牌只在本次运行有效；设置 http_api_token 或传入 --api-token 可固定令牌。",
                    ),
                ));
                out.push('\n');
            }
            if let Some(url) = event["uiUrl"].as_str() {
                out.push_str(&field(tr("Web UI:", "网页："), url));
            }
            // a WebUI being downloaded is said on a pending line of its own
            if event["ui"] == false && event["uiDownloading"].is_null() {
                out.push_str(&paint.paint(
                    Role::Muted,
                    tr(
                        "API only, no WebUI. Pass --ui-dir with the folder of a built WebUI to serve one.",
                        "只提供 API，没有网页。用 --ui-dir 指定构建好的网页目录即可提供网页。",
                    ),
                ));
                out.push('\n');
            }
            out
        }
        "webui_ready" => field(tr("Web UI:", "网页："), &text("uiUrl")),
        // after the pending line that ends with `failed`
        "webui_unavailable" => {
            let then = if enabled("kept") {
                tr(
                    "The version already here is shown.",
                    "继续使用已有的网页版本。",
                )
            } else {
                tr("The API runs alone.", "只提供 API。")
            };
            let error = crate::locale::localize(text("error"));
            format!(
                "{}\n{then}\n",
                paint.paint(Role::Muted, &crate::render::reason_line(&error))
            )
        }
        _ => render(event, paint),
    }
}

/// How progress events are reported on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    /// Nothing.
    Off,
    /// One NDJSON event per line (`--progress`).
    Ndjson,
    /// A single-line progress bar that appears once a command has run for 5 seconds, only when
    /// stderr is a terminal.
    Auto,
}

/// Default number of seconds a command must run before the automatic progress bar appears.
const DEFAULT_BAR_DELAY_SECS: u64 = 5;

static BAR_DELAY_SECS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_BAR_DELAY_SECS);

/// Sets the delay before the automatic bar appears (`0` = from the start).
pub fn set_progress_delay(secs: u64) {
    BAR_DELAY_SECS.store(secs, std::sync::atomic::Ordering::Relaxed);
}

/// Parses a delay setting given as a number or a numeric string.
pub fn parse_delay(value: &str) -> Option<u64> {
    value.trim().parse::<u64>().ok()
}

struct BarState {
    mode: ProgressMode,
    started: std::time::Instant,
    stage: String,
    stage_started: std::time::Instant,
    last_draw: Option<std::time::Instant>,
    last_line: String,
    /// visible width of the last drawn line, to pad shorter updates instead of clearing the row
    last_width: usize,
    /// smoothed items per second of the current stage
    rate: Option<f64>,
    rate_sample: Option<(std::time::Instant, usize)>,
    drawn: bool,
    is_tty: bool,
}

fn bar_state() -> &'static std::sync::Mutex<BarState> {
    static STATE: std::sync::OnceLock<std::sync::Mutex<BarState>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        let now = std::time::Instant::now();
        std::sync::Mutex::new(BarState {
            mode: ProgressMode::Off,
            started: now,
            stage: String::new(),
            stage_started: now,
            last_draw: None,
            last_line: String::new(),
            last_width: 0,
            rate: None,
            rate_sample: None,
            drawn: false,
            is_tty: false,
        })
    })
}

/// Selects the progress mode and starts the clock for the automatic bar.
pub fn set_progress_mode(mode: ProgressMode) {
    use std::io::IsTerminal;
    let mut st = bar_state().lock_or_panic();
    st.mode = mode;
    st.started = std::time::Instant::now();
    st.is_tty = std::io::stderr().is_terminal();
}

use crate::locale::tr;

fn format_duration(secs: u64) -> String {
    if secs >= 3600 {
        format!("{}h{:02}m{:02}s", secs / 3600, secs % 3600 / 60, secs % 60)
    } else if secs >= 60 {
        format!("{}m{:02}s", secs / 60, secs % 60)
    } else {
        format!("{secs}s")
    }
}

/// One line of the bar; `total == 0` means the total is unknown (spinner, no percentage).
/// `eta_secs` is the (smoothed) remaining time, when known.
fn render_bar(
    message: &str,
    current: usize,
    total: usize,
    elapsed_secs: u64,
    eta_secs: Option<u64>,
    width: usize,
    tick: usize,
) -> String {
    if total == 0 {
        let spin = ['|', '/', '-', '\\'][tick % 4];
        let (processed, elapsed) = (tr("processed", "已处理"), tr("elapsed", "已用时"));
        return format!(
            "{message}  {spin} {current} {processed}  {elapsed} {}",
            format_duration(elapsed_secs)
        );
    }
    let frac = (current as f64 / total as f64).clamp(0.0, 1.0);
    #[expect(
        clippy::cast_possible_truncation,
        reason = "`frac` is in [0, 1], so both fit"
    )]
    let (filled, percent) = (
        (frac * width as f64).round() as usize,
        (frac * 100.0) as u32,
    );
    let bar: String = "█".repeat(filled) + &"░".repeat(width.saturating_sub(filled));
    let eta = match eta_secs {
        Some(e) if current < total => {
            format!("  {} {}", tr("remaining", "剩余"), format_duration(e))
        }
        _ => String::new(),
    };
    format!(
        "{message}  {bar} {percent}%  {current}/{total}  {} {}{eta}",
        tr("elapsed", "已用时"),
        format_duration(elapsed_secs)
    )
}

/// Cuts a line to `max` characters so it never wraps (a wrapped line cannot be redrawn in place).
fn fit_line(line: &str, max: usize) -> String {
    if max == 0 || line.chars().count() <= max {
        line.to_string()
    } else {
        line.chars().take(max).collect()
    }
}

fn terminal_columns() -> usize {
    terminal_size::terminal_size_of(std::io::stderr())
        .map(|(w, _)| w.0 as usize)
        .filter(|w| *w > 8)
        .unwrap_or(100)
}

/// [`progress`] of the formats of one export written side by side (#628): `(format, written)`. With `--progress` the
/// event also says which are written, as `formats`, for the WebUI to show them running at once.
pub fn progress_formats(formats: &[(String, bool)]) {
    const MESSAGE: &str = "writing formats";
    let total = formats.len();
    let done = formats.iter().filter(|(_, written)| *written).count();
    if !matches!(bar_state().lock_or_panic().mode, ProgressMode::Ndjson) {
        progress("formats", MESSAGE, done, total);
        return;
    }
    let lanes: Vec<_> = formats
        .iter()
        .map(|(name, written)| serde_json::json!({ "name": name, "done": written }))
        .collect();
    let event = serde_json::json!({
        "type": "progress", "stage": "formats", "message": crate::locale::localize(MESSAGE.to_string()),
        "current": done, "total": total, "percent": done * 100 / total.max(1), "formats": lanes,
    });
    let _ = eprint(&format!("{event}\n"));
}

/// Reports that `current` of `total` steps of `stage` are done, as the [`ProgressMode`] says: an NDJSON `progress`
/// event on stderr with `--progress`, else a progress bar on a terminal once the command has run for the delay set.
pub fn progress(stage: &str, message: &str, current: usize, total: usize) {
    let message = &crate::locale::localize(message.to_string());
    let mut st = bar_state().lock_or_panic();
    match st.mode {
        ProgressMode::Off => {}
        ProgressMode::Ndjson => {
            drop(st);
            #[expect(
                clippy::cast_possible_truncation,
                reason = "a percentage; `as` saturates should `current` pass `total`"
            )]
            let percent = if total > 0 {
                ((current as f64 / total as f64) * 100.0) as u8
            } else {
                0
            };
            let event = serde_json::json!({ "type": "progress", "stage": stage, "message": message, "current": current, "total": total, "percent": percent });
            let _ = eprint(&format!("{event}\n"));
        }
        ProgressMode::Auto => {
            use std::io::Write;
            let now = std::time::Instant::now();
            if st.stage != stage {
                st.stage = stage.to_string();
                st.stage_started = now;
                st.rate = None;
                st.rate_sample = None;
            }
            // smoothed rate (exponential average over ~1 s samples) so `remaining` does not jump around
            match st.rate_sample {
                Some((t, c)) if now.duration_since(t).as_millis() >= 1000 && current >= c => {
                    let inst = (current - c) as f64 / now.duration_since(t).as_secs_f64();
                    st.rate = Some(st.rate.map_or(inst, |r| r * 0.8 + inst * 0.2));
                    st.rate_sample = Some((now, current));
                }
                None => st.rate_sample = Some((now, current)),
                _ => {}
            }
            if !st.is_tty
                || now.duration_since(st.started).as_secs()
                    < BAR_DELAY_SECS.load(std::sync::atomic::Ordering::Relaxed)
            {
                return;
            }
            let done = total > 0 && current >= total;
            if !done
                && st
                    .last_draw
                    .is_some_and(|t| now.duration_since(t).as_millis() < 200)
            {
                return;
            }
            let elapsed = now.duration_since(st.stage_started).as_secs();
            #[expect(
                clippy::cast_possible_truncation,
                reason = "seconds left; `as` saturates"
            )]
            let eta = st
                .rate
                .filter(|r| *r > 0.0 && total > current)
                .map(|r| ((total - current) as f64 / r).round() as u64);
            let tick = (now.duration_since(st.started).as_millis() / 200) as usize;
            let cols = terminal_columns();
            let line = fit_line(
                &render_bar(message, current, total, elapsed, eta, 16, tick),
                cols.saturating_sub(1),
            );
            if line == st.last_line {
                return;
            }
            st.last_draw = Some(now);
            if !st.drawn {
                // the bar takes a line of its own, below an open pending line
                crate::status::break_line();
            }
            // one write: return to column 0, overwrite in place, pad over a longer previous line
            let width = line.chars().count();
            let pad = st.last_width.saturating_sub(width);
            let mut out = String::with_capacity(line.len() + pad + 2);
            out.push('\r');
            out.push_str(&line);
            out.extend(std::iter::repeat_n(' ', pad));
            if pad > 0 {
                out.extend(std::iter::repeat_n('\u{8}', pad));
            }
            let mut err = std::io::stderr().lock();
            let _ = err.write_all(out.as_bytes());
            let _ = err.flush();
            st.last_width = width;
            st.last_line = line;
            st.drawn = true;
        }
    }
}

/// Erases the automatic progress bar and ends an open pending line (call before printing the final result).
pub fn finish_progress() {
    crate::status::break_line();
    let mut st = bar_state().lock_or_panic();
    if st.drawn {
        use std::io::Write;
        // blank the row once and return to its start so the next output begins on a clean line
        let blank = " ".repeat(st.last_width);
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "\r{blank}\r");
        let _ = err.flush();
        st.drawn = false;
        st.last_line.clear();
        st.last_width = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_shows_percentage_and_eta() {
        let s = render_bar("exporting images", 4056, 7800, 227, Some(208), 16, 0);
        assert!(
            s.starts_with(
                "exporting images  ████████░░░░░░░░ 52%  4056/7800  elapsed 3m47s  remaining 3m28s"
            ),
            "{s}"
        );
    }

    #[test]
    fn unknown_totals_show_a_spinner() {
        let s = render_bar("scanning", 12, 0, 65, None, 16, 1);
        assert_eq!(s, "scanning  / 12 processed  elapsed 1m05s");
        assert!(!s.contains('%'));
    }

    #[test]
    fn delay_settings_parse() {
        assert_eq!(parse_delay(" 30 "), Some(30));
        assert_eq!(parse_delay("0"), Some(0));
        assert_eq!(parse_delay("abc"), None);
        assert_eq!(parse_delay("-1"), None);
    }

    #[test]
    fn long_lines_are_cut_to_the_terminal_width() {
        assert_eq!(fit_line("abcdef", 4), "abcd");
        assert_eq!(fit_line("abc", 10), "abc");
        assert_eq!(fit_line("██████", 3).chars().count(), 3);
    }

    #[test]
    fn durations() {
        assert_eq!(format_duration(9), "9s");
        assert_eq!(format_duration(125), "2m05s");
        assert_eq!(format_duration(3725), "1h02m05s");
    }

    /// A writer that fails every write with `error`.
    struct Failing(fn() -> std::io::Error);
    impl std::io::Write for Failing {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err((self.0)())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn output_nobody_reads_is_dropped() {
        let mut written = Vec::new();
        write_to(&mut written, "answer\n").unwrap();
        assert_eq!(written, b"answer\n");
        let broken_pipe = || std::io::Error::from(std::io::ErrorKind::BrokenPipe);
        assert!(write_to(&mut Failing(broken_pipe), "answer\n").is_ok());
        #[cfg(unix)]
        assert!(write_to(
            &mut Failing(|| std::io::Error::from_raw_os_error(libc::EIO)),
            "answer\n"
        )
        .is_ok());
        let full = || std::io::Error::from(std::io::ErrorKind::StorageFull);
        assert_eq!(
            write_to(&mut Failing(full), "answer\n").unwrap_err().kind(),
            std::io::ErrorKind::StorageFull
        );
    }

    #[test]
    fn finished_bars_have_no_eta() {
        let s = render_bar("x", 10, 10, 5, Some(3), 8, 0);
        assert!(s.contains("100%") && !s.contains("remaining"));
    }
}
