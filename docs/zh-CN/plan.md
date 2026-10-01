# 计划:单文件原生命令行

[English](../plan.md) | **简体中文**

原生命令行所依据的计划、已经完成的部分,以及还需要验证的部分。原来放在仓库根目录的 `PLAN.md`,现移到这里。

## 目标

把 WeFlow 的后端重写成 **Rust 原生命令行**,每个平台发布一个可执行文件(Windows x64/arm64、macOS arm64、Linux x64),
保留完整的后端能力:密钥提取、图片解密、朋友圈、导出、统计分析、HTTP API、AI 见解。

- "单一可执行文件"指每个平台、每种架构各一个二进制,而不是一个文件跑所有系统。
- 无法重写的平台辅助程序(`wx_key.dll`、`img_helper.dll`、`libwx_key.dylib`、`xkey_helper_linux`、WASM 解码器)内嵌在程序里,
  首次运行时解压到带版本号的缓存目录,再从那里加载。
- TypeScript 服务是移植的参考,也是回归对比的基准。

**计划变更(2026-10):** 数据库层原计划通过 FFI 调用闭源的 `wcdb_api` 库。这个库带有效期检查(2026-09-30 23:59:59 之后
`wcdb_init` 返回 `-1000`)和未经核实的网络代码,所以命令行改为自己解密微信 4.x 数据库(SQLCipher 4),用纯 Rust **只读**
读取(`crates/weflow-native/src/{sqlcipher,native_*}.rs`)。命令行和桌面端都不再内嵌或加载 `wcdb_api`、`WCDB.dll`、
`libwcdb_api.*`、`libWCDB.dylib`;它们留在仓库里给原版桌面端使用(包括原作者提供的新版 `wcdb_api.dll`),见 [wcdb-api.md](wcdb-api.md)。

## 结构

| Crate | 作用 |
|---|---|
| `crates/weflow-cli` | 命令入口、参数解析、输出协议 |
| `crates/weflow-core` | 配置、账号、聊天、导出、统计分析、朋友圈、备份、AI 见解、HTTP API |
| `crates/weflow-native` | 原生数据库读取(SQLCipher 解密、消息、联系人、朋友圈、统计、报告)、密钥辅助、图片解密、WASM、平台封装 |
| `crates/weflow-assets` | 内嵌资源、解压、哈希校验 |
| `crates/weflow-silk` | 内置的 SILK 解码器,用于语音消息 |

使用的库:`clap`、`serde`/`serde_json`、`tokio`/`axum`、`libloading`(只用于平台辅助库),Excel、CSV、压缩和文件处理用对应的 Rust crate。

资源:每个二进制只内嵌本平台需要的辅助程序,解压到 `WEFLOW_HOME/runtime/<版本>/<target>/`;每次启动校验清单里的哈希(版本或哈希
不一致时重新解压);动态库只从这个目录加载,不会隐式从当前目录加载。

配置:`WEFLOW_HOME`,否则是平台配置目录下的 `weflow`;配置文件 `config.json`(也接受 TOML);缓存、日志、运行时分目录存放。
`weflow config import` 迁移桌面端可读的设置,加密的 `safe:` / `lock:` 字段会跳过并提示重新设置。

## 命令行约定

- stdout 只输出一个 JSON 文档:`{ "success": true, "data": ..., "meta": ... }` 或
  `{ "success": false, "error": { "code": "...", "message": "...", "details": ... } }`。
- 全局选项:`--config`、`--profile`、`--db-path`、`--decrypt-key`、`--wxid`、`--lang`、`--json`(默认)、`--pretty`、
  `--progress`(在 stderr 输出 NDJSON)、`--no-progress`、`--progress-delay`。
- 退出码:`0` 成功,`1` 运行错误,`2` 参数错误,`3` 配置/密钥错误,`4` 数据库/原生库错误,`130` 用户中断。
- 命令列表见 [native-cli.md](native-cli.md)。

## 进度

