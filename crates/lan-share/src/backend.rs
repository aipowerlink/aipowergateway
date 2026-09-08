//! 执行后端：驱动本机算力/官方大模型 API 执行请求。
//!
//! - `MockBackend`：0.1.0 本地验证（无外部依赖）
//! - `OpenAICompatBackend`：转发官方 OpenAI 兼容 API（DeepSeek / Kimi / 智谱 GLM / CodeBuddy）

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
    /// CodeBuddy（腾讯 Copilot，https://copilot.tencent.com/v2）。
    CodeBuddy,
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
            Provider::CodeBuddy => "codebuddy",
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
            Provider::CodeBuddy => Some("https://copilot.tencent.com/v2"),
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
            Provider::CodeBuddy => "deepseek-v4-flash",
            Provider::Custom => "custom",
        }
    }

    /// 内置模型目录（配置未显式指定时兜底；CodeBuddy 无 /models 端点，目录即官方清单）。
    pub fn default_models(&self) -> Vec<&'static str> {
        match self {
            Provider::CodeBuddy => vec!["hy4-preview", "deepseek-v4-flash"],
            _ => vec![self.default_model()],
        }
    }

    /// 上游 User-Agent（官方 OpenAI 兼容端无需显式 UA；CodeBuddy 网关要求自定义 UA）。
    pub fn user_agent(&self) -> Option<String> {
        match self {
            // 腾讯 Copilot WAF：识别浏览器/常规客户端 UA；aipowergateway/{版本} 已验证可通过
            Provider::CodeBuddy => Some(format!("aipowergateway/{}", env!("CARGO_PKG_VERSION"))),
            _ => None,
        }
    }

    /// 解析提供商字符串（官方名或自定义标识 → 枚举）。
    pub fn from_str(s: &str) -> Provider {
        match s {
            "mock" => Provider::Mock,
            "deepseek" => Provider::DeepSeek,
            "kimi" => Provider::Kimi,
            "zhipu" => Provider::Zhipu,
            "codebuddy" => Provider::CodeBuddy,
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
        let mut builder = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs.max(1)));
        if let Some(ua) = cfg.provider.user_agent() {
            builder = builder.user_agent(ua);
        }
        let client = builder.build().unwrap_or_default();
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
        // 配置模型列表优先；兼容单值 model；再回退提供商内置目录
        let from_list: Vec<String> = self.cfg.models.iter().filter(|m| !m.is_empty()).cloned().collect();
        if !from_list.is_empty() { return from_list; }
        match &self.cfg.model {
            Some(m) if !m.is_empty() => vec![m.clone()],
            _ => self.cfg.provider.default_models().into_iter().map(|s| s.to_string()).collect(),
        }
    }

    async fn chat(&self, request: &Value) -> RuntimeResult<Value> {
        let url = self.cfg.completions_url();
        let mut body = request.clone();
        let has_model = body.get("model").and_then(|v| v.as_str()).map(|s| !s.is_empty()).unwrap_or(false);
        if !has_model {
            body["model"] = json!(self.cfg.effective_model());
        }
        if self.cfg.provider == Provider::CodeBuddy {
            return self.chat_codebuddy_stream(&url, &body).await;
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

impl OpenAICompatBackend {
    /// CodeBuddy 专用通道：腾讯 Copilot 网关仅支持流式（非流式返回 400「Non-stream chat request is currently not supported」），
    /// 因此强制 `stream:true`（+ `stream_options.include_usage`）上行，再从 SSE 组装回标准非流式响应。
    async fn chat_codebuddy_stream(&self, url: &str, body: &Value) -> RuntimeResult<Value> {
        let mut upstream = body.clone();
        upstream["stream"] = json!(true);
        upstream["stream_options"] = json!({ "include_usage": true });
        let resp = self.client
            .post(url)
            .header("Authorization", format!("Bearer {}", self.cfg.api_key))
            .json(&upstream)
            .send()
            .await
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("upstream request: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| aipg_runtime::RuntimeError::Other(format!("upstream body: {e}")))?;
        if !status.is_success() {
            let detail = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("msg").and_then(|m| m.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| truncate(&text, 300));
            return Err(aipg_runtime::RuntimeError::Other(format!("upstream {}: {}", status, detail)));
        }
        assemble_sse_response(&text)
    }
}

/// 解析 OpenAI 兼容 SSE 文本（`data: {json}` 行 + `data: [DONE]`），组装为单块 `chat.completion` JSON。
/// 支持 `content` 与 `reasoning_content` 增量拼接；`finish_reason` 取第一个非空块；`usage` 取非空 usage 块。
fn assemble_sse_response(body: &str) -> RuntimeResult<Value> {
    let mut id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut created: Option<u64> = None;
    let mut content = String::new();
    let mut reasoning = String::new();
    let mut finish_reason = "stop".to_string();
    let mut usage: Option<Value> = None;

    for line in body.lines() {
        let line = line.trim();
        if !line.starts_with("data:") { continue; }
        let payload = line[5..].trim();
        if payload == "[DONE]" { break; }
        let chunk: Value = serde_json::from_str(payload)
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("upstream sse json: {e}")))?;
        if id.is_none() { id = chunk.get("id").and_then(|v| v.as_str()).map(|s| s.to_string()); }
        if model.is_none() { model = chunk.get("model").and_then(|v| v.as_str()).map(|s| s.to_string()); }
        if created.is_none() { created = chunk.get("created").and_then(|v| v.as_u64()); }
        if let Some(delta) = chunk.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("delta")) {
            if let Some(c) = delta.get("content").and_then(|v| v.as_str()) { content.push_str(c); }
            if let Some(r) = delta.get("reasoning_content").and_then(|v| v.as_str()) { reasoning.push_str(r); }
        }
        if let Some(fr) = chunk.get("choices").and_then(|c| c.get(0)).and_then(|ch| ch.get("finish_reason")).and_then(|v| v.as_str()) {
            if !fr.is_empty() { finish_reason = fr.to_string(); }
        }
        if let Some(u) = chunk.get("usage") {
            if !u.is_null() { usage = Some(u.clone()); }
        }
    }

    let mut message = json!({ "role": "assistant", "content": content });
    if !reasoning.is_empty() {
        message["reasoning_content"] = json!(reasoning);
    }
    let mut out = json!({
        "id": id.unwrap_or_else(|| "chatcmpl-aipg-codebuddy".to_string()),
        "object": "chat.completion",
        "created": created.unwrap_or(0),
        "model": model.unwrap_or_default(),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason,
        }],
    });
    if let Some(u) = usage {
        out["usage"] = u;
    }
    Ok(out)
}

