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
    /// Local key audit commands (fingerprint, permission inspection).
    Key {
        /// Key action.
        #[command(subcommand)]
        subcommand: KeySubcommand,
    },
    /// Run pre-live diagnostic matrix (config, keys, confirmation, REST/clock, daemon UDS).
    Doctor {
        /// Optional explicit TOML configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Path to daemon Unix domain socket.
        #[arg(long, default_value = DEFAULT_SOCKET_PATH)]
        socket: PathBuf,
        /// Skip remote REST `/ping` and `/time` checks.
        #[arg(long)]
        offline: bool,
        /// Output full diagnostic report as JSON.
        #[arg(long)]
        json: bool,
    },
    /// Non-destructive pre-live exchange probes (zero orders ever transmitted).
    Probe {
        /// Probe action.
        #[command(subcommand)]
        subcommand: ProbeSubcommand,
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
    /// Atomically switch active trading venue (`spot` or `futures`, §3.5).
    SwitchVenue {
        /// Target venue (`spot` or `futures`).
        #[arg(long)]
        target: String,
        /// Policy for open positions on the old venue (`drain`, `refuse`, or `flatten`).
        #[arg(long, default_value = "drain")]
        on_open_positions: String,
        /// Required when `--on-open-positions flatten` is specified.
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

/// Subcommands under `binance-botctl key` (local only; never prints private key material).
#[derive(Debug, Subcommand)]
pub enum KeySubcommand {
    /// Print the `SHA256:<hex>` fingerprint of an Ed25519 public or private PEM.
    Fingerprint {
        /// Path to a public (`.pub.pem`) or private (`.pem`) key file.
        #[arg(long)]
        file: PathBuf,
        /// Output JSON.
        #[arg(long)]
        json: bool,
    },
    /// Inspect PEM validity, fingerprint, and Unix permissions (`0600` key, `0700` dir).
    Inspect {
        /// Path to a public or private key file.
        #[arg(long)]
        file: PathBuf,
        /// Output JSON.
        #[arg(long)]
        json: bool,
    },
}

/// Subcommands under `binance-botctl probe`.
#[derive(Debug, Subcommand)]
pub enum ProbeSubcommand {
    /// Verify Ed25519 signing, clock offset, and read-only signed account access (no orders).
    Signed {
        /// Target venue (`spot` or `futures`).
        #[arg(long, default_value = "spot")]
        venue: String,
        /// Optional explicit TOML configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Output JSON.
        #[arg(long)]
        json: bool,
    },
    /// Quantize and sign a synthetic entry/exit order locally (never transmits over network).
    DryRunOrder {
        /// Target venue (`spot` or `futures`).
        #[arg(long, default_value = "spot")]
        venue: String,
        /// Symbol to dry-run (e.g. `BTCUSDT`).
        #[arg(long, default_value = "BTCUSDT")]
        symbol: String,
        /// Use built-in offline symbol filter fixture (no network calls).
        #[arg(long)]
        offline: bool,
        /// Optional explicit TOML configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Output JSON.
        #[arg(long)]
        json: bool,
    },
    /// Probe WS-API authentication (Spot `session.logon` / Futures signed `v2/account.balance`) and close cleanly.
    WsLogon {
        /// Target venue (`spot` or `futures`).
        #[arg(long, default_value = "spot")]
        venue: String,
        /// Optional explicit TOML configuration file path.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Output JSON.
        #[arg(long)]
        json: bool,
    },
}
