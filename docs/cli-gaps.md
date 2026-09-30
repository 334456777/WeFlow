# Native Rust CLI — what is NOT covered

**English** | [简体中文](zh-CN/cli-gaps.md)

Companion to [cli-coverage.md](cli-coverage.md). Baseline is the original TypeScript backend (`ca6c479`). Everything below is
either missing or behaves differently from the desktop app. The native database layer has been verified against one real account (section 4); the complete list of what does not
work at all is in [cli-unsupported.md](cli-unsupported.md).

## 1. Missing entirely

| Desktop channel | What it does | Why it is missing |
|---|---|---|
| `chat:getVoiceTranscript` | Speech-to-text of a voice message | Needs sherpa-onnx + Whisper models; no Rust binding wired up |
| `whisper:downloadModel`, `whisper:getModelStatus` | Download / inspect the Whisper model | Same |
| `chat:clearCurrentAccountData` | Wipes cached data of the current account | Not ported |
| `cache:clearAll` | Clears every cache | Only `analytics clear-cache` and `image clear-cache` exist; the message/contact/avatar caches are not ported (the CLI has no long-lived cache) |
| `sns:debugResource` | Debug dump of a Moments resource | Not ported |

## 2. Partial or different behaviour

| Area | Difference |
|---|---|
| Message push / insight triggers / `chat:getNewMessages` | The desktop app reacts to WCDB monitor callbacks; the CLI polls (push ≈ 5 s, insight ≈ 5 s). |
| Contacts (`chat:getContacts`, `getContact`) | No contact labels, signature or region: needs the extended-column parser and the ~9.4k-line region table. |
| Message export (`export messages`, `exportSession(s)`, `getExportStats`) | All 9 formats work, but media files are **not embedded** into the export; export media separately (`export media`, HTTP API `media=1`). |
| Voice in the HTTP API / `chat voice-data` | Works only if the media database holds the SILK data (WeChat must have played the message once). |
| WXGF images | Converted through an external `ffmpeg` (`PATH` or `FFMPEG_PATH`); the desktop app bundles `ffmpeg-static`. Without it the image is reported as a failed decrypt. |
| Image auto-download (`image auto-download`, `serve --image-auto-download`) | Windows x64 only (`img_helper.dll`). The hook lives only while the `weflow` process runs, so `status` from another process always says "not hooked". |
| Image service events | `image:cacheResolved`, `decryptProgress`, `updateAvailable` and the background "better quality available" check are not emitted; `hasUpdate` is always `false`. |
| AI insight notifications | No popup window; `serve --insight` prints each insight as a JSON line on stderr (Telegram push still works). |
| Image key memory scan | `key scan-image` (scan WeChat memory for the AES key) works on macOS only; on Windows use `key image` (kvcomm cache + template verification). The desktop app's Windows memory-scan fallback is not ported. |
| Sorting | `localeCompare` ordering of Chinese names is approximated. |
| Video | Looks up the file WeChat already stored under `msg/video`; there is no download or decrypt path (the desktop app has none either). |

## 3. Deliberately not ported (desktop-process concerns)

Window/dialog/shell/app/auth/log IPC, auto update, autostart, app lock, cloud control, diagnostics, social-cookie UI helpers,
export task pause/resume, renderer-only report screenshots, the Moments cache-migration UI.

## 4. Not verified

- The native database layer (sessions, messages, contacts, Moments, voice, reports) was verified against one real Windows
  WeChat 4.x account using a Linux build. The other parts (HTTP server, image/`.dat` decryption, AI, Moments network
  downloads) were tested only against synthetic encrypted fixtures and fake HTTP servers, never against real `.dat` files,
  real Moments servers or a real AI provider.
- The Windows build is cross-compiled (`x86_64-pc-windows-gnu`) from Linux; it has not been run on Windows yet.
- Backup archives are not verified for compatibility with the desktop app's own backups.
- The Windows-only image hook and the key-extraction helpers can only be tested on a machine with WeChat running.

## 5. Bugs of the original code that were fixed rather than copied

TypeScript's ISAAC-64 fallback (precision bug, the vendor WASM is followed), and three defects in the *earlier Rust CLI*
(`.dat` layout/keys, derived AES key, AI endpoint `/v1`). See cli-coverage.md.

## The database layer is native Rust (no `wcdb_api`)

The closed-source `wcdb_api` library is **no longer used, embedded or loaded**: it had a built-in validity check (after
2026-09-30 23:59:59 local time `wcdb_init` returned `-1000` and the library tried to delete itself) and unverified network
code. The CLI now decrypts the WeChat 4.x databases itself (SQLCipher 4 layout: PBKDF2-HMAC-SHA512 with the raw 32-byte key,
AES-256-CBC pages with HMAC-SHA512, encrypted WAL) and reads them with bundled SQLite; nothing plaintext is written to disk.

Ported so far: sessions, messages (all shards, zstd-compressed content, paging, per-day statistics, cursors), contacts,
display names, avatars, group members/nicknames/owner. Every other database call still returns
`... is not implemented in the native database backend yet` until it is ported; see `crates/weflow-native/src/wcdb.rs` for
the list and `#[ignore = "pending native port ..."]` in `crates/weflow-core/tests` for the tests waiting on them.
