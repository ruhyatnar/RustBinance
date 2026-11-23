//! Shared signal quality gates (§6.5): cost gate, spread/freshness, gap-free/warmup,
//! BTC regime, vol targeting, single-position & cooldown, and macro blackout.

use bot_types::{Direction, Price};
use rust_decimal::Decimal;

use crate::types::{CommissionRates, GateLog, StrategyCtx};

/// Maximum `bookTicker` age allowed for signal emission (500 ms per §6.5 #2).
pub const MAX_BOOK_TICKER_AGE_MS: i64 = 500;
/// Minimum closed bars cooldown after exiting the same symbol (4 bars per §6.5 #6).
pub const DEFAULT_EXIT_COOLDOWN_BARS: u32 = 4;
/// Minimum vol-targeting multiplier (§6.5 #5).
pub const VOL_TARGET_MIN_MULT: Decimal = Decimal::from_parts(5, 0, 0, false, 1);
/// Maximum vol-targeting multiplier (§6.5 #5).
pub const VOL_TARGET_MAX_MULT: Decimal = Decimal::from_parts(125, 0, 0, false, 2);
/// Volatility-spike ATR% 90-day percentile cap threshold (`> 85` → `<= 0.6` per §6.5 #5).
pub const ATR_PERCENTILE_CAP_THRESHOLD: Decimal = Decimal::from_parts(85, 0, 0, false, 0);
/// Cap applied to `size_mult` when 90-day ATR% percentile exceeds 85 (§6.5 #5).
pub const ATR_PERCENTILE_MAX_MULT: Decimal = Decimal::from_parts(6, 0, 0, false, 1);

/// Compute volatility-targeting `size_mult = clamp(target_vol / realized_vol_30d, 0.5, 1.25)`,
/// further capped at `<= 0.6` when `atr_pct_percentile_90d > 85` (§6.5 #5).
#[must_use]
pub fn compute_vol_target_size_mult(
    target_vol: Decimal,
    realized_vol_30d: Decimal,
    atr_pct_percentile_90d: Decimal,
) -> Decimal {
    let raw = if target_vol > Decimal::ZERO && realized_vol_30d > Decimal::ZERO {
        target_vol
            .checked_div(realized_vol_30d)
            .unwrap_or(Decimal::ONE)
    } else {
        Decimal::ONE
    };
    let clamped = raw.clamp(VOL_TARGET_MIN_MULT, VOL_TARGET_MAX_MULT);
    if atr_pct_percentile_90d > ATR_PERCENTILE_CAP_THRESHOLD {
        clamped.min(ATR_PERCENTILE_MAX_MULT)
    } else {
        clamped
    }
}

/// Evaluate §6.5 #1 Cost Gate using actual `CommissionRates` from `account.commission`:
/// `(tp1 - entry) / entry >= cost_multiple * round_trip_cost`,
/// where `round_trip_cost = fee_entry + fee_exit + 2 * expected_slippage`.
#[must_use]
pub fn check_cost_gate(
    entry_ref: Price,
    tp1: Price,
    direction: Direction,
    commission: CommissionRates,
    expected_slippage_bps: Decimal,
    cost_multiple: Decimal,
) -> GateLog {
    let entry = entry_ref.as_decimal();
    let target = tp1.as_decimal();
    if entry <= Decimal::ZERO {
        return GateLog::fail("cost_gate", "entry_ref must be positive");
    }

    let reward = match direction {
        Direction::Long => target - entry,
        Direction::Short => entry - target,
    };
    let reward_bps = reward
        .checked_div(entry)
        .and_then(|r| r.checked_mul(Decimal::from(10_000_u32)))
        .unwrap_or(Decimal::ZERO);

    // Passive maker entry + conservative taker exit + 2 * expected slippage (bps).
    let round_trip_bps = commission
        .maker_bps
        .saturating_add(commission.taker_bps)
        .saturating_add(expected_slippage_bps.saturating_mul(Decimal::from(2_u32)));
    let required_bps = round_trip_bps.saturating_mul(cost_multiple);

    if reward_bps >= required_bps {
        GateLog::pass(
            "cost_gate",
            format!(
                "reward {reward_bps:.2} bps >= {cost_multiple}x round-trip ({required_bps:.2} bps)"
            ),
        )
    } else {
        GateLog::fail(
            "cost_gate",
            format!(
                "reward {reward_bps:.2} bps < {cost_multiple}x round-trip ({required_bps:.2} bps)"
            ),
        )
    }
}

