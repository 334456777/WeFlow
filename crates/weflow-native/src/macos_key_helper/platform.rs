//! Attaching to WeChat, locating the database-key sink and reading the key.
//!
//! The original helper set a Mach exception port and got the breakpoint that
//! way. This rewrite keeps the same scan, breakpoint and register rules but
//! lets `ptrace` report the breakpoint: WeChat stops for the few instructions
//! that read the key, then continues. Every exit path restores the patched
//! instruction and detaches.
//!
//! x86_64 `key db` does not use this path: the hardened runtime kills a process
//! that changes another process's thread state or code, so `dtrace.rs` reads the
//! key from WeChat's call to `CCKeyDerivationPBKDF` instead. The breakpoint code
//! here serves ARM64.

use std::ffi::CStr;
use std::slice;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use mach2::kern_return::KERN_SUCCESS;
use mach2::mach_port::mach_port_deallocate;
use mach2::mach_types::{task_t, thread_act_array_t, thread_t};
use mach2::message::mach_msg_type_number_t;
use mach2::port::mach_port_t;
#[cfg(target_arch = "aarch64")]
use mach2::structs::arm_thread_state64_t;
#[cfg(target_arch = "x86_64")]
use mach2::structs::x86_thread_state64_t;
use mach2::task::task_info;
use mach2::task_info::{task_dyld_info, TASK_DYLD_INFO, TASK_DYLD_INFO_COUNT};
use mach2::thread_act::{thread_get_state, thread_set_state};
use mach2::vm::{mach_vm_protect, mach_vm_read_overwrite, mach_vm_region, mach_vm_write};
use mach2::vm_region::{vm_region_basic_info_64, VM_REGION_BASIC_INFO_64};
use mach2::vm_types::{integer_t, mach_vm_address_t, mach_vm_size_t, vm_offset_t};

use super::locator;
use super::module;
use super::process;

extern "C" {
    static mach_task_self_: mach_port_t;
}

pub(super) fn this_task() -> mach_port_t {
    // SAFETY: `mach_task_self_` is the process's task port, initialized by
    // libSystem before `main`.
    unsafe { mach_task_self_ }
}

const KEY_LENGTH: u64 = 32;
const KEY_LENGTH_USIZE: usize = 32;
#[cfg(target_arch = "aarch64")]
const BREAKPOINT: u32 = 0xd420_0000;
#[cfg(target_arch = "x86_64")]
const BREAKPOINT: u32 = 0xcc;
const CHUNK: u64 = 0x10_0000;
const MAX_REGION: u64 = 0x2000_0000;
const ATTACH_TIMEOUT: Duration = Duration::from_secs(5);
const POLL: Duration = Duration::from_millis(10);
#[cfg(target_arch = "aarch64")]
const ARM_THREAD_STATE64: integer_t = 6;
#[cfg(target_arch = "aarch64")]
const ARM_THREAD_STATE64_COUNT: mach_msg_type_number_t = 68;
#[cfg(target_arch = "x86_64")]
const X86_THREAD_STATE64: integer_t = 4;
#[cfg(target_arch = "x86_64")]
const X86_THREAD_STATE64_COUNT: mach_msg_type_number_t = 42;

static INTERRUPTED: AtomicBool = AtomicBool::new(false);

/// Ends a running capture from another thread, like Ctrl+C.
pub fn interrupt() {
    INTERRUPTED.store(true, Ordering::Release);
}

pub(super) fn interrupted() -> bool {
    INTERRUPTED.load(Ordering::Acquire)
}

fn mach_error(kr: i32) -> String {
    if kr == KERN_SUCCESS {
        return String::new();
    }
    // SAFETY: `mach_error_string` returns a NUL-terminated string owned by
    // libSystem, or null; it is copied at once.
    let text = unsafe { libc::mach_error_string(kr) };
    if text.is_null() {
        format!("kr={kr}")
    } else {
        // SAFETY: the pointer is a live NUL-terminated string returned above.
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    }
}

pub(super) fn task_port(pid: u32) -> Result<task_t, String> {
    let mut task = 0;
    // SAFETY: `this_task()` is this process's task port and `task` is a
    // live local that receives the new right.
    let kr = unsafe { libc::task_for_pid(this_task(), pid as libc::pid_t, &raw mut task) };
    if kr != KERN_SUCCESS {
        return Err(format!("task_for_pid({kr}): {}", mach_error(kr)));
    }
    Ok(task)
}

