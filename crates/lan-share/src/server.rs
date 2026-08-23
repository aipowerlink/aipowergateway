//! lan-share-server：axum HTTP 服务装配 + 共享开关 + UDP 广播 + Module 实现。

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use axum::routing::{get, post};
use axum::Router;

use aipg_runtime::{Module, ModuleContext, RuntimeError, RuntimeResult};

use crate::api::{self, ApiState};
use crate::auth::AuthService;
use crate::backend::{Backend, MockBackend};
use crate::member::MemberRegistry;
use crate::usage::UsageService;

/// 共享服务配置。
#[derive(Debug, Clone)]
pub struct ShareServerConfig {
    pub port: u16,
    pub password: String,
    pub token_ttl_secs: u64,
    pub heartbeat_timeout_secs: u64,
    pub data_dir: std::path::PathBuf,
}

impl Default for ShareServerConfig {
    fn default() -> Self {
        Self {
            port: 39091,
            password: "aipowerlink".to_string(),
            token_ttl_secs: 12 * 3600,
            heartbeat_timeout_secs: 90,
            data_dir: std::env::temp_dir().join("aipowerlink-test"),
        }
    }
}

/// 共享服务：管理 API 服务生命周期。
#[derive(Clone)]
pub struct ShareServer {
    state: ApiState,
    port: u16,
}

impl ShareServer {
    /// 构造服务（不启动监听）。
    pub fn new(cfg: &ShareServerConfig, backend: Arc<dyn Backend>) -> Self {
        let usage_path = cfg.data_dir.join("usage.json");
        Self {
            state: ApiState {
                auth: AuthService::new(&cfg.password, cfg.token_ttl_secs),
                members: MemberRegistry::new(cfg.heartbeat_timeout_secs),
                usage: UsageService::new(usage_path),
                backend,
                sharing: Arc::new(AtomicBool::new(true)),
            },
            port: cfg.port,
        }
    }

    pub fn state(&self) -> &ApiState {
        &self.state
    }

    /// 构建 axum Router。
    pub fn router(&self) -> Router {
        let state = self.state.clone();
        Router::new()
            .route("/v1/chat/completions", post(api::chat_completions))
            .route("/v1/messages", post(api::messages))
            .route("/auth/token", post(api::auth_token))
            .route("/auth/rename", post(api::auth_rename))
            .route("/api/control", post(api::api_control))
            .route("/api/members", get(api::api_members))
            .with_state(state)
    }

    /// 启动 HTTP 监听（阻塞直到服务端 shutdown）。
    pub async fn serve(&self) -> RuntimeResult<()> {
        let addr = SocketAddr::from(([0, 0, 0, 0], self.port));
        let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| {
            RuntimeError::Other(format!("bind {addr}: {e}"))
        })?;
        tracing::info!("lan-share-server listening on http://{addr}");
        axum::serve(listener, self.router()).await.map_err(|e| {
            RuntimeError::Other(format!("serve: {e}"))
        })
    }

    /// 共享开关。
    pub fn set_sharing(&self, on: bool) {
        self.state.sharing.store(on, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn sharing(&self) -> bool {
        self.state.sharing.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// 广播指纹访问。
impl ShareServer {
    pub fn fingerprint(&self, n: usize) -> String {
        self.state.auth.fingerprint(n)
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
        let backend = Arc::new(MockBackend::default()) as Arc<dyn Backend>;
        let server = ShareServer::new(&self.cfg, backend);
        // 注册服务供其他模块消费
        ctx.host.provide("lan-share-server", server.clone());
        ctx.host.provide("lan-auth", server.state().auth.clone());
        ctx.host.provide("lan-member-registry", server.state().members.clone());
        ctx.host.provide("lan-usage", server.state().usage.clone());
        tracing::info!(port = self.cfg.port, "lan-share-server module applied");
        Ok(())
    }
}