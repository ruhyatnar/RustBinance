//! Strict range and cross-field safety validation for [`BotConfig`] (§7.2, §7.8, §9).

use std::fmt::Display;
use std::net::SocketAddr;

use bot_types::{TradingMode, Venue};
use rust_decimal::Decimal;

use crate::{
    BinanceConfig, BotConfig, BreakoutConfig, ConfigError, ExitConfig, FuturesRiskConfig,
    GainersMomentumConfig, GeneralConfig, IpcConfig, MarginMode, PositionMode, RangeViolation,
    RiskConfig, StorageConfig, StrategyConfig, TrendPullbackConfig, UniverseConfig, VenuesConfig,
};

fn push_violation(
    violations: &mut Vec<RangeViolation>,
    field: &'static str,
    value: &(impl Display + ?Sized),
    reason: impl Into<String>,
) {
    violations.push(RangeViolation {
        field,
        value: value.to_string(),
        reason: reason.into(),
    });
}

fn check_dec_range(
    violations: &mut Vec<RangeViolation>,
    field: &'static str,
    value: Decimal,
    min: Decimal,
    max: Decimal,
) {
    if value < min || value > max {
        push_violation(
            violations,
            field,
            &value,
            format!("must be within [{min}, {max}]"),
        );
    }
}

fn check_u32_range(
    violations: &mut Vec<RangeViolation>,
    field: &'static str,
    value: u32,
    min: u32,
    max: u32,
) {
    if value < min || value > max {
        push_violation(
            violations,
            field,
            &value,
            format!("must be within [{min}, {max}]"),
        );
    }
}

fn check_u64_range(
    violations: &mut Vec<RangeViolation>,
    field: &'static str,
    value: u64,
    min: u64,
    max: u64,
) {
    if value < min || value > max {
        push_violation(
            violations,
            field,
            &value,
            format!("must be within [{min}, {max}]"),
        );
    }
}

fn check_positive(violations: &mut Vec<RangeViolation>, field: &'static str, value: Decimal) {
    if value <= Decimal::ZERO {
        push_violation(violations, field, &value, "must be > 0");
    }
}

fn check_unit_ratio(violations: &mut Vec<RangeViolation>, field: &'static str, value: Decimal) {
    if value <= Decimal::ZERO || value > Decimal::ONE {
        push_violation(violations, field, &value, "must be in (0, 1]");
    }
}

fn validate_general(general: &GeneralConfig, violations: &mut Vec<RangeViolation>) {
    if general.quote_assets.is_empty() {
        push_violation(
            violations,
            "general.quote_assets",
            "[]",
            "must specify at least one quote asset",
        );
    }
    for asset in &general.quote_assets {
        if asset.is_empty()
            || !asset
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        {
            push_violation(
                violations,
                "general.quote_assets",
                asset,
                "each quote asset must be non-empty uppercase ASCII",
            );
        }
    }
    if general.timezone_reporting.trim().is_empty() {
        push_violation(
            violations,
            "general.timezone_reporting",
            &general.timezone_reporting,
            "must not be empty",
        );
    }
}

fn validate_binance(binance: &BinanceConfig, violations: &mut Vec<RangeViolation>) {
    check_u64_range(
        violations,
        "binance.recv_window_ms",
        binance.recv_window_ms,
        100,
        60_000,
    );
    check_u64_range(
        violations,
        "binance.max_clock_drift_ms",
        binance.max_clock_drift_ms,
        100,
        5_000,
    );
    check_u64_range(
        violations,
        "binance.stale_market_ms",
        binance.stale_market_ms,
        200,
        30_000,
    );
}

fn validate_universe(universe: &UniverseConfig, violations: &mut Vec<RangeViolation>) {
    check_positive(
        violations,
        "universe.min_quote_volume_24h_usd",
        universe.min_quote_volume_24h_usd,
    );
    check_positive(
        violations,
        "universe.scanner_min_quote_volume_24h_usd",
        universe.scanner_min_quote_volume_24h_usd,
    );
    if universe.scanner_min_quote_volume_24h_usd > universe.min_quote_volume_24h_usd {
        push_violation(
            violations,
            "universe.scanner_min_quote_volume_24h_usd",
            &universe.scanner_min_quote_volume_24h_usd,
            format!(
                "must be <= universe.min_quote_volume_24h_usd ({})",
                universe.min_quote_volume_24h_usd
            ),
        );
    }
    check_dec_range(
        violations,
        "universe.max_spread_bps",
        universe.max_spread_bps,
        Decimal::new(1, 1),
        Decimal::from(200_u32),
    );
    check_dec_range(
        violations,
        "universe.scanner_max_spread_bps",
        universe.scanner_max_spread_bps,
        Decimal::new(1, 1),
        Decimal::from(500_u32),
    );
    check_u32_range(
        violations,
        "universe.refresh_minutes",
        universe.refresh_minutes,
        1,
        1440,
    );
}

