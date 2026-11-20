//! The command line a run takes. `weflow.exe` double-clicked in Explorer gets no arguments, and its help would show
//! in a window that closes as soon as it is printed: such a run is `weflow serve` instead, which opens the WebUI in
//! the browser and stops when that window is closed (see `stop`).

use std::ffi::OsString;

/// The arguments to parse, from the program's name on: the process's own, or `weflow serve` for a program started
/// with none in a console of its own.
pub(crate) fn args() -> Vec<OsString> {
    let mut args: Vec<OsString> = std::env::args_os().collect();
    if args.len() == 1 && console_of_its_own() {
        args.push("serve".into());
    }
    args
}

/// Whether Windows made this process's console for it alone, as it does for a console program started from Explorer
/// (a double-click, the Start menu, Run); a shell that starts it shares its own console with it. The console closes
/// when the program ends.
#[cfg(windows)]
fn console_of_its_own() -> bool {
    use windows_sys::Win32::System::Console::GetConsoleProcessList;
    let mut id = 0u32;
    // SAFETY: `id` is the one entry the call is told of; it writes at most that one and returns how many processes
    // the console has, 0 when this process has no console.
    let attached = unsafe { GetConsoleProcessList(&raw mut id, 1) };
    attached == 1
}

/// Elsewhere a program is not given a terminal of its own when it is opened from a file manager.
#[cfg(not(windows))]
fn console_of_its_own() -> bool {
    false
}
