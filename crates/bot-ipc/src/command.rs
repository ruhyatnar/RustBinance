//! Audited, idempotent IPC command dispatcher and backend trait (§10.1–§10.2, §7.4).

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use async_trait::async_trait;
use bot_config::{BotConfig, is_tightening};
use bot_store::{JournalEnvelope, JournalEvent, SqliteStore};
use bot_types::{EngineMode, TradingMode, Venue, VenueState};
use rust_decimal::Decimal;
use tokio::sync::{Mutex, RwLock};
use tracing::{info, warn};
use uuid::Uuid;

use crate::auth::AuthScope;
use crate::error::IpcError;
use crate::nonce::{ConfirmNonceStore, unix_now_ms};
use crate::protocol::{
    CommandKind, CommandPayload, CommandResultPayload, ConfirmAction, EngineSummary, HelloPayload,
    KlineBar, PROTOCOL_VERSION, RateStatus, RiskLimitsSummary, RiskSummary, SnapshotPayload,
    OnOpenPositionsArg, TradeRecord, VenuesSummary,
};

/// Maximum cached `cmd_id` results retained for idempotency.
const MAX_IDEMPOTENCY_CACHE: usize = 1024;

/// Backend interface implemented by the trading engine daemon (or an in-memory harness in tests).
#[async_trait]
pub trait IpcBackend: Send + Sync + 'static {
    /// Engine instance UUID.
    fn engine_instance_id(&self) -> Uuid;
    /// Active trading/build mode (`paper`, `testnet`, `live`).
    fn build_mode(&self) -> TradingMode;
    /// Current full snapshot for UI/CLI.
    async fn snapshot(&self) -> SnapshotPayload;
    /// Current engine mode.
    async fn engine_mode(&self) -> EngineMode;
    /// Pause new entries (`Ready -> Paused`).
    async fn pause(&self) -> Result<String, IpcError>;
    /// Resume new entries (`Paused -> Ready`).
    async fn resume(&self) -> Result<String, IpcError>;
    /// Tighten-only runtime parameter update (§7.8, Hard Rule #8).
    async fn set_param(&self, key: &str, value: &str) -> Result<String, IpcError>;
    /// Disable a strategy at runtime.
    async fn disable_strategy(&self, id: &str) -> Result<String, IpcError>;
    /// Request closing a position by UUID.
    async fn close_position(&self, position_id: Uuid) -> Result<String, IpcError>;
    /// Emergency flatten all positions and transition engine to `Killed` (§7.4).
    async fn panic_flatten(&self) -> Result<String, IpcError>;
    /// Manually clear `Killed` mode back to `Paused` (§7.4).
    async fn reset_kill(&self) -> Result<String, IpcError>;
    /// Acknowledge an operator alert by ID.
    async fn ack_alert(&self, id: &str) -> Result<String, IpcError>;
    /// Atomically switch active venue (`Spot <-> UsdsFutures`, §3.5, §10.1).
    async fn switch_venue(
        &self,
        _target: Venue,
        _policy: OnOpenPositionsArg,
        _actor: &str,
    ) -> Result<String, IpcError> {
        Err(IpcError::NotSupported(
            "switch_venue backend not configured".to_owned(),
        ))
    }
    /// Query recent executed trades filtered by optional `venue` and UTC millisecond range.
    async fn list_trades(
        &self,
        _venue: Option<Venue>,
        _from_ms: Option<i64>,
        _to_ms: Option<i64>,
    ) -> Vec<TradeRecord> {
        Vec::new()
    }
    /// Query candlestick bars for `(venue, symbol, tf)` up to `limit` bars.
    async fn list_klines(
        &self,
        _venue: Venue,
        _symbol: &str,
        _tf: &str,
        _limit: usize,
    ) -> Vec<KlineBar> {
        Vec::new()
    }
    /// Prometheus text exposition for `GET /metrics`.
    async fn prometheus_metrics(&self) -> String {
        let snap = self.snapshot().await;
        format!(
            "# HELP binance_bot_open_positions Number of currently open positions\n\
             # TYPE binance_bot_open_positions gauge\n\
             binance_bot_open_positions {}\n\
             # HELP binance_bot_ws_latency_ms WebSocket latency in milliseconds\n\
             # TYPE binance_bot_ws_latency_ms gauge\n\
             binance_bot_ws_latency_ms {}\n",
            snap.risk.open_positions, snap.engine.ws_latency_ms
        )
    }
}

