//! QUIC 承载：打洞成功后在同一 UDP socket 上承载 QUIC。
//!
//! 安全模型（零知识契约 §5）：
//! - 组长每次启动生成自签证书（rcgen），证书指纹（sha256 DER）经信令 `/v1/signal`
//!   随候选一起交给成员（候选 `cert_fp` 字段）；
//! - 成员用 **唯一信任该指纹** 的 rustls 配置连接：端到端锁定组长公钥，抗中间人；
//! - server 不参与数据面，只中转候选/指纹（零留存信箱，已于信令端实现）。
//!
//! QUIC 提供传输层（多路复用/可靠/拥塞），应用层 AES-GCM 链路加密原样保留（可叠加）。

use std::sync::Arc;
use std::time::Duration;

use crate::error::{Error, Result};

/// 打洞证书：自签证书 + 指纹（sha256 of DER，hex 小写）。
#[derive(Debug, Clone)]
pub struct PunchCert {
    pub der: Vec<u8>,
    pub key_der: Vec<u8>,
    /// 证书指纹（hex 小写，无冒号）。经信令交给成员校验。
    pub fingerprint: String,
}

impl PunchCert {
    /// 生成自签证书。SAN=localhost（QUIC SNI 不必匹配 IP，打洞目标地址任意）。
    pub fn generate() -> Result<Self> {
        let certified = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .map_err(|e| Error::Cert(e.to_string()))?;
        let der = certified.cert.der().to_vec();
        let key_der = certified.signing_key.serialize_der();
        let fingerprint = Self::fingerprint_of_der(&der);
        Ok(Self { der, key_der, fingerprint })
    }

    /// 从持久化文件加载（生成时落盘 data_dir/punch-cert.der + punch-key.der）。
    /// 持久化保证重启后指纹稳定，成员无需重新同步。
    pub fn load_or_generate(cert_path: &std::path::Path, key_path: &std::path::Path) -> Result<Self> {
        if let (Ok(der), Ok(key_der)) =
            (std::fs::read(cert_path), std::fs::read(key_path))
        {
            let fingerprint = Self::fingerprint_of_der(&der);
            if !der.is_empty() && !key_der.is_empty() {
                return Ok(Self { der, key_der, fingerprint });
            }
            tracing::warn!(?cert_path, "证书文件为空，重新生成");
        }
        let cert = Self::generate()?;
        if let Some(dir) = cert_path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(cert_path, &cert.der);
        let _ = std::fs::write(key_path, &cert.key_der);
        Ok(cert)
    }

    /// 计算 DER 指纹（与成员侧校验使用同一算法）。
    pub fn fingerprint_of_der(der: &[u8]) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(der);
        h.finalize().iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// 组长侧 QUIC endpoint：监听打洞后的 socket，接受成员连接。
pub fn leader_endpoint(
    socket: std::net::UdpSocket,
    cert: &PunchCert,
) -> Result<quinn::Endpoint> {
    let cert_der = rustls::pki_types::CertificateDer::from(cert.der.clone());
    let key_der = rustls::pki_types::PrivateKeyDer::from(rustls::pki_types::PrivatePkcs8KeyDer::from(
        cert.key_der.clone(),
    ));

    let tls_config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| Error::Cert(format!("tls server: {e}")))?;
    let quic_server = quinn::crypto::rustls::QuicServerConfig::try_from(tls_config)
        .map_err(|e| Error::Cert(format!("quic server cfg: {e}")))?;
    let server_config = quinn::ServerConfig::with_crypto(Arc::new(quic_server));
    let endpoint_config = quinn::EndpointConfig::default();

    let endpoint = quinn::Endpoint::new(
        endpoint_config,
        Some(server_config),
        socket,
        Arc::new(quinn::TokioRuntime),
    )
    .map_err(|e| Error::Quic(format!("endpoint bind: {e}")))?;
    Ok(endpoint)
}

/// 成员侧 QUIC endpoint：信任指纹（组长公钥锁定），打洞后 connect 组长映射地址。
pub fn member_endpoint(
    socket: std::net::UdpSocket,
    expected_fingerprint: &str,
) -> Result<quinn::Endpoint> {
    let verifier = FingerprintVerifier::new(expected_fingerprint.to_string());
    let tls_config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verifier))
        .with_no_client_auth();
    let quic_client = quinn::crypto::rustls::QuicClientConfig::try_from(tls_config)
        .map_err(|e| Error::Cert(format!("quic client cfg: {e}")))?;
    let client_config = quinn::ClientConfig::new(Arc::new(quic_client));

    let endpoint_config = quinn::EndpointConfig::default();
    let mut endpoint = quinn::Endpoint::new(endpoint_config, None, socket, Arc::new(quinn::TokioRuntime))
        .map_err(|e| Error::Quic(format!("endpoint bind: {e}")))?;
    endpoint.set_default_client_config(client_config);
    Ok(endpoint)
}

