//! Rate limit tracking (`REQUEST_WEIGHT` / `ORDERS`) and reconnect backoff (§4.1–§4.2).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

use rand::RngExt;
use serde::{Deserialize, Serialize};

/// Rate limit bucket category reported by Binance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RateLimitType {
    /// IP-scoped request weight.
    RequestWeight,
    /// Account-scoped order count.
    Orders,
    /// Raw connection/request limit.
    RawRequests,
    /// Forward-compatible unknown rate limit type.
    #[serde(other)]
    Unknown,
}

/// Rate limit interval unit reported by Binance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RateLimitInterval {
    /// Per-second window.
    Second,
    /// Per-minute window.
    Minute,
    /// Per-day window.
    Day,
    /// Forward-compatible unknown interval.
    #[serde(other)]
    Unknown,
}

/// Static rate limit rule parsed from `exchangeInfo.rateLimits`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitRule {
    /// Limit bucket type.
    pub rate_limit_type: RateLimitType,
    /// Window unit.
    pub interval: RateLimitInterval,
    /// Number of interval units.
    pub interval_num: u32,
    /// Ceiling for this window.
    pub limit: u64,
}

/// Live rate limit usage entry returned in WS API responses (`rateLimits[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RateLimitReading {
    /// Limit bucket type.
    pub rate_limit_type: RateLimitType,
    /// Window unit.
    pub interval: RateLimitInterval,
    /// Number of interval units.
    pub interval_num: u32,
    /// Ceiling for this window.
    pub limit: u64,
    /// Current usage reported by the exchange.
    pub count: u64,
}

/// Composite key identifying a specific rate-limit bucket.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RateLimitBucketKey {
    /// Bucket type.
    pub rate_limit_type: RateLimitType,
    /// Interval unit.
    pub interval: RateLimitInterval,
    /// Interval multiplier.
    pub interval_num: u32,
}

/// Snapshot of a single tracked rate limit bucket.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RateLimitBucketState {
    /// Limit ceiling.
    pub limit: u64,
    /// Latest reported usage count.
    pub count: u64,
}

#[derive(Debug, Default)]
struct TrackerInner {
    buckets: HashMap<RateLimitBucketKey, RateLimitBucketState>,
    retry_after_ms: Option<i64>,
    ip_banned: bool,
}

/// Thread-safe per-venue rate limit tracker updated from `exchangeInfo` and WS responses.
#[derive(Debug, Default)]
pub struct RateLimitTracker {
    inner: Mutex<TrackerInner>,
}

impl RateLimitTracker {
    /// Create an empty rate limit tracker.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Seed or update bucket ceilings from `exchangeInfo.rateLimits`.
    pub fn apply_rules(&self, rules: &[RateLimitRule]) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        for rule in rules {
            let key = RateLimitBucketKey {
                rate_limit_type: rule.rate_limit_type.clone(),
                interval: rule.interval.clone(),
                interval_num: rule.interval_num,
            };
            guard
                .buckets
                .entry(key)
                .and_modify(|s| s.limit = rule.limit)
                .or_insert(RateLimitBucketState {
                    limit: rule.limit,
                    count: 0,
                });
        }
    }

    /// Update live usage counts from a WS API response's `rateLimits` array.
    pub fn record_readings(&self, readings: &[RateLimitReading]) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        for r in readings {
            let key = RateLimitBucketKey {
                rate_limit_type: r.rate_limit_type.clone(),
                interval: r.interval.clone(),
                interval_num: r.interval_num,
            };
            guard.buckets.insert(
                key,
                RateLimitBucketState {
                    limit: r.limit,
                    count: r.count,
                },
            );
        }
    }

    /// Record a 429 rate-limit or 418 IP-ban `retryAfter` UTC timestamp (ms).
    ///
    /// Keeps the furthest future timestamp if multiple responses arrive concurrently.
    pub fn record_retry_after(&self, retry_after_ms: i64, is_ip_ban: bool) {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        guard.retry_after_ms = Some(match guard.retry_after_ms {
            Some(prev) => prev.max(retry_after_ms),
            None => retry_after_ms,
        });
        if is_ip_ban {
            guard.ip_banned = true;
        }
    }

    /// Return the remaining wait duration if currently throttled/banned at `now_ms`.
    #[must_use]
    pub fn retry_after_delay(&self, now_ms: i64) -> Option<Duration> {
        let Ok(guard) = self.inner.lock() else {
            return None;
        };
        let target = guard.retry_after_ms?;
        if target > now_ms {
            let diff = u64::try_from(target.saturating_sub(now_ms)).ok()?;
            Some(Duration::from_millis(diff))
        } else {
            None
        }
    }

    /// Returns `true` if a 418 IP ban is still active at `now_ms`.
    #[must_use]
    pub fn is_ip_banned(&self, now_ms: i64) -> bool {
        let Ok(guard) = self.inner.lock() else {
            return false;
        };
        guard.ip_banned && guard.retry_after_ms.is_some_and(|t| t > now_ms)
    }

    /// Return a copy of all currently tracked buckets.
    #[must_use]
    pub fn snapshot(&self) -> HashMap<RateLimitBucketKey, RateLimitBucketState> {
        self.inner
            .lock()
            .map(|g| g.buckets.clone())
            .unwrap_or_default()
    }
}

