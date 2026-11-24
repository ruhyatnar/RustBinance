use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use async_trait::async_trait;
use bot_config::BotConfig;
use bot_engine::{EngineActor, EngineError};
use bot_exchange::{
    AccountBalance, AccountSnapshot, ActivationReport, CancelOrderAck, CancelOrderListAck,
    CancelOrderListRequest, CancelOrderRequest, CancelReplaceAck, CancelReplaceRequest,
    ExchangeGateway, ExchangeInfoSnapshot, GatewayError, OcoOrderAck, OrderAck, OrderSnapshot,
    PlaceOcoRequest, PlaceOrderRequest, QueryOrderRequest, VenueAdapter,
};
use bot_risk::ModeTrigger;
use bot_store::SqliteStore;
use bot_types::{
    EngineMode, InstrumentRules, OrderStatus, ProtectionScheme, Qty, Symbol, Venue, VenueCaps,
    VenueState,
};
use bot_venue::SwitchPolicy;
use rust_decimal::Decimal;
use tempfile::tempdir;

fn dec(s: &str) -> Decimal {
    Decimal::from_str_exact(s).expect("dec")
}

struct MockVenueAdapter {
    venue: Venue,
    fail_activate: AtomicBool,
    cancel_count: AtomicUsize,
}

impl MockVenueAdapter {
    fn new(venue: Venue, fail_activate: bool) -> Arc<Self> {
        Arc::new(Self {
            venue,
            fail_activate: AtomicBool::new(fail_activate),
            cancel_count: AtomicUsize::new(0),
        })
    }
}

#[async_trait]
impl ExchangeGateway for MockVenueAdapter {
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError> {
        Ok(OrderAck {
            venue: self.venue,
            symbol: req.symbol,
            client_order_id: req.client_order_id,
            order_id: 1,
            status: OrderStatus::New,
            executed_qty: Qty(Decimal::ZERO),
            cumulative_quote_qty: Decimal::ZERO,
            transact_time_ms: 1_000,
        })
    }

    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError> {
        self.cancel_count.fetch_add(1, Ordering::SeqCst);
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

    async fn place_oco(&self, _req: PlaceOcoRequest) -> Result<OcoOrderAck, GatewayError> {
        Err(GatewayError::Unsupported("mock".into()))
    }

    async fn cancel_list(
        &self,
        _req: CancelOrderListRequest,
    ) -> Result<CancelOrderListAck, GatewayError> {
        Err(GatewayError::Unsupported("mock".into()))
    }

    async fn query_order(&self, _req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError> {
        Err(GatewayError::Unsupported("mock".into()))
    }

    async fn open_orders(&self, _symbol: Option<&Symbol>) -> Result<Vec<OrderSnapshot>, GatewayError> {
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
impl VenueAdapter for MockVenueAdapter {
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
        if self.fail_activate.load(Ordering::SeqCst) {
            return Err(GatewayError::Transport("simulated WS/margin failure".into()));
        }
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

async fn setup_engine(
    fail_futures: bool,
) -> Result<
    (
        bot_engine::EngineHandle,
        tokio::sync::mpsc::Sender<bot_exchange::GatewayEvent>,
        tempfile::TempDir,
    ),
    EngineError,
> {
    let dir = tempdir().expect("tempdir");
    let db = dir.path().join("bot.sqlite");
    let store = SqliteStore::open(&db).expect("open store");
    let spot = MockVenueAdapter::new(Venue::Spot, false);
    let futures = MockVenueAdapter::new(Venue::UsdsFutures, fail_futures);
    let mut actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b1".to_owned(),
        store,
        Arc::clone(&spot) as Arc<dyn VenueAdapter>,
        dec("10000"),
        1_000_000,
    )
    .await?;
    actor.register_secondary_adapter(Arc::clone(&futures) as Arc<dyn VenueAdapter>, true, true);
    let (ev_tx, ev_rx) = tokio::sync::mpsc::channel(16);
    let (handle, _join) = actor.spawn(ev_rx);
    Ok((handle, ev_tx, dir))
}

#[tokio::test]
async fn chaos_13_4_activation_failure_rolls_back_and_pauses_engine() {
    let (handle, _spot, _dir) = setup_engine(true).await.expect("bootstrap");

    let res = handle
        .switch_venue(
            Venue::UsdsFutures,
            SwitchPolicy::KeepManaging,
            "operator".to_owned(),
            2_000_000,
        )
        .await
        .expect("channel ok");
    assert!(res.is_err(), "failed activation must return Err");

    let tel = handle.telemetry();
    assert_eq!(tel.venue, Venue::Spot, "must roll back to Spot");
    assert_eq!(tel.spot_state, VenueState::Active);
    assert_eq!(tel.futures_state, VenueState::Disabled);
    assert_eq!(tel.mode, EngineMode::Paused, "rollback must pause engine");
}

#[tokio::test]
async fn chaos_13_4_flapping_and_halted_rejected_without_resetting_state() {
    let (handle, _spot, _dir) = setup_engine(false).await.expect("bootstrap");

    // 1st switch Spot -> Futures succeeds.
    let ok1 = handle
        .switch_venue(
            Venue::UsdsFutures,
            SwitchPolicy::KeepManaging,
            "operator".to_owned(),
            2_000_000,
        )
        .await
        .expect("channel ok");
    assert!(ok1.is_ok(), "first switch must succeed: {ok1:?}");
    let tel = handle.telemetry();
    assert_eq!(tel.venue, Venue::UsdsFutures);
    assert_eq!(tel.futures_state, VenueState::Active);

    // Immediate switch back within 60s cooldown is rejected.
    let err_cd = handle
        .switch_venue(
            Venue::Spot,
            SwitchPolicy::KeepManaging,
            "operator".to_owned(),
            2_010_000,
        )
        .await
        .expect("channel ok");
    assert!(err_cd.is_err(), "cooldown must reject rapid switch");

    // Trigger Killed mode -> switch must be rejected.
    let _ = handle
        .apply_mode_trigger(ModeTrigger::ManualPanic, 2_100_000)
        .await
        .expect("panic");
    let err_halted = handle
        .switch_venue(
            Venue::Spot,
            SwitchPolicy::KeepManaging,
            "operator".to_owned(),
            2_200_000,
        )
        .await
        .expect("channel ok");
    assert!(
        err_halted.is_err(),
        "switch must be rejected when Killed/Halted"
    );
}
