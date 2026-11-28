---
name: weflow-cli-command
description: 新增、修改或删除 weflow 的 CLI 子命令和参数时要一起改的地方：commands/ 下的命令文件与 cli.rs、serve 的方法表、进程内读取、中文帮助和报错的中文、契约、PGO 训练清单、Wiki 的命令页面，以及检查这些的测试和容易漏的地方。加命令或参数、改参数名或默认值、改 --help 的文字、让命令进入 /api/ 进程内路由时使用。
---

# 改 CLI 命令

清单的出处是 Wiki [开发指南#常见改动的检查清单](https://github.com/334456777/WeFlow/wiki/开发指南#常见改动的检查清单)，各模块的分工见 [CLAUDE.md](../../../CLAUDE.md) 的「结构」，`serve` 怎样由命令树生成路由见 Wiki [服务端实现#路由由命令树生成](https://github.com/334456777/WeFlow/wiki/服务端实现#路由由命令树生成)。下面按要改的文件排列。

## 命令本身

- 命令组的 clap 定义（`XxxCommand`、`XxxSubcommand`）和执行它的 `handle_xxx` 在 `crates/weflow-cli/src/commands/<命令组>.rs`。新的命令组：加一个文件，在 [`commands.rs`](../../../crates/weflow-cli/src/commands.rs) 里声明，加进 `cli.rs` 的 `Commands` 和 `main.rs` 里 `run` 的分派。业务逻辑写进 `weflow-core` 的 `services/` 中对应领域的 `impl ServiceHub`，命令文件只把参数转成调用。
- `--json` 的回答用 `weflow_core::contract` 的类型构造，新增或修改这些类型后的步骤见 [weflow-contract](../weflow-contract/SKILL.md)。
- clap 参数上的 `///` 就是英文的 `--help`，同时是 [`help_zh.rs`](../../../crates/weflow-cli/src/help_zh.rs) 里中文帮助的键：改了英文就同步改那一项的键和中文。文字里的 `<名字>`、`[::1]` 被 rustdoc 当成 HTML 标签或链接时，在那个字段上加 `#[expect(rustdoc::invalid_html_tags, reason = "…")]`，不改文字（例子见 `commands/export.rs`）。
- 运行时报错的中文在 `weflow-core/src/messages_zh.rs` 的 `CATALOG`，`{}` 匹配任意文字；没有对应项的报错显示英文。新的报错要在终端里显示中文时加一项。
- 不能为负的参数（`--limit`、`--offset`、`--cursor`，以及 Unix 秒的 `--start`、`--end`、`--since`）用 [`numbers.rs`](../../../crates/weflow-cli/src/numbers.rs) 的 `non_negative_*` 作 `value_parser`，并写 `allow_negative_numbers = true`：`--limit -1` 才走到取值校验、报 `-1 is not in 0..=…`，而不是 clap 的 `unexpected argument`。进程内路由的 `Given` 读这些参数用 `non_negative`、`non_negative_or`，否则 `typed_reads_take_what_clap_takes_and_read_it_the_same` 失败。每个 `--limit` 的帮助写明 0 的含义。
- 有上下限的计数（`group member-messages` 的 `--limit`，1 到 100）：范围常量写在 `weflow-core`（如 `MEMBER_MESSAGES_LIMIT`），服务层用它返回用法错误，命令行用 `numbers.rs` 的 `usize_in(范围)`、进程内路由的 `Given` 用 `within_or`，三处同一个范围；范围之外（含 0 和负数）都报用法错误。
- 几个可选参数至少要给一个（`image decrypt`、`image resolve-cache` 的 `--md5`、`--dat-name`）：在参数结构上加 `#[command(group(ArgGroup::new(…).args([…]).required(true).multiple(true)))]`，缺少时 clap 报用法错误并列出这组参数；不写 `multiple(true)` 时组里的参数互相冲突。`/openapi.json` 表达不了「至少一个」，在这些参数的帮助里写明。给了空值也算没给的，由服务层的兜底检查返回用法错误（`AppError::usage`）。

## serve

- 每个命令在 [`serve_api/rules.rs`](../../../crates/weflow-cli/src/serve_api/rules.rs) 的 `METHODS` 里有一个 HTTP 方法：读用 `GET`，运行任务或写文件用 `POST`，改设置用 `PUT`，删除用 `DELETE`。方法随参数变化时用 `Rule::Arg` 或 `Rule::Value`。
- 下面三张表由人登记，漏登记时命令照样能运行，只是在 serve 下行为不对：
  - 只写出一个文件的命令加进 [`serve_api/files.rs`](../../../crates/weflow-cli/src/serve_api/files.rs) 的 `FILE_COMMANDS`，`?return=file` 才能把文件带回：写明请求不给 `--out` 时用的名字（空字符串表示命令自己在文件夹里起名）和结果里存放写出路径的字段（`None` 表示就是 `--out`）。命令要有 `out` 参数，`file_commands_write_to_out` 检查；某些参数组合会写出文件夹时（`export messages` 的多个格式），在同一文件的 `not_one_file` 里提前拒绝。
  - 在微信里下钩子的命令（`key db`、`image auto-download start`）被直接终止会把钩子留在微信里：加进 [`serve_api/child.rs`](../../../crates/weflow-cli/src/serve_api/child.rs) 的 `ends_by_itself`，serve 不终止它，而是关闭它的 stdin。命令在设了 `CANCEL_ON_STDIN` 时把 stdin 结束当作停止请求，先撤钩子再退出：一直运行的命令用 `stop::requested`，`key db` 用 `commands/key.rs` 的 `interrupt_when_stdin_ends`。见 Wiki [服务端实现#子进程路由](https://github.com/334456777/WeFlow/wiki/服务端实现#子进程路由)。
  - 回答服务进程自己持有的状态的命令（运行中的钩子、见解引擎的计数）在子进程里只能读到子进程自己的空状态：加进 `rules.rs` 的 `OWN_STATE`，并在 `serve_api.rs` 的 `own_state` 里回答，`own_state_answers_the_commands_declared`、`own_state_names_real_commands` 检查两边一致。见 Wiki [服务端实现#服务进程自己回答的命令](https://github.com/334456777/WeFlow/wiki/服务端实现#服务进程自己回答的命令)。
- 会写微信数据库的操作不做成命令：数据库层只读，见 Wiki [不支持的功能](https://github.com/334456777/WeFlow/wiki/不支持的功能#不提供的写操作)。
- 网页经常读的命令可以进入 `/api/` 进程内路由，判断标准见 Wiki [服务端实现#哪些读取进入进程内](https://github.com/334456777/WeFlow/wiki/服务端实现#哪些读取进入进程内)。要进入时改四处：
  1. `weflow-core/src/reads.rs` 的 `Read` 加一个成员，并实现它的读取；
  2. `weflow-cli/src/reads.rs` 的 `read_of` 把命令映射到这个成员；
  3. `serve_api/in_process.rs` 的 `COMMANDS` 加上命令名；
  4. 同一文件 `read` 里的 `match` 把查询参数读成这个成员。
- 服务测试在 `crates/weflow-cli/tests/serve_api/`：子进程路由放 `commands.rs`，进程内路由放 `in_process.rs`。GET 路由不读请求体，参数写进查询串。

## PGO 训练清单

- 常用的 CLI 命令加进 `.github/scripts/pgo-build.sh` 里调用 `run` 的那一串；会联网的命令（表情、朋友圈媒体和 HTML 的头像、AI）不加。删改这些命令的参数时同步修改。
- `/api/` 的读取路由由 `pgo-serve.py` 从 `/openapi.json` 自动训练，但它只会填 `session-id`、`chatroom-id`、`keyword`、`username` 四种必填参数和会话列表 `sessions`。新路由的必填参数叫别的名字时，这条路由不会被调用，训练结束时发 `was not called` 警告；要训练它，就在 `pgo-serve.py` 里补上这个参数的取值。
- 不带会话就回答空对象的读取（如 `chat counts`），CLI 的训练命令也要写上会话。
- 改完在本地运行一次，确认没有警告，见 [weflow-release](../weflow-release/SKILL.md#pgo-训练清单)。

## 删改已有的参数

- 删改参数、改默认值、改回答的字段会让脚本和已发布的网页出错：要不要提升 `API_VERSION` 见 CLAUDE.md 的「版本与 tag」，版本号升哪一位见 [weflow-release](../weflow-release/SKILL.md#先问维护者)。
- 在 `web/src/api/` 和 `web/src/mock/` 里搜参数名：网页以查询串传参数，名字是 kebab-case（如 `session-id`）。示例服务端也要按新的参数回答，见 [weflow-web-feature](../weflow-web-feature/SKILL.md#示例数据)。
- README 的「首次设置与导出」用到了这个命令时，两份 README 一起改，见 [weflow-wiki](../weflow-wiki/SKILL.md#readme)。

## Wiki

- 新命令登记到 [命令参考#命令总表](https://github.com/334456777/WeFlow/wiki/命令参考#命令总表) 所指的命令组页面（如[聊天与媒体命令](https://github.com/334456777/WeFlow/wiki/聊天与媒体命令)）。
- 进程内路由写进 [命令-API#进程内路由](https://github.com/334456777/WeFlow/wiki/命令-API#进程内路由)；新的报错和处理办法写进[故障排查](https://github.com/334456777/WeFlow/wiki/故障排查)。
- Wiki 先于 PR 合并推送，写法见 [weflow-wiki](../weflow-wiki/SKILL.md)。

## 测试

`cargo test -p weflow-cli --bin weflow` 一次跑完下面这些检查。只跑其中几个时，容易漏掉别处的失败。

| 测试 | 检查什么 |
|---|---|
| `every_command_has_a_method` | 每个命令在 `METHODS` 里都有方法 |
| `file_commands_write_to_out`、`own_state_names_real_commands`、`own_state_answers_the_commands_declared` | `FILE_COMMANDS` 的命令都有 `out` 参数；`OWN_STATE` 的命令都存在，且 `own_state` 都回答了 |
| `every_command_and_argument_has_help` | 每个命令和参数都有 `///` |
| `all_help_text_is_translated`、`every_translation_is_used` | `help_zh.rs` 没有缺项，也没有用不上的项 |
| `the_list_matches_the_cli`、`typed_reads_take_what_clap_takes_and_read_it_the_same` | 进程内路由与 `read_of` 一致，参数读法与 clap 相同 |
| `every_answer_is_described`、`contract_snapshot_is_current` | 回答在契约里有描述，快照是最新的 |

命令背后的业务逻辑在 `weflow-core` 里写单元测试或端到端测试，见 [weflow-tests](../weflow-tests/SKILL.md)。