/// rustls custom verifier：仅接受 DER 指纹 == 信令收到的期望指纹。
#[derive(Debug)]
struct FingerprintVerifier {
    expected: String,
}

impl FingerprintVerifier {
    fn new(expected: String) -> Self {
        Self { expected }
    }
}

impl rustls::client::danger::ServerCertVerifier for FingerprintVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &rustls::pki_types::CertificateDer<'_>,
        _intermediates: &[rustls::pki_types::CertificateDer<'_>],
        _server_name: &rustls::pki_types::ServerName<'_>,
        _ocsp_response: &[u8],
        _now: rustls::pki_types::UnixTime,
    ) -> std::result::Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp = crate::quic::PunchCert::fingerprint_of_der(end_entity.as_ref());
        if fp == self.expected {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!(
                "证书指纹不匹配（期望 {}，实际 {fp}），拒绝连接",
                self.expected
            )))
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::CryptoProvider::get_default()
                .ok_or(rustls::Error::General("no crypto provider".into()))?
                .signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &rustls::pki_types::CertificateDer<'_>,
        dss: &rustls::DigitallySignedStruct,
    ) -> std::result::Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::CryptoProvider::get_default()
                .ok_or(rustls::Error::General("no crypto provider".into()))?
                .signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        rustls::crypto::CryptoProvider::get_default()
            .map(|p| p.signature_verification_algorithms.supported_schemes().to_vec())
            .unwrap_or_default()
    }
}

