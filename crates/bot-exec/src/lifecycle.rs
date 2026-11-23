//! Position lifecycle evaluator (§5.5) with strict safety-first ordering and `PnL` reporting.

use bot_config::ExitConfig;
use bot_risk::ratchet_stop_monotonic;
use bot_types::{Direction, Position, PositionState, Price, Qty};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::protection::HardStopWatcher;

/// Minimum `best_price` advance (as multiple of ATR) before journaling high-water mark (§5.5 #3).
pub const BEST_PRICE_JOURNAL_ATR_MULT: Decimal = Decimal::from_parts(1, 0, 0, false, 1);
/// Time-stop stagnation band: `|profit_R| < 0.3` (§5.5 #9).
pub const TIME_STOP_STAGNATION_R: Decimal = Decimal::from_parts(3, 0, 0, false, 1);
/// Minimum `profit_R` before RSI exhaustion tightening activates (§5.5 #7).
pub const EXHAUSTION_MIN_PROFIT_R: Decimal = Decimal::from_parts(2, 0, 0, false, 0);

/// Inputs to [`evaluate_position_tick`] for a single open position.
#[derive(Debug, Clone, Copy)]
pub struct PositionTickInput {
    /// Current mark/bid price for exit evaluation.
    pub price: Price,
    /// Current UTC timestamp (ms).
    pub now_ms: i64,
    /// Current ATR (in price units).
    pub atr: Decimal,
    /// Current RSI(14) value, if available.
    pub rsi_14: Option<Decimal>,
    /// Number of signal timeframe bars elapsed since entry.
    pub bars_in_trade: u32,
    /// Whether exchange-side protective OCO/algo order is confirmed active.
    pub has_exchange_protection: bool,
    /// Consecutive protection reinstall failures so far.
    pub reinstall_failures: u32,
    /// Exchange instrument `tick_size` (used for BE +1 tick buffer).
    pub tick_size: Decimal,
    /// Taker fee in basis points (used for BE `2 * taker_fee_bps * entry + 1 tick`).
    pub taker_fee_bps: Decimal,
}

/// Ordered action returned by [`evaluate_position_tick`] (strictly safety-first per §5.5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleAction {
    /// No action required on this tick.
    Hold,
    /// §5.5 #1: Protection missing on exchange → reinstall immediately.
    ReinstallProtection,
    /// §5.5 #1 (failed 2×) or #2 (`HardStopWatcher` grace exceeded) → immediate market exit.
    EmergencyMarketExit {
        /// Human-readable cause.
        reason: String,
    },
    /// §5.5 #5–#7: Ratchet stop-loss monotonically (Break-Even or Trailing).
    MoveStop {
        /// New protective stop price (strictly tighter than current stop).
        new_stop: Price,
        /// Whether this move completes break-even (`be_done = true`).
        is_breakeven: bool,
        /// Optional updated `best_price` if it advanced ≥ 0.1 ATR.
        journal_best_price: Option<Price>,
    },
    /// §5.5 #3: Only `best_price` advanced ≥ 0.1 ATR (journal high-water mark).
    UpdateBestPrice {
        /// New high-water / low-water mark.
        best_price: Price,
    },
    /// §5.5 #9: Position stagnated (`|profit_R| < 0.3`) past `max_bars_in_trade` → exit.
    TimeStopExit {
        /// Bars elapsed.
        bars_in_trade: u32,
    },
}

/// Standardized exit `PnL` summary recorded to journal & UI (§5.5 #10).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitPnlReport {
    /// Gross `PnL` before fees (quote currency).
    #[serde(with = "rust_decimal::serde::str")]
    pub gross_pnl_quote: Decimal,
    /// Total fees paid (entry + exit, quote currency).
    #[serde(with = "rust_decimal::serde::str")]
    pub fees_quote: Decimal,
    /// Net `PnL` (`gross - fees`, quote currency).
    #[serde(with = "rust_decimal::serde::str")]
    pub net_pnl_quote: Decimal,
    /// Realized R-multiple (`net_pnl / (qty_initial * r_distance)`).
    #[serde(with = "rust_decimal::serde::str")]
    pub r_multiple: Decimal,
}

