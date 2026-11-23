//! Spot position sizing, portfolio heat enforcement, filter rounding, and monotonic stop ratchet (§7.3).

use bot_config::SizingMode;
use bot_types::{Direction, InstrumentRules, Price, Qty, floor_to_step};
use rust_decimal::Decimal;
use thiserror::Error;

/// Default fee buffer required in free quote balance (0.2% per §7.2 #8).
pub const DEFAULT_FEE_BUFFER_FRACTION: Decimal = Decimal::from_parts(2, 0, 0, false, 3);
/// Free quote balance safety factor (98% per §7.3).
pub const FREE_QUOTE_USABLE_FRACTION: Decimal = Decimal::from_parts(98, 0, 0, false, 2);

/// Errors returned by [`size_spot_position`].
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SizingError {
    /// Equity, entry price, or stop distance is non-positive.
    #[error("invalid sizing input: {0}")]
    InvalidInput(String),
    /// Arithmetic overflow during Decimal calculation.
    #[error("decimal arithmetic overflow during sizing")]
    ArithmeticOverflow,
    /// Rounded quantity is below `min_qty` (never rounded up per §7.2 #7).
    #[error("rounded qty {qty} is below min_qty {min_qty}")]
    QtyBelowMin {
        /// Floored quantity.
        qty: Decimal,
        /// Exchange `min_qty`.
        min_qty: Decimal,
    },
    /// Rounded quantity exceeds `max_qty`.
    #[error("rounded qty {qty} exceeds max_qty {max_qty}")]
    QtyAboveMax {
        /// Floored quantity.
        qty: Decimal,
        /// Exchange `max_qty`.
        max_qty: Decimal,
    },
    /// Computed notional is below `min_notional` (never rounded up).
    #[error("notional {notional} is below min_notional {min_notional}")]
    NotionalBelowMin {
        /// `qty * entry_price`.
        notional: Decimal,
        /// Exchange `min_notional`.
        min_notional: Decimal,
    },
    /// Adding this trade would exceed `portfolio_heat_max_pct`.
    #[error("portfolio heat {heat_after_pct}% would exceed max {max_heat_pct}%")]
    PortfolioHeatExceeded {
        /// Projected heat percentage after entry.
        heat_after_pct: Decimal,
        /// Configured ceiling.
        max_heat_pct: Decimal,
    },
    /// Total exposure cap leaves insufficient notional room.
    #[error("remaining total exposure capacity {remaining_usd} is exhausted")]
    ExposureCapReached {
        /// Remaining notional allowance in USD.
        remaining_usd: Decimal,
    },
    /// Free quote balance cannot cover `notional * (1 + fee_buffer)`.
    #[error("insufficient free quote balance: need {required}, have {available}")]
    InsufficientFreeQuote {
        /// Required quote balance including fee buffer.
        required: Decimal,
        /// Available free quote balance.
        available: Decimal,
    },
}

/// Inputs for Spot position sizing (§7.3).
#[derive(Debug, Clone)]
pub struct SpotSizingInput<'a> {
    /// Combined account equity in quote currency (USD/USDT).
    pub equity: Decimal,
    /// Available free quote balance.
    pub free_quote: Decimal,
    /// Intended entry reference price.
    pub entry_price: Price,
    /// Actual protective stop price (including any buffer).
    pub stop_price: Price,
    /// Exchange instrument filter rules (`step_size`, `min_qty`, `max_qty`, `min_notional`).
    pub rules: &'a InstrumentRules,
    /// Effective risk per trade percentage (already reduced by Kelly if active).
    pub risk_per_trade_pct: Decimal,
    /// Volatility targeting multiplier (§6.5, clamped to `[0.5, 1.25]`).
    pub size_mult: Decimal,
    /// Strategy scanner multiplier (`1.0` core, `0.5` scanner S3).
    pub scanner_mult: Decimal,
    /// Daily profit-lock multiplier (`1.0` normal, `0.5` reduce mode).
    pub profit_lock_mult: Decimal,
    /// Sizing mode (`FixedRiskPct` or `FixedNotional`).
    pub sizing_mode: SizingMode,
    /// Target notional when `sizing_mode == SizingMode::FixedNotional`.
    pub fixed_notional_usd: Option<Decimal>,
    /// Maximum notional per single position as `%` of equity.
    pub max_notional_per_position_pct: Decimal,
    /// Current combined portfolio heat `%` across all open positions.
    pub current_portfolio_heat_pct: Decimal,
    /// Maximum allowed combined portfolio heat `%`.
    pub portfolio_heat_max_pct: Decimal,
    /// Current total open exposure in USD across all open positions.
    pub current_total_exposure_usd: Decimal,
    /// Maximum allowed total exposure as `%` of equity.
    pub max_total_exposure_pct: Decimal,
}

