# WeFlow 单文件原生 CLI 改造计划

> **状态更新(2026-10)**:数据库层已不再通过 FFI 复用闭源的 `wcdb_api`(它带有到期检查和未经核实的网络代码),改为纯 Rust 原生实现:自己解密微信 4.x 数据库(SQLCipher 4 布局)并以只读方式读取,见 `crates/weflow-native/src/{sqlcipher,native_*}.rs`。下文中涉及 `wcdb_api` / `WCDB.dll` / WCDB FFI 的条目保留为原始计划,已被此更新取代;仍使用 FFI/动态加载的只有 `wx_key.dll`(密钥提取)、`img_helper.dll`(图片钩子)等平台辅助库。不支持的功能清单见 `docs/cli-unsupported.md`。

## Summary
将仓库重构为 **Rust 原生 CLI**，最终每个平台发布一个单一可执行文件：Windows x64/arm64、macOS arm64、Linux x64。运行时允许首次自解压内嵌 native/WASM/helper 资源到版本化缓存目录，再通过 FFI 动态加载(数据库读取除外,已改为原生 Rust),从而保留密钥提取、图片解密、朋友圈、导出、分析、HTTP API 等完整后端能力。

## Key Changes
- 使用 Rust 替换 Electron/React/Node：
  - CLI 框架：`clap`
  - JSON/配置：`serde`、`serde_json`
  - 异步/HTTP：`tokio`、`axum`
  - 动态库加载：`libloading`(仅用于 `wx_key.dll`、`img_helper.dll` 等平台辅助库;数据库层不使用)
  - Excel/CSV/压缩/文件处理选用 Rust crate 等价实现
- 删除前端与 JS 运行时：
  - 移除 React/Vite/Electron/preload/IPC/window/update 相关代码和依赖
  - 保留现有 TypeScript 服务作为迁移参考，完成 parity 后移除
- 新增 Rust 工程结构：
  - `crates/weflow-cli`：命令入口、输出协议、参数解析
  - `crates/weflow-core`：配置、账号、聊天、导出、分析、朋友圈、备份、AI 见解
  - `crates/weflow-native`：原生数据库读取(SQLCipher 解密、消息/联系人/朋友圈/统计/报告)、key helper、wedecrypt、WASM、平台能力封装
  - `crates/weflow-assets`：编译期嵌入资源、自解压、hash 校验
- 单文件资源策略：
  - 每个目标平台的二进制只嵌入本平台需要的 `resources/`、`wasm_video_decode.*`、helper/native 库
  - 首次运行解压到 `WEFLOW_HOME/runtime/<version>/<target>/`
  - 每次启动校验 manifest hash；版本变更或 hash 不一致时重新解压
  - 动态库只从该受控目录加载，不从当前目录隐式加载
- 配置策略：
  - 默认目录：`WEFLOW_HOME`，否则平台配置目录下 `weflow`
  - 配置文件：`config.toml` 或 `config.json`，缓存、日志、运行时资源分目录存放
  - 提供 `weflow config import` 迁移旧 Electron 可读配置；`safe:`/`lock:` 加密字段跳过并提示重新设置

## CLI Contract
- 所有命令默认 stdout 输出统一 JSON：
  - 成功：`{ "success": true, "data": ..., "meta": ... }`
  - 失败：`{ "success": false, "error": { "code": "...", "message": "...", "details": ... } }`
- 全局参数：
  - `--config <path>`、`--profile <name>`、`--db-path <path>`、`--decrypt-key <hex>`、`--wxid <id>`
  - `--json` 默认开启，`--pretty` 输出人类可读表格，`--progress` 向 stderr 输出 NDJSON 进度
