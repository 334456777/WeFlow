# WeFlow WebUI

[English](README.md) | 简体中文

`weflow serve` 的网页界面（#130）。用 React + TypeScript 编写，Vite 构建成静态文件：
- 由 `weflow serve` 托管在 `/ui/` 下（#146）；
- 与 `weflow` 二进制分开发布（#148）。

它调用的 API 见 [docs/zh-CN/serve-api.md](../docs/zh-CN/serve-api.md)。

界面来自 #130 的原型，目前显示的还是示例记录（`src/mock/`）。只有连接状态读的是真实的服务。

## 开发

需要 Node.js 20.19+ 和 npm，CI 用的是 24。先启动 `weflow serve`，再启动开发服务器，两边使用同一个令牌：

```sh
export WEFLOW_HTTP_TOKEN=dev-token        # PowerShell：$env:WEFLOW_HTTP_TOKEN = "dev-token"
weflow serve                              # 或：cargo run -p weflow-cli -- serve

cd web
npm ci
npm run dev                               # http://localhost:5173/ui/
```

开发服务器自己处理 `/ui/`。其他路径都转发给 `weflow serve`，包括 `/chat/sessions`、`/api/…`、`/jobs`、`/health`、`/openapi.json`。转发时加上 `Authorization: Bearer $WEFLOW_HTTP_TOKEN`。令牌只留在代理里，不会编译进网页。这两个变量也可以写在 `web/.env.local` 里（不提交）：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `WEFLOW_SERVE_URL` | `http://127.0.0.1:18108` | `weflow serve` 的地址，例如用了 `serve --port 8080` 时 |
| `WEFLOW_HTTP_TOKEN` | 无 | 代理加上的令牌 |

**只调整界面时**：先运行一次 `npm ci`，之后运行 `npm run dev` 就够了，不需要 `weflow serve`。这时：
- 界面显示示例记录；
- 连接状态显示「未连接」；
- 每次尝试连接 `/health/events`（按 1、2、4 …… 最长 30 秒的间隔重试），终端都会打印一条代理错误，可以忽略。

要从另一台电脑打开开发服务器，运行 `npm run dev -- --host`，再打开 `http://<这台机器的地址>:5173/ui/`。这时不要设置 `WEFLOW_HTTP_TOKEN`。否则，代理会把令牌交给任何能访问这个端口的人，他们就能运行任何 `weflow` 命令。

运行 `npm run build` 后用 `weflow serve --ui-dir web/dist` 打开网页时，网页从地址里取令牌（`/ui/?token=…`）。令牌存在当前标签页的 `sessionStorage` 里，同时从地址栏去掉。

令牌被拒绝（HTTP 401）时，网页提示输入令牌后重新连接。也可以在设置里更换或清除当前标签页的令牌；令牌不写入 `localStorage` 或构建产物。设置同时显示服务端与 API 版本、兼容性和处理建议，以及 `/health` 或状态流的 `hello` 提供的任务、读取、媒体限额和灵感引擎状态（#214）。

## 脚本

| 脚本 | 说明 |
|---|---|
| `npm run dev` | 开发服务器，支持热更新 |
| `npm run typecheck` | `tsc --noEmit` |
| `npm test` | Vitest（jsdom） |
| `npm run build` | 类型检查后构建到 `dist/` |
| `npm run preview` | 用同样的代理预览 `dist/` |
| `npm run check` | 类型检查、测试和构建，与 CI 相同 |

`web/` 下有改动时，`.github/workflows/web.yml` 会运行 `typecheck`、`test` 和 `build`，并上传 `dist/`。

## 版本

`package.json` 记录了 WebUI 自己的版本。`weflow` 字段下记录它需要的 API：
- `apiVersion` 必须等于 `GET /health` 返回的 `apiVersion`；
- 服务端的 `version` 不能低于 `minBackendVersion`。开发构建的 `git describe` 版本按它所在的 tag 计算。

构建时，这些信息写入 `dist/weflow-web.json`，供 `weflow web` 使用（#148）。网页也会拿它们和 `/health/events` 的 `hello` 比对，不满足时显示「版本不兼容」。没有 `/health/events` 的服务端（5.0.0-rc.5 及更早）不再支持，连接时显示「未连接」。

