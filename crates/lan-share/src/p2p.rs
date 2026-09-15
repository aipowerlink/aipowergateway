//! 组长侧跨网 P2P 打洞会话（路径2 数据面）。
//!
//! 职责（只做打洞 + QUIC 承载接线，策略/认证/加密语义零改动）：
//! 1. 轮询信令信箱（GET /v1/signal）收成员候选；
//! 2. 对每个成员：新绑 UDP socket → 收集该 socket 候选（local + STUN 反射）→
//!    经信令回投「组长候选 + cert_fp」给该成员；
//! 3. 双方同时 punch → 成功后把该 socket 交给 quinn（组长 endpoint），
//!    每流桥接到组长本机 `127.0.0.1:{share_port}`（share_router）。
//!
//! 多个成员互不干扰：每个成员一个独立 socket/QUIC endpoint。
//! 打洞失败 → 只记录日志（成员侧会明确报「无法连接」，无中继兜底）。

use std::collections::HashSet;
use std::time::Duration;

use aipg_coord_client::SignalMessage;
use aipg_coord_client::DeviceClient;
use aipg_punch::{Candidate, CandidateKind, Error, PunchCert, PunchOptions, PunchSocket, leader_endpoint, run_leader_tunnel};

/// 组长打洞会话配置。
#[derive(Debug, Clone)]
pub struct LeaderPunchConfig {
    /// 打洞成功后 QUIC 流桥接目标（组长本机 share_router，如 127.0.0.1:39092）
    pub tunnel_target: std::net::SocketAddr,
    /// 证书持久化目录（存 punch-cert.der / punch-key.der，重启指纹稳定）
    pub cert_dir: std::path::PathBuf,
    /// STUN 回显端点（形如 "127.0.0.1:3478"）；None = 只收集本地候选
    pub stun_addr: Option<String>,
    /// 打洞总超时（默认 15s；含信令回投 + 成员拉起的时间窗）
    pub punch_timeout: Duration,
}

impl Default for LeaderPunchConfig {
    fn default() -> Self {
        Self {
            tunnel_target: ([127, 0, 0, 1], 39092).into(),
            cert_dir: std::path::PathBuf::new(),
            stun_addr: None,
            punch_timeout: Duration::from_secs(15),
        }
    }
}

/// 运行组长打洞会话（无限循环，由调用方 spawn）。
/// `client` 必须是已 register 的 DeviceClient（已持 share_id + device_token）。
pub async fn run_leader_punch(client: &DeviceClient, cfg: &LeaderPunchConfig) -> Result<(), Error> {
    let cert = PunchCert::load_or_generate(
        &cfg.cert_dir.join("punch-cert.der"),
        &cfg.cert_dir.join("punch-key.der"),
    )?;
    let mut tunnels: HashSet<String> = HashSet::new();
    tracing::info!(fingerprint = %cert.fingerprint, "leader punch session ready (fingerprint distributed via signal)");

    loop {
        // 每 1s 拉一次信令信箱（取走即删）
        let messages: Vec<SignalMessage> = match client.signal_pull().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(%e, "leader signal_pull failed (retry)");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };
        for msg in messages {
            if tunnels.contains(&msg.from_share_id) {
                continue; // 该成员隧道已建立
            }
            let Some(body) = msg.body.as_object() else { continue };
            if body.get("type").and_then(|t| t.as_str()) != Some("candidate") {
                continue;
            }
            let Some(cands) = body.get("candidates").and_then(|c| c.as_array()) else { continue };
            let member_cands: Vec<Candidate> = cands
                .into_iter()
                .filter_map(|c| {
                    let ip = c.get("ip")?.as_str()?;
                    let port = c.get("port")?.as_u64()? as u16;
                    let kind = match c.get("kind").and_then(|k| k.as_str()) {
                        Some("mapped") => CandidateKind::Mapped,
                        _ => CandidateKind::Local,
                    };
                    Some(Candidate::new(ip, port, kind))
                })
                .collect();
            if member_cands.is_empty() {
                tracing::warn!(from = %msg.from_share_id, "成员候选为空，忽略");
                continue;
            }

            // 本成员专用 socket + 候选（local + STUN 反射）
            let socket = match PunchSocket::bind() {
                Ok(s) => s,
                Err(e) => { tracing::warn!(%e, "绑定打洞 socket 失败"); continue; }
            };
            let mut leader_cands = socket.local_candidates();
            if let Some(stun) = &cfg.stun_addr {
                match socket.stun_mapped_candidate(stun, Duration::from_secs(3)).await {
                    Ok(c) => leader_cands.push(c),
                    Err(e) => tracing::warn!(%e, "STUN 反射失败（仅本地候选）"),
                }
            }

            let member_id = msg.from_share_id.clone();
            // 回投组长候选 + cert_fp → 成员据此 punch
            let reply = serde_json::json!({
                "type": "candidate",
                "candidates": leader_cands,
                "cert_fp": cert.fingerprint,
            });
            if let Err(e) = client.signal_push(&member_id, reply).await {
                tracing::warn!(%e, "回投候选失败");
                continue;
            }

            // 双方 punch（组长向成员候选探测；成员收到回投后同时探测）
            match socket.punch(&member_cands, &PunchOptions { timeout: cfg.punch_timeout, ..Default::default() }).await {
                Ok(peer) => {
                    tracing::info!(from = %member_id, %peer, "打洞成功");
                    // 同一 socket 承载 QUIC（组长 endpooint 监听，流桥到 share_router）
                    match socket.std_socket().and_then(|s| leader_endpoint(s, &cert)) {
                        Ok(endpoint) => {
                            let target = cfg.tunnel_target;
                            let _ = tokio::spawn(async move {
                                let _ = run_leader_tunnel(endpoint, target).await;
                            });
                            tunnels.insert(member_id.clone());
                            println!("p2p: member {member_id} connected (QUIC tunnel → {target})");
                        }
                        Err(e) => tracing::warn!(%e, "QUIC 组长 endpoint 创建失败"),
                    }
                }
                Err(Error::Timeout(_)) | Err(Error::PunchFailed(_)) => {
                    tracing::warn!(from = %member_id, "打洞失败（成员将收到明确错误，无中继兜底）");
                }
                Err(e) => tracing::warn!(%e, "打洞异常"),
            }
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}

/// 组长侧入口（供 cli 调用）：spawn 打洞会话并返回 JoinHandle。
pub fn spawn_leader_punch(client: DeviceClient, cfg: LeaderPunchConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = run_leader_punch(&client, &cfg).await {
            tracing::error!(%e, "leader punch session exited");
        }
    })
}