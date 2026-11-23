//! Combined cross-venue equity counters, daily/weekly/drawdown breakers,
//! loss streak cooldown, and profit-lock (§7.2, §7.4, §7.8).

use std::collections::BTreeMap;

use bot_config::{ProfitLockMode, RiskConfig};
use bot_types::Venue;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::mode::ModeTrigger;

const MS_PER_DAY: i64 = 86_400_000;
const MS_PER_HOUR: i64 = 3_600_000;

/// Daily profit-lock threshold for `ProfitLockMode::Reduce` (+2% per §7.4).
pub const PROFIT_LOCK_REDUCE_TRIGGER_PCT: Decimal = Decimal::from_parts(2, 0, 0, false, 0);
/// Daily profit-lock threshold for `ProfitLockMode::Stop` (+3% per §7.4).
pub const PROFIT_LOCK_STOP_TRIGGER_PCT: Decimal = Decimal::from_parts(3, 0, 0, false, 0);

/// Result of evaluating global equity counters against [`RiskConfig`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EquityCheckOutcome {
    /// Current combined equity (`Σ venue equities`).
    pub combined_equity: Decimal,
    /// Daily `PnL` percentage vs UTC 00:00 start equity.
    pub daily_pnl_pct: Decimal,
    /// Weekly `PnL` percentage vs Monday UTC 00:00 start equity.
    pub weekly_pnl_pct: Decimal,
    /// Peak-to-trough drawdown percentage (`<= 0`).
    pub drawdown_pct: Decimal,
    /// Circuit breaker trigger fired (highest severity first: `DrawdownKill` > `Weekly` > `Daily`).
    pub breaker_trigger: Option<ModeTriggerKind>,
    /// Risk multiplier from profit-lock (`0.5` when reduce-locked, `1.0` otherwise).
    pub profit_lock_mult: Decimal,
    /// `true` if `ProfitLockMode::Stop` has stopped new entries for today.
    pub profit_lock_stop: bool,
}

/// Highest-severity equity breaker fired by [`CombinedEquityCounters::update_equity`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeTriggerKind {
    /// Peak drawdown hit `drawdown_kill_pct` → `Killed`.
    DrawdownKill,
    /// Weekly loss hit `weekly_loss_halt_pct` → `Halted`.
    WeeklyLossHalt,
    /// Daily loss hit `daily_loss_halt_pct` → `Halted`.
    DailyLossHalt,
}

impl ModeTriggerKind {
    /// Convert into the corresponding [`ModeTrigger`].
    #[must_use]
    pub const fn into_trigger(self) -> ModeTrigger {
        match self {
            Self::DrawdownKill => ModeTrigger::DrawdownKillHit,
            Self::WeeklyLossHalt => ModeTrigger::WeeklyLossHit,
            Self::DailyLossHalt => ModeTrigger::DailyLossHit,
        }
    }
}

/// Cross-venue global equity, streak, order-count, and profit-lock tracker (§7.2, §7.8).
///
/// Venue switches **never** reset these counters (§7.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CombinedEquityCounters {
    venue_equities: BTreeMap<Venue, Decimal>,
    day_start_equity: Decimal,
    week_start_equity: Decimal,
    peak_equity: Decimal,
    current_utc_day: i64,
    current_utc_week: i64,
    consecutive_losses: u32,
    loss_cooldown_until_ms: Option<i64>,
    orders_today: u32,
    consecutive_order_errors: u32,
}

impl CombinedEquityCounters {
    /// Initialize counters with an initial combined equity at `now_ms`.
    #[must_use]
    pub fn new(initial_venue: Venue, initial_equity: Decimal, now_ms: i64) -> Self {
        let mut venue_equities = BTreeMap::new();
        venue_equities.insert(initial_venue, initial_equity.max(Decimal::ZERO));
        let eq = initial_equity.max(Decimal::ZERO);
        Self {
            venue_equities,
            day_start_equity: eq,
            week_start_equity: eq,
            peak_equity: eq,
            current_utc_day: utc_day_index(now_ms),
            current_utc_week: utc_monday_week_index(now_ms),
            consecutive_losses: 0,
            loss_cooldown_until_ms: None,
            orders_today: 0,
            consecutive_order_errors: 0,
        }
    }

    /// Combined equity across all tracked venues (`equity_spot + equity_futures`).
    #[must_use]
    pub fn combined_equity(&self) -> Decimal {
        self.venue_equities
            .values()
            .copied()
            .fold(Decimal::ZERO, Decimal::saturating_add)
    }

    /// Current peak combined equity.
    #[must_use]
    pub const fn peak_equity(&self) -> Decimal {
        self.peak_equity
    }

    /// Orders placed so far in the current UTC day.
    #[must_use]
    pub const fn orders_today(&self) -> u32 {
        self.orders_today
    }

    /// Current loss-streak cooldown deadline (UTC ms), if active.
    #[must_use]
    pub const fn loss_cooldown_until_ms(&self) -> Option<i64> {
        self.loss_cooldown_until_ms
    }

    /// Consecutive losing trade count.
    #[must_use]
    pub const fn consecutive_losses(&self) -> u32 {
        self.consecutive_losses
    }

