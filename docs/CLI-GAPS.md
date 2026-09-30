# Native Rust CLI — what is NOT covered

**English** | [简体中文](zh-CN/CLI-GAPS.md)

Companion to [CLI-COVERAGE.md](CLI-COVERAGE.md). Baseline is the original TypeScript backend (`ca6c479`). Everything below is
either missing or behaves differently from the desktop app. Nothing in the CLI has been verified against real WeChat data.

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
| Voice in the HTTP API / `chat voice-data` | Works only if WCDB returns the SILK blob (WeChat must have played the message once). |
| WXGF images | Converted through an external `ffmpeg` (`PATH` or `FFMPEG_PATH`); the desktop app bundles `ffmpeg-static`. Without it the image is reported as a failed decrypt. |
| Image auto-download (`image auto-download`, `serve --image-auto-download`) | Windows x64 only (`img_helper.dll`). The hook lives only while the `weflow` process runs, so `status` from another process always says "not hooked". |
| Image service events | `image:cacheResolved`, `decryptProgress`, `updateAvailable` and the background "better quality available" check are not emitted; `hasUpdate` is always `false`. |
| AI insight notifications | No popup window; `serve --insight` prints each insight as a JSON line on stderr (Telegram push still works). |
| Sorting | `localeCompare` ordering of Chinese names is approximated. |
| Video | Looks up the file WeChat already stored under `msg/video`; there is no download or decrypt path (the desktop app has none either). |

## 3. Deliberately not ported (desktop-process concerns)

Window/dialog/shell/app/auth/log IPC, auto update, autostart, app lock, cloud control, diagnostics, social-cookie UI helpers,
export task pause/resume, renderer-only report screenshots, the Moments cache-migration UI.

## 4. Not verified

- Every port was tested against a **generated mock WCDB library** and fake HTTP servers, never against a real WeChat database,
  the real `wcdb_api` library, real `.dat` files, real Moments servers or a real AI provider.
- The Windows build is cross-compiled (`x86_64-pc-windows-gnu`) from Linux; it has not been run on Windows by the author.
- Backup archives are not verified for compatibility with the desktop app's own backups.
- The Windows-only image hook and the key-extraction helpers can only be tested on a machine with WeChat running.

## 5. Bugs of the original code that were fixed rather than copied

TypeScript's ISAAC-64 fallback (precision bug, the vendor WASM is followed), and three defects in the *earlier Rust CLI*
(`.dat` layout/keys, derived AES key, AI endpoint `/v1`). See CLI-COVERAGE.md.
