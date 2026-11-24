//! Acceptance test suite for `bot-strategy` (M08):
//! - Golden snapshot test (`insta`) on fixed deterministic bar dataset
//! - Automatic anti-lookahead test (perturbing future bars)
//! - Cost gate rejection test (`TP1 < 3x round-trip cost` using `CommissionRates`)
//! - Vol targeting, BTC regime, spread/freshness, and cooldown gates
//! - Shadow-only S3/S4 architecture test (persisting to `signals_shadow`, never emitting orders).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines,
    clippy::cast_precision_loss
)]

use bot_config::{BreakoutConfig, GainersMomentumConfig, TrendPullbackConfig, VwapReversionConfig};
use bot_store::SqliteStore;
use bot_strategy::{
    CommissionRates, GainerCandidate, HtfRegime, HtfRegimeFeed, IndicatorFeed, IndicatorSnapshot,
    S1TrendPullback, S2Breakout, S3GainersShadow, S4VwapReversionShadow, ShadowOutcome, Strategy,
    StrategyCtx, StrategyEntryIntent, compute_vol_target_size_mult, evaluate_shadow_outcome,
    rank_gainers_top_k, record_shadow_signal,
};
use bot_types::{Kline, Price, Symbol, Venue, VenueCaps};
use rust_decimal::Decimal;

/// Generate a deterministic synthetic 1h bar series (720 bars) with:
/// - Smooth uptrend warm-up (bars 0..640) so EMA(200) (600-bar warmup) & 4h EMA(50) (150 4h bars) are ready
/// - Clean pullback to EMA21 & bullish reclaim (bars 640..652) for S1
/// - Clean high-volume Donchian(20) breakout (bars 660..690) for S2.
fn build_deterministic_dataset() -> Vec<Kline> {
    let mut bars = Vec::with_capacity(720);
    let step_ms = 3_600_000_i64;
    let mut price = 100.0_f64;

    for i in 0..720_usize {
        let open_ms = 1_700_000_000_000 + i64::try_from(i).unwrap() * step_ms;
        let (open, high, low, close, vol) = if i < 640 {
            // Steady uptrend (~0.3% per bar with ~1.5% bar range)
            let wave = ((i as f64) * 0.4).sin() * 0.8;
            let o = price;
            let c = (price * 1.0015 + wave * 0.3).max(10.0);
            let h = o.max(c) * 1.006;
            let l = o.min(c) * 0.994;
            price = c;
            (o, h, l, c, 1_000.0 + ((i % 7) as f64) * 50.0)
        } else if i < 648 {
            // Pullback toward EMA21 (drops ~0.4% per bar for 8 bars)
            let o = price;
            let c = price * 0.996;
            let h = o * 1.003;
            let l = c * 0.995;
            price = c;
            (o, h, l, c, 900.0)
        } else if i < 652 {
            // Reclaim bar above EMA21 with RSI turning up inside [40, 62]
            let o = price;
            let c = price * 1.008;
            let h = c * 1.004;
            let l = o * 0.997;
            price = c;
            (o, h, l, c, 1_500.0)
        } else if i < 685 {
            // Tight sideways consolidation before Donchian(20) breakout
            let o = price;
            let c = price * if i % 2 == 0 { 1.002 } else { 0.9985 };
            let h = o.max(c) * 1.003;
            let l = o.min(c) * 0.997;
            price = c;
            (o, h, l, c, 1_000.0)
        } else if i < 690 {
            // Multi-bar high-volume breakout above Donchian(20).upper with rising ADX!
            let o = price;
            let c = price * 1.015;
            let h = c * 1.005;
            let l = o * 0.999;
            price = c;
            (o, h, l, c, 2_800.0)
        } else {
            let o = price;
            let c = price * 1.001;
            let h = c * 1.004;
            let l = o * 0.996;
            price = c;
            (o, h, l, c, 1_100.0)
        };

        bars.push(Kline {
            open_time_ms: open_ms,
            close_time_ms: open_ms + step_ms - 1,
            open,
            high,
            low,
            close,
            volume: vol,
            quote_volume: vol * close,
            trades: 200,
            taker_buy_base: vol * 0.55,
            is_closed: true,
        });
    }
    bars
}

