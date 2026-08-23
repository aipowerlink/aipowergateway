//! 事件总线：跨模块通信（对应 DSH/Cordis ctx.on/ctx.emit）。

use std::any::Any;
use std::collections::HashMap;
use std::sync::Mutex;

/// 事件负载：类型擦除的 Box<dyn Any>。
pub type EventPayload = Box<dyn Any + Send>;

/// 事件处理函数（订阅者）。
pub type EventHandler = Box<dyn Fn(&str, &EventPayload) + Send + Sync>;

/// 事件总线：按事件名订阅/发布。
#[derive(Default)]
pub struct EventBus {
    handlers: Mutex<HashMap<String, Vec<EventHandler>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 订阅事件（返回订阅 id，供取消）。
    pub fn subscribe(&self, event: &str, handler: EventHandler) -> usize {
        let mut map = self.handlers.lock().unwrap();
        let list = map.entry(event.to_string()).or_default();
        list.push(handler);
        list.len() - 1
    }

    /// 发布事件（同步调用全部订阅者）。
    pub fn emit(&self, event: &str, payload: EventPayload) {
        let map = self.handlers.lock().unwrap();
        if let Some(list) = map.get(event) {
            for h in list {
                h(event, &payload);
            }
        }
    }

    /// 取消订阅（按事件名 + 订阅 id）。
    pub fn unsubscribe(&self, event: &str, id: usize) {
        let mut map = self.handlers.lock().unwrap();
        if let Some(list) = map.get_mut(event) {
            if id < list.len() {
                list[id] = Box::new(|_, _| {}); // no-op 占位
            }
        }
    }

    pub fn has_subscribers(&self, event: &str) -> bool {
        self.handlers.lock().unwrap().contains_key(event)
    }
}