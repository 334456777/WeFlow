# weflow serve：命令 API

[English](../serve-api.md) | **简体中文**

`weflow serve` 把每条 CLI 命令都提供为 HTTP 接口。路径就是命令路径，参数就是命令的选项，返回的就是终端看到的内容：`GET /config/list` 返回的正是 `weflow --json config list` 的输出。设计与计划见 issue #131。

桌面端的 HTTP API（`/api/v1/*`、SSE 推送）是另一个服务，用 `weflow serve --desktop` 启动，见 [HTTP-API.md](HTTP-API.md)。

## 启动

```sh
weflow serve                       # 命令 API，地址 http://127.0.0.1:18108
weflow serve --port 8080 --host 127.0.0.1
weflow serve --desktop             # 改为启动桌面端 API（端口 `http_api_port`，默认 5031）
weflow serve --ui-dir web/dist     # 同时提供网页，地址 http://127.0.0.1:18108/ui/
```

| 选项 | 含义 |
|---|---|
| `--host`、`--port` | 监听地址；默认 `127.0.0.1:18108`（加 `--desktop` 时：`http_api_host` / `http_api_port`，默认 `127.0.0.1:5031`） |
| `--api-token` | 访问令牌（也可用 `WEFLOW_HTTP_TOKEN`，或配置中的 `http_api_token`） |
| `--desktop` | 启动桌面端 API，而不是命令 API |
| `--message-push` | 桌面端 API 的 SSE 推送；需要 `--desktop` |
| `--insight`、`--image-auto-download` | 与任一 API 一起运行这些服务 |
| `--max-tasks` | 同时运行的任务（`POST` 命令）数；默认 1（见[并发限制](#并发限制)） |
| `--max-reads` | 同时运行的其他命令数；默认为 CPU 核数 |
| `--max-media` | `/api/` 媒体路由同时解码的图片和语音数，超出的排队等待；默认为 CPU 核数（见[媒体](#媒体)） |
| `--log-requests` | 每个请求在 stderr 输出一行：方法、路径、状态码和耗时（见[时间花在哪里](#时间花在哪里)） |
| `--ui-dir` | 只在本次运行中，从这个目录（构建好的网页，例如 `web/dist`）提供网页（见[网页](#网页)） |
| `--no-browser` | 启动时不在浏览器中打开网页 |
| `--web-version` | 只在本次运行中显示这个网页版本，需要时先下载（见[网页的版本](#网页的版本)） |
| `--api-only` | 只提供 API，不提供网页 |

启动提示（stderr 上的事件；加 `--json` 时为一行 JSON）会显示地址。默认端口低于 49152，不在 Windows 的动态端口范围内。Hyper-V、WSL 2、Docker 会在这个范围里成段保留端口，被保留的端口任何程序都无法绑定。绑定回环以外的地址时会给出警告：持有令牌的人可以在这台机器上运行任何 `weflow` 命令，使用服务端的文件系统和配置。

## 网页

`serve` 还在 `/ui/` 下提供网页界面（`web/`，issue #130）。用哪个网页：

1. 加 `--api-only` 时不提供网页；
2. 用 `--ui-dir` 指定的目录，只对本次运行有效（目录里没有 `index.html` 时，启动就会报错）；
3. 用 `--web-version` 指定的版本，只对本次运行有效，需要时先下载；
4. 否则用 `weflow web use` 选定的版本；还没有选定时，用这个 `weflow` 自带的版本（构建时 `web/package.json` 里的版本）。这个版本还没下载时，服务端立刻启动（先用本机已有的、适用的最新版本，没有就只提供 API），同时在后台下载，下载完成后显示新版本：发出事件 `webui_ready`；下载失败时发出 `webui_unavailable`，已有的版本保持不变。本机已有但不适用于这个 `weflow` 的版本（例如由更新的 `weflow` 下载的）不会显示：服务端给出警告，改用本机已有的、适用的最新版本，没有就只提供 API。

没有网页时只提供 API，启动提示里会说明。

有网页时，启动提示给出带令牌的地址 `http://127.0.0.1:18108/ui/?token=…`（加 `--json` 时为 `uiUrl`；令牌经过百分号编码，含 `+`、`&`、`#`、`=` 的令牌也能原样传给网页）。stderr 是终端时，服务端会在浏览器中打开它（`--no-browser` 可以关掉）。网页把令牌存在当前标签页里，并从地址栏去掉；令牌不会写进网页的文件。

- 网页的文件不需要令牌，这一点和 API 不同，但下面的 `Host` 检查照样进行。只响应 `GET` 和 `HEAD`。
- `/ui` 跳到 `/ui/`（`308`）。有网页时，`/` 也跳到 `/ui/`（`302`）。查询参数都会保留。
- 网页自己的路径（不带扩展名，例如 `/ui/chat`）不是文件时，返回 `index.html`，由网页自己处理。带扩展名的文件或 `/ui/assets/` 下的文件不存在时，返回 `404`。`/ui/` 以外的路径不会这样回退：未知的 API 路径仍然返回 JSON 格式的 `404`。
- 只提供目录里面的文件：`..`（无论是否编码）和指向目录外的链接都返回 `404`。
- 构建出的资源（`/ui/assets/`，文件名随内容变化）长期缓存；`index.html` 和其他文件每次都重新确认（`Cache-Control: no-cache`），所以新版本立刻生效。所有文件都带 `X-Content-Type-Options: nosniff`，网页还带 `Referrer-Policy: no-referrer`。

### 网页的版本

网页和 `weflow` 分开发布，作为 OCI artifact 发布在 GHCR 上（`ghcr.io/334456777/weflow-web:<版本>`，由 `.github/workflows/web-release.yml` 发布），由 `weflow` 自己下载：不需要 Docker、ORAS、Node.js 或 npm，没有网页时 CLI 和 API 照常使用。

| 命令 | 作用 |
|---|---|
| `web info` | 正在使用的版本、选定的版本、这个 `weflow` 自带的版本，以及本机已有的版本 |
| `web list [--remote]` | 本机已有的版本；加 `--remote` 时列出已发布的版本，以及每个版本是否适用于这个 `weflow` |
| `web install <版本>` | 下载一个版本，校验后保存 |
| `web use <版本>` | 选定 `serve` 显示的版本（需要时先下载） |
| `web update` | 下载并选定已发布的、适用于这个 `weflow` 的最新版本（正式版优先于预发布版） |
| `web prune` | 删除本机上既没有选定、也不是这个 `weflow` 自带的版本；正在运行的 `serve` 显示的版本会保留（列在 `shown` 里），等该服务停止后才能删除 |

- 一个版本适用于这个 `weflow`，要求它需要的 API 正是这个 `weflow` 提供的（`/health` 的 `apiVersion`），并且它要求的最低 `weflow` 版本（`minBackendVersion`）不高于这个 `weflow`。manifest 里写着这两项，所以不适用的版本在下载前就会被拒绝；`weflow-web.json` 里也写着，下载后再检查一次。本机已有的版本可能是其他 `weflow` 下载的，所以每次安装、选定或显示时都会重新检查；`web use` 拒绝不适用的版本时，原来选定的版本保持不变。
- 下载的内容要和 manifest 给出的大小与 SHA-256 摘要一致，然后在限制下解压到单独的目录：压缩包最大 64 MB，解压后最多 256 MB、10,000 个文件，不允许指向目录外的路径和链接。检查 `index.html`、元数据和所要的版本之后，再用一次重命名放到正式位置。下载失败、损坏或中断时，本机已有的版本保持不变；同时下载同一个版本时，只保留一份。
- 这些版本和 WeFlow 的其他缓存放在一起，存放在 `<home>/cache/web/<版本>/`；选定的版本记在 `<home>/cache/web/current` 里。正在运行的 `serve` 对所显示的版本持有 `<home>/cache/web/.lock-<版本>` 的共享锁，`web prune` 删除前会检查这个锁；服务无论以什么方式结束，锁都会随之释放。`cache clear --web` 会把它们全部删除，`serve` 需要时会重新下载。
- 访问仓库的方式遵循 OCI Distribution API：先取匿名令牌，再读 manifest 和 blob。用 `WEFLOW_WEB_REGISTRY` 可以指定其他仓库（镜像，或测试用的仓库）。

## 鉴权

除 `GET /health` 和 `GET /health/events` 外，所有请求都需要令牌，放在 `Authorization: Bearer <令牌>` 头里，或作为 `access_token` 查询参数。没有配置令牌时，`serve` 每次启动生成一个，显示在启动提示里（加 `--json` 时为 `generatedToken`）；设置 `http_api_token` 或传入 `--api-token` 可以固定令牌。

监听回环地址时（默认如此），服务端还会检查每个请求的 `Host` 头，`/health` 也不例外：只响应 `localhost` 和回环地址（`127.0.0.1`、`[::1]` 等），端口不限；其他名称返回 `403`，错误码为 `invalid_host`。这样网页就无法把自己的域名解析到 `127.0.0.1` 来访问 API（DNS rebinding）。用 `--host` 监听其他地址时不做这项检查，因为这时本来就要通过其他名称访问这台机器。

## 请求

### 路径与方法

路径就是命令路径：`weflow chat date-counts` 是 `/chat/date-counts`，`weflow report annual years` 是 `/report/annual/years`。不是命令的路径返回 `404`，`/serve` 也一样。

方法表示命令有哪类副作用：

| 方法 | 命令 |
|---|---|
| `GET` | 读取（可能写入可重建的缓存）：`config path/list/get`、`db *`、`chat sessions/messages/search/…`、`analytics *`、`group list/members/…`、`sns timeline/…`、`biz *`、`runtime *`、`cache list` 等 |
| `POST` | 执行任务或写文件：`export *`、`report * generate`、`key *`、`backup create/restore`、`chat voice-data/image-data/video-data/…`、`sns export/media/…`、`image decrypt`、`insight test/trigger/scan`、`config import`、`ffmpeg install` 等 |
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
curl -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:18108/chat/latest?session-id=wxid_xxx&limit=20"
curl -X PUT -H "Authorization: Bearer $TOKEN" -d '{"key":"no_progress","value":"true"}' http://127.0.0.1:18108/config/set
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

### 体积

请求的 `Accept-Encoding` 接受时，响应用 gzip 或 br 压缩；浏览器总会带上这个头。图片、语音、视频、`?return=file` 的文件、文件的一部分（`206`）和事件流原样发送。`fields=<键>,<键>`（也可以重复写这个参数）让成功的 `--json` 文档里每一行只保留这些键：`data` 是列表时，处理其中的每个对象；`data` 是对象时，处理它里面每个列表中的对象；失败的文档原样返回。它由服务端处理，不是命令的参数，所以两种路由都能用，但不能与 `std`、`return=file`、`Prefer: respond-async` 或媒体路由同时使用（返回 `400`）。

| 响应（大账号，`/api/`） | 原样 | br | `fields` | `fields` 加 br |
|---|---:|---:|---:|---:|
| `chat messages`，50 条消息 | 126 KB，3.5 ms | 18 KB，5.1 ms | 87 KB | 15 KB，4.9 ms |
| `chat contacts`，全部 | 538 KB，6.1 ms | 119 KB，10.7 ms | 154 KB | 71 KB，7.4 ms |

（聊天窗口用的 `fields`：`local_id,server_id,local_type,sort_seq,create_time,sender_username,is_send,message_content,_cursor`；联系人列表用的：`username,alias,remark,nick_name,small_head_url,local_type`。）gzip 比 br 稍大，耗时差不多。两者一起用，全部联系人一次约 70 KB，所以 `chat contacts` 不分页：由网页自己搜索和排序。

## 返回文件

写出单个文件的命令可以在响应里直接带回这个文件：加上 `?return=file`（默认为 `return=json`）。

| 命令 | 文件 |
|---|---|
| `chat voice-data` | WAV 文件 |
| `chat image-data` | 图片 |
| `chat video-data` | 视频（带 `thumb` 时为缩略图或封面） |
| `export sessions`、`export contacts`、`export footprint`、`export messages` | 导出的文件（`export messages --media` 只返回消息文件，媒体留在磁盘上；加 `--zip` 时返回所有格式的文件和 `media` 文件夹打成的 zip） |
| `group export-member-messages`、`group export-members` | `.csv` / `.xlsx` 文件 |
| `sns media`、`sns download-image` | 图片或视频 |
| `backup create` | 备份压缩包 |

其他命令返回 `400`：它们输出的是目录或多个文件（`export media`、`sns export`、`chat emoji` 等），或者根本不写文件。

- 可以不传 `out`，命令要求必填时也一样：服务端会写到自己的目录，即缓存目录（`<home>/cache` 或 `cache_path`）下的 `serve-files/<请求 id>/`，并自行命名文件（图片和视频的扩展名按内容判断）。传了 `out` 就写到那里，同样返回文件。
- 成功时返回 `200`，`Content-Type: multipart/form-data`，分两部分：`result` 为 `application/json`，内容是 `{"success": true, "data": {"path": "<服务端文件路径>"}}`；`file` 为文件内容，`Content-Type` 按扩展名决定，文件名写在 `filename`（UTF-8）和 `filename*`（RFC 5987）里。文件边读边发。浏览器里用 `await (await fetch(url, init)).formData()` 即可取到两部分。
- 失败时不带文件，返回普通的 JSON 错误和对应状态码。
- `serve-files/` 在 `serve` 启动时清空，并纳入 `cache list` / `cache clear --api`。
- 同时带上 `Prefer: respond-async` 时，命令改为作为[作业](#异步作业)运行，并保留它写出的文件：作业成功后，它的 `file` 给出文件名和大小（`{"name", "size"}`），`GET /jobs/<id>/file` 单独发回这个文件，带 `Content-Disposition: attachment`，网页上的链接点开即可保存。作业失败、被取消或被删除时，服务端为它选的文件夹也会删除。

```sh
curl -X POST -H "Authorization: Bearer $TOKEN" -o reply.bin "http://127.0.0.1:18108/export/sessions?return=file"
```

## 异步作业

耗时长的命令（导出、`key db`、`backup create`）不必一直占着一个请求。请求时带上 `Prefer: respond-async`，命令就作为作业在后台运行：响应为 `202 Accepted`，带 `Location: /jobs/<id>`，内容是 `{"success": true, "data": <作业>}`。

| 请求 | 响应 |
|---|---|
| `GET /jobs` | `{"success": true, "data": [<作业>, …]}`，从旧到新，不含结果 |
| `GET /jobs/<id>` | 作业及其 `result` |
| `GET /jobs/<id>/events` | 服务器推送事件（SSE），直到作业结束（见下文） |
| `DELETE /jobs/<id>` | 排队中或运行中的作业被取消（终止其命令），返回取消后的作业；已结束的作业则从列表中删除 |
| `GET /jobs/<id>/file` | 以 `?return=file` 启动的作业成功后写出的文件（见[返回文件](#返回文件)）；否则返回 `404` |

作业的格式为 `{"id", "status", "command", "createdAt", "startedAt", "finishedAt", "exitCode", "httpStatus", "progress", "file", "result"}`：`status` 为 `queued`、`running`、`succeeded`、`failed` 或 `cancelled`；`command` 是实际运行的命令行（总是带 `--json --progress`）；时间为 Unix 毫秒；`httpStatus` 是直接请求时会返回的状态码；`progress` 是最后一个进度事件；`file` 是以 `?return=file` 启动并已成功的作业写出的文件（`{"name", "size"}`），否则为 `null`；`result` 是 `--json` 文档，与直接请求的响应相同。被取消的作业 `httpStatus` 为 `499`，错误码为 `user_interrupt`。

事件流依次发送以下事件，每个都带 `id`：

- `status`：作业出队开始运行时发送 `{"type": "status", "status": "running"}`；
- 命令的 `--progress` 事件，以其 `type` 命名（`progress`：`{"type": "progress", "stage", "message", "current", "total", "percent"}`），以及命令写在 stderr 上的其他事件；
- `stderr`：stderr 上的其他文本行，`{"type": "stderr", "text"}`；
- `result`：结束后的作业及其 `result`，之后事件流结束。

晚订阅或重新连接的客户端会收到已保存的事件（每个作业最多 1000 个）；带 `Last-Event-ID` 时只收到它之后的事件。作业只保存在服务端内存里：服务停止就没有了，已结束的作业只保留最近 100 个。

`?std=out` 和 `?std=err` 都是直接返回的，不能作为作业（返回 `400`）；`?std=in` 从不执行命令。

```sh
curl -i -X POST -H "Authorization: Bearer $TOKEN" -H "Prefer: respond-async" \
  -d '{"session-id":"wxid_xxx","out":"chat.html","format":"html"}' http://127.0.0.1:18108/export/messages
curl -N -H "Authorization: Bearer $TOKEN" http://127.0.0.1:18108/jobs/<id>/events
```

## 并发限制

为了不让机器卡住，有两个上限：任务（`POST` 命令：导出、取钥、备份、报告等）一次只运行一个（`--max-tasks`），其他命令最多同时运行与 CPU 核数相同的数量（`--max-reads`）。不带 `/api/` 的路由每个请求都要启动一个进程，超出上限的直接请求不会排队等待，而是返回 `429`，错误码为 `busy`，并带 `Retry-After`（任务为 5 秒，其他为 1 秒）。超出上限的作业在队列里等待（`queued`），有空位时开始运行。

[`/api/` 的读取](#进程内读取)和它们共用 `--max-reads`，但超出上限时会排队等位置：网页会同时发出好几个请求，而每个只要几毫秒。只有已有 256 个请求在排队，或者一个请求已经等了 10 秒，才返回 `429`（`busy`，`Retry-After: 1`）。`/api/` 的媒体路由有自己的上限和队列（`--max-media`，见[媒体](#媒体)），所以一屏图片不会挡住读取。

| 同时请求同一项读取（消息第一页，大账号，16 核） | `/api/<路径>` | `/<路径>` |
|---|---:|---:|
| 1 个 | 3.7 ms | 31 ms |
| 6 个 | 12 ms | 48 ms |
| 16 个 | 24 ms | 98 ms |
| 32 个 | 46 ms | 105 ms，一半返回 `429` |
| 64 个 | 84 ms，每秒约 760 个 | – |

浏览器对同一个服务器最多同时发 6 个请求（HTTP/1.1），打开的事件流（如 `/api/push/messages`）还要占一个，所以单个网页不会超过默认上限；更多的并发来自多个标签页或脚本。给这些读取的每个数据库多开几个连接并没有变快：时间花在生成每个回复上（一页消息约 120 KB 的 JSON），而不是等数据库。

## OpenAPI 描述

`GET /openapi.json`（需要令牌）返回 API 的 OpenAPI 3.1 描述。它由正在运行的 `weflow` 的命令树和方法表生成，所以总是与之一致；摘要和参数说明来自命令的帮助文字，语言按 `Accept-Language` 决定（没有时用服务端的语言）。

- 每条命令的每个方法是一个操作，`x-weflow-command` 写明对应的命令。`operationId` 是用 `_` 连接的命令路径（`chat_sessions`）；方法由参数决定的命令，每个方法各有一个操作（`analytics_excluded_get`、`analytics_excluded_put`）。
- 会写入微信数据库、因而总被命令以 `501` 拒绝的操作（`chat update-message`、`chat delete-message`、`chat anti-revoke install/uninstall`、带 `install` 或 `uninstall` 的 `sns block-delete`、`sns delete`）带有 `"x-weflow-refused": true`，客户端可以不提供它们。
- `GET` 和 `DELETE` 把参数列为查询参数，其他方法列为 JSON 请求体；两种写法对所有方法都有效（见[参数](#参数)）。类型来自 CLI：数字、开关、列表和可选值。
- 响应只描述外层格式（`{"success", "data"}` 或 `{"success", "error"}`）、作业和 `?return=file`；不描述每条命令 `data` 的结构。
- `/health`、`/health/events`、`/openapi.json`、`/jobs` 和 `/api/push/messages` 归在 `server` 标签下；[进程内读取](#进程内读取)有各自的操作，如 `api_chat_sessions`。

```sh
curl -H "Authorization: Bearer $TOKEN" -H "Accept-Language: zh-CN" -o openapi.json http://127.0.0.1:18108/openapi.json
```

客户端可以通过两个版本号判断自己连的是什么：`version`（`/health` 里，以及 `info.version`）是 `weflow` 的版本；`apiVersion`（`/health` 里，以及 `info.x-weflow-api-version`）是 API 规则的版本，涵盖路由、方法、参数、响应和作业，目前为 `1`。只有会让现有客户端出错的改动才会提升它，新增命令或选项不会。`GET /health` 不需要令牌，所以 WebUI 这样的客户端可以在拿到令牌前先检查这两个版本：

```json
{"status": "ok", "apiVersion": "1", "version": "5.0.0", "data": "5f1c0a9e3b7d2c41",
 "limits": {"maxTasks": 1, "maxReads": 4, "maxMedia": 4}, "insight": false}
```

`data` 是一个不透明的值：服务器每次启动、配置每次改变（例如换成另一个账号）时都会变化。变化之前读到的记录可能属于另一个账号，所以网页会丢掉已经保存的内容（消息、群成员、有消息的日期），重新读取。它是带密钥的哈希，密钥每次运行重新生成，不会透露配置的内容。

`limits` 是可同时运行的数量（`--max-tasks`、`--max-reads`、`--max-media`，见[并发限制](#并发限制)），`insight` 表示是否同时运行灵感引擎（`--insight`）；网页在设置里显示它们。它们只说明服务端的运行方式，不涉及账号。

### 保持连接

`GET /health/events` 以服务器发送事件（SSE）的形式提供同样的内容。网页保持这一条连接，不必反复请求 `/health`。它和 `/health` 一样不需要令牌（`EventSource` 不能带请求头），同样做 `Host` 检查，也不会启动任何后台工作。

- 连上后先发 `event: hello`，数据就是 `/health` 的响应；之后每 15 秒发一次 `event: heartbeat`，数据为 `{"data": "…"}`。心跳是事件而不是注释行，网页能收到。37.5 秒（两个半心跳间隔）没有任何事件，就说明连接半开（断网、NAT 超时等），这种情况单靠连接关闭是发现不了的。
- `/api/push/messages` 开头发同样的 `hello`，也发同样的心跳，所以订阅它的网页不需要再开第二条流。
- 服务端按 Ctrl+C 停止时，这两种流最后都会发一个 `event: bye`（数据为 `{"reason": "stopped"}`，服务端最多等半秒让它发出去），客户端由此能区分服务是正常停止还是连接断了。

网页只保持一条这样的流，并据此显示连接状态：流一断开（服务停止），指示立刻变为「未连接」；之后按 1、2、4 …… 最长 30 秒的间隔重连（上下浮动五分之一），网络恢复、切回标签页或点「重新检测」时立即重连。空闲时不发任何请求。网页和服务在同一台机器上（`127.0.0.1`）时，断网不会切断两者的连接，所以这时只立即重新确认一次，不直接变红。没有 `/health/events` 的服务端（5.0.0-rc.5 及更早）网页不再支持。

打开「实时消息」时（默认打开，在设置里），网页改为保持 `/api/push/messages` 这条流，令牌放在地址的 `access_token` 里，左下角的指示显示「实时」。收到消息事件后，网页重新读取：会话列表，以及打开的会话从最后一条消息往后的部分（`after`）；撤回或断线恢复时，还会重读最新的一页。断线后带着最后一个事件的 id（`last-event-id`）重连；每次运行的事件 id 从启动时刻开始，所以连到重启后的服务端也不会漏掉事件。

## 执行方式

每个请求都以子进程再运行一次 `weflow`，使用同一份配置，所以输出和退出码与终端一致，单条命令出错也不会拖垮服务。客户端在响应完成前断开连接时，它的命令随之终止；取消作业也一样。

代价是每个请求都要付出：启动进程和读取配置约 20 ms，而且每次都要重新打开数据库，缓存都是冷的。在一个大账号上（Windows，release 构建），这样取会话列表约 37 ms，取一页消息约 31 ms。聊天客户端频繁使用的读取请改用 [`/api/`](#进程内读取)。

每个数据库第一次打开时，都要先用配置里的密钥派生出它自己的密钥（约 0.1 秒）。服务端把已经派生出的密钥（即聊天页面要用的数据库的密钥，见下文的预热）通过 stdin 交给它启动的每条命令，命令就不必再派生；没有这一步时，上面两项读取分别约 500 ms 和 175 ms。交来的密钥打不开对应的文件时，命令会重新派生。

### 时间花在哪里

每个响应都带 `Server-Timing` 头，浏览器的开发者工具会在请求旁边显示：`cmd` 是执行命令的时间，`desc="in process"` 表示在服务进程内执行，`desc="child process"` 表示在子进程执行；`wait` 是 `/api/` 请求排队等位置的时间（见[并发限制](#并发限制)）；`total` 是请求在服务端的全部时间，从收到请求到发出响应头（事件流则到它开始）。从 `localhost` 打开的网页还会收到 `Timing-Allow-Origin`，脚本可以在 `performance.getEntriesByType("resource")` 里读到这些阶段。

```text
Server-Timing: wait;desc="in line for a place";dur=0.0, cmd;desc="in process";dur=3.1, total;dur=3.4
```

`--log-requests` 让每个请求在 stderr 输出一行：时间、方法、路径、状态码和毫秒数；带 `--json` 时输出事件 `{"type": "request", "method", "path", "status", "ms"}`。不输出查询参数，因为里面可能有会话 ID 和令牌；也不输出响应的任何内容。默认关闭。

## 进程内读取

`GET /api/<命令路径>` 在服务进程内执行少数几条读命令，数据库在请求之间保持打开：`chat sessions`、`chat messages`、`chat latest`、`chat message`、`chat contact`、`chat contacts`、`chat counts`、`chat search`、`chat dates`、`chat date-counts`、`chat detail`（带 `fast=true`）、`chat resources`、`chat image-info`、`group members`、`sns timeline`（不带 `with-media`）、`sns users`、`sns stats`、`sns post-counts`。参数、回复、状态码和响应头都与不带 `/api/` 的同一路径相同（同一套解析、同一个处理函数，有测试逐字节比较），只是更快：

| 读取（大账号，Windows） | `/<路径>` | `/api/<路径>` |
|---|---|---|
| `chat sessions` | 37 ms | 9 ms |
| `chat messages`，一页 50 条 | 31 ms | 5 ms |
| `chat latest`，20 条 | 29 ms | 4 ms |
| `chat contact` | 24 ms | 2 ms |
| `group members` | 33 ms | 2 ms |
| `chat date-counts`，21.3 万条消息 | 584 ms | 63 ms |
| `chat detail --fast` | 55 ms | 2 ms |
| `chat resources`，一页 300 条 | 109 ms | 42 ms |
| `sns timeline`，20 条 | 235 ms | 6 ms |
| 同时请求 16 页消息（总耗时） | 156 ms | 140 ms |

（这些是 `serve_bench` 测得的中位数，即客户端看到的耗时，见 [native-cli.md](native-cli.md)。）在会话里搜索时，先从最新的消息往前读，找够一页就停，所以常见的词约 5 ms；读了几千条（或 15 ms）还不够，就改查微信自己的索引（见 [cli-unsupported.md](cli-unsupported.md)），所以在有 21 万条消息的会话里，哪里都没有的词约 60 ms，不必读完所有消息。长会话里很靠后的消息页用 `offset` 取会很慢（21 万条消息的中间一页约 1.4 秒），用游标则不会：把已显示的最后一条消息的 `_cursor` 作为 `before` 传入（要更新的消息用 `after`），同一页约 3 ms。

- `chat messages` 带 `parsed=true`（`--parsed`）时，每条消息按桌面端的方式解析后返回，而不是数据库的原始行：`localType`、`parsedContent`、引用（`quotedContent`、`quotedSender`）、媒体的标识（`imageMd5`、`videoMd5`、`voiceDurationSeconds`、`emojiCdnUrl`）、链接、文件、名片、位置、视频号、聊天记录（应用消息的种类见 `appMsgKind`），`_cursor` 照旧。这样聊天窗口不必自己解析 XML。在大账号上，一页 50 条，服务端多花约 1 ms（中位数 1.9 → 2.8 ms），回复比原始行还小；用 `fields` 可以去掉 `rawContent`，即解析前的 XML。
- `chat messages` 带 `since=<Unix 秒>`（`--since`）时，返回从这个时间在聊天顺序中的位置开始最早的若干条消息，仍按最新在前排列，每条都带 `_cursor`：聊天窗口跳到某一天时传那天零点，再用 `before` 和 `after` 往两边翻。这里的顺序就是微信自己的顺序（`sort_seq`，即发送时间的毫秒数再加上一点），也是聊天窗口显示的顺序。个别消息的排序位置和发送时间相差一两秒，这时以排序位置为准：在这个时间之前一点发送的消息可能出现在这一页开头，而恰好在这个时间发送的消息可能紧挨在这一页上面，往前翻一页（`before`）就能看到。这样换来的是前后各页首尾相接，不漏也不重复。它和游标走同一个索引，一样快（大账号上，debug 版读一页 50 条并解析约 22 ms）。
- `chat message` 带 `parsed=true`（`--parsed`）时，只返回这条消息，结构与 `chat messages --parsed` 一致，并带上 `_cursor`。媒体列表可以用 `local-id` 和 `create-time` 找到消息，再用 `before` 和 `after` 翻到它前后的消息。不传 `parsed` 时，保持原来的查询回复。
- `chat contacts` 带 `parsed=true`（`--parsed`）时，按桌面应用的样子返回通讯录，而不是数据库的原始行：只含好友、群聊、公众号和曾经的好友，每一项带 `type`（`friend`、`group`、`official`、`former_friend`）、`displayName`（备注，没有则昵称，再没有则微信号）、`remark`、`nickname`、`alias`、`labels`、`region`、`detailDescription`（签名）和 `avatarUrl`，最近聊过的排在前面。`export contacts` 也接受 `parsed=true`，导出这份列表而不是原始行；再加 `kind`（可重复：`friend`、`group`、`official`、`former_friend`）可以只导出这几类。
- `chat search` 的每条结果都带上所在消息的 `_cursor`（`_session_id` 照旧）：拿它传 `before` 和 `after`，就能读到这条结果前后的消息。带 `parsed=true`（`--parsed`）时，结果按 `chat messages` 的方式解析。
- 不支持 `?std`、`?return=file` 和 `Prefer: respond-async`（返回 `400`）；其他命令返回 `404`。
- 回复使用服务端的语言：`Accept-Language` 不起作用，因为语言是整个进程统一设置的。
- 配置文件变化后（`PUT /config/set`、切换配置档案），下一个请求会重新读取。
- 服务刚启动时，以及配置变化之后，会在后台同时打开聊天页面要用的数据库：会话、联系人、头像、图片链接、搜索索引、消息和语音。每个数据库第一次打开都要先派生密钥（约 0.1 秒），否则最初的几次读取要等它们用到的每个库。在大账号上，预热半秒内完成；之后第一次读取会话列表约 12 ms，原来约 440 ms；如果启动后立即请求，约 240 ms。
- `group members` 和桌面端一样，会把成员面板缓存一段时间：这段时间内返回 `"fromCache": true`；传 `refresh=true` 可以重新生成。
- 不带 `fast=true` 的 `chat detail` 和带 `with-media=true` 的 `sns timeline` 按不带 `/api/` 的路由执行：其余的详情（数据表、最早和最新时间）读起来慢，时间线的媒体要下载。
- 带 `fast=true` 的 `sns stats` 和带 `prefer-cache=true` 的 `sns post-counts` 和桌面端一样，把数字保留 5 分钟：新动态要过了这段时间才计入，不带这两个开关则立即计入。
- 这些读取计入 `--max-reads` 的上限，超出时排队等位置，而不是返回 `429`（见[并发限制](#并发限制)）。

### 媒体

`GET /api/chat/image-data`、`GET /api/chat/voice-data` 和 `GET /api/chat/video-data` 直接返回文件本身，方便网页用 `<img>`、`<audio>`、`<video>` 显示：解密后的图片（`Content-Type` 按内容判断，如 `image/jpeg`），解码为 `audio/wav` 的语音，或微信保存的视频（`video/mp4`）。参数与命令相同，放在查询字符串里：`session-id`、`msg-id`、`create-time`，语音另有 `server-id`、`sender`。不接受 `out`，给了返回 `400`。只允许 `GET`，其他方法返回 `405`。不带 `/api/` 的路由不变：仍是 `POST`，属于任务，写出文件。

```html
<img src="http://127.0.0.1:18108/api/chat/image-data?session-id=wxid_xxx&msg-id=123&create-time=1700000000&access_token=TOKEN">
```

- 加上 `thumb=true`（即命令的 `--thumb`）时，返回微信自己保存的图片缩略图，只有几 KB，适合列表；没有缩略图时返回原图。在大账号上，一张缩略图约 2 ms，大小约为原图的五分之一。
- 一张图片微信最多保存三个文件：聊天里显示的缩略图（`<md5>_t.dat`），点开时显示的图片（`<md5>.dat`），以及点「查看原图」后下载的原图（`<md5>_h.dat`）。用 `variant` 选择其中一个：`best`（默认，本机现有的最好的一个）、`thumb`（与 `thumb=true` 相同）、`normal`（点开时的图片，没有时取现有最好的）、`hd`（原图，微信没下载时返回 `404`，`failureKind: not_found`）。`GET /api/chat/image-info`（`chat image-info`）给出本机有哪几个（`thumb`、`normal`、`hd`），以及消息里记录的大小（`normalBytes`、`hdBytes`，单位字节；原图没下载时也有），供网页判断能不能显示原图。
- 视频加上 `thumb=true` 时，返回微信保存的视频缩略图，没有时返回封面。视频不解码：边从微信的视频文件夹读边发送，只读请求的那一段，所以再长的视频也不占内存，播放器拖动进度时能立即开始：在大账号上，视频的第一个 1 MB 约 4.5 ms，从中间取 1 MB 约 3.5 ms。微信还没下载的视频返回 `404`（`failureKind: not_found`），要先在微信里打开它。
- 回复带 `ETag`（视频的 `ETag` 由文件的大小和修改时间得出）和 `Cache-Control: private, max-age=3600`。带 `If-None-Match` 再次请求时，如果内容没变，返回 `304`，不带文件。
- 可以用 `Range: bytes=…` 只取文件的一部分，`<audio>` 播放器就是这样请求的：返回 `206` 并带 `Content-Range`；起点超出文件末尾时返回 `416`。请求多个区间，或 `If-Range` 指的是文件的另一个版本时，返回整个文件。
- 出错时返回命令的 `--json` 文档和对应的状态码，与其他进程内读取一样。
- 有自己的上限 `--max-media`（默认每个 CPU 一个；视频只在查找文件时占用名额，发送时不占），不计入 `--max-reads`，也不计入 `--max-tasks`。所以一屏图片不会挡住旁边消息的读取。
- 超出上限的请求排队等待，不会立即返回 `429`：一屏缩略图是同时请求的，而每张只要几毫秒。只有已有 256 个请求在排队，或者一个请求已经等了 10 秒，才返回 `429`（`busy`，`Retry-After: 1`）。
- 在大账号上，一张图片约 4 ms；走 `POST /chat/image-data` 约 190 ms。同时请求 20 张时，10 ms 内都有回复。

`GET /api/sns/media` 对朋友圈的图片和视频做同样的事，参数与 `sns media` 相同：`sns timeline` 中某个媒体的 `url` 和 `key`（`media[].thumb` 或 `media[].url`，以及 `media[].key`）。这些媒体加密存放在微信的服务器上，所以第一次请求时服务端会下载并解密，然后保存在缓存里（`<cache>/sns_cache`，用 `cache clear --sns` 清理），之后的请求直接从缓存读取。视频从文件发送，所以和消息里的视频一样支持 `Range`。

- 只接受微信媒体服务器的 `https` 地址（`*.qpic.cn`、`*.wechat.com`、`*.video.qq.com`、`*.tc.qq.com`），其他地址返回 `400`（`invalid_arguments`），因此不能借这个路由访问别的地址。`POST /sns/media` 不变。
- 第一次请求要等微信服务器返回，等待期间占用一个媒体请求名额。

### 消息推送

`GET /api/push/messages` 以服务器发送事件（SSE）推送新消息和撤回，网页不必轮询 `chat latest`：

```js
const events = new EventSource("http://127.0.0.1:18108/api/push/messages?access_token=TOKEN");
events.addEventListener("message.new", (e) => console.log(JSON.parse(e.data)));
```

- 先发送 `hello` 事件（数据就是 `/health` 的响应）和 `ready` 事件，之后每条新消息是一个 `message.new` 事件，每次撤回是一个 `message.revoke` 事件。字段与桌面端的推送相同（见 [HTTP-API.md](HTTP-API.md#2-主动推送)），包括 `sessionId`、`sessionType`、`rawid`、`sourceName`、`groupName`、`content`、`timestamp` 等。每个消息事件都有 `id`。每 15 秒发一次 `heartbeat` 事件，防止空闲连接被关闭，也让客户端知道连接还活着（见[保持连接](#保持连接)）。
- 服务器保留最近 10 分钟的事件，最多 1000 条。客户端可以带 `Last-Event-ID` 重连（EventSource 会自动这样做），也可以新开连接时带上 `?last-event-id=`，这两种情况都会收到该 id 之后的事件。两者都不带时，会收到所有保留的事件。
- 只有在有人订阅时，服务器才会每秒检查一次账号的会话库和消息库是否有变化，有变化时才用进程内读取的 hub 查找新消息；即使没有变化，至少每 30 秒也会查找一次。新消息一两秒内就会到达网页。在大账号上，有人订阅时服务端每秒约占用 5 ms CPU。第一个订阅者以订阅那一刻账号里已有的内容为起点；配置变化（例如换了账号）之后的订阅者也一样。
- 只允许 `GET`。这个连接不计入 `--max-reads`。它与桌面端的 `--message-push` 不是一回事，后者属于 `--desktop`。
