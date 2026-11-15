//! Attaching to WeChat x64 and reading its database key through a hardware
//! execution breakpoint. The client's code is not patched, and the debug
//! registers are restored before the tracer detaches.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context as _, Result};

use super::locator::{self, ImageKind};

const DR0: usize = 0x350;
const DR7: usize = 0x388;
const RF: u64 = 1 << 16;
const KEY_SIZE: u32 = 32;
const KEY_BYTES: usize = KEY_SIZE as usize;
const KEY_FIELDS: usize = 24;
const STOP_TIMEOUT: Duration = Duration::from_secs(1);

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Ends a running capture from another thread, like Ctrl+C: the hook restores
/// WeChat's debug registers and detaches before it returns.
pub fn interrupt() {
    INTERRUPTED.store(true, Ordering::Release);
}

fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Acquire)
}

fn io_error(operation: &str, error: io::Error) -> anyhow::Error {
    let denied = matches!(error.raw_os_error(), Some(libc::EPERM | libc::EACCES));
    anyhow!(
        "{}{operation}: {error}",
        if denied { "ACCESS_DENIED: " } else { "" }
    )
}

fn is_gone(error: &io::Error) -> bool {
    error.raw_os_error() == Some(libc::ESRCH)
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

fn thread_ids(pid: u32) -> Result<Vec<libc::pid_t>> {
    let directory = format!("/proc/{pid}/task");
    let entries = std::fs::read_dir(&directory)
        .map_err(|error| io_error(&format!("cannot list {directory}"), error))?;
    let mut ids = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| io_error(&format!("cannot list {directory}"), error))?;
        if let Some(name) = entry.file_name().to_str() {
            if let Ok(tid) = name.parse() {
                ids.push(tid);
            }
        }
    }
    ids.sort_unstable();
    ids.dedup();
    if ids.is_empty() {
        bail!("WeChat has no threads to attach to");
    }
    Ok(ids)
}

fn executable_path(pid: u32) -> Result<PathBuf> {
    std::fs::read_link(format!("/proc/{pid}/exe"))
        .map_err(|error| io_error("cannot read WeChat's executable path", error))
}

/// One line of `/proc/<pid>/maps`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Mapping {
    start: usize,
    end: usize,
    executable: bool,
    path: String,
}

fn maps_path(path: &str) -> &str {
    path.strip_suffix(" (deleted)").unwrap_or(path)
}

/// Parses the mapping lines this module needs; the file offset, device and
/// inode are ignored.
fn parse_maps(text: &str) -> Result<Vec<Mapping>> {
    let mut mappings = Vec::new();
    for line in text.lines() {
        // The inode column is padded with spaces and the pathname may contain
        // spaces, so split off exactly the five leading columns first.
        let mut rest = line;
        let mut fields = [""; 5];
        for field in &mut fields {
            rest = rest.trim_start();
            let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
            *field = &rest[..end];
            rest = &rest[end..];
        }
        if fields.iter().any(|field| field.is_empty()) {
            bail!("truncated /proc/<pid>/maps line: {line}");
        }
        let path = rest.trim_start().to_string();
        let range = fields[0];
        let permissions = fields[1];
        let (start, end) = range
            .split_once('-')
            .context("invalid /proc/<pid>/maps range")?;
        let start = usize::from_str_radix(start, 16)
            .with_context(|| format!("invalid mapping start in {line}"))?;
        let end = usize::from_str_radix(end, 16)
            .with_context(|| format!("invalid mapping end in {line}"))?;
        if end <= start {
            bail!("invalid /proc/<pid>/maps range in {line}");
        }
        mappings.push(Mapping {
            start,
            end,
            executable: permissions.as_bytes().get(2) == Some(&b'x'),
            path,
        });
    }
    Ok(mappings)
}

fn mappings(pid: u32) -> Result<Vec<Mapping>> {
    let path = format!("/proc/{pid}/maps");
    let text = std::fs::read_to_string(&path)
        .map_err(|error| io_error(&format!("cannot read {path}"), error))?;
    parse_maps(&text)
}

fn same_path(mapping: &str, executable: &Path) -> bool {
    let executable = executable.to_string_lossy();
    maps_path(mapping) == maps_path(executable.as_ref())
}

