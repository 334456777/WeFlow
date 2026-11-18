//! OpenAI's Chat Completions (`POST {base}/chat/completions`), as its OpenAPI description gives it: the key in
//! `Authorization: Bearer`, the instructions as a `system` message (which the services compatible with OpenAI read
//! too), the limit in `max_completion_tokens` (`max_tokens` is deprecated and refused by reasoning models), and no
//! `temperature`, whose non-default values reasoning models refuse. The answer is `choices[0].message.content`.
use serde_json::{json, Value};

use super::{text_of, Answer, ModelApiError, ModelApiFormat, ModelInfo, Prompt};

pub(super) const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";
pub(super) const REQUEST_ID_HEADER: &str = "x-request-id";

const CHAT_PATH: &str = "/chat/completions";

/// `{base}/chat/completions`; a base that is that address already is taken as it is.
pub(super) fn endpoint(base: &str) -> String {
    if base.ends_with(CHAT_PATH) {
        base.to_string()
    } else {
        format!("{base}{CHAT_PATH}")
    }
}

/// `{base}/models`.
pub(super) fn models_endpoint(base: &str) -> String {
    format!("{}/models", base.strip_suffix(CHAT_PATH).unwrap_or(base))
}

/// The key, when there is one: a service of one's own may take none.
pub(super) fn headers(key: &str) -> Vec<(&'static str, String)> {
    if key.is_empty() {
        return Vec::new();
    }
    vec![("authorization", format!("Bearer {key}"))]
}

pub(super) fn request(model: &str, prompt: &Prompt<'_>, max_tokens: u64) -> Value {
    let mut messages = Vec::with_capacity(2);
    if let Some(system) = prompt.system.filter(|s| !s.trim().is_empty()) {
        messages.push(json!({ "role": "system", "content": system }));
    }
    messages.push(json!({ "role": "user", "content": prompt.user }));
    json!({
        "model": model,
        "messages": messages,
        "max_completion_tokens": max_tokens,
        "stream": false,
    })
}

/// The answer of a chat completion (`body` is `value`'s text, quoted when it is not one).
pub(super) fn answer(value: &Value, body: &str) -> Result<Answer, ModelApiError> {
    let Some(choice) = value["choices"].get(0) else {
        // a compatible service that answers an error with a success status
        if value["error"].is_object() || value["error"].is_string() {
            return Err(ModelApiError::Status(Box::new(super::StatusError::read(
                200, body, None,
            ))));
        }
        return Err(ModelApiError::unexpected(ModelApiFormat::OpenAi, body));
    };
    let message = &choice["message"];
    if !message.is_object() {
        return Err(ModelApiError::unexpected(ModelApiFormat::OpenAi, body));
    }
    let text = content_text(&message["content"]);
    let model = text_of(&value["model"]);
    if !text.is_empty() {
        return Ok(Answer { text, model });
    }
    if let Some(reason) = text_of(&message["refusal"]) {
        return Err(ModelApiError::Refused {
            reason: Some(reason),
        });
    }
    Err(match choice["finish_reason"].as_str() {
        Some("length") => ModelApiError::Truncated,
        Some("content_filter") => ModelApiError::Refused {
            reason: Some("content_filter".into()),
        },
        _ => ModelApiError::Empty,
    })
}

/// The text of `content`: a string, or the `text` parts of an array (as some compatible services answer).
fn content_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.trim().to_string(),
        Value::Array(parts) => parts
            .iter()
            .filter(|p| p["type"].as_str().is_none_or(|t| t == "text"))
            .filter_map(|p| p["text"].as_str())
            .collect::<String>()
            .trim()
            .to_string(),
        _ => String::new(),
    }
}

