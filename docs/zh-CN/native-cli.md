# 原生命令行 (`weflow`)

[English](../native-cli.md) | **简体中文**

WeFlow 后端的 Rust 命令行版本。默认在 stdout 输出便于阅读的文本（对齐的 `键: 值`，列表用表格），错误写到 stderr，退出码不变。加 `--json` 时，每条命令在 stdout 输出一个 JSON 文档（`{"success": true, "data": ...}` 或 `{"success": false, "error": {...}}`）；加 `--progress` 时进度输出到 stderr。

帮助：`-h` / `--help` 显示任意命令的帮助；需要参数或子命令的命令在完全不带参数时也会显示帮助（`./weflow config`、`./weflow config set`、`./weflow lang`）；只缺一部分参数时会提示缺少哪些。用法行里 `[选项]` 放在最后（`./weflow config set <KEY> <VALUE> [选项]`）。

## 语言

语言跟随系统(中文系统输出中文，否则输出英文)。优先级依次为:`WEFLOW_LANG`、配置文件中保存的语言(运行 `./weflow lang zh` 会保存;`./weflow config unset lang` 可删除)、环境变量（第一个已设置且非空的变量决定结果）、操作系统显示语言：

`WEFLOW_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `LANGUAGE`

以 `zh` 开头的取值（`zh_CN.UTF-8`、`zh-TW`、`zh`）输出中文；其他取值（包括 `C` 和 `POSIX`）输出英文。这些变量都没设置时（Windows 上很常见）由操作系统显示语言决定（Windows、macOS）；无法判断时输出英文。`--lang en|zh <命令>` 可对单次运行覆盖以上所有。

语言会影响 `--help`、参数错误、运行时错误、进度文字和生成的文本：TXT/Excel 导出标签（`[Image]` / `[图片]`）、公众号支付的默认商户名称，以及默认的 AI 见解提示词。JSON 键、错误码和 HTTP API 的错误响应仍为英文。

## 命令

```
./weflow config    path | list | get | set | unset | clear | import
./weflow lang      en | zh
./weflow db        detect | scan <root> | wxid | test | open
./weflow key       db | image | scan-image <user-dir>
./weflow chat      sessions | messages | latest | search | contacts | contact | update-message | delete-message
                 anti-revoke | message | dates | date-counts | counts | statuses | detail | mark-read | tab-counts
                 export-stats | group-hint | resources | images | voice-messages | media-stream | transfer-names
                 voice | voice-data | voice-cache | voice-preload | image-data | emoji | clear-account-data
./weflow export    sessions | contacts | footprint | media | messages   （messages 支持：chatlab、chatlab-jsonl、json、
                 arkme-json、html、txt、excel、weclone、sql）
./weflow analytics overall | rankings | time | excluded | exclude-candidates | clear-cache
./weflow group     list | members | ranking | hours | media | member | member-messages | export-member-messages | export-members
./weflow report    annual years|generate | dual generate
./weflow sns       timeline | users | stats | post-counts | export | media | download-emoji | download-image | debug-resource
                 block-delete | delete
