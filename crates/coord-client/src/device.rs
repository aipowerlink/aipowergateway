//! 设备注册/心跳/解析（PS1 消费侧）。

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 协调服务器地址配置。
#[derive(Debug, Clone)]
pub struct DeviceClientConfig {
    /// 协调服务器 base URL，如 http://127.0.0.1:8080
    pub base_url: String,
    /// 心跳间隔（秒），默认 60
    pub heartbeat_interval_s: u64,
    /// 超时（秒）
    pub timeout_s: u64,
}

impl Default for DeviceClientConfig {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:8080".into(),
            heartbeat_interval_s: 60,
            timeout_s: 10,
        }
    }
}

/// 节点信息（注册请求）。
#[derive(Debug, Clone, Serialize)]
pub struct NodeInfo {
    pub name: String,
    pub platform: String,
    pub version: String,
    pub public_ip: String,
    pub api_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub region_hint: Option<String>,
}

/// 注册响应。
#[derive(Debug, Clone, Deserialize)]
pub struct RegisterResponse {
    pub device_token: String,
    pub share_id: String,
    #[serde(default)]
    pub heartbeat_interval_s: u64,
}

/// 心跳遥测（opt-in，默认关闭）。
#[derive(Debug, Clone, Serialize, Default)]
pub struct HeartbeatTelemetry {
    pub enabled: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub platform: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub version: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub region_hint: String,
}

/// 解析结果（shareId → 节点）。
#[derive(Debug, Clone, Deserialize)]
pub struct ResolveResult {
    pub name: String,
    pub public_ip: String,
    pub api_port: u16,
    pub online: bool,
    #[serde(default)]
    pub fingerprint: String,
}

/// 设备客户端。
#[derive(Debug, Clone)]
pub struct DeviceClient {
    cfg: DeviceClientConfig,
    http: reqwest::Client,
    /// 注册后持有（内存态；持久化由上层 store 负责）
    device_token: std::sync::Arc<std::sync::Mutex<Option<String>>>,
    share_id: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl DeviceClient {
    pub fn new(cfg: DeviceClientConfig) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(cfg.timeout_s))
                .build()
                .expect("build http client"),
            device_token: std::sync::Arc::new(std::sync::Mutex::new(None)),
            share_id: std::sync::Arc::new(std::sync::Mutex::new(None)),
            cfg,
        }
    }

    /// 注册设备（首次），保存 device_token + share_id。
    pub async fn register(&self, info: &NodeInfo) -> Result<RegisterResponse> {
        let url = format!("{}/v1/device/register", self.cfg.base_url);
        let resp = self
            .http
            .post(&url)
            .json(info)
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Relay { status: status.as_u16(), body });
        }
        let parsed: RegisterResponse = serde_json::from_str(&body)
            .map_err(|_| Error::MissingField("register response".into()))?;
        *self.device_token.lock().unwrap() = Some(parsed.device_token.clone());
        *self.share_id.lock().unwrap() = Some(parsed.share_id.clone());
        Ok(parsed)
    }

    /// 心跳 + 可选遥测。未注册返回 NotRegistered。
    pub async fn heartbeat(&self, telemetry: &HeartbeatTelemetry) -> Result<()> {
        let token = self.device_token();
        let url = format!("{}/v1/device/heartbeat", self.cfg.base_url);
        let body = serde_json::json!({ "telemetry": telemetry });
        let resp = self
            .http
            .post(&url)
            .header("X-Device-Token", token)
            .json(&body)
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = resp.status();
        let b = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Relay { status: status.as_u16(), body: b });
        }
        Ok(())
    }

    /// 心跳循环（上层 tokio 任务调用；每间隔发送一次，直到取消）。
    pub async fn heartbeat_loop(&self, telemetry: HeartbeatTelemetry) -> Result<()> {
        loop {
            if self.heartbeat(&telemetry).await.is_err() {
                tracing::warn!("coord heartbeat failed (will retry)");
            }
            tokio::time::sleep(std::time::Duration::from_secs(self.cfg.heartbeat_interval_s)).await;
        }
    }

    /// 解析 shareId（Deep Link 找组长）。
    pub async fn resolve(&self, share_id: &str) -> Result<ResolveResult> {
        let url = format!("{}/v1/resolve?shareId={}", self.cfg.base_url, share_id);
        let resp = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Relay { status: status.as_u16(), body });
        }
        serde_json::from_str(&body).map_err(|_| Error::MissingField("resolve".into()))
    }

    pub fn device_token(&self) -> String {
        self.device_token
            .lock()
            .unwrap()
            .clone()
            .ok_or(Error::NotRegistered)
            .unwrap_or_default()
    }

    pub fn share_id(&self) -> Option<String> {
        self.share_id.lock().unwrap().clone()
    }
}
