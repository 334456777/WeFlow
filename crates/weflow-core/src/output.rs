use serde::Serialize;
use serde_json::Value;

use crate::error::ErrorPayload;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum CliResponse<T: Serialize> {
    Success {
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<T>,
        #[serde(skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },
    Failure {
        success: bool,
        error: ErrorPayload,
    },
}

pub fn success<T: Serialize>(data: T) -> CliResponse<T> {
    CliResponse::Success {
        success: true,
        data: Some(data),
        meta: None,
    }
}

pub fn success_with_meta<T: Serialize>(data: T, meta: Value) -> CliResponse<T> {
    CliResponse::Success {
        success: true,
        data: Some(data),
        meta: Some(meta),
    }
}

pub fn ok() -> CliResponse<Value> {
    CliResponse::Success {
        success: true,
        data: Some(serde_json::json!({ "ok": true })),
        meta: None,
    }
}

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

pub fn json_output() -> bool {
    JSON_OUTPUT.load(std::sync::atomic::Ordering::Relaxed)
}

static STATUS_WIDTH: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Rewrites one status line in place on stderr (a terminal only, never with `--json`); [`end_status_line`] finishes it.
pub fn status_line(text: &str) {
    use std::io::{IsTerminal, Write};
    if json_output() || !std::io::stderr().is_terminal() {
        return;
    }
    let width = crate::render::display_width(text);
    let previous = STATUS_WIDTH.swap(width, std::sync::atomic::Ordering::Relaxed);
    let mut err = std::io::stderr();
    let _ = write!(
        err,
        "\r{text}{}",
        " ".repeat(previous.saturating_sub(width))
    );
    let _ = err.flush();
}

/// Ends the line started by [`status_line`] so that normal output can follow.
pub fn end_status_line() {
    if STATUS_WIDTH.swap(0, std::sync::atomic::Ordering::Relaxed) > 0 {
        eprintln!();
    }
}

/// A status event on stderr: one JSON line with `--json`, a short text for people otherwise.
pub fn event(event: Value) {
    if json_output() {
        eprintln!("{event}");
    } else {
        eprint!("{}", human_event(&event));
    }
}

fn human_event(event: &Value) -> String {
    use crate::render::render;
    let text = |key: &str| event[key].as_str().unwrap_or("").to_string();
    let enabled = |key: &str| event[key].as_bool() == Some(true);
    match event["type"].as_str().unwrap_or("") {
        "key_status" => format!("{}\n", text("message")),
        "auto_download_started" => format!(
            "{}\n",
            tr(
                "image auto download started (Ctrl+C to stop)",
                "图片自动下载已启动（按 Ctrl+C 停止）"
            )
        ),
        "insight" => format!(
            "{}\n{}",
            tr("insight record:", "洞察记录："),
            render(&event["record"])
        ),
        "server_started" => {
            let mut services = Vec::new();
            for (key, en, zh) in [
                ("http", "HTTP API", "HTTP API"),
                ("messagePush", "message push", "消息推送"),
                ("insight", "insight", "洞察"),
                ("imageAutoDownload", "image auto download", "图片自动下载"),
            ] {
                if enabled(key) {
                    services.push(tr(en, zh));
                }
            }
            format!(
                "{} {}\n{} {}\n",
                tr("server started:", "服务已启动："),
                text("url"),
                tr("running:", "运行中："),
                services.join(", ")
            )
        }
        _ => render(event),
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
pub const DEFAULT_BAR_DELAY_SECS: u64 = 5;

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
    let mut st = bar_state().lock().unwrap();
    st.mode = mode;
    st.started = std::time::Instant::now();
    st.is_tty = std::io::stderr().is_terminal();
}

use crate::locale::tr;

pub fn format_duration(secs: u64) -> String {
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
pub fn render_bar(
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
    let filled = (frac * width as f64).round() as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(width.saturating_sub(filled));
    let eta = match eta_secs {
        Some(e) if current < total => {
            format!("  {} {}", tr("remaining", "剩余"), format_duration(e))
        }
        _ => String::new(),
    };
    format!(
        "{message}  [{bar}] {}%  {current}/{total}  {} {}{eta}",
        (frac * 100.0) as u32,
        tr("elapsed", "已用时"),
        format_duration(elapsed_secs)
    )
}

/// Cuts a line to `max` characters so it never wraps (a wrapped line cannot be redrawn in place).
pub fn fit_line(line: &str, max: usize) -> String {
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

pub fn progress(stage: &str, message: &str, current: usize, total: usize) {
    let message = &crate::locale::localize(message.to_string());
    let mut st = bar_state().lock().unwrap();
    match st.mode {
        ProgressMode::Off => {}
        ProgressMode::Ndjson => {
            drop(st);
            let percent = if total > 0 {
                ((current as f64 / total as f64) * 100.0) as u8
            } else {
                0
            };
            eprintln!(
                "{}",
                serde_json::json!({ "type": "progress", "stage": stage, "message": message, "current": current, "total": total, "percent": percent })
            );
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

/// Erases the automatic progress bar (call before printing the final result).
pub fn finish_progress() {
    let mut st = bar_state().lock().unwrap();
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
        assert!(s.starts_with("exporting images  [████████░░░░░░░░] 52%  4056/7800  elapsed 3m47s  remaining 3m28s"), "{s}");
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

    #[test]
    fn finished_bars_have_no_eta() {
        let s = render_bar("x", 10, 10, 5, Some(3), 8, 0);
        assert!(s.contains("100%") && !s.contains("remaining"));
    }
}
