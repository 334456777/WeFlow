# WeFlow WebUI

[English](README.md) | 简体中文

`weflow serve` 的网页界面（#130）。用 React + TypeScript 编写，Vite 构建时由 Preact 渲染成静态文件，由 `weflow serve` 托管在 `/ui/` 下（#146），与 `weflow` 二进制分开发布（#148）。

本文件只说明如何开发。其他内容见 Wiki：

- [WebUI 实现](https://github.com/334456777/WeFlow/wiki/WebUI-实现)：目录、页面结构、请求与缓存、语言、API 的类型和测试
- [WebUI](https://github.com/334456777/WeFlow/wiki/WebUI)：托管和版本管理
- [命令 API](https://github.com/334456777/WeFlow/wiki/命令-API)：网页调用的接口
- [发布与 CI](https://github.com/334456777/WeFlow/wiki/发布与-CI#发布-webui)：发布网页

## 开发

需要 Bun 1.4.3+（版本写在 `package.json` 的 `packageManager` 里）和仓库指定的 Rust 工具链（`rustup`，见 `rust-toolchain.toml`），不需要 Node.js。

```sh
cd web
bun install --frozen-lockfile
bun run dev                               # http://localhost:5173/ui/
```

`bun run dev`（`scripts/dev.mjs`，#333）会启动两样东西，停止时一起停：

- **网页**：Vite 直接提供这些源码，和 `bun run build` 做出来的是同一个网页。
- **后端**：一个读合成账号的 `weflow serve`（调试版，先自动 `cargo build`），监听 127.0.0.1 上的一个空闲端口，令牌每次随机生成，由代理加上。合成账号由 `crates/weflow-cli/examples/dev_account.rs` 生成在 `target/dev-account/` 里，第一次约 20 秒，其中的「图片测试」一张图片一种情况，对应微信保存的各种图片文件。不读任何真实微信账号。

设置 `WEFLOW_DEV_ACCOUNT=bench`（在终端或 `web/.env.local` 里）时改用基准账号（`crates/weflow-cli/examples/bench_account.rs`，#488）：一个 20 万条消息的群和 300 个其他会话，第一次生成约需一两分钟，占用约 2.5 GB。

要连别的 `weflow serve`（例如真实账号）时，在终端或 `web/.env.local`（不提交）里设置下面的变量，这时只启动 Vite：

| 变量 | 默认值 | 说明 |
|---|---|---|
| `WEFLOW_SERVE_URL` | 合成账号 | 那个 `weflow serve` 的地址，例如 `http://127.0.0.1:18108` |
| `WEFLOW_HTTP_TOKEN` | 无 | 它的令牌，由代理加上 |

要从另一台电脑或手机打开开发服务器，运行 `bun run dev --host`，再打开 `http://<这台机器的地址>:5173/ui/`。用合成账号时这样做是安全的。设置了 `WEFLOW_SERVE_URL` 和 `WEFLOW_HTTP_TOKEN` 时不要这样做：代理会把令牌交给任何能访问这个端口的人，他们就能对那个账号运行任何 `weflow` 命令。

运行 `bun run build` 后，可以用 `weflow serve --ui-dir web/dist` 打开构建出的网页。

`bun run smoke`（#789）用无头 Chrome 试用这份构建，也就是发布出去的网页（由 Preact 渲染，而 `bun test` 用 React）：它像 `bun run dev` 一样编译并启动合成账号的 `weflow serve`，每项检查在一个新标签页里打开网页、操作，检查显示的文字和元素个数，并要求控制台没有错误。需要安装 Chrome（不在常见位置时用 `CHROME` 指定可执行文件）；失败的检查在 `target/web-smoke/` 留下截图。

## 脚本

| 脚本 | 说明 |
|---|---|
| `bun run dev` | 开发服务器，支持热更新 |
| `bun run typecheck` | `tsc --noEmit` |
| `bun test` | 运行测试（`bun:test`，DOM 由 happy-dom 提供） |
| `bun run build` | 类型检查后构建到 `dist/` |
| `bun run preview` | 用同样的代理预览 `dist/` |
| `bun run smoke` | 在无头 Chrome 里对 `dist/` 做冒烟测试；`bun run smoke <词>` 只跑名字里有这个词的检查 |
| `bun run contract` | 由 `contract/openapi.json` 生成 `src/api/contract.gen.ts`；带 `--check` 时只检查 |
| `bun run i18n` | 列出缺少英文的界面文字、没有文字使用的英文，以及尚未经过 `t()` 的中文；带 `--check` 时前两种不为空就失败 |
| `bun run check` | 检查契约类型和英文，再做类型检查、测试和构建，与 CI 相同（CI 另外把契约与最新发布版对比，并运行 `bun run smoke`） |
