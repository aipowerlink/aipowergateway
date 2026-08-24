//! lan-member-gateway：成员侧本地 gateway。
//!
//! 成员机运行 `--role client`：本机监听 127.0.0.1:port，把 OpenAI/Anthropic
//! 请求转发给组长。组长来源有两种：
//! 1. **局域网**：UDP 发现（DiscoveryClient）
//! 2. **跨网络**：Deep Link 解析（coord-client resolve → 静态组长注入）
//! 令牌换取（/auth/token）、鉴权与计量全部由组长完成；成员侧保持无状态透传。

use std::sync::{Arc, RwLock};

use super::discovery::{DiscoveryClient, LeaderInfo};

/// 成员侧 gateway：组长发现 + 请求转发。
#[derive(Clone)]
pub struct MemberGateway {
    discovery: DiscoveryClient,
    /// 静态组长（Deep Link 解析注入，跨网络场景；优先于 UDP 发现）。
    static_leader: Arc<RwLock<Option<LeaderInfo>>>,
    http: reqwest::Client,
}

impl MemberGateway {
    pub fn new(discovery: DiscoveryClient) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self {
            discovery,
            static_leader: Arc::new(RwLock::new(None)),
            http,
        }
    }

    /// 注入静态组长（Deep Link 解析结果；跨网络场景，不经 UDP 发现）。
    pub fn set_static_leader(&self, leader: LeaderInfo) {
        *self.static_leader.write().unwrap() = Some(leader);
    }

    /// 当前在线组长数量（静态组长计 1，若无静态则统计 UDP 发现）。
    pub fn leader_count(&self) -> usize {
        if self.static_leader.read().unwrap().is_some() {
            return 1;
        }
        self.discovery.leaders().iter().filter(|l| l.online).count()
    }

    /// 组长摘要（用于本地状态展示）。
    pub fn leader_summary(&self) -> String {
        if let Some(l) = self.static_leader.read().unwrap().as_ref() {
            return format!("[deep-link] {}@{}:{}", l.name, l.address, l.share_port.filter(|p| *p > 0).unwrap_or(l.api_port));
        }
        let list = self.discovery.leaders();
        match list.iter().find(|l| l.online) {
            Some(l) => format!("{}@{}:{}", l.name, l.address, l.share_port.filter(|p| *p > 0).unwrap_or(l.api_port)),
            None => if list.is_empty() { "none".to_string() } else { "offline".to_string() },
        }
    }

    /// 选择组长：静态组长（跨网络）优先；否则 UDP 发现中最近在线的。
    fn pick_leader(&self) -> Option<LeaderInfo> {
        if let Some(l) = self.static_leader.read().unwrap().as_ref() {
            return Some(l.clone());
        }
        let mut list: Vec<LeaderInfo> = self.discovery.leaders().into_iter().filter(|l| l.online).collect();
        list.sort_by_key(|l| std::cmp::Reverse(l.last_seen));
        list.into_iter().next()
    }

    /// 透传转发：路径 + 方法 + 可选 Bearer + 可选原始 body，返回组长 (状态码, 响应体)。
    pub async fn proxy(
        &self,
        path: &str,
        auth: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> Result<(u16, Vec<u8>), String> {
        let leader = self.pick_leader().ok_or_else(|| "no leader available (LAN discovery empty + no deep-link target)".to_string())?;
        let base = leader.link_base();
        let url = format!("{}{}", base.trim_end_matches('/'), path);
        let method = if body.is_some() { reqwest::Method::POST } else { reqwest::Method::GET };
        let mut req = self.http.request(method, &url);
        if let Some(a) = auth {
            req = req.header(reqwest::header::AUTHORIZATION, a);
        }
        req = req.header(reqwest::header::CONTENT_TYPE, "application/json");
        if let Some(b) = body {
            req = req.body(b);
        }
        let resp = req.send().await
            .map_err(|e| format!("leader unreachable: {e}"))?;
        let status = resp.status().as_u16();
        let bytes = resp.bytes().await.map_err(|e| format!("read response: {e}"))?.to_vec();
        tracing::debug!("member gateway -> {url} [{status}]");
        Ok((status, bytes))
    }
}