    /// Update a venue's equity (realized + unrealized), handle UTC day/week rollovers,
    /// update `peak_equity`, and evaluate circuit breakers + profit-lock.
    pub fn update_venue_equity(
        &mut self,
        venue: Venue,
        equity: Decimal,
        now_ms: i64,
        cfg: &RiskConfig,
    ) -> EquityCheckOutcome {
        self.venue_equities.insert(venue, equity.max(Decimal::ZERO));
        let combined = self.combined_equity();

        let day = utc_day_index(now_ms);
        if day > self.current_utc_day {
            self.current_utc_day = day;
            self.day_start_equity = combined;
            self.orders_today = 0;
            if self
                .loss_cooldown_until_ms
                .is_some_and(|until| now_ms >= until)
            {
                self.loss_cooldown_until_ms = None;
                self.consecutive_losses = 0;
            }
        }

        let week = utc_monday_week_index(now_ms);
        if week > self.current_utc_week {
            self.current_utc_week = week;
            self.week_start_equity = combined;
        }

        if combined > self.peak_equity {
            self.peak_equity = combined;
        }

        let daily_pnl_pct = pct_change(combined, self.day_start_equity);
        let weekly_pnl_pct = pct_change(combined, self.week_start_equity);
        let drawdown_pct = pct_change(combined, self.peak_equity);

        let breaker_trigger = if drawdown_pct <= cfg.drawdown_kill_pct {
            Some(ModeTriggerKind::DrawdownKill)
        } else if weekly_pnl_pct <= cfg.weekly_loss_halt_pct {
            Some(ModeTriggerKind::WeeklyLossHalt)
        } else if daily_pnl_pct <= cfg.daily_loss_halt_pct {
            Some(ModeTriggerKind::DailyLossHalt)
        } else {
            None
        };

        let (profit_lock_mult, profit_lock_stop) = match cfg.profit_lock_mode {
            ProfitLockMode::Reduce => {
                if daily_pnl_pct >= PROFIT_LOCK_REDUCE_TRIGGER_PCT {
                    (Decimal::new(5, 1), false)
                } else {
                    (Decimal::ONE, false)
                }
            }
            ProfitLockMode::Stop => {
                if daily_pnl_pct >= PROFIT_LOCK_STOP_TRIGGER_PCT {
                    (Decimal::ZERO, true)
                } else {
                    (Decimal::ONE, false)
                }
            }
            ProfitLockMode::Off => (Decimal::ONE, false),
        };

        EquityCheckOutcome {
            combined_equity: combined,
            daily_pnl_pct,
            weekly_pnl_pct,
            drawdown_pct,
            breaker_trigger,
            profit_lock_mult,
            profit_lock_stop,
        }
    }

    /// Record a closed trade's net `PnL` to update loss-streak cooldown (§7.2:
    /// 3 consecutive losses → 2 h cooldown; 5 consecutive losses → until next 00:00 UTC).
    pub fn record_trade_outcome(&mut self, net_pnl_quote: Decimal, now_ms: i64) {
        if net_pnl_quote < Decimal::ZERO {
            self.consecutive_losses = self.consecutive_losses.saturating_add(1);
            if self.consecutive_losses >= 5 {
                let next_utc_midnight = (utc_day_index(now_ms) + 1).saturating_mul(MS_PER_DAY);
                self.loss_cooldown_until_ms = Some(next_utc_midnight);
            } else if self.consecutive_losses >= 3 {
                let two_hours = now_ms.saturating_add(2 * MS_PER_HOUR);
                self.loss_cooldown_until_ms = Some(two_hours);
            }
        } else {
            self.consecutive_losses = 0;
            self.loss_cooldown_until_ms = None;
        }
    }

    /// Increment daily order counter (`orders_today`).
    pub fn record_order_submitted(&mut self) {
        self.orders_today = self.orders_today.saturating_add(1);
    }

    /// Record an order submission result; returns `Some(ModeTrigger::ConsecutiveOrderErrors)`
    /// once 3 consecutive errors occur (§7.4).
    pub fn record_order_error_state(&mut self, is_error: bool) -> Option<ModeTrigger> {
        if is_error {
            self.consecutive_order_errors = self.consecutive_order_errors.saturating_add(1);
            if self.consecutive_order_errors >= 3 {
                return Some(ModeTrigger::ConsecutiveOrderErrors {
                    count: self.consecutive_order_errors,
                });
            }
        } else {
            self.consecutive_order_errors = 0;
        }
        None
    }
}

fn pct_change(current: Decimal, baseline: Decimal) -> Decimal {
    if baseline <= Decimal::ZERO {
        return Decimal::ZERO;
    }
    let hundred = Decimal::from(100_u32);
    current
        .checked_sub(baseline)
        .and_then(|d| d.checked_div(baseline))
        .and_then(|r| r.checked_mul(hundred))
        .unwrap_or(Decimal::ZERO)
}

const fn utc_day_index(ts_ms: i64) -> i64 {
    ts_ms.div_euclid(MS_PER_DAY)
}

/// Monday-anchored UTC week index (Unix epoch 1970-01-01 was Thursday, offset +3 days).
const fn utc_monday_week_index(ts_ms: i64) -> i64 {
    (utc_day_index(ts_ms) + 3).div_euclid(7)
}
