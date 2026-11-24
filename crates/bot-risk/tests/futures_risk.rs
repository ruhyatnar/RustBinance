#![allow(clippy::expect_used)]

use std::str::FromStr;

use bot_risk::futures::{
    FuturesGateInput, FuturesGateRejection, FuturesRiskConfig, check_futures_entry_gates,
    estimate_isolated_liquidation_price,
};
use bot_types::{Price, Side};
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

fn default_cfg() -> FuturesRiskConfig {
    FuturesRiskConfig {
        max_adverse_funding_rate: dec("0.0005"),
        funding_blackout_before_ms: 300_000,
        min_liq_buffer_pct: dec("0.02"),
        max_mark_last_divergence_pct: dec("0.005"),
    }
}

fn base_long_input() -> FuturesGateInput {
    FuturesGateInput {
        side: Side::Buy,
        entry_price: Price(dec("50000")),
        stop_loss_price: Price(dec("48500")),
        leverage: 3,
        maint_margin_rate: dec("0.004"),
        mark_price: Price(dec("50000")),
        last_price: Price(dec("50010")),
        funding_rate: dec("0.0001"),
        now_ms: 1_000_000,
        next_funding_time_ms: 2_000_000,
    }
}

#[test]
fn accepts_healthy_long_and_short_entries() {
    let cfg = default_cfg();
    let long_liq = check_futures_entry_gates(&base_long_input(), &cfg).expect("healthy long");
    assert!(long_liq.0 < dec("48500"));

    let mut short = base_long_input();
    short.side = Side::Sell;
    short.stop_loss_price = Price(dec("51500"));
    short.funding_rate = dec("0.0010"); // positive funding favors Short!
    let short_liq = check_futures_entry_gates(&short, &cfg).expect("healthy short");
    assert!(short_liq.0 > dec("51500"));
}

#[test]
fn rejects_adverse_funding_and_blackout_window() {
    let cfg = default_cfg();
    let mut bad_funding = base_long_input();
    bad_funding.funding_rate = dec("0.0010");
    assert!(matches!(
        check_futures_entry_gates(&bad_funding, &cfg),
        Err(FuturesGateRejection::AdverseFundingRate { .. })
    ));

    let mut blackout = base_long_input();
    blackout.next_funding_time_ms = blackout.now_ms + 60_000;
    assert!(matches!(
        check_futures_entry_gates(&blackout, &cfg),
        Err(FuturesGateRejection::FundingBlackout { .. })
    ));
}

#[test]
fn rejects_insufficient_liquidation_buffer_and_mark_last_divergence() {
    let cfg = default_cfg();

    // High leverage (20x -> liq ~47700) with SL=48000 violates 2% (1000) buffer
    let mut near_liq = base_long_input();
    near_liq.leverage = 20;
    near_liq.stop_loss_price = Price(dec("48000"));
    assert!(matches!(
        check_futures_entry_gates(&near_liq, &cfg),
        Err(FuturesGateRejection::InsufficientLiquidationBuffer { .. })
    ));

    // Mark vs Last diverges by 1% > 0.5%
    let mut div = base_long_input();
    div.last_price = Price(dec("50500"));
    assert!(matches!(
        check_futures_entry_gates(&div, &cfg),
        Err(FuturesGateRejection::MarkLastDivergence { .. })
    ));
}

#[test]
fn isolated_liquidation_price_is_symmetric_for_long_and_short() {
    let long_liq =
        estimate_isolated_liquidation_price(Side::Buy, Price(dec("100")), 5, dec("0.005"))
            .expect("long liq");
    let short_liq =
        estimate_isolated_liquidation_price(Side::Sell, Price(dec("100")), 5, dec("0.005"))
            .expect("short liq");
    assert_eq!(long_liq.0, dec("80.500"));
    assert_eq!(short_liq.0, dec("119.500"));
}
