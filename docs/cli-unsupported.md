# What the native CLI does not support

**English** | [简体中文](zh-CN/cli-unsupported.md)

Everything `weflow` cannot do, does not do yet, or does differently from the desktop app, compared with the original TypeScript
backend (`ca6c479`). What *is* covered, with numbers, is in [cli-coverage.md](cli-coverage.md).

The database layer is pure Rust: it decrypts WeChat 4.x databases itself and opens them **read-only** (nothing is written to
WeChat's files, nothing plaintext is written to disk). The closed-source `wcdb_api` library is no longer used, embedded or loaded.
Sections 1 and 2 list every database-level function that does not work; a test keeps them in sync with the code (see
[section 6](#6-keeping-this-list-current)).

## 1. Refused by design: anything that would modify WeChat's databases

These commands exist for compatibility but always fail, with the message
`<function> is not supported: the native database backend opens WeChat's databases read-only`.

| CLI command | Native function | What it did in the desktop app |
|---|---|---|
| `chat update-message` | `update_message` | Edit the text of a stored message |
| `chat delete-message` | `delete_message` | Delete a stored message |
| `chat anti-revoke check` / `install` / `uninstall` | `anti_revoke_check`, `anti_revoke_install`, `anti_revoke_uninstall` | Query / install / remove a database trigger that keeps revoked messages (`check` only reads, but is refused together with the other two) |
| `chat mark-read` | `mark_all_sessions_read` | Clear the unread counters of all sessions |
| `sns block-delete` (check / install / uninstall) | `sns_block_delete_check`, `sns_block_delete_install`, `sns_block_delete_uninstall` | Trigger that keeps Moments that friends delete |
| `sns delete` | `sns_delete_post` | Delete a Moments post from the local database |
| (no command) | `import_table_snapshot`, `import_table_snapshot_with_schema` | Restore a table dump into a database (the dump side works and writes only outside `db_storage`) |

Why: writing into a database that the running WeChat client also has open can corrupt it, and the SQLCipher files would have to be
re-encrypted page by page. If you need one of these, it has to be designed separately (WeChat closed, backup first).

## 2. Not implemented in the native database layer

None at the moment: every database function the service layer can call is either implemented or refused by design (section 1).
A function that is added to `crates/weflow-native/src/wcdb.rs` before it is ported answers
`<function> is not implemented in the native database backend yet`; list it here until it is ported.

## 3. Desktop features that are missing or different

| Area | Difference |
|---|---|
| Voice-to-text (`chat:getVoiceTranscript`, `whisper:downloadModel`, `whisper:getModelStatus`) | Missing: needs sherpa-onnx and Whisper models; not planned. |
| Live updates (message push, insight triggers, `chat:getNewMessages`) | The desktop app reacts to WCDB monitor callbacks; the CLI polls (push and insights about every 5 s). |
| Message export | All 9 formats work; `--media` copies media into the CLI's own folder layout (section 5). |
| Voice in the HTTP API / `chat voice-data` | Works only if the media database holds the SILK data (WeChat must have played the message once). |
| WXGF images | Converted through an external `ffmpeg` (`PATH` or `FFMPEG_PATH`); the desktop app bundles `ffmpeg-static`. Without it the image is reported as a failed decrypt. |
| Image auto-download (`image auto-download`, `serve --image-auto-download`) | Windows x64 only (`img_helper.dll`). The hook lives only while the `weflow` process runs, so `status` from another process always says "not hooked". |
| Image service events | `image:cacheResolved`, `decryptProgress`, `updateAvailable` and the background "better quality available" check are not emitted; `hasUpdate` is always `false` (like the desktop app's headless worker mode). |
| AI insight notifications | No popup window; `serve --insight` prints each insight as a JSON line on stderr (Telegram push still works). |
| Image key memory scan | `key scan-image` works on macOS only; on Windows use `key image` (kvcomm cache + template verification). The desktop app's Windows memory-scan fallback is not ported. |
| Video | Looks up the file WeChat already stored under `msg/video`; there is no download or decrypt path (the desktop app has none either). |

Deliberately not ported because they only make sense in the desktop process: window/dialog/shell/app/auth/log IPC, auto update,
autostart, app lock, cloud control, diagnostics, social-cookie UI helpers, message/contact/session/avatar caches, export task
pause/resume, renderer-only report screenshots, the Moments cache-migration UI.

## 4. Platform and verification limits

- The native database layer (session, message, contact, Moments, media and voice databases) was verified against one real Windows
  WeChat 4.x account with a **Linux build**, and the cross-compiled Windows `weflow.exe` (`x86_64-pc-windows-gnu`) was run on
  Windows against the same account (about 80 commands, message exports with media, image exports). Other Windows versions have
  not been tried.
- macOS and Linux WeChat databases use the same file format but have not been tested.
- The HTTP server, image `.dat` decryption, AI and Moments downloads were also checked against synthetic encrypted fixtures and
  local fake HTTP servers; Moments servers and AI providers have not been tried for real.
- The key extraction helpers (`key db`, `key image`) and the Windows image hook need a running WeChat and cannot be tested offline.
- Only one account's data was used; unusual databases (very large shards, old schema versions) may expose gaps.
- Still to verify (macOS/Linux real accounts, the Windows image auto-download hook, AI insight with a real provider, backup
  compatibility with the desktop app): see [plan.md](plan.md#verification-still-to-do).

## 5. Behaviour you may not expect

- **Snapshots**: each database is decrypted into memory the first time a command touches it (several hundred MB for a large message
  shard; the cache is capped at about 1.5 GB). A long-running `serve` picks up new messages when a file or its `-wal` changes.
- **Memory of big exports**: `export messages` turns the conversation into export records page by page and writes the file from
  them. For a 200,000-message group the peak is about 0.6 GB (`txt`, `weclone`, `sql`, `excel`, `chatlab`) to 0.8 GB (`json`,
  `arkme-json`, `html`); about 200 MB of that is the decrypted message database. Export a date range (`--start/--end`) if memory
  is tight; the cost follows the range, not its age.
- **Time zones**: the `--start/--end` dates of the exports, the times written into them, `chat dates`, `chat date-counts` and the per-day
  statistics all use the machine's local time zone (like the desktop app). The same database read on a machine in another zone
  puts a late-night message on a different day.
- **Media in message exports**: `export messages --media image,voice,video,emoji` (or `all`) copies the files to
  `media/<output file name>/{images,voices,videos,emojis}` beside the output file and points the messages at them. Which formats have a
  place for them: `json`/`arkme-json`, `txt`, `excel` and `weclone` (the content or `src` becomes the relative path), `chatlab` (images
  only), `html` (`<img>`, `<audio>`, `<video>`); `sql` has none. A file that is not on disk keeps its placeholder text. Stickers need
  network access; the desktop app uses its own folder layout.
- **Ordering of Chinese names** (contact list, group members) follows ICU/CLDR pinyin collation like `Intl.Collator('zh-CN')`:
  digits first, then Han characters by pinyin, then Latin letters.
- **Group text**: exported group messages keep the line break that follows the sender prefix (`wxid_xxx:` is removed, the newline stays),
  as in the desktop app's exporter.
- **Search** looks at text, link/file and quote-reply messages; the keyword is matched literally (case-insensitive), compressed
  messages are decoded before matching. Images, voice and stickers are not searched.
- **Folded / muted** session state is derived from contact flag bits (folded: bit 28; muted: bit 9 or the group notify flag) following
  WeChat's conventions; it has not been verified against a folded chat.
- **Footprint** counts `@all` (`notify@all`) as a mention of you, and a mention needs both an `@` in the text and your id (or
  `notify@all`) in the message's `atuserlist`. A private chat is split into segments after an hour of silence.
- **Dual report** phrases are exact-match texts of 2-20 characters (no links or markup) typed at least twice; response times measure
  only your replies within one conversation (gaps over an hour start a new one).
- **Moments annual statistics** count your own posts, the friends who liked them most, and the friends whose posts you liked most.
- Write operations (section 1) are refused, but read-only commands never change WeChat's data.

## 6. Keeping this list current

`cargo test -p weflow-native --test unsupported_docs` fails when a database function that returns *not implemented* or *not supported*
is missing from sections 1-2 of this file or of [zh-CN/cli-unsupported.md](zh-CN/cli-unsupported.md). When you port a function, remove its
row in the same change; when you add a limitation, add a row.
