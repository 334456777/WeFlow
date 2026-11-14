# CLAUDE.md

## 命令
 - `make help` 查看构建指南
 - `rust-toolchain.toml` 固定了 Rust 版本（本地与 CI 共用）；升级时改 channel，跑 `make ci` 修掉新增的 clippy 警告，一起提交
 - `target/` 不会自动清理，用 `cargo sweep` 清理过期编译缓存：
   - 精确清理：`cargo sweep --stamp` → 正常编译一次（实际会用到的 profile 都要编译，如 `cargo build`、`cargo build --release`、`cargo test --no-run`）→ `cargo sweep --file`，删除时间戳之后没被用到的产物
   - 日常清理：`cargo sweep --time 30` 删除 30 天没用过的产物；`cargo sweep --installed` 删除已卸载工具链编译的产物
   - 首次执行任何删除命令前先加 `--dry-run` 预览；不要用 `find -delete` 自制清理，也不要为了腾空间直接 `cargo clean`
 - Windows PowerShell 终端上可以用常用的 UNIX/Linux 命令行工具；读文件、改文件、搜索仍用自带工具（见「注意事项」）

## 结构
仓库由两部分组成：Rust workspace（CLI `weflow`、`weflow serve` 的命令 API 和纯 Rust 只读数据库层）和 WebUI（`web/`）。

### Rust workspace（`crates/`）
依赖方向：`weflow-cli` → `weflow-core` → `weflow-native` / `weflow-silk` / `weflow-assets`
- `weflow-native`：纯 Rust + 内置 SQLite 的只读数据库层，替代闭源 `wcdb_api`。`cipher_vfs`/`sqlcipher` 负责按页解密，`db` 是账号句柄（`Account`），`msg`、`contact`、`sns`、`media`、`stats`、`report` 按领域（消息、联系人、朋友圈、媒体、统计、报告）查询，`wcdb.rs` 是给上层用的门面，`wxkey` 处理密钥；未移植的调用返回 "not implemented"，必须同步登记到 Wiki 的「不支持的功能」（见下面的 Wiki 一节）。`make docs-check` 检查两份 README 的相对链接和锚点是否有效、两份 README 和 CLAUDE.md 里汉字旁的逗号是否为中文逗号（只报告，不自动修改）
- `weflow-core`：业务层。`services/` 下按领域划分（account、key、chat、sessions、group、analytics、sns、media、image、voice、footprint、insight、reports、export、export_stream、export_media、backup、cleanup），每个领域一个 `impl ServiceHub`，外加 CLI 与命令 API 共用的类型化读取（`reads`，#397）、命令 API 的契约类型（`contract`，按领域分在 `contract/` 下，#399）、导出（`export*`；`message/` 按消息种类解析，`export_msg/` 每种格式一个模块）、消息推送（`push`）、配置（`config`）、图片/视频/语音解密等；HTML 导出的页面模板是 `export_msg/` 下的 `template.html`（聊天记录）、`list.html`（会话列表、联系人）和 `moments.html`（朋友圈），三者共用的配色在 `colours.css`，嵌入程序的 `<模板>.{zh,en}.html` 由 `build.mjs` 生成，CI（`html-template.yml`）在推送后生成并提交到该分支
- `weflow-cli`：命令行入口（clap），生成二进制 `weflow`。`main.rs` 只是入口（分配器、`run` 的分派、结果汇总和退出码），`cli.rs` 是顶层的 `Cli`、`Commands`，`commands/` 每个子命令一个文件（命令定义和执行它的函数放在一起），`reads.rs` 是命令行与 `serve` 共用的读取映射（`read_of`、`media_of`），`term.rs`/`files.rs` 是终端输出和写文件；`help_zh.rs`/`i18n.rs` 为中文帮助与本地化；`serve_api.rs` 与 `serve_api/` 是 `weflow serve` 的命令 API：路由和 `/openapi.json` 由 clap 命令树生成（`endpoints`，方法表和拒绝的命令在 `rules`），`in_process` 在服务进程内执行 `/api/` 读取，`jobs`（生命周期，不依赖 axum）与 `job_routes`（HTTP 层）是后台任务（#398），其余路由由 `child` 在子进程里运行命令（见 Wiki「服务端实现」的「子进程路由」）
- `weflow-silk`：内置 Skype SILK SDK 的语音解码器（`vendor/` 为第三方 C 源码，勿改）
- `weflow-assets`：构建期把 `assets/` 下的文件（各平台的辅助程序、VC 运行库）嵌入二进制，运行时解压到运行时目录，路径与 `assets/` 下相同
- 测试：`crates/weflow-core/tests/*_e2e.rs` 为端到端测试（共享 `tests/common`，用 `weflow-native` 的 `test-fixtures` 合成加密账号）；测试要写文件时用 `weflow_native::fixture::TestDir` 建自己的目录（丢弃时删除，#685），不要自己拼 `std::env::temp_dir()` 下的路径；性质测试（proptest，#502）写在各模块的单元测试里，名字以 `any_` 等开头：分页、页解密、WXGF 解码、导出无损，CI 用默认的较小用例数，设 `PROPTEST_CASES` 可以调高，失败时打印缩小后的最小输入，不写回源码目录；Kani 证明（#501）写在 weflow-native 各模块的 `#[cfg(kani)] mod proofs` 里（WAL 解析、页数、第 1 页修补、块范围与 VFS 读位置、消息排序键），只能在 Linux/macOS 上跑：WSL 里 `cargo kani -p weflow-native --lib`（约 9 分钟，用 `ulimit -v` 限制内存，CBMC 的规模失控时能吃掉几十 GB），每个证明的输入上限写在它的注释里，不在 CI 里跑；各 crate 的 `examples/*_probe.rs` 是调研用探针；`fixture::bench`（`weflow-cli/examples/bench_account.rs`）按种子生成与真实账号同规模、同分布的基准账号（#488），分布是 `fixture::bench::Profile`：默认值照 `weflow-core/examples/account_profile_probe.rs` 在真实账号上测得的汇总比例写在代码里，`bench_account --profile` 改用本机校准的 profile（`account_profile_probe --calibrate`，#605）

