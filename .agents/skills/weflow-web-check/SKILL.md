---
name: weflow-web-check
description: 自己检查 WebUI（web/）的改动：启动开发页（示例数据或合成账号）、用无头 Chrome 和 web/smoke/cdp.ts 操作页面和截图、测手机宽度和深色模式、比较改动前后的计算样式和截图，以及收尾时只结束自己启动的浏览器。改了网页、要看页面效果、要截图或排查网页的问题时使用。
---

# 检查网页

脚本见 [`web/README.zh-CN.md`](../../../web/README.zh-CN.md)，测试和开发页的设计见 Wiki [WebUI 实现#测试](https://github.com/334456777/WeFlow/wiki/WebUI-实现#测试)。检查不需要维护者动手：自己启动、操作、截图，再把结论和截图交给维护者。

## 开发页

- `cd web && bun run dev`：只启动 Vite，网页显示示例数据（`src/mock/`），不需要微信账号，也不编译 Rust。
- 需要真实的接口行为、「图片测试」会话或英文界面时，用 `WEFLOW_DEV_ACCOUNT=dev bun run dev`：同时启动读合成账号的 `weflow serve`（带随机令牌）和假模型服务。在 worktree 里同样设 `CARGO_TARGET_DIR`。
- 要在别的设备上打开时，加 `--host 0.0.0.0 --port <端口> --strictPort`，地址写 IP：主机名会被 Vite 的 host 检查拒绝（403）。监听 `0.0.0.0` 的开发页绝不连读真实账号的服务端。
- 用完停掉开发服务器，确认它的整个进程树已经结束。

## 无头 Chrome

- 临时的探查脚本直接 import `web/smoke/cdp.ts`（`Browser.launch`、`newPage`、`click`、`wheel`、`until`、`screenshot`），不手写 CDP 客户端；`web/scripts/account.ts` 启动合成账号的服务端。脚本用完删除。`bun run smoke <词>` 只运行名字含这个词的冒烟检查。Chrome 不在常见位置时用环境变量 `CHROME` 指定。
- 每次加载页面都开新标签页，用完关掉：页面占一条 `/api/push/messages` 长连接，同一浏览器的连接数有上限（[#799](https://github.com/334456777/WeFlow/issues/799)）。
- `--headless=new` 中同一窗口只有最前面的标签页是 `visible`，其余是 `hidden`（`Page.bringToFront` 切换），可以借此测试与页面可见性有关的行为。
- 视口用 `Emulation.setDeviceMetricsOverride`，手机再加 `mobile: true` 和 `Emulation.setTouchEmulationEnabled`；默认视口是窄屏布局，测桌面布局先设成 1440×900。深色用 `Emulation.setEmulatedMedia`。真实的点击和拖动用 `Input.dispatchMouseEvent`，触屏用 `Input.dispatchTouchEvent`，测量用 `Runtime.evaluate`。
- 聊天窗口用 `mouseWheel` 事件滚动：`scrollIntoView` 会被贴底的逻辑拉回去。
- 看动画：`Animation.enable` 后用 `Animation.setPlaybackRate`（如 0.1）放慢再连续截图，JS 的定时器不受影响。下载用 `Browser.setDownloadBehavior` 指定目录。
- 覆盖桌面宽度、手机宽度（390）和深色模式。
- 截图用示例数据或合成账号，不会带出真实数据；必须连真实账号时只截需要的区域。
- Linux 上（包括 WSL）给 Chrome 加 `--password-store=basic`，否则 keyring 锁着时页面一直加载不完；在同一个脚本里启动服务、截图和清理。
- 收尾只结束自己启动的 Chrome（命令行里带自己的临时 profile 目录的进程），然后删掉临时 profile。

## 改样式而不该改变外观

- 改动前后各运行一次 `bun run shots <名字>`，再用 `bun run shots --compare <a> <b>` 逐像素比较；只比较同一台机器、同一天截的图。
- 比较前后的计算样式时逐个属性比较，不比较整串：样式表的顺序一变，`getComputedStyle` 里自定义属性的枚举顺序也变，整串就全都不同。截图和计算样式只覆盖画出来的状态，悬停、没打开的对话框等状态要另外按选择器和优先级核对。
- 改动前的截图在另一个 worktree 里用 `main` 构建，不在当前工作区来回切换。
