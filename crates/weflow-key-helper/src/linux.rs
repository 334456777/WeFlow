//! x86-64 ptrace capture with per-thread hardware-breakpoint restoration.
use std::collections::BTreeMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};

static CANCELLED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
extern "C" fn cancel(_signal: i32) {
    CANCELLED.store(true, std::sync::atomic::Ordering::Relaxed);
}
pub fn install_signal_handlers() -> Result<()> {
    // The standalone helper restores traced threads before responding to termination.
    for signal in [libc::SIGINT, libc::SIGTERM] {
        let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
        action.sa_sigaction = cancel as *const () as usize;
        unsafe {
            libc::sigemptyset(&mut action.sa_mask);
            if libc::sigaction(signal, &action, std::ptr::null_mut()) == -1 {
                return Err(std::io::Error::last_os_error())
                    .context("signal handler installation failed");
            }
        }
    }
    Ok(())
}

/// Translate namespace-local PIDs when procfs was mounted by a parent namespace.
pub fn proc_directory(pid: i32) -> Result<std::path::PathBuf> {
    let namespace = std::fs::read_link("/proc/self/ns/pid")?;
    let matches = |path: &std::path::Path| {
        std::fs::read_link(path.join("ns/pid")).ok().as_ref() == Some(&namespace)
            && std::fs::read_to_string(path.join("status"))
                .ok()
                .and_then(|status| namespace_pid(&status))
                == Some(pid)
    };
    let direct = std::path::PathBuf::from(format!("/proc/{pid}"));
    if matches(&direct) {
        return Ok(direct);
    }
    for entry in std::fs::read_dir("/proc")? {
        let entry = entry?;
        if matches(&entry.path()) {
            return Ok(entry.path());
        }
    }
    bail!("target process is not visible in procfs")
}

pub fn namespace_pid(status: &str) -> Option<i32> {
    status.lines().find_map(|line| {
        line.strip_prefix("NSpid:")
            .and_then(|s| s.split_whitespace().last()?.parse().ok())
    })
}

fn thread_ids(process: &std::path::Path) -> Result<Vec<i32>> {
    std::fs::read_dir(process.join("task"))?
        .map(|entry| -> Result<i32> {
            let status = std::fs::read_to_string(entry?.path().join("status"))?;
            namespace_pid(&status).context("thread namespace PID not found")
        })
        .collect()
}

fn ptrace(request: libc::c_uint, tid: i32, address: usize, data: usize) -> Result<libc::c_long> {
    // PEEK may successfully return -1, so errno is the only reliable error indicator.
    unsafe {
        *libc::__errno_location() = 0;
        let value = libc::ptrace(request, tid, address as *mut c_void, data as *mut c_void);
        let error = *libc::__errno_location();
        if value == -1 && error != 0 {
            return Err(std::io::Error::from_raw_os_error(error))
                .with_context(|| format!("ptrace request {request} failed; check ownership, root/CAP_SYS_PTRACE and ptrace_scope"));
        }
        Ok(value)
    }
}

fn debug_offset(index: usize) -> usize {
    std::mem::offset_of!(libc::user, u_debugreg) + index * std::mem::size_of::<u64>()
}
fn peek_debug(tid: i32, index: usize) -> Result<u64> {
    Ok(ptrace(libc::PTRACE_PEEKUSER, tid, debug_offset(index), 0)? as u64)
}
fn poke_debug(tid: i32, index: usize, value: u64) -> Result<()> {
    ptrace(
        libc::PTRACE_POKEUSER,
        tid,
        debug_offset(index),
        value as usize,
    )?;
    Ok(())
}

#[derive(Default)]
struct Thread {
    stopped: bool,
    saved: Option<[u64; 3]>, // DR0, DR6, DR7
    pending_signal: i32,
    inherited: Option<[u64; 3]>,
}

#[derive(Default)]
struct TraceSession {
    threads: BTreeMap<i32, Thread>,
    target: u64,
}

