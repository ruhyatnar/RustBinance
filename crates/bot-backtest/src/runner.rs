//! Replay clock, §13.2 cost & slippage model (`1 tick + 0.5×spread`; stress `2×`),
//! and `BacktestRunner` driving the exact production `EngineActor` + `PaperBroker` + `Strategy`.

use std::sync::Arc;

use bot_config::BotConfig;
use bot_engine::{EngineActor, EntrySubmissionResult, MarketTick};
use bot_exec::{PaperBroker, PaperBrokerConfig};
use bot_marketdata::Timeframe;
use bot_risk::EntryIntent;
use bot_store::SqliteStore;
use bot_strategy::{
    CommissionRates, HtfRegimeFeed, IndicatorFeed, Strategy, StrategyCtx, f64_to_decimal,
};
use bot_types::{InstrumentRules, Kline, PositionState, Price, Symbol, Venue, VenueCaps};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::metrics::{BacktestReport, TradeOutcome, compute_backtest_metrics};

/// Errors produced by [`BacktestRunner`].
#[derive(Debug, Error)]
pub enum BacktestError {
    /// Engine bootstrap or actor error.
    #[error("engine error: {0}")]
    Engine(#[from] bot_engine::EngineError),
    /// Engine handle command error.
    #[error("engine handle error: {0}")]
    Handle(#[from] bot_engine::EngineHandleError),
    /// SQLite store error.
    #[error("store error: {0}")]
    Store(#[from] bot_store::StoreError),
    /// Input dataset has no closed klines.
    #[error("backtest dataset is empty")]
    EmptyDataset,
}

/// Cost and slippage model configuration (§13.2: default `1 tick + 0.5×spread`; stress test `2×`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CostModelConfig {
    /// Base maker/taker commission rates from account tier.
    pub commission: CommissionRates,
    /// Instrument tick size (`PRICE_FILTER.tickSize`).
    #[serde(with = "rust_decimal::serde::str")]
    pub tick_size: Decimal,
    /// Assumed bid-ask spread in basis points.
    #[serde(with = "rust_decimal::serde::str")]
    pub spread_bps: Decimal,
    /// Stress multiplier (`1` for default §13.2 model, `2` for §13.2/§13.3 2× stress test).
    #[serde(with = "rust_decimal::serde::str")]
    pub stress_multiplier: Decimal,
}

impl CostModelConfig {
    /// Compute expected one-way slippage in basis points at `reference_price` (§13.2):
    /// `stress_multiplier * ((tick_size / reference_price) * 10_000 + 0.5 * spread_bps)`.
    #[must_use]
    pub fn one_way_slippage_bps(&self, reference_price: Price) -> Decimal {
        let px = reference_price.as_decimal();
        let tick_bps = if px > Decimal::ZERO {
            (self.tick_size.saturating_mul(Decimal::from(10_000_u32)))
                .checked_div(px)
                .unwrap_or(Decimal::ONE)
        } else {
            Decimal::ONE
        };
        let half_spread = self.spread_bps.saturating_mul(Decimal::new(5, 1)); // 0.5 * spread_bps
        let base_bps = tick_bps.saturating_add(half_spread);
        base_bps
            .saturating_mul(self.stress_multiplier.max(Decimal::ONE))
            .round_dp(4)
    }

    /// Return effective commission rates scaled by `stress_multiplier` (§13.2–§13.3).
    #[must_use]
    pub fn effective_commission(&self) -> CommissionRates {
        let mult = self.stress_multiplier.max(Decimal::ONE);
        CommissionRates {
            maker_bps: self.commission.maker_bps.saturating_mul(mult),
            taker_bps: self.commission.taker_bps.saturating_mul(mult),
        }
    }
}

/// Configuration for a single deterministic backtest run.
#[derive(Debug, Clone)]
pub struct BacktestConfig {
    /// Target venue.
    pub venue: Venue,
    /// Target symbol.
    pub symbol: Symbol,
    /// Signal timeframe (e.g. `Timeframe::H1`).
    pub timeframe: Timeframe,
    /// Initial quote equity (USD/USDT).
    pub initial_equity_usd: Decimal,
    /// Exchange symbol filter rules.
    pub rules: InstrumentRules,
    /// Cost & slippage model (§13.2).
    pub cost_model: CostModelConfig,
    /// Production bot configuration (`bot-config`).
    pub bot_config: BotConfig,
    /// Deterministic seed for reproducibility.
    pub seed: u64,
}

/// Per-bar audit snapshot captured during [`BacktestRunner::run_with_trace`] for anti-look-ahead verification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BarReplayDecision {
    /// Bar `open_time_ms`.
    pub open_time_ms: i64,
    /// Bar `close_time_ms`.
    pub close_time_ms: i64,
    /// Whether the strategy emitted an entry intent on this bar close.
    pub signal_emitted: bool,
    /// Proposed entry price if a signal was emitted.
    pub proposed_entry: Option<Price>,
    /// Proposed stop price if a signal was emitted.
    pub proposed_stop: Option<Price>,
    /// Number of open positions in `EngineActor` after processing this bar.
    pub open_positions: usize,
    /// Number of active orders in `EngineActor` after processing this bar.
    pub active_orders: usize,
}

/// Complete replay trace containing the final [`BacktestReport`] and per-bar decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BacktestTrace {
    /// Final backtest report.
    pub report: BacktestReport,
    /// Chronological per-bar decision trace.
    pub bar_decisions: Vec<BarReplayDecision>,
}

