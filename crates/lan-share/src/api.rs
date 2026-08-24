//! 双协议 HTTP handlers：
//! - OpenAI 兼容：POST /v1/chat/completions（标准请求/响应，含 usage）
//! - Anthropic 兼容：POST /v1/messages（非流式 + SSE 流式）

use std::net::SocketAddr;

use axum::body::Body;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use crate::auth::AuthService;
use crate::backend::BackendEntry;
use crate::backend_store::BackendStore;
use crate::member::MemberRegistry;
use crate::quota::QuotaService;
use crate::usage::UsageService;

/// 共享 API 状态。
#[derive(Clone)]
pub struct ApiState {
    pub auth: AuthService,
    pub members: MemberRegistry,
    pub usage: UsageService,
    /// 按成员 token 配额（0/未设置 = 不限）。
    pub quota: QuotaService,
    /// 多后端注册表（DeepSeek/Kimi/智谱...按模型名路由；面板保存后热更新）。
    pub backends: std::sync::Arc<crate::registry::BackendRegistry>,
    /// 后端配置存储（backends.yaml，对齐 DeepSeek Harness 的 providers 配置）。
    pub backends_config: std::sync::Arc<BackendStore>,
    pub sharing: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

/// 提取 Bearer token。
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
}

fn sharing_on(state: &ApiState) -> bool {
    state.sharing.load(std::sync::atomic::Ordering::Relaxed)
}

fn unauthorized() -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({ "error": { "message": "unauthorized" } }))).into_response()
}

/// 客户端来源 IP（IPv4-mapped IPv6 转回 v4）。
fn client_ip(addr: SocketAddr) -> String {
    let ip = addr.ip();
    if let std::net::IpAddr::V6(v6) = ip {
        if let Some(v4) = v6.to_ipv4_mapped() {
            return v4.to_string();
        }
    }
    ip.to_string()
}

fn bad_request(msg: &str) -> Response {
    (StatusCode::BAD_REQUEST, Json(json!({ "error": { "message": msg } }))).into_response()
}

fn service_unavailable() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": { "message": "sharing paused" } }))).into_response()
}

fn internal_error(msg: &str) -> Response {
    (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": { "message": msg } }))).into_response()
}

/// 配额超限（OpenAI/Anthropic 通用：429，语义同 LiteLLM per-key quota / AgentGateway budget）。
fn quota_exceeded(limit: u64) -> Response {
    (
        StatusCode::TOO_MANY_REQUESTS,
        Json(json!({
            "error": {
                "message": format!("quota exceeded: limit {limit} tokens"),
                "type": "insufficient_quota",
                "code": "quota_exceeded",
                "quota_limit": limit,
            }
        })),
    )
        .into_response()
}

/// POST /v1/chat/completions（OpenAI 兼容）。
pub async fn chat_completions(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !sharing_on(&state) { return service_unavailable(); }
    let token = match bearer_token(&headers) { Some(t) => t, None => return unauthorized() };
    let session = match state.auth.verify(&token) { Some(s) => s, None => return unauthorized() };
    state.members.upsert(&session.machine_name, &session.display_name, "");
    // 配额检查（按成员累计用量，超限 429）
    let used = state.usage.get(&session.member_id).map(|u| u.total()).unwrap_or(0);
    if let Err(q) = state.quota.check(&session.member_id, used) {
        return quota_exceeded(q.limit);
    }
    // 按模型名路由到对应后端
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let backend = match state.backends.route(model) {
        Some((name, b)) => {
            tracing::debug!(model, backend = name, "routed");
            b
        }
        None => return bad_request(&format!("model not available: {model} (see /v1/models)")),
    };
    match backend.chat(&body).await {
        Ok(resp) => {
            let (pt, ct) = extract_openai_usage(&resp);
            state.usage.record(&session.member_id, model, pt, ct);
            (StatusCode::OK, Json(resp)).into_response()
        }
        Err(e) => internal_error(&format!("backend error: {e}")),
    }
}

/// POST /v1/messages（Anthropic 兼容）：body.stream=true 走 SSE，否则非流式。
pub async fn messages(
    State(state): State<ApiState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !sharing_on(&state) { return service_unavailable(); }
    let token = match bearer_token(&headers) { Some(t) => t, None => return unauthorized() };
    let session = match state.auth.verify(&token) { Some(s) => s, None => return unauthorized() };
    state.members.upsert(&session.machine_name, &session.display_name, "");
    // 配额检查（Anthropic 入口同样限制）
    let used = state.usage.get(&session.member_id).map(|u| u.total()).unwrap_or(0);
    if let Err(q) = state.quota.check(&session.member_id, used) {
        return quota_exceeded(q.limit);
    }
    let body: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return bad_request("invalid JSON body"),
    };
    let stream = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let openai_req = anthropic_to_openai(&body);
    let model = openai_req.get("model").and_then(|v| v.as_str()).unwrap_or("");
    let backend = match state.backends.route(model) {
        Some((name, b)) => {
            tracing::debug!(model, backend = name, "routed (anthropic)");
            b
        }
        None => return bad_request(&format!("model not available: {model} (see /v1/models)")),
    };
    match backend.chat(&openai_req).await {
        Ok(resp) => {
            let (pt, ct) = extract_openai_usage(&resp);
            state.usage.record(&session.member_id, model, pt, ct);
            if stream {
                let sse = anthropic_sse_stream(&resp);
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/event-stream")
                    .header("Cache-Control", "no-cache")
                    .body(Body::from(sse))
                    .unwrap()
            } else {
                let anthropic_resp = openai_to_anthropic(&resp);
                (StatusCode::OK, Json(anthropic_resp)).into_response()
            }
        }
        Err(e) => internal_error(&format!("backend error: {e}")),
    }
}

