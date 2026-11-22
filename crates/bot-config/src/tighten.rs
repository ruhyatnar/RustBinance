//! Runtime tighten-only configuration guard (§7.8, §9, Hard Rule #8).
//!
//! Every struct in [`BotConfig`] is exhaustively destructured so adding a new config field
//! produces a compile error until its runtime tightening policy is explicitly defined.

use std::collections::HashSet;
use std::fmt::{Debug, Display};

use rust_decimal::Decimal;

use crate::{
    BinanceConfig, BotConfig, BreakoutConfig, CrossVenueSameSymbolPolicy, ExitConfig,
    FuturesRiskConfig, FuturesVenueConfig, GainersMomentumConfig, GeneralConfig, IpcConfig,
    LoosenError, LoosenViolation, RiskConfig, SafetyConfig, SpotVenueConfig, StorageConfig,
    StrategyConfig, TrendPullbackConfig, UniverseConfig, VenuesConfig, VwapReversionConfig,
};

fn push_loosen(
    violations: &mut Vec<LoosenViolation>,
    field: &'static str,
    old: &(impl Display + ?Sized),
    new: &(impl Display + ?Sized),
    reason: &'static str,
) {
    violations.push(LoosenViolation {
        field,
        old: old.to_string(),
        new: new.to_string(),
        reason,
    });
}

fn check_le<T: PartialOrd + Display>(
    violations: &mut Vec<LoosenViolation>,
    field: &'static str,
    old: &T,
    new: &T,
) {
    if new > old {
        push_loosen(violations, field, old, new, "must decrease or stay equal");
    }
}

fn check_ge<T: PartialOrd + Display>(
    violations: &mut Vec<LoosenViolation>,
    field: &'static str,
    old: &T,
    new: &T,
) {
    if new < old {
        push_loosen(violations, field, old, new, "must increase or stay equal");
    }
}

fn check_disable_only(
    violations: &mut Vec<LoosenViolation>,
    field: &'static str,
    old: bool,
    new: bool,
) {
    if !old && new {
        push_loosen(
            violations,
            field,
            &old,
            &new,
            "cannot be re-enabled at runtime",
        );
    }
}

fn check_enable_only(
    violations: &mut Vec<LoosenViolation>,
    field: &'static str,
    old: bool,
    new: bool,
) {
    if old && !new {
        push_loosen(
            violations,
            field,
            &old,
            &new,
            "safety guard cannot be disabled at runtime",
        );
    }
}

fn check_immutable<T: PartialEq + Debug>(
    violations: &mut Vec<LoosenViolation>,
    field: &'static str,
    old: &T,
    new: &T,
) {
    if old != new {
        violations.push(LoosenViolation {
            field,
            old: format!("{old:?}"),
            new: format!("{new:?}"),
            reason: "immutable at runtime; requires restart or reload --allow-loosen",
        });
    }
}

fn check_capital_cap(
    violations: &mut Vec<LoosenViolation>,
    field: &'static str,
    old: Decimal,
    new: Decimal,
) {
    // 0 means unbounded (full venue quote balance).
    let loosened = match (old.is_zero(), new.is_zero()) {
        (true, _) => false,          // unbounded -> unbounded or bounded (tightening)
        (false, true) => true,       // bounded -> unbounded (loosening)
        (false, false) => new > old, // raising cap (loosening)
    };
    if loosened {
        push_loosen(
            violations,
            field,
            &old,
            &new,
            "capital allocation cap cannot be raised or removed (0 = unbounded) at runtime",
        );
    }
}

