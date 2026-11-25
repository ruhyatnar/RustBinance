//! Single-writer Engine Actor (§3.2–§3.3), Gatekeeper integration, Dual-OCO lifecycle,
//! crash recovery, and SIGTERM graceful shutdown preserving exchange-side protection (M07 g–h).

use std::collections::BTreeMap;
use std::sync::Arc;

use bot_config::BotConfig;
use bot_exchange::{
    CancelAlgoOrderRequest, CancelOrderListRequest, CancelOrderRequest, GatewayError, GatewayEvent,
    PlaceOrderRequest, TimeInForce, VenueAdapter,
};
use bot_exec::futures_protection::{AlgoProtectionManager, AlgoProtectionState};
use bot_exec::{
    DualOcoManager, FsmApplyOutcome, HardStopWatcher, LifecycleAction, OrderFsm, PositionTickInput,
    ReconcileConfig, ReconcileReport, evaluate_position_tick, format_client_order_id, is_terminal,
    reconcile_venue,
};
use bot_risk::{
    BtcCrashGuard, BtcMarketSample, BtcSpikeGuard, BtcSpikeSample, CombinedEquityCounters,
    CrossVenueBudget, CrossVenuePosition, CrossVenueSameSymbolPolicy, EngineModeMachine,
    EntryIntent, EquityCheckOutcome, GateDecision, Gatekeeper, GatekeeperContext, ModeTrigger,
    RejectReason,
};
use bot_store::{
    JournalEnvelope, JournalEvent, PersistedEngineState, SqliteStore, VenueStateRecord,
};
use bot_types::{
    Direction, EngineMode, InstrumentRules, Order, OrderKind, OrderPurpose, OrderStatus, Position,
    PositionState, Price, Qty, Symbol, Venue, VenueState,
};
use bot_venue::{SwitchConfig, SwitchPolicy, SwitchPrecheckContext, VenueManager};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use time::OffsetDateTime;
use tokio::sync::{mpsc, oneshot, watch};
use tokio::task::JoinHandle;
use tracing::{info, warn};
use uuid::Uuid;

use crate::helpers::{
    compute_gross_realized_pnl, compute_tp2_price, disable_strategy_in_config,
    entry_side_for_direction, exit_side_for_direction, tighten_runtime_param,
};

/// Bounded command channel capacity (§3.3: 64, full → reject `Busy`).
pub const COMMAND_CHANNEL_CAPACITY: usize = 64;
/// Bounded user-data event channel capacity (§3.3: 4096, never drop).
pub const USER_EVENT_CHANNEL_CAPACITY: usize = 4096;

/// Errors returned by [`EngineHandle`] when sending commands to [`EngineActor`].
#[derive(Debug, Error)]
pub enum EngineHandleError {
    /// Command channel is full (64 pending commands per §3.3).
    #[error("engine command queue busy")]
    Busy,
    /// Engine actor task has exited.
    #[error("engine actor closed")]
    Closed,
}

/// Errors returned by [`EngineActor`] operations.
#[derive(Debug, Error)]
pub enum EngineError {
    /// SQLite persistence failure (triggers `EngineMode::Halted`).
    #[error("store error: {0}")]
    Store(#[from] bot_store::StoreError),
    /// Exchange gateway error.
    #[error("gateway error: {0}")]
    Gateway(#[from] GatewayError),
    /// Protection installation error.
    #[error("protection error: {0}")]
    Protection(#[from] bot_exec::ProtectionError),
    /// Reconciliation error.
    #[error("reconcile error: {0}")]
    Reconcile(#[from] bot_exec::ReconcileError),
    /// Client order ID generation error.
    #[error("client order id error: {0}")]
    ClientId(#[from] bot_exec::ClientIdError),
}

/// Real-time operational health metrics fed into [`EngineActor`] and [`Gatekeeper`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeHealth {
    /// Age of latest market data tick (ms).
    pub market_data_age_ms: i64,
    /// Whether the active user-data event stream is connected and healthy.
    pub user_stream_healthy: bool,
    /// Absolute local-vs-server clock drift (ms).
    pub clock_drift_ms: i64,
    /// Observed WS round-trip latency (ms).
    pub ws_latency_ms: u32,
    /// Current `REQUEST_WEIGHT` utilization (`0..=100`).
    #[serde(with = "rust_decimal::serde::str")]
    pub request_weight_pct: Decimal,
    /// Current `ORDERS` (10 s) utilization (`0..=100`).
    #[serde(with = "rust_decimal::serde::str")]
    pub orders_10s_pct: Decimal,
}

impl Default for RuntimeHealth {
    fn default() -> Self {
        Self {
            market_data_age_ms: 0,
            user_stream_healthy: true,
            clock_drift_ms: 0,
            ws_latency_ms: 0,
            request_weight_pct: Decimal::ZERO,
            orders_10s_pct: Decimal::ZERO,
        }
    }
}

/// Conflated telemetry snapshot published over `watch` (§3.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TelemetrySnapshot {
    /// Current global engine mode.
    pub mode: EngineMode,
    /// Active venue.
    pub venue: Venue,
    /// Current Spot venue lifecycle state.
    pub spot_state: VenueState,
    /// Current USDⓈ-M Futures venue lifecycle state.
    pub futures_state: VenueState,
    /// Number of currently open positions.
    pub open_positions: usize,
    /// Number of currently active (non-terminal) orders.
    pub active_orders: usize,
    /// Combined global equity (USD).
    #[serde(with = "rust_decimal::serde::str")]
    pub equity_total_usd: Decimal,
    /// Highest applied journal sequence number.
    pub last_seq: i64,
}

/// Complete live snapshot returned by [`EngineHandle::full_snapshot`] for IPC/UI consumers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineFullSnapshot {
    /// Base telemetry counters and venue states.
    pub telemetry: TelemetrySnapshot,
    /// Daemon start timestamp (UTC ms).
    pub started_at_ms: i64,
    /// Last venue switch timestamp (UTC ms), or 0 if never switched.
    pub last_switch_ms: i64,
    /// Number of venue switches performed today (UTC).
    pub switches_today: u32,
    /// Latest runtime health observations.
    pub health: RuntimeHealth,
    /// Daily `PnL` percentage vs UTC 00:00 equity.
    pub day_pnl_pct: Decimal,
    /// Weekly `PnL` percentage vs Monday UTC 00:00 equity.
    pub week_pnl_pct: Decimal,
    /// Peak-to-trough drawdown percentage.
    pub drawdown_pct: Decimal,
    /// Combined cross-venue portfolio heat percentage.
    pub heat_pct: Decimal,
    /// Active BTC guard status string (`OK`, `CRASH_GUARD`, `SPIKE_GUARD`).
    pub btc_guard: String,
    /// Currently open (non-closed) positions across all venues.
    pub positions: Vec<Position>,
    /// Currently active (non-terminal) orders across all venues.
    pub orders: Vec<Order>,
    /// Current live risk limits (may have been tightened at runtime).
    pub config: BotConfig,
}

/// Summary returned after a completed runtime venue switch (§3.5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueSwitchReport {
    /// Previous active venue.
    pub from: Venue,
    /// Newly active venue.
    pub to: Venue,
    /// Resulting state of the previous venue (`Draining` or `Disabled`).
    pub old_venue_state: VenueState,
    /// Number of unfilled entry orders canceled on the old venue (protections preserved).
    pub canceled_entry_orders: usize,
}

/// Market price tick delivered to [`EngineActor`] for position lifecycle evaluation.
#[derive(Debug, Clone)]
pub struct MarketTick {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Current mark/bid price.
    pub price: Price,
    /// Current ATR in price units.
    pub atr: Decimal,
    /// Optional RSI(14).
    pub rsi_14: Option<Decimal>,
    /// Bars elapsed in trade.
    pub bars_in_trade: u32,
    /// UTC timestamp (ms).
    pub now_ms: i64,
}

/// Result of submitting an [`EntryIntent`] through [`EngineActor`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntrySubmissionResult {
    /// Passed Gatekeeper, journaled `OrderIntended`, and submitted to exchange.
    Submitted {
        /// Deterministic `clientOrderId`.
        client_order_id: String,
        /// Allocated position UUID.
        position_id: Uuid,
        /// Exchange-valid quantity.
        qty: Qty,
    },
    /// Rejected by `bot-risk::Gatekeeper`.
    Rejected(GateDecision),
    /// Exchange submission failed after journaling.
    GatewayFailed(String),
}

