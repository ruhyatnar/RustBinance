#![forbid(unsafe_code)]
//! Binance Rust trading daemon (`binance-botd`, headless engine).

mod health;
mod ipc_backend;
mod lock;
mod logging;
mod market_runner;
mod sd_notify;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bot_config::{BotConfig, ConfigSourcePaths, load_from_paths};
use bot_engine::{EngineActor, USER_EVENT_CHANNEL_CAPACITY};
use bot_exchange::futures::{FuturesAdapter, FuturesAdapterConfig, FuturesRestClient};
use bot_exchange::{
    Ed25519Credentials, GatewayEvent, SpotAdapter, SpotAdapterConfig, SpotRestClient,
    SpotSessionConfig, VenueAdapter,
};
use bot_exec::paper_futures::PaperFuturesBroker;
use bot_exec::{PaperBroker, PaperBrokerConfig};
use bot_ipc::{
    CommandDispatcher, ConfirmNonceStore, HttpServerConfig, OriginPolicy, PeerAuthPolicy,
    RateLimiter, TelemetryHub, TokenAuth, UdsServerConfig, serve_http, serve_uds,
};
use bot_store::SqliteStore;
use bot_types::{TradingMode, Venue};
use ipc_backend::{EngineIpcBackend, VenueRateTrackers, spawn_telemetry_bridge};
use lock::ProcessLock;
use rust_decimal::Decimal;
use time::OffsetDateTime;
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};

const SPOT_PROD_WS_URL: &str = "wss://ws-api.binance.com:443/ws-api/v3";
const SPOT_TESTNET_WS_URL: &str = "wss://ws-api.testnet.binance.vision:443/ws-api/v3";
const FUTURES_PROD_WS_URL: &str = "wss://ws-fapi.binance.com/ws-fapi/v1";
const FUTURES_TESTNET_WS_URL: &str = "wss://testnet.binancefuture.com/ws-fapi/v1";

fn unix_now_ms() -> i64 {
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000).unwrap_or(0)
}

fn parse_config_arg() -> Result<Option<PathBuf>> {
    let mut args = std::env::args().skip(1);
    let mut config_path: Option<PathBuf> = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--config" | "-c" => {
                let val = args.next().context("missing path value after --config")?;
                config_path = Some(PathBuf::from(val));
            }
            "--version" | "-V" => {
                println!("binance-botd {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            other => bail!("unknown argument: {other}"),
        }
    }
    Ok(config_path)
}

/// Resolve an optional sibling `conf.d` directory next to `config_path`.
fn resolve_conf_d_dir(config_path: &Path) -> Option<PathBuf> {
    let candidate = config_path.parent()?.join("conf.d");
    candidate.is_dir().then_some(candidate)
}

/// Resolve static `web-ui` bundle directory (installed path first, then local build).
fn resolve_static_web_dir() -> Option<PathBuf> {
    for candidate in [
        Path::new("/usr/share/binance-bot/web"),
        Path::new("web-ui/build"),
    ] {
        if candidate.is_dir() && candidate.join("index.html").is_file() {
            return Some(candidate.to_path_buf());
        }
    }
    None
}

/// Hard Rule #2: never allow `mode=live` without an existing `live_confirmation_file`
/// for every enabled venue.
fn enforce_live_confirmation_guard(cfg: &BotConfig) -> Result<()> {
    if cfg.general.mode != TradingMode::Live {
        return Ok(());
    }
    if cfg.venues.spot.enabled {
        check_confirmation_file("spot", &cfg.venues.spot.live_confirmation_file)?;
    }
    if cfg.venues.futures.enabled {
        check_confirmation_file("futures", &cfg.venues.futures.live_confirmation_file)?;
    }
    Ok(())
}

/// The confirmation file must exist AND contain text, so a bare `touch` is not enough.
fn check_confirmation_file(venue: &str, path: &str) -> Result<()> {
    let content = std::fs::read_to_string(path).map_err(|_| {
        anyhow::anyhow!("refusing to start in live mode: {venue} live_confirmation_file '{path}' is missing or unreadable")
    })?;
    if content.trim().is_empty() {
        bail!("refusing to start in live mode: {venue} live_confirmation_file '{path}' is empty");
    }
    Ok(())
}

