//! lan-identity：本机身份——机器名（默认）+ 显示名（可改）。

use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

/// 本机身份。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Identity {
    /// 机器名（不可改，作为成员 id）。
    pub machine_name: String,
    /// 显示名（可改，默认=机器名）。
    pub display_name: String,
}

impl Identity {
    /// 新建：默认显示名=机器名。
    pub fn new(machine_name: &str) -> Self {
        Self {
            machine_name: machine_name.to_string(),
            display_name: machine_name.to_string(),
        }
    }

    /// 修改显示名。
    pub fn rename(&mut self, new_display: &str) {
        self.display_name = new_display.to_string();
    }

    /// 持久化（JSON 到数据目录）。
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(self)?;
        std::fs::write(path, data)
    }

    /// 从磁盘加载（不存在则新建）。
    pub fn load_or_create(path: &std::path::Path, machine_name: &str) -> std::io::Result<Self> {
        if path.exists() {
            let data = std::fs::read(path)?;
            return Ok(serde_json::from_slice(&data).unwrap_or_else(|_| Self::new(machine_name)));
        }
        Ok(Self::new(machine_name))
    }
}

/// 线程安全身份持有者。
#[derive(Clone)]
pub struct IdentityHolder {
    inner: Arc<RwLock<Identity>>,
}

impl IdentityHolder {
    pub fn new(identity: Identity) -> Self {
        Self { inner: Arc::new(RwLock::new(identity)) }
    }

    pub fn get(&self) -> Identity {
        self.inner.read().unwrap().clone()
    }

    pub fn rename(&self, new_display: &str) {
        self.inner.write().unwrap().rename(new_display);
    }

    pub fn machine_name(&self) -> String {
        self.inner.read().unwrap().machine_name.clone()
    }

    pub fn display_name(&self) -> String {
        self.inner.read().unwrap().display_name.clone()
    }
}

/// 获取本机机器名（hostname）。
pub fn hostname() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .or_else(|_| hostname_fallback())
        .unwrap_or_else(|_| "unknown-machine".to_string())
}

fn hostname_fallback() -> Result<String, std::io::Error> {
    // 跨平台兜底：直接读环境或默认
    Ok("machine".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_display_is_machine() {
        let id = Identity::new("pc-1");
        assert_eq!(id.display_name, "pc-1");
    }

    #[test]
    fn rename_updates_display() {
        let mut id = Identity::new("pc-1");
        id.rename("alice");
        assert_eq!(id.display_name, "alice");
        assert_eq!(id.machine_name, "pc-1");
    }

    #[test]
    fn persist_roundtrip() {
        let path = std::env::temp_dir().join("aipg-identity-test.json");
        let _ = std::fs::remove_file(&path);
        let mut id = Identity::new("pc-1");
        id.rename("alice");
        id.save(&path).unwrap();
        let loaded = Identity::load_or_create(&path, "pc-1").unwrap();
        assert_eq!(loaded.display_name, "alice");
        let _ = std::fs::remove_file(&path);
    }
}