//! Typed storage errors enabling fail-safe `EngineMode::Halted` on disk/IO failure (§8).

use rusqlite::ErrorCode;
use thiserror::Error;

/// Errors returned by [`crate::SqliteStore`].
#[derive(Debug, Error)]
pub enum StoreError {
    /// Storage volume is full (`SQLITE_FULL`); engine **must** transition to `Halted`.
    #[error("storage disk full: {0}")]
    DiskFull(String),

    /// Filesystem or SQLite I/O error (`SQLITE_IOERR`, `SQLITE_READONLY`, `SQLITE_CANTOPEN`, etc.);
    /// engine **must** transition to `Halted`.
    #[error("storage I/O error: {0}")]
    IoError(String),

    /// Database corruption or integrity check failure; engine **must** transition to `Halted`.
    #[error("storage corruption detected: {0}")]
    Corruption(String),

    /// General SQLite query/constraint error.
    #[error("sqlite error: {0}")]
    Sqlite(String),

    /// JSON/Decimal serialization or deserialization error.
    #[error("serialization error: {0}")]
    Serialization(String),

    /// Journal event carries a newer schema version than this binary supports.
    #[error("unsupported journal event schema version {version} (max supported: {max_supported})")]
    UnsupportedEventVersion {
        /// Version found in persisted payload.
        version: u32,
        /// Maximum version supported by this build.
        max_supported: u32,
    },
}

impl StoreError {
    /// Returns `true` if this error represents a fatal storage fault (disk full, I/O error,
    /// or corruption) requiring the engine to halt immediately.
    #[must_use]
    pub const fn is_fatal_storage(&self) -> bool {
        matches!(
            self,
            Self::DiskFull(_) | Self::IoError(_) | Self::Corruption(_)
        )
    }

    /// Classify a [`rusqlite::Error`] into the appropriate [`StoreError`] variant.
    #[must_use]
    pub fn from_rusqlite(err: &rusqlite::Error) -> Self {
        if let rusqlite::Error::SqliteFailure(ffi_err, _) = err {
            match ffi_err.code {
                ErrorCode::DiskFull => return Self::DiskFull(err.to_string()),
                ErrorCode::SystemIoFailure
                | ErrorCode::CannotOpen
                | ErrorCode::ReadOnly
                | ErrorCode::PermissionDenied => return Self::IoError(err.to_string()),
                ErrorCode::DatabaseCorrupt | ErrorCode::NotADatabase => {
                    return Self::Corruption(err.to_string());
                }
                _ => {}
            }
        }
        Self::Sqlite(err.to_string())
    }
}

impl From<rusqlite::Error> for StoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self::from_rusqlite(&err)
    }
}

impl From<std::io::Error> for StoreError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

impl From<serde_json::Error> for StoreError {
    fn from(err: serde_json::Error) -> Self {
        Self::Serialization(err.to_string())
    }
}
