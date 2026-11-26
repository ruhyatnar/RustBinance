//! Pre-live diagnostic matrix (`binance-botctl doctor`, M19).
//!
//! Runs independent checks in sequence without short-circuiting so operators see
//! every configuration, key-permission, confirmation-file, connectivity, and daemon
//! issue in one invocation. Never prints private key or API key contents.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use bot_config::{BotConfig, ConfigSourcePaths, load_config, load_from_paths};
use bot_exchange::{FuturesRestClient, RateLimitTracker, SpotRestClient};
use bot_types::TradingMode;
use serde::Serialize;

use crate::client::UdsClient;
use crate::key_audit::{KeyFileKind, inspect_key_file};

const REST_TIMEOUT: Duration = Duration::from_secs(5);

/// Outcome of a single diagnostic check.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    /// Check passed.
    Pass,
    /// Non-blocking warning (does not fail `overall_ok`).
    Warn,
    /// Blocking failure (sets `overall_ok = false` and non-zero exit).
    Fail,
}

/// One row in [`DoctorReport`].
#[derive(Debug, Clone, Serialize)]
pub struct DoctorCheck {
    /// Machine-readable check identifier.
    pub name: String,
    /// Pass, Warn, or Fail.
    pub status: CheckStatus,
    /// Human-readable summary (never contains secrets).
    pub detail: String,
    /// Optional actionable remediation hint.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hint: Option<String>,
}

/// Full diagnostic matrix output.
#[derive(Debug, Clone, Serialize)]
pub struct DoctorReport {
    /// `binance-botctl` package version.
    pub binary_version: String,
    /// `true` iff no check has status [`CheckStatus::Fail`].
    pub overall_ok: bool,
    /// Ordered list of diagnostic checks.
    pub checks: Vec<DoctorCheck>,
}

impl DoctorReport {
    /// Render as human-readable text.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "binance-botctl doctor v{} [{}]",
            self.binary_version,
            if self.overall_ok { "PASS" } else { "FAIL" }
        );
        for c in &self.checks {
            let tag = match c.status {
                CheckStatus::Pass => "PASS",
                CheckStatus::Warn => "WARN",
                CheckStatus::Fail => "FAIL",
            };
            let _ = write!(out, "\n  [{tag}] {}: {}", c.name, c.detail);
            if let Some(hint) = &c.hint {
                let _ = write!(out, "\n         hint: {hint}");
            }
        }
        out
    }
}

/// Input options for [`run_doctor`].
#[derive(Debug, Clone)]
pub struct DoctorOptions {
    /// Optional explicit TOML configuration file path.
    pub config: Option<PathBuf>,
    /// Daemon Unix domain socket path.
    pub socket: PathBuf,
    /// Optional Spot REST base URL override (primarily for tests/custom endpoints).
    pub spot_rest_url: Option<String>,
    /// Optional Futures REST base URL override (primarily for tests/custom endpoints).
    pub futures_rest_url: Option<String>,
    /// Skip remote REST `/ping` and `/time` checks.
    pub offline: bool,
}

/// Execute the full `binance-botctl doctor` diagnostic matrix.
pub async fn run_doctor(opts: &DoctorOptions) -> DoctorReport {
    let mut checks = Vec::new();
    let binary_version = env!("CARGO_PKG_VERSION").to_owned();
    checks.push(DoctorCheck {
        name: "binary_version".to_owned(),
        status: CheckStatus::Pass,
        detail: format!("binance-botctl {binary_version}"),
        hint: None,
    });

    let loaded_cfg = load_doctor_config(opts.config.as_deref());
    let cfg = match loaded_cfg {
        Ok(cfg) => {
            checks.push(DoctorCheck {
                name: "config".to_owned(),
                status: CheckStatus::Pass,
                detail: format!(
                    "valid (mode={:?}, spot={}, futures={})",
                    cfg.general.mode, cfg.venues.spot.enabled, cfg.venues.futures.enabled
                ),
                hint: None,
            });
            Some(cfg)
        }
        Err(err) => {
            checks.push(DoctorCheck {
                name: "config".to_owned(),
                status: CheckStatus::Fail,
                detail: format!("failed to load/validate config: {err}"),
                hint: Some("run `binance-botctl config validate --config <path>`".to_owned()),
            });
            None
        }
    };

    if let Some(cfg) = &cfg {
        check_venue_keys_and_live_confirm(cfg, &mut checks);
        if opts.offline {
            checks.push(DoctorCheck {
                name: "rest_connectivity".to_owned(),
                status: CheckStatus::Warn,
                detail: "skipped (--offline)".to_owned(),
                hint: None,
            });
        } else {
            check_rest_and_clock(cfg, opts, &mut checks).await;
        }
    }

    let mode = cfg.as_ref().map_or(TradingMode::Paper, |c| c.general.mode);
    check_daemon_uds(&opts.socket, mode, &mut checks).await;

    let overall_ok = checks.iter().all(|c| c.status != CheckStatus::Fail);
    DoctorReport {
        binary_version,
        overall_ok,
        checks,
    }
}

