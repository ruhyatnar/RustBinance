//! Runtime market-data -> indicators -> strategy runner wired into [`EngineHandle`].
//!
//! Consumes [`ClosedKlineEvent`] and [`BookTickerEvent`] updates, maintains incremental
//! per-`(Venue, Symbol)` [`IndicatorFeed`] and [`HtfRegimeFeed`] state, forwards
//! [`MarketTick`] updates to [`EngineHandle::on_market_tick`], evaluates enabled
//! strategies (`S1TrendPullback`, `S2Breakout`), and submits generated intents via
//! [`EngineHandle::submit_entry`].

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use bot_config::BotConfig;
use bot_engine::{EngineHandle, EntrySubmissionResult, MarketTick};
use bot_indicators::Bar;
use bot_marketdata::{
    BookTickerEvent, ClosedKlineEvent, MarketStreamConfig, MarketStreamHandle, StreamPlan,
    Timeframe, spawn_market_stream,
};
use bot_risk::EntryIntent;
use bot_strategy::{
    CommissionRates, HtfRegimeFeed, IndicatorFeed, S1TrendPullback, S2Breakout, Strategy,
    StrategyCtx, StrategyEntryIntent, f64_to_decimal,
};
use bot_types::{EngineMode, Price, Symbol, TradingMode, Venue, VenueCaps};
use rust_decimal::Decimal;
use time::OffsetDateTime;
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;
use tracing::{debug, info, warn};

const SPOT_PROD_MARKET_WS_URL: &str = "wss://stream.binance.com:9443";
const SPOT_TESTNET_MARKET_WS_URL: &str = "wss://stream.testnet.binance.vision";
const FUTURES_PROD_MARKET_WS_URL: &str = "wss://fstream.binance.com";
const FUTURES_TESTNET_MARKET_WS_URL: &str = "wss://stream.binancefuture.com";

/// Default core liquid symbols subscribed when no dynamic universe is injected.
const DEFAULT_CORE_SYMBOLS: [&str; 3] = ["BTCUSDT", "ETHUSDT", "SOLUSDT"];

fn unix_now_ms() -> i64 {
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000).unwrap_or(0)
}

/// Summary of a single closed-kline processing step (used for observability and tests).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MarketStepOutcome {
    /// `true` if a [`MarketTick`] was forwarded to [`EngineHandle::on_market_tick`].
    pub tick_sent: bool,
    /// Number of strategy entry intents generated on this closed bar.
    pub signals_generated: usize,
    /// Number of intents accepted by [`EngineHandle::submit_entry`] (`Submitted`).
    pub entries_submitted: usize,
}

#[derive(Debug, Default)]
struct SymbolMarketState {
    signal_feed: IndicatorFeed,
    htf_feed: HtfRegimeFeed,
    closed_signal_bars: u64,
    bars_in_trade: u32,
    bars_since_last_exit: Option<u32>,
    had_open_position: bool,
}

/// Pure, deterministic market -> indicator -> strategy runner targeting [`EngineHandle`].
pub struct MarketStrategyRunner {
    config: BotConfig,
    engine: EngineHandle,
    signal_tf: Timeframe,
    htf: Timeframe,
    states: HashMap<(Venue, Symbol), SymbolMarketState>,
    btc_long_regime_ok: bool,
}

impl MarketStrategyRunner {
    /// Construct a new [`MarketStrategyRunner`] from [`BotConfig`] and [`EngineHandle`].
    #[must_use]
    pub fn new(config: BotConfig, engine: EngineHandle) -> Self {
        let signal_tf = Timeframe::from_binance_str(&config.strategy.trend_pullback.signal_tf)
            .unwrap_or(Timeframe::H1);
        let htf = Timeframe::from_binance_str(&config.strategy.trend_pullback.htf)
            .unwrap_or(Timeframe::H4);
        Self {
            config,
            engine,
            signal_tf,
            htf,
            states: HashMap::new(),
            btc_long_regime_ok: true,
        }
    }