/// Evaluate all §6.5 signal quality gates and return `(all_passed, size_mult, gate_logs)`.
#[must_use]
pub fn evaluate_quality_gates(
    ctx: &StrategyCtx,
    entry_ref: Price,
    tp1: Price,
    direction: Direction,
    cost_multiple: Decimal,
    max_spread_bps: Decimal,
    cooldown_bars: u32,
) -> (bool, Decimal, Vec<GateLog>) {
    let mut logs = Vec::with_capacity(8);

    // 0. Closed bar & gap-free 300 bars (§6.3, §6.5 #3)
    if !ctx.bar.is_closed {
        logs.push(GateLog::fail("closed_bar", "signal bar is not closed"));
    } else if !ctx.gap_free_300_bars {
        logs.push(GateLog::fail(
            "gap_free_300",
            "bar gap detected within last 300 bars",
        ));
    } else {
        logs.push(GateLog::pass(
            "closed_bar_and_gaps",
            "closed bar with 300-bar continuity",
        ));
    }

    // 1. Cost gate (§6.5 #1)
    logs.push(check_cost_gate(
        entry_ref,
        tp1,
        direction,
        ctx.commission,
        ctx.expected_slippage_bps,
        cost_multiple,
    ));

    // 2. Spread & fresh bookTicker < 500 ms (§6.5 #2)
    if ctx.spread_bps <= max_spread_bps && ctx.book_ticker_age_ms < MAX_BOOK_TICKER_AGE_MS {
        logs.push(GateLog::pass(
            "spread_and_freshness",
            format!(
                "spread {} bps <= {} bps, age {} ms < {} ms",
                ctx.spread_bps, max_spread_bps, ctx.book_ticker_age_ms, MAX_BOOK_TICKER_AGE_MS
            ),
        ));
    } else {
        logs.push(GateLog::fail(
            "spread_and_freshness",
            format!(
                "spread {} bps (max {}) or book age {} ms (max {})",
                ctx.spread_bps, max_spread_bps, ctx.book_ticker_age_ms, MAX_BOOK_TICKER_AGE_MS
            ),
        ));
    }

    // 3. BTC regime (§6.5 #4) — required for altcoin Longs (BTCUSDT itself is exempt from altcoin check).
    let is_btc = ctx.symbol.as_str().eq_ignore_ascii_case("BTCUSDT");
    if is_btc || ctx.btc_long_regime_ok {
        logs.push(GateLog::pass(
            "btc_regime",
            "BTC 4h regime & crash guard OK",
        ));
    } else {
        logs.push(GateLog::fail(
            "btc_regime",
            "BTC 4h close <= EMA50 or BTC Crash Guard active",
        ));
    }

    // 4. Single position per symbol & 4-bar cooldown (§6.5 #6)
    let cooldown_ok = ctx
        .bars_since_last_exit
        .is_none_or(|bars| bars >= cooldown_bars);
    if !ctx.has_open_position && cooldown_ok {
        logs.push(GateLog::pass(
            "position_and_cooldown",
            "no open position and cooldown clear",
        ));
    } else {
        logs.push(GateLog::fail(
            "position_and_cooldown",
            format!(
                "has_open_position={}, bars_since_last_exit={:?} (need >= {cooldown_bars})",
                ctx.has_open_position, ctx.bars_since_last_exit
            ),
        ));
    }

    // 5. Macro event blackout (§6.5 #7)
    if ctx.macro_blackout_active {
        logs.push(GateLog::fail(
            "macro_blackout",
            "within macro event blackout window (-5m..+2m)",
        ));
    } else {
        logs.push(GateLog::pass("macro_blackout", "no macro event blackout"));
    }

    // 6. Volatility targeting multiplier (§6.5 #5)
    let size_mult = compute_vol_target_size_mult(
        ctx.target_vol,
        ctx.realized_vol_30d,
        ctx.atr_pct_percentile_90d,
    );
    logs.push(GateLog::pass(
        "vol_targeting",
        format!("size_mult={size_mult}"),
    ));

    let all_passed = logs.iter().all(|g| g.passed);
    (all_passed, size_mult, logs)
}