fn check_general_safety_binance(
    old_g: &GeneralConfig,
    new_g: &GeneralConfig,
    old_s: &SafetyConfig,
    new_s: &SafetyConfig,
    old_b: &BinanceConfig,
    new_b: &BinanceConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let GeneralConfig {
        mode,
        quote_assets,
        timezone_reporting,
    } = old_g;
    check_immutable(violations, "general.mode", mode, &new_g.mode);
    check_immutable(
        violations,
        "general.quote_assets",
        quote_assets,
        &new_g.quote_assets,
    );
    check_immutable(
        violations,
        "general.timezone_reporting",
        timezone_reporting,
        &new_g.timezone_reporting,
    );

    let SafetyConfig { kill_flatten } = old_s;
    check_enable_only(
        violations,
        "safety.kill_flatten",
        *kill_flatten,
        new_s.kill_flatten,
    );

    let BinanceConfig {
        recv_window_ms,
        rest_fallback,
        max_clock_drift_ms,
        stale_market_ms,
    } = old_b;
    check_immutable(
        violations,
        "binance.recv_window_ms",
        recv_window_ms,
        &new_b.recv_window_ms,
    );
    check_immutable(
        violations,
        "binance.rest_fallback",
        rest_fallback,
        &new_b.rest_fallback,
    );
    check_immutable(
        violations,
        "binance.max_clock_drift_ms",
        max_clock_drift_ms,
        &new_b.max_clock_drift_ms,
    );
    check_immutable(
        violations,
        "binance.stale_market_ms",
        stale_market_ms,
        &new_b.stale_market_ms,
    );
}

fn check_universe(
    old: &UniverseConfig,
    new: &UniverseConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let UniverseConfig {
        min_quote_volume_24h_usd,
        scanner_min_quote_volume_24h_usd,
        min_listing_age_days,
        scanner_min_listing_age_days,
        max_spread_bps,
        scanner_max_spread_bps,
        refresh_minutes,
        exclude_symbols,
    } = old;
    check_ge(
        violations,
        "universe.min_quote_volume_24h_usd",
        min_quote_volume_24h_usd,
        &new.min_quote_volume_24h_usd,
    );
    check_ge(
        violations,
        "universe.scanner_min_quote_volume_24h_usd",
        scanner_min_quote_volume_24h_usd,
        &new.scanner_min_quote_volume_24h_usd,
    );
    check_ge(
        violations,
        "universe.min_listing_age_days",
        min_listing_age_days,
        &new.min_listing_age_days,
    );
    check_ge(
        violations,
        "universe.scanner_min_listing_age_days",
        scanner_min_listing_age_days,
        &new.scanner_min_listing_age_days,
    );
    check_le(
        violations,
        "universe.max_spread_bps",
        max_spread_bps,
        &new.max_spread_bps,
    );
    check_le(
        violations,
        "universe.scanner_max_spread_bps",
        scanner_max_spread_bps,
        &new.scanner_max_spread_bps,
    );
    check_immutable(
        violations,
        "universe.refresh_minutes",
        refresh_minutes,
        &new.refresh_minutes,
    );

    let new_set: HashSet<&str> = new.exclude_symbols.iter().map(String::as_str).collect();
    if !exclude_symbols
        .iter()
        .all(|sym| new_set.contains(sym.as_str()))
    {
        violations.push(LoosenViolation {
            field: "universe.exclude_symbols",
            old: format!("{exclude_symbols:?}"),
            new: format!("{:?}", new.exclude_symbols),
            reason: "excluded symbols cannot be removed at runtime",
        });
    }
}

