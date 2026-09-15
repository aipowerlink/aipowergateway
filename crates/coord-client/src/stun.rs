//! UDP 打洞候选回显客户端（PS2 —— 配合服务器 `{stun_addr}` 端点）。
//!
//! 客户端向服务器的 UDP 回显端点发一个包，服务器回显其 NAT 映射后的公网
//! 源地址（ip:port），该地址作为打洞 "mapped" 候选。纯反射、无状态、不落盘。

use std::net::SocketAddr;

use crate::error::{Error, Result};

/// 向 `server`（形如 "relay.aipowerlink.net:3478"）发送 UDP 探测包，
/// 返回服务器视角看到的本端公网映射地址（NAT 后的 ip:port）。
///
/// 本地 socket 由系统分配临时端口；`bind` 可显式指定本地端口
/// （打洞场景需复用同一端口保持 NAT 映射，传 `None` 则自动分配）。
pub async fn stun_mapped_addr(
    server: &str,
    bind: Option<SocketAddr>,
) -> Result<(String, u16)> {
    let server: SocketAddr = server
        .parse()
        .map_err(|_| Error::MissingField("stun server addr".into()))?;
    let socket = match bind {
        Some(addr) => tokio::net::UdpSocket::bind(addr).await,
        None => {
            let any = if server.is_ipv4() {
                "0.0.0.0:0"
            } else {
                "[::]:0"
            };
            tokio::net::UdpSocket::bind(any).await
        }
    }
    .map_err(|e| Error::Unreachable(format!("bind udp: {e}")))?;

    socket
        .connect(server)
        .await
        .map_err(|e| Error::Unreachable(format!("connect stun: {e}")))?;
    socket
        .send(b"AIPG1")
        .await
        .map_err(|e| Error::Unreachable(format!("send stun probe: {e}")))?;

    let mut buf = [0u8; 512];
    let n = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        socket.recv(&mut buf),
    )
    .await
    .map_err(|_| Error::Unreachable("stun echo timeout".into()))?
    .map_err(|e| Error::Unreachable(format!("recv stun echo: {e}")))?;

    #[derive(serde::Deserialize)]
    struct Echo {
        #[serde(rename = "type")]
        kind: String,
        ip: String,
        port: u16,
    }
    let echo: Echo = serde_json::from_slice(&buf[..n])
        .map_err(|_| Error::MissingField("stun echo".into()))?;
    if echo.kind != "mapped" {
        return Err(Error::MissingField("stun echo type".into()));
    }
    Ok((echo.ip, echo.port))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket as StdUdp;

    /// 本地回显服务器（模拟服务器 {stun_addr} 端点：回显源地址）。
    fn spawn_echo_server() -> SocketAddr {
        let sock = StdUdp::bind("127.0.0.1:0").expect("bind echo");
        let addr = sock.local_addr().expect("local addr");
        std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((_n, src)) => {
                        let resp = format!(
                            r#"{{"type":"mapped","ip":"{}","port":{}}}"#,
                            src.ip(),
                            src.port()
                        );
                        let _ = sock.send_to(resp.as_bytes(), src);
                    }
                    Err(_) => break,
                }
            }
        });
        addr
    }

    #[tokio::test]
    async fn stun_echo_returns_mapped_addr() {
        let server = spawn_echo_server();
        let (ip, port) = stun_mapped_addr(&server.to_string(), None)
            .await
            .expect("stun");
        assert_eq!(ip, "127.0.0.1");
        assert!(port > 0, "mapped port should be non-zero");
    }
}