    /// Process one [`ClosedKlineEvent`] with an optional latest [`BookTickerEvent`].
    ///
    /// - HTF closed bars update [`HtfRegimeFeed`] (and BTC regime filter for `BTCUSDT`).
    /// - Signal-TF closed bars update [`IndicatorFeed`], forward a [`MarketTick`] to
    ///   [`EngineHandle::on_market_tick`], evaluate enabled strategies, and submit any
    ///   emitted [`StrategyEntryIntent`] to [`EngineHandle::submit_entry`].
    #[allow(clippy::too_many_lines)]
    pub async fn on_closed_kline(
        &mut self,
        event: ClosedKlineEvent,
        book_ticker: Option<&BookTickerEvent>,
        now_ms: i64,
    ) -> MarketStepOutcome {
        let mut outcome = MarketStepOutcome::default();
        if !event.kline.is_closed {
            return outcome;
        }

        let key = (event.venue, event.symbol.clone());

        if event.timeframe == self.htf {
            let state = self.states.entry(key).or_default();
            let regime = state.htf_feed.on_closed_htf_bar(&event.kline);
            if event.symbol.as_str() == "BTCUSDT" && regime.ready {
                self.btc_long_regime_ok = regime.close > regime.ema50;
            }
            return outcome;
        }

        if event.timeframe != self.signal_tf {
            return outcome;
        }

        // Update indicators strictly on closed signal bars.
        let (indicators, htf_regime, closed_bars, bars_in_trade, bars_since_last_exit) = {
            let state = self.states.entry(key.clone()).or_default();
            let bar: &Bar = &event.kline;
            let indicators = state.signal_feed.on_closed_bar(bar);
            state.closed_signal_bars = state.closed_signal_bars.saturating_add(1);
            let htf_regime = state.htf_feed.regime();
            (
                indicators,
                htf_regime,
                state.closed_signal_bars,
                state.bars_in_trade,
                state.bars_since_last_exit,
            )
        };

        let Some(close_dec) = f64_to_decimal(event.kline.close) else {
            return outcome;
        };
        if close_dec <= Decimal::ZERO {
            return outcome;
        }

        let atr_dec = indicators
            .atr14
            .and_then(f64_to_decimal)
            .filter(|d| *d > Decimal::ZERO)
            .unwrap_or_else(|| (close_dec * Decimal::new(5, 3)).max(Decimal::new(1, 8)));
        let rsi_dec = indicators.rsi14.and_then(f64_to_decimal);

        // 1. Forward MarketTick for open-position lifecycle evaluation (BE / trailing / time-stop).
        let tick = MarketTick {
            venue: event.venue,
            symbol: event.symbol.clone(),
            price: Price(close_dec),
            atr: atr_dec,
            rsi_14: rsi_dec,
            bars_in_trade,
            now_ms,
        };
        match self.engine.on_market_tick(tick).await {
            Ok(()) => outcome.tick_sent = true,
            Err(err) => {
                debug!(error = %err, symbol = %event.symbol, "engine on_market_tick skipped");
                return outcome;
            }
        }

        // 2. Check engine mode & active venue before evaluating new entry strategies.
        let telemetry = self.engine.telemetry();
        if telemetry.mode != EngineMode::Ready || telemetry.venue != event.venue {
            return outcome;
        }

        // Refresh open-position presence and live config from EngineActor.
        let (live_cfg, has_open_position, btc_guard_ok) =
            match self.engine.full_snapshot(now_ms).await {
                Ok(full) => {
                    let open = full
                        .positions
                        .iter()
                        .any(|p| p.venue == event.venue && p.symbol == event.symbol);
                    let guard_ok = full.btc_guard == "OK";
                    (full.config, open, guard_ok)
                }
                Err(_) => (self.config.clone(), false, true),
            };

        // Track bars_in_trade and post-exit cooldown per (Venue, Symbol).
        if let Some(state) = self.states.get_mut(&key) {
            if has_open_position {
                state.bars_in_trade = state.bars_in_trade.saturating_add(1);
                state.had_open_position = true;
                state.bars_since_last_exit = None;
            } else {
                if state.had_open_position {
                    state.bars_since_last_exit = Some(0);
                    state.had_open_position = false;
                } else if let Some(prev) = state.bars_since_last_exit {
                    state.bars_since_last_exit = Some(prev.saturating_add(1));
                }
                state.bars_in_trade = 0;
            }
        }

        let (spread_bps, book_ticker_age_ms) = match book_ticker {
            Some(bt) => (
                bt.spread_bps().unwrap_or(Decimal::new(5, 0)),
                now_ms.saturating_sub(bt.received_at_ms).max(0),
            ),
            None => (Decimal::new(5, 0), 0),
        };

        let venue_caps = match event.venue {
            Venue::Spot => VenueCaps::spot(),
            Venue::UsdsFutures => VenueCaps::usds_futures(live_cfg.venues.futures.max_leverage),
        };
        let commission = match event.venue {
            Venue::Spot => CommissionRates {
                maker_bps: Decimal::from(10_u32),
                taker_bps: Decimal::from(10_u32),
            },
            Venue::UsdsFutures => CommissionRates {
                maker_bps: Decimal::from(2_u32),
                taker_bps: Decimal::from(4_u32),
            },
        };

        let ctx = StrategyCtx {
            venue: event.venue,
            venue_caps,
            symbol: event.symbol.clone(),
            bar: event.kline,
            indicators,
            htf: htf_regime,
            spread_bps,
            book_ticker_age_ms,
            gap_free_300_bars: closed_bars >= 20,
            btc_long_regime_ok: self.btc_long_regime_ok && btc_guard_ok,
            target_vol: Decimal::new(2, 2),
            realized_vol_30d: Decimal::new(2, 2),
            atr_pct_percentile_90d: Decimal::from(50_u32),
            has_open_position,
            bars_since_last_exit,
            macro_blackout_active: false,
            obi_smooth: None,
            commission,
            expected_slippage_bps: Decimal::from(5_u32),
        };

        let mut intents: Vec<StrategyEntryIntent> = Vec::new();
        if live_cfg.strategy.trend_pullback.enabled {
            let mut s1 = S1TrendPullback::new(
                live_cfg.strategy.trend_pullback.clone(),
                live_cfg.universe.max_spread_bps,
            )
            .with_allow_short(venue_caps.can_short);
            if let Some(intent) = s1.on_bar_close(&ctx) {
                intents.push(intent);
            }
        }
        if live_cfg.strategy.breakout.enabled {
            let mut s2 = S2Breakout::new(
                live_cfg.strategy.breakout.clone(),
                live_cfg.universe.max_spread_bps,
                live_cfg.strategy.trend_pullback.cost_multiple,
            )
            .with_allow_short(venue_caps.can_short);
            if let Some(intent) = s2.on_bar_close(&ctx) {
                intents.push(intent);
            }
        }

        outcome.signals_generated = intents.len();
        for intent in intents {
            let entry_intent = map_strategy_intent(&intent);
            match self.engine.submit_entry(entry_intent, now_ms).await {
                Ok(EntrySubmissionResult::Submitted {
                    client_order_id,
                    position_id,
                    qty,
                }) => {
                    outcome.entries_submitted = outcome.entries_submitted.saturating_add(1);
                    info!(
                        venue = %intent.venue,
                        symbol = %intent.symbol,
                        strategy = ?intent.strategy,
                        client_order_id = %client_order_id,
                        position_id = %position_id,
                        qty = %qty.as_decimal(),
                        "strategy entry submitted"
                    );
                    // Only submit one entry per symbol per bar.
                    break;
                }
                Ok(EntrySubmissionResult::Rejected(decision)) => {
                    debug!(
                        venue = %intent.venue,
                        symbol = %intent.symbol,
                        strategy = ?intent.strategy,
                        decision = ?decision,
                        "strategy entry rejected by Gatekeeper"
                    );
                }
                Ok(EntrySubmissionResult::GatewayFailed(err)) => {
                    warn!(
                        venue = %intent.venue,
                        symbol = %intent.symbol,
                        error = %err,
                        "strategy entry gateway submission failed"
                    );
                }
                Err(err) => {
                    warn!(
                        venue = %intent.venue,
                        symbol = %intent.symbol,
                        error = %err,
                        "failed to deliver strategy entry to EngineActor"
                    );
                }
            }
        }

        outcome
    }

