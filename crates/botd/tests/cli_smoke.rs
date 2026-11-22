use std::process::Command;

#[test]
fn botd_emits_valid_json_log_in_paper_mode_and_exits_cleanly() {
    let bin = env!("CARGO_BIN_EXE_binance-botd");
    let output = Command::new(bin)
        .env("RUST_LOG", "info")
        .output()
        .expect("failed to execute binance-botd");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let combined = if output.stdout.is_empty() {
        String::from_utf8_lossy(&output.stderr).into_owned()
    } else {
        String::from_utf8_lossy(&output.stdout).into_owned()
    };
    let first_line = combined
        .lines()
        .next()
        .expect("expected at least one JSON log line");
    let parsed: serde_json::Value =
        serde_json::from_str(first_line).expect("log line must be valid JSON");
    assert_eq!(parsed["level"], "INFO");
    assert_eq!(parsed["fields"]["mode"], "paper");
}

#[test]
fn botd_respects_rust_log_filter() {
    let bin = env!("CARGO_BIN_EXE_binance-botd");
    let output = Command::new(bin)
        .env("RUST_LOG", "error")
        .output()
        .expect("failed to execute binance-botd");

    assert!(output.status.success());
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "");
    assert_eq!(String::from_utf8_lossy(&output.stderr).trim(), "");
}