fn load_doctor_config(config_path: Option<&Path>) -> anyhow::Result<BotConfig> {
    if let Some(path) = config_path {
        if !path.is_file() {
            anyhow::bail!("config file not found: {}", path.display());
        }
        let conf_d_dir = path
            .parent()
            .map(|p| p.join("conf.d"))
            .filter(|d| d.is_dir());
        Ok(load_from_paths(&ConfigSourcePaths {
            base_file: Some(path.to_path_buf()),
            conf_d_dir,
            env_prefix: Some("BOT__".to_owned()),
        })?)
    } else {
        Ok(load_config()?)
    }
}

fn check_venue_keys_and_live_confirm(cfg: &BotConfig, checks: &mut Vec<DoctorCheck>) {
    if cfg.venues.spot.enabled {
        check_single_venue_files(
            "spot",
            &cfg.venues.spot.api_key_file,
            &cfg.venues.spot.private_key_path,
            &cfg.venues.spot.live_confirmation_file,
            cfg.general.mode,
            checks,
        );
    }
    if cfg.venues.futures.enabled {
        check_single_venue_files(
            "futures",
            &cfg.venues.futures.api_key_file,
            &cfg.venues.futures.private_key_path,
            &cfg.venues.futures.live_confirmation_file,
            cfg.general.mode,
            checks,
        );
    }
}

fn check_single_venue_files(
    label: &str,
    api_key_file: &str,
    private_key_path: &str,
    live_confirmation_file: &str,
    mode: TradingMode,
    checks: &mut Vec<DoctorCheck>,
) {
    let api_path = Path::new(api_key_file);
    match std::fs::read_to_string(api_path) {
        Ok(contents) if !contents.trim().is_empty() => {
            checks.push(DoctorCheck {
                name: format!("{label}_api_key_file"),
                status: CheckStatus::Pass,
                detail: format!("present and non-empty ({api_key_file})"),
                hint: None,
            });
        }
        Ok(_) => {
            checks.push(DoctorCheck {
                name: format!("{label}_api_key_file"),
                status: CheckStatus::Fail,
                detail: format!("API key file is empty ({api_key_file})"),
                hint: Some("write the venue API key into this file (mode 0600)".to_owned()),
            });
        }
        Err(err) => {
            let status = if mode == TradingMode::Paper {
                CheckStatus::Warn
            } else {
                CheckStatus::Fail
            };
            checks.push(DoctorCheck {
                name: format!("{label}_api_key_file"),
                status,
                detail: format!("cannot read {api_key_file}: {}", err.kind()),
                hint: Some("provision API key file before testnet/live runs".to_owned()),
            });
        }
    }

    let priv_path = Path::new(private_key_path);
    match inspect_key_file(priv_path) {
        Ok(rep) if rep.kind != KeyFileKind::PrivatePkcs8 => {
            checks.push(DoctorCheck {
                name: format!("{label}_key_permissions"),
                status: CheckStatus::Fail,
                detail: format!(
                    "expected PKCS#8 private key PEM at {private_key_path}, found {:?}",
                    rep.kind
                ),
                hint: Some(
                    "point private_key_path to the .pem private key (not the .pub.pem public key)"
                        .to_owned(),
                ),
            });
        }
        Ok(rep) if rep.is_ok() => {
            checks.push(DoctorCheck {
                name: format!("{label}_key_permissions"),
                status: CheckStatus::Pass,
                detail: format!(
                    "valid Ed25519 key ({}, file={}, dir={})",
                    rep.sha256_fingerprint, rep.file_mode_octal, rep.dir_mode_octal
                ),
                hint: None,
            });
        }
        Ok(rep) => {
            checks.push(DoctorCheck {
                name: format!("{label}_key_permissions"),
                status: CheckStatus::Fail,
                detail: rep.issues.join("; "),
                hint: Some(format!(
                    "run `binance-botctl key inspect --file {private_key_path}`"
                )),
            });
        }
        Err(err) => {
            let status = if mode == TradingMode::Paper {
                CheckStatus::Warn
            } else {
                CheckStatus::Fail
            };
            checks.push(DoctorCheck {
                name: format!("{label}_key_permissions"),
                status,
                detail: format!("cannot inspect {private_key_path}: {err}"),
                hint: Some("generate keypair with `binance-botctl keygen`".to_owned()),
            });
        }
    }

    check_live_confirmation(label, live_confirmation_file, mode, checks);
}

