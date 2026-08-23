//! 运行时错误类型。

use std::fmt;

/// 运行时错误。
#[derive(Debug, Clone)]
pub enum RuntimeError {
    /// 模块重复注册。
    DuplicateModule(String),
    /// 必需模块缺失。
    MissingModule(String),
    /// 依赖循环。
    DependencyCycle(String),
    /// 模块应用失败。
    ModuleError(String),
    /// 配置错误。
    ConfigError(String),
    /// 角色错误。
    RoleError(String),
    /// 鉴权错误。
    Auth(String),
    /// 其他。
    Other(String),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RuntimeError::DuplicateModule(m) => write!(f, "duplicate module: {m}"),
            RuntimeError::MissingModule(m) => write!(f, "missing required module: {m}"),
            RuntimeError::DependencyCycle(d) => write!(f, "dependency cycle: {d}"),
            RuntimeError::ModuleError(e) => write!(f, "module error: {e}"),
            RuntimeError::ConfigError(e) => write!(f, "config error: {e}"),
            RuntimeError::RoleError(e) => write!(f, "role error: {e}"),
            RuntimeError::Auth(e) => write!(f, "auth: {e}"),
            RuntimeError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for RuntimeError {}

impl From<std::io::Error> for RuntimeError {
    fn from(e: std::io::Error) -> Self {
        RuntimeError::Other(format!("io: {e}"))
    }
}

pub type RuntimeResult<T> = Result<T, RuntimeError>;