//! CLI argument definitions for `binance-botctl` (§12.3, §12.5).

use std::path::PathBuf;

use clap::{Parser, Subcommand};

/// Default Unix domain socket path for the local trading daemon.
pub const DEFAULT_SOCKET_PATH: &str = "/run/binance-bot/bot.sock";

/// Control CLI for the Binance Rust Trading Bot (`binance-botd`).
#[derive(Debug, Parser)]
#[command(
    name = "binance-botctl",
    version,
    about = "Control CLI for binance-botd"
)]
pub struct Cli {
    /// Subcommand to execute.
    #[command(subcommand)]
    pub command: Command,
}

/// Top-level `binance-botctl` subcommands.
#[derive(Debug, Subcommand)]
pub enum Command {
    /// Generate an Ed25519 keypair (`0600` private key from creation; never overwrites).
    Keygen {
        /// Output directory for `<name>.pem` and `<name>.pub.pem`.
        #[arg(long)]
        out: PathBuf,
        /// Base filename prefix (e.g. `binance_spot_ed25519`).
        #[arg(long)]
        name: String,
    },
    /// Configuration management commands.
    Config {
        /// Config action.
        #[command(subcommand)]
        subcommand: ConfigSubcommand,
    },
    /// Query current engine, venue, risk, and position status.
    Status {
        /// Path to daemon Unix domain socket.
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
        /// Output full snapshot as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Pause new entries while keeping open position protections active.
    Pause {
        /// Path to daemon Unix domain socket.
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
    /// Resume entries from `Paused` mode.
    Resume {
        /// Path to daemon Unix domain socket.
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
    /// Emergency flatten all open positions and transition engine to `Killed` (requires `--confirm`).
    PanicFlatten {
        /// Explicit confirmation flag required for destructive command.
        #[arg(long)]
        confirm: bool,
        /// Path to daemon Unix domain socket.
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
    /// Manually reset `Killed` mode back to `Paused` after investigation (requires `--confirm`).
    ResetKill {
        /// Explicit confirmation flag required for destructive command.
        #[arg(long)]
        confirm: bool,
        /// Path to daemon Unix domain socket.
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
    },
}

/// Subcommands under `binance-botctl config`.
#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
    /// Load and validate layered configuration files and safety invariants.
    Validate {
        /// Optional explicit TOML configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
    },
}
