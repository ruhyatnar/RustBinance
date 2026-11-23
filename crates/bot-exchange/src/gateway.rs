//! Venue-agnostic `ExchangeGateway` and `VenueAdapter` traits, DTOs, and events (§3.5, §4.4).

use async_trait::async_trait;
use bot_types::{
    InstrumentRules, OrderKind, OrderStatus, Price, Qty, Side, Symbol, Venue, VenueCaps,
};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::error::GatewayError;
use crate::rate_limit::RateLimitRule;

/// Validate `clientOrderId` against §5.2 (`^[A-Za-z0-9_-]{1,36}$`).
///
/// # Errors
/// Returns [`GatewayError::InvalidRequest`] if empty, longer than 36 bytes, or containing
/// disallowed characters.
pub fn validate_client_order_id(id: &str) -> Result<(), GatewayError> {
    if id.is_empty() || id.len() > 36 {
        return Err(GatewayError::InvalidRequest(format!(
            "clientOrderId length must be 1..=36, got {}",
            id.len()
        )));
    }
    if !id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err(GatewayError::InvalidRequest(
            "clientOrderId must match ^[A-Za-z0-9_-]{1,36}$".into(),
        ));
    }
    Ok(())
}

/// Time-in-force policy for limit orders.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeInForce {
    /// Good-Til-Canceled.
    Gtc,
    /// Immediate-Or-Cancel.
    Ioc,
    /// Fill-Or-Kill.
    Fok,
}

impl TimeInForce {
    /// Wire representation expected by Binance Spot API.
    #[must_use]
    pub const fn as_binance_str(self) -> &'static str {
        match self {
            Self::Gtc => "GTC",
            Self::Ioc => "IOC",
            Self::Fok => "FOK",
        }
    }
}

/// Request to place a single order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceOrderRequest {
    /// Target symbol.
    pub symbol: Symbol,
    /// Buy or Sell.
    pub side: Side,
    /// Order kind (`LimitMaker`, `Limit`, `Market`, `StopLossLimit`, `TakeProfitLimit`).
    pub kind: OrderKind,
    /// Deterministic client order ID (§5.2).
    pub client_order_id: String,
    /// Base asset quantity (already rounded to `step_size`).
    pub quantity: Option<Qty>,
    /// Quote asset quantity (for Spot `MARKET` orders when preferred over `quantity`).
    pub quote_order_qty: Option<Decimal>,
    /// Limit price (required for `LimitMaker`, `Limit`, `StopLossLimit`, `TakeProfitLimit`).
    pub price: Option<Price>,
    /// Stop trigger price (for `StopLossLimit` / `TakeProfitLimit`).
    pub stop_price: Option<Price>,
    /// Time in force (required for `Limit`, `StopLossLimit`, `TakeProfitLimit`; omitted for `LimitMaker`/`Market`).
    pub time_in_force: Option<TimeInForce>,
}

/// Exchange acknowledgment for a newly placed order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderAck {
    /// Venue where the order was accepted.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Exchange numeric order ID.
    pub order_id: i64,
    /// Client order ID echoed by exchange.
    pub client_order_id: String,
    /// Transaction timestamp (UTC ms).
    pub transact_time_ms: i64,
    /// Order status returned by exchange.
    pub status: OrderStatus,
    /// Executed quantity (may be non-zero for immediate fills).
    pub executed_qty: Qty,
    /// Cumulative quote quantity transacted.
    pub cumulative_quote_qty: Decimal,
}

/// Request to cancel an open order by `origClientOrderId` or `orderId`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrderRequest {
    /// Symbol.
    pub symbol: Symbol,
    /// Original client order ID (preferred per §4.4).
    pub orig_client_order_id: Option<String>,
    /// Exchange numeric order ID.
    pub order_id: Option<i64>,
    /// Optional client ID for the cancel action itself.
    pub new_client_order_id: Option<String>,
}

/// Exchange acknowledgment for a canceled order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CancelOrderAck {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Exchange numeric order ID.
    pub order_id: i64,
    /// Original client order ID.
    pub orig_client_order_id: String,
    /// Resulting status (usually `Canceled`).
    pub status: OrderStatus,
    /// Executed quantity prior to cancel.
    pub executed_qty: Qty,
}

/// Failure behavior for `order.cancelReplace`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelReplaceMode {
    /// Abort placement if cancel fails.
    StopOnFailure,
    /// Attempt placement even if cancel fails.
    AllowFailure,
}

impl CancelReplaceMode {
    /// Wire string for Binance `cancelReplaceMode`.
    #[must_use]
    pub const fn as_binance_str(self) -> &'static str {
        match self {
            Self::StopOnFailure => "STOP_ON_FAILURE",
            Self::AllowFailure => "ALLOW_FAILURE",
        }
    }
}

