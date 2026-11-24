#![allow(clippy::expect_used, clippy::large_types_passed_by_value)]
use bot_config::{BreakoutConfig, TrendPullbackConfig};
use bot_strategy::{
    CommissionRates, HtfRegime, IndicatorSnapshot, S1TrendPullback, S2Breakout, Strategy,
    StrategyCtx,
};
use bot_types::{Direction, Kline, ProtectionScheme, Symbol, Venue, VenueCaps};
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str_exact(s).expect("valid decimal")
}

fn spot_caps() -> VenueCaps {
    VenueCaps {
        can_short: false,
        max_leverage: 1,
        has_funding: false,
        protection: ProtectionScheme::DualOco,
    }
}

fn futures_caps() -> VenueCaps {
    VenueCaps {
        can_short: true,
        max_leverage: 3,
        has_funding: true,
        protection: ProtectionScheme::AlgoReduceOnly,
    }
}

fn bearish_ctx(
    indicators: IndicatorSnapshot,
    bar_close: f64,
    bar_high: f64,
    bar_low: f64,
) -> StrategyCtx {
    StrategyCtx {
        venue: Venue::UsdsFutures,
        symbol: Symbol("ETHUSDT".to_owned()),
        venue_caps: futures_caps(),
        bar: Kline {
            open_time_ms: 1_700_000_000_000,
            close_time_ms: 1_700_003_599_999,
            open: bar_close + 5.0,
            high: bar_high,
            low: bar_low,
            close: bar_close,
            volume: 5000.0,
            quote_volume: 14_000_000.0,
            taker_buy_base: 2000.0,
            trades: 1000,
            is_closed: true,
        },
        htf: HtfRegime {
            close: 2800.0,
            ema50: 3000.0,
            ema50_rising_3bars: false,
            ema50_falling_3bars: true,
            ready: true,
        },
        indicators,
        spread_bps: dec("2"),
        book_ticker_age_ms: 100,
        gap_free_300_bars: true,
        btc_long_regime_ok: false,
        realized_vol_30d: dec("40"),
        target_vol: dec("40"),
        atr_pct_percentile_90d: dec("50"),
        has_open_position: false,
        bars_since_last_exit: None,
        macro_blackout_active: false,
        commission: CommissionRates {
            maker_bps: dec("2"),
            taker_bps: dec("5"),
        },
        expected_slippage_bps: dec("2"),
        obi_smooth: None,
    }
}

#[test]
fn supports_matrix_respects_venue_caps_and_allow_short() {
    let s1_no_short = S1TrendPullback::new(TrendPullbackConfig::default(), dec("15"));
    let s1_short =
        S1TrendPullback::new(TrendPullbackConfig::default(), dec("15")).with_allow_short(true);
    let s2_short =
        S2Breakout::new(BreakoutConfig::default(), dec("15"), dec("3")).with_allow_short(true);

    // Spot never supports Short.
    assert!(s1_short.supports(&spot_caps(), Direction::Long));
    assert!(!s1_short.supports(&spot_caps(), Direction::Short));
    assert!(!s2_short.supports(&spot_caps(), Direction::Short));

    // Futures supports Short only when allow_short is enabled.
    assert!(!s1_no_short.supports(&futures_caps(), Direction::Short));
    assert!(s1_short.supports(&futures_caps(), Direction::Short));
    assert!(s2_short.supports(&futures_caps(), Direction::Short));
}

#[test]
fn s1_emits_short_signal_on_bearish_pullback_mirror() {
    let mut s1 =
        S1TrendPullback::new(TrendPullbackConfig::default(), dec("15")).with_allow_short(true);
    let ind = IndicatorSnapshot {
        ema9: Some(2900.0),
        ema21: Some(2950.0),
        ema55: Some(3050.0),
        ema200: Some(3200.0),
        rsi14: Some(46.0),
        prev_rsi14: Some(50.0),
        atr14: Some(40.0),
        adx14: Some(26.0),
        prev_adx14: Some(24.0),
        plus_di: Some(15.0),
        minus_di: Some(30.0),
        donchian_upper: Some(3100.0),
        donchian_lower: Some(2850.0),
        donchian_mid: Some(2975.0),
        session_vwap: Some(2960.0),
        vol_sma20: Some(2000.0),
        min_low_3: Some(2900.0),
        min_low_5: Some(2880.0),
        max_high_3: Some(2955.0),
        max_high_5: Some(2980.0),
    };
    let ctx = bearish_ctx(ind, 2930.0, 2955.0, 2920.0);
    let decision = s1.evaluate(&ctx);
    let intent = decision.intent.expect("S1 Short signal must be emitted");
    assert_eq!(intent.direction, Direction::Short);
    assert!(
        intent.stop.0 > intent.entry_ref.0,
        "Short stop must be above entry"
    );
    assert!(
        intent.tp1.0 < intent.entry_ref.0,
        "Short TP1 must be below entry"
    );
}

#[test]
fn s2_emits_short_signal_on_donchian_lower_breakdown() {
    let mut s2 =
        S2Breakout::new(BreakoutConfig::default(), dec("15"), dec("3")).with_allow_short(true);
    let ind = IndicatorSnapshot {
        ema9: Some(2860.0),
        ema21: Some(2900.0),
        ema55: Some(3000.0),
        ema200: Some(3150.0),
        rsi14: Some(38.0),
        prev_rsi14: Some(42.0),
        atr14: Some(35.0),
        adx14: Some(24.0),
        prev_adx14: Some(21.0),
        plus_di: Some(14.0),
        minus_di: Some(28.0),
        donchian_upper: Some(3100.0),
        donchian_lower: Some(2850.0),
        donchian_mid: Some(2975.0),
        session_vwap: Some(2950.0),
        vol_sma20: Some(2000.0),
        min_low_3: Some(2830.0),
        min_low_5: Some(2830.0),
        max_high_3: Some(2900.0),
        max_high_5: Some(2920.0),
    };
    let ctx = bearish_ctx(ind, 2840.0, 2860.0, 2830.0);
    let decision = s2.evaluate(&ctx);
    let intent = decision.intent.expect("S2 Short signal must be emitted");
    assert_eq!(intent.direction, Direction::Short);
    assert!(
        intent.stop.0 > intent.entry_ref.0,
        "Short stop must be above entry"
    );
    assert!(
        intent.tp1.0 < intent.entry_ref.0,
        "Short TP1 must be below entry"
    );
}