pub(super) fn deallocate_task(task: task_t) {
    // SAFETY: `task` is a send right obtained from `task_for_pid` or
    // `mach_task_self`, and is deallocated once.
    let _ = unsafe { mach_port_deallocate(this_task(), task) };
}

pub(super) fn read_memory(task: task_t, address: u64, size: usize) -> Result<Vec<u8>, String> {
    let mut buffer = vec![0u8; size];
    let mut read = 0u64;
    // SAFETY: `buffer` is `size` writable bytes and `read` a live local;
    // `mach_vm_read_overwrite` writes at most `size` bytes into it.
    let kr = unsafe {
        mach_vm_read_overwrite(
            task,
            address,
            size as mach_vm_size_t,
            buffer.as_mut_ptr() as mach_vm_address_t,
            &raw mut read,
        )
    };
    if kr != KERN_SUCCESS {
        return Err(format!("mach_vm_read_overwrite({kr}): {}", mach_error(kr)));
    }
    if read != size as u64 {
        buffer.truncate(usize::try_from(read).unwrap_or(buffer.len()));
    }
    Ok(buffer)
}

fn read_cstring(task: task_t, address: u64, max: usize) -> Result<String, String> {
    let bytes = read_memory(task, address, max)?;
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    Ok(String::from_utf8_lossy(&bytes[..end]).into_owned())
}

fn write_memory(task: task_t, address: u64, bytes: &[u8]) -> Result<(), String> {
    // SAFETY: `bytes` is a live slice of `bytes.len()` bytes, passed to
    // `mach_vm_write` with that length; the target page is made writable by
    // `write_word` before this is called.
    let kr = unsafe {
        mach_vm_write(
            task,
            address,
            bytes.as_ptr() as vm_offset_t,
            mach_msg_type_number_t::try_from(bytes.len()).unwrap_or(u32::MAX),
        )
    };
    if kr != KERN_SUCCESS {
        return Err(format!("mach_vm_write({kr}): {}", mach_error(kr)));
    }
    Ok(())
}

fn page_size() -> u64 {
    // SAFETY: `sysconf` has no pointer arguments and returns the page size.
    let size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
    u64::try_from(size).unwrap_or(4096)
}

/// Protects the page of `address` and writes `word`; then restores `r-x`.
fn write_word(task: task_t, address: u64, word: u32) -> Result<(), String> {
    let page = address & !(page_size() - 1);
    let size = page_size();
    // SAFETY: both calls protect the page containing `address`, which belongs
    // to the target task; the protection values are the Mach `VM_PROT_*`
    // flags.
    let made_writable = unsafe {
        mach_vm_protect(
            task,
            page,
            size,
            0,
            libc::VM_PROT_READ | libc::VM_PROT_WRITE | libc::VM_PROT_EXECUTE,
        )
    };
    if made_writable != KERN_SUCCESS {
        return Err(format!(
            "protect_fail({made_writable}): {}",
            mach_error(made_writable)
        ));
    }
    let written = write_memory(task, address, &word.to_le_bytes());
    // SAFETY: as above, put the page back to read and execute only.
    let restored = unsafe {
        mach_vm_protect(
            task,
            page,
            size,
            0,
            libc::VM_PROT_READ | libc::VM_PROT_EXECUTE,
        )
    };
    written?;
    if restored != KERN_SUCCESS {
        return Err(format!(
            "restore_rx_fail({restored}): {}",
            mach_error(restored)
        ));
    }
    Ok(())
}

fn ptrace(request: libc::c_int, pid: u32, address: *mut libc::c_char, data: libc::c_int) -> bool {
    // SAFETY: `request` selects the operation; `pid` is the traced process,
    // `address` a kernel-defined address, and `data` the signal or value the
    // request takes. The callers below pass those.
    unsafe { libc::ptrace(request, pid as libc::pid_t, address, data) == 0 }
}

fn wait_status(pid: u32) -> Option<libc::c_int> {
    let mut status = 0;
    loop {
        // SAFETY: `status` is a live local and the options are plain integers.
        let result = unsafe { libc::waitpid(pid as libc::pid_t, &raw mut status, libc::WNOHANG) };
        if result == 0 {
            return None;
        }
        if result > 0 {
            return Some(status);
        }
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return None;
    }
}

fn exited(status: libc::c_int) -> bool {
    status & 0x7f == 0
}

