//! lan-discovery-client：UDP 监听广播 + 主动扫描，维护组长列表。

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// 发现的组长信息。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderInfo {
    /// 服务名。
    pub name: String,
    /// API 端口。
    pub api_port: u16,
    /// gateway 间共享通道端口（成员 gateway 经此端口接入组长 gateway；旧组长无宣告时为 None 回落到 api_port）。
    #[serde(default)]
    pub share_port: Option<u16>,
    /// 指纹（密码哈希前 N 位）。
    pub fingerprint: String,
    /// 组长来源地址（IP）。
    pub address: String,
    /// 最近发现时间。
    pub last_seen: u64,
    /// 在线。
    pub online: bool,
}

impl LeaderInfo {
    /// 完整 API 地址。
    pub fn api_base(&self) -> String {
        format!("http://{}:{}", self.address, self.api_port)
    }
    /// 完整网关接入地址（共享通道端口优先，老组长回落 api_port）。
    pub fn link_base(&self) -> String {
        let p = self.share_port.filter(|p| *p > 0).unwrap_or(self.api_port);
        format!("http://{}:{}", self.address, p)
    }
    /// 服务标识（name@ip:port）。
    pub fn id(&self) -> String {
        format!("{}@{}:{}", self.name, self.address, self.api_port)
    }
}

/// 发现服务配置。
#[derive(Debug, Clone)]
pub struct DiscoveryConfig {
    /// 监听端口（组长广播端口）。
    pub port: u16,
    /// 在线阈值（秒，超过标记离线）。
    pub online_threshold_secs: u64,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self { port: 39090, online_threshold_secs: 30 }
    }
}

/// 发现客户端：UDP 监听 + 扫描。
#[derive(Clone)]
pub struct DiscoveryClient {
    #[allow(dead_code)]
    cfg: DiscoveryConfig,
    leaders: Arc<RwLock<HashMap<String, LeaderInfo>>>,
}

impl DiscoveryClient {
    pub fn new(cfg: DiscoveryConfig) -> Self {
        Self { cfg, leaders: Arc::new(RwLock::new(HashMap::new())) }
    }

    /// 启动后台监听（接收组长广播）。
    pub fn start_listen(&self) {
        let cfg = self.cfg.clone();
        let leaders = self.leaders.clone();
        tokio::spawn(async move {
            let socket = match tokio::net::UdpSocket::bind(format!("0.0.0.0:{}", cfg.port)).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("discovery listen bind :{} failed: {e}", cfg.port);
                    return;
                }
            };
            tracing::info!("discovery listening on UDP :{}", cfg.port);
            let mut buf = [0u8; 4096];
            loop {
                match socket.recv_from(&mut buf).await {
                    Ok((len, src)) => {
                        let text = String::from_utf8_lossy(&buf[..len]);
                        handle_announce(&leaders, &text, src);
                    }
                    Err(e) => {
                        tracing::debug!("discovery recv error: {e}");
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        });
    }

    /// 主动广播 PING 一次（应对启动时序）。
    pub fn ping_once(&self) {
        let cfg = self.cfg.clone();
        tokio::spawn(async move {
            let socket = match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
                Ok(s) => s,
                Err(_) => return,
            };
            let _ = socket.set_broadcast(true);
            let addr: SocketAddr = format!("255.255.255.255:{}", cfg.port).parse().unwrap_or_else(|_| {
                SocketAddr::from(([255, 255, 255, 255], cfg.port))
            });
            let _ = socket.send_to(b"AIPG_PING", addr).await;
        });
    }

    /// 组长列表（去重 + 在线判定）。
    pub fn leaders(&self) -> Vec<LeaderInfo> {
        let now = now_secs();
        let threshold = self.cfg.online_threshold_secs;
        let map = self.leaders.read().unwrap();
        let mut v: Vec<LeaderInfo> = map.values().cloned().collect();
        for l in &mut v {
            l.online = now.saturating_sub(l.last_seen) <= threshold;
        }
        v.sort_by(|a, b| a.id().cmp(&b.id()));
        v
    }

