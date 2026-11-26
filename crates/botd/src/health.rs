//! Periodic runtime health probe for SQLite liveness (`SELECT 1`) and
//! Binance exchange maintenance detection (`REST /ping` + `GatewayError::is_maintenance()`).

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use bot_config::BotConfig;
use bot_engine::EngineHandle;
use bot_exchange::futures::FuturesRestClient;
use bot_exchange::{GatewayError, RateLimitTracker, SpotRestClient};
use bot_store::SqliteStore;
use bot_types::{TradingMode, Venue};
use tokio::sync::{Mutex, watch};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;
use tracing::{debug, info, warn};

/// Default periodic health probe interval (15 seconds).
pub const DEFAULT_HEALTH_PROBE_INTERVAL: Duration = Duration::from_secs(15);

/// Result of a single health probe cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HealthProbeOutcome {
    /// `true` when the active exchange REST probe reports maintenance/service unavailability.
    pub exchange_maintenance: bool,
    /// `true` when `SqliteStore::ping()` succeeded.
    pub db_ok: bool,
}

/// Abstract exchange ping probe allowing offline deterministic testing and paper-mode no-op.
#[async_trait]
pub trait ExchangePinger: Send + Sync {
    /// Ping the exchange REST endpoint for `venue`.
    async fn ping(&self, venue: Venue) -> Result<(), GatewayError>;
}

/// No-op pinger used in `TradingMode::Paper` (makes zero network calls).
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopExchangePinger;

#[async_trait]
impl ExchangePinger for NoopExchangePinger {
    async fn ping(&self, _venue: Venue) -> Result<(), GatewayError> {
        Ok(())
    }
}

/// Live/Testnet REST pinger backed by [`SpotRestClient`] and/or [`FuturesRestClient`].
pub struct RestExchangePinger {
    spot: Option<SpotRestClient>,
    futures: Option<FuturesRestClient>,
}

impl RestExchangePinger {
    /// Construct a [`RestExchangePinger`] for enabled venues in `Testnet` or `Live` mode.
    ///
    /// # Errors
    /// Returns [`GatewayError::Http`] if an underlying HTTP client fails to initialize.
    pub fn from_config(cfg: &BotConfig, timeout: Duration) -> Result<Self, GatewayError> {
        let is_live = cfg.general.mode == TradingMode::Live;
        let spot = if cfg.venues.spot.enabled {
            let url = if is_live {
                SpotRestClient::PRODUCTION_URL
            } else {
                SpotRestClient::TESTNET_URL
            };
            Some(SpotRestClient::new(url, timeout)?)
        } else {
            None
        };
        let futures = if cfg.venues.futures.enabled {
            let url = if is_live {
                FuturesRestClient::PRODUCTION_URL.to_owned()
            } else {
                FuturesRestClient::TESTNET_URL.to_owned()
            };
            Some(FuturesRestClient::new(
                url,
                timeout,
                None,
                Arc::new(RateLimitTracker::new()),
            )?)
        } else {
            None
        };
        Ok(Self { spot, futures })
    }
}

#[async_trait]
impl ExchangePinger for RestExchangePinger {
    async fn ping(&self, venue: Venue) -> Result<(), GatewayError> {
        match venue {
            Venue::Spot => match &self.spot {
                Some(client) => client.ping().await,
                None => Ok(()),
            },
            Venue::UsdsFutures => match &self.futures {
                Some(client) => client.ping().await,
                None => Ok(()),
            },
        }
    }
}

/// Periodic health probe runner that checks SQLite liveness and exchange maintenance,
/// forwarding state transitions to [`EngineHandle`].
pub struct HealthProbeRunner {
    store: Arc<Mutex<SqliteStore>>,
    pinger: Arc<dyn ExchangePinger>,
    engine: EngineHandle,
    interval: Duration,
    last_maintenance: Option<bool>,
    last_db_ok: Option<bool>,
}

impl HealthProbeRunner {
    /// Construct a new [`HealthProbeRunner`].
    #[must_use]
    pub fn new(
        store: Arc<Mutex<SqliteStore>>,
        pinger: Arc<dyn ExchangePinger>,
        engine: EngineHandle,
        interval: Duration,
    ) -> Self {
        Self {
            store,
            pinger,
            engine,
            interval,
            last_maintenance: None,
            last_db_ok: None,
        }
    }

