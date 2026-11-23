//! Incremental O(1) Wilder ADX, +DI, and −DI (§6.3).

use bot_types::Kline;
use serde::{Deserialize, Serialize};

use crate::atr::true_range;

/// Output of [`Adx`] containing Wilder-smoothed ADX, +DI, and −DI.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct AdxValue {
    /// Average Directional Index in `[0.0, 100.0]`.
    pub adx: f64,
    /// Positive Directional Indicator (`+DI`) in `[0.0, 100.0]`.
    pub plus_di: f64,
    /// Negative Directional Indicator (`-DI`) in `[0.0, 100.0]`.
    pub minus_di: f64,
}

/// Wilder-smoothed ADX / +DI / −DI (`warmup_bars = 2 * period`, matching TA-Lib).
#[derive(Debug, Clone)]
pub struct Adx {
    period: usize,
    count: usize,
    prev_high: f64,
    prev_low: f64,
    prev_close: f64,
    sm_tr: f64,
    sm_plus_dm: f64,
    sm_minus_dm: f64,
    dx_sum: f64,
    dx_count: usize,
    current: Option<AdxValue>,
}

impl Adx {
    /// Construct an `Adx(period)` (`period` clamped to `>= 1`, default 14).
    #[must_use]
    pub fn new(period: usize) -> Self {
        Self {
            period: period.max(1),
            count: 0,
            prev_high: 0.0,
            prev_low: 0.0,
            prev_close: 0.0,
            sm_tr: 0.0,
            sm_plus_dm: 0.0,
            sm_minus_dm: 0.0,
            dx_sum: 0.0,
            dx_count: 0,
            current: None,
        }
    }

    /// Configured ADX period `n`.
    #[must_use]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Number of bars required before [`Self::update`] returns `Some` (`2 * period`).
    #[must_use]
    pub const fn warmup_bars(&self) -> usize {
        self.period.saturating_mul(2)
    }

    /// Returns `true` once warm-up is satisfied.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.current.is_some()
    }

    /// Latest [`AdxValue`], or `None` if still warming up.
    #[must_use]
    pub const fn value(&self) -> Option<AdxValue> {
        self.current
    }

    /// Update ADX / +DI / −DI with a closed bar in O(1) without allocating.
    #[allow(clippy::cast_precision_loss, clippy::similar_names)]
    pub fn update(&mut self, bar: &Kline) -> Option<AdxValue> {
        if self.count == 0 {
            self.prev_high = bar.high;
            self.prev_low = bar.low;
            self.prev_close = bar.close;
            self.count = 1;
            return None;
        }

        let up_move = bar.high - self.prev_high;
        let down_move = self.prev_low - bar.low;
        let plus_dm = if up_move > down_move && up_move > 0.0 {
            up_move
        } else {
            0.0
        };
        let minus_dm = if down_move > up_move && down_move > 0.0 {
            down_move
        } else {
            0.0
        };
        let tr = true_range(bar.high, bar.low, self.prev_close);

        self.prev_high = bar.high;
        self.prev_low = bar.low;
        self.prev_close = bar.close;

        let sample_idx = self.count; // 1-based sample index
        self.count = self.count.saturating_add(1);
        let p_f64 = self.period as f64;

        if sample_idx < self.period {
            self.sm_tr += tr;
            self.sm_plus_dm += plus_dm;
            self.sm_minus_dm += minus_dm;
            return None;
        }

        if sample_idx == self.period {
            self.sm_tr += tr;
            self.sm_plus_dm += plus_dm;
            self.sm_minus_dm += minus_dm;
        } else {
            self.sm_tr = self.sm_tr - (self.sm_tr / p_f64) + tr;
            self.sm_plus_dm = self.sm_plus_dm - (self.sm_plus_dm / p_f64) + plus_dm;
            self.sm_minus_dm = self.sm_minus_dm - (self.sm_minus_dm / p_f64) + minus_dm;
        }

        let plus_di = if self.sm_tr > 0.0 {
            100.0 * self.sm_plus_dm / self.sm_tr
        } else {
            0.0
        };
        let minus_di = if self.sm_tr > 0.0 {
            100.0 * self.sm_minus_dm / self.sm_tr
        } else {
            0.0
        };
        let di_sum = plus_di + minus_di;
        let dx = if di_sum > 0.0 {
            100.0 * (plus_di - minus_di).abs() / di_sum
        } else {
            0.0
        };

        self.dx_count = self.dx_count.saturating_add(1);
        if self.dx_count < self.period {
            self.dx_sum += dx;
            return None;
        }
        if self.dx_count == self.period {
            self.dx_sum += dx;
            let adx = self.dx_sum / p_f64;
            self.current = Some(AdxValue {
                adx,
                plus_di,
                minus_di,
            });
        } else if let Some(prev) = self.current {
            let adx = (prev.adx * (p_f64 - 1.0) + dx) / p_f64;
            self.current = Some(AdxValue {
                adx,
                plus_di,
                minus_di,
            });
        }
        self.current
    }
}