/// Exponential backoff with full jitter (§4.2: 250 ms → 30 s) honoring `retryAfter`.
#[derive(Debug, Clone)]
pub struct ReconnectBackoff {
    base: Duration,
    max: Duration,
    attempt: u32,
}

impl Default for ReconnectBackoff {
    fn default() -> Self {
        Self::new(Duration::from_millis(250), Duration::from_secs(30))
    }
}

impl ReconnectBackoff {
    /// Create a backoff schedule bounded by `[0, max]`.
    #[must_use]
    pub const fn new(base: Duration, max: Duration) -> Self {
        Self {
            base,
            max,
            attempt: 0,
        }
    }

    /// Reset backoff state after a healthy connection is established.
    pub const fn reset(&mut self) {
        self.attempt = 0;
    }

    /// Compute the deterministic upper cap for the current attempt (before jitter).
    #[must_use]
    pub fn current_cap(&self) -> Duration {
        let shift = self.attempt.min(16);
        let mult = 1u32.checked_shl(shift).unwrap_or(u32::MAX);
        self.base.saturating_mul(mult).min(self.max)
    }

    /// Advance the attempt counter and return the next delay using full jitter,
    /// always honoring `retry_after` if it exceeds the jittered delay.
    pub fn next_delay(&mut self, retry_after: Option<Duration>) -> Duration {
        let cap = self.current_cap();
        self.attempt = self.attempt.saturating_add(1);
        let cap_ms = u64::try_from(cap.as_millis()).unwrap_or(u64::MAX);
        let jittered_ms = if cap_ms == 0 {
            0
        } else {
            rand::rng().random_range(0..=cap_ms)
        };
        let jittered = Duration::from_millis(jittered_ms);
        match retry_after {
            Some(ra) if ra > jittered => ra,
            _ => jittered,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{
        RateLimitBucketKey, RateLimitInterval, RateLimitReading, RateLimitRule, RateLimitTracker,
        RateLimitType, ReconnectBackoff,
    };

    #[test]
    fn tracker_updates_rules_readings_and_retry_after() {
        let tracker = RateLimitTracker::new();
        tracker.apply_rules(&[RateLimitRule {
            rate_limit_type: RateLimitType::RequestWeight,
            interval: RateLimitInterval::Minute,
            interval_num: 1,
            limit: 6000,
        }]);
        tracker.record_readings(&[RateLimitReading {
            rate_limit_type: RateLimitType::RequestWeight,
            interval: RateLimitInterval::Minute,
            interval_num: 1,
            limit: 6000,
            count: 42,
        }]);

        let snap = tracker.snapshot();
        let key = RateLimitBucketKey {
            rate_limit_type: RateLimitType::RequestWeight,
            interval: RateLimitInterval::Minute,
            interval_num: 1,
        };
        let state = snap.get(&key).expect("bucket present");
        assert_eq!(state.limit, 6000);
        assert_eq!(state.count, 42);

        tracker.record_retry_after(10_500, false);
        assert_eq!(
            tracker.retry_after_delay(10_000),
            Some(Duration::from_millis(500))
        );
        assert_eq!(tracker.retry_after_delay(10_500), None);
        assert!(!tracker.is_ip_banned(10_000));

        tracker.record_retry_after(20_000, true);
        assert!(tracker.is_ip_banned(15_000));
        assert!(!tracker.is_ip_banned(20_001));
    }

    #[test]
    fn backoff_scales_exponentially_and_honors_retry_after() {
        let mut backoff = ReconnectBackoff::default();
        assert_eq!(backoff.current_cap(), Duration::from_millis(250));
        let d0 = backoff.next_delay(None);
        assert!(d0 <= Duration::from_millis(250));

        assert_eq!(backoff.current_cap(), Duration::from_millis(500));
        let forced = backoff.next_delay(Some(Duration::from_secs(12)));
        assert_eq!(forced, Duration::from_secs(12));

        for _ in 0..20 {
            let _ = backoff.next_delay(None);
        }
        assert_eq!(backoff.current_cap(), Duration::from_secs(30));

        backoff.reset();
        assert_eq!(backoff.current_cap(), Duration::from_millis(250));
    }
}
