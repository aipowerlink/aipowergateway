//! lan-share-client：密码 → Bearer token、双协议 API 调用（OpenAI + Anthropic）。

use std::sync::Arc;

use serde_json::{json, Value};

use aipg_runtime::RuntimeResult;

/// 组员接入会话。
#[derive(Debug, Clone)]
pub struct Session {
    /// Bearer token。
    pub token: String,
    /// 组长 API base（http://ip:port）。
    pub base: String,
    /// 组长名。
    pub leader_name: String,
    /// 过期时间（unix 秒）。
    pub expires_at: u64,
    /// 是否已失效（被踢/改密）。
    pub revoked: bool,
}

impl Session {
    /// 构造带 Authorization 的请求。
    pub fn auth_header(&self) -> String {
        format!("Bearer {}", self.token)
    }
}

/// 共享客户端配置。
#[derive(Debug, Clone)]
pub struct ShareClientConfig {
    /// 请求超时（秒）。
    pub timeout_secs: u64,
}

impl Default for ShareClientConfig {
    fn default() -> Self {
        Self { timeout_secs: 30 }
    }
}

/// 共享客户端：接入 + 调用。
#[derive(Clone)]
pub struct ShareClient {
    #[allow(dead_code)] // cfg.timeout 用于 reqwest 构造
    cfg: ShareClientConfig,
    http: reqwest::Client,
    session: Option<Arc<std::sync::RwLock<Session>>>,
}

