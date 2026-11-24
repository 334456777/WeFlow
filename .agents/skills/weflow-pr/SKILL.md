---
name: weflow-pr
description: WeFlow 一次改动从开分支到合并、清理的步骤：建 worktree 和分支、推送前的检查、开 PR（标题、正文、标签、看板）、等待 CI、合并（维护者逐个 PR 同意后才合并或开自动合并）、合并后的清理、用 cargo sweep 清理 target/ 的编译缓存，以及叠层 PR（gh stack）和多人并行开发的注意事项。开始改 WeFlow 的代码或文档、准备提交或推送、开 PR、合并 PR 或开自动合并、清理分支、worktree 和编译缓存时使用。
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
   - Windows 上 `target/debug/weflow.exe` 被别的进程占用、链接失败时，换一个 `CARGO_TARGET_DIR`，不加 `--target x86_64-pc-windows-msvc` 绕开：显式给出 `--target` 时，`.cargo/config.toml` 里按目标设的 `rustflags` 不作用于构建脚本和过程宏，所有依赖的哈希都会变，要整套重编。`bun run dev`、`bun run smoke` 运行的是副本，不占用这个文件，占着它的一般是手动启动的 `weflow serve`。
4. 维护者：开始做某个 issue 时，把它和它的父 issue 在看板上设为 `In Progress`，做法见 [weflow-issues](../weflow-issues/SKILL.md)。

## 推送前

- 改了 Rust：对改过的 crate 运行 `cargo fmt --all -- --check` 和 `cargo clippy -p <crate> --all-targets -- -D warnings`。`cargo check` 不报 clippy 的警告，例如删掉函数后留下的孤立文档注释（`empty_line_after_doc_comments`）。完整的 `make ci` 由 CI 运行，本地只在升级工具链时运行。
  - 格式检查放在最后一次编辑之后、提交之前：clippy 通过后又改了几行，CI 的 fmt-check 照样失败。
  - 改了文档注释里的链接（`` [`名字`] ``）或可见性时，再运行 `make rustdoc`：clippy 不检查文档链接，链接指向的项不存在或不可见时 rustdoc 的警告在 CI 里是错误。
