//! Golden reference tests (tolerance 1e-6), warm-up edge cases, anti-look-ahead checks,
//! and O(1) scaling verification for `bot-indicators` (M04).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::cast_precision_loss,
    clippy::too_many_lines
)]

use std::time::Instant;

use bot_indicators::{
    Adx, AdxValue, Atr, Bar, Donchian, DonchianValue, Ema, Rsi, SessionVwap, VolumeSma,
};
use serde::Deserialize;

const TOL: f64 = 1e-6;

#[derive(Debug, Deserialize)]
struct ReferenceFixture {
    bars: Vec<Bar>,
    ema_9: Vec<Option<f64>>,
    ema_21: Vec<Option<f64>>,
    ema_200: Vec<Option<f64>>,
    rsi_14: Vec<Option<f64>>,
    atr_14: Vec<Option<f64>>,
    adx_14: Vec<Option<AdxValue>>,
    donchian_20: Vec<Option<DonchianValue>>,
    session_vwap: Vec<Option<f64>>,
    volume_sma_20: Vec<Option<f64>>,
}

fn load_fixture() -> ReferenceFixture {
    let raw = include_str!("fixtures/reference_indicators.json");
    serde_json::from_str(raw).expect("parse reference_indicators.json")
}

fn assert_opt_close(label: &str, idx: usize, got: Option<f64>, expected: Option<f64>) {
    match (got, expected) {
        (None, None) => {}
        (Some(g), Some(e)) => {
            let diff = (g - e).abs();
            assert!(
                diff <= TOL,
                "{label} mismatch at bar {idx}: got={g}, expected={e}, diff={diff}"
            );
        }
        (g, e) => panic!("{label} readiness mismatch at bar {idx}: got={g:?}, expected={e:?}"),
    }
}

#[test]
fn all_indicators_match_reference_fixture_within_1e6() {
    let fx = load_fixture();
    assert_eq!(fx.bars.len(), 650);

    let mut ema9 = Ema::new(9);
    let mut ema21 = Ema::new(21);
    let mut ema200 = Ema::new(200);
    let mut rsi14 = Rsi::new(14);
    let mut atr14 = Atr::new(14);
    let mut adx14 = Adx::new(14);
    let mut donchian20 = Donchian::new(20);
    let mut vwap = SessionVwap::new();
    let mut vol_sma20 = VolumeSma::new(20);

    for (i, bar) in fx.bars.iter().enumerate() {
        assert_opt_close("ema_9", i, ema9.update(bar), fx.ema_9[i]);
        assert_opt_close("ema_21", i, ema21.update(bar), fx.ema_21[i]);
        assert_opt_close("ema_200", i, ema200.update(bar), fx.ema_200[i]);
        assert_opt_close("rsi_14", i, rsi14.update(bar), fx.rsi_14[i]);
        assert_opt_close("atr_14", i, atr14.update(bar), fx.atr_14[i]);
        assert_opt_close("session_vwap", i, vwap.update(bar), fx.session_vwap[i]);
        assert_opt_close(
            "volume_sma_20",
            i,
            vol_sma20.update(bar),
            fx.volume_sma_20[i],
        );

        let got_adx = adx14.update(bar);
        match (got_adx, fx.adx_14[i]) {
            (None, None) => {}
            (Some(g), Some(e)) => {
                assert!((g.adx - e.adx).abs() <= TOL, "adx mismatch at {i}");
                assert!(
                    (g.plus_di - e.plus_di).abs() <= TOL,
                    "plus_di mismatch at {i}"
                );
                assert!(
                    (g.minus_di - e.minus_di).abs() <= TOL,
                    "minus_di mismatch at {i}"
                );
            }
            (g, e) => panic!("adx_14 mismatch at {i}: got={g:?}, expected={e:?}"),
        }

        let got_dc = donchian20.update(bar);
        match (got_dc, fx.donchian_20[i]) {
            (None, None) => {}
            (Some(g), Some(e)) => {
                assert!((g.upper - e.upper).abs() <= TOL, "donchian upper at {i}");
                assert!((g.lower - e.lower).abs() <= TOL, "donchian lower at {i}");
                assert!((g.mid - e.mid).abs() <= TOL, "donchian mid at {i}");
            }
            (g, e) => panic!("donchian_20 mismatch at {i}: got={g:?}, expected={e:?}"),
        }
    }

    assert!(ema200.is_ready());
    assert!(adx14.is_ready());
    assert!(donchian20.is_ready());
}

