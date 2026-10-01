# Plan: a single-file native CLI

**English** | [简体中文](zh-CN/plan.md)

The plan the native CLI was built from, what has been done, and what is still to verify. Moved here from the repository's
`PLAN.md`.

## Goal

Rebuild WeFlow's backend as a **native Rust CLI**, released as one executable per platform (Windows x64/arm64, macOS arm64,
Linux x64), keeping the full backend: key extraction, image decryption, Moments, exports, analytics, HTTP API, AI insights.

- A "single executable" means one binary per platform and architecture, not one binary for every OS.
- The platform helpers that cannot be rewritten (`wx_key.dll`, `img_helper.dll`, `libwx_key.dylib`, `xkey_helper_linux`, the
  WASM decoder) are embedded and unpacked on first run into a versioned cache, then loaded from there.
- The TypeScript services are the reference for the port and the baseline for regression checks.

**Change of plan (2026-10):** the database layer was first meant to call the closed-source `wcdb_api` library through FFI. That
library has an expiry check (after 2026-09-30 23:59:59 `wcdb_init` returns `-1000`) and unverified network code, so the CLI now
decrypts WeChat 4.x databases itself (SQLCipher 4) and reads them **read-only** in pure Rust
(`crates/weflow-native/src/{sqlcipher,native_*}.rs`). `wcdb_api`, `WCDB.dll`, `libwcdb_api.*` and `libWCDB.dylib` are no longer
embedded or loaded; they stay in the repository for reference only.

## Architecture

| Crate | Role |
|---|---|
| `crates/weflow-cli` | Command entry, argument parsing, output protocol |
| `crates/weflow-core` | Configuration, accounts, chats, exports, analytics, Moments, backup, AI insights, HTTP API |
| `crates/weflow-native` | Native database reader (SQLCipher decryption, messages, contacts, Moments, statistics, reports), key helpers, image decryption, WASM, platform wrappers |
| `crates/weflow-assets` | Embedded resources, unpacking, hash check |
| `crates/weflow-silk` | Vendored SILK decoder for voice messages |

Libraries: `clap`, `serde`/`serde_json`, `tokio`/`axum`, `libloading` (platform helpers only), Rust crates for Excel, CSV,
compression and file handling.

Resources: each binary embeds only its platform's helpers, unpacks them into `WEFLOW_HOME/runtime/<version>/<target>/`, checks
the manifest hash on every start (unpacks again when the version or a hash differs) and loads libraries only from that
directory, never implicitly from the current directory.

Configuration: `WEFLOW_HOME`, otherwise `weflow` under the platform's configuration directory; `config.json` (TOML is accepted
too); caches, logs and runtime in separate directories. `weflow config import` migrates the desktop app's readable settings and
skips the encrypted `safe:` / `lock:` values with a hint to set them again.

## CLI contract

- stdout carries one JSON document: `{ "success": true, "data": ..., "meta": ... }` or
  `{ "success": false, "error": { "code": "...", "message": "...", "details": ... } }`.
- Global options: `--config`, `--profile`, `--db-path`, `--decrypt-key`, `--wxid`, `--lang`, `--json` (default), `--pretty`,
  `--progress` (NDJSON on stderr), `--no-progress`, `--progress-delay`.
- Exit codes: `0` ok, `1` runtime error, `2` bad arguments, `3` configuration/key error, `4` database/native library error,
  `130` interrupted.
- The command list is in [native-cli.md](native-cli.md).

## Status

