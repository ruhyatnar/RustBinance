//! Pure helper functions for [`crate::EngineActor`] (direction mapping, `PnL`, runtime param updates).

use bot_config::{BotConfig, is_tightening};
use bot_types::{Direction, Price, Qty, Side};
use rust_decimal::Decimal;

/// Entry order side for a trade direction (`Long -> Buy`, `Short -> Sell`).
#[must_use]
pub const fn entry_side_for_direction(dir: Direction) -> Side {
    match dir {
        Direction::Long => Side::Buy,
        Direction::Short => Side::Sell,
    }
}

/// Closing/exit order side for a trade direction (`Long -> Sell`, `Short -> Buy`).
#[must_use]
pub const fn exit_side_for_direction(dir: Direction) -> Side {
    match dir {
        Direction::Long => Side::Sell,
        Direction::Short => Side::Buy,
    }
}

/// Compute TP2 target price respecting direction (`Long: entry + r*mult`, `Short: entry - r*mult`).
#[must_use]
pub fn compute_tp2_price(
    dir: Direction,
    entry: Price,
    r_dist: Decimal,
    tp2_r: Decimal,
    tick_size: Decimal,
) -> Price {
    let delta = r_dist.saturating_mul(tp2_r);
    let raw = match dir {
        Direction::Long => entry.as_decimal().saturating_add(delta),
        Direction::Short => entry.as_decimal().saturating_sub(delta).max(tick_size),
    };
    Price(raw)
}

/// Compute gross realized quote `PnL` for closing `qty` of a position at `exit_price`.
#[must_use]
pub fn compute_gross_realized_pnl(
    dir: Direction,
    entry_price: Price,
    exit_price: Price,
    qty: Qty,
) -> Decimal {
    let diff = match dir {
        Direction::Long => exit_price
            .as_decimal()
            .saturating_sub(entry_price.as_decimal()),
        Direction::Short => entry_price
            .as_decimal()
            .saturating_sub(exit_price.as_decimal()),
    };
    diff.saturating_mul(qty.as_decimal())
}

/// Apply a whitelisted runtime risk parameter update to `cfg`, enforcing `validate()` and
/// Hard Rule #8 (`is_tightening`).
///
/// # Errors
/// Returns a descriptive error string if the key/value is invalid, out of range, or loosens risk.
pub fn tighten_runtime_param(
    current: &BotConfig,
    key: &str,
    value: &str,
) -> Result<BotConfig, String> {
    let parse_dec = |v: &str| {
        Decimal::from_str_exact(v).map_err(|_| format!("invalid decimal value '{v}' for '{key}'"))
    };
    let parse_u32 = |v: &str| {
        v.parse::<u32>()
            .map_err(|_| format!("invalid u32 value '{v}' for '{key}'"))
    };

    let mut candidate = current.clone();
    match key {
        "risk.risk_per_trade_pct" => candidate.risk.risk_per_trade_pct = parse_dec(value)?,
        "risk.max_open_positions" => candidate.risk.max_open_positions = parse_u32(value)?,
        "risk.portfolio_heat_max_pct" => candidate.risk.portfolio_heat_max_pct = parse_dec(value)?,
        "risk.max_notional_per_position_pct" => {
            candidate.risk.max_notional_per_position_pct = parse_dec(value)?;
        }
        "risk.max_total_exposure_pct" => {
            candidate.risk.max_total_exposure_pct = parse_dec(value)?;
        }
        "risk.max_same_direction_correlated" => {
            candidate.risk.max_same_direction_correlated = parse_u32(value)?;
        }
        "risk.daily_loss_halt_pct" => candidate.risk.daily_loss_halt_pct = parse_dec(value)?,
        "risk.weekly_loss_halt_pct" => candidate.risk.weekly_loss_halt_pct = parse_dec(value)?,
        "risk.drawdown_kill_pct" => candidate.risk.drawdown_kill_pct = parse_dec(value)?,
        "risk.max_entry_slippage_bps" => {
            candidate.risk.max_entry_slippage_bps = parse_dec(value)?;
        }
        "risk.max_orders_per_day" => candidate.risk.max_orders_per_day = parse_u32(value)?,
        _ => return Err(format!("parameter '{key}' is not runtime-modifiable")),
    }

    candidate.validate().map_err(|err| err.to_string())?;
    is_tightening(current, &candidate).map_err(|err| err.to_string())?;
    Ok(candidate)
}

/// Disable a strategy by identifier in `cfg`.
///
/// # Errors
/// Returns an error if `id` is unknown.
pub fn disable_strategy_in_config(cfg: &mut BotConfig, id: &str) -> Result<(), String> {
    match id {
        "TrendPullback" | "trend_pullback" | "S1" | "s1" => {
            cfg.strategy.trend_pullback.enabled = false;
            Ok(())
        }
        "Breakout" | "breakout" | "S2" | "s2" => {
            cfg.strategy.breakout.enabled = false;
            Ok(())
        }
        "GainersMomentum" | "gainers_momentum" | "S3" | "s3" => {
            cfg.strategy.gainers_momentum.enabled = false;
            Ok(())
        }
        "VwapReversion" | "vwap_reversion" | "S4" | "s4" => {
            cfg.strategy.vwap_reversion.enabled = false;
            Ok(())
        }
        _ => Err(format!("unknown strategy '{id}'")),
    }
}
