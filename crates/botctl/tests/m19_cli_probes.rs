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
                socket: dir.path().join("missing.sock"),
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