fn check_strategy_trend_and_breakout(
    old_tp: &TrendPullbackConfig,
    new_tp: &TrendPullbackConfig,
    old_br: &BreakoutConfig,
    new_br: &BreakoutConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let TrendPullbackConfig {
        enabled,
        signal_tf,
        htf,
        rsi_min,
        rsi_max,
        adx_min,
        tp1_r,
        tp2_r,
        tp1_close_ratio,
        cost_multiple,
    } = old_tp;
    check_disable_only(
        violations,
        "strategy.trend_pullback.enabled",
        *enabled,
        new_tp.enabled,
    );
    check_immutable(
        violations,
        "strategy.trend_pullback.signal_tf",
        signal_tf,
        &new_tp.signal_tf,
    );
    check_immutable(violations, "strategy.trend_pullback.htf", htf, &new_tp.htf);
    check_immutable(
        violations,
        "strategy.trend_pullback.rsi_min",
        rsi_min,
        &new_tp.rsi_min,
    );
    check_immutable(
        violations,
        "strategy.trend_pullback.rsi_max",
        rsi_max,
        &new_tp.rsi_max,
    );
    check_immutable(
        violations,
        "strategy.trend_pullback.adx_min",
        adx_min,
        &new_tp.adx_min,
    );
    check_immutable(
        violations,
        "strategy.trend_pullback.tp1_r",
        tp1_r,
        &new_tp.tp1_r,
    );
    check_immutable(
        violations,
        "strategy.trend_pullback.tp2_r",
        tp2_r,
        &new_tp.tp2_r,
    );
    check_immutable(
        violations,
        "strategy.trend_pullback.tp1_close_ratio",
        tp1_close_ratio,
        &new_tp.tp1_close_ratio,
    );
    check_immutable(
        violations,
        "strategy.trend_pullback.cost_multiple",
        cost_multiple,
        &new_tp.cost_multiple,
    );

    let BreakoutConfig {
        enabled: br_enabled,
        donchian,
        min_volume_mult,
    } = old_br;
    check_disable_only(
        violations,
        "strategy.breakout.enabled",
        *br_enabled,
        new_br.enabled,
    );
    check_immutable(
        violations,
        "strategy.breakout.donchian",
        donchian,
        &new_br.donchian,
    );
    check_immutable(
        violations,
        "strategy.breakout.min_volume_mult",
        min_volume_mult,
        &new_br.min_volume_mult,
    );
}

fn check_strategy_gainers_and_vwap(
    old_gm: &GainersMomentumConfig,
    new_gm: &GainersMomentumConfig,
    old_vr: &VwapReversionConfig,
    new_vr: &VwapReversionConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let GainersMomentumConfig {
        enabled,
        shadow_only,
        min_gain_24h_pct,
        max_gain_24h_pct,
        size_multiplier,
        max_concurrent,
        time_stop_hours,
    } = old_gm;
    check_disable_only(
        violations,
        "strategy.gainers_momentum.enabled",
        *enabled,
        new_gm.enabled,
    );
    check_enable_only(
        violations,
        "strategy.gainers_momentum.shadow_only",
        *shadow_only,
        new_gm.shadow_only,
    );
    check_immutable(
        violations,
        "strategy.gainers_momentum.min_gain_24h_pct",
        min_gain_24h_pct,
        &new_gm.min_gain_24h_pct,
    );
    check_immutable(
        violations,
        "strategy.gainers_momentum.max_gain_24h_pct",
        max_gain_24h_pct,
        &new_gm.max_gain_24h_pct,
    );
    check_le(
        violations,
        "strategy.gainers_momentum.size_multiplier",
        size_multiplier,
        &new_gm.size_multiplier,
    );
    check_le(
        violations,
        "strategy.gainers_momentum.max_concurrent",
        max_concurrent,
        &new_gm.max_concurrent,
    );
    check_immutable(
        violations,
        "strategy.gainers_momentum.time_stop_hours",
        time_stop_hours,
        &new_gm.time_stop_hours,
    );

    let VwapReversionConfig {
        enabled: vr_enabled,
        shadow_only: vr_shadow,
    } = old_vr;
    check_disable_only(
        violations,
        "strategy.vwap_reversion.enabled",
        *vr_enabled,
        new_vr.enabled,
    );
    check_enable_only(
        violations,
        "strategy.vwap_reversion.shadow_only",
        *vr_shadow,
        new_vr.shadow_only,
    );
}

