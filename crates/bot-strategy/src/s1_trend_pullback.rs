//! S1 — Trend-Pullback strategy (Long, §6.4).

use bot_config::TrendPullbackConfig;
use bot_types::{Direction, Price, StrategyId, VenueCaps};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::gates::{DEFAULT_EXIT_COOLDOWN_BARS, evaluate_quality_gates};
use crate::types::{
    GateLog, SignalDecision, Strategy, StrategyCtx, StrategyEntryIntent, f64_to_decimal,
};

/// S1 Trend-Pullback strategy (Long profile for Spot/M08, §6.4).
#[derive(Debug, Clone)]
pub struct S1TrendPullback {
    config: TrendPullbackConfig,
    max_spread_bps: Decimal,
    require_obi_filter: bool,
    obi_min: f64,
    cooldown_bars: u32,
    allow_short: bool,
}

impl S1TrendPullback {
    /// Construct an [`S1TrendPullback`] strategy from validated config.
    #[must_use]
    pub const fn new(config: TrendPullbackConfig, max_spread_bps: Decimal) -> Self {
        Self {
            config,
            max_spread_bps,
            require_obi_filter: false,
            obi_min: 0.05,
            cooldown_bars: DEFAULT_EXIT_COOLDOWN_BARS,
            allow_short: false,
        }
    }

    /// Enable or disable Short side evaluation on venues that support shorting (§6.4.1).
    #[must_use]
    pub const fn with_allow_short(mut self, allow_short: bool) -> Self {
        self.allow_short = allow_short;
        self
    }

