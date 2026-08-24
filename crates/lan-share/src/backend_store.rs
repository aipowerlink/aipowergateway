//! BackendStore：data_dir/backends.yaml 读写（服务端配置持久化）。
//!
//! 对齐 DeepSeek Harness 的配置方式：
//! - 文件形态：providers 列表（providers: [...]），键值与 DSH dsh.yaml 一致
//! - 启动时 CLI/环境变量条目作为初始补齐，面板保存后固化到文件

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

use crate::backend::BackendEntry;

/// backends.yaml 顶层结构。
#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct FileSchema {
    providers: Vec<BackendEntry>,
}

/// 后端配置存储（内存 + 文件）。
pub struct BackendStore {
    path: PathBuf,
    entries: RwLock<Vec<BackendEntry>>,
}

impl BackendStore {
    /// 打开存储：读取已有文件；`initial`（启动时经 --backend/环境变量解析的条目）
    /// 仅补齐文件中缺失的条目（不写盘，面板首次保存时固化）。
    pub fn new(path: PathBuf, initial: Vec<BackendEntry>) -> Self {
        let mut loaded = Self::load_entries(&path);
        let mut known: HashMap<String, ()> = loaded.iter().map(|e| (e.backend_id(), ())).collect();
        for e in initial {
            if !known.contains_key(&e.backend_id()) {
                loaded.push(e);
                known.insert(loaded.last().unwrap().backend_id(), ());
            }
        }
        Self { path, entries: RwLock::new(loaded) }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 按 id 排序返回全部条目。
    pub fn list(&self) -> Vec<BackendEntry> {
        let mut v = self.entries.read().unwrap().clone();
        v.sort_by(|a, b| a.backend_id().cmp(&b.backend_id()));
        v
    }

    /// 新增或替换（按 backend_id）；不落盘，需显式 save。
    pub fn upsert(&self, e: BackendEntry) {
        let id = e.backend_id();
        let mut w = self.entries.write().unwrap();
        if let Some(pos) = w.iter().position(|x| x.backend_id() == id) {
            w[pos] = e;
        } else {
            w.push(e);
        }
    }

    /// 删除（按 backend_id）；返回是否命中。
    pub fn remove(&self, id: &str) -> bool {
        let mut w = self.entries.write().unwrap();
        let before = w.len();
        w.retain(|x| x.backend_id() != id);
        w.len() != before
    }

    /// 落盘（yaml 序列化 → 临时文件 → rename；Windows 先删旧再改名）。
    pub fn save(&self) -> anyhow::Result<()> {
        let entries = self.list();
        let schema = FileSchema { providers: entries };
        let data = serde_yaml::to_string(&schema)?;
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = self.path.with_extension("yaml.tmp");
        std::fs::write(&tmp, &data)?;
        let _ = std::fs::remove_file(&self.path);
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn load_entries(path: &Path) -> Vec<BackendEntry> {
        if let Ok(data) = std::fs::read(path) {
            if let Ok(schema) = serde_yaml::from_slice::<FileSchema>(&data) {
                return schema.providers;
            }
        }
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk(provider: &str, id: &str, key: Option<&str>, model: Option<&str>, url: Option<&str>) -> BackendEntry {
        BackendEntry {
            provider: provider.to_string(),
            id: Some(id.to_string()),
            api_key: key.map(|s| s.to_string()),
            api_key_env: None,
            model: model.map(|s| s.to_string()),
            base_url: url.map(|s| s.to_string()),
        }
    }

    #[test]
    fn upsert_remove_roundtrip() {
        let tmp = std::env::temp_dir().join("aipg-bke-test.yaml");
        let _ = std::fs::remove_file(&tmp);
        let s = BackendStore::new(tmp.clone(), vec![]);
        s.upsert(mk("deepseek", "ds", Some("sk-1"), Some("deepseek-chat"), None));
        s.upsert(mk("kimi", "kimi", None, None, None));
        assert_eq!(s.list().len(), 2);
        s.upsert(mk("deepseek", "ds", Some("sk-2"), Some("deepseek-chat"), None));
        assert_eq!(s.list().len(), 2);
        assert_eq!(s.list().iter().find(|e| e.backend_id() == "ds").unwrap().api_key.as_deref(), Some("sk-2"));
        assert!(s.remove("ds"));
        assert_eq!(s.list().len(), 1);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn saves_and_reloads() {
        let tmp = std::env::temp_dir().join("aipg-bke-save.yaml");
        let _ = std::fs::remove_file(&tmp);
        {
            let s = BackendStore::new(tmp.clone(), vec![]);
            s.upsert(mk("zhipu", "glm", Some("sk-g"), Some("glm-4"), None));
            s.save().unwrap();
        }
        let s2 = BackendStore::new(tmp.clone(), vec![]);
        let rows = s2.list();
        let row = rows.iter().find(|e| e.backend_id() == "glm").expect("reloaded");
        assert_eq!(row.provider, "zhipu");
        assert_eq!(row.api_key.as_deref(), Some("sk-g"));
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn initial_only_fills_missing() {
        let tmp = std::env::temp_dir().join("aipg-bke-init.yaml");
        let _ = std::fs::remove_file(&tmp);
        {
            let s = BackendStore::new(tmp.clone(), vec![mk("mock", "mock", None, None, None)]);
            assert_eq!(s.list().len(), 1);
            s.save().unwrap();
        }
        let s2 = BackendStore::new(tmp.clone(), vec![
            mk("mock", "mock", None, None, None),
            mk("deepseek", "ds", None, None, None),
        ]);
        assert_eq!(s2.list().len(), 2);
        let _ = std::fs::remove_file(&tmp);
    }
}