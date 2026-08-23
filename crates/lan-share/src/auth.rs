//! lan-auth：密码 → Bearer token 签发/吊销、改密、禁止名单。

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use rand::Rng;
use sha2::{Digest, Sha256};

use aipg_runtime::RuntimeResult;

/// 会话令牌。
#[derive(Debug, Clone)]
pub struct Session {
    /// 令牌（Bearer 值）。
    pub token: String,
    /// 成员 id（机器名。指纹）。
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

/// 鉴权服务。
#[derive(Clone)]
pub struct AuthService {
    inner: Arc<AuthInner>,
}

struct AuthInner {
    /// 密码哈希。
    password_hash: RwLock<String>,
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
    pub fn new(password: &str, ttl_secs: u64) -> Self {
        Self {
            inner: Arc::new(AuthInner {
                password_hash: RwLock::new(hash_password(password)),
                sessions: RwLock::new(HashMap::new()),
                banned: RwLock::new(HashSet::new()),
                banned_ips: RwLock::new(HashSet::new()),
                ttl_secs,
            }),
        }
    }

    /// 校验密码并签发 token。
    pub fn issue(&self, password: &str, machine_name: &str, display_name: &str, ip: &str) -> RuntimeResult<Session> {
        if self.is_banned(ip) {
            return Err(aipg_runtime::RuntimeError::Auth("banned".to_string()));
        }
        let expected = self.inner.password_hash.read().unwrap().clone();
        if hash_password(password) != expected {
            return Err(aipg_runtime::RuntimeError::Auth("wrong password".to_string()));
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

    /// 修改密码：旧 token 全部失效。
    pub fn change_password(&self, new_password: &str) {
        *self.inner.password_hash.write().unwrap() = hash_password(new_password);
        self.inner.sessions.write().unwrap().clear();
    }

    /// 广播指纹（密码哈希前 N 位 hex）。
    pub fn fingerprint(&self, n: usize) -> String {
        let h = self.inner.password_hash.read().unwrap().clone();
        h.chars().take(n).collect()
    }

    pub fn is_banned(&self, ip: &str) -> bool {
        self.inner.banned_ips.read().unwrap().contains(ip)
    }

    pub fn session_count(&self) -> usize {
        self.inner.sessions.read().unwrap().len()
    }
}

fn hash_password(pw: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(pw.as_bytes());
    let digest = hasher.finalize();
    digest.iter().map(|b| format!("{:02x}", b)).collect()
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
        let a = AuthService::new("secret", 3600);
        let s = a.issue("secret", "pc-1", "alice", "10.0.0.2").unwrap();
        assert_eq!(s.display_name, "alice");
        let v = a.verify(&s.token);
        assert!(v.is_some());
    }

    #[test]
    fn wrong_password_rejected() {
        let a = AuthService::new("secret", 3600);
        assert!(a.issue("wrong", "pc-1", "", "10.0.0.2").is_err());
    }

    #[test]
    fn revoke_kills_token() {
        let a = AuthService::new("secret", 3600);
        let s = a.issue("secret", "pc-1", "", "10.0.0.2").unwrap();
        a.revoke_member(&s.member_id, "10.0.0.2");
        assert!(a.verify(&s.token).is_none());
    }

    #[test]
    fn change_password_invalidates_all() {
        let a = AuthService::new("secret", 3600);
        let s = a.issue("secret", "pc-1", "", "10.0.0.2").unwrap();
        a.change_password("new-secret");
        assert!(a.verify(&s.token).is_none());
        let s2 = a.issue("new-secret", "pc-2", "", "10.0.0.3").unwrap();
        assert!(a.verify(&s2.token).is_some());
    }

    #[test]
    fn fingerprint_stable() {
        let a = AuthService::new("secret", 3600);
        assert_eq!(a.fingerprint(8), a.fingerprint(8));
        assert!(a.fingerprint(8).len() <= 8);
    }
}