//! Versioned append-only journal events and persisted record DTOs (§8.2).

use bot_types::{
    EngineMode, Order, OrderStatus, Position, PositionState, Price, Qty, Symbol, Venue, VenueState,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

/// Current schema version for serialized [`JournalEnvelope`] payloads.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

/// Persisted trade fill record keyed by `(venue, symbol, trade_id)` (§8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillRecord {
    /// Venue.
    pub venue: Venue,
    /// Exchange trade ID (idempotency key together with `(venue, symbol)`).
    pub trade_id: i64,
    /// Symbol.
    pub symbol: Symbol,
    /// Deterministic client order ID.
    pub client_id: String,
    /// Execution price (`Decimal` string in JSON).
    pub price: Price,
    /// Executed quantity (`Decimal` string in JSON).
    pub qty: Qty,
    /// Commission paid (`Decimal` string in JSON).
    #[serde(with = "rust_decimal::serde::str")]
    pub fee: Decimal,
    /// Asset in which commission was charged.
    pub fee_asset: String,
    /// Whether this fill was maker side.
    pub is_maker: bool,
    /// Trade timestamp (UTC ms).
    pub ts_ms: i64,
}

/// Persisted per-venue lifecycle state row (`venue_state` table, §8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VenueStateRecord {
    /// Venue.
    pub venue: Venue,
    /// Current venue lifecycle state.
    pub state: VenueState,
    /// Timestamp (UTC ms) since entering `state`.
    pub since_ms: i64,
    /// Timestamp (UTC ms) of the last venue switch, if any.
    pub last_switch_ms: Option<i64>,
    /// Number of switches performed today.
    pub switches_today: u32,
}

/// Versioned envelope written to `journal.payload` as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JournalEnvelope {
    /// Schema version (must be `<= EVENT_SCHEMA_VERSION`).
    pub schema_version: u32,
    /// Event body.
    pub event: JournalEvent,
}

impl JournalEnvelope {
    /// Wrap a [`JournalEvent`] in the current schema version.
    #[must_use]
    pub const fn new(event: JournalEvent) -> Self {
        Self {
            schema_version: EVENT_SCHEMA_VERSION,
            event,
        }
    }

    /// Machine-readable event kind string stored in `journal.kind`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        self.event.kind()
    }

    /// Associated venue, if this event is venue-scoped.
    #[must_use]
    pub const fn venue(&self) -> Option<Venue> {
        self.event.venue()
    }

    /// Associated position ID, if any.
    #[must_use]
    pub const fn position_id(&self) -> Option<Uuid> {
        self.event.position_id()
    }

    /// Associated client order ID, if any.
    #[must_use]
    pub fn client_id(&self) -> Option<&str> {
        self.event.client_id()
    }
}

/// Append-only domain events persisted to the SQLite `journal` table (§5.1, §8.1–§8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum JournalEvent {
    /// Journal-first order intent recorded BEFORE sending to the exchange (§5.1).
    OrderIntended {
        /// Full initial order record.
        order: Order,
    },
    /// Exchange acknowledgment or lifecycle status update for an existing order.
    OrderStatusUpdated {
        /// Venue.
        venue: Venue,
        /// Deterministic client order ID.
        client_id: String,
        /// Exchange numeric order ID, if assigned.
        exchange_id: Option<i64>,
        /// Updated order status.
        status: OrderStatus,
        /// Cumulative filled quantity.
        filled_qty: Qty,
        /// Volume-weighted average fill price, if any fills occurred.
        avg_fill_price: Option<Price>,
        /// Update timestamp (UTC RFC 3339).
        #[serde(with = "time::serde::rfc3339")]
        updated_at: OffsetDateTime,
    },
    /// Idempotent trade fill event.
    FillRecorded {
        /// Trade fill record.
        fill: FillRecord,
    },
    /// New position opened after entry fill.
    PositionOpened {
        /// Initial position state.
        position: Position,
    },
    /// Protective stop price moved (break-even or trailing ratchet).
    PositionStopMoved {
        /// Venue.
        venue: Venue,
        /// Position UUID.
        position_id: Uuid,
        /// New stop price.
        new_stop_price: Price,
        /// Whether break-even has been achieved.
        breakeven_done: bool,
    },
    /// High-water / low-water mark updated (journaled when change >= 0.1 ATR).
    PositionBestPriceUpdated {
        /// Venue.
        venue: Venue,
        /// Position UUID.
        position_id: Uuid,
        /// Updated extreme price.
        best_price: Price,
    },
    /// Partial TP1 fill recorded on a position.
    PositionTp1Done {
        /// Venue.
        venue: Venue,
        /// Position UUID.
        position_id: Uuid,
        /// Remaining open quantity.
        qty_open: Qty,
        /// Cumulative realized `PnL` in quote currency.
        #[serde(with = "rust_decimal::serde::str")]
        realized_pnl_quote: Decimal,
        /// Cumulative fees paid in quote currency.
        #[serde(with = "rust_decimal::serde::str")]
        fees_paid_quote: Decimal,
    },
    /// Position lifecycle state transition (including closing).
    PositionStateChanged {
        /// Venue.
        venue: Venue,
        /// Position UUID.
        position_id: Uuid,
        /// New position state.
        state: PositionState,
        /// Remaining open quantity.
        qty_open: Qty,
        /// Cumulative realized `PnL` in quote currency.
        #[serde(with = "rust_decimal::serde::str")]
        realized_pnl_quote: Decimal,
        /// Cumulative fees paid in quote currency.
        #[serde(with = "rust_decimal::serde::str")]
        fees_paid_quote: Decimal,
        /// Optional close reason.
        close_reason: Option<String>,
    },
    /// Global engine mode transition (`Ready`, `Paused`, `ReduceOnly`, `Halted`, `Killed`).
    EngineModeChanged {
        /// New engine mode.
        mode: EngineMode,
        /// Reason for transition.
        reason: String,
    },
    /// Per-venue lifecycle state transition (`Disabled`, `Activating`, `Active`, `Draining`).
    VenueStateChanged {
        /// Updated venue state row.
        record: VenueStateRecord,
    },
    /// Persistent risk state key/value update (`day_start_equity`, `peak_equity`, etc.).
    RiskStateSet {
        /// Risk state key.
        key: String,
        /// Risk state value (Decimal serialized as string or scalar text).
        value: String,
    },
    /// Audited IPC control command execution (`journal.kind = "Command"`, §10.1).
    Command {
        /// Idempotency UUID of the command.
        cmd_id: Uuid,
        /// Authenticated caller principal (e.g. `uds:uid=1000` or `http:full`).
        principal: String,
        /// Machine-readable command name (e.g. `pause`, `panic_flatten`).
        command: String,
        /// Optional target venue if the command is venue-scoped.
        venue: Option<Venue>,
        /// Whether the command succeeded.
        ok: bool,
        /// Machine-readable outcome code (e.g. `ok`, `not_supported`, `forbidden`).
        code: String,
        /// Human-readable outcome summary (never contains secrets).
        message: String,
        /// Execution timestamp (UTC ms).
        ts_ms: i64,
    },
}

