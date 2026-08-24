//! lan-member-registry：成员登记/在线状态/改名。

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

/// 成员。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Member {
    /// 成员 id（机器名）。
    pub member_id: String,
    /// 机器名。
    pub machine_name: String,
    /// 来源 IP。
    pub ip: String,
    /// 网关标识（所连组长，name:port）。
    #[serde(default)]
    pub gateway_id: String,
    /// 显示名。
    pub display_name: String,
    /// 最后心跳（unix 秒）。
    pub last_seen: u64,
    /// 接入时间。
    pub joined_at: u64,
    /// 在线。
    #[serde(default = "default_true")]
    pub online: bool,
}

fn default_true() -> bool { true }

impl Member {
    /// 在线判定（心跳超时离线，默认 90s）。
    pub fn is_online(&self, now: u64, timeout: u64) -> bool {
        self.online && now.saturating_sub(self.last_seen) <= timeout
    }
}

/// 成员注册表。
#[derive(Clone)]
pub struct MemberRegistry {
    inner: Arc<RwLock<HashMap<String, Member>>>,
    /// 心跳超时（秒）。
    pub timeout_secs: u64,
    /// 网关标识（本组长，name:port）。
    gateway_id: String,
}

impl MemberRegistry {
    pub fn new(timeout_secs: u64, gateway_id: &str) -> Self {
        Self { inner: Arc::new(RwLock::new(HashMap::new())), timeout_secs, gateway_id: gateway_id.to_string() }
    }

    /// 登记或刷新成员。
    pub fn upsert(&self, machine_name: &str, display_name: &str, ip: &str) -> Member {
        let now = now_secs();
        let mut map = self.inner.write().unwrap();
        let e = map.entry(machine_name.to_string()).or_insert_with(|| Member {
            member_id: machine_name.to_string(),
            machine_name: machine_name.to_string(),
            ip: ip.to_string(),
            gateway_id: self.gateway_id.clone(),
            display_name: if display_name.is_empty() { machine_name.to_string() } else { display_name.to_string() },
            last_seen: now,
            joined_at: now,
            online: true,
        });
        e.last_seen = now;
        e.online = true;
        if !ip.is_empty() { e.ip = ip.to_string(); }
        if !display_name.is_empty() { e.display_name = display_name.to_string(); }
        e.clone()
    }

    /// 改名。
    pub fn rename(&self, machine_name: &str, new_display: &str) -> bool {
        let mut map = self.inner.write().unwrap();
        if let Some(e) = map.get_mut(machine_name) {
            e.display_name = new_display.to_string();
            true
        } else {
            false
        }
    }

    /// 标记离线。
    pub fn mark_offline(&self, machine_name: &str) {
        let mut map = self.inner.write().unwrap();
        if let Some(e) = map.get_mut(machine_name) {
            e.online = false;
        }
    }

    /// 清理超时离线成员状态。
    pub fn sweep(&self) {
        let now = now_secs();
        let timeout = self.timeout_secs;
        let mut map = self.inner.write().unwrap();
        for e in map.values_mut() {
            if now.saturating_sub(e.last_seen) > timeout {
                e.online = false;
            }
        }
    }

    /// 全部成员（在线判定实时）。
    pub fn all(&self) -> Vec<Member> {
        let now = now_secs();
        let timeout = self.timeout_secs;
        let map = self.inner.read().unwrap();
        let mut v: Vec<Member> = map.values().cloned().collect();
        for m in &mut v {
            m.online = m.is_online(now, timeout);
        }
        v.sort_by(|a, b| a.joined_at.cmp(&b.joined_at));
        v
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upsert_and_rename() {
        let r = MemberRegistry::new(90, "aipowerlink-share:39091");
        let m = r.upsert("pc-1", "", "10.0.0.2");
        assert_eq!(m.display_name, "pc-1");
        assert!(r.rename("pc-1", "alice"));
        assert_eq!(r.all()[0].display_name, "alice");
    }

    #[test]
    fn offline_after_timeout() {
        let r = MemberRegistry::new(90, "aipowerlink-share:39091");
        r.upsert("pc-1", "", "10.0.0.2");
        // 手动把 last_seen 调老
        {
            let mut map = r.inner.write().unwrap();
            if let Some(e) = map.get_mut("pc-1") { e.last_seen = 0; }
        }
        r.sweep();
        assert!(!r.all()[0].online);
    }
}