### WebUI（`web/`）
`weflow serve` 的网页界面（#130），独立的 React + TypeScript + Vite 项目，有自己的 `package.json`。构建产物 `dist/` 由 `serve` 托管在 `/ui/`（#146），与二进制分开发布（#148）。
- `src/api/` 调用命令 API，用 `/health/events` 长连接显示连接状态并检查版本（#221）；`src/mock/` 是未连接服务时显示的示例记录（会话列表、记录浏览和控制台的默认日期范围），不是暂用的数据
- `bun run dev`（`scripts/dev.mjs`，#333；依赖由 Bun 按 `bun.lock` 安装；测试用 `bun test` 加 happy-dom，`test/` 下是测试 API 和 preload，不需要 Node.js）同时启动 Vite 和一个读合成账号的 `weflow serve`（`crates/weflow-cli/examples/dev_account.rs` 生成在 `target/dev-account/`，含「图片测试」会话），随机令牌由代理加上，不编进网页；设了 `WEFLOW_SERVE_URL` 时只启动 Vite，转给那个 serve；设 `WEFLOW_DEV_ACCOUNT=bench` 时改用基准账号（#488）
- 样式：各页面共用的在 `src/app.css`，只属于一个页面的放在页面目录下的 CSS 文件里，由页面的模块导入；页面的样式表排在 `app.css` 之前，页面规则要盖过同一元素上的共用规则时，选择器里同时写上共用的类名（如 `.icon-button.resource-locate`），以优先级取胜，不靠先后顺序（#732、#737，见 Wiki「WebUI 实现」的「设计语言」）
- `package.json` 的 `weflow.apiVersion` / `minBackendVersion` 声明需要的 API，构建时写入 `dist/weflow-web.json`
- 界面跟随服务端的语言（#439，`/health` 的 `lang`）：文字以中文原文作键写成 `t('…')`，英文在 `src/i18n/en.ts`（单独的 chunk，需要英文时才下载），带变量的写 `{name}`，英文的计数可写 `{n|one|other}`，模块顶层的标签用 `msg()` 标出、显示时再 `t()`；日期和星期用 `formatDate` / `relativeDay`（`Intl`）。`bun run i18n` 列出缺英文、多余的英文和还没进 `t()` 的中文，`--check` 在 CI 里拦前两种；读取的数据里的中文（如微信支付消息的标签）在那一行写 `// i18n-ignore: <原因>`