/// Thread-safe idempotent command dispatcher with confirmation nonces and SQLite journal audit.
pub struct CommandDispatcher<B: IpcBackend> {
    backend: Arc<B>,
    audit_store: Option<Arc<Mutex<SqliteStore>>>,
    nonces: ConfirmNonceStore,
    idempotency: Mutex<IdempotencyCache>,
}

#[derive(Default)]
struct IdempotencyCache {
    order: VecDeque<Uuid>,
    results: HashMap<Uuid, CommandResultPayload>,
}

impl IdempotencyCache {
    fn get(&self, cmd_id: &Uuid) -> Option<CommandResultPayload> {
        self.results.get(cmd_id).cloned()
    }

    fn insert(&mut self, cmd_id: Uuid, result: CommandResultPayload) {
        if self.results.contains_key(&cmd_id) {
            return;
        }
        if self.order.len() >= MAX_IDEMPOTENCY_CACHE
            && let Some(oldest) = self.order.pop_front()
        {
            self.results.remove(&oldest);
        }
        self.order.push_back(cmd_id);
        self.results.insert(cmd_id, result);
    }
}

impl<B: IpcBackend> CommandDispatcher<B> {
    /// Create a new command dispatcher wrapping `backend`, optional SQLite audit store, and nonce store.
    #[must_use]
    pub fn new(
        backend: Arc<B>,
        audit_store: Option<Arc<Mutex<SqliteStore>>>,
        nonces: ConfirmNonceStore,
    ) -> Self {
        Self {
            backend,
            audit_store,
            nonces,
            idempotency: Mutex::new(IdempotencyCache::default()),
        }
    }

    /// Access the underlying backend (for snapshot/query endpoints).
    #[must_use]
    pub fn backend(&self) -> &Arc<B> {
        &self.backend
    }

    /// Construct the initial `hello` payload for a newly connected client.
    #[must_use]
    pub fn hello_payload(&self) -> HelloPayload {
        HelloPayload {
            protocol_version: PROTOCOL_VERSION,
            build_mode: self.backend.build_mode(),
            capabilities: vec![
                "uds".to_owned(),
                "http".to_owned(),
                "ws".to_owned(),
                "conflation".to_owned(),
            ],
            engine_instance_id: self.backend.engine_instance_id(),
        }
    }

    /// Dispatch a control command with scope check, idempotency, nonce enforcement, and journal audit.
    pub async fn dispatch(
        &self,
        principal: &str,
        scope: AuthScope,
        payload: CommandPayload,
    ) -> CommandResultPayload {
        let cmd_id = payload.cmd_id;
        let cmd_name = payload.cmd.name();
        let cmd_venue = payload.cmd.venue();

        // Authorization: all control commands require Full scope (§10.1).
        if !scope.can_mutate() {
            let res = CommandResultPayload {
                cmd_id,
                ok: false,
                code: "forbidden".to_owned(),
                message: "read-only token cannot execute mutating commands".to_owned(),
                confirm_nonce: None,
                expires_at_ms: None,
            };
            self.record_audit(principal, cmd_name, cmd_venue, &res)
                .await;
            return res;
        }

        // Idempotency check by cmd_id (§10.2).
        if let Some(cached) = self.idempotency.lock().await.get(&cmd_id) {
            return cached;
        }

        let res = self.execute_command(principal, payload).await;
        self.record_audit(principal, cmd_name, cmd_venue, &res)
            .await;
        self.idempotency.lock().await.insert(cmd_id, res.clone());
        res
    }

