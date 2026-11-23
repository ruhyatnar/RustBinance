//! IPC Protocol v1 wire types matching `protocol/ipc-v1.schema.json` (§10.2–§10.3).
//!
//! All monetary/decimal values are serialized as strings (`rust_decimal::serde::str`),
//! timestamps are UTC epoch milliseconds (`i64`), and every position/order/market item
//! carries its `venue`.

use bot_types::{EngineMode, TradingMode, Venue, VenueState};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Current IPC wire protocol version.
pub const PROTOCOL_VERSION: u32 = 1;

/// Wire envelope shared by UDS (NDJSON) and WebSocket (one JSON message per frame).
///
/// Serializes as:
/// `{ "v": 1, "type": "<kind>", "seq": 1042, "ts_ms": 1790000000000, "payload": { ... } }`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WireEnvelope<T> {
    /// Wire protocol version (always `1`).
    pub v: u32,
    /// Monotonically increasing sequence number per connection.
    pub seq: u64,
    /// UTC timestamp in milliseconds.
    pub ts_ms: i64,
    /// Tagged message type and payload (`"type"` + `"payload"`).
    #[serde(flatten)]
    pub body: T,
}

impl WireEnvelope<ServerMessage> {
    /// Build a v1 server-to-client envelope.
    #[must_use]
    pub const fn server(seq: u64, ts_ms: i64, body: ServerMessage) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            seq,
            ts_ms,
            body,
        }
    }
}

impl WireEnvelope<ClientMessage> {
    /// Build a v1 client-to-server envelope.
    #[must_use]
    pub const fn client(seq: u64, ts_ms: i64, body: ClientMessage) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            seq,
            ts_ms,
            body,
        }
    }
}

/// Server-to-client tagged message variants (§10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ServerMessage {
    /// Sent once upon connection establishment.
    Hello(HelloPayload),
    /// Full state snapshot (on connect and on request).
    Snapshot(SnapshotPayload),
    /// Incremental state delta (conflated at `telemetry_hz`).
    Delta(DeltaPayload),
    /// Watchlist market summary (conflated at `<= 5 Hz`).
    Market(MarketPayload),
    /// Immediate domain/operational event (fill, stop moved, mode changed, guard, error).
    Event(EventPayload),
    /// 1 Hz liveness heartbeat.
    Heartbeat(HeartbeatPayload),
    /// Response to a client command identified by `cmd_id`.
    CommandResult(CommandResultPayload),
}

/// Client-to-server tagged message variants (§10.1–§10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "payload", rename_all = "snake_case")]
pub enum ClientMessage {
    /// Control command with idempotency `cmd_id`.
    Command(CommandPayload),
    /// WebSocket first-message authentication (alternative to header/subprotocol; never query string).
    Auth(AuthPayload),
    /// Request a fresh full snapshot (e.g. after detecting a `seq` gap).
    RequestSnapshot(EmptyPayload),
}

/// Empty JSON object `{}`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmptyPayload {}

/// First-message WebSocket authentication payload.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthPayload {
    /// Bearer token string (redacted in `Debug`).
    pub token: String,
}

impl std::fmt::Debug for AuthPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthPayload")
            .field("token", &"[REDACTED]")
            .finish()
    }
}

/// Alert / event severity level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlertLevel {
    /// Informational message.
    Info,
    /// Non-fatal warning.
    Warn,
    /// Error requiring operator attention.
    Error,
    /// Critical risk or safety condition.
    Critical,
}

/// Exchange API rate budget snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RateStatus {
    /// Request weight consumed in current window.
    pub weight_used: u32,
    /// Request weight limit per minute.
    pub weight_limit: u32,
    /// Orders placed in the last 10 seconds.
    pub orders_10s: u32,
}

/// Multi-venue lifecycle summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenuesSummary {
    /// Currently active venue.
    pub active: Venue,
    /// Spot venue lifecycle state.
    pub spot: VenueState,
    /// USDⓈ-M Futures venue lifecycle state.
    pub futures: VenueState,
    /// Timestamp (UTC ms) of the last venue switch (`0` if never switched).
    pub last_switch_ms: u64,
    /// Number of venue switches performed today.
    pub switches_today: u32,
}

/// Engine runtime health summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineSummary {
    /// Current engine circuit-breaker mode.
    pub mode: EngineMode,
    /// Active build/execution mode (`paper`, `testnet`, `live`).
    pub build_mode: TradingMode,
    /// Daemon uptime in seconds.
    pub uptime_s: u64,
    /// Measured WebSocket round-trip latency in milliseconds.
    pub ws_latency_ms: u64,
    /// Clock drift vs Binance server time in milliseconds.
    pub clock_drift_ms: i64,
    /// API rate-limit usage.
    pub rate: RateStatus,
}

