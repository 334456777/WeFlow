# CLAUDE.md

## 命令
 - `make help` 查看构建指南
 - `rust-toolchain.toml` 固定了 Rust 版本（本地与 CI 共用）；升级时改 channel，跑 `make ci` 修掉新增的 clippy 警告，一起提交
 - `target/` 不会自动清理，用 `cargo sweep` 清理过期编译缓存：
   - 精确清理：`cargo sweep --stamp` → 正常编译一次（实际会用到的 profile 都要编译，如 `cargo build`、`cargo build --release`、`cargo test --no-run`）→ `cargo sweep --file`，删除时间戳之后没被用到的产物
   - 日常清理：`cargo sweep --time 30` 删除 30 天没用过的产物；`cargo sweep --installed` 删除已卸载工具链编译的产物
   - 首次执行任何删除命令前先加 `--dry-run` 预览；不要用 `find -delete` 自制清理，也不要为了腾空间直接 `cargo clean`
 - Windows PowerShell 终端上操作时尝试使用常用的 UNIX/Linux 命令行工具

## 结构
仓库由两部分组成：Rust workspace（CLI `weflow`、`weflow serve` 的命令 API 和纯 Rust 只读数据库层）和 WebUI（`web/`）。

### Rust workspace（`crates/`）
依赖方向：`weflow-cli` → `weflow-core` → `weflow-native` / `weflow-silk` / `weflow-assets`
- `weflow-native`：纯 Rust + 内置 SQLite 的只读数据库层，替代闭源 `wcdb_api`。`cipher_vfs`/`sqlcipher` 负责按页解密，`native_*` 按领域（消息、联系人、朋友圈、媒体、统计、报告）查询，`wcdb.rs` 是给上层用的门面，`wxkey` 处理密钥；未移植的调用返回 "not implemented"，必须同步登记到 `docs/cli-unsupported.md`（`make docs-check` 校验；它还会检查文档的相对链接和锚点是否有效、`docs/` 与 `docs/zh-CN/` 是否成对、`native-cli.md` 的 crate 表是否完整、汉字旁的逗号是否为中文逗号（只报告，不自动修改））
- `weflow-core`：业务层。`services/` 下按功能划分（chat、group、analytics、sns、image、voice、insight、reports、cleanup、api），外加 CLI 与命令 API 共用的类型化读取（`reads`，#397）、命令 API 的契约类型（`contract`，#399）、导出（`export*`）、消息推送（`push`）、配置（`config`）、图片/视频/语音解密等
- `weflow-cli`：命令行入口（clap），生成二进制 `weflow`；`help_zh.rs`/`i18n.rs` 为中文帮助与本地化；`serve_api.rs` 与 `serve_api/` 是 `weflow serve` 的命令 API：路由和 `/openapi.json` 由 clap 命令树生成，`in_process` 在服务进程内执行 `/api/` 读取，`jobs`（生命周期，不依赖 axum）与 `job_routes`（HTTP 层）是后台任务（#398），其余路由在子进程里运行命令（见 `docs/serve-api.md`「How it runs」）
- `weflow-silk`：内置 Skype SILK SDK 的语音解码器（`vendor/` 为第三方 C 源码，勿改）
- `weflow-assets`：构建期把 `assets/` 下的文件（各平台的辅助程序、VC 运行库）嵌入二进制，运行时解压到运行时目录，路径与 `assets/` 下相同
- 测试：`crates/weflow-core/tests/*_e2e.rs` 为端到端测试（共享 `tests/common`，用 `weflow-native` 的 `test-fixtures` 合成加密账号）；各 crate 的 `examples/*_probe.rs` 是调研用探针；`fixture::bench`（`weflow-cli/examples/bench_account.rs`）按种子生成与真实账号同规模、同分布的基准账号（#488），分布照 `weflow-core/examples/account_profile_probe.rs` 在真实账号上测得的汇总比例

