//! Deterministic mock Spot REST + WS API integration tests covering all M02 acceptance criteria.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::too_many_lines
)]

use std::collections::HashMap;
use std::io;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use bot_exchange::{
    CancelOrderListRequest, CancelOrderRequest, CancelReplaceMode, CancelReplaceRequest,
    Ed25519Credentials, ExchangeGateway, GatewayError, GatewayEvent, PlaceOcoRequest,
    PlaceOrderRequest, QueryOrderRequest, RateLimitBucketKey, RateLimitInterval, RateLimitType,
    SessionState, SpotAdapter, SpotAdapterConfig, SpotSessionConfig, TimeInForce, VenueAdapter,
};
use bot_types::{OrderKind, OrderStatus, Price, Qty, Side, Symbol, Venue};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use futures_util::{SinkExt, StreamExt};
use rust_decimal::Decimal;
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Notify;
use tokio_tungstenite::accept_async;
use tokio_tungstenite::tungstenite::Message;

/// Minimal local HTTP/1.1 mock server for `GET /api/v3/time` and `GET /api/v3/exchangeInfo`.
async fn spawn_mock_rest_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock REST");
    let addr = listener.local_addr().expect("rest local_addr");

    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let body = if req.contains("GET /api/v3/time") {
                    let now_ms = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("unix epoch")
                        .as_millis();
                    format!(r#"{{"serverTime":{now_ms}}}"#)
                } else {
                    r#"{
                        "timezone": "UTC",
                        "serverTime": 1770736694138,
                        "rateLimits": [
                            {"rateLimitType":"REQUEST_WEIGHT","interval":"MINUTE","intervalNum":1,"limit":6000},
                            {"rateLimitType":"ORDERS","interval":"SECOND","intervalNum":10,"limit":100}
                        ],
                        "symbols": [
                            {
                                "symbol": "BTCUSDT",
                                "status": "TRADING",
                                "baseAssetPrecision": 8,
                                "quoteAssetPrecision": 8,
                                "filters": [
                                    {"filterType":"PRICE_FILTER","tickSize":"0.01"},
                                    {"filterType":"LOT_SIZE","minQty":"0.00001","maxQty":"9000.0","stepSize":"0.00001"},
                                    {"filterType":"NOTIONAL","minNotional":"5.0"}
                                ]
                            }
                        ]
                    }"#
                    .to_owned()
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });

    format!("http://{addr}")
}

#[derive(Default)]
struct MockWsState {
    conn_count: AtomicUsize,
    logon_count: AtomicUsize,
    subscribe_count: AtomicUsize,
    order_place_count: AtomicUsize,
    cancel_count: AtomicUsize,
    pong_received: Mutex<Vec<Vec<u8>>>,
    /// Orders stored by clientOrderId so `order.status` can resolve `UnknownNeedsQuery`.
    orders: Mutex<HashMap<String, Value>>,
    /// Behavior toggles for specific test scenarios:
    /// When set, `order.place` for matching cid sleeps (never replies) to trigger timeout.
    drop_response_for_cid: Mutex<Option<String>>,
    /// When set, `order.place` for matching cid returns 503.
    return_503_for_cid: Mutex<Option<String>>,
    /// When set, next `account.status` returns `(status, retry_after_offset_ms)`.
    next_rate_limit_error: Mutex<Option<(u16, i64)>>,
    /// When true, connection 1 sends a `Ping` and verifies `Pong`.
    send_ping_on_first_conn: AtomicBool,
    /// When true, connection 1 drops abruptly after `drop_first_conn_notify`.
    drop_first_conn_notify: Notify,
    /// Trigger pushing a `serverShutdown` + duplicate/new fills across conn 1 & conn 2.
    send_shutdown_notify: Notify,
    /// Delay specific `order.place` cid response by N ms (tests in-flight drain during rotation).
    delay_order_place_ms: Mutex<HashMap<String, u64>>,
    /// Expected Ed25519 verifying key to validate `session.logon` signatures.
    verifying_key: Mutex<Option<VerifyingKey>>,
}

