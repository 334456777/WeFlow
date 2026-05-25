use serde::Serialize;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Clone, Serialize)]
pub struct ErrorPayload {
    pub code: String,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: String,
    pub message: String,
    pub exit_code: i32,
    pub details: Option<Value>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, exit_code: i32) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            exit_code,
            details: None,
        }
    }

    pub fn runtime(message: impl Into<String>) -> Self {
        Self::new("runtime_error", message, 1)
    }

    pub fn usage(message: impl Into<String>) -> Self {
        Self::new("invalid_arguments", message, 2)
    }

    pub fn config(message: impl Into<String>) -> Self {
        Self::new("config_error", message, 3)
    }

    pub fn native(message: impl Into<String>) -> Self {
        Self::new("native_error", message, 4)
    }

    pub fn with_details(mut self, details: Value) -> Self {
        self.details = Some(details);
        self
    }

    pub fn payload(&self) -> ErrorPayload {
        ErrorPayload {
            code: self.code.clone(),
            message: self.message.clone(),
            details: self.details.clone(),
        }
    }
}

impl From<anyhow::Error> for AppError {
    fn from(value: anyhow::Error) -> Self {
        Self::runtime(value.to_string())
    }
}
