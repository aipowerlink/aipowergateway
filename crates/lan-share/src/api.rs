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
    /// 连接测试状态表（backend_id → {ok, latencyMs?, error?}；进程内存，随测试刷新）。
    /// 对应 DeepSeek Harness 的连接状态指示：配置正确 → 绿色图标。
    pub test_status: std::sync::Arc<std::sync::RwLock<std::collections::HashMap<String, serde_json::Value>>>,
    /// 监听端口（接入信息展示用）。
    pub port: u16,
    /// 绑定地址（127.0.0.1 = 仅本机；0.0.0.0 = 局域网共享）。
    pub bind: std::net::IpAddr,
    /// gateway 间共享通道端口（成员 gateway 接入）。
    pub share_port: u16,
}

/// 提取访问令牌：优先 Authorization: Bearer（Claude Code CLI 方式），
/// 其次 x-api-key（Anthropic 标准 header，cc-switch 界面测试/获取模型常用）。
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    if let Some(t) = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.strip_prefix("Bearer "))
        .map(|s| s.trim().to_string())
    {
        return Some(t);
    }
    headers
        .get("x-api-key")
        .and_then(|v| v.to_str().ok())
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

/// 记录一次连接测试结果（DeepSeek Harness 式状态点：ok → 绿）。
fn record_test_status(state: &ApiState, id: &str, ok: bool, latency_ms: Option<u64>, error: Option<String>) {
    let mut map = state.test_status.write().expect("test_status lock");
    if ok {
        map.insert(id.to_string(), json!({ "ok": true, "latencyMs": latency_ms }));
    } else {
        map.insert(id.to_string(), json!({ "ok": false, "error": error }));
    }
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
    // 客户端请求流式时，上游强制非流式（backend 只解析 JSON），由网关组装 OpenAI SSE 回放。
    let stream_req = body.get("stream").and_then(|v| v.as_bool()).unwrap_or(false);
    let mut fwd = body.clone();
    if stream_req {
        fwd["stream"] = json!(false);
    }
    match backend.chat(&fwd).await {
        Ok(resp) => {
            let (pt, ct) = extract_openai_usage(&resp);
            state.usage.record(&session.member_id, model, pt, ct);
            if stream_req {
                let sse = openai_sse_stream(&resp);
                Response::builder()
                    .status(StatusCode::OK)
                    .header("Content-Type", "text/event-stream")
                    .header("Cache-Control", "no-cache")
                    .body(Body::from(sse))
                    .unwrap()
            } else {
                (StatusCode::OK, Json(resp)).into_response()
            }
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
/// 幂等：同机器已有有效 token 直接复用（key 稳定）；body 带 "force": true 时显式轮换（页面「重新换取」）。
pub async fn auth_token(
    State(state): State<ApiState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Json(body): Json<Value>,
) -> Response {
    if !sharing_on(&state) { return service_unavailable(); }
    let machine = body.get("machineName").and_then(|v| v.as_str()).unwrap_or("");
    let display = body.get("displayName").and_then(|v| v.as_str()).unwrap_or("");
    let force = body.get("force").and_then(|v| v.as_bool()).unwrap_or(false);
    if machine.is_empty() { return bad_request("machineName required"); }
    let ip = client_ip(addr);
    let issued = if force {
        state.auth.rotate(machine, display, &ip)
    } else {
        state.auth.issue(machine, display, &ip)
    };
    match issued {
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
        "rename" => {
            let member_id = body.get("memberId").and_then(|v| v.as_str()).unwrap_or("");
            let display_name = body.get("displayName").and_then(|v| v.as_str()).unwrap_or("");
            if member_id.is_empty() || display_name.is_empty() { return bad_request("memberId and displayName required"); }
            if !state.members.rename(member_id, display_name) { return bad_request("member not found"); }
            (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
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

/// 判断来源 IP 是否本机回环地址。
fn is_loopback_ip(s: &str) -> bool {
    match s.parse::<std::net::IpAddr>() {
        Ok(ip) => ip.is_loopback(),
        Err(_) => s == "127.0.0.1" || s == "::1" || s.eq_ignore_ascii_case("localhost"),
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
            "isLocal": is_loopback_ip(&m.ip),
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
    let statuses = state.test_status.read().expect("test_status lock");
    let rows: Vec<Value> = state.backends_config.list().iter().map(|e| {
        let models = e.effective_models();
        let id = e.backend_id();
        let test_status = match statuses.get(&id) {
            Some(v) if v.get("ok").and_then(|o| o.as_bool()) == Some(true) =>
                json!({ "status": "ok", "latencyMs": v.get("latencyMs").and_then(|l| l.as_u64()) }),
            Some(v) => json!({ "status": "fail", "error": v.get("error").and_then(|e| e.as_str()) }),
            None => json!({ "status": "untested" }),
        };
        json!({
            "id": e.backend_id(),
            "provider": e.provider,
            "model": models.first().cloned().unwrap_or_default(),
            "models": models,
            "baseUrl": e.base_url.clone().unwrap_or_default(),
            "keySource": e.key_source(),
            "maskedKey": e.masked_key(),
            "registered": registered.contains(&id),
            "testStatus": test_status,
        })
    }).collect();
    (StatusCode::OK, Json(json!({ "backends": rows }))).into_response()
}

/// 从请求体解析后端条目（共用：保存 / 测试）。
/// 模型支持 models 数组；兼容旧客户端仅传单值 model。key 字段可为空（测试时继承已保存密钥）。
fn entry_from_body(body: &Value) -> Result<BackendEntry, String> {
    let f = |k: &str| -> Option<String> {
        body.get(k).and_then(|v| v.as_str()).map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
    };
    let provider = f("provider").ok_or_else(|| "provider required".to_string())?;
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
    Ok(BackendEntry {
        provider,
        id: f("id"),
        api_key: f("apiKey"),
        api_key_env: f("apiKeyEnv"),
        model: None,
        models,
        base_url: f("baseUrl"),
    })
}

/// POST /api/backends（新增/更新；直填 key 或环境变量引用，保存即热生效）。
pub async fn api_backends_set(
    State(state): State<ApiState>,
    Json(body): Json<Value>,
) -> Response {
    let mut entry = match entry_from_body(&body) {
        Ok(e) => e,
        Err(msg) => return bad_request(&msg),
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
    // 保存后自动连接测试（cc-switch/DeepSeek Harness 式：配置后立即探活，绿色=配置正确）
    let auto_id = entry.backend_id();
    let auto_entry = entry.clone();
    let auto_state = state.clone();
    tokio::spawn(async move {
        match test_target(&auto_entry) {
            Ok(Some(t)) => match probe(&t).await {
                Ok(out) => {
                    record_test_status(&auto_state, &auto_id, true, Some(out.latency_ms), None);
                    // 模型添加后自动获取其具体模型列表（cc-switch 式）：未显式配置模型 → 用服务器返回的真实清单
                    if !out.models.is_empty() && auto_entry.models.is_empty() {
                        let mut e2 = auto_entry.clone();
                        e2.models = out.models;
                        // 用户可能在保存后立刻又改了配置，仅当 backend_id 未变化才写回
                        if auto_state.backends_config.list().iter().any(|x| x.backend_id() == auto_id) {
                            auto_state.backends_config.upsert(e2);
                            let _ = auto_state.backends_config.save();
                            let _ = apply_backend_config(&auto_state);
                        }
                    }
                }
                Err(msg) => record_test_status(&auto_state, &auto_id, false, None, Some(msg)),
            },
            Ok(None) => record_test_status(&auto_state, &auto_id, true, Some(0), None),
            Err(msg) => record_test_status(&auto_state, &auto_id, false, None, Some(msg)),
        }
    });
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
    state.test_status.write().expect("test_status lock").remove(&id);
    (StatusCode::OK, Json(json!({ "ok": true, "removed": id }))).into_response()
}

/// 测试目标（探活 GET {base}/models 所需）。
struct TestTarget {
    url: String,
    key: String,
}

/// 解析测试目标：mock → Ok(None)（本地直通）；否则校验密钥与 base_url。
fn test_target(entry: &BackendEntry) -> Result<Option<TestTarget>, String> {
    use crate::backend::Provider;
    let provider = Provider::from_str(&entry.provider);
    if provider == Provider::Mock {
        return Ok(None);
    }
    // 先校验端点（custom 必填 base_url），再校验密钥
    let base = entry.base_url.clone()
        .or_else(|| provider.base_url().map(|s| s.to_string()))
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| "base_url required for custom providers".to_string())?;
    let key = entry.resolve_api_key()
        .ok_or_else(|| "no API key — fill it in or set an env var reference".to_string())?;
    Ok(Some(TestTarget { url: format!("{}/models", base.trim_end_matches('/')), key }))
}

fn api_truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max { s.to_string() } else { s.chars().take(max).collect::<String>() + "…" }
}

/// 探活结果：延迟 + 该端点返回的具体模型列表（cc-switch「获取模型」/「添加后自动获取列表」）。
struct ProbeOutcome {
    latency_ms: u64,
    models: Vec<String>,
}

/// OpenAI 兼容 /models 响应 → 模型 ID 列表（data[].id；去重、去空、上限 200）。
fn parse_models_from_response(v: &Value) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let ids = v.get("data")
        .and_then(|d| d.as_array())
        .map(|arr| arr.iter().filter_map(|m| m.get("id").and_then(|i| i.as_str()).map(|s| s.trim().to_string())).filter(|s| !s.is_empty()).collect::<Vec<_>>())
        .unwrap_or_default();
    ids.into_iter().filter(|m| seen.insert(m.clone())).take(200).collect()
}

/// 单次探活：GET {base_url}/models（5s 超时），解析具体模型列表。Ok(ProbeOutcome) / Err(可读错误)。
async fn probe(target: &TestTarget) -> Result<ProbeOutcome, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| format!("build client: {e}"))?;
    let start = std::time::Instant::now();
    let resp = client.get(&target.url)
        .header("Authorization", format!("Bearer {}", target.key))
        .send().await
        .map_err(|e| api_truncate(&format!("连接失败（connection failed: {e}）"), 160))?;
    let status = resp.status();
    let latency_ms = start.elapsed().as_millis() as u64;
    if status.is_success() {
        let models = resp.json::<Value>().await
            .map(|v| parse_models_from_response(&v))
            .unwrap_or_default();
        return Ok(ProbeOutcome { latency_ms, models });
    }
    let code = status.as_u16();
    let reason = status.canonical_reason().unwrap_or("error");
    let body_text = resp.text().await.unwrap_or_default();
    let detail = api_truncate(&body_text, 120);
    // 常见鉴权错误给出可读提示
    let hint = match code {
        401 => "（API 密钥无效或已过期）",
        403 => "（无权限，请检查密钥/账号）",
        429 => "（请求过快或余额不足）",
        _ => "",
    };
    Err(api_truncate(&format!("HTTP {code} {reason}{hint}: {detail}"), 220))
}

/// 执行连通性测试：GET {base_url}/models（cc-switch 式测试连接，验证密钥与端点）。
/// 返回 200 {ok:true, latencyMs} 或 {ok:false, error}；结果记入测试状态表（绿/红状态点）。
/// 错误不落盘、不影响配置。
pub async fn api_backends_test(
    State(state): State<ApiState>,
    Json(body): Json<Value>,
) -> Response {
    let mut entry = match entry_from_body(&body) {
        Ok(e) => e,
        Err(msg) => return bad_request(&msg),
    };
    // 未带密钥字段（卡片/编辑测试）→ 从已保存条目继承密钥与地址
    if entry.api_key.is_none() && entry.api_key_env.is_none() {
        if let Some(old) = state.backends_config.list().iter().find(|e| e.backend_id() == entry.backend_id()) {
            entry.api_key = old.api_key.clone();
            entry.api_key_env = old.api_key_env.clone();
            if entry.base_url.is_none() { entry.base_url = old.base_url.clone(); }
            if entry.models.is_empty() { entry.models = old.models.clone(); }
        }
    }
    let id = entry.backend_id();
    let target = match test_target(&entry) {
        Ok(Some(t)) => t,
        Ok(None) => {
            record_test_status(&state, &id, true, Some(0), None);
            return (StatusCode::OK, Json(json!({ "ok": true, "latencyMs": 0 }))).into_response();
        }
        Err(msg) => {
            record_test_status(&state, &id, false, None, Some(msg.clone()));
            return bad_request(&msg);
        }
    };
    match probe(&target).await {
        Ok(out) => {
            record_test_status(&state, &id, true, Some(out.latency_ms), None);
            (StatusCode::OK, Json(json!({
                "ok": true,
                "latencyMs": out.latency_ms,
                "models": out.models,
            }))).into_response()
        }
        Err(msg) => {
            record_test_status(&state, &id, false, None, Some(msg.clone()));
            (StatusCode::OK, Json(json!({ "ok": false, "error": msg }))).into_response()
        }
    }
}
/// 从 OpenAI 响应提取 usage。
pub fn extract_openai_usage(resp: &Value) -> (u64, u64) {
    let usage = resp.get("usage").cloned().unwrap_or(json!({}));
    let pt = usage.get("prompt_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    let ct = usage.get("completion_tokens").and_then(|v| v.as_u64()).unwrap_or(0);
    (pt, ct)
}

/// Anthropic 请求 → OpenAI 请求。
/// 转换规则：
/// - tools（Anthropic 的 name/description/input_schema）→ OpenAI function tools
/// - 消息 content 块：text → 普通文本；assistant 的 tool_use → OpenAI tool_calls；
///   user 的 tool_result → role:"tool" 消息（tool_call_id 对齐）
pub fn anthropic_to_openai(body: &Value) -> Value {
    let model = body.get("model").and_then(|v| v.as_str()).unwrap_or("default");
    let max_tokens = body.get("max_tokens").and_then(|v| v.as_u64()).unwrap_or(4096);
    // system 可能是字符串或 text 块数组
    let system_texts: Vec<String> = match body.get("system") {
        Some(Value::String(s)) => vec![s.clone()],
        Some(Value::Array(arr)) => arr.iter()
            .filter_map(|c| c.get("text").and_then(|t| t.as_str()).map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };
    let system = system_texts.join("\n");

    // tools：Anthropic → OpenAI
    let openai_tools: Vec<Value> = body.get("tools")
        .and_then(|t| t.as_array())
        .map(|arr| arr.iter().filter_map(|tool| {
            let name = tool.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name.is_empty() { return None; }
            let description = tool.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let parameters = tool.get("input_schema").cloned().unwrap_or(json!({}));
            Some(json!({
                "type": "function",
                "function": {
                    "name": name,
                    "description": description,
                    "parameters": parameters,
                },
            }))
        }).collect())
        .unwrap_or_default();

    let messages = body.get("messages").cloned().unwrap_or(json!([]));
    let mut openai_messages: Vec<Value> = Vec::new();
    if !system.is_empty() {
        openai_messages.push(json!({ "role": "system", "content": system }));
    }
    if let Some(arr) = messages.as_array() {
        for m in arr {
            let role = m.get("role").and_then(|v| v.as_str()).unwrap_or("user");
            let content = m.get("content").cloned().unwrap_or(json!(""));
            match &content {
                Value::String(s) => {
                    openai_messages.push(json!({ "role": role, "content": s }));
                }
                Value::Array(blocks) => {
                    let mut text_parts: Vec<String> = Vec::new();
                    let mut tool_calls: Vec<Value> = Vec::new();
                    let mut tool_results: Vec<Value> = Vec::new();
                    for b in blocks {
                        let btype = b.get("type").and_then(|v| v.as_str()).unwrap_or("text");
                        match btype {
                            "tool_use" => {
                                let id = b.get("id").and_then(|v| v.as_str()).unwrap_or("call_0").to_string();
                                let fname = b.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string();
                                let args = b.get("input").cloned().unwrap_or(json!({}));
                                tool_calls.push(json!({
                                    "id": id,
                                    "type": "function",
                                    "function": { "name": fname, "arguments": args.to_string() },
                                }));
                            }
                            "tool_result" => {
                                let id = b.get("tool_use_id").and_then(|v| v.as_str()).unwrap_or("call_0").to_string();
                                let result = match b.get("content") {
                                    Some(Value::String(s)) => s.clone(),
                                    Some(Value::Array(inner)) => inner.iter()
                                        .filter_map(|c| c.get("text").and_then(|t| t.as_str()))
                                        .collect::<Vec<_>>().join("\n"),
                                    _ => String::new(),
                                };
                                tool_results.push(json!({
                                    "tool_call_id": id,
                                    "content": result,
                                }));
                            }
                            _ => {
                                if let Some(t) = b.get("text").and_then(|v| v.as_str()) {
                                    text_parts.push(t.to_string());
                                }
                            }
                        }
                    }
                    let text = text_parts.join("\n");
                    // 纯 tool_result 消息（Anthropic 里 role=user）跳过空 user 壳，只发 role:"tool"
                    if !text.is_empty() || !tool_calls.is_empty() {
                        let mut msg = json!({ "role": role, "content": text });
                        if !tool_calls.is_empty() {
                            msg["tool_calls"] = json!(tool_calls);
                        }
                        openai_messages.push(msg);
                    }
                    for tr in tool_results {
                        openai_messages.push(json!({ "role": "tool", "content": tr["content"], "tool_call_id": tr["tool_call_id"] }));
                    }
                }
                _ => {
                    openai_messages.push(json!({ "role": role, "content": "" }));
                }
            }
        }
    }
    let mut out = json!({
        "model": model,
        "max_tokens": max_tokens,
        "messages": openai_messages,
    });
    if !openai_tools.is_empty() {
        out["tools"] = json!(openai_tools);
    }
    out
}

/// 从 OpenAI choices[0].message 提取 content、tool_calls 与 finish_reason 的公共逻辑。
struct OpenAiChoice {
    text: String,
    tool_calls: Vec<Value>,
    finish_reason: Option<String>,
}

fn first_choice(resp: &Value) -> Option<OpenAiChoice> {
    let choice = resp
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|arr| arr.first())?;
    let message = choice.get("message")?;
    let text = message
        .get("content")
        .and_then(|c| c.as_str())
        .unwrap_or("")
        .to_string();
    let tool_calls = message
        .get("tool_calls")
        .and_then(|t| t.as_array())
        .cloned()
        .unwrap_or_default();
    let finish_reason = choice
        .get("finish_reason")
        .and_then(|f| f.as_str())
        .map(|s| s.to_string());
    Some(OpenAiChoice { text, tool_calls, finish_reason })
}

/// OpenAI 响应 → Anthropic 响应。
/// tool_calls → content 里的 tool_use 块；finish_reason "tool_calls" → stop_reason "tool_use"。
pub fn openai_to_anthropic(resp: &Value) -> Value {
    let model = resp.get("model").and_then(|v| v.as_str()).unwrap_or("default");
    let (pt, ct) = extract_openai_usage(resp);

    let mut blocks: Vec<Value> = Vec::new();
    let mut stop_reason = "end_turn";
    if let Some(c) = first_choice(resp) {
        if !c.text.is_empty() {
            blocks.push(json!({ "type": "text", "text": c.text }));
        }
        for tc in &c.tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("call_0");
            let fname = tc.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // arguments 是 JSON 字符串，尽量解析成对象；解析失败就用原始字符串
            let input: Value = tc.get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| tc.get("function")
                    .and_then(|f| f.get("arguments"))
                    .cloned()
                    .unwrap_or(json!({})));
            blocks.push(json!({
                "type": "tool_use",
                "id": id,
                "name": fname,
                "input": input,
            }));
        }
        if c.finish_reason.as_deref() == Some("tool_calls") {
            stop_reason = "tool_use";
        } else if c.finish_reason.as_deref() == Some("length") {
            stop_reason = "max_tokens";
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({ "type": "text", "text": "" }));
    }
    json!({
        "id": "msg_mock_0001",
        "type": "message",
        "role": "assistant",
        "model": model,
        "content": blocks,
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": {
            "input_tokens": pt,
            "output_tokens": ct,
        },
    })
}

/// Anthropic SSE 流事件（把 OpenAI 上游的完整响应聚合成一次 SSE 回放；支持工具块）。
pub fn anthropic_sse_stream(resp: &Value) -> String {
    let model = resp.get("model").and_then(|v| v.as_str()).unwrap_or("default");
    let (pt, ct) = extract_openai_usage(resp);

    // 把 OpenAI 响应拆成 Anthropic 内容块（text / tool_use）
    let mut blocks: Vec<Value> = Vec::new();
    let mut stop_reason = "end_turn";
    if let Some(c) = first_choice(resp) {
        if !c.text.is_empty() {
            blocks.push(json!({ "type": "text", "text": c.text }));
        }
        for tc in &c.tool_calls {
            let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("call_0");
            let fname = tc.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let input: Value = tc.get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|a| a.as_str())
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_else(|| tc.get("function")
                    .and_then(|f| f.get("arguments"))
                    .cloned()
                    .unwrap_or(json!({})));
            blocks.push(json!({
                "type": "tool_use",
                "id": id,
                "name": fname,
                "input": input,
            }));
        }
        if c.finish_reason.as_deref() == Some("tool_calls") {
            stop_reason = "tool_use";
        } else if c.finish_reason.as_deref() == Some("length") {
            stop_reason = "max_tokens";
        }
    }
    if blocks.is_empty() {
        blocks.push(json!({ "type": "text", "text": "" }));
    }

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
    for (i, block) in blocks.iter().enumerate() {
        let btype = block.get("type").and_then(|v| v.as_str()).unwrap_or("text");
        out.push_str(&format!("event: content_block_start\ndata: {}\n\n", json!({
            "type": "content_block_start",
            "index": i,
            "content_block": block,
        })));
        match btype {
            "text" => {
                let text = block.get("text").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!("event: content_block_delta\ndata: {}\n\n", json!({
                    "type": "content_block_delta",
                    "index": i,
                    "delta": { "type": "text_delta", "text": text },
                })));
            }
            "tool_use" => {
                let input = block.get("input").cloned().unwrap_or(json!({}));
                out.push_str(&format!("event: content_block_delta\ndata: {}\n\n", json!({
                    "type": "content_block_delta",
                    "index": i,
                    "delta": { "type": "input_json_delta", "partial_json": input.to_string() },
                })));
            }
            _ => {}
        }
        out.push_str(&format!("event: content_block_stop\ndata: {}\n\n", json!({
            "type": "content_block_stop",
            "index": i,
        })));
    }
    out.push_str(&format!("event: message_delta\ndata: {}\n\n", json!({
        "type": "message_delta",
        "delta": { "stop_reason": stop_reason, "stop_sequence": null },
        "usage": { "output_tokens": ct },
    })));
    out.push_str(&format!("event: message_stop\ndata: {}\n\n", json!({
        "type": "message_stop",
    })));
    out
}

