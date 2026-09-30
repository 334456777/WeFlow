# Rust 原生命令行 — 未覆盖的部分

[English](../cli-gaps.md) | **简体中文**

本文是 [cli-coverage.md](cli-coverage.md) 的补充。基线是原 TypeScript 后端（`ca6c479`）。下面列出的内容要么缺失，要么与桌面端行为不同。原生数据库层已用一个真实账号验证过(见第 4 节);完全不可用的功能的完整清单见 [cli-unsupported.md](cli-unsupported.md)。

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
| HTTP API / `chat voice-data` 中的语音 | 仅当媒体数据库里有 SILK 数据时可用（微信必须播放过该条消息）。 |
| WXGF 图片 | 通过外部 `ffmpeg` 转换（`PATH` 或 `FFMPEG_PATH`）；桌面端自带 `ffmpeg-static`。没有 ffmpeg 时，图片会被报告为解密失败。 |
| 图片自动下载（`image auto-download`、`serve --image-auto-download`） | 仅 Windows x64（`img_helper.dll`）。钩子只在 `weflow` 进程运行期间存在，因此从另一个进程执行 `status` 总是显示"未挂钩"。 |
| 图片服务事件 | `image:cacheResolved`、`decryptProgress`、`updateAvailable` 以及后台"有更高质量版本"检查都不会发出；`hasUpdate` 始终为 `false`。 |
| AI 见解通知 | 没有弹窗；`serve --insight` 把每条见解以 JSON 行输出到 stderr（Telegram 推送仍可用）。 |
| 图片密钥内存扫描 | `key scan-image`（扫描微信内存找 AES 密钥）仅 macOS 可用；Windows 请用 `key image`（kvcomm 缓存 + 模板校验）。桌面端 Windows 的内存扫描回退未移植。 |
| 排序 | 中文名称的 `localeCompare` 排序只是近似实现。 |
| 视频 | 只查找微信已存放在 `msg/video` 下的文件；没有下载或解密路径（桌面端同样没有）。 |

## 3. 有意不移植（桌面进程相关）

窗口/对话框/shell/app/auth/log 相关 IPC、自动更新、开机自启、应用锁、云控、诊断、社交 cookie 的 UI 辅助、导出任务暂停/恢复、仅渲染进程使用的报告截图、朋友圈缓存迁移 UI。

## 4. 未经验证

- 原生数据库层（会话、消息、联系人、朋友圈、语音、报告）已用一个真实的 Windows 微信 4.x 账号在 Linux 构建上验证过。其余部分（HTTP 服务、图片/`.dat` 解密、AI、朋友圈网络下载）只针对合成的加密夹具和假 HTTP 服务器测试过，从未针对真实 `.dat` 文件、真实朋友圈服务器或真实 AI 服务商测试。
- Windows 版本是在 Linux 上交叉编译（`x86_64-pc-windows-gnu`）的；还没有在 Windows 上运行过。
- 备份归档是否与桌面端自己的备份兼容，尚未验证。
- 仅 Windows 的图片钩子和密钥提取辅助程序，只能在运行着微信的机器上测试。

## 5. 修复而非照搬的问题

TypeScript 的 ISAAC-64 回退实现（精度问题，遵循厂商 WASM），以及*早期 Rust CLI* 中的三个缺陷（`.dat` 布局/密钥、派生 AES 密钥、AI 接口 `/v1`）。详见 cli-coverage.md。

## 数据库层已是原生 Rust（不再使用 `wcdb_api`）

闭源的 `wcdb_api` 库**不再被使用、内嵌或加载**：它带有有效期检查（超过 2026-09-30 23:59:59 本地时间后 `wcdb_init` 返回 `-1000` 并尝试删除自身），还含有未经核实的网络代码。CLI 现在自己解密微信 4.x 数据库（SQLCipher 4 布局：用 32 字节原始密钥做 PBKDF2-HMAC-SHA512、AES-256-CBC 页加 HMAC-SHA512、WAL 同样加密），再用内置 SQLite 读取；明文不会写到磁盘。

目前已移植：会话、消息（所有分片、zstd 压缩内容、分页、按日统计、游标）、联系人、显示名、头像、群成员/昵称/群主。其余数据库调用在移植前仍返回 `... is not implemented in the native database backend yet`；清单见 `crates/weflow-native/src/wcdb.rs`，等待它们的测试在 `crates/weflow-core/tests` 里标有 `#[ignore = "pending native port ..."]`。
