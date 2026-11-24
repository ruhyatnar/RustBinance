#![forbid(unsafe_code)]
//! Binance Rust trading daemon (`binance-botd`, headless engine).

mod ipc_backend;
mod logging;
mod sd_notify;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use bot_config::{BotConfig, ConfigSourcePaths, load_from_paths};
use bot_engine::EngineActor;
use bot_exchange::VenueAdapter;
use bot_exec::{PaperBroker, PaperBrokerConfig};
use bot_ipc::{
    CommandDispatcher, ConfirmNonceStore, PeerAuthPolicy, TelemetryHub, UdsServerConfig, serve_uds,
};
use bot_store::SqliteStore;
use bot_types::{TradingMode, Venue};
use ipc_backend::{EngineIpcBackend, spawn_telemetry_bridge};
use rust_decimal::Decimal;
use time::OffsetDateTime;
use tracing::{info, warn};

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

/// Hard Rule #2: never allow `mode=live` without an existing `live_confirmation_file`
/// for every enabled venue.
fn enforce_live_confirmation_guard(cfg: &BotConfig) -> Result<()> {
    if cfg.general.mode != TradingMode::Live {
        return Ok(());
    }
    if cfg.venues.spot.enabled && !Path::new(&cfg.venues.spot.live_confirmation_file).is_file() {
        bail!(
            "refusing to start in live mode: spot live_confirmation_file '{}' does not exist",
            cfg.venues.spot.live_confirmation_file
        );
    }
    if cfg.venues.futures.enabled
        && !Path::new(&cfg.venues.futures.live_confirmation_file).is_file()
    {
        bail!(
            "refusing to start in live mode: futures live_confirmation_file '{}' does not exist",
            cfg.venues.futures.live_confirmation_file
        );
    }
    Ok(())
}

/// Refuse to silently fall back to `PaperBroker` when `mode` is `testnet` or `live`
/// before `VenueManager` (M16) wires real exchange adapters into `botd`.
fn enforce_supported_runtime_mode(cfg: &BotConfig) -> Result<()> {
    if cfg.general.mode != TradingMode::Paper {
        bail!(
            "refusing to start binance-botd in {:?} mode: real exchange adapters are wired in M16 (VenueManager); only mode=paper is supported currently",
            cfg.general.mode
        );
    }
    Ok(())
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
    enforce_supported_runtime_mode(&cfg)?;

    let db_path = Path::new(&cfg.storage.db_path);
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

    let initial_equity = if cfg.venues.spot.capital_allocation_usd > Decimal::ZERO {
        cfg.venues.spot.capital_allocation_usd
    } else {
        Decimal::from(10_000_u32)
    };

    let broker = Arc::new(PaperBroker::new(PaperBrokerConfig {
        initial_quote_balance: initial_equity,
        ..PaperBrokerConfig::default()
    }));
    broker
        .activate()
        .await
        .context("failed to activate paper broker")?;
    let event_rx = broker
        .take_event_rx()
        .context("failed to take paper broker event channel")?;

    let now_ms = unix_now_ms();
    let actor = EngineActor::bootstrap(
        cfg.clone(),
        "b01".to_owned(),
        store,
        broker,
        initial_equity,
        now_ms,
    )
    .await
    .context("failed to bootstrap EngineActor")?;
    let (engine_handle, engine_join) = actor.spawn(event_rx);

    let backend = Arc::new(EngineIpcBackend::new(cfg.clone(), engine_handle.clone()));
    let dispatcher = Arc::new(CommandDispatcher::new(
        backend,
        None,
        ConfirmNonceStore::default(),
    ));
    let hub = Arc::new(TelemetryHub::new(256));
    let telemetry_bridge = spawn_telemetry_bridge(
        engine_handle.telemetry_rx(),
        Arc::clone(&hub),
        cfg.general.mode,
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

    info!(
        mode = ?cfg.general.mode,
        venue = %Venue::Spot,
        uds_path = %uds_path.display(),
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
    if let Some(wd) = watchdog_task {
        wd.abort();
    }

    let canceled_entries = engine_handle.shutdown(unix_now_ms()).await.unwrap_or(0);
    let _ = engine_join.await;
    telemetry_bridge.abort();
    uds_handle.abort();

    info!(
        canceled_entry_orders = canceled_entries,
        "binance-botd stopped cleanly"
    );
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
    fn enforce_supported_runtime_mode_rejects_testnet_and_live() {
        let mut cfg = BotConfig::default();
        assert!(enforce_supported_runtime_mode(&cfg).is_ok());

        cfg.general.mode = TradingMode::Testnet;
        assert!(enforce_supported_runtime_mode(&cfg).is_err());

        cfg.general.mode = TradingMode::Live;
        assert!(enforce_supported_runtime_mode(&cfg).is_err());
    }
}
