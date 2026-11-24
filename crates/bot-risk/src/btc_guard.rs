//! BTC Crash Guard with hysteresis and anti-flapping cooldown (§7.5).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Minimum time BTC Crash Guard stays active once triggered (30 minutes per §7.5).
pub const BTC_GUARD_MIN_ACTIVE_MS: i64 = 30 * 60 * 1000;
/// Alternative release time without any new low (60 minutes per §7.5).
pub const BTC_GUARD_NO_NEW_LOW_RELEASE_MS: i64 = 60 * 60 * 1000;
/// Minimum cooldown between consecutive triggers to prevent flapping (15 minutes per §7.5).
pub const BTC_GUARD_RETRIGGER_COOLDOWN_MS: i64 = 15 * 60 * 1000;

/// Instantaneous BTC market metrics fed into [`BtcCrashGuard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BtcMarketSample {
    /// Current UTC timestamp (ms).
    pub now_ms: i64,
    /// Current BTCUSDT price.
    pub price: Decimal,
    /// Percentage drop over the last 15 minutes (positive number, e.g. `1.8` means −1.8%).
    pub drop_15m_pct: Decimal,
    /// Percentage drop over the last 60 minutes (positive number, e.g. `3.2` means −3.2%).
    pub drop_60m_pct: Decimal,
    /// Pre-drop reference price (local 15m/60m high) used to measure 50% recovery.
    pub reference_high_price: Decimal,
    /// Current `ATR(1m, 14) / price`.
    pub atr_1m_pct: Decimal,
    /// 24-hour median of `ATR(1m, 14) / price`.
    pub median_atr_24h_pct: Decimal,
}

/// Output action produced by [`BtcCrashGuard::update`].
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BtcGuardAction {
    /// Whether the guard is currently active (blocking new entries).
    pub active: bool,
    /// True on the exact tick the guard transitioned inactive → active.
    pub just_triggered: bool,
    /// True on the exact tick the guard transitioned active → inactive.
    pub just_released: bool,
    /// True when profitable open positions should move stops to `max(stop, entry)`.
    pub tighten_profitable_stops_to_be: bool,
}

/// Stateful BTC Crash Guard with hysteresis (§7.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtcCrashGuard {
    crash_15m_pct: Decimal,
    crash_60m_pct: Decimal,
    active: bool,
    triggered_at_ms: i64,
    reference_high_price: Decimal,
    lowest_price: Decimal,
    last_new_low_ms: i64,
    cooldown_until_ms: i64,
}

impl BtcCrashGuard {
    /// Create a new [`BtcCrashGuard`] with configured 15m and 60m drop thresholds (default 1.5% / 3.0%).
    #[must_use]
    pub const fn new(crash_15m_pct: Decimal, crash_60m_pct: Decimal) -> Self {
        Self {
            crash_15m_pct,
            crash_60m_pct,
            active: false,
            triggered_at_ms: 0,
            reference_high_price: Decimal::ZERO,
            lowest_price: Decimal::ZERO,
            last_new_low_ms: 0,
            cooldown_until_ms: 0,
        }
    }

    /// Whether the guard is currently active.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Process a new [`BtcMarketSample`] and return the resulting [`BtcGuardAction`].
    pub fn update(&mut self, sample: BtcMarketSample) -> BtcGuardAction {
        if !self.active {
            let triggered = sample.drop_15m_pct >= self.crash_15m_pct
                || sample.drop_60m_pct >= self.crash_60m_pct;
            if triggered && sample.now_ms >= self.cooldown_until_ms {
                self.active = true;
                self.triggered_at_ms = sample.now_ms;
                self.reference_high_price = sample.reference_high_price.max(sample.price);
                self.lowest_price = sample.price;
                self.last_new_low_ms = sample.now_ms;
                return BtcGuardAction {
                    active: true,
                    just_triggered: true,
                    just_released: false,
                    tighten_profitable_stops_to_be: true,
                };
            }
            return BtcGuardAction {
                active: false,
                just_triggered: false,
                just_released: false,
                tighten_profitable_stops_to_be: false,
            };
        }

        // Guard is currently active: track any new low.
        if sample.price < self.lowest_price {
            self.lowest_price = sample.price;
            self.last_new_low_ms = sample.now_ms;
        }

        let elapsed_ms = sample.now_ms.saturating_sub(self.triggered_at_ms);
        let atr_limit = sample
            .median_atr_24h_pct
            .checked_mul(Decimal::new(15, 1))
            .unwrap_or(Decimal::MAX);
        let volatility_normalized = sample.atr_1m_pct < atr_limit;

        let drop_size = (self.reference_high_price - self.lowest_price).max(Decimal::ZERO);
        let half_drop = drop_size / Decimal::from(2_u32);
        let recovered_50_pct =
            drop_size > Decimal::ZERO && (sample.price - self.lowest_price) >= half_drop;
        let no_new_low_60m =
            sample.now_ms.saturating_sub(self.last_new_low_ms) >= BTC_GUARD_NO_NEW_LOW_RELEASE_MS;

        if elapsed_ms >= BTC_GUARD_MIN_ACTIVE_MS
            && volatility_normalized
            && (recovered_50_pct || no_new_low_60m)
        {
            self.active = false;
            self.cooldown_until_ms = sample
                .now_ms
                .saturating_add(BTC_GUARD_RETRIGGER_COOLDOWN_MS);
            return BtcGuardAction {
                active: false,
                just_triggered: false,
                just_released: true,
                tighten_profitable_stops_to_be: false,
            };
        }

        BtcGuardAction {
            active: true,
            just_triggered: false,
            just_released: false,
            tighten_profitable_stops_to_be: false,
        }
    }
}