/// Active circuit-breaker threshold summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskLimitsSummary {
    /// Daily loss halt percentage (negative string, e.g. `"-2.0"`).
    #[serde(with = "rust_decimal::serde::str")]
    pub daily_halt_pct: Decimal,
    /// Drawdown kill percentage (negative string, e.g. `"-10.0"`).
    #[serde(with = "rust_decimal::serde::str")]
    pub kill_pct: Decimal,
}

/// Portfolio risk summary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskSummary {
    /// Combined portfolio equity in quote USD.
    #[serde(with = "rust_decimal::serde::str")]
    pub equity: Decimal,
    /// Daily `PnL` percentage.
    #[serde(with = "rust_decimal::serde::str")]
    pub day_pnl_pct: Decimal,
    /// Weekly `PnL` percentage.
    #[serde(with = "rust_decimal::serde::str")]
    pub week_pnl_pct: Decimal,
    /// Peak-to-current drawdown percentage.
    #[serde(with = "rust_decimal::serde::str")]
    pub drawdown_pct: Decimal,
    /// Current open portfolio heat percentage.
    #[serde(with = "rust_decimal::serde::str")]
    pub heat_pct: Decimal,
    /// Configured maximum portfolio heat percentage.
    #[serde(with = "rust_decimal::serde::str")]
    pub heat_max_pct: Decimal,
    /// Number of currently open positions.
    pub open_positions: u32,
    /// Maximum allowed open positions.
    pub max_positions: u32,
    /// BTC Crash Guard state (`"SECURE"`, `"TRIGGERED"`, `"COOLDOWN"`).
    pub btc_guard: String,
    /// Configured halt and kill thresholds.
    pub limits: RiskLimitsSummary,
}

/// UI view of an open position.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionView {
    /// Position UUID.
    pub id: Uuid,
    /// Venue.
    pub venue: Venue,
    /// Symbol (e.g. `"ETHUSDT"`).
    pub symbol: String,
    /// Strategy name (e.g. `"TrendPullback"`).
    pub strategy: String,
    /// Lifecycle state name (e.g. `"OpenProtected"`).
    pub state: String,
    /// Entry price.
    #[serde(with = "rust_decimal::serde::str")]
    pub entry: Decimal,
    /// Current mark/last price.
    #[serde(with = "rust_decimal::serde::str")]
    pub mark: Decimal,
    /// Open base quantity.
    #[serde(with = "rust_decimal::serde::str")]
    pub qty: Decimal,
    /// Active protective stop price.
    #[serde(with = "rust_decimal::serde::str")]
    pub stop: Decimal,
    /// First take-profit target.
    #[serde(with = "rust_decimal::serde::str")]
    pub tp1: Decimal,
    /// Second take-profit target.
    #[serde(with = "rust_decimal::serde::str")]
    pub tp2: Decimal,
    /// Current unrealized profit in R-multiples.
    #[serde(with = "rust_decimal::serde::str")]
    pub profit_r: Decimal,
    /// Unrealized `PnL` in quote currency.
    #[serde(with = "rust_decimal::serde::str")]
    pub unrealized_quote: Decimal,
    /// Position age in seconds.
    pub age_s: u64,
    /// Whether TP1 partial fill is complete.
    pub tp1_done: bool,
    /// Whether stop has moved to breakeven.
    pub be_done: bool,
}

/// UI view of an active order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderView {
    /// Deterministic client order ID.
    pub client_id: String,
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: String,
    /// Order side (`"Buy"` / `"Sell"`).
    pub side: String,
    /// Order kind (`"LimitMaker"`, `"Market"`, etc.).
    pub kind: String,
    /// Order purpose (`"Entry"`, `"StopLoss"`, `"TakeProfit1"`, etc.).
    pub purpose: String,
    /// Order status (`"Working"`, `"PartiallyFilled"`, etc.).
    pub status: String,
    /// Optional limit/stop price as decimal string.
    #[serde(default, with = "rust_decimal::serde::str_option")]
    pub price: Option<Decimal>,
    /// Order quantity.
    #[serde(with = "rust_decimal::serde::str")]
    pub qty: Decimal,
    /// Filled quantity.
    #[serde(with = "rust_decimal::serde::str")]
    pub filled_qty: Decimal,
}