/// Verified Spot sizing result.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpotSizingOutput {
    /// Exchange-valid quantity floored to `step_size`.
    pub qty: Qty,
    /// Actual order notional (`qty * entry_price`).
    pub notional_usd: Decimal,
    /// Actual dollar risk (`qty * |entry_price - stop_price|`).
    pub actual_risk_usd: Decimal,
    /// Actual risk as percentage of equity (`actual_risk_usd / equity * 100`).
    pub actual_risk_pct: Decimal,
    /// Projected combined portfolio heat `%` after adding this position.
    pub heat_after_pct: Decimal,
}

/// Compute Spot position size per §7.3 with strict filter rounding and cap enforcement.
///
/// Never rounds quantity UP to satisfy `min_qty` or `min_notional`; rejects instead.
///
/// # Errors
/// Returns [`SizingError`] if any invariant, cap, or exchange filter fails.
#[allow(clippy::too_many_lines)]
pub fn size_spot_position(input: &SpotSizingInput<'_>) -> Result<SpotSizingOutput, SizingError> {
    let hundred = Decimal::from(100_u32);
    let entry = input.entry_price.as_decimal();
    let stop = input.stop_price.as_decimal();

    if input.equity <= Decimal::ZERO {
        return Err(SizingError::InvalidInput("equity must be positive".into()));
    }
    if entry <= Decimal::ZERO || stop <= Decimal::ZERO {
        return Err(SizingError::InvalidInput(
            "entry and stop prices must be positive".into(),
        ));
    }
    let stop_dist = (entry - stop).abs();
    if stop_dist <= Decimal::ZERO {
        return Err(SizingError::InvalidInput(
            "stop distance must be positive".into(),
        ));
    }
    if input.risk_per_trade_pct <= Decimal::ZERO
        || input.size_mult <= Decimal::ZERO
        || input.scanner_mult <= Decimal::ZERO
        || input.profit_lock_mult <= Decimal::ZERO
    {
        return Err(SizingError::InvalidInput(
            "risk percentage and multipliers must be positive".into(),
        ));
    }

    let risk_cap = input
        .equity
        .checked_mul(input.risk_per_trade_pct)
        .and_then(|v| v.checked_div(hundred))
        .and_then(|v| v.checked_mul(input.size_mult))
        .and_then(|v| v.checked_mul(input.scanner_mult))
        .and_then(|v| v.checked_mul(input.profit_lock_mult))
        .ok_or(SizingError::ArithmeticOverflow)?;

    let raw_notional = match input.sizing_mode {
        SizingMode::FixedRiskPct => {
            let qty_raw = risk_cap
                .checked_div(stop_dist)
                .ok_or(SizingError::ArithmeticOverflow)?;
            qty_raw
                .checked_mul(entry)
                .ok_or(SizingError::ArithmeticOverflow)?
        }
        SizingMode::FixedNotional => {
            let fixed = input.fixed_notional_usd.ok_or_else(|| {
                SizingError::InvalidInput(
                    "fixed_notional_usd required for FixedNotional sizing mode".into(),
                )
            })?;
            if fixed <= Decimal::ZERO {
                return Err(SizingError::InvalidInput(
                    "fixed_notional_usd must be positive".into(),
                ));
            }
            // Still bound fixed notional by risk_cap so actual risk never exceeds cap.
            let max_by_risk = risk_cap
                .checked_div(stop_dist)
                .and_then(|q| q.checked_mul(entry))
                .ok_or(SizingError::ArithmeticOverflow)?;
            fixed.min(max_by_risk)
        }
    };

    let max_pos_notional = input
        .equity
        .checked_mul(input.max_notional_per_position_pct)
        .and_then(|v| v.checked_div(hundred))
        .ok_or(SizingError::ArithmeticOverflow)?;

    let max_total_exposure_usd = input
        .equity
        .checked_mul(input.max_total_exposure_pct)
        .and_then(|v| v.checked_div(hundred))
        .ok_or(SizingError::ArithmeticOverflow)?;
    let remaining_exposure = max_total_exposure_usd
        .checked_sub(input.current_total_exposure_usd)
        .ok_or(SizingError::ArithmeticOverflow)?;
    if remaining_exposure <= Decimal::ZERO {
        return Err(SizingError::ExposureCapReached {
            remaining_usd: remaining_exposure,
        });
    }

    let max_free_notional = input
        .free_quote
        .max(Decimal::ZERO)
        .checked_mul(FREE_QUOTE_USABLE_FRACTION)
        .ok_or(SizingError::ArithmeticOverflow)?;

    let target_notional = raw_notional
        .min(max_pos_notional)
        .min(remaining_exposure)
        .min(max_free_notional);

    let qty_unrounded = target_notional
        .checked_div(entry)
        .ok_or(SizingError::ArithmeticOverflow)?;
    let qty_dec = floor_to_step(Qty(qty_unrounded), input.rules.step_size)
        .map_err(|e| SizingError::InvalidInput(e.to_string()))?
        .as_decimal();

    if qty_dec < input.rules.min_qty {
        return Err(SizingError::QtyBelowMin {
            qty: qty_dec,
            min_qty: input.rules.min_qty,
        });
    }
    if qty_dec > input.rules.max_qty {
        return Err(SizingError::QtyAboveMax {
            qty: qty_dec,
            max_qty: input.rules.max_qty,
        });
    }

    let notional_usd = qty_dec
        .checked_mul(entry)
        .ok_or(SizingError::ArithmeticOverflow)?;
    if notional_usd < input.rules.min_notional {
        return Err(SizingError::NotionalBelowMin {
            notional: notional_usd,
            min_notional: input.rules.min_notional,
        });
    }

    let required_quote = notional_usd
        .checked_mul(Decimal::ONE + DEFAULT_FEE_BUFFER_FRACTION)
        .ok_or(SizingError::ArithmeticOverflow)?;
    if input.free_quote < required_quote {
        return Err(SizingError::InsufficientFreeQuote {
            required: required_quote,
            available: input.free_quote,
        });
    }

    let actual_risk_usd = qty_dec
        .checked_mul(stop_dist)
        .ok_or(SizingError::ArithmeticOverflow)?;
    let actual_risk_pct = actual_risk_usd
        .checked_div(input.equity)
        .and_then(|v| v.checked_mul(hundred))
        .ok_or(SizingError::ArithmeticOverflow)?;
    let heat_after_pct = input
        .current_portfolio_heat_pct
        .checked_add(actual_risk_pct)
        .ok_or(SizingError::ArithmeticOverflow)?;

    if heat_after_pct > input.portfolio_heat_max_pct {
        return Err(SizingError::PortfolioHeatExceeded {
            heat_after_pct,
            max_heat_pct: input.portfolio_heat_max_pct,
        });
    }

    Ok(SpotSizingOutput {
        qty: Qty(qty_dec),
        notional_usd,
        actual_risk_usd,
        actual_risk_pct,
        heat_after_pct,
    })
}

/// Monotonic protective stop ratchet: Long stops may only move UP; Short stops may only move DOWN.
#[must_use]
pub fn ratchet_stop_monotonic(
    direction: Direction,
    current_stop: Price,
    candidate_stop: Price,
) -> Price {
    match direction {
        Direction::Long => Price(current_stop.as_decimal().max(candidate_stop.as_decimal())),
        Direction::Short => Price(current_stop.as_decimal().min(candidate_stop.as_decimal())),
    }
}