fn stopped_signal(status: libc::c_int) -> Option<libc::c_int> {
    (status & 0xff == 0x7f).then_some((status >> 8) & 0xff)
}

fn thread_states(task: task_t) -> Result<Vec<thread_t>, String> {
    let mut threads: thread_act_array_t = std::ptr::null_mut();
    let mut count: mach_msg_type_number_t = 0;
    // SAFETY: the out parameters are live locals; on success `task_threads`
    // returns an array of `count` send rights allocated by the kernel.
    let kr = unsafe { mach2::task::task_threads(task, &raw mut threads, &raw mut count) };
    if kr != KERN_SUCCESS {
        return Err(format!("task_threads({kr}): {}", mach_error(kr)));
    }
    // SAFETY: `task_threads` returned `count` live thread ports at `threads`.
    let ports = unsafe { slice::from_raw_parts(threads, count as usize) }.to_vec();
    // SAFETY: the array itself is kernel memory returned by `task_threads`.
    unsafe {
        libc::vm_deallocate(
            this_task(),
            threads as libc::vm_address_t,
            count as usize * size_of::<thread_t>(),
        );
    }
    Ok(ports)
}

fn deallocate_threads(threads: &[thread_t]) {
    for thread in threads {
        // SAFETY: each entry is a send right returned by `task_threads`.
        let _ = unsafe { mach_port_deallocate(this_task(), *thread) };
    }
}

/// The values the two architectures use for the key pointer and length.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct KeyArguments {
    pub pointer: u64,
    pub length: u64,
    pub registers: [u64; 5],
}

#[cfg(target_arch = "aarch64")]
fn key_arguments(state: &arm_thread_state64_t) -> KeyArguments {
    KeyArguments {
        pointer: state.__x[1],
        length: state.__x[2],
        registers: [
            state.__x[0],
            state.__x[1],
            state.__x[2],
            state.__x[3],
            state.__x[4],
        ],
    }
}

#[cfg(target_arch = "aarch64")]
fn instruction_pointer(state: &arm_thread_state64_t) -> u64 {
    state.__pc
}

#[cfg(target_arch = "aarch64")]
fn set_instruction_pointer(state: &mut arm_thread_state64_t, value: u64) {
    state.__pc = value;
}

#[cfg(target_arch = "aarch64")]
fn read_state(thread: thread_t) -> Result<arm_thread_state64_t, String> {
    let mut state = arm_thread_state64_t::default();
    let mut count = ARM_THREAD_STATE64_COUNT;
    // SAFETY: `state` is `count` words of writable memory and the flavor names
    // the structure it receives.
    let kr = unsafe {
        thread_get_state(
            thread,
            ARM_THREAD_STATE64,
            (&raw mut state).cast::<u32>(),
            &raw mut count,
        )
    };
    (kr == KERN_SUCCESS)
        .then_some(state)
        .ok_or_else(|| format!("thread_get_state({kr}): {}", mach_error(kr)))
}

#[cfg(target_arch = "aarch64")]
fn write_state(thread: thread_t, state: &arm_thread_state64_t) -> Result<(), String> {
    // SAFETY: `state` is the full structure named by the flavor.
    let kr = unsafe {
        thread_set_state(
            thread,
            ARM_THREAD_STATE64,
            std::ptr::from_ref(state).cast::<u32>().cast_mut(),
            ARM_THREAD_STATE64_COUNT,
        )
    };
    (kr == KERN_SUCCESS)
        .then_some(())
        .ok_or_else(|| format!("thread_set_state({kr}): {}", mach_error(kr)))
}

#[cfg(target_arch = "x86_64")]
fn key_arguments(state: &x86_thread_state64_t) -> KeyArguments {
    KeyArguments {
        pointer: state.__rsi,
        length: state.__rdx,
        registers: [
            state.__rdi,
            state.__rsi,
            state.__rdx,
            state.__rcx,
            state.__rax,
        ],
    }
}

#[cfg(target_arch = "x86_64")]
fn instruction_pointer(state: &x86_thread_state64_t) -> u64 {
    state.__rip
}

#[cfg(target_arch = "x86_64")]
fn set_instruction_pointer(state: &mut x86_thread_state64_t, value: u64) {
    state.__rip = value;
}

