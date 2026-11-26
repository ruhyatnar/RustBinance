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

/// Error returned when placing an order would exceed or collide with client-side rate budget.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "rate budget exceeded for {:?} ({} * {:?}): count={} limit={}",
    bucket.rate_limit_type,
    bucket.interval_num,
    bucket.interval,
    count,
    limit
)]
pub struct RateBudgetExceeded {
    /// Rate-limit bucket that triggered the block (or `Unknown` for an active 429/418 cooldown).
    pub bucket: RateLimitBucketKey,
    /// Observed usage count (`0` when blocked by an active 429/418 cooldown).
    pub count: u64,
    /// Configured bucket ceiling (`0` when blocked by an active 429/418 cooldown).
    pub limit: u64,
}

fn parse_rest_rate_limit_header(header_name: &str) -> Option<RateLimitBucketKey> {
    let lower = header_name.trim().to_ascii_lowercase();
    let (rate_limit_type, suffix) = if let Some(s) = lower.strip_prefix("x-mbx-used-weight-") {
        (RateLimitType::RequestWeight, s)
    } else {
        let s = lower.strip_prefix("x-mbx-order-count-")?;
        (RateLimitType::Orders, s)
    };

    let mut chars = suffix.chars();
    let unit_char = chars.next_back()?;
    let num_str = chars.as_str();
    if num_str.is_empty() {
        return None;
    }
    let interval_num: u32 = num_str.parse().ok()?;
    if interval_num == 0 {
        return None;
    }
    let interval = match unit_char {
        's' => RateLimitInterval::Second,
        'm' => RateLimitInterval::Minute,
        'd' => RateLimitInterval::Day,
        _ => return None,
    };

    Some(RateLimitBucketKey {
        rate_limit_type,
        interval,
        interval_num,
    })
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

    /// Record a single Binance REST rate-limit header (`x-mbx-used-weight-*` or
    /// `x-mbx-order-count-*`), preserving any seeded `limit` (or `0` when unseeded).
    pub fn record_rest_header(&self, header_name: &str, value: &str) {
        self.record_rest_headers([(header_name, value)]);
    }

    /// Record a batch of Binance REST rate-limit headers from an HTTP response.
    ///
    /// Unrecognized header names and non-integer values are ignored cleanly.
    pub fn record_rest_headers<'a, I>(&self, headers: I)
    where
        I: IntoIterator<Item = (&'a str, &'a str)>,
    {
        let Ok(mut guard) = self.inner.lock() else {
            return;
        };
        for (name, value) in headers {
            let Some(key) = parse_rest_rate_limit_header(name) else {
                continue;
            };
            let Ok(count) = value.trim().parse::<u64>() else {
                continue;
            };
            guard
                .buckets
                .entry(key)
                .and_modify(|s| s.count = count)
                .or_insert(RateLimitBucketState { limit: 0, count });
        }
    }

    /// Check whether a new order can be placed at `now_ms` without breaching `safety_margin_pct`
    /// on any seeded `Orders` bucket or violating an active 429/418 cooldown.
    ///
    /// # Errors
    /// Returns [`RateBudgetExceeded`] when an IP ban / `retryAfter` cooldown is active at
    /// `now_ms`, or when any `Orders` bucket with `limit > 0` satisfies
    /// `count * 100 >= limit * safety_margin_pct`.
    pub fn can_place_order(
        &self,
        now_ms: i64,
        safety_margin_pct: u8,
    ) -> Result<(), RateBudgetExceeded> {
        let Ok(guard) = self.inner.lock() else {
            return Err(RateBudgetExceeded {
                bucket: RateLimitBucketKey {
                    rate_limit_type: RateLimitType::Unknown,
                    interval: RateLimitInterval::Unknown,
                    interval_num: 0,
                },
                count: 0,
                limit: 0,
            });
        };

        let cooldown_active = guard.retry_after_ms.is_some_and(|t| t > now_ms);
        if cooldown_active {
            return Err(RateBudgetExceeded {
                bucket: RateLimitBucketKey {
                    rate_limit_type: RateLimitType::Unknown,
                    interval: RateLimitInterval::Unknown,
                    interval_num: 0,
                },
                count: 0,
                limit: 0,
            });
        }

        let margin = u64::from(safety_margin_pct);
        for (key, state) in &guard.buckets {
            if key.rate_limit_type == RateLimitType::Orders
                && state.limit > 0
                && state.count.saturating_mul(100) >= state.limit.saturating_mul(margin)
            {
                return Err(RateBudgetExceeded {
                    bucket: key.clone(),
                    count: state.count,
                    limit: state.limit,
                });
            }
        }

        Ok(())
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

    #[test]
    fn rest_headers_update_buckets_and_enforce_order_budget() {
        let tracker = RateLimitTracker::new();
        // Review Focus #2: headers before rules must not panic or block with limit=0.
        tracker.record_rest_headers([
            ("x-mbx-used-weight-1m", "120"),
            ("x-mbx-order-count-10s", "45"),
        ]);
        assert!(tracker.can_place_order(10_000, 90).is_ok());

        tracker.apply_rules(&[RateLimitRule {
            rate_limit_type: RateLimitType::Orders,
            interval: RateLimitInterval::Second,
            interval_num: 10,
            limit: 50,
        }]);
        // 45 / 50 == 90% -> blocked at 90% safety margin.
        let err = tracker.can_place_order(10_000, 90).unwrap_err();
        assert_eq!(err.count, 45);
        assert_eq!(err.limit, 50);

        // Lower count below threshold -> allowed.
        tracker.record_rest_header("X-MBX-ORDER-COUNT-10S", "44");
        assert!(tracker.can_place_order(10_000, 90).is_ok());
    }

    #[test]
    fn rest_header_parser_handles_units_case_and_invalid_inputs() {
        let tracker = RateLimitTracker::new();
        tracker.record_rest_headers([
            ("X-MBX-USED-WEIGHT-5M", "250"),
            ("x-mbx-order-count-1d", "12"),
            ("x-mbx-used-weight-0m", "99"),
            ("x-mbx-used-weight-1x", "99"),
            ("x-mbx-order-count-m", "99"),
            ("x-mbx-used-weight-1m", "not_a_num"),
            ("content-type", "application/json"),
        ]);

        let snap = tracker.snapshot();
        assert_eq!(snap.len(), 2);
        assert_eq!(
            snap.get(&RateLimitBucketKey {
                rate_limit_type: RateLimitType::RequestWeight,
                interval: RateLimitInterval::Minute,
                interval_num: 5,
            })
            .expect("5m weight")
            .count,
            250
        );
        assert_eq!(
            snap.get(&RateLimitBucketKey {
                rate_limit_type: RateLimitType::Orders,
                interval: RateLimitInterval::Day,
                interval_num: 1,
            })
            .expect("1d orders")
            .count,
            12
        );
    }

    #[test]
    fn can_place_order_blocks_during_retry_after_and_ip_ban() {
        let tracker = RateLimitTracker::new();
        tracker.record_retry_after(20_000, false);
        assert!(tracker.can_place_order(19_999, 90).is_err());
        assert!(tracker.can_place_order(20_000, 90).is_ok());

        tracker.record_retry_after(30_000, true);
        assert!(tracker.can_place_order(25_000, 90).is_err());
        assert!(tracker.can_place_order(30_000, 90).is_ok());
    }
}
