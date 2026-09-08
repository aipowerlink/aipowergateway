//! lan-member-gateway：成员侧本地 gateway。
//!
//! 成员机运行 `--role client`：本机监听 127.0.0.1:port，把 OpenAI/Anthropic
//! 请求转发给组长。组长来源有两种：
//!
//!   1. **局域网**：UDP 发现（DiscoveryClient）
//!   2. **跨网络**：Deep Link 解析（coord-client resolve → 静态组长注入）
//!
//! 令牌换取（/auth/token）、鉴权与计量全部由组长完成；成员侧保持无状态透传。
//!
//! 链路加密（design 2026-08-26）：`link.encrypt = off|aes-gcm|tls`（缺省 off）。
//!
//!   - **静态组长（跨网络深链）且 encrypt != off** → 请求加密 +
//!     `x-aipg-enc: v1`，响应按头解密（密钥 = SHA-256(bearer_token)，与组长协商一致）；
//!   - `/auth/token`、`/auth/rename` 始终明文（换令牌先于密钥协商）；
//!   - LAN（无静态组长）或 encrypt=off → 明文透传（零开销快路径）。

use std::sync::{Arc, RwLock};

use aipg_link_crypto::{ENC_HEADER, ENC_VERSION, LinkCrypto, LinkEncryptMode};

use super::discovery::{DiscoveryClient, LeaderInfo};

/// 成员侧 gateway：组长发现 + 请求转发。
#[derive(Clone)]
pub struct MemberGateway {
    discovery: DiscoveryClient,
    /// 静态组长（Deep Link 解析注入，跨网络场景；优先于 UDP 发现）。
    static_leader: Arc<RwLock<Option<LeaderInfo>>>,
    http: reqwest::Client,
    /// 链路加密模式（跨网络深链时生效）。
    encrypt: LinkEncryptMode,
}

impl MemberGateway {
    pub fn new(discovery: DiscoveryClient) -> Self {
        Self::with_encrypt(discovery, LinkEncryptMode::Off)
    }

    /// 指定链路加密模式构造（config `link.encrypt`）。
    pub fn with_encrypt(discovery: DiscoveryClient, encrypt: LinkEncryptMode) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(120))
            .build()
            .unwrap_or_default();
        Self {
            discovery,
            static_leader: Arc::new(RwLock::new(None)),
            http,
            encrypt,
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

    /// 换令牌/改名端点始终明文（密钥协商前必须可达）。
    fn is_exempt(path: &str) -> bool {
        path == "/auth/token" || path == "/auth/rename"
    }

    /// 跨网络深链 + encrypt != off → 启用链路加密（LAN/off 走明文快路径）。
    fn link_encrypts(&self) -> bool {
        self.encrypt != LinkEncryptMode::Off && self.static_leader.read().unwrap().is_some()
    }

    /// 透传转发：路径 + 方法 + 可选 Bearer + 可选原始 body，返回组长 (状态码, 响应体)。
    /// 跨网络深链且 link.encrypt != off 时自动加密请求/解密响应（raw 二进制，高性能）。
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

        // 链路加密协商：跨网深链 + 非排除端点 + 已启用 + 持有 bearer token → 加密；
        // 无 token（如 /v1/models 的 None auth）回落明文，避免死锁
        let token = auth
            .and_then(|a| a.strip_prefix("Bearer "))
            .unwrap_or_default()
            .trim()
            .to_string();
        let encrypting = self.link_encrypts() && !Self::is_exempt(path) && !token.is_empty();
        let mut enc_key = [0u8; 32];
        if encrypting {
            // 密钥 = SHA-256(bearer_token)：与组长协商一致
            enc_key = LinkCrypto::derive_key(&token);
            req = req.header(ENC_HEADER, ENC_VERSION);
            if let Some(b) = body {
                let crypto = LinkCrypto::new();
                req = req.body(crypto.encrypt(&enc_key, &b));
            }
        } else if let Some(b) = body {
            req = req.body(b);
        }

        let resp = req.send().await
            .map_err(|e| format!("leader unreachable: {e}"))?;
        let status = resp.status().as_u16();
        let resp_enc = resp
            .headers()
            .get(ENC_HEADER)
            .and_then(|v| v.to_str().ok())
            .map(|v| v == ENC_VERSION)
            .unwrap_or(false);
        let bytes = resp.bytes().await.map_err(|e| format!("read response: {e}"))?.to_vec();
        tracing::debug!("member gateway -> {url} [{status}] enc_req={encrypting} enc_resp={resp_enc}");
        // 组长加密响应 → 同 key 解密；否则透传（兼容无加密组长）
        if resp_enc {
            let crypto = LinkCrypto::new();
            match crypto.decrypt(&enc_key, &bytes) {
                Ok(plain) => Ok((status, plain)),
                Err(e) => Err(format!("leader response decrypt failed: {e}")),
            }
        } else {
            Ok((status, bytes))
        }
    }
}