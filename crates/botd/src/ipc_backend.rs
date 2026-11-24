//! Real [`IpcBackend`] and [`TelemetryHub`] bridge wired directly to [`EngineHandle`].

use std::sync::Arc;

use async_trait::async_trait;
use bot_config::BotConfig;
use bot_engine::{EngineHandle, TelemetrySnapshot};
use bot_ipc::protocol::{
    DeltaPayload, EngineSummary, RateStatus, RiskLimitsSummary, RiskSummary, ServerMessage,
    SnapshotPayload, VenuesSummary,
};
use bot_ipc::{IpcBackend, IpcError, TelemetryHub};
use bot_risk::ModeTrigger;
use bot_types::{EngineMode, TradingMode, VenueState};
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

    fn build_snapshot(&self, snap: &TelemetrySnapshot) -> SnapshotPayload {
        let spot_state = if self.config.venues.spot.enabled {
            VenueState::Active
        } else {
            VenueState::Disabled
        };
        let futures_state = if self.config.venues.futures.enabled {
            VenueState::Active
        } else {
            VenueState::Disabled
        };
        SnapshotPayload {
            venues: VenuesSummary {
                active: snap.venue,
                spot: spot_state,
                futures: futures_state,
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
        let snap = self.handle.telemetry();
        self.build_snapshot(&snap)
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

    async fn set_param(&self, key: &str, _value: &str) -> Result<String, IpcError> {
        Err(IpcError::BadRequest(format!(
            "runtime parameter mutation '{key}' is not yet wired to EngineActor"
        )))
    }

    async fn disable_strategy(&self, id: &str) -> Result<String, IpcError> {
        Err(IpcError::BadRequest(format!(
            "runtime strategy disable '{id}' is not yet wired to EngineActor"
        )))
    }

    async fn close_position(&self, position_id: Uuid) -> Result<String, IpcError> {
        Err(IpcError::BadRequest(format!(
            "manual close_position '{position_id}' is not yet wired to EngineActor"
        )))
    }

    async fn panic_flatten(&self) -> Result<String, IpcError> {
        let next = self
            .handle
            .apply_mode_trigger(ModeTrigger::ManualPanic, unix_now_ms())
            .await
            .map_err(|err| IpcError::BadRequest(err.to_string()))?;
        Ok(format!("panic applied; engine mode -> {next:?}"))
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
    async fn engine_ipc_backend_mutates_real_engine_actor_mode() {
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

        backend.pause().await.expect("pause");
        assert_eq!(handle.telemetry().mode, EngineMode::Paused);

        backend.panic_flatten().await.expect("panic");
        assert_eq!(handle.telemetry().mode, EngineMode::Killed);

        // Bridge should have published at least one Delta frame to TelemetryHub.
        let frame = tokio::time::timeout(std::time::Duration::from_secs(2), sub_rx.recv())
            .await
            .expect("telemetry delta timeout")
            .expect("telemetry delta frame");
        assert!(matches!(frame.body, ServerMessage::Delta(_)));

        // Unwired mutations must fail explicitly rather than silently succeeding.
        assert!(
            backend
                .set_param("risk.max_open_positions", "1")
                .await
                .is_err()
        );

        bridge.abort();
        let _ = handle.shutdown(1_700_000_001_000).await;
        let _ = join.await;
    }
}
