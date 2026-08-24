//! lan-auth：Bearer token 签发/吊销、禁止名单（免密接入，黑名单持久化到 banned.json）。

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;
use serde::{Deserialize, Serialize};

use aipg_runtime::RuntimeResult;

/// 会话令牌。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    /// 令牌（Bearer 值）。
    pub token: String,
    /// 成员 id（机器名）。
    pub member_id: String,
    /// 机器名。
    pub machine_name: String,
    /// 显示名。
    pub display_name: String,
    /// 过期时间（unix 秒）。
    pub expires_at: u64,
    /// 签发时间。
    pub issued_at: u64,
}

/// 黑名单磁盘格式。
#[derive(Debug, Default, Serialize, Deserialize)]
struct BannedState {
    members: Vec<String>,
    ips: Vec<String>,
}

/// 会话磁盘格式（token 持久化：重启后 key 保持有效）。
#[derive(Debug, Serialize, Deserialize)]
struct SessionsState {
    sessions: Vec<Session>,
}

/// 鉴权服务（免密：成员声明机器名即签发 token；revoke = 拉黑并持久化）。
#[derive(Clone)]
pub struct AuthService {
    inner: Arc<AuthInner>,
}

struct AuthInner {
    /// token -> session。
    sessions: RwLock<HashMap<String, Session>>,
    /// 禁止名单（member_id）。
    banned: RwLock<HashSet<String>>,
    /// 禁止名单（IP）。
    banned_ips: RwLock<HashSet<String>>,
    /// token 有效期。
    ttl_secs: u64,
    /// 黑名单持久化路径（None = 不落盘，用于测试）。
    persist_path: Option<PathBuf>,
    /// 会话持久化路径（None = 不落盘，重启后 token 失效）。
    sessions_path: Option<PathBuf>,
}

impl AuthService {
    pub fn new(ttl_secs: u64, persist_path: Option<PathBuf>) -> Self {
        Self::new_with_store(ttl_secs, persist_path, None)
    }

    pub fn new_with_store(ttl_secs: u64, persist_path: Option<PathBuf>, sessions_path: Option<PathBuf>) -> Self {
        let svc = Self {
            inner: Arc::new(AuthInner {
                sessions: RwLock::new(HashMap::new()),
                banned: RwLock::new(HashSet::new()),
                banned_ips: RwLock::new(HashSet::new()),
                ttl_secs,
                persist_path,
                sessions_path,
            }),
        };
        svc.load();
        svc.load_sessions();
        svc
    }

    /// 免密签发 token（被禁 IP 或黑名单成员拒绝——换 IP 也无法绕过）。
    /// 幂等：同机器已有未过期 token 时直接复用——key 保持稳定，其他软件反复调用也换不掉。
    pub fn issue(&self, machine_name: &str, display_name: &str, ip: &str) -> RuntimeResult<Session> {
        if self.is_banned(ip) || self.is_member_banned(machine_name) {
            return Err(aipg_runtime::RuntimeError::Auth("banned".to_string()));
        }
        let member_id = format!("{}", machine_name);
        let now = now_secs();
        {
            let sessions = self.inner.sessions.read().unwrap();
            if let Some(existing) = sessions.values().find(|s| s.member_id == member_id && s.expires_at > now) {
                return Ok(existing.clone());
            }
        }
        let session = Session {
            token: gen_token(),
            member_id: member_id.clone(),
            machine_name: machine_name.to_string(),
            display_name: if display_name.is_empty() { machine_name.to_string() } else { display_name.to_string() },
            expires_at: now + self.inner.ttl_secs,
            issued_at: now,
        };
        self.inner.sessions.write().unwrap().insert(session.token.clone(), session.clone());
        self.save_sessions();
        Ok(session)
    }

    /// 显式轮换：吊销该机器全部旧 token 后签发新 token（仅页面「重新换取」主动触发）。
    pub fn rotate(&self, machine_name: &str, display_name: &str, ip: &str) -> RuntimeResult<Session> {
        {
            let mut sessions = self.inner.sessions.write().unwrap();
            sessions.retain(|_, s| s.member_id != machine_name);
        }
        self.save_sessions();
        self.issue(machine_name, display_name, ip)
    }