    /// Spawn the market/strategy event loop, exiting cleanly when `shutdown_rx` becomes `true`.
    #[must_use]
    pub fn spawn(
        mut self,
        mut closed_klines_rx: mpsc::Receiver<ClosedKlineEvent>,
        book_tickers_rx: watch::Receiver<HashMap<Symbol, BookTickerEvent>>,
        mut shutdown_rx: watch::Receiver<bool>,
    ) -> JoinHandle<()> {
        tokio::spawn(async move {
            loop {
                if *shutdown_rx.borrow() {
                    break;
                }
                tokio::select! {
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            break;
                        }
                    }
                    maybe_event = closed_klines_rx.recv() => {
                        let Some(event) = maybe_event else {
                            break;
                        };
                        let bt = book_tickers_rx.borrow().get(&event.symbol).cloned();
                        let _ = self.on_closed_kline(event, bt.as_ref(), unix_now_ms()).await;
                    }
                }
            }
            debug!("market strategy runner stopped");
        })
    }
}

fn map_strategy_intent(intent: &StrategyEntryIntent) -> EntryIntent {
    EntryIntent {
        venue: intent.venue,
        symbol: intent.symbol.clone(),
        direction: intent.direction,
        strategy: intent.strategy,
        entry_price: intent.entry_ref,
        stop_price: intent.stop,
        tp1_price: intent.tp1,
        size_mult: intent.size_mult,
        scanner_mult: Decimal::ONE,
        is_scanner: false,
    }
}

