# Native Rust CLI — coverage of the original WeFlow backend

**English** | [简体中文](zh-CN/cli-coverage.md)

The original author's last commit was [ca6c479](https://github.com/334456777/WeFlow/tree/ca6c479496d4c7f00ccf234d567b1c51c79fe170) (2026-05-15).

**Verification method:** A channel counts as *covered* when a WeFlow Rust CLI command or HTTP route reproduces its behavior. Covered Rust implementations match their TypeScript counterparts. See section 4 of [cli-unsupported.md](cli-unsupported.md#4-platform-and-verification-limits) for details of the verification scope. The IPC classification below was done manually; challenges are welcome.

| Metric | Covered | Total | Percentage |
|---|---:|---:|---:|
| Backend IPC channels (`electron/main.ts`; 172 total, excluding 76 UI-only channels) | 79 | 96 | **82%** |
| Backend IPC channels including [partial coverage](#channels-that-are-still-missing-or-partial) | 83 | 96 | **86%** |
| Database functions called by the CLI (native Rust; 44 native implementations and 10 write operations) | 54 | 54 | **100%** |
| Chat-message export formats (chatlab, chatlab-jsonl, json, arkme-json, html, txt, excel, weclone, sql) | 9 | 9 | **100%** |

## Channels that are still missing or partial

**Missing (13):** The 10 write operations above, `chat:getVoiceTranscript`, `whisper:downloadModel`, and `whisper:getModelStatus` (voice transcription requires sherpa-onnx; ~~not planned~~).

**Partial (4):** `chat:getNewMessages` (polling instead of reacting to WCDB monitor callbacks), plus `image:startAutoDownload`, `stopAutoDownload`, and `getAutoDownloadStatus` (Windows x64 only, and only while the `./weflow` process is running).

For details, and for differences that do not change a channel's classification (WXGF requiring `ffmpeg`, the exported-media directory layout, image-service events, and desktop-only features), see section 3 of [cli-unsupported.md](cli-unsupported.md).

## Reproducing the numbers

The TypeScript paths below are files in [ca6c479](https://github.com/334456777/WeFlow/tree/ca6c479496d4c7f00ccf234d567b1c51c79fe170).

- IPC: `grep -oE "ipcMain\.handle\('[^']+'" electron/main.ts` (172 results), classified manually as described above.
- Database functions: the public `Wcdb` methods in `crates/weflow-native/src/wcdb.rs` that are called by `crates/weflow-core` / `weflow-cli`; refused or unimplemented functions are listed in [cli-unsupported.md](cli-unsupported.md) (a test keeps the list synchronized).
- Export formats: `MESSAGE_EXPORT_FORMATS` in `crates/weflow-core` and `tests/export_e2e.rs`.
