//! S2 — Donchian Breakout strategy (Long, §6.4).

use bot_config::BreakoutConfig;
use bot_types::{Direction, Price, StrategyId, VenueCaps};
use rust_decimal::Decimal;
use rust_decimal::prelude::ToPrimitive;

use crate::gates::{DEFAULT_EXIT_COOLDOWN_BARS, evaluate_quality_gates};
use crate::types::{
    GateLog, SignalDecision, Strategy, StrategyCtx, StrategyEntryIntent, f64_to_decimal,
};

/// Minimum ADX for S2 Breakout (§6.4: ADX >= 18 and rising).
pub const S2_MIN_ADX: f64 = 18.0;
/// S2 RSI lower bound (§6.4: RSI in [52, 75]).
pub const S2_RSI_MIN: f64 = 52.0;
/// S2 RSI upper bound (§6.4: RSI in [52, 75]).
pub const S2_RSI_MAX: f64 = 75.0;

/// S2 Donchian Breakout strategy (Long profile for Spot/M08, §6.4).
#[derive(Debug, Clone)]
pub struct S2Breakout {
    config: BreakoutConfig,
    max_spread_bps: Decimal,
    cost_multiple: Decimal,
    tp1_r: Decimal,
    tp2_r: Decimal,
    cooldown_bars: u32,
    allow_short: bool,
}

