# Native CLI (`weflow`)

**English** | [简体中文](zh-CN/native-cli.md)

A Rust command-line build of WeFlow's backend. By default a command prints human-readable text on stdout (aligned `key: value` lines, tables for lists) and
errors on stderr, with the same exit codes. With `--json` every command prints one JSON document on stdout
(`{"success": true, "data": ...}` or `{"success": false, "error": {...}}`); progress goes to stderr with `--progress`.
Argument errors follow the same rule: with `--json` (before any `--`) a mistyped option or a missing subcommand prints
`{"success": false, "error": {"code": "invalid_arguments", "message": ..., "details": {"usage": ...}}}` on stdout and exits with `2`.
A command whose operation fails (an image that cannot be found, `insight test` without an API key, ...) is an error too:
`success` is `false` with a non-zero exit code, and fields such as `failureKind` are kept in `error.details`; `success: true` always means it worked.
The HTTP API keeps its own response shapes.

Help: `-h` / `--help` shows the help of any command, and a command that needs arguments or a subcommand shows its help when it is run without any (`./weflow config`, `./weflow config set`, `./weflow lang`);
with only some of the arguments it reports which are missing. Usage lines list `[OPTIONS]` last (`./weflow config set <KEY> <VALUE> [OPTIONS]`).

## Language

The language follows the system (Chinese on a Chinese system, English otherwise). In order of precedence: `WEFLOW_LANG`, the language saved in the config file (`./weflow lang zh` saves it; `./weflow config unset lang` removes it), the environment
(the first variable that is set and non-empty decides), then the operating system's display language:

`WEFLOW_LANG` → `LC_ALL` → `LC_MESSAGES` → `LANG` → `LANGUAGE`

A value starting with `zh` (`zh_CN.UTF-8`, `zh-TW`, `zh`) gives Chinese; anything else, including `C` and `POSIX`, gives English.
When none of the variables is set (usual on Windows) the OS display language decides (Windows, macOS); if it cannot be determined, English.
`WEFLOW_LANG=en|zh` overrides the saved language for a single run or shell session.

The language affects `--help`, usage errors, runtime errors, progress text and generated text: TXT/Excel export labels (`[Image]` / `[图片]`), the default official-account payment
merchant name, and the default AI insight prompt. JSON keys, error codes and the HTTP API's error responses stay English.

## Commands

```
./weflow config    path | list | get | set | unset | clear | import
./weflow lang      en | zh
./weflow db        detect | scan <root> | wxid | test | open
./weflow key       db | image | scan-image <user-dir>
./weflow chat      sessions | messages | latest | search | contacts | contact | update-message | delete-message
                 anti-revoke | message | dates | date-counts | counts | statuses | detail | mark-read | tab-counts
                 export-stats | group-hint | resources | images | voice-messages | media-stream | transfer-names
                 voice | voice-data | voice-cache | voice-preload | image-data | image-info | video-data | emoji
./weflow export    sessions | contacts | footprint | media | messages   (messages: chatlab, chatlab-jsonl, json,
                 arkme-json, html, txt, excel, weclone, sql)
./weflow analytics overall | rankings | time | excluded | exclude-candidates
./weflow group     list | members | ranking | hours | media | member | member-messages | export-member-messages | export-members
./weflow report    annual years|generate | dual generate
./weflow sns       timeline | users | stats | post-counts | export | media | download-emoji | download-image | debug-resource
                 block-delete | delete
./weflow biz       accounts | messages | pay-records
./weflow insight   test | trigger | records | get | mark-read | clear | today-stats | scan | footprint | footprint-summary
./weflow video     info | parse-md5
./weflow image     decrypt | resolve-cache | resolve-batch | auto-download start|status
./weflow backup    create | inspect | restore
./weflow serve     [--desktop [--message-push]] --insight --image-auto-download
./weflow runtime   info | manifest
./weflow cache     list | clear | clear-account
./weflow ffmpeg    install | path | set baseurl <url> | unset baseurl
```

The database layer is native Rust and **read-only**: `chat update-message`, `chat delete-message`, `chat anti-revoke`, `chat mark-read`, `sns block-delete` and `sns delete` would modify WeChat's databases and are always refused (see [cli-unsupported.md](cli-unsupported.md) for these and everything else that is not supported). Such a refusal has the error code `unsupported` (exit code `4`). Exit code `4` is also used when a database cannot be opened (wrong key, unreadable file), with the error code `native_error`.