impl ShareClient {
    pub fn new(cfg: ShareClientConfig) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(cfg.timeout_secs.max(1)))
            .build()
            .unwrap_or_default();
        Self { cfg, http, session: None }
    }

    /// 是否已接入。
    pub fn is_connected(&self) -> bool {
        self.session.is_some()
    }

    /// 密码接入：换 Bearer token。
    pub async fn connect(&mut self, base: &str, leader_name: &str, password: &str, machine_name: &str, display_name: &str) -> RuntimeResult<Session> {
        let url = format!("{}/auth/token", base.trim_end_matches('/'));
        let body = json!({
            "password": password,
            "machineName": machine_name,
            "displayName": display_name,
        });
        let resp = self.http.post(&url).json(&body).send().await
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("connect: {e}")))?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(aipg_runtime::RuntimeError::Auth(if status == reqwest::StatusCode::UNAUTHORIZED {
                "wrong password or banned".to_string()
            } else {
                format!("connect failed {}: {}", status, truncate(&text, 200))
            }));
        }
        let v: Value = resp.json().await.map_err(|e| aipg_runtime::RuntimeError::Other(format!("connect json: {e}")))?;
        let token = v.get("token").and_then(|t| t.as_str()).unwrap_or("").to_string();
        let expires = v.get("expiresAt").and_then(|t| t.as_u64()).unwrap_or(0);
        if token.is_empty() {
            return Err(aipg_runtime::RuntimeError::Auth("no token in response".to_string()));
        }
        let session = Session {
            token,
            base: base.trim_end_matches('/').to_string(),
            leader_name: leader_name.to_string(),
            expires_at: expires,
            revoked: false,
        };
        self.session = Some(Arc::new(std::sync::RwLock::new(session.clone())));
        Ok(session)
    }

    /// 当前会话（未接入返回错误）。
    fn current(&self) -> RuntimeResult<Arc<std::sync::RwLock<Session>>> {
        self.session.clone().ok_or_else(|| aipg_runtime::RuntimeError::Auth("not connected".to_string()))
    }

    /// OpenAI 兼容调用。
    pub async fn chat_openai(&self, model: &str, messages: &Value) -> RuntimeResult<Value> {
        let s = self.current()?;
        let base = { s.read().unwrap().base.clone() };
        let url = format!("{}/v1/chat/completions", base);
        let body = json!({ "model": model, "messages": messages });
        self.call_with_session(&s, &url, &body).await
    }

    /// Anthropic 兼容调用（非流式）。
    pub async fn chat_anthropic(&self, model: &str, system: &str, messages: &Value) -> RuntimeResult<Value> {
        let s = self.current()?;
        let base = { s.read().unwrap().base.clone() };
        let url = format!("{}/v1/messages", base);
        let body = json!({
            "model": model,
            "max_tokens": 4096,
            "system": system,
            "messages": messages,
        });
        self.call_with_session(&s, &url, &body).await
    }

    /// Anthropic SSE 流式调用（返回原始 SSE 文本）。
    pub async fn chat_anthropic_stream(&self, model: &str, system: &str, messages: &Value) -> RuntimeResult<String> {
        let s = self.current()?;
        let base = { s.read().unwrap().base.clone() };
        let url = format!("{}/v1/messages", base);
        let body = json!({
            "model": model,
            "max_tokens": 4096,
            "stream": true,
            "system": system,
            "messages": messages,
        });
        let token = { s.read().unwrap().token.clone() };
        let resp = self.http.post(&url).header("Authorization", format!("Bearer {token}")).json(&body).send().await
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("stream: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            s.write().unwrap().revoked = true;
            return Err(aipg_runtime::RuntimeError::Auth("access revoked (401)".to_string()));
        }
        if !status.is_success() {
            return Err(aipg_runtime::RuntimeError::Other(format!("stream failed: {status}")));
        }
        resp.text().await.map_err(|e| aipg_runtime::RuntimeError::Other(format!("stream body: {e}")))
    }

    /// 改名同步到组长。
    pub async fn rename(&self, new_display: &str) -> RuntimeResult<()> {
        let s = self.current()?;
        let base = { s.read().unwrap().base.clone() };
        let token = { s.read().unwrap().token.clone() };
        let url = format!("{}/auth/rename", base);
        let resp = self.http.post(&url).header("Authorization", format!("Bearer {token}")).json(&json!({ "displayName": new_display })).send().await
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("rename: {e}")))?;
        if resp.status().is_success() { Ok(()) } else { Err(aipg_runtime::RuntimeError::Other("rename failed".to_string())) }
    }

    /// 查询组长模型目录（/v1/models）。
    pub async fn list_models(&self) -> RuntimeResult<Vec<String>> {
        let s = self.current()?;
        let base = { s.read().unwrap().base.clone() };
        let token = { s.read().unwrap().token.clone() };
        let url = format!("{}/v1/models", base);
        let resp = self.http.get(&url).header("Authorization", format!("Bearer {token}")).send().await
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("models: {e}")))?;
        if !resp.status().is_success() {
            return Err(aipg_runtime::RuntimeError::Other(format!("models failed: {}", resp.status())));
        }
        let v: Value = resp.json().await.map_err(|e| aipg_runtime::RuntimeError::Other(format!("models json: {e}")))?;
        let names = v.get("data").and_then(|d| d.as_array()).map(|arr| {
            arr.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.to_string())).collect::<Vec<_>>()
        }).unwrap_or_default();
        Ok(names)
    }

    /// 标记会话失效（被踢/改密）。
    pub fn mark_revoked(&self) {
        if let Some(s) = &self.session {
            s.write().unwrap().revoked = true;
        }
    }

    /// 断开（清除会话）。
    pub fn disconnect(&mut self) {
        self.session = None;
    }

    /// 内部：带会话调用的通用逻辑（401 → revoked）。
    async fn call_with_session(&self, s: &Arc<std::sync::RwLock<Session>>, url: &str, body: &Value) -> RuntimeResult<Value> {
        let token = { s.read().unwrap().token.clone() };
        let resp = self.http.post(url).header("Authorization", format!("Bearer {token}")).json(body).send().await
            .map_err(|e| aipg_runtime::RuntimeError::Other(format!("call: {e}")))?;
        let status = resp.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            s.write().unwrap().revoked = true;
            return Err(aipg_runtime::RuntimeError::Auth("access revoked (401)".to_string()));
        }
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(aipg_runtime::RuntimeError::Other(format!("call failed {}: {}", status, truncate(&text, 300))));
        }
        resp.json().await.map_err(|e| aipg_runtime::RuntimeError::Other(format!("call json: {e}")))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { s.chars().take(max).collect() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_auth_header() {
        let s = Session { token: "abc".into(), base: "http://x:1".into(), leader_name: "l".into(), expires_at: 0, revoked: false };
        assert_eq!(s.auth_header(), "Bearer abc");
    }

    #[test]
    fn not_connected_error() {
        let c = ShareClient::new(ShareClientConfig::default());
        assert!(!c.is_connected());
    }
}