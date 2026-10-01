# Native CLI (`weflow`)

**English** | [简体中文](zh-CN/native-cli.md)

A Rust command-line build of WeFlow's backend. By default a command prints human-readable text on stdout (aligned `key: value` lines, tables for lists) and
errors on stderr, with the same exit codes. With `--json` (compact) or `--pretty` (indented) every command prints one JSON document on stdout
(`{"success": true, "data": ...}` or `{"success": false, "error": {...}}`); progress goes to stderr with `--progress`.

## Language

The language follows the system (Chinese on a Chinese system, English otherwise). In order of precedence: `--lang`, `WEFLOW_LANG`, the language saved in the config file (`weflow --lang zh` on its own saves it; `weflow config unset lang` removes it), the environment
(the first variable that is set and non-empty decides), then the operating system's display language:

`WEFLOW_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `LANGUAGE`

A value starting with `zh` (`zh_CN.UTF-8`, `zh-TW`, `zh`) gives Chinese; anything else, including `C` and `POSIX`, gives English.
When none of the variables is set (usual on Windows) the OS display language decides (Windows, macOS); if it cannot be determined, English.
`--lang en|zh <command>` overrides all of this for a single run.

The language affects `--help`, usage errors, runtime errors, progress text and generated text: TXT/Excel export labels (`[Image]` / `[图片]`), the default official-account payment
merchant name, and the default AI insight prompt. JSON keys, error codes and the HTTP API's error responses stay English.

## Commands

```
weflow config    list | get | set | unset | clear | import
weflow db        detect | scan <root> | wxid [root] | test | open
weflow key       db | image | scan-image <user-dir>
weflow chat      sessions | messages | latest | search | contacts | contact | update-message | delete-message
                 anti-revoke | message | dates | date-counts | counts | statuses | detail | mark-read | tab-counts
                 export-stats | group-hint | resources | images | voice-messages | media-stream | transfer-names
                 voice | voice-data | voice-cache | voice-preload | image-data | emoji | clear-account-data
weflow export    sessions | contacts | footprint | media | messages   (messages: chatlab, chatlab-jsonl, json,
                 arkme-json, html, txt, excel, weclone, sql)
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

The database layer is native Rust and **read-only**: `chat update-message`, `chat delete-message`, `chat anti-revoke`, `chat mark-read`, `sns block-delete` and `sns delete` would modify WeChat's databases and are always refused (see [cli-unsupported.md](cli-unsupported.md) for these and everything else that is not supported). Exit code `4` is also used when a database cannot be opened (wrong key, unreadable file).

Progress: commands that run longer than the delay (default 5 seconds; `--progress-delay <s>`, env `WEFLOW_PROGRESS_DELAY`, or `weflow config set progress_delay_seconds <s>`, `0` = always) show a single-line progress bar on stderr (only when stderr is a
terminal; stdout is never touched). `--no-progress` turns it off, `--progress` prints machine-readable NDJSON events instead.

`export media --type image|voice|video|emoji|all [--session <id>] [--start YYYY-MM-DD --end YYYY-MM-DD]` walks the media
messages (an image sent twice counts twice, so `found` can exceed the unique files listed by `chat images`). `missing` counts
messages whose file is not on disk (never downloaded in WeChat) or could not be resolved, per kind in `missingByKind`.
`thumbOnly` counts exported images that are only the thumbnail (each image entry also has `isThumb`); open the original in WeChat and export again to get the HD file. `export media` always prefers the HD original (like `image decrypt --force`). Stickers may need network access; voice export decodes every message, so a full export of hundreds of voices takes minutes.

`export messages` reads only the requested date range from the database (cost follows the range size, not its age) and shows a progress bar; `--start/--end` are dates in the machine's local time zone. `--media image,voice,video,emoji` (or `all`) copies the media into `media/<output name>/` beside the output file and points the messages at the copies (for `json`, `arkme-json`, `txt`, `excel`, `weclone`, `html`; `chatlab` for images; not `sql`). The messages are written as they are built, so a 200,000-message group needs about 0.1 GB (`txt`) to 0.6 GB (`json`, `html`). `--sender` keeps one person's messages in every format; plain `txt` names senders by group nickname, then remark, nickname, alias, unless `--display-name` says otherwise.

`chat clear-account-data --cache [--exports-dir <dir>] --yes` removes WeFlow's caches of the current account (images, voices,
stickers, Moments, analytics) and signs the account out of the profile (`db_path`, `wxid`, `decrypt_key` and the image keys are
removed); `--exports-dir` also removes the entries named after the account in that folder. `cache clear-all` clears every cache.
Neither touches WeChat's files.