async fn spawn_mock_ws_server(state: Arc<MockWsState>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock WS");
    let addr = listener.local_addr().expect("ws local_addr");

    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let st = Arc::clone(&state);
            let conn_num = st.conn_count.fetch_add(1, Ordering::SeqCst) + 1;

            tokio::spawn(async move {
                let Ok(ws) = accept_async(stream).await else {
                    return;
                };
                let (mut ws_tx, mut ws_rx) = ws.split();
                let (out_tx, mut out_rx) = tokio::sync::mpsc::channel::<Message>(128);

                let writer = tokio::spawn(async move {
                    while let Some(m) = out_rx.recv().await {
                        if ws_tx.send(m).await.is_err() {
                            break;
                        }
                    }
                });

                if conn_num == 1 && st.send_ping_on_first_conn.load(Ordering::SeqCst) {
                    let _ = out_tx
                        .send(Message::Ping(b"binance-ping-42".to_vec().into()))
                        .await;
                }

                // If conn 1 receives shutdown trigger, push fill (777, 501) + serverShutdown.
                if conn_num == 1 {
                    let st_s = Arc::clone(&st);
                    let tx_s = out_tx.clone();
                    tokio::spawn(async move {
                        st_s.send_shutdown_notify.notified().await;
                        let fill1 = make_exec_fill_event(777, 501, "b01r1S-000777-ENT");
                        let _ = tx_s.send(Message::Text(fill1.to_string().into())).await;
                        let shutdown = json!({
                            "event": {
                                "e": "serverShutdown",
                                "E": 1_770_736_700_000_i64,
                                "T": 1_770_737_300_000_i64
                            }
                        });
                        let _ = tx_s.send(Message::Text(shutdown.to_string().into())).await;
                    });
                }

                let drop_fut = async {
                    if conn_num == 1 {
                        st.drop_first_conn_notify.notified().await;
                    } else {
                        std::future::pending::<()>().await;
                    }
                };
                tokio::pin!(drop_fut);

                loop {
                    tokio::select! {
                        () = &mut drop_fut => {
                            // Abruptly close connection 1 without Close handshake.
                            break;
                        }
                        msg = ws_rx.next() => {
                            let Some(Ok(msg)) = msg else { break; };
                            match msg {
                                Message::Pong(payload) => {
                                    st.pong_received.lock().unwrap().push(payload.to_vec());
                                }
                                Message::Close(_) => break,
                                Message::Text(text) => {
                                    let req: Value = serde_json::from_str(text.as_str()).expect("valid json req");
                                    let id = req["id"].as_str().expect("req id").to_owned();
                                    let method = req["method"].as_str().expect("method").to_owned();
                                    let params = req.get("params").cloned().unwrap_or(Value::Null);

                                    let st_c = Arc::clone(&st);
                                    let tx_c = out_tx.clone();
                                    tokio::spawn(async move {
                                        handle_mock_rpc(conn_num, id, method, params, st_c, tx_c).await;
                                    });
                                }
                                _ => {}
                            }
                        }
                    }
                }

                drop(out_tx);
                writer.abort();
            });
        }
    });

    format!("ws://{addr}/ws-api/v3")
}

