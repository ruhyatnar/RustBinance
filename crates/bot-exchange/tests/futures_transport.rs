#![allow(clippy::expect_used, clippy::indexing_slicing)]
//! Mock WS API (`/ws-fapi/v1`) and REST fallback (`/fapi/v1`) tests for `FuturesAdapter`.

use std::sync::Arc;
use std::time::Duration;

use bot_exchange::auth::Ed25519Credentials;
use bot_exchange::futures::{FuturesAdapter, FuturesAdapterConfig};
use bot_exchange::{
    AlgoOrderType, ExchangeGateway, GatewayError, PlaceAlgoOrderRequest, PlaceOrderRequest,
    VenueAdapter, WorkingType,
};
use bot_types::{OrderKind, Price, Qty, Side, Symbol};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

async fn spawn_mock_rest(dual_side: bool) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind rest");
    let addr = listener.local_addr().expect("rest addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let Ok(n) = sock.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let body = if first_line.contains("/fapi/v1/positionSide/dual") {
                    json!({"dualSidePosition": dual_side}).to_string()
                } else if first_line.contains("/fapi/v1/multiAssetsMargin") {
                    json!({"multiAssetsMargin": false}).to_string()
                } else if first_line.contains("/fapi/v2/positionRisk") {
                    json!([{"positionAmt": "0.0", "marginType": "isolated", "leverage": "2"}])
                        .to_string()
                } else if first_line.contains("/fapi/v1/order") {
                    json!({
                        "symbol": "BTCUSDT",
                        "orderId": 4242,
                        "clientOrderId": "rest-fallback-1",
                        "status": "NEW",
                        "updateTime": 1_700_000_000_000_i64
                    })
                    .to_string()
                } else if first_line.contains("/fapi/v1/algoOrder") {
                    json!({
                        "symbol": "BTCUSDT",
                        "algoId": 9001,
                        "clientAlgoId": "algo-rest-1",
                        "algoStatus": "NEW",
                        "createTime": 1_700_000_000_000_i64
                    })
                    .to_string()
                } else {
                    "{}".to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nX-MBX-USED-WEIGHT-1M: 12\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    format!("http://{addr}")
}

async fn spawn_mock_ws() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let addr = listener.local_addr().expect("ws addr");
    tokio::spawn(async move {
        while let Ok((stream, _)) = listener.accept().await {
            tokio::spawn(async move {
                let Ok(mut ws) = tokio_tungstenite::accept_async(stream).await else {
                    return;
                };
                while let Some(Ok(Message::Text(txt))) = ws.next().await {
                    let req: Value = serde_json::from_str(&txt).expect("valid json");
                    let id = req.get("id").and_then(Value::as_u64).unwrap_or(0);
                    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
                    let params = req.get("params").expect("params");
                    assert!(params.get("apiKey").is_some(), "per-request apiKey");
                    assert!(params.get("signature").is_some(), "per-request signature");
                    assert!(params.get("timestamp").is_some(), "per-request timestamp");

                    let result = if method == "order.place" {
                        json!({
                            "symbol": "BTCUSDT",
                            "orderId": 777,
                            "clientOrderId": "ws-primary-1",
                            "status": "NEW",
                            "updateTime": 1_700_000_000_000_i64
                        })
                    } else {
                        json!({
                            "symbol": "BTCUSDT",
                            "algoId": 888,
                            "clientAlgoId": "algo-ws-1",
                            "algoStatus": "NEW",
                            "createTime": 1_700_000_000_000_i64
                        })
                    };
                    let reply = json!({
                        "id": id,
                        "status": 200,
                        "result": result,
                        "rateLimits": [{"rateLimitType": "REQUEST_WEIGHT", "count": 5}]
                    });
                    let _ = ws.send(Message::Text(reply.to_string().into())).await;
                }
            });
        }
    });
    format!("ws://{addr}")
}

#[tokio::test]
async fn futures_adapter_uses_ws_primary_and_falls_back_to_rest() {
    let rest_url = spawn_mock_rest(false).await;
    let ws_url = spawn_mock_ws().await;
    let creds =
        Arc::new(Ed25519Credentials::from_seed("TEST_KEY".into(), [7_u8; 32]).expect("test creds"));

    let adapter = FuturesAdapter::with_transport(
        FuturesAdapterConfig {
            rest_base_url: rest_url.clone(),
            ws_api_url: ws_url,
            max_leverage_cap: 3,
        },
        Arc::clone(&creds),
        Duration::from_secs(3),
    )
    .expect("construct adapter");

    adapter.activate().await.expect("activate ok");

    // 1. Primary path: WS API
    let ack = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".to_owned()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "ws-primary-1".to_owned(),
            quantity: Some(Qty(Decimal::new(1, 3))),
            quote_order_qty: None,
            price: Some(Price(Decimal::from(50_000_u32))),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect("ws place_order");
    assert_eq!(ack.order_id, 777);

    let algo_ack = adapter
        .place_algo_order(PlaceAlgoOrderRequest {
            symbol: Symbol("BTCUSDT".to_owned()),
            side: Side::Sell,
            order_type: AlgoOrderType::StopMarket,
            quantity: Some(Qty(Decimal::new(1, 3))),
            trigger_price: Some(Price(Decimal::from(49_000_u32))),
            activate_price: None,
            callback_rate: None,
            working_type: WorkingType::MarkPrice,
            close_position: false,
            reduce_only: true,
            price_protect: true,
            client_algo_id: "algo-ws-1".to_owned(),
        })
        .await
        .expect("ws place_algo_order");
    assert_eq!(algo_ack.algo_id, 888);

    // 2. Fallback path: close WS -> REST fallback succeeds seamlessly
    adapter.deactivate().await.expect("deactivate");
    let rest_ack = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".to_owned()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "rest-fallback-1".to_owned(),
            quantity: Some(Qty(Decimal::new(1, 3))),
            quote_order_qty: None,
            price: Some(Price(Decimal::from(50_000_u32))),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect("rest fallback place_order");
    assert_eq!(rest_ack.order_id, 4242);
}

#[tokio::test]
async fn futures_adapter_activate_rejects_hedge_mode_from_live_rest_check() {
    let rest_url = spawn_mock_rest(true).await;
    let creds =
        Arc::new(Ed25519Credentials::from_seed("TEST_KEY".into(), [9_u8; 32]).expect("test creds"));
    let adapter = FuturesAdapter::with_transport(
        FuturesAdapterConfig {
            rest_base_url: rest_url,
            ws_api_url: "ws://127.0.0.1:1".to_owned(),
            max_leverage_cap: 3,
        },
        creds,
        Duration::from_secs(2),
    )
    .expect("construct adapter");

    let err = adapter
        .activate()
        .await
        .expect_err("hedge mode must be rejected");
    assert!(matches!(err, GatewayError::InvalidRequest(_)));
}
