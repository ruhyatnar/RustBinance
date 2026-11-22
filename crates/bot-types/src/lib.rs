#![forbid(unsafe_code)]
//! Core venue-agnostic domain types and secret wrappers.

pub mod market;
pub mod order;
pub mod position;
pub mod primitives;
pub mod secret;
pub mod venue;

pub use market::Kline;
pub use order::{Order, OrderKind, OrderPurpose, OrderStatus};
pub use position::{Position, PositionState};
pub use primitives::{
    Direction, EngineMode, InstrumentKey, Price, Qty, Side, StrategyId, Symbol, TradingMode, Venue,
};
pub use secret::Secret;
pub use venue::{ProtectionScheme, VenueCaps, VenueState};