fn check_live_confirmation(
    label: &str,
    live_confirmation_file: &str,
    mode: TradingMode,
    checks: &mut Vec<DoctorCheck>,
) {
    if mode == TradingMode::Live {
        let confirm_ok = !live_confirmation_file.trim().is_empty()
            && std::fs::read_to_string(live_confirmation_file).is_ok_and(|s| !s.trim().is_empty());
        checks.push(DoctorCheck {
            name: format!("{label}_live_confirmation"),
            status: if confirm_ok {
                CheckStatus::Pass
            } else {
                CheckStatus::Fail
            },
            detail: if confirm_ok {
                format!("present and non-empty ({live_confirmation_file})")
            } else {
                format!("missing or empty live_confirmation_file ({live_confirmation_file})")
            },
            hint: (!confirm_ok)
                .then(|| "complete G2-G5 gates before creating live_confirmation_file".to_owned()),
        });
    } else {
        checks.push(DoctorCheck {
            name: format!("{label}_live_confirmation"),
            status: CheckStatus::Pass,
            detail: "not required in paper/testnet".to_owned(),
            hint: None,
        });
    }
}

async fn check_rest_and_clock(
    cfg: &BotConfig,
    opts: &DoctorOptions,
    checks: &mut Vec<DoctorCheck>,
) {
    let max_drift = i64::try_from(cfg.binance.max_clock_drift_ms).unwrap_or(i64::MAX);
    if cfg.venues.spot.enabled {
        let base_url = opts.spot_rest_url.clone().unwrap_or_else(|| {
            if cfg.general.mode == TradingMode::Live {
                SpotRestClient::PRODUCTION_URL.to_owned()
            } else {
                SpotRestClient::TESTNET_URL.to_owned()
            }
        });
        check_spot_rest(&base_url, max_drift, checks).await;
    }
    if cfg.venues.futures.enabled {
        let base_url = opts.futures_rest_url.clone().unwrap_or_else(|| {
            if cfg.general.mode == TradingMode::Live {
                FuturesRestClient::PRODUCTION_URL.to_owned()
            } else {
                FuturesRestClient::TESTNET_URL.to_owned()
            }
        });
        check_futures_rest(&base_url, max_drift, checks).await;
    }
}

async fn check_spot_rest(base_url: &str, max_drift: i64, checks: &mut Vec<DoctorCheck>) {
    match SpotRestClient::new(base_url, REST_TIMEOUT) {
        Ok(client) => {
            match client.ping().await {
                Ok(()) => checks.push(DoctorCheck {
                    name: "spot_rest_ping".to_owned(),
                    status: CheckStatus::Pass,
                    detail: format!("reachable ({base_url}/api/v3/ping)"),
                    hint: None,
                }),
                Err(err) => checks.push(DoctorCheck {
                    name: "spot_rest_ping".to_owned(),
                    status: CheckStatus::Fail,
                    detail: format!("ping failed: {err}"),
                    hint: Some("check network/DNS/TLS connectivity to Binance Spot".to_owned()),
                }),
            }
            match client.server_time().await {
                Ok(sync) => {
                    record_clock_check("spot_clock_offset", sync.offset_ms, max_drift, checks);
                }
                Err(err) => checks.push(DoctorCheck {
                    name: "spot_clock_offset".to_owned(),
                    status: CheckStatus::Fail,
                    detail: format!("/api/v3/time failed: {err}"),
                    hint: Some("verify Spot REST connectivity".to_owned()),
                }),
            }
        }
        Err(err) => checks.push(DoctorCheck {
            name: "spot_rest_ping".to_owned(),
            status: CheckStatus::Fail,
            detail: format!("HTTP client init failed: {err}"),
            hint: None,
        }),
    }
}

