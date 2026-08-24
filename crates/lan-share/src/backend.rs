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
    /// 自定义 OpenAI 兼容端（组员自建/局域网推理服务）。
    Custom,
}

impl Provider {
    pub fn name(&self) -> &'static str {
        match self {
            Provider::Mock => "mock",
            Provider::DeepSeek => "deepseek",
            Provider::Kimi => "kimi",
            Provider::Zhipu => "zhipu",
            Provider::Custom => "custom",
        }
    }

    /// 官方 API base URL（OpenAI 兼容 /chat/completions）。
    pub fn base_url(&self) -> Option<&'static str> {
        match self {
            Provider::Mock => None,
            Provider::DeepSeek => Some("https://api.deepseek.com"),
            Provider::Kimi => Some("https://api.moonshot.cn/v1"),
            Provider::Zhipu => Some("https://open.bigmodel.cn/api/paas/v4"),
            Provider::Custom => None,
        }
    }

    /// 默认模型名。
    pub fn default_model(&self) -> &'static str {
        match self {
            Provider::Mock => "mock-7b",
            Provider::DeepSeek => "deepseek-chat",
            Provider::Kimi => "moonshot-v1-8k",
            Provider::Zhipu => "glm-4-flash",
            Provider::Custom => "custom",
        }
    }

    /// 解析提供商字符串（官方名或自定义标识 → 枚举）。
    pub fn from_str(s: &str) -> Provider {
        match s {
            "mock" => Provider::Mock,
            "deepseek" => Provider::DeepSeek,
            "kimi" => Provider::Kimi,
            "zhipu" => Provider::Zhipu,
            _ => Provider::Custom,
        }
    }

    /// 是否为官方内置提供商（否则为自定义 OpenAI 兼容端）。
    pub fn is_builtin(&self) -> bool {
        !matches!(self, Provider::Custom)
    }
}

/// 执行后端抽象：输入 OpenAI 兼容请求，返回标准响应（含 usage）。
#[async_trait]
pub trait Backend: Send + Sync {
    /// 执行 chat completion（OpenAI 语义）。
    async fn chat(&self, request: &Value) -> RuntimeResult<Value>;
    /// 后端名（健康/诊断；自定义提供方可为动态名）。
    fn name(&self) -> &str;
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
    /// 模型名（兼容单值；新配置用 models）。
    #[serde(default)]
    pub model: Option<String>,
    /// 模型列表（参考 cc-switch：一 provider 多模型，模型目录全量路由）。
    #[serde(default)]
    pub models: Vec<String>,
    /// 自定义 base URL（覆盖提供商默认；可选）。
    #[serde(default)]
    pub base_url: Option<String>,
    /// 请求超时（秒）。
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// 展示名（面板显示/路由键；默认用提供商名）。
    #[serde(default)]
    pub name: Option<String>,
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
    /// 注册名（默认提供商名；自定义端用其标识）。
    name: String,
}

impl OpenAICompatBackend {
    pub fn new(cfg: OpenAICompatConfig) -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .unwrap_or_default();
        let name = cfg.name.clone().unwrap_or_else(|| cfg.provider.name().to_string());
        Self { cfg, client, name }
    }
}

#[async_trait]
impl Backend for OpenAICompatBackend {
    fn name(&self) -> &str {
        &self.name
    }

    fn provider(&self) -> Provider {
        self.cfg.provider
    }

