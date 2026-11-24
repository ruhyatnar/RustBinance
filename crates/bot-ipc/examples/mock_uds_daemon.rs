#![forbid(unsafe_code)]
//! Deterministic paper-mode UDS mock daemon for `desktop-ui` smoke & integration tests (M12).
//!
//! Usage: `cargo run -p bot-ipc --example mock_uds_daemon -- --socket /path/to/bot.sock`

use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use bot_config::BotConfig;
use bot_ipc::auth::PeerAuthPolicy;
use bot_ipc::command::{CommandDispatcher, InMemoryIpcBackend, IpcBackend};
use bot_ipc::conflate::TelemetryHub;
use bot_ipc::nonce::{ConfirmNonceStore, unix_now_ms};
use bot_ipc::protocol::{
    AlertLevel, AlertView, EngineSummary, HeartbeatPayload, KlineBar, MarketPayload,
    MarketTickSummary, OrderView, PositionView, RateStatus, RiskLimitsSummary, RiskSummary,
    ServerMessage, ShadowSignalView, SnapshotPayload, TradeRecord, VenuesSummary,
};
use bot_ipc::uds::{UdsServerConfig, serve_uds};
use bot_types::{EngineMode, TradingMode, Venue, VenueState};
use rust_decimal::Decimal;
use tokio::signal::unix::{SignalKind, signal};
use uuid::Uuid;

fn parse_socket_arg() -> Result<PathBuf, String> {
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--socket" {
            return args
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| "missing value after --socket".to_owned());
        }
    }
    Err("usage: mock_uds_daemon --socket <path>".to_owned())
}

fn dec(mantissa: i64, scale: u32) -> Decimal {
    Decimal::new(mantissa, scale)
}

fn seed_snapshot(pos_id: Uuid) -> SnapshotPayload {
    SnapshotPayload {
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
            uptime_s: 3_600,
            ws_latency_ms: 14,
            clock_drift_ms: 2,
            rate: RateStatus {
                weight_used: 142,
                weight_limit: 6_000,
                orders_10s: 1,
            },
        },
        risk: RiskSummary {
            equity: dec(101_240, 2),
            day_pnl_pct: dec(82, 2),
            week_pnl_pct: dec(190, 2),
            drawdown_pct: dec(-110, 2),
            heat_pct: dec(90, 2),
            heat_max_pct: dec(200, 2),
            open_positions: 1,
            max_positions: 4,
            btc_guard: "SECURE".to_owned(),
            limits: RiskLimitsSummary {
                daily_halt_pct: dec(-200, 2),
                kill_pct: dec(-1_000, 2),
            },
        },
        positions: vec![PositionView {
            id: pos_id,
            venue: Venue::Spot,
            symbol: "ETHUSDT".to_owned(),
            strategy: "TrendPullback".to_owned(),
            state: "OpenProtected".to_owned(),
            entry: dec(312_040, 2),
            mark: dec(318_810, 2),
            qty: dec(320, 3),
            stop: dec(309_800, 2),
            tp1: dec(320_000, 2),
            tp2: dec(328_000, 2),
            profit_r: dec(19, 1),
            unrealized_quote: dec(2_166, 2),
            age_s: 1_800,
            tp1_done: false,
            be_done: true,
        }],
        orders: vec![OrderView {
            client_id: "oco_tp1_eth_01".to_owned(),
            venue: Venue::Spot,
            symbol: "ETHUSDT".to_owned(),
            side: "SELL".to_owned(),
            kind: "LIMIT_MAKER".to_owned(),
            purpose: "TP1".to_owned(),
            status: "NEW".to_owned(),
            price: Some(dec(320_000, 2)),
            qty: dec(160, 3),
            filled_qty: Decimal::ZERO,
        }],
        signals_shadow: vec![ShadowSignalView {
            venue: Venue::Spot,
            symbol: "SOLUSDT".to_owned(),
            strategy: "Breakout".to_owned(),
            direction: "LONG".to_owned(),
            entry: dec(15240, 2),
            stop: dec(14810, 2),
            ts_ms: 1_790_000_000_000,
        }],
        alerts: vec![AlertView {
            id: "a1".to_owned(),
            venue: Some(Venue::Spot),
            level: AlertLevel::Warn,
            msg: "Spread BTCUSDT 6.1 bps > gate".to_owned(),
            acked: false,
        }],
    }
}

