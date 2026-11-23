//! Incremental O(1) Exponential Moving Average with SMA seeding (§6.3).

use bot_types::Kline;

/// Exponential Moving Average (`α = 2 / (n + 1)`), seeded by the SMA of the first `n` bars,
/// with warm-up `3 * n` bars per §6.3 (e.g. EMA(200) requires 600 bars).
#[derive(Debug, Clone)]
pub struct Ema {
    period: usize,
    warmup: usize,
    alpha: f64,
    count: usize,
    seed_sum: f64,
    current: f64,
}

impl Ema {
    /// Construct an `Ema(period)` (`period` clamped to `>= 1`).
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn new(period: usize) -> Self {
        let p = period.max(1);
        Self {
            period: p,
            warmup: p.saturating_mul(3),
            alpha: 2.0 / (p as f64 + 1.0),
            count: 0,
            seed_sum: 0.0,
            current: 0.0,
        }
    }

    /// Configured EMA period `n`.
    #[must_use]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Number of bars required before [`Self::update`] returns `Some` (`3 * period`).
    #[must_use]
    pub const fn warmup_bars(&self) -> usize {
        self.warmup
    }

    /// Returns `true` once warm-up is satisfied.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.count >= self.warmup
    }

    /// Current warmed-up EMA value, or `None` if still warming up.
    #[must_use]
    pub const fn value(&self) -> Option<f64> {
        if self.count >= self.warmup {
            Some(self.current)
        } else {
            None
        }
    }

    /// Update EMA with a scalar value in O(1) without allocating.
    #[allow(clippy::cast_precision_loss)]
    pub fn update_value(&mut self, x: f64) -> Option<f64> {
        self.count = self.count.saturating_add(1);
        if self.count < self.period {
            self.seed_sum += x;
            return None;
        }
        if self.count == self.period {
            self.seed_sum += x;
            self.current = self.seed_sum / (self.period as f64);
        } else {
            self.current = self.alpha * x + (1.0 - self.alpha) * self.current;
        }
        self.value()
    }

    /// Update EMA with `bar.close` in O(1) without allocating.
    pub fn update(&mut self, bar: &Kline) -> Option<f64> {
        self.update_value(bar.close)
    }
}
