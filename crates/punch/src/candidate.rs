//! 打洞候选：本地绑定地址 + STUN 反射的公网映射地址。

use serde::{Deserialize, Serialize};

/// 候选来源。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CandidateKind {
    /// 本地绑定地址（socket 视角）
    Local,
    /// STUN 反射的公网映射地址（NAT 后经回显端点获得）
    Mapped,
}

/// 一个打洞候选（ip:port）。两候选交换后，双方同时向对端候选发包打洞。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    pub ip: String,
    pub port: u16,
    pub kind: CandidateKind,
}

impl Candidate {
    pub fn new(ip: impl Into<String>, port: u16, kind: CandidateKind) -> Self {
        Self { ip: ip.into(), port, kind }
    }

    /// 用作 UDP 目标地址（std::net 兼容字符串）。
    pub fn to_socket_addr(&self) -> std::net::SocketAddr {
        format!("{}:{}", self.ip, self.port)
            .parse()
            .unwrap_or_else(|_| std::net::SocketAddr::from(([0, 0, 0, 0], self.port)))
    }
}

/// 收集本地网络接口的候选地址（排除回环/链路本地，优先 IPv4）。
/// 绑定端口为 `port`（0 = 随机）。返回空列表兜底回环（回环场景自测/同机直连）。
pub fn local_candidates(port: u16) -> Vec<Candidate> {
    let mut out = Vec::new();

    // 确定本机出站 IP：UDP "connect" 到一个通用地址（不实际发包），
    // 内核会选出与目标同路由的本地接口 IP。
    for probe in ["1.1.1.1:53", "8.8.8.8:53", "223.5.5.5:53"] {
        if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
            if sock.connect(probe).is_ok() {
                if let Ok(addr) = sock.local_addr() {
                    let ip = addr.ip();
                    if !ip.is_loopback() && !ip.is_unspecified() {
                        out.push(Candidate::new(ip.to_string(), port, CandidateKind::Local));
                        break;
                    }
                }
            }
        }
    }
    // 兜底：回环（同一台机器直连/本地测试）
    out.push(Candidate::new("127.0.0.1", port, CandidateKind::Local));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidate_serializes_for_signal() {
        let c = Candidate::new("203.0.113.7", 39093, CandidateKind::Mapped);
        let s = serde_json::to_string(&c).expect("serialize");
        assert_eq!(s, r#"{"ip":"203.0.113.7","port":39093,"kind":"mapped"}"#);
        let back: Candidate = serde_json::from_str(&s).expect("deserialize");
        assert_eq!(back, c);
    }

    #[test]
    fn local_candidates_always_has_something() {
        let cs = local_candidates(39093);
        assert!(!cs.is_empty(), "should have at least loopback");
        assert!(cs.iter().any(|c| c.port == 39093 && c.kind == CandidateKind::Local));
    }
}