//! How a request to the model API fails, in words for the person who set it up: what the service said and what to
//! check.
use std::fmt;

use serde_json::Value;

use super::{text_of, ModelApiFormat};
use crate::locale::{self, tr, Lang};

/// The longest piece of an answer quoted in an error.
const QUOTED_CHARS: usize = 200;

/// Why asking the model API failed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ModelApiError {
    /// The address cannot be requested.
    InvalidUrl { endpoint: String },
    /// No answer within the time given to one request.
    Timeout { seconds: u64 },
    /// The service could not be reached, or the connection broke.
    Network { cause: String, retries: u32 },
    /// The service answered with an error status, or with an error in a successful one.
    Status(Box<StatusError>),
    /// The answer is not what the format answers.
    UnexpectedBody {
        format: ModelApiFormat,
        head: String,
    },
    /// The model declined to answer.
    Refused { reason: Option<String> },
    /// The model stopped at the token limit before it wrote any answer (a reasoning model spends tokens on reasoning).
    Truncated,
    /// The answer holds no text.
    Empty,
}

/// What the failed request asked for, which decides what to check.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Request {
    /// A question to the model.
    #[default]
    Answer,
    /// The list of models, which some services do not offer.
    Models,
}

/// An error status of the model API and what its body says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatusError {
    pub status: u16,
    pub request: Request,
    /// The error's type: `error.type` of both formats (`rate_limit_error`, `invalid_request_error`, …).
    pub kind: Option<String>,
    /// OpenAI's `error.code` (`invalid_api_key`, `insufficient_quota`, …).
    pub code: Option<String>,
    pub message: Option<String>,
    /// The start of a body that is not an error of either format.
    pub body: Option<String>,
    /// `x-request-id` (OpenAI) or `request-id` (Anthropic), or `request_id` in the body.
    pub request_id: Option<String>,
    /// The requests tried again before the last one failed.
    pub retries: u32,
    /// A 429 for a quota, a credit or a spend limit used up, which waiting does not restore.
    pub used_up: bool,
}

impl StatusError {
    /// The error of an answer with `status` and `body`: `{"error": {"type", "message", "code"}}` of OpenAI and
    /// `{"type": "error", "error": {"type", "message"}, "request_id"}` of Anthropic are read alike; any other body is
    /// quoted.
    pub(super) fn read(status: u16, body: &str, request_id: Option<String>) -> Self {
        let value: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let (kind, code, message) = match value.get("error") {
            Some(error @ Value::Object(_)) => (
                text_of(&error["type"]),
                text_of(&error["code"]).or_else(|| error["code"].as_i64().map(|c| c.to_string())),
                text_of(&error["message"]),
            ),
            // `{"error": "…"}` of some compatible services
            Some(message @ Value::String(_)) => (None, None, text_of(message)),
            _ => (
                None,
                None,
                text_of(&value["message"]).or_else(|| text_of(&value["detail"])),
            ),
        };
        let body = message
            .is_none()
            .then(|| quote(body))
            .filter(|b| !b.is_empty());
        Self {
            status,
            request: Request::Answer,
            kind,
            code,
            message,
            body,
            request_id: request_id.or_else(|| text_of(&value["request_id"])),
            retries: 0,
            used_up: false,
        }
    }

    /// Whether the account is out of quota or credit, which waiting does not fix.
    pub(super) fn out_of_quota(&self) -> bool {
        [&self.kind, &self.code].into_iter().flatten().any(|name| {
            let name = name.to_ascii_lowercase();
            ["quota", "credit", "billing", "spend_limit", "usage_limit"]
                .iter()
                .any(|word| name.contains(word))
        })
    }

