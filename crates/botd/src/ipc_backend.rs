//! Real [`IpcBackend`] and [`TelemetryHub`] bridge wired directly to [`EngineHandle`].

use std::sync::Arc;

use async_trait::async_trait;
use bot_config::BotConfig;
use bot_engine::{EngineFullSnapshot, EngineHandle, TelemetrySnapshot};
use bot_ipc::protocol::{
    DeltaPayload, EngineSummary, OrderView, PositionView, RateStatus, RiskLimitsSummary,
    RiskSummary, ServerMessage, SnapshotPayload, VenuesSummary,
};
use bot_ipc::{IpcBackend, IpcError, TelemetryHub};
use bot_risk::ModeTrigger;
use bot_types::{EngineMode, TradingMode};
use rust_decimal::Decimal;
use time::OffsetDateTime;
use tokio::sync::watch;
use tokio::task::JoinHandle;
use uuid::Uuid;

fn unix_now_ms() -> i64 {
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000).unwrap_or(0)
}

/// [`IpcBackend`] implementation backed by the live [`EngineHandle`] of [`bot_engine::EngineActor`].
pub struct EngineIpcBackend {
    engine_instance_id: Uuid,
    config: BotConfig,
    handle: EngineHandle,
}

impl EngineIpcBackend {
    /// Construct a new IPC backend wired to `handle`.
    #[must_use]
    pub fn new(config: BotConfig, handle: EngineHandle) -> Self {
        Self {
            engine_instance_id: Uuid::now_v7(),
            config,
            handle,
        }
    }

    fn map_full_snapshot(&self, full: &EngineFullSnapshot, now_ms: i64) -> SnapshotPayload {
        let uptime_s =
            u64::try_from(now_ms.saturating_sub(full.started_at_ms).max(0) / 1000).unwrap_or(0);
        let positions = full
            .positions
            .iter()
            .map(|p| {
                let opened_ms =
                    i64::try_from(p.opened_at.unix_timestamp_nanos() / 1_000_000).unwrap_or(now_ms);
                let age_s =
                    u64::try_from(now_ms.saturating_sub(opened_ms).max(0) / 1000).unwrap_or(0);
                PositionView {
                    id: p.id,
                    venue: p.venue,
                    symbol: p.symbol.as_str().to_owned(),
                    strategy: format!("{:?}", p.strategy),
                    state: format!("{:?}", p.state),
                    entry: p.entry_price.as_decimal(),
                    mark: p.best_price.as_decimal(),
                    qty: p.qty_open.as_decimal(),
                    stop: p.stop_price.as_decimal(),
                    tp1: p.tp1_price.as_decimal(),
                    tp2: p
                        .tp2_price
                        .map_or_else(|| p.tp1_price.as_decimal(), bot_types::Price::as_decimal),
                    profit_r: Decimal::ZERO,
                    unrealized_quote: p.realized_pnl_quote,
                    age_s,
                    tp1_done: p.tp1_done,
                    be_done: p.breakeven_done,
                }
            })
            .collect();

        let orders = full
            .orders
            .iter()
            .map(|o| OrderView {
                client_id: o.client_id.clone(),
                venue: o.venue,
                symbol: o.symbol.as_str().to_owned(),
                side: format!("{:?}", o.side),
                kind: format!("{:?}", o.kind),
                purpose: format!("{:?}", o.purpose),
                status: format!("{:?}", o.status),
                price: o.price.map(bot_types::Price::as_decimal),
                qty: o.qty.as_decimal(),
                filled_qty: o.filled_qty.as_decimal(),
            })
            .collect();

        SnapshotPayload {
            venues: VenuesSummary {
                active: full.telemetry.venue,
                spot: full.telemetry.spot_state,
                futures: full.telemetry.futures_state,
                last_switch_ms: u64::try_from(full.last_switch_ms).unwrap_or(0),
                switches_today: full.switches_today,
            },
            engine: EngineSummary {
                mode: full.telemetry.mode,
                build_mode: self.config.general.mode,
                uptime_s,
                ws_latency_ms: u64::from(full.health.ws_latency_ms),
                clock_drift_ms: full.health.clock_drift_ms,
                rate: RateStatus {
                    weight_used: 0,
                    weight_limit: 6000,
                    orders_10s: 0,
                },
            },
            risk: RiskSummary {
                equity: full.telemetry.equity_total_usd,
                day_pnl_pct: full.day_pnl_pct,
                week_pnl_pct: full.week_pnl_pct,
                drawdown_pct: full.drawdown_pct,
                heat_pct: full.heat_pct,
                heat_max_pct: full.config.risk.portfolio_heat_max_pct,
                open_positions: u32::try_from(full.telemetry.open_positions).unwrap_or(u32::MAX),
                max_positions: full.config.risk.max_open_positions,
                btc_guard: full.btc_guard.clone(),
                limits: RiskLimitsSummary {
                    daily_halt_pct: full.config.risk.daily_loss_halt_pct,
                    kill_pct: full.config.risk.drawdown_kill_pct,
                },
            },
            positions,
            orders,
            signals_shadow: Vec::new(),
            alerts: Vec::new(),
        }
    }