/// OpenAI SSE 流事件（把上游完整 JSON 响应聚合成 OpenAI 兼容 chunk 序列；供 stream=true 请求回放）。
/// 与 anthropic_sse_stream 对称：上游始终非流式，网关负责把完整响应切成客户端可消费的 SSE。
pub fn openai_sse_stream(resp: &Value) -> String {
    let id = resp.get("id").and_then(|v| v.as_str()).unwrap_or("chatcmpl-mock").to_string();
    let model = resp.get("model").and_then(|v| v.as_str()).unwrap_or("default").to_string();
    let created = resp.get("created").and_then(|v| v.as_u64()).unwrap_or(0);
    let (pt, ct) = extract_openai_usage(resp);

    let mut out = String::new();
    // 首块：带 role 的空 delta（OpenAI SDK 期望先收到 role）
    out.push_str(&format!("data: {}\n\n", json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": { "role": "assistant", "content": "" },
            "finish_reason": null,
        }],
    })));

    let mut finish_reason = "stop";
    if let Some(c) = first_choice(resp) {
        if !c.text.is_empty() {
            out.push_str(&format!("data: {}\n\n", json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": { "content": c.text },
                    "finish_reason": null,
                }],
            })));
        }
        for tc in &c.tool_calls {
            let tname = tc.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let targs = tc.get("function")
                .and_then(|f| f.get("arguments"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let tid = tc.get("id").and_then(|v| v.as_str()).unwrap_or("call_0");
            let idx = tc.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
            out.push_str(&format!("data: {}\n\n", json!({
                "id": id,
                "object": "chat.completion.chunk",
                "created": created,
                "model": model,
                "choices": [{
                    "index": 0,
                    "delta": { "tool_calls": [{
                        "index": idx,
                        "id": tid,
                        "type": "function",
                        "function": { "name": tname, "arguments": targs },
                    }] },
                    "finish_reason": null,
                }],
            })));
        }
        if c.finish_reason.as_deref() == Some("tool_calls") {
            finish_reason = "tool_calls";
        } else if c.finish_reason.as_deref() == Some("length") {
            finish_reason = "length";
        }
    }
    // 收尾块：finish_reason
    out.push_str(&format!("data: {}\n\n", json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": {},
            "finish_reason": finish_reason,
        }],
        "usage": { "prompt_tokens": pt, "completion_tokens": ct, "total_tokens": pt + ct },
    })));
    out.push_str("data: [DONE]\n\n");
    out
}

