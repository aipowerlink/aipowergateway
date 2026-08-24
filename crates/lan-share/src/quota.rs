//! lan-quota：按成员 token 配额（组长设置每人上限，超出返回 429）。
//!
//! 插件形态示例（ApiState 服务组件）：独立持久化（quota.json，与 usage.json 同模式），
//! 用量从 UsageService 读取，本服务只管配额上限。0 或未设置 = 不限。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

/// 配额超限错误。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaExceeded {
    /// 配额上限（token）。
    pub limit: u64,
}

/// 成员配额项。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemberQuota {
    /// 成员 id（机器名）。
    pub member_id: String,
    /// token 配额上限（0 = 不限）。
    pub limit: u64,
}

/// 配额服务。
#[derive(Clone)]
pub struct QuotaService {
    inner: Arc<QuotaInner>,
}

struct QuotaInner {
    /// member_id -> 配额上限。
    quotas: RwLock<HashMap<String, u64>>,
    persist_path: PathBuf,
}

impl QuotaService {
    pub fn new(persist_path: PathBuf) -> Self {
        let svc = Self {
            inner: Arc::new(QuotaInner {
                quotas: RwLock::new(HashMap::new()),
                persist_path,
            }),
        };
        svc.load();
        svc
    }

    /// 设置成员配额（0 = 解除限制）。
    pub fn set(&self, member_id: &str, limit: u64) {
        let mut map = self.inner.quotas.write().unwrap();
        if limit == 0 {
            map.remove(member_id);
        } else {
            map.insert(member_id.to_string(), limit);
        }
        drop(map);
        self.save();
    }

    /// 查询成员配额（None = 不限）。
    pub fn get(&self, member_id: &str) -> Option<u64> {
        self.inner.quotas.read().unwrap().get(member_id).copied()
    }

    /// 全部配额（按成员名排序）。
    pub fn all(&self) -> Vec<MemberQuota> {
        let map = self.inner.quotas.read().unwrap();
        let mut v: Vec<MemberQuota> = map
            .iter()
            .map(|(id, limit)| MemberQuota { member_id: id.clone(), limit: *limit })
            .collect();
        v.sort_by(|a, b| a.member_id.cmp(&b.member_id));
        v
    }

    /// 检查成员当前用量是否超配额。
    /// `used` 为成员累计 total tokens（来自 UsageService）。
    pub fn check(&self, member_id: &str, used: u64) -> Result<(), QuotaExceeded> {
        match self.inner.quotas.read().unwrap().get(member_id) {
            Some(&limit) if limit > 0 && used >= limit => Err(QuotaExceeded { limit }),
            _ => Ok(()),
        }
    }

    fn save(&self) {
        let map = self.inner.quotas.read().unwrap();
        if let Ok(data) = serde_json::to_vec(&*map) {
            let _ = std::fs::write(&self.inner.persist_path, data);
        }
    }

    fn load(&self) {
        if let Ok(data) = std::fs::read(&self.inner.persist_path) {
            if let Ok(map) = serde_json::from_slice::<HashMap<String, u64>>(&data) {
                *self.inner.quotas.write().unwrap() = map;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_and_check() {
        let dir = std::env::temp_dir().join("aipg-quota-test.json");
        let _ = std::fs::remove_file(&dir);
        let q = QuotaService::new(dir.clone());
        assert_eq!(q.get("pc-1"), None);
        q.set("pc-1", 1000);
        assert_eq!(q.get("pc-1"), Some(1000));
        assert!(q.check("pc-1", 999).is_ok());
        assert_eq!(q.check("pc-1", 1000), Err(QuotaExceeded { limit: 1000 }));
        assert!(q.check("pc-2", 999_999).is_ok());
        q.set("pc-1", 0);
        assert_eq!(q.get("pc-1"), None);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn persists_across_reload() {
        let dir = std::env::temp_dir().join("aipg-quota-reload.json");
        let _ = std::fs::remove_file(&dir);
        {
            let q = QuotaService::new(dir.clone());
            q.set("pc-1", 500);
        }
        let q2 = QuotaService::new(dir.clone());
        assert_eq!(q2.get("pc-1"), Some(500));
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn all_lists_sorted() {
        let dir = std::env::temp_dir().join("aipg-quota-all.json");
        let _ = std::fs::remove_file(&dir);
        let q = QuotaService::new(dir.clone());
        q.set("pc-2", 100);
        q.set("pc-1", 200);
        let all = q.all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].member_id, "pc-1");
        assert_eq!(all[1].limit, 100);
        let _ = std::fs::remove_file(&dir);
    }
}
