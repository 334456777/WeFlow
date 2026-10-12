# weflow serve: the command API

**English** | [简体中文](zh-CN/serve-api.md)

`weflow serve` makes every CLI command available over HTTP. The path is the command path, the parameters are the command's options, and the answer is what the terminal gets: `GET /config/list` returns exactly what `weflow --json config list` prints. Design and plans: issue #131.

The desktop app's HTTP API (`/api/v1/*`, SSE push) is a separate server started with `weflow serve --desktop`; see [HTTP-API.md](HTTP-API.md).

## Starting

```sh
weflow serve                       # command API on http://127.0.0.1:57108
weflow serve --port 8080 --host 127.0.0.1
weflow serve --desktop             # the desktop app's API instead (port `http_api_port`, default 5031)
```

| Option | Meaning |
|---|---|
| `--host`, `--port` | Listen address; default `127.0.0.1:57108` (with `--desktop`: `http_api_host` / `http_api_port`, default `127.0.0.1:5031`) |
| `--api-token` | Access token (also `WEFLOW_HTTP_TOKEN`, or `http_api_token` in the config) |
| `--desktop` | Serve the desktop app's API instead of the command API (`--http` is an old name for it) |
| `--message-push` | SSE push of the desktop API; needs `--desktop` |
| `--insight`, `--image-auto-download` | Run these services next to either API |

The startup message (an event on stderr; a JSON line with `--json`) shows the address. Binding an address other than loopback prints a warning: anyone with the token can run any `weflow` command on this machine, with the server's file system and configuration.

## Authentication

Every request except `GET /health` needs the token, as `Authorization: Bearer <token>` or as the `access_token` query parameter. Without a configured token, `serve` makes one up for each run and prints it in the startup message (`generatedToken` with `--json`); set `http_api_token` or pass `--api-token` to keep a fixed one.

## Requests

### Path and method

The path is the command path: `weflow chat date-counts` is `/chat/date-counts`, `weflow report annual years` is `/report/annual/years`. Paths that are not a command answer `404`; so does `/serve`.

The method says what kind of side effect the command has:

| Method | Commands |
|---|---|
| `GET` | Reading (it may fill caches that can be rebuilt): `config path/list/get`, `db *`, `chat sessions/messages/search/…`, `analytics *`, `group list/members/…`, `sns timeline/…`, `biz *`, `runtime *`, `cache list`, … |
| `POST` | Running a task or writing files: `export *`, `report * generate`, `key *`, `backup create/restore`, `chat voice-data/image-data/…`, `sns export/media/…`, `image decrypt`, `insight test/trigger/scan`, `config import`, `ffmpeg install`, … |
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
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:57108/chat/latest?session-id=wxid_xxx&limit=20"
curl -X PUT -H "Authorization: Bearer $TOKEN" -d '{"key":"no_progress","value":"true"}' http://127.0.0.1:57108/config/set
```

`Accept-Language` picks the language of messages (`zh…` is Chinese, anything else English); without it the command follows its usual rules (`WEFLOW_LANG`, `weflow lang`, the system).

## Answers

Every answer is JSON (`application/json; charset=utf-8`), with the headers `X-WeFlow-Exit-Code` (the command's exit code) and, on failure, `X-WeFlow-Error-Code`.

| Request | Runs the command | Body |
|---|---|---|
| no `std` | yes, with `--json` | The `--json` document as printed: `{"success": true, "data": …}` or `{"success": false, "error": {"code", "message", "details"}}` |
| `?std=out` | yes, without `--json` | `{"exitCode": 0, "stdout": "<the text a terminal shows>"}` |
| `?std=err` | yes, without `--json` | `{"exitCode": 0, "stderr": "<warnings, progress>"}` |
| `?std=in` | no | `{"argv": ["weflow", "--json", …], "stdin": null}`: the equivalent command line |

### Status codes

By the error code first, then by the exit code. The CLI's exit codes do not change.

| Error code | Exit code | HTTP |
|---|---|---|
| (success) | `0` | `200` |
| `invalid_arguments` | `2` | `400` |
| `config_error` | `3` | `409` (no account, key or database path configured yet) |
| `unsupported` | `4` | `501` (refused by the read-only database, or not ported) |
| `native_error` | `4` | `500` |
| `runtime_error` | `1` | `500`; `404` when `details.failureKind` is `not_found` |
| `user_interrupt` | `130` | `499` |

`std=out` and `std=err` have no error code and go by the exit code alone. Errors of the server itself use the same shape: `unauthorized` (`401`), `not_found` (`404`), `method_not_allowed` (`405`), `invalid_arguments` (`400`).

## How it runs

Each request runs `weflow` again as a child process with the same configuration, so output and exit code are those of the terminal and a failing command cannot take the server down. Starting the process, reading the configuration and opening the database cost some tens of milliseconds per request.

Not here yet (issue #131): asynchronous jobs with progress and cancellation, limits on parallel commands, returning a produced file in the answer, an OpenAPI description, `Host` header checks, and the WebUI.
