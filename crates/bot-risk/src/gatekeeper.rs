//! Pre-trade Gatekeeper (§7.2), operational health hooks, and runtime tighten-only updates.

use bot_config::{BotConfig, ConfigError, LoosenError, RiskConfig, is_tightening};
use bot_types::{
    Direction, EngineMode, InstrumentRules, Position, PositionState, Price, Qty, StrategyId,
    Symbol, Venue,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::kelly::{KellyDecision, KellyInput, evaluate_kelly};
use crate::mode::ModeTrigger;
use crate::sizing::{SizingError, SpotSizingInput, size_spot_position};

/// Maximum market data age allowed for pre-trade entry (§7.2 #4: 1500 ms).
pub const MAX_MARKET_DATA_AGE_MS: i64 = 1_500;
/// Stale data threshold triggering `ReduceOnly` (§7.4: 30 s).
pub const STALE_DATA_REDUCE_ONLY_MS: i64 = 30_000;
/// Maximum clock drift allowed for pre-trade entry (§7.2 #4: 800 ms).
pub const MAX_CLOCK_DRIFT_MS: i64 = 800;
/// Minimum stop distance percentage (`0.3%` per §7.2 #6).
pub const DEFAULT_MIN_STOP_PCT: Decimal = Decimal::from_parts(3, 0, 0, false, 1);
/// Maximum stop distance percentage (`8.0%` per §7.2 #6).
pub const DEFAULT_MAX_STOP_PCT: Decimal = Decimal::from_parts(80, 0, 0, false, 1);
/// Rate-limit guard ceiling for `REQUEST_WEIGHT` (`60%` per §7.2 #9).
pub const MAX_REQUEST_WEIGHT_PCT: Decimal = Decimal::from_parts(60, 0, 0, false, 0);
/// Rate-limit guard ceiling for `ORDERS` 10 s (`50%` per §7.2 #9).
pub const MAX_ORDERS_10S_PCT: Decimal = Decimal::from_parts(50, 0, 0, false, 0);

/// Strategy-proposed entry intent evaluated by [`Gatekeeper`] (§6.4, §7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryIntent {
    /// Target venue.
    pub venue: Venue,
    /// Target symbol.
    pub symbol: Symbol,
    /// Trade direction (Spot supports Long only).
    pub direction: Direction,
    /// Originating strategy.
    pub strategy: StrategyId,
    /// Reference entry price.
    pub entry_price: Price,
    /// Actual protective stop price (including buffer).
    pub stop_price: Price,
    /// First take-profit price.
    pub tp1_price: Price,
    /// Volatility targeting size multiplier (§6.5, `[0.5, 1.25]`).
    #[serde(with = "rust_decimal::serde::str")]
    pub size_mult: Decimal,
    /// Strategy scanner multiplier (`1.0` core, `0.5` scanner S3).
    #[serde(with = "rust_decimal::serde::str")]
    pub scanner_mult: Decimal,
    /// Whether this intent originates from the S3 gainers scanner (max 2 scanner positions).
    pub is_scanner: bool,
}

/// Snapshot of runtime context required by [`Gatekeeper::evaluate`].
#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone)]
pub struct GatekeeperContext<'a> {
    /// Current UTC timestamp (ms).
    pub now_ms: i64,
    /// Global engine mode.
    pub mode: EngineMode,
    /// Exchange symbol filter rules.
    pub rules: &'a InstrumentRules,
    /// Whether symbol is currently in the active filtered universe.
    pub symbol_in_universe: bool,
    /// Whether symbol is quarantined due to anomalies.
    pub symbol_quarantined: bool,
    /// Per-`(venue, symbol)` post-exit cooldown deadline (UTC ms), if any.
    pub symbol_cooldown_until_ms: Option<i64>,
    /// Whether BTC Crash Guard is active (§7.5).
    pub btc_guard_active: bool,
    /// Whether a macro event blackout window is active (§6.5 #7).
    pub macro_blackout_active: bool,
    /// Age of latest market data tick (ms).
    pub market_data_age_ms: i64,
    /// Whether the user data stream is connected and healthy.
    pub user_stream_healthy: bool,
    /// Absolute local-vs-server clock drift (ms).
    pub clock_drift_ms: i64,
    /// Currently open positions across ALL venues (for global heat & correlated direction cap).
    pub open_positions: &'a [Position],
    /// Combined global equity (USD).
    pub equity_total_usd: Decimal,
    /// Free quote balance available on the target venue (USD/USDT).
    pub free_quote_usd: Decimal,
    /// Current `REQUEST_WEIGHT` utilization percentage (`0..=100`).
    pub request_weight_pct: Decimal,
    /// Current `ORDERS` (10 s) utilization percentage (`0..=100`).
    pub orders_10s_pct: Decimal,
    /// Loss-streak cooldown deadline (UTC ms), if active.
    pub loss_streak_cooldown_until_ms: Option<i64>,
    /// Orders submitted so far today.
    pub orders_today: u32,
    /// Whether `ProfitLockMode::Stop` has stopped new entries today.
    pub profit_lock_stop: bool,
    /// Daily profit-lock risk multiplier (`1.0` normal, `0.5` reduce mode).
    pub profit_lock_mult: Decimal,
    /// Optional Kelly input for the originating strategy.
    pub kelly: Option<KellyInput>,
    /// Optional `PERCENT_PRICE_BY_SIDE` allowed price bounds `(min_price, max_price)`.
    pub percent_price_bounds: Option<(Price, Price)>,
    /// Optional fixed notional USD when `SizingMode::FixedNotional` is active.
    pub fixed_notional_usd: Option<Decimal>,
}

