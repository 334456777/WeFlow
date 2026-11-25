---
name: weflow-perf
description: 测量和对比 WeFlow 的性能：选测法、serve_bench 的两种账号、新旧二进制的对比（单独的 target 目录、旧新旧三轮、--compare 的假阳性、hyperfine 的坑）、Linux perf 交替测量与热点定位（对照组、让其他任务暂停）、导出的墙钟（输出写到固态盘、先弄清在等什么）、内存优化要同时报告磁盘读写，以及 bench 标签、在 runner 上试跑 bench.yml、删除和重测已发布的结果（连同 runner 的 pending 副本）、装 runner 和校准基准账号。做性能优化、比较改动前后的速度或内存、改基准脚本或 bench.yml、删除或重测基准结果、维护基准的 runner 时使用。
---

# 性能测量

基准的设计见 Wiki [测试与基准](https://github.com/334456777/WeFlow/wiki/测试与基准)：serve_bench、bench.yml 和基准账号。

## 选测法

| 要回答的问题 | 测法 |
|---|---|
| 答案是否不变 | 新旧二进制 `--json` 逐项对比，见 [weflow-real-account](../weflow-real-account/SKILL.md#改动前后对比) |
| 对实际使用的影响 | serve_bench 两种账号都测；新旧二进制在真实账号上的耗时 |
| 几个百分点的小幅改动是否真的更快 | Linux perf 交替测量 `cycles:u` |
| 合并前后、各版本之间的长期趋势 | bench.yml（维护者的自托管 runner） |

## 两种账号

serve_bench 每次都同时测合成账号和真实账号：合成账号检查答案是否正确、两条路由是否一致，真实账号测耗时和并发。性能优化的 PR 附上两者的结果，基线和 `--compare` 用真实账号的结果。真实账号的规矩见 [weflow-real-account](../weflow-real-account/SKILL.md)。

## 新旧二进制

- 旧提交用 `git worktree add --detach target/<目录>/old-src <提交>` 建在被忽略的目录里。
- 每个版本用单独的 `CARGO_TARGET_DIR`，用完删除。两个工作副本共用 target 时，cargo 会把另一份源码编出的产物当成最新的直接复用：对比显示没有差异，或者报「方法不存在」，`--version` 也可能停在旧提交。`touch` 源文件、新建 worktree 都不保险。
- 复制二进制之前核对编译的退出码：`| tail`、`| grep | head` 会吞掉它（加 `set -o pipefail`），否则编译失败后会把上一个版本的二进制当成「旧版」。复制后确认两个文件确实不同。
- 用同一个（新版的）`serve_bench` 加 `--weflow` 分别测两个二进制，按「旧、新、旧」跑三轮，以两轮旧版之间的差距作为噪声。
- 跨版本对比时不带 `--compare`：旧版没有的 `/api/` 路由返回 404（约 0.4 ms），新版真正计算后被判为变慢并以 1 退出，退出前也不写表格和 `--save` 文件。两边都 `--save` 后自己比较；这类路由拿旧版的子进程路由和新版的 `/api/` 比较。
- hyperfine 的 `-N` 按 POSIX 规则拆分命令行，Windows 的反斜杠路径会被吃掉：传正斜杠路径（Python 里 `as_posix()` 加 `shlex.join`）。用 `-n` 命名后，JSON 的结构是 `name`、`summary.time_wall_clock.median` 和 `measurements`。

## Linux perf 交替测量

- 分支和 `main` 用同样的配置（帧指针加 line-tables）编译，各用自己的 target 目录；在基准账号最大的会话上交替各跑 7 次导出，报告 `instructions:u`、`cycles:u`、`task-clock` 的中位数。先跑完 `main` 再跑分支会差好几个百分点，必须交替。
- 快慢看 `cycles:u`：SIMD 的改动可能让指令数增加而速度更快。指令数不受其他进程影响，一起报告，用来分辨周期数的变化是不是噪声。
- 把基线复制一份、换个名字，与基线和分支一起测，作为对照：几个二进制每轮各跑一次，顺序逐轮轮换。对照与基线的差距就是这次测量的噪声，收益要明显超过它才算数。
- 同一台机器上有编译、测试等其他工作时，`cycles:u` 的波动大到能掩盖 1%～2% 的收益。测量前先让其他会话和任务暂停，并记下每一轮期间其他进程用掉的 CPU。在机器繁忙时判为「在噪声内」的改动，安静时重测可能就在噪声之外（[#1232](https://github.com/334456777/WeFlow/pull/1232)）。
- 标准库的 `str::contains` 对短串已有 SIMD 的快速路径，比每次新建查找器的 `memchr::memmem` 快；`str::find`、`rfind` 逐字节搜索，换成 memmem 有效。
- `perf report --children` 的 self 列对内联函数不可信：动手改之前，用 `perf script -F ip,sym` 按叶子帧计数，或用 `--no-inline` 的调用链复核。
- `perf script -F srcline` 只给文件名，不带目录，按「最近的 weflow 函数名 @ 行号」汇总才分得清。
- Windows 上在 WSL 里测，见 [weflow-cross-platform](../weflow-cross-platform/SKILL.md#windows-上用-wsl)。

## 导出的墙钟

- 输出写到固态盘。写很多文件的导出（带媒体时上万个）写到机械硬盘时，墙钟由盘的写速度决定：CPU 上的改动看不出差别，写入排队这类并发问题也被掩盖（[#1231](https://github.com/334456777/WeFlow/pull/1231)）。
- 墙钟远大于 CPU 时间时，先弄清进程在等什么：边导出边每隔几百毫秒采样一次它的线程数、正在运行的线程数和累计的 CPU 时间（Windows 上是 `Get-Process` 的 `Threads`、`TotalProcessorTime`）。CPU 长时间几乎不涨、只有一个线程在内核里等待，是在等盘；几个线程里每时刻只有一个在跑，是在等锁。

## 内存与磁盘读写

内存优化要同时测磁盘读写的次数和字节数（`bench.py` 的结果里有 `read_bytes`、`write_bytes`）。不靠把数据暂存到磁盘来降低内存：把大会话的正文暂存到临时 SQLite 能省约 30% 的峰值内存，但读写次数多约 9 倍（[#135](https://github.com/334456777/WeFlow/issues/135)），导出到机械硬盘或 U 盘时更慢。读写明显增加时，先把取舍摆给维护者。

导出的内存或线程数变了，同步更新 Wiki [消息导出#线程数的自适应](https://github.com/334456777/WeFlow/wiki/消息导出#线程数的自适应)和[消息导出#内存](https://github.com/334456777/WeFlow/wiki/消息导出#内存)里的数字。

## bench.yml（维护者）

- 会明显影响性能的 PR 开出时就加 `bench` 标签，见 [weflow-pr](../weflow-pr/SKILL.md#开-pr)。汇报时主动建议跑 bench，`commits` 填 `#N前=<合并提交>^,#N后=<合并提交>`。小改动、文档和只改 WebUI 的 PR 不建议。
- 按阶段推进的优化，每做完一阶段跑一次再决定下一阶段；中途只想确认几项时用 `only` 和较少的 `runs`。
- 在 runner 上试跑而不发布结果：在临时分支上给 `bench.yml` 的 `publish` job 加 `if: false`，然后 `gh workflow run bench.yml --ref <分支> -f commits=<提交> -f force=true`，结果从 `bench-results` artifact 取，用完删除分支。从默认分支以外的分支发起的请求不进夜间队列，立即测量。
- `force` 会覆盖 `gh-pages` 上已有的结果，触发前说明；已有结果的提交不加 `force` 会被跳过。
- 删除 `gh-pages` 上的测量结果（`bench/data/<环境>/` 下的文件，以及 `bench/data/index.json` 里的条目）时，runner 目录下 `pending/<环境>/<提交>/` 里的副本也要一起删掉。`bench-commits.py` 的 `load_pending` 只丢掉 `gh-pages` 上已有的结果，留下的副本会在下一次运行时被当成没发布的结果，重新发布出去。
- 删除结果后要重测，并且重新写入 PR 评论或发布说明时：
  - 把这些 PR 移出 `queue.json` 的 `labeled`，下一次夜间运行会把它们重新排进队列，请求里带上 PR 号；
  - 发布说明另外排一个带 `release_tag` 的请求，可以用 `bench_queue.py` 的 `new_request` 生成；
  - 删掉旧评论和旧的那一节之前，先把它们备份下来。
- 看对比结果时先看 `quiet`（其他进程占用的 CPU）是否接近：机器忙时的差距不能归因于代码。
- 改 bench.yml、测量脚本或校准时：
  - 环境键只放真正改变测法的东西；能靠锚点发现的漂移（如操作系统的小版本）记录下来，不进键。测量脚本的改动攒在一起改，用显式的版本号而不是文件的哈希。换环境后补测各个 release，让趋势接得上。
  - 只有真正需要那台机器的步骤才放到 runner 上，判断、排队和发布在 GitHub 托管的机器上做；状态存在 `gh-pages` 和 runner 的 `pending/` 里，运行中断不丢数据，下次接着测。
- 装 runner 或换机器：在管理员终端里运行 `.github/bench/setup-runner.ps1 -Token <令牌> -BenchDir <目录> -Protect <目录>`。令牌用 `gh api -X POST repos/334456777/WeFlow/actions/runners/registration-token -q .token` 取，一小时内有效。runner 的账户不是管理员，读不到用户目录和 `-Protect` 给出的目录；重新运行脚本会更新目录并重新注册。
- 校准基准账号的 profile：以配置了真实账号的用户（不是 runner 的账户）运行 `.github/bench/calibrate.ps1`，`-Register` 把它设成每六周一次的计划任务，改时间也要重新 `-Register`。读的配置默认是 `weflow` 自己读的那份（跟随 `config set config_path` 选定的文件），计划任务记下的是注册时找到的路径，换了配置文件后也要重新 `-Register`。它只输出是否更换、变化的项数和 profile 的哈希；profile 只存在本机，不提交。profile 换了以后，夜间运行会先在新环境里测最近的三个发布和带 `bench` 标签的 PR，见 Wiki [测试与基准#更换 profile 后的补测](https://github.com/334456777/WeFlow/wiki/测试与基准#更换-profile-后的补测)。
- 夜间运行的计划任务：在 runner 所在的电脑上，以 `gh` 已登录的用户（不是 runner 的账户，不用管理员终端）运行 `.github/bench/night.ps1 -Register`，每天日本时间 23:25 发起夜间运行，只在该用户已登录时运行，错过的夜晚不补发。`bench.yml` 没有定时运行，这个计划任务是夜间队列唯一的入口。每次的结果（包括中断它的错误）追加到 `%LOCALAPPDATA%\weflow-bench\night.log`；没有发起时先看这里。任务计划程序里的「上次运行结果」只说明 `pwsh` 有没有启动：`conhost --headless` 不论脚本怎样退出都交回 0。换机器或者 `gh` 换了位置要重新 `-Register`。从分支试跑时，手动运行 `gh workflow run bench.yml --ref <分支> -f commits=`：分支上的夜间运行不碰队列，可以看 `queue` job 是否把它当成夜间运行。见 Wiki [测试与基准#夜间队列](https://github.com/334456777/WeFlow/wiki/测试与基准#夜间队列)。
- 优化合并后，把试过但没采用的做法评论到对应的 issue，见 [weflow-issues](../weflow-issues/SKILL.md#写正文和评论)。
