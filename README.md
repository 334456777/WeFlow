# WeFlow Native CLI

**English** | [简体中文](README.zh-CN.md)

`weflow` is a native Rust command-line build of [WeFlow](docs/WEFLOW-README.md)'s backend. It reads, analyzes and exports your local WeChat 4.0+ chat history from the terminal, without the Electron desktop app.

- Every command prints one JSON document on stdout (`{"success": true, "data": ...}`), which makes it easy to script.
- Chat sessions, messages, contacts, Moments, group/private analytics, annual and dual reports.
- Message export in 9 formats: `txt`, `json`, `arkme-json`, `chatlab`, `chatlab-jsonl`, `excel`, `weclone`, `html`, `sql`.
- Images (`.dat` decryption), voice (SILK → WAV), video lookup, stickers.
- Local HTTP API with token auth and SSE push (`serve --http`), message push, AI insights.
- English by default, Chinese on request.

> [!WARNING]
> The CLI was ported from the original TypeScript backend and tested against a mock WCDB library, **not against real WeChat data**. Expect rough edges and report what you find. What is covered and what is not: [coverage](docs/CLI-COVERAGE.md) · [gaps](docs/CLI-GAPS.md).

The original WeFlow project (Electron desktop app) is documented in [docs/WEFLOW-README.md](docs/WEFLOW-README.md).

## Build

```bash
make build                                  # or: cargo build --release -p weflow-cli
weflow --help
```

Windows x64 releases are a single `weflow.exe`. WXGF images need `ffmpeg` on `PATH` (or `FFMPEG_PATH`).

## Switching between English and Chinese

English is the default. Use `--lang en|zh` for a single run, or the environment:

```powershell
.\weflow.exe --lang zh chat sessions --pretty    # Chinese output for this run
$env:WEFLOW_LANG = "zh"                          # Chinese for the whole PowerShell session
```

```bash
LANG=zh_CN.UTF-8 weflow export messages <session-id> --out chat.txt
```

Chinese is used only when `WEFLOW_LANG`, `LC_ALL`, `LC_MESSAGES`, `LANG` or `LANGUAGE` starts with `zh` (the first one that is set wins; anything else, including `C`/`POSIX`, gives English). `--lang` is an option, not a command: it must accompany a subcommand, e.g. `weflow.exe --lang zh chat sessions --pretty`; running `weflow.exe --lang zh` alone reports a missing subcommand. The language only affects generated text (export labels such as `[Image]` / `[图片]`, default AI prompt); JSON keys, error codes and `--help` are always English.

## First-time setup and export (required steps)

Windows PowerShell example. Log in to WeChat (4.0+) and keep it running:

```powershell
# 1. Find the WeChat data directory
.\weflow.exe db detect --pretty

# 2. Get the database key (WeChat must be running) and the image keys
.\weflow.exe key db --pretty
.\weflow.exe key image --pretty

# 3. Save the configuration (once)
.\weflow.exe config set db_path "C:\Users\<you>\Documents\xwechat_files"
.\weflow.exe config set wxid wxid_xxxxxxxx
.\weflow.exe config set decrypt_key <database key>
.\weflow.exe config set image_xor_key <image xor key>
.\weflow.exe config set image_aes_key <image aes key>

# 4. Required: list the sessions to confirm the connection and find the session ID to export
.\weflow.exe --lang zh chat sessions --pretty

# 5. Export (private chat: the other party's wxid; group: xxx@chatroom)
.\weflow.exe export messages <session-id> --format html --out chat.html
```

Useful export options: `--start 2025-01-01 --end 2025-12-31` (Beijing time, inclusive), `--display-name remark|nickname|group-nickname`, `--sender wxid_xxx`, `--excel-compact`. Messages are exported without embedded media; export media separately with `weflow export media --help`.

## Documentation

- [Command list](docs/NATIVE-CLI.md)
- [Coverage of the original backend](docs/CLI-COVERAGE.md) · [What the CLI does not cover](docs/CLI-GAPS.md)
- [HTTP API](docs/HTTP-API.md) · [macOS key troubleshooting](docs/MAC-KEY-FAQ.md)
- [Original WeFlow README](docs/WEFLOW-README.md)

Please use this tool responsibly and comply with relevant laws and regulations.