    /// Execute a single probe cycle and forward any state transitions to [`EngineHandle`].
    pub async fn probe_once(&mut self) -> HealthProbeOutcome {
        let db_ok = {
            let guard = self.store.lock().await;
            match guard.ping() {
                Ok(()) => true,
                Err(err) => {
                    warn!(error = %err, "SQLite active health probe failed");
                    false
                }
            }
        };

        let active_venue = self.engine.telemetry().venue;
        let (exchange_maintenance, maint_reason) = match self.pinger.ping(active_venue).await {
            Ok(()) => (false, "exchange ping ok".to_owned()),
            Err(err) if err.is_maintenance() => {
                let reason = err.to_string();
                warn!(
                    venue = ?active_venue,
                    error = %reason,
                    "exchange maintenance detected via REST ping"
                );
                (true, reason)
            }
            Err(err) => {
                let preserved = self.last_maintenance.unwrap_or(false);
                debug!(
                    venue = ?active_venue,
                    error = %err,
                    preserved_maintenance = preserved,
                    "exchange REST ping failed with non-maintenance error; preserving prior maintenance state"
                );
                (preserved, String::new())
            }
        };

        // Forward maintenance transitions:
        // - Entering maintenance (None/Some(false) -> true): pause engine.
        // - Clearing maintenance (Some(true) -> false): clear flag, stay Paused until operator Resume.
        let should_notify_maint = matches!(
            (self.last_maintenance, exchange_maintenance),
            (None | Some(false), true) | (Some(true), false)
        );
        if should_notify_maint {
            if let Err(err) = self
                .engine
                .set_maintenance_paused(exchange_maintenance, maint_reason)
            {
                warn!(error = %err, "failed to forward maintenance state to engine");
            } else {
                self.last_maintenance = Some(exchange_maintenance);
            }
        } else if self.last_maintenance.is_none() {
            self.last_maintenance = Some(false);
        }

        if self.last_db_ok != Some(db_ok) {
            if let Err(err) = self.engine.report_db_health(db_ok) {
                warn!(error = %err, "failed to forward DB health state to engine");
            } else {
                self.last_db_ok = Some(db_ok);
            }
        }

        HealthProbeOutcome {
            exchange_maintenance,
            db_ok,
        }
    }