./weflow biz       accounts | messages | pay-records
./weflow insight   test | trigger | records | get | mark-read | clear | today-stats | scan | footprint | footprint-summary
./weflow video     info | parse-md5
./weflow image     decrypt | resolve-cache | resolve-batch | clear-cache | auto-download start|status
./weflow backup    create | inspect | restore
./weflow serve     --http --message-push --insight --image-auto-download
./weflow runtime   info | manifest
./weflow cache     clear-all
./weflow ffmpeg    install | path | set baseurl <url> | unset baseurl
```

数据库层是原生 Rust 且**只读**:`chat update-message`、`chat delete-message`、`chat anti-revoke`、`chat mark-read`、`sns block-delete` 和 `sns delete` 会修改微信数据库，因此一律被拒绝(这些以及其他所有不支持的功能见 [cli-unsupported.md](cli-unsupported.md))。数据库无法打开(密钥错误、文件不可读)时同样返回退出码 `4`。

进度：运行超过延迟时间（默认 5 秒；可用 `./weflow config set progress_delay_seconds <秒>` 或环境变量 `WEFLOW_PROGRESS_DELAY` 设置，`0` 表示立即显示）的命令会在 stderr 显示单行进度条（仅当 stderr 是终端时；stdout 不受影响）。`./weflow config set no_progress true` 关闭，`--progress` 改为输出机器可读的 NDJSON 事件。

`export media --type image|voice|video|emoji|all [--session <id>] [--start YYYY-MM-DD --end YYYY-MM-DD]` 按媒体消息遍历（同一张图发两次算两条，所以 `found` 可能大于 `chat images` 列出的唯一文件数）。`missing` 统计文件不在磁盘上（微信里没下载过）或无法解析的消息，按类型分列在 `missingByKind`。`thumbOnly` 统计只导出了缩略图的图片（每条图片记录也带 `isThumb`）；在微信里点开原图后再导出即可得到高清图。`export media` 始终优先使用高清原图（等同 `image decrypt --force`）。表情可能需要联网；语音导出要逐条解码，几百条语音的全量导出需要数分钟。

`export messages` 只从数据库读取所选日期范围（耗时与范围大小相关，与日期早晚无关），并显示进度条；`--start/--end` 是本机本地时区的日期。`--media image,voice,video,emoji`（或 `all`）把媒体复制到输出文件旁边的 `media/<输出文件名>/`，并让消息指向这些副本（适用于 `json`、`arkme-json`、`txt`、`excel`、`weclone`、`html`；`chatlab` 仅图片；`sql` 不支持）。所有格式都边读边写（`json`、`chatlab`、`html`、`excel` 的文件头要用到总数，所以会先把消息写到输出文件旁的临时文件 `<输出文件>.part`；带 `--media` 的导出要先读完整个会话），20 万条消息的群约需 0.1 GB（`excel` 约 0.2 GB）内存。`--sender` 在所有格式中都只保留该人的消息；所有格式默认依次用群昵称、备注、昵称、微信号称呼发送者（`--display-name group-nickname`）；`--display-name remark` 跳过群昵称，`--display-name nickname` 只用昵称。

`export messages` 的消息文本：所有格式对同一条消息写同样的文本，所以各格式之间可以逐条对照。

| 消息 | 文本 |
|---|---|
| 文本 | 正文；群聊里微信在前面加的 `wxid:` 一行会去掉（正文本身像 `4:1` 的会保留） |
| 图片 / 语音 / 视频 | `[图片]` / `[语音消息]` / `[视频]`（`--media` 复制了文件时是文件路径） |
| 表情 | `[表情]`；表情库里有描述时是 `[表情：<描述>]`（优先你自己给表情写的文字，否则商店表情的描述） |
| 系统消息 | `[系统: <内容>]` |
| 链接卡片 | `[链接] <标题>`，下一行是 URL |
| 转账 | 金额加「谁转给谁」：`[转账] (A 转账给 B) ¥66.00` |
| 引用（回复） | `<回复内容>[引用 <名字>：<被引用内容>]`；被引用的是文件、笔记、链接或另一条回复时显示简短标签或标题，不会是 XML |
| 转发的聊天记录 | `[转发的聊天记录]` 加里面的消息 |
| 微信笔记 | `[笔记]` 加完整笔记正文（图片显示为 `[图片]`） |

个别格式保留自己的规则：`chatlab` 的链接写成 `[标题](URL)`，系统消息不加包裹（它已有 `type` 字段）；`html` 自己渲染链接卡片和系统消息；`weclone` 不包含引用消息。`txt` 是一行 `<时间> '<发送者>'`，下一行起是正文，名字按 `--display-name`。`arkme-json` 原样带出微信自己的字段（`source`、`appMsgDesc` 等）。

所有格式都按时间顺序边读边写（如果某个会话读出来的顺序不是时间顺序，会按读取顺序写出）。文件头需要总数的格式，会先把消息写到 `<输出>.part`，读完再拼接；结束后（失败时也一样）删除该文件，范围内没有消息时不写文件。

`chat clear-account-data --cache [--exports-dir <目录>] --yes` 删除 WeFlow 为当前账号保存的缓存（图片、语音、表情、朋友圈、统计），并把该账号从配置档案中移除（删除 `db_path`、`wxid`、`decrypt_key` 和图片密钥）；`--exports-dir` 还会删除该目录下以账号命名的条目。`cache clear-all` 清除所有缓存。两者都不会动微信自己的文件。

`db detect` 以 `db_path: <路径>` 输出存在的微信数据目录，`db wxid` 以 `wxid: <wxid>` 输出检测到的账号的 wxid（文件夹名去掉 `_ab12` 后缀；也可附带数据目录作为可选参数来指定目录）；这两个名字都是 `config set` 要填的名字。

`key db`（Windows）通过 `wx_key.dll` 挂钩微信。微信只在打开数据库时才会产生密钥，所以命令会先请你完全退出微信（如果它正在运行）再重新打开，每秒检查一次微信进程（整个过程超过 `--timeout`，默认 180 秒，就会自动退出，并显示剩余秒数），然后挂钩新启动的微信，并请你在登录窗口点击「进入微信」。结果以 `decrypt_key: <密钥>` 输出，名字与 `config set` 一致。需要管理员终端，依次查找 `Weixin.exe`、`WeChat.exe`；用 `--pid` 指定时直接挂钩该进程，不再等待重启。`key image` 从 `kvcomm` 缓存推导图片密钥，用账号目录下的 `_t.dat` 模板校验，并输出 `image_xor_key` 和 `image_aes_key`。

路径：配置 `%APPDATA%\weflow\config.json`，解压出的运行时 `%APPDATA%\weflow\runtime\<版本>\<target>`（Linux/macOS 位于各平台的数据目录）。

`serve --http` 提供桌面端的 HTTP API（除 `/health` 外都需要 token；设置 `http_api_token` 或使用 `--api-token`）。
`serve --insight` 运行 AI 见解引擎，每条生成的见解输出到 stderr（加 `--json` 时为 JSON 行）。
`image auto-download` 与 `serve --image-auto-download` 通过 `img_helper.dll` 钩住微信，仅支持 Windows x64。
语音消息使用内置的 Skype SILK SDK 副本（`crates/weflow-silk`）解码；WXGF 图片需要 `ffmpeg`：先用 `FFMPEG_PATH`，其次是 `PATH` 中的 `ffmpeg`，最后是 `ffmpeg install` 安装到 WeFlow 文件夹的副本（`ffmpeg path` 显示实际使用哪一个）。`ffmpeg install` 下载桌面端自带的同一版本（npm `ffmpeg-static` 5.3.0，即 eugeneware/ffmpeg-static 的 `b6.1.1` 发布，GPL-3.0，附带其许可证文件），校验 SHA-256 后解压到配置文件旁的 `ffmpeg/b6.1.1/`；只有运行这个命令时才会下载。`ffmpeg set baseurl <url>` 可改从这些发布文件的镜像下载，而不是 GitHub（例如 `https://registry.npmmirror.com/-/binary/ffmpeg-static`；地址保存在配置文件里，`ffmpeg unset baseurl` 恢复为 GitHub），文件同样会被校验。Windows 版本是 x64 构建（Windows on Arm 通过模拟运行）。第一次导出图片前请先用 `ffmpeg path` 检查（见 [README](README.md#检查-ffmpeg)）：找不到 ffmpeg 的 WXGF 图片会以 `failure_kind` `ffmpeg_missing` 失败，导出会把这类图片计入 `ffmpegMissing`。

