//! Venue-tagged market data domain events and keys (§3.3, §4.3).

use bot_types::{Kline, Price, Qty, Symbol, Venue};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Supported candlestick timeframes (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Timeframe {
    /// 15-minute candles (scanner / S3).
    M15,
    /// 1-hour candles (primary signal TF).
    H1,
    /// 4-hour candles (higher timeframe regime).
    H4,
}

impl Timeframe {
    /// Binance wire interval identifier.
    #[must_use]
    pub const fn as_binance_str(self) -> &'static str {
        match self {
            Self::M15 => "15m",
            Self::H1 => "1h",
            Self::H4 => "4h",
        }
    }

    /// Parse Binance wire interval string.
    #[must_use]
    pub fn from_binance_str(s: &str) -> Option<Self> {
        match s {
            "15m" => Some(Self::M15),
            "1h" => Some(Self::H1),
            "4h" => Some(Self::H4),
            _ => None,
        }
    }

    /// Exact bar duration in milliseconds.
    #[must_use]
    pub const fn interval_ms(self) -> i64 {
        match self {
            Self::M15 => 15 * 60 * 1000,
            Self::H1 => 60 * 60 * 1000,
            Self::H4 => 4 * 60 * 60 * 1000,
        }
    }
}

/// Composite key `(Venue, Symbol, Timeframe)` for kline buffers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct KlineKey {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Candle timeframe.
    pub timeframe: Timeframe,
}

/// Event emitted strictly when a kline closes (`x == true`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClosedKlineEvent {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Timeframe.
    pub timeframe: Timeframe,
    /// Closed OHLCV candle.
    pub kline: Kline,
}

/// Best bid/ask tick from `{symbol}@bookTicker`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BookTickerEvent {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Order book update ID.
    pub update_id: u64,
    /// Best bid price.
    pub bid_price: Price,
    /// Best bid quantity.
    pub bid_qty: Qty,
    /// Best ask price.
    pub ask_price: Price,
    /// Best ask quantity.
    pub ask_qty: Qty,
    /// Local receipt timestamp (UTC ms) for staleness gating (< 500 ms).
    pub received_at_ms: i64,
}

impl BookTickerEvent {
    /// Exact spread in basis points: `(ask - bid) / mid * 10_000`.
    /// Returns `None` if `bid <= 0`, `ask < bid`, or `mid == 0`.
    #[must_use]
    pub fn spread_bps(&self) -> Option<Decimal> {
        let bid = self.bid_price.as_decimal();
        let ask = self.ask_price.as_decimal();
        if bid <= Decimal::ZERO || ask < bid {
            return None;
        }
        let two = Decimal::from(2);
        let mid = bid.checked_add(ask)?.checked_div(two)?;
        if mid <= Decimal::ZERO {
            return None;
        }
        let spread = ask.checked_sub(bid)?;
        spread.checked_div(mid)?.checked_mul(Decimal::from(10_000))
    }
}

/// Single ticker item parsed from `!miniTicker@arr`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MiniTickerItem {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Event timestamp (UTC ms).
    pub event_time_ms: i64,
    /// Last close price.
    pub close: Price,
    /// 24h open price.
    pub open: Price,
    /// 24h high price.
    pub high: Price,
    /// 24h low price.
    pub low: Price,
    /// 24h traded base asset volume.
    pub base_volume: Decimal,
    /// 24h traded quote asset volume.
    pub quote_volume: Decimal,
}

/// Single price/quantity level in an order book snapshot or diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthLevel {
    /// Price level.
    pub price: Price,
    /// Absolute quantity at this price level (`0` removes the level).
    pub qty: Qty,
}

/// Incremental depth diff from `{symbol}@depth@100ms`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthDiffEvent {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Event time (UTC ms).
    pub event_time_ms: i64,
    /// First update ID in this diff (`U`).
    pub first_update_id: u64,
    /// Final update ID in this diff (`u`).
    pub final_update_id: u64,
    /// Previous final update ID (`pu`, reserved for Futures M15 abstraction).
    pub prev_final_update_id: Option<u64>,
    /// Bid level updates (`b`).
    pub bids: Vec<DepthLevel>,
    /// Ask level updates (`a`).
    pub asks: Vec<DepthLevel>,
}

/// Full depth snapshot fetched from REST `GET /api/v3/depth`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DepthSnapshot {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Snapshot `lastUpdateId`.
    pub last_update_id: u64,
    /// Bid levels.
    pub bids: Vec<DepthLevel>,
    /// Ask levels.
    pub asks: Vec<DepthLevel>,
}

/// Local order book synchronization health (§4.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BookStatus {
    /// Buffering diffs while awaiting initial snapshot.
    Syncing,
    /// Synchronized and continuously applying valid diffs.
    Synced,
    /// Sequence gap detected; OBI and spread MUST NOT be computed until resynced.
    Invalid,
}

/// Conflated telemetry snapshot of a local order book (pushed over `watch` channel).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BookTelemetry {
    /// Venue.
    pub venue: Venue,
    /// Symbol.
    pub symbol: Symbol,
    /// Sync health.
    pub status: BookStatus,
    /// Latest applied update ID (`0` if not synced).
    pub last_update_id: u64,
    /// Best bid price (only when `Synced`).
    pub best_bid: Option<Price>,
    /// Best ask price (only when `Synced`).
    pub best_ask: Option<Price>,
    /// Spread in bps (only when `Synced`).
    pub spread_bps: Option<Decimal>,
    /// Unsmoothed top-N order book imbalance in `[-1.0, 1.0]` (only when `Synced`).
    pub obi_raw: Option<f64>,
    /// EMA-smoothed top-N order book imbalance (only when `Synced`).
    pub obi_ema: Option<f64>,
}
