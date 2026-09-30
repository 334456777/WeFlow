# What the native CLI does not support

**English** | [简体中文](zh-CN/cli-unsupported.md)

This is the detailed list of what `weflow` cannot do, cannot do yet, or does differently from the desktop app. It complements
[cli-gaps.md](cli-gaps.md) (comparison with the original TypeScript backend) and [cli-coverage.md](cli-coverage.md).

The database layer is pure Rust: it decrypts WeChat 4.x databases itself and opens them **read-only** (nothing is written to
WeChat's files, nothing plaintext is written to disk). Sections 1 and 2 list every database-level function that does not work;
a test keeps them in sync with the code (see [section 6](#6-keeping-this-list-current)).

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

Why: writing into a database that the running WeChat client also has open can corrupt it, and the SQLCipher files would have to be
re-encrypted page by page. If you need one of these, it has to be designed separately (WeChat closed, backup first).

## 2. Not implemented in the native database layer

Calling one of these returns `<function> is not implemented in the native database backend yet`. **No CLI command calls them
today**, so you will not see the error in normal use; if you do, it is a bug worth reporting.

| Function | Purpose | Status |
|---|---|---|
| `annual_report_stats`, `annual_report_extras` | Native shortcuts for the annual report | Intentionally unimplemented: `report annual` computes the same numbers from message cursors (the service layer treats the error as "compute it yourself") |
| `message_meta` | Lightweight id/time/type rows of a message table | Not needed by any command |
| `db_status` | Open/readable state of every database | Not needed |
| `media_schema_summary` | Column summary of a media database | Not needed |
| `message_table_columns`, `list_tables`, `table_schema` | Raw schema browsing | Not needed; use `db scan` / `chat detail` |
| `export_table_snapshot`, `import_table_snapshot`, `import_table_snapshot_with_schema` | Raw table dump / restore for the old backup tooling | Export could be added; import writes (see section 1). `backup` works on files instead |
| `head_image_buffers` | Avatar image bytes from `head_image.db` | Not needed; avatars come from contact URLs |
| `resolve_image_hardlink`, `resolve_image_hardlink_batch`, `resolve_video_hardlink_md5`, `resolve_video_hardlink_md5_batch` | Find image/video files through `hardlink.db` | Not needed; media files are located by the CLI's own path logic |

## 3. Features of the desktop app the CLI does not have

Details and reasons are in [cli-gaps.md](cli-gaps.md). In short:

- Speech-to-text of voice messages and the Whisper model download/status (needs sherpa-onnx + Whisper models).
- "Clear current account data" and "clear all caches" (the CLI has no long-lived cache; only `analytics clear-cache` and `image clear-cache` exist).
- `sns:debugResource` (Moments resource debug dump).
- Contact labels, signature and region (needs the extended-column parser and the region table).
- Message exports do **not** embed media files; export media separately (`export media`).
- WXGF images need an external `ffmpeg` (`PATH` or `FFMPEG_PATH`).
- Image auto-download hook (`image auto-download`, `serve --image-auto-download`) works on Windows x64 only and only while the process runs.
- `key scan-image` (AES key memory scan) works on macOS only; on Windows use `key image`.
- Live updates use polling (message push and insights about every 5 s) instead of WCDB monitor callbacks.
- No popup windows, auto update, autostart, app lock, cloud control or other desktop-process features.

## 4. Platform and verification limits

- The native database layer was verified against a real Windows WeChat 4.x account (session, message, contact, Moments, media and
  voice databases) using a **Linux build**. The Windows `weflow.exe` is cross-compiled and has not been run on Windows yet.
- macOS and Linux WeChat databases use the same file format but have not been tested.
- The key extraction helpers (`key db`, `key image`) need a running WeChat and cannot be tested offline.
- Only one account's data was used for verification; unusual databases (very large shards, old schema versions) may expose gaps.

## 5. Behaviour you may not expect

- **Snapshots**: each database is decrypted into memory the first time a command touches it (several hundred MB for a large message
  shard; the cache is capped at about 1.5 GB). A long-running `serve` picks up new messages when a file or its `-wal` changes.
- **Memory of big exports**: `export messages` holds the whole conversation in memory while it builds the file. For a 200,000-message
  group the peak was about 1.3 GB (`txt`) to 1.8 GB (`json`, `excel`, `chatlab`); opening the message database alone takes about 200 MB.
  Export a date range (`--start/--end`) if memory is tight; the cost follows the range, not its age.
- **Time zones**: `export ... --start/--end` take Beijing dates (UTC+8), while `chat dates`, `chat date-counts` and the per-day
  statistics use the machine's local time, so on a machine outside UTC+8 the two can differ by a few messages at the day edges.
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
