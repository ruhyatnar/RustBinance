#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]
//! Integration tests for `binance-botctl` (`keygen`, `config validate`, `status`, `pause`, `resume`, `panic-flatten`, `reset-kill`).

use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;

use bot_config::BotConfig;
use bot_ipc::{
    CommandDispatcher, ConfirmNonceStore, InMemoryIpcBackend, IpcBackend, PeerAuthPolicy,
    TelemetryHub, UdsServerConfig, serve_uds,
};
use bot_types::EngineMode;
use botctl::cli::{Cli, Command, ConfigSubcommand};
use botctl::run_cli;
use clap::Parser;
use tempfile::TempDir;

#[test]
fn keygen_creates_0600_private_key_and_refuses_overwrite() {
    let dir = TempDir::new().expect("tempdir");
    let out_str = dir.path().to_str().expect("utf8 path");

    let cli = Cli::try_parse_from([
        "binance-botctl",
        "keygen",
        "--out",
        out_str,
        "--name",
        "binance_spot_ed25519",
    ])
    .expect("parse keygen");

    let rt = tokio::runtime::Runtime::new().expect("rt");
    let output = rt.block_on(run_cli(cli)).expect("keygen should succeed");
    assert!(output.contains("binance_spot_ed25519.pem"));

    let priv_path = dir.path().join("binance_spot_ed25519.pem");
    let pub_path = dir.path().join("binance_spot_ed25519.pub.pem");
    assert!(priv_path.exists());
    assert!(pub_path.exists());

    let priv_mode = std::fs::metadata(&priv_path)
        .expect("priv meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(priv_mode, 0o600, "private key must have 0600 permissions");

    let pub_mode = std::fs::metadata(&pub_path)
        .expect("pub meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(pub_mode, 0o644, "public key must have 0644 permissions");

    let priv_pem = std::fs::read_to_string(&priv_path).expect("read priv");
    let pub_pem = std::fs::read_to_string(&pub_path).expect("read pub");
    assert!(priv_pem.contains("BEGIN PRIVATE KEY"));
    assert!(pub_pem.contains("BEGIN PUBLIC KEY"));

    // Second invocation MUST fail and NOT overwrite the original key
    let cli_again = Cli::try_parse_from([
        "binance-botctl",
        "keygen",
        "--out",
        out_str,
        "--name",
        "binance_spot_ed25519",
    ])
    .expect("parse keygen again");
    let err = rt.block_on(run_cli(cli_again));
    assert!(err.is_err(), "keygen must refuse to overwrite existing key");

    let priv_pem_after = std::fs::read_to_string(&priv_path).expect("read priv after");
    assert_eq!(
        priv_pem, priv_pem_after,
        "existing private key must remain untouched"
    );
}

#[test]
fn config_validate_accepts_valid_and_rejects_invalid_toml() {
    let dir = TempDir::new().expect("tempdir");
    let valid_path = dir.path().join("valid.toml");
    std::fs::write(
        &valid_path,
        "[general]\nmode = \"paper\"\n[risk]\nrisk_per_trade_pct = \"0.5\"\n",
    )
    .expect("write valid toml");

    let invalid_path = dir.path().join("invalid.toml");
    std::fs::write(
        &invalid_path,
        "[general]\nmode = \"paper\"\n[risk]\nrisk_per_trade_pct = \"50.0\"\n",
    )
    .expect("write invalid toml");

    let rt = tokio::runtime::Runtime::new().expect("rt");

    let ok_cli = Cli {
        command: Command::Config {
            subcommand: ConfigSubcommand::Validate {
                config: Some(valid_path),
            },
        },
    };
    let ok_out = rt.block_on(run_cli(ok_cli)).expect("valid config passes");
    assert!(ok_out.contains("valid"));

    let bad_cli = Cli {
        command: Command::Config {
            subcommand: ConfigSubcommand::Validate {
                config: Some(invalid_path),
            },
        },
    };
    let bad_res = rt.block_on(run_cli(bad_cli));
    assert!(bad_res.is_err(), "out-of-range risk config must fail");
}

#[tokio::test]
async fn botctl_uds_control_flow_status_pause_resume_panic_and_reset_kill() {
    let dir = TempDir::new().expect("tempdir");
    let sock_path = dir.path().join("bot.sock");
    let sock_str = sock_path.to_str().expect("utf8");

    let backend = Arc::new(InMemoryIpcBackend::new(BotConfig::default()));
    let dispatcher = Arc::new(CommandDispatcher::new(
        backend.clone(),
        None,
        ConfirmNonceStore::new(Duration::from_secs(10)),
    ));
    let hub = Arc::new(TelemetryHub::new(16));

    let server = serve_uds(
        UdsServerConfig {
            socket_path: sock_path.clone(),
            peer_auth: PeerAuthPolicy::allow_current_process(),
        },
        dispatcher,
        hub,
    )
    .await
    .expect("start uds");

    // 1. status --json
    let status_out = run_cli(
        Cli::try_parse_from(["binance-botctl", "status", "--socket", sock_str, "--json"])
            .expect("parse status"),
    )
    .await
    .expect("status");
    let snap_val: serde_json::Value = serde_json::from_str(&status_out).expect("status json");
    assert_eq!(snap_val["engine"]["mode"], "Ready");

    // 2. pause -> Paused
    run_cli(Cli::try_parse_from(["binance-botctl", "pause", "--socket", sock_str]).expect("parse"))
        .await
        .expect("pause");
    assert_eq!(backend.engine_mode().await, EngineMode::Paused);

    // 3. resume -> Ready
    run_cli(
        Cli::try_parse_from(["binance-botctl", "resume", "--socket", sock_str]).expect("parse"),
    )
    .await
    .expect("resume");
    assert_eq!(backend.engine_mode().await, EngineMode::Ready);

    // 4. panic-flatten WITHOUT --confirm must fail
    let panic_no_confirm = run_cli(
        Cli::try_parse_from(["binance-botctl", "panic-flatten", "--socket", sock_str])
            .expect("parse"),
    )
    .await;
    assert!(
        panic_no_confirm.is_err(),
        "panic-flatten without --confirm must be refused"
    );
    assert_eq!(backend.engine_mode().await, EngineMode::Ready);

    // 5. panic-flatten --confirm -> Killed
    run_cli(
        Cli::try_parse_from([
            "binance-botctl",
            "panic-flatten",
            "--confirm",
            "--socket",
            sock_str,
        ])
        .expect("parse"),
    )
    .await
    .expect("panic-flatten --confirm");
    assert_eq!(backend.engine_mode().await, EngineMode::Killed);

    // 6. reset-kill WITHOUT --confirm must fail
    let reset_no_confirm = run_cli(
        Cli::try_parse_from(["binance-botctl", "reset-kill", "--socket", sock_str]).expect("parse"),
    )
    .await;
    assert!(
        reset_no_confirm.is_err(),
        "reset-kill without --confirm must be refused"
    );
    assert_eq!(backend.engine_mode().await, EngineMode::Killed);

    // 7. reset-kill --confirm -> Paused
    run_cli(
        Cli::try_parse_from([
            "binance-botctl",
            "reset-kill",
            "--confirm",
            "--socket",
            sock_str,
        ])
        .expect("parse"),
    )
    .await
    .expect("reset-kill --confirm");
    assert_eq!(backend.engine_mode().await, EngineMode::Paused);

    server.abort();
}