退出码：`0` 成功，`1` 运行时错误，`2` 参数错误，`3` 配置/密钥错误，`4` 数据库/原生库错误，`130` 用户中断。

## 结构

| Crate | 作用 |
|---|---|
| `crates/weflow-cli` | 命令入口、参数解析、输出 |
| `crates/weflow-core` | 配置、账号、聊天、导出、统计分析、朋友圈、备份、AI 见解、HTTP API |
| `crates/weflow-native` | 原生数据库读取(SQLCipher 解密、消息、联系人、朋友圈、统计、报告)、密钥辅助、图片解密、ISAAC-64 密钥流(移植自厂商 WASM)、平台封装 |
| `crates/weflow-wcdb-ffi` | 把 `weflow-native` 导出为与 `wcdb_api` 接口兼容的 C ABI 动态库 `weflow_wcdb`，供桌面端加载(见 [desktop-rust-layer.md](desktop-rust-layer.md)) |
| `crates/weflow-assets` | 内嵌资源、解压、哈希校验 |
| `crates/weflow-silk` | 内置的 SILK 解码器，用于语音消息 |

无法重写的平台辅助程序(`wx_key.dll`、`img_helper.dll`、`libwx_key.dylib`、`xkey_helper_linux`)内嵌在程序里:
每个二进制只内嵌本平台需要的辅助程序，解压到 `WEFLOW_HOME/runtime/<版本>/<target>/`;每次启动校验清单里的哈希(版本或哈希
不一致时重新解压);动态库只从这个目录加载，不会隐式从当前目录加载。
厂商的 WASM 解码器(`WxIsaac64`)不在其中:它已用纯 Rust 移植在 `weflow-core/src/isaac64.rs`，并用从原模块抓取的测试向量验证过。

