//! The model API the AI insights ask: OpenAI's Chat Completions or Anthropic's Messages, as their documents describe
//! them, which is also what the services compatible with one of the two take. One question goes out (a system prompt
//! and the user's text) and the text of the answer comes back. A failure says what the service said: its HTTP status,
//! its error type and message, and the request id. A rate limit, a server error or a lost connection is tried again
//! twice, after the pause the service asks for, as the official SDKs of both services do.
use std::fmt;
use std::str::FromStr;
use std::time::Duration;

use reqwest::header::{HeaderMap, ACCEPT, CONTENT_TYPE, USER_AGENT};
use reqwest::Method;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

mod anthropic;
mod error;
mod openai;
mod retry;

pub use error::{ModelApiError, Request, StatusError};

/// The wire format a model API speaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
pub enum ModelApiFormat {
    /// OpenAI's Chat Completions: `POST {base}/chat/completions` with `Authorization: Bearer`.
    #[default]
    #[serde(rename = "openai")]
    OpenAi,
    /// Anthropic's Messages: `POST {base}/v1/messages` with `x-api-key` and `anthropic-version`.
    #[serde(rename = "anthropic")]
    Anthropic,
}

impl ModelApiFormat {
    /// The names `ai_model_api_format` takes.
    pub const NAMES: [&'static str; 2] = ["openai", "anthropic"];

    /// Its name, one of [`Self::NAMES`].
    pub fn as_str(self) -> &'static str {
        match self {
            ModelApiFormat::OpenAi => "openai",
            ModelApiFormat::Anthropic => "anthropic",
        }
    }

    /// The address of the service itself, used when no base URL is set.
    pub fn default_base_url(self) -> &'static str {
        match self {
            ModelApiFormat::OpenAi => openai::DEFAULT_BASE_URL,
            ModelApiFormat::Anthropic => anthropic::DEFAULT_BASE_URL,
        }
    }
}

impl fmt::Display for ModelApiFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name that is not one of [`ModelApiFormat::NAMES`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseFormatError(pub String);

impl fmt::Display for ParseFormatError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "unknown model API format '{}'; use openai or anthropic",
            self.0
        )
    }
}

impl std::error::Error for ParseFormatError {}

impl FromStr for ModelApiFormat {
    type Err = ParseFormatError;

    /// `openai` or `anthropic`, in any case, around any spaces.
    fn from_str(text: &str) -> Result<Self, Self::Err> {
        match text.trim().to_ascii_lowercase().as_str() {
            "openai" => Ok(ModelApiFormat::OpenAi),
            "anthropic" => Ok(ModelApiFormat::Anthropic),
            _ => Err(ParseFormatError(text.trim().to_string())),
        }
    }
}

/// A model API as the settings give it.
#[derive(Clone, Default)]
pub struct ModelApi {
    /// The wire format it speaks.
    pub format: ModelApiFormat,
    /// The service's base URL; empty is the service of the format itself ([`ModelApiFormat::default_base_url`]).
    pub base_url: String,
    /// The API key.
    pub key: String,
    /// The model asked for.
    pub model: String,
    /// The most tokens of one answer, reasoning included where the model reasons.
    pub max_tokens: u64,
}

impl fmt::Debug for ModelApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ModelApi")
            .field("format", &self.format)
            .field("base_url", &self.base_url)
            .field("key", &if self.key.is_empty() { "" } else { "<set>" })
            .field("model", &self.model)
            .field("max_tokens", &self.max_tokens)
            .finish()
    }
}

/// What is asked: the instructions and the user's text.
#[derive(Clone, Copy, Debug)]
pub struct Prompt<'a> {
    /// The system prompt; none sends only the user's text.
    pub system: Option<&'a str>,
    /// The user's text.
    pub user: &'a str,
}

/// The answer to a [`Prompt`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reply {
    /// The text of the answer, trimmed; never empty.
    pub text: String,
    /// The model that answered, as the service names it.
    pub model: Option<String>,
    /// The requests tried again before this answer.
    pub retries: u32,
}

