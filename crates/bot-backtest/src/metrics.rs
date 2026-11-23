//! Backtest performance metrics, R-distribution, Deflated Sharpe multiple-testing correction,
//! ±20% parameter neighborhood sensitivity, and profit-factor without top-3 trades (§13.2–§13.3).

use bot_types::{StrategyId, Symbol, Venue};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

/// Outcome of a single completed round-trip trade in the backtester.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeOutcome {
    /// Entry timestamp (ms).
    pub entry_time_ms: i64,
    /// Exit timestamp (ms).
    pub exit_time_ms: i64,
    /// Net realized `PnL` in quote currency (USD/USDT) after maker/taker fees and slippage.
    #[serde(with = "rust_decimal::serde::str")]
    pub net_pnl_usd: Decimal,
    /// Realized R-multiple (`net_pnl_usd / initial_risk_usd`).
    #[serde(with = "rust_decimal::serde::str")]
    pub r_multiple: Decimal,
    /// Entry notional in USD.
    #[serde(with = "rust_decimal::serde::str")]
    pub notional_usd: Decimal,
    /// Total round-trip fees paid in USD.
    #[serde(with = "rust_decimal::serde::str")]
    pub fees_usd: Decimal,
}

/// R-multiple distribution summary across completed trades (M09 #5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RDistribution {
    /// Minimum R-multiple (worst trade).
    #[serde(with = "rust_decimal::serde::str")]
    pub min_r: Decimal,
    /// 25th percentile R-multiple.
    #[serde(with = "rust_decimal::serde::str")]
    pub p25_r: Decimal,
    /// Median (50th percentile) R-multiple.
    #[serde(with = "rust_decimal::serde::str")]
    pub median_r: Decimal,
    /// 75th percentile R-multiple.
    #[serde(with = "rust_decimal::serde::str")]
    pub p75_r: Decimal,
    /// Maximum R-multiple (best trade).
    #[serde(with = "rust_decimal::serde::str")]
    pub max_r: Decimal,
}

impl Default for RDistribution {
    fn default() -> Self {
        Self {
            min_r: Decimal::ZERO,
            p25_r: Decimal::ZERO,
            median_r: Decimal::ZERO,
            p75_r: Decimal::ZERO,
            max_r: Decimal::ZERO,
        }
    }
}

/// Parameter neighborhood (±20%) sensitivity report (§13.3 G1: >= 70% of neighbors PF > 1.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ParameterSensitivityReport {
    /// Total parameter neighbor variations evaluated.
    pub total_neighbors: usize,
    /// Number of neighbor variations achieving `profit_factor > min_pf_threshold`.
    pub passing_neighbors: usize,
    /// Percentage (`0..=100`) of neighbors passing `min_pf_threshold`.
    #[serde(with = "rust_decimal::serde::str")]
    pub passing_ratio_pct: Decimal,
}

/// Comprehensive backtest performance metrics (§13.2–§13.3, M09 #5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacktestMetrics {
    /// Total completed trades.
    pub total_trades: usize,
    /// Winning trades (`net_pnl_usd > 0`).
    pub winning_trades: usize,
    /// Losing trades (`net_pnl_usd <= 0`).
    pub losing_trades: usize,
    /// Net realized `PnL` (USD) after fees & slippage.
    #[serde(with = "rust_decimal::serde::str")]
    pub net_pnl_usd: Decimal,
    /// Profit Factor (`gross_profit / |gross_loss|`; capped at `999.0` when `gross_loss == 0`).
    #[serde(with = "rust_decimal::serde::str")]
    pub profit_factor: Decimal,
    /// Profit Factor after removing the 3 highest-profit trades (§13.3 G1: must remain `> 1.0`).
    #[serde(with = "rust_decimal::serde::str")]
    pub profit_factor_without_top3: Decimal,
    /// Trade expectancy in R-multiples (`mean(r_multiple)`).
    #[serde(with = "rust_decimal::serde::str")]
    pub expectancy_r: Decimal,
    /// Maximum peak-to-trough equity drawdown percentage (`0..=100`).
    #[serde(with = "rust_decimal::serde::str")]
    pub max_drawdown_pct: Decimal,
    /// Annualized Sharpe ratio over trade/bar returns.
    #[serde(with = "rust_decimal::serde::str")]
    pub sharpe_ratio: Decimal,
    /// Annualized Sortino ratio (downside deviation only).
    #[serde(with = "rust_decimal::serde::str")]
    pub sortino_ratio: Decimal,
    /// Deflated Sharpe adjustment factoring in `parameter_trials` multiple testing (§13.2).
    #[serde(with = "rust_decimal::serde::str")]
    pub deflated_sharpe: Decimal,
    /// Portfolio turnover multiple (`total_traded_notional / initial_equity`).
    #[serde(with = "rust_decimal::serde::str")]
    pub turnover_multiple: Decimal,
    /// Time-in-market exposure percentage (`0..=100`).
    #[serde(with = "rust_decimal::serde::str")]
    pub exposure_pct: Decimal,
    /// R-multiple distribution percentiles.
    pub r_distribution: RDistribution,
}

