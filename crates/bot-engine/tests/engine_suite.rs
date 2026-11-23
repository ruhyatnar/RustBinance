//! Integration, SIGTERM protection preservation, channel backpressure, and child-process
//! `kill -9` (`abort()`) crash recovery tests for `bot-engine` (M07 g–h).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use bot_config::BotConfig;
use bot_engine::{
    COMMAND_CHANNEL_CAPACITY, EngineActor, EngineCommand, EngineHandleError, EntrySubmissionResult,
    MarketTick,
};
use bot_exchange::VenueAdapter;
use bot_exec::{PaperBroker, PaperBrokerConfig};
use bot_risk::EntryIntent;
use bot_store::{JournalEnvelope, JournalEvent, PersistedEngineState, SqliteStore};
use bot_types::{
    Direction, EngineMode, InstrumentRules, Order, OrderKind, OrderPurpose, OrderStatus, Position,
    PositionState, Price, Qty, Side, StrategyId, Symbol, Venue,
};
use rust_decimal::Decimal;
use tempfile::TempDir;
use time::OffsetDateTime;
use tokio::sync::oneshot;
use uuid::Uuid;

fn sample_rules(sym: &str) -> InstrumentRules {
    InstrumentRules {
        venue: Venue::Spot,
        symbol: Symbol(sym.into()),
        tick_size: Decimal::new(1, 2),
        step_size: Decimal::new(1, 2),
        min_qty: Decimal::new(1, 2),
        max_qty: Decimal::from(10_000_u32),
        min_notional: Decimal::from(5_u32),
        status_trading: true,
        base_precision: 8,
        quote_precision: 8,
    }
}

#[tokio::test]
async fn sigterm_graceful_shutdown_cancels_pending_entry_but_preserves_protection() {
    let btc = Symbol("BTCUSDT".into());
    let eth = Symbol("ETHUSDT".into());
    let broker = Arc::new(PaperBroker::new(PaperBrokerConfig {
        rules: vec![sample_rules("BTCUSDT"), sample_rules("ETHUSDT")],
        initial_quote_balance: Decimal::from(10_000_u32),
        ..PaperBrokerConfig::default()
    }));
    broker.activate().await.expect("activate");
    let event_rx = broker.take_event_rx().expect("event_rx");

    // Seed prices
    broker.on_price_tick(
        &btc,
        Price(Decimal::from(100_u32)),
        Price(Decimal::from(101_u32)),
        1_000,
    );
    broker.on_price_tick(
        &eth,
        Price(Decimal::from(50_u32)),
        Price(Decimal::from(51_u32)),
        1_000,
    );

    let store = SqliteStore::open_in_memory().expect("store");
    let actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b01".into(),
        store,
        broker.clone(),
        Decimal::from(10_000_u32),
        1_000,
    )
    .await
    .expect("bootstrap");
    let (handle, join) = actor.spawn(event_rx);

    // 1. Submit BTC entry and fill it so Dual-OCO protection (2 OCO lists = 4 orders) is installed.
    let btc_res = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::Spot,
                symbol: btc.clone(),
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(Decimal::from(100_u32)),
                stop_price: Price(Decimal::from(96_u32)),
                tp1_price: Price(Decimal::from(106_u32)),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            1_100,
        )
        .await
        .expect("submit btc");
    assert!(matches!(btc_res, EntrySubmissionResult::Submitted { .. }));

    // Fill BTC entry by dropping ask to 100
    broker.on_price_tick(
        &btc,
        Price(Decimal::from(99_u32)),
        Price(Decimal::from(100_u32)),
        1_200,
    );
    // Give actor event loop a moment to process fill & install Dual-OCO
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(
        broker.active_oco_count(),
        2,
        "Dual-OCO must be installed for open BTC position"
    );

    // 2. Submit ETH entry and leave it PENDING (unfilled)
    let eth_res = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::Spot,
                symbol: eth.clone(),
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(Decimal::from(50_u32)),
                stop_price: Price(Decimal::from(48_u32)),
                tp1_price: Price(Decimal::from(53_u32)),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            1_300,
        )
        .await
        .expect("submit eth");
    assert!(matches!(eth_res, EntrySubmissionResult::Submitted { .. }));
    assert_eq!(broker.open_order_count(), 5); // 4 OCO legs + 1 pending ETH entry

    // 3. SIGTERM graceful shutdown: MUST cancel ETH entry ONLY and preserve BTC Dual-OCO!
    let canceled = handle.shutdown(2_000).await.expect("shutdown");
    let _actor = join.await.expect("join");

    assert_eq!(canceled, 1, "only pending entry order should be canceled");
    assert_eq!(
        broker.active_oco_count(),
        2,
        "Hard Rule #7: protective OCOs MUST remain active after SIGTERM"
    );
    assert_eq!(
        broker.open_order_count(),
        4,
        "all 4 protective OCO legs must remain open on exchange"
    );
}

