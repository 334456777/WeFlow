//! Database-key capture of `xkey_helper_linux`, with the one allowed change:
//! threads are seized and interrupted instead of attached and SIGSTOPed, so
//! cleanup never group-stops WeChat.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::locator;
use super::process;

const DR0: usize = 0x350;
const DR7: usize = 0x388;
const KEY_LENGTH: u32 = 32;
const STOP_TIMEOUT: Duration = Duration::from_secs(1);
const WAIT_STEP: Duration = Duration::from_millis(10);

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Ends a running capture from another thread, like Ctrl+C.
pub fn interrupt() {
    INTERRUPTED.store(true, Ordering::Release);
}

fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Acquire)
}

fn wifexited(status: libc::c_int) -> bool {
    status & 0x7f == 0
}

fn wifsignaled(status: libc::c_int) -> bool {
    let signal = status & 0x7f;
    signal != 0 && signal != 0x7f
}

fn wifstopped(status: libc::c_int) -> bool {
    status & 0xff == 0x7f
}

fn wstopsig(status: libc::c_int) -> libc::c_int {
    (status >> 8) & 0xff
}

fn wevent(status: libc::c_int) -> libc::c_int {
    (status >> 16) & 0xffff
}

fn ptrace(request: libc::c_uint, tid: libc::pid_t, address: usize, data: usize) -> io::Result<()> {
    // SAFETY: `request` selects the operation; its arguments are the thread it
    // applies to, a kernel-defined address or zero, and a pointer or value of
    // the type that request expects. All callers below pass matching arguments.
    let result = unsafe {
        libc::ptrace(
            request,
            tid,
            address as *mut libc::c_void,
            data as *mut libc::c_void,
        )
    };
    if result == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn waitpid_now(tid: libc::pid_t) -> io::Result<Option<libc::c_int>> {
    loop {
        let mut status = 0;
        // SAFETY: `status` is a live local and `tid` and the options are plain
        // integers.
        let result = unsafe { libc::waitpid(tid, &mut status, libc::__WALL | libc::WNOHANG) };
        if result == 0 {
            return Ok(None);
        }
        if result == -1 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        return Ok(Some(status));
    }
}

fn wait_stop(tid: libc::pid_t, timeout: Duration) -> io::Result<libc::c_int> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = waitpid_now(tid)? {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "the thread did not stop",
            ));
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn set_breakpoint(tid: libc::pid_t, target: usize, enable: bool) -> io::Result<()> {
    if enable {
        ptrace(libc::PTRACE_POKEUSER, tid, DR0, target)?;
        ptrace(libc::PTRACE_POKEUSER, tid, DR7, 1)
    } else {
        ptrace(libc::PTRACE_POKEUSER, tid, DR7, 0)
    }
}

fn detach(tid: libc::pid_t) -> io::Result<()> {
    ptrace(libc::PTRACE_DETACH, tid, 0, 0)
}

fn continue_thread(tid: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    ptrace(libc::PTRACE_CONT, tid, 0, signal as usize)
}

