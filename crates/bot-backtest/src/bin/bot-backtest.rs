#![forbid(unsafe_code)]
//! Thin CLI binary for M14/M17 Spot & Futures gates:
//! - `bot-backtest g1 --input <csv> --symbol <SYMBOL> [--venue spot|futures] --out <md>`
//! - `bot-backtest gate-report --db <sqlite> [--scope <spot|futures-long|futures-short> | --venue <spot|futures>] --gate <G2|G3|G4> --out <md>`

use std::env;
use std::fs;
use std::path::Path;
use std::process::ExitCode;

use bot_backtest::gate_report::{
    GateScope, SpotGate, evaluate_g1_gate, evaluate_sqlite_gate, evaluate_sqlite_gate_for_scope,
    render_g1_markdown, render_operational_gate_markdown,
};
use bot_backtest::{
    BacktestConfig, BacktestRunner, CostModelConfig, ParameterSensitivityReport, WalkForwardConfig,
    WalkForwardRunner, evaluate_parameter_sensitivity, parse_binance_kline_csv,
};
use bot_config::BotConfig;
use bot_marketdata::Timeframe;
use bot_store::SqliteStore;
use bot_strategy::{CommissionRates, S1TrendPullback};
use bot_types::{InstrumentRules, Symbol, Venue};
use rust_decimal::Decimal;

fn usage() -> &'static str {
    "Usage:\n  \
     bot-backtest g1 --input <kline_csv> --symbol <SYMBOL> [--venue <spot|futures>] --out <markdown_path>\n  \
     bot-backtest gate-report --db <sqlite_path> [--scope <spot|futures-long|futures-short> | --venue <spot|futures>] --gate <G2|G3|G4> --out <markdown_path>"
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
    let symbol_raw = find_flag(args, "--symbol").ok_or_else(|| "missing --symbol".to_owned())?;
    let out_path = find_flag(args, "--out").ok_or_else(|| "missing --out".to_owned())?;
    let venue = match find_flag(args, "--venue") {
        Some(raw) => parse_venue(&raw)
            .ok_or_else(|| format!("invalid --venue `{raw}` (expected spot|futures)"))?,
        None => Venue::Spot,
    };

    let csv_body = fs::read_to_string(&input)
        .map_err(|e| format!("failed to read input CSV `{input}`: {e}"))?;
    let klines = parse_binance_kline_csv(&csv_body)
        .map_err(|e| format!("failed to parse Binance kline CSV: {e}"))?;
    if klines.is_empty() {
        return Err("input kline CSV contains 0 bars".to_owned());
    }

    let span_days = match (klines.first(), klines.last()) {
        (Some(first), Some(last)) if last.close_time_ms > first.open_time_ms => {
            u64::try_from((last.close_time_ms - first.open_time_ms) / 86_400_000).unwrap_or(0)
        }
        _ => 0,
    };

    let base_cfg = BacktestConfig {
        venue,
        symbol: Symbol(symbol_raw),
        timeframe: Timeframe::H1,
        initial_equity_usd: Decimal::from(10_000_u32),
        rules: InstrumentRules {
            venue,
            symbol: Symbol(find_flag(args, "--symbol").unwrap_or_else(|| "BTCUSDT".to_owned())),
            tick_size: Decimal::new(1, 2),
            step_size: Decimal::new(1, 5),
            min_qty: Decimal::new(1, 5),
            max_qty: Decimal::from(10_000_u32),
            min_notional: Decimal::from(10_u32),
            status_trading: true,
            base_precision: 5,
            quote_precision: 2,
        },
        cost_model: CostModelConfig {
            commission: CommissionRates {
                maker_bps: Decimal::from(10_u32),
                taker_bps: Decimal::from(10_u32),
            },
            tick_size: Decimal::new(1, 2),
            spread_bps: Decimal::from(2_u32),
            stress_multiplier: Decimal::from(2_u32), // §13.3 2x slippage stress
        },
        bot_config: BotConfig::default(),
        seed: 42,
    };

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create tokio runtime: {e}"))?;

    let (summary, metrics, sensitivity) = rt.block_on(async {
        let wf_cfg = WalkForwardConfig::default_12m_3m_3m(Timeframe::H1);
        let candidates = [
            Decimal::new(24, 1), // -20%
            Decimal::new(30, 1), // baseline 3.0R
            Decimal::new(36, 1), // +20%
        ];
        let wf_summary = WalkForwardRunner::run_s1(&base_cfg, &wf_cfg, &candidates, &klines)
            .await
            .map_err(|e| format!("walk-forward run failed: {e}"))?;

        let mut neighbor_pfs = Vec::with_capacity(candidates.len());
        let mut baseline_metrics = None;
        for &tp2_r in &candidates {
            let mut cfg = base_cfg.clone();
            cfg.bot_config.strategy.trend_pullback.tp2_r = tp2_r;
            let mut strat = S1TrendPullback::new(
                cfg.bot_config.strategy.trend_pullback.clone(),
                Decimal::from(5_u32),
            );
            let trace = BacktestRunner::run_with_trace_and_trials(
                &cfg,
                &mut strat,
                &klines,
                candidates.len(),
            )
            .await
            .map_err(|e| format!("replay run failed: {e}"))?;
            neighbor_pfs.push(trace.report.metrics.profit_factor);
            if tp2_r == Decimal::new(30, 1) {
                baseline_metrics = Some(trace.report.metrics);
            }
        }
        let metrics =
            baseline_metrics.ok_or_else(|| "baseline S1 replay metrics missing".to_owned())?;
        let sensitivity: ParameterSensitivityReport =
            evaluate_parameter_sensitivity(&neighbor_pfs, Decimal::new(11, 1));
        Ok::<_, String>((wf_summary, metrics, sensitivity))
    })?;

    let verdict = evaluate_g1_gate(&metrics, &sensitivity, span_days);
    let md = render_g1_markdown(venue, &summary, &verdict, &input);
    write_markdown(&out_path, &md)
}

fn run_gate_report(args: &[String]) -> Result<(), String> {
    let db_path = find_flag(args, "--db").ok_or_else(|| "missing --db".to_owned())?;
    let gate_raw = find_flag(args, "--gate").ok_or_else(|| "missing --gate".to_owned())?;
    let out_path = find_flag(args, "--out").ok_or_else(|| "missing --out".to_owned())?;

    let gate = SpotGate::parse(&gate_raw)
        .ok_or_else(|| format!("invalid --gate `{gate_raw}` (expected G1|G2|G3|G4)"))?;

    if !Path::new(&db_path).exists() {
        return Err(format!("SQLite DB not found: `{db_path}`"));
    }
    let store = SqliteStore::open(Path::new(&db_path))
        .map_err(|e| format!("failed to open SQLite DB `{db_path}`: {e}"))?;

    let metrics = if let Some(scope_raw) = find_flag(args, "--scope") {
        let scope = GateScope::parse(&scope_raw).ok_or_else(|| {
            format!("invalid --scope `{scope_raw}` (expected spot|futures-long|futures-short)")
        })?;
        evaluate_sqlite_gate_for_scope(&store, scope, gate)
    } else {
        let venue_raw =
            find_flag(args, "--venue").ok_or_else(|| "missing --scope or --venue".to_owned())?;
        let venue = parse_venue(&venue_raw)
            .ok_or_else(|| format!("invalid --venue `{venue_raw}` (expected spot|futures)"))?;
        evaluate_sqlite_gate(&store, venue, gate)
    }
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
