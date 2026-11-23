#![forbid(unsafe_code)]
//! Pure, deterministic, zero-lookahead trading strategies (`bot-strategy`, M08):
//! - S1 Trend-Pullback (Long)
//! - S2 Donchian Breakout (Long)
//! - Shared §6.5 signal quality gates (cost gate with `CommissionRates`, vol targeting, BTC regime)
//! - Shadow-only S3 Gainers Momentum & S4 VWAP Reversion (`ShadowSignal` → `signals_shadow`).

pub mod gates;
pub mod pipeline;
pub mod s1_trend_pullback;
pub mod s2_breakout;
pub mod shadow;
pub mod types;

pub use gates::{
    ATR_PERCENTILE_CAP_THRESHOLD, ATR_PERCENTILE_MAX_MULT, DEFAULT_EXIT_COOLDOWN_BARS,
    MAX_BOOK_TICKER_AGE_MS, VOL_TARGET_MAX_MULT, VOL_TARGET_MIN_MULT, check_cost_gate,
    compute_vol_target_size_mult, evaluate_quality_gates,
};
pub use pipeline::{HtfRegimeFeed, IndicatorFeed};
pub use s1_trend_pullback::S1TrendPullback;
pub use s2_breakout::{S2_MIN_ADX, S2_RSI_MAX, S2_RSI_MIN, S2Breakout};
pub use shadow::{
    GainerCandidate, S3GainersShadow, S4VwapReversionShadow, ShadowOutcome, ShadowSignal,
    evaluate_shadow_outcome, rank_gainers_top_k, record_shadow_signal,
};
pub use types::{
    CommissionRates, GateLog, HtfRegime, IndicatorSnapshot, SignalDecision, Strategy, StrategyCtx,
    StrategyEntryIntent, f64_to_decimal,
};
