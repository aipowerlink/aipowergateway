//! 隧道桥接原语：把 quinn 双向流与 TCP 连接双向拷贝，以及组长/成员两侧的隧道循环。
//!
//! 数据面设计（路径2 QUIC 隧道模式）：
//!   - **组长**：打洞成功后，把每个 QUIC 连接的每条双向流通向组长本机
//!     `127.0.0.1:{share_port}`（share_router，已达 axum）。
//!   - **成员**：打洞成功后建立 QUIC 连接，在本机起一个 TCP 监听端口，
//!     每个 TCP 连接开一条 QUIC 双向流；应用层把代理目标切到该本地端口，
//!     现有 axum + AES-GCM 语义零改动。
//!
//! 桥接用 `tokio::io::copy_bidirectional`：需要双端都实现 AsyncRead+AsyncWrite。
//! quinn 的 `RecvStream` 只实现 AsyncRead、`SendStream` 只实现 AsyncWrite，
//! 因此这里先组合成 `QuicBiStream`（send/recv 合一），再与 TcpStream 双向拷贝。

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

/// 组合 SendStream + RecvStream，向 tokio 呈现一个双向 IO 对象。
pub struct QuicBiStream {
    send: quinn::SendStream,
    recv: quinn::RecvStream,
}

impl QuicBiStream {
    pub fn new(send: quinn::SendStream, recv: quinn::RecvStream) -> Self {
        Self { send, recv }
    }

    /// 终止对端写方向（关闭 send 半流）。
    pub fn finish(&mut self) -> Result<(), quinn::ClosedStream> {
        self.send.finish()
    }
}

impl AsyncRead for QuicBiStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        tokio::io::AsyncRead::poll_read(Pin::new(&mut self.get_mut().recv), cx, buf)
    }
}

impl AsyncWrite for QuicBiStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(Pin::new(&mut self.get_mut().send), cx, buf)
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(Pin::new(&mut self.get_mut().send), cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        tokio::io::AsyncWrite::poll_shutdown(Pin::new(&mut self.get_mut().send), cx)
    }
}

/// 一条 QUIC 双向流 ↔ 一个 TCP 连接的双向拷贝（拷贝完毕即返回）。
pub async fn bridge_bi_to_tcp(
    send: quinn::SendStream,
    recv: quinn::RecvStream,
    tcp: tokio::net::TcpStream,
) -> io::Result<()> {
    let mut bi = QuicBiStream::new(send, recv);
    let mut tcp = tcp;
    tokio::io::copy_bidirectional(&mut tcp, &mut bi).await?;
    Ok(())
}

/// 组长侧隧道循环：接受 QUIC 连接，每个连接循环 accept_bi，
/// 每条流 spawn 桥接到 `target`（组长本机 127.0.0.1:{share_port}）。
/// endpoint 被 accept 前是移动进循环；连接/流错误只记日志并继续。
pub async fn run_leader_tunnel(
    endpoint: quinn::Endpoint,
    target: SocketAddr,
) -> Result<(), crate::error::Error> {
    loop {
        let incoming = match endpoint.accept().await {
            Some(i) => i,
            None => break, // endpoint 关闭
        };
        tracing::info!(?target, "punch tunnel: leader accepted QUIC connection");
        tokio::spawn(leader_conn_loop(incoming, target));
    }
    Ok(())
}

async fn leader_conn_loop(incoming: quinn::Incoming, target: SocketAddr) {
    let conn = match incoming.await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(%e, "punch tunnel: handshake failed");
            return;
        }
    };
    tracing::debug!("punch tunnel: leader conn established");
    loop {
        let (send, recv) = match conn.accept_bi().await {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(%e, "punch tunnel: accept_bi ended");
                break;
            }
        };
        let target = target;
        tokio::spawn(async move {
            match tokio::net::TcpStream::connect(target).await {
                Ok(tcp) => {
                    if let Err(e) = bridge_bi_to_tcp(send, recv, tcp).await {
                        tracing::debug!(%e, "punch tunnel: bridge closed");
                    }
                }
                Err(e) => {
                    tracing::warn!(%e, %target, "punch tunnel: leader bridge target unreachable");
                }
            }
        });
    }
    let _ = conn;
}

/// 成员侧隧道服务：持有已建立的 QUIC 连接（Arc 共享），
/// 在本机 `bind` 起 TCP 监听；每个 TCP 连接 open_bi 一条流并双向桥接。
/// 绑定完成后立即返回实际监听地址（绑定端口可能是 0 = 随机），
/// 监听与桥接循环在后台任务中持续运行。
pub async fn spawn_member_tunnel(
    conn: Arc<quinn::Connection>,
    bind: SocketAddr,
) -> Result<SocketAddr, crate::error::Error> {
    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|e| crate::error::Error::Io(e))?;
    let result = listener.local_addr().map_err(|e| crate::error::Error::Io(e));
    tracing::info!(?result, "punch tunnel: member local bridge binding");
    tokio::spawn(member_tunnel_loop(listener, conn));
    result
}

