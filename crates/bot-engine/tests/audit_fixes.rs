#![allow(clippy::expect_used, clippy::too_many_lines)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bot_config::BotConfig;
use bot_engine::{EngineActor, EntrySubmissionResult, RuntimeHealth};
use bot_exchange::{
    AccountBalance, AccountSnapshot, ActivationReport, AlgoOrderAck, CancelAlgoOrderAck,
    CancelAlgoOrderRequest, CancelOrderAck, CancelOrderListAck, CancelOrderListRequest,
    CancelOrderRequest, CancelReplaceAck, CancelReplaceRequest, ExchangeGateway,
    ExchangeInfoSnapshot, ExecEvent, GatewayError, GatewayEvent, OcoOrderAck, OrderAck,
    OrderSnapshot, PlaceAlgoOrderRequest, PlaceOcoRequest, PlaceOrderRequest, QueryOrderRequest,
    TimeInForce, VenueAdapter,
};
use bot_risk::{BtcSpikeSample, EntryIntent};
use bot_store::SqliteStore;
use bot_types::{
    Direction, EngineMode, InstrumentRules, OrderKind, OrderStatus, PositionState, Price,
    ProtectionScheme, Qty, Side, StrategyId, Symbol, Venue, VenueCaps,
};
use bot_venue::SwitchPolicy;
use rust_decimal::Decimal;
use tokio::sync::mpsc;

fn dec(s: &str) -> Decimal {
    Decimal::from_str_exact(s).expect("dec")
}

struct RecordingAdapter {
    venue: Venue,
    fail_protection: bool,
    placed_orders: Mutex<Vec<PlaceOrderRequest>>,
    placed_algos: Mutex<Vec<PlaceAlgoOrderRequest>>,
    canceled_algos: Mutex<Vec<CancelAlgoOrderRequest>>,
}

impl RecordingAdapter {
    fn new(venue: Venue, fail_protection: bool) -> Arc<Self> {
        Arc::new(Self {
            venue,
            fail_protection,
            placed_orders: Mutex::new(Vec::new()),
            placed_algos: Mutex::new(Vec::new()),
            canceled_algos: Mutex::new(Vec::new()),
        })
    }
}

