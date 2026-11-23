//! Golden sizing test (§7.3), property tests, §7.4 mode table tests, BTC Crash Guard
//! hysteresis tests, and Gatekeeper / tighten-only tests for `bot-risk` (M06).

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::str::FromStr;

use bot_config::{ProfitLockMode, RiskConfig, SizingMode};
use bot_risk::{
    BTC_GUARD_MIN_ACTIVE_MS, BTC_GUARD_NO_NEW_LOW_RELEASE_MS, BTC_GUARD_RETRIGGER_COOLDOWN_MS,
    BtcCrashGuard, BtcMarketSample, CombinedEquityCounters, EngineModeMachine, EntryIntent,
    GateDecision, Gatekeeper, GatekeeperContext, KellyInput, ModeTrigger, ModeTriggerKind,
    PAUSE_STABLE_RELEASE_MS, REDUCE_ONLY_HEALTHY_RELEASE_MS, RejectReason, SpotSizingInput,
    ratchet_stop_monotonic, size_spot_position,
};
use bot_types::{Direction, EngineMode, InstrumentRules, Price, StrategyId, Symbol, Venue};
use proptest::prelude::*;
use rust_decimal::Decimal;

fn sample_rules() -> InstrumentRules {
    InstrumentRules {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        tick_size: Decimal::new(1, 2),
        step_size: Decimal::new(1, 1), // 0.1
        min_qty: Decimal::new(1, 1),
        max_qty: Decimal::from(10_000_u32),
        min_notional: Decimal::from(10_u32),
        status_trading: true,
        base_precision: 8,
        quote_precision: 8,
    }
}

#[test]
fn golden_spec_7_3_spot_sizing_example() {
    // §7.3 example: equity 1,000 USDT, risk 0.5% = 5 USDT, entry 100, stop 98
    // -> qty_raw = 2.5 -> notional 250 (25% > 20% cap) -> capped to 200 USDT -> qty = 2.0
    // -> actual risk = 4.0 USDT (0.4%).
    let rules = sample_rules();
    let out = size_spot_position(&SpotSizingInput {
        equity: Decimal::from(1_000_u32),
        free_quote: Decimal::from(1_000_u32),
        entry_price: Price(Decimal::from(100_u32)),
        stop_price: Price(Decimal::from(98_u32)),
        rules: &rules,
        risk_per_trade_pct: Decimal::new(5, 1),
        size_mult: Decimal::ONE,
        scanner_mult: Decimal::ONE,
        profit_lock_mult: Decimal::ONE,
        sizing_mode: SizingMode::FixedRiskPct,
        fixed_notional_usd: None,
        max_notional_per_position_pct: Decimal::from(20_u32),
        current_portfolio_heat_pct: Decimal::ZERO,
        portfolio_heat_max_pct: Decimal::new(20, 1),
        current_total_exposure_usd: Decimal::ZERO,
        max_total_exposure_pct: Decimal::from(60_u32),
    })
    .expect("golden sizing must succeed");

    assert_eq!(out.qty.as_decimal(), Decimal::from(2_u32));
    assert_eq!(out.notional_usd, Decimal::from(200_u32));
    assert_eq!(out.actual_risk_usd, Decimal::from(4_u32));
    assert_eq!(out.actual_risk_pct, Decimal::new(4, 1));
    assert_eq!(out.heat_after_pct, Decimal::new(4, 1));
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    #[test]
    fn prop_sizing_never_exceeds_risk_heat_exposure_and_always_step_multiple(
        equity_cents in 50_000_i64..10_000_000_i64,
        entry_cents in 1_000_i64..500_000_i64,
        stop_bps in 40_i64..750_i64, // 0.4%..7.5%
        risk_tenths in 1_i64..=10_i64, // 0.1%..1.0%
        heat_now_tenths in 0_i64..=30_i64, // 0.0%..3.0%
        free_ratio_pct in 10_i64..=100_i64,
    ) {
        let equity = Decimal::new(equity_cents, 2);
        let free_quote = equity * Decimal::new(free_ratio_pct, 2);
        let entry = Decimal::new(entry_cents, 2);
        let stop = entry * (Decimal::ONE - Decimal::new(stop_bps, 4));
        let risk_pct = Decimal::new(risk_tenths, 1);
        let heat_now = Decimal::new(heat_now_tenths, 1);
        let heat_max = Decimal::new(20, 1); // 2.0%
        let step = Decimal::from_str("0.001").unwrap();

        let rules = InstrumentRules {
            venue: Venue::Spot,
            symbol: Symbol("ETHUSDT".into()),
            tick_size: Decimal::new(1, 2),
            step_size: step,
            min_qty: step,
            max_qty: Decimal::from(100_000_u32),
            min_notional: Decimal::from(5_u32),
            status_trading: true,
            base_precision: 8,
            quote_precision: 8,
        };

        let input = SpotSizingInput {
            equity,
            free_quote,
            entry_price: Price(entry),
            stop_price: Price(stop),
            rules: &rules,
            risk_per_trade_pct: risk_pct,
            size_mult: Decimal::ONE,
            scanner_mult: Decimal::ONE,
            profit_lock_mult: Decimal::ONE,
            sizing_mode: SizingMode::FixedRiskPct,
            fixed_notional_usd: None,
            max_notional_per_position_pct: Decimal::from(20_u32),
            current_portfolio_heat_pct: heat_now,
            portfolio_heat_max_pct: heat_max,
            current_total_exposure_usd: Decimal::ZERO,
            max_total_exposure_pct: Decimal::from(60_u32),
        };

        if let Ok(out) = size_spot_position(&input) {
            let risk_cap = equity * risk_pct / Decimal::from(100_u32);
            prop_assert!(out.actual_risk_usd <= risk_cap, "risk exceeded cap");
            prop_assert!(out.heat_after_pct <= heat_max, "heat exceeded max");
            prop_assert_eq!(out.qty.as_decimal() % step, Decimal::ZERO, "qty not step multiple");
            prop_assert!(out.qty.as_decimal() >= rules.min_qty);
            prop_assert!(out.notional_usd >= rules.min_notional);
            prop_assert!(out.notional_usd <= equity * Decimal::new(20, 2));
        }
    }

    #[test]
    fn prop_ratchet_stop_is_strictly_monotonic(
        a in 100_i64..1_000_000_i64,
        b in 100_i64..1_000_000_i64,
    ) {
        let cur = Price(Decimal::new(a, 2));
        let cand = Price(Decimal::new(b, 2));

        let next_long = ratchet_stop_monotonic(Direction::Long, cur, cand);
        prop_assert!(next_long >= cur);

        let next_short = ratchet_stop_monotonic(Direction::Short, cur, cand);
        prop_assert!(next_short <= cur);
    }
}