/// Request to atomically cancel an existing order and place a replacement (`order.cancelReplace`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelReplaceRequest {
    /// Symbol.
    pub symbol: Symbol,
    /// Replacement order side.
    pub side: Side,
    /// Replacement order kind.
    pub kind: OrderKind,
    /// Failure handling mode.
    pub cancel_replace_mode: CancelReplaceMode,
    /// Existing order client ID to cancel.
    pub cancel_orig_client_order_id: Option<String>,
    /// Existing exchange order ID to cancel.
    pub cancel_order_id: Option<i64>,
    /// Deterministic client order ID for the new replacement order.
    pub new_client_order_id: String,
    /// Replacement quantity.
    pub quantity: Qty,
    /// Replacement limit price.
    pub price: Option<Price>,
    /// Replacement time-in-force.
    pub time_in_force: Option<TimeInForce>,
}

/// Response for `order.cancelReplace`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelReplaceAck {
    /// Venue.
    pub venue: Venue,
    /// Cancel leg result (`SUCCESS` / `FAILURE`).
    pub cancel_result: String,
    /// New order leg result (`SUCCESS` / `FAILURE` / `NOT_ATTEMPTED`).
    pub new_order_result: String,
    /// Newly placed order acknowledgment, if `new_order_result == "SUCCESS"`.
    pub new_order: Option<OrderAck>,
}

/// Request to place a Spot OCO protective order list (`orderList.place.oco`, §4.4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceOcoRequest {
    /// Symbol.
    pub symbol: Symbol,
    /// Side (`Sell` for protecting a Long Spot position).
    pub side: Side,
    /// Order list client ID.
    pub list_client_order_id: String,
    /// Quantity shared by both OCO legs.
    pub quantity: Qty,
    /// Above leg order type (typically `OrderKind::LimitMaker` for TP on Long).
    pub above_type: OrderKind,
    /// Above leg client order ID.
    pub above_client_order_id: Option<String>,
    /// Above leg limit price (TP price).
    pub above_price: Option<Price>,
    /// Above leg stop price (if applicable).
    pub above_stop_price: Option<Price>,
    /// Above leg time-in-force.
    pub above_time_in_force: Option<TimeInForce>,
    /// Below leg order type (typically `OrderKind::StopLossLimit` for SL on Long).
    pub below_type: OrderKind,
    /// Below leg client order ID.
    pub below_client_order_id: Option<String>,
    /// Below leg limit price (SL limit price with offset).
    pub below_price: Option<Price>,
    /// Below leg stop trigger price (SL trigger).
    pub below_stop_price: Option<Price>,
    /// Below leg time-in-force (required for `STOP_LOSS_LIMIT`, typically `GTC`).
    pub below_time_in_force: Option<TimeInForce>,
    /// Optional server-side trailing delta in bips for the below leg (`belowTrailingDelta`).
    pub below_trailing_delta: Option<u64>,
}

/// Summary of an individual order inside an OCO order list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListOrderItem {
    /// Symbol.
    pub symbol: Symbol,
    /// Exchange numeric order ID.
    pub order_id: i64,
    /// Client order ID.
    pub client_order_id: String,
}

/// Exchange acknowledgment for `orderList.place.oco`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OcoOrderAck {
    /// Venue.
    pub venue: Venue,
    /// Exchange order list ID.
    pub order_list_id: i64,
    /// List client order ID.
    pub list_client_order_id: String,
    /// List status (`EXECUTING`, `ALL_DONE`, `REJECT`).
    pub list_order_status: String,
    /// Symbol.
    pub symbol: Symbol,
    /// Transaction timestamp (UTC ms).
    pub transaction_time_ms: i64,
    /// Legs belonging to this order list.
    pub orders: Vec<ListOrderItem>,
}

/// Request to cancel an entire OCO order list (`orderList.cancel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrderListRequest {
    /// Symbol.
    pub symbol: Symbol,
    /// Exchange order list ID.
    pub order_list_id: Option<i64>,
    /// List client order ID.
    pub list_client_order_id: Option<String>,
}

/// Exchange acknowledgment for `orderList.cancel`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CancelOrderListAck {
    /// Venue.
    pub venue: Venue,
    /// Exchange order list ID.
    pub order_list_id: i64,
    /// List client order ID.
    pub list_client_order_id: String,
    /// List order status.
    pub list_order_status: String,
    /// Symbol.
    pub symbol: Symbol,
}

/// Request to query an order's authoritative status (`order.status`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QueryOrderRequest {
    /// Symbol.
    pub symbol: Symbol,
    /// Original client order ID (primary key for `UnknownNeedsQuery` reconciliation).
    pub orig_client_order_id: Option<String>,
    /// Exchange numeric order ID.
    pub order_id: Option<i64>,
}