/// Compute standardized gross/fee/net `PnL` and R-multiple for a closed position (§5.5 #10).
#[must_use]
pub fn compute_exit_pnl_report(
    direction: Direction,
    entry_price: Price,
    exit_price: Price,
    qty_closed: Qty,
    qty_initial: Qty,
    r_distance: Decimal,
    total_fees_quote: Decimal,
) -> ExitPnlReport {
    let diff = match direction {
        Direction::Long => exit_price.as_decimal() - entry_price.as_decimal(),
        Direction::Short => entry_price.as_decimal() - exit_price.as_decimal(),
    };
    let gross_pnl_quote = diff.saturating_mul(qty_closed.as_decimal());
    let net_pnl_quote = gross_pnl_quote.saturating_sub(total_fees_quote);
    let initial_risk = qty_initial.as_decimal().saturating_mul(r_distance);
    let r_multiple = if initial_risk > Decimal::ZERO {
        net_pnl_quote
            .checked_div(initial_risk)
            .unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };
    ExitPnlReport {
        gross_pnl_quote,
        fees_quote: total_fees_quote,
        net_pnl_quote,
        r_multiple,
    }
}

/// Evaluate a single position tick strictly in §5.5 safety-first order:
/// 1. Protection presence check (`OpenUnprotected` → reinstall, or 2× fail → `EmergencyMarketExit`).
/// 2. `HardStopWatcher` breach check (`>= hard_stop_grace_ms` → `EmergencyMarketExit`).
/// 3. `best_price` update (journal threshold `>= 0.1 * ATR`).
/// 4. `profit_R` calculation.
/// 5. Break-even (`profit_R >= be_trigger_r`).
/// 6. Monotonic Chandelier trailing (`profit_R >= trail_activate_r`) with step `>= trail_min_step_atr * ATR`.
/// 7. Momentum exhaustion tightening (`RSI(14) >= exhaustion_rsi` & `profit_R >= 2R`).
/// 8. Time stop (`bars_in_trade >= max_bars_in_trade` & `|profit_R| < 0.3`).
#[allow(clippy::too_many_lines)]
pub fn evaluate_position_tick(
    pos: &mut Position,
    input: PositionTickInput,
    cfg: &ExitConfig,
    watcher: &mut HardStopWatcher,
) -> LifecycleAction {
    if matches!(
        pos.state,
        PositionState::Closed | PositionState::EntryPending
    ) {
        return LifecycleAction::Hold;
    }

    // 1. Protection check (safety first!)
    let unprotected_expired = watcher.check_unprotected_duration(
        pos.id,
        input.has_exchange_protection,
        input.now_ms,
        cfg.hard_stop_grace_ms,
    );
    if !input.has_exchange_protection {
        pos.state = PositionState::OpenUnprotected;
        if input.reinstall_failures >= 2 || unprotected_expired {
            return LifecycleAction::EmergencyMarketExit {
                reason: "exchange protection missing beyond retry/grace limit".into(),
            };
        }
        return LifecycleAction::ReinstallProtection;
    }

    // 2. HardStopWatcher check
    if watcher.check_stop_breach(
        pos.id,
        pos.direction,
        input.price,
        pos.stop_price,
        input.now_ms,
        cfg.hard_stop_grace_ms,
    ) {
        return LifecycleAction::EmergencyMarketExit {
            reason: format!(
                "hard stop breached past {} ms grace (price={}, stop={})",
                cfg.hard_stop_grace_ms,
                input.price.as_decimal(),
                pos.stop_price.as_decimal()
            ),
        };
    }

    // 3. Update best_price (journal only if advance >= 0.1 * ATR)
    let prev_best = pos.best_price.as_decimal();
    let new_best = match pos.direction {
        Direction::Long => prev_best.max(input.price.as_decimal()),
        Direction::Short => prev_best.min(input.price.as_decimal()),
    };
    let best_delta = (new_best - prev_best).abs();
    let journal_best = if best_delta >= input.atr.saturating_mul(BEST_PRICE_JOURNAL_ATR_MULT)
        && new_best != prev_best
    {
        pos.best_price = Price(new_best);
        Some(Price(new_best))
    } else {
        None
    };

    // 4. profit_R
    let profit_r = if pos.r_distance > Decimal::ZERO {
        let pnl_per_unit = match pos.direction {
            Direction::Long => input.price.as_decimal() - pos.entry_price.as_decimal(),
            Direction::Short => pos.entry_price.as_decimal() - input.price.as_decimal(),
        };
        pnl_per_unit
            .checked_div(pos.r_distance)
            .unwrap_or(Decimal::ZERO)
    } else {
        Decimal::ZERO
    };

    let mut candidate_stop = pos.stop_price;
    let mut is_breakeven = false;

    // 5. Break-even: SL = entry + 2 * taker_fee_bps * entry + 1 tick (Long)
    if !pos.breakeven_done && profit_r >= cfg.be_trigger_r {
        let fee_mult =
            input.taker_fee_bps.saturating_mul(Decimal::from(2_u32)) / Decimal::from(10_000_u32);
        let fee_buf = pos
            .entry_price
            .as_decimal()
            .saturating_mul(fee_mult)
            .saturating_add(input.tick_size);
        let be_price = match pos.direction {
            Direction::Long => Price(pos.entry_price.as_decimal().saturating_add(fee_buf)),
            Direction::Short => Price(pos.entry_price.as_decimal().saturating_sub(fee_buf)),
        };
        let ratcheted = ratchet_stop_monotonic(pos.direction, candidate_stop, be_price);
        if ratcheted != candidate_stop {
            candidate_stop = ratcheted;
            is_breakeven = true;
        }
    }

    // 6 & 7. Monotonic trailing + RSI exhaustion tightening
    if profit_r >= cfg.trail_activate_r && input.atr > Decimal::ZERO {
        let mut k = cfg.trail_atr_mult;
        if profit_r >= EXHAUSTION_MIN_PROFIT_R
            && let Some(rsi) = input.rsi_14
            && ((pos.direction == Direction::Long && rsi >= cfg.exhaustion_rsi)
                || (pos.direction == Direction::Short
                    && rsi <= Decimal::from(100_u32).saturating_sub(cfg.exhaustion_rsi)))
        {
            k = k.saturating_mul(cfg.exhaustion_trail_factor);
        }
        let trail_dist = k.saturating_mul(input.atr);
        let trail_raw = match pos.direction {
            Direction::Long => Price(new_best.saturating_sub(trail_dist)),
            Direction::Short => Price(new_best.saturating_add(trail_dist)),
        };
        let min_step = cfg.trail_min_step_atr.saturating_mul(input.atr);
        let ratcheted = ratchet_stop_monotonic(pos.direction, candidate_stop, trail_raw);
        let step_gain = (ratcheted.as_decimal() - pos.stop_price.as_decimal()).abs();
        if ratcheted != pos.stop_price && (is_breakeven || step_gain >= min_step) {
            candidate_stop = ratcheted;
        }
    }

    if candidate_stop != pos.stop_price {
        return LifecycleAction::MoveStop {
            new_stop: candidate_stop,
            is_breakeven,
            journal_best_price: journal_best,
        };
    }

    // 9. Time stop: stagnated (|profit_R| < 0.3) past max_bars_in_trade
    if input.bars_in_trade >= cfg.max_bars_in_trade && profit_r.abs() < TIME_STOP_STAGNATION_R {
        return LifecycleAction::TimeStopExit {
            bars_in_trade: input.bars_in_trade,
        };
    }

    if let Some(best_price) = journal_best {
        return LifecycleAction::UpdateBestPrice { best_price };
    }

    LifecycleAction::Hold
}
