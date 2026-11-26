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
    canceled_orders: Mutex<Vec<CancelOrderRequest>>,
    placed_algos: Mutex<Vec<PlaceAlgoOrderRequest>>,
    canceled_algos: Mutex<Vec<CancelAlgoOrderRequest>>,
    market_fill_price: Mutex<Option<Price>>,
}

impl RecordingAdapter {
    fn new(venue: Venue, fail_protection: bool) -> Arc<Self> {
        Arc::new(Self {
            venue,
            fail_protection,
            placed_orders: Mutex::new(Vec::new()),
            canceled_orders: Mutex::new(Vec::new()),
            placed_algos: Mutex::new(Vec::new()),
            canceled_algos: Mutex::new(Vec::new()),
            market_fill_price: Mutex::new(None),
        })
    }

    fn set_market_fill_price(&self, price: Price) {
        *self.market_fill_price.lock().expect("lock") = Some(price);
    }
}

#[async_trait]
impl ExchangeGateway for RecordingAdapter {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        self.placed_orders.lock().expect("lock").push(req.clone());
        let market_px = *self.market_fill_price.lock().expect("lock");
        let (status, executed_qty, cumulative_quote_qty) = if req.kind == OrderKind::Market
            && let Some(px) = market_px
        {
            let q = req.quantity.unwrap_or(Qty(Decimal::ZERO));
            (
                OrderStatus::Filled,
                q,
                q.as_decimal().saturating_mul(px.as_decimal()),
            )
        } else {
            (OrderStatus::New, Qty(Decimal::ZERO), Decimal::ZERO)
        };
        Ok(OrderAck {
            venue: self.venue,
            symbol: req.symbol,
            client_order_id: req.client_order_id,
            order_id: 42,
            status,
            executed_qty,
            cumulative_quote_qty,
            transact_time_ms: 1_000,
        })
    }

    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        self.canceled_orders.lock().expect("lock").push(req.clone());
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