/// Build the default [`StreamPlan`] from config (respecting `universe.exclude_symbols`).
#[must_use]
pub fn build_default_stream_plan(cfg: &BotConfig) -> StreamPlan {
    let excluded: HashSet<&str> = cfg
        .universe
        .exclude_symbols
        .iter()
        .map(String::as_str)
        .collect();
    let signal_tf = Timeframe::from_binance_str(&cfg.strategy.trend_pullback.signal_tf)
        .unwrap_or(Timeframe::H1);
    let htf =
        Timeframe::from_binance_str(&cfg.strategy.trend_pullback.htf).unwrap_or(Timeframe::H4);

    let mut plan = StreamPlan::new();
    for raw in DEFAULT_CORE_SYMBOLS {
        if excluded.contains(raw) {
            continue;
        }
        let sym = Symbol(raw.to_owned());
        plan.add_kline(&sym, signal_tf);
        plan.add_kline(&sym, htf);
        plan.add_book_ticker(&sym);
    }
    plan
}

/// Handle owning the spawned [`MarketStrategyRunner`] task and optional live WS stream.
pub struct MarketRunnerHandle {
    runner_task: JoinHandle<()>,
    stream_handle: Option<MarketStreamHandle>,
    offline_kline_tx: Option<mpsc::Sender<ClosedKlineEvent>>,
}

impl MarketRunnerHandle {
    /// Cleanly stop the runner task and any underlying market stream.
    pub async fn shutdown(self) {
        if let Some(stream) = self.stream_handle {
            stream.shutdown().await;
        }
        drop(self.offline_kline_tx);
        let _ = self.runner_task.await;
    }
}