impl JournalEvent {
    /// Stable event kind tag stored in `journal.kind`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::OrderIntended { .. } => "OrderIntended",
            Self::OrderStatusUpdated { .. } => "OrderStatusUpdated",
            Self::FillRecorded { .. } => "FillRecorded",
            Self::PositionOpened { .. } => "PositionOpened",
            Self::PositionStopMoved { .. } => "PositionStopMoved",
            Self::PositionBestPriceUpdated { .. } => "PositionBestPriceUpdated",
            Self::PositionTp1Done { .. } => "PositionTp1Done",
            Self::PositionStateChanged { .. } => "PositionStateChanged",
            Self::EngineModeChanged { .. } => "EngineModeChanged",
            Self::VenueStateChanged { .. } => "VenueStateChanged",
            Self::RiskStateSet { .. } => "RiskStateSet",
            Self::Command { .. } => "Command",
        }
    }

    /// Associated venue, if event is venue-scoped.
    #[must_use]
    pub const fn venue(&self) -> Option<Venue> {
        match self {
            Self::OrderIntended { order } => Some(order.venue),
            Self::OrderStatusUpdated { venue, .. }
            | Self::PositionStopMoved { venue, .. }
            | Self::PositionBestPriceUpdated { venue, .. }
            | Self::PositionTp1Done { venue, .. }
            | Self::PositionStateChanged { venue, .. } => Some(*venue),
            Self::FillRecorded { fill } => Some(fill.venue),
            Self::PositionOpened { position } => Some(position.venue),
            Self::VenueStateChanged { record } => Some(record.venue),
            Self::Command { venue, .. } => *venue,
            Self::EngineModeChanged { .. } | Self::RiskStateSet { .. } => None,
        }
    }

    /// Associated position UUID, if any.
    #[must_use]
    pub const fn position_id(&self) -> Option<Uuid> {
        match self {
            Self::OrderIntended { order } => order.position_id,
            Self::PositionOpened { position } => Some(position.id),
            Self::PositionStopMoved { position_id, .. }
            | Self::PositionBestPriceUpdated { position_id, .. }
            | Self::PositionTp1Done { position_id, .. }
            | Self::PositionStateChanged { position_id, .. } => Some(*position_id),
            Self::OrderStatusUpdated { .. }
            | Self::FillRecorded { .. }
            | Self::EngineModeChanged { .. }
            | Self::VenueStateChanged { .. }
            | Self::RiskStateSet { .. }
            | Self::Command { .. } => None,
        }
    }

    /// Associated client order ID, if any.
    #[must_use]
    pub fn client_id(&self) -> Option<&str> {
        match self {
            Self::OrderIntended { order } => Some(&order.client_id),
            Self::OrderStatusUpdated { client_id, .. } => Some(client_id),
            Self::FillRecorded { fill } => Some(&fill.client_id),
            Self::PositionOpened { .. }
            | Self::PositionStopMoved { .. }
            | Self::PositionBestPriceUpdated { .. }
            | Self::PositionTp1Done { .. }
            | Self::PositionStateChanged { .. }
            | Self::EngineModeChanged { .. }
            | Self::VenueStateChanged { .. }
            | Self::RiskStateSet { .. }
            | Self::Command { .. } => None,
        }
    }
}