/// POST /auth/token（换 token，0.2.0 起免密：仅需 machineName，password 字段忽略）。
pub async fn auth_token(
    State(state): State<ApiState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Response {
    if !sharing_on(&state) { return service_unavailable(); }
    let machine = body.get("machineName").and_then(|v| v.as_str()).unwrap_or("");
    let display = body.get("displayName").and_then(|v| v.as_str()).unwrap_or("");
    if machine.is_empty() { return bad_request("machineName required"); }
    let ip = client_ip(addr);
    match state.auth.issue(machine, display, &ip) {
        Ok(session) => {
            state.members.upsert(machine, display, &ip);
            (StatusCode::OK, Json(json!({ "token": session.token, "expiresAt": session.expires_at }))).into_response()
        }
        Err(_) => (StatusCode::UNAUTHORIZED, Json(json!({ "error": { "message": "banned" } }))).into_response(),
    }
}

/// POST /auth/rename（改名）。
pub async fn auth_rename(
    State(state): State<ApiState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let token = match bearer_token(&headers) { Some(t) => t, None => return unauthorized() };
    let session = match state.auth.verify(&token) { Some(s) => s, None => return unauthorized() };
    let new_name = body.get("displayName").and_then(|v| v.as_str()).unwrap_or("");
    if new_name.is_empty() { return bad_request("displayName required"); }
    state.members.rename(&session.machine_name, new_name);
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
}

/// POST /api/control（管理：踢人/暂停/恢复；0.2.0 起无改密）。
pub async fn api_control(
    State(state): State<ApiState>,
    Json(body): Json<Value>,
) -> Response {
    let action = body.get("action").and_then(|v| v.as_str()).unwrap_or("");
    match action {
        "revoke" => {
            let member_id = body.get("memberId").and_then(|v| v.as_str()).unwrap_or("");
            let ip = body.get("ip").and_then(|v| v.as_str()).unwrap_or("");
            state.auth.revoke_member(member_id, ip);
            state.members.mark_offline(member_id);
            (StatusCode::OK, Json(json!({ "ok": true, "banned": true }))).into_response()
        }
        "unban" => {
            let member_id = body.get("memberId").and_then(|v| v.as_str()).unwrap_or("");
            if member_id.is_empty() { return bad_request("memberId required"); }
            let ip = body.get("ip").and_then(|v| v.as_str()).unwrap_or("");
            state.auth.unban(member_id, ip);
            (StatusCode::OK, Json(json!({ "ok": true, "banned": false }))).into_response()
        }
        "pause" => {
            state.sharing.store(false, std::sync::atomic::Ordering::Relaxed);
            (StatusCode::OK, Json(json!({ "ok": true, "sharing": false }))).into_response()
        }
        "resume" => {
            state.sharing.store(true, std::sync::atomic::Ordering::Relaxed);
            (StatusCode::OK, Json(json!({ "ok": true, "sharing": true }))).into_response()
        }
        _ => bad_request("unknown action"),
    }
}

/// GET /api/members（成员列表 + 用量）。
pub async fn api_members(State(state): State<ApiState>) -> Response {
    state.members.sweep();
    let members = state.members.all();
    let usage = state.usage.all();
    let rows: Vec<Value> = members.iter().map(|m| {
        let u = usage.iter().find(|u| u.member_id == m.member_id);
        json!({
            "memberId": m.member_id,
            "machineName": m.machine_name,
            "ip": m.ip,
            "gatewayId": m.gateway_id,
            "banned": state.auth.is_member_banned(&m.member_id),
            "displayName": m.display_name,
            "online": m.online,
            "joinedAt": m.joined_at,
            "lastSeen": m.last_seen,
            "usage": u.map(|u| json!({
                "promptTokens": u.prompt_tokens,
                "completionTokens": u.completion_tokens,
                "totalTokens": u.total(),
                "calls": u.calls,
                "modelTokens": u.model_tokens,
            })).unwrap_or(json!({})),
        })
    }).collect();
    (StatusCode::OK, Json(json!({ "members": rows }))).into_response()
}

/// GET /api/usage/export（账单 CSV 导出，text/csv 附件）。
pub async fn api_usage_export(State(state): State<ApiState>) -> Response {
    let csv = state.usage.export_csv();
    Response::builder()
        .status(StatusCode::OK)
        .header("Content-Type", "text/csv; charset=utf-8")
        .header("Content-Disposition", "attachment; filename=\"usage.csv\"")
        .body(Body::from(csv))
        .expect("valid csv response")
}

/// POST /api/quota（设置成员配额：{memberId, quota}；quota=0 解除限制）。
pub async fn api_quota_set(
    State(state): State<ApiState>,
    Json(body): Json<Value>,
) -> Response {
    let member_id = body.get("memberId").and_then(|v| v.as_str()).unwrap_or("");
    let quota = body.get("quota").and_then(|v| v.as_u64()).unwrap_or(0);
    if member_id.is_empty() {
        return bad_request("memberId required");
    }
    state.quota.set(member_id, quota);
    (StatusCode::OK, Json(json!({ "ok": true, "memberId": member_id, "quota": quota }))).into_response()
}

/// GET /api/quota（全部成员配额）。
pub async fn api_quota_list(State(state): State<ApiState>) -> Response {
    let rows: Vec<Value> = state
        .quota
        .all()
        .iter()
        .map(|q| json!({ "memberId": q.member_id, "quota": q.limit }))
        .collect();
    (StatusCode::OK, Json(json!({ "quotas": rows }))).into_response()
}


// ------------------ 后端配置（模型设置，对齐 DeepSeek Harness 配置方式） ------------------

/// 配置变更后重建注册表并热替换（无需重启）。
fn apply_backend_config(state: &ApiState) -> Result<(), String> {
    let entries = state.backends_config.list();
    let built = entries
        .iter()
        .map(crate::registry::backend_from_entry)
        .collect::<anyhow::Result<Vec<_>>>()
        .map_err(|e| format!("rebuild backends: {e}"))?;
    state.backends.replace_all(built);
    Ok(())
}

/// GET /api/backends（面板「模型设置」；密钥只回传掩码/来源，不回明文）。
pub async fn api_backends_list(State(state): State<ApiState>) -> Response {
    let registered = state.backends.backend_names();
    let rows: Vec<Value> = state.backends_config.list().iter().map(|e| {
        let models = e.effective_models();
        json!({
            "id": e.backend_id(),
            "provider": e.provider,
            "model": models.first().cloned().unwrap_or_default(),
            "models": models,
            "baseUrl": e.base_url.clone().unwrap_or_default(),
            "keySource": e.key_source(),
            "maskedKey": e.masked_key(),
            "registered": registered.contains(&e.backend_id()),
        })
    }).collect();
    (StatusCode::OK, Json(json!({ "backends": rows }))).into_response()
}

/// POST /api/backends（新增/更新；直填 key 或环境变量引用，保存即热生效）。
pub async fn api_backends_set(
    State(state): State<ApiState>,
    Json(body): Json<Value>,
) -> Response {
    let f = |k: &str| -> Option<String> {
        body.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };
    let provider = f("provider").unwrap_or_default();
    if provider.is_empty() { return bad_request("provider required"); }
    let models: Vec<String> = body.get("models")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|m| m.as_str().map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    // 兼容旧客户端：仅传单值 model → 视为单模型列表
    let model_single = f("model");
    let models = if models.is_empty() { model_single.map(|m| vec![m]).unwrap_or_default() } else { models };
    // 去重（保持顺序）
    let mut seen = std::collections::HashSet::new();
    let models = models.into_iter().filter(|m| seen.insert(m.clone())).collect::<Vec<_>>();
    let mut entry = BackendEntry {
        provider,
        id: f("id"),
        api_key: f("apiKey"),
        api_key_env: f("apiKeyEnv"),
        model: None,
        models,
        base_url: f("baseUrl"),
    };
    // 未提供任何密钥字段（如只改模型/地址）→ 保留原密钥配置
    let has_key_field = entry.api_key.is_some() || entry.api_key_env.is_some();
    if !has_key_field {
        if let Some(old) = state.backends_config.list().iter().find(|e| e.backend_id() == entry.backend_id()) {
            entry.api_key = old.api_key.clone();
            entry.api_key_env = old.api_key_env.clone();
        }
    }
    // 先校验（custom 需要 base_url/model；官方配置无碍）再落盘
    if let Err(e) = crate::registry::backend_from_entry(&entry) {
        return bad_request(&format!("invalid backend: {e}"));
    }
    state.backends_config.upsert(entry.clone());
    if let Err(e) = state.backends_config.save() {
        return internal_error(&format!("save backends.yaml: {e}"));
    }
    if let Err(e) = apply_backend_config(&state) {
        return internal_error(&e);
    }
    (StatusCode::OK, Json(json!({
        "ok": true,
        "backend": { "id": entry.backend_id(), "provider": entry.provider },
    }))).into_response()
}

/// DELETE /api/backends/{id}（移除后端；保存即热生效）。
pub async fn api_backends_delete(
    State(state): State<ApiState>,
    Path(id): Path<String>,
) -> Response {
    if !state.backends_config.remove(&id) {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": { "message": format!("backend not found: {id}") } }))).into_response();
    }
    if let Err(e) = state.backends_config.save() {
        return internal_error(&format!("save backends.yaml: {e}"));
    }
    if let Err(e) = apply_backend_config(&state) {
        return internal_error(&e);
    }
    (StatusCode::OK, Json(json!({ "ok": true, "removed": id }))).into_response()
}
/// 从 OpenAI 响应提取 usage。
pub fn extract_openai_usage(resp: &Value) -> (u64, u64) {
    let usage = resp.get("usage").cloned().unwrap_or(json!({}));
    let pt = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let ct = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    (pt, ct)
}

/// Anthropic 请求 → OpenAI 请求。
pub fn anthropic_to_openai(body: &Value) -> Value {
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("default");
    let max_tokens = body.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(4096);
    let system = body.get("system").and_then(|v| v.as_str()).unwrap_or("");
    let messages = body.get("messages").cloned().unwrap_or(json!([]));
    let mut openai_messages: Vec<Value> = Vec::new();
    if !system.is_empty() {
        openai_messages.push(json!({ "role": "system", "content": system }));
    }
    if let Some(arr) = messages.as_array() {
        for m in arr {
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = m.get("content").cloned().unwrap_or(json!(""));
            let text = match &content {
                Value::String(s) => s.clone(),
                Value::Array(arr) => arr.iter()
                    .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                    .collect::<Vec<_>>().join("\n"),
                _ => String::new(),
            };
            openai_messages.push(json!({ "role": role, "content": text }));
        }
    }
    json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": openai_messages,
    })
}

