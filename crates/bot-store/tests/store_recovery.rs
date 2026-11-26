//! Persistence, replay property tests, child-process `abort()` crash recovery,
//! idempotent migration, online `.backup`, and I/O fault tests for `bot-store` (M05).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::Command;

use bot_store::{
    EVENT_SCHEMA_VERSION, FillRecord, JournalEnvelope, JournalEvent, PersistedEngineState,
    SqliteStore, StoreError, VenueStateRecord,
};
use bot_types::{
    Direction, EngineMode, Order, OrderKind, OrderPurpose, OrderStatus, Position, PositionState,
    Price, Qty, Side, StrategyId, Symbol, Venue, VenueState,
};
use proptest::prelude::*;
use rust_decimal::Decimal;
use tempfile::TempDir;
use time::OffsetDateTime;
use uuid::Uuid;

fn sample_dt(secs: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(secs).expect("valid ts")
}

fn sample_order(venue: Venue, client_id: &str, price: i64, qty: i64) -> Order {
    Order {
        venue,
        client_id: client_id.to_owned(),
        exchange_id: None,
        symbol: Symbol("BTCUSDT".into()),
        side: Side::Buy,
        kind: OrderKind::LimitMaker,
        price: Some(Price(Decimal::new(price, 2))),
        qty: Qty(Decimal::new(qty, 4)),
        filled_qty: Qty(Decimal::ZERO),
        avg_fill_price: None,
        status: OrderStatus::PendingNew,
        position_id: None,
        purpose: OrderPurpose::Entry,
        created_at: sample_dt(1_700_000_000),
        updated_at: sample_dt(1_700_000_000),
    }
}

fn sample_position(id: Uuid, venue: Venue, entry: i64, stop: i64) -> Position {
    Position {
        id,
        symbol: Symbol("BTCUSDT".into()),
        venue,
        direction: Direction::Long,
        strategy: StrategyId::TrendPullback,
        entry_price: Price(Decimal::new(entry, 2)),
        qty_initial: Qty(Decimal::new(100, 4)),
        qty_open: Qty(Decimal::new(100, 4)),
        r_distance: Decimal::new((entry - stop).abs().max(1), 2),
        leverage: if venue == Venue::UsdsFutures {
            Some(3)
        } else {
            None
        },
        liq_price: None,
        stop_price: Price(Decimal::new(stop, 2)),
        tp1_price: Price(Decimal::new(entry + 500, 2)),
        tp2_price: Some(Price(Decimal::new(entry + 1000, 2))),
        best_price: Price(Decimal::new(entry, 2)),
        tp1_done: false,
        breakeven_done: false,
        fees_paid_quote: Decimal::ZERO,
        realized_pnl_quote: Decimal::ZERO,
        state: PositionState::OpenProtected,
        opened_at: sample_dt(1_700_000_000),
    }
}

