use bot_config::{BotConfig, ConfigError, MarginMode, PositionMode, load_from_toml_str};
use bot_types::{TradingMode, Venue};
use rust_decimal::Decimal;
use std::str::FromStr;

type ConfigCheck = (&'static str, fn(&mut BotConfig));

#[allow(clippy::expect_used)]
fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

#[allow(clippy::expect_used, clippy::panic)]
fn assert_invalid_field(cfg: &BotConfig, expected_field: &str) {
    let err = cfg.validate().expect_err("expected validation failure");
    match err {
        ConfigError::Validation(vs) => {
            assert!(
                vs.iter().any(|v| v.field == expected_field),
                "expected violation on `{expected_field}`, got {vs:?}"
            );
        }
        other => panic!("expected ConfigError::Validation, got {other:?}"),
    }
}

fn run_invalid_checks(checks: &[ConfigCheck]) {
    for &(field, mutate) in checks {
        let mut cfg = BotConfig::default();
        mutate(&mut cfg);
        assert_invalid_field(&cfg, field);
    }
}

#[test]
fn default_and_example_config_pass_validation() {
    assert!(BotConfig::default().validate().is_ok());
    let example = include_str!("../../../packaging/bot.toml.example");
    assert!(load_from_toml_str(example).is_ok());
}

#[test]
fn spot_risk_out_of_range_parameters_are_rejected_without_clamping() {
    let checks: &[ConfigCheck] = &[
        ("risk.risk_per_trade_pct", |c| {
            c.risk.risk_per_trade_pct = dec("0.09");
        }),
        ("risk.risk_per_trade_pct", |c| {
            c.risk.risk_per_trade_pct = dec("1.01");
        }),
        ("risk.max_open_positions", |c| {
            c.risk.max_open_positions = 0;
        }),
        ("risk.max_open_positions", |c| {
            c.risk.max_open_positions = 9;
        }),
        ("risk.portfolio_heat_max_pct", |c| {
            c.risk.portfolio_heat_max_pct = dec("0.4");
        }),
        ("risk.portfolio_heat_max_pct", |c| {
            c.risk.portfolio_heat_max_pct = dec("4.1");
        }),
        ("risk.max_notional_per_position_pct", |c| {
            c.risk.max_notional_per_position_pct = dec("4");
        }),
        ("risk.max_notional_per_position_pct", |c| {
            c.risk.max_notional_per_position_pct = dec("36");
        }),
        ("risk.max_total_exposure_pct", |c| {
            c.risk.max_total_exposure_pct = dec("19");
        }),
        ("risk.max_total_exposure_pct", |c| {
            c.risk.max_total_exposure_pct = dec("81");
        }),
        ("risk.max_same_direction_correlated", |c| {
            c.risk.max_same_direction_correlated = 0;
        }),
        ("risk.max_same_direction_correlated", |c| {
            c.risk.max_same_direction_correlated = 6;
        }),
        ("risk.daily_loss_halt_pct", |c| {
            c.risk.daily_loss_halt_pct = dec("2.0");
        }),
        ("risk.daily_loss_halt_pct", |c| {
            c.risk.daily_loss_halt_pct = dec("-0.4");
        }),
        ("risk.daily_loss_halt_pct", |c| {
            c.risk.daily_loss_halt_pct = dec("-4.1");
        }),
        ("risk.weekly_loss_halt_pct", |c| {
            c.risk.weekly_loss_halt_pct = dec("-1.5");
        }),
        ("risk.drawdown_kill_pct", |c| {
            c.risk.drawdown_kill_pct = dec("-4.9");
        }),
        ("risk.drawdown_kill_pct", |c| {
            c.risk.drawdown_kill_pct = dec("-20.1");
        }),
        ("risk.max_entry_slippage_bps", |c| {
            c.risk.max_entry_slippage_bps = dec("1");
        }),
        ("risk.max_entry_slippage_bps", |c| {
            c.risk.max_entry_slippage_bps = dec("26");
        }),
        ("risk.max_orders_per_day", |c| {
            c.risk.max_orders_per_day = 0;
        }),
    ];
    run_invalid_checks(checks);
}

#[test]
fn futures_risk_out_of_range_parameters_are_rejected() {
    let checks: &[ConfigCheck] = &[
        ("risk.futures.risk_per_trade_pct", |c| {
            c.risk.futures.risk_per_trade_pct = dec("0.09");
        }),
        ("risk.futures.risk_per_trade_pct", |c| {
            c.risk.futures.risk_per_trade_pct = dec("0.76");
        }),
        ("risk.futures.short_size_multiplier", |c| {
            c.risk.futures.short_size_multiplier = dec("0.24");
        }),
        ("risk.futures.short_size_multiplier", |c| {
            c.risk.futures.short_size_multiplier = dec("1.01");
        }),
        ("risk.futures.max_effective_leverage", |c| {
            c.risk.futures.max_effective_leverage = dec("0.4");
        }),
        ("risk.futures.max_effective_leverage", |c| {
            c.risk.futures.max_effective_leverage = dec("3.1");
        }),
        ("risk.futures.liq_buffer_mult", |c| {
            c.risk.futures.liq_buffer_mult = dec("1.9");
        }),
        ("risk.futures.liq_buffer_mult", |c| {
            c.risk.futures.liq_buffer_mult = dec("6.1");
        }),
        ("risk.futures.liq_emergency_mult", |c| {
            c.risk.futures.liq_emergency_mult = dec("0.9");
        }),
        ("risk.futures.liq_emergency_mult", |c| {
            c.risk.futures.liq_emergency_mult = dec("2.1");
        }),
        ("risk.futures.funding_adverse_block_pct", |c| {
            c.risk.futures.funding_adverse_block_pct = dec("0.005");
        }),
        ("risk.futures.funding_adverse_block_pct", |c| {
            c.risk.futures.funding_adverse_block_pct = dec("0.11");
        }),
        ("risk.futures.mark_last_divergence_pct", |c| {
            c.risk.futures.mark_last_divergence_pct = dec("0.19");
        }),
        ("risk.futures.mark_last_divergence_pct", |c| {
            c.risk.futures.mark_last_divergence_pct = dec("1.01");
        }),
        ("risk.futures.max_open_positions", |c| {
            c.risk.futures.max_open_positions = 0;
        }),
        ("risk.futures.max_open_positions", |c| {
            c.risk.futures.max_open_positions = 7;
        }),
    ];
    run_invalid_checks(checks);
}

#[test]
fn venue_and_cross_field_safety_rules_are_enforced() {
    let checks: &[ConfigCheck] = &[
        ("venues.futures.max_leverage", |c| {
            c.venues.futures.max_leverage = 0;
        }),
        ("venues.futures.max_leverage", |c| {
            c.venues.futures.max_leverage = 6;
        }),
        ("venues.futures.margin_mode", |c| {
            c.venues.futures.margin_mode = MarginMode::Cross;
        }),
        ("venues.futures.position_mode", |c| {
            c.venues.futures.position_mode = PositionMode::Hedge;
        }),
        ("venues.default_active", |c| {
            c.venues.default_active = Venue::UsdsFutures;
        }),
        ("venues.spot.live_confirmation_file", |c| {
            c.general.mode = TradingMode::Live;
        }),
        ("risk.futures.max_effective_leverage", |c| {
            c.venues.futures.max_leverage = 1;
            c.risk.futures.max_effective_leverage = dec("2.0");
        }),
        ("risk.futures.liq_emergency_mult", |c| {
            c.risk.futures.liq_buffer_mult = dec("2.0");
            c.risk.futures.liq_emergency_mult = dec("2.0");
        }),
    ];
    run_invalid_checks(checks);
}

#[test]
fn loader_rejects_invalid_toml_values() {
    let err = load_from_toml_str("[risk]\nrisk_per_trade_pct = 5.0\n")
        .expect_err("out-of-range TOML must fail at load");
    assert!(matches!(err, ConfigError::Validation(_)));
}
