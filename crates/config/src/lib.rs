//! aipg-config: 配置管理——单一 SQLite 配置库 + 角色分区 + Vault 加密 + schema 驱动。
//!
//! 对应 design D6.4：
//! - 表分区：settings/node_identity/server_config/members/usage/client_config/client_credentials
//! - Vault：密码/token 加密存储（AES-GCM）
//! - 脱敏：secret 字段读取/导出不回传明文

pub mod store;
pub mod vault;

pub use store::{ConfigService, DbError, RoleView};
pub use vault::Vault;