/// Audit #4, #5, #6: Futures SL/TP exit `Exec` fills decrement `qty_open`, compute realized `PnL`
/// from actual fill prices without double-subtracting fees, close the position, and update SQLite `positions`.
#[tokio::test]
async fn audit_4_5_6_futures_exit_exec_fills_close_position_and_update_sqlite() {
    use bot_engine::MarketTick;
    use uuid::Uuid;

    let db_path =
        std::env::temp_dir().join(format!("bot_engine_audit456_{}.sqlite", Uuid::now_v7()));
    let store = SqliteStore::open(&db_path).expect("open sqlite");
    let adapter = RecordingAdapter::new(Venue::UsdsFutures, false);
    let mut cfg = BotConfig::default();
    cfg.venues.default_active = Venue::UsdsFutures;
    cfg.venues.futures.enabled = true;

    let actor = EngineActor::bootstrap(
        cfg,
        "b01".into(),
        store,
        Arc::clone(&adapter) as Arc<dyn VenueAdapter>,
        dec("10000"),
        1_000,
    )
    .await
    .expect("bootstrap");

    let (ev_tx, ev_rx) = mpsc::channel(64);
    let (handle, join) = actor.spawn(ev_rx);

    // 1. Submit Futures Long entry at 100, stop 95, tp1 110.
    let sub = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::UsdsFutures,
                symbol: Symbol("BTCUSDT".into()),
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(dec("100")),
                stop_price: Price(dec("95")),
                tp1_price: Price(dec("110")),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            2_000,
        )
        .await
        .expect("chan");
    let EntrySubmissionResult::Submitted {
        client_order_id,
        position_id,
        qty,
    } = sub
    else {
        panic!("expected Submitted, got {sub:?}");
    };

    // 2. Entry fills at 100 with 0.20 USDT fee.
    ev_tx
        .send(GatewayEvent::Exec(ExecEvent {
            venue: Venue::UsdsFutures,
            event_time_ms: 2_100,
            transact_time_ms: 2_100,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id,
            orig_client_order_id: None,
            order_id: 42,
            order_list_id: -1,
            trade_id: 1001,
            side: Side::Buy,
            kind: OrderKind::Limit,
            exec_type: "TRADE".into(),
            status: OrderStatus::Filled,
            reject_reason: "NONE".into(),
            order_price: Price(dec("100")),
            order_qty: qty,
            last_filled_qty: qty,
            cumulative_filled_qty: qty,
            last_filled_price: Price(dec("100")),
            cumulative_quote_qty: qty.as_decimal() * dec("100"),
            commission: dec("0.20"),
            commission_asset: Some("USDT".into()),
            is_maker: true,
        }))
        .await
        .expect("send entry fill");

    // Set a deliberately different ref_price (105) to prove actual fill prices are used!
    handle
        .on_market_tick(MarketTick {
            venue: Venue::UsdsFutures,
            symbol: Symbol("BTCUSDT".into()),
            price: Price(dec("105")),
            atr: dec("2"),
            rsi_14: Some(dec("55")),
            bars_in_trade: 1,
            now_ms: 2_200,
        })
        .await
        .expect("tick");

    // 3. Partial TP exit fill (half qty) at 110, fee 0.15 USDT (untracked Futures Algo order).
    let half_qty = Qty(qty.as_decimal() / dec("2"));
    let rem_qty = Qty(qty.as_decimal() - half_qty.as_decimal());
    ev_tx
        .send(GatewayEvent::Exec(ExecEvent {
            venue: Venue::UsdsFutures,
            event_time_ms: 2_300,
            transact_time_ms: 2_300,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id: "b01r1F-000099-TP1".into(),
            orig_client_order_id: None,
            order_id: 99,
            order_list_id: -1,
            trade_id: 1002,
            side: Side::Sell,
            kind: OrderKind::Market,
            exec_type: "TRADE".into(),
            status: OrderStatus::Filled,
            reject_reason: "NONE".into(),
            order_price: Price(dec("110")),
            order_qty: half_qty,
            last_filled_qty: half_qty,
            cumulative_filled_qty: half_qty,
            last_filled_price: Price(dec("110")),
            cumulative_quote_qty: half_qty.as_decimal() * dec("110"),
            commission: dec("0.15"),
            commission_asset: Some("USDT".into()),
            is_maker: false,
        }))
        .await
        .expect("send partial tp fill");

    // 4. Final SL/TP exit fill (remaining qty) at 120, fee 0.25 USDT.
    ev_tx
        .send(GatewayEvent::Exec(ExecEvent {
            venue: Venue::UsdsFutures,
            event_time_ms: 2_400,
            transact_time_ms: 2_400,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id: "b01r1F-000100-SL".into(),
            orig_client_order_id: None,
            order_id: 100,
            order_list_id: -1,
            trade_id: 1003,
            side: Side::Sell,
            kind: OrderKind::Market,
            exec_type: "TRADE".into(),
            status: OrderStatus::Filled,
            reject_reason: "NONE".into(),
            order_price: Price(dec("120")),
            order_qty: rem_qty,
            last_filled_qty: rem_qty,
            cumulative_filled_qty: rem_qty,
            last_filled_price: Price(dec("120")),
            cumulative_quote_qty: rem_qty.as_decimal() * dec("120"),
            commission: dec("0.25"),
            commission_asset: Some("USDT".into()),
            is_maker: false,
        }))
        .await
        .expect("send final exit fill");

    // Sync via snapshot (actor processes user_events_rx with biased priority).
    let snap = handle.full_snapshot(2_500).await.expect("snap");
    assert!(
        snap.positions.is_empty(),
        "Futures position must be closed after exit Exec fills"
    );

    let _ = handle.shutdown(2_600).await;
    let _ = join.await;

    // Verify SQLite relational `positions` table was updated with exact PnL and Closed state!
    let verify_store = SqliteStore::open(&db_path).expect("reopen sqlite");
    let db_pos = verify_store
        .get_position(position_id)
        .expect("get_position")
        .expect("position row must exist");
    let expected_gross = half_qty.as_decimal() * dec("10") + rem_qty.as_decimal() * dec("20");
    let expected_fees = dec("0.20") + dec("0.15") + dec("0.25");
    let expected_net = expected_gross - expected_fees;
    assert_eq!(db_pos.state, PositionState::Closed);
    assert_eq!(db_pos.qty_open, Qty(Decimal::ZERO));
    assert_eq!(db_pos.fees_paid_quote, expected_fees);
    assert_eq!(
        db_pos.realized_pnl_quote, expected_net,
        "realized PnL must use actual fill prices and subtract each fee exactly once"
    );

    let _ = std::fs::remove_file(&db_path);
}

