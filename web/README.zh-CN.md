# WeFlow WebUI

[English](README.md) | 简体中文

`weflow serve` 的网页界面（#130）。用 React + TypeScript 编写，Vite 构建成静态文件：
- 由 `weflow serve` 托管在 `/ui/` 下（#146）；
- 与 `weflow` 二进制分开发布（#148）。

它调用的 API 见 [docs/zh-CN/serve-api.md](../docs/zh-CN/serve-api.md)。

连接到服务后，各个界面读的都是真实的服务。连接之前（没有服务时；令牌不对或仍在检测连接时除外），聊天、朋友圈、支付三个列表和控制台显示示例记录（`src/mock/`），并标明是示例，这样没有服务也能看到页面。

## 开发

需要 Bun 1.4.3+（版本写在 `package.json` 的 `packageManager` 里）和仓库指定的 Rust 工具链（`rustup`，见 `rust-toolchain.toml`），不需要 Node.js。Bun 按 `bun.lock` 安装依赖，脚本、`tsc`、Vite 和测试也都由它运行：

```sh
cd web
bun install --frozen-lockfile
bun run dev                               # http://localhost:5173/ui/
```

`bun run dev`（`scripts/dev.mjs`，#333）会启动两样东西：

- **网页**：Vite 直接提供这些源码，和 `bun run build` 做出来的是同一个网页，不加任何只在开发时才有的代码；
- **后端**：一个读合成账号的 `weflow serve`（调试版，先自动 `cargo build`），监听 127.0.0.1 上的一个空闲端口，令牌每次随机生成。合成账号由 `crates/weflow-cli/examples/dev_account.rs` 生成在 `target/dev-account/` 里，第一次约 20 秒，之后只在生成器改了时才重建。里面有假的好友、一个群、几条朋友圈，还有「图片测试」：一张图片一种情况，对应微信保存的图片文件（缩略图 `_t.dat`、图片 `.dat`、高清原图 `_h.dat`，各自有或没有），每张图上写着它是哪个文件和尺寸。不读任何真实微信账号。

Vite 自己处理 `/ui/`，其他路径（`/chat/sessions`、`/api/…`、`/jobs`、`/health`、`/openapi.json`）都转发给这个后端，并带上它的令牌，网页里不用填。停止 `bun run dev` 时两样一起停。

要连别的 `weflow serve`（例如真实账号）时，在终端或 `web/.env.local`（不提交）里设置下面的变量，这时只启动 Vite：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `WEFLOW_SERVE_URL` | 合成账号 | 那个 `weflow serve` 的地址，例如 `http://127.0.0.1:18108` |
| `WEFLOW_HTTP_TOKEN` | 无 | 它的令牌，由代理加上 |

要从另一台电脑或手机打开开发服务器，运行 `bun run dev --host`，再打开 `http://<这台机器的地址>:5173/ui/`。用合成账号时这样做是安全的。设置了 `WEFLOW_SERVE_URL` 和 `WEFLOW_HTTP_TOKEN` 时不要这样做：代理会把令牌交给任何能访问这个端口的人，他们就能对那个账号运行任何 `weflow` 命令。

运行 `bun run build` 后用 `weflow serve --ui-dir web/dist` 打开网页时，网页从地址里取令牌（`/ui/?token=…`）。令牌存在当前标签页的 `sessionStorage` 里，同时从地址栏去掉。

令牌被拒绝（HTTP 401）时，网页提示输入令牌后重新连接。也可以在设置里更换或清除当前标签页的令牌；令牌不写入 `localStorage` 或构建产物。设置同时显示服务端与 API 版本、兼容性和处理建议，以及 `/health` 或状态流的 `hello` 提供的任务、读取、媒体限额和灵感引擎状态（#214）。

## 脚本

| 脚本 | 说明 |
|---|---|
| `bun run dev` | 开发服务器，支持热更新 |
| `bun run typecheck` | `tsc --noEmit` |
| `bun test` | 运行测试（`bun:test`，DOM 由 happy-dom 提供） |
| `bun run build` | 类型检查后构建到 `dist/` |
| `bun run preview` | 用同样的代理预览 `dist/` |
| `bun run contract` | 由 `contract/openapi.json` 生成 `src/api/contract.gen.ts`（见下文）；带 `--check` 时只检查 |
| `bun run check` | 检查契约类型，再做类型检查、测试和构建，与 CI 相同 |