    async fn execute_command(
        &self,
        principal: &str,
        payload: CommandPayload,
    ) -> CommandResultPayload {
        let cmd_id = payload.cmd_id;
        match payload.cmd {
            CommandKind::Pause => Self::from_result(cmd_id, self.backend.pause().await),
            CommandKind::Resume => Self::from_result(cmd_id, self.backend.resume().await),
            CommandKind::SwitchVenue {
                target,
                on_open_positions,
                confirm_nonce,
            } => {
                if on_open_positions == OnOpenPositionsArg::Flatten {
                    if confirm_nonce.trim().is_empty() {
                        return CommandResultPayload {
                            cmd_id,
                            ok: false,
                            code: "invalid_confirm_nonce".to_owned(),
                            message: "flatten policy requires confirm_nonce".to_owned(),
                            confirm_nonce: None,
                            expires_at_ms: None,
                        };
                    }
                    if let Err(err) =
                        self.nonces
                            .consume(&confirm_nonce, ConfirmAction::SwitchVenue, principal)
                    {
                        return Self::from_result(cmd_id, Err(err));
                    }
                }
                Self::from_result(
                    cmd_id,
                    self.backend
                        .switch_venue(target, on_open_positions, principal)
                        .await,
                )
            }
            CommandKind::SetParam { key, value } => {
                Self::from_result(cmd_id, self.backend.set_param(&key, &value).await)
            }
            CommandKind::DisableStrategy { id } => {
                Self::from_result(cmd_id, self.backend.disable_strategy(&id).await)
            }
            CommandKind::ClosePosition { position_id } => {
                Self::from_result(cmd_id, self.backend.close_position(position_id).await)
            }
            CommandKind::RequestConfirm { action } => {
                let issued = self.nonces.issue(action, principal);
                CommandResultPayload {
                    cmd_id,
                    ok: true,
                    code: "ok".to_owned(),
                    message: format!("confirmation nonce issued for {}", action.as_str()),
                    confirm_nonce: Some(issued.nonce),
                    expires_at_ms: Some(issued.expires_at_ms),
                }
            }
            CommandKind::PanicFlatten { confirm_nonce } => {
                if let Err(err) =
                    self.nonces
                        .consume(&confirm_nonce, ConfirmAction::PanicFlatten, principal)
                {
                    return Self::from_err(cmd_id, &err);
                }
                Self::from_result(cmd_id, self.backend.panic_flatten().await)
            }
            CommandKind::ResetKill { confirm_nonce } => {
                if let Err(err) =
                    self.nonces
                        .consume(&confirm_nonce, ConfirmAction::ResetKill, principal)
                {
                    return Self::from_err(cmd_id, &err);
                }
                Self::from_result(cmd_id, self.backend.reset_kill().await)
            }
            CommandKind::AckAlert { id } => {
                Self::from_result(cmd_id, self.backend.ack_alert(&id).await)
            }
        }
    }

    fn from_result(cmd_id: Uuid, res: Result<String, IpcError>) -> CommandResultPayload {
        match res {
            Ok(message) => CommandResultPayload {
                cmd_id,
                ok: true,
                code: "ok".to_owned(),
                message,
                confirm_nonce: None,
                expires_at_ms: None,
            },
            Err(err) => Self::from_err(cmd_id, &err),
        }
    }

    fn from_err(cmd_id: Uuid, err: &IpcError) -> CommandResultPayload {
        CommandResultPayload {
            cmd_id,
            ok: false,
            code: err.code().to_owned(),
            message: err.to_string(),
            confirm_nonce: None,
            expires_at_ms: None,
        }
    }