async fn check_futures_rest(base_url: &str, max_drift: i64, checks: &mut Vec<DoctorCheck>) {
    match FuturesRestClient::new(
        base_url,
        REST_TIMEOUT,
        None,
        Arc::new(RateLimitTracker::new()),
    ) {
        Ok(client) => {
            match client.ping().await {
                Ok(()) => checks.push(DoctorCheck {
                    name: "futures_rest_ping".to_owned(),
                    status: CheckStatus::Pass,
                    detail: format!("reachable ({base_url}/fapi/v1/ping)"),
                    hint: None,
                }),
                Err(err) => checks.push(DoctorCheck {
                    name: "futures_rest_ping".to_owned(),
                    status: CheckStatus::Fail,
                    detail: format!("ping failed: {err}"),
                    hint: Some("check network/DNS/TLS connectivity to Binance Futures".to_owned()),
                }),
            }
            match client.server_time().await {
                Ok(sync) => {
                    record_clock_check("futures_clock_offset", sync.offset_ms, max_drift, checks);
                }
                Err(err) => checks.push(DoctorCheck {
                    name: "futures_clock_offset".to_owned(),
                    status: CheckStatus::Fail,
                    detail: format!("/fapi/v1/time failed: {err}"),
                    hint: Some("verify Futures REST connectivity".to_owned()),
                }),
            }
        }
        Err(err) => checks.push(DoctorCheck {
            name: "futures_rest_ping".to_owned(),
            status: CheckStatus::Fail,
            detail: format!("HTTP client init failed: {err}"),
            hint: None,
        }),
    }
}

fn record_clock_check(name: &str, offset_ms: i64, max_drift: i64, checks: &mut Vec<DoctorCheck>) {
    let within = offset_ms.saturating_abs() <= max_drift;
    checks.push(DoctorCheck {
        name: name.to_owned(),
        status: if within {
            CheckStatus::Pass
        } else {
            CheckStatus::Fail
        },
        detail: format!("offset={offset_ms}ms (limit=±{max_drift}ms)"),
        hint: (!within).then(|| {
            "synchronize host clock via NTP/chronyd (`chronyc tracking`) to avoid -1021 errors"
                .to_owned()
        }),
    });
}