/// Structured rejection reason recorded by [`Gatekeeper::evaluate`] (§7.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RejectReason {
    /// Step 1: Engine is not in `EngineMode::Ready`.
    EngineNotReady(EngineMode),
    /// Step 1b: Daily profit-lock `stop` mode active or loss-streak cooldown or order cap reached.
    DailyOrderLimitReached {
        /// Orders today.
        orders_today: u32,
        /// Configured max.
        max_orders_per_day: u32,
    },
    /// Loss-streak cooldown active.
    LossStreakCooldownActive {
        /// Cooldown end timestamp (UTC ms).
        until_ms: i64,
    },
    /// Profit-lock stop mode active.
    ProfitLockStopped,
    /// Step 2: Symbol is not `TRADING`, quarantined, not in universe, or on cooldown.
    SymbolNotTrading,
    /// Symbol quarantined.
    SymbolQuarantined,
    /// Symbol outside active universe.
    SymbolNotInUniverse,
    /// Per-symbol cooldown active.
    SymbolCooldownActive {
        /// Cooldown end timestamp (UTC ms).
        until_ms: i64,
    },
    /// Spot does not support Short direction.
    SpotShortUnsupported,
    /// Step 3: BTC Crash Guard or macro blackout active.
    BtcCrashGuardActive,
    /// Macro event blackout active.
    MacroBlackoutActive,
    /// Step 4: Stale market data, unhealthy user stream, or excessive clock drift.
    MarketDataStale {
        /// Observed age (ms).
        age_ms: i64,
    },
    /// User data stream unhealthy.
    UserStreamUnhealthy,
    /// Clock drift exceeds 800 ms.
    ClockDriftTooHigh {
        /// Observed absolute drift (ms).
        drift_ms: i64,
    },
    /// Step 5: Position count, duplicate symbol, strategy cap, or correlated direction cap.
    DuplicateOpenSymbol,
    /// Max simultaneous open positions reached.
    MaxOpenPositionsReached {
        /// Open count.
        open: usize,
        /// Max allowed.
        max: u32,
    },
    /// Scanner strategy max 2 concurrent positions reached (§6.4 S3).
    MaxScannerPositionsReached,
    /// Cross-venue correlated same-direction position cap reached (§7.2).
    MaxSameDirectionCorrelated {
        /// Current count in same direction.
        count: usize,
        /// Configured ceiling.
        max: u32,
    },
    /// Step 6: Stop price on wrong side or distance outside `[min_stop_pct, max_stop_pct]`.
    InvalidStopDistance {
        /// Detail.
        detail: String,
    },
    /// Step 7a: Kelly fraction non-positive → strategy auto-disabled (§7.3).
    KellyAutoDisabled {
        /// Strategy that must be disabled.
        strategy: StrategyId,
    },
    /// Step 7b: Entry price violates `PERCENT_PRICE_BY_SIDE` filter bounds.
    PercentPriceViolation,
    /// Step 7c / 8: Sizing, filter floor, portfolio heat, exposure cap, or free quote check failed.
    SizingRejected(String),
    /// Step 9: Rate-limit guard (`REQUEST_WEIGHT >= 60%` or `ORDERS 10s >= 50%`).
    RateLimitGuard {
        /// Observed `REQUEST_WEIGHT` %.
        request_weight_pct: Decimal,
        /// Observed `ORDERS` 10 s %.
        orders_10s_pct: Decimal,
    },
}

