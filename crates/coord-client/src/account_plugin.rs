//! coord-account 登录插件(匿名优先,可选增强)。
//!
//! 产品三原则:
//! 1. 不登录即可完整使用全部核心功能(共享/接入/策略/本地拦截)—— 插件默认关闭;
//! 2. 登录仅为增强(账号状态展示/设备绑定/跨机找回凭据),可随时关闭(`account logout`
//!    即时降级匿名,`config set account.enabled false` 关闭插件);
//! 3. 登录板块插件化可卸载 —— 本模块实现 runtime `Module`(MOD_COORD_ACCOUNT),
//!    遵循模块系统语义(optional 装配 / config 驱动 / Host.provide 服务注册),
//!    自定义角色可从模块清单移除实现彻底卸载。

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::account::{AccountClient, AccountClientConfig};

/// 账号会话(持久化 data_dir/account.json;明文 tokens 受本机文件权限保护)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AccountSession {
    /// 已登录用户名(仅登录后)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// 会话令牌(登录后)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    /// 本机设备是否已绑定到账号(1:1)。
    #[serde(default)]
    pub device_bound: bool,
    /// 绑定时的本机 shareId(展示用)。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub share_id: Option<String>,
}

impl AccountSession {
    pub fn is_logged_in(&self) -> bool {
        self.token.is_some()
    }

    /// 面向用户的一句话状态。
    pub fn summary(&self) -> String {
        if !self.is_logged_in() {
            "匿名模式(未登录)——全部核心功能可用,登录为可选增强".to_string()
        } else {
            let username = self.username.as_deref().unwrap_or("?");
            let bound = if self.device_bound {
                format!(", 本机设备已绑定(shareId={})", self.share_id.as_deref().unwrap_or("?"))
            } else {
                ", 本机设备未绑定(可运行 account bind)".to_string()
            };
            format!("已登录(账号: {username}{bound})")
        }
    }
}

/// 账号会话存储(data_dir/account.json)。
#[derive(Debug, Clone)]
pub struct AccountStore {
    path: PathBuf,
}

impl AccountStore {
    pub fn new(data_dir: &Path) -> Self {
        Self { path: data_dir.join("account.json") }
    }

    /// 读取会话(不存在返回空会话 = 匿名)。
    pub fn load(&self) -> AccountSession {
        std::fs::read(&self.path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, session: &AccountSession) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(session).map_err(std::io::Error::other)?;
        std::fs::write(&self.path, data)
    }

    /// 清除会话(登出 = 恢复匿名)。
    pub fn clear(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e),
        }
    }
}

/// 本机设备注册凭据(持久化 data_dir/device.json;供 account bind 与跨机找回)。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DeviceSession {
    pub device_token: String,
    pub share_id: String,
}

/// 设备凭据存储(data_dir/device.json)。
#[derive(Debug, Clone)]
pub struct DeviceStore {
    path: PathBuf,
}

impl DeviceStore {
    pub fn new(data_dir: &Path) -> Self {
        Self { path: data_dir.join("device.json") }
    }

    pub fn save(&self, device_token: &str, share_id: &str) -> std::io::Result<()> {
        let data = serde_json::to_vec_pretty(&DeviceSession {
            device_token: device_token.to_string(),
            share_id: share_id.to_string(),
        })
        .map_err(std::io::Error::other)?;
        std::fs::write(&self.path, data)
    }

    pub fn load(&self) -> Option<DeviceSession> {
        std::fs::read(&self.path)
            .ok()
            .and_then(|data| serde_json::from_slice(&data).ok())
    }
}

/// 装配后的登录插件:匿名关闭时为 None;开启时携带客户端与会话(重启后免重复登录)。
#[derive(Debug, Clone)]
pub struct AccountPlugin {
    pub client: AccountClient,
    pub store: AccountStore,
    pub session: AccountSession,
}