### Wiki（`wiki/`）
文档写在 GitHub Wiki（ https://github.com/334456777/WeFlow/wiki ），仓库里不新增文档文件。
- 只写中文，以技术文档为主，用正式语气书写
- 根据 issue、PR、注释和代码本身写，要写详细
- 只写现状，写成最终结论，不写中间过程：不写「原来/原先……」「已删除」「从未起作用」「改为」「从 X 变为 Y」、改动前后的测量对比和改动经过，这些写进 issue 和 PR；「命令-API」#版本 按版本列出破坏性改动，每项也只写一句结果
- `wiki/` 是 Wiki（`WeFlow.wiki.git`，分支 `master`）的 clone，是独立的 git 仓库：`.gitignore` 把它排除在主仓库之外，`.ignore` 让 ripgrep 等搜索工具仍能搜到它；改 Wiki 时在这个目录里单独提交和推送
- Wiki 是公开的，隐私规则同样适用
- 写法要求见 GitHub 的[关于 Wiki 页面](https://docs.github.com/zh/communities/documenting-your-project-with-wikis/about-wikis)：README 是快速介绍，Wiki 写如何使用项目、如何设计项目和核心原则，不重复 README 的内容；按主题组织，同一内容只写在一处，改动页面结构时同步重写 `_Sidebar.md`
- 页面之间用完整地址 `[文字](https://github.com/334456777/WeFlow/wiki/页面名#锚点)`，指向仓库文件用完整的 `https://github.com/334456777/WeFlow/blob/main/...` 地址，issue 写完整链接（Wiki 不会自动链接 `#123`）；页面标题由文件名决定，文件名不用 `\ / : * ? " < > |`；小节标题不加序号，否则插入一节后它后面的锚点全部失效
- 用户可见的行为变了就同步改对应页面，包括 API 版本（`API_VERSION`，写在「命令-API」页）；未移植或被拒绝的调用增减时，同步改「不支持的功能」；报错的文字或处理办法变了，同步改「故障排查」（按现象列原因和去处，细节留在主题页）；新增在多个页面出现的术语时，登记到「术语表」。这些靠人工同步，没有测试校验
- Wiki 不跟代码走同一个 PR：先推送 Wiki，再合并引用新页面或新锚点的 PR

### 其他
- `README.md`、`README.zh-CN.md`：英文和中文 README，内容保持同步
- `docs/images/`：README 用的图片；`docs/` 下其他目录是不进仓库的本地参考资料
- `.github/workflows/`：CI 与发布的工作流，见 Wiki 的[发布与 CI](https://github.com/334456777/WeFlow/wiki/发布与-CI)；基准测试（`bench.yml`）见[测试与基准](https://github.com/334456777/WeFlow/wiki/测试与基准#持续基准benchyml)


## 约定
- 此 git 仓库所连接的远程 GitHub 仓库是公开仓库（public），GitHub Project「WeFlow」是公开 Project
- 提交信息使用中文和 Conventional Commits 规范，信息末尾另起一行按此格式署名：`Co-Authored-By: <你的AI名称与模型版本> <noreply@厂商域名>`（例如 `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>` `Co-Authored-By: GPT-5 <noreply@openai.com>`），已有署名行时不要重复添加。
- Pull Request 分支名称使用 Conventional Commits 风格的 `<type>/<scope>-<description>` 格式，合并用 merge commit （`gh pr merge --merge`），合并提交的标题由 GitHub 取 PR 的标题加 `(#N)`。
- 版本与 tag 规范：版本号遵循语义化版本控制（SemVer）`vMAJOR.MINOR.PATCH`，预发布版本用 `-rc.1` 后缀；tag 一律用附注 tag，不用轻量 tag（`git tag -a vX.Y.Z -m "WeFlow vX.Y.Z" <commit>`，用 `git cat-file -t vX.Y.Z` 检查，应输出 `tag`）；发布前先查看现有 tag 和上次发布以来的提交，判断该升哪一位并告诉我理由，我确认后再打 tag。只想验证发版流程（改了 `release.yml`、`pgo-build.sh`、`release-check.sh` 等）而不发布时，用 `test.<名称>` tag 试跑：`gh workflow run release.yml --ref <分支> -f tag=test.<名称>`，照常构建和测试，不发布 Release、不建 tag、不跑基准，压缩包留在运行的 artifact 里（见 Wiki「发布与 CI」#试跑）。
- API 版本（`crates/weflow-core/src/contract.rs` 的 `API_VERSION`，与 `web/package.json` 的 `weflow.apiVersion` 一致）只在会让现有客户端出错的改动时提升（路由、参数、响应、配置键）。提出升版本之前先核对：最新 `web-v*` tag 中 `web/package.json` 的 `weflow.apiVersion`，和 `origin/main` 上的 `API_VERSION`。main 已经比发布版高一个版本（升过、还没发布）时，改动并入这个版本，不再提醒升版本，只把改动记进 Wiki「命令-API」#版本 中当前版本的条目；两者相同时才提出要升
- GitHub Issues 根据 Lables 分类，筛选和表达状态，根据 Relationships 梳理 Issue 依赖关系（Issue dependencies），创建 Issue 时及时添加 Lables 和 Relationships （如果有对应的 Development 则添加对应 Development）
- GitHub Issues 会有人类和其他大语言模型（LLM）参与讨论，以下情况记录到本仓库的 GitHub Issues: 被列为计划内的工作，需要多轮确认的bug，不符合当前工作的主题但是需要解决的bug，可以稍后解决的bug，被要求将不会被开展的工作，需要多轮和长时间研究思考的精益求精的要求，工作中遇到的反常/不太对劲的地方，新功能或请求，对文档的改进或补充，一些在修改代码后会经常出现的问题和需要更多信息的问题以及其他需要在本次会话之外继续跟踪的事；同时在遇到上述情况时也可以查询Issues寻找解决方法或继续跟踪评论issues
- GitHub Project「WeFlow」（`gh project` 编号 3，owner 为仓库所有者，需要 token 带 `project` 权限，仓库内相对路径与公开的 owner 名可以出现）是看板，所有 Issue 和 PR（开着的和关着的）都要放进去，并随工作进展及时更新 Status：
  - `Backlog` 已记录但尚未排期；`Todo` 已排期待开始；`In Progress` 正在进行；`In Review` PR 已提交等待审查；`Blocked` 被依赖或外部因素阻塞（同时用 Relationships 标出阻塞来源）；`Done` 已关闭或已合并
  - 新建 Issue 或 PR 后立即加入看板并设置 Status 与 `Priority`（`P0` 紧急，立即处理；`P1` 高；`P2` 普通；`P3` 低，可随时搁置）；PR 打开时设为 `In Review`，合并或关闭后与其关联的 Issue 一起设为 `Done`
  - 用 `gh project item-add` 加入条目，用 `gh project item-edit` 改字段；重写 Status 选项会清空所有条目的状态，改选项后必须重新设置
- Pull request 创建时及时添加 Lables（如果有对应 Development 则添加对应 Development）；PR关闭时PR描述或 commit message 里用 `closed` `fixed` `resolved` 关键词关闭关联 Issues；PR 标题和描述用中文，只有这几个关键词用英文；用户明确要求英文时才用英文
- 代码按职责划分模块：一个文件只承担一类职责，新代码放进所属的模块（没有就新建），不往入口文件或已经很长的文件里追加；拆分只移动代码、调整可见性和引用路径，与功能改动分开提交，可见性用最小的范围（见 Wiki「开发指南」的[代码组织](https://github.com/334456777/WeFlow/wiki/开发指南#代码组织)）
- 用户可能会出错，当用户出现明显错误或决策冲突时，请指出
- 对于需要多个依赖 PR 的开发任务，请使用 `gh stack`
- 本仓库的工作副本可能同时有多个会话在用，一个会话切换分支，另一个会话接下来的提交就会落到那个分支上：每个会话用自己的 `git worktree` 在自己的分支上工作，不在共用的工作副本里切换分支；worktree 里的工作合并到远程后，在主工作副本里 `git pull` 拉取远程，再清理 worktree 和分支；push 显示 up-to-date 但远程没有变化时，先查 `git reflog`

## 注意事项
- 默认用中文回复，代码注释用英文
- 严禁将微信聊天记录，微信wxid，隐私信息，本地路径，密钥，API Key 等等隐私信息上传到公开的 GitHub 仓库或写入 CLAUDE.md
- 读文件、改文件、搜索必须使用 agent 自带的文件工具（读取、写入、编辑、按文件名查找、按内容搜索等），不要写脚本（`.sh`/`.py`/`.ps1`）或用 shell heredoc、`sed`、`echo >` 去做：修改已有文件用编辑工具（多处修改就多次调用），新建文件用写入工具；只有批量机械替换、长时间测量这类自带工具确实做不了的事才用脚本，脚本先用写入工具写成文件再运行，用完即删
