#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]
//! Contract tests validating `protocol/ipc-v1.schema.json` against Rust IPC types and UI fixtures.

use std::fs;
use std::path::PathBuf;

use bot_ipc::protocol::{
    AlertLevel, AlertView, ClientMessage, CommandKind, CommandPayload, CommandResultPayload,
    ConfirmAction, DeltaPayload, EngineSummary, EventPayload, GetKlinesPayload, GetTradesPayload,
    HeartbeatPayload, HelloPayload, HttpErrorResponse, KlineBar, KlinesPayload, MarketPayload,
    MarketTickSummary, OnOpenPositionsArg, PROTOCOL_VERSION, PositionView, RateStatus,
    RiskLimitsSummary, RiskSummary, ServerMessage, SnapshotPayload, TradeRecord, TradesPayload,
    VenuesSummary, WireEnvelope,
};
use bot_types::{EngineMode, TradingMode, Venue, VenueState};
use rust_decimal::Decimal;
use serde_json::Value;
use uuid::Uuid;

fn dec(s: &str) -> Decimal {
    Decimal::from_str_exact(s).expect("valid decimal literal")
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn load_schema() -> Value {
    let path = repo_root().join("protocol/ipc-v1.schema.json");
    let raw = fs::read_to_string(&path).expect("read ipc-v1.schema.json");
    serde_json::from_str(&raw).expect("parse ipc-v1.schema.json")
}

#[test]
fn ui_fixtures_validate_against_schema_and_roundtrip_rust() {
    let schema = load_schema();
    let validator = jsonschema::validator_for(&schema).expect("compile schema");

    let fixtures_dir = repo_root().join("protocol/fixtures/v1");
    let envelope_files = [
        "hello.json",
        "snapshot.json",
        "delta.json",
        "market.json",
        "event.json",
        "heartbeat.json",
        "command_result.json",
        "command_pause.json",
        "klines.json",
        "trades.json",
    ];

    for file in envelope_files {
        let raw = fs::read_to_string(fixtures_dir.join(file)).expect("read fixture");
        let val: Value = serde_json::from_str(&raw).expect("parse fixture");
        assert!(
            validator.is_valid(&val),
            "fixture {file} failed schema validation"
        );

        if file == "command_pause.json" {
            let parsed: WireEnvelope<ClientMessage> =
                serde_json::from_value(val.clone()).expect("deserialize client fixture");
            let reserialized = serde_json::to_value(&parsed).expect("reserialize client fixture");
            assert!(
                validator.is_valid(&reserialized),
                "reserialized {file} failed schema validation"
            );
        } else {
            let parsed: WireEnvelope<ServerMessage> =
                serde_json::from_value(val.clone()).expect("deserialize server fixture");
            let reserialized = serde_json::to_value(&parsed).expect("reserialize server fixture");
            assert!(
                validator.is_valid(&reserialized),
                "reserialized {file} failed schema validation"
            );
        }
    }

    // Validate HTTP error fixture against $defs.HttpErrorResponse
    let http_err_raw = fs::read_to_string(fixtures_dir.join("http_error.json")).expect("read");
    let http_err_val: Value = serde_json::from_str(&http_err_raw).expect("parse");
    let parsed_err: HttpErrorResponse =
        serde_json::from_value(http_err_val.clone()).expect("deserialize HttpErrorResponse");
    assert_eq!(parsed_err.error.code, "unauthorized");
    let mut err_schema = schema;
    if let Some(obj) = err_schema.as_object_mut() {
        obj.remove("oneOf");
        obj.insert(
            "$ref".to_owned(),
            Value::String("#/$defs/HttpErrorResponse".to_owned()),
        );
    }
    let err_validator = jsonschema::validator_for(&err_schema).expect("compile error schema");
    assert!(err_validator.is_valid(&http_err_val));
}

#[test]
fn all_rust_protocol_messages_conform_to_json_schema() {
    let schema = load_schema();
    let validator = jsonschema::validator_for(&schema).expect("compile schema");

    let instance_id = Uuid::now_v7();
    let cmd_id = Uuid::now_v7();
    let pos_id = Uuid::now_v7();

    let server_messages = vec![
        WireEnvelope::server(
            1,
            1_790_000_000_000,
            ServerMessage::Hello(HelloPayload {
                protocol_version: PROTOCOL_VERSION,
                build_mode: TradingMode::Paper,
                capabilities: vec!["uds".to_owned(), "http".to_owned(), "ws".to_owned()],
                engine_instance_id: instance_id,
            }),
        ),
        WireEnvelope::server(
            2,
            1_790_000_000_100,
            ServerMessage::Snapshot(SnapshotPayload {
                venues: VenuesSummary {
                    active: Venue::Spot,
                    spot: VenueState::Active,
                    futures: VenueState::Disabled,
                    last_switch_ms: 0,
                    switches_today: 0,
                },
                engine: EngineSummary {
                    mode: EngineMode::Ready,
                    build_mode: TradingMode::Paper,
                    uptime_s: 120,
                    ws_latency_ms: 10,
                    clock_drift_ms: -2,
                    rate: RateStatus {
                        weight_used: 50,
                        weight_limit: 6000,
                        orders_10s: 1,
                    },
                },
                risk: RiskSummary {
                    equity: dec("1000.00"),
                    day_pnl_pct: dec("0.50"),
                    week_pnl_pct: dec("1.20"),
                    drawdown_pct: dec("-0.30"),
                    heat_pct: dec("0.50"),
                    heat_max_pct: dec("2.00"),
                    open_positions: 1,
                    max_positions: 4,
                    btc_guard: "SECURE".to_owned(),
                    limits: RiskLimitsSummary {
                        daily_halt_pct: dec("-2.0"),
                        kill_pct: dec("-10.0"),
                    },
                },
                positions: vec![PositionView {
                    id: pos_id,
                    venue: Venue::Spot,
                    symbol: "BTCUSDT".to_owned(),
                    strategy: "TrendPullback".to_owned(),
                    state: "OpenProtected".to_owned(),
                    entry: dec("68000.00"),
                    mark: dec("68500.00"),
                    qty: dec("0.010"),
                    stop: dec("67500.00"),
                    tp1: dec("69000.00"),
                    tp2: dec("70000.00"),
                    profit_r: dec("1.0"),
                    unrealized_quote: dec("5.00"),
                    age_s: 600,
                    tp1_done: false,
                    be_done: false,
                }],
                orders: vec![],
                signals_shadow: vec![],
                alerts: vec![AlertView {
                    id: "a1".to_owned(),
                    venue: Some(Venue::Spot),
                    level: AlertLevel::Warn,
                    msg: "test alert".to_owned(),
                    acked: false,
                }],
            }),
        ),
        WireEnvelope::server(
            3,
            1_790_000_000_200,
            ServerMessage::Delta(DeltaPayload::default()),
        ),
        WireEnvelope::server(
            4,
            1_790_000_000_300,
            ServerMessage::Market(MarketPayload {
                ticks: vec![MarketTickSummary {
                    venue: Venue::Spot,
                    symbol: "ETHUSDT".to_owned(),
                    bid: dec("3100.10"),
                    ask: dec("3100.20"),
                    last: dec("3100.15"),
                    spread_bps: dec("0.32"),
                    ts_ms: 1_790_000_000_299,
                }],
            }),
        ),
        WireEnvelope::server(
            5,
            1_790_000_000_400,
            ServerMessage::Event(EventPayload {
                venue: Some(Venue::Spot),
                symbol: Some("ETHUSDT".to_owned()),
                kind: "Fill".to_owned(),
                level: AlertLevel::Info,
                message: "Filled entry".to_owned(),
            }),
        ),
        WireEnvelope::server(
            6,
            1_790_000_001_000,
            ServerMessage::Heartbeat(HeartbeatPayload {
                engine_state: EngineMode::Ready,
                ws_latency_ms: 15,
                weight_used: 100,
                weight_limit: 6000,
            }),
        ),
        WireEnvelope::server(
            7,
            1_790_000_001_100,
            ServerMessage::CommandResult(CommandResultPayload {
                cmd_id,
                ok: true,
                code: "ok".to_owned(),
                message: "done".to_owned(),
                confirm_nonce: None,
                expires_at_ms: None,
            }),
        ),
        WireEnvelope::server(
            8,
            1_790_000_001_200,
            ServerMessage::Klines(KlinesPayload {
                req_id: Uuid::now_v7(),
                venue: Venue::Spot,
                symbol: "BTCUSDT".to_owned(),
                tf: "15m".to_owned(),
                klines: vec![KlineBar {
                    time: 1_790_000_000,
                    open: dec("68000.00"),
                    high: dec("68200.00"),
                    low: dec("67950.00"),
                    close: dec("68120.50"),
                    volume: dec("12.50"),
                }],
            }),
        ),
        WireEnvelope::server(
            9,
            1_790_000_001_300,
            ServerMessage::Trades(TradesPayload {
                req_id: Uuid::now_v7(),
                venue: Some(Venue::Spot),
                trades: vec![TradeRecord {
                    venue: Venue::Spot,
                    symbol: "ETHUSDT".to_owned(),
                    side: "BUY".to_owned(),
                    price: dec("3120.40"),
                    qty: dec("0.320"),
                    fee: dec("0.9985"),
                    realized_pnl: dec("21.66"),
                    ts_ms: 1_789_995_000_000,
                }],
            }),
        ),
    ];

    for msg in server_messages {
        let val = serde_json::to_value(&msg).expect("serialize server message");
        assert!(
            validator.is_valid(&val),
            "server message failed schema validation: {val}"
        );
    }

    let commands = vec![
        CommandKind::Pause,
        CommandKind::Resume,
        CommandKind::SwitchVenue {
            target: Venue::UsdsFutures,
            on_open_positions: OnOpenPositionsArg::Refuse,
            confirm_nonce: "nonce123".to_owned(),
        },
        CommandKind::SetParam {
            key: "risk.risk_per_trade_pct".to_owned(),
            value: "0.25".to_owned(),
        },
        CommandKind::DisableStrategy {
            id: "TrendPullback".to_owned(),
        },
        CommandKind::ClosePosition {
            position_id: pos_id,
        },
        CommandKind::RequestConfirm {
            action: ConfirmAction::PanicFlatten,
        },
        CommandKind::PanicFlatten {
            confirm_nonce: "nonce123".to_owned(),
        },
        CommandKind::ResetKill {
            confirm_nonce: "nonce456".to_owned(),
        },
        CommandKind::AckAlert {
            id: "a1".to_owned(),
        },
    ];

    for (idx, cmd) in commands.into_iter().enumerate() {
        let env = WireEnvelope::client(
            idx as u64 + 1,
            1_790_000_002_000,
            ClientMessage::Command(CommandPayload {
                cmd_id: Uuid::now_v7(),
                cmd,
            }),
        );
        let val = serde_json::to_value(&env).expect("serialize command");
        assert!(
            validator.is_valid(&val),
            "command failed schema validation: {val}"
        );
    }

    let query_messages = vec![
        WireEnvelope::client(
            20,
            1_790_000_002_100,
            ClientMessage::GetKlines(GetKlinesPayload {
                req_id: Uuid::now_v7(),
                venue: Venue::Spot,
                symbol: "BTCUSDT".to_owned(),
                tf: "15m".to_owned(),
                limit: 20,
            }),
        ),
        WireEnvelope::client(
            21,
            1_790_000_002_200,
            ClientMessage::GetTrades(GetTradesPayload {
                req_id: Uuid::now_v7(),
                venue: Some(Venue::Spot),
                from_ms: Some(1_789_000_000_000),
                to_ms: None,
            }),
        ),
    ];
    for qm in query_messages {
        let val = serde_json::to_value(&qm).expect("serialize query message");
        assert!(
            validator.is_valid(&val),
            "query message failed schema validation: {val}"
        );
    }
}
