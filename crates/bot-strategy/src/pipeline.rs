//! Incremental zero-lookahead indicator feed (`bot-indicators` → `IndicatorSnapshot` & `HtfRegime`).

use std::collections::VecDeque;

use bot_indicators::{Adx, Atr, Donchian, Ema, Rsi, SessionVwap, VolumeSma};
use bot_types::Kline;

use crate::types::{HtfRegime, IndicatorSnapshot};

/// Incremental O(1) per-symbol indicator pipeline strictly consuming **closed** bars (§6.3).
#[derive(Debug, Clone)]
pub struct IndicatorFeed {
    ema9: Ema,
    ema21: Ema,
    ema55: Ema,
    ema200: Ema,
    rsi14: Rsi,
    prev_rsi14: Option<f64>,
    atr14: Atr,
    adx14: Adx,
    prev_adx14: Option<f64>,
    donchian20: Donchian,
    vwap: SessionVwap,
    vol_sma20: VolumeSma,
    recent_lows: VecDeque<f64>,
    recent_highs: VecDeque<f64>,
}

impl Default for IndicatorFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl IndicatorFeed {
    /// Construct a fresh [`IndicatorFeed`] with standard §6.3/§6.4 periods.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ema9: Ema::new(9),
            ema21: Ema::new(21),
            ema55: Ema::new(55),
            ema200: Ema::new(200),
            rsi14: Rsi::new(14),
            prev_rsi14: None,
            atr14: Atr::new(14),
            adx14: Adx::new(14),
            prev_adx14: None,
            donchian20: Donchian::new(20),
            vwap: SessionVwap::new(),
            vol_sma20: VolumeSma::new(20),
            recent_lows: VecDeque::with_capacity(6),
            recent_highs: VecDeque::with_capacity(6),
        }
    }

    /// Update all indicators with one **closed** bar and return the new [`IndicatorSnapshot`].
    ///
    /// Non-closed bars (`!bar.is_closed`) are ignored and return the previous snapshot without mutating state.
    pub fn on_closed_bar(&mut self, bar: &Kline) -> IndicatorSnapshot {
        if !bar.is_closed {
            return self.snapshot();
        }

        self.prev_rsi14 = self.rsi14.value();
        self.prev_adx14 = self.adx14.value().map(|v| v.adx);

        let _ = self.ema9.update(bar);
        let _ = self.ema21.update(bar);
        let _ = self.ema55.update(bar);
        let _ = self.ema200.update(bar);
        let _ = self.rsi14.update(bar);
        let _ = self.atr14.update(bar);
        let _ = self.adx14.update(bar);
        let _ = self.donchian20.update(bar);
        let _ = self.vwap.update(bar);
        let _ = self.vol_sma20.update(bar);

        if self.recent_lows.len() >= 5 {
            let _ = self.recent_lows.pop_front();
        }
        self.recent_lows.push_back(bar.low);

        if self.recent_highs.len() >= 5 {
            let _ = self.recent_highs.pop_front();
        }
        self.recent_highs.push_back(bar.high);

        self.snapshot()
    }

    /// Return the current [`IndicatorSnapshot`] without mutating state.
    #[must_use]
    pub fn snapshot(&self) -> IndicatorSnapshot {
        let adx = self.adx14.value();
        let dc = self.donchian20.value();
        let min_low_3 = self
            .recent_lows
            .iter()
            .rev()
            .take(3)
            .copied()
            .reduce(f64::min);
        let min_low_5 = self.recent_lows.iter().copied().reduce(f64::min);
        let max_high_3 = self
            .recent_highs
            .iter()
            .rev()
            .take(3)
            .copied()
            .reduce(f64::max);
        let max_high_5 = self.recent_highs.iter().copied().reduce(f64::max);

        IndicatorSnapshot {
            ema9: self.ema9.value(),
            ema21: self.ema21.value(),
            ema55: self.ema55.value(),
            ema200: self.ema200.value(),
            rsi14: self.rsi14.value(),
            prev_rsi14: self.prev_rsi14,
            atr14: self.atr14.value(),
            adx14: adx.map(|v| v.adx),
            prev_adx14: self.prev_adx14,
            plus_di: adx.map(|v| v.plus_di),
            minus_di: adx.map(|v| v.minus_di),
            donchian_upper: dc.map(|v| v.upper),
            donchian_lower: dc.map(|v| v.lower),
            donchian_mid: dc.map(|v| v.mid),
            session_vwap: self.vwap.value(),
            vol_sma20: self.vol_sma20.value(),
            min_low_3,
            min_low_5,
            max_high_3,
            max_high_5,
        }
    }
}

/// Incremental Higher-Timeframe (4h) `EMA(50)` regime tracker using strictly **closed** HTF bars (§6.4).
#[derive(Debug, Clone)]
pub struct HtfRegimeFeed {
    ema50: Ema,
    last_close: f64,
    ema_history: VecDeque<f64>,
}

impl Default for HtfRegimeFeed {
    fn default() -> Self {
        Self::new()
    }
}

impl HtfRegimeFeed {
    /// Construct a fresh [`HtfRegimeFeed`] (`EMA(50)`).
    #[must_use]
    pub fn new() -> Self {
        Self {
            ema50: Ema::new(50),
            last_close: 0.0,
            ema_history: VecDeque::with_capacity(5),
        }
    }

    /// Ingest a **closed** HTF (4h) bar and return the updated [`HtfRegime`].
    pub fn on_closed_htf_bar(&mut self, bar: &Kline) -> HtfRegime {
        if !bar.is_closed {
            return self.regime();
        }
        self.last_close = bar.close;
        if let Some(val) = self.ema50.update(bar) {
            if self.ema_history.len() >= 4 {
                let _ = self.ema_history.pop_front();
            }
            self.ema_history.push_back(val);
        }
        self.regime()
    }

    /// Current [`HtfRegime`] state.
    #[must_use]
    pub fn regime(&self) -> HtfRegime {
        let Some(&latest) = self.ema_history.back() else {
            return HtfRegime {
                close: self.last_close,
                ema50: 0.0,
                ema50_rising_3bars: false,
                ema50_falling_3bars: false,
                ready: false,
            };
        };

        // Rising over 3 bars = 4 consecutive samples: e[0] < e[1] < e[2] < e[3].
        let (rising, falling) = if self.ema_history.len() >= 4 {
            let mut it = self.ema_history.iter().copied();
            if let (Some(a), Some(b), Some(c), Some(d)) =
                (it.next(), it.next(), it.next(), it.next())
            {
                (a < b && b < c && c < d, a > b && b > c && c > d)
            } else {
                (false, false)
            }
        } else {
            (false, false)
        };

        HtfRegime {
            close: self.last_close,
            ema50: latest,
            ema50_rising_3bars: rising,
            ema50_falling_3bars: falling,
            ready: true,
        }
    }
}