#[cfg(target_arch = "x86_64")]
fn read_state(thread: thread_t) -> Result<x86_thread_state64_t, String> {
    let mut state = x86_thread_state64_t::default();
    let mut count = X86_THREAD_STATE64_COUNT;
    // SAFETY: `state` is `count` words of writable memory and the flavor names
    // the structure it receives.
    let kr = unsafe {
        thread_get_state(
            thread,
            X86_THREAD_STATE64,
            (&raw mut state).cast::<u32>(),
            &raw mut count,
        )
    };
    (kr == KERN_SUCCESS)
        .then_some(state)
        .ok_or_else(|| format!("thread_get_state({kr}): {}", mach_error(kr)))
}

#[cfg(target_arch = "x86_64")]
fn write_state(thread: thread_t, state: &x86_thread_state64_t) -> Result<(), String> {
    // SAFETY: `state` is the full structure named by the flavor.
    let kr = unsafe {
        thread_set_state(
            thread,
            X86_THREAD_STATE64,
            std::ptr::from_ref(state).cast::<u32>().cast_mut(),
            X86_THREAD_STATE64_COUNT,
        )
    };
    (kr == KERN_SUCCESS)
        .then_some(())
        .ok_or_else(|| format!("thread_set_state({kr}): {}", mach_error(kr)))
}

#[cfg(target_arch = "aarch64")]
fn is_breakpoint_pc(pc: u64, target: u64) -> bool {
    pc & 0xffff_ffff_ffff == target || pc & 0xffff_ffff_ffff == target.wrapping_add(4)
}

#[cfg(target_arch = "x86_64")]
fn is_breakpoint_pc(pc: u64, target: u64) -> bool {
    pc & 0xffff_ffff_ffff == target.wrapping_add(1)
}

fn key_pointer(arguments: &KeyArguments) -> Option<u64> {
    (arguments.pointer != 0 && arguments.length == KEY_LENGTH).then_some(arguments.pointer)
}

#[cfg(target_arch = "aarch64")]
fn log_hit(arguments: &KeyArguments) {
    tracing::debug!(
        "breakpoint hit! x0=0x{:x} x1=0x{:x} x2=0x{:x} x3=0x{:x} x4=0x{:x}",
        arguments.registers[0],
        arguments.registers[1],
        arguments.registers[2],
        arguments.registers[3],
        arguments.registers[4]
    );
}

