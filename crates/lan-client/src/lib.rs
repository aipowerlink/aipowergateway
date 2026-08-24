//! aipg-lan-client: 消费端（组员）模块。
//!
//! - `lan-discovery-client`：UDP 监听广播 + 扫描，维护组长列表
//! - `lan-share-client`：密码 → Bearer token、双协议 API 调用
//! - `lan-identity`：机器名/显示名管理
//! - `lan-usage-view`：个人用量记录

pub mod discovery;
pub mod gateway;
pub mod identity;
pub mod share_client;
pub mod usage_view;

pub use discovery::{DiscoveryClient, DiscoveryConfig, LeaderInfo};
pub use identity::Identity;
pub use share_client::{ShareClient, ShareClientConfig};
pub use usage_view::UsageView;