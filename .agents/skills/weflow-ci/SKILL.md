---
name: weflow-ci
description: 修改 WeFlow 的 GitHub Actions 工作流（.github/workflows/ 和 .github/scripts/）时的做法和坑：新工作流要有 changes 和汇总任务并登记进 ruleset、改了发版流程先用 test.<名称> 试跑、改缓存后核对是否真的复用了产物、bot 推送后的批准运行、GitHub 表达式和 GITHUB_TOKEN 的限制。改 CI 或发版的工作流和脚本、排查工作流为什么没运行或运行异常时使用。
---

# 改 CI 工作流

各个工作流的作用和合并的条件见 Wiki [发布与 CI](https://github.com/334456777/WeFlow/wiki/发布与-CI)。

- 新增在 PR 上运行的工作流时，同样加上 `changes` 和汇总任务，并把汇总任务加进 `main` 的 ruleset（维护者），见 [发布与 CI#合并的条件](https://github.com/334456777/WeFlow/wiki/发布与-CI#合并的条件)。新增要检查的文件时，同步修改相关工作流的 `on.push.paths` 和 `changes` 任务里的 `case`。
- 改了 `release.yml`、`pgo-build.sh`、`release-check.sh` 等发版流程：先试跑 `gh workflow run release.yml --ref <分支> -f tag=test.<名称>`，见 [发布与 CI#试跑](https://github.com/334456777/WeFlow/wiki/发布与-CI#试跑)。
- GitHub 表达式里 `null == false` 为真：推送 tag 触发时 `inputs.x` 是 `null`，判断手动输入要加上 `github.event_name == 'workflow_dispatch'`。
- `GITHUB_TOKEN` 在旧提交上创建 release 会得到 403，见 [发布与 CI#补发旧版本](https://github.com/334456777/WeFlow/wiki/发布与-CI#补发旧版本)。
- 改缓存：命中缓存不等于复用了产物。缓存的 key 只看 Cargo 清单和工具链，不看 feature 的组合：`cargo test -p <crate>` 与 `--workspace` 编出的依赖 feature 不同，共用 key 时命中后仍要重编几十个依赖。
  - 改了缓存配置后，对比作业的耗时，并数日志里 `Compiling` 的行数；不同的 cargo 命令（`-p` 与 `--workspace`、不同的 profile）不共用 key。
  - 验证保存行为：在临时分支上把 `save-if` 改为 true，跑两次 `test.<名称>` 比较，测完删除分支、缓存（`gh cache delete <id>`）和这些运行（`gh run delete <id>`）。
  - 各作业缓存的现状写在 `ci.yml`、`pgo-build.sh` 的注释里，以那里为准。新增缓存后在 `main` 上手动运行一次 `ci.yml` 补存。
- `html-template.yml` 以 github-actions[bot] 推送副本后，PR 页面出现批准运行的按钮；批准后 GitHub 按 bot 的那次提交运行工作流，运行的 `actor` 是 bot，`triggering_actor` 是批准的人（`gh api repos/334456777/WeFlow/actions/runs/<id> --jq .triggering_actor.login`）。
- PR 开出后没有任何运行：见 [weflow-pr](../weflow-pr/SKILL.md#开-pr)。
- 自托管 runner（`weflow-bench`）不能被 `pull_request` 触发的 job 使用，见 [测试与基准#自托管 runner](https://github.com/334456777/WeFlow/wiki/测试与基准#自托管-runner)。