    /// 校验 token，返回会话（过期/被踢/被禁均拒绝）。
    pub fn verify(&self, token: &str) -> Option<Session> {
        let sessions = self.inner.sessions.read().unwrap();
        let s = sessions.get(token)?.clone();
        if s.expires_at < now_secs() {
            return None;
        }
        if self.is_member_banned(&s.member_id) {
            return None;
        }
        Some(s)
    }

    /// 拉黑：禁该成员与来源 IP，吊销其全部 token（持久化）。
    pub fn revoke_member(&self, member_id: &str, ip: &str) {
        self.inner.banned.write().unwrap().insert(member_id.to_string());
        if !ip.is_empty() {
            self.inner.banned_ips.write().unwrap().insert(ip.to_string());
        }
        let mut sessions = self.inner.sessions.write().unwrap();
        sessions.retain(|_, s| s.member_id != member_id);
        drop(sessions);
        self.save();
        self.save_sessions();
    }

    /// 解禁：移除成员与对应来源 IP 的拉黑（持久化）。
    pub fn unban(&self, member_id: &str, ip: &str) {
        self.inner.banned.write().unwrap().remove(member_id);
        if !ip.is_empty() {
            self.inner.banned_ips.write().unwrap().remove(ip);
        }
        self.save();
    }

    /// 成员是否在黑名单。
    pub fn is_member_banned(&self, member_id: &str) -> bool {
        self.inner.banned.read().unwrap().contains(member_id)
    }

    pub fn is_banned(&self, ip: &str) -> bool {
        self.inner.banned_ips.read().unwrap().contains(ip)
    }

    pub fn session_count(&self) -> usize {
        self.inner.sessions.read().unwrap().len()
    }

    fn load(&self) {
        let Some(path) = &self.inner.persist_path else { return };
        if let Ok(data) = std::fs::read(path) {
            if let Ok(s) = serde_json::from_slice::<BannedState>(&data) {
                *self.inner.banned.write().unwrap() = s.members.into_iter().collect();
                *self.inner.banned_ips.write().unwrap() = s.ips.into_iter().collect();
            }
        }
    }

    fn save(&self) {
        let Some(path) = &self.inner.persist_path else { return };
        let mut members = self.inner.banned.read().unwrap().iter().cloned().collect::<Vec<_>>();
        let mut ips = self.inner.banned_ips.read().unwrap().iter().cloned().collect::<Vec<_>>();
        members.sort();
        ips.sort();
        if let Ok(data) = serde_json::to_vec(&BannedState { members, ips }) {
            let _ = std::fs::write(path, data);
        }
    }

    /// 重启后恢复持久化 token（未过期、未拉黑者继续有效）。
    fn load_sessions(&self) {
        let Some(path) = &self.inner.sessions_path else { return };
        if let Ok(data) = std::fs::read(path) {
            if let Ok(s) = serde_json::from_slice::<SessionsState>(&data) {
                let now = now_secs();
                let mut sessions = self.inner.sessions.write().unwrap();
                for sess in s.sessions {
                    if sess.expires_at > now {
                        sessions.insert(sess.token.clone(), sess);
                    }
                }
            }
        }
    }

    fn save_sessions(&self) {
        let Some(path) = &self.inner.sessions_path else { return };
        let sessions = self.inner.sessions.read().unwrap();
        let state = SessionsState { sessions: sessions.values().cloned().collect() };
        if let Ok(data) = serde_json::to_vec(&state) {
            let _ = std::fs::write(path, data);
        }
    }
}

