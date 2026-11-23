//! Binance Spot REST and WebSocket API implementation.

pub mod adapter;
pub mod rest;
pub mod session;
pub mod wire;

pub use adapter::{SpotAdapter, SpotAdapterConfig};
pub use rest::{ServerTimeSync, SpotRestClient, parse_spot_exchange_info};
pub use session::{SpotSessionConfig, SpotWsSession};