fn validate_trend_pullback(tp: &TrendPullbackConfig, violations: &mut Vec<RangeViolation>) {
    check_dec_range(
        violations,
        "strategy.trend_pullback.rsi_min",
        tp.rsi_min,
        Decimal::ZERO,
        Decimal::from(100_u32),
    );
    check_dec_range(
        violations,
        "strategy.trend_pullback.rsi_max",
        tp.rsi_max,
        Decimal::ZERO,
        Decimal::from(100_u32),
    );
    if tp.rsi_min >= tp.rsi_max {
        push_violation(
            violations,
            "strategy.trend_pullback.rsi_min",
            &tp.rsi_min,
            format!("must be < rsi_max ({})", tp.rsi_max),
        );
    }
    check_dec_range(
        violations,
        "strategy.trend_pullback.adx_min",
        tp.adx_min,
        Decimal::ZERO,
        Decimal::from(100_u32),
    );
    check_positive(violations, "strategy.trend_pullback.tp1_r", tp.tp1_r);
    if tp.tp2_r <= tp.tp1_r {
        push_violation(
            violations,
            "strategy.trend_pullback.tp2_r",
            &tp.tp2_r,
            format!("must be > tp1_r ({})", tp.tp1_r),
        );
    }
    check_unit_ratio(
        violations,
        "strategy.trend_pullback.tp1_close_ratio",
        tp.tp1_close_ratio,
    );
    if tp.cost_multiple < Decimal::ONE {
        push_violation(
            violations,
            "strategy.trend_pullback.cost_multiple",
            &tp.cost_multiple,
            "must be >= 1",
        );
    }
}

fn validate_breakout_and_gainers(
    br: &BreakoutConfig,
    gm: &GainersMomentumConfig,
    violations: &mut Vec<RangeViolation>,
) {
    if br.donchian < 2 {
        push_violation(
            violations,
            "strategy.breakout.donchian",
            &br.donchian,
            "must be >= 2",
        );
    }
    check_positive(
        violations,
        "strategy.breakout.min_volume_mult",
        br.min_volume_mult,
    );
    check_positive(
        violations,
        "strategy.gainers_momentum.min_gain_24h_pct",
        gm.min_gain_24h_pct,
    );
    if gm.max_gain_24h_pct <= gm.min_gain_24h_pct {
        push_violation(
            violations,
            "strategy.gainers_momentum.max_gain_24h_pct",
            &gm.max_gain_24h_pct,
            format!("must be > min_gain_24h_pct ({})", gm.min_gain_24h_pct),
        );
    }
    check_unit_ratio(
        violations,
        "strategy.gainers_momentum.size_multiplier",
        gm.size_multiplier,
    );
    if gm.max_concurrent < 1 {
        push_violation(
            violations,
            "strategy.gainers_momentum.max_concurrent",
            &gm.max_concurrent,
            "must be >= 1",
        );
    }
    if gm.time_stop_hours < 1 {
        push_violation(
            violations,
            "strategy.gainers_momentum.time_stop_hours",
            &gm.time_stop_hours,
            "must be >= 1",
        );
    }
}

fn validate_strategy(strategy: &StrategyConfig, violations: &mut Vec<RangeViolation>) {
    validate_trend_pullback(&strategy.trend_pullback, violations);
    validate_breakout_and_gainers(&strategy.breakout, &strategy.gainers_momentum, violations);
}

