//! Incremental O(1) UTC Session VWAP resetting at 00:00 UTC (§6.3).

use bot_types::Kline;

/// Milliseconds per UTC day (`24 * 60 * 60 * 1000`).
const MS_PER_DAY: i64 = 86_400_000;

/// Session Volume-Weighted Average Price (`Σ(typical × vol) / Σvol`), resetting daily at 00:00 UTC.
#[derive(Debug, Clone, Default)]
pub struct SessionVwap {
    current_day: Option<i64>,
    cum_tp_vol: f64,
    cum_vol: f64,
    current: Option<f64>,
}

impl SessionVwap {
    /// Construct a fresh [`SessionVwap`].
    #[must_use]
    pub const fn new() -> Self {
        Self {
            current_day: None,
            cum_tp_vol: 0.0,
            cum_vol: 0.0,
            current: None,
        }
    }

    /// Number of bars required before [`Self::update`] returns `Some` (`1`).
    #[must_use]
    pub const fn warmup_bars(&self) -> usize {
        1
    }

    /// Returns `true` once a positive-volume bar has been ingested in the current session.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.current.is_some()
    }

    /// Current session VWAP, or `None` if no positive volume has occurred yet in the session.
    #[must_use]
    pub const fn value(&self) -> Option<f64> {
        self.current
    }

    /// Update Session VWAP with a closed bar in O(1) without allocating.
    pub fn update(&mut self, bar: &Kline) -> Option<f64> {
        let day = bar.open_time_ms.div_euclid(MS_PER_DAY);
        if self.current_day != Some(day) {
            self.current_day = Some(day);
            self.cum_tp_vol = 0.0;
            self.cum_vol = 0.0;
            self.current = None;
        }

        let typical = (bar.high + bar.low + bar.close) / 3.0;
        self.cum_tp_vol += typical * bar.volume;
        self.cum_vol += bar.volume;

        if self.cum_vol > 0.0 {
            self.current = Some(self.cum_tp_vol / self.cum_vol);
        }
        self.current
    }
}