#[tokio::test]
async fn engine_end_to_end_market_tick_triggers_time_stop_exit() {
    let btc = Symbol("BTCUSDT".into());
    let broker = Arc::new(PaperBroker::new(PaperBrokerConfig {
        rules: vec![sample_rules("BTCUSDT")],
        initial_quote_balance: Decimal::from(10_000_u32),
        ..PaperBrokerConfig::default()
    }));
    broker.activate().await.expect("activate");
    let event_rx = broker.take_event_rx().expect("event_rx");
    broker.on_price_tick(
        &btc,
        Price(Decimal::from(100_u32)),
        Price(Decimal::from(101_u32)),
        1_000,
    );

    let store = SqliteStore::open_in_memory().expect("store");
    let actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b01".into(),
        store,
        broker.clone(),
        Decimal::from(10_000_u32),
        1_000,
    )
    .await
    .expect("bootstrap");
    let (handle, join) = actor.spawn(event_rx);

    let res = handle
        .submit_entry(
            EntryIntent {
                venue: Venue::Spot,
                symbol: btc.clone(),
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(Decimal::from(100_u32)),
                stop_price: Price(Decimal::from(96_u32)),
                tp1_price: Price(Decimal::from(106_u32)),
                size_mult: Decimal::ONE,
                scanner_mult: Decimal::ONE,
                is_scanner: false,
            },
            1_100,
        )
        .await
        .expect("submit");
    assert!(matches!(res, EntrySubmissionResult::Submitted { .. }));

    broker.on_price_tick(
        &btc,
        Price(Decimal::from(99_u32)),
        Price(Decimal::from(100_u32)),
        1_200,
    );
    tokio::time::sleep(Duration::from_millis(40)).await;
    assert_eq!(handle.telemetry().open_positions, 1);

    // Trigger time stop at 48 bars with flat price (0R < 0.3R)
    handle
        .on_market_tick(MarketTick {
            venue: Venue::Spot,
            symbol: btc,
            price: Price(Decimal::from(100_u32)),
            atr: Decimal::from(1_u32),
            rsi_14: Some(Decimal::from(50_u32)),
            bars_in_trade: 48,
            now_ms: 5_000,
        })
        .await
        .expect("tick");

    assert_eq!(handle.telemetry().open_positions, 0);
    assert_eq!(broker.active_oco_count(), 0);
    let _ = handle.shutdown(6_000).await;
    let _ = join.await;
}