    pub fn count(&self) -> usize {
        self.leaders.read().unwrap().len()
    }
}

/// 处理 AIPG_ANNOUNCE 广播。
fn handle_announce(leaders: &Arc<RwLock<HashMap<String, LeaderInfo>>>, text: &str, src: SocketAddr) {
    let trimmed = text.trim();
    if trimmed == "AIPG_PING" {
        // 组员 ping：不处理（组长端广播端会响应？0.1.0 简化：组长已周期广播）
        return;
    }
    let parsed: serde_json::Value = match serde_json::from_str(trimmed) {
        Ok(v) => v,
        Err(_) => return,
    };
    if parsed.get("type").and_then(|t| t.as_str()) != Some("AIPG_ANNOUNCE") {
        return;
    }
    let name = parsed.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let api_port = parsed.get("api_port").and_then(|v| v.as_u64()).unwrap_or(0) as u16;
    let share_port = parsed.get("share_port").and_then(|v| v.as_u64()).map(|v| v as u16).filter(|p| *p > 0);
    let fingerprint = parsed.get("fingerprint").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if name.is_empty() || api_port == 0 {
        return;
    }
    let info = LeaderInfo {
        name: name.clone(),
        api_port,
        share_port,
        fingerprint,
        address: src.ip().to_string(),
        last_seen: now_secs(),
        online: true,
    };
    let mut map = leaders.write().unwrap();
    map.insert(info.id(), info);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or(Duration::ZERO).as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr};

    #[test]
    fn parse_announce_with_share_channel() {
        let leaders: Arc<RwLock<HashMap<String, LeaderInfo>>> = Arc::new(RwLock::new(HashMap::new()));
        let src = SocketAddr::from((IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5)), 39090));
        handle_announce(&leaders, r#"{"type":"AIPG_ANNOUNCE","name":"aipowerlink-share","api_port":39091,"share_port":39092,"fingerprint":""}"#, src);
        let list = leaders.read().unwrap();
        let l = list.values().next().unwrap();
        assert_eq!(l.share_port, Some(39092));
        assert_eq!(l.link_base(), "http://192.168.1.5:39092");
    }

    #[test]
    fn parse_announce() {
        let leaders: Arc<RwLock<HashMap<String, LeaderInfo>>> = Arc::new(RwLock::new(HashMap::new()));
        let src = SocketAddr::from((IpAddr::V4(Ipv4Addr::new(192, 168, 1, 5)), 39090));
        handle_announce(&leaders, r#"{"type":"AIPG_ANNOUNCE","name":"aipowerlink-share","api_port":39091,"fingerprint":"abc123"}"#, src);
        let list = leaders.read().unwrap();
        assert_eq!(list.len(), 1);
        let l = list.values().next().unwrap();
        assert_eq!(l.name, "aipowerlink-share");
        assert_eq!(l.api_port, 39091);
        assert_eq!(l.address, "192.168.1.5");
        assert_eq!(l.api_base(), "http://192.168.1.5:39091");
        assert_eq!(l.share_port, None);
        assert_eq!(l.link_base(), "http://192.168.1.5:39091"); // 无共享通道宣告时回落 api_port
    }

    #[test]
    fn dedupe_same_leader() {
        let leaders: Arc<RwLock<HashMap<String, LeaderInfo>>> = Arc::new(RwLock::new(HashMap::new()));
        let src = SocketAddr::from((IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)), 39090));
        handle_announce(&leaders, r#"{"type":"AIPG_ANNOUNCE","name":"s1","api_port":39091,"fingerprint":"x"}"#, src);
        handle_announce(&leaders, r#"{"type":"AIPG_ANNOUNCE","name":"s1","api_port":39091,"fingerprint":"x"}"#, src);
        assert_eq!(leaders.read().unwrap().len(), 1);
    }
}