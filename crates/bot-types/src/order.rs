//! Order domain types (§3.4).

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{Price, Qty, Side, Symbol, Venue};

/// Order lifecycle status.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderStatus {
    /// Submitted locally, awaiting exchange acknowledgment.
    PendingNew,
    /// Accepted and resting on the exchange.
    New,
    /// Partially executed.
    PartiallyFilled,
    /// Fully executed.
    Filled,
    /// Canceled before full execution.
    Canceled,
    /// Rejected by the exchange or gatekeeper.
    Rejected,
    /// Expired by time-in-force or self-trade prevention.
    Expired,
    /// Ambiguous outcome; must be reconciled via exchange query.
    UnknownNeedsQuery,
}

/// Exchange order type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderKind {
    /// Post-only limit order (`LIMIT_MAKER` / `GTX`).
    LimitMaker,
    /// Standard limit order.
    Limit,
    /// Market order.
    Market,
    /// Stop-loss limit order.
    StopLossLimit,
    /// Take-profit limit order.
    TakeProfitLimit,
    /// Leg belonging to an OCO order list.
    OcoLeg,
}

/// Semantic role of an order in a position lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum OrderPurpose {
    /// Opens a position.
    Entry,
    /// Protective stop-loss order.
    ProtectStop,
    /// First take-profit target.
    TakeProfit1,
    /// Second take-profit target.
    TakeProfit2,
    /// Emergency market/IOC exit triggered by circuit breaker.
    EmergencyExit,
    /// Operator-initiated flatten.
    ManualFlatten,
}

/// Venue-tagged order record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Order {
    /// Venue where this order lives; unique key is `(venue, client_id)`.
    pub venue: Venue,
    /// Deterministic client order ID (`^[A-Za-z0-9_-]{1,36}$`).
    pub client_id: String,
    /// Exchange-assigned numeric order ID, if acknowledged.
    pub exchange_id: Option<i64>,
    /// Instrument symbol.
    pub symbol: Symbol,
    /// Order side.
    pub side: Side,
    /// Order kind.
    pub kind: OrderKind,
    /// Limit price, if applicable.
    pub price: Option<Price>,
    /// Requested order quantity.
    pub qty: Qty,
    /// Cumulative filled quantity.
    pub filled_qty: Qty,
    /// Volume-weighted average fill price, if any fills occurred.
    pub avg_fill_price: Option<Price>,
    /// Current lifecycle status.
    pub status: OrderStatus,
    /// Owning position ID, if associated.
    pub position_id: Option<Uuid>,
    /// Semantic purpose of this order.
    pub purpose: OrderPurpose,
    /// Creation timestamp (UTC, RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    pub created_at: OffsetDateTime,
    /// Last update timestamp (UTC, RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    pub updated_at: OffsetDateTime,
}