fn validate_exit(exit: &ExitConfig, violations: &mut Vec<RangeViolation>) {
    check_positive(violations, "exit.be_trigger_r", exit.be_trigger_r);
    if exit.be_fee_buffer_bps < Decimal::ZERO {
        push_violation(
            violations,
            "exit.be_fee_buffer_bps",
            &exit.be_fee_buffer_bps,
            "must be >= 0",
        );
    }
    check_positive(violations, "exit.trail_activate_r", exit.trail_activate_r);
    check_positive(violations, "exit.trail_atr_mult", exit.trail_atr_mult);
    check_positive(
        violations,
        "exit.trail_min_step_atr",
        exit.trail_min_step_atr,
    );
    check_dec_range(
        violations,
        "exit.exhaustion_rsi",
        exit.exhaustion_rsi,
        Decimal::from(50_u32),
        Decimal::from(100_u32),
    );
    check_unit_ratio(
        violations,
        "exit.exhaustion_trail_factor",
        exit.exhaustion_trail_factor,
    );
    if exit.max_bars_in_trade < 1 {
        push_violation(
            violations,
            "exit.max_bars_in_trade",
            &exit.max_bars_in_trade,
            "must be >= 1",
        );
    }
    check_u64_range(
        violations,
        "exit.hard_stop_grace_ms",
        exit.hard_stop_grace_ms,
        100,
        30_000,
    );
}

fn validate_risk_exposure(risk: &RiskConfig, violations: &mut Vec<RangeViolation>) {
    check_dec_range(
        violations,
        "risk.risk_per_trade_pct",
        risk.risk_per_trade_pct,
        Decimal::new(1, 1),
        Decimal::ONE,
    );
    check_u32_range(
        violations,
        "risk.max_open_positions",
        risk.max_open_positions,
        1,
        8,
    );
    check_dec_range(
        violations,
        "risk.portfolio_heat_max_pct",
        risk.portfolio_heat_max_pct,
        Decimal::new(5, 1),
        Decimal::from(4_u32),
    );
    check_dec_range(
        violations,
        "risk.max_notional_per_position_pct",
        risk.max_notional_per_position_pct,
        Decimal::from(5_u32),
        Decimal::from(35_u32),
    );
    check_dec_range(
        violations,
        "risk.max_total_exposure_pct",
        risk.max_total_exposure_pct,
        Decimal::from(20_u32),
        Decimal::from(80_u32),
    );
    if risk.max_total_exposure_pct < risk.max_notional_per_position_pct {
        push_violation(
            violations,
            "risk.max_total_exposure_pct",
            &risk.max_total_exposure_pct,
            format!(
                "must be >= max_notional_per_position_pct ({})",
                risk.max_notional_per_position_pct
            ),
        );
    }
    check_u32_range(
        violations,
        "risk.max_same_direction_correlated",
        risk.max_same_direction_correlated,
        1,
        5,
    );
}

fn validate_risk_breakers(risk: &RiskConfig, violations: &mut Vec<RangeViolation>) {
    check_dec_range(
        violations,
        "risk.daily_loss_halt_pct",
        risk.daily_loss_halt_pct,
        Decimal::new(-40, 1),
        Decimal::new(-5, 1),
    );
    check_dec_range(
        violations,
        "risk.weekly_loss_halt_pct",
        risk.weekly_loss_halt_pct,
        Decimal::new(-150, 1),
        Decimal::new(-10, 1),
    );
    if risk.weekly_loss_halt_pct > risk.daily_loss_halt_pct {
        push_violation(
            violations,
            "risk.weekly_loss_halt_pct",
            &risk.weekly_loss_halt_pct,
            format!(
                "must be <= daily_loss_halt_pct ({})",
                risk.daily_loss_halt_pct
            ),
        );
    }
    check_dec_range(
        violations,
        "risk.drawdown_kill_pct",
        risk.drawdown_kill_pct,
        Decimal::new(-200, 1),
        Decimal::new(-50, 1),
    );
    if risk.drawdown_kill_pct > risk.weekly_loss_halt_pct {
        push_violation(
            violations,
            "risk.drawdown_kill_pct",
            &risk.drawdown_kill_pct,
            format!(
                "must be <= weekly_loss_halt_pct ({})",
                risk.weekly_loss_halt_pct
            ),
        );
    }
    check_dec_range(
        violations,
        "risk.max_entry_slippage_bps",
        risk.max_entry_slippage_bps,
        Decimal::from(2_u32),
        Decimal::from(25_u32),
    );
    check_u32_range(
        violations,
        "risk.max_orders_per_day",
        risk.max_orders_per_day,
        1,
        1000,
    );
    check_dec_range(
        violations,
        "risk.btc_crash_15m_pct",
        risk.btc_crash_15m_pct,
        Decimal::new(5, 1),
        Decimal::from(10_u32),
    );
    check_dec_range(
        violations,
        "risk.btc_crash_60m_pct",
        risk.btc_crash_60m_pct,
        Decimal::ONE,
        Decimal::from(20_u32),
    );
    if risk.btc_crash_60m_pct < risk.btc_crash_15m_pct {
        push_violation(
            violations,
            "risk.btc_crash_60m_pct",
            &risk.btc_crash_60m_pct,
            format!("must be >= btc_crash_15m_pct ({})", risk.btc_crash_15m_pct),
        );
    }
}

