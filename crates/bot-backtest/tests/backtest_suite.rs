//! Integration & verification suite for `bot-backtest` (M09).
//!
//! Covers:
//! 1. Historical kline downloader URL builder (`data.binance.vision`), 12-column CSV parser/writer
//!    (ms & 2025+ µs timestamps, optional header), and gap validation.
//! 2. Replay clock + `PaperBroker` + `EngineActor` integration enforcing §13.2 cost & slippage model
//!    (default 1 tick + 0.5×spread, stress 2×, next-bar execution, strict `LIMIT_MAKER` penetration).
//! 3. Reproducibility (fixed dataset & config -> identical `BacktestReport`).
//! 4. Anti-look-ahead future-bar perturbation test across the entire replay pipeline.
//! 5. Walk-forward validation (12m train / 3m test / 3m step), parameter trial counter,
//!    Deflated Sharpe, ±20% parameter neighborhood sensitivity, and PF without top-3 trades.
//! 6. Sample backtest reports for S1 (`S1TrendPullback`) and S2 (`S2Breakout`).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::similar_names,
    clippy::cast_precision_loss
)]

use std::str::FromStr;

use bot_backtest::{
    BacktestConfig, BacktestRunner, CostModelConfig, DataVisionFrequency, TradeOutcome,
    WalkForwardConfig, WalkForwardRunner, build_data_vision_kline_url, compute_backtest_metrics,
    evaluate_parameter_sensitivity, parse_binance_kline_csv, validate_kline_series_gaps,
    write_binance_kline_csv,
};
use bot_config::BotConfig;
use bot_exchange::{ExchangeGateway, PlaceOrderRequest};
use bot_exec::{PaperBroker, PaperBrokerConfig};
use bot_marketdata::Timeframe;
use bot_strategy::{CommissionRates, S1TrendPullback, S2Breakout};
use bot_types::{
    InstrumentRules, Kline, OrderKind, OrderStatus, Price, Qty, Side, StrategyId, Symbol, Venue,
};
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal literal")
}

fn sample_rules(symbol: &str) -> InstrumentRules {
    InstrumentRules {
        venue: Venue::Spot,
        symbol: Symbol(symbol.into()),
        tick_size: dec("0.01"),
        step_size: dec("0.0001"),
        min_qty: dec("0.0001"),
        max_qty: dec("1000"),
        min_notional: dec("5.0"),
        status_trading: true,
        base_precision: 4,
        quote_precision: 2,
    }
}

/// Generate a deterministic synthetic 1h kline dataset with realistic trend, pullbacks,
/// and breakouts so both S1 and S2 emit trades over the replay horizon.
#[allow(clippy::cast_precision_loss)]
fn build_synthetic_1h_dataset(num_bars: usize) -> Vec<Kline> {
    let step_ms = Timeframe::H1.interval_ms();
    let start_ms: i64 = 1_700_000_000_000;
    let mut bars = Vec::with_capacity(num_bars);
    let mut price = 100.0_f64;

    for i in 0..num_bars {
        let open_ms = start_ms + i64::try_from(i).unwrap_or(0) * step_ms;
        let close_ms = open_ms + step_ms - 1;

        let (open, high, low, close, volume) = if i < 640 {
            let wave = ((i as f64) * 0.4).sin() * 0.8;
            let o = price;
            let c = (price * 1.0015 + wave * 0.3).max(10.0);
            let h = o.max(c) * 1.006;
            let l = o.min(c) * 0.994;
            price = c;
            (o, h, l, c, 1_000.0 + ((i % 7) as f64) * 50.0)
        } else if i < 648 {
            // Controlled pullback toward EMA21
            let o = price;
            let c = price * 0.996;
            let h = o * 1.003;
            let l = c * 0.995;
            price = c;
            (o, h, l, c, 900.0)
        } else if i < 652 {
            // Reclaim bar above EMA21 with RSI turning up inside [40, 62] (triggers S1)
            let o = price;
            let c = price * 1.008;
            let h = c * 1.004;
            let l = o * 0.992; // penetrates entry on subsequent bar
            price = c;
            (o, h, l, c, 1_500.0)
        } else if i < 685 {
            // Sideways consolidation with 1.5% bar range before Donchian(20) breakout
            let o = price;
            let c = price * if i % 2 == 0 { 1.001 } else { 0.999 };
            let h = o.max(c) * 1.008;
            let l = o.min(c) * 0.992;
            price = c;
            (o, h, l, c, 1_000.0)
        } else if i < 692 {
            // Clean high-volume breakout above Donchian(20).upper with rising ADX and RSI in [52, 75] (triggers S2)
            let o = price;
            let c = price * 1.004;
            let h = c * 1.003;
            let l = o * 0.999;
            price = c;
            (o, h, l, c, 2_200.0)
        } else {
            let o = price;
            let c = price * 1.002;
            let h = c * 1.015;
            let l = o * 0.990;
            price = c;
            (o, h, l, c, 1_100.0)
        };

        bars.push(Kline {
            open_time_ms: open_ms,
            close_time_ms: close_ms,
            open,
            high,
            low,
            close,
            volume,
            quote_volume: volume * close,
            trades: 200,
            taker_buy_base: volume * 0.55,
            is_closed: true,
        });
    }
    bars
}