#[cfg(target_arch = "x86_64")]
fn log_hit(arguments: &KeyArguments) {
    tracing::debug!(
        "breakpoint hit! rdi=0x{:x} rsi=0x{:x} rdx=0x{:x} rcx=0x{:x}",
        arguments.registers[0],
        arguments.registers[1],
        arguments.registers[2],
        arguments.registers[3]
    );
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// A target task with the database-key location.
#[derive(Debug)]
pub struct Hook {
    pid: u32,
    task: task_t,
    target: u64,
    timeout: Duration,
    original: u32,
    patched: bool,
    attached: bool,
    stopped: bool,
    done: bool,
}

enum Hit {
    Key(String),
    Skip,
    Unrelated,
}

impl Hook {
    /// Attaches to `pid`, finds the database-key sink and sets the breakpoint.
    pub fn start(pid: u32, timeout: Duration) -> Result<Self, String> {
        tracing::debug!("SearchForHookAddress start...");
        let task = match task_port(pid) {
            Ok(task) => task,
            Err(error) => return Err(format!("attach:{error}")),
        };
        let target = match locate(pid, task) {
            Ok(target) => target,
            Err(error) => {
                deallocate_task(task);
                return Err(if error.starts_with("scan:") {
                    error
                } else {
                    format!("scan:{error}")
                });
            }
        };
        tracing::debug!("SearchForHookAddress OK: target=0x{target:x} msg=");
        tracing::debug!(
            "CaptureKeyByBreakpoint start: target=0x{target:x} timeout={}",
            timeout.as_millis()
        );
        let mut hook = Self {
            pid,
            task,
            target,
            timeout,
            original: 0,
            patched: false,
            attached: false,
            stopped: false,
            done: false,
        };
        if let Err(error) = hook.attach_and_patch() {
            hook.cleanup();
            return Err(error);
        }
        Ok(hook)
    }

    fn attach_and_patch(&mut self) -> Result<(), String> {
        tracing::debug!("attaching debugger to pid={} ...", self.pid);
        if !ptrace(libc::PT_ATTACH, self.pid, std::ptr::null_mut(), 0) {
            return Err(format!(
                "ptrace_attach_failed:{}",
                std::io::Error::last_os_error()
            ));
        }
        self.attached = true;
        let deadline = Instant::now() + ATTACH_TIMEOUT;
        loop {
            if let Some(status) = wait_status(self.pid) {
                if exited(status) {
                    return Err("process_died_while_attaching".into());
                }
                if stopped_signal(status).is_some() {
                    break;
                }
            }
            if Instant::now() >= deadline {
                return Err("attach_wait_timeout".into());
            }
            std::thread::sleep(POLL);
        }
        tracing::debug!("debugger attached, patching breakpoint ...");
        self.stopped = true;
        let original = read_memory(self.task, self.target, 4)?;
        self.original = u32::from_le_bytes(
            original
                .get(..4)
                .ok_or("read_target_insn_failed")?
                .try_into()
                .map_err(|_| "read_target_insn_failed")?,
        );
        self.patch_instruction()?;
        if !ptrace(
            libc::PT_CONTINUE,
            self.pid,
            std::ptr::dangling_mut::<libc::c_char>(),
            0,
        ) {
            return Err(format!(
                "ptrace_continue_failed:{}",
                std::io::Error::last_os_error()
            ));
        }
        self.stopped = false;
        tracing::debug!(
            "hook installed at 0x{:x}: native ptrace breakpoint",
            self.target
        );
        tracing::debug!("target continued, waiting hook trigger ...");
        Ok(())
    }

    fn patch_instruction(&mut self) -> Result<(), String> {
        if write_word(self.task, self.target, BREAKPOINT).is_err()
            && !ptrace(
                libc::PT_WRITE_I,
                self.pid,
                self.target as *mut libc::c_char,
                BREAKPOINT as libc::c_int,
            )
            && !ptrace(
                libc::PT_WRITE_D,
                self.pid,
                self.target as *mut libc::c_char,
                BREAKPOINT as libc::c_int,
            )
        {
            return Err(format!(
                "patch_breakpoint_failed:ptrace_word:{}",
                std::io::Error::last_os_error()
            ));
        }
        self.patched = true;
        Ok(())
    }

    fn restore_instruction(&mut self) -> Result<(), String> {
        let restored = write_word(self.task, self.target, self.original).is_ok()
            || ptrace(
                libc::PT_WRITE_I,
                self.pid,
                self.target as *mut libc::c_char,
                self.original as libc::c_int,
            )
            || ptrace(
                libc::PT_WRITE_D,
                self.pid,
                self.target as *mut libc::c_char,
                self.original as libc::c_int,
            );
        self.patched = !restored;
        if restored {
            Ok(())
        } else {
            Err("restore_breakpoint_failed".into())
        }
    }

    /// Restores the original instruction, runs it once and puts the breakpoint
    /// back, so a call with another length does not trap forever.
    fn step_over(&mut self) -> Result<(), String> {
        self.restore_instruction()?;
        if !ptrace(libc::PT_STEP, self.pid, std::ptr::null_mut(), 0) {
            return Err(format!(
                "ptrace_step_failed:{}",
                std::io::Error::last_os_error()
            ));
        }
        self.stopped = false;
        let deadline = Instant::now() + Duration::from_secs(1);
        loop {
            if let Some(status) = wait_status(self.pid) {
                if exited(status) {
                    return Err("process_died_while_stepping".into());
                }
                if stopped_signal(status).is_some() {
                    self.stopped = true;
                    break;
                }
            }
            if Instant::now() >= deadline {
                return Err("step_wait_timeout".into());
            }
            std::thread::sleep(POLL);
        }
        self.patch_instruction()
    }

    /// Waits for the breakpoint, checks the registers and reads the key.
    pub fn wait(&mut self, on_tick: &mut dyn FnMut(u64)) -> Result<String, String> {
        let timeout = self.timeout;
        let started = Instant::now();
        let mut last_tick = u64::MAX;
        loop {
            if interrupted() {
                self.cleanup();
                return Err("interrupted by user".into());
            }
            let left = timeout.saturating_sub(started.elapsed()).as_secs();
            if left != last_tick {
                last_tick = left;
                on_tick(left);
            }
            if started.elapsed() >= timeout {
                self.cleanup();
                return Err("HOOK_TIMEOUT:no_breakpoint_hit".into());
            }
            let Some(status) = wait_status(self.pid) else {
                std::thread::sleep(POLL);
                continue;
            };
            if exited(status) {
                self.cleanup();
                return Err("process_died_while_waiting".into());
            }
            let Some(signal) = stopped_signal(status) else {
                continue;
            };
            if signal == libc::SIGTRAP {
                self.stopped = true;
                match self.hit() {
                    Ok(Hit::Key(key)) => {
                        self.cleanup();
                        return Ok(key);
                    }
                    Ok(Hit::Skip) => {
                        if let Err(error) = self.step_over() {
                            self.cleanup();
                            return Err(error);
                        }
                        self.continue_process(0);
                    }
                    Ok(Hit::Unrelated) => {
                        self.continue_process(0);
                    }
                    Err(error) => {
                        self.cleanup();
                        return Err(error);
                    }
                }
            } else {
                self.continue_process(signal);
            }
        }
    }

    /// Handles one stop: `Some(key)` when this was the 32-byte call, `None`
    /// for an unrelated trap that was resumed.
    fn hit(&mut self) -> Result<Hit, String> {
        let threads = thread_states(self.task)?;
        let mut result = Hit::Unrelated;
        for thread in &threads {
            let Ok(mut state) = read_state(*thread) else {
                continue;
            };
            let pc = instruction_pointer(&state);
            if !is_breakpoint_pc(pc, self.target) {
                continue;
            }
            let arguments = key_arguments(&state);
            log_hit(&arguments);
            set_instruction_pointer(&mut state, self.target);
            let _ = write_state(*thread, &state);
            let Some(pointer) = key_pointer(&arguments) else {
                tracing::debug!(
                    "skip: x1=0x{:x} x2={}, not the 32-byte key call; continuing",
                    arguments.pointer,
                    arguments.length
                );
                result = Hit::Skip;
                break;
            };
            tracing::debug!(
                "args ok: x1=0x{:x} x2={}; reading 32 bytes",
                arguments.pointer,
                arguments.length
            );
            let key = match read_memory(self.task, pointer, KEY_LENGTH_USIZE) {
                Ok(key) if key.len() == KEY_LENGTH_USIZE => hex(&key),
                _ => {
                    deallocate_threads(&threads);
                    self.cleanup();
                    return Err("read_key_memory_failed".into());
                }
            };
            result = Hit::Key(key);
            break;
        }
        deallocate_threads(&threads);
        Ok(result)
    }

    fn continue_process(&mut self, signal: libc::c_int) {
        if ptrace(
            libc::PT_CONTINUE,
            self.pid,
            std::ptr::dangling_mut::<libc::c_char>(),
            signal,
        ) {
            self.stopped = false;
        }
    }

    /// Restores the instruction and detaches; safe to call more than once.
    pub fn cleanup(&mut self) {
        if self.done {
            return;
        }
        self.done = true;
        if self.attached && !self.stopped {
            // The process is running after a timeout: stop it before putting
            // the original instruction back.
            // SAFETY: `kill` sends a signal to a process this hook may trace.
            unsafe { libc::kill(self.pid as libc::pid_t, libc::SIGSTOP) };
            let deadline = Instant::now() + Duration::from_secs(1);
            while Instant::now() < deadline {
                if let Some(status) = wait_status(self.pid) {
                    if stopped_signal(status).is_some() || exited(status) {
                        self.stopped = !exited(status);
                        break;
                    }
                }
                std::thread::sleep(POLL);
            }
        }
        if self.patched {
            let _ = self.restore_instruction();
        }
        if self.attached {
            let _ = ptrace(
                libc::PT_DETACH,
                self.pid,
                std::ptr::dangling_mut::<libc::c_char>(),
                0,
            );
            self.attached = false;
        }
        deallocate_task(self.task);
    }
}

impl Drop for Hook {
    fn drop(&mut self) {
        self.cleanup();
    }
}

fn locate(pid: u32, task: task_t) -> Result<u64, String> {
    let executable = process::executable_path(pid)?;
    tracing::debug!("Scanner target main exec={executable}");
    let version = module::read_version(&executable);
    let known_rva = module::known_rva(version.as_deref());
    tracing::debug!(
        "WeChat version {}, KNOWN_RVA=0x{known_rva:x}",
        version.as_deref().unwrap_or("unknown")
    );
    let modules = list_modules(task)?;
    let selected = match module::select(&modules) {
        module::Choice::WechatDylib(module) => {
            tracing::debug!(
                "Selected wechat.dylib, size={:.1}MB",
                module.size as f64 / (1024.0 * 1024.0)
            );
            module
        }
        module::Choice::Main(module) => {
            tracing::debug!(
                "Selected main WeChat binary, size={:.1}MB",
                module.size as f64 / (1024.0 * 1024.0)
            );
            module
        }
        module::Choice::Ambiguous => {
            return Err(
                "scan:Both wechat.dylib and the main binary are over 100MB, so which to scan is ambiguous".into(),
            );
        }
        module::Choice::None => {
            return Err(
                "scan:No suitable module found: neither wechat.dylib nor a WeChat binary over 100MB".into(),
            );
        }
    };
    tracing::debug!(
        "Scanner module={} base=0x{:x} size=0x{:x}",
        selected.path,
        selected.base,
        selected.size
    );
    scan_module(task, &selected, known_rva)
}

fn scan_module(task: task_t, module: &module::Module, known_rva: u64) -> Result<u64, String> {
    tracing::debug!(
        "SemanticScan: module.base=0x{:x} module.size=0x{:x}",
        module.base,
        module.size
    );
    let end = module.base.saturating_add(module.size);
    let mut candidates = Vec::new();
    let mut address = module.base;
    let mut range = 0;
    loop {
        let mut size: mach_vm_size_t = 0;
        let mut object: mach_port_t = 0;
        let mut info = vm_region_basic_info_64::default();
        let mut count = vm_region_basic_info_64::count();
        // SAFETY: all out parameters are live locals of the types the flavor
        // writes; `address` starts at the region to query.
        let kr = unsafe {
            mach_vm_region(
                task,
                &raw mut address,
                &raw mut size,
                VM_REGION_BASIC_INFO_64,
                (&raw mut info).cast::<integer_t>(),
                &raw mut count,
                &raw mut object,
            )
        };
        if kr != KERN_SUCCESS || address >= end || size == 0 {
            break;
        }
        if object != 0 {
            // SAFETY: the object right was returned by `mach_vm_region`.
            let _ = unsafe { mach_port_deallocate(this_task(), object) };
        }
        if info.protection & (libc::VM_PROT_READ | libc::VM_PROT_EXECUTE)
            == (libc::VM_PROT_READ | libc::VM_PROT_EXECUTE)
            && size <= MAX_REGION
        {
            range += 1;
            tracing::debug!("Scanning range #{range}: addr=0x{address:x} size=0x{size:x}");
            let mut offset = 0;
            while offset < size {
                let length = (size - offset).min(CHUNK) as usize;
                let Ok(bytes) = read_memory(task, address + offset, length) else {
                    offset += CHUNK;
                    continue;
                };
                for anchor in locator::anchor_offsets(&bytes) {
                    let anchor = address + offset + anchor as u64;
                    let Some(candidate) = locator::candidate(anchor) else {
                        continue;
                    };
                    if candidate < module.base || candidate >= end {
                        continue;
                    }
                    let Ok(window) = read_memory(task, candidate, 0x80) else {
                        continue;
                    };
                    if locator::initial_shape(&window) {
                        candidates.push(candidate);
                    }
                }
                offset += CHUNK;
            }
        }
        address = address.saturating_add(size);
    }
    tracing::debug!("semantic candidates={}", candidates.len());
    let mut strict = Vec::new();
    for candidate in &candidates {
        let Ok(window) = read_memory(task, *candidate, 0x80) else {
            continue;
        };
        for target in locator::branch_targets(&window, *candidate) {
            if !(module.base..end).contains(&target) {
                continue;
            }
            let Ok(sink) = read_memory(task, target, 0x20) else {
                continue;
            };
            if locator::strict_sink(&sink) {
                strict.push(*candidate);
                break;
            }
        }
    }
    tracing::debug!("semantic strict candidates={}", strict.len());
    let picked = locator::pick(&strict, module.base, known_rva)
        .ok_or_else(|| "scan:Sink pattern not found".to_string())?;
    tracing::debug!("semantic strict picked=0x{picked:x}");
    Ok(picked)
}

fn list_modules(task: task_t) -> Result<Vec<module::Module>, String> {
    let mut info = task_dyld_info::default();
    let mut count = TASK_DYLD_INFO_COUNT;
    // SAFETY: `info` is the structure of the requested flavor and `count` its
    // size in words on entry and the returned size on exit.
    let kr = unsafe {
        task_info(
            task,
            TASK_DYLD_INFO,
            (&raw mut info).cast::<integer_t>(),
            &raw mut count,
        )
    };
    if kr != KERN_SUCCESS || info.all_image_info_addr == 0 {
        return Err(format!("task_info(TASK_DYLD_INFO): {}", mach_error(kr)));
    }
    let header = read_memory(task, info.all_image_info_addr, 32)?;
    let image_count = u32::from_le_bytes(
        header
            .get(4..8)
            .ok_or("dyld image info header")?
            .try_into()
            .map_err(|_| "dyld image info header")?,
    );
    let array = u64::from_le_bytes(
        header
            .get(8..16)
            .ok_or("dyld image info header")?
            .try_into()
            .map_err(|_| "dyld image info header")?,
    );
    if array == 0 || image_count > 4096 {
        return Err("dyld image info header".into());
    }
    let mut modules = Vec::new();
    for index in 0..u64::from(image_count) {
        let record = read_memory(task, array + index * 24, 24)?;
        let base = u64::from_le_bytes(
            record
                .get(0..8)
                .ok_or("dyld image info")?
                .try_into()
                .map_err(|_| "dyld image info")?,
        );
        let path = u64::from_le_bytes(
            record
                .get(8..16)
                .ok_or("dyld image info")?
                .try_into()
                .map_err(|_| "dyld image info")?,
        );
        if base == 0 || path == 0 {
            continue;
        }
        let Ok(path) = read_cstring(task, path, 4096) else {
            continue;
        };
        let size = module_size(task, base).unwrap_or(0);
        modules.push(module::Module { path, base, size });
    }
    Ok(modules)
}

/// The size of a loaded Mach-O image, from the `LC_SEGMENT_64` ranges; the
/// original ignored `__PAGEZERO` and took the span of the rest.
fn module_size(task: task_t, base: u64) -> Option<u64> {
    let header = read_memory(task, base, 32).ok()?;
    let magic = u32::from_le_bytes(header.get(0..4)?.try_into().ok()?);
    if magic != 0xfeed_facf {
        return None;
    }
    let commands = u32::from_le_bytes(header.get(16..20)?.try_into().ok()?) as usize;
    let bytes = u32::from_le_bytes(header.get(20..24)?.try_into().ok()?) as usize;
    if commands == 0 || bytes == 0 || bytes > 0x10_0000 {
        return None;
    }
    let data = read_memory(task, base + 32, bytes).ok()?;
    let mut at = 0;
    let mut lowest: Option<u64> = None;
    let mut highest = 0u64;
    for _ in 0..commands {
        let command = u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?);
        let size = u32::from_le_bytes(data.get(at + 4..at + 8)?.try_into().ok()?) as usize;
        if size < 8 || at.checked_add(size)? > data.len() {
            break;
        }
        if command == 0x19 && size >= 72 {
            let name = data.get(at + 8..at + 24)?;
            if name != [0u8; 16] && !name.starts_with(b"__PAGEZERO") {
                let address = u64::from_le_bytes(data.get(at + 24..at + 32)?.try_into().ok()?);
                let length = u64::from_le_bytes(data.get(at + 32..at + 40)?.try_into().ok()?);
                if length > 0 {
                    lowest = Some(lowest.map_or(address, |value| value.min(address)));
                    highest = highest.max(address.saturating_add(length));
                }
            }
        }
        at += size;
    }
    let lowest = lowest?;
    (highest > lowest).then_some(highest - lowest)
}

/// The address `GetDbKey` returns: the sink, as 16 lowercase hex digits.
pub fn get_db_key_target(pid: u32) -> Result<String, String> {
    let task = task_port(pid)?;
    let result = locate(pid, task).map(|target| format!("{target:016x}"));
    deallocate_task(task);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_only_a_non_null_pointer_to_32_bytes() {
        let arguments = KeyArguments {
            pointer: 0x1234,
            length: 32,
            registers: [0; 5],
        };
        assert_eq!(key_pointer(&arguments), Some(0x1234));
        assert_eq!(
            key_pointer(&KeyArguments {
                pointer: 0,
                ..arguments
            }),
            None
        );
        assert_eq!(
            key_pointer(&KeyArguments {
                length: 99,
                ..arguments
            }),
            None
        );
    }

    #[test]
    fn formats_the_key_as_lowercase_hex() {
        assert_eq!(hex(&[0xab, 0x01, 0xff]), "ab01ff");
    }
}