fn validate_futures_risk_core(
    fut: &FuturesRiskConfig,
    futures_max_leverage: u32,
    violations: &mut Vec<RangeViolation>,
) {
    check_dec_range(
        violations,
        "risk.futures.risk_per_trade_pct",
        fut.risk_per_trade_pct,
        Decimal::new(1, 1),
        Decimal::new(75, 2),
    );
    check_dec_range(
        violations,
        "risk.futures.short_size_multiplier",
        fut.short_size_multiplier,
        Decimal::new(25, 2),
        Decimal::ONE,
    );
    check_dec_range(
        violations,
        "risk.futures.max_effective_leverage",
        fut.max_effective_leverage,
        Decimal::new(5, 1),
        Decimal::from(3_u32),
    );
    if fut.max_effective_leverage > Decimal::from(futures_max_leverage) {
        push_violation(
            violations,
            "risk.futures.max_effective_leverage",
            &fut.max_effective_leverage,
            format!("must be <= venues.futures.max_leverage ({futures_max_leverage})"),
        );
    }
    check_dec_range(
        violations,
        "risk.futures.liq_buffer_mult",
        fut.liq_buffer_mult,
        Decimal::from(2_u32),
        Decimal::from(6_u32),
    );
    check_dec_range(
        violations,
        "risk.futures.liq_emergency_mult",
        fut.liq_emergency_mult,
        Decimal::ONE,
        Decimal::from(2_u32),
    );
    if fut.liq_emergency_mult >= fut.liq_buffer_mult {
        push_violation(
            violations,
            "risk.futures.liq_emergency_mult",
            &fut.liq_emergency_mult,
            format!("must be < liq_buffer_mult ({})", fut.liq_buffer_mult),
        );
    }
}

fn validate_futures_risk_guards(fut: &FuturesRiskConfig, violations: &mut Vec<RangeViolation>) {
    check_dec_range(
        violations,
        "risk.futures.funding_adverse_block_pct",
        fut.funding_adverse_block_pct,
        Decimal::new(1, 2),
        Decimal::new(1, 1),
    );
    check_dec_range(
        violations,
        "risk.futures.max_funding_cost_r",
        fut.max_funding_cost_r,
        Decimal::new(1, 2),
        Decimal::new(5, 1),
    );
    check_dec_range(
        violations,
        "risk.futures.mark_last_divergence_pct",
        fut.mark_last_divergence_pct,
        Decimal::new(2, 1),
        Decimal::ONE,
    );
    check_dec_range(
        violations,
        "risk.futures.max_notional_per_position_pct",
        fut.max_notional_per_position_pct,
        Decimal::from(5_u32),
        Decimal::from(35_u32),
    );
    check_u32_range(
        violations,
        "risk.futures.max_open_positions",
        fut.max_open_positions,
        1,
        6,
    );
    check_dec_range(
        violations,
        "risk.futures.btc_spike_15m_pct",
        fut.btc_spike_15m_pct,
        Decimal::new(5, 1),
        Decimal::from(10_u32),
    );
    check_dec_range(
        violations,
        "risk.futures.btc_spike_60m_pct",
        fut.btc_spike_60m_pct,
        Decimal::ONE,
        Decimal::from(20_u32),
    );
    if fut.btc_spike_60m_pct < fut.btc_spike_15m_pct {
        push_violation(
            violations,
            "risk.futures.btc_spike_60m_pct",
            &fut.btc_spike_60m_pct,
            format!("must be >= btc_spike_15m_pct ({})", fut.btc_spike_15m_pct),
        );
    }
}

