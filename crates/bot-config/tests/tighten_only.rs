use bot_config::{
    BotConfig, CrossVenueSameSymbolPolicy, LoosenError, ProfitLockMode, is_tightening,
};
use rust_decimal::Decimal;
use std::str::FromStr;

type TightenCheck = (&'static str, fn(&mut BotConfig));

#[allow(clippy::expect_used)]
fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

#[allow(clippy::expect_used, clippy::panic)]
fn assert_loosened(old: &BotConfig, new: &BotConfig, expected_field: &str) {
    let err = is_tightening(old, new).expect_err("expected loosening rejection");
    match err {
        LoosenError::Loosened(vs) => {
            assert!(
                vs.iter().any(|v| v.field == expected_field),
                "expected loosened field `{expected_field}`, got {vs:?}"
            );
        }
        LoosenError::InvalidTarget(err) => {
            panic!("expected LoosenError::Loosened, got InvalidTarget({err:?})")
        }
    }
}

#[test]
fn identical_and_strictly_tighter_configs_are_allowed() {
    let old = BotConfig::default();
    assert!(is_tightening(&old, &old).is_ok());

    let mut tighter = old.clone();
    tighter.risk.risk_per_trade_pct = dec("0.25");
    tighter.risk.max_open_positions = 2;
    tighter.risk.daily_loss_halt_pct = dec("-1.5");
    tighter.risk.drawdown_kill_pct = dec("-8.0");
    tighter.risk.profit_lock_mode = ProfitLockMode::Stop;
    tighter.risk.futures.risk_per_trade_pct = dec("0.2");
    tighter.risk.futures.liq_buffer_mult = dec("4.0");
    tighter.risk.futures.liq_emergency_mult = dec("1.8");
    tighter.venues.spot.capital_allocation_usd = dec("1000");
    tighter.venues.futures.max_leverage = 2;
    tighter.strategy.breakout.enabled = false;
    tighter.universe.exclude_symbols.push("MEMEUSDT".to_owned());
    assert!(is_tightening(&old, &tighter).is_ok());

    let mut tighter2 = tighter.clone();
    tighter2.venues.spot.capital_allocation_usd = dec("500");
    assert!(is_tightening(&tighter, &tighter2).is_ok());
}

#[test]
fn loosening_any_risk_or_futures_parameter_is_rejected() {
    let old = BotConfig::default();
    let cases: &[TightenCheck] = &[
        ("risk.risk_per_trade_pct", |c| {
            c.risk.risk_per_trade_pct = dec("0.75");
        }),
        ("risk.max_open_positions", |c| {
            c.risk.max_open_positions = 5;
        }),
        ("risk.portfolio_heat_max_pct", |c| {
            c.risk.portfolio_heat_max_pct = dec("2.5");
        }),
        ("risk.daily_loss_halt_pct", |c| {
            c.risk.daily_loss_halt_pct = dec("-3.0");
        }),
        ("risk.weekly_loss_halt_pct", |c| {
            c.risk.weekly_loss_halt_pct = dec("-6.0");
        }),
        ("risk.drawdown_kill_pct", |c| {
            c.risk.drawdown_kill_pct = dec("-12.0");
        }),
        ("risk.profit_lock_mode", |c| {
            c.risk.profit_lock_mode = ProfitLockMode::Off;
        }),
        ("risk.futures.risk_per_trade_pct", |c| {
            c.risk.futures.risk_per_trade_pct = dec("0.5");
        }),
        ("risk.futures.liq_buffer_mult", |c| {
            c.risk.futures.liq_buffer_mult = dec("2.5");
        }),
        ("risk.futures.liq_emergency_mult", |c| {
            c.risk.futures.liq_emergency_mult = dec("1.2");
        }),
        ("risk.futures.funding_adverse_block_pct", |c| {
            c.risk.futures.funding_adverse_block_pct = dec("0.05");
        }),
        ("venues.futures.max_leverage", |c| {
            c.venues.futures.max_leverage = 4;
        }),
        ("venues.cross_venue_same_symbol", |c| {
            c.venues.cross_venue_same_symbol = CrossVenueSameSymbolPolicy::Allow;
        }),
        ("strategy.gainers_momentum.shadow_only", |c| {
            c.strategy.gainers_momentum.shadow_only = false;
        }),
        ("safety.kill_flatten", |c| {
            c.safety.kill_flatten = false;
        }),
        ("ipc.web_bind", |c| {
            c.ipc.web_bind = "127.0.0.1:4000".to_owned();
        }),
    ];
    for &(field, mutate) in cases {
        let mut new_cfg = old.clone();
        mutate(&mut new_cfg);
        assert_loosened(&old, &new_cfg, field);
    }
}

#[test]
fn capital_allocation_zero_sentinel_and_exclude_symbols_removal_are_rejected() {
    let mut capped = BotConfig::default();
    capped.venues.spot.capital_allocation_usd = dec("500");
    capped.universe.exclude_symbols = vec!["BADUSDT".to_owned()];

    let mut unbounded = capped.clone();
    unbounded.venues.spot.capital_allocation_usd = Decimal::ZERO;
    assert_loosened(&capped, &unbounded, "venues.spot.capital_allocation_usd");

    let mut larger = capped.clone();
    larger.venues.spot.capital_allocation_usd = dec("800");
    assert_loosened(&capped, &larger, "venues.spot.capital_allocation_usd");

    let mut unexcluded = capped.clone();
    unexcluded.universe.exclude_symbols.clear();
    assert_loosened(&capped, &unexcluded, "universe.exclude_symbols");
}

#[test]
fn tighter_but_out_of_range_target_returns_invalid_target() {
    let old = BotConfig::default();
    let mut invalid = old.clone();
    invalid.risk.risk_per_trade_pct = dec("0.01");
    assert!(matches!(
        is_tightening(&old, &invalid),
        Err(LoosenError::InvalidTarget(_))
    ));
}