/// OpenAI 响应 → Anthropic 响应。
pub fn openai_to_anthropic(resp: &Value) -> Value {
    let model = resp.get("model").and_then(|v| v.as_str()).unwrap_or("default");
    let content = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let (pt, ct) = extract_openai_usage(resp);
    json!({
        "id": "msg_mock_0001",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": [{ "type": "text", "text": content }],
        "stop_reason": "end_turn",
        "stop_sequence": null,
        "usage": {
            "input_tokens": pt,
            "output_tokens": ct,
        },
    })
}

/// Anthropic SSE 流事件。
pub fn anthropic_sse_stream(resp: &Value) -> String {
    let model = resp.get("model").and_then(|v| v.as_str()).unwrap_or("default");
    let content = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("message"))
        .and_then(|m| m.get("content"))
        .and_then(|c| c.as_str())
        .unwrap_or("");
    let (pt, ct) = extract_openai_usage(resp);
    let mut out = String::new();
    out.push_str(&format!("event: message_start\ndata: {}\n\n", json!({
        "type": "message_start",
        "message": {
            "id": "msg_mock_0001",
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": [],
            "usage": { "input_tokens": pt, "output_tokens": 0 },
        },
    })));
    out.push_str(&format!("event: content_block_start\ndata: {}\n\n", json!({
        "type": "content_block_start",
        "index": 0,
        "content_block": { "type": "text", "text": "" },
    })));
    out.push_str(&format!("event: content_block_delta\ndata: {}\n\n", json!({
        "type": "content_block_delta",
        "index": 0,
        "delta": { "type": "text_delta", "text": content },
    })));
    out.push_str(&format!("event: content_block_stop\ndata: {}\n\n", json!({
        "type": "content_block_stop",
        "index": 0,
    })));
    out.push_str(&format!("event: message_delta\ndata: {}\n\n", json!({
        "type": "message_delta",
        "delta": { "stop_reason": "end_turn", "stop_sequence": null },
        "usage": { "output_tokens": ct },
    })));
    out.push_str(&format!("event: message_stop\ndata: {}\n\n", json!({
        "type": "message_stop",
    })));
    out
}