/// A model the service offers (`GET /v1/models`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    /// What `ai_model_api_model` takes.
    pub id: String,
    /// The name the service shows, where it gives one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// What a successful answer held, before it becomes a [`Reply`].
#[derive(Debug, PartialEq, Eq)]
struct Answer {
    text: String,
    model: Option<String>,
}

impl ModelApi {
    /// The base URL without the slashes and spaces around it, or the format's own service.
    fn base(&self) -> &str {
        let base = self.base_url.trim().trim_end_matches('/');
        if base.is_empty() {
            self.format.default_base_url()
        } else {
            base
        }
    }

    /// Where a question goes.
    pub fn endpoint(&self) -> String {
        match self.format {
            ModelApiFormat::OpenAi => openai::endpoint(self.base()),
            ModelApiFormat::Anthropic => anthropic::endpoint(self.base()),
        }
    }

    /// Where the list of models is.
    pub fn models_endpoint(&self) -> String {
        match self.format {
            ModelApiFormat::OpenAi => openai::models_endpoint(self.base()),
            ModelApiFormat::Anthropic => anthropic::models_endpoint(self.base()),
        }
    }

    /// Ask `prompt`; each request may take `timeout`.
    ///
    /// # Errors
    ///
    /// The service could not be reached or did not answer in time, answered with an error status (after the retries a
    /// transient one gets), or answered without text: a refusal, a stop at the token limit or nothing at all.
    pub async fn complete(
        &self,
        prompt: &Prompt<'_>,
        timeout: Duration,
    ) -> Result<Reply, ModelApiError> {
        let model = self.model.trim();
        let body = match self.format {
            ModelApiFormat::OpenAi => openai::request(model, prompt, self.max_tokens),
            ModelApiFormat::Anthropic => anthropic::request(model, prompt, self.max_tokens),
        };
        let (text, retries) = self
            .exchange(
                Method::POST,
                &self.endpoint(),
                Some(body.to_string()),
                timeout,
            )
            .await?;
        let value = self.json_of(&text)?;
        let answer = match self.format {
            ModelApiFormat::OpenAi => openai::answer(&value, &text),
            ModelApiFormat::Anthropic => anthropic::answer(&value, &text),
        }?;
        Ok(Reply {
            text: answer.text,
            model: answer.model,
            retries,
        })
    }

    /// The models the service offers, as it lists them.
    ///
    /// # Errors
    ///
    /// As [`ModelApi::complete`], and when the answer is not a list of models; an error status is marked
    /// [`Request::Models`], so a service without the list is told apart from a wrong address.
    pub async fn list_models(&self, timeout: Duration) -> Result<Vec<ModelInfo>, ModelApiError> {
        let (text, _) = self
            .exchange(Method::GET, &self.models_endpoint(), None, timeout)
            .await
            .map_err(|err| match err {
                ModelApiError::Status(mut failure) => {
                    failure.request = Request::Models;
                    ModelApiError::Status(failure)
                }
                other => other,
            })?;
        let value = self.json_of(&text)?;
        match self.format {
            ModelApiFormat::OpenAi => openai::models(&value),
            ModelApiFormat::Anthropic => anthropic::models(&value),
        }
        .ok_or_else(|| ModelApiError::unexpected(self.format, &text))
    }

    fn json_of(&self, text: &str) -> Result<Value, ModelApiError> {
        serde_json::from_str(text).map_err(|_| ModelApiError::unexpected(self.format, text))
    }

    fn headers(&self) -> Vec<(&'static str, String)> {
        let key = self.key.trim();
        match self.format {
            ModelApiFormat::OpenAi => openai::headers(key),
            ModelApiFormat::Anthropic => anthropic::headers(key),
        }
    }

