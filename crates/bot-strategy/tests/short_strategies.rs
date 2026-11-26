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
    // Stop mirror: dc_mid (2975).min(close + 2*ATR = 2910).max(close + ATR = 2875) = 2910
    assert_eq!(intent.stop.0, dec("2910"));
}

fn valid_s1_short_ctx() -> StrategyCtx {
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
    bearish_ctx(ind, 2930.0, 2955.0, 2920.0)
}

fn valid_s2_short_ctx() -> StrategyCtx {
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
    bearish_ctx(ind, 2840.0, 2860.0, 2830.0)
}

#[test]
fn s1_and_s2_short_enforce_all_quality_gates_and_vol_cap() {
    type CtxMutator = fn(&mut StrategyCtx);
    let mut s1 =
        S1TrendPullback::new(TrendPullbackConfig::default(), dec("15")).with_allow_short(true);
    let mut s2 =
        S2Breakout::new(BreakoutConfig::default(), dec("15"), dec("3")).with_allow_short(true);

    let cases: [(&str, CtxMutator); 8] = [
        ("closed_bar", |c| c.bar.is_closed = false),
        ("gap_free_300", |c| c.gap_free_300_bars = false),
        ("spread_and_freshness", |c| c.book_ticker_age_ms = 500),
        ("spread_and_freshness", |c| c.spread_bps = dec("20")),
        ("cost_gate", |c| {
            c.commission = CommissionRates {
                maker_bps: dec("100"),
                taker_bps: dec("100"),
            };
        }),
        ("position_and_cooldown", |c| c.has_open_position = true),
        ("position_and_cooldown", |c| {
            c.bars_since_last_exit = Some(2);
        }),
        ("macro_blackout", |c| c.macro_blackout_active = true),
    ];

    for (expected_gate, mutate) in &cases {
        let mut c1 = valid_s1_short_ctx();
        mutate(&mut c1);
        let d1 = s1.evaluate(&c1);
        assert!(
            d1.intent.is_none(),
            "S1 Short must reject when {expected_gate} fails"
        );
        assert!(
            d1.gate_logs
                .iter()
                .any(|g| g.gate == *expected_gate && !g.passed),
            "S1 Short gate_logs must include failed {expected_gate}"
        );

        let mut c2 = valid_s2_short_ctx();
        mutate(&mut c2);
        let d2 = s2.evaluate(&c2);
        assert!(
            d2.intent.is_none(),
            "S2 Short must reject when {expected_gate} fails"
        );
        assert!(
            d2.gate_logs
                .iter()
                .any(|g| g.gate == *expected_gate && !g.passed),
            "S2 Short gate_logs must include failed {expected_gate}"
        );
    }

    // ATR% > 85 vol cap must cap size_mult <= 0.6 on both S1 and S2 Short.
    let mut c1 = valid_s1_short_ctx();
    c1.target_vol = dec("80");
    c1.realized_vol_30d = dec("40");
    c1.atr_pct_percentile_90d = dec("90");
    let i1 = s1.evaluate(&c1).intent.expect("S1 Short passes with cap");
    assert_eq!(i1.size_mult, dec("0.6"));

    let mut c2 = valid_s2_short_ctx();
    c2.target_vol = dec("80");
    c2.realized_vol_30d = dec("40");
    c2.atr_pct_percentile_90d = dec("90");
    let i2 = s2.evaluate(&c2).intent.expect("S2 Short passes with cap");
    assert_eq!(i2.size_mult, dec("0.6"));
}

