#![forbid(unsafe_code)]
//! Venue-agnostic exchange gateway traits and Binance Spot REST + WS API implementation.

pub mod auth;
pub mod dedup;
pub mod error;
pub mod gateway;
pub mod rate_limit;
pub mod spot;

pub use auth::Ed25519Credentials;
pub use dedup::EventDeduplicator;
pub use error::{ExchangeErrorKind, GatewayError, SessionState};
pub use gateway::{
    AccountBalance, AccountEvent, AccountSnapshot, ActivationReport, CancelOrderAck,
    CancelOrderListAck, CancelOrderListRequest, CancelOrderRequest, CancelReplaceAck,
    CancelReplaceMode, CancelReplaceRequest, ExchangeGateway, ExchangeInfoSnapshot, ExecEvent,
    GatewayEvent, ListOrderItem, ListStatusEvent, OcoOrderAck, OrderAck, OrderSnapshot,
    PlaceOcoRequest, PlaceOrderRequest, QueryOrderRequest, TimeInForce, VenueAdapter,
    validate_client_order_id,
};
pub use rate_limit::{
    RateLimitBucketKey, RateLimitBucketState, RateLimitInterval, RateLimitReading, RateLimitRule,
    RateLimitTracker, RateLimitType, ReconnectBackoff,
};
pub use spot::{
    ServerTimeSync, SpotAdapter, SpotAdapterConfig, SpotRestClient, SpotSessionConfig,
    SpotWsSession,
};