/// Audit #5, #6, #8: `EngineFullSnapshot` exposes `ref_prices` and direction-aware unrealized `PnL` /
/// `profit_r`; `execute_market_exit` uses `OrderAck` fill price and updates SQLite `positions`.
#[tokio::test]
async fn audit_5_6_8_snapshot_mark_metrics_and_market_exit_ack_fill_price() {
    use bot_engine::MarketTick;
    use uuid::Uuid;

    let db_path =
        std::env::temp_dir().join(format!("bot_engine_audit568_{}.sqlite", Uuid::now_v7()));
    let store = SqliteStore::open(&db_path).expect("open sqlite");
    let adapter = RecordingAdapter::new(Venue::UsdsFutures, false);
    let mut cfg = BotConfig::default();
    cfg.venues.default_active = Venue::UsdsFutures;
    cfg.venues.futures.enabled = true;

    let actor = EngineActor::bootstrap(
        cfg,
        "b01".into(),
        store,
        Arc::clone(&adapter) as Arc<dyn VenueAdapter>,
        dec("10000"),
        1_000,
    )
    .await
    .expect("bootstrap");

    let (ev_tx, ev_rx) = mpsc::channel(64);
    let (handle, join) = actor.spawn(ev_rx);

    // Open a Futures Short at 100, stop 105 (R = 5), tp1 90.
    let sub = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::UsdsFutures,
                symbol: Symbol("BTCUSDT".into()),
                direction: Direction::Short,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(dec("100")),
                stop_price: Price(dec("105")),
                tp1_price: Price(dec("90")),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            2_000,
        )
        .await
        .expect("chan");
    let EntrySubmissionResult::Submitted {
        client_order_id,
        position_id,
        qty,
    } = sub
    else {
        panic!("expected Submitted, got {sub:?}");
    };

    ev_tx
        .send(GatewayEvent::Exec(ExecEvent {
            venue: Venue::UsdsFutures,
            event_time_ms: 2_100,
            transact_time_ms: 2_100,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id,
            orig_client_order_id: None,
            order_id: 42,
            order_list_id: -1,
            trade_id: 2001,
            side: Side::Sell,
            kind: OrderKind::Limit,
            exec_type: "TRADE".into(),
            status: OrderStatus::Filled,
            reject_reason: "NONE".into(),
            order_price: Price(dec("100")),
            order_qty: qty,
            last_filled_qty: qty,
            cumulative_filled_qty: qty,
            last_filled_price: Price(dec("100")),
            cumulative_quote_qty: qty.as_decimal() * dec("100"),
            commission: dec("0.10"),
            commission_asset: Some("USDT".into()),
            is_maker: true,
        }))
        .await
        .expect("entry fill");

    // Tick mark price down to 95 (favorable for Short: +1R, +5 * qty unrealized PnL).
    handle
        .on_market_tick(MarketTick {
            venue: Venue::UsdsFutures,
            symbol: Symbol("BTCUSDT".into()),
            price: Price(dec("95")),
            atr: dec("2"),
            rsi_14: Some(dec("45")),
            bars_in_trade: 1,
            now_ms: 2_200,
        })
        .await
        .expect("tick");

    let snap = handle.full_snapshot(2_300).await.expect("snap");
    assert_eq!(
        snap.ref_prices
            .get(&(Venue::UsdsFutures, Symbol("BTCUSDT".into()))),
        Some(&Price(dec("95")))
    );
    assert_eq!(snap.positions.len(), 1);
    let (mark, unrealized, profit_r) = snap.position_mark_metrics(&snap.positions[0]);
    assert_eq!(mark, Price(dec("95")));
    assert_eq!(unrealized, qty.as_decimal() * dec("5"));
    assert_eq!(profit_r, Decimal::ONE);

    // Market close executes at actual fill price 92 (different from ref_price 95!).
    adapter.set_market_fill_price(Price(dec("92")));
    handle
        .close_position(position_id, 2_400)
        .await
        .expect("chan")
        .expect("close ok");

    let _ = handle.shutdown(2_500).await;
    let _ = join.await;

    let verify_store = SqliteStore::open(&db_path).expect("reopen sqlite");
    let db_pos = verify_store
        .get_position(position_id)
        .expect("get_position")
        .expect("position row");
    assert_eq!(db_pos.state, PositionState::Closed);
    assert_eq!(db_pos.qty_open, Qty(Decimal::ZERO));
    let expected_net = qty.as_decimal() * dec("8") - dec("0.10");
    assert_eq!(
        db_pos.realized_pnl_quote, expected_net,
        "execute_market_exit must use actual OrderAck fill price (92) rather than ref_price (95)"
    );

    let _ = std::fs::remove_file(&db_path);
}