#[test]
fn s1_and_s2_short_reject_non_positive_tp1_or_tp2() {
    let mut s1 =
        S1TrendPullback::new(TrendPullbackConfig::default(), dec("15")).with_allow_short(true);
    let mut c1 = valid_s1_short_ctx();
    // close=100, stop=max(140, 100+15)+1=141 => r_dist=41 => tp2=100-123=-23 <= 0
    c1.bar.close = 100.0;
    c1.indicators.ema9 = Some(100.0);
    c1.indicators.ema21 = Some(105.0);
    c1.indicators.ema55 = Some(115.0);
    c1.indicators.ema200 = Some(130.0);
    c1.indicators.atr14 = Some(10.0);
    c1.indicators.max_high_3 = Some(106.0);
    c1.indicators.max_high_5 = Some(140.0);
    let d1 = s1.evaluate(&c1);
    assert!(d1.intent.is_none(), "S1 Short must reject non-positive TP2");
    assert!(
        d1.gate_logs
            .iter()
            .any(|g| g.gate == "target_geometry" && !g.passed)
    );

    let mut s2 =
        S2Breakout::new(BreakoutConfig::default(), dec("15"), dec("3")).with_allow_short(true);
    let mut c2 = valid_s2_short_ctx();
    // close=100, atr=20, dc_mid=140 => stop=min(140, 140).max(120)=140 => r_dist=40 => tp2=100-120=-20 <= 0
    c2.bar.close = 100.0;
    c2.indicators.ema21 = Some(110.0);
    c2.indicators.atr14 = Some(20.0);
    c2.indicators.donchian_lower = Some(105.0);
    c2.indicators.donchian_mid = Some(140.0);
    let d2 = s2.evaluate(&c2);
    assert!(d2.intent.is_none(), "S2 Short must reject non-positive TP2");
    assert!(
        d2.gate_logs
            .iter()
            .any(|g| g.gate == "target_geometry" && !g.passed)
    );
}

#[test]
fn btc_regime_gate_respects_direction() {
    let mut s1 =
        S1TrendPullback::new(TrendPullbackConfig::default(), dec("15")).with_allow_short(true);

    // Altcoin Short with bearish BTC (btc_long_regime_ok=false) passes.
    let c_bear = valid_s1_short_ctx();
    assert!(s1.evaluate(&c_bear).intent.is_some());

    // Altcoin Short with bullish BTC (btc_long_regime_ok=true) is blocked.
    let mut c_bull = valid_s1_short_ctx();
    c_bull.btc_long_regime_ok = true;
    let d_bull = s1.evaluate(&c_bull);
    assert!(d_bull.intent.is_none());
    assert!(
        d_bull
            .gate_logs
            .iter()
            .any(|g| g.gate == "btc_regime" && !g.passed)
    );

    // BTCUSDT itself is exempt from altcoin BTC regime gate on Short.
    let mut c_btc = valid_s1_short_ctx();
    c_btc.symbol = Symbol("BTCUSDT".to_owned());
    c_btc.btc_long_regime_ok = true;
    assert!(s1.evaluate(&c_btc).intent.is_some());
}