async fn check_daemon_uds(socket: &Path, mode: TradingMode, checks: &mut Vec<DoctorCheck>) {
    match UdsClient::connect(socket).await {
        Ok(client) => {
            let snap = client.snapshot();
            checks.push(DoctorCheck {
                name: "daemon_uds".to_owned(),
                status: CheckStatus::Pass,
                detail: format!(
                    "connected ({}) — engine={:?}, active_venue={}",
                    socket.display(),
                    snap.engine.mode,
                    snap.venues.active
                ),
                hint: None,
            });
        }
        Err(err) => {
            let exists = socket.exists();
            let status = if exists || mode == TradingMode::Live {
                CheckStatus::Fail
            } else {
                CheckStatus::Warn
            };
            checks.push(DoctorCheck {
                name: "daemon_uds".to_owned(),
                status,
                detail: format!("cannot connect to {}: {err}", socket.display()),
                hint: Some(if exists {
                    "check socket permissions (`0660`, group `binancebot`) or stale socket file"
                        .to_owned()
                } else {
                    "daemon is not running (start `binance-botd` when ready)".to_owned()
                }),
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    use super::{CheckStatus, DoctorOptions, run_doctor};
    use crate::keygen::generate_ed25519_keypair;

    fn make_secure_key_dir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
        dir
    }

    #[tokio::test]
    async fn doctor_passes_with_valid_paper_config_and_0600_keys_offline() {
        let dir = make_secure_key_dir();
        let pair = generate_ed25519_keypair(dir.path(), "spot").unwrap();
        let api_key_file = dir.path().join("api_key.txt");
        fs::write(&api_key_file, "dummy-testnet-api-key\n").unwrap();

        let cfg_path = dir.path().join("bot.toml");
        fs::write(
            &cfg_path,
            format!(
                "[general]\nmode = \"paper\"\n[venues.spot]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n[venues.futures]\nenabled = false\n",
                api_key_file.display(),
                pair.private_path.display()
            ),
        )
        .unwrap();

        let report = run_doctor(&DoctorOptions {
            config: Some(cfg_path),
            socket: dir.path().join("missing.sock"),
            spot_rest_url: None,
            futures_rest_url: None,
            offline: true,
        })
        .await;

        assert!(
            report.overall_ok,
            "expected overall_ok: {:?}",
            report.checks
        );
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "config" && c.status == CheckStatus::Pass)
        );
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "spot_key_permissions" && c.status == CheckStatus::Pass)
        );
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "daemon_uds" && c.status == CheckStatus::Warn)
        );
    }

    #[tokio::test]
    async fn doctor_fails_when_live_mode_missing_confirmation_file_and_runs_other_checks() {
        let dir = make_secure_key_dir();
        let pair = generate_ed25519_keypair(dir.path(), "spot").unwrap();
        fs::set_permissions(&pair.private_path, fs::Permissions::from_mode(0o644)).unwrap();
        let api_key_file = dir.path().join("api_key.txt");
        fs::write(&api_key_file, "dummy-key\n").unwrap();

        let cfg_path = dir.path().join("bot.toml");
        fs::write(
            &cfg_path,
            format!(
                "[general]\nmode = \"live\"\n[venues.spot]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\nlive_confirmation_file = \"{}\"\n[venues.futures]\nenabled = false\n",
                api_key_file.display(),
                pair.private_path.display(),
                dir.path().join("nonexistent_confirm.txt").display()
            ),
        )
        .unwrap();

        let report = run_doctor(&DoctorOptions {
            config: Some(cfg_path),
            socket: dir.path().join("missing.sock"),
            spot_rest_url: None,
            futures_rest_url: None,
            offline: true,
        })
        .await;

        assert!(!report.overall_ok);
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "spot_live_confirmation" && c.status == CheckStatus::Fail),
            "{:?}",
            report.checks
        );
        assert!(
            report
                .checks
                .iter()
                .any(|c| c.name == "spot_key_permissions" && c.status == CheckStatus::Fail),
            "{:?}",
            report.checks
        );
    }

    #[tokio::test]
    async fn doctor_checks_rest_ping_and_clock_offset_against_mock_server() {
        let dir = make_secure_key_dir();
        let pair = generate_ed25519_keypair(dir.path(), "spot").unwrap();
        let api_key_file = dir.path().join("api_key.txt");
        fs::write(&api_key_file, "dummy-key\n").unwrap();

        let cfg_path = dir.path().join("bot.toml");
        fs::write(
            &cfg_path,
            format!(
                "[general]\nmode = \"paper\"\n[binance]\nmax_clock_drift_ms = 500\n[venues.spot]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n[venues.futures]\nenabled = false\n",
                api_key_file.display(),
                pair.private_path.display()
            ),
        )
        .unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 1024];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(buf.get(..n).unwrap());
                // Deliberately return a drifted serverTime far in the past.
                let body = if req.starts_with("GET /api/v3/ping ") {
                    "{}"
                } else {
                    r#"{"serverTime":1000000000000}"#
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
        });

        let report = run_doctor(&DoctorOptions {
            config: Some(cfg_path),
            socket: dir.path().join("missing.sock"),
            spot_rest_url: Some(format!("http://{addr}")),
            futures_rest_url: None,
            offline: false,
        })
        .await;
        server.await.unwrap();

        assert!(!report.overall_ok);
        let clock_check = report
            .checks
            .iter()
            .find(|c| c.name == "spot_clock_offset")
            .unwrap();
        assert_eq!(clock_check.status, CheckStatus::Fail);
        assert!(
            clock_check
                .hint
                .as_deref()
                .unwrap_or("")
                .to_ascii_lowercase()
                .contains("ntp")
                || clock_check
                    .hint
                    .as_deref()
                    .unwrap_or("")
                    .to_ascii_lowercase()
                    .contains("chrony")
        );
    }
}