- 命令分组：
  - `weflow config list|get|set|unset|clear|import`
  - `weflow db detect|scan|test|open`
  - `weflow key db|image|scan-image`
  - `weflow chat sessions|messages|latest|search|contacts|contact|update-message|delete-message`
  - `weflow chat anti-revoke check|install|uninstall`
  - `weflow export sessions|contacts|footprint`
  - `weflow analytics overall|rankings|time|excluded`
  - `weflow group list|members|ranking|hours|media|member|export-*`
  - `weflow report annual years|generate`
  - `weflow report dual generate`
  - `weflow sns timeline|users|stats|export|download-image|block-delete|delete`
  - `weflow biz accounts|messages|pay-records`
  - `weflow insight test|records|get|mark-read|clear|trigger|footprint`
  - `weflow serve --http --message-push --insight --image-auto-download`
- 退出码：
  - `0` 成功
  - `1` 运行错误
  - `2` 参数错误
  - `3` 配置/密钥错误
  - `4` 数据库/native 加载错误
  - `130` 用户中断

## Implementation Plan
- 先实现 Rust native 基座：
  - 配置系统、统一 JSON 输出、错误码、日志、运行时资源自解压
  - ~~WCDB FFI 绑定~~ → 原生 Rust 数据库读取器(`wcdb.rs` 保留原接口签名,未移植的函数明确报错,见 `docs/cli-unsupported.md`)
  - 连接数据库、获取会话、获取消息、搜索、联系人作为第一批端到端链路
- 再迁移完整业务：
  - 导出：JSON、HTML、TXT、Excel、CSV/WeClone、SQL、ChatLab、媒体导出
  - 分析：私聊统计、群聊统计、年度报告、双人报告、足迹
  - 媒体：图片 `.dat` 解密、视频定位、语音读取/转写、表情下载
  - 朋友圈、公众号、备份、HTTP API、消息推送、AI 见解
- 平台能力：
  - Windows：复用 `wx_key.dll`、`img_helper.dll`(不再使用 `wcdb_api.dll`、`WCDB.dll`)
  - macOS：复用 `libwx_key.dylib`、helper(不再使用 `libwcdb_api.dylib`、`libWCDB.dylib`)
  - Linux：复用 `xkey_helper_linux`(不再使用 `libwcdb_api.so`)
- 构建发布：
  - 使用 GitHub Actions matrix 构建 `weflow-{target}` 单文件
  - 产物命名：`weflow-windows-x64.exe`、`weflow-macos-arm64`、`weflow-linux-x64`
  - 每个产物内置 manifest：版本、target、资源 hash、构建 commit

## Test Plan
- 单元测试：
  - 配置读写、旧配置导入、路径解析、资源解压、hash 校验、错误 JSON
- 数据库层测试(替代原 FFI 测试)：
  - 加解密往返、错误密钥、页篡改、WAL 合并(单元测试);合成加密账号夹具上的端到端测试(`weflow_native::fixture`、`crates/weflow-core/tests/native_e2e.rs`)
  - wedecrypt 图片解密、WASM 文件加载、key helper 路径解析
- CLI 冒烟：
  - `weflow --version`
  - `weflow config set/get`
  - `weflow db detect`
  - `weflow db test`
  - `weflow chat sessions --limit 5`
  - `weflow chat messages <session> --limit 20`
  - `weflow export sessions --format json --out <dir>`
- 真实数据回归：
  - 用同一微信数据目录对比旧 Electron 版和 Rust CLI 输出的会话数、消息数、联系人数、导出文件数量
- 发布验证：
  - 每个平台下载单个可执行文件即可运行
  - 首次运行生成 runtime 缓存
  - 删除 runtime 缓存后可自动恢复
  - 不依赖 Node、npm、Electron、Vite、React

## Assumptions
- “单一可执行文件”表示每个平台/架构一个独立二进制，不是一个文件同时跑所有 OS。
- 允许运行时把 native 库、WASM、helper 解压到用户缓存目录；否则现有 dylib/dll/.node/helper 能力无法完整保留(数据库读取已不需要 native 库)。
- 最终版本不包含 Node/Electron/React 运行时；旧 TypeScript 只作为迁移参考和回归对照。
