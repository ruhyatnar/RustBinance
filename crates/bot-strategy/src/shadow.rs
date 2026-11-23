//! Shadow-only strategy framework for S3 (Gainers Momentum) and S4 (VWAP Reversion) (§6.4, M08).
//!
//! Architectural safety guarantee: [`ShadowStrategy`] returns [`ShadowSignal`] (never
//! [`crate::StrategyEntryIntent`]), making it structurally impossible for shadow strategies
//! to emit executable orders.

use bot_config::{GainersMomentumConfig, VwapReversionConfig};
use bot_store::{SqliteStore, StoreError};
use bot_types::{Direction, Price, StrategyId, Symbol, Venue};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;
use serde::{Deserialize, Serialize};

use crate::types::{GateLog, StrategyCtx, f64_to_decimal};

/// Shadow-only signal recorded into `signals_shadow` — CANNOT be submitted as an executable order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowSignal {
    /// Originating shadow strategy (`GainersMomentum` or `VwapReversion`).
    pub strategy: StrategyId,
    /// Target venue.
    pub venue: Venue,
    /// Target symbol.
    pub symbol: Symbol,
    /// Direction.
    pub direction: Direction,
    /// Signal bar close timestamp (ms).
    pub ts_ms: i64,
    /// Hypothetical entry price.
    pub entry_ref: Price,
    /// Hypothetical stop price.
    pub stop: Price,
    /// Hypothetical TP1 price.
    pub tp1: Price,
    /// Optional scanner score (z-score composite for S3).
    pub score_bps: Option<i64>,
    /// Diagnostic rationale.
    pub rationale: String,
    /// Structured gate logs.
    pub gate_logs: Vec<GateLog>,
}

/// Hypothetical outcome of a recorded shadow signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShadowOutcome {
    /// Still open / unresolved.
    Pending,
    /// Hypothetical price hit TP1 first.
    HitTp1,
    /// Hypothetical price hit stop first.
    HitStop,
    /// Expired at time stop.
    TimeStop,
}

impl ShadowOutcome {
    /// Stable string tag stored in `signals_shadow.outcome`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "PENDING",
            Self::HitTp1 => "HIT_TP1",
            Self::HitStop => "HIT_STOP",
            Self::TimeStop => "TIME_STOP",
        }
    }
}

/// Evaluate hypothetical outcome of a Long [`ShadowSignal`] against a subsequent bar `(high, low)`.
#[must_use]
pub fn evaluate_shadow_outcome(
    signal: &ShadowSignal,
    bar_high: Price,
    bar_low: Price,
    expired: bool,
) -> ShadowOutcome {
    // Conservative ordering: if stop and TP both touched in same bar, assume stop hit first.
    if bar_low.as_decimal() <= signal.stop.as_decimal() {
        return ShadowOutcome::HitStop;
    }
    if bar_high.as_decimal() >= signal.tp1.as_decimal() {
        return ShadowOutcome::HitTp1;
    }
    if expired {
        return ShadowOutcome::TimeStop;
    }
    ShadowOutcome::Pending
}

/// Persist a [`ShadowSignal`] and its hypothetical [`ShadowOutcome`] into SQLite `signals_shadow`.
///
/// # Errors
/// Returns [`StoreError`] on serialization or SQLite write failure.
pub fn record_shadow_signal(
    store: &mut SqliteStore,
    signal: &ShadowSignal,
    outcome: ShadowOutcome,
) -> Result<i64, StoreError> {
    let payload = serde_json::to_string(signal)?;
    let strat_str = match signal.strategy {
        StrategyId::GainersMomentum => "gainers_momentum",
        StrategyId::VwapReversion => "vwap_reversion",
        StrategyId::TrendPullback => "trend_pullback",
        StrategyId::Breakout => "breakout",
    };
    store.insert_shadow_signal(
        signal.ts_ms,
        strat_str,
        &signal.symbol,
        &payload,
        outcome.as_str(),
    )
}