| 步骤 | 状态 |
|---|---|
| 基础:配置、JSON 输出、错误码、日志、运行时解压 | 已完成 |
| 数据库层 | 已完成,改为原生 Rust 只读实现,不再走 WCDB FFI;写操作被拒绝([cli-unsupported.md](cli-unsupported.md)) |
| 会话、消息、搜索、联系人 | 已完成 |
| 导出:JSON、HTML、TXT、Excel、WeClone、SQL、ChatLab、arkme-json、媒体 | 已完成(9 种格式,`--media` 内嵌媒体) |
| 统计分析:私聊和群聊统计、年度报告、双人报告、足迹 | 已完成 |
| 媒体:图片 `.dat` 解密、视频查找、语音解码、表情 | 已完成;语音转写不打算做 |
| 朋友圈、公众号、备份、HTTP API、消息推送、AI 见解 | 已完成 |
| 平台辅助:Windows `wx_key.dll`、`img_helper.dll`;macOS `libwx_key.dylib`;Linux `xkey_helper_linux` | 已接入;只有 Windows 用真实账号跑过 |
| 各平台发布构建 | Windows x64 可构建为单个 `weflow.exe`;GitHub Actions 工作流放在 `.github/weflow/` 下,不会运行 |
| 移除 Electron/React | 没有做:桌面端保留,通过 `weflow_wcdb` 加载同一个 Rust 层([desktop-rust-layer.md](desktop-rust-layer.md)) |

对原后端的覆盖率见 [cli-coverage.md](cli-coverage.md)。

## 测试

- 各 crate 的单元测试:配置和旧配置导入、内嵌运行时清单、SQLCipher(加解密往返、错误密钥、页被篡改、WAL 合并)、图片和朋友圈解密、导出格式。
- 端到端测试用合成的加密账号(`weflow_native::fixture`:SQLCipher 页、WAL、zstd、SILK)和本地假 HTTP 服务器,位于
  `crates/weflow-core/tests/`。`cargo test --workspace` 会全部运行。
- 真实数据回归:一个 Windows 微信 4.x 账号,用 Linux 构建,以及在 Windows 上运行 `weflow.exe`(约 80 个命令、带媒体的消息导出、
  图片导出、HTTP API)。

## 还需要验证的部分

这里记录还没有用真实数据验证过的部分,以及打算怎么验证。每完成一项,就在这里记下结果,并同步更新
[cli-unsupported.md](cli-unsupported.md) 第 4 节。

| # | 项目 | 状态 | 验证方法 | 完成标准 |
|---|---|---|---|---|
| 1 | **macOS 和 Linux 真实账号** | 计划中 | 在各平台上:`key db`(或对应平台的密钥辅助程序)、`db detect`/`db test`,然后跑回归脚本(会话、消息、联系人、朋友圈、报告、每种导出格式加 `--media`、`export media`、HTTP API),并与桌面端对比数量。 | 两个平台的回归都通过、没有意外失败;差异要么修复,要么写进 cli-unsupported.md。 |
| 2a | **Windows 图片自动下载钩子**(`image auto-download start`、`serve --image-auto-download`) | 计划中 | 在运行着微信的 Windows x64 上:启动钩子,打开含有从未下载过图片的聊天,确认文件出现在 `msg/attach/…/Img` 下,之后 `export media` 能找到它们;停止钩子,确认微信工作正常。 | 钩子运行期间图片会被下载,停止后不再有动作,微信不受影响。 |
| 2b | **AI 见解对接真实服务商**(`insight test`、`insight trigger`、`serve --insight`、足迹总结) | 计划中 | 为一个 OpenAI 兼容的服务商配置 `ai_model_api_base_url`、`ai_model_api_key`、`ai_model_api_model`;运行 `insight test`、手动触发和足迹总结;检查请求(`/chat/completions`,没有多余的 `/v1`)、解析出的回答和保存的记录;可选验证 Telegram 推送。 | 所有见解命令在一个真实服务商上端到端可用;服务商返回的错误能清楚地报出来。 |
| 3 | **备份与桌面端是否兼容** | 计划中 | 分别用桌面端和 `weflow backup create` 生成备份;用 `weflow backup inspect` 查看两者;用另一个工具把各自的备份恢复到空目录,比较文件列表和哈希;用 `db test` 和桌面端打开恢复出的账号。 | 两个方向恢复出的文件一致,或者差异有记录并说明原因。 |
| 4 | **桌面端在 Rust 层上运行,包括界面**([desktop-rust-layer.md](desktop-rust-layer.md)) | 计划中 | 在 Windows、macOS、Linux 上用 `npm run build` 构建;打开账号,浏览聊天、联系人、群、朋友圈,运行各个报告和导出,观察新消息能否到达(监听管道),尝试编辑/删除(应出现只读错误)。 | 三个平台上只读功能都可用;差异要么修复,要么写进 [desktop-rust-layer.md](desktop-rust-layer.md)。 |

有了发布构建之后还要检查:下载的可执行文件能单独运行,首次运行会生成运行时缓存,缓存被删后能自动恢复,不需要 Node、npm 或 Electron。