fn load_venue_credentials(
    api_key_file: &str,
    private_key_path: &str,
) -> Result<Ed25519Credentials> {
    let api_key = std::fs::read_to_string(api_key_file)
        .with_context(|| format!("failed to read API key file '{api_key_file}'"))?
        .trim()
        .to_owned();
    Ed25519Credentials::from_pkcs8_pem_file(api_key, Path::new(private_key_path))
        .with_context(|| format!("failed to load Ed25519 private key '{private_key_path}'"))
}

struct BuiltAdapters {
    primary: Arc<dyn VenueAdapter>,
    secondary: Option<Arc<dyn VenueAdapter>>,
    event_rx: mpsc::Receiver<GatewayEvent>,
    /// Keeps `event_rx` open for the full daemon lifetime even if no adapter event stream
    /// is currently attached (e.g., Futures primary before secondary activation or switch).
    event_tx_guard: mpsc::Sender<GatewayEvent>,
    rate_trackers: VenueRateTrackers,
}

/// Forward gateway user-data events from an adapter receiver into the merged engine channel.
fn forward_events(mut rx: mpsc::Receiver<GatewayEvent>, tx: mpsc::Sender<GatewayEvent>) {
    tokio::spawn(async move {
        while let Some(ev) = rx.recv().await {
            if tx.send(ev).await.is_err() {
                break;
            }
        }
    });
}

