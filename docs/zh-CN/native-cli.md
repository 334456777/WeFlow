# 原生命令行 (`weflow`)

[English](../native-cli.md) | **简体中文**

WeFlow 后端的 Rust 命令行版本。每条命令在 stdout 输出一个 JSON 文档（`{"success": true, "data": ...}` 或 `{"success": false, "error": {...}}`）；加 `--progress` 时进度输出到 stderr。

## 语言

默认输出英文。只有环境变量要求时才输出中文，优先级如下（第一个已设置且非空的变量决定结果）：

`WEFLOW_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `LANGUAGE`

以 `zh` 开头的取值（`zh_CN.UTF-8`、`zh-TW`、`zh`）输出中文；其他取值（包括 `C` 和 `POSIX`）输出英文。`--lang en|zh` 可对单次运行覆盖环境变量。

语言只影响生成的文本：TXT/Excel 导出标签（`[Image]` / `[图片]`）、公众号支付的默认商户名称，以及默认的 AI 见解提示词。JSON 键、错误码和 `--help` 文本始终为英文。

## 命令

```
weflow config    list | get | set | unset | clear | import
weflow db        detect | scan <root> | test | open
weflow key       db | image | scan-image <user-dir>
weflow chat      sessions | messages | latest | search | contacts | contact | update-message | delete-message
                 anti-revoke | message | dates | date-counts | counts | statuses | detail | mark-read | tab-counts
                 export-stats | group-hint | resources | images | voice-messages | media-stream | transfer-names
                 voice | voice-data | voice-cache | voice-preload | image-data | emoji
weflow export    sessions | contacts | footprint | media | messages   （messages 支持：chatlab、chatlab-jsonl、json、
                 arkme-json、html、txt、excel、weclone、sql）
weflow analytics overall | rankings | time | excluded | exclude-candidates | clear-cache
weflow group     list | members | ranking | hours | media | member | member-messages | export-member-messages | export-members
weflow report    annual years|generate | dual generate
weflow sns       timeline | users | stats | post-counts | export | media | download-emoji | download-image | block-delete | delete
weflow biz       accounts | messages | pay-records
weflow insight   test | trigger | records | get | mark-read | clear | today-stats | scan | footprint | footprint-summary
weflow video     info | parse-md5
weflow image     decrypt | resolve-cache | resolve-batch | clear-cache | auto-download start|status
weflow backup    create | inspect | restore
weflow serve     --http --message-push --insight --image-auto-download
weflow runtime   info | manifest
```

进度：运行超过 10 秒的命令会在 stderr 显示单行进度条（仅当 stderr 是终端时；stdout 始终只有 JSON）。`--no-progress` 关闭，`--progress` 改为输出机器可读的 NDJSON 事件。

`export media --type image|voice|video|emoji|all [--session <id>] [--start YYYY-MM-DD --end YYYY-MM-DD]` 按媒体消息遍历（同一张图发两次算两条，所以 `found` 可能大于 `chat images` 列出的唯一文件数）。`missing` 统计文件不在磁盘上（微信里没下载过）或无法解析的消息，按类型分列在 `missingByKind`。表情可能需要联网；语音导出要逐条解码，几百条语音的全量导出需要数分钟。

`key db`（Windows）通过 `wx_key.dll` 挂钩微信并持续轮询（`--timeout`，默认 180 秒），因为微信只在打开数据库时才会产生密钥：命令等待期间请登录或重启微信。需要管理员终端，依次查找 `Weixin.exe`、`WeChat.exe`，也可用 `--pid` 指定。`key image` 从 `kvcomm` 缓存推导图片密钥，并用账号目录下的 `_t.dat` 模板校验。

路径：配置 `%APPDATA%\weflow\config.json`，解压出的运行时 `%APPDATA%\weflow\runtime\<版本>\<target>`（Linux/macOS 位于各平台的数据目录）。

`serve --http` 提供桌面端的 HTTP API（除 `/health` 外都需要 token；设置 `http_api_token` 或使用 `--api-token`）。
`serve --insight` 运行 AI 见解引擎，每条生成的见解以 JSON 行输出到 stderr。
`image auto-download` 与 `serve --image-auto-download` 通过 `img_helper.dll` 钩住微信，仅支持 Windows x64。
语音消息使用内置的 Skype SILK SDK 副本（`crates/weflow-silk`）解码；WXGF 图片需要 `PATH`（或 `FFMPEG_PATH`）中有 `ffmpeg`。

退出码：`0` 成功，`1` 运行时错误，`2` 参数错误，`3` 配置/密钥错误，`4` 数据库/原生库错误。