配置放在 `WEFLOW_HOME`，否则是平台配置目录下的 `./weflow`:配置文件 `config.json`(也接受 TOML)，缓存、日志、运行时分目录存放。
`./weflow config import` 迁移桌面端可读的设置，加密的 `safe:` / `lock:` 字段会跳过并提示重新设置。

连接相关的设置（`db_path`、`wxid`、`decrypt_key`）只从配置文件读取，用 `./weflow config set` 设置，没有命令行覆盖选项。`./weflow config set --help` 会解释每个键。`./weflow config set config_path <文件>` 让之后的运行改用另一个配置文件（记录在默认位置旁的 `config_path` 文件里，`./weflow config unset config_path` 或传入默认路径即恢复），`./weflow config path` 显示当前使用的路径。`./weflow config set current_profile <名称>` 切换当前配置档案（档案不存在时自动创建）。`-h` / `--help` 和 `-V` / `-v` / `--version` 在所有命令中都可用，只是不显示在选项列表里。

**为什么数据库层是纯 Rust。** 命令行原计划通过 FFI 调用闭源的 `wcdb_api` 库。这个库带有效期检查(2026-09-30 23:59:59 之后
`wcdb_init` 返回 `-1000`)和未经核实的网络代码，所以命令行改为自己解密微信 4.x 数据库(SQLCipher 4)，用纯 Rust **只读**
读取(`crates/weflow-native/src/{sqlcipher,native_*}.rs`)。命令行和桌面端都不再内嵌或加载 `wcdb_api`、`WCDB.dll`、
`libwcdb_api.*`、`libWCDB.dylib`;它们留在仓库里给原版桌面端使用，见 [wcdb-api.md](wcdb-api.md)。对原后端的覆盖率见 [cli-coverage.md](cli-coverage.md)。

## 测试

- 各 crate 的单元测试:配置和旧配置导入、内嵌运行时清单、SQLCipher(加解密往返、错误密钥、页被篡改、WAL 合并)、图片和朋友圈解密、导出格式。
- 端到端测试用合成的加密账号(`weflow_native::fixture`:SQLCipher 页、WAL、zstd、SILK)和本地假 HTTP 服务器，位于
  `crates/weflow-core/tests/`。`cargo test --workspace` 会全部运行。
- 真实数据回归:一个 Windows 微信 4.x 账号，用 Linux 构建，以及在 Windows 上运行 `weflow.exe`(约 80 个命令、带媒体的消息导出、
  图片导出、HTTP API)。还没验证的部分见 [cli-unsupported.md](cli-unsupported.md#4-平台与验证范围)。
