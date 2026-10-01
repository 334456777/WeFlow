# WeFlow 原生命令行

[English](../../README.md) | **简体中文**

`weflow` 是 [WeFlow](weflow-readme.md) 后端的 Rust 原生命令行版本。无需 Electron 桌面端，直接在终端读取、分析和导出本地的微信 4.0 及以上版本聊天记录。

- 默认输出便于阅读的文本（对齐的 `键: 值` 和表格；错误写到 stderr）。加 `--json`（紧凑）或 `--pretty`（缩进）则在 stdout 输出一个 JSON 文档（`{"success": true, "data": ...}`），方便脚本处理。
- 会话、消息、联系人、朋友圈，私聊/群聊统计分析，年度报告与双人报告。
- 消息导出支持 9 种格式：`txt`、`json`、`arkme-json`、`chatlab`、`chatlab-jsonl`、`excel`、`weclone`、`html`、`sql`。
- 图片（`.dat` 解密）、语音（SILK → WAV）、视频查找、表情。
- 本地 HTTP API（token 鉴权、SSE 推送，`serve --http`）、消息推送、AI 见解。
- `--help`、参数错误、运行错误和生成的文本默认跟随系统语言(中文或英文);`--lang en|zh` 可对单次运行覆盖。

> [!WARNING]
> CLI 是从原 TypeScript 后端移植而来。数据库层是纯 Rust(自己解密并以只读方式读取微信数据库),已用一个真实的 Windows 微信 4.x 账号验证过(Linux 构建,以及在 Windows 上运行的 `weflow.exe`);macOS/Linux 的微信数据还没有测试过([尚待验证](cli-unsupported.md#尚待验证))。可能有粗糙之处,欢迎反馈。已覆盖和未覆盖的内容:[覆盖率](cli-coverage.md) · [不支持的功能](cli-unsupported.md)。

原 WeFlow 项目（Electron 桌面端）的说明见 [docs/zh-CN/weflow-readme.md](weflow-readme.md)。

## 构建

```bash
make build                                  # 或：cargo build --release -p weflow-cli
weflow --help
```

Windows x64 发布版是单个 `weflow.exe`。WXGF 图片需要 `PATH`（或 `FFMPEG_PATH`）中有 `ffmpeg`。

## 切换中英文

语言跟随系统:中文系统输出中文,否则输出英文。单独运行 `weflow --lang zh`(不带命令)会把选择保存到配置文件(`weflow config unset lang` 恢复为跟随系统);`weflow --lang zh <命令>` 只对单次运行生效,也可以用环境变量:

```powershell
.\weflow.exe --lang zh chat sessions       # 本次输出中文
$env:WEFLOW_LANG = "zh"                     # 整个 PowerShell 会话都用中文
```

```bash
LANG=zh_CN.UTF-8 weflow export messages <session-id> --out chat.txt
```

单独运行 `weflow.exe --lang zh` 会把语言保存到配置文件;与命令一起使用时只对本次运行生效。语言会影响 `--help`、参数错误、运行时错误和生成的文本;JSON 键和错误码仍为英文,HTTP API 的错误响应也保持英文。完整的优先级见 [docs/zh-CN/native-cli.md](native-cli.md#语言)。

## 首次设置与导出（必要步骤）

以 Windows PowerShell 为例，先登录并保持微信运行（微信 4.0 及以上）：

```powershell
# 1. 找到微信数据目录
.\weflow.exe --lang zh db detect

# 2. 获取数据库密钥和图片密钥（请以管理员身份运行 PowerShell）
#    `key db` 会等待（默认 180 秒）：请按提示完全退出微信、重新打开，并在登录窗口点击「进入微信」，
#    因为密钥只会在微信打开数据库时出现。
.\weflow.exe --lang zh key db
.\weflow.exe --lang zh key image

# 3. 写入配置（只需一次）
.\weflow.exe config set db_path "C:\Users\<你>\Documents\xwechat_files"
.\weflow.exe config set wxid wxid_xxxxxxxx
.\weflow.exe config set decrypt_key <数据库密钥>
.\weflow.exe config set image_xor_key <图片xor密钥>
.\weflow.exe config set image_aes_key <图片aes密钥>

# 4. 必要步骤：查看会话列表，确认连接成功并找到要导出的会话 ID
.\weflow.exe --lang zh chat sessions

# 5. 导出（私聊为对方 wxid，群聊为 xxx@chatroom）
.\weflow.exe --lang zh export messages <会话ID> --format html --out chat.html
```

Windows 说明：`key db` 需要管理员终端（否则会提示权限不足），会自动查找 `Weixin.exe` / `WeChat.exe`（也可用 `--pid` 指定），`--timeout <秒>` 可调整等待时间。配置文件在 `%APPDATA%\weflow\config.json`，解压出的运行时在 `%APPDATA%\weflow\runtime\<版本>\<target>`。WXGF 图片需要 `PATH`（或 `FFMPEG_PATH`）中有 `ffmpeg`，导出图片前请先准备好。

常用导出选项：`--start 2025-01-01 --end 2025-12-31`（本机本地时间，含首尾）、`--display-name remark|nickname|group-nickname`、`--sender wxid_xxx`、`--excel-compact`。加 `--media all`（或 `image,voice,video,emoji`）会把媒体复制到导出文件旁边并在消息里链接到它们；`weflow export media --help` 可单独导出媒体。

## CLI 不支持的功能

数据库层是纯 Rust 且**只读**:不会往微信的文件里写任何东西。因此会修改微信数据库的命令(`chat update-message`、`chat delete-message`、
`chat anti-revoke`、`chat mark-read`、`sns block-delete`、`sns delete`)被有意拒绝,另有一些桌面端功能缺失
(语音转文字、弹窗等桌面进程功能)。

详细清单见 **[docs/zh-CN/cli-unsupported.md](cli-unsupported.md)**([English](../cli-unsupported.md))。

## 文档

- [命令列表](native-cli.md)
- [CLI 不支持的功能(详细清单)](cli-unsupported.md) · [对原后端的覆盖率](cli-coverage.md)
- [桌面端改用 Rust 数据库层](desktop-rust-layer.md)
- [`wcdb_api.dll` 说明:CLI 和新桌面端不依赖它;新旧版本](wcdb-api.md)
- [HTTP API](HTTP-API.md) · [macOS 密钥排障](MAC-KEY-FAQ.md)
- [原 WeFlow README](weflow-readme.md) · [Español](../es-ES/weflow-readme.md)

请负责任地使用本工具，遵守相关法律法规。