#[test]
fn engine_mode_table_7_4_all_transitions_and_killed_latch() {
    let mut m = EngineModeMachine::new(0);
    assert_eq!(m.mode(), EngineMode::Starting);
    assert!(!m.allows_new_entries());

    assert_eq!(m.apply(ModeTrigger::SyncStarted, 100), EngineMode::Syncing);
    assert_eq!(m.apply(ModeTrigger::SyncComplete, 200), EngineMode::Ready);
    assert!(m.allows_new_entries());

    // Ready -> Paused (reconnect) -> auto-exit after >= 3s stable
    assert_eq!(
        m.apply(
            ModeTrigger::ReconnectOrSyncNeeded {
                reason: "ws drop".into()
            },
            1_000
        ),
        EngineMode::Paused
    );
    assert_eq!(
        m.apply(ModeTrigger::StableTick { now_ms: 2_000 }, 2_000),
        EngineMode::Paused
    );
    assert_eq!(
        m.apply(
            ModeTrigger::StableTick {
                now_ms: 2_000 + PAUSE_STABLE_RELEASE_MS - 1
            },
            2_000 + PAUSE_STABLE_RELEASE_MS - 1
        ),
        EngineMode::Paused
    );
    assert_eq!(
        m.apply(
            ModeTrigger::StableTick {
                now_ms: 2_000 + PAUSE_STABLE_RELEASE_MS
            },
            2_000 + PAUSE_STABLE_RELEASE_MS
        ),
        EngineMode::Ready
    );

    // Ready -> ReduceOnly (stale > 30s) -> auto-exit after >= 60s healthy
    assert_eq!(
        m.apply(ModeTrigger::DataStaleOver30s, 10_000),
        EngineMode::ReduceOnly
    );
    assert!(!m.allows_new_entries());
    assert!(m.allows_position_management());
    assert_eq!(
        m.apply(ModeTrigger::HealthyTick { now_ms: 11_000 }, 11_000),
        EngineMode::ReduceOnly
    );
    assert_eq!(
        m.apply(
            ModeTrigger::HealthyTick {
                now_ms: 11_000 + REDUCE_ONLY_HEALTHY_RELEASE_MS
            },
            11_000 + REDUCE_ONLY_HEALTHY_RELEASE_MS
        ),
        EngineMode::Ready
    );

    // DailyLossHit -> Halted -> cleared by UtcDayRollover
    assert_eq!(
        m.apply(ModeTrigger::DailyLossHit, 100_000),
        EngineMode::Halted
    );
    assert_eq!(
        m.apply(ModeTrigger::UtcDayRollover, 200_000),
        EngineMode::Ready
    );

    // ConsecutiveOrderErrors(3) -> Halted -> NOT cleared by UtcDayRollover; requires manual resume
    assert_eq!(
        m.apply(ModeTrigger::ConsecutiveOrderErrors { count: 2 }, 201_000),
        EngineMode::Ready
    );
    assert_eq!(
        m.apply(ModeTrigger::ConsecutiveOrderErrors { count: 3 }, 202_000),
        EngineMode::Halted
    );
    assert_eq!(
        m.apply(ModeTrigger::UtcDayRollover, 300_000),
        EngineMode::Halted
    );
    assert_eq!(
        m.apply(ModeTrigger::ManualResumeFromHalt, 301_000),
        EngineMode::Ready
    );

    // DrawdownKillHit -> Killed -> ignores all auto/manual-resume triggers; ONLY confirmed reset exits!
    assert_eq!(
        m.apply(ModeTrigger::DrawdownKillHit, 400_000),
        EngineMode::Killed
    );
    assert_eq!(
        m.apply(ModeTrigger::UtcDayRollover, 500_000),
        EngineMode::Killed
    );
    assert_eq!(
        m.apply(ModeTrigger::ManualResumeFromHalt, 501_000),
        EngineMode::Killed
    );
    assert_eq!(
        m.apply(ModeTrigger::ManualResetKill { confirmed: false }, 502_000),
        EngineMode::Killed
    );
    assert_eq!(
        m.apply(ModeTrigger::ManualResetKill { confirmed: true }, 503_000),
        EngineMode::Paused
    );
}

