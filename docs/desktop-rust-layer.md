# Desktop app on the Rust database layer (for LLMs)

**English** | [简体中文](zh-CN/desktop-rust-layer.md)

The desktop app (Electron) originally read WeChat databases through the closed-source `wcdb_api` library. The copy in this repository expired after 2026-09-30 (`wcdb_init` returns `-1000`). The desktop app now loads **`weflow_wcdb`**, a C-interface library built from `crates/weflow-wcdb-ffi` and backed by the same pure-Rust, read-only database layer as the CLI.

## Architecture

- `crates/weflow-wcdb-ffi` exports 91 of the 92 functions declared by `electron/services/wcdbCore.ts` (`wcdb_open_account`, `wcdb_get_sessions`, `wcdb_open_message_cursor`, and so on), with the same parameters, status codes, and JSON results. The one it does not export is `VerifyUser` (see the table below). Strings are still returned through `_Out_ void**` parameters and freed with `wcdb_free_string`.
- The only change in `wcdbCore.ts` is where it looks for the library: `resources/native-db/<platform>/<arch>/weflow_wcdb.dll` (`libweflow_wcdb.so` or `libweflow_wcdb.dylib`). During development it also checks `target/release/`, and `WCDB_DLL_PATH` can specify another path. It no longer preloads `WCDB.dll`, `SDL2.dll`, or `libWCDB.dylib`.
- Packages include `resources/native-db/` instead of `resources/wcdb/`. Those libraries (the newer `wcdb_api.dll` and the expired older build) remain in the repository for the original desktop app; see [wcdb-api.md](wcdb-api.md).
- Change notifications: `wcdb_start_monitor_pipe` opens a named pipe on Windows or a Unix socket and sends one JSON line (`session_change`, `message_change`, or `contact_change`) whenever a database changes, detected by checking files once per second. The desktop app's existing pipe client receives these notifications unchanged.

## Building

```
npm run native-db:build          # cargo build -p weflow-wcdb-ffi, then copy into resources/native-db/
npm run build                    # native-db:build, then tsc, vite, and electron-builder
```

For cross-compilation—for example, building the Windows library on Linux—run `node scripts/build-native-db.cjs --target x86_64-pc-windows-gnu` (requires the MinGW toolchain).

## Differences from the old library

| Area | Behavior |
|---|---|
| Editing/deleting messages, anti-revoke and Moments delete-blocking triggers, “mark all as read,” deleting Moments posts, and importing tables | Refused with status code `-4` and a read-only explanation; trigger checks report “not installed.” The UI displays this error. |
| `InitProtection`, `wcdb_init`, and cloud reporting (`wcdb_cloud_*`) | Accepted as no-ops: there is no expiry check and no network access. |
| `VerifyUser` (Windows Hello verification in the old library) | Not exported; the desktop app treats the function as unavailable. |
| Change monitoring | Checks file sizes and modification times once per second instead of using `ReadDirectoryChangesW`; events contain only the database path, not a specific table or session. |
| Group members | Returned as an array, including `avatarUrl` from the contact table. |

## Verification

- Rust tests of the C interface use a synthetic encrypted account (`cargo test -p weflow-wcdb-ffi`) and include the monitor socket.
- On Windows, Node's `koffi` loaded `weflow_wcdb.dll` and tested a real account: opening, sessions, messages, a cursor over 200,000 messages, contacts, avatars, annual reports, refusal of write operations, and connection to the monitor pipe.
- The desktop app's own packaged `wcdbCore.ts` was run under Node with the Linux library against a real account. About 60 calls (sessions, messages, counts, cursors, contacts, groups, analytics, reports, Moments, search, media streams, hardlinks, table browsing, and monitoring) all succeeded; write operations were refused as expected.
- `tsc -p tsconfig.node.json` has no new errors.
- **Not yet done:** Running the packaged desktop UI on Windows, macOS, and Linux.
