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

#[derive(Debug)]
pub struct AppErrorInner {
    pub code: String,
    pub message: String,
    pub exit_code: i32,
    pub details: Option<Value>,
}

/// The error of every service call. The payload is boxed so that `Result<T, AppError>` stays one pointer wide
/// instead of over 128 bytes; the fields are still read and written through `Deref`/`DerefMut`
/// (`err.exit_code`, `err.details = …`), but cannot be moved out that way: use [`AppError::into_message`].
#[derive(Debug, Error)]
#[error("{}: {}", .0.code, .0.message)]
pub struct AppError(Box<AppErrorInner>);

impl std::ops::Deref for AppError {
    type Target = AppErrorInner;

    fn deref(&self) -> &AppErrorInner {
        &self.0
    }
}

impl std::ops::DerefMut for AppError {
    fn deref_mut(&mut self) -> &mut AppErrorInner {
        &mut self.0
    }
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: impl Into<String>, message: impl Into<String>, exit_code: i32) -> Self {
        Self(Box::new(AppErrorInner {
            code: code.into(),
            message: crate::locale::localize(message.into()),
            exit_code,
            details: None,
        }))
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

    /// Moves the message out (fields cannot be moved out through `Deref`).
    pub fn into_message(self) -> String {
        self.0.message
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

#[cfg(test)]
mod tests {
    use std::mem::size_of;

    use serde_json::json;

    use super::*;

    #[test]
    fn constructors_set_code_and_exit_code() {
        for (err, code, exit) in [
            (AppError::runtime("boom 1"), "runtime_error", 1),
            (AppError::usage("boom 2"), "invalid_arguments", 2),
            (AppError::config("boom 3"), "config_error", 3),
            (AppError::native("boom 4"), "native_error", 4),
            (AppError::new("custom", "boom 5", 9), "custom", 9),
        ] {
            assert_eq!(err.code, code);
            assert_eq!(err.exit_code, exit);
            assert!(err.details.is_none());
        }
    }

    #[test]
    fn display_is_code_then_message() {
        let err = AppError::native("boom 6");
        assert_eq!(err.to_string(), "native_error: boom 6");
        assert!(format!("{err:?}").contains("boom 6"));
        let boxed: Box<dyn std::error::Error + Send + Sync> = Box::new(AppError::config("boom 7"));
        assert_eq!(boxed.to_string(), "config_error: boom 7");
    }

    #[test]
    fn payload_copies_every_field_and_skips_missing_details() {
        let plain = AppError::runtime("boom 8").payload();
        assert_eq!(
            serde_json::to_value(&plain).unwrap(),
            json!({ "code": "runtime_error", "message": "boom 8" })
        );
        let detailed = AppError::usage("boom 9").with_details(json!({ "field": "x" }));
        assert_eq!(
            serde_json::to_value(detailed.payload()).unwrap(),
            json!({ "code": "invalid_arguments", "message": "boom 9", "details": { "field": "x" } })
        );
    }

    #[test]
    fn fields_can_be_read_and_written_through_deref() {
        let mut err = AppError::runtime("boom 10");
        err.exit_code = 42;
        err.code = "changed".into();
        err.details = Some(json!(1));
        assert_eq!((err.exit_code, err.code.as_str()), (42, "changed"));
        assert_eq!(err.details.as_ref(), Some(&json!(1)));
        assert_eq!(err.into_message(), "boom 10");
    }

    #[test]
    fn converts_from_anyhow() {
        let err: AppError = anyhow::anyhow!("boom 11").into();
        assert_eq!((err.code.as_str(), err.exit_code), ("runtime_error", 1));
        assert_eq!(err.message, "boom 11");
    }

    #[test]
    fn results_stay_one_pointer_wide() {
        // Guards the point of boxing: a large error type makes every Result in the services large.
        assert_eq!(size_of::<AppError>(), size_of::<usize>());
        assert_eq!(size_of::<AppResult<()>>(), size_of::<usize>());
        assert!(size_of::<AppResult<String>>() <= 3 * size_of::<usize>());
    }

    #[test]
    fn is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<AppError>();
    }
}