    /// What to check for this status.
    fn hint(&self) -> Option<&'static str> {
        Some(match self.status {
            401 => tr("Check the API key.", "请检查 API Key。"),
            402 => tr("Check the account's billing.", "请检查账户的付费状态。"),
            403 => tr(
                "The key may not use this model or address.",
                "这个 Key 无权使用该模型或地址。",
            ),
            // the request names no model: the address has no list, which is what some services offer
            404 | 405 if self.request == Request::Models => tr(
                "This address has no list of models: some services offer none, so enter the model name yourself. If testing the connection fails too, check the API address and the API format.",
                "这个地址没有模型列表：有的服务不提供模型列表，请直接填写模型名称。如果「测试连接」也失败，再检查接口地址和接口格式。",
            ),
            404 => tr(
                "Check the API address, the API format and the model name.",
                "请检查接口地址、接口格式和模型名称。",
            ),
            413 => tr("The prompt is too long.", "提示太长。"),
            429 if self.used_up || self.out_of_quota() => tr(
                "The account's quota, credit or spend limit is used up.",
                "账户的额度、余额或消费上限已用完。",
            ),
            429 => tr(
                "Too many requests; try again later.",
                "请求太频繁，请稍后再试。",
            ),
            408 | 500..=599 => tr(
                "The service is unavailable for now; try again later.",
                "服务暂时不可用，请稍后再试。",
            ),
            _ => return None,
        })
    }
}

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let zh = locale::current() == Lang::Zh;
        write!(f, "HTTP {}", self.status)?;
        let names: Vec<&str> = [&self.kind, &self.code]
            .into_iter()
            .flatten()
            .map(String::as_str)
            .collect();
        if !names.is_empty() {
            write!(f, " {}", names.join(tr(", ", "、")))?;
        }
        // a sentence of the service's that ends in Latin punctuation takes a space before the hint in either language
        let said = self.message.as_ref().or(self.body.as_ref());
        if let Some(said) = said {
            f.write_str(tr(": ", "："))?;
            f.write_str(said)?;
        }
        let ends_latin = match said {
            Some(said) if said.ends_with(['.', '!', '?']) => true,
            Some(said) if said.ends_with(['。', '！', '？']) => false,
            _ => {
                f.write_str(tr(".", "。"))?;
                !zh
            }
        };
        if let Some(hint) = self.hint() {
            if ends_latin {
                f.write_str(" ")?;
            }
            f.write_str(hint)?;
        }
        // what helps to look into it, on a line of its own
        let mut notes = Vec::new();
        if let Some(id) = &self.request_id {
            notes.push(format!("{}{id}", tr("Request id ", "请求 ID：")));
        }
        if self.retries > 0 {
            let tried = if notes.is_empty() {
                tr("Tried {n} more times", "已重试 {n} 次")
            } else {
                tr("tried {n} more times", "已重试 {n} 次")
            };
            notes.push(tried.replace("{n}", &self.retries.to_string()));
        }
        if !notes.is_empty() {
            write!(f, "\n{}{}", notes.join(tr("; ", "；")), tr(".", "。"))?;
        }
        Ok(())
    }
}

impl ModelApiError {
    /// An answer of `format` that is not what it should be, quoting its start.
    pub(super) fn unexpected(format: ModelApiFormat, body: &str) -> Self {
        ModelApiError::UnexpectedBody {
            format,
            head: quote(body),
        }
    }

    /// The HTTP status the service answered with, for an error status.
    pub fn status(&self) -> Option<u16> {
        match self {
            ModelApiError::Status(error) => Some(error.status),
            _ => None,
        }
    }
}