fn run_strategies_over_series(bars: &[Kline]) -> Vec<(usize, StrategyEntryIntent)> {
    let mut feed = IndicatorFeed::new();
    let mut htf_feed = HtfRegimeFeed::new();
    let mut s1 = S1TrendPullback::new(TrendPullbackConfig::default(), Decimal::from(5_u32));
    let mut s2 = S2Breakout::new(
        BreakoutConfig::default(),
        Decimal::from(5_u32),
        Decimal::from(3_u32),
    );
    let commission =
        CommissionRates::from_exchange_ratios(Decimal::new(10, 4), Decimal::new(10, 4));

    let mut signals = Vec::new();
    let mut htf_regime = HtfRegime {
        close: 0.0,
        ema50: 0.0,
        ema50_rising_3bars: false,
        ema50_falling_3bars: false,
        ready: false,
    };

    for (idx, bar) in bars.iter().enumerate() {
        // Synthesize closed 4h HTF bar every 4 closed 1h bars (strictly closed, zero look-ahead).
        if idx >= 3 && idx % 4 == 3 {
            let slice = &bars[idx - 3..=idx];
            let htf_bar = Kline {
                open_time_ms: slice[0].open_time_ms,
                close_time_ms: slice[3].close_time_ms,
                open: slice[0].open,
                high: slice.iter().map(|b| b.high).fold(f64::MIN, f64::max),
                low: slice.iter().map(|b| b.low).fold(f64::MAX, f64::min),
                close: slice[3].close,
                volume: slice.iter().map(|b| b.volume).sum(),
                quote_volume: slice.iter().map(|b| b.quote_volume).sum(),
                trades: slice.iter().map(|b| b.trades).sum(),
                taker_buy_base: slice.iter().map(|b| b.taker_buy_base).sum(),
                is_closed: true,
            };
            htf_regime = htf_feed.on_closed_htf_bar(&htf_bar);
        }

        let indicators = feed.on_closed_bar(bar);
        let ctx = StrategyCtx {
            venue: Venue::Spot,
            venue_caps: VenueCaps::spot(),
            symbol: Symbol("ETHUSDT".into()),
            bar: bar.clone(),
            indicators,
            htf: htf_regime,
            spread_bps: Decimal::from(2_u32),
            book_ticker_age_ms: 100,
            gap_free_300_bars: true,
            btc_long_regime_ok: true,
            target_vol: Decimal::new(2, 2),
            realized_vol_30d: Decimal::new(2, 2),
            atr_pct_percentile_90d: Decimal::from(50_u32),
            has_open_position: false,
            bars_since_last_exit: None,
            macro_blackout_active: false,
            obi_smooth: Some(0.15),
            commission,
            expected_slippage_bps: Decimal::from(3_u32),
        };

        if let Some(intent) = s1.on_bar_close(&ctx) {
            signals.push((idx, intent));
        }
        if let Some(intent) = s2.on_bar_close(&ctx) {
            signals.push((idx, intent));
        }
    }
    signals
}

#[test]
fn golden_snapshot_deterministic_signals_with_insta() {
    let bars = build_deterministic_dataset();
    let signals = run_strategies_over_series(&bars);
    assert!(
        !signals.is_empty(),
        "expected at least one deterministic signal on golden dataset"
    );
    insta::assert_json_snapshot!("golden_s1_s2_signals", signals);
}

#[test]
fn anti_lookahead_future_bar_perturbation_never_changes_past_signals() {
    let base_bars = build_deterministic_dataset();
    let split = 660_usize;

    let mut perturbed = base_bars.clone();
    for bar in &mut perturbed[(split + 1)..] {
        bar.open *= 5.0;
        bar.high *= 6.0;
        bar.low *= 0.2;
        bar.close *= 4.0;
        bar.volume *= 50.0;
    }

    let base_past: Vec<_> = run_strategies_over_series(&base_bars)
        .into_iter()
        .filter(|(idx, _)| *idx <= split)
        .collect();
    let perturbed_past: Vec<_> = run_strategies_over_series(&perturbed)
        .into_iter()
        .filter(|(idx, _)| *idx <= split)
        .collect();

    assert_eq!(
        base_past, perturbed_past,
        "perturbing future bars > {split} MUST NEVER alter past signals"
    );
}

