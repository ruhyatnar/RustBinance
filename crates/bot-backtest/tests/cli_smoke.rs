#![allow(clippy::expect_used)]

use std::fmt::Write as _;
use std::fs;
use std::process::Command;
use std::str::FromStr;

use bot_store::{FillRecord, JournalEnvelope, JournalEvent, SqliteStore};
use bot_types::{Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;

fn bin_path() -> &'static str {
    env!("CARGO_BIN_EXE_bot-backtest")
}

#[test]
fn cli_rejects_unknown_subcommand_and_invalid_gate() {
    let out = Command::new(bin_path())
        .arg("unknown-cmd")
        .output()
        .expect("run bin");
    assert!(!out.status.success());

    let out_gate = Command::new(bin_path())
        .args([
            "gate-report",
            "--db",
            "/tmp/nonexistent.sqlite",
            "--venue",
            "spot",
            "--gate",
            "G9",
            "--out",
            "/tmp/out.md",
        ])
        .output()
        .expect("run bin");
    assert!(!out_gate.status.success());
}

#[test]
fn cli_gate_report_generates_markdown_from_temp_sqlite_db() {
    let dir = std::env::temp_dir().join(format!("bot-backtest-cli-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let db_path = dir.join("journal.sqlite");
    let out_path = dir.join("G2.md");

    let mut store = SqliteStore::open(&db_path).expect("open sqlite");
    store
        .append_event(
            1_700_000_000_000,
            &JournalEnvelope::new(JournalEvent::FillRecorded {
                fill: FillRecord {
                    venue: Venue::Spot,
                    trade_id: 1,
                    symbol: Symbol("BTCUSDT".into()),
                    client_id: "spot-1".into(),
                    price: Price(Decimal::from_str("50000").expect("dec")),
                    qty: Qty(Decimal::from_str("0.01").expect("dec")),
                    fee: Decimal::from_str("0.5").expect("dec"),
                    fee_asset: "USDT".into(),
                    is_maker: true,
                    ts_ms: 1_700_000_000_000,
                },
            }),
        )
        .expect("append");
    drop(store);

    let out = Command::new(bin_path())
        .args([
            "gate-report",
            "--db",
            db_path.to_str().expect("utf8"),
            "--venue",
            "spot",
            "--gate",
            "G2",
            "--out",
            out_path.to_str().expect("utf8"),
        ])
        .output()
        .expect("run bin");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md = fs::read_to_string(&out_path).expect("read md");
    assert!(md.contains("Gate G2"));
    assert!(md.contains("Spot"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn cli_g1_generates_markdown_from_temp_kline_csv() {
    let dir = std::env::temp_dir().join(format!("bot-backtest-g1-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let csv_path = dir.join("btcusdt_1h.csv");
    let out_path = dir.join("G1.md");

    // 40 synthetic 1h Binance kline rows (< 3 years, so G1 must cleanly render FAIL)
    let mut csv = String::new();
    let base_ts: i64 = 1_700_000_000_000;
    for i in 0..40_i64 {
        let open_ts = base_ts + i * 3_600_000;
        let close_ts = open_ts + 3_599_999;
        let p = 50000 + i * 10;
        let _ = writeln!(
            csv,
            "{open_ts},{p}.00,{}.00,{}.00,{}.00,10.0,{close_ts},500000.0,100,5.0,250000.0,0",
            p + 50,
            p - 50,
            p + 20
        );
    }
    fs::write(&csv_path, csv).expect("write csv");

    let out = Command::new(bin_path())
        .args([
            "g1",
            "--input",
            csv_path.to_str().expect("utf8"),
            "--symbol",
            "BTCUSDT",
            "--out",
            out_path.to_str().expect("utf8"),
        ])
        .output()
        .expect("run bin");

    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let md = fs::read_to_string(&out_path).expect("read md");
    assert!(md.contains("Gate G1"));
    assert!(md.contains("`FAIL`"));

    let _ = fs::remove_dir_all(&dir);
}
