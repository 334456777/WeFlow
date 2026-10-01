# WeFlow Native CLI

**English** | [简体中文](README.zh-CN.md)

`weflow` is a native Rust command-line build of [WeFlow](docs/weflow-readme.md)'s backend. It reads, analyzes and exports your local WeChat 4.0+ chat history from the terminal, without the Electron desktop app.

- Every command prints one JSON document on stdout (`{"success": true, "data": ...}`), which makes it easy to script.
- Chat sessions, messages, contacts, Moments, group/private analytics, annual and dual reports.
- Message export in 9 formats: `txt`, `json`, `arkme-json`, `chatlab`, `chatlab-jsonl`, `excel`, `weclone`, `html`, `sql`.
- Images (`.dat` decryption), voice (SILK → WAV), video lookup, stickers.
- Local HTTP API with token auth and SSE push (`serve --http`), message push, AI insights.
- Follows the system language (Chinese or English); `--lang en|zh` overrides it per run.

> [!WARNING]
> The CLI was ported from the original TypeScript backend. Its database layer is pure Rust (it decrypts and reads WeChat's databases itself, read-only) and was verified against one real Windows WeChat 4.x account using a Linux build; the Windows `weflow.exe` is cross-compiled and has not been run on Windows yet, and macOS/Linux WeChat data is untested. Expect rough edges and report what you find. What is covered and what is not: [coverage](docs/cli-coverage.md) · [gaps](docs/cli-gaps.md) · [unsupported](docs/cli-unsupported.md).

The original WeFlow project (Electron desktop app) is documented in [docs/weflow-readme.md](docs/weflow-readme.md).

## Build

```bash
make build                                  # or: cargo build --release -p weflow-cli
weflow --help
```

Windows x64 releases are a single `weflow.exe`. WXGF images need `ffmpeg` on `PATH` (or `FFMPEG_PATH`).

## Switching between English and Chinese

The language follows the system: Chinese on a Chinese system, English otherwise. Use `--lang en|zh` for a single run, or the environment:

```powershell
.\weflow.exe --lang zh chat sessions --pretty    # Chinese output for this run
$env:WEFLOW_LANG = "zh"                          # Chinese for the whole PowerShell session
```

```bash
LANG=zh_CN.UTF-8 weflow export messages <session-id> --out chat.txt
```

Order: `--lang`, then the first of `WEFLOW_LANG`, `LC_ALL`, `LC_MESSAGES`, `LANG`, `LANGUAGE` that is set and non-empty (Chinese when it starts with `zh`; anything else, including `C`/`POSIX`, gives English), then — when none of them is set, which is usual on Windows — the operating system's display language (Windows, macOS), then English. `--lang` is an option, not a command: it must accompany a subcommand, e.g. `weflow.exe --lang zh chat sessions --pretty`; running `weflow.exe --lang zh` alone reports a missing subcommand. The language only affects generated text (export labels such as `[Image]` / `[图片]`, default AI prompt); JSON keys, error codes and `--help` are always English.

## First-time setup and export (required steps)

Windows PowerShell example. Log in to WeChat (4.0+) and keep it running:

```powershell
# 1. Find the WeChat data directory
.\weflow.exe db detect --pretty

# 2. Get the database key and the image keys (run PowerShell as administrator)
#    `key db` hooks WeChat and WAITS (180 s by default): log out and log in to WeChat (or restart it)
#    while it is waiting, the key only appears while WeChat opens its databases.
.\weflow.exe key db --pretty
.\weflow.exe key image --pretty

# 3. Save the configuration (once)
.\weflow.exe config set db_path "C:\Users\<you>\Documents\xwechat_files"
.\weflow.exe config set wxid wxid_xxxxxxxx
.\weflow.exe config set decrypt_key <database key>
.\weflow.exe config set image_xor_key <image xor key>
.\weflow.exe config set image_aes_key <image aes key>

# 4. Required: list the sessions to confirm the connection and find the session ID to export
.\weflow.exe chat sessions --pretty

# 5. Export (private chat: the other party's wxid; group: xxx@chatroom)
.\weflow.exe export messages <session-id> --format html --out chat.html
```

Notes for Windows: `key db` needs an administrator terminal (otherwise it reports "permission denied"), finds `Weixin.exe` / `WeChat.exe` automatically (or use `--pid`), and `--timeout <seconds>` changes the wait. The configuration lives in `%APPDATA%\weflow\config.json`, the extracted runtime in `%APPDATA%\weflow\runtime\<version>\<target>`. WXGF images need `ffmpeg` on `PATH` (or `FFMPEG_PATH`) — set it up before exporting images.

Useful export options: `--start 2025-01-01 --end 2025-12-31` (local time, inclusive), `--display-name remark|nickname|group-nickname`, `--sender wxid_xxx`, `--excel-compact`. Add `--media all` (or `image,voice,video,emoji`) to copy the media next to the export and link it from the messages; `weflow export media --help` exports media on its own.

## What the CLI does not support

The database layer is pure Rust and **read-only**: it never writes into WeChat's files. So commands that would modify WeChat's
databases (`chat update-message`, `chat delete-message`, `chat anti-revoke`, `chat mark-read`, `sns block-delete`, `sns delete`) are
refused on purpose, and some desktop-app features are missing (voice-to-text, cache management, and more).

The detailed list is in **[docs/cli-unsupported.md](docs/cli-unsupported.md)** ([简体中文](docs/zh-CN/cli-unsupported.md)).

## Documentation

- [Command list](docs/native-cli.md)
- [What the CLI does not support (detailed list)](docs/cli-unsupported.md)
- [Coverage of the original backend](docs/cli-coverage.md) · [What the CLI does not cover](docs/cli-gaps.md)
- [HTTP API](docs/HTTP-API.md) · [macOS key troubleshooting](docs/MAC-KEY-FAQ.md)
- [Original WeFlow README](docs/weflow-readme.md)

Please use this tool responsibly and comply with relevant laws and regulations.
