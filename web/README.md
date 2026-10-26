# WeFlow WebUI

English | [简体中文](README.zh-CN.md)

The web interface of `weflow serve` (#130): React + TypeScript, built with Vite into static files that `weflow serve` hosts under `/ui/` (#146) and that are published apart from the `weflow` binary (#148). The API it talks to is described in [docs/serve-api.md](../docs/serve-api.md).

The screens come from the #130 prototype and still show sample records (`src/mock/`); only the connection state reads the real server so far.

## Development

Needs Bun 1.4.3+ (the version is pinned in `packageManager` of `package.json`) and the Rust toolchain of the repository (`rustup`, see `rust-toolchain.toml`); no Node.js. Bun installs the packages from `bun.lock` and runs everything: the scripts, `tsc`, Vite and the tests:

```sh
cd web
bun install --frozen-lockfile
bun run dev                               # http://localhost:5173/ui/
```

`bun run dev` (`scripts/dev.mjs`, #333) starts two things:

- **the page**: Vite serves these sources as they are, the same page `bun run build` makes, nothing added for development;
- **the server**: a `weflow serve` (debug build, `cargo build` first) of a synthetic account, on a free port of 127.0.0.1 and with a random token of its own. The account is made by `crates/weflow-cli/examples/dev_account.rs` in `target/dev-account/` (about 20 s the first time, again only when the generator changes): made-up friends, a group, Moments and 「图片测试」, a chat of one picture per case of the files WeChat keeps of a picture (thumbnail `_t.dat`, image `.dat`, HD original `_h.dat`, each there or not), every picture written with its file and size. Nothing of a real WeChat account is read.

Vite answers `/ui/` itself and passes every other path (`/chat/sessions`, `/api/…`, `/jobs`, `/health`, `/openapi.json`) to that server, adding its token; the page asks for none. Stopping `bun run dev` stops both.

To work against another `weflow serve`, a real account for instance, set it in the shell or `web/.env.local` (not committed); then only Vite starts:

| Variable | Default | |
|---|---|---|
| `WEFLOW_SERVE_URL` | the synthetic account | Where that `weflow serve` listens, for example `http://127.0.0.1:18108` |
| `WEFLOW_HTTP_TOKEN` | none | Its token, which the proxy adds |

To open the dev server from another computer or a phone, run `bun run dev --host` and open `http://<this machine's address>:5173/ui/`. On the synthetic account that is safe. With `WEFLOW_SERVE_URL` and `WEFLOW_HTTP_TOKEN`, do not: the proxy hands the token to everyone who can reach the port, and with it every `weflow` command on that account.

Opened by `weflow serve --ui-dir web/dist` (after `bun run build`), the page gets its token from the address (`/ui/?token=…`), keeps it in `sessionStorage` for the tab and removes it from the address bar.

A refused token (HTTP 401) shows an input to reconnect. Settings can replace or clear the tab's token; it never goes into `localStorage` or the build. Settings also show the server and API versions, compatibility advice, and the server's task/read/media limits and insight status when provided by `/health` or the stream's `hello` (#214).

## Scripts

| Script | |
|---|---|
| `bun run dev` | Dev server with hot reload |
| `bun run typecheck` | `tsc --noEmit` |
| `bun test` | The tests (`bun:test`, with a DOM from happy-dom) |
| `bun run build` | Type-check, then build into `dist/` |
| `bun run preview` | Serve `dist/` with the same proxy |
| `bun run check` | Type-check, test and build, as CI does |

The tests import their API from `#test` (`test/index.ts`): `bun:test`, and a `vi` with what they use of Vitest's that Bun's lacks (`stubGlobal`, `unstubAllGlobals`, `setSystemTime`, `advanceTimersByTimeAsync`, `waitFor`, fake timers for `Date` alone). `test/setup.ts`, loaded first (`bunfig.toml`), registers happy-dom and keeps it to what the tests were written for under jsdom: a wide window, the runtime's own `fetch`, a `WheelEvent` that is a `MouseEvent`, `display: inline` for inline elements, and short failure messages for DOM nodes. Bun runs every test file in one process, so a test leaves no global behind: `vi.unstubAllGlobals()`, `vi.useRealTimers()` and `cleanup()` in `afterEach`.

`.github/workflows/web.yml` runs `typecheck`, `test` and `build` for changes under `web/`, and uploads `dist/`.

The build fails when the JavaScript the page loads before it shows is over the budget in `plugins/sizeBudget.ts` (#159); raise it there on purpose when a change needs more. Icons keep only the weights the app draws (`plugins/iconWeights.ts`: regular, fill, bold); a test fails when the code asks for another.

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
| `src/api/biz.ts` | Official accounts: the ones followed (`/biz/accounts`) and their pushes (`/biz/messages`) (#203) |
| `src/api/sns.ts`, `src/moments/` | Moments (#202): the timeline, who posts and how often, the totals (`/api/sns/timeline`, `post-counts`, `stats`), and the pictures and videos of a post through `/api/sns/media`; the page beside the timeline lists the posters, the console narrows it by words and days |
| `src/api/pay.ts`, `src/payments/` | WeChat Pay (#204): every notice of the WeChat Pay account, read a page after another (`/biz/pay-records`), with the amount, method, state and payee taken from its lines; the list beside them sums up each payee, the console narrows them by days, adds them up and exports them as HTML, JSON or Excel |
| `src/api/details.ts` | Conversation details, group member statistics, media and parsed message lookups; serializes command reads (#200) |
| `src/chat/SessionInfo.tsx`, `src/chat/MediaPanel.tsx` | Console cards loaded while open: conversation facts, virtualized group members, and media that open at their message (#200) |
| `src/api/chat.ts` | Chat reads: the conversation list (`/api/chat/sessions`), and the folded and muted states (`/chat/statuses`, in batches of 100) |
| `src/styles/tokens.css` | Design tokens: type, control heights, radii, colours (with dark mode), the five neumorphic surfaces (#220), motion durations and curves (#192, #193). Page styles use only these variables |
| `src/motion.ts` | Spring presets; `usePresence` keeps popovers, dialogs and notices until their exit has played |
| `src/theme.ts` | The colour theme chosen in the settings: follow the system (default), light or dark, kept in `localStorage`; sets `data-theme` on `<html>`, which `index.html` already sets before the first paint (#252) |
| `src/CardStack.tsx` | The console's cards: the header opens and closes a card; drag it with the mouse or press Alt+↑ / Alt+↓ to reorder. Order and open state stay in the browser's `localStorage` |
| `src/VirtualList.tsx` | A list that draws only the rows in sight, for lists of thousands of rows (#159) |
| `src/Scrollbar.tsx` | The page's own scrollbar: floats over a scroll area (the browser's is hidden), takes no width, can be dragged |
| `src/HoverTrack.tsx` | The hover hollow that slides (#232): in a group of items, the pointer crossing into another item slides the hollow over from the last one |
| `src/Popover.tsx` | A floating layer anchored to a control: grows out of it, stays on screen, closes on Esc or a click outside |
| `src/calendar/` | The page's own calendar (#224): `Calendar.tsx` (a month in the ARIA grid pattern, Monday first, month and year views), `DateRange.tsx` (a range picked in one calendar), `DateField.tsx` (one day), `dates.ts` (days as `YYYY-MM-DD` in local time) |
| `src/api/messages.ts` | Messages of a conversation as the server parses them (`chat messages --parsed`, cursor pages), group members, and media addresses for `<img>`, `<audio>`, `<video>` |
| `src/chat/` | The chat page (#196): the conversation list with search, filters, folded groups and arrow keys (#197); the messages of each kind, paged back as the window scrolls up (#198); the search card and the jump to a day or a search hit, with newer pages loaded below (#199) |
| `src/App.tsx`, `src/app.css` | The workspace (#401): navigation on the left, a page in the middle, the operations console on the right; it holds what the pages share (the connection, the account, the conversation list, the jobs), opens one page from another and gathers the pages' cards into the console |
| `src/shell/` | What every page is in: the sidebar with the connection indicator, the media preview, the preferences of the whole page (live messages, media in exports, reduced motion), the two shapes of a page (`ScrollPage`, `BrowserPanel`), and `page.ts`, what a page is given (`Shell`) and gives back (`Page`) |
| `src/mock/` | Sample records, until the screens read the API |
| `src/api/jobs.ts`, `src/jobs/` | Jobs (#211): commands started with `Prefer: respond-async` (`?return=file` for an export), the list of `/jobs`, the running one followed through `/jobs/<id>/events`, cancelled or forgotten with `DELETE`, its file saved from `/jobs/<id>/file`; the task centre shows them all, and the console exports the open conversation with `export messages` |
| `src/exports/` | The export page (#211): conversations picked from the list (by kind, all those shown at once), exported one job each with `export messages` in any of its 9 formats, days, sender names, media and Excel's compact columns; and the list itself with `export sessions` |
| `src/files.ts` | Files the page saves: an .xlsx of rows, any bytes as a download |

Each page is a hook in its own folder, `use…Page` (`src/chat/useChatPage.tsx`, `src/contacts/useContactsPage.tsx`, …): it keeps the page's state and returns what the page shows, its cards of the console and the cards it shows. App calls them all on every render, so a page is as it was left when it is opened again, while each reads its records only when open. State sits where it is used: a page's filters and picks in its hook; what several pages use (the conversation list, the jobs, the profiles) in App, which hands it to them in `Shell`; what one component alone needs in that component. One page opens another only through App (`chatWith`, `openInChat`).

## Requests and what is kept

What the page reads belongs to one server and one account, its scope (`src/api/scope.ts`). A new scope starts when the `data` of `/health/events` changes (another account or configuration, or the server started again) and when the server comes back after it was gone: everything kept is forgotten, and an answer to a request of the scope before is not shown (#198, #402).

| Kept | Where | For | Read again on |
|---|---|---|---|
| The conversation list and its states | `chat/useSessions.ts`, held by App | the scope; none is shown in another, so nothing of the account before is asked of the new one | a new scope; live messages (quietly); retry |
| The messages of up to 20 conversations (#159) | `chat/useMessages.ts` | the scope and the conversation; only threads that reach the newest message | a new scope; a live message of the conversation; a reconnection (all of them) |
| The contact lists, parsed and whole | `api/kept.ts`: `contactList` (contacts page, footprint), the posters of Moments | the scope and the list | a new scope; retry |
| Private chat analytics, the group list | `analytics/useAnalytics.ts` in the page's hook | the scope | a new scope; 「重新统计」; the people left out changed |
| The rest: the Moments timeline, payments, resources, insights, the footprint, the reports' years, the settings | the component that shows them | — | each time the page or card is opened, as they grow |

A read that several views share goes through `keptRead`: views asking at the same time share one request. A view that is left lets its answers go (`useRead` and the hooks drop an answer that comes after their effect was cleaned up), and a new search or page cancels the one before. `src/requests.test.tsx` holds the requests of the usual flows, late answers among them; a change of those lists is explained in its PR.

With the pages of the workspace (#401), the hooks that read are called with `(enabled, scope)`: those whose records several pages use in App, the others in the page's hook; what is kept between views sits under them, in `src/api/`, so a page does not change for it.
