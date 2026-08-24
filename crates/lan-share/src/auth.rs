//! lan-auth：Bearer token 签发/吊销、禁止名单（免密接入，0.2.0 起无访问密码）。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;

use aipg_runtime::RuntimeResult;

/// 会话令牌。
#[derive(Debug, Clone)]
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

/// 鉴权服务（免密：成员声明机器名即签发 token）。
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
}

impl AuthService {
    pub fn new(ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(AuthInner {
                sessions: RwLock::new(HashMap::new()),
                banned: RwLock::new(HashSet::new()),
                banned_ips: RwLock::new(HashSet::new()),
                ttl_secs,
            }),
        }
    }

    /// 免密签发 token（被禁 IP 拒绝）。
    pub fn issue(&self, machine_name: &str, display_name: &str, ip: &str) -> RuntimeResult<Session> {
        if self.is_banned(ip) {
            return Err(aipg_runtime::RuntimeError::Auth("banned".to_string()));
        }
        let member_id = format!("{}", machine_name);
        let now = now_secs();
        let session = Session {
            token: gen_token(),
            member_id: member_id.clone(),
            machine_name: machine_name.to_string(),
            display_name: if display_name.is_empty() { machine_name.to_string() } else { display_name.to_string() },
            expires_at: now + self.inner.ttl_secs,
            issued_at: now,
        };
        self.inner.sessions.write().unwrap().insert(session.token.clone(), session.clone());
        Ok(session)
    }

    /// 校验 token，返回会话（过期/被踢/被禁均拒绝）。
    pub fn verify(&self, token: &str) -> Option<Session> {
        let sessions = self.inner.sessions.read().unwrap();
        let s = sessions.get(token)?.clone();
        if s.expires_at < now_secs() {
            return None;
        }
        if self.inner.banned.read().unwrap().contains(&s.member_id) {
            return None;
        }
        Some(s)
    }

    /// 踢人：吊销该成员全部 token + 禁止名单。
    pub fn revoke_member(&self, member_id: &str, ip: &str) {
        self.inner.banned.write().unwrap().insert(member_id.to_string());
        if !ip.is_empty() {
            self.inner.banned_ips.write().unwrap().insert(ip.to_string());
        }
        let mut sessions = self.inner.sessions.write().unwrap();
        sessions.retain(|_, s| s.member_id != member_id);
    }

    pub fn is_banned(&self, ip: &str) -> bool {
        self.inner.banned_ips.read().unwrap().contains(ip)
    }

    pub fn session_count(&self) -> usize {
        self.inner.sessions.read().unwrap().len()
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
        let a = AuthService::new(3600);
        let s = a.issue("pc-1", "alice", "10.0.0.2").unwrap();
        assert_eq!(s.display_name, "alice");
        let v = a.verify(&s.token);
        assert!(v.is_some());
    }

    #[test]
    fn issue_without_display_uses_machine_name() {
        let a = AuthService::new(3600);
        let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
        assert_eq!(s.display_name, "pc-1");
        assert_eq!(s.member_id, "pc-1");
    }

    #[test]
    fn revoke_kills_token() {
        let a = AuthService::new(3600);
        let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
        a.revoke_member(&s.member_id, "10.0.0.2");
        assert!(a.verify(&s.token).is_none());
        // 被踢成员再接入被拒
        assert!(a.issue("pc-1", "", "10.0.0.2").is_err());
    }

    #[test]
    fn banned_ip_rejected() {
        let a = AuthService::new(3600);
        a.revoke_member("pc-1", "10.0.0.9");
        assert!(a.issue("pc-2", "", "10.0.0.9").is_err());
        assert!(a.issue("pc-2", "", "10.0.0.8").is_ok());
    }

    #[test]
    fn expired_token_rejected() {
        let a = AuthService::new(1);
        let s = a.issue("pc-1", "", "10.0.0.2").unwrap();
        // unix 秒精度：睡眠 2s 跨越至少一个整秒边界
        std::thread::sleep(std::time::Duration::from_millis(2100));
        assert!(a.verify(&s.token).is_none());
    }
}