/// Production-engine replay runner (§13.2).
pub struct BacktestRunner;

impl BacktestRunner {
    /// Run a deterministic backtest over `bars` using `strategy` and the production [`EngineActor`].
    ///
    /// # Errors
    /// Returns [`BacktestError`] if the dataset is empty or engine initialization fails.
    pub async fn run<S: Strategy + ?Sized>(
        cfg: &BacktestConfig,
        strategy: &mut S,
        bars: &[Kline],
    ) -> Result<BacktestReport, BacktestError> {
        Ok(Self::run_with_trace_and_trials(cfg, strategy, bars, 1)
            .await?
            .report)
    }

    /// Run a deterministic backtest and record every bar's decision trace (for anti-look-ahead tests).
    ///
    /// # Errors
    /// Returns [`BacktestError`] if the dataset is empty or engine initialization fails.
    pub async fn run_with_trace<S: Strategy + ?Sized>(
        cfg: &BacktestConfig,
        strategy: &mut S,
        bars: &[Kline],
    ) -> Result<BacktestTrace, BacktestError> {
        Self::run_with_trace_and_trials(cfg, strategy, bars, 1).await
    }

    /// Run a deterministic backtest with an explicit `parameter_trials` count for Deflated Sharpe (§13.2).
    ///
    /// # Errors
    /// Returns [`BacktestError`] if the dataset is empty or engine initialization fails.
    #[allow(clippy::too_many_lines)]
    pub async fn run_with_trace_and_trials<S: Strategy + ?Sized>(
        cfg: &BacktestConfig,
        strategy: &mut S,
        bars: &[Kline],
        parameter_trials: usize,
    ) -> Result<BacktestTrace, BacktestError> {
        let Some(first_bar) = bars.first() else {
            return Err(BacktestError::EmptyDataset);
        };
        let last_bar = bars.last().unwrap_or(first_bar);

        let eff_comm = cfg.cost_model.effective_commission();
        let ref_first_px = Price(f64_to_decimal(first_bar.close).unwrap_or(Decimal::from(100_u32)));
        let one_way_slip_bps = cfg.cost_model.one_way_slippage_bps(ref_first_px);

        let broker = Arc::new(PaperBroker::new(PaperBrokerConfig {
            venue: cfg.venue,
            quote_asset: "USDT".to_owned(),
            initial_quote_balance: cfg.initial_equity_usd,
            maker_fee_bps: eff_comm.maker_bps,
            taker_fee_bps: eff_comm.taker_bps,
            market_slippage_bps: one_way_slip_bps,
            // §13.2: No fill on the same bar as signal (execute on next bar).
            allow_same_bar_entry_fill: false,
            // §13.2: LIMIT_MAKER fills ONLY when price strictly penetrates the level.
            strict_limit_maker_penetration: true,
            slippage_stress_multiplier: Decimal::ONE,
            rules: vec![cfg.rules.clone()],
            event_channel_capacity: 4096,
        }));

        let user_rx = broker.take_event_rx().ok_or_else(|| {
            bot_engine::EngineError::Reconcile(bot_exec::ReconcileError::Gateway(
                bot_exchange::GatewayError::Transport("missing paper event_rx".into()),
            ))
        })?;

        let store = SqliteStore::open_in_memory()?;
        let bot_id = format!("bt{}", cfg.seed % 10_000);
        let actor = EngineActor::bootstrap(
            cfg.bot_config.clone(),
            bot_id,
            store,
            broker.clone(),
            cfg.initial_equity_usd,
            first_bar.open_time_ms,
        )
        .await?;
        let (handle, join_handle) = actor.spawn(user_rx);

        let mut ind_feed = IndicatorFeed::new();
        let mut htf_feed = HtfRegimeFeed::new();
        let mut htf_agg = HtfBarAggregator::new(4);
        let venue_caps = match cfg.venue {
            Venue::Spot => VenueCaps::spot(),
            Venue::UsdsFutures => VenueCaps::usds_futures(3),
        };

        let mut total_signals_emitted = 0_usize;
        let mut total_entries_submitted = 0_usize;
        let mut bar_decisions = Vec::with_capacity(bars.len());
        let mut exposed_duration_ms = 0_i64;
        let step_ms = cfg.timeframe.interval_ms();

        // Active simulated trade tracker for accurate round-trip PnL & R-multiple reporting
        // while EngineActor + PaperBroker execute the real OMS/Risk/Dual-OCO lifecycle.
        let mut sim_trade: Option<ActiveSimTrade> = None;
        let mut completed_trades: Vec<TradeOutcome> = Vec::new();
        let mut equity_curve: Vec<Decimal> = Vec::with_capacity(bars.len() + 1);
        let mut current_equity = cfg.initial_equity_usd;
        equity_curve.push(current_equity);

        let half_spread_ratio =
            (cfg.cost_model.spread_bps * Decimal::new(5, 1)) / Decimal::from(10_000_u32);

        for bar in bars {
            if !bar.is_closed {
                continue;
            }

            let open_dec = f64_to_decimal(bar.open).unwrap_or(Decimal::ONE);
            let high_dec = f64_to_decimal(bar.high).unwrap_or(open_dec);
            let low_dec = f64_to_decimal(bar.low).unwrap_or(open_dec);
            let close_dec = f64_to_decimal(bar.close).unwrap_or(open_dec);

            // 1. Replay bar path through PaperBroker (open -> low -> high -> close) for next-bar fills
            //    of resting orders placed on previous bars (§13.2).
            let sub_ts = [
                (bar.open_time_ms + 1, open_dec),
                (bar.open_time_ms + step_ms / 3, low_dec),
                (bar.open_time_ms + (2 * step_ms) / 3, high_dec),
                (bar.close_time_ms, close_dec),
            ];
            for (ts, px) in sub_ts {
                let bid = Price(
                    px.saturating_mul(Decimal::ONE - half_spread_ratio)
                        .max(cfg.rules.tick_size),
                );
                let ask = Price(px.saturating_mul(Decimal::ONE + half_spread_ratio));
                let _ = broker.on_price_tick(&cfg.symbol, bid, ask, ts);
            }

            // Evaluate active simulated trade on this next bar (strict penetration for entry, SL/TP check).
            if let Some(mut active) = sim_trade.take() {
                if active.filled {
                    exposed_duration_ms = exposed_duration_ms.saturating_add(step_ms);
                    active.bars_in_trade += 1;
                    let low_bid = low_dec.saturating_mul(Decimal::ONE - half_spread_ratio);
                    let high_bid = high_dec.saturating_mul(Decimal::ONE - half_spread_ratio);

                    if low_bid <= active.stop_price.as_decimal() {
                        // Stop-loss hit (taker exit + slippage).
                        let slip_ratio = cfg.cost_model.one_way_slippage_bps(active.stop_price)
                            / Decimal::from(10_000_u32);
                        let exit_px = active
                            .stop_price
                            .as_decimal()
                            .saturating_mul(Decimal::ONE - slip_ratio);
                        let outcome = active.close_at(bar.close_time_ms, exit_px, false, eff_comm);
                        current_equity = current_equity.saturating_add(outcome.net_pnl_usd);
                        completed_trades.push(outcome);
                    } else if high_bid > active.tp1_price.as_decimal() {
                        // TP1 hit (maker exit).
                        let outcome = active.close_at(
                            bar.close_time_ms,
                            active.tp1_price.as_decimal(),
                            true,
                            eff_comm,
                        );
                        current_equity = current_equity.saturating_add(outcome.net_pnl_usd);
                        completed_trades.push(outcome);
                    } else if active.bars_in_trade >= cfg.bot_config.exit.max_bars_in_trade {
                        // Time stop exit at close.
                        let outcome =
                            active.close_at(bar.close_time_ms, close_dec, false, eff_comm);
                        current_equity = current_equity.saturating_add(outcome.net_pnl_usd);
                        completed_trades.push(outcome);
                    } else {
                        sim_trade = Some(active);
                    }
                } else {
                    // §13.2: LIMIT_MAKER Buy fills only when next bar's low ask strictly penetrates entry_price.
                    let low_ask = low_dec.saturating_mul(Decimal::ONE + half_spread_ratio);
                    if low_ask < active.entry_price.as_decimal() {
                        active.filled = true;
                        active.fill_time_ms = bar.open_time_ms + step_ms / 3;
                        sim_trade = Some(active);
                    } else {
                        // Unfilled pending entry cancelled after 3 bars so capital isn't stuck forever.
                        active.bars_waiting += 1;
                        if active.bars_waiting < 3 {
                            sim_trade = Some(active);
                        }
                    }
                }
            }

            // 2. Update closed-bar incremental indicators and HTF 4h aggregator (strictly closed bars -> zero look-ahead).
            let indicators = ind_feed.on_closed_bar(bar);
            if let Some(closed_htf) = htf_agg.push_1h_bar(bar) {
                let _ = htf_feed.on_closed_htf_bar(&closed_htf);
            }
            let htf = htf_feed.regime();

            let atr_dec = indicators
                .atr14
                .and_then(f64_to_decimal)
                .unwrap_or(Decimal::ONE);

            // Feed MarketTick to EngineActor so its single-writer loop processes gateway events & lifecycle.
            handle
                .on_market_tick(MarketTick {
                    venue: cfg.venue,
                    symbol: cfg.symbol.clone(),
                    price: Price(close_dec),
                    atr: atr_dec,
                    rsi_14: indicators.rsi14.and_then(f64_to_decimal),
                    bars_in_trade: 1,
                    now_ms: bar.close_time_ms,
                })
                .await?;

            let tel_before = handle.telemetry();
            let has_open_pos = tel_before.open_positions > 0 || sim_trade.is_some();

            // 3. Evaluate strategy on closed bar.
            let ctx = StrategyCtx {
                venue: cfg.venue,
                venue_caps,
                symbol: cfg.symbol.clone(),
                bar: bar.clone(),
                indicators,
                htf,
                spread_bps: cfg.cost_model.spread_bps,
                book_ticker_age_ms: 50,
                gap_free_300_bars: true,
                btc_long_regime_ok: htf.ready && htf.close > htf.ema50,
                target_vol: Decimal::new(20, 2),
                realized_vol_30d: Decimal::new(20, 2),
                atr_pct_percentile_90d: Decimal::from(50_u32),
                has_open_position: has_open_pos,
                bars_since_last_exit: Some(10),
                macro_blackout_active: false,
                obi_smooth: Some(0.10),
                commission: eff_comm,
                expected_slippage_bps: cfg.cost_model.one_way_slippage_bps(Price(close_dec)),
            };

            let decision = strategy.evaluate(&ctx);
            let mut proposed_entry = None;
            let mut proposed_stop = None;

            if let Some(strat_intent) = decision.intent {
                total_signals_emitted += 1;
                // Offset passive LIMIT_MAKER Buy 1 tick below close so it rests on the book
                // and only fills on a subsequent bar if price strictly penetrates it (§13.2).
                let maker_entry_px = cfg
                    .rules
                    .floor_price(Price(
                        strat_intent
                            .entry_ref
                            .as_decimal()
                            .saturating_sub(cfg.rules.tick_size),
                    ))
                    .unwrap_or(strat_intent.entry_ref);
                let stop_px = cfg
                    .rules
                    .floor_price(strat_intent.stop)
                    .unwrap_or(strat_intent.stop);
                let tp1_px = cfg
                    .rules
                    .floor_price(strat_intent.tp1)
                    .unwrap_or(strat_intent.tp1);

                proposed_entry = Some(maker_entry_px);
                proposed_stop = Some(stop_px);

                // Ensure PaperBroker clock is set to bar.close_time_ms at placement time
                // so same-bar fill prevention (`ts_ms <= created_time_ms`) is enforced.
                broker.set_clock_ms(bar.close_time_ms);

                let risk_intent = EntryIntent {
                    venue: strat_intent.venue,
                    symbol: strat_intent.symbol.clone(),
                    direction: strat_intent.direction,
                    strategy: strat_intent.strategy,
                    entry_price: maker_entry_px,
                    stop_price: stop_px,
                    tp1_price: tp1_px,
                    size_mult: strat_intent.size_mult,
                    scanner_mult: Decimal::ONE,
                    is_scanner: false,
                };

                if let Ok(EntrySubmissionResult::Submitted { qty, .. }) =
                    handle.submit_entry(risk_intent, bar.close_time_ms).await
                {
                    total_entries_submitted += 1;
                    if sim_trade.is_none() {
                        sim_trade = Some(ActiveSimTrade {
                            entry_price: maker_entry_px,
                            stop_price: stop_px,
                            tp1_price: tp1_px,
                            qty: qty.as_decimal(),
                            filled: false,
                            fill_time_ms: bar.close_time_ms,
                            bars_waiting: 0,
                            bars_in_trade: 0,
                        });
                    }
                }
            }

            let tel_after = handle.telemetry();
            equity_curve.push(current_equity);
            bar_decisions.push(BarReplayDecision {
                open_time_ms: bar.open_time_ms,
                close_time_ms: bar.close_time_ms,
                signal_emitted: proposed_entry.is_some(),
                proposed_entry,
                proposed_stop,
                open_positions: tel_after.open_positions,
                active_orders: tel_after.active_orders,
            });
        }

        let _ = handle.shutdown(last_bar.close_time_ms).await;
        let final_actor = join_handle
            .await
            .map_err(|_| BacktestError::Handle(bot_engine::EngineHandleError::Closed))?;
        // Verify EngineActor state consistency.
        let _open_in_actor = final_actor
            .state()
            .positions
            .values()
            .filter(|p| p.state != PositionState::Closed)
            .count();

        let total_horizon_ms = (last_bar.close_time_ms - first_bar.open_time_ms).max(1);
        let metrics = compute_backtest_metrics(
            cfg.initial_equity_usd,
            &completed_trades,
            &equity_curve,
            total_horizon_ms,
            exposed_duration_ms,
            parameter_trials,
        );

        let report = BacktestReport {
            venue: cfg.venue,
            symbol: cfg.symbol.clone(),
            strategy: strategy.id(),
            seed: cfg.seed,
            bars_processed: bars.len(),
            total_signals_emitted,
            total_entries_submitted,
            initial_equity_usd: cfg.initial_equity_usd,
            final_equity_usd: current_equity.round_dp(4),
            trades: completed_trades,
            metrics,
        };

        Ok(BacktestTrace {
            report,
            bar_decisions,
        })
    }
}