    async fn record_audit(
        &self,
        principal: &str,
        command: &str,
        venue: Option<Venue>,
        result: &CommandResultPayload,
    ) {
        info!(
            principal,
            command,
            cmd_id = %result.cmd_id,
            ok = result.ok,
            code = %result.code,
            "ipc command audited"
        );
        let Some(store) = &self.audit_store else {
            return;
        };
        let ts_ms = unix_now_ms();
        let env = JournalEnvelope::new(JournalEvent::Command {
            cmd_id: result.cmd_id,
            principal: principal.to_owned(),
            command: command.to_owned(),
            venue,
            ok: result.ok,
            code: result.code.clone(),
            message: result.message.clone(),
            ts_ms,
        });
        let mut guard = store.lock().await;
        if let Err(err) = guard.append_event(ts_ms, &env) {
            warn!(error = %err, "failed to persist IPC command audit record");
        }
    }
}

/// Default stateful in-memory backend enforcing tighten-only `set_param` via `bot_config::is_tightening`
/// and §7.4 engine mode transitions (`Ready ⇄ Paused`, `Any -> Killed`, `Killed -> Paused`).
pub struct InMemoryIpcBackend {
    instance_id: Uuid,
    build_mode: TradingMode,
    state: RwLock<InMemoryState>,
}

struct InMemoryState {
    config: BotConfig,
    mode: EngineMode,
    snapshot: SnapshotPayload,
    klines: HashMap<(Venue, String, String), Vec<KlineBar>>,
    trades: Vec<TradeRecord>,
}

impl InMemoryIpcBackend {
    /// Construct a backend initialized from `config` (defaults to `TradingMode::Paper`).
    #[must_use]
    pub fn new(config: BotConfig) -> Self {
        let mode = EngineMode::Ready;
        let build_mode = config.general.mode;
        let snapshot = SnapshotPayload {
            venues: VenuesSummary {
                active: Venue::Spot,
                spot: VenueState::Active,
                futures: VenueState::Disabled,
                last_switch_ms: 0,
                switches_today: 0,
            },
            engine: EngineSummary {
                mode,
                build_mode,
                uptime_s: 0,
                ws_latency_ms: 0,
                clock_drift_ms: 0,
                rate: RateStatus {
                    weight_used: 0,
                    weight_limit: 6000,
                    orders_10s: 0,
                },
            },
            risk: RiskSummary {
                equity: Decimal::new(1000, 0),
                day_pnl_pct: Decimal::ZERO,
                week_pnl_pct: Decimal::ZERO,
                drawdown_pct: Decimal::ZERO,
                heat_pct: Decimal::ZERO,
                heat_max_pct: config.risk.portfolio_heat_max_pct,
                open_positions: 0,
                max_positions: config.risk.max_open_positions,
                btc_guard: "SECURE".to_owned(),
                limits: RiskLimitsSummary {
                    daily_halt_pct: config.risk.daily_loss_halt_pct,
                    kill_pct: config.risk.drawdown_kill_pct,
                },
            },
            positions: Vec::new(),
            orders: Vec::new(),
            signals_shadow: Vec::new(),
            alerts: Vec::new(),
        };
        Self {
            instance_id: Uuid::now_v7(),
            build_mode,
            state: RwLock::new(InMemoryState {
                config,
                mode,
                snapshot,
                klines: HashMap::new(),
                trades: Vec::new(),
            }),
        }
    }

    /// Read current runtime `BotConfig`.
    pub async fn config(&self) -> BotConfig {
        self.state.read().await.config.clone()
    }

    /// Replace the in-memory snapshot (synchronizing `mode` with `snapshot.engine.mode`).
    pub async fn set_snapshot(&self, snapshot: SnapshotPayload) {
        let mut guard = self.state.write().await;
        guard.mode = snapshot.engine.mode;
        guard.snapshot = snapshot;
    }

    /// Seed candlestick bars for `(venue, symbol, tf)`.
    pub async fn set_klines(&self, venue: Venue, symbol: String, tf: String, bars: Vec<KlineBar>) {
        let mut guard = self.state.write().await;
        guard.klines.insert((venue, symbol, tf), bars);
    }

