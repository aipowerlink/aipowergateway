//! lan-discovery-broadcast：UDP 周期广播共享服务信息（服务名/API 端口/指纹）。
//! 组员端据此自动发现组长（消费端 lan-discovery-client 的对端）。

use std::net::{SocketAddr, UdpSocket};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aipg_runtime::{Module, ModuleContext, RuntimeResult};

/// 广播服务配置。
#[derive(Debug, Clone)]
pub struct BroadcastConfig {
    /// 广播端口（组员端监听同端口）。
    pub port: u16,
    /// 服务名（组长标识）。
    pub name: String,
    /// API 端口（组员连这个端口调用）。
    pub api_port: u16,
    /// gateway 间共享通道端口（成员 gateway 经此端口与组长 gateway 通信）。
    pub share_port: u16,
    /// 指纹（密码哈希前 N 位，组员可预校验）。
    pub fingerprint: String,
    /// 广播间隔（秒）。
    pub interval_secs: u64,
    /// 广播目标地址（默认子网广播地址）。
    pub target: String,
}

impl Default for BroadcastConfig {
    fn default() -> Self {
        Self {
            port: 39090,
            name: "aipowerlink-share".to_string(),
            api_port: 39091,
            share_port: 39092,
            fingerprint: "".to_string(),
            interval_secs: 10,
            target: "255.255.255.255".to_string(),
        }
    }
}

/// UDP 周期广播器。
#[derive(Clone)]
pub struct BroadcastService {
    cfg: BroadcastConfig,
    running: Arc<AtomicBool>,
}

impl BroadcastService {
    pub fn new(cfg: BroadcastConfig) -> Self {
        Self { cfg, running: Arc::new(AtomicBool::new(false)) }
    }

    /// 启动周期广播（后台任务，直到 stop 或进程退出）。
    pub fn start(&self) {
        if self.running.swap(true, Ordering::Relaxed) {
            return; // 已在运行
        }
        let cfg = self.cfg.clone();
        let running = self.running.clone();
        tokio::spawn(async move {
            tracing::info!(port = cfg.port, name = %cfg.name, "discovery broadcast started");
            // 绑定广播源端口（任意可用）
            let socket = match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("broadcast bind failed: {e}");
                    running.store(false, Ordering::Relaxed);
                    return;
                }
            };
            // 允许广播（Windows 需 set_broadcast）
            if let Err(e) = socket.set_broadcast(true) {
                tracing::warn!("set_broadcast failed: {e}");
            }
            let payload = serde_json::json!({
                "type": "AIPG_ANNOUNCE",
                "name": cfg.name,
                "api_port": cfg.api_port,
                "share_port": cfg.share_port,
                "fingerprint": cfg.fingerprint,
            });
            let data = payload.to_string();
            let addr: SocketAddr = format!("{}:{}", cfg.target, cfg.port)
                .parse()
                .unwrap_or_else(|_| SocketAddr::from(([255, 255, 255, 255], cfg.port)));
            let interval = std::time::Duration::from_secs(cfg.interval_secs);
            while running.load(Ordering::Relaxed) {
                if let Err(e) = socket.send_to(data.as_bytes(), addr) {
                    tracing::debug!("broadcast send failed: {e}");
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    /// 停止广播。
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
        tracing::info!("discovery broadcast stopped");
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

/// Module 实现：lan-discovery-broadcast。
pub struct LanDiscoveryBroadcastModule {
    cfg: BroadcastConfig,
}

impl LanDiscoveryBroadcastModule {
    pub fn new(cfg: BroadcastConfig) -> Self {
        Self { cfg }
    }
}

impl Module for LanDiscoveryBroadcastModule {
    fn name(&self) -> &'static str {
        aipg_runtime::MOD_LAN_DISCOVERY_BROADCAST
    }

    fn apply(&self, ctx: ModuleContext<'_>) -> RuntimeResult<()> {
        let svc = BroadcastService::new(self.cfg.clone());
        ctx.host.provide("lan-discovery-broadcast", svc.clone());
        // 由装配方显式 start（模块 apply 只注册服务，不启动循环）
        tracing::info!(port = self.cfg.port, "lan-discovery-broadcast module applied");
        Ok(())
    }
}