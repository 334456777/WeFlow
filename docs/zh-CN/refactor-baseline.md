# 重构基线与回归矩阵

[English](../refactor-baseline.md) | **简体中文**

这份文档服务于 #391 的结构重构（#397、#398、#402 等）。它给出改动前的行为与资源基线，以及每个实施 PR 都要跑的回归矩阵：哪些场景、怎么跑、预期是什么、失败了去哪里找原因。调用路径和职责的审计见 [serve-architecture.md](serve-architecture.md)。这里的方法沿用 #16（指标与测量原则）和 #257（前后对照的要求）。

## 实施 PR 要做的事

1. 跑[回归矩阵](#回归矩阵)：合成账号上的正确性测试在 CI 里自动运行，全部通过才能合并。
2. 动到性能或请求量的 PR，在和[基线](#基线结果)相同的条件下重跑[性能测量](#性能测量)，用 `--compare` 对照，并把[对照表](#对照表模板)填进 PR 正文。
3. 有意改变请求或回复的，在 PR 里说明，并同步更新对应测试的预期（例如 `web/src/requests.test.tsx` 的请求列表）。

## 合成数据

所有正确性测试都用合成的加密账号，不需要真实微信数据：

| 用途 | 来源 | 内容 |
|---|---|---|
| Rust 端到端测试 | `weflow_native::fixture`（`crates/weflow-native/src/fixture.rs`），测试运行时自动生成 | `Fixture::standard`：私聊和一个群，两个消息分片（跨分片的游标翻页）、图片、zstd 压缩消息；测试再按需加朋友圈、媒体、第二个账号 |
| 性能测量 | `serve_bench` 在 `<工作目录>/data` 生成，规模参数不变时复用 | 一个群 200,000 条消息，分 4 个消息库；500 个私聊各 30 条；3,000 个联系人；2,000 条朋友圈；最新 1,000 条图片带 `.dat` 文件；搜索索引缺最新 50 条 |
| 手工查看网页 | `crates/weflow-cli/examples/dev_account.rs`，由 `web/` 下的 `bun run dev` 生成在 `target/dev-account/` | 几个好友、一个带图片的群、朋友圈和「图片测试」会话 |
| WebUI 测试 | 测试里的假后端（`fetch` 和 `EventSource` 的替身） | 每个测试自己的会话、消息和账号 |

## 回归矩阵

Rust 场景都在 `crates/weflow-cli/tests/serve_api.rs` 里：真的启动一个 `weflow serve` 进程，发真实的 HTTP 请求，必要时再直接运行同一条命令做对照。WebUI 场景用 `bun test`。CI 中 `ci.yml` 在 Linux、macOS 上运行 `make ci`，Windows 上运行全部测试，`web/` 有改动时 `web.yml` 运行 `bun test`。

| 场景 | 输入 | 预期 | 命令 |
|---|---|---|---|
| CLI 与 HTTP 结果一致 | `config`、`chat` 等命令分别经终端和 HTTP 执行 | 正文与 `--json` 输出逐字节相同，状态码由错误码和退出码决定 | `cargo test -p weflow-cli --test serve_api commands_answer_like_the_terminal account_commands_and_read_only_refusals` |
| `/api/` 与子进程结果一致 | 26 个读请求，含跨分片游标翻页 | 两条路由的状态码、正文和 `x-weflow-*` 头相同 | `… reads_run_in_process_with_the_same_answers` |
| 出错时一致 | 缺必填参数、类型不对、参数互斥、未知参数、命令自身拒绝、查无结果、错误方法、没有或错误的令牌 | 两条路由给出相同的 400 / 200 / 405 / 401；`/api/` 不接受 `std`、`return=file`、`Prefer: respond-async`（400），不在进程内执行的命令返回 404 | `… reads_fail_the_same_in_process_and_in_a_child` |
| 鉴权与 Host | 不带令牌、令牌放在头里或查询参数里、非本机 `Host` | `/health` 系列免令牌，其余 401；非本机 `Host` 403 | `… health_needs_no_token_and_everything_else_does only_requests_to_this_machine_are_answered`；单元测试 `tokens_are_checked_from_the_header_or_the_query` |
| 拒绝写微信库 | `/openapi.json` 标记的每个命令 | 全部 501 `unsupported`，标记集合固定 | `… refused_commands_are_marked_and_refused` |
| 账号切换 | 两个档案指向两个合成账号，经 `PUT /config/set` 来回切 4 次，两个档案名长度相同 | 切换后立刻读取，两条路由都只返回当前账号的会话和消息 | `… reads_follow_the_profile_right_after_it_changes`；`health_data_changes_with_the_configuration_and_each_run` |
| 长任务与取消 | 卡住的 AI 请求作为任务；排队、运行中取消，重复取消，客户端断开 | 取消立即杀掉子进程；第二次取消只是忘记任务，结果不变；断开的同步请求也终止命令 | `… jobs_are_cancelled_and_limited a_job_is_cancelled_once_and_then_forgotten` |
| 进度与 SSE | `backup create` 作为任务；`Last-Event-ID` 重连；服务停止 | 事件依次为 `status`、`progress`、`result`；重连只补发后面的事件；停止时发 `bye` | `… jobs_run_in_the_background event_streams_say_bye_when_the_server_stops health_events_say_hello_then_beat_without_a_token` |
| 消息推送 | 监听期间写入一条新消息 | 推送一次 `message.new` | `… new_messages_are_pushed_as_events` |
| 媒体与文件 | 图片、语音、视频、朋友圈媒体；`Range`、`ETag`；`return=file` | 回复文件本身、分段正确、`304`；任务保留写出的文件 | `… media_come_back_as_files_from_the_process videos_are_sent_in_the_parts_asked_for moments_media_come_from_the_cache_and_only_from_wechat files_come_back_with_the_result jobs_keep_the_file_they_wrote_for_the_browser` |
| 限流 | 超过 `--max-tasks`、`--max-reads` | 任务 429 `Retry-After: 5`；`/api/` 读排队等待 | `… jobs_are_cancelled_and_limited`；单元测试 `requests_wait_in_line_until_it_is_full_or_too_long` |
| WebUI 聊天页的请求 | 启动、换会话、切回、搜索、换账号 | 每一步的请求列表与基线相同；换账号后只显示新账号的内容 | `cd web && bun test src/requests.test.tsx` |
| WebUI 迟到的旧账号回复 | 换账号前发出的请求在换账号后才返回 | 旧回复不进入页面 | 同上 |
| WebUI 页面流程 | 导航、会话与消息、搜索、导出任务（进度、取消、保存）、通讯录、设置、令牌、账号设置、朋友圈、支付记录、主题、减少动画 | 见各测试 | `cd web && bun test`（`src/App.test.tsx` 等） |

**失败了怎么找**：测试名对应上表的场景；断言消息里带着请求路径（例如 `GET /chat/messages?…`）和轮次。Rust 测试的服务进程 stderr 存在 `Server::stderr` 里，可以在断言里打印；需要看服务端每个请求时，启动参数加 `--log-requests`。WebUI 请求列表不符时，断言会给出完整的前后两个列表。

## 性能测量

### 工具

| 工具 | 测什么 | 命令 |
|---|---|---|
| `serve_bench`（`crates/weflow-cli/examples/serve_bench.rs`） | 端到端：每个读请求经两条路由的延迟（中位数、p95、最小、最大、次数）、回复大小、并发、图片和视频、推送引擎的 CPU，以及服务进程整段测量的 CPU 和峰值内存。同时测合成账号（核对答案）和真实账号（测速度） | `cargo build --release -p weflow-cli --bin weflow --example serve_bench`，然后 `target/release/examples/serve_bench <工作目录> --home <WEFLOW_HOME> --save base.json`；改动后加 `--compare base.json` |
| `clap_round_trip`（`crates/weflow-cli/src/serve_api/in_process.rs` 里的 ignored 测试） | 一个 `/api/` 读请求拼命令行、克隆命令树、clap 解析的耗时 | `cargo test --release -p weflow-cli --bin weflow clap_round_trip -- --ignored --nocapture` |
| `export_pipeline_probe`、`cursor_bench` | 导出流水线和游标（#257 的消息与导出路径） | 见 [native-cli.md](native-cli.md#测试) |
| `web/src/requests.test.tsx` | WebUI 每一步的请求数和顺序 | `cd web && bun test src/requests.test.tsx` |

### 测量条件

- 改动前后用两个明确提交的 release 构建，同一台机器、同一工具链（`rust-toolchain.toml`）、同一数据和参数。
- 每个读至少 10 次（`--runs`，默认 10），报告中位数、p95、最小值、最大值和次数；只测一次的行（启动、启动后第一次读）标为 `once`。
- **冷与热**：`serve_bench` 每次都新启动服务进程，“启动后第一次读”和“启动后打开第一个会话”是进程冷启动的数字；其余是预热后的稳态。操作系统的页缓存没有清空，所以“冷”只是新进程，不是冷磁盘。
- **资源**：服务进程的 CPU 时间和峰值内存（Windows 为峰值工作集，Linux 为 `VmHWM`；macOS 不报告）。命令路由起的子进程不计在内。Windows 上的峰值工作集包含 mimalloc 预留区里实际用到的页。
- **噪声**：`--compare` 只在中位数慢了 2 倍以上（`--tolerance`）**且**多出 20 ms 以上时判为退化；小于这个幅度的变化不下结论。CI 不跑性能测量，也不设性能阈值。
- 真实账号的结果只发汇总数字，不发会话 id、名字、消息或路径。

## 基线结果

基线提交 `a85f867`（#407 合并后的 `main`；本 PR 只加测试和测量，不改服务端代码）。Windows 11 专业版（build 26300），Intel Core i7-10700F（16 个逻辑 CPU），64 GB 内存，NVMe SSD，Rust 1.99.1，release 构建（thin LTO、`codegen-units = 1`、mimalloc），`serve` 默认参数（`--max-tasks 1`，`--max-reads` 与 `--max-media` 为 16）。`serve_bench` 的完整结果在 [`docs/baselines/serve-bench-a85f867-windows.json`](../baselines/serve-bench-a85f867-windows.json)，可以直接作为 `--compare` 的输入。

### 正确性

- 合成账号：`serve_bench` 的 22 项检查全部通过；新消息 1,440 ms 后推送到监听者。
- 回归矩阵：Windows 和 Linux（WSL）上全部通过；WebUI 228 个测试，按顺序和打乱顺序（`--randomize`）都通过。

### 延迟（真实账号，毫秒，客户端视角）

真实账号最大的会话有 234,513 条消息。表中为中位数，括号里是最小值–最大值，每项 n = 10。

| 读 | 子进程路由 | `/api/` |
|---|---:|---:|
| 会话列表，50 个 | 48.5（47.9–123.1） | 10.7（10.5–14.1） |
| 消息，第一页 | 38.8（38.1–46.5） | 3.8（3.6–4.3） |
| 消息，中间一页，用 `offset` | 2,390（2,378–2,491） | 2,230（2,127–2,401） |
| 消息，中间一页，用游标 | 39.3（38.1–40.4） | 3.0（3.0–3.4） |
| 最新 20 条（轮询） | 39.2（38.6–118.0） | 3.2（2.9–3.5） |
| 一个联系人 | 31.8（31.2–38.2） | 1.3（1.3–3.1） |
| 全部联系人 | 41.3（40.7–59.5） | 7.6（7.0–18.4） |
| 会话内搜索 | 44.1（43.0–46.3） | 4.2（4.1–4.6） |
| 会话内搜索，找不到的词 | 177.4（168.2–245.7） | 66.7（66.2–139.5） |
| 群成员 | 41.9（39.9–44.0） | 1.5（1.4–10.9） |
| 每天的消息数 | 657.4（654.5–680.8） | 68.7（67.7–71.1） |
| 朋友圈，20 条 | 245.3（241.3–264.9） | 6.4（6.1–213.1） |
| 16 页消息同时请求（全部完成） | 177.6（163.8–188.7） | 99.1（94.9–105.2） |
| 图片，每张第一次（n = 20） | 53.5（52.7–117.8） | 6.5（4.3–31.3） |
| 20 张图片同时请求 | 52.9，200 个里只成功 10 个（其余 429） | 12.5，200 个全部成功 |

WebUI 仍走子进程的读（只有子进程路由）：会话状态（60 个）30.7、标签页计数 30.4、群列表 35.0、`analytics overall`（已缓存）29.6、`analytics rankings` 34.8、`analytics time` 30.0。它们的耗时几乎全是起进程和打开数据库的固定开销，正是 [serve-architecture.md](serve-architecture.md#附录clap-往返的耗时) 说的、迁到进程内收益最大的部分。

**冷启动**（各测一次）：服务启动到开始监听 36.6 ms；启动后第一次读会话列表 391.9 ms（要先派生数据库密钥）；接着打开第一个会话 6.9 ms。

### 资源

- 服务进程在整段真实账号测量中（上表和完整结果里的所有读与并发请求）用了 46.8 s CPU，峰值工作集 1,170 MiB；子进程不计在内。
- 有人监听推送时，推送引擎每秒用 7.8 ms CPU；没人监听时为 0。

### clap 往返

`clap_round_trip`：每个 `/api/` 读请求拼命令行加解析约 96 µs，其中克隆命令树约 60 µs（每轮 2,000 次取平均，3 轮结果一致）。

### WebUI 请求数

`web/src/requests.test.tsx` 固定了聊天页每一步的请求：

| 步骤 | 请求 |
|---|---|
| 启动 | 1 个事件流（`/api/push/messages`）+ 5 个：`/jobs`（检查令牌）、`/config/profiles`、会话列表、会话状态、第一个会话的消息 |
| 打开另一个会话 | 1 个：它的消息 |
| 回到打开过的会话 | 0 个（已缓存） |
| 会话内搜索 | 1 个 |
| 换账号（`/health/events` 的 `data` 变了） | 6 个：先用**旧账号**打开着的会话向新账号请求一次消息（多余，交给 #402），然后 `/jobs`、`/config/profiles`、会话列表、会话状态、新账号第一个会话的消息 |

换账号前发出、换账号后才返回的旧账号回复不会进入页面（`api/scope.ts` 的作用域在起作用）。

## 对照表模板

实施 PR 复制下表，填入同条件下的数字：

```markdown
| 场景 | 路由 | 基线 中位数（最小–最大，n） | 改动后 中位数（最小–最大，n） | 比值 | 说明 |
|---|---|---:|---:|---:|---|
| sessions, 50 | api | | | | |
| messages, first page | api | | | | |
| … | | | | | |
| 服务进程 CPU / 峰值内存 | - | | | | |
| WebUI 请求数（启动 / 换会话 / 换账号） | - | 6 / 1 / 6 | | | |
```

**验收口径**：

- 回归矩阵全部通过；有意的接口或请求变化在 PR 里说明，并更新了当前消费者和测试预期。
- 没有 `--compare` 判定的退化；判定为退化的项必须解释原因（例如换来了正确性），否则不合并。
- CPU 和峰值内存与基线相比有明显增加时，要说明原因。
- 结构性改动（例如去掉 clap 往返）不能拿来声称性能收益，除非数字超出噪声范围。
