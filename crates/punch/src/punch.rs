//! UDP 打洞核心：绑定 socket → 收集候选 → 双方同时互发探测包 → 双向可达确认。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rand::Rng;

use crate::candidate::{Candidate, CandidateKind, local_candidates};
use crate::error::{Error, Result};

/// 探测帧魔数（AIPG + 帧类型）。
const FRAME_MAGIC: &[u8; 4] = b"AIPG";
/// 探测请求帧类型。
const FRAME_PROBE: u8 = 1;
/// 帧头长度 = 魔数 4 + 类型 1 + 随机 tag 8 = 13。
const FRAME_HEADER_LEN: usize = 13;

/// 打洞选项。
#[derive(Debug, Clone)]
pub struct PunchOptions {
    /// 打洞总超时（默认 10s）
    pub timeout: Duration,
    /// 探测包发送间隔（默认 100ms）
    pub probe_interval: Duration,
    /// 每轮向每个候选发送的探测次数（默认 1；间隔内重复发提高穿越成功率）
    pub probes_per_round: u32,
}

impl Default for PunchOptions {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(10),
            probe_interval: Duration::from_millis(100),
            probes_per_round: 1,
        }
    }
}

/// 打洞 socket：绑定的 UDP + 本地候选 + 观测到的对端映射地址。
///
/// 内部持 std::net::UdpSocket；每次 async 操作经 `try_clone` 建立共享同一
/// fd 的 tokio 句柄（NAT 映射保持在原 fd 上）。打洞完成后把克隆交给 quinn
/// 承载 QUIC（同一 UDP 端口，映射不失效）。
#[derive(Clone)]
pub struct PunchSocket {
    socket: Arc<std::net::UdpSocket>,
    local: SocketAddr,
    /// 打洞过程中观测到的对端实际源地址（NAT 映射后的公网端点）
    peer_seen: Arc<std::sync::Mutex<Option<SocketAddr>>>,
}

impl PunchSocket {
    /// 绑定 UDP socket（随机端口）。
    pub fn bind() -> Result<Self> {
        Self::bind_with_port(0)
    }

    /// 绑定固定端口（打洞端口）。与 `bind(0)` 的区别：显式端口便于 NAT 预测。
    pub fn bind_with_port(port: u16) -> Result<Self> {
        let socket = std::net::UdpSocket::bind(("0.0.0.0", port))
            .map_err(|e| Error::Io(e))?;
        let local = socket.local_addr()?;
        let _ = socket.set_broadcast(true);
        let _ = socket.set_nonblocking(true);
        Ok(Self {
            socket: Arc::new(socket),
            local,
            peer_seen: Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// 绑定后的本地地址（ip:port）。
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// 当前 socket 的 tokio 句柄（与内部 fd 共享）。
    fn tokio_handle(&self) -> Result<tokio::net::UdpSocket> {
        let std_clone = self.socket.try_clone().map_err(|e| Error::Io(e))?;
        let handle = tokio::net::UdpSocket::from_std(std_clone).map_err(|e| Error::Io(e))?;
        Ok(handle)
    }

    /// 打洞候选（本地地址）。
    pub fn local_candidates(&self) -> Vec<Candidate> {
        local_candidates(self.local.port())
    }

    /// STUN 反射候选：向服务器回显端点发 UDP 包（非 connect，避免污染 socket
    /// 的已连接状态，打洞后续可继续 send_to/recv_from 任意对端）。
    pub async fn stun_mapped_candidate(&self, stun_addr: &str, timeout: Duration) -> Result<Candidate> {
        let server: SocketAddr = stun_addr.parse().map_err(|_| {
            Error::PunchFailed(format!("STUN 地址无效: {stun_addr}"))
        })?;
        let sock = self.tokio_handle()?;
        sock.send_to(b"AIPG1", server).await?;
        let mut buf = [0u8; 512];
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep(remaining) => break,
                r = sock.recv_from(&mut buf) => {
                    let (n, _src) = r?;
                    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&buf[..n]) {
                        if v.get("type").and_then(|t| t.as_str()) == Some("mapped") {
                            let ip = v.get("ip").and_then(|i| i.as_str())
                                .ok_or_else(|| Error::PunchFailed("STUN 响应缺 ip".into()))?;
                            let port = v.get("port").and_then(|p| p.as_u64())
                                .ok_or_else(|| Error::PunchFailed("STUN 响应缺 port".into()))?;
                            return Ok(Candidate::new(ip, port as u16, CandidateKind::Mapped));
                        }
                    }
                }
            }
        }
        Err(Error::Timeout(format!("STUN 回显超时: {stun_addr}")))
    }