/// Commands accepted by [`EngineActor`] over its bounded `mpsc(64)` channel.
pub enum EngineCommand {
    /// Evaluate `EntryIntent` through `Gatekeeper`, journal `OrderIntended`, and place entry order.
    SubmitEntry {
        /// Strategy entry intent.
        intent: EntryIntent,
        /// Current UTC ms.
        now_ms: i64,
        /// Reply channel.
        reply: oneshot::Sender<EntrySubmissionResult>,
    },
    /// Feed a market tick to evaluate open positions (§5.5).
    MarketTick {
        /// Tick payload.
        tick: MarketTick,
        /// Completion reply.
        reply: oneshot::Sender<()>,
    },
    /// Run startup/periodic reconciliation against exchange truth (§8.3).
    Reconcile {
        /// Current UTC ms.
        now_ms: i64,
        /// Reply channel.
        reply: oneshot::Sender<Result<ReconcileReport, String>>,
    },
    /// Apply a manual or operational [`ModeTrigger`] (Pause, Resume, Panic, `ResetKill`).
    ApplyModeTrigger {
        /// Trigger event.
        trigger: ModeTrigger,
        /// Current UTC ms.
        now_ms: i64,
        /// Reply with resulting [`EngineMode`].
        reply: oneshot::Sender<EngineMode>,
    },
    /// Emergency panic flatten: cancel all pending entries, market-close all open positions
    /// (when `safety.kill_flatten` is enabled), and latch `EngineMode::Killed`.
    PanicFlatten {
        /// Current UTC ms.
        now_ms: i64,
        /// Reply with `(resulting_mode, flattened_positions_count)`.
        reply: oneshot::Sender<(EngineMode, usize)>,
    },
    /// Manually close a single open position at market price.
    ClosePosition {
        /// Position UUID.
        position_id: Uuid,
        /// Current UTC ms.
        now_ms: i64,
        /// Reply channel.
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Tighten-only runtime risk parameter mutation (Hard Rule #8).
    SetParam {
        /// Dotted parameter key (e.g. `risk.max_open_positions`).
        key: String,
        /// New value string.
        value: String,
        /// Reply channel.
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Disable a strategy at runtime.
    DisableStrategy {
        /// Strategy identifier (`trend_pullback`, `breakout`, etc.).
        id: String,
        /// Reply channel.
        reply: oneshot::Sender<Result<String, String>>,
    },
    /// Update live operational health and optional BTC crash/spike samples.
    UpdateHealth {
        /// Updated health metrics.
        health: RuntimeHealth,
        /// Optional BTC crash sample (Long guard).
        btc_crash: Option<BtcMarketSample>,
        /// Optional BTC spike sample (Short guard).
        btc_spike: Option<BtcSpikeSample>,
        /// Current UTC ms.
        now_ms: i64,
        /// Completion reply.
        reply: oneshot::Sender<()>,
    },
    /// Request a full live snapshot (positions, orders, risk counters, venue state).
    GetFullSnapshot {
        /// Current UTC ms.
        now_ms: i64,
        /// Reply channel.
        reply: oneshot::Sender<EngineFullSnapshot>,
    },
    /// Persist a full SQLite snapshot immediately.
    SnapshotNow {
        /// Current UTC ms.
        now_ms: i64,
        /// Reply with snapshot row ID.
        reply: oneshot::Sender<Result<i64, String>>,
    },
    /// Atomically switch active venue (`Spot <-> UsdsFutures`, §3.5).
    SwitchVenue {
        /// Target venue to activate.
        target: Venue,
        /// Open-position handling policy on the old venue.
        policy: SwitchPolicy,
        /// Authenticated caller principal for audit journaling.
        actor: String,
        /// Current UTC ms.
        now_ms: i64,
        /// Reply channel.
        reply: oneshot::Sender<Result<VenueSwitchReport, String>>,
    },
    /// Graceful shutdown (`SIGTERM`): cancel ONLY pending entry orders, preserve all protections!
    Shutdown {
        /// Current UTC ms.
        now_ms: i64,
        /// Completion reply with number of canceled entry orders.
        reply: oneshot::Sender<usize>,
    },
}

/// Cheaply cloneable handle to communicate with a running [`EngineActor`].
#[derive(Clone)]
pub struct EngineHandle {
    cmd_tx: mpsc::Sender<EngineCommand>,
    telemetry_rx: watch::Receiver<TelemetrySnapshot>,
}

impl EngineHandle {
    /// Latest conflated [`TelemetrySnapshot`].
    #[must_use]
    pub fn telemetry(&self) -> TelemetrySnapshot {
        self.telemetry_rx.borrow().clone()
    }

    /// Subscribe to telemetry updates.
    #[must_use]
    pub fn telemetry_rx(&self) -> watch::Receiver<TelemetrySnapshot> {
        self.telemetry_rx.clone()
    }

    /// Non-blocking command send enforcing §3.3 backpressure (returns `Busy` if 64 queue is full).
    ///
    /// # Errors
    /// Returns [`EngineHandleError::Busy`] if queue is full or [`EngineHandleError::Closed`] if exited.
    pub fn try_send(&self, cmd: EngineCommand) -> Result<(), EngineHandleError> {
        self.cmd_tx.try_send(cmd).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => EngineHandleError::Busy,
            mpsc::error::TrySendError::Closed(_) => EngineHandleError::Closed,
        })
    }

    /// Submit an [`EntryIntent`] through `Gatekeeper` and OMS.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn submit_entry(
        &self,
        intent: EntryIntent,
        now_ms: i64,
    ) -> Result<EntrySubmissionResult, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::SubmitEntry {
            intent,
            now_ms,
            reply: tx,
        })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Feed a [`MarketTick`] and wait for single-writer processing.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn on_market_tick(&self, tick: MarketTick) -> Result<(), EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::MarketTick { tick, reply: tx })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Trigger graceful shutdown (cancels pending entries ONLY; preserves protective OCOs/Algos).
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the actor is already closed.
    pub async fn shutdown(&self, now_ms: i64) -> Result<usize, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.cmd_tx
            .send(EngineCommand::Shutdown { now_ms, reply: tx })
            .await
            .map_err(|_| EngineHandleError::Closed)?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Trigger a [`ModeTrigger`] on the engine mode machine.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn apply_mode_trigger(
        &self,
        trigger: ModeTrigger,
        now_ms: i64,
    ) -> Result<EngineMode, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::ApplyModeTrigger {
            trigger,
            now_ms,
            reply: tx,
        })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Emergency panic flatten: cancel pending entries, market-close all open positions, and latch `Killed`.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn panic_flatten(
        &self,
        now_ms: i64,
    ) -> Result<(EngineMode, usize), EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::PanicFlatten { now_ms, reply: tx })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Manually close a single open position by UUID.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn close_position(
        &self,
        position_id: Uuid,
        now_ms: i64,
    ) -> Result<Result<String, String>, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::ClosePosition {
            position_id,
            now_ms,
            reply: tx,
        })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Tighten a runtime risk parameter (Hard Rule #8).
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn set_param(
        &self,
        key: String,
        value: String,
    ) -> Result<Result<String, String>, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::SetParam {
            key,
            value,
            reply: tx,
        })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Disable a strategy at runtime.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn disable_strategy(
        &self,
        id: String,
    ) -> Result<Result<String, String>, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::DisableStrategy { id, reply: tx })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Feed updated operational health and optional BTC crash/spike samples into the engine.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn update_health(
        &self,
        health: RuntimeHealth,
        btc_crash: Option<BtcMarketSample>,
        btc_spike: Option<BtcSpikeSample>,
        now_ms: i64,
    ) -> Result<(), EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::UpdateHealth {
            health,
            btc_crash,
            btc_spike,
            now_ms,
            reply: tx,
        })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Query a complete live [`EngineFullSnapshot`] for IPC/UI consumers.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn full_snapshot(
        &self,
        now_ms: i64,
    ) -> Result<EngineFullSnapshot, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::GetFullSnapshot { now_ms, reply: tx })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Request an immediate SQLite snapshot.
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the queue is busy or actor closed.
    pub async fn snapshot_now(
        &self,
        now_ms: i64,
    ) -> Result<Result<i64, String>, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::SnapshotNow { now_ms, reply: tx })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }

    /// Request an atomic runtime venue switch (`Spot <-> UsdsFutures`, §3.5).
    ///
    /// # Errors
    /// Returns [`EngineHandleError`] if the bounded command queue is full or actor closed.
    pub async fn switch_venue(
        &self,
        target: Venue,
        policy: SwitchPolicy,
        actor: String,
        now_ms: i64,
    ) -> Result<Result<VenueSwitchReport, String>, EngineHandleError> {
        let (tx, rx) = oneshot::channel();
        self.try_send(EngineCommand::SwitchVenue {
            target,
            policy,
            actor,
            now_ms,
            reply: tx,
        })?;
        rx.await.map_err(|_| EngineHandleError::Closed)
    }
}

/// Single-writer trading Engine Actor (§3.2).
pub struct EngineActor {
    config: BotConfig,
    venue: Venue,
    venue_manager: VenueManager,
    secondary_adapters: BTreeMap<Venue, Arc<dyn VenueAdapter>>,
    bot_id: String,
    run_id: u64,
    started_at_ms: i64,
    store: SqliteStore,
    state: PersistedEngineState,
    adapter: Arc<dyn VenueAdapter>,
    gatekeeper: Gatekeeper,
    mode_machine: EngineModeMachine,
    counters: CombinedEquityCounters,
    venue_equities: BTreeMap<Venue, Decimal>,
    last_equity_outcome: EquityCheckOutcome,
    btc_guard: BtcCrashGuard,
    btc_spike_guard: BtcSpikeGuard,
    health: RuntimeHealth,
    last_market_tick_ms: Option<i64>,
    last_switch_ms: i64,
    fsm: OrderFsm,
    oco_mgr: DualOcoManager,
    futures_protections: BTreeMap<Uuid, AlgoProtectionState>,
    watcher: HardStopWatcher,
    rules_by_venue_symbol: BTreeMap<(Venue, Symbol), InstrumentRules>,
    ref_prices: BTreeMap<(Venue, Symbol), Price>,
    free_quote_usd: Decimal,
    telemetry_tx: watch::Sender<TelemetrySnapshot>,
}

