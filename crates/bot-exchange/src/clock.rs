//! Thread-safe clock synchronization state with an explicit unsynced (`None`) sentinel.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::spot::ServerTimeSync;

#[derive(Debug, Default)]
struct ClockInner {
    offset_ms: Option<i64>,
    last_sync_ms: Option<i64>,
}

/// Shared clock-offset state distinguishing "unsynced" (`None`) from "0 ms offset" (`Some(0)`).
#[derive(Debug, Clone, Default)]
pub struct ClockSyncState {
    inner: Arc<Mutex<ClockInner>>,
}

impl ClockSyncState {
    /// Create a new unsynced [`ClockSyncState`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a completed server-time synchronization measurement.
    pub fn record(&self, sync: ServerTimeSync) {
        let mut guard = self.lock_inner();
        guard.offset_ms = Some(sync.offset_ms);
        guard.last_sync_ms = Some(sync.local_time_ms);
    }

    /// Measured clock offset (`server_time_ms - local_time_ms`), or `None` if never synced.
    #[must_use]
    pub fn offset_ms(&self) -> Option<i64> {
        self.lock_inner().offset_ms
    }

    /// Local UTC epoch timestamp (ms) of the most recent successful sync, or `None` if unsynced.
    #[must_use]
    pub fn last_sync_ms(&self) -> Option<i64> {
        self.lock_inner().last_sync_ms
    }

    /// Returns `true` if never synced or if `now_ms - last_sync_ms > max_age_ms`.
    #[must_use]
    pub fn needs_sync(&self, now_ms: i64, max_age_ms: i64) -> bool {
        let Some(last) = self.last_sync_ms() else {
            return true;
        };
        now_ms.saturating_sub(last) > max_age_ms
    }

    /// Adjust `local_now_ms` by the recorded clock offset (returns `local_now_ms` if unsynced).
    #[must_use]
    pub fn adjusted_now_ms(&self, local_now_ms: i64) -> i64 {
        local_now_ms.saturating_add(self.offset_ms().unwrap_or(0))
    }

    fn lock_inner(&self) -> MutexGuard<'_, ClockInner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

#[cfg(test)]
mod tests {
    use super::ClockSyncState;
    use crate::spot::ServerTimeSync;

    #[test]
    fn zero_ms_offset_is_distinct_from_unsynced() {
        let clock = ClockSyncState::new();
        assert_eq!(clock.offset_ms(), None);
        assert_eq!(clock.last_sync_ms(), None);
        assert!(clock.needs_sync(1_000, 60_000));

        clock.record(ServerTimeSync {
            server_time_ms: 1_000,
            local_time_ms: 1_000,
            offset_ms: 0,
        });
        assert_eq!(clock.offset_ms(), Some(0));
        assert_eq!(clock.last_sync_ms(), Some(1_000));
        assert!(!clock.needs_sync(30_000, 60_000));
        assert!(clock.needs_sync(61_001, 60_000));
        assert_eq!(clock.adjusted_now_ms(5_000), 5_000);
    }

    #[test]
    fn nonzero_offset_adjusts_local_timestamp() {
        let clock = ClockSyncState::new();
        clock.record(ServerTimeSync {
            server_time_ms: 10_250,
            local_time_ms: 10_000,
            offset_ms: 250,
        });
        assert_eq!(clock.offset_ms(), Some(250));
        assert_eq!(clock.adjusted_now_ms(20_000), 20_250);
    }
}
