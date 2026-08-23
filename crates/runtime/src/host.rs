//! Host：模块可用的宿主能力（服务注册/事件订阅/配置）。
//! 对应参考实现 pkg/plugin 的 Host 接口（Provide/On/Config）。

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::event::{EventBus, EventHandler, EventPayload};

/// 服务值：类型擦除。
pub type ServiceValue = Arc<dyn Any + Send + Sync>;

/// 宿主能力：模块经此注册服务、订阅事件、读取配置。
#[derive(Clone)]
pub struct Host {
    services: Arc<Mutex<HashMap<String, ServiceValue>>>,
    events: Arc<EventBus>,
}

impl Host {
    pub fn new() -> Self {
        Self {
            services: Arc::new(Mutex::new(HashMap::new())),
            events: Arc::new(EventBus::new()),
        }
    }

    /// 注册具名服务（其他模块可按名消费）。
    pub fn provide<T: Send + Sync + 'static>(&self, name: &str, svc: T) {
        let mut map = self.services.lock().unwrap();
        map.insert(name.to_string(), Arc::new(svc));
    }

    /// 按名获取服务（泛型向下转换）。
    pub fn get<T: Send + Sync + 'static>(&self, name: &str) -> Option<Arc<T>> {
        let map = self.services.lock().unwrap();
        map.get(name).and_then(|v| v.clone().downcast::<T>().ok())
    }

    /// 订阅事件。
    pub fn on(&self, event: &str, handler: EventHandler) -> usize {
        self.events.subscribe(event, handler)
    }

    /// 发布事件。
    pub fn emit(&self, event: &str, payload: EventPayload) {
        self.events.emit(event, payload)
    }

    /// 服务名列表（诊断）。
    pub fn service_names(&self) -> Vec<String> {
        self.services.lock().unwrap().keys().cloned().collect()
    }
}

impl Default for Host {
    fn default() -> Self {
        Self::new()
    }
}