fn validate_venues(mode: TradingMode, venues: &VenuesConfig, violations: &mut Vec<RangeViolation>) {
    if venues.switch_cooldown_s < 1 {
        push_violation(
            violations,
            "venues.switch_cooldown_s",
            &venues.switch_cooldown_s,
            "must be >= 1",
        );
    }
    check_u32_range(
        violations,
        "venues.max_switches_per_day",
        venues.max_switches_per_day,
        0,
        24,
    );
    if venues.spot.capital_allocation_usd < Decimal::ZERO {
        push_violation(
            violations,
            "venues.spot.capital_allocation_usd",
            &venues.spot.capital_allocation_usd,
            "must be >= 0",
        );
    }
    if venues.futures.capital_allocation_usd < Decimal::ZERO {
        push_violation(
            violations,
            "venues.futures.capital_allocation_usd",
            &venues.futures.capital_allocation_usd,
            "must be >= 0",
        );
    }
    if venues.futures.margin_mode != MarginMode::Isolated {
        push_violation(
            violations,
            "venues.futures.margin_mode",
            "cross",
            "only isolated margin is permitted (§7.8)",
        );
    }
    if venues.futures.position_mode != PositionMode::OneWay {
        push_violation(
            violations,
            "venues.futures.position_mode",
            "hedge",
            "only one_way position mode is permitted (§9)",
        );
    }
    check_u32_range(
        violations,
        "venues.futures.max_leverage",
        venues.futures.max_leverage,
        1,
        5,
    );
    check_positive(
        violations,
        "venues.futures.min_quote_volume_24h_usd",
        venues.futures.min_quote_volume_24h_usd,
    );
    let default_enabled = match venues.default_active {
        Venue::Spot => venues.spot.enabled,
        Venue::UsdsFutures => venues.futures.enabled,
    };
    if !default_enabled {
        push_violation(
            violations,
            "venues.default_active",
            &venues.default_active,
            "default_active venue must have enabled = true",
        );
    }
    if mode == TradingMode::Live {
        if venues.spot.enabled && venues.spot.live_confirmation_file.trim().is_empty() {
            push_violation(
                violations,
                "venues.spot.live_confirmation_file",
                "",
                "live mode requires a non-empty live_confirmation_file for enabled spot venue",
            );
        }
        if venues.futures.enabled && venues.futures.live_confirmation_file.trim().is_empty() {
            push_violation(
                violations,
                "venues.futures.live_confirmation_file",
                "",
                "live mode requires a non-empty live_confirmation_file for enabled futures venue",
            );
        }
    }
}

fn validate_ipc_and_storage(
    ipc: &IpcConfig,
    storage: &StorageConfig,
    violations: &mut Vec<RangeViolation>,
) {
    if ipc.uds_path.trim().is_empty() {
        push_violation(
            violations,
            "ipc.uds_path",
            &ipc.uds_path,
            "must not be empty",
        );
    }
    if ipc.web_bind.parse::<SocketAddr>().is_err() {
        push_violation(
            violations,
            "ipc.web_bind",
            &ipc.web_bind,
            "must be a valid SocketAddr (e.g. 127.0.0.1:3001)",
        );
    }
    check_u32_range(violations, "ipc.telemetry_hz", ipc.telemetry_hz, 1, 60);
    if storage.db_path.trim().is_empty() {
        push_violation(
            violations,
            "storage.db_path",
            &storage.db_path,
            "must not be empty",
        );
    }
    if storage.backup_dir.trim().is_empty() {
        push_violation(
            violations,
            "storage.backup_dir",
            &storage.backup_dir,
            "must not be empty",
        );
    }
}

impl BotConfig {
    /// Validates all parameters against strict §7.2, §7.8, and §9 bounds without clamping.
    ///
    /// # Errors
    /// Returns [`ConfigError::Validation`] listing every violated field and reason.
    pub fn validate(&self) -> Result<(), ConfigError> {
        let mut violations = Vec::new();
        validate_general(&self.general, &mut violations);
        validate_binance(&self.binance, &mut violations);
        validate_universe(&self.universe, &mut violations);
        validate_strategy(&self.strategy, &mut violations);
        validate_exit(&self.exit, &mut violations);
        validate_risk_exposure(&self.risk, &mut violations);
        validate_risk_breakers(&self.risk, &mut violations);
        validate_futures_risk_core(
            &self.risk.futures,
            self.venues.futures.max_leverage,
            &mut violations,
        );
        validate_futures_risk_guards(&self.risk.futures, &mut violations);
        validate_venues(self.general.mode, &self.venues, &mut violations);
        validate_ipc_and_storage(&self.ipc, &self.storage, &mut violations);

        if violations.is_empty() {
            Ok(())
        } else {
            Err(ConfigError::Validation(violations))
        }
    }
}
