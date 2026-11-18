//! Anthropic's side of the fake: `POST /v1/messages` and `GET /v1/models`, with the checks, answers and errors of
//! Anthropic's API reference: `anthropic-version` required, the key in `x-api-key` or `Authorization: Bearer`,
//! `max_tokens` required, alternating turns that end with the user's, no sampling parameters in version 2026-10-21.
use serde_json::{json, Value};

use crate::http::{Request, Response};
use crate::scenario::{self, Behaviour};
use crate::Shared;

/// The API version that refuses `temperature`, `top_p` and `top_k`.
const SAMPLING_REMOVED: &str = "2026-10-21";

/// `{"type": "error", "error": {"type", "message"}, "request_id"}` with `status`.
fn error(status: u16, kind: &str, message: &str, request_id: &str) -> Response {
    Response::json(
        status,
        &json!({ "type": "error", "error": { "type": kind, "message": message }, "request_id": request_id }),
    )
    .with_header("request-id", request_id)
}

/// The version and the key, or the error Anthropic answers without them.
fn admit<'r>(request: &'r Request, shared: &Shared, request_id: &str) -> Result<&'r str, Response> {
    let Some(version) = request.header("anthropic-version") else {
        return Err(error(
            400,
            "invalid_request_error",
            "anthropic-version: header is required",
            request_id,
        ));
    };
    let key = request.header("x-api-key").or_else(|| {
        request
            .header("authorization")
            .and_then(|v| v.strip_prefix("Bearer "))
    });
    match key {
        Some(key) if key.trim() == shared.key => Ok(version),
        _ => Err(error(
            401,
            "authentication_error",
            "invalid x-api-key",
            request_id,
        )),
    }
}

pub fn models(request: &Request, shared: &Shared) -> Response {
    let request_id = format!("req_fake{:06}", shared.next());
    if let Err(denied) = admit(request, shared, &request_id) {
        return denied;
    }
    let data: Vec<Value> = scenario::MODELS
        .iter()
        .map(|m| json!({ "type": "model", "id": m.id, "display_name": m.display_name, "created_at": "2026-01-01T00:00:00Z" }))
        .collect();
    Response::json(
        200,
        &json!({
            "data": data,
            "has_more": false,
            "first_id": scenario::MODELS.first().map(|m| m.id),
            "last_id": scenario::MODELS.last().map(|m| m.id),
        }),
    )
    .with_header("request-id", request_id)
}

pub fn messages(request: &Request, shared: &Shared) -> Response {
    let n = shared.next();
    let request_id = format!("req_fake{n:06}");
    let bad = |message: &str| error(400, "invalid_request_error", message, &request_id);
    let version = match admit(request, shared, &request_id) {
        Ok(version) => version,
        Err(denied) => return denied,
    };
    let Some(body) = request.json().filter(Value::is_object) else {
        return bad("There was an issue with the request body: it is not valid JSON.");
    };
    let Some(model_id) = body["model"].as_str() else {
        return bad("model: Field required");
    };
    let Some(limit) = body["max_tokens"].as_u64() else {
        return bad("max_tokens: Field required");
    };
    if version >= SAMPLING_REMOVED {
        for field in ["temperature", "top_p", "top_k"] {
            if body.get(field).is_some() {
                return bad(&format!(
                    "{field}: this parameter is not supported in API version {version}"
                ));
            }
        }
    }
    let Some(messages) = body["messages"].as_array().filter(|m| !m.is_empty()) else {
        return bad("messages: at least one message is required");
    };
    let mut user = String::new();
    for (i, message) in messages.iter().enumerate() {
        let want = if i % 2 == 0 { "user" } else { "assistant" };
        if message["role"] != want {
            return bad("messages: roles must alternate between \"user\" and \"assistant\", starting with \"user\"");
        }
        if want == "user" {
            user = blocks_text(&message["content"]);
        }
    }
    if messages.len() % 2 == 0 {
        return bad("This model does not support assistant message prefill. The conversation must end with a user message.");
    }
    if body["stream"] == true {
        return bad("The fake model does not stream; leave stream out.");
    }
    let system = blocks_text(&body["system"]);
    let Some(model) = scenario::model(model_id) else {
        return error(
            404,
            "not_found_error",
            &format!("model: {model_id}"),
            &request_id,
        );
    };
    if let Some(failure) = scenario_failure(model.behaviour, shared, &request_id) {
        return failure;
    }
    if model.behaviour == Behaviour::Slow {
        std::thread::sleep(scenario::SLOW_ANSWER);
    }
    let reasoning = if model.behaviour == Behaviour::Reason {
        scenario::REASONING_TOKENS
    } else {
        0
    };
    let mut content = Vec::new();
    if reasoning > 0 {
        // thinking is not shown by default: an empty text and a signature
        content.push(json!({ "type": "thinking", "thinking": "", "signature": "fake-signature" }));
    }
    let answer = scenario::answer(model.behaviour, &system, &user, n);
    let room = limit.saturating_sub(reasoning);
    let (text, stop) = if scenario::tokens(&answer) > room {
        let kept: String = answer
            .chars()
            .take(usize::try_from(room).unwrap_or(usize::MAX))
            .collect();
        (kept, "max_tokens")
    } else {
        (answer, "end_turn")
    };
    if !text.is_empty() {
        content.push(json!({ "type": "text", "text": text }));
    }
    let mut message = json!({
        "id": format!("msg_fake{n:06}"),
        "type": "message",
        "role": "assistant",
        "model": model.id,
        "content": content,
        "stop_reason": stop,
        "stop_sequence": null,
        "stop_details": null,
        "usage": {
            "input_tokens": scenario::tokens(&system) + scenario::tokens(&user),
            "output_tokens": reasoning.min(limit) + scenario::tokens(&text),
        },
    });
    if model.behaviour == Behaviour::Refuse {
        message["content"] = json!([]);
        message["stop_reason"] = json!("refusal");
        message["stop_details"] = json!({ "type": "refusal", "category": "cyber", "explanation": "The fake model declines every request." });
    }
    Response::json(200, &message).with_header("request-id", request_id)
}

/// The error a failing model answers with, if it fails this time.
fn scenario_failure(behaviour: Behaviour, shared: &Shared, request_id: &str) -> Option<Response> {
    let overloaded =
        || error(529, "overloaded_error", "Overloaded", request_id).with_header("retry-after", "0");
    match behaviour {
        Behaviour::RateLimit => Some(
            error(
                429,
                "rate_limit_error",
                "This request would exceed the rate limit for your organization. Please try again later.",
                request_id,
            )
            .with_header("retry-after", "0"),
        ),
        // a spend limit: no retry-after, it fails until the limit is lifted
        Behaviour::Quota => Some(error(
            429,
            "rate_limit_error",
            "You have reached your specified workspace API usage limits.",
            request_id,
        )),
        Behaviour::Overloaded => Some(overloaded()),
        Behaviour::Flaky => shared.flaky_fails().then(overloaded),
        Behaviour::ServerError => Some(
            Response::text(500, "<html><body>upstream exploded</body></html>")
                .with_header("request-id", request_id),
        ),
        Behaviour::Answer
        | Behaviour::Reason
        | Behaviour::Skip
        | Behaviour::Refuse
        | Behaviour::Slow => None,
    }
}

/// The text of `content` or `system`: a string, or the `text` blocks of an array.
fn blocks_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|b| b["type"] == "text")
            .filter_map(|b| b["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}