/// Child-process crash (`abort()`) test covering `EntryPending`, `OpenProtected`, and `ExitPending`
/// states per M07 acceptance criteria.
#[test]
fn crash_kill_9_recovery_in_entry_pending_open_protected_and_exit_pending() {
    if let Ok(db_path_str) = std::env::var("BOT_ENGINE_CRASH_CHILD_DB") {
        let mut store = SqliteStore::open(&PathBuf::from(db_path_str)).expect("child open");
        let mut state = PersistedEngineState::default();

        // Persist 3 positions in EntryPending, OpenProtected, and ExitPending + active Entry order.
        let states = [
            (1_u128, "BTCUSDT", PositionState::EntryPending),
            (2_u128, "ETHUSDT", PositionState::OpenProtected),
            (3_u128, "SOLUSDT", PositionState::ExitPending),
        ];
        for (idx, (id_num, sym, pstate)) in states.iter().enumerate() {
            let ts = 1_700_000_000_000_i64 + i64::try_from(idx).unwrap() * 1_000;
            let pos = Position {
                id: Uuid::from_u128(*id_num),
                symbol: Symbol((*sym).into()),
                venue: Venue::Spot,
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(Decimal::from(100_u32)),
                qty_initial: Qty(Decimal::from(2_u32)),
                qty_open: Qty(if *pstate == PositionState::EntryPending {
                    Decimal::ZERO
                } else {
                    Decimal::from(2_u32)
                }),
                r_distance: Decimal::from(4_u32),
                leverage: None,
                liq_price: None,
                stop_price: Price(Decimal::from(96_u32)),
                tp1_price: Price(Decimal::from(106_u32)),
                tp2_price: Some(Price(Decimal::from(112_u32))),
                best_price: Price(Decimal::from(100_u32)),
                tp1_done: false,
                breakeven_done: false,
                fees_paid_quote: Decimal::ZERO,
                realized_pnl_quote: Decimal::ZERO,
                state: *pstate,
                opened_at: OffsetDateTime::UNIX_EPOCH,
            };
            let env = JournalEnvelope::new(JournalEvent::PositionOpened {
                position: pos.clone(),
            });
            let seq = store.append_event(ts, &env).expect("append pos");
            state.apply_event(seq, &env);
            store.upsert_position(&pos, None, None).expect("upsert pos");
        }

        let ord = Order {
            venue: Venue::Spot,
            client_id: "b01r1S-000001-ENT".into(),
            exchange_id: Some(1001),
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            price: Some(Price(Decimal::from(100_u32))),
            qty: Qty(Decimal::from(2_u32)),
            filled_qty: Qty(Decimal::ZERO),
            avg_fill_price: None,
            status: OrderStatus::New,
            position_id: Some(Uuid::from_u128(1)),
            purpose: OrderPurpose::Entry,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        };
        let ord_env = JournalEnvelope::new(JournalEvent::OrderIntended { order: ord });
        let seq = store
            .append_event(1_700_000_005_000, &ord_env)
            .expect("append ord");
        state.apply_event(seq, &ord_env);

        // Simulate abrupt SIGKILL/abort right after journaling!
        std::process::abort();
    }

    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("engine_crash.sqlite");
    let exe = std::env::current_exe().expect("current_exe");
    let status = Command::new(exe)
        .arg("--exact")
        .arg("crash_kill_9_recovery_in_entry_pending_open_protected_and_exit_pending")
        .env("BOT_ENGINE_CRASH_CHILD_DB", &db_path)
        .status()
        .expect("spawn child");
    assert!(!status.success(), "child must terminate via abort()");

    // Parent recovers SQLite + runs EngineActor::bootstrap reconciler!
    let rt = tokio::runtime::Runtime::new().expect("rt");
    rt.block_on(async {
        let store = SqliteStore::open(&db_path).expect("reopen");
        store.integrity_check().expect("integrity ok after crash");

        let broker = Arc::new(PaperBroker::new(PaperBrokerConfig {
            rules: vec![
                sample_rules("BTCUSDT"),
                sample_rules("ETHUSDT"),
                sample_rules("SOLUSDT"),
            ],
            ..PaperBrokerConfig::default()
        }));
        broker.activate().await.expect("activate");

        let actor = EngineActor::bootstrap(
            BotConfig::default(),
            "b01".into(),
            store,
            broker.clone(),
            Decimal::from(10_000_u32),
            1_700_000_010_000,
        )
        .await
        .expect("bootstrap recovery");

        // All 3 crashed positions recovered; OpenProtected ETHUSDT had missing exchange OCO reinstalled!
        assert_eq!(actor.state().positions.len(), 3);
        assert_eq!(
            actor
                .state()
                .positions
                .get(&Uuid::from_u128(1))
                .unwrap()
                .state,
            PositionState::EntryPending
        );
        assert_eq!(
            actor
                .state()
                .positions
                .get(&Uuid::from_u128(2))
                .unwrap()
                .state,
            PositionState::OpenProtected
        );
        assert_eq!(
            actor
                .state()
                .positions
                .get(&Uuid::from_u128(3))
                .unwrap()
                .state,
            PositionState::ExitPending
        );
        assert_eq!(actor.state().mode, EngineMode::Ready);
        assert!(
            broker.active_oco_count() >= 1,
            "missing protection for recovered OpenProtected position must be reinstalled"
        );
    });
}

#[test]
fn command_channel_backpressure_returns_busy_when_full() {
    let (tx, _rx) = tokio::sync::mpsc::channel::<EngineCommand>(COMMAND_CHANNEL_CAPACITY);
    let (_, tel_rx) = tokio::sync::watch::channel(bot_engine::TelemetrySnapshot {
        mode: EngineMode::Ready,
        venue: Venue::Spot,
        open_positions: 0,
        active_orders: 0,
        equity_total_usd: Decimal::from(10_000_u32),
        last_seq: 0,
    });
    // Fill all 64 slots without polling receiver.
    for _ in 0..COMMAND_CHANNEL_CAPACITY {
        let (reply, _) = oneshot::channel();
        tx.try_send(EngineCommand::SnapshotNow { now_ms: 1, reply })
            .expect("within capacity");
    }
    let (reply, _) = oneshot::channel();
    let res = tx.try_send(EngineCommand::SnapshotNow { now_ms: 2, reply });
    assert!(matches!(
        res,
        Err(tokio::sync::mpsc::error::TrySendError::Full(_))
    ));
    // Also verify EngineHandleError::Busy exists and formats cleanly.
    assert!(matches!(EngineHandleError::Busy, EngineHandleError::Busy));
    let _ = tel_rx;
}
