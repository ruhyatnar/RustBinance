//! Market data domain structures.

use serde::{Deserialize, Serialize};

/// OHLCV candlestick bar.
///
/// Numeric fields use `f64` exclusively for indicator calculations per §3.4.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kline {
    /// Bar open timestamp in milliseconds since Unix epoch.
    pub open_time_ms: i64,
    /// Bar close timestamp in milliseconds since Unix epoch.
    pub close_time_ms: i64,
    /// Open price (indicator domain only).
    pub open: f64,
    /// High price (indicator domain only).
    pub high: f64,
    /// Low price (indicator domain only).
    pub low: f64,
    /// Close price (indicator domain only).
    pub close: f64,
    /// Base asset volume.
    pub volume: f64,
    /// Quote asset volume.
    pub quote_volume: f64,
    /// Number of trades in the bar.
    pub trades: u64,
    /// Taker buy base asset volume.
    pub taker_buy_base: f64,
    /// Whether the candle has closed.
    pub is_closed: bool,
}