发布新版本时，先提高 `package.json` 里的 `version`，再推送 tag `web-v<版本>`（或在 Actions 里运行 *WebUI release*）：`.github/workflows/web-release.yml` 构建后发布到 `ghcr.io/334456777/weflow-web:<版本>`，`weflow web` 和 `weflow serve` 从那里下载。同一个版本只能发布一次。每个 `weflow` 构建默认显示构建时 `package.json` 里的版本（见 [docs/serve-api.md](../docs/zh-CN/serve-api.md#网页的版本)）。

## 目录

| 路径 | 说明 |
|---|---|
| `src/api/client.ts` | 调用 `weflow serve`：负责查询参数或 JSON 请求体，带上令牌，出错时抛出带服务端错误码的 `ApiError` |
| `src/TokenForm.tsx` | 令牌输入与重新连接提示（#214） |
| `src/api/backend.ts` | 连接状态：保持 `/health/events` 长连接，加上看门狗和退避重连（#221）；打开实时消息时改为 `/api/push/messages`（#201）；检查版本，提供 `useBackend` |
| `src/api/live.ts` | 实时消息事件，分发给会话列表和打开的会话（#201） |
| `src/api/biz.ts` | 公众号：关注的公众号（`/biz/accounts`）和它们的推送（`/biz/messages`）（#203） |
| `src/api/details.ts` | 会话资料、群成员统计、媒体及解析后的单条消息查询；串行执行子进程读取（#200） |
| `src/chat/SessionInfo.tsx`、`src/chat/MediaPanel.tsx` | 展开时读取的控制台卡片：会话资料、虚拟化的群成员列表，以及可定位到原消息的媒体文件（#200） |
| `src/api/chat.ts` | 聊天的读取：会话列表（`/api/chat/sessions`），折叠和免打扰状态（`/chat/statuses`，每次 100 个） |
| `src/styles/tokens.css` | 设计令牌：字号、控件高度、圆角、颜色（含深色模式）、新拟态的五种表面（#220）、动效时长和缓动（#192、#193）。页面样式只引用这些变量 |
| `src/motion.ts` | 弹簧预设，`usePresence` 让浮层、对话框和提示播完退出动画再移除 |
| `src/CardStack.tsx` | 操作控制台的卡片：点标题展开或收起；用鼠标拖动或按 Alt+↑ / Alt+↓ 调整顺序。顺序和展开状态保存在浏览器的 `localStorage` 里 |
| `src/VirtualList.tsx` | 只绘制看得见的行的列表，用于成千上万行的列表（#159） |
| `src/Scrollbar.tsx` | 网页自己的滚动条：浮在滚动区域上（隐藏浏览器自带的），不占宽度，可以拖动 |
| `src/HoverTrack.tsx` | 会平移的悬停凹陷（#232）：在一组元素里，指针移到另一个元素时，凹陷从上一个元素滑过去 |
| `src/Popover.tsx` | 依附在控件上的浮层：从控件处展开，不超出屏幕，按 Esc 或点外面收起 |
| `src/calendar/` | 网页自己的日历（#224）：`Calendar.tsx`（按 ARIA grid 模式排的一个月，周一开头，可切换到按月、按年选择），`DateRange.tsx`（在一个日历里选时间范围），`DateField.tsx`（选一天），`dates.ts`（用本地时间的 `YYYY-MM-DD` 表示日期） |
| `src/api/messages.ts` | 会话的消息（服务端解析好的 `chat messages --parsed`，按游标翻页）、群成员，以及给 `<img>`、`<audio>`、`<video>` 用的媒体地址 |
| `src/chat/` | 聊天页（#196）：会话列表，含搜索、筛选、折叠的群聊和方向键切换（#197）；按类型显示消息，往上滚动时加载更早的消息（#198）；搜索卡片，以及跳到某一天或某条搜索结果，之后往下滚动时加载更新的消息（#199） |
| `src/App.tsx`、`src/app.css` | 工作台界面：左侧导航，中间是记录，右侧是操作控制台 |
| `src/mock/` | 示例记录，界面接入 API 后删除 |
| `src/export.ts` | 在浏览器里导出示例记录。真正的导出将调用 `export messages` 并用 `?return=file` 取回文件 |
