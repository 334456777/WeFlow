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

所有响应都是 JSON（`application/json; charset=utf-8`），带 `X-WeFlow-Exit-Code` 头（命令的退出码），失败时还带 `X-WeFlow-Error-Code`。

| 请求 | 是否执行命令 | 响应体 |
|---|---|---|
| 不带 `std` | 执行，加 `--json` | 原样的 `--json` 文档：`{"success": true, "data": …}` 或 `{"success": false, "error": {"code", "message", "details"}}` |
| `?std=out` | 执行，不加 `--json` | `{"exitCode": 0, "stdout": "<终端显示的文本>"}` |
| `?std=err` | 执行，不加 `--json` | `{"exitCode": 0, "stderr": "<警告、进度等>"}` |
| `?std=in` | 不执行 | `{"argv": ["weflow", "--json", …], "stdin": null}`：等价的命令行 |

### 状态码

先按错误码，再按退出码。CLI 的退出码不变。

| 错误码 | 退出码 | HTTP |
|---|---|---|
| （成功） | `0` | `200` |
| `invalid_arguments` | `2` | `400` |
| `config_error` | `3` | `409`（尚未配置账号、密钥或数据库路径） |
| `unsupported` | `4` | `501`（只读数据库拒绝，或尚未移植） |
| `native_error` | `4` | `500` |
| `runtime_error` | `1` | `500`；`details.failureKind` 为 `not_found` 时 `404` |
| `user_interrupt` | `130` | `499` |

`std=out` 和 `std=err` 没有错误码，只按退出码决定。服务端自己的错误也是同样的格式：`unauthorized`（`401`）、`not_found`（`404`）、`method_not_allowed`（`405`）、`invalid_arguments`（`400`）。

## 执行方式

每个请求都以子进程再运行一次 `weflow`，使用同一份配置，所以输出和退出码与终端一致，单条命令出错也不会拖垮服务。启动进程、读取配置和打开数据库，每个请求要花几十毫秒。

尚未实现（见 issue #131）：带进度和取消的异步作业、并发数限制、在响应里直接返回生成的文件、OpenAPI 描述、`Host` 头校验，以及 WebUI。