#[test]
fn warmup_edge_cases_return_none_before_ready_and_handle_degenerate_inputs() {
    let fx = load_fixture();

    let mut ema200 = Ema::new(200);
    assert_eq!(ema200.warmup_bars(), 600);
    for (i, bar) in fx.bars.iter().take(599).enumerate() {
        assert!(
            ema200.update(bar).is_none(),
            "EMA(200) must return None before bar 600 (at {i})"
        );
        assert!(!ema200.is_ready());
    }
    assert!(ema200.update(&fx.bars[599]).is_some());
    assert!(ema200.is_ready());

    // RSI flat price -> 50.0, strictly rising -> 100.0
    let mut rsi_flat = Rsi::new(14);
    let mut rsi_up = Rsi::new(14);
    for i in 0..20 {
        let flat_bar = Bar {
            open_time_ms: i * 3_600_000,
            close_time_ms: (i + 1) * 3_600_000 - 1,
            open: 100.0,
            high: 100.0,
            low: 100.0,
            close: 100.0,
            volume: 10.0,
            quote_volume: 1000.0,
            trades: 1,
            taker_buy_base: 5.0,
            is_closed: true,
        };
        let up_bar = Bar {
            close: 100.0 + i as f64,
            high: 100.0 + i as f64,
            ..flat_bar.clone()
        };
        let rf = rsi_flat.update(&flat_bar);
        let ru = rsi_up.update(&up_bar);
        if i < 14 {
            assert!(rf.is_none());
            assert!(ru.is_none());
        } else {
            assert_eq!(rf, Some(50.0));
            assert_eq!(ru, Some(100.0));
        }
    }

    // Session VWAP resets at 00:00 UTC and returns None on zero volume
    let mut vwap = SessionVwap::new();
    let zero_vol_bar = Bar {
        open_time_ms: 1_704_067_200_000,
        close_time_ms: 1_704_070_799_999,
        open: 100.0,
        high: 102.0,
        low: 98.0,
        close: 100.0,
        volume: 0.0,
        quote_volume: 0.0,
        trades: 0,
        taker_buy_base: 0.0,
        is_closed: true,
    };
    assert!(vwap.update(&zero_vol_bar).is_none());
}

#[test]
fn anti_look_ahead_mutating_future_bars_never_changes_past_outputs() {
    type SnapshotRow = (
        Option<f64>,
        Option<f64>,
        Option<f64>,
        Option<AdxValue>,
        Option<DonchianValue>,
        Option<f64>,
        Option<f64>,
    );

    let fx = load_fixture();
    let cut = 250;

    let run_all = |bars: &[Bar]| -> Vec<SnapshotRow> {
        let mut ema = Ema::new(21);
        let mut rsi = Rsi::new(14);
        let mut atr = Atr::new(14);
        let mut adx = Adx::new(14);
        let mut dc = Donchian::new(20);
        let mut vwap = SessionVwap::new();
        let mut vsma = VolumeSma::new(20);

        bars.iter()
            .map(|b| {
                (
                    ema.update(b),
                    rsi.update(b),
                    atr.update(b),
                    adx.update(b),
                    dc.update(b),
                    vwap.update(b),
                    vsma.update(b),
                )
            })
            .collect()
    };

    let baseline = run_all(&fx.bars);

    let mut mutated = fx.bars.clone();
    for bar in mutated.iter_mut().skip(cut) {
        bar.high = 999_999.0;
        bar.low = 0.01;
        bar.close = 500_000.0;
        bar.volume = 1_000_000.0;
    }
    let after_mutation = run_all(&mutated);

    assert_eq!(
        &baseline[..cut],
        &after_mutation[..cut],
        "mutating future bars [cut..] MUST NOT alter any past indicator output [0..cut]"
    );
}

#[test]
fn incremental_updates_remain_o1_over_long_history() {
    let mut ema = Ema::new(200);
    let mut rsi = Rsi::new(14);
    let mut atr = Atr::new(14);
    let mut adx = Adx::new(14);
    let mut dc = Donchian::new(20);
    let mut vwap = SessionVwap::new();
    let mut vsma = VolumeSma::new(20);

    let make_bar = |i: i64| Bar {
        open_time_ms: 1_704_067_200_000 + i * 3_600_000,
        close_time_ms: 1_704_067_200_000 + (i + 1) * 3_600_000 - 1,
        open: 40_000.0 + (i % 100) as f64,
        high: 40_100.0 + (i % 100) as f64,
        low: 39_900.0 + (i % 100) as f64,
        close: 40_050.0 + (i % 100) as f64,
        volume: 100.0 + (i % 50) as f64,
        quote_volume: 4_000_000.0,
        trades: 100,
        taker_buy_base: 50.0,
        is_closed: true,
    };

    // Measure first 5,000 bars vs bars 45,000..50,000
    let t0 = Instant::now();
    for i in 0..5_000_i64 {
        let b = make_bar(i);
        let _ = (
            ema.update(&b),
            rsi.update(&b),
            atr.update(&b),
            adx.update(&b),
            dc.update(&b),
            vwap.update(&b),
            vsma.update(&b),
        );
    }
    let early = t0.elapsed();

    for i in 5_000..45_000_i64 {
        let b = make_bar(i);
        let _ = (
            ema.update(&b),
            rsi.update(&b),
            atr.update(&b),
            adx.update(&b),
            dc.update(&b),
            vwap.update(&b),
            vsma.update(&b),
        );
    }

    let t1 = Instant::now();
    for i in 45_000..50_000_i64 {
        let b = make_bar(i);
        let _ = (
            ema.update(&b),
            rsi.update(&b),
            atr.update(&b),
            adx.update(&b),
            dc.update(&b),
            vwap.update(&b),
            vsma.update(&b),
        );
    }
    let late = t1.elapsed();

    // At 10x deeper history, O(1) late window should stay well within 5x of early window
    // (generous bound to avoid CI jitter while still catching any O(N) regression).
    assert!(
        late <= early.saturating_mul(5) + std::time::Duration::from_millis(50),
        "expected O(1) update cost; early={early:?}, late={late:?}"
    );
}