    /// Enable or disable the optional soft `OBI_smooth >= +0.05` filter (§6.4).
    #[must_use]
    pub const fn with_obi_filter(mut self, enabled: bool) -> Self {
        self.require_obi_filter = enabled;
        self
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate_short(&self, ctx: &StrategyCtx, mut logs: Vec<GateLog>) -> SignalDecision {
        // 1. HTF 4h closed regime: close < EMA50 and EMA50 falling for 3 bars.
        let htf_bearish =
            ctx.htf.ready && ctx.htf.close < ctx.htf.ema50 && ctx.htf.ema50_falling_3bars;
        if !htf_bearish {
            logs.push(GateLog::fail(
                "htf_regime",
                "HTF 4h regime not bearish or not warmed up",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }
        logs.push(GateLog::pass(
            "htf_regime",
            format!(
                "4h close {:.4} < EMA50 {:.4} (falling 3 bars)",
                ctx.htf.close, ctx.htf.ema50
            ),
        ));

        // Unpack required warm-up indicators.
        let (
            Some(ema9),
            Some(ema21),
            Some(ema55),
            Some(ema200),
            Some(rsi),
            Some(prev_rsi),
            Some(atr),
            Some(adx),
            Some(plus_di),
            Some(minus_di),
            Some(vol_sma20),
            Some(max_high_3),
            Some(max_high_5),
        ) = (
            ctx.indicators.ema9,
            ctx.indicators.ema21,
            ctx.indicators.ema55,
            ctx.indicators.ema200,
            ctx.indicators.rsi14,
            ctx.indicators.prev_rsi14,
            ctx.indicators.atr14,
            ctx.indicators.adx14,
            ctx.indicators.plus_di,
            ctx.indicators.minus_di,
            ctx.indicators.vol_sma20,
            ctx.indicators.max_high_3,
            ctx.indicators.max_high_5,
        )
        else {
            logs.push(GateLog::fail("warmup", "S1 Short indicators warming up"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        };

        if atr <= 0.0 || vol_sma20 <= 0.0 {
            logs.push(GateLog::fail("indicators_positive", "ATR or VolSMA20 <= 0"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let close = ctx.bar.close;

        // 2. 1h trend: EMA21 < EMA55 and close < EMA200
        if ema21 < ema55 && close < ema200 {
            logs.push(GateLog::pass(
                "trend_1h",
                format!("EMA21 {ema21:.4} < EMA55 {ema55:.4} & close < EMA200 {ema200:.4}"),
            ));
        } else {
            logs.push(GateLog::fail(
                "trend_1h",
                "1h bearish trend alignment failed (EMA21 >= EMA55 or close >= EMA200)",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 3. Pullback mirror: in last 3 bars high >= EMA21 - 0.1*ATR, close < EMA21, close >= EMA9 - 0.5*ATR
        let touched_pullback = max_high_3 >= ema21 - 0.1 * atr;
        let rejected_ema21 = close < ema21;
        let not_extended_ema9 = close >= ema9 - 0.5 * atr;
        if touched_pullback && rejected_ema21 && not_extended_ema9 {
            logs.push(GateLog::pass(
                "pullback",
                format!("max_high_3={max_high_3:.4}, close={close:.4} rejected EMA21"),
            ));
        } else {
            logs.push(GateLog::fail(
                "pullback",
                format!(
                    "touched={touched_pullback}, rejected={rejected_ema21}, not_ext_ema9={not_extended_ema9}"
                ),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 4. Momentum mirror: RSI in [100 - rsi_max, 100 - rsi_min] & RSI < prev_RSI; ADX >= adx_min & -DI > +DI
        let long_rsi_min = self.config.rsi_min.to_f64().unwrap_or(40.0);
        let long_rsi_max = self.config.rsi_max.to_f64().unwrap_or(62.0);
        let rsi_min = (100.0 - long_rsi_max).max(0.0);
        let rsi_max = (100.0 - long_rsi_min).min(100.0);
        let adx_min = self.config.adx_min.to_f64().unwrap_or(20.0);
        let momentum_ok = rsi >= rsi_min
            && rsi <= rsi_max
            && rsi < prev_rsi
            && adx >= adx_min
            && minus_di > plus_di;
        if momentum_ok {
            logs.push(GateLog::pass(
                "momentum",
                format!("RSI {rsi:.2} < {prev_rsi:.2}, ADX {adx:.2}, -DI > +DI"),
            ));
        } else {
            logs.push(GateLog::fail(
                "momentum",
                format!(
                    "RSI={rsi:.2} (prev {prev_rsi:.2}), ADX={adx:.2}, +DI={plus_di:.2}, -DI={minus_di:.2}"
                ),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 5. Volume: vol >= 0.8 * VolSMA20
        if ctx.bar.volume >= 0.8 * vol_sma20 {
            logs.push(GateLog::pass(
                "volume",
                format!("vol {:.2} >= 0.8x SMA20 ({vol_sma20:.2})", ctx.bar.volume),
            ));
        } else {
            logs.push(GateLog::fail(
                "volume",
                format!("vol {:.2} < 0.8x SMA20 ({vol_sma20:.2})", ctx.bar.volume),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 6. Anti-overextension: |close - EMA21| <= 1.5 * ATR
        if (close - ema21).abs() <= 1.5 * atr {
            logs.push(GateLog::pass(
                "anti_overextension",
                "|close - EMA21| <= 1.5*ATR",
            ));
        } else {
            logs.push(GateLog::fail(
                "anti_overextension",
                "|close - EMA21| > 1.5*ATR",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 7. Optional OBI soft filter mirror: OBI_smooth <= -obi_min
        if self.require_obi_filter {
            if ctx.obi_smooth.is_some_and(|v| v <= -self.obi_min) {
                logs.push(GateLog::pass(
                    "obi_soft_filter",
                    format!("OBI_smooth <= -{}", self.obi_min),
                ));
            } else {
                logs.push(GateLog::fail(
                    "obi_soft_filter",
                    format!("OBI_smooth {:?} > -{}", ctx.obi_smooth, self.obi_min),
                ));
                return SignalDecision {
                    intent: None,
                    gate_logs: logs,
                };
            }
        }

        // Compute stop & targets in exact Decimal (§6.4.1):
        // stop = max(max_high_5, entry + 1.5 * ATR) + 0.1 * ATR; TP1 = entry - tp1_r*R, TP2 = entry - tp2_r*R.
        let stop_f64 = max_high_5.max(close + 1.5 * atr) + 0.1 * atr;
        let (Some(entry_d), Some(stop_d), Some(atr_d)) = (
            f64_to_decimal(close),
            f64_to_decimal(stop_f64),
            f64_to_decimal(atr),
        ) else {
            logs.push(GateLog::fail(
                "decimal_conv",
                "failed to convert Short prices",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        };

        if entry_d <= Decimal::ZERO || stop_d <= entry_d {
            logs.push(GateLog::fail(
                "stop_geometry",
                "invalid Short stop <= entry or entry <= 0",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let r_dist = stop_d - entry_d;
        let tp1_d = entry_d.saturating_sub(r_dist.saturating_mul(self.config.tp1_r));
        let tp2_d = entry_d.saturating_sub(r_dist.saturating_mul(self.config.tp2_r));

        if tp1_d <= Decimal::ZERO || tp2_d <= Decimal::ZERO {
            logs.push(GateLog::fail(
                "target_geometry",
                format!("non-positive Short target: tp1={tp1_d}, tp2={tp2_d}"),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let entry_ref = Price(entry_d);
        let tp1 = Price(tp1_d);

        // §6.5 Shared Quality Gates (cost gate, spread, freshness, BTC regime, cooldown, vol targeting)
        let (quality_ok, size_mult, mut q_logs) = evaluate_quality_gates(
            ctx,
            entry_ref,
            tp1,
            Direction::Short,
            self.config.cost_multiple,
            self.max_spread_bps,
            self.cooldown_bars,
        );
        logs.append(&mut q_logs);

        if !quality_ok {
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let intent = StrategyEntryIntent {
            venue: ctx.venue,
            symbol: ctx.symbol.clone(),
            direction: Direction::Short,
            entry_ref,
            stop: Price(stop_d),
            tp1,
            tp2: Some(Price(tp2_d)),
            atr: atr_d,
            size_mult,
            strategy: StrategyId::TrendPullback,
            rationale: format!(
                "S1 Trend-Pullback Short: 4h bearish, 1h EMA21<EMA55, RSI {rsi:.1} falling, ADX {adx:.1}"
            ),
            gate_logs: logs.clone(),
        };

        SignalDecision {
            intent: Some(intent),
            gate_logs: logs,
        }
    }
}

impl Strategy for S1TrendPullback {
    fn id(&self) -> StrategyId {
        StrategyId::TrendPullback
    }

    fn supports(&self, caps: &VenueCaps, dir: Direction) -> bool {
        match dir {
            Direction::Long => true,
            Direction::Short => caps.can_short && self.allow_short,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&mut self, ctx: &StrategyCtx) -> SignalDecision {
        let mut logs = Vec::with_capacity(14);

        if !self.config.enabled {
            logs.push(GateLog::fail("enabled", "S1 trend_pullback is disabled"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // If HTF is bearish and venue/config supports Short, evaluate Short mirror (§6.4.1).
        if ctx.htf.ready
            && ctx.htf.close < ctx.htf.ema50
            && ctx.htf.ema50_falling_3bars
            && self.supports(&ctx.venue_caps, Direction::Short)
        {
            return self.evaluate_short(ctx, logs);
        }

        if !self.supports(&ctx.venue_caps, Direction::Long) {
            logs.push(GateLog::fail("venue_caps", "Long unsupported on venue"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 1. HTF 4h closed regime: close > EMA50 and EMA50 rising for 3 bars.
        let htf_ok = ctx.htf.ready && ctx.htf.close > ctx.htf.ema50 && ctx.htf.ema50_rising_3bars;
        if htf_ok {
            logs.push(GateLog::pass(
                "htf_regime",
                format!(
                    "4h close {:.4} > EMA50 {:.4} (rising 3 bars)",
                    ctx.htf.close, ctx.htf.ema50
                ),
            ));
        } else {
            logs.push(GateLog::fail(
                "htf_regime",
                "HTF 4h regime not bullish or not warmed up",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // Unpack required warm-up indicators.
        let (
            Some(ema9),
            Some(ema21),
            Some(ema55),
            Some(ema200),
            Some(rsi),
            Some(prev_rsi),
            Some(atr),
            Some(adx),
            Some(plus_di),
            Some(minus_di),
            Some(vol_sma20),
            Some(min_low_3),
            Some(min_low_5),
        ) = (
            ctx.indicators.ema9,
            ctx.indicators.ema21,
            ctx.indicators.ema55,
            ctx.indicators.ema200,
            ctx.indicators.rsi14,
            ctx.indicators.prev_rsi14,
            ctx.indicators.atr14,
            ctx.indicators.adx14,
            ctx.indicators.plus_di,
            ctx.indicators.minus_di,
            ctx.indicators.vol_sma20,
            ctx.indicators.min_low_3,
            ctx.indicators.min_low_5,
        )
        else {
            logs.push(GateLog::fail("warmup", "1h indicators still warming up"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        };

        if atr <= 0.0 || vol_sma20 <= 0.0 {
            logs.push(GateLog::fail("indicators_positive", "ATR or VolSMA20 <= 0"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let close = ctx.bar.close;

        // 2. 1h trend: EMA21 > EMA55 and close > EMA200
        if ema21 > ema55 && close > ema200 {
            logs.push(GateLog::pass(
                "trend_1h",
                format!("EMA21 {ema21:.4} > EMA55 {ema55:.4} & close > EMA200 {ema200:.4}"),
            ));
        } else {
            logs.push(GateLog::fail(
                "trend_1h",
                "1h trend alignment failed (EMA21 <= EMA55 or close <= EMA200)",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 3. Pullback: in last 3 bars low <= EMA21 + 0.1*ATR, close > EMA21, close <= EMA9 + 0.5*ATR
        let touched_pullback = min_low_3 <= ema21 + 0.1 * atr;
        let reclaimed_ema21 = close > ema21;
        let not_extended_ema9 = close <= ema9 + 0.5 * atr;
        if touched_pullback && reclaimed_ema21 && not_extended_ema9 {
            logs.push(GateLog::pass(
                "pullback",
                format!("min_low_3={min_low_3:.4}, close={close:.4} reclaimed EMA21"),
            ));
        } else {
            logs.push(GateLog::fail(
                "pullback",
                format!(
                    "touched={touched_pullback}, reclaimed={reclaimed_ema21}, not_ext_ema9={not_extended_ema9}"
                ),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 4. Momentum: RSI in [rsi_min, rsi_max] & RSI > prev_RSI; ADX >= adx_min & +DI > -DI
        let rsi_min = self.config.rsi_min.to_f64().unwrap_or(40.0);
        let rsi_max = self.config.rsi_max.to_f64().unwrap_or(62.0);
        let adx_min = self.config.adx_min.to_f64().unwrap_or(20.0);
        let momentum_ok = rsi >= rsi_min
            && rsi <= rsi_max
            && rsi > prev_rsi
            && adx >= adx_min
            && plus_di > minus_di;
        if momentum_ok {
            logs.push(GateLog::pass(
                "momentum",
                format!("RSI {rsi:.2} > {prev_rsi:.2}, ADX {adx:.2}, +DI > -DI"),
            ));
        } else {
            logs.push(GateLog::fail(
                "momentum",
                format!(
                    "RSI={rsi:.2} (prev {prev_rsi:.2}), ADX={adx:.2}, +DI={plus_di:.2}, -DI={minus_di:.2}"
                ),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 5. Volume: vol >= 0.8 * VolSMA20
        if ctx.bar.volume >= 0.8 * vol_sma20 {
            logs.push(GateLog::pass(
                "volume",
                format!("vol {:.2} >= 0.8x SMA20 ({vol_sma20:.2})", ctx.bar.volume),
            ));
        } else {
            logs.push(GateLog::fail(
                "volume",
                format!("vol {:.2} < 0.8x SMA20 ({vol_sma20:.2})", ctx.bar.volume),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 6. Anti-overextension: |close - EMA21| <= 1.5 * ATR
        if (close - ema21).abs() <= 1.5 * atr {
            logs.push(GateLog::pass(
                "anti_overextension",
                "|close - EMA21| <= 1.5*ATR",
            ));
        } else {
            logs.push(GateLog::fail(
                "anti_overextension",
                "|close - EMA21| > 1.5*ATR",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 7. Optional OBI soft filter: OBI_smooth >= +0.05
        if self.require_obi_filter {
            if ctx.obi_smooth.is_some_and(|v| v >= self.obi_min) {
                logs.push(GateLog::pass("obi_soft_filter", "OBI_smooth >= +0.05"));
            } else {
                logs.push(GateLog::fail(
                    "obi_soft_filter",
                    format!("OBI_smooth {:?} < {}", ctx.obi_smooth, self.obi_min),
                ));
                return SignalDecision {
                    intent: None,
                    gate_logs: logs,
                };
            }
        }

        // Compute stop & targets in exact Decimal (§6.4):
        // stop = min(min_low_5, entry - 1.5 * ATR) - 0.1 * ATR; TP1 = 1.5R, TP2 = 3R.
        let stop_f64 = min_low_5.min(close - 1.5 * atr) - 0.1 * atr;
        let (Some(entry_d), Some(stop_d), Some(atr_d)) = (
            f64_to_decimal(close),
            f64_to_decimal(stop_f64),
            f64_to_decimal(atr),
        ) else {
            logs.push(GateLog::fail("decimal_conv", "non-finite price/ATR"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        };

        if stop_d <= Decimal::ZERO || stop_d >= entry_d {
            logs.push(GateLog::fail(
                "stop_geometry",
                "invalid stop <= 0 or >= entry",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let r_dist = entry_d - stop_d;
        let tp1_d = entry_d.saturating_add(r_dist.saturating_mul(self.config.tp1_r));
        let tp2_d = entry_d.saturating_add(r_dist.saturating_mul(self.config.tp2_r));

        let entry_ref = Price(entry_d);
        let tp1 = Price(tp1_d);

        // §6.5 Shared Quality Gates (cost gate, spread, freshness, BTC regime, cooldown, vol targeting)
        let (quality_ok, size_mult, mut q_logs) = evaluate_quality_gates(
            ctx,
            entry_ref,
            tp1,
            Direction::Long,
            self.config.cost_multiple,
            self.max_spread_bps,
            self.cooldown_bars,
        );
        logs.append(&mut q_logs);

        if !quality_ok {
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let intent = StrategyEntryIntent {
            venue: ctx.venue,
            symbol: ctx.symbol.clone(),
            direction: Direction::Long,
            entry_ref,
            stop: Price(stop_d),
            tp1,
            tp2: Some(Price(tp2_d)),
            atr: atr_d,
            size_mult,
            strategy: StrategyId::TrendPullback,
            rationale: format!(
                "S1 Trend-Pullback Long: 4h bullish, 1h EMA21>EMA55, RSI {rsi:.1} rising, ADX {adx:.1}"
            ),
            gate_logs: logs.clone(),
        };

        SignalDecision {
            intent: Some(intent),
            gate_logs: logs,
        }
    }
}
