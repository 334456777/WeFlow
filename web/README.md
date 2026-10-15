# WeFlow WebUI

[简体中文](README.zh-CN.md)

The web interface of `weflow serve` (#130): React + TypeScript, built with Vite into static files that `weflow serve` hosts under `/ui/` (#146) and that are published apart from the `weflow` binary (#148). The API it talks to is described in [docs/serve-api.md](../docs/serve-api.md).

The screens come from the #130 prototype and still show sample records (`src/mock/`); only the connection state reads the real server so far.

## Development

Needs Node.js 20.19+ (CI uses 24) and npm. Start `weflow serve`, then the dev server, with the same token in both:

```sh
export WEFLOW_HTTP_TOKEN=dev-token        # PowerShell: $env:WEFLOW_HTTP_TOKEN = "dev-token"
weflow serve                              # or: cargo run -p weflow-cli -- serve

cd web
npm ci
npm run dev                               # http://localhost:5173/ui/
```

The dev server answers `/ui/` itself and passes every other path (`/chat/sessions`, `/api/…`, `/jobs`, `/health`, `/openapi.json`) to `weflow serve`, adding `Authorization: Bearer $WEFLOW_HTTP_TOKEN`. The token stays in the proxy and is never compiled into the app. Settings can also go in `web/.env.local` (not committed):

| Variable | Default | |
|---|---|---|
| `WEFLOW_SERVE_URL` | `http://127.0.0.1:57108` | Where `weflow serve` listens, for example after `serve --port 18108` |
| `WEFLOW_HTTP_TOKEN` | none | The token the proxy adds |

Opened by `weflow serve` (once #146 is done), the page gets its token from the address (`/ui/?token=…`), keeps it in `sessionStorage` for the tab and removes it from the address bar.

## Scripts

| Script | |
|---|---|
| `npm run dev` | Dev server with hot reload |
| `npm run typecheck` | `tsc --noEmit` |
| `npm test` | Vitest (jsdom) |
| `npm run build` | Type-check, then build into `dist/` |
| `npm run preview` | Serve `dist/` with the same proxy |
| `npm run check` | Type-check, test and build, as CI does |

`.github/workflows/web.yml` runs `typecheck`, `test` and `build` for changes under `web/`, and uploads `dist/`.

## Versions

`package.json` holds the WebUI's own version and, under `weflow`, the API it needs: `apiVersion` must equal the `apiVersion` of `GET /health`, and the server's `version` must be at least `minBackendVersion` (a development build's `git describe` version counts as its tag). The build writes these to `dist/weflow-web.json` for `weflow web` (#148); the page checks them against `/health` and shows "版本不兼容" otherwise.

## Layout

| Path | |
|---|---|
| `src/api/client.ts` | Calls to `weflow serve`: query or JSON body, token, `ApiError` with the server's error code |
| `src/api/backend.ts` | `/health`, version checks, `useBackend` |
| `src/App.tsx`, `src/app.css` | The workspace screens |
| `src/mock/` | Sample records, until the screens read the API |
| `src/export.ts` | Sample export in the browser; real exports will use `export messages` with `?return=file` |