/// Spawn the daemon's market/strategy runner for the configured [`TradingMode`].
///
/// - In `Testnet` / `Live`: connects to Binance public market streams via `bot-marketdata`.
/// - In `Paper`: starts an offline-safe runner loop without making unsolicited network calls,
///   ensuring `binance-botd` never crashes when offline or in tests.
#[must_use]
pub fn spawn_daemon_market_runner(
    cfg: &BotConfig,
    active_venue: Venue,
    engine: EngineHandle,
    shutdown_rx: watch::Receiver<bool>,
) -> MarketRunnerHandle {
    let runner = MarketStrategyRunner::new(cfg.clone(), engine);

    match cfg.general.mode {
        TradingMode::Paper => {
            let (kline_tx, kline_rx) = mpsc::channel(256);
            let (_bt_tx, bt_rx) = watch::channel(HashMap::new());
            let runner_task = runner.spawn(kline_rx, bt_rx, shutdown_rx);
            info!(
                venue = %active_venue,
                mode = "paper",
                "market strategy runner started (offline-safe paper channel)"
            );
            MarketRunnerHandle {
                runner_task,
                stream_handle: None,
                offline_kline_tx: Some(kline_tx),
            }
        }
        TradingMode::Testnet | TradingMode::Live => {
            let is_live = cfg.general.mode == TradingMode::Live;
            let ws_base_url = match (active_venue, is_live) {
                (Venue::Spot, true) => SPOT_PROD_MARKET_WS_URL,
                (Venue::Spot, false) => SPOT_TESTNET_MARKET_WS_URL,
                (Venue::UsdsFutures, true) => FUTURES_PROD_MARKET_WS_URL,
                (Venue::UsdsFutures, false) => FUTURES_TESTNET_MARKET_WS_URL,
            };
            let plan = build_default_stream_plan(cfg);
            let stream_cfg = MarketStreamConfig {
                venue: active_venue,
                ws_base_url: ws_base_url.to_owned(),
                stale_timeout: Duration::from_millis(cfg.binance.stale_market_ms.max(1500)),
                ..MarketStreamConfig::default()
            };

            match spawn_market_stream(stream_cfg, &plan, None, None) {
                Ok((stream_handle, channels)) => {
                    let runner_task = runner.spawn(
                        channels.closed_klines_rx,
                        channels.book_tickers_rx,
                        shutdown_rx,
                    );
                    info!(
                        venue = %active_venue,
                        ws_base_url,
                        streams = plan.len(),
                        "market strategy runner connected to market stream"
                    );
                    MarketRunnerHandle {
                        runner_task,
                        stream_handle: Some(stream_handle),
                        offline_kline_tx: None,
                    }
                }
                Err(err) => {
                    warn!(
                        error = %err,
                        "failed to start market stream; falling back to idle runner channel"
                    );
                    let (kline_tx, kline_rx) = mpsc::channel(64);
                    let (_bt_tx, bt_rx) = watch::channel(HashMap::new());
                    let runner_task = runner.spawn(kline_rx, bt_rx, shutdown_rx);
                    MarketRunnerHandle {
                        runner_task,
                        stream_handle: None,
                        offline_kline_tx: Some(kline_tx),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use bot_engine::EngineActor;
    use bot_exchange::VenueAdapter;
    use bot_exec::{PaperBroker, PaperBrokerConfig};
    use bot_store::SqliteStore;
    use bot_types::{InstrumentRules, Kline};

    use super::*;

    fn make_bar(
        open_ms: i64,
        close_ms: i64,
        open: f64,
        high: f64,
        low: f64,
        close: f64,
        vol: f64,
    ) -> Kline {
        Kline {
            open_time_ms: open_ms,
            close_time_ms: close_ms,
            open,
            high,
            low,
            close,
            volume: vol,
            quote_volume: vol * close,
            trades: 100,
            taker_buy_base: vol * 0.5,
            is_closed: true,
        }
    }

    #[tokio::test]
    #[allow(clippy::too_many_lines)]
    async fn market_runner_feeds_synthetic_klines_into_engine_and_submits_breakout_entry() {
        let mut cfg = BotConfig::default();
        cfg.strategy.trend_pullback.enabled = true;
        cfg.strategy.breakout.enabled = true;

        let store = SqliteStore::open_in_memory().expect("in-memory sqlite");
        let btc_rules = InstrumentRules {
            venue: Venue::Spot,
            symbol: Symbol("BTCUSDT".to_owned()),
            tick_size: Decimal::new(1, 2),
            step_size: Decimal::new(1, 5),
            min_qty: Decimal::new(1, 5),
            max_qty: Decimal::from(10_000_u32),
            min_notional: Decimal::from(5_u32),
            status_trading: true,
            base_precision: 5,
            quote_precision: 2,
        };
        let broker = Arc::new(PaperBroker::new(PaperBrokerConfig {
            initial_quote_balance: Decimal::from(10_000_u32),
            rules: vec![btc_rules],
            ..PaperBrokerConfig::default()
        }));
        broker.activate().await.expect("activate paper broker");
        let event_rx = broker.take_event_rx().expect("event rx");

        let start_ms = 1_700_000_000_000_i64;
        let actor = EngineActor::bootstrap(
            cfg.clone(),
            "b01".to_owned(),
            store,
            broker,
            Decimal::from(10_000_u32),
            start_ms,
        )
        .await
        .expect("bootstrap engine");
        let (handle, join) = actor.spawn(event_rx);

        let mut runner = MarketStrategyRunner::new(cfg, handle.clone());
        let sym = Symbol("BTCUSDT".to_owned());

        // Deterministic 720-bar 1h dataset (with synthesized closed 4h bars every 4 1h bars):
        // warms up 4h EMA(50) (150 bars) & 1h EMA(200) (600 bars), then produces a pullback + breakout.
        let h1_ms = Timeframe::H1.interval_ms();
        let mut bars = Vec::with_capacity(720);
        let mut price = 100.0_f64;
        for i in 0_u32..720 {
            let open_ms = start_ms + i64::from(i) * h1_ms;
            let (open, high, low, close, vol) = if i < 640 {
                let wave = (f64::from(i) * 0.4).sin() * 0.8;
                let o = price;
                let c = (price * 1.0015 + wave * 0.3).max(10.0);
                let h = o.max(c) * 1.006;
                let l = o.min(c) * 0.994;
                price = c;
                (o, h, l, c, 1_000.0 + f64::from(i % 7) * 50.0)
            } else if i < 648 {
                let o = price;
                let c = price * 0.996;
                price = c;
                (o, o * 1.003, c * 0.995, c, 900.0)
            } else if i < 652 {
                let o = price;
                let c = price * 1.008;
                price = c;
                (o, c * 1.004, o * 0.997, c, 1_500.0)
            } else if i < 685 {
                let o = price;
                let c = price * if i % 2 == 0 { 1.002 } else { 0.9985 };
                price = c;
                (o, o.max(c) * 1.003, o.min(c) * 0.997, c, 1_000.0)
            } else if i < 690 {
                let o = price;
                let c = price * 1.015;
                price = c;
                (o, c * 1.005, o * 0.999, c, 2_800.0)
            } else {
                let o = price;
                let c = price * 1.001;
                price = c;
                (o, c * 1.004, o * 0.996, c, 1_100.0)
            };
            bars.push(make_bar(
                open_ms,
                open_ms + h1_ms - 1,
                open,
                high,
                low,
                close,
                vol,
            ));
        }

        let mut total_ticks = 0_usize;
        let mut total_signals = 0_usize;
        let mut total_submitted = 0_usize;

        for (idx, bar) in bars.iter().enumerate() {
            let now = bar.close_time_ms + 1;
            if idx >= 3 && idx % 4 == 3 {
                let window = bars.get(idx - 3..=idx).expect("4-bar window");
                let first = window.first().expect("first");
                let last = window.last().expect("last");
                let htf_bar = Kline {
                    open_time_ms: first.open_time_ms,
                    close_time_ms: last.close_time_ms,
                    open: first.open,
                    high: window.iter().map(|b| b.high).fold(f64::MIN, f64::max),
                    low: window.iter().map(|b| b.low).fold(f64::MAX, f64::min),
                    close: last.close,
                    volume: window.iter().map(|b| b.volume).sum(),
                    quote_volume: window.iter().map(|b| b.quote_volume).sum(),
                    trades: window.iter().map(|b| b.trades).sum(),
                    taker_buy_base: window.iter().map(|b| b.taker_buy_base).sum(),
                    is_closed: true,
                };
                let htf_out = runner
                    .on_closed_kline(
                        ClosedKlineEvent {
                            venue: Venue::Spot,
                            symbol: sym.clone(),
                            timeframe: Timeframe::H4,
                            kline: htf_bar,
                        },
                        None,
                        now,
                    )
                    .await;
                assert!(!htf_out.tick_sent, "HTF bars should only update HTF regime");
            }

            let out = runner
                .on_closed_kline(
                    ClosedKlineEvent {
                        venue: Venue::Spot,
                        symbol: sym.clone(),
                        timeframe: Timeframe::H1,
                        kline: bar.clone(),
                    },
                    None,
                    now,
                )
                .await;
            if out.tick_sent {
                total_ticks += 1;
            }
            total_signals += out.signals_generated;
            total_submitted += out.entries_submitted;
        }

        let final_now = bars.last().expect("last bar").close_time_ms + 1000;
        assert_eq!(
            total_ticks, 720,
            "every closed signal bar must forward MarketTick"
        );
        assert!(
            total_signals >= 1,
            "dataset must generate at least one strategy signal"
        );
        assert!(
            total_submitted >= 1,
            "generated strategy signal must be submitted through EngineActor into PaperBroker"
        );

        let snap = handle
            .full_snapshot(final_now)
            .await
            .expect("full snapshot");
        assert!(
            !snap.positions.is_empty() || !snap.orders.is_empty(),
            "EngineActor must record submitted entry order/position"
        );

        let _ = handle.shutdown(final_now + 1000).await;
        let _ = join.await;
    }

    #[tokio::test]
    async fn market_runner_task_exits_cleanly_on_shutdown_signal() {
        let cfg = BotConfig::default();
        let store = SqliteStore::open_in_memory().expect("in-memory sqlite");
        let broker = Arc::new(PaperBroker::new(PaperBrokerConfig::default()));
        broker.activate().await.expect("activate paper broker");
        let event_rx = broker.take_event_rx().expect("event rx");

        let actor = EngineActor::bootstrap(
            cfg.clone(),
            "b01".to_owned(),
            store,
            broker,
            Decimal::from(10_000_u32),
            1_700_000_000_000,
        )
        .await
        .expect("bootstrap engine");
        let (handle, join) = actor.spawn(event_rx);

        let (shutdown_tx, shutdown_rx) = watch::channel(false);
        let runner_handle =
            spawn_daemon_market_runner(&cfg, Venue::Spot, handle.clone(), shutdown_rx);

        let _ = shutdown_tx.send(true);
        tokio::time::timeout(Duration::from_secs(2), runner_handle.shutdown())
            .await
            .expect("market runner shutdown must complete promptly");

        let _ = handle.shutdown(1_700_000_001_000).await;
        let _ = join.await;
    }
}