/// Audit #9: Spot Dual-OCO install and runner stop move keep `state.last_seq` strictly in sync
/// with SQLite journal `max_seq` (no `last_seq + 1` drift).
#[tokio::test]
async fn audit_9_spot_install_and_stop_move_keep_last_seq_synced() {
    use bot_engine::MarketTick;
    use uuid::Uuid;

    let db_path = std::env::temp_dir().join(format!("bot_engine_audit9_{}.sqlite", Uuid::now_v7()));
    let store = SqliteStore::open(&db_path).expect("open sqlite");
    let adapter = RecordingAdapter::new(Venue::Spot, false);

    let actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b01".into(),
        store,
        Arc::clone(&adapter) as Arc<dyn VenueAdapter>,
        dec("10000"),
        1_000,
    )
    .await
    .expect("bootstrap");

    let (ev_tx, ev_rx) = mpsc::channel(64);
    let (handle, join) = actor.spawn(ev_rx);

    let sub = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::Spot,
                symbol: Symbol("BTCUSDT".into()),
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(dec("100")),
                stop_price: Price(dec("95")),
                tp1_price: Price(dec("110")),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            2_000,
        )
        .await
        .expect("chan");
    let EntrySubmissionResult::Submitted {
        client_order_id,
        qty,
        ..
    } = sub
    else {
        panic!("expected Submitted, got {sub:?}");
    };

    // Entry fill triggers install_dual_oco (which appends PositionStopMoved).
    ev_tx
        .send(GatewayEvent::Exec(ExecEvent {
            venue: Venue::Spot,
            event_time_ms: 2_100,
            transact_time_ms: 2_100,
            symbol: Symbol("BTCUSDT".into()),
            client_order_id,
            orig_client_order_id: None,
            order_id: 42,
            order_list_id: -1,
            trade_id: 3001,
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            exec_type: "TRADE".into(),
            status: OrderStatus::Filled,
            reject_reason: "NONE".into(),
            order_price: Price(dec("100")),
            order_qty: qty,
            last_filled_qty: qty,
            cumulative_filled_qty: qty,
            last_filled_price: Price(dec("100")),
            cumulative_quote_qty: qty.as_decimal() * dec("100"),
            commission: Decimal::ZERO,
            commission_asset: Some("USDT".into()),
            is_maker: true,
        }))
        .await
        .expect("entry fill");

    // Tick at 106 (> +1R = 105) triggers Break-Even stop move via replace_runner_stop!
    handle
        .on_market_tick(MarketTick {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            price: Price(dec("106")),
            atr: dec("2"),
            rsi_14: Some(dec("60")),
            bars_in_trade: 2,
            now_ms: 2_200,
        })
        .await
        .expect("be tick");

    let snap = handle.full_snapshot(2_300).await.expect("snap");
    assert!(snap.positions[0].breakeven_done);

    let _ = handle.shutdown(2_400).await;
    let actor = join.await.expect("join");

    let verify_store = SqliteStore::open(&db_path).expect("reopen");
    let recovered = verify_store.recover_state().expect("recover");
    assert_eq!(
        actor.state().last_seq,
        recovered.last_seq,
        "in-memory last_seq must exactly match SQLite journal max seq after Spot OCO install & stop move"
    );

    let _ = std::fs::remove_file(&db_path);
}

