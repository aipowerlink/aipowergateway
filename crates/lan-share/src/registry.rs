//! BackendRegistry：多后端注册 + 按模型名前缀路由 + 模型目录。
//!
//! 组长可同时共享 DeepSeek / Kimi / 智谱 等多家模型；
//! 组员在协议（OpenAI/Anthropic 二选一）里直接传模型名（如 deepseek-chat / kimi-2.7-code），
//! 注册表按模型名前缀路由到对应后端。

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::json;


use crate::backend::{Backend, Provider};

/// 模型 → 后端映射：前缀 → 后端名。
/// 内置前缀规则：deepseek-* → DeepSeek、kimi-* → Kimi、glm-* → 智谱、mock-* → Mock。
fn prefix_for(provider: Provider) -> &'static str {
    match provider {
        Provider::DeepSeek => "deepseek-",
        Provider::Kimi => "kimi-",
        Provider::Zhipu => "glm-",
        Provider::Mock => "mock-",
    }
}

/// 多后端注册表。
pub struct BackendRegistry {
    /// 后端名 → 后端实例。
    backends: HashMap<String, Arc<dyn Backend>>,
    /// 前缀 → 后端名（显式路由；内置前缀自动建立）。
    prefixes: HashMap<String, String>,
    /// 精确模型名 → 后端名（覆盖前缀路由）。
    exact: HashMap<String, String>,
}

impl Default for BackendRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl BackendRegistry {
    pub fn new() -> Self {
        Self {
            backends: HashMap::new(),
            prefixes: HashMap::new(),
            exact: HashMap::new(),
        }
    }

    /// 注册后端（自动建立内置前缀路由）。
    pub fn register(&mut self, backend: Arc<dyn Backend>) {
        let name = backend.name().to_string();
        let provider = backend.provider();
        // 自动前缀：provider 的内置前缀 → 该后端
        if provider != Provider::Mock || self.backends.is_empty() {
            self.prefixes.insert(prefix_for(provider).to_string(), name.clone());
        }
        // 模型精确映射
        for m in backend.models() {
            self.exact.insert(m, name.clone());
        }
        self.backends.insert(name, backend);
    }

    /// 按模型名路由到后端（精确匹配优先，其次前缀）。
    pub fn route(&self, model: &str) -> Option<(&str, Arc<dyn Backend>)> {
        // 精确模型名
        if let Some(name) = self.exact.get(model) {
            if let Some(b) = self.backends.get(name) {
                return Some((name.as_str(), b.clone()));
            }
        }
        // 前缀匹配（最长前缀优先）
        let mut best: Option<(usize, &String)> = None;
        for (prefix, name) in &self.prefixes {
            if model.starts_with(prefix.as_str()) && self.backends.contains_key(name) {
                let len = prefix.len();
                if best.map(|(bl, _)| len > bl).unwrap_or(true) {
                    best = Some((len, name));
                }
            }
        }
        if let Some((_, name)) = best {
            let b = self.backends.get(name).unwrap().clone();
            return Some((name.as_str(), b));
        }
        // 回退：仅一个后端时使用之
        if self.backends.len() == 1 {
            let (name, b) = self.backends.iter().next().unwrap();
            return Some((name.as_str(), b.clone()));
        }
        None
    }

    /// 模型目录：全部可用模型（模型名 → 后端名）。
    pub fn models_catalog(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (name, b) in &self.backends {
            for m in b.models() {
                out.push((m, name.clone()));
            }
        }
        out.sort();
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    /// OpenAI 格式 /v1/models 响应。
    pub fn openai_models_response(&self) -> serde_json::Value {
        let data: Vec<serde_json::Value> = self.models_catalog().iter().map(|(m, _)| {
            json!({
                "id": m,
                "object": "model",
                "created": 0,
                "owned_by": "aipowerlink",
            })
        }).collect();
        json!({
            "object": "list",
            "data": data,
        })
    }

    /// Anthropic 格式 /v1/models 响应。
    pub fn anthropic_models_response(&self) -> serde_json::Value {
        let data: Vec<serde_json::Value> = self.models_catalog().iter().map(|(m, _)| {
            json!({
                "id": m,
                "type": "model",
                "display_name": m,
                "created_at": "2026-01-01T00:00:00Z",
            })
        }).collect();
        json!({
            "data": data,
            "has_more": false,
            "first_id": data.first().and_then(|v| v.get("id")).cloned().unwrap_or(json!(null)),
            "last_id": data.last().and_then(|v| v.get("id")).cloned().unwrap_or(json!(null)),
        })
    }

    pub fn backend_count(&self) -> usize {
        self.backends.len()
    }

    /// 模型目录摘要（用于广播/诊断）。
    pub fn summary(&self) -> Vec<String> {
        self.models_catalog().iter().map(|(m, _)| m.clone()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::OpenAICompatConfig;
    use crate::backend::OpenAICompatBackend;

    fn mk_backend(provider: Provider, model: &str) -> Arc<dyn Backend> {
        let cfg = OpenAICompatConfig {
            provider,
            api_key: "sk-test".into(),
            model: Some(model.to_string()),
            base_url: None,
            timeout_secs: 60,
        };
        Arc::new(OpenAICompatBackend::new(cfg))
    }

    #[test]
    fn routes_by_model_prefix() {
        let mut reg = BackendRegistry::new();
        reg.register(mk_backend(Provider::DeepSeek, "deepseek-chat"));
        reg.register(mk_backend(Provider::Kimi, "kimi-2.7-code"));
        reg.register(mk_backend(Provider::Zhipu, "glm-4-flash"));
        assert_eq!(reg.backend_count(), 3);
        // 前缀路由
        let (name, _) = reg.route("deepseek-v4-flash").unwrap();
        assert_eq!(name, "deepseek");
        let (name, _) = reg.route("kimi-2.7-code").unwrap();
        assert_eq!(name, "kimi");
        let (name, _) = reg.route("glm-4-flash").unwrap();
        assert_eq!(name, "zhipu");
        // 未知名 → None（多后端时）
        assert!(reg.route("unknown-model").is_none());
    }

    #[test]
    fn catalog_lists_all_models() {
        let mut reg = BackendRegistry::new();
        reg.register(mk_backend(Provider::DeepSeek, "deepseek-chat"));
        reg.register(mk_backend(Provider::Kimi, "kimi-2.7-code"));
        let catalog = reg.models_catalog();
        assert!(catalog.iter().any(|(m, _)| m == "deepseek-chat"));
        assert!(catalog.iter().any(|(m, _)| m == "kimi-2.7-code"));
        // OpenAI 格式
        let resp = reg.openai_models_response();
        assert_eq!(resp["data"].as_array().unwrap().len(), 2);
        // Anthropic 格式
        let aresp = reg.anthropic_models_response();
        assert_eq!(aresp["data"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn single_backend_fallback() {
        let mut reg = BackendRegistry::new();
        reg.register(mk_backend(Provider::DeepSeek, "deepseek-chat"));
        let (name, _) = reg.route("anything").unwrap();
        assert_eq!(name, "deepseek");
    }
}