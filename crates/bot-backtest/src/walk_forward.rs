//! Walk-forward validation (default 12-month train / 3-month test / 3-month rolling step),
//! per-window out-of-sample report tracking, and multiple-testing trial counting (§13.2, M09 #4).

use bot_marketdata::Timeframe;
use bot_strategy::S1TrendPullback;
use bot_types::Kline;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::metrics::BacktestReport;
use crate::runner::{BacktestConfig, BacktestError, BacktestRunner};

/// Rolling walk-forward window configuration (§13.2: train 12 months / test 3 months / step 3 months).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkForwardConfig {
    /// Number of closed bars in each in-sample training window.
    pub train_bars: usize,
    /// Number of closed bars in each out-of-sample test window.
    pub test_bars: usize,
    /// Rolling step size in closed bars between consecutive windows.
    pub step_bars: usize,
}

/// Index slice boundaries `(train_start..train_end, test_start..test_end)` for one walk-forward window.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkForwardSlice {
    /// 0-based window index.
    pub window_index: usize,
    /// Inclusive train start index.
    pub train_start: usize,
    /// Exclusive train end index (`== test_start`).
    pub train_end: usize,
    /// Inclusive test start index.
    pub test_start: usize,
    /// Exclusive test end index.
    pub test_end: usize,
}

/// Out-of-sample and in-sample evaluation result for a single walk-forward window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkForwardWindowResult {
    /// 0-based window index.
    pub window_index: usize,
    /// Slice indices in the input bar series.
    pub slice: WalkForwardSlice,
    /// Number of parameter combinations evaluated during in-sample training on this window.
    pub parameter_trials: usize,
    /// Selected parameter value (`tp2_r` candidate) from the training window.
    #[serde(with = "rust_decimal::serde::str")]
    pub selected_param: Decimal,
    /// In-sample training report for the selected parameter.
    pub train_report: BacktestReport,
    /// Out-of-sample test report evaluated on the unseen test window.
    pub test_report: BacktestReport,
}

/// Complete walk-forward validation summary across all rolling windows (§13.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalkForwardSummary {
    /// Per-window results (reporting the full distribution across windows, not a single cherry-picked number).
    pub windows: Vec<WalkForwardWindowResult>,
    /// Total parameter trials evaluated across all windows (for multiple-testing audit).
    pub total_parameter_trials: usize,
    /// Median out-of-sample Profit Factor across windows.
    #[serde(with = "rust_decimal::serde::str")]
    pub median_test_profit_factor: Decimal,
    /// Median out-of-sample Expectancy (R) across windows.
    #[serde(with = "rust_decimal::serde::str")]
    pub median_test_expectancy_r: Decimal,
    /// Worst (maximum) out-of-sample Drawdown (%) across all test windows.
    #[serde(with = "rust_decimal::serde::str")]
    pub worst_test_max_drawdown_pct: Decimal,
}

impl WalkForwardConfig {
    /// Standard §13.2 walk-forward schedule: 12 months (365d) train, 3 months (90d) test, 3 months (90d) step.
    #[must_use]
    pub fn default_12m_3m_3m(timeframe: Timeframe) -> Self {
        let interval_ms = timeframe.interval_ms().max(1);
        let day_ms: i64 = 86_400_000;
        let bars_per_day = usize::try_from(day_ms / interval_ms).unwrap_or(24).max(1);
        Self {
            train_bars: bars_per_day * 365,
            test_bars: bars_per_day * 90,
            step_bars: bars_per_day * 90,
        }
    }

    /// Compute all valid rolling [`WalkForwardSlice`]s for a dataset of `total_bars`.
    #[must_use]
    pub fn slice_windows(&self, total_bars: usize) -> Vec<WalkForwardSlice> {
        let window_len = self.train_bars.saturating_add(self.test_bars);
        if self.train_bars == 0
            || self.test_bars == 0
            || self.step_bars == 0
            || total_bars < window_len
        {
            return Vec::new();
        }
        let mut slices = Vec::new();
        let mut start = 0_usize;
        let mut window_index = 0_usize;

        while start.saturating_add(window_len) <= total_bars {
            let train_end = start + self.train_bars;
            let test_end = train_end + self.test_bars;
            slices.push(WalkForwardSlice {
                window_index,
                train_start: start,
                train_end,
                test_start: train_end,
                test_end,
            });
            window_index += 1;
            start = start.saturating_add(self.step_bars);
        }
        slices
    }
}

