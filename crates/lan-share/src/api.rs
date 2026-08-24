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
/// 检测本机主出口 IPv4（UDP connect 到公共 DNS，不实际发包）。
fn primary_lan_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:53").ok()?;
    match s.local_addr().ok()? {
        std::net::SocketAddr::V4(v4) => Some(v4.ip().to_string()),
        _ => None,
    }
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
            "models": models,
        })),
    )
        .into_response()
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
}