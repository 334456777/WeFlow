# WeFlow

**English** | [简体中文](README.zh-CN.md)

**WeFlow** is a fully local tool for viewing, analyzing, and exporting WeChat chat history in real time. It generates unique analysis reports based on your chat history.

<p align="center">
  <img src="app.jpg" alt="WeFlow app preview" width="90%">
</p>

<p align="center">
  <a href="https://github.com/hicccc77/WeFlow/stargazers"><img src="https://img.shields.io/github/stars/hicccc77/WeFlow?style=flat&label=Stars&labelColor=1F2937&color=2563EB" alt="Stargazers"></a>
  <a href="https://github.com/hicccc77/WeFlow/network/members"><img src="https://img.shields.io/github/forks/hicccc77/WeFlow?style=flat&label=Forks&labelColor=1F2937&color=7C3AED" alt="Forks"></a>
  <a href="https://github.com/hicccc77/WeFlow/issues"><img src="https://img.shields.io/github/issues/hicccc77/WeFlow?style=flat&label=Issues&labelColor=1F2937&color=D97706" alt="Issues"></a>
  <a href="https://github.com/hicccc77/WeFlow/releases"><img src="https://img.shields.io/github/downloads/hicccc77/WeFlow/total?style=flat&label=Downloads&labelColor=1F2937&color=059669" alt="Downloads"></a>
  <br><br>
  <a href="https://t.me/weflow_cc"><img src="https://img.shields.io/badge/Telegram-Channel-1D9BF0?style=flat&logo=telegram&logoColor=white&labelColor=1F2937&color=1D9BF0" alt="Telegram Channel" style="height: 22px; vertical-align: middle;"></a>
  <a href="https://www.star-history.com/hicccc77/weflow"><img src="https://api.star-history.com/badge?repo=hicccc77/WeFlow&theme=dark" alt="Star History Rank" style="height: 32px; vertical-align: middle;"></a>
</p>

> [!TIP]
> If you want to analyze your exported chat content in depth, try [ChatLab](https://chatlab.fun/)

> [!NOTE]
> Only supports WeChat **version 4.0 and above**. Please ensure your WeChat version meets the requirements.

## Key Features

- View chat history locally in real-time
- Preview and decrypt Moments photos, videos, and **Live Photos**
- Statistical analysis and group chat insights
- Annual reports and visual overviews
- Export chat history to HTML and other formats
- HTTP API (for developer integration)
- View the complete feature list: [Detailed Features](#detailed-feature-list)

## Supported Platforms & Devices

| Platform | Device/Architecture | Package |
|----------|---------------------|---------|
| Windows | Windows 10+, x64 (amd64) | `.exe` |
| macOS | Apple Silicon (M series, arm64) | `.dmg` |
| Linux | x64 devices (amd64) | `.AppImage`, `.tar.gz` |

## Desktop App Language

The desktop app is English by default. It switches to Chinese when the system locale is Chinese (Electron follows `LANG` / `LC_*` on Linux), or you can pick **English**, **简体中文** or **System** under *Settings → Appearance → Language*. Changing it reloads the window.

Developers: wrap UI text in `t('中文原文')` (renderer) or `mt('中文原文')` (main process), add the English entry under `src/i18n/locales/en/` or `electron/i18n/en.ts`, and run `npm run i18n:check`.

## Native CLI

WeFlow ships a Rust command-line build (`weflow`) next to the desktop app. It defaults to **English** and follows your system locale: Chinese output is used only when `WEFLOW_LANG`, `LC_ALL`, `LC_MESSAGES`, `LANG` or `LANGUAGE` starts with `zh` (first one set wins; anything else, including `C`/`POSIX`, gives English). Override it per run with `--lang en|zh`.

```bash
make build                                  # or: cargo build --release -p weflow-cli
weflow --help
weflow --lang zh export messages <session-id> --out chat.txt
LANG=zh_CN.UTF-8 weflow export messages <session-id> --out chat.txt
```

Docs: [command list](docs/NATIVE-CLI.md) · [coverage of the desktop backend](docs/CLI-COVERAGE.md) · [what the CLI does not cover](docs/CLI-GAPS.md).

## Quick Start

If you just want to use the pre-compiled application, go to [Releases](https://github.com/hicccc77/WeFlow/releases) to download and install.

> ArchLinux users can quickly install with `yay -S weflow`

## Detailed Feature List

The current version supports the following capabilities:

| Feature Module | Description |
|----------------|-------------|
| **Chat** | Decrypt images, videos, and Live Photos in chats (only supports Live Photos captured with Google protocol); supports **modifying** and deleting **local** messages; real-time refresh of latest messages without generating decrypted intermediate databases |
| **Anti-Recall** | Prevent messages sent by others from being recalled |
| **Real-time Notifications** | Desktop popup notifications when new messages arrive, convenient for timely viewing of important conversations, with blacklist/whitelist functionality |
| **Private Chat Analysis** | Statistics on message counts between friends; analysis of message types and sending ratios; view message time distribution, etc. |
| **Group Chat Analysis** | View detailed group member information; analyze group activity rankings, active periods, and media content |
| **Annual Report** | Generate annual reports by year, or long-term historical reports across years |
| **Duo Report** | Select a specific friend and generate an exclusive analysis report based on your mutual chat history |
| **Message Export** | Export WeChat chat history to multiple formats: JSON, HTML, TXT, Excel, CSV, PGSQL, ChatLab proprietary format, etc. |
| **Moments** | Decrypt Moments photos, videos, and Live Photos; export Moments content; intercept deletion and hiding operations in Moments; bypass time-based access restrictions |
| **Contacts** | Export WeChat friends, group chats, and official account information; attempt to recover deleted friends (work in progress) |
| **HTTP API** | Map local message capabilities to HTTP API for easy integration with external systems, automation scripts, and secondary development |

## HTTP API

> [!WARNING]
> This feature is currently in its early stages, and the interface may change. Stay tuned for future updates.

WeFlow provides a local HTTP API service that supports querying message data through interfaces, which can be used for integration with other tools or secondary development.

- **Enable Method**: Settings → API Service → Start Service
- **Default Port**: 5031
- **Access Address**: `http://127.0.0.1:5031`
- **Supported Formats**: Raw JSON or [ChatLab](https://chatlab.fun/) standard format

Complete API documentation: [Click to view](docs/HTTP-API.md)

## For Developers

If you want to build from source or contribute code to the project, please follow these steps:

```bash
# 1. Clone the project locally
git clone https://github.com/hicccc77/WeFlow.git
cd WeFlow

# 2. Install project dependencies
npm install

# 3. Run the application (development mode)
npm run dev
```

## Acknowledgments

- [CipherTalk](https://github.com/ILoveBingLu/miyu) provided the basic framework for this project
- [WeChat-Channels-Video-File-Decryption](https://github.com/Evil0ctal/WeChat-Channels-Video-File-Decryption) provided technical references for video decryption

## Support Us

If WeFlow has truly helped you, consider buying us a coffee:

> TRC20 **Address:** `TZCtAw8CaeARWZBfvjidCnTcfnAtf6nvS6`

## Star History

<a href="https://www.star-history.com/#hicccc77/WeFlow&type=date&legend=top-left">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=hicccc77/WeFlow&type=date&theme=dark&legend=top-left" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=hicccc77/WeFlow&type=date&legend=top-left" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=hicccc77/WeFlow&type=date&legend=top-left" />
  </picture>
</a>

<div align="center">

---

**Please use this tool responsibly and comply with relevant laws and regulations**

</div>