    fn fallback_snapshot(&self, snap: &TelemetrySnapshot) -> SnapshotPayload {
        SnapshotPayload {
            venues: VenuesSummary {
                active: snap.venue,
                spot: snap.spot_state,
                futures: snap.futures_state,
                last_switch_ms: 0,
                switches_today: 0,
            },
            engine: EngineSummary {
                mode: snap.mode,
                build_mode: self.config.general.mode,
                uptime_s: 0,
                ws_latency_ms: 0,
                clock_drift_ms: 0,
                rate: RateStatus {
                    weight_used: 0,
                    weight_limit: 6000,
                    orders_10s: 0,
                },
            },
            risk: RiskSummary {
                equity: snap.equity_total_usd,
                day_pnl_pct: Decimal::ZERO,
                week_pnl_pct: Decimal::ZERO,
                drawdown_pct: Decimal::ZERO,
                heat_pct: Decimal::ZERO,
                heat_max_pct: self.config.risk.portfolio_heat_max_pct,
                open_positions: u32::try_from(snap.open_positions).unwrap_or(u32::MAX),
                max_positions: self.config.risk.max_open_positions,
                btc_guard: "OK".to_owned(),
                limits: RiskLimitsSummary {
                    daily_halt_pct: self.config.risk.daily_loss_halt_pct,
                    kill_pct: self.config.risk.drawdown_kill_pct,
                },
            },
            positions: Vec::new(),
            orders: Vec::new(),
            signals_shadow: Vec::new(),
            alerts: Vec::new(),
        }
    }
}

#[async_trait]
impl IpcBackend for EngineIpcBackend {
    fn engine_instance_id(&self) -> Uuid {
        self.engine_instance_id
    }

    fn build_mode(&self) -> TradingMode {
        self.config.general.mode
    }

    async fn snapshot(&self) -> SnapshotPayload {
        let now_ms = unix_now_ms();
        match self.handle.full_snapshot(now_ms).await {
            Ok(full) => self.map_full_snapshot(&full, now_ms),
            Err(_) => self.fallback_snapshot(&self.handle.telemetry()),
        }
    }

    async fn engine_mode(&self) -> EngineMode {
        self.handle.telemetry().mode
    }

    async fn pause(&self) -> Result<String, IpcError> {
        let current = self.handle.telemetry().mode;
        if current == EngineMode::Killed {
            return Err(IpcError::Forbidden(
                "cannot pause from Killed mode; use reset_kill --confirm first".to_owned(),
            ));
        }
        let next = self
            .handle
            .apply_mode_trigger(
                ModeTrigger::ManualPause {
                    reason: "ipc pause".to_owned(),
                },
                unix_now_ms(),
            )
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!("engine mode -> {next:?}"))
    }

    async fn resume(&self) -> Result<String, IpcError> {
        let current = self.handle.telemetry().mode;
        if current != EngineMode::Paused && current != EngineMode::Ready {
            return Err(IpcError::Forbidden(format!(
                "cannot resume from {current:?} mode"
            )));
        }
        let next = self
            .handle
            .apply_mode_trigger(ModeTrigger::ManualResumeFromPause, unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!("engine mode -> {next:?}"))
    }

    async fn set_param(&self, key: &str, value: &str) -> Result<String, IpcError> {
        self.handle
            .set_param(key.to_owned(), value.to_owned())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(|err| {
                if err.to_ascii_lowercase().contains("loosen") {
                    IpcError::LoosenRejected(err)
                } else {
                    IpcError::BadRequest(err)
                }
            })
    }

    async fn disable_strategy(&self, id: &str) -> Result<String, IpcError> {
        self.handle
            .disable_strategy(id.to_owned())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(IpcError::BadRequest)
    }

    async fn close_position(&self, position_id: Uuid) -> Result<String, IpcError> {
        self.handle
            .close_position(position_id, unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(IpcError::NotFound)
    }

    async fn panic_flatten(&self) -> Result<String, IpcError> {
        let (next, flattened) = self
            .handle
            .panic_flatten(unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!(
            "panic flatten complete ({flattened} positions closed); engine mode -> {next:?}"
        ))
    }

    async fn reset_kill(&self) -> Result<String, IpcError> {
        let next = self
            .handle
            .apply_mode_trigger(
                ModeTrigger::ManualResetKill { confirmed: true },
                unix_now_ms(),
            )
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!("kill reset; engine mode -> {next:?}"))
    }

    async fn ack_alert(&self, id: &str) -> Result<String, IpcError> {
        Ok(format!("alert '{id}' acknowledged"))
    }

    async fn switch_venue(
        &self,
        target: bot_types::Venue,
        policy: bot_ipc::protocol::OnOpenPositionsArg,
        actor: &str,
    ) -> Result<String, IpcError> {
        let mapped = match policy {
            bot_ipc::protocol::OnOpenPositionsArg::Drain => bot_venue::SwitchPolicy::KeepManaging,
            bot_ipc::protocol::OnOpenPositionsArg::Refuse => bot_venue::SwitchPolicy::RequireFlat,
            bot_ipc::protocol::OnOpenPositionsArg::Flatten => bot_venue::SwitchPolicy::Flatten,
        };
        let report = self
            .handle
            .switch_venue(target, mapped, actor.to_owned(), unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?
            .map_err(IpcError::BadRequest)?;
        Ok(format!(
            "switched {:?} -> {:?} (old venue {:?}, canceled {} entry orders)",
            report.from, report.to, report.old_venue_state, report.canceled_entry_orders
        ))
    }
}

