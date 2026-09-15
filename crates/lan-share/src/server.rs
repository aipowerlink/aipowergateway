//! lan-share-server：axum HTTP 服务装配 + 共享开关 + UDP 广播 + Module 实现。

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::routing::{get, post};
use axum::Router;

use aipg_runtime::{Module, ModuleContext, RuntimeError, RuntimeResult};
use aipg_link_crypto::LinkEncryptMode;

use crate::api::{self, ApiState};
use crate::auth::AuthService;
use crate::backend::{BackendEntry, MockBackend};
use crate::backend_store::BackendStore;
use crate::registry::BackendRegistry;
use crate::member::MemberRegistry;
use crate::quota::QuotaService;
use crate::usage::UsageService;

/// 共享服务配置。
#[derive(Debug, Clone)]
pub struct ShareServerConfig {
    pub port: u16,
    /// 绑定地址：0.0.0.0 = 局域网共享（默认）；127.0.0.1 = 仅本机访问。
    pub bind: std::net::IpAddr,
    /// gateway 间共享通道端口：成员 gateway 经此端口与组长 gateway 通信（独立于管理/API 端口）。
    pub share_port: u16,
    pub token_ttl_secs: u64,
    pub heartbeat_timeout_secs: u64,
    /// 广播/网关名（gatewayId = {name}:{port}）。
    pub name: String,
    pub data_dir: std::path::PathBuf,
    /// 管理网页静态资源目录（web/dist）。
    pub web_dir: std::path::PathBuf,
    /// 链路加密策略（M3 组长端）：off=纯透传 / aes-gcm=协商式(默认) / enforce=未加密 /v1/* 回 426。
    /// 持久化：data_dir/link-encrypt.json 运行时可改（面板开关），重启后文件优先于本字段。
    pub link_encrypt: LinkEncryptMode,
}

impl Default for ShareServerConfig {
    fn default() -> Self {
        Self {
            port: 39091,
            // 默认仅本机访问（管理页面 / OpenAI / Anthropic 三类入口）；局域网共享需显式 bind 0.0.0.0
            bind: [127, 0, 0, 1].into(),
            share_port: 39092,
            // 0 = 永久有效（管理/API 只监听 127.0.0.1，key 仅暴露在本机，无需定期轮换）
            token_ttl_secs: 0,
            heartbeat_timeout_secs: 90,
            name: "aipowerlink-share".to_string(),
            data_dir: std::env::temp_dir().join("aipowerlink-test"),
            web_dir: std::env::current_dir().unwrap_or_default().join("web").join("dist"),
            // 缺省协商式：带 x-aipg-enc 才加解密，无头明文透传（旧成员/LAN 完全兼容）
            link_encrypt: LinkEncryptMode::AesGcm,
        }
    }
}

/// 共享服务：管理 API 服务生命周期。
#[derive(Clone)]
pub struct ShareServer {
    state: ApiState,
    port: u16,
    bind: std::net::IpAddr,
    share_port: u16,
    web_dir: std::path::PathBuf,
}