/// Walk-forward validation runner (§13.2).
pub struct WalkForwardRunner;

impl WalkForwardRunner {
    /// Execute rolling walk-forward validation for S1 (`S1TrendPullback`) over `tp2_r_candidates`.
    ///
    /// On each window:
    /// 1. Evaluates every candidate in `tp2_r_candidates` on the training slice `[train_start..train_end]`,
    ///    counting every parameter trial.
    /// 2. Selects the parameter with the highest in-sample expectancy (tie-broken deterministically).
    /// 3. Evaluates the selected parameter on the unseen test window (with warm-up prefix from training
    ///    so EMA200 is warmed up without leaking future test bars).
    ///
    /// # Errors
    /// Returns [`BacktestError`] if any window replay fails.
    pub async fn run_s1(
        base_cfg: &BacktestConfig,
        wf_cfg: &WalkForwardConfig,
        tp2_r_candidates: &[Decimal],
        bars: &[Kline],
    ) -> Result<WalkForwardSummary, BacktestError> {
        let candidates = if tp2_r_candidates.is_empty() {
            vec![base_cfg.bot_config.strategy.trend_pullback.tp2_r]
        } else {
            tp2_r_candidates.to_vec()
        };
        let slices = wf_cfg.slice_windows(bars.len());
        let mut windows = Vec::with_capacity(slices.len());
        let mut total_parameter_trials = 0_usize;

        for slice in slices {
            let Some(train_bars) = bars.get(slice.train_start..slice.train_end) else {
                continue;
            };
            let Some(full_window_bars) = bars.get(slice.train_start..slice.test_end) else {
                continue;
            };

            let trials_this_window = candidates.len();
            total_parameter_trials = total_parameter_trials.saturating_add(trials_this_window);

            let mut best_param = candidates.first().copied().unwrap_or(Decimal::from(3_u32));
            let mut best_train_rep: Option<BacktestReport> = None;

            for &param_tp2_r in &candidates {
                let mut cfg = base_cfg.clone();
                cfg.bot_config.strategy.trend_pullback.tp2_r = param_tp2_r;
                let mut strat = S1TrendPullback::new(
                    cfg.bot_config.strategy.trend_pullback.clone(),
                    Decimal::from(5_u32),
                );
                let trace = BacktestRunner::run_with_trace_and_trials(
                    &cfg,
                    &mut strat,
                    train_bars,
                    trials_this_window,
                )
                .await?;

                let is_better = best_train_rep.as_ref().is_none_or(|prev| {
                    trace.report.metrics.expectancy_r > prev.metrics.expectancy_r
                });
                if is_better {
                    best_param = param_tp2_r;
                    best_train_rep = Some(trace.report);
                }
            }

            let Some(train_report) = best_train_rep else {
                continue;
            };

            // Out-of-sample evaluation on test window.
            let mut test_cfg = base_cfg.clone();
            test_cfg.bot_config.strategy.trend_pullback.tp2_r = best_param;
            let mut test_strat = S1TrendPullback::new(
                test_cfg.bot_config.strategy.trend_pullback.clone(),
                Decimal::from(5_u32),
            );
            let test_trace = BacktestRunner::run_with_trace_and_trials(
                &test_cfg,
                &mut test_strat,
                full_window_bars,
                total_parameter_trials,
            )
            .await?;

            windows.push(WalkForwardWindowResult {
                window_index: slice.window_index,
                slice,
                parameter_trials: trials_this_window,
                selected_param: best_param,
                train_report,
                test_report: test_trace.report,
            });
        }

        let mut test_pfs: Vec<Decimal> = windows
            .iter()
            .map(|w| w.test_report.metrics.profit_factor)
            .collect();
        test_pfs.sort();

        let mut test_exps: Vec<Decimal> = windows
            .iter()
            .map(|w| w.test_report.metrics.expectancy_r)
            .collect();
        test_exps.sort();

        let median_pf = test_pfs
            .get(test_pfs.len() / 2)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let median_exp = test_exps
            .get(test_exps.len() / 2)
            .copied()
            .unwrap_or(Decimal::ZERO);
        let worst_dd = windows
            .iter()
            .map(|w| w.test_report.metrics.max_drawdown_pct)
            .max()
            .unwrap_or(Decimal::ZERO);

        Ok(WalkForwardSummary {
            windows,
            total_parameter_trials,
            median_test_profit_factor: median_pf,
            median_test_expectancy_r: median_exp,
            worst_test_max_drawdown_pct: worst_dd,
        })
    }
}