    /// Spawn the periodic probe background task, cancelling cleanly when `shutdown_rx` becomes `true`
    /// or its sender is dropped.
    #[must_use]
    pub fn spawn(mut self, mut shutdown_rx: watch::Receiver<bool>) -> JoinHandle<()> {
        tokio::spawn(async move {
            if *shutdown_rx.borrow() {
                info!("health probe runner already signaled to shut down");
                return;
            }
            let mut ticker = tokio::time::interval(self.interval);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    biased;
                    changed = shutdown_rx.changed() => {
                        if changed.is_err() || *shutdown_rx.borrow() {
                            info!("health probe runner stopping on shutdown signal");
                            break;
                        }
                    }
                    _ = ticker.tick() => {
                        tokio::select! {
                            biased;
                            changed = shutdown_rx.changed() => {
                                if changed.is_err() || *shutdown_rx.borrow() {
                                    info!("health probe runner stopping on shutdown signal during active probe");
                                    break;
                                }
                            }
                            _ = self.probe_once() => {}
                        }
                    }
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU8, Ordering};

    use bot_engine::EngineActor;
    use bot_exchange::VenueAdapter;
    use bot_exec::{PaperBroker, PaperBrokerConfig};
    use bot_types::EngineMode;
    use rust_decimal::Decimal;

    use super::*;

    /// 0 = Ok, 1 = Maintenance (HTTP 503), 2 = Non-maintenance network error.
    struct MockPinger {
        mode: AtomicU8,
    }

    impl MockPinger {
        fn new(initial: u8) -> Self {
            Self {
                mode: AtomicU8::new(initial),
            }
        }

        fn set_mode(&self, m: u8) {
            self.mode.store(m, Ordering::SeqCst);
        }
    }

    #[async_trait]
    impl ExchangePinger for MockPinger {
        async fn ping(&self, _venue: Venue) -> Result<(), GatewayError> {
            match self.mode.load(Ordering::SeqCst) {
                0 => Ok(()),
                1 => Err(GatewayError::UnknownNeedsQuery {
                    client_order_id: None,
                    reason: "HTTP 503 System maintenance in progress".to_owned(),
                }),
                _ => Err(GatewayError::Transport("transient timeout".to_owned())),
            }
        }
    }

    async fn spawn_test_engine() -> (EngineHandle, JoinHandle<EngineActor>) {
        let cfg = BotConfig::default();
        let store = SqliteStore::open_in_memory().expect("in-memory sqlite");
        let broker = Arc::new(PaperBroker::new(PaperBrokerConfig::default()));
        broker.activate().await.expect("activate paper broker");
        let event_rx = broker.take_event_rx().expect("event rx");

        let actor = EngineActor::bootstrap(
            cfg,
            "b01".to_owned(),
            store,
            broker,
            Decimal::from(10_000_u32),
            1_700_000_000_000,
        )
        .await
        .expect("bootstrap");
        actor.spawn(event_rx)
    }

    #[tokio::test]
    async fn probe_once_triggers_maintenance_pause_and_stays_paused_on_clear() {
        let (handle, join) = spawn_test_engine().await;
        let probe_store = Arc::new(Mutex::new(
            SqliteStore::open_in_memory().expect("probe sqlite"),
        ));
        let pinger = Arc::new(MockPinger::new(0));
        let mut runner = HealthProbeRunner::new(
            probe_store,
            pinger.clone(),
            handle.clone(),
            Duration::from_secs(15),
        );

        // Healthy tick: stays Ready, db_ok = true, maintenance_paused = false.
        let out = runner.probe_once().await;
        assert_eq!(
            out,
            HealthProbeOutcome {
                exchange_maintenance: false,
                db_ok: true,
            }
        );
        let snap = handle.full_snapshot(1_700_000_001_000).await.expect("snap");
        assert_eq!(snap.telemetry.mode, EngineMode::Ready);
        assert!(!snap.telemetry.maintenance_paused);
        assert!(snap.telemetry.db_ok);

        // Maintenance detected: auto-pauses Ready -> Paused, maintenance_paused = true.
        pinger.set_mode(1);
        let out = runner.probe_once().await;
        assert_eq!(
            out,
            HealthProbeOutcome {
                exchange_maintenance: true,
                db_ok: true,
            }
        );
        let snap = handle.full_snapshot(1_700_000_002_000).await.expect("snap");
        assert_eq!(snap.telemetry.mode, EngineMode::Paused);
        assert!(snap.telemetry.maintenance_paused);

        // Maintenance clears: maintenance_paused = false, stays Paused (Opsi 1)!
        pinger.set_mode(0);
        let out = runner.probe_once().await;
        assert_eq!(
            out,
            HealthProbeOutcome {
                exchange_maintenance: false,
                db_ok: true,
            }
        );
        let snap = handle.full_snapshot(1_700_000_003_000).await.expect("snap");
        assert_eq!(snap.telemetry.mode, EngineMode::Paused);
        assert!(!snap.telemetry.maintenance_paused);

        let _ = handle.shutdown(1_700_000_004_000).await;
        let _ = join.await;
    }

    #[tokio::test]
    async fn probe_once_ignores_non_maintenance_gateway_errors() {
        let (handle, join) = spawn_test_engine().await;
        let probe_store = Arc::new(Mutex::new(
            SqliteStore::open_in_memory().expect("probe sqlite"),
        ));
        let pinger = Arc::new(MockPinger::new(2));
        let mut runner =
            HealthProbeRunner::new(probe_store, pinger, handle.clone(), Duration::from_secs(15));

        let out = runner.probe_once().await;
        assert_eq!(
            out,
            HealthProbeOutcome {
                exchange_maintenance: false,
                db_ok: true,
            }
        );
        let snap = handle.full_snapshot(1_700_000_001_000).await.expect("snap");
        assert_eq!(snap.telemetry.mode, EngineMode::Ready);
        assert!(!snap.telemetry.maintenance_paused);

        let _ = handle.shutdown(1_700_000_002_000).await;
        let _ = join.await;
    }

    #[tokio::test]
    async fn probe_once_non_maintenance_error_does_not_clear_active_maintenance() {
        let (handle, join) = spawn_test_engine().await;
        let probe_store = Arc::new(Mutex::new(
            SqliteStore::open_in_memory().expect("probe sqlite"),
        ));
        let pinger = Arc::new(MockPinger::new(1));
        let mut runner = HealthProbeRunner::new(
            probe_store,
            pinger.clone(),
            handle.clone(),
            Duration::from_secs(15),
        );

        // 1. Maintenance detected -> maintenance_paused = true, mode = Paused.
        let out = runner.probe_once().await;
        assert_eq!(
            out,
            HealthProbeOutcome {
                exchange_maintenance: true,
                db_ok: true,
            }
        );
        let snap = handle.full_snapshot(1_700_000_001_000).await.expect("snap");
        assert_eq!(snap.telemetry.mode, EngineMode::Paused);
        assert!(snap.telemetry.maintenance_paused);

        // 2. Subsequent non-maintenance transport error must NOT clear maintenance_paused.
        pinger.set_mode(2);
        let out = runner.probe_once().await;
        assert_eq!(
            out,
            HealthProbeOutcome {
                exchange_maintenance: true,
                db_ok: true,
            }
        );
        let snap = handle.full_snapshot(1_700_000_002_000).await.expect("snap");
        assert_eq!(snap.telemetry.mode, EngineMode::Paused);
        assert!(
            snap.telemetry.maintenance_paused,
            "non-maintenance transport error must not falsely clear maintenance_paused"
        );

        // 3. Only Ok(()) clears maintenance_paused (while staying Paused per Opsi 1).
        pinger.set_mode(0);
        let out = runner.probe_once().await;
        assert_eq!(
            out,
            HealthProbeOutcome {
                exchange_maintenance: false,
                db_ok: true,
            }
        );
        let snap = handle.full_snapshot(1_700_000_003_000).await.expect("snap");
        assert_eq!(snap.telemetry.mode, EngineMode::Paused);
        assert!(!snap.telemetry.maintenance_paused);

        let _ = handle.shutdown(1_700_000_004_000).await;
        let _ = join.await;
    }

    #[tokio::test]
    async fn paper_noop_pinger_and_spawned_probe_cancel_cleanly_on_shutdown() {
        let (handle, join) = spawn_test_engine().await;
        let probe_store = Arc::new(Mutex::new(
            SqliteStore::open_in_memory().expect("probe sqlite"),
        ));
        let runner = HealthProbeRunner::new(
            probe_store,
            Arc::new(NoopExchangePinger),
            handle.clone(),
            Duration::from_millis(20),
        );
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = runner.spawn(shutdown_rx);

        // Cancel via shutdown token; task must exit promptly.
        shutdown_tx.send(true).expect("send shutdown");
        tokio::time::timeout(Duration::from_secs(2), task)
            .await
            .expect("health probe shutdown timeout")
            .expect("health probe task join");

        let _ = handle.shutdown(1_700_000_002_000).await;
        let _ = join.await;
    }

    struct PendingPinger {
        entered: Arc<tokio::sync::Notify>,
    }

    #[async_trait]
    impl ExchangePinger for PendingPinger {
        async fn ping(&self, _venue: Venue) -> Result<(), GatewayError> {
            self.entered.notify_one();
            std::future::pending::<Result<(), GatewayError>>().await
        }
    }

    #[tokio::test]
    async fn spawned_probe_cancels_in_flight_ping_promptly_on_shutdown() {
        let (handle, join) = spawn_test_engine().await;
        let probe_store = Arc::new(Mutex::new(
            SqliteStore::open_in_memory().expect("probe sqlite"),
        ));
        let entered = Arc::new(tokio::sync::Notify::new());
        let pinger = Arc::new(PendingPinger {
            entered: Arc::clone(&entered),
        });
        let runner =
            HealthProbeRunner::new(probe_store, pinger, handle.clone(), Duration::from_secs(60));
        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let task = runner.spawn(shutdown_rx);

        // Wait until the first tick has entered the never-resolving ping().
        tokio::time::timeout(Duration::from_secs(1), entered.notified())
            .await
            .expect("ping should start promptly on first interval tick");

        // Signaling shutdown must cancel the in-flight probe immediately (well under 1s).
        shutdown_tx.send(true).expect("send shutdown");
        tokio::time::timeout(Duration::from_millis(500), task)
            .await
            .expect("in-flight health probe must cancel promptly on shutdown")
            .expect("health probe task join");

        let _ = handle.shutdown(1_700_000_003_000).await;
        let _ = join.await;
    }
}
