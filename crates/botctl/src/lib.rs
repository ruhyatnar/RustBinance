#![forbid(unsafe_code)]
//! Library implementation for `binance-botctl` (`keygen`, `config validate`, UDS control commands).

pub mod cli;
pub mod client;
pub mod doctor;
pub mod dry_run_signal;
pub mod key_audit;
pub mod keygen;
pub mod probes;

use anyhow::{Context, Result, bail};
use bot_config::{ConfigSourcePaths, load_config, load_from_paths};
use bot_ipc::{CommandKind, ConfirmAction};

use crate::cli::{
    Cli, Command, ConfigSubcommand, DryRunSubcommand, KeySubcommand, ProbeSubcommand,
};
use crate::client::UdsClient;
use crate::key_audit::{fingerprint_key_file, inspect_key_file};
use crate::keygen::generate_ed25519_keypair;

/// Error returned when a diagnostic command completed but reported failures.
///
/// The `output` is the full report (text or JSON) and should be printed to stdout
/// before exiting with a non-zero status.
#[derive(Debug)]
pub struct ReportFailure {
    /// Rendered report.
    pub output: String,
}

impl std::fmt::Display for ReportFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("diagnostic report contains failures")
    }
}

impl std::error::Error for ReportFailure {}

/// Return `Ok(output)` when `ok`, otherwise `Err(ReportFailure)` carrying the output.
fn report_result(ok: bool, output: String) -> Result<String> {
    if ok {
        Ok(output)
    } else {
        Err(ReportFailure { output }.into())
    }
}

fn run_key(subcommand: KeySubcommand) -> Result<String> {
    match subcommand {
        KeySubcommand::Fingerprint { file, json } => {
            let fp = fingerprint_key_file(&file)?;
            if json {
                Ok(serde_json::to_string_pretty(&serde_json::json!({
                    "path": file,
                    "sha256_fingerprint": fp,
                }))?)
            } else {
                Ok(fp)
            }
        }
        KeySubcommand::Inspect { file, json } => {
            let rep = inspect_key_file(&file)?;
            let out = if json {
                serde_json::to_string_pretty(&rep)?
            } else {
                let mut s = format!(
                    "{} [{}]\n  kind: {:?}\n  fingerprint: {}\n  file mode: {} ({})\n  dir mode: {} ({})",
                    rep.path.display(),
                    if rep.is_ok() { "OK" } else { "FAIL" },
                    rep.kind,
                    rep.sha256_fingerprint,
                    rep.file_mode_octal,
                    if rep.file_mode_ok { "ok" } else { "too open" },
                    rep.dir_mode_octal,
                    if rep.dir_mode_ok { "ok" } else { "too open" },
                );
                for issue in &rep.issues {
                    s.push_str("\n  issue: ");
                    s.push_str(issue);
                }
                s
            };
            report_result(rep.is_ok(), out)
        }
    }
}

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
        Command::Key { subcommand } => run_key(subcommand),
        Command::Doctor {
            config,
            socket,
            offline,
            json,
        } => {
            let report = doctor::run_doctor(&doctor::DoctorOptions {
                config,
                socket,
                spot_rest_url: None,
                futures_rest_url: None,
                offline,
            })
            .await;
            let out = if json {
                serde_json::to_string_pretty(&report)?
            } else {
                report.render_text()
            };
            report_result(report.overall_ok, out)
        }
        Command::Probe { subcommand } => run_probe(subcommand).await,
        Command::DryRun { subcommand } => run_dry_run(subcommand).await,
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

pub(crate) fn parse_venue_arg(raw: &str) -> Result<bot_types::Venue> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "spot" => Ok(bot_types::Venue::Spot),
        "futures" | "usds-futures" | "usdm" => Ok(bot_types::Venue::UsdsFutures),
        other => bail!("invalid venue '{other}' (expected spot|futures)"),
    }
}

async fn run_probe(subcommand: ProbeSubcommand) -> Result<String> {
    match subcommand {
        ProbeSubcommand::Signed {
            venue,
            config,
            json,
        } => {
            let venue = parse_venue_arg(&venue)?;
            let report = probes::run_signed_probe(&probes::SignedProbeOptions {
                venue,
                config,
                rest_url_override: None,
            })
            .await?;
            let out = if json {
                serde_json::to_string_pretty(&report)?
            } else {
                report.render_text()
            };
            report_result(report.ok, out)
        }
        ProbeSubcommand::DryRunOrder {
            venue,
            symbol,
            offline,
            config,
            json,
        } => {
            let venue = parse_venue_arg(&venue)?;
            let report = probes::run_dry_run_order(&probes::DryRunOrderOptions {
                venue,
                symbol,
                price: None,
                qty: None,
                offline,
                config,
                rest_url_override: None,
            })
            .await?;
            if json {
                Ok(serde_json::to_string_pretty(&report)?)
            } else {
                Ok(report.render_text())
            }
        }
        ProbeSubcommand::WsLogon {
            venue,
            config,
            json,
        } => {
            let venue = parse_venue_arg(&venue)?;
            let report = probes::run_ws_logon_probe(&probes::WsLogonProbeOptions {
                venue,
                config,
                ws_url_override: None,
            })
            .await?;
            let out = if json {
                serde_json::to_string_pretty(&report)?
            } else {
                report.render_text()
            };
            report_result(report.authenticated && report.closed_cleanly, out)
        }
    }
}

async fn run_dry_run(subcommand: DryRunSubcommand) -> Result<String> {
    match subcommand {
        DryRunSubcommand::Signal {
            symbol,
            venue,
            offline,
            config,
            json,
        } => {
            let venue = parse_venue_arg(&venue)?;
            let report = dry_run_signal::run_dry_run_signal(&dry_run_signal::DryRunSignalOptions {
                venue,
                symbol,
                offline,
                config,
                rest_url_override: None,
            })
            .await?;
            if json {
                Ok(serde_json::to_string_pretty(&report)?)
            } else {
                Ok(report.render_text())
            }
        }
    }
}

async fn run_switch_venue(
    target: &str,
    on_open_positions: &str,
    confirm: bool,
    socket: &std::path::Path,
) -> Result<String> {
    let venue = parse_venue_arg(target)?;
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
