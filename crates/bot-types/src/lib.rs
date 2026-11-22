#![forbid(unsafe_code)]
//! Core venue-agnostic domain types and secret wrappers.

pub mod market;
pub mod order;
pub mod position;
pub mod primitives;
pub mod rules;
pub mod secret;
pub mod venue;

pub use market::Kline;
pub use order::{Order, OrderKind, OrderPurpose, OrderStatus};
pub use position::{Position, PositionState};
pub use primitives::{
    Direction, EngineMode, InstrumentKey, Price, Qty, Side, StrategyId, Symbol, TradingMode, Venue,
};
pub use rules::{
    InstrumentRules, RulesError, ceil_to_tick, floor_to_step, floor_to_tick, round_to_tick,
};
pub use secret::Secret;
pub use venue::{ProtectionScheme, VenueCaps, VenueState};