/// Spawn a lightweight bridge task that forwards [`EngineActor`] telemetry updates to [`TelemetryHub`].
#[must_use]
pub fn spawn_telemetry_bridge(
    mut rx: watch::Receiver<TelemetrySnapshot>,
    hub: Arc<TelemetryHub>,
    build_mode: TradingMode,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while rx.changed().await.is_ok() {
            let snap = rx.borrow_and_update().clone();
            let delta = DeltaPayload {
                venues: Some(VenuesSummary {
                    active: snap.venue,
                    spot: snap.spot_state,
                    futures: snap.futures_state,
                    last_switch_ms: 0,
                    switches_today: 0,
                }),
                engine: Some(EngineSummary {
                    mode: snap.mode,
                    build_mode,
                    uptime_s: 0,
                    ws_latency_ms: 0,
                    clock_drift_ms: 0,
                    rate: RateStatus {
                        weight_used: 0,
                        weight_limit: 6000,
                        orders_10s: 0,
                    },
                }),
                ..DeltaPayload::default()
            };
            hub.publish(ServerMessage::Delta(delta), unix_now_ms())
                .await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use bot_engine::EngineActor;
    use bot_exchange::VenueAdapter;
    use bot_exec::{PaperBroker, PaperBrokerConfig};
    use bot_store::SqliteStore;

    #[tokio::test]
    async fn engine_ipc_backend_mutates_real_engine_actor_and_returns_live_snapshot() {
        let cfg = BotConfig::default();
        let store = SqliteStore::open_in_memory().expect("in-memory sqlite");
        let broker = Arc::new(PaperBroker::new(PaperBrokerConfig::default()));
        broker.activate().await.expect("activate paper broker");
        let event_rx = broker.take_event_rx().expect("event rx");

        let actor = EngineActor::bootstrap(
            cfg.clone(),
            "b01".to_owned(),
            store,
            broker,
            Decimal::from(10_000_u32),
            1_700_000_000_000,
        )
        .await
        .expect("bootstrap engine");
        let (handle, join) = actor.spawn(event_rx);

        let backend = EngineIpcBackend::new(cfg, handle.clone());
        let hub = Arc::new(TelemetryHub::new(32));
        let (_cid, mut sub_rx) = hub.subscribe().await;
        let bridge =
            spawn_telemetry_bridge(handle.telemetry_rx(), Arc::clone(&hub), TradingMode::Paper);

        assert_eq!(backend.engine_mode().await, EngineMode::Ready);

        // Tighten-only set_param must succeed for tightening and reject loosening!
        assert!(
            backend
                .set_param("risk.max_open_positions", "2")
                .await
                .is_ok()
        );
        assert!(
            backend
                .set_param("risk.max_open_positions", "9")
                .await
                .is_err()
        );
        let snap = backend.snapshot().await;
        assert_eq!(snap.risk.max_positions, 2);

        // disable_strategy wired directly to EngineActor.
        assert!(backend.disable_strategy("breakout").await.is_ok());
        assert!(backend.disable_strategy("unknown").await.is_err());

        backend.pause().await.expect("pause");
        assert_eq!(handle.telemetry().mode, EngineMode::Paused);

        backend.panic_flatten().await.expect("panic");
        assert_eq!(handle.telemetry().mode, EngineMode::Killed);

        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), sub_rx.recv())
            .await
            .expect("telemetry delta timeout")
            .expect("telemetry delta frame");
        assert!(matches!(frame.body, ServerMessage::Delta(_)));

        bridge.abort();
        let _ = handle.shutdown(1_700_000_001_000).await;
        let _ = join.await;
    }
}