/// UI view of a shadow strategy signal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShadowSignalView {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: String,
    /// Strategy name.
    pub strategy: String,
    /// Direction (`"Long"` / `"Short"`).
    pub direction: String,
    /// Proposed entry price.
    #[serde(with = "rust_decimal::serde::str")]
    pub entry: Decimal,
    /// Proposed stop price.
    #[serde(with = "rust_decimal::serde::str")]
    pub stop: Decimal,
    /// Signal timestamp (UTC ms).
    pub ts_ms: i64,
}

/// UI alert item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlertView {
    /// Unique alert identifier.
    pub id: String,
    /// Optional associated venue.
    #[serde(default)]
    pub venue: Option<Venue>,
    /// Alert severity level.
    pub level: AlertLevel,
    /// Human-readable alert message.
    pub msg: String,
    /// Whether the operator has acknowledged this alert.
    #[serde(default)]
    pub acked: bool,
}

/// `hello` message payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloPayload {
    /// Wire protocol version (`1`).
    pub protocol_version: u32,
    /// Build/execution mode.
    pub build_mode: TradingMode,
    /// Supported server capabilities.
    pub capabilities: Vec<String>,
    /// Unique engine process instance UUID.
    pub engine_instance_id: Uuid,
}

/// `snapshot` message payload (§10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPayload {
    /// Venues summary.
    pub venues: VenuesSummary,
    /// Engine health summary.
    pub engine: EngineSummary,
    /// Portfolio risk summary.
    pub risk: RiskSummary,
    /// Open positions.
    pub positions: Vec<PositionView>,
    /// Open orders.
    pub orders: Vec<OrderView>,
    /// Recent shadow strategy signals.
    pub signals_shadow: Vec<ShadowSignalView>,
    /// Active alerts.
    pub alerts: Vec<AlertView>,
}

/// `delta` message payload (changed fields since previous `seq`).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeltaPayload {
    /// Updated venues summary, if changed.
    #[serde(default)]
    pub venues: Option<VenuesSummary>,
    /// Updated engine summary, if changed.
    #[serde(default)]
    pub engine: Option<EngineSummary>,
    /// Updated risk summary, if changed.
    #[serde(default)]
    pub risk: Option<RiskSummary>,
    /// Updated positions list, if changed.
    #[serde(default)]
    pub positions: Option<Vec<PositionView>>,
    /// Updated orders list, if changed.
    #[serde(default)]
    pub orders: Option<Vec<OrderView>>,
    /// Updated alerts list, if changed.
    #[serde(default)]
    pub alerts: Option<Vec<AlertView>>,
}

/// Conflated market tick summary for a watchlist symbol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketTickSummary {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: String,
    /// Best bid price.
    #[serde(with = "rust_decimal::serde::str")]
    pub bid: Decimal,
    /// Best ask price.
    #[serde(with = "rust_decimal::serde::str")]
    pub ask: Decimal,
    /// Last trade price.
    #[serde(with = "rust_decimal::serde::str")]
    pub last: Decimal,
    /// Bid-ask spread in basis points.
    #[serde(with = "rust_decimal::serde::str")]
    pub spread_bps: Decimal,
    /// Quote timestamp (UTC ms).
    pub ts_ms: i64,
}

/// `market` message payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketPayload {
    /// Conflated market ticks.
    pub ticks: Vec<MarketTickSummary>,
}

/// `event` message payload (immediate important event).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventPayload {
    /// Optional associated venue.
    #[serde(default)]
    pub venue: Option<Venue>,
    /// Optional associated symbol.
    #[serde(default)]
    pub symbol: Option<String>,
    /// Event kind (e.g. `"Fill"`, `"StopMoved"`, `"ModeChanged"`, `"GuardTriggered"`).
    pub kind: String,
    /// Severity level.
    pub level: AlertLevel,
    /// Human-readable event description.
    pub message: String,
}

/// `heartbeat` message payload (1 Hz).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeartbeatPayload {
    /// Current engine circuit-breaker state.
    pub engine_state: EngineMode,
    /// WebSocket latency in milliseconds.
    pub ws_latency_ms: u64,
    /// Consumed API weight.
    pub weight_used: u32,
    /// API weight limit.
    pub weight_limit: u32,
}

/// `command_result` message payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandResultPayload {
    /// Echoed idempotency UUID from the client command.
    pub cmd_id: Uuid,
    /// Whether the command succeeded.
    pub ok: bool,
    /// Machine-readable result code (`"ok"`, `"not_supported"`, `"forbidden"`, etc.).
    pub code: String,
    /// Human-readable result message.
    pub message: String,
    /// Issued confirmation nonce (present on successful `request_confirm`).
    #[serde(default)]
    pub confirm_nonce: Option<String>,
    /// Nonce expiration timestamp in UTC ms (present on successful `request_confirm`).
    #[serde(default)]
    pub expires_at_ms: Option<i64>,
}