/// GET /v1/models（OpenAI 格式模型目录）。
/// 检测本机主出口 IPv4（UDP connect 到公共 DNS，不实际发包）。
fn primary_lan_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:53").ok()?;
    match s.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) => Some(v4.ip().to_string()),
        _ => None,
    }
}

/// 本机机器名（UI 预填「本机 key」用的 machineName）。
fn host_name() -> String {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "this-machine".to_string())
}

/// GET /api/info：接入信息（监听端口、本机局域网地址、暴露模型），供组长配置客户端 / cc-switch。
pub async fn api_info(State(state): State<ApiState>) -> Response {
    let lan_ip = if state.bind.is_loopback() {
        "127.0.0.1".to_string()
    } else if state.bind.is_unspecified() {
        primary_lan_ip().unwrap_or_else(|| "127.0.0.1".to_string())
    } else {
        state.bind.to_string()
    };
    let unique: std::collections::HashSet<String> =
        state.backends.models_catalog().into_iter().map(|(m, _)| m).collect();
    let mut models: Vec<String> = unique.into_iter().collect();
    models.sort();
    (
        StatusCode::OK,
        Json(json!({
            "port": state.port,
            "sharePort": state.share_port,
            "lanIp": lan_ip,
            "baseUrl": format!("http://{lan_ip}:{}/v1", state.port),
            "anthropicBaseUrl": format!("http://{lan_ip}:{}", state.port),
            "consoleUrl": format!("http://127.0.0.1:{}", state.port),
            "localOnly": state.bind.is_loopback(),
            "hostName": host_name(),
            "models": models,
        })),
    )
        .into_response()
}

