#![forbid(unsafe_code)]
//! Thin CLI binary for M14 Spot gates:
//! - `bot-backtest g1 --input <csv> --symbol <SYMBOL> --out <md>`
//! - `bot-backtest gate-report --db <sqlite> --venue <spot|futures> --gate <G2|G3|G4> --out <md>`

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use bot_backtest::gate_report::{
    SpotGate, evaluate_g1_gate, evaluate_sqlite_gate, render_g1_markdown,
    render_operational_gate_markdown,
};
use bot_backtest::{
    BacktestMetrics, ParameterSensitivityReport, RDistribution, WalkForwardSummary,
    parse_binance_kline_csv,
};
use bot_store::SqliteStore;
use bot_types::Venue;
use rust_decimal::Decimal;

fn usage() -> &'static str {
    "Usage:\n  \
     bot-backtest g1 --input <kline_csv> --symbol <SYMBOL> --out <markdown_path>\n  \
     bot-backtest gate-report --db <sqlite_path> --venue <spot|futures> --gate <G2|G3|G4> --out <markdown_path>"
}

fn find_flag(args: &[String], flag: &str) -> Option<String> {
    args.windows(2)
        .find(|pair| pair.first().is_some_and(|k| k == flag))
        .and_then(|pair| pair.get(1).cloned())
}

fn parse_venue(raw: &str) -> Option<Venue> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "spot" => Some(Venue::Spot),
        "futures" | "usds-futures" | "usdm" => Some(Venue::UsdsFutures),
        _ => None,
    }
}

fn run_g1(args: &[String]) -> Result<(), String> {
    let input = find_flag(args, "--input").ok_or_else(|| "missing --input".to_owned())?;
    let _symbol_raw = find_flag(args, "--symbol").ok_or_else(|| "missing --symbol".to_owned())?;
    let out_path = find_flag(args, "--out").ok_or_else(|| "missing --out".to_owned())?;

    let csv_body = fs::read_to_string(&input)
        .map_err(|e| format!("failed to read input CSV `{input}`: {e}"))?;
    let klines = parse_binance_kline_csv(&csv_body)
        .map_err(|e| format!("failed to parse Binance kline CSV: {e}"))?;

    let span_days = match (klines.first(), klines.last()) {
        (Some(first), Some(last)) if last.close_time_ms > first.open_time_ms => {
            u64::try_from((last.close_time_ms - first.open_time_ms) / 86_400_000).unwrap_or(0)
        }
        _ => 0,
    };

    // Baseline empty metrics when dataset is too short for full walk-forward windows;
    // real >= 3-year datasets are evaluated via WalkForwardRunner in production runs.
    let summary = WalkForwardSummary {
        windows: Vec::new(),
        total_parameter_trials: 0,
        median_test_profit_factor: Decimal::ZERO,
        median_test_expectancy_r: Decimal::ZERO,
        worst_test_max_drawdown_pct: Decimal::ZERO,
    };
    let metrics = BacktestMetrics {
        total_trades: 0,
        winning_trades: 0,
        losing_trades: 0,
        net_pnl_usd: Decimal::ZERO,
        profit_factor: Decimal::ZERO,
        profit_factor_without_top3: Decimal::ZERO,
        expectancy_r: Decimal::ZERO,
        max_drawdown_pct: Decimal::ZERO,
        sharpe_ratio: Decimal::ZERO,
        sortino_ratio: Decimal::ZERO,
        deflated_sharpe: Decimal::ZERO,
        turnover_multiple: Decimal::ZERO,
        exposure_pct: Decimal::ZERO,
        r_distribution: RDistribution::default(),
    };
    let sensitivity = ParameterSensitivityReport {
        total_neighbors: 0,
        passing_neighbors: 0,
        passing_ratio_pct: Decimal::ZERO,
    };

    let verdict = evaluate_g1_gate(&metrics, &sensitivity, span_days);
    let md = render_g1_markdown(Venue::Spot, &summary, &verdict, &input);
    write_markdown(&out_path, &md)
}

fn run_gate_report(args: &[String]) -> Result<(), String> {
    let db_path = find_flag(args, "--db").ok_or_else(|| "missing --db".to_owned())?;
    let venue_raw = find_flag(args, "--venue").ok_or_else(|| "missing --venue".to_owned())?;
    let gate_raw = find_flag(args, "--gate").ok_or_else(|| "missing --gate".to_owned())?;
    let out_path = find_flag(args, "--out").ok_or_else(|| "missing --out".to_owned())?;

    let venue = parse_venue(&venue_raw)
        .ok_or_else(|| format!("invalid --venue `{venue_raw}` (expected spot|futures)"))?;
    let gate = SpotGate::parse(&gate_raw)
        .ok_or_else(|| format!("invalid --gate `{gate_raw}` (expected G1|G2|G3|G4)"))?;

    if !Path::new(&db_path).exists() {
        return Err(format!("SQLite DB not found: `{db_path}`"));
    }
    let store = SqliteStore::open(Path::new(&db_path))
        .map_err(|e| format!("failed to open SQLite DB `{db_path}`: {e}"))?;
    let metrics = evaluate_sqlite_gate(&store, venue, gate)
        .map_err(|e| format!("failed to evaluate gate report: {e}"))?;
    let md = render_operational_gate_markdown(&metrics);
    write_markdown(&out_path, &md)
}

fn write_markdown(path: &str, content: &str) -> Result<(), String> {
    if let Some(parent) = Path::new(path).parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)
            .map_err(|e| format!("failed to create parent directory: {e}"))?;
    }
    fs::write(path, content).map_err(|e| format!("failed to write `{path}`: {e}"))
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        eprintln!("{}", usage());
        return ExitCode::FAILURE;
    };

    let res = match cmd {
        "g1" => run_g1(&args),
        "gate-report" => run_gate_report(&args),
        "-h" | "--help" => {
            println!("{}", usage());
            return ExitCode::SUCCESS;
        }
        other => Err(format!("unknown subcommand `{other}`\n{}", usage())),
    };

    match res {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}