fn seed_klines(base_price_cents: i64) -> Vec<KlineBar> {
    let mut bars = Vec::with_capacity(24);
    for idx in 0_i64..24_i64 {
        let step = idx * 850;
        let open_c = base_price_cents + step;
        let close_c = open_c + if idx % 2 == 0 { 1_250 } else { -450 };
        let high_c = open_c.max(close_c) + 600;
        let low_c = open_c.min(close_c) - 500;
        bars.push(KlineBar {
            time: 1_789_980_000 + idx * 900,
            open: dec(open_c, 2),
            high: dec(high_c, 2),
            low: dec(low_c, 2),
            close: dec(close_c, 2),
            volume: dec(1_250 + idx * 35, 2),
        });
    }
    bars
}

async fn populate_backend(backend: &InMemoryIpcBackend) {
    let pos_id = Uuid::from_u128(0x0192_f0a8_1111_7000_8000_0000_0000_0001);
    backend.set_snapshot(seed_snapshot(pos_id)).await;
    for (sym, tf, base) in [
        ("BTCUSDT", "15m", 6_800_000),
        ("ETHUSDT", "15m", 310_000),
        ("ETHUSDT", "1h", 308_000),
    ] {
        backend
            .set_klines(
                Venue::Spot,
                sym.to_owned(),
                tf.to_owned(),
                seed_klines(base),
            )
            .await;
    }
    backend
        .set_trades(vec![TradeRecord {
            venue: Venue::Spot,
            symbol: "ETHUSDT".to_owned(),
            side: "BUY".to_owned(),
            price: dec(312_040, 2),
            qty: dec(320, 3),
            fee: dec(9_985, 4),
            realized_pnl: dec(2_166, 2),
            ts_ms: 1_789_995_000_000,
        }])
        .await;
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let socket_path = parse_socket_arg()?;
    let backend = Arc::new(InMemoryIpcBackend::new(BotConfig::default()));
    populate_backend(&backend).await;

    let dispatcher = Arc::new(CommandDispatcher::new(
        Arc::clone(&backend),
        None,
        ConfirmNonceStore::new(Duration::from_secs(10)),
    ));
    let hub = Arc::new(TelemetryHub::new(64));

    let server_handle = serve_uds(
        UdsServerConfig {
            socket_path: socket_path.clone(),
            peer_auth: PeerAuthPolicy::allow_current_process(),
        },
        Arc::clone(&dispatcher),
        Arc::clone(&hub),
    )
    .await?;

    println!("READY {}", socket_path.display());

    let mut sigterm = signal(SignalKind::terminate())?;
    let mut ticker = tokio::time::interval(Duration::from_millis(200));

    loop {
        tokio::select! {
            _ = tokio::signal::ctrl_c() => break,
            _ = sigterm.recv() => break,
            _ = ticker.tick() => {
                let mode = backend.engine_mode().await;
                hub.publish(
                    ServerMessage::Heartbeat(HeartbeatPayload {
                        engine_state: mode,
                        ws_latency_ms: 14,
                        weight_used: 142,
                        weight_limit: 6_000,
                    }),
                    unix_now_ms(),
                ).await;
                hub.publish(
                    ServerMessage::Market(MarketPayload {
                        ticks: vec![
                            MarketTickSummary {
                                venue: Venue::Spot,
                                symbol: "BTCUSDT".to_owned(),
                                bid: dec(6_812_000, 2),
                                ask: dec(6_812_100, 2),
                                last: dec(6_812_050, 2),
                                spread_bps: dec(15, 2),
                                ts_ms: unix_now_ms(),
                            },
                            MarketTickSummary {
                                venue: Venue::Spot,
                                symbol: "ETHUSDT".to_owned(),
                                bid: dec(318_800, 2),
                                ask: dec(318_820, 2),
                                last: dec(318_810, 2),
                                spread_bps: dec(63, 2),
                                ts_ms: unix_now_ms(),
                            },
                        ],
                    }),
                    unix_now_ms(),
                ).await;
            }
        }
    }

    server_handle.abort();
    let _ = std::fs::remove_file(&socket_path);
    Ok(())
}