#[test]
fn migrations_are_idempotent_and_crud_counters_backup_work() {
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("bot.sqlite");

    // First open creates schema and migrates cleanly.
    {
        let mut store = SqliteStore::open(&db_path).expect("open empty db");
        store.integrity_check().expect("integrity ok");

        assert_eq!(store.next_run_id().expect("run 1"), 1);
        assert_eq!(store.next_order_seq().expect("seq 1"), 1);
        assert_eq!(store.next_order_seq().expect("seq 2"), 2);

        let ord = sample_order(Venue::Spot, "b01r1S-000001-ENT", 6_000_000, 150);
        store.upsert_order(&ord).expect("upsert_order");
        assert_eq!(
            store
                .get_order(Venue::Spot, "b01r1S-000001-ENT")
                .expect("get_order"),
            Some(ord)
        );
        // Same client_id on different venue must be isolated!
        assert_eq!(
            store
                .get_order(Venue::UsdsFutures, "b01r1S-000001-ENT")
                .expect("get_order futures"),
            None
        );

        let fill = FillRecord {
            venue: Venue::Spot,
            trade_id: 9001,
            symbol: Symbol("BTCUSDT".into()),
            client_id: "b01r1S-000001-ENT".into(),
            price: Price(Decimal::new(6_000_000, 2)),
            qty: Qty(Decimal::new(150, 4)),
            fee: Decimal::new(90, 2),
            fee_asset: "USDT".into(),
            is_maker: true,
            ts_ms: 1_700_000_001_000,
        };
        store.upsert_fill(&fill).expect("upsert_fill 1");
        store.upsert_fill(&fill).expect("upsert_fill idempotent");
        assert_eq!(
            store
                .list_fills(Venue::Spot, &Symbol("BTCUSDT".into()))
                .expect("list_fills"),
            vec![fill]
        );

        let pos_id = Uuid::from_u128(42);
        let pos = sample_position(pos_id, Venue::Spot, 6_000_000, 5_900_000);
        store
            .upsert_position(&pos, None, None)
            .expect("upsert_position");
        assert_eq!(store.get_position(pos_id).expect("get_position"), Some(pos));

        store
            .risk_state_set("peak_equity", "10500.25")
            .expect("risk_state_set");
        assert_eq!(
            store.risk_state_get("peak_equity").expect("risk_state_get"),
            Some("10500.25".into())
        );

        let vs = VenueStateRecord {
            venue: Venue::Spot,
            state: VenueState::Active,
            since_ms: 1_700_000_000_000,
            last_switch_ms: None,
            switches_today: 0,
        };
        store.venue_state_set(&vs).expect("venue_state_set");
        assert_eq!(
            store.venue_state_get(Venue::Spot).expect("venue_state_get"),
            Some(vs)
        );

        let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
            mode: EngineMode::Ready,
            reason: "boot".into(),
        });
        let seq = store.append_event(1_700_000_005_000, &env).expect("append");
        assert_eq!(seq, 1);

        // Online backup (.backup)
        let backup_path = dir.path().join("backup").join("bot-daily.sqlite");
        store.backup_to(&backup_path).expect("online backup");
        let backup_store = SqliteStore::open(&backup_path).expect("open backup");
        backup_store.integrity_check().expect("backup integrity ok");
        assert_eq!(
            backup_store.recover_state().expect("backup recover").mode,
            EngineMode::Ready
        );
    }

    // Reopening existing DB is idempotent and preserves counters.
    let mut reopened = SqliteStore::open(&db_path).expect("reopen db");
    reopened.integrity_check().expect("reopened integrity ok");
    assert_eq!(reopened.next_run_id().expect("run 2"), 2);
    assert_eq!(reopened.next_order_seq().expect("seq 3"), 3);
}

#[derive(Debug, Clone)]
enum StepOp {
    OrderIntended {
        venue_spot: bool,
        idx: u8,
        price: i64,
        qty: i64,
    },
    OrderAck {
        venue_spot: bool,
        idx: u8,
        filled: i64,
    },
    Fill {
        venue_spot: bool,
        trade_id: i64,
        qty: i64,
    },
    OpenPos {
        venue_spot: bool,
        pos_idx: u8,
        entry: i64,
    },
    MoveStop {
        pos_idx: u8,
        new_stop: i64,
        be: bool,
    },
    ClosePos {
        pos_idx: u8,
        pnl: i64,
    },
    SetMode(u8),
    SetRisk(u8, i64),
    SetVenue {
        spot: bool,
        active: bool,
    },
    TakeSnapshot,
}