impl AccountPlugin {
    /// 装配登录插件(模块 apply 等价物,供 CLI 运行时装配点直接使用)。
    /// base_url 为账号/协调服务器地址;enabled=false 或地址为空 → None(匿名模式,不阻塞任何功能)。
    pub fn assemble(data_dir: &Path, base_url: &str, enabled: bool) -> Option<Self> {
        if !enabled || base_url.is_empty() {
            return None;
        }
        let store = AccountStore::new(data_dir);
        let client = AccountClient::new(AccountClientConfig { base_url: base_url.to_string(), ..Default::default() });
        let session = store.load();
        // 恢复持久化会话:重启后免重复登录
        if let Some(token) = session.token.as_deref() {
            client.restore_session(token);
        }
        Some(Self { client, store, session })
    }
}

/// runtime 模块实现(插件化卸载语义):注册为 MOD_COORD_ACCOUNT。
#[derive(Debug, Clone)]
pub struct AccountModule {
    /// 账号/协调服务器 base_url(空 = 不启用)。
    pub base_url: String,
    /// 插件开关(默认关闭 = 匿名优先)。
    pub enabled: bool,
    /// 会话存储目录。
    pub data_dir: PathBuf,
}

impl aipg_runtime::Module for AccountModule {
    fn name(&self) -> &'static str {
        aipg_runtime::MOD_COORD_ACCOUNT
    }

    /// 可选模块:装配失败/未启用不阻塞主体启动(匿名模式合法)。
    fn optional(&self) -> bool {
        true
    }

    fn default_config(&self) -> serde_json::Value {
        serde_json::json!({ "enabled": false, "base_url": "" })
    }

    fn apply(&self, ctx: aipg_runtime::ModuleContext<'_>) -> aipg_runtime::RuntimeResult<()> {
        let plugin = AccountPlugin::assemble(&self.data_dir, &self.base_url, self.enabled);
        match plugin {
            Some(p) => {
                ctx.host.provide("coord-account", p.client.clone());
                tracing::info!(username = ?p.session.username, "coord-account enabled (登录为可选增强)");
                Ok(())
            }
            None => {
                tracing::info!("coord-account disabled — 匿名模式,全部核心功能可用");
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("aipg-account-plugin-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn session_roundtrip() {
        let dir = tmp_dir("session");
        let store = AccountStore::new(&dir);
        assert!(!store.load().is_logged_in(), "默认匿名");

        let mut s = AccountSession::default();
        s.username = Some("alice".into());
        s.token = Some("tok-1".into());
        s.device_bound = true;
        s.share_id = Some("aipg-1".into());
        store.save(&s).unwrap();

        let loaded = store.load();
        assert!(loaded.is_logged_in());
        assert_eq!(loaded.username.as_deref(), Some("alice"));
        assert!(loaded.device_bound);
        assert_eq!(loaded.summary(), "已登录(账号: alice, 本机设备已绑定(shareId=aipg-1))");

        store.clear().unwrap();
        assert!(!store.load().is_logged_in(), "登出后恢复匿名");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn assemble_respects_enabled_and_url() {
        let dir = tmp_dir("assemble");
        // 关闭 → None(匿名模式)
        assert!(AccountPlugin::assemble(&dir, "http://127.0.0.1:6800", false).is_none());
        // 无地址 → None
        assert!(AccountPlugin::assemble(&dir, "", true).is_none());
        // 开启 → Some,且恢复持久化会话
        let store = AccountStore::new(&dir);
        let mut s = AccountSession::default();
        s.username = Some("bob".into());
        s.token = Some("tok-2".into());
        store.save(&s).unwrap();

        let p = AccountPlugin::assemble(&dir, "http://127.0.0.1:6800", true).unwrap();
        assert_eq!(p.session.username.as_deref(), Some("bob"), "重启后会话恢复");
        assert!(p.client.is_session_restored(), "客户端会话令牌已恢复");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn device_store_roundtrip() {
        let dir = tmp_dir("device");
        let store = DeviceStore::new(&dir);
        assert!(store.load().is_none(), "未注册前无凭据");
        store.save("dev-tok", "aipg-42").unwrap();
        let d = store.load().unwrap();
        assert_eq!(d.device_token, "dev-tok");
        assert_eq!(d.share_id, "aipg-42");

        let _ = std::fs::remove_dir_all(&dir);
    }
}