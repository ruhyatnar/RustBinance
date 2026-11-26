//! End-to-end offline M19 CLI diagnostics & probes integration suite.

use std::fs;
use std::os::unix::fs::PermissionsExt;

use bot_types::Venue;
use botctl::cli::{Cli, Command, DryRunSubcommand, KeySubcommand, ProbeSubcommand};
use botctl::dry_run_signal::{DryRunSignalOptions, run_dry_run_signal};
use botctl::keygen::generate_ed25519_keypair;
use botctl::run_cli;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[tokio::test]
async fn dry_run_signal_offline_evaluates_s1_and_s2_with_gate_breakdown() {
    let report = run_dry_run_signal(&DryRunSignalOptions {
        venue: Venue::Spot,
        symbol: "BTCUSDT".to_owned(),
        offline: true,
        config: None,
        rest_url_override: None,
    })
    .await
    .unwrap();

    assert!(!report.order_sent);
    assert!(report.bars_loaded >= 600, "bars={}", report.bars_loaded);
    assert_eq!(report.strategies.len(), 2);
    for strat in &report.strategies {
        assert!(
            !strat.gates.is_empty(),
            "empty gates for {}",
            strat.strategy
        );
    }
    let names: Vec<_> = report
        .strategies
        .iter()
        .map(|s| s.strategy.as_str())
        .collect();
    assert!(names.iter().any(|n| n.contains("S1")));
    assert!(names.iter().any(|n| n.contains("S2")));
}

#[tokio::test]
async fn dry_run_signal_fetches_public_klines_from_mock_rest() {
    for (venue, expected_path) in [
        (Venue::Spot, "/api/v3/klines"),
        (Venue::UsdsFutures, "/fapi/v1/klines"),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut paths = Vec::new();
            for _ in 0..2 {
                let (mut sock, _) = listener.accept().await.unwrap();
                let mut buf = [0u8; 4096];
                let n = sock.read(&mut buf).await.unwrap();
                let req = String::from_utf8_lossy(buf.get(..n).unwrap()).to_string();
                let first = req.lines().next().unwrap_or("").to_owned();
                paths.push(first);
                // Return 650 closed synthetic klines
                let mut rows = Vec::with_capacity(650);
                for i in 0_i32..650 {
                    let open_t = 1_700_000_000_000_i64 + i64::from(i) * 300_000;
                    let close_t = open_t + 299_999;
                    let base = 50_000.0 + f64::from(i) * 2.0;
                    rows.push(format!(
                        r#"[{open_t},"{:.2}","{:.2}","{:.2}","{:.2}","100.0",{close_t},"5000000.0",50,"55.0","2750000.0","0"]"#,
                        base,
                        base + 25.0,
                        base - 20.0,
                        base + 5.0,
                    ));
                }
                let body = format!("[{}]", rows.join(","));
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                sock.write_all(resp.as_bytes()).await.unwrap();
            }
            paths
        });

        let report = run_dry_run_signal(&DryRunSignalOptions {
            venue,
            symbol: "BTCUSDT".to_owned(),
            offline: false,
            config: None,
            rest_url_override: Some(format!("http://{addr}")),
        })
        .await
        .unwrap();
        let paths = server.await.unwrap();

        assert!(!report.order_sent);
        assert!(report.bars_loaded >= 600);
        assert!(
            paths.iter().all(|p| p.contains(expected_path)),
            "unexpected paths: {paths:?}"
        );
        let json = serde_json::to_string(&report).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["order_sent"], serde_json::Value::Bool(false));
    }
}