#[allow(clippy::too_many_lines)]
async fn build_adapters(cfg: &BotConfig, initial_equity: Decimal) -> Result<BuiltAdapters> {
    let (merged_tx, merged_rx) = mpsc::channel(USER_EVENT_CHANNEL_CAPACITY);

    match cfg.general.mode {
        TradingMode::Paper => {
            let spot = Arc::new(PaperBroker::new(PaperBrokerConfig {
                initial_quote_balance: initial_equity,
                ..PaperBrokerConfig::default()
            }));
            spot.activate()
                .await
                .context("failed to activate paper spot broker")?;
            if let Some(rx) = spot.take_event_rx() {
                forward_events(rx, merged_tx.clone());
            }

            let futures: Option<Arc<dyn VenueAdapter>> = if cfg.venues.futures.enabled {
                let fut_eq = if cfg.venues.futures.capital_allocation_usd > Decimal::ZERO {
                    cfg.venues.futures.capital_allocation_usd
                } else {
                    initial_equity
                };
                Some(Arc::new(PaperFuturesBroker::new(
                    fut_eq,
                    cfg.venues.futures.max_leverage,
                    Decimal::new(5, 3), // 0.5% maintenance margin rate
                )))
            } else {
                None
            };

            let (primary, secondary) = if cfg.venues.default_active == Venue::UsdsFutures
                && let Some(fut) = futures
            {
                fut.activate()
                    .await
                    .context("failed to activate paper futures broker")?;
                (fut, Some(spot as Arc<dyn VenueAdapter>))
            } else {
                (spot as Arc<dyn VenueAdapter>, futures)
            };

            Ok(BuiltAdapters {
                primary,
                secondary,
                event_rx: merged_rx,
                event_tx_guard: merged_tx,
                rate_trackers: VenueRateTrackers::default(),
            })
        }
        TradingMode::Testnet | TradingMode::Live => {
            let is_live = cfg.general.mode == TradingMode::Live;
            let timeout = Duration::from_secs(10);

            let spot_adapter: Option<Arc<SpotAdapter>> = if cfg.venues.spot.enabled {
                let creds = load_venue_credentials(
                    &cfg.venues.spot.api_key_file,
                    &cfg.venues.spot.private_key_path,
                )?;
                let adp = SpotAdapter::new(
                    SpotAdapterConfig {
                        rest_url: if is_live {
                            SpotRestClient::PRODUCTION_URL.to_owned()
                        } else {
                            SpotRestClient::TESTNET_URL.to_owned()
                        },
                        rest_timeout: timeout,
                        session: SpotSessionConfig {
                            ws_url: if is_live {
                                SPOT_PROD_WS_URL.to_owned()
                            } else {
                                SPOT_TESTNET_WS_URL.to_owned()
                            },
                            recv_window_ms: Some(cfg.binance.recv_window_ms),
                            ..SpotSessionConfig::default()
                        },
                    },
                    creds,
                )
                .context("failed to initialize SpotAdapter")?;
                Some(Arc::new(adp))
            } else {
                None
            };

            let futures_adapter: Option<Arc<FuturesAdapter>> = if cfg.venues.futures.enabled {
                let creds = Arc::new(load_venue_credentials(
                    &cfg.venues.futures.api_key_file,
                    &cfg.venues.futures.private_key_path,
                )?);
                let adp = FuturesAdapter::with_transport(
                    FuturesAdapterConfig {
                        rest_base_url: if is_live {
                            FuturesRestClient::PRODUCTION_URL.to_owned()
                        } else {
                            FuturesRestClient::TESTNET_URL.to_owned()
                        },
                        ws_api_url: if is_live {
                            FUTURES_PROD_WS_URL.to_owned()
                        } else {
                            FUTURES_TESTNET_WS_URL.to_owned()
                        },
                        max_leverage_cap: cfg.venues.futures.max_leverage,
                    },
                    creds,
                    timeout,
                )
                .context("failed to initialize FuturesAdapter")?;
                Some(Arc::new(adp))
            } else {
                None
            };

            let rate_trackers = VenueRateTrackers {
                spot: spot_adapter.as_ref().map(|s| Arc::clone(s.rate_limits())),
                futures: futures_adapter
                    .as_ref()
                    .map(|f| Arc::clone(f.rate_limits())),
            };

            // Wire user-data event streams for BOTH primary and secondary adapters so runtime
            // venue switches never drop events or close `user_events_rx`.
            if let Some(spot) = &spot_adapter
                && let Some(rx) = spot.take_event_rx()
            {
                forward_events(rx, merged_tx.clone());
            }
            // TODO(verifier): wire `futures_adapter.take_event_rx()` via `forward_events(rx, merged_tx.clone())`
            // once `exchange-fixer` adds `FuturesAdapter::take_event_rx()`.

            let (primary, secondary): (Arc<dyn VenueAdapter>, Option<Arc<dyn VenueAdapter>>) =
                match cfg.venues.default_active {
                    Venue::Spot => {
                        let p = spot_adapter.clone().context(
                            "venues.default_active=spot requires venues.spot.enabled=true",
                        )?;
                        p.activate()
                            .await
                            .context("failed to activate SpotAdapter")?;
                        (
                            p as Arc<dyn VenueAdapter>,
                            futures_adapter.map(|f| f as Arc<dyn VenueAdapter>),
                        )
                    }
                    Venue::UsdsFutures => {
                        let p = futures_adapter.clone().context(
                            "venues.default_active=futures requires venues.futures.enabled=true",
                        )?;
                        p.activate()
                            .await
                            .context("failed to activate FuturesAdapter")?;
                        (
                            p as Arc<dyn VenueAdapter>,
                            spot_adapter.map(|s| s as Arc<dyn VenueAdapter>),
                        )
                    }
                };

            Ok(BuiltAdapters {
                primary,
                secondary,
                event_rx: merged_rx,
                event_tx_guard: merged_tx,
                rate_trackers,
            })
        }
    }
}