impl fmt::Display for ModelApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelApiError::InvalidUrl { endpoint } => {
                write!(f, "{}{endpoint}", tr("Invalid API address: ", "无效的 API 地址："))
            }
            ModelApiError::Timeout { seconds } => f.write_str(
                &tr(
                    "The model API did not answer within {s} seconds.",
                    "模型接口 {s} 秒内没有回应。",
                )
                .replace("{s}", &seconds.to_string()),
            ),
            ModelApiError::Network { cause, retries } => {
                write!(
                    f,
                    "{}{cause}",
                    tr("Cannot reach the model API: ", "连不上模型接口：")
                )?;
                if *retries > 0 {
                    f.write_str(
                        &tr("; tried {n} more times", "；已重试 {n} 次")
                            .replace("{n}", &retries.to_string()),
                    )?;
                }
                Ok(())
            }
            ModelApiError::Status(error) => error.fmt(f),
            ModelApiError::UnexpectedBody { format, head } => {
                let name = match format {
                    ModelApiFormat::OpenAi => "OpenAI",
                    ModelApiFormat::Anthropic => "Anthropic",
                };
                f.write_str(
                    &tr(
                        "The answer is not an {format} answer; check the API format and address: {head}",
                        "回复不是 {format} 格式，请检查接口格式和地址：{head}",
                    )
                    .replace("{format}", name)
                    .replace("{head}", head),
                )
            }
            ModelApiError::Refused { reason } => {
                f.write_str(tr("The model declined to answer", "模型拒绝回答"))?;
                match reason {
                    Some(reason) => write!(f, "{}{reason}", tr(": ", "：")),
                    None => f.write_str(tr(".", "。")),
                }
            }
            ModelApiError::Truncated => f.write_str(tr(
                "The model reached its token limit before it wrote an answer; raise the most tokens of one reply, the setting ai_model_api_max_tokens.",
                "模型在写出回答之前就用完了 token 上限，请调高「单次回复的最多 token」，即设置 ai_model_api_max_tokens。",
            )),
            ModelApiError::Empty => f.write_str(tr("The model answered nothing.", "模型返回为空。")),
        }
    }
}

impl std::error::Error for ModelApiError {}

/// The start of `body` on one line, for an error.
fn quote(body: &str) -> String {
    let line = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut head: String = line.chars().take(QUOTED_CHARS).collect();
    if line.chars().count() > QUOTED_CHARS {
        head.push('…');
    }
    head
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_error_bodies_are_read() {
        let openai = StatusError::read(
            401,
            r#"{"error":{"message":"Incorrect API key provided","type":"invalid_request_error","param":null,"code":"invalid_api_key"}}"#,
            Some("req_o".into()),
        );
        assert_eq!(openai.kind.as_deref(), Some("invalid_request_error"));
        assert_eq!(openai.code.as_deref(), Some("invalid_api_key"));
        assert_eq!(
            openai.to_string(),
            "HTTP 401 invalid_request_error, invalid_api_key: Incorrect API key provided. Check the API key.\nRequest id req_o."
        );
        let anthropic = StatusError::read(
            529,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"},"request_id":"req_a"}"#,
            None,
        );
        assert_eq!(anthropic.request_id.as_deref(), Some("req_a"));
        assert!(anthropic
            .to_string()
            .starts_with("HTTP 529 overloaded_error: Overloaded. The service is unavailable"));
    }

    #[test]
    fn other_bodies_are_quoted_on_one_line() {
        let error = StatusError::read(502, "<html>\n  Bad   gateway\n</html>", None);
        assert_eq!(error.body.as_deref(), Some("<html> Bad gateway </html>"));
        assert_eq!(
            StatusError::read(400, r#"{"error":"bad model"}"#, None)
                .message
                .as_deref(),
            Some("bad model")
        );
        let long = "x".repeat(500);
        assert_eq!(quote(&long).chars().count(), QUOTED_CHARS + 1);
    }

    #[test]
    fn a_missing_model_list_is_not_a_wrong_model() {
        let mut missing = StatusError::read(404, r#"{"error":{"message":"Not Found"}}"#, None);
        assert!(missing.to_string().contains("the model name."), "{missing}");
        missing.request = Request::Models;
        let text = missing.to_string();
        assert!(
            text.starts_with("HTTP 404: Not Found. This address has no list of models"),
            "{text}"
        );
        missing.status = 401;
        assert!(missing.to_string().contains("Check the API key."));
    }

    #[test]
    fn quota_is_told_from_a_rate_limit() {
        let quota = StatusError::read(
            429,
            r#"{"error":{"message":"You exceeded your current quota","type":"insufficient_quota","code":"insufficient_quota"}}"#,
            None,
        );
        assert!(quota.out_of_quota());
        assert!(quota
            .to_string()
            .contains("quota, credit or spend limit is used up"));
        let busy = StatusError::read(
            429,
            r#"{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}"#,
            None,
        );
        assert!(!busy.out_of_quota());
    }
}
