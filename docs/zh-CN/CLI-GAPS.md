# Rust 原生命令行 — 未覆盖的部分

[English](../CLI-GAPS.md) | **简体中文**

本文是 [CLI-COVERAGE.md](CLI-COVERAGE.md) 的补充。基线是原 TypeScript 后端（`ca6c479`）。下面列出的内容要么缺失，要么与桌面端行为不同。CLI 中的任何功能都没有用真实微信数据验证过。

## 1. 完全缺失

| 桌面端通道 | 作用 | 缺失原因 |
|---|---|---|
| `chat:getVoiceTranscript` | 语音消息转文字 | 需要 sherpa-onnx 和 Whisper 模型；尚未接入 Rust 绑定 |
| `whisper:downloadModel`、`whisper:getModelStatus` | 下载/查看 Whisper 模型 | 同上 |
| `chat:clearCurrentAccountData` | 清除当前账号的缓存数据 | 未移植 |
| `cache:clearAll` | 清除所有缓存 | 只有 `analytics clear-cache` 和 `image clear-cache`；消息/联系人/头像缓存未移植（CLI 没有长期驻留的缓存） |
| `sns:debugResource` | 朋友圈资源调试输出 | 未移植 |

## 2. 部分覆盖或行为不同

| 方面 | 差异 |
|---|---|
| 消息推送 / 见解触发 / `chat:getNewMessages` | 桌面端响应 WCDB 监听回调；CLI 采用轮询（推送约 5 秒，见解约 5 秒）。 |
| 联系人（`chat:getContacts`、`getContact`） | 没有联系人标签、个性签名和地区：需要扩展列解析器和约 9.4k 行的地区表。 |
| 消息导出（`export messages`、`exportSession(s)`、`getExportStats`） | 9 种格式都可用，但媒体文件**不会内嵌**到导出中；请单独导出媒体（`export media`，或 HTTP API 的 `media=1`）。 |
| HTTP API / `chat voice-data` 中的语音 | 仅当 WCDB 返回 SILK 数据时可用（微信必须播放过该条消息）。 |
| WXGF 图片 | 通过外部 `ffmpeg` 转换（`PATH` 或 `FFMPEG_PATH`）；桌面端自带 `ffmpeg-static`。没有 ffmpeg 时，图片会被报告为解密失败。 |
| 图片自动下载（`image auto-download`、`serve --image-auto-download`） | 仅 Windows x64（`img_helper.dll`）。钩子只在 `weflow` 进程运行期间存在，因此从另一个进程执行 `status` 总是显示"未挂钩"。 |
| 图片服务事件 | `image:cacheResolved`、`decryptProgress`、`updateAvailable` 以及后台"有更高质量版本"检查都不会发出；`hasUpdate` 始终为 `false`。 |
| AI 见解通知 | 没有弹窗；`serve --insight` 把每条见解以 JSON 行输出到 stderr（Telegram 推送仍可用）。 |
| 排序 | 中文名称的 `localeCompare` 排序只是近似实现。 |
| 视频 | 只查找微信已存放在 `msg/video` 下的文件；没有下载或解密路径（桌面端同样没有）。 |

## 3. 有意不移植（桌面进程相关）

窗口/对话框/shell/app/auth/log 相关 IPC、自动更新、开机自启、应用锁、云控、诊断、社交 cookie 的 UI 辅助、导出任务暂停/恢复、仅渲染进程使用的报告截图、朋友圈缓存迁移 UI。

## 4. 未经验证

- 每个移植都只针对**生成的 mock WCDB 库**和假 HTTP 服务器测试过，从未针对真实微信数据库、真实的 `wcdb_api` 库、真实 `.dat` 文件、真实朋友圈服务器或真实 AI 服务商测试。
- Windows 版本是在 Linux 上交叉编译（`x86_64-pc-windows-gnu`）的；作者没有在 Windows 上运行过。
- 备份归档是否与桌面端自己的备份兼容，尚未验证。
- 仅 Windows 的图片钩子和密钥提取辅助程序，只能在运行着微信的机器上测试。

## 5. 修复而非照搬的问题

TypeScript 的 ISAAC-64 回退实现（精度问题，遵循厂商 WASM），以及*早期 Rust CLI* 中的三个缺陷（`.dat` 布局/密钥、派生 AES 密钥、AI 接口 `/v1`）。详见 CLI-COVERAGE.md。
