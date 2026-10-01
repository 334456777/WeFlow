# Desktop app on the Rust database layer

**English** | [简体中文](zh-CN/desktop-rust-layer.md)

The desktop app (Electron) used to read WeChat's databases through the closed-source `wcdb_api` library, whose build in this
repository stopped working after 2026-09-30 (`wcdb_init` returns `-1000`). It now loads **`weflow_wcdb`** instead: a C library
built from `crates/weflow-wcdb-ffi` on top of the same pure-Rust, read-only layer as the CLI.

## How it fits

- `crates/weflow-wcdb-ffi` exports the functions `electron/services/wcdbCore.ts` declares (`wcdb_open_account`,
  `wcdb_get_sessions`, `wcdb_open_message_cursor`, … 88 of the 92), with the same arguments, status codes and JSON results.
  Strings are returned through `_Out_ void**` arguments and released with `wcdb_free_string`, as before.
- `wcdbCore.ts` only changed where it looks for the library: `resources/native-db/<platform>/<arch>/weflow_wcdb.dll`
  (`libweflow_wcdb.so`, `libweflow_wcdb.dylib`), or `target/release/` during development, or `WCDB_DLL_PATH`. It no longer
  preloads `WCDB.dll` / `SDL2.dll` / `libWCDB.dylib`.
- Packaging ships `resources/native-db/` and leaves out `resources/wcdb/`. Those libraries stay in the repository for the
  original desktop app (the new build of `wcdb_api.dll` and the expired one): see [wcdb-api.md](wcdb-api.md).
- Change notifications: `wcdb_start_monitor_pipe` opens a named pipe (Windows) or a Unix socket and sends one JSON line per
  database change (`session_change`, `message_change`, `contact_change`), found by checking the files once a second. The app's
  existing pipe client receives them unchanged.

## Building

```
npm run native-db:build          # cargo build -p weflow-wcdb-ffi, copied into resources/native-db/
npm run build                    # runs native-db:build first, then tsc, vite and electron-builder
```

Cross-building, for example the Windows library from Linux:
`node scripts/build-native-db.cjs --target x86_64-pc-windows-gnu` (needs the MinGW toolchain).

## Differences from the old library

| Area | Behaviour |
|---|---|
| Editing / deleting messages, anti-revoke and Moments delete-blocking triggers, "mark all read", Moments delete, table import | Refused with status `-4` and a "read-only" message; trigger checks report "not installed". The UI shows the error. |
| `InitProtection`, `wcdb_init`, cloud reporting (`wcdb_cloud_*`) | Accepted and do nothing: there is no expiry check and no network access. |
| `VerifyUser` (Windows Hello prompt through the old library) | Not exported; the app treats the function as unavailable. |
| Change monitor | Polling of file sizes and times (about 1 s) instead of `ReadDirectoryChangesW`; events carry the database path, not the changed table or session. |
| Group members | Returned as an array with `avatarUrl` from the contact table. |

## Verification

- Rust tests of the C ABI against synthetic encrypted accounts (`cargo test -p weflow-wcdb-ffi`), including the monitor socket.
- `weflow_wcdb.dll` loaded from Node with `koffi` on Windows against a real account: open, sessions, messages, a 202,000-message
  cursor, contacts, avatars, annual report, refused writes, and a client connected to the monitor pipe.
- The desktop app's own `wcdbCore.ts`, bundled and run under Node against the real account with the Linux library: about 60
  calls (sessions, messages, counts, cursors, contacts, groups, statistics, reports, Moments, search, media stream, hardlinks,
  table browsing, monitor) all succeed; the write calls are refused as expected.
- `tsc -p tsconfig.node.json` reports no new errors.
- **Not done yet:** running the packaged desktop app with its UI on Windows, macOS and Linux (see
  [cli-unsupported.md](cli-unsupported.md#still-to-verify), item 4).
