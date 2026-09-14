//! BackendRegistry：多后端注册 + 按模型名前缀路由 + 模型目录。
//!
//! 组长可同时共享 DeepSeek / Kimi / 智谱 等多家模型；
//! 组员在协议（OpenAI/Anthropic 二选一）里直接传模型名（如 deepseek-chat / kimi-2.7-code），
//! 注册表按模型名前缀路由到对应后端。
//!
//! 内部使用 RwLock：面板「模型设置」保存后可在运行期整体热替换（无需重启）。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use serde_json::json;

use crate::backend::{Backend, BackendEntry, Provider};
use crate::health::ProviderState;

/// 模型 → 后端映射：前缀 → 后端名。
/// 内置前缀规则：deepseek-* → DeepSeek、kimi-* → Kimi、glm-* → 智谱、mock-* → Mock。
/// 自定义提供方无前缀（仅精确模型名路由）；CodeBuddy 模型名无统一前缀，同样走精确路由。
fn prefix_for(provider: Provider) -> Option<&'static str> {
    match provider {
        Provider::DeepSeek => Some("deepseek-"),
        Provider::Kimi => Some("kimi-"),
        Provider::Zhipu => Some("glm-"),
        Provider::Mock => Some("mock-"),
        Provider::CodeBuddy => None,
        Provider::Custom => None,
    }
}

/// 注册表内部状态（可整体替换以实现热更新）。
struct RegistryInner {
    /// 后端名 → 后端实例。
    backends: HashMap<String, Arc<dyn Backend>>,
    /// 前缀 → 后端名（显式路由；内置前缀自动建立）。
    prefixes: HashMap<String, String>,
    /// 精确模型名 → 后端名（覆盖前缀路由；自定义提供方依赖此路由）。
    exact: HashMap<String, String>,
}

impl Default for RegistryInner {
    fn default() -> Self {
        Self {
            backends: HashMap::new(),
            prefixes: HashMap::new(),
            exact: HashMap::new(),
        }
    }
}

/// 多后端注册表（运行期热更新安全）。
#[derive(Default)]
pub struct BackendRegistry {
    inner: RwLock<RegistryInner>,
    /// 健康状态表（与 HealthMonitor 共享同一 Arc；热替换注册表不丢健康状态）。
    /// OnceLock：attach 仅在构造早期调用一次；未 attach = 健康感知关闭。
    health: std::sync::OnceLock<Arc<RwLock<HashMap<String, ProviderState>>>>,
}

impl BackendRegistry {
    /// 挂接健康状态表（HealthMonitor 启动时调用；共享同一 Arc，只允许一次）。
    pub fn attach_health(&self, health: Arc<RwLock<HashMap<String, ProviderState>>>) {
        let _ = self.health.set(health);
    }

    pub fn new() -> Self {
        Self::default()
    }

    /// 注册后端（自动建立内置前缀路由）。
    pub fn register(&self, backend: Arc<dyn Backend>) {
        let mut inner = self.inner.write().unwrap();
        let name = backend.name().to_string();
        let provider = backend.provider();
        // 自动前缀：内置提供商建前缀；mock 仅在无其他后端时占位；custom 无前缀
        if let Some(prefix) = prefix_for(provider) {
            if provider != Provider::Mock || inner.backends.is_empty() {
                inner.prefixes.insert(prefix.to_string(), name.clone());
            }
        }
        // 模型精确映射
        for m in backend.models() {
            inner.exact.insert(m, name.clone());
        }
        inner.backends.insert(name, backend);
    }

    /// 整体热替换（面板保存后调用；原子换出旧状态）。
    pub fn replace_all(&self, backends: Vec<Arc<dyn Backend>>) {
        let mut inner = RegistryInner::default();
        for b in backends {
            let name = b.name().to_string();
            let provider = b.provider();
            if let Some(prefix) = prefix_for(provider) {
                if provider != Provider::Mock || inner.backends.is_empty() {
                    inner.prefixes.insert(prefix.to_string(), name.clone());
                }
            }
            for m in b.models() {
                inner.exact.insert(m, name.clone());
            }
            inner.backends.insert(name, b);
        }
        *self.inner.write().unwrap() = inner;
    }

    /// 按模型名路由到后端（精确匹配优先，其次前缀）。
    /// 后端是否已摘除（removed）：退出候选序列，不可路由、不进入模型目录。
    fn is_removed(&self, name: &str) -> bool {
        let Some(health) = self.health.get() else { return false };
        health.read().unwrap().get(name).map(|s| s.level == crate::health::HealthLevel::Removed).unwrap_or(false)
    }

