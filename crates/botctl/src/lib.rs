#![forbid(unsafe_code)]
//! Library implementation for `binance-botctl` (`keygen`, `config validate`, UDS control commands).

pub mod cli;
pub mod client;
pub mod keygen;

use anyhow::{Context, Result, bail};
use bot_config::{ConfigSourcePaths, load_config, load_from_paths};
use bot_ipc::{CommandKind, ConfirmAction};

use crate::cli::{Cli, Command, ConfigSubcommand};
use crate::client::UdsClient;
use crate::keygen::generate_ed25519_keypair;

/// Execute a parsed [`Cli`] command and return human-readable (or JSON) output.
///
/// # Errors
/// Returns an error if validation, keygen, confirmation check, or IPC execution fails.
#[allow(clippy::too_many_lines)]
pub async fn run_cli(cli: Cli) -> Result<String> {
    match cli.command {
        Command::Keygen { out, name } => {
            let pair = generate_ed25519_keypair(&out, &name)?;
            Ok(format!(
                "Generated Ed25519 keypair:\n  private (0600): {}\n  public  (0644): {}",
                pair.private_path.display(),
                pair.public_path.display()
            ))
        }
        Command::Config { subcommand } => match subcommand {
            ConfigSubcommand::Validate { config } => {
                let loaded = if let Some(path) = config {
                    load_from_paths(&ConfigSourcePaths {
                        base_file: Some(path.clone()),
                        conf_d_dir: None,
                        env_prefix: Some("BOT__".to_owned()),
                    })
                    .with_context(|| {
                        format!("configuration validation failed for {}", path.display())
                    })?
                } else {
                    load_config().context("configuration validation failed")?
                };
                Ok(format!(
                    "Configuration valid (mode={:?}, spot_enabled={}, futures_enabled={})",
                    loaded.general.mode, loaded.venues.spot.enabled, loaded.venues.futures.enabled
                ))
            }
        },
        Command::Status { socket, json } => {
            let client = UdsClient::connect(&socket).await?;
            let snap = client.snapshot();
            if json {
                Ok(serde_json::to_string_pretty(snap)?)
            } else {
                Ok(format!(
                    "Mode: {:?} ({:?}) | Active venue: {} | Equity: {} | Open positions: {}/{} | BTC Guard: {}",
                    snap.engine.mode,
                    snap.engine.build_mode,
                    snap.venues.active,
                    snap.risk.equity,
                    snap.risk.open_positions,
                    snap.risk.max_positions,
                    snap.risk.btc_guard
                ))
            }
        }
        Command::Pause { socket } => {
            let mut client = UdsClient::connect(&socket).await?;
            let res = client.send_command(CommandKind::Pause).await?;
            if !res.ok {
                bail!("pause rejected ({}): {}", res.code, res.message);
            }
            Ok(res.message)
        }
        Command::Resume { socket } => {
            let mut client = UdsClient::connect(&socket).await?;
            let res = client.send_command(CommandKind::Resume).await?;
            if !res.ok {
                bail!("resume rejected ({}): {}", res.code, res.message);
            }
            Ok(res.message)
        }
        Command::PanicFlatten { confirm, socket } => {
            if !confirm {
                bail!(
                    "refusing to run panic-flatten without --confirm (destructive emergency action)"
                );
            }
            let mut client = UdsClient::connect(&socket).await?;
            let res = client
                .send_confirmed_command(ConfirmAction::PanicFlatten)
                .await?;
            if !res.ok {
                bail!("panic-flatten rejected ({}): {}", res.code, res.message);
            }
            Ok(res.message)
        }
        Command::ResetKill { confirm, socket } => {
            if !confirm {
                bail!(
                    "refusing to run reset-kill without --confirm (verify root cause before resetting Killed mode)"
                );
            }
            let mut client = UdsClient::connect(&socket).await?;
            let res = client
                .send_confirmed_command(ConfirmAction::ResetKill)
                .await?;
            if !res.ok {
                bail!("reset-kill rejected ({}): {}", res.code, res.message);
            }
            Ok(res.message)
        }
        Command::SwitchVenue {
            target,
            on_open_positions,
            confirm,
            socket,
        } => run_switch_venue(&target, &on_open_positions, confirm, &socket).await,
    }
}

async fn run_switch_venue(
    target: &str,
    on_open_positions: &str,
    confirm: bool,
    socket: &std::path::Path,
) -> Result<String> {
    let venue = match target.trim().to_ascii_lowercase().as_str() {
        "spot" => bot_types::Venue::Spot,
        "futures" | "usds-futures" | "usdm" => bot_types::Venue::UsdsFutures,
        other => bail!("invalid --target '{other}' (expected spot|futures)"),
    };
    let policy = match on_open_positions.trim().to_ascii_lowercase().as_str() {
        "drain" | "keep_managing" => bot_ipc::protocol::OnOpenPositionsArg::Drain,
        "refuse" | "require_flat" => bot_ipc::protocol::OnOpenPositionsArg::Refuse,
        "flatten" => bot_ipc::protocol::OnOpenPositionsArg::Flatten,
        other => bail!("invalid --on-open-positions '{other}' (expected drain|refuse|flatten)"),
    };
    if policy == bot_ipc::protocol::OnOpenPositionsArg::Flatten && !confirm {
        bail!("refusing switch-venue with --on-open-positions flatten without --confirm");
    }
    let mut client = UdsClient::connect(socket).await?;
    let res = client.send_switch_venue(venue, policy).await?;
    if !res.ok {
        bail!("switch-venue rejected ({}): {}", res.code, res.message);
    }
    Ok(res.message)
}
