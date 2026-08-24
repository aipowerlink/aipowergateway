//! aipg-coord-client: 协调服务器客户端（跨网络互联 + 遥测上报）。
//!
//! - 设备注册/心跳（PS1 消费侧）
//! - shareId 解析（Deep Link 找组长）
//! - 信令/中继（PS2/PS3 消费侧，0.1.0 预留）
//! - 账号（PS4 消费侧：用户名+邮箱+动态密码）
//! - 匿名遥测（opt-in：版本/平台/区域粗粒度）
//!
//! 契约：apl_docs/03-tech/13-协调服务器API契约.md

mod account;
mod device;
pub mod error;

pub use account::{AccountClient, AccountClientConfig};
pub use device::{DeviceClient, DeviceClientConfig, NodeInfo, HeartbeatTelemetry};
pub use error::{Error, Result};
