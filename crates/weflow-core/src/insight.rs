use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::ProfileConfig;
use crate::error::{AppError, AppResult};

const DEFAULT_SYSTEM_PROMPT: &str = "你是用户的私人关系观察助手，名叫\"见解\"。你的任务是主动提供有价值的观察和建议。

要求：
1. 必须给出见解。基于聊天记录分析对方情绪、话题趋势、关系动态，或给出回复建议、聊天话题推荐。
2. 控制在 80 字以内，直接、具体、一针见血。不要废话。
3. 输出纯文本，不使用 Markdown。
4. 只有在完全没有任何可说的内容时（比如对话只有一条\"嗯\"），才回复\"SKIP\"。绝大多数情况下你应该输出见解。";

const API_TIMEOUT_SECS: u64 = 45;
const API_TEMPERATURE: f32 = 0.7;
const API_MAX_TOKENS_DEFAULT: u32 = 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InsightRecord {
    pub id: String,
    pub created_at: u64,
    pub session_id: String,
    pub display_name: String,
    pub trigger_reason: String,
    pub insight: String,
    pub read: bool,
}

#[derive(Debug, Clone)]
pub struct AiModelConfig {
    pub api_base_url: String,
    pub api_key: String,
    pub model: String,
    pub max_tokens: u32,
}

pub struct InsightStore {
    path: PathBuf,
    records: Vec<InsightRecord>,
}

impl InsightStore {
    pub fn load(home_dir: &Path) -> AppResult<Self> {
        let path = home_dir.join("insight_records.json");
        let records = if path.exists() {
            let raw = fs::read_to_string(&path).map_err(|err| {
                AppError::runtime(format!("failed to read insight records: {err}"))
            })?;
            serde_json::from_str(&raw).unwrap_or_default()
        } else {
            Vec::new()
        };
        Ok(Self { path, records })
    }

    pub fn save(&self) -> AppResult<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).map_err(|err| {
                AppError::runtime(format!("failed to create {}: {err}", parent.display()))
            })?;
        }
        let bytes = serde_json::to_vec_pretty(&self.records).map_err(|err| {
            AppError::runtime(format!("failed to serialize insight records: {err}"))
        })?;
        fs::write(&self.path, bytes).map_err(|err| {
            AppError::runtime(format!("failed to write {}: {err}", self.path.display()))
        })
    }

    pub fn records(&self) -> &[InsightRecord] {
        &self.records
    }

    pub fn get(&self, id: &str) -> Option<&InsightRecord> {
        self.records.iter().find(|r| r.id == id)
    }

    pub fn add(&mut self, record: InsightRecord) {
        self.records.push(record);
    }

    pub fn mark_read(&mut self, id: &str) -> bool {
        if let Some(record) = self.records.iter_mut().find(|r| r.id == id) {
            record.read = true;
            return true;
        }
        false
    }

    pub fn clear(&mut self) {
        self.records.clear();
    }
}

pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

pub fn build_api_url(base_url: &str, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let suffix = if path.starts_with('/') { path } else { &format!("/{path}") };
    format!("{base}{suffix}")
}

pub fn extract_ai_config(profile: &ProfileConfig) -> AppResult<AiModelConfig> {
    let base_url = profile
        .ai_model_api_base_url
        .clone()
        .ok_or_else(|| AppError::config("ai_model_api_base_url not configured; run config set ai_model_api_base_url <url>"))?;
    let api_key = profile
        .ai_model_api_key
        .clone()
        .ok_or_else(|| AppError::config("ai_model_api_key not configured; run config set ai_model_api_key <key>"))?;
    let model = profile
        .ai_model_api_model
        .clone()
        .unwrap_or_else(|| "gpt-4o-mini".to_string());
    let max_tokens = profile.ai_model_api_max_tokens.unwrap_or(API_MAX_TOKENS_DEFAULT);
    Ok(AiModelConfig {
        api_base_url: base_url,
        api_key,
        model,
        max_tokens,
    })
}

pub async fn call_ai_api(
    config: &AiModelConfig,
    system_prompt: &str,
    user_prompt: &str,
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(API_TIMEOUT_SECS))
        .build()?;
    let url = build_api_url(&config.api_base_url, "/v1/chat/completions");
    let body = json!({
        "model": config.model,
        "messages": [
            { "role": "system", "content": system_prompt },
            { "role": "user", "content": user_prompt }
        ],
        "max_tokens": config.max_tokens,
        "temperature": API_TEMPERATURE
    });
    let response = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", config.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await?;
    if !response.status().is_success() {
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        return Err(anyhow!("AI API returned {status}: {text}"));
    }
    let resp_json: Value = response.json().await?;
    let content = resp_json
        .pointer("/choices/0/message/content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(content)
}

pub async fn test_ai_connection(config: &AiModelConfig) -> AppResult<Value> {
    match call_ai_api(config, "You are a test assistant.", "Say OK").await {
        Ok(response) => Ok(json!({
            "success": true,
            "message": format!("Connection OK, model responded: {}", response.chars().take(50).collect::<String>())
        })),
        Err(err) => Ok(json!({
            "success": false,
            "message": err.to_string()
        })),
    }
}

pub async fn generate_insight(
    config: &AiModelConfig,
    session_id: &str,
    display_name: &str,
    messages_text: &str,
    trigger_reason: &str,
) -> AppResult<String> {
    let user_prompt = format!(
        "联系人：{}（{}）\n触发原因：{}\n\n最近聊天记录：\n{}\n\n请给出你的见解（≤80字）：",
        display_name, session_id, trigger_reason, messages_text
    );
    let result = call_ai_api(config, DEFAULT_SYSTEM_PROMPT, &user_prompt)
        .await
        .map_err(|err| AppError::runtime(format!("AI insight generation failed: {err}")))?;
    if result.trim().eq_ignore_ascii_case("SKIP") {
        return Ok(String::new());
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_api_url() {
        assert_eq!(
            build_api_url("https://api.openai.com", "/v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
        assert_eq!(
            build_api_url("https://api.openai.com/", "v1/chat/completions"),
            "https://api.openai.com/v1/chat/completions"
        );
    }

    #[test]
    fn insight_store_crud() {
        let dir = std::env::temp_dir().join(format!("weflow-insight-test-{}", now_millis()));
        std::fs::create_dir_all(&dir).unwrap();

        let mut store = InsightStore::load(&dir).unwrap();
        assert!(store.records().is_empty());

        store.add(InsightRecord {
            id: "rec1".to_string(),
            created_at: 1000,
            session_id: "wxid_test".to_string(),
            display_name: "Test".to_string(),
            trigger_reason: "manual".to_string(),
            insight: "Hello".to_string(),
            read: false,
        });
        store.save().unwrap();

        let loaded = InsightStore::load(&dir).unwrap();
        assert_eq!(loaded.records().len(), 1);
        assert_eq!(loaded.records()[0].id, "rec1");

        let dir = dir.clone();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn extracts_ai_config() {
        let mut profile = ProfileConfig::default();
        assert!(extract_ai_config(&profile).is_err());

        profile.ai_model_api_base_url = Some("https://api.test.com".to_string());
        profile.ai_model_api_key = Some("sk-test".to_string());
        profile.ai_model_api_model = Some("gpt-4".to_string());

        let config = extract_ai_config(&profile).unwrap();
        assert_eq!(config.api_base_url, "https://api.test.com");
        assert_eq!(config.api_key, "sk-test");
        assert_eq!(config.model, "gpt-4");
        assert_eq!(config.max_tokens, API_MAX_TOKENS_DEFAULT);
    }
}
