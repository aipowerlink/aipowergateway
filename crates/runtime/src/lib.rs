//! aipg-runtime: 微内核——模块化装配（参考 DSH/Cordis 与 06 §11.4）。
//!
//! - `Module` trait：name/requires/optional/apply（对应 Cordis 插件契约）
//! - `Host`：服务注册表（provide/get）+ 事件总线（emit/subscribe）
//! - `Runtime::boot`：按角色/配置选择模块集，依赖拓扑装配，Boot/Stop 逆序回收

pub mod config;
pub mod data_dir;
pub mod error;
pub mod event;
pub mod host;
pub mod module;
pub mod runtime;

pub use error::{RuntimeError, RuntimeResult};
pub use event::{EventBus, EventHandler};
pub use host::Host;
pub use module::{Module, ModuleContext};
pub use runtime::{BootResult, Role, Runtime};

/// 模块名常量：服务端（组长）
pub const MOD_LAN_SHARE_SERVER: &str = "lan-share-server";
pub const MOD_LAN_AUTH: &str = "lan-auth";
pub const MOD_LAN_MEMBER_REGISTRY: &str = "lan-member-registry";
pub const MOD_LAN_USAGE: &str = "lan-usage";
pub const MOD_LAN_DISCOVERY_BROADCAST: &str = "lan-discovery-broadcast";
pub const MOD_LAN_WEB_CONSOLE: &str = "lan-web-console";

/// 模块名常量：消费端（组员）
pub const MOD_LAN_DISCOVERY_CLIENT: &str = "lan-discovery-client";
pub const MOD_LAN_SHARE_CLIENT: &str = "lan-share-client";
pub const MOD_LAN_IDENTITY: &str = "lan-identity";
pub const MOD_LAN_USAGE_VIEW: &str = "lan-usage-view";

/// 模块名常量：双角色共用
pub const MOD_LAN_TRAY: &str = "lan-tray";

/// 版本
pub const VERSION: &str = env!("CARGO_PKG_VERSION");