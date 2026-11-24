#![allow(clippy::expect_used)]

use std::str::FromStr;

use bot_marketdata::BookStatus;
use bot_marketdata::futures::{
    FuturesDepthApplyResult, FuturesDepthDelta, FuturesOrderBookSync, parse_futures_mark_price,
};
use bot_types::{Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;

#[test]
fn parses_futures_mark_price_stream_payload() {
    let raw = r#"{
        "e": "markPriceUpdate",
        "E": 1700000001000,
        "s": "BTCUSDT",
        "p": "50123.45000000",
        "i": "50120.10000000",
        "r": "0.00010000",
        "T": 1700006400000
    }"#;
    let tick = parse_futures_mark_price(raw).expect("parse markPriceUpdate");
    assert_eq!(tick.venue, Venue::UsdsFutures);
    assert_eq!(tick.symbol, Symbol("BTCUSDT".into()));
    assert_eq!(
        tick.mark_price,
        Price(Decimal::from_str("50123.45000000").expect("dec"))
    );
    assert_eq!(
        tick.funding_rate,
        Decimal::from_str("0.00010000").expect("dec")
    );
    assert_eq!(tick.next_funding_time_ms, 1_700_006_400_000);
}

#[test]
fn futures_orderbook_syncs_via_pu_chain_and_invalidates_on_gap() {
    let sym = Symbol("BTCUSDT".into());
    let mut book = FuturesOrderBookSync::new(sym.clone());
    assert_eq!(book.status(), BookStatus::Invalid);

    book.apply_snapshot(
        100,
        &[(Price(Decimal::from(50000)), Qty(Decimal::from(2)))],
        &[(Price(Decimal::from(50010)), Qty(Decimal::from(3)))],
    );
    assert_eq!(book.status(), BookStatus::Synced);

    // First post-snapshot delta bridges last_update_id=100 (U <= 100 <= u)
    let d1 = FuturesDepthDelta {
        symbol: sym.clone(),
        first_update_id: 99,
        final_update_id: 102,
        prev_final_update_id: 98,
        bids: vec![(Price(Decimal::from(50005)), Qty(Decimal::from(1)))],
        asks: vec![],
    };
    assert_eq!(book.apply_delta(&d1), FuturesDepthApplyResult::Applied);
    assert_eq!(book.best_bid(), Some(Price(Decimal::from(50005))));

    // Next delta chains strictly via pu == 102
    let d2 = FuturesDepthDelta {
        symbol: sym.clone(),
        first_update_id: 103,
        final_update_id: 105,
        prev_final_update_id: 102,
        bids: vec![],
        asks: vec![(Price(Decimal::from(50008)), Qty(Decimal::from(4)))],
    };
    assert_eq!(book.apply_delta(&d2), FuturesDepthApplyResult::Applied);
    assert_eq!(book.best_ask(), Some(Price(Decimal::from(50008))));

    // Gap: pu=104 != last_u=105 -> Invalid!
    let gap = FuturesDepthDelta {
        symbol: sym,
        first_update_id: 106,
        final_update_id: 108,
        prev_final_update_id: 104,
        bids: vec![],
        asks: vec![],
    };
    assert_eq!(
        book.apply_delta(&gap),
        FuturesDepthApplyResult::GapDetectedNeedsResync
    );
    assert_eq!(book.status(), BookStatus::Invalid);
}