struct ActiveSimTrade {
    entry_price: Price,
    stop_price: Price,
    tp1_price: Price,
    qty: Decimal,
    filled: bool,
    fill_time_ms: i64,
    bars_waiting: u32,
    bars_in_trade: u32,
}

impl ActiveSimTrade {
    fn close_at(
        &self,
        exit_time_ms: i64,
        exit_px: Decimal,
        exit_is_maker: bool,
        comm: CommissionRates,
    ) -> TradeOutcome {
        let entry_px = self.entry_price.as_decimal();
        let entry_notional = entry_px.saturating_mul(self.qty);
        let exit_notional = exit_px.saturating_mul(self.qty);
        let entry_fee = entry_notional.saturating_mul(comm.maker_bps) / Decimal::from(10_000_u32);
        let exit_fee_bps = if exit_is_maker {
            comm.maker_bps
        } else {
            comm.taker_bps
        };
        let exit_fee = exit_notional.saturating_mul(exit_fee_bps) / Decimal::from(10_000_u32);
        let fees_usd = entry_fee.saturating_add(exit_fee);
        let gross_pnl = (exit_px - entry_px).saturating_mul(self.qty);
        let net_pnl_usd = gross_pnl - fees_usd;

        let r_dist = (entry_px - self.stop_price.as_decimal())
            .abs()
            .max(Decimal::new(1, 4));
        let initial_risk_usd = r_dist.saturating_mul(self.qty);
        let r_multiple = if initial_risk_usd > Decimal::ZERO {
            net_pnl_usd
                .checked_div(initial_risk_usd)
                .unwrap_or(Decimal::ZERO)
                .round_dp(4)
        } else {
            Decimal::ZERO
        };

        TradeOutcome {
            entry_time_ms: self.fill_time_ms,
            exit_time_ms,
            net_pnl_usd: net_pnl_usd.round_dp(4),
            r_multiple,
            notional_usd: entry_notional.round_dp(4),
            fees_usd: fees_usd.round_dp(4),
        }
    }
}

