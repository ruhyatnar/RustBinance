//! USDⓈ-M Futures pre-trade risk gates (§7.8): funding gate, isolated liquidation estimation &
//! buffer check, and mark↔last divergence guard.

use bot_types::{Price, Side};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Configuration for USDⓈ-M Futures risk gates (§7.8).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FuturesRiskConfig {
    /// Maximum adverse funding rate per 8h interval (e.g. `0.0005` = 0.05%).
    pub max_adverse_funding_rate: Decimal,
    /// Reject new entries within this many milliseconds before next funding settlement.
    pub funding_blackout_before_ms: i64,
    /// Minimum buffer between stop-loss and estimated liquidation price as a fraction of entry
    /// (e.g. `0.02` = 2% extra margin of safety beyond SL).
    pub min_liq_buffer_pct: Decimal,
    /// Maximum allowed `|mark - last| / mark` divergence fraction (e.g. `0.005` = 0.5%).
    pub max_mark_last_divergence_pct: Decimal,
}

/// Inputs evaluated by [`check_futures_entry_gates`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuturesGateInput {
    /// Intended entry direction (`Side::Buy` = Long, `Side::Sell` = Short).
    pub side: Side,
    /// Intended entry price.
    pub entry_price: Price,
    /// Planned initial stop-loss price.
    pub stop_loss_price: Price,
    /// Configured isolated leverage (`>= 1`).
    pub leverage: u32,
    /// Maintenance margin rate from `leverageBracket` (e.g. `0.004` = 0.4%).
    pub maint_margin_rate: Decimal,
    /// Current mark price from `@markPrice@1s`.
    pub mark_price: Price,
    /// Current last traded price.
    pub last_price: Price,
    /// Current funding rate (positive = longs pay shorts; negative = shorts pay longs).
    pub funding_rate: Decimal,
    /// Current UTC timestamp (ms).
    pub now_ms: i64,
    /// Next funding settlement UTC timestamp (ms).
    pub next_funding_time_ms: i64,
}

/// Reason a USDⓈ-M Futures entry was rejected by §7.8 risk gates.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FuturesGateRejection {
    /// Adverse funding rate exceeds configured threshold.
    #[error("adverse funding rate {funding_rate} exceeds max {max_adverse}")]
    AdverseFundingRate {
        /// Observed funding rate.
        funding_rate: Decimal,
        /// Configured ceiling.
        max_adverse: Decimal,
    },
    /// Entry is too close to the next funding settlement window.
    #[error("within funding blackout window ({remaining_ms}ms < {blackout_ms}ms)")]
    FundingBlackout {
        /// Milliseconds remaining until funding.
        remaining_ms: i64,
        /// Configured blackout window.
        blackout_ms: i64,
    },
    /// Stop-loss does not have sufficient safety buffer before estimated liquidation price.
    #[error("insufficient liquidation buffer: sl={stop_loss}, liq={liquidation_price}")]
    InsufficientLiquidationBuffer {
        /// Planned stop-loss price.
        stop_loss: Decimal,
        /// Estimated isolated liquidation price.
        liquidation_price: Decimal,
    },
    /// Mark price and last price have diverged beyond the safe threshold.
    #[error("mark/last divergence {divergence_pct} exceeds max {max_pct}")]
    MarkLastDivergence {
        /// Observed `|mark - last| / mark`.
        divergence_pct: Decimal,
        /// Configured maximum divergence.
        max_pct: Decimal,
    },
    /// Invalid leverage or non-positive prices.
    #[error("invalid futures gate input: {0}")]
    InvalidInput(String),
}

/// Conservative isolated liquidation price estimate for a single USDⓈ-M Futures position (§7.8).
///
/// - Long (`Side::Buy`): `entry * (1 - 1/leverage + maint_margin_rate)`
/// - Short (`Side::Sell`): `entry * (1 + 1/leverage - maint_margin_rate)`
///
/// # Errors
/// Returns [`FuturesGateRejection::InvalidInput`] if `leverage == 0` or `entry_price <= 0`.
pub fn estimate_isolated_liquidation_price(
    side: Side,
    entry_price: Price,
    leverage: u32,
    maint_margin_rate: Decimal,
) -> Result<Price, FuturesGateRejection> {
    if leverage == 0 || entry_price.0 <= Decimal::ZERO {
        return Err(FuturesGateRejection::InvalidInput(
            "leverage and entry_price must be positive".into(),
        ));
    }
    let inv_lev = Decimal::ONE / Decimal::from(leverage);
    let factor = match side {
        Side::Buy => (Decimal::ONE - inv_lev + maint_margin_rate).max(Decimal::ZERO),
        Side::Sell => Decimal::ONE + inv_lev - maint_margin_rate,
    };
    Ok(Price(entry_price.0 * factor))
}

/// Evaluate all §7.8 USDⓈ-M Futures pre-trade gates (mark↔last, funding, liquidation buffer).
///
/// # Errors
/// Returns [`FuturesGateRejection`] on the first violated safety rule.
pub fn check_futures_entry_gates(
    input: &FuturesGateInput,
    cfg: &FuturesRiskConfig,
) -> Result<Price, FuturesGateRejection> {
    if input.mark_price.0 <= Decimal::ZERO || input.entry_price.0 <= Decimal::ZERO {
        return Err(FuturesGateRejection::InvalidInput(
            "mark_price and entry_price must be positive".into(),
        ));
    }

    // 1. Mark <-> Last divergence guard
    let divergence = (input.mark_price.0 - input.last_price.0).abs() / input.mark_price.0;
    if divergence > cfg.max_mark_last_divergence_pct {
        return Err(FuturesGateRejection::MarkLastDivergence {
            divergence_pct: divergence,
            max_pct: cfg.max_mark_last_divergence_pct,
        });
    }

    // 2. Funding rate gate (adverse direction only: positive hurts Long, negative hurts Short)
    let adverse_funding = match input.side {
        Side::Buy => input.funding_rate,
        Side::Sell => -input.funding_rate,
    };
    if adverse_funding > cfg.max_adverse_funding_rate {
        return Err(FuturesGateRejection::AdverseFundingRate {
            funding_rate: input.funding_rate,
            max_adverse: cfg.max_adverse_funding_rate,
        });
    }
    let remaining_ms = input.next_funding_time_ms.saturating_sub(input.now_ms);
    if remaining_ms >= 0 && remaining_ms < cfg.funding_blackout_before_ms && adverse_funding > Decimal::ZERO {
        return Err(FuturesGateRejection::FundingBlackout {
            remaining_ms,
            blackout_ms: cfg.funding_blackout_before_ms,
        });
    }

    // 3. Isolated liquidation price & buffer vs SL
    let liq = estimate_isolated_liquidation_price(
        input.side,
        input.entry_price,
        input.leverage,
        input.maint_margin_rate,
    )?;
    let required_buffer = input.entry_price.0 * cfg.min_liq_buffer_pct;
    let buffer_ok = match input.side {
        Side::Buy => input.stop_loss_price.0 >= liq.0 + required_buffer,
        Side::Sell => input.stop_loss_price.0 + required_buffer <= liq.0,
    };
    if !buffer_ok {
        return Err(FuturesGateRejection::InsufficientLiquidationBuffer {
            stop_loss: input.stop_loss_price.0,
            liquidation_price: liq.0,
        });
    }

    Ok(liq)
}