impl S2Breakout {
    /// Construct an [`S2Breakout`] strategy from validated config.
    #[must_use]
    pub const fn new(
        config: BreakoutConfig,
        max_spread_bps: Decimal,
        cost_multiple: Decimal,
    ) -> Self {
        Self {
            config,
            max_spread_bps,
            cost_multiple,
            tp1_r: Decimal::from_parts(15, 0, 0, false, 1), // 1.5R
            tp2_r: Decimal::from_parts(3, 0, 0, false, 0),  // 3.0R
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

    fn evaluate_short(ctx: &StrategyCtx, mut logs: Vec<GateLog>) -> SignalDecision {
        let (Some(dc_lower), Some(dc_mid), Some(rsi), Some(atr), Some(vol_sma20)) = (
            ctx.indicators.donchian_lower,
            ctx.indicators.donchian_mid,
            ctx.indicators.rsi14,
            ctx.indicators.atr14,
            ctx.indicators.vol_sma20,
        ) else {
            logs.push(GateLog::fail("warmup", "S2 Short indicators warming up"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        };

        let close = ctx.bar.close;
        let vol = ctx.bar.volume;
        if atr <= 0.0
            || vol_sma20 <= 0.0
            || close >= dc_lower
            || vol < 1.5 * vol_sma20
            || !(25.0..=48.0).contains(&rsi)
        {
            logs.push(GateLog::fail(
                "s2_short_rules",
                "S2 Short breakdown conditions not met",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let stop_f = dc_mid.max(close + 1.5 * atr);
        let r_dist = stop_f - close;
        if r_dist <= 0.0 {
            logs.push(GateLog::fail(
                "stop_distance",
                "non-positive Short R distance",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }
        let tp1_f = close - 1.5 * r_dist;
        let tp2_f = close - 3.0 * r_dist;

        let (Some(entry_dec), Some(stop_dec), Some(tp1_dec), Some(tp2_dec), Some(atr_dec)) = (
            f64_to_decimal(close),
            f64_to_decimal(stop_f),
            f64_to_decimal(tp1_f),
            f64_to_decimal(tp2_f),
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

        logs.push(GateLog::pass("s2_short_rules", "S2 Short breakdown passed"));
        SignalDecision {
            intent: Some(StrategyEntryIntent {
                venue: ctx.venue,
                symbol: ctx.symbol.clone(),
                direction: Direction::Short,
                entry_ref: Price(entry_dec),
                stop: Price(stop_dec),
                tp1: Price(tp1_dec),
                tp2: Some(Price(tp2_dec)),
                atr: atr_dec,
                size_mult: Decimal::ONE,
                strategy: StrategyId::Breakout,
                rationale: "S2 Short Donchian(20) lower breakdown".to_owned(),
                gate_logs: logs.clone(),
            }),
            gate_logs: logs,
        }
    }
}

impl Strategy for S2Breakout {
    fn id(&self) -> StrategyId {
        StrategyId::Breakout
    }

    fn supports(&self, caps: &VenueCaps, dir: Direction) -> bool {
        match dir {
            Direction::Long => true,
            Direction::Short => caps.can_short && self.allow_short,
        }
    }

    #[allow(clippy::too_many_lines)]
    fn evaluate(&mut self, ctx: &StrategyCtx) -> SignalDecision {
        let mut logs = Vec::with_capacity(12);

        if !self.config.enabled {
            logs.push(GateLog::fail("enabled", "S2 breakout is disabled"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        if ctx.htf.ready
            && ctx.htf.close < ctx.htf.ema50
            && ctx.htf.ema50_falling_3bars
            && self.supports(&ctx.venue_caps, Direction::Short)
        {
            return Self::evaluate_short(ctx, logs);
        }

        if !self.supports(&ctx.venue_caps, Direction::Long) {
            logs.push(GateLog::fail("venue_caps", "Long unsupported on venue"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 1. HTF aligned bullish
        let htf_ok = ctx.htf.ready && ctx.htf.close > ctx.htf.ema50 && ctx.htf.ema50_rising_3bars;
        if htf_ok {
            logs.push(GateLog::pass("htf_regime", "HTF 4h bullish and rising"));
        } else {
            logs.push(GateLog::fail("htf_regime", "HTF 4h not bullish"));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        let (
            Some(dc_upper),
            Some(dc_mid),
            Some(ema21),
            Some(rsi),
            Some(atr),
            Some(adx),
            Some(prev_adx),
            Some(vol_sma20),
        ) = (
            ctx.indicators.donchian_upper,
            ctx.indicators.donchian_mid,
            ctx.indicators.ema21,
            ctx.indicators.rsi14,
            ctx.indicators.atr14,
            ctx.indicators.adx14,
            ctx.indicators.prev_adx14,
            ctx.indicators.vol_sma20,
        )
        else {
            logs.push(GateLog::fail("warmup", "S2 indicators still warming up"));
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

        // 2. Breakout: close > Donchian(20).upper (strictly previous 20 bars)
        if close > dc_upper {
            logs.push(GateLog::pass(
                "donchian_breakout",
                format!("close {close:.4} > Donchian.upper {dc_upper:.4}"),
            ));
        } else {
            logs.push(GateLog::fail(
                "donchian_breakout",
                format!("close {close:.4} <= Donchian.upper {dc_upper:.4}"),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 3. Volume: vol >= min_volume_mult * VolSMA20 (default 1.5x)
        let vol_mult = self.config.min_volume_mult.to_f64().unwrap_or(1.5);
        if ctx.bar.volume >= vol_mult * vol_sma20 {
            logs.push(GateLog::pass(
                "volume",
                format!("vol {:.2} >= {vol_mult:.2}x SMA20", ctx.bar.volume),
            ));
        } else {
            logs.push(GateLog::fail(
                "volume",
                format!("vol {:.2} < {vol_mult:.2}x SMA20", ctx.bar.volume),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 4. ADX >= 18 and rising; RSI in [52, 75]
        if adx >= S2_MIN_ADX && adx > prev_adx && (S2_RSI_MIN..=S2_RSI_MAX).contains(&rsi) {
            logs.push(GateLog::pass(
                "momentum",
                format!("ADX {adx:.2} > {prev_adx:.2}, RSI {rsi:.2} in [52,75]"),
            ));
        } else {
            logs.push(GateLog::fail(
                "momentum",
                format!("ADX={adx:.2} (prev {prev_adx:.2}), RSI={rsi:.2}"),
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 5. Anti-overextension: close - EMA21 <= 2.5 * ATR
        if (close - ema21) <= 2.5 * atr {
            logs.push(GateLog::pass(
                "anti_overextension",
                "close - EMA21 <= 2.5*ATR",
            ));
        } else {
            logs.push(GateLog::fail(
                "anti_overextension",
                "close - EMA21 > 2.5*ATR",
            ));
            return SignalDecision {
                intent: None,
                gate_logs: logs,
            };
        }

        // 6. Stop: closer of Donchian mid or entry - 2*ATR, but at least 1*ATR away (§6.4).
        let atr2_stop = close - 2.0 * atr;
        let max_stop = close - atr;
        let stop_f64 = dc_mid.max(atr2_stop).min(max_stop);

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
        let tp1 = Price(entry_d.saturating_add(r_dist.saturating_mul(self.tp1_r)));
        let tp2 = Price(entry_d.saturating_add(r_dist.saturating_mul(self.tp2_r)));
        let entry_ref = Price(entry_d);

        let (quality_ok, size_mult, mut q_logs) = evaluate_quality_gates(
            ctx,
            entry_ref,
            tp1,
            Direction::Long,
            self.cost_multiple,
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
            tp2: Some(tp2),
            atr: atr_d,
            size_mult,
            strategy: StrategyId::Breakout,
            rationale: format!(
                "S2 Breakout Long: close {close:.4} > Donchian {dc_upper:.4}, ADX {adx:.1} rising, RSI {rsi:.1}"
            ),
            gate_logs: logs.clone(),
        };

        SignalDecision {
            intent: Some(intent),
            gate_logs: logs,
        }
    }
}
