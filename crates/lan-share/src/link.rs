//! link-mw：组长侧「协商式 + 可强制」链路压缩+加密中间件。
//!
//! 规则（design 2026-08-26 §5.3/§6/§8 M2+M3）：
//! - 请求带 `x-aipg-enc: v1` 且非排除端点 → 解密 body 后再进 handler；响应加密返回
//!   （解密必须先于模型路由与计量；密钥 = SHA-256(Authorization Bearer token)）；
//! - **快路径**：无 `x-aipg-enc` header（LAN 明文 / 旧成员 / 管理端点）→ 原样透传，
//!   零拷贝零开销，不经过任何加解密；
//! - **强制策略（M3）**：`link.encrypt = enforce` 时，未声明加密的 `/v1/*` 模型端点
//!   回 426 Upgrade Required（带 `Upgrade: x-aipg-enc` 头）；管理 `/api/*` 与
//!   `/auth/*` 排除端点仍明文可达（面板/换令牌不可被锁死）；
//! - 排除端点 `/auth/token`、`/auth/rename`：换 token 类端点必须在加密协商前可用，
//!   一律明文透传（带 header 也忽略）。

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderMap, Request};
use axum::middleware::Next;
use axum::response::Response;

use aipg_link_crypto::{ENC_HEADER, ENC_VERSION, LinkCrypto};

use crate::api::ApiState;

/// 排除端点：换 token / 改名，参与密钥协商前必须先明文可达。
const EXEMPT_PATHS: [&str; 2] = ["/auth/token", "/auth/rename"];

/// 收集上限（64MB）：防恶意/异常大包拖垮内存；正常对话上下文远小于此。
const BODY_LIMIT: usize = 64 * 1024 * 1024;

/// 提取访问令牌：优先 Authorization: Bearer，其次 x-api-key（与 api.rs 保持一致）。
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

/// 组长侧「协商式 + 可强制」加密中间件。策略取自 ApiState.link_policy：
/// - Off：纯透传快路径（关闭加密支持，LAN 明文）；
/// - AesGcm：协商式（带 x-aipg-enc 才解密/加密，无头透传，兼容旧成员）；
/// - Enforce：未声明加密的 /v1/* 模型端点回 426 Upgrade Required。
pub async fn link_enc_middleware(State(state): State<ApiState>, req: Request<Body>, next: Next) -> Response {
    let path = req.uri().path();
    let wants_enc = req
        .headers()
        .get(ENC_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| v == ENC_VERSION)
        .unwrap_or(false);
    let policy = *state.link_policy.read().unwrap();

    // 快路径：无协商头（LAN 明文 / 旧成员 / 管理端点）→ 原样透传
    if !wants_enc {
        // M3 强制策略：enforce 下未声明加密的 /v1/* 模型端点回 426（管理与 /auth 保持可达）
        if policy == aipg_link_crypto::LinkEncryptMode::Enforce
            && path.starts_with("/v1/")
            && !EXEMPT_PATHS.contains(&path)
        {
            return upgrade_required();
        }
        return next.run(req).await;
    }
    if EXEMPT_PATHS.contains(&path) {
        return next.run(req).await;
    }

    // off = 纯透传：组长不参与加解密，加密请求按明文处理（与旧组长行为一致）
    if policy == aipg_link_crypto::LinkEncryptMode::Off {
        return next.run(req).await;
    }

    // 加密请求必须有 bearer 令牌（key = SHA-256(token)）；缺失按 400 拒绝，防无钥死锁
    let token = match bearer_token(req.headers()) {
        Some(t) if !t.is_empty() => t,
        _ => return bad_request("encrypted request requires bearer token"),
    };
    let crypto = LinkCrypto::new();
    let key = LinkCrypto::derive_key(&token);
    // 请求侧：整包读取（两端均非真流式）→ 解密 → 重建 body
    // GET 等无体请求（如 /v1/models）：不解密 body，仍按协商加密响应
    let (parts, body) = req.into_parts();
    let inner = if parts.method == axum::http::Method::GET {
        Request::from_parts(parts, body)
    } else {
        let enc_body = match to_bytes(body, BODY_LIMIT).await {
            Ok(b) => b.to_vec(),
            Err(_) => return bad_request("read encrypted body failed"),
        };
        let plain = match crypto.decrypt(&key, &enc_body) {
            Ok(p) => p,
            Err(e) => return bad_request(&format!("decrypt failed: {e} (bad token or tampered)")),
        };
        let mut r = Request::from_parts(parts, Body::from(plain));
        r.headers_mut().remove(ENC_HEADER); // 内层不再携带协商头，避免 handler 误读
        r
    };
    let mut resp = next.run(inner).await;

    // 响应侧：整包加密返回（成员 proxy 按同 key 解密）
    let (rparts, rbody) = resp.into_parts();
    let rblob = match to_bytes(rbody, BODY_LIMIT).await {
        Ok(b) => b.to_vec(),
        Err(_) => return bad_request("read response body failed"),
    };
    let enc = crypto.encrypt(&key, &rblob);
    resp = Response::builder()
        .status(rparts.status)
        .header(ENC_HEADER, ENC_VERSION)
        // 密文替换原 body：Content-Type 语义在解密后恢复（保持 application/json 语义）
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(enc))
        .unwrap_or_else(|e| Response::new(Body::from(format!("encrypt response error: {e}"))));
    // 回填其余响应头（保留状态码语义；跳过 content-length，body 已变）
    for (name, value) in rparts.headers {
        if let Some(name) = name {
            if name != axum::http::header::CONTENT_LENGTH {
                resp.headers_mut().append(name, value);
            }
        }
    }
    resp
}

/// 426 Upgrade Required：强制策略下未声明加密的模型请求（M3）。
fn upgrade_required() -> Response {
    Response::builder()
        .status(axum::http::StatusCode::UPGRADE_REQUIRED)
        .header("Upgrade", ENC_HEADER)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            "{{\"error\":{{\"message\":\"link encryption is enforced: send request with {ENC_HEADER}: {ENC_VERSION}\"}}}}"
        )))
        .unwrap()
}

fn bad_request(msg: &str) -> Response {
    Response::builder()
        .status(axum::http::StatusCode::BAD_REQUEST)
        .header(axum::http::header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!("{{\"error\":{{\"message\":\"{msg}\"}}}}")))
        .unwrap()
}