/// Instantaneous BTC market metrics fed into [`BtcSpikeGuard`] for Short entries (§6.4.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BtcSpikeSample {
    /// Current UTC timestamp (ms).
    pub now_ms: i64,
    /// Current BTCUSDT price.
    pub price: Decimal,
    /// Percentage rise over the last 15 minutes (positive number, e.g. `1.8` means +1.8%).
    pub rise_15m_pct: Decimal,
    /// Percentage rise over the last 60 minutes (positive number, e.g. `3.2` means +3.2%).
    pub rise_60m_pct: Decimal,
    /// Pre-spike reference price (local 15m/60m low) used to measure 50% retrace.
    pub reference_low_price: Decimal,
    /// Current `ATR(1m, 14) / price`.
    pub atr_1m_pct: Decimal,
    /// 24-hour median of `ATR(1m, 14) / price`.
    pub median_atr_24h_pct: Decimal,
}

/// Stateful BTC Spike Guard for Short entries with hysteresis matching §7.5 (§6.4.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BtcSpikeGuard {
    spike_15m_pct: Decimal,
    spike_60m_pct: Decimal,
    active: bool,
    triggered_at_ms: i64,
    reference_low_price: Decimal,
    highest_price: Decimal,
    last_new_high_ms: i64,
    cooldown_until_ms: i64,
}

impl BtcSpikeGuard {
    /// Create a new [`BtcSpikeGuard`] with configured 15m and 60m rise thresholds (default 1.5% / 3.0%).
    #[must_use]
    pub const fn new(spike_15m_pct: Decimal, spike_60m_pct: Decimal) -> Self {
        Self {
            spike_15m_pct,
            spike_60m_pct,
            active: false,
            triggered_at_ms: 0,
            reference_low_price: Decimal::ZERO,
            highest_price: Decimal::ZERO,
            last_new_high_ms: 0,
            cooldown_until_ms: 0,
        }
    }

    /// Whether the spike guard is currently active (blocking new Short entries).
    #[must_use]
    pub const fn is_active(&self) -> bool {
        self.active
    }

    /// Process a new [`BtcSpikeSample`] and return the resulting [`BtcGuardAction`].
    pub fn update(&mut self, sample: BtcSpikeSample) -> BtcGuardAction {
        if !self.active {
            let triggered = sample.rise_15m_pct >= self.spike_15m_pct
                || sample.rise_60m_pct >= self.spike_60m_pct;
            if triggered && sample.now_ms >= self.cooldown_until_ms {
                self.active = true;
                self.triggered_at_ms = sample.now_ms;
                self.reference_low_price = sample.reference_low_price.min(sample.price);
                self.highest_price = sample.price;
                self.last_new_high_ms = sample.now_ms;
                return BtcGuardAction {
                    active: true,
                    just_triggered: true,
                    just_released: false,
                    tighten_profitable_stops_to_be: true,
                };
            }
            return BtcGuardAction {
                active: false,
                just_triggered: false,
                just_released: false,
                tighten_profitable_stops_to_be: false,
            };
        }

        if sample.price > self.highest_price {
            self.highest_price = sample.price;
            self.last_new_high_ms = sample.now_ms;
        }

        let elapsed_ms = sample.now_ms.saturating_sub(self.triggered_at_ms);
        let atr_limit = sample
            .median_atr_24h_pct
            .checked_mul(Decimal::new(15, 1))
            .unwrap_or(Decimal::MAX);
        let volatility_normalized = sample.atr_1m_pct < atr_limit;

        let spike_size = (self.highest_price - self.reference_low_price).max(Decimal::ZERO);
        let half_spike = spike_size / Decimal::from(2_u32);
        let retraced_50_pct =
            spike_size > Decimal::ZERO && (self.highest_price - sample.price) >= half_spike;
        let no_new_high_60m =
            sample.now_ms.saturating_sub(self.last_new_high_ms) >= BTC_GUARD_NO_NEW_LOW_RELEASE_MS;

        if elapsed_ms >= BTC_GUARD_MIN_ACTIVE_MS
            && volatility_normalized
            && (retraced_50_pct || no_new_high_60m)
        {
            self.active = false;
            self.cooldown_until_ms = sample
                .now_ms
                .saturating_add(BTC_GUARD_RETRIGGER_COOLDOWN_MS);
            return BtcGuardAction {
                active: false,
                just_triggered: false,
                just_released: true,
                tighten_profitable_stops_to_be: false,
            };
        }

        BtcGuardAction {
            active: true,
            just_triggered: false,
            just_released: false,
            tighten_profitable_stops_to_be: false,
        }
    }
}