    /// 后端路由顺位（degraded 降权：排在 ok 后端之后）。
    fn route_rank(&self, name: &str) -> u8 {
        let Some(health) = self.health.get() else { return 0 };
        match health.read().unwrap().get(name).map(|s| s.level) {
            Some(crate::health::HealthLevel::Degraded) => 1,
            _ => 0,
        }
    }

    pub fn route(&self, model: &str) -> Option<(String, Arc<dyn Backend>)> {
        let inner = self.inner.read().unwrap();
        // 精确模型名（removed 时视为不可路由）
        if let Some(name) = inner.exact.get(model) {
            if let Some(b) = inner.backends.get(name) {
                if self.is_removed(name) {
                    return None;
                }
                return Some((name.clone(), b.clone()));
            }
        }
        // 前缀匹配（最长前缀优先；removed 后端跳过）
        let mut best: Option<(usize, &String, u8)> = None;
        for (prefix, name) in &inner.prefixes {
            if model.starts_with(prefix.as_str()) && inner.backends.contains_key(name) && !self.is_removed(name) {
                let len = prefix.len();
                let rank = self.route_rank(name);
                let better = match best {
                    None => true,
                    Some((bl, _, br)) => len > bl || (len == bl && rank < br),
                };
                if better {
                    best = Some((len, name, rank));
                }
            }
        }
        if let Some((_, name, _)) = best {
            let b = inner.backends.get(name).unwrap().clone();
            return Some((name.clone(), b));
        }
        // 回退：仅一个后端时使用之（removed 不可用）
        if inner.backends.len() == 1 {
            let (name, b) = inner.backends.iter().next().unwrap();
            if self.is_removed(name) {
                return None;
            }
            return Some((name.clone(), b.clone()));
        }
        None
    }

    /// 模型目录：全部可用模型（模型名 → 后端名；removed 后端剔除，degraded 排后）。
    pub fn models_catalog(&self) -> Vec<(String, String)> {
        let inner = self.inner.read().unwrap();
        let mut out: Vec<(String, String)> = Vec::new();
        for (name, b) in &inner.backends {
            if self.is_removed(name) {
                continue;
            }
            for m in b.models() {
                out.push((m, name.clone()));
            }
        }
        // 降权排序：degraded 后端模型排到 ok 后端之后
        out.sort_by(|(m1, n1), (m2, n2)| {
            let r1 = self.route_rank(n1);
            let r2 = self.route_rank(n2);
            (r1, m1).cmp(&(r2, m2))
        });
        out.dedup_by(|a, b| a.0 == b.0);
        out
    }

    /// 后端数量。
    pub fn backend_count(&self) -> usize {
        self.inner.read().unwrap().backends.len()
    }

    /// 已注册后端名列表。
    pub fn backend_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.inner.read().unwrap().backends.keys().cloned().collect();
        names.sort();
        names
    }

    /// OpenAI 格式 /v1/models 响应。
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
}

/// 从配置条目构建后端（对应 backends.yaml providers 段）。
pub fn backend_from_entry(entry: &BackendEntry) -> anyhow::Result<Arc<dyn Backend>> {
    use crate::backend::{MockBackend, OpenAICompatBackend, OpenAICompatConfig};
    let provider = Provider::from_str(&entry.provider);
    match provider {
        Provider::Mock => Ok(Arc::new(MockBackend::default())),
        _ => {
            // 自定义提供方需要 base_url；官方可选用默认
            if !provider.is_builtin() {
                let url = entry.base_url.clone().unwrap_or_default();
                if url.trim().is_empty() {
                    anyhow::bail!("custom provider '{}' requires base_url", entry.backend_id());
                }
            }
            let models = entry.effective_models();
            // 执行体预设（pair/agent）：模型列表可为空——保存后异步探活自动获取真实清单落盘；
            // 常规 custom 仍需至少一个模型才能路由
            let is_executor_preset = crate::backend::is_executor_provider(&entry.provider);
            if !provider.is_builtin() && models.is_empty() && !is_executor_preset {
                anyhow::bail!("custom provider '{}' requires at least one model", entry.backend_id());
            }
            let cfg = OpenAICompatConfig {
                provider,
                api_key: entry.resolve_api_key().unwrap_or_default(),
                model: entry.model.clone().filter(|m| !m.is_empty()),
                models,
                base_url: entry.base_url.clone(),
                timeout_secs: 60,
                provider_label: Some(entry.provider.clone()),
                name: Some(entry.backend_id()),
            };
            Ok(Arc::new(OpenAICompatBackend::new(cfg)))
        }
    }
}