fn check_exit(old: &ExitConfig, new: &ExitConfig, violations: &mut Vec<LoosenViolation>) {
    let ExitConfig {
        be_trigger_r,
        be_fee_buffer_bps,
        trail_activate_r,
        trail_atr_mult,
        trail_min_step_atr,
        exhaustion_rsi,
        exhaustion_trail_factor,
        max_bars_in_trade,
        hard_stop_grace_ms,
    } = old;
    check_immutable(
        violations,
        "exit.be_trigger_r",
        be_trigger_r,
        &new.be_trigger_r,
    );
    check_immutable(
        violations,
        "exit.be_fee_buffer_bps",
        be_fee_buffer_bps,
        &new.be_fee_buffer_bps,
    );
    check_immutable(
        violations,
        "exit.trail_activate_r",
        trail_activate_r,
        &new.trail_activate_r,
    );
    check_immutable(
        violations,
        "exit.trail_atr_mult",
        trail_atr_mult,
        &new.trail_atr_mult,
    );
    check_immutable(
        violations,
        "exit.trail_min_step_atr",
        trail_min_step_atr,
        &new.trail_min_step_atr,
    );
    check_immutable(
        violations,
        "exit.exhaustion_rsi",
        exhaustion_rsi,
        &new.exhaustion_rsi,
    );
    check_immutable(
        violations,
        "exit.exhaustion_trail_factor",
        exhaustion_trail_factor,
        &new.exhaustion_trail_factor,
    );
    check_immutable(
        violations,
        "exit.max_bars_in_trade",
        max_bars_in_trade,
        &new.max_bars_in_trade,
    );
    check_immutable(
        violations,
        "exit.hard_stop_grace_ms",
        hard_stop_grace_ms,
        &new.hard_stop_grace_ms,
    );
}

fn check_risk(old: &RiskConfig, new: &RiskConfig, violations: &mut Vec<LoosenViolation>) {
    let RiskConfig {
        sizing_mode,
        risk_per_trade_pct,
        max_open_positions,
        portfolio_heat_max_pct,
        max_notional_per_position_pct,
        max_total_exposure_pct,
        max_same_direction_correlated,
        daily_loss_halt_pct,
        weekly_loss_halt_pct,
        drawdown_kill_pct,
        profit_lock_mode,
        max_entry_slippage_bps,
        max_orders_per_day,
        orphan_policy,
        btc_crash_15m_pct,
        btc_crash_60m_pct,
        futures,
    } = old;
    check_immutable(
        violations,
        "risk.sizing_mode",
        sizing_mode,
        &new.sizing_mode,
    );
    check_le(
        violations,
        "risk.risk_per_trade_pct",
        risk_per_trade_pct,
        &new.risk_per_trade_pct,
    );
    check_le(
        violations,
        "risk.max_open_positions",
        max_open_positions,
        &new.max_open_positions,
    );
    check_le(
        violations,
        "risk.portfolio_heat_max_pct",
        portfolio_heat_max_pct,
        &new.portfolio_heat_max_pct,
    );
    check_le(
        violations,
        "risk.max_notional_per_position_pct",
        max_notional_per_position_pct,
        &new.max_notional_per_position_pct,
    );
    check_le(
        violations,
        "risk.max_total_exposure_pct",
        max_total_exposure_pct,
        &new.max_total_exposure_pct,
    );
    check_le(
        violations,
        "risk.max_same_direction_correlated",
        max_same_direction_correlated,
        &new.max_same_direction_correlated,
    );
    check_risk_breakers(
        (
            daily_loss_halt_pct,
            weekly_loss_halt_pct,
            drawdown_kill_pct,
            *profit_lock_mode,
        ),
        (
            max_entry_slippage_bps,
            *max_orders_per_day,
            *orphan_policy,
            btc_crash_15m_pct,
            btc_crash_60m_pct,
        ),
        new,
        violations,
    );
    check_futures_risk(futures, &new.futures, violations);
}

