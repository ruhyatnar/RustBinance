//! Error types for runtime venue switching and lifecycle management.

use bot_types::{EngineMode, Venue};
use thiserror::Error;

/// Rejection or failure reason during `switch_venue`.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SwitchError {
    /// Engine mode does not permit switching (only `Ready` and `Paused` are allowed).
    #[error("switch rejected: engine mode {0:?} does not allow venue switching")]
    InvalidEngineMode(EngineMode),
    /// Reconciliation or initial sync is currently in progress.
    #[error("switch rejected: engine is currently syncing or reconciling")]
    SyncingInProgress,
    /// Target venue is already the active venue.
    #[error("switch rejected: venue {0:?} is already active")]
    AlreadyActive(Venue),
    /// Target venue is disabled in configuration.
    #[error("switch rejected: target venue {0:?} is disabled in config")]
    TargetDisabled(Venue),
    /// Target venue lacks valid credentials.
    #[error("switch rejected: target venue {0:?} has invalid or missing credentials")]
    InvalidCredentials(Venue),
    /// Live mode requires the target venue's `live_confirmation_file`.
    #[error("switch rejected: missing live confirmation file for venue {0:?}")]
    MissingLiveConfirmation(Venue),
    /// Switch cooldown window has not elapsed yet.
    #[error("switch rejected: cooldown active ({remaining_s}s remaining)")]
    CooldownActive {
        /// Remaining cooldown seconds.
        remaining_s: u64,
    },
    /// Daily switch limit reached for the current UTC day.
    #[error("switch rejected: daily switch limit ({limit}) reached")]
    DailyLimitReached {
        /// Configured maximum switches per UTC day.
        limit: u32,
    },
    /// `RequireFlat` policy rejected because the old venue still has open positions.
    #[error("switch rejected: policy require_flat blocked by {count} open position(s)")]
    OpenPositionsRequireFlat {
        /// Number of open positions on the old venue.
        count: usize,
    },
    /// Target venue activation or reconciliation failed (rolled back).
    #[error("switch failed: activation of {venue:?} failed: {reason}")]
    ActivationFailed {
        /// Target venue that failed to activate.
        venue: Venue,
        /// Underlying failure reason.
        reason: String,
    },
}