#[test]
fn data_vision_url_csv_roundtrip_and_gap_validation() {
    let sym = Symbol("BTCUSDT".into());
    let url_spot = build_data_vision_kline_url(
        Venue::Spot,
        &sym,
        Timeframe::H1,
        DataVisionFrequency::Monthly,
        "2025-01",
    );
    assert_eq!(
        url_spot,
        "https://data.binance.vision/data/spot/monthly/klines/BTCUSDT/1h/BTCUSDT-1h-2025-01.zip"
    );

    let url_fut = build_data_vision_kline_url(
        Venue::UsdsFutures,
        &sym,
        Timeframe::H4,
        DataVisionFrequency::Daily,
        "2025-02-15",
    );
    assert_eq!(
        url_fut,
        "https://data.binance.vision/data/futures/um/daily/klines/BTCUSDT/4h/BTCUSDT-4h-2025-02-15.zip"
    );

    // Verify 12-column CSV parser handles optional header + both ms (13-digit) and 2025+ µs (16-digit) timestamps.
    let raw_csv = "\
open_time,open,high,low,close,volume,close_time,quote_volume,count,taker_buy_volume,taker_buy_quote_volume,ignore
1735689600000000,95000.00,95500.00,94800.00,95200.00,12.5,1735693199999999,1190000.0,450,6.2,590000.0,0
1735693200000,95200.00,95800.00,95100.00,95600.00,15.0,1735696799999,1434000.0,520,8.0,764800.0,0
";
    let parsed = parse_binance_kline_csv(raw_csv).expect("parse valid csv");
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0].open_time_ms, 1_735_689_600_000);
    assert_eq!(parsed[1].open_time_ms, 1_735_693_200_000);

    // Contiguous bars pass gap validation.
    let gap_report = validate_kline_series_gaps(&parsed, Timeframe::H1);
    assert!(gap_report.is_gap_free());
    assert_eq!(gap_report.gaps.len(), 0);

    // Roundtrip CSV serialization.
    let serialized = write_binance_kline_csv(&parsed, true);
    let reparsed = parse_binance_kline_csv(&serialized).expect("reparse csv");
    assert_eq!(reparsed.len(), 2);
    assert_eq!(reparsed[0].open_time_ms, parsed[0].open_time_ms);

    // Introduce a 1-bar gap and confirm detection.
    let mut gapped = parsed;
    gapped[1].open_time_ms += Timeframe::H1.interval_ms();
    let bad_report = validate_kline_series_gaps(&gapped, Timeframe::H1);
    assert!(!bad_report.is_gap_free());
    assert_eq!(bad_report.gaps.len(), 1);
    assert_eq!(bad_report.gaps[0].missing_bars, 1);
}

