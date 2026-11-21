---
name: weflow-upgrade
description: 升级 WeFlow 用到的工具和依赖（Rust 工具链、crate、Bun 与 npm 包、GitHub Action 等）：只用发布满 3 天的版本、查发布时间的方法、PR 里要写的内容，以及用 [patch.crates-io] 固定在 fork 上的依赖何时、如何换回 crates.io。升级或新增依赖、处理 dependabot 的 PR、检查 fork 的上游是否已经发版时使用。
---

# 升级工具和依赖

## 3 天的安全期

- 只升到发布满 3 天的版本：刚发布的版本可能被撤回、紧急补丁，或者是供应链投毒。不满 3 天时用上一个满 3 天的版本，或者等期限过了再升；例外只由维护者逐个批准。
- 查发布时间：
  - npm 包：`bun info <包> time --json`
  - crate：`https://crates.io/api/v1/crates/<名>/versions` 里各版本的 `created_at`
  - GitHub 上的项目和 Action：`gh api repos/<owner>/<repo>/releases --jq '.[] | {tag_name, published_at}'`
- `web/bunfig.toml` 已设 `[install] minimumReleaseAge = 259200`（秒），`bun install` 只接受满 3 天的版本。
- PR 描述写明所选版本的发布日期和距今几天；不满 3 天的写明暂不升级、还差几天。

## Rust 工具链

改 `rust-toolchain.toml` 的 `channel`，运行 `make ci`，把新增的 clippy 警告修掉，和工具链的改动一起提交。

## 固定在 fork 上的依赖

根 `Cargo.toml` 的 `[patch.crates-io]` 把几个依赖固定在 fork 的提交上，用途见 Wiki [发布与 CI#第三方依赖补丁](https://github.com/334456777/WeFlow/wiki/发布与-CI#第三方依赖补丁)。换回 crates.io 之前，fork 和那个提交不能删除。

上游合并了补丁、发布了新版本，并且新版本满 3 天之后：

1. 删掉 `[patch.crates-io]` 里的这一项，升级依赖（`cargo update -p <名>`）。
2. 确认补丁带来的东西仍然在：
   - `heic-rs`：`crates/weflow-core/src/hevc.rs` 的 `every_layout_and_bit_depth_decodes_to_ffmpegs_samples` 仍然通过；再在真实账号上用 `wxgf_coverage_probe` 输出的 `decoded_jpegs_sha256` 比对前后，见 [weflow-real-account](../weflow-real-account/SKILL.md)。
   - `sha2`：weflow-native 的测试通过；再用 Linux perf 交替测量 analytics，确认收益仍在，见 [weflow-perf](../weflow-perf/SKILL.md#linux-perf-交替测量)。
