#![forbid(unsafe_code)]
//! Deterministic historical replay, §13.2 cost & slippage model, walk-forward validation,
//! and performance reporting using the production `EngineActor`, `PaperBroker`, and `Strategy` (M09).

pub mod data;
pub mod gate_report;
pub mod metrics;
pub mod runner;
pub mod walk_forward;

pub use data::{
    DataError, DataVisionFrequency, GapValidationReport, KlineGap, build_data_vision_checksum_url,
    build_data_vision_kline_url, normalize_binance_timestamp_ms, parse_binance_kline_csv,
    validate_kline_series_gaps, write_binance_kline_csv,
};
pub use metrics::{
    BacktestMetrics, BacktestReport, ParameterSensitivityReport, RDistribution, TradeOutcome,
    compute_backtest_metrics, evaluate_parameter_sensitivity,
};
pub use runner::{
    BacktestConfig, BacktestError, BacktestRunner, BacktestTrace, BarReplayDecision,
    CostModelConfig,
};
pub use walk_forward::{
    WalkForwardConfig, WalkForwardRunner, WalkForwardSlice, WalkForwardSummary,
    WalkForwardWindowResult,
};