async fn member_tunnel_loop(
    listener: tokio::net::TcpListener,
    conn: Arc<quinn::Connection>,
) {
    tracing::info!("punch tunnel: member local bridge listening (background)");
    loop {
        let (tcp, peer) = match listener.accept().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(%e, "punch tunnel: accept failed");
                continue;
            }
        };
        let conn = conn.clone();
        tokio::spawn(async move {
            match conn.open_bi().await {
                Ok((send, recv)) => {
                    if let Err(e) = bridge_bi_to_tcp(send, recv, tcp).await {
                        tracing::debug!(%e, %peer, "punch tunnel: member bridge closed");
                    }
                }
                Err(e) => {
                    tracing::warn!(%e, "punch tunnel: open_bi failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// 打洞 → QUIC → 双向流隧道：成员侧本地 TCP → 组长侧本地 TCP echo 服务端，完整往返。
    /// 验证 QuicBiStream + 双向拷贝在真实数据面可用（成员发字节经隧道到组长侧 echo 服务器返回）。
    #[tokio::test]
    async fn tunnel_roundtrip_through_quic() {
        use crate::quic::{connect, leader_endpoint, member_endpoint, PunchCert};
        use crate::punch::{PunchOptions, PunchSocket};

        let cert = PunchCert::generate().expect("cert");

        // 组长侧 echo TCP 服务端（模拟组长本机 127.0.0.1:share_port）
        let echo = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("echo bind");
        let echo_addr = echo.local_addr().expect("echo addr");
        tokio::spawn(async move {
            loop {
                let (mut tcp, _) = echo.accept().await.expect("echo accept");
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    loop {
                        match tcp.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => {
                                if tcp.write_all(&buf[..n]).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                });
            }
        });

        // 双端打洞
        let leader = PunchSocket::bind().expect("leader socket");
        let member = PunchSocket::bind().expect("member socket");
        let opts = PunchOptions { timeout: Duration::from_secs(5), ..Default::default() };
        let leader_cand = crate::Candidate::new("127.0.0.1", leader.local_addr().port(), crate::CandidateKind::Local);
        let member_cand = crate::Candidate::new("127.0.0.1", member.local_addr().port(), crate::CandidateKind::Local);
        let leader_targets = vec![member_cand];
        let member_targets = vec![leader_cand];
        let (po_l, po_m) = tokio::join!(
            leader.punch(&leader_targets, &opts),
            member.punch(&member_targets, &opts),
        );
        let peer_of_leader = po_l.expect("leader punch");
        let peer_of_member = po_m.expect("member punch");
        let _ = peer_of_leader;
        let leader_addr = peer_of_member;

        // QUIC 端点
        let leader_ep = leader_endpoint(leader.std_socket().expect("leader std"), &cert).expect("leader ep");
        let member_ep = member_endpoint(member.std_socket().expect("member std"), &cert.fingerprint).expect("member ep");

        // 组长隧道：把 QUIC 流桥到 echo_addr（模拟 share_port）
        let leader_ep = Arc::new(leader_ep);
        let tunnel_ep = leader_ep.clone();
        let leader_tunnel = tokio::spawn(async move {
            let _ = run_leader_tunnel((*tunnel_ep).clone(), echo_addr).await;
        });

        // 成员 QUIC 连接 + 本地桥（spawn 后台循环，绑定即返回本地端口）
        let conn = connect(&member_ep, leader_addr, "localhost", Duration::from_secs(5))
            .await
            .expect("member connect");
        let conn = Arc::new(conn);
        let member_bind: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let member_local = spawn_member_tunnel(conn.clone(), member_bind)
            .await
            .expect("member tunnel start");

        // 成员侧本地 TCP 客户端经隧道发数据 → 组长侧 echo 返回
        let mut client = tokio::net::TcpStream::connect(member_local).await.expect("client connect");
        client.write_all(b"tunnel-payload").await.expect("client write");
        let mut buf = [0u8; 64];
        let mut got = Vec::new();
        // echo 收满后关闭；这里读若干次直到拿到全部
        loop {
            let n = client.read(&mut buf).await.expect("client read");
            if n == 0 {
                break;
            }
            got.extend_from_slice(&buf[..n]);
            // 组长 echo 侧在读完一段后即回显，数据量小，一次通常足够；
            // 若不足则继续读（对端尚未关闭）。
            if got.len() >= b"tunnel-payload".len() {
                break;
            }
        }
        assert_eq!(&got, b"tunnel-payload");

        // 成员显式关闭 QUIC 连接，结束隧道任务
        conn.close(quinn::VarInt::from_u32(0), b"done");
        leader_ep.close(quinn::VarInt::from_u32(0), b"done");
        let _ = leader_tunnel;
    }
}