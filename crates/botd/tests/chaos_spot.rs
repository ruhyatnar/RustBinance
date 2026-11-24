//! Automated Spot Chaos Suite covering all 12 Spot scenarios in §13.4
//! plus live `binance-botd` `sd_notify` + `SIGTERM` lifecycle verification (M13).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::doc_markdown
)]

use std::collections::BTreeMap;
use std::os::unix::net::UnixDatagram;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use bot_config::{BotConfig, ExitConfig, OrphanPolicy};
use bot_engine::EngineActor;
use bot_exchange::{EventDeduplicator, GatewayError, VenueAdapter};
use bot_exec::{
    DualOcoManager, HardStopWatcher, LifecycleAction, OrderFsm, PaperBroker, PaperBrokerConfig,
    PositionTickInput, ReconcileConfig, evaluate_position_tick, reconcile_venue,
};
use bot_marketdata::{
    BookStatus, DepthDiffEvent, DepthLevel, DepthSnapshot, DiffApplyResult, LocalOrderBook,
};
use bot_risk::{BtcCrashGuard, BtcMarketSample, EngineModeMachine, ModeTrigger};
use bot_store::{JournalEnvelope, JournalEvent, PersistedEngineState, SqliteStore};
use bot_types::{
    Direction, EngineMode, InstrumentRules, Order, OrderKind, OrderPurpose, OrderStatus, Position,
    PositionState, Price, Qty, Side, StrategyId, Symbol, Venue,
};
use rust_decimal::Decimal;
use tempfile::TempDir;
use time::OffsetDateTime;
use uuid::Uuid;