fn gen_token() -> String {
    let mut rng = rand::rng();
    let bytes: [u8; 32] = rng.random();
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn issue_and_verify() {
        let a = AuthService::new(3600, None);
        let s = a.issue("pc-1", "alice", "10.0.0.2").unwrap();
        assert_eq!(s.display_name, "alice");
        let v = a.verify(&s.token);
        assert!(v.is_some());
    }

    #[test]
    fn issue_is_idempotent_per_machine() {
        let a = AuthService::new(3600, None);
        let s1 = a.issue("pc-1", "", "10.0.0.2").unwrap();
        // 同机器再次签发 → 复用同一 token（key 保持稳定，其他软件换不掉）
        let s2 = a.issue("pc-1", "", "10.0.0.3").unwrap();
        assert_eq!(s1.token, s2.token, "同机器应复用同一有效 token");
        // 不同机器 → 各自独立 token
        let s3 = a.issue("pc-2", "", "10.0.0.9").unwrap();
        assert_ne!(s1.token, s3.token, "不同机器应各自持有 token");
    }

    #[test]
    fn rotate_replaces_token_and_old_dies() {
        let a = AuthService::new(3600, None);
        let s1 = a.issue("pc-1", "", "10.0.0.2").unwrap();
        let s2 = a.rotate("pc-1", "", "10.0.0.2").unwrap();
        assert_ne!(s1.token, s2.token, "轮换后应签发新 token");
        assert!(a.verify(&s1.token).is_none(), "旧 token 应失效");
        assert!(a.verify(&s2.token).is_some());
    }

    #[test]
    fn sessions_persist_across_reload() {
        let dir = std::env::temp_dir().join("aipg-auth-sessions.json");
        let _ = std::fs::remove_file(&dir);
        {
            let a = AuthService::new_with_store(3600, None, Some(dir.clone()));
            let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
            assert!(a.verify(&s.token).is_some());
        }
        {
            // 重启（重建实例）后旧 token 仍有效——key 不丢
            let a = AuthService::new_with_store(3600, None, Some(dir.clone()));
            let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
            assert!(a.verify(&s.token).is_some());
        }
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn expired_persisted_sessions_are_dropped() {
        let dir = std::env::temp_dir().join("aipg-auth-sess-exp.json");
        let _ = std::fs::remove_file(&dir);
        let ttl = 1;
        let first: Option<String>;
        {
            let a = AuthService::new_with_store(ttl, None, Some(dir.clone()));
            let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
            first = Some(s.token.clone());
            std::thread::sleep(std::time::Duration::from_millis(2100));
        }
        {
            let a = AuthService::new_with_store(ttl, None, Some(dir.clone()));
            assert!(a.verify(&first.unwrap()).is_none(), "过期 token 不应恢复");
        }
        let _ = std::fs::remove_file(&dir);
    }

    #[test]
    fn issue_without_display_uses_machine_name() {
        let a = AuthService::new(3600, None);
        let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
        assert_eq!(s.display_name, "pc-1");
        assert_eq!(s.member_id, "pc-1");
    }

    #[test]
    fn revoke_kills_token() {
        let a = AuthService::new(3600, None);
        let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
        a.revoke_member(&s.member_id, "10.0.0.2");
        assert!(a.verify(&s.token).is_none());
        // 被拉黑成员再接入被拒
        assert!(a.issue("pc-1", "", "10.0.0.2").is_err());
    }

    #[test]
    fn banned_ip_rejected() {
        let a = AuthService::new(3600, None);
        a.revoke_member("pc-1", "10.0.0.9");
        assert!(a.issue("pc-2", "", "10.0.0.9").is_err());
        assert!(a.issue("pc-2", "", "10.0.0.8").is_ok());
    }

    #[test]
    fn expired_token_rejected() {
        let a = AuthService::new(1, None);
        let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
        // unix 秒精度：睡眠 2s 跨越至少一个整秒边界
        std::thread::sleep(std::time::Duration::from_millis(2100));
        assert!(a.verify(&s.token).is_none());
    }

    #[test]
    fn banned_persists_across_reload() {
        let dir = std::env::temp_dir().join("aipg-auth-ban.json");
        let _ = std::fs::remove_file(&dir);
        {
            let a = AuthService::new(3600, Some(dir.clone()));
            a.revoke_member("pc-1", "10.0.0.9");
            assert!(a.is_member_banned("pc-1"));
        }
        {
            let a = AuthService::new(3600, Some(dir.clone()));
            assert!(a.is_member_banned("pc-1"));
            assert!(a.issue("pc-1", "", "10.0.0.8").is_err());
            assert!(a.issue("pc-1", "", "10.0.0.9").is_err());
            // 解禁恢复
            a.unban("pc-1", "10.0.0.9");
            assert!(!a.is_member_banned("pc-1"));
            assert!(a.issue("pc-1", "", "10.0.0.9").is_ok());
        }
        {
            let a = AuthService::new(3600, Some(dir.clone()));
            assert!(!a.is_member_banned("pc-1"));
        }
        let _ = std::fs::remove_file(&dir);
    }
}