fn wait_stop(tid: i32, timeout: Duration) -> Result<Option<i32>> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut status = 0;
        let result = unsafe { libc::waitpid(tid, &mut status, libc::__WALL | libc::WNOHANG) };
        if result == tid {
            if libc::WIFSTOPPED(status) {
                return Ok(Some(status));
            }
            if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                return Ok(None);
            }
        } else if result == -1 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ECHILD)) {
                return Ok(None);
            }
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error).context("waitpid failed");
            }
        }
        if Instant::now() >= deadline {
            bail!("timed out waiting for a traced thread to stop");
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

impl TraceSession {
    fn seize(&mut self, tid: i32) -> Result<()> {
        ptrace(
            libc::PTRACE_SEIZE,
            tid,
            0,
            (libc::PTRACE_O_TRACECLONE | libc::PTRACE_O_TRACEEXEC) as usize,
        )?;
        self.threads.insert(tid, Thread::default());
        self.stop(tid)
    }

    fn stop(&mut self, tid: i32) -> Result<()> {
        if self.threads[&tid].stopped {
            return Ok(());
        }
        // A clone or breakpoint stop may already be waiting to be consumed.
        let mut pending = 0;
        let waited = unsafe { libc::waitpid(tid, &mut pending, libc::__WALL | libc::WNOHANG) };
        if waited == tid {
            if libc::WIFSTOPPED(pending) {
                self.threads.get_mut(&tid).unwrap().stopped = true;
                return self.remember_event(tid, pending);
            }
            self.threads.remove(&tid);
            return Ok(());
        }
        if waited == -1 {
            let error = std::io::Error::last_os_error();
            if matches!(error.raw_os_error(), Some(libc::ESRCH | libc::ECHILD)) {
                self.threads.remove(&tid);
                return Ok(());
            }
            if error.raw_os_error() != Some(libc::EINTR) {
                return Err(error).context("cleanup waitpid failed");
            }
        }
        ptrace(libc::PTRACE_INTERRUPT, tid, 0, 0)?;
        if let Some(status) = wait_stop(tid, Duration::from_secs(5))? {
            self.threads.get_mut(&tid).unwrap().stopped = true;
            self.remember_event(tid, status)?;
        } else {
            self.threads.remove(&tid);
        }
        Ok(())
    }

    fn remember_event(&mut self, tid: i32, status: i32) -> Result<()> {
        let event = status >> 16;
        if event == libc::PTRACE_EVENT_CLONE {
            let mut child = 0u64;
            ptrace(
                libc::PTRACE_GETEVENTMSG,
                tid,
                0,
                (&mut child as *mut u64) as usize,
            )?;
            // The kernel automatically seizes the new thread before it can run.
            let inherited = self.threads[&tid].saved.or(self.threads[&tid].inherited);
            self.threads.entry(i32::try_from(child)?).or_insert(Thread {
                inherited,
                ..Thread::default()
            });
        } else if event == 0 {
            let signal = libc::WSTOPSIG(status);
            // Never forward our execution breakpoint as an application SIGTRAP,
            // including when a hit races with timeout/cancellation cleanup.
            let owned = signal == libc::SIGTRAP
                && self.target != 0
                && peek_debug(tid, 0)? == self.target
                && peek_debug(tid, 7)? & 3 != 0
                && peek_debug(tid, 6)? & 1 != 0;
            self.threads.get_mut(&tid).unwrap().pending_signal = if owned { 0 } else { signal };
        }
        Ok(())
    }

    fn configure(&mut self, tid: i32, target: u64, inherited: Option<[u64; 3]>) -> Result<()> {
        let mut saved = [
            peek_debug(tid, 0)?,
            peek_debug(tid, 6)?,
            peek_debug(tid, 7)?,
        ];
        if saved[2] & 3 != 0 {
            if saved[0] == target {
                saved = inherited.context("DR0 is already in use by another debugger")?;
            } else {
                bail!("DR0 is already in use by another debugger");
            }
        }
        self.threads.get_mut(&tid).unwrap().saved = Some(saved);
        poke_debug(tid, 7, saved[2] & !3)?;
        poke_debug(tid, 0, target)?;
        // Local execution breakpoint, one byte, preserving the other three slots.
        poke_debug(tid, 7, (saved[2] & !(3 | (0xf << 16))) | 1)?;
        Ok(())
    }

    fn resume(&mut self, tid: i32, signal: i32) -> Result<()> {
        ptrace(libc::PTRACE_CONT, tid, 0, signal as usize)?;
        let thread = self.threads.get_mut(&tid).unwrap();
        thread.stopped = false;
        thread.pending_signal = 0;
        Ok(())
    }

    fn cleanup(&mut self) -> Result<()> {
        let mut failures = Vec::new();
        // Stopping a parent can expose clone events, so keep draining newly discovered threads.
        let deadline = Instant::now() + Duration::from_secs(10);
        while self.threads.values().any(|thread| !thread.stopped) && Instant::now() < deadline {
            let tids: Vec<_> = self.threads.keys().copied().collect();
            for tid in tids {
                if !self.threads.contains_key(&tid) {
                    continue;
                }
                if let Err(error) = self.stop(tid) {
                    failures.push(error.to_string());
                }
            }
        }
        let tids: Vec<_> = self.threads.keys().copied().collect();
        for tid in tids {
            let thread = &self.threads[&tid];
            if !thread.stopped {
                failures.push("thread did not stop during cleanup".into());
                continue;
            }
            let saved = thread.saved.or_else(|| {
                // A newly traced child can inherit our slot before configuration.
                (peek_debug(tid, 0).ok() == Some(self.target))
                    .then_some(thread.inherited)
                    .flatten()
            });
            if let Some(saved) = saved {
                let restored = (|| -> Result<()> {
                    poke_debug(tid, 7, saved[2] & !3)?;
                    poke_debug(tid, 0, saved[0])?;
                    poke_debug(tid, 6, saved[1])?;
                    poke_debug(tid, 7, saved[2])?;
                    Ok(())
                })();
                if let Err(error) = restored {
                    failures.push(error.to_string());
                }
            }
            let signal = self.threads[&tid].pending_signal;
            match ptrace(libc::PTRACE_DETACH, tid, 0, signal as usize) {
                Ok(_) => {
                    self.threads.remove(&tid);
                }
                Err(error) => failures.push(error.to_string()),
            }
        }
        if !failures.is_empty() {
            bail!("helper cleanup failed: {}", failures.join("; "));
        }
        Ok(())
    }
}

impl Drop for TraceSession {
    fn drop(&mut self) {
        if !self.threads.is_empty() {
            let _ = self.cleanup();
        }
    }
}

pub fn capture(pid: i32, target: u64, timeout: Duration) -> Result<[u8; 32]> {
    if pid <= 0 || target == 0 || timeout.is_zero() {
        bail!("invalid pid, target or timeout");
    }
    let mut session = TraceSession {
        target,
        threads: BTreeMap::new(),
    };
    let result = capture_inner(&mut session, pid, target, timeout);
    // A key is not reported as a successful capture until all threads are restored/detached.
    let cleanup = session.cleanup();
    cleanup?;
    result
}

fn capture_inner(
    session: &mut TraceSession,
    pid: i32,
    target: u64,
    timeout: Duration,
) -> Result<[u8; 32]> {
    let deadline = Instant::now() + timeout;
    let process = proc_directory(pid)?;
    // Quiesce and enumerate repeatedly; a single /proc/task snapshot misses racing clones.
    let mut quiesced = false;
    for _ in 0..100 {
        let tids = thread_ids(&process)?;
        let missing: Vec<_> = tids
            .into_iter()
            .filter(|tid| !session.threads.contains_key(tid))
            .collect();
        if missing.is_empty() {
            quiesced = true;
            break;
        }
        for tid in missing {
            if let Err(error) = session.seize(tid) {
                if thread_ids(&process).unwrap_or_default().contains(&tid) {
                    return Err(error);
                }
            }
        }
        if Instant::now() >= deadline {
            bail!("thread attachment timed out");
        }
    }
    if !quiesced {
        bail!("thread set did not stabilize during attachment");
    }
    if session.threads.is_empty() {
        bail!("no threads available to trace");
    }
    let tids: Vec<_> = session.threads.keys().copied().collect();
    for tid in &tids {
        if !session.threads[tid].stopped {
            if let Some(status) = wait_stop(*tid, Duration::from_secs(5))? {
                session.threads.get_mut(tid).unwrap().stopped = true;
                session.remember_event(*tid, status)?;
            } else {
                bail!("thread exited during attachment");
            }
        }
        session.configure(*tid, target, None)?;
    }
    for tid in tids {
        let signal = session.threads[&tid].pending_signal;
        session.resume(tid, signal)?;
    }
    while Instant::now() < deadline {
        if CANCELLED.load(std::sync::atomic::Ordering::Relaxed) {
            bail!("database key capture cancelled");
        }
        if session.threads.is_empty() {
            bail!("target process exited");
        }
        let tids: Vec<_> = session.threads.keys().copied().collect();
        for tid in tids {
            if !session.threads.contains_key(&tid) {
                continue;
            }
            let mut status = 0;
            let waited = unsafe { libc::waitpid(tid, &mut status, libc::__WALL | libc::WNOHANG) };
            if waited == 0 {
                continue;
            }
            if waited == -1 {
                let error = std::io::Error::last_os_error();
                if matches!(error.raw_os_error(), Some(libc::ECHILD | libc::ESRCH)) {
                    session.threads.remove(&tid);
                    continue;
                }
                if error.raw_os_error() == Some(libc::EINTR) {
                    continue;
                }
                return Err(error).context("capture waitpid failed");
            }
            if libc::WIFEXITED(status) || libc::WIFSIGNALED(status) {
                session.threads.remove(&tid);
                continue;
            }
            if !libc::WIFSTOPPED(status) {
                continue;
            }
            session.threads.get_mut(&tid).unwrap().stopped = true;
            session.remember_event(tid, status)?;
            let event = status >> 16;
            if event == libc::PTRACE_EVENT_EXEC {
                bail!("target executed a new program; restart acquisition");
            }
            if event == libc::PTRACE_EVENT_CLONE {
                let children: Vec<_> = session
                    .threads
                    .iter()
                    .filter(|(_, thread)| thread.saved.is_none())
                    .map(|(tid, _)| *tid)
                    .collect();
                let inherited = session.threads[&tid].saved;
                for child in children {
                    if let Some(status) = wait_stop(child, Duration::from_secs(5))? {
                        session.threads.get_mut(&child).unwrap().stopped = true;
                        session.remember_event(child, status)?;
                        session.configure(child, target, inherited)?;
                        session.resume(child, 0)?;
                    } else {
                        session.threads.remove(&child);
                    }
                }
                session.resume(tid, 0)?;
                continue;
            }
            if event == 0 && libc::WSTOPSIG(status) == libc::SIGTRAP && peek_debug(tid, 6)? & 1 != 0
            {
                session.threads.get_mut(&tid).unwrap().pending_signal = 0;
                let mut registers = std::mem::MaybeUninit::<libc::user_regs_struct>::uninit();
                ptrace(
                    libc::PTRACE_GETREGS,
                    tid,
                    0,
                    registers.as_mut_ptr() as usize,
                )?;
                let mut registers = unsafe { registers.assume_init() };
                if registers.rip == target && registers.rsi != 0 && registers.rdx == 32 {
                    let mut key = [0u8; 32];
                    for at in (0..32).step_by(8) {
                        let word = ptrace(
                            libc::PTRACE_PEEKDATA,
                            tid,
                            (registers.rsi as usize)
                                .checked_add(at)
                                .context("key pointer overflow")?,
                            0,
                        )?;
                        key[at..at + 8].copy_from_slice(&word.to_ne_bytes());
                    }
                    return Ok(key);
                }
                registers.eflags |= 1 << 16; // RF skips this execution breakpoint once.
                ptrace(
                    libc::PTRACE_SETREGS,
                    tid,
                    0,
                    (&registers as *const libc::user_regs_struct) as usize,
                )?;
                poke_debug(tid, 6, 0)?;
                session.resume(tid, 0)?;
            } else {
                let signal = session.threads[&tid].pending_signal;
                session.resume(tid, signal)?;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    bail!("database key capture timed out; log in while the helper is waiting")
}