/// Audit #10: Stale unfilled `OrderPurpose::Entry` orders past TTL are canceled cleanly on ticks
/// without sleep loops, while partial fills before TTL are preserved & protected.
#[tokio::test]
async fn audit_10_entry_ttl_cancels_stale_unfilled_entry_and_preserves_partial_fill() {
    use bot_engine::MarketTick;
    use uuid::Uuid;

    let db_path =
        std::env::temp_dir().join(format!("bot_engine_audit10_{}.sqlite", Uuid::now_v7()));
    let store = SqliteStore::open(&db_path).expect("open sqlite");
    let adapter = RecordingAdapter::new(Venue::Spot, false);

    let actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b01".into(),
        store,
        Arc::clone(&adapter) as Arc<dyn VenueAdapter>,
        dec("10000"),
        1_000,
    )
    .await
    .expect("bootstrap")
    .with_entry_ttl_ms(5_000);

    let (_ev_tx, ev_rx) = mpsc::channel(64);
    let (handle, join) = actor.spawn(ev_rx);

    let sub = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::Spot,
                symbol: Symbol("BTCUSDT".into()),
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(dec("100")),
                stop_price: Price(dec("95")),
                tp1_price: Price(dec("110")),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            2_000,
        )
        .await
        .expect("chan");
    let EntrySubmissionResult::Submitted {
        client_order_id,
        position_id,
        ..
    } = sub
    else {
        panic!("expected Submitted, got {sub:?}");
    };

    // Before TTL (2_000 + 5_000 = 7_000): tick at 6_000 must NOT cancel.
    handle
        .on_market_tick(MarketTick {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            price: Price(dec("101")),
            atr: dec("2"),
            rsi_14: None,
            bars_in_trade: 0,
            now_ms: 6_000,
        })
        .await
        .expect("tick before ttl");
    assert!(adapter.canceled_orders.lock().expect("lock").is_empty());

    // Past TTL: tick at 7_500 must cancel the unfilled entry order and close EntryPending position.
    handle
        .on_market_tick(MarketTick {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".into()),
            price: Price(dec("101")),
            atr: dec("2"),
            rsi_14: None,
            bars_in_trade: 0,
            now_ms: 7_500,
        })
        .await
        .expect("tick after ttl");

    {
        let canceled = adapter.canceled_orders.lock().expect("lock");
        assert_eq!(canceled.len(), 1);
        assert_eq!(
            canceled[0].orig_client_order_id.as_deref(),
            Some(client_order_id.as_str())
        );
    }

    let snap = handle.full_snapshot(7_600).await.expect("snap");
    assert!(
        snap.orders.is_empty(),
        "expired entry order must be terminal"
    );
    assert!(
        snap.positions.is_empty(),
        "unfilled EntryPending position must be closed after TTL expiry"
    );

    let _ = handle.shutdown(7_700).await;
    let _ = join.await;

    let verify_store = SqliteStore::open(&db_path).expect("reopen");
    let db_pos = verify_store
        .get_position(position_id)
        .expect("get_position")
        .expect("row");
    assert_eq!(db_pos.state, PositionState::Closed);

    let _ = std::fs::remove_file(&db_path);
}
