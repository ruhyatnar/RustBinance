//! Incremental O(1) Wilder Relative Strength Index (§6.3).

use bot_types::Kline;

/// Wilder-smoothed Relative Strength Index (`α = 1 / n`, `warmup_bars = n + 1`).
#[derive(Debug, Clone)]
pub struct Rsi {
    period: usize,
    count: usize,
    prev_close: f64,
    sum_gain: f64,
    sum_loss: f64,
    avg_gain: f64,
    avg_loss: f64,
    current: Option<f64>,
}

impl Rsi {
    /// Construct an `Rsi(period)` (`period` clamped to `>= 1`, default 14).
    #[must_use]
    pub fn new(period: usize) -> Self {
        Self {
            period: period.max(1),
            count: 0,
            prev_close: 0.0,
            sum_gain: 0.0,
            sum_loss: 0.0,
            avg_gain: 0.0,
            avg_loss: 0.0,
            current: None,
        }
    }

    /// Configured RSI period `n`.
    #[must_use]
    pub const fn period(&self) -> usize {
        self.period
    }

    /// Number of bars required before [`Self::update`] returns `Some` (`period + 1`).
    #[must_use]
    pub const fn warmup_bars(&self) -> usize {
        self.period.saturating_add(1)
    }

    /// Returns `true` once warm-up is satisfied.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.current.is_some()
    }

    /// Latest RSI value in `[0.0, 100.0]`, or `None` if still warming up.
    #[must_use]
    pub const fn value(&self) -> Option<f64> {
        self.current
    }

    /// Update RSI with a closed bar in O(1) without allocating.
    #[allow(clippy::cast_precision_loss)]
    pub fn update(&mut self, bar: &Kline) -> Option<f64> {
        let close = bar.close;
        if self.count == 0 {
            self.prev_close = close;
            self.count = 1;
            return None;
        }

        let delta = close - self.prev_close;
        self.prev_close = close;
        let gain = if delta > 0.0 { delta } else { 0.0 };
        let loss = if delta < 0.0 { -delta } else { 0.0 };

        let diff_idx = self.count; // 1-based index of price change
        self.count = self.count.saturating_add(1);

        let p_f64 = self.period as f64;
        if diff_idx < self.period {
            self.sum_gain += gain;
            self.sum_loss += loss;
            return None;
        }
        if diff_idx == self.period {
            self.sum_gain += gain;
            self.sum_loss += loss;
            self.avg_gain = self.sum_gain / p_f64;
            self.avg_loss = self.sum_loss / p_f64;
        } else {
            self.avg_gain = (self.avg_gain * (p_f64 - 1.0) + gain) / p_f64;
            self.avg_loss = (self.avg_loss * (p_f64 - 1.0) + loss) / p_f64;
        }

        let rsi = if self.avg_loss == 0.0 && self.avg_gain == 0.0 {
            50.0
        } else if self.avg_loss == 0.0 {
            100.0
        } else {
            let rs = self.avg_gain / self.avg_loss;
            100.0 - 100.0 / (1.0 + rs)
        };
        self.current = Some(rsi);
        self.current
    }
}
