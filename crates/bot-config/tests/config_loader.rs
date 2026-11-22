use bot_config::{
    BotConfig, ConfigError, ConfigSourcePaths, load_from_paths, load_from_paths_with_env_pairs,
    load_from_toml_str,
};
use bot_types::{TradingMode, Venue};
use rust_decimal::Decimal;
use std::fs;
use std::str::FromStr;

#[allow(clippy::expect_used)]
fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

#[test]
fn example_bot_toml_loads_matches_defaults_and_roundtrips_json_decimals_as_strings() {
    let example = include_str!("../../../packaging/bot.toml.example");
    let cfg = load_from_toml_str(example).expect("example bot.toml must load");
    assert_eq!(cfg.general.mode, TradingMode::Paper);
    assert_eq!(cfg.venues.default_active, Venue::Spot);
    assert_eq!(cfg.risk.risk_per_trade_pct, dec("0.5"));
    assert_eq!(cfg.risk.daily_loss_halt_pct, dec("-2.0"));
    assert_eq!(cfg.risk.futures.risk_per_trade_pct, dec("0.35"));
    assert_eq!(cfg.universe.min_quote_volume_24h_usd, dec("50000000"));
    assert_eq!(cfg, BotConfig::default());

    let val = serde_json::to_value(&cfg).expect("serialize BotConfig");
    assert!(val["risk"]["risk_per_trade_pct"].is_string());
    assert!(val["risk"]["daily_loss_halt_pct"].is_string());
    assert!(val["risk"]["futures"]["liq_buffer_mult"].is_string());
    assert!(val["universe"]["min_quote_volume_24h_usd"].is_string());
    let rt: BotConfig = serde_json::from_value(val).expect("deserialize BotConfig");
    assert_eq!(rt, cfg);
}

#[test]
fn unknown_field_in_any_section_is_rejected() {
    let cases = [
        "[general]\nunknown_key = 1\n",
        "[risk]\ntypo_risk = 0.5\n",
        "[risk.futures]\nunknown_fut = 1\n",
        "[venues.spot]\nsecret_key = \"nope\"\n",
        "[strategy.trend_pullback]\nextra = true\n",
        "[top_level_bogus]\nx = 1\n",
    ];
    for case in cases {
        let err = load_from_toml_str(case).expect_err("unknown field must fail");
        assert!(
            matches!(err, ConfigError::Load(_)),
            "case={case} err={err:?}"
        );
    }
}

#[test]
fn layered_loading_respects_defaults_base_sorted_conf_d_and_env_precedence() {
    let dir = tempfile::tempdir().expect("tempdir");
    let base = dir.path().join("bot.toml");
    let conf_d = dir.path().join("conf.d");
    fs::create_dir(&conf_d).expect("mkdir conf.d");

    fs::write(
        &base,
        "[risk]\nrisk_per_trade_pct = 0.4\nmax_open_positions = 3\n",
    )
    .expect("write base");
    fs::write(
        conf_d.join("10-first.toml"),
        "[risk]\nrisk_per_trade_pct = 0.3\n",
    )
    .expect("write 10");
    fs::write(
        conf_d.join("20-second.toml"),
        "[risk]\nrisk_per_trade_pct = 0.25\n",
    )
    .expect("write 20");
    fs::write(
        conf_d.join("99-ignored.bak"),
        "[risk]\nrisk_per_trade_pct = 0.99\n",
    )
    .expect("write bak");

    let paths = ConfigSourcePaths {
        base_file: Some(base),
        conf_d_dir: Some(conf_d),
        env_prefix: None,
    };
    let cfg_no_env = load_from_paths(&paths).expect("layered load without env");
    assert_eq!(cfg_no_env.risk.risk_per_trade_pct, dec("0.25"));
    assert_eq!(cfg_no_env.risk.max_open_positions, 3);
    assert_eq!(cfg_no_env.risk.portfolio_heat_max_pct, dec("2.0"));

    let cfg_with_env =
        load_from_paths_with_env_pairs(&paths, &[("RISK__RISK_PER_TRADE_PCT", "0.2")])
            .expect("layered load with env override");
    assert_eq!(cfg_with_env.risk.risk_per_trade_pct, dec("0.2"));
    assert_eq!(cfg_with_env.risk.max_open_positions, 3);
    assert_eq!(cfg_with_env.risk.portfolio_heat_max_pct, dec("2.0"));
}

#[test]
fn env_override_parses_decimal_and_nested_futures_keys() {
    let cfg = load_from_paths_with_env_pairs(
        &ConfigSourcePaths {
            base_file: None,
            conf_d_dir: None,
            env_prefix: None,
        },
        &[
            ("RISK__RISK_PER_TRADE_PCT", "0.2"),
            ("RISK__FUTURES__RISK_PER_TRADE_PCT", "0.25"),
            ("VENUES__SPOT__CAPITAL_ALLOCATION_USD", "1500.50"),
        ],
    )
    .expect("env override load");
    assert_eq!(cfg.risk.risk_per_trade_pct, dec("0.2"));
    assert_eq!(cfg.risk.futures.risk_per_trade_pct, dec("0.25"));
    assert_eq!(cfg.venues.spot.capital_allocation_usd, dec("1500.50"));
}

#[test]
fn missing_optional_paths_use_defaults_while_malformed_toml_fails() {
    let dir = tempfile::tempdir().expect("tempdir");
    let cfg = load_from_paths(&ConfigSourcePaths {
        base_file: Some(dir.path().join("nonexistent.toml")),
        conf_d_dir: Some(dir.path().join("no_conf_d")),
        env_prefix: None,
    })
    .expect("missing optional paths fall back to defaults");
    assert_eq!(cfg, BotConfig::default());

    let bad = dir.path().join("bad.toml");
    fs::write(&bad, "[risk\nbroken = ").expect("write bad toml");
    let err = load_from_paths(&ConfigSourcePaths {
        base_file: Some(bad),
        conf_d_dir: None,
        env_prefix: None,
    })
    .expect_err("malformed toml must fail");
    assert!(matches!(err, ConfigError::Load(_)));
}
