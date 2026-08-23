//! 角色配置（Role Profile）：角色 = 命名的模块装配配置。
//! 参考 DSH agent-preset：trust 由所在目录决定（内置目录=system，用户目录=user）。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::{RuntimeError, RuntimeResult};

/// 角色 id 约束（对应 DSH PRESET_ID：^[a-z0-9][a-z0-9-]*$）。
pub const ROLE_ID_RE: &str = "^[a-z0-9][a-z0-9-]*$";

/// 信任级别。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Trust {
    /// 内置（随安装包，只读）。
    System,
    /// 用户（可编辑）。
    User,
}

/// 模块装配项。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleEntry {
    /// 是否启用。
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// 模块配置覆盖。
    #[serde(default)]
    pub config: serde_json::Value,
}

fn default_true() -> bool { true }

/// 角色配置文件结构。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoleProfile {
    /// 角色 id（目录名）。
    pub id: String,
    /// 显示名。
    #[serde(default)]
    pub name: Option<String>,
    /// 模块启用清单（模块名 → 配置）。
    #[serde(default)]
    pub modules: std::collections::HashMap<String, ModuleEntry>,
    /// 继承基底（可选，如 server/client）。
    #[serde(default)]
    pub base: Option<String>,
}

impl RoleProfile {
    /// 内置 server 角色。
    pub fn builtin_server() -> Self {
        let mut modules = std::collections::HashMap::new();
        for m in crate::Role::Server.default_modules() {
            modules.insert((*m).to_string(), ModuleEntry { enabled: true, config: serde_json::json!({}) });
        }
        Self { id: "server".into(), name: Some("内置组长".into()), modules, base: None }
    }

    /// 内置 client 角色。
    pub fn builtin_client() -> Self {
        let mut modules = std::collections::HashMap::new();
        for m in crate::Role::Client.default_modules() {
            modules.insert((*m).to_string(), ModuleEntry { enabled: true, config: serde_json::json!({}) });
        }
        Self { id: "client".into(), name: Some("内置组员".into()), modules, base: None }
    }

    /// 校验角色 id 合法。
    pub fn validate_id(id: &str) -> bool {
        let re = regex_lite::Regex::new(ROLE_ID_RE).unwrap();
        re.is_match(id)
    }
}

/// 角色管理器：内置 + 用户角色。
pub struct RoleManager {
    /// 用户角色目录。
    user_roles_dir: PathBuf,
}

impl RoleManager {
    pub fn new(data_dir: &Path) -> Self {
        Self { user_roles_dir: data_dir.join("roles") }
    }

    pub fn user_roles_dir(&self) -> &Path {
        &self.user_roles_dir
    }

    /// 内置角色列表（system trust，只读）。
    pub fn builtin_roles(&self) -> Vec<RoleProfile> {
        vec![RoleProfile::builtin_server(), RoleProfile::builtin_client()]
    }

