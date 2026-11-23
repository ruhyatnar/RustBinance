//! Core strategy trait, `StrategyEntryIntent`, structured gate logs, and context types (§6.4–§6.5).

use bot_types::{Direction, Kline, Price, StrategyId, Symbol, Venue, VenueCaps};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Actual account commission rates retrieved from Binance `account.commission` via gateway (§6.1, §6.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommissionRates {
    /// Maker fee rate in basis points (e.g. `10` bps = `0.0010` = 0.10%).
    #[serde(with = "rust_decimal::serde::str")]
    pub maker_bps: Decimal,
    /// Taker fee rate in basis points (e.g. `10` bps = `0.0010` = 0.10%).
    #[serde(with = "rust_decimal::serde::str")]
    pub taker_bps: Decimal,
}

impl CommissionRates {
    /// Build [`CommissionRates`] from Binance `account.commission` decimal ratios
    /// (e.g. `"0.00100000"` → `10` bps).
    #[must_use]
    pub fn from_exchange_ratios(maker_ratio: Decimal, taker_ratio: Decimal) -> Self {
        let ten_k = Decimal::from(10_000_u32);
        Self {
            maker_bps: maker_ratio.max(Decimal::ZERO).saturating_mul(ten_k),
            taker_bps: taker_ratio.max(Decimal::ZERO).saturating_mul(ten_k),
        }
    }
}

/// Structured pass/reject record for a single strategy or quality gate (for UI Signals, M08 #5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateLog {
    /// Stable gate identifier (e.g. `"htf_regime"`, `"cost_gate"`, `"btc_regime"`).
    pub gate: String,
    /// `true` if this gate passed.
    pub passed: bool,
    /// Human-readable diagnostic detail.
    pub detail: String,
}

impl GateLog {
    /// Create a passed gate log entry.
    #[must_use]
    pub fn pass(gate: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            passed: true,
            detail: detail.into(),
        }
    }

    /// Create a rejected gate log entry.
    #[must_use]
    pub fn fail(gate: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            gate: gate.into(),
            passed: false,
            detail: detail.into(),
        }
    }
}

/// Pure strategy entry proposal (NO position sizing, NO order submission — §6.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StrategyEntryIntent {
    /// Target venue.
    pub venue: Venue,
    /// Target symbol.
    pub symbol: Symbol,
    /// Trade direction (Spot: Long only).
    pub direction: Direction,
    /// Reference entry price (signal bar close).
    pub entry_ref: Price,
    /// Initial protective stop price.
    pub stop: Price,
    /// First take-profit price (TP1).
    pub tp1: Price,
    /// Optional runner take-profit price (TP2).
    pub tp2: Option<Price>,
    /// Current ATR (in price units) at signal time.
    #[serde(with = "rust_decimal::serde::str")]
    pub atr: Decimal,
    /// Volatility-targeting size multiplier (§6.5, `[0.5, 1.25]`).
    #[serde(with = "rust_decimal::serde::str")]
    pub size_mult: Decimal,
    /// Originating strategy.
    pub strategy: StrategyId,
    /// Human-readable rationale for journal & UI.
    pub rationale: String,
    /// Ordered gate pass/reject audit trail.
    pub gate_logs: Vec<GateLog>,
}

/// Full evaluation output including structured gate logs even when no signal is emitted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalDecision {
    /// Proposed entry intent if all strategy & quality gates passed.
    pub intent: Option<StrategyEntryIntent>,
    /// Complete structured pass/reject audit trail.
    pub gate_logs: Vec<GateLog>,
}

/// Closed Higher-Timeframe (4h) regime state (§6.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HtfRegime {
    /// Latest closed HTF bar close price.
    pub close: f64,
    /// Latest closed HTF `EMA(50)`.
    pub ema50: f64,
    /// `true` if `EMA(50)` has strictly positive slope over the last 3 closed HTF bars.
    pub ema50_rising_3bars: bool,
    /// `true` if `EMA(50)` has strictly negative slope over the last 3 closed HTF bars.
    pub ema50_falling_3bars: bool,
    /// `true` if HTF `EMA(50)` warm-up is complete.
    pub ready: bool,
}