/// Outcome of [`Gatekeeper::evaluate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GateDecision {
    /// Entry approved with exact exchange-valid quantity and risk metrics.
    Accept {
        /// Exchange-valid quantity floored to `step_size`.
        qty: Qty,
        /// Order notional in quote currency.
        notional_usd: Decimal,
        /// Actual risk in quote currency (`qty * |entry - stop|`).
        actual_risk_usd: Decimal,
        /// Projected combined portfolio heat `%` after this entry.
        heat_after_pct: Decimal,
    },
    /// Entry rejected with all recorded reasons (stopped at first failing stage per §7.2).
    Reject {
        /// Ordered rejection reasons.
        reasons: Vec<RejectReason>,
        /// `true` if Kelly `f <= 0` requires auto-disabling the originating strategy.
        auto_disable_strategy: bool,
    },
}

/// Error returned when a runtime risk config update is invalid or attempts to loosen parameters.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TightenOnlyError {
    /// New risk configuration violates strict schema/range bounds (`BotConfig::validate`).
    #[error("runtime risk config update rejected (invalid bounds): {0}")]
    Validation(#[from] ConfigError),
    /// New risk configuration loosens one or more parameters relative to the active config.
    #[error("runtime risk config update rejected (tighten-only violated): {0}")]
    Loosened(#[from] LoosenError),
}

/// Stateless-per-call pre-trade Gatekeeper holding tighten-only [`RiskConfig`] (§7.2).
#[derive(Debug, Clone)]
pub struct Gatekeeper {
    config: RiskConfig,
    min_stop_pct: Decimal,
    max_stop_pct: Decimal,
}

impl Gatekeeper {
    /// Construct a [`Gatekeeper`] from validated [`RiskConfig`].
    #[must_use]
    pub const fn new(config: RiskConfig) -> Self {
        Self {
            config,
            min_stop_pct: DEFAULT_MIN_STOP_PCT,
            max_stop_pct: DEFAULT_MAX_STOP_PCT,
        }
    }

    /// Access current [`RiskConfig`].
    #[must_use]
    pub const fn config(&self) -> &RiskConfig {
        &self.config
    }

    /// Update runtime [`RiskConfig`] enforcing valid bounds and strict **tighten-only** invariant (hard rule #8).
    ///
    /// # Errors
    /// Returns [`TightenOnlyError::Validation`] if `new_risk` violates config range bounds, or
    /// [`TightenOnlyError::Loosened`] if any parameter is loosened relative to the current config.
    pub fn update_config(&mut self, new_risk: RiskConfig) -> Result<(), TightenOnlyError> {
        let old_full = BotConfig {
            risk: self.config.clone(),
            ..BotConfig::default()
        };
        let new_full = BotConfig {
            risk: new_risk.clone(),
            ..BotConfig::default()
        };
        new_full.validate()?;
        is_tightening(&old_full, &new_full)?;
        self.config = new_risk;
        Ok(())
    }

    /// Operational health hook for stale data (> 30 s) or excessive clock drift (> 800 ms) (§7.4).
    #[must_use]
    pub const fn check_operational_health(
        market_data_age_ms: i64,
        clock_drift_ms: i64,
    ) -> Option<ModeTrigger> {
        if market_data_age_ms > STALE_DATA_REDUCE_ONLY_MS {
            return Some(ModeTrigger::DataStaleOver30s);
        }
        if clock_drift_ms.abs() > MAX_CLOCK_DRIFT_MS {
            return Some(ModeTrigger::ClockDriftExceeded);
        }
        None
    }

    /// Execute the 9-stage §7.2 pre-trade check pipeline (stopping at the first failing stage).
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn evaluate(&self, intent: &EntryIntent, ctx: &GatekeeperContext<'_>) -> GateDecision {
        // Stage 1: Engine mode == Ready + daily order cap + loss-streak cooldown + profit-lock stop
        if ctx.mode != EngineMode::Ready {
            return reject(RejectReason::EngineNotReady(ctx.mode));
        }
        if ctx.profit_lock_stop {
            return reject(RejectReason::ProfitLockStopped);
        }
        if let Some(until_ms) = ctx.loss_streak_cooldown_until_ms
            && ctx.now_ms < until_ms
        {
            return reject(RejectReason::LossStreakCooldownActive { until_ms });
        }
        if ctx.orders_today >= self.config.max_orders_per_day {
            return reject(RejectReason::DailyOrderLimitReached {
                orders_today: ctx.orders_today,
                max_orders_per_day: self.config.max_orders_per_day,
            });
        }

        // Stage 2: Symbol TRADING, not quarantined, in universe, cooldown clear, Spot Long-only
        if intent.venue == Venue::Spot && intent.direction == Direction::Short {
            return reject(RejectReason::SpotShortUnsupported);
        }
        if !ctx.rules.status_trading {
            return reject(RejectReason::SymbolNotTrading);
        }
        if ctx.symbol_quarantined {
            return reject(RejectReason::SymbolQuarantined);
        }
        if !ctx.symbol_in_universe {
            return reject(RejectReason::SymbolNotInUniverse);
        }
        if let Some(until_ms) = ctx.symbol_cooldown_until_ms
            && ctx.now_ms < until_ms
        {
            return reject(RejectReason::SymbolCooldownActive { until_ms });
        }

        // Stage 3: BTC Crash Guard & macro blackout
        if ctx.btc_guard_active {
            return reject(RejectReason::BtcCrashGuardActive);
        }
        if ctx.macro_blackout_active {
            return reject(RejectReason::MacroBlackoutActive);
        }

        // Stage 4: Fresh market data (< 1500 ms), healthy user stream, clock drift (< 800 ms)
        if ctx.market_data_age_ms >= MAX_MARKET_DATA_AGE_MS {
            return reject(RejectReason::MarketDataStale {
                age_ms: ctx.market_data_age_ms,
            });
        }
        if !ctx.user_stream_healthy {
            return reject(RejectReason::UserStreamUnhealthy);
        }
        if ctx.clock_drift_ms.abs() >= MAX_CLOCK_DRIFT_MS {
            return reject(RejectReason::ClockDriftTooHigh {
                drift_ms: ctx.clock_drift_ms.abs(),
            });
        }

        // Stage 5: Open position count, duplicate symbol, scanner cap, correlated direction cap
        let active_positions: Vec<&Position> = ctx
            .open_positions
            .iter()
            .filter(|p| p.state != PositionState::Closed)
            .collect();

        if active_positions
            .iter()
            .any(|p| p.venue == intent.venue && p.symbol == intent.symbol)
        {
            return reject(RejectReason::DuplicateOpenSymbol);
        }
        if active_positions.len() >= usize::try_from(self.config.max_open_positions).unwrap_or(0) {
            return reject(RejectReason::MaxOpenPositionsReached {
                open: active_positions.len(),
                max: self.config.max_open_positions,
            });
        }
        if intent.is_scanner {
            let open_scanner = active_positions
                .iter()
                .filter(|p| p.strategy == StrategyId::GainersMomentum)
                .count();
            if open_scanner >= 2 {
                return reject(RejectReason::MaxScannerPositionsReached);
            }
        }
        let same_dir_count = active_positions
            .iter()
            .filter(|p| p.direction == intent.direction)
            .count();
        if same_dir_count >= usize::try_from(self.config.max_same_direction_correlated).unwrap_or(0)
        {
            return reject(RejectReason::MaxSameDirectionCorrelated {
                count: same_dir_count,
                max: self.config.max_same_direction_correlated,
            });
        }

        // Stage 6: Valid stop distance: 0.3% <= |entry - stop| / entry <= 8%
        let entry = intent.entry_price.as_decimal();
        let stop = intent.stop_price.as_decimal();
        if entry <= Decimal::ZERO || stop <= Decimal::ZERO {
            return reject(RejectReason::InvalidStopDistance {
                detail: "entry and stop prices must be positive".into(),
            });
        }
        let wrong_side = match intent.direction {
            Direction::Long => stop >= entry,
            Direction::Short => stop <= entry,
        };
        if wrong_side {
            return reject(RejectReason::InvalidStopDistance {
                detail: "stop price is on the wrong side of entry".into(),
            });
        }
        let stop_pct = (entry - stop)
            .abs()
            .checked_div(entry)
            .and_then(|r| r.checked_mul(Decimal::from(100_u32)))
            .unwrap_or(Decimal::MAX);
        if stop_pct < self.min_stop_pct || stop_pct > self.max_stop_pct {
            return reject(RejectReason::InvalidStopDistance {
                detail: format!(
                    "stop distance {stop_pct}% outside [{}%, {}%]",
                    self.min_stop_pct, self.max_stop_pct
                ),
            });
        }

        // Stage 7: Kelly reducer + PERCENT_PRICE_BY_SIDE + Spot sizing + heat + Stage 8 free quote
        let effective_risk_pct = match ctx.kelly {
            Some(k) => match evaluate_kelly(k, self.config.risk_per_trade_pct) {
                KellyDecision::InactiveInsufficientSamples => self.config.risk_per_trade_pct,
                KellyDecision::EffectiveRiskPct { risk_pct, .. } => risk_pct,
                KellyDecision::AutoDisableStrategy { .. } => {
                    return GateDecision::Reject {
                        reasons: vec![RejectReason::KellyAutoDisabled {
                            strategy: intent.strategy,
                        }],
                        auto_disable_strategy: true,
                    };
                }
            },
            None => self.config.risk_per_trade_pct,
        };

        if let Some((min_p, max_p)) = ctx.percent_price_bounds
            && (intent.entry_price < min_p || intent.entry_price > max_p)
        {
            return reject(RejectReason::PercentPriceViolation);
        }

        let (current_heat_pct, current_exposure_usd) =
            compute_portfolio_heat_and_exposure(&active_positions, ctx.equity_total_usd);

        let sizing_input = SpotSizingInput {
            equity: ctx.equity_total_usd,
            free_quote: ctx.free_quote_usd,
            entry_price: intent.entry_price,
            stop_price: intent.stop_price,
            rules: ctx.rules,
            risk_per_trade_pct: effective_risk_pct,
            size_mult: intent.size_mult,
            scanner_mult: intent.scanner_mult,
            profit_lock_mult: ctx.profit_lock_mult,
            sizing_mode: self.config.sizing_mode,
            fixed_notional_usd: ctx.fixed_notional_usd,
            max_notional_per_position_pct: self.config.max_notional_per_position_pct,
            current_portfolio_heat_pct: current_heat_pct,
            portfolio_heat_max_pct: self.config.portfolio_heat_max_pct,
            current_total_exposure_usd: current_exposure_usd,
            max_total_exposure_pct: self.config.max_total_exposure_pct,
        };

        let sized = match size_spot_position(&sizing_input) {
            Ok(s) => s,
            Err(err) => return reject(map_sizing_error(&err)),
        };

        // Stage 9: Rate-limit guard (REQUEST_WEIGHT < 60% and ORDERS 10s < 50%)
        if ctx.request_weight_pct >= MAX_REQUEST_WEIGHT_PCT
            || ctx.orders_10s_pct >= MAX_ORDERS_10S_PCT
        {
            return reject(RejectReason::RateLimitGuard {
                request_weight_pct: ctx.request_weight_pct,
                orders_10s_pct: ctx.orders_10s_pct,
            });
        }

        GateDecision::Accept {
            qty: sized.qty,
            notional_usd: sized.notional_usd,
            actual_risk_usd: sized.actual_risk_usd,
            heat_after_pct: sized.heat_after_pct,
        }
    }
}

/// Compute current combined portfolio heat `%` and total notional exposure (USD) across open positions.
#[must_use]
pub fn compute_portfolio_heat_and_exposure(
    positions: &[&Position],
    combined_equity: Decimal,
) -> (Decimal, Decimal) {
    let mut total_risk_usd = Decimal::ZERO;
    let mut total_exposure_usd = Decimal::ZERO;

    for pos in positions {
        let qty = pos.qty_open.as_decimal();
        let entry = pos.entry_price.as_decimal();
        let stop = pos.stop_price.as_decimal();
        // Risk of an open position: if stop has moved into profit (above entry for Long,
        // below entry for Short), remaining downside risk is 0.
        let risk_per_unit = match pos.direction {
            Direction::Long => (entry - stop).max(Decimal::ZERO),
            Direction::Short => (stop - entry).max(Decimal::ZERO),
        };
        total_risk_usd = total_risk_usd.saturating_add(qty.saturating_mul(risk_per_unit));
        total_exposure_usd = total_exposure_usd.saturating_add(qty.saturating_mul(entry));
    }

    let heat_pct = if combined_equity > Decimal::ZERO {
        total_risk_usd
            .checked_div(combined_equity)
            .and_then(|r| r.checked_mul(Decimal::from(100_u32)))
            .unwrap_or(Decimal::MAX)
    } else {
        Decimal::MAX
    };

    (heat_pct, total_exposure_usd)
}

fn reject(reason: RejectReason) -> GateDecision {
    GateDecision::Reject {
        reasons: vec![reason],
        auto_disable_strategy: false,
    }
}

fn map_sizing_error(err: &SizingError) -> RejectReason {
    RejectReason::SizingRejected(err.to_string())
}