/// Authoritative order snapshot returned by `order.status` or `openOrders.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderSnapshot {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Exchange numeric order ID.
    pub order_id: i64,
    /// Exchange order list ID (`-1` if standalone).
    pub order_list_id: i64,
    /// Client order ID.
    pub client_order_id: String,
    /// Order side.
    pub side: Side,
    /// Order kind.
    pub kind: OrderKind,
    /// Limit price.
    pub price: Price,
    /// Original requested quantity.
    pub orig_qty: Qty,
    /// Cumulative executed quantity.
    pub executed_qty: Qty,
    /// Cumulative quote quantity transacted.
    pub cumulative_quote_qty: Decimal,
    /// Current exchange order status.
    pub status: OrderStatus,
    /// Last update timestamp (UTC ms).
    pub update_time_ms: i64,
}

/// Single asset balance entry in an account snapshot or event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountBalance {
    /// Asset ticker (e.g. `"USDT"`, `"BTC"`).
    pub asset: String,
    /// Available (free) balance.
    pub free: Decimal,
    /// Locked balance in open orders.
    pub locked: Decimal,
}

/// Account state snapshot returned by `account.status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSnapshot {
    /// Venue.
    pub venue: Venue,
    /// Whether trading is enabled on the account.
    pub can_trade: bool,
    /// Last account update time (UTC ms).
    pub update_time_ms: i64,
    /// Non-zero (or all reported) asset balances.
    pub balances: Vec<AccountBalance>,
}

/// Exchange metadata snapshot returned by `exchangeInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExchangeInfoSnapshot {
    /// Venue.
    pub venue: Venue,
    /// Exchange server timestamp (UTC ms).
    pub server_time_ms: i64,
    /// Parsed symbol filter rules.
    pub rules: Vec<InstrumentRules>,
    /// Exchange-published rate limit rules.
    pub rate_limits: Vec<RateLimitRule>,
}

/// Report returned by [`VenueAdapter::activate`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivationReport {
    /// Activated venue.
    pub venue: Venue,
    /// Measured clock offset (`server_time_ms - local_time_ms`).
    pub server_time_offset_ms: i64,
    /// Number of symbol rules loaded from `exchangeInfo`.
    pub symbols_loaded: usize,
    /// Active user data stream subscription ID, if returned by exchange.
    pub subscription_id: Option<i64>,
}

/// Venue-tagged execution report event parsed from User Data Stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecEvent {
    /// Venue.
    pub venue: Venue,
    /// Event timestamp (UTC ms).
    pub event_time_ms: i64,
    /// Transaction timestamp (UTC ms).
    pub transact_time_ms: i64,
    /// Symbol.
    pub symbol: Symbol,
    /// Client order ID.
    pub client_order_id: String,
    /// Original client order ID (present on cancel/replace events).
    pub orig_client_order_id: Option<String>,
    /// Exchange numeric order ID.
    pub order_id: i64,
    /// Exchange order list ID (`-1` if not part of a list).
    pub order_list_id: i64,
    /// Trade ID (`-1` if not a fill).
    pub trade_id: i64,
    /// Order side.
    pub side: Side,
    /// Order kind.
    pub kind: OrderKind,
    /// Execution type (`NEW`, `TRADE`, `CANCELED`, `REPLACED`, `REJECTED`, `EXPIRED`).
    pub exec_type: String,
    /// Current order status.
    pub status: OrderStatus,
    /// Reject reason (`NONE` if not rejected).
    pub reject_reason: String,
    /// Order limit price.
    pub order_price: Price,
    /// Order quantity.
    pub order_qty: Qty,
    /// Last executed fill quantity.
    pub last_filled_qty: Qty,
    /// Cumulative filled quantity.
    pub cumulative_filled_qty: Qty,
    /// Last executed fill price.
    pub last_filled_price: Price,
    /// Cumulative quote quantity transacted.
    pub cumulative_quote_qty: Decimal,
    /// Commission paid on this fill.
    pub commission: Decimal,
    /// Asset in which commission was charged, if any.
    pub commission_asset: Option<String>,
    /// Whether this fill was maker side.
    pub is_maker: bool,
}

/// Venue-tagged account balance update event parsed from User Data Stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountEvent {
    /// `outboundAccountPosition`: full changed-asset snapshot.
    PositionUpdate {
        /// Venue.
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Last account update time (UTC ms).
        last_update_ms: i64,
        /// Updated asset balances.
        balances: Vec<AccountBalance>,
    },
    /// `balanceUpdate`: deposit/withdrawal/transfer delta.
    BalanceDelta {
        /// Venue.
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Asset ticker.
        asset: String,
        /// Balance delta.
        delta: Decimal,
        /// Clear time (UTC ms).
        clear_time_ms: i64,
    },
}