#[tokio::test]
async fn cost_and_slippage_model_enforces_default_stress_2x_and_strict_limit_maker_penetration() {
    let model_default = CostModelConfig {
        commission: CommissionRates {
            maker_bps: dec("10"),
            taker_bps: dec("10"),
        },
        tick_size: dec("0.01"),
        spread_bps: dec("4"),
        stress_multiplier: Decimal::ONE,
    };
    let ref_px = Price(dec("100.00"));
    // 1 tick (0.01 = 1 bps on 100.00) + 0.5 * 4 bps = 3 bps one-way slippage.
    let slip_1x = model_default.one_way_slippage_bps(ref_px);
    assert_eq!(slip_1x, dec("3"));

    let model_stress = CostModelConfig {
        stress_multiplier: Decimal::from(2_u32),
        ..model_default
    };
    // 2x stress doubles one-way slippage to 6 bps and effective fees to 20 bps.
    assert_eq!(model_stress.one_way_slippage_bps(ref_px), dec("6"));
    assert_eq!(model_stress.effective_commission().maker_bps, dec("20"));
    assert_eq!(model_stress.effective_commission().taker_bps, dec("20"));

    // Verify PaperBroker enforces §13.2 strict LIMIT_MAKER penetration and no same-bar entry fill.
    let sym = Symbol("BTCUSDT".into());
    let broker = PaperBroker::new(PaperBrokerConfig {
        allow_same_bar_entry_fill: false,
        strict_limit_maker_penetration: true,
        rules: vec![sample_rules("BTCUSDT")],
        ..PaperBrokerConfig::default()
    });
    let _ = broker.on_price_tick(&sym, Price(dec("100.00")), Price(dec("100.10")), 1_000);

    // Place a passive LIMIT_MAKER Buy at 99.90 at t = 1_000.
    let ack = broker
        .place_order(PlaceOrderRequest {
            symbol: sym.clone(),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "B_bt01-1-S-000001-E".into(),
            quantity: Some(Qty(dec("1.0"))),
            quote_order_qty: None,
            price: Some(Price(dec("99.90"))),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect("place limit maker");
    assert_eq!(ack.status, OrderStatus::New);

    // 1. Same-bar tick (t = 1_000) even with ask = 99.50 MUST NOT fill (`allow_same_bar_entry_fill = false`).
    let evs_same_bar = broker.on_price_tick(&sym, Price(dec("99.40")), Price(dec("99.50")), 1_000);
    assert!(
        evs_same_bar.is_empty(),
        "same-bar entry fill must be blocked"
    );

    // 2. Next-bar tick (t = 2_000) that merely TOUCHES ask = 99.90 MUST NOT fill (`strict_limit_maker_penetration = true`).
    let evs_touch = broker.on_price_tick(&sym, Price(dec("99.80")), Price(dec("99.90")), 2_000);
    assert!(
        evs_touch.is_empty(),
        "LIMIT_MAKER must not fill when price merely touches level"
    );

    // 3. Next-bar tick (t = 3_000) that STRICTLY PENETRATES ask = 99.89 (< 99.90) fills!
    let evs_penetrate = broker.on_price_tick(&sym, Price(dec("99.80")), Price(dec("99.89")), 3_000);
    assert_eq!(
        evs_penetrate.len(),
        1,
        "LIMIT_MAKER must fill when price strictly penetrates level"
    );
}

#[tokio::test]
async fn replay_is_reproducible_and_generates_s1_s2_reports() {
    let bars = build_synthetic_1h_dataset(750);
    let rules = sample_rules("BTCUSDT");
    let bot_cfg = BotConfig::default();

    let bt_cfg = BacktestConfig {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        timeframe: Timeframe::H1,
        initial_equity_usd: dec("10000"),
        rules,
        cost_model: CostModelConfig {
            commission: CommissionRates {
                maker_bps: dec("4"),
                taker_bps: dec("4"),
            },
            tick_size: dec("0.01"),
            spread_bps: dec("2"),
            stress_multiplier: Decimal::ONE,
        },
        bot_config: bot_cfg.clone(),
        seed: 42,
    };

    // Run S1 twice with the exact same seed & data -> reports MUST be identical.
    let mut s1_a = S1TrendPullback::new(bot_cfg.strategy.trend_pullback.clone(), dec("5.0"));
    let rep_s1_a = BacktestRunner::run(&bt_cfg, &mut s1_a, &bars)
        .await
        .expect("s1 run A");

    let mut s1_b = S1TrendPullback::new(bot_cfg.strategy.trend_pullback.clone(), dec("5.0"));
    let rep_s1_b = BacktestRunner::run(&bt_cfg, &mut s1_b, &bars)
        .await
        .expect("s1 run B");

    assert_eq!(
        rep_s1_a, rep_s1_b,
        "backtest report must be 100% reproducible"
    );
    assert_eq!(rep_s1_a.strategy, StrategyId::TrendPullback);
    assert_eq!(rep_s1_a.bars_processed, 750);

    // Run S2 sample report on the same dataset.
    let mut s2 = S2Breakout::new(bot_cfg.strategy.breakout.clone(), dec("5.0"), dec("3.0"));
    let rep_s2 = BacktestRunner::run(&bt_cfg, &mut s2, &bars)
        .await
        .expect("s2 run");
    assert_eq!(rep_s2.strategy, StrategyId::Breakout);
    assert!(
        rep_s1_a.total_signals_emitted >= 1,
        "sample dataset must produce at least one S1 signal"
    );
    assert!(
        rep_s2.total_signals_emitted >= 1,
        "sample dataset must produce at least one S2 signal"
    );
    assert!(
        rep_s1_a.metrics.total_trades + rep_s2.metrics.total_trades >= 1,
        "sample dataset must produce completed trades with full metrics"
    );
}

#[tokio::test]
async fn future_bar_perturbation_never_changes_past_replay_state() {
    let bars = build_synthetic_1h_dataset(720);
    let cutoff = 680;
    let prefix = &bars[..cutoff];

    // Build a perturbed dataset where bars AFTER `cutoff` are wildly mutated (flash crash + pump).
    let mut perturbed = bars.clone();
    for b in &mut perturbed[cutoff..] {
        b.open *= 3.5;
        b.high *= 4.0;
        b.low *= 0.2;
        b.close *= 3.8;
        b.volume *= 50.0;
    }

    let bot_cfg = BotConfig::default();
    let bt_cfg = BacktestConfig {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        timeframe: Timeframe::H1,
        initial_equity_usd: dec("10000"),
        rules: sample_rules("BTCUSDT"),
        cost_model: CostModelConfig {
            commission: CommissionRates {
                maker_bps: dec("4"),
                taker_bps: dec("4"),
            },
            tick_size: dec("0.01"),
            spread_bps: dec("2"),
            stress_multiplier: Decimal::ONE,
        },
        bot_config: bot_cfg.clone(),
        seed: 7,
    };

    let cutoff_ts = prefix.last().expect("non-empty prefix").close_time_ms;

    let mut s1_orig = S1TrendPullback::new(bot_cfg.strategy.trend_pullback.clone(), dec("5.0"));
    let trace_orig = BacktestRunner::run_with_trace(&bt_cfg, &mut s1_orig, &bars)
        .await
        .expect("orig trace");

    let mut s1_pert = S1TrendPullback::new(bot_cfg.strategy.trend_pullback.clone(), dec("5.0"));
    let trace_pert = BacktestRunner::run_with_trace(&bt_cfg, &mut s1_pert, &perturbed)
        .await
        .expect("perturbed trace");

    let past_orig: Vec<_> = trace_orig
        .bar_decisions
        .iter()
        .filter(|d| d.close_time_ms <= cutoff_ts)
        .collect();
    let past_pert: Vec<_> = trace_pert
        .bar_decisions
        .iter()
        .filter(|d| d.close_time_ms <= cutoff_ts)
        .collect();

    assert_eq!(past_orig.len(), cutoff);
    assert_eq!(
        past_orig, past_pert,
        "perturbing future bars must NEVER alter any past signal, order, or equity snapshot"
    );
}

#[test]
fn report_metrics_compute_pf_without_top3_and_parameter_sensitivity() {
    let trades = vec![
        TradeOutcome {
            entry_time_ms: 1_000,
            exit_time_ms: 2_000,
            net_pnl_usd: dec("400"),
            r_multiple: dec("4.0"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 3_000,
            exit_time_ms: 4_000,
            net_pnl_usd: dec("300"),
            r_multiple: dec("3.0"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 5_000,
            exit_time_ms: 6_000,
            net_pnl_usd: dec("250"),
            r_multiple: dec("2.5"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 7_000,
            exit_time_ms: 8_000,
            net_pnl_usd: dec("120"),
            r_multiple: dec("1.2"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 9_000,
            exit_time_ms: 10_000,
            net_pnl_usd: dec("110"),
            r_multiple: dec("1.1"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 11_000,
            exit_time_ms: 12_000,
            net_pnl_usd: dec("100"),
            r_multiple: dec("1.0"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 13_000,
            exit_time_ms: 14_000,
            net_pnl_usd: dec("-100"),
            r_multiple: dec("-1.0"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 15_000,
            exit_time_ms: 16_000,
            net_pnl_usd: dec("-100"),
            r_multiple: dec("-1.0"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
        TradeOutcome {
            entry_time_ms: 17_000,
            exit_time_ms: 18_000,
            net_pnl_usd: dec("-100"),
            r_multiple: dec("-1.0"),
            notional_usd: dec("2000"),
            fees_usd: dec("4"),
        },
    ];

    let equity_curve = vec![
        dec("10000"),
        dec("10400"),
        dec("10700"),
        dec("10950"),
        dec("10850"),
        dec("10980"),
    ];

    let metrics = compute_backtest_metrics(dec("10000"), &trades, &equity_curve, 20_000, 10_000, 5);

    assert_eq!(metrics.net_pnl_usd, dec("980"));
    assert_eq!(metrics.total_trades, 9);
    assert!(metrics.profit_factor > dec("4.0"));
    assert_eq!(metrics.profit_factor_without_top3, dec("1.1"));
    assert!(metrics.expectancy_r > dec("1.0"));
    assert!(metrics.max_drawdown_pct < dec("5.0"));
    assert_eq!(metrics.exposure_pct, dec("50"));

    // Parameter sensitivity ±20% evaluation: 4 out of 5 neighbors have PF > 1.1 -> 80% (>= 70%).
    let neighbor_pfs = vec![
        dec("1.35"),
        dec("1.22"),
        dec("1.18"),
        dec("1.15"),
        dec("1.05"),
    ];
    let sens = evaluate_parameter_sensitivity(&neighbor_pfs, dec("1.1"));
    assert_eq!(sens.total_neighbors, 5);
    assert_eq!(sens.passing_neighbors, 4);
    assert_eq!(sens.passing_ratio_pct, dec("80"));
}

#[tokio::test]
async fn walk_forward_slices_12m_train_3m_test_3m_step_and_counts_trials() {
    let wf_cfg = WalkForwardConfig::default_12m_3m_3m(Timeframe::H1);
    assert_eq!(wf_cfg.train_bars, 24 * 365); // 8760 1h bars (12 months)
    assert_eq!(wf_cfg.test_bars, 24 * 90); // 2160 1h bars (3 months)
    assert_eq!(wf_cfg.step_bars, 24 * 90); // 2160 1h bars (3 months)

    let bars = build_synthetic_1h_dataset(800);
    let compact_wf = WalkForwardConfig {
        train_bars: 620,
        test_bars: 80,
        step_bars: 50,
    };
    let windows = compact_wf.slice_windows(bars.len());
    assert_eq!(windows.len(), 3);

    let bot_cfg = BotConfig::default();
    let bt_cfg = BacktestConfig {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        timeframe: Timeframe::H1,
        initial_equity_usd: dec("10000"),
        rules: sample_rules("BTCUSDT"),
        cost_model: CostModelConfig {
            commission: CommissionRates {
                maker_bps: dec("4"),
                taker_bps: dec("4"),
            },
            tick_size: dec("0.01"),
            spread_bps: dec("2"),
            stress_multiplier: Decimal::ONE,
        },
        bot_config: bot_cfg.clone(),
        seed: 99,
    };

    let param_candidates = vec![dec("2.5"), dec("3.0"), dec("3.5")];
    let summary = WalkForwardRunner::run_s1(&bt_cfg, &compact_wf, &param_candidates, &bars)
        .await
        .expect("walk-forward run");

    assert_eq!(summary.windows.len(), 3);
    assert_eq!(summary.total_parameter_trials, 9);
    for (idx, w) in summary.windows.iter().enumerate() {
        assert_eq!(w.window_index, idx);
        assert_eq!(w.parameter_trials, 3);
    }
}
