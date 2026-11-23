//! Conservative fractional Kelly risk reducer and strategy auto-disable (§7.3).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Minimum completed live/paper trades required before Kelly activates (§7.3).
pub const KELLY_MIN_TRADES: usize = 100;

/// Lower-confidence-bound performance inputs for Kelly evaluation (§7.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KellyInput {
    /// Completed trade count for this strategy (live/paper).
    pub trade_count: usize,
    /// Conservative lower-bound win rate `W` in `[0, 1]` (e.g. 25th-percentile bootstrap).
    #[serde(with = "rust_decimal::serde::str")]
    pub lower_win_rate: Decimal,
    /// Conservative lower-bound average win / average loss payoff ratio `R` (`> 0`).
    #[serde(with = "rust_decimal::serde::str")]
    pub lower_payoff_ratio: Decimal,
}

/// Outcome of evaluating conservative quarter-Kelly against configured `risk_per_trade_pct`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KellyDecision {
    /// Fewer than [`KELLY_MIN_TRADES`] trades recorded; keep configured `risk_per_trade_pct`.
    InactiveInsufficientSamples,
    /// Effective risk percentage: `min(base_risk_pct, 0.25 * f * 100)` (reducer only).
    EffectiveRiskPct {
        /// Raw Kelly fraction `f = W - (1 - W) / R`.
        kelly_f: Decimal,
        /// Final risk per trade percentage (never exceeds `base_risk_pct`).
        risk_pct: Decimal,
    },
    /// `f <= 0` (or invalid payoff ratio): strategy **must** be auto-disabled with alarm (§7.3).
    AutoDisableStrategy {
        /// Non-positive Kelly fraction.
        kelly_f: Decimal,
    },
}

/// Evaluate conservative quarter-Kelly as a **reducer only** (§7.3).
///
/// - If `trade_count < 100` → [`KellyDecision::InactiveInsufficientSamples`].
/// - Computes `f = W - (1 - W) / R`.
/// - If `f <= 0` → [`KellyDecision::AutoDisableStrategy`].
/// - Otherwise returns `min(base_risk_pct, 0.25 * f * 100)`.
#[must_use]
pub fn evaluate_kelly(input: KellyInput, base_risk_pct: Decimal) -> KellyDecision {
    if input.trade_count < KELLY_MIN_TRADES {
        return KellyDecision::InactiveInsufficientSamples;
    }

    if input.lower_payoff_ratio <= Decimal::ZERO
        || input.lower_win_rate < Decimal::ZERO
        || input.lower_win_rate > Decimal::ONE
    {
        return KellyDecision::AutoDisableStrategy {
            kelly_f: Decimal::ZERO,
        };
    }

    let loss_rate = Decimal::ONE - input.lower_win_rate;
    let Some(loss_over_r) = loss_rate.checked_div(input.lower_payoff_ratio) else {
        return KellyDecision::AutoDisableStrategy {
            kelly_f: Decimal::ZERO,
        };
    };
    let kelly_f = input.lower_win_rate - loss_over_r;
    if kelly_f <= Decimal::ZERO {
        return KellyDecision::AutoDisableStrategy { kelly_f };
    }

    // 0.25 * f * 100 = 25 * f
    let quarter_kelly_pct = kelly_f
        .checked_mul(Decimal::from(25_u32))
        .unwrap_or(base_risk_pct);
    let risk_pct = base_risk_pct.min(quarter_kelly_pct);

    KellyDecision::EffectiveRiskPct { kelly_f, risk_pct }
}

#[cfg(test)]
mod tests {
    use rust_decimal::Decimal;

    use super::{KELLY_MIN_TRADES, KellyDecision, KellyInput, evaluate_kelly};

    #[test]
    fn kelly_is_inactive_below_100_trades() {
        let res = evaluate_kelly(
            KellyInput {
                trade_count: KELLY_MIN_TRADES - 1,
                lower_win_rate: Decimal::new(30, 2),
                lower_payoff_ratio: Decimal::ONE,
            },
            Decimal::new(5, 1),
        );
        assert_eq!(res, KellyDecision::InactiveInsufficientSamples);
    }

    #[test]
    fn kelly_only_reduces_and_auto_disables_when_non_positive() {
        // W = 0.52, R = 1.0 -> f = 0.04 -> 0.25 * f * 100 = 1.0% -> capped at base 0.5%
        let capped = evaluate_kelly(
            KellyInput {
                trade_count: 150,
                lower_win_rate: Decimal::new(52, 2),
                lower_payoff_ratio: Decimal::ONE,
            },
            Decimal::new(5, 1),
        );
        assert!(matches!(
            capped,
            KellyDecision::EffectiveRiskPct { risk_pct, .. } if risk_pct == Decimal::new(5, 1)
        ));

        // W = 0.505, R = 1.0 -> f = 0.01 -> 0.25 * f * 100 = 0.25% < base 0.5% -> reduced to 0.25%
        let reduced = evaluate_kelly(
            KellyInput {
                trade_count: 150,
                lower_win_rate: Decimal::new(505, 3),
                lower_payoff_ratio: Decimal::ONE,
            },
            Decimal::new(5, 1),
        );
        assert!(matches!(
            reduced,
            KellyDecision::EffectiveRiskPct { risk_pct, .. } if risk_pct == Decimal::new(25, 2)
        ));

        // W = 0.40, R = 1.2 -> f = 0.40 - 0.50 = -0.10 <= 0 -> AutoDisableStrategy
        let disabled = evaluate_kelly(
            KellyInput {
                trade_count: 120,
                lower_win_rate: Decimal::new(40, 2),
                lower_payoff_ratio: Decimal::new(12, 1),
            },
            Decimal::new(5, 1),
        );
        assert!(matches!(
            disabled,
            KellyDecision::AutoDisableStrategy { kelly_f } if kelly_f <= Decimal::ZERO
        ));
    }
}
