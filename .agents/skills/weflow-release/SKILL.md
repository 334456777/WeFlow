---
name: weflow-release
description: 发布 weflow 和 WebUI 的新版本（维护者）：定版本号并问要不要跑 bench、发版 PR、推送 web-v tag、运行 release.yml、发布后检查 PGO 训练的警告、给 Wiki 打 tag，以及维护 PGO 训练清单、补发旧版本和修改发版工作流时的坑。要发版、打 tag、发布 rc 或补发旧版本，或删改 CLI 参数和 API 必填参数时使用。
---

# 发版

步骤和原因见 Wiki [发布与 CI#发版步骤](https://github.com/334456777/WeFlow/wiki/发布与-CI#发版步骤)，版本号和 tag 的规则见 [CLAUDE.md](../../../CLAUDE.md) 的「版本与 tag」。下面是执行时要做的事和容易出错的地方。

## 先问维护者

1. 看现有的 tag、发布和上次发布以来的提交：`git tag --sort=-v:refname | head`、`gh release list`（Latest 与 rc 以它为准）、`git log <上一个 tag>..origin/main --oneline`。按 SemVer 判断升哪一位：删改命令、参数、配置键等不兼容的改动升主版本号。
2. 把版本号和理由告诉维护者，同时问这一版要不要跑 bench（`release.yml` 手动运行时的 `bench` 选项，默认不开），并附一句判断：这一版有没有明显影响性能的改动。确认后再继续。
3. API 版本：比较最新 `web-v*` tag 中 `web/package.json` 的 `weflow.apiVersion` 和 `origin/main` 上的 `API_VERSION`，规则见 CLAUDE.md。

## 发版 PR

- 改根 `Cargo.toml` 的 `[workspace.package] version`，运行 `cargo update -w` 刷新 `Cargo.lock`；有网页改动时改 `web/package.json` 的 `version`。
- 新网页依赖这一版 weflow 才有的功能或 API 时，把 `weflow.minBackendVersion` 提到这一版。在发版 PR 里改，不提前改：`main` 上编出的 weflow 报告的版本是 `X.Y.Z-N-g<提交>`，按 SemVer 比上一版 `X.Y.Z` 还小，提前改会让开发页和 `--ui-dir` 判定为不兼容。
- 这个 PR 自己的 CI 全部通过后再合并，只改版本号的 PR 也一样：版本号会改变 `serve_api` 测试的行为（[#381](https://github.com/334456777/WeFlow/pull/381)）。

## 发布

1. 在发版 PR 的合并提交上打附注 tag 并推送，`git cat-file -t` 应输出 `tag`；等 `web-release.yml` 完成。网页必须先于 weflow 发布。

   ```sh
   git tag -a web-vX.Y.Z -m "WeFlow WebUI X.Y.Z" <合并提交>
   git push origin web-vX.Y.Z
   ```

2. 运行 `release.yml`，tag 由 github-actions[bot] 建在 `main` 的最新提交上。要跑 bench 时加 `-f bench=true`，`bench_base` 留空即与上一版比较。

   ```sh
   gh workflow run release.yml --ref main -f tag=vX.Y.Z -f notes="…"
   ```

   `notes` 和手动修改的发布说明里，不是指人的 `@` 写在反引号里（见 [weflow-issues](../weflow-issues/SKILL.md#写正文和评论)），否则同名的账号会被列进发布页的 Contributors。

3. 发布完成后，在各平台 `build` job 的日志里查 `PGO training run failed` 和 `PGO serve training` 两种警告：单条训练失败只给警告，发布照常完成，那部分代码按冷代码优化。有就修训练清单。
4. 在 Wiki 仓库的最新提交上打与版本同名的附注 tag 并推送。

## PGO 训练清单

CLI 命令和参数写死在 `.github/scripts/pgo-build.sh`，`serve` 的读取路由由 `pgo-serve.py` 从 `/openapi.json` 自动取，必填参数按名字填写。`bench.yml` 测的是普通编译版，发现不了训练清单的失效。

- 删改 CLI 子命令或参数、改 `/api/` 路由必填参数的名字时，同步修改这两个脚本，并在本地运行一次，确认没有警告（Windows 上约 10 分钟）：

  ```sh
  bash .github/scripts/pgo-build.sh x86_64-pc-windows-msvc .exe
  ```

- 训练覆盖所有常用功能，不常用的功能可以少训练；会联网的功能（表情、朋友圈媒体和 HTML 的头像、AI）不进训练。

## 坑

- 补发旧版本、`null == false` 为真：见 Wiki [发布与 CI#补发旧版本](https://github.com/334456777/WeFlow/wiki/发布与-CI#补发旧版本)。改了发版的构建或检查，先用 `test.<名称>` 试跑（[发布与 CI#试跑](https://github.com/334456777/WeFlow/wiki/发布与-CI#试跑)）。
- Windows 的 Git Bash 会把 `origin/x:.github/y`、`/users/...` 这类参数改写成本地路径：设 `MSYS_NO_PATHCONV=1`，`gh api` 的端点不以 `/` 开头。
- `gh run download` 不覆盖已有的文件，重新下载前先清空目录。
- 推送历史 tag 又不想触发工作流时，先 `gh workflow disable Release`，推完再 `gh workflow enable Release`；禁用期间的事件直接丢弃。
