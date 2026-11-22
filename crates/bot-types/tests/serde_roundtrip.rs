use bot_types::{
    Direction, EngineMode, InstrumentKey, Kline, Order, OrderKind, OrderPurpose, OrderStatus,
    Position, PositionState, Price, ProtectionScheme, Qty, Side, StrategyId, Symbol, TradingMode,
    Venue, VenueCaps, VenueState,
};
use rust_decimal::Decimal;
use std::collections::HashSet;
use std::str::FromStr;
use time::OffsetDateTime;
use uuid::Uuid;

#[allow(clippy::expect_used)]
fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal literal")
}

#[test]
fn all_domain_types_roundtrip_and_serialize_decimals_as_strings() {
    let now = OffsetDateTime::from_unix_timestamp(1_763_800_000).expect("valid timestamp");
    let order = Order {
        venue: Venue::Spot,
        client_id: "S_tp_01".to_owned(),
        exchange_id: Some(42),
        symbol: Symbol("BTCUSDT".to_owned()),
        side: Side::Buy,
        kind: OrderKind::LimitMaker,
        price: Some(Price(dec("65432.10"))),
        qty: Qty(dec("0.015")),
        filled_qty: Qty(dec("0.005")),
        avg_fill_price: Some(Price(dec("65432.00"))),
        status: OrderStatus::PartiallyFilled,
        position_id: Some(Uuid::nil()),
        purpose: OrderPurpose::Entry,
        created_at: now,
        updated_at: now,
    };
    let order_val = serde_json::to_value(&order).expect("serialize order");
    assert!(
        order_val["price"].is_string(),
        "Price must serialize as JSON string"
    );
    assert_eq!(order_val["price"], "65432.10");
    assert!(
        order_val["qty"].is_string(),
        "Qty must serialize as JSON string"
    );
    assert_eq!(order_val["venue"], "spot");
    let order_rt: Order = serde_json::from_value(order_val).expect("deserialize order");
    assert_eq!(order_rt, order);

    let position = Position {
        id: Uuid::nil(),
        symbol: Symbol("ETHUSDT".to_owned()),
        venue: Venue::UsdsFutures,
        direction: Direction::Short,
        strategy: StrategyId::TrendPullback,
        entry_price: Price(dec("3200.50")),
        qty_initial: Qty(dec("1.500")),
        qty_open: Qty(dec("0.750")),
        r_distance: dec("45.25"),
        leverage: Some(3),
        liq_price: Some(Price(dec("4200.00"))),
        stop_price: Price(dec("3245.75")),
        tp1_price: Price(dec("3132.62")),
        tp2_price: Some(Price(dec("3064.75"))),
        best_price: Price(dec("3150.00")),
        tp1_done: true,
        breakeven_done: true,
        fees_paid_quote: dec("1.92"),
        realized_pnl_quote: dec("50.91"),
        state: PositionState::OpenProtected,
        opened_at: now,
    };
    let pos_val = serde_json::to_value(&position).expect("serialize position");
    assert!(pos_val["entry_price"].is_string());
    assert!(pos_val["r_distance"].is_string());
    assert!(pos_val["fees_paid_quote"].is_string());
    assert!(pos_val["realized_pnl_quote"].is_string());
    assert_eq!(pos_val["venue"], "futures");
    let pos_rt: Position = serde_json::from_value(pos_val).expect("deserialize position");
    assert_eq!(pos_rt, position);

    let kline = Kline {
        open_time_ms: 1_763_800_000_000,
        close_time_ms: 1_763_803_599_999,
        open: 65000.0,
        high: 65500.5,
        low: 64900.25,
        close: 65432.1,
        volume: 1234.5,
        quote_volume: 80_000_000.0,
        trades: 12_345,
        taker_buy_base: 600.25,
        is_closed: true,
    };
    let kline_rt: Kline =
        serde_json::from_str(&serde_json::to_string(&kline).expect("ser kline")).expect("de kline");
    assert_eq!(kline_rt, kline);

    let caps = VenueCaps::usds_futures(3);
    assert!(caps.can_short && caps.has_funding && caps.max_leverage == 3);
    assert_eq!(caps.protection, ProtectionScheme::AlgoReduceOnly);
    let caps_rt: VenueCaps =
        serde_json::from_str(&serde_json::to_string(&caps).expect("ser caps")).expect("de caps");
    assert_eq!(caps_rt, caps);

    assert_eq!(TradingMode::default(), TradingMode::Paper);
    assert_eq!(
        serde_json::from_str::<VenueState>("\"Draining\"").expect("de venue state"),
        VenueState::Draining
    );
    assert_eq!(
        serde_json::from_str::<EngineMode>("\"ReduceOnly\"").expect("de engine mode"),
        EngineMode::ReduceOnly
    );
}

#[test]
fn instrument_key_distinguishes_same_symbol_across_venues() {
    let spot = InstrumentKey {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".to_owned()),
    };
    let fut = InstrumentKey {
        venue: Venue::UsdsFutures,
        symbol: Symbol("BTCUSDT".to_owned()),
    };
    assert_ne!(spot, fut);
    let mut set = HashSet::new();
    set.insert(spot.clone());
    set.insert(fut.clone());
    assert_eq!(set.len(), 2);

    let spot_rt: InstrumentKey =
        serde_json::from_str(&serde_json::to_string(&spot).expect("ser key")).expect("de key");
    assert_eq!(spot_rt, spot);
}
