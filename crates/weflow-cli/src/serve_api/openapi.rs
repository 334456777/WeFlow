//! `GET /openapi.json`: an OpenAPI 3.1 description of the API, built from the command tree and [`METHODS`] the
//! same way the routes are, so it always matches the running binary. Help texts are in the server's language, as
//! every answer is (#433).

use std::any::TypeId;

use clap::builder::StyledStr;
use clap::{Arg, Command, CommandFactory};
use serde_json::{json, Map, Value};

use super::{
    api_args, files, in_process, param_of, refused, Kind, Rule, Verb, API_VERSION, HIDDEN, METHODS,
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
/// does (`std`, `return=file`, jobs).
fn in_process_item(item: &Value, name: &str, zh: bool) -> Value {
    let mut item = item.clone();
    for op in item.as_object_mut().unwrap().values_mut() {
        op["operationId"] = format!("api_{}", op["operationId"].as_str().unwrap()).into();
        let mut note = pick(
            zh,
            "Runs in the server's process on databases it keeps open: much faster than the route without \
             `/api/`, with the same parameters and answer.",
            "在服务进程内执行，数据库保持打开：比不带 `/api/` 的路由快得多，参数和回复相同。",
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
        Some("video-data") | Some("media") => json!({ "video/mp4": {}, "image/*": {} }),
        _ => json!({ "image/*": {} }),
    };
    let what = match path.last().map(String::as_str) {
        Some("voice-data") => pick(zh, "The voice as WAV", "WAV 格式的语音"),
        Some("video-data") => pick(
            zh,
            "The video as WeChat keeps it; with `thumb=true`, its thumbnail or cover",
            "微信保存的视频；带 `thumb=true` 时为缩略图或封面",
        ),
        Some("media") => pick(
            zh,
            "The image or video of a Moments post, fetched from WeChat's media servers (https only) and decrypted \
             with `key`; kept in the cache after the first time",
            "朋友圈的图片或视频：从微信的媒体服务器取回（只接受 https 地址），用 `key` 解密；第一次之后从缓存读取",
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
    for common in ["std", "fields", "Prefer"] {
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
        "responses": command_responses(&name, file, zh),
    });
    let description: Vec<String> = [text(cmd.get_long_about())]
        .into_iter()
        .chain(note.map(str::to_string))
        .filter(|s| !s.is_empty())
        .collect();
    if !description.is_empty() {
        op["description"] = description.join("\n\n").into();
    }
    if refused(&name, verb) {
        op["x-weflow-refused"] = true.into();
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

fn command_responses(name: &str, file: bool, zh: bool) -> Value {
    // the contract (#399) says what `data` holds for the commands it describes
    let schema = match contract().data.get(name) {
        Some(data) => {
            json!({ "oneOf": [success_of(data.clone()), { "$ref": "#/components/schemas/Failure" }] })
        }
        None => json!({ "$ref": "#/components/schemas/Result" }),
    };
    let mut ok = json!({
        "description": pick(
            zh,
            "The command's `--json` document (`std` asks for other shapes)",
            "命令的 `--json` 文档（`std` 可以要求其他形式）",
        ),
        "headers": { "X-WeFlow-Exit-Code": { "$ref": "#/components/headers/ExitCode" } },
        "content": { "application/json": { "schema": schema } },
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

/// The contract (#399): what `data` holds for the commands it describes, and the types those schemas refer to, for
/// `components/schemas`. The types are those of `weflow_core::contract`, which the commands answer with.
pub(super) struct Contract {
    pub data: std::collections::BTreeMap<&'static str, Value>,
    pub schemas: Map<String, Value>,
}

pub(super) fn contract() -> &'static Contract {
    static CONTRACT: std::sync::OnceLock<Contract> = std::sync::OnceLock::new();
    CONTRACT.get_or_init(|| {
        use schemars::generate::SchemaSettings;
        use weflow_core::contract::{
            AnnualReport, BizMessages, CacheUsage, ChatStatus, Contact, DbAccounts, DbCandidates, DbConnection, DualReport, ExportStats, Footprint, ContactRank, ExcludeCandidate, GroupChat, GroupHours, GroupMedia, GroupMembers, GroupRank, ImageInfo, InsightCleared, InsightLookup, InsightMarked, InsightRecords, InsightReply, InsightScan, InsightTodayStat,
            MemberAnalytics, MemberMessages, MessageLookup, OverallStatistics, ParsedMessage, PayRecords, ProfileSummary, ReportYears, RuntimeInfo,
            ResourcePage, Session, SnsStats, SnsTimeline, SnsUsers, TransferNames, UserPosts,
            SessionDateCounts, SessionDates, SessionDetail, SessionFacts, SessionTables, TabCounts,
            TimeDistribution,
        };
        let mut generator = SchemaSettings::draft2020_12()
            .for_serialize()
            .with(|s| s.definitions_path = "/components/schemas".into())
            .into_generator();
        let of = |schema: schemars::Schema| schema.to_value();
        let rows = json!({ "type": "array", "items": { "$ref": "#/components/schemas/Row" } });
        let parsed_or_rows = |parsed: Value, what: &str| {
            json!({
                "description": format!("With `parsed=true`, {what} as the chat window shows them; else the rows of WeChat's table, each with its `_cursor`"),
                "anyOf": [parsed, rows.clone()],
            })
        };
        let mut data = std::collections::BTreeMap::new();
        data.insert("chat sessions", of(generator.subschema_for::<Vec<Session>>()));
        let messages = of(generator.subschema_for::<Vec<ParsedMessage>>());
        data.insert("chat messages", parsed_or_rows(messages.clone(), "the messages"));
        data.insert("chat search", parsed_or_rows(messages, "the hits, each with its `_session_id`,"));
        data.insert("chat latest", rows.clone());
        data.insert(
            "chat message",
            json!({
                "description": "With `parsed=true`, the message as the chat window shows it; else its row and the message as the query commands list it",
                "anyOf": [
                    of(generator.subschema_for::<ParsedMessage>()),
                    of(generator.subschema_for::<MessageLookup>()),
                ],
            }),
        );
        let contacts = of(generator.subschema_for::<Vec<Contact>>());
        data.insert(
            "chat contacts",
            json!({
                "description": "With `parsed=true`, the contacts as the desktop app lists them; else the rows of WeChat's table",
                "anyOf": [contacts, rows],
            }),
        );
        data.insert("chat resources", of(generator.subschema_for::<ResourcePage>()));
        // a map of the ids asked for
        let by_id = |value: Value, what: &str| {
            json!({ "type": "object", "description": what, "additionalProperties": value })
        };
        data.insert(
            "chat statuses",
            by_id(
                of(generator.subschema_for::<ChatStatus>()),
                "Each username asked for, with whether its conversation is folded and muted",
            ),
        );
        data.insert(
            "chat counts",
            by_id(
                json!({ "type": "integer" }),
                "Each conversation asked for, with how many messages it has",
            ),
        );
        data.insert("chat tab-counts", of(generator.subschema_for::<TabCounts>()));
        data.insert("chat dates", of(generator.subschema_for::<SessionDates>()));
        data.insert("chat date-counts", of(generator.subschema_for::<SessionDateCounts>()));
        data.insert(
            "chat detail",
            json!({
                "description": "The conversation's details; with `fast`, only its facts; with `extra` alone, only where its messages are kept",
                "anyOf": [
                    of(generator.subschema_for::<SessionDetail>()),
                    of(generator.subschema_for::<SessionFacts>()),
                    of(generator.subschema_for::<SessionTables>()),
                ],
            }),
        );
        data.insert("chat image-info", of(generator.subschema_for::<ImageInfo>()));
        data.insert("group list", of(generator.subschema_for::<Vec<GroupChat>>()));
        data.insert("group members", of(generator.subschema_for::<GroupMembers>()));
        data.insert("group ranking", of(generator.subschema_for::<Vec<GroupRank>>()));
        data.insert("group hours", of(generator.subschema_for::<GroupHours>()));
        data.insert("group media", of(generator.subschema_for::<GroupMedia>()));
        data.insert("group member", of(generator.subschema_for::<MemberAnalytics>()));
        data.insert("group member-messages", of(generator.subschema_for::<MemberMessages>()));
        data.insert("analytics overall", of(generator.subschema_for::<OverallStatistics>()));
        data.insert("analytics rankings", of(generator.subschema_for::<Vec<ContactRank>>()));
        data.insert("analytics time", of(generator.subschema_for::<TimeDistribution>()));
        data.insert(
            "analytics excluded",
            json!({
                "description": "The usernames left out of the analytics, as saved",
                "type": "array",
                "items": { "type": "string" },
            }),
        );
        data.insert(
            "analytics exclude-candidates",
            of(generator.subschema_for::<Vec<ExcludeCandidate>>()),
        );
        data.insert("sns timeline", of(generator.subschema_for::<SnsTimeline>()));
        data.insert("sns users", of(generator.subschema_for::<SnsUsers>()));
        data.insert("sns stats", of(generator.subschema_for::<SnsStats>()));
        data.insert(
            "sns post-counts",
            json!({
                "description": "How many posts each poster has, by username; with `user`, that one's",
                "anyOf": [
                    by_id(json!({ "type": "integer" }), "Each poster, with how many posts they have"),
                    of(generator.subschema_for::<UserPosts>()),
                ],
            }),
        );
        data.insert("report annual years", of(generator.subschema_for::<ReportYears>()));
        data.insert("report annual generate", of(generator.subschema_for::<AnnualReport>()));
        data.insert("report dual generate", of(generator.subschema_for::<DualReport>()));
        data.insert(
            "biz accounts",
            json!({
                "type": "object",
                "description": "The official accounts followed: the rows of WeChat's contact table, each with `lastTime`, the time of its conversation's last message (null without one)",
                "required": ["accounts"],
                "properties": { "accounts": rows.clone() },
            }),
        );
        data.insert("biz messages", of(generator.subschema_for::<BizMessages>()));
        data.insert("biz pay-records", of(generator.subschema_for::<PayRecords>()));
        data.insert("config profiles", of(generator.subschema_for::<Vec<ProfileSummary>>()));
        data.insert("runtime info", of(generator.subschema_for::<RuntimeInfo>()));
        data.insert("cache list", of(generator.subschema_for::<Vec<CacheUsage>>()));
        data.insert("db detect", of(generator.subschema_for::<DbCandidates>()));
        data.insert("db wxid", of(generator.subschema_for::<DbAccounts>()));
        data.insert("db test", of(generator.subschema_for::<DbConnection>()));
        data.insert("insight footprint", of(generator.subschema_for::<Footprint>()));
        data.insert("chat export-stats", of(generator.subschema_for::<ExportStats>()));
        data.insert("chat transfer-names", of(generator.subschema_for::<TransferNames>()));
        data.insert("insight get", of(generator.subschema_for::<InsightLookup>()));
        data.insert("insight records", of(generator.subschema_for::<InsightRecords>()));
        data.insert("insight today-stats", of(generator.subschema_for::<Vec<InsightTodayStat>>()));
        data.insert("insight mark-read", of(generator.subschema_for::<InsightMarked>()));
        data.insert("insight clear", of(generator.subschema_for::<InsightCleared>()));
        data.insert("insight scan", of(generator.subschema_for::<InsightScan>()));
        for command in ["insight test", "insight trigger", "insight footprint-summary"] {
            data.insert(command, of(generator.subschema_for::<InsightReply>()));
        }
        // referred to by the server's own routes and by Failure
        generator.subschema_for::<super::jobs::JobView>();
        generator.subschema_for::<weflow_core::error::ErrorPayload>();
        let mut schemas = generator.take_definitions(true);
        for schema in schemas.values_mut() {
            left_out_not_null(schema);
        }
        schemas.insert(
            "Row".into(),
            json!({
                "type": "object",
                "description": "A row of one of WeChat's tables, with its columns; not described further, as WeChat decides them",
            }),
        );
        Contract { data, schemas }
    })
}

/// An optional field of the contract is left out when it has no value (`skip_serializing_if`), never `null`; schemars
/// writes the `null` of its `Option` all the same, which is taken out here.
fn left_out_not_null(schema: &mut Value) {
    let required: Vec<String> = schema["required"]
        .as_array()
        .map(|r| {
            r.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default();
    let Some(properties) = schema.get_mut("properties").and_then(Value::as_object_mut) else {
        return;
    };
    for (name, property) in properties.iter_mut() {
        if required.contains(name) {
            continue;
        }
        if let Some(kinds) = property.get_mut("type").and_then(Value::as_array_mut) {
            kinds.retain(|k| k != "null");
            if let [one] = kinds.as_slice() {
                property["type"] = one.clone();
            }
        }
        if let Some(any) = property.get_mut("anyOf").and_then(Value::as_array_mut) {
            any.retain(|s| s != &json!({ "type": "null" }));
            if let [one] = any.as_slice() {
                let one = one.clone();
                let object = property.as_object_mut().unwrap();
                object.remove("anyOf");
                for (k, v) in one.as_object().into_iter().flatten() {
                    object.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// The contract alone (#399), as `web/contract/openapi.json` keeps it for the WebUI's types: the `/api/` routes it
/// describes and their types. It has no help texts nor anything of the build, so it changes only with the contract.
#[cfg(test)]
fn contract_document() -> Value {
    let c = contract();
    let mut paths = Map::new();
    for (name, data) in &c.data {
        let path = name.replace(' ', "/");
        let route = if in_process::COMMANDS.contains(name) {
            format!("/api/{path}")
        } else {
            format!("/{path}")
        };
        paths.insert(
            route,
            json!({ "get": {
                "operationId": format!("api_{}", name.replace([' ', '-'], "_")),
                "x-weflow-command": name,
                "responses": {
                    "200": { "description": "Its `--json` document", "content": json_content(success_of(data.clone())) },
                    "default": {
                        "description": "A failure",
                        "content": json_content(json!({ "$ref": "#/components/schemas/Failure" })),
                    },
                },
            } }),
        );
    }
    paths.insert(
        "/jobs/{id}".into(),
        json!({ "get": {
            "operationId": "job",
            "parameters": [{ "name": "id", "in": "path", "required": true, "schema": { "type": "string" } }],
            "responses": {
                "200": { "description": "The job", "content": json_content(success_of(json!({ "$ref": "#/components/schemas/Job" }))) },
            },
        } }),
    );
    let mut schemas = c.schemas.clone();
    schemas.insert(
        "Failure".into(),
        components(false)["schemas"]["Failure"].clone(),
    );
    json!({
        "openapi": "3.1.0",
        "info": {
            "title": "WeFlow command API contract",
            "version": API_VERSION,
            "description": "The types of the answers of the command API that the WeFlow WebUI reads (#399); `/openapi.json` of `weflow serve` describes every route. Generated by a test of weflow-cli, see docs/serve-api.md.",
        },
        "paths": paths,
        "components": { "schemas": schemas },
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
                            "limits": {
                                "type": "object",
                                "description": pick(
                                    zh,
                                    "What may run at once: --max-tasks, --max-reads, --max-media",
                                    "可同时运行的数量：--max-tasks、--max-reads、--max-media",
                                ),
                                "properties": {
                                    "maxTasks": { "type": "integer" },
                                    "maxReads": { "type": "integer" },
                                    "maxMedia": { "type": "integer" },
                                },
                            },
                            "insight": {
                                "type": "boolean",
                                "description": pick(zh, "Whether the insight engine runs beside (--insight)", "是否同时运行灵感引擎（--insight）"),
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
        "/jobs/{id}/file".into(),
        json!({ "get": {
            "operationId": "jobs_file",
            "tags": ["server"],
            "summary": pick(
                zh,
                "The file a job started with `return=file` wrote, to be saved",
                "以 `return=file` 启动的作业写出的文件，以附件形式下载",
            ),
            "parameters": [job_id],
            "responses": {
                "200": {
                    "description": pick(
                        zh,
                        "The file, with `Content-Disposition: attachment`",
                        "文件本身，带 `Content-Disposition: attachment`",
                    ),
                    "content": { "application/octet-stream": { "schema": { "type": "string", "format": "binary" } } },
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
    let mut components = json!({
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
    });
    // the types of the contract, Error and Job among them
    let schemas = components["schemas"].as_object_mut().unwrap();
    schemas.extend(contract().schemas.clone());
    components
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

    /// `web/contract/openapi.json` is the contract the code answers with (#399), from which the WebUI's types are
    /// generated.
    #[test]
    fn contract_snapshot_is_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/contract/openapi.json");
        let now = serde_json::to_string_pretty(&contract_document()).unwrap() + "\n";
        if std::env::var_os("WEFLOW_UPDATE_CONTRACT").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, now).unwrap();
            return;
        }
        // a checkout may have turned its line ends into CRLF
        let kept = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            kept == now,
            "web/contract/openapi.json is not the contract the code answers with; rewrite it with \
             `WEFLOW_UPDATE_CONTRACT=1 cargo test -p weflow-cli --bin weflow contract_`, then \
             `bun run contract` in web/"
        );
    }

    /// One answer or more of each type of the contract, from fixed rows: what `web/contract/samples.json` keeps for
    /// the WebUI's tests (#400), which hand them to the page's own code. Each says something a client may get wrong:
    /// an id past 2^53 or past i64, a row that does not say who sent it, a day that turns at midnight, a page that
    /// ends or goes on, an optional field left out, a job in each state.
    fn samples() -> Value {
        use weflow_core::contract::*;
        use weflow_native::native_msg::MsgPos;

        let rows = [
            json!({ "local_id": 1, "server_id": "9007199254740993", "create_time": 1700000000, "local_type": 1,
                "sort_seq": 1700000000000i64, "is_send": 0, "sender_username": "wxid_bob", "message_content": "hello",
                "db_name": "message_0" }),
            json!({ "local_id": 2, "server_id": "18446744073709551615", "create_time": 1700000060, "local_type": 49,
                "sort_seq": 1700000060000i64, "is_send": 1, "message_content":
                "<msg><appmsg><type>6</type><title>report.pdf</title><appattach><totallen>2048</totallen>\
                 <fileext>pdf</fileext></appattach></appmsg></msg>", "db_name": "message_0" }),
            json!({ "local_id": 3, "server_id": "0", "create_time": 1700000120, "local_type": 49,
                "sort_seq": 1700000120000i64, "is_send": 0, "sender_username": "wxid_bob", "message_content":
                "<msg><appmsg><type>57</type><title>yes</title><refermsg><type>1</type>\
                 <displayname>Me</displayname><content>are you there?</content></refermsg></appmsg></msg>",
                "db_name": "message_0" }),
            // the last second of 2023 in UTC, and a row that says neither who sent it nor whether I did
            json!({ "local_id": 4, "server_id": "0", "create_time": 1704067199, "local_type": 10000,
                "sort_seq": 1704067199000i64, "message_content": "\"Bob\" 撤回了一条消息", "db_name": "message_1" }),
            // the next second, in another database, with the same sort_seq as the row before
            json!({ "local_id": 1, "server_id": "9007199254740995", "create_time": 1704067200, "local_type": 3,
                "sort_seq": 1704067199000i64, "is_send": 1, "sender_username": "wxid_me", "message_content":
                "<msg><img md5=\"aabbccddeeff00112233445566778899\" /></msg>", "db_name": "message_2" }),
        ];
        let cursor = |row: &Value| MsgPos::of_row(row).unwrap().token();
        let parsed: Vec<ParsedMessage> = weflow_core::chat_msg::map_rows(&rows, "wxid_me")
            .into_iter()
            .zip(&rows)
            .enumerate()
            .map(|(i, (message, row))| ParsedMessage {
                message,
                cursor: Some(cursor(row)),
                // the first is a hit of `chat search`
                session_id: (i == 0).then(|| "wxid_bob".to_string()),
            })
            .collect();

        let session = Session {
            username: "wxid_bob".into(),
            kind: 1,
            unread_count: 2,
            summary: "hello".into(),
            sort_timestamp: 1700000120,
            last_timestamp: 1700000120,
            last_msg_type: 1,
            message_count_hint: Some(3),
            display_name: "Bob".into(),
            self_wxid: "wxid_me".into(),
            avatar_url: Some("https://example.com/bob.png".into()),
            ..Session::default()
        };
        let group = Session {
            username: "room1@chatroom".into(),
            kind: 2,
            summary: "ok".into(),
            sort_timestamp: 1704067200,
            last_timestamp: 1704067200,
            last_msg_type: 1,
            display_name: "Project Room".into(),
            last_msg_sender: Some("wxid_bob".into()),
            last_sender_display_name: Some("Bob in room".into()),
            self_wxid: "wxid_me".into(),
            ..Session::default()
        };
        // an official account WeChat lists without a conversation
        let official = Session {
            username: "gh_news".into(),
            summary: "查看公众号历史消息".into(),
            display_name: "News".into(),
            self_wxid: "wxid_me".into(),
            ..Session::default()
        };

        let contact = |username: &str, display_name: &str, kind| Contact {
            username: username.into(),
            display_name: display_name.into(),
            remark: None,
            nickname: None,
            alias: None,
            labels: None,
            detail_description: None,
            region: None,
            kind,
            avatar_url: None,
        };
        let contacts = [
            Contact {
                remark: Some("Bobby".into()),
                nickname: Some("Bob".into()),
                alias: Some("bobby_id".into()),
                labels: Some(vec!["Family".into()]),
                region: Some("广东 深圳".into()),
                ..contact("wxid_bob", "Bobby", ContactKind::Friend)
            },
            contact("room1@chatroom", "Project Room", ContactKind::Group),
            contact("wxid_gone", "wxid_gone", ContactKind::FormerFriend),
        ];

        let image = weflow_core::message::ExportMsg {
            local_id: 4,
            server_id_raw: Some("9007199254740995".into()),
            create_time: 1700000180,
            local_type: 3,
            content: "<msg><img md5=\"aabbccddeeff00112233445566778899\" /></msg>".into(),
            sender_username: "wxid_bob".into(),
            image_md5: Some("aabbccddeeff00112233445566778899".into()),
            ..Default::default()
        };
        let file = weflow_core::message::ExportMsg {
            local_id: 2,
            server_id_raw: Some("18446744073709551615".into()),
            create_time: 1700000060,
            local_type: 49,
            content: rows[1]["message_content"].as_str().unwrap().into(),
            sender_username: "wxid_me".into(),
            is_send: true,
            file_name: Some("report.pdf".into()),
            file_ext: Some("pdf".into()),
            file_size: Some(2048),
            ..Default::default()
        };
        let resource = |m: &weflow_core::message::ExportMsg, sort_seq: i64, kind| Resource {
            message: MessageView::of(m, "wxid_me"),
            cursor: format!(
                "{}:wxid_bob",
                MsgPos {
                    sort_seq,
                    create_time: m.create_time,
                    local_id: m.local_id,
                    shard: 0,
                }
                .token()
            ),
            session_display_name: "Bob".into(),
            session_id: "wxid_bob".into(),
            resource_type: kind,
        };
        let newest = resource(&image, 1700000180000, ResourceKind::Image);
        let next_cursor = Some(newest.cursor.clone());
        let pages = [
            // the first page of one, more to come
            ResourcePage {
                items: vec![newest],
                has_more: true,
                next_cursor,
            },
            // the last page
            ResourcePage {
                items: vec![resource(&file, 1700000060000, ResourceKind::File)],
                has_more: false,
                next_cursor: None,
            },
            ResourcePage {
                items: vec![],
                has_more: false,
                next_cursor: None,
            },
        ];
        let lookups = [
            MessageLookup {
                row: rows[0].clone(),
                message: Some(MessageView::of(&image, "wxid_me")),
            },
            MessageLookup {
                row: json!({ "local_id": 9, "local_type": 1 }),
                message: None,
            },
        ];

        let error = |code: &str, message: &str, details| weflow_core::error::ErrorPayload {
            code: code.into(),
            message: message.into(),
            details,
        };
        let errors = [
            error(
                "invalid_arguments",
                "`chat messages` needs `session-id`",
                Some(json!({ "usage": "weflow chat messages --session-id <SESSION_ID>" })),
            ),
            error(
                "runtime_error",
                "the video has no thumbnail or cover",
                Some(json!({ "failureKind": "not_found" })),
            ),
            error("busy", "too many reads at once", None),
        ];

        use super::super::jobs::{JobFile, JobView, Status};
        let job = |id: &str, status| JobView {
            id: id.into(),
            status,
            command: [
                "weflow",
                "--json",
                "--progress",
                "export",
                "messages",
                "--session-id",
                "wxid_bob",
            ]
            .map(String::from)
            .into(),
            // milliseconds, unlike the times of messages
            created_at: 1700000000000,
            started_at: None,
            finished_at: None,
            exit_code: None,
            http_status: None,
            progress: None,
            file: None,
            result: None,
        };
        let jobs = [
            job("a1b2c3d4e5f60001", Status::Queued),
            JobView {
                started_at: Some(1700000000500),
                progress: Some(
                    json!({ "type": "progress", "stage": "messages", "current": 1, "total": 3 }),
                ),
                ..job("a1b2c3d4e5f60002", Status::Running)
            },
            JobView {
                started_at: Some(1700000000500),
                finished_at: Some(1700000003000),
                exit_code: Some(0),
                http_status: Some(200),
                progress: Some(
                    json!({ "type": "progress", "stage": "messages", "current": 3, "total": 3 }),
                ),
                // a path without a file name
                file: Some(JobFile {
                    name: None,
                    size: Some(5120),
                }),
                result: Some(json!({ "success": true, "data": { "count": 3 } })),
                ..job("a1b2c3d4e5f60003", Status::Succeeded)
            },
            JobView {
                started_at: Some(1700000000500),
                finished_at: Some(1700000001000),
                exit_code: Some(4),
                http_status: Some(501),
                result: Some(json!({ "success": false, "error": errors[2] })),
                ..job("a1b2c3d4e5f60004", Status::Failed)
            },
            // cancelled while it waited for a place
            JobView {
                finished_at: Some(1700000000200),
                http_status: Some(499),
                result: Some(super::super::jobs::cancelled_result()),
                ..job("a1b2c3d4e5f60005", Status::Cancelled)
            },
        ];

        json!({
            "ParsedMessage": parsed,
            "Session": [session, group, official],
            "Contact": contacts,
            "ResourcePage": pages,
            "MessageLookup": lookups,
            "Error": errors,
            "Job": jobs,
        })
    }

    /// `web/contract/samples.json` is what [`samples`] answers. Rewrite it with
    /// `WEFLOW_UPDATE_CONTRACT=1 cargo test -p weflow-cli --bin weflow contract_`.
    #[test]
    fn contract_samples_are_current() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../web/contract/samples.json");
        let now = serde_json::to_string_pretty(&samples()).unwrap() + "\n";
        if std::env::var_os("WEFLOW_UPDATE_CONTRACT").is_some() {
            std::fs::write(&path, now).unwrap();
            return;
        }
        let kept = std::fs::read_to_string(&path)
            .unwrap_or_default()
            .replace("\r\n", "\n");
        assert!(
            kept == now,
            "web/contract/samples.json is not what the contract's types answer; rewrite it with \
             `WEFLOW_UPDATE_CONTRACT=1 cargo test -p weflow-cli --bin weflow contract_`"
        );
    }

    /// The errors of `value` against the schema `name` of the contract's components, as `path: error`.
    fn misfits(name: &str, value: &Value) -> Vec<String> {
        let root = json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "$ref": format!("#/components/schemas/{name}"),
            "components": contract_document()["components"],
        });
        let validator = jsonschema::validator_for(&root).unwrap();
        validator
            .iter_errors(value)
            .map(|e| format!("{}: {e}", e.instance_path()))
            .collect()
    }

    /// Each sample fits its schema, and breaking what a client relies on makes it not fit: the contract's schemas
    /// say enough to catch a change of the answers (#400).
    #[test]
    fn the_samples_fit_the_contract_and_broken_ones_do_not() {
        let samples = samples();
        for (name, values) in samples.as_object().unwrap() {
            for value in values.as_array().unwrap() {
                assert_eq!(
                    misfits(name, value),
                    Vec::<String>::new(),
                    "{name}: {value}"
                );
            }
        }
        let broken = |name: &str, i: usize, change: &dyn Fn(&mut Value)| {
            let mut value = samples[name][i].clone();
            change(&mut value);
            let errors = misfits(name, &value);
            assert!(!errors.is_empty(), "{name} fits when broken: {value}");
        };
        // an id as a number would lose its last digits in JavaScript
        broken("ParsedMessage", 0, &|v| {
            v["serverId"] = json!(9007199254740993u64)
        });
        // the cursor of a page is what the next page is asked from
        broken("ParsedMessage", 0, &|v| v["_cursor"] = json!(1));
        broken("ParsedMessage", 1, &|v| {
            v.as_object_mut().unwrap().remove("messageKey");
        });
        // an optional field is left out, never null; a nullable one is there
        broken("ParsedMessage", 1, &|v| v["fileName"] = Value::Null);
        broken("ParsedMessage", 3, &|v| {
            v.as_object_mut().unwrap().remove("isSend");
        });
        broken("ParsedMessage", 0, &|v| v["isSend"] = json!(false));
        // times are whole seconds
        broken("ParsedMessage", 0, &|v| {
            v["createTime"] = json!("2023-11-14")
        });
        broken("ParsedMessage", 0, &|v| {
            v["createTime"] = json!(1700000000.5)
        });
        broken("Session", 0, &|v| v["avatarUrl"] = Value::Null);
        broken("Session", 2, &|v| {
            v.as_object_mut().unwrap().remove("sortTimestamp");
        });
        broken("Contact", 0, &|v| v["type"] = json!("stranger"));
        broken("Contact", 0, &|v| v["labels"] = json!("Family"));
        // a page says whether more follow and where they start
        broken("ResourcePage", 1, &|v| {
            v.as_object_mut().unwrap().remove("nextCursor");
        });
        broken("ResourcePage", 0, &|v| v["hasMore"] = json!(1));
        broken("ResourcePage", 0, &|v| {
            v["items"][0]["resourceType"] = json!("emoji")
        });
        broken("ResourcePage", 0, &|v| {
            v["items"][0].as_object_mut().unwrap().remove("_cursor");
        });
        broken("ResourcePage", 0, &|v| {
            v["items"][0]["serverId"] = json!(9007199254740995u64)
        });
        broken("MessageLookup", 1, &|v| {
            v.as_object_mut().unwrap().remove("message");
        });
        broken("Error", 0, &|v| {
            v.as_object_mut().unwrap().remove("code");
        });
        broken("Job", 0, &|v| v["status"] = json!("done"));
        broken("Job", 0, &|v| {
            v.as_object_mut().unwrap().remove("file");
        });
        broken("Job", 2, &|v| v["file"]["size"] = json!(-1));
        broken("Job", 2, &|v| v["finishedAt"] = json!("1700000003000"));
    }

    /// The WebUI asks for the API the server answers with, and its types are those of this API's contract.
    #[test]
    fn the_webui_and_the_contract_have_the_server_s_api_version() {
        let text = |path: &str| {
            std::fs::read_to_string(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(path))
                .unwrap()
        };
        let package: Value = serde_json::from_str(&text("../../web/package.json")).unwrap();
        assert_eq!(package["weflow"]["apiVersion"], API_VERSION);
        assert_eq!(contract_document()["info"]["version"], API_VERSION);
    }

    #[test]
    fn the_answers_of_the_contract_are_described() {
        let doc = document(false);
        let ok = |path: &str| {
            doc["paths"][path]["get"]["responses"]["200"]["content"]["application/json"]["schema"]
                .clone()
        };
        for path in ["/chat/sessions", "/api/chat/sessions"] {
            assert_eq!(
                ok(path)["oneOf"][0]["properties"]["data"]["items"]["$ref"],
                "#/components/schemas/Session",
                "{path}"
            );
        }
        let schemas = &doc["components"]["schemas"];
        for name in [
            "Session",
            "ParsedMessage",
            "Contact",
            "ResourcePage",
            "Job",
            "Error",
            "Row",
            "Failure",
        ] {
            assert!(schemas[name].is_object(), "{name}");
        }
        // a big id is a string, a page's cursor nullable
        assert_eq!(
            schemas["ParsedMessage"]["properties"]["serverId"]["type"],
            "string"
        );
        assert!(schemas["ResourcePage"]["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|r| r == "nextCursor"));
        // other commands keep the generic answer
        // `config get` answers the value of any key
        assert_eq!(ok("/config/get")["$ref"], "#/components/schemas/Result");
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
        // the media of a Moments post (#202): its address and key, no `out`
        let op = &doc["paths"]["/api/sns/media"]["get"];
        assert!(op["responses"]["200"]["content"]["video/mp4"].is_object());
        assert!(op["responses"]["200"]["content"]["image/*"].is_object());
        let params: Vec<&str> = op["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| p["name"].as_str().unwrap())
            .collect();
        assert!(
            params.contains(&"url") && params.contains(&"key"),
            "{params:?}"
        );
        assert!(!params.contains(&"out"));
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
