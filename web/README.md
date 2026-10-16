# WeFlow WebUI

English | [简体中文](README.zh-CN.md)

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
| `WEFLOW_SERVE_URL` | `http://127.0.0.1:18108` | Where `weflow serve` listens, for example after `serve --port 8080` |
| `WEFLOW_HTTP_TOKEN` | none | The token the proxy adds |

**Working on the screens only**: `npm ci` once, then `npm run dev` is enough; `weflow serve` is not needed. The screens show the sample records, the connection state says "未连接", and the terminal reports a proxy error for each try to connect to `/health/events` (after 1, 2, 4 … at most 30 s), which can be ignored.

To open the dev server from another computer, run `npm run dev -- --host` and open `http://<this machine's address>:5173/ui/`. Do it without `WEFLOW_HTTP_TOKEN`: with it, the proxy hands the token to everyone who can reach the port, and with it every `weflow` command.

Opened by `weflow serve --ui-dir web/dist` (after `npm run build`), the page gets its token from the address (`/ui/?token=…`), keeps it in `sessionStorage` for the tab and removes it from the address bar.

A refused token (HTTP 401) shows an input to reconnect. Settings can replace or clear the tab's token; it never goes into `localStorage` or the build. Settings also show the server and API versions, compatibility advice, and the server's task/read/media limits and insight status when provided by `/health` or the stream's `hello` (#214).

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

`package.json` holds the WebUI's own version and, under `weflow`, the API it needs: `apiVersion` must equal the `apiVersion` of `GET /health`, and the server's `version` must be at least `minBackendVersion` (a development build's `git describe` version counts as its tag). The build writes these to `dist/weflow-web.json` for `weflow web` (#148); the page checks them against the `hello` of `/health/events` and shows "版本不兼容" otherwise. A server without `/health/events` (5.0.0-rc.5 and older) is not supported: the page shows "未连接".

To publish a version, raise `version` in `package.json` and push the tag `web-v<version>` (or run *WebUI release* in Actions): `.github/workflows/web-release.yml` builds it and publishes it to `ghcr.io/334456777/weflow-web:<version>`, where `weflow web` and `weflow serve` find it. A version is published once. Each `weflow` build shows, by default, the version `package.json` had when it was built (see [docs/serve-api.md](../docs/serve-api.md#versions-of-the-webui)).

## Layout

| Path | |
|---|---|
| `src/api/client.ts` | Calls to `weflow serve`: query or JSON body, token, `ApiError` with the server's error code |
| `src/TokenForm.tsx` | Token input and reconnection prompt (#214) |
| `src/api/backend.ts` | The connection: one `/health/events` stream with a watchdog and backoff (#221), or `/api/push/messages` with live messages on (#201); version checks, `useBackend` |
| `src/api/live.ts` | Live message events, handed to the conversation list and the open conversation (#201) |
| `src/api/chat.ts` | Chat reads: the conversation list (`/api/chat/sessions`), and the folded and muted states (`/chat/statuses`, in batches of 100) |
| `src/styles/tokens.css` | Design tokens: type, control heights, radii, colours (with dark mode), the five neumorphic surfaces (#220), motion durations and curves (#192, #193). Page styles use only these variables |
| `src/motion.ts` | Spring presets; `usePresence` keeps popovers, dialogs and notices until their exit has played |
| `src/CardStack.tsx` | The console's cards: the header opens and closes a card; drag it with the mouse or press Alt+↑ / Alt+↓ to reorder. Order and open state stay in the browser's `localStorage` |
| `src/VirtualList.tsx` | A list that draws only the rows in sight, for lists of thousands of rows (#159) |
| `src/Scrollbar.tsx` | The page's own scrollbar: floats over a scroll area (the browser's is hidden), takes no width, can be dragged |
| `src/HoverTrack.tsx` | The hover hollow that slides (#232): in a group of items, the pointer crossing into another item slides the hollow over from the last one |
| `src/Popover.tsx` | A floating layer anchored to a control: grows out of it, stays on screen, closes on Esc or a click outside |
| `src/calendar/` | The page's own calendar (#224): `Calendar.tsx` (a month in the ARIA grid pattern, Monday first, month and year views), `DateRange.tsx` (a range picked in one calendar), `DateField.tsx` (one day), `dates.ts` (days as `YYYY-MM-DD` in local time) |
| `src/api/messages.ts` | Messages of a conversation as the server parses them (`chat messages --parsed`, cursor pages), group members, and media addresses for `<img>`, `<audio>`, `<video>` |
| `src/chat/` | The chat page (#196): the conversation list with search, filters, folded groups and arrow keys (#197); the messages of each kind, paged back as the window scrolls up (#198); the search card and the jump to a day or a search hit, with newer pages loaded below (#199) |
| `src/App.tsx`, `src/app.css` | The workspace: navigation on the left, records in the middle, the operations console on the right |
| `src/mock/` | Sample records, until the screens read the API |
| `src/export.ts` | Sample export in the browser; real exports will use `export messages` with `?return=file` |
