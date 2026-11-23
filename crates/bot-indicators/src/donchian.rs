//! Incremental amortized O(1) Donchian channel over the previous `n` bars (§6.3).
//!
//! Strictly excludes the current/signal bar to prevent look-ahead bias (`warmup_bars = n + 1`).

use bot_types::Kline;
use serde::{Deserialize, Serialize};

/// Output of [`Donchian`] channel over the previous `n` bars (excluding the current bar).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DonchianValue {
    /// Highest high of the previous `n` bars.
    pub upper: f64,
    /// Lowest low of the previous `n` bars.
    pub lower: f64,
    /// Midpoint `(upper + lower) / 2`.
    pub mid: f64,
}

/// Fixed-capacity circular monotonic deque with zero allocations during `update`.
#[derive(Debug, Clone)]
struct FixedMonoDeque {
    buf: Box<[(u64, f64)]>,
    head: usize,
    len: usize,
    is_max: bool,
}

impl FixedMonoDeque {
    fn new(capacity: usize, is_max: bool) -> Self {
        let cap = capacity.max(1);
        Self {
            buf: vec![(0, 0.0); cap].into_boxed_slice(),
            head: 0,
            len: 0,
            is_max,
        }
    }

    fn cap(&self) -> usize {
        self.buf.len()
    }

    fn front(&self) -> Option<(u64, f64)> {
        if self.len == 0 {
            return None;
        }
        self.buf.get(self.head).copied()
    }

    fn back(&self) -> Option<(u64, f64)> {
        if self.len == 0 {
            return None;
        }
        let idx = (self.head + self.len - 1) % self.cap();
        self.buf.get(idx).copied()
    }

    fn pop_front(&mut self) {
        if self.len > 0 {
            self.head = (self.head + 1) % self.cap();
            self.len -= 1;
        }
    }

    fn pop_back(&mut self) {
        if self.len > 0 {
            self.len -= 1;
        }
    }

    fn evict_older_than(&mut self, min_idx: u64) {
        while let Some((idx, _)) = self.front() {
            if idx < min_idx {
                self.pop_front();
            } else {
                break;
            }
        }
    }

    fn push(&mut self, index: u64, value: f64) {
        while let Some((_, back_val)) = self.back() {
            let dominated = if self.is_max {
                back_val <= value
            } else {
                back_val >= value
            };
            if dominated {
                self.pop_back();
            } else {
                break;
            }
        }
        // Capacity equals window length; after evicting older indices there is always room.
        if self.len == self.cap() {
            self.pop_front();
        }
        let tail = (self.head + self.len) % self.cap();
        if let Some(slot) = self.buf.get_mut(tail) {
            *slot = (index, value);
            self.len += 1;
        }
    }
}

/// Amortized O(1), zero-allocation-per-update Donchian channel over the previous `n` bars.
#[derive(Debug, Clone)]
pub struct Donchian {
    period: usize,
    index: u64,
    max_high: FixedMonoDeque,
    min_low: FixedMonoDeque,
    current: Option<DonchianValue>,
}

impl Donchian {
    /// Construct a `Donchian(period)` (`period` clamped to `>= 1`, default 20).
    #[must_use]
    pub fn new(period: usize) -> Self {
        let p = period.max(1);
        Self {
            period: p,
            index: 0,
            max_high: FixedMonoDeque::new(p, true),
            min_low: FixedMonoDeque::new(p, false),
            current: None,
        }
    }

    /// Configured lookback period `n`.
    #[must_use]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Number of bars required before [`Self::update`] returns `Some` (`period + 1`,
    /// because the current signal bar is excluded from its own channel).
    #[must_use]
    pub const fn warmup_bars(&self) -> usize {
        self.period.saturating_add(1)
    }

    /// Returns `true` once warm-up is satisfied.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.current.is_some()
    }

    /// Latest [`DonchianValue`], or `None` if still warming up.
    #[must_use]
    pub const fn value(&self) -> Option<DonchianValue> {
        self.current
    }

    /// Update Donchian with a closed bar in amortized O(1) without allocating.
    ///
    /// Computes `(upper, lower, mid)` strictly from the **previous `n` bars** BEFORE
    /// inserting `bar` into the rolling window for subsequent bars.
    pub fn update(&mut self, bar: &Kline) -> Option<DonchianValue> {
        let period_u64 = u64::try_from(self.period).unwrap_or(u64::MAX);

        if self.index >= period_u64 {
            let min_idx = self.index - period_u64;
            self.max_high.evict_older_than(min_idx);
            self.min_low.evict_older_than(min_idx);
            if let (Some((_, upper)), Some((_, lower))) =
                (self.max_high.front(), self.min_low.front())
            {
                self.current = Some(DonchianValue {
                    upper,
                    lower,
                    mid: f64::midpoint(upper, lower),
                });
            }
        }

        // Insert current bar into window for future bars.
        let min_next = self.index.saturating_add(1).saturating_sub(period_u64);
        self.max_high.evict_older_than(min_next);
        self.min_low.evict_older_than(min_next);
        self.max_high.push(self.index, bar.high);
        self.min_low.push(self.index, bar.low);
        self.index = self.index.saturating_add(1);

        self.current
    }
}
