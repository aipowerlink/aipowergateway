//! 模块契约：Module trait + ModuleContext（对应 Cordis 插件：name/inject/apply）。

use std::collections::HashMap;
use std::fmt;

use crate::host::Host;
use crate::{RuntimeResult};

/// 模块上下文：装配时注入 Host 与模块配置。
pub struct ModuleContext<'a> {
    /// 宿主能力（服务注册/事件总线/配置）。
    pub host: &'a mut Host,
    /// 本模块合并后的配置（角色覆盖已应用）。
    pub config: &'a serde_json::Value,
    /// 模块名（诊断用）。
    pub name: &'a str,
}

/// 模块契约：一切皆模块（对应 DSH/Cordis 插件语义）。
pub trait Module: Send + Sync {
    /// 唯一模块名。
    fn name(&self) -> &'static str;

    /// 依赖模块名列表（先装配）。
    fn requires(&self) -> &'static [&'static str] {
        &[]
    }

    /// 可选模块：装配失败降级跳过（默认必选）。
    fn optional(&self) -> bool {
        false
    }

    /// 模块默认配置（schema 驱动，供合并）。
    fn default_config(&self) -> serde_json::Value {
        serde_json::json!({})
    }

    /// 应用模块：注册服务/订阅事件/启动资源。
    fn apply(&self, ctx: ModuleContext<'_>) -> RuntimeResult<()>;
}

impl fmt::Debug for dyn Module {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Module({})", self.name())
    }
}

/// 模块注册表：按名注册与查询。
#[derive(Default)]
pub struct Registry {
    modules: HashMap<&'static str, Box<dyn Module>>,
}

impl Registry {
    pub fn new() -> Self {
        Self::default()
    }

    /// 注册模块（重名报错）。
    pub fn register(&mut self, m: Box<dyn Module>) -> RuntimeResult<()> {
        let name = m.name();
        if self.modules.contains_key(name) {
            return Err(crate::RuntimeError::DuplicateModule(name.to_string()));
        }
        self.modules.insert(name, m);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&Box<dyn Module>> {
        self.modules.get(name)
    }

    pub fn names(&self) -> Vec<&'static str> {
        self.modules.keys().copied().collect()
    }

    pub fn len(&self) -> usize {
        self.modules.len()
    }

    pub fn is_empty(&self) -> bool {
        self.modules.is_empty()
    }
}