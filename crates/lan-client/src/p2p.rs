//! 成员侧跨网 P2P 打洞会话（路径2 数据面）。
//!
//! 职责（只做打洞 + QUIC 承载接线）：
//! 1. 绑 UDP socket → 收集候选（local + STUN 反射）；
//! 2. 经信令投递候选给组长 share_id；
//! 3. 轮询拉取组长回投的候选 + cert_fp；
//! 4. 双方 punch → 成功后同一 socket 承载 QUIC（成员端）并建立本地 TCP 桥；
//! 5. 返回本地桥端口 —— 上层把静态组长目标切到 127.0.0.1:{port}，
//!    现有 axum + AES-GCM 代理语义零改动。
//!
//! 打洞失败 → 返回明确错误（"无法连接"），无中继兜底。

use std::sync::Arc;
use std::time::Duration;

use aipg_coord_client::SignalMessage;
use aipg_coord_client::DeviceClient;
use aipg_punch::{
    Candidate, Error, PunchOptions, PunchSocket, connect, member_endpoint, spawn_member_tunnel,
};

/// 成员打洞会话结果：本地桥端口（QUIC 隧道入口）。
#[derive(Debug, Clone)]
pub struct MemberTunnel {
    /// 成员本机 TCP 监听端口（127.0.0.1:{port} 即组长共享通道入口）
    pub local_port: u16,
}

/// 运行一次成员打洞会话（一次性，成功返回本地桥端口；失败返回明确错误）。
/// 内部时序：投候选 → 拉组长候选+cert_fp（最长 `signal_timeout`）→ punch → QUIC → 本地桥。
pub async fn punch_join(
    client: &DeviceClient,
    leader_share_id: &str,
    stun_addr: Option<String>,
) -> Result<MemberTunnel, Error> {
    let socket = PunchSocket::bind()?;
    let mut my_cands = socket.local_candidates();
    if let Some(stun) = &stun_addr {
        match socket.stun_mapped_candidate(stun, Duration::from_secs(3)).await {
            Ok(c) => my_cands.push(c),
            Err(e) => tracing::warn!(%e, "STUN 反射失败（仅本地候选）"),
        }
    }

    // 1. 把自己的候选投给组长
    let body = serde_json::json!({
        "type": "candidate",
        "candidates": my_cands,
    });
    client
        .signal_push(leader_share_id, body)
        .await
        .map_err(|e| Error::Quic(format!("投递候选失败: {e}")))?;
    tracing::info!(leader = %leader_share_id, "成员候选已投递（等待组长回投候选 + cert_fp）");

    // 2. 轮询拉组长候选 + cert_fp（信令信箱取走即删；TTL 60s）
    let signal_timeout = Duration::from_secs(15);
    let deadline = tokio::time::Instant::now() + signal_timeout;
    let mut leader_msg: Option<SignalMessage> = None;
    while tokio::time::Instant::now() < deadline {
        match client.signal_pull().await {
            Ok(msgs) => {
                for m in msgs {
                    let is_candidate = m
                        .body
                        .get("type")
                        .and_then(|t| t.as_str())
                        .map(|t| t == "candidate")
                        .unwrap_or(false);
                    if is_candidate {
                        leader_msg = Some(m);
                        break;
                    }
                }
                if leader_msg.is_some() {
                    break;
                }
            }
            Err(e) => tracing::warn!(%e, "signal_pull failed (retry)"),
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let msg = leader_msg
        .ok_or_else(|| Error::Timeout("等待组长候选超时（打洞失败：无法连接）".into()))?;
    let body = msg
        .body
        .as_object()
        .ok_or_else(|| Error::Quic("组长信令格式无效".into()))?;
    let cert_fp = body
        .get("cert_fp")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Quic("组长信令缺 cert_fp".into()))?
        .to_string();
    let cands = body
        .get("candidates")
        .and_then(|c| c.as_array())
        .ok_or_else(|| Error::Quic("组长信令缺 candidates".into()))?
        .iter()
        .filter_map(|c| {
            let ip = c.get("ip")?.as_str()?;
            let port = c.get("port")?.as_u64()? as u16;
            let kind = match c.get("kind").and_then(|k| k.as_str()) {
                Some("mapped") => aipg_punch::CandidateKind::Mapped,
                _ => aipg_punch::CandidateKind::Local,
            };
            Some(Candidate::new(ip, port, kind))
        })
        .collect::<Vec<_>>();
    tracing::info!(cert_fp = %cert_fp, "已收到组长候选 + 指纹");

    // 3. 双方 punch（组长侧同时向我的候选探测）
    let peer_addr = socket
        .punch(&cands, &PunchOptions { timeout: Duration::from_secs(10), ..Default::default() })
        .await?;
    tracing::info!(%peer_addr, "打洞成功（组长映射端点）");

    // 4. 同一 socket 承载 QUIC（成员端，锁定组长指纹）+ 本地 TCP 桥
    let endpoint = member_endpoint(socket.std_socket()?, &cert_fp)?;
    let conn = connect(&endpoint, peer_addr, "localhost", Duration::from_secs(10)).await?;
    let conn = Arc::new(conn);
    let bind: std::net::SocketAddr = ([127, 0, 0, 1], 0).into();
    let local = spawn_member_tunnel(conn, bind).await?;
    tracing::info!(port = local.port(), "成员隧道就绪：经 127.0.0.1:{} 进入组长共享通道", local.port());
    // endpoint 被 spawn_member_tunnel 的 QUIC 连接持有期间不得 drop（连接生命周期绑定 Arc<Connection>）；
    // 这里显式丢弃 endpoint 即可：Connection 的 Arc 引用使底层继续存活。
    Ok(MemberTunnel { local_port: local.port() })
}