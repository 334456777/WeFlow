use super::*;
use crate::fixture::TestDir;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

use aes::cipher::{Array, BlockCipherEncrypt, KeyInit};

const KEY: [u8; 32] = [0x42; 32];

const SOURCE: &str = r#"
#include <pthread.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

extern void target(void *first, void *argument);
struct key_argument {
    uint64_t reserved;
    const void *key;
    uint64_t size;
};
static unsigned char key[32];
static struct key_argument argument;

static void call_target(int size)
{
    argument.reserved = 0;
    argument.key = key;
    argument.size = (uint64_t)size;
    target(NULL, &argument);
}

static void *loop_thread(void *unused)
{
    (void)unused;
    for (;;) {
        call_target(32);
        usleep(10000);
    }
    return NULL;
}

int main(void)
{
    char line[64];
    memset(key, 0x42, sizeof(key));
    printf("TARGET %lu\n", (unsigned long)target);
    fflush(stdout);
    while (fgets(line, sizeof(line), stdin)) {
        if (strncmp(line, "call", 4) == 0) {
            call_target(32);
        } else if (strncmp(line, "invalid", 7) == 0) {
            call_target(31);
        } else if (strncmp(line, "new-thread", 10) == 0) {
            pthread_t thread;
            pthread_create(&thread, NULL, loop_thread, NULL);
            pthread_detach(thread);
        } else if (strncmp(line, "exit", 4) == 0) {
            return 0;
        }
    }
    return 0;
}
"#;

const ASSEMBLY: &str = r#"
.section .rodata
.global anchor
anchor:
    .asciz "com.Tencent.WCDB.Config.Cipher"
.section .data
cfg:
    .quad 0
.section .text
.global ref
ref:
    lea cfg(%rip), %rdi
    lea anchor(%rip), %rsi
    ret
.global target
target:
    push %rbp
    push %r15
    push %rdi
    lea cfg(%rip), %rsi
    pop %rdi
    pop %r15
    pop %rbp
    ret
.section .note.GNU-stack,"",@progbits
"#;

fn target_binary() -> &'static Path {
    static BINARY: OnceLock<PathBuf> = OnceLock::new();
    BINARY.get_or_init(|| {
        let directory = TestDir::new("weflow-key-target");
        let source = directory.join("target.c");
        let assembly = directory.join("target.S");
        std::fs::write(&source, SOURCE).unwrap();
        std::fs::write(&assembly, ASSEMBLY).unwrap();
        let binary = directory.join("target");
        let output = Command::new("cc")
            .args(["-O0", "-fPIE", "-pie", "-pthread", "-o"])
            .arg(&binary)
            .arg(&source)
            .arg(&assembly)
            .output()
            .expect("a C compiler for the synthetic target");
        assert!(
            output.status.success(),
            "cc failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        // Keep the directory and binary for the life of the test process.
        std::mem::forget(directory);
        binary
    })
}

fn helper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../weflow-assets/assets/key/linux/x64/xkey_helper_linux")
}

struct Target {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<std::process::ChildStdout>,
}

impl Target {
    fn spawn() -> Self {
        let mut child = Command::new(target_binary())
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
        hook.wait(Duration::from_secs(5), &mut |_| {})
            .expect("the synthetic key")
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
    assert!(!matches!(target.child.try_wait(), Ok(Some(_))));
}

#[test]
fn locates_the_hook_point_in_a_real_elf_image() {
    let mut target = Target::spawn();
    let _ = target.address();
    let ours = scan(target.child.id()).unwrap();
    let pid = target.child.id().to_string();
    let output = Command::new(helper())
        .args(["db_scan", &pid])
        .output()
        .unwrap();
    assert!(output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let theirs: usize = value["target_addr"].as_str().unwrap().parse().unwrap();
    assert_eq!(ours, theirs);
    let mut hook = Hook::start(target.child.id()).unwrap();
    target.send("call");
    assert_eq!(target.wait_for_key(&mut hook), expected_key());
}

#[test]
fn follows_threads_created_after_the_hook() {
    let mut target = Target::spawn();
    let address = target.address();
    let mut hook = Hook::start_at(target.child.id(), address).unwrap();
    target.send("new-thread");
    assert_eq!(target.wait_for_key(&mut hook), expected_key());
}

#[test]
fn single_steps_over_arguments_that_are_not_a_32_byte_key() {
    let mut target = Target::spawn();
    let address = target.address();
    let mut hook = Hook::start_at(target.child.id(), address).unwrap();
    target.send("invalid");
    target.send("call");
    assert_eq!(target.wait_for_key(&mut hook), expected_key());
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
    let error = hook
        .wait(Duration::from_secs(5), &mut |_| {})
        .expect_err("the target exits");
    assert_eq!(error, "All threads exited or crashed.");
}

#[test]
fn image_mem_finds_the_key_in_the_target() {
    let mut target = Target::spawn();
    let _ = target.address();
    let key = [0x42u8; 16];
    let mut block = Array::from(*b"\xff\xd8\xffhello world!!");
    aes::Aes128::new((&key).into()).encrypt_block(&mut block);
    let ciphertext: [u8; 16] = block.into();
    let ciphertext: String = ciphertext
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        crate::linux_key_helper::image_mem(target.child.id(), &ciphertext),
        Some("42".repeat(16))
    );
}

#[test]
fn run_helper_reproduces_the_command_line() {
    use crate::linux_key_helper::run_helper_at;

    let args = |values: &[&str]| {
        values
            .iter()
            .map(|value| value.to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        run_helper_at(&args(&["helper"]), None).0,
        "{\"success\":false,\"result\":\"ERROR:USAGE: helper <pid> [timeout_ms]\"}\n"
    );
    assert_eq!(run_helper_at(&args(&["helper", "bogus"]), None).1, 1);
    let mut target = Target::spawn();
    let _ = target.address();
    let pid = target.child.id().to_string();
    let (body, status) = run_helper_at(&args(&["helper", "db_scan", &pid]), None);
    assert_eq!(status, 0);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        value["target_addr"]
            .as_str()
            .unwrap()
            .parse::<usize>()
            .unwrap(),
        scan(target.child.id()).unwrap()
    );
}

#[test]
fn run_helper_reproduces_the_db_hook_json() {
    use crate::linux_key_helper::run_helper_at;

    let mut target = Target::spawn();
    let address = target.address();
    let pid = target.child.id();
    let handle = std::thread::spawn(move || {
        let address = address.to_string();
        let pid = pid.to_string();
        run_helper_at(
            &[
                "helper".to_string(),
                "db_hook".to_string(),
                pid,
                address,
                "5000".to_string(),
            ],
            None,
        )
    });
    std::thread::sleep(Duration::from_millis(500));
    target.send("call");
    let (body, status) = handle.join().unwrap();
    assert_eq!(status, 0);
    let value: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(value["key"], expected_key());
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
    std::thread::sleep(Duration::from_millis(500));
    target.send("call");
    let (key, installed) = handle.join().unwrap().unwrap();
    assert!(installed);
    assert_eq!(key, expected_key());
}