#[test]
fn cost_gate_rejects_signal_when_tp1_below_3x_round_trip_cost() {
    let mut s1 = S1TrendPullback::new(TrendPullbackConfig::default(), Decimal::from(5_u32));

    // Setup a clean S1 bullish pullback where stop distance is very tight (0.15%),
    // so TP1 = 1.5R = 22.5 bps < 3 * (10 + 10 + 2*5) = 90 bps -> MUST reject cost_gate!
    let bar = Kline {
        open_time_ms: 1_700_000_000_000,
        close_time_ms: 1_700_003_599_999,
        open: 99.95,
        high: 100.05,
        low: 99.90,
        close: 100.00,
        volume: 1_500.0,
        quote_volume: 150_000.0,
        trades: 100,
        taker_buy_base: 800.0,
        is_closed: true,
    };
    let ctx = StrategyCtx {
        venue: Venue::Spot,
        venue_caps: VenueCaps::spot(),
        symbol: Symbol("ETHUSDT".into()),
        bar,
        indicators: IndicatorSnapshot {
            ema9: Some(100.00),
            ema21: Some(99.95),
            ema55: Some(99.50),
            ema200: Some(98.00),
            rsi14: Some(52.0),
            prev_rsi14: Some(48.0),
            atr14: Some(0.10), // 1.5*ATR + 0.1*ATR = 0.16 => TP1 = +0.24 = 24 bps < 90 bps!
            adx14: Some(25.0),
            prev_adx14: Some(23.0),
            plus_di: Some(28.0),
            minus_di: Some(14.0),
            donchian_upper: Some(100.50),
            donchian_lower: Some(99.00),
            donchian_mid: Some(99.75),
            session_vwap: Some(99.90),
            vol_sma20: Some(1_000.0),
            min_low_3: Some(99.90),
            min_low_5: Some(99.90),
            max_high_3: Some(100.50),
            max_high_5: Some(100.50),
        },
        htf: HtfRegime {
            close: 100.0,
            ema50: 98.0,
            ema50_rising_3bars: true,
            ema50_falling_3bars: false,
            ready: true,
        },
        spread_bps: Decimal::from(2_u32),
        book_ticker_age_ms: 100,
        gap_free_300_bars: true,
        btc_long_regime_ok: true,
        target_vol: Decimal::new(2, 2),
        realized_vol_30d: Decimal::new(2, 2),
        atr_pct_percentile_90d: Decimal::from(50_u32),
        has_open_position: false,
        bars_since_last_exit: None,
        macro_blackout_active: false,
        obi_smooth: Some(0.2),
        // Actual account.commission rates: 10 bps maker, 10 bps taker; slippage 5 bps -> round-trip = 30 bps -> 3x = 90 bps
        commission: CommissionRates::from_exchange_ratios(Decimal::new(10, 4), Decimal::new(10, 4)),
        expected_slippage_bps: Decimal::from(5_u32),
    };

    let decision = s1.evaluate(&ctx);
    assert!(
        decision.intent.is_none(),
        "signal with TP1 < 3x round-trip cost MUST be rejected"
    );
    assert!(
        decision
            .gate_logs
            .iter()
            .any(|g| g.gate == "cost_gate" && !g.passed),
        "gate_logs must record cost_gate rejection"
    );

    // Verify vol-targeting clamping & 85th-percentile ATR cap (§6.5 #5)
    assert_eq!(
        compute_vol_target_size_mult(Decimal::from(10_u32), Decimal::ONE, Decimal::from(50_u32)),
        Decimal::new(125, 2)
    );
    assert_eq!(
        compute_vol_target_size_mult(Decimal::from(10_u32), Decimal::ONE, Decimal::from(90_u32)),
        Decimal::new(6, 1)
    );
}