impl EngineActor {
    /// Construct and recover an [`EngineActor`] from `store`, synchronizing `exchangeInfo`
    /// and running startup reconciliation (§8.3) before entering `EngineMode::Ready`.
    ///
    /// # Errors
    /// Returns [`EngineError`] if recovery, `exchangeInfo`, or reconciliation fails.
    #[allow(clippy::too_many_lines)]
    pub async fn bootstrap(
        config: BotConfig,
        bot_id: String,
        mut store: SqliteStore,
        adapter: Arc<dyn VenueAdapter>,
        initial_equity_usd: Decimal,
        now_ms: i64,
    ) -> Result<Self, EngineError> {
        let venue = adapter.venue();
        let run_id = store.next_run_id()?;
        let mut state = store.recover_state()?;
        let mut fsm = OrderFsm::from_recovered(state.orders.clone(), &state.fills);
        let mut oco_mgr = DualOcoManager::new();

        let mut mode_machine = EngineModeMachine::new(now_ms);
        mode_machine.apply(ModeTrigger::SyncStarted, now_ms);

        let info = adapter.exchange_info().await?;
        let mut rules_by_symbol = BTreeMap::new();
        let mut rules_by_venue_symbol = BTreeMap::new();
        for r in info.rules {
            rules_by_symbol.insert(r.symbol.clone(), r.clone());
            rules_by_venue_symbol.insert((venue, r.symbol.clone()), r);
        }

        let account = adapter.account_snapshot().await?;
        let quote_asset = config
            .general
            .quote_assets
            .first()
            .cloned()
            .unwrap_or_else(|| "USDT".to_owned());
        let free_quote_usd = account
            .balances
            .iter()
            .find(|b| b.asset.eq_ignore_ascii_case(&quote_asset))
            .map_or(initial_equity_usd, |b| b.free);

        let ref_prices_reconcile = BTreeMap::new();
        let _rep = reconcile_venue(
            adapter.as_ref(),
            &mut store,
            &mut state,
            &mut fsm,
            &mut oco_mgr,
            &rules_by_symbol,
            &ref_prices_reconcile,
            &ReconcileConfig {
                venue,
                quote_asset,
                orphan_policy: config.risk.orphan_policy,
                bot_id: bot_id.clone(),
                run_id,
                tp1_close_ratio: config.strategy.trend_pullback.tp1_close_ratio,
                adopt_stop_pct: Decimal::from(2_u32),
            },
            now_ms,
        )
        .await?;

        mode_machine.apply(ModeTrigger::SyncComplete, now_ms);
        let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
            mode: mode_machine.mode(),
            reason: "bootstrap reconciliation complete".into(),
        });
        let seq = store.append_event(now_ms, &env)?;
        state.apply_event(seq, &env);

        let mut counters = CombinedEquityCounters::new(venue, initial_equity_usd, now_ms);
        let last_equity_outcome =
            counters.update_venue_equity(venue, initial_equity_usd, now_ms, &config.risk);
        let mut venue_equities = BTreeMap::new();
        venue_equities.insert(venue, initial_equity_usd);
        let btc_guard =
            BtcCrashGuard::new(config.risk.btc_crash_15m_pct, config.risk.btc_crash_60m_pct);
        let btc_spike_guard = BtcSpikeGuard::new(
            config.risk.futures.btc_spike_15m_pct,
            config.risk.futures.btc_spike_60m_pct,
        );
        let gatekeeper = Gatekeeper::new(config.risk.clone());

        let venue_manager = VenueManager::new(venue, SwitchConfig::default());
        let initial_tel = TelemetrySnapshot {
            mode: mode_machine.mode(),
            venue,
            spot_state: venue_manager.state(Venue::Spot),
            futures_state: venue_manager.state(Venue::UsdsFutures),
            open_positions: state.positions.len(),
            active_orders: state.orders.len(),
            equity_total_usd: counters.combined_equity(),
            last_seq: state.last_seq,
        };
        let (telemetry_tx, _) = watch::channel(initial_tel);

        Ok(Self {
            config,
            venue,
            venue_manager,
            secondary_adapters: BTreeMap::new(),
            bot_id,
            run_id,
            started_at_ms: now_ms,
            store,
            state,
            adapter,
            gatekeeper,
            mode_machine,
            counters,
            venue_equities,
            last_equity_outcome,
            btc_guard,
            btc_spike_guard,
            health: RuntimeHealth::default(),
            last_market_tick_ms: None,
            last_switch_ms: 0,
            fsm,
            oco_mgr,
            futures_protections: BTreeMap::new(),
            watcher: HardStopWatcher::new(),
            rules_by_venue_symbol,
            ref_prices: BTreeMap::new(),
            free_quote_usd,
            telemetry_tx,
        })
    }

    /// Spawn the single-writer [`EngineActor`] loop and return `(EngineHandle, JoinHandle)`.
    #[must_use]
    #[allow(clippy::too_many_lines)]
    pub fn spawn(
        mut self,
        mut user_events_rx: mpsc::Receiver<GatewayEvent>,
    ) -> (EngineHandle, JoinHandle<Self>) {
        let (cmd_tx, mut cmd_rx) = mpsc::channel(COMMAND_CHANNEL_CAPACITY);
        let telemetry_rx = self.telemetry_tx.subscribe();
        let handle = EngineHandle {
            cmd_tx,
            telemetry_rx,
        };

        let task = tokio::spawn(async move {
            let mut events_closed = false;
            loop {
                tokio::select! {
                    biased;
                    maybe_ev = user_events_rx.recv(), if !events_closed => {
                        if let Some(ev) = maybe_ev {
                            self.handle_gateway_event(ev).await;
                            self.publish_telemetry();
                        } else {
                            // Never busy-loop if the upstream event channel closes; mark unhealthy and pause.
                            events_closed = true;
                            self.health.user_stream_healthy = false;
                            warn!("user_events_rx closed; disabling event branch and pausing new entries");
                            let now_ms = unix_now_ms();
                            let mode = self.mode_machine.apply(
                                ModeTrigger::ReconnectOrSyncNeeded {
                                    reason: "user data event channel closed".to_owned(),
                                },
                                now_ms,
                            );
                            let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
                                mode,
                                reason: self.mode_machine.reason().to_owned(),
                            });
                            if let Ok(seq) = self.store.append_event(now_ms, &env) {
                                self.state.apply_event(seq, &env);
                            }
                            self.publish_telemetry();
                        }
                    }
                    maybe_cmd = cmd_rx.recv() => {
                        let Some(cmd) = maybe_cmd else {
                            let _ = self.perform_graceful_shutdown(unix_now_ms()).await;
                            break;
                        };
                        if self.dispatch_command(cmd).await {
                            break;
                        }
                    }
                }
            }
            self
        });

        (handle, task)
    }

    /// Process a single [`EngineCommand`]; returns `true` when the actor loop should exit.
    #[allow(clippy::too_many_lines)]
    async fn dispatch_command(&mut self, cmd: EngineCommand) -> bool {
        match cmd {
            EngineCommand::SubmitEntry {
                intent,
                now_ms,
                reply,
            } => {
                let res = self.handle_submit_entry(&intent, now_ms).await;
                self.publish_telemetry();
                let _ = reply.send(res);
            }
            EngineCommand::MarketTick { tick, reply } => {
                self.handle_market_tick(&tick).await;
                self.publish_telemetry();
                let _ = reply.send(());
            }
            EngineCommand::Reconcile { now_ms, reply } => {
                let res = self
                    .handle_reconcile(now_ms)
                    .await
                    .map_err(|e| e.to_string());
                self.publish_telemetry();
                let _ = reply.send(res);
            }
            EngineCommand::ApplyModeTrigger {
                trigger,
                now_ms,
                reply,
            } => {
                let mode = self.mode_machine.apply(trigger, now_ms);
                let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
                    mode,
                    reason: self.mode_machine.reason().to_owned(),
                });
                if let Ok(seq) = self.store.append_event(now_ms, &env) {
                    self.state.apply_event(seq, &env);
                }
                self.publish_telemetry();
                let _ = reply.send(mode);
            }
            EngineCommand::PanicFlatten { now_ms, reply } => {
                let res = self.handle_panic_flatten(now_ms).await;
                self.publish_telemetry();
                let _ = reply.send(res);
            }
            EngineCommand::ClosePosition {
                position_id,
                now_ms,
                reply,
            } => {
                let res = self.handle_close_position(position_id, now_ms).await;
                self.publish_telemetry();
                let _ = reply.send(res);
            }
            EngineCommand::SetParam { key, value, reply } => {
                let res = self.handle_set_param(&key, &value);
                self.publish_telemetry();
                let _ = reply.send(res);
            }
            EngineCommand::DisableStrategy { id, reply } => {
                let res = disable_strategy_in_config(&mut self.config, &id)
                    .map(|()| format!("strategy '{id}' disabled"));
                self.publish_telemetry();
                let _ = reply.send(res);
            }
            EngineCommand::UpdateHealth {
                health,
                btc_crash,
                btc_spike,
                now_ms,
                reply,
            } => {
                self.health = health;
                if let Some(sample) = btc_crash {
                    let _ = self.btc_guard.update(sample);
                }
                if let Some(sample) = btc_spike {
                    let _ = self.btc_spike_guard.update(sample);
                }
                if let Some(trig) = Gatekeeper::check_operational_health(
                    self.health.market_data_age_ms,
                    self.health.clock_drift_ms,
                ) {
                    let mode = self.mode_machine.apply(trig, now_ms);
                    let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
                        mode,
                        reason: self.mode_machine.reason().to_owned(),
                    });
                    if let Ok(seq) = self.store.append_event(now_ms, &env) {
                        self.state.apply_event(seq, &env);
                    }
                }
                self.publish_telemetry();
                let _ = reply.send(());
            }
            EngineCommand::GetFullSnapshot { now_ms, reply } => {
                let snap = self.build_full_snapshot(now_ms);
                let _ = reply.send(snap);
            }
            EngineCommand::SnapshotNow { now_ms, reply } => {
                let res = self
                    .store
                    .snapshot_write(now_ms, &self.state)
                    .map_err(|e| e.to_string());
                let _ = reply.send(res);
            }
            EngineCommand::SwitchVenue {
                target,
                policy,
                actor,
                now_ms,
                reply,
            } => {
                let res = self
                    .handle_switch_venue(target, policy, &actor, now_ms)
                    .await;
                self.publish_telemetry();
                let _ = reply.send(res);
            }
            EngineCommand::Shutdown { now_ms, reply } => {
                let canceled = self.perform_graceful_shutdown(now_ms).await;
                self.publish_telemetry();
                let _ = reply.send(canceled);
                return true;
            }
        }
        false
    }

    /// Register a secondary venue adapter (e.g. `UsdsFutures` when bootstrapped on `Spot`)
    /// so runtime `switch_venue` can activate and switch to it (§3.5).
    pub fn register_secondary_adapter(
        &mut self,
        adapter: Arc<dyn VenueAdapter>,
        enabled: bool,
        credentials_valid: bool,
    ) {
        let v = adapter.venue();
        self.venue_manager.set_venue_enabled(v, enabled);
        self.venue_manager
            .set_credentials_valid(v, credentials_valid);
        self.secondary_adapters.insert(v, adapter);
    }

    /// Access recovered/current [`PersistedEngineState`] (useful after actor shutdown in tests).
    #[must_use]
    pub const fn state(&self) -> &PersistedEngineState {
        &self.state
    }

    fn quote_asset(&self) -> &str {
        self.config
            .general
            .quote_assets
            .first()
            .map_or("USDT", String::as_str)
    }

    fn adapter_for_venue(&self, venue: Venue) -> Option<Arc<dyn VenueAdapter>> {
        if self.venue == venue {
            Some(Arc::clone(&self.adapter))
        } else {
            self.secondary_adapters.get(&venue).cloned()
        }
    }

    fn rules_map_for_venue(&self, venue: Venue) -> BTreeMap<Symbol, InstrumentRules> {
        self.rules_by_venue_symbol
            .iter()
            .filter(|((v, _), _)| *v == venue)
            .map(|((_, s), r)| (s.clone(), r.clone()))
            .collect()
    }

    fn ref_prices_for_venue(&self, venue: Venue) -> BTreeMap<Symbol, Price> {
        self.ref_prices
            .iter()
            .filter(|((v, _), _)| *v == venue)
            .map(|((_, s), p)| (s.clone(), *p))
            .collect()
    }

    fn cross_venue_positions(&self) -> Vec<CrossVenuePosition> {
        self.state
            .positions
            .values()
            .filter(|p| p.state != PositionState::Closed && p.qty_open.as_decimal() > Decimal::ZERO)
            .map(|p| {
                let stop_dist = (p.entry_price.as_decimal() - p.stop_price.as_decimal()).abs();
                CrossVenuePosition {
                    venue: p.venue,
                    symbol: p.symbol.clone(),
                    direction: p.direction,
                    risk_usd: p.qty_open.as_decimal().saturating_mul(stop_dist),
                }
            })
            .collect()
    }

    fn build_full_snapshot(&self, now_ms: i64) -> EngineFullSnapshot {
        let utc_day = u32::try_from(now_ms.max(0) / 86_400_000).unwrap_or(0);
        let open_positions: Vec<Position> = self
            .state
            .positions
            .values()
            .filter(|p| p.state != PositionState::Closed)
            .cloned()
            .collect();
        let active_orders: Vec<Order> = self
            .state
            .orders
            .values()
            .filter(|o| !is_terminal(o.status))
            .cloned()
            .collect();
        let cv_pos = self.cross_venue_positions();
        let heat_pct =
            CrossVenueBudget::combined_portfolio_heat_pct(&cv_pos, self.counters.combined_equity());
        let btc_guard = if self.btc_guard.is_active() {
            "CRASH_GUARD".to_owned()
        } else if self.btc_spike_guard.is_active() {
            "SPIKE_GUARD".to_owned()
        } else {
            "OK".to_owned()
        };

        EngineFullSnapshot {
            telemetry: TelemetrySnapshot {
                mode: self.mode_machine.mode(),
                venue: self.venue,
                spot_state: self.venue_manager.state(Venue::Spot),
                futures_state: self.venue_manager.state(Venue::UsdsFutures),
                open_positions: open_positions.len(),
                active_orders: active_orders.len(),
                equity_total_usd: self.counters.combined_equity(),
                last_seq: self.state.last_seq,
            },
            started_at_ms: self.started_at_ms,
            last_switch_ms: self.last_switch_ms,
            switches_today: self.venue_manager.switches_today(utc_day),
            health: self.health.clone(),
            day_pnl_pct: self.last_equity_outcome.daily_pnl_pct,
            week_pnl_pct: self.last_equity_outcome.weekly_pnl_pct,
            drawdown_pct: self.last_equity_outcome.drawdown_pct,
            heat_pct,
            btc_guard,
            positions: open_positions,
            orders: active_orders,
            config: self.config.clone(),
        }
    }

    fn publish_telemetry(&self) {
        let open_positions = self
            .state
            .positions
            .values()
            .filter(|p| p.state != PositionState::Closed)
            .count();
        let active_orders = self
            .state
            .orders
            .values()
            .filter(|o| !is_terminal(o.status))
            .count();
        let _ = self.telemetry_tx.send(TelemetrySnapshot {
            mode: self.mode_machine.mode(),
            venue: self.venue,
            spot_state: self.venue_manager.state(Venue::Spot),
            futures_state: self.venue_manager.state(Venue::UsdsFutures),
            open_positions,
            active_orders,
            equity_total_usd: self.counters.combined_equity(),
            last_seq: self.state.last_seq,
        });
    }

    fn record_closed_position_pnl(&mut self, venue: Venue, net_pnl_quote: Decimal, now_ms: i64) {
        self.counters.record_trade_outcome(net_pnl_quote, now_ms);
        let current_eq = self
            .venue_equities
            .get(&venue)
            .copied()
            .unwrap_or_else(|| self.counters.combined_equity());
        let next_eq = current_eq.saturating_add(net_pnl_quote).max(Decimal::ZERO);
        self.venue_equities.insert(venue, next_eq);
        let outcome = self
            .counters
            .update_venue_equity(venue, next_eq, now_ms, &self.config.risk);
        self.last_equity_outcome = outcome;
        if let Some(kind) = outcome.breaker_trigger {
            let mode = self.mode_machine.apply(kind.into_trigger(), now_ms);
            let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
                mode,
                reason: self.mode_machine.reason().to_owned(),
            });
            if let Ok(seq) = self.store.append_event(now_ms, &env) {
                self.state.apply_event(seq, &env);
            }
        }
    }

    async fn handle_panic_flatten(&mut self, now_ms: i64) -> (EngineMode, usize) {
        // 1. Cancel all pending entry orders across active and secondary venues.
        let pending_entries: Vec<(Venue, Symbol, String)> = self
            .state
            .orders
            .values()
            .filter(|o| o.purpose == OrderPurpose::Entry && !is_terminal(o.status))
            .map(|o| (o.venue, o.symbol.clone(), o.client_id.clone()))
            .collect();
        for (v, sym, cid) in pending_entries {
            if let Some(adp) = self.adapter_for_venue(v) {
                let _ = adp
                    .cancel_order(CancelOrderRequest {
                        symbol: sym,
                        orig_client_order_id: Some(cid),
                        order_id: None,
                        new_client_order_id: None,
                    })
                    .await;
            }
        }

        // 2. Flatten all open positions if kill_flatten is enabled (§7.4).
        let mut flattened = 0_usize;
        if self.config.safety.kill_flatten {
            let open_positions: Vec<Position> = self
                .state
                .positions
                .values()
                .filter(|p| {
                    p.state != PositionState::Closed && p.qty_open.as_decimal() > Decimal::ZERO
                })
                .cloned()
                .collect();
            for pos in open_positions {
                if self
                    .execute_market_exit(&pos, "panic_flatten", now_ms)
                    .await
                {
                    flattened = flattened.saturating_add(1);
                }
            }
        }

        // 3. Latch Killed mode and journal.
        let mode = self.mode_machine.apply(ModeTrigger::ManualPanic, now_ms);
        let env = JournalEnvelope::new(JournalEvent::EngineModeChanged {
            mode,
            reason: self.mode_machine.reason().to_owned(),
        });
        if let Ok(seq) = self.store.append_event(now_ms, &env) {
            self.state.apply_event(seq, &env);
        }
        (mode, flattened)
    }

    async fn handle_close_position(
        &mut self,
        position_id: Uuid,
        now_ms: i64,
    ) -> Result<String, String> {
        let Some(pos) = self
            .state
            .positions
            .get(&position_id)
            .filter(|p| p.state != PositionState::Closed)
            .cloned()
        else {
            return Err(format!("open position '{position_id}' not found"));
        };
        if self.execute_market_exit(&pos, "manual_close", now_ms).await {
            Ok(format!("position '{position_id}' closed at market"))
        } else {
            Err(format!(
                "market exit failed for position '{position_id}'; kept OpenUnprotected for retry"
            ))
        }
    }

    fn handle_set_param(&mut self, key: &str, value: &str) -> Result<String, String> {
        let candidate = tighten_runtime_param(&self.config, key, value)?;
        self.gatekeeper
            .update_config(candidate.risk.clone())
            .map_err(|err| err.to_string())?;
        self.config = candidate;
        Ok(format!("parameter '{key}' tightened to '{value}'"))
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_switch_venue(
        &mut self,
        target: Venue,
        policy: SwitchPolicy,
        actor: &str,
        now_ms: i64,
    ) -> Result<VenueSwitchReport, String> {
        let old_venue = self.venue;
        let old_open_positions_list: Vec<Position> = self
            .state
            .positions
            .values()
            .filter(|p| p.venue == old_venue && p.state != PositionState::Closed)
            .cloned()
            .collect();
        let old_open_positions = old_open_positions_list.len();
        let utc_day = u32::try_from(now_ms.max(0) / 86_400_000).unwrap_or(0);
        let ctx = SwitchPrecheckContext {
            engine_mode: self.mode_machine.mode(),
            trading_mode: self.config.general.mode,
            syncing: self.mode_machine.mode() == EngineMode::Syncing,
            old_open_positions,
            now_ms,
            utc_day,
        };

        self.venue_manager
            .begin_switch(target, policy, &ctx)
            .map_err(|e| e.to_string())?;

        // Step 2 (§3.5): cancel ONLY unfilled entry orders on old venue; preserve all protections!
        let mut canceled_entry_orders = 0_usize;
        let pending_entries: Vec<(Symbol, String)> = self
            .state
            .orders
            .values()
            .filter(|o| {
                o.venue == old_venue && o.purpose == OrderPurpose::Entry && !is_terminal(o.status)
            })
            .map(|o| (o.symbol.clone(), o.client_id.clone()))
            .collect();
        for (sym, cid) in pending_entries {
            if self
                .adapter
                .cancel_order(CancelOrderRequest {
                    symbol: sym,
                    orig_client_order_id: Some(cid.clone()),
                    order_id: None,
                    new_client_order_id: None,
                })
                .await
                .is_ok()
            {
                canceled_entry_orders = canceled_entry_orders.saturating_add(1);
            }
        }

        // Step 3 (§3.5): if policy == Flatten, actually flatten all open positions on old venue!
        if policy == SwitchPolicy::Flatten {
            for pos in &old_open_positions_list {
                if pos.qty_open.as_decimal() > Decimal::ZERO
                    && !self
                        .execute_market_exit(pos, "venue_switch_flatten", now_ms)
                        .await
                {
                    return Err(self.rollback_switch_and_pause(
                        old_venue,
                        target,
                        &format!("failed to flatten position {} on {old_venue:?}", pos.id),
                        now_ms,
                    ));
                }
            }
        }

        // Step 4 (§3.5): activate target adapter, load its exchangeInfo rules, and reconcile target!
        let Some(target_adapter) = self.secondary_adapters.get(&target).cloned() else {
            return Err(self.rollback_switch_and_pause(
                old_venue,
                target,
                &format!("no adapter registered for target venue {target:?}"),
                now_ms,
            ));
        };

        if let Err(err) = target_adapter.activate().await {
            return Err(self.rollback_switch_and_pause(
                old_venue,
                target,
                &format!("activation failed: {err}"),
                now_ms,
            ));
        }

        let target_info = match target_adapter.exchange_info().await {
            Ok(info) => info,
            Err(err) => {
                let _ = target_adapter.deactivate().await;
                return Err(self.rollback_switch_and_pause(
                    old_venue,
                    target,
                    &format!("target exchange_info failed: {err}"),
                    now_ms,
                ));
            }
        };
        let mut target_rules_map = BTreeMap::new();
        for r in target_info.rules {
            target_rules_map.insert(r.symbol.clone(), r.clone());
            self.rules_by_venue_symbol
                .insert((target, r.symbol.clone()), r);
        }

        let target_refs = self.ref_prices_for_venue(target);
        let quote_asset = self.quote_asset().to_owned();
        if let Err(err) = reconcile_venue(
            target_adapter.as_ref(),
            &mut self.store,
            &mut self.state,
            &mut self.fsm,
            &mut self.oco_mgr,
            &target_rules_map,
            &target_refs,
            &ReconcileConfig {
                venue: target,
                quote_asset,
                orphan_policy: self.config.risk.orphan_policy,
                bot_id: self.bot_id.clone(),
                run_id: self.run_id,
                tp1_close_ratio: self.config.strategy.trend_pullback.tp1_close_ratio,
                adopt_stop_pct: Decimal::from(2_u32),
            },
            now_ms,
        )
        .await
        {
            let _ = target_adapter.deactivate().await;
            return Err(self.rollback_switch_and_pause(
                old_venue,
                target,
                &format!("target reconciliation failed: {err}"),
                now_ms,
            ));
        }

        // Step 6 (§3.5): finalize switch, swap active adapter, journal VenueSwitched & VenueStateChanged.
        // Note: self.counters is NEVER reset across venue switches.
        self.venue_manager
            .complete_switch(target, policy, now_ms, utc_day);
        let prev_adapter = std::mem::replace(&mut self.adapter, target_adapter);
        self.secondary_adapters.insert(old_venue, prev_adapter);
        self.venue = target;
        self.last_switch_ms = now_ms;

        let old_state = self.venue_manager.state(old_venue);
        let switches_today = self.venue_manager.switches_today(utc_day);
        for (v, st) in [(old_venue, old_state), (target, VenueState::Active)] {
            let rec = VenueStateRecord {
                venue: v,
                state: st,
                since_ms: now_ms,
                last_switch_ms: Some(now_ms),
                switches_today,
            };
            let env = JournalEnvelope::new(JournalEvent::VenueStateChanged { record: rec });
            if let Ok(seq) = self.store.append_event(now_ms, &env) {
                self.state.apply_event(seq, &env);
            }
        }
        let policy_str = match policy {
            SwitchPolicy::KeepManaging => "keep_managing",
            SwitchPolicy::RequireFlat => "require_flat",
            SwitchPolicy::Flatten => "flatten",
        };
        let sw_env = JournalEnvelope::new(JournalEvent::VenueSwitched {
            from: old_venue,
            to: target,
            policy: policy_str.to_owned(),
            actor: actor.to_owned(),
            ts_ms: now_ms,
        });
        if let Ok(seq) = self.store.append_event(now_ms, &sw_env) {
            self.state.apply_event(seq, &sw_env);
        }

        info!(
            ?old_venue,
            ?target,
            ?old_state,
            canceled_entry_orders,
            "venue switched"
        );
        Ok(VenueSwitchReport {
            from: old_venue,
            to: target,
            old_venue_state: old_state,
            canceled_entry_orders,
        })
    }

    fn rollback_switch_and_pause(
        &mut self,
        old_venue: Venue,
        target: Venue,
        detail: &str,
        now_ms: i64,
    ) -> String {
        warn!(
            ?old_venue,
            ?target,
            detail,
            "venue switch failed; rolling back"
        );
        self.venue_manager.rollback_switch(old_venue, target);
        let reason = format!("venue switch rollback ({target:?} {detail})");
        let mode = self.mode_machine.apply(
            ModeTrigger::ManualPause {
                reason: reason.clone(),
            },
            now_ms,
        );
        let env = JournalEnvelope::new(JournalEvent::EngineModeChanged { mode, reason });
        if let Ok(seq) = self.store.append_event(now_ms, &env) {
            self.state.apply_event(seq, &env);
        }
        format!("{detail} (rolled back)")
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_submit_entry(
        &mut self,
        intent: &EntryIntent,
        now_ms: i64,
    ) -> EntrySubmissionResult {
        let Some(rules) = self
            .rules_by_venue_symbol
            .get(&(intent.venue, intent.symbol.clone()))
            .cloned()
        else {
            return EntrySubmissionResult::GatewayFailed("unknown symbol rules".into());
        };
        let Some(venue_adapter) = self.adapter_for_venue(intent.venue) else {
            return EntrySubmissionResult::GatewayFailed(format!(
                "no adapter for venue {:?}",
                intent.venue
            ));
        };

        // Cross-venue same-symbol guard (§3.5, §7.8) + combined portfolio heat check.
        let cv_positions = self.cross_venue_positions();
        let cv_policy = match self.config.venues.cross_venue_same_symbol {
            bot_config::CrossVenueSameSymbolPolicy::BlockOpposite => {
                CrossVenueSameSymbolPolicy::BlockOpposite
            }
            bot_config::CrossVenueSameSymbolPolicy::Allow => CrossVenueSameSymbolPolicy::Allow,
        };
        if CrossVenueBudget::check_same_symbol(
            cv_policy,
            &cv_positions,
            intent.venue,
            &intent.symbol,
            intent.direction,
        )
        .is_err()
        {
            return EntrySubmissionResult::Rejected(GateDecision::Reject {
                reasons: vec![RejectReason::DuplicateOpenSymbol],
                auto_disable_strategy: false,
            });
        }

        let btc_guard_active = match intent.direction {
            Direction::Long => self.btc_guard.is_active(),
            Direction::Short => self.btc_spike_guard.is_active(),
        };
        let market_data_age_ms =
            self.last_market_tick_ms
                .map_or(self.health.market_data_age_ms, |last| {
                    now_ms
                        .saturating_sub(last)
                        .max(self.health.market_data_age_ms)
                });

        let open_positions: Vec<Position> = self
            .state
            .positions
            .values()
            .filter(|p| p.state != PositionState::Closed)
            .cloned()
            .collect();
        let ctx = GatekeeperContext {
            now_ms,
            mode: self.mode_machine.mode(),
            rules: &rules,
            symbol_in_universe: true,
            symbol_quarantined: false,
            symbol_cooldown_until_ms: None,
            btc_guard_active,
            macro_blackout_active: false,
            market_data_age_ms,
            user_stream_healthy: self.health.user_stream_healthy,
            clock_drift_ms: self.health.clock_drift_ms,
            open_positions: &open_positions,
            equity_total_usd: self.counters.combined_equity(),
            free_quote_usd: self.free_quote_usd,
            request_weight_pct: self.health.request_weight_pct,
            orders_10s_pct: self.health.orders_10s_pct,
            loss_streak_cooldown_until_ms: self.counters.loss_cooldown_until_ms(),
            orders_today: self.counters.orders_today(),
            profit_lock_stop: self.last_equity_outcome.profit_lock_stop,
            profit_lock_mult: self.last_equity_outcome.profit_lock_mult,
            kelly: None,
            percent_price_bounds: None,
            fixed_notional_usd: None,
        };

        let qty = match self.gatekeeper.evaluate(intent, &ctx) {
            GateDecision::Accept { qty, .. } => qty,
            rej @ GateDecision::Reject { .. } => return EntrySubmissionResult::Rejected(rej),
        };

        let seq = match self.store.next_order_seq() {
            Ok(s) => s,
            Err(e) => {
                self.halt_on_storage_error(&e.to_string(), now_ms);
                return EntrySubmissionResult::GatewayFailed(e.to_string());
            }
        };
        let Ok(cid) = format_client_order_id(
            &self.bot_id,
            self.run_id,
            intent.venue,
            seq,
            OrderPurpose::Entry,
        ) else {
            return EntrySubmissionResult::GatewayFailed("invalid clientOrderId".into());
        };

        let entry_side = entry_side_for_direction(intent.direction);
        let (entry_kind, entry_tif) = match intent.venue {
            Venue::Spot => (OrderKind::LimitMaker, None),
            Venue::UsdsFutures => (OrderKind::Limit, Some(TimeInForce::Gtx)),
        };

        let pos_id = Uuid::now_v7();
        let dt = ms_to_dt(now_ms);
        let order = Order {
            venue: intent.venue,
            client_id: cid.clone(),
            exchange_id: None,
            symbol: intent.symbol.clone(),
            side: entry_side,
            kind: entry_kind,
            price: Some(intent.entry_price),
            qty,
            filled_qty: Qty(Decimal::ZERO),
            avg_fill_price: None,
            status: OrderStatus::PendingNew,
            position_id: Some(pos_id),
            purpose: OrderPurpose::Entry,
            created_at: dt,
            updated_at: dt,
        };

        let r_dist = (intent.entry_price.as_decimal() - intent.stop_price.as_decimal()).abs();
        let tp2_price = compute_tp2_price(
            intent.direction,
            intent.entry_price,
            r_dist,
            self.config.strategy.trend_pullback.tp2_r,
            rules.tick_size,
        );
        let leverage =
            (intent.venue == Venue::UsdsFutures).then_some(self.config.venues.futures.max_leverage);
        let pending_pos = Position {
            id: pos_id,
            symbol: intent.symbol.clone(),
            venue: intent.venue,
            direction: intent.direction,
            strategy: intent.strategy,
            entry_price: intent.entry_price,
            qty_initial: qty,
            qty_open: Qty(Decimal::ZERO),
            r_distance: r_dist,
            leverage,
            liq_price: None,
            stop_price: intent.stop_price,
            tp1_price: intent.tp1_price,
            tp2_price: Some(tp2_price),
            best_price: intent.entry_price,
            tp1_done: false,
            breakeven_done: false,
            fees_paid_quote: Decimal::ZERO,
            realized_pnl_quote: Decimal::ZERO,
            state: PositionState::EntryPending,
            opened_at: dt,
        };

        // Journal-first: OrderIntended + PositionOpened(EntryPending) BEFORE sending to exchange!
        let ord_env = JournalEnvelope::new(JournalEvent::OrderIntended {
            order: order.clone(),
        });
        let pos_env = JournalEnvelope::new(JournalEvent::PositionOpened {
            position: pending_pos.clone(),
        });
        if let Err(e) = (|| -> Result<(), bot_store::StoreError> {
            let s1 = self.store.append_event(now_ms, &ord_env)?;
            self.state.apply_event(s1, &ord_env);
            self.store.upsert_order(&order)?;
            let s2 = self.store.append_event(now_ms, &pos_env)?;
            self.state.apply_event(s2, &pos_env);
            self.store.upsert_position(&pending_pos, None, None)?;
            Ok(())
        })() {
            self.halt_on_storage_error(&e.to_string(), now_ms);
            return EntrySubmissionResult::GatewayFailed(e.to_string());
        }
        self.fsm.register_intended(order);

        match venue_adapter
            .place_order(PlaceOrderRequest {
                symbol: intent.symbol.clone(),
                side: entry_side,
                kind: entry_kind,
                client_order_id: cid.clone(),
                quantity: Some(qty),
                quote_order_qty: None,
                price: Some(intent.entry_price),
                stop_price: None,
                time_in_force: entry_tif,
            })
            .await
        {
            Ok(ack) => {
                self.counters.record_order_submitted();
                self.counters.record_order_error_state(false);
                let upd_env = JournalEnvelope::new(JournalEvent::OrderStatusUpdated {
                    venue: intent.venue,
                    client_id: cid.clone(),
                    exchange_id: Some(ack.order_id),
                    status: ack.status,
                    filled_qty: ack.executed_qty,
                    avg_fill_price: None,
                    updated_at: dt,
                });
                if let Ok(s) = self.store.append_event(now_ms, &upd_env) {
                    self.state.apply_event(s, &upd_env);
                }
                EntrySubmissionResult::Submitted {
                    client_order_id: cid,
                    position_id: pos_id,
                    qty,
                }
            }
            Err(err) => {
                if let Some(trig) = self.counters.record_order_error_state(true) {
                    self.mode_machine.apply(trig, now_ms);
                }
                self.fsm.mark_rejected(intent.venue, &cid, dt);
                EntrySubmissionResult::GatewayFailed(err.to_string())
            }
        }
    }

    async fn install_venue_protection(
        &mut self,
        pos: &Position,
        rules: &InstrumentRules,
        ts_ms: i64,
    ) -> Result<(), String> {
        let Some(adp) = self.adapter_for_venue(pos.venue) else {
            return Err(format!("no adapter for venue {:?}", pos.venue));
        };
        match pos.venue {
            Venue::Spot => self
                .oco_mgr
                .install_dual_oco(
                    adp.as_ref(),
                    &mut self.store,
                    pos,
                    rules,
                    self.config.strategy.trend_pullback.tp1_close_ratio,
                    &self.bot_id,
                    self.run_id,
                    ts_ms,
                )
                .await
                .map(|_| ())
                .map_err(|e| e.to_string()),
            Venue::UsdsFutures => {
                let sl_seq = self.store.next_order_seq().map_err(|e| e.to_string())?;
                let sl_cid = format_client_order_id(
                    &self.bot_id,
                    self.run_id,
                    pos.venue,
                    sl_seq,
                    OrderPurpose::ProtectStop,
                )
                .map_err(|e| e.to_string())?;
                let tp_seq = self.store.next_order_seq().map_err(|e| e.to_string())?;
                let tp_cid = format_client_order_id(
                    &self.bot_id,
                    self.run_id,
                    pos.venue,
                    tp_seq,
                    OrderPurpose::TakeProfit1,
                )
                .map_err(|e| e.to_string())?;

                let pos_side = entry_side_for_direction(pos.direction);
                let state = AlgoProtectionManager
                    .install_protection(
                        adp.as_ref(),
                        pos.symbol.clone(),
                        pos_side,
                        pos.qty_open,
                        pos.stop_price,
                        Some(pos.tp1_price),
                        sl_cid,
                        Some(tp_cid),
                    )
                    .await
                    .map_err(|e| e.to_string())?;
                self.futures_protections.insert(pos.id, state);
                Ok(())
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_gateway_event(&mut self, ev: GatewayEvent) {
        match ev {
            GatewayEvent::Exec(exec) => {
                let quote = self.quote_asset().to_owned();
                let outcome = self.fsm.apply_exec_event(&exec, &quote, None);
                if let FsmApplyOutcome::Updated {
                    status,
                    new_fill,
                    fee_quote,
                } = outcome
                {
                    if let Some(fill) = new_fill {
                        let _ = self.store.upsert_fill(&fill);
                        let env = JournalEnvelope::new(JournalEvent::FillRecorded { fill });
                        if let Ok(seq) = self.store.append_event(exec.event_time_ms, &env) {
                            self.state.apply_event(seq, &env);
                        }
                    }

                    let tracked = self.fsm.get(exec.venue, &exec.client_order_id).cloned();
                    if let Some(ord) = tracked {
                        let upd_env = JournalEnvelope::new(JournalEvent::OrderStatusUpdated {
                            venue: ord.venue,
                            client_id: ord.client_id.clone(),
                            exchange_id: ord.exchange_id,
                            status,
                            filled_qty: ord.filled_qty,
                            avg_fill_price: ord.avg_fill_price,
                            updated_at: ord.updated_at,
                        });
                        if let Ok(seq) = self.store.append_event(exec.event_time_ms, &upd_env) {
                            self.state.apply_event(seq, &upd_env);
                        }
                        let _ = self.store.upsert_order(&ord);

                        // Entry filled or partially filled -> install protection FIRST, then record OpenProtected/OpenUnprotected!
                        let has_fill_qty = ord.filled_qty.as_decimal() > Decimal::ZERO;
                        let entry_needs_protection = ord.purpose == OrderPurpose::Entry
                            && has_fill_qty
                            && matches!(status, OrderStatus::Filled | OrderStatus::PartiallyFilled);
                        if entry_needs_protection
                            && let Some(pos_id) = ord.position_id
                            && let Some(mut pos) = self.state.positions.get(&pos_id).cloned()
                        {
                            let already_protected = pos.state == PositionState::OpenProtected
                                && self.has_exchange_protection(pos.venue, pos.id);
                            pos.qty_open = ord.filled_qty;
                            if let Some(avg) = ord.avg_fill_price {
                                pos.entry_price = avg;
                                pos.best_price = avg;
                            }
                            pos.fees_paid_quote = pos.fees_paid_quote.saturating_add(fee_quote);

                            let target_state = if already_protected {
                                PositionState::OpenProtected
                            } else if let Some(rules) = self
                                .rules_by_venue_symbol
                                .get(&(pos.venue, pos.symbol.clone()))
                                .cloned()
                            {
                                match self
                                    .install_venue_protection(&pos, &rules, exec.event_time_ms)
                                    .await
                                {
                                    Ok(()) => PositionState::OpenProtected,
                                    Err(err) => {
                                        warn!(
                                            error = %err,
                                            position_id = %pos.id,
                                            venue = ?pos.venue,
                                            "failed to install protection after entry fill; marking OpenUnprotected"
                                        );
                                        self.oco_mgr
                                            .record_reinstall_failure(pos.id, pos.stop_price);
                                        PositionState::OpenUnprotected
                                    }
                                }
                            } else {
                                self.oco_mgr
                                    .record_reinstall_failure(pos.id, pos.stop_price);
                                PositionState::OpenUnprotected
                            };

                            pos.state = target_state;
                            let pos_env =
                                JournalEnvelope::new(JournalEvent::PositionStateChanged {
                                    venue: pos.venue,
                                    position_id: pos.id,
                                    state: target_state,
                                    qty_open: pos.qty_open,
                                    realized_pnl_quote: pos.realized_pnl_quote,
                                    fees_paid_quote: pos.fees_paid_quote,
                                    close_reason: None,
                                });
                            if let Ok(seq) = self.store.append_event(exec.event_time_ms, &pos_env) {
                                self.state.apply_event(seq, &pos_env);
                            }
                            let _ = self.store.upsert_position(&pos, None, None);
                        }
                    }
                }
            }
            GatewayEvent::ListStatus(ls) => {
                if ls.list_order_status == "ALL_DONE"
                    && let Some((pos_id, was_oco_a)) = self.oco_mgr.on_list_done(ls.order_list_id)
                    && let Some(mut pos) = self.state.positions.get(&pos_id).cloned()
                {
                    if was_oco_a
                        && self
                            .oco_mgr
                            .get(pos_id)
                            .is_some_and(|p| p.oco_b_list_id.is_some())
                    {
                        let closed_part = pos
                            .qty_open
                            .as_decimal()
                            .saturating_mul(self.config.strategy.trend_pullback.tp1_close_ratio);
                        pos.qty_open = Qty(pos.qty_open.as_decimal().saturating_sub(closed_part));
                        pos.tp1_done = true;
                        let env = JournalEnvelope::new(JournalEvent::PositionTp1Done {
                            venue: pos.venue,
                            position_id: pos.id,
                            qty_open: pos.qty_open,
                            realized_pnl_quote: pos.realized_pnl_quote,
                            fees_paid_quote: pos.fees_paid_quote,
                        });
                        if let Ok(seq) = self.store.append_event(ls.event_time_ms, &env) {
                            self.state.apply_event(seq, &env);
                        }
                    } else if self
                        .oco_mgr
                        .get(pos_id)
                        .is_none_or(|p| !p.has_active_protection())
                    {
                        let exit_px = self
                            .ref_prices
                            .get(&(pos.venue, pos.symbol.clone()))
                            .copied()
                            .unwrap_or(pos.entry_price);
                        let gross = compute_gross_realized_pnl(
                            pos.direction,
                            pos.entry_price,
                            exit_px,
                            pos.qty_open,
                        );
                        let net_pnl = pos
                            .realized_pnl_quote
                            .saturating_add(gross)
                            .saturating_sub(pos.fees_paid_quote);
                        let env = JournalEnvelope::new(JournalEvent::PositionStateChanged {
                            venue: pos.venue,
                            position_id: pos.id,
                            state: PositionState::Closed,
                            qty_open: Qty(Decimal::ZERO),
                            realized_pnl_quote: net_pnl,
                            fees_paid_quote: pos.fees_paid_quote,
                            close_reason: Some("OCO_ALL_DONE".into()),
                        });
                        if let Ok(seq) = self.store.append_event(ls.event_time_ms, &env) {
                            self.state.apply_event(seq, &env);
                        }
                        self.record_closed_position_pnl(pos.venue, net_pnl, ls.event_time_ms);
                        self.oco_mgr.remove(pos_id);
                        self.futures_protections.remove(&pos_id);
                        self.watcher.clear(pos_id);
                    }
                }
            }
            _ => {}
        }
    }

    fn has_exchange_protection(&self, venue: Venue, pos_id: Uuid) -> bool {
        match venue {
            Venue::Spot => self
                .oco_mgr
                .get(pos_id)
                .is_some_and(bot_exec::PositionProtection::has_active_protection),
            Venue::UsdsFutures => self.futures_protections.contains_key(&pos_id),
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_market_tick(&mut self, tick: &MarketTick) {
        self.last_market_tick_ms = Some(tick.now_ms);
        self.ref_prices
            .insert((tick.venue, tick.symbol.clone()), tick.price);
        let matching_ids: Vec<Uuid> = self
            .state
            .positions
            .values()
            .filter(|p| {
                p.venue == tick.venue
                    && p.symbol == tick.symbol
                    && matches!(
                        p.state,
                        PositionState::OpenProtected | PositionState::OpenUnprotected
                    )
            })
            .map(|p| p.id)
            .collect();

        for pos_id in matching_ids {
            let Some(mut pos) = self.state.positions.get(&pos_id).cloned() else {
                continue;
            };
            let Some(rules) = self
                .rules_by_venue_symbol
                .get(&(pos.venue, pos.symbol.clone()))
                .cloned()
            else {
                continue;
            };
            let has_prot = self.has_exchange_protection(pos.venue, pos_id);
            let failures = self.oco_mgr.get(pos_id).map_or(0, |p| p.reinstall_failures);

            let action = evaluate_position_tick(
                &mut pos,
                PositionTickInput {
                    price: tick.price,
                    now_ms: tick.now_ms,
                    atr: tick.atr,
                    rsi_14: tick.rsi_14,
                    bars_in_trade: tick.bars_in_trade,
                    has_exchange_protection: has_prot,
                    reinstall_failures: failures,
                    tick_size: rules.tick_size,
                    taker_fee_bps: Decimal::from(10_u32),
                },
                &self.config.exit,
                &mut self.watcher,
            );

            match action {
                LifecycleAction::Hold => {}
                LifecycleAction::UpdateBestPrice { best_price } => {
                    let env = JournalEnvelope::new(JournalEvent::PositionBestPriceUpdated {
                        venue: pos.venue,
                        position_id: pos.id,
                        best_price,
                    });
                    if let Ok(seq) = self.store.append_event(tick.now_ms, &env) {
                        self.state.apply_event(seq, &env);
                    }
                }
                LifecycleAction::MoveStop {
                    new_stop,
                    is_breakeven,
                    journal_best_price,
                } => {
                    if let Some(bp) = journal_best_price {
                        let bp_env = JournalEnvelope::new(JournalEvent::PositionBestPriceUpdated {
                            venue: pos.venue,
                            position_id: pos.id,
                            best_price: bp,
                        });
                        if let Ok(seq) = self.store.append_event(tick.now_ms, &bp_env) {
                            self.state.apply_event(seq, &bp_env);
                        }
                    }
                    self.move_venue_stop(&pos, new_stop, is_breakeven, &rules, tick.now_ms)
                        .await;
                }
                LifecycleAction::ReinstallProtection => {
                    if self
                        .install_venue_protection(&pos, &rules, tick.now_ms)
                        .await
                        .is_err()
                    {
                        self.oco_mgr
                            .record_reinstall_failure(pos_id, pos.stop_price);
                    }
                }
                LifecycleAction::EmergencyMarketExit { reason } => {
                    let _ = self.execute_market_exit(&pos, &reason, tick.now_ms).await;
                }
                LifecycleAction::TimeStopExit { bars_in_trade } => {
                    let _ = self
                        .execute_market_exit(
                            &pos,
                            &format!("time_stop ({bars_in_trade} bars)"),
                            tick.now_ms,
                        )
                        .await;
                }
            }
        }
    }

    async fn move_venue_stop(
        &mut self,
        pos: &Position,
        new_stop: Price,
        is_breakeven: bool,
        rules: &InstrumentRules,
        now_ms: i64,
    ) {
        let Some(adp) = self.adapter_for_venue(pos.venue) else {
            return;
        };
        match pos.venue {
            Venue::Spot => {
                match self
                    .oco_mgr
                    .replace_runner_stop(
                        adp.as_ref(),
                        &mut self.store,
                        pos,
                        new_stop,
                        is_breakeven,
                        rules,
                        &self.bot_id,
                        self.run_id,
                        now_ms,
                    )
                    .await
                {
                    Ok(_) => {
                        let stop_env = JournalEnvelope::new(JournalEvent::PositionStopMoved {
                            venue: pos.venue,
                            position_id: pos.id,
                            new_stop_price: new_stop,
                            breakeven_done: is_breakeven,
                        });
                        let next_seq = self.state.last_seq.saturating_add(1);
                        self.state.apply_event(next_seq, &stop_env);
                    }
                    Err(err) => {
                        warn!(
                            position_id = %pos.id,
                            error = %err,
                            "replace_runner_stop failed; keeping local stop unchanged"
                        );
                        self.oco_mgr
                            .record_reinstall_failure(pos.id, pos.stop_price);
                    }
                }
            }
            Venue::UsdsFutures => {
                if let Some(mut algo_state) = self.futures_protections.get(&pos.id).cloned()
                    && let Ok(seq) = self.store.next_order_seq()
                    && let Ok(cid) = format_client_order_id(
                        &self.bot_id,
                        self.run_id,
                        pos.venue,
                        seq,
                        OrderPurpose::ProtectStop,
                    )
                {
                    match AlgoProtectionManager
                        .move_stop_make_before_break(adp.as_ref(), &mut algo_state, new_stop, cid)
                        .await
                    {
                        Ok(()) => {
                            self.futures_protections.insert(pos.id, algo_state);
                            let stop_env = JournalEnvelope::new(JournalEvent::PositionStopMoved {
                                venue: pos.venue,
                                position_id: pos.id,
                                new_stop_price: new_stop,
                                breakeven_done: is_breakeven,
                            });
                            if let Ok(jseq) = self.store.append_event(now_ms, &stop_env) {
                                self.state.apply_event(jseq, &stop_env);
                            }
                        }
                        Err(err) => {
                            warn!(
                                position_id = %pos.id,
                                error = %err,
                                "move_stop_make_before_break failed; old Futures SL remains active"
                            );
                            self.oco_mgr
                                .record_reinstall_failure(pos.id, pos.stop_price);
                        }
                    }
                }
            }
        }
    }

    #[allow(clippy::too_many_lines)]
    async fn execute_market_exit(&mut self, pos: &Position, reason: &str, now_ms: i64) -> bool {
        let Some(adp) = self.adapter_for_venue(pos.venue) else {
            return false;
        };

        // Cancel existing exchange-side protection for this position before market exit.
        match pos.venue {
            Venue::Spot => {
                if let Some(prot) = self.oco_mgr.remove(pos.id) {
                    for (list_id, list_cid) in [
                        (prot.oco_a_list_id, prot.oco_a_list_cid),
                        (prot.oco_b_list_id, prot.oco_b_list_cid),
                    ] {
                        if let Some(id) = list_id {
                            let _ = adp
                                .cancel_list(CancelOrderListRequest {
                                    symbol: pos.symbol.clone(),
                                    order_list_id: Some(id),
                                    list_client_order_id: list_cid,
                                })
                                .await;
                        }
                    }
                }
            }
            Venue::UsdsFutures => {
                if let Some(algo) = self.futures_protections.remove(&pos.id) {
                    let _ = adp
                        .cancel_algo_order(CancelAlgoOrderRequest {
                            symbol: pos.symbol.clone(),
                            algo_id: Some(algo.active_sl_algo_id),
                            client_algo_id: Some(algo.active_sl_client_id),
                        })
                        .await;
                    if let Some(tp_id) = algo.active_tp_algo_id {
                        let _ = adp
                            .cancel_algo_order(CancelAlgoOrderRequest {
                                symbol: pos.symbol.clone(),
                                algo_id: Some(tp_id),
                                client_algo_id: algo.active_tp_client_id,
                            })
                            .await;
                    }
                }
            }
        }

        let exit_side = exit_side_for_direction(pos.direction);
        let placed = if let Ok(seq) = self.store.next_order_seq()
            && let Ok(cid) = format_client_order_id(
                &self.bot_id,
                self.run_id,
                pos.venue,
                seq,
                OrderPurpose::EmergencyExit,
            ) {
            match adp
                .place_order(PlaceOrderRequest {
                    symbol: pos.symbol.clone(),
                    side: exit_side,
                    kind: OrderKind::Market,
                    client_order_id: cid,
                    quantity: Some(pos.qty_open),
                    quote_order_qty: None,
                    price: None,
                    stop_price: None,
                    time_in_force: None,
                })
                .await
            {
                Ok(_) => true,
                Err(err) => {
                    warn!(
                        position_id = %pos.id,
                        reason = %reason,
                        error = %err,
                        "emergency market exit order failed; keeping position open for retry"
                    );
                    false
                }
            }
        } else {
            false
        };

        if !placed {
            if pos.state != PositionState::OpenUnprotected {
                let unprot_env = JournalEnvelope::new(JournalEvent::PositionStateChanged {
                    venue: pos.venue,
                    position_id: pos.id,
                    state: PositionState::OpenUnprotected,
                    qty_open: pos.qty_open,
                    realized_pnl_quote: pos.realized_pnl_quote,
                    fees_paid_quote: pos.fees_paid_quote,
                    close_reason: None,
                });
                if let Ok(seq) = self.store.append_event(now_ms, &unprot_env) {
                    self.state.apply_event(seq, &unprot_env);
                }
            }
            return false;
        }

        let exit_px = self
            .ref_prices
            .get(&(pos.venue, pos.symbol.clone()))
            .copied()
            .unwrap_or(pos.entry_price);
        let gross =
            compute_gross_realized_pnl(pos.direction, pos.entry_price, exit_px, pos.qty_open);
        let net_pnl = pos
            .realized_pnl_quote
            .saturating_add(gross)
            .saturating_sub(pos.fees_paid_quote);

        let close_env = JournalEnvelope::new(JournalEvent::PositionStateChanged {
            venue: pos.venue,
            position_id: pos.id,
            state: PositionState::Closed,
            qty_open: Qty(Decimal::ZERO),
            realized_pnl_quote: net_pnl,
            fees_paid_quote: pos.fees_paid_quote,
            close_reason: Some(reason.to_owned()),
        });
        if let Ok(seq) = self.store.append_event(now_ms, &close_env) {
            self.state.apply_event(seq, &close_env);
        }
        self.record_closed_position_pnl(pos.venue, net_pnl, now_ms);
        self.watcher.clear(pos.id);
        true
    }

    async fn handle_reconcile(&mut self, now_ms: i64) -> Result<ReconcileReport, EngineError> {
        let quote_asset = self.quote_asset().to_owned();
        let rules_map = self.rules_map_for_venue(self.venue);
        let refs_map = self.ref_prices_for_venue(self.venue);
        let rep = reconcile_venue(
            self.adapter.as_ref(),
            &mut self.store,
            &mut self.state,
            &mut self.fsm,
            &mut self.oco_mgr,
            &rules_map,
            &refs_map,
            &ReconcileConfig {
                venue: self.venue,
                quote_asset,
                orphan_policy: self.config.risk.orphan_policy,
                bot_id: self.bot_id.clone(),
                run_id: self.run_id,
                tp1_close_ratio: self.config.strategy.trend_pullback.tp1_close_ratio,
                adopt_stop_pct: Decimal::from(2_u32),
            },
            now_ms,
        )
        .await?;
        if rep.material_mismatch {
            self.mode_machine
                .apply(ModeTrigger::MinorReconcileMismatch, now_ms);
        }
        Ok(rep)
    }

    /// Hard Rule #7 SIGTERM graceful shutdown:
    /// Cancel ONLY pending entry orders (`OrderPurpose::Entry`); NEVER cancel protective OCO/SL/TP!
    async fn perform_graceful_shutdown(&mut self, now_ms: i64) -> usize {
        let pending_entries: Vec<(Venue, Symbol, String)> = self
            .state
            .orders
            .values()
            .filter(|o| o.purpose == OrderPurpose::Entry && !is_terminal(o.status))
            .map(|o| (o.venue, o.symbol.clone(), o.client_id.clone()))
            .collect();

        let mut canceled_count = 0;
        for (v, sym, cid) in pending_entries {
            if let Some(adp) = self.adapter_for_venue(v)
                && adp
                    .cancel_order(CancelOrderRequest {
                        symbol: sym,
                        orig_client_order_id: Some(cid.clone()),
                        order_id: None,
                        new_client_order_id: None,
                    })
                    .await
                    .is_ok()
            {
                canceled_count += 1;
                let env = JournalEnvelope::new(JournalEvent::OrderStatusUpdated {
                    venue: v,
                    client_id: cid,
                    exchange_id: None,
                    status: OrderStatus::Canceled,
                    filled_qty: Qty(Decimal::ZERO),
                    avg_fill_price: None,
                    updated_at: ms_to_dt(now_ms),
                });
                if let Ok(seq) = self.store.append_event(now_ms, &env) {
                    self.state.apply_event(seq, &env);
                }
            }
        }

        let _ = self.store.snapshot_write(now_ms, &self.state);
        let _ = self.adapter.deactivate().await;
        for sec in self.secondary_adapters.values() {
            let _ = sec.deactivate().await;
        }
        info!(
            canceled_entries = canceled_count,
            "graceful shutdown complete (exchange protections preserved)"
        );
        canceled_count
    }

    fn halt_on_storage_error(&mut self, reason: &str, now_ms: i64) {
        self.mode_machine.apply(
            ModeTrigger::StorageFatalError {
                reason: reason.to_owned(),
            },
            now_ms,
        );
    }
}

fn unix_now_ms() -> i64 {
    let nanos = OffsetDateTime::now_utc().unix_timestamp_nanos();
    i64::try_from(nanos / 1_000_000).unwrap_or(0)
}

fn ms_to_dt(ms: i64) -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(ms).saturating_mul(1_000_000))
        .unwrap_or(OffsetDateTime::UNIX_EPOCH)
}