`key db` (Windows) hooks WeChat through `wx_key.dll`. WeChat only produces the key while it opens its databases, so the command
asks you to quit WeChat completely (when it is running) and open it again, checks once a second for the process (the
whole procedure gives up after `--timeout`, default 180 s, and shows the seconds left), hooks the new process and asks you to
click "Enter WeChat" in the login window. It prints `decrypt_key: <key>`, the name `config set` expects. It needs an
administrator terminal, looks for `Weixin.exe` then `WeChat.exe`, and with `--pid` it hooks that process directly instead of
waiting for a restart. `key image` derives the image keys from the `kvcomm` cache, verifies them against a `_t.dat` template
under the account directory, and prints `image_xor_key` and `image_aes_key`.

Paths: configuration `%APPDATA%\weflow\config.json`, extracted runtime `%APPDATA%\weflow\runtime\<version>\<target>`
(on Linux/macOS under the platform's data directory).

`serve --http` exposes the desktop app's HTTP API (token required except `/health`; set `http_api_token` or `--api-token`).
`serve --insight` runs the AI insight engine; each generated insight is printed to stderr (as a JSON line with `--json`).
`image auto-download` and `serve --image-auto-download` hook WeChat through `img_helper.dll` and only work on Windows x64.
Voice messages are decoded with a vendored copy of the Skype SILK SDK (`crates/weflow-silk`); WXGF images need `ffmpeg`
on `PATH` (or `FFMPEG_PATH`).

Exit codes: `0` ok, `1` runtime error, `2` bad arguments, `3` config/key error, `4` database/native library error, `130` interrupted.

## Architecture

| Crate | Role |
|---|---|
| `crates/weflow-cli` | Command entry, argument parsing, output |
| `crates/weflow-core` | Configuration, accounts, chats, exports, analytics, Moments, backup, AI insights, HTTP API |
| `crates/weflow-native` | Native database reader (SQLCipher decryption, messages, contacts, Moments, statistics, reports), key helpers, image decryption, WASM, platform wrappers |
| `crates/weflow-assets` | Embedded resources, unpacking, hash check |
| `crates/weflow-silk` | Vendored SILK decoder for voice messages |

The platform helpers that cannot be rewritten (`wx_key.dll`, `img_helper.dll`, `libwx_key.dylib`, `xkey_helper_linux`, the WASM
decoder) are embedded: each binary carries only its own platform's helpers, unpacks them into
`WEFLOW_HOME/runtime/<version>/<target>/`, checks the manifest hash on every start (unpacking again when the version or a hash
differs) and loads libraries only from that directory, never implicitly from the current directory.

Configuration lives in `WEFLOW_HOME`, otherwise in `weflow` under the platform's configuration directory: `config.json` (TOML is
accepted too), with caches, logs and the runtime in separate directories. `weflow config import` migrates the desktop app's
readable settings and skips the encrypted `safe:` / `lock:` values with a hint to set them again.

**Why the database layer is pure Rust.** The CLI was first meant to call the closed-source `wcdb_api` library through FFI. That
library has an expiry check (after 2026-09-30 23:59:59 `wcdb_init` returns `-1000`) and unverified network code, so the CLI now
decrypts WeChat 4.x databases itself (SQLCipher 4) and reads them **read-only** in pure Rust
(`crates/weflow-native/src/{sqlcipher,native_*}.rs`). Neither the CLI nor the desktop app embeds or loads `wcdb_api`,
`WCDB.dll`, `libwcdb_api.*` or `libWCDB.dylib` any more; they stay in the repository for the original desktop app, see
[wcdb-api.md](wcdb-api.md). Coverage of the original backend: [cli-coverage.md](cli-coverage.md).

## Tests

- Unit tests in each crate: configuration and legacy config import, the embedded runtime manifest, SQLCipher (round trip, wrong
  key, tampered page, WAL merge), image and Moments decryption, export formats.
- End-to-end tests on synthetic encrypted accounts (`weflow_native::fixture`: SQLCipher pages, WAL, zstd, SILK) and local fake
  HTTP servers, under `crates/weflow-core/tests/`. `cargo test --workspace` runs everything.
- Real-data regression: one Windows WeChat 4.x account, with a Linux build and with the Windows `weflow.exe` run on Windows
  (about 80 commands, message exports with media, image exports, the HTTP API). What is still unverified is listed in
  [cli-unsupported.md](cli-unsupported.md#still-to-verify).
