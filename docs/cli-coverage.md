# Native Rust CLI — coverage of the original WeFlow backend

**English** | [简体中文](zh-CN/cli-coverage.md)

Baseline: the TypeScript/Electron backend as of the last upstream commit by the original author,
`ca6c479` (2026-05-15). Everything under `crates/` was added afterwards.

**Method and honesty.** A channel counts as *covered* when a CLI command or HTTP route reproduces its behaviour; the
TypeScript code was ported function by function (same formulas, same key order in JSON results, same fallbacks). The
database layer is native Rust and was **verified against one real Windows WeChat 4.x account** (using a Linux build);
everything else (HTTP server, image/`.dat` decryption, AI, Moments downloads) was verified through unit tests and
end-to-end tests against synthetic encrypted databases and local fake HTTP servers. The cross-compiled Windows `weflow.exe` was
run on Windows against the same account (about 80 commands, exports with media); macOS and Linux accounts were not tried. Expect
differences that synthetic data cannot reveal.
The IPC classification below was done by hand; disagree with it if you like.

## Summary

| Measure | Covered | Total | % |
|---|---|---|---|
| Backend IPC channels (`electron/main.ts`; 172 total, 76 UI-only excluded) — full | 79 | 96 | **82%** |
| Same, full + partial | 83 | 96 | **86%** |
| Database functions the CLI calls (native Rust; 44 native, 10 refused as read-only) | 54 | 54 | **100%** |
| Chat-message export formats (chatlab, chatlab-jsonl, json, arkme-json, html, txt, excel, weclone, sql) | 9 | 9 | **100%** |
| HTTP API routes (`httpService.ts`, same path, token auth, SSE push) | 19 | 19 | **100%** |

The 10 write channels (`chat:updateMessage`, `chat:deleteMessage`, `chat:{check,install,uninstall}AntiRevokeTriggers`,
`chat:markAllSessionsRead`, `sns:{check,install,uninstall}BlockDeleteTrigger`, `sns:deleteSnsPost`) used to count as covered;
since the database layer became read-only they are refused and counted as missing (see [cli-unsupported.md](cli-unsupported.md)).

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
- ChatLab export: image, voice, video, emoji and call messages (`<msg>` XML with a non-49 type) were mapped to LINK; only
  real app messages (type 49 / `<appmsg`) are now links. The TypeScript original has the same bug.
- `chat anti-revoke` reported success even when every session failed; it now returns an error.
- The annual report's "top friend per month" came out empty on the native layer (the per-session monthly counts were missing); they are native now, and the extended statistics (heatmap, night owl, initiative, response speed, phrases, streak) are native too, with the same numbers as the cursor fallback.
- Native rows carry `is_send` (computed against the account wxid), which the export and report code relies on.

## Channels that are still missing or partial

**Missing (13):** the 10 write channels above (refused on purpose: the native database layer opens WeChat's databases read-only),
`chat:getVoiceTranscript`, `whisper:downloadModel`, `whisper:getModelStatus` (voice transcription needs sherpa-onnx; not planned).

**Partial (4):**

- `chat:getNewMessages`, message push and insight triggers poll instead of reacting to WCDB monitor callbacks.
- `image:startAutoDownload` / `stopAutoDownload` / `getAutoDownloadStatus`: Windows x64 only; the hook lives only while the
  `weflow` process runs, so `status` from another process cannot see it.

Also different, without changing a channel's classification: WXGF → JPEG needs an `ffmpeg` binary on `PATH` (the desktop app
bundles `ffmpeg-static`), and message exports copy media with `export messages --media …` into `media/<output name>/` instead of
the desktop app's folder layout.

UI events (`image:cacheResolved`, `image:decryptProgress`, `image:updateAvailable`, notification popups) have no CLI counterpart;
like the desktop app's headless worker mode the image service does not emit them and never reports `hasUpdate`.

Not ported because they only make sense in the desktop process: message/contact/session/avatar caches, cloud control, export
task pause/resume.

## Security note

The HTTP server requires a token (`Authorization: Bearer …`, `access_token` query or JSON body) on every route except `/health`;
with no token configured every other request is refused. It binds to `127.0.0.1` by default.

## Reproducing the numbers

- IPC: `grep -oE "ipcMain\.handle\('[^']+'" electron/main.ts` (172), classified by hand as above.
- Database functions: public methods of `Wcdb` in `crates/weflow-native/src/wcdb.rs` that `crates/weflow-core` / `weflow-cli`
  call; the ones that refuse or are not implemented are listed in [cli-unsupported.md](cli-unsupported.md) (kept in sync by a test).
- Export formats: `MESSAGE_EXPORT_FORMATS` in `crates/weflow-core` and `tests/export_e2e.rs`.
- HTTP routes: `pathname ===` / `startsWith('/api/v1/…')` in `electron/services/httpService.ts` vs the router in
  `crates/weflow-core/src/http_server.rs` (`tests/http_e2e.rs`).
