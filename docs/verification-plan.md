# Planned verification

**English** | [简体中文](zh-CN/verification-plan.md)

What has not been tried against real data yet, and how it will be checked. The native database layer, the exports and the
other commands were verified on one real Windows WeChat 4.x account (Linux build and the Windows `weflow.exe`); the items
below are the remaining gaps. When one is done, record the result here and update
[cli-unsupported.md](cli-unsupported.md) section 4.

| # | Item | Status | How to verify | Done when |
|---|---|---|---|---|
| 1 | **macOS and Linux with real accounts** | Planned | On each platform: `key db` (or the platform key helper), `db detect`/`db test`, then the regression sweep (sessions, messages, contacts, Moments, reports, every export format with `--media`, `export media`, HTTP API). Compare counts with the desktop app. | The sweep passes on both platforms with no unexpected failures; differences are fixed or listed in cli-unsupported.md. |
| 2a | **Windows image auto-download hook** (`image auto-download start`, `serve --image-auto-download`) | Planned | With WeChat running on Windows x64: start the hook, open chats with images that were never downloaded, check that the files appear under `msg/attach/…/Img` and that `export media` then finds them; stop the hook and confirm WeChat keeps working. | Images are downloaded while the hook runs, nothing happens after it stops, WeChat is unaffected. |
| 2b | **AI insight against a real provider** (`insight test`, `insight trigger`, `serve --insight`, footprint summary) | Planned | Configure `ai_model_api_base_url`, `ai_model_api_key`, `ai_model_api_model` for an OpenAI-compatible provider; run `insight test`, a manual trigger and a footprint summary; check the request (`/chat/completions`, no extra `/v1`), the parsed answer and the stored records; optionally Telegram delivery. | All insight commands work end to end with one real provider; errors from the provider are reported clearly. |
| 3 | **Backup compatibility with the desktop app** | Planned | Create a backup with the desktop app and with `weflow backup create`; `weflow backup inspect` both; restore each with the other tool into an empty folder and compare file lists and hashes; open the restored account with `db test` and the desktop app. | Both directions restore the same files, or the differences are documented with a reason. |

Related work in progress elsewhere: the desktop app still loads `wcdb_api.dll` and is being moved onto the Rust layer in a
separate branch (`claude/desktop-rust-layer`).