/// `GET /models`: `{"object": "list", "data": [{"id", "created", "owned_by"}]}`, the newest first.
pub(super) fn models(value: &Value) -> Option<Vec<ModelInfo>> {
    let mut listed: Vec<(i64, ModelInfo)> = value["data"]
        .as_array()?
        .iter()
        .filter_map(|m| {
            Some((
                m["created"].as_i64().unwrap_or(0),
                ModelInfo {
                    id: text_of(&m["id"])?,
                    display_name: None,
                },
            ))
        })
        .collect();
    listed.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.id.cmp(&b.1.id)));
    Some(listed.into_iter().map(|(_, m)| m).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_endpoint_follows_the_base() {
        assert_eq!(
            endpoint("https://api.ohmygpt.com/v1"),
            "https://api.ohmygpt.com/v1/chat/completions"
        );
        assert_eq!(
            endpoint("https://x.test/v1/chat/completions"),
            "https://x.test/v1/chat/completions"
        );
        assert_eq!(
            models_endpoint("https://x.test/v1/chat/completions"),
            "https://x.test/v1/models"
        );
        assert_eq!(
            models_endpoint("https://x.test/v1"),
            "https://x.test/v1/models"
        );
    }

    #[test]
    fn the_request_uses_max_completion_tokens_and_no_temperature() {
        let body = request(
            "gpt-test",
            &Prompt {
                system: Some("be brief"),
                user: "hi",
            },
            512,
        );
        assert_eq!(
            body,
            json!({
                "model": "gpt-test",
                "messages": [{ "role": "system", "content": "be brief" }, { "role": "user", "content": "hi" }],
                "max_completion_tokens": 512,
                "stream": false,
            })
        );
        let bare = request(
            "m",
            &Prompt {
                system: Some(" "),
                user: "hi",
            },
            1,
        );
        assert_eq!(bare["messages"].as_array().unwrap().len(), 1);
    }

    fn answer_of(value: Value) -> Result<Answer, ModelApiError> {
        answer(&value, &value.to_string())
    }

    #[test]
    fn answers_and_their_failures() {
        let ok = answer_of(json!({
            "id": "chatcmpl-1", "model": "gpt-test-2026",
            "choices": [{ "index": 0, "finish_reason": "stop", "message": { "role": "assistant", "content": " hello ", "refusal": null } }]
        }));
        assert_eq!(
            ok,
            Ok(Answer {
                text: "hello".into(),
                model: Some("gpt-test-2026".into())
            })
        );
        let parts = answer_of(
            json!({ "choices": [{ "message": { "content": [{ "type": "text", "text": "a" }, { "type": "text", "text": "b" }] } }] }),
        );
        assert_eq!(parts.unwrap().text, "ab");
        assert_eq!(
            answer_of(
                json!({ "choices": [{ "finish_reason": "stop", "message": { "content": null, "refusal": "I can't help with that." } }] })
            ),
            Err(ModelApiError::Refused {
                reason: Some("I can't help with that.".into())
            })
        );
        assert_eq!(
            answer_of(
                json!({ "choices": [{ "finish_reason": "length", "message": { "content": "" } }] })
            ),
            Err(ModelApiError::Truncated)
        );
        assert_eq!(
            answer_of(
                json!({ "choices": [{ "finish_reason": "stop", "message": { "content": "  " } }] })
            ),
            Err(ModelApiError::Empty)
        );
        assert!(matches!(
            answer_of(json!({ "type": "message", "content": [] })),
            Err(ModelApiError::UnexpectedBody {
                format: ModelApiFormat::OpenAi,
                ..
            })
        ));
        let error =
            answer_of(json!({ "error": { "message": "model busy", "type": "server_error" } }));
        assert_eq!(error.unwrap_err().status(), Some(200));
    }

    #[test]
    fn models_are_listed_newest_first() {
        let list = models(&json!({ "object": "list", "data": [
            { "id": "old", "created": 1 }, { "id": "new", "created": 9 }, { "id": "" }
        ] }))
        .unwrap();
        let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["new", "old"]);
        assert_eq!(models(&json!({ "models": [] })), None);
    }
}