fn step_op_strategy() -> impl Strategy<Value = StepOp> {
    prop_oneof![
        (any::<bool>(), 0u8..6, 100_000i64..9_000_000, 10i64..5000).prop_map(
            |(venue_spot, idx, price, qty)| StepOp::OrderIntended {
                venue_spot,
                idx,
                price,
                qty
            }
        ),
        (any::<bool>(), 0u8..6, 0i64..5000).prop_map(|(venue_spot, idx, filled)| {
            StepOp::OrderAck {
                venue_spot,
                idx,
                filled,
            }
        }),
        (any::<bool>(), 1i64..20, 1i64..1000).prop_map(|(venue_spot, trade_id, qty)| {
            StepOp::Fill {
                venue_spot,
                trade_id,
                qty,
            }
        }),
        (any::<bool>(), 0u8..4, 200_000i64..8_000_000).prop_map(|(venue_spot, pos_idx, entry)| {
            StepOp::OpenPos {
                venue_spot,
                pos_idx,
                entry,
            }
        }),
        (0u8..4, 100_000i64..7_000_000, any::<bool>()).prop_map(|(pos_idx, new_stop, be)| {
            StepOp::MoveStop {
                pos_idx,
                new_stop,
                be,
            }
        }),
        (0u8..4, -50_000i64..50_000).prop_map(|(pos_idx, pnl)| StepOp::ClosePos { pos_idx, pnl }),
        (0u8..5).prop_map(StepOp::SetMode),
        (0u8..5, 0i64..100_000).prop_map(|(k, v)| StepOp::SetRisk(k, v)),
        (any::<bool>(), any::<bool>()).prop_map(|(spot, active)| StepOp::SetVenue { spot, active }),
        Just(StepOp::TakeSnapshot),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(32))]
    #[test]
    fn prop_replay_snapshot_plus_journal_equals_live_state(ops in prop::collection::vec(step_op_strategy(), 1..60)) {
        let mut store = SqliteStore::open_in_memory().expect("in-memory store");
        let mut expected = PersistedEngineState::default();
        let mut ts_ms: i64 = 1_700_000_000_000;

        for op in ops {
            ts_ms += 1000;
            let event_opt = match op {
                StepOp::TakeSnapshot => {
                    store.snapshot_write(ts_ms, &expected).expect("snapshot_write");
                    None
                }
                StepOp::OrderIntended { venue_spot, idx, price, qty } => {
                    let v = if venue_spot { Venue::Spot } else { Venue::UsdsFutures };
                    let cid = format!("b01r1-{idx:03}-ENT");
                    Some(JournalEvent::OrderIntended {
                        order: sample_order(v, &cid, price, qty),
                    })
                }
                StepOp::OrderAck { venue_spot, idx, filled } => {
                    let v = if venue_spot { Venue::Spot } else { Venue::UsdsFutures };
                    let cid = format!("b01r1-{idx:03}-ENT");
                    Some(JournalEvent::OrderStatusUpdated {
                        venue: v,
                        client_id: cid,
                        exchange_id: Some(1000 + i64::from(idx)),
                        status: OrderStatus::PartiallyFilled,
                        filled_qty: Qty(Decimal::new(filled, 4)),
                        avg_fill_price: Some(Price(Decimal::new(5_000_000, 2))),
                        updated_at: sample_dt(ts_ms / 1000),
                    })
                }
                StepOp::Fill { venue_spot, trade_id, qty } => {
                    let v = if venue_spot { Venue::Spot } else { Venue::UsdsFutures };
                    Some(JournalEvent::FillRecorded {
                        fill: FillRecord {
                            venue: v,
                            trade_id,
                            symbol: Symbol("BTCUSDT".into()),
                            client_id: "b01r1-000-ENT".into(),
                            price: Price(Decimal::new(5_000_000, 2)),
                            qty: Qty(Decimal::new(qty, 4)),
                            fee: Decimal::new(5, 2),
                            fee_asset: "USDT".into(),
                            is_maker: true,
                            ts_ms,
                        },
                    })
                }
                StepOp::OpenPos { venue_spot, pos_idx, entry } => {
                    let v = if venue_spot { Venue::Spot } else { Venue::UsdsFutures };
                    let id = Uuid::from_u128(u128::from(pos_idx) + 1);
                    Some(JournalEvent::PositionOpened {
                        position: sample_position(id, v, entry, entry - 5000),
                    })
                }
                StepOp::MoveStop { pos_idx, new_stop, be } => {
                    let id = Uuid::from_u128(u128::from(pos_idx) + 1);
                    Some(JournalEvent::PositionStopMoved {
                        venue: Venue::Spot,
                        position_id: id,
                        new_stop_price: Price(Decimal::new(new_stop, 2)),
                        breakeven_done: be,
                    })
                }
                StepOp::ClosePos { pos_idx, pnl } => {
                    let id = Uuid::from_u128(u128::from(pos_idx) + 1);
                    Some(JournalEvent::PositionStateChanged {
                        venue: Venue::Spot,
                        position_id: id,
                        state: PositionState::Closed,
                        qty_open: Qty(Decimal::ZERO),
                        realized_pnl_quote: Decimal::new(pnl, 2),
                        fees_paid_quote: Decimal::new(10, 2),
                        close_reason: Some("TP2".into()),
                    })
                }
                StepOp::SetMode(m) => {
                    let mode = match m {
                        0 => EngineMode::Ready,
                        1 => EngineMode::Paused,
                        2 => EngineMode::ReduceOnly,
                        3 => EngineMode::Halted,
                        _ => EngineMode::Syncing,
                    };
                    Some(JournalEvent::EngineModeChanged {
                        mode,
                        reason: "prop".into(),
                    })
                }
                StepOp::SetRisk(k, v) => Some(JournalEvent::RiskStateSet {
                    key: format!("k{k}"),
                    value: v.to_string(),
                }),
                StepOp::SetVenue { spot, active } => Some(JournalEvent::VenueStateChanged {
                    record: VenueStateRecord {
                        venue: if spot { Venue::Spot } else { Venue::UsdsFutures },
                        state: if active { VenueState::Active } else { VenueState::Draining },
                        since_ms: ts_ms,
                        last_switch_ms: Some(ts_ms),
                        switches_today: 1,
                    },
                }),
            };

            if let Some(ev) = event_opt {
                let env = JournalEnvelope::new(ev);
                let seq = store.append_event(ts_ms, &env).expect("append_event");
                expected.apply_event(seq, &env);
            }
        }

        let recovered = store.recover_state().expect("recover_state");
        prop_assert_eq!(recovered, expected);
    }
}

