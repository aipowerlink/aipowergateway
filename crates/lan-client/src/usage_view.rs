//! lan-usage-view：个人 token 用量记录与展示。

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

/// 个人用量。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonalUsage {
    /// 累计 prompt tokens。
    pub prompt_tokens: u64,
    /// 累计 completion tokens。
    pub completion_tokens: u64,
    /// 累计调用次数。
    pub calls: u64,
    /// 组长名（可多个）。
    pub leaders: std::collections::HashMap<String, u64>,
}

impl PersonalUsage {
    pub fn total(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }

    /// 记录一次调用用量。
    pub fn record(&mut self, leader: &str, prompt: u64, completion: u64) {
        self.prompt_tokens += prompt;
        self.completion_tokens += completion;
        self.calls += 1;
        *self.leaders.entry(leader.to_string()).or_insert(0) += prompt + completion;
    }
}

/// 用量视图（线程安全 + 持久化）。
#[derive(Clone)]
pub struct UsageView {
    inner: Arc<RwLock<PersonalUsage>>,
    persist_path: Option<std::path::PathBuf>,
}

impl UsageView {
    pub fn new(persist_path: Option<std::path::PathBuf>) -> Self {
        let svc = Self {
            inner: Arc::new(RwLock::new(PersonalUsage::default())),
            persist_path,
        };
        if let Some(p) = &svc.persist_path {
            if let Ok(data) = std::fs::read(p) {
                if let Ok(u) = serde_json::from_slice::<PersonalUsage>(&data) {
                    *svc.inner.write().unwrap() = u;
                }
            }
        }
        svc
    }

    /// 记录用量（自动持久化）。
    pub fn record(&self, leader: &str, prompt: u64, completion: u64) {
        self.inner.write().unwrap().record(leader, prompt, completion);
        if let Some(p) = &self.persist_path {
            if let Ok(data) = serde_json::to_vec(&*self.inner.read().unwrap()) {
                let _ = std::fs::write(p, data);
            }
        }
    }

    pub fn get(&self) -> PersonalUsage {
        self.inner.read().unwrap().clone()
    }

    pub fn total(&self) -> u64 {
        self.inner.read().unwrap().total()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_accumulates() {
        let u = UsageView::new(None);
        u.record("leader1", 10, 20);
        u.record("leader1", 5, 5);
        let g = u.get();
        assert_eq!(g.calls, 2);
        assert_eq!(g.total(), 40);
        assert_eq!(g.leaders.get("leader1"), Some(&40));
    }

    #[test]
    fn persists() {
        let path = std::env::temp_dir().join("aipg-usage-view.json");
        let _ = std::fs::remove_file(&path);
        {
            let u = UsageView::new(Some(path.clone()));
            u.record("l", 1, 2);
        }
        let u2 = UsageView::new(Some(path.clone()));
        assert_eq!(u2.total(), 3);
        let _ = std::fs::remove_file(&path);
    }
}