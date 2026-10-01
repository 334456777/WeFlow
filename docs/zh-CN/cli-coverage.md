# Rust 原生命令行 — 对原 WeFlow 后端的覆盖率

[English](../cli-coverage.md) | **简体中文**

基线：原作者最后一次提交 `ca6c479`（2026-05-15）时的 TypeScript/Electron 后端。`crates/` 下的所有内容都是之后添加的。

**方法与说明。** 当某个 CLI 命令或 HTTP 路由复现了某个通道的行为时，该通道记为*已覆盖*；TypeScript 代码是逐函数移植的（相同公式、JSON 结果中相同的键顺序、相同的回退逻辑）。数据库层是原生 Rust，已**用一个真实的 Windows 微信 4.x 账号验证**（使用 Linux 构建）；其余部分（HTTP 服务、图片/`.dat` 解密、AI、朋友圈下载）通过单元测试、针对合成加密数据库的端到端测试以及本地假 HTTP 服务器验证。交叉编译出的 Windows `weflow.exe` 已在 Windows 上对同一个账号运行过（约 80 个命令、带媒体的导出）；macOS 和 Linux 账号没有试过。合成数据无法暴露的差异依然可能存在。下面的 IPC 分类是手工完成的，欢迎提出异议。

## 汇总

| 指标 | 已覆盖 | 总数 | 占比 |
|---|---|---|---|
| 后端 IPC 通道（`electron/main.ts`；共 172 个，排除 76 个纯 UI 通道）— 完整 | 79 | 96 | **82%** |
| 同上，完整 + 部分 | 83 | 96 | **86%** |
| CLI 调用的数据库函数（原生 Rust；44 个原生实现，10 个因只读被拒绝） | 54 | 54 | **100%** |
| 聊天消息导出格式（chatlab、chatlab-jsonl、json、arkme-json、html、txt、excel、weclone、sql） | 9 | 9 | **100%** |
| HTTP API 路由（`httpService.ts`，路径一致，token 鉴权，SSE 推送） | 19 | 19 | **100%** |

10 个写操作通道（`chat:updateMessage`、`chat:deleteMessage`、`chat:{check,install,uninstall}AntiRevokeTriggers`、`chat:markAllSessionsRead`、`sns:{check,install,uninstall}BlockDeleteTrigger`、`sns:deleteSnsPost`）原先记为已覆盖；数据库层改为只读之后它们被拒绝，改记为缺失（见 [cli-unsupported.md](cli-unsupported.md)）。

排除的纯 UI 通道：`window:*`、`dialog:*`、`shell:*`、`app:*`、`auth:*`、`log:*`、`cloud:*`、`diagnostics:*`、`social:*`、`http:*` 启停，以及仅渲染进程使用的 `annualReport:{captureCurrentWindow,exportImages,startAvailableYearsLoad,cancelAvailableYearsLoad}` 和 `sns:{getCacheMigrationStatus,startCacheMigration}`（剩余 96 个）。

## 相比第一次评估新增了什么

朋友圈服务（时间线、统计、基于 ISAAC-64 的媒体代理/解密、表情下载、json/html/arkmejson 导出、防删触发器）、聊天消息模型及全部四十余个聊天查询、群分析、含排除名单的统计分析、年度与双人报告（含游标回退）、完整 HTTP API（token 鉴权、媒体、朋友圈路由、SSE 推送）、消息推送引擎、九种消息导出格式、视频查找、语音解码（SILK → WAV）、带会话月份 `.dat` 查找与高清升级的图片解密、Windows 图片自动下载钩子、AI 见解引擎（记录、沉默扫描、活跃触发、Telegram、足迹总结）以及微博上下文客户端。

## 移植过程中修复的问题

- `.dat` 解密：V1 文件使用默认密钥 `cfcd208495d565ef`；V1/V2 共享布局 *AES-128-ECB 头（PKCS7）· 原文中段 · XOR 尾*。CLI 第一版把头部之后的内容全部做了 XOR。
- 派生的图片 AES 密钥是 `md5(code + wxid)` 十六进制的**前 16 个字符按 ASCII 使用**，与桌面端密钥服务一致（CLI 第一版用了 16 个摘要字节）。
- AI 接口地址是 `<base>/chat/completions`；CLI 第一版多插入了一段 `/v1`。
- TypeScript 的 ISAAC-64 回退实现有精度问题（`Number(x>>3n)&255`）；Rust 版本遵循厂商 WASM，它才是权威实现。
- ChatLab 导出：图片、语音、视频、表情、通话消息（类型不是 49 的 `<msg>` XML）被误标为“链接”；现在只有真正的应用消息（类型 49 / 含 `<appmsg`）才是链接。TypeScript 原版有同样的问题。
- `chat anti-revoke` 在所有会话都失败时也会返回成功；现在会返回错误。
- 年度报告里的“每月聊得最多的人”在原生层上是空的（缺少每个会话的月度计数）；现在已原生提供，扩展统计（热力图、夜猫子、主动发起、响应速度、常用语、连续天数）也已原生实现，数字与游标回退一致。
- 原生行带有 `is_send`（按账号 wxid 计算），导出和报告代码依赖它。

## 仍然缺失或仅部分覆盖的通道

**缺失（13 个）：** 上面的 10 个写操作通道（有意拒绝：原生数据库层以只读方式打开微信数据库）、`chat:getVoiceTranscript`、`whisper:downloadModel`、`whisper:getModelStatus`（语音转写需要 sherpa-onnx；不打算做）。

**部分（4 个）：**

- `chat:getNewMessages`、消息推送和见解触发采用轮询，而不是响应 WCDB 监听回调。
- `image:startAutoDownload` / `stopAutoDownload` / `getAutoDownloadStatus`：仅 Windows x64；钩子只在 `weflow` 进程运行期间存在，因此从另一个进程执行 `status` 看不到它。

另有不同之处，但不改变通道的分类：WXGF → JPEG 需要 `PATH` 中有 `ffmpeg` 可执行文件（桌面端自带 `ffmpeg-static`）；消息导出用 `export messages --media …` 把媒体复制到 `media/<输出文件名>/`，目录布局与桌面端不同。

UI 事件（`image:cacheResolved`、`image:decryptProgress`、`image:updateAvailable`、通知弹窗）在命令行中没有对应物；与桌面端无界面的 worker 模式一样，图片服务不发出这些事件，也从不报告 `hasUpdate`。

未移植，因为它们只对桌面进程有意义：消息/联系人/会话/头像缓存、云控、导出任务暂停/恢复。

## 安全说明

除 `/health` 外，HTTP 服务对所有路由都要求 token（`Authorization: Bearer …`、`access_token` 查询参数或 JSON 请求体）；未配置 token 时，其他请求一律被拒绝。默认绑定 `127.0.0.1`。

## 如何复现这些数字

- IPC：`grep -oE "ipcMain\.handle\('[^']+'" electron/main.ts`（172 个），按上文手工分类。
- 数据库函数：`crates/weflow-native/src/wcdb.rs` 中 `Wcdb` 的公开方法里被 `crates/weflow-core` / `weflow-cli` 调用的那些；拒绝或未实现的列在 [cli-unsupported.md](cli-unsupported.md)（有测试保证同步）。
- 导出格式：`crates/weflow-core` 中的 `MESSAGE_EXPORT_FORMATS` 与 `tests/export_e2e.rs`。
- HTTP 路由：`electron/services/httpService.ts` 中的 `pathname ===` / `startsWith('/api/v1/…')` 与 `crates/weflow-core/src/http_server.rs` 中的路由（`tests/http_e2e.rs`）对照。
