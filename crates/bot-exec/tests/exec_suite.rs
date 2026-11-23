//! Integration and property-based test suite for `bot-exec` (M07 a–f).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::collections::BTreeMap;
use std::time::Duration;

use bot_config::{ExitConfig, OrphanPolicy};
use bot_exchange::{ExecEvent, GatewayEvent, VenueAdapter};
use bot_exec::{
    DualOcoManager, FsmApplyOutcome, HardStopWatcher, LifecycleAction, OrderFsm, PaperBroker,
    PaperBrokerConfig, PassiveEntryOutcome, PassiveEntryPlan, PositionTickInput, ReconcileConfig,
    TopOfBook, compute_exit_pnl_report, evaluate_position_tick, execute_passive_entry,
    reconcile_venue,
};
use bot_store::{PersistedEngineState, SqliteStore};
use bot_types::{
    Direction, InstrumentRules, Order, OrderKind, OrderPurpose, OrderStatus, Position,
    PositionState, Price, Qty, Side, StrategyId, Symbol, Venue,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use time::OffsetDateTime;
use tokio::sync::watch;
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

fn make_position(id: Uuid, entry: u32, stop: u32, tp1: u32, tp2: u32, qty: u32) -> Position {
    let entry_d = Decimal::from(entry);
    let stop_d = Decimal::from(stop);
    Position {
        id,
        symbol: Symbol("BTCUSDT".into()),
        venue: Venue::Spot,
        direction: Direction::Long,
        strategy: StrategyId::TrendPullback,
        entry_price: Price(entry_d),
        qty_initial: Qty(Decimal::from(qty)),
        qty_open: Qty(Decimal::from(qty)),
        r_distance: (entry_d - stop_d).abs(),
        leverage: None,
        liq_price: None,
        stop_price: Price(stop_d),
        tp1_price: Price(Decimal::from(tp1)),
        tp2_price: Some(Price(Decimal::from(tp2))),
        best_price: Price(entry_d),
        tp1_done: false,
        breakeven_done: false,
        fees_paid_quote: Decimal::ZERO,
        realized_pnl_quote: Decimal::ZERO,
        state: PositionState::OpenProtected,
        opened_at: OffsetDateTime::UNIX_EPOCH,
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]
    #[test]
    fn prop_fsm_filled_qty_is_monotonic_and_duplicate_fills_are_idempotent(
        events in prop::collection::vec((1_i64..15, 1_u32..50, 0_u8..3), 1..40)
    ) {
        let mut fsm = OrderFsm::new();
        fsm.register_intended(Order {
            venue: Venue::Spot,
            client_id: "b01r1S-000001-ENT".into(),
            exchange_id: None,
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            price: Some(Price(Decimal::from(100_u32))),
            qty: Qty(Decimal::from(10_000_u32)),
            filled_qty: Qty(Decimal::ZERO),
            avg_fill_price: None,
            status: OrderStatus::PendingNew,
            position_id: None,
            purpose: OrderPurpose::Entry,
            created_at: OffsetDateTime::UNIX_EPOCH,
            updated_at: OffsetDateTime::UNIX_EPOCH,
        });

        let mut prev_filled = Decimal::ZERO;
        let mut seen_trade_ids = std::collections::HashSet::new();

        for (trade_id, qty_step, kind_tag) in events {
            let status = match kind_tag {
                0 => OrderStatus::New,
                _ => OrderStatus::PartiallyFilled,
            };
            let ev = ExecEvent {
                venue: Venue::Spot,
                event_time_ms: 1_700_000_000_000,
                transact_time_ms: 1_700_000_000_000,
                symbol: Symbol("BTCUSDT".into()),
                client_order_id: "b01r1S-000001-ENT".into(),
                orig_client_order_id: None,
                order_id: 100,
                order_list_id: -1,
                trade_id,
                side: Side::Buy,
                kind: OrderKind::LimitMaker,
                exec_type: "TRADE".into(),
                status,
                reject_reason: "NONE".into(),
                order_price: Price(Decimal::from(100_u32)),
                order_qty: Qty(Decimal::from(10_000_u32)),
                last_filled_qty: Qty(Decimal::from(qty_step)),
                cumulative_filled_qty: Qty(prev_filled + Decimal::from(qty_step)),
                last_filled_price: Price(Decimal::from(100_u32)),
                cumulative_quote_qty: (prev_filled + Decimal::from(qty_step)) * Decimal::from(100_u32),
                commission: Decimal::new(1, 2),
                commission_asset: Some("USDT".into()),
                is_maker: true,
            };

            let was_seen = !seen_trade_ids.insert(trade_id);
            let out = fsm.apply_exec_event(&ev, "USDT", None);
            let cur_filled = fsm.get(Venue::Spot, "b01r1S-000001-ENT").unwrap().filled_qty.as_decimal();

            // §5.3 invariant: filled_qty NEVER decreases.
            prop_assert!(cur_filled >= prev_filled);
            if was_seen {
                prop_assert_eq!(out, FsmApplyOutcome::DuplicateFillIgnored);
                prop_assert_eq!(cur_filled, prev_filled);
            }
            prev_filled = cur_filled;
        }
    }

    #[test]
    fn prop_no_open_position_unprotected_beyond_hard_stop_grace(
        grace_ms in 200_u64..3000,
        step_ms in 50_i64..500,
    ) {
        let mut pos = make_position(Uuid::from_u128(1), 100, 95, 107, 115, 4);
        let cfg = ExitConfig {
            hard_stop_grace_ms: grace_ms,
            ..ExitConfig::default()
        };
        let mut watcher = HardStopWatcher::new();

        let mut now_ms = 1_000_000_i64;
        let start_ms = now_ms;
        let mut triggered = false;

        for _ in 0..100 {
            let action = evaluate_position_tick(
                &mut pos,
                PositionTickInput {
                    price: Price(Decimal::from(99_u32)),
                    now_ms,
                    atr: Decimal::from(2_u32),
                    rsi_14: None,
                    bars_in_trade: 1,
                    has_exchange_protection: false,
                    reinstall_failures: 0,
                    tick_size: Decimal::new(1, 2),
                    taker_fee_bps: Decimal::from(10_u32),
                },
                &cfg,
                &mut watcher,
            );
            if matches!(action, LifecycleAction::EmergencyMarketExit { .. }) {
                triggered = true;
                prop_assert!(now_ms - start_ms >= i64::try_from(grace_ms).unwrap());
                break;
            }
            now_ms += step_ms;
        }
        prop_assert!(triggered, "unprotected position must trigger EmergencyMarketExit after grace");
    }
}

#[tokio::test]
async fn e2e_paper_entry_tp1_breakeven_trailing_and_exit() {
    let rules = sample_rules();
    let broker = PaperBroker::new(PaperBrokerConfig {
        rules: vec![rules.clone()],
        ..PaperBrokerConfig::default()
    });
    broker.activate().await.expect("activate");
    let mut exec_rx = broker.take_event_rx().expect("exec_rx");

    let sym = Symbol("BTCUSDT".into());
    // Seed market at bid=100.00, ask=100.50
    broker.on_price_tick(
        &sym,
        Price(Decimal::from(100_u32)),
        Price(Decimal::new(10050, 2)),
        1_000,
    );

    let mut store = SqliteStore::open_in_memory().expect("store");
    let mut fsm = OrderFsm::new();
    let pos_id = Uuid::now_v7();

    let (book_tx, mut book_rx) = watch::channel(TopOfBook {
        bid: Price(Decimal::from(100_u32)),
        ask: Price(Decimal::new(10050, 2)),
        ts_ms: 1_000,
    });

    let plan = PassiveEntryPlan {
        venue: Venue::Spot,
        symbol: sym.clone(),
        bot_id: "b01".into(),
        run_id: 1,
        position_id: pos_id,
        qty: Qty(Decimal::from(4_u32)),
        limit_price_max: Price(Decimal::new(10020, 2)),
        ttl: Duration::from_secs(2),
        max_replaces: 3,
        rules: rules.clone(),
        quote_asset: "USDT".into(),
    };

    // Drive a chase to 100.10 then fill when ask drops to 100.10.
    let broker_ref = &broker;
    let sym_c = sym.clone();
    let driver = async {
        tokio::time::sleep(Duration::from_millis(20)).await;
        broker_ref.on_price_tick(
            &sym_c,
            Price(Decimal::new(10010, 2)),
            Price(Decimal::new(10050, 2)),
            1_100,
        );
        let _ = book_tx.send(TopOfBook {
            bid: Price(Decimal::new(10010, 2)),
            ask: Price(Decimal::new(10050, 2)),
            ts_ms: 1_100,
        });
        tokio::time::sleep(Duration::from_millis(25)).await;
        // Ask drops to 100.10 -> fills resting LIMIT_MAKER Buy at 100.10!
        broker_ref.on_price_tick(
            &sym_c,
            Price(Decimal::new(10005, 2)),
            Price(Decimal::new(10010, 2)),
            1_200,
        );
    };

    let (entry_res, ()) = tokio::join!(
        execute_passive_entry(
            &broker,
            &mut store,
            &mut fsm,
            &plan,
            &mut book_rx,
            &mut exec_rx
        ),
        driver
    );

    let (filled_qty, avg_entry, entry_fees) = match entry_res.expect("entry ok") {
        PassiveEntryOutcome::Filled {
            filled_qty,
            avg_fill_price,
            fees_quote,
            replaces_used,
            ..
        } => {
            assert_eq!(replaces_used, 1, "expected 1 cancelReplace chase");
            (filled_qty, avg_fill_price, fees_quote)
        }
        other => panic!("expected Filled, got {other:?}"),
    };
    assert_eq!(filled_qty.as_decimal(), Decimal::from(4_u32));
    assert_eq!(avg_entry.as_decimal(), Decimal::new(10010, 2));

    // Journal-first check: OrderIntended must be in journal.
    let journal = store.read_journal_after(0).expect("journal");
    assert!(
        journal
            .iter()
            .any(|(_, _, env)| matches!(env.event, bot_store::JournalEvent::OrderIntended { .. })),
        "OrderIntended must be journaled before submission"
    );

    // Install Dual-OCO: OCO-A (2 qty, TP1=106, SL=96) + OCO-B (2 qty, TP2=112, SL=96)
    let mut pos = make_position(pos_id, 100, 96, 106, 112, 4);
    pos.entry_price = avg_entry;
    pos.fees_paid_quote = entry_fees;
    let mut oco_mgr = DualOcoManager::new();
    let prot = oco_mgr
        .install_dual_oco(
            &broker,
            &mut store,
            &pos,
            &rules,
            Decimal::new(5, 1),
            "b01",
            1,
            1_300,
        )
        .await
        .expect("install dual oco");
    assert!(prot.oco_a_list_id.is_some() && prot.oco_b_list_id.is_some());
    assert_eq!(broker.active_oco_count(), 2);

    // Price reaches TP1 (106.00) -> OCO-A fills!
    let tp1_evs = broker.on_price_tick(
        &sym,
        Price(Decimal::from(106_u32)),
        Price(Decimal::new(10610, 2)),
        2_000,
    );
    assert!(
        tp1_evs.iter().any(
            |e| matches!(e, GatewayEvent::ListStatus(ls) if ls.list_order_status == "ALL_DONE")
        )
    );
    let done = oco_mgr.on_list_done(prot.oco_a_list_id.unwrap());
    assert_eq!(done, Some((pos_id, true)));
    pos.tp1_done = true;
    pos.qty_open = Qty(Decimal::from(2_u32));

    // Lifecycle check at 106 (1.5R) -> triggers MoveStop (Break-Even)
    let exit_cfg = ExitConfig::default();
    let mut watcher = HardStopWatcher::new();
    let act_be = evaluate_position_tick(
        &mut pos,
        PositionTickInput {
            price: Price(Decimal::from(106_u32)),
            now_ms: 2_100,
            atr: Decimal::from(1_u32),
            rsi_14: Some(Decimal::from(65_u32)),
            bars_in_trade: 3,
            has_exchange_protection: true,
            reinstall_failures: 0,
            tick_size: rules.tick_size,
            taker_fee_bps: Decimal::from(10_u32),
        },
        &exit_cfg,
        &mut watcher,
    );
    let be_stop = match act_be {
        LifecycleAction::MoveStop {
            new_stop,
            is_breakeven: true,
            ..
        } => {
            assert!(new_stop.as_decimal() > pos.entry_price.as_decimal());
            new_stop
        }
        other => panic!("expected MoveStop(BE), got {other:?}"),
    };
    pos.stop_price = be_stop;
    pos.breakeven_done = true;
    oco_mgr
        .replace_runner_stop(
            &broker, &mut store, &pos, be_stop, true, &rules, "b01", 1, 2_200,
        )
        .await
        .expect("replace OCO-B with BE");

    // Price rallies to 110 with RSI=80 (exhaustion tightening k=3.0*0.7=2.1) -> trailing stop ratchets up
    let act_trail = evaluate_position_tick(
        &mut pos,
        PositionTickInput {
            price: Price(Decimal::from(110_u32)),
            now_ms: 3_000,
            atr: Decimal::from(1_u32),
            rsi_14: Some(Decimal::from(80_u32)),
            bars_in_trade: 5,
            has_exchange_protection: true,
            reinstall_failures: 0,
            tick_size: rules.tick_size,
            taker_fee_bps: Decimal::from(10_u32),
        },
        &exit_cfg,
        &mut watcher,
    );
    let trail_stop = match act_trail {
        LifecycleAction::MoveStop { new_stop, .. } => {
            assert!(new_stop.as_decimal() > be_stop.as_decimal());
            new_stop
        }
        other => panic!("expected trailing MoveStop, got {other:?}"),
    };
    pos.stop_price = trail_stop;
    oco_mgr
        .replace_runner_stop(
            &broker, &mut store, &pos, trail_stop, true, &rules, "b01", 1, 3_100,
        )
        .await
        .expect("replace OCO-B with trail");

    // Price hits TP2 (112) -> runner OCO-B fills!
    let tp2_evs = broker.on_price_tick(
        &sym,
        Price(Decimal::from(112_u32)),
        Price(Decimal::new(11210, 2)),
        4_000,
    );
    assert!(
        tp2_evs.iter().any(
            |e| matches!(e, GatewayEvent::ListStatus(ls) if ls.list_order_status == "ALL_DONE")
        )
    );
    assert_eq!(broker.active_oco_count(), 0);
    assert_eq!(broker.open_order_count(), 0);

    let report = compute_exit_pnl_report(
        Direction::Long,
        pos.entry_price,
        Price(Decimal::from(112_u32)),
        Qty(Decimal::from(2_u32)),
        pos.qty_initial,
        pos.r_distance,
        Decimal::new(50, 2),
    );
    assert!(report.net_pnl_quote > Decimal::ZERO);
    assert!(report.r_multiple > Decimal::ZERO);
}

#[tokio::test]
async fn e2e_paper_stop_loss_and_time_stop_scenarios() {
    let rules = sample_rules();
    let broker = PaperBroker::new(PaperBrokerConfig {
        rules: vec![rules.clone()],
        ..PaperBrokerConfig::default()
    });
    broker.activate().await.expect("activate");
    let sym = Symbol("BTCUSDT".into());
    broker.inject_orphan_balance("BTC", Decimal::from(4_u32));

    let mut store = SqliteStore::open_in_memory().expect("store");
    let mut oco_mgr = DualOcoManager::new();
    let pos = make_position(Uuid::now_v7(), 100, 96, 106, 112, 4);
    oco_mgr
        .install_dual_oco(
            &broker,
            &mut store,
            &pos,
            &rules,
            Decimal::new(5, 1),
            "b01",
            1,
            1_000,
        )
        .await
        .expect("install dual oco");
    assert_eq!(broker.active_oco_count(), 2);

    // Scenario A: Price drops to 96 -> both OCO SL legs trigger and exit cleanly.
    let sl_evs = broker.on_price_tick(
        &sym,
        Price(Decimal::from(96_u32)),
        Price(Decimal::new(9610, 2)),
        2_000,
    );
    assert!(sl_evs.len() >= 4);
    assert_eq!(broker.active_oco_count(), 0);
    assert_eq!(broker.open_order_count(), 0);

    // Scenario B: Time stop after max_bars_in_trade (48) with |profit_R| < 0.3
    let mut stagnant_pos = make_position(Uuid::now_v7(), 100, 96, 106, 112, 4);
    let mut watcher = HardStopWatcher::new();
    let act = evaluate_position_tick(
        &mut stagnant_pos,
        PositionTickInput {
            price: Price(Decimal::new(10050, 2)), // +0.125R < 0.3R
            now_ms: 5_000,
            atr: Decimal::from(1_u32),
            rsi_14: Some(Decimal::from(50_u32)),
            bars_in_trade: 48,
            has_exchange_protection: true,
            reinstall_failures: 0,
            tick_size: rules.tick_size,
            taker_fee_bps: Decimal::from(10_u32),
        },
        &ExitConfig::default(),
        &mut watcher,
    );
    assert_eq!(act, LifecycleAction::TimeStopExit { bars_in_trade: 48 });
}

#[tokio::test]
async fn reconciler_handles_orphan_orders_and_balances_per_orphan_policy() {
    let rules = sample_rules();
    let sym = Symbol("BTCUSDT".into());
    let mut rules_map = BTreeMap::new();
    rules_map.insert(sym.clone(), rules.clone());
    let mut ref_prices = BTreeMap::new();
    ref_prices.insert(sym.clone(), Price(Decimal::from(100_u32)));

    // 1. OrphanPolicy::Flatten cancels orphan order and flattens orphan balance.
    let broker_flat = PaperBroker::new(PaperBrokerConfig {
        rules: vec![rules.clone()],
        ..PaperBrokerConfig::default()
    });
    broker_flat.activate().await.expect("activate");
    broker_flat.on_price_tick(
        &sym,
        Price(Decimal::from(100_u32)),
        Price(Decimal::from(101_u32)),
        1_000,
    );
    broker_flat.inject_orphan_order(
        sym.clone(),
        "manual-orphan-1".into(),
        Side::Buy,
        OrderKind::Limit,
        Price(Decimal::from(90_u32)),
        Qty(Decimal::from(1_u32)),
    );
    broker_flat.inject_orphan_balance("BTC", Decimal::from(2_u32));

    let mut store = SqliteStore::open_in_memory().expect("store");
    let mut state = PersistedEngineState::default();
    let mut fsm = OrderFsm::new();
    let mut oco_mgr = DualOcoManager::new();

    let rep_flat = reconcile_venue(
        &broker_flat,
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
    .expect("reconcile flatten");

    assert!(rep_flat.material_mismatch);
    assert_eq!(rep_flat.orphan_orders_handled, 1);
    assert_eq!(rep_flat.orphan_balances_handled, 1);
    assert_eq!(broker_flat.open_order_count(), 0);

    // 2. OrphanPolicy::AdoptWithStop adopts orphan balance as protected position with Dual-OCO.
    let broker_adopt = PaperBroker::new(PaperBrokerConfig {
        rules: vec![rules],
        ..PaperBrokerConfig::default()
    });
    broker_adopt.activate().await.expect("activate");
    broker_adopt.inject_orphan_balance("BTC", Decimal::from(2_u32));

    let mut store2 = SqliteStore::open_in_memory().expect("store2");
    let mut state2 = PersistedEngineState::default();
    let mut fsm2 = OrderFsm::new();
    let mut oco_mgr2 = DualOcoManager::new();

    let rep_adopt = reconcile_venue(
        &broker_adopt,
        &mut store2,
        &mut state2,
        &mut fsm2,
        &mut oco_mgr2,
        &rules_map,
        &ref_prices,
        &ReconcileConfig {
            venue: Venue::Spot,
            quote_asset: "USDT".into(),
            orphan_policy: OrphanPolicy::AdoptWithStop,
            bot_id: "b01".into(),
            run_id: 1,
            tp1_close_ratio: Decimal::new(5, 1),
            adopt_stop_pct: Decimal::from(2_u32),
        },
        2_000,
    )
    .await
    .expect("reconcile adopt");

    assert_eq!(rep_adopt.orphan_balances_handled, 1);
    assert_eq!(state2.positions.len(), 1);
    assert!(broker_adopt.active_oco_count() >= 1);
}
