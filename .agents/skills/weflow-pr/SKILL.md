---
name: weflow-pr
description: WeFlow 一次改动从开分支到合并、清理的步骤：建 worktree 和分支、推送前的检查、开 PR（标题、正文、标签、看板）、自动合并与等待 CI、合并后的清理，以及叠层 PR（gh stack）和多人并行开发的注意事项。开始改 WeFlow 的代码或文档、准备提交或推送、开 PR、合并 PR、清理分支和 worktree 时使用。
---

# 从开分支到合并

规则写在 [CLAUDE.md](../../../CLAUDE.md) 的「提交与 Pull Request」「代码与工作副本」「Issue 与看板」；这里是按顺序执行的步骤和容易出错的地方。标「维护者」的步骤需要仓库的写权限或看板权限，从 fork 贡献时跳过，由维护者完成。

## 开始

1. `git branch --show-current` 确认当前分支。每个修复和功能各开一个分支，不在别人的分支或共用的工作副本里提交。
2. 在 `.claude/worktrees/<名>` 建 worktree 和分支，分支名 `<type>/<scope>-<description>`：

   ```sh
   git fetch origin main
   git worktree add .claude/worktrees/<名> -b <type>/<scope>-<description> origin/main
   ```

3. 在 worktree 里编译 Rust 时，`CARGO_TARGET_DIR` 设为主工作副本 `target/` 下的 `wt-<名>`，原因见 Wiki [开发指南#提交与 Pull Request](https://github.com/334456777/WeFlow/wiki/开发指南#提交与-pull-request)。`bun run dev`、`bun run smoke` 按同一个变量找编译出的 `weflow`。
4. 维护者：开始做某个 issue 时，把它和它的父 issue 在看板上设为 `In Progress`，做法见 [weflow-issues](../weflow-issues/SKILL.md)。

## 推送前

- 改了 Rust：对改过的 crate 运行 `cargo fmt --all -- --check` 和 `cargo clippy -p <crate> --all-targets -- -D warnings`。`cargo check` 不报 clippy 的警告，例如删掉函数后留下的孤立文档注释（`empty_line_after_doc_comments`）。完整的 `make ci` 由 CI 运行，本地只在升级工具链时运行。
- 平台相关的代码或测试，或改了公开函数的签名：见 [weflow-cross-platform](../weflow-cross-platform/SKILL.md)。
- 改了契约类型、它们的文档注释或路由：见 [weflow-contract](../weflow-contract/SKILL.md)。改了 HTML 导出模板：见 [weflow-html-template](../weflow-html-template/SKILL.md)。
- 改了 `web/`：在 `web/` 下运行 `bun run check`。
- 改了 README、CLAUDE.md 或 skill：`make docs-check`。
- 碰后端的功能：合并前在真实账号上验证一次，见 [weflow-real-account](../weflow-real-account/SKILL.md)。
- 注释和文档只写现状，写完用 [weflow-wiki](../weflow-wiki/SKILL.md#写法) 里的扫描命令检查新增的文字。

## 开 PR

1. `git push -u origin <分支>`。
2. `gh pr create`：标题写成 `type(scope): 中文描述`；正文用中文，说明做了什么、怎么验证的，关闭 issue 用 `closed #N`、`fixed #N` 或 `resolved #N`。长期任务 issue 只写 `Refs #N`，不写关闭关键词，否则合并时会关掉它。
3. 正文里的裸链接两侧留空格，或写成 `[文字](url)`：GitHub 会把紧挨着的中文算进链接。
4. 维护者：加标签、加入看板并设置 `In Review` 和 `Priority`，做法见 [weflow-issues](../weflow-issues/SKILL.md)。改动会明显影响性能（基准所测的命令、读取层、导出、统计、serve 路由、体积或启动、依赖与编译参数）时，开 PR 时就加 `bench` 标签：合并后夜间的定时运行会自动测合并前后（Wiki [测试与基准#夜间队列](https://github.com/334456777/WeFlow/wiki/测试与基准#夜间队列)），漏标就不会测。
5. 看一眼 `gh run list --branch <分支>`。PR 开出后偶尔没有触发任何运行，`mergeable` 一直是 `UNKNOWN`，关闭再重开也没用；手动运行一次（`gh workflow run <工作流>.yml --ref <分支>`）后，PR 自己的运行也会出现。

## 合并（维护者）

- 合并的条件和自动合并见 Wiki [发布与 CI#合并的条件](https://github.com/334456777/WeFlow/wiki/发布与-CI#合并的条件)。开启自动合并：`gh pr merge <n> --auto --merge`。PR 已经是 `CLEAN` 状态时，`--auto` 会立即合并。
- 不绕过 ruleset：不用 `--admin`，不直接推送 `main`。
- 以下情况先不开自动合并：改动需要维护者先看（方案有未定的取舍、用户可见的大改动、要在真实环境里确认）、PR 引用的 Wiki 还没推送、叠层 PR 的上层、发版相关的 PR。
- CI 跑完再汇报结果，不在检查进行中下结论。能在后台运行命令时，用 [`scripts/wait-pr.sh`](scripts/wait-pr.sh) 等待：PR 合并或关闭、或某项必需检查失败时退出。不要用 `gh pr checks --watch --required` 判断检查是否跑完：汇总检查要等它的任务开始才出现，`--watch` 只看已经出现的检查，会提前以 0 退出。
- 检查失败先修，修好后推送；推送不会取消自动合并。
- 分支开出后 `main` 新增了 ruleset 要求的检查时，合并会报「base branch policy prohibits the merge」：rebase 到最新的 `main` 再推送。

## 合并后清理

1. 先确认 `gh pr view <n> --json state -q .state` 是 `MERGED`。合并和删分支不写在同一条命令里：合并失败后删了分支，PR 会被关闭。
2. 在主工作副本里 `git pull`，再删 worktree、它的 `target/wt-<名>` 和本地分支。当前目录不能在要删的 worktree 里，否则会留下一个空目录。

   ```sh
   git worktree remove .claude/worktrees/<名>
   git branch -D <分支>
   git fetch --prune
   ```

   远程的 head 分支由 GitHub 在合并后自动删除。没合并就关闭的 PR，远程分支要手动删除，分支名逐个写出。
3. 维护者：PR 和它关闭的 issue 在看板上设为 `Done`。

## 会关掉 PR 的操作

- 在 GitHub 上重命名分支：以它为 head 的开放 PR 被关闭，而且无法重开。必须改名时立刻开替代 PR，复制描述、标签和看板条目，并在旧 PR 上留言。
- `gh pr merge --delete-branch` 合并父 PR：以它为 base 的子 PR 被关闭，不会改指向 `main`。补救：把分支临时推回原提交 → `gh pr reopen` → `gh pr edit --base main` → 再删分支。GitHub 合并后自动删除 head 分支时，子 PR 会改指向 base。

## 叠层 PR（gh stack）

- 一次合并整个 stack：`gh stack merge <顶层 PR> --yes --merge`。`main` 上只多一个合并提交，标题取顶层 PR，各层的关闭关键词照样生效。
- 只有一层时不算 stack：`gh stack merge` 报「is not a stack number or a stacked pull request」，改用 `gh pr merge <PR> --merge`。
- 合并后本地的 stack 记录不会删除：删完分支后运行 `gh stack unstack <stack 编号> --local`，编号是 `submit` 时打印的「stack #N」，不是 PR 号。旧记录攒到两个以上时，在 `main` 上运行 `gh stack view` 会报「main belongs to multiple stacks」。不要手改 `.git/gh-stack`。
- 未跟踪的文件也算未提交的改动，`gh stack rebase`、`add`、`submit` 会中止：先 `git stash push -u -m <标签> -- <路径>`，跑完再取回。
- 已有的分支和 PR 可以用 `gh stack init <底层分支> <新分支>` 接管成 stack。

## 并行开发

几个智能体或会话同时改代码时：

- 每个 worktree 用自己的 `target/wt-<名>`。
- 按文件分批，写明每批可以改的文件和别人在改的文件；有依赖的改动先建 issue 设为 `Blocked`，前一个合并后再开始。
- 共用临时目录时，文件名带分支名（如 `temp/pr-body-<分支>.md`），避免互相覆盖。
- 先合并的 PR 改了后面的 PR 用到的接口时，后面的 PR rebase 到最新的 `main`、本地 clippy 通过后再推送：GitHub 显示能合并不代表能编译。
- 已结束的智能体锁住的 worktree，先 `git worktree unlock` 再删除。
