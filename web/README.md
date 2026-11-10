# WeFlow WebUI

English | [简体中文](README.zh-CN.md)

The web interface of `weflow serve` (#130): React + TypeScript, built with Vite into static files that `weflow serve` hosts under `/ui/` (#146) and that are published apart from the `weflow` binary (#148).

This file covers development only. The rest is in the wiki, in Chinese:

- [WebUI 实现](https://github.com/334456777/WeFlow/wiki/WebUI-实现): layout, page structure, requests and caching, language, the API's types and the tests
- [WebUI](https://github.com/334456777/WeFlow/wiki/WebUI): hosting and versions
- [命令 API](https://github.com/334456777/WeFlow/wiki/命令-API): the API the page calls
- [发布与 CI](https://github.com/334456777/WeFlow/wiki/发布与-CI#发布-webui): publishing the page

## Development

Needs Bun 1.4.3+ (the version is pinned in `packageManager` of `package.json`) and the Rust toolchain of the repository (`rustup`, see `rust-toolchain.toml`); no Node.js.

```sh
cd web
bun install --frozen-lockfile
bun run dev                               # http://localhost:5173/ui/
```

`bun run dev` (`scripts/dev.mjs`, #333) starts two things, and stopping it stops both:

- **the page**: Vite serves these sources as they are, the same page `bun run build` makes.
- **the server**: a `weflow serve` (debug build, `cargo build` first) of a synthetic account, on a free port of 127.0.0.1 with a random token, which the proxy adds. The account is made by `crates/weflow-cli/examples/dev_account.rs` in `target/dev-account/` (about 20 s the first time); its 「图片测试」 chat has one picture per case of the files WeChat keeps of a picture. Nothing of a real WeChat account is read.

With `WEFLOW_DEV_ACCOUNT=bench` (in the shell or `web/.env.local`) it serves the benchmark account instead (`crates/weflow-cli/examples/bench_account.rs`, #488): a group of 200,000 messages and 300 other conversations. The first build takes a minute or two and about 2.5 GB.

To work against another `weflow serve`, a real account for instance, set these in the shell or `web/.env.local` (not committed); then only Vite starts:

| Variable | Default | |
|---|---|---|
| `WEFLOW_SERVE_URL` | the synthetic account | Where that `weflow serve` listens, for example `http://127.0.0.1:18108` |
| `WEFLOW_HTTP_TOKEN` | none | Its token, which the proxy adds |

To open the dev server from another computer or a phone, run `bun run dev --host` and open `http://<this machine's address>:5173/ui/`. On the synthetic account that is safe. With `WEFLOW_SERVE_URL` and `WEFLOW_HTTP_TOKEN`, do not: the proxy hands the token to everyone who can reach the port, and with it every `weflow` command on that account.

After `bun run build`, `weflow serve --ui-dir web/dist` opens the built page.

## Scripts

| Script | |
|---|---|
| `bun run dev` | Dev server with hot reload |
| `bun run typecheck` | `tsc --noEmit` |
| `bun test` | The tests (`bun:test`, with a DOM from happy-dom) |
| `bun run build` | Type-check, then build into `dist/` |
| `bun run preview` | Serve `dist/` with the same proxy |
| `bun run contract` | Write `src/api/contract.gen.ts` from `contract/openapi.json`; `--check` only checks it |
| `bun run i18n` | List the page's texts without their English, the English no text uses any more, and the Chinese not passed through `t()` yet; `--check` fails on the first two |
| `bun run check` | Check the types of the contract and the English, type-check, test and build, as CI does (CI also compares the contract with the latest release's) |