fn check_risk_breakers(
    loss_and_lock: (&Decimal, &Decimal, &Decimal, crate::ProfitLockMode),
    guards: (&Decimal, u32, crate::OrphanPolicy, &Decimal, &Decimal),
    new: &RiskConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let (daily_loss_halt_pct, weekly_loss_halt_pct, drawdown_kill_pct, profit_lock_mode) =
        loss_and_lock;
    let (
        max_entry_slippage_bps,
        max_orders_per_day,
        orphan_policy,
        btc_crash_15m_pct,
        btc_crash_60m_pct,
    ) = guards;
    check_ge(
        violations,
        "risk.daily_loss_halt_pct",
        daily_loss_halt_pct,
        &new.daily_loss_halt_pct,
    );
    check_ge(
        violations,
        "risk.weekly_loss_halt_pct",
        weekly_loss_halt_pct,
        &new.weekly_loss_halt_pct,
    );
    check_ge(
        violations,
        "risk.drawdown_kill_pct",
        drawdown_kill_pct,
        &new.drawdown_kill_pct,
    );
    if new.profit_lock_mode < profit_lock_mode {
        violations.push(LoosenViolation {
            field: "risk.profit_lock_mode",
            old: format!("{profit_lock_mode:?}"),
            new: format!("{:?}", new.profit_lock_mode),
            reason: "profit_lock_mode can only become stricter (Off < Reduce < Stop)",
        });
    }
    check_le(
        violations,
        "risk.max_entry_slippage_bps",
        max_entry_slippage_bps,
        &new.max_entry_slippage_bps,
    );
    check_le(
        violations,
        "risk.max_orders_per_day",
        &max_orders_per_day,
        &new.max_orders_per_day,
    );
    check_immutable(
        violations,
        "risk.orphan_policy",
        &orphan_policy,
        &new.orphan_policy,
    );
    check_le(
        violations,
        "risk.btc_crash_15m_pct",
        btc_crash_15m_pct,
        &new.btc_crash_15m_pct,
    );
    check_le(
        violations,
        "risk.btc_crash_60m_pct",
        btc_crash_60m_pct,
        &new.btc_crash_60m_pct,
    );
}

fn check_futures_risk(
    old: &FuturesRiskConfig,
    new: &FuturesRiskConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let FuturesRiskConfig {
        risk_per_trade_pct,
        short_size_multiplier,
        max_effective_leverage,
        liq_buffer_mult,
        liq_emergency_mult,
        funding_adverse_block_pct,
        max_funding_cost_r,
        mark_last_divergence_pct,
        max_notional_per_position_pct,
        max_open_positions,
        btc_spike_15m_pct,
        btc_spike_60m_pct,
    } = old;
    check_le(
        violations,
        "risk.futures.risk_per_trade_pct",
        risk_per_trade_pct,
        &new.risk_per_trade_pct,
    );
    check_le(
        violations,
        "risk.futures.short_size_multiplier",
        short_size_multiplier,
        &new.short_size_multiplier,
    );
    check_le(
        violations,
        "risk.futures.max_effective_leverage",
        max_effective_leverage,
        &new.max_effective_leverage,
    );
    check_ge(
        violations,
        "risk.futures.liq_buffer_mult",
        liq_buffer_mult,
        &new.liq_buffer_mult,
    );
    check_ge(
        violations,
        "risk.futures.liq_emergency_mult",
        liq_emergency_mult,
        &new.liq_emergency_mult,
    );
    check_le(
        violations,
        "risk.futures.funding_adverse_block_pct",
        funding_adverse_block_pct,
        &new.funding_adverse_block_pct,
    );
    check_le(
        violations,
        "risk.futures.max_funding_cost_r",
        max_funding_cost_r,
        &new.max_funding_cost_r,
    );
    check_le(
        violations,
        "risk.futures.mark_last_divergence_pct",
        mark_last_divergence_pct,
        &new.mark_last_divergence_pct,
    );
    check_le(
        violations,
        "risk.futures.max_notional_per_position_pct",
        max_notional_per_position_pct,
        &new.max_notional_per_position_pct,
    );
    check_le(
        violations,
        "risk.futures.max_open_positions",
        max_open_positions,
        &new.max_open_positions,
    );
    check_le(
        violations,
        "risk.futures.btc_spike_15m_pct",
        btc_spike_15m_pct,
        &new.btc_spike_15m_pct,
    );
    check_le(
        violations,
        "risk.futures.btc_spike_60m_pct",
        btc_spike_60m_pct,
        &new.btc_spike_60m_pct,
    );
}