#[async_trait]
impl ExchangeGateway for RecordingAdapter {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        self.placed_orders.lock().expect("lock").push(req.clone());
        Ok(OrderAck {
            venue: self.venue,
            symbol: req.symbol,
            client_order_id: req.client_order_id,
            order_id: 42,
            status: OrderStatus::New,
            executed_qty: Qty(Decimal::ZERO),
            cumulative_quote_qty: Decimal::ZERO,
            transact_time_ms: 1_000,
        })
    }

    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        Ok(CancelOrderAck {
            venue: self.venue,
            symbol: req.symbol,
            orig_client_order_id: req.orig_client_order_id.unwrap_or_default(),
            order_id: req.order_id.unwrap_or(1),
            status: OrderStatus::Canceled,
            executed_qty: Qty(Decimal::ZERO),
        })
    }

    async fn cancel_replace(
        &self,
        _req: CancelReplaceRequest,
    ) -> Result<CancelReplaceAck, GatewayError> {
        Err(GatewayError::Unsupported("mock".into()))
    }

    async fn place_oco(&self, req: PlaceOcoRequest) -> Result<OcoOrderAck, GatewayError> {
        if self.fail_protection {
            return Err(GatewayError::Transport("simulated OCO failure".into()));
        }
        Ok(OcoOrderAck {
            venue: self.venue,
            order_list_id: 77,
            list_client_order_id: req.list_client_order_id,
            symbol: req.symbol,
            list_order_status: "EXECUTING".into(),
            transaction_time_ms: 1_000,
            orders: vec![],
        })
    }

    async fn cancel_list(
        &self,
        req: CancelOrderListRequest,
    ) -> Result<CancelOrderListAck, GatewayError> {
        Ok(CancelOrderListAck {
            venue: self.venue,
            order_list_id: req.order_list_id.unwrap_or(77),
            list_client_order_id: req.list_client_order_id.unwrap_or_default(),
            symbol: req.symbol,
            list_order_status: "ALL_DONE".into(),
        })
    }

    async fn place_algo_order(
        &self,
        req: PlaceAlgoOrderRequest,
    ) -> Result<AlgoOrderAck, GatewayError> {
        if self.fail_protection {
            return Err(GatewayError::Transport("simulated algo failure".into()));
        }
        self.placed_algos.lock().expect("lock").push(req.clone());
        Ok(AlgoOrderAck {
            venue: self.venue,
            symbol: req.symbol,
            algo_id: 9001,
            client_algo_id: req.client_algo_id,
            status: "NEW".into(),
            transact_time_ms: 1_000,
        })
    }

    async fn cancel_algo_order(
        &self,
        req: CancelAlgoOrderRequest,
    ) -> Result<CancelAlgoOrderAck, GatewayError> {
        self.canceled_algos.lock().expect("lock").push(req.clone());
        Ok(CancelAlgoOrderAck {
            venue: self.venue,
            symbol: req.symbol,
            algo_id: req.algo_id.unwrap_or(9001),
            client_algo_id: req.client_algo_id.unwrap_or_default(),
        })
    }

    async fn query_order(&self, _req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError> {
        Err(GatewayError::Unsupported("mock".into()))
    }

    async fn open_orders(
        &self,
        _symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError> {
        Ok(vec![])
    }

    async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError> {
        Ok(AccountSnapshot {
            venue: self.venue,
            balances: vec![AccountBalance {
                asset: "USDT".to_owned(),
                free: dec("10000"),
                locked: Decimal::ZERO,
            }],
            can_trade: true,
            update_time_ms: 1_000,
        })
    }

    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError> {
        Ok(ExchangeInfoSnapshot {
            venue: self.venue,
            server_time_ms: 1_000,
            rate_limits: vec![],
            rules: vec![InstrumentRules {
                venue: self.venue,
                symbol: Symbol("BTCUSDT".to_owned()),
                tick_size: dec("0.1"),
                step_size: dec("0.001"),
                min_qty: dec("0.001"),
                max_qty: dec("1000"),
                min_notional: dec("10"),
                status_trading: true,
                base_precision: 3,
                quote_precision: 2,
            }],
        })
    }
}

#[async_trait]
impl VenueAdapter for RecordingAdapter {
    fn venue(&self) -> Venue {
        self.venue
    }

    fn caps(&self) -> VenueCaps {
        if self.venue == Venue::UsdsFutures {
            VenueCaps {
                can_short: true,
                max_leverage: 3,
                has_funding: true,
                protection: ProtectionScheme::AlgoReduceOnly,
            }
        } else {
            VenueCaps {
                can_short: false,
                max_leverage: 1,
                has_funding: false,
                protection: ProtectionScheme::DualOco,
            }
        }
    }

    async fn activate(&self) -> Result<ActivationReport, GatewayError> {
        Ok(ActivationReport {
            venue: self.venue,
            symbols_loaded: 1,
            server_time_offset_ms: 0,
            subscription_id: None,
        })
    }

    async fn deactivate(&self) -> Result<(), GatewayError> {
        Ok(())
    }
}

#[tokio::test]
async fn closed_user_events_channel_does_not_busy_loop_or_starve_commands() {
    let store = SqliteStore::open_in_memory().expect("sqlite");
    let spot = RecordingAdapter::new(Venue::Spot, false);
    let actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b1".into(),
        store,
        spot,
        dec("10000"),
        1_000_000,
    )
    .await
    .expect("bootstrap");

    let (ev_tx, ev_rx) = mpsc::channel(4);
    let (handle, join) = actor.spawn(ev_rx);
    drop(ev_tx); // Close event stream immediately.

    // Engine must still process commands promptly without starving in a busy-loop!
    let snap = tokio::time::timeout(Duration::from_secs(2), handle.full_snapshot(1_001_000))
        .await
        .expect("must not hang on closed user_events_rx")
        .expect("full_snapshot ok");
    assert_eq!(snap.telemetry.mode, EngineMode::Paused);
    assert!(!snap.health.user_stream_healthy);

    let _ = handle.shutdown(1_002_000).await;
    let _ = join.await;
}