/// Open-position handling policy argument for `switch_venue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnOpenPositionsArg {
    /// Refuse to switch while positions are open.
    Refuse,
    /// Drain existing venue until flat, then switch.
    Drain,
    /// Flatten open positions before switching.
    Flatten,
}

/// Destructive/sensitive actions that require a 10-second `confirm_nonce`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfirmAction {
    /// Emergency flatten all positions and enter `Killed` mode.
    PanicFlatten,
    /// Manually clear `Killed` mode after operator investigation.
    ResetKill,
    /// Switch active venue (stubbed until M16).
    SwitchVenue,
}

impl ConfirmAction {
    /// Stable string name for audit logs.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PanicFlatten => "panic_flatten",
            Self::ResetKill => "reset_kill",
            Self::SwitchVenue => "switch_venue",
        }
    }
}

/// Tagged control command variants (§10.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum CommandKind {
    /// Pause new entries while continuing to manage open positions and protections.
    Pause,
    /// Resume entries from `Paused` mode.
    Resume,
    /// Switch active venue (stubbed: returns `not_supported` until M16).
    SwitchVenue {
        /// Target venue.
        target: Venue,
        /// Open position policy.
        on_open_positions: OnOpenPositionsArg,
        /// Confirmation nonce.
        confirm_nonce: String,
    },
    /// Tighten-only runtime configuration update.
    SetParam {
        /// Dotted parameter key (e.g. `"risk.risk_per_trade_pct"`).
        key: String,
        /// New value as string.
        value: String,
    },
    /// Disable a strategy at runtime (cannot be loosened/re-enabled at runtime).
    DisableStrategy {
        /// Strategy identifier.
        id: String,
    },
    /// Request orderly market close of an open position.
    ClosePosition {
        /// Position UUID.
        position_id: Uuid,
    },
    /// Request a 10-second single-use confirmation nonce for a destructive command.
    RequestConfirm {
        /// Action the nonce will authorize.
        action: ConfirmAction,
    },
    /// Emergency flatten all positions and transition to `Killed`.
    PanicFlatten {
        /// Nonce obtained via `request_confirm`.
        confirm_nonce: String,
    },
    /// Manually reset `Killed` mode back to `Paused`.
    ResetKill {
        /// Nonce obtained via `request_confirm`.
        confirm_nonce: String,
    },
    /// Acknowledge an operator alert.
    AckAlert {
        /// Alert identifier.
        id: String,
    },
}

impl CommandKind {
    /// Stable command name for audit records and metrics.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Pause => "pause",
            Self::Resume => "resume",
            Self::SwitchVenue { .. } => "switch_venue",
            Self::SetParam { .. } => "set_param",
            Self::DisableStrategy { .. } => "disable_strategy",
            Self::ClosePosition { .. } => "close_position",
            Self::RequestConfirm { .. } => "request_confirm",
            Self::PanicFlatten { .. } => "panic_flatten",
            Self::ResetKill { .. } => "reset_kill",
            Self::AckAlert { .. } => "ack_alert",
        }
    }

    /// Associated venue, if explicitly carried by the command.
    #[must_use]
    pub const fn venue(&self) -> Option<Venue> {
        match self {
            Self::SwitchVenue { target, .. } => Some(*target),
            Self::Pause
            | Self::Resume
            | Self::SetParam { .. }
            | Self::DisableStrategy { .. }
            | Self::ClosePosition { .. }
            | Self::RequestConfirm { .. }
            | Self::PanicFlatten { .. }
            | Self::ResetKill { .. }
            | Self::AckAlert { .. } => None,
        }
    }
}

/// `command` client payload carrying an idempotency `cmd_id` and tagged command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandPayload {
    /// Client-generated idempotency UUID.
    pub cmd_id: Uuid,
    /// Tagged command fields (`"cmd": "..."`, plus command-specific arguments).
    #[serde(flatten)]
    pub cmd: CommandKind,
}

/// Uniform HTTP error response body (§10.3).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpErrorResponse {
    /// Nested error object.
    pub error: HttpErrorBody,
}

/// Inner error object of [`HttpErrorResponse`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HttpErrorBody {
    /// Machine-readable error code.
    pub code: String,
    /// Human-readable error message.
    pub message: String,
}

impl HttpErrorResponse {
    /// Construct a uniform HTTP error response.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            error: HttpErrorBody {
                code: code.into(),
                message: message.into(),
            },
        }
    }
}