### WebUI（`web/`）
`weflow serve` 的网页界面（#130），独立的 React + TypeScript + Vite 项目，有自己的 `package.json`。构建产物 `dist/` 由 `serve` 托管在 `/ui/`（#146），与二进制分开发布（#148）。
- `src/api/` 调用命令 API，用 `/health/events` 长连接显示连接状态并检查版本（#221）；`src/mock/` 是未连接服务时显示的示例记录（会话列表、记录浏览和控制台的默认日期范围），不是暂用的数据
- `bun run dev`（`scripts/dev.mjs`，#333；依赖由 Bun 按 `bun.lock` 安装；测试用 `bun test` 加 happy-dom，`test/` 下是测试 API 和 preload，不需要 Node.js）同时启动 Vite 和一个读合成账号的 `weflow serve`（`crates/weflow-cli/examples/dev_account.rs` 生成在 `target/dev-account/`，含「图片测试」会话），随机令牌由代理加上，不编进网页；设了 `WEFLOW_SERVE_URL` 时只启动 Vite，转给那个 serve；设 `WEFLOW_DEV_ACCOUNT=bench` 时改用基准账号（#488）
- `package.json` 的 `weflow.apiVersion` / `minBackendVersion` 声明需要的 API，构建时写入 `dist/weflow-web.json`

### 其他
- `docs/`：英文文档，`docs/zh-CN/` 为对应中文版
- `.github/workflows/` 下的 `release.yml`（推送 tag 或在 Actions 页面手动运行并填写 tag 时触发，tag 不存在时由 github-actions[bot] 在所选分支的最新提交上创建附注 tag，已有的轻量 tag 会被拒绝；依次 build → test → release：构建 6 个平台的发布版（PGO，#500：`.github/scripts/pgo-build.sh` 在各平台自己的 runner 上先编插桩版，在现场生成的合成账号上跑各格式导出和主要读取，再用得到的 profile 编发布版），在每个平台上跑测试并核对包里 `weflow --version` 与 tag 一致，全部通过才发布；正式版 tag 与根 `Cargo.toml` 的版本号不一致时只给出警告，不拦截；发布说明来自 `.github/release-notes.md`，开头的「更新日志」由 `.github/scripts/release-changelog.sh` 生成：手动运行时填写的 notes，加上与上一个版本相比关闭的 issue（按标签分组，不含文档类）、没有关联 issue 的 PR、新登记的 issue；手动运行时勾选 `bench` 会在发布之后调用 `bench.yml`，`bench_base` 留空时与上一个版本对比（正式版对比上一个正式版），报告发布到 GitHub Pages，链接和几行结论补进更新日志「性能基准」一节，#516）和 `ci.yml`（代码变动时只测三个主流平台：Linux、macOS 运行 `make ci`（rustfmt、clippy、全部测试），外加 `make docs-check`；Windows x64 构建发布版、跑测试并运行一次）和 `docs-check.yml`（仅文档变动时运行 `make docs-check`）和 `web.yml`（`web/` 变动时类型检查、测试、构建并上传 `dist/`）和 `web-release.yml`（推送 `web-v<版本>` tag 或手动运行：版本须与 `web/package.json` 一致，构建 `web/` 后打成 zip，用 ORAS 发布到 `ghcr.io/334456777/weflow-web:<版本>`，manifest 注明 API 版本和最低 weflow 版本；已发布的版本拒绝覆盖，`weflow web` / `serve` 从这里下载）和 `bench.yml`（基准测试，#259：只能在 Actions 页面手动运行，填写 base 和 head 两个提交（commit、分支或 tag），可选运行次数、基准账号的种子与规模、持续负载秒数、`only` 只测部分项目；两个版本都编译 release（不带发布版的 PGO，差别只来自代码），在同一台 Windows runner 上轮流测量，账号由 `bench_account` 在 runner 上现场生成，不用真实数据；测量由 `.github/scripts/bench.py` 完成：最耗时的命令和消息读取的耗时、首批结果时间、CPU、内存曲线与峰值、I/O、线程数，分配次数、累计分配字节与峰值存活堆（另编一份带 `weflow-cli` 的 `alloc-stats` feature 的二进制单独跑，#513），`weflow serve` 持续请求下的延迟、吞吐和内存是否稳定，`serve_bench` 各路由延迟，以及二进制体积；结果和两个二进制作为 artifact `bench-results` 上传，汇总表和内存曲线（Mermaid 折线图）显示在运行页面上，填了 `pages_path`（如 `bench/<名字>`）时报告还会推到 `gh-pages` 分支，由 GitHub Pages 在 `https://334456777.github.io/WeFlow/bench/` 下展示（`gh-pages` 分支不能当作已合并分支删掉），只报告数字，head 变慢也不会失败；大改动后对性能有疑问时才运行，小修改不用）是实际生效的 CI


