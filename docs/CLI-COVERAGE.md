# Native Rust CLI — coverage of the original WeFlow backend

**English** | [简体中文](zh-CN/CLI-COVERAGE.md)

Baseline: the TypeScript/Electron backend as of the last upstream commit by the original author,
`ca6c479` (2026-05-15). Everything under `crates/` was added afterwards.

**Method and honesty.** A channel counts as *covered* when a CLI command or HTTP route reproduces its behaviour; the
TypeScript code was ported function by function (same formulas, same key order in JSON results, same fallbacks). **None of
it has been run against real WeChat data**: the repository has no account data and no real WCDB library on Linux, so every
behaviour was verified through unit tests and end-to-end tests against a generated mock WCDB library
(`crates/weflow-native/tests/fixtures/gen_mock.py`) plus local fake HTTP servers. Expect differences the mock cannot reveal.
The IPC classification below was done by hand; disagree with it if you like.

## Summary

| Measure | Covered | Total | % |
|---|---|---|---|
| Backend IPC channels (`electron/main.ts`; 172 total, 76 UI-only excluded) — full | 81 | 96 | **84%** |
| Same, full + partial | 90 | 96 | **94%** |
| WCDB C-ABI functions (`wcdbCore.ts` → `weflow-native`) | 90 | 90 | **100%** |
| Chat-message export formats (chatlab, chatlab-jsonl, json, arkme-json, html, txt, excel, weclone, sql) | 9 | 9 | **100%** |
| HTTP API routes (`httpService.ts`, same path, token auth, SSE push) | 19 | 19 | **100%** |

UI-only channels excluded: `window:*`, `dialog:*`, `shell:*`, `app:*`, `auth:*`, `log:*`, `cloud:*`, `diagnostics:*`,
`social:*`, `http:*` start/stop, plus the renderer-only `annualReport:{captureCurrentWindow,exportImages,startAvailableYearsLoad,
cancelAvailableYearsLoad}` and `sns:{getCacheMigrationStatus,startCacheMigration}` (96 remain).

## What was added since the first evaluation

Moments service (timeline, stats, media proxy/decrypt with ISAAC-64, sticker download, json/html/arkmejson export, block-delete
triggers), chat message model and all 40-odd chat queries, group analytics, analytics incl. exclusions, annual and dual reports
(with the cursor fallbacks), the full HTTP API (token auth, media, SNS routes, SSE push), message push engine, the nine message
export formats, video lookup, voice decoding (SILK → WAV), image decryption with session-month `.dat` lookup and HD promotion,
the Windows image auto-download hook, the AI insight engine (records, silence scan, activity trigger, Telegram, footprint
summary) and the Weibo context client.

## Fixes found while porting

- `.dat` decryption: V1 files use the default key `cfcd208495d565ef`; V1/V2 share the layout *AES-128-ECB head (PKCS7) · raw
  middle · XOR tail*. The first CLI version XOR-ed everything after the head.
- Derived image AES key is `md5(code + wxid)` hex **first 16 characters used as ASCII**, as the desktop key service does (the
  first CLI version used the 16 digest bytes).
- The AI endpoint is `<base>/chat/completions`; the first CLI version inserted an extra `/v1`.
- TypeScript's ISAAC-64 fallback has a precision bug (`Number(x>>3n)&255`); the Rust port follows the vendor WASM, which is the
  authoritative implementation.

## Channels that are still missing or partial

**Missing (6):** `chat:clearCurrentAccountData`, `chat:getVoiceTranscript`, `whisper:downloadModel`, `whisper:getModelStatus`
(voice transcription needs sherpa-onnx), `cache:clearAll` (only the analytics and image caches can be cleared),
`sns:debugResource`.

**Partial (9):**

- `chat:getNewMessages`, message push and insight triggers poll instead of reacting to WCDB monitor callbacks.
- `chat:getContacts` / `getContact`: no contact labels, signature or region (needs the extended-column parser and the 9.4k-line
  region table).
- `export:exportSession(s)`, `export:getExportStats`: message exports do not embed media files (image/voice/video/emoji
  export runs separately via `export media` or the HTTP API).
- `image:startAutoDownload` / `stopAutoDownload` / `getAutoDownloadStatus`: Windows x64 only; the hook lives only while the
  `weflow` process runs, so `status` from another process cannot see it.
- WXGF → JPEG needs an `ffmpeg` binary on `PATH` (the desktop app bundles `ffmpeg-static`).
- `localeCompare` ordering of Chinese names is approximated.

UI events (`image:cacheResolved`, `image:decryptProgress`, `image:updateAvailable`, notification popups) have no CLI counterpart;
like the desktop app's headless worker mode the image service does not emit them and never reports `hasUpdate`.

Not ported because they only make sense in the desktop process: message/contact/session/avatar caches, cloud control, export
task pause/resume.

## Security note

The HTTP server requires a token (`Authorization: Bearer …`, `access_token` query or JSON body) on every route except `/health`;
with no token configured every other request is refused. It binds to `127.0.0.1` by default.

## Reproducing the numbers

- IPC: `grep -oE "ipcMain\.handle\('[^']+'" electron/main.ts` (172), classified by hand as above.
- WCDB: `lib.func('…wcdb_*(` declarations in `electron/services/wcdbCore.ts` (90); every name appears in
  `crates/weflow-native/src` or `crates/weflow-core/src`.
- Export formats: `MESSAGE_EXPORT_FORMATS` in `crates/weflow-core` and `tests/export_e2e.rs`.
- HTTP routes: `pathname ===` / `startsWith('/api/v1/…')` in `electron/services/httpService.ts` vs the router in
  `crates/weflow-core/src/http_server.rs` (`tests/http_e2e.rs`).
