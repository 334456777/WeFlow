//! `GET /openapi.json`: an OpenAPI 3.1 description of the API, built from the command tree and [`METHODS`] the
//! same way the routes are, so it always matches the running binary. Help texts are in the language of the
//! request (`Accept-Language`), else the server's.

use std::any::TypeId;

use clap::builder::StyledStr;
use clap::{Arg, Command, CommandFactory};
use serde_json::{json, Map, Value};

use super::{
    api_args, files, in_process, param_of, Kind, Rule, Verb, API_VERSION, HIDDEN, METHODS,
};

fn pick(zh: bool, en: &str, cn: &str) -> String {
    if zh { cn } else { en }.to_string()
}

fn text(s: Option<&StyledStr>) -> String {
    s.map(|s| s.to_string().trim().to_string())
        .unwrap_or_default()
}

fn verb_name(verb: Verb) -> &'static str {
    match verb {
        Verb::Get => "GET",
        Verb::Post => "POST",
        Verb::Put => "PUT",
        Verb::Patch => "PATCH",
        Verb::Delete => "DELETE",
    }
}

/// The description, in Chinese when `zh`.
pub(super) fn document(zh: bool) -> Value {
    let mut cmd = crate::Cli::command();
    if zh {
        cmd = crate::i18n::localize_command(cmd);
    }
    cmd.build();
    let mut paths = Map::new();
    fixed_paths(zh, &mut paths);
    let mut tags = vec![json!({
        "name": "server",
        "description": pick(zh, "The server's own endpoints", "服务器自身的接口"),
    })];
    for sub in cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help" && !HIDDEN.contains(&c.get_name()))
    {
        tags.push(json!({ "name": sub.get_name(), "description": text(sub.get_about()) }));
        walk(sub, &mut vec![sub.get_name().to_string()], zh, &mut paths);
    }
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "WeFlow",
            "version": env!("WEFLOW_VERSION"),
            "x-weflow-api-version": API_VERSION,
            "description": pick(
                zh,
                "Every `weflow` command over HTTP: the path is the command path, the parameters are its long \
                 option names (or argument names), and the answer is its `--json` document. `GET` and `DELETE` \
                 take parameters in the query string, the other methods in a JSON body; either way works for \
                 every method, and body keys win. See docs/serve-api.md.",
                "通过 HTTP 调用每个 `weflow` 命令：路径就是命令路径，参数是命令的长选项名（或参数名），回复是命令的 \
                 `--json` 文档。`GET` 和 `DELETE` 的参数写在查询字符串里，其他方法写在 JSON 请求体里；两种写法对所有\
                 方法都有效，同名时以请求体为准。见 docs/zh-CN/serve-api.md。",
            ),
        },
        "tags": tags,
        "security": [{ "bearer": [] }, { "accessToken": [] }],
        "paths": paths,
        "components": components(zh),
    })
}

fn walk(cmd: &Command, path: &mut Vec<String>, zh: bool, paths: &mut Map<String, Value>) {
    let subs: Vec<&Command> = cmd
        .get_subcommands()
        .filter(|c| c.get_name() != "help")
        .collect();
    if subs.is_empty() {
        let name = path.join(" ");
        if let Some((_, rule)) = METHODS.iter().find(|(p, _)| *p == name) {
            let item = path_item(cmd, path, *rule, zh);
            if in_process::COMMANDS.contains(&name.as_str()) {
                paths.insert(
                    format!("/api/{}", path.join("/")),
                    in_process_item(&item, &name, zh),
                );
            }
            if in_process::MEDIA.contains(&name.as_str()) {
                paths.insert(
                    format!("/api/{}", path.join("/")),
                    media_item(cmd, path, zh),
                );
            }
            paths.insert(format!("/{}", path.join("/")), item);
        }
        return;
    }
    for sub in subs {
        path.push(sub.get_name().to_string());
        walk(sub, path, zh, paths);
        path.pop();
    }
}