fn check_venues(old: &VenuesConfig, new: &VenuesConfig, violations: &mut Vec<LoosenViolation>) {
    let VenuesConfig {
        default_active,
        allow_switch,
        switch_cooldown_s,
        max_switches_per_day,
        on_open_positions_default,
        cross_venue_same_symbol,
        spot,
        futures,
    } = old;
    check_immutable(
        violations,
        "venues.default_active",
        default_active,
        &new.default_active,
    );
    check_disable_only(
        violations,
        "venues.allow_switch",
        *allow_switch,
        new.allow_switch,
    );
    check_ge(
        violations,
        "venues.switch_cooldown_s",
        switch_cooldown_s,
        &new.switch_cooldown_s,
    );
    check_le(
        violations,
        "venues.max_switches_per_day",
        max_switches_per_day,
        &new.max_switches_per_day,
    );
    check_immutable(
        violations,
        "venues.on_open_positions_default",
        on_open_positions_default,
        &new.on_open_positions_default,
    );
    if *cross_venue_same_symbol == CrossVenueSameSymbolPolicy::BlockOpposite
        && new.cross_venue_same_symbol == CrossVenueSameSymbolPolicy::Allow
    {
        violations.push(LoosenViolation {
            field: "venues.cross_venue_same_symbol",
            old: "BlockOpposite".to_owned(),
            new: "Allow".to_owned(),
            reason: "cannot relax cross-venue opposite-direction block at runtime",
        });
    }
    check_spot_venue(spot, &new.spot, violations);
    check_futures_venue(futures, &new.futures, violations);
}

fn check_spot_venue(
    old: &SpotVenueConfig,
    new: &SpotVenueConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let SpotVenueConfig {
        enabled,
        api_key_file,
        private_key_path,
        capital_allocation_usd,
        live_confirmation_file,
    } = old;
    check_disable_only(violations, "venues.spot.enabled", *enabled, new.enabled);
    check_immutable(
        violations,
        "venues.spot.api_key_file",
        api_key_file,
        &new.api_key_file,
    );
    check_immutable(
        violations,
        "venues.spot.private_key_path",
        private_key_path,
        &new.private_key_path,
    );
    check_capital_cap(
        violations,
        "venues.spot.capital_allocation_usd",
        *capital_allocation_usd,
        new.capital_allocation_usd,
    );
    check_immutable(
        violations,
        "venues.spot.live_confirmation_file",
        live_confirmation_file,
        &new.live_confirmation_file,
    );
}

fn check_futures_venue(
    old: &FuturesVenueConfig,
    new: &FuturesVenueConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let FuturesVenueConfig {
        enabled,
        api_key_file,
        private_key_path,
        capital_allocation_usd,
        live_confirmation_file,
        allow_short,
        position_mode,
        margin_mode,
        max_leverage,
        min_quote_volume_24h_usd,
        min_listing_age_days,
    } = old;
    check_disable_only(violations, "venues.futures.enabled", *enabled, new.enabled);
    check_immutable(
        violations,
        "venues.futures.api_key_file",
        api_key_file,
        &new.api_key_file,
    );
    check_immutable(
        violations,
        "venues.futures.private_key_path",
        private_key_path,
        &new.private_key_path,
    );
    check_capital_cap(
        violations,
        "venues.futures.capital_allocation_usd",
        *capital_allocation_usd,
        new.capital_allocation_usd,
    );
    check_immutable(
        violations,
        "venues.futures.live_confirmation_file",
        live_confirmation_file,
        &new.live_confirmation_file,
    );
    check_disable_only(
        violations,
        "venues.futures.allow_short",
        *allow_short,
        new.allow_short,
    );
    check_immutable(
        violations,
        "venues.futures.position_mode",
        position_mode,
        &new.position_mode,
    );
    check_immutable(
        violations,
        "venues.futures.margin_mode",
        margin_mode,
        &new.margin_mode,
    );
    check_le(
        violations,
        "venues.futures.max_leverage",
        max_leverage,
        &new.max_leverage,
    );
    check_ge(
        violations,
        "venues.futures.min_quote_volume_24h_usd",
        min_quote_volume_24h_usd,
        &new.min_quote_volume_24h_usd,
    );
    check_ge(
        violations,
        "venues.futures.min_listing_age_days",
        min_listing_age_days,
        &new.min_listing_age_days,
    );
}