fn attach(tid: libc::pid_t, target: usize) -> io::Result<()> {
    let seized = ptrace(
        libc::PTRACE_SEIZE,
        tid,
        0,
        libc::PTRACE_O_TRACECLONE as usize,
    );
    let armed = match seized {
        Ok(()) => ptrace(libc::PTRACE_INTERRUPT, tid, 0, 0)
            .and_then(|()| wait_stop(tid, STOP_TIMEOUT))
            .and_then(|status| {
                if wifstopped(status) {
                    Ok(())
                } else {
                    Err(io::Error::from_raw_os_error(libc::ESRCH))
                }
            })
            .and_then(|()| {
                ptrace(
                    libc::PTRACE_SETOPTIONS,
                    tid,
                    0,
                    libc::PTRACE_O_TRACECLONE as usize,
                )
            })
            .and_then(|()| set_breakpoint(tid, target, true)),
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::EINVAL | libc::EPERM | libc::EBUSY)
            ) =>
        {
            // A thread created while its parent was being seized is already
            // attached by TRACECLONE and waits for its initial stop.
            match waitpid_now(tid) {
                Ok(Some(status)) if wifstopped(status) => ptrace(
                    libc::PTRACE_SETOPTIONS,
                    tid,
                    0,
                    libc::PTRACE_O_TRACECLONE as usize,
                )
                .and_then(|()| set_breakpoint(tid, target, true)),
                _ => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    if let Err(error) = armed {
        let _ = detach(tid);
        return Err(error);
    }
    Ok(())
}

#[derive(Debug)]
pub(super) enum ScanError {
    ExecutablePath,
    Scan(&'static str),
    BaseAddress,
}

pub(super) fn scan(pid: u32) -> Result<usize, ScanError> {
    let executable = process::executable_path(pid).map_err(|_| ScanError::ExecutablePath)?;
    if executable.as_os_str().is_empty() {
        return Err(ScanError::ExecutablePath);
    }
    let bytes = process::read_executable(&executable)
        .map_err(|_| ScanError::Scan("Failed to open or read executable."))?;
    let address = locator::locate(&bytes).map_err(ScanError::Scan)?;
    let maps = process::mappings(pid).map_err(|_| ScanError::BaseAddress)?;
    let base = process::base_address(&maps, &executable).ok_or(ScanError::BaseAddress)?;
    base.checked_add(address).ok_or(ScanError::BaseAddress)
}

enum Polled {
    Idle,
    Event,
    Key(String),
}

struct Thread {
    stopped: bool,
}

pub struct Hook {
    pid: u32,
    target: usize,
    threads: BTreeMap<libc::pid_t, Thread>,
    attached: bool,
}

impl Hook {
    /// Reads WeChat's ELF image, finds the hook point and attaches to every
    /// thread. The returned error is the helper's message.
    pub fn start(pid: u32) -> Result<Self, String> {
        if interrupted() {
            return Err("interrupted by user".into());
        }
        let target = scan(pid).map_err(|error| match error {
            ScanError::ExecutablePath => "CANNOT_GET_EXEC_PATH".to_string(),
            ScanError::Scan(message) => message.to_string(),
            ScanError::BaseAddress => "CANNOT_GET_BASE_ADDR".to_string(),
        })?;
        if interrupted() {
            return Err("interrupted by user".into());
        }
        Self::start_at(pid, target)
    }

    pub(super) fn start_at(pid: u32, target: usize) -> Result<Self, String> {
        let ids =
            process::thread_ids(pid).map_err(|_| "Failed to open task directory.".to_string())?;
        if ids.is_empty() {
            return Err("Failed to attach to any thread; permission may be denied.".into());
        }
        let mut threads = BTreeMap::new();
        let mut seen = BTreeSet::new();
        loop {
            let ids = process::thread_ids(pid)
                .map_err(|_| "Failed to open task directory.".to_string())?;
            let new: Vec<_> = ids.into_iter().filter(|tid| seen.insert(*tid)).collect();
            if new.is_empty() {
                break;
            }
            for tid in new {
                match attach(tid, target) {
                    Ok(()) => {
                        threads.insert(tid, Thread { stopped: true });
                    }
                    Err(error)
                        if error.raw_os_error() == Some(libc::ESRCH)
                            || error.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => {}
                }
            }
        }
        if threads.is_empty() {
            return Err("Failed to attach to any thread; permission may be denied.".into());
        }
        let mut hook = Self {
            pid,
            target,
            threads,
            attached: true,
        };
        for tid in hook.threads.keys().copied().collect::<Vec<_>>() {
            if continue_thread(tid, 0).is_err() {
                let _ = hook.cleanup();
                return Err("Failed to attach to any thread; permission may be denied.".into());
            }
            if let Some(thread) = hook.threads.get_mut(&tid) {
                thread.stopped = false;
            }
        }
        Ok(hook)
    }

    /// Waits for the key, calling `on_tick` once per second with the remaining
    /// whole seconds. The helper's messages are returned on failure.
    pub fn wait(
        &mut self,
        timeout: Duration,
        on_tick: &mut dyn FnMut(u64),
    ) -> Result<String, String> {
        let start = Instant::now();
        let mut last_tick = u64::MAX;
        loop {
            if interrupted() {
                let _ = self.cleanup();
                return Err("interrupted by user".into());
            }
            let left = timeout.saturating_sub(start.elapsed()).as_secs();
            if left != last_tick {
                last_tick = left;
                on_tick(left);
            }
            if start.elapsed() >= timeout {
                let _ = self.cleanup();
                return Err("Timeout waiting for 32-byte key".into());
            }
            let mut progress = false;
            for tid in self.threads.keys().copied().collect::<Vec<_>>() {
                if self.threads.get(&tid).is_some_and(|thread| thread.stopped) {
                    continue;
                }
                match self.poll_thread(tid) {
                    Ok(Polled::Idle) => {}
                    Ok(Polled::Event) => progress = true,
                    Ok(Polled::Key(key)) => {
                        let _ = self.cleanup();
                        return Ok(key);
                    }
                    Err(message) => {
                        let _ = self.cleanup();
                        return Err(message);
                    }
                }
            }
            if self.threads.is_empty() {
                let _ = self.cleanup();
                return Err("All threads exited or crashed.".into());
            }
            if !progress {
                std::thread::sleep(WAIT_STEP);
            }
        }
    }

    fn poll_thread(&mut self, tid: libc::pid_t) -> Result<Polled, String> {
        let status = match waitpid_now(tid) {
            Ok(None) => return Ok(Polled::Idle),
            Ok(Some(status)) => status,
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.threads.remove(&tid);
                    return Ok(Polled::Event);
                }
                return Err(error.to_string());
            }
        };
        if wifexited(status) || wifsignaled(status) {
            self.threads.remove(&tid);
            return Ok(Polled::Event);
        }
        if !wifstopped(status) {
            return Ok(Polled::Idle);
        }
        let signal = wstopsig(status);
        let event = wevent(status);
        if event == libc::PTRACE_EVENT_CLONE && signal == libc::SIGTRAP {
            let new_tid = event_message(tid).map_err(|error| error.to_string())?;
            self.follow_new_thread(new_tid)?;
            self.resume_thread(tid, 0)?;
            return Ok(Polled::Event);
        }
        if event == libc::PTRACE_EVENT_STOP {
            self.resume_thread(tid, 0)?;
            return Ok(Polled::Event);
        }
        if signal == libc::SIGTRAP && event == 0 {
            let regs = get_regs(tid).map_err(|error| error.to_string())?;
            if regs.rip != self.target as u64 {
                self.resume_thread(tid, 0)?;
                return Ok(Polled::Event);
            }
            match self.read_key(&regs) {
                Ok(Some(key)) => {
                    if let Some(thread) = self.threads.get_mut(&tid) {
                        thread.stopped = true;
                    }
                    return Ok(Polled::Key(key));
                }
                Ok(None) => {
                    self.single_step(tid)?;
                    return Ok(Polled::Event);
                }
                Err(_) => {
                    self.single_step(tid)?;
                    return Ok(Polled::Event);
                }
            }
        }
        // The helper suppresses SIGSTOP; other signals belong to WeChat.
        let signal = if signal == libc::SIGSTOP { 0 } else { signal };
        self.resume_thread(tid, signal)?;
        Ok(Polled::Event)
    }

    fn read_key(&mut self, regs: &libc::user_regs_struct) -> io::Result<Option<String>> {
        let structure = usize::try_from(regs.rsi).map_err(|_| io::Error::other("invalid RSI"))?;
        let length_at = structure
            .checked_add(0x10)
            .ok_or_else(|| io::Error::other("invalid key structure"))?;
        let mut length = [0u8; 4];
        process::read_memory(self.pid, length_at, &mut length)?;
        let length = u32::from_le_bytes(length);
        if length != KEY_LENGTH {
            return Ok(None);
        }
        let pointer_at = structure
            .checked_add(8)
            .ok_or_else(|| io::Error::other("invalid key structure"))?;
        let mut pointer = [0u8; 8];
        process::read_memory(self.pid, pointer_at, &mut pointer)?;
        let pointer = usize::try_from(u64::from_le_bytes(pointer))
            .map_err(|_| io::Error::other("invalid key pointer"))?;
        if pointer == 0 {
            return Ok(None);
        }
        let mut key = [0u8; KEY_LENGTH as usize];
        process::read_memory(self.pid, pointer, &mut key)?;
        Ok(Some(key.iter().map(|byte| format!("{byte:02x}")).collect()))
    }

    fn single_step(&mut self, tid: libc::pid_t) -> Result<(), String> {
        let _ = set_breakpoint(tid, self.target, false);
        ptrace(libc::PTRACE_SINGLESTEP, tid, 0, 0).map_err(|error| error.to_string())?;
        // The helper waits without WNOHANG here; the step itself is short.
        let _ = wait_stop(tid, STOP_TIMEOUT);
        let _ = set_breakpoint(tid, self.target, true);
        self.resume_thread(tid, 0)?;
        Ok(())
    }

    fn follow_new_thread(&mut self, tid: libc::pid_t) -> Result<(), String> {
        if self.threads.contains_key(&tid) {
            return Ok(());
        }
        let status = wait_stop(tid, STOP_TIMEOUT).map_err(|error| error.to_string())?;
        if !wifstopped(status) {
            return Ok(());
        }
        ptrace(
            libc::PTRACE_SETOPTIONS,
            tid,
            0,
            libc::PTRACE_O_TRACECLONE as usize,
        )
        .map_err(|error| error.to_string())?;
        set_breakpoint(tid, self.target, true).map_err(|error| error.to_string())?;
        continue_thread(tid, 0).map_err(|error| error.to_string())?;
        self.threads.insert(tid, Thread { stopped: false });
        Ok(())
    }

    fn resume_thread(&mut self, tid: libc::pid_t, signal: libc::c_int) -> Result<(), String> {
        match continue_thread(tid, signal) {
            Ok(()) => {
                if let Some(thread) = self.threads.get_mut(&tid) {
                    thread.stopped = false;
                }
                Ok(())
            }
            Err(error) if error.raw_os_error() == Some(libc::ESRCH) => {
                self.threads.remove(&tid);
                Ok(())
            }
            Err(error) => Err(error.to_string()),
        }
    }

    pub fn cleanup(&mut self) -> Result<(), String> {
        if !self.attached {
            return Ok(());
        }
        let mut pending: Vec<(libc::pid_t, bool)> = self
            .threads
            .iter()
            .map(|(tid, thread)| (*tid, thread.stopped))
            .collect();
        self.threads.clear();
        self.attached = false;
        let mut first_error = None;
        while let Some((tid, stopped)) = pending.pop() {
            if !stopped {
                match self.interrupt_thread(tid) {
                    Ok(Some(new_tid)) => pending.push((new_tid, true)),
                    Ok(None) => {}
                    Err(error) => {
                        first_error.get_or_insert(error.to_string());
                    }
                }
            }
            if let Err(error) = set_breakpoint(tid, self.target, false) {
                if error.raw_os_error() != Some(libc::ESRCH) {
                    first_error.get_or_insert(error.to_string());
                }
            }
            if let Err(error) = detach(tid) {
                if error.raw_os_error() != Some(libc::ESRCH) {
                    first_error.get_or_insert(error.to_string());
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn interrupt_thread(&self, tid: libc::pid_t) -> io::Result<Option<libc::pid_t>> {
        if let Err(error) = ptrace(libc::PTRACE_INTERRUPT, tid, 0, 0) {
            return if error.raw_os_error() == Some(libc::ESRCH) {
                Ok(None)
            } else {
                Err(error)
            };
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        loop {
            match waitpid_now(tid) {
                Ok(Some(status)) if wifexited(status) || wifsignaled(status) => return Ok(None),
                Ok(Some(status)) if wifstopped(status) => {
                    if wevent(status) == libc::PTRACE_EVENT_CLONE {
                        return Ok(Some(event_message(tid)?));
                    }
                    return Ok(None);
                }
                Ok(Some(_)) | Ok(None) => {}
                Err(error) if error.raw_os_error() == Some(libc::ECHILD) => return Ok(None),
                Err(error) => return Err(error),
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the thread did not stop",
                ));
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }
}

fn get_regs(tid: libc::pid_t) -> io::Result<libc::user_regs_struct> {
    // SAFETY: `user_regs_struct` consists of integer fields, so all zeroes is a
    // valid value; PTRACE_GETREGS overwrites it immediately.
    let mut regs = unsafe { std::mem::zeroed::<libc::user_regs_struct>() };
    // SAFETY: `regs` is a live `user_regs_struct` and PTRACE_GETREGS fills one
    // for the stopped thread.
    ptrace(
        libc::PTRACE_GETREGS,
        tid,
        0,
        &mut regs as *mut libc::user_regs_struct as usize,
    )?;
    Ok(regs)
}

fn event_message(tid: libc::pid_t) -> io::Result<libc::pid_t> {
    let mut message = 0u64;
    // SAFETY: PTRACE_GETEVENTMSG writes one unsigned long to the live local
    // pointed to by `data`.
    ptrace(
        libc::PTRACE_GETEVENTMSG,
        tid,
        0,
        &mut message as *mut u64 as usize,
    )?;
    libc::pid_t::try_from(message).map_err(|_| io::Error::other("invalid thread id"))
}

impl Drop for Hook {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

#[cfg(test)]
#[path = "platform_tests.rs"]
mod tests;
