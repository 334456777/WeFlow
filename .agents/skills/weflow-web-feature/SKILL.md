---
name: weflow-web-feature
description: 给 WebUI（web/）加页面、读取或改界面时的步骤和容易漏的地方：src/api 的类型与读取方式、作用域和缓存、实时消息后的重读、示例服务端（src/mock）的回答和示例内容、界面文字的 t() 与英文、页面样式和角色类名、JSDoc、请求列表测试和冒烟测试。在 web/src 里写或改页面、组件、hook、api 调用或示例数据时使用。
---

# 改 WebUI 的功能

规则（渲染、样式的层叠层与角色、皮肤、语言）见 [CLAUDE.md](../../../CLAUDE.md) 的「WebUI（`web/`）」，设计和原因见 Wiki [WebUI 实现](https://github.com/334456777/WeFlow/wiki/WebUI-实现)，脚本见 [`web/README.zh-CN.md`](../../../web/README.zh-CN.md#脚本)。看页面效果和截图见 [weflow-web-check](../weflow-web-check/SKILL.md)，加皮肤见 [weflow-web-skin](../weflow-web-skin/SKILL.md)。

## 读取

- 要读的数据没有路由时，先在服务端加：[weflow-cli-command](../weflow-cli-command/SKILL.md) 加命令，[weflow-contract](../weflow-contract/SKILL.md) 重新生成契约，再 `bun run contract` 生成 `src/api/contract.gen.ts`。
- 调用都经过 `src/api/client.ts` 的 `call`。有 `/api/` 进程内路由的读取走 `/api/`，见 Wiki [WebUI 实现#请求与状态](https://github.com/334456777/WeFlow/wiki/WebUI-实现#请求与状态)。
- 回复的类型从 `contract.gen.ts` 取。用 `fields` 只取部分键时，键的列表写成 `as const satisfies readonly (keyof Contract.Xxx)[]`，契约改了键名时类型检查会报出（例子见 `src/api/chat.ts`）。
- 选读取的方式：
  - 只给一个视图用：`useRead`，键变化时取消上一次请求。
  - 多个视图共用、只随服务端和账号变化：`api/kept.ts` 的 `keptRead`。
  - 自己保存记录的模块：用 `api/scope.ts` 的 `onNewScope` 登记清空；保存答复前，用 `currentScope()` 比对请求开始时的作用域，旧作用域的答复不保存。
  - 新加的缓存写进 Wiki [WebUI 实现#缓存](https://github.com/334456777/WeFlow/wiki/WebUI-实现#缓存)的表。
- 实时消息、断线恢复后的重读，下面这些地方容易出错：
  - 重读时保留已经加载的更早内容，不先清空、不闪骨架屏。
  - 同一份数据只请求一次；在事件之前开始的读取，结果不进缓存。
  - 「加载更多」之类的分页请求，在会话、筛选条件变化时一起取消。
  - 连续的事件合并成一次读取并限制频率，不被源源不断的事件一直推迟。
- 断开后页面继续显示已读到的记录：显示什么以 `Shell` 交给页面的 `records` 为准；要服务端执行操作的入口以 `connected` 为准，示例服务端也能回答的入口以 `usable` 为准。见 Wiki [WebUI 实现#断开后的记录](https://github.com/334456777/WeFlow/wiki/WebUI-实现#断开后的记录)。

## 示例数据

没连上服务端时由 `src/mock/` 回答，设计见 Wiki [WebUI 实现#示例数据](https://github.com/334456777/WeFlow/wiki/WebUI-实现#示例数据)。

- 新的读取在 `src/mock/server.ts` 的路由表 `table` 里登记，键是「方法 路径」（如 `'GET api/chat/sessions'`），失败返回 `Failure`。键对应的命令（去掉 `api/`、斜杠换成空格）要在 `contract.gen.ts` 的 `Answers` 里，回答按它的类型检查；报「not assignable to type 'Failure'」说明命令名拼错了或契约里没有这条命令，契约之外的例外只有 `health` 和 `config get`（`server.ts` 的 `Answer`）。没有登记的路由回答 `refused`，页面显示「这是示例数据，连接 weflow serve 后才能这样做。」。以 `Prefer: respond-async` 发出的请求作为作业由 `mock/jobs.ts` 运行，成功后要改动示例状态的写进 `effectOf`。
- 示例服务端是单独的 chunk：从页面代码只能 `import type`，`plugins/plainImport.ts` 在构建时检查。
- 时间用 `mock/clock.ts` 的 `ago`、`at`、`daysSince` 从打开网页的时刻往前推算，不写死日期。
- 内容围绕 WeFlow、WebUI 和账号主人的生活，聊天、朋友圈、支付、报告和见解互相呼应；人名、微信号、地址都是虚构的。排行或见解里出现的人要有私聊。
- 示例只模拟交互，不重新实现真实功能：
  - 导出、备份只给一个说明文本，不在浏览器里按格式生成文件。
  - 不手写命令列表，不写死版本号；这类功能在示例时显示「连接后可用」。
  - 原因：重新实现的代码会和真实实现逐渐不一致，也不代表真实的效果（[#896](https://github.com/334456777/WeFlow/issues/896)）。
- 改动只保存在页面的内存里，刷新后复原；行为由 `mock/server.test.ts` 测试。

## 界面文字

- 写成 `t('中文')`，英文加进 `src/i18n/en.ts`，再运行 `bun run i18n`：它列出缺少的英文、多余的英文和还没进 `t()` 的中文。
- 拼接句子用的中文标点（`、`、`：`、括号）也要经过 `t()`，否则英文界面里会夹着中文标点。
- 模块顶层的常量用 `msg()` 标出、显示时再 `t()`：界面的语言随连上的服务端变化，模块顶层调用 `t()` 只会得到加载时的语言。
- 从读取的数据里比对的中文（如微信支付消息的标签）在那一行写 `// i18n-ignore: <原因>`。

## 样式

- 只属于一个页面的规则写在页面目录下的 CSS 文件里，整个包在 `@layer pages { … }` 中，由页面的模块导入。选择器只写页面自己的类名，不为提高优先级加上共用的类名。
- 按钮、可选的项、列表行、勾选框加上 `as-button`、`as-control`、`as-option`、`as-row` 等角色类名；页面只写布局和文字，颜色和阴影用角色令牌（如 `--row-bg-selected`），不用 `--neu-*`。这些元素的 `::before` 留给皮肤的外形层，页面不用。`styles/roles.test.ts`、`styles/layers.test.ts` 检查这几点。
- 页面元素在窄屏或减少动画时要变的样式，写在页面自己的样式表里：`app.css` 的媒体查询在更低的层，盖不过页面规则。
- 换页、换右侧详情时用 `motion.ts` 的 `pageIn` 播放进入动画。
- 每帧都运行的脚本先读后写，只在值变化时写，见 Wiki [WebUI 实现#性能](https://github.com/334456777/WeFlow/wiki/WebUI-实现#性能)。从 `react`、`react-dom` 只用具名导入。

## 测试和检查

- 测试从 `#test` 导入。改过的全局状态在 `afterEach` 里还原（`vi.unstubAllGlobals()`、`vi.useRealTimers()`、`cleanup()`），不直接改 `URL` 等全局对象：Bun 在同一个进程里运行所有测试文件。
- 常用流程发出的请求变了：更新 `src/requests.test.tsx`，并在 PR 正文里说明请求有什么变化。
- 新的导出项前写一行 JSDoc，`bun run jsdoc` 检查。
- 推送前在 `web/` 下运行 `bun run check`。改了交互还要运行 `bun run build` 和 `bun run smoke <词>`：`bun test` 用 React 渲染，发布的构建用 Preact，两者的更新时机和事件不同。新的主要流程在 `smoke/checks.ts` 里加一项检查。
- 用户看得到的变化写进 Wiki [WebUI 使用](https://github.com/334456777/WeFlow/wiki/WebUI-使用)，实现写进 [WebUI 实现](https://github.com/334456777/WeFlow/wiki/WebUI-实现)，写法见 [weflow-wiki](../weflow-wiki/SKILL.md)。