async fn maybe_start_http_server(
    cfg: &BotConfig,
    dispatcher: Arc<CommandDispatcher<EngineIpcBackend>>,
    hub: Arc<TelemetryHub>,
) -> Option<tokio::task::JoinHandle<()>> {
    let bind_raw = cfg.ipc.web_bind.trim();
    if bind_raw.is_empty() {
        return None;
    }
    let Ok(addr) = bind_raw.parse::<SocketAddr>() else {
        warn!(web_bind = %bind_raw, "invalid ipc.web_bind address; HTTP/WS server disabled");
        return None;
    };
    let token_path = Path::new(&cfg.ipc.web_token_file);
    if !token_path.is_file() {
        warn!(
            web_token_file = %cfg.ipc.web_token_file,
            "ipc.web_token_file not found; HTTP/WS server disabled"
        );
        return None;
    }
    let ro_path = (!cfg.ipc.web_readonly_token_file.trim().is_empty())
        .then(|| Path::new(&cfg.ipc.web_readonly_token_file));
    let auth = match TokenAuth::from_files(token_path, ro_path) {
        Ok(a) => a,
        Err(err) => {
            warn!(error = %err, "failed to load HTTP bearer tokens; HTTP/WS server disabled");
            return None;
        }
    };
    let http_cfg = HttpServerConfig {
        auth,
        origin_policy: OriginPolicy::loopback_default(addr.port()),
        rate_limiter: Arc::new(RateLimiter::new(60, 20)),
    };
    let static_dir = resolve_static_web_dir();
    match serve_http(addr, dispatcher, hub, http_cfg, static_dir.clone()).await {
        Ok(handle) => {
            info!(
                web_bind = %addr,
                static_dir = ?static_dir,
                "HTTP/WS IPC server started"
            );
            Some(handle)
        }
        Err(err) => {
            warn!(error = %err, web_bind = %addr, "failed to bind HTTP/WS IPC server");
            None
        }
    }
}

/// Build the single-instance lock file path beside the SQLite database.
fn lock_file_path(db_path: &Path, mode: TradingMode) -> PathBuf {
    let mode_str = match mode {
        TradingMode::Paper => "paper",
        TradingMode::Testnet => "testnet",
        TradingMode::Live => "live",
    };
    let file_name = format!("binance-botd.{mode_str}.lock");
    match db_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
    }
}

