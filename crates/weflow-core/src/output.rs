use serde::Serialize;
use serde_json::Value;

use crate::error::ErrorPayload;

#[derive(Debug, Serialize)]
#[serde(untagged)]
pub enum CliResponse<T: Serialize> {
    Success {
        success: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<T>,
        #[serde(skip_serializing_if = "Option::is_none")]
        meta: Option<Value>,
    },
    Failure {
        success: bool,
        error: ErrorPayload,
    },
}

pub fn success<T: Serialize>(data: T) -> CliResponse<T> {
    CliResponse::Success {
        success: true,
        data: Some(data),
        meta: None,
    }
}

pub fn success_with_meta<T: Serialize>(data: T, meta: Value) -> CliResponse<T> {
    CliResponse::Success {
        success: true,
        data: Some(data),
        meta: Some(meta),
    }
}

pub fn ok() -> CliResponse<Value> {
    CliResponse::Success {
        success: true,
        data: Some(serde_json::json!({ "ok": true })),
        meta: None,
    }
}

pub fn failure(error: ErrorPayload) -> CliResponse<Value> {
    CliResponse::Failure {
        success: false,
        error,
    }
}

pub fn progress(stage: &str, message: &str, current: usize, total: usize) {
    let percent = if total > 0 {
        ((current as f64 / total as f64) * 100.0) as u8
    } else {
        0
    };
    eprintln!(
        "{}",
        serde_json::json!({
            "type": "progress",
            "stage": stage,
            "message": message,
            "current": current,
            "total": total,
            "percent": percent
        })
    );
}
