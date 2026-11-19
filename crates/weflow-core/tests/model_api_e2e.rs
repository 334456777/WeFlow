//! The model API client against the fake model API (weflow-fake-model): what goes out in each format, what comes back,
//! and how each failure the documents describe is retried and reported.
#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]
use std::time::Duration;

use serde_json::Value;
use weflow_core::model_api::{ModelApi, ModelApiError, ModelApiFormat, Prompt};
use weflow_fake_model::{FakeModel, KEY};

const TIMEOUT: Duration = Duration::from_secs(10);

fn api(fake: &FakeModel, format: ModelApiFormat, model: &str) -> ModelApi {
    // the errors are compared in English, whatever the machine's language
    weflow_core::locale::set(weflow_core::locale::Lang::En);
    ModelApi {
        format,
        base_url: match format {
            ModelApiFormat::OpenAi => fake.openai_base_url(),
            ModelApiFormat::Anthropic => fake.anthropic_base_url(),
        },
        key: KEY.into(),
        model: model.into(),
        max_tokens: 4096,
    }
}

const PROMPT: Prompt<'static> = Prompt {
    system: Some("You are brief."),
    user: "Please reply with the two words \"connection successful\".",
};

fn body(fake: &FakeModel, index: usize) -> Value {
    fake.requests()[index].json().unwrap()
}

#[tokio::test]
async fn openai_chat_completions_go_out_as_documented() {
    let fake = FakeModel::start().unwrap();
    let reply = api(&fake, ModelApiFormat::OpenAi, "fake-chat")
        .complete(&PROMPT, TIMEOUT)
        .await
        .unwrap();
    assert_eq!(reply.text, "connection successful");
    assert_eq!(reply.model.as_deref(), Some("fake-chat"));
    assert_eq!(reply.retries, 0);
    let request = &fake.requests()[0];
    assert_eq!(
        (request.method.as_str(), request.path.as_str()),
        ("POST", "/v1/chat/completions")
    );
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {KEY}").as_str())
    );
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert!(request.header("user-agent").unwrap().starts_with("weflow/"));
    let body = body(&fake, 0);
    assert_eq!(body["messages"][0]["role"], "system");
    assert_eq!(body["messages"][1]["role"], "user");
    assert_eq!(body["max_completion_tokens"], 4096);
    assert_eq!(body["stream"], false);
    assert!(
        body.get("temperature").is_none() && body.get("max_tokens").is_none(),
        "{body}"
    );
}

#[tokio::test]
async fn anthropic_messages_go_out_as_documented() {
    let fake = FakeModel::start().unwrap();
    let reply = api(&fake, ModelApiFormat::Anthropic, "fake-claude")
        .complete(&PROMPT, TIMEOUT)
        .await
        .unwrap();
    assert_eq!(reply.text, "connection successful");
    let request = &fake.requests()[0];
    assert_eq!(request.path, "/v1/messages");
    assert_eq!(request.header("x-api-key"), Some(KEY));
    assert_eq!(request.header("anthropic-version"), Some("2026-10-21"));
    let body = body(&fake, 0);
    assert_eq!(body["system"], "You are brief.");
    assert_eq!(body["max_tokens"], 4096);
    assert_eq!(body["messages"].as_array().unwrap().len(), 1);
    assert!(body.get("temperature").is_none(), "{body}");

    // a base that ends with /v1 gains /messages only
    let mut v1 = api(&fake, ModelApiFormat::Anthropic, "fake-claude");
    v1.base_url = format!("{}/v1/", fake.anthropic_base_url());
    v1.complete(&PROMPT, TIMEOUT).await.unwrap();
    assert_eq!(fake.requests()[1].path, "/v1/messages");
}

#[tokio::test]
async fn reasoning_counts_against_the_token_limit() {
    let fake = FakeModel::start().unwrap();
    for format in [ModelApiFormat::OpenAi, ModelApiFormat::Anthropic] {
        let mut reasoner = api(&fake, format, "fake-reasoner");
        assert!(
            reasoner.complete(&PROMPT, TIMEOUT).await.is_ok(),
            "{format}"
        );
        reasoner.max_tokens = 100;
        let err = reasoner.complete(&PROMPT, TIMEOUT).await.unwrap_err();
        assert_eq!(err, ModelApiError::Truncated, "{format}");
        assert!(err.to_string().contains("ai_model_api_max_tokens"));
    }
}

#[tokio::test]
async fn failures_say_what_the_service_said() {
    let fake = FakeModel::start().unwrap();
    let mut wrong_key = api(&fake, ModelApiFormat::Anthropic, "fake-claude");
    wrong_key.key = "sk-wrong".into();
    let err = wrong_key.complete(&PROMPT, TIMEOUT).await.unwrap_err();
    assert_eq!(err.status(), Some(401));
    let text = err.to_string();
    assert!(
        text.contains("authentication_error") && text.contains("invalid x-api-key"),
        "{text}"
    );
    assert!(
        text.contains("Check the API key") && text.contains("request id req_fake"),
        "{text}"
    );

    let unknown = api(&fake, ModelApiFormat::OpenAi, "no-such-model");
    let text = unknown
        .complete(&PROMPT, TIMEOUT)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        text.starts_with("HTTP 404 (invalid_request_error, model_not_found)"),
        "{text}"
    );
    assert!(text.contains("the model name"), "{text}");

    // an OpenAI request sent to where Anthropic's is served
    let mut misplaced = api(&fake, ModelApiFormat::OpenAi, "fake-chat");
    misplaced.base_url = fake.anthropic_base_url();
    assert_eq!(
        misplaced
            .complete(&PROMPT, TIMEOUT)
            .await
            .unwrap_err()
            .status(),
        Some(404)
    );

    for format in [ModelApiFormat::OpenAi, ModelApiFormat::Anthropic] {
        let err = api(&fake, format, "fake-refusal")
            .complete(&PROMPT, TIMEOUT)
            .await
            .unwrap_err();
        assert!(
            matches!(err, ModelApiError::Refused { reason: Some(_) }),
            "{format}: {err:?}"
        );
    }
}

