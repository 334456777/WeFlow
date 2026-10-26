# weflow serve: the command API

**English** | [简体中文](zh-CN/serve-api.md)

`weflow serve` makes every CLI command available over HTTP. The path is the command path, the parameters are the command's options, and the answer is what the terminal gets: `GET /config/list` returns exactly what `weflow --json config list` prints. Design and plans: issue #131.

The desktop app's HTTP API (`/api/v1/*`, SSE push) is a separate server started with `weflow serve --desktop`; see [HTTP-API.md](HTTP-API.md).

## Starting

```sh
weflow serve                       # command API on http://127.0.0.1:18108
weflow serve --port 8080 --host 127.0.0.1
weflow serve --desktop             # the desktop app's API instead (port `http_api_port`, default 5031)
weflow serve --ui-dir web/dist     # with the WebUI at http://127.0.0.1:18108/ui/
```

| Option | Meaning |
|---|---|
| `--host`, `--port` | Listen address; default `127.0.0.1:18108` (with `--desktop`: `http_api_host` / `http_api_port`, default `127.0.0.1:5031`) |
| `--api-token` | Access token (also `WEFLOW_HTTP_TOKEN`, or `http_api_token` in the config) |
| `--desktop` | Serve the desktop app's API instead of the command API |
| `--message-push` | SSE push of the desktop API; needs `--desktop` |
| `--insight`, `--image-auto-download` | Run these services next to either API |
| `--max-tasks` | Tasks (`POST` commands) that run at once; default 1 (see [Limits](#limits)) |
| `--max-reads` | Other commands that run at once; default the number of CPUs |
| `--max-media` | Images and voices the `/api/` media routes decode at once, more wait in line; default the number of CPUs (see [Media](#media)) |
| `--log-requests` | A line on stderr for each request: method, path, status and time (see [Where the time goes](#where-the-time-goes)) |
| `--ui-dir` | Serve the WebUI from this folder (a built WebUI, such as `web/dist`) for this run only (see [WebUI](#webui)) |
| `--no-browser` | Do not open the WebUI in the browser at the start |
| `--web-version` | Show this WebUI version for this run, downloaded first when needed (see [Versions of the WebUI](#versions-of-the-webui)) |
| `--api-only` | Serve the API alone, without the WebUI |

The startup message (an event on stderr; a JSON line with `--json`) shows the address. The default port lies below 49152, outside the range Windows hands out to programs and from which Hyper-V, WSL 2 and Docker reserve blocks of ports that no one can bind. Binding an address other than loopback prints a warning: anyone with the token can run any `weflow` command on this machine, with the server's file system and configuration.

## WebUI

`serve` also hosts the web interface (`web/`, issue #130) under `/ui/`. Which one:

1. none with `--api-only`;
2. the folder `--ui-dir` names, for this run only (a folder without `index.html` is refused at the start);
3. the version `--web-version` names, for this run only, downloaded first when needed;
4. else the version picked with `weflow web use`, or, until one is picked, the default version of this `weflow` (the one in `web/package.json` when it was built; the binary holds only that version number, not the page, which is downloaded like any other version). When that version is not here yet, the server starts at once (with the newest suitable version already here, or as the API alone), downloads it beside, and shows it once it is in place: the event `webui_ready`, or `webui_unavailable` when the download fails, in which case what was there stays. A version kept here that does not suit this `weflow` (kept by a newer one, say) is not shown: the server warns, shows the newest suitable version kept here instead, or runs as the API alone.

Without a WebUI the server runs as the API alone, and the startup message says so.

With a WebUI, the startup message gives its address with the token, `http://127.0.0.1:18108/ui/?token=…` (`uiUrl` with `--json`; the token percent-encoded, so one with `+`, `&`, `#` or `=` reaches the page as it is), and when stderr is a terminal the server opens it in the browser (`--no-browser` turns that off). The page keeps the token for its tab and removes it from the address bar; it is never written into the WebUI's files.

- The page's files need no token, unlike the API, but the `Host` check below still applies. Only `GET` and `HEAD` are answered.
- `/ui` goes to `/ui/` (`308`), and with a WebUI `/` goes to `/ui/` too (`302`), keeping the query.
- A path of the page's own (without an extension, such as `/ui/chat`) that is not a file gets `index.html`, so the page routes it; a missing file with an extension, or under `/ui/assets/`, answers `404`. Nothing outside `/ui/` falls back: an unknown API path still answers `404` in JSON.
- Only files inside the folder are served: `..`, encoded or not, and links that lead out of it answer `404`.
- The built assets (`/ui/assets/`, named after their content) are cached for good; `index.html` and the other files are checked again each time (`Cache-Control: no-cache`), so a new version shows at once. Every file is sent with `X-Content-Type-Options: nosniff`, and the page with `Referrer-Policy: no-referrer`.

### Versions of the WebUI

The WebUI is published apart from `weflow`, as OCI artifacts on GHCR (`ghcr.io/334456777/weflow-web:<version>`, by `.github/workflows/web-release.yml`), and `weflow` downloads them itself: no Docker, ORAS, Bun or Node.js is needed, and the CLI and the API work without any of it.

| Command | What it does |
|---|---|
| `web info` | The version in use, the one picked, the default one of this `weflow`, and the versions kept here |
| `web list [--remote]` | The versions kept here; with `--remote` the published ones, and whether each suits this `weflow` |
| `web install <version>` | Downloads a version, checks it and keeps it |
| `web use <version>` | Picks the version `serve` shows (downloaded first when needed) |
| `web update` | Downloads and picks the newest published version that suits this `weflow` (a release before a pre-release) |
| `web prune` | Removes the versions kept here that are neither picked nor the default one of this `weflow`; one a running `serve` shows is left (listed in `shown`) until that server stops |

- A version suits this `weflow` when it needs the API it serves (`apiVersion` of `/health`) and a `weflow` no newer than it (`minBackendVersion`). The manifest says both, so an unsuitable version is refused before it is downloaded, and its `weflow-web.json` says them again, checked after. A version kept already is checked again each time it is installed, picked or shown, as another `weflow` may have kept it; `web use` refuses an unsuitable one and the pick stays as it was.
- A download is checked against the size and the SHA-256 digest of its manifest, and unpacked under limits (at most 64 MB packed, 256 MB and 10,000 files unpacked; no path leading out of its folder, no links) into a folder of its own, then checked (`index.html`, the metadata, the version asked for) before one rename moves it into place. A failed, damaged or interrupted download leaves the versions already here as they were; two downloads of one version at once keep one copy.
- The versions are kept with WeFlow's other caches, in `<home>/cache/web/<version>/`; the one picked is named in `<home>/cache/web/current`. A running `serve` holds a shared lock on `<home>/cache/web/.lock-<version>` for the version it shows, which `web prune` checks before removing it; the lock ends with the server, however it ends. `cache clear --web` removes them all; `serve` downloads the one it needs again.
- The registry is reached as the OCI Distribution API describes (an anonymous token, then the manifest and the blob); `WEFLOW_WEB_REGISTRY` names another (a mirror, or a test's).

## Authentication

Every request except `GET /health` and `GET /health/events` needs the token, as `Authorization: Bearer <token>` or as the `access_token` query parameter. Without a configured token, `serve` makes one up for each run and prints it in the startup message (`generatedToken` with `--json`); set `http_api_token` or pass `--api-token` to keep a fixed one.

On a loopback address (the default) the server also checks the `Host` header of every request, `/health` included: only `localhost` and loopback addresses (`127.0.0.1`, `[::1]`, …), with any port, are answered; any other name gets `403` with the error code `invalid_host`. This keeps a web page from reaching the API through a name of its own that it makes resolve to `127.0.0.1` (DNS rebinding). Listening on another address with `--host` turns the check off, as the machine is then reached by other names.

## Requests

### Path and method

The path is the command path: `weflow chat date-counts` is `/chat/date-counts`, `weflow report annual years` is `/report/annual/years`. Paths that are not a command answer `404`; so does `/serve`.

The method says what kind of side effect the command has:

| Method | Commands |
|---|---|
| `GET` | Reading (it may fill caches that can be rebuilt): `config path/list/get`, `db *`, `chat sessions/messages/search/…`, `analytics *`, `group list/members/…`, `sns timeline/…`, `biz *`, `runtime *`, `cache list`, … |
| `POST` | Running a task or writing files: `export *`, `report * generate`, `key *`, `backup create/restore`, `chat voice-data/image-data/video-data/…`, `sns export/media/…`, `image decrypt`, `insight test/trigger/scan`, `config import`, `ffmpeg install`, … |
| `PUT` | Changing a setting: `config set`, `lang`, `ffmpeg set baseurl`, `insight mark-read`, `analytics excluded` and `chat group-hint` with `set` |
| `DELETE` | Removing: `config unset/clear`, `cache clear/clear-account`, `ffmpeg unset baseurl`, `insight clear` |

The full table is `METHODS` in `crates/weflow-cli/src/serve_api.rs`; a test keeps it in step with the command tree. A request with another method answers `405` with an `Allow` header. Three commands take their method from an argument: `analytics excluded` and `chat group-hint` are `GET` without `set` and `PUT` with it; `sns block-delete` is `GET` for `action=check`, `PUT` for `install` and `DELETE` for `uninstall`.

Commands that would write WeChat's databases have their route too (`PATCH /chat/update-message`, `DELETE /chat/delete-message`, `POST /chat/mark-read`, `PUT`/`DELETE /chat/anti-revoke/install|uninstall`, `sns block-delete`, `DELETE /sns/delete`); the database is read-only, so once an account is configured they answer `501` with the error code `unsupported`.

### Parameters

Parameter names are the long option names; positional arguments use their name in kebab case (`session-id`). `_` may be written for `-`.

- `GET`: query parameters. Other methods: query parameters and/or a JSON object body; a key in the body replaces the same key in the query.
- Several values: repeat the key (`?session=a&session=b`) or use a JSON array.
- Switches: `true` / `false` (`?force=true`; an empty value means `true`).
- A JSON value given for a single-valued argument is passed as JSON text (for the arguments that take a JSON payload).

The server turns the parameters into a command line and the command checks it as on a terminal: a missing or malformed argument answers `400` with the command's own `invalid_arguments` error. A name the command does not have answers `400` before anything runs.

```sh
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:18108/chat/latest?session-id=wxid_xxx&limit=20"
curl -X PUT -H "Authorization: Bearer $TOKEN" -d '{"key":"no_progress","value":"true"}' http://127.0.0.1:18108/config/set
```

`Accept-Language` picks the language of messages (`zh…` is Chinese, anything else English); without it the command follows its usual rules (`WEFLOW_LANG`, `weflow lang`, the system).

## Answers

Answers are JSON (`application/json; charset=utf-8`), apart from [`?return=file`](#files) and the [event stream of a job](#jobs), with the headers `X-WeFlow-Exit-Code` (the command's exit code) and, on failure, `X-WeFlow-Error-Code`.

| Request | Runs the command | Body |
|---|---|---|
| no `std` | yes, with `--json` | The `--json` document as printed: `{"success": true, "data": …}` or `{"success": false, "error": {"code", "message", "details"}}` |
| `?std=out` | yes, without `--json` | `{"exitCode": 0, "stdout": "<the text a terminal shows>"}` |
| `?std=err` | yes, without `--json` | `{"exitCode": 0, "stderr": "<warnings, progress>"}` |
| `?std=in` | no | `{"argv": ["weflow", "--json", …], "stdin": null}`: the equivalent command line |
| `?return=file` | yes, with `--json` | The result and the file the command wrote (see [Files](#files)); not with `std` |

### Status codes

By the error code first, then by the exit code. The CLI's exit codes do not change.

| Error code | Exit code | HTTP |
|---|---|---|
| (success) | `0` | `200` (`202` for a request started as a [job](#jobs)) |
| `invalid_arguments` | `2` | `400` |
| `config_error` | `3` | `409` (no account, key or database path configured yet) |
| `unsupported` | `4` | `501` (refused by the read-only database, or not ported) |
| `native_error` | `4` | `500` |
| `runtime_error` | `1` | `500`; `404` when `details.failureKind` is `not_found` |
| `user_interrupt` | `130` | `499` |

`std=out` and `std=err` have no error code and go by the exit code alone. Errors of the server itself use the same shape: `unauthorized` (`401`), `invalid_host` (`403`, see [Authentication](#authentication)), `not_found` (`404`), `method_not_allowed` (`405`), `invalid_arguments` (`400`), `busy` (`429`, see [Limits](#limits)).

### Size

Answers are compressed with gzip or br when the request's `Accept-Encoding` takes them, as browsers always do. Images, voices, videos, the files of `?return=file`, a part of a file (`206`) and event streams are sent as they are. `fields=<key>,<key>` (or the key repeated) keeps only those keys of each row in a successful `--json` document: of each object in `data` when it is a list, or of each object in the lists of `data` when it is an object; a failure comes back whole. It is the server's, not the command's, so it works on both routes, but not with `std`, `return=file`, `Prefer: respond-async` or a media route (`400`).

| Answer (large account, `/api/`) | As printed | br | `fields` | `fields` and br |
|---|---:|---:|---:|---:|
| `chat messages`, 50 messages | 126 KB, 3.5 ms | 18 KB, 5.1 ms | 87 KB | 15 KB, 4.9 ms |
| `chat contacts`, all | 538 KB, 6.1 ms | 119 KB, 10.7 ms | 154 KB | 71 KB, 7.4 ms |

(The `fields` of a chat window: `local_id,server_id,local_type,sort_seq,create_time,sender_username,is_send,message_content,_cursor`; of a contact list: `username,alias,remark,nick_name,small_head_url,local_type`.) gzip is a little bigger than br and costs about as much. With both, all the contacts come in one answer of about 70 KB, so `chat contacts` is not paged: a page searches and sorts them itself.

## Files

Commands that write one file can send it back in the answer: add `?return=file` (the default is `return=json`).

| Command | File |
|---|---|
| `chat voice-data` | the WAV |
| `chat image-data` | the image |
| `chat video-data` | the video (with `thumb`, its thumbnail or cover) |
| `export sessions`, `export contacts`, `export footprint`, `export messages` | the export (with `export messages --media`, only the messages file, the media stay on disk; with `--zip`, the zip of every format and the `media` folder) |
| `group export-member-messages`, `group export-members` | the `.csv` / `.xlsx` |
| `sns media`, `sns download-image` | the image or video |
| `backup create` | the backup archive |

Other commands answer `400`: their output is a folder or many files (`export media`, `sns export`, `chat emoji`, …) or no file at all.

- `out` may be left out, also where the command requires it: the server then writes into a folder of its own, `serve-files/<request id>/` in the cache folder (`<home>/cache` or `cache_path`), and names the file itself (the extension of an image or video comes from its content). Given an `out`, the file is written there and sent as well.
- On success the answer is `200` with `Content-Type: multipart/form-data` and two parts: `result`, `application/json`, `{"success": true, "data": {"path": "<the file on the server>"}}`; then `file`, the content, with a `Content-Type` from its extension and the file name in `filename` (UTF-8) and `filename*` (RFC 5987). The file is read from disk as it is sent. In a browser, `await (await fetch(url, init)).formData()` gives both parts.
- On failure there is no file: the usual JSON error and status code.
- `serve-files/` is emptied when `serve` starts and is part of `cache list` / `cache clear --api`.
- With `Prefer: respond-async` the command runs as a [job](#jobs) instead and keeps its file: once the job has succeeded, its `file` names it (`{"name", "size"}`) and `GET /jobs/<id>/file` sends the file alone, with `Content-Disposition: attachment`, so that a link saves it. A folder the server chose is removed when the job fails, is cancelled or is forgotten.

```sh
curl -X POST -H "Authorization: Bearer $TOKEN" -o reply.bin "http://127.0.0.1:18108/export/sessions?return=file"
```

## Jobs

A long command (an export, `key db`, `backup create`) need not hold a request open. Send `Prefer: respond-async` and the command runs in the background as a job: the answer is `202 Accepted`, `Location: /jobs/<id>`, and `{"success": true, "data": <job>}`.

| Request | Answer |
|---|---|
| `GET /jobs` | `{"success": true, "data": [<job>, …]}`, oldest first, without results |
| `GET /jobs/<id>` | The job with its `result` |
| `GET /jobs/<id>/events` | Server-sent events until the job ends (below) |
| `DELETE /jobs/<id>` | A queued or running job is cancelled (its command is killed; `key db` is asked to end and killed only after 10 s, see below) and the answer is the cancelled job; a finished job is forgotten |
| `GET /jobs/<id>/file` | The file of a job started with `?return=file`, once it has succeeded (see [Files](#files)); `404` otherwise |

A job is `{"id", "status", "command", "createdAt", "startedAt", "finishedAt", "exitCode", "httpStatus", "progress", "file", "result"}`: `status` is `queued`, `running`, `succeeded`, `failed` or `cancelled`; `command` the command line it runs (always `--json --progress`); times in Unix milliseconds; `httpStatus` the status a direct request would have answered; `progress` the last progress event; `file` the file of a `?return=file` job that succeeded (`{"name", "size"}`), else `null`; `result` the `--json` document, as a direct request would have answered. A cancelled job has `httpStatus` `499` and the error code `user_interrupt`.

The event stream sends, each with an `id`:

- `status`: `{"type": "status", "status": "running"}` when the job leaves the queue;
- the command's `--progress` events, named by their `type` (`progress`: `{"type": "progress", "stage", "message", "current", "total", "percent"}`), and its other events on stderr;
- `stderr`: `{"type": "stderr", "text"}` for other lines on stderr;
- `result`: the finished job with its `result`, after which the stream ends.

A client that subscribes late or reconnects gets the events kept so far (up to 1000 per job); with `Last-Event-ID`, only those after it. Jobs live in the server's memory only: they are gone when it stops, and only the 100 most recent finished jobs are kept.

`?std=out` and `?std=err` are answered right away and cannot be jobs (`400`); `?std=in` never runs anything.

```sh
curl -i -X POST -H "Authorization: Bearer $TOKEN" -H "Prefer: respond-async" \
  -d '{"session-id":"wxid_xxx","out":"chat.html","format":"html"}' http://127.0.0.1:18108/export/messages
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:18108/jobs/<id>/events
```

## Limits

Two limits keep the machine responsive: tasks (`POST` commands: exports, keys, backups, reports, …) run one at a time (`--max-tasks`), other commands up to one per CPU at once (`--max-reads`). A direct request over the limit through a route without `/api/`, which starts a process, is not kept waiting: it answers `429` with the error code `busy` and `Retry-After` (5 seconds for tasks, 1 otherwise). A job over the limit waits in the queue (`queued`) and starts when a place is free.

The [`/api/` reads](#in-process-reads) share `--max-reads` with them, but a request over it waits in line for a place: a page asks for several at once, and each takes a few milliseconds. Only when 256 are already waiting, or one has waited 10 seconds, does it answer `429` (`busy`, `Retry-After: 1`). The `/api/` media routes have a limit and a line of their own (`--max-media`, see [Media](#media)), so a screen of images does not hold up the reads. A read whose client goes away before the answer runs to its end, as it cannot be stopped halfway, and keeps its place until then: the limits count every read still running.

| The same read asked for at once (first page of messages, large account, 16 CPUs) | `/api/<path>` | `/<path>` |
|---|---:|---:|
| 1 | 3.7 ms | 31 ms |
| 6 | 12 ms | 48 ms |
| 16 | 24 ms | 98 ms |
| 32 | 46 ms | 105 ms, half of them `429` |
| 64 | 84 ms, about 760 answered a second | – |

A browser sends at most 6 requests at once to one server (HTTP/1.1), and an open event stream such as `/api/push/messages` holds one of them, so a page alone stays below the default limit; more come from several tabs or from scripts. Allowing a database more connections for these reads did not make them faster: the time goes into each answer (about 120 KB of JSON for a page), not into waiting for the database.

## OpenAPI description

`GET /openapi.json` (with the token) answers an OpenAPI 3.1 description of the API. It is built from the command tree and the method table of the running `weflow`, so it always matches it; summaries and parameter descriptions are the commands' help, in the language of `Accept-Language` (else the server's).

- Each command is one operation per method, with `x-weflow-command` naming the command. `operationId` is the command path with `_` (`chat_sessions`); a command whose method follows an argument has one operation per method (`analytics_excluded_get`, `analytics_excluded_put`).
- An operation that would write WeChat's databases, which the command always refuses with `501` (`chat update-message`, `chat delete-message`, `chat anti-revoke install/uninstall`, `sns block-delete` with `install` or `uninstall`, `sns delete`), has `"x-weflow-refused": true`, so a client need not offer it.
- `GET` and `DELETE` list the parameters as query parameters, the other methods as a JSON body; both ways work for every method (see [Parameters](#parameters)). Types come from the CLI: numbers, switches, lists and choices.
- The answers are described by their envelope (`{"success", "data"}` or `{"success", "error"}`), jobs, and `?return=file`. For the reads the WebUI pages through, `data` is described too (#399): `chat sessions`, `chat messages`, `chat search`, `chat latest`, `chat message`, `chat contacts` and `chat resources`, with their types under `components/schemas` (`Session`, `ParsedMessage`, `Contact`, `ResourcePage`…), as are `Job` and `Error`. The types are the Rust structs the commands answer with (`weflow_core::contract`), so the description cannot drift from the answers. Their descriptions are in English. Times are Unix seconds unless a field says otherwise; WeChat's ids that may pass 2^53 are strings; an optional field is left out when there is nothing to say, a nullable one is `null`. The rows of WeChat's own tables (without `parsed`) are a `Row`, an object whose columns WeChat decides. The other commands' `data` is not described yet.
- `web/contract/openapi.json` keeps those types alone, without help texts or anything of the build, and the WebUI's types are generated from it (see web/README.md). A test fails when it is not what the code answers with; after changing a type, rewrite it with `WEFLOW_UPDATE_CONTRACT=1 cargo test -p weflow-cli --bin weflow contract_snapshot`.
- `/health`, `/health/events`, `/openapi.json`, `/jobs` and `/api/push/messages` are described under the tag `server`; the [in-process reads](#in-process-reads) have their own operations, `api_chat_sessions` and so on.

```sh
curl -H "Authorization: Bearer $TOKEN" -H "Accept-Language: en" -o openapi.json http://127.0.0.1:18108/openapi.json
```

Two versions tell clients what they talk to: `version` (in `/health`, and `info.version`) is the version of `weflow`; `apiVersion` (in `/health`, and `info.x-weflow-api-version`) is the version of the API's rules — routes, methods, parameters, answers, jobs — now `2` (it was `1` until a parsed message's `serverId` became a string, #399). It is raised only by a change that breaks existing clients, not by new commands or options. `GET /health` needs no token, so a client such as the WebUI can check both before it asks for one:

```json
{"status": "ok", "apiVersion": "2", "version": "5.0.0", "data": "5f1c0a9e3b7d2c41",
 "limits": {"maxTasks": 1, "maxReads": 4, "maxMedia": 4}, "insight": false}
```

`data` is an opaque value that changes with each start of the server and whenever its configuration changes, such as when another account is set up. Records a client read before it changed may be another account's: the WebUI forgets what it kept (messages, group members, the days with messages) and reads again. It is a keyed hash, drawn anew each run, and tells nothing of the configuration.

`limits` is what may run at once (`--max-tasks`, `--max-reads`, `--max-media`, see [Limits](#limits)) and `insight` whether the insight engine runs beside (`--insight`); the WebUI shows them in its settings. They say how the server runs, nothing of the account.

### Staying connected

`GET /health/events` is the same as a stream of server-sent events, for a page that keeps one connection open instead of asking `/health` again and again. Like `/health` it needs no token (an `EventSource` cannot send a header) and is behind the `Host` check; it starts no work.

- It starts with `event: hello`, whose data is what `/health` answers, then sends `event: heartbeat` every 15 seconds with `{"data": "…"}`. A heartbeat is an event, not a comment line, so a page sees it: no event for 37.5 seconds (two and a half heartbeats) means a half-open connection (the network gone, a NAT that forgot it), which closing the stream alone would not tell.
- `/api/push/messages` starts with the same `hello` and sends the same heartbeat, so a page listening to it needs no second stream.
- When the server stops on Ctrl+C, both streams end with `event: bye` (`{"reason": "stopped"}`; the server waits at most half a second for them to go out), so a client can tell a server that stopped from one it lost.

The WebUI holds one such stream and reports the connection by it: the moment the stream breaks (the server stopped), the indicator says 「未连接」; it connects again after 1, 2, 4 … at most 30 seconds (give or take a fifth), at once when the network or the tab comes back and on 「重新检测」. Idle, it sends no request. On this machine (`127.0.0.1`) losing the network does not cut the page from the server, so there it only checks again. A server without `/health/events` (5.0.0-rc.5 and older) is not supported by the WebUI.

With 「实时消息」 on (the default, in the settings), the WebUI holds `/api/push/messages` instead, the token in its address (`access_token`), and the indicator says 「实时」. A message event has the conversation read again: the list, and the open conversation from its last message on (`after`); a recall, or a break, reads its newest page again too. After a break it reconnects with the id of the last event (`last-event-id`); the ids of a run start at the time it started, so a page coming back to a restarted server misses none.

## How it runs

A command route (a path without `/api/`, run at once or as a job) runs `weflow` again as a child process with the same configuration, so output and exit code are those of the terminal and a failing command cannot take the server down. A client that closes the connection before the answer ends its command; so does cancelling a job. `key db` is not killed but let to end by itself (#287): its stdin stays open while it runs, and once it closes (the job cancelled, the connection or the server gone) the command ends as on Ctrl+C, taking its hardware breakpoints out of WeChat and detaching first, with `user_interrupt`. A cancelled `key db` job is killed only when it has not ended within 10 s.

The price is paid on every such request: starting the process and reading the configuration take about 20 ms, and each request opens the databases again with cold caches. On a large account (Windows, release build) the conversation list takes about 37 ms this way and a page of messages about 31 ms. For the reads a chat client makes all the time, use [`/api/`](#in-process-reads).

Opening a database first derives its key from the one in the configuration (about 0.1 s). The server hands the keys it has derived (those of a chat page's databases, see the warm-up below) to each command it runs, through the command's stdin, so the command does not derive them again; without this, the two reads above took about 500 ms and 175 ms. A key that does not open its file is derived again.

### Where the time goes

Every answer has a `Server-Timing` header, which a browser's developer tools show next to the request: `cmd` is the command, with `desc="in process"` or `desc="child process"`; `wait` is the time an `/api/` request waited in line for a place (see [Limits](#limits)); `total` is the whole request in the server, from its arrival to its answer's headers (for an event stream, until it opens). A page served from `localhost` also gets `Timing-Allow-Origin`, so its scripts can read the stages in `performance.getEntriesByType("resource")`.

```text
Server-Timing: wait;desc="in line for a place";dur=0.0, cmd;desc="in process";dur=3.1, total;dur=3.4
```

`--log-requests` prints a line on stderr for each request: the time, method, path, status and milliseconds, or with `--json` an event `{"type": "request", "method", "path", "status", "ms"}`. The query is left out, as it may hold conversation ids and the token; nothing of the answer is printed. It is off by default.

## In-process reads

`GET /api/<command path>` runs a read inside the server, on databases it keeps open between requests: `chat sessions`, `chat messages`, `chat latest`, `chat message`, `chat contact`, `chat contacts`, `chat counts`, `chat search`, `chat dates`, `chat date-counts`, `chat detail` (with `fast=true`), `chat resources`, `chat image-info`, `chat statuses`, `chat tab-counts`, `group list`, `group members`, `group ranking`, `group hours`, `group media`, `group member`, `group member-messages`, `sns timeline` (without `with-media`), `sns users`, `sns stats`, `sns post-counts`, `analytics overall`, `analytics rankings`, `analytics time`, `analytics excluded` (without `set`), `analytics exclude-candidates`, `biz accounts`, `biz messages`, `biz pay-records`, `insight footprint`, `insight get`, `report annual years`, `cache list`, `config get`, `config profiles`, `runtime info`, `db detect`, `db test`, `db wxid`. Parameters, answer, status code and headers are those of the same path without `/api/` (the request is read into the same typed read the command runs, without a command line; a test compares them byte for byte, another that the parameters are read as the command reads them; a request the command would refuse runs as the route without `/api/` does, so its error is the terminal's), only faster:

| Read (large account, Windows) | `/<path>` | `/api/<path>` |
|---|---|---|
| `chat sessions` | 37 ms | 9 ms |
| `chat messages`, one page of 50 | 31 ms | 5 ms |
| `chat latest`, 20 messages | 29 ms | 4 ms |
| `chat contact` | 24 ms | 2 ms |
| `group members` | 33 ms | 2 ms |
| `chat date-counts`, 213,000 messages | 584 ms | 63 ms |
| `chat detail --fast` | 55 ms | 2 ms |
| `chat resources`, a page of 300 | 109 ms | 42 ms |
| `sns timeline`, 20 posts | 235 ms | 6 ms |
| `chat statuses`, 60 conversations | 24 ms | 2 ms |
| `group list` | 28 ms | 4 ms |
| `analytics overall`, the aggregate kept | 25 ms | 2 ms |
| `report annual years` | 97 ms | 34 ms |
| 16 pages of messages at once (in all) | 156 ms | 140 ms |

(Medians measured by `serve_bench`, as the client sees them; see [native-cli.md](native-cli.md).) A search in a conversation first reads its newest messages and stops once a page is found, so a common word takes about 5 ms; after a few thousand messages (or 15 ms) it turns to WeChat's own index (see [cli-unsupported.md](cli-unsupported.md)), so a word found nowhere takes about 60 ms in a conversation of 210,000 messages instead of reading them all. Pages deep into a long conversation are slow with `offset` (about 1.4 s in the middle of 210,000 messages) but not from a cursor: pass the `_cursor` of the last message shown as `before` (`after` for newer ones), and the same page takes about 3 ms.

- `chat messages` with `parsed=true` (`--parsed`) answers each message as the desktop app parses it instead of the database row: `localType`, `parsedContent`, the quote (`quotedContent`, `quotedSender`), the keys of its media (`imageMd5`, `videoMd5`, `voiceDurationSeconds`, `emojiCdnUrl`), links, files, cards, locations, Channels posts, chat records (`appMsgKind` names the kind of an app message), with its `_cursor` as before. `serverId` is a decimal string, as WeChat's ids pass 2^53, past which a JavaScript number loses digits. A chat window then reads no XML of its own. On a large account the server takes about 1 ms more for a page of 50 (median 1.9 → 2.8 ms), and the answer is smaller than the rows; `fields` leaves out `rawContent`, the XML it was parsed from.
- `chat messages` with `since=<Unix seconds>` (`--since`) answers the first messages from where that time falls in the chat's order, still newest first, each with its `_cursor`: a chat window jumps to a day with the start of that day, then pages on with `before` and `after`. The order is WeChat's own (`sort_seq`, the send time in milliseconds plus a little), the one the chat window shows; a message ordered a second or two off its send time falls on the side its order puts it, so a message sent just before that time may open the page and one sent at it may sit right above it, one `before` page away. In exchange the pages follow one another without a gap or a repeat. It reads from the same index as a cursor, so it is as fast (a page of 50 parsed in about 22 ms in a debug build on a large account).
- `chat message` with `parsed=true` (`--parsed`) returns the single message in the same shape as `chat messages --parsed`, including its `_cursor`. A media list can look up a message with `local-id` and `create-time`, then page around it with `before` and `after`. Without `parsed`, the existing lookup response is unchanged.
- `chat contacts` with `parsed=true` (`--parsed`) answers the contact list as the desktop app shows it instead of the database rows: the friends, groups, official accounts and former friends only, each with its `type` (`friend`, `group`, `official`, `former_friend`), `displayName` (remark, else nickname, else WeChat ID), `remark`, `nickname`, `alias`, `labels`, `region`, `detailDescription` (the signature) and `avatarUrl`, the most recent conversation first. `export contacts` takes `parsed=true` too, to write that list instead of the rows, and `kind` (repeatable: `friend`, `group`, `official`, `former_friend`) to keep only those kinds.
- `chat search` gives each hit the `_cursor` of its message (and `_session_id`, as before): `before` and `after` with it read the messages around a hit. With `parsed=true` (`--parsed`) the hits are parsed as `chat messages` parses them.
- `?std`, `?return=file` and `Prefer: respond-async` are not available (`400`); other commands answer `404`.
- The answer is in the server's language: `Accept-Language` does not apply, as the language is set once per process.
- A changed config file (`PUT /config/set`, another profile, another file chosen with `config_path`) is read again on the next request. A change is told by the file's time and size, and for a few seconds after a write by its content too: two writes within one tick of a coarse clock (2 s on FAT) leave the same time, and two profiles whose names are as long the same size.
- Right after the start, and again after the config changed, the server opens the databases of a chat page in the background, all at once: conversations, contacts, avatars, image links, the search index, messages and voices. Opening a database first derives its key (about 0.1 s), so the first reads would otherwise wait for every database they touch. On a large account the warm-up is over within half a second; the conversation list then first takes about 12 ms instead of about 440 ms, and about 240 ms when asked for right at the start.
- `group members` keeps a member panel for a while, as the desktop app does: within that time it answers `"fromCache": true`; pass `refresh=true` to build it again.
- `chat detail` without `fast=true`, `sns timeline` with `with-media=true` and `analytics excluded` with `set` run as the route without `/api/` does: the rest of the details (the tables, the first and last times) is slower to read, the media of the timeline are downloaded, and `set` saves the list.
- `analytics overall`, `rankings` and `time` answer the aggregate kept from last time, as the command does (`force=true` reads every chat again). The server also keeps it in memory for 5 minutes, so an aggregate that a command route computed again within that time shows after it.
- The other commands stay command routes only: long tasks and exports, which a cancel stops by ending their process; the key commands, which attach to WeChat; commands that ask on the terminal or set their own process's output; and commands that write.
- `sns stats` with `fast=true` and `sns post-counts` with `prefer-cache=true` keep their numbers for 5 minutes, as the desktop app does: new posts show in them after that, or at once without those switches.
- These reads count towards the `--max-reads` limit, and wait in line for a place instead of answering `429` (see [Limits](#limits)).

### Media

`GET /api/chat/image-data`, `GET /api/chat/voice-data` and `GET /api/chat/video-data` answer the file itself, for a page to show in `<img>`, `<audio>` or `<video>`: the decrypted image (`Content-Type` from its bytes, such as `image/jpeg`), the voice decoded to `audio/wav`, or the video as WeChat keeps it (`video/mp4`). They take the parameters of the commands (`session-id`, `msg-id`, `create-time`; for voices also `server-id`, `sender`) as a query, but not `out`, which answers `400`. Only `GET` is allowed (`405` otherwise); the routes without `/api/` are unchanged (`POST`, a task, writing a file).

```html
<img src="http://127.0.0.1:18108/api/chat/image-data?session-id=wxid_xxx&msg-id=123&create-time=1700000000&access_token=TOKEN">
```

- `thumb=true` (`--thumb` of the command) answers the thumbnail WeChat keeps of the image, a few kilobytes, for a list; the image itself when there is none. On a large account a thumbnail takes about 2 ms and is about a fifth of the size of the image.
- WeChat keeps up to three files of an image: the thumbnail a chat shows (`<md5>_t.dat`), the image shown when it is opened (`<md5>.dat`), and the original it downloads when View original is clicked (`<md5>_h.dat`). `variant` picks one: `best` (the default: the best of them on this computer), `thumb` (as `thumb=true`), `normal` (the image opened; the best there is when it is missing), `hd` (the original; `404`, `failureKind: not_found`, when WeChat has not downloaded it). `GET /api/chat/image-info` (`chat image-info`) says which of them are on this computer (`thumb`, `normal`, `hd`) and their sizes in bytes as the message gives them (`normalBytes`, `hdBytes`; the original's even before it is downloaded), for a page to tell whether the original can be shown.
- For a video, `thumb=true` answers the thumbnail WeChat keeps of it, or its cover when there is none. A video is not decoded: it is read from WeChat's video folder as it is sent, only the part asked for, so a long one takes no memory and a player can skip ahead at once: on a large account the first megabyte of a video takes about 4.5 ms, a megabyte from its middle about 3.5 ms. A video WeChat has not downloaded answers `404` (`failureKind: not_found`): open it in WeChat first.
- The answer has an `ETag` (for a video, from the size and time of its file) and `Cache-Control: private, max-age=3600`. Asking again with `If-None-Match` answers `304` without the file when it has not changed.
- A part of the file can be asked for with `Range: bytes=…`, as an `<audio>` player does: `206` with `Content-Range`, or `416` past the end. Several ranges, or an `If-Range` that names another version of the file, get the whole file.
- A failure is the command's `--json` document with its status code, as for the other in-process reads.
- They have a limit of their own, `--max-media` (one per CPU by default; a video holds its place while its file is found, not while it is sent), and count towards neither `--max-reads` nor `--max-tasks`, so a screen of images does not hold up the messages beside it.
- A request over the limit waits in line instead of answering `429` straight away: a screen of thumbnails asks for its images at once, and each takes a few milliseconds. Only when 256 requests are already waiting, or a request has waited 10 seconds, does it answer `429` (`busy`, `Retry-After: 1`).
- On a large account an image takes about 4 ms instead of about 190 ms through `POST /chat/image-data`, and 20 images asked for at once all answer within 10 ms.

`GET /api/sns/media` does the same for the images and videos of Moments posts, with the parameters of `sns media`: the `url` and `key` of a medium of `sns timeline` (`media[].thumb` or `media[].url`, and `media[].key`). WeChat keeps them on its servers, encrypted, so the first request fetches the medium and decrypts it; it is kept in the cache (`<cache>/sns_cache`, cleared by `cache clear --sns`) and later requests read it from there. A video is sent from its file, so `Range` works as for the videos of messages.

- Only `https` addresses of WeChat's media servers are taken (`*.qpic.cn`, `*.wechat.com`, `*.video.qq.com`, `*.tc.qq.com`); anything else answers `400` (`invalid_arguments`), so the route cannot be made to fetch other addresses. `POST /sns/media` is unchanged.
- A request waits for WeChat's servers the first time, while holding its place among the media requests.

### Message push

`GET /api/push/messages` sends new and recalled messages as server-sent events, so a page need not poll `chat latest`:

```js
const events = new EventSource("http://127.0.0.1:18108/api/push/messages?access_token=TOKEN");
events.addEventListener("message.new", (e) => console.log(JSON.parse(e.data)));
```

- First come a `hello` event (what `/health` answers) and a `ready` event, then each new message as `message.new` and each recall as `message.revoke`, with the fields of the desktop app's push ([HTTP-API.md](HTTP-API.md#2-active-push)): `sessionId`, `sessionType`, `rawid`, `sourceName`, `groupName`, `content`, `timestamp`, … Every message event has an `id`; a `heartbeat` event every 15 seconds keeps a quiet stream open and shows it alive (see [Staying connected](#staying-connected)).
- The server keeps the events of the last 10 minutes (at most 1000). A client that reconnects with `Last-Event-ID` (EventSource does it by itself), or opens a new stream with `?last-event-id=`, gets the ones after that id; without either it gets all kept events.
- Only while someone listens, the server checks once a second whether the account's session and message databases changed, and then looks for new messages on the hub of the in-process reads (at least every 30 seconds anyway). A new message reaches the page within a second or two; listening costs the server about 5 ms of CPU a second on a large account. The first listener starts from what the account holds at that moment, and so does a listener after the config changed (another account).
- Only `GET`; the stream is not counted towards `--max-reads`. It is not the desktop app's `--message-push`, which belongs to `--desktop`.
