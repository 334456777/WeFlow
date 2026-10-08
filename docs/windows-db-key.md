# Windows database-key capture

[简体中文](zh-CN/windows-db-key.md)

Windows x64 `key db` uses Rust in `weflow-native/src/windows_db_key/`. The version signatures and the key argument layout (RDX points to a structure with a pointer at +8 and a byte count at +16) come from [ycccccccy/wx_key](https://github.com/ycccccccy/wx_key), under its [MIT license](../crates/weflow-native/src/windows_db_key/LICENSE).

The implementation reads executable regions of `Weixin.dll`, requires exactly one signature match and attaches with the documented [Windows debugging API](https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-debugactiveprocess). It uses a free hardware breakpoint slot on existing and newly created threads. It preserves other slots, forwards unrelated exceptions, reads exactly 32 key bytes, restores its registers and [detaches](https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-debugactiveprocessstop) before publishing a key. It does not allocate executable memory or patch the client's instructions. Timeout, explicit cleanup, Ctrl+C and process exit are handled by the same worker.

Only x64 WeChat 4.x is supported. The upstream signatures select 4.0/4.1.0–4.1.3 and 4.1.4 or later; signatures do not guarantee that every future 4.x release will work. Missing or ambiguous matches fail before debugger attachment. Windows ARM64 builds do not load an x64 DLL as a substitute.

The CLI links the capture code directly. `weflow-wxkey` exports the original C++ bool ABI (`InitializeHook`, `PollKeyData`, `CleanupHook`, `GetStatusMessage`, `GetLastErrorMsg`, `GetImageKey`) for desktop callers. `GetImageKey` returns candidate codes through #95's Rust parser; callers must verify them against the selected account. `npm run native-db:build` builds the database library and, on Windows x64, the Rust key DLL into `resources/native-key/win32/x64/`. The DLL must be cleaned up before unloading. There is no vendor-library fallback or environment variable for switching implementations.

An existing debugger, an exhausted hardware-breakpoint set or insufficient process permissions causes an explicit error. Builds are unsigned; no certificate is supplied by the repository. Security software may restrict debugger attachment: inspect its recorded decision and permit only a build you trust; this implementation does not change antivirus settings or use indirect syscalls to bypass monitoring.

Automated tests use a separate synthetic x64 process: main/new threads, invalid key arguments, function return values, cancellation, repeated attach, process exit, and debugger/register restoration. Real-client acceptance still requires recording the client version and architecture, capturing during login, checking the result with `db test`, and checking that WeChat remains usable after cleanup. No real account IDs, keys, paths or chat data belong in public test artifacts.
