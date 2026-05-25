use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::broadcast;

pub struct EventChannel {
    sender: broadcast::Sender<Value>,
}

impl EventChannel {
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }

    pub fn sender(&self) -> broadcast::Sender<Value> {
        self.sender.clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.sender.subscribe()
    }
}

pub async fn message_push_loop(
    hub: crate::services::ServiceHub,
    sender: broadcast::Sender<Value>,
    interval_secs: u64,
) {
    let mut baseline: Vec<String> = Vec::new();
    loop {
        tokio::time::sleep(Duration::from_secs(interval_secs)).await;
        match hub.sessions() {
            Ok(sessions) => {
                let current = extract_session_ids(&sessions);
                for session_id in &current {
                    if !baseline.contains(session_id) {
                        let event = json!({
                            "type": "new_message",
                            "sessionId": session_id,
                            "timestamp": std::time::SystemTime::now()
                                .duration_since(std::time::UNIX_EPOCH)
                                .unwrap_or_default()
                                .as_millis()
                        });
                        let _ = sender.send(event);
                    }
                }
                baseline = current;
            }
            Err(_) => {
                let _ = sender.send(json!({
                    "type": "server_status",
                    "message": "failed to poll sessions",
                    "timestamp": std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_millis()
                }));
            }
        }
    }
}

fn extract_session_ids(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    for key in ["session_id", "sessionId", "username", "userName", "talker"] {
                        if let Some(text) = item.get(key).and_then(Value::as_str) {
                            if !text.trim().is_empty() {
                                return Some(text.to_string());
                            }
                        }
                    }
                    None
                })
                .collect()
        })
        .unwrap_or_default()
}