fn sample_rules() -> InstrumentRules {
    InstrumentRules {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
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

/// §13.4 #1: Putus WS market (stale > 1500 ms) → Paused; tidak ada entry saat basi.
#[test]
fn chaos_01_market_ws_disconnect_pauses_within_1500ms() {
    let mut mode = EngineModeMachine::new(0);
    mode.apply(ModeTrigger::SyncStarted, 100);
    mode.apply(ModeTrigger::SyncComplete, 1_000);
    assert_eq!(mode.mode(), EngineMode::Ready);
    assert!(mode.allows_new_entries());

    // Market stream stale > 1500 ms triggers Paused; no new entries permitted.
    mode.apply(
        ModeTrigger::ReconnectOrSyncNeeded {
            reason: "market WS stale > 1500ms".into(),
        },
        2_600,
    );
    assert_eq!(mode.mode(), EngineMode::Paused);
    assert!(!mode.allows_new_entries());

    // Stable for >= 3 s recovers back to Ready.
    mode.apply(ModeTrigger::StableTick { now_ms: 3_000 }, 3_000);
    mode.apply(ModeTrigger::StableTick { now_ms: 6_500 }, 6_500);
    assert_eq!(mode.mode(), EngineMode::Ready);
}

/// §13.4 #2: Putus WS API saat order in-flight → UnknownNeedsQuery → query by clientOrderId; no duplicate order.
#[test]
fn chaos_02_ws_api_disconnect_in_flight_resolves_unknown_without_duplicate() {
    let mut fsm = OrderFsm::new();
    let cid = "b01r1S-000001-ENT".to_owned();
    fsm.register_intended(Order {
        venue: Venue::Spot,
        client_id: cid.clone(),
        exchange_id: None,
        symbol: Symbol("BTCUSDT".into()),
        side: Side::Buy,
        kind: OrderKind::LimitMaker,
        price: Some(Price(Decimal::from(100_u32))),
        qty: Qty(Decimal::from(2_u32)),
        filled_qty: Qty(Decimal::ZERO),
        avg_fill_price: None,
        status: OrderStatus::PendingNew,
        position_id: None,
        purpose: OrderPurpose::Entry,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
    });

    // Transport timeout / disconnect marks UnknownNeedsQuery.
    assert_eq!(
        fsm.mark_unknown_needs_query(Venue::Spot, &cid, OffsetDateTime::UNIX_EPOCH),
        Some(OrderStatus::UnknownNeedsQuery)
    );

    // 3 consecutive NotFound misses resolve safely to Rejected (no blind retry).
    assert_eq!(
        fsm.record_unknown_query_not_found(Venue::Spot, &cid, OffsetDateTime::UNIX_EPOCH),
        Some(OrderStatus::UnknownNeedsQuery)
    );
    assert_eq!(
        fsm.record_unknown_query_not_found(Venue::Spot, &cid, OffsetDateTime::UNIX_EPOCH),
        Some(OrderStatus::UnknownNeedsQuery)
    );
    assert_eq!(
        fsm.record_unknown_query_not_found(Venue::Spot, &cid, OffsetDateTime::UNIX_EPOCH),
        Some(OrderStatus::Rejected)
    );
    assert_eq!(
        fsm.get(Venue::Spot, &cid).unwrap().status,
        OrderStatus::Rejected
    );
}

/// §13.4 #3: Rotasi 24 jam / serverShutdown → tanpa kehilangan event; dedup fill.
#[test]
fn chaos_03_24h_rotation_and_server_shutdown_dedup_fills() {
    let mut dedup = EventDeduplicator::new(1024);
    // Same (order_id, trade_id) arriving across overlapping old+new WS sessions is deduplicated.
    assert!(dedup.check_execution_report(1001, 50001, 1_000, "TRADE"));
    assert!(!dedup.check_execution_report(1001, 50001, 1_000, "TRADE"));
    // Next trade on same order is accepted.
    assert!(dedup.check_execution_report(1001, 50002, 1_100, "TRADE"));
}

/// §13.4 #4: kill -9 saat EntryPending, OpenProtected, ExitPending → recovery state benar & proteksi terpasang.
#[tokio::test]
async fn chaos_04_crash_recovery_preserves_states_and_reinstalls_protection() {
    let dir = TempDir::new().expect("tmpdir");
    let db_path = dir.path().join("chaos04.db");
    let rules = sample_rules();

    {
        let mut store = SqliteStore::open(&db_path).expect("open store");
        for (id_u, state) in [
            (1_u128, PositionState::EntryPending),
            (2_u128, PositionState::OpenProtected),
            (3_u128, PositionState::ExitPending),
        ] {
            let pos = Position {
                id: Uuid::from_u128(id_u),
                symbol: Symbol("BTCUSDT".into()),
                venue: Venue::Spot,
                direction: Direction::Long,
                strategy: StrategyId::TrendPullback,
                entry_price: Price(Decimal::from(100_u32)),
                qty_initial: Qty(Decimal::from(2_u32)),
                qty_open: Qty(Decimal::from(2_u32)),
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
                state,
                opened_at: OffsetDateTime::UNIX_EPOCH,
            };
            store
                .append_event(
                    1_000,
                    &JournalEnvelope::new(JournalEvent::PositionOpened { position: pos }),
                )
                .expect("append");
        }
    }

    let broker = Arc::new(PaperBroker::new(PaperBrokerConfig {
        rules: vec![rules],
        initial_quote_balance: Decimal::from(10_000_u32),
        ..PaperBrokerConfig::default()
    }));
    broker.activate().await.expect("activate");
    broker.inject_orphan_balance("BTC", Decimal::from(2_u32));

    let store = SqliteStore::open(&db_path).expect("reopen");
    store.integrity_check().expect("integrity ok");
    let actor = EngineActor::bootstrap(
        BotConfig::default(),
        "b01".into(),
        store,
        broker.clone(),
        Decimal::from(10_000_u32),
        2_000,
    )
    .await
    .expect("bootstrap");

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
    assert!(broker.active_oco_count() >= 1);
}

/// §13.4 #5: Fill ganda / event tidak berurutan → idempoten (kunci tradeId).
#[test]
fn chaos_05_duplicate_and_out_of_order_fills_are_idempotent() {
    let mut fsm = OrderFsm::new();
    let cid = "b01r1S-000002-ENT".to_owned();
    fsm.register_intended(Order {
        venue: Venue::Spot,
        client_id: cid.clone(),
        exchange_id: Some(77),
        symbol: Symbol("BTCUSDT".into()),
        side: Side::Buy,
        kind: OrderKind::LimitMaker,
        price: Some(Price(Decimal::from(100_u32))),
        qty: Qty(Decimal::from(2_u32)),
        filled_qty: Qty(Decimal::ZERO),
        avg_fill_price: None,
        status: OrderStatus::New,
        position_id: None,
        purpose: OrderPurpose::Entry,
        created_at: OffsetDateTime::UNIX_EPOCH,
        updated_at: OffsetDateTime::UNIX_EPOCH,
    });

    let fill_ev = bot_exchange::ExecEvent {
        venue: Venue::Spot,
        event_time_ms: 1_000,
        transact_time_ms: 1_000,
        symbol: Symbol("BTCUSDT".into()),
        client_order_id: cid.clone(),
        orig_client_order_id: None,
        order_id: 77,
        order_list_id: -1,
        trade_id: 9001,
        side: Side::Buy,
        kind: OrderKind::LimitMaker,
        exec_type: "TRADE".into(),
        status: OrderStatus::PartiallyFilled,
        reject_reason: "NONE".into(),
        order_price: Price(Decimal::from(100_u32)),
        order_qty: Qty(Decimal::from(2_u32)),
        last_filled_qty: Qty(Decimal::ONE),
        cumulative_filled_qty: Qty(Decimal::ONE),
        last_filled_price: Price(Decimal::from(100_u32)),
        cumulative_quote_qty: Decimal::from(100_u32),
        commission: Decimal::ZERO,
        commission_asset: Some("USDT".into()),
        is_maker: true,
    };

    let first = fsm.apply_exec_event(&fill_ev, "USDT", None);
    let dup = fsm.apply_exec_event(&fill_ev, "USDT", None);
    assert!(matches!(first, bot_exec::FsmApplyOutcome::Updated { .. }));
    assert!(matches!(
        dup,
        bot_exec::FsmApplyOutcome::DuplicateFillIgnored
    ));
    assert_eq!(
        fsm.get(Venue::Spot, &cid).unwrap().filled_qty,
        Qty(Decimal::ONE)
    );
}

/// §13.4 #6: 429 lalu 418 (mock) → back-off sesuai retryAfter; tidak spam.
#[test]
fn chaos_06_rate_limit_429_and_ip_ban_418_respect_retry_after() {
    let err_429 = GatewayError::from_exchange_error(
        429,
        Some(-1003),
        "Too many requests".into(),
        Some(12_000),
        None,
    );
    assert!(matches!(
        err_429,
        GatewayError::RateLimited {
            status: 429,
            retry_after_ms: Some(12_000),
            ..
        }
    ));

    let err_418 = GatewayError::from_exchange_error(
        418,
        Some(-1003),
        "IP auto-banned".into(),
        Some(120_000),
        None,
    );
    assert!(matches!(
        err_418,
        GatewayError::IpBanned {
            retry_after_ms: Some(120_000),
            ..
        }
    ));
}

/// §13.4 #7: Drift jam > 800 ms → ReduceOnly/Paused + alarm; -1021 tidak memicu loop retry.
#[test]
fn chaos_07_clock_drift_pauses_and_1021_is_non_retryable() {
    let err_1021 = GatewayError::from_exchange_error(
        400,
        Some(-1021),
        "Timestamp for this request is outside recvWindow".into(),
        None,
        None,
    );
    assert!(matches!(
        err_1021,
        GatewayError::TimestampOutOfWindow { .. }
    ));

    let mut mode = EngineModeMachine::new(0);
    mode.apply(ModeTrigger::SyncStarted, 100);
    mode.apply(ModeTrigger::SyncComplete, 1_000);
    mode.apply(ModeTrigger::ClockDriftExceeded, 2_000);
    assert!(!mode.allows_new_entries());
}

/// §13.4 #8: Gap pada depth stream → Book Invalid → resync → OBI tidak dipakai selama invalid.
#[test]
fn chaos_08_depth_stream_gap_invalidates_book_until_resync() {
    let mut book = LocalOrderBook::new(Venue::Spot, Symbol("BTCUSDT".into()));
    book.apply_snapshot(&DepthSnapshot {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        last_update_id: 100,
        bids: vec![DepthLevel {
            price: Price(Decimal::from(100_u32)),
            qty: Qty(Decimal::from(5_u32)),
        }],
        asks: vec![DepthLevel {
            price: Price(Decimal::from(101_u32)),
            qty: Qty(Decimal::from(5_u32)),
        }],
    });
    assert_eq!(book.status(), BookStatus::Synced);
    assert!(book.obi_raw().is_some());

    // Gap: first_update_id = 105 > last_update_id + 1 (101) → Invalid.
    let res = book.apply_diff(DepthDiffEvent {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        first_update_id: 105,
        final_update_id: 106,
        prev_final_update_id: None,
        event_time_ms: 2_000,
        bids: vec![],
        asks: vec![],
    });
    assert_eq!(res, DiffApplyResult::GapDetectedNeedsResync);
    assert_eq!(book.status(), BookStatus::Invalid);
    assert!(
        book.obi_raw().is_none(),
        "OBI must not be used while book is Invalid"
    );
}

/// §13.4 #9: Proteksi OCO dibatalkan manual → terdeteksi → pasang ulang <= 2 s atau flatten.
#[test]
fn chaos_09_manual_oco_cancel_triggers_reinstall_then_emergency_flatten() {
    let mut pos = Position {
        id: Uuid::now_v7(),
        symbol: Symbol("BTCUSDT".into()),
        venue: Venue::Spot,
        direction: Direction::Long,
        strategy: StrategyId::TrendPullback,
        entry_price: Price(Decimal::from(100_u32)),
        qty_initial: Qty(Decimal::from(2_u32)),
        qty_open: Qty(Decimal::from(2_u32)),
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
        state: PositionState::OpenProtected,
        opened_at: OffsetDateTime::UNIX_EPOCH,
    };
    let exit_cfg = ExitConfig::default();
    let mut watcher = HardStopWatcher::new();

    // First tick without protection → ReinstallProtection.
    let act1 = evaluate_position_tick(
        &mut pos,
        PositionTickInput {
            price: Price(Decimal::from(100_u32)),
            now_ms: 1_000,
            atr: Decimal::from(2_u32),
            rsi_14: Some(Decimal::from(50_u32)),
            bars_in_trade: 1,
            has_exchange_protection: false,
            reinstall_failures: 0,
            tick_size: Decimal::new(1, 2),
            taker_fee_bps: Decimal::from(10_u32),
        },
        &exit_cfg,
        &mut watcher,
    );
    assert_eq!(act1, LifecycleAction::ReinstallProtection);

    // After 2 reinstall failures (or > hard_stop_grace_ms) → EmergencyMarketExit.
    let act2 = evaluate_position_tick(
        &mut pos,
        PositionTickInput {
            price: Price(Decimal::from(100_u32)),
            now_ms: 1_500,
            atr: Decimal::from(2_u32),
            rsi_14: Some(Decimal::from(50_u32)),
            bars_in_trade: 1,
            has_exchange_protection: false,
            reinstall_failures: 2,
            tick_size: Decimal::new(1, 2),
            taker_fee_bps: Decimal::from(10_u32),
        },
        &exit_cfg,
        &mut watcher,
    );
    assert!(matches!(act2, LifecycleAction::EmergencyMarketExit { .. }));
}

/// §13.4 #10: Saldo/order berubah manual di app Binance → rekonsiliasi mendeteksi → orphan policy.
#[tokio::test]
async fn chaos_10_manual_exchange_changes_detected_by_reconcile_orphan_policy() {
    let rules = sample_rules();
    let broker = PaperBroker::new(PaperBrokerConfig {
        rules: vec![rules.clone()],
        ..PaperBrokerConfig::default()
    });
    broker.activate().await.expect("activate");
    broker.inject_orphan_order(
        Symbol("BTCUSDT".into()),
        "manual-app-order".into(),
        Side::Buy,
        OrderKind::Limit,
        Price(Decimal::from(90_u32)),
        Qty(Decimal::ONE),
    );
    broker.inject_orphan_balance("BTC", Decimal::from(2_u32));

    let mut store = SqliteStore::open_in_memory().expect("store");
    let mut state = PersistedEngineState::default();
    let mut fsm = OrderFsm::new();
    let mut oco_mgr = DualOcoManager::new();
    let mut rules_map = BTreeMap::new();
    rules_map.insert(Symbol("BTCUSDT".into()), rules);
    let mut ref_prices = BTreeMap::new();
    ref_prices.insert(Symbol("BTCUSDT".into()), Price(Decimal::from(100_u32)));

    let rep = reconcile_venue(
        &broker,
        &mut store,
        &mut state,
        &mut fsm,
        &mut oco_mgr,
        &rules_map,
        &ref_prices,
        &ReconcileConfig {
            venue: Venue::Spot,
            quote_asset: "USDT".into(),
            orphan_policy: OrphanPolicy::Flatten,
            bot_id: "b01".into(),
            run_id: 1,
            tp1_close_ratio: Decimal::new(5, 1),
            adopt_stop_pct: Decimal::from(2_u32),
        },
        2_000,
    )
    .await
    .expect("reconcile");

    assert!(rep.material_mismatch);
    assert_eq!(rep.orphan_orders_handled, 1);
    assert_eq!(rep.orphan_balances_handled, 1);
    assert_eq!(broker.open_order_count(), 0);
}

/// §13.4 #11: Disk penuh / SQLite error → Halted (tidak trading tanpa journal).
#[test]
fn chaos_11_storage_error_halts_engine() {
    let mut mode = EngineModeMachine::new(0);
    mode.apply(ModeTrigger::SyncStarted, 100);
    mode.apply(ModeTrigger::SyncComplete, 1_000);
    mode.apply(
        ModeTrigger::StorageFatalError {
            reason: "SQLITE_FULL".into(),
        },
        2_000,
    );
    assert_eq!(mode.mode(), EngineMode::Halted);
    assert!(!mode.allows_new_entries());
}

/// §13.4 #12: Flash crash BTC (replay data) → Crash Guard aktif, histeresis benar.
#[test]
fn chaos_12_btc_flash_crash_activates_guard_with_hysteresis() {
    let cfg = BotConfig::default();
    let mut guard = BtcCrashGuard::new(cfg.risk.btc_crash_15m_pct, cfg.risk.btc_crash_60m_pct);

    // -5% drop in 15m triggers Crash Guard.
    let act1 = guard.update(BtcMarketSample {
        now_ms: 15 * 60 * 1_000,
        price: Decimal::from(57_000_u32),
        drop_15m_pct: Decimal::from(5_u32),
        drop_60m_pct: Decimal::from(5_u32),
        reference_high_price: Decimal::from(60_000_u32),
        atr_1m_pct: Decimal::new(15, 3),
        median_atr_24h_pct: Decimal::new(3, 3),
    });
    assert!(act1.active);

    // Immediate bounce 5 minutes later stays blocked due to 30m hysteresis.
    let act2 = guard.update(BtcMarketSample {
        now_ms: 20 * 60 * 1_000,
        price: Decimal::from(60_000_u32),
        drop_15m_pct: Decimal::ZERO,
        drop_60m_pct: Decimal::ZERO,
        reference_high_price: Decimal::from(60_000_u32),
        atr_1m_pct: Decimal::new(2, 3),
        median_atr_24h_pct: Decimal::new(3, 3),
    });
    assert!(act2.active);
}

/// M13 daemon E2E: `binance-botd --config` emits `READY=1` on `NOTIFY_SOCKET`,
/// handles `SIGTERM` cleanly, emits `STOPPING=1`, and exits 0.
#[test]
fn daemon_sd_notify_ready_and_sigterm_graceful_shutdown() {
    let dir = TempDir::new().expect("tmpdir");
    let notify_sock = dir.path().join("notify.sock");
    let receiver = UnixDatagram::bind(&notify_sock).expect("bind notify sock");
    receiver
        .set_read_timeout(Some(Duration::from_secs(5)))
        .expect("timeout");

    let uds_sock = dir.path().join("bot.sock");
    let db_path = dir.path().join("bot.db");
    let backup_dir = dir.path().join("backup");
    let cfg_path = dir.path().join("bot.toml");
    std::fs::write(
        &cfg_path,
        format!(
            "[general]\nmode = \"paper\"\n\n[ipc]\nuds_path = \"{}\"\n\n[storage]\ndb_path = \"{}\"\nbackup_dir = \"{}\"\n",
            uds_sock.display(),
            db_path.display(),
            backup_dir.display()
        ),
    )
    .expect("write config");

    let bin = env!("CARGO_BIN_EXE_binance-botd");
    let mut child = Command::new(bin)
        .arg("--config")
        .arg(&cfg_path)
        .env("NOTIFY_SOCKET", &notify_sock)
        .env("RUST_LOG", "info")
        .spawn()
        .expect("spawn binance-botd");

    let mut buf = [0_u8; 64];
    let n = receiver.recv(&mut buf).expect("receive READY=1");
    assert_eq!(std::str::from_utf8(&buf[..n]).expect("utf8"), "READY=1");

    // Send SIGTERM via `kill -TERM <pid>` (zero unsafe).
    let status_kill = Command::new("kill")
        .args(["-TERM", &child.id().to_string()])
        .status()
        .expect("send SIGTERM");
    assert!(status_kill.success());

    let n2 = receiver.recv(&mut buf).expect("receive STOPPING=1");
    assert_eq!(std::str::from_utf8(&buf[..n2]).expect("utf8"), "STOPPING=1");

    let exit = child.wait().expect("wait child");
    assert!(exit.success(), "binance-botd must exit 0 on SIGTERM");
}