    /// Send a request and read the body of a successful answer, trying a transient failure again; with the retries made.
    async fn exchange(
        &self,
        method: Method,
        url: &str,
        body: Option<String>,
        timeout: Duration,
    ) -> Result<(String, u32), ModelApiError> {
        let client = crate::http_client::builder()
            .timeout(timeout)
            .build()
            .map_err(|err| ModelApiError::Network {
                cause: crate::http_client::failure(err),
                retries: 0,
            })?;
        let timed_out = || ModelApiError::Timeout {
            seconds: timeout.as_secs(),
        };
        let mut retries = 0;
        loop {
            let mut request = client
                .request(method.clone(), url)
                .header(ACCEPT, "application/json")
                .header(USER_AGENT, concat!("weflow/", env!("CARGO_PKG_VERSION")));
            for (name, value) in self.headers() {
                request = request.header(name, value);
            }
            if let Some(body) = &body {
                request = request
                    .header(CONTENT_TYPE, "application/json")
                    .body(body.clone());
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(err) if err.is_builder() => {
                    return Err(ModelApiError::InvalidUrl {
                        endpoint: url.to_string(),
                    })
                }
                Err(err) if err.is_timeout() => return Err(timed_out()),
                Err(err) => {
                    let cause = crate::http_client::failure(err);
                    if retries < retry::MAX_RETRIES {
                        tokio::time::sleep(retry::backoff(retries)).await;
                        retries += 1;
                        continue;
                    }
                    return Err(ModelApiError::Network { cause, retries });
                }
            };
            let status = response.status().as_u16();
            let request_id = header_text(response.headers(), self.request_id_header());
            let asked = retry::requested_wait(response.headers());
            let text = response.text().await.map_err(|err| {
                if err.is_timeout() {
                    timed_out()
                } else {
                    ModelApiError::Network {
                        cause: crate::http_client::failure(err),
                        retries,
                    }
                }
            })?;
            if (200..300).contains(&status) {
                return Ok((text, retries));
            }
            let mut failure = StatusError::read(status, &text, request_id);
            failure.retries = retries;
            failure.used_up = retry::used_up(self.format, &failure, asked.is_some());
            if retries < retry::MAX_RETRIES && retry::worth_retrying(&failure) {
                if let Some(pause) = retry::pause(asked, retries) {
                    tokio::time::sleep(pause).await;
                    retries += 1;
                    continue;
                }
            }
            return Err(ModelApiError::Status(Box::new(failure)));
        }
    }

    fn request_id_header(&self) -> &'static str {
        match self.format {
            ModelApiFormat::OpenAi => openai::REQUEST_ID_HEADER,
            ModelApiFormat::Anthropic => anthropic::REQUEST_ID_HEADER,
        }
    }
}

fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .map(str::to_string)
}

/// The text of a string field, trimmed; `None` when it is missing, not text or empty.
fn text_of(value: &Value) -> Option<String> {
    value
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn api(format: ModelApiFormat, base: &str) -> ModelApi {
        ModelApi {
            format,
            base_url: base.into(),
            ..ModelApi::default()
        }
    }

    #[test]
    fn formats_read_their_names() {
        assert_eq!(" OpenAI ".parse(), Ok(ModelApiFormat::OpenAi));
        assert_eq!("anthropic".parse(), Ok(ModelApiFormat::Anthropic));
        assert!("claude".parse::<ModelApiFormat>().is_err());
        for name in ModelApiFormat::NAMES {
            assert_eq!(name.parse::<ModelApiFormat>().unwrap().as_str(), name);
        }
    }

    #[test]
    fn an_empty_base_is_the_service_of_the_format() {
        assert_eq!(
            api(ModelApiFormat::OpenAi, " ").endpoint(),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            api(ModelApiFormat::Anthropic, "").endpoint(),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            api(ModelApiFormat::Anthropic, "").models_endpoint(),
            "https://api.anthropic.com/v1/models?limit=1000"
        );
    }

    #[test]
    fn the_key_stays_out_of_debug_output() {
        let api = ModelApi {
            key: "sk-secret".into(),
            ..ModelApi::default()
        };
        let shown = format!("{api:?}");
        assert!(
            !shown.contains("sk-secret") && shown.contains("<set>"),
            "{shown}"
        );
    }
}