/// Child-process crash test: when `BOT_STORE_CRASH_CHILD_DB` is set, this test writes
/// committed events + a snapshot + more committed events, then begins an uncommitted
/// transaction and immediately invokes `std::process::abort()`.
#[test]
fn crash_recovery_after_child_process_abort_mid_write() {
    if let Ok(db_path_str) = std::env::var("BOT_STORE_CRASH_CHILD_DB") {
        let mut store = SqliteStore::open(&PathBuf::from(db_path_str)).expect("child open");
        let mut state = PersistedEngineState::default();

        // Commit 5 events + snapshot.
        for i in 1..=5_i64 {
            let env = JournalEnvelope::new(JournalEvent::RiskStateSet {
                key: format!("committed_{i}"),
                value: format!("{i}"),
            });
            let seq = store
                .append_event(1_700_000_000_000 + i, &env)
                .expect("append");
            state.apply_event(seq, &env);
        }
        store
            .snapshot_write(1_700_000_000_005, &state)
            .expect("snapshot");

        // Commit 3 more events after snapshot.
        for i in 6..=8_i64 {
            let env = JournalEnvelope::new(JournalEvent::RiskStateSet {
                key: format!("committed_{i}"),
                value: format!("{i}"),
            });
            let _ = store
                .append_event(1_700_000_000_000 + i, &env)
                .expect("append");
        }

        // Start an uncommitted write and abort abruptly mid-transaction!
        store
            .execute_raw_batch(
                "BEGIN IMMEDIATE;
                 INSERT INTO journal (ts_ms, kind, payload) VALUES (9999999999, 'RiskStateSet', '{\"schema_version\":1,\"event\":{\"type\":\"RiskStateSet\",\"key\":\"uncommitted\",\"value\":\"bad\"}}');",
            )
            .expect("begin uncommitted tx");
        std::process::abort();
    }

    // Parent process: spawn ourselves running only this test in child mode.
    let dir = TempDir::new().expect("tempdir");
    let db_path = dir.path().join("crash.sqlite");

    let exe = std::env::current_exe().expect("current_exe");
    let status = Command::new(exe)
        .arg("--exact")
        .arg("crash_recovery_after_child_process_abort_mid_write")
        .arg("--nocapture")
        .env("BOT_STORE_CRASH_CHILD_DB", &db_path)
        .status()
        .expect("spawn child");
    assert!(
        !status.success(),
        "child process must terminate via abort()"
    );

    // Reopen DB in parent: integrity must be intact and all 8 committed events recovered,
    // while the uncommitted aborted transaction is cleanly rolled back!
    let store = SqliteStore::open(&db_path).expect("reopen after crash");
    store
        .integrity_check()
        .expect("SQLite integrity_check must pass after abort");

    let recovered = store.recover_state().expect("recover after crash");
    assert_eq!(recovered.last_seq, 8);
    for i in 1..=8_i64 {
        assert_eq!(
            recovered
                .risk_state
                .get(&format!("committed_{i}"))
                .map(String::as_str),
            Some(i.to_string().as_str())
        );
    }
    assert!(
        !recovered.risk_state.contains_key("uncommitted"),
        "uncommitted write before abort() must be rolled back"
    );
}

#[test]
fn io_errors_and_unsupported_schema_version_return_typed_errors() {
    // Opening a path inside a non-existent directory returns typed fatal IoError.
    let bad_path = PathBuf::from("/nonexistent_dir_bot_store_xyz/nested/bot.sqlite");
    let err = SqliteStore::open(&bad_path).expect_err("nonexistent parent dir must fail");
    assert!(
        err.is_fatal_storage(),
        "expected fatal storage error, got {err:?}"
    );

    // Read-only DB file returns fatal IoError on write.
    let dir = TempDir::new().expect("tempdir");
    let ro_path = dir.path().join("readonly.sqlite");
    {
        let _ = SqliteStore::open(&ro_path).expect("create db");
    }
    let mut perms = std::fs::metadata(&ro_path).expect("meta").permissions();
    perms.set_mode(0o444);
    std::fs::set_permissions(&ro_path, perms).expect("chmod 444");

    // Note: if running as non-root (standard), writing to 0o444 file fails with IoError/ReadOnly.
    if let Ok(mut store) = SqliteStore::open(&ro_path) {
        let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
            mode: EngineMode::Paused,
            reason: "ro".into(),
        });
        let write_err = store
            .append_event(1000, &env)
            .expect_err("write to readonly must fail");
        assert!(write_err.is_fatal_storage());
    }

    // Unsupported future event schema version returns typed error.
    let mut mem = SqliteStore::open_in_memory().expect("mem");
    let future_env = JournalEnvelope {
        schema_version: EVENT_SCHEMA_VERSION + 99,
        event: JournalEvent::EngineModeChanged {
            mode: EngineMode::Ready,
            reason: "future".into(),
        },
    };
    let ver_err = mem
        .append_event(1000, &future_env)
        .expect_err("future schema version must be rejected");
    assert!(matches!(
        ver_err,
        StoreError::UnsupportedEventVersion { .. }
    ));
}

