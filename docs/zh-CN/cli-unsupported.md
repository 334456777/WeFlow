# 原生命令行不支持的功能

[English](../cli-unsupported.md) | **简体中文**

这份清单详细列出 `weflow` 做不到、暂时做不到、或与桌面端表现不同的地方。它是
[cli-gaps.md](cli-gaps.md)(与原 TypeScript 后端的对比)和 [cli-coverage.md](cli-coverage.md) 的补充。

数据库层是纯 Rust:自己解密微信 4.x 数据库,并以**只读**方式打开(不会写微信的任何文件,也不会把明文写到磁盘)。
第 1、2 节列出了所有数据库层不可用的函数,并有测试保证它们和代码保持一致(见[第 6 节](#6-保持清单最新))。

## 1. 有意拒绝:任何会修改微信数据库的操作

这些命令为兼容而保留,但一律失败,报错信息是
`<函数名> is not supported: the native database backend opens WeChat's databases read-only`。

| CLI 命令 | 原生函数 | 桌面端里的作用 |
|---|---|---|
| `chat update-message` | `update_message` | 修改已存消息的文本 |
| `chat delete-message` | `delete_message` | 删除已存消息 |
| `chat anti-revoke check` / `install` / `uninstall` | `anti_revoke_check`、`anti_revoke_install`、`anti_revoke_uninstall` | 查询 / 安装 / 移除保留撤回消息的数据库触发器(`check` 本身只读,但与另两个一起被拒绝) |
| `chat mark-read` | `mark_all_sessions_read` | 清除所有会话的未读数 |
| `sns block-delete`(check / install / uninstall) | `sns_block_delete_check`、`sns_block_delete_install`、`sns_block_delete_uninstall` | 保留好友已删除朋友圈的触发器 |
| `sns delete` | `sns_delete_post` | 从本地数据库删除一条朋友圈 |
| (无命令) | `import_table_snapshot`、`import_table_snapshot_with_schema` | 把表快照恢复进数据库(导出一侧可用,且只写到 `db_storage` 之外) |

原因:正在运行的微信同时打开着这些数据库,往里写可能损坏它们,而且 SQLCipher 文件还得逐页重新加密。
如果确实需要其中某项,需要单独设计(先关闭微信、先备份)。

## 2. 原生数据库层尚未实现

目前没有:服务层能调用的每个数据库函数,要么已实现,要么被有意拒绝(第 1 节)。如果有函数先加进 `crates/weflow-native/src/wcdb.rs`、还没移植,
它会返回 `<函数名> is not implemented in the native database backend yet`;在移植完成前把它列在这里。

## 3. 桌面端有、CLI 没有的功能

细节和原因见 [cli-gaps.md](cli-gaps.md)。简要如下:

- 语音转文字,以及 Whisper 模型的下载/状态(需要 sherpa-onnx 和 Whisper 模型)。
- "清除当前账号数据"和"清除所有缓存"(CLI 没有长期缓存,只有 `analytics clear-cache` 和 `image clear-cache`)。
- `sns:debugResource`(朋友圈资源调试导出)。
- WXGF 图片需要外部 `ffmpeg`(在 `PATH` 中,或设置 `FFMPEG_PATH`)。
- 图片自动下载钩子(`image auto-download`、`serve --image-auto-download`)只支持 Windows x64,且只在进程运行期间有效。
- `key scan-image`(内存扫描 AES 密钥)只支持 macOS;Windows 上用 `key image`。
- 实时更新靠轮询(消息推送和洞察约每 5 秒一次),不是 WCDB 监听回调。
- 没有弹窗、自动更新、开机自启、应用锁、云控等桌面进程功能。

## 4. 平台与验证范围

- 原生数据库层用一个真实的 Windows 微信 4.x 账号(会话、消息、联系人、朋友圈、媒体、语音库)做过验证,用的是 **Linux 构建**;交叉编译出的
  Windows `weflow.exe` 也在 Windows 上对同一个账号运行过(约 80 个命令、带媒体的消息导出、图片导出)。其他 Windows 版本没有试过。
- macOS 和 Linux 的微信数据库文件格式相同,但没有测试过。
- 密钥提取辅助程序(`key db`、`key image`)需要微信正在运行,无法离线测试。
- 只用了一个账号的数据验证;特殊的数据库(特别大的分片、旧版本表结构)可能暴露遗漏。

## 5. 可能出乎意料的行为

- **快照**:命令第一次接触某个数据库时,会把它解密到内存里(大的消息分片要几百 MB,缓存上限约 1.5 GB)。长时间运行的
  `serve` 会在文件或其 `-wal` 变化时读到新消息。
- **大会话导出的内存**:`export messages` 按页把会话转成导出记录,再据此写文件。20 万条消息的群,峰值约 0.6 GB(`txt`、`weclone`、`sql`、`excel`、`chatlab`)
  到 0.8 GB(`json`、`arkme-json`、`html`),其中约 200 MB 是解密后的消息数据库。内存紧张时请导出日期范围(`--start/--end`),耗时只与范围大小有关,和日期远近无关。
- **时区**:导出的 `--start/--end` 日期、写进导出文件的时间、`chat dates`、`chat date-counts` 和按日统计都使用本机本地时区(与桌面端一致)。同一份数据库在另一个时区的机器上读取,深夜的消息会归到不同的日子。
- **导出里的媒体**:`export messages --media image,voice,video,emoji`(或 `all`)把文件复制到输出文件旁边的 `media/<输出文件名>/{images,voices,videos,emojis}`,并让消息指向它们。
  哪些格式有位置放媒体:`json`/`arkme-json`、`txt`、`excel`、`weclone`(内容或 `src` 变成相对路径)、`chatlab`(仅图片)、`html`(`<img>`、`<audio>`、`<video>`);`sql` 没有。磁盘上找不到的文件保留占位文字。
  表情需要联网;桌面端用自己的目录布局。
- **中文名称排序**(联系人列表、群成员)遵循 ICU/CLDR 拼音排序,与 `Intl.Collator('zh-CN')` 一致:数字在前,然后是按拼音排列的汉字,最后是拉丁字母。
- **群聊文本**:导出的群消息保留发送者前缀后面的换行(`wxid_xxx:` 被去掉,换行还在),与桌面端导出一致。
- **搜索**覆盖文本、链接/文件、引用回复消息;关键字按字面匹配(不区分大小写),压缩消息会先解码再匹配。图片、语音、表情不参与搜索。
- **折叠 / 免打扰**状态由联系人标志位推断(折叠:第 28 位;免打扰:第 9 位或群通知标志),依据微信的惯例;没有用真实的折叠会话验证过。
- **足迹**把 `@所有人`(`notify@all`)算作 @ 了你;判定为 @ 需要文本里有 `@`,且消息 `atuserlist` 里有你的 id(或 `notify@all`)。
  私聊静默超过 1 小时就切成新的一段。
- **双人报告**的常用语是 2~20 个字、不含链接和标记、至少出现 2 次的完全相同文本;响应时间只统计你在同一轮聊天内的回复
  (间隔超过 1 小时算新一轮)。
- **朋友圈年度统计**统计你自己的帖子、给你点赞最多的好友、以及你点赞最多的好友的帖子。
- 写操作(第 1 节)会被拒绝,只读命令绝不会改动微信数据。

## 6. 保持清单最新

当有返回 *not implemented* 或 *not supported* 的数据库函数没有出现在本文件或 [../cli-unsupported.md](../cli-unsupported.md) 的第 1~2 节里时,
`cargo test -p weflow-native --test unsupported_docs` 会失败。移植一个函数时,在同一次改动里删掉它的那一行;新增限制时,补一行。