fn load_base(maps: &[Mapping], executable: &Path, kind: ImageKind) -> Result<usize> {
    if kind == ImageKind::Executable {
        return Ok(0);
    }
    maps.iter()
        .filter(|mapping| same_path(&mapping.path, executable))
        .map(|mapping| mapping.start)
        .min()
        .context("WeChat's executable is not mapped")
}

fn is_executable(maps: &[Mapping], address: usize) -> bool {
    maps.iter()
        .any(|mapping| mapping.executable && mapping.start <= address && address < mapping.end)
}

/// The runtime address of the hook point.
struct KeyLocation {
    address: usize,
}

fn locate(pid: u32) -> Result<KeyLocation> {
    let executable = executable_path(pid)?;
    let mut file = File::open(format!("/proc/{pid}/exe"))
        .map_err(|error| io_error("cannot open WeChat's executable", error))?;
    let point = locator::locate(&mut file)?;
    let maps = mappings(pid)?;
    let base = load_base(&maps, &executable, point.kind)?;
    let address = base
        .checked_add(point.virtual_address)
        .context("WeChat's hook address overflows")?;
    if !is_executable(&maps, address) {
        bail!("WeChat's hook address is not in an executable mapping");
    }
    Ok(KeyLocation { address })
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

fn attach(tid: libc::pid_t, target: usize) -> io::Result<()> {
    // SEIZE does not stop the thread itself; INTERRUPT stops it with an event
    // stop, which, unlike SIGSTOP, does not group-stop the whole process.
    let seized = ptrace(
        libc::PTRACE_SEIZE,
        tid,
        0,
        libc::PTRACE_O_TRACECLONE as usize,
    );
    let status = match seized {
        Ok(()) => {
            ptrace(libc::PTRACE_INTERRUPT, tid, 0, 0).and_then(|()| wait_stop(tid, STOP_TIMEOUT))
        }
        // A thread created while its parent was being attached is already
        // attached by TRACECLONE and waits for its initial stop.
        Err(error)
            if matches!(
                error.raw_os_error(),
                Some(libc::EINVAL | libc::EPERM | libc::EBUSY)
            ) =>
        {
            match waitpid_now(tid) {
                Ok(Some(status)) if wifstopped(status) => Ok(status),
                _ => Err(error),
            }
        }
        Err(error) => Err(error),
    };
    let status = match status {
        Ok(status) => status,
        Err(error) => {
            let _ = detach(tid);
            return Err(error);
        }
    };
    if wifexited(status) || wifsignaled(status) {
        let _ = detach(tid);
        // The thread exited while it was being attached; the caller skips it.
        return Err(io::Error::from_raw_os_error(libc::ESRCH));
    }
    if !wifstopped(status) {
        let _ = detach(tid);
        return Err(io::Error::other(
            "attached thread stopped for an unexpected event",
        ));
    }
    // SEIZE already stored the options; setting them again keeps a thread that
    // TRACECLONE attached while its parent was being seized in the same path.
    let armed = ptrace(
        libc::PTRACE_SETOPTIONS,
        tid,
        0,
        libc::PTRACE_O_TRACECLONE as usize,
    )
    .and_then(|()| set_breakpoint(tid, target, true));
    if let Err(error) = armed {
        let _ = detach(tid);
        return Err(error);
    }
    Ok(())
}

fn resume(tid: libc::pid_t, signal: libc::c_int) -> io::Result<()> {
    ptrace(libc::PTRACE_CONT, tid, 0, signal as usize)
}

fn regs(tid: libc::pid_t) -> io::Result<libc::user_regs_struct> {
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

fn set_regs(tid: libc::pid_t, regs: &libc::user_regs_struct) -> io::Result<()> {
    // SAFETY: `regs` is a live `user_regs_struct` for the stopped thread.
    ptrace(
        libc::PTRACE_SETREGS,
        tid,
        0,
        regs as *const libc::user_regs_struct as usize,
    )
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
    libc::pid_t::try_from(message)
        .map_err(|_| io::Error::other("invalid thread id from clone event"))
}

#[derive(Debug, Default, Clone, serde::Serialize)]
pub struct CaptureDiagnostics {
    pub breakpoint_hits: u64,
    pub invalid_arguments: u64,
    pub unreadable_arguments: u64,
    pub unreadable_keys: u64,
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
    diagnostics: CaptureDiagnostics,
}

impl Hook {
    /// Locates the hook point and attaches to every thread of WeChat.
    pub fn start(pid: u32) -> Result<Self> {
        if interrupted() {
            bail!("interrupted by user");
        }
        let location = locate(pid)?;
        if interrupted() {
            bail!("interrupted by user");
        }
        Self::start_at(pid, location.address)
    }

    pub(crate) fn start_at(pid: u32, target: usize) -> Result<Self> {
        let maps = mappings(pid)?;
        if !is_executable(&maps, target) {
            bail!("the hook address is not in an executable mapping");
        }
        let mut threads = BTreeMap::new();
        let mut seen = BTreeSet::new();
        loop {
            let ids = thread_ids(pid)?;
            let new: Vec<_> = ids.into_iter().filter(|tid| seen.insert(*tid)).collect();
            if new.is_empty() {
                break;
            }
            for tid in new {
                // A thread may vanish between the directory listing and the
                // attach; only a real failure aborts the hook.
                if let Err(error) = attach(tid, target) {
                    if is_gone(&error) || error.kind() == io::ErrorKind::NotFound {
                        continue;
                    }
                    for tid in threads.keys().copied().collect::<Vec<_>>() {
                        let _ = set_breakpoint(tid, target, false);
                        let _ = detach(tid);
                    }
                    return Err(io_error("cannot attach to WeChat", error));
                }
                threads.insert(tid, Thread { stopped: true });
            }
        }
        if threads.is_empty() {
            bail!("WeChat has no threads left to attach to");
        }
        let mut hook = Self {
            pid,
            target,
            threads,
            attached: true,
            diagnostics: CaptureDiagnostics::default(),
        };
        for tid in hook.threads.keys().copied().collect::<Vec<_>>() {
            if let Err(error) = resume(tid, 0) {
                let _ = hook.cleanup();
                return Err(io_error("cannot resume WeChat after attaching", error));
            }
            if let Some(thread) = hook.threads.get_mut(&tid) {
                thread.stopped = false;
            }
        }
        Ok(hook)
    }

    pub fn poll(&mut self) -> Result<Option<String>> {
        if interrupted() {
            return Err(anyhow!("interrupted by user"));
        }
        let mut progress = true;
        while progress {
            progress = false;
            for tid in self.threads.keys().copied().collect::<Vec<_>>() {
                if self.threads.get(&tid).is_some_and(|thread| thread.stopped) {
                    continue;
                }
                match self.poll_thread(tid)? {
                    Polled::Idle => {}
                    Polled::Event => progress = true,
                    Polled::Key(key) => return Ok(Some(key)),
                }
            }
            if interrupted() {
                return Err(anyhow!("interrupted by user"));
            }
        }
        Ok(None)
    }

    /// Handles one stop of `tid`.
    fn poll_thread(&mut self, tid: libc::pid_t) -> Result<Polled> {
        let status = match waitpid_now(tid) {
            Ok(None) => return Ok(Polled::Idle),
            Ok(Some(status)) => status,
            Err(error) => {
                if error.raw_os_error() == Some(libc::ECHILD) {
                    self.threads.remove(&tid);
                    if self.threads.is_empty() {
                        bail!("WeChat exited before the key was captured");
                    }
                    return Ok(Polled::Event);
                }
                return Err(io_error("cannot wait for WeChat", error));
            }
        };
        if wifexited(status) || wifsignaled(status) {
            self.threads.remove(&tid);
            if self.threads.is_empty() {
                bail!("WeChat exited before the key was captured");
            }
            return Ok(Polled::Event);
        }
        if !wifstopped(status) {
            return Ok(Polled::Idle);
        }
        let signal = wstopsig(status);
        let event = wevent(status);
        if event == libc::PTRACE_EVENT_CLONE && signal == libc::SIGTRAP {
            let new_tid = event_message(tid).map_err(|error| io_error("clone event", error))?;
            self.follow_new_thread(new_tid)?;
            self.resume_thread(tid, 0)?;
            return Ok(Polled::Event);
        }
        if event == libc::PTRACE_EVENT_STOP {
            self.resume_thread(tid, 0)?;
            return Ok(Polled::Event);
        }
        if signal == libc::SIGTRAP && event == 0 {
            let mut regs = regs(tid).map_err(|error| io_error("PTRACE_GETREGS", error))?;
            if regs.rip == self.target as u64 {
                self.diagnostics.breakpoint_hits += 1;
                if let Some(key) = self.read_key(&regs)? {
                    if let Some(thread) = self.threads.get_mut(&tid) {
                        thread.stopped = true;
                    }
                    return Ok(Polled::Key(key));
                }
                // The instruction breakpoint is a fault: resume with RF so the
                // instruction executes once without trapping again.
                regs.eflags |= RF;
                set_regs(tid, &regs).map_err(|error| io_error("PTRACE_SETREGS", error))?;
            }
            self.resume_thread(tid, 0)?;
            return Ok(Polled::Event);
        }
        // A stop signal belongs to WeChat.
        self.resume_thread(tid, signal)?;
        Ok(Polled::Event)
    }

    fn follow_new_thread(&mut self, tid: libc::pid_t) -> Result<()> {
        if self.threads.contains_key(&tid) {
            return Ok(());
        }
        let status = match wait_stop(tid, STOP_TIMEOUT) {
            Ok(status) => status,
            Err(error) if is_gone(&error) || error.raw_os_error() == Some(libc::ECHILD) => {
                return Ok(())
            }
            Err(error) => return Err(io_error("cannot follow a new WeChat thread", error)),
        };
        if wifexited(status) || wifsignaled(status) {
            return Ok(());
        }
        if !wifstopped(status) {
            return Ok(());
        }
        let armed = ptrace(
            libc::PTRACE_SETOPTIONS,
            tid,
            0,
            libc::PTRACE_O_TRACECLONE as usize,
        )
        .and_then(|()| set_breakpoint(tid, self.target, true));
        if let Err(error) = armed {
            if is_gone(&error) {
                return Ok(());
            }
            return Err(io_error("cannot arm a new WeChat thread", error));
        }
        if let Err(error) = resume(tid, 0) {
            if is_gone(&error) {
                return Ok(());
            }
            return Err(io_error("cannot resume a new WeChat thread", error));
        }
        self.threads.insert(tid, Thread { stopped: false });
        Ok(())
    }

    fn resume_thread(&mut self, tid: libc::pid_t, signal: libc::c_int) -> Result<()> {
        match resume(tid, signal) {
            Ok(()) => {
                if let Some(thread) = self.threads.get_mut(&tid) {
                    thread.stopped = false;
                }
                Ok(())
            }
            Err(error) if is_gone(&error) => {
                self.threads.remove(&tid);
                Ok(())
            }
            Err(error) => Err(io_error("cannot resume WeChat", error)),
        }
    }

    fn read_key(&mut self, regs: &libc::user_regs_struct) -> Result<Option<String>> {
        let mut fields = [0u8; KEY_FIELDS];
        let arguments = usize::try_from(regs.rsi).unwrap_or(0);
        if read_memory(self.pid, arguments, &mut fields).is_err() {
            self.diagnostics.unreadable_arguments += 1;
            return Ok(None);
        }
        let address = usize::try_from(u64::from_le_bytes(fields[8..16].try_into()?)).ok();
        let size = u32::from_le_bytes(fields[16..20].try_into()?);
        let Some(address) = address.filter(|address| *address != 0) else {
            self.diagnostics.invalid_arguments += 1;
            return Ok(None);
        };
        if size != KEY_SIZE {
            self.diagnostics.invalid_arguments += 1;
            return Ok(None);
        }
        let mut key = [0u8; KEY_BYTES];
        if read_memory(self.pid, address, &mut key).is_err() {
            self.diagnostics.unreadable_keys += 1;
            key.fill(0);
            return Ok(None);
        }
        let key = key.iter().map(|byte| format!("{byte:02x}")).collect();
        Ok(Some(key))
    }

    pub fn diagnostics(&self) -> &CaptureDiagnostics {
        &self.diagnostics
    }

    pub fn cleanup(&mut self) -> Result<()> {
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
                match stop_thread(tid) {
                    Ok(Stop::Stopped(new_thread)) => {
                        if let Some(new_tid) = new_thread {
                            pending.push((new_tid, true));
                        }
                    }
                    Ok(Stop::Gone) => {}
                    Err(error) => {
                        first_error.get_or_insert(io_error("cannot stop WeChat", error));
                    }
                }
            }
            if let Err(error) = set_breakpoint(tid, self.target, false) {
                if !is_gone(&error) {
                    first_error.get_or_insert(io_error("cannot clear the breakpoint", error));
                }
            }
            if let Err(error) = detach(tid) {
                if !is_gone(&error) {
                    first_error.get_or_insert(io_error("cannot detach from WeChat", error));
                }
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }
}

enum Stop {
    Stopped(Option<libc::pid_t>),
    Gone,
}

fn stop_thread(tid: libc::pid_t) -> io::Result<Stop> {
    if let Err(error) = ptrace(libc::PTRACE_INTERRUPT, tid, 0, 0) {
        return if is_gone(&error) {
            Ok(Stop::Gone)
        } else {
            Err(error)
        };
    }
    let deadline = Instant::now() + STOP_TIMEOUT;
    loop {
        match waitpid_now(tid) {
            Ok(Some(status)) if wifexited(status) || wifsignaled(status) => return Ok(Stop::Gone),
            Ok(Some(status)) if wifstopped(status) => {
                if wevent(status) == libc::PTRACE_EVENT_CLONE {
                    return Ok(Stop::Stopped(Some(event_message(tid)?)));
                }
                return Ok(Stop::Stopped(None));
            }
            Ok(Some(_)) => {}
            Ok(None) => {}
            Err(error) if error.raw_os_error() == Some(libc::ECHILD) => return Ok(Stop::Gone),
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

impl Drop for Hook {
    fn drop(&mut self) {
        let _ = self.cleanup();
    }
}

fn read_memory(pid: u32, address: usize, bytes: &mut [u8]) -> io::Result<()> {
    let pid = libc::pid_t::try_from(pid).map_err(|_| io::Error::other("invalid process id"))?;
    let local = libc::iovec {
        iov_base: bytes.as_mut_ptr().cast(),
        iov_len: bytes.len(),
    };
    let remote = libc::iovec {
        iov_base: address as *mut libc::c_void,
        iov_len: bytes.len(),
    };
    // SAFETY: both iovecs describe live buffers of their own length; the
    // kernel either fills `bytes` or reports an error.
    let read = unsafe { libc::process_vm_readv(pid, &local, 1, &remote, 1, 0) };
    if read < 0 {
        return Err(io::Error::last_os_error());
    }
    if read as usize != bytes.len() {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "short process_vm_readv",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStdin, Command, Stdio};

    const KEY: [u8; 32] = [0x42; 32];

    #[test]
    fn parses_mappings_and_finds_the_load_base() {
        let maps = parse_maps(
            "00400000-00401000 r--p 00000000 08:01       1 /opt/wechat/wechat\n\
             00401000-00402000 r-xp 00001000 08:01 1 /opt/wechat/wechat\n\
             7f0000000000-7f0000001000 rw-p 00000000 00:00 0 [heap]\n\
             7f0000001000-7f0000002000 r-xp 00000000 08:01 2 /usr/lib/libc.so.6\n",
        )
        .unwrap();
        let executable = Path::new("/opt/wechat/wechat");
        assert_eq!(
            load_base(&maps, executable, ImageKind::Dynamic).unwrap(),
            0x400000
        );
        assert_eq!(
            load_base(&maps, executable, ImageKind::Executable).unwrap(),
            0
        );
        assert!(is_executable(&maps, 0x401234));
        assert!(!is_executable(&maps, 0x400abc));
        assert!(!is_executable(&maps, 0x7f0000000010));
        assert_eq!(maps[1].path, "/opt/wechat/wechat");
        assert_eq!(
            load_base(&maps, Path::new("/opt/wechat/wechat"), ImageKind::Dynamic).unwrap(),
            0x400000
        );
        let deleted =
            parse_maps("00400000-00401000 r-xp 00000000 08:01 1 /opt/wechat/wechat (deleted)\n")
                .unwrap();
        assert_eq!(maps_path(&deleted[0].path), "/opt/wechat/wechat");
        assert_eq!(
            load_base(
                &deleted,
                Path::new("/opt/wechat/wechat (deleted)"),
                ImageKind::Dynamic
            )
            .unwrap(),
            0x400000
        );
        assert!(parse_maps("broken\n").is_err());
        assert!(parse_maps("1000-1000 r-xp 0 0 0 /x\n").is_err());
    }

    #[repr(C)]
    struct KeyArgument {
        reserved: u64,
        key: usize,
        size: u64,
    }

    const _: () = {
        assert!(std::mem::offset_of!(KeyArgument, key) == 8);
        assert!(std::mem::offset_of!(KeyArgument, size) == 16);
    };

    // The same reference chain WeChat uses, so `Hook::start` can locate it in
    // this test binary's own ELF image.
    std::arch::global_asm!(
        r#"
        .section .rodata
        .global __weflow_test_anchor
    __weflow_test_anchor:
        .asciz "com.Tencent.WCDB.Config.Cipher"
        .section .data
    __weflow_test_config:
        .quad 0
        .section .text
        .global __weflow_test_ref
    __weflow_test_ref:
        lea rdi, [rip + __weflow_test_config]
        lea rsi, [rip + __weflow_test_anchor]
        ret
        .global __weflow_test_target
    __weflow_test_target:
        push rbp
        push r15
        push rdi
        lea rsi, [rip + __weflow_test_config]
        pop rdi
        pop r15
        pop rbp
        ret
    "#
    );

    unsafe extern "C" {
        fn __weflow_test_ref();
        fn __weflow_test_target(first: usize, argument: *const KeyArgument);
    }

    #[inline(never)]
    extern "C" fn key_target(first: usize, argument: &KeyArgument) -> usize {
        let size = usize::try_from(argument.size).unwrap();
        std::hint::black_box(first.wrapping_add(size))
    }

    // A real independent process with a generated key. Launched by the tests
    // below through the Rust test harness, never by the normal suite itself.
    #[test]
    #[ignore = "synthetic ptrace target, launched by capture tests"]
    fn target_process() {
        let key = KEY;
        let argument = KeyArgument {
            reserved: 0,
            key: key.as_ptr() as usize,
            size: key.len() as u64,
        };
        println!("TARGET {}", key_target as *const () as usize);
        std::hint::black_box(__weflow_test_ref as *const () as usize);
        std::io::stdout().flush().unwrap();
        for command in std::io::stdin().lock().lines() {
            match command.unwrap().as_str() {
                "call" => {
                    key_target(0, &argument);
                }
                "real-call" => {
                    // SAFETY: the assembly function only reads its argument
                    // register and returns.
                    unsafe { __weflow_test_target(0, &argument) };
                }
                "invalid" => {
                    let invalid = KeyArgument {
                        reserved: 0,
                        key: key.as_ptr() as usize,
                        size: 31,
                    };
                    key_target(0, &invalid);
                }
                "new-thread" => {
                    let address = key.as_ptr() as usize;
                    std::thread::spawn(move || loop {
                        let argument = KeyArgument {
                            reserved: 0,
                            key: address,
                            size: 32,
                        };
                        key_target(0, &argument);
                        std::thread::sleep(Duration::from_millis(10));
                    });
                }
                "exit" => return,
                _ => {}
            }
        }
    }

    struct Target {
        child: Child,
        stdin: ChildStdin,
        stdout: BufReader<std::process::ChildStdout>,
    }

    impl Target {
        fn spawn() -> Self {
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "linux_db_key::platform::tests::target_process",
                    "--ignored",
                    "--nocapture",
                ])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::inherit())
                .spawn()
                .unwrap();
            let stdout = BufReader::new(child.stdout.take().unwrap());
            let stdin = child.stdin.take().unwrap();
            Self {
                child,
                stdin,
                stdout,
            }
        }

        fn address(&mut self) -> usize {
            let mut line = String::new();
            loop {
                line.clear();
                if self.stdout.read_line(&mut line).unwrap() == 0 {
                    panic!("the synthetic target exited before printing its address");
                }
                if let Some(address) = line.strip_prefix("TARGET ") {
                    return address.trim().parse().unwrap();
                }
            }
        }

        fn send(&mut self, command: &str) {
            let _ = writeln!(self.stdin, "{command}");
            let _ = self.stdin.flush();
        }

        fn wait_for_key(&self, hook: &mut Hook) -> String {
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if let Some(key) = hook.poll().unwrap() {
                    return key;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out waiting for the synthetic key");
        }
    }

    impl Drop for Target {
        fn drop(&mut self) {
            self.send("exit");
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    fn expected_key() -> String {
        KEY.iter().map(|byte| format!("{byte:02x}")).collect()
    }

    #[test]
    fn captures_the_key_from_a_synthetic_process() {
        let mut target = Target::spawn();
        let address = target.address();
        let mut hook = Hook::start_at(target.child.id(), address).unwrap();
        target.send("call");
        assert_eq!(target.wait_for_key(&mut hook), expected_key());
        hook.cleanup().unwrap();
        assert!(!matches!(target.child.try_wait(), Ok(Some(_))));
    }

    #[test]
    fn locates_the_hook_point_in_a_real_elf_image() {
        let mut target = Target::spawn();
        let mut hook = Hook::start(target.child.id()).unwrap();
        target.send("real-call");
        assert_eq!(target.wait_for_key(&mut hook), expected_key());
        hook.cleanup().unwrap();
    }

    #[test]
    fn wxkey_captures_the_key_through_the_public_entry() {
        let mut target = Target::spawn();
        let _ = target.address();
        let pid = target.child.id();
        let handle = std::thread::spawn(move || {
            let wxkey = crate::wxkey::WxKey::load(Path::new("")).unwrap();
            assert!(wxkey.is_available());
            let mut installed = false;
            wxkey
                .get_db_key_with_tick(
                    pid,
                    Duration::from_secs(5),
                    &mut |message, level| {
                        if message == "Hook installed; log in to WeChat now" && level == 1 {
                            installed = true;
                        }
                    },
                    &mut |_| {},
                )
                .map(|key| (key, installed))
        });
        std::thread::sleep(Duration::from_millis(1000));
        target.send("real-call");
        let (key, installed) = handle.join().unwrap().unwrap();
        assert!(installed);
        assert_eq!(key, expected_key());
    }

    #[test]
    fn follows_threads_created_after_the_hook() {
        let mut target = Target::spawn();
        let address = target.address();
        let mut hook = Hook::start_at(target.child.id(), address).unwrap();
        target.send("new-thread");
        assert_eq!(target.wait_for_key(&mut hook), expected_key());
        hook.cleanup().unwrap();
    }

    #[test]
    fn ignores_arguments_that_are_not_a_32_byte_key() {
        let mut target = Target::spawn();
        let address = target.address();
        let mut hook = Hook::start_at(target.child.id(), address).unwrap();
        target.send("invalid");
        target.send("call");
        assert_eq!(target.wait_for_key(&mut hook), expected_key());
        assert_eq!(hook.diagnostics().invalid_arguments, 1);
        assert_eq!(hook.diagnostics().breakpoint_hits, 2);
        hook.cleanup().unwrap();
    }

    #[test]
    fn cleanup_leaves_the_target_running() {
        let mut target = Target::spawn();
        let address = target.address();
        let mut hook = Hook::start_at(target.child.id(), address).unwrap();
        hook.cleanup().unwrap();
        target.send("call");
        std::thread::sleep(Duration::from_millis(100));
        assert!(!matches!(target.child.try_wait(), Ok(Some(_))));
    }

    #[test]
    fn reports_a_target_that_exits_while_waiting() {
        let mut target = Target::spawn();
        let address = target.address();
        let mut hook = Hook::start_at(target.child.id(), address).unwrap();
        target.send("exit");
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut error = None;
        while Instant::now() < deadline {
            match hook.poll() {
                Ok(Some(key)) => panic!("unexpected key {key}"),
                Ok(None) => std::thread::sleep(Duration::from_millis(10)),
                Err(e) => {
                    error = Some(e);
                    break;
                }
            }
        }
        assert!(error.unwrap().to_string().contains("exited"));
    }
}