impl ShareServer {
    /// 内部构造（backends 注册表 + 空配置存储）。
    fn assemble(cfg: &ShareServerConfig, backends: BackendRegistry, store: BackendStore) -> Self {
        let usage_path = cfg.data_dir.join("usage.json");
        let quota_path = cfg.data_dir.join("quota.json");
        let gateway_id = format!("{}:{}", cfg.name, cfg.port);
        // 链路加密策略（M3）：文件优先（面板运行时更改落盘），无文件时回落到 CLI/config 字段
        let link_policy_file = cfg.data_dir.join("link-encrypt.json");
        let link_policy = match std::fs::read_to_string(&link_policy_file) {
            Ok(s) => LinkEncryptMode::parse(s.trim()),
            Err(_) => cfg.link_encrypt,
        };
        // 健康轮询（P1）：与注册表共享状态表；无启用条目不 spawn（零开销）
        let health = Arc::new(crate::health::HealthMonitor::new());
        backends.attach_health(health.states());
        // 负载红线拦截（挖矿/深伪）：默认开启，load-policy.json 文件优先（面板开关落盘）
        let policy = Arc::new(crate::policy::LoadPolicy::new(cfg.data_dir.join("load-policy.json")));
        // 规则执行引擎（模型名=规则名）：model-rule-set.json 文件优先（面板保存落盘，重启后生效）
        let rules_file = cfg.data_dir.join("model-rule-set.json");
        let rules = Arc::new(crate::rules::RuleResolver::load_from_file(&rules_file));
        Self {
            state: ApiState {
                auth: AuthService::new_with_store(
                    cfg.token_ttl_secs,
                    Some(cfg.data_dir.join("banned.json")),
                    Some(cfg.data_dir.join("sessions.json")),
                ),
                members: MemberRegistry::new(cfg.heartbeat_timeout_secs, &gateway_id),
                usage: UsageService::new(usage_path),
                quota: QuotaService::new(quota_path),
                backends: Arc::new(backends),
                backends_config: Arc::new(store),
                sharing: Arc::new(AtomicBool::new(true)),
                test_status: Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())),
                health,
                link_policy: Arc::new(std::sync::RwLock::new(link_policy)),
                link_policy_file,
                policy,
                rules,
                rules_file,
                port: cfg.port,
                bind: cfg.bind,
                share_port: cfg.share_port,
            },
            port: cfg.port,
            bind: cfg.bind,
            share_port: cfg.share_port,
            web_dir: cfg.web_dir.clone(),
        }
    }

    /// 构造服务（不启动监听）。backend 为单后端时自动包装为注册表。
    pub fn new(cfg: &ShareServerConfig, backends: BackendRegistry) -> Self {
        let store = BackendStore::new(cfg.data_dir.join("backends.yaml"), Vec::new());
        Self::assemble(cfg, backends, store)
    }

    /// 构造服务：从配置条目（--backend/环境变量 + backends.yaml）构建注册表。
    /// 文件配置优先；启动条目仅补齐文件缺失项；面板保存后热更新（首次保存固化到文件）。
    pub fn with_entries(cfg: &ShareServerConfig, initial_entries: Vec<BackendEntry>) -> anyhow::Result<Self> {
        let store = BackendStore::new(cfg.data_dir.join("backends.yaml"), initial_entries);
        let registry = crate::registry::registry_from_entries(&store.list())?;
        Ok(Self::assemble(cfg, registry, store))
    }

    pub fn state(&self) -> &ApiState {
        &self.state
    }

    /// 构建 axum Router（含管理网页静态托管）。
    pub fn router(&self) -> Router {
        let state = self.state.clone();
        let web_dir = self.web_dir.clone();
        // 管理网页禁止浏览器缓存：vite 产物 hash 变更时若缓存旧 index.html 会一直看到旧面板
        let no_cache = tower_http::set_header::SetResponseHeaderLayer::if_not_present(
            http::header::CACHE_CONTROL,
            http::HeaderValue::from_static("no-cache"),
        );
        let serve_dir = tower_http::services::ServeDir::new(&web_dir)
            .append_index_html_on_directories(true);
        let static_service = tower::ServiceBuilder::new()
            .layer(no_cache)
            .service(serve_dir);
        Router::new()
            .route("/v1/chat/completions", post(api::chat_completions))
            .route("/v1/models", get(api::models_openai))
            .route("/v1/messages", post(api::messages))
            .route("/auth/token", post(api::auth_token))
            .route("/auth/rename", post(api::auth_rename))
            .route("/api/control", post(api::api_control))
            .route("/api/members", get(api::api_members))
            .route("/api/strategy/me", get(api::api_strategy_me))
            .route("/api/usage/export", get(api::api_usage_export))
            .route("/api/quota", get(api::api_quota_list).post(api::api_quota_set))
            .route("/api/backends", get(api::api_backends_list).post(api::api_backends_set))
            .route("/api/rules", get(api::api_rules_list).post(api::api_rules_set))
            .route("/api/backends/test", axum::routing::post(api::api_backends_test))
            .route("/api/backends/{id}", axum::routing::delete(api::api_backends_delete))
            .route("/api/backends/{id}/polling", axum::routing::put(api::api_backends_polling))
            .route("/api/info", axum::routing::get(api::api_info))
        .route("/api/models", axum::routing::get(api::api_models))
            .fallback_service(static_service)
            .layer(axum::middleware::from_fn_with_state(state.clone(), crate::link::link_enc_middleware))
            .with_state(state)
    }

    /// gateway 间共享通道 Router：仅承载成员 gateway 接入端点（免密换令牌 + 双协议调用）。
    /// 独立监听（默认 0.0.0.0:39092），不暴露管理/配置端点。
    pub fn share_router(&self) -> Router {
        let state = self.state.clone();
        Router::new()
            .route("/v1/chat/completions", post(api::chat_completions))
            .route("/v1/models", get(api::models_openai))
            .route("/v1/messages", post(api::messages))
            .route("/auth/token", post(api::auth_token))
            // 策略摘要：成员机 client 经共享通道拉取只读策略镜像（规则名/配额/拉黑）
            .route("/api/strategy/me", get(api::api_strategy_me))
            .layer(axum::middleware::from_fn_with_state(state.clone(), crate::link::link_enc_middleware))
            .with_state(state)
    }

    /// 启动监听（阻塞直到服务端 shutdown）：管理/API 入口走 bind:port，共享通道走 0.0.0.0:share_port。
    pub async fn serve(&self) -> RuntimeResult<()> {
        // 健康轮询（P1）：存在启用条目则拉起调度循环；无条目时零开销（不 spawn）
        let health = self.state.health.clone();
        let store = self.state.backends_config.clone();
        tokio::spawn(async move { health.ensure_running(store) });
        let addr = SocketAddr::from((self.bind, self.port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            RuntimeError::Other(format!("bind {addr}: {e}"))
        })?;
        tracing::info!("lan-share-server listening on http://{addr}");
        let share_addr = SocketAddr::from((std::net::IpAddr::from([0, 0, 0, 0]), self.share_port));
        let share_listener = tokio::net::TcpListener::bind(share_addr).await.map_err(|e| {
            RuntimeError::Other(format!("bind share channel {share_addr}: {e}"))
        })?;
        tracing::info!("lan-share gateway channel listening on http://{share_addr} (member gateways)");
        let srv = axum::serve(listener, self.router().into_make_service_with_connect_info::<SocketAddr>());
        let share_srv = axum::serve(share_listener, self.share_router().into_make_service_with_connect_info::<SocketAddr>());
        tokio::try_join!(srv, share_srv).map_err(|e| RuntimeError::Other(format!("serve: {e}")))?;
        Ok(())
    }

    /// 共享开关。
    pub fn set_sharing(&self, on: bool) {
        self.state.sharing.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn sharing(&self) -> bool {
        self.state.sharing.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Module 实现：lan-share-server。
pub struct LanShareServerModule {
    cfg: ShareServerConfig,
}

impl LanShareServerModule {
    pub fn new(cfg: ShareServerConfig) -> Self {
        Self { cfg }
    }
}

impl Module for LanShareServerModule {
    fn name(&self) -> &'static str {
        aipg_runtime::MOD_LAN_SHARE_SERVER
    }

    fn requires(&self) -> &'static [&'static str] {
        &[]
    }

    fn apply(&self, ctx: ModuleContext<'_>) -> RuntimeResult<()> {
        let registry = BackendRegistry::new();
        registry.register(Arc::new(MockBackend::default()) as Arc<dyn crate::backend::Backend>);
        let server = ShareServer::new(&self.cfg, registry);
        // 注册服务供其他模块消费
        ctx.host.provide("lan-share-server", server.clone());
        ctx.host.provide("lan-auth", server.state().auth.clone());
        ctx.host.provide("lan-member-registry", server.state().members.clone());
        ctx.host.provide("lan-usage", server.state().usage.clone());
        ctx.host.provide("lan-backends", server.state().backends.clone());
        tracing::info!(port = self.cfg.port, "lan-share-server module applied");
        Ok(())
    }
}