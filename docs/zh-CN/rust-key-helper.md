# Rust Linux 数据库取钥 helper

此实验性实现对应 #90，支持 Linux x86-64，保留原有 `xkey_helper_linux` 供对照。
执行 `cargo build --release -p weflow-key-helper` 构建，再显式选择产物：

```sh
weflow --json key db --rust-helper ./target/release/xkey_helper_rust --pid <pid> --timeout 180
```

独立程序协议为 `xkey_helper_rust --db-key [--pid <pid>] [--timeout <秒数>]`。
成功时 stdout 仅输出一行 64 位十六进制密钥，诊断写入 stderr，失败使用非零退出码。
CLI 严格校验纯密钥和旧式 JSON 输出，即使 helper 退出码为 0，也拒绝失败 JSON。
当前随包的旧 Linux helper 对服务层的 `--db-key` 返回 `ERROR:UNKNOWN_MODE`，
现在会明确报错，不再将其当成数据库密钥。

定位算法按文档遍历 ELF `.rodata` 的 cipher 字符串、两跳 RIP 相对引用和有界函数序言回溯。
要求目标唯一，根据 PT_LOAD 映射计算 PIE 和固定地址可执行文件的加载偏移，并确认目标处于可执行映射。
其他架构明确返回不支持。procfs 来自父 PID 命名空间时会转换进程编号。

捕获使用 `PTRACE_SEIZE` 和 `PTRACE_INTERRUPT`，停止并重新枚举线程，
通过 `PTRACE_O_TRACECLONE` 跟踪新线程。在 DR0 设置执行断点，
仅接受 RIP 等于目标、RSI 非空、RDX 等于 32 的调用，再读取 32 字节。
已有 DR0 断点时拒绝接管。转发无关信号，SIGINT/SIGTERM 请求清理；
超时、失败和成功捕获后恢复 DR0/DR6/DR7 并解除线程跟踪。
helper 不修改 `ptrace_scope`、进程归属和程序签名。运行账号需要有跟踪微信的权限，
或按系统 ptrace 策略使用管理员 / root 权限。

测试包含合成 ELF 引用链与加载偏移，以及运行时生成的 C 进程：创建新线程，
分别以 99 和 32 的长度调用目标函数，并检查捕获 / 超时后进程解除跟踪且继续存活。
进程测试需要 ptrace 权限；当前云端环境对 `PTRACE_SEIZE` 返回 EPERM。
合成测试通过仍不足以关闭 #90。默认启用或替换原有二进制前，
必须在真实客户端记录 Linux 架构、微信版本、唯一目标定位、主密钥捕获、
`weflow db test` 成功和干净退出。公开验收记录不得包含真实账号数据和密钥。