/// GET /api/models：支持的模型 ID 列表（去重排序、纯 JSON 数组，便于粘贴给 cc-switch）。
pub async fn api_models(State(state): State<ApiState>) -> Response {
    let unique: std::collections::HashSet<String> =
        state.backends.models_catalog().into_iter().map(|(m, _)| m).collect();
    let mut models: Vec<String> = unique.into_iter().collect();
    models.sort();
    (StatusCode::OK, Json(models)).into_response()
}

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
    fn bearer_accepts_authorization_and_x_api_key() {
        // Authorization: Bearer
        let mut h1 = HeaderMap::new();
        h1.insert(axum::http::header::AUTHORIZATION, "Bearer abc123".parse().unwrap());
        assert_eq!(bearer_token(&h1).as_deref(), Some("abc123"));

        // x-api-key（Anthropic 标准，cc-switch 界面测试使用）
        let mut h2 = HeaderMap::new();
        h2.insert("x-api-key", "tok-xyz".parse().unwrap());
        assert_eq!(bearer_token(&h2).as_deref(), Some("tok-xyz"));

        // 无认证头 → None
        let h3 = HeaderMap::new();
        assert_eq!(bearer_token(&h3), None);

        // x-api-key 优先于 Authorization 缺失时的回退，Authorization 存在时优先
        let mut h4 = HeaderMap::new();
        h4.insert("x-api-key", "fallback".parse().unwrap());
        h4.insert(axum::http::header::AUTHORIZATION, "Bearer primary".parse().unwrap());
        assert_eq!(bearer_token(&h4).as_deref(), Some("primary"));
    }

    #[test]
    fn test_target_mock_is_local() {
        let e = BackendEntry { provider: "mock".into(), ..Default::default() };
        assert!(test_target(&e).unwrap().is_none(), "mock 免网络");
    }

    #[test]
    fn test_target_requires_key() {
        let e = BackendEntry { provider: "deepseek".into(), ..Default::default() };
        assert!(test_target(&e).is_err(), "无密钥应报错");
    }

    #[test]
    fn test_target_custom_requires_url() {
        let e = BackendEntry {
            provider: "ollama".into(),
            api_key: Some("x".into()),
            ..Default::default()
        };
        assert!(test_target(&e).is_err(), "custom 无 base_url 应报错");
    }

    #[test]
    fn test_target_builds_url_with_key() {
        let e = BackendEntry {
            provider: "deepseek".into(),
            api_key: Some("sk-test".into()),
            ..Default::default()
        };
        let t = test_target(&e).unwrap().expect("deepseek 官方 base_url");
        assert_eq!(t.url, "https://api.deepseek.com/models");
        assert_eq!(t.key, "sk-test");
    }

    #[test]
    fn parse_models_from_response_extracts_ids() {
        let v = json!({ "data": [
            { "id": "deepseek-chat", "object": "model" },
            { "id": "deepseek-reasoner", "object": "model" },
            { "id": "deepseek-reasoner", "object": "model" }, // 重复去重
            { "object": "model" },                            // 无 id 跳过
            { "id": " ", "object": "model" },                 // 空串跳过
        ] });
        let ids = parse_models_from_response(&v);
        assert_eq!(ids, vec!["deepseek-chat", "deepseek-reasoner"]);
        // 无 data 或非数组 → 空
        assert!(parse_models_from_response(&json!({ "foo": 1 })).is_empty());
        assert!(parse_models_from_response(&json!({ "data": {} })).is_empty());
    }

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

    #[test]
    fn anthropic_tools_to_openai() {
        let body = json!({
            "model": "claude-3",
            "max_tokens": 1024,
            "system": [{ "type": "text", "text": "be brief" }],
            "tools": [{
                "name": "get_weather",
                "description": "look up weather",
                "input_schema": { "type": "object", "properties": { "city": { "type": "string" } } },
            }],
            "messages": [
                { "role": "user", "content": "what is the weather?" },
                {
                    "role": "assistant",
                    "content": [
                        { "type": "text", "text": "checking" },
                        { "type": "tool_use", "id": "tu_1", "name": "get_weather", "input": { "city": "beijing" } },
                    ],
                },
                {
                    "role": "user",
                    "content": [
                        { "type": "tool_result", "tool_use_id": "tu_1", "content": "sunny" },
                    ],
                },
            ],
        });
        let openai = anthropic_to_openai(&body);
        assert_eq!(openai["tools"][0]["type"], "function");
        assert_eq!(openai["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(openai["tools"][0]["function"]["parameters"]["properties"]["city"]["type"], "string");
        // 第一条 user 消息
        assert_eq!(openai["messages"][1]["role"], "user");
        assert_eq!(openai["messages"][1]["content"], "what is the weather?");
        // assistant tool_use → tool_calls；text 保留
        assert_eq!(openai["messages"][2]["role"], "assistant");
        assert_eq!(openai["messages"][2]["content"], "checking");
        assert_eq!(openai["messages"][2]["tool_calls"][0]["id"], "tu_1");
        assert_eq!(openai["messages"][2]["tool_calls"][0]["function"]["name"], "get_weather");
        // tool_result → role tool
        assert_eq!(openai["messages"][3]["role"], "tool");
        assert_eq!(openai["messages"][3]["tool_call_id"], "tu_1");
        assert_eq!(openai["messages"][3]["content"], "sunny");
    }

    #[test]
    fn openai_tool_calls_to_anthropic_tool_use() {
        let resp = json!({
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_abc",
                        "type": "function",
                        "function": { "name": "get_weather", "arguments": "{\"city\":\"shanghai\"}" },
                    }],
                },
                "finish_reason": "tool_calls",
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 },
        });
        let a = openai_to_anthropic(&resp);
        assert_eq!(a["stop_reason"], "tool_use");
        assert_eq!(a["content"][0]["type"], "tool_use");
        assert_eq!(a["content"][0]["id"], "call_abc");
        assert_eq!(a["content"][0]["name"], "get_weather");
        assert_eq!(a["content"][0]["input"]["city"], "shanghai");
    }

    #[test]
    fn sse_emits_tool_use_events() {
        let resp = json!({
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "let me check",
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": { "name": "run_shell", "arguments": "{\"cmd\":\"pwd\"}" },
                    }],
                },
                "finish_reason": "tool_calls",
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 },
        });
        let sse = anthropic_sse_stream(&resp);
        assert!(sse.contains("\"type\":\"tool_use\""));
        assert!(sse.contains("\"type\":\"content_block_start\""));
        assert!(sse.contains("input_json_delta"));
        assert!(sse.contains("\"stop_reason\":\"tool_use\""));
        assert!(sse.contains("\"name\":\"run_shell\""));
    }

    #[test]
    fn anthropic_tool_roundtrip_through_backend() {
        // 模拟一次带工具调用的完整往返：Claude Code 请求 → 转换 → 后端响应 → 转换回 Anthropic
        let body = json!({
            "model": "deepseek-chat",
            "max_tokens": 512,
            "system": "You are helpful",
            "messages": [
                { "role": "user", "content": "List files" },
                { "role": "assistant", "content": [{ "type": "tool_use", "id": "call_1", "name": "Bash", "input": { "command": "ls" } }] },
                { "role": "user", "content": [{ "type": "tool_result", "tool_use_id": "call_1", "content": "src main.rs" }] },
            ],
        });
        let openai_req = anthropic_to_openai(&body);
        // backend 会把 openai_req 原样转发（含 tools 定义由整体请求控制），这里模拟 OpenAI 响应
        let openai_resp = json!({
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Done.", "tool_calls": [] },
                "finish_reason": "stop",
            }],
            "usage": { "prompt_tokens": 20, "completion_tokens": 3 },
        });
        let anthropic = openai_to_anthropic(&openai_resp);
        assert_eq!(anthropic["content"][0]["type"], "text");
        assert_eq!(anthropic["content"][0]["text"], "Done.");
        assert_eq!(anthropic["stop_reason"], "end_turn");
        // OpenAI 工具消息顺序正确：system → user → assistant(tool_calls) → tool
        assert_eq!(openai_req["messages"][0]["role"], "system");
        assert_eq!(openai_req["messages"][1]["role"], "user");
        assert_eq!(openai_req["messages"][2]["role"], "assistant");
        assert_eq!(openai_req["messages"][2]["tool_calls"][0]["function"]["name"], "Bash");
        assert_eq!(openai_req["messages"][3]["role"], "tool");
        assert_eq!(openai_req["messages"][3]["tool_call_id"], "call_1");
    }

    #[test]
    fn openai_sse_stream_emits_standard_chunks() {
        // OpenAI 兼容路径（/v1/chat/completions stream=true）：上游完整 JSON → 网关回放标准 SSE
        let openai_resp = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1730000000,
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": { "role": "assistant", "content": "Hello!" },
                "finish_reason": "stop",
            }],
            "usage": { "prompt_tokens": 7, "completion_tokens": 2 },
        });
        let sse = openai_sse_stream(&openai_resp);
        let events: Vec<Value> = sse
            .lines()
            .filter_map(|l| l.strip_prefix("data: ").and_then(|d| serde_json::from_str(d).ok()))
            .collect();
        assert!(sse.trim_end().ends_with("data: [DONE]"), "应以 [DONE] 结束");
        assert!(!events.is_empty());
        // 首块：role delta
        assert_eq!(events[0]["object"], "chat.completion.chunk");
        assert_eq!(events[0]["choices"][0]["delta"]["role"], "assistant");
        // 中间块：真实内容
        let content_chunks: Vec<&Value> = events
            .iter()
            .filter(|e| e["choices"][0]["delta"]["content"].as_str().map(|s| !s.is_empty()).unwrap_or(false))
            .collect();
        assert_eq!(content_chunks.len(), 1);
        assert_eq!(content_chunks[0]["choices"][0]["delta"]["content"], "Hello!");
        // 收尾块：finish_reason
        let last = events.last().unwrap();
        assert_eq!(last["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn openai_sse_stream_emits_tool_call_chunk() {
        let openai_resp = json!({
            "id": "chatcmpl-456",
            "object": "chat.completion",
            "created": 1730000001,
            "model": "deepseek-chat",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "",
                    "tool_calls": [{
                        "index": 0,
                        "id": "call_A",
                        "type": "function",
                        "function": { "name": "get_time", "arguments": "{\"city\": \"Beijing\"}" },
                    }],
                },
                "finish_reason": "tool_calls",
            }],
            "usage": { "prompt_tokens": 7, "completion_tokens": 9 },
        });
        let sse = openai_sse_stream(&openai_resp);
        let events: Vec<Value> = sse
            .lines()
            .filter_map(|l| l.strip_prefix("data: ").and_then(|d| serde_json::from_str(d).ok()))
            .collect();
        assert!(sse.trim_end().ends_with("data: [DONE]"));
        // 找到携带 tool_calls 的块
        let tc_chunks: Vec<&Value> = events
            .iter()
            .filter(|e| e["choices"][0]["delta"]["tool_calls"][0]["function"]["name"].as_str().is_some())
            .collect();
        assert_eq!(tc_chunks.len(), 1);
        let tc = tc_chunks[0];
        assert_eq!(tc["choices"][0]["delta"]["tool_calls"][0]["type"], "function");
        assert_eq!(tc["choices"][0]["delta"]["tool_calls"][0]["id"], "call_A");
        assert_eq!(tc["choices"][0]["delta"]["tool_calls"][0]["function"]["name"], "get_time");
        let args: Value = serde_json::from_str(
            tc["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"].as_str().unwrap(),
        ).unwrap();
        assert_eq!(args["city"], "Beijing");
        // 收尾块：finish_reason tool_calls
        let last = events.last().unwrap();
        assert_eq!(last["choices"][0]["finish_reason"], "tool_calls");
    }
}