- 平台相关的代码或测试，或改了公开函数的签名：见 [weflow-cross-platform](../weflow-cross-platform/SKILL.md)。
- 改了契约类型、它们的文档注释或路由：见 [weflow-contract](../weflow-contract/SKILL.md)。改了 HTML 导出模板：见 [weflow-html-template](../weflow-html-template/SKILL.md)。
- 改了 `web/`：在 `web/` 下运行 `bun run check`。
- 改了 README、CLAUDE.md 或 skill：`make docs-check`。
- 碰后端的功能：合并前在真实账号上验证一次，见 [weflow-real-account](../weflow-real-account/SKILL.md)。
- 注释和文档只写现状，写完用 [weflow-wiki](../weflow-wiki/SKILL.md#写法) 里的扫描命令检查新增的文字。

## 开 PR

1. `git push -u origin <分支>`。
2. `gh pr create`：标题、正文的语言和关闭 issue 的关键词按 CLAUDE.md 的「提交与 Pull Request」，正文说明做了什么、怎么验证的，链接的写法见 [weflow-issues](../weflow-issues/SKILL.md#写正文和评论)。长期任务 issue 只写 `Refs #N`，不写关闭关键词，否则合并时会关掉它。
3. 维护者：加标签、加入看板并设置 `In Review` 和 `Priority`，做法见 [weflow-issues](../weflow-issues/SKILL.md)。改动会明显影响性能（基准所测的命令、读取层、导出、统计、serve 路由、体积或启动、依赖与编译参数）时，开 PR 时就加 `bench` 标签：合并后的夜间运行会自动测合并前后，测完在 PR 下评论 `BENCH-` 对比（Wiki [测试与基准#夜间队列](https://github.com/334456777/WeFlow/wiki/测试与基准#夜间队列)），漏标就不会测。
4. 看一眼 `gh run list --branch <分支>`。PR 开出后偶尔没有触发任何运行，`mergeable` 一直是 `UNKNOWN`，关闭再重开也没用；手动运行一次（`gh workflow run <工作流>.yml --ref <分支>`）后，PR 自己的运行也会出现。

## 等 CI

- 需要等检查跑完时，能在后台运行命令就用 [`scripts/wait-pr.sh`](scripts/wait-pr.sh) 等待：必需检查全部通过或某项失败、PR 合并或关闭时退出。不要用 `gh pr checks --watch --required` 判断检查是否跑完：汇总检查要等它的任务开始才出现，`--watch` 只看已经出现的检查，会提前以 0 退出。
- 检查失败先修，修好后推送。
- 分支开出后 `main` 新增了 ruleset 要求的检查时，合并会报「base branch policy prohibits the merge」：rebase 到最新的 `main` 再推送。

## 合并（维护者同意后）

- 默认不合并，也不开自动合并：检查跑完后把结果报给维护者，由维护者决定。
- 维护者同意合并某个 PR 后才合并，合并前核对标题和正文与最终的改动一致：检查已经通过就 `gh pr merge <n> --merge`；还在进行就开启自动合并 `gh pr merge <n> --auto --merge`，检查通过后由 GitHub 合并（PR 已经是 `CLEAN` 状态时 `--auto` 会立即合并）。开了自动合并后推送修复不会取消它。
- 同意以一个 PR 为单位：只对维护者点名的那个 PR 有效，不延伸到之后的 PR、同一批的其他 PR 或叠层 PR 的其他层。
- 例外是修 bug 的智能体的 PR，按 [`.claude/loop.md`](../../../.claude/loop.md) 的「要求」合并：它从 `bug` 标签的 issue 修的直接开自动合并；别的智能体委托它修的，由委托方决定合不合并，委托时可以说明，也可以等它附上修复结果来问。
- 合并的条件见 Wiki [发布与 CI#合并的条件](https://github.com/334456777/WeFlow/wiki/发布与-CI#合并的条件)。不绕过 ruleset：不用 `--admin`，不直接推送 `main`。

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

## 清理编译缓存

`target/` 下的产物不会自动删除，用 `cargo sweep` 清理。每种删除命令第一次运行时先加 `--dry-run` 预览。

- 精确清理：`cargo sweep --stamp` → 把实际会用到的 profile 都正常编译一次（如 `cargo build`、`cargo build --release`、`cargo test --no-run`）→ `cargo sweep --file`，删除时间戳之后没被用到的产物。
- 日常清理：`cargo sweep --time 30` 删除 30 天没用过的产物；`cargo sweep --installed` 删除已卸载的工具链编译的产物。
- 不用 `cargo clean` 腾空间。必须清掉某个 crate 时，`cargo clean -p <crate>` 不带 `--profile` 会删掉它在所有 profile 下的产物，只清 debug 时加 `--profile dev`；限定了 profile，增量编译的缓存也可能很大，同样先加 `--dry-run` 看会删多少。

## 会关掉 PR 的操作

- 在 GitHub 上重命名分支：以它为 head 的开放 PR 被关闭，而且无法重开。必须改名时立刻开替代 PR，复制描述、标签和看板条目，并在旧 PR 上留言。
- `gh pr merge --delete-branch` 合并父 PR：以它为 base 的子 PR 被关闭，不会改指向 `main`。补救：把分支临时推回原提交 → `gh pr reopen` → `gh pr edit --base main` → 再删分支。GitHub 合并后自动删除 head 分支时，子 PR 会改指向 base。

## 叠层 PR（gh stack）

- 维护者同意合并整个 stack 时，一次合并：`gh stack merge <顶层 PR> --yes --merge`。`main` 上只多一个合并提交，标题取顶层 PR，各层的关闭关键词照样生效。
- 只有一层时不算 stack：`gh stack merge` 报「is not a stack number or a stacked pull request」，改用 `gh pr merge <PR> --merge`。
- `gh stack push` 会把预留的空层也推送到远程，合并后连同它们一起删除。
- 合并后本地的 stack 记录不会删除：删完分支后运行 `gh stack unstack <stack 编号> --local`，编号是 `submit` 时打印的「stack #N」，不是 PR 号。旧记录攒到两个以上时，在 `main` 上运行 `gh stack view` 会报「main belongs to multiple stacks」。
- 未跟踪的文件也算未提交的改动，`gh stack rebase`、`add`、`submit` 会中止：先 `git stash push -u -m <标签> -- <路径>`，跑完再取回。
- 已有的分支和 PR 可以用 `gh stack init <底层分支> <新分支>` 接管成 stack。

## 并行开发

几个智能体或会话同时改代码时：

- 每个 worktree 用自己的 `target/wt-<名>`。
- 按文件分批，写明每批可以改的文件和别人在改的文件；有依赖的改动先建 issue 设为 `Blocked`，前一个合并后再开始。
- 共用临时目录时，文件名带分支名（如 `temp/pr-body-<分支>.md`），避免互相覆盖。
- 被别人还没合并的修复阻塞时，可以把它分支上的文件临时放进自己的工作区验证，不提交。
- 先合并的 PR 改了后面的 PR 用到的接口时，后面的 PR rebase 到最新的 `main`、本地 clippy 通过后再推送：GitHub 显示能合并不代表能编译。
- 已结束的智能体锁住的 worktree，先 `git worktree unlock` 再删除。
