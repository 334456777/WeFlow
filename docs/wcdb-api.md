# `wcdb_api.dll`: old and new builds, and why the CLI and the new desktop app do not need it

**English** | [简体中文](zh-CN/wcdb-api.md)

## In short

- The **native Rust CLI** (`./weflow`) and this repository's **desktop app**, which runs on the Rust layer (see
  [desktop-rust-layer.md](desktop-rust-layer.md)), do **not** load, embed or ship `wcdb_api.dll` (nor `libwcdb_api.so`,
  `libwcdb_api.dylib`, `WCDB.dll`, `SDL2.dll`, `libWCDB.dylib`). They read WeChat's databases with the repository's own
  pure-Rust, read-only layer (`crates/weflow-native`); the desktop app reaches it through `weflow_wcdb`
  (`crates/weflow-wcdb-ffi`). Nothing in `resources/wcdb/` is needed to build, run or package them.
- `resources/wcdb/` stays in the repository for the **original desktop app** (upstream WeFlow, whose
  `electron/services/wcdbCore.ts` loads `wcdb_api`; this repository's own `wcdbCore.ts` loads `weflow_wcdb` through the same
  C interface): to supplement, fix and maintain the original version, and as a reference. A new build of `wcdb_api.dll` from the original author replaces the expired one there; the expired one is
  kept next to it.

## Which file is which

| File | Build | Status |
|---|---|---|
| `resources/wcdb/win32/x64/wcdb_api.dll` | New build from the original author (PE timestamp 2026-07-07, sha256 `0904956c…a79350`) | No expiry date, according to the author. For the original desktop app only. |
| `resources/wcdb/win32/x64/wcdb_api.dll.old` | Previous build (PE timestamp 2026-05-07, sha256 `6915913a…9d44b8`) | Stopped working after 2026-09-30 23:59:59 local time: `wcdb_init` returns `-1000` and the library schedules its own deletion. Kept unchanged for research. |
| `resources/wcdb/win32/arm64/wcdb_api.dll`, `resources/wcdb/linux/x64/libwcdb_api.so`, `resources/wcdb/macos/universal/libwcdb_api.dylib` | Previous builds | Not replaced: no new build for these platforms. Whether they carry the same expiry date was not checked. |
| `WCDB.dll`, `SDL2.dll`, `libWCDB.dylib` | Unchanged | Dependencies of `wcdb_api`; the new build uses the same `WCDB.dll`. |

Where the libraries are used:

| | Loads `wcdb_api` | Packages `resources/wcdb/` |
|---|---|---|
| Native Rust CLI (`./weflow`) | No | No (`crates/weflow-assets/build.rs` never embeds `resources/wcdb/`) |
| Desktop app in this repository (Rust layer) | No (`wcdbCore.ts` loads `weflow_wcdb`) | No (`package.json` leaves out `wcdb/**`) |
| Original desktop app (upstream WeFlow, and this repository before the move to the Rust layer) | Yes | Yes |

## Differences between the old and the new x64 build

Read from the files themselves (exports and imports), without running them against data:

- **Added exports**: `wcdb_add_custom_emoticon`, `wcdb_update_custom_emoticon`, `wcdb_delete_custom_emoticon`,
  `wcdb_get_db_status`, `wcdb_probe_fts_schema`, `wcdb_purge_memory`. Of these, only `wcdb_get_db_status` is used by the
  repository's `wcdbCore.ts` (bound as optional); `weflow_wcdb` implements it.
- **Removed exports**: `wcdb_open_message_cursor_lite`, `wcdb_open_message_cursor_lite_with_key`. The original
  `wcdbCore.ts` binds the first one as optional and falls back to `wcdb_open_message_cursor` without it, so the original
  app keeps working, with full rows instead of "lite" rows. `weflow_wcdb` still exports it.
- **Same dependencies**: `WCDB.dll`, the Visual C++ runtime and WinHTTP. Both builds link WinHTTP and contain the addresses
  of a reporting service of the original author; whether and what they send was not verified. The Rust layer has no
  network code.

## Verification against the Rust layer

Attempted on 2026-10-01: the same read functions (sessions, messages, cursors, contacts, groups, statistics, reports,
Moments, search, media, hardlinks, table browsing; about 60 calls) through the original `wcdbCore.ts`, once with the new
`wcdb_api.dll` and once with `weflow_wcdb`, each on its own copy of a real account's databases, comparing the results.

Result: the new `wcdb_api.dll` loads and `InitProtection` succeeds, but `wcdb_open_account` returns **`-1005`**: the
library's own security check does not accept the test program, so no data was read through it and the comparison could
not be made. We did not try to get around that check. Comparing the two needs the host the library is built for (the
original desktop app) or the author's help; this is an open question below.

The Rust layer is verified on its own (synthetic encrypted accounts in the test suite, a real Windows account, outputs
compared byte for byte between versions): see section 4 of [cli-unsupported.md](cli-unsupported.md#4-platform-and-verification-limits).

## Discussion

Questions about `wcdb_api.dll` are discussed in this issue:


- Issue: [hicccc77/WeFlow#1220](https://github.com/hicccc77/WeFlow/issues/1220#issuecomment-5925015014)