#[test]
fn btc_crash_guard_hysteresis_no_flapping_and_release() {
    let mut guard = BtcCrashGuard::new(Decimal::new(15, 1), Decimal::new(30, 1));
    let t0 = 1_700_000_000_000_i64;
    let median_atr = Decimal::new(10, 4); // 0.10%

    // 1. Trigger on 1.8% 15m drop from 60,000 to 58,920
    let a1 = guard.update(BtcMarketSample {
        now_ms: t0,
        price: Decimal::from(58_920_u32),
        drop_15m_pct: Decimal::new(18, 1),
        drop_60m_pct: Decimal::new(20, 1),
        reference_high_price: Decimal::from(60_000_u32),
        atr_1m_pct: Decimal::new(25, 4), // elevated
        median_atr_24h_pct: median_atr,
    });
    assert!(a1.active && a1.just_triggered && a1.tighten_profitable_stops_to_be);

    // 2. At 20m (< 30m): even with full recovery and low ATR, must stay active!
    let a2 = guard.update(BtcMarketSample {
        now_ms: t0 + 20 * 60 * 1000,
        price: Decimal::from(59_800_u32),
        drop_15m_pct: Decimal::ZERO,
        drop_60m_pct: Decimal::ZERO,
        reference_high_price: Decimal::from(60_000_u32),
        atr_1m_pct: Decimal::new(10, 4),
        median_atr_24h_pct: median_atr,
    });
    assert!(a2.active && !a2.just_released);

    // 3. At 35m (>= 30m) with 60% recovery, but ATR still >= 1.5x median -> must stay active!
    let a3 = guard.update(BtcMarketSample {
        now_ms: t0 + BTC_GUARD_MIN_ACTIVE_MS + 5 * 60 * 1000,
        price: Decimal::from(59_600_u32),
        drop_15m_pct: Decimal::ZERO,
        drop_60m_pct: Decimal::ZERO,
        reference_high_price: Decimal::from(60_000_u32),
        atr_1m_pct: Decimal::new(16, 4), // 1.6x median
        median_atr_24h_pct: median_atr,
    });
    assert!(a3.active);

    // 4. At 36m with normalized ATR (< 1.5x) and >= 50% recovery -> releases!
    let release_t = t0 + BTC_GUARD_MIN_ACTIVE_MS + 6 * 60 * 1000;
    let a4 = guard.update(BtcMarketSample {
        now_ms: release_t,
        price: Decimal::from(59_500_u32), // > 58,920 + 540 = 59,460
        drop_15m_pct: Decimal::ZERO,
        drop_60m_pct: Decimal::ZERO,
        reference_high_price: Decimal::from(60_000_u32),
        atr_1m_pct: Decimal::new(12, 4), // 1.2x median
        median_atr_24h_pct: median_atr,
    });
    assert!(!a4.active && a4.just_released);

    // 5. Anti-flapping: another drop 5 minutes later (within 15m cooldown) does NOT re-trigger!
    let a5 = guard.update(BtcMarketSample {
        now_ms: release_t + 5 * 60 * 1000,
        price: Decimal::from(58_500_u32),
        drop_15m_pct: Decimal::new(20, 1),
        drop_60m_pct: Decimal::new(20, 1),
        reference_high_price: Decimal::from(59_500_u32),
        atr_1m_pct: Decimal::new(20, 4),
        median_atr_24h_pct: median_atr,
    });
    assert!(!a5.active && !a5.just_triggered);

    // 6. After 15m cooldown expires, a 60m drop >= 3% triggers again, and releases after 60m without new low!
    let retrigger_t = release_t + BTC_GUARD_RETRIGGER_COOLDOWN_MS;
    let a6 = guard.update(BtcMarketSample {
        now_ms: retrigger_t,
        price: Decimal::from(57_000_u32),
        drop_15m_pct: Decimal::new(5, 1),
        drop_60m_pct: Decimal::new(32, 1),
        reference_high_price: Decimal::from(59_000_u32),
        atr_1m_pct: Decimal::new(20, 4),
        median_atr_24h_pct: median_atr,
    });
    assert!(a6.active && a6.just_triggered);

    let a7 = guard.update(BtcMarketSample {
        now_ms: retrigger_t + BTC_GUARD_NO_NEW_LOW_RELEASE_MS,
        price: Decimal::from(57_100_u32), // < 50% recovery, but 60m without new low!
        drop_15m_pct: Decimal::ZERO,
        drop_60m_pct: Decimal::ZERO,
        reference_high_price: Decimal::from(59_000_u32),
        atr_1m_pct: Decimal::new(11, 4),
        median_atr_24h_pct: median_atr,
    });
    assert!(!a7.active && a7.just_released);
}

