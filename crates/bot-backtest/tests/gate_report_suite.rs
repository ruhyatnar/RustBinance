#![allow(clippy::expect_used)]

use std::str::FromStr;

use bot_backtest::gate_report::{
    SpotGate, evaluate_g1_gate, evaluate_sqlite_gate, render_operational_gate_markdown,
};
use bot_backtest::{BacktestMetrics, ParameterSensitivityReport, RDistribution};
use bot_store::{FillRecord, JournalEnvelope, JournalEvent, SqliteStore};
use bot_types::{Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;

fn dec(s: &str) -> Decimal {
    Decimal::from_str(s).expect("valid decimal")
}

fn passing_metrics() -> BacktestMetrics {
    BacktestMetrics {
        total_trades: 320,
        winning_trades: 180,
        losing_trades: 140,
        net_pnl_usd: dec("4500"),
        profit_factor: dec("1.45"),
        profit_factor_without_top3: dec("1.22"),
        expectancy_r: dec("0.22"),
        max_drawdown_pct: dec("11.5"),
        sharpe_ratio: dec("1.4"),
        sortino_ratio: dec("2.1"),
        deflated_sharpe: dec("1.1"),
        turnover_multiple: dec("12.0"),
        exposure_pct: dec("35.0"),
        r_distribution: RDistribution::default(),
    }
}

fn passing_sensitivity() -> ParameterSensitivityReport {
    ParameterSensitivityReport {
        total_neighbors: 10,
        passing_neighbors: 8,
        passing_ratio_pct: dec("80.0"),
    }
}

#[test]
fn g1_gate_passes_only_when_all_section_13_3_criteria_met() {
    let verdict = evaluate_g1_gate(&passing_metrics(), &passing_sensitivity(), 1100);
    assert!(verdict.passed, "expected G1 to pass: {:?}", verdict.checks);
    assert_eq!(verdict.checks.len(), 7);
    assert!(verdict.checks.iter().all(|c| c.passed));
}

#[test]
fn g1_gate_fails_when_any_single_threshold_misses() {
    assert!(!evaluate_g1_gate(&passing_metrics(), &passing_sensitivity(), 365).passed);

    let mut m_trades = passing_metrics();
    m_trades.total_trades = 299;
    assert!(!evaluate_g1_gate(&m_trades, &passing_sensitivity(), 1100).passed);

    let mut m_pf = passing_metrics();
    m_pf.profit_factor = dec("1.29");
    assert!(!evaluate_g1_gate(&m_pf, &passing_sensitivity(), 1100).passed);

    let mut m_exp = passing_metrics();
    m_exp.expectancy_r = dec("0.14");
    assert!(!evaluate_g1_gate(&m_exp, &passing_sensitivity(), 1100).passed);

    let mut m_dd = passing_metrics();
    m_dd.max_drawdown_pct = dec("15.1");
    assert!(!evaluate_g1_gate(&m_dd, &passing_sensitivity(), 1100).passed);

    let mut s_low = passing_sensitivity();
    s_low.passing_ratio_pct = dec("69.9");
    assert!(!evaluate_g1_gate(&passing_metrics(), &s_low, 1100).passed);

    let mut m_top3 = passing_metrics();
    m_top3.profit_factor_without_top3 = dec("1.0");
    assert!(!evaluate_g1_gate(&m_top3, &passing_sensitivity(), 1100).passed);
}

#[test]
fn sqlite_gate_report_filters_strictly_by_venue() {
    let mut store = SqliteStore::open_in_memory().expect("in-memory sqlite");
    let btc = Symbol("BTCUSDT".into());

    store
        .append_event(
            1_700_000_000_000,
            &JournalEnvelope::new(JournalEvent::FillRecorded {
                fill: FillRecord {
                    venue: Venue::Spot,
                    trade_id: 1,
                    symbol: btc.clone(),
                    client_id: "spot-1".into(),
                    price: Price(dec("50000")),
                    qty: Qty(dec("0.01")),
                    fee: dec("0.5"),
                    fee_asset: "USDT".into(),
                    is_maker: true,
                    ts_ms: 1_700_000_000_000,
                },
            }),
        )
        .expect("append spot fill");

    store
        .append_event(
            1_700_000_010_000,
            &JournalEnvelope::new(JournalEvent::FillRecorded {
                fill: FillRecord {
                    venue: Venue::UsdsFutures,
                    trade_id: 2,
                    symbol: btc,
                    client_id: "fut-1".into(),
                    price: Price(dec("50000")),
                    qty: Qty(dec("0.05")),
                    fee: dec("1.25"),
                    fee_asset: "USDT".into(),
                    is_maker: false,
                    ts_ms: 1_700_000_010_000,
                },
            }),
        )
        .expect("append futures fill");

    let spot_metrics = evaluate_sqlite_gate(&store, Venue::Spot, SpotGate::G2).expect("eval spot");
    assert_eq!(spot_metrics.venue, Venue::Spot);
    assert_eq!(spot_metrics.total_fills, 1);
    assert_eq!(spot_metrics.total_fees_quote, dec("0.5"));
    assert_eq!(spot_metrics.halt_events, 0);

    let md = render_operational_gate_markdown(&spot_metrics);
    assert!(md.contains("Spot"));
    assert!(md.contains("G2"));
}

#[test]
fn sqlite_gate_report_handles_empty_db_cleanly() {
    let store = SqliteStore::open_in_memory().expect("in-memory sqlite");
    let metrics = evaluate_sqlite_gate(&store, Venue::Spot, SpotGate::G3).expect("eval empty");
    assert_eq!(metrics.total_fills, 0);
    assert_eq!(metrics.total_fees_quote, Decimal::ZERO);
    assert!(!metrics.passed);
}

#[test]
#[allow(clippy::too_many_lines)]
fn futures_gate_scope_separates_long_and_short_and_flags_incidents() {
    use bot_backtest::gate_report::{GateScope, evaluate_sqlite_gate_for_scope};
    use bot_types::{
        Direction, Order, OrderKind, OrderPurpose, OrderStatus, Position, PositionState, Side,
        StrategyId,
    };
    use time::OffsetDateTime;
    use uuid::Uuid;

    let mut store = SqliteStore::open_in_memory().expect("in-memory sqlite");
    let btc = Symbol("BTCUSDT".into());
    let long_id = Uuid::new_v4();
    let short_id = Uuid::new_v4();
    let now = OffsetDateTime::UNIX_EPOCH;
    let t0 = 1_700_000_000_000_i64;
    let t15d = t0 + 15 * 86_400_000_i64;

    let mk_pos = |id: Uuid, direction: Direction| Position {
        id,
        venue: Venue::UsdsFutures,
        symbol: btc.clone(),
        strategy: StrategyId::TrendPullback,
        direction,
        state: PositionState::OpenProtected,
        qty_initial: Qty(dec("0.01")),
        qty_open: Qty(dec("0.01")),
        entry_price: Price(dec("50000")),
        r_distance: dec("1000"),
        leverage: Some(2),
        liq_price: Some(Price(dec("25000"))),
        stop_price: Price(dec("49000")),
        tp1_price: Price(dec("51500")),
        tp2_price: Some(Price(dec("53000"))),
        best_price: Price(dec("50000")),
        breakeven_done: false,
        tp1_done: false,
        realized_pnl_quote: Decimal::ZERO,
        fees_paid_quote: Decimal::ZERO,
        opened_at: now,
    };

    let mk_order = |client_id: &str, pid: Uuid, side: Side| Order {
        venue: Venue::UsdsFutures,
        client_id: client_id.into(),
        exchange_id: None,
        position_id: Some(pid),
        symbol: btc.clone(),
        side,
        kind: OrderKind::Limit,
        purpose: OrderPurpose::Entry,
        price: Some(Price(dec("50000"))),
        qty: Qty(dec("0.01")),
        filled_qty: Qty(dec("0.01")),
        avg_fill_price: Some(Price(dec("50000"))),
        status: OrderStatus::Filled,
        created_at: now,
        updated_at: now,
    };

    // Open & fill Long position spanning 15 days (>= G2 14 days), closed cleanly.
    store
        .append_event(
            t0,
            &JournalEnvelope::new(JournalEvent::PositionOpened {
                position: mk_pos(long_id, Direction::Long),
            }),
        )
        .expect("open long");
    store
        .append_event(
            t0 + 1000,
            &JournalEnvelope::new(JournalEvent::OrderIntended {
                order: mk_order("fut-long-1", long_id, Side::Buy),
            }),
        )
        .expect("order long");
    store
        .append_event(
            t0 + 2000,
            &JournalEnvelope::new(JournalEvent::FillRecorded {
                fill: FillRecord {
                    venue: Venue::UsdsFutures,
                    trade_id: 101,
                    symbol: btc.clone(),
                    client_id: "fut-long-1".into(),
                    price: Price(dec("50000")),
                    qty: Qty(dec("0.01")),
                    fee: dec("0.20"),
                    fee_asset: "USDT".into(),
                    is_maker: true,
                    ts_ms: t0 + 2000,
                },
            }),
        )
        .expect("fill long");
    store
        .append_event(
            t15d,
            &JournalEnvelope::new(JournalEvent::PositionStateChanged {
                venue: Venue::UsdsFutures,
                position_id: long_id,
                state: PositionState::Closed,
                qty_open: Qty(Decimal::ZERO),
                realized_pnl_quote: dec("25.0"),
                fees_paid_quote: dec("0.20"),
                close_reason: Some("tp2_hit".into()),
            }),
        )
        .expect("close long");

    // Open & fill Short position spanning 15 days, closed via liquidation incident.
    store
        .append_event(
            t0,
            &JournalEnvelope::new(JournalEvent::PositionOpened {
                position: mk_pos(short_id, Direction::Short),
            }),
        )
        .expect("open short");
    store
        .append_event(
            t0 + 1000,
            &JournalEnvelope::new(JournalEvent::OrderIntended {
                order: mk_order("fut-short-1", short_id, Side::Sell),
            }),
        )
        .expect("order short");
    store
        .append_event(
            t0 + 2000,
            &JournalEnvelope::new(JournalEvent::FillRecorded {
                fill: FillRecord {
                    venue: Venue::UsdsFutures,
                    trade_id: 202,
                    symbol: btc,
                    client_id: "fut-short-1".into(),
                    price: Price(dec("50000")),
                    qty: Qty(dec("0.01")),
                    fee: dec("0.30"),
                    fee_asset: "USDT".into(),
                    is_maker: false,
                    ts_ms: t0 + 2000,
                },
            }),
        )
        .expect("fill short");
    store
        .append_event(
            t15d,
            &JournalEnvelope::new(JournalEvent::PositionStateChanged {
                venue: Venue::UsdsFutures,
                position_id: short_id,
                state: PositionState::Closed,
                qty_open: Qty(Decimal::ZERO),
                realized_pnl_quote: dec("-15.0"),
                fees_paid_quote: dec("0.30"),
                close_reason: Some("margin_call_liquidation".into()),
            }),
        )
        .expect("close short");

    let long_m = evaluate_sqlite_gate_for_scope(&store, GateScope::FuturesLong, SpotGate::G2)
        .expect("eval futures-long");
    assert_eq!(long_m.scope, GateScope::FuturesLong);
    assert_eq!(long_m.total_fills, 1);
    assert_eq!(long_m.closed_positions, 1);
    assert_eq!(long_m.realized_pnl_quote, dec("25.0"));
    assert_eq!(long_m.liquidation_incidents, 0);
    assert!(long_m.passed, "clean futures-long G2 should pass");

    let short_m = evaluate_sqlite_gate_for_scope(&store, GateScope::FuturesShort, SpotGate::G2)
        .expect("eval futures-short");
    assert_eq!(short_m.scope, GateScope::FuturesShort);
    assert_eq!(short_m.total_fills, 1);
    assert_eq!(short_m.closed_positions, 1);
    assert_eq!(short_m.realized_pnl_quote, dec("-15.0"));
    assert_eq!(short_m.liquidation_incidents, 1);
    assert!(
        !short_m.passed,
        "futures-short with liquidation incident must fail"
    );

    let md = render_operational_gate_markdown(&short_m);
    assert!(md.contains("Futures-Short"));
    assert!(md.contains("Liquidation / MARGIN_CALL incidents"));
}