/// Candidate metrics for S3 Gainers Momentum z-score ranking (§6.4).
#[derive(Debug, Clone)]
pub struct GainerCandidate {
    /// Symbol.
    pub symbol: Symbol,
    /// 24-hour return in `%` (e.g. `12.5` = +12.5%).
    pub return_24h_pct: f64,
    /// 4-hour return in `%`.
    pub return_4h_pct: f64,
    /// Ratio of 1h volume to 24h hourly average volume.
    pub vol_ratio_1h_24h: f64,
}

/// Rank S3 gainers candidates by `z(capped_ret_24h) + z(ret_4h) + z(vol_ratio)` and return top `k` (§6.4).
#[must_use]
pub fn rank_gainers_top_k(candidates: &[GainerCandidate], k: usize) -> Vec<(Symbol, f64)> {
    if candidates.is_empty() || k == 0 {
        return Vec::new();
    }
    let daily_ret: Vec<f64> = candidates
        .iter()
        .map(|c| c.return_24h_pct.clamp(-50.0, 50.0))
        .collect();
    let short_ret: Vec<f64> = candidates.iter().map(|c| c.return_4h_pct).collect();
    let vol_ratio: Vec<f64> = candidates.iter().map(|c| c.vol_ratio_1h_24h).collect();

    let z24 = z_scores(&daily_ret);
    let z4 = z_scores(&short_ret);
    let zv = z_scores(&vol_ratio);

    let mut scored: Vec<(Symbol, f64)> = candidates
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let s = z24.get(i).copied().unwrap_or(0.0)
                + z4.get(i).copied().unwrap_or(0.0)
                + zv.get(i).copied().unwrap_or(0.0);
            (c.symbol.clone(), s)
        })
        .collect();

    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    scored.truncate(k);
    scored
}

#[allow(clippy::cast_precision_loss)]
fn z_scores(xs: &[f64]) -> Vec<f64> {
    if xs.is_empty() {
        return Vec::new();
    }
    let n = xs.len() as f64;
    let mean = xs.iter().sum::<f64>() / n;
    let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let std = var.sqrt();
    if std <= 1e-12 {
        return vec![0.0; xs.len()];
    }
    xs.iter().map(|x| (x - mean) / std).collect()
}

/// S3 — Gainers Momentum Scanner (strictly shadow-only in M08, §6.4).
#[derive(Debug, Clone)]
pub struct S3GainersShadow {
    config: GainersMomentumConfig,
    max_spread_bps: Decimal,
}

impl S3GainersShadow {
    /// Construct an [`S3GainersShadow`] evaluator.
    #[must_use]
    pub const fn new(config: GainersMomentumConfig, max_spread_bps: Decimal) -> Self {
        Self {
            config,
            max_spread_bps,
        }
    }

    /// Evaluate a closed 15m bar for a top-K gainer candidate and return a [`ShadowSignal`] (never an order).
    #[must_use]
    pub fn evaluate_shadow(
        &self,
        ctx: &StrategyCtx,
        return_24h_pct: Decimal,
        composite_z_score: f64,
    ) -> Option<ShadowSignal> {
        if !self.config.enabled || !ctx.bar.is_closed {
            return None;
        }
        let mut logs = Vec::new();

        // Return 24h strictly between [+min_gain_24h_pct, +max_gain_24h_pct] (default +5%..+25%)
        if return_24h_pct < self.config.min_gain_24h_pct
            || return_24h_pct > self.config.max_gain_24h_pct
        {
            return None;
        }
        logs.push(GateLog::pass(
            "s3_gain_band",
            format!("ret24h {return_24h_pct}% in band"),
        ));

        if ctx.spread_bps > self.max_spread_bps || !ctx.btc_long_regime_ok {
            return None;
        }
        logs.push(GateLog::pass("s3_spread_btc", "spread & BTC regime OK"));

        let (Some(vwap), Some(ema21), Some(rsi), Some(atr)) = (
            ctx.indicators.session_vwap,
            ctx.indicators.ema21,
            ctx.indicators.rsi14,
            ctx.indicators.atr14,
        ) else {
            return None;
        };

        let close = ctx.bar.close;
        if !(close > vwap && close > ema21 && (45.0..=65.0).contains(&rsi) && atr > 0.0) {
            return None;
        }

        let entry_d = f64_to_decimal(close)?;
        let stop_d = f64_to_decimal(close - 1.5 * atr)?;
        if stop_d <= Decimal::ZERO || stop_d >= entry_d {
            return None;
        }
        let r = entry_d - stop_d;
        let tp1_d = entry_d.saturating_add(r.saturating_mul(Decimal::new(15, 1)));

        #[allow(clippy::cast_possible_truncation)]
        let score_bps = (composite_z_score * 100.0).round() as i64;

        Some(ShadowSignal {
            strategy: StrategyId::GainersMomentum,
            venue: ctx.venue,
            symbol: ctx.symbol.clone(),
            direction: Direction::Long,
            ts_ms: ctx.bar.close_time_ms,
            entry_ref: Price(entry_d),
            stop: Price(stop_d),
            tp1: Price(tp1_d),
            score_bps: Some(score_bps),
            rationale: format!(
                "S3 Shadow Gainers: ret24h={return_24h_pct}%, z={composite_z_score:.2}, RSI={rsi:.1}"
            ),
            gate_logs: logs,
        })
    }
}

