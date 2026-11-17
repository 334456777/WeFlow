//! Requests to stop a command that runs until it is told to (`serve`, `image auto-download start`), #810: Ctrl+C,
//! and what service managers, terminals and Windows send instead. On Unix that is SIGTERM (`systemctl stop`,
//! `docker stop`, `kill`) and SIGHUP (the terminal closed); on Windows Ctrl+Break, closing the console window,
//! logging off and shutting down. Each is taken the same way, so the command cleans up whichever came.
//!
//! On Windows the last three end the process once their handler returns; tokio's handler waits instead, so the
//! process ends when the command returns, or when Windows stops waiting (5 seconds for a closed window).

/// How the command was asked to stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stop {
    /// Ctrl+C: the user interrupted it.
    Interrupt,
    /// Anything else: it was asked to end, as a service is.
    End,
}

/// Waits for the first request to stop. A request that cannot be listened for is left to its default.
#[cfg(unix)]
pub(crate) async fn requested() -> Stop {
    use tokio::signal::unix::{signal, SignalKind};

    async fn next(kind: SignalKind) {
        match signal(kind) {
            Ok(mut signals) => {
                signals.recv().await;
            }
            Err(_) => std::future::pending().await,
        }
    }
    // `nohup` asks for a closed terminal to be ignored: listening would undo that
    let hangup = async {
        if hangup_ignored() {
            std::future::pending().await
        } else {
            next(SignalKind::hangup()).await;
        }
    };
    tokio::select! {
        () = next(SignalKind::interrupt()) => Stop::Interrupt,
        () = next(SignalKind::terminate()) => Stop::End,
        () = hangup => Stop::End,
    }
}

/// Whether SIGHUP is ignored, as `nohup` leaves it.
#[cfg(unix)]
fn hangup_ignored() -> bool {
    // SAFETY: `sigaction` with no new action only writes the current one into `old`, a valid, owned value; a zeroed
    // `sigaction` is a valid value of that plain C struct.
    unsafe {
        let mut old: libc::sigaction = std::mem::zeroed();
        libc::sigaction(libc::SIGHUP, std::ptr::null(), &raw mut old) == 0
            && old.sa_sigaction == libc::SIG_IGN
    }
}

/// Waits for the first request to stop. A request that cannot be listened for is left to its default.
#[cfg(windows)]
pub(crate) async fn requested() -> Stop {
    use tokio::signal::windows;

    macro_rules! next {
        ($listen:path) => {
            async {
                match $listen() {
                    Ok(mut events) => {
                        events.recv().await;
                    }
                    Err(_) => std::future::pending::<()>().await,
                }
            }
        };
    }
    tokio::select! {
        () = next!(windows::ctrl_c) => Stop::Interrupt,
        () = next!(windows::ctrl_break) => Stop::End,
        () = next!(windows::ctrl_close) => Stop::End,
        () = next!(windows::ctrl_logoff) => Stop::End,
        () = next!(windows::ctrl_shutdown) => Stop::End,
    }
}