#[test]
fn gatekeeper_checks_kelly_autodisable_tighten_only_and_equity_counters() {
    let mut gk = Gatekeeper::new(RiskConfig::default());

    // Tighten-only: reducing risk_per_trade_pct succeeds; increasing it fails!
    let tighter = RiskConfig {
        risk_per_trade_pct: Decimal::new(3, 1),
        ..RiskConfig::default()
    };
    gk.update_config(tighter).expect("tightening allowed");
    let looser = RiskConfig {
        risk_per_trade_pct: Decimal::new(8, 1),
        ..RiskConfig::default()
    };
    assert!(gk.update_config(looser).is_err());

    let rules = sample_rules();
    let intent = EntryIntent {
        venue: Venue::Spot,
        symbol: Symbol("BTCUSDT".into()),
        direction: Direction::Long,
        strategy: StrategyId::TrendPullback,
        entry_price: Price(Decimal::from(100_u32)),
        stop_price: Price(Decimal::from(98_u32)),
        tp1_price: Price(Decimal::from(103_u32)),
        size_mult: Decimal::ONE,
        scanner_mult: Decimal::ONE,
        is_scanner: false,
    };

    let base_ctx = GatekeeperContext {
        now_ms: 1_700_000_000_000,
        mode: EngineMode::Ready,
        rules: &rules,
        symbol_in_universe: true,
        symbol_quarantined: false,
        symbol_cooldown_until_ms: None,
        btc_guard_active: false,
        macro_blackout_active: false,
        market_data_age_ms: 200,
        user_stream_healthy: true,
        clock_drift_ms: 50,
        open_positions: &[],
        equity_total_usd: Decimal::from(1_000_u32),
        free_quote_usd: Decimal::from(1_000_u32),
        request_weight_pct: Decimal::from(20_u32),
        orders_10s_pct: Decimal::from(10_u32),
        loss_streak_cooldown_until_ms: None,
        orders_today: 5,
        profit_lock_stop: false,
        profit_lock_mult: Decimal::ONE,
        kelly: None,
        percent_price_bounds: Some((Price(Decimal::from(90_u32)), Price(Decimal::from(110_u32)))),
        fixed_notional_usd: None,
    };

    assert!(matches!(
        gk.evaluate(&intent, &base_ctx),
        GateDecision::Accept { .. }
    ));

    // Rate limit guard rejects at >= 60% weight
    let rl_ctx = GatekeeperContext {
        request_weight_pct: Decimal::from(60_u32),
        ..base_ctx.clone()
    };
    assert!(matches!(
        gk.evaluate(&intent, &rl_ctx),
        GateDecision::Reject { ref reasons, .. }
            if matches!(reasons.first(), Some(RejectReason::RateLimitGuard { .. }))
    ));

    // Kelly f <= 0 rejects with auto_disable_strategy = true
    let kelly_ctx = GatekeeperContext {
        kelly: Some(KellyInput {
            trade_count: 150,
            lower_win_rate: Decimal::new(35, 2),
            lower_payoff_ratio: Decimal::ONE,
        }),
        ..base_ctx.clone()
    };
    assert!(matches!(
        gk.evaluate(&intent, &kelly_ctx),
        GateDecision::Reject {
            auto_disable_strategy: true,
            ..
        }
    ));

    // Operational health hooks
    assert_eq!(
        Gatekeeper::check_operational_health(35_000, 100),
        Some(ModeTrigger::DataStaleOver30s)
    );
    assert_eq!(
        Gatekeeper::check_operational_health(200, 900),
        Some(ModeTrigger::ClockDriftExceeded)
    );

    // CombinedEquityCounters across venues + profit lock reduce (+2%) + daily halt (-2%) + loss streak
    let mut counters =
        CombinedEquityCounters::new(Venue::Spot, Decimal::from(1_000_u32), 1_700_000_000_000);
    let cfg = RiskConfig {
        profit_lock_mode: ProfitLockMode::Reduce,
        ..RiskConfig::default()
    };
    let up = counters.update_venue_equity(
        Venue::Spot,
        Decimal::from(1_025_u32),
        1_700_000_060_000,
        &cfg,
    );
    assert_eq!(up.profit_lock_mult, Decimal::new(5, 1));
    assert!(up.breaker_trigger.is_none());

    let down =
        counters.update_venue_equity(Venue::Spot, Decimal::from(975_u32), 1_700_000_120_000, &cfg);
    assert_eq!(down.breaker_trigger, Some(ModeTriggerKind::DailyLossHalt));

    // Loss streak: 3 losses -> 2h cooldown
    for i in 0..3 {
        counters.record_trade_outcome(Decimal::from(-10_i32), 1_700_000_200_000 + i * 1000);
    }
    assert_eq!(counters.consecutive_losses(), 3);
    assert!(counters.loss_cooldown_until_ms().is_some());
}

