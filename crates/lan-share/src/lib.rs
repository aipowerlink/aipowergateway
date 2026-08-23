//! aipg-lan-share: 服务端（组长）共享模块。
//!
//! 模块：
//! - `lan-share-server`：双协议 HTTP API（OpenAI /v1/chat/completions + Anthropic /v1/messages SSE）
//! - `lan-auth`：密码 → Bearer token、踢人吊销、改密
//! - `lan-usage`：按成员计量 token（消费 OpenAI/Anthropic usage）、持久化
//! - `lan-member-registry`：成员登记/在线/改名
//! - `lan-discovery-broadcast`：UDP 周期广播

pub mod api;
pub mod broadcast;
pub mod auth;
pub mod backend;
pub mod member;
pub mod server;
pub mod usage;

pub use auth::AuthService;
pub use broadcast::{BroadcastConfig, BroadcastService};
pub use backend::{Backend, MockBackend};
pub use member::MemberRegistry;
pub use server::{ShareServer, ShareServerConfig};
pub use usage::UsageService;