    /// Seed recent trade records.
    pub async fn set_trades(&self, trades: Vec<TradeRecord>) {
        let mut guard = self.state.write().await;
        guard.trades = trades;
    }
}

#[async_trait]
impl IpcBackend for InMemoryIpcBackend {
    fn engine_instance_id(&self) -> Uuid {
        self.instance_id
    }

    fn build_mode(&self) -> TradingMode {
        self.build_mode
    }

    async fn snapshot(&self) -> SnapshotPayload {
        let guard = self.state.read().await;
        guard.snapshot.clone()
    }

    async fn engine_mode(&self) -> EngineMode {
        self.state.read().await.mode
    }

    async fn pause(&self) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        if guard.mode == EngineMode::Killed {
            return Err(IpcError::Forbidden(
                "cannot pause from Killed mode; use reset_kill --confirm first".to_owned(),
            ));
        }
        guard.mode = EngineMode::Paused;
        guard.snapshot.engine.mode = EngineMode::Paused;
        Ok("engine paused".to_owned())
    }

    async fn resume(&self) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        if guard.mode != EngineMode::Paused && guard.mode != EngineMode::Ready {
            return Err(IpcError::Forbidden(format!(
                "cannot resume from {:?} mode",
                guard.mode
            )));
        }
        guard.mode = EngineMode::Ready;
        guard.snapshot.engine.mode = EngineMode::Ready;
        Ok("engine resumed".to_owned())
    }

    async fn set_param(&self, key: &str, value: &str) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        let mut candidate = guard.config.clone();
        apply_param_to_config(&mut candidate, key, value)?;
        is_tightening(&guard.config, &candidate)
            .map_err(|err| IpcError::LoosenRejected(err.to_string()))?;
        guard.config = candidate;
        guard.snapshot.risk.heat_max_pct = guard.config.risk.portfolio_heat_max_pct;
        guard.snapshot.risk.max_positions = guard.config.risk.max_open_positions;
        guard.snapshot.risk.limits.daily_halt_pct = guard.config.risk.daily_loss_halt_pct;
        guard.snapshot.risk.limits.kill_pct = guard.config.risk.drawdown_kill_pct;
        Ok(format!("parameter '{key}' tightened to '{value}'"))
    }

    async fn disable_strategy(&self, id: &str) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        match id {
            "TrendPullback" | "trend_pullback" => {
                guard.config.strategy.trend_pullback.enabled = false;
            }
            "Breakout" | "breakout" => {
                guard.config.strategy.breakout.enabled = false;
            }
            "VwapReversion" | "vwap_reversion" => {
                guard.config.strategy.vwap_reversion.enabled = false;
            }
            "GainersMomentum" | "gainers_momentum" => {
                guard.config.strategy.gainers_momentum.enabled = false;
            }
            _ => {
                return Err(IpcError::NotFound(format!("unknown strategy '{id}'")));
            }
        }
        Ok(format!("strategy '{id}' disabled"))
    }

    async fn close_position(&self, position_id: Uuid) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        let before = guard.snapshot.positions.len();
        guard.snapshot.positions.retain(|p| p.id != position_id);
        if guard.snapshot.positions.len() == before {
            return Err(IpcError::NotFound(format!(
                "position '{position_id}' not found"
            )));
        }
        guard.snapshot.risk.open_positions =
            u32::try_from(guard.snapshot.positions.len()).unwrap_or(0);
        Ok(format!("close requested for position {position_id}"))
    }

    async fn panic_flatten(&self) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        let closed = guard.snapshot.positions.len();
        guard.snapshot.positions.clear();
        guard.snapshot.risk.open_positions = 0;
        guard.mode = EngineMode::Killed;
        guard.snapshot.engine.mode = EngineMode::Killed;
        Ok(format!(
            "panic flatten executed ({closed} positions closed); engine mode is now Killed"
        ))
    }

    async fn reset_kill(&self) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        if guard.mode != EngineMode::Killed {
            return Err(IpcError::BadRequest(format!(
                "engine is in {:?} mode, not Killed",
                guard.mode
            )));
        }
        guard.mode = EngineMode::Paused;
        guard.snapshot.engine.mode = EngineMode::Paused;
        Ok("kill state reset; engine is now Paused".to_owned())
    }

    async fn ack_alert(&self, id: &str) -> Result<String, IpcError> {
        let mut guard = self.state.write().await;
        let Some(alert) = guard.snapshot.alerts.iter_mut().find(|a| a.id == id) else {
            return Err(IpcError::NotFound(format!("alert '{id}' not found")));
        };
        alert.acked = true;
        Ok(format!("alert '{id}' acknowledged"))
    }

    async fn list_trades(
        &self,
        venue: Option<Venue>,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
    ) -> Vec<TradeRecord> {
        let guard = self.state.read().await;
        guard
            .trades
            .iter()
            .filter(|t| venue.is_none_or(|v| t.venue == v))
            .filter(|t| from_ms.is_none_or(|lo| t.ts_ms >= lo))
            .filter(|t| to_ms.is_none_or(|hi| t.ts_ms <= hi))
            .cloned()
            .collect()
    }

    async fn list_klines(
        &self,
        venue: Venue,
        symbol: &str,
        tf: &str,
        limit: usize,
    ) -> Vec<KlineBar> {
        let guard = self.state.read().await;
        let Some(bars) = guard.klines.get(&(venue, symbol.to_owned(), tf.to_owned())) else {
            return Vec::new();
        };
        let take_n = limit.min(bars.len());
        let skip_n = bars.len().saturating_sub(take_n);
        bars.iter().skip(skip_n).cloned().collect()
    }
}

