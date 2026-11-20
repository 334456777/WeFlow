---
name: weflow-cross-platform
description: 检查 WeFlow 在 Windows、Linux、macOS 之间的差异：只在部分平台编译的文件、依赖 cfg!(windows) 或路径分隔符的测试、只给某个平台用的 import，在 Windows 上用 WSL 编译和测试 Linux，以及接手只在一个平台上编译过的分支（如云端会话的分支）时逐个提交检查。改平台相关的代码、改公开函数的签名、写平台相关的测试、接手别处推送的分支时使用。
---

# 跨平台检查

CI 在三个平台上的分工见 Wiki [发布与 CI#工作流](https://github.com/334456777/WeFlow/wiki/发布与-CI#工作流)，Windows 上在 WSL 里编译 Linux 版见 [开发指南#构建命令](https://github.com/334456777/WeFlow/wiki/开发指南#构建命令)。

## 哪些改动要在别的平台上检查

- 开头写了 `#![cfg(...)]` 的文件在别的平台上不编译，例如 `crates/weflow-core/tests/insight_e2e.rs` 只在 Linux 上编译。改了公开函数的签名或返回类型时，先 `grep -rln '#!\[cfg(' crates` 找出这些文件，再在对应的平台上运行 `cargo clippy -p weflow-core -p weflow-cli --all-targets -- -D warnings`。
- 依赖 `cfg!(windows)`、反斜杠是否为路径分隔符、`#[cfg(unix)]` 的符号链接的测试：`\` 在 Unix 上是普通的文件名字符，在 Windows 上通过的测试在 Linux 和 macOS 上可能失败。推送前在另一个平台上运行受影响的测试（如 `cargo test -p weflow-core --lib backup`）。
- 只给某个平台的测试用的 `use` 写进那个测试函数里，不放在文件顶部，否则另一个平台会报未使用的 import。
- macOS 专属的代码在别的平台上编译不了，以 CI 的结果为准。

## Windows 上用 WSL

- 仓库里的符号链接（`AGENTS.md`、`.claude/skills`）要开启开发者模式并设 `core.symlinks` 才会检出为链接，见 Wiki [开发指南#Windows 上的符号链接](https://github.com/334456777/WeFlow/wiki/开发指南#windows-上的符号链接)；没有生效时 `make docs-check` 的 `symlinks_are_checked_out_as_links` 失败。
- Linux 的编译和测试在 WSL 里做，target 目录放在 WSL 自己的磁盘上（如 `$HOME/wf-target`），比放在 `/mnt/c` 下快得多；worktree 用单独的 WSL target 目录，用完删除。需要的 Linux 二进制复制到 Windows 侧，用完删除，增量缓存保留。
- Windows 侧的 cargo 配置了 `rustc-wrapper`（如 sccache）时，WSL 里的 cargo 会沿目录向上读到 `/mnt/c/Users/<用户>/.cargo/config.toml`，报找不到那个 `.exe`：在 WSL 的命令前加 `RUSTC_WRAPPER=`。
- 经 `wsl.exe` 传含 `|`、多层引号或 `$()` 的命令会被截断或提前展开：写成脚本文件，再 `wsl.exe bash <脚本>`。从 Git Bash 调用时加 `MSYS_NO_PATHCONV=1`，否则 `/tmp/...` 这类参数会被改写成 Windows 路径。写在 `bash -lc '…'` 里的 `pkill -f` 会匹配并杀掉这个 bash 自己。
- 对 `//wsl.localhost/...` 下的仓库运行 git 会报 dubious ownership，在 WSL 里面运行。
- 在 WSL 里测网页：把 `git ls-files -co --exclude-standard web` 列出的文件打包，解压到 WSL 的磁盘上再 `bun install --frozen-lockfile`，不在 `/mnt/c` 上安装依赖。

## 接手只在一个平台上编译过的分支

例如云端会话的分支只在 Linux 上编译、跑过 `make ci`，`#[cfg(windows)]` 的代码和只在 Windows 上编译的测试从没编译过。接手后，在自己的 worktree 里对 `origin/main..` 的每个提交运行 `cargo fmt --all -- --check` 和 `cargo clippy --workspace --all-targets -- -D warnings`（用单独的 `CARGO_TARGET_DIR`），修好再开 PR 或建 stack；否则 Windows 的 CI 会在每一层都编译失败。
