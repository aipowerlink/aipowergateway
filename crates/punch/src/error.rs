//! 错误类型。

/// punch crate 统一错误。
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("打洞失败: {0}")]
    PunchFailed(String),

    #[error("打洞超时(无法建立跨网直连): {0}")]
    Timeout(String),

    #[error("UDP IO: {0}")]
    Io(#[from] std::io::Error),

    #[error("QUIC 承载: {0}")]
    Quic(String),

    #[error("证书: {0}")]
    Cert(String),
}

pub type Result<T> = std::result::Result<T, Error>;