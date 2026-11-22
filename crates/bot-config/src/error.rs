//! Typed configuration and tighten-only error definitions.

use std::fmt;
use std::path::PathBuf;

use thiserror::Error;

/// A single configuration validation violation (range or cross-field invariant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RangeViolation {
    /// Dot-separated configuration field path (e.g. `"risk.risk_per_trade_pct"`).
    pub field: &'static str,
    /// Stringified offending value.
    pub value: String,
    /// Explanation of the required range or invariant.
    pub reason: String,
}

impl fmt::Display for RangeViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} = {} ({})", self.field, self.value, self.reason)
    }
}

/// Errors returned when loading or validating [`crate::BotConfig`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ConfigError {
    /// Failed to parse or extract configuration layers (including unknown fields).
    #[error("configuration load error: {0}")]
    Load(String),
    /// Failed to read a configuration file or directory from disk.
    #[error("configuration I/O error at {}: {message}", path.display())]
    Io {
        /// Path that failed to read.
        path: PathBuf,
        /// Underlying I/O error message.
        message: String,
    },
    /// One or more configuration fields violated strict bounds or safety invariants.
    #[error("configuration validation failed: {}", format_range_violations(.0))]
    Validation(Vec<RangeViolation>),
}

/// A single runtime config change that would loosen risk or modify an immutable field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoosenViolation {
    /// Dot-separated configuration field path.
    pub field: &'static str,
    /// Previous value.
    pub old: String,
    /// Proposed value.
    pub new: String,
    /// Why this change is forbidden at runtime.
    pub reason: &'static str,
}

impl fmt::Display for LoosenViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} -> {} ({})",
            self.field, self.old, self.new, self.reason
        )
    }
}

/// Errors returned by runtime tighten-only checks.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoosenError {
    /// Proposed target config itself failed strict validation.
    #[error("proposed config is invalid: {0}")]
    InvalidTarget(#[from] ConfigError),
    /// One or more proposed changes loosen risk or mutate an immutable runtime field.
    #[error("runtime config change rejected (tighten-only): {}", format_loosen_violations(.0))]
    Loosened(Vec<LoosenViolation>),
}

fn format_range_violations(violations: &[RangeViolation]) -> String {
    violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_loosen_violations(violations: &[LoosenViolation]) -> String {
    violations
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join("; ")
}
