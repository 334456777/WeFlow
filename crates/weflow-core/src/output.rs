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

/// How progress events are reported on stderr.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProgressMode {
    /// Nothing.
    Off,
    /// One NDJSON event per line (`--progress`).
    Ndjson,
    /// A single-line progress bar that appears once a command has run for 10 seconds, only when
    /// stderr is a terminal.
    Auto,
}

/// Default number of seconds a command must run before the automatic progress bar appears.
pub const DEFAULT_BAR_DELAY_SECS: u64 = 10;

static BAR_DELAY_SECS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(DEFAULT_BAR_DELAY_SECS);

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
    drawn: bool,
    is_tty: bool,
}

fn bar_state() -> &'static std::sync::Mutex<BarState> {
    static STATE: std::sync::OnceLock<std::sync::Mutex<BarState>> = std::sync::OnceLock::new();
    STATE.get_or_init(|| {
        let now = std::time::Instant::now();
        std::sync::Mutex::new(BarState { mode: ProgressMode::Off, started: now, stage: String::new(), stage_started: now, last_draw: None, drawn: false, is_tty: false })
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
pub fn render_bar(message: &str, current: usize, total: usize, elapsed_secs: u64, width: usize, tick: usize) -> String {
    if total == 0 {
        let spin = ['|', '/', '-', '\\'][tick % 4];
        return format!("{message}  {spin} {current} processed  elapsed {}", format_duration(elapsed_secs));
    }
    let frac = (current as f64 / total as f64).clamp(0.0, 1.0);
    let filled = (frac * width as f64).round() as usize;
    let bar: String = "█".repeat(filled) + &"░".repeat(width.saturating_sub(filled));
    let eta = if current > 0 && current < total { format!("  remaining {}", format_duration((elapsed_secs as f64 / current as f64 * (total - current) as f64).round() as u64)) } else { String::new() };
    format!("{message}  [{bar}] {}%  {current}/{total}  elapsed {}{eta}", (frac * 100.0) as u32, format_duration(elapsed_secs))
}

pub fn progress(stage: &str, message: &str, current: usize, total: usize) {
    let mut st = bar_state().lock().unwrap();
    match st.mode {
        ProgressMode::Off => {}
        ProgressMode::Ndjson => {
            drop(st);
            let percent = if total > 0 { ((current as f64 / total as f64) * 100.0) as u8 } else { 0 };
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
            }
            if !st.is_tty || now.duration_since(st.started).as_secs() < BAR_DELAY_SECS.load(std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            let done = total > 0 && current >= total;
            if !done && st.last_draw.map_or(false, |t| now.duration_since(t).as_millis() < 120) {
                return;
            }
            st.last_draw = Some(now);
            let elapsed = now.duration_since(st.stage_started).as_secs();
            let tick = (now.duration_since(st.started).as_millis() / 120) as usize;
            let line = render_bar(message, current, total, elapsed, 16, tick);
            let mut err = std::io::stderr().lock();
            let _ = write!(err, "\r\x1b[2K{line}");
            let _ = err.flush();
            st.drawn = true;
        }
    }
}

/// Erases the automatic progress bar (call before printing the final result).
pub fn finish_progress() {
    let mut st = bar_state().lock().unwrap();
    if st.drawn {
        use std::io::Write;
        let mut err = std::io::stderr().lock();
        let _ = write!(err, "\r\x1b[2K");
        let _ = err.flush();
        st.drawn = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bar_shows_percentage_and_eta() {
        let s = render_bar("exporting images", 4056, 7800, 227, 16, 0);
        assert!(s.starts_with("exporting images  [████████░░░░░░░░] 52%  4056/7800  elapsed 3m47s  remaining "), "{s}");
    }

    #[test]
    fn unknown_totals_show_a_spinner() {
        let s = render_bar("scanning", 12, 0, 65, 16, 1);
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
    fn durations() {
        assert_eq!(format_duration(9), "9s");
        assert_eq!(format_duration(125), "2m05s");
        assert_eq!(format_duration(3725), "1h02m05s");
    }

    #[test]
    fn finished_bars_have_no_eta() {
        let s = render_bar("x", 10, 10, 5, 8, 0);
        assert!(s.contains("100%") && !s.contains("remaining"));
    }
}