fn check_ipc_and_storage(
    old_ipc: &IpcConfig,
    new_ipc: &IpcConfig,
    old_st: &StorageConfig,
    new_st: &StorageConfig,
    violations: &mut Vec<LoosenViolation>,
) {
    let IpcConfig {
        uds_path,
        web_bind,
        web_token_file,
        web_readonly_token_file,
        telemetry_hz,
    } = old_ipc;
    check_immutable(violations, "ipc.uds_path", uds_path, &new_ipc.uds_path);
    check_immutable(violations, "ipc.web_bind", web_bind, &new_ipc.web_bind);
    check_immutable(
        violations,
        "ipc.web_token_file",
        web_token_file,
        &new_ipc.web_token_file,
    );
    check_immutable(
        violations,
        "ipc.web_readonly_token_file",
        web_readonly_token_file,
        &new_ipc.web_readonly_token_file,
    );
    check_immutable(
        violations,
        "ipc.telemetry_hz",
        telemetry_hz,
        &new_ipc.telemetry_hz,
    );

    let StorageConfig {
        db_path,
        backup_dir,
    } = old_st;
    check_immutable(violations, "storage.db_path", db_path, &new_st.db_path);
    check_immutable(
        violations,
        "storage.backup_dir",
        backup_dir,
        &new_st.backup_dir,
    );
}

/// Verifies that transitioning from `old` to `new` at runtime only tightens risk or leaves fields unchanged.
///
/// # Errors
/// Returns [`LoosenError::InvalidTarget`] if `new` fails [`BotConfig::validate`], or
/// [`LoosenError::Loosened`] listing every field that would loosen risk or mutate an immutable setting.
pub fn is_tightening(old: &BotConfig, new: &BotConfig) -> Result<(), LoosenError> {
    new.validate()?;

    let BotConfig {
        general,
        safety,
        binance,
        universe,
        strategy,
        exit,
        risk,
        venues,
        ipc,
        storage,
    } = old;

    let StrategyConfig {
        trend_pullback,
        breakout,
        gainers_momentum,
        vwap_reversion,
    } = strategy;

    let mut violations = Vec::new();
    check_general_safety_binance(
        general,
        &new.general,
        safety,
        &new.safety,
        binance,
        &new.binance,
        &mut violations,
    );
    check_universe(universe, &new.universe, &mut violations);
    check_strategy_trend_and_breakout(
        trend_pullback,
        &new.strategy.trend_pullback,
        breakout,
        &new.strategy.breakout,
        &mut violations,
    );
    check_strategy_gainers_and_vwap(
        gainers_momentum,
        &new.strategy.gainers_momentum,
        vwap_reversion,
        &new.strategy.vwap_reversion,
        &mut violations,
    );
    check_exit(exit, &new.exit, &mut violations);
    check_risk(risk, &new.risk, &mut violations);
    check_venues(venues, &new.venues, &mut violations);
    check_ipc_and_storage(ipc, &new.ipc, storage, &new.storage, &mut violations);

    if violations.is_empty() {
        Ok(())
    } else {
        Err(LoosenError::Loosened(violations))
    }
}