#[allow(clippy::too_many_lines)]
async fn run_daemon(config_path: &Path) -> Result<()> {
    let cfg = load_from_paths(&ConfigSourcePaths {
        base_file: Some(config_path.to_path_buf()),
        conf_d_dir: resolve_conf_d_dir(config_path),
        env_prefix: Some("BOT__".to_owned()),
    })
    .with_context(|| format!("failed to load config from {}", config_path.display()))?;
    enforce_live_confirmation_guard(&cfg)?;

    let db_path = Path::new(&cfg.storage.db_path);
    let lock_path = lock_file_path(db_path, cfg.general.mode);
    let process_lock = ProcessLock::acquire(&lock_path).with_context(|| {
        format!(
            "failed to acquire process lock at {} (is another binance-botd running?)",
            lock_path.display()
        )
    })?;

    if let Some(parent) = db_path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("failed to create db directory {}", parent.display()))?;
    }
    let store = SqliteStore::open(db_path)
        .with_context(|| format!("failed to open sqlite store at {}", db_path.display()))?;
    store
        .integrity_check()
        .context("sqlite integrity check failed")?;
    let audit_store = SqliteStore::open(db_path)
        .with_context(|| format!("failed to open sqlite audit store at {}", db_path.display()))?;

    let initial_equity = if cfg.venues.spot.capital_allocation_usd > Decimal::ZERO {
        cfg.venues.spot.capital_allocation_usd
    } else {
        Decimal::from(10_000_u32)
    };

    let adapters = build_adapters(&cfg, initial_equity).await?;
    let active_venue = adapters.primary.venue();
    let event_tx_guard = adapters.event_tx_guard;
    let rate_trackers = adapters.rate_trackers;

    let now_ms = unix_now_ms();
    let mut actor = EngineActor::bootstrap(
        cfg.clone(),
        "b01".to_owned(),
        store,
        adapters.primary,
        initial_equity,
        now_ms,
    )
    .await
    .context("failed to bootstrap EngineActor")?;
    if let Some(sec) = adapters.secondary {
        let sec_enabled = match sec.venue() {
            Venue::Spot => cfg.venues.spot.enabled,
            Venue::UsdsFutures => cfg.venues.futures.enabled,
        };
        actor.register_secondary_adapter(sec, sec_enabled, true);
    }
    let (engine_handle, engine_join) = actor.spawn(adapters.event_rx);

    let shared_audit_store = Arc::new(Mutex::new(audit_store));
    let backend = Arc::new(
        EngineIpcBackend::new(cfg.clone(), engine_handle.clone())
            .with_rate_trackers(rate_trackers.clone()),
    );
    let dispatcher = Arc::new(CommandDispatcher::new(
        backend,
        Some(Arc::clone(&shared_audit_store)),
        ConfirmNonceStore::default(),
    ));
    let hub = Arc::new(TelemetryHub::new(256));
    let telemetry_bridge = spawn_telemetry_bridge(
        engine_handle.clone(),
        Arc::clone(&hub),
        cfg.general.mode,
        now_ms,
        rate_trackers,
    );

    let exchange_pinger: Arc<dyn health::ExchangePinger> = match cfg.general.mode {
        TradingMode::Paper => Arc::new(health::NoopExchangePinger),
        TradingMode::Testnet | TradingMode::Live => Arc::new(
            health::RestExchangePinger::from_config(&cfg, Duration::from_secs(5))
                .context("failed to initialize REST health pinger")?,
        ),
    };
    let (daemon_shutdown_tx, health_shutdown_rx) = tokio::sync::watch::channel(false);
    let health_probe_task = health::HealthProbeRunner::new(
        Arc::clone(&shared_audit_store),
        exchange_pinger,
        engine_handle.clone(),
        health::DEFAULT_HEALTH_PROBE_INTERVAL,
    )
    .spawn(health_shutdown_rx);

    let market_runner_handle = market_runner::spawn_daemon_market_runner(
        &cfg,
        active_venue,
        engine_handle.clone(),
        daemon_shutdown_tx.subscribe(),
    );

    let uds_path = PathBuf::from(&cfg.ipc.uds_path);
    let uds_handle = tokio::spawn({
        let dispatcher = Arc::clone(&dispatcher);
        let hub = Arc::clone(&hub);
        let uds_path = uds_path.clone();
        async move {
            let uds_cfg = UdsServerConfig {
                socket_path: uds_path,
                peer_auth: PeerAuthPolicy::allow_current_process(),
            };
            if let Err(err) = serve_uds(uds_cfg, dispatcher, hub).await {
                warn!(error = %err, "UDS IPC server exited with error");
            }
        }
    });

    let http_handle =
        maybe_start_http_server(&cfg, Arc::clone(&dispatcher), Arc::clone(&hub)).await;

    info!(
        mode = ?cfg.general.mode,
        venue = %active_venue,
        uds_path = %uds_path.display(),
        lock_path = %process_lock.path().display(),
        version = env!("CARGO_PKG_VERSION"),
        "binance-botd started"
    );

    if let Err(err) = sd_notify::notify_ready() {
        warn!(error = %err, "sd_notify READY=1 failed");
    }

    let watchdog_task = sd_notify::watchdog_interval().map(|interval| {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                if let Err(err) = sd_notify::notify_watchdog() {
                    warn!(error = %err, "sd_notify WATCHDOG=1 failed");
                }
            }
        })
    });

    wait_for_shutdown_signal().await;

    info!("shutdown signal received; canceling pending entry orders only (preserving protections)");
    let _ = sd_notify::notify_stopping();
    let _ = daemon_shutdown_tx.send(true);
    if let Some(wd) = watchdog_task {
        wd.abort();
    }

    market_runner_handle.shutdown().await;
    let canceled_entries = match engine_handle.shutdown(unix_now_ms()).await {
        Ok(n) => n,
        Err(err) => {
            warn!(error = %err, "engine actor shutdown failed or channel closed; canceled entry count unknown");
            0
        }
    };
    let _ = engine_join.await;
    let _ = health_probe_task.await;
    telemetry_bridge.abort();
    uds_handle.abort();
    if let Some(http) = http_handle {
        http.abort();
    }
    drop(event_tx_guard);

    info!(
        canceled_entry_orders = canceled_entries,
        "binance-botd stopped cleanly"
    );
    drop(process_lock);
    Ok(())
}

async fn wait_for_shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut sigterm) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            let _ = sigterm.recv().await;
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
}