#[test]
fn s1_short_respects_config_ema9_volume_and_obi_filter() {
    // 1. Custom config: adx_min=30 blocks adx=26; custom tp1_r/tp2_r are applied.
    let cfg = TrendPullbackConfig {
        adx_min: dec("30"),
        ..Default::default()
    };
    let mut s1_high_adx = S1TrendPullback::new(cfg, dec("15")).with_allow_short(true);
    let d_adx = s1_high_adx.evaluate(&valid_s1_short_ctx());
    assert!(d_adx.intent.is_none());
    assert!(
        d_adx
            .gate_logs
            .iter()
            .any(|g| g.gate == "momentum" && !g.passed)
    );

    let cfg_tp = TrendPullbackConfig {
        tp1_r: dec("2.0"),
        tp2_r: dec("4.0"),
        ..Default::default()
    };
    let mut s1_tp = S1TrendPullback::new(cfg_tp, dec("15")).with_allow_short(true);
    let i_tp = s1_tp
        .evaluate(&valid_s1_short_ctx())
        .intent
        .expect("passes with custom TP R");
    let r = i_tp.stop.0 - i_tp.entry_ref.0;
    assert_eq!(i_tp.tp1.0, i_tp.entry_ref.0 - r * dec("2.0"));
    assert_eq!(i_tp.tp2.expect("tp2").0, i_tp.entry_ref.0 - r * dec("4.0"));

    // 2. EMA9 overextension: close < ema9 - 0.5*ATR rejected.
    let mut s1 =
        S1TrendPullback::new(TrendPullbackConfig::default(), dec("15")).with_allow_short(true);
    let mut c_ema9 = valid_s1_short_ctx();
    c_ema9.indicators.ema9 = Some(2960.0); // 2930 < 2960 - 20 = 2940
    let d_ema9 = s1.evaluate(&c_ema9);
    assert!(d_ema9.intent.is_none());
    assert!(
        d_ema9
            .gate_logs
            .iter()
            .any(|g| g.gate == "pullback" && !g.passed)
    );

    // 3. Volume < 0.8 * vol_sma20 rejected.
    let mut c_vol = valid_s1_short_ctx();
    c_vol.bar.volume = 1000.0; // < 0.8 * 2000 = 1600
    let d_vol = s1.evaluate(&c_vol);
    assert!(d_vol.intent.is_none());
    assert!(
        d_vol
            .gate_logs
            .iter()
            .any(|g| g.gate == "volume" && !g.passed)
    );

    // 4. Optional OBI filter mirror: requires OBI_smooth <= -0.05.
    let mut s1_obi = S1TrendPullback::new(TrendPullbackConfig::default(), dec("15"))
        .with_allow_short(true)
        .with_obi_filter(true);
    let mut c_obi = valid_s1_short_ctx();
    c_obi.obi_smooth = Some(0.02);
    assert!(s1_obi.evaluate(&c_obi).intent.is_none());
    c_obi.obi_smooth = Some(-0.08);
    assert!(s1_obi.evaluate(&c_obi).intent.is_some());
}

#[test]
fn s2_short_respects_config_adx_slope_anti_overextension_and_dc_mid_stop() {
    // 1. config.min_volume_mult=3.0 rejects vol=5000 when vol_sma20=2000 (need >= 6000).
    let cfg = BreakoutConfig {
        min_volume_mult: dec("3.0"),
        ..Default::default()
    };
    let mut s2_vol = S2Breakout::new(cfg, dec("15"), dec("3")).with_allow_short(true);
    let d_vol = s2_vol.evaluate(&valid_s2_short_ctx());
    assert!(d_vol.intent.is_none());
    assert!(
        d_vol
            .gate_logs
            .iter()
            .any(|g| g.gate == "volume" && !g.passed)
    );

    let mut s2 =
        S2Breakout::new(BreakoutConfig::default(), dec("15"), dec("3")).with_allow_short(true);

    // 2. Falling ADX (adx <= prev_adx) rejected.
    let mut c_adx = valid_s2_short_ctx();
    c_adx.indicators.adx14 = Some(20.0);
    c_adx.indicators.prev_adx14 = Some(22.0);
    let d_adx = s2.evaluate(&c_adx);
    assert!(d_adx.intent.is_none());
    assert!(
        d_adx
            .gate_logs
            .iter()
            .any(|g| g.gate == "momentum" && !g.passed)
    );

    // 3. Anti-overextension: ema21 - close > 2.5 * atr rejected.
    let mut c_ext = valid_s2_short_ctx();
    c_ext.indicators.ema21 = Some(2940.0); // 2940 - 2840 = 100 > 2.5 * 35 = 87.5
    let d_ext = s2.evaluate(&c_ext);
    assert!(d_ext.intent.is_none());
    assert!(
        d_ext
            .gate_logs
            .iter()
            .any(|g| g.gate == "anti_overextension" && !g.passed)
    );

    // 4. dc_mid stop mirror when dc_mid is between close + ATR and close + 2*ATR.
    let mut c_mid = valid_s2_short_ctx();
    c_mid.indicators.donchian_mid = Some(2890.0); // in [2875, 2910] -> stop = 2890
    let i_mid = s2.evaluate(&c_mid).intent.expect("passes with dc_mid stop");
    assert_eq!(i_mid.stop.0, dec("2890"));
}
