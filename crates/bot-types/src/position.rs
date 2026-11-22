//! Position domain types (§3.4).

use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use time::OffsetDateTime;
use uuid::Uuid;

use crate::{Direction, Price, Qty, StrategyId, Symbol, Venue};

/// Position lifecycle state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PositionState {
    /// Entry order placed, not yet filled.
    EntryPending,
    /// Entry partially filled.
    EntryPartial,
    /// Open with verified exchange-side stop protection.
    OpenProtected,
    /// Open without verified exchange-side stop protection (urgent recovery state).
    OpenUnprotected,
    /// Exit order in flight.
    ExitPending,
    /// Fully closed.
    Closed,
    /// Discovered during reconciliation without matching local journal state.
    Orphaned,
}

/// Open or historical trading position across Spot or USDⓈ-M Futures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Position {
    /// Unique local position identifier.
    pub id: Uuid,
    /// Instrument symbol.
    pub symbol: Symbol,
    /// Venue where this position is held.
    pub venue: Venue,
    /// Trade direction.
    pub direction: Direction,
    /// Strategy that originated this position.
    pub strategy: StrategyId,
    /// Average entry price.
    pub entry_price: Price,
    /// Initial filled quantity.
    pub qty_initial: Qty,
    /// Remaining open quantity.
    pub qty_open: Qty,
    /// Initial 1R risk distance (`|entry - initial_stop|`) in price units.
    #[serde(with = "rust_decimal::serde::str")]
    pub r_distance: Decimal,
    /// Active exchange leverage (Futures only).
    pub leverage: Option<u32>,
    /// Estimated liquidation price (Futures only).
    pub liq_price: Option<Price>,
    /// Current protective stop price.
    pub stop_price: Price,
    /// Primary take-profit target price.
    pub tp1_price: Price,
    /// Secondary take-profit target price, if configured.
    pub tp2_price: Option<Price>,
    /// High-water mark (Long) or low-water mark (Short).
    pub best_price: Price,
    /// Whether TP1 partial close has executed.
    pub tp1_done: bool,
    /// Whether protective stop has moved to breakeven.
    pub breakeven_done: bool,
    /// Cumulative fees paid in quote currency.
    #[serde(with = "rust_decimal::serde::str")]
    pub fees_paid_quote: Decimal,
    /// Realized profit/loss in quote currency.
    #[serde(with = "rust_decimal::serde::str")]
    pub realized_pnl_quote: Decimal,
    /// Current lifecycle state.
    pub state: PositionState,
    /// Position open timestamp (UTC, RFC 3339).
    #[serde(with = "time::serde::rfc3339")]
    pub opened_at: OffsetDateTime,
}