fn main() -> Result<()> {
    logging::init_tracing()?;
    let Some(config_path) = parse_config_arg()? else {
        tracing::info!(
            mode = "paper",
            version = env!("CARGO_PKG_VERSION"),
            "binance-botd initialized (pass --config <path> to start daemon)"
        );
        return Ok(());
    };

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("failed to build tokio runtime")?;
    rt.block_on(run_daemon(&config_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_conf_d_dir_detects_sibling_directory() {
        let dir = tempfile::TempDir::new().expect("tmpdir");
        let cfg_file = dir.path().join("bot.toml");
        assert!(resolve_conf_d_dir(&cfg_file).is_none());

        let conf_d = dir.path().join("conf.d");
        std::fs::create_dir(&conf_d).expect("mkdir conf.d");
        assert_eq!(resolve_conf_d_dir(&cfg_file), Some(conf_d));
    }

    #[test]
    fn live_confirmation_guard_blocks_live_without_file_and_allows_paper_or_testnet() {
        let mut cfg = BotConfig::default();
        assert!(enforce_live_confirmation_guard(&cfg).is_ok());

        cfg.general.mode = TradingMode::Testnet;
        assert!(enforce_live_confirmation_guard(&cfg).is_ok());

        cfg.general.mode = TradingMode::Live;
        assert!(enforce_live_confirmation_guard(&cfg).is_err());
    }

    #[test]
    fn live_confirmation_guard_rejects_empty_file_accepts_nonempty() {
        let dir = tempfile::TempDir::new().expect("tmpdir");
        let f = dir.path().join("confirm");
        std::fs::write(&f, "  \n").expect("write");
        let p = f.to_str().expect("utf8");
        assert!(check_confirmation_file("spot", p).is_err());
        std::fs::write(&f, "I CONFIRM LIVE").expect("write");
        assert!(check_confirmation_file("spot", p).is_ok());
    }

    #[test]
    fn lock_file_path_uses_db_parent_and_mode() {
        let db = Path::new("/var/lib/binance-bot/state.sqlite");
        assert_eq!(
            lock_file_path(db, TradingMode::Paper),
            PathBuf::from("/var/lib/binance-bot/binance-botd.paper.lock")
        );
        assert_eq!(
            lock_file_path(db, TradingMode::Testnet),
            PathBuf::from("/var/lib/binance-bot/binance-botd.testnet.lock")
        );
        assert_eq!(
            lock_file_path(db, TradingMode::Live),
            PathBuf::from("/var/lib/binance-bot/binance-botd.live.lock")
        );
        assert_eq!(
            lock_file_path(Path::new("state.sqlite"), TradingMode::Paper),
            PathBuf::from("binance-botd.paper.lock")
        );
    }

    #[tokio::test]
    async fn build_adapters_keeps_event_channel_open_when_futures_is_default_active() {
        let mut cfg = BotConfig::default();
        cfg.general.mode = TradingMode::Paper;
        cfg.venues.futures.enabled = true;
        cfg.venues.default_active = Venue::UsdsFutures;

        let mut adapters = build_adapters(&cfg, Decimal::from(10_000_u32))
            .await
            .expect("build paper adapters with futures default");
        assert_eq!(adapters.primary.venue(), Venue::UsdsFutures);
        assert!(adapters.secondary.is_some());

        // Channel must remain open (Empty, NOT Disconnected) while event_tx_guard is held.
        assert!(matches!(
            adapters.event_rx.try_recv(),
            Err(mpsc::error::TryRecvError::Empty)
        ));

        // Verify event_tx_guard delivers events into event_rx.
        adapters
            .event_tx_guard
            .send(GatewayEvent::StreamTerminated {
                venue: Venue::UsdsFutures,
                event_time_ms: 1_700_000_000_000,
            })
            .await
            .expect("send through guard");
        let received = adapters.event_rx.recv().await.expect("event must arrive");
        assert!(matches!(
            received,
            GatewayEvent::StreamTerminated {
                venue: Venue::UsdsFutures,
                ..
            }
        ));
    }
}
