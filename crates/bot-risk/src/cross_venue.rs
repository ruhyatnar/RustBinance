//! Cross-venue risk budget, combined equity, combined portfolio heat, and
//! same-symbol opposite-direction guard (§3.5, §7.8).

use bot_types::{Direction, Symbol, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Policy governing same-symbol positions across Spot and Futures (§3.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrossVenueSameSymbolPolicy {
    /// Block opening an opposite direction on the same symbol in another venue (default).
    #[default]
    BlockOpposite,
    /// Allow opposite-direction positions across venues.
    Allow,
}

/// Minimal open-position descriptor used for cross-venue risk aggregation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CrossVenuePosition {
    /// Venue holding the position.
    pub venue: Venue,
    /// Trading symbol.
    pub symbol: Symbol,
    /// Position direction (`Long` or `Short`).
    pub direction: Direction,
    /// Open risk in USD (`qty * |entry - stop|`).
    pub risk_usd: Decimal,
}

/// Stateless cross-venue risk budget helpers (§7.8).
pub struct CrossVenueBudget;

impl CrossVenueBudget {
    /// Combined equity across Spot and Futures margin balance in USD (§7.8).
    #[must_use]
    pub fn combined_equity_usd(spot_equity: Decimal, futures_equity: Decimal) -> Decimal {
        spot_equity
            .max(Decimal::ZERO)
            .saturating_add(futures_equity.max(Decimal::ZERO))
    }

    /// Effective equity available for sizing on a single venue:
    /// `min(venue_equity, capital_allocation_usd)` when `capital_allocation_usd > 0` (§3.5, §7.8).
    #[must_use]
    pub fn effective_venue_equity(venue_equity: Decimal, capital_allocation_usd: Decimal) -> Decimal {
        let clamped_equity = venue_equity.max(Decimal::ZERO);
        if capital_allocation_usd > Decimal::ZERO {
            clamped_equity.min(capital_allocation_usd)
        } else {
            clamped_equity
        }
    }

    /// Combined portfolio heat percentage across all open positions in both venues
    /// (`Σ risk_usd / combined_equity_usd * 100`).
    #[must_use]
    pub fn combined_portfolio_heat_pct(
        positions: &[CrossVenuePosition],
        combined_equity_usd: Decimal,
    ) -> Decimal {
        if combined_equity_usd <= Decimal::ZERO {
            return Decimal::ZERO;
        }
        let total_risk: Decimal = positions
            .iter()
            .map(|p| p.risk_usd.max(Decimal::ZERO))
            .fold(Decimal::ZERO, Decimal::saturating_add);
        total_risk
            .saturating_mul(Decimal::from(100))
            .checked_div(combined_equity_usd)
            .unwrap_or(Decimal::ZERO)
    }

    /// Enforce `cross_venue_same_symbol` policy before opening a new position (§3.5).
    ///
    /// # Errors
    /// Returns an error string if `policy == BlockOpposite` and another venue already holds
    /// an open position on `candidate_symbol` in the opposite direction.
    pub fn check_same_symbol(
        policy: CrossVenueSameSymbolPolicy,
        existing: &[CrossVenuePosition],
        candidate_venue: Venue,
        candidate_symbol: &Symbol,
        candidate_dir: Direction,
    ) -> Result<(), String> {
        if policy == CrossVenueSameSymbolPolicy::Allow {
            return Ok(());
        }
        for pos in existing {
            if pos.venue != candidate_venue
                && pos.symbol == *candidate_symbol
                && pos.direction != candidate_dir
            {
                return Err(format!(
                    "cross_venue_same_symbol=block_opposite: {candidate_symbol} already open {:?} on {:?}",
                    pos.direction, pos.venue
                ));
            }
        }
        Ok(())
    }
}
