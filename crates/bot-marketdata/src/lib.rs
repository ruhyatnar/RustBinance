#![forbid(unsafe_code)]
//! Market streams, kline buffers with gap backfill, synchronized local order books,
//! OBI/spread telemetry, and universe filtering (§3.3, §4.3, §6.2).

pub mod error;
pub mod futures;
pub mod kline;
pub mod orderbook;
pub mod rest;
pub mod stream;
pub mod types;
pub mod universe;
pub mod wire;

pub use error::MarketDataError;
pub use kline::{KlineBackfillClient, KlineBuffer, KlineIngestResult};
pub use orderbook::{DepthSnapshotClient, DiffApplyResult, LocalOrderBook, SnapshotSyncResult};
pub use rest::{SpotMarketRestClient, parse_rest_depth_snapshot, parse_rest_klines};
pub use stream::{
    MarketStreamChannels, MarketStreamConfig, MarketStreamHandle, spawn_market_stream,
};
pub use types::{
    BookStatus, BookTelemetry, BookTickerEvent, ClosedKlineEvent, DepthDiffEvent, DepthLevel,
    DepthSnapshot, KlineKey, MiniTickerItem, Timeframe,
};
pub use universe::{
    FilteredUniverseProvider, UniverseCandidate, UniverseFilter, UniverseProvider,
    UniverseSnapshot, spawn_universe_refresher,
};
pub use wire::{
    MAX_STREAMS_PER_CONNECTION, MarketStreamMessage, StreamPlan, parse_market_stream_frame,
};
