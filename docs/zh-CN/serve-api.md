# weflow serve：命令 API

[English](../serve-api.md) | **简体中文**

`weflow serve` 把每条 CLI 命令都提供为 HTTP 接口。路径就是命令路径，参数就是命令的选项，返回的就是终端看到的内容：`GET /config/list` 返回的正是 `weflow --json config list` 的输出。设计与计划见 issue #131。

桌面端的 HTTP API（`/api/v1/*`、SSE 推送）是另一个服务，用 `weflow serve --desktop` 启动，见 [HTTP-API.md](HTTP-API.md)。

## 启动

```sh
weflow serve                       # 命令 API，地址 http://127.0.0.1:57108
weflow serve --port 8080 --host 127.0.0.1
weflow serve --desktop             # 改为启动桌面端 API（端口 `http_api_port`，默认 5031）
```

| 选项 | 含义 |
|---|---|
| `--host`、`--port` | 监听地址；默认 `127.0.0.1:57108`（加 `--desktop` 时：`http_api_host` / `http_api_port`，默认 `127.0.0.1:5031`） |
| `--api-token` | 访问令牌（也可用 `WEFLOW_HTTP_TOKEN`，或配置中的 `http_api_token`） |
| `--desktop` | 启动桌面端 API，而不是命令 API |
| `--message-push` | 桌面端 API 的 SSE 推送；需要 `--desktop` |
| `--insight`、`--image-auto-download` | 与任一 API 一起运行这些服务 |
| `--max-tasks` | 同时运行的任务（`POST` 命令）数；默认 1（见[并发限制](#并发限制)） |
| `--max-reads` | 同时运行的其他命令数；默认为 CPU 核数 |

启动提示（stderr 上的事件；加 `--json` 时为一行 JSON）会显示地址。绑定回环以外的地址时会给出警告：持有令牌的人可以在这台机器上运行任何 `weflow` 命令，使用服务端的文件系统和配置。

## 鉴权

除 `GET /health` 外，所有请求都需要令牌，放在 `Authorization: Bearer <令牌>` 头里，或作为 `access_token` 查询参数。没有配置令牌时，`serve` 每次启动生成一个，显示在启动提示里（加 `--json` 时为 `generatedToken`）；设置 `http_api_token` 或传入 `--api-token` 可以固定令牌。

## 请求

### 路径与方法

路径就是命令路径：`weflow chat date-counts` 是 `/chat/date-counts`，`weflow report annual years` 是 `/report/annual/years`。不是命令的路径返回 `404`，`/serve` 也一样。

方法表示命令有哪类副作用：

| 方法 | 命令 |
|---|---|
| `GET` | 读取（可能写入可重建的缓存）：`config path/list/get`、`db *`、`chat sessions/messages/search/…`、`analytics *`、`group list/members/…`、`sns timeline/…`、`biz *`、`runtime *`、`cache list` 等 |
| `POST` | 执行任务或写文件：`export *`、`report * generate`、`key *`、`backup create/restore`、`chat voice-data/image-data/…`、`sns export/media/…`、`image decrypt`、`insight test/trigger/scan`、`config import`、`ffmpeg install` 等 |
| `PUT` | 修改设置：`config set`、`lang`、`ffmpeg set baseurl`、`insight mark-read`，以及带 `set` 的 `analytics excluded` 和 `chat group-hint` |
| `DELETE` | 删除：`config unset/clear`、`cache clear/clear-account`、`ffmpeg unset baseurl`、`insight clear` |

完整的表是 `crates/weflow-cli/src/serve_api.rs` 里的 `METHODS`，有测试保证它与命令树一致。用其他方法请求返回 `405`，并带 `Allow` 头。有三条命令的方法由参数决定：`analytics excluded` 和 `chat group-hint` 不带 `set` 时是 `GET`，带 `set` 时是 `PUT`；`sns block-delete` 在 `action=check` 时是 `GET`，`install` 时是 `PUT`，`uninstall` 时是 `DELETE`。

会写微信数据库的命令也有路由（`PATCH /chat/update-message`、`DELETE /chat/delete-message`、`POST /chat/mark-read`、`PUT`/`DELETE /chat/anti-revoke/install|uninstall`、`sns block-delete`、`DELETE /sns/delete`）；数据库是只读的，配置好账号后它们返回 `501`，错误码为 `unsupported`。

### 参数

参数名就是长选项名；位置参数用其名称的 kebab-case 形式（`session-id`）。`-` 也可以写成 `_`。

- `GET`：查询参数。其他方法：查询参数和/或 JSON 对象请求体；请求体里的键会替换查询参数里的同名键。
- 多个值：重复同一个键（`?session=a&session=b`），或使用 JSON 数组。
- 开关：`true` / `false`（`?force=true`；值为空表示 `true`）。
- 给单值参数传 JSON 值时，按 JSON 文本传入（用于接收 JSON 数据的参数）。

服务端把参数还原成命令行，再由命令像在终端里一样检查：缺少参数或参数格式错误时返回 `400`，内容是命令自己的 `invalid_arguments` 错误。命令没有的参数名在执行前就返回 `400`。

```sh
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:57108/chat/latest?session-id=wxid_xxx&limit=20"
curl -X PUT -H "Authorization: Bearer $TOKEN" -d '{"key":"no_progress","value":"true"}' http://127.0.0.1:57108/config/set
```

`Accept-Language` 决定消息的语言（`zh…` 为中文，其他为英文）；没有这个头时，命令按平常的规则决定（`WEFLOW_LANG`、`weflow lang`、系统语言）。

## 响应

除 [`?return=file`](#返回文件) 和[作业的事件流](#异步作业)外，响应都是 JSON（`application/json; charset=utf-8`），带 `X-WeFlow-Exit-Code` 头（命令的退出码），失败时还带 `X-WeFlow-Error-Code`。

| 请求 | 是否执行命令 | 响应体 |
|---|---|---|
| 不带 `std` | 执行，加 `--json` | 原样的 `--json` 文档：`{"success": true, "data": …}` 或 `{"success": false, "error": {"code", "message", "details"}}` |
| `?std=out` | 执行，不加 `--json` | `{"exitCode": 0, "stdout": "<终端显示的文本>"}` |
| `?std=err` | 执行，不加 `--json` | `{"exitCode": 0, "stderr": "<警告、进度等>"}` |
| `?std=in` | 不执行 | `{"argv": ["weflow", "--json", …], "stdin": null}`：等价的命令行 |
| `?return=file` | 执行，加 `--json` | 结果加上命令写出的文件（见[返回文件](#返回文件)）；不能与 `std` 同时使用 |

### 状态码

先按错误码，再按退出码。CLI 的退出码不变。

| 错误码 | 退出码 | HTTP |
|---|---|---|
| （成功） | `0` | `200`（以[作业](#异步作业)方式启动的请求为 `202`） |
| `invalid_arguments` | `2` | `400` |
| `config_error` | `3` | `409`（尚未配置账号、密钥或数据库路径） |
| `unsupported` | `4` | `501`（只读数据库拒绝，或尚未移植） |
| `native_error` | `4` | `500` |
| `runtime_error` | `1` | `500`；`details.failureKind` 为 `not_found` 时 `404` |
| `user_interrupt` | `130` | `499` |

`std=out` 和 `std=err` 没有错误码，只按退出码决定。服务端自己的错误也是同样的格式：`unauthorized`（`401`）、`not_found`（`404`）、`method_not_allowed`（`405`）、`invalid_arguments`（`400`）、`busy`（`429`，见[并发限制](#并发限制)）。

## 返回文件

写出单个文件的命令可以在响应里直接带回这个文件：加上 `?return=file`（默认为 `return=json`）。

| 命令 | 文件 |
|---|---|
| `chat voice-data` | WAV 文件 |
| `chat image-data` | 图片 |
| `export sessions`、`export contacts`、`export footprint`、`export messages` | 导出的文件（`export messages --media` 只返回消息文件，媒体留在磁盘上） |
| `group export-member-messages`、`group export-members` | `.csv` / `.xlsx` 文件 |
| `sns media`、`sns download-image` | 图片或视频 |
| `backup create` | 备份压缩包 |

其他命令返回 `400`：它们输出的是目录或多个文件（`export media`、`sns export`、`chat emoji` 等），或者根本不写文件。

- 可以不传 `out`，命令要求必填时也一样：服务端会写到自己的目录，即缓存目录（`<home>/cache` 或 `cache_path`）下的 `serve-files/<请求 id>/`，并自行命名文件（图片和视频的扩展名按内容判断）。传了 `out` 就写到那里，同样返回文件。
- 成功时返回 `200`，`Content-Type: multipart/form-data`，分两部分：`result` 为 `application/json`，内容是 `{"success": true, "data": {"path": "<服务端文件路径>"}}`；`file` 为文件内容，`Content-Type` 按扩展名决定，文件名写在 `filename`（UTF-8）和 `filename*`（RFC 5987）里。文件边读边发。浏览器里用 `await (await fetch(url, init)).formData()` 即可取到两部分。
- 失败时不带文件，返回普通的 JSON 错误和对应状态码。
- `serve-files/` 在 `serve` 启动时清空，并纳入 `cache list` / `cache clear --api`。

```sh
curl -X POST -H "Authorization: Bearer $TOKEN" -o reply.bin "http://127.0.0.1:57108/export/sessions?return=file"
```

## 异步作业

耗时长的命令（导出、`key db`、`backup create`）不必一直占着一个请求。请求时带上 `Prefer: respond-async`，命令就作为作业在后台运行：响应为 `202 Accepted`，带 `Location: /jobs/<id>`，内容是 `{"success": true, "data": <作业>}`。

| 请求 | 响应 |
|---|---|
| `GET /jobs` | `{"success": true, "data": [<作业>, …]}`，从旧到新，不含结果 |
| `GET /jobs/<id>` | 作业及其 `result` |
| `GET /jobs/<id>/events` | 服务器推送事件（SSE），直到作业结束（见下文） |
| `DELETE /jobs/<id>` | 排队中或运行中的作业被取消（终止其命令），返回取消后的作业；已结束的作业则从列表中删除 |

作业的格式为 `{"id", "status", "command", "createdAt", "startedAt", "finishedAt", "exitCode", "httpStatus", "progress", "result"}`：`status` 为 `queued`、`running`、`succeeded`、`failed` 或 `cancelled`；`command` 是实际运行的命令行（总是带 `--json --progress`）；时间为 Unix 毫秒；`httpStatus` 是直接请求时会返回的状态码；`progress` 是最后一个进度事件；`result` 是 `--json` 文档，与直接请求的响应相同。被取消的作业 `httpStatus` 为 `499`，错误码为 `user_interrupt`。

事件流依次发送以下事件，每个都带 `id`：

- `status`：作业出队开始运行时发送 `{"type": "status", "status": "running"}`；
- 命令的 `--progress` 事件，以其 `type` 命名（`progress`：`{"type": "progress", "stage", "message", "current", "total", "percent"}`），以及命令写在 stderr 上的其他事件；
- `stderr`：stderr 上的其他文本行，`{"type": "stderr", "text"}`；
- `result`：结束后的作业及其 `result`，之后事件流结束。

晚订阅或重新连接的客户端会收到已保存的事件（每个作业最多 1000 个）；带 `Last-Event-ID` 时只收到它之后的事件。作业只保存在服务端内存里：服务停止就没有了，已结束的作业只保留最近 100 个。

`?std=out`、`?std=err` 和 `?return=file` 都是直接返回的，不能作为作业（返回 `400`）；`?std=in` 从不执行命令。

```sh
curl -i -X POST -H "Authorization: Bearer $TOKEN" -H "Prefer: respond-async" \
  -d '{"session-id":"wxid_xxx","out":"chat.html","format":"html"}' http://127.0.0.1:57108/export/messages
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:57108/jobs/<id>/events
```

## 并发限制

为了不让机器卡住，有两个上限：任务（`POST` 命令：导出、取钥、备份、报告等）一次只运行一个（`--max-tasks`），其他命令最多同时运行与 CPU 核数相同的数量（`--max-reads`）。超出上限的直接请求不会排队等待，而是返回 `429`，错误码为 `busy`，并带 `Retry-After`（任务为 5 秒，其他为 1 秒）。超出上限的作业在队列里等待（`queued`），有空位时开始运行。

## 执行方式

每个请求都以子进程再运行一次 `weflow`，使用同一份配置，所以输出和退出码与终端一致，单条命令出错也不会拖垮服务。启动进程、读取配置和打开数据库，每个请求要花几十毫秒。客户端在响应完成前断开连接时，它的命令随之终止；取消作业也一样。

尚未实现（见 issue #131）：OpenAPI 描述、`Host` 头校验，以及 WebUI。
