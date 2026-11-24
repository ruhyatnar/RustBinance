#![allow(clippy::expect_used)]

use std::str::FromStr;

use bot_exchange::futures::{
    FuturesAccountSetup, FuturesAdapter, FuturesAdapterConfig, FuturesMarginType, FuturesUserEvent,
    build_place_algo_order_params, parse_futures_exchange_info, parse_futures_user_event,
};
use bot_exchange::{AlgoOrderType, PlaceAlgoOrderRequest, VenueAdapter, WorkingType};
use bot_types::{Price, Side, Symbol, Venue};
use rust_decimal::Decimal;

fn test_cfg() -> FuturesAdapterConfig {
    FuturesAdapterConfig {
        rest_base_url: "https://testnet.binancefuture.com".into(),
        ws_api_url: "wss://testnet.binancefuture.com/ws-fapi/v1".into(),
        max_leverage_cap: 5,
    }
}

fn clean_setup() -> FuturesAccountSetup {
    FuturesAccountSetup {
        dual_side_position: false,
        multi_assets_margin: false,
        margin_type: FuturesMarginType::Isolated,
        leverage: 3,
        has_open_positions: false,
    }
}

#[tokio::test]
async fn activate_succeeds_when_flat_isolated_oneway_within_cap() {
    let adapter = FuturesAdapter::new(test_cfg(), clean_setup());
    let report = adapter.activate().await.expect("clean setup must activate");
    assert_eq!(report.venue, Venue::UsdsFutures);
    assert_eq!(adapter.venue(), Venue::UsdsFutures);
    assert!(adapter.caps().can_short);
    assert!(adapter.caps().has_funding);
}

#[tokio::test]
async fn activate_rejects_hedge_mode_multi_assets_cross_and_non_flat() {
    let mut hedge = clean_setup();
    hedge.dual_side_position = true;
    assert!(
        FuturesAdapter::new(test_cfg(), hedge)
            .activate()
            .await
            .is_err()
    );

    let mut multi = clean_setup();
    multi.multi_assets_margin = true;
    assert!(
        FuturesAdapter::new(test_cfg(), multi)
            .activate()
            .await
            .is_err()
    );

    let mut cross = clean_setup();
    cross.margin_type = FuturesMarginType::Crossed;
    assert!(
        FuturesAdapter::new(test_cfg(), cross)
            .activate()
            .await
            .is_err()
    );

    let mut non_flat = clean_setup();
    non_flat.has_open_positions = true;
    assert!(
        FuturesAdapter::new(test_cfg(), non_flat)
            .activate()
            .await
            .is_err()
    );

    let mut over_lev = clean_setup();
    over_lev.leverage = 10;
    assert!(
        FuturesAdapter::new(test_cfg(), over_lev)
            .activate()
            .await
            .is_err()
    );
}

#[test]
fn parses_futures_exchange_info_with_notional_filter() {
    let raw = r#"{
        "serverTime": 1700000000000,
        "symbols": [{
            "symbol": "BTCUSDT",
            "baseAsset": "BTC",
            "quoteAsset": "USDT",
            "status": "TRADING",
            "filters": [
                {"filterType": "PRICE_FILTER", "tickSize": "0.10"},
                {"filterType": "LOT_SIZE", "stepSize": "0.001", "minQty": "0.001", "maxQty": "1000"},
                {"filterType": "MIN_NOTIONAL", "notional": "5.0"}
            ]
        }]
    }"#;
    let snap = parse_futures_exchange_info(raw).expect("parse futures exchangeInfo");
    assert_eq!(snap.venue, Venue::UsdsFutures);
    assert_eq!(snap.rules.len(), 1);
    assert_eq!(
        snap.rules[0].min_notional,
        Decimal::from_str("5.0").expect("dec")
    );
    assert_eq!(
        snap.rules[0].tick_size,
        Decimal::from_str("0.10").expect("dec")
    );
}

#[test]
fn parses_all_futures_user_data_events() {
    let order_ev = r#"{"e":"ORDER_TRADE_UPDATE","E":1700000000001,"o":{"s":"BTCUSDT","c":"b01r1F-000001-ENT","S":"BUY","x":"TRADE","X":"FILLED","i":42,"l":"0.01","z":"0.01","L":"50000.0","rp":"0"}}"#;
    assert!(matches!(
        parse_futures_user_event(order_ev).expect("order event"),
        FuturesUserEvent::OrderTradeUpdate { order_id: 42, .. }
    ));

    let algo_ev = r#"{"e":"ALGO_UPDATE","E":1700000000002,"o":{"s":"BTCUSDT","aid":99,"caid":"b01r1F-000002-SL","X":"NEW"}}"#;
    assert!(matches!(
        parse_futures_user_event(algo_ev).expect("algo event"),
        FuturesUserEvent::AlgoUpdate { algo_id: 99, .. }
    ));

    let acct_ev = r#"{"e":"ACCOUNT_UPDATE","E":1700000000003,"a":{"m":"FUNDING_FEE"}}"#;
    assert!(matches!(
        parse_futures_user_event(acct_ev).expect("acct event"),
        FuturesUserEvent::AccountUpdate { .. }
    ));

    let mc_ev = r#"{"e":"MARGIN_CALL","E":1700000000004,"p":[{"s":"BTCUSDT"}]}"#;
    assert!(matches!(
        parse_futures_user_event(mc_ev).expect("margin call"),
        FuturesUserEvent::MarginCall {
            position_count: 1,
            ..
        }
    ));

    let cfg_ev = r#"{"e":"ACCOUNT_CONFIG_UPDATE","E":1700000000005}"#;
    assert!(matches!(
        parse_futures_user_event(cfg_ev).expect("cfg event"),
        FuturesUserEvent::AccountConfigUpdate { .. }
    ));
}

#[test]
fn algo_order_builder_rejects_close_position_with_quantity() {
    let req = PlaceAlgoOrderRequest {
        symbol: Symbol("BTCUSDT".into()),
        side: Side::Sell,
        order_type: AlgoOrderType::StopMarket,
        client_algo_id: "b01r1F-000010-SL".into(),
        quantity: None,
        trigger_price: Some(Price(Decimal::from(49000))),
        activate_price: None,
        callback_rate: None,
        working_type: WorkingType::MarkPrice,
        close_position: true,
        reduce_only: false,
        price_protect: true,
    };
    let params = build_place_algo_order_params(&req).expect("valid closePosition SL");
    assert_eq!(params["type"], "STOP_MARKET");
    assert_eq!(params["workingType"], "MARK_PRICE");
    assert_eq!(params["closePosition"], "true");
}
