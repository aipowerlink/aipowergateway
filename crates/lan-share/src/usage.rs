//! lan-usage：按成员计量 token（消费 API 响应 usage），SQLite 持久化。
//! 0.1.0 用内存 + JSON 文件持久化（SQLite 接入放阶段 6 配置库）。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

/// 单模型用量。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ModelUsage {
    /// 模型名。
    pub model: String,
    /// 该模型累计 prompt tokens。
    pub prompt_tokens: u64,
    /// 该模型累计 completion tokens。
    pub completion_tokens: u64,
    /// 该模型调用次数。
    pub calls: u64,
}

impl ModelUsage {
    pub fn total(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

use serde::{Deserialize, Serialize};

/// 成员用量。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MemberUsage {
    /// 成员 id（机器名）。
    pub member_id: String,
    /// 累计 prompt tokens。
    pub prompt_tokens: u64,
    /// 累计 completion tokens。
    pub completion_tokens: u64,
    /// 总调用次数。
    pub calls: u64,
    /// 按模型拆分的用量（model -> 累计 tokens）。
    #[serde(default)]
    pub model_tokens: HashMap<String, u64>,
}

impl MemberUsage {
    pub fn total(&self) -> u64 {
        self.prompt_tokens + self.completion_tokens
    }
}

/// 用量服务。
#[derive(Clone)]
pub struct UsageService {
    inner: Arc<UsageInner>,
}

struct UsageInner {
    by_member: RwLock<HashMap<String, MemberUsage>>,
    persist_path: PathBuf,
}

impl UsageService {
    pub fn new(persist_path: PathBuf) -> Self {
        let svc = Self {
            inner: Arc::new(UsageInner {
                by_member: RwLock::new(HashMap::new()),
                persist_path,
            }),
        };
        svc.load();
        svc
    }

    /// 记录一次调用用量（含模型维度）。
    pub fn record(&self, member_id: &str, model: &str, prompt_tokens: u64, completion_tokens: u64) {
        let mut map = self.inner.by_member.write().unwrap();
        let e = map.entry(member_id.to_string()).or_insert_with(|| MemberUsage {
            member_id: member_id.to_string(),
            ..Default::default()
        });
        e.prompt_tokens += prompt_tokens;
        e.completion_tokens += completion_tokens;
        e.calls += 1;
        if !model.is_empty() {
            *e.model_tokens.entry(model.to_string()).or_insert(0) += prompt_tokens + completion_tokens;
        }
        drop(map);
        self.save();
    }

    /// 查询全部成员用量。
    pub fn all(&self) -> Vec<MemberUsage> {
        let map = self.inner.by_member.read().unwrap();
        let mut v: Vec<MemberUsage> = map.values().cloned().collect();
        v.sort_by(|a, b| b.total().cmp(&a.total()));
        v
    }

    /// 查询单成员。
    pub fn get(&self, member_id: &str) -> Option<MemberUsage> {
        self.inner.by_member.read().unwrap().get(member_id).cloned()
    }

    /// 导出账单 CSV（按总量降序）：member_id,prompt,completion,total,calls。
    pub fn export_csv(&self) -> String {
        let mut out = String::from("member_id,prompt_tokens,completion_tokens,total_tokens,calls\n");
        for u in self.all() {
            out.push_str(&format!(
                "{},{},{},{},{}\n",
                u.member_id, u.prompt_tokens, u.completion_tokens, u.total(), u.calls,
            ));
        }
        out
    }

    fn save(&self) {
        let map = self.inner.by_member.read().unwrap();
        if let Ok(data) = serde_json::to_vec(&*map) {
            let _ = std::fs::write(&self.inner.persist_path, data);
        }
    }

    fn load(&self) {
        if let Ok(data) = std::fs::read(&self.inner.persist_path) {
            if let Ok(map) = serde_json::from_slice::<HashMap<String, MemberUsage>>(&data) {
                *self.inner.by_member.write().unwrap() = map;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_query() {
        let dir = std::env::temp_dir().join("aipg-usage-test.json");
        let _ = std::fs::remove_file(&dir);
        let u = UsageService::new(dir.clone());
        u.record("pc-1", "mock-7b", 10, 20);
        u.record("pc-1", "mock-7b", 5, 5);
        u.record("pc-2", "deepseek-chat", 100, 50);
        let all = u.all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].member_id, "pc-2"); // 总量 150 最大
        assert_eq!(u.get("pc-1").unwrap().calls, 2);
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn model_dimension_tracked() {
        let dir = std::env::temp_dir().join("aipg-usage-model.json");
        let _ = std::fs::remove_file(&dir);
        let u = UsageService::new(dir.clone());
        u.record("pc-1", "deepseek-chat", 10, 20);
        u.record("pc-1", "deepseek-chat", 2, 8);
        u.record("pc-1", "kimi", 100, 0);
        let m = u.get("pc-1").unwrap();
        assert_eq!(m.model_tokens.get("deepseek-chat").copied(), Some(40));
        assert_eq!(m.model_tokens.get("kimi").copied(), Some(100));
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn export_csv_format() {
        let dir = std::env::temp_dir().join("aipg-usage-csv.json");
        let _ = std::fs::remove_file(&dir);
        let u = UsageService::new(dir.clone());
        u.record("pc-1", "m", 10, 20);
        u.record("pc-2", "m", 1, 2);
        let csv = u.export_csv();
        let lines: Vec<&str> = csv.trim().lines().collect();
        assert_eq!(lines[0], "member_id,prompt_tokens,completion_tokens,total_tokens,calls");
        assert_eq!(lines.len(), 3);
        assert!(lines[1].starts_with("pc-1,")); // 按总量降序（pc-1=30 > pc-2=3）
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn persists_across_reload() {
        let dir = std::env::temp_dir().join("aipg-usage-reload.json");
        let _ = std::fs::remove_file(&dir);
        {
            let u = UsageService::new(dir.clone());
            u.record("pc-1", "m", 10, 20);
        }
        let u2 = UsageService::new(dir.clone());
        assert_eq!(u2.get("pc-1").unwrap().total(), 30);
        let _ = std::fs::remove_file(&dir);
    }
}