测试从 `#test`（`test/index.ts`）导入测试 API：其中是 `bun:test` 的内容，外加一个 `vi`，补上了测试里用到、而 Bun 没有的 Vitest 方法（`stubGlobal`、`unstubAllGlobals`、`setSystemTime`、`advanceTimersByTimeAsync`、`waitFor`，以及只伪造 `Date` 的假计时器）。`test/setup.ts` 由 `bunfig.toml` 指定为最先加载的文件，它注册 happy-dom，并把 happy-dom 调整成原来按 jsdom 写测试时的样子：窗口够宽，`fetch` 用运行时自带的，`WheelEvent` 继承自 `MouseEvent`，inline 元素的 `display` 是 `inline`，DOM 节点出现在失败信息里时只打印简短的标签。Bun 在同一个进程里运行所有测试文件，所以测试结束时不能留下改过的全局状态：在 `afterEach` 里调用 `vi.unstubAllGlobals()`、`vi.useRealTimers()` 和 `cleanup()`。

`web/` 下有改动时，`.github/workflows/web.yml` 会运行 `contract --check`、契约的兼容性检查（见下文）、`typecheck`、`test` 和 `build`，并上传 `dist/`。

网页显示前要加载的 JavaScript 超过 `plugins/sizeBudget.ts` 里的上限时，构建会失败（#159）；确实需要更多时，在那里有意地调高。图标只保留网页用到的粗细（`plugins/iconWeights.ts`：regular、fill、bold）；代码用到其他粗细时，测试会失败。

## API 的类型

网页最常读的回复（会话、消息、搜索结果、通讯录、资源、任务和错误）的类型在 `src/api/contract.gen.ts` 里，由 `contract/openapi.json` 生成（#399）。这个文件就是 `weflow serve` 回复所依据的契约：其中的 schema 来自回复所用的 Rust 类型（`weflow_core::contract`），它不是最新时 weflow-cli 的测试会失败，所以网页的类型总跟着服务端。`src/api/` 在它的基础上定义自己的类型：用 `Pick` 取出请求用 `fields` 要的那些键，并用 `satisfies` 核对这些键在契约里存在。

`scripts/contract.ts` 不依赖任何包来生成类型：它只认 schemars 会写出的那几种 JSON Schema，遇到其他写法就报错。回复的类型改了以后：

1. `WEFLOW_UPDATE_CONTRACT=1 cargo test -p weflow-cli --bin weflow contract_` 重新生成 `contract/openapi.json` 和 `contract/samples.json`（各类回复的样本，供 #400 的边界测试使用）；
2. `bun run contract` 重新生成 `src/api/contract.gen.ts`；
3. `bun run typecheck` 显示网页需要跟着改的地方。

API 的客户端会读错的改动要提高 `apiVersion`（见「版本」）。`scripts/contract-compat.ts` 用来发现这类改动：`bun scripts/contract-compat.ts <之前的 openapi.json>` 列出按之前的契约写的客户端在现在的回复里可能读错的地方（字段可能省略或变为可为空、出现原来没有的类型或取值、路由不再描述），有这类改动而 `info.version` 没变时失败。`web.yml` 拿改动所基于的提交的契约运行它。新增字段或路由不算这类改动。

`src/api/contract.test.ts` 把 weflow-cli 生成的 `contract/samples.json` 交给网页自己的代码：先经过 `src/api/` 的读取，再交给聊天窗口、会话列表、通讯录、资源浏览和任务中心处理。weflow-cli 有测试拿契约校验同一份样本和运行中的服务端的回复（docs/zh-CN/serve-api.md，「OpenAPI 描述」），所以网页测的是服务端真正的回复，而不是网页自己的夹具。

## 版本

`package.json` 记录了 WebUI 自己的版本。`weflow` 字段下记录它需要的 API：
- `apiVersion` 必须等于 `GET /health` 返回的 `apiVersion`；
- 服务端的 `version` 不能低于 `minBackendVersion`。开发构建的 `git describe` 版本按它所在的 tag 计算。

构建时，这些信息写入 `dist/weflow-web.json`，供 `weflow web` 使用（#148）。网页也会拿它们和 `/health/events` 的 `hello` 比对，不满足时显示「版本不兼容」。没有 `/health/events` 的服务端（5.0.0-rc.5 及更早）不再支持，连接时显示「未连接」。

