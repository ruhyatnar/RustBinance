//! Typed error definitions for `bot-marketdata`.

use thiserror::Error;

/// Errors produced by market data streams, order book sync, backfill, or universe filtering.
#[derive(Debug, Error)]
pub enum MarketDataError {
    /// Requested stream subscription count exceeds Binance per-connection ceiling (1024).
    #[error("stream count {count} exceeds per-connection limit {max}")]
    TooManyStreams {
        /// Requested stream count.
        count: usize,
        /// Exchange maximum (1024 for Spot).
        max: usize,
    },

    /// Empty or invalid stream subscription plan.
    #[error("invalid stream specification: {0}")]
    InvalidStream(String),

    /// Malformed wire JSON or unexpected field type.
    #[error("wire format error: {0}")]
    WireFormat(String),

    /// WebSocket transport failure.
    #[error("websocket transport error: {0}")]
    Transport(String),

    /// HTTP REST failure during backfill or snapshot fetch.
    #[error("http error: {0}")]
    Http(String),

    /// Backfill or snapshot resync failed after retries.
    #[error("sync/backfill failed: {0}")]
    SyncFailed(String),

    /// Downstream channel closed.
    #[error("downstream channel closed")]
    ChannelClosed,
}