    fn models(&self) -> Vec<String> {
        // 配置模型列表优先；兼容单值 model；再回退提供商默认
        let from_list: Vec<String> = self.cfg.models.iter().filter(|m| !m.is_empty()).cloned().collect();
        if !from_list.is_empty() { return from_list; }
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

/// 配置文件（data_dir/backends.yaml）中的后端条目。
///
/// 对齐 DeepSeek Harness 的配置方式：providers 列表，
/// 密钥支持直填（api_key）或环境变量引用（api_key_env，推荐，避免明文落盘）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendEntry {
    /// 提供商标识：mock / deepseek / kimi / zhipu 或任意自定义端名。
    pub provider: String,
    /// 显示名（路由键；默认取 provider）。
    #[serde(default)]
    pub id: Option<String>,
    /// 直填 API 密钥（credential）。
    #[serde(default)]
    pub api_key: Option<String>,
    /// 环境变量引用 API 密钥（credential-ref）。
    #[serde(default)]
    pub api_key_env: Option<String>,
    /// 模型名（兼容旧配置；新配置请用 models 数组）。
    #[serde(default)]
    pub model: Option<String>,
    /// 标准模型列表（参考 cc-switch 添加模型：提供方带官方模型清单，可增删）。
    #[serde(default)]
    pub models: Vec<String>,
    /// 自定义 base URL（自定义提供方必填；官方可覆盖）。
    #[serde(default)]
    pub base_url: Option<String>,
}

impl BackendEntry {
    /// 路由键：id 或 provider。
    pub fn backend_id(&self) -> String {
        self.id.clone().unwrap_or_else(|| self.provider.clone())
    }

    /// 生效模型集合：models 数组优先；其次兼容单值 model；空则用提供商默认（调用方兜底）。
    pub fn effective_models(&self) -> Vec<String> {
        let from_list: Vec<String> = self.models.iter().filter(|m| !m.is_empty()).cloned().collect();
        if !from_list.is_empty() { return from_list; }
        match &self.model {
            Some(m) if !m.is_empty() => vec![m.clone()],
            _ => Vec::new(),
        }
    }

    /// 解析生效 API key：直填 > 环境变量引用 > 提供商官方环境变量兜底。
    pub fn resolve_api_key(&self) -> Option<String> {
        if let Some(k) = &self.api_key {
            if !k.is_empty() { return Some(k.clone()); }
        }
        if let Some(env) = &self.api_key_env {
            if !env.is_empty() {
                if let Ok(v) = std::env::var(env) {
                    if !v.is_empty() { return Some(v); }
                }
            }
        }
        let official = format!("AIPOWERLINK_{}_API_KEY", self.provider.to_uppercase());
        if let Ok(v) = std::env::var(official) {
            if !v.is_empty() { return Some(v); }
        }
        std::env::var("AIPOWERLINK_API_KEY").ok().filter(|k| !k.is_empty())
    }

    /// 密钥来源（面板展示）。
    pub fn key_source(&self) -> &'static str {
        if let Some(k) = &self.api_key {
            if !k.is_empty() { return "file"; }
        }
        if let Some(e) = &self.api_key_env {
            if !e.is_empty() && std::env::var(e).map(|v| !v.is_empty()).unwrap_or(false) {
                return "env";
            }
        }
        let official = format!("AIPOWERLINK_{}_API_KEY", self.provider.to_uppercase());
        if std::env::var(official).map(|v| !v.is_empty()).unwrap_or(false) {
            return "env";
        }
        if std::env::var("AIPOWERLINK_API_KEY").map(|v| !v.is_empty()).unwrap_or(false) {
            return "env";
        }
        "none"
    }

    /// 掩码展示密钥（面板不回传明文）：sk-***1234 或 env:NAME。
    pub fn masked_key(&self) -> String {
        if let Some(k) = &self.api_key {
            if !k.is_empty() {
                if k.chars().count() > 7 {
                    let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                    return format!("***{tail}");
                }
                return "***".to_string();
            }
        }
        if let Some(e) = &self.api_key_env {
            if !e.is_empty() { return format!("env:{e}"); }
        }
        let official = format!("AIPOWERLINK_{}_API_KEY", self.provider.to_uppercase());
        if std::env::var(&official).is_ok() { return format!("env:{official}"); }
        if std::env::var("AIPOWERLINK_API_KEY").is_ok() { return "env:AIPOWERLINK_API_KEY".to_string(); }
        String::new()
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
            models: Vec::new(),
            base_url: None,
            timeout_secs: 60,
            name: None,
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
            models: Vec::new(),
            base_url: Some("http://127.0.0.1:9999/v1".into()),
            timeout_secs: 60,
            name: None,
        };
        assert_eq!(cfg.completions_url(), "http://127.0.0.1:9999/v1/chat/completions");
        assert_eq!(cfg.effective_model(), "custom-model");
    }
}