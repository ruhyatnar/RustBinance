//! Standalone signal dry-run (`binance-botctl dry-run signal`, M19).
//!
//! Fetches public klines (or uses deterministic offline fixtures when `--offline` is set),
//! runs the exact `bot-strategy` indicator + S1/S2 gate pipeline, and prints a gate-by-gate
//! breakdown without starting the daemon or sending any order.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bot_config::BotConfig;
use bot_exchange::{FuturesRestClient, RateLimitTracker, SpotRestClient};
use bot_strategy::{
    CommissionRates, GateLog, HtfRegimeFeed, IndicatorFeed, S1TrendPullback, S2Breakout, Strategy,
    StrategyCtx, f64_to_decimal,
};
use bot_types::{Kline, Symbol, Venue, VenueCaps};
use rust_decimal::Decimal;
use serde::Serialize;

use crate::probes::{load_probe_config, resolve_rest_base_url};

const KLINE_TIMEOUT: Duration = Duration::from_secs(10);
const SIGNAL_BAR_LIMIT: u16 = 650;
const HTF_BAR_LIMIT: u16 = 250;

/// Options for [`run_dry_run_signal`].
#[derive(Debug, Clone)]
pub struct DryRunSignalOptions {
    /// Target venue (`Spot` or `UsdsFutures`).
    pub venue: Venue,
    /// Target symbol (e.g. `BTCUSDT`).
    pub symbol: String,
    /// Use deterministic offline synthetic klines without network calls.
    pub offline: bool,
    /// Optional explicit TOML configuration path.
    pub config: Option<PathBuf>,
    /// Optional REST base URL override (tests).
    pub rest_url_override: Option<String>,
}

/// Gate-by-gate evaluation breakdown for one strategy.
#[derive(Debug, Clone, Serialize)]
pub struct StrategyGateBreakdown {
    /// Strategy name (e.g. `"S1TrendPullback"`).
    pub strategy: String,
    /// `"SIGNAL"` (with side/entry/stop) or `"NO_SIGNAL"`.
    pub decision: String,
    /// Ordered gate evaluations returned by [`Strategy::evaluate`].
    pub gates: Vec<GateLog>,
}

/// Output of `binance-botctl dry-run signal`.
#[derive(Debug, Clone, Serialize)]
pub struct DryRunSignalReport {
    /// Target venue.
    pub venue: Venue,
    /// Normalized symbol.
    pub symbol: String,
    /// Number of closed signal bars fed into [`IndicatorFeed`].
    pub bars_loaded: usize,
    /// Latest closed bar close price (exact `Decimal`).
    pub latest_close: Decimal,
    /// Per-strategy decision and gate breakdown.
    pub strategies: Vec<StrategyGateBreakdown>,
    /// Always `false` — standalone dry-run never transmits orders.
    pub order_sent: bool,
}

impl DryRunSignalReport {
    /// Render as human-readable text with per-gate pass/fail breakdown.
    #[must_use]
    pub fn render_text(&self) -> String {
        let mut out = format!(
            "dry-run signal ({:?} {}) — bars={} latest_close={} order_sent={}",
            self.venue, self.symbol, self.bars_loaded, self.latest_close, self.order_sent
        );
        for strat in &self.strategies {
            let _ = write!(out, "\n  [{}] {}", strat.strategy, strat.decision);
            for g in &strat.gates {
                let tag = if g.passed { "PASS" } else { "FAIL" };
                let _ = write!(out, "\n    - [{tag}] {}: {}", g.gate, g.detail);
            }
        }
        out
    }
}

fn synthetic_closed_klines(count: usize, step_ms: i64) -> Vec<Kline> {
    let start_ms = 1_700_000_000_000_i64;
    let mut out = Vec::with_capacity(count);
    for idx in 0..count {
        let i_i64 = i64::try_from(idx).unwrap_or(0);
        let open_time_ms = start_ms.saturating_add(i_i64.saturating_mul(step_ms));
        let close_time_ms = open_time_ms.saturating_add(step_ms.saturating_sub(1));
        #[allow(clippy::cast_precision_loss)]
        let base = 50_000.0 + (idx as f64) * 3.0;
        out.push(Kline {
            open_time_ms,
            close_time_ms,
            open: base,
            high: base + 30.0,
            low: base - 25.0,
            close: base + 10.0,
            volume: 120.0,
            quote_volume: (base + 10.0) * 120.0,
            trades: 80,
            taker_buy_base: 65.0,
            is_closed: true,
        });
    }
    out
}