#[test]
fn manual_pause_and_kill_reset_can_resume_to_ready_and_invalid_risk_update_rejected() {
    let mut m = EngineModeMachine::new(0);
    m.apply(ModeTrigger::SyncStarted, 100);
    m.apply(ModeTrigger::SyncComplete, 200);
    assert_eq!(m.mode(), EngineMode::Ready);

    // ManualPause ignores StableTick, then resumes explicitly via ManualResumeFromPause.
    m.apply(
        ModeTrigger::ManualPause {
            reason: "operator".into(),
        },
        1_000,
    );
    assert_eq!(m.mode(), EngineMode::Paused);
    m.apply(ModeTrigger::StableTick { now_ms: 10_000 }, 10_000);
    m.apply(ModeTrigger::StableTick { now_ms: 20_000 }, 20_000);
    assert_eq!(m.mode(), EngineMode::Paused);
    m.apply(ModeTrigger::ManualResumeFromPause, 21_000);
    assert_eq!(m.mode(), EngineMode::Ready);

    // Killed -> ManualResetKill { confirmed: true } -> Paused -> ManualResumeFromPause -> Ready.
    m.apply(ModeTrigger::ManualPanic, 22_000);
    assert_eq!(m.mode(), EngineMode::Killed);
    m.apply(ModeTrigger::ManualResetKill { confirmed: true }, 23_000);
    assert_eq!(m.mode(), EngineMode::Paused);
    m.apply(ModeTrigger::ManualResumeFromPause, 24_000);
    assert_eq!(m.mode(), EngineMode::Ready);

    // Gatekeeper::update_config must reject out-of-range values even if they satisfy directional check.
    let mut gk = Gatekeeper::new(RiskConfig::default());
    let invalid_daily = RiskConfig {
        daily_loss_halt_pct: Decimal::from(10_i32), // +10% >= -2.5% but outside [-4.0, -0.5]!
        ..RiskConfig::default()
    };
    let err = gk
        .update_config(invalid_daily)
        .expect_err("out-of-range daily_loss_halt_pct must be rejected");
    assert!(
        matches!(err, bot_risk::TightenOnlyError::Validation(_)),
        "expected Validation error, got {err:?}"
    );
}