发布新版本时，先提高 `package.json` 里的 `version`，再推送 tag `web-v<版本>`（或在 Actions 里运行 *WebUI release*）：`.github/workflows/web-release.yml` 构建后发布到 `ghcr.io/334456777/weflow-web:<版本>`，`weflow web` 和 `weflow serve` 从那里下载。同一个版本只能发布一次。每个 `weflow` 构建默认显示构建时 `package.json` 里的版本（见 [docs/serve-api.md](../docs/zh-CN/serve-api.md#网页的版本)）。

## 目录

| 路径 | 说明 |
|---|---|
| `src/api/contract.gen.ts`、`scripts/contract.ts`、`contract/` | 由契约生成的回复类型（#399），见「API 的类型」 |
| `scripts/contract-compat.ts`、`src/api/contract.test.ts` | 契约的改动会不会让客户端读错，以及网页代码处理契约样本的测试（#400） |
| `src/api/client.ts` | 调用 `weflow serve`：负责查询参数或 JSON 请求体，带上令牌，出错时抛出带服务端错误码的 `ApiError` |
| `src/TokenForm.tsx` | 令牌输入与重新连接提示（#214） |
| `src/api/backend.ts` | 连接状态：保持 `/health/events` 长连接，加上看门狗和退避重连（#221）；打开实时消息时改为 `/api/push/messages`（#201）；检查版本，提供 `useBackend` |
| `src/api/live.ts` | 实时消息事件，分发给会话列表和打开的会话（#201） |
| `src/api/biz.ts` | 公众号：关注的公众号（`/api/biz/accounts`）和它们的推送（`/api/biz/messages`）（#203） |
| `src/api/sns.ts`、`src/moments/` | 朋友圈（#202）：时间线、发布者及其条数、总数（`/api/sns/timeline`、`post-counts`、`stats`），以及通过 `/api/sns/media` 读取的图片和视频；时间线旁列出发布者，控制台按文字和日期筛选 |
| `src/api/pay.ts`、`src/payments/` | 微信支付（#204）：逐页读取「微信支付」公众号的全部通知（`/api/biz/pay-records`），从各行文字里拆出金额、方式、状态和收款方；旁边的列表按商户合计，控制台按日期筛选、汇总，并导出为 HTML、JSON 或 Excel |
| `src/api/details.ts` | 会话资料、群成员统计、媒体及解析后的单条消息查询；唯一要起子进程的读取（`chat detail --extra`）逐个执行（#200） |
| `src/chat/SessionInfo.tsx`、`src/chat/MediaPanel.tsx` | 展开时读取的控制台卡片：会话资料、虚拟化的群成员列表，以及可定位到原消息的媒体文件（#200） |
| `src/api/chat.ts` | 聊天的读取：会话列表（`/api/chat/sessions`），折叠和免打扰状态（`/api/chat/statuses`，每次 100 个） |
| `src/styles/tokens.css` | 设计令牌：字号、控件高度、圆角、颜色（含深色模式）、新拟态的五种表面（#220）、动效时长和缓动（#192、#193）。页面样式只引用这些变量 |
| `src/motion.ts` | 弹簧预设，`usePresence` 让浮层、对话框和提示播完退出动画再移除 |
| `src/theme.ts` | 设置里选择的颜色主题：跟随系统（默认）、浅色或深色，保存在 `localStorage`；把生效的主题写到 `<html>` 的 `data-theme`。`index.html` 在首次绘制前已经设好，页面不会先闪一下另一种主题（#252） |
| `src/CardStack.tsx` | 操作控制台的卡片：点标题展开或收起；用鼠标拖动或按 Alt+↑ / Alt+↓ 调整顺序。顺序和展开状态保存在浏览器的 `localStorage` 里 |
| `src/VirtualList.tsx` | 只绘制看得见的行的列表，用于成千上万行的列表（#159） |
| `src/Scrollbar.tsx` | 网页自己的滚动条：浮在滚动区域上（隐藏浏览器自带的），不占宽度，可以拖动 |
| `src/HoverTrack.tsx` | 会平移的悬停凹陷（#232）：在一组元素里，指针移到另一个元素时，凹陷从上一个元素滑过去 |
| `src/Popover.tsx` | 依附在控件上的浮层：从控件处展开，不超出屏幕，按 Esc 或点外面收起 |
| `src/calendar/` | 网页自己的日历（#224）：`Calendar.tsx`（按 ARIA grid 模式排的一个月，周一开头，可切换到按月、按年选择），`DateRange.tsx`（在一个日历里选时间范围），`DateField.tsx`（选一天），`dates.ts`（用本地时间的 `YYYY-MM-DD` 表示日期） |
| `src/api/messages.ts` | 会话的消息（服务端解析好的 `chat messages --parsed`，按游标翻页）、群成员，以及给 `<img>`、`<audio>`、`<video>` 用的媒体地址 |
| `src/chat/` | 聊天页（#196）：会话列表，含搜索、筛选、折叠的群聊和方向键切换（#197）；按类型显示消息，往上滚动时加载更早的消息（#198）；搜索卡片，以及跳到某一天或某条搜索结果，之后往下滚动时加载更新的消息（#199） |
| `src/App.tsx`、`src/app.css` | 工作台（#401）：左侧导航，中间是页面，右侧是操作控制台；持有各页面共用的东西（连接、账号、会话列表、任务），负责从一个页面打开另一个页面，并把各页面的卡片汇集到控制台 |
| `src/shell/` | 所有页面共用的外壳：侧边栏与连接状态、媒体预览、整个网页的偏好（实时消息、导出附带媒体、减少动画）、页面的两种形状（`ScrollPage`、`BrowserPanel`），以及 `page.ts`：页面拿到什么（`Shell`）、交回什么（`Page`） |
| `src/mock/` | 未连接服务时显示的示例记录，以及控制台对示例默认使用的日期范围 |
| `src/api/jobs.ts`、`src/jobs/` | 作业（#211）：用 `Prefer: respond-async` 启动命令（导出时带 `?return=file`），读取 `/jobs` 列表，通过 `/jobs/<id>/events` 跟踪正在运行的作业，用 `DELETE` 取消或删除，从 `/jobs/<id>/file` 保存文件；任务中心列出全部作业，控制台用 `export messages` 导出打开的会话 |
| `src/exports/` | 导出页（#211）：从会话列表里勾选（可按类型筛选、一次全选当前列出的），每个会话一个作业，用 `export messages` 导出，可选 9 种格式、日期范围、发送者名称、媒体文件和 Excel 精简列；也可以用 `export sessions` 导出会话列表本身 |
| `src/files.ts` | 网页保存的文件：把多行数据生成 .xlsx，或把任意内容交给浏览器下载 |

每个页面是它自己目录下的一个 hook，`use…Page`（`src/chat/useChatPage.tsx`、`src/contacts/useContactsPage.tsx` 等）：它保存页面的状态，交回页面显示的内容、它在控制台的卡片，以及要显示哪些卡片。App 每次渲染都调用全部页面，所以重新打开一个页面时，它保持离开时的样子；而每个页面只在打开时才读取记录。状态放在用到它的地方：页面的筛选和选择放在页面的 hook 里；几个页面共用的（会话列表、任务、配置档案）放在 App，通过 `Shell` 交给它们；只有一个组件用到的放在那个组件里。一个页面只能通过 App 打开另一个页面（`chatWith`、`openInChat`）。

## 请求与缓存

网页读到的东西都属于一个服务端和一个账号，即它的 scope（`src/api/scope.ts`）。`/health/events` 的 `data` 变了（换了账号或配置，或服务端重新启动），或者服务端断开后又回来，就开始新的 scope：缓存全部清掉，上一个 scope 的请求迟到的回复也不会显示（#198、#402）。

| 缓存什么 | 在哪里 | 按什么区分 | 什么时候重新读 |
|---|---|---|---|
| 会话列表及其状态 | `chat/useSessions.ts`，由 App 持有 | scope；不在别的 scope 里显示，所以不会拿旧账号的会话去问新账号 | 新的 scope；实时消息（不显示加载）；重试 |
| 最多 20 个会话的消息（#159） | `chat/useMessages.ts` | scope 和会话；只缓存读到最新消息的 | 新的 scope；这个会话有实时消息；重新连接（全部） |
| 联系人列表（解析过的和完整的表） | `api/kept.ts`：`contactList`（通讯录页、足迹），以及朋友圈的发布者 | scope 和哪一份列表 | 新的 scope；重试 |
| 私聊分析、群列表 | 页面 hook 里的 `analytics/useAnalytics.ts` | scope | 新的 scope；「重新统计」；不统计的好友有变化 |
| 其余：朋友圈动态、支付、资源、灵感、足迹、报告的年份、设置 | 显示它们的组件 | — | 每次打开页面或卡片时，因为它们会增长 |

几个界面共用的读取都经过 `keptRead`：同时请求的界面共用一个请求。离开的界面不再接收回复（`useRead` 和各个 hook 在 effect 清理后丢掉迟到的回复）；新的搜索或翻页会取消上一个。`src/requests.test.tsx` 固定了常用流程的请求，包括迟到回复的情况；这些请求列表有变化时，要在 PR 里说明。

和工作台的页面结构（#401）的关系：读取数据的 hook 都以 `(enabled, scope)` 调用，多个页面共用的放在 App，其余的放在页面 hook；界面之间要保留的缓存放在它们下面的 `src/api/`，页面不用为此改动。