/// The requests of `fake` since `before`.
fn sent_since(fake: &FakeModel, before: usize) -> usize {
    fake.requests().len() - before
}

#[tokio::test]
async fn transient_failures_are_retried_twice() {
    let fake = FakeModel::start().unwrap();
    for (format, model, status) in [
        (ModelApiFormat::OpenAi, "fake-rate-limit", 429),
        (ModelApiFormat::Anthropic, "fake-rate-limit", 429),
        (ModelApiFormat::OpenAi, "fake-overloaded", 503),
        (ModelApiFormat::Anthropic, "fake-overloaded", 529),
        (ModelApiFormat::OpenAi, "fake-server-error", 500),
    ] {
        let before = fake.requests().len();
        let err = api(&fake, format, model)
            .complete(&PROMPT, TIMEOUT)
            .await
            .unwrap_err();
        let ModelApiError::Status(failure) = &err else {
            panic!("{format} {model}: {err:?}");
        };
        assert_eq!(
            (failure.status, failure.retries),
            (status, 2),
            "{format} {model}"
        );
        assert_eq!(sent_since(&fake, before), 3, "{format} {model}");
        assert!(err.to_string().contains("tried 2 more times"), "{err}");
    }
    let text = api(&fake, ModelApiFormat::Anthropic, "fake-server-error")
        .complete(&PROMPT, TIMEOUT)
        .await
        .unwrap_err()
        .to_string();
    assert!(
        text.contains("upstream exploded"),
        "a body that is not JSON is quoted: {text}"
    );

    let reply = api(&fake, ModelApiFormat::OpenAi, "fake-flaky")
        .complete(&PROMPT, TIMEOUT)
        .await
        .unwrap();
    assert_eq!(reply.retries, 1);
}

#[tokio::test]
async fn a_used_up_quota_is_not_retried() {
    let fake = FakeModel::start().unwrap();
    for format in [ModelApiFormat::OpenAi, ModelApiFormat::Anthropic] {
        let before = fake.requests().len();
        let err = api(&fake, format, "fake-quota")
            .complete(&PROMPT, TIMEOUT)
            .await
            .unwrap_err();
        assert_eq!(err.status(), Some(429), "{format}");
        assert!(err.to_string().contains("is used up"), "{format}: {err}");
        assert_eq!(sent_since(&fake, before), 1, "{format}");
    }
}

#[tokio::test]
async fn an_unreachable_service_is_retried_then_reported() {
    let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = closed.local_addr().unwrap().port();
    drop(closed);
    weflow_core::locale::set(weflow_core::locale::Lang::En);
    let api = ModelApi {
        base_url: format!("http://127.0.0.1:{port}/v1"),
        key: KEY.into(),
        model: "fake-chat".into(),
        max_tokens: 10,
        ..ModelApi::default()
    };
    let err = api.complete(&PROMPT, TIMEOUT).await.unwrap_err();
    assert!(
        matches!(err, ModelApiError::Network { retries: 2, .. }),
        "{err:?}"
    );
    assert!(!err.to_string().contains(KEY));
}

#[tokio::test]
async fn both_formats_list_the_models() {
    let fake = FakeModel::start().unwrap();
    let openai = api(&fake, ModelApiFormat::OpenAi, "")
        .list_models(TIMEOUT)
        .await
        .unwrap();
    assert!(openai
        .iter()
        .any(|m| m.id == "fake-chat" && m.display_name.is_none()));
    let anthropic = api(&fake, ModelApiFormat::Anthropic, "")
        .list_models(TIMEOUT)
        .await
        .unwrap();
    assert_eq!(anthropic[0].id, "fake-chat");
    assert_eq!(anthropic[0].display_name.as_deref(), Some("Fake Chat"));
    let requests = fake.requests();
    assert_eq!(requests[1].path, "/v1/models");
    assert_eq!(requests[1].query, "limit=1000");
    assert_eq!(requests[1].header("anthropic-version"), Some("2026-10-21"));
}

#[tokio::test]
async fn a_service_without_a_model_list_says_so() {
    let fake = FakeModel::start().unwrap();
    for format in [ModelApiFormat::OpenAi, ModelApiFormat::Anthropic] {
        let mut listless = api(&fake, format, "fake-chat");
        listless.base_url = format!("{}/no-list", listless.base_url);
        let err = listless.list_models(TIMEOUT).await.unwrap_err();
        assert_eq!(err.status(), Some(404), "{format}");
        let text = err.to_string();
        assert!(
            text.contains("no list of models") && !text.contains("the model name."),
            "{format}: {text}"
        );
    }
}