/// 从配置条目列表构建注册表（面板/启动共用）。
pub fn registry_from_entries(entries: &[BackendEntry]) -> anyhow::Result<BackendRegistry> {
    let registry = BackendRegistry::new();
    let mut built: Vec<Arc<dyn Backend>> = Vec::new();
    for e in entries {
        built.push(backend_from_entry(e)?);
    }
    registry.replace_all(built);
    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(provider: &str, id: Option<&str>, model: Option<&str>, url: Option<&str>) -> BackendEntry {
        BackendEntry {
            provider: provider.to_string(),
            id: id.map(|s| s.to_string()),
            api_key: None,
            api_key_env: None,
            model: model.map(|s| s.to_string()),
            models: Vec::new(),
            base_url: url.map(|s| s.to_string()),
            split_ratio: None,
            listing_id: None,
        }
    }

    #[test]
    fn builtin_prefix_routing() {
        let reg = registry_from_entries(&[
            entry("mock", None, None, None),
            entry("deepseek", None, None, None),
        ]).unwrap();
        assert_eq!(reg.backend_count(), 2);
        assert!(reg.models_catalog().iter().any(|(m, _)| m == "deepseek-chat"));
        let (name, _) = reg.route("deepseek-chat").expect("route deepseek-chat");
        assert_eq!(name, "deepseek");
        let (name, _) = reg.route("mock-anything").expect("route mock prefix");
        assert_eq!(name, "mock");
    }

    #[test]
    fn custom_provider_routes_by_exact_model() {
        let reg = registry_from_entries(&[
            entry("ollama", Some("ollama"), Some("qwen2.5:7b"), Some("http://127.0.0.1:11434/v1")),
            entry("deepseek", None, Some("deepseek-chat"), None),
        ]).unwrap();
        // 自定义端模型：精确路由
        let (name, _) = reg.route("qwen2.5:7b").expect("route custom model");
        assert_eq!(name, "ollama");
        // 官方前缀不受影响
        let (name2, _) = reg.route("deepseek-chat").expect("route deepseek");
        assert_eq!(name2, "deepseek");
        assert!(reg.models_catalog().iter().any(|(m, _)| m == "qwen2.5:7b"));
    }

    #[test]
    fn custom_provider_requires_url_and_model() {
        let e = entry("ollama", Some("ollama"), None, Some("http://x/v1"));
        assert!(backend_from_entry(&e).is_err(), "missing model");
        let e2 = entry("ollama", Some("ollama"), Some("m"), None);
        assert!(backend_from_entry(&e2).is_err(), "missing base_url");
    }

    #[test]
    fn multi_model_entry_catalog_and_route() {
        // cc-switch 式多模型：一提供方多个模型全部进入目录并可路由
        let mut e = entry("deepseek", None, None, None);
        e.models = vec!["deepseek-chat".into(), "deepseek-reasoner".into()];
        let reg = registry_from_entries(&[e]).unwrap();
        assert_eq!(reg.backend_count(), 1);
        for m in ["deepseek-chat", "deepseek-reasoner"] {
            assert!(reg.models_catalog().iter().any(|(x, _)| x == m), "catalog missing {m}");
            let (name, _) = reg.route(m).expect(&format!("route {m}"));
            assert_eq!(name, "deepseek");
        }
        // 未列入目录的模型不出现在目录（路由前缀仍按 deepseek-* 生效）
        assert!(!reg.models_catalog().iter().any(|(x, _)| x == "deepseek-other"));
    }

    #[test]
    fn codebuddy_routes_by_exact_model() {
        // codebuddy + deepseek 双后端：验证 codebuddy 无前缀、仅精确模型路由
        let reg = registry_from_entries(&[
            entry("codebuddy", None, None, None),
            entry("deepseek", None, None, None),
        ]).unwrap();
        assert_eq!(reg.backend_count(), 2);
        for m in ["hy4-preview", "deepseek-v4-flash"] {
            assert!(reg.models_catalog().iter().any(|(x, _)| x == m), "catalog missing {m}");
            let (name, _) = reg.route(m).expect(&format!("route {m}"));
            assert_eq!(name, "codebuddy");
        }
        // deepseek 前缀路由不受影响
        let (name, _) = reg.route("deepseek-chat").expect("route deepseek");
        assert_eq!(name, "deepseek");
        // codebuddy 无前缀：未知模型不路由到它
        assert!(reg.route("codebuddy-other").is_none(), "codebuddy 无前缀路由");
        assert!(reg.route("hy4").is_none(), "无前缀不按部分名路由");
    }

    #[test]
    fn hot_replace_swaps_routing() {
        let reg = BackendRegistry::new();
        reg.register(Arc::new(crate::backend::MockBackend::default()));
        assert_eq!(reg.backend_count(), 1);
        // 热替换：mock 换成 deepseek+custom
        let entries = vec![
            entry("deepseek", None, Some("deepseek-chat"), None),
            entry("ollama", Some("ollama"), Some("qwen2.5:7b"), Some("http://127.0.0.1:11434/v1")),
        ];
        let built: Vec<Arc<dyn Backend>> = entries.iter().map(backend_from_entry).collect::<anyhow::Result<_>>().unwrap();
        reg.replace_all(built);
        assert_eq!(reg.backend_count(), 2);
        assert!(reg.route("mock-7b").is_none(), "mock 应已移除");
        let (name, _) = reg.route("deepseek-chat").expect("新后端可路由");
        assert_eq!(name, "deepseek");
        let (name2, _) = reg.route("qwen2.5:7b").expect("custom 精确路由");
        assert_eq!(name2, "ollama");
    }

    /// removed 后端：退出候选序列（不可路由、目录剔除）；degraded 仍可路由。
    #[test]
    fn removed_excluded_from_route_and_catalog() {
        let reg = registry_from_entries(&[
            entry("deepseek", None, Some("deepseek-chat"), None),
            entry("ollama", Some("ollama"), Some("qwen2.5:7b"), Some("http://127.0.0.1:11434/v1")),
        ]).unwrap();
        // 挂接健康状态表：deepseek → removed，ollama → degraded
        let st: Arc<std::sync::RwLock<std::collections::HashMap<String, crate::health::ProviderState>>> =
            Arc::new(std::sync::RwLock::new(std::collections::HashMap::from([
                ("deepseek".into(), crate::health::ProviderState {
                    level: crate::health::HealthLevel::Removed,
                    failures: 10,
                    latency_ms: None,
                    last_error: Some("连续失败摘除".into()),
                }),
                ("ollama".into(), crate::health::ProviderState {
                    level: crate::health::HealthLevel::Degraded,
                    failures: 4,
                    latency_ms: None,
                    last_error: Some("延迟异常".into()),
                }),
            ])));
        reg.attach_health(st);
        // removed：路由不可达、目录剔除
        assert!(reg.route("deepseek-chat").is_none(), "removed 后端不可路由");
        assert!(!reg.models_catalog().iter().any(|(m, _)| m == "deepseek-chat"), "removed 模型剔除目录");
        // degraded：仍可路由、目录保留（降权排后）
        let (name, _) = reg.route("qwen2.5:7b").expect("degraded 仍可路由");
        assert_eq!(name, "ollama");
        assert!(reg.models_catalog().iter().any(|(m, _)| m == "qwen2.5:7b"), "degraded 模型保留目录");
    }

    /// 热替换注册表后健康状态不丢（状态表与注册表解耦）。
    #[test]
    fn health_survives_hot_replace() {
        let reg = registry_from_entries(&[entry("ollama", Some("ollama"), Some("qwen2.5:7b"), Some("http://x/v1"))]).unwrap();
        let st: Arc<std::sync::RwLock<std::collections::HashMap<String, crate::health::ProviderState>>> =
            Arc::new(std::sync::RwLock::new(std::collections::HashMap::from([
                ("ollama".into(), crate::health::ProviderState {
                    level: crate::health::HealthLevel::Removed,
                    failures: 10,
                    latency_ms: None,
                    last_error: None,
                }),
            ])));
        reg.attach_health(st);
        reg.replace_all(vec![crate::registry::backend_from_entry(&entry("ollama", Some("ollama"), Some("qwen2.5:7b"), Some("http://x/v1"))).unwrap()]);
        assert!(reg.route("qwen2.5:7b").is_none(), "热替换后 removed 仍然生效");
    }
}
