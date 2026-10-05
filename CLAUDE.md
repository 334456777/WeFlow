# CLAUDE.md

## 命令
 - `make help` 查看构建指南
 - `rust-toolchain.toml` 固定了 Rust 版本（本地与 CI 共用）；升级时改 channel，跑 `make ci` 修掉新增的 clippy 警告，一起提交

## 结构
仓库由两部分组成：Rust workspace（CLI + 纯 Rust 只读数据库层）和 Electron 桌面端。两者共用 `weflow-native` 读取微信数据库。

### Rust workspace（`crates/`）
依赖方向：`weflow-cli` → `weflow-core` → `weflow-native` / `weflow-silk` / `weflow-assets`；`weflow-wcdb-ffi` → `weflow-native`
- `weflow-native`：纯 Rust + 内置 SQLite 的只读数据库层，替代闭源 `wcdb_api`。`cipher_vfs`/`sqlcipher` 负责按页解密，`native_*` 按领域（消息、联系人、朋友圈、媒体、统计、报告）查询，`wcdb.rs` 是给上层用的门面，`wxkey` 处理密钥；未移植的调用返回 "not implemented"，必须同步登记到 `docs/cli-unsupported.md`（`make docs-check` 校验；它还会检查文档的相对链接和锚点是否有效、`docs/` 与 `docs/zh-CN/` 是否成对、`native-cli.md` 的 crate 表是否完整、汉字旁的逗号是否为中文逗号（只报告，不自动修改））
- `weflow-core`：业务层。`services/` 下按功能划分（chat、group、analytics、sns、image、voice、insight、reports、cleanup、api），外加导出（`export*`）、HTTP API（`http_server`）、消息推送（`push`）、配置（`config`）、图片/视频/语音解密等
- `weflow-cli`：命令行入口（clap），生成二进制 `weflow`；`help_zh.rs`/`i18n.rs` 为中文帮助与本地化
- `weflow-wcdb-ffi`：把 `weflow-native` 导出为与 `wcdb_api` 兼容的 C ABI 动态库（`weflow_wcdb`），供桌面端加载，详见 `docs/desktop-rust-layer.md`
- `weflow-silk`：内置 Skype SILK SDK 的语音解码器（`vendor/` 为第三方 C 源码，勿改）
- `weflow-assets`：构建期把静态资源嵌入二进制
- 测试：`crates/weflow-core/tests/*_e2e.rs` 为端到端测试（共享 `tests/common`，用 `weflow-native` 的 `test-fixtures` 合成加密账号）；各 crate 的 `examples/*_probe.rs` 是调研用探针

### Electron 桌面端
- `electron/`：主进程。`main.ts` 入口，`preload.ts` 暴露 IPC，`services/` 为各功能服务，`*Worker.ts` 为后台 worker；`services/wcdbCore.ts` 通过 koffi 加载 `resources/native-db/` 下的 `weflow_wcdb`
- `src/`：渲染进程（React + Vite + Zustand）。`pages/` 页面、`components/` 组件、`stores/` 状态、`services/ipc.ts` 调用主进程、`i18n/` 多语言
- `resources/`：随包分发的原生库与辅助程序（密钥/图片解密 helper、旧版 `wcdb` 库、字体等）；`public/` 为前端静态资源
- `scripts/`：构建脚本（`build-native-db.cjs` 编译 FFI 库并复制到 `resources/native-db/`，`i18n-check.cjs` 等）；`scripts/research/` 为调研探针

### 其他
- `docs/`：英文文档，`docs/zh-CN/` 为对应中文版；`docs/weflow-tech-docs/` 为微信数据库/密钥/媒体格式的技术资料
- `.github/workflows/` 下的 `release-cli.yml`（打 tag 时构建发布）和 `ci.yml`（代码变动时运行 `make ci`：rustfmt、clippy、全部测试，外加 `make docs-check`）和 `docs-check.yml`（仅文档变动时运行 `make docs-check`）是实际生效的 CI；`.github/weflow/` 是上游桌面端的 GitHub 配置副本，不会被触发


## 约定
- 此 git 仓库所连接的远程 GitHub 仓库是公开仓库（public）
- 严禁将微信聊天记录，微信wxid，隐私信息，本地路径，密钥，API Key 等等重要信息上传到公开的GitHub仓库或写入 CLAUDE.md
- GitHub Issues会有人类和其他大语言模型（LLM）参与讨论，以下情况记录到本仓库的GitHub Issues中: 被列为计划内的工作，需要多轮确认的bug，不符合当前工作的主题但是需要解决的bug，可以稍后解决的bug，被要求将不会被开展的工作，需要多轮和长时间研究思考的精益求精的要求，工作中遇到的反常/不太对劲的地方，新功能或请求，对文档的改进或补充，一些在修改代码后会经常出现的问题和需要更多信息的问题；同时在遇到上述情况时也可以查询Issues寻找解决方法。
- 通过 GitHub Issue 创建的 Pull request 要在背景里提到原 Issue
- 修改 `docs/` 下的文档时，必须同步更新 `docs/zh-CN/` 中的对应版本（反之亦然），新增文档也要两种语言都提供；`docs/weflow-tech-docs/` 和 `docs/es-ES/` 不受此限
- 提交信息格式使用Conventional Commits 规范，默认英语提交
- 创建 Pull Request 时，PR 标题和 PR description（PR body）必须使用中文，除非用户明确要求英文，否则不要使用英文撰写 PR description
- 中文附近的逗号要用中文逗号

## 注意事项
- 默认用中文回复，代码注释用英文
