//! Incremental O(1) Volume Simple Moving Average (§6.3).

use bot_types::Kline;

/// O(1) zero-allocation-per-update Simple Moving Average of closed-bar volume (`warmup_bars = n`).
#[derive(Debug, Clone)]
pub struct VolumeSma {
    buf: Box<[f64]>,
    cursor: usize,
    count: usize,
    sum: f64,
    current: Option<f64>,
}

impl VolumeSma {
    /// Construct a `VolumeSma(period)` (`period` clamped to `>= 1`, default 20).
    #[must_use]
    pub fn new(period: usize) -> Self {
        let p = period.max(1);
        Self {
            buf: vec![0.0; p].into_boxed_slice(),
            cursor: 0,
            count: 0,
            sum: 0.0,
            current: None,
        }
    }

    /// Configured SMA period `n`.
    #[must_use]
    pub fn period(&self) -> usize {
        self.buf.len()
    }

    /// Number of bars required before [`Self::update`] returns `Some` (`period`).
    #[must_use]
    pub fn warmup_bars(&self) -> usize {
        self.buf.len()
    }

    /// Returns `true` once warm-up is satisfied.
    #[must_use]
    pub const fn is_ready(&self) -> bool {
        self.current.is_some()
    }

    /// Latest volume SMA, or `None` if still warming up.
    #[must_use]
    pub const fn value(&self) -> Option<f64> {
        self.current
    }

    /// Update Volume SMA with a closed bar in O(1) without allocating.
    #[allow(clippy::cast_precision_loss)]
    pub fn update(&mut self, bar: &Kline) -> Option<f64> {
        let period = self.buf.len();
        let vol = bar.volume;

        if self.count < period {
            if let Some(slot) = self.buf.get_mut(self.cursor) {
                *slot = vol;
            }
            self.sum += vol;
            self.count += 1;
            self.cursor = (self.cursor + 1) % period;
        } else {
            if let Some(slot) = self.buf.get_mut(self.cursor) {
                self.sum = self.sum - *slot + vol;
                *slot = vol;
            }
            self.cursor = (self.cursor + 1) % period;
        }

        if self.count >= period {
            self.current = Some(self.sum / (period as f64));
        }
        self.current
    }
}
