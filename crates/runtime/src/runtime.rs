//! Runtime：装配引擎——按角色选择模块集，依赖拓扑装配，Boot/Stop 逆序回收。

use std::collections::HashSet;

use crate::host::Host;
use crate::module::{Module, ModuleContext, Registry};
use crate::{RuntimeError, RuntimeResult};

/// 内置角色（system trust）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// 组长端（服务端）：开放共享。
    Server,
    /// 组员端（消费端）：接入使用。
    Client,
    /// 自定义角色（id 字符串）。
    Custom(&'static str),
}

impl Role {
    /// 内置角色名。
    pub fn name(&self) -> &str {
        match self {
            Role::Server => "server",
            Role::Client => "client",
            Role::Custom(id) => id,
        }
    }

    /// 内置角色默认模块集。
    pub fn default_modules(&self) -> &'static [&'static str] {
        match self {
            Role::Server => &[
                crate::MOD_LAN_SHARE_SERVER,
                crate::MOD_LAN_AUTH,
                crate::MOD_LAN_MEMBER_REGISTRY,
                crate::MOD_LAN_USAGE,
                crate::MOD_LAN_DISCOVERY_BROADCAST,
                crate::MOD_LAN_WEB_CONSOLE,
            ],
            Role::Client => &[
                crate::MOD_LAN_DISCOVERY_CLIENT,
                crate::MOD_LAN_SHARE_CLIENT,
                crate::MOD_LAN_IDENTITY,
                crate::MOD_LAN_USAGE_VIEW,
            ],
            Role::Custom(_) => &[], // 自定义角色从 role 文件读取模块清单
        }
    }
}

/// 装配结果：已启用的模块（boot 顺序）。
pub struct BootResult {
    host: Host,
    booted: Vec<&'static str>,
    skipped: Vec<(String, String)>,
}

impl BootResult {
    pub fn host(&self) -> &Host {
        &self.host
    }

    pub fn booted(&self) -> &[&'static str] {
        &self.booted
    }

    /// 被跳过的模块（Optional 失败降级）及原因。
    pub fn skipped(&self) -> &[(String, String)] {
        &self.skipped
    }
}

/// 运行时：模块注册 + 装配执行。
pub struct Runtime {
    registry: Registry,
    host: Host,
}

impl Runtime {
    pub fn new() -> Self {
        Self {
            registry: Registry::new(),
            host: Host::new(),
        }
    }

    pub fn register(&mut self, m: Box<dyn Module>) -> RuntimeResult<()> {
        self.registry.register(m)
    }

    pub fn host(&self) -> &Host {
        &self.host
    }

    /// 按角色装配：拓扑排序 + 依赖检查 + 生命周期（Boot 顺序，Stop 逆序由调用方 drop）。
    pub fn boot(&mut self, role: &Role, overrides: &serde_json::Value) -> RuntimeResult<BootResult> {
        // 自定义角色：从配置读取模块清单（role 文件解析由上层完成）
        let want: Vec<&'static str> = if let Role::Custom(id) = role {
            let list = overrides
                .get("modules")
                .and_then(|v| v.as_array())
                .ok_or_else(|| RuntimeError::ConfigError(format!("custom role {id}: missing modules list")))?;
            list.iter()
                .filter_map(|v| v.as_str())
                .map(|s| Box::leak(s.to_string().into_boxed_str()) as &'static str)
                .collect()
        } else {
            role.default_modules().to_vec()
        };

        // 检查必需模块是否存在
        let available: HashSet<&str> = self.registry.names().iter().copied().collect();
        for name in &want {
            if !available.contains(name) {
                return Err(RuntimeError::MissingModule((*name).to_string()));
            }
        }

        // 拓扑排序（依赖先装配）
        let order = topo_sort(&self.registry, &want)?;

        // 逐模块 apply
        let mut booted: Vec<&'static str> = Vec::new();
        let mut skipped: Vec<(String, String)> = Vec::new();
        for name in &order {
            let module = self.registry.get(name).unwrap();
            // 合并配置：模块默认 + 角色覆盖
            let merged = merge_module_config(module.default_config(), overrides, name);
            let ctx = ModuleContext {
                host: &mut self.host,
                config: &merged,
                name,
            };
            match module.apply(ctx) {
                Ok(()) => {
                    tracing::info!(module = name, "booted");
                    booted.push(name);
                }
                Err(e) if module.optional() => {
                    tracing::warn!(module = name, error = %e, "optional module failed, skipped");
                    skipped.push(((*name).to_string(), e.to_string()));
                }
                Err(e) => {
                    tracing::error!(module = name, error = %e, "required module failed");
                    return Err(e);
                }
            }
        }

        Ok(BootResult { host: self.host.clone(), booted, skipped })
    }
}

