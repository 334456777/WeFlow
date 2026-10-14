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
| `--max-media` | `/api/` 媒体路由同时解码的图片和语音数，超出的排队等待；默认为 CPU 核数（见[媒体](#媒体)） |

启动提示（stderr 上的事件；加 `--json` 时为一行 JSON）会显示地址。绑定回环以外的地址时会给出警告：持有令牌的人可以在这台机器上运行任何 `weflow` 命令，使用服务端的文件系统和配置。

## 鉴权

除 `GET /health` 外，所有请求都需要令牌，放在 `Authorization: Bearer <令牌>` 头里，或作为 `access_token` 查询参数。没有配置令牌时，`serve` 每次启动生成一个，显示在启动提示里（加 `--json` 时为 `generatedToken`）；设置 `http_api_token` 或传入 `--api-token` 可以固定令牌。

监听回环地址时（默认如此），服务端还会检查每个请求的 `Host` 头，`/health` 也不例外：只响应 `localhost` 和回环地址（`127.0.0.1`、`[::1]` 等），端口不限；其他名称返回 `403`，错误码为 `invalid_host`。这样网页就无法把自己的域名解析到 `127.0.0.1` 来访问 API（DNS rebinding）。用 `--host` 监听其他地址时不做这项检查，因为这时本来就要通过其他名称访问这台机器。

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

`std=out` 和 `std=err` 没有错误码，只按退出码决定。服务端自己的错误也是同样的格式：`unauthorized`（`401`）、`invalid_host`（`403`，见[鉴权](#鉴权)）、`not_found`（`404`）、`method_not_allowed`（`405`）、`invalid_arguments`（`400`）、`busy`（`429`，见[并发限制](#并发限制)）。

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

为了不让机器卡住，有两个上限：任务（`POST` 命令：导出、取钥、备份、报告等）一次只运行一个（`--max-tasks`），其他命令最多同时运行与 CPU 核数相同的数量（`--max-reads`）。超出上限的直接请求不会排队等待，而是返回 `429`，错误码为 `busy`，并带 `Retry-After`（任务为 5 秒，其他为 1 秒）。超出上限的作业在队列里等待（`queued`），有空位时开始运行。`/api/` 的媒体路由另有自己的上限（`--max-media`，见[媒体](#媒体)）。

## OpenAPI 描述

`GET /openapi.json`（需要令牌）返回 API 的 OpenAPI 3.1 描述。它由正在运行的 `weflow` 的命令树和方法表生成，所以总是与之一致；摘要和参数说明来自命令的帮助文字，语言按 `Accept-Language` 决定（没有时用服务端的语言）。

- 每条命令的每个方法是一个操作，`x-weflow-command` 写明对应的命令。`operationId` 是用 `_` 连接的命令路径（`chat_sessions`）；方法由参数决定的命令，每个方法各有一个操作（`analytics_excluded_get`、`analytics_excluded_put`）。
- `GET` 和 `DELETE` 把参数列为查询参数，其他方法列为 JSON 请求体；两种写法对所有方法都有效（见[参数](#参数)）。类型来自 CLI：数字、开关、列表和可选值。
- 响应只描述外层格式（`{"success", "data"}` 或 `{"success", "error"}`）、作业和 `?return=file`；不描述每条命令 `data` 的结构。
- `/health`、`/openapi.json`、`/jobs` 和 `/api/push/messages` 归在 `server` 标签下；[进程内读取](#进程内读取)有各自的操作，如 `api_chat_sessions`。

```sh
curl -H "Authorization: Bearer $TOKEN" -H "Accept-Language: zh-CN" -o openapi.json http://127.0.0.1:57108/openapi.json
```

客户端可以通过两个版本号判断自己连的是什么：`version`（`/health` 里，以及 `info.version`）是 `weflow` 的版本；`apiVersion`（`/health` 里，以及 `info.x-weflow-api-version`）是 API 规则的版本，涵盖路由、方法、参数、响应和作业，目前为 `1`。只有会让现有客户端出错的改动才会提升它，新增命令或选项不会。`GET /health` 不需要令牌，所以 WebUI 这样的客户端可以在拿到令牌前先检查这两个版本：

```json
{"status": "ok", "apiVersion": "1", "version": "5.0.0"}
```

## 执行方式

每个请求都以子进程再运行一次 `weflow`，使用同一份配置，所以输出和退出码与终端一致，单条命令出错也不会拖垮服务。客户端在响应完成前断开连接时，它的命令随之终止；取消作业也一样。

代价是每个请求都要付出：启动进程和读取配置约 20 ms，而且每次都要重新打开数据库，缓存都是冷的。在一个大账号上（Windows，release 构建），这样取会话列表约 37 ms，取一页消息约 31 ms。聊天客户端频繁使用的读取请改用 [`/api/`](#进程内读取)。

每个数据库第一次打开时，都要先用配置里的密钥派生出它自己的密钥（约 0.1 秒）。服务端把已经派生出的密钥（即聊天页面要用的数据库的密钥，见下文的预热）通过 stdin 交给它启动的每条命令，命令就不必再派生；没有这一步时，上面两项读取分别约 500 ms 和 175 ms。交来的密钥打不开对应的文件时，命令会重新派生。

## 进程内读取

`GET /api/<命令路径>` 在服务进程内执行少数几条读命令，数据库在请求之间保持打开：`chat sessions`、`chat messages`、`chat latest`、`chat message`、`chat contact`、`chat contacts`、`chat counts`、`chat search`、`group members`。参数、回复、状态码和响应头都与不带 `/api/` 的同一路径相同（同一套解析、同一个处理函数，有测试逐字节比较），只是更快：

| 读取（大账号，Windows） | `/<路径>` | `/api/<路径>` |
|---|---|---|
| `chat sessions` | 37 ms | 9 ms |
| `chat messages`，一页 50 条 | 31 ms | 5 ms |
| `chat latest`，20 条 | 29 ms | 4 ms |
| `chat contact` | 24 ms | 2 ms |
| `group members` | 33 ms | 2 ms |
| 同时请求 16 页消息（总耗时） | 156 ms | 140 ms |

（这些是 `serve_bench` 测得的中位数，即客户端看到的耗时，见 [native-cli.md](native-cli.md)。）搜索从最新的消息往前读，找够一页就停，所以常见的词约 5 ms；但哪里都没有的词仍要读完所有消息（21 万条约 0.7 秒）。长会话里很靠后的消息页用 `offset` 取会很慢（21 万条消息的中间一页约 1.4 秒），用游标则不会：把已显示的最后一条消息的 `_cursor` 作为 `before` 传入（要更新的消息用 `after`），同一页约 3 ms。

- 不支持 `?std`、`?return=file` 和 `Prefer: respond-async`（返回 `400`）；其他命令返回 `404`。
- 回复使用服务端的语言：`Accept-Language` 不起作用，因为语言是整个进程统一设置的。
- 配置文件变化后（`PUT /config/set`、切换配置档案），下一个请求会重新读取。
- 服务刚启动时，以及配置变化之后，会在后台同时打开聊天页面要用的数据库：会话、联系人、头像、图片链接、消息和语音。每个数据库第一次打开都要先派生密钥（约 0.1 秒），否则最初的几次读取要等它们用到的每个库。在大账号上，预热半秒内完成；之后第一次读取会话列表约 12 ms，原来约 440 ms；如果启动后立即请求，约 240 ms。
- `group members` 和桌面端一样，会把成员面板缓存一段时间：这段时间内返回 `"fromCache": true`；传 `refresh=true` 可以重新生成。
- 这些读取计入 `--max-reads` 的上限。

### 媒体

`GET /api/chat/image-data` 和 `GET /api/chat/voice-data` 直接返回文件本身，方便网页用 `<img>`、`<audio>` 显示：解密后的图片（`Content-Type` 按内容判断，如 `image/jpeg`），或解码为 `audio/wav` 的语音。参数与命令相同，放在查询字符串里：`session-id`、`msg-id`、`create-time`，语音另有 `server-id`、`sender`。不接受 `out`，给了返回 `400`。只允许 `GET`，其他方法返回 `405`。不带 `/api/` 的路由不变：仍是 `POST`，属于任务，写出文件。

```html
<img src="http://127.0.0.1:57108/api/chat/image-data?session-id=wxid_xxx&msg-id=123&create-time=1700000000&access_token=TOKEN">
```

- 回复带 `ETag` 和 `Cache-Control: private, max-age=3600`。带 `If-None-Match` 再次请求时，如果内容没变，返回 `304`，不带文件。
- 出错时返回命令的 `--json` 文档和对应的状态码，与其他进程内读取一样。
- 有自己的上限 `--max-media`（默认每个 CPU 一个），不计入 `--max-reads`，也不计入 `--max-tasks`。所以一屏图片不会挡住旁边消息的读取。
- 超出上限的请求排队等待，不会立即返回 `429`：一屏缩略图是同时请求的，而每张只要几毫秒。只有已有 256 个请求在排队，或者一个请求已经等了 10 秒，才返回 `429`（`busy`，`Retry-After: 1`）。
- 在大账号上，一张图片约 4 ms；走 `POST /chat/image-data` 约 190 ms。同时请求 20 张时，10 ms 内都有回复。

### 消息推送

`GET /api/push/messages` 以服务器发送事件（SSE）推送新消息和撤回，网页不必轮询 `chat latest`：

```js
const events = new EventSource("http://127.0.0.1:57108/api/push/messages?access_token=TOKEN");
events.addEventListener("message.new", (e) => console.log(JSON.parse(e.data)));
```

- 先发送 `ready` 事件，之后每条新消息是一个 `message.new` 事件，每次撤回是一个 `message.revoke` 事件。字段与桌面端的推送相同（见 [HTTP-API.md](HTTP-API.md#2-主动推送)），包括 `sessionId`、`sessionType`、`rawid`、`sourceName`、`groupName`、`content`、`timestamp` 等。每个事件都有 `id`。连接空闲时每 25 秒发送一行注释，防止连接被关闭。
- 服务器保留最近 10 分钟的事件，最多 1000 条。客户端可以带 `Last-Event-ID` 重连（EventSource 会自动这样做），也可以新开连接时带上 `?last-event-id=`，这两种情况都会收到该 id 之后的事件。两者都不带时，会收到所有保留的事件。
- 只有在有人订阅时，服务器才会每秒检查一次账号的会话库和消息库是否有变化，有变化时才用进程内读取的 hub 查找新消息；即使没有变化，至少每 30 秒也会查找一次。新消息一两秒内就会到达网页。在大账号上，有人订阅时服务端每秒约占用 5 ms CPU。第一个订阅者以订阅那一刻账号里已有的内容为起点；配置变化（例如换了账号）之后的订阅者也一样。
- 只允许 `GET`。这个连接不计入 `--max-reads`。它与桌面端的 `--message-push` 不是一回事，后者属于 `--desktop`。