| Step | Status |
|---|---|
| Base: configuration, JSON output, error codes, logging, runtime unpacking | Done |
| Database layer | Done, native Rust and read-only instead of WCDB FFI; writes are refused ([cli-unsupported.md](cli-unsupported.md)) |
| Sessions, messages, search, contacts | Done |
| Exports: JSON, HTML, TXT, Excel, WeClone, SQL, ChatLab, arkme-json, media | Done (9 formats, media with `--media`) |
| Analytics: private and group statistics, annual and dual reports, footprint | Done |
| Media: image `.dat` decryption, video lookup, voice decoding, stickers | Done; voice transcription is not planned |
| Moments, official accounts, backup, HTTP API, message push, AI insights | Done |
| Platform helpers: Windows `wx_key.dll`, `img_helper.dll`; macOS `libwx_key.dylib`; Linux `xkey_helper_linux` | Wired up; only Windows was run with a real account |
| Release builds per platform | Windows x64 builds as a single `weflow.exe`; the GitHub Actions workflows are kept under `.github/weflow/` and do not run |
| Removing Electron/React | Not done: the desktop app stays and loads the same Rust layer through `weflow_wcdb` ([desktop-rust-layer.md](desktop-rust-layer.md)) |

Coverage of the original backend: [cli-coverage.md](cli-coverage.md).

## Tests

- Unit tests in each crate: configuration and legacy config import, the embedded runtime manifest, SQLCipher (round trip, wrong
  key, tampered page, WAL merge), image and Moments decryption, export formats.
- End-to-end tests on synthetic encrypted accounts (`weflow_native::fixture`: SQLCipher pages, WAL, zstd, SILK) and local fake
  HTTP servers, under `crates/weflow-core/tests/`. `cargo test --workspace` runs everything.
- Real-data regression: one Windows WeChat 4.x account, with a Linux build and with the Windows `weflow.exe` run on Windows
  (about 80 commands, message exports with media, image exports, the HTTP API).

## Verification still to do

What has not been tried against real data yet, and how it will be checked. When an item is done, record the result here and
update section 4 of [cli-unsupported.md](cli-unsupported.md).

| # | Item | Status | How to verify | Done when |
|---|---|---|---|---|
| 1 | **macOS and Linux with real accounts** | Planned | On each platform: `key db` (or the platform key helper), `db detect`/`db test`, then the regression sweep (sessions, messages, contacts, Moments, reports, every export format with `--media`, `export media`, HTTP API). Compare counts with the desktop app. | The sweep passes on both platforms with no unexpected failures; differences are fixed or listed in cli-unsupported.md. |
| 2a | **Windows image auto-download hook** (`image auto-download start`, `serve --image-auto-download`) | Planned | With WeChat running on Windows x64: start the hook, open chats with images that were never downloaded, check that the files appear under `msg/attach/…/Img` and that `export media` then finds them; stop the hook and confirm WeChat keeps working. | Images are downloaded while the hook runs, nothing happens after it stops, WeChat is unaffected. |
| 2b | **AI insight against a real provider** (`insight test`, `insight trigger`, `serve --insight`, footprint summary) | Planned | Configure `ai_model_api_base_url`, `ai_model_api_key`, `ai_model_api_model` for an OpenAI-compatible provider; run `insight test`, a manual trigger and a footprint summary; check the request (`/chat/completions`, no extra `/v1`), the parsed answer and the stored records; optionally Telegram delivery. | All insight commands work end to end with one real provider; errors from the provider are reported clearly. |
| 3 | **Backup compatibility with the desktop app** | Planned | Create a backup with the desktop app and with `weflow backup create`; `weflow backup inspect` both; restore each with the other tool into an empty folder and compare file lists and hashes; open the restored account with `db test` and the desktop app. | Both directions restore the same files, or the differences are documented with a reason. |
| 4 | **Desktop app on the Rust layer, with its UI** (branch `claude/desktop-rust-layer`) | Planned | Build with `npm run build` on Windows, macOS and Linux; open an account, browse chats, contacts, groups, Moments, run each report and export, watch new messages arrive (monitor pipe), try an edit/delete (expect the read-only error). | The app works for the read-only features on all three platforms; differences are fixed or documented in [desktop-rust-layer.md](desktop-rust-layer.md). |

Release checks, once release builds exist: the downloaded executable runs on its own, the first run creates the runtime cache, a
deleted cache is restored, nothing needs Node, npm or Electron.
