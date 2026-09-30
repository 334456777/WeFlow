# WeFlow 原生命令行

[English](README.md) | **简体中文**

`weflow` 是 [WeFlow](docs/zh-CN/weflow-readme.md) 后端的 Rust 原生命令行版本。无需 Electron 桌面端，直接在终端读取、分析和导出本地的微信 4.0 及以上版本聊天记录。

- 每条命令在 stdout 输出一个 JSON 文档（`{"success": true, "data": ...}`），方便脚本处理。
- 会话、消息、联系人、朋友圈，私聊/群聊统计分析，年度报告与双人报告。
- 消息导出支持 9 种格式：`txt`、`json`、`arkme-json`、`chatlab`、`chatlab-jsonl`、`excel`、`weclone`、`html`、`sql`。
- 图片（`.dat` 解密）、语音（SILK → WAV）、视频查找、表情。
- 本地 HTTP API（token 鉴权、SSE 推送，`serve --http`）、消息推送、AI 见解。
- 默认英文，按需切换中文。

> [!WARNING]
> CLI 是从原 TypeScript 后端移植而来，只在 mock WCDB 库上测试过，**没有用真实微信数据验证**。可能有粗糙之处，欢迎反馈。已覆盖和未覆盖的内容：[覆盖率](docs/zh-CN/cli-coverage.md) · [未覆盖部分](docs/zh-CN/cli-gaps.md)。

原 WeFlow 项目（Electron 桌面端）的说明见 [docs/zh-CN/weflow-readme.md](docs/zh-CN/weflow-readme.md)。

## 构建

```bash
make build                                  # 或：cargo build --release -p weflow-cli
weflow --help
```

Windows x64 发布版是单个 `weflow.exe`。WXGF 图片需要 `PATH`（或 `FFMPEG_PATH`）中有 `ffmpeg`。

## 切换中英文

默认英文。单次运行用 `--lang en|zh`，也可以用环境变量：

```powershell
.\weflow.exe --lang zh chat sessions --pretty    # 本次输出中文
$env:WEFLOW_LANG = "zh"                          # 整个 PowerShell 会话都用中文
```

```bash
LANG=zh_CN.UTF-8 weflow export messages <session-id> --out chat.txt
```

只有当 `WEFLOW_LANG`、`LC_ALL`、`LC_MESSAGES`、`LANG` 或 `LANGUAGE` 以 `zh` 开头时才输出中文（以先设置的变量为准；其他取值，包括 `C`/`POSIX`，均为英文）。`--lang` 是选项而不是命令，必须配合子命令使用，例如 `weflow.exe --lang zh chat sessions --pretty`；单独运行 `weflow.exe --lang zh` 会提示缺少子命令。语言只影响生成的文本（`[Image]` / `[图片]` 之类的导出标签、默认 AI 提示词）；JSON 键、错误码和 `--help` 始终是英文。

## 首次设置与导出（必要步骤）

以 Windows PowerShell 为例，先登录并保持微信运行（微信 4.0 及以上）：

```powershell
# 1. 找到微信数据目录
.\weflow.exe --lang zh db detect --pretty

# 2. 获取数据库密钥和图片密钥（请以管理员身份运行 PowerShell）
#    `key db` 会挂钩微信并等待（默认 180 秒）：等待期间请退出并重新登录微信（或重启微信），
#    密钥只会在微信打开数据库时出现。
.\weflow.exe --lang zh key db --pretty
.\weflow.exe --lang zh key image --pretty

# 3. 写入配置（只需一次）
.\weflow.exe config set db_path "C:\Users\<你>\Documents\xwechat_files"
.\weflow.exe config set wxid wxid_xxxxxxxx
.\weflow.exe config set decrypt_key <数据库密钥>
.\weflow.exe config set image_xor_key <图片xor密钥>
.\weflow.exe config set image_aes_key <图片aes密钥>

# 4. 必要步骤：查看会话列表，确认连接成功并找到要导出的会话 ID
.\weflow.exe --lang zh chat sessions --pretty

# 5. 导出（私聊为对方 wxid，群聊为 xxx@chatroom）
.\weflow.exe --lang zh export messages <会话ID> --format html --out chat.html
```

Windows 说明：`key db` 需要管理员终端（否则会提示权限不足），会自动查找 `Weixin.exe` / `WeChat.exe`（也可用 `--pid` 指定），`--timeout <秒>` 可调整等待时间。配置文件在 `%APPDATA%\weflow\config.json`，解压出的运行时在 `%APPDATA%\weflow\runtime\<版本>\<target>`。WXGF 图片需要 `PATH`（或 `FFMPEG_PATH`）中有 `ffmpeg`，导出图片前请先准备好。

常用导出选项：`--start 2025-01-01 --end 2025-12-31`（北京时间，含首尾）、`--display-name remark|nickname|group-nickname`、`--sender wxid_xxx`、`--excel-compact`。消息导出不内嵌媒体文件；媒体请单独导出，见 `weflow export media --help`。

## 文档

- [命令列表](docs/zh-CN/native-cli.md)
- [对原后端的覆盖率](docs/zh-CN/cli-coverage.md) · [CLI 未覆盖的部分](docs/zh-CN/cli-gaps.md)
- [HTTP API](docs/zh-CN/HTTP-API.md) · [macOS 密钥排障](docs/zh-CN/MAC-KEY-FAQ.md)
- [原 WeFlow README](docs/zh-CN/weflow-readme.md)

请负责任地使用本工具，遵守相关法律法规。
