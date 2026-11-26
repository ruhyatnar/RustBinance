#![allow(clippy::expect_used, clippy::indexing_slicing, clippy::too_many_lines)]
//! Mock WS API (`/ws-fapi/v1`) and REST fallback (`/fapi/v1`) tests for `FuturesAdapter`.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bot_exchange::auth::Ed25519Credentials;
use bot_exchange::futures::{
    FuturesAccountSetup, FuturesAdapter, FuturesAdapterConfig, FuturesMarginType,
};
use bot_exchange::{
    AccountEvent, AlgoOrderType, BalanceSource, ExchangeErrorKind, ExchangeGateway, GatewayError,
    GatewayEvent, PlaceAlgoOrderRequest, PlaceOrderRequest, RateLimitInterval, RateLimitRule,
    RateLimitType, VenueAdapter, WorkingType,
};
use bot_types::{OrderKind, OrderStatus, Price, Qty, Side, Symbol, Venue};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

fn local_now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

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
                let body = if first_line.contains("/fapi/v1/time") {
                    json!({"serverTime": local_now_ms()}).to_string()
                } else if first_line.contains("/fapi/v1/positionSide/dual") {
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
                    } else if method == "v2/account.status" {
                        json!({
                            "canTrade": true,
                            "updateTime": local_now_ms(),
                            "assets": [{
                                "asset": "USDT",
                                "availableBalance": "1000.0",
                                "initialMargin": "0.0"
                            }]
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

#[tokio::test]
async fn futures_user_data_stream_delivers_events_updates_balance_cache_and_deactivates_cleanly() {
    let listen_key_closed = Arc::new(AtomicUsize::new(0));
    let algo_cancel_calls = Arc::new(AtomicUsize::new(0));
    let closed_clone = Arc::clone(&listen_key_closed);
    let cancel_clone = Arc::clone(&algo_cancel_calls);

    // Mock REST server supporting time, setup, account, and listenKey lifecycle.
    let rest_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind rest");
    let rest_addr = rest_listener.local_addr().expect("rest addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = rest_listener.accept().await else {
                break;
            };
            let closed_c = Arc::clone(&closed_clone);
            let cancel_c = Arc::clone(&cancel_clone);
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let Ok(n) = sock.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let body = if first_line.contains("/fapi/v1/time") {
                    json!({"serverTime": local_now_ms()}).to_string()
                } else if first_line.contains("/fapi/v1/positionSide/dual") {
                    json!({"dualSidePosition": false}).to_string()
                } else if first_line.contains("/fapi/v1/multiAssetsMargin") {
                    json!({"multiAssetsMargin": false}).to_string()
                } else if first_line.contains("/fapi/v2/positionRisk") {
                    json!([{"positionAmt": "0.0", "marginType": "isolated", "leverage": "2"}])
                        .to_string()
                } else if first_line.starts_with("POST /fapi/v1/listenKey") {
                    json!({"listenKey": "test-listen-key-123"}).to_string()
                } else if first_line.starts_with("DELETE /fapi/v1/listenKey") {
                    closed_c.fetch_add(1, Ordering::SeqCst);
                    "{}".to_owned()
                } else if first_line.contains("/fapi/v2/account") {
                    json!({
                        "canTrade": true,
                        "updateTime": local_now_ms(),
                        "assets": [{
                            "asset": "USDT",
                            "availableBalance": "1000.0",
                            "initialMargin": "0.0"
                        }]
                    })
                    .to_string()
                } else if first_line.starts_with("DELETE /fapi/v1/algoOrder") {
                    cancel_c.fetch_add(1, Ordering::SeqCst);
                    "{}".to_owned()
                } else {
                    "{}".to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    // Mock Futures User Data Stream WS server (/ws/test-listen-key-123).
    let (send_events_tx, send_events_rx) = tokio::sync::oneshot::channel::<()>();
    let stream_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind stream");
    let stream_addr = stream_listener.local_addr().expect("stream addr");
    tokio::spawn(async move {
        if let Ok((stream, _)) = stream_listener.accept().await {
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept user stream");

            // Wait until the test has seeded BalanceCache via account_snapshot().
            let _ = send_events_rx.await;

            let order_ev = json!({
                "e": "ORDER_TRADE_UPDATE",
                "E": local_now_ms(),
                "o": {
                    "s": "BTCUSDT",
                    "c": "b01r1F-000001-ENT",
                    "S": "BUY",
                    "o": "LIMIT",
                    "x": "TRADE",
                    "X": "FILLED",
                    "i": 42,
                    "t": 7,
                    "p": "50000.0",
                    "q": "0.01",
                    "l": "0.01",
                    "z": "0.01",
                    "L": "50000.0",
                    "rp": "0"
                }
            });
            let acct_ev = json!({
                "e": "ACCOUNT_UPDATE",
                "E": local_now_ms(),
                "T": local_now_ms(),
                "a": {
                    "m": "ORDER",
                    "B": [{
                        "a": "USDT",
                        "wb": "950.0",
                        "cw": "900.0",
                        "bc": "-50.0"
                    }]
                }
            });
            let algo_ev = json!({
                "e": "ALGO_UPDATE",
                "E": local_now_ms(),
                "o": {
                    "s": "BTCUSDT",
                    "S": "SELL",
                    "o": "STOP_MARKET",
                    "aid": 99,
                    "caid": "b01r1F-000002-SL",
                    "X": "NEW",
                    "q": "0.01",
                    "tp": "49000.0"
                }
            });

            let _ = ws.send(Message::Text(order_ev.to_string().into())).await;
            let _ = ws.send(Message::Text(acct_ev.to_string().into())).await;
            let _ = ws.send(Message::Text(algo_ev.to_string().into())).await;

            // Keep socket alive until client closes on deactivate().
            while let Some(Ok(msg)) = ws.next().await {
                if matches!(msg, Message::Close(_)) {
                    break;
                }
            }
        }
    });

    let ws_api_url = spawn_mock_ws().await;
    let creds =
        Arc::new(Ed25519Credentials::from_seed("TEST_KEY".into(), [11_u8; 32]).expect("creds"));

    let adapter = FuturesAdapter::with_transport(
        FuturesAdapterConfig {
            rest_base_url: format!("http://{rest_addr}"),
            ws_api_url,
            max_leverage_cap: 3,
        },
        creds,
        Duration::from_secs(3),
    )
    .expect("construct adapter")
    .with_user_stream_base_url(format!("ws://{stream_addr}"));

    adapter.activate().await.expect("activate");
    let mut rx = adapter.take_event_rx().expect("event rx present");
    assert!(
        adapter.take_event_rx().is_none(),
        "take_event_rx is single-consumer"
    );

    // Seed balance cache via account_snapshot() (WS v2/account.status) before streaming WS deltas.
    let initial = adapter.account_snapshot().await.expect("account_snapshot");
    assert_eq!(initial.venue, Venue::UsdsFutures);
    let _ = send_events_tx.send(());

    // Receive all 3 user-data events.
    let ev1 = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("ev1 timeout")
        .expect("ev1");
    assert!(matches!(
        ev1,
        GatewayEvent::Exec(ref e)
            if e.venue == Venue::UsdsFutures
                && e.order_id == 42
                && e.status == OrderStatus::Filled
    ));

    let ev2 = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("ev2 timeout")
        .expect("ev2");
    assert!(matches!(
        ev2,
        GatewayEvent::Account(AccountEvent::PositionUpdate {
            venue: Venue::UsdsFutures,
            ..
        })
    ));

    let ev3 = tokio::time::timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("ev3 timeout")
        .expect("ev3");
    assert!(matches!(
        ev3,
        GatewayEvent::Exec(ref e)
            if e.venue == Venue::UsdsFutures
                && e.order_id == 99
                && e.exec_type == "ALGO_UPDATE"
                && e.client_order_id == "b01r1F-000002-SL"
    ));

    // BalanceCache should now serve fresh WS-updated USDT balance (free=900, locked=50).
    let cached = adapter
        .cached_account_snapshot()
        .await
        .expect("cached snapshot");
    assert_eq!(cached.provenance.source, BalanceSource::Ws);
    let usdt = cached
        .snapshot
        .balances
        .iter()
        .find(|b| b.asset == "USDT")
        .expect("USDT balance");
    assert_eq!(usdt.free, Decimal::from(900));
    assert_eq!(usdt.locked, Decimal::from(50));
    assert_eq!(
        adapter.balance_provenance().map(|p| p.source),
        Some(BalanceSource::Ws)
    );

    // Deactivate must close listenKey, invalidate BalanceCache, and NEVER cancel Algo orders.
    adapter.deactivate().await.expect("deactivate");
    assert_eq!(listen_key_closed.load(Ordering::SeqCst), 1);
    assert_eq!(algo_cancel_calls.load(Ordering::SeqCst), 0);
    assert!(adapter.balance_provenance().is_none());
}

#[tokio::test]
async fn futures_offline_inject_event_channel_and_rate_budget_enforcement() {
    let adapter = FuturesAdapter::new(
        FuturesAdapterConfig {
            rest_base_url: "https://testnet.binancefuture.com".into(),
            ws_api_url: "wss://testnet.binancefuture.com/ws-fapi/v1".into(),
            max_leverage_cap: 5,
        },
        FuturesAccountSetup {
            dual_side_position: false,
            multi_assets_margin: false,
            margin_type: FuturesMarginType::Isolated,
            leverage: 3,
            has_open_positions: false,
        },
    );

    let _tx = adapter.inject_event_channel();
    let mut rx = adapter.take_event_rx().expect("injected rx");
    let frame = json!({
        "e": "ORDER_TRADE_UPDATE",
        "E": 1_700_000_000_001_i64,
        "o": {
            "s": "ETHUSDT",
            "c": "b01r1F-000010-ENT",
            "S": "SELL",
            "o": "LIMIT",
            "x": "NEW",
            "X": "NEW",
            "i": 555,
            "l": "0",
            "z": "0",
            "L": "0"
        }
    })
    .to_string();
    adapter
        .ingest_user_data_frame(&frame)
        .expect("ingest frame");
    let ev = rx.try_recv().expect("received injected event");
    assert!(matches!(ev, GatewayEvent::Exec(e) if e.order_id == 555));

    // Seed ORDERS rate-limit rule at 90% usage -> place_order & place_algo_order must be blocked.
    adapter.rate_limits().apply_rules(&[RateLimitRule {
        rate_limit_type: RateLimitType::Orders,
        interval: RateLimitInterval::Second,
        interval_num: 10,
        limit: 10,
    }]);
    adapter
        .rate_limits()
        .record_rest_header("X-MBX-ORDER-COUNT-10S", "9");

    let order_err = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "b01r1F-000011-ENT".into(),
            quantity: Some(Qty(Decimal::new(1, 3))),
            quote_order_qty: None,
            price: Some(Price(Decimal::from(50_000))),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect_err("90% budget must block place_order");
    assert!(matches!(
        order_err,
        GatewayError::RateLimited { status: 429, .. }
    ));

    let algo_err = adapter
        .place_algo_order(PlaceAlgoOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Sell,
            order_type: AlgoOrderType::StopMarket,
            client_algo_id: "b01r1F-000012-SL".into(),
            quantity: None,
            trigger_price: Some(Price(Decimal::from(49_000))),
            activate_price: None,
            callback_rate: None,
            working_type: WorkingType::MarkPrice,
            close_position: true,
            reduce_only: false,
            price_protect: true,
        })
        .await
        .expect_err("90% budget must block place_algo_order");
    assert!(matches!(
        algo_err,
        GatewayError::RateLimited { status: 429, .. }
    ));
}

#[tokio::test]
async fn futures_adapter_never_falls_back_to_rest_on_permanent_ws_error() {
    let rest_order_hits = Arc::new(AtomicUsize::new(0));
    let rest_hits_clone = Arc::clone(&rest_order_hits);

    let rest_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind rest");
    let rest_addr = rest_listener.local_addr().expect("rest addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = rest_listener.accept().await else {
                break;
            };
            let hits = Arc::clone(&rest_hits_clone);
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let Ok(n) = sock.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let body = if first_line.contains("/fapi/v1/time") {
                    json!({"serverTime": local_now_ms()}).to_string()
                } else if first_line.contains("/fapi/v1/positionSide/dual") {
                    json!({"dualSidePosition": false}).to_string()
                } else if first_line.contains("/fapi/v1/multiAssetsMargin") {
                    json!({"multiAssetsMargin": false}).to_string()
                } else if first_line.contains("/fapi/v2/positionRisk") {
                    json!([{"positionAmt": "0.0", "marginType": "isolated", "leverage": "2"}])
                        .to_string()
                } else if first_line.contains("/fapi/v1/order") {
                    hits.fetch_add(1, Ordering::SeqCst);
                    "{}".to_owned()
                } else {
                    "{}".to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    // Mock WS API returning permanent -1022 InvalidSignature.
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let ws_addr = ws_listener.local_addr().expect("ws addr");
    tokio::spawn(async move {
        if let Ok((stream, _)) = ws_listener.accept().await {
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept ws");
            while let Some(Ok(Message::Text(txt))) = ws.next().await {
                let req: Value = serde_json::from_str(&txt).expect("valid json");
                let id = req.get("id").and_then(Value::as_u64).unwrap_or(0);
                let reply = json!({
                    "id": id,
                    "status": 400,
                    "error": {
                        "code": -1022,
                        "msg": "Signature for this request is not valid."
                    }
                });
                let _ = ws.send(Message::Text(reply.to_string().into())).await;
            }
        }
    });

    let creds =
        Arc::new(Ed25519Credentials::from_seed("TEST_KEY".into(), [12_u8; 32]).expect("creds"));
    let adapter = FuturesAdapter::with_transport(
        FuturesAdapterConfig {
            rest_base_url: format!("http://{rest_addr}"),
            ws_api_url: format!("ws://{ws_addr}"),
            max_leverage_cap: 3,
        },
        creds,
        Duration::from_secs(3),
    )
    .expect("adapter");

    adapter.activate().await.expect("activate");

    let err = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "perm-err-1".into(),
            quantity: Some(Qty(Decimal::new(1, 3))),
            quote_order_qty: None,
            price: Some(Price(Decimal::from(50_000))),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect_err("permanent -1022 must fail without REST fallback");

    assert!(!err.is_retryable());
    assert!(matches!(
        err,
        GatewayError::ExchangeRejected {
            code: -1022,
            kind: ExchangeErrorKind::InvalidSignature,
            ..
        }
    ));
    assert_eq!(
        rest_order_hits.load(Ordering::SeqCst),
        0,
        "permanent WS error must never trigger REST fallback"
    );
    adapter.deactivate().await.expect("deactivate");
}

#[tokio::test]
async fn futures_ws_session_uses_shared_clock_sync_offset_from_activate() {
    let simulated_offset_ms: i64 = 60_000;

    let rest_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind rest");
    let rest_addr = rest_listener.local_addr().expect("rest addr");
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = rest_listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0_u8; 4096];
                let Ok(n) = sock.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let first_line = req.lines().next().unwrap_or("");
                let body = if first_line.contains("/fapi/v1/time") {
                    json!({"serverTime": local_now_ms() + simulated_offset_ms}).to_string()
                } else if first_line.contains("/fapi/v1/positionSide/dual") {
                    json!({"dualSidePosition": false}).to_string()
                } else if first_line.contains("/fapi/v1/multiAssetsMargin") {
                    json!({"multiAssetsMargin": false}).to_string()
                } else if first_line.contains("/fapi/v2/positionRisk") {
                    json!([{"positionAmt": "0.0", "marginType": "isolated", "leverage": "2"}])
                        .to_string()
                } else {
                    "{}".to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });

    let (ts_tx, ts_rx) = tokio::sync::oneshot::channel::<i64>();
    let ws_listener = TcpListener::bind("127.0.0.1:0").await.expect("bind ws");
    let ws_addr = ws_listener.local_addr().expect("ws addr");
    tokio::spawn(async move {
        let mut ts_tx = Some(ts_tx);
        if let Ok((stream, _)) = ws_listener.accept().await {
            let mut ws = tokio_tungstenite::accept_async(stream)
                .await
                .expect("accept ws");
            while let Some(Ok(Message::Text(txt))) = ws.next().await {
                let req: Value = serde_json::from_str(&txt).expect("valid json");
                let id = req.get("id").and_then(Value::as_u64).unwrap_or(0);
                let ts = req
                    .pointer("/params/timestamp")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse::<i64>().ok())
                    .expect("timestamp param");
                if let Some(tx) = ts_tx.take() {
                    let _ = tx.send(ts);
                }
                let reply = json!({
                    "id": id,
                    "status": 200,
                    "result": {
                        "symbol": "BTCUSDT",
                        "orderId": 1001,
                        "clientOrderId": "clock-sync-1",
                        "status": "NEW",
                        "updateTime": ts
                    }
                });
                let _ = ws.send(Message::Text(reply.to_string().into())).await;
            }
        }
    });

    let creds =
        Arc::new(Ed25519Credentials::from_seed("TEST_KEY".into(), [13_u8; 32]).expect("creds"));
    let adapter = FuturesAdapter::with_transport(
        FuturesAdapterConfig {
            rest_base_url: format!("http://{rest_addr}"),
            ws_api_url: format!("ws://{ws_addr}"),
            max_leverage_cap: 3,
        },
        creds,
        Duration::from_secs(3),
    )
    .expect("adapter");

    let report = adapter.activate().await.expect("activate");
    assert!(
        (report.server_time_offset_ms - simulated_offset_ms).abs() < 2_000,
        "expected ~+60s offset, got {}",
        report.server_time_offset_ms
    );
    assert_eq!(
        adapter.clock_state().offset_ms(),
        Some(report.server_time_offset_ms)
    );

    let before_local = local_now_ms();
    let ack = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "clock-sync-1".into(),
            quantity: Some(Qty(Decimal::new(1, 3))),
            quote_order_qty: None,
            price: Some(Price(Decimal::from(50_000))),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect("place_order");
    assert_eq!(ack.order_id, 1001);

    let sent_ts = ts_rx.await.expect("captured ws timestamp");
    let delta_from_local = sent_ts - before_local;
    assert!(
        (delta_from_local - simulated_offset_ms).abs() < 5_000,
        "FuturesWsSession must use clock-adjusted timestamp (delta={delta_from_local}ms, expected ~{simulated_offset_ms}ms)"
    );
    adapter.deactivate().await.expect("deactivate");
}