Message ids: a message's local id (`local_id` / `localId`) is only unique within one `message_N.db`, and a conversation spread over several of them can have different messages with the same local id. Commands that take a local id (`chat message --local-id`, `chat image-data`, `chat voice-data`, `chat video-data`) also take `--create-time` (seconds, the message's `create_time`) to tell them apart; `chat image-data` and `chat voice-data` also prefer the message of their own kind. When a local id still names several messages, `chat message` and `chat image-data` answer `invalid_arguments` with the candidates (`createTime`, `localType`, `dbName`) in `details.candidates` instead of picking one (`chat voice-data` then needs `--create-time`). The server id (`chat message --server-id`) is unique.

Progress: commands that run longer than the delay (default 5 seconds; `./weflow config set progress_delay_seconds <s>` or env `WEFLOW_PROGRESS_DELAY`, `0` = always) show a single-line progress bar on stderr (only when stderr is a
terminal; stdout is never touched). `./weflow config set no_progress true` turns it off, `--progress` prints machine-readable NDJSON events instead.

`export media --type image|voice|video|emoji|all [--session <id>] [--start YYYY-MM-DD --end YYYY-MM-DD]` walks the media
messages (an image sent twice counts twice, so `found` can exceed the unique files listed by `chat images`). `missing` counts
messages whose file is not on disk (never downloaded in WeChat) or could not be resolved, per kind in `missingByKind`.
`thumbOnly` counts exported images that are only the thumbnail (each image entry also has `isThumb`); open the original in WeChat and export again to get the HD file. `export media` always prefers the HD original (like `image decrypt --force`). Stickers may need network access; voice export decodes every message, so a full export of hundreds of voices takes minutes.

`export messages` selects messages by their real timestamps in the database and shows a progress bar; `--start/--end` are dates in the machine's local time zone. With `--start` or `--end` the keys are read from the message rows, because an unusual `sort_seq` would make the faster index-only keys miss a message inside the range; so a short range of a large conversation takes about as long to prepare as the whole conversation. `--media image,voice,video,emoji` (or `all`) copies the media into `media/<output name>/` beside the output file and points the messages at the copies (for `json`, `arkme-json`, `txt`, `excel`, `weclone`, `html`; `chatlab` for images; not `sql`). Every format is written while the messages are read (`json`, `chatlab`, `html` and `excel` put the messages in a temporary `<output>.part` file next to the output first, because their header needs the totals; an export with `--media` reads the whole conversation first), so a 200,000-message group needs about 0.15–0.45 GB depending on the format (with `--media` about 0.6 GB, 1 GB the first time). `--sender` keeps one person's messages in every format (it takes the bare wxid, as `chat contacts` lists it: the one identifier every message has, unique and fixed; the account folder name with its `_xxxx` suffix is refused with the bare form to use, and an export that matches nothing says what `--sender` takes); in every format senders are named by group nickname, then remark, nickname, wxid (`--display-name group-nickname`, the default); `--display-name remark` skips the group nickname and `--display-name nickname` uses the nickname only.

Message text in `export messages`: every format writes the same text for a message, so the formats can be compared line by line.

| Message | Text |
|---|---|
| text | the text, with the `wxid:` line WeChat prepends in groups removed (a body such as `4:1` is kept) |
| image / voice / video | `[图片]` / `[语音消息]` / `[视频]` (a media file path when `--media` copied the file) |
| sticker | `[表情]`, or `[表情：<caption>]` when the sticker library has a caption for it (your own caption first, else the store caption) |
| system message | `[系统: <text>]` |
| link card | `[链接] <title>` and the URL on the next line |
| transfer | the amount with who paid whom: `[转账] (A 转账给 B) ¥66.00` |
| reply to a message | `<reply>[引用 <name>：<quoted text>]`; a quoted file, note, link or another reply shows as a short label or its title, never as XML |
| forwarded chat history | `[转发的聊天记录]` followed by the messages |
| WeChat note | `[笔记]` and the full note text (images as `[图片]`) |

Some formats keep their own rules: `chatlab` writes links as `[title](URL)` and leaves system messages unwrapped (it has a `type` field for them), `html` draws link cards and system messages itself, and `weclone` leaves out replies. `txt` is `<time> '<sender>'` on one line and the text under it; names follow `--display-name`. `arkme-json` copies WeChat's own fields (`source`, `appMsgDesc`, ...) unchanged.

Every format is written while the messages are read, in time order (a conversation that comes back out of order would be written in the order read). A format whose header needs totals first writes its messages to `<output>.part` and joins them when the read ends; the file is removed afterwards, also on failure, and an empty range writes no file.

