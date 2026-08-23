//! 执行后端：驱动本机算力执行请求。0.1.0 为 mock（返回标准响应 + usage）。

use async_trait::async_trait;
use serde_json::{json, Value};

use aipg_runtime::RuntimeResult;

/// 执行后端抽象：输入 OpenAI 兼容请求，返回标准响应（含 usage）。
#[async_trait]
pub trait Backend: Send + Sync {
    /// 执行 chat completion（OpenAI 语义）。
    async fn chat(&self, request: &Value) -> RuntimeResult<Value>;
    /// 后端名（健康/诊断）。
    fn name(&self) -> &'static str;
}

/// Mock 执行后端：0.1.0 验证链路（1.x 接真实推理）。
pub struct MockBackend {
    /// 每 token 耗时系数（模拟推理）。
    pub model: &'static str,
}

impl Default for MockBackend {
    fn default() -> Self {
        Self { model: "mock-7b" }
    }
}

#[async_trait]
impl Backend for MockBackend {
    fn name(&self) -> &'static str {
        "mock"
    }

    async fn chat(&self, request: &Value) -> RuntimeResult<Value> {
        // 提取用户消息用于 mock 回复
        let user_msg = request
            .get("messages")
            .and_then(|m| m.as_array())
            .and_then(|arr| arr.last())
            .and_then(|last| last.get("content"))
            .and_then(|c| c.as_str())
            .unwrap_or("");
        let reply = format!("mock reply to: {}", truncate(user_msg, 80));
        // 模拟 token 数：按回复长度估算
        let completion_tokens = (reply.chars().count() / 2).max(1) as u64;
        let prompt_tokens = (user_msg.chars().count() / 2).max(1) as u64;

        Ok(json!({
            "id": "chatcmpl-mock-0001",
            "object": "chat.completion",
            "created": 0,
            "model": self.model,
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": reply,
                },
                "finish_reason": "stop",
            }],
            "usage": {
                "prompt_tokens": prompt_tokens,
                "completion_tokens": completion_tokens,
                "total_tokens": prompt_tokens + completion_tokens,
            },
        }))
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max).collect()
    }
}