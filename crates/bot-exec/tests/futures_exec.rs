use std::str::FromStr;

use bot_exchange::{AlgoOrderType, ExchangeGateway, PlaceOrderRequest, TimeInForce};
use bot_exec::futures_protection::AlgoProtectionManager;
use bot_exec::paper_futures::{PaperFuturesBroker, PaperFuturesOp, PaperMarkOutcome};
use bot_types::{OrderKind, Price, Qty, Side, Symbol};
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

#[tokio::test]
async fn full_long_lifecycle_entry_sl_tp_be_trail_exit_with_zero_sl_gap() {
    let broker = PaperFuturesBroker::new(dec("10000"), 3, dec("0.004"));
    let mgr = AlgoProtectionManager;
    let sym = Symbol("BTCUSDT".into());

    // 1. Entry Long (LIMIT GTX)
    broker
        .place_order(PlaceOrderRequest {
            symbol: sym.clone(),
            side: Side::Buy,
            kind: OrderKind::Limit,
            client_order_id: "b01r1F-000001-ENT".into(),
            quantity: Some(Qty(dec("0.1"))),
            quote_order_qty: None,
            price: Some(Price(dec("50000"))),
            stop_price: None,
            time_in_force: Some(TimeInForce::Gtx),
        })
        .await
        .expect("entry long");

    // 2. Install SL (closePosition) + TP (reduceOnly)
    let mut prot = mgr
        .install_protection(
            &broker,
            sym.clone(),
            Side::Buy,
            Qty(dec("0.1")),
            Price(dec("49000")),
            Some(Price(dec("53000"))),
            "b01r1F-000002-SL".into(),
            Some("b01r1F-000003-TP".into()),
        )
        .await
        .expect("install protection");
    assert_eq!(broker.active_sl_count(&sym), 1);

    // 3. Move SL to breakeven (make-before-break)
    mgr.move_stop_make_before_break(&broker, &mut prot, Price(dec("50050")), "b01r1F-000004-BE".into())
        .await
        .expect("move SL to BE");
    assert_eq!(broker.active_sl_count(&sym), 1);
    assert_eq!(prot.active_sl_price, Price(dec("50050")));

    // Verify ZERO gap: during move, active_sl_count went 1 -> 2 (PlaceAlgo) -> 1 (CancelAlgo)
    let ops = broker.recorded_ops();
    let be_ops: Vec<_> = ops
        .iter()
        .skip_while(|op| !matches!(op, PaperFuturesOp::PlaceAlgo { algo_id: 4, .. }))
        .collect();
    assert!(matches!(
        be_ops.first(),
        Some(PaperFuturesOp::PlaceAlgo { active_sl_count_after: 2, .. })
    ));
    assert!(matches!(
        be_ops.get(1),
        Some(PaperFuturesOp::CancelAlgo { active_sl_count_after: 1, .. })
    ));

    // 4. Install trailing stop
    mgr.install_trailing_stop(
        &broker,
        &mut prot,
        Qty(dec("0.1")),
        Price(dec("52000")),
        dec("0.5"),
        "b01r1F-000005-TRL".into(),
    )
    .await
    .expect("install trailing");
    assert!(prot.active_trail_algo_id.is_some());

    // 5. Exit via TP trigger
    let out = broker.on_mark_price(&sym, Price(dec("53050")));
    assert!(matches!(
        out,
        PaperMarkOutcome::AlgoTriggered {
            order_type: AlgoOrderType::TakeProfitMarket,
            realized_pnl,
            ..
        } if realized_pnl > Decimal::ZERO
    ));
    assert!(broker.position(&sym).is_none());
}

#[tokio::test]
async fn full_short_lifecycle_and_failed_sl_move_keeps_old_sl_active() {
    let broker = PaperFuturesBroker::new(dec("10000"), 3, dec("0.004"));
    let mgr = AlgoProtectionManager;
    let sym = Symbol("ETHUSDT".into());

    // 1. Entry Short
    broker
        .place_order(PlaceOrderRequest {
            symbol: sym.clone(),
            side: Side::Sell,
            kind: OrderKind::Limit,
            client_order_id: "b01r1F-000010-ENT".into(),
            quantity: Some(Qty(dec("1.0"))),
            quote_order_qty: None,
            price: Some(Price(dec("3000"))),
            stop_price: None,
            time_in_force: Some(TimeInForce::Gtx),
        })
        .await
        .expect("entry short");

    // 2. Install Short SL (above entry) + TP (below entry)
    let mut prot = mgr
        .install_protection(
            &broker,
            sym.clone(),
            Side::Sell,
            Qty(dec("1.0")),
            Price(dec("3100")),
            Some(Price(dec("2700"))),
            "b01r1F-000011-SL".into(),
            Some("b01r1F-000012-TP".into()),
        )
        .await
        .expect("install short protection");
    let old_sl_id = prot.active_sl_algo_id;

    // 3. Inject failure on new SL placement -> old SL MUST remain active!
    broker.inject_next_place_algo_failure();
    assert!(
        mgr.move_stop_make_before_break(
            &broker,
            &mut prot,
            Price(dec("2990")),
            "b01r1F-000013-BE".into()
        )
        .await
        .is_err()
    );
    assert_eq!(prot.active_sl_algo_id, old_sl_id);
    assert_eq!(broker.active_sl_count(&sym), 1);

    // 4. Positive funding pays Short
    let funding = broker.apply_funding(&sym, Price(dec("2900")), dec("0.0001"));
    assert!(funding > Decimal::ZERO);

    // 5. Exit Short via TP
    let out = broker.on_mark_price(&sym, Price(dec("2690")));
    assert!(matches!(
        out,
        PaperMarkOutcome::AlgoTriggered {
            order_type: AlgoOrderType::TakeProfitMarket,
            realized_pnl,
            ..
        } if realized_pnl > Decimal::ZERO
    ));
}

#[tokio::test]
async fn paper_futures_liquidates_when_mark_crosses_liq_price() {
    let broker = PaperFuturesBroker::new(dec("1000"), 10, dec("0.004"));
    let sym = Symbol("SOLUSDT".into());
    broker
        .place_order(PlaceOrderRequest {
            symbol: sym.clone(),
            side: Side::Buy,
            kind: OrderKind::Limit,
            client_order_id: "b01r1F-000020-ENT".into(),
            quantity: Some(Qty(dec("10"))),
            quote_order_qty: None,
            price: Some(Price(dec("100"))),
            stop_price: None,
            time_in_force: Some(TimeInForce::Gtx),
        })
        .await
        .expect("entry");

    let pos = broker.position(&sym).expect("open pos");
    let out = broker.on_mark_price(&sym, pos.liq_price);
    assert!(matches!(out, PaperMarkOutcome::Liquidated { realized_pnl } if realized_pnl < Decimal::ZERO));
    assert!(broker.position(&sym).is_none());
}
