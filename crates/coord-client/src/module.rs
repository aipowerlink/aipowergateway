//! coord-client 的 runtime Module 实现：装配时启动注册 + 心跳循环（后台 tokio 任务）。
//! 对应角色默认模块：server/client 均含 MOD_COORD_CLIENT（0.3.0 起）。

use aipg_runtime::module::ModuleContext;
use aipg_runtime::{Module, RuntimeResult, MOD_COORD_CLIENT};

use crate::{DeviceClient, DeviceClientConfig};
use crate::device::HeartbeatTelemetry;

/// 协调服务器客户端模块配置。
#[derive(Debug, Clone)]
pub struct CoordClientModule {
    /// 协调服务器 base URL（空 = 不启用，纯局域网模式）。
    pub base_url: String,
    /// 节点信息（注册用）。
    pub node: crate::NodeInfo,
    /// 遥测开关（默认关闭，opt-in）。
    pub telemetry_enabled: bool,
}

impl Default for CoordClientModule {
    fn default() -> Self {
        Self {
            base_url: String::new(),
            node: crate::NodeInfo {
                name: String::new(),
                platform: String::new(),
                version: aipg_runtime::VERSION.to_string(),
                public_ip: String::new(),
                api_port: 39091,
                region_hint: None,
            },
            telemetry_enabled: false,
        }
    }
}

impl CoordClientModule {
    /// 从模块配置 JSON 构造（兼容 schema 驱动）。
    pub fn from_config(cfg: &serde_json::Value) -> Self {
        let base = cfg.get("base_url").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let node = crate::NodeInfo {
            name: cfg.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            platform: cfg.get("platform").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            version: cfg.get("version").and_then(|v| v.as_str()).unwrap_or(aipg_runtime::VERSION).to_string(),
            public_ip: cfg.get("public_ip").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            api_port: cfg.get("api_port").and_then(|v| v.as_u64()).unwrap_or(39091) as u16,
            region_hint: cfg.get("region_hint").and_then(|v| v.as_str()).map(|s| s.to_string()),
        };
        Self {
            base_url: base,
            node,
            telemetry_enabled: cfg.get("telemetry_enabled").and_then(|v| v.as_bool()).unwrap_or(false),
        }
    }
}

impl Module for CoordClientModule {
    fn name(&self) -> &'static str {
        MOD_COORD_CLIENT
    }

    fn requires(&self) -> &'static [&'static str] {
        &[]
    }

    /// 装配：注册协调服务器服务 + 启动后台注册/心跳任务。
    fn apply(&self, ctx: ModuleContext<'_>) -> RuntimeResult<()> {
        let base_url = self.base_url.clone();
        if base_url.is_empty() {
            // 未配置协调服务器 → 纯局域网模式（零服务器独立运行，合法边界）
            tracing::info!("coord-client disabled (no base_url) — LAN-only mode");
            return Ok(());
        }
        let client_cfg = DeviceClientConfig {
            base_url: base_url.clone(),
            heartbeat_interval_s: 60,
            timeout_s: 10,
        };
        let client = DeviceClient::new(client_cfg);
        let node = self.node.clone();
        let telemetry = HeartbeatTelemetry {
            enabled: self.telemetry_enabled,
            platform: node.platform.clone(),
            version: node.version.clone(),
            region_hint: node.region_hint.clone().unwrap_or_default(),
        };

        // 注册服务（其他模块可获取 device_token / share_id）
        let client_clone = client.clone();
        ctx.host.provide("coord-client", client_clone);

        // 后台任务：注册 → 心跳循环
        let node2 = node.clone();
        let telemetry2 = telemetry.clone();
        tokio::spawn(async move {
            match client.register(&node2).await {
                Ok(resp) => {
                    tracing::info!(share_id = %resp.share_id, "coord registered");
                    // 心跳循环
                    let _ = client.heartbeat_loop(telemetry2).await;
                }
                Err(e) => {
                    tracing::warn!(%e, "coord register failed (will retry on next boot)");
                }
            }
        });
        let _ = ctx.config;
        Ok(())
    }
}
