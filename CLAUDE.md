# CLAUDE.md

本文件写给在这个仓库里工作的编码智能体；[`AGENTS.md`](AGENTS.md) 是指向它的符号链接，改动只写在本文件里。设计和用法的细节在 [Wiki](https://github.com/334456777/WeFlow/wiki)（见下面的 [Wiki](#wikiwiki) 一节），改代码前先对照[开发指南-常见改动的检查清单](https://github.com/334456777/WeFlow/wiki/开发指南#常见改动的检查清单)。开 PR、发版、真实账号验证等常见流程的步骤写成 skill，见下面的 [Skills](#skillsagentsskills) 一节。

## 命令
Linux、macOS、Windows 都可以开发。命令里的 `VAR=1 命令` 是 POSIX shell 的写法，PowerShell 里写 `$env:VAR=1; 命令`；[`Makefile`](Makefile) 在 Windows 上要在 Git Bash 或 WSL 里运行。

### Rust
- 构建与检查：`make help` 列出全部目标，各目标的作用见[开发指南-构建命令](https://github.com/334456777/WeFlow/wiki/开发指南#构建命令)
  - `make build`、`make test`（`cargo test --workspace`）、`make fmt`、`make rustdoc`（rustdoc 的警告视为错误）
  - `make lint` 即 `cargo clippy --workspace --all-targets -- -D warnings`；`cargo check` 不报 clippy 的警告
  - `make ci` 依次跑 fmt-check、lint、rustdoc、test，与 CI 在 Linux、macOS 上跑的相同；CI 在 Windows 上另做 release 构建、`cargo doc` 和 `cargo test --workspace`（见[发布与 CI-工作流](https://github.com/334456777/WeFlow/wiki/发布与-CI#工作流)）
- 单个测试：
  - 端到端测试：`cargo test -p weflow-core --test chat_e2e <测试名>`
  - 命令 API 的服务测试：`cargo test -p weflow-cli --test serve_api <测试名>`
  - 单元测试：`cargo test -p weflow-native --lib <测试名>`
- 工作区的 lint（根 [`Cargo.toml`](Cargo.toml) 的 `[workspace.lints]`）在本地是警告，在 CI 里是错误，写法见[开发指南-注释与文档](https://github.com/334456777/WeFlow/wiki/开发指南#注释与文档)：
  - 每个公开项都要有 `///`，每个 crate 根都要有 `//!`（`missing_docs`，[#915](https://github.com/334456777/WeFlow/issues/915)）；集成测试、示例和构建脚本各自也是一个 crate
  - `unwrap`、`expect` 只在测试里允许（[`clippy.toml`](clippy.toml)）
  - 每个 `unsafe` 块前写 `// SAFETY:`；`unsafe fn` 里的不安全操作也各写在自己的 `unsafe` 块里（`unsafe_op_in_unsafe_fn`）
  - 每个公开类型都实现 `Debug`（`missing_debug_implementations`）；含密钥、令牌的类型不派生，手写只显示有没有，参照 `config.rs` 的 `ProfileConfig`
  - 放过某条警告用带 `reason` 的 `#[expect]`；警告随平台时有时无、`expect` 会落空时才用带 `reason` 的 `#[allow]`
- 只在部分平台编译的代码：开头写了 `#![cfg(...)]` 的文件在别的平台上不编译，如 [`insight_e2e.rs`](crates/weflow-core/tests/insight_e2e.rs) 只在 Linux 编译。改了这类文件用到的公开接口，或者测试依赖 `cfg!(windows)`、反斜杠路径、符号链接时，在对应的平台上（Windows 上用 WSL）跑一次相关的 clippy 和测试
- `make docs-check`（`cargo test -p weflow-native --test docs_consistency`）检查两份 README、skill 和本文件的相对链接和锚点、汉字旁的逗号是否为中文逗号（只报告，不自动修改）、skill 的格式，以及 `AGENTS.md`、`.claude/skills` 是否检出为符号链接
- [`rust-toolchain.toml`](rust-toolchain.toml) 固定了 Rust 版本（本地与 CI 共用）
- `target/` 不会自动清理：用 `cargo sweep` 清理过期的编译缓存，不为了腾空间直接 `cargo clean`

### WebUI
在 `web/` 下运行，脚本的完整列表见 [`web/README.zh-CN.md`](web/README.zh-CN.md#脚本)。
- `bun install --frozen-lockfile` 按 `bun.lock` 安装依赖：只需要 Bun（版本见 `package.json` 的 `packageManager`），不需要 Node.js；[`bunfig.toml`](web/bunfig.toml) 只接受发布满 3 天的版本
- `bun run check` 依次检查契约类型、英文、导出项的 JSDoc（`src/` 的每个导出项都要有，[#915](https://github.com/334456777/WeFlow/issues/915)），再做类型检查、测试和构建；CI 另外把契约与最新发布版对比，并跑 `bun run smoke`
- 单个测试：`bun test <文件>`（happy-dom 由 `bunfig.toml` 的 preload 加载，`test/` 下是测试 API）
- 开发服务器（`bun run dev`）和用生产构建做的检查（`bun run smoke`、`bun run shots`）见下面的 [WebUI](#webuiweb) 一节

## 结构
仓库由两部分组成：Rust workspace（CLI `weflow`、`weflow serve` 的命令 API 和纯 Rust 只读数据库层）和 WebUI（`web/`）；模块之间的关系见 Wiki 的[架构](https://github.com/334456777/WeFlow/wiki/架构)。

### Rust workspace（`crates/`）
依赖方向：`weflow-cli` → `weflow-core` → `weflow-native` / `weflow-silk`
- [`weflow-native`](crates/weflow-native)：纯 Rust + 内置 SQLite 的只读数据库层，替代闭源 `wcdb_api`
  - `cipher_vfs`/`sqlcipher` 负责按页解密，`db` 是账号句柄（`Account`），`wxkey` 处理密钥
  - `msg`、`contact`、`sns`、`media`、`stats`、`report` 按领域（消息、联系人、朋友圈、媒体、统计、报告）查询，[`wcdb.rs`](crates/weflow-native/src/wcdb.rs) 是给上层用的门面，每个调用都是带类型参数的方法
- [`weflow-core`](crates/weflow-core)：业务层
  - [`services/`](crates/weflow-core/src/services) 下按领域划分，每个领域一个 `impl ServiceHub`
  - `reads`：CLI 与命令 API 共用的类型化读取（[#397](https://github.com/334456777/WeFlow/issues/397)）
  - `contract/`：命令 API 的契约类型，按领域划分（[#399](https://github.com/334456777/WeFlow/issues/399)），见下面的[命令 API 的契约](#命令-api-的契约)
  - 导出（`export*`）：`message/` 按消息种类解析，`export_msg/` 每种格式一个模块
    - HTML 导出的页面模板是 [`export_msg/`](crates/weflow-core/src/export_msg) 下的 `template.html`（聊天记录）、`list.html`（会话列表、联系人）和 `moments.html`（朋友圈），三者共用的配色在 `colours.css`
    - 嵌入程序的 `<模板>.{zh,en}.html` 由 `build.mjs` 生成，CI（[`html-template.yml`](.github/workflows/html-template.yml)）在推送后生成并提交到该分支
  - 另有消息推送（`push`）、见解的模型接口（`model_api`，OpenAI 与 Anthropic 两种格式）、配置（`config`）、图片/视频/语音解密等
- [`weflow-cli`](crates/weflow-cli)：命令行入口（clap），生成二进制 `weflow`
  - [`main.rs`](crates/weflow-cli/src/main.rs) 只是入口（分配器、`run` 的分派、结果汇总和退出码），`cli.rs` 是顶层的 `Cli`、`Commands`，`commands/` 每个子命令一个文件（命令定义和执行它的函数放在一起）
  - `reads.rs` 是命令行与 `serve` 共用的读取映射（`read_of`、`media_of`），`term.rs`/`files.rs` 是终端输出和写文件，`help_zh.rs`/`i18n.rs` 为中文帮助与本地化
  - [`serve_api.rs`](crates/weflow-cli/src/serve_api.rs) 与 [`serve_api/`](crates/weflow-cli/src/serve_api) 是 `weflow serve` 的命令 API（见 Wiki 的[服务端实现](https://github.com/334456777/WeFlow/wiki/服务端实现)）：
    - 路由和 `/openapi.json` 由 clap 命令树生成（`endpoints`，方法表和拒绝的命令在 `rules`）
    - `in_process` 在服务进程内执行 `/api/` 读取
    - `jobs`（生命周期，不依赖 axum）与 `job_routes`（HTTP 层）是后台任务（[#398](https://github.com/334456777/WeFlow/issues/398)）
    - 其余路由由 `child` 在子进程里运行命令（见[子进程路由](https://github.com/334456777/WeFlow/wiki/服务端实现#子进程路由)）
- [`weflow-silk`](crates/weflow-silk)：内置 Skype SILK SDK 的语音解码器（`vendor/` 为第三方 C 源码，勿改）
- [`weflow-fake-model`](crates/weflow-fake-model)：只用于测试和开发的假模型服务（不随 `weflow` 发布，见[假模型服务](https://github.com/334456777/WeFlow/wiki/测试与基准#假模型服务)）
  - 按 OpenAI 和 Anthropic 的接口文档实现 `/v1/chat/completions`、`/v1/messages`、`/v1/models`，用模型名切换场景（限流、过载、拒绝回答等）
  - `weflow-core` 的 `model_api`（见解的模型接口）的测试和设了 `WEFLOW_DEV_ACCOUNT` 的 `bun run dev` 用它

### 命令 API 的契约
契约把 Rust 和 WebUI 两边连起来，流程见 Wiki 的[类型化契约](https://github.com/334456777/WeFlow/wiki/服务端实现#类型化契约)：
- `weflow-core` 的 `contract/` 类型由 schemars 生成 JSON Schema，类型的文档注释就是其中的 `description`
- `weflow-cli` 的测试把整份契约和每种回答的示例写在 [`web/contract/`](web/contract) 的 `openapi.json` 和 `samples.json` 里，并检查它们是否最新
- `bun run contract` 由 `openapi.json` 生成 WebUI 的类型 [`src/api/contract.gen.ts`](web/src/api/contract.gen.ts)，加 `--check` 时只检查
- 改了契约类型（包括文档注释）或路由之后，重写这两个 JSON 文件、重新生成网页的类型，并跑整个 `cargo test -p weflow-cli --bin weflow`
- web CI 用 [`contract-compat.ts`](web/scripts/contract-compat.ts) 和最新发布版的 `openapi.json` 比较：会让已发布的客户端读错的改动要提升 API 版本（见[约定](#约定)）

### 测试
测试的分层和工具见 Wiki 的[测试与基准](https://github.com/334456777/WeFlow/wiki/测试与基准)。
- 端到端测试是 [`crates/weflow-core/tests/`](crates/weflow-core/tests) 下的 `*_e2e.rs`（共享 `tests/common`，用 `weflow-native` 的 `test-fixtures` 合成加密账号）；命令 API 的服务测试在 [`crates/weflow-cli/tests/serve_api/`](crates/weflow-cli/tests/serve_api)
- 测试要写文件时用 `weflow_native::fixture::TestDir` 建自己的目录（丢弃时删除，[#685](https://github.com/334456777/WeFlow/issues/685)），不要自己拼 `std::env::temp_dir()` 下的路径；设 `WEFLOW_KEEP_TEST_DIRS` 时保留目录（见[测试目录](https://github.com/334456777/WeFlow/wiki/测试与基准#测试目录)）
- 性质测试（proptest，[#502](https://github.com/334456777/WeFlow/issues/502)）写在各模块的单元测试里，名字以 `any_` 等开头；设 `PROPTEST_CASES` 调高用例数
- Kani 证明（[#501](https://github.com/334456777/WeFlow/issues/501)）写在 weflow-native 各模块的 `#[cfg(kani)] mod proofs` 里，只在 Linux/macOS 上跑（Windows 上用 WSL），不进 CI；运行前必须用 `ulimit -v` 限制内存，见 [Kani 证明](https://github.com/334456777/WeFlow/wiki/测试与基准#kani-证明)
- 各 crate 的 `examples/*_probe.rs` 是调研用探针（见[探针](https://github.com/334456777/WeFlow/wiki/测试与基准#探针)）
- 基准账号：`fixture::bench`（[`bench_account.rs`](crates/weflow-cli/examples/bench_account.rs)）按种子生成与真实账号同规模、同分布的账号，分布和本机校准见[基准账号](https://github.com/334456777/WeFlow/wiki/测试与基准#基准账号)
- 碰后端的功能 PR 合并前，维护者在真实账号上验证一次，结果只公开汇总
- WebUI 的测试见下一节

### WebUI（`web/`）
`weflow serve` 的网页界面（[#130](https://github.com/334456777/WeFlow/issues/130)），独立的 TypeScript + Vite 项目，有自己的 [`package.json`](web/package.json)，开发说明见 [`web/README.zh-CN.md`](web/README.zh-CN.md)。
- 渲染：源码按 React 19 来写，构建时由 Preact 11 渲染（`vite.config.ts` 把 `react`、`react-dom` 指到 `preact/compat`，[#783](https://github.com/334456777/WeFlow/issues/783)）；`bun test` 仍用 React
- 发布：构建产物 `dist/` 由 `serve` 托管在 `/ui/`（[#146](https://github.com/334456777/WeFlow/issues/146)），与二进制分开发布（[#148](https://github.com/334456777/WeFlow/issues/148)）；`package.json` 的 `weflow.apiVersion` / `minBackendVersion` 声明需要的 API，构建时写入 `dist/weflow-web.json`
- [`src/api/`](web/src/api) 调用命令 API，用 `/health/events` 长连接显示连接状态并检查版本（[#221](https://github.com/334456777/WeFlow/issues/221)）
- [`src/mock/`](web/src/mock) 是示例服务端（[#879](https://github.com/334456777/WeFlow/issues/879)，见[示例数据](https://github.com/334456777/WeFlow/wiki/WebUI-实现#示例数据)）：
  - 还没连上服务时，`api/client.ts` 的读取由它按命令 API 的格式回答，各页面用同一套组件显示示例记录，内容围绕 WeFlow 和 WebUI
  - 改动保存在页面的内存里（刷新后复原），作业在页面里按时间跑完，导出只给说明文件，不按格式生成（[#896](https://github.com/334456777/WeFlow/issues/896)）
  - 它是单独的 chunk，只能从页面代码导入类型（[`plugins/plainImport.ts`](web/plugins/plainImport.ts)）
  - 连上过的服务断开后，页面保留已读到的记录，不换回示例（`api/records.ts`，`Shell` 的 `held`、`records`、`sample`）
- `bun run dev`（[`scripts/dev.mjs`](web/scripts/dev.mjs)，[#333](https://github.com/334456777/WeFlow/issues/333)）只启动 Vite，网页显示示例数据；`WEFLOW_DEV_ACCOUNT=dev`（合成账号，含「图片测试」会话）或 `bench`（基准账号）时同时启动读它的 `weflow serve` 和假模型服务，`WEFLOW_SERVE_URL` 把请求转给已有的 serve。起服务的 [`scripts/account.ts`](web/scripts/account.ts) 按 `CARGO_TARGET_DIR` 找编译出的 `weflow`
- `bun run smoke`（[`smoke/`](web/smoke)，[#789](https://github.com/334456777/WeFlow/issues/789)）在 `bun run build` 之后用无头 Chrome 检查 Preact 渲染的生产构建，CI 在 [`web.yml`](.github/workflows/web.yml) 里跑；`bun run shots`（[#827](https://github.com/334456777/WeFlow/issues/827)）截下各个场景，`--compare` 逐像素比较前后两次，只在同一台机器上用。细节见 Wiki 的 [WebUI 实现#测试](https://github.com/334456777/WeFlow/wiki/WebUI-实现#测试)
- 样式（见 Wiki 的[设计语言](https://github.com/334456777/WeFlow/wiki/WebUI-实现#设计语言)）：
  - 文件：各页面共用的在 [`src/app.css`](web/src/app.css)，只属于一个页面的放在页面目录下的 CSS 文件里，由页面的模块导入
  - 层叠层（[#827](https://github.com/334456777/WeFlow/issues/827)）：从低到高是 `tokens`、`app`（`app.css`）、`pages`（页面的样式表）、`roles`（[`styles/roles.css`](web/src/styles/roles.css)）、`skin`
    - 后面的层不论优先级都胜过前面的层：页面规则直接盖过共用规则，共用规则（包括窄屏的媒体查询）也盖不过页面规则
    - 每个样式表的规则都包在所属的层里，`styles/layers.test.ts` 检查
  - 角色（[#829](https://github.com/334456777/WeFlow/issues/829)）：按钮、可选的项、列表行、勾选框在各状态（悬停、按下、选中、禁用）下的样子、过渡和按下的缩小只写在 `roles` 层
    - 页面的元素用 `as-button`、`as-control`、`as-option`、`as-row` 类名取得角色
    - 页面和 `app.css` 只写布局和文字，不写这些元素的阴影、底色和过渡（写了也会被 `roles` 层盖掉），元素之间的差别用变量（`--press`、`--transition`）表达
  - 皮肤（[#828](https://github.com/334456777/WeFlow/issues/828)）：颜色、底色和阴影来自皮肤
    - 页面只用皮肤给出的角色令牌（按控件的用途和状态命名，如 `--button-shadow-hover`、`--row-bg-selected`），不用皮肤的原始令牌（新拟态的 `--neu-*` 只在默认皮肤 [`styles/neumorphism.css`](web/src/styles/neumorphism.css) 里，其他皮肤的 `--<id>-*` 只在它自己的样式表里），`styles/roles.test.ts` 检查
    - 其他皮肤是 [`styles/skins/<id>.css`](web/src/styles/skins)，只设置新拟态或其他样式表已给出的令牌，以及自己的原始令牌，登记在 [`src/skin.ts`](web/src/skin.ts)，选中时才下载，与浅色、深色分开选择（`data-skin`，[#830](https://github.com/334456777/WeFlow/issues/830)）
    - 登记时写了 `shapes` 的皮肤由按钮、控件、可选项和列表行的 `::before`（外形层）画底色和阴影，可以裁剪、描边、倾斜、投影（`data-shapes`，[#831](https://github.com/334456777/WeFlow/issues/831)），所以这些元素的 `::before` 不给页面用
  - 动画：换页和换右侧详情时用 [`motion.ts`](web/src/motion.ts) 的 `pageIn` 播放进入动画，它同时通知皮肤（装饰层 `Backdrop.tsx` 轮换、皮肤自己的进入动画 `--page-in`，[#832](https://github.com/334456777/WeFlow/issues/832)）
- 语言：界面跟随服务端的语言（[#439](https://github.com/334456777/WeFlow/issues/439)，`/health` 的 `lang`，见[语言](https://github.com/334456777/WeFlow/wiki/WebUI-实现#语言)）
  - 文字以中文原文作键写成 `t('…')`，英文在 [`src/i18n/en.ts`](web/src/i18n/en.ts)（单独的 chunk，需要英文时才下载），带变量的写 `{name}`，英文的计数可写 `{n|one|other}`
  - 模块顶层的标签用 `msg()` 标出、显示时再 `t()`；日期和星期用 `formatDate` / `relativeDay`（`Intl`）
  - `bun run i18n` 列出缺英文、多余的英文和还没进 `t()` 的中文，`--check` 在 CI 里拦前两种
  - 读取的数据里的中文（如微信支付消息的标签）在那一行写 `// i18n-ignore: <原因>`

### Skills（`.agents/skills/`）
常见流程的步骤和易错之处写成 skill（[Agent Skills 规范](https://agentskills.io/specification)），智能体启动时读到每个 skill 的名字和描述，按任务自己使用；设计和原因见 Wiki 的[智能体的流程](https://github.com/334456777/WeFlow/wiki/开发指南#智能体的流程)。维护 skill 时：
- 只改 `.agents/skills/` 下的正本；`.claude/skills` 是给 Claude Code 的符号链接，Claude Code 不允许写 `.claude/` 下的文件。Windows 上要开启开发者模式并设 `core.symlinks` 才能检出这个链接和 `AGENTS.md`，见 Wiki 的 [Windows 上的符号链接](https://github.com/334456777/WeFlow/wiki/开发指南#windows-上的符号链接)
- `name` 用 `weflow-` 开头；智能体只凭 `description` 决定用不用，写清做什么、什么时候用，写成一行
- 只写步骤和易错之处，规则和原因链接到本文件和 Wiki，不重复；需要维护者权限的步骤标「维护者」
- skill 是公开的，写给所有贡献者和各种智能体：不写本地路径、个人情况和只适用于某个智能体平台的做法；用中文、只写现状，脚本注释用英文
- 流程（命令、参数、工作流、规则）变了同步改对应的 skill；用 skill 时发现过时或缺了坑，直接改

### Wiki（`wiki/`）
文档写在 GitHub Wiki（ https://github.com/334456777/WeFlow/wiki ），仓库里不新增文档文件。
- 只写中文，以技术文档为主，用正式语气书写
- 根据 issue、PR、注释和代码本身写，要写详细
- 只写现状，写成最终结论，不写中间过程：不写「原来/原先……」「已删除」「从未起作用」「改为」「从 X 变为 Y」、改动前后的测量对比和改动经过，这些写进 issue 和 PR；「命令-API」[#版本](https://github.com/334456777/WeFlow/wiki/命令-API#版本) 按版本列出破坏性改动，每项也只写一句结果
- `wiki/` 是 Wiki（`WeFlow.wiki.git`，分支 `master`）的 clone，是独立的 git 仓库：`.gitignore` 把它排除在主仓库之外，`.ignore` 让 ripgrep 等搜索工具仍能搜到它；改 Wiki 时在这个目录里单独提交和推送
- Wiki 是公开的，隐私规则同样适用
- 写法要求见 GitHub 的[关于 Wiki 页面](https://docs.github.com/zh/communities/documenting-your-project-with-wikis/about-wikis)：README 是快速介绍，Wiki 写如何使用项目、如何设计项目和核心原则，不重复 README 的内容；按主题组织，同一内容只写在一处，改动页面结构时同步重写 `_Sidebar.md`
- 页面之间用完整地址 `[文字](https://github.com/334456777/WeFlow/wiki/页面名#锚点)`，指向仓库文件用完整的 `https://github.com/334456777/WeFlow/blob/main/...` 地址，issue 写完整链接（Wiki 不会自动链接 `#123`）；页面标题由文件名决定，文件名不用 `\ / : * ? " < > |`；小节标题不加序号，否则插入一节后它后面的锚点全部失效
- 用户可见的行为变了就同步改对应页面。这些靠人工同步，没有测试校验：
  - API 版本（`API_VERSION`）写在[命令-API](https://github.com/334456777/WeFlow/wiki/命令-API#版本)页
  - 不支持的功能或与桌面版的差异增减时，同步改[不支持的功能](https://github.com/334456777/WeFlow/wiki/不支持的功能)
  - 报错的文字或处理办法变了，同步改[故障排查](https://github.com/334456777/WeFlow/wiki/故障排查)（按现象列原因和去处，细节留在主题页）
  - 新增在多个页面出现的术语时，登记到[术语表](https://github.com/334456777/WeFlow/wiki/术语表)
- Wiki 不跟代码走同一个 PR：先推送 Wiki，再合并引用新页面或新锚点的 PR

### 其他
- [`README.md`](README.md)、[`README.zh-CN.md`](README.zh-CN.md)：英文和中文 README，内容保持同步
- [`docs/images/`](docs/images)：README 用的图片；`docs/` 下其他目录是不进仓库的本地参考资料
- [`.github/workflows/`](.github/workflows)：CI 与发布的工作流，见 Wiki 的[发布与 CI](https://github.com/334456777/WeFlow/wiki/发布与-CI)；基准测试（`bench.yml`）见[测试与基准](https://github.com/334456777/WeFlow/wiki/测试与基准#持续基准benchyml)


## 约定

### 隐私
- 此 git 仓库所连接的远程 GitHub 仓库是公开仓库（public），GitHub Project「WeFlow」是公开 Project；严禁将微信聊天记录，微信wxid，隐私信息，本地路径，密钥，API Key 等等隐私信息上传到公开的 GitHub 仓库或写入 CLAUDE.md 和 skill
- 公开的内容（issue、PR、Wiki、注释、skill）也不写维护者的个人情况（作息、设备的使用状况），只写系统做什么

### 提交与 Pull Request
- 提交信息使用中文和 Conventional Commits 规范：前缀（`type(scope):`）用英文，描述用中文；信息末尾另起一行按此格式署名：`Co-Authored-By: <你的AI名称与模型版本> <noreply@厂商域名>`（例如 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`、`Co-Authored-By: GPT-5 <noreply@openai.com>`），已有署名行时不要重复添加
- Pull Request 分支名称使用 Conventional Commits 风格的 `<type>/<scope>-<description>` 格式
- 合并用 merge commit（见 Wiki「发布与 CI」[#合并的条件](https://github.com/334456777/WeFlow/wiki/发布与-CI#合并的条件)）
- 合并提交的标题由 GitHub 取 PR 的标题加 `(#N)`，所以 PR 标题也以 `type(scope):` 前缀开头（前缀用英文，描述用中文）；PR 描述同样用中文，用户明确要求英文时才用英文
- 创建 PR 时及时添加 Labels（如果有对应 Development 则添加对应 Development）；PR 关闭关联 Issues 时，在 PR 描述或 commit message 里用 `closed` `fixed` `resolved` 关键词（这几个关键词用英文）
- 对于需要多个依赖 PR 的开发任务，请使用 `gh stack`

### 版本与 tag
- 版本号遵循语义化版本控制（SemVer）`vMAJOR.MINOR.PATCH`，预发布版本用 `-rc.1` 后缀
- tag 一律用附注 tag，不用轻量 tag（`git cat-file -t <tag>` 应输出 `tag`）
- 发版前把要升的版本位和理由告诉维护者，确认后再打 tag
- API 版本（[`crates/weflow-core/src/contract.rs`](crates/weflow-core/src/contract.rs) 的 `API_VERSION`，与 `web/package.json` 的 `weflow.apiVersion` 一致）只在会让现有客户端出错的改动时提升（路由、参数、响应、配置键）
  - 提出升版本之前先核对：最新 `web-v*` tag 中 `web/package.json` 的 `weflow.apiVersion`，和 `origin/main` 上的 `API_VERSION`
  - main 已经比发布版高一个版本（升过、还没发布）时，改动并入这个版本，不再提醒升版本，只把改动记进 Wiki「命令-API」[#版本](https://github.com/334456777/WeFlow/wiki/命令-API#版本) 中当前版本的条目；两者相同时才提出要升

### Issue 与看板
- GitHub Issues 根据 Labels 分类，筛选和表达状态，根据 Relationships 梳理 Issue 依赖关系（Issue dependencies），创建 Issue 时及时添加 Labels 和 Relationships （如果有对应的 Development 则添加对应 Development）
- GitHub Issues 会有人类和其他大语言模型（LLM）参与讨论，以下情况记录到本仓库的 GitHub Issues:
  - 被列为计划内的工作
  - 需要多轮确认的bug
  - 不符合当前工作的主题但是需要解决的bug
  - 可以稍后解决的bug
  - 被要求将不会被开展的工作
  - 需要多轮和长时间研究思考的精益求精的要求
  - 工作中遇到的反常/不太对劲的地方
  - 新功能或请求
  - 对文档的改进或补充
  - 一些在修改代码后会经常出现的问题和需要更多信息的问题
  - 以及其他需要在本次会话之外继续跟踪的事
- 同时在遇到上述情况时也可以查询Issues寻找解决方法或继续跟踪评论issues
- GitHub Project「WeFlow」（`gh project` 编号 3，owner 为仓库所有者，需要 token 带 `project` 权限，仓库内相对路径与公开的 owner 名可以出现）是看板，所有 Issue 和 PR（开着的和关着的，CI 配置类的除外）都要放进去，并随工作进展及时更新 Status：
  - `Backlog` 已记录但尚未排期
  - `Todo` 已排期待开始
  - `In Progress` 正在进行
  - `In Review` PR 已提交等待审查
  - `Blocked` 被依赖或外部因素阻塞（同时用 Relationships 标出阻塞来源）
  - `Done` 已完成并关闭或已合并
  - `Not Planned` 已关闭但不会开展（对应 GitHub 的 not planned 关闭原因）
- 新建 Issue 或 PR 后立即加入看板并设置 Status 与 `Priority`（`P0` 紧急，立即处理；`P1` 高；`P2` 普通；`P3` 低，可随时搁置）；PR 打开时设为 `In Review`，合并或关闭后与其关联的 Issue 一起设为 `Done`

### 代码与工作副本
- 代码按职责划分模块：一个文件只承担一类职责，新代码放进所属的模块（没有就新建），不往入口文件或已经很长的文件里追加；拆分只移动代码、调整可见性和引用路径，与功能改动分开提交，可见性用最小的范围（见 Wiki「开发指南」的[代码组织](https://github.com/334456777/WeFlow/wiki/开发指南#代码组织)）
- 本仓库的工作副本可能同时有多个会话在用，一个会话切换分支，另一个会话接下来的提交就会落到那个分支上：每个会话用 `git worktree` 在 `.claude/worktrees/` 创建自己的 worktree ，在此 worktree 创建自己的分支工作，不在共用的工作副本里切换分支
  - worktree 里编译 Rust 时设 `CARGO_TARGET_DIR` 用它自己的 target 目录（主工作副本 `target/` 下的 `wt-<worktree 名>`），不与主工作副本或其他 worktree 共用 `target/debug`（原因见 Wiki「开发指南」的[提交与 Pull Request](https://github.com/334456777/WeFlow/wiki/开发指南#提交与-pull-request)）

### 第三方代码的许可
发布包的 `licenses/` 文件夹列出程序所含他人代码的许可（[#1111](https://github.com/334456777/WeFlow/issues/1111)），做法见 Wiki「发布与 CI」的[打包](https://github.com/334456777/WeFlow/wiki/发布与-CI#打包)：
- 新增的 crate 依赖由 cargo-about 自动列出；它的许可不在 [`.github/about/about.toml`](.github/about/about.toml) 的 `accepted` 里时发版失败，先确认这个许可允许随二进制分发再加进去
- 移植、照着重写其他项目的代码或数据（如特征码、偏移量、算法的常量）时：
  - 对方的许可文件原样放进这些代码所在的文件夹（如 `windows_db_key/LICENSE`），模块的 `//!` 写明来源和许可
  - cargo-about 跳过本仓库自己的 crate，看不到这份许可：在 [`licenses.hbs`](.github/about/licenses.hbs) 的「Code that is not a crate's own」一节加一项，在 [`third-party-licenses.sh`](.github/scripts/third-party-licenses.sh) 里把它拷成那一项链接的文件
  - 对方的许可有非商业、相同方式共享等限制，或要求声明（如 IJG）时，先告诉维护者

### 修 bug 的智能体
- 有一个专门修 bug 的智能体，按 [`.claude/loop.md`](.claude/loop.md) 循环监视标为 `bug` 的未关闭 Issue 并修复
- 其他智能体在工作中发现 bug 时（为保持会话连贯性）可以建 Issue、加 `bug` 标签并加入看板，那块代码交给它改；能给其他会话发消息的智能体还可以直接交给它，写明原因、建议的修法和自己这边会改的文件
- 它合并后会通知发来请求的一方，收到后把自己的分支 rebase 到最新的 main 上
- 查不到正在运行的修 bug 智能体时，新建一个，让它读 `.claude/loop.md`、按其中的[「启动」](.claude/loop.md#启动)开始循环修 bug，再把手上的 bug 交给它；新建后告诉用户并附上会话链接，不自己关掉，关不关由用户决定（停止的步骤见 `.claude/loop.md` 的[「停止」](.claude/loop.md#停止)）

## 注意事项
- 默认用中文回复，代码注释用英文；用户可能会出错，当用户出现明显错误或决策冲突时，请指出
- 读文件、改文件、搜索必须使用 agent 自带的文件工具（读取、写入、编辑、按文件名查找、按内容搜索等），不要写脚本（`.sh`/`.py`/`.ps1`）或用 shell heredoc、`sed`、`echo >` 去做：修改已有文件用编辑工具（多处修改就多次调用），新建文件用写入工具；只有批量机械替换、长时间测量这类自带工具确实做不了的事才用脚本，脚本先用写入工具写成文件再运行，用完即删
- 次要描述或标签转换成注释的形式存在，用布局来替代解释性小字