/// Incremental indicator snapshot computed strictly on **closed** signal bars (§6.3–§6.4).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct IndicatorSnapshot {
    /// `EMA(9)` on signal timeframe.
    pub ema9: Option<f64>,
    /// `EMA(21)` on signal timeframe.
    pub ema21: Option<f64>,
    /// `EMA(55)` on signal timeframe.
    pub ema55: Option<f64>,
    /// `EMA(200)` on signal timeframe.
    pub ema200: Option<f64>,
    /// Current `RSI(14)` Wilder.
    pub rsi14: Option<f64>,
    /// Previous closed bar's `RSI(14)`.
    pub prev_rsi14: Option<f64>,
    /// Current `ATR(14)` Wilder.
    pub atr14: Option<f64>,
    /// Current `ADX(14)` Wilder.
    pub adx14: Option<f64>,
    /// Previous closed bar's `ADX(14)`.
    pub prev_adx14: Option<f64>,
    /// Current `+DI(14)`.
    pub plus_di: Option<f64>,
    /// Current `-DI(14)`.
    pub minus_di: Option<f64>,
    /// `Donchian(20)` upper band (strictly previous 20 closed bars — zero look-ahead).
    pub donchian_upper: Option<f64>,
    /// `Donchian(20)` lower band (strictly previous 20 closed bars).
    pub donchian_lower: Option<f64>,
    /// `Donchian(20)` midline.
    pub donchian_mid: Option<f64>,
    /// Session VWAP (00:00 UTC reset).
    pub session_vwap: Option<f64>,
    /// `Volume SMA(20)`.
    pub vol_sma20: Option<f64>,
    /// Minimum low across the last 3 closed bars (including current closed bar).
    pub min_low_3: Option<f64>,
    /// Minimum low across the last 5 closed bars (including current closed bar).
    pub min_low_5: Option<f64>,
}

/// Immutable evaluation context passed to [`Strategy::on_bar_close`] on every **closed** bar (§6.4–§6.5).
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct StrategyCtx {
    /// Target venue.
    pub venue: Venue,
    /// Venue capabilities (Spot vs Futures).
    pub venue_caps: VenueCaps,
    /// Target symbol.
    pub symbol: Symbol,
    /// Newly closed signal bar (`bar.is_closed` MUST be `true`).
    pub bar: Kline,
    /// Incremental indicator snapshot computed strictly up to `bar`.
    pub indicators: IndicatorSnapshot,
    /// Closed HTF (4h) regime state.
    pub htf: HtfRegime,
    /// Current top-of-book spread in basis points.
    pub spread_bps: Decimal,
    /// Age of latest `bookTicker` update in milliseconds (`< 500` ms required).
    pub book_ticker_age_ms: i64,
    /// `true` if the last 300 closed bars had zero gaps (§6.5 #3).
    pub gap_free_300_bars: bool,
    /// `true` if BTC 4h `close > EMA50` AND BTC Crash Guard is NOT active (§6.5 #4).
    pub btc_long_regime_ok: bool,
    /// Target annualized/daily volatility for vol targeting (§6.5 #5).
    pub target_vol: Decimal,
    /// Realized 30-day volatility for vol targeting (§6.5 #5).
    pub realized_vol_30d: Decimal,
    /// 90-day ATR% percentile (`0..=100`; `> 85` caps `size_mult <= 0.6`).
    pub atr_pct_percentile_90d: Decimal,
    /// Whether an open position already exists on `(venue, symbol)`.
    pub has_open_position: bool,
    /// Closed signal bars elapsed since last exit on `(venue, symbol)` (`None` if never exited).
    pub bars_since_last_exit: Option<u32>,
    /// `true` if inside a macro event blackout window (−5m…+2m, §6.5 #7).
    pub macro_blackout_active: bool,
    /// Optional EMA-smoothed top-N Order Book Imbalance (`[-1.0, +1.0]`).
    pub obi_smooth: Option<f64>,
    /// Actual account commission rates from `account.commission` via gateway (§6.1, §6.5).
    pub commission: CommissionRates,
    /// Expected one-way slippage in basis points (used in cost gate §6.5 #1).
    pub expected_slippage_bps: Decimal,
}

/// Core pure strategy interface (§6.4, M08 #1).
///
/// Strategies NEVER size positions and NEVER submit orders; they only propose [`StrategyEntryIntent`].
pub trait Strategy: Send + Sync {
    /// Strategy identifier.
    fn id(&self) -> StrategyId;

    /// Returns `true` if this strategy supports `dir` on a venue with capabilities `caps` (§6.4.1).
    fn supports(&self, caps: &VenueCaps, dir: Direction) -> bool;

    /// Evaluate a closed bar and return an optional [`StrategyEntryIntent`].
    fn on_bar_close(&mut self, ctx: &StrategyCtx) -> Option<StrategyEntryIntent> {
        self.evaluate(ctx).intent
    }

    /// Evaluate a closed bar and return both the optional intent and full structured [`GateLog`]s.
    fn evaluate(&mut self, ctx: &StrategyCtx) -> SignalDecision;
}

/// Safely convert a finite positive `f64` indicator price/ATR into exact `Decimal` (8 dp max).
#[must_use]
pub fn f64_to_decimal(val: f64) -> Option<Decimal> {
    if !val.is_finite() {
        return None;
    }
    Decimal::try_from(val).ok().map(|d| d.round_dp(8))
}