/// 依赖拓扑排序：返回满足依赖顺序的模块名列表（DFS）。
fn topo_sort(registry: &Registry, want: &[&'static str]) -> RuntimeResult<Vec<&'static str>> {
    let want_set: HashSet<&str> = want.iter().copied().collect();
    let mut visited: HashSet<&'static str> = HashSet::new();
    let mut order: Vec<&'static str> = Vec::new();
    let mut visiting: HashSet<&'static str> = HashSet::new();

    fn visit(
        name: &'static str,
        registry: &Registry,
        want_set: &HashSet<&str>,
        visited: &mut HashSet<&'static str>,
        visiting: &mut HashSet<&'static str>,
        order: &mut Vec<&'static str>,
    ) -> RuntimeResult<()> {
        if visited.contains(name) {
            return Ok(());
        }
        if visiting.contains(name) {
            return Err(RuntimeError::DependencyCycle(format!("cycle at {name}")));
        }
        visiting.insert(name);
        let module = registry.get(name).unwrap();
        for dep in module.requires() {
            if want_set.contains(dep) {
                visit(dep, registry, want_set, visited, visiting, order)?;
            }
        }
        visiting.remove(name);
        visited.insert(name);
        order.push(name);
        Ok(())
    }

    for name in want {
        visit(name, registry, &want_set, &mut visited, &mut visiting, &mut order)?;
    }
    Ok(order)
}

/// 合并模块配置：模块默认 + 角色级 overrides（按模块名取值）。
fn merge_module_config(default: serde_json::Value, overrides: &serde_json::Value, module: &str) -> serde_json::Value {
    let module_ov = overrides.get(module).cloned().unwrap_or(serde_json::json!({}));
    let mut merged = default;
    crate::config::deep_merge(&mut merged, &module_ov);
    merged
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModuleContext;

    struct TestModule {
        name: &'static str,
        reqs: &'static [&'static str],
        optional: bool,
    }

    impl Module for TestModule {
        fn name(&self) -> &'static str { self.name }
        fn requires(&self) -> &'static [&'static str] { self.reqs }
        fn optional(&self) -> bool { self.optional }
        fn apply(&self, _ctx: ModuleContext<'_>) -> RuntimeResult<()> { Ok(()) }
    }

    #[test]
    fn boot_respects_dependency_order() {
        let mut rt = Runtime::new();
        rt.register(Box::new(TestModule { name: "b", reqs: &["a"], optional: false })).unwrap();
        rt.register(Box::new(TestModule { name: "a", reqs: &[], optional: false })).unwrap();
        let res = rt.boot(&Role::Custom("test"), &serde_json::json!({ "modules": ["a", "b"] })).unwrap();
        assert_eq!(res.booted(), &["a", "b"]);
    }

    #[test]
    fn optional_module_failure_skips() {
        struct FailModule;
        impl Module for FailModule {
            fn name(&self) -> &'static str { "fail" }
            fn optional(&self) -> bool { true }
            fn apply(&self, _ctx: ModuleContext<'_>) -> RuntimeResult<()> {
                Err(RuntimeError::ModuleError("boom".to_string()))
            }
        }
        let mut rt = Runtime::new();
        rt.register(Box::new(FailModule)).unwrap();
        let res = rt.boot(&Role::Custom("t"), &serde_json::json!({ "modules": ["fail"] })).unwrap();
        assert_eq!(res.booted().len(), 0);
        assert_eq!(res.skipped().len(), 1);
    }
}