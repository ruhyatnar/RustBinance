//! Optional Binance Spot Testnet market stream contract test (`--features testnet-it`).
//!
//! Never runs during default `cargo test --workspace` (hard rule #9).

#![cfg(feature = "testnet-it")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use bot_marketdata::{
    BookStatus, MarketStreamConfig, SpotMarketRestClient, StreamPlan, Timeframe,
    spawn_market_stream,
};
use bot_types::Symbol;

#[tokio::test]
async fn spot_testnet_market_stream_receives_book_ticker_and_syncs_depth() {
    let rest = Arc::new(
        SpotMarketRestClient::new(SpotMarketRestClient::TESTNET_URL, Duration::from_secs(10))
            .expect("rest client"),
    );

    let sym = Symbol("BTCUSDT".into());
    let mut plan = StreamPlan::new();
    plan.add_kline(&sym, Timeframe::M15)
        .add_book_ticker(&sym)
        .add_depth_100ms(&sym);

    let cfg = MarketStreamConfig {
        stale_timeout: Duration::from_secs(10),
        ..MarketStreamConfig::default()
    };

    let (handle, mut channels) = spawn_market_stream(cfg, &plan, Some(rest.clone()), Some(rest))
        .expect("spawn testnet stream");

    let mut synced = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while tokio::time::Instant::now() < deadline {
        if let Some(tel) = channels.book_telemetry_rx.borrow().get(&sym)
            && tel.status == BookStatus::Synced
            && tel.spread_bps.is_some()
        {
            synced = true;
            break;
        }
        let _ = tokio::time::timeout(Duration::from_secs(1), channels.book_telemetry_rx.changed())
            .await;
    }

    assert!(
        synced,
        "expected BTCUSDT local order book to sync on testnet"
    );
    handle.shutdown().await;
}