/// 配置文件（data_dir/backends.yaml）中的后端条目。
///
/// 对齐 DeepSeek Harness 的配置方式：providers 列表，
/// 密钥支持直填（api_key）或环境变量引用（api_key_env，推荐，避免明文落盘）。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BackendEntry {
    /// 提供商标识：mock / deepseek / kimi / zhipu / codebuddy 或任意自定义端名。
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
        // CodeBuddy 兼容 DSH 的凭证变量名（CODEBUDDY_API_KEY）
        if self.provider == "codebuddy" {
            if let Ok(v) = std::env::var("CODEBUDDY_API_KEY") {
                if !v.is_empty() { return Some(v); }
            }
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
        assert!(Provider::CodeBuddy.base_url().unwrap().contains("copilot.tencent.com"));
    }

    #[test]
    fn codebuddy_defaults() {
        assert_eq!(Provider::from_str("codebuddy"), Provider::CodeBuddy);
        assert_eq!(Provider::CodeBuddy.name(), "codebuddy");
        assert_eq!(Provider::CodeBuddy.default_model(), "deepseek-v4-flash");
        assert_eq!(Provider::CodeBuddy.default_models(), vec!["hy4-preview", "deepseek-v4-flash"]);
        assert!(Provider::CodeBuddy.is_builtin());
        let ua = Provider::CodeBuddy.user_agent().expect("codebuddy UA");
        assert!(ua.starts_with("aipowergateway/") && ua.ends_with(env!("CARGO_PKG_VERSION")), "UA={ua}");
        assert!(Provider::DeepSeek.user_agent().is_none(), "官方 OpenAI 兼容端无需显式 UA");
        // 其他官方提供方：目录回退到默认模型单元素
        assert_eq!(Provider::Zhipu.default_models(), vec!["glm-4-flash"]);
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

    #[test]
    fn codebuddy_completions_url() {
        let cfg = OpenAICompatConfig {
            provider: Provider::CodeBuddy,
            api_key: "ck-test".into(),
            model: None,
            models: Vec::new(),
            base_url: None,
            timeout_secs: 60,
            name: Some("cb".into()),
        };
        assert_eq!(cfg.completions_url(), "https://copilot.tencent.com/v2/chat/completions");
        assert_eq!(cfg.effective_model(), "deepseek-v4-flash");
        // 未显式配置模型 → 内置目录（hy4-preview / deepseek-v4-flash）
        let backend = OpenAICompatBackend::new(cfg);
        assert_eq!(backend.models(), vec!["hy4-preview", "deepseek-v4-flash"]);
        assert_eq!(backend.provider(), Provider::CodeBuddy);
    }

    #[test]
    fn assemble_sse_response_flash() {
        let sse = concat!(
            "data: {\"id\":\"cmb-x1\",\"model\":\"deepseek-v4-flash\",\"object\":\"chat.completion.chunk\",\"created\":1730000000,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"},\"logprobs\":null,\"finish_reason\":\"\"}],\"usage\":null}\n",
            "data: {\"id\":\"cmb-x1\",\"model\":\"deepseek-v4-flash\",\"object\":\"chat.completion.chunk\",\"created\":1730000000,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"logprobs\":null,\"finish_reason\":\"\"}],\"usage\":null}\n",
            "data: {\"id\":\"cmb-x1\",\"model\":\"deepseek-v4-flash\",\"object\":\"chat.completion.chunk\",\"created\":1730000000,\"choices\":[{\"index\":0,\"delta\":{},\"logprobs\":null,\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":1,\"total_tokens\":10,\"reasoning_tokens\":0}}\n",
            "data: [DONE]\n",
        );
        let out = assemble_sse_response(sse).expect("assemble");
        assert_eq!(out["id"], "cmb-x1");
        assert_eq!(out["model"], "deepseek-v4-flash");
        assert_eq!(out["object"], "chat.completion");
        assert_eq!(out["created"], 1730000000);
        assert_eq!(out["choices"][0]["message"]["role"], "assistant");
        assert_eq!(out["choices"][0]["message"]["content"], "ok");
        assert!(out["choices"][0]["message"].get("reasoning_content").is_none(), "无思考内容不应带 reasoning_content");
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
        assert_eq!(out["usage"]["prompt_tokens"], 9);
        assert_eq!(out["usage"]["total_tokens"], 10);
        assert_eq!(out["usage"]["reasoning_tokens"], 0, "usage 透传上游完整字段");
    }

    #[test]
    fn assemble_sse_response_reasoning_and_graceful() {
        // hy4-preview：reasoning_content 分块 + 无 usage 块 + 无 [DONE] 也能收敛
        let sse = concat!(
            "data: {\"id\":\"cmb-y\",\"model\":\"hy4-preview\",\"object\":\"chat.completion.chunk\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\"We\"},\"finish_reason\":\"\"}]}\n",
            "data: {\"id\":\"cmb-y\",\"model\":\"hy4-preview\",\"object\":\"chat.completion.chunk\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"reasoning_content\":\" need\"},\"finish_reason\":\"\"}]}\n",
            "data: {\"id\":\"cmb-y\",\"model\":\"hy4-preview\",\"object\":\"chat.completion.chunk\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"42\"},\"finish_reason\":\"\"}]}\n",
        );
        let out = assemble_sse_response(sse).expect("assemble");
        assert_eq!(out["choices"][0]["message"]["reasoning_content"], "We need");
        assert_eq!(out["choices"][0]["message"]["content"], "42");
        assert_eq!(out["choices"][0]["finish_reason"], "stop", "无 finish_reason 时默认 stop");
        assert!(out.get("usage").is_none(), "无 usage 块不输出 usage");
        // 含 [DONE] 提前终止
        let out2 = assemble_sse_response("data: [DONE]\n").expect("empty done");
        assert_eq!(out2["choices"][0]["message"]["content"], "");
        // 非 data: 行忽略；非法 data: JSON 报错（不静默吞失败）
        let ok = assemble_sse_response("banner\n\ndata: {\"id\":\"i\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"\"}]}\n");
        assert!(ok.is_ok());
        assert!(assemble_sse_response("data: not-json\n").is_err());
    }

    #[test]
    fn resolve_api_key_codebuddy_alias() {
        // CODEBUDDY_API_KEY（DSH 凭证变量名）作为 codebuddy 的环境回退
        std::env::set_var("CODEBUDDY_API_KEY", "ck-env-1");
        let e = BackendEntry { provider: "codebuddy".into(), ..Default::default() };
        assert_eq!(e.resolve_api_key().as_deref(), Some("ck-env-1"));
        // 直填优先于环境别名
        let e2 = BackendEntry { provider: "codebuddy".into(), api_key: Some("ck-file".into()), ..Default::default() };
        assert_eq!(e2.resolve_api_key().as_deref(), Some("ck-file"));
        // 别名不影响其他提供方
        let e3 = BackendEntry { provider: "deepseek".into(), ..Default::default() };
        assert_eq!(e3.resolve_api_key(), None);
        std::env::remove_var("CODEBUDDY_API_KEY");
    }
}