/// Full reproducible report returned by [`crate::BacktestRunner`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BacktestReport {
    /// Venue tested.
    pub venue: Venue,
    /// Symbol tested.
    pub symbol: Symbol,
    /// Strategy profile tested.
    pub strategy: StrategyId,
    /// Deterministic run seed.
    pub seed: u64,
    /// Number of closed bars processed.
    pub bars_processed: usize,
    /// Total signals proposed by the strategy.
    pub total_signals_emitted: usize,
    /// Total entry orders accepted by `bot-risk::Gatekeeper`.
    pub total_entries_submitted: usize,
    /// Starting equity (USD).
    #[serde(with = "rust_decimal::serde::str")]
    pub initial_equity_usd: Decimal,
    /// Final equity (USD).
    #[serde(with = "rust_decimal::serde::str")]
    pub final_equity_usd: Decimal,
    /// Completed trade list.
    pub trades: Vec<TradeOutcome>,
    /// Computed performance metrics.
    pub metrics: BacktestMetrics,
}

/// Evaluate ±20% parameter neighborhood stability (§13.3 G1).
#[must_use]
pub fn evaluate_parameter_sensitivity(
    neighbor_profit_factors: &[Decimal],
    min_pf_threshold: Decimal,
) -> ParameterSensitivityReport {
    let total = neighbor_profit_factors.len();
    if total == 0 {
        return ParameterSensitivityReport {
            total_neighbors: 0,
            passing_neighbors: 0,
            passing_ratio_pct: Decimal::ZERO,
        };
    }
    let passing = neighbor_profit_factors
        .iter()
        .filter(|&&pf| pf > min_pf_threshold)
        .count();
    let ratio_pct = (Decimal::from(passing) * Decimal::from(100_u32))
        .checked_div(Decimal::from(total))
        .unwrap_or(Decimal::ZERO)
        .round_dp(2);

    ParameterSensitivityReport {
        total_neighbors: total,
        passing_neighbors: passing,
        passing_ratio_pct: ratio_pct,
    }
}

/// Compute comprehensive [`BacktestMetrics`] from completed trades and equity curve (§13.2–§13.3).
#[allow(clippy::too_many_lines)]
#[must_use]
pub fn compute_backtest_metrics(
    initial_equity_usd: Decimal,
    trades: &[TradeOutcome],
    equity_curve: &[Decimal],
    total_horizon_ms: i64,
    exposed_duration_ms: i64,
    parameter_trials: usize,
) -> BacktestMetrics {
    let total_trades = trades.len();
    let mut net_pnl_usd = Decimal::ZERO;
    let mut gross_profit = Decimal::ZERO;
    let mut gross_loss_abs = Decimal::ZERO;
    let mut winning_trades = 0_usize;
    let mut losing_trades = 0_usize;
    let mut r_sum = Decimal::ZERO;
    let mut total_notional = Decimal::ZERO;
    let mut r_values: Vec<Decimal> = Vec::with_capacity(total_trades);
    let mut positive_pnls: Vec<Decimal> = Vec::new();

    for t in trades {
        net_pnl_usd = net_pnl_usd.saturating_add(t.net_pnl_usd);
        r_sum = r_sum.saturating_add(t.r_multiple);
        total_notional = total_notional.saturating_add(t.notional_usd);
        r_values.push(t.r_multiple);

        if t.net_pnl_usd > Decimal::ZERO {
            winning_trades += 1;
            gross_profit = gross_profit.saturating_add(t.net_pnl_usd);
            positive_pnls.push(t.net_pnl_usd);
        } else {
            losing_trades += 1;
            gross_loss_abs = gross_loss_abs.saturating_add(t.net_pnl_usd.abs());
        }
    }

    let profit_factor = compute_pf(gross_profit, gross_loss_abs);

    // Profit Factor without top 3 best trades (§13.3 G1).
    positive_pnls.sort_by(|a, b| b.cmp(a));
    let top3_sum: Decimal = positive_pnls.iter().take(3).copied().sum();
    let gross_profit_no_top3 = gross_profit.saturating_sub(top3_sum).max(Decimal::ZERO);
    let profit_factor_without_top3 = compute_pf(gross_profit_no_top3, gross_loss_abs);

    let expectancy_r = if total_trades > 0 {
        r_sum
            .checked_div(Decimal::from(total_trades))
            .unwrap_or(Decimal::ZERO)
            .round_dp(4)
    } else {
        Decimal::ZERO
    };

    // Peak-to-trough max drawdown (%).
    let mut peak = initial_equity_usd;
    let mut max_dd_pct = Decimal::ZERO;
    for &eq in equity_curve {
        if eq > peak {
            peak = eq;
        }
        if peak > Decimal::ZERO && eq < peak {
            let dd = ((peak - eq) * Decimal::from(100_u32))
                .checked_div(peak)
                .unwrap_or(Decimal::ZERO);
            if dd > max_dd_pct {
                max_dd_pct = dd;
            }
        }
    }

    // R-distribution percentiles.
    r_values.sort();
    let r_distribution = if r_values.is_empty() {
        RDistribution::default()
    } else {
        RDistribution {
            min_r: percentile_sorted(&r_values, 0),
            p25_r: percentile_sorted(&r_values, 25),
            median_r: percentile_sorted(&r_values, 50),
            p75_r: percentile_sorted(&r_values, 75),
            max_r: percentile_sorted(&r_values, 100),
        }
    };

    // Sharpe, Sortino, and Deflated Sharpe over trade R-multiples.
    let (sharpe_ratio, sortino_ratio, deflated_sharpe) =
        compute_sharpe_sortino_deflated(&r_values, parameter_trials);

    let turnover_multiple = if initial_equity_usd > Decimal::ZERO {
        total_notional
            .checked_div(initial_equity_usd)
            .unwrap_or(Decimal::ZERO)
            .round_dp(4)
    } else {
        Decimal::ZERO
    };

    let exposure_pct = if total_horizon_ms > 0 {
        let clamped_exp = exposed_duration_ms.clamp(0, total_horizon_ms);
        (Decimal::from(clamped_exp) * Decimal::from(100_u32))
            .checked_div(Decimal::from(total_horizon_ms))
            .unwrap_or(Decimal::ZERO)
            .round_dp(2)
    } else {
        Decimal::ZERO
    };

    BacktestMetrics {
        total_trades,
        winning_trades,
        losing_trades,
        net_pnl_usd: net_pnl_usd.round_dp(4),
        profit_factor,
        profit_factor_without_top3,
        expectancy_r,
        max_drawdown_pct: max_dd_pct.round_dp(4),
        sharpe_ratio,
        sortino_ratio,
        deflated_sharpe,
        turnover_multiple,
        exposure_pct,
        r_distribution,
    }
}

