# WeFlow Native CLI

**English** | [简体中文](docs/zh-CN/README.md)

`weflow` is a native Rust command-line build of [WeFlow](docs/weflow-readme.md)'s backend. It reads, analyzes and exports your local WeChat 4.0+ chat history from the terminal, without the Electron desktop app.

- Output is human-readable by default (aligned `key: value` lines and tables; errors go to stderr). Add `--json` (compact) or `--pretty` (indented) for one JSON document on stdout (`{"success": true, "data": ...}`), which makes it easy to script.
- Chat sessions, messages, contacts, Moments, group/private analytics, annual and dual reports.
- Message export in 9 formats: `txt`, `json`, `arkme-json`, `chatlab`, `chatlab-jsonl`, `excel`, `weclone`, `html`, `sql`.
- Images (`.dat` decryption), voice (SILK → WAV), video lookup, stickers.
- Local HTTP API with token auth and SSE push (`serve --http`), message push, AI insights.
- Follows the system language (Chinese or English) for `--help`, usage errors, runtime errors and generated text; `--lang en|zh` overrides it per run.

> [!WARNING]
> The CLI was ported from the original TypeScript backend. Its database layer is pure Rust (it decrypts and reads WeChat's databases itself, read-only) and was verified against one real Windows WeChat 4.x account, with a Linux build and with the Windows `weflow.exe` run on Windows; macOS/Linux WeChat data is untested ([what is still to verify](docs/cli-unsupported.md#still-to-verify)). Expect rough edges and report what you find. What is covered and what is not: [coverage](docs/cli-coverage.md) · [unsupported](docs/cli-unsupported.md).

The original WeFlow project (Electron desktop app) is documented in [docs/weflow-readme.md](docs/weflow-readme.md).

## Build

```bash
make build                                  # or: cargo build --release -p weflow-cli
weflow --help
```

Windows x64 releases are a single `weflow.exe`. WXGF images need `ffmpeg` on `PATH` (or `FFMPEG_PATH`).

## Switching between English and Chinese

The language follows the system: Chinese on a Chinese system, English otherwise. `weflow --lang zh` (on its own, no command) saves the choice in the config file (`weflow config unset lang` goes back to the system language); `weflow --lang zh <command>` applies to a single run, or use the environment:

```powershell
.\weflow.exe --lang zh chat sessions       # Chinese output for this run
$env:WEFLOW_LANG = "zh"                     # Chinese for the whole PowerShell session
```

```bash
LANG=zh_CN.UTF-8 weflow export messages <session-id> --out chat.txt
```

`weflow.exe --lang zh` on its own saves the language in the config file; with a command it applies to that run only. The language affects `--help`, usage errors, runtime errors and generated text; JSON keys and error codes stay English, and so do the HTTP API's error responses. The full order of precedence is in [docs/native-cli.md](docs/native-cli.md#language).

## First-time setup and export (required steps)

Windows PowerShell example (WeChat 4.0+ must have been logged in on this computer at least once). Run PowerShell **as
administrator**, because step 3 hooks the WeChat process. The `#   …` lines show what each command prints.

```powershell
# 1. Find the WeChat data directory (prints the directories that exist) and save it
.\weflow.exe db detect
#   db_path: C:\Users\<you>\Documents\xwechat_files
.\weflow.exe config set db_path "C:\Users\<you>\Documents\xwechat_files"

# 2. Show your wxid and save it
.\weflow.exe db wxid
#   wxid: wxid_xxxxxxxx
.\weflow.exe config set wxid wxid_xxxxxxxx

# 3. Get the database key (follow the prompts, see below) and save it
.\weflow.exe key db
#   decrypt_key: <database key>
.\weflow.exe config set decrypt_key <database key>

# 4. Get the image keys (open a few images in WeChat first) and save them
.\weflow.exe key image
#   image_xor_key: <image xor key>
#   image_aes_key: <image aes key>
.\weflow.exe config set image_xor_key <image xor key>
.\weflow.exe config set image_aes_key <image aes key>

# 5. Required: list the sessions to confirm the connection and find the session ID to export
.\weflow.exe chat sessions

# 6. Export (private chat: the other party's wxid; group: xxx@chatroom)
.\weflow.exe export messages <session-id> --format html --out chat.html
```

The key names printed by `db detect`, `db wxid`, `key db` and `key image` are exactly the names `config set` takes, so each
line can be copied over as it is. `weflow config list` shows what is saved, `weflow config path` where the file is.

**What `key db` does.** WeChat only produces the database key while it opens its databases, so the command guides you through it
(it exits by itself after 180 seconds, `--timeout <seconds>` changes that; the seconds left are shown):

```
WeChat is running (pid 31912). Quit it completely first:
system tray icon -> right click -> Quit WeChat
Waiting for WeChat to quit... (exits automatically in 178 s)
WeChat has quit. Open it again.
Waiting for WeChat to start... (exits automatically in 171 s)
WeChat found (pid 20816). Click "Enter WeChat" in the login window.
Waiting for the key... (exits automatically in 150 s)
Database key obtained
decrypt_key: <database key>
```

If WeChat is not running yet, it starts at "Open WeChat". With `--pid <id>` it hooks that process directly and skips the
quit-and-reopen steps.

Notes for Windows: `key db` needs an administrator terminal (otherwise it reports that it cannot open the WeChat process) and
looks for `Weixin.exe` / `WeChat.exe`. `key image` uses the saved `db_path` and `wxid` to check the keys against an image of
your account; without them (or without images) it prints a "not verified" note. The configuration lives in
`%APPDATA%\weflow\config.json`, the extracted runtime in `%APPDATA%\weflow\runtime\<version>\<target>`. WXGF images need
`ffmpeg` on `PATH` (or `FFMPEG_PATH`) — set it up before exporting images.

Useful export options: `--start 2025-01-01 --end 2025-12-31` (local time, inclusive), `--display-name group-nickname|remark|nickname` (default `group-nickname`: group nickname, then remark, nickname, wxid), `--sender wxid_xxx`, `--excel-compact`. Add `--media all` (or `image,voice,video,emoji`) to copy the media next to the export and link it from the messages; `weflow export media --help` exports media on its own.

## What the CLI does not support

The database layer is pure Rust and **read-only**: it never writes into WeChat's files. So commands that would modify WeChat's
databases (`chat update-message`, `chat delete-message`, `chat anti-revoke`, `chat mark-read`, `sns block-delete`, `sns delete`) are
refused on purpose, and some desktop-app features are missing (voice-to-text, popups and other desktop-process features).

The detailed list is in **[docs/cli-unsupported.md](docs/cli-unsupported.md)** ([简体中文](docs/zh-CN/cli-unsupported.md)).

## Documentation

- [Command list](docs/native-cli.md)
- [What the CLI does not support (detailed list)](docs/cli-unsupported.md) · [Coverage of the original backend](docs/cli-coverage.md)
- [Desktop app on the Rust database layer](docs/desktop-rust-layer.md)
- [`wcdb_api.dll`: not needed by the CLI or the new desktop app; old and new builds](docs/wcdb-api.md)
- [HTTP API](docs/HTTP-API.md) · [macOS key troubleshooting](docs/MAC-KEY-FAQ.md)
- [Original WeFlow README](docs/weflow-readme.md) · [Español](docs/es-ES/weflow-readme.md)

Please use this tool responsibly and comply with relevant laws and regulations.