    /// 用户角色列表（user trust，从目录发现）。
    pub fn user_roles(&self) -> Vec<RoleProfile> {
        let mut out = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.user_roles_dir) {
            for e in entries.flatten() {
                let dir = e.path();
                if !dir.is_dir() { continue; }
                let id = dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
                let file = dir.join("role.json");
                if let Ok(data) = std::fs::read(&file) {
                    if let Ok(profile) = serde_json::from_slice::<RoleProfile>(&data) {
                        out.push(profile);
                        continue;
                    }
                }
                // 文件缺失/损坏：仍登记（broken 语义，DSH 同款）
                out.push(RoleProfile { id, name: None, modules: Default::default(), base: None });
            }
        }
        out.sort_by(|a, b| a.id.cmp(&b.id));
        out
    }

    /// 全部角色（内置 + 用户），带 trust 标记。
    pub fn all(&self) -> Vec<(RoleProfile, Trust)> {
        let mut out: Vec<(RoleProfile, Trust)> = self.builtin_roles().into_iter().map(|r| (r, Trust::System)).collect();
        for r in self.user_roles() {
            out.push((r, Trust::User));
        }
        out
    }

    /// 查找角色（用户优先，其次内置）。
    pub fn find(&self, id: &str) -> Option<(RoleProfile, Trust)> {
        for (r, t) in self.all() {
            if r.id == id { return Some((r, t)); }
        }
        None
    }

    /// 复制角色为自定义（内置 → user）。
    pub fn clone_role(&self, from: &str, to: &str) -> RuntimeResult<RoleProfile> {
        if !RoleProfile::validate_id(to) {
            return Err(RuntimeError::RoleError(format!("invalid role id: {to} (must match {ROLE_ID_RE})")));
        }
        let src = self.find(from).ok_or_else(|| RuntimeError::RoleError(format!("source role not found: {from}")))?;
        let mut profile = src.0.clone();
        profile.id = to.to_string();
        profile.name = Some(if from == "server" { "自定义组长".to_string() } else if from == "client" { "自定义组员".to_string() } else { to.to_string() });
        // 继承基底
        profile.base = Some(from.to_string());
        self.save_user_role(&profile)?;
        Ok(profile)
    }

    /// 新建空自定义角色。
    pub fn new_role(&self, id: &str) -> RuntimeResult<RoleProfile> {
        if !RoleProfile::validate_id(id) {
            return Err(RuntimeError::RoleError(format!("invalid role id: {id} (must match {ROLE_ID_RE})")));
        }
        if self.find(id).is_some() {
            return Err(RuntimeError::RoleError(format!("role already exists: {id}")));
        }
        let profile = RoleProfile {
            id: id.to_string(),
            name: Some(id.to_string()),
            modules: Default::default(),
            base: None,
        };
        self.save_user_role(&profile)?;
        Ok(profile)
    }

    /// 保存用户角色。
    pub fn save_user_role(&self, profile: &RoleProfile) -> RuntimeResult<()> {
        let dir = self.user_roles_dir.join(&profile.id);
        std::fs::create_dir_all(&dir).map_err(|e| RuntimeError::Other(format!("role dir: {e}")))?;
        let data = serde_json::to_vec_pretty(profile).map_err(|e| RuntimeError::Other(format!("role json: {e}")))?;
        std::fs::write(dir.join("role.json"), data).map_err(|e| RuntimeError::Other(format!("role write: {e}")))?;
        Ok(())
    }

    /// 删除用户角色（内置拒绝）。
    pub fn delete_role(&self, id: &str) -> RuntimeResult<()> {
        if let Some((_, Trust::System)) = self.find(id) {
            return Err(RuntimeError::RoleError(format!("builtin role {id} is read-only; clone it to customize")));
        }
        let dir = self.user_roles_dir.join(id);
        if dir.exists() {
            std::fs::remove_dir_all(&dir).map_err(|e| RuntimeError::Other(format!("role rm: {e}")))?;
        }
        Ok(())
    }

    /// 修改角色（内置拒绝）。允许编辑 modules 清单与配置。
    pub fn edit_role(&self, id: &str, modules: std::collections::HashMap<String, ModuleEntry>) -> RuntimeResult<RoleProfile> {
        let (profile, trust) = self.find(id).ok_or_else(|| RuntimeError::RoleError(format!("role not found: {id}")))?;
        if trust == Trust::System {
            return Err(RuntimeError::RoleError(format!("builtin role {id} is read-only; clone it to customize")));
        }
        let mut profile = profile;
        profile.modules = modules;
        self.save_user_role(&profile)?;
        Ok(profile)
    }

    /// 解析角色启用的模块清单（含 base 继承，本角色禁用可覆盖 base 启用）。
    pub fn enabled_modules(&self, id: &str) -> RuntimeResult<Vec<String>> {
        let (profile, trust) = self.find(id).ok_or_else(|| RuntimeError::RoleError(format!("role not found: {id}")))?;
        let mut merged: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
        // 先继承 base
        if let Some(base) = &profile.base {
            if let Some((base_profile, _)) = self.find(base) {
                for (m, e) in &base_profile.modules {
                    merged.insert(m.clone(), e.enabled);
                }
            }
        }
        // 本角色覆盖（禁用 = 覆盖 base 的启用）
        for (m, e) in &profile.modules {
            merged.insert(m.clone(), e.enabled);
        }
        // 收集启用的
        let mut out: Vec<String> = merged.iter().filter(|(_, en)| **en).map(|(m, _)| m.clone()).collect();
        out.sort();
        if out.is_empty() && trust == Trust::System {
            // 内置空 modules 时回退默认模块集
            let defs = if id == "server" { crate::Role::Server.default_modules() } else { crate::Role::Client.default_modules() };
            out = defs.iter().map(|s| s.to_string()).collect();
        }
        Ok(out)
    }
}

/// 轻量 regex（避免引入 regex 依赖）。
mod regex_lite {
    pub struct Regex { pattern: String }

    impl Regex {
        pub fn new(p: &str) -> std::io::Result<Self> {
            Ok(Self { pattern: p.to_string() })
        }
        pub fn is_match(&self, s: &str) -> bool {
            // 简化：^[a-z0-9][a-z0-9-]*$ 的字符级检查
            if s.is_empty() { return false; }
            let bytes = s.as_bytes();
            if !(bytes[0].is_ascii_lowercase() || bytes[0].is_ascii_digit()) { return false; }
            bytes.iter().all(|&b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        }
        #[allow(dead_code)]
        fn _pattern(&self) -> &str { &self.pattern }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp() -> PathBuf {
        let d = std::env::temp_dir().join(format!("aipg-roles-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn builtin_roles_present() {
        let m = RoleManager::new(&tmp());
        assert_eq!(m.builtin_roles().len(), 2);
    }

    #[test]
    fn clone_builtin_to_user() {
        let dir = tmp();
        let m = RoleManager::new(&dir);
        let p = m.clone_role("server", "my-server").unwrap();
        assert_eq!(p.base.as_deref(), Some("server"));
        assert_eq!(m.find("my-server").unwrap().1, Trust::User);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn builtin_readonly() {
        let dir = tmp();
        let m = RoleManager::new(&dir);
        assert!(m.delete_role("server").is_err());
        assert!(m.edit_role("server", Default::default()).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn enabled_modules_with_base() {
        let dir = tmp();
        let m = RoleManager::new(&dir);
        let p = m.clone_role("server", "light").unwrap();
        let mut p = p;
        p.modules.insert("lan-usage".to_string(), ModuleEntry { enabled: false, config: serde_json::json!({}) });
        m.save_user_role(&p).unwrap();
        let mods = m.enabled_modules("light").unwrap();
        assert!(!mods.contains(&"lan-usage".to_string()));
        assert!(mods.contains(&"lan-share-server".to_string()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn id_validation() {
        assert!(RoleProfile::validate_id("my-role"));
        assert!(!RoleProfile::validate_id("My Role"));
        assert!(!RoleProfile::validate_id(""));
    }
}