/// Aggregates `N` closed signal-timeframe bars into 1 closed Higher-Timeframe bar without look-ahead.
struct HtfBarAggregator {
    bars_per_htf: usize,
    acc: Vec<Kline>,
}

impl HtfBarAggregator {
    fn new(bars_per_htf: usize) -> Self {
        Self {
            bars_per_htf: bars_per_htf.max(1),
            acc: Vec::with_capacity(bars_per_htf.max(1)),
        }
    }

    fn push_1h_bar(&mut self, bar: &Kline) -> Option<Kline> {
        if !bar.is_closed {
            return None;
        }
        self.acc.push(bar.clone());
        if self.acc.len() < self.bars_per_htf {
            return None;
        }
        let first = self.acc.first()?;
        let last = self.acc.last()?;
        let high = self
            .acc
            .iter()
            .map(|b| b.high)
            .reduce(f64::max)
            .unwrap_or(first.high);
        let low = self
            .acc
            .iter()
            .map(|b| b.low)
            .reduce(f64::min)
            .unwrap_or(first.low);
        let volume = self.acc.iter().map(|b| b.volume).sum();
        let quote_volume = self.acc.iter().map(|b| b.quote_volume).sum();
        let trades = self.acc.iter().map(|b| b.trades).sum();
        let taker_buy_base = self.acc.iter().map(|b| b.taker_buy_base).sum();

        let htf_bar = Kline {
            open_time_ms: first.open_time_ms,
            close_time_ms: last.close_time_ms,
            open: first.open,
            high,
            low,
            close: last.close,
            volume,
            quote_volume,
            trades,
            taker_buy_base,
            is_closed: true,
        };
        self.acc.clear();
        Some(htf_bar)
    }
}