/// 用 quinn 建立连接（打洞成功后调用）。`server_name` 传 `localhost`（证书 SAN）。
pub async fn connect(
    endpoint: &quinn::Endpoint,
    addr: std::net::SocketAddr,
    server_name: &str,
    timeout: Duration,
) -> Result<quinn::Connection> {
    let connecting = endpoint
        .connect(addr, server_name)
        .map_err(|e| Error::Quic(format!("connect err: {e}")))?;
    let conn = tokio::time::timeout(timeout, connecting)
        .await
        .map_err(|_| Error::Timeout(format!("QUIC 握手超时: {addr}")))?
        .map_err(|e| Error::Quic(format!("握手失败: {e}")))?;
    Ok(conn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::punch::{PunchOptions, PunchSocket};
    use std::time::Duration;

    /// 纯 quinn 直连（不经打洞）：端到端验证 leader/member_endpoint 与指纹校验本身。
    #[tokio::test]
    async fn plain_quinn_connects() {
        let cert = PunchCert::generate().expect("cert");

        let leader_sock = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("leader bind");
        leader_sock.set_nonblocking(true).ok();
        let leader_addr = leader_sock.local_addr().expect("leader addr");
        let member_sock = std::net::UdpSocket::bind(("127.0.0.1", 0)).expect("member bind");
        member_sock.set_nonblocking(true).ok();

        let leader_ep = leader_endpoint(leader_sock, &cert).expect("leader ep");
        let member_ep = member_endpoint(member_sock, &cert.fingerprint).expect("member ep");

        let leader_task = tokio::spawn(async move {
            let incoming = leader_ep.accept().await.expect("accept");
            let conn = incoming.await.expect("handshake");
            let (mut send, mut recv) = conn.accept_bi().await.expect("accept bi");
            let mut buf = [0u8; 128];
            let n = recv.read(&mut buf).await.expect("read").expect("eof");
            send.write_all(&buf[..n]).await.expect("echo");
            send.finish().expect("finish");
            // 保活到对端读完 echo 并自行关闭连接（否则 drop 会中断未确认数据）
            let _ = conn.closed().await;
        });

        let conn = connect(&member_ep, leader_addr, "localhost", Duration::from_secs(5))
            .await
            .expect("quic connect");
        let (mut send, mut recv) = conn.open_bi().await.expect("open bi");
        send.write_all(b"plain-quinn").await.expect("write");
        send.finish().expect("finish");
        let buf = recv.read_to_end(128).await.expect("read echo");
        assert_eq!(&buf, b"plain-quinn");
        // 显式关闭：让组长端 conn.closed() 立即返回，避免等 idle 超时
        conn.close(quinn::VarInt::from_u32(0), b"done");
        leader_task.await.expect("leader task");
    }

    /// 打洞 → QUIC 握手 → 双向流数据收发 的完整回环链路。
    #[tokio::test]
    async fn punch_then_quic_echo_roundtrip() {
        let cert = PunchCert::generate().expect("cert");

        // 1) 双端打洞（回环，互为候选）
        let leader = PunchSocket::bind().expect("leader socket");
        let member = PunchSocket::bind().expect("member socket");
        let opts = PunchOptions { timeout: Duration::from_secs(5), ..Default::default() };
        let leader_cand = crate::Candidate::new("127.0.0.1", leader.local_addr().port(), crate::CandidateKind::Local);
        let member_cand = crate::Candidate::new("127.0.0.1", member.local_addr().port(), crate::CandidateKind::Local);
        let leader_targets = vec![member_cand];
        let member_targets = vec![leader_cand];
        let (peer_of_leader, peer_of_member) = tokio::join!(
            leader.punch(&leader_targets, &opts),
            member.punch(&member_targets, &opts),
        );
        // 成员要把 leader 的映射地址作为 QUIC 连接目标：取成员侧观测到的组长地址
        let leader_addr = peer_of_member.expect("member 观测组长失败");
        let _ = peer_of_leader.expect("leader 观测成员失败");

        // 2) 组长 QUIC 服务端（同一 socket），成员以指纹校验连接
        let leader_ep = leader_endpoint(leader.std_socket().expect("leader std"), &cert).expect("leader ep");
        let member_ep = member_endpoint(
            member.std_socket().expect("member std"),
            &cert.fingerprint,
        )
        .expect("member ep");

        // 组长接受连接 + 双向流回显
        let leader_task = tokio::spawn(async move {
            let conn = leader_ep
                .accept()
                .await
                .expect("accept conn")
                .await
                .expect("conn handshake");
            let (mut send, mut recv) = conn.accept_bi().await.expect("accept bi");
            let mut buf = vec![0u8; 256];
            // quinn read 返回 Option<usize>：None 表示对端关闭流
            let n = recv.read(&mut buf).await.expect("read request").expect("stream closed");
            let msg = &buf[..n];
            send.write_all(msg).await.expect("echo");
            send.finish().expect("finish");
            // 保活到对端读完 echo（否则 drop 会中断未确认数据）
            let _ = conn.closed().await;
        });

        // 成员连接（打洞观测到的对端地址）并收发
        let conn = connect(&member_ep, leader_addr, "localhost", Duration::from_secs(5))
            .await
            .expect("quic connect");
        let (mut send, mut recv) = conn.open_bi().await.expect("open bi");
        send.write_all(b"AIPG-punch-echo").await.expect("write");
        send.finish().expect("finish");
        let buf = recv.read_to_end(1024).await.expect("read echo");
        assert_eq!(&buf, b"AIPG-punch-echo");
        // 显式关闭：让组长端 conn.closed() 立即返回，避免等 idle 超时
        conn.close(quinn::VarInt::from_u32(0), b"done");
        leader_task.await.expect("leader task");
    }

    /// 指纹不匹配必须拒绝握手（抗中间人）。
    #[tokio::test]
    async fn wrong_fingerprint_rejected() {
        let cert = PunchCert::generate().expect("cert");
        let other = PunchCert::generate().expect("other cert");
        assert_ne!(cert.fingerprint, other.fingerprint);

        let leader = PunchSocket::bind().expect("leader socket");
        let member = PunchSocket::bind().expect("member socket");
        let opts = PunchOptions { timeout: Duration::from_secs(5), ..Default::default() };
        let leader_cand = crate::Candidate::new("127.0.0.1", leader.local_addr().port(), crate::CandidateKind::Local);
        let member_cand = crate::Candidate::new("127.0.0.1", member.local_addr().port(), crate::CandidateKind::Local);
        let leader_targets = vec![member_cand];
        let member_targets = vec![leader_cand];
        let _ = tokio::join!(
            leader.punch(&leader_targets, &opts),
            member.punch(&member_targets, &opts),
        );

        let leader_ep = leader_endpoint(leader.std_socket().expect("leader std"), &cert).expect("leader ep");
        // 成员拿错误指纹（他端证书）→ 连接必须失败
        let member_ep = member_endpoint(
            member.std_socket().expect("member std"),
            &other.fingerprint,
        )
        .expect("member ep");

        let leader_task = tokio::spawn(async move {
            // 客户端指纹不符会中止握手：incoming 完成后即连接失败
            if let Some(incoming) = leader_ep.accept().await {
                let _ = incoming.await;
            }
        });

        let res = connect(
            &member_ep,
            std::net::SocketAddr::from(([127, 0, 0, 1], leader.local_addr().port())),
            "localhost",
            Duration::from_secs(3),
        )
        .await;
        assert!(res.is_err(), "错误指纹应导致握手失败");
        leader_task.await.expect("leader task");
    }
}