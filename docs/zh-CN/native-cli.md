# 原生命令行 (`weflow`)

[English](../native-cli.md) | **简体中文**

WeFlow 后端的 Rust 命令行版本。每条命令在 stdout 输出一个 JSON 文档（`{"success": true, "data": ...}` 或 `{"success": false, "error": {...}}`）；加 `--progress` 时进度输出到 stderr。

## 语言

语言跟随系统(中文系统输出中文,否则输出英文)。优先级依次为:`--lang`、`WEFLOW_LANG`、配置文件中保存的语言(单独运行 `weflow --lang zh` 会保存;`weflow config unset lang` 可删除)、环境变量（第一个已设置且非空的变量决定结果）、操作系统显示语言：

`WEFLOW_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `LANGUAGE`

以 `zh` 开头的取值（`zh_CN.UTF-8`、`zh-TW`、`zh`）输出中文；其他取值（包括 `C` 和 `POSIX`）输出英文。这些变量都没设置时（Windows 上很常见）由操作系统显示语言决定（Windows、macOS）；无法判断时输出英文。`--lang en|zh <命令>` 可对单次运行覆盖以上所有。

语言会影响 `--help`、参数错误、运行时错误、进度文字和生成的文本：TXT/Excel 导出标签（`[Image]` / `[图片]`）、公众号支付的默认商户名称，以及默认的 AI 见解提示词。JSON 键、错误码和 HTTP API 的错误响应仍为英文。

## 命令

```
weflow config    list | get | set | unset | clear | import
weflow db        detect | scan <root> | test | open
weflow key       db | image | scan-image <user-dir>
weflow chat      sessions | messages | latest | search | contacts | contact | update-message | delete-message
                 anti-revoke | message | dates | date-counts | counts | statuses | detail | mark-read | tab-counts
                 export-stats | group-hint | resources | images | voice-messages | media-stream | transfer-names
                 voice | voice-data | voice-cache | voice-preload | image-data | emoji | clear-account-data
weflow export    sessions | contacts | footprint | media | messages   （messages 支持：chatlab、chatlab-jsonl、json、
                 arkme-json、html、txt、excel、weclone、sql）
weflow analytics overall | rankings | time | excluded | exclude-candidates | clear-cache
weflow group     list | members | ranking | hours | media | member | member-messages | export-member-messages | export-members
weflow report    annual years|generate | dual generate
weflow sns       timeline | users | stats | post-counts | export | media | download-emoji | download-image | debug-resource
                 block-delete | delete
weflow biz       accounts | messages | pay-records
weflow insight   test | trigger | records | get | mark-read | clear | today-stats | scan | footprint | footprint-summary
weflow video     info | parse-md5
weflow image     decrypt | resolve-cache | resolve-batch | clear-cache | auto-download start|status
weflow backup    create | inspect | restore
weflow serve     --http --message-push --insight --image-auto-download
weflow runtime   info | manifest
weflow cache     clear-all
```

数据库层是原生 Rust 且**只读**:`chat update-message`、`chat delete-message`、`chat anti-revoke`、`chat mark-read`、`sns block-delete` 和 `sns delete` 会修改微信数据库,因此一律被拒绝(这些以及其他所有不支持的功能见 [cli-unsupported.md](cli-unsupported.md))。数据库无法打开(密钥错误、文件不可读)时同样返回退出码 `4`。

进度：运行超过延迟时间（默认 5 秒；可用 `--progress-delay <秒>`、环境变量 `WEFLOW_PROGRESS_DELAY` 或 `weflow config set progress_delay_seconds <秒>` 设置，`0` 表示立即显示）的命令会在 stderr 显示单行进度条（仅当 stderr 是终端时；stdout 始终只有 JSON）。`--no-progress` 关闭，`--progress` 改为输出机器可读的 NDJSON 事件。

`export media --type image|voice|video|emoji|all [--session <id>] [--start YYYY-MM-DD --end YYYY-MM-DD]` 按媒体消息遍历（同一张图发两次算两条，所以 `found` 可能大于 `chat images` 列出的唯一文件数）。`missing` 统计文件不在磁盘上（微信里没下载过）或无法解析的消息，按类型分列在 `missingByKind`。`thumbOnly` 统计只导出了缩略图的图片（每条图片记录也带 `isThumb`）；在微信里点开原图后再导出即可得到高清图。`export media` 始终优先使用高清原图（等同 `image decrypt --force`）。表情可能需要联网；语音导出要逐条解码，几百条语音的全量导出需要数分钟。

`export messages` 只从数据库读取所选日期范围（耗时与范围大小相关，与日期早晚无关），并显示进度条；`--start/--end` 是本机本地时区的日期。`--media image,voice,video,emoji`（或 `all`）把媒体复制到输出文件旁边的 `media/<输出文件名>/`，并让消息指向这些副本（适用于 `json`、`arkme-json`、`txt`、`excel`、`weclone`、`html`；`chatlab` 仅图片；`sql` 不支持）。消息边构建边写出，20 万条消息的群约需 0.1 GB（`txt`）到 0.6 GB（`json`、`html`）内存。`--sender` 在所有格式中都只保留该人的消息；普通 `txt` 默认依次用群昵称、备注、昵称、微信号称呼发送者，可用 `--display-name` 改变。

`chat clear-account-data --cache [--exports-dir <目录>] --yes` 删除 WeFlow 为当前账号保存的缓存（图片、语音、表情、朋友圈、统计），并把该账号从配置档案中移除（删除 `db_path`、`wxid`、`decrypt_key` 和图片密钥）；`--exports-dir` 还会删除该目录下以账号命名的条目。`cache clear-all` 清除所有缓存。两者都不会动微信自己的文件。

`key db`（Windows）通过 `wx_key.dll` 挂钩微信并持续轮询（`--timeout`，默认 180 秒），因为微信只在打开数据库时才会产生密钥：命令等待期间请登录或重启微信。需要管理员终端，依次查找 `Weixin.exe`、`WeChat.exe`，也可用 `--pid` 指定。`key image` 从 `kvcomm` 缓存推导图片密钥，并用账号目录下的 `_t.dat` 模板校验。

路径：配置 `%APPDATA%\weflow\config.json`，解压出的运行时 `%APPDATA%\weflow\runtime\<版本>\<target>`（Linux/macOS 位于各平台的数据目录）。

`serve --http` 提供桌面端的 HTTP API（除 `/health` 外都需要 token；设置 `http_api_token` 或使用 `--api-token`）。
`serve --insight` 运行 AI 见解引擎，每条生成的见解以 JSON 行输出到 stderr。
`image auto-download` 与 `serve --image-auto-download` 通过 `img_helper.dll` 钩住微信，仅支持 Windows x64。
语音消息使用内置的 Skype SILK SDK 副本（`crates/weflow-silk`）解码；WXGF 图片需要 `PATH`（或 `FFMPEG_PATH`）中有 `ffmpeg`。

退出码：`0` 成功，`1` 运行时错误，`2` 参数错误，`3` 配置/密钥错误，`4` 数据库/原生库错误。