#[tokio::test]
async fn cli_end_to_end_json_output_is_valid_json_for_all_m19_commands() {
    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let pair = generate_ed25519_keypair(dir.path(), "spot").unwrap();
    let api_key_file = dir.path().join("api_key.txt");
    fs::write(&api_key_file, "SUPER_SECRET_TEST_KEY\n").unwrap();
    fs::set_permissions(&api_key_file, fs::Permissions::from_mode(0o600)).unwrap();
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

    let commands = vec![
        Cli {
            command: Command::Key {
                subcommand: KeySubcommand::Fingerprint {
                    file: pair.private_path.clone(),
                    json: true,
                },
            },
        },
        Cli {
            command: Command::Key {
                subcommand: KeySubcommand::Inspect {
                    file: pair.private_path.clone(),
                    json: true,
                },
            },
        },
        Cli {
            command: Command::Doctor {
                config: Some(cfg_path.clone()),
                socket: Some(dir.path().join("missing.sock")),
                offline: true,
                json: true,
            },
        },
        Cli {
            command: Command::Probe {
                subcommand: ProbeSubcommand::DryRunOrder {
                    venue: "spot".to_owned(),
                    symbol: "BTCUSDT".to_owned(),
                    offline: true,
                    config: Some(cfg_path.clone()),
                    json: true,
                },
            },
        },
        Cli {
            command: Command::DryRun {
                subcommand: DryRunSubcommand::Signal {
                    symbol: "BTCUSDT".to_owned(),
                    venue: "spot".to_owned(),
                    offline: true,
                    config: Some(cfg_path),
                    json: true,
                },
            },
        },
    ];

    for cli in commands {
        let out = run_cli(cli).await.unwrap();
        let _: serde_json::Value = serde_json::from_str(&out).expect("valid JSON output");
        assert!(!out.contains("BEGIN PRIVATE KEY"));
        assert!(!out.contains("SUPER_SECRET_TEST_KEY"));
    }
}

#[tokio::test]
async fn review_c1_i2_dry_run_order_spot_omits_time_in_force_and_verifies_exit_payload() {
    use botctl::probes::{DryRunOrderOptions, run_dry_run_order};

    let spot = run_dry_run_order(&DryRunOrderOptions {
        venue: Venue::Spot,
        symbol: "BTCUSDT".to_owned(),
        price: None,
        qty: None,
        offline: true,
        config: None,
        rest_url_override: None,
    })
    .await
    .unwrap();
    assert!(
        !spot.rest_entry_query_preview.contains("timeInForce="),
        "Spot LIMIT_MAKER must not send timeInForce: {}",
        spot.rest_entry_query_preview
    );
    assert!(
        spot.rest_entry_query_preview
            .contains("newOrderRespType=RESULT"),
        "Spot entry must use bot-exchange builder: {}",
        spot.rest_entry_query_preview
    );
    assert!(spot.exit_signature_verified_locally);
    assert_ne!(spot.exit_payload_preview, "");

    let fut = run_dry_run_order(&DryRunOrderOptions {
        venue: Venue::UsdsFutures,
        symbol: "BTCUSDT".to_owned(),
        price: None,
        qty: None,
        offline: true,
        config: None,
        rest_url_override: None,
    })
    .await
    .unwrap();
    assert!(fut.rest_entry_query_preview.contains("timeInForce=GTX"));
    assert!(fut.exit_signature_verified_locally);
    assert!(fut.exit_payload_preview.contains("algoType=CONDITIONAL"));
}

#[tokio::test]
async fn review_c2_i1_doctor_fails_missing_config_and_rejects_public_key_as_private() {
    use botctl::doctor::{CheckStatus, DoctorOptions, run_doctor};

    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

    // Missing explicit --config must FAIL, never silently fall back to BotConfig::default()
    let missing_rep = run_doctor(&DoctorOptions {
        config: Some(dir.path().join("does_not_exist.toml")),
        socket: Some(dir.path().join("missing.sock")),
        spot_rest_url: None,
        futures_rest_url: None,
        offline: true,
    })
    .await;
    assert!(!missing_rep.overall_ok);
    assert!(
        missing_rep
            .checks
            .iter()
            .any(|c| c.name == "config" && c.status == CheckStatus::Fail),
        "{:?}",
        missing_rep.checks
    );

    // Public key configured as private_key_path must FAIL doctor
    let pair = generate_ed25519_keypair(dir.path(), "spot").unwrap();
    let api_key_file = dir.path().join("api_key.txt");
    fs::write(&api_key_file, "dummy-key\n").unwrap();
    let cfg_path = dir.path().join("bot.toml");
    fs::write(
        &cfg_path,
        format!(
            "[general]\nmode = \"paper\"\n[venues.spot]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n[venues.futures]\nenabled = false\n",
            api_key_file.display(),
            pair.public_path.display()
        ),
    )
    .unwrap();

    let pub_rep = run_doctor(&DoctorOptions {
        config: Some(cfg_path),
        socket: Some(dir.path().join("missing.sock")),
        spot_rest_url: None,
        futures_rest_url: None,
        offline: true,
    })
    .await;
    assert!(!pub_rep.overall_ok);
    assert!(
        pub_rep
            .checks
            .iter()
            .any(|c| c.name == "spot_key_permissions" && c.status == CheckStatus::Fail),
        "{:?}",
        pub_rep.checks
    );
}