/// GET /v1/models（OpenAI 格式模型目录）。
pub async fn models_openai(State(state): State<ApiState>) -> Response {
    let resp = state.backends.openai_models_response();
    (StatusCode::OK, Json(resp)).into_response()
}

/// GET /v1/models（Anthropic 格式模型目录）。
pub async fn models_anthropic(State(state): State<ApiState>) -> Response {
    let resp = state.backends.anthropic_models_response();
    (StatusCode::OK, Json(resp)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn anthropic_to_openai_converts() {
        let body = json!({
            "model": "claude-3",
            "max_tokens": 1024,
            "system": "be brief",
            "messages": [
                { "role": "user", "content": [{ "type": "text", "text": "hi" }] },
            ],
        });
        let openai = anthropic_to_openai(&body);
        assert_eq!(openai["messages"][0]["role"], "system");
        assert_eq!(openai["messages"][1]["content"], "hi");
    }

    #[test]
    fn openai_to_anthropic_converts() {
        let resp = json!({
            "model": "mock-7b",
            "choices": [{ "message": { "content": "hello" } }],
            "usage": { "prompt_tokens": 5, "completion_tokens": 3 },
        });
        let a = openai_to_anthropic(&resp);
        assert_eq!(a["content"][0]["text"], "hello");
        assert_eq!(a["usage"]["output_tokens"], 3);
    }

    #[test]
    fn sse_has_all_events() {
        let resp = json!({ "model": "m", "choices": [{ "message": { "content": "x" } }], "usage": { "prompt_tokens": 1, "completion_tokens": 1 } });
        let sse = anthropic_sse_stream(&resp);
        assert!(sse.contains("message_start"));
        assert!(sse.contains("content_block_delta"));
        assert!(sse.contains("message_stop"));
        assert!(sse.contains("text_delta"));
    }
}