/// S4 — VWAP Reversion (strictly shadow-only, disabled by default, §6.4).
#[derive(Debug, Clone)]
pub struct S4VwapReversionShadow {
    config: VwapReversionConfig,
    adx_ceiling: f64,
}

impl S4VwapReversionShadow {
    /// Construct an [`S4VwapReversionShadow`] evaluator (`adx_ceiling = max(adx_min + 12, 38)`).
    #[must_use]
    pub fn new(config: VwapReversionConfig, adx_min: Decimal) -> Self {
        let ceiling = (adx_min.to_f64().unwrap_or(20.0) + 12.0).max(38.0);
        Self {
            config,
            adx_ceiling: ceiling,
        }
    }

    /// Evaluate a closed bar for S4 VWAP Reversion and return a [`ShadowSignal`] (never an order).
    #[must_use]
    pub fn evaluate_shadow(&self, ctx: &StrategyCtx) -> Option<ShadowSignal> {
        if !self.config.enabled || !ctx.bar.is_closed {
            return None;
        }
        let (Some(vwap), Some(atr), Some(adx), Some(rsi), Some(prev_rsi)) = (
            ctx.indicators.session_vwap,
            ctx.indicators.atr14,
            ctx.indicators.adx14,
            ctx.indicators.rsi14,
            ctx.indicators.prev_rsi14,
        ) else {
            return None;
        };

        if atr <= 0.0 || adx >= self.adx_ceiling {
            return None;
        }

        let close = ctx.bar.close;
        // Long reversion: price deviated >= 1.1*ATR below VWAP, extreme RSI <= 30 turning upward.
        if (vwap - close) < 1.1 * atr || rsi > 30.0 || rsi <= prev_rsi {
            return None;
        }

        let entry_d = f64_to_decimal(close)?;
        let stop_d = f64_to_decimal(close - 1.5 * atr)?;
        let tp1_d = f64_to_decimal(vwap)?;
        if stop_d <= Decimal::ZERO || stop_d >= entry_d || tp1_d <= entry_d {
            return None;
        }

        Some(ShadowSignal {
            strategy: StrategyId::VwapReversion,
            venue: ctx.venue,
            symbol: ctx.symbol.clone(),
            direction: Direction::Long,
            ts_ms: ctx.bar.close_time_ms,
            entry_ref: Price(entry_d),
            stop: Price(stop_d),
            tp1: Price(tp1_d),
            score_bps: None,
            rationale: format!(
                "S4 Shadow VWAP Reversion: ADX={adx:.1} < {:.1}, RSI={rsi:.1} turning up from {prev_rsi:.1}",
                self.adx_ceiling
            ),
            gate_logs: vec![GateLog::pass(
                "s4_vwap_reversion",
                "VWAP deviation >= 1.1 ATR with turning extreme RSI",
            )],
        })
    }
}
