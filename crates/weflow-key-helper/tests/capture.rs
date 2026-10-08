#![cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

struct Target {
    child: Child,
    root: std::path::PathBuf,
    address: u64,
}
impl Drop for Target {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
fn target(tag: &str, capture: bool) -> Target {
    let root =
        std::env::temp_dir().join(format!("weflow-trace-fixture-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&root).unwrap();
    let source = root.join("fixture.c");
    // Synthetic key material. Threads are created after attachment to exercise TRACECLONE.
    std::fs::write(
        &source,
        r#"
#include <stdio.h>
#include <stdlib.h>
#include <unistd.h>
#include <pthread.h>
static unsigned char key[32];
static int capture;
__attribute__((noinline)) void sink(void *unused, const unsigned char *key, unsigned long len) {
    __asm__ volatile("" : : "r"(unused), "r"(key), "r"(len) : "memory");
}
static void *worker(void *unused) {
    for (;;) { sink(unused, key, 99); if(capture) sink(unused, key, 32); usleep(10000); }
}
int main(int argc, char **argv) {
    capture = argc > 1 && atoi(argv[1]);
    for(int i=0; i<32; ++i) key[i] = 0x5a;
    printf("%p\n", (void*)sink); fflush(stdout);
    usleep(100000);
    pthread_t thread; pthread_create(&thread, NULL, worker, NULL);
    worker(NULL);
}
"#,
    )
    .unwrap();
    let binary = root.join("fixture");
    let status = Command::new("cc")
        .args(["-O0", "-pthread"])
        .arg(source)
        .arg("-o")
        .arg(&binary)
        .status()
        .unwrap();
    assert!(status.success());
    let mut child = Command::new(binary)
        .arg(if capture { "1" } else { "0" })
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut line = String::new();
    BufReader::new(child.stdout.take().unwrap())
        .read_line(&mut line)
        .unwrap();
    let address = u64::from_str_radix(line.trim().trim_start_matches("0x"), 16).unwrap();
    Target {
        child,
        root,
        address,
    }
}
fn assert_detached_and_alive(target: &mut Target) {
    let proc = weflow_key_helper::linux::proc_directory(target.child.id() as i32).unwrap();
    let status = std::fs::read_to_string(proc.join("status")).unwrap();
    assert!(status
        .lines()
        .any(|line| line.split_whitespace().collect::<Vec<_>>() == ["TracerPid:", "0"]));
    std::thread::sleep(Duration::from_millis(100));
    assert!(
        target.child.try_wait().unwrap().is_none(),
        "target must survive after detach"
    );
}
#[test]
fn captures_only_32_byte_calls_including_new_threads_and_detaches() {
    let mut target = target("capture", true);
    let key = weflow_key_helper::linux::capture(
        target.child.id() as i32,
        target.address,
        Duration::from_secs(3),
    )
    .unwrap();
    assert_eq!(key, [0x5a; 32]);
    assert_detached_and_alive(&mut target);
}
#[test]
fn timeout_restores_threads_without_killing_the_target() {
    let mut target = target("timeout", false);
    let error = weflow_key_helper::linux::capture(
        target.child.id() as i32,
        target.address,
        Duration::from_millis(300),
    )
    .unwrap_err();
    assert!(error.to_string().contains("capture timed out"), "{error:#}");
    assert_detached_and_alive(&mut target);
}