fn compute_pf(gross_profit: Decimal, gross_loss_abs: Decimal) -> Decimal {
    if gross_loss_abs > Decimal::ZERO {
        gross_profit
            .checked_div(gross_loss_abs)
            .unwrap_or(Decimal::ZERO)
            .round_dp(4)
    } else if gross_profit > Decimal::ZERO {
        Decimal::from(999_u32)
    } else {
        Decimal::ZERO
    }
}

fn percentile_sorted(sorted: &[Decimal], pct: usize) -> Decimal {
    if sorted.is_empty() {
        return Decimal::ZERO;
    }
    let max_idx = sorted.len().saturating_sub(1);
    let idx = (max_idx.saturating_mul(pct)) / 100;
    sorted.get(idx).copied().unwrap_or(Decimal::ZERO)
}

#[allow(clippy::cast_precision_loss)]
fn compute_sharpe_sortino_deflated(
    r_values: &[Decimal],
    parameter_trials: usize,
) -> (Decimal, Decimal, Decimal) {
    if r_values.len() < 2 {
        return (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    }
    let vals: Vec<f64> = r_values.iter().filter_map(ToPrimitive::to_f64).collect();
    if vals.len() < 2 {
        return (Decimal::ZERO, Decimal::ZERO, Decimal::ZERO);
    }
    let n = vals.len() as f64;
    let mean = vals.iter().sum::<f64>() / n;
    let var = vals
        .iter()
        .map(|&x| {
            let d = x - mean;
            d * d
        })
        .sum::<f64>()
        / (n - 1.0);
    let std_dev = var.sqrt();

    let downside_var = vals
        .iter()
        .map(|&x| {
            let d = x.min(0.0);
            d * d
        })
        .sum::<f64>()
        / n;
    let downside_dev = downside_var.sqrt();

    let ann_factor = n.min(252.0).sqrt();
    let sharpe_f = if std_dev > 1e-9 {
        (mean / std_dev) * ann_factor
    } else {
        0.0
    };
    let sortino_f = if downside_dev > 1e-9 {
        (mean / downside_dev) * ann_factor
    } else if mean > 0.0 {
        99.0
    } else {
        0.0
    };

    // Bailey & Lopez de Prado multiple-testing haircut penalty: sqrt(2 * ln(trials)).
    let trials = parameter_trials.max(1) as f64;
    let haircut = if trials > 1.0 {
        (2.0 * trials.ln()).sqrt()
    } else {
        0.0
    };
    let deflated_f = sharpe_f - haircut;

    let to_dec = |v: f64| -> Decimal {
        if v.is_finite() {
            Decimal::try_from(v).unwrap_or(Decimal::ZERO).round_dp(4)
        } else {
            Decimal::ZERO
        }
    };

    (to_dec(sharpe_f), to_dec(sortino_f), to_dec(deflated_f))
}