#[tokio::test]
async fn futures_short_entry_uses_sell_gtx_algo_protection_and_buy_exit() {
    let store = SqliteStore::open_in_memory().expect("sqlite");
    let futures = RecordingAdapter::new(Venue::UsdsFutures, false);
    let actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b1".into(),
        store,
        Arc::clone(&futures) as Arc<dyn VenueAdapter>,
        dec("10000"),
        1_000_000,
    )
    .await
    .expect("bootstrap");
    let (ev_tx, ev_rx) = mpsc::channel(16);
    let (handle, join) = actor.spawn(ev_rx);

    let intent = EntryIntent {
        venue: Venue::UsdsFutures,
        symbol: Symbol("BTCUSDT".into()),
        direction: Direction::Short,
        strategy: StrategyId::TrendPullback,
        entry_price: Price(dec("50000")),
        stop_price: Price(dec("51000")),
        tp1_price: Price(dec("48500")),
        size_mult: Decimal::ONE,
        scanner_mult: Decimal::ONE,
        is_scanner: false,
    };

    let (cid, pos_id, qty) = match handle
        .submit_entry(intent, 1_001_000)
        .await
        .expect("submit")
    {
        EntrySubmissionResult::Submitted {
            client_order_id,
            position_id,
            qty,
        } => (client_order_id, position_id, qty),
        other => panic!("expected Submitted, got {other:?}"),
    };

    {
        let orders = futures.placed_orders.lock().expect("lock");
        assert_eq!(orders.len(), 1);
        assert_eq!(orders[0].side, Side::Sell);
        assert_eq!(orders[0].kind, OrderKind::Limit);
        assert_eq!(orders[0].time_in_force, Some(TimeInForce::Gtx));
    }

    // Deliver partial fill -> must install Futures Algo SL + TP immediately!
    let half_qty = Qty(qty.as_decimal() / dec("2"));
    ev_tx
        .send(GatewayEvent::Exec(ExecEvent {
            venue: Venue::UsdsFutures,
            event_time_ms: 1_002_000,
            transact_time_ms: 1_002_000,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id: cid,
            orig_client_order_id: None,
            order_id: 42,
            order_list_id: -1,
            trade_id: 777,
            side: Side::Sell,
            kind: OrderKind::Limit,
            exec_type: "TRADE".into(),
            status: OrderStatus::PartiallyFilled,
            reject_reason: "NONE".into(),
            order_price: Price(dec("50000")),
            order_qty: qty,
            last_filled_qty: half_qty,
            cumulative_filled_qty: half_qty,
            last_filled_price: Price(dec("50000")),
            cumulative_quote_qty: half_qty.as_decimal() * dec("50000"),
            commission: dec("0.05"),
            commission_asset: Some("USDT".into()),
            is_maker: true,
        }))
        .await
        .expect("send exec");

    // Verify Algo SL/TP installed and full_snapshot exposes open Short position.
    let snap = handle.full_snapshot(1_003_000).await.expect("snapshot");
    assert_eq!(snap.positions.len(), 1);
    assert_eq!(snap.positions[0].id, pos_id);
    assert_eq!(snap.positions[0].state, PositionState::OpenProtected);
    assert!(
        snap.positions[0].tp2_price.expect("tp2").as_decimal() < dec("50000"),
        "Short TP2 must be below entry"
    );
    assert_eq!(futures.placed_algos.lock().expect("lock").len(), 2);

    // Panic flatten must close Short position with Side::Buy and cancel Algo SL/TP!
    let (mode, flattened) = handle.panic_flatten(1_004_000).await.expect("panic");
    assert_eq!(mode, EngineMode::Killed);
    assert_eq!(flattened, 1);
    {
        let orders = futures.placed_orders.lock().expect("lock");
        let exit_order = orders.last().expect("exit order");
        assert_eq!(exit_order.side, Side::Buy, "Short exit MUST be Side::Buy");
        assert_eq!(exit_order.kind, OrderKind::Market);
    }
    assert_eq!(futures.canceled_algos.lock().expect("lock").len(), 2);

    let _ = handle.shutdown(1_005_000).await;
    let _ = join.await;
}

