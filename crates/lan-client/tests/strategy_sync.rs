//! 策略同步 E2E（方案「策略同步到需求人节点」）：真组长(ShareServer + 规则 + 配额) + 真成员(MemberGateway)。
//!
//! 覆盖：
//! 1. 成员机经共享通道拉取 GET /api/strategy/me → 规则名列表 + 本机配额 + 拉黑状态；
//! 2. 零知识边界：摘要只含规则名（不含真实上游候选模型）；
//! 3. 组长拉黑成员后，同 token 拉取 → banned:true（本地提前 403 的依据）；
//! 4. 跨网络深链(AesGcm)下同样可达（/api/strategy/me 挂共享通道 + 链路加密自动协商）。

use std::net::SocketAddr;
use std::sync::Arc;

use aipg_lan_client::gateway::MemberGateway;
use aipg_lan_client::{DiscoveryClient, DiscoveryConfig, LeaderInfo, fetch_strategy};
use aipg_lan_share::backend::{Backend, MockBackend};
use aipg_lan_share::registry::BackendRegistry;
use aipg_lan_share::rules::{Candidate, Rule, RuleSet};
use aipg_lan_share::server::{ShareServer, ShareServerConfig};
use aipg_link_crypto::LinkEncryptMode;

fn test_config() -> ShareServerConfig {
    let dir = std::env::temp_dir().join(format!("aipg-strategy-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    ShareServerConfig {
        port: 0,
        bind: [127, 0, 0, 1].into(),
        share_port: 0,
        token_ttl_secs: 0,
        heartbeat_timeout_secs: 90,
        name: "strategy-leader".to_string(),
        data_dir: dir.clone(),
        web_dir: dir.join("web"),
        link_encrypt: LinkEncryptMode::AesGcm,
    }
}

/// 启动组长共享通道到随机端口；注入一条规则 + 成员配额。返回 (server, port, MemberGateway, handle)。
async fn spawn_leader(cfg: ShareServerConfig) -> (ShareServer, u16, MemberGateway, tokio::task::JoinHandle<()>) {
    let registry = BackendRegistry::new();
    registry.register(Arc::new(MockBackend::default()) as Arc<dyn Backend>);
    let server = ShareServer::new(&cfg, registry);
    // 注入规则（真实候选 moonshot-v1-8k 只存在于组长侧：摘要不下发候选，零知识）
    server.state().rules.load(vec![RuleSet {
        id: "rs-1".into(),
        name: "deepseek-r1".into(),
        version: 1,
        rules: vec![Rule {
            match_model: "*".into(),
            order: 0,
            strategy: "token_tier".into(),
            candidates: vec![Candidate { model: "moonshot-v1-8k".into(), max_prompt_tokens: None }],
            fallback: vec![],
        }],
    }]);
    server.state().quota.set("member-pc-1", 1000);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = server.share_router().into_make_service_with_connect_info::<SocketAddr>();
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let gateway = MemberGateway::with_encrypt(DiscoveryClient::new(DiscoveryConfig::default()), LinkEncryptMode::AesGcm);
    let leader = LeaderInfo {
        name: "strategy-leader".to_string(),
        api_port: port,
        share_port: Some(port),
        fingerprint: "e2e".to_string(),
        address: "127.0.0.1".to_string(),
        last_seen: 0,
        online: true,
    };
    gateway.set_static_leader(leader);
    (server, port, gateway, handle)
}

#[tokio::test]
async fn member_pulls_strategy_mirror_via_share_channel() {
    let (server, port, gateway, handle) = spawn_leader(test_config()).await;
    wait_up(port).await;

    let summary = fetch_strategy(&gateway, "member-pc-1").await.expect("策略摘要应可拉取");
    // 1) 规则名下发（能力可见）
    assert_eq!(summary.rules, vec!["deepseek-r1"]);
    // 2) 零知识：真实上游候选绝不下发
    assert!(!summary.rules.iter().any(|r| r.contains("moonshot")), "摘要不得泄露真实候选模型: {:?}", summary.rules);
    // 3) 本机配额
    assert_eq!(summary.quota.limit, 1000);
    assert_eq!(summary.quota.used, 0);
    assert!(!summary.banned);
    // 4) version 指纹稳定
    assert!(summary.version > 0);

    handle.abort();
    let _ = server;
}

#[tokio::test]
async fn banned_member_receives_banned_flag() {
    let (server, port, gateway, handle) = spawn_leader(test_config()).await;
    wait_up(port).await;

    // 成员已换 token 接入后,组长拉黑 → 同 token 应能读到 banned:true(本地提前 403 的依据)
    let _ = fetch_strategy(&gateway, "member-pc-1").await.expect("首拉应成功");
    server.state().auth.revoke_member("member-pc-1", "127.0.0.1");
    let summary = fetch_strategy(&gateway, "member-pc-1").await.expect("被拉黑成员仍应拉到 banned:true");
    assert!(summary.banned, "拉黑后策略摘要应上报 banned:true");
    assert!(!summary.rules.is_empty(), "规则目录对被拉黑成员仍可读(本地 /v1/models 仍展示,请求被 403 拦)");

    handle.abort();
    let _ = server;
}

// ---------- 辅助 ----------

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().unwrap()
}

async fn wait_up(port: u16) {
    for _ in 0..20 {
        if client().get(format!("http://127.0.0.1:{port}/auth/token")).send().await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("leader not ready in 1s");
}