async fn handle_mock_rpc(
    conn_num: usize,
    id: String,
    method: String,
    params: Value,
    st: Arc<MockWsState>,
    out_tx: tokio::sync::mpsc::Sender<Message>,
) {
    let rate_limits = json!([
        {
            "rateLimitType": "REQUEST_WEIGHT",
            "interval": "MINUTE",
            "intervalNum": 1,
            "limit": 6000,
            "count": 15
        }
    ]);

    match method.as_str() {
        "session.logon" => {
            // Verify Ed25519 signature if verifying key is registered.
            if let Some(vk) = *st.verifying_key.lock().unwrap() {
                let api_key = params["apiKey"].as_str().expect("apiKey");
                let ts = params["timestamp"].as_i64().expect("timestamp");
                let sig_b64 = params["signature"].as_str().expect("signature");
                let payload = if let Some(rw) = params.get("recvWindow").and_then(Value::as_u64) {
                    format!("apiKey={api_key}&recvWindow={rw}&timestamp={ts}")
                } else {
                    format!("apiKey={api_key}&timestamp={ts}")
                };
                let sig_bytes = BASE64_STANDARD.decode(sig_b64).expect("b64 sig");
                let sig = Signature::from_slice(&sig_bytes).expect("ed25519 sig");
                vk.verify(payload.as_bytes(), &sig)
                    .expect("mock server verified Ed25519 session.logon signature");
            }
            st.logon_count.fetch_add(1, Ordering::SeqCst);
            let resp = json!({
                "id": id,
                "status": 200,
                "result": {
                    "apiKey": params["apiKey"],
                    "authorizedSince": 1_770_736_694_138_i64,
                    "connectedSince": 1_770_736_694_100_i64,
                    "returnRateLimits": true,
                    "serverTime": 1_770_736_694_138_i64,
                    "userDataStream": false
                },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "userDataStream.subscribe" => {
            let sub_id = st.subscribe_count.fetch_add(1, Ordering::SeqCst);
            let resp = json!({
                "id": id,
                "status": 200,
                "result": { "subscriptionId": sub_id },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;

            // On connection 2 (during rotation overlap), replay fill (777, 501) AND emit new fill (777, 502).
            if conn_num == 2 {
                let dup_fill = make_exec_fill_event(777, 501, "b01r1S-000777-ENT");
                let new_fill = make_exec_fill_event(777, 502, "b01r1S-000777-ENT");
                let _ = out_tx
                    .send(Message::Text(dup_fill.to_string().into()))
                    .await;
                let _ = out_tx
                    .send(Message::Text(new_fill.to_string().into()))
                    .await;
            }
        }
        "order.place" => {
            let cid = params["newClientOrderId"].as_str().expect("cid").to_owned();
            let symbol = params["symbol"].as_str().expect("symbol").to_owned();
            let side = params["side"].as_str().expect("side").to_owned();
            let otype = params["type"].as_str().expect("type").to_owned();
            let qty = params
                .get("quantity")
                .and_then(Value::as_str)
                .unwrap_or("0.01")
                .to_owned();
            let price = params
                .get("price")
                .and_then(Value::as_str)
                .unwrap_or("50000.00")
                .to_owned();

            let order_id =
                i64::try_from(st.order_place_count.fetch_add(1, Ordering::SeqCst) + 1000)
                    .expect("order_id fits i64");
            let order_record = json!({
                "symbol": symbol,
                "orderId": order_id,
                "orderListId": -1,
                "clientOrderId": cid,
                "side": side,
                "type": otype,
                "price": price,
                "origQty": qty,
                "executedQty": "0",
                "cummulativeQuoteQty": "0",
                "status": "NEW",
                "updateTime": 1_770_736_695_000_i64
            });
            st.orders.lock().unwrap().insert(cid.clone(), order_record);

            let delay_ms = st.delay_order_place_ms.lock().unwrap().get(&cid).copied();
            if let Some(delay_ms) = delay_ms {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }

            let should_drop =
                st.drop_response_for_cid.lock().unwrap().as_deref() == Some(cid.as_str());
            if should_drop {
                // Intentionally never respond -> client times out with UnknownNeedsQuery!
                return;
            }

            let should_503 = st.return_503_for_cid.lock().unwrap().as_deref() == Some(cid.as_str());
            if should_503 {
                let resp = json!({
                    "id": id,
                    "status": 503,
                    "error": { "code": -1001, "msg": "Internal error; unable to process your request." }
                });
                let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
                return;
            }

            let resp = json!({
                "id": id,
                "status": 200,
                "result": {
                    "symbol": symbol,
                    "orderId": order_id,
                    "clientOrderId": cid,
                    "transactTime": 1_770_736_695_000_i64,
                    "status": "NEW",
                    "executedQty": "0",
                    "cummulativeQuoteQty": "0"
                },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "order.status" => {
            let cid = params["origClientOrderId"]
                .as_str()
                .expect("origClientOrderId");
            let found = st.orders.lock().unwrap().get(cid).cloned();
            let resp = match found {
                Some(rec) => {
                    json!({ "id": id, "status": 200, "result": rec, "rateLimits": rate_limits })
                }
                None => {
                    json!({ "id": id, "status": 400, "error": { "code": -2013, "msg": "Order does not exist." } })
                }
            };
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "order.cancel" => {
            st.cancel_count.fetch_add(1, Ordering::SeqCst);
            let cid = params["origClientOrderId"].as_str().unwrap_or("cid");
            let resp = json!({
                "id": id,
                "status": 200,
                "result": {
                    "symbol": params["symbol"],
                    "orderId": 1001,
                    "origClientOrderId": cid,
                    "status": "CANCELED",
                    "executedQty": "0"
                },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "order.cancelReplace" => {
            let new_cid = params["newClientOrderId"].as_str().unwrap_or("new-cid");
            let resp = json!({
                "id": id,
                "status": 200,
                "result": {
                    "cancelResult": "SUCCESS",
                    "newOrderResult": "SUCCESS",
                    "newOrderResponse": {
                        "symbol": params["symbol"],
                        "orderId": 2002,
                        "clientOrderId": new_cid,
                        "transactTime": 1_770_736_695_100_i64,
                        "status": "NEW",
                        "executedQty": "0",
                        "cummulativeQuoteQty": "0"
                    }
                },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "orderList.place.oco" => {
            let lcid = params["listClientOrderId"].as_str().unwrap_or("oco-1");
            let resp = json!({
                "id": id,
                "status": 200,
                "result": {
                    "orderListId": 500,
                    "listClientOrderId": lcid,
                    "listOrderStatus": "EXECUTING",
                    "symbol": params["symbol"],
                    "transactionTime": 1_770_736_695_200_i64,
                    "orders": [
                        { "symbol": params["symbol"], "orderId": 501, "clientOrderId": "tp-leg" },
                        { "symbol": params["symbol"], "orderId": 502, "clientOrderId": "sl-leg" }
                    ]
                },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "orderList.cancel" => {
            st.cancel_count.fetch_add(1, Ordering::SeqCst);
            let resp = json!({
                "id": id,
                "status": 200,
                "result": {
                    "orderListId": 500,
                    "listClientOrderId": params.get("listClientOrderId").and_then(Value::as_str).unwrap_or("oco-1"),
                    "listOrderStatus": "ALL_DONE",
                    "symbol": params["symbol"]
                },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "openOrders.status" => {
            let orders: Vec<Value> = st.orders.lock().unwrap().values().cloned().collect();
            let resp =
                json!({ "id": id, "status": 200, "result": orders, "rateLimits": rate_limits });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        "account.status" => {
            let rl_err = st.next_rate_limit_error.lock().unwrap().take();
            if let Some((status, offset_ms)) = rl_err {
                let now_ms = i64::try_from(
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .expect("epoch")
                        .as_millis(),
                )
                .unwrap();
                let resp = json!({
                    "id": id,
                    "status": status,
                    "error": {
                        "code": -1003,
                        "msg": "Too many requests",
                        "data": { "retryAfter": now_ms + offset_ms }
                    }
                });
                let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
                return;
            }
            let resp = json!({
                "id": id,
                "status": 200,
                "result": {
                    "canTrade": true,
                    "updateTime": 1_770_736_695_300_i64,
                    "balances": [
                        { "asset": "USDT", "free": "5000.00", "locked": "100.00" }
                    ]
                },
                "rateLimits": rate_limits
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
        other => {
            let resp = json!({
                "id": id,
                "status": 400,
                "error": { "code": -1100, "msg": format!("unsupported mock method {other}") }
            });
            let _ = out_tx.send(Message::Text(resp.to_string().into())).await;
        }
    }
}

fn make_exec_fill_event(order_id: i64, trade_id: i64, cid: &str) -> Value {
    json!({
        "subscriptionId": 0,
        "event": {
            "e": "executionReport",
            "E": 1_770_736_696_000_i64 + trade_id,
            "s": "BTCUSDT",
            "c": cid,
            "S": "BUY",
            "o": "LIMIT_MAKER",
            "f": "GTC",
            "q": "0.02",
            "p": "60000.00",
            "P": "0.00",
            "g": -1,
            "C": "",
            "x": "TRADE",
            "X": "PARTIALLY_FILLED",
            "r": "NONE",
            "i": order_id,
            "l": "0.01",
            "z": "0.01",
            "L": "60000.00",
            "n": "0.60",
            "N": "USDT",
            "T": 1_770_736_696_000_i64,
            "t": trade_id,
            "m": true,
            "Z": "600.00"
        }
    })
}

fn test_credentials(api_key: &str) -> (Ed25519Credentials, VerifyingKey) {
    let creds = Ed25519Credentials::from_seed(api_key.to_owned(), [99u8; 32]).expect("seed creds");
    let vk = VerifyingKey::from_bytes(&creds.public_key_bytes()).expect("vk");
    (creds, vk)
}

#[tokio::test]
async fn parallel_request_correlation_and_ping_pong_and_rate_limits() {
    let rest_url = spawn_mock_rest_server().await;
    let state = Arc::new(MockWsState::default());
    state.send_ping_on_first_conn.store(true, Ordering::SeqCst);
    let (creds, vk) = test_credentials("MOCK_API_KEY_PARALLEL");
    *state.verifying_key.lock().unwrap() = Some(vk);
    let ws_url = spawn_mock_ws_server(Arc::clone(&state)).await;

    let adapter = Arc::new(
        SpotAdapter::new(
            SpotAdapterConfig {
                rest_url,
                rest_timeout: Duration::from_secs(5),
                session: SpotSessionConfig {
                    ws_url,
                    request_timeout: Duration::from_secs(5),
                    ..SpotSessionConfig::default()
                },
            },
            creds,
        )
        .expect("build adapter"),
    );

    let report = adapter.activate().await.expect("activate");
    assert_eq!(report.venue, Venue::Spot);
    assert_eq!(report.symbols_loaded, 1);
    assert_eq!(adapter.session_state().await, SessionState::Ready);

    // Fire 32 concurrent place_order requests with varying delays to force out-of-order responses.
    for i in 0..32u64 {
        let cid = format!("b01r1S-{i:06}-ENT");
        let delay = (32 - i) * 2;
        state
            .delay_order_place_ms
            .lock()
            .unwrap()
            .insert(cid, delay);
    }

    let mut handles = Vec::new();
    for i in 0..32u64 {
        let ad = Arc::clone(&adapter);
        handles.push(tokio::spawn(async move {
            let cid = format!("b01r1S-{i:06}-ENT");
            let ack = ad
                .place_order(PlaceOrderRequest {
                    symbol: Symbol("BTCUSDT".into()),
                    side: Side::Buy,
                    kind: OrderKind::LimitMaker,
                    client_order_id: cid.clone(),
                    quantity: Some(Qty(Decimal::from_str("0.01").unwrap())),
                    quote_order_qty: None,
                    price: Some(Price(Decimal::from_str("60000.00").unwrap())),
                    stop_price: None,
                    time_in_force: None,
                })
                .await
                .expect("place_order ok");
            assert_eq!(ack.client_order_id, cid);
        }));
    }
    for h in handles {
        h.await.expect("task join");
    }

    // Verify rate limit tracker was populated from WS responses.
    let snap = adapter.rate_limits().snapshot();
    let weight_bucket = snap
        .get(&RateLimitBucketKey {
            rate_limit_type: RateLimitType::RequestWeight,
            interval: RateLimitInterval::Minute,
            interval_num: 1,
        })
        .expect("REQUEST_WEIGHT bucket");
    assert_eq!(weight_bucket.count, 15);
    assert_eq!(weight_bucket.limit, 6000);

    // Verify Ping payload was echoed back in Pong.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        state
            .pong_received
            .lock()
            .unwrap()
            .iter()
            .any(|p| p.as_slice() == b"binance-ping-42"),
        "expected Pong echoing Ping payload"
    );

    // Verify deactivate does NOT send any cancel commands (hard rule #7).
    let cancels_before = state.cancel_count.load(Ordering::SeqCst);
    adapter.deactivate().await.expect("deactivate");
    assert_eq!(state.cancel_count.load(Ordering::SeqCst), cancels_before);
    assert_eq!(adapter.session_state().await, SessionState::Disconnected);
}

#[tokio::test]
async fn server_shutdown_make_before_break_rotation_preserves_inflight_and_dedups_fills() {
    let rest_url = spawn_mock_rest_server().await;
    let state = Arc::new(MockWsState::default());
    let (creds, vk) = test_credentials("MOCK_API_KEY_ROTATE");
    *state.verifying_key.lock().unwrap() = Some(vk);
    state
        .delay_order_place_ms
        .lock()
        .unwrap()
        .insert("b01r1S-INFLT1-ENT".into(), 180);

    let ws_url = spawn_mock_ws_server(Arc::clone(&state)).await;
    let adapter = Arc::new(
        SpotAdapter::new(
            SpotAdapterConfig {
                rest_url,
                rest_timeout: Duration::from_secs(5),
                session: SpotSessionConfig {
                    ws_url,
                    request_timeout: Duration::from_secs(5),
                    drain_timeout: Duration::from_secs(2),
                    ..SpotSessionConfig::default()
                },
            },
            creds,
        )
        .expect("build adapter"),
    );

    adapter.activate().await.expect("activate");
    let mut events = adapter.take_event_rx().expect("event_rx");

    // Start a slow in-flight request on Connection 1.
    let ad_clone = Arc::clone(&adapter);
    let inflight = tokio::spawn(async move {
        ad_clone
            .place_order(PlaceOrderRequest {
                symbol: Symbol("BTCUSDT".into()),
                side: Side::Buy,
                kind: OrderKind::LimitMaker,
                client_order_id: "b01r1S-INFLT1-ENT".into(),
                quantity: Some(Qty(Decimal::from_str("0.01").unwrap())),
                quote_order_qty: None,
                price: Some(Price(Decimal::from_str("60000.00").unwrap())),
                stop_price: None,
                time_in_force: None,
            })
            .await
    });

    // Trigger serverShutdown + fill (777, 501) on Connection 1 -> opens Connection 2 which
    // replays (777, 501) and emits new fill (777, 502).
    tokio::time::sleep(Duration::from_millis(25)).await;
    state.send_shutdown_notify.notify_one();

    // In-flight request on Connection 1 MUST still succeed cleanly during drain.
    let ack = inflight
        .await
        .expect("join inflight")
        .expect("in-flight order must complete during drain");
    assert_eq!(ack.client_order_id, "b01r1S-INFLT1-ENT");

    // Collect events until we see fill 502.
    let mut trade_ids = Vec::new();
    let mut saw_shutdown = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(300), events.recv()).await {
            Ok(Some(GatewayEvent::Exec(ev))) => {
                trade_ids.push((ev.order_id, ev.trade_id));
                if ev.trade_id == 502 {
                    break;
                }
            }
            Ok(Some(GatewayEvent::ServerShutdown { .. })) => saw_shutdown = true,
            Ok(Some(_)) => {}
            _ => break,
        }
    }

    assert!(saw_shutdown, "expected ServerShutdown event");
    assert_eq!(
        trade_ids,
        vec![(777, 501), (777, 502)],
        "fill (777, 501) must be deduplicated across overlap and (777, 502) must not be lost"
    );
    assert_eq!(state.conn_count.load(Ordering::SeqCst), 2);
    assert_eq!(state.logon_count.load(Ordering::SeqCst), 2);
    assert_eq!(state.subscribe_count.load(Ordering::SeqCst), 2);

    // Subsequent request succeeds on Connection 2.
    let acct = adapter.account_snapshot().await.expect("account on conn 2");
    assert!(acct.can_trade);

    adapter.deactivate().await.expect("deactivate");
}

#[tokio::test]
async fn abrupt_disconnect_triggers_reconnect_and_resubscribe() {
    let rest_url = spawn_mock_rest_server().await;
    let state = Arc::new(MockWsState::default());
    let (creds, vk) = test_credentials("MOCK_API_KEY_RECONN");
    *state.verifying_key.lock().unwrap() = Some(vk);
    let ws_url = spawn_mock_ws_server(Arc::clone(&state)).await;

    let adapter = SpotAdapter::new(
        SpotAdapterConfig {
            rest_url,
            rest_timeout: Duration::from_secs(5),
            session: SpotSessionConfig {
                ws_url,
                request_timeout: Duration::from_secs(5),
                backoff_base: Duration::from_millis(20),
                backoff_max: Duration::from_millis(100),
                ..SpotSessionConfig::default()
            },
        },
        creds,
    )
    .expect("build adapter");

    adapter.activate().await.expect("activate");
    let mut events = adapter.take_event_rx().expect("event_rx");

    // Abruptly kill connection 1.
    state.drop_first_conn_notify.notify_one();

    // Wait for GatewayEvent::Reconnected.
    let mut reconnected = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), events.recv()).await {
            Ok(Some(GatewayEvent::Reconnected {
                venue: Venue::Spot, ..
            })) => {
                reconnected = true;
                break;
            }
            Ok(Some(_)) => {}
            _ => break,
        }
    }
    assert!(reconnected, "expected Reconnected event after drop");
    assert!(state.logon_count.load(Ordering::SeqCst) >= 2);
    assert!(state.subscribe_count.load(Ordering::SeqCst) >= 2);

    let acct = adapter
        .account_snapshot()
        .await
        .expect("requests work after reconnect");
    assert!(acct.can_trade);

    adapter.deactivate().await.expect("deactivate");
}

#[tokio::test]
async fn respects_retry_after_on_429_and_418() {
    let rest_url = spawn_mock_rest_server().await;
    let state = Arc::new(MockWsState::default());
    let (creds, _) = test_credentials("MOCK_API_KEY_RL");
    let ws_url = spawn_mock_ws_server(Arc::clone(&state)).await;

    let adapter = SpotAdapter::new(
        SpotAdapterConfig {
            rest_url,
            rest_timeout: Duration::from_secs(5),
            session: SpotSessionConfig {
                ws_url,
                request_timeout: Duration::from_secs(5),
                ..SpotSessionConfig::default()
            },
        },
        creds,
    )
    .expect("build adapter");
    adapter.activate().await.expect("activate");

    // 1. Trigger 429 with retryAfter = now + 250ms.
    *state.next_rate_limit_error.lock().unwrap() = Some((429, 250));
    let err = adapter
        .account_snapshot()
        .await
        .expect_err("first call gets 429");
    assert!(matches!(err, GatewayError::RateLimited { status: 429, .. }));

    // Immediate retry must be blocked locally without hitting server.
    let blocked = adapter
        .account_snapshot()
        .await
        .expect_err("second call blocked locally");
    assert!(matches!(blocked, GatewayError::RateLimited { .. }));

    // Wait past retryAfter -> call succeeds.
    tokio::time::sleep(Duration::from_millis(280)).await;
    adapter
        .account_snapshot()
        .await
        .expect("succeeds after retryAfter");

    // 2. Trigger 418 IP ban with retryAfter = now + 200ms.
    *state.next_rate_limit_error.lock().unwrap() = Some((418, 200));
    let err418 = adapter
        .account_snapshot()
        .await
        .expect_err("gets 418 IP ban");
    assert!(matches!(err418, GatewayError::IpBanned { .. }));

    let blocked418 = adapter
        .account_snapshot()
        .await
        .expect_err("blocked locally during IP ban");
    assert!(matches!(blocked418, GatewayError::IpBanned { .. }));

    adapter.deactivate().await.expect("deactivate");
}

#[tokio::test]
async fn timeout_and_5xx_return_unknown_needs_query_and_resolve_by_client_order_id() {
    let rest_url = spawn_mock_rest_server().await;
    let state = Arc::new(MockWsState::default());
    let (creds, _) = test_credentials("MOCK_API_KEY_UNK");
    *state.drop_response_for_cid.lock().unwrap() = Some("b01r1S-TIMOUT-ENT".into());
    *state.return_503_for_cid.lock().unwrap() = Some("b01r1S-ERR503-ENT".into());
    let ws_url = spawn_mock_ws_server(Arc::clone(&state)).await;

    let adapter = SpotAdapter::new(
        SpotAdapterConfig {
            rest_url,
            rest_timeout: Duration::from_secs(5),
            session: SpotSessionConfig {
                ws_url,
                request_timeout: Duration::from_millis(150),
                ..SpotSessionConfig::default()
            },
        },
        creds,
    )
    .expect("build adapter");
    adapter.activate().await.expect("activate");

    // Case A: Request timeout -> UnknownNeedsQuery.
    let err_timeout = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "b01r1S-TIMOUT-ENT".into(),
            quantity: Some(Qty(Decimal::from_str("0.01").unwrap())),
            quote_order_qty: None,
            price: Some(Price(Decimal::from_str("60000.00").unwrap())),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect_err("timed-out order must return UnknownNeedsQuery");
    assert!(err_timeout.is_unknown_needs_query());

    // Reconcile via query_order(origClientOrderId) -> recovers authoritative OrderStatus::New.
    let snap = adapter
        .query_order(QueryOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            orig_client_order_id: Some("b01r1S-TIMOUT-ENT".into()),
            order_id: None,
        })
        .await
        .expect("query_order resolves UnknownNeedsQuery");
    assert_eq!(snap.client_order_id, "b01r1S-TIMOUT-ENT");
    assert_eq!(snap.status, OrderStatus::New);

    // Case B: 503 response -> UnknownNeedsQuery, also resolvable via query_order.
    let err_503 = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "b01r1S-ERR503-ENT".into(),
            quantity: Some(Qty(Decimal::from_str("0.01").unwrap())),
            quote_order_qty: None,
            price: Some(Price(Decimal::from_str("60000.00").unwrap())),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect_err("503 order must return UnknownNeedsQuery");
    assert!(err_503.is_unknown_needs_query());

    let snap2 = adapter
        .query_order(QueryOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            orig_client_order_id: Some("b01r1S-ERR503-ENT".into()),
            order_id: None,
        })
        .await
        .expect("query_order resolves 503 order");
    assert_eq!(snap2.status, OrderStatus::New);

    adapter.deactivate().await.expect("deactivate");
}

#[derive(Clone, Default)]
struct LogBuf(Arc<Mutex<Vec<u8>>>);

impl io::Write for LogBuf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[tokio::test]
async fn no_secrets_appear_in_captured_logs_during_full_lifecycle() {
    let secret_api_key = "SUPER_SECRET_KEY_NEVER_LOG_987654";
    let log_buf = LogBuf::default();
    let make_writer = {
        let b = log_buf.clone();
        move || b.clone()
    };

    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(make_writer)
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);

    let rest_url = spawn_mock_rest_server().await;
    let state = Arc::new(MockWsState::default());
    let (creds, _) = test_credentials(secret_api_key);
    let ws_url = spawn_mock_ws_server(Arc::clone(&state)).await;

    let adapter = SpotAdapter::new(
        SpotAdapterConfig {
            rest_url,
            rest_timeout: Duration::from_secs(5),
            session: SpotSessionConfig {
                ws_url,
                request_timeout: Duration::from_secs(5),
                ..SpotSessionConfig::default()
            },
        },
        creds,
    )
    .expect("build adapter");

    // Log debug representation of adapter explicitly to verify redaction.
    tracing::debug!(?adapter, "starting adapter lifecycle test");

    adapter.activate().await.expect("activate");

    let _ = adapter
        .place_order(PlaceOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            client_order_id: "b01r1S-LOG001-ENT".into(),
            quantity: Some(Qty(Decimal::from_str("0.01").unwrap())),
            quote_order_qty: None,
            price: Some(Price(Decimal::from_str("60000.00").unwrap())),
            stop_price: None,
            time_in_force: None,
        })
        .await
        .expect("place_order");

    let _ = adapter
        .cancel_replace(CancelReplaceRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Buy,
            kind: OrderKind::LimitMaker,
            cancel_replace_mode: CancelReplaceMode::StopOnFailure,
            cancel_orig_client_order_id: Some("b01r1S-LOG001-ENT".into()),
            cancel_order_id: None,
            new_client_order_id: "b01r1S-LOG002-ENT".into(),
            quantity: Qty(Decimal::from_str("0.01").unwrap()),
            price: Some(Price(Decimal::from_str("60010.00").unwrap())),
            time_in_force: None,
        })
        .await
        .expect("cancel_replace");

    let oco = adapter
        .place_oco(PlaceOcoRequest {
            symbol: Symbol("BTCUSDT".into()),
            side: Side::Sell,
            list_client_order_id: "b01r1S-OCO001-SL".into(),
            quantity: Qty(Decimal::from_str("0.01").unwrap()),
            above_type: OrderKind::LimitMaker,
            above_client_order_id: Some("b01r1S-TP0001-TP1".into()),
            above_price: Some(Price(Decimal::from_str("65000.00").unwrap())),
            above_stop_price: None,
            above_time_in_force: None,
            below_type: OrderKind::StopLossLimit,
            below_client_order_id: Some("b01r1S-SL0001-SL".into()),
            below_price: Some(Price(Decimal::from_str("58900.00").unwrap())),
            below_stop_price: Some(Price(Decimal::from_str("59000.00").unwrap())),
            below_time_in_force: Some(TimeInForce::Gtc),
            below_trailing_delta: None,
        })
        .await
        .expect("place_oco");
    assert_eq!(oco.order_list_id, 500);

    let _ = adapter
        .cancel_list(CancelOrderListRequest {
            symbol: Symbol("BTCUSDT".into()),
            order_list_id: Some(500),
            list_client_order_id: None,
        })
        .await
        .expect("cancel_list");

    let _ = adapter
        .cancel_order(CancelOrderRequest {
            symbol: Symbol("BTCUSDT".into()),
            orig_client_order_id: Some("b01r1S-LOG002-ENT".into()),
            order_id: None,
            new_client_order_id: None,
        })
        .await
        .expect("cancel_order");

    let _ = adapter.open_orders(None).await.expect("open_orders");

    adapter.deactivate().await.expect("deactivate");

    let captured = String::from_utf8_lossy(&log_buf.0.lock().unwrap()).into_owned();
    assert!(!captured.is_empty(), "expected tracing logs to be captured");
    assert!(
        !captured.contains(secret_api_key),
        "API key leaked into tracing logs!"
    );
    assert!(
        !captured.contains("BEGIN PRIVATE KEY"),
        "PEM leaked into tracing logs!"
    );
}
