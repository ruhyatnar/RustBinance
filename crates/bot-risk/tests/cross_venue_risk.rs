#![allow(clippy::expect_used)]
use bot_risk::{
    BTC_GUARD_MIN_ACTIVE_MS, BtcSpikeGuard, BtcSpikeSample, CrossVenueBudget, CrossVenuePosition,
    CrossVenueSameSymbolPolicy,
};
use bot_types::{Direction, Symbol, Venue};
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str_exact(s).expect("valid decimal")
}

#[test]
fn combined_equity_and_effective_allocation() {
    assert_eq!(
        CrossVenueBudget::combined_equity_usd(dec("6000"), dec("4000")),
        dec("10000")
    );
    assert_eq!(
        CrossVenueBudget::effective_venue_equity(dec("6000"), dec("2500")),
        dec("2500")
    );
    assert_eq!(
        CrossVenueBudget::effective_venue_equity(dec("2000"), dec("2500")),
        dec("2000")
    );
    // Zero allocation means unlimited (use full venue equity).
    assert_eq!(
        CrossVenueBudget::effective_venue_equity(dec("2000"), Decimal::ZERO),
        dec("2000")
    );
}

#[test]
fn combined_portfolio_heat_includes_spot_and_futures_positions() {
    let btc = Symbol("BTCUSDT".to_owned());
    let eth = Symbol("ETHUSDT".to_owned());
    let positions = vec![
        CrossVenuePosition {
            venue: Venue::Spot,
            symbol: btc,
            direction: Direction::Long,
            risk_usd: dec("100"),
        },
        CrossVenuePosition {
            venue: Venue::UsdsFutures,
            symbol: eth,
            direction: Direction::Short,
            risk_usd: dec("150"),
        },
    ];
    let heat = CrossVenueBudget::combined_portfolio_heat_pct(&positions, dec("10000"));
    assert_eq!(heat, dec("2.5"));
}

#[test]
fn block_opposite_rejects_cross_venue_opposite_direction_same_symbol() {
    let btc = Symbol("BTCUSDT".to_owned());
    let existing = vec![CrossVenuePosition {
        venue: Venue::Spot,
        symbol: btc.clone(),
        direction: Direction::Long,
        risk_usd: dec("50"),
    }];

    // Opposite direction on Futures for BTCUSDT -> rejected.
    assert!(
        CrossVenueBudget::check_same_symbol(
            CrossVenueSameSymbolPolicy::BlockOpposite,
            &existing,
            Venue::UsdsFutures,
            &btc,
            Direction::Short,
        )
        .is_err()
    );

    // Same direction on Futures for BTCUSDT -> allowed.
    assert!(
        CrossVenueBudget::check_same_symbol(
            CrossVenueSameSymbolPolicy::BlockOpposite,
            &existing,
            Venue::UsdsFutures,
            &btc,
            Direction::Long,
        )
        .is_ok()
    );

    // Allow policy permits opposite direction.
    assert!(
        CrossVenueBudget::check_same_symbol(
            CrossVenueSameSymbolPolicy::Allow,
            &existing,
            Venue::UsdsFutures,
            &btc,
            Direction::Short,
        )
        .is_ok()
    );
}

#[test]
fn btc_spike_guard_blocks_short_on_spike_and_releases_with_hysteresis() {
    let mut guard = BtcSpikeGuard::new(dec("1.5"), dec("3.0"));
    assert!(!guard.is_active());

    let trig = guard.update(BtcSpikeSample {
        now_ms: 1_000_000,
        price: dec("61000"),
        rise_15m_pct: dec("1.8"),
        rise_60m_pct: dec("2.0"),
        reference_low_price: dec("60000"),
        atr_1m_pct: dec("0.4"),
        median_atr_24h_pct: dec("0.2"),
    });
    assert!(trig.active);
    assert!(trig.just_triggered);
    assert!(guard.is_active());

    // Before 30m minimum active window, stays active even if price retraces.
    let mid = guard.update(BtcSpikeSample {
        now_ms: 1_000_000 + BTC_GUARD_MIN_ACTIVE_MS - 1_000,
        price: dec("60400"),
        rise_15m_pct: dec("0.2"),
        rise_60m_pct: dec("0.5"),
        reference_low_price: dec("60000"),
        atr_1m_pct: dec("0.15"),
        median_atr_24h_pct: dec("0.2"),
    });
    assert!(mid.active);

    // After 30m with >= 50% retrace of spike and normalized ATR, releases.
    let rel = guard.update(BtcSpikeSample {
        now_ms: 1_000_000 + BTC_GUARD_MIN_ACTIVE_MS + 1_000,
        price: dec("60400"),
        rise_15m_pct: dec("0.1"),
        rise_60m_pct: dec("0.4"),
        reference_low_price: dec("60000"),
        atr_1m_pct: dec("0.15"),
        median_atr_24h_pct: dec("0.2"),
    });
    assert!(!rel.active);
    assert!(rel.just_released);
    assert!(!guard.is_active());
}