async fn fetch_klines(
    cfg: &BotConfig,
    opts: &DryRunSignalOptions,
    symbol: &str,
) -> Result<(Vec<Kline>, Vec<Kline>)> {
    if opts.offline {
        return Ok((
            synthetic_closed_klines(usize::from(SIGNAL_BAR_LIMIT), 300_000),
            synthetic_closed_klines(usize::from(HTF_BAR_LIMIT), 14_400_000),
        ));
    }
    let base_url = resolve_rest_base_url(cfg, opts.venue, opts.rest_url_override.as_deref());
    match opts.venue {
        Venue::Spot => {
            let client = SpotRestClient::new(&base_url, KLINE_TIMEOUT)
                .map_err(|e| anyhow::anyhow!("SpotRestClient init failed: {e}"))?;
            let sig = client
                .klines(symbol, "5m", SIGNAL_BAR_LIMIT)
                .await
                .map_err(|e| anyhow::anyhow!("Spot 5m klines failed: {e}"))?;
            let htf = client
                .klines(symbol, "4h", HTF_BAR_LIMIT)
                .await
                .map_err(|e| anyhow::anyhow!("Spot 4h klines failed: {e}"))?;
            Ok((sig, htf))
        }
        Venue::UsdsFutures => {
            let client = FuturesRestClient::new(
                &base_url,
                KLINE_TIMEOUT,
                None,
                Arc::new(RateLimitTracker::new()),
            )
            .map_err(|e| anyhow::anyhow!("FuturesRestClient init failed: {e}"))?;
            let sig = client
                .klines(symbol, "5m", SIGNAL_BAR_LIMIT)
                .await
                .map_err(|e| anyhow::anyhow!("Futures 5m klines failed: {e}"))?;
            let htf = client
                .klines(symbol, "4h", HTF_BAR_LIMIT)
                .await
                .map_err(|e| anyhow::anyhow!("Futures 4h klines failed: {e}"))?;
            Ok((sig, htf))
        }
    }
}

/// Fetch public klines (or offline fixture), run `IndicatorFeed` + `HtfRegimeFeed` + S1/S2
/// evaluation, and return the structured gate breakdown. Zero orders are sent.
///
/// # Errors
/// Returns an error if configuration loading, public kline fetch, or bar data is empty.
pub async fn run_dry_run_signal(opts: &DryRunSignalOptions) -> Result<DryRunSignalReport> {
    let cfg = if let Some(path) = opts.config.as_deref() {
        load_probe_config(Some(path))?
    } else {
        BotConfig::default()
    };
    let symbol = opts.symbol.trim().to_ascii_uppercase();
    let (signal_bars, htf_bars) = fetch_klines(&cfg, opts, &symbol).await?;

    let mut htf_feed = HtfRegimeFeed::new();
    for bar in htf_bars.iter().filter(|b| b.is_closed) {
        let _ = htf_feed.on_closed_htf_bar(bar);
    }
    let htf = htf_feed.regime();

    let mut feed = IndicatorFeed::new();
    let mut last_bar: Option<Kline> = None;
    let mut last_snap = feed.snapshot();
    let mut closed_count = 0usize;
    for bar in signal_bars.into_iter().filter(|b| b.is_closed) {
        last_snap = feed.on_closed_bar(&bar);
        last_bar = Some(bar);
        closed_count += 1;
    }

    let Some(bar) = last_bar else {
        bail!("no closed kline bars available for {symbol}");
    };
    let latest_close = f64_to_decimal(bar.close).context("latest bar close price is not finite")?;

    let venue_caps = match opts.venue {
        Venue::Spot => VenueCaps::spot(),
        Venue::UsdsFutures => VenueCaps::usds_futures(cfg.venues.futures.max_leverage),
    };
    let commission = match opts.venue {
        Venue::Spot => CommissionRates::from_exchange_ratios(
            Decimal::new(1, 3), // 10 bps VIP0 Spot
            Decimal::new(1, 3),
        ),
        Venue::UsdsFutures => CommissionRates::from_exchange_ratios(
            Decimal::new(2, 4), // 2 bps maker
            Decimal::new(5, 4), // 5 bps taker
        ),
    };

    let ctx = StrategyCtx {
        venue: opts.venue,
        venue_caps,
        symbol: Symbol(symbol.clone()),
        bar,
        indicators: last_snap,
        htf,
        spread_bps: Decimal::new(1, 0),
        book_ticker_age_ms: 50,
        gap_free_300_bars: closed_count >= 300,
        btc_long_regime_ok: true,
        target_vol: Decimal::new(2, 2),
        realized_vol_30d: Decimal::new(2, 2),
        atr_pct_percentile_90d: Decimal::from(50),
        has_open_position: false,
        bars_since_last_exit: None,
        macro_blackout_active: false,
        obi_smooth: None,
        commission,
        expected_slippage_bps: Decimal::new(2, 0),
    };

    let mut s1 = S1TrendPullback::new(
        cfg.strategy.trend_pullback.clone(),
        cfg.universe.max_spread_bps,
    )
    .with_allow_short(opts.venue == Venue::UsdsFutures);
    let mut s2 = S2Breakout::new(
        cfg.strategy.breakout.clone(),
        cfg.universe.max_spread_bps,
        cfg.strategy.trend_pullback.cost_multiple,
    )
    .with_allow_short(opts.venue == Venue::UsdsFutures);

    let d1 = s1.evaluate(&ctx);
    let d2 = s2.evaluate(&ctx);

    Ok(DryRunSignalReport {
        venue: opts.venue,
        symbol,
        bars_loaded: closed_count,
        latest_close,
        strategies: vec![
            StrategyGateBreakdown {
                strategy: "S1TrendPullback".to_owned(),
                decision: format_decision(&d1),
                gates: d1.gate_logs,
            },
            StrategyGateBreakdown {
                strategy: "S2Breakout".to_owned(),
                decision: format_decision(&d2),
                gates: d2.gate_logs,
            },
        ],
        order_sent: false,
    })
}

fn format_decision(d: &bot_strategy::SignalDecision) -> String {
    match &d.intent {
        Some(intent) => format!(
            "SIGNAL ({:?}, entry={}, stop={}, tp1={})",
            intent.direction, intent.entry_ref.0, intent.stop.0, intent.tp1.0
        ),
        None => "NO_SIGNAL".to_owned(),
    }
}