/// Apply a whitelisted runtime-tunable dotted parameter key to a candidate [`BotConfig`].
/// Tighten-only validation is then enforced by `bot_config::is_tightening`.
fn apply_param_to_config(cfg: &mut BotConfig, key: &str, value: &str) -> Result<(), IpcError> {
    let parse_dec = |v: &str| {
        Decimal::from_str_exact(v)
            .map_err(|_| IpcError::BadRequest(format!("invalid decimal value '{v}' for '{key}'")))
    };
    let parse_u32 = |v: &str| {
        v.parse::<u32>()
            .map_err(|_| IpcError::BadRequest(format!("invalid u32 value '{v}' for '{key}'")))
    };
    match key {
        "risk.risk_per_trade_pct" => cfg.risk.risk_per_trade_pct = parse_dec(value)?,
        "risk.max_open_positions" => cfg.risk.max_open_positions = parse_u32(value)?,
        "risk.portfolio_heat_max_pct" => cfg.risk.portfolio_heat_max_pct = parse_dec(value)?,
        "risk.max_notional_per_position_pct" => {
            cfg.risk.max_notional_per_position_pct = parse_dec(value)?;
        }
        "risk.max_total_exposure_pct" => cfg.risk.max_total_exposure_pct = parse_dec(value)?,
        "risk.max_same_direction_correlated" => {
            cfg.risk.max_same_direction_correlated = parse_u32(value)?;
        }
        "risk.daily_loss_halt_pct" => cfg.risk.daily_loss_halt_pct = parse_dec(value)?,
        "risk.weekly_loss_halt_pct" => cfg.risk.weekly_loss_halt_pct = parse_dec(value)?,
        "risk.drawdown_kill_pct" => cfg.risk.drawdown_kill_pct = parse_dec(value)?,
        "risk.max_entry_slippage_bps" => cfg.risk.max_entry_slippage_bps = parse_dec(value)?,
        "risk.max_orders_per_day" => cfg.risk.max_orders_per_day = parse_u32(value)?,
        _ => {
            return Err(IpcError::BadRequest(format!(
                "parameter '{key}' is not runtime-modifiable"
            )));
        }
    }
    Ok(())
}
