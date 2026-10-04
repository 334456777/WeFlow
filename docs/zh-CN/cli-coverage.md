# Rust 原生命令行 — 对原 WeFlow 后端的覆盖率

[English](../cli-coverage.md) | **简体中文**

原作者最后一次提交是 [ca6c479](https://github.com/334456777/WeFlow/tree/ca6c479496d4c7f00ccf234d567b1c51c79fe170)（2026-05-15）

**验证方式**: 当WeFlow Rust CLI 命令或 HTTP 路由复现了某个通道的行为时，该通道记为*已覆盖*。Rust 版本已覆盖的实现与 TypeScript 版本保持一致。验证方式详情见 [cli-unsupported.md](cli-unsupported.md#4-平台与验证范围) 第 4 节。下面的 IPC 分类是手工完成的，欢迎提出异议。

| 指标 | 已覆盖 | 总数 | 占比 |
|---|---|---|---|
| 后端 IPC 通道（`electron/main.ts`；共 172 个，排除 76 个纯 UI 通道） | 79 | 96 | **82%** |
| 后端 IPC 通道 + [部分](#仍然缺失或仅部分覆盖的通道) | 83 | 96 | **86%** |
| CLI 调用的数据库函数（原生 Rust；44 个原生实现，10 个写入操作） | 54 | 54 | **100%** |
| 聊天消息导出格式（chatlab、chatlab-jsonl、json、arkme-json、html、txt、excel、weclone、sql） | 9 | 9 | **100%** |
| HTTP API 路由（`httpService.ts`，路径一致，token 鉴权，SSE 推送） | 19 | 19 | **100%** |

## 仍然缺失或仅部分覆盖的通道

**缺失（13 个）：** 上面的 10 个写入操作、`chat:getVoiceTranscript`、`whisper:downloadModel`、`whisper:getModelStatus`（语音转写需要 sherpa-onnx；~~不打算做~~）。

**部分（4 个）：** `chat:getNewMessages`（采用轮询，而不是响应 WCDB 监听回调），以及 `image:startAutoDownload` / `stopAutoDownload` / `getAutoDownloadStatus`（仅 Windows x64，且只在 `./weflow` 进程运行期间有效）。

这些的细节，以及不改变通道分类的差异（WXGF 需要 `ffmpeg`、导出的媒体目录布局、图片服务事件、仅桌面端的功能），见 [cli-unsupported.md](cli-unsupported.md) 第 3 节。

## 安全说明

除 `/health` 外，HTTP 服务对所有路由都要求 token（`Authorization: Bearer …`、`access_token` 查询参数或 JSON 请求体）；未配置 token 时，其他请求一律被拒绝。默认绑定 `127.0.0.1`。

## 如何复现这些数字

- IPC：`grep -oE "ipcMain\.handle\('[^']+'" electron/main.ts`（172 个），按上文手工分类。
- 数据库函数：`crates/weflow-native/src/wcdb.rs` 中 `Wcdb` 的公开方法里被 `crates/weflow-core` / `weflow-cli` 调用的那些；拒绝或未实现的列在 [cli-unsupported.md](cli-unsupported.md)（有测试保证同步）。
- 导出格式：`crates/weflow-core` 中的 `MESSAGE_EXPORT_FORMATS` 与 `tests/export_e2e.rs`。
- HTTP 路由：`electron/services/httpService.ts` 中的 `pathname ===` / `startsWith('/api/v1/…')` 与 `crates/weflow-core/src/http_server.rs` 中的路由（`tests/http_e2e.rs`）对照。