#[tokio::test]
async fn failed_protection_marks_open_unprotected_and_switch_flatten_closes_positions() {
    let store = SqliteStore::open_in_memory().expect("sqlite");
    let spot = RecordingAdapter::new(Venue::Spot, true); // OCO fails
    let futures = RecordingAdapter::new(Venue::UsdsFutures, false);
    let mut actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b1".into(),
        store,
        Arc::clone(&spot) as Arc<dyn VenueAdapter>,
        dec("10000"),
        1_000_000,
    )
    .await
    .expect("bootstrap");
    actor.register_secondary_adapter(Arc::clone(&futures) as Arc<dyn VenueAdapter>, true, true);
    let (ev_tx, ev_rx) = mpsc::channel(16);
    let (handle, join) = actor.spawn(ev_rx);

    let intent = EntryIntent {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        direction: Direction::Long,
        strategy: StrategyId::TrendPullback,
        entry_price: Price(dec("50000")),
        stop_price: Price(dec("49000")),
        tp1_price: Price(dec("51500")),
        size_mult: Decimal::ONE,
        scanner_mult: Decimal::ONE,
        is_scanner: false,
    };
    let EntrySubmissionResult::Submitted {
        client_order_id,
        qty,
        ..
    } = handle
        .submit_entry(intent, 1_001_000)
        .await
        .expect("submit")
    else {
        panic!("expected submitted");
    };

    ev_tx
        .send(GatewayEvent::Exec(ExecEvent {
            venue: Venue::Spot,
            event_time_ms: 1_002_000,
            transact_time_ms: 1_002_000,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id,
            orig_client_order_id: None,
            order_id: 42,
            order_list_id: -1,
            trade_id: 888,
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            exec_type: "TRADE".into(),
            status: OrderStatus::Filled,
            reject_reason: "NONE".into(),
            order_price: Price(dec("50000")),
            order_qty: qty,
            last_filled_qty: qty,
            cumulative_filled_qty: qty,
            last_filled_price: Price(dec("50000")),
            cumulative_quote_qty: qty.as_decimal() * dec("50000"),
            commission: dec("0.05"),
            commission_asset: Some("USDT".into()),
            is_maker: true,
        }))
        .await
        .expect("send fill");

    let snap = handle.full_snapshot(1_003_000).await.expect("snap");
    assert_eq!(snap.positions.len(), 1);
    assert_eq!(
        snap.positions[0].state,
        PositionState::OpenUnprotected,
        "failed OCO must mark position OpenUnprotected"
    );

    // Switch Spot -> Futures with Flatten must actually close the Spot open position!
    let rep = handle
        .switch_venue(
            Venue::UsdsFutures,
            SwitchPolicy::Flatten,
            "operator".into(),
            1_100_000,
        )
        .await
        .expect("channel")
        .expect("switch flatten ok");
    assert_eq!(rep.to, Venue::UsdsFutures);
    let snap2 = handle.full_snapshot(1_101_000).await.expect("snap2");
    assert!(
        snap2.positions.is_empty(),
        "Flatten must close open Spot positions"
    );

    // Verify BtcSpikeGuard blocks Short entries on Futures and set_param enforces tighten-only.
    handle
        .update_health(
            RuntimeHealth::default(),
            None,
            Some(BtcSpikeSample {
                now_ms: 1_102_000,
                price: dec("51000"),
                rise_15m_pct: dec("2.0"),
                rise_60m_pct: dec("1.0"),
                reference_low_price: dec("50000"),
                atr_1m_pct: dec("0.3"),
                median_atr_24h_pct: dec("0.2"),
            }),
            1_102_000,
        )
        .await
        .expect("update health");
    let blocked = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::UsdsFutures,
                symbol: Symbol("BTCUSDT".into()),
                direction: Direction::Short,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(dec("50000")),
                stop_price: Price(dec("51000")),
                tp1_price: Price(dec("48500")),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            1_103_000,
        )
        .await
        .expect("submit");
    assert!(matches!(blocked, EntrySubmissionResult::Rejected(_)));

    assert!(
        handle
            .set_param("risk.max_open_positions".into(), "2".into())
            .await
            .expect("chan")
            .is_ok()
    );
    assert!(
        handle
            .set_param("risk.max_open_positions".into(), "10".into())
            .await
            .expect("chan")
            .is_err(),
        "loosening must be rejected"
    );
    assert!(
        handle
            .disable_strategy("breakout".into())
            .await
            .expect("chan")
            .is_ok()
    );

    let _ = handle.shutdown(1_105_000).await;
    let _ = join.await;
}