## 约定
- 此 git 仓库所连接的远程 GitHub 仓库是公开仓库（public），GitHub Project「WeFlow」是公开 Project
- 提交信息使用 Conventional Commits 规范，默认使用英语；Pull Request 分支名称使用 Conventional Commits 风格的 `<type>/<scope>-<description>` 格式。
- 版本与 tag 规范：版本号遵循语义化版本控制（SemVer）`vMAJOR.MINOR.PATCH`，预发布版本用 `-rc.1` 后缀；tag 一律用附注 tag，不用轻量 tag（`git tag -a vX.Y.Z -m "WeFlow vX.Y.Z" <commit>`，用 `git cat-file -t vX.Y.Z` 检查，应输出 `tag`）；发布前先查看现有 tag 和上次发布以来的提交，判断该升哪一位并告诉我理由，我确认后再打 tag。
- 严禁将微信聊天记录，微信wxid，隐私信息，本地路径，密钥，API Key 等等隐私信息上传到公开的 GitHub 仓库或写入 CLAUDE.md
- GitHub Issues 根据 Lables 分类，筛选和表达状态，根据 Relationships 梳理 Issue 依赖关系（Issue dependencies），创建 Issue 时及时添加 Lables 和 Relationships （如果有对应的 Development 则添加对应 Development）
- GitHub Issues 会有人类和其他大语言模型（LLM）参与讨论，以下情况记录到本仓库的 GitHub Issues: 被列为计划内的工作，需要多轮确认的bug，不符合当前工作的主题但是需要解决的bug，可以稍后解决的bug，被要求将不会被开展的工作，需要多轮和长时间研究思考的精益求精的要求，工作中遇到的反常/不太对劲的地方，新功能或请求，对文档的改进或补充，一些在修改代码后会经常出现的问题和需要更多信息的问题以及任何你想提出的issues；同时在遇到上述情况时也可以查询Issues寻找解决方法或继续跟踪评论issues
- GitHub Project「WeFlow」（`gh project` 编号 3，owner 为仓库所有者，需要 token 带 `project` 权限，仓库内相对路径与公开的 owner 名可以出现）是看板，所有 Issue 和 PR（开着的和关着的）都要放进去，并随工作进展及时更新 Status：
  - `Backlog` 已记录但尚未排期；`Todo` 已排期待开始；`In Progress` 正在进行；`In Review` PR 已提交等待审查；`Blocked` 被依赖或外部因素阻塞（同时用 Relationships 标出阻塞来源）；`Done` 已关闭或已合并
  - 新建 Issue 或 PR 后立即加入看板并设置 Status 与 `Priority`（`P0` 紧急，立即处理；`P1` 高；`P2` 普通；`P3` 低，可随时搁置）；PR 打开时设为 `In Review`，合并或关闭后与其关联的 Issue 一起设为 `Done`
  - 用 `gh project item-add` 加入条目，用 `gh project item-edit` 改字段；重写 Status 选项会清空所有条目的状态，改选项后必须重新设置
- Pull request 创建时及时添加 Lables（如果有对应 Development 则添加对应 Development）；PR关闭时PR描述或 commit message 里用 `closed` `fixed` `resolved` 关键词关闭关联 （只有这几个关键词是英文，PR描述其他部分仍然是中文）Issues；PR 标题和 PR description（PR body）必须使用中文，除非用户明确要求英文，否则不要使用英文撰写 PR description
- 用户可能会出错，当用户出现明显错误或决策冲突时，请指出
- 对于需要多个依赖 PR 的开发任务，请使用 `gh stack`
- 修改 `docs/` 下的文档时，必须同步更新 `docs/zh-CN/` 中的对应版本（反之亦然），新增文档也要两种语言都提供

## 注意事项
- 默认用中文回复，代码注释用英文