#[tokio::test]
async fn review_c3_i5_dry_run_signal_uses_configured_signal_tf_and_htf() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let mut paths = Vec::new();
        for _ in 0..2 {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(buf.get(..n).unwrap()).to_string();
            paths.push(req.lines().next().unwrap_or("").to_owned());
            let mut rows = Vec::with_capacity(650);
            for i in 0_i32..650 {
                let open_t = 1_700_000_000_000_i64 + i64::from(i) * 3_600_000;
                let close_t = open_t + 3_599_999;
                let base = 50_000.0 + f64::from(i) * 2.0;
                rows.push(format!(
                    r#"[{open_t},"{:.2}","{:.2}","{:.2}","{:.2}","100.0",{close_t},"5000000.0",50,"55.0","2750000.0","0"]"#,
                    base,
                    base + 25.0,
                    base - 20.0,
                    base + 5.0,
                ));
            }
            let body = format!("[{}]", rows.join(","));
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
        paths
    });

    let _ = run_dry_run_signal(&DryRunSignalOptions {
        venue: Venue::Spot,
        symbol: "BTCUSDT".to_owned(),
        offline: false,
        config: None,
        rest_url_override: Some(format!("http://{addr}")),
    })
    .await
    .unwrap();
    let paths = server.await.unwrap();
    assert!(
        paths.iter().any(|p| p.contains("interval=1h")),
        "expected configured signal_tf 1h, got {paths:?}"
    );
    assert!(
        paths.iter().any(|p| p.contains("interval=4h")),
        "expected configured htf 4h, got {paths:?}"
    );
}

#[tokio::test]
async fn review_i3_ws_logon_returns_structured_report_with_hint_on_auth_error() {
    use botctl::probes::{WsLogonProbeOptions, run_ws_logon_probe};
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::Message;

    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let pair = generate_ed25519_keypair(dir.path(), "fut").unwrap();
    let api_key_file = dir.path().join("fut_key.txt");
    fs::write(&api_key_file, "fut-test-key\n").unwrap();
    let cfg_path = dir.path().join("bot.toml");
    fs::write(
        &cfg_path,
        format!(
            "[general]\nmode = \"testnet\"\n[venues]\ndefault_active = \"futures\"\n[venues.spot]\nenabled = false\n[venues.futures]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n",
            api_key_file.display(),
            pair.private_path.display()
        ),
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        if let Some(Ok(Message::Text(text))) = ws.next().await {
            let req: serde_json::Value = serde_json::from_str(&text).unwrap();
            let id = req.get("id").cloned().unwrap_or(serde_json::Value::Null);
            let resp = serde_json::json!({
                "id": id,
                "status": 400,
                "error": {"code": -1022, "msg": "Signature for this request is not valid."}
            });
            let _ = ws.send(Message::Text(resp.to_string().into())).await;
        }
    });

    let rep = run_ws_logon_probe(&WsLogonProbeOptions {
        venue: Venue::UsdsFutures,
        config: Some(cfg_path),
        ws_url_override: Some(format!("ws://{addr}")),
    })
    .await
    .unwrap();
    server.await.unwrap();

    assert!(!rep.authenticated);
    assert!(
        rep.hint
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase()
            .contains("ed25519"),
        "{rep:?}"
    );
    let _json: serde_json::Value =
        serde_json::from_str(&serde_json::to_string(&rep).unwrap()).unwrap();
}

