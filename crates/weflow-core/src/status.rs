//! Lines on stderr that say what a command is doing: one that is under way and how it ended ([`Pending`]), and
//! plain lines ([`say`]) that never run into it.
//!
//! On a terminal a pending line stays open: its head (`Downloading WebUI 0.3.0…`) is written first, a countdown may
//! stand after it meanwhile, and the outcome (`done`) ends the same line. Any other line printed before then ends
//! the open line first, and the outcome is written later on a line that repeats the head. Elsewhere (a pipe, a log)
//! the head is a line of its own and the outcome follows on a line that repeats it, so every line reads alone. With
//! `--json` nothing is printed here: commands report in events.

use std::io::{IsTerminal, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use crate::colour::{Paint, Role, Stream};
use crate::render::display_width;

/// The line a [`Pending`] keeps open at the end of the terminal.
#[derive(Debug)]
struct Open {
    id: u64,
    head: String,
    /// columns written after the head (a countdown and the spaces that blanked a longer one)
    after: usize,
}

static OPEN: Mutex<Option<Open>> = Mutex::new(None);
static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Something under way, said on stderr; see the module documentation. Dropped without [`finish`](Self::finish), it
/// only ends its line.
#[derive(Debug)]
#[must_use = "the line ends when this is dropped"]
pub struct Pending {
    id: u64,
    head: String,
    shown: bool,
}

/// Starts a pending line that says `head`, such as `Downloading WebUI 0.3.0…`.
pub fn begin(head: impl Into<String>) -> Pending {
    let head = head.into();
    let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
    let shown = !crate::output::json_output();
    if shown {
        write_with(|open, terminal| begin_text(open, id, &head, terminal));
    }
    Pending { id, head, shown }
}

impl Pending {
    /// Shows `tail` (a countdown) grey after the head, in place of the one before; a terminal only.
    pub fn tick(&self, tail: &str) {
        if !self.shown {
            return;
        }
        let shown = Paint::of(Stream::Stderr).paint(Role::Muted, tail);
        write_with(|open, terminal| {
            if terminal {
                tick_text(open, self.id, &self.head, &shown, display_width(tail))
            } else {
                String::new()
            }
        });
    }

    /// Ends the line with `outcome`, such as `done`, shown as `role`.
    pub fn finish(mut self, outcome: &str, role: Role) {
        if !self.shown {
            return;
        }
        self.shown = false;
        let shown = Paint::of(Stream::Stderr).paint(role, outcome);
        write_with(|open, _| {
            finish_text(open, self.id, &self.head, &shown, display_width(outcome))
        });
    }
}

impl Drop for Pending {
    fn drop(&mut self) {
        if !self.shown {
            return;
        }
        // best effort: a poisoned lock leaves the line as it is
        let Ok(mut open) = OPEN.lock() else {
            return;
        };
        if open.as_ref().is_some_and(|line| line.id == self.id) {
            let _ = std::io::stderr().write_all(break_text(&mut open).as_bytes());
        }
    }
}

/// Ends the open pending line, if any, so that what follows starts on a line of its own.
pub fn break_line() {
    write_with(|open, _| break_text(open));
}

/// Prints `text` on stderr as a line of its own (or several, one per line of `text`).
pub fn say(text: &str) {
    write_with(|open, _| {
        let mut out = break_text(open);
        out.push_str(text);
        if !text.ends_with('\n') {
            out.push('\n');
        }
        out
    });
}

/// Writes what `text` makes of the open line on stderr, under the lock that keeps lines whole.
fn write_with(text: impl FnOnce(&mut Option<Open>, bool) -> String) {
    let terminal = std::io::stderr().is_terminal();
    let mut open = OPEN.lock().unwrap_or_else(|poisoned| {
        // a thread that panicked mid-write leaves at worst a broken line
        poisoned.into_inner()
    });
    let out = text(&mut open, terminal);
    if !out.is_empty() {
        let mut err = std::io::stderr().lock();
        let _ = err.write_all(out.as_bytes());
        let _ = err.flush();
    }
}

/// Ends the open line: the head alone stays, as a countdown after it would soon be out of date.
fn break_text(open: &mut Option<Open>) -> String {
    match open.take() {
        Some(line) if line.after > 0 => format!("\r{}{}\n", line.head, " ".repeat(line.after)),
        Some(_) => "\n".to_string(),
        None => String::new(),
    }
}

fn begin_text(open: &mut Option<Open>, id: u64, head: &str, terminal: bool) -> String {
    let mut out = break_text(open);
    out.push_str(head);
    if terminal {
        *open = Some(Open {
            id,
            head: head.to_string(),
            after: 0,
        });
    } else {
        out.push('\n');
    }
    out
}

fn tick_text(open: &mut Option<Open>, id: u64, head: &str, tail: &str, width: usize) -> String {
    let (mut out, after) = match open.as_ref() {
        Some(line) if line.id == id => ("\r".to_string(), line.after),
        // another line came in between: start this one again below it
        _ => (break_text(open), 0),
    };
    let blank = after.saturating_sub(width + 1);
    out.push_str(&format!("{head} {tail}{}", " ".repeat(blank)));
    *open = Some(Open {
        id,
        head: head.to_string(),
        after: after.max(width + 1),
    });
    out
}

fn finish_text(
    open: &mut Option<Open>,
    id: u64,
    head: &str,
    outcome: &str,
    width: usize,
) -> String {
    match open.as_ref() {
        Some(line) if line.id == id => {
            let blank = line.after.saturating_sub(width + 1);
            *open = None;
            format!("\r{head} {outcome}{}\n", " ".repeat(blank))
        }
        _ => {
            let mut out = break_text(open);
            out.push_str(&format!("{head} {outcome}\n"));
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What a terminal shows after `writes`: `\r` returns to the start of the line, which is then overwritten.
    fn screen(writes: &[String]) -> Vec<String> {
        let mut lines = vec![String::new()];
        let mut column = 0;
        for c in writes.concat().chars() {
            let line = lines.last_mut().expect("one line at least");
            match c {
                '\n' => {
                    lines.push(String::new());
                    column = 0;
                }
                '\r' => column = 0,
                c => {
                    let mut chars: Vec<char> = line.chars().collect();
                    if column < chars.len() {
                        chars[column] = c;
                    } else {
                        chars.push(c);
                    }
                    *line = chars.into_iter().collect();
                    column += 1;
                }
            }
        }
        lines.iter().map(|l| l.trim_end().to_string()).collect()
    }

    #[test]
    fn on_a_terminal_the_outcome_ends_the_line_of_the_head() {
        let mut open = None;
        let writes = vec![
            begin_text(&mut open, 1, "Downloading…", true),
            tick_text(&mut open, 1, "Downloading…", "120 s left", 10),
            tick_text(&mut open, 1, "Downloading…", "9 s left", 8),
            finish_text(&mut open, 1, "Downloading…", "done", 4),
        ];
        assert_eq!(screen(&writes), ["Downloading… done", ""]);
        assert!(open.is_none());
    }

    #[test]
    fn a_line_in_between_ends_the_open_line_and_the_outcome_repeats_the_head() {
        let mut open = None;
        let writes = vec![
            begin_text(&mut open, 1, "Downloading…", true),
            break_text(&mut open),
            "GET /api 200\n".to_string(),
            finish_text(&mut open, 1, "Downloading…", "done", 4),
        ];
        assert_eq!(
            screen(&writes),
            ["Downloading…", "GET /api 200", "Downloading… done", ""]
        );
    }

    #[test]
    fn a_countdown_after_a_line_in_between_starts_again_below_it() {
        let mut open = None;
        let writes = vec![
            begin_text(&mut open, 1, "Waiting…", true),
            tick_text(&mut open, 1, "Waiting…", "120 s left", 10),
            // the countdown is out of date once the line is left: the head alone stays
            break_text(&mut open),
            "a warning\n".to_string(),
            tick_text(&mut open, 1, "Waiting…", "5 s left", 8),
            finish_text(&mut open, 1, "Waiting…", "done", 4),
        ];
        assert_eq!(
            screen(&writes),
            ["Waiting…", "a warning", "Waiting… done", ""]
        );
    }

    #[test]
    fn elsewhere_each_line_reads_alone() {
        let mut open = None;
        let writes = [
            begin_text(&mut open, 1, "Downloading…", false),
            finish_text(&mut open, 1, "Downloading…", "done", 4),
        ];
        assert_eq!(writes.concat(), "Downloading…\nDownloading… done\n");
    }
}
