//! coord-client 错误类型。
use thiserror::Error;

#[derive(Debug, Error)]
pub enum Error {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("relay returned {status}: {body}")]
    Relay { status: u16, body: String },
    #[error("relay unreachable: {0}")]
    Unreachable(String),
    #[error("missing response field: {0}")]
    MissingField(String),
    #[error("device not registered")]
    NotRegistered,
    #[error("telemetry disabled")]
    TelemetryDisabled,
}

pub type Result<T> = std::result::Result<T, Error>;
