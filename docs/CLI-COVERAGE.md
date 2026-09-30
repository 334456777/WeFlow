# Native Rust CLI — coverage of the original WeFlow backend

Baseline: the TypeScript/Electron backend as of the last upstream commit by the original author,
`ca6c479` (2026-05-15). Everything under `crates/` was added afterwards (`70ef1b1` onwards).

**Method.** Counting is by surface, not by behaviour: a channel counts as *covered* when a CLI command or HTTP route
exists for it. It does not mean the output is identical to the desktop app, and nothing here was run against real WeChat
data. Judgement calls are listed so you can disagree with them.

## Summary

| Measure | Covered | Total | % |
|---|---|---|---|
| Backend IPC channels (`electron/main.ts`, 172 total, 69 UI-only excluded) — full | 60 | 103 | **58%** |
| Same, full + partial | 75 | 103 | **73%** |
| WCDB C-ABI functions (`wcdbCore.ts` → `weflow-native`) | 39 | 90 | **43%** |
| Chat-message export formats (chatlab, chatlab-jsonl, json, arkme-json, html, txt, excel, weclone, sql) | 1 (txt) | 9 | **11%** |
| HTTP API routes (`httpService.ts`, same path) | 8 | 17 | **47%** (+2 under different paths) |

UI-only channels excluded from the IPC count: `window:*`, `dialog:*`, `shell:*`, `app:*` (updates, autostart), `auth:*`
(app lock), `log:*`, plus cache/cloud/diagnostics/social-cookie/pause-resume/HTTP-start-stop channels that have no meaning
in a stateless CLI.

## Per area

| Area | Status |
|---|---|
| Config, db path detection, key extraction (db/image/memory scan) | Covered |
| Backup create / inspect / restore | Covered (archive format compatibility with the desktop app not verified) |
| Group analytics (list, members, ranking, hours, media, export) | Covered |
| Insight (test, records, trigger, footprint) | Covered, except `getTodayStats` |
| SNS / Moments | Mostly covered; `downloadEmoji` missing; export stats are thin wrappers |
| Analytics | Overall, rankings, time, excluded read. **Missing:** exclude candidates, set excluded usernames |
| Chat | 16 of 40 applicable channels full, 3 partial, **21 missing** (see below) |
| Annual / dual report | **Partial.** Returns raw WCDB aggregates; the TS services (1.6k + 0.8k lines) build the full report |
| Export | **Partial.** Messages export is TXT only. `export sessions --format html\|excel\|sql\|chatlab\|weclone` formats the *session list*, not messages |
| Images / video / voice | Bulk export of decrypted images and voice files only. **Missing:** video lookup (`video:*`), per-message image/voice fetch, voice transcription (`whisper:*`), hard-link resolution |
| HTTP API | Health, sessions, messages, contacts, groups, SNS timeline/users/stats, SSE events. **Missing:** `access_token` auth, `/api/v1/media/*`, `/api/v1/sns/media/proxy`, `sns/export`, `sns/block-delete/*`, `sns/post/*`, `group-members` path, `push/messages` path (Rust uses `/api/v1/events`) |

Not ported: message/contact/session caches, avatar cache, cloud control, export task pause/resume, Weibo cookie helpers.

### Missing IPC channels (28)

`analytics`: getExcludeCandidates, setExcludedUsernames ·
`chat`: clearCurrentAccountData, getAllImageMessages, getAllVoiceMessages, getAntiRevokeSessions, getExportSessionStats,
getExportTabCounts, getGroupMyMessageCountHint, getMediaStream, getMessage, getMessageDateCounts, getMessageDates,
getResourceMessages, getSessionDetail, getSessionDetailExtra, getSessionDetailFast, getSessionMessageCounts,
getSessionStatuses, getVoiceTranscript, markAllSessionsRead, resolveTransferDisplayNames, resolveVoiceCache ·
`insight`: getTodayStats · `sns`: downloadEmoji · `video`: getVideoInfo, parseVideoMd5 · `whisper`: downloadModel

### Partial (15)

annualReport:generateReport, dualReport:generateReport, chat:getNewMessages (only via `serve --message-push`),
chat:getImageData, chat:getVoiceData, export:exportSession, export:exportSessions, export:getExportStats,
image:resolveCache, image:startAutoDownload, image:stopAutoDownload, sns:getExportStatsFast, sns:getUserPostCounts,
sns:getUserPostStats, sns:proxyImage

### Missing WCDB functions (51)

Message cursors (`open/close/fetch_message_cursor*`), single-message lookup (`get_message_by_id`, `_by_svrid`,
`get_message_count`, `get_messages_by_type`, `get_message_meta`), table/schema introspection (`list_tables`,
`get_message_tables`, `get_table_schema`, `list_message_dbs`, `list_media_dbs`, …), batch stats, voice data, image/video
hard-link resolution, emoticon captions/CDN URLs, avatars/display names, table snapshot import/export, cloud and monitor-pipe
functions.

## Security note

The Rust HTTP server has no `access_token` check, unlike the TypeScript one. It binds to `127.0.0.1` by default; do not
expose it with `--host 0.0.0.0` until token auth is ported.

## Reproducing the numbers

- IPC: `grep -oE "ipcMain\.handle\('[^']+'" electron/main.ts` (172), classified by hand as above.
- WCDB: `lib.func('…wcdb_*(` declarations in `electron/services/wcdbCore.ts` (90) vs `b"wcdb_*` symbols in
  `crates/weflow-native/src/wcdb.rs` (39, all also present in the TS list).
- Export formats: `format:` union in `electron/services/exportService.ts` vs `export_txt` being the only message exporter in
  `crates/weflow-core/src/services.rs`.
