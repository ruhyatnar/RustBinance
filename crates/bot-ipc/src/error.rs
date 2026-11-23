//! Typed IPC errors (`thiserror`).

use thiserror::Error;

/// Errors returned by IPC authentication, command dispatch, and transport layers.
#[derive(Debug, Error)]
pub enum IpcError {
    /// Missing or invalid bearer token or UDS peer credentials.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// Authenticated principal lacks permission for the requested action (e.g. read-only token on mutation).
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// Invalid or unsupported request payload or parameter.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// Confirmation nonce missing, unknown, expired, or mismatched.
    #[error("invalid confirmation nonce: {0}")]
    InvalidConfirmNonce(String),
    /// Rate limit exceeded for caller.
    #[error("rate limit exceeded")]
    RateLimited,
    /// Feature or command intentionally not supported in this milestone (e.g. `switch_venue` before M16).
    #[error("not supported: {0}")]
    NotSupported(String),
    /// Requested resource (e.g. position or alert) was not found.
    #[error("not found: {0}")]
    NotFound(String),
    /// Configuration tightening violation (`set_param` attempted to loosen a risk parameter).
    #[error("parameter loosening rejected: {0}")]
    LoosenRejected(String),
    /// Underlying storage / audit journal error.
    #[error("journal audit error: {0}")]
    Audit(String),
    /// Transport / IO error.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    /// Serialization / deserialization error.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

impl IpcError {
    /// Machine-readable error code for JSON responses and `command_result`.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Unauthorized(_) => "unauthorized",
            Self::Forbidden(_) => "forbidden",
            Self::BadRequest(_) => "bad_request",
            Self::InvalidConfirmNonce(_) => "invalid_confirm_nonce",
            Self::RateLimited => "rate_limited",
            Self::NotSupported(_) => "not_supported",
            Self::NotFound(_) => "not_found",
            Self::LoosenRejected(_) => "loosen_rejected",
            Self::Audit(_) => "audit_error",
            Self::Io(_) => "io_error",
            Self::Json(_) => "invalid_json",
        }
    }
}
