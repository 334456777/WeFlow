//! 假服务的 OpenAI 格式：`POST /v1/chat/completions` 和 `GET /v1/models`，检查、回答和错误按 OpenAI 的 OpenAPI
//! 描述（`CreateChatCompletionRequest`、`CreateChatCompletionResponse`、`ErrorResponse`）实现。
use serde_json::{json, Value};

use crate::http::{Request, Response};
use crate::scenario::{self, Behaviour};
use crate::Shared;

/// 状态码为 `status` 的错误：`{"error": {"message", "type", "param", "code"}}`。
fn error(
    status: u16,
    kind: &str,
    code: Option<&str>,
    param: Option<&str>,
    message: &str,
) -> Response {
    Response::json(
        status,
        &json!({ "error": { "message": message, "type": kind, "param": param, "code": code } }),
    )
}

fn bad_request(param: Option<&str>, message: &str) -> Response {
    error(400, "invalid_request_error", None, param, message)
}

/// 检查 `Authorization: Bearer` 里的密钥；密钥不对或没有时返回 OpenAI 的 401。
fn authorize(request: &Request, shared: &Shared) -> Result<(), Response> {
    match request
        .header("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
    {
        Some(key) if key.trim() == shared.key => Ok(()),
        Some(_) => Err(error(
            401,
            "invalid_request_error",
            Some("invalid_api_key"),
            None,
            "Incorrect API key provided. You can find your API key in the fake's output.",
        )),
        None => Err(error(
            401,
            "invalid_request_error",
            None,
            None,
            "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY).",
        )),
    }
}

/// `GET /v1/models`：列出全部模型。
pub fn models(request: &Request, shared: &Shared) -> Response {
    if let Err(denied) = authorize(request, shared) {
        return denied;
    }
    let data: Vec<Value> = scenario::MODELS
        .iter()
        .map(|m| json!({ "id": m.id, "object": "model", "created": 1_767_225_600, "owned_by": "weflow-fake" }))
        .collect();
    Response::json(200, &json!({ "object": "list", "data": data }))
}

/// `POST /v1/chat/completions`：检查请求，再按模型的回答方式回答；回答超出 `max_completion_tokens`（或旧的
/// `max_tokens`）时截断，`finish_reason` 为 `length`。
pub fn chat(request: &Request, shared: &Shared) -> Response {
    if let Err(denied) = authorize(request, shared) {
        return denied;
    }
    let Some(body) = request.json().filter(Value::is_object) else {
        return bad_request(None, "We could not parse the JSON body of your request.");
    };
    let Some(model_id) = body["model"].as_str() else {
        return bad_request(Some("model"), "you must provide a model parameter");
    };
    let Some(model) = scenario::model(model_id) else {
        return error(
            404,
            "invalid_request_error",
            Some("model_not_found"),
            None,
            &format!("The model `{model_id}` does not exist or you do not have access to it."),
        );
    };
    let messages = match body["messages"].as_array() {
        Some(list) if !list.is_empty() => list,
        Some(_) => return bad_request(Some("messages"), "[] is too short - 'messages'"),
        None => return bad_request(Some("messages"), "Missing required parameter: 'messages'."),
    };
    let mut system = String::new();
    let mut user = String::new();
    for (i, message) in messages.iter().enumerate() {
        let text = content_text(&message["content"]);
        match message["role"].as_str() {
            Some("system" | "developer") => system.push_str(&text),
            Some("user") => user = text,
            Some("assistant" | "tool") => {}
            _ => {
                return bad_request(
                    Some(&format!("messages.[{i}].role")),
                    &format!("Invalid value for 'messages[{i}].role'."),
                )
            }
        }
    }
    if body["stream"] == true {
        return bad_request(
            Some("stream"),
            "The fake model does not stream; send stream: false.",
        );
    }
    if model.behaviour == Behaviour::Reason {
        if body.get("max_tokens").is_some() {
            return error(
                400,
                "invalid_request_error",
                Some("unsupported_parameter"),
                Some("max_tokens"),
                "Unsupported parameter: 'max_tokens' is not supported with this model. Use 'max_completion_tokens' instead.",
            );
        }
        if body["temperature"].as_f64().is_some_and(|t| t != 1.0) {
            return error(
                400,
                "invalid_request_error",
                Some("unsupported_value"),
                Some("temperature"),
                "Unsupported value: 'temperature' does not support this value with this model. Only the default (1) value is supported.",
            );
        }
    }
    let n = shared.next();
    let request_id = format!("req_fake{n:06}");
    let answer = match scenario_failure(model.behaviour, shared) {
        Some(failure) => return failure.with_header("x-request-id", request_id),
        None => {
            if model.behaviour == Behaviour::Slow {
                std::thread::sleep(scenario::SLOW_ANSWER);
            }
            scenario::answer(model.behaviour, &system, &user, n)
        }
    };
    let limit = body["max_completion_tokens"]
        .as_u64()
        .or_else(|| body["max_tokens"].as_u64())
        .unwrap_or(u64::MAX);
    let reasoning = if model.behaviour == Behaviour::Reason {
        scenario::REASONING_TOKENS
    } else {
        0
    };
    let room = limit.saturating_sub(reasoning);
    let (content, finish) = if scenario::tokens(&answer) > room {
        let kept: String = answer
            .chars()
            .take(usize::try_from(room).unwrap_or(usize::MAX))
            .collect();
        (kept, "length")
    } else {
        (answer, "stop")
    };
    let (message, finish) = if model.behaviour == Behaviour::Refuse {
        (
            json!({ "role": "assistant", "content": null, "refusal": "I can't help with that.", "annotations": [] }),
            "stop",
        )
    } else {
        (
            json!({ "role": "assistant", "content": content, "refusal": null, "annotations": [] }),
            finish,
        )
    };
    let prompt_tokens = scenario::tokens(&system) + scenario::tokens(&user);
    let completion_tokens =
        reasoning.min(limit) + scenario::tokens(message["content"].as_str().unwrap_or(""));
    Response::json(
        200,
        &json!({
            "id": format!("chatcmpl-fake{n:06}"),
            "object": "chat.completion",
            "created": crate::unix_now(),
            "model": model.id,
            "choices": [{ "index": 0, "message": message, "logprobs": null, "finish_reason": finish }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
                "completion_tokens_details": { "reasoning_tokens": reasoning.min(limit) }
            },
            "system_fingerprint": null,
        }),
    )
    .with_header("x-request-id", request_id)
}

/// 会失败的模型这次失败时返回的错误；这次不失败时为 `None`。
fn scenario_failure(behaviour: Behaviour, shared: &Shared) -> Option<Response> {
    let overloaded = || {
        error(
            503,
            "service_unavailable_error",
            Some("server_is_overloaded"),
            None,
            "The server is overloaded. Please try again later.",
        )
        .with_header("retry-after", "0")
        .with_header("retry-after-ms", "0")
    };
    match behaviour {
        Behaviour::RateLimit => Some(
            error(
                429,
                "requests",
                Some("rate_limit_exceeded"),
                None,
                "Rate limit reached for requests. Please try again in 0s.",
            )
            .with_header("retry-after", "0")
            .with_header("retry-after-ms", "0"),
        ),
        Behaviour::Quota => Some(error(
            429,
            "insufficient_quota",
            Some("insufficient_quota"),
            None,
            "You exceeded your current quota, please check your plan and billing details.",
        )),
        Behaviour::Overloaded => Some(overloaded()),
        Behaviour::Flaky => shared.flaky_fails().then(overloaded),
        Behaviour::ServerError => Some(Response::text(
            500,
            "<html><body>upstream exploded</body></html>",
        )),
        Behaviour::Answer
        | Behaviour::Reason
        | Behaviour::Skip
        | Behaviour::Refuse
        | Behaviour::Slow => None,
    }
}

/// 消息的 `content` 里的文字：字符串本身，或者数组中各部分的 `text`。
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts.iter().filter_map(|p| p["text"].as_str()).collect(),
        _ => String::new(),
    }
}
