# Rust Linux database-key helper

This experimental implementation belongs to issue #90. It supports Linux x86-64 and keeps the original
`xkey_helper_linux` available for comparison. Build with
`cargo build --release -p weflow-key-helper`, then explicitly select the resulting executable:

```sh
weflow --json key db --rust-helper ./target/release/xkey_helper_rust --pid <pid> --timeout 180
```

The standalone protocol is `xkey_helper_rust --db-key [--pid <pid>] [--timeout <seconds>]`.
Successful stdout is exactly one 64-digit hexadecimal key; diagnostics go to stderr. Errors use a nonzero exit
status. The CLI validates both plain keys and legacy JSON responses, rejecting failure JSON even when its process
exited with status zero. The bundled legacy Linux helper currently rejects the service's `--db-key` argument with
`ERROR:UNKNOWN_MODE`; this is now reported as a failure rather than accepted as a database key.

The locator follows the documented ELF `.rodata` cipher string, two RIP-relative reference hops and a bounded
function-prologue search. It requires a unique target and calculates load bias from PT_LOAD mappings for PIE and
fixed-address executables. It checks that the result lies in an executable mapping. Other architectures are
explicitly rejected. Procfs PIDs are translated when procfs belongs to a parent PID namespace.

Capture uses `PTRACE_SEIZE` and `PTRACE_INTERRUPT`, quiesces/re-enumerates the thread set, and follows new threads
with `PTRACE_O_TRACECLONE`. It installs an execution breakpoint in DR0, accepts only RIP at the target with
non-null RSI and RDX equal to 32, then reads 32 bytes. Occupied DR0 is rejected. Unrelated signals are forwarded;
SIGINT/SIGTERM request cleanup. Timeout, error and successful capture restore DR0/DR6/DR7 and detach threads.
The helper does not change `ptrace_scope`, process ownership or executable signatures. Run under an account
allowed to trace WeChat, or with the administrator/root permissions appropriate to your system's ptrace policy.

Tests include synthetic ELF reference chains/load bias and a generated C process that creates a new thread,
calls the target with both 99-byte and 32-byte arguments, and survives capture/timeout after detachment.
Those process tests require ptrace permission; the current cloud environment rejects `PTRACE_SEIZE` with EPERM.
Passing synthetic tests cannot close #90. Record the actual Linux architecture, WeChat version, unique candidate,
master-key capture, successful `weflow db test`, and clean exit on a real client before enabling this by default
or replacing the legacy binary. No real account data or key material belongs in public validation records.