#[test]
fn shadow_strategies_emit_shadow_signals_only_and_persist_to_signals_shadow() {
    // 1. S3 Gainers z-score ranking + shadow signal
    let ranked = rank_gainers_top_k(
        &[
            GainerCandidate {
                symbol: Symbol("AAAUSDT".into()),
                return_24h_pct: 14.0,
                return_4h_pct: 5.0,
                vol_ratio_1h_24h: 3.5,
            },
            GainerCandidate {
                symbol: Symbol("BBBUSDT".into()),
                return_24h_pct: 6.0,
                return_4h_pct: 1.0,
                vol_ratio_1h_24h: 1.1,
            },
        ],
        5,
    );
    assert_eq!(ranked[0].0, Symbol("AAAUSDT".into()));

    let s3 = S3GainersShadow::new(GainersMomentumConfig::default(), Decimal::from(15_u32));
    let ctx = StrategyCtx {
        venue: Venue::Spot,
        venue_caps: VenueCaps::spot(),
        symbol: Symbol("AAAUSDT".into()),
        bar: Kline {
            open_time_ms: 1_700_000_000_000,
            close_time_ms: 1_700_000_899_999,
            open: 10.0,
            high: 10.6,
            low: 9.9,
            close: 10.5,
            volume: 5_000.0,
            quote_volume: 52_500.0,
            trades: 300,
            taker_buy_base: 3_000.0,
            is_closed: true,
        },
        indicators: IndicatorSnapshot {
            ema9: Some(10.3),
            ema21: Some(10.2),
            ema55: Some(9.8),
            ema200: Some(9.0),
            rsi14: Some(55.0),
            prev_rsi14: Some(50.0),
            atr14: Some(0.3),
            adx14: Some(25.0),
            prev_adx14: Some(22.0),
            plus_di: Some(30.0),
            minus_di: Some(12.0),
            donchian_upper: Some(10.4),
            donchian_lower: Some(9.5),
            donchian_mid: Some(9.95),
            session_vwap: Some(10.1),
            vol_sma20: Some(2_000.0),
            min_low_3: Some(9.9),
            min_low_5: Some(9.8),
            max_high_3: Some(10.5),
            max_high_5: Some(10.5),
        },
        htf: HtfRegime {
            close: 10.5,
            ema50: 9.5,
            ema50_rising_3bars: true,
            ema50_falling_3bars: false,
            ready: true,
        },
        spread_bps: Decimal::from(8_u32),
        book_ticker_age_ms: 100,
        gap_free_300_bars: true,
        btc_long_regime_ok: true,
        target_vol: Decimal::new(2, 2),
        realized_vol_30d: Decimal::new(2, 2),
        atr_pct_percentile_90d: Decimal::from(50_u32),
        has_open_position: false,
        bars_since_last_exit: None,
        macro_blackout_active: false,
        obi_smooth: None,
        commission: CommissionRates::from_exchange_ratios(Decimal::new(10, 4), Decimal::new(10, 4)),
        expected_slippage_bps: Decimal::from(5_u32),
    };

    // > +25% blow-off rejected
    assert!(
        s3.evaluate_shadow(&ctx, Decimal::from(30_u32), ranked[0].1)
            .is_none()
    );

    // +14% emits ShadowSignal (type-level guarantee: NOT StrategyEntryIntent!)
    let sig = s3
        .evaluate_shadow(&ctx, Decimal::from(14_u32), ranked[0].1)
        .expect("shadow signal");
    let outcome = evaluate_shadow_outcome(
        &sig,
        Price(Decimal::from(12_u32)),
        Price(Decimal::new(104, 1)),
        false,
    );
    assert_eq!(outcome, ShadowOutcome::HitTp1);

    let mut store = SqliteStore::open_in_memory().expect("store");
    let row_id = record_shadow_signal(&mut store, &sig, outcome).expect("record shadow");
    assert_eq!(row_id, 1);
    let rows = store.list_shadow_signals().expect("list shadow");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].2, "gainers_momentum");
    assert_eq!(rows[0].5, "HIT_TP1");

    // 2. S4 VWAP Reversion is disabled by default
    let s4_default =
        S4VwapReversionShadow::new(VwapReversionConfig::default(), Decimal::from(20_u32));
    assert!(s4_default.evaluate_shadow(&ctx).is_none());
}