    /// 打洞：同时向对端全部候选互发探测，等待对方回显/探测包。
    /// 成功返回对端实际映射地址（NAT 打洞后可用于 QUIC 直连）。
    ///
    /// 内部单任务 select 循环：一边发探测帧一边收帧；收到对端 AIPG 帧 →
    /// 回显给对方（帮助对端也确认双向）+ 记录源地址 → 打通。
    pub async fn punch(
        &self,
        peer_candidates: &[Candidate],
        opts: &PunchOptions,
    ) -> Result<SocketAddr> {
        let sock = self.tokio_handle()?;
        let peer_seen = self.peer_seen.clone();
        let deadline = tokio::time::Instant::now() + opts.timeout;

        let mut ticker = tokio::time::interval(opts.probe_interval.max(Duration::from_millis(10)));
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut buf = [0u8; 2048];
        let mut sent_any = false;

        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                break;
            }
            tokio::select! {
                _ = tokio::time::sleep_until(deadline) => break,
                _ = ticker.tick() => {
                    // 每 tick 向对端候选发探测帧
                    let mut frame = [0u8; FRAME_HEADER_LEN];
                    frame[..4].copy_from_slice(FRAME_MAGIC);
                    frame[4] = FRAME_PROBE;
                    rand::rng().fill(&mut frame[5..FRAME_HEADER_LEN]);
                    for _p in 0..opts.probes_per_round {
                        for c in peer_candidates {
                            let dst: SocketAddr = c.to_socket_addr();
                            if sock.send_to(&frame, dst).await.is_ok() {
                                sent_any = true;
                            }
                            if tokio::time::Instant::now() >= deadline {
                                break;
                            }
                        }
                    }
                }
                r = sock.recv_from(&mut buf) => {
                    match r {
                        Ok((n, src)) => {
                            let is_aipg = n >= FRAME_HEADER_LEN && &buf[..4] == FRAME_MAGIC;
                            // 任何源（只要不是未指定地址）都可能是对端经 NAT 映射后的端点；
                            // AIPG 帧优先确认。回显供对端确认双向。
                            if is_aipg {
                                let _ = sock.send_to(&buf[..n], src).await;
                            }
                            *peer_seen.lock().unwrap() = Some(src);
                            return Ok(src);
                        }
                        Err(_) => break,
                    }
                }
            }
        }
        if !sent_any {
            return Err(Error::PunchFailed("无法发送任何探测包（候选不可达）".into()));
        }
        Err(Error::Timeout(format!(
            "在 {}s 内未与对端候选 {peer_candidates:?} 打通（跨网直连失败）",
            opts.timeout.as_secs()
        )))
    }

    /// 打洞成功后，把 socket 交给 quinn（克隆出 std 句柄共享同一 fd）。
    /// 调用方持 std::net::UdpSocket 构造 quinn Endpoint；本对象可继续持有或丢弃。
    pub fn std_socket(&self) -> Result<std::net::UdpSocket> {
        Ok(self.socket.try_clone().map_err(|e| Error::Io(e))?)
    }

    /// 打洞过程中观测到的对端实际源地址（打通后可用于 QUIC 直连目标）。
    pub fn peer_seen(&self) -> Option<SocketAddr> {
        self.peer_seen.lock().unwrap().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 回环双端打洞：两个 PunchSocket 互以对端为候选，应能在超时内打通。
    #[tokio::test]
    async fn loopback_punch_connects() {
        let a = PunchSocket::bind().expect("bind a");
        let b = PunchSocket::bind().expect("bind b");

        let opts = PunchOptions {
            timeout: Duration::from_secs(5),
            ..Default::default()
        };
        // 双向同时打洞（真实场景双方均主动发探测）
        let b_cand = Candidate::new("127.0.0.1", b.local_addr().port(), CandidateKind::Local);
        let a_cand = Candidate::new("127.0.0.1", a.local_addr().port(), CandidateKind::Local);
        let a_targets = vec![b_cand];
        let b_targets = vec![a_cand];
        let (ra, rb) = tokio::join!(
            a.punch(&a_targets, &opts),
            b.punch(&b_targets, &opts),
        );
        let peer_a = ra.expect("a 打洞失败");
        let peer_b = rb.expect("b 打洞失败");
        // a 观测到的对端应就是 b 的端口；b 观测到的应就是 a 的端口
        assert_eq!(peer_a.port(), b.local_addr().port(), "a 看到 b");
        assert_eq!(peer_b.port(), a.local_addr().port(), "b 看到 a");
    }

    /// 打洞确认后 socket 仍可在同端口转发（NAT 映射已建立）。
    #[tokio::test]
    async fn socket_reusable_after_punch() {
        let a = PunchSocket::bind().expect("bind a");
        let b = PunchSocket::bind().expect("bind b");
        let opts = PunchOptions { timeout: Duration::from_secs(5), ..Default::default() };
        let b_cand = Candidate::new("127.0.0.1", b.local_addr().port(), CandidateKind::Local);
        let a_cand = Candidate::new("127.0.0.1", a.local_addr().port(), CandidateKind::Local);
        let a_targets = vec![b_cand];
        let b_targets = vec![a_cand];
        let (ra, _) = tokio::join!(
            a.punch(&a_targets, &opts),
            b.punch(&b_targets, &opts),
        );
        ra.expect("punch failed");
        // 打洞后直接互发普通数据应可达。目标必须是可路由地址：
        // 绑定是 0.0.0.0（通配），但发送目标用 127.0.0.1 而非 0.0.0.0。
        let b_addr: SocketAddr = (std::net::Ipv4Addr::LOCALHOST, b.local_addr().port()).into();
        let a_sock = a.tokio_handle().expect("handle a");
        a_sock.send_to(b"hello-after-punch", b_addr).await.expect("send");
        let mut buf = [0u8; 64];
        let b_sock = b.tokio_handle().expect("handle b");
        // socket 缓冲区可能残留打洞探测帧（AIPG 魔数开头），跳过它们直到业务数据
        let (n, _) = loop {
            let (n, src) = b_sock.recv_from(&mut buf).await.expect("recv");
            if &buf[..4] != super::FRAME_MAGIC {
                break (n, src);
            }
        };
        assert_eq!(&buf[..n], b"hello-after-punch");
    }
}