/// The `/api/` twin of a command's path item: the same parameters and answer, without what only a child process
/// does (`std`, `return=file`, jobs, `Accept-Language`).
fn in_process_item(item: &Value, name: &str, zh: bool) -> Value {
    let mut item = item.clone();
    for op in item.as_object_mut().unwrap().values_mut() {
        op["operationId"] = format!("api_{}", op["operationId"].as_str().unwrap()).into();
        let mut note = pick(
            zh,
            "Runs in the server's process on databases it keeps open: much faster than the route without \
             `/api/`, with the same parameters and answer, in the server's language.",
            "在服务进程内执行，数据库保持打开：比不带 `/api/` 的路由快得多，参数和回复相同，语言为服务端的语言。",
        );
        if let Some((_, en, cn)) = in_process::IN_PART.iter().find(|(n, ..)| *n == name) {
            note = format!("{note}{}{}", if zh { "" } else { " " }, pick(zh, en, cn));
        }
        op["description"] = match op["description"].as_str() {
            Some(d) => format!("{note}\n\n{d}").into(),
            None => note.into(),
        };
        if let Some(params) = op["parameters"].as_array_mut() {
            // of the parameters every route has, only `fields` applies to /api/
            params.retain(|p| {
                p.get("$ref")
                    .is_none_or(|r| r == "#/components/parameters/fields")
            });
        }
        if let Some(responses) = op["responses"].as_object_mut() {
            responses.remove("202");
        }
    }
    item
}

/// The `/api/` route of a media command: a `GET` with the command's parameters but `out`, answering the file.
fn media_item(cmd: &Command, path: &[String], zh: bool) -> Value {
    let mut op = operation(cmd, path, Verb::Get, &Variant::All, false, None, zh);
    op["operationId"] = format!("api_{}", op["operationId"].as_str().unwrap()).into();
    if let Some(params) = op["parameters"].as_array_mut() {
        params.retain(|p| p.get("$ref").is_none() && p["name"] != "out");
    }
    op["description"] = pick(
        zh,
        "Runs in the server's process and answers the file itself, for a page to show; counted with the other \
         media requests (`--max-media`), which wait in line for a place. The answer has an `ETag` and may be kept \
         for an hour; ask again with `If-None-Match` to get `304` when it has not changed. `Range: bytes=…` asks \
         for a part (`206`, or `416` past the end). Errors are the command's `--json` document, in the server's \
         language.",
        "在服务进程内执行，直接返回文件本身，供网页显示；计入媒体请求的上限（`--max-media`），超出时排队等待。\
         回复带 `ETag`，可以缓存一小时；带 `If-None-Match` 再次请求时，内容没变就返回 `304`。`Range: bytes=…` \
         只取一部分（`206`；起点超出末尾时 `416`）。出错时返回命令的 `--json` 文档，语言为服务端的语言。",
    )
    .into();
    let content = match path.last().map(String::as_str) {
        Some("voice-data") => json!({ "audio/wav": {} }),
        Some("video-data") => json!({ "video/mp4": {}, "image/*": {} }),
        _ => json!({ "image/*": {} }),
    };
    let what = match path.last().map(String::as_str) {
        Some("voice-data") => pick(zh, "The voice as WAV", "WAV 格式的语音"),
        Some("video-data") => pick(
            zh,
            "The video as WeChat keeps it; with `thumb=true`, its thumbnail or cover",
            "微信保存的视频；带 `thumb=true` 时为缩略图或封面",
        ),
        _ => pick(zh, "The decrypted image", "解密后的图片"),
    };
    op["responses"] = json!({
        "200": {
            "description": what,
            "headers": {
                "ETag": { "schema": { "type": "string" } },
                "Cache-Control": { "schema": { "type": "string" } },
            },
            "content": content.clone(),
        },
        "206": {
            "description": pick(zh, "The part asked for in `Range`", "`Range` 请求的那一部分"),
            "headers": { "Content-Range": { "schema": { "type": "string" } } },
            "content": content,
        },
        "304": { "description": pick(zh, "Unchanged since the `ETag` sent in `If-None-Match`", "自 `If-None-Match` 中的 `ETag` 以来没有变化") },
        "416": { "description": pick(zh, "The range starts past the end of the file", "请求的区间起点超出文件末尾") },
        "4XX": { "$ref": "#/components/responses/Error" },
        "5XX": { "$ref": "#/components/responses/Error" },
    });
    let mut item = Map::new();
    item.insert("get".into(), op);
    Value::Object(item)
}

