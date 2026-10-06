# What the native CLI does not support

**English** | [简体中文](zh-CN/cli-unsupported.md)

Everything `./weflow` cannot do, does not do yet, or does differently from the desktop app, compared with the original TypeScript
backend ([ca6c479](https://github.com/334456777/WeFlow/tree/ca6c479496d4c7f00ccf234d567b1c51c79fe170)). What *is* covered, with numbers, is in [cli-coverage.md](cli-coverage.md).

## 1. Unimplemented placeholders

These commands are retained for compatibility, but because the native database layer opens WeChat's databases read-only, they always fail with the message
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
re-encrypted page by page. If you need one of these, it has to be designed separately.

## 2. Not implemented in the native database layer

None at the moment: every database function the service layer can call is either implemented or refused by design (section 1).
A function that is added to `crates/weflow-native/src/wcdb.rs` before it is ported answers
`<function> is not implemented in the native database backend yet`; list it here until it is ported.

## 3. Desktop features that are missing or different

| Area | Difference |
|---|---|
| Voice-to-text (`chat:getVoiceTranscript`, `whisper:downloadModel`, `whisper:getModelStatus`) | Missing: needs sherpa-onnx and Whisper models; ~~not planned~~. |
| Live updates (message push, insight triggers, `chat:getNewMessages`) | The desktop app reacts to WCDB monitor callbacks; the CLI polls (push and insights about every 5 s). |
| Message export | All 9 formats work; `--media` copies media into the CLI's own folder layout (section 5). |
| Voice in the HTTP API / `chat voice-data` | Works only if the media database holds the SILK data (WeChat must have played the message once). |
| WXGF images | Decoded by the CLI itself (the desktop app converts them with its bundled ffmpeg), into a JPEG that is about a quarter larger than ffmpeg's for the same fidelity. A picture its decoder cannot read (10-bit, 4:2:2 or 4:4:4; none of about 1,800 on a real account) goes through an external `ffmpeg` (`FFMPEG_PATH`, `PATH`, or the copy `ffmpeg install` downloads: the same `ffmpeg-static` build the desktop app bundles, checked against its SHA-256). The CLI does not ship it and never downloads it on its own. Without it such an image fails with `failure_kind` `ffmpeg_missing` and a message that points to `weflow ffmpeg install`; an export with media counts these images once in `ffmpegMissing` and says so in a `hint` (a `FFMPEG_PATH` that cannot be started is reported as such). |
| Image auto-download (`image auto-download`, `serve --image-auto-download`) | Windows x64 only (`img_helper.dll`). The hook lives only while the `./weflow` process runs, so `status` from another process always says "not hooked". |
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

## 5. Behaviour you may not expect

- **Snapshots**: databases are decrypted page by page as queries read them, and SQLite keeps the pages it has read (a command
  that walks a whole message database can hold all of it, a few hundred MB; together the caches give memory back above about
  1 GB). A long-running `serve` picks up new messages when a file or its `-wal` changes. If WeChat writes a checkpoint past the
  snapshot while a query reads it, the query runs again on a fresh snapshot. `export messages` lists the messages it exports
  when it starts: messages WeChat writes while it runs are left for the next export.
- **Memory of big exports**: `export messages` turns the conversation into export records page by page and writes the file from
  them, reading the database with a small page cache per reading thread. For a 200,000-message group of a real account, with the
  threads an export picks on its own (16 logical CPUs), the peak is about 0.2 GB (`json`, `arkme-json`), 0.2–0.3 GB (`txt`,
  `sql`, `html`, `excel`) and 0.25–0.4 GB (`chatlab`, `chatlab-jsonl`, `weclone`); the upper end is the resident set on Linux,
  the lower one the working set on Windows. An export with media (`--media`) reads the whole conversation into memory before
  it copies the media and writes the file: about 0.5 GB on Windows for the same group with images, voices and videos, and
  about 0.8 GB the first time, while its WXGF images are decoded into the image cache (several at once). Export a date range (`--start/--end`) if memory is tight; the cost follows the range, not its age.
- **Threads of big exports**: several threads read and parse the pages while the file is written. An export starts with two
  and adds one, up to one per CPU but one (at most 8), while writing the file keeps waiting for pages, and stops adding once
  one more thread no longer reads clearly faster (by at least half of what it could add). `chatlab` and `chatlab-jsonl`
  entries are also rendered on the reading threads, so these exports and `weclone` use the most threads (5–7 on 16 logical
  CPUs); `txt`, `sql`, `html` and `excel` reach about 3, and `json` / `arkme-json` stay at 2 (writing is the bottleneck).
  An export with media reads with at most 2 threads, since it only collects the messages. Every reading thread keeps its
  own page cache and a few pages in flight, about 20–60 MiB each, and the peaks above include them.
  `WEFLOW_EXPORT_WORKERS=1` reads with one thread (about 0.13 GB, but slower); any other number fixes the thread count.
  `RUST_LOG=weflow::export=debug` logs the threads used and the time spent reading, parsing (and rendering) and waiting
  (`=trace` also logs each decision to add a thread). With nothing cached (the first export after a restart) the same
  exports took 2–25% longer from an NVMe SSD, and the extra threads saved as much time as with a warm cache.
- **Memory on Windows at startup**: the CLI reserves and commits 128 MiB for its allocator when it starts, which saves exports
  hundreds of thousands of page faults. It counts against the commit limit (private bytes) of even a short command, not against
  physical memory (working set). `MIMALLOC_RESERVE_OS_MEMORY` sets another size.
- **Key check**: the first command with a key proves it on `session.db` and remembers a one-way fingerprint of it (key,
  database salt and account; the key cannot be recovered from it) in the cache folder, so later commands skip that slow step.
  `chat clear-account-data --cache` deletes the fingerprints; `db test` always checks the key.
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

## 6. Keeping this list current (for LLMs)

`cargo test -p weflow-native --test unsupported_docs` fails when a database function that returns *not implemented* or *not supported*
is missing from sections 1-2 of this file or of [zh-CN/cli-unsupported.md](zh-CN/cli-unsupported.md). When you port a function, remove its
row in the same change; when you add a limitation, add a row.
