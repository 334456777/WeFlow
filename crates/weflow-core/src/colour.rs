//! Colours and greys for text on a terminal, chosen from what the terminal says it can show.
//!
//! Colour marks the outcome (red for an error, yellow for a warning, green for success) and weight marks the
//! layers: headings bold, body text in the terminal's own colour, labels and side notes grey. Words carry the
//! meaning on their own, so text without colour (piped, `NO_COLOR`, a terminal without colours) reads the same.

use std::sync::OnceLock;

/// How many colours a stream shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Depth {
    /// Plain text: not a terminal, `NO_COLOR`, `TERM=dumb`.
    Plain,
    /// The 16 colours of every colour terminal; grey is bright black.
    Basic,
    /// 256 colours or more; grey is a fixed grey of the 256-colour palette, which a theme does not remap to the
    /// background colour the way some remap bright black.
    Extended,
}

/// The stream text goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    Stdout,
    Stderr,
}

/// What a piece of text is, which decides how it is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// The label of a failure: bold red.
    Error,
    /// The label of a warning: bold yellow.
    Warning,
    /// What worked: green.
    Success,
    /// Headings and table headers: bold.
    Strong,
    /// Labels, side notes, what is left out: grey.
    Muted,
}

/// Shows text in the colours of one stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Paint {
    depth: Depth,
}

impl Paint {
    /// Colours of `depth`, whatever the stream.
    pub const fn new(depth: Depth) -> Self {
        Self { depth }
    }

    /// Plain text, whatever the stream.
    pub const fn plain() -> Self {
        Self::new(Depth::Plain)
    }

    /// The colours `stream` shows, worked out once per run.
    pub fn of(stream: Stream) -> Self {
        static STDOUT: OnceLock<Depth> = OnceLock::new();
        static STDERR: OnceLock<Depth> = OnceLock::new();
        let cell = match stream {
            Stream::Stdout => &STDOUT,
            Stream::Stderr => &STDERR,
        };
        Self {
            depth: *cell.get_or_init(|| stream_depth(stream)),
        }
    }

    /// Whether this paints nothing.
    pub fn is_plain(self) -> bool {
        self.depth == Depth::Plain
    }

    /// `text` shown as `role`; unchanged when this paints nothing or `text` is empty.
    pub fn paint(self, role: Role, text: &str) -> String {
        let code = match (self.depth, role) {
            (Depth::Plain, _) => return text.to_string(),
            (_, Role::Error) => "1;31",
            (_, Role::Warning) => "1;33",
            (_, Role::Success) => "32",
            (_, Role::Strong) => "1",
            (Depth::Basic, Role::Muted) => "90",
            (Depth::Extended, Role::Muted) => "38;5;244",
        };
        if text.is_empty() {
            return String::new();
        }
        format!("\x1b[{code}m{text}\x1b[0m")
    }
}

fn stream_depth(stream: Stream) -> Depth {
    use std::io::IsTerminal;
    let terminal = match stream {
        Stream::Stdout => std::io::stdout().is_terminal(),
        Stream::Stderr => std::io::stderr().is_terminal(),
    };
    let env = |name: &str| std::env::var(name).ok();
    let depth = depth_from(&env, terminal);
    // a console that cannot show escape sequences would print them
    if depth != Depth::Plain && terminal && !console_shows_escapes(stream) {
        return Depth::Plain;
    }
    depth
}

/// The depth the environment asks for, by the conventions of <https://no-color.org> and
/// <https://bixense.com/clicolors/>: `NO_COLOR` turns colour off; `CLICOLOR_FORCE` or `FORCE_COLOR` turns it on
/// even when the stream is not a terminal (`FORCE_COLOR=0` or `false` turns it off); otherwise only a terminal gets
/// colours, unless `CLICOLOR=0` or `TERM=dumb`. `COLORTERM=truecolor` / `24bit` or a `TERM` with `256color` in it
/// means 256 colours or more, as does the Windows console.
fn depth_from(env: &dyn Fn(&str) -> Option<String>, terminal: bool) -> Depth {
    let set = |name: &str| env(name).filter(|v| !v.is_empty());
    if set("NO_COLOR").is_some() {
        return Depth::Plain;
    }
    let forced = match env("FORCE_COLOR").as_deref() {
        Some("0" | "false") => return Depth::Plain,
        Some("2" | "3") => return Depth::Extended,
        Some(_) => true,
        None => set("CLICOLOR_FORCE").is_some_and(|v| v != "0"),
    };
    if !forced && (!terminal || set("CLICOLOR").as_deref() == Some("0")) {
        return Depth::Plain;
    }
    let term = set("TERM").unwrap_or_default();
    if term == "dumb" && !forced {
        return Depth::Plain;
    }
    let colorterm = set("COLORTERM").unwrap_or_default();
    if matches!(colorterm.as_str(), "truecolor" | "24bit")
        || term.contains("256color")
        || term.contains("direct")
        || cfg!(windows)
    {
        Depth::Extended
    } else if term.is_empty() && !forced {
        Depth::Plain
    } else {
        Depth::Basic
    }
}

