//! 跨网络深链加密端到端测试：真组长(ShareServer + MockBackend) + 真成员(MemberGateway)。
//!
//! 覆盖（design 2026-08-26 §9 测试计划）：
//! 1. 免密 /auth/token 明文换取令牌（排除端点，加密协商前可达）；
//! 2. 成员 encrypt=AesGcm + 静态组长 → 请求/响应经 x-aipg-enc 全链路加密，业务明文两端可见；
//! 3. 无 x-aipg-enc 明文请求（LAN 旧成员 / encrypt=Off）组长照样 200（零兼容回归）；
//! 4. 加密请求携带错误 bearer → 组长解密失败返回 400；
//! 5. GET /v1/models 无体加密协商（仅加密响应）。

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use aipg_lan_client::gateway::MemberGateway;
use aipg_lan_client::{DiscoveryClient, DiscoveryConfig, LeaderInfo};
use aipg_lan_share::backend::{Backend, MockBackend};
use aipg_lan_share::registry::BackendRegistry;
use aipg_lan_share::server::{ShareServer, ShareServerConfig};
use aipg_link_crypto::LinkEncryptMode;

fn test_config() -> ShareServerConfig {
    let dir = std::env::temp_dir().join(format!("aipg-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    ShareServerConfig {
        port: 39091,
        bind: [127, 0, 0, 1].into(),
        share_port: 39092,
        token_ttl_secs: 0,
        heartbeat_timeout_secs: 90,
        name: "e2e-leader".to_string(),
        data_dir: dir.clone(),
        web_dir: dir.join("web"),
        link_encrypt: LinkEncryptMode::AesGcm,
    }
}

/// 启动组长共享通道(share_router)到随机端口，返回 (端口, JoinHandle)。
async fn spawn_leader_with(cfg: ShareServerConfig) -> (MemberGateway, u16, tokio::task::JoinHandle<()>) {
    let registry = BackendRegistry::new();
    registry.register(Arc::new(MockBackend::default()) as Arc<dyn Backend>);
    let server = ShareServer::new(&cfg, registry);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    // auth_token 使用 ConnectInfo，须带上连接地址扩展
    let app = server.share_router().into_make_service_with_connect_info::<SocketAddr>();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let gateway = MemberGateway::with_encrypt(DiscoveryClient::new(DiscoveryConfig::default()), LinkEncryptMode::AesGcm);
    let leader = LeaderInfo {
        name: "e2e-leader".to_string(),
        api_port: port,
        share_port: Some(port), // 深链直连组长 API 端口
        fingerprint: "e2e".to_string(),
        address: "127.0.0.1".to_string(),
        last_seen: 0,
        online: true,
    };
    gateway.set_static_leader(leader);
    (gateway, port, handle)
}

/// 启动组长共享通道(share_router)到随机端口，返回 (港口, JoinHandle)。默认协商式策略。
async fn spawn_leader() -> (MemberGateway, u16, tokio::task::JoinHandle<()>) {
    spawn_leader_with(test_config()).await
}

fn client() -> reqwest::Client {
    let cli = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap();
    cli
}

async fn wait_up(port: u16) {
    for _ in 0..20 {
        if client().get(format!("http://127.0.0.1:{port}/v1/models")).send().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("leader not ready in 1s");
}

#[tokio::test]
async fn cross_network_encrypted_chat_roundtrip() {
    let (gw, port, handle) = spawn_leader().await;
    wait_up(port).await;

    // 1) 明文换取令牌（/auth/token 为排除端点，加密协商前可达）
    let resp = client()
        .post(format!("http://127.0.0.1:{port}/auth/token"))
        .header("content-type", "application/json")
        .body(r#"{"machineName":"e2e-member-1","displayName":"E2E"}"#)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200, "auth/token 明文可达");
    let token = resp.json::<serde_json::Value>().await.unwrap()["token"].as_str().unwrap().to_string();

    // 2) 加密调用链：成员 proxy 加密请求 → 组长解密 → mock 回复 → 组长加密 → 成员解密
    let body = serde_json::json!({
        "model": "mock-7b",
        "messages": [{ "role": "user", "content": "hello e2e" }]
    });
    let (status, bytes) = gw
        .proxy("/v1/chat/completions", Some(&format!("Bearer {token}")), Some(serde_json::to_vec(&body).unwrap()))
        .await
        .unwrap_or_else(|e| panic!("encrypted proxy failed: {e}"));
    assert_eq!(status, 200, "加密链路应 200");
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["choices"][0]["message"]["content"].to_string().contains("mock reply"), "明文两端可见: {json}");

    // 3) 兼容：无协商头明文请求（encrypt=Off 的旧成员/web 客户端）组长照常 200
    let plain_resp = client()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(serde_json::to_vec(&body).unwrap())
        .send().await.unwrap();
    assert_eq!(plain_resp.status().as_u16(), 200, "明文请求零兼容回归");
    let plain_json: serde_json::Value = plain_resp.json().await.unwrap();
    assert!(plain_json["choices"][0]["message"]["content"].to_string().contains("mock reply"));

    // 4) 密钥与 bearer 不一致（篡改/错钥）→ 组长解密失败 400
    let crypto = aipg_link_crypto::LinkCrypto::new();
    let key = aipg_link_crypto::LinkCrypto::derive_key("key-used-to-encrypt");
    let enc_body = crypto.encrypt(&key, &serde_json::to_vec(&body).unwrap());
    let tampered = client()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("content-type", "application/json")
        .header("authorization", "Bearer different-token") // 与加密 key 不一致
        .header("x-aipg-enc", "v1")
        .body(enc_body)
        .send().await.unwrap();
    assert_eq!(tampered.status().as_u16(), 400, "错钥应拒绝: {}", tampered.text().await.unwrap());

    // 4b) bearer 与加密 key 一致但令牌无效 → 解密成功、认证层拒绝（401）
    let (bad_status, _) = gw
        .proxy("/v1/chat/completions", Some("Bearer wrong-token"), Some(serde_json::to_vec(&body).unwrap()))
        .await
        .unwrap_or_else(|e| panic!("bad-token proxy: {e}"));
    assert_eq!(bad_status, 401, "无效令牌应认证拒绝");

    handle.abort();
}

#[tokio::test]
async fn cross_network_encrypted_models_get() {
    let (gw, port, handle) = spawn_leader().await;
    wait_up(port).await;

    // GET /v1/models：成员持有 bearer 时协商加密（无体请求不解密 body，仅加密响应）
    let resp = client()
        .post(format!("http://127.0.0.1:{port}/auth/token"))
        .header("content-type", "application/json")
        .body(r#"{"machineName":"e2e-models","displayName":"M"}"#)
        .send().await.unwrap();
    let token = resp.json::<serde_json::Value>().await.unwrap()["token"].as_str().unwrap().to_string();

    let (status, bytes) = gw
        .proxy("/v1/models", Some(&format!("Bearer {token}")), None)
        .await
        .unwrap_or_else(|e| panic!("encrypted models proxy failed: {e}"));
    assert_eq!(status, 200);
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    // OpenAI 格式 { "data": [ {id, object, created, owned_by}, ... ], "object": "list" }
    assert!(json["data"].as_array().is_some() && json["data"].as_array().unwrap().len() > 0, "models 目录解密成功: {json}");

    handle.abort();
}

/// M3 组长端强制策略：link.encrypt = enforce 时未声明加密的 /v1/* 回 426，加密请求照常 200。
#[tokio::test]
async fn enforce_leader_rejects_unencrypted_v1() {
    let mut cfg = test_config();
    // 强制策略：未加密 /v1/* → 426；排除端点 /auth 与加密请求不受影响
    cfg.link_encrypt = LinkEncryptMode::Enforce;
    let (gw, port, handle) = spawn_leader_with(cfg).await;
    wait_up(port).await;

    // 1) 排除端点明文换令牌（enforce 下仍必须可达，否则协商前死锁）
    let resp = client()
        .post(format!("http://127.0.0.1:{port}/auth/token"))
        .header("content-type", "application/json")
        .body(r#"{"machineName":"e2e-enforce","displayName":"E2E"}"#)
        .send().await.unwrap();
    assert_eq!(resp.status().as_u16(), 200, "enforce 下 /auth/token 仍明文可达");
    let token = resp.json::<serde_json::Value>().await.unwrap()["token"].as_str().unwrap().to_string();

    // 2) 未声明加密的 /v1/chat/completions → 426 Upgrade Required（带 Upgrade 头提示协议）
    let plain = client()
        .post(format!("http://127.0.0.1:{port}/v1/chat/completions"))
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {token}"))
        .body(r#"{"model":"mock-7b","messages":[{"role":"user","content":"hi"}]}"#)
        .send().await.unwrap();
    assert_eq!(plain.status().as_u16(), 426, "未声明加密的 /v1/* 在 enforce 下应 426: {}", plain.text().await.unwrap());
    assert_eq!(
        plain.headers().get("upgrade").and_then(|v| v.to_str().ok()).unwrap_or_default(),
        "x-aipg-enc",
        "426 应带 Upgrade: x-aipg-enc 提示所需协议"
    );

    // 3) 成员加密请求（x-aipg-enc: v1）→ 照常 200，business 明文两端可见
    let body = serde_json::json!({
        "model": "mock-7b",
        "messages": [{ "role": "user", "content": "hello enforce" }]
    });
    let (status, bytes) = gw
        .proxy("/v1/chat/completions", Some(&format!("Bearer {token}")), Some(serde_json::to_vec(&body).unwrap()))
        .await
        .unwrap_or_else(|e| panic!("enforced encrypted proxy failed: {e}"));
    assert_eq!(status, 200, "enforce 下加密请求应 200");
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(json["choices"][0]["message"]["content"].to_string().contains("mock reply"), "加密链路在 enforce 下正常: {json}");

    handle.abort();
}