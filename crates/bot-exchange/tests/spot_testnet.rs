//! Manual Binance Spot Testnet integration test (gated behind `--features testnet-it`).
//!
//! Never runs during default `cargo test --workspace` (hard rule #9).
//!
//! Required environment variables:
//! - `BINANCE_TESTNET_API_KEY`: Spot Testnet Ed25519 API key
//! - `BINANCE_TESTNET_PRIVATE_KEY_PEM_PATH`: path to Ed25519 PKCS#8 PEM private key file
//!
//! Optional:
//! - `BINANCE_TESTNET_SYMBOL` (default: `BTCUSDT`)
//! - `BINANCE_TESTNET_LIMIT_PRICE` (default: `10000.00` — intentionally far below market)
//! - `BINANCE_TESTNET_QTY` (default: `0.001`)

#![cfg(feature = "testnet-it")]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::Path;
use std::str::FromStr;
use std::time::Duration;

use bot_exchange::{
    CancelOrderRequest, Ed25519Credentials, ExchangeGateway, GatewayEvent, PlaceOrderRequest,
    SpotAdapter, SpotAdapterConfig, VenueAdapter,
};
use bot_types::{OrderKind, Price, Qty, Side, Symbol, Venue};
use rust_decimal::Decimal;
use uuid::Uuid;

#[tokio::test]
async fn spot_testnet_logon_place_limit_maker_and_cancel() {
    let api_key = std::env::var("BINANCE_TESTNET_API_KEY")
        .expect("BINANCE_TESTNET_API_KEY must be set for testnet-it");
    let pem_path = std::env::var("BINANCE_TESTNET_PRIVATE_KEY_PEM_PATH")
        .expect("BINANCE_TESTNET_PRIVATE_KEY_PEM_PATH must be set for testnet-it");

    let symbol_str = std::env::var("BINANCE_TESTNET_SYMBOL").unwrap_or_else(|_| "BTCUSDT".into());
    let price_str =
        std::env::var("BINANCE_TESTNET_LIMIT_PRICE").unwrap_or_else(|_| "10000.00".into());
    let qty_str = std::env::var("BINANCE_TESTNET_QTY").unwrap_or_else(|_| "0.001".into());

    let creds = Ed25519Credentials::from_pkcs8_pem_file(api_key, Path::new(&pem_path))
        .expect("load testnet Ed25519 PEM");

    let adapter =
        SpotAdapter::new(SpotAdapterConfig::default(), creds).expect("construct SpotAdapter");

    let report = adapter.activate().await.expect("activate on Spot testnet");
    assert_eq!(report.venue, Venue::Spot);
    assert!(report.symbols_loaded > 0);

    let mut events = adapter.take_event_rx().expect("event receiver");

    let suffix = &Uuid::now_v7().simple().to_string()[..10];
    let cid = format!("b01r0S-{suffix}-TST");
    let symbol = Symbol(symbol_str);

    let ack = adapter
        .place_order(PlaceOrderRequest {
            symbol: symbol.clone(),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: cid.clone(),
            quantity: Some(Qty(Decimal::from_str(&qty_str).expect("qty decimal"))),
            quote_order_qty: None,
            price: Some(Price(Decimal::from_str(&price_str).expect("price decimal"))),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect("place LIMIT_MAKER far below market");
    assert_eq!(ack.client_order_id, cid);

    let cancel = adapter
        .cancel_order(CancelOrderRequest {
            symbol,
            orig_client_order_id: Some(cid.clone()),
            order_id: None,
            new_client_order_id: None,
        })
        .await
        .expect("cancel LIMIT_MAKER");
    assert_eq!(cancel.orig_client_order_id, cid);

    // Confirm at least one executionReport event was received over User Data Stream.
    let mut saw_exec = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_secs(1), events.recv()).await {
            Ok(Some(GatewayEvent::Exec(ev)))
                if ev.client_order_id == cid
                    || ev.orig_client_order_id.as_deref() == Some(cid.as_str()) =>
            {
                saw_exec = true;
                break;
            }
            Ok(Some(_)) => {}
            _ => break,
        }
    }
    assert!(saw_exec, "expected executionReport for {cid}");

    adapter.deactivate().await.expect("clean deactivate");
}