/// Venue-tagged OCO/order-list status event (`listStatus`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListStatusEvent {
    /// Venue.
    pub venue: Venue,
    /// Event time (UTC ms).
    pub event_time_ms: i64,
    /// Symbol.
    pub symbol: Symbol,
    /// Exchange order list ID.
    pub order_list_id: i64,
    /// List client order ID.
    pub list_client_order_id: String,
    /// List status type (`RESPONSE`, `EXEC_STARTED`, `ALL_DONE`).
    pub list_status_type: String,
    /// List order status (`EXECUTING`, `ALL_DONE`, `REJECT`).
    pub list_order_status: String,
    /// Reject reason (`NONE` if accepted).
    pub reject_reason: String,
    /// Legs in this order list.
    pub orders: Vec<ListOrderItem>,
}

/// Unified venue-tagged stream of events emitted by an exchange gateway.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GatewayEvent {
    /// Order lifecycle / fill event (`executionReport`).
    Exec(ExecEvent),
    /// Account balance event (`outboundAccountPosition` / `balanceUpdate`).
    Account(AccountEvent),
    /// Order list status event (`listStatus`).
    ListStatus(ListStatusEvent),
    /// User data stream terminated (`eventStreamTerminated`).
    StreamTerminated {
        /// Venue.
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
    },
    /// Server shutdown notice (`serverShutdown`); session initiates make-before-break rotation.
    ServerShutdown {
        /// Venue.
        venue: Venue,
        /// Event time (UTC ms).
        event_time_ms: i64,
        /// Planned shutdown timestamp (UTC ms), if supplied by server.
        shutdown_time_ms: Option<i64>,
    },
    /// Session reconnected and re-subscribed; engine **must** reconcile before new entries (§4.2).
    Reconnected {
        /// Venue.
        venue: Venue,
        /// New subscription ID, if provided.
        subscription_id: Option<i64>,
    },
}

/// Venue-agnostic exchange execution and query interface (§3.5).
#[async_trait]
pub trait ExchangeGateway: Send + Sync {
    /// Place a single order (`order.place`).
    async fn place_order(&self, req: PlaceOrderRequest) -> Result<OrderAck, GatewayError>;

    /// Cancel an existing order (`order.cancel`).
    async fn cancel_order(&self, req: CancelOrderRequest) -> Result<CancelOrderAck, GatewayError>;

    /// Atomically cancel and replace an order (`order.cancelReplace`).
    async fn cancel_replace(
        &self,
        req: CancelReplaceRequest,
    ) -> Result<CancelReplaceAck, GatewayError>;

    /// Place a Spot OCO order list (`orderList.place.oco`).
    async fn place_oco(&self, req: PlaceOcoRequest) -> Result<OcoOrderAck, GatewayError>;

    /// Cancel an OCO order list (`orderList.cancel`).
    async fn cancel_list(
        &self,
        req: CancelOrderListRequest,
    ) -> Result<CancelOrderListAck, GatewayError>;

    /// Query a single order's status (`order.status`) — used to resolve `UnknownNeedsQuery`.
    async fn query_order(&self, req: QueryOrderRequest) -> Result<OrderSnapshot, GatewayError>;

    /// Query all open orders, optionally filtered by symbol (`openOrders.status`).
    async fn open_orders(
        &self,
        symbol: Option<&Symbol>,
    ) -> Result<Vec<OrderSnapshot>, GatewayError>;

    /// Fetch current account snapshot (`account.status`).
    async fn account_snapshot(&self) -> Result<AccountSnapshot, GatewayError>;

    /// Fetch exchange info and symbol filter rules (`exchangeInfo`).
    async fn exchange_info(&self) -> Result<ExchangeInfoSnapshot, GatewayError>;
}

/// Venue lifecycle adapter extending [`ExchangeGateway`] (§3.5).
#[async_trait]
pub trait VenueAdapter: ExchangeGateway + Send + Sync {
    /// Static venue identifier (`Venue::Spot` or `Venue::UsdsFutures`).
    fn venue(&self) -> Venue;

    /// Static venue capabilities (`VenueCaps`).
    fn caps(&self) -> VenueCaps;

    /// Connect, authenticate, subscribe to user data, sync clock, and load exchange info.
    async fn activate(&self) -> Result<ActivationReport, GatewayError>;

    /// Cleanly close the session without canceling any protective orders (hard rule #7).
    async fn deactivate(&self) -> Result<(), GatewayError>;
}

#[cfg(test)]
mod tests {
    use super::validate_client_order_id;

    #[test]
    fn validates_client_order_id_format() {
        assert!(validate_client_order_id("b01r7S-000123-ENT").is_ok());
        assert!(validate_client_order_id("b01r7F_000124-SL").is_ok());
        assert!(validate_client_order_id("").is_err());
        assert!(validate_client_order_id(&"a".repeat(37)).is_err());
        assert!(validate_client_order_id("bad id!").is_err());
    }
}