All cache cleanup is under `weflow cache`:

| Command | What it does |
|---|---|
| `cache list` | Each part of the cache with the option that clears it, its size and its path |
| `cache clear --images` | Decrypted images |
| `cache clear --voices` | Decoded voice messages |
| `cache clear --emojis` | Stickers |
| `cache clear --sns` | Images and videos of Moments |
| `cache clear --analytics` | The analytics aggregate |
| `cache clear --api` | Media exported (`api-media`) and avatars pushed (`push-avatar-files`) by the HTTP API, and files returned by `serve` (`serve-files`, see [serve-api.md](serve-api.md#files)) |
| `cache clear --keys` | Fingerprints of keys that worked; the next command checks the key again |
| `cache clear --runtime` | Runtimes unpacked by other WeFlow versions under `runtime/`; the running version's stays |
| `cache clear --web` | WebUI versions downloaded by `weflow web` and `serve` (`cache/web/`, see [serve-api.md](serve-api.md#webui)); `serve` downloads the one it needs again |
| `cache clear --all` | All of the above |
| `cache clear-account [--exports-dir <dir>]` | The current account's cached images, voices, stickers, Moments, analytics and key fingerprints; then signs the account out of the profile (`db_path`, `wxid`, `decrypt_key` and the image keys are removed). `--exports-dir` (repeatable) also removes the entries named after the account in that folder |

Options of `cache clear` combine (`cache clear --images --voices`). Before removing anything, `cache clear` and
`cache clear-account` list the paths with their sizes and ask `[y/N]`: only `y` or `yes` goes ahead, and Enter or any other
answer cancels. `-y`/`--yes` skips the question; without a terminal (a pipe or a script) it is required. `--dry-run` only lists
what would go, with the sizes (and for `clear-account` the profile keys it would remove), and removes nothing: a page can show it
before it asks. Nothing in WeChat's own folders is touched.

`db detect` prints the WeChat data directories that exist as `db_path: <path>`, and `db wxid` prints the wxid of the
account(s) it finds as `wxid: <wxid>` (the folder name without its `_ab12` suffix; add a data directory as an optional argument to read a specific one); both names are the ones `config set` takes.

`key db` (Windows) hooks WeChat through `wx_key.dll`. WeChat only produces the key while it opens its databases, so the command
asks you to quit WeChat completely (when it is running) and open it again, checks once a second for the process (the
whole procedure gives up after `--timeout`, default 180 s, and shows the seconds left), hooks the new process and asks you to
click "Enter WeChat" in the login window. It prints `decrypt_key: <key>`, the name `config set` expects. It needs an
administrator terminal, looks for `Weixin.exe` then `WeChat.exe`, and with `--pid` it hooks that process directly instead of
waiting for a restart. `key image` derives the image keys from the `kvcomm` cache, verifies them against a `_t.dat` template
under the account directory, and prints `image_xor_key` and `image_aes_key`. With `--save`, both commands write the keys
to the active profile instead of printing them: the result says `saved` (the names of the keys written) and leaves the
keys out, which is how the WebUI's setup gets them through `serve` without the keys reaching the browser.

`key image` (Windows only; Linux and macOS report that it is not supported yet) reads the `kvcomm` cache files directly, without
any DLL or process access (the Windows entry point of #89):

```powershell
./weflow --json key image
```

Without `--kvcomm-dir` it uses the existing `%APPDATA%\Tencent\xwechat\net*\kvcomm` directories (`net`, `net_1`, ...);
pass `--kvcomm-dir` (repeatable) to use other directories.
`--user-dir <account directory>` overrides the configured sample directory. Only the configured wxid and
that directory's name are considered, normalized and deduplicated; sibling accounts are not searched.
Stored image keys are ignored and nothing is written to the configuration.

`key_` filenames yield ASCII decimal underscore-delimited tokens in `1..=4294967295` (an optional
`.statistic` suffix is removed). These are **loose candidates**: unrelated numeric fields must pass sample
verification. Codes and derived pairs are deduplicated; `sources` retains filenames marked
`loose_decimal_tokens`. Multiple matching pairs produce `image_key_ambiguous`, rather than selecting the first; if exactly one of them is fully verified, that one is returned.

`--scan-budget` defaults to 10,000 filesystem entries, including directories. Traversal is deterministic and
does not follow symlinks; the newest 32 structurally valid V2 `_t.dat` files within that budget are retained.
JSON `scan` reports `entries_scanned`, `valid_templates`, `invalid_format`, `damaged_templates`, `read_errors`,
`truncated` (traversal budget), and `templates_truncated` (retention limit). It does not claim globally newest
samples when traversal is truncated. `collection.errors` preserves partial directory/entry read failures.
Failures distinguish `image_key_directory_unreadable`, `image_key_no_candidates`, `image_key_no_template`,
`image_key_verification_failed`, and `image_key_ambiguous`, with collection/scan diagnostics.

`aes_verified` means an AES first block matched an image header. `xor_verified` and the overall `verified`
become true only when a matching JPEG sample fully decrypts and decodes, and its JPEG EOI is in the XOR
tail. Otherwise the unique AES-matching pair is returned as `verification: aes_header_only`, with XOR
and pair verification false; do not treat it as a verified pair. Full sample reads are limited to 16 MiB and
JPEG decoding to a 64 MiB pixel buffer. Non-JPEG samples or samples without an XOR tail currently provide
only AES evidence. No configuration is written. Linux/macOS directory discovery and real accounts, and a
fresh-key Windows real-client decode, remain separate manual validation requirements.

Paths: configuration `%APPDATA%\weflow\config.json`, extracted runtime `%APPDATA%\weflow\runtime\<version>\<target>`
(on Linux/macOS under the platform's data directory).

`serve` offers every command over HTTP on `127.0.0.1:18108` ([serve-api.md](serve-api.md)). `serve --desktop` exposes the desktop app's HTTP API instead ([HTTP-API.md](HTTP-API.md); token required except `/health`; set `http_api_token` or `--api-token`).
`serve --insight` runs the AI insight engine; each generated insight is printed to stderr (as a JSON line with `--json`).
`image auto-download` and `serve --image-auto-download` hook WeChat through `img_helper.dll` and only work on Windows x64.
Voice messages are decoded with a vendored copy of the Skype SILK SDK (`crates/weflow-silk`). WXGF images (an HEVC picture in
WeChat's container) are decoded in Rust (`heic-rs`, then written as a JPEG); only a picture it cannot read (10-bit, 4:2:2 or
4:4:4) goes to `ffmpeg`: `FFMPEG_PATH`, else `ffmpeg` on `PATH`, else the copy `ffmpeg install` put in WeFlow's folder
(`ffmpeg path` shows which one is used). `ffmpeg install` downloads the build the desktop app bundles (npm `ffmpeg-static` 5.3.0, release `b6.1.1` of
eugeneware/ffmpeg-static, GPL-3.0, with its license file), checks its SHA-256 and unpacks it to `ffmpeg/b6.1.1/` beside the
config file; it never runs on its own. An intact copy is not downloaded again (`--force` does); a new copy replaces the old
one only once it is complete and checked, and the folders of other releases under `ffmpeg/` are removed. `ffmpeg set baseurl <url>` makes it download from a mirror of those releases instead of
GitHub (for example `https://registry.npmmirror.com/-/binary/ffmpeg-static`; it is kept in the config file, `ffmpeg unset
baseurl` goes back to GitHub), and the files are checked the same way. The Windows build is x64 (Windows on Arm runs it
emulated). `ffmpeg path` shows whether one is found (see the [README](../README.md#check-ffmpeg-optional-step)): a WXGF image that needs
ffmpeg and finds none fails with `failure_kind` `ffmpeg_missing` and an error that names the time its message was sent, and
an export counts these images in `ffmpegMissing` and lists each one's conversation and time in `ffmpegMissingImages`.

Exit codes: `0` ok, `1` runtime error, `2` bad arguments, `3` config/key error, `4` database/native library error, `130` interrupted.

## Architecture

| Crate | Role |
|---|---|
| `crates/weflow-cli` | Command entry, argument parsing, output |
| `crates/weflow-core` | Configuration, accounts, chats, exports, analytics, Moments, backup, AI insights, HTTP API |
| `crates/weflow-native` | Native database reader (SQLCipher decryption, messages, contacts, Moments, statistics, reports), key helpers, image decryption, ISAAC-64 keystream (ported from the vendor WASM), platform wrappers |
| `crates/weflow-wcdb-ffi` | C-ABI shared library `weflow_wcdb` that exports `weflow-native` with the same interface as `wcdb_api`; loaded by the desktop app (see [desktop-rust-layer.md](desktop-rust-layer.md)) |
| `crates/weflow-wxkey` | Windows x64 `wx_key.dll` compatibility ABI, built from the Rust database-key capture code |
| `crates/weflow-assets` | Embedded resources, unpacking, hash check |
| `crates/weflow-silk` | Vendored SILK decoder for voice messages |

Windows x64 `key db` captures the database key directly in Rust, using a hardware execution breakpoint during login. It restores debug registers and detaches on success, timeout and Ctrl+C; there is no vendor DLL or method selector. The desktop app builds the compatible Rust `wx_key.dll` from the same code. See [Windows database keys](windows-db-key.md) for version limits and verification.

The remaining platform helpers (`img_helper.dll`, `libwx_key.dylib`, `xkey_helper_linux`) are embedded: each binary carries only its own platform's helpers, unpacks them into
`WEFLOW_HOME/runtime/<version>/<target>/`, checks the manifest hash on every start (unpacking again when the version or a hash
differs) and loads libraries only from that directory, never implicitly from the current directory.
The vendor WASM decoder (`WxIsaac64`) is not among them: it is ported to pure Rust in `weflow-core/src/isaac64.rs` and verified against vectors captured from the original module.

Configuration lives in `WEFLOW_HOME`, otherwise in `./weflow` under the platform's configuration directory: `config.json` (TOML is
accepted too), with caches, logs and the runtime in separate directories. Changes are written under a lock on `config.json.lock`
and through a temporary file, so several `weflow` processes can change the config at once without losing each other's changes. `./weflow config import` migrates the desktop app's
readable settings and skips the encrypted `safe:` / `lock:` values with a hint to set them again.

The connection settings (`db_path`, `wxid`, `decrypt_key`) are only read from the config file, set them with `./weflow config set`; there are no command-line
overrides. `./weflow config set --help` explains every key. `./weflow config set config_path <file>` makes later runs use another config file (it is remembered in
`config_path` next to the default location; `./weflow config unset config_path` or the default path switches back), `./weflow config path` shows the one in use.
`./weflow config set current_profile <name>` switches the active profile (created when it does not exist), and `./weflow config profiles` lists the profiles with their account and data directory, whether the database key, the image keys and the AI model API key are set (`true` / `false`, never the keys themselves), and whether the profile is `complete` (enough to open the database). `-h` / `--help` and `-V` / `-v` / `--version` work everywhere but are left out of the option lists.

**Why the database layer is pure Rust.** The CLI was first meant to call the closed-source `wcdb_api` library through FFI. That
library has an expiry check (after 2026-09-30 23:59:59 `wcdb_init` returns `-1000`) and unverified network code, so the CLI now
decrypts WeChat 4.x databases itself (SQLCipher 4) and reads them **read-only** in pure Rust
(`crates/weflow-native/src/{sqlcipher,native_*}.rs`). Neither the CLI nor the desktop app embeds or loads `wcdb_api`,
`WCDB.dll`, `libwcdb_api.*` or `libWCDB.dylib` any more; they stay in the repository for the original desktop app, see
[wcdb-api.md](wcdb-api.md). Coverage of the original backend: [cli-coverage.md](cli-coverage.md).

## Tests

- Unit tests in each crate: configuration and legacy config import, the embedded runtime manifest, SQLCipher (round trip, wrong
  key, tampered page, WAL merge), image and Moments decryption, export formats.
- End-to-end tests on synthetic encrypted accounts (`weflow_native::fixture`: SQLCipher pages, WAL, zstd, SILK) and local fake
  HTTP servers, under `crates/weflow-core/tests/`. `cargo test --workspace` runs everything.
- Real-data regression: one Windows WeChat 4.x account, with a Linux build and with the Windows `weflow.exe` run on Windows
  (about 80 commands, message exports with media, image exports, the HTTP API). What is still unverified is listed in
  [cli-unsupported.md](cli-unsupported.md#4-platform-and-verification-limits).

For repeatable cursor measurements, run `cargo run --release -p weflow-core --example export_pipeline_probe -- <work-dir> --mode keys --messages 200000 --shards 2 --runs 5`
with a disposable work directory: the probe replaces `<work-dir>/data` with synthetic encrypted databases. It warms the snapshots,
then reports each run's key preparation, first internal batch and batch fetch times for exact, index-derived and sender-filtered
cursors. Fetch times exclude the row-count and ordered SHA-256 checks; exact and index-derived rows must agree, and the filtered
rows must match the corresponding subset. Reuse the same work directory when comparing builds, because row digests include shard
paths. These are warm-snapshot measurements, not cold-disk timings or the time at which the final export file becomes readable.
Cursor keys still take memory proportional to the number of selected messages.

For local account regression, `cargo run -p weflow-core --example real_account_probe -- <isolated-config.json> <private-inventory.json> --compare-bounded`
reads the configured account without changing its databases and compares indexed and exact cursors around unusual timestamp keys.
Use a stable local database copy and separate configuration/cache folders when comparing builds. Keep the inventory, exports and detailed
logs private: they contain conversation identifiers, dates, messages or local paths. Only the probe's aggregate stdout is suitable for a
local summary; publishing account-derived statistics requires the account owner's authorization.

To benchmark the command API ([serve-api.md](serve-api.md)), build `cargo build --release -p weflow-cli --bin weflow --example serve_bench`
and run `target/release/examples/serve_bench <work-dir> --home <WEFLOW_HOME> --save base.json`. It always uses two accounts:

- A synthetic encrypted account checks the answers. It lives in `<work-dir>/data`, rebuilt only when `--messages`, `--sessions` or
  `--contacts` change; by default it has one group of 200,000 messages with images, all but the newest 50 of them in WeChat's search
  index, 500 private conversations, 3,000 contacts and 2,000 Moments posts. The
  reads a chat window makes (conversation list, first, middle and last page of messages, polling, one message, contacts, counts, search
  (also for a word found nowhere, which reads the whole index), group members, 16 pages at once, images) go through both the child-process routes and `/api/`. Both must give the same answer, and that
  answer must match what the account holds. Last, a message is added while `/api/push/messages` is listened to; it must arrive exactly once.
  Any failed check fails the run.
- The real account of `--home` measures the speed (use a copy of the configuration). First, once each: the server's start, the first
  read of the conversation list, and the first page of the conversation on top, as a page asks for them right after the start. Then the same reads go through both routes, `--runs`
  times each (default 10), and the median and 95th percentile are printed, followed by the server's CPU time per second while the push is
  listened to. The first page of messages and the contacts are also asked for compressed (gzip, br) and cut to the `fields` a page uses, to
  show what each saves. Videos are read as a player reads them: the first megabyte, then one from the middle. The same page is also asked for many times at once (up to 64), to show how the limits hold. The conversations are picked by message count, and only
  aggregate numbers are printed. `--compare base.json` adds the ratio to an earlier run and fails when a median is more than `--tolerance`
  times (default 2) and more than 20 ms slower.
- Both accounts also time the reads that had no `/api/` route ([#175](https://github.com/334456777/WeFlow/issues/175)): through the
  child-process route, and through both routes for those given one since. They decided which were worth one: a read the page makes when
  it opens, on a click or while scrolling, slower than 50 ms through the child process, or asked for many times at once.

| Read (large account, Windows) | `/<path>` | `/api/<path>` | Decision |
|---|---:|---:|---|
| `chat dates`, `chat date-counts` (213,000 messages) | 589 ms | 63 ms | added: the calendar opens on a click |
| `chat detail --fast` | 55 ms | 2 ms | added: shown when a conversation is picked |
| `sns timeline`, 20 posts | 235 ms | 6 ms | added, without `with-media` |
| `sns users`, `sns stats --fast`, `sns post-counts --prefer-cache` | 22 ms | under 2 ms | added: asked for with the timeline; a child process takes 160 ms more while the server has not opened `sns.db` itself |
| `chat statuses` (60 conversations), `chat tab-counts`, `group list` | 21–24 ms | – | not added: about the cost of starting the process, once per list |
| `chat resources`, a page of 300 | 52–123 ms | 16–56 ms | added once it pages from a cursor ([#178](https://github.com/334456777/WeFlow/issues/178)): the media card pages through it while scrolling. It used to read the newest 90 of each type in 80 conversations and cut an `offset` out of them, so it missed newer ones and deep pages came back empty; now it reads every conversation in the chats' order, newest first, and stops once no conversation left can hold a newer one. It is slowest when few conversations hold the type asked for, as each one is then read: 190–360 ms on the synthetic account, with no video, voice or file in its 501 conversations |
| `analytics overall`, `rankings`, `time` with their cache | 20–23 ms | – | not added: fast enough with the cache; without it (`--force`, 129 ms here) the statistics are counted again, which is for a job |

The same measurements found two reads of the Moments slow in themselves, not only through the child process: the timeline parsed
every post to show 20 (2.5 s for 2,000 posts on the synthetic account, now 77 ms), and `sns post-counts` read the timeline again
for every 500 posts (12 s, now 27 ms).
