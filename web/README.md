# WeFlow WebUI

English | [简体中文](README.zh-CN.md)

The web interface of `weflow serve` (#130): React + TypeScript, rendered by Preact in the Vite build, into static files that `weflow serve` hosts under `/ui/` (#146) and that are published apart from the `weflow` binary (#148).

This file covers development only. The rest is in the wiki, in Chinese:

- [WebUI 实现](https://github.com/334456777/WeFlow/wiki/WebUI-实现): source layout, page structure, requests and caching, language, the API's types and the tests
- [WebUI](https://github.com/334456777/WeFlow/wiki/WebUI): hosting and versions
- [命令 API](https://github.com/334456777/WeFlow/wiki/命令-API): the API the page calls
- [发布与 CI](https://github.com/334456777/WeFlow/wiki/发布与-CI#发布-webui): publishing the page

## Development

Needs Bun 1.4.3+ (the version is pinned in `packageManager` of `package.json`); no Node.js. The synthetic accounts and `bun run smoke` also need the Rust toolchain of the repository (`rustup`, see `rust-toolchain.toml`).

```sh
cd web
bun install --frozen-lockfile
bun run dev                               # http://localhost:5173/ui/
```

`bun run dev` (`scripts/dev.mjs`, #333) starts Vite alone, which serves these sources as they are, the same page `bun run build` makes. No server is behind it, so the page shows the sample records (`src/mock`, #879), as it does wherever no `weflow serve` answers: settings, jobs and insights change and run in the page's memory, and a reload brings the sample back.

With `WEFLOW_DEV_ACCOUNT` (in the shell or `web/.env.local`) it also starts a server of a synthetic account, and stopping it stops them all:

- **the server**: a `weflow serve` (debug build, `cargo build` first) on a free port of 127.0.0.1 with a random token, which the proxy adds. Nothing of a real WeChat account is read.
  - `WEFLOW_DEV_ACCOUNT=dev`: the account made by `crates/weflow-cli/examples/dev_account.rs` in `target/dev-account/` (about 20 s the first time); its 「图片测试」 chat has one picture per case of the files WeChat keeps of a picture.
  - `WEFLOW_DEV_ACCOUNT=bench`: the benchmark account (`crates/weflow-cli/examples/bench_account.rs`, #488), a group of 200,000 messages and 300 other conversations. The first build takes a minute or two and about 2.5 GB.
- **a fake model API**: `crates/weflow-fake-model`, which answers as OpenAI's and Anthropic's API documents describe, on a free port of 127.0.0.1. Unless the synthetic account's AI settings name another service, they point at it (model `fake-chat`), so the settings' test and model list and the manual insight of the inbox work as they are; a model such as `fake-rate-limit` or `fake-refusal` shows the errors, and the settings list them all (Settings → AI insights → list the models).

To work against another `weflow serve`, a real account for instance, set these in the shell or `web/.env.local` (not committed); then only Vite starts, and its proxy hands everything outside `/ui/` to that server:

| Variable | Default | |
|---|---|---|
| `WEFLOW_SERVE_URL` | none: no proxy | Where that `weflow serve` listens, for example `http://127.0.0.1:18108` |
| `WEFLOW_HTTP_TOKEN` | none | Its token, which the proxy adds |

To open the dev server from another computer or a phone, run `bun run dev --host` and open `http://<this machine's address>:5173/ui/`. With the sample records or a synthetic account that is safe. With `WEFLOW_SERVE_URL` and `WEFLOW_HTTP_TOKEN`, do not: the proxy hands the token to everyone who can reach the port, and with it every `weflow` command on that account.

After `bun run build`, `weflow serve --ui-dir web/dist` opens the built page.

`bun run smoke` (#789) tries that build in headless Chrome, the page as it is released (rendered by Preact, while `bun test` runs React): it builds and starts a `weflow serve` of the synthetic account as `bun run dev` does with `WEFLOW_DEV_ACCOUNT=dev`, and each check opens the page in a new tab, works it, and checks the text and the number of elements shown and that the console has no error. It needs Chrome (`CHROME` names its executable when it is not where it is usually installed); a check that fails leaves a screenshot in `target/web-smoke/`.

## Scripts

| Script | |
|---|---|
| `bun run dev` | Dev server with hot reload |
| `bun run typecheck` | `tsc --noEmit` |
| `bun test` | The tests (`bun:test`, with a DOM from happy-dom) |
| `bun run build` | Type-check, then build into `dist/` |
| `bun run preview` | Serve `dist/` with the same proxy |
| `bun run smoke` | The smoke test of `dist/` in headless Chrome; `bun run smoke <word>` runs only the checks with that word in their name |
| `bun run contract` | Write `src/api/contract.gen.ts` from `contract/openapi.json`; `--check` only checks it |
| `bun run i18n` | List the page's texts without their English, the English entries no text uses, and the Chinese not passed through `t()` yet; `--check` fails on the first two |
| `bun run jsdoc` | List the exports of `src/` without a JSDoc comment right above them, and fail if there is any |
| `bun run check` | Check the types of the contract, the English and the comments of the exports, type-check, test and build, as CI does (CI also compares the contract with the latest release's and runs `bun run smoke`) |