/// What one operation of a command whose method depends on an argument covers.
#[derive(Clone)]
enum Variant {
    /// The command has one method.
    All,
    /// The argument is not given.
    Without(&'static str),
    /// The argument is given.
    With(&'static str),
    /// The argument has one of these values.
    OneOf(&'static str, Vec<&'static str>),
}

fn path_item(cmd: &Command, path: &[String], rule: Rule, zh: bool) -> Value {
    let (variants, note) = match rule {
        Rule::Fixed(v) => (vec![(v, Variant::All)], None),
        Rule::Arg { arg, without, with } => (
            vec![(without, Variant::Without(arg)), (with, Variant::With(arg))],
            Some(pick(
                zh,
                &format!(
                    "{} when `{arg}` is given, {} otherwise.",
                    verb_name(with),
                    verb_name(without)
                ),
                &format!(
                    "给出 `{arg}` 时用 {}，否则用 {}。",
                    verb_name(with),
                    verb_name(without)
                ),
            )),
        ),
        Rule::Value { arg, values } => {
            let mut variants: Vec<(Verb, Variant)> = Vec::new();
            for (value, verb) in values {
                match variants.iter_mut().find(|(v, _)| v == verb) {
                    Some((_, Variant::OneOf(_, list))) => list.push(value),
                    _ => variants.push((*verb, Variant::OneOf(arg, vec![value]))),
                }
            }
            let pairs: Vec<String> = values
                .iter()
                .map(|(value, verb)| format!("`{value}` → {}", verb_name(*verb)))
                .collect();
            let note = pick(
                zh,
                &format!("The method follows `{arg}`: {}.", pairs.join(", ")),
                &format!("方法由 `{arg}` 决定：{}。", pairs.join("，")),
            );
            (variants, Some(note))
        }
    };
    let many = variants.len() > 1;
    let mut item = Map::new();
    for (verb, variant) in variants {
        item.insert(
            verb_name(verb).to_ascii_lowercase(),
            operation(cmd, path, verb, &variant, many, note.as_deref(), zh),
        );
    }
    Value::Object(item)
}

fn operation(
    cmd: &Command,
    path: &[String],
    verb: Verb,
    variant: &Variant,
    many: bool,
    note: Option<&str>,
    zh: bool,
) -> Value {
    let name = path.join(" ");
    let in_body = matches!(verb, Verb::Post | Verb::Put | Verb::Patch);
    let mut parameters = Vec::new();
    let mut properties = Map::new();
    let mut required = Vec::new();
    for arg in api_args(cmd) {
        let param = param_of(arg);
        let mut schema = schema_of(arg, param.kind);
        let mut needed = arg.is_required_set();
        match variant {
            Variant::Without(a) if param.name == *a => continue,
            Variant::With(a) if param.name == *a => needed = true,
            Variant::OneOf(a, values) if param.name == *a => {
                schema = json!({ "type": "string", "enum": values });
                needed = true;
            }
            _ => {}
        }
        let help = text(arg.get_long_help().or(arg.get_help()));
        if in_body {
            if !help.is_empty() {
                schema["description"] = help.into();
            }
            if needed {
                required.push(param.name.clone());
            }
            properties.insert(param.name, schema);
        } else {
            let mut p = json!({ "name": param.name, "in": "query", "schema": schema });
            if needed {
                p["required"] = true.into();
            }
            if !help.is_empty() {
                p["description"] = help.into();
            }
            parameters.push(p);
        }
    }
    let file = files::file_command(&name).is_some();
    for common in ["std", "fields", "Prefer", "Accept-Language"] {
        parameters.push(json!({ "$ref": format!("#/components/parameters/{common}") }));
    }
    if file {
        parameters.push(json!({ "$ref": "#/components/parameters/return" }));
    }

    let mut id = name.replace([' ', '-'], "_");
    if many {
        id = format!("{id}_{}", verb_name(verb).to_ascii_lowercase());
    }
    let mut op = json!({
        "operationId": id,
        "tags": [path[0]],
        "summary": text(cmd.get_about()),
        "x-weflow-command": name,
        "parameters": parameters,
        "responses": command_responses(file, zh),
    });
    let description: Vec<String> = [text(cmd.get_long_about())]
        .into_iter()
        .chain(note.map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect();
    if !description.is_empty() {
        op["description"] = description.join("\n\n").into();
    }
    if in_body && !properties.is_empty() {
        let mut schema = json!({
            "type": "object",
            "properties": properties,
            "additionalProperties": false,
        });
        if !required.is_empty() {
            schema["required"] = json!(required);
        }
        op["requestBody"] = json!({
            "required": !required.is_empty(),
            "content": { "application/json": { "schema": schema } },
        });
    }
    op
}

/// The schema of one value of an argument, from its value parser.
fn value_schema(arg: &Arg) -> Value {
    let id = arg.get_value_parser().type_id();
    let is = |types: &[TypeId]| types.iter().any(|t| id == *t);
    let integers = [
        TypeId::of::<i8>(),
        TypeId::of::<i16>(),
        TypeId::of::<i32>(),
        TypeId::of::<i64>(),
        TypeId::of::<isize>(),
        TypeId::of::<u8>(),
        TypeId::of::<u16>(),
        TypeId::of::<u32>(),
        TypeId::of::<u64>(),
        TypeId::of::<usize>(),
    ];
    let ty = if is(&integers) {
        "integer"
    } else if is(&[TypeId::of::<f32>(), TypeId::of::<f64>()]) {
        "number"
    } else if is(&[TypeId::of::<bool>()]) {
        "boolean"
    } else {
        "string"
    };
    let values: Vec<String> = arg
        .get_possible_values()
        .into_iter()
        .filter(|v| !v.is_hide_set())
        .map(|v| v.get_name().to_string())
        .collect();
    if ty == "string" && !values.is_empty() {
        return json!({ "type": "string", "enum": values });
    }
    json!({ "type": ty })
}

fn schema_of(arg: &Arg, kind: Kind) -> Value {
    let mut schema = match kind {
        Kind::Flag => json!({ "type": "boolean" }),
        Kind::Single => value_schema(arg),
        Kind::Multi | Kind::MultiOrBare => json!({ "type": "array", "items": value_schema(arg) }),
    };
    if kind == Kind::Single {
        if let Some(default) = arg.get_default_values().first() {
            let default = default.to_string_lossy();
            schema["default"] = match schema["type"].as_str() {
                Some("string") => Value::from(default.as_ref()),
                _ => serde_json::from_str(&default).unwrap_or_else(|_| default.as_ref().into()),
            };
        }
    }
    schema
}

fn command_responses(file: bool, zh: bool) -> Value {
    let mut ok = json!({
        "description": pick(
            zh,
            "The command's `--json` document (`std` asks for other shapes)",
            "命令的 `--json` 文档（`std` 可以要求其他形式）",
        ),
        "headers": { "X-WeFlow-Exit-Code": { "$ref": "#/components/headers/ExitCode" } },
        "content": { "application/json": { "schema": { "$ref": "#/components/schemas/Result" } } },
    });
    if file {
        ok["content"]["multipart/form-data"] = json!({
            "schema": { "$ref": "#/components/schemas/FileResult" },
            "encoding": { "result": { "contentType": "application/json" } },
        });
    }
    json!({
        "200": ok,
        "202": { "$ref": "#/components/responses/Accepted" },
        "4XX": { "$ref": "#/components/responses/Error" },
        "5XX": { "$ref": "#/components/responses/Error" },
    })
}

fn json_content(schema: Value) -> Value {
    json!({ "application/json": { "schema": schema } })
}

fn success_of(data: Value) -> Value {
    json!({
        "type": "object",
        "required": ["success", "data"],
        "properties": { "success": { "const": true }, "data": data },
    })
}

fn fixed_paths(zh: bool, paths: &mut Map<String, Value>) {
    let error = json!({ "$ref": "#/components/responses/Error" });
    let job = json!({ "$ref": "#/components/schemas/Job" });
    let job_id = json!({
        "name": "id",
        "in": "path",
        "required": true,
        "schema": { "type": "string" },
    });
    paths.insert(
        "/health".into(),
        json!({ "get": {
            "operationId": "health",
            "tags": ["server"],
            "summary": pick(zh, "Whether the server is up, and its versions", "服务器是否在运行，以及版本号"),
            "security": [],
            "responses": {
                "200": {
                    "description": pick(zh, "The server is up", "服务器在运行"),
                    "content": json_content(json!({
                        "type": "object",
                        "required": ["status", "apiVersion", "version"],
                        "properties": {
                            "status": { "const": "ok" },
                            "apiVersion": {
                                "type": "string",
                                "description": pick(
                                    zh,
                                    "Version of the API's rules; raised only by changes that break clients",
                                    "API 规则的版本；只有会让现有客户端出错的改动才会提升",
                                ),
                            },
                            "version": {
                                "type": "string",
                                "description": pick(zh, "Version of weflow", "weflow 的版本"),
                            },
                            "data": {
                                "type": "string",
                                "description": pick(
                                    zh,
                                    "Changes with each start of the server and with its configuration (so with the account); records read before it changed may be another account's",
                                    "每次启动服务器、配置改变（包括切换账号）时都会变化；变化之前读到的记录可能属于另一个账号",
                                ),
                            },
                        },
                    })),
                },
                "4XX": error,
            },
        }}),
    );
    paths.insert(
        "/health/events".into(),
        json!({ "get": {
            "operationId": "health_events",
            "tags": ["server"],
            "summary": pick(zh, "The server's state as a stream: hello, then a heartbeat every 15 s", "以事件流提供服务器状态：先发 hello，之后每 15 秒一次心跳"),
            "description": pick(
                zh,
                "Server-sent events. `hello` carries what `/health` answers; `heartbeat` carries `{\"data\": …}`; `bye` ends the stream when the server stops on Ctrl+C. No event for 37.5 s means the connection is dead.",
                "服务器发送事件（SSE）。`hello` 的数据就是 `/health` 的响应；`heartbeat` 的数据为 `{\"data\": …}`；服务端按 Ctrl+C 停止时，以 `bye` 结束这条流。37.5 秒收不到任何事件，说明连接已经断开。",
            ),
            "security": [],
            "responses": {
                "200": {
                    "description": pick(zh, "The event stream", "事件流"),
                    "content": { "text/event-stream": { "schema": { "type": "string" } } },
                },
                "4XX": error,
            },
        }}),
    );
    paths.insert(
        "/openapi.json".into(),
        json!({ "get": {
            "operationId": "openapi",
            "tags": ["server"],
            "summary": pick(zh, "This description", "本描述文档"),
            "parameters": [{ "$ref": "#/components/parameters/Accept-Language" }],
            "responses": {
                "200": {
                    "description": pick(zh, "An OpenAPI 3.1 document", "OpenAPI 3.1 文档"),
                    "content": json_content(json!({ "type": "object" })),
                },
                "4XX": error,
            },
        }}),
    );
    paths.insert(
        "/jobs".into(),
        json!({ "get": {
            "operationId": "jobs_list",
            "tags": ["server"],
            "summary": pick(
                zh,
                "Jobs started with `Prefer: respond-async`, oldest first",
                "用 `Prefer: respond-async` 启动的作业，按时间先后排列",
            ),
            "responses": {
                "200": {
                    "description": pick(zh, "The jobs, without their results", "作业列表，不含结果"),
                    "content": json_content(success_of(json!({ "type": "array", "items": job }))),
                },
                "4XX": error,
            },
        }}),
    );
    paths.insert(
        "/jobs/{id}".into(),
        json!({
            "parameters": [job_id],
            "get": {
                "operationId": "jobs_get",
                "tags": ["server"],
                "summary": pick(zh, "A job, with its result once it has finished", "一个作业，结束后带上结果"),
                "responses": {
                    "200": {
                        "description": pick(zh, "The job", "作业"),
                        "content": json_content(success_of(job.clone())),
                    },
                    "4XX": error,
                },
            },
            "delete": {
                "operationId": "jobs_delete",
                "tags": ["server"],
                "summary": pick(
                    zh,
                    "Cancels a queued or running job, or forgets a finished one",
                    "取消排队中或运行中的作业，或删除已结束的作业",
                ),
                "responses": {
                    "200": {
                        "description": pick(zh, "The job as it ended", "作业结束时的状态"),
                        "content": json_content(success_of(job)),
                    },
                    "4XX": error,
                },
            },
        }),
    );
    paths.insert(
        "/jobs/{id}/events".into(),
        json!({ "get": {
            "operationId": "jobs_events",
            "tags": ["server"],
            "summary": pick(
                zh,
                "The job's progress as server-sent events, ending with `result`",
                "以服务器发送事件（SSE）推送作业进度，最后一个事件是 `result`",
            ),
            "parameters": [
                job_id,
                {
                    "name": "Last-Event-ID",
                    "in": "header",
                    "schema": { "type": "integer" },
                    "description": pick(
                        zh,
                        "Resume after this event",
                        "从这个事件之后继续",
                    ),
                },
            ],
            "responses": {
                "200": {
                    "description": pick(
                        zh,
                        "Events named `status`, `progress`, `stderr`, the command's own event types, and `result`",
                        "事件名为 `status`、`progress`、`stderr`、命令自身的事件类型，以及 `result`",
                    ),
                    "content": { "text/event-stream": { "schema": { "type": "string" } } },
                },
                "4XX": error,
            },
        }}),
    );
    paths.insert(
        "/api/push/messages".into(),
        json!({ "get": {
            "operationId": "api_push_messages",
            "tags": ["server"],
            "summary": pick(
                zh,
                "New and recalled messages as server-sent events",
                "以服务器发送事件（SSE）推送新消息和撤回",
            ),
            "description": pick(
                zh,
                "First a `ready` event, then the kept events (up to 10 minutes or 1000 of them) after `Last-Event-ID`, \
                 then each new one as `message.new` or `message.revoke`, with the fields of the desktop app's push. \
                 The server looks for new messages once a second while someone listens. The stream is not counted \
                 with the reads.",
                "先发送 `ready` 事件，再补发 `Last-Event-ID` 之后保留的事件（最多 10 分钟或 1000 条），之后每条新事件为 \
                 `message.new` 或 `message.revoke`，字段与桌面端的推送相同。有人订阅时，服务器每秒检查一次新消息。\
                 这个连接不计入读取的并发上限。",
            ),
            "parameters": [
                {
                    "name": "Last-Event-ID",
                    "in": "header",
                    "schema": { "type": "integer" },
                    "description": pick(zh, "Resume after this event", "从这个事件之后继续"),
                },
                {
                    "name": "last-event-id",
                    "in": "query",
                    "schema": { "type": "integer" },
                    "description": pick(
                        zh,
                        "The same, for a page that opens a new stream",
                        "作用相同，供网页新开连接时使用",
                    ),
                },
            ],
            "responses": {
                "200": {
                    "description": pick(
                        zh,
                        "Events named `ready`, `message.new` and `message.revoke`",
                        "事件名为 `ready`、`message.new` 和 `message.revoke`",
                    ),
                    "content": { "text/event-stream": { "schema": { "type": "string" } } },
                },
                "4XX": error,
            },
        }}),
    );
}

fn components(zh: bool) -> Value {
    json!({
        "securitySchemes": {
            "bearer": {
                "type": "http",
                "scheme": "bearer",
                "description": pick(
                    zh,
                    "`http_api_token` from the config, `--api-token`, or the token printed at start",
                    "配置里的 `http_api_token`、`--api-token`，或启动时打印的令牌",
                ),
            },
            "accessToken": { "type": "apiKey", "in": "query", "name": "access_token" },
        },
        "parameters": {
            "std": {
                "name": "std",
                "in": "query",
                "schema": { "type": "string", "enum": ["in", "out", "err"] },
                "description": pick(
                    zh,
                    "`in`: the equivalent command line, not run (`{argv, stdin}`); `out` / `err`: run without \
                     `--json` and answer its stdout / stderr (`{exitCode, stdout}` / `{exitCode, stderr}`)",
                    "`in`：返回等价的命令行，不执行（`{argv, stdin}`）；`out` / `err`：不带 `--json` 运行，返回它的 \
                     stdout / stderr（`{exitCode, stdout}` / `{exitCode, stderr}`）",
                ),
            },
            "fields": {
                "name": "fields",
                "in": "query",
                "style": "form",
                "explode": false,
                "schema": { "type": "array", "items": { "type": "string" } },
                "description": pick(
                    zh,
                    "Only these keys of each row of a successful answer: of each object in `data`, or in the lists of \
                     `data`; not with `std`, `return=file` or `Prefer: respond-async`",
                    "成功的回复里每一行只保留这些键：`data` 里的每个对象，或 `data` 里各个列表中的对象；不能与 `std`、\
                     `return=file` 或 `Prefer: respond-async` 同时使用",
                ),
            },
            "return": {
                "name": "return",
                "in": "query",
                "schema": { "type": "string", "enum": ["json", "file"] },
                "description": pick(
                    zh,
                    "`file`: answer the written file with the result, as multipart/form-data",
                    "`file`：以 multipart/form-data 返回结果和写出的文件",
                ),
            },
            "Prefer": {
                "name": "Prefer",
                "in": "header",
                "schema": { "type": "string", "examples": ["respond-async"] },
                "description": pick(
                    zh,
                    "`respond-async`: run the command as a job and answer 202 with its `Location`",
                    "`respond-async`：把命令作为作业运行，回复 202 和它的 `Location`",
                ),
            },
            "Accept-Language": {
                "name": "Accept-Language",
                "in": "header",
                "schema": { "type": "string", "examples": ["zh-CN", "en"] },
                "description": pick(zh, "Language of messages: zh or en", "消息的语言：zh 或 en"),
            },
        },
        "headers": {
            "ExitCode": {
                "schema": { "type": "integer" },
                "description": pick(zh, "Exit code of the command", "命令的退出码"),
            },
            "ErrorCode": {
                "schema": { "type": "string" },
                "description": pick(zh, "`error.code` of the answer", "回复里的 `error.code`"),
            },
        },
        "schemas": {
            "Error": {
                "type": "object",
                "required": ["code", "message"],
                "properties": {
                    "code": {
                        "type": "string",
                        "examples": [
                            "invalid_arguments", "unauthorized", "invalid_host", "not_found",
                            "method_not_allowed", "config_error", "busy", "user_interrupt",
                            "runtime_error", "native_error", "unsupported",
                        ],
                    },
                    "message": { "type": "string" },
                    "details": {},
                },
            },
            "Failure": {
                "type": "object",
                "required": ["success", "error"],
                "properties": {
                    "success": { "const": false },
                    "error": { "$ref": "#/components/schemas/Error" },
                },
            },
            "Result": {
                "oneOf": [success_of(json!({})), { "$ref": "#/components/schemas/Failure" }],
            },
            "FileResult": {
                "type": "object",
                "required": ["result", "file"],
                "properties": {
                    "result": success_of(json!({
                        "type": "object",
                        "required": ["path"],
                        "properties": { "path": { "type": "string" } },
                    })),
                    "file": { "type": "string", "contentMediaType": "application/octet-stream" },
                },
            },
            "Job": {
                "type": "object",
                "required": ["id", "status", "command", "createdAt"],
                "properties": {
                    "id": { "type": "string" },
                    "status": {
                        "type": "string",
                        "enum": ["queued", "running", "succeeded", "failed", "cancelled"],
                    },
                    "command": { "type": "array", "items": { "type": "string" } },
                    "createdAt": { "type": "integer", "description": pick(zh, "Unix time in ms", "Unix 时间（毫秒）") },
                    "startedAt": { "type": ["integer", "null"] },
                    "finishedAt": { "type": ["integer", "null"] },
                    "exitCode": { "type": ["integer", "null"] },
                    "httpStatus": {
                        "type": ["integer", "null"],
                        "description": pick(
                            zh,
                            "The status a synchronous request would have got",
                            "同步请求会得到的 HTTP 状态码",
                        ),
                    },
                    "progress": { "description": pick(zh, "The last `progress` event", "最近一次 `progress` 事件") },
                    "result": {
                        "$ref": "#/components/schemas/Result",
                        "description": pick(
                            zh,
                            "The command's `--json` document; only in `GET /jobs/{id}`",
                            "命令的 `--json` 文档；只出现在 `GET /jobs/{id}` 里",
                        ),
                    },
                },
            },
        },
        "responses": {
            "Error": {
                "description": pick(
                    zh,
                    "An error, in the shape of a failed `--json` command",
                    "错误，格式与失败的 `--json` 命令相同",
                ),
                "headers": { "X-WeFlow-Error-Code": { "$ref": "#/components/headers/ErrorCode" } },
                "content": json_content(json!({ "$ref": "#/components/schemas/Failure" })),
            },
            "Accepted": {
                "description": pick(
                    zh,
                    "Started as a job (`Prefer: respond-async`); follow `Location`",
                    "已作为作业启动（`Prefer: respond-async`）；查看 `Location`",
                ),
                "headers": { "Location": { "schema": { "type": "string" } } },
                "content": json_content(success_of(json!({ "$ref": "#/components/schemas/Job" }))),
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_command_is_described() {
        for zh in [false, true] {
            let doc = document(zh);
            let paths = doc["paths"].as_object().unwrap();
            for (name, ep) in &super::super::Endpoints::new().0 {
                let item = &paths[&format!("/{}", ep.path.join("/"))];
                for verb in ep.verbs() {
                    let op = &item[verb_name(verb).to_ascii_lowercase()];
                    assert_eq!(op["x-weflow-command"], name.as_str(), "{name} {verb:?}");
                }
            }
            let mut ids = std::collections::HashSet::new();
            for op in paths
                .values()
                .flat_map(|item| item.as_object().unwrap().values())
            {
                if let Some(id) = op["operationId"].as_str() {
                    assert!(ids.insert(id), "operationId {id} is used twice");
                }
            }
        }
    }

    #[test]
    fn arguments_become_parameters() {
        let doc = document(false);
        let op = &doc["paths"]["/chat/messages"]["get"];
        let params = op["parameters"].as_array().unwrap();
        let session = params.iter().find(|p| p["name"] == "session-id").unwrap();
        assert_eq!(session["required"], true);
        assert_eq!(session["in"], "query");
        let limit = params.iter().find(|p| p["name"] == "limit").unwrap();
        assert_eq!(limit["schema"]["type"], "integer");

        // POST takes a JSON body, and file commands may ask for the file
        let op = &doc["paths"]["/backup/create"]["post"];
        assert!(
            op["requestBody"]["content"]["application/json"]["schema"]["properties"]["out"]
                .is_object()
        );
        assert!(op["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["$ref"] == "#/components/parameters/return"));
        assert!(op["responses"]["200"]["content"]["multipart/form-data"].is_object());

        // a method chosen by an argument: one operation per method
        let item = &doc["paths"]["/analytics/excluded"];
        assert!(!item["get"]["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "set"));
        assert_eq!(
            item["put"]["requestBody"]["content"]["application/json"]["schema"]["required"],
            json!(["set"])
        );
        let item = &doc["paths"]["/sns/block-delete"];
        let action = |m: &str| {
            item[m]["parameters"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["name"] == "action")
                .map(|p| p["schema"]["enum"].clone())
                .unwrap_or_else(|| {
                    item[m]["requestBody"]["content"]["application/json"]["schema"]["properties"]
                        ["action"]["enum"]
                        .clone()
                })
        };
        assert_eq!(action("get"), json!(["check"]));
        assert_eq!(action("put"), json!(["install"]));
        assert_eq!(action("delete"), json!(["uninstall"]));
    }

    #[test]
    fn in_process_reads_have_their_own_routes() {
        let doc = document(false);
        for name in in_process::COMMANDS {
            let plain = &doc["paths"][format!("/{}", name.replace(' ', "/"))]["get"];
            let own = &doc["paths"][format!("/api/{}", name.replace(' ', "/"))]["get"];
            assert_eq!(
                own["operationId"],
                format!("api_{}", plain["operationId"].as_str().unwrap())
            );
            assert_eq!(own["x-weflow-command"], *name);
            // of the parameters of every route, only `fields`
            let shared: Vec<&Value> = own["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|p| p.get("$ref"))
                .collect();
            assert_eq!(shared, ["#/components/parameters/fields"]);
            assert!(own["responses"]["202"].is_null());
        }
        assert!(doc["paths"]["/api/export/sessions"].is_null());
    }

    #[test]
    fn media_routes_answer_files() {
        let doc = document(false);
        for (name, mime) in [
            ("chat/image-data", "image/*"),
            ("chat/voice-data", "audio/wav"),
            ("chat/video-data", "video/mp4"),
        ] {
            let item = &doc["paths"][format!("/api/{name}")];
            assert_eq!(item.as_object().unwrap().len(), 1, "{name}: only GET");
            let op = &item["get"];
            assert!(
                op["responses"]["200"]["content"][mime].is_object(),
                "{name}"
            );
            assert!(op["responses"]["304"].is_object(), "{name}");
            let params: Vec<&str> = op["parameters"]
                .as_array()
                .unwrap()
                .iter()
                .map(|p| p["name"].as_str().unwrap())
                .collect();
            assert!(
                params.contains(&"session-id") && params.contains(&"msg-id"),
                "{name}: {params:?}"
            );
            assert!(!params.contains(&"out"), "{name}");
            // the route without /api/ is unchanged
            assert!(
                doc["paths"][format!("/{name}")]["post"].is_object(),
                "{name}"
            );
        }
    }

    #[test]
    fn help_follows_the_language() {
        let en = document(false);
        let zh = document(true);
        let summary = |d: &Value| d["paths"]["/chat/sessions"]["get"]["summary"].clone();
        assert_ne!(summary(&en), summary(&zh));
        assert!(summary(&zh)
            .as_str()
            .unwrap()
            .chars()
            .any(|c| ('\u{4e00}'..='\u{9fff}').contains(&c)));
    }
}
