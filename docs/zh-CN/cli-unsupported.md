# 原生命令行不支持的功能

[English](../cli-unsupported.md) | **简体中文**

这里列出 `./weflow` 做不到、暂时没做、或与桌面端表现不同的所有地方，对照的是原 TypeScript 后端[ca6c479](https://github.com/334456777/WeFlow/tree/ca6c479496d4c7f00ccf234d567b1c51c79fe170)。已经覆盖的部分及其数字见
[cli-coverage.md](cli-coverage.md)。

## 1. 占位符未实现

这些命令为兼容而保留，但由于原生数据库层以只读方式打开微信数据库，因此一律报错，报错信息是
`<函数名> is not supported: the native database backend opens WeChat's databases read-only`。

| CLI 命令 | 原生函数 | 桌面端里的作用 |
|---|---|---|
| `chat update-message` | `update_message` | 修改已存消息的文本 |
| `chat delete-message` | `delete_message` | 删除已存消息 |
| `chat anti-revoke check` / `install` / `uninstall` | `anti_revoke_check`、`anti_revoke_install`、`anti_revoke_uninstall` | 查询 / 安装 / 移除保留撤回消息的数据库触发器(`check` 本身只读，但与另两个一起被拒绝) |
| `chat mark-read` | `mark_all_sessions_read` | 清除所有会话的未读数 |
| `sns block-delete`(check / install / uninstall) | `sns_block_delete_check`、`sns_block_delete_install`、`sns_block_delete_uninstall` | 保留好友已删除朋友圈的触发器 |
| `sns delete` | `sns_delete_post` | 从本地数据库删除一条朋友圈 |
| (无命令) | `import_table_snapshot`、`import_table_snapshot_with_schema` | 把表快照恢复进数据库(导出一侧可用，且只写到 `db_storage` 之外) |

原因: 正在运行的微信同时打开着这些数据库，往里写可能损坏它们，而且 SQLCipher 文件还得逐页重新加密。
如果确实需要其中某项，需要单独设计。

## 2. 原生数据库层尚未实现

目前没有:服务层能调用的每个数据库函数，要么已实现，要么被有意拒绝(第 1 节)。如果有函数先加进 `crates/weflow-native/src/wcdb.rs`、还没移植，
它会返回 `<函数名> is not implemented in the native database backend yet`;在移植完成前把它列在这里。

## 3. 桌面端功能:缺失或表现不同

| 方面 | 差异 |
|---|---|
| 语音转文字(`chat:getVoiceTranscript`、`whisper:downloadModel`、`whisper:getModelStatus`) | 缺失:需要 sherpa-onnx 和 Whisper 模型;不打算做。 |
| 实时更新(消息推送、见解触发、`chat:getNewMessages`) | 桌面端响应 WCDB 监听回调;CLI 采用轮询(推送和见解约每 5 秒一次)。 |
| 消息导出 | 9 种格式都可用;`--media` 按 CLI 自己的目录布局复制媒体(见第 5 节)。 |
| HTTP API / `chat voice-data` 中的语音 | 仅当媒体数据库里有 SILK 数据时可用(微信必须播放过该条消息)。 |
| WXGF 图片 | 由 CLI 自己解码(桌面端用自带的 ffmpeg 转换)，写成的 JPEG 在画质相当时比 ffmpeg 的大约四分之一。内置解码器读不了的画面(10-bit、4:2:2 或 4:4:4;真实账号约 1800 张里一张也没有)才通过外部 `ffmpeg` 转换(`FFMPEG_PATH`、`PATH`，或 `ffmpeg install` 下载的副本：与桌面端自带的 `ffmpeg-static` 是同一构建，并校验 SHA-256)。CLI 不附带 ffmpeg，也不会自行下载。没有 ffmpeg 时，这类图片会失败，`failure_kind` 为 `ffmpeg_missing`，错误信息指向 `weflow ffmpeg install`；带媒体的导出会把这类图片只计一次，记在 `ffmpegMissing` 里，并在 `hint` 中说明(`FFMPEG_PATH` 指向的程序无法启动时也会如实报告)。 |
| 图片自动下载(`image auto-download`、`serve --image-auto-download`) | 仅 Windows x64(`img_helper.dll`)。钩子只在 `./weflow` 进程运行期间存在，因此从另一个进程执行 `status` 总是显示"未挂钩"。 |
| 图片服务事件 | `image:cacheResolved`、`decryptProgress`、`updateAvailable` 以及后台"有更高质量版本"检查都不会发出;`hasUpdate` 始终为 `false`(与桌面端无界面的 worker 模式一样)。 |
| AI 见解通知 | 没有弹窗;`serve --insight` 把每条见解以 JSON 行输出到 stderr(Telegram 推送仍可用)。 |
| 图片密钥内存扫描 | `key scan-image` 仅 macOS 可用;Windows 请用 `key image`(kvcomm 缓存 + 模板校验)。桌面端 Windows 的内存扫描回退未移植。 |
| 视频 | 只查找微信已存放在 `msg/video` 下的文件;没有下载或解密路径(桌面端同样没有)。 |

有意不移植，因为它们只对桌面进程有意义:窗口/对话框/shell/app/auth/log 相关 IPC、自动更新、开机自启、应用锁、云控、诊断、社交 cookie 的 UI 辅助、
消息/联系人/会话/头像缓存、导出任务暂停/恢复、仅渲染进程使用的报告截图、朋友圈缓存迁移界面。

## 4. 平台与验证范围

- 原生数据库层(会话、消息、联系人、朋友圈、媒体、语音库)用一个真实的 Windows 微信 4.x 账号做过验证，用的是 **Linux 构建**;交叉编译出的
  Windows `weflow.exe`(`x86_64-pc-windows-gnu`)也在 Windows 上对同一个账号运行过(约 80 个命令、带媒体的消息导出、图片导出)。其他 Windows 版本没有试过。
- macOS 和 Linux 的微信数据库文件格式相同，但没有测试过。
- HTTP 服务、图片 `.dat` 解密、AI 和朋友圈下载另外用合成的加密夹具和本地假 HTTP 服务器测试过;真实的朋友圈服务器和 AI 服务商没有试过。
- 密钥提取辅助程序(`key db`、`key image`)和 Windows 图片钩子需要微信正在运行，无法离线测试。
- 只用了一个账号的数据验证;特殊的数据库(特别大的分片、旧版本表结构)可能暴露遗漏。

## 5. 可能出乎意料的行为

- **快照**: 查询读到哪些页才解密哪些页，SQLite 会把读过的页留在缓存里(遍历整个消息库的命令可能留下整个库，几百 MB;
  各缓存合计超过约 1 GB 时会释放内存)。长时间运行的 `serve` 会在文件或其 `-wal` 变化时读到新消息。查询读取期间，如果微信
  做了超出快照范围的检查点，这次查询会在新快照上重新执行。`export messages` 在开始时就确定要导出哪些消息，导出期间微信新写入的
  消息留给下一次导出。
- **大会话导出的内存**: `export messages` 按页把会话转成导出记录，再据此写文件，每个读取线程读库时只用很小的页缓存。真实账号
  里 20 万条消息的群，按导出自己选的线程数(16 个逻辑处理器)，峰值约 0.2 GB(`json`、`arkme-json`)、0.2~0.3 GB(`txt`、`sql`、
  `html`、`excel`)、0.25~0.4 GB(`chatlab`、`chatlab-jsonl`、`weclone`);区间上限是 Linux 上的常驻内存，下限是 Windows 上的
  working set。带媒体的导出(`--media`)会先把整个会话读进内存，再复制媒体、最后写文件：同一个群带图片、语音和视频时，
  Windows 上约 0.5 GB;第一次导出时要把其中的 WXGF 图片解码进图片缓存(同时解码好几张)，约 0.8 GB。内存紧张时请导出日期范围(`--start/--end`)，耗时只与范围大小有关，和日期远近无关。
- **大会话导出的线程**: 写文件的同时，有多个线程读取并解析各页。导出从两个线程开始，写文件经常要等新页时就增加一个，
  最多为 CPU 数减一(不超过 8 个);新增的线程如果没让读取明显变快(提速不到它理论上能带来的一半)，就不再增加。`chatlab` 和
  `chatlab-jsonl` 的条目也在读取线程上渲染，所以这两种格式和 `weclone` 用的线程最多(16 个逻辑处理器上 5~7 个);`txt`、
  `sql`、`html`、`excel` 约 3 个，`json`、`arkme-json` 保持 2 个(瓶颈在写出)。带媒体的导出只收集消息，最多用 2 个读取线程。
  每个读取线程有自己的页缓存，还会提前读好几页，每个约 20~60 MiB，上面的峰值已经包含在内。`WEFLOW_EXPORT_WORKERS=1`
  只用一个线程读取(约 0.13 GB，但更慢)，设为其他数字则固定线程数。`RUST_LOG=weflow::export=debug` 会记录使用的线程数，以及
  读取、解析(和渲染)、等待所花的时间(设为 `=trace` 还会记录每次是否增加线程的判断)。没有任何缓存时(重启后第一次导出)，
  同样的导出从 NVMe 固态硬盘读取要多花 2~25% 的时间，多线程省下的时间与热缓存时相当。
- **Windows 上启动时的内存**: CLI 启动时会为分配器预留并提交 128 MiB，这样导出时能少几十万次缺页。即使是很短的命令，这部分也
  计入提交量(专用字节)，但不占物理内存(working set)。可以用 `MIMALLOC_RESERVE_OS_MEMORY` 改成别的大小。
- **密钥校验**: 第一次用某个密钥运行命令时，会在 `session.db` 上验证它，并在缓存目录里记下一个单向指纹(由密钥、数据库盐值和
  账号算出，无法反推出密钥)，之后的命令就跳过这一步较慢的校验。`chat clear-account-data --cache` 会删除这些指纹;
  `db test` 总是重新校验密钥。
- **时区**: 导出的 `--start/--end` 日期、写进导出文件的时间、`chat dates`、`chat date-counts` 和按日统计都使用本机本地时区(与桌面端一致)。同一份数据库在另一个时区的机器上读取，深夜的消息会归到不同的日子。
- **导出里的媒体**: `export messages --media image,voice,video,emoji`(或 `all`)把文件复制到输出文件旁边的 `media/<输出文件名>/{images,voices,videos,emojis}`，并让消息指向它们。
  哪些格式有位置放媒体:`json`/`arkme-json`、`txt`、`excel`、`weclone`(内容或 `src` 变成相对路径)、`chatlab`(仅图片)、`html`(`<img>`、`<audio>`、`<video>`);`sql` 没有。磁盘上找不到的文件保留占位文字。
  表情需要联网;桌面端用自己的目录布局。
- **中文名称排序**(联系人列表、群成员)遵循 ICU/CLDR 拼音排序，与 `Intl.Collator('zh-CN')` 一致:数字在前，然后是按拼音排列的汉字，最后是拉丁字母。
- **群聊文本**: 导出的群消息保留发送者前缀后面的换行(`wxid_xxx:` 被去掉，换行还在)，与桌面端导出一致。
- **搜索**覆盖文本、链接/文件、引用回复消息;关键字按字面匹配(不区分大小写)，压缩消息会先解码再匹配。图片、语音、表情不参与搜索。
- **折叠 / 免打扰**状态由联系人标志位推断(折叠:第 28 位;免打扰:第 9 位或群通知标志)，依据微信的惯例;没有用真实的折叠会话验证过。
- **足迹**把 `@所有人`(`notify@all`)算作 @ 了你;判定为 @ 需要文本里有 `@`，且消息 `atuserlist` 里有你的 id(或 `notify@all`)。
  私聊静默超过 1 小时就切成新的一段。
- **双人报告**的常用语是 2~20 个字、不含链接和标记、至少出现 2 次的完全相同文本;响应时间只统计你在同一轮聊天内的回复
  (间隔超过 1 小时算新一轮)。
- **朋友圈年度统计**统计你自己的帖子、给你点赞最多的好友、以及你点赞最多的好友的帖子。
- 写操作(第 1 节)会被拒绝，只读命令绝不会改动微信数据。

## 6. 保持清单最新（面向LLM）

当有返回 *not implemented* 或 *not supported* 的数据库函数没有出现在本文件或 [../cli-unsupported.md](../cli-unsupported.md) 的第 1~2 节里时，
`cargo test -p weflow-native --test unsupported_docs` 会失败。移植一个函数时，在同一次改动里删掉它的那一行;新增限制时，补一行。
