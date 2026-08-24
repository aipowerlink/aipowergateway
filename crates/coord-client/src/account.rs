//! 账号（PS4 消费侧：用户名+邮箱+动态密码）。

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// 账号客户端配置。
#[derive(Debug, Clone)]
pub struct AccountClientConfig {
    pub base_url: String,
    pub timeout_s: u64,
}

impl Default for AccountClientConfig {
    fn default() -> Self {
        Self { base_url: "http://127.0.0.1:8080".into(), timeout_s: 10 }
    }
}

/// 登录响应。
#[derive(Debug, Clone, Deserialize)]
pub struct LoginResponse {
    pub token: String,
    #[serde(default)]
    pub device_bound: bool,
}

/// 账号客户端。
#[derive(Debug, Clone)]
pub struct AccountClient {
    cfg: AccountClientConfig,
    http: reqwest::Client,
    session: std::sync::Arc<std::sync::Mutex<Option<String>>>,
}

impl AccountClient {
    pub fn new(cfg: AccountClientConfig) -> Self {
        Self {
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(cfg.timeout_s))
                .build()
                .expect("build http client"),
            session: std::sync::Arc::new(std::sync::Mutex::new(None)),
            cfg,
        }
    }

    /// 注册账号（发动态密码到邮箱）。
    pub async fn register(&self, username: &str, email: &str) -> Result<()> {
        let url = format!("{}/v1/auth/register", self.cfg.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "username": username, "email": email }))
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Relay { status: status.as_u16(), body });
        }
        Ok(())
    }

    /// 登录（动态密码 → 会话令牌）。
    pub async fn login(&self, username: &str, otp: &str) -> Result<LoginResponse> {
        let url = format!("{}/v1/auth/login", self.cfg.base_url);
        let resp = self
            .http
            .post(&url)
            .json(&serde_json::json!({ "username": username, "otp": otp }))
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Relay { status: status.as_u16(), body });
        }
        let parsed: LoginResponse =
            serde_json::from_str(&body).map_err(|_| Error::MissingField("login".into()))?;
        *self.session.lock().unwrap() = Some(parsed.token.clone());
        Ok(parsed)
    }

    /// 绑定设备（账号 1:1 绑定）。
    pub async fn bind_device(&self, device_token: &str) -> Result<()> {
        let sess = self.session.lock().unwrap().clone().ok_or(Error::NotRegistered)?;
        let url = format!("{}/v1/auth/bind", self.cfg.base_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&sess)
            .json(&serde_json::json!({ "device_token": device_token }))
            .send()
            .await
            .map_err(|e| Error::Unreachable(e.to_string()))?;
        let status = resp.status();
        let body = resp.text().await?;
        if !status.is_success() {
            return Err(Error::Relay { status: status.as_u16(), body });
        }
        Ok(())
    }
}

// 保留序列化导入（结构体未直接用时可去掉警告）
#[allow(unused_imports)]
use Serialize as _;
