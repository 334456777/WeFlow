---
name: weflow-tests
description: 给 WeFlow 的 Rust 代码写测试：选哪一层、用合成账号写端到端测试（tests/common、fixture 缺表时怎样补）、TestDir 与绑定的顺序、性质测试（proptest、property_cases、PROPTEST_CASES）、假模型服务、Kani 证明（只在 Linux 或 WSL、先 ulimit -v），以及测试辅助代码的 lint 和平台限定。写或改 Rust 测试、复现 bug、给解析或分页加性质测试、跑 Kani 时使用。
---

# 写 Rust 测试

测试的分层、合成账号、假模型服务、测试目录、性质测试和 Kani 的设计见 Wiki [测试与基准](https://github.com/334456777/WeFlow/wiki/测试与基准)，单个测试的命令见 [CLAUDE.md](../../../CLAUDE.md) 的「命令」。网页的测试见 [weflow-web-feature](../weflow-web-feature/SKILL.md#测试和检查)，serve 的测试见 [weflow-contract](../weflow-contract/SKILL.md#坑) 和 [weflow-cli-command](../weflow-cli-command/SKILL.md#测试)。

## 选哪一层

| 要测什么 | 写在哪里 |
|---|---|
| 一个函数的输入输出、解析、格式 | 被测函数所在模块的 `#[cfg(test)] mod tests` |
| 对任意输入都应成立的性质（分页、解密、解码、导出无损） | 同一个 `mod tests` 里的 proptest |
| 从加密数据库读到服务的结果 | `crates/weflow-core/tests/<领域>_e2e.rs`，用合成账号 |
| 运行中的 `weflow serve` | `crates/weflow-cli/tests/serve_api/` |
| 有界输入下的内存安全和算术 | `weflow-native` 的 `#[cfg(kani)] mod proofs` |

修 bug 时先写一个在修复前失败的测试，放在最低能复现它的那一层。

## 端到端测试和合成账号

- `tests/common` 提供 `mock_hub`（标准账号）、`custom_hub`（在闭包里用 `Fixture` 自己建库）、`hub_for_account` 等。标准账号不够用时用 `custom_hub`，不改 `Fixture::standard`：别的测试依赖它的内容。
- 合成账号缺少要测的表或列时，在 `weflow_native::fixture` 里加 `XxxSpec` 和建库的方法（参照 `session_db`、`message_shard`），表结构照真实账号的写，可以先用 `schema_probe` 查，见 [weflow-real-account](../weflow-real-account/SKILL.md#调研表结构)。网页开发和冒烟测试用的合成账号也要用到时，同步改 `crates/weflow-cli/examples/dev_account.rs`。
- 返回账号或 hub 的辅助函数把 `TestDir` 放在元组第一位：`let (root, hub) = common::mock_hub("tag")`。后绑定的变量先被丢弃，所以 hub 先关闭打开的文件，目录后删除；反过来写在 Windows 上会删不掉目录。
- 要写文件时用 `TestDir` 或 `common::temp_dir` 建自己的目录，不自己拼 `std::env::temp_dir()` 下的路径。要看测试留下的文件时设 `WEFLOW_KEEP_TEST_DIRS`，看完手动删除。
- 测 AI 见解和模型接口时，用 `weflow_fake_model::FakeModel::start()` 在进程内启动假模型服务，用模型名切换场景（限流、过载、拒绝回答等），场景表见 Wiki [测试与基准#假模型服务](https://github.com/334456777/WeFlow/wiki/测试与基准#假模型服务)。不连真实的模型服务。

## 性质测试

- 测试名以 `any_` 开头，写成一句断言（如 `any_page_decrypts_to_its_plain_bytes`）。
- 用 `#![proptest_config(crate::property_cases(<用例数>))]` 设用例数：CI 用这个较小的数，本地设 `PROPTEST_CASES` 调高。
- 失败时打印缩小后的最小输入，但不写回源码目录：要保留这个用例，把它写成一个普通的单元测试。

## Kani 证明

- 只能在 Linux 或 macOS 上运行，Windows 上在 WSL 里运行（target 目录放在 WSL 自己的磁盘上，见 [weflow-cross-platform](../weflow-cross-platform/SKILL.md#windows-上用-wsl)）。不在 CI 中运行。
- 运行前必须先 `ulimit -v <KB>`，按机器内存的一半左右限制：CBMC 的问题规模失控时会占用几十 GB 内存。

  ```sh
  ulimit -v <KB>
  cargo kani -p weflow-native --lib
  ```

  全部证明约 9 分钟；只跑一个时加 `--harness <名字>`。
- 每个证明的输入上限写在它的注释里；`#[kani::unwind(n)]` 要比循环的最大次数多 1。加了新证明后在 Wiki [测试与基准#Kani 证明](https://github.com/334456777/WeFlow/wiki/测试与基准#kani-证明)的覆盖范围里补上。

## lint 和平台

- 集成测试、示例和构建脚本各自是一个 crate，开头要有 `//!`（`missing_docs`）。
- `unwrap`、`expect` 只在 `#[test]` 函数和 `#[cfg(test)]` 模块里允许（[`clippy.toml`](../../../clippy.toml)）。`tests/common` 这类辅助模块里的普通函数不算测试，要在模块开头写 `#![expect(clippy::unwrap_used, reason = "…")]`。
- 开头写了 `#![cfg(...)]` 的测试文件只在部分平台编译（如 `insight_e2e.rs` 只在 Linux），改了它用到的接口要在那个平台上编译，见 [weflow-cross-platform](../weflow-cross-platform/SKILL.md)。
- 测试依赖路径分隔符、`cfg!(windows)` 或符号链接时，也要在另一个平台上运行一次。
