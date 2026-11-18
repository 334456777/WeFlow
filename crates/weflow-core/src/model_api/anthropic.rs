//! Anthropic's Messages (`POST {base}/v1/messages`), as its API reference gives it: the version in
//! `anthropic-version` (2026-10-21, which refuses `temperature`, `top_p` and `top_k`), the instructions in the top-level
//! `system`, the required `max_tokens`. The key goes in `Authorization: Bearer` and in `x-api-key` alike: the API takes
//! either, and the services compatible with it read one or the other. The answer is the `text` blocks of `content`;
//! `stop_reason` tells a refusal and a stop at the token limit.
use serde_json::{json, Value};

use super::{text_of, Answer, ModelApiError, ModelApiFormat, ModelInfo, Prompt};

pub(super) const DEFAULT_BASE_URL: &str = "https://api.anthropic.com";
pub(super) const REQUEST_ID_HEADER: &str = "request-id";

/// The API version sent in `anthropic-version`.
pub(super) const VERSION: &str = "2026-10-21";

/// The most models one page of `GET /v1/models` lists.
const MODELS_PAGE: u32 = 1000;

/// The base without `/v1/messages` or `/v1` at its end.
fn root(base: &str) -> &str {
    base.strip_suffix("/v1/messages")
        .or_else(|| base.strip_suffix("/v1"))
        .unwrap_or(base)
}

/// `{base}/v1/messages`; a base that ends with `/v1` gains `/messages` only, and that address itself is taken as it is.
pub(super) fn endpoint(base: &str) -> String {
    format!("{}/v1/messages", root(base))
}

/// `{base}/v1/models`, a page as long as the API allows.
pub(super) fn models_endpoint(base: &str) -> String {
    format!("{}/v1/models?limit={MODELS_PAGE}", root(base))
}

/// The version, and the key when there is one: a service of one's own may take none.
pub(super) fn headers(key: &str) -> Vec<(&'static str, String)> {
    let mut headers = vec![("anthropic-version", VERSION.to_string())];
    if !key.is_empty() {
        headers.push(("x-api-key", key.to_string()));
        headers.push(("authorization", format!("Bearer {key}")));
    }
    headers
}

pub(super) fn request(model: &str, prompt: &Prompt<'_>, max_tokens: u64) -> Value {
    let mut body = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": [{ "role": "user", "content": prompt.user }],
    });
    if let Some(system) = prompt.system.filter(|s| !s.trim().is_empty()) {
        body["system"] = json!(system);
    }
    body
}

/// The answer of a message (`body` is `value`'s text, quoted when it is not one).
pub(super) fn answer(value: &Value, body: &str) -> Result<Answer, ModelApiError> {
    if value["type"] == "error" {
        return Err(ModelApiError::Status(Box::new(super::StatusError::read(
            200, body, None,
        ))));
    }
    let Some(blocks) = value["content"].as_array() else {
        return Err(ModelApiError::unexpected(ModelApiFormat::Anthropic, body));
    };
    let stop = value["stop_reason"].as_str();
    if stop == Some("refusal") {
        let details = &value["stop_details"];
        return Err(ModelApiError::Refused {
            reason: text_of(&details["explanation"]).or_else(|| text_of(&details["category"])),
        });
    }
    // thinking blocks come before the text, which can come in several blocks
    let text = blocks
        .iter()
        .filter(|b| b["type"] == "text")
        .filter_map(|b| b["text"].as_str())
        .collect::<String>()
        .trim()
        .to_string();
    if !text.is_empty() {
        return Ok(Answer {
            text,
            model: text_of(&value["model"]),
        });
    }
    Err(match stop {
        Some("max_tokens" | "model_context_window_exceeded") => ModelApiError::Truncated,
        _ => ModelApiError::Empty,
    })
}

/// `GET /v1/models`: `{"data": [{"id", "display_name", "created_at"}], "has_more"}`, the newest first.
pub(super) fn models(value: &Value) -> Option<Vec<ModelInfo>> {
    Some(
        value["data"]
            .as_array()?
            .iter()
            .filter_map(|m| {
                Some(ModelInfo {
                    id: text_of(&m["id"])?,
                    display_name: text_of(&m["display_name"]),
                })
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_follows_the_base() {
        assert_eq!(
            endpoint("https://api.anthropic.com"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            endpoint("https://gw.test/anthropic/v1"),
            "https://gw.test/anthropic/v1/messages"
        );
        assert_eq!(
            endpoint("https://gw.test/v1/messages"),
            "https://gw.test/v1/messages"
        );
        assert_eq!(
            models_endpoint("https://gw.test/v1/messages"),
            "https://gw.test/v1/models?limit=1000"
        );
    }

    #[test]
    fn the_request_has_a_top_level_system_and_no_sampling() {
        let body = request(
            "claude-test",
            &Prompt {
                system: Some("be brief"),
                user: "hi",
            },
            512,
        );
        assert_eq!(
            body,
            json!({
                "model": "claude-test",
                "max_tokens": 512,
                "system": "be brief",
                "messages": [{ "role": "user", "content": "hi" }],
            })
        );
        assert!(request(
            "m",
            &Prompt {
                system: None,
                user: "hi"
            },
            1
        )
        .get("system")
        .is_none());
        let headers = headers("sk-ant");
        assert!(headers.contains(&("anthropic-version", "2026-10-21".into())));
        assert!(headers.contains(&("x-api-key", "sk-ant".into())));
    }

    fn answer_of(value: Value) -> Result<Answer, ModelApiError> {
        answer(&value, &value.to_string())
    }

    #[test]
    fn answers_and_their_failures() {
        let ok = answer_of(json!({
            "id": "msg_1", "type": "message", "role": "assistant", "model": "claude-test",
            "content": [{ "type": "thinking", "thinking": "", "signature": "x" }, { "type": "text", "text": "Hel" }, { "type": "text", "text": "lo " }],
            "stop_reason": "end_turn", "usage": { "input_tokens": 3, "output_tokens": 2 }
        }));
        assert_eq!(
            ok,
            Ok(Answer {
                text: "Hello".into(),
                model: Some("claude-test".into())
            })
        );
        assert_eq!(
            answer_of(
                json!({ "content": [{ "type": "text", "text": "partial" }], "stop_reason": "refusal",
                "stop_details": { "type": "refusal", "category": "cyber", "explanation": "not allowed" } })
            ),
            Err(ModelApiError::Refused {
                reason: Some("not allowed".into())
            })
        );
        assert_eq!(
            answer_of(
                json!({ "content": [{ "type": "thinking", "thinking": "" }], "stop_reason": "max_tokens" })
            ),
            Err(ModelApiError::Truncated)
        );
        assert_eq!(
            answer_of(json!({ "content": [], "stop_reason": "end_turn" })),
            Err(ModelApiError::Empty)
        );
        assert!(matches!(
            answer_of(json!({ "choices": [] })),
            Err(ModelApiError::UnexpectedBody {
                format: ModelApiFormat::Anthropic,
                ..
            })
        ));
    }

    #[test]
    fn models_keep_their_names() {
        let list = models(&json!({ "data": [{ "type": "model", "id": "claude-a", "display_name": "Claude A" }], "has_more": false })).unwrap();
        assert_eq!(
            list,
            [ModelInfo {
                id: "claude-a".into(),
                display_name: Some("Claude A".into())
            }]
        );
    }
}
