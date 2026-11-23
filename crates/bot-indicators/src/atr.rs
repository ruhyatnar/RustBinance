//! Incremental O(1) Wilder Average True Range (§6.3).

use bot_types::Kline;

/// Wilder-smoothed Average True Range (`warmup_bars = period + 1`, matching TA-Lib).
#[derive(Debug, Clone)]
pub struct Atr {
    period: usize,
    count: usize,
    prev_close: f64,
    sum_tr: f64,
    current: Option<f64>,
}

impl Atr {
    /// Construct an `Atr(period)` (`period` clamped to `>= 1`, default 14).
    #[must_use]
    pub fn new(period: usize) -> Self {
        Self {
            period: period.max(1),
            count: 0,
            prev_close: 0.0,
            sum_tr: 0.0,
            current: None,
        }
    }

    /// Configured ATR period `n`.
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

    /// Latest ATR value, or `None` if still warming up.
    #[must_use]
    pub const fn value(&self) -> Option<f64> {
        self.current
    }

    /// Update ATR with a closed bar in O(1) without allocating.
    #[allow(clippy::cast_precision_loss)]
    pub fn update(&mut self, bar: &Kline) -> Option<f64> {
        if self.count == 0 {
            self.prev_close = bar.close;
            self.count = 1;
            return None;
        }

        let tr = true_range(bar.high, bar.low, self.prev_close);
        self.prev_close = bar.close;

        let tr_idx = self.count; // 1-based TR sample index
        self.count = self.count.saturating_add(1);

        let p_f64 = self.period as f64;
        if tr_idx < self.period {
            self.sum_tr += tr;
            return None;
        }
        if tr_idx == self.period {
            self.sum_tr += tr;
            self.current = Some(self.sum_tr / p_f64);
        } else if let Some(prev_atr) = self.current {
            self.current = Some((prev_atr * (p_f64 - 1.0) + tr) / p_f64);
        }
        self.current
    }
}

/// Compute True Range `max(H - L, |H - prev_close|, |L - prev_close|)`.
#[inline]
#[must_use]
pub fn true_range(high: f64, low: f64, prev_close: f64) -> f64 {
    let hl = high - low;
    let hc = (high - prev_close).abs();
    let lc = (low - prev_close).abs();
    hl.max(hc).max(lc)
}