#[tokio::test]
async fn review_i4_probe_signed_futures_counts_only_non_zero_balances() {
    use botctl::probes::{SignedProbeOptions, run_signed_probe};

    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let pair = generate_ed25519_keypair(dir.path(), "fut").unwrap();
    let api_key_file = dir.path().join("fut_key.txt");
    fs::write(&api_key_file, "fut-test-key\n").unwrap();
    let cfg_path = dir.path().join("bot.toml");
    fs::write(
        &cfg_path,
        format!(
            "[general]\nmode = \"testnet\"\n[binance]\nmax_clock_drift_ms = 5000\n[venues]\ndefault_active = \"futures\"\n[venues.spot]\nenabled = false\n[venues.futures]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n",
            api_key_file.display(),
            pair.private_path.display()
        ),
    )
    .unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let now_ms = bot_exchange::spot::rest::now_unix_ms();
    let server = tokio::spawn(async move {
        for _ in 0..2 {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 4096];
            let n = sock.read(&mut buf).await.unwrap();
            let req = String::from_utf8_lossy(buf.get(..n).unwrap());
            let body = if req.starts_with("GET /fapi/v1/time ") {
                format!(r#"{{"serverTime":{now_ms}}}"#)
            } else {
                r#"{"canTrade":true,"updateTime":100,"assets":[
                    {"asset":"BNB","availableBalance":"0.00000000","initialMargin":"0.00000000"},
                    {"asset":"BTC","availableBalance":"0","initialMargin":"0"},
                    {"asset":"USDT","availableBalance":"250.5","initialMargin":"0"}
                ]}"#
                .to_owned()
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            sock.write_all(resp.as_bytes()).await.unwrap();
        }
    });

    let rep = run_signed_probe(&SignedProbeOptions {
        venue: Venue::UsdsFutures,
        config: Some(cfg_path),
        rest_url_override: Some(format!("http://{addr}")),
    })
    .await
    .unwrap();
    server.await.unwrap();

    assert!(rep.ok);
    assert_eq!(rep.non_zero_balances, 1);
}

#[tokio::test]
async fn minor_doctor_flags_loose_api_key_permissions_and_uses_cfg_uds_path() {
    use botctl::doctor::{CheckStatus, DoctorOptions, run_doctor};

    let dir = tempfile::tempdir().unwrap();
    fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();
    let pair = generate_ed25519_keypair(dir.path(), "spot").unwrap();
    let api_key_file = dir.path().join("api_key.txt");
    fs::write(&api_key_file, "dummy-key\n").unwrap();
    fs::set_permissions(&api_key_file, fs::Permissions::from_mode(0o644)).unwrap();

    let custom_sock = dir.path().join("custom_bot.sock");
    let cfg_path = dir.path().join("bot.toml");
    fs::write(
        &cfg_path,
        format!(
            "[general]\nmode = \"paper\"\n[ipc]\nuds_path = \"{}\"\n[venues.spot]\nenabled = true\napi_key_file = \"{}\"\nprivate_key_path = \"{}\"\n[venues.futures]\nenabled = false\n",
            custom_sock.display(),
            api_key_file.display(),
            pair.private_path.display()
        ),
    )
    .unwrap();

    let rep = run_doctor(&DoctorOptions {
        config: Some(cfg_path),
        socket: None,
        spot_rest_url: None,
        futures_rest_url: None,
        offline: true,
    })
    .await;

    assert!(!rep.overall_ok);
    assert!(
        rep.checks
            .iter()
            .any(|c| c.name == "spot_api_key_file" && c.status == CheckStatus::Fail),
        "loose 0644 api_key_file must FAIL doctor: {:?}",
        rep.checks
    );
    let uds = rep
        .checks
        .iter()
        .find(|c| c.name == "daemon_uds")
        .expect("daemon_uds check");
    assert!(
        uds.detail.contains("custom_bot.sock"),
        "doctor should fall back to cfg.ipc.uds_path: {}",
        uds.detail
    );
}
