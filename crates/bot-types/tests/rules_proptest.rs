use bot_types::{
    InstrumentRules, Price, Qty, RulesError, Symbol, Venue, ceil_to_tick, floor_to_step,
    floor_to_tick, round_to_tick,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use std::str::FromStr;

#[allow(clippy::expect_used)]
fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

fn btc_rules() -> InstrumentRules {
    InstrumentRules {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".to_owned()),
        tick_size: dec("0.01"),
        step_size: dec("0.0001"),
        min_qty: dec("0.001"),
        max_qty: dec("1000"),
        min_notional: dec("10"),
        status_trading: true,
        base_precision: 8,
        quote_precision: 8,
    }
}

#[test]
fn degenerate_and_overflow_inputs_return_errors_without_panicking() {
    assert_eq!(
        floor_to_step(Qty(dec("1.5")), Decimal::ZERO),
        Err(RulesError::InvalidStepSize(Decimal::ZERO))
    );
    assert_eq!(
        floor_to_step(Qty(dec("1.5")), dec("-0.01")),
        Err(RulesError::InvalidStepSize(dec("-0.01")))
    );
    assert_eq!(
        floor_to_step(Qty(dec("-1.0")), dec("0.01")),
        Err(RulesError::NegativeValue(dec("-1.0")))
    );
    assert_eq!(
        round_to_tick(Price(dec("100")), Decimal::ZERO),
        Err(RulesError::InvalidTickSize(Decimal::ZERO))
    );
    assert_eq!(
        round_to_tick(Price(dec("-1")), dec("0.01")),
        Err(RulesError::NegativeValue(dec("-1")))
    );
    let res = floor_to_step(Qty(Decimal::MAX), dec("0.00000001"));
    assert!(matches!(res, Ok(_) | Err(RulesError::ArithmeticOverflow)));
    let ceil_res = ceil_to_tick(Price(Decimal::MAX), dec("0.01"));
    assert!(matches!(
        ceil_res,
        Ok(_) | Err(RulesError::ArithmeticOverflow)
    ));
}

#[test]
fn validate_order_enforces_trading_status_step_tick_qty_and_min_notional() {
    let rules = btc_rules();
    assert!(rules.validate_rules().is_ok());
    assert!(
        rules
            .validate_order(Price(dec("50000.01")), Qty(dec("0.001")))
            .is_ok()
    );

    assert!(matches!(
        rules.validate_order(Price(dec("5000.00")), Qty(dec("0.001"))),
        Err(RulesError::NotionalBelowMin { .. })
    ));
    assert!(matches!(
        rules.validate_order(Price(dec("50000.00")), Qty(dec("0.0005"))),
        Err(RulesError::QtyBelowMin { .. })
    ));
    assert!(matches!(
        rules.validate_order(Price(dec("50000.00")), Qty(dec("1000.0001"))),
        Err(RulesError::QtyAboveMax { .. })
    ));
    assert!(matches!(
        rules.validate_order(Price(dec("50000.00")), Qty(dec("0.00105"))),
        Err(RulesError::QtyNotStepMultiple { .. })
    ));
    assert!(matches!(
        rules.validate_order(Price(dec("50000.005")), Qty(dec("0.001"))),
        Err(RulesError::PriceNotTickMultiple { .. })
    ));
    let mut halted = rules.clone();
    halted.status_trading = false;
    assert!(matches!(
        halted.validate_order(Price(dec("50000.00")), Qty(dec("0.001"))),
        Err(RulesError::NotTrading { .. })
    ));

    let val = serde_json::to_value(&rules).expect("ser rules");
    assert!(val["tick_size"].is_string());
    assert!(val["min_notional"].is_string());
    let rt: InstrumentRules = serde_json::from_value(val).expect("de rules");
    assert_eq!(rt, rules);
}

#[test]
fn round_to_tick_midpoint_rounds_half_up() {
    assert_eq!(
        round_to_tick(Price(dec("10.005")), dec("0.01")),
        Ok(Price(dec("10.01")))
    );
    assert_eq!(
        round_to_tick(Price(dec("10.004")), dec("0.01")),
        Ok(Price(dec("10.00")))
    );
    assert_eq!(
        floor_to_tick(Price(dec("10.009")), dec("0.01")),
        Ok(Price(dec("10.00")))
    );
    assert_eq!(
        ceil_to_tick(Price(dec("10.001")), dec("0.01")),
        Ok(Price(dec("10.01")))
    );
    assert_eq!(
        ceil_to_tick(Price(dec("10.00")), dec("0.01")),
        Ok(Price(dec("10.00")))
    );
}

proptest! {
    #[test]
    fn prop_floor_to_step_is_multiple_and_le_input(
        raw_units in 0_i64..1_000_000_000_i64,
        raw_scale in 0_u32..8_u32,
        step_mantissa in prop_oneof![Just(1_i64), Just(2_i64), Just(5_i64), Just(25_i64), Just(10_i64)],
        step_scale in 0_u32..6_u32,
    ) {
        let qty = Decimal::new(raw_units, raw_scale);
        let step = Decimal::new(step_mantissa, step_scale);
        let floored = floor_to_step(Qty(qty), step).expect("valid floor");
        prop_assert!(floored.0 <= qty);
        prop_assert!((floored.0 % step).is_zero());
        prop_assert!(qty - floored.0 < step);
    }

    #[test]
    fn prop_round_floor_ceil_to_tick_invariants(
        raw_units in 0_i64..1_000_000_000_i64,
        raw_scale in 0_u32..6_u32,
        tick_mantissa in prop_oneof![Just(1_i64), Just(5_i64), Just(10_i64), Just(25_i64)],
        tick_scale in 0_u32..5_u32,
    ) {
        let price = Decimal::new(raw_units, raw_scale);
        let tick = Decimal::new(tick_mantissa, tick_scale);

        let floored = floor_to_tick(Price(price), tick).expect("valid floor_to_tick");
        prop_assert!(floored.0 <= price);
        prop_assert!((floored.0 % tick).is_zero());
        prop_assert!(price - floored.0 < tick);

        let rounded = round_to_tick(Price(price), tick).expect("valid round_to_tick");
        prop_assert!((rounded.0 % tick).is_zero());
        let diff = (rounded.0 - price).abs();
        prop_assert!(diff * Decimal::from(2) <= tick);

        let ceiled = ceil_to_tick(Price(price), tick).expect("valid ceil_to_tick");
        prop_assert!(ceiled.0 >= price);
        prop_assert!((ceiled.0 % tick).is_zero());
        prop_assert!(ceiled.0 - price < tick);
    }
}
