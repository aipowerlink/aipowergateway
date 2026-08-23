//! 执行后端：驱动本机算力/官方大模型 API 执行请求。
//!
//! - `MockBackend`：0.1.0 本地验证（无外部依赖）
//! - `OpenAICompatBackend`：转发官方 OpenAI 兼容 API（DeepSeek / Kimi / 智谱 GLM）

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use aipg_runtime::RuntimeResult;

/// 官方大模型提供商（均为 OpenAI 兼容 API）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// 本地 mock（0.1.0 验证）。
    Mock,
    /// DeepSeek 官方（https://api.deepseek.com）。
    DeepSeek,
    /// Kimi / 月之暗面（https://api.moonshot.cn/v1）。
    Kimi,
    /// 智谱 GLM（https://open.bigmodel.cn/api/paas/v4）。
    Zhipu,
}

impl Provider {
    pub fn name(&self) -> &'static str {
        match self {
            Provider::Mock => "mock",
            Provider::DeepSeek => "deepseek",
            Provider::Kimi => "kimi",
            Provider::Zhipu => "zhipu",
        }
    }

    /// 官方 API base URL（OpenAI 兼容 /chat/completions）。
    pub fn base_url(&self) -> Option<&'static str> {
        match self {
            Provider::Mock => None,
            Provider::DeepSeek => Some("https://api.deepseek.com"),
            Provider::Kimi => Some("https://api.moonshot.cn/v1"),
            Provider::Zhipu => Some("https://open.bigmodel.cn/api/paas/v4"),
        }
    }

    /// 默认模型名。
    pub fn default_model(&self) -> &'static str {
        match self {
            Provider::Mock => "mock-7b",
            Provider::DeepSeek => "deepseek-chat",
            Provider::Kimi => "moonshot-v1-8k",
            Provider::Zhipu => "glm-4-flash",
        }
    }
}

/// 执行后端抽象：输入 OpenAI 兼容请求，返回标准响应（含 usage）。
#[async_trait]
pub trait Backend: Send + Sync {
    /// 执行 chat completion（OpenAI 语义）。
    async fn chat(&self, request: &Value) -> RuntimeResult<Value>;
    /// 后端名（健康/诊断）。
    fn name(&self) -> &'static str;
    /// 提供商。
    fn provider(&self) -> Provider;
    /// 该后端提供的模型列表（模型目录）。
    fn models(&self) -> Vec<String>;
}

/// Mock 执行后端：0.1.0 验证链路。
pub struct MockBackend {
    /// 模型名。
    pub model: &'static str,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self { model: "mock-7b" }
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    fn provider(&self) -> Provider {
        Provider::Mock
    }

    fn models(&self) -> Vec<String> {
        vec![self.model.to_string()]
    }

    async fn chat(&self, request: &Value) -> RuntimeResult<Value> {
        let user_msg = request
            .get("messages")
            .and_then(|m| m.as_array())
            .and_then(|arr| arr.last())
            .and_then(|last| last.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        let reply = format!("mock reply to: {}", truncate(user_msg, 80));
        let completion_tokens = (reply.chars().count() / 2).max(1) as u64;
        let prompt_tokens = (user_msg.chars().count() / 2).max(1) as u64;

        Ok(json!({
            "id": "chatcmpl-mock-0001",
            "object": "chat.completion",
            "created": 0,
            "model": self.model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": reply,
                },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
            },
        }))
    }
}

/// OpenAI 兼容后端配置（官方大模型）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenAICompatConfig {
    /// 提供商。
    pub provider: Provider,
    /// 官方 API key（Vault 加密存储，不回传明文）。
    pub api_key: String,
    /// 模型名（默认用提供商默认模型）。
    #[serde(default)]
    pub model: Option<String>,
    /// 自定义 base URL（覆盖提供商默认；可选）。
    #[serde(default)]
    pub base_url: Option<String>,
    /// 请求超时（秒）。
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_timeout() -> u64 { 60 }

impl OpenAICompatConfig {
    /// 请求完整 URL（base + /chat/completions）。
    pub fn completions_url(&self) -> String {
        let base = self.base_url.clone()
            .or_else(|| self.provider.base_url().map(|s| s.to_string()))
            .unwrap_or_default();
        format!("{}/chat/completions", base.trim_end_matches('/'))
    }

    /// 生效模型名。
    pub fn effective_model(&self) -> String {
        self.model.clone().unwrap_or_else(|| self.provider.default_model().to_string())
    }
}

/// OpenAI 兼容后端：转发官方 API（DeepSeek / Kimi / 智谱）。
pub struct OpenAICompatBackend {
    cfg: OpenAICompatConfig,
    client: reqwest::Client,
}

impl OpenAICompatBackend {
    pub fn new(cfg: OpenAICompatConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .unwrap_or_default();
        Self { cfg, client }
    }
}

#[async_trait]
impl Backend for OpenAICompatBackend {
    fn name(&self) -> &'static str {
        self.cfg.provider.name()
    }

    fn provider(&self) -> Provider {
        self.cfg.provider
    }

    fn models(&self) -> Vec<String> {
        // 配置模型优先；否则提供商默认模型
        match &self.cfg.model {
            Some(m) if !m.is_empty() => vec![m.clone()],
            _ => vec![self.cfg.provider.default_model().to_string()],
        }
    }

    async fn chat(&self, request: &Value) -> RuntimeResult<Value> {
        let url = self.cfg.completions_url();
        let mut body = request.clone();
        let has_model = body.get("model").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        if !has_model {
            body["model"] = json!(self.cfg.effective_model());
        }
        let resp = self.client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.cfg.api_key))
            .json(&body)
            .send()
            .await
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("upstream request: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| aipg_runtime::RuntimeError::Other(format!("upstream body: {e}")))?;
        if !status.is_success() {
            return Err(aipg_runtime::RuntimeError::Other(format!("upstream {}: {}", status, truncate(&text, 300))));
        }
        serde_json::from_str(&text).map_err(|e| aipg_runtime::RuntimeError::Other(format!("upstream json: {e}")))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_have_official_urls() {
        assert!(Provider::DeepSeek.base_url().unwrap().contains("deepseek.com"));
        assert!(Provider::Kimi.base_url().unwrap().contains("moonshot.cn"));
        assert!(Provider::Zhipu.base_url().unwrap().contains("bigmodel.cn"));
    }

    #[test]
    fn completions_url_built() {
        let cfg = OpenAICompatConfig {
            provider: Provider::DeepSeek,
            api_key: "sk-test".into(),
            model: None,
            base_url: None,
            timeout_secs: 60,
        };
        assert_eq!(cfg.completions_url(), "https://api.deepseek.com/chat/completions");
        assert_eq!(cfg.effective_model(), "deepseek-chat");
    }

    #[test]
    fn custom_url_overrides() {
        let cfg = OpenAICompatConfig {
            provider: Provider::DeepSeek,
            api_key: "sk-test".into(),
            model: Some("custom-model".into()),
            base_url: Some("http://127.0.0.1:9999/v1".into()),
            timeout_secs: 60,
        };
        assert_eq!(cfg.completions_url(), "http://127.0.0.1:9999/v1/chat/completions");
        assert_eq!(cfg.effective_model(), "custom-model");
    }
}