/// The Windows console shows escape sequences only once asked to; other terminals always do.
#[cfg(windows)]
fn console_shows_escapes(stream: Stream) -> bool {
    use windows_sys::Win32::System::Console::{
        GetConsoleMode, GetStdHandle, SetConsoleMode, ENABLE_VIRTUAL_TERMINAL_PROCESSING,
        STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
    };
    let which = match stream {
        Stream::Stdout => STD_OUTPUT_HANDLE,
        Stream::Stderr => STD_ERROR_HANDLE,
    };
    // SAFETY: console calls on this process's own standard handle; `mode` outlives the call that writes it
    unsafe {
        let handle = GetStdHandle(which);
        let mut mode = 0;
        GetConsoleMode(handle, &mut mode) != 0
            && (mode & ENABLE_VIRTUAL_TERMINAL_PROCESSING != 0
                || SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING) != 0)
    }
}

#[cfg(not(windows))]
fn console_shows_escapes(_stream: Stream) -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn depth(pairs: &[(&str, &str)], terminal: bool) -> Depth {
        let pairs: Vec<(String, String)> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        let env = move |name: &str| {
            pairs
                .iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v.clone())
        };
        depth_from(&env, terminal)
    }

    #[test]
    fn a_terminal_gets_colours_and_a_pipe_does_not() {
        let basic = if cfg!(windows) {
            Depth::Extended
        } else {
            Depth::Basic
        };
        assert_eq!(depth(&[("TERM", "xterm")], true), basic);
        assert_eq!(depth(&[("TERM", "xterm-256color")], true), Depth::Extended);
        assert_eq!(
            depth(&[("TERM", "xterm"), ("COLORTERM", "truecolor")], true),
            Depth::Extended
        );
        assert_eq!(depth(&[("TERM", "xterm-256color")], false), Depth::Plain);
    }

    #[test]
    fn the_environment_turns_colours_off() {
        for env in [
            &[("TERM", "xterm-256color"), ("NO_COLOR", "1")][..],
            &[("TERM", "xterm-256color"), ("CLICOLOR", "0")],
            &[("TERM", "dumb")],
            &[("TERM", "xterm-256color"), ("FORCE_COLOR", "0")],
            &[("TERM", "xterm-256color"), ("FORCE_COLOR", "false")],
        ] {
            assert_eq!(depth(env, true), Depth::Plain, "{env:?}");
        }
        // an empty NO_COLOR is not set
        assert_ne!(
            depth(&[("TERM", "xterm-256color"), ("NO_COLOR", "")], true),
            Depth::Plain
        );
    }

    #[test]
    fn the_environment_forces_colours_onto_a_pipe() {
        assert_ne!(depth(&[("CLICOLOR_FORCE", "1")], false), Depth::Plain);
        assert_eq!(depth(&[("CLICOLOR_FORCE", "0")], false), Depth::Plain);
        assert_ne!(depth(&[("FORCE_COLOR", "1")], false), Depth::Plain);
        assert_eq!(depth(&[("FORCE_COLOR", "3")], false), Depth::Extended);
        // NO_COLOR wins
        assert_eq!(
            depth(&[("FORCE_COLOR", "1"), ("NO_COLOR", "1")], false),
            Depth::Plain
        );
    }

    #[test]
    fn plain_paint_leaves_text_alone() {
        let paint = Paint::plain();
        assert!(paint.is_plain());
        assert_eq!(paint.paint(Role::Error, "error:"), "error:");
        let basic = Paint::new(Depth::Basic);
        assert_eq!(
            basic.paint(Role::Error, "error:"),
            "\x1b[1;31merror:\x1b[0m"
        );
        assert_eq!(basic.paint(Role::Muted, "x"), "\x1b[90mx\x1b[0m");
        assert_eq!(basic.paint(Role::Muted, ""), "");
        let extended = Paint::new(Depth::Extended);
        assert_eq!(extended.paint(Role::Muted, "x"), "\x1b[38;5;244mx\x1b[0m");
    }
}