#[test]
fn persisted_state_prunes_terminal_orders_closed_positions_and_caps_fills() {
    let mut store = SqliteStore::open_in_memory().expect("mem");
    let mut state = PersistedEngineState::default();
    let mut seq_ts = 1_700_000_000_000_i64;

    // Intended order -> Filled -> pruned from active orders map.
    let ord = sample_order(Venue::Spot, "b01r1S-PRUNE1-ENT", 6_000_000, 100);
    let e1 = JournalEnvelope::new(JournalEvent::OrderIntended { order: ord });
    let s1 = store.append_event(seq_ts, &e1).expect("append");
    state.apply_event(s1, &e1);
    assert_eq!(state.orders.len(), 1);

    seq_ts += 1_000;
    let e2 = JournalEnvelope::new(JournalEvent::OrderStatusUpdated {
        venue: Venue::Spot,
        client_id: "b01r1S-PRUNE1-ENT".into(),
        exchange_id: Some(77),
        status: OrderStatus::Filled,
        filled_qty: Qty(Decimal::new(100, 4)),
        avg_fill_price: Some(Price(Decimal::new(6_000_000, 2))),
        updated_at: sample_dt(seq_ts / 1000),
    });
    let s2 = store.append_event(seq_ts, &e2).expect("append");
    state.apply_event(s2, &e2);
    assert!(
        state.orders.is_empty(),
        "terminal orders must be pruned from active snapshot state"
    );

    // Open position -> Closed -> pruned from active positions map.
    let pos_id = Uuid::from_u128(999);
    seq_ts += 1_000;
    let e3 = JournalEnvelope::new(JournalEvent::PositionOpened {
        position: sample_position(pos_id, Venue::Spot, 6_000_000, 5_900_000),
    });
    let s3 = store.append_event(seq_ts, &e3).expect("append");
    state.apply_event(s3, &e3);
    assert_eq!(state.positions.len(), 1);

    seq_ts += 1_000;
    let e4 = JournalEnvelope::new(JournalEvent::PositionStateChanged {
        venue: Venue::Spot,
        position_id: pos_id,
        state: PositionState::Closed,
        qty_open: Qty(Decimal::ZERO),
        realized_pnl_quote: Decimal::new(500, 2),
        fees_paid_quote: Decimal::new(10, 2),
        close_reason: Some("TP1".into()),
    });
    let s4 = store.append_event(seq_ts, &e4).expect("append");
    state.apply_event(s4, &e4);
    assert!(
        state.positions.is_empty(),
        "closed positions must be pruned from active snapshot state"
    );

    // Exceeding MAX_RETAINED_SNAPSHOT_FILLS caps fills deterministically.
    let cap = bot_store::MAX_RETAINED_SNAPSHOT_FILLS;
    for i in 1..=(cap + 20) {
        seq_ts += 10;
        let fill_env = JournalEnvelope::new(JournalEvent::FillRecorded {
            fill: FillRecord {
                venue: Venue::Spot,
                trade_id: i64::try_from(i).expect("i64"),
                symbol: Symbol("BTCUSDT".into()),
                client_id: "b01r1S-PRUNE1-ENT".into(),
                price: Price(Decimal::new(6_000_000, 2)),
                qty: Qty(Decimal::new(10, 4)),
                fee: Decimal::new(1, 2),
                fee_asset: "USDT".into(),
                is_maker: true,
                ts_ms: seq_ts,
            },
        });
        let s = store.append_event(seq_ts, &fill_env).expect("append");
        state.apply_event(s, &fill_env);
    }
    assert_eq!(state.fills.len(), cap);
    store
        .snapshot_write(seq_ts, &state)
        .expect("snapshot_write");
    let recovered = store.recover_state().expect("recover");
    assert_eq!(recovered, state);
}

#[test]
fn sqlite_store_ping_succeeds_on_open_db() {
    let store = SqliteStore::open_in_memory().expect("open in-memory store");
    assert!